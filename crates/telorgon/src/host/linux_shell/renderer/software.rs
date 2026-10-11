use std::collections::{BTreeMap, VecDeque};

use crate::foundation::{ColorRgba8, RectI};
use crate::graphics::render::RenderBackend;
use crate::graphics::renderers::software::{
    SoftwareCompositeLayer, SoftwareRenderer, SoftwareScene, SoftwareSurface,
};

use super::super::geometry::{accumulated_damage, full_rect};
use super::super::scene::{ShellFrame, ShellLayerKey, ShellPlacement, ShellSceneKey};
mod motion;
use crate::host::application::{AppError, AppResult};

pub(in crate::host::linux_shell) struct SoftwareShellRenderer {
    renderer: SoftwareRenderer,
    scenes: BTreeMap<ShellSceneKey, SoftwareScene>,
    motion_snapshots: BTreeMap<u64, crate::graphics::render::ImageResource>,
    motion_mixes: BTreeMap<u64, motion::Mix>,
    motion_outputs: BTreeMap<u64, motion::Output>,
    surface: SoftwareSurface,
    backdrop: Option<Backdrop>,
    content_version: u64,
    target_versions: Vec<u64>,
    damage_history: VecDeque<(u64, Option<RectI>)>,
}

struct Backdrop {
    placements: Vec<ShellPlacement>,
    pixels: SoftwareSurface,
}

impl SoftwareShellRenderer {
    pub(super) fn invalidate_targets(&mut self) {
        self.target_versions.fill(0);
        self.damage_history.clear();
    }

    pub(super) fn new(targets: usize) -> Self {
        Self {
            renderer: SoftwareRenderer,
            scenes: BTreeMap::new(),
            motion_snapshots: BTreeMap::new(),
            motion_mixes: BTreeMap::new(),
            motion_outputs: BTreeMap::new(),
            surface: SoftwareSurface::default(),
            backdrop: None,
            content_version: 0,
            target_versions: vec![0; targets],
            damage_history: VecDeque::new(),
        }
    }

    pub(super) fn render(&mut self, target_index: usize, frame: ShellFrame) -> AppResult<RectI> {
        self.content_version = self.content_version.wrapping_add(1).max(1);
        self.damage_history
            .push_back((self.content_version, frame.damage));
        while self.damage_history.len() > 64 {
            self.damage_history.pop_front();
        }
        for update in &frame.updates {
            let scene = match self.scenes.entry(update.key) {
                std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::btree_map::Entry::Vacant(entry) => entry.insert(
                    self.renderer
                        .create_scene()
                        .map_err(|error| AppError::new(error.to_string()))?,
                ),
            };
            for delta in &update.deltas {
                self.renderer
                    .apply_scene_delta(scene, delta)
                    .map_err(|error| AppError::new(error.to_string()))?;
            }
        }
        self.scenes.retain(|key, _| frame.live_scenes.contains(key));
        for (key, scene) in &mut self.scenes {
            scene.set_frame_border(frame.frame_borders.get(key).cloned());
        }
        self.render_motion(&frame.motion)?;
        let previous_target_version = *self
            .target_versions
            .get(target_index)
            .ok_or_else(|| AppError::new("software scanout target index is invalid"))?;
        // The retained CPU surface already has the previous composition. Only the
        // scanout copy needs damage accumulated for a particular presentation target.
        let copy_damage = accumulated_damage(
            previous_target_version,
            self.content_version,
            &self.damage_history,
            frame.extent,
        );
        let mut layers = Vec::with_capacity(frame.placements.len());
        for placement in &frame.placements {
            let scene = self.scenes.get(&placement.scene).ok_or_else(|| {
                AppError::new(format!(
                    "software desktop scene {:?} has no retained content",
                    placement.scene
                ))
            })?;
            layers.push(SoftwareCompositeLayer {
                scene,
                target: placement.target,
                clip: placement.clip,
                rounded_clips: placement.rounded_clips,
            });
        }
        let output = full_rect(frame.extent);
        // Cache only the contiguous, full-output bottom layers. Partial, clipped, glass or
        // moving layers stay on the ordinary compositor path. Scene deltas invalidate pixels.
        let base_count = frame.placements.iter().take_while(|placement| {
            matches!(placement.key, ShellLayerKey::Background | ShellLayerKey::Widget(_))
                && placement.target == output
                && placement.clip.is_none_or(|clip| clip == output)
                && placement.rounded_clips.iter().all(Option::is_none)
                && !frame.glass.contains_key(&placement.scene)
                && !frame.frame_borders.contains_key(&placement.scene)
        }).count();
        if base_count == 0 {
            self.backdrop = None;
            self.renderer.render_composite(&mut self.surface, &layers, frame.extent,
                frame.damage, ColorRgba8::rgba(0, 0, 0, 255))
                .map_err(|error| AppError::new(error.to_string()))?;
        } else {
            let placements = &frame.placements[..base_count];
            let stale = self.backdrop.as_ref().is_none_or(|cached| {
                cached.pixels.framebuffer_extent() != frame.extent
                    || cached.placements != placements
                    || frame.updates.iter().any(|update| {
                        placements.iter().any(|placement| placement.scene == update.key)
                    })
            });
            if stale {
                let mut pixels = SoftwareSurface::default();
                self.renderer.render_composite(&mut pixels, &layers[..base_count], frame.extent,
                    None, ColorRgba8::rgba(0, 0, 0, 255))
                    .map_err(|error| AppError::new(error.to_string()))?;
                self.backdrop = Some(Backdrop { placements: placements.to_vec(), pixels });
            }
            self.renderer.render_composite_over(&mut self.surface, &layers[base_count..],
                frame.extent, frame.damage, &self.backdrop.as_ref().unwrap().pixels)
                .map_err(|error| AppError::new(error.to_string()))?;
        }
        for scene in self.scenes.values_mut() {
            scene.discard_pending_damage();
        }
        Ok(copy_damage.unwrap_or_else(|| full_rect(frame.extent)))
    }

    pub(super) fn pixels(&self) -> &[u8] {
        self.surface.pixels_rgba8()
    }

    pub(super) fn mark_copied(&mut self, target_index: usize) {
        self.target_versions[target_index] = self.content_version;
    }
}

#[cfg(test)]
mod motion_tests;

#[cfg(test)]
mod backdrop_tests;

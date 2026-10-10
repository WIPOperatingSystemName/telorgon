use crate::host::linux_shell::motion::{SnapshotInput, position_images};
use std::collections::{BTreeMap, VecDeque};

use crate::foundation::{ColorRgba8, RectI};
use crate::graphics::render::RenderBackend;
use crate::graphics::renderers::software::{
    SoftwareCompositeLayer, SoftwareRenderer, SoftwareScene, SoftwareSurface,
};

use super::super::geometry::{accumulated_damage, full_rect};
use super::super::scene::{ShellFrame, ShellLayerKey, ShellPlacement, ShellSceneKey};
use crate::host::application::{AppError, AppResult};

pub(in crate::host::linux_shell) struct SoftwareShellRenderer {
    renderer: SoftwareRenderer,
    scenes: BTreeMap<ShellSceneKey, SoftwareScene>,
    motion_snapshots: BTreeMap<u64, SoftwareSurface>,
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

    fn render_motion(&mut self, motion: &super::super::motion::MotionFrame) -> AppResult<()> {
        use super::super::motion::SnapshotContent;
        for command in &motion.snapshots {
            let mut surface = SoftwareSurface::default();
            match &command.content {
                SnapshotContent::Capture(placements) => {
                    let layers = placements
                        .iter()
                        .map(|p| {
                            Ok(SoftwareCompositeLayer {
                                scene: self.scenes.get(&p.scene).ok_or_else(|| {
                                    AppError::new(format!(
                                        "motion capture {} scene {:?} missing (layer {:?})",
                                        command.id, p.scene, p.key
                                    ))
                                })?,
                                target: p.target,
                                clip: p.clip,
                                rounded_clips: p.rounded_clips,
                            })
                        })
                        .collect::<AppResult<Vec<_>>>()?;
                    self.renderer
                        .render_composite(
                            &mut surface,
                            &layers,
                            command.extent,
                            None,
                            ColorRgba8::rgba(0, 0, 0, 0),
                        )
                        .map_err(|e| AppError::new(e.to_string()))?;
                }
                SnapshotContent::Mix(inputs) => {
                    let scene = self.sample_motion_scene(command.extent, inputs, true)?;
                    self.renderer
                        .render_composite(
                            &mut surface,
                            &[SoftwareCompositeLayer {
                                scene: &scene,
                                target: full_rect(command.extent),
                                clip: None,
                                rounded_clips: [None; 2],
                            }],
                            command.extent,
                            None,
                            ColorRgba8::rgba(0, 0, 0, 0),
                        )
                        .map_err(|e| AppError::new(e.to_string()))?;
                }
            }
            self.motion_snapshots.insert(command.id, surface);
        }
        for output in &motion.outputs {
            let scene =
                self.sample_motion_scene(output.extent, &[(output.source, output.opacity).into()], false)?;
            self.scenes.insert(ShellSceneKey::Motion(output.id), scene);
        }
        self.motion_snapshots
            .retain(|id, _| motion.live.contains(id));
        Ok(())
    }
    fn sample_motion_scene(
        &self,
        extent: crate::foundation::SizeI,
        inputs: &[SnapshotInput],
        additive: bool,
    ) -> AppResult<SoftwareScene> {
        use crate::graphics::render::{
            ImageAlphaMode, ImageColorEncoding, ImageId, ImagePixelFormat, ImageResource,
        };
        let images = inputs
            .iter()
            .enumerate()
            .map(|(i, input)| (ImageId(i as u32 + 1), input.weight))
            .collect::<Vec<_>>();
        let mut source = super::super::motion::image_scene(extent, &images, additive);
        position_images(&mut source, inputs);
        for (input, (image, _)) in inputs.iter().zip(images) {
            let snapshot = &input.id;
            let pixels = self
                .motion_snapshots
                .get(snapshot)
                .ok_or_else(|| AppError::new("motion snapshot missing"))?;
            source
                .set_image_resource(ImageResource {
                    image,
                    content_version: 1,
                    extent: pixels.framebuffer_extent(),
                    color_encoding: ImageColorEncoding::Srgb,
                    alpha_mode: ImageAlphaMode::Premultiplied,
                    pixel_format: ImagePixelFormat::Rgba8,
                    pixels: pixels.pixels_rgba8().into(),
                })
                .map_err(|e| AppError::new(e.to_string()))?;
        }
        let mut scene = self
            .renderer
            .create_scene()
            .map_err(|e| AppError::new(e.to_string()))?;
        self.renderer
            .apply_scene_delta(
                &mut scene,
                &source.take_delta().expect("new motion image scene"),
            )
            .map_err(|e| AppError::new(e.to_string()))?;
        Ok(scene)
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

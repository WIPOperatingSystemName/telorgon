//! Retained immutable pixels and output scenes. Ordinary crossfades resolve at composition;
//! an interrupted or rescaled mixture is frozen once before it becomes another capture input.
use super::*;
use crate::foundation::SizeI;
use crate::graphics::render::{ImageAlphaMode, ImageColorEncoding, ImageId, ImagePixelFormat, ImageResource, RenderScene};
use crate::host::linux_shell::motion::{MotionFrame, SnapshotContent, SnapshotInput, image_scene, position_images, update_image_scene};
use std::sync::Arc;

pub(super) struct Mix {
    extent: SizeI,
    inputs: Vec<SnapshotInput>,
}

#[derive(Default)]
pub(super) struct Output {
    pub(super) source: RenderScene,
    inputs: Vec<u64>,
    signature: Option<(SizeI, Vec<SnapshotInput>, Option<f32>)>,
}

impl SoftwareShellRenderer {
    pub(super) fn render_motion(&mut self, motion: &MotionFrame) -> AppResult<()> {
        for command in &motion.snapshots {
            match &command.content {
                SnapshotContent::Capture(placements) => {
                    let layers = placements.iter().map(|p| {
                        Ok(SoftwareCompositeLayer {
                            scene: self.scenes.get(&p.scene).ok_or_else(|| AppError::new(format!(
                                "motion capture {} scene {:?} missing (layer {:?})", command.id, p.scene, p.key)))?,
                            target: p.target, clip: p.clip, rounded_clips: p.rounded_clips,
                        })
                    }).collect::<AppResult<Vec<_>>>()?;
                    let mut surface = SoftwareSurface::default();
                    self.renderer.render_composite(&mut surface, &layers, command.extent, None,
                        ColorRgba8::rgba(0, 0, 0, 0)).map_err(|e| AppError::new(e.to_string()))?;
                    self.store_snapshot(command.id, surface);
                }
                SnapshotContent::Mix(inputs) => {
                    // Freeze a previous displayed mixture only when an interruption references
                    // it. This also bounds recipe depth and releases superseded endpoints.
                    for input in inputs { self.materialize_mix(input.id)?; }
                    self.motion_mixes.insert(command.id, Mix { extent: command.extent, inputs: inputs.clone() });
                }
            }
        }
        for output in &motion.outputs {
            if self.motion_mixes.get(&output.source).is_some_and(|mix|
                mix.extent != output.extent || mix.inputs.iter().any(|input| {
                    let rect = input.target;
                    [rect.x * mix.extent.width as f32, rect.y * mix.extent.height as f32,
                        rect.width * mix.extent.width as f32, rect.height * mix.extent.height as f32]
                        .iter().any(|value| value.fract() != 0.0)
                })) {
                // Preserve the original capture's sampling grid during geometry animation.
                self.materialize_mix(output.source)?;
            }
            let (inputs, mixed) = self.motion_mixes.get(&output.source)
                .map_or_else(|| (vec![(output.source, output.opacity).into()], false),
                    |mix| (mix.inputs.clone(), true));
            let state = self.motion_outputs.entry(output.id).or_default();
            let signature = (output.extent, inputs.clone(), mixed.then_some(output.opacity));
            if state.signature.as_ref() == Some(&signature)
                && self.scenes.contains_key(&ShellSceneKey::Motion(output.id)) { continue; }
            let images = inputs.iter().enumerate().map(|(i, input)| (ImageId(i as u32 + 1), input.weight)).collect::<Vec<_>>();
            update_image_scene(&mut state.source, output.extent, &images, mixed);
            position_images(&mut state.source, &inputs);
            for (index, input) in inputs.iter().enumerate() {
                if state.inputs.get(index) == Some(&input.id) { continue; }
                let image = ImageId(index as u32 + 1);
                let snapshot = self.motion_snapshots.get(&input.id).ok_or_else(|| AppError::new("motion snapshot missing"))?;
                let mut resource = snapshot.clone();
                resource.image = image;
                resource.content_version = state.source.image_resource_version(image).unwrap_or(0) + 1;
                state.source.set_image_resource(resource).map_err(|e| AppError::new(e.to_string()))?;
            }
            state.inputs = inputs.iter().map(|input| input.id).collect();
            let scene = self.scenes.entry(ShellSceneKey::Motion(output.id)).or_default();
            if let Some(delta) = state.source.take_delta() {
                self.renderer.apply_scene_delta(scene, &delta).map_err(|e| AppError::new(e.to_string()))?;
            }
            scene.set_image_mix_opacity(mixed.then_some(output.opacity));
            state.signature = Some(signature);
        }
        // A deferred mixture owns its endpoints until it is frozen or retired. These pins
        // must survive even if the controller only references the displayed mixture itself.
        self.motion_mixes.retain(|id, _| motion.live.contains(id));
        let mut live = motion.live.clone();
        for mix in self.motion_mixes.values() { live.extend(mix.inputs.iter().map(|input| input.id)); }
        self.motion_snapshots.retain(|id, _| live.contains(id));
        self.motion_outputs.retain(|id, _| motion.outputs.iter().any(|output| output.id == *id));
        Ok(())
    }

    fn store_snapshot(&mut self, id: u64, surface: SoftwareSurface) {
        self.motion_snapshots.insert(id, ImageResource {
            image: ImageId(1), content_version: 1, extent: surface.framebuffer_extent(),
            color_encoding: ImageColorEncoding::Srgb, alpha_mode: ImageAlphaMode::Premultiplied,
            pixel_format: ImagePixelFormat::Rgba8, pixels: surface.into_pixels_rgba8(),
        });
    }

    fn materialize_mix(&mut self, id: u64) -> AppResult<()> {
        let Some(mix) = self.motion_mixes.remove(&id) else { return Ok(()); };
        let scene = self.sample_motion_scene(mix.extent, &mix.inputs, true)?;
        let mut surface = SoftwareSurface::default();
        self.renderer.render_composite(&mut surface, &[SoftwareCompositeLayer {
            scene: &scene, target: full_rect(mix.extent), clip: None, rounded_clips: [None; 2],
        }], mix.extent, None, ColorRgba8::rgba(0, 0, 0, 0)).map_err(|e| AppError::new(e.to_string()))?;
        self.store_snapshot(id, surface);
        Ok(())
    }

    fn sample_motion_scene(&self, extent: SizeI, inputs: &[SnapshotInput], additive: bool) -> AppResult<SoftwareScene> {
        let images = inputs.iter().enumerate().map(|(i, input)| (ImageId(i as u32 + 1), input.weight)).collect::<Vec<_>>();
        let mut source = image_scene(extent, &images, additive);
        position_images(&mut source, inputs);
        for (input, (image, _)) in inputs.iter().zip(images) {
            let snapshot = self.motion_snapshots.get(&input.id).ok_or_else(|| AppError::new("motion snapshot missing"))?;
            source.set_image_resource(ImageResource { image, pixels: Arc::clone(&snapshot.pixels), ..snapshot.clone() })
                .map_err(|e| AppError::new(e.to_string()))?;
        }
        let mut scene = SoftwareScene::default();
        self.renderer.apply_scene_delta(&mut scene, &source.take_delta().expect("new motion image scene"))
            .map_err(|e| AppError::new(e.to_string()))?;
        Ok(scene)
    }
}

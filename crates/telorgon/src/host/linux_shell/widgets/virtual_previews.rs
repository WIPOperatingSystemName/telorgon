//! Virtual thumbnails reference the same isolated placements as authorized capture.
use super::*;

impl WidgetLayer {
    pub(in crate::host::linux_shell) fn virtual_preview_layers(
        &self,
        sources: &[ShellLayer],
        outputs: &super::super::virtual_outputs::VirtualOutputs,
        locked: bool,
    ) -> Vec<ShellLayer> {
        if locked || !self.presented {
            return Vec::new();
        }
        let extent = self.layer.runtime.extent();
        let mut result = Vec::new();
        for (index, preview) in self.binding.3.borrow().iter().take(8).enumerate() {
            let Some(scene) = outputs
                .get(preview.output)
                .and_then(|output| output.scene.as_ref())
            else {
                continue;
            };
            let r = preview.rect;
            if ![r.x, r.y, r.width, r.height]
                .into_iter()
                .all(f32::is_finite)
                || r.x < 0.0
                || r.y < 0.0
                || r.width <= 0.0
                || r.height <= 0.0
                || r.x + r.width > extent.width
                || r.y + r.height > extent.height
            {
                continue;
            }
            let sx = self.sampled.width as f32 / extent.width.max(1.0);
            let sy = self.sampled.height as f32 / extent.height.max(1.0);
            let slot = RectI {
                x: self.sampled.x.saturating_add((r.x * sx).round() as i32),
                y: self.sampled.y.saturating_add((r.y * sy).round() as i32),
                width: (r.width * sx).round() as i32,
                height: (r.height * sy).round() as i32,
            };
            let bounds = RectI {
                x: 0,
                y: 0,
                width: scene.layout.width() as i32,
                height: scene.layout.height() as i32,
            };
            let Some(fitted) = fit_preview(
                SizeI {
                    width: bounds.width,
                    height: bounds.height,
                },
                slot,
            ) else {
                continue;
            };
            for (placement_index, placement) in scene.placements.iter().enumerate() {
                let ShellSceneKey::Surface(raw) = placement.scene else {
                    continue;
                };
                let Some(source) = sources
                    .iter()
                    .find(|source| source.key == ShellLayerKey::Surface(raw))
                else {
                    continue;
                };
                let ShellLayerContent::Image {
                    scene: source_scene,
                    content_version,
                    alpha_mode,
                    pixel_format,
                    ..
                } = source.content
                else {
                    continue;
                };
                if source_scene != placement.scene
                    || !scene.sampled.contains(&(raw, content_version))
                {
                    continue;
                }
                let clip = map_preview_rect(placement.clip.unwrap_or(bounds), bounds, fitted);
                let Some(clip) = intersect_rect(clip, fitted)
                    .and_then(|clip| intersect_rect(clip, self.sampled))
                else {
                    continue;
                };
                result.push(ShellLayer::image(
                    ShellLayerKey::OutputPreview(self.id, index as u32, placement_index as u32),
                    source_scene,
                    content_version,
                    ShellImageUpdate::Unchanged,
                    source.source_extent,
                    map_preview_rect(placement.target, bounds, fitted),
                    Some(clip),
                    alpha_mode,
                    pixel_format,
                    true,
                ));
            }
        }
        result
    }
}

use super::*;
use crate::graphics::render::{ImageResource, frame_border};

#[derive(Clone, Debug)]
pub(super) struct FrameCache {
    epoch: u64,
    interior: SoftwareScene,
    pub(super) output: SoftwareScene,
}

impl SoftwareScene {
    pub(crate) fn set_frame_border(&mut self, border: Option<BoxInstance>) {
        if self.frame_border != border {
            self.frame_border = border;
            self.frame_cache = Default::default();
        }
    }

    pub(super) fn prepare_frame_border(&self) -> RenderResult<()> {
        let Some(border) = &self.frame_border else {
            return Ok(());
        };
        let mut guard = self.frame_cache.lock().expect("frame cache poisoned");
        if guard.as_ref().is_some_and(|cache| cache.epoch == self.epoch) {
            return Ok(());
        }
        let extent = SizeI {
            width: self.extent.width.round() as i32,
            height: self.extent.height.round() as i32,
        };
        if extent.width <= 0 || extent.height <= 0 {
            return Err(RenderError::new(
                RenderErrorKind::InvalidTarget,
                "software frame extent must be positive",
            ));
        }
        let bounds = RectF {
            x: 0.0, y: 0.0,
            width: extent.width as f32, height: extent.height as f32,
        };
        let fresh = guard.as_ref().is_none_or(|cache| cache.output.extent != self.extent);
        if fresh {
            *guard = Some(FrameCache {
                epoch: 0,
                interior: empty_image_scene(self.extent, extent)?,
                output: empty_image_scene(self.extent, extent)?,
            });
        }
        let cache = guard.as_mut().unwrap();
        // Damage accumulates across deltas until composition. A new cache or missing
        // damage history needs a full resolve; ordinary hover updates retain both images.
        let region = if fresh || self.pending_damage.full || self.pending_damage.is_empty() {
            Some(bounds)
        } else {
            self.pending_damage.rects.iter().filter_map(|rect| {
                let left = rect.x.floor();
                let top = rect.y.floor();
                RectF {
                    x: left, y: top,
                    width: rect.right().ceil() - left,
                    height: rect.bottom().ceil() - top,
                }.intersection(bounds)
            }).reduce(RectF::union)
        };
        if let Some(region) = region {
            let width = extent.width as usize;
            let height = extent.height as usize;
            let interior = &mut cache.interior.image_resources
                .get_mut(&frame_border::IMAGE).unwrap().pixels;
            clear_region(interior, width, height, region,
                ColorRgba8::rgba(0, 0, 0, 0), ColorSpace::Srgb);
            self.draw_region(&mut RasterTarget {
                pixels: interior, width, height, origin: Default::default(),
                blend_mode: BlendMode::Alpha, color_space: ColorSpace::Srgb,
                rounded_clips: [None; 2],
                coverage_normalization: Some(frame_border::InteriorPaint::new(border)),
            }, region);

            let output = &mut cache.output.image_resources
                .get_mut(&frame_border::IMAGE).unwrap().pixels;
            clear_region(output, width, height, region,
                ColorRgba8::rgba(0, 0, 0, 0), ColorSpace::Srgb);
            let mut target = RasterTarget {
                pixels: output, width, height, origin: Default::default(),
                blend_mode: BlendMode::Alpha, color_space: ColorSpace::Srgb,
                rounded_clips: frame_border::resolve_clips(border),
                coverage_normalization: None,
            };
            cache.interior.draw_region(&mut target, region);
            let mut ring = border.clone();
            ring.background = None;
            ring.shadows = Default::default();
            ring.outline = Default::default();
            ring.clip = crate::graphics::render::ClipId(0);
            ring.spatial = SpatialId(0);
            target.rounded_clips = [None; 2];
            target.blend_mode = BlendMode::Add;
            draw_box(&mut target, &ring, None, None, region);
        }
        cache.epoch = self.epoch;
        Ok(())
    }
}

fn empty_image_scene(logical: SizeF, extent: SizeI) -> RenderResult<SoftwareScene> {
    let mut scene = frame_border::image_scene(logical);
    scene.set_image_resource(ImageResource {
        image: frame_border::IMAGE, content_version: 1, extent,
        color_encoding: ImageColorEncoding::Srgb,
        alpha_mode: ImageAlphaMode::Premultiplied,
        pixel_format: ImagePixelFormat::Rgba8,
        pixels: vec![0; extent.width as usize * extent.height as usize * 4].into(),
    })?;
    let mut output = SoftwareRenderer.create_scene()?;
    SoftwareRenderer.apply_scene_delta(&mut output, &scene.take_delta().unwrap())?;
    Ok(output)
}

#[cfg(test)]
mod tests;

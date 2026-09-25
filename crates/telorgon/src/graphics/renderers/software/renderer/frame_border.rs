use super::*;
use crate::graphics::render::{ImageResource, frame_border};

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
        if self
            .frame_cache
            .lock()
            .expect("frame cache poisoned")
            .as_ref()
            .is_some_and(|(epoch, _)| *epoch == self.epoch)
        {
            return Ok(());
        }
        let extent = SizeI {
            width: self.extent.width.round() as i32,
            height: self.extent.height.round() as i32,
        };
        let target = RectI {
            x: 0,
            y: 0,
            width: extent.width,
            height: extent.height,
        };
        let mut source = self.clone();
        source.frame_border = None;
        source.frame_cache = Default::default();
        source.coverage_normalization = Some(frame_border::InteriorPaint::new(border));
        let mut interior = SoftwareSurface::default();
        SoftwareRenderer.render_composite(
            &mut interior,
            &[SoftwareCompositeLayer {
                scene: &source,
                target,
                clip: None,
                rounded_clips: [None; 2],
            }],
            extent,
            None,
            ColorRgba8::rgba(0, 0, 0, 0),
        )?;

        let mut resolve = frame_border::image_scene(self.extent);
        attach_image(&mut resolve, &interior)?;
        let mut resolve_source = SoftwareRenderer.create_scene()?;
        SoftwareRenderer.apply_scene_delta(&mut resolve_source, &resolve.take_delta().unwrap())?;
        let mut ring = SoftwareRenderer.create_scene()?;
        SoftwareRenderer.apply_scene_delta(
            &mut ring,
            &frame_border::border_scene(self.extent, border)
                .take_delta()
                .unwrap(),
        )?;
        let mut resolved = SoftwareSurface::default();
        SoftwareRenderer.render_composite(
            &mut resolved,
            &[
                SoftwareCompositeLayer {
                    scene: &resolve_source,
                    target,
                    clip: None,
                    rounded_clips: frame_border::resolve_clips(border),
                },
                SoftwareCompositeLayer {
                    scene: &ring,
                    target,
                    clip: None,
                    rounded_clips: [None; 2],
                },
            ],
            extent,
            None,
            ColorRgba8::rgba(0, 0, 0, 0),
        )?;
        let mut output = frame_border::image_scene(self.extent);
        attach_image(&mut output, &resolved)?;
        let mut cached = SoftwareRenderer.create_scene()?;
        SoftwareRenderer.apply_scene_delta(&mut cached, &output.take_delta().unwrap())?;
        *self.frame_cache.lock().expect("frame cache poisoned") =
            Some((self.epoch, Box::new(cached)));
        Ok(())
    }
}

fn attach_image(
    scene: &mut crate::graphics::render::RenderScene,
    surface: &SoftwareSurface,
) -> RenderResult<()> {
    scene.set_image_resource(ImageResource {
        image: frame_border::IMAGE,
        content_version: 1,
        extent: surface.framebuffer_extent(),
        color_encoding: ImageColorEncoding::Srgb,
        alpha_mode: ImageAlphaMode::Premultiplied,
        pixel_format: ImagePixelFormat::Rgba8,
        pixels: surface.pixels_rgba8().into(),
    })
}

#[cfg(test)]
mod tests;

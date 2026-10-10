use super::*;

#[cfg(test)]
mod tests;

impl SoftwareRenderer {
    /// Composites independent retained scenes directly into one software framebuffer.
    ///
    /// The caller supplies back-to-front placement order and output-space damage. Ordinary scenes
    /// draw directly; bordered frames cache an isolated interior and border resolve.
    pub(crate) fn render_composite(
        &self,
        surface: &mut SoftwareSurface,
        layers: &[SoftwareCompositeLayer<'_>],
        extent: SizeI,
        damage: Option<RectI>,
        clear: ColorRgba8,
    ) -> RenderResult<RenderStats> {
        self.composite(surface, layers, extent, damage, clear, None)
    }

    /// Restore an unchanged opaque desktop backdrop before painting foreground damage.
    pub(crate) fn render_composite_over(
        &self,
        surface: &mut SoftwareSurface,
        layers: &[SoftwareCompositeLayer<'_>],
        extent: SizeI,
        damage: Option<RectI>,
        backdrop: &SoftwareSurface,
    ) -> RenderResult<RenderStats> {
        if backdrop.framebuffer_extent != extent {
            return Err(RenderError::new(RenderErrorKind::InvalidTarget, "cached backdrop extent changed"));
        }
        self.composite(surface, layers, extent, damage, ColorRgba8::rgba(0, 0, 0, 255), Some(backdrop))
    }

    fn composite(
        &self,
        surface: &mut SoftwareSurface,
        layers: &[SoftwareCompositeLayer<'_>],
        extent: SizeI,
        damage: Option<RectI>,
        clear: ColorRgba8,
        backdrop: Option<&SoftwareSurface>,
    ) -> RenderResult<RenderStats> {
        if extent.width <= 0 || extent.height <= 0 {
            return Err(RenderError::new(
                RenderErrorKind::InvalidTarget,
                "software composite target extent must be positive",
            ));
        }
        let output = RectI {
            x: 0,
            y: 0,
            width: extent.width,
            height: extent.height,
        };
        let resized = surface.ensure_framebuffer(extent);
        let render_region = if resized {
            output
        } else {
            damage.unwrap_or(output)
        };
        if !rect_contains(output, render_region) {
            return Err(RenderError::new(
                RenderErrorKind::InvalidTarget,
                "software composite damage lies outside the output",
            ));
        }
        surface.presented_damage = if render_region == output {
            DamageRegion {
                full: true,
                rects: Vec::new(),
                ..DamageRegion::default()
            }
        } else {
            DamageRegion {
                full: false,
                rects: vec![rect_i_to_f(render_region)],
                ..DamageRegion::default()
            }
        };
        let width = extent.width as usize;
        let height = extent.height as usize;
        if let Some(backdrop) = backdrop {
            let stride = width * 4;
            for y in render_region.y as usize..render_region.bottom() as usize {
                let start = y * stride + render_region.x as usize * 4;
                let end = start + render_region.width as usize * 4;
                surface.framebuffer_rgba8[start..end]
                    .copy_from_slice(&backdrop.framebuffer_rgba8[start..end]);
            }
        } else {
            clear_region(&mut surface.framebuffer_rgba8, width, height,
                rect_i_to_f(render_region), clear, ColorSpace::Srgb);
        }

        let mut batches = 0_u32;
        let mut epoch = 0_u64;
        for layer in layers {
            layer.scene.prepare_frame_border()?;
            let cached = layer.scene.frame_cache.lock().expect("frame cache poisoned");
            let scene = cached.as_ref().map_or(layer.scene, |cache| &cache.output);
            if layer.rounded_clips.iter().flatten().any(|c| !c.is_valid()) {
                return Err(RenderError::new(
                    RenderErrorKind::HostContract,
                    "invalid rounded composite clip",
                ));
            }
            let expected = SizeI {
                width: layer.scene.extent.width.round() as i32,
                height: layer.scene.extent.height.round() as i32,
            };
            if expected.width <= 0 || expected.height <= 0 {
                return Err(RenderError::new(
                    RenderErrorKind::HostContract,
                    "software desktop scene extent must be positive",
                ));
            }
            epoch = epoch.max(layer.scene.epoch);
            let visible = intersect_rect_i(layer.target, render_region).and_then(|region| {
                layer
                    .clip
                    .map_or(Some(region), |clip| intersect_rect_i(region, clip))
            });
            if let Some(visible) = visible {
                let local = RectF {
                    x: (visible.x - layer.target.x) as f32,
                    y: (visible.y - layer.target.y) as f32,
                    width: visible.width as f32,
                    height: visible.height as f32,
                };
                let mut target = RasterTarget {
                    pixels: &mut surface.framebuffer_rgba8,
                    width,
                    height,
                    origin: crate::foundation::PointI {
                        x: layer.target.x,
                        y: layer.target.y,
                    },
                    blend_mode: BlendMode::Alpha,
                    color_space: ColorSpace::Srgb,
                    rounded_clips: layer.rounded_clips,
                    coverage_normalization: scene.coverage_normalization,
                };
                if expected.width == layer.target.width && expected.height == layer.target.height {
                    scene.draw_region(&mut target, local);
                } else {
                    self.draw_scaled_composite(scene, expected, SizeI {
                        width: layer.target.width,
                        height: layer.target.height,
                    }, &mut target, local);
                }
                batches = batches.saturating_add(
                    layer
                        .scene
                        .draw_order
                        .iter()
                        .enumerate()
                        .filter(|(index, item)| {
                            *index == 0 || layer.scene.draw_order[index - 1].batch != item.batch
                        })
                        .count() as u32,
                );
            }
        }
        Ok(RenderStats {
            recorded: true,
            epoch,
            upload_bytes_recorded: 0,
            buffer_copies: 0,
            buffer_allocations: 0,
            descriptor_writes: 0,
            passes: 1,
            barriers: 0,
            batches,
            draws: batches,
            dispatches: 0,
            damage_area: render_region.width as f32 * render_region.height as f32,
        })
    }
}

impl SoftwareRenderer {
    fn draw_scaled_composite(
        &self,
        scene: &SoftwareScene,
        source_extent: SizeI,
        destination_extent: SizeI,
        target: &mut RasterTarget<'_>,
        region: RectF,
    ) {
        // Visibility animations and previews scale placements independently of retained UI
        // layout. Resolve at native size, then sample the group, preserving internal blending.
        let mut surface = SoftwareSurface::default();
        surface.ensure_framebuffer(source_extent);
        let source_rect = RectF {
            x: 0.0,
            y: 0.0,
            width: source_extent.width as f32,
            height: source_extent.height as f32,
        };
        scene.draw_region(&mut RasterTarget {
            pixels: &mut surface.framebuffer_rgba8,
            width: source_extent.width as usize,
            height: source_extent.height as usize,
            origin: Default::default(),
            blend_mode: BlendMode::Alpha,
            color_space: ColorSpace::Srgb,
            rounded_clips: [None; 2],
            coverage_normalization: scene.coverage_normalization,
        }, source_rect);
        let rect = RectF {
            width: destination_extent.width as f32,
            height: destination_extent.height as f32,
            ..source_rect
        };
        let instance = ImageInstance {
            node: crate::ui::UiNodeId::new(0, 1),
            image: ImageId(1),
            tint: None,
            rect,
            view_bounds: rect,
            content_version: 1,
            opacity: 1.0,
            clip: crate::ui::layout::ClipId(0),
            spatial: SpatialId(0),
        };
        let image = SoftwareImage {
            extent: source_extent,
            color_encoding: ImageColorEncoding::Srgb,
            alpha_mode: ImageAlphaMode::Premultiplied,
            pixel_format: ImagePixelFormat::Rgba8,
            pixels: surface.framebuffer_rgba8,
        };
        // Interior normalization was resolved in the source; output clips apply once here.
        target.coverage_normalization = None;
        draw_image(target, &instance, None, None, region, &image);
    }
}

use super::*;

pub(super) struct RenderedCursor {
    pub(super) rgba: Vec<u8>,
    pub(super) size: SizeI,
    pub(super) logical_size: SizeI,
    pub(super) hotspot: PointI,
    pub(super) premultiplied: bool,
}

impl RenderedCursor {
    /// Hardware cursors consume physical pixels and a physical hotspot. The composited form
    /// retains a logical hotspot, so switching paths cannot shift the pointer's active pixel.
    pub(super) fn for_hardware(
        &self,
        scale: crate::platform::contracts::ScaleFactor,
        maximum: SizeI,
    ) -> AppResult<Self> {
        let size = SizeI {
            width: (self.logical_size.width as f32 * scale.get())
                .round()
                .max(1.0) as i32,
            height: (self.logical_size.height as f32 * scale.get())
                .round()
                .max(1.0) as i32,
        };
        if size.width > maximum.width
            || size.height > maximum.height
            || self.size.width <= 0
            || self.size.height <= 0
            || self.logical_size.width <= 0
            || self.logical_size.height <= 0
            || (self.size.width as usize)
                .checked_mul(self.size.height as usize)
                .and_then(|pixels| pixels.checked_mul(4))
                != Some(self.rgba.len())
        {
            return Err(AppError::new(
                "cursor image cannot fit the hardware cursor plane",
            ));
        }
        let rgba = if size == self.size {
            self.rgba.clone()
        } else {
            let mut rgba = vec![0; size.width as usize * size.height as usize * 4];
            for y in 0..size.height as usize {
                for x in 0..size.width as usize {
                    let sx = (x * self.size.width as usize / size.width as usize)
                        .min(self.size.width as usize - 1);
                    let sy = (y * self.size.height as usize / size.height as usize)
                        .min(self.size.height as usize - 1);
                    let src = (sy * self.size.width as usize + sx) * 4;
                    let dst = (y * size.width as usize + x) * 4;
                    rgba[dst..dst + 4].copy_from_slice(&self.rgba[src..src + 4]);
                }
            }
            rgba
        };
        Ok(Self {
            rgba,
            size,
            logical_size: self.logical_size,
            premultiplied: self.premultiplied,
            hotspot: PointI {
                x: (self.hotspot.x as f32 * scale.get()).round() as i32,
                y: (self.hotspot.y as f32 * scale.get()).round() as i32,
            },
        })
    }
}

pub(super) enum CursorVisual {
    Image(RenderedCursor),
}

impl CursorVisual {
    pub(super) fn image(&self) -> Option<&RenderedCursor> {
        match self {
            Self::Image(image) => Some(image),
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_cursor_image(
    image: CursorImage,
    pointer: &mut Option<Layer>,
    icons: &mut [(String, Layer)],
    windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
    extent: SizeI,
    now: u64,
    pointer_config: &PointerConfiguration,
    pointer_theme: Option<&PointerTheme>,
    pointer_media: &mut AssetMediaCache,
    output_scale: crate::platform::contracts::ScaleFactor,
) -> AppResult<Option<CursorVisual>> {
    // A cursor surface request can arrive before its asynchronous SHM publication.
    // Missing pixels mean "not ready", not an explicit request to hide the pointer.
    let image = ready_cursor_image(image, windows);
    let rendered =
        match image {
            CursorImage::TelorgonDefault => render_semantic_pointer(
                PointerIcon::Default,
                None,
                pointer,
                icons,
                extent,
                now,
                pointer_config,
                pointer_theme,
                pointer_media,
                output_scale,
            )?,
            CursorImage::Shape(shape) => render_semantic_pointer(
                cursor_shape_pointer_icon(shape).unwrap_or(PointerIcon::Default),
                cursor_shape_icon_name(shape),
                pointer,
                icons,
                extent,
                now,
                pointer_config,
                pointer_theme,
                pointer_media,
                output_scale,
            )?,
            CursorImage::ClientSurface {
                surface,
                hotspot_x,
                hotspot_y,
            } => match resolve_pointer(
                PointerRequest::ClientSurface,
                pointer_config.client_cursor_mode(),
                pointer_config.pointer_overrides(),
                pointer_theme,
            ) {
                PointerResolution::ClientSurface => windows
                    .get(&surface)
                    .filter(|cursor| !cursor.presentation.pixels.is_empty())
                    .map(|cursor| {
                        CursorVisual::Image(client_cursor_image(cursor, hotspot_x, hotspot_y))
                    }),
                PointerResolution::Graphic(graphic) => Some(CursorVisual::Image(
                    render_asset_pointer(graphic, extent, now, pointer_media, output_scale)?,
                )),
                PointerResolution::System(_) => {
                    return Err(AppError::new(
                        "validated cursor theme is missing the normal pointer",
                    ));
                }
                PointerResolution::Hidden => None,
            },
            CursorImage::Hidden => None,
        };
    Ok(rendered)
}

fn ready_cursor_image(
    image: CursorImage,
    windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
) -> CursorImage {
    match image {
        CursorImage::ClientSurface { surface, .. }
            if windows
                .get(&surface)
                .is_none_or(|w| w.presentation.pixels.is_empty()) =>
        {
            CursorImage::TelorgonDefault
        }
        _ => image,
    }
}

fn client_cursor_image(cursor: &ClientWindow, hotspot_x: i32, hotspot_y: i32) -> RenderedCursor {
    let density = cursor.surface_scale.max(1);
    RenderedCursor {
        rgba: client_pixels_rgba(cursor),
        size: cursor.presentation.image_size,
        logical_size: SizeI {
            width: (cursor.presentation.size.width / density).max(1),
            height: (cursor.presentation.size.height / density).max(1),
        },
        hotspot: PointI {
            x: hotspot_x / density,
            y: hotspot_y / density,
        },
        premultiplied: true,
    }
}

fn client_pixels_rgba(window: &ClientWindow) -> Vec<u8> {
    let mut rgba = window.presentation.pixels.clone();
    if window.presentation.pixel_format == ImagePixelFormat::Bgra8 {
        for pixel in rgba.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
    }
    if window.presentation.alpha_mode == ImageAlphaMode::Opaque {
        for pixel in rgba.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
    }
    rgba
}

#[allow(clippy::too_many_arguments)]
fn render_semantic_pointer(
    icon: PointerIcon,
    _composed_icon_name: Option<&str>,
    _pointer: &mut Option<Layer>,
    _icons: &mut [(String, Layer)],
    extent: SizeI,
    now: u64,
    pointer_config: &PointerConfiguration,
    pointer_theme: Option<&PointerTheme>,
    pointer_media: &mut AssetMediaCache,
    output_scale: crate::platform::contracts::ScaleFactor,
) -> AppResult<Option<CursorVisual>> {
    match resolve_pointer(
        PointerRequest::Semantic(icon),
        pointer_config.client_cursor_mode(),
        pointer_config.pointer_overrides(),
        pointer_theme,
    ) {
        PointerResolution::Graphic(graphic) => Ok(Some(CursorVisual::Image(render_asset_pointer(
            graphic,
            extent,
            now,
            pointer_media,
            output_scale,
        )?))),
        PointerResolution::System(icon) => Err(AppError::new(format!(
            "validated cursor theme is missing `{}`",
            icon.name()
        ))),
        PointerResolution::Hidden => Ok(None),
        PointerResolution::ClientSurface => {
            unreachable!("semantic requests cannot resolve to a client surface")
        }
    }
}

fn render_asset_pointer(
    graphic: &CursorGraphic,
    fallback_extent: SizeI,
    now_nanoseconds: u64,
    media: &mut AssetMediaCache,
    output_scale: crate::platform::contracts::ScaleFactor,
) -> AppResult<RenderedCursor> {
    let frame = if graphic.frames().len() == 1 {
        graphic.frames()[0]
    } else {
        let cycle_ms = graphic
            .frames()
            .iter()
            .filter_map(|frame| frame.duration_ms)
            .map(|duration| u64::from(duration.get()))
            .sum::<u64>();
        let mut elapsed_ms = (now_nanoseconds / 1_000_000) % cycle_ms.max(1);
        *graphic
            .frames()
            .iter()
            .find(|frame| {
                let duration = u64::from(
                    frame
                        .duration_ms
                        .expect("animated frame was validated")
                        .get(),
                );
                if elapsed_ms < duration {
                    true
                } else {
                    elapsed_ms -= duration;
                    false
                }
            })
            .unwrap_or_else(|| {
                graphic
                    .frames()
                    .last()
                    .expect("pointer graphic has a frame")
            })
    };
    let logical_size = graphic
        .logical_size()
        .map_or(fallback_extent, |size| SizeI {
            width: i32::from(size),
            height: i32::from(size),
        });
    let exact_size = graphic
        .exact_logical_size()
        .unwrap_or(logical_size.width as f32);
    let requested = AssetRasterSize::new(
        (exact_size * output_scale.get()).round().max(1.0) as u32,
        (exact_size * output_scale.get()).round().max(1.0) as u32,
    )
    .map_err(app_error)?;
    let decoded = match graphic.tint_color() {
        Some(tint) => media.tinted_cursor(frame.asset, Some(requested), tint),
        None => media.cursor(frame.asset, Some(requested)),
    }
    .map_err(app_error)?;
    let hotspot = graphic.pointer_hotspot();
    if i32::from(hotspot.x) >= logical_size.width || i32::from(hotspot.y) >= logical_size.height {
        return Err(AppError::new(
            "pointer hotspot is outside the decoded cursor image",
        ));
    }
    Ok(RenderedCursor {
        rgba: decoded.pixels_rgba8.to_vec(),
        size: decoded.extent,
        logical_size,
        hotspot: PointI {
            x: i32::from(hotspot.x),
            y: i32::from(hotspot.y),
        },
        premultiplied: decoded.alpha_mode == ImageAlphaMode::Premultiplied,
    })
}

pub(super) fn cursor_image_signature(cursor: &RenderedCursor) -> u64 {
    let mut hasher = DefaultHasher::new();
    cursor.size.width.hash(&mut hasher);
    cursor.size.height.hash(&mut hasher);
    cursor.logical_size.width.hash(&mut hasher);
    cursor.logical_size.height.hash(&mut hasher);
    cursor.hotspot.x.hash(&mut hasher);
    cursor.hotspot.y.hash(&mut hasher);
    cursor.premultiplied.hash(&mut hasher);
    cursor.rgba.hash(&mut hasher);
    hasher.finish()
}

pub(super) fn pointer_request_cursor_image(request: PointerRequest) -> CursorImage {
    match request {
        PointerRequest::Hidden => CursorImage::Hidden,
        PointerRequest::ClientSurface => CursorImage::TelorgonDefault,
        PointerRequest::Semantic(icon) => CursorImage::Shape(pointer_icon_cursor_shape(icon)),
    }
}

pub(super) fn cursor_transition_requires_presentation(
    previous: CursorImage,
    current: CursorImage,
) -> bool {
    previous != current
}

pub(super) fn resize_edge_pointer_icon(edge: ResizeEdge) -> PointerIcon {
    match edge {
        ResizeEdge::None => PointerIcon::Default,
        ResizeEdge::Top => PointerIcon::NResize,
        ResizeEdge::TopRight => PointerIcon::NeResize,
        ResizeEdge::Right => PointerIcon::EResize,
        ResizeEdge::BottomRight => PointerIcon::SeResize,
        ResizeEdge::Bottom => PointerIcon::SResize,
        ResizeEdge::BottomLeft => PointerIcon::SwResize,
        ResizeEdge::Left => PointerIcon::WResize,
        ResizeEdge::TopLeft => PointerIcon::NwResize,
    }
}

pub(super) fn cursor_shape_icon_name(shape: u32) -> Option<&'static str> {
    Some(match shape {
        1 => "cursor.default",
        2 => "cursor.context-menu",
        3 => "cursor.help",
        4 => "cursor.pointer",
        5 => "cursor.progress",
        6 => "cursor.wait",
        7 => "cursor.cell",
        8 => "cursor.crosshair",
        9 => "cursor.text",
        10 => "cursor.vertical-text",
        11 => "cursor.alias",
        12 => "cursor.copy",
        13 => "cursor.move",
        14 => "cursor.no-drop",
        15 => "cursor.not-allowed",
        16 => "cursor.grab",
        17 => "cursor.grabbing",
        18 => "cursor.e-resize",
        19 => "cursor.n-resize",
        20 => "cursor.ne-resize",
        21 => "cursor.nw-resize",
        22 => "cursor.s-resize",
        23 => "cursor.se-resize",
        24 => "cursor.sw-resize",
        25 => "cursor.w-resize",
        26 => "cursor.ew-resize",
        27 => "cursor.ns-resize",
        28 => "cursor.nesw-resize",
        29 => "cursor.nwse-resize",
        30 => "cursor.col-resize",
        31 => "cursor.row-resize",
        32 => "cursor.all-scroll",
        33 => "cursor.zoom-in",
        34 => "cursor.zoom-out",
        35 => "cursor.dnd-ask",
        36 => "cursor.all-resize",
        _ => return None,
    })
}

pub(super) fn cursor_shape_pointer_icon(shape: u32) -> Option<PointerIcon> {
    Some(match shape {
        1 => PointerIcon::Default,
        2 => PointerIcon::ContextMenu,
        3 => PointerIcon::Help,
        4 => PointerIcon::Pointer,
        5 => PointerIcon::Progress,
        6 => PointerIcon::Wait,
        7 => PointerIcon::Cell,
        8 => PointerIcon::Crosshair,
        9 => PointerIcon::Text,
        10 => PointerIcon::VerticalText,
        11 => PointerIcon::Alias,
        12 => PointerIcon::Copy,
        13 => PointerIcon::Move,
        14 => PointerIcon::NoDrop,
        15 => PointerIcon::NotAllowed,
        16 => PointerIcon::Grab,
        17 => PointerIcon::Grabbing,
        18 => PointerIcon::EResize,
        19 => PointerIcon::NResize,
        20 => PointerIcon::NeResize,
        21 => PointerIcon::NwResize,
        22 => PointerIcon::SResize,
        23 => PointerIcon::SeResize,
        24 => PointerIcon::SwResize,
        25 => PointerIcon::WResize,
        26 => PointerIcon::EwResize,
        27 => PointerIcon::NsResize,
        28 => PointerIcon::NeswResize,
        29 => PointerIcon::NwseResize,
        30 => PointerIcon::ColResize,
        31 => PointerIcon::RowResize,
        32 => PointerIcon::AllScroll,
        33 => PointerIcon::ZoomIn,
        34 => PointerIcon::ZoomOut,
        35 => PointerIcon::DndAsk,
        36 => PointerIcon::AllResize,
        _ => return None,
    })
}

pub(super) fn pointer_icon_cursor_shape(icon: PointerIcon) -> u32 {
    match icon {
        PointerIcon::Default => 1,
        PointerIcon::ContextMenu => 2,
        PointerIcon::Help => 3,
        PointerIcon::Pointer => 4,
        PointerIcon::Progress => 5,
        PointerIcon::Wait => 6,
        PointerIcon::Cell => 7,
        PointerIcon::Crosshair => 8,
        PointerIcon::Text => 9,
        PointerIcon::VerticalText => 10,
        PointerIcon::Alias => 11,
        PointerIcon::Copy => 12,
        PointerIcon::Move => 13,
        PointerIcon::NoDrop => 14,
        PointerIcon::NotAllowed => 15,
        PointerIcon::Grab => 16,
        PointerIcon::Grabbing => 17,
        PointerIcon::EResize => 18,
        PointerIcon::NResize => 19,
        PointerIcon::NeResize => 20,
        PointerIcon::NwResize => 21,
        PointerIcon::SResize => 22,
        PointerIcon::SeResize => 23,
        PointerIcon::SwResize => 24,
        PointerIcon::WResize => 25,
        PointerIcon::EwResize => 26,
        PointerIcon::NsResize => 27,
        PointerIcon::NeswResize => 28,
        PointerIcon::NwseResize => 29,
        PointerIcon::ColResize => 30,
        PointerIcon::RowResize => 31,
        PointerIcon::AllScroll => 32,
        PointerIcon::ZoomIn => 33,
        PointerIcon::ZoomOut => 34,
        PointerIcon::DndAsk => 35,
        PointerIcon::AllResize => 36,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn x11_dense_cursor_keeps_native_pixels_and_logical_hotspot() {
        let mut window = super::super::client::maximize_preview_tests::test_window(
            SizeI {
                width: 72,
                height: 72,
            },
            PointI::default(),
        );
        window.surface_scale = 3;
        window.presentation.image_size = SizeI {
            width: 72,
            height: 72,
        };
        window.presentation.pixels = vec![255; 72 * 72 * 4];
        // One-pixel detail must not be lost to a 24x24 intermediate.
        window.presentation.pixels[0] = 0;
        let cursor = client_cursor_image(&window, 15, 9);
        assert_eq!(
            cursor.logical_size,
            SizeI {
                width: 24,
                height: 24
            }
        );
        assert_eq!(cursor.hotspot, PointI { x: 5, y: 3 });
        let physical = cursor
            .for_hardware(
                crate::platform::contracts::ScaleFactor::new(3.0).unwrap(),
                SizeI {
                    width: 256,
                    height: 256,
                },
            )
            .unwrap();
        assert_eq!(
            physical.size,
            SizeI {
                width: 72,
                height: 72
            }
        );
        assert_eq!(physical.hotspot, PointI { x: 15, y: 9 });
        assert_eq!(physical.rgba, cursor.rgba);
    }
    #[test]
    fn unpublished_cursor_uses_default_but_explicit_hidden_cursor_stays_hidden() {
        let surface = WaylandSurfaceId::from_raw(42).unwrap();
        let requested = CursorImage::ClientSurface {
            surface,
            hotspot_x: 1,
            hotspot_y: 2,
        };
        let mut windows = BTreeMap::new();
        assert_eq!(
            ready_cursor_image(requested, &windows),
            CursorImage::TelorgonDefault
        );
        assert_eq!(
            ready_cursor_image(CursorImage::Hidden, &windows),
            CursorImage::Hidden
        );
        let mut window = super::super::client::maximize_preview_tests::test_window(
            SizeI {
                width: 1,
                height: 1,
            },
            PointI::default(),
        );
        windows.insert(surface, window);
        assert_eq!(
            ready_cursor_image(requested, &windows),
            CursorImage::TelorgonDefault
        );
        window = windows.remove(&surface).unwrap();
        // Transparent pixels are a valid deliberate client cursor, not missing data.
        window.presentation.pixels = vec![0; 4];
        windows.insert(surface, window);
        assert_eq!(ready_cursor_image(requested, &windows), requested);
    }
    #[test]
    fn cursor_hotspot_and_pixels_follow_output_density_with_bounded_hardware_fallback() {
        let cursor = RenderedCursor {
            rgba: vec![255; 2 * 2 * 4],
            size: SizeI {
                width: 2,
                height: 2,
            },
            logical_size: SizeI {
                width: 2,
                height: 2,
            },
            hotspot: PointI { x: 1, y: 1 },
            premultiplied: true,
        };
        for factor in [1.0, 1.5, 2.0] {
            let scale = crate::platform::contracts::ScaleFactor::new(factor).unwrap();
            let dense = cursor
                .for_hardware(
                    scale,
                    SizeI {
                        width: 4,
                        height: 4,
                    },
                )
                .unwrap();
            assert_eq!(dense.size.width, (2.0 * factor) as i32);
            assert_eq!(dense.hotspot.x, factor.round() as i32);
            assert_eq!(
                dense.rgba,
                vec![255; dense.size.width as usize * dense.size.height as usize * 4]
            );
            assert_eq!(dense.logical_size, cursor.logical_size);
        }
        assert!(
            cursor
                .for_hardware(
                    crate::platform::contracts::ScaleFactor::new(2.0).unwrap(),
                    SizeI {
                        width: 3,
                        height: 3
                    }
                )
                .is_err()
        );
        let missing_pixels = RenderedCursor {
            rgba: Vec::new(),
            ..cursor
        };
        assert!(
            missing_pixels
                .for_hardware(
                    Default::default(),
                    SizeI {
                        width: 4,
                        height: 4
                    }
                )
                .is_err()
        );
    }
}

#[cfg(test)]
mod theme_render_tests {
    use super::*;

    #[test]
    fn theme_cursor_raster_and_hotspot_follow_output_density() {
        let assets = crate::assets::cursor_test_bundle();
        let config = crate::assets::cursor_test_theme()
            .prepare(assets, crate::ClientCursorMode::Allow)
            .unwrap();
        let graphic = config
            .pointer_overrides()
            .graphic(PointerIcon::Default)
            .unwrap();
        let mut media = AssetMediaCache::new(assets).unwrap();
        let scale = crate::platform::contracts::ScaleFactor::new(2.0).unwrap();
        let rendered = render_asset_pointer(
            graphic,
            SizeI {
                width: 99,
                height: 99,
            },
            0,
            &mut media,
            scale,
        )
        .unwrap();
        assert_eq!(
            rendered.logical_size,
            SizeI {
                width: 24,
                height: 24
            }
        );
        assert_eq!(
            rendered.size,
            SizeI {
                width: 48,
                height: 48
            }
        );
        assert_eq!(rendered.hotspot, PointI { x: 12, y: 12 });
        let hardware = rendered
            .for_hardware(
                scale,
                SizeI {
                    width: 64,
                    height: 64,
                },
            )
            .unwrap();
        assert_eq!(hardware.hotspot, PointI { x: 24, y: 24 });
    }
}

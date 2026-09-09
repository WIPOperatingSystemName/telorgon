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
        scale: crate::platform::ScaleFactor,
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

#[derive(Clone, Debug)]
pub(super) enum ComposedCursorSource {
    Pointer,
    Icon(usize),
}

pub(super) enum CursorVisual {
    Image(RenderedCursor),
    Composed {
        source: ComposedCursorSource,
        size: SizeI,
    },
}

impl CursorVisual {
    pub(super) fn image(&self) -> Option<&RenderedCursor> {
        match self {
            Self::Image(image) => Some(image),
            Self::Composed { .. } => None,
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
    output_scale: crate::platform::ScaleFactor,
) -> AppResult<Option<CursorVisual>> {
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
                        CursorVisual::Image(RenderedCursor {
                            rgba: client_pixels_rgba(cursor),
                            size: cursor.presentation.image_size,
                            logical_size: cursor.presentation.size,
                            hotspot: PointI {
                                x: hotspot_x,
                                y: hotspot_y,
                            },
                            premultiplied: true,
                        })
                    }),
                PointerResolution::Graphic(graphic) => Some(CursorVisual::Image(
                    render_asset_pointer(graphic, extent, now, pointer_media, output_scale)?,
                )),
                PointerResolution::System(_) => {
                    render_composed_pointer(None, pointer, icons, extent)
                }
                PointerResolution::Hidden => None,
            },
            CursorImage::Hidden => None,
        };
    Ok(rendered)
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
    composed_icon_name: Option<&str>,
    pointer: &mut Option<Layer>,
    icons: &mut [(String, Layer)],
    extent: SizeI,
    now: u64,
    pointer_config: &PointerConfiguration,
    pointer_theme: Option<&PointerTheme>,
    pointer_media: &mut AssetMediaCache,
    output_scale: crate::platform::ScaleFactor,
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
        PointerResolution::System(icon) => {
            let rendered = render_composed_pointer(composed_icon_name, pointer, icons, extent);
            if rendered.is_some() {
                return Ok(rendered);
            }
            let Some(fallback) = semantic_pointer_fallback(icon) else {
                return Ok(None);
            };
            render_semantic_pointer(
                fallback,
                cursor_shape_icon_name(pointer_icon_cursor_shape(fallback)),
                pointer,
                icons,
                extent,
                now,
                pointer_config,
                pointer_theme,
                pointer_media,
                output_scale,
            )
        }
        PointerResolution::Hidden => Ok(None),
        PointerResolution::ClientSurface => {
            unreachable!("semantic requests cannot resolve to a client surface")
        }
    }
}

pub(super) fn semantic_pointer_fallback(icon: PointerIcon) -> Option<PointerIcon> {
    match icon {
        PointerIcon::EResize
        | PointerIcon::NResize
        | PointerIcon::NeResize
        | PointerIcon::NwResize
        | PointerIcon::SResize
        | PointerIcon::SeResize
        | PointerIcon::SwResize
        | PointerIcon::WResize
        | PointerIcon::EwResize
        | PointerIcon::NsResize
        | PointerIcon::NeswResize
        | PointerIcon::NwseResize
        | PointerIcon::ColResize
        | PointerIcon::RowResize => Some(PointerIcon::AllResize),
        PointerIcon::Default => None,
        _ => Some(PointerIcon::Default),
    }
}

fn render_asset_pointer(
    graphic: &PointerGraphic,
    fallback_extent: SizeI,
    now_nanoseconds: u64,
    media: &mut AssetMediaCache,
    output_scale: crate::platform::ScaleFactor,
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
    let requested = AssetRasterSize::new(
        (logical_size.width as f32 * output_scale.get())
            .round()
            .max(1.0) as u32,
        (logical_size.height as f32 * output_scale.get())
            .round()
            .max(1.0) as u32,
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

fn render_composed_pointer(
    composed_icon_name: Option<&str>,
    pointer: &mut Option<Layer>,
    icons: &mut [(String, Layer)],
    extent: SizeI,
) -> Option<CursorVisual> {
    if let Some(index) = composed_icon_name
        .and_then(|name| icons.iter().position(|(candidate, _)| candidate == name))
    {
        return Some(CursorVisual::Composed {
            source: ComposedCursorSource::Icon(index),
            size: extent,
        });
    }
    pointer.as_ref().map(|_| CursorVisual::Composed {
        source: ComposedCursorSource::Pointer,
        size: extent,
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
            let scale = crate::platform::ScaleFactor::new(factor).unwrap();
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
                    crate::platform::ScaleFactor::new(2.0).unwrap(),
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

use super::*;

pub(super) fn retained_requested_size(previous: Option<SizeI>, committed: SizeI) -> SizeI {
    previous.unwrap_or(committed)
}

pub(super) fn rounded_pointer_delta(start: PointF, current: PointF) -> PointI {
    PointI {
        x: (current.x - start.x).round() as i32,
        y: (current.y - start.y).round() as i32,
    }
}

pub(super) fn resize_drag_geometry(
    position_start: PointI,
    size_start: SizeI,
    edge: ResizeEdge,
    delta: PointI,
    output: SizeI,
) -> (PointI, SizeI) {
    const MINIMUM_WIDTH: i32 = 1;
    const MINIMUM_HEIGHT: i32 = 1;

    let mut position = position_start;
    let mut size = size_start;
    let maximum_width = output.width.max(MINIMUM_WIDTH);
    let maximum_height = output.height.max(MINIMUM_HEIGHT);
    if matches!(
        edge,
        ResizeEdge::Left | ResizeEdge::TopLeft | ResizeEdge::BottomLeft
    ) {
        size.width = size_start
            .width
            .saturating_sub(delta.x)
            .clamp(MINIMUM_WIDTH, maximum_width);
        position.x = position_start
            .x
            .saturating_add(size_start.width.saturating_sub(size.width));
    }
    if matches!(
        edge,
        ResizeEdge::Right | ResizeEdge::TopRight | ResizeEdge::BottomRight
    ) {
        size.width = size_start
            .width
            .saturating_add(delta.x)
            .clamp(MINIMUM_WIDTH, maximum_width);
    }
    if matches!(
        edge,
        ResizeEdge::Top | ResizeEdge::TopLeft | ResizeEdge::TopRight
    ) {
        size.height = size_start
            .height
            .saturating_sub(delta.y)
            .clamp(MINIMUM_HEIGHT, maximum_height);
        position.y = position_start
            .y
            .saturating_add(size_start.height.saturating_sub(size.height));
    }
    if matches!(
        edge,
        ResizeEdge::Bottom | ResizeEdge::BottomLeft | ResizeEdge::BottomRight
    ) {
        size.height = size_start
            .height
            .saturating_add(delta.y)
            .clamp(MINIMUM_HEIGHT, maximum_height);
    }
    (position, size)
}

pub(super) fn window_content_offset(window: &ClientWindow, config: &LinuxDesktopConfig) -> PointI {
    if !window_has_frame(window) {
        PointI::default()
    } else {
        window.chrome_content_offset.unwrap_or(PointI {
            x: config.window_border,
            y: config.window_border
                + if window_is_decorated(window) {
                    config.titlebar_height
                } else {
                    0
                },
        })
    }
}

pub(super) fn window_content_rect(
    window: &ClientWindow,
    position: PointI,
    config: &LinuxDesktopConfig,
) -> RectI {
    content_rect(
        position,
        window_content_offset(window, config),
        window.requested_size,
    )
}

fn content_rect(position: PointI, offset: PointI, size: SizeI) -> RectI {
    RectI {
        x: position.x.saturating_add(offset.x),
        y: position.y.saturating_add(offset.y),
        width: size.width,
        height: size.height,
    }
}

pub(super) fn legacy_window_outer(window: &ClientWindow, config: &LinuxDesktopConfig) -> SizeI {
    SizeI {
        width: window.requested_size.width + config.window_border * 2,
        height: window.requested_size.height
            + config.window_border * 2
            + if window_is_decorated(window) {
                config.titlebar_height
            } else {
                0
            },
    }
}

pub(super) fn wayland_resize_edge(edge: WindowResizeEdge) -> ResizeEdge {
    match edge {
        WindowResizeEdge::Top => ResizeEdge::Top,
        WindowResizeEdge::TopRight => ResizeEdge::TopRight,
        WindowResizeEdge::Right => ResizeEdge::Right,
        WindowResizeEdge::BottomRight => ResizeEdge::BottomRight,
        WindowResizeEdge::Bottom => ResizeEdge::Bottom,
        WindowResizeEdge::BottomLeft => ResizeEdge::BottomLeft,
        WindowResizeEdge::Left => ResizeEdge::Left,
        WindowResizeEdge::TopLeft => ResizeEdge::TopLeft,
    }
}

/// Frame appearance and application title-bar ownership are independent for managed X11 windows.
pub(super) fn window_has_frame(window: &ClientWindow) -> bool {
    if window.fullscreen {
        return false;
    }
    #[cfg(all(feature = "desktop-xwayland", target_env = "gnu"))]
    if matches!(window.backend, Some(WindowBackend::X11(_))) {
        return true;
    }
    window_is_decorated(window)
}

pub(super) fn window_is_decorated(window: &ClientWindow) -> bool {
    window.server_decorated && !window.fullscreen
}

pub(super) fn surface_local_position(
    windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
    surface: WaylandSurfaceId,
    position: PointF,
    config: &LinuxDesktopConfig,
) -> PointF {
    let Some(window) = windows.get(&surface) else {
        return position;
    };
    surface_placement(window, window.position, config).surface_local(
        window
            .motion_input
            .map_or(position, |input| input.map(position)),
    )
}

pub(super) fn surface_placement(
    window: &ClientWindow,
    position: PointI,
    config: &LinuxDesktopConfig,
) -> SurfacePlacement {
    if window.role == SurfaceRole::XdgToplevel {
        return SurfacePlacement::toplevel(
            window.presentation.size,
            window.window_geometry,
            window_content_rect(window, position, config),
            window.native_configure.resize_anchor,
        );
    }
    let offset = window_content_offset(window, config);
    let origin = PointI {
        x: position.x.saturating_add(offset.x),
        y: position.y.saturating_add(offset.y),
    };
    let density = window.surface_scale.max(1);
    let mut placement = SurfacePlacement::native(
        SizeI {
            width: (window.presentation.size.width / density).max(1),
            height: (window.presentation.size.height / density).max(1),
        },
        origin,
    );
    placement.surface_scale = density;
    placement
}

pub(super) fn constrain_pointer(
    windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
    current: PointF,
    proposed: PointF,
    constraint: &PointerConstraintState,
    config: &LinuxDesktopConfig,
) -> PointF {
    let Some(window) = windows.get(&constraint.surface) else {
        return current;
    };
    let placement = surface_placement(window, window.position, config);
    let local = placement.surface_local(proposed);
    let Some(visible) = placement.visible_rect() else {
        return current;
    };
    let surface = RectI {
        x: visible
            .x
            .saturating_sub(placement.target.x)
            .saturating_mul(placement.surface_scale),
        y: visible
            .y
            .saturating_sub(placement.target.y)
            .saturating_mul(placement.surface_scale),
        width: visible.width.saturating_mul(placement.surface_scale),
        height: visible.height.saturating_mul(placement.surface_scale),
    };
    let regions = constraint.region.as_ref().map_or_else(
        || vec![surface],
        |region| {
            region
                .rectangles()
                .iter()
                .filter_map(|rectangle| intersect_rect(*rectangle, surface))
                .collect()
        },
    );
    let Some(nearest) = regions
        .iter()
        .map(|rectangle| {
            let maximum_x = (rectangle.x + rectangle.width) as f32 - 0.001;
            let maximum_y = (rectangle.y + rectangle.height) as f32 - 0.001;
            let point = PointF {
                x: local.x.clamp(rectangle.x as f32, maximum_x),
                y: local.y.clamp(rectangle.y as f32, maximum_y),
            };
            let distance_x = local.x - point.x;
            let distance_y = local.y - point.y;
            (distance_x * distance_x + distance_y * distance_y, point)
        })
        .min_by(|left, right| left.0.total_cmp(&right.0))
        .map(|(_, point)| point)
    else {
        return current;
    };
    placement.output_position(nearest)
}

pub(super) fn intersect_rect(left: RectI, right: RectI) -> Option<RectI> {
    let x = left.x.max(right.x);
    let y = left.y.max(right.y);
    let right_edge = (left.x + left.width).min(right.x + right.width);
    let bottom_edge = (left.y + left.height).min(right.y + right.height);
    (right_edge > x && bottom_edge > y).then_some(RectI {
        x,
        y,
        width: right_edge - x,
        height: bottom_edge - y,
    })
}

pub(super) fn full_rect(size: SizeI) -> RectI {
    RectI {
        x: 0,
        y: 0,
        width: size.width,
        height: size.height,
    }
}

pub(super) fn union_rect(left: RectI, right: RectI) -> RectI {
    let x = left.x.min(right.x);
    let y = left.y.min(right.y);
    let right_edge = left.right().max(right.right());
    let bottom = left.bottom().max(right.bottom());
    RectI {
        x,
        y,
        width: right_edge.saturating_sub(x),
        height: bottom.saturating_sub(y),
    }
}

pub(super) fn union_surface_damage(damage: &[RectI], extent: SizeI) -> Option<RectI> {
    damage
        .iter()
        .filter_map(|rect| intersect_rect(*rect, full_rect(extent)))
        .reduce(union_rect)
}

#[cfg(feature = "profiler")]
pub(super) fn rect_area(rect: RectI) -> u64 {
    u64::try_from(rect.width)
        .ok()
        .and_then(|width| {
            u64::try_from(rect.height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .unwrap_or(0)
}

/// Returns the damage that must be applied to bring a retained scanout target from
/// `previous_version` to `current_version`. `None` deliberately means a full redraw.
pub(super) fn accumulated_damage(
    previous_version: u64,
    current_version: u64,
    history: &VecDeque<(u64, Option<RectI>)>,
    extent: SizeI,
) -> Option<RectI> {
    if previous_version == 0 || previous_version >= current_version {
        return None;
    }
    let oldest_version = history.front().map(|(version, _)| *version)?;
    if previous_version.saturating_add(1) < oldest_version {
        return None;
    }
    let mut combined = None::<RectI>;
    for (_, damage) in history
        .iter()
        .filter(|(version, _)| *version > previous_version && *version <= current_version)
    {
        let Some(damage) = damage else {
            return None;
        };
        combined = Some(combined.map_or(*damage, |old| union_rect(old, *damage)));
    }
    combined.and_then(|damage| intersect_rect(damage, full_rect(extent)))
}

pub(super) fn shell_work_area(output: SizeI, widgets: &[WidgetLayer]) -> RectI {
    let mut area = RectI {
        x: 0,
        y: 0,
        width: output.width,
        height: output.height,
    };
    for widget in widgets {
        let reserved = widget.reserved_space.max(0);
        match widget.anchor {
            ShellWidgetAnchor::Top => {
                let reserved = reserved.min(area.height.saturating_sub(1));
                area.y = area.y.saturating_add(reserved);
                area.height = area.height.saturating_sub(reserved);
            }
            ShellWidgetAnchor::Right | ShellWidgetAnchor::Bottom => {
                let extent = if widget.anchor == ShellWidgetAnchor::Right {
                    &mut area.width
                } else {
                    &mut area.height
                };
                let reserved = reserved.min(extent.saturating_sub(1));
                *extent = extent.saturating_sub(reserved);
            }
            ShellWidgetAnchor::Left => {
                let reserved = reserved.min(area.width.saturating_sub(1));
                area.x = area.x.saturating_add(reserved);
                area.width = area.width.saturating_sub(reserved);
            }
            ShellWidgetAnchor::Floating => {}
        }
    }
    area
}

pub(super) fn resolved_widget_extent(
    output: SizeI,
    width: ShellWidgetExtent,
    height: ShellWidgetExtent,
) -> SizeI {
    SizeI {
        width: match width {
            ShellWidgetExtent::Fill => output.width,
            ShellWidgetExtent::Pixels(value) => value.round().max(1.0) as i32,
        },
        height: match height {
            ShellWidgetExtent::Fill => output.height,
            ShellWidgetExtent::Pixels(value) => value.round().max(1.0) as i32,
        },
    }
}

pub(super) fn widget_position(output: SizeI, widget: SizeI, anchor: ShellWidgetAnchor) -> PointI {
    match anchor {
        ShellWidgetAnchor::Top | ShellWidgetAnchor::Left => PointI::default(),
        ShellWidgetAnchor::Right => PointI {
            x: output.width - widget.width,
            y: 0,
        },
        ShellWidgetAnchor::Bottom => PointI {
            x: 0,
            y: output.height - widget.height,
        },
        ShellWidgetAnchor::Floating => PointI {
            x: (output.width - widget.width) / 2,
            y: (output.height - widget.height) / 2,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compositor_content_clip_is_independent_of_client_buffer_margins() {
        assert_eq!(
            content_rect(
                PointI { x: 100, y: 80 },
                PointI { x: 4, y: 28 },
                SizeI {
                    width: 900,
                    height: 600,
                },
            ),
            RectI {
                x: 104,
                y: 108,
                width: 900,
                height: 600,
            }
        );
    }

    #[test]
    fn live_resize_geometry_is_consistent_for_every_edge_and_corner() {
        let start = PointI { x: 100, y: 80 };
        let size = SizeI {
            width: 400,
            height: 300,
        };
        let delta = PointI { x: 50, y: 40 };
        let output = SizeI {
            width: 1920,
            height: 1080,
        };
        let cases = [
            (
                ResizeEdge::Top,
                PointI { x: 100, y: 120 },
                SizeI {
                    width: 400,
                    height: 260,
                },
            ),
            (
                ResizeEdge::TopRight,
                PointI { x: 100, y: 120 },
                SizeI {
                    width: 450,
                    height: 260,
                },
            ),
            (
                ResizeEdge::Right,
                PointI { x: 100, y: 80 },
                SizeI {
                    width: 450,
                    height: 300,
                },
            ),
            (
                ResizeEdge::BottomRight,
                PointI { x: 100, y: 80 },
                SizeI {
                    width: 450,
                    height: 340,
                },
            ),
            (
                ResizeEdge::Bottom,
                PointI { x: 100, y: 80 },
                SizeI {
                    width: 400,
                    height: 340,
                },
            ),
            (
                ResizeEdge::BottomLeft,
                PointI { x: 150, y: 80 },
                SizeI {
                    width: 350,
                    height: 340,
                },
            ),
            (
                ResizeEdge::Left,
                PointI { x: 150, y: 80 },
                SizeI {
                    width: 350,
                    height: 300,
                },
            ),
            (
                ResizeEdge::TopLeft,
                PointI { x: 150, y: 120 },
                SizeI {
                    width: 350,
                    height: 260,
                },
            ),
        ];

        for (edge, expected_position, expected_size) in cases {
            let (position, resized) = resize_drag_geometry(start, size, edge, delta, output);
            assert_eq!(
                (position, resized),
                (expected_position, expected_size),
                "{edge:?}"
            );

            if matches!(
                edge,
                ResizeEdge::Left | ResizeEdge::TopLeft | ResizeEdge::BottomLeft
            ) {
                assert_eq!(position.x + resized.width, start.x + size.width, "{edge:?}");
            }
            if matches!(
                edge,
                ResizeEdge::Top | ResizeEdge::TopLeft | ResizeEdge::TopRight
            ) {
                assert_eq!(
                    position.y + resized.height,
                    start.y + size.height,
                    "{edge:?}"
                );
            }
        }
    }
}

/// X11 surface coordinates include the private server's integer density already.
/// Native Wayland surfaces use density one; wl_surface buffer_scale is separate.
pub(super) fn surface_raster_scale(
    output_scale: crate::platform::ScaleFactor,
    coordinate_density: i32,
) -> crate::platform::ScaleFactor {
    crate::platform::ScaleFactor::new(output_scale.get() / coordinate_density.max(1) as f32)
        .expect("validated output scale and positive coordinate density")
}

#[cfg(test)]
mod raster_scale_tests {
    use super::*;

    #[test]
    fn fractional_and_integer_x11_density_does_not_multiply_pixel_size_twice() {
        for (output, density, expected) in
            [(1.0, 1, 1.0), (1.5, 2, 0.75), (2.0, 2, 1.0), (3.0, 3, 1.0)]
        {
            let output = crate::platform::ScaleFactor::new(output).unwrap();
            assert_eq!(surface_raster_scale(output, density).get(), expected);
            assert_eq!(surface_raster_scale(output, 1), output);
        }
    }
}

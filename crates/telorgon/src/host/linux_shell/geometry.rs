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

pub(super) fn window_content_offset(window: &ClientWindow, config: &LinuxShellConfig) -> PointI {
    if !window_has_frame(window) {
        PointI::default()
    } else {
        window.chrome_content_offset.unwrap_or(PointI {
            x: window_border_width(window, config),
            y: window_border_width(window, config)
                + if window_has_titlebar(window) { config.titlebar_height } else { 0 },
        })
    }
}

pub(super) fn window_content_rect(
    window: &ClientWindow,
    position: PointI,
    config: &LinuxShellConfig,
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

pub(super) fn legacy_window_outer(window: &ClientWindow, config: &LinuxShellConfig) -> SizeI {
    SizeI {
        width: window.requested_size.width + window_border_width(window, config) * 2,
        height: window.requested_size.height
            + window_border_width(window, config) * 2
            + if window_has_titlebar(window) {
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

/// Client headers can retain ownership while the shell supplies only the outer frame.
pub(super) fn window_has_frame(window: &ClientWindow) -> bool {
    (window.server_decorated || window.frame_client_decorations)
        && !window.fullscreen && window.backend.is_some()
}

pub(super) fn window_has_titlebar(window: &ClientWindow) -> bool {
    window.server_decorated && window_has_frame(window)
}

pub(super) fn window_border_width(window: &ClientWindow, config: &LinuxShellConfig) -> i32 {
    if window_has_frame(window) {
        config.window_border
    } else {
        0
    }
}

pub(super) fn surface_tree_position(
    windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
    surface: WaylandSurfaceId,
    config: &LinuxShellConfig,
) -> PointI {
    let Some(window) = windows.get(&surface) else {
        return PointI::default();
    };
    let mut chain = Vec::new();
    let mut current = window;
    while let Some(parent) = current.parent.and_then(|id| windows.get(&id)) {
        if chain.len() >= windows.len() {
            return window.position;
        }
        chain.push((current, parent));
        current = parent;
    }
    let mut position = current.position;
    for (child, parent) in chain.into_iter().rev() {
        let origin = if child.role == SurfaceRole::Subsurface {
            // wl_subsurface positions are relative to the parent's surface origin,
            // not its window geometry or compositor frame. This also handles nesting.
            let placement = surface_placement(parent, position, config);
            PointI {
                x: placement.target.x,
                y: placement.target.y,
            }
        } else {
            let offset = if parent.role == SurfaceRole::Xwayland {
                window_content_offset(parent, config)
            } else {
                PointI::default()
            };
            PointI {
                x: position.x.saturating_add(offset.x),
                y: position.y.saturating_add(offset.y),
            }
        };
        position = PointI {
            x: origin.x.saturating_add(child.offset.x),
            y: origin.y.saturating_add(child.offset.y),
        };
    }
    position
}

pub(super) fn surface_local_position(
    windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
    surface: WaylandSurfaceId,
    position: PointF,
    config: &LinuxShellConfig,
) -> PointF {
    let Some(window) = windows.get(&surface) else {
        return position;
    };
    surface_placement(
        window,
        surface_tree_position(windows, surface, config),
        config,
    )
    .surface_local(
        window
            .motion_input
            .map_or(position, |input| input.map(position)),
    )
}

pub(super) fn surface_placement(
    window: &ClientWindow,
    position: PointI,
    config: &LinuxShellConfig,
) -> SurfacePlacement {
    if window.role == SurfaceRole::XdgToplevel {
        let mut placement = SurfacePlacement::toplevel(
            window.presentation.size,
            window.window_geometry,
            window_content_rect(window, position, config),
            window.native_configure.resize_anchor,
        );
        if !window_has_frame(window) && !window.fullscreen {
            // CSD geometry excludes client shadows, but the complete surface still renders.
            placement.clip = None;
        }
        return placement;
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
    config: &LinuxShellConfig,
) -> PointF {
    let Some(window) = windows.get(&constraint.surface) else {
        return current;
    };
    let placement = surface_placement(
        window,
        surface_tree_position(windows, constraint.surface, config),
        config,
    );
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
    // Reservations are distances from output edges. Overlapping panels reserve the union,
    // rather than adding the same edge interval twice.
    let (mut top, mut right, mut bottom, mut left) = (0, 0, 0, 0);
    for widget in widgets {
        let Some((edge, reserved)) = widget.reservation() else {
            continue;
        };
        let current = match edge {
            crate::ShellEdge::Top => &mut top,
            crate::ShellEdge::Right => &mut right,
            crate::ShellEdge::Bottom => &mut bottom,
            crate::ShellEdge::Left => &mut left,
        };
        *current = (*current).max(reserved);
    }
    left = left.min(output.width.saturating_sub(1));
    right = right.min(output.width.saturating_sub(left + 1));
    top = top.min(output.height.saturating_sub(1));
    bottom = bottom.min(output.height.saturating_sub(top + 1));
    RectI {
        x: left,
        y: top,
        width: (output.width - left - right).max(1),
        height: (output.height - top - bottom).max(1),
    }
}
pub(super) fn shell_work_area_for_spec(output: SizeI) -> crate::foundation::RectF {
    crate::foundation::RectF {
        x: 0.0,
        y: 0.0,
        width: output.width as f32,
        height: output.height as f32,
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
    output_scale: crate::platform::contracts::ScaleFactor,
    coordinate_density: i32,
) -> crate::platform::contracts::ScaleFactor {
    crate::platform::contracts::ScaleFactor::new(
        output_scale.get() / coordinate_density.max(1) as f32,
    )
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
            let output = crate::platform::contracts::ScaleFactor::new(output).unwrap();
            assert_eq!(surface_raster_scale(output, density).get(), expected);
            assert_eq!(surface_raster_scale(output, 1), output);
        }
    }
}

#[cfg(test)]
mod surface_tree_tests {
    use super::super::client::maximize_preview_tests::test_window;
    use super::*;

    #[test]
    fn decoration_ownership_preserves_client_shadows_and_clips_server_content() {
        let config = LinuxShellConfig::default();
        let mut window = test_window(
            SizeI { width: 440, height: 340 },
            PointI { x: 100, y: 80 },
        );
        window.window_geometry = RectI { x: 20, y: 20, width: 400, height: 300 };
        window.requested_size = SizeI { width: 400, height: 300 };
        for server_decorated in [false, true, false] {
            window.server_decorated = server_decorated;
            let placement = surface_placement(&window, window.position, &config);
            let content = window_content_rect(&window, window.position, &config);
            assert_eq!(placement.target.x, content.x - 20);
            assert_eq!(placement.target.y, content.y - 20);
            assert_eq!(placement.clip, server_decorated.then_some(content));
            assert_eq!(placement.surface_local(PointF {
                x: content.x as f32, y: content.y as f32,
            }), PointF { x: 20.0, y: 20.0 });
        }
        window.frame_client_decorations = true;
        let content = window_content_rect(&window, window.position, &config);
        assert!(!window_has_titlebar(&window));
        assert_eq!(window_content_offset(&window, &config), PointI {
            x: config.window_border, y: config.window_border,
        });
        assert_eq!(surface_placement(&window, window.position, &config).clip, Some(content));
        window.fullscreen = true;
        assert!(!window_has_frame(&window));
        assert!(surface_placement(&window, window.position, &config).clip.is_some());
    }

    #[test]
    fn subsurfaces_share_parent_surface_origin_for_painting_and_input() {
        let config = LinuxShellConfig::default();
        let root = WaylandSurfaceId::from_raw(30).unwrap();
        let child = WaylandSurfaceId::from_raw(20).unwrap();
        let nested = WaylandSurfaceId::from_raw(10).unwrap();
        let world = test_input_world(&[root, child, nested]);
        for decorated in [false, true] {
            let mut parent = test_window(
                SizeI {
                    width: 400,
                    height: 300,
                },
                PointI { x: 100, y: 80 },
            );
            parent.server_decorated = decorated;
            parent.window_geometry.x = 26;
            parent.window_geometry.y = 26;
            parent.chrome_content_offset = Some(PointI { x: 7, y: 37 });
            let mut content = test_window(
                SizeI {
                    width: 200,
                    height: 150,
                },
                PointI { x: 999, y: 999 },
            );
            content.role = SurfaceRole::Subsurface;
            content.backend = None;
            content.parent = Some(root);
            content.offset = PointI { x: 26, y: 26 };
            let mut descendant = test_window(
                SizeI {
                    width: 50,
                    height: 40,
                },
                PointI { x: 999, y: 999 },
            );
            descendant.role = SurfaceRole::Subsurface;
            descendant.backend = None;
            descendant.parent = Some(child);
            descendant.offset = PointI { x: 11, y: 13 };
            let mut windows =
                BTreeMap::from([(root, parent), (child, content), (nested, descendant)]);
            for moved in [0, 100] {
                windows.get_mut(&root).unwrap().position.x = 100 + moved;
                let expected = PointI {
                    x: 100 + moved + if decorated { 7 } else { 0 },
                    y: 80 + if decorated { 37 } else { 0 },
                };
                assert_eq!(surface_tree_position(&windows, child, &config), expected);
                let expected = PointI {
                    x: expected.x + 11,
                    y: expected.y + 13,
                };
                assert_eq!(surface_tree_position(&windows, nested, &config), expected);
                let pointer = PointF {
                    x: expected.x as f32 + 5.0,
                    y: expected.y as f32 + 6.0,
                };
                assert_eq!(
                    surface_local_position(&windows, nested, pointer, &config),
                    PointF { x: 5.0, y: 6.0 }
                );
                assert_eq!(
                    hit_test_surface(&world, &windows, &[root, child, nested], pointer, &config, false),
                    Some(nested)
                );
            }
        }
    }
}

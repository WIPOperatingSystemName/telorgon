use super::*;

pub(super) fn output_transform_wire(
    transform: crate::integrations::wayland::compositor::OutputTransform,
) -> i32 {
    match transform {
        crate::integrations::wayland::compositor::OutputTransform::Normal => 0,
        crate::integrations::wayland::compositor::OutputTransform::Rotate90 => 1,
        crate::integrations::wayland::compositor::OutputTransform::Rotate180 => 2,
        crate::integrations::wayland::compositor::OutputTransform::Rotate270 => 3,
        crate::integrations::wayland::compositor::OutputTransform::Flipped => 4,
        crate::integrations::wayland::compositor::OutputTransform::Flipped90 => 5,
        crate::integrations::wayland::compositor::OutputTransform::Flipped180 => 6,
        crate::integrations::wayland::compositor::OutputTransform::Flipped270 => 7,
    }
}

pub(super) fn resize_edge(
    value: u32,
) -> Result<crate::integrations::wayland::compositor::ResizeEdge, NativeCompositorError> {
    Ok(match value {
        1 => crate::integrations::wayland::compositor::ResizeEdge::Top,
        2 => crate::integrations::wayland::compositor::ResizeEdge::Bottom,
        4 => crate::integrations::wayland::compositor::ResizeEdge::Left,
        5 => crate::integrations::wayland::compositor::ResizeEdge::TopLeft,
        6 => crate::integrations::wayland::compositor::ResizeEdge::BottomLeft,
        8 => crate::integrations::wayland::compositor::ResizeEdge::Right,
        9 => crate::integrations::wayland::compositor::ResizeEdge::TopRight,
        10 => crate::integrations::wayland::compositor::ResizeEdge::BottomRight,
        _ => return Err(NativeCompositorError::new("invalid xdg resize edge")),
    })
}

#[repr(C)]
struct NativeTimespec {
    seconds: c_long,
    pub(super) nanoseconds: c_long,
}

pub(super) struct MonotonicTimestamp {
    pub(super) seconds: u64,
    pub(super) nanoseconds: u32,
}

#[link(name = "c")]
unsafe extern "C" {
    fn clock_gettime(clock: i32, time: *mut NativeTimespec) -> i32;
}

pub(super) fn monotonic_timestamp() -> Result<MonotonicTimestamp, NativeCompositorError> {
    let mut time = NativeTimespec {
        seconds: 0,
        nanoseconds: 0,
    };
    let result = unsafe { clock_gettime(1, &mut time) };
    if result != 0 || time.seconds < 0 || !(0..1_000_000_000).contains(&time.nanoseconds) {
        return Err(NativeCompositorError::new(
            "CLOCK_MONOTONIC timestamp query failed",
        ));
    }
    Ok(MonotonicTimestamp {
        seconds: time.seconds as u64,
        nanoseconds: time.nanoseconds as u32,
    })
}

pub(super) fn transformed_size(
    size: crate::foundation::SizeI,
    transform: crate::integrations::wayland::compositor::BufferTransform,
) -> crate::foundation::SizeI {
    match transform {
        crate::integrations::wayland::compositor::BufferTransform::Rotate90
        | crate::integrations::wayland::compositor::BufferTransform::Rotate270
        | crate::integrations::wayland::compositor::BufferTransform::Flipped90
        | crate::integrations::wayland::compositor::BufferTransform::Flipped270 => {
            crate::foundation::SizeI {
                width: size.height,
                height: size.width,
            }
        }
        _ => size,
    }
}

pub(super) fn next_nonzero(value: u32) -> Result<u32, NativeCompositorError> {
    value
        .checked_add(1)
        .filter(|value| *value != 0)
        .ok_or_else(|| NativeCompositorError::new("native compositor identity exhausted"))
}

pub(super) fn fixed(value: f32) -> i32 {
    let scaled = f64::from(value) * 256.0;
    scaled.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

pub(super) fn fixed_f64(value: f64) -> i32 {
    (value * 256.0).clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

pub(super) fn rectangles_intersect(left: RectI, right: RectI) -> bool {
    left.x < right.x.saturating_add(right.width)
        && left.x.saturating_add(left.width) > right.x
        && left.y < right.y.saturating_add(right.height)
        && left.y.saturating_add(left.height) > right.y
}

pub(super) fn subtract_rectangle(source: RectI, cut: RectI) -> Vec<RectI> {
    if !rectangles_intersect(source, cut) {
        return vec![source];
    }

    let source_right = source.x.saturating_add(source.width);
    let source_bottom = source.y.saturating_add(source.height);
    let cut_left = cut.x.max(source.x);
    let cut_top = cut.y.max(source.y);
    let cut_right = cut.x.saturating_add(cut.width).min(source_right);
    let cut_bottom = cut.y.saturating_add(cut.height).min(source_bottom);
    let mut result = Vec::with_capacity(4);
    let mut push = |x: i32, y: i32, width: i32, height: i32| {
        if width > 0 && height > 0 {
            result.push(RectI {
                x,
                y,
                width,
                height,
            });
        }
    };

    push(source.x, source.y, source.width, cut_top - source.y);
    push(
        source.x,
        cut_bottom,
        source.width,
        source_bottom - cut_bottom,
    );
    push(source.x, cut_top, cut_left - source.x, cut_bottom - cut_top);
    push(
        cut_right,
        cut_top,
        source_right - cut_right,
        cut_bottom - cut_top,
    );
    result
}

pub(super) fn popup_geometry(
    positioner: crate::integrations::wayland::compositor::XdgPositioner,
) -> RectI {
    let anchor = positioner.anchor_rect;
    let horizontal_center = anchor.x.saturating_add(anchor.width / 2);
    let vertical_center = anchor.y.saturating_add(anchor.height / 2);
    let anchor_point = match positioner.anchor {
        1 => PointI {
            x: horizontal_center,
            y: anchor.y,
        },
        2 => PointI {
            x: horizontal_center,
            y: anchor.y.saturating_add(anchor.height),
        },
        3 => PointI {
            x: anchor.x,
            y: vertical_center,
        },
        4 => PointI {
            x: anchor.x.saturating_add(anchor.width),
            y: vertical_center,
        },
        5 => PointI {
            x: anchor.x,
            y: anchor.y,
        },
        6 => PointI {
            x: anchor.x,
            y: anchor.y.saturating_add(anchor.height),
        },
        7 => PointI {
            x: anchor.x.saturating_add(anchor.width),
            y: anchor.y,
        },
        8 => PointI {
            x: anchor.x.saturating_add(anchor.width),
            y: anchor.y.saturating_add(anchor.height),
        },
        _ => PointI {
            x: horizontal_center,
            y: vertical_center,
        },
    };
    let size = positioner.size;
    let (gravity_x, gravity_y) = match positioner.gravity {
        1 => (-size.width / 2, -size.height),
        2 => (-size.width / 2, 0),
        3 => (-size.width, -size.height / 2),
        4 => (0, -size.height / 2),
        5 => (-size.width, -size.height),
        6 => (-size.width, 0),
        7 => (0, -size.height),
        8 => (0, 0),
        _ => (-size.width / 2, -size.height / 2),
    };
    RectI {
        x: anchor_point
            .x
            .saturating_add(gravity_x)
            .saturating_add(positioner.offset.x),
        y: anchor_point
            .y
            .saturating_add(gravity_y)
            .saturating_add(positioner.offset.y),
        width: size.width,
        height: size.height,
    }
}

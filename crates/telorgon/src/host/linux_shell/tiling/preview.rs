use super::*;

/// Keep the selected region through small pointer jitter. Entry still uses the
/// configured hitboxes; switching or dismissal requires leaving an expanded region.
fn tile_target(
    policy: WindowTiling,
    pointer: PointF,
    output: RectI,
    previous: Option<TileTarget>,
) -> Option<TileTarget> {
    let raw = policy.target(pointer, output);
    let Some(previous) = previous else { return raw };
    let x = pointer.x - output.x as f32;
    let y = pointer.y - output.y as f32;
    let w = output.width as f32;
    let h = output.height as f32;
    if !x.is_finite()
        || !y.is_finite()
        || x < 0.0
        || y < 0.0
        || x > w
        || y > h
        || w < 2.0
        || h < 2.0
    {
        return raw;
    }
    let margin = 8.0_f32.min(w / 4.0).min(h / 4.0);
    let corner = policy.corner_threshold.min(w / 2.0).min(h / 2.0);
    let edge = policy.edge_threshold.min(w / 2.0);
    let middle = !policy.quadrants || (y > corner - margin && y < h - corner + margin);
    let retain = match previous {
        TileTarget::Left => policy.halves && x <= edge + margin && middle,
        TileTarget::Right => policy.halves && x >= w - edge - margin && middle,
        TileTarget::TopLeft => policy.quadrants && x <= corner + margin && y <= corner + margin,
        TileTarget::TopRight => {
            policy.quadrants && x >= w - corner - margin && y <= corner + margin
        }
        TileTarget::BottomLeft => {
            policy.quadrants && x <= corner + margin && y >= h - corner - margin
        }
        TileTarget::BottomRight => {
            policy.quadrants && x >= w - corner - margin && y >= h - corner - margin
        }
    };
    if retain { Some(previous) } else { raw }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SnapTarget {
    Tile(TileTarget),
    Maximize,
}
impl From<TileTarget> for SnapTarget {
    fn from(target: TileTarget) -> Self {
        Self::Tile(target)
    }
}
impl SnapTarget {
    pub(super) fn rect(self, area: RectI, splits: [f32; 3]) -> RectI {
        match self {
            Self::Tile(target) => tile_rect(area, splits, target),
            Self::Maximize => area,
        }
    }
}

pub(super) fn target(
    policy: WindowTiling,
    pointer: PointF,
    output: RectI,
    previous: Option<SnapTarget>,
) -> Option<SnapTarget> {
    let x = pointer.x - output.x as f32;
    let y = pointer.y - output.y as f32;
    let w = output.width as f32;
    let h = output.height as f32;
    if !x.is_finite()
        || !y.is_finite()
        || x < 0.0
        || y < 0.0
        || x > w
        || y > h
        || w < 2.0
        || h < 2.0
    {
        return None;
    }
    let edge = policy.edge_threshold.min(h / 2.0);
    let side = if policy.quadrants {
        policy.corner_threshold.min(w / 2.0).min(h / 2.0)
    } else if policy.halves {
        policy.edge_threshold.min(w / 2.0)
    } else {
        0.0
    };
    let margin = 8.0_f32.min(w / 4.0).min(h / 4.0);
    if previous == Some(SnapTarget::Maximize)
        && y <= edge + margin
        && x > side - margin
        && x < w - side + margin
    {
        return Some(SnapTarget::Maximize);
    }
    let previous_tile = match previous {
        Some(SnapTarget::Tile(t)) => Some(t),
        _ => None,
    };
    // Preserve the corner and side destinations; the remaining top edge maximizes.
    if let Some(tile) = tile_target(policy, pointer, output, previous_tile) {
        return Some(SnapTarget::Tile(tile));
    }
    (y <= edge).then_some(SnapTarget::Maximize)
}

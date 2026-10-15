use crate::foundation::{PointI, RectI, SizeI};

/// Preserve placement where possible after the usable workspace shrinks.
/// Clients with a minimum larger than the workspace retain a reachable top edge.
pub(super) fn fit(position: PointI, outer: SizeI, area: RectI) -> PointI {
    PointI {
        x: position.x.clamp(area.x, area.x.saturating_add(area.width.saturating_sub(outer.width).max(0))),
        y: position.y.clamp(area.y, area.y.saturating_add(area.height.saturating_sub(outer.height).max(0))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shrinking_workspace_keeps_window_footer_inside_reserved_panel_bounds() {
        let area = RectI { x: 0, y: 72, width: 1024, height: 504 };
        assert_eq!(fit(PointI { x: 48, y: 120 }, SizeI { width: 962, height: 500 }, area), PointI { x: 48, y: 76 });
        // A fixed minimum slightly exceeding the available height still puts
        // almost all of the client and its bottom controls on screen.
        assert_eq!(fit(PointI { x: 48, y: 120 }, SizeI { width: 962, height: 508 }, area), PointI { x: 48, y: 72 });
    }
    #[test]
    fn fitting_preserves_visible_placement_and_handles_oversized_clients() {
        let area = RectI { x: 20, y: 40, width: 800, height: 600 };
        let position = PointI { x: 80, y: 100 };
        assert_eq!(fit(position, SizeI { width: 400, height: 300 }, area), position);
        assert_eq!(fit(position, SizeI { width: 1000, height: 900 }, area), PointI { x: 20, y: 40 });
    }
}

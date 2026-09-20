//! Host-owned snap layout and coordinated divider grabs. No GPU ownership lives here.
use super::*;
use crate::compose::{TileTarget, WindowTiling};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TilePlacement {
    pub target: TileTarget,
    pub rect: RectI,
    pub shared_resize: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Divider {
    Vertical,
    Left,
    Right,
}
impl Divider {
    pub fn icon(self) -> PointerIcon {
        if self == Self::Vertical {
            PointerIcon::EwResize
        } else {
            PointerIcon::NsResize
        }
    }
}
#[derive(Clone, Copy, Debug)]
struct DividerGrab {
    divider: Divider,
    start: PointF,
    splits: [f32; 3],
}
pub(super) struct TilingController {
    splits: [f32; 3],
    grab: Option<DividerGrab>,
    reveal: BTreeMap<WaylandSurfaceId, TileTarget>,
    pub policy: Option<WindowTiling>,
    candidate: Option<(WaylandSurfaceId, TileTarget)>,
    area: RectI,
}
impl Default for TilingController {
    fn default() -> Self {
        Self {
            splits: [0.5; 3],
            grab: None,
            reveal: BTreeMap::new(),
            policy: None,
            candidate: None,
            area: RectI {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
        }
    }
}
fn affects(d: Divider, t: TileTarget) -> bool {
    d == Divider::Vertical || t.row().is_some() && t.left() == (d == Divider::Left)
}
fn tile_rect(area: RectI, splits: [f32; 3], target: TileTarget) -> RectI {
    let split_x =
        ((area.width as f32 * splits[0]).round() as i32).clamp(1, (area.width - 1).max(1));
    let (x, width, col) = if target.left() {
        (area.x, split_x, 1)
    } else {
        (area.x + split_x, area.width - split_x, 2)
    };
    let split_y =
        ((area.height as f32 * splits[col]).round() as i32).clamp(1, (area.height - 1).max(1));
    let (y, height) = match target.row() {
        None => (area.y, area.height),
        Some(false) => (area.y, split_y),
        Some(true) => (area.y + split_y, area.height - split_y),
    };
    RectI {
        x,
        y,
        width,
        height,
    }
}
/// Inset only work-area-facing edges; shared partition coordinates stay exact.
fn preview_rect(
    area: RectI,
    splits: [f32; 3],
    target: TileTarget,
    padding: crate::compose::Insets,
) -> RectI {
    let rect = tile_rect(area, splits, target);
    let p = padding.0;
    let inset =
        |distance: f32, available: i32| (distance.round() as i32).clamp(0, available.max(0));
    let left = if target.left() {
        inset(p.left, rect.width - 1)
    } else {
        0
    };
    let right = if !target.left() {
        inset(p.right, rect.width - left - 1)
    } else {
        0
    };
    let top = if target.row() != Some(true) {
        inset(p.top, rect.height - 1)
    } else {
        0
    };
    let bottom = if target.row() != Some(false) {
        inset(p.bottom, rect.height - top - 1)
    } else {
        0
    };
    RectI {
        x: rect.x + left,
        y: rect.y + top,
        width: rect.width - left - right,
        height: rect.height - top - bottom,
    }
}

fn overhead(w: &ClientWindow, config: &LinuxShellConfig) -> SizeI {
    if !window_has_frame(w) {
        return SizeI::default();
    }
    let outer = w
        .chrome_outer
        .unwrap_or_else(|| legacy_window_outer(w, config));
    let content = w
        .chrome
        .as_ref()
        .map(|c| SizeI {
            width: c.content.bounds.width.round() as i32,
            height: c.content.bounds.height.round() as i32,
        })
        .unwrap_or(w.requested_size);
    SizeI {
        width: (outer.width - content.width).max(0),
        height: (outer.height - content.height).max(0),
    }
}
fn content_size(w: &ClientWindow, rect: RectI, config: &LinuxShellConfig) -> SizeI {
    let extra = overhead(w, config);
    SizeI {
        width: rect.width - extra.width,
        height: rect.height - extra.height,
    }
}
fn fits(w: &ClientWindow, rect: RectI, config: &LinuxShellConfig) -> bool {
    #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
    if let Some(hints) = w.tile_size_hints {
        let size = content_size(w, rect, config);
        let scale = w.surface_scale.max(1);
        if !hints.accepts_size(SizeI {
            width: size.width.saturating_mul(scale),
            height: size.height.saturating_mul(scale),
        }) {
            return false;
        }
    }
    fits_limits(w, rect, config)
}
fn fits_limits(w: &ClientWindow, rect: RectI, config: &LinuxShellConfig) -> bool {
    let size = content_size(w, rect, config);
    let mut policy = w.size_policy;
    policy.preferred = config.preferred_window_minimum;
    size.width > 0 && size.height > 0 && policy.resolve(size) == size
}
fn place(
    w: &mut ClientWindow,
    target: TileTarget,
    rect: RectI,
    shared: bool,
    surface: WaylandSurfaceId,
    scheduler: &mut ConfigureScheduler,
    config: &LinuxShellConfig,
    interactive: bool,
) {
    let size = content_size(w, rect, config);
    let changed =
        w.tile.is_none_or(|t| t.target != target || t.rect != rect) || w.requested_size != size;
    w.tile = Some(TilePlacement {
        target,
        rect,
        shared_resize: shared,
    });
    w.position = PointI {
        x: rect.x,
        y: rect.y,
    };
    w.requested_size = size;
    w.last_policy_request = None;
    if changed && !interactive {
        w.motion_veil_pending = w.motion_style.enabled();
        terminal(w, surface, scheduler);
    }
}
fn terminal(w: &mut ClientWindow, surface: WaylandSurfaceId, scheduler: &mut ConfigureScheduler) {
    if w.backend == Some(WindowBackend::Wayland) {
        w.native_configure.resize_final = Some(FinalResizeConfigure::pending(w.requested_size));
        scheduler.schedule_final(surface, w.requested_size);
    }
    #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
    if matches!(w.backend, Some(WindowBackend::X11(_))) {
        w.resize_preview.finish();
    }
}
pub(super) fn float_window(
    w: &mut ClientWindow,
    surface: WaylandSurfaceId,
    scheduler: &mut ConfigureScheduler,
) {
    if w.tile.take().is_none() {
        return;
    }
    w.motion_veil_pending = w.motion_style.enabled();
    if let Some((position, size)) = w.restore_geometry.take() {
        w.position = position;
        w.requested_size = size;
    }
    w.chrome_outer = None;
    w.native_configure.resize_anchor = None;
    terminal(w, surface, scheduler);
}
impl TilingController {
    fn reset_empty_layout(&mut self, windows: &BTreeMap<WaylandSurfaceId, ClientWindow>) {
        if !windows.values().any(|window| window.tile.is_some()) {
            self.splits = [0.5; 3];
            self.grab = None;
        }
    }

    pub fn dragging(&self) -> bool {
        self.grab.is_some()
    }
    pub fn sync(
        &mut self,
        widgets: &mut [WidgetLayer],
        windows: &mut BTreeMap<WaylandSurfaceId, ClientWindow>,
        scheduler: &mut ConfigureScheduler,
        area: RectI,
        config: &LinuxShellConfig,
        locked: bool,
    ) -> AppResult<()> {
        let policies = widgets
            .iter()
            .filter_map(WidgetLayer::tiling_policy)
            .collect::<Vec<_>>();
        if policies.len() > 1 {
            return Err(AppError::new(
                "only one WindowTiling widget may manage an output",
            ));
        }
        self.policy = policies.first().copied();
        if locked || self.policy.is_none() {
            self.finish(windows, scheduler);
            self.candidate = None;
        }
        let area_changed = area != self.area;
        if area_changed {
            self.finish(windows, scheduler);
        }
        self.area = area;
        let shared = self.policy.is_some_and(|p| p.shared_resize);
        for (id, w) in windows.iter_mut() {
            let Some(t) = w.tile else {
                continue;
            };
            if w.fullscreen || w.maximized {
                w.tile = None;
                continue;
            }
            if self.policy.is_none() {
                float_window(w, *id, scheduler);
                continue;
            }
            let rect = tile_rect(area, self.splits, t.target);
            if !fits(w, rect, config) {
                float_window(w, *id, scheduler);
            } else if area_changed
                || t.rect != rect
                || t.shared_resize != shared
                || w.requested_size != content_size(w, rect, config)
            {
                place(
                    w,
                    t.target,
                    rect,
                    shared,
                    *id,
                    scheduler,
                    config,
                    self.dragging(),
                );
            }
        }
        self.reset_empty_layout(windows);
        Ok(())
    }
    pub fn preview(
        &mut self,
        widgets: &mut [WidgetLayer],
        windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
        interaction: Option<WindowInteraction>,
        pointer: PointF,
        output: SizeI,
        config: &LinuxShellConfig,
        locked: bool,
    ) {
        self.reset_empty_layout(windows);
        self.candidate = if !locked {
            self.policy.and_then(|p| {
                let WindowInteraction::Move {
                    surface,
                    pointer_origin,
                    ..
                } = interaction?
                else {
                    return None;
                };
                if (pointer.x - pointer_origin.x).hypot(pointer.y - pointer_origin.y) < 4.0 {
                    return None;
                }
                let w = windows.get(&surface)?;
                if w.fullscreen || w.minimized {
                    return None;
                }
                let target = p.target(pointer, full_rect(output))?;
                fits(w, tile_rect(self.area, self.splits, target), config)
                    .then_some((surface, target))
            })
        } else {
            None
        };
        for widget in widgets.iter_mut().filter(|w| w.spec.tiling.is_some()) {
            let padding = widget.spec.tiling.unwrap().preview.padding;
            widget.tile_preview = self.candidate.map(|(surface, target)| {
                (
                    surface,
                    preview_rect(self.area, self.splits, target, padding),
                )
            });
            if let Some((surface, _)) = widget.tile_preview {
                widget.tile_preview_owner = Some(surface);
            }
        }
    }
    pub fn snap(
        &mut self,
        windows: &mut BTreeMap<WaylandSurfaceId, ClientWindow>,
        scheduler: &mut ConfigureScheduler,
        surface: WaylandSurfaceId,
        target: TileTarget,
        config: &LinuxShellConfig,
    ) -> bool {
        // Snap commands can follow untile in the same input batch, before sync/preview.
        self.reset_empty_layout(windows);
        let Some(policy) = self.policy else {
            return false;
        };
        if (target.row().is_none() && !policy.halves)
            || (target.row().is_some() && !policy.quadrants)
        {
            return false;
        }
        let rect = tile_rect(self.area, self.splits, target);
        if !windows.get(&surface).is_some_and(|w| {
            w.backend.is_some() && !w.fullscreen && !w.minimized && fits(w, rect, config)
        }) {
            return false;
        }
        for (id, w) in windows.iter_mut() {
            if *id != surface && w.tile.is_some_and(|t| t.target.conflicts(target)) {
                // A quadrant splits an occupied half: preserve the existing window in
                // the complementary quadrant instead of ejecting it to floating geometry.
                let complement = match target {
                    TileTarget::TopLeft => Some(TileTarget::BottomLeft),
                    TileTarget::BottomLeft => Some(TileTarget::TopLeft),
                    TileTarget::TopRight => Some(TileTarget::BottomRight),
                    TileTarget::BottomRight => Some(TileTarget::TopRight),
                    _ => None,
                }
                .filter(|_| w.tile.is_some_and(|tile| tile.target.row().is_none()));
                if let Some(other) = complement
                    .filter(|other| fits(w, tile_rect(self.area, self.splits, *other), config))
                {
                    w.native_configure.resize_anchor = None;
                    place(
                        w,
                        other,
                        tile_rect(self.area, self.splits, other),
                        policy.shared_resize,
                        *id,
                        scheduler,
                        config,
                        false,
                    );
                } else {
                    float_window(w, *id, scheduler);
                }
            }
        }
        let w = windows.get_mut(&surface).unwrap();
        if w.tile.is_none() && !w.maximized {
            w.restore_geometry = Some((w.position, w.requested_size));
        }
        w.maximized = false;
        w.native_configure.resize_anchor = None;
        place(
            w,
            target,
            rect,
            policy.shared_resize,
            surface,
            scheduler,
            config,
            false,
        );
        true
    }
    pub fn commit(
        &mut self,
        windows: &mut BTreeMap<WaylandSurfaceId, ClientWindow>,
        scheduler: &mut ConfigureScheduler,
        interaction: WindowInteraction,
        config: &LinuxShellConfig,
    ) {
        if let WindowInteraction::Move { surface, .. } = interaction {
            if let Some((owner, target)) = self.candidate.take() {
                if owner == surface {
                    self.snap(windows, scheduler, surface, target, config);
                }
            }
        }
    }
    pub fn hover(
        &self,
        windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
        stack: &[WaylandSurfaceId],
        p: PointF,
        config: &LinuxShellConfig,
    ) -> Option<Divider> {
        if let Some(g) = self.grab {
            return Some(g.divider);
        }
        let policy = self.policy.filter(|p| p.shared_resize)?;
        // An overlying floating window occludes divider hit regions.
        let top = stack
            .iter()
            .rev()
            .filter_map(|id| windows.get(id))
            .find(|w| {
                if w.minimized || w.backend.is_none() {
                    return false;
                }
                let o = w
                    .chrome_outer
                    .unwrap_or_else(|| legacy_window_outer(w, config));
                p.x >= w.position.x as f32
                    && p.y >= w.position.y as f32
                    && p.x < (w.position.x + o.width) as f32
                    && p.y < (w.position.y + o.height) as f32
            })?;
        top.tile?;
        let r = tile_rect(self.area, self.splits, TileTarget::Left);
        let half = policy.divider_hit_width / 2.0;
        if (p.x - r.right() as f32).abs() <= half
            && p.y >= self.area.y as f32
            && p.y < self.area.bottom() as f32
        {
            return Some(Divider::Vertical);
        }
        let left = p.x < r.right() as f32;
        let quadrant = tile_rect(
            self.area,
            self.splits,
            if left {
                TileTarget::TopLeft
            } else {
                TileTarget::TopRight
            },
        );
        if top.tile.is_some_and(|t| t.target.row().is_some())
            && (p.y - quadrant.bottom() as f32).abs() <= half
        {
            Some(if left { Divider::Left } else { Divider::Right })
        } else {
            None
        }
    }
    pub fn begin(
        &mut self,
        divider: Divider,
        p: PointF,
        windows: &mut BTreeMap<WaylandSurfaceId, ClientWindow>,
        scheduler: &mut ConfigureScheduler,
    ) {
        self.reveal.clear();
        for w in windows.values_mut() {
            w.tile_resize_hold = false;
        }
        self.grab = Some(DividerGrab {
            divider,
            start: p,
            splits: self.splits,
        });
        for (id, w) in windows
            .iter_mut()
            .filter(|(_, w)| w.tile.is_some_and(|t| affects(divider, t.target)) && !w.minimized)
        {
            if w.backend == Some(WindowBackend::Wayland) {
                w.native_configure.resize_anchor = Some(ResizeAnchor::new(
                    w.position,
                    w.requested_size,
                    ResizeEdge::BottomRight,
                ));
                w.native_configure.resize_final = None;
                scheduler.schedule_resize(*id, w.requested_size);
            }
            #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
            if matches!(w.backend, Some(WindowBackend::X11(_))) {
                w.resize_preview
                    .begin(w.position, w.requested_size, ResizeEdge::BottomRight);
            }
        }
    }
    pub fn update(
        &mut self,
        p: PointF,
        windows: &mut BTreeMap<WaylandSurfaceId, ClientWindow>,
        scheduler: &mut ConfigureScheduler,
        config: &LinuxShellConfig,
    ) {
        let Some(g) = self.grab else {
            return;
        };
        let index = match g.divider {
            Divider::Vertical => 0,
            Divider::Left => 1,
            Divider::Right => 2,
        };
        let delta = if index == 0 {
            (p.x - g.start.x) / self.area.width.max(1) as f32
        } else {
            (p.y - g.start.y) / self.area.height.max(1) as f32
        };
        let desired = (g.splits[index] + delta).clamp(0.0, 1.0);
        // Find the nearest feasible shared split; all clients constrain the same transaction.
        let valid = |value: f32| {
            let mut splits = self.splits;
            splits[index] = value;
            windows.values().all(|w| {
                w.tile
                    .is_none_or(|t| fits_limits(w, tile_rect(self.area, splits, t.target), config))
            })
        };
        let next = if valid(desired) {
            desired
        } else {
            let mut lo = self.splits[index];
            let mut hi = desired;
            for _ in 0..24 {
                let m = (lo + hi) / 2.0;
                if valid(m) { lo = m } else { hi = m }
            }
            lo
        };
        let extent = if index == 0 {
            self.area.width
        } else {
            self.area.height
        }
        .max(1);
        let current_pixel = (self.splits[index] * extent as f32).round() as i32;
        let desired_pixel = (next * extent as f32).round() as i32;
        if current_pixel == desired_pixel {
            return;
        }
        let step = (current_pixel - desired_pixel).signum();
        let mut pixel = desired_pixel;
        // ICCCM increments/aspect constraints are discrete. Search inward from the clamped
        // pointer position, never resize one member off the shared split. Bounded by output size.
        loop {
            let mut splits = self.splits;
            splits[index] = pixel as f32 / extent as f32;
            if windows.values().all(|w| {
                w.tile
                    .is_none_or(|t| fits(w, tile_rect(self.area, splits, t.target), config))
            }) {
                self.splits[index] = splits[index];
                break;
            }
            if pixel == current_pixel {
                break;
            }
            pixel += step;
        }
        for (id, w) in windows.iter_mut() {
            if let Some(t) = w.tile {
                let rect = tile_rect(self.area, self.splits, t.target);
                if rect == t.rect {
                    continue;
                }
                place(
                    w,
                    t.target,
                    rect,
                    t.shared_resize,
                    *id,
                    scheduler,
                    config,
                    true,
                );
            }
        }
    }
    /// Keep a shared resize veiled until every surviving member has final-size content.
    /// This is presentation-only; each client's protocol readiness remains independent.
    pub fn release_ready_group(&mut self, windows: &mut BTreeMap<WaylandSurfaceId, ClientWindow>) {
        self.reveal.retain(|id, target| {
            windows.get(id).is_some_and(|w| {
                !w.minimized && !w.fullscreen && w.tile.is_some_and(|t| t.target == *target)
            })
        });
        let waiting = self
            .reveal
            .keys()
            .any(|id| windows[id].waiting_for_resize_content());
        if !waiting {
            self.reveal.clear();
        }
        for (id, w) in windows.iter_mut() {
            w.tile_resize_hold = self.reveal.contains_key(id);
        }
    }

    pub fn finish(
        &mut self,
        windows: &mut BTreeMap<WaylandSurfaceId, ClientWindow>,
        scheduler: &mut ConfigureScheduler,
    ) {
        let Some(grab) = self.grab.take() else {
            return;
        };
        for (id, w) in windows
            .iter_mut()
            .filter(|(_, w)| w.tile.is_some_and(|t| affects(grab.divider, t.target)))
        {
            terminal(w, *id, scheduler);
            if !w.minimized {
                self.reveal.insert(*id, w.tile.unwrap().target);
                w.tile_resize_hold = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_padding_insets_outer_edges_without_opening_tile_seams() {
        let area = RectI {
            x: 17,
            y: 41,
            width: 1001,
            height: 703,
        };
        for splits in [[0.5, 0.5, 0.5], [0.37, 0.61, 0.42]] {
            let padding = crate::compose::Insets::new(7.0, 11.0, 13.0, 17.0);
            for target in [
                TileTarget::Left,
                TileTarget::Right,
                TileTarget::TopLeft,
                TileTarget::TopRight,
                TileTarget::BottomLeft,
                TileTarget::BottomRight,
            ] {
                let raw = tile_rect(area, splits, target);
                let padded = preview_rect(area, splits, target, padding);
                assert_eq!(
                    preview_rect(area, splits, target, crate::compose::Insets::ZERO),
                    raw
                );
                assert_eq!(padded.x, raw.x + if target.left() { 17 } else { 0 });
                assert_eq!(
                    padded.right(),
                    raw.right() - if target.left() { 0 } else { 11 }
                );
                assert_eq!(
                    padded.y,
                    raw.y + if target.row() != Some(true) { 7 } else { 0 }
                );
                assert_eq!(
                    padded.bottom(),
                    raw.bottom() - if target.row() != Some(false) { 13 } else { 0 }
                );
            }
            let top = preview_rect(area, splits, TileTarget::TopLeft, padding);
            let bottom = preview_rect(area, splits, TileTarget::BottomLeft, padding);
            let right = preview_rect(area, splits, TileTarget::Right, padding);
            assert_eq!(top.bottom(), bottom.y);
            assert_eq!(top.right(), right.x);
            assert_eq!(bottom.right(), right.x);
        }
    }

    #[test]
    fn preview_padding_rounds_and_clamps_to_nonempty_bounds() {
        let area = RectI {
            x: 0,
            y: 30,
            width: 101,
            height: 71,
        };
        let splits = [0.5; 3];
        let raw = tile_rect(area, splits, TileTarget::Left);
        let rounded = preview_rect(
            area,
            splits,
            TileTarget::Left,
            crate::compose::Insets::all(2.5),
        );
        assert_eq!(rounded.x, raw.x + 3);
        assert_eq!(rounded.y, raw.y + 3);
        assert_eq!(rounded.right(), raw.right());
        for target in [
            TileTarget::Left,
            TileTarget::Right,
            TileTarget::TopLeft,
            TileTarget::TopRight,
            TileTarget::BottomLeft,
            TileTarget::BottomRight,
        ] {
            let raw = tile_rect(area, splits, target);
            let padded = preview_rect(area, splits, target, crate::compose::Insets::all(f32::MAX));
            assert_eq!(padded.width, 1);
            assert_eq!(padded.height, 1);
            assert!(padded.x >= raw.x && padded.y >= raw.y);
            assert!(padded.right() <= raw.right() && padded.bottom() <= raw.bottom());
            if target.left() {
                assert_eq!(padded.right(), raw.right());
            } else {
                assert_eq!(padded.x, raw.x);
            }
        }
    }

    #[test]
    fn odd_work_area_partitions_without_gaps() {
        let area = RectI {
            x: 17,
            y: 41,
            width: 1001,
            height: 703,
        };
        let splits = [0.37, 0.61, 0.42];
        let a = tile_rect(area, splits, TileTarget::TopLeft);
        let b = tile_rect(area, splits, TileTarget::BottomLeft);
        let c = tile_rect(area, splits, TileTarget::Right);
        assert_eq!(a.bottom(), b.y);
        assert_eq!(a.right(), c.x);
        assert_eq!(b.bottom(), area.bottom());
        assert_eq!(c.right(), area.right());
        assert_eq!(
            a.width * a.height + b.width * b.height + c.width * c.height,
            area.width * area.height
        );
    }
}

#[cfg(test)]
mod host_tests {
    use super::super::client::maximize_preview_tests::test_window;
    use super::*;
    fn id(n: u32) -> WaylandSurfaceId {
        WaylandSurfaceId::from_raw(n).unwrap()
    }
    fn setup() -> (
        TilingController,
        BTreeMap<WaylandSurfaceId, ClientWindow>,
        ConfigureScheduler,
        LinuxShellConfig,
    ) {
        let mut windows = BTreeMap::new();
        for n in 1..=4 {
            let mut w = test_window(
                SizeI {
                    width: 600,
                    height: 400,
                },
                PointI { x: 80, y: 90 },
            );
            w.server_decorated = false;
            windows.insert(id(n), w);
        }
        (
            TilingController {
                policy: Some(WindowTiling::snap()),
                area: RectI {
                    x: 0,
                    y: 40,
                    width: 1200,
                    height: 760,
                },
                ..Default::default()
            },
            windows,
            ConfigureScheduler::default(),
            LinuxShellConfig {
                preferred_window_minimum: SizeI {
                    width: 100,
                    height: 100,
                },
                ..Default::default()
            },
        )
    }
    #[test]
    fn quadrant_dividers_veil_every_affected_member() {
        for divider in [Divider::Vertical, Divider::Left, Divider::Right] {
            let (mut t, mut w, mut q, c) = setup();
            for (n, target) in [
                (1, TileTarget::TopLeft),
                (2, TileTarget::BottomLeft),
                (3, TileTarget::TopRight),
                (4, TileTarget::BottomRight),
            ] {
                assert!(t.snap(&mut w, &mut q, id(n), target, &c));
                w.get_mut(&id(n)).unwrap().native_configure.resize_final = None;
            }
            t.begin(divider, PointF { x: 600., y: 420. }, &mut w, &mut q);
            for window in w.values() {
                assert_eq!(
                    window.resize_veil_active(),
                    affects(divider, window.tile.unwrap().target)
                );
            }
        }
    }

    #[test]
    fn shared_resize_reveals_together_after_the_slowest_client() {
        let (mut t, mut w, mut q, c) = setup();
        for (n, target) in [(1, TileTarget::Left), (2, TileTarget::Right)] {
            assert!(t.snap(&mut w, &mut q, id(n), target, &c));
        }
        t.begin(
            Divider::Vertical,
            PointF { x: 600., y: 400. },
            &mut w,
            &mut q,
        );
        t.finish(&mut w, &mut q);
        let ready = w.get_mut(&id(1)).unwrap();
        ready.native_configure.resize_final = None;
        ready.native_configure.resize_anchor = None;
        t.release_ready_group(&mut w);
        assert!(w[&id(1)].resize_veil_active());
        assert!(w[&id(2)].resize_veil_active());
        assert!(!w[&id(3)].tile_resize_hold);
        let ready = w.get_mut(&id(2)).unwrap();
        ready.native_configure.resize_final = None;
        ready.native_configure.resize_anchor = None;
        t.release_ready_group(&mut w);
        assert!(!w[&id(1)].resize_veil_active());
        assert!(!w[&id(2)].resize_veil_active());
    }

    #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
    #[test]
    fn x11_publication_holds_the_shared_reveal_until_ready() {
        let (mut t, mut w, mut q, c) = setup();
        for (n, target) in [(1, TileTarget::Left), (2, TileTarget::Right)] {
            w.get_mut(&id(n)).unwrap().backend =
                Some(WindowBackend::X11(crate::xwayland::association::XWindow {
                    generation: 1,
                    xid: n,
                    incarnation: 1,
                }));
            assert!(t.snap(&mut w, &mut q, id(n), target, &c));
        }
        t.begin(
            Divider::Vertical,
            PointF { x: 600., y: 400. },
            &mut w,
            &mut q,
        );
        t.finish(&mut w, &mut q);
        w.get_mut(&id(1)).unwrap().resize_preview = Default::default();
        t.release_ready_group(&mut w);
        assert!(w[&id(1)].tile_resize_hold);
        w.get_mut(&id(2)).unwrap().resize_preview = Default::default();
        t.release_ready_group(&mut w);
        assert!(!w[&id(1)].resize_veil_active());
        assert!(!w[&id(2)].resize_veil_active());
    }

    #[test]
    fn removed_resize_member_does_not_block_the_group() {
        let (mut t, mut w, mut q, c) = setup();
        for (n, target) in [(1, TileTarget::Left), (2, TileTarget::Right)] {
            assert!(t.snap(&mut w, &mut q, id(n), target, &c));
        }
        t.begin(
            Divider::Vertical,
            PointF { x: 600., y: 400. },
            &mut w,
            &mut q,
        );
        t.finish(&mut w, &mut q);
        w.remove(&id(2));
        let ready = w.get_mut(&id(1)).unwrap();
        ready.native_configure.resize_final = None;
        ready.native_configure.resize_anchor = None;
        t.release_ready_group(&mut w);
        assert!(!w[&id(1)].resize_veil_active());
    }

    #[test]
    fn snap_float_and_displacement_request_motion_handoff_but_dividers_do_not() {
        let (mut t, mut windows, mut scheduler, config) = setup();
        for window in windows.values_mut() {
            window.motion_style = crate::WindowMotion::smooth();
        }
        assert!(t.snap(
            &mut windows,
            &mut scheduler,
            id(1),
            TileTarget::Left,
            &config
        ));
        let window = windows.get_mut(&id(1)).unwrap();
        // Even an immediately-ready client must retain one placeholder frame for entry.
        window.native_configure.resize_final = None;
        assert!(window.resize_veil_active());
        assert!(std::mem::take(&mut window.motion_veil_pending));
        assert!(t.snap(
            &mut windows,
            &mut scheduler,
            id(2),
            TileTarget::Left,
            &config
        ));
        assert!(windows[&id(1)].tile.is_none());
        assert!(
            windows[&id(1)].motion_veil_pending,
            "displaced occupant restores with motion"
        );
        assert!(windows[&id(2)].motion_veil_pending);
        windows.get_mut(&id(2)).unwrap().motion_veil_pending = false;
        t.begin(
            Divider::Vertical,
            PointF { x: 600.0, y: 300.0 },
            &mut windows,
            &mut scheduler,
        );
        t.update(
            PointF { x: 620.0, y: 300.0 },
            &mut windows,
            &mut scheduler,
            &config,
        );
        assert!(
            !windows[&id(2)].motion_veil_pending,
            "divider updates stay direct"
        );
        t.finish(&mut windows, &mut scheduler);
        assert!(!windows[&id(2)].motion_veil_pending);
        float_window(windows.get_mut(&id(2)).unwrap(), id(2), &mut scheduler);
        assert!(windows[&id(2)].motion_veil_pending);
        windows.get_mut(&id(3)).unwrap().motion_style = crate::WindowMotion::none();
        assert!(t.snap(
            &mut windows,
            &mut scheduler,
            id(3),
            TileTarget::Right,
            &config
        ));
        assert!(!windows[&id(3)].motion_veil_pending);
    }

    #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
    #[test]
    fn x11_tile_transitions_keep_the_existing_content_readiness_gate() {
        let (mut t, mut windows, mut scheduler, config) = setup();
        let window = windows.get_mut(&id(1)).unwrap();
        window.backend = Some(WindowBackend::X11(crate::xwayland::association::XWindow {
            generation: 1,
            xid: 10,
            incarnation: 1,
        }));
        window.motion_style = crate::WindowMotion::fluid();
        assert!(t.snap(
            &mut windows,
            &mut scheduler,
            id(1),
            TileTarget::Left,
            &config
        ));
        let window = windows.get_mut(&id(1)).unwrap();
        assert!(std::mem::take(&mut window.motion_veil_pending));
        assert!(window.resize_preview.active());
        assert!(!window.resize_preview.dragging());
        assert!(
            window.resize_veil_active(),
            "X11 must still wait for configured-size content"
        );
        float_window(window, id(1), &mut scheduler);
        assert!(window.motion_veil_pending);
        assert!(window.resize_preview.active());
    }

    #[test]
    fn preview_padding_does_not_change_committed_window_geometry() {
        let (mut controller, mut windows, mut scheduler, config) = setup();
        controller.policy = Some(WindowTiling::snap().preview(crate::TilePreviewDesign {
            padding: crate::compose::Insets::all(20.0),
            ..Default::default()
        }));
        let expected = tile_rect(controller.area, controller.splits, TileTarget::TopLeft);
        assert!(controller.snap(
            &mut windows,
            &mut scheduler,
            id(1),
            TileTarget::TopLeft,
            &config
        ));
        assert_eq!(windows[&id(1)].tile.unwrap().rect, expected);
        assert_eq!(
            windows[&id(1)].position,
            PointI {
                x: expected.x,
                y: expected.y
            }
        );
        assert_eq!(
            windows[&id(1)].requested_size,
            SizeI {
                width: expected.width,
                height: expected.height
            }
        );
    }

    #[test]
    fn snap_saves_floating_geometry_and_displaces_conflicting_slots() {
        let (mut t, mut w, mut q, c) = setup();
        assert!(t.snap(&mut w, &mut q, id(1), TileTarget::TopLeft, &c));
        assert_eq!(w[&id(1)].position, PointI { x: 0, y: 40 });
        assert_eq!(
            w[&id(1)].requested_size,
            SizeI {
                width: 600,
                height: 380
            }
        );
        assert!(w[&id(1)].native_configure.resize_final.is_some());
        let states = window_toplevel_states(&w[&id(1)], false, false);
        assert!(states.tiled_left && states.tiled_right && states.tiled_top && states.tiled_bottom);
        assert!(!states.maximized && !states.resizing);
        assert!(t.snap(&mut w, &mut q, id(2), TileTarget::BottomLeft, &c));
        assert!(w[&id(1)].tile.is_some());
        assert!(t.snap(&mut w, &mut q, id(3), TileTarget::Left, &c));
        for n in 1..=2 {
            assert!(w[&id(n)].tile.is_none());
            assert_eq!(w[&id(n)].position, PointI { x: 80, y: 90 });
            assert_eq!(
                w[&id(n)].requested_size,
                SizeI {
                    width: 600,
                    height: 400
                }
            );
        }
    }
    #[test]
    fn empty_layout_forgets_dividers_before_preview_and_direct_snap() {
        for through_preview in [false, true] {
            let (mut t, mut w, mut q, c) = setup();
            assert!(t.snap(&mut w, &mut q, id(1), TileTarget::Left, &c));
            assert!(t.snap(&mut w, &mut q, id(2), TileTarget::Right, &c));
            t.begin(
                Divider::Vertical,
                PointF { x: 600.0, y: 400.0 },
                &mut w,
                &mut q,
            );
            t.update(PointF { x: 800.0, y: 400.0 }, &mut w, &mut q, &c);
            t.finish(&mut w, &mut q);
            assert_ne!(t.splits[0], 0.5);
            float_window(w.get_mut(&id(1)).unwrap(), id(1), &mut q);
            let existing = w[&id(2)].tile;
            // An active layout keeps its divider, so newly filled slots stay adjacent.
            assert!(t.snap(&mut w, &mut q, id(3), TileTarget::Left, &c));
            assert_eq!(w[&id(2)].tile, existing);
            assert_eq!(
                w[&id(3)].tile.unwrap().rect.right(),
                existing.unwrap().rect.x
            );
            for n in [2, 3] {
                float_window(w.get_mut(&id(n)).unwrap(), id(n), &mut q);
            }
            t.splits[1] = 0.3;
            t.splits[2] = 0.7;
            if through_preview {
                t.preview(
                    &mut [],
                    &w,
                    None,
                    PointF::default(),
                    SizeI {
                        width: 1200,
                        height: 800,
                    },
                    &c,
                    false,
                );
                assert_eq!(t.splits, [0.5; 3]);
            }
            assert!(t.snap(&mut w, &mut q, id(1), TileTarget::Left, &c));
            assert_eq!(t.splits, [0.5; 3]);
            assert_eq!(w[&id(1)].tile.unwrap().rect.width, 600);
            assert!(t.snap(&mut w, &mut q, id(2), TileTarget::TopLeft, &c));
            assert_eq!(w[&id(2)].tile.unwrap().rect.height, 380);
            assert_eq!(w[&id(1)].tile.unwrap().rect.height, 380);
        }
    }

    #[test]
    fn quadrant_snap_splits_existing_half_and_preserves_restore_and_motion() {
        for (half, incoming, remaining, opposite) in [
            (
                TileTarget::Left,
                TileTarget::TopLeft,
                TileTarget::BottomLeft,
                TileTarget::Right,
            ),
            (
                TileTarget::Left,
                TileTarget::BottomLeft,
                TileTarget::TopLeft,
                TileTarget::Right,
            ),
            (
                TileTarget::Right,
                TileTarget::TopRight,
                TileTarget::BottomRight,
                TileTarget::Left,
            ),
            (
                TileTarget::Right,
                TileTarget::BottomRight,
                TileTarget::TopRight,
                TileTarget::Left,
            ),
        ] {
            let (mut t, mut w, mut q, c) = setup();
            for n in [1, 2] {
                w.get_mut(&id(n)).unwrap().motion_style = crate::WindowMotion::smooth();
            }
            assert!(t.snap(&mut w, &mut q, id(1), half, &c));
            assert!(t.snap(&mut w, &mut q, id(3), opposite, &c));
            let restore = w[&id(1)].restore_geometry;
            let untouched = w[&id(3)].tile;
            w.get_mut(&id(1)).unwrap().motion_veil_pending = false;
            assert!(t.snap(&mut w, &mut q, id(2), incoming, &c));
            for (n, target) in [(1, remaining), (2, incoming)] {
                let window = &w[&id(n)];
                assert_eq!(window.tile.unwrap().target, target);
                assert_eq!(
                    window.tile.unwrap().rect,
                    tile_rect(t.area, t.splits, target)
                );
                assert!(window.motion_veil_pending);
                assert!(window.native_configure.resize_final.is_some());
            }
            assert_eq!(w[&id(1)].restore_geometry, restore);
            assert_eq!(w[&id(3)].tile, untouched);
            float_window(w.get_mut(&id(1)).unwrap(), id(1), &mut q);
            assert_eq!(
                Some((w[&id(1)].position, w[&id(1)].requested_size)),
                restore
            );
        }
    }

    #[test]
    fn quadrant_split_respects_existing_half_minimum_size() {
        let (mut t, mut w, mut q, c) = setup();
        assert!(t.snap(&mut w, &mut q, id(1), TileTarget::Left, &c));
        w.get_mut(&id(1)).unwrap().size_policy.minimum = Some(SizeI {
            width: 100,
            height: 500,
        });
        assert!(t.snap(&mut w, &mut q, id(2), TileTarget::TopLeft, &c));
        assert!(w[&id(1)].tile.is_none());
        assert_eq!(w[&id(2)].tile.unwrap().target, TileTarget::TopLeft);
    }

    #[test]
    fn shared_divider_clamps_all_members_and_horizontal_splits_are_independent() {
        let (mut t, mut w, mut q, c) = setup();
        for (n, target) in [
            (1, TileTarget::TopLeft),
            (2, TileTarget::BottomLeft),
            (3, TileTarget::TopRight),
            (4, TileTarget::BottomRight),
        ] {
            assert!(t.snap(&mut w, &mut q, id(n), target, &c));
        }
        w.get_mut(&id(3)).unwrap().size_policy.minimum = Some(SizeI {
            width: 450,
            height: 200,
        });
        t.begin(
            Divider::Vertical,
            PointF { x: 600., y: 400. },
            &mut w,
            &mut q,
        );
        t.update(PointF { x: 1150., y: 400. }, &mut w, &mut q, &c);
        assert_eq!(w[&id(1)].tile.unwrap().rect.right(), 750);
        assert_eq!(w[&id(3)].requested_size.width, 450);
        assert!(w[&id(1)].native_configure.resize_anchor.is_some());
        assert!(w[&id(1)].native_configure.resize_final.is_none());
        t.finish(&mut w, &mut q);
        assert!(w[&id(3)].native_configure.resize_final.is_some());
        let right = w[&id(3)].tile;
        t.begin(Divider::Left, PointF { x: 100., y: 420. }, &mut w, &mut q);
        t.update(PointF { x: 100., y: 520. }, &mut w, &mut q, &c);
        assert_eq!(w[&id(1)].tile.unwrap().rect.bottom(), 520);
        assert_eq!(w[&id(2)].position.y, 520);
        assert_eq!(w[&id(3)].tile, right);
    }
    #[test]
    fn restored_tile_can_snap_on_the_same_pointer_event() {
        let (mut t, mut w, mut q, c) = setup();
        assert!(t.snap(&mut w, &mut q, id(1), TileTarget::TopLeft, &c));
        let mut grab =
            WindowInteraction::begin_move(&w, id(1), PointF { x: 300., y: 45. }).unwrap();
        let pointer = PointF { x: 1., y: 1. };
        let output = SizeI {
            width: 1200,
            height: 800,
        };
        apply_window_interaction(&mut w, &mut grab, &mut q, pointer, output, &c).unwrap();
        assert!(w[&id(1)].tile.is_none());
        t.preview(&mut [], &w, Some(grab), pointer, output, &c, false);
        t.commit(&mut w, &mut q, grab, &c);
        assert_eq!(w[&id(1)].tile.unwrap().target, TileTarget::TopLeft);
    }

    #[test]
    fn impossible_snap_does_not_displace_existing_window() {
        let (mut t, mut w, mut q, c) = setup();
        assert!(t.snap(&mut w, &mut q, id(1), TileTarget::Left, &c));
        w.get_mut(&id(2)).unwrap().size_policy.minimum = Some(SizeI {
            width: 800,
            height: 400,
        });
        assert!(!t.snap(&mut w, &mut q, id(2), TileTarget::Left, &c));
        assert!(w[&id(1)].tile.is_some());
        assert!(w[&id(2)].tile.is_none());
    }
    #[test]
    fn tiled_title_drag_restores_and_rebases_grab() {
        let (mut t, mut w, mut q, c) = setup();
        assert!(t.snap(&mut w, &mut q, id(1), TileTarget::TopLeft, &c));
        let mut grab =
            WindowInteraction::begin_move(&w, id(1), PointF { x: 300., y: 45. }).unwrap();
        apply_window_interaction(
            &mut w,
            &mut grab,
            &mut q,
            PointF { x: 302., y: 45. },
            SizeI {
                width: 1200,
                height: 800,
            },
            &c,
        )
        .unwrap();
        assert!(w[&id(1)].tile.is_some());
        apply_window_interaction(
            &mut w,
            &mut grab,
            &mut q,
            PointF { x: 500., y: 100. },
            SizeI {
                width: 1200,
                height: 800,
            },
            &c,
        )
        .unwrap();
        assert!(w[&id(1)].tile.is_none());
        assert_eq!(
            w[&id(1)].requested_size,
            SizeI {
                width: 600,
                height: 400
            }
        );
        let position = w[&id(1)].position;
        apply_window_interaction(
            &mut w,
            &mut grab,
            &mut q,
            PointF { x: 510., y: 110. },
            SizeI {
                width: 1200,
                height: 800,
            },
            &c,
        )
        .unwrap();
        assert_eq!(
            w[&id(1)].position,
            PointI {
                x: position.x + 10,
                y: position.y + 10
            }
        );
    }
    #[test]
    fn overlaying_floating_window_blocks_divider_and_disabling_widget_restores_members() {
        let (mut t, mut w, mut q, c) = setup();
        t.snap(&mut w, &mut q, id(1), TileTarget::Left, &c);
        t.snap(&mut w, &mut q, id(2), TileTarget::Right, &c);
        let p = PointF { x: 600., y: 400. };
        assert_eq!(t.hover(&w, &[id(1), id(2)], p, &c), Some(Divider::Vertical));
        assert_eq!(t.hover(&w, &[id(1), id(2), id(3)], p, &c), None);
        t.sync(
            &mut [],
            &mut w,
            &mut q,
            RectI {
                x: 0,
                y: 40,
                width: 1200,
                height: 760,
            },
            &c,
            false,
        )
        .unwrap();
        assert!(w.values().all(|w| w.tile.is_none()));
    }
    #[test]
    fn maximize_from_tile_preserves_original_floating_restore() {
        let (mut t, mut w, mut q, c) = setup();
        t.snap(&mut w, &mut q, id(1), TileTarget::Left, &c);
        set_window_maximized(&mut w, &mut q, id(1), true, t.area, &c).unwrap();
        assert!(w[&id(1)].tile.is_none());
        set_window_maximized(&mut w, &mut q, id(1), false, t.area, &c).unwrap();
        assert_eq!(w[&id(1)].position, PointI { x: 80, y: 90 });
        assert_eq!(
            w[&id(1)].requested_size,
            SizeI {
                width: 600,
                height: 400
            }
        );
    }
}

#[cfg(all(test, feature = "shell-xwayland", target_env = "gnu"))]
mod x11_constraint_tests {
    use super::super::client::maximize_preview_tests::test_window;
    use super::*;
    #[test]
    fn divider_respects_both_discrete_size_grids() {
        let ids = [
            WaylandSurfaceId::from_raw(1).unwrap(),
            WaylandSurfaceId::from_raw(2).unwrap(),
        ];
        let mut windows = BTreeMap::new();
        for id in ids {
            let mut w = test_window(
                SizeI {
                    width: 400,
                    height: 300,
                },
                PointI { x: 0, y: 0 },
            );
            w.server_decorated = false;
            w.tile_size_hints = Some(crate::xwayland::normal_hints::NormalHints {
                increment: Some(SizeI {
                    width: 8,
                    height: 1,
                }),
                ..Default::default()
            });
            windows.insert(id, w);
        }
        let mut controller = TilingController {
            policy: Some(WindowTiling::snap()),
            area: RectI {
                x: 0,
                y: 0,
                width: 1200,
                height: 800,
            },
            ..Default::default()
        };
        let config = LinuxShellConfig {
            preferred_window_minimum: SizeI {
                width: 100,
                height: 100,
            },
            ..Default::default()
        };
        let mut scheduler = ConfigureScheduler::default();
        assert!(controller.snap(
            &mut windows,
            &mut scheduler,
            ids[0],
            TileTarget::Left,
            &config
        ));
        assert!(controller.snap(
            &mut windows,
            &mut scheduler,
            ids[1],
            TileTarget::Right,
            &config
        ));
        controller.begin(
            Divider::Vertical,
            PointF { x: 600., y: 300. },
            &mut windows,
            &mut scheduler,
        );
        controller.update(
            PointF { x: 621., y: 300. },
            &mut windows,
            &mut scheduler,
            &config,
        );
        assert_eq!(windows[&ids[0]].requested_size.width, 616);
        assert_eq!(windows[&ids[1]].requested_size.width, 584);
    }
}

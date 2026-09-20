//! Host-owned snap layout and coordinated divider grabs. No GPU ownership lives here.
use super::*;
use crate::authoring::compose::{TileTarget, WindowTiling};

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
    padding: crate::authoring::compose::Insets,
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
mod tests;

#[cfg(test)]
mod host_tests;

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
            w.tile_size_hints = Some(crate::integrations::x11::normal_hints::NormalHints {
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

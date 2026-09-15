//! X11 adapter for shared frame geometry. No renderer, frame layout or pointer
//! gesture lives here; root coordinates cross this boundary exactly once.
use super::*;
use crate::xwayland::{
    association::XWindow,
    window::{Geometry, RequestedConfigure},
    xwm::Xwm,
};

pub(super) fn authorize_move_resize(
    seat: &crate::compositor_wayland::SeatState,
    surface: WaylandSurfaceId,
    button: u32,
) -> bool {
    let button = match button {
        1 => 0x110,
        2 => 0x112,
        3 => 0x111,
        _ => return false,
    };
    seat.pressed_buttons().contains(&button)
        && seat
            .pointer_grab_focus()
            .is_some_and(|focus| focus.surface == surface)
}

/// Visual diagnostic only: never replaces client acknowledgement or buffer checks.
fn diagnostic_preview_hold() -> Duration {
    static HOLD: std::sync::OnceLock<Duration> = std::sync::OnceLock::new();
    *HOLD.get_or_init(|| {
        let milliseconds = std::env::var("TELORGON_X11_PREVIEW_HOLD_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0)
            .min(2000);
        Duration::from_millis(milliseconds)
    })
}

/// X11 completion evidence for the shared resize veil. X11 has no xdg ack:
/// a changed size needs both a checked server configure and newly published
/// content at that size. No-op resizes can reuse already matching content.
#[derive(Clone, Copy, Debug, Default)]
pub(super) enum ResizePreview {
    #[default]
    Idle,
    Dragging {
        anchor: ResizeAnchor,
    },
    Settling {
        anchor: Option<ResizeAnchor>,
        target: Option<ResizeTarget>,
        not_before: Option<Instant>,
    },
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ResizeTarget {
    size: SizeI,
    revision_after: Option<u64>,
}

impl ResizePreview {
    pub(super) fn active(self) -> bool {
        !matches!(self, Self::Idle)
    }
    pub(super) fn dragging(self) -> bool {
        matches!(self, Self::Dragging { .. })
    }
    pub(super) fn begin(&mut self, position: PointI, size: SizeI, edge: ResizeEdge) {
        *self = Self::Dragging {
            anchor: ResizeAnchor::new(position, size, edge),
        };
    }
    pub(super) fn finish(&mut self) {
        let anchor = self.anchor();
        let hold = diagnostic_preview_hold();
        *self = Self::Settling {
            anchor,
            target: None,
            not_before: (!hold.is_zero()).then(|| Instant::now() + hold),
        };
        if !hold.is_zero() {
            eprintln!(
                "telorgon-xwayland: resize veil started; diagnostic minimum={}ms",
                hold.as_millis()
            );
        }
    }
    fn hold_deadline(self, now: Instant) -> Option<Instant> {
        match self {
            Self::Settling {
                not_before: Some(deadline),
                ..
            } if deadline > now => Some(deadline),
            _ => None,
        }
    }
    fn anchor(self) -> Option<ResizeAnchor> {
        match self {
            Self::Dragging { anchor } => Some(anchor),
            Self::Settling { anchor, .. } => anchor,
            Self::Idle => None,
        }
    }
    pub(super) fn submitted(&mut self, size: SizeI, revision: u64, matching_content: bool) {
        if let Self::Settling { target, .. } = self {
            if target.is_none_or(|old| old.size != size) {
                *target = Some(ResizeTarget {
                    size,
                    revision_after: (!matching_content).then_some(revision),
                });
            }
        }
    }
    fn settle(&mut self, server: SizeI, content: SizeI, revision: u64, pending: bool) -> bool {
        self.settle_at(server, content, revision, pending, Instant::now())
    }
    fn settle_at(
        &mut self,
        server: SizeI,
        content: SizeI,
        revision: u64,
        pending: bool,
        now: Instant,
    ) -> bool {
        if self.hold_deadline(now).is_some() {
            return false;
        }
        let Self::Settling {
            target: Some(target),
            ..
        } = self
        else {
            return false;
        };
        if pending
            || server != target.size
            || content != target.size
            || target
                .revision_after
                .is_some_and(|minimum| revision <= minimum)
        {
            return false;
        }
        *self = Self::Idle;
        if !diagnostic_preview_hold().is_zero() {
            eprintln!("telorgon-xwayland: resize veil cleared; completion checks passed");
        }
        true
    }
}

#[derive(Default)]
pub(super) struct X11Windows {
    entries: BTreeMap<XWindow, Entry>,
    requests: BTreeMap<XWindow, RequestedConfigure>,
    unattached_replies: BTreeMap<XWindow, u16>,
    preview_deadline: Option<Instant>,
}
struct Entry {
    surface: WaylandSurfaceId,
    sent: Option<Geometry>,
    notify_border: Option<u16>,
    requested_border: u16,
}

impl X11Windows {
    pub(super) fn preview_deadline(&self) -> Option<Instant> {
        self.preview_deadline
    }
    pub(super) fn retain(&mut self, live: &BTreeSet<XWindow>) {
        self.entries.retain(|id, _| live.contains(id));
        self.requests.retain(|id, _| live.contains(id));
        self.unattached_replies.retain(|id, _| live.contains(id));
    }

    pub(super) fn configure_requested(&mut self, id: XWindow, request: RequestedConfigure) {
        let pending = self.requests.entry(id).or_default();
        if request.x.is_some() {
            pending.x = request.x;
        }
        if request.y.is_some() {
            pending.y = request.y;
        }
        if request.width.is_some() {
            pending.width = request.width;
        }
        if request.height.is_some() {
            pending.height = request.height;
        }
        if request.border.is_some() {
            pending.border = request.border;
        }
    }

    pub(super) fn attach(
        &mut self,
        id: XWindow,
        surface: WaylandSurfaceId,
        geometry: Geometry,
        unmanaged: bool,
        window: &mut ClientWindow,
        config: &LinuxShellConfig,
    ) {
        let density = window.surface_scale.max(1);
        if unmanaged {
            window.resize_preview = Default::default();
            window.backend = None;
            window.server_decorated = false;
            window.chrome = None;
            window.chrome_outer = None;
            window.chrome_content_offset = None;
            window.position = PointI {
                x: i32::from(geometry.x).div_euclid(density),
                y: i32::from(geometry.y).div_euclid(density),
            };
            window.minimized = false;
            return;
        }
        let fresh = self
            .entries
            .get(&id)
            .is_none_or(|entry| entry.surface != surface)
            || window.backend != Some(WindowBackend::X11(id));
        window.backend = Some(WindowBackend::X11(id));
        if fresh {
            window.server_decorated = true;
            window.resize_preview = Default::default();
            let offset = window_content_offset(window, config);
            window.position = PointI {
                x: i32::from(geometry.x)
                    .div_euclid(density)
                    .saturating_sub(offset.x)
                    .max(0),
                y: i32::from(geometry.y)
                    .div_euclid(density)
                    .saturating_sub(offset.y)
                    .max(0),
            };
            window.requested_size = SizeI {
                width: (i32::from(geometry.width) / density).max(1),
                height: (i32::from(geometry.height) / density).max(1),
            };
            window.minimized = false;
            let (notify_border, requested_border) = self
                .entries
                .get(&id)
                .map(|entry| (entry.notify_border, entry.requested_border))
                .unwrap_or((None, geometry.border));
            self.entries.insert(
                id,
                Entry {
                    surface,
                    sent: None,
                    notify_border,
                    requested_border,
                },
            );
        }
        if let Some(request) = self.requests.remove(&id) {
            let entry = self.entries.get_mut(&id).expect("attached entry");
            if let Some(border) = request.border {
                entry.requested_border = border;
            }
            entry.notify_border = Some(entry.requested_border);
            // Client stacking is denied by the desktop's click-to-raise policy.
            // Even a stack-only/no-op request receives the actual geometry below.
            // Explicit client configure requests enter shared policy. A server
            // ConfigureNotify is confirmation, not authority to undo a drag.
            if !window.maximized && !window.fullscreen && !window.resize_preview.active() {
                let offset = window_content_offset(window, config);
                if let Some(x) = request.x {
                    window.position.x = i32::from(x).div_euclid(density) - offset.x;
                }
                if let Some(y) = request.y {
                    window.position.y = i32::from(y).div_euclid(density) - offset.y;
                }
                if let Some(width) = request.width {
                    window.requested_size.width = (i32::from(width) / density).max(1);
                }
                if let Some(height) = request.height {
                    window.requested_size.height = (i32::from(height) / density).max(1);
                }
            }
        }
    }

    /// Before a Wayland image is associated, geometry negotiation must still
    /// progress: clients may wait for ConfigureNotify before creating that image.
    fn flush_unattached(
        &mut self,
        xwm: &mut Xwm,
        submitted: &mut usize,
    ) -> crate::xwayland::Result<()> {
        let requests: Vec<_> = self
            .requests
            .keys()
            .filter(|id| {
                self.entries.get(id).is_none_or(|entry| {
                    xwm.windows()
                        .and_then(|windows| windows.presentable_surface(**id))
                        != Some(u64::from(entry.surface.get()))
                })
            })
            .copied()
            .take(16)
            .collect();
        for id in requests {
            if *submitted >= 16 || xwm.command_capacity() == 0 {
                break;
            }
            if xwm.commands_pending(id) {
                continue;
            }
            let Some(actual) = xwm
                .windows()
                .and_then(|windows| windows.get(id.xid))
                .filter(|window| window.id == id)
                .map(|window| window.geometry)
            else {
                continue;
            };
            let request = self.requests.remove(&id).expect("listed request");
            let desired = Geometry {
                x: request.x.unwrap_or(actual.x),
                y: request.y.unwrap_or(actual.y),
                width: request.width.unwrap_or(actual.width).max(1),
                height: request.height.unwrap_or(actual.height).max(1),
                border: request.border.unwrap_or(actual.border),
            };
            if desired != actual {
                xwm.configure_window(id, desired, Instant::now())?;
                *submitted += 1;
            }
            let notify_border = if let Some(entry) = self.entries.get_mut(&id) {
                if let Some(border) = request.border {
                    entry.requested_border = border;
                }
                entry.notify_border = None;
                entry.requested_border
            } else {
                desired.border
            };
            self.unattached_replies.insert(id, notify_border);
        }
        let replies: Vec<_> = self.unattached_replies.keys().copied().take(16).collect();
        for id in replies {
            if *submitted >= 16 || xwm.command_capacity() == 0 {
                break;
            }
            if xwm.commands_pending(id) {
                continue;
            }
            let Some(actual) = xwm
                .windows()
                .and_then(|windows| windows.get(id.xid))
                .filter(|window| window.id == id)
                .map(|window| window.geometry)
            else {
                continue;
            };
            let border = self.unattached_replies.remove(&id).expect("listed reply");
            xwm.notify_configured(
                id,
                notification_geometry(actual, border),
                None,
                Instant::now(),
            )?;
            *submitted += 1;
        }
        Ok(())
    }

    pub(super) fn flush(
        &mut self,
        xwm: &mut Xwm,
        windows: &mut BTreeMap<WaylandSurfaceId, ClientWindow>,
        config: &LinuxShellConfig,
    ) -> crate::xwayland::Result<bool> {
        let mut changed = false;
        let started = Instant::now();
        self.preview_deadline = windows
            .values()
            .filter_map(|window| window.resize_preview.hold_deadline(started))
            .min();
        let mut submitted = 0;
        self.flush_unattached(xwm, &mut submitted)?;
        for (id, entry) in &mut self.entries {
            if submitted >= 16 || started.elapsed() >= Duration::from_millis(1) {
                break;
            }
            let Some(window) = windows
                .get_mut(&entry.surface)
                .filter(|window| window.backend == Some(WindowBackend::X11(*id)))
            else {
                continue;
            };
            if xwm
                .windows()
                .and_then(|registry| registry.presentable_surface(*id))
                != Some(u64::from(entry.surface.get()))
            {
                continue;
            }
            if !xwm.set_frame_extents(*id, frame_extents(window, config), Instant::now())? {
                break;
            }
            if !xwm.set_maximized_state(*id, window.maximized, Instant::now())? {
                break;
            }
            if window.resize_preview.dragging() {
                if !xwm.commands_pending(*id) && xwm.command_capacity() > 0 {
                    if let Some(border) = entry.notify_border.take() {
                        let actual = xwm
                            .windows()
                            .and_then(|registry| registry.get(id.xid))
                            .expect("presentable window")
                            .geometry;
                        xwm.notify_configured(
                            *id,
                            notification_geometry(actual, border),
                            None,
                            Instant::now(),
                        )?;
                        submitted += 1;
                    }
                }
                continue;
            }
            let density = window.surface_scale.max(1);
            let mut constrained_pixels = None;
            if !window.maximized && !window.fullscreen {
                let hints = xwm.normal_hints(*id).unwrap_or_default();
                let logical = |size: SizeI| SizeI {
                    width: (size.width / density).max(1),
                    height: (size.height / density).max(1),
                };
                window.size_policy.minimum = hints.minimum.map(logical);
                window.size_policy.maximum = hints.maximum.map(logical);
                let pixels = |size: SizeI| SizeI {
                    width: size.width.saturating_mul(density).min(65535),
                    height: size.height.saturating_mul(density).min(65535),
                };
                let size = preferred_size(
                    hints,
                    pixels(window.requested_size),
                    pixels(window.size_policy.preferred),
                    pixels(window.size_policy.available),
                );
                constrained_pixels = Some(size);
                let previous = window.requested_size;
                window.requested_size = logical(size);
                if previous != window.requested_size {
                    if !window.resize_preview.active() {
                        window.resize_preview.begin(
                            window.position,
                            previous,
                            ResizeEdge::BottomRight,
                        );
                        window.resize_preview.finish();
                    }
                    changed = true;
                }
            }
            if let Some(anchor) = window.resize_preview.anchor() {
                let previous = window.position;
                window.position = anchor.reconcile_position(window.position, window.requested_size);
                changed |= previous != window.position;
            }
            let mut desired = frame_geometry(window, config);
            if let Some(size) = constrained_pixels {
                desired.width = size.width.clamp(1, u16::MAX.into()) as u16;
                desired.height = size.height.clamp(1, u16::MAX.into()) as u16;
            }
            let Some(server) = xwm
                .windows()
                .and_then(|registry| registry.get(id.xid))
                .map(|window| window.geometry)
            else {
                continue;
            };
            let server_size = SizeI {
                width: server.width.into(),
                height: server.height.into(),
            };
            let target_size = SizeI {
                width: desired.width.into(),
                height: desired.height.into(),
            };
            let pending = xwm.commands_pending(*id);
            if !pending && entry.sent != Some(desired) {
                if !xwm.configure_window_synced(*id, desired, Instant::now())? {
                    continue;
                }
                // Snapshot before dispatch can publish content for the queued request.
                if window.resize_preview.active() {
                    resize_trace::begin(
                        entry.surface.get(),
                        format_args!(
                            "xid={} target={:?} previous_revision={} sync={:?}",
                            id.xid,
                            target_size,
                            window.presentation.revision,
                            xwm.resize_sync_status(*id)
                        ),
                    );
                }
                window.resize_preview.submitted(
                    target_size,
                    window.presentation.revision,
                    server_size == target_size && window.presentation.size == target_size,
                );
                entry.sent = Some(desired);
                submitted += 1;
            } else if !pending && entry.sent == Some(desired) {
                window.resize_preview.submitted(
                    target_size,
                    window.presentation.revision,
                    !pending
                        && server_size == target_size
                        && window.presentation.size == target_size,
                );
            }
            if !xwm.commands_pending(*id) && xwm.command_capacity() > 0 {
                if let Some(border) = entry.notify_border.take() {
                    xwm.notify_configured(
                        *id,
                        notification_geometry(server, border),
                        None,
                        Instant::now(),
                    )?;
                    submitted += 1;
                }
            }
            let settled = settle_resize(
                window,
                server_size,
                xwm.commands_pending(*id) || xwm.repaint_pending(*id),
            );
            if settled {
                resize_trace::reveal(
                    entry.surface.get(),
                    format_args!(
                        "revision={} size={:?}",
                        window.presentation.revision, window.presentation.size
                    ),
                );
            }
            changed |= settled;
        }
        // Refresh optional diagnostic wakeups after completion changes.
        self.preview_deadline = windows
            .values()
            .filter_map(|window| window.resize_preview.hold_deadline(started))
            .min();
        Ok(changed)
    }
}

/// The client can finish before retained chrome is laid out. Never reveal its
/// replacement image against a frame snapshot from the previous requested size.
pub(super) fn settle_resize(window: &mut ClientWindow, server: SizeI, pending: bool) -> bool {
    let frame_pending = window.chrome.as_ref().is_some_and(|chrome| {
        let content = chrome.content.bounds;
        content.width.round().max(1.0) as i32 != window.requested_size.width
            || content.height.round().max(1.0) as i32 != window.requested_size.height
    });
    window.resize_preview.settle(
        server,
        window.presentation.size,
        window.presentation.revision,
        pending || frame_pending,
    )
}

/// Switch frame ownership without moving the client content or dropping management.
pub(super) fn apply_decorations(
    window: &mut ClientWindow,
    decorated: bool,
    config: &LinuxShellConfig,
) {
    if window.server_decorated == decorated {
        return;
    }
    let old_offset = window_content_offset(window, config);
    let old_outer = if window_has_frame(window) {
        window
            .chrome_outer
            .unwrap_or_else(|| legacy_window_outer(window, config))
    } else {
        window.requested_size
    };
    window.server_decorated = decorated;
    window.chrome = None;
    window.chrome_outer = None;
    window.chrome_content_offset = None;
    let new_offset = window_content_offset(window, config);
    let delta = PointI {
        x: old_offset.x - new_offset.x,
        y: old_offset.y - new_offset.y,
    };
    if window.maximized && !window.fullscreen {
        let new_outer = if window_has_frame(window) {
            legacy_window_outer(window, config)
        } else {
            window.requested_size
        };
        window.requested_size = SizeI {
            width: (old_outer.width - (new_outer.width - window.requested_size.width)).max(1),
            height: (old_outer.height - (new_outer.height - window.requested_size.height)).max(1),
        };
        window.resize_preview.finish();
    } else {
        window.position.x = window.position.x.saturating_add(delta.x);
        window.position.y = window.position.y.saturating_add(delta.y);
    }
    if let Some((position, _)) = &mut window.restore_geometry {
        // Restore geometry describes a non-fullscreen frame, even when the current frame is hidden.
        let title_delta = if decorated {
            -config.titlebar_height
        } else {
            config.titlebar_height
        };
        position.y = position.y.saturating_add(title_delta);
    }
}

pub(super) fn frame_extents(window: &ClientWindow, config: &LinuxShellConfig) -> [u32; 4] {
    if !window_has_frame(window) {
        return [0; 4];
    }
    let offset = window_content_offset(window, config);
    let outer = window
        .chrome_outer
        .unwrap_or_else(|| legacy_window_outer(window, config));
    let density = window.surface_scale.max(1);
    [
        offset.x,
        outer.width - window.requested_size.width - offset.x,
        offset.y,
        outer.height - window.requested_size.height - offset.y,
    ]
    .map(|value| value.max(0).saturating_mul(density) as u32)
}

fn frame_geometry(window: &ClientWindow, config: &LinuxShellConfig) -> Geometry {
    let content = window_content_rect(window, window.position, config);
    let density = window.surface_scale.max(1);
    Geometry {
        x: content
            .x
            .saturating_mul(density)
            .clamp(i16::MIN.into(), i16::MAX.into()) as i16,
        y: content
            .y
            .saturating_mul(density)
            .clamp(i16::MIN.into(), i16::MAX.into()) as i16,
        width: content
            .width
            .saturating_mul(density)
            .clamp(1, u16::MAX.into()) as u16,
        height: content
            .height
            .saturating_mul(density)
            .clamp(1, u16::MAX.into()) as u16,
        // The compositor supplies the border; an X11 server border would be a
        // second frame and would shift the surface/input origin.
        border: 0,
    }
}

/// Bounded grid/aspect projection. Candidate count is independent of client
/// dimensions; inconsistent hints fall back to confirmed server size.
fn preferred_size(
    hints: crate::xwayland::normal_hints::NormalHints,
    wanted: SizeI,
    preferred: SizeI,
    available: SizeI,
) -> SizeI {
    let client_max = hints.maximum.unwrap_or(SizeI {
        width: 65535,
        height: 65535,
    });
    let floor = SizeI {
        width: preferred
            .width
            .min(available.width)
            .min(client_max.width)
            .max(1),
        height: preferred
            .height
            .min(available.height)
            .min(client_max.height)
            .max(1),
    };
    constrained_size_in(hints, wanted, floor, available)
        .or_else(|| {
            constrained_size_in(
                hints,
                wanted,
                SizeI {
                    width: 1,
                    height: 1,
                },
                available,
            )
        })
        .or_else(|| constrained_size(hints, wanted))
        .unwrap_or_else(|| {
            super::size_policy::SizePolicy {
                preferred,
                available,
                ..Default::default()
            }
            .resolve(wanted)
        })
}

fn constrained_size(
    hints: crate::xwayland::normal_hints::NormalHints,
    wanted: SizeI,
) -> Option<SizeI> {
    constrained_size_in(
        hints,
        wanted,
        SizeI {
            width: 1,
            height: 1,
        },
        SizeI {
            width: 65535,
            height: 65535,
        },
    )
}

fn constrained_size_in(
    hints: crate::xwayland::normal_hints::NormalHints,
    wanted: SizeI,
    floor: SizeI,
    ceiling: SizeI,
) -> Option<SizeI> {
    let client_min = hints.minimum_size();
    let client_max = hints.maximum.unwrap_or(SizeI {
        width: 65535,
        height: 65535,
    });
    let minimum = SizeI {
        width: client_min.width.max(floor.width),
        height: client_min.height.max(floor.height),
    };
    let maximum = SizeI {
        width: client_max.width.min(ceiling.width),
        height: client_max.height.min(ceiling.height),
    };
    if hints.accepts_size(wanted)
        && wanted.width >= minimum.width
        && wanted.height >= minimum.height
        && wanted.width <= maximum.width
        && wanted.height <= maximum.height
    {
        return Some(wanted);
    }
    let base = hints.increment_base();
    let step = hints.increment.unwrap_or(SizeI {
        width: 1,
        height: 1,
    });
    let (min_w, min_h) = (
        i64::from(minimum.width.max(1)),
        i64::from(minimum.height.max(1)),
    );
    let (max_w, max_h) = (
        i64::from(maximum.width.min(65535)),
        i64::from(maximum.height.min(65535)),
    );
    if min_w > max_w || min_h > max_h || step.width <= 0 || step.height <= 0 {
        return None;
    }
    let snap = |value: i64, base: i32, step: i32| {
        i64::from(base) + (value - i64::from(base)).div_euclid(i64::from(step)) * i64::from(step)
    };
    let height = snap(
        i64::from(wanted.height).clamp(min_h, max_h),
        base.height,
        step.height,
    );
    let mut heights = vec![height, height + i64::from(step.height), min_h, max_h];
    if let Some((low, high)) = hints.aspect {
        let aspect_base = hints.base.unwrap_or_default();
        let width = i64::from(wanted.width) - i64::from(aspect_base.width);
        for ratio in [low, high] {
            if ratio.numerator <= 0 || ratio.denominator <= 0 {
                return None;
            }
            heights.push(
                width * i64::from(ratio.denominator) / i64::from(ratio.numerator)
                    + i64::from(aspect_base.height),
            );
        }
    }
    let mut best = None;
    let mut distance = i64::MAX;
    for height in heights {
        let height = snap(height.clamp(min_h, max_h), base.height, step.height);
        for h in [height, height + i64::from(step.height)] {
            if h < min_h || h > max_h {
                continue;
            }
            let (mut lo, mut hi) = (min_w, max_w);
            if let Some((low, high)) = hints.aspect {
                let aspect_base = hints.base.unwrap_or_default();
                let h = h - i64::from(aspect_base.height);
                let ceil = (h * i64::from(low.numerator) + i64::from(low.denominator) - 1)
                    / i64::from(low.denominator);
                lo = lo.max(ceil + i64::from(aspect_base.width));
                hi = hi.min(
                    h * i64::from(high.numerator) / i64::from(high.denominator)
                        + i64::from(aspect_base.width),
                );
            }
            if lo > hi {
                continue;
            }
            let width = snap(
                i64::from(wanted.width).clamp(lo, hi),
                base.width,
                step.width,
            );
            for w in [width, width + i64::from(step.width)] {
                if w < lo || w > hi {
                    continue;
                }
                let candidate = SizeI {
                    width: w as i32,
                    height: h as i32,
                };
                let d = (w - i64::from(wanted.width)).abs() + (h - i64::from(wanted.height)).abs();
                if d < distance && hints.accepts_size(candidate) {
                    best = Some(candidate);
                    distance = d;
                }
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::super::client::maximize_preview_tests::test_window;
    use super::*;
    use crate::xwayland::window::Windows;

    fn id() -> XWindow {
        let mut registry = Windows::new(1, 16, 1, 2, 100).unwrap();
        let token = registry.begin_inspection(10).unwrap().unwrap();
        registry
            .finish_inspection(
                token,
                1,
                Geometry {
                    x: 100,
                    y: 100,
                    width: 640,
                    height: 480,
                    border: 0,
                },
                false,
                true,
            )
            .unwrap();
        registry.get(10).unwrap().id
    }

    #[test]
    fn app_header_move_requires_a_live_press_owned_by_that_surface() {
        use crate::compositor_wayland::{
            ButtonState, ClientId, PointerFocus, SeatCapabilities, SeatState,
        };
        let surface = WaylandSurfaceId::from_raw(1).unwrap();
        let other = WaylandSurfaceId::from_raw(2).unwrap();
        let mut seat = SeatState::new(
            "test",
            SeatCapabilities {
                pointer: true,
                keyboard: false,
                touch: false,
            },
        );
        seat.pointer_focus = Some(PointerFocus {
            client: ClientId::from_raw(1).unwrap(),
            surface,
            position: PointF::default(),
            enter_serial: 1,
        });
        assert!(!authorize_move_resize(&seat, surface, 1));
        seat.pointer_button_target(0x110, ButtonState::Pressed, false);
        assert!(authorize_move_resize(&seat, surface, 1));
        assert!(!authorize_move_resize(&seat, other, 1));
        assert!(!authorize_move_resize(&seat, surface, 3));
        assert!(!authorize_move_resize(&seat, surface, 0));
        seat.pointer_button_target(0x110, ButtonState::Released, false);
        assert!(!authorize_move_resize(&seat, surface, 1));
    }

    #[test]
    fn client_header_receives_input_while_outer_border_keeps_resize_hits() {
        let config = LinuxShellConfig::default();
        let surface = WaylandSurfaceId::from_raw(1).unwrap();
        let mut window = test_window(
            SizeI {
                width: 640,
                height: 480,
            },
            PointI { x: 100, y: 100 },
        );
        window.role = SurfaceRole::Xwayland;
        window.backend = Some(WindowBackend::X11(id()));
        apply_decorations(&mut window, false, &config);
        let content = window_content_rect(&window, window.position, &config);
        let border = PointF {
            x: window.position.x as f32 + 1.0,
            y: window.position.y as f32 + 100.0,
        };
        let windows = BTreeMap::from([(surface, window)]);
        assert_eq!(
            hit_test_decoration(
                &windows,
                &[surface],
                PointF {
                    x: content.x as f32 + 20.0,
                    y: content.y as f32 + 2.0
                },
                &config,
                &[]
            ),
            None
        );
        assert!(matches!(
            hit_test_decoration(&windows, &[surface], border, &config, &[]),
            Some((_, DecorationHit::Resize(ResizeEdge::Left)))
        ));
    }

    #[test]
    fn decoration_changes_preserve_client_geometry_management_and_scaled_extents() {
        let config = LinuxShellConfig::default();
        let surface = WaylandSurfaceId::from_raw(1).unwrap();
        for density in [1, 3] {
            let mut window = test_window(
                SizeI {
                    width: 640,
                    height: 480,
                },
                PointI { x: 100, y: 100 },
            );
            window.role = SurfaceRole::Xwayland;
            window.surface_scale = density;
            let mut adapter = X11Windows::default();
            let initial = frame_geometry(&window, &config);
            adapter.attach(id(), surface, initial, false, &mut window, &config);
            let content = frame_geometry(&window, &config);
            let extents = frame_extents(&window, &config);
            assert_eq!(
                extents[2],
                ((config.titlebar_height + config.window_border) * density) as u32
            );
            apply_decorations(&mut window, false, &config);
            assert_eq!(frame_geometry(&window, &config), content);
            assert_eq!(
                frame_extents(&window, &config),
                [(config.window_border * density) as u32; 4]
            );
            assert_eq!(window.backend, Some(WindowBackend::X11(id())));
            assert!(!window.minimized);
            adapter.attach(id(), surface, content, false, &mut window, &config);
            assert!(!window.server_decorated); // Subsequent image sync must not restore chrome.
            apply_decorations(&mut window, true, &config);
            assert_eq!(frame_geometry(&window, &config), content);
            assert_eq!(frame_extents(&window, &config), extents);
            window.fullscreen = true;
            assert_eq!(frame_extents(&window, &config), [0; 4]);
        }
    }
    #[test]
    fn decoration_toggle_keeps_maximized_outer_size_and_custom_extents() {
        let config = LinuxShellConfig::default();
        let mut window = test_window(
            SizeI {
                width: 640,
                height: 480,
            },
            PointI { x: 10, y: 20 },
        );
        window.role = SurfaceRole::Xwayland;
        window.backend = Some(WindowBackend::X11(id()));
        window.chrome_content_offset = Some(PointI { x: 7, y: 45 });
        window.chrome_outer = Some(SizeI {
            width: 659,
            height: 535,
        });
        assert_eq!(frame_extents(&window, &config), [7, 12, 45, 10]);
        window.maximized = true;
        let position = window.position;
        apply_decorations(&mut window, false, &config);
        assert_eq!(window.position, position);
        assert_eq!(
            legacy_window_outer(&window, &config),
            SizeI {
                width: 659,
                height: 535
            }
        );
        assert!(window.chrome_content_offset.is_none());
        assert!(window.chrome_outer.is_none());
        apply_decorations(&mut window, true, &config);
        assert_eq!(window.position, position);
        assert_eq!(
            legacy_window_outer(&window, &config),
            SizeI {
                width: 659,
                height: 535
            }
        );
    }

    #[test]
    fn preferred_pixel_minimum_preserves_grid_aspect_and_fixed_exceptions() {
        use crate::xwayland::normal_hints::{AspectRatio, NormalHints};
        let wanted = SizeI {
            width: 300,
            height: 300,
        };
        let preferred = SizeI {
            width: 900,
            height: 600,
        }; // 300x200 logical at 3x
        let available = SizeI {
            width: 1800,
            height: 1200,
        };
        let ordinary = preferred_size(NormalHints::default(), wanted, preferred, available);
        assert_eq!(ordinary, preferred);
        let fixed = NormalHints {
            minimum: Some(wanted),
            maximum: Some(wanted),
            ..Default::default()
        };
        assert_eq!(
            preferred_size(fixed, preferred, preferred, available),
            wanted
        );
        let grid = NormalHints {
            base: Some(SizeI {
                width: 2,
                height: 4,
            }),
            increment: Some(SizeI {
                width: 8,
                height: 16,
            }),
            aspect: Some((
                AspectRatio {
                    numerator: 1,
                    denominator: 1,
                },
                AspectRatio {
                    numerator: 2,
                    denominator: 1,
                },
            )),
            ..Default::default()
        };
        let size = preferred_size(grid, wanted, preferred, available);
        assert!(grid.accepts_size(size));
        assert!(size.width >= preferred.width && size.height >= preferred.height);
        assert!(size.width <= available.width && size.height <= available.height);
        let invalid = NormalHints {
            minimum: Some(preferred),
            maximum: Some(wanted),
            ..Default::default()
        };
        assert_eq!(
            preferred_size(invalid, wanted, preferred, available),
            preferred
        );
    }
    #[test]
    fn dense_x11_geometry_round_trips_without_scaling_chrome_or_input_twice() {
        let config = LinuxShellConfig::default();
        let mut window = test_window(
            SizeI {
                width: 900,
                height: 600,
            },
            PointI::default(),
        );
        window.role = SurfaceRole::Xwayland;
        window.surface_scale = 3;
        window.presentation.size = SizeI {
            width: 900,
            height: 600,
        };
        let geometry = Geometry {
            x: 300,
            y: 300,
            width: 900,
            height: 600,
            border: 0,
        };
        let surface = WaylandSurfaceId::from_raw(1).unwrap();
        let mut adapter = X11Windows::default();
        adapter.attach(id(), surface, geometry, false, &mut window, &config);
        assert_eq!(
            window.requested_size,
            SizeI {
                width: 300,
                height: 200
            }
        );
        assert_eq!(frame_geometry(&window, &config), geometry);
        let placement = surface_placement(&window, window.position, &config);
        assert_eq!(
            placement.target,
            RectI {
                x: 100,
                y: 100,
                width: 300,
                height: 200
            }
        );
        let desktop = PointF {
            x: 110.5,
            y: 120.25,
        };
        assert_eq!(
            placement.surface_local(desktop),
            PointF { x: 31.5, y: 60.75 }
        );
        assert_eq!(
            placement.output_position(placement.surface_local(desktop)),
            desktop
        );
        adapter.attach(
            id(),
            surface,
            Geometry {
                x: -90,
                y: 150,
                ..geometry
            },
            true,
            &mut window,
            &config,
        );
        assert_eq!(window.position, PointI { x: -30, y: 50 });
        assert!(!window.server_decorated);
    }
    #[test]
    fn resize_preview_waits_for_checked_matching_new_content_and_can_be_superseded() {
        let old = SizeI {
            width: 640,
            height: 480,
        };
        let target = SizeI {
            width: 800,
            height: 600,
        };
        let mut preview = ResizePreview::default();
        preview.begin(PointI::default(), old, ResizeEdge::BottomRight);
        assert!(preview.active() && preview.dragging());
        assert!(!preview.settle(target, target, 9, false));
        preview.finish();
        preview.submitted(target, 7, false);
        assert!(!preview.dragging());
        assert!(!preview.settle(target, target, 8, true));
        assert!(!preview.settle(old, target, 8, false));
        assert!(!preview.settle(target, old, 8, false));
        assert!(!preview.settle(target, target, 7, false));
        assert!(preview.settle(target, target, 8, false));
        assert!(!preview.active());

        preview.begin(PointI::default(), target, ResizeEdge::TopLeft);
        preview.finish();
        preview.submitted(old, 9, false);
        // Another grab invalidates the old final target, even if it arrives late.
        preview.begin(PointI::default(), old, ResizeEdge::BottomRight);
        assert!(!preview.settle(old, old, 10, false));
        preview.finish();
        assert!(!preview.settle(old, old, 10, false));
        preview.submitted(target, 10, false);
        assert!(!preview.settle(old, old, 11, false));
        assert!(preview.settle(target, target, 11, false));
    }

    #[test]
    fn no_op_resize_can_settle_without_a_new_buffer() {
        let size = SizeI {
            width: 640,
            height: 480,
        };
        let mut preview = ResizePreview::default();
        preview.begin(PointI::default(), size, ResizeEdge::BottomRight);
        preview.finish();
        preview.submitted(size, 7, true);
        assert!(!preview.settle(size, size, 7, true));
        assert!(preview.settle(size, size, 7, false));
    }

    #[test]
    fn x11_final_resize_keeps_frame_paced_clients_running_behind_the_veil() {
        let mut window = test_window(
            SizeI {
                width: 640,
                height: 480,
            },
            PointI::default(),
        );
        window.backend = Some(WindowBackend::X11(id()));
        window.role = SurfaceRole::Xwayland;
        assert!(!window.waiting_for_resize_content());
        window.resize_preview.begin(
            window.position,
            window.requested_size,
            ResizeEdge::BottomRight,
        );
        assert!(!window.waiting_for_resize_content());
        window.resize_preview.finish();
        assert!(window.waiting_for_resize_content());
        assert!(window.native_configure.resize_final.is_none());
        window
            .resize_preview
            .submitted(window.requested_size, 7, false);
        assert!(!window.resize_preview.settle(
            window.requested_size,
            window.requested_size,
            7,
            false
        ));
        assert!(window.waiting_for_resize_content());
        assert!(window.resize_preview.settle(
            window.requested_size,
            window.requested_size,
            8,
            false
        ));
        assert!(!window.waiting_for_resize_content());
    }

    #[test]
    fn diagnostic_hold_preserves_completion_checks_and_has_a_wakeup_deadline() {
        let now = Instant::now();
        let size = SizeI {
            width: 800,
            height: 600,
        };
        let deadline = now + Duration::from_secs(60);
        let mut preview = ResizePreview::Settling {
            anchor: None,
            target: Some(ResizeTarget {
                size,
                revision_after: Some(7),
            }),
            not_before: Some(deadline),
        };
        assert_eq!(preview.hold_deadline(now), Some(deadline));
        assert!(!preview.settle(size, size, 8, false));
        if let ResizePreview::Settling { not_before, .. } = &mut preview {
            *not_before = Some(now - Duration::from_secs(1));
        }
        assert_eq!(preview.hold_deadline(now), None);
        assert!(!preview.settle(size, size, 8, true));
        assert!(!preview.settle(size, size, 7, false));
        assert!(preview.settle(size, size, 8, false));
    }

    #[test]
    fn shared_titlebar_drag_and_coordinates_work_for_both_backends() {
        let id = id();
        let surface = WaylandSurfaceId::from_raw(20).unwrap();
        let config = LinuxShellConfig::default();
        let mut adapter = X11Windows::default();
        for backend in [WindowBackend::Wayland, WindowBackend::X11(id)] {
            let mut image = test_window(
                SizeI {
                    width: 640,
                    height: 480,
                },
                PointI { x: 100, y: 80 },
            );
            image.backend = Some(backend);
            if matches!(backend, WindowBackend::X11(_)) {
                image.role = SurfaceRole::Xwayland;
                image.backend = None;
                adapter.attach(
                    id,
                    surface,
                    Geometry {
                        x: 100,
                        y: 100,
                        width: 640,
                        height: 480,
                        border: 0,
                    },
                    false,
                    &mut image,
                    &config,
                );
            }
            let original = image.position;
            let pointer = PointF {
                x: original.x as f32 + 120.0,
                y: original.y as f32 + config.window_border as f32 + 8.0,
            };
            let mut windows = BTreeMap::from([(surface, image)]);
            assert!(matches!(
                hit_test_decoration(&windows, &[surface], pointer, &config, &[]),
                Some((_, DecorationHit::Titlebar))
            ));
            let mut drag = WindowInteraction::begin_move(&windows, surface, pointer).unwrap();
            let mut scheduler = ConfigureScheduler::default();
            apply_window_interaction(
                &mut windows,
                &mut drag,
                &mut scheduler,
                PointF {
                    x: pointer.x + 55.0,
                    y: pointer.y + 42.0,
                },
                SizeI {
                    width: 1920,
                    height: 1080,
                },
                &config,
            )
            .unwrap();
            assert_eq!(
                windows[&surface].position,
                PointI {
                    x: original.x + 55,
                    y: original.y + 42
                }
            );
            if matches!(backend, WindowBackend::X11(_)) {
                let window = windows.get_mut(&surface).unwrap();
                let moved = window.position;
                // Delayed server notifications must not undo compositor policy.
                adapter.attach(
                    id,
                    surface,
                    Geometry {
                        x: 100,
                        y: 100,
                        width: 640,
                        height: 480,
                        border: 0,
                    },
                    false,
                    window,
                    &config,
                );
                assert_eq!(window.position, moved);
                let geometry = frame_geometry(window, &config);
                let offset = window_content_offset(window, &config);
                assert_eq!(i32::from(geometry.x), moved.x + offset.x);
                assert_eq!(i32::from(geometry.y), moved.y + offset.y);
                let local = surface_local_position(
                    &windows,
                    surface,
                    PointF {
                        x: geometry.x as f32 + 10.0,
                        y: geometry.y as f32 + 12.0,
                    },
                    &config,
                );
                assert_eq!(local, PointF { x: 10.0, y: 12.0 });
            }
            finish_window_interaction(&mut windows, &mut scheduler, drag);
            assert!(scheduler.drain().next().is_none());
        }
    }

    #[test]
    fn x11_resize_maximize_and_minimize_do_not_create_xdg_transactions() {
        let id = id();
        let surface = WaylandSurfaceId::from_raw(20).unwrap();
        let config = LinuxShellConfig::default();
        let mut adapter = X11Windows::default();
        let original = Geometry {
            x: 100,
            y: 100,
            width: 640,
            height: 480,
            border: 0,
        };
        let mut image = test_window(
            SizeI {
                width: 640,
                height: 480,
            },
            PointI::default(),
        );
        image.role = SurfaceRole::Xwayland;
        image.backend = None;
        adapter.attach(id, surface, original, false, &mut image, &config);
        let mut windows = BTreeMap::from([(surface, image)]);
        let mut scheduler = ConfigureScheduler::default();
        let mut drag = WindowInteraction::begin_resize(
            &mut windows,
            &mut scheduler,
            surface,
            ResizeEdge::BottomRight,
            PointF::default(),
        )
        .unwrap();
        apply_window_interaction(
            &mut windows,
            &mut drag,
            &mut scheduler,
            PointF { x: 80.0, y: 60.0 },
            SizeI {
                width: 1920,
                height: 1080,
            },
            &config,
        )
        .unwrap();
        finish_window_interaction(&mut windows, &mut scheduler, drag);
        assert_eq!(
            windows[&surface].requested_size,
            SizeI {
                width: 720,
                height: 540
            }
        );
        assert!(windows[&surface].native_configure.resize_anchor.is_none());
        assert!(windows[&surface].native_configure.resize_final.is_none());
        let saved = (windows[&surface].position, windows[&surface].requested_size);
        let work = RectI {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        set_window_maximized(&mut windows, &mut scheduler, surface, true, work, &config).unwrap();
        assert!(windows[&surface].native_configure.resize_final.is_none());
        let maximized_size = windows[&surface].requested_size;
        let preview = &mut windows.get_mut(&surface).unwrap().resize_preview;
        assert!(preview.active());
        preview.submitted(maximized_size, 10, false);
        assert!(preview.settle(maximized_size, maximized_size, 11, false));
        set_window_maximized(&mut windows, &mut scheduler, surface, false, work, &config).unwrap();
        let preview = &mut windows.get_mut(&surface).unwrap().resize_preview;
        assert!(preview.active());
        preview.submitted(saved.1, 11, false);
        assert!(!preview.settle(saved.1, maximized_size, 12, false));
        assert!(!preview.settle(saved.1, saved.1, 11, false));
        assert!(preview.settle(saved.1, saved.1, 12, false));
        assert_eq!(
            (windows[&surface].position, windows[&surface].requested_size),
            saved
        );
        // Restoring through a titlebar drag uses the same completion gate.
        set_window_maximized(&mut windows, &mut scheduler, surface, true, work, &config).unwrap();
        windows.get_mut(&surface).unwrap().resize_preview = ResizePreview::Idle;
        let mut drag =
            WindowInteraction::begin_move(&windows, surface, PointF { x: 100.0, y: 10.0 }).unwrap();
        apply_window_interaction(
            &mut windows,
            &mut drag,
            &mut scheduler,
            PointF { x: 140.0, y: 50.0 },
            SizeI {
                width: 1920,
                height: 1080,
            },
            &config,
        )
        .unwrap();
        assert!(!windows[&surface].maximized);
        assert_eq!(windows[&surface].requested_size, saved.1);
        assert!(windows[&surface].resize_preview.active());
        assert!(windows[&surface].native_configure.resize_final.is_none());
        windows.get_mut(&surface).unwrap().minimized = true;
        adapter.attach(
            id,
            surface,
            original,
            false,
            windows.get_mut(&surface).unwrap(),
            &config,
        );
        assert!(windows[&surface].minimized);
    }

    #[test]
    fn unmanaged_popups_remain_unframed() {
        let mut adapter = X11Windows::default();
        let mut image = test_window(
            SizeI {
                width: 100,
                height: 100,
            },
            PointI::default(),
        );
        image.role = SurfaceRole::Xwayland;
        adapter.attach(
            id(),
            WaylandSurfaceId::from_raw(20).unwrap(),
            Geometry {
                x: 40,
                y: 50,
                width: 100,
                height: 100,
                border: 0,
            },
            true,
            &mut image,
            &LinuxShellConfig::default(),
        );
        assert_eq!(image.backend, None);
        assert!(!window_is_decorated(&image));
        assert_eq!(image.position, PointI { x: 40, y: 50 });
        let native = WaylandSurfaceId::from_raw(21).unwrap();
        let popup = WaylandSurfaceId::from_raw(20).unwrap();
        let mut windows = BTreeMap::from([
            (popup, image),
            (
                native,
                test_window(
                    SizeI {
                        width: 640,
                        height: 480,
                    },
                    PointI { x: 40, y: 50 },
                ),
            ),
        ]);
        let point = PointF { x: 100.0, y: 60.0 };
        assert!(
            hit_test_decoration(
                &windows,
                &[native, popup],
                point,
                &LinuxShellConfig::default(),
                &[]
            )
            .is_none()
        );
        windows.remove(&popup);
        assert!(matches!(
            hit_test_decoration(
                &windows,
                &[native],
                point,
                &LinuxShellConfig::default(),
                &[]
            ),
            Some((_, DecorationHit::Titlebar))
        ));
    }

    #[test]
    fn size_constraints_project_to_client_grid_and_aspect() {
        use crate::xwayland::normal_hints::{AspectRatio, NormalHints};
        let hints = NormalHints {
            minimum: Some(SizeI {
                width: 80,
                height: 40,
            }),
            maximum: Some(SizeI {
                width: 800,
                height: 600,
            }),
            increment: Some(SizeI {
                width: 8,
                height: 16,
            }),
            ..Default::default()
        };
        let size = constrained_size(
            hints,
            SizeI {
                width: 333,
                height: 217,
            },
        )
        .unwrap();
        assert!(hints.accepts_size(size));
        assert!((size.width - 333).abs() <= 8 && (size.height - 217).abs() <= 16);
        let square = NormalHints {
            aspect: Some((
                AspectRatio {
                    numerator: 1,
                    denominator: 1,
                },
                AspectRatio {
                    numerator: 1,
                    denominator: 1,
                },
            )),
            ..Default::default()
        };
        let size = constrained_size(
            square,
            SizeI {
                width: 300,
                height: 150,
            },
        )
        .unwrap();
        assert_eq!(size.width, size.height);
        assert!(square.accepts_size(size));
    }
}

fn notification_geometry(actual: Geometry, requested_border: u16) -> Geometry {
    let adjust = |value: i16| {
        (i32::from(value) + i32::from(actual.border) - i32::from(requested_border))
            .clamp(i16::MIN.into(), i16::MAX.into()) as i16
    };
    Geometry {
        x: adjust(actual.x),
        y: adjust(actual.y),
        border: requested_border,
        ..actual
    }
}

#[cfg(test)]
mod configure_reply_tests {
    use super::*;
    #[test]
    fn configure_reply_does_not_require_a_wayland_image() {
        use crate::xwayland::xwm::decoration_tests::wire_request as request;
        use crate::xwayland::xwm::decoration_tests::{drive, fixture};
        let (mut xwm, mut peer, now, id) = fixture();
        let mut adapter = X11Windows::default();
        // A previous image's adapter may remain while the X window is unmapped.
        adapter.entries.insert(
            id,
            Entry {
                surface: WaylandSurfaceId::from_raw(100).unwrap(),
                sent: None,
                notify_border: None,
                requested_border: 5,
            },
        );
        adapter.configure_requested(id, RequestedConfigure::default());
        let mut submitted = 0;
        adapter.flush_unattached(&mut xwm, &mut submitted).unwrap();
        assert_eq!(submitted, 1);
        drive(&mut xwm, now);
        let bytes = request(&mut peer);
        assert_eq!(bytes[0], 25); // SendEvent, with no preceding ConfigureWindow for a no-op.
        assert_eq!(bytes[12] & 0x7f, 22); // ConfigureNotify.
        assert_eq!(
            u32::from_ne_bytes(bytes[20..24].try_into().unwrap()),
            id.xid
        );
        assert!(adapter.requests.is_empty());
        assert!(adapter.unattached_replies.is_empty());
    }

    #[test]
    fn denied_and_no_op_requests_keep_actual_geometry_and_last_requested_border() {
        use super::super::client::maximize_preview_tests::test_window;
        let surface = WaylandSurfaceId::from_raw(10).unwrap();
        let id = XWindow {
            generation: 1,
            xid: 100,
            incarnation: 1,
        };
        let geometry = Geometry {
            x: 10,
            y: 20,
            width: 640,
            height: 480,
            border: 0,
        };
        let config = LinuxShellConfig::default();
        let mut window = test_window(
            SizeI {
                width: 640,
                height: 480,
            },
            PointI::default(),
        );
        window.role = SurfaceRole::Xwayland;
        let mut adapter = X11Windows::default();
        adapter.attach(id, surface, geometry, false, &mut window, &config);
        window.maximized = true;
        let original = (window.position, window.requested_size);
        adapter.configure_requested(
            id,
            RequestedConfigure {
                x: Some(50),
                width: Some(320),
                border: Some(5),
                ..Default::default()
            },
        );
        adapter.attach(id, surface, geometry, false, &mut window, &config);
        assert_eq!((window.position, window.requested_size), original);
        assert_eq!(adapter.entries[&id].notify_border, Some(5));
        adapter.entries.get_mut(&id).unwrap().notify_border = None;
        adapter.configure_requested(id, RequestedConfigure::default());
        adapter.attach(id, surface, geometry, false, &mut window, &config);
        assert_eq!(adapter.entries[&id].notify_border, Some(5));
    }

    #[test]
    fn synthetic_coordinates_preserve_client_origin_with_requested_border() {
        let actual = Geometry {
            x: 10,
            y: 20,
            width: 640,
            height: 480,
            border: 0,
        };
        assert_eq!(
            notification_geometry(actual, 3),
            Geometry {
                x: 7,
                y: 17,
                border: 3,
                ..actual
            }
        );
        assert_eq!(notification_geometry(actual, 0), actual);
    }
}

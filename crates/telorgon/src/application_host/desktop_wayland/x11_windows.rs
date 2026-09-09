//! X11 adapter for shared frame geometry. No renderer, frame layout or pointer
//! gesture lives here; root coordinates cross this boundary exactly once.
use super::*;
use crate::xwayland::{
    association::XWindow,
    window::{Geometry, RequestedConfigure},
    xwm::Xwm,
};

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
        *self = Self::Settling {
            anchor,
            target: None,
        };
    }
    fn anchor(self) -> Option<ResizeAnchor> {
        match self {
            Self::Dragging { anchor } => Some(anchor),
            Self::Settling { anchor, .. } => anchor,
            Self::Idle => None,
        }
    }
    fn submitted(&mut self, size: SizeI, revision: u64, matching_content: bool) {
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
        let Self::Settling {
            target: Some(target),
            ..
        } = *self
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
        true
    }
}

#[derive(Default)]
pub(super) struct X11Windows {
    entries: BTreeMap<XWindow, Entry>,
    requests: BTreeMap<XWindow, RequestedConfigure>,
}
struct Entry {
    surface: WaylandSurfaceId,
    sent: Option<Geometry>,
}

impl X11Windows {
    pub(super) fn retain(&mut self, live: &BTreeSet<XWindow>) {
        self.entries.retain(|id, _| live.contains(id));
        self.requests.retain(|id, _| live.contains(id));
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
        config: &LinuxDesktopConfig,
    ) {
        if unmanaged {
            window.resize_preview = Default::default();
            window.backend = None;
            window.server_decorated = false;
            window.chrome = None;
            window.chrome_outer = None;
            window.chrome_content_offset = None;
            window.position = PointI {
                x: geometry.x.into(),
                y: geometry.y.into(),
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
        window.server_decorated = true;
        if fresh {
            window.resize_preview = Default::default();
            let offset = window_content_offset(window, config);
            window.position = PointI {
                x: i32::from(geometry.x).saturating_sub(offset.x).max(0),
                y: i32::from(geometry.y).saturating_sub(offset.y).max(0),
            };
            window.requested_size = SizeI {
                width: geometry.width.into(),
                height: geometry.height.into(),
            };
            window.minimized = false;
            self.entries.insert(
                id,
                Entry {
                    surface,
                    sent: None,
                },
            );
        }
        if let Some(request) = self.requests.remove(&id) {
            // Explicit client configure requests enter shared policy. A server
            // ConfigureNotify is confirmation, not authority to undo a drag.
            if !window.maximized && !window.fullscreen && !window.resize_preview.active() {
                let offset = window_content_offset(window, config);
                if let Some(x) = request.x {
                    window.position.x = i32::from(x) - offset.x;
                }
                if let Some(y) = request.y {
                    window.position.y = i32::from(y) - offset.y;
                }
                if let Some(width) = request.width {
                    window.requested_size.width = i32::from(width.max(1));
                }
                if let Some(height) = request.height {
                    window.requested_size.height = i32::from(height.max(1));
                }
            }
        }
    }

    pub(super) fn flush(
        &mut self,
        xwm: &mut Xwm,
        windows: &mut BTreeMap<WaylandSurfaceId, ClientWindow>,
        config: &LinuxDesktopConfig,
    ) -> crate::xwayland::Result<bool> {
        let mut changed = false;
        let started = Instant::now();
        let mut submitted = 0;
        for (id, entry) in &mut self.entries {
            if submitted == 16 || started.elapsed() >= Duration::from_millis(1) {
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
            if window.resize_preview.dragging() {
                continue;
            }
            if !window.maximized && !window.fullscreen {
                if let Some(hints) = xwm.normal_hints(*id) {
                    let fallback =
                        xwm.windows()
                            .and_then(|registry| registry.get(id.xid))
                            .map(|actual| SizeI {
                                width: actual.geometry.width.into(),
                                height: actual.geometry.height.into(),
                            });
                    let previous_size = window.requested_size;
                    window.requested_size = constrained_size(hints, window.requested_size)
                        .or(fallback)
                        .unwrap_or(window.requested_size);
                    changed |= previous_size != window.requested_size;
                }
            }
            if let Some(anchor) = window.resize_preview.anchor() {
                let previous = window.position;
                window.position = anchor.reconcile_position(window.position, window.requested_size);
                changed |= previous != window.position;
            }
            let desired = frame_geometry(window, config);
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
                // Register the image revision before issuing the resize request.
                window.resize_preview.submitted(
                    target_size,
                    window.presentation.revision,
                    server_size == target_size && window.presentation.size == target_size,
                );
                xwm.configure_window(*id, desired, Instant::now())?;
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
            changed |= window.resize_preview.settle(
                server_size,
                window.presentation.size,
                window.presentation.revision,
                xwm.commands_pending(*id),
            );
        }
        Ok(changed)
    }
}

fn frame_geometry(window: &ClientWindow, config: &LinuxDesktopConfig) -> Geometry {
    let content = window_content_rect(window, window.position, config);
    Geometry {
        x: content.x.clamp(i16::MIN.into(), i16::MAX.into()) as i16,
        y: content.y.clamp(i16::MIN.into(), i16::MAX.into()) as i16,
        width: content.width.clamp(1, u16::MAX.into()) as u16,
        height: content.height.clamp(1, u16::MAX.into()) as u16,
        // The compositor supplies the border; an X11 server border would be a
        // second frame and would shift the surface/input origin.
        border: 0,
    }
}

/// Bounded grid/aspect projection. Candidate count is independent of client
/// dimensions; inconsistent hints fall back to confirmed server size.
fn constrained_size(
    hints: crate::xwayland::normal_hints::NormalHints,
    wanted: SizeI,
) -> Option<SizeI> {
    if hints.accepts_size(wanted) {
        return Some(wanted);
    }
    let minimum = hints.minimum_size();
    let maximum = hints.maximum.unwrap_or(SizeI {
        width: 65535,
        height: 65535,
    });
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
    fn shared_titlebar_drag_and_coordinates_work_for_both_backends() {
        let id = id();
        let surface = WaylandSurfaceId::from_raw(20).unwrap();
        let config = LinuxDesktopConfig::default();
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
        let config = LinuxDesktopConfig::default();
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
            &LinuxDesktopConfig::default(),
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
                &LinuxDesktopConfig::default(),
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
                &LinuxDesktopConfig::default(),
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

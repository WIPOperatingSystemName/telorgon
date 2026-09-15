use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::compositor_wayland::{ResizeEdge, WaylandSurfaceId, XdgConfigure};
use crate::core::{PointF, PointI, RectI, SizeI};

/// A pending page flip may overlap one render, but don't build a queue behind GPU/ready work.
pub(super) fn primary_render_budget(gpu_pending: usize, ready_pending: usize) -> bool {
    gpu_pending == 0 && ready_pending == 0
}

/// Native xdg configure transaction state. Desktop geometry and retained surface
/// imagery are deliberately outside this record. X11 geometry commands have no
/// xdg acknowledgement or terminal-configure transaction to store here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct NativeConfigureState {
    pub(super) resize_anchor: Option<ResizeAnchor>,
    pub(super) resize_final: Option<FinalResizeConfigure>,
}

/// Removes the next deferred SHM surface that has neither an in-flight copy nor a resize pause.
///
/// A submitted surface remains in the queue so its latest deferred revision can be retried after
/// that surface's completion is observed. The bounded pass prevents an all-blocked queue from
/// spinning while still allowing unrelated surfaces behind a blocked entry to make progress.
pub(super) fn take_ready_deferred_shm_surface(
    deferred: &mut VecDeque<WaylandSurfaceId>,
    blocked: &BTreeSet<WaylandSurfaceId>,
) -> Option<WaylandSurfaceId> {
    let candidate_count = deferred.len();
    for _ in 0..candidate_count {
        let surface = deferred.pop_front()?;
        if blocked.contains(&surface) {
            deferred.push_back(surface);
        } else {
            return Some(surface);
        }
    }
    None
}

/// The one configure that should be emitted for a surface at the end of an input turn.
/// Replacing an entry is intentional: raw motion is compositor-local preview state, not a
/// one-configure-per-event protocol stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PendingResizeConfigure {
    pub surface: WaylandSurfaceId,
    pub size: SizeI,
    pub resizing: bool,
}

#[derive(Debug, Default)]
pub(super) struct ConfigureScheduler {
    pending: BTreeMap<WaylandSurfaceId, PendingResizeConfigure>,
}

/// The terminal configure for an interactive resize or a maximization.
///
/// The size is known when the pointer grab ends, but the protocol serial does not exist until the
/// scheduler emits the configure. Keeping both prevents an older activation configure with the
/// same size from being mistaken for the end of the resize transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FinalResizeConfigure {
    pub size: SizeI,
    pub serial: Option<u32>,
    acknowledged: bool,
}

impl FinalResizeConfigure {
    pub fn pending(size: SizeI) -> Self {
        Self {
            size,
            serial: None,
            acknowledged: false,
        }
    }

    pub fn record_sent(&mut self, size: SizeI, serial: u32) {
        if self.size == size && self.serial.is_none() {
            self.serial = Some(serial);
        }
    }

    /// Retains acknowledgement independently from asynchronous image publication. A later SHM
    /// commit can supersede the worker result that carried this acknowledgement, but it still
    /// consumes the terminal configure and therefore completes the protocol transaction.
    pub fn observe_acknowledgement(&mut self, acknowledged: Option<XdgConfigure>) {
        self.acknowledged |= self
            .serial
            .zip(acknowledged)
            .is_some_and(|(serial, configure)| serial_was_superseded_by(serial, configure.serial));
    }

    pub fn was_acknowledged(self) -> bool {
        self.acknowledged
    }
}

/// Wayland serials are unsigned and may wrap. Protocol traffic cannot span half the serial space,
/// so a wrapping subtraction in the lower half identifies the same or a newer serial.
fn serial_was_superseded_by(serial: u32, acknowledged: u32) -> bool {
    acknowledged.wrapping_sub(serial) < (1_u32 << 31)
}

impl ConfigureScheduler {
    pub fn schedule_state(&mut self, surface: WaylandSurfaceId, size: SizeI, resizing: bool) {
        self.pending.insert(
            surface,
            PendingResizeConfigure {
                surface,
                size,
                resizing,
            },
        );
    }

    pub fn schedule_resize(&mut self, surface: WaylandSurfaceId, size: SizeI) {
        self.schedule_state(surface, size, true);
    }

    pub fn schedule_final(&mut self, surface: WaylandSurfaceId, size: SizeI) {
        self.schedule_state(surface, size, false);
    }

    pub fn cancel(&mut self, surface: WaylandSurfaceId) {
        self.pending.remove(&surface);
    }

    pub fn defer(&mut self, configure: PendingResizeConfigure) {
        self.pending.insert(configure.surface, configure);
    }

    pub fn drain(&mut self) -> impl Iterator<Item = PendingResizeConfigure> + '_ {
        std::mem::take(&mut self.pending).into_values()
    }
}

/// Fixed opposite edges retained while an xdg-toplevel resize transaction is in flight.
/// This lets a cell-based client commit a size smaller than the configured maximum without
/// turning top/left resizing into window movement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ResizeAnchor {
    edge: ResizeEdge,
    fixed_right: i32,
    fixed_bottom: i32,
}

impl ResizeAnchor {
    pub fn new(position: PointI, size: SizeI, edge: ResizeEdge) -> Self {
        Self {
            edge,
            fixed_right: position.x.saturating_add(size.width),
            fixed_bottom: position.y.saturating_add(size.height),
        }
    }

    pub fn reconcile_position(self, preview_position: PointI, committed_size: SizeI) -> PointI {
        PointI {
            x: if resizes_left(self.edge) {
                self.fixed_right.saturating_sub(committed_size.width)
            } else {
                preview_position.x
            },
            y: if resizes_top(self.edge) {
                self.fixed_bottom.saturating_sub(committed_size.height)
            } else {
                preview_position.y
            },
        }
    }
}

fn resizes_left(edge: ResizeEdge) -> bool {
    matches!(
        edge,
        ResizeEdge::Left | ResizeEdge::TopLeft | ResizeEdge::BottomLeft
    )
}

fn resizes_top(edge: ResizeEdge) -> bool {
    matches!(
        edge,
        ResizeEdge::Top | ResizeEdge::TopLeft | ResizeEdge::TopRight
    )
}

/// Native surface-coordinate placement, shared by painting and input. Buffer scale, transform,
/// and viewport conversion have already been applied to `source_extent` before this boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SurfacePlacement {
    pub surface_scale: i32,
    pub target: RectI,
    pub clip: Option<RectI>,
}

impl SurfacePlacement {
    pub fn native(source_extent: SizeI, origin: PointI) -> Self {
        Self {
            surface_scale: 1,
            target: RectI {
                x: origin.x,
                y: origin.y,
                width: source_extent.width,
                height: source_extent.height,
            },
            clip: None,
        }
    }

    pub fn toplevel(
        source_extent: SizeI,
        window_geometry: RectI,
        content_slot: RectI,
        resize_anchor: Option<ResizeAnchor>,
    ) -> Self {
        // Keep the committed content against the stationary edge while the live frame follows
        // the pointer. A new buffer changes the native extent, never a texture scale factor.
        let committed = RectI {
            x: content_slot.x.saturating_add(
                if resize_anchor.is_some_and(|anchor| resizes_left(anchor.edge)) {
                    content_slot.width.saturating_sub(window_geometry.width)
                } else {
                    0
                },
            ),
            y: content_slot.y.saturating_add(
                if resize_anchor.is_some_and(|anchor| resizes_top(anchor.edge)) {
                    content_slot.height.saturating_sub(window_geometry.height)
                } else {
                    0
                },
            ),
            width: window_geometry.width,
            height: window_geometry.height,
        };
        let mut placement = Self::native(
            source_extent,
            PointI {
                x: committed.x.saturating_sub(window_geometry.x),
                y: committed.y.saturating_sub(window_geometry.y),
            },
        );
        // Intersect both geometries: growing must not expose the old buffer's shadow margins,
        // and shrinking must not let old content cover the compositor's frame or neighbours.
        placement.clip = Some(intersection(committed, content_slot).unwrap_or(RectI {
            x: content_slot.x,
            y: content_slot.y,
            width: 0,
            height: 0,
        }));
        placement
    }

    pub fn visible_rect(self) -> Option<RectI> {
        self.clip
            .map_or(Some(self.target), |clip| intersection(self.target, clip))
    }

    pub fn contains(self, position: PointF) -> bool {
        self.visible_rect().is_some_and(|rect| {
            position.x >= rect.x as f32
                && position.y >= rect.y as f32
                && position.x < rect.right() as f32
                && position.y < rect.bottom() as f32
        })
    }

    pub fn surface_local(self, position: PointF) -> PointF {
        PointF {
            x: (position.x - self.target.x as f32) * self.surface_scale as f32,
            y: (position.y - self.target.y as f32) * self.surface_scale as f32,
        }
    }

    pub fn output_position(self, position: PointF) -> PointF {
        PointF {
            x: position.x / self.surface_scale as f32 + self.target.x as f32,
            y: position.y / self.surface_scale as f32 + self.target.y as f32,
        }
    }
}

fn intersection(left: RectI, right: RectI) -> Option<RectI> {
    let x = left.x.max(right.x);
    let y = left.y.max(right.y);
    let right_edge = left.right().min(right.right());
    let bottom = left.bottom().min(right.bottom());
    (right_edge > x && bottom > y).then_some(RectI {
        x,
        y,
        width: right_edge.saturating_sub(x),
        height: bottom.saturating_sub(y),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_backpressure_resumes_after_gpu_and_ready_retirement() {
        // A flip may be pending during each of these transitions. It doesn't consume the render
        // budget itself; a completed frame waiting behind it does.
        assert!(primary_render_budget(0, 0));
        assert!(!primary_render_budget(1, 0)); // queued GPU work
        assert!(!primary_render_budget(0, 1)); // completion observed, waiting for KMS
        assert!(primary_render_budget(0, 0)); // moved to the pending flip
        assert!(!primary_render_budget(2, 1)); // tolerate a pre-existing queue during transition
    }

    #[test]
    fn native_content_does_not_stretch_or_expose_shadow_margins() {
        let placement = SurfacePlacement::toplevel(
            SizeI {
                width: 816,
                height: 620,
            },
            RectI {
                x: 8,
                y: 10,
                width: 800,
                height: 600,
            },
            RectI {
                x: 120,
                y: 80,
                width: 1000,
                height: 450,
            },
            None,
        );
        assert_eq!(
            placement.target,
            RectI {
                x: 112,
                y: 70,
                width: 816,
                height: 620
            }
        );
        assert_eq!(
            placement.clip,
            Some(RectI {
                x: 120,
                y: 80,
                width: 800,
                height: 450
            })
        );
        let output = PointF { x: 620.0, y: 305.0 };
        assert_eq!(
            placement.surface_local(output),
            PointF { x: 508.0, y: 235.0 }
        );
        assert_eq!(
            placement.output_position(placement.surface_local(output)),
            output
        );
        assert!(placement.contains(output));
        assert!(!placement.contains(PointF { x: 950.0, y: 200.0 })); // grow padding
        assert!(!placement.contains(PointF { x: 620.0, y: 530.0 })); // cropped content
        assert!(!placement.contains(PointF { x: 116.0, y: 100.0 })); // shadow margin
    }

    #[test]
    fn native_content_keeps_the_opposite_edge_for_all_eight_resize_directions() {
        let start = PointI { x: 100, y: 80 };
        let original = SizeI {
            width: 800,
            height: 600,
        };
        for (edge, left, top) in [
            (ResizeEdge::Top, false, true),
            (ResizeEdge::TopRight, false, true),
            (ResizeEdge::Right, false, false),
            (ResizeEdge::BottomRight, false, false),
            (ResizeEdge::Bottom, false, false),
            (ResizeEdge::BottomLeft, true, false),
            (ResizeEdge::Left, true, false),
            (ResizeEdge::TopLeft, true, true),
        ] {
            let anchor = ResizeAnchor::new(start, original, edge);
            for requested in [
                SizeI {
                    width: 940,
                    height: 710,
                },
                SizeI {
                    width: 650,
                    height: 490,
                },
            ] {
                let position = anchor.reconcile_position(start, requested);
                let slot = RectI {
                    x: position.x + 4,
                    y: position.y + 36,
                    width: requested.width,
                    height: requested.height,
                };
                for committed in [
                    original,
                    SizeI {
                        width: 832,
                        height: 624,
                    },
                ] {
                    let geometry = RectI {
                        x: 8,
                        y: 10,
                        width: committed.width,
                        height: committed.height,
                    };
                    let source = SizeI {
                        width: committed.width + 16,
                        height: committed.height + 20,
                    };
                    let placement =
                        SurfacePlacement::toplevel(source, geometry, slot, Some(anchor));
                    assert_eq!(
                        (placement.target.width, placement.target.height),
                        (source.width, source.height)
                    );
                    let x = placement.target.x + geometry.x;
                    let y = placement.target.y + geometry.y;
                    assert_eq!(
                        if left { x + committed.width } else { x },
                        if left { 904 } else { 104 },
                        "{edge:?}"
                    );
                    assert_eq!(
                        if top { y + committed.height } else { y },
                        if top { 716 } else { 116 },
                        "{edge:?}"
                    );
                    let final_position = anchor.reconcile_position(position, committed);
                    let final_slot = RectI {
                        x: final_position.x + 4,
                        y: final_position.y + 36,
                        width: committed.width,
                        height: committed.height,
                    };
                    let final_placement =
                        SurfacePlacement::toplevel(source, geometry, final_slot, None);
                    assert_eq!(
                        placement.target, final_placement.target,
                        "final acknowledgement must not jump {edge:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn native_non_toplevel_placement_preserves_its_input_space() {
        let placement = SurfacePlacement::native(
            SizeI {
                width: 40,
                height: 30,
            },
            PointI { x: -10, y: 20 },
        );
        assert_eq!(placement.clip, None);
        assert_eq!(placement.visible_rect(), Some(placement.target));
        assert_eq!(
            placement.surface_local(PointF { x: 4.5, y: 30.0 }),
            PointF { x: 14.5, y: 10.0 }
        );
    }

    #[test]
    fn a_paused_copy_does_not_block_other_surfaces_and_resumes_on_release() {
        let mut deferred = VecDeque::from([surface(), other_surface()]);
        let paused = BTreeSet::from([surface()]);
        assert_eq!(
            take_ready_deferred_shm_surface(&mut deferred, &paused),
            Some(other_surface())
        );
        assert_eq!(
            take_ready_deferred_shm_surface(&mut deferred, &paused),
            None
        );
        assert_eq!(deferred, VecDeque::from([surface()]));
        assert_eq!(
            take_ready_deferred_shm_surface(&mut deferred, &BTreeSet::new()),
            Some(surface())
        );
        assert!(deferred.is_empty());
    }

    fn surface() -> WaylandSurfaceId {
        WaylandSurfaceId::from_raw(7).unwrap()
    }

    fn other_surface() -> WaylandSurfaceId {
        WaylandSurfaceId::from_raw(8).unwrap()
    }

    #[test]
    fn deferred_shm_copy_waits_for_its_own_submitted_copy() {
        let mut deferred = VecDeque::from([surface(), other_surface()]);
        let submitted = BTreeSet::from([surface()]);

        assert_eq!(
            take_ready_deferred_shm_surface(&mut deferred, &submitted),
            Some(other_surface())
        );
        assert_eq!(deferred, VecDeque::from([surface()]));
        assert_eq!(
            take_ready_deferred_shm_surface(&mut deferred, &submitted),
            None
        );
        assert_eq!(deferred, VecDeque::from([surface()]));
    }

    #[test]
    fn high_rate_motion_coalesces_to_one_configure_per_turn() {
        let mut scheduler = ConfigureScheduler::default();
        for width in 1..=10_000 {
            scheduler.schedule_resize(surface(), SizeI { width, height: 480 });
        }

        assert_eq!(
            scheduler.drain().collect::<Vec<_>>(),
            vec![PendingResizeConfigure {
                surface: surface(),
                size: SizeI {
                    width: 10_000,
                    height: 480,
                },
                resizing: true,
            }]
        );
    }

    #[test]
    fn same_turn_release_replaces_motion_with_one_final_configure() {
        let mut scheduler = ConfigureScheduler::default();
        scheduler.schedule_resize(
            surface(),
            SizeI {
                width: 801,
                height: 600,
            },
        );
        scheduler.schedule_final(
            surface(),
            SizeI {
                width: 801,
                height: 600,
            },
        );

        assert_eq!(
            scheduler.drain().collect::<Vec<_>>(),
            vec![PendingResizeConfigure {
                surface: surface(),
                size: SizeI {
                    width: 801,
                    height: 600,
                },
                resizing: false,
            }]
        );
    }

    #[test]
    fn a_deferred_configure_is_replaced_by_newer_motion() {
        let mut scheduler = ConfigureScheduler::default();
        scheduler.defer(PendingResizeConfigure {
            surface: surface(),
            size: SizeI {
                width: 700,
                height: 500,
            },
            resizing: true,
        });
        scheduler.schedule_resize(
            surface(),
            SizeI {
                width: 900,
                height: 650,
            },
        );

        assert_eq!(
            scheduler.drain().collect::<Vec<_>>(),
            vec![PendingResizeConfigure {
                surface: surface(),
                size: SizeI {
                    width: 900,
                    height: 650,
                },
                resizing: true,
            }]
        );
    }

    #[test]
    fn destroyed_surface_cancels_its_pending_configure() {
        let mut scheduler = ConfigureScheduler::default();
        scheduler.schedule_resize(
            surface(),
            SizeI {
                width: 900,
                height: 700,
            },
        );
        scheduler.cancel(surface());
        assert!(scheduler.drain().next().is_none());
    }

    #[test]
    fn final_resize_does_not_complete_before_its_configure_is_sent() {
        let size = SizeI {
            width: 801,
            height: 603,
        };
        assert!(!FinalResizeConfigure::pending(size).was_acknowledged());
    }

    #[test]
    fn the_final_or_a_newer_acked_configure_can_complete_a_resize() {
        use crate::compositor_wayland::{DecorationMode, ToplevelState};

        let size = SizeI {
            width: 801,
            height: 603,
        };
        let configure = |serial| XdgConfigure {
            serial,
            size: Some(size),
            bounds: None,
            states: ToplevelState::default(),
            decoration: DecorationMode::ServerSide,
        };
        let mut final_resize = FinalResizeConfigure::pending(size);
        final_resize.record_sent(size, 4);

        final_resize.observe_acknowledgement(Some(configure(3)));
        assert!(!final_resize.was_acknowledged());
        final_resize.observe_acknowledgement(Some(configure(4)));
        assert!(final_resize.was_acknowledged());

        let mut newer = FinalResizeConfigure::pending(size);
        newer.record_sent(size, 4);
        newer.observe_acknowledgement(Some(configure(5)));
        assert!(newer.was_acknowledged());
    }

    #[test]
    fn final_configure_comparison_handles_serial_wraparound() {
        use crate::compositor_wayland::{DecorationMode, ToplevelState};

        let size = SizeI {
            width: 801,
            height: 603,
        };
        let configure = |serial| XdgConfigure {
            serial,
            size: Some(size),
            bounds: None,
            states: ToplevelState::default(),
            decoration: DecorationMode::ServerSide,
        };
        let mut final_resize = FinalResizeConfigure::pending(size);
        final_resize.record_sent(size, u32::MAX);

        final_resize.observe_acknowledgement(Some(configure(1)));
        assert!(final_resize.was_acknowledged());

        let mut older = FinalResizeConfigure::pending(size);
        older.record_sent(size, u32::MAX);
        older.observe_acknowledgement(Some(configure(u32::MAX - 1)));
        assert!(!older.was_acknowledged());
    }

    #[test]
    fn acknowledgement_survives_a_superseded_image_publication() {
        use crate::compositor_wayland::{DecorationMode, ToplevelState};

        let final_size = SizeI {
            width: 960,
            height: 640,
        };
        let configure = XdgConfigure {
            serial: 12,
            size: Some(final_size),
            bounds: None,
            states: ToplevelState::default(),
            decoration: DecorationMode::ServerSide,
        };
        let mut final_resize = FinalResizeConfigure::pending(final_size);
        final_resize.record_sent(final_size, configure.serial);

        final_resize.observe_acknowledgement(Some(configure));
        final_resize.observe_acknowledgement(None);

        assert!(final_resize.was_acknowledged());
    }

    #[test]
    fn newer_configure_supersedes_the_terminal_resize_for_cell_sized_clients() {
        use crate::compositor_wayland::{DecorationMode, ToplevelState};

        let final_size = SizeI {
            width: 960,
            height: 640,
        };
        let configure = XdgConfigure {
            serial: 13,
            size: Some(SizeI {
                width: 800,
                height: 600,
            }),
            bounds: None,
            states: ToplevelState::default(),
            decoration: DecorationMode::ServerSide,
        };
        let mut final_resize = FinalResizeConfigure::pending(final_size);
        final_resize.record_sent(final_size, 12);

        final_resize.observe_acknowledgement(Some(configure));

        assert!(final_resize.was_acknowledged());
    }

    #[test]
    fn left_and_top_edges_stay_fixed_when_client_snaps_size() {
        let anchor = ResizeAnchor::new(
            PointI { x: 100, y: 80 },
            SizeI {
                width: 800,
                height: 600,
            },
            ResizeEdge::TopLeft,
        );
        assert_eq!(
            anchor.reconcile_position(
                PointI { x: 43, y: 29 },
                SizeI {
                    width: 832,
                    height: 624,
                },
            ),
            PointI { x: 68, y: 56 }
        );
    }
    #[test]
    fn measured_maximize_size_requires_its_own_configure_acknowledgement() {
        use crate::compositor_wayland::{DecorationMode, ToplevelState};
        let fallback = SizeI {
            width: 1272,
            height: 760,
        };
        let measured = SizeI {
            width: 1280,
            height: 776,
        };
        let ack = |serial, size| XdgConfigure {
            serial,
            size: Some(size),
            bounds: None,
            states: ToplevelState {
                maximized: true,
                ..ToplevelState::default()
            },
            decoration: DecorationMode::ServerSide,
        };
        let mut transition = FinalResizeConfigure::pending(fallback);
        transition.record_sent(fallback, 10);
        transition = FinalResizeConfigure::pending(measured);
        transition.observe_acknowledgement(Some(ack(10, fallback)));
        assert!(!transition.was_acknowledged());
        transition.record_sent(measured, 11);
        transition.observe_acknowledgement(Some(ack(10, fallback)));
        assert!(!transition.was_acknowledged());
        transition.observe_acknowledgement(Some(ack(11, measured)));
        assert!(transition.was_acknowledged());
    }
}

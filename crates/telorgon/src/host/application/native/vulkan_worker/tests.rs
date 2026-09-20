use super::*;
use crate::host::application::native::resize::ResizeInteractionPhase;
use crate::graphics::render::RenderScene;

fn resize(
    generation: u64,
    phase: ResizeInteractionPhase,
    extent: SizeI,
    surface: SurfaceResizeAction,
) -> ResizeUpdate {
    ResizeUpdate {
        generation,
        metrics_revision: generation.max(1),
        phase,
        extent,
        surface,
    }
}

#[test]
fn mailbox_keeps_only_the_newest_extent_and_bounds_scene_deltas() {
    let mut mailbox = WorkerMailbox::new();
    mailbox.resize = Some(resize(
        4,
        ResizeInteractionPhase::Updating,
        SizeI {
            width: 800,
            height: 600,
        },
        SurfaceResizeAction::KeepCurrent,
    ));
    mailbox.resize = Some(resize(
        4,
        ResizeInteractionPhase::Ended,
        SizeI {
            width: 1280,
            height: 720,
        },
        SurfaceResizeAction::CommitAfterPreview,
    ));
    let mut scene = RenderScene::default();
    for _ in 0..8 {
        scene.damage.full = true;
        mailbox.deltas.push(scene.take_delta().unwrap());
    }
    let batch = mailbox.take(false);
    assert_eq!(
        batch.resize,
        Some(resize(
            4,
            ResizeInteractionPhase::Ended,
            SizeI {
                width: 1280,
                height: 720,
            },
            SurfaceResizeAction::CommitAfterPreview,
        ))
    );
    assert!(batch.deltas.len() <= WORKER_DELTA_CAPACITY);
    assert_eq!(batch.deltas.last().unwrap().epoch, 8);
}

#[test]
fn retry_becomes_one_present_request_instead_of_an_immediate_loop() {
    let mut mailbox = WorkerMailbox::new();
    let batch = mailbox.take(true);
    assert!(batch.request_present);
    assert!(!mailbox.has_work());
}

#[test]
fn suspend_batch_retains_pending_scene_updates() {
    let mut mailbox = WorkerMailbox::new();
    let mut scene = RenderScene::default();
    mailbox.resize = Some(resize(
        6,
        ResizeInteractionPhase::Updating,
        SizeI {
            width: 1280,
            height: 720,
        },
        SurfaceResizeAction::KeepCurrent,
    ));
    mailbox.deltas.push(scene.take_delta().unwrap());
    mailbox.request_present = true;
    mailbox.suspend = true;

    let batch = mailbox.take(false);

    assert!(batch.suspend);
    assert_eq!(
        batch.resize,
        Some(resize(
            6,
            ResizeInteractionPhase::Updating,
            SizeI {
                width: 1280,
                height: 720,
            },
            SurfaceResizeAction::KeepCurrent,
        ))
    );
    assert_eq!(batch.deltas.len(), 1);
}

#[test]
fn final_resize_stages_a_preview_before_reconfiguring_the_surface() {
    let final_resize = resize(
        7,
        ResizeInteractionPhase::Ended,
        SizeI {
            width: 1280,
            height: 720,
        },
        SurfaceResizeAction::CommitAfterPreview,
    );
    assert!(should_preview_before_surface_commit(
        final_resize,
        Some(SizeI {
            width: 800,
            height: 600,
        }),
        PresenterState::Ready,
    ));

    let mut pending = PendingSurfaceCommit::new(final_resize);
    assert!(!pending.is_ready());
    pending.mark_preview_presented();
    assert!(pending.is_ready());
}

#[test]
fn responsive_commit_never_stages_a_scaled_preview() {
    let responsive = resize(
        7,
        ResizeInteractionPhase::Ended,
        SizeI {
            width: 1280,
            height: 720,
        },
        SurfaceResizeAction::Commit,
    );
    assert!(!should_preview_before_surface_commit(
        responsive,
        Some(SizeI {
            width: 800,
            height: 600,
        }),
        PresenterState::Ready,
    ));
}

#[test]
fn resize_barrier_completes_only_for_an_exact_current_surface_present() {
    let current = SizeI {
        width: 1001,
        height: 700,
    };
    assert_eq!(
        completed_resize_revision(Some(current), current, false, 23),
        Some(23)
    );
    assert_eq!(
        completed_resize_revision(
            Some(current),
            SizeI {
                width: 1000,
                height: 700,
            },
            false,
            23,
        ),
        None
    );
    assert_eq!(
        completed_resize_revision(Some(current), current, true, 23),
        None
    );
}

#[test]
fn surface_preview_is_skipped_when_it_cannot_be_presented() {
    let stable = resize(
        0,
        ResizeInteractionPhase::Stable,
        SizeI {
            width: 1280,
            height: 720,
        },
        SurfaceResizeAction::CommitAfterPreview,
    );
    let ended = resize(
        8,
        ResizeInteractionPhase::Ended,
        stable.extent,
        stable.surface,
    );
    let old_extent = Some(SizeI {
        width: 800,
        height: 600,
    });

    assert!(!should_preview_before_surface_commit(
        stable,
        old_extent,
        PresenterState::Ready,
    ));
    assert!(!should_preview_before_surface_commit(
        ended,
        Some(ended.extent),
        PresenterState::Ready,
    ));
    assert!(!should_preview_before_surface_commit(
        ended,
        old_extent,
        PresenterState::NeedsReconfigure,
    ));
}

#[test]
fn acquire_stall_suppresses_only_the_active_resize_generation() {
    let mut guard = AcquireStallCircuitBreaker::default();
    let started = resize(
        12,
        ResizeInteractionPhase::Started,
        SizeI {
            width: 900,
            height: 600,
        },
        SurfaceResizeAction::KeepCurrent,
    );
    assert!(!guard.observe_resize(started));
    assert!(guard.observe_acquire(Duration::from_secs(2), Duration::from_millis(17)));
    assert!(guard.suppresses_preview());

    let ended = resize(
        12,
        ResizeInteractionPhase::Ended,
        SizeI {
            width: 1200,
            height: 800,
        },
        SurfaceResizeAction::CommitAfterPreview,
    );
    assert!(guard.observe_resize(ended));
    assert!(!guard.suppresses_preview());

    let next = resize(
        13,
        ResizeInteractionPhase::Started,
        ended.extent,
        SurfaceResizeAction::KeepCurrent,
    );
    assert!(!guard.observe_resize(next));
    assert!(!guard.suppresses_preview());
}

#[test]
fn responsive_resize_commits_are_never_suppressed_by_acquire_latency() {
    let mut guard = AcquireStallCircuitBreaker::default();
    let responsive = resize(
        21,
        ResizeInteractionPhase::Started,
        SizeI {
            width: 900,
            height: 600,
        },
        SurfaceResizeAction::Commit,
    );

    assert!(!guard.observe_resize(responsive));
    assert!(!guard.observe_acquire(Duration::from_secs(2), Duration::from_millis(17)));
    assert!(!guard.suppresses_preview());

    // Switching away from a previously suppressed deferred preview also clears its breaker.
    let deferred = resize(
        22,
        ResizeInteractionPhase::Started,
        responsive.extent,
        SurfaceResizeAction::KeepCurrent,
    );
    assert!(!guard.observe_resize(deferred));
    assert!(guard.observe_acquire(Duration::from_secs(2), Duration::from_millis(17)));
    assert!(guard.suppresses_preview());

    let responsive = resize(
        22,
        ResizeInteractionPhase::Updating,
        SizeI {
            width: 901,
            height: 600,
        },
        SurfaceResizeAction::Commit,
    );
    assert!(!guard.observe_resize(responsive));
    assert!(!guard.suppresses_preview());
}

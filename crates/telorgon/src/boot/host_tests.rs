use super::*;
use core::time::Duration;

fn session() -> BootSession<BootInterface> {
    BootApplication::new()
        .target(
            BootTarget::linux("linux", "Linux").unwrap().source(
                BootSource::efi("\\EFI\\Linux\\loader.efi")
                    .unwrap()
                    .arguments("quiet splash")
                    .unwrap(),
            ),
        )
        .target(
            BootTarget::custom("custom", "My OS")
                .unwrap()
                .source(BootSource::efi("\\EFI\\Custom\\kernel.efi").unwrap()),
        )
        .build()
        .unwrap()
        .into_session()
        .unwrap()
}

fn single_target_application() -> BootApplication {
    BootApplication::new().target(
        BootTarget::linux("linux", "Linux")
            .unwrap()
            .source(BootSource::efi("\\EFI\\Linux\\loader.efi").unwrap()),
    )
}

#[test]
fn one_target_starts_with_one_launch_and_multiple_targets_wait_for_selection() {
    let ready = single_target_application().build().unwrap();
    assert!(ready.auto_boots());
    let session = ready.into_session().unwrap();
    let snapshot = session.controller.snapshot();
    assert_eq!(snapshot.phase, BootPhase::Loading);
    assert_eq!(snapshot.progress_kind, BootProgress::Unknown);
    assert!(!snapshot.simulation);
    session.controller.dispatch(BootCommand::Launch);
    let request = session.controller.take_request().unwrap();
    assert_eq!(request.target.id.as_str(), "linux");
    assert_eq!(session.active_request(), Some(request.id));
    assert!(session.controller.take_request().is_none());

    let multiple = self::session();
    assert_eq!(multiple.controller.snapshot().phase, BootPhase::Selecting);
    assert!(multiple.controller.take_request().is_none());
}

#[test]
fn explicit_selection_and_recovery_sessions_do_not_auto_launch() {
    let ready = single_target_application()
        .selection_mode(BootSelectionMode::Always)
        .build()
        .unwrap();
    assert!(!ready.auto_boots());
    let session = ready.into_session().unwrap();
    assert_eq!(session.controller.snapshot().phase, BootPhase::Selecting);
    assert!(session.controller.take_request().is_none());

    let recovery = single_target_application()
        .build()
        .unwrap()
        .into_selector_session()
        .unwrap();
    assert_eq!(recovery.controller.snapshot().phase, BootPhase::Selecting);
    assert!(recovery.controller.take_request().is_none());
    recovery.controller.dispatch(BootCommand::Launch);
    assert!(recovery.controller.take_request().is_some());
}

#[test]
fn custom_screens_observe_the_initial_loading_splash_and_preserve_selection_policy() {
    let session = single_target_application()
        .screens(|controller: BootController| {
            assert_eq!(controller.snapshot().phase, BootPhase::Loading);
            BootInterface::new(controller)
        })
        .build()
        .unwrap()
        .into_session()
        .unwrap();
    assert!(session.controller.take_request().is_some());
    assert!(session.controller.take_request().is_none());

    single_target_application()
        .selection_mode(BootSelectionMode::Always)
        .screens(|controller: BootController| {
            assert_eq!(controller.snapshot().phase, BootPhase::Selecting);
            BootInterface::new(controller)
        })
        .build()
        .unwrap()
        .into_session()
        .unwrap();
}

#[test]
fn reset_cancels_a_queued_autoboot_and_recovery_waits_for_manual_launch() {
    let session = single_target_application()
        .build()
        .unwrap()
        .into_session()
        .unwrap();
    let cancelled = session.active_request().unwrap();
    session.controller.dispatch(BootCommand::Reset);
    assert_eq!(session.controller.snapshot().phase, BootPhase::Selecting);
    assert_eq!(session.active_request(), None);
    assert!(session.controller.take_request().is_none());
    assert!(
        session
            .controller
            .report(BootHostEvent::Loading {
                request: cancelled,
                status: "Cancelled launch".into(),
                progress: BootProgress::Unknown,
            })
            .is_err()
    );
    session.controller.advance(Duration::from_secs(120));
    assert!(session.controller.take_request().is_none());
    session.controller.dispatch(BootCommand::Launch);
    assert_ne!(session.controller.take_request().unwrap().id, cancelled);
}

#[test]
fn reset_cannot_cancel_execution_and_failed_autoboot_recovers_without_a_loop() {
    let session = single_target_application()
        .build()
        .unwrap()
        .into_session()
        .unwrap();
    let request = session.controller.take_request().unwrap();
    session.controller.dispatch(BootCommand::Reset);
    assert_eq!(session.controller.snapshot().phase, BootPhase::Loading);
    assert_eq!(session.active_request(), Some(request.id));
    session
        .controller
        .report(BootHostEvent::Failed {
            request: request.id,
            message: "Image rejected".into(),
        })
        .unwrap();
    assert_eq!(session.controller.snapshot().phase, BootPhase::Failed);
    assert!(session.controller.take_request().is_none());
    session.controller.dispatch(BootCommand::Reset);
    session.controller.advance(Duration::from_secs(120));
    assert_eq!(session.controller.snapshot().phase, BootPhase::Selecting);
    assert!(session.controller.take_request().is_none());
    session.controller.dispatch(BootCommand::Launch);
    assert_ne!(session.controller.take_request().unwrap().id, request.id);
}

#[test]
fn real_launch_emits_a_source_request_and_time_never_invents_progress() {
    let session = session();
    session
        .controller
        .dispatch(BootCommand::Boot(TargetId::new("custom").unwrap()));
    let request = session.controller.take_request().unwrap();
    assert_eq!(request.target.id.as_str(), "custom");
    assert!(matches!(
        request.source,
        BootSource::EfiImage {
            volume: BootVolume::Current,
            ..
        }
    ));
    assert!(session.controller.take_request().is_none());
    session.controller.advance(Duration::from_secs(120));
    let snapshot = session.controller.snapshot();
    assert_eq!(snapshot.phase, BootPhase::Loading);
    assert_eq!(snapshot.progress_kind, BootProgress::Unknown);
    assert!(!snapshot.simulation);
    assert_eq!(snapshot.elapsed, Duration::from_secs(120));
}

#[test]
fn host_reports_require_active_request_and_reset_progress_at_handoff() {
    let session = session();
    session.controller.dispatch(BootCommand::Launch);
    let request = session.controller.take_request().unwrap();
    assert!(
        session
            .controller
            .report(BootHostEvent::Loading {
                request: BootRequestId(request.id.0 + 1),
                status: "wrong request".into(),
                progress: BootProgress::Measured {
                    completed: 50,
                    total: 100
                },
            })
            .is_err()
    );
    assert!(
        session
            .controller
            .report(BootHostEvent::Loading {
                request: request.id,
                status: "invalid progress".into(),
                progress: BootProgress::Measured {
                    completed: 1,
                    total: 0
                },
            })
            .is_err()
    );
    session
        .controller
        .report(BootHostEvent::Loading {
            request: request.id,
            status: "Reading image".into(),
            progress: BootProgress::Measured {
                completed: 50,
                total: 100,
            },
        })
        .unwrap();
    assert_eq!(session.controller.snapshot().progress, 0.5);
    session
        .controller
        .report(BootHostEvent::Handoff {
            request: request.id,
        })
        .unwrap();
    assert_eq!(
        session.controller.snapshot().progress_kind,
        BootProgress::Unknown
    );
    session.controller.advance(Duration::from_secs(120));
    assert_eq!(session.controller.snapshot().phase, BootPhase::OsStarting);
    session
        .controller
        .report(BootHostEvent::Complete {
            request: request.id,
        })
        .unwrap();
    assert_eq!(session.controller.snapshot().phase, BootPhase::Complete);
    assert!(
        session
            .controller
            .report(BootHostEvent::Failed {
                request: request.id,
                message: "stale".into()
            })
            .is_err()
    );
}

#[test]
fn failed_retry_has_a_new_request_identity() {
    let session = session();
    session.controller.dispatch(BootCommand::Launch);
    let first = session.controller.take_request().unwrap();
    session
        .controller
        .report(BootHostEvent::Failed {
            request: first.id,
            message: "Image rejected".into(),
        })
        .unwrap();
    session.controller.dispatch(BootCommand::Retry);
    let retry = session.controller.take_request().unwrap();
    assert_ne!(retry.id, first.id);
    assert_eq!(retry.target.id, first.target.id);
    assert_eq!(
        session.controller.snapshot().progress_kind,
        BootProgress::Unknown
    );
}

#[test]
fn native_sessions_require_valid_sources_separate_from_display_kind() {
    assert!(BootSource::efi("EFI/Linux/loader.efi").is_err());
    assert!(BootSource::efi("\\EFI\\..\\loader.efi").is_err());
    assert!(
        BootSource::efi("\\EFI\\Linux\\loader.efi")
            .unwrap()
            .load_options(vec![0; 32769])
            .is_err()
    );
    let app = BootApplication::new()
        .target(BootTarget::linux("linux", "Linux").unwrap())
        .build()
        .unwrap();
    assert!(app.into_session().is_err());
    let app = BootApplication::new()
        .target(BootTarget::linux("linux", "Linux").unwrap())
        .build()
        .unwrap();
    assert!(app.into_selector_session().is_err());
}

#[test]
fn os_owned_splash_has_no_execution_request_and_waits_for_real_milestones() {
    let session = BootApplication::new()
        .target(BootTarget::linux("linux", "Linux").unwrap())
        .build()
        .unwrap()
        .into_splash_session("linux")
        .unwrap();
    let request = session.active_request().unwrap();
    assert!(session.controller.take_request().is_none());
    assert_eq!(session.controller.snapshot().phase, BootPhase::OsStarting);
    session.controller.advance(Duration::from_secs(120));
    assert_eq!(
        session.controller.snapshot().progress_kind,
        BootProgress::Unknown
    );
    session
        .controller
        .report(BootHostEvent::OsStarting {
            request,
            status: "Mounting filesystems".into(),
            progress: BootProgress::Measured {
                completed: 2,
                total: 5,
            },
        })
        .unwrap();
    assert_eq!(session.controller.snapshot().progress, 0.4);
    session
        .controller
        .report(BootHostEvent::Complete { request })
        .unwrap();
    assert_eq!(session.controller.snapshot().phase, BootPhase::Complete);
}

#[test]
fn production_views_fit_firmware_extent_without_simulation_controls_or_fake_percentages() {
    use crate::foundation::{MonotonicInstant, SizeI};
    use crate::host::application::ComposedAppRuntime;
    use crate::ui::SemanticRole;
    let session = BootApplication::new()
        .target(
            BootTarget::linux("linux", "Linux")
                .unwrap()
                .source(BootSource::efi("\\EFI\\Linux\\boot.efi").unwrap()),
        )
        .target(
            BootTarget::windows("windows", "Windows")
                .unwrap()
                .source(BootSource::efi("\\EFI\\Microsoft\\boot.efi").unwrap()),
        )
        .target(
            BootTarget::custom("custom", "My OS")
                .unwrap()
                .source(BootSource::efi("\\EFI\\Custom\\boot.efi").unwrap()),
        )
        .build()
        .unwrap()
        .into_session()
        .unwrap();
    let mut runtime = ComposedAppRuntime::from_composed_with_extent(
        session.component,
        SizeI {
            width: 800,
            height: 600,
        },
    )
    .unwrap();
    runtime
        .prepare_frame(MonotonicInstant::from_nanos(0), true)
        .unwrap();
    let buttons: Vec<_> = runtime
        .ui()
        .semantics
        .iter()
        .filter_map(|(node, semantic)| (semantic.role == SemanticRole::Button).then_some(node))
        .collect();
    assert_eq!(buttons.len(), 3);
    for button in buttons {
        let bounds = runtime.layout().computed(button).unwrap().border_rect;
        assert!(bounds.width >= 100.0 && bounds.height >= 44.0);
        assert!(
            bounds.x >= 0.0
                && bounds.y >= 0.0
                && bounds.x + bounds.width <= 800.0
                && bounds.y + bounds.height <= 600.0,
            "{bounds:?}"
        );
    }
    session.controller.dispatch(BootCommand::Launch);
    runtime
        .prepare_frame(MonotonicInstant::from_nanos(1_000_000), true)
        .unwrap();
    assert_eq!(
        runtime
            .ui()
            .semantics
            .iter()
            .filter(|(_, semantic)| semantic.role == SemanticRole::Button)
            .count(),
        0
    );
    for (_, label) in runtime.ui().texts.iter() {
        assert!(
            !runtime
                .ui()
                .string(label.content)
                .unwrap_or_default()
                .contains('%')
        );
    }
}

#[cfg(feature = "boot-preview")]
#[test]
fn simulation_commands_cannot_mutate_a_real_session_or_emit_execution_requests() {
    let session = session();
    session.controller.dispatch(PreviewCommand::Launch);
    session
        .controller
        .dispatch(PreviewCommand::SetScenario(PreviewScenario::OsStarting));
    assert_eq!(session.controller.snapshot().phase, BootPhase::Selecting);
    assert!(session.controller.take_request().is_none());
}

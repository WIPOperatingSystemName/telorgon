use super::preview::model::PreviewModel;
use super::*;
use std::time::Duration;

fn targets() -> Vec<BootTarget> {
    vec![
        BootTarget::linux("linux", "Linux").unwrap(),
        BootTarget::windows("windows", "Windows").unwrap(),
        BootTarget::custom("custom", "My OS").unwrap(),
    ]
}

fn selecting() -> PreviewModel {
    PreviewModel::new(
        targets(),
        0,
        PreviewTheme::Disks,
        PreviewScenario::Selecting,
    )
}

#[test]
fn application_rejects_duplicate_and_missing_default_targets() {
    let duplicate = BootApplication::new()
        .target(BootTarget::linux("same", "Linux").unwrap())
        .target(BootTarget::custom("same", "Other OS").unwrap())
        .build();
    assert!(
        matches!(duplicate, Err(BootError::Configuration(message)) if message.contains("duplicate target"))
    );

    let missing = BootApplication::new()
        .target(BootTarget::linux("linux", "Linux").unwrap())
        .default_target("missing")
        .build();
    assert!(
        matches!(missing, Err(BootError::Configuration(message)) if message.contains("default target missing"))
    );

    let app = BootApplication::new()
        .target(BootTarget::linux("linux", "Linux").unwrap())
        .target(BootTarget::custom("custom", "My OS").unwrap())
        .default_target("custom")
        .build()
        .unwrap();
    assert_eq!(app.selected, 1);
}

#[test]
fn application_requires_a_bounded_nonempty_target_list() {
    assert!(matches!(
        BootApplication::new().build(),
        Err(BootError::Configuration(_))
    ));
    let mut app = BootApplication::new();
    for index in 0..17 {
        app = app.target(BootTarget::custom(&format!("os{index}"), "Custom OS").unwrap());
    }
    assert!(matches!(app.build(), Err(BootError::Configuration(_))));
}

#[test]
fn startup_preview_respects_the_application_selection_policy() {
    let single = BootApplication::new()
        .target(BootTarget::linux("linux", "Linux").unwrap())
        .build()
        .unwrap();
    assert_eq!(
        single.preview_startup_scenario(PreviewScenario::Startup),
        PreviewScenario::Startup
    );
    assert_eq!(
        single.preview_startup_scenario(PreviewScenario::Selecting),
        PreviewScenario::Selecting
    );

    let forced = BootApplication::new()
        .target(BootTarget::linux("linux", "Linux").unwrap())
        .selection_mode(BootSelectionMode::Always)
        .build()
        .unwrap();
    assert_eq!(
        forced.preview_startup_scenario(PreviewScenario::Startup),
        PreviewScenario::Selecting
    );
    assert_eq!(
        forced.preview_startup_scenario(PreviewScenario::Loading),
        PreviewScenario::Loading
    );

    let multiple = BootApplication::new()
        .target(BootTarget::linux("linux", "Linux").unwrap())
        .target(BootTarget::windows("windows", "Windows").unwrap())
        .build()
        .unwrap();
    assert_eq!(
        multiple.preview_startup_scenario(PreviewScenario::Startup),
        PreviewScenario::Selecting
    );
}

#[test]
fn selection_wraps_and_rejects_invalid_or_inactive_selection() {
    let mut model = selecting();
    model.dispatch(PreviewCommand::MoveSelection(-1));
    assert_eq!(model.snapshot.selected, 2);
    model.dispatch(PreviewCommand::MoveSelection(1));
    assert_eq!(model.snapshot.selected, 0);
    model.dispatch(PreviewCommand::MoveSelection(i32::MAX));
    assert_eq!(model.snapshot.selected, 1);
    model.dispatch(PreviewCommand::Select(usize::MAX));
    assert_eq!(model.snapshot.selected, 1);
    model.dispatch(PreviewCommand::Select(2));
    model.dispatch(PreviewCommand::Launch);
    model.dispatch(PreviewCommand::Select(0));
    model.dispatch(PreviewCommand::MoveSelection(-1));
    assert_eq!(model.snapshot.selected, 2);
}

#[test]
fn boot_by_id_selects_and_launches_atomically_without_switching_an_active_target() {
    let mut model = selecting();
    let custom = TargetId::new("custom").unwrap();
    model.dispatch(PreviewCommand::Boot(custom));
    assert_eq!(model.snapshot.selected, 2);
    assert_eq!(model.snapshot.phase, PreviewPhase::Loading);
    assert_eq!(model.snapshot.progress, 0.0);
    assert_eq!(model.snapshot.status, "Reading the boot image");
    model.tick(Duration::from_secs(1));
    model.dispatch(PreviewCommand::Boot(TargetId::new("windows").unwrap()));
    assert_eq!(model.snapshot.selected, 2);
    assert_eq!(model.snapshot.progress, 0.25);
    model.tick(Duration::from_secs(3));
    assert_eq!(model.snapshot.phase, PreviewPhase::OsStarting);
    assert_eq!(model.snapshot.status, "Your kernel is drawing this splash");
}

#[test]
fn unknown_boot_id_leaves_selection_and_startup_state_unchanged() {
    let mut model = selecting();
    model.dispatch(PreviewCommand::Select(1));
    let previous = model.snapshot.clone();
    model.dispatch(PreviewCommand::Boot(TargetId::new("missing").unwrap()));
    assert_eq!(model.snapshot, previous);
}

#[test]
fn retry_restarts_only_a_failed_attempt_at_zero_for_the_same_target() {
    let mut model = selecting();
    model.dispatch(PreviewCommand::Retry);
    assert_eq!(model.snapshot.phase, PreviewPhase::Selecting);
    model.dispatch(PreviewCommand::Boot(TargetId::new("windows").unwrap()));
    model.tick(Duration::from_secs(3));
    model.dispatch(PreviewCommand::Fail);
    model.dispatch(PreviewCommand::TogglePause);
    model.dispatch(PreviewCommand::Retry);
    assert_eq!(model.snapshot.selected, 1);
    assert_eq!(model.snapshot.phase, PreviewPhase::Loading);
    assert_eq!(model.snapshot.progress, 0.0);
    assert!(!model.snapshot.paused);
    model.tick(Duration::from_secs(1));
    model.dispatch(PreviewCommand::Retry);
    assert_eq!(model.snapshot.progress, 0.25);
    model.tick(Duration::from_secs(3));
    assert_eq!(model.snapshot.phase, PreviewPhase::OsStarting);
    assert_eq!(model.snapshot.progress, 0.0);
    assert_eq!(
        model.snapshot.status,
        "Windows is now drawing its own startup screen"
    );
}

#[test]
fn launch_starts_at_zero_and_preserves_each_four_second_stage() {
    let mut model = selecting();
    model.dispatch(PreviewCommand::Launch);
    assert_eq!(model.snapshot.phase, PreviewPhase::Loading);
    assert_eq!(model.snapshot.progress, 0.0);
    assert_eq!(model.snapshot.status, "Reading the boot image");

    model.tick(Duration::from_secs(2));
    assert_eq!(model.snapshot.phase, PreviewPhase::Loading);
    assert_eq!(model.snapshot.progress, 0.5);
    model.tick(Duration::from_secs(2));
    assert_eq!(model.snapshot.phase, PreviewPhase::OsStarting);
    assert_eq!(model.snapshot.progress, 0.0);
    model.tick(Duration::from_secs(3));
    assert_eq!(model.snapshot.phase, PreviewPhase::OsStarting);
    assert_eq!(model.snapshot.progress, 0.75);
    model.tick(Duration::from_secs(1));
    assert_eq!(model.snapshot.phase, PreviewPhase::Complete);
    assert_eq!(model.snapshot.progress, 1.0);
}

#[test]
fn slow_ticks_carry_time_across_handoff_and_clamp_completion() {
    let mut model = selecting();
    model.dispatch(PreviewCommand::Launch);
    model.tick(Duration::from_millis(6500));
    assert_eq!(model.snapshot.phase, PreviewPhase::OsStarting);
    assert_eq!(model.snapshot.progress, 0.625);
    model.tick(Duration::from_secs(30));
    assert_eq!(model.snapshot.phase, PreviewPhase::Complete);
    assert_eq!(model.snapshot.progress, 1.0);

    let mut single_tick = selecting();
    single_tick.dispatch(PreviewCommand::Launch);
    single_tick.tick(Duration::from_secs(9));
    assert_eq!(single_tick.snapshot.phase, PreviewPhase::Complete);
    assert_eq!(single_tick.snapshot.progress, 1.0);
}

#[test]
fn manual_handoff_starts_the_os_stage_at_zero() {
    let mut model = selecting();
    model.dispatch(PreviewCommand::ContinueOs);
    assert_eq!(model.snapshot.phase, PreviewPhase::Selecting);
    model.dispatch(PreviewCommand::Launch);
    model.tick(Duration::from_secs(1));
    model.dispatch(PreviewCommand::ContinueOs);
    assert_eq!(model.snapshot.phase, PreviewPhase::OsStarting);
    assert_eq!(model.snapshot.progress, 0.0);
    model.tick(Duration::from_secs(1));
    model.dispatch(PreviewCommand::ContinueOs);
    assert_eq!(model.snapshot.progress, 0.25);
}

#[test]
fn pause_freezes_the_snapshot_and_resumes_without_advancing_paused_time() {
    let mut model = selecting();
    model.dispatch(PreviewCommand::Launch);
    model.tick(Duration::from_secs(1));
    model.dispatch(PreviewCommand::TogglePause);
    let paused = model.snapshot.clone();
    model.tick(Duration::from_secs(30));
    assert_eq!(model.snapshot, paused);

    model.dispatch(PreviewCommand::TogglePause);
    model.tick(Duration::from_secs(1));
    assert!(!model.snapshot.paused);
    assert_eq!(model.snapshot.phase, PreviewPhase::Loading);
    assert_eq!(model.snapshot.progress, 0.5);
    assert_eq!(model.snapshot.elapsed, Duration::from_secs(2));
}

#[test]
fn failure_and_reset_recover_with_the_selected_target_and_theme() {
    let mut model = selecting();
    model.dispatch(PreviewCommand::Select(2));
    model.dispatch(PreviewCommand::SetTheme(PreviewTheme::Voxel));
    model.dispatch(PreviewCommand::Launch);
    model.tick(Duration::from_secs(2));
    model.dispatch(PreviewCommand::TogglePause);
    model.dispatch(PreviewCommand::Fail);
    assert_eq!(model.snapshot.phase, PreviewPhase::Failed);
    assert_eq!(model.snapshot.progress, 0.0);
    assert!(!model.snapshot.paused);
    model.dispatch(PreviewCommand::Launch);
    assert_eq!(model.snapshot.phase, PreviewPhase::Failed);

    model.dispatch(PreviewCommand::Reset);
    assert_eq!(model.snapshot.phase, PreviewPhase::Selecting);
    assert_eq!(model.snapshot.progress, 0.0);
    assert_eq!(model.snapshot.selected, 2);
    assert_eq!(model.snapshot.theme, PreviewTheme::Voxel);
    model.dispatch(PreviewCommand::Launch);
    model.tick(Duration::from_secs(4));
    assert_eq!(model.snapshot.phase, PreviewPhase::OsStarting);
    assert_eq!(model.snapshot.progress, 0.0);
}

#[test]
fn os_startup_status_distinguishes_shared_splashes_from_windows() {
    let linux = PreviewModel::new(
        targets(),
        0,
        PreviewTheme::Disks,
        PreviewScenario::OsStarting,
    );
    assert_eq!(
        linux.snapshot.status,
        "Linux startup companion is drawing this splash"
    );
    let windows = PreviewModel::new(
        targets(),
        1,
        PreviewTheme::Disks,
        PreviewScenario::OsStarting,
    );
    assert_eq!(
        windows.snapshot.status,
        "Windows is now drawing its own startup screen"
    );
    let custom = PreviewModel::new(
        targets(),
        2,
        PreviewTheme::Disks,
        PreviewScenario::OsStarting,
    );
    assert_eq!(custom.snapshot.status, "Your kernel is drawing this splash");
}

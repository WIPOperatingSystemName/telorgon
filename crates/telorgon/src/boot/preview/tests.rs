use super::*;
use crate::boot::{BootPreview, PreviewCommand, PreviewPhase};
use crate::foundation::{MonotonicInstant, PointF, SizeF, SizeI};
use crate::host::application::{ComposedAppRuntime, PlatformInput};
use crate::input::{
    ButtonState, InputEvent, KeyEvent, LogicalKey, NamedKey, PhysicalKey, PhysicalKeyCode,
    PointerButton,
};
use crate::ui::{MountedUi, SemanticName, SemanticRole, UiNodeId};
use std::time::Duration;

#[test]
fn startup_with_one_target_simulates_boot_from_zero_without_execution_requests() {
    let target = BootTarget::linux("linux", "Linux")
        .unwrap()
        .source(crate::boot::BootSource::efi("\\EFI\\Linux\\linux.efi").unwrap());
    let controller = controller(
        vec![target],
        0,
        PreviewTheme::Disks,
        PreviewScenario::Startup,
    );
    let snapshot = controller.snapshot();
    assert_eq!(snapshot.phase, PreviewPhase::Loading);
    assert_eq!(snapshot.progress, 0.0);
    assert_eq!(snapshot.elapsed, Duration::ZERO);
    assert_eq!(snapshot.status, "Reading the boot image");
    assert!(snapshot.simulation);
    assert!(controller.take_request().is_none());
    assert!(controller.active_request().is_none());

    controller.advance(Duration::from_secs(4));
    assert_eq!(controller.snapshot().phase, PreviewPhase::OsStarting);
    assert_eq!(controller.snapshot().progress, 0.0);
    assert!(controller.take_request().is_none());
    controller.advance(Duration::from_secs(4));
    assert_eq!(controller.snapshot().phase, PreviewPhase::Complete);

    controller.dispatch(PreviewCommand::Reset);
    controller.advance(Duration::from_secs(1));
    assert_eq!(controller.snapshot().phase, PreviewPhase::Selecting);
    assert_eq!(controller.snapshot().progress, 0.0);
}

#[test]
fn startup_with_multiple_targets_and_explicit_selection_show_the_picker() {
    let linux = BootTarget::linux("linux", "Linux").unwrap();
    let windows = BootTarget::windows("windows", "Windows").unwrap();
    for (targets, scenario) in [
        (vec![linux.clone(), windows], PreviewScenario::Startup),
        (vec![linux], PreviewScenario::Selecting),
    ] {
        let controller = controller(targets, 0, PreviewTheme::Voxel, scenario);
        controller.advance(Duration::from_secs(30));
        let snapshot = controller.snapshot();
        assert_eq!(snapshot.phase, PreviewPhase::Selecting);
        assert_eq!(snapshot.status, "Choose where to start");
        assert!(controller.take_request().is_none());
    }
}

fn text_contents(ui: &MountedUi, node: UiNodeId) -> String {
    let mut text = String::new();
    if let Some(value) = ui.texts.get(node) {
        text.push_str(ui.string(value.content).unwrap_or_default());
    }
    for child in ui.nodes.children(node) {
        text.push_str(&text_contents(ui, child));
    }
    text
}

fn button(runtime: &ComposedAppRuntime, label: &str) -> UiNodeId {
    let ui = runtime.ui();
    ui.semantics
        .iter()
        .find_map(|(node, semantics)| {
            if semantics.role != SemanticRole::Button {
                return None;
            }
            let name = match semantics.name {
                SemanticName::Text(value) => ui.string(value).unwrap_or_default().to_owned(),
                SemanticName::Contents => text_contents(ui, node),
                _ => String::new(),
            };
            name.contains(label).then_some(node)
        })
        .unwrap_or_else(|| panic!("missing button {label}"))
}

fn prepare(runtime: &mut ComposedAppRuntime, nanos: u64) {
    let now = MonotonicInstant::from_nanos(nanos);
    runtime.flush_input(now);
    runtime.prepare_frame(now, true).unwrap();
}

fn click(runtime: &mut ComposedAppRuntime, label: &str, nanos: u64) {
    let bounds = runtime
        .layout()
        .computed(button(runtime, label))
        .unwrap()
        .border_rect;
    runtime.queue_input(InputEvent::mouse_moved(PointF {
        x: bounds.x + bounds.width / 2.0,
        y: bounds.y + bounds.height / 2.0,
    }));
    runtime.queue_input(InputEvent::mouse_button(
        PointerButton::PRIMARY,
        ButtonState::Pressed,
    ));
    runtime.queue_input(InputEvent::mouse_button(
        PointerButton::PRIMARY,
        ButtonState::Released,
    ));
    prepare(runtime, nanos);
}

fn press_key(
    runtime: &mut ComposedAppRuntime,
    physical: PhysicalKeyCode,
    logical: LogicalKey,
    nanos: u64,
) {
    for state in [ButtonState::Pressed, ButtonState::Released] {
        runtime.queue_input(InputEvent::Key(
            KeyEvent::new(PhysicalKey::from_code(physical), state)
                .with_logical_key(logical.clone()),
        ));
    }
    prepare(runtime, nanos);
}

fn mounted(theme: PreviewTheme, extent: SizeI) -> (ComposedAppRuntime, PreviewController) {
    let targets = vec![
        BootTarget::linux("linux", "Linux").unwrap(),
        BootTarget::windows("windows", "Windows").unwrap(),
        BootTarget::custom("custom", "My OS").unwrap(),
    ];
    mounted_with_targets(theme, extent, targets)
}

fn mounted_with_targets(
    theme: PreviewTheme,
    extent: SizeI,
    targets: Vec<BootTarget>,
) -> (ComposedAppRuntime, PreviewController) {
    let controller = controller(targets, 0, theme, PreviewScenario::Selecting);
    let mut runtime =
        ComposedAppRuntime::from_composed_with_extent(BootPreview::new(controller.clone()), extent)
            .unwrap();
    runtime.queue_input(PlatformInput::Resize(SizeF {
        width: extent.width as f32,
        height: extent.height as f32,
    }));
    prepare(&mut runtime, 0);
    (runtime, controller)
}

#[test]
fn carousel_choices_are_horizontal_and_fit_default_and_minimum_windows() {
    for extent in [
        SizeI {
            width: 1100,
            height: 760,
        },
        SizeI {
            width: 900,
            height: 680,
        },
    ] {
        let (runtime, _) = mounted(PreviewTheme::Disks, extent);
        let button_count = runtime
            .ui()
            .semantics
            .iter()
            .filter(|(_, semantics)| semantics.role == SemanticRole::Button)
            .count();
        assert_eq!(button_count, 3, "only three OS choices");
        for label in ["Boot Linux", "Boot Windows", "Boot My OS"] {
            let bounds = runtime
                .layout()
                .computed(button(&runtime, label))
                .unwrap()
                .border_rect;
            assert!(
                bounds.width >= 55.0 && bounds.height >= 28.0,
                "{label}: {bounds:?}"
            );
            assert!(
                bounds.x >= 0.0
                    && bounds.y >= 0.0
                    && bounds.x + bounds.width <= extent.width as f32
                    && bounds.y + bounds.height <= extent.height as f32,
                "{label} outside {extent:?}: {bounds:?}"
            );
        }
        let choices = ["Boot Linux", "Boot Windows", "Boot My OS"].map(|label| {
            runtime
                .layout()
                .computed(button(&runtime, label))
                .unwrap()
                .border_rect
        });
        for pair in choices.windows(2) {
            let [left, right] = pair else { unreachable!() };
            assert!(
                left.x + left.width <= right.x,
                "choices must be side by side: {left:?}, {right:?}"
            );
            assert!(
                (left.y - right.y).abs() < 1.0 && (left.height - right.height).abs() < 1.0,
                "choices must share a horizontal row: {left:?}, {right:?}"
            );
        }
    }
}

#[test]
fn carousel_navigation_reveals_targets_beyond_the_visible_choices() {
    let targets = (1..=5)
        .map(|index| BootTarget::custom(&format!("os{index}"), &format!("OS {index}")).unwrap())
        .collect::<Vec<_>>();
    for (extent, visible_choices) in [
        (
            SizeI {
                width: 1100,
                height: 760,
            },
            4,
        ),
        (
            SizeI {
                width: 900,
                height: 680,
            },
            3,
        ),
    ] {
        let (mut runtime, controller) =
            mounted_with_targets(PreviewTheme::Disks, extent, targets.clone());
        let button_count = runtime
            .ui()
            .semantics
            .iter()
            .filter(|(_, semantics)| semantics.role == SemanticRole::Button)
            .count();
        assert_eq!(
            button_count,
            visible_choices + 2,
            "visible choices and two carousel controls"
        );
        for selected in 1..5 {
            press_key(
                &mut runtime,
                PhysicalKeyCode::ArrowRight,
                LogicalKey::Named(NamedKey::ArrowRight),
                selected as u64 * 1_000_000,
            );
            assert_eq!(controller.snapshot().selected, selected);
            let bounds = runtime
                .layout()
                .computed(button(&runtime, &format!("Boot OS {}", selected + 1)))
                .unwrap()
                .border_rect;
            assert!(
                bounds.x >= 0.0 && bounds.x + bounds.width <= extent.width as f32,
                "selected target outside carousel: {bounds:?}"
            );
        }
        click(&mut runtime, "Previous OS", 5_000_000);
        assert_eq!(controller.snapshot().selected, 3);
        button(&runtime, "Boot OS 4");
        click(&mut runtime, "Next OS", 6_000_000);
        assert_eq!(controller.snapshot().selected, 4);
        press_key(
            &mut runtime,
            PhysicalKeyCode::ArrowLeft,
            LogicalKey::Named(NamedKey::ArrowLeft),
            7_000_000,
        );
        assert_eq!(controller.snapshot().selected, 3);
        press_key(
            &mut runtime,
            PhysicalKeyCode::Enter,
            LogicalKey::Named(NamedKey::Enter),
            8_000_000,
        );
        assert_eq!(controller.snapshot().selected, 3);
        assert_eq!(controller.snapshot().phase, PreviewPhase::Loading);
    }
}

#[test]
fn clicking_an_os_option_selects_and_launches_it() {
    let (mut runtime, controller) = mounted(
        PreviewTheme::Disks,
        SizeI {
            width: 900,
            height: 680,
        },
    );
    click(&mut runtime, "Boot Windows", 1_000_000);
    assert_eq!(controller.snapshot().selected, 1);
    assert_eq!(controller.snapshot().phase, PreviewPhase::Loading);
    assert_eq!(controller.snapshot().progress, 0.0);
}

#[test]
fn keyboard_selection_then_enter_launches_the_selected_target() {
    let (mut runtime, controller) = mounted(
        PreviewTheme::Disks,
        SizeI {
            width: 900,
            height: 680,
        },
    );
    let selected = controller.snapshot().selected;
    press_key(
        &mut runtime,
        PhysicalKeyCode::ArrowRight,
        LogicalKey::Named(NamedKey::ArrowRight),
        1_000_000,
    );
    let selected = (selected + 1) % controller.snapshot().targets.len();
    assert_eq!(controller.snapshot().selected, selected);
    assert_eq!(controller.snapshot().phase, PreviewPhase::Selecting);
    assert!(
        runtime
            .ui()
            .interactions
            .get(button(&runtime, "Boot Windows"))
            .unwrap()
            .flags
            .contains(crate::ui::InteractionFlags::FOCUSED)
    );
    press_key(
        &mut runtime,
        PhysicalKeyCode::Enter,
        LogicalKey::Named(NamedKey::Enter),
        2_000_000,
    );
    assert_eq!(controller.snapshot().selected, selected);
    assert_eq!(controller.snapshot().phase, PreviewPhase::Loading);
}

#[test]
fn focused_theme_shortcut_and_direct_boot_buttons_drive_the_shared_controller() {
    let (mut runtime, controller) = mounted(
        PreviewTheme::Disks,
        SizeI {
            width: 1100,
            height: 760,
        },
    );
    press_key(
        &mut runtime,
        PhysicalKeyCode::KeyT,
        LogicalKey::character("t").unwrap(),
        1_000_000,
    );
    assert_eq!(controller.snapshot().theme, PreviewTheme::Voxel);
    click(&mut runtime, "Boot My OS", 2_000_000);
    assert_eq!(controller.snapshot().selected, 2);
    assert_eq!(controller.snapshot().phase, PreviewPhase::Loading);
    assert_eq!(controller.snapshot().progress, 0.0);
    controller.dispatch(PreviewCommand::Fail);
    prepare(&mut runtime, 3_000_000);
    click(&mut runtime, "Retry startup", 4_000_000);
    assert_eq!(controller.snapshot().phase, PreviewPhase::Loading);
    assert_eq!(controller.snapshot().progress, 0.0);
}

use telorgon::{ChangeSource, NodeKind, ViewRuntime, app::*, ui::UiEventKind};

#[component(no_default)]
struct RoutedControl {
    #[input]
    title: String,
    #[state]
    events: u32,
    #[state]
    enabled: bool,
    #[state]
    stage: u8,
}
impl Component for RoutedControl {
    fn view(&self) -> impl View {
        let label = format!("{}: {}", self.title, self.events);
        if self.stage == 2 {
            return text(label).into_element();
        }
        let mut control = button()
            .child(text(label))
            .enabled(self.enabled)
            .on_press(|this: &mut Self| this.stage += 1);
        if self.stage == 0 {
            control = control.on_input(|this: &mut Self, event| {
                let UiEventKind::Focus(focused) = event.kind else {
                    return false;
                };
                this.title = "child-owned".into();
                this.events += 1;
                this.enabled = !focused;
                true
            });
        }
        control.into_element()
    }
}

#[test]
fn targeted_input_restores_inputs_honors_disabled_controls_and_removes_old_routes() {
    let mut runtime = ViewRuntime::from_composed(RoutedControl {
        title: "parent-owned".into(),
        events: 0,
        enabled: true,
        stage: 0,
    })
    .unwrap();
    let button = runtime
        .ui()
        .nodes
        .alive()
        .iter()
        .copied()
        .find(|node| runtime.ui().kinds.get(*node) == Some(&NodeKind::Button))
        .unwrap();
    let contains = |runtime: &ViewRuntime<_>, value| {
        runtime
            .ui()
            .texts
            .values()
            .iter()
            .any(|text| runtime.ui().string(text.content) == Some(value))
    };

    runtime.dispatch_ui(button, UiEventKind::Focus(true), u16::MAX, 0);
    assert!(contains(&runtime, "parent-owned: 1"));
    assert!(!runtime.ui().interactions.get(button).unwrap().enabled);
    runtime.dispatch_ui(button, UiEventKind::Focus(true), u16::MAX, 1);
    assert!(contains(&runtime, "parent-owned: 1"));

    // Disabled controls still receive focus loss so their local focus state can clear.
    runtime.dispatch_ui(button, UiEventKind::Focus(false), u16::MAX, 2);
    assert!(contains(&runtime, "parent-owned: 2"));
    assert!(runtime.ui().interactions.get(button).unwrap().enabled);
    assert_eq!(
        runtime.composition_diagnostics().input_mutations_restored,
        2
    );

    assert!(runtime.dispatch_activation(button, ChangeSource::Programmatic));
    assert!(runtime.ui().nodes.contains(button));
    runtime.dispatch_ui(button, UiEventKind::Focus(true), u16::MAX, 3);
    assert!(contains(&runtime, "parent-owned: 2"));
    assert!(runtime.dispatch_activation(button, ChangeSource::Programmatic));
    assert!(!runtime.ui().nodes.contains(button));
    runtime.dispatch_ui(button, UiEventKind::Focus(true), u16::MAX, 4);
    assert!(contains(&runtime, "parent-owned: 2"));
}

#[component]
struct Foreign {}
impl Component for Foreign {
    fn view(&self) -> impl View {
        text("Foreign")
    }
}
#[component]
struct Invalid {}
impl Component for Invalid {
    fn view(&self) -> impl View {
        button()
            .child(text("Invalid"))
            .on_input(|_: &mut Foreign, _| false)
    }
}
#[test]
fn input_callback_owner_is_checked_before_mount() {
    let error = match ViewRuntime::from_composed(Invalid::default()) {
        Ok(_) => panic!("a foreign input owner should be rejected"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("callback component type mismatch")
    );
}

#[component]
struct ResponsivePanel {}
impl Component for ResponsivePanel {
    fn view(&self) -> impl View {
        let width = self.viewport_size().width;
        let content = column()
            .key("responsive-content")
            .child(text(format!("Width: {width}")));
        if width < 600.0 {
            content.scrollable()
        } else {
            content
        }
    }
}
#[component]
struct ResponsiveHost {}
impl Component for ResponsiveHost {
    fn view(&self) -> impl View {
        column().padding(20.0).child(ResponsivePanel::default())
    }
}
#[test]
fn composed_viewport_changes_update_nested_components_and_remount_scroll_containers() {
    let mut runtime = telorgon::application_host::AppRuntimeCore::from_composed_with_extent(
        ResponsiveHost::default(),
        telorgon::SizeI {
            width: 900,
            height: 700,
        },
    )
    .unwrap();
    for (width, scrolls) in [(900, 0), (500, 1), (850, 0)] {
        runtime
            .resize(telorgon::SizeI { width, height: 700 })
            .unwrap();
        runtime
            .prepare_frame(telorgon::MonotonicInstant::ZERO, true)
            .unwrap();
        assert!(
            runtime
                .ui()
                .texts
                .iter()
                .any(|(_, text)| runtime.ui().string(text.content)
                    == Some(format!("Width: {width}").as_str()))
        );
        assert_eq!(
            runtime
                .ui()
                .nodes
                .alive()
                .iter()
                .filter(|node| runtime.ui().kinds.get(**node) == Some(&NodeKind::Scroll))
                .count(),
            scrolls
        );
    }
}

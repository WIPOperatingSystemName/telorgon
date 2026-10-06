use telorgon::app::*;
use telorgon::{ChangeSource, NodeKind, SemanticCheckState, ValueChangePhase, ViewRuntime};

#[component]
struct Counter {
    #[input]
    title: String,
    #[state]
    count: u32,
}

impl Counter {
    fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            count: 0,
        }
    }
}

impl Component for Counter {
    fn view(&self) -> impl View {
        column()
            .gap(16.0)
            .padding(24.0)
            .child(text(&self.title).size(20.0))
            .child(
                text(format!("Count: {}", self.count))
                    .style(TextStyle::new().size(32.0).weight(600)),
            )
            .child(
                button()
                    .child(text("Increment").color(telorgon::ColorRgba8::rgba(248, 249, 252, 255)))
                    .primary()
                    .on_press(|this: &mut Self| this.count += 1),
            )
    }
}

#[component]
struct DefaultCounter {
    #[state]
    count: u32,
}

impl Component for DefaultCounter {
    fn view(&self) -> impl View {
        text(format!("Count: {}", self.count))
    }
}

#[test]
fn component_derives_default_automatically() {
    let runtime = ViewRuntime::from_composed(DefaultCounter::default()).unwrap();
    assert!(
        runtime
            .ui()
            .texts
            .values()
            .iter()
            .any(|value| runtime.ui().string(value.content) == Some("Count: 0"))
    );
}

#[component]
struct ForeignCallbackOwner {}

impl Component for ForeignCallbackOwner {
    fn view(&self) -> impl View {
        text("foreign")
    }
}

#[component]
struct InvalidCallbackOwner {}

impl Component for InvalidCallbackOwner {
    fn view(&self) -> impl View {
        button()
            .child(text("Invalid").color(telorgon::ColorRgba8::rgba(248, 249, 252, 255)))
            .on_press(|_: &mut ForeignCallbackOwner| {})
    }
}

#[test]
fn callback_type_mismatches_are_rejected_before_mount() {
    let error = match ViewRuntime::from_composed(InvalidCallbackOwner::default()) {
        Ok(_) => panic!("mismatched callback owner should fail"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("callback component type mismatch")
    );
}

#[test]
fn counter_rerenders_dynamic_text_without_remounting_the_button() {
    let mut runtime = ViewRuntime::from_composed(Counter::new("Counter")).unwrap();
    let button = runtime
        .ui()
        .nodes
        .alive()
        .iter()
        .copied()
        .find(|node| runtime.ui().kinds.get(*node) == Some(&NodeKind::Button))
        .unwrap();
    let count_text = runtime
        .ui()
        .texts
        .values()
        .iter()
        .find(|text| runtime.ui().string(text.content) == Some("Count: 0"))
        .copied()
        .unwrap();
    let count_node = runtime
        .ui()
        .nodes
        .alive()
        .iter()
        .copied()
        .find(|node| runtime.ui().texts.get(*node) == Some(&count_text))
        .unwrap();

    assert_eq!(count_text.style.size, 32.0);
    assert_eq!(count_text.style.weight, 600);

    assert!(runtime.dispatch_activation(button, ChangeSource::Programmatic));

    assert!(runtime.ui().nodes.contains(button));
    assert_eq!(
        runtime
            .ui()
            .texts
            .get(count_node)
            .and_then(|text| runtime.ui().string(text.content)),
        Some("Count: 1")
    );
    let diagnostics = runtime.composition_diagnostics();
    assert_eq!(diagnostics.events_delivered, 1);
    assert!(diagnostics.elements_reused >= 3);
}

#[test]
fn sealed_application_declaration_owns_initial_content() {
    let application = Application::gui("org.telorgon.examples.counter", "Counter")
        .renderer(Renderer::Auto)
        .window(
            Window::new("Counter")
                .size(480, 320)
                .content(Counter::new("Example")),
        );
    let debug = format!("{application:?}");
    assert!(debug.contains("has_content: true"));
    assert!(debug.contains("renderer: Auto"));
}

#[component]
struct FrameFixture {}

impl Component for FrameFixture {
    fn view(&self) -> impl View {
        window_frame()
            .decoration(
                BoxDecoration::new()
                    .background(Background::Color(ColorRgba8::rgba(24, 27, 35, 255)))
                    .corner_radius(12.0)
                    .shadow(Shadow {
                        offset: PointF { x: 0.0, y: 8.0 },
                        blur: 24.0,
                        spread: 0.0,
                        color: ColorRgba8::rgba(0, 0, 0, 96),
                    }),
            )
            .child(
                row()
                    .height(36.0)
                    .child(text("Example").window_title())
                    .window_drag_region(),
            )
            .child(
                button()
                    .child(text("Close").color(telorgon::ColorRgba8::rgba(248, 249, 252, 255)))
                    .width(32.0)
                    .height(32.0)
                    .corner_radius(8.0)
                    .window_action(WindowAction::Close),
            )
            .content_slot(window_content_slot().margin((40.0, 6.0, 6.0, 6.0)))
    }
}

#[test]
fn composed_window_frame_exposes_layout_derived_shell_regions() {
    let mut runtime = telorgon::application_host::AppRuntimeCore::from_composed_with_extent(
        FrameFixture::default(),
        telorgon::SizeI {
            width: 332,
            height: 252,
        },
    )
    .unwrap();
    runtime
        .prepare_frame(telorgon::MonotonicInstant::ZERO, true)
        .unwrap();

    let snapshot = telorgon::WindowChromeSnapshot::derive(runtime.ui(), runtime.layout()).unwrap();
    assert_eq!(snapshot.frame.bounds.width, 332.0);
    assert_eq!(snapshot.content.bounds.x, 6.0);
    assert_eq!(snapshot.content.bounds.y, 40.0);
    assert_eq!(snapshot.content.bounds.width, 320.0);
    assert!(
        snapshot.regions.iter().any(|region| {
            region.role == telorgon::WindowChromeRole::Action(WindowAction::Close)
        })
    );
    assert!(
        snapshot
            .regions
            .iter()
            .any(|region| region.role == telorgon::WindowChromeRole::DragRegion)
    );
    assert_eq!(
        snapshot.hit_test(4.0, 4.0),
        Some(telorgon::WindowChromeRole::Action(WindowAction::Close))
    );
}

#[component]
struct PointerFixture {}

impl Component for PointerFixture {
    fn view(&self) -> impl View {
        row()
            .child(text("Editable").cursor(CursorIcon::Text))
            .child(
                button()
                    .cursor(CursorIcon::None)
                    .cursor(CursorIcon::Pointer)
                    .height(32.0)
                    .hover_effect(InteractionEffect::Lift(2.0))
                    .child(text("Open").color(telorgon::ColorRgba8::rgba(248, 249, 252, 255))),
            )
            .child(spacer().cursor(CursorIcon::None))
            .child(
                button()
                    .cursor(CursorIcon::None)
                    .height(32.0)
                    .child(text("Hidden")),
            )
            .child(button().cursor(CursorIcon::Default).child(text("Default")))
            .child(button().child(text("Default pointer cursor")))
    }
}

#[test]
fn composed_views_retain_semantic_pointer_requests() {
    let runtime = ViewRuntime::from_composed(PointerFixture::default()).unwrap();
    let requests = runtime
        .ui()
        .pointer_requests
        .values()
        .iter()
        .copied()
        .collect::<Vec<_>>();
    assert!(requests.contains(&telorgon::PointerRequest::Semantic(
        telorgon::PointerIcon::Text
    )));
    assert!(requests.contains(&telorgon::PointerRequest::Semantic(
        telorgon::PointerIcon::Pointer
    )));
    assert!(requests.contains(&telorgon::PointerRequest::Hidden));
    let button_requests: Vec<_> = runtime
        .ui()
        .kinds
        .iter()
        .filter(|(_, kind)| **kind == telorgon::NodeKind::Button)
        .map(|(node, _)| runtime.ui().pointer_requests.get(node).copied())
        .collect();
    assert!(button_requests.contains(&Some(telorgon::PointerRequest::Hidden)));
    assert!(
        button_requests.contains(&Some(telorgon::PointerRequest::Semantic(
            telorgon::PointerIcon::Default
        )))
    );
    assert_eq!(
        button_requests
            .iter()
            .filter(|request| **request == Some(telorgon::PointerRequest::Hidden))
            .count(),
        1
    );
    assert_eq!(
        button_requests
            .iter()
            .filter(|request| **request
                == Some(telorgon::PointerRequest::Semantic(
                    telorgon::PointerIcon::Pointer
                )))
            .count(),
        2
    );
}

#[component]
struct IconControlFixture {}

impl Component for IconControlFixture {
    fn view(&self) -> impl View {
        button().accessible_label("Close").child(
            image(telorgon::IconAsset::new(telorgon::AssetKey::new(
                "icons/close.svg",
            )))
            .width(18.0)
            .height(18.0)
            .tint(ColorRgba8::rgba(255, 255, 255, 255)),
        )
    }
}

#[test]
fn composed_icon_buttons_keep_accessible_text_and_typed_artwork() {
    let runtime = ViewRuntime::from_composed(IconControlFixture::default()).unwrap();
    let expected = telorgon::asset_image_id(telorgon::AssetKey::new("icons/close.svg"));
    assert!(runtime.ui().images.values().iter().any(|image| {
        image.image == expected && image.tint == Some(ColorRgba8::rgba(255, 255, 255, 255))
    }));
    assert!(runtime.ui().semantics.values().iter().any(|semantic| {
        let telorgon::SemanticName::Text(name) = semantic.name else {
            return false;
        };
        runtime.ui().string(name) == Some("Close")
    }));
}

#[test]
fn app_facade_includes_common_composition_styling() {
    let style = BoxStyle {
        width: SizeRule::Fill(1.0),
        decoration: BoxDecoration {
            background: Background::Color(ColorRgba8::rgba(20, 22, 28, 255)),
            ..BoxDecoration::default()
        },
        ..BoxStyle::default()
    };

    assert!(matches!(style.width, SizeRule::Fill(1.0)));
}

#[component]
struct Controls {
    #[state]
    first: bool,
    #[state]
    second: bool,
    #[state]
    level: f32,
}

impl Component for Controls {
    fn view(&self) -> impl View {
        column()
            .child(
                checkbox("First", self.first).on_change(|this: &mut Self, checked| {
                    this.first = checked;
                }),
            )
            .child(
                switch("Second", self.second).on_change(|this: &mut Self, checked| {
                    this.second = checked;
                }),
            )
            .child(
                slider("Level", self.level).on_change(|this: &mut Self, value| {
                    this.level = value;
                }),
            )
    }
}

#[test]
fn controls_reconcile_independently_and_slider_thumb_tracks_the_value() {
    let mut runtime = ViewRuntime::from_composed(Controls {
        first: false,
        second: true,
        level: 0.25,
    })
    .unwrap();
    let toggles: Vec<_> = runtime
        .ui()
        .nodes
        .alive()
        .iter()
        .copied()
        .filter(|node| runtime.ui().kinds.get(*node) == Some(&NodeKind::Toggle))
        .collect();
    assert_eq!(toggles.len(), 2);
    let slider = runtime
        .ui()
        .nodes
        .alive()
        .iter()
        .copied()
        .find(|node| runtime.ui().kinds.get(*node) == Some(&NodeKind::Slider))
        .unwrap();

    let checkbox_style = telorgon::ComponentStyleId::named(
        telorgon::ThemeDomainId::APPLICATION,
        "checkbox",
        "default",
    );
    let switch_style = telorgon::ComponentStyleId::named(
        telorgon::ThemeDomainId::APPLICATION,
        "switch",
        "default",
    );
    let slider_style = telorgon::ComponentStyleId::named(
        telorgon::ThemeDomainId::APPLICATION,
        "slider",
        "default",
    );
    assert!(runtime.ui().style_bindings().iter().any(|binding| {
        binding.state_root == toggles[0]
            && binding.component_style == checkbox_style
            && binding.slots.len() == 6
    }));
    assert!(runtime.ui().style_bindings().iter().any(|binding| {
        binding.state_root == toggles[1]
            && binding.component_style == switch_style
            && binding.slots.len() == 4
    }));
    assert!(runtime.ui().style_bindings().iter().any(|binding| {
        binding.state_root == slider
            && binding.component_style == slider_style
            && binding.slots.len() == 5
    }));

    assert!(runtime.dispatch_activation(toggles[0], ChangeSource::Pointer));
    assert_eq!(
        runtime
            .ui()
            .semantics
            .get(toggles[0])
            .unwrap()
            .state
            .checked,
        Some(SemanticCheckState::Checked)
    );
    assert_eq!(
        runtime
            .ui()
            .semantics
            .get(toggles[1])
            .unwrap()
            .state
            .checked,
        Some(SemanticCheckState::Checked)
    );

    assert!(runtime.dispatch_value(
        slider,
        0.75,
        ValueChangePhase::Commit,
        ChangeSource::Pointer,
    ));
    assert_eq!(runtime.ui().interactions.get(slider).unwrap().value, 0.75);
}

#[component(no_default)]
struct SignalLabel {
    #[input]
    value: Signal<u32>,
}

impl Component for SignalLabel {
    fn view(&self) -> impl View {
        let value = self.watch(&self.value);
        text(format!("External: {}", *value))
    }
}

#[test]
fn watched_signal_invalidates_only_after_a_changed_publication() {
    let (value, writer) = Signal::new(4_u32);
    let mut runtime = ViewRuntime::from_composed(SignalLabel { value }).unwrap();
    let label = runtime
        .ui()
        .nodes
        .alive()
        .iter()
        .copied()
        .find(|node| {
            runtime
                .ui()
                .texts
                .get(*node)
                .and_then(|text| runtime.ui().string(text.content))
                == Some("External: 4")
        })
        .unwrap();

    writer.publish_if_changed(4);
    assert!(!runtime.external_updates_ready());

    writer.publish(9);
    assert!(runtime.external_updates_ready());
    assert_eq!(runtime.process_external_updates(), 1);
    assert!(!runtime.external_updates_ready());
    assert_eq!(
        runtime
            .ui()
            .texts
            .get(label)
            .and_then(|text| runtime.ui().string(text.content)),
        Some("External: 9")
    );
    let diagnostics = runtime.composition_diagnostics();
    assert_eq!(diagnostics.externally_invalidated_components, 1);
    assert_eq!(diagnostics.externally_reconciled_components, 1);
}

#[component]
struct KeyedItem {
    #[input]
    name: String,
    #[state]
    count: u32,
}

impl KeyedItem {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            count: 0,
        }
    }
}

impl Component for KeyedItem {
    fn view(&self) -> impl View {
        button()
            .child(
                text(format!("{}: {}", self.name, self.count))
                    .color(telorgon::ColorRgba8::rgba(248, 249, 252, 255)),
            )
            .on_press(|this: &mut Self| this.count += 1)
    }
}

#[component]
struct KeyedList {
    #[state]
    reversed: bool,
}

impl Component for KeyedList {
    fn view(&self) -> impl View {
        let first = if self.reversed { "B" } else { "A" };
        let second = if self.reversed { "A" } else { "B" };
        column()
            .child(
                button()
                    .child(text("Reverse").color(telorgon::ColorRgba8::rgba(248, 249, 252, 255)))
                    .on_press(|this: &mut Self| this.reversed = !this.reversed),
            )
            .child(KeyedItem::new(first).keyed(first))
            .child(KeyedItem::new(second).keyed(second))
    }
}

#[test]
fn keyed_component_reorder_preserves_local_state_and_control_identity() {
    let mut runtime = ViewRuntime::from_composed(KeyedList { reversed: false }).unwrap();
    let named_button = |runtime: &ViewRuntime<telorgon::CompositionDriver>, name: &str| {
        runtime
            .ui()
            .semantics
            .iter()
            .find_map(|(node, semantic)| {
                let child_matches = runtime.ui().nodes.children(node).any(|child| {
                    runtime
                        .ui()
                        .texts
                        .get(child)
                        .is_some_and(|text| runtime.ui().string(text.content) == Some(name))
                });
                (semantic.role == telorgon::SemanticRole::Button && child_matches).then_some(node)
            })
            .unwrap()
    };

    let item_a = named_button(&runtime, "A: 0");
    assert!(runtime.dispatch_activation(item_a, ChangeSource::Programmatic));
    let item_a = named_button(&runtime, "A: 1");
    let reverse = named_button(&runtime, "Reverse");
    assert!(runtime.dispatch_activation(reverse, ChangeSource::Programmatic));

    assert!(runtime.ui().nodes.contains(item_a));
    assert_eq!(named_button(&runtime, "A: 1"), item_a);
    assert_eq!(runtime.composition_diagnostics().live_components, 3);
}

#[component]
struct DuplicateLabelButtons {}

impl Component for DuplicateLabelButtons {
    fn view(&self) -> impl View {
        row()
            .child(KeyedItem::new("Same").keyed("first"))
            .child(KeyedItem::new("Same").keyed("second"))
    }
}

#[test]
fn buttons_with_equal_labels_do_not_share_mutable_accessible_names() {
    let mut runtime = ViewRuntime::from_composed(DuplicateLabelButtons {}).unwrap();
    let buttons = runtime
        .ui()
        .semantics
        .iter()
        .filter_map(|(node, semantic)| {
            (semantic.role == telorgon::SemanticRole::Button).then_some(node)
        })
        .collect::<Vec<_>>();
    assert_eq!(buttons.len(), 2);
    let name = |runtime: &ViewRuntime<telorgon::CompositionDriver>, node| {
        let semantic = runtime.ui().semantics.get(node).unwrap();
        assert_eq!(semantic.name, telorgon::SemanticName::Contents);
        let child = runtime.ui().nodes.children(node).next().unwrap();
        let text = runtime.ui().texts.get(child).unwrap();
        runtime.ui().string(text.content).unwrap().to_owned()
    };
    assert_eq!(name(&runtime, buttons[0]), "Same: 0");
    assert_eq!(name(&runtime, buttons[1]), "Same: 0");
    assert!(runtime.dispatch_action(buttons[0]));
    assert_eq!(name(&runtime, buttons[0]), "Same: 1");
    assert_eq!(name(&runtime, buttons[1]), "Same: 0");
}

#[component]
struct MountedUpdate {
    #[state]
    value: u32,
}

impl Component for MountedUpdate {
    fn mounted(&mut self, cx: &mut telorgon::MountContext<Self>) {
        self.value = 7;
        cx.request_update();
    }

    fn view(&self) -> impl View {
        text(format!("Mounted: {}", self.value))
    }
}

#[test]
fn mounted_lifecycle_can_request_one_coalesced_followup_view() {
    let runtime = ViewRuntime::from_composed(MountedUpdate { value: 0 }).unwrap();
    assert!(
        runtime
            .ui()
            .texts
            .values()
            .iter()
            .any(|visual| { runtime.ui().string(visual.content) == Some("Mounted: 7") })
    );
    assert_eq!(runtime.composition_diagnostics().view_evaluations, 2);
}

#[component]
struct InvalidInitialView {
    #[state]
    unused: bool,
}

impl Component for InvalidInitialView {
    fn view(&self) -> impl View {
        button().accessible_label("")
    }
}

#[test]
fn invalid_initial_view_returns_an_error_instead_of_panicking() {
    let result = ViewRuntime::from_composed(InvalidInitialView { unused: false });
    assert!(result.is_err());
}

#[component]
struct InputAuthority {
    #[input]
    title: String,
    #[state]
    attempts: u32,
}

impl Component for InputAuthority {
    fn view(&self) -> impl View {
        column()
            .child(text(&self.title))
            .child(
                button()
                    .child(
                        text("Attempt input write")
                            .color(telorgon::ColorRgba8::rgba(248, 249, 252, 255)),
                    )
                    .on_press(|this: &mut Self| {
                        this.title = "child-owned".to_owned();
                        this.attempts += 1;
                    }),
            )
            .child(text(format!("Attempts: {}", self.attempts)))
    }
}

#[test]
fn event_callbacks_cannot_take_authority_over_input_fields() {
    let mut runtime = ViewRuntime::from_composed(InputAuthority {
        title: "parent-owned".to_owned(),
        attempts: 0,
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

    assert!(runtime.dispatch_activation(button, ChangeSource::Programmatic));
    let copy = runtime
        .ui()
        .texts
        .values()
        .iter()
        .filter_map(|visual| runtime.ui().string(visual.content))
        .collect::<Vec<_>>();
    assert!(copy.contains(&"parent-owned"));
    assert!(copy.contains(&"Attempts: 1"));
    assert!(!copy.contains(&"child-owned"));
    assert_eq!(
        runtime.composition_diagnostics().input_mutations_restored,
        1
    );
}

#[component]
struct ContainerButton {
    #[state]
    expanded: bool,
}

impl Component for ContainerButton {
    fn view(&self) -> impl View {
        button()
            .child(
                row()
                    .height(40.0)
                    .child(
                        text(if self.expanded {
                            "Expanded"
                        } else {
                            "Collapsed"
                        })
                        .size(23.0)
                        .key("title"),
                    )
                    .maybe(self.expanded, DefaultCounter::default().keyed("nested")),
            )
            .on_press(|this: &mut Self| this.expanded = !this.expanded)
    }
}

#[test]
fn button_children_reconcile_and_unmount_nested_components() {
    let mut runtime = ViewRuntime::from_composed(ContainerButton::default()).unwrap();
    let root = runtime
        .ui()
        .kinds
        .iter()
        .find_map(|(node, kind)| (*kind == NodeKind::Button).then_some(node))
        .unwrap();
    let row = runtime.ui().nodes.children(root).next().unwrap();
    let title = runtime.ui().nodes.children(row).next().unwrap();
    assert_eq!(runtime.ui().texts.get(title).unwrap().style.size, 23.0);
    assert_eq!(
        runtime.ui().nodes.children(root).count(),
        1,
        "no hidden label or icon nodes"
    );
    assert!(runtime.dispatch_activation(root, ChangeSource::Programmatic));
    assert_eq!(runtime.ui().nodes.children(row).next(), Some(title));
    assert_eq!(
        runtime
            .ui()
            .string(runtime.ui().texts.get(title).unwrap().content),
        Some("Expanded")
    );
    let mounted = runtime.composition_diagnostics().components_mounted;
    assert!(runtime.dispatch_activation(root, ChangeSource::Programmatic));
    assert_eq!(runtime.ui().nodes.children(row).count(), 1);
    assert!(runtime.composition_diagnostics().components_unmounted > 0);
    assert_eq!(
        runtime.composition_diagnostics().components_mounted,
        mounted
    );
}

#[component]
struct InvalidButtonChildren {}
impl Component for InvalidButtonChildren {
    fn view(&self) -> impl View {
        button().children([text("A").key("duplicate"), text("B").key("duplicate")])
    }
}

#[test]
fn button_rejects_duplicate_child_keys() {
    assert!(ViewRuntime::from_composed(InvalidButtonChildren::default()).is_err());
}

#[component]
struct ReplacingButton {
    #[state]
    replaced: bool,
}
impl Component for ReplacingButton {
    fn view(&self) -> impl View {
        if self.replaced {
            text("Done").into_element()
        } else {
            button()
                .child(DefaultCounter::default())
                .on_press(|this: &mut Self| this.replaced = true)
                .into_element()
        }
    }
}

#[test]
fn replacing_a_button_tears_down_its_child_components() {
    let mut runtime = ViewRuntime::from_composed(ReplacingButton::default()).unwrap();
    let node = runtime
        .ui()
        .kinds
        .iter()
        .find_map(|(node, kind)| (*kind == NodeKind::Button).then_some(node))
        .unwrap();
    assert!(runtime.dispatch_action(node));
    assert!(!runtime.ui().nodes.contains(node));
    assert_eq!(runtime.composition_diagnostics().components_unmounted, 1);
    assert!(!runtime.dispatch_action(node));
}

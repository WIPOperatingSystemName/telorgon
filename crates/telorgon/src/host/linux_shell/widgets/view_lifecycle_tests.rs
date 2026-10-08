use super::*;
use crate::authoring::compose::*;
use std::{cell::Cell, rc::Rc};

struct ButtonWidget {
    spec: Signal<ShellSurfaceSpec>,
    presses: Rc<Cell<u32>>,
}

struct ParentWidget {
    child_spec: Signal<ShellSurfaceSpec>,
    presses: Rc<Cell<u32>>,
}

macro_rules! fields {
    ($component:ty) => {
        impl ComponentFields for $component {
            type InputSnapshot = ();
            fn capture_inputs(&self) {}
            fn restore_inputs(&mut self, _: ()) -> bool {
                false
            }
            fn update_inputs(&mut self, _: Self) -> bool {
                false
            }
        }
    };
}
fields!(ButtonWidget);
fields!(ParentWidget);

impl Component for ButtonWidget {
    fn view(&self) -> impl View {
        button()
            .accessible_label("Activate")
            .width(100.0)
            .height(40.0)
            .on_press(|this: &mut Self| this.presses.set(this.presses.get() + 1))
    }
}

impl ShellWidget for ButtonWidget {
    fn surface(&self) -> ShellSurfaceSpec {
        *self.watch(&self.spec)
    }
}

impl Component for ParentWidget {
    fn view(&self) -> impl View {
        column()
    }
}

impl ShellWidget for ParentWidget {
    fn surface(&self) -> ShellSurfaceSpec {
        positioned_spec(100.0, ShellFocus::None).placement(
            WidgetPlacement::positioned(PointF { x: 100.0, y: 100.0 })
                .width(200.0)
                .height(80.0),
        )
    }

    fn children(&self) -> Vec<ShellChild> {
        vec![ShellChild::new(
            "button-child",
            ButtonWidget {
                spec: self.child_spec.clone(),
                presses: self.presses.clone(),
            },
        )]
    }
}

fn output() -> SizeI {
    SizeI {
        width: 800,
        height: 600,
    }
}

fn positioned_spec(x: f32, focus: ShellFocus) -> ShellSurfaceSpec {
    ShellSurfaceSpec::new()
        .placement(
            WidgetPlacement::positioned(PointF { x, y: 0.0 })
                .width(100.0)
                .height(40.0),
        )
        .layer(ShellSurfaceLayer::Overlay)
        .pointer(ShellPointer::Surface)
        .focus(focus)
}

fn layer<W: ShellWidget>(id: u32, widget: W) -> WidgetLayer {
    let (root, binding) = crate::authoring::compose::shell_widget::erase(widget);
    let host = crate::authoring::compose::shell_services::ShellServiceHost::new();
    WidgetLayer::new(
        id,
        crate::host::application::declaration::RegisteredShellWidget {
            content: CompositionDriver::from_erased_for_target(root, RuntimeTarget::ShellWidget),
            surface: binding,
        },
        output(),
        &LayerAssets::new(AssetBundle::default()).unwrap(),
        crate::platform::contracts::ScaleFactor::new(1.0).unwrap(),
        &EventNotifier::new("widget lifecycle test").unwrap(),
        host.services.clone(),
    )
    .unwrap()
}

fn prepare(widgets: &mut [WidgetLayer], now: u64) {
    prepare_widget_surfaces(
        widgets,
        output(),
        shell_work_area_for_spec(output()),
        now,
        crate::theme::MotionPreference::Full,
    )
    .unwrap();
}

fn click(widgets: &mut [WidgetLayer], position: PointF, now: u64) {
    for pressed in [true, false] {
        assert!(
            widget_pointer_button(
                widgets,
                position,
                0x110,
                pressed,
                MonotonicInstant::from_nanos(now),
                false,
            )
            .unwrap()
        );
    }
    prepare(widgets, now);
}

#[test]
fn attached_child_button_activates_on_first_open_with_or_without_keyboard_focus() {
    for focus in [ShellFocus::None, ShellFocus::OnOpen] {
        let spec = ShellSurfaceSpec::new()
            .placement(
                WidgetPlacement::attached(ShellEdge::Bottom)
                    .width(100.0)
                    .height(40.0),
            )
            .layer(ShellSurfaceLayer::Overlay)
            .pointer(ShellPointer::Surface)
            .focus(focus);
        let (child_spec, _) = Signal::new(spec);
        let presses = Rc::new(Cell::new(0));
        let mut widgets = vec![layer(
            0,
            ParentWidget {
                child_spec,
                presses: presses.clone(),
            },
        )];
        let host = crate::authoring::compose::shell_services::ShellServiceHost::new();
        let mut next_id = 1;
        sync_widget_children(
            &mut widgets,
            &mut next_id,
            output(),
            &LayerAssets::new(AssetBundle::default()).unwrap(),
            crate::platform::contracts::ScaleFactor::new(1.0).unwrap(),
            &EventNotifier::new("child lifecycle test").unwrap(),
            &host.services,
        )
        .unwrap();
        prepare(&mut widgets, 1);
        let child = widgets
            .iter()
            .find(|widget| widget.parent == Some(0))
            .unwrap();
        assert!(child.spec.visible);
        assert_eq!(child.focused, focus == ShellFocus::OnOpen);
        let position = PointF {
            x: child.sampled.x as f32 + 20.0,
            y: child.sampled.y as f32 + 20.0,
        };
        if focus == ShellFocus::OnOpen {
            for key in [crate::input::NamedKey::Tab, crate::input::NamedKey::Enter] {
                assert!(
                    widget_key(
                        &mut widgets,
                        crate::input::KeyEvent::new(
                            crate::input::PhysicalKey::UNIDENTIFIED,
                            crate::input::ButtonState::Pressed,
                        )
                        .with_logical_key(crate::input::LogicalKey::Named(key)),
                        MonotonicInstant::from_nanos(2),
                        false,
                    )
                    .unwrap()
                );
            }
            assert_eq!(
                presses.get(),
                1,
                "opening a keyboard-focused child must activate its runtime before pointer input"
            );
        }
        click(&mut widgets, position, 3);
        assert_eq!(
            presses.get(),
            1 + u32::from(focus == ShellFocus::OnOpen),
            "fresh attached child must dispatch its composed button callback"
        );
    }
}

#[test]
fn retained_button_activates_after_hide_and_show_without_keyboard_focus() {
    let spec = positioned_spec(0.0, ShellFocus::None);
    let (signal, writer) = Signal::new(spec);
    let presses = Rc::new(Cell::new(0));
    let mut widgets = vec![layer(
        0,
        ButtonWidget {
            spec: signal,
            presses: presses.clone(),
        },
    )];
    let position = PointF { x: 20.0, y: 20.0 };
    click(&mut widgets, position, 1);
    assert_eq!(presses.get(), 1);
    writer.publish(spec.visible(false));
    prepare(&mut widgets, 2);
    assert!(!widgets[0].spec.visible);
    assert!(
        !widget_pointer_button(
            &mut widgets,
            position,
            0x110,
            true,
            MonotonicInstant::from_nanos(3),
            false
        )
        .unwrap()
    );
    writer.publish(spec);
    prepare(&mut widgets, 4);
    assert!(
        !widgets[0].focused,
        "pointer availability does not grant keyboard focus"
    );
    click(&mut widgets, position, 5);
    assert_eq!(
        presses.get(),
        2,
        "showing a retained widget must restore composed input"
    );
    assert!(!widgets[0].focused);
}

#[test]
fn visible_button_activates_after_focus_moves_to_another_widget_and_returns() {
    let (first_spec, _) = Signal::new(positioned_spec(0.0, ShellFocus::OnClick));
    let (second_spec, _) = Signal::new(positioned_spec(200.0, ShellFocus::OnClick));
    let first = Rc::new(Cell::new(0));
    let second = Rc::new(Cell::new(0));
    let mut widgets = vec![
        layer(
            0,
            ButtonWidget {
                spec: first_spec,
                presses: first.clone(),
            },
        ),
        layer(
            1,
            ButtonWidget {
                spec: second_spec,
                presses: second.clone(),
            },
        ),
    ];
    click(&mut widgets, PointF { x: 20.0, y: 20.0 }, 1);
    assert!(widgets[0].focused);
    click(&mut widgets, PointF { x: 220.0, y: 20.0 }, 2);
    assert!(!widgets[0].focused);
    assert!(widgets[0].spec.visible);
    assert!(widgets[1].focused);
    click(&mut widgets, PointF { x: 20.0, y: 20.0 }, 3);
    assert!(widgets[0].focused);
    assert!(!widgets[1].focused);
    assert_eq!(
        first.get(),
        2,
        "returning pointer input must reactivate the visible button"
    );
    assert_eq!(second.get(), 1);
}

#[test]
fn cancelled_press_and_ineligible_widgets_never_dispatch_button_callbacks() {
    let spec = positioned_spec(0.0, ShellFocus::None);
    let (signal, writer) = Signal::new(spec);
    let presses = Rc::new(Cell::new(0));
    let mut widgets = vec![layer(
        0,
        ButtonWidget {
            spec: signal,
            presses: presses.clone(),
        },
    )];
    let position = PointF { x: 20.0, y: 20.0 };
    assert!(
        widget_pointer_button(
            &mut widgets,
            position,
            0x110,
            true,
            MonotonicInstant::from_nanos(1),
            false,
        )
        .unwrap()
    );
    assert_eq!(presses.get(), 0);
    writer.publish(spec.visible(false));
    prepare(&mut widgets, 2);
    assert!(widgets[0].captured.is_empty());
    writer.publish(spec);
    prepare(&mut widgets, 3);
    assert!(
        !widget_pointer_button(
            &mut widgets,
            position,
            0x110,
            false,
            MonotonicInstant::from_nanos(4),
            false,
        )
        .unwrap()
    );
    assert_eq!(
        presses.get(),
        0,
        "showing a widget must not revive its cancelled press"
    );
    click(&mut widgets, position, 5);
    assert_eq!(presses.get(), 1);

    for pressed in [true, false] {
        assert!(
            !widget_pointer_button(
                &mut widgets,
                position,
                0x110,
                pressed,
                MonotonicInstant::from_nanos(6),
                true,
            )
            .unwrap()
        );
    }
    assert_eq!(
        presses.get(),
        1,
        "locked input must not activate a visible widget"
    );

    widgets[0].retiring = true;
    assert!(
        widgets[0].spec.visible,
        "retirement precedes the next surface preparation"
    );
    for pressed in [true, false] {
        assert!(
            !widget_pointer_button(
                &mut widgets,
                position,
                0x110,
                pressed,
                MonotonicInstant::from_nanos(7),
                false,
            )
            .unwrap()
        );
    }
    assert_eq!(
        presses.get(),
        1,
        "retiring widgets must immediately reject input"
    );
}

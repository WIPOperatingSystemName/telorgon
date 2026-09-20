use super::*;
use crate::authoring::compose::{
    Component, ComponentFields, ShellSurfaceLayer, ShellWidget, Signal, SignalWriter, View,
    WidgetPlacement, text,
};
struct Fixture {
    placement: Signal<ShellSurfaceSpec>,
    dismissals: std::rc::Rc<std::cell::Cell<u32>>,
}
impl ComponentFields for Fixture {
    type InputSnapshot = ();
    fn capture_inputs(&self) {}
    fn restore_inputs(&mut self, _: ()) -> bool {
        false
    }
    fn update_inputs(&mut self, _: Self) -> bool {
        false
    }
}
impl Component for Fixture {
    fn view(&self) -> impl View {
        text("shell")
    }
}
impl ShellWidget for Fixture {
    fn surface(&self) -> ShellSurfaceSpec {
        *self.watch(&self.placement)
    }
    fn dismissed(&mut self, _: ShellDismissReason) {
        self.dismissals.set(self.dismissals.get() + 1);
    }
    fn input(&mut self, event: crate::input::InputEvent) -> bool {
        if matches!(
            event,
            crate::input::InputEvent::PointerMoved { .. }
                | crate::input::InputEvent::Scroll { .. }
        ) {
            self.dismissals.set(self.dismissals.get() + 1);
        }
        false
    }
}
fn output() -> SizeI {
    SizeI {
        width: 800,
        height: 600,
    }
}
fn fixture(
    spec: ShellSurfaceSpec,
) -> (
    WidgetLayer,
    SignalWriter<ShellSurfaceSpec>,
    std::rc::Rc<std::cell::Cell<u32>>,
) {
    let (signal, writer) = Signal::new(spec);
    let dismissals = std::rc::Rc::new(std::cell::Cell::new(0));
    let (root, binding) = crate::authoring::compose::shell_widget::erase(Fixture {
        placement: signal,
        dismissals: dismissals.clone(),
    });
    let registered = crate::host::application::declaration::RegisteredShellWidget {
        content: CompositionDriver::from_erased_for_target(
            root,
            crate::authoring::compose::RuntimeTarget::ShellWidget,
        ),
        surface: binding,
    };
    let services = crate::authoring::compose::shell_services::ShellServiceHost::new();
    let layer = WidgetLayer::new(
        7,
        registered,
        output(),
        AssetBundle::default(),
        crate::platform::contracts::ScaleFactor::new(1.0).unwrap(),
        &EventNotifier::new("widget test").unwrap(),
        services.services.clone(),
    )
    .unwrap();
    (layer, writer, dismissals)
}
#[test]
fn preview_slots_use_live_generations_include_subsurfaces_and_obey_lock() {
    use super::super::client::maximize_preview_tests::test_window;
    use crate::authoring::compose::ShellWindowPreview;
    use std::num::NonZeroU32;
    let id =
        crate::shell::WindowId::new(NonZeroU32::new(1).unwrap(), NonZeroU32::new(1).unwrap());
    let root_id = WaylandSurfaceId::from_raw(10).unwrap();
    let child_id = WaylandSurfaceId::from_raw(11).unwrap();
    let root_rect = RectI {
        x: 100,
        y: 100,
        width: 400,
        height: 200,
    };
    let child_rect = RectI {
        x: 200,
        y: 150,
        width: 100,
        height: 50,
    };
    let mut root = test_window(
        SizeI {
            width: 400,
            height: 200,
        },
        PointI { x: 100, y: 100 },
    );
    root.desktop_id = Some(id);
    root.minimized = true;
    let mut child = test_window(
        SizeI {
            width: 100,
            height: 50,
        },
        PointI { x: 200, y: 150 },
    );
    child.backend = None;
    child.role = SurfaceRole::Subsurface;
    child.parent = Some(root_id);
    let mut windows = BTreeMap::from([(root_id, root), (child_id, child)]);
    let image = |raw, rect: RectI| {
        ShellLayer::image(
            ShellLayerKey::Surface(raw),
            ShellSceneKey::Surface(raw),
            1,
            ShellImageUpdate::Unchanged,
            SizeI {
                width: rect.width,
                height: rect.height,
            },
            rect,
            None,
            ImageAlphaMode::Opaque,
            ImagePixelFormat::Rgba8,
            false,
        )
    };
    let sources = vec![image(10, root_rect), image(11, child_rect)];
    let (widget, _, _) = fixture(
        ShellSurfaceSpec::new().placement(
            WidgetPlacement::positioned(PointF { x: 20.0, y: 20.0 })
                .width(300.0)
                .height(200.0),
        ),
    );
    let slot = crate::foundation::RectF {
        x: 10.0,
        y: 10.0,
        width: 200.0,
        height: 100.0,
    };
    *widget.binding.2.borrow_mut() = vec![ShellWindowPreview::new(id, slot)];
    let preview = widget.preview_layers(&windows, &sources, false);
    assert_eq!(preview.len(), 2);
    assert_eq!(
        preview[0].target,
        RectI {
            x: 30,
            y: 30,
            width: 200,
            height: 100
        }
    );
    assert_eq!(
        preview[1].target,
        RectI {
            x: 80,
            y: 55,
            width: 50,
            height: 25
        }
    );
    assert!(preview.iter().all(|p| p.visible
        && matches!(
            p.content,
            ShellLayerContent::Image {
                update: ShellImageUpdate::Unchanged,
                ..
            }
        )));
    assert!(widget.preview_layers(&windows, &sources, true).is_empty());
    windows.get_mut(&child_id).unwrap().role = SurfaceRole::XdgPopup;
    assert_eq!(widget.preview_layers(&windows, &sources, false).len(), 1);
    windows.get_mut(&root_id).unwrap().desktop_id = Some(crate::shell::WindowId::new(
        NonZeroU32::new(1).unwrap(),
        NonZeroU32::new(2).unwrap(),
    ));
    assert!(widget.preview_layers(&windows, &sources, false).is_empty());
    windows.get_mut(&root_id).unwrap().desktop_id = Some(id);
    widget.binding.2.borrow_mut()[0].rect.x = f32::NAN;
    assert!(widget.preview_layers(&windows, &sources, false).is_empty());
    widget.binding.2.borrow_mut()[0].rect = crate::foundation::RectF {
        width: 1000.0,
        ..slot
    };
    assert!(widget.preview_layers(&windows, &sources, false).is_empty());
}
#[test]
fn changing_only_preview_slots_requests_a_new_frame() {
    use std::num::NonZeroU32;
    let (mut widget, _, _) = fixture(
        ShellSurfaceSpec::new().placement(WidgetPlacement::center().width(300.0).height(200.0)),
    );
    widget.scene(false);
    *widget.binding.2.borrow_mut() = vec![crate::authoring::compose::ShellWindowPreview::new(
        crate::shell::WindowId::new(NonZeroU32::new(1).unwrap(), NonZeroU32::new(1).unwrap()),
        crate::foundation::RectF {
            x: 10.0,
            y: 10.0,
            width: 100.0,
            height: 80.0,
        },
    )];
    widget
        .prepare(output(), shell_work_area_for_spec(output()), 1)
        .unwrap();
    assert!(widget.dirty());
    widget.scene(false);
    widget
        .prepare(output(), shell_work_area_for_spec(output()), 2)
        .unwrap();
    assert!(!widget.dirty());
    widget.binding.2.borrow_mut()[0].rect.x = 20.0;
    widget
        .prepare(output(), shell_work_area_for_spec(output()), 3)
        .unwrap();
    assert!(widget.dirty());
}
#[test]
fn hover_popup_keeps_anchor_and_gap_then_dismisses_outside() {
    let spec = ShellSurfaceSpec::new()
        .placement(
            WidgetPlacement::attached_to(
                crate::foundation::RectF {
                    x: 30.0,
                    y: 0.0,
                    width: 40.0,
                    height: 48.0,
                },
                ShellEdge::Top,
            )
            .width(224.0)
            .height(160.0)
            .offset(0.0, -8.0),
        )
        .pointer(ShellPointer::Surface)
        .dismiss_on_pointer_leave(true);
    let (mut widget, _, dismissals) = fixture(spec);
    widget.parent = Some(1);
    widget.parent_bounds = Some(RectI {
        x: 0,
        y: 552,
        width: 800,
        height: 48,
    });
    widget
        .prepare(output(), shell_work_area_for_spec(output()), 1)
        .unwrap();
    assert!(widget.hover_contains(PointF { x: 45.0, y: 565.0 }));
    assert!(widget.hover_contains(PointF { x: 45.0, y: 548.0 }));
    assert!(widget.hover_contains(PointF { x: 45.0, y: 400.0 }));
    assert!(!widget.hover_contains(PointF { x: 700.0, y: 300.0 }));
    let before = dismissals.get();
    widget_pointer_motion(
        std::slice::from_mut(&mut widget),
        PointF { x: 700.0, y: 300.0 },
        MonotonicInstant::from_nanos(2),
        false,
    )
    .unwrap();
    assert_eq!(dismissals.get(), before + 1);
}
#[test]
fn shell_hook_receives_pointer_motion_when_pointer_leaves_surface() {
    let (widget, _, count) = fixture(
        ShellSurfaceSpec::new()
            .placement(
                WidgetPlacement::positioned(PointF { x: 20.0, y: 20.0 })
                    .width(200.0)
                    .height(150.0),
            )
            .pointer(ShellPointer::Surface),
    );
    let mut widgets = vec![widget];
    let now = MonotonicInstant::from_nanos(1);
    assert!(
        widget_pointer_motion(&mut widgets, PointF { x: 50.0, y: 50.0 }, now, false).unwrap()
    );
    assert_eq!(count.get(), 1);
    assert!(
        !widget_pointer_motion(&mut widgets, PointF { x: 700.0, y: 500.0 }, now, false)
            .unwrap()
    );
    assert_eq!(count.get(), 2);
}
#[test]
fn scroll_routes_only_to_hit_shell_and_never_while_locked() {
    let (widget, _, count) = fixture(
        ShellSurfaceSpec::new()
            .placement(
                WidgetPlacement::positioned(PointF { x: 20.0, y: 20.0 })
                    .width(200.0)
                    .height(150.0),
            )
            .pointer(ShellPointer::Surface),
    );
    let mut widgets = vec![widget];
    let now = MonotonicInstant::from_nanos(1);
    let p = PointF { x: 50.0, y: 50.0 };
    assert!(
        widget_pointer_scroll(&mut widgets, p, PointF { x: 0.0, y: 15.0 }, now, false).unwrap()
    );
    assert_eq!(count.get(), 1);
    assert!(
        !widget_pointer_scroll(&mut widgets, p, PointF { x: 0.0, y: 15.0 }, now, true).unwrap()
    );
    assert!(
        !widget_pointer_scroll(
            &mut widgets,
            PointF { x: 500.0, y: 500.0 },
            PointF::default(),
            now,
            false
        )
        .unwrap()
    );
    assert_eq!(count.get(), 1);
}
#[test]
fn preview_fits_portrait_and_landscape_without_stretching() {
    let slot = RectI {
        x: 10,
        y: 20,
        width: 200,
        height: 100,
    };
    assert_eq!(
        fit_preview(
            SizeI {
                width: 100,
                height: 200
            },
            slot
        ),
        Some(RectI {
            x: 85,
            y: 20,
            width: 50,
            height: 100
        })
    );
    assert_eq!(
        fit_preview(
            SizeI {
                width: 400,
                height: 100
            },
            slot
        ),
        Some(RectI {
            x: 10,
            y: 45,
            width: 200,
            height: 50
        })
    );
    assert_eq!(fit_preview(SizeI::default(), slot), None);
}
#[test]
fn reactive_geometry_keeps_component_identity_and_retargets_motion() {
    let initial = ShellSurfaceSpec::new()
        .placement(WidgetPlacement::edge(ShellEdge::Bottom).height(48.0))
        .movement(crate::GeometryMotion::Spring(crate::Spring::new()));
    let (mut widget, writer, _) = fixture(initial);
    let mounted = widget
        .layer
        .runtime
        .composition_diagnostics()
        .components_mounted;
    writer.publish_if_changed(
        initial.placement(WidgetPlacement::edge(ShellEdge::Top).height(48.0)),
    );
    widget
        .prepare(output(), shell_work_area_for_spec(output()), 10_000_000)
        .unwrap();
    widget
        .prepare(output(), shell_work_area_for_spec(output()), 100_000_000)
        .unwrap();
    let before = widget.sampled;
    writer.publish_if_changed(initial);
    widget
        .prepare(output(), shell_work_area_for_spec(output()), 100_000_000)
        .unwrap();
    assert_eq!(before, widget.sampled);
    assert_eq!(
        widget
            .layer
            .runtime
            .composition_diagnostics()
            .components_mounted,
        mounted
    );
    widget
        .prepare(output(), shell_work_area_for_spec(output()), 900_000_000)
        .unwrap();
    assert_eq!(widget.sampled.y, 552);
    assert!(!widget.animating());
}
#[test]
fn tile_widget_scene_survives_owner_close_and_fast_reentry() {
    use super::super::scene::{ShellComposition, ShellSceneKey};
    use crate::graphics::renderers::vulkan::VulkanScene;
    let (root, surface) = crate::authoring::compose::shell_widget::erase(crate::WindowTiling::snap());
    let registered = crate::host::application::declaration::RegisteredShellWidget {
        content: CompositionDriver::from_erased_for_target(
            root,
            crate::authoring::compose::RuntimeTarget::ShellWidget,
        ),
        surface,
    };
    let widget = WidgetLayer::new(
        7,
        registered,
        output(),
        AssetBundle::default(),
        crate::platform::contracts::ScaleFactor::new(1.0).unwrap(),
        &EventNotifier::new("tile retention test").unwrap(),
        crate::authoring::compose::shell_services::ShellServiceHost::new()
            .services
            .clone(),
    )
    .unwrap();
    let mut widgets = vec![widget];
    let mut composition = ShellComposition::new(output());
    let mut retained = BTreeMap::<ShellSceneKey, VulkanScene>::new();
    let key = ShellSceneKey::Widget(widgets[0].id);
    for (step, raw) in [None, Some(91), None, Some(92), None, Some(93)]
        .into_iter()
        .enumerate()
    {
        let mut windows = BTreeMap::new();
        if let Some(raw) = raw {
            let owner = WaylandSurfaceId::from_raw(raw).unwrap();
            widgets[0].tile_preview_owner = Some(owner);
            widgets[0].tile_preview = Some((
                owner,
                RectI {
                    x: 400,
                    y: 0,
                    width: 400 - step as i32 * 10,
                    height: 600,
                },
            ));
            windows.insert(
                owner,
                super::super::client::maximize_preview_tests::test_window(
                    SizeI {
                        width: 300,
                        height: 200,
                    },
                    PointI { x: 400, y: 10 },
                ),
            );
        } else {
            widgets[0].tile_preview = None;
        }
        let order: Vec<_> = windows.keys().copied().collect();
        let layers = super::super::layers::prepare_desktop_layers(
            false,
            output(),
            step as u64 * 200_000_000,
            false,
            &mut BTreeMap::new(),
            &mut windows,
            &order,
            &mut widgets,
            &mut [],
            &mut None,
            None,
            PointF::default(),
            None,
            PointF::default(),
            &LinuxShellConfig::default(),
        )
        .unwrap();
        let frame = composition
            .synchronize_with_force(output(), layers, true)
            .unwrap();
        for update in frame.updates.iter().filter(|u| u.key == key) {
            let scene = retained.entry(key).or_default();
            for delta in &update.deltas {
                scene.apply_delta_checked(delta).unwrap();
            }
        }
        retained.retain(|key, _| frame.live_scenes.contains(key));
        assert!(
            retained.contains_key(&key),
            "mounted widget scene retired at step {step}"
        );
        if raw.is_none() {
            assert!(!frame.placements.iter().any(|p| p.scene == key));
        }
    }
    let unmounted = composition
        .synchronize_with_force(output(), Vec::new(), true)
        .unwrap();
    retained.retain(|key, _| unmounted.live_scenes.contains(key));
    assert!(
        !retained.contains_key(&key),
        "unmount still releases the scene"
    );
}

#[test]
fn tile_preview_retargets_fades_and_never_takes_input() {
    use crate::authoring::compose::ShellWidget;
    let spec = crate::WindowTiling::snap().surface();
    let (mut widget, _, _) = fixture(spec);
    let owner = WaylandSurfaceId::from_raw(91).unwrap();
    let first = RectI {
        x: 0,
        y: 40,
        width: 400,
        height: 560,
    };
    widget.tile_preview = Some((owner, first));
    widget
        .prepare(output(), shell_work_area_for_spec(output()), 10_000_000)
        .unwrap();
    assert_eq!(widget.sampled, first);
    assert_eq!(widget.opacity, 0.0);
    assert_eq!(widget.spec.pointer, crate::ShellPointer::PassThrough);
    widget
        .prepare(output(), shell_work_area_for_spec(output()), 70_000_000)
        .unwrap();
    assert!(widget.opacity > 0.0 && widget.opacity < 1.0);
    let second = RectI {
        x: 400,
        y: 40,
        width: 400,
        height: 280,
    };
    widget.tile_preview = Some((owner, second));
    widget
        .prepare(output(), shell_work_area_for_spec(output()), 70_000_000)
        .unwrap();
    assert_eq!(widget.sampled, first);
    widget
        .prepare(output(), shell_work_area_for_spec(output()), 300_000_000)
        .unwrap();
    assert_eq!(widget.sampled, second);
    assert_eq!(widget.opacity, 1.0);
    widget.tile_preview = None;
    widget
        .prepare(output(), shell_work_area_for_spec(output()), 300_000_000)
        .unwrap();
    assert!(!widget.input_visible());
    assert!(widget.scene(false).visible);
    assert!(widget.tile_material(true).is_none());
    widget
        .prepare(output(), shell_work_area_for_spec(output()), 400_000_000)
        .unwrap();
    assert!(!widget.scene(false).visible);
    assert!(!widget.animating());
}

#[test]
fn visibility_fades_shrinks_reverses_and_releases_input() {
    let spec = ShellSurfaceSpec::new()
        .placement(WidgetPlacement::center().width(200.0).height(100.0))
        .pointer(ShellPointer::Surface)
        .visibility_motion(crate::Minimize::shrink_and_fade(130));
    let (mut widget, writer, _) = fixture(spec);
    assert_eq!(widget.opacity, 0.0);
    assert_eq!(widget.sampled.width, 184);
    widget
        .prepare(output(), shell_work_area_for_spec(output()), 65_000_000)
        .unwrap();
    assert!(widget.opacity > 0.0 && widget.opacity < 1.0);
    let halfway = widget.opacity;
    writer.publish_if_changed(spec.visible(false));
    widget
        .prepare(output(), shell_work_area_for_spec(output()), 65_000_000)
        .unwrap();
    assert_eq!(widget.opacity, halfway);
    assert!(!widget.input_visible());
    assert!(widget.scene(false).visible);
    widget
        .prepare(output(), shell_work_area_for_spec(output()), 100_000_000)
        .unwrap();
    let exiting = widget.opacity;
    assert!(exiting < halfway);
    writer.publish_if_changed(spec);
    widget
        .prepare(output(), shell_work_area_for_spec(output()), 100_000_000)
        .unwrap();
    assert_eq!(widget.opacity, exiting);
    widget
        .prepare(output(), shell_work_area_for_spec(output()), 230_000_000)
        .unwrap();
    assert_eq!(widget.opacity, 1.0);
    assert_eq!(widget.sampled.width, 200);
    assert!(!widget.animating());
    writer.publish_if_changed(spec.visible(false));
    widget
        .prepare(output(), shell_work_area_for_spec(output()), 231_000_000)
        .unwrap();
    widget
        .prepare(output(), shell_work_area_for_spec(output()), 361_000_000)
        .unwrap();
    assert!(!widget.scene(false).visible);
    assert_eq!(widget.opacity, 0.0);
}

#[test]
fn visibility_motion_snaps_with_reduced_motion() {
    let spec = ShellSurfaceSpec::new().visibility_motion(crate::Minimize::shrink_and_fade(130));
    let (mut widget, writer, _) = fixture(spec);
    widget
        .layer
        .runtime
        .set_motion_preference(crate::theme::MotionPreference::Reduced);
    widget
        .prepare(output(), shell_work_area_for_spec(output()), 1)
        .unwrap();
    assert_eq!(widget.opacity, 1.0);
    assert!(!widget.animating());
    writer.publish_if_changed(spec.visible(false));
    widget
        .prepare(output(), shell_work_area_for_spec(output()), 2)
        .unwrap();
    assert_eq!(widget.opacity, 0.0);
    assert!(!widget.scene(false).visible);
}

#[test]
fn exiting_retains_pixels_but_releases_reservation_and_input() {
    let spec = ShellSurfaceSpec::new()
        .placement(WidgetPlacement::edge(ShellEdge::Bottom).height(48.0))
        .pointer(ShellPointer::Surface)
        .reserve_space(ShellReservation::WhenVisible)
        .movement(crate::GeometryMotion::Tween(crate::tween_ms(
            100,
            crate::Easing::Linear,
        )))
        .exit_to(ShellEdge::Bottom);
    let (mut widget, writer, _) = fixture(spec);
    assert_eq!(widget.reservation(), Some((ShellEdge::Bottom, 48)));
    writer.publish_if_changed(spec.visible(false));
    widget
        .prepare(output(), shell_work_area_for_spec(output()), 1_000_000)
        .unwrap();
    assert!(widget.scene(false).visible);
    assert_eq!(widget.reservation(), None);
    assert!(!widget.contains(PointF { x: 20.0, y: 580.0 }));
    widget
        .prepare(output(), shell_work_area_for_spec(output()), 101_000_000)
        .unwrap();
    assert!(!widget.scene(false).visible);
}
#[test]
fn dismissal_is_a_request_and_lock_hides_surfaces() {
    let spec = ShellSurfaceSpec::new()
        .placement(WidgetPlacement::center().width(300.0).height(100.0))
        .layer(ShellSurfaceLayer::Overlay)
        .focus(ShellFocus::OnOpen)
        .dismiss_on_escape(true);
    let (widget, _, count) = fixture(spec);
    let mut widgets = vec![widget];
    let event = crate::input::KeyEvent::new(
        crate::input::PhysicalKey::UNIDENTIFIED,
        crate::input::ButtonState::Pressed,
    )
    .with_logical_key(crate::input::LogicalKey::Named(
        crate::input::NamedKey::Escape,
    ));
    assert!(widget_key(&mut widgets, event, MonotonicInstant::from_nanos(0), false).unwrap());
    assert_eq!(count.get(), 1);
    assert!(widgets[0].spec.visible);
    assert!(!widgets[0].scene(true).visible);
}
#[test]
fn captured_pointer_stays_with_owner_above_another_surface() {
    let base = ShellSurfaceSpec::new()
        .placement(
            WidgetPlacement::positioned(PointF { x: 0.0, y: 0.0 })
                .width(100.0)
                .height(100.0),
        )
        .pointer(ShellPointer::Surface);
    let (left, _, left_events) = fixture(base);
    let (mut right, _, right_events) = fixture(
        base.placement(
            WidgetPlacement::positioned(PointF { x: 200.0, y: 0.0 })
                .width(100.0)
                .height(100.0),
        )
        .layer(ShellSurfaceLayer::Overlay),
    );
    right.id = 8;
    let mut widgets = vec![left, right];
    let now = MonotonicInstant::from_nanos(0);
    assert!(
        widget_pointer_button(&mut widgets, PointF { x: 20.0, y: 20.0 }, true, now, false)
            .unwrap()
    );
    assert!(
        widget_pointer_motion(&mut widgets, PointF { x: 220.0, y: 20.0 }, now, false).unwrap()
    );
    assert_eq!(left_events.get(), 1);
    assert_eq!(right_events.get(), 0);
    assert!(widgets[0].captured);
    assert!(!widgets[1].captured);
}
#[test]
fn content_sizing_and_overlapping_reservations_do_not_feed_back() {
    let (mut content, _, _) = fixture(ShellSurfaceSpec::new());
    let first = content.sampled;
    assert!(first.width > 1 && first.width < 200);
    assert!(first.height > 1 && first.height < 100);
    content
        .prepare(output(), shell_work_area_for_spec(output()), 1)
        .unwrap();
    assert_eq!(content.sampled, first);
    let panel = ShellSurfaceSpec::new()
        .placement(
            WidgetPlacement::edge(ShellEdge::Bottom)
                .height(48.0)
                .margin(10.0),
        )
        .reserve_space(ShellReservation::WhenVisible);
    let (a, _, _) = fixture(panel);
    let (b, _, _) = fixture(panel);
    assert_eq!(shell_work_area(output(), &[a, b]).height, 542);
}
#[test]
fn reduced_motion_reaches_target_without_animation() {
    let spec = ShellSurfaceSpec::new()
        .placement(WidgetPlacement::edge(ShellEdge::Bottom).height(48.0))
        .movement(crate::GeometryMotion::Spring(crate::Spring::new()));
    let (mut widget, writer, _) = fixture(spec);
    widget
        .layer
        .runtime
        .set_motion_preference(crate::theme::MotionPreference::Reduced);
    writer
        .publish_if_changed(spec.placement(WidgetPlacement::edge(ShellEdge::Top).height(48.0)));
    widget
        .prepare(output(), shell_work_area_for_spec(output()), 1)
        .unwrap();
    assert_eq!(widget.sampled.y, 0);
    assert!(!widget.animating());
}

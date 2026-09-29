use super::*;
use crate::authoring::compose::{Component, Dimension, View, column};
use crate::host::application::runtime::ComposedAppRuntime;
use crate::{SizeI, foundation::MonotonicInstant, input::InputEvent};

#[crate::component]
struct ScrollFixture {}

impl Component for ScrollFixture {
    fn view(&self) -> impl View {
        column()
            .width(Dimension::FILL)
            .height(Dimension::FILL)
            .scrollable()
            .children((0..6).map(|_| column().height(100.0)))
    }
}

#[test]
fn wheel_requests_a_frame_without_component_updates() {
    let mut runtime = ComposedAppRuntime::from_composed_with_extent(
        ScrollFixture {},
        SizeI {
            width: 200,
            height: 150,
        },
    )
    .unwrap();
    runtime.prepare_frame(MonotonicInstant::ZERO, true).unwrap();
    runtime.queue_input(InputEvent::mouse_moved(PointF { x: 20.0, y: 20.0 }));
    runtime.flush_input(MonotonicInstant::from_nanos(1));
    runtime
        .prepare_frame(MonotonicInstant::from_nanos(2), true)
        .unwrap();

    for (time, delta) in [(3, -48.0), (5, 48.0)] {
        runtime.queue_input(InputEvent::mouse_scroll(PointF { x: 0.0, y: delta }));
        let outcome = runtime.flush_input(MonotonicInstant::from_nanos(time));
        assert!(!outcome.frame_needed_before);
        assert!(
            outcome.frame_needed_after,
            "wheel movement must schedule its own redraw"
        );
        let frame = runtime
            .prepare_frame(MonotonicInstant::from_nanos(time + 1), false)
            .unwrap();
        assert!(
            frame.changed,
            "scrolling must update the scene without a forced frame"
        );
    }
}

fn time(ms: u64) -> MonotonicInstant {
    MonotonicInstant::from_nanos(ms * 1_000_000)
}
fn fixture() -> ComposedAppRuntime {
    let mut runtime = ComposedAppRuntime::from_composed_with_extent(
        ScrollFixture {},
        SizeI {
            width: 200,
            height: 150,
        },
    )
    .unwrap();
    runtime.prepare_frame(time(0), true).unwrap();
    runtime.queue_input(InputEvent::mouse_moved(PointF { x: 20.0, y: 20.0 }));
    runtime.flush_input(time(1));
    runtime.prepare_frame(time(1), false).unwrap();
    runtime
}
fn offset(runtime: &ComposedAppRuntime) -> f32 {
    let node = runtime
        .ui()
        .nodes
        .alive()
        .iter()
        .copied()
        .find(|n| runtime.ui().kinds.get(*n) == Some(&NodeKind::Scroll))
        .unwrap();
    runtime
        .ui()
        .layouts
        .get(node)
        .map_or(0.0, |s| s.scroll_offset.y)
}
fn send(runtime: &mut ComposedAppRuntime, ms: u64, event: InputEvent) {
    runtime.queue_input(event);
    runtime.flush_input(time(ms));
    runtime.prepare_frame(time(ms), false).unwrap();
}
fn wheel(steps: f32) -> InputEvent {
    InputEvent::mouse_wheel(PointF { x: 0.0, y: -steps })
}

#[test]
fn wheel_eases_extends_destination_and_stops_requesting_frames() {
    let mut runtime = fixture();
    send(&mut runtime, 10, wheel(1.0));
    assert!(runtime.animation_active());
    runtime.prepare_frame(time(26), false).unwrap();
    assert!(offset(&runtime) > 0.0 && offset(&runtime) < 48.0);
    send(&mut runtime, 30, wheel(1.0));
    runtime.prepare_frame(time(130), false).unwrap();
    assert_eq!(
        offset(&runtime),
        96.0,
        "a second notch extends the destination"
    );
    assert!(!runtime.animation_active());
    runtime.prepare_frame(time(131), false).unwrap();
    assert!(!runtime.needs_frame(), "idle scrolls must let hosts sleep");
}

#[test]
fn reversal_cancels_remaining_travel_and_precise_input_is_immediate() {
    let mut runtime = fixture();
    send(&mut runtime, 10, wheel(4.0));
    runtime.prepare_frame(time(60), false).unwrap();
    let before = offset(&runtime);
    send(&mut runtime, 60, wheel(-1.0));
    runtime.prepare_frame(time(76), false).unwrap();
    assert!(
        offset(&runtime) < before,
        "the very next frame must reverse direction"
    );
    runtime.prepare_frame(time(160), false).unwrap();
    assert!((offset(&runtime) - (before - 48.0)).abs() < 0.001);
    send(&mut runtime, 170, wheel(2.0));
    let before = offset(&runtime);
    send(
        &mut runtime,
        170,
        InputEvent::mouse_scroll(PointF { x: 0.0, y: -1.25 }),
    );
    assert_eq!(offset(&runtime), before + 1.25);
    assert!(
        !runtime.animation_active(),
        "precise input cancels synthetic wheel motion"
    );
    runtime.prepare_frame(time(300), false).unwrap();
    assert_eq!(
        offset(&runtime),
        before + 1.25,
        "do not add a second momentum tail"
    );
}

#[test]
fn reduced_motion_speed_adjustment_and_fractional_steps() {
    let mut runtime = fixture();
    runtime.set_motion_preference(MotionPreference::Reduced);
    send(&mut runtime, 10, wheel(0.5));
    assert_eq!(offset(&runtime), 24.0);
    assert!(!runtime.animation_active());
    runtime.set_motion_preference(MotionPreference::Full);
    runtime.set_wheel_scroll_settings(WheelScrollSettings::new(80.0, Duration::ZERO).unwrap());
    send(&mut runtime, 20, wheel(0.5));
    assert_eq!(offset(&runtime), 64.0);
    assert!(!runtime.animation_active());
    assert!(WheelScrollSettings::new(f32::NAN, Duration::ZERO).is_err());
}

#[test]
fn boundaries_resize_and_view_deactivation_retire_motion() {
    let mut runtime = fixture();
    send(&mut runtime, 10, wheel(100.0));
    runtime.prepare_frame(time(110), false).unwrap();
    assert_eq!(offset(&runtime), 450.0);
    assert!(!runtime.animation_active());
    send(&mut runtime, 120, wheel(-2.0));
    runtime
        .resize(SizeI {
            width: 200,
            height: 800,
        })
        .unwrap();
    runtime.prepare_frame(time(130), false).unwrap();
    assert_eq!(offset(&runtime), 0.0);
    assert!(!runtime.animation_active());
    runtime
        .resize(SizeI {
            width: 200,
            height: 150,
        })
        .unwrap();
    runtime.prepare_frame(time(140), false).unwrap();
    send(&mut runtime, 150, wheel(2.0));
    runtime.prepare_frame(time(166), false).unwrap();
    let before = offset(&runtime);
    runtime.deactivate_view(time(167));
    runtime.prepare_frame(time(300), false).unwrap();
    assert_eq!(offset(&runtime), before);
    assert!(!runtime.animation_active());
}

#[crate::component]
struct NestedFixture {}
impl Component for NestedFixture {
    fn view(&self) -> impl View {
        column()
            .scrollable()
            .child(
                column()
                    .height(100.0)
                    .scrollable()
                    .children((0..3).map(|_| column().height(100.0))),
            )
            .children((0..4).map(|_| column().height(100.0)))
    }
}
#[test]
fn nested_boundaries_pass_only_unused_distance_to_parent() {
    let mut runtime = ComposedAppRuntime::from_composed_with_extent(
        NestedFixture {},
        SizeI {
            width: 200,
            height: 150,
        },
    )
    .unwrap();
    runtime.prepare_frame(time(0), true).unwrap();
    // Pointer and wheel in the same batch must keep their order.
    runtime.queue_input(InputEvent::mouse_moved(PointF { x: 20.0, y: 20.0 }));
    send(&mut runtime, 10, wheel(5.0)); // 240 px: inner takes 200, outer takes 40.
    runtime.prepare_frame(time(110), false).unwrap();
    let offsets = runtime
        .ui()
        .nodes
        .alive()
        .iter()
        .copied()
        .filter(|n| runtime.ui().kinds.get(*n) == Some(&NodeKind::Scroll))
        .map(|n| {
            runtime
                .ui()
                .layouts
                .get(n)
                .map_or(0.0, |s| s.scroll_offset.y)
        })
        .collect::<Vec<_>>();
    assert_eq!(offsets, vec![40.0, 200.0]);
    assert!(!runtime.animation_active());
}

#[test]
fn explicit_offset_replacement_and_unmount_do_not_keep_stale_motion() {
    let mut runtime = fixture();
    let node = runtime
        .ui()
        .nodes
        .alive()
        .iter()
        .copied()
        .find(|n| runtime.ui().kinds.get(*n) == Some(&NodeKind::Scroll))
        .unwrap();
    let mut ui = runtime.ui().clone();
    let mut scroll = ScrollRuntime::default();
    scroll.wheel(
        &mut ui,
        runtime.layout(),
        node,
        PointF { x: 0.0, y: -48.0 },
        ScrollPrecision::Discrete,
        time(0),
        MotionPreference::Full,
    );
    let mut style = ui.layouts.get(node).copied().unwrap_or_default();
    style.scroll_offset.y = 100.0;
    ui.set_layout_style(node, style);
    scroll.advance(&mut ui, runtime.layout(), time(50), MotionPreference::Full);
    assert!(!scroll.active());
    assert_eq!(ui.layouts.get(node).unwrap().scroll_offset.y, 100.0);
    send(&mut runtime, 10, wheel(1.0));
    runtime.close_composition().unwrap();
    runtime.prepare_frame(time(20), false).unwrap();
    assert!(!runtime.animation_active());
}

use super::*;

use crate::authoring::compose::{ComponentFields, View, column};
use crate::foundation::{ColorRgba8, PointF, SizeF};
use crate::graphics::presentation::{AlphaMode, ColorSpace, SurfaceRevision};
use crate::input::{ButtonState, InputEvent, PointerButton};
use crate::runtime::{CreateContext, State, Ui, UpdateContext};
use crate::ui::{Background, BoxDecoration, BoxStyle, LayoutStyle, SizeRule, UiRoot};

const RED: ColorRgba8 = ColorRgba8::rgba(220, 30, 10, 255);
const GREEN: ColorRgba8 = ColorRgba8::rgba(10, 200, 30, 255);

fn metrics(scale: f64) -> SurfaceMetrics {
    SurfaceMetrics {
        revision: SurfaceRevision::new(1),
        logical_extent: SizeF {
            width: 32.0,
            height: 16.0,
        },
        physical_extent: SizeI {
            width: (32.0 * scale) as i32,
            height: (16.0 * scale) as i32,
        },
        scale_factor: scale,
        color_space: ColorSpace::Srgb,
        alpha_mode: AlphaMode::Opaque,
    }
}

struct ClickColor;

impl Component for ClickColor {
    type State = State<ColorRgba8>;
    type Action = ();

    fn create(&self, cx: &mut CreateContext<'_>) -> Self::State {
        cx.state(RED)
    }

    fn mount(&self, color: &Self::State, ui: &mut Ui<'_, '_, ()>) -> UiRoot {
        let root = ui.foundation().root(
            BoxStyle {
                width: SizeRule::Fill(1.0),
                height: SizeRule::Fill(1.0),
                ..BoxStyle::default()
            },
            LayoutStyle::default(),
            |_| {},
        );
        let button = ui
            .button(
                root.0,
                || (),
                BoxStyle {
                    width: SizeRule::Logical(20.0),
                    height: SizeRule::Logical(12.0),
                    decoration: BoxDecoration {
                        background: Background::Color(RED),
                        ..BoxDecoration::default()
                    },
                    ..BoxStyle::default()
                },
                |_| {},
            )
            .unwrap();
        ui.bind(*color, button.background).unwrap();
        root
    }

    fn action(&self, color: &mut Self::State, _: (), cx: &mut UpdateContext<'_, Self>) {
        cx.set(*color, GREEN).unwrap();
    }
}

fn pixel(frame: &SoftwareUiFrame<'_>, x: usize, y: usize) -> [u8; 4] {
    let offset = (y * frame.metrics.physical_extent.width as usize + x) * 4;
    frame.pixels_rgba8().unwrap()[offset..offset + 4]
        .try_into()
        .unwrap()
}

#[test]
fn input_updates_component_and_cpu_pixels_then_idle_returns_no_stale_damage() {
    let mut host = SoftwareUiHost::new(ClickColor, metrics(1.0)).unwrap();
    {
        let frame = host.advance(MonotonicInstant::ZERO, false).unwrap();
        assert!(frame.render.recorded);
        assert_eq!(pixel(&frame, 6, 6), [220, 30, 10, 255]);
        assert!(frame.damage().unwrap().full);
    }
    host.queue_input(InputEvent::mouse_moved(PointF { x: 6.0, y: 6.0 }));
    host.queue_input(InputEvent::mouse_button(
        PointerButton::PRIMARY,
        ButtonState::Pressed,
    ));
    host.queue_input(InputEvent::mouse_button(
        PointerButton::PRIMARY,
        ButtonState::Released,
    ));
    {
        let frame = host
            .advance(MonotonicInstant::from_nanos(1), false)
            .unwrap();
        assert_eq!(frame.input.events_dispatched, 3);
        assert!(frame.render.recorded);
        assert_eq!(pixel(&frame, 6, 6), [10, 200, 30, 255]);
    }
    let frame = host
        .advance(MonotonicInstant::from_nanos(2), false)
        .unwrap();
    assert!(!frame.render.recorded);
    assert!(frame.surface().is_none());
    assert!(frame.damage().is_none());
}

#[test]
fn adopting_rendered_runtime_initializes_complete_scene_and_scales_logical_input() {
    let mut runtime = AppRuntime::with_extent(
        ClickColor,
        SizeI {
            width: 32,
            height: 16,
        },
    )
    .unwrap();
    runtime.prepare_frame(MonotonicInstant::ZERO, true).unwrap();
    while runtime.pop_scene_delta().is_some() {}
    let mut host = SoftwareUiHost::from_runtime(runtime, metrics(2.0)).unwrap();
    {
        let frame = host.advance(MonotonicInstant::ZERO, false).unwrap();
        assert_eq!(pixel(&frame, 12, 12), [220, 30, 10, 255]);
        assert_eq!(frame.pixels_rgba8().unwrap().len(), 64 * 32 * 4);
    }
    host.queue_input(InputEvent::mouse_moved(PointF { x: 6.0, y: 6.0 }));
    host.queue_input(InputEvent::mouse_button(
        PointerButton::PRIMARY,
        ButtonState::Pressed,
    ));
    host.queue_input(InputEvent::mouse_button(
        PointerButton::PRIMARY,
        ButtonState::Released,
    ));
    let frame = host
        .advance(MonotonicInstant::from_nanos(1), false)
        .unwrap();
    assert_eq!(pixel(&frame, 12, 12), [10, 200, 30, 255]);
    assert_eq!(pixel(&frame, 48, 12), [0, 0, 0, 0]);
}

struct Solid;

impl ComponentFields for Solid {
    type InputSnapshot = ();
    fn update_inputs(&mut self, _: Self) -> bool {
        false
    }
    fn capture_inputs(&self) -> Self::InputSnapshot {}
    fn restore_inputs(&mut self, _: Self::InputSnapshot) -> bool {
        false
    }
}

impl crate::authoring::compose::Component for Solid {
    fn view(&self) -> impl View {
        column().background(RED)
    }
}

#[test]
fn composed_runtime_suspends_and_reconfigures_without_losing_component_state() {
    let mut host = SoftwareUiHost::from_composed(Solid, metrics(1.0)).unwrap();
    assert_eq!(
        pixel(
            &host.advance(MonotonicInstant::ZERO, false).unwrap(),
            30,
            14
        ),
        [220, 30, 10, 255]
    );
    let mut suspended = metrics(1.0);
    suspended.physical_extent = SizeI {
        width: 0,
        height: 0,
    };
    host.configure(suspended).unwrap();
    assert!(
        host.advance(MonotonicInstant::from_nanos(1), false)
            .unwrap()
            .surface()
            .is_none()
    );
    host.configure(metrics(2.0)).unwrap();
    {
        let frame = host
            .advance(MonotonicInstant::from_nanos(2), false)
            .unwrap();
        assert!(frame.damage().unwrap().full);
        assert_eq!(pixel(&frame, 62, 30), [220, 30, 10, 255]);
    }
    assert!(matches!(
        host.advance(MonotonicInstant::from_nanos(1), false),
        Err(SoftwareUiHostError::TimeRegression)
    ));
    let before = host.metrics();
    let mut invalid = before;
    invalid.scale_factor = f64::MIN_POSITIVE;
    assert!(host.configure(invalid).is_err());
    assert_eq!(host.metrics(), before);
}

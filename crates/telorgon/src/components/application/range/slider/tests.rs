use std::cell::{Cell, RefCell};

use crate::input::{GestureArenaRequest, PointerButton, PointerId};
use crate::runtime::{Component, CreateContext, State, UpdateContext, ViewRuntime};
use crate::ui::{SemanticAction, UiRoot};

use crate::components::application::{DensityClass, RangeFormat};

use super::*;

fn model() -> RangeModel<f64> {
    RangeModel::new(0.0, 10.0, 1.0, 5.0).unwrap()
}

fn behavior(
    orientation: SliderOrientation,
    direction: WritingDirection,
    reversed: bool,
) -> SliderBehavior<f64> {
    SliderBehavior::new(model(), orientation, direction, reversed, true).unwrap()
}

fn requested(
    behavior: &SliderBehavior<f64>,
    command: SliderCommand,
) -> Option<ValueChange<f64>> {
    behavior
        .request(5.0, command, ChangeSource::Directional)
        .unwrap()
}

#[test]
fn directional_page_and_bound_commands_follow_axis_direction_and_reversal() {
    let ltr = behavior(
        SliderOrientation::Horizontal,
        WritingDirection::LeftToRight,
        false,
    );
    assert_eq!(
        requested(&ltr, SliderCommand::ArrowLeft).unwrap().value,
        4.0
    );
    assert_eq!(
        requested(&ltr, SliderCommand::ArrowRight).unwrap().value,
        6.0
    );
    assert_eq!(requested(&ltr, SliderCommand::ArrowUp), None);
    assert_eq!(requested(&ltr, SliderCommand::PageUp).unwrap().value, 10.0);
    assert_eq!(requested(&ltr, SliderCommand::PageDown).unwrap().value, 0.0);
    assert_eq!(requested(&ltr, SliderCommand::Home).unwrap().value, 0.0);
    assert_eq!(requested(&ltr, SliderCommand::End).unwrap().value, 10.0);

    let rtl = behavior(
        SliderOrientation::Horizontal,
        WritingDirection::RightToLeft,
        false,
    );
    assert_eq!(
        requested(&rtl, SliderCommand::ArrowRight).unwrap().value,
        4.0
    );
    let reversed = behavior(
        SliderOrientation::Horizontal,
        WritingDirection::LeftToRight,
        true,
    );
    assert_eq!(
        requested(&reversed, SliderCommand::ArrowRight)
            .unwrap()
            .value,
        4.0
    );

    let vertical = behavior(
        SliderOrientation::Vertical,
        WritingDirection::RightToLeft,
        false,
    );
    assert_eq!(
        requested(&vertical, SliderCommand::ArrowUp).unwrap().value,
        6.0
    );
    assert_eq!(
        requested(&vertical, SliderCommand::ArrowDown)
            .unwrap()
            .value,
        4.0
    );
    assert_eq!(requested(&vertical, SliderCommand::ArrowLeft), None);
    assert_eq!(
        vertical
            .request(5.0, SliderCommand::Increment, ChangeSource::Accessibility)
            .unwrap(),
        Some(ValueChange::new(
            6.0,
            ChangePhase::Commit,
            ChangeSource::Accessibility,
        ))
    );
    assert_eq!(
        ltr.request(10.0, SliderCommand::End, ChangeSource::Keyboard)
            .unwrap(),
        None
    );
}

#[test]
fn drag_emits_phases_suppresses_duplicate_updates_and_restores_start_on_cancel() {
    let mut behavior = behavior(
        SliderOrientation::Horizontal,
        WritingDirection::LeftToRight,
        false,
    );
    let pointer = PointerId::new(9);
    let track = SliderTrackGeometry::new(0.0, 100.0).unwrap();
    assert_eq!(
        behavior
            .handle_pointer(
                5.0,
                GestureInput::PointerDown {
                    pointer,
                    button: PointerButton::PRIMARY,
                    position: PointF { x: 50.0, y: 0.0 },
                },
                track,
            )
            .unwrap()
            .change,
        None
    );
    assert_eq!(
        behavior
            .handle_pointer(
                5.0,
                GestureInput::PointerMoved {
                    pointer,
                    position: PointF { x: 61.0, y: 0.0 },
                },
                track,
            )
            .unwrap()
            .arena,
        GestureArenaRequest::Accept(pointer)
    );
    let begin = behavior
        .handle_pointer(5.0, GestureInput::ArenaWon { pointer }, track)
        .unwrap()
        .change
        .unwrap();
    assert_eq!(
        begin,
        ValueChange::new(6.0, ChangePhase::Begin, ChangeSource::Pointer)
    );
    assert_eq!(
        behavior
            .handle_pointer(
                5.0,
                GestureInput::PointerMoved {
                    pointer,
                    position: PointF { x: 62.0, y: 0.0 },
                },
                track,
            )
            .unwrap()
            .change,
        None
    );
    let update = behavior
        .handle_pointer(
            5.0,
            GestureInput::PointerMoved {
                pointer,
                position: PointF { x: 78.0, y: 0.0 },
            },
            track,
        )
        .unwrap()
        .change
        .unwrap();
    assert_eq!(update.value, 8.0);
    assert_eq!(update.phase, ChangePhase::Update);
    let cancel = behavior
        .handle_pointer(5.0, GestureInput::PointerCancelled { pointer }, track)
        .unwrap()
        .change
        .unwrap();
    assert_eq!(
        cancel,
        ValueChange::new(5.0, ChangePhase::Cancel, ChangeSource::Pointer)
    );
}

#[test]
fn completed_drag_commits_and_disabled_or_invalid_geometry_cannot_request() {
    let mut behavior = behavior(
        SliderOrientation::Horizontal,
        WritingDirection::LeftToRight,
        false,
    );
    let pointer = PointerId::new(3);
    let track = SliderTrackGeometry::new(0.0, 100.0).unwrap();
    behavior
        .handle_pointer(
            2.0,
            GestureInput::PointerDown {
                pointer,
                button: PointerButton::PRIMARY,
                position: PointF { x: 20.0, y: 0.0 },
            },
            track,
        )
        .unwrap();
    let arena = behavior
        .handle_pointer(
            2.0,
            GestureInput::PointerMoved {
                pointer,
                position: PointF { x: 40.0, y: 0.0 },
            },
            track,
        )
        .unwrap();
    assert_eq!(arena.arena, GestureArenaRequest::Accept(pointer));
    let begin = behavior
        .handle_pointer(2.0, GestureInput::ArenaWon { pointer }, track)
        .unwrap()
        .change
        .unwrap();
    assert_eq!(begin.phase, ChangePhase::Begin);
    let commit = behavior
        .handle_pointer(
            2.0,
            GestureInput::PointerUp {
                pointer,
                button: PointerButton::PRIMARY,
                position: PointF { x: 70.0, y: 0.0 },
            },
            track,
        )
        .unwrap()
        .change
        .unwrap();
    assert_eq!(
        commit,
        ValueChange::new(7.0, ChangePhase::Commit, ChangeSource::Pointer)
    );
    behavior
        .handle_pointer(7.0, GestureInput::SetEnabled(false), track)
        .unwrap();
    assert_eq!(
        behavior
            .request(7.0, SliderCommand::Increment, ChangeSource::Keyboard)
            .unwrap(),
        None
    );
    assert_eq!(
        SliderTrackGeometry::new(0.0, 0.0),
        Err(SliderError::InvalidTrackGeometry)
    );
}

#[test]
fn style_priority_is_deterministic() {
    let style = SliderStyle::default();
    let resolved = style.resolve(SliderInteractionState {
        enabled: true,
        hovered: true,
        focused: true,
        dragging: true,
    });
    assert_eq!(resolved.state, SliderStyleState::Dragging);
    let disabled = style.resolve(SliderInteractionState {
        enabled: false,
        hovered: true,
        focused: true,
        dragging: true,
    });
    assert_eq!(disabled.state, SliderStyleState::Disabled);
}

#[test]
fn thumb_is_centered_on_the_track_cross_axis() {
    let mut horizontal = SliderStyle::default().resting;
    configure_visual_geometry(
        &mut horizontal,
        SliderOrientation::Horizontal,
        0.25,
        false,
        WritingDirection::LeftToRight,
    );
    assert_eq!(horizontal.track_thickness, 6.0);
    assert_eq!(horizontal.thumb_size, 18.0);
    assert_eq!(horizontal.thumb.transform.translation.y, -6.0);
    assert_eq!(
        horizontal.thumb.transform.translation.y + horizontal.thumb_size * 0.5,
        horizontal.track_thickness * 0.5
    );

    let mut vertical = SliderStyle::default().resting;
    configure_visual_geometry(
        &mut vertical,
        SliderOrientation::Vertical,
        0.25,
        false,
        WritingDirection::LeftToRight,
    );
    assert_eq!(vertical.thumb.transform.translation.x, -6.0);
    assert_eq!(
        vertical.thumb.transform.translation.x + vertical.thumb_size * 0.5,
        vertical.track_thickness * 0.5
    );
}

struct MountedSlider {
    node: Rc<Cell<Option<UiNodeId>>>,
    reference: Rc<RefCell<Option<SliderRef<f64>>>>,
    enabled: bool,
}

impl Component for MountedSlider {
    type State = State<f64>;
    type Action = ();

    fn create(&self, context: &mut CreateContext<'_>) -> Self::State {
        context.state(25.0)
    }

    fn mount(&self, state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
        let root = ui
            .foundation()
            .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
        assert!(matches!(
            Slider::new(" ", state.read(), model()),
            Err(SliderError::MissingAccessibleName)
        ));
        let model = RangeModel::new(0.0_f64, 100.0, 5.0, 20.0)
            .unwrap()
            .with_format(RangeFormat::new(0).unwrap().suffix("%").unwrap());
        let reference = Slider::new("Volume", state.read(), model)
            .unwrap()
            .enabled(self.enabled)
            .density(DensityMetrics::baseline(DensityClass::Touch))
            .mount(ui, root.0)
            .unwrap();
        self.node.set(Some(reference.node()));
        *self.reference.borrow_mut() = Some(reference);
        root
    }

    fn action(
        &self,
        _state: &mut Self::State,
        _action: Self::Action,
        _context: &mut UpdateContext<'_, Self>,
    ) {
    }
}

#[test]
fn mounted_slider_exposes_named_range_semantics_touch_floor_and_focused_behavior() {
    let node = Rc::new(Cell::new(None));
    let reference = Rc::new(RefCell::new(None));
    let runtime = ViewRuntime::from_component(MountedSlider {
        node: node.clone(),
        reference: reference.clone(),
        enabled: true,
    })
    .unwrap();
    let node = node.get().unwrap();
    let semantic = runtime.ui().semantics.get(node).unwrap();
    assert_eq!(semantic.role, SemanticRole::Slider);
    assert!(semantic.actions.contains(SemanticAction::Increment));
    assert!(semantic.actions.contains(SemanticAction::Decrement));
    let SemanticValue::Number {
        current,
        minimum,
        maximum,
        step,
        value_text,
    } = semantic.value
    else {
        panic!("slider must expose a numeric semantic value");
    };
    assert_eq!(
        (current, minimum, maximum, step),
        (25.0, 0.0, 100.0, Some(5.0))
    );
    assert_eq!(runtime.ui().string(value_text.unwrap()), Some("25%"));
    assert_eq!(
        runtime.ui().box_styles.get(node).unwrap().min_size,
        SizeRule2D {
            width: SizeRule::Logical(44.0),
            height: SizeRule::Logical(44.0),
        }
    );
    assert_eq!(
        reference
            .borrow()
            .as_ref()
            .unwrap()
            .request(25.0, SliderCommand::Increment, ChangeSource::Programmatic)
            .unwrap(),
        Some(ValueChange::new(
            30.0,
            ChangePhase::Commit,
            ChangeSource::Programmatic,
        ))
    );
}

#[test]
fn disabled_mounted_slider_keeps_value_semantics_but_suppresses_actions() {
    let node = Rc::new(Cell::new(None));
    let reference = Rc::new(RefCell::new(None));
    let runtime = ViewRuntime::from_component(MountedSlider {
        node: node.clone(),
        reference,
        enabled: false,
    })
    .unwrap();
    let semantic = runtime.ui().semantics.get(node.get().unwrap()).unwrap();
    assert!(semantic.state.disabled);
    assert!(semantic.effective_actions().is_empty());
    assert!(matches!(
        semantic.value,
        SemanticValue::Number { current: 25.0, .. }
    ));
}

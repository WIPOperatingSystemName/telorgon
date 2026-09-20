use std::cell::RefCell;

use crate::input::{PointerButton, PointerId};
use crate::runtime::{Component, CreateContext, State, UpdateContext, ViewRuntime};
use crate::ui::{SemanticAction, UiRoot};

use super::*;
use crate::components::application::DensityClass;

fn model() -> RangeModel<f64> {
    RangeModel::new(0.0, 10.0, 1.0, 5.0).unwrap()
}

fn behavior(policy: RangeSliderCrossingPolicy) -> RangeSliderBehavior<f64> {
    RangeSliderBehavior::new(
        model(),
        policy,
        SliderOrientation::Horizontal,
        WritingDirection::LeftToRight,
        false,
        true,
    )
    .unwrap()
}

#[test]
fn crossing_policy_clamps_or_swaps_with_explicit_active_thumb() {
    let current = RangeSliderValue::new(2.0, 8.0);
    let clamped = behavior(RangeSliderCrossingPolicy::Clamp)
        .propose(
            current,
            RangeSliderThumb::Lower,
            10.0,
            ChangePhase::Update,
            ChangeSource::Pointer,
        )
        .unwrap();
    assert_eq!(clamped.value(), &RangeSliderValue::new(8.0, 8.0));
    assert_eq!(clamped.active_thumb(), RangeSliderThumb::Lower);
    assert!(!clamped.role_swapped());

    let swapped = behavior(RangeSliderCrossingPolicy::Swap)
        .propose(
            current,
            RangeSliderThumb::Lower,
            10.0,
            ChangePhase::Update,
            ChangeSource::Pointer,
        )
        .unwrap();
    assert_eq!(swapped.value(), &RangeSliderValue::new(8.0, 10.0));
    assert_eq!(swapped.requested_thumb(), RangeSliderThumb::Lower);
    assert_eq!(swapped.active_thumb(), RangeSliderThumb::Upper);
    assert!(swapped.role_swapped());
}

#[test]
fn independent_commands_are_committed_source_preserving_and_nonmutating() {
    let current = RangeSliderValue::new(2.0, 8.0);
    let proposal = behavior(RangeSliderCrossingPolicy::Clamp)
        .request(
            current,
            RangeSliderThumb::Upper,
            SliderCommand::Decrement,
            ChangeSource::Accessibility,
        )
        .unwrap()
        .unwrap();
    assert_eq!(proposal.value(), &RangeSliderValue::new(2.0, 7.0));
    assert_eq!(proposal.phase(), ChangePhase::Commit);
    assert_eq!(proposal.source(), ChangeSource::Accessibility);
    assert_eq!(current, RangeSliderValue::new(2.0, 8.0));
    assert_eq!(
        behavior(RangeSliderCrossingPolicy::Clamp)
            .validate_value(RangeSliderValue::new(9.0, 3.0)),
        Err(RangeSliderError::UnorderedControlledValue)
    );
}

#[test]
fn pointer_lifecycle_reuses_shared_drag_phases_and_cancels_to_the_start() {
    let mut behavior = behavior(RangeSliderCrossingPolicy::Clamp);
    let current = RangeSliderValue::new(2.0, 8.0);
    let pointer = PointerId::new(7);
    let track = SliderTrackGeometry::new(0.0, 100.0).unwrap();
    behavior
        .handle_pointer(
            current,
            RangeSliderThumb::Lower,
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
            current,
            RangeSliderThumb::Lower,
            GestureInput::PointerMoved {
                pointer,
                position: PointF { x: 40.0, y: 0.0 },
            },
            track,
        )
        .unwrap();
    assert_eq!(arena.arena, GestureArenaRequest::Accept(pointer));
    let begin = behavior
        .handle_pointer(
            current,
            RangeSliderThumb::Lower,
            GestureInput::ArenaWon { pointer },
            track,
        )
        .unwrap()
        .proposal
        .unwrap();
    assert_eq!(begin.phase(), ChangePhase::Begin);
    assert_eq!(begin.source(), ChangeSource::Pointer);
    assert_eq!(begin.value(), &RangeSliderValue::new(4.0, 8.0));

    let update = behavior
        .handle_pointer(
            current,
            RangeSliderThumb::Lower,
            GestureInput::PointerMoved {
                pointer,
                position: PointF { x: 60.0, y: 0.0 },
            },
            track,
        )
        .unwrap()
        .proposal
        .unwrap();
    assert_eq!(update.phase(), ChangePhase::Update);
    assert_eq!(update.value(), &RangeSliderValue::new(6.0, 8.0));

    let cancel = behavior
        .handle_pointer(
            current,
            RangeSliderThumb::Lower,
            GestureInput::PointerCancelled { pointer },
            track,
        )
        .unwrap()
        .proposal
        .unwrap();
    assert_eq!(cancel.phase(), ChangePhase::Cancel);
    assert_eq!(cancel.value(), &current);
}

struct Fixture {
    reference: Rc<RefCell<Option<RangeSliderRef<f64>>>>,
}

impl Component for Fixture {
    type State = State<RangeSliderValue<f64>>;
    type Action = ();

    fn create(&self, context: &mut CreateContext<'_>) -> Self::State {
        context.state(RangeSliderValue::new(20.0, 80.0))
    }

    fn mount(&self, state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
        let root = ui
            .foundation()
            .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
        let range = RangeSlider::new(
            "Price range",
            "Minimum price",
            "Maximum price",
            state.read(),
            RangeModel::new(0.0, 100.0, 5.0, 20.0).unwrap(),
        )
        .unwrap()
        .density(DensityMetrics::baseline(DensityClass::Touch));
        *self.reference.borrow_mut() = Some(range.mount(ui, root.0).unwrap());
        root
    }

    fn action(&self, _: &mut Self::State, _: Self::Action, _: &mut UpdateContext<'_, Self>) {}
}

#[test]
fn mounted_thumbs_have_stable_independent_semantics_and_density_targets() {
    let reference = Rc::new(RefCell::new(None));
    let runtime = ViewRuntime::from_component(Fixture {
        reference: reference.clone(),
    })
    .unwrap();
    let reference = reference.borrow();
    let reference = reference.as_ref().unwrap();
    let root = runtime.ui().semantics.get(reference.node()).unwrap();
    assert_eq!(root.relationships.len(), 2);
    for (thumb, expected, bounds) in [
        (RangeSliderThumb::Lower, 20.0, (0.0, 80.0)),
        (RangeSliderThumb::Upper, 80.0, (20.0, 100.0)),
    ] {
        let node = reference.thumb_node(thumb);
        let semantics = runtime.ui().semantics.get(node).unwrap();
        assert_eq!(semantics.role, SemanticRole::Slider);
        assert!(semantics.actions.contains(SemanticAction::Increment));
        assert!(semantics.actions.contains(SemanticAction::Decrement));
        let SemanticValue::Number {
            current,
            minimum,
            maximum,
            ..
        } = semantics.value
        else {
            panic!("range thumb must expose numeric semantics");
        };
        assert_eq!((current, minimum, maximum), (expected, bounds.0, bounds.1));
        assert_eq!(
            runtime.ui().box_styles.get(node).unwrap().min_size,
            SizeRule2D {
                width: SizeRule::Logical(44.0),
                height: SizeRule::Logical(44.0),
            }
        );
    }
}

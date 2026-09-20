use std::cell::{Cell, RefCell};

use crate::foundation::PointF;
use crate::input::{PointerButton, PointerId};
use crate::runtime::{
    Component, ComponentRuntimeDriver, CreateContext, State, UpdateContext, ViewRuntime,
};
use crate::ui::{SemanticAction, UiRoot};

use super::*;
use crate::components::application::DensityClass;

fn constraints() -> SplitViewConstraints {
    SplitViewConstraints::new(100.0, 20.0, 30.0, 5.0, 20.0).unwrap()
}

fn behavior(
    collapse: SplitViewCollapsePolicy,
    orientation: SplitViewOrientation,
) -> SplitViewBehavior {
    SplitViewBehavior::new(constraints(), collapse, orientation, true).unwrap()
}

#[test]
fn constraints_validate_both_minimums_and_expose_the_resizable_interval() {
    let constraints = constraints();
    assert_eq!(constraints.divider_model().minimum(), 20.0);
    assert_eq!(constraints.divider_model().maximum(), 70.0);
    assert_eq!(
        constraints
            .effective_extents(SplitViewValue::expanded(45.0))
            .unwrap(),
        (45.0, 55.0)
    );
    assert_eq!(
        SplitViewConstraints::new(100.0, 60.0, 40.0, 5.0, 20.0),
        Err(SplitViewError::InsufficientResizableExtent)
    );
    assert_eq!(
        SplitViewConstraints::new(100.0, -1.0, 20.0, 5.0, 20.0),
        Err(SplitViewError::InvalidPrimaryMinimum)
    );
}

#[test]
fn keyboard_resize_is_axis_aware_source_preserving_and_nonmutating() {
    let current = SplitViewValue::expanded(40.0);
    let horizontal = behavior(
        SplitViewCollapsePolicy::Disabled,
        SplitViewOrientation::Horizontal,
    )
    .request(
        current,
        SplitViewCommand::ArrowRight,
        ChangeSource::Directional,
    )
    .unwrap()
    .unwrap();
    assert_eq!(horizontal.value(), SplitViewValue::expanded(45.0));
    assert_eq!(horizontal.operation(), SplitViewOperation::Resize);
    assert_eq!(horizontal.phase(), ChangePhase::Commit);
    assert_eq!(horizontal.source(), ChangeSource::Directional);
    assert_eq!(current, SplitViewValue::expanded(40.0));

    let vertical = behavior(
        SplitViewCollapsePolicy::Disabled,
        SplitViewOrientation::Vertical,
    )
    .request(
        current,
        SplitViewCommand::ArrowDown,
        ChangeSource::Accessibility,
    )
    .unwrap()
    .unwrap();
    assert_eq!(vertical.value(), SplitViewValue::expanded(45.0));
}

#[test]
fn collapse_retains_restore_position_and_blocks_resize_until_restored() {
    let behavior = behavior(
        SplitViewCollapsePolicy::Secondary,
        SplitViewOrientation::Horizontal,
    );
    let current = SplitViewValue::expanded(40.0);
    let collapse = behavior
        .request(
            current,
            SplitViewCommand::Collapse,
            ChangeSource::Accessibility,
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        collapse.value(),
        SplitViewValue::collapsed(40.0, SplitViewPane::Secondary)
    );
    assert_eq!(
        collapse.operation(),
        SplitViewOperation::Collapse(SplitViewPane::Secondary)
    );
    assert_eq!(
        behavior.propose_resize(
            collapse.value(),
            60.0,
            ChangePhase::Update,
            ChangeSource::Pointer
        ),
        Err(SplitViewError::CannotResizeCollapsed)
    );
    let restore = behavior
        .request(
            collapse.value(),
            SplitViewCommand::Restore,
            ChangeSource::Programmatic,
        )
        .unwrap()
        .unwrap();
    assert_eq!(restore.value(), current);
    assert_eq!(
        restore.operation(),
        SplitViewOperation::Restore(SplitViewPane::Secondary)
    );
}

#[test]
fn pointer_cancel_restores_the_controlled_interaction_start() {
    let mut behavior = behavior(
        SplitViewCollapsePolicy::Disabled,
        SplitViewOrientation::Horizontal,
    );
    let current = SplitViewValue::expanded(40.0);
    let pointer = PointerId::new(11);
    let track = SliderTrackGeometry::new(20.0, 50.0).unwrap();
    behavior
        .handle_pointer(
            current,
            GestureInput::PointerDown {
                pointer,
                button: PointerButton::PRIMARY,
                position: PointF { x: 40.0, y: 0.0 },
            },
            track,
        )
        .unwrap();
    let arena = behavior
        .handle_pointer(
            current,
            GestureInput::PointerMoved {
                pointer,
                position: PointF { x: 50.0, y: 0.0 },
            },
            track,
        )
        .unwrap();
    assert_eq!(arena.arena, GestureArenaRequest::Accept(pointer));
    let begin = behavior
        .handle_pointer(current, GestureInput::ArenaWon { pointer }, track)
        .unwrap()
        .proposal
        .unwrap();
    assert_eq!(begin.phase(), ChangePhase::Begin);
    assert_eq!(begin.value(), SplitViewValue::expanded(50.0));
    let cancel = behavior
        .handle_pointer(current, GestureInput::PointerCancelled { pointer }, track)
        .unwrap()
        .proposal
        .unwrap();
    assert_eq!(cancel.phase(), ChangePhase::Cancel);
    assert_eq!(cancel.value(), current);
}

struct Fixture {
    initial: SplitViewValue,
    reference: Rc<RefCell<Option<SplitViewRef>>>,
    content_calls: Rc<Cell<usize>>,
}

impl Component for Fixture {
    type State = State<SplitViewValue>;
    type Action = ();

    fn create(&self, context: &mut CreateContext<'_>) -> Self::State {
        context.state(self.initial)
    }

    fn mount(&self, state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
        let root = ui
            .foundation()
            .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
        let split = SplitView::new(
            "Editor split",
            "Source pane",
            "Preview pane",
            "Resize editor panes",
            state.read(),
            constraints(),
        )
        .unwrap()
        .collapse_policy(SplitViewCollapsePolicy::Secondary)
        .density(DensityMetrics::baseline(DensityClass::Touch));
        let reference = split
            .mount(ui, root.0, |pane, writer| {
                self.content_calls.set(self.content_calls.get() + 1);
                writer.text(
                    format!("{pane:?}"),
                    ColorRgba8::rgba(255, 255, 255, 255),
                    12.0,
                );
            })
            .unwrap();
        *self.reference.borrow_mut() = Some(reference);
        root
    }

    fn action(&self, _: &mut Self::State, _: Self::Action, _: &mut UpdateContext<'_, Self>) {}
}

fn mounted(
    initial: SplitViewValue,
) -> (
    ViewRuntime<ComponentRuntimeDriver<Fixture>>,
    Rc<RefCell<Option<SplitViewRef>>>,
) {
    let reference = Rc::new(RefCell::new(None));
    let content_calls = Rc::new(Cell::new(0));
    let runtime = ViewRuntime::from_component(Fixture {
        initial,
        reference: reference.clone(),
        content_calls: content_calls.clone(),
    })
    .unwrap();
    assert_eq!(content_calls.get(), 2);
    (runtime, reference)
}

#[test]
fn mounted_panes_and_divider_are_stable_named_owned_and_density_aware() {
    let (runtime, reference) = mounted(SplitViewValue::expanded(40.0));
    let reference = reference.borrow();
    let reference = reference.as_ref().unwrap();
    let root = runtime.ui().semantics.get(reference.node()).unwrap();
    assert_eq!(root.relationships.len(), 3);
    for pane in [SplitViewPane::Primary, SplitViewPane::Secondary] {
        let node = reference.pane(pane).node();
        assert_eq!(
            runtime.ui().semantics.get(node).unwrap().role,
            SemanticRole::Region
        );
        assert!(
            runtime
                .ui()
                .interactions
                .get(node)
                .is_none_or(|interaction| interaction.visible)
        );
    }
    let divider = runtime
        .ui()
        .semantics
        .get(reference.divider_node())
        .unwrap();
    assert_eq!(divider.role, SemanticRole::Separator);
    assert!(divider.actions.contains(SemanticAction::Increment));
    assert!(divider.actions.contains(SemanticAction::Collapse));
    assert_eq!(divider.state.expanded, Some(true));
    assert_eq!(
        runtime
            .ui()
            .box_styles
            .get(reference.divider_node())
            .unwrap()
            .min_size,
        SizeRule2D {
            width: SizeRule::Logical(44.0),
            height: SizeRule::Logical(44.0),
        }
    );

    let (runtime, reference) =
        mounted(SplitViewValue::collapsed(40.0, SplitViewPane::Secondary));
    let reference = reference.borrow();
    let reference = reference.as_ref().unwrap();
    let secondary = reference.pane(SplitViewPane::Secondary).node();
    assert!(!runtime.ui().interactions.get(secondary).unwrap().visible);
    assert!(runtime.ui().semantics.get(secondary).unwrap().state.hidden);
    let divider = runtime
        .ui()
        .semantics
        .get(reference.divider_node())
        .unwrap();
    assert_eq!(divider.value, SemanticValue::None);
    assert!(divider.actions.contains(SemanticAction::Expand));
    assert!(!divider.actions.contains(SemanticAction::Increment));
    assert_eq!(divider.state.expanded, Some(false));
}

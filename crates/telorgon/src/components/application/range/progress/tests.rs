use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::runtime::{Component, CreateContext, State, UpdateContext, ViewRuntime};
use crate::ui::{LayoutStyle, UiRoot};

use crate::components::application::RangeFormat;

use super::*;

fn model() -> RangeModel<f64> {
    RangeModel::new(0.0, 100.0, 1.0, 10.0)
        .unwrap()
        .with_format(RangeFormat::new(0).unwrap().suffix("%").unwrap())
}

#[test]
fn style_resolution_selects_mode_and_explicit_density_variant() {
    let style = ProgressStyle::default();
    let compact = style.resolve(DensityClass::Compact, ProgressMode::Determinate);
    let touch = style.resolve(DensityClass::Touch, ProgressMode::Determinate);
    let indeterminate = style.resolve(DensityClass::Standard, ProgressMode::Indeterminate);
    assert_eq!(compact.visual.track_thickness, 3.0);
    assert_eq!(touch.visual.track_thickness, 6.0);
    assert_eq!(compact.visual.label_size, 12.0);
    assert_eq!(touch.visual.label_size, 16.0);
    assert_eq!(indeterminate.mode, ProgressMode::Indeterminate);
    assert_ne!(
        indeterminate.visual.fill.decoration.background,
        style
            .resolve(DensityClass::Standard, ProgressMode::Determinate)
            .visual
            .fill
            .decoration
            .background
    );
}

struct MountedProgress {
    initial: ProgressValue<f64>,
    density: DensityClass,
    node: Rc<Cell<Option<UiNodeId>>>,
    fill: Rc<Cell<Option<UiNodeId>>>,
    error: Rc<RefCell<Option<String>>>,
}

impl Component for MountedProgress {
    type State = State<ProgressValue<f64>>;
    type Action = ProgressValue<f64>;

    fn create(&self, context: &mut CreateContext<'_>) -> Self::State {
        context.state(self.initial)
    }

    fn mount(&self, state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
        let root = ui
            .foundation()
            .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
        assert!(matches!(
            ProgressIndicator::new(" ", state.read(), model()),
            Err(ProgressError::MissingAccessibleName)
        ));
        match ProgressIndicator::new("Download", state.read(), model())
            .unwrap()
            .density(self.density)
            .mount(ui, root.0)
        {
            Ok(reference) => {
                self.node.set(Some(reference.node()));
                self.fill.set(Some(reference.fill_node()));
            }
            Err(error) => *self.error.borrow_mut() = Some(error.to_string()),
        }
        root
    }

    fn action(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        context: &mut UpdateContext<'_, Self>,
    ) {
        context.set(*state, action).unwrap();
    }
}

fn mounted(
    initial: ProgressValue<f64>,
) -> (
    ViewRuntime<crate::runtime::ComponentRuntimeDriver<MountedProgress>>,
    UiNodeId,
    UiNodeId,
) {
    let node = Rc::new(Cell::new(None));
    let fill = Rc::new(Cell::new(None));
    let error = Rc::new(RefCell::new(None));
    let runtime = ViewRuntime::from_component(MountedProgress {
        initial,
        density: DensityClass::Touch,
        node: node.clone(),
        fill: fill.clone(),
        error,
    })
    .unwrap();
    let node = node.get().unwrap();
    (runtime, node, fill.get().unwrap())
}

#[test]
fn determinate_mount_reports_bounded_formatted_value_without_actions_or_focus() {
    let (runtime, node, _) = mounted(ProgressValue::Determinate(40.0));
    let semantic = runtime.ui().semantics.get(node).unwrap();
    assert_eq!(semantic.role, SemanticRole::ProgressIndicator);
    assert!(!semantic.state.busy);
    assert!(semantic.actions.is_empty());
    assert!(!semantic.state.focusable);
    assert!(
        runtime
            .ui()
            .interactions
            .get(node)
            .is_none_or(|interaction| !interaction.focusable)
    );
    let SemanticValue::Number {
        current,
        minimum,
        maximum,
        step,
        value_text,
    } = semantic.value
    else {
        panic!("determinate progress must expose a numeric value");
    };
    assert_eq!(
        (current, minimum, maximum, step),
        (40.0, 0.0, 100.0, Some(1.0))
    );
    assert_eq!(runtime.ui().string(value_text.unwrap()), Some("40%"));
}

#[test]
fn indeterminate_mount_reports_busy_without_fabricating_numeric_value() {
    let (runtime, node, _) = mounted(ProgressValue::Indeterminate);
    let semantic = runtime.ui().semantics.get(node).unwrap();
    assert_eq!(semantic.role, SemanticRole::ProgressIndicator);
    assert!(semantic.state.busy);
    assert_eq!(semantic.value, SemanticValue::None);
    assert!(semantic.actions.is_empty());
    assert!(!semantic.state.focusable);
}

#[test]
fn out_of_range_controlled_value_is_rejected_without_a_semantic_node() {
    let node = Rc::new(Cell::new(None));
    let error = Rc::new(RefCell::new(None));
    let runtime = ViewRuntime::from_component(MountedProgress {
        initial: ProgressValue::Determinate(120.0),
        density: DensityClass::Standard,
        node: node.clone(),
        fill: Rc::new(Cell::new(None)),
        error: error.clone(),
    })
    .unwrap();
    assert!(node.get().is_none());
    assert!(
        error
            .borrow()
            .as_deref()
            .unwrap()
            .contains("outside the bounds")
    );
    assert_eq!(runtime.ui().semantics.len(), 0);
}

#[test]
fn determinate_progress_patches_numeric_semantics_and_fill_geometry() {
    let (mut runtime, node, fill) = mounted(ProgressValue::Determinate(40.0));
    let before = *runtime.ui().box_styles.get(fill).unwrap();
    runtime
        .send_component_action(ProgressValue::Determinate(75.0))
        .unwrap();
    let after = *runtime.ui().box_styles.get(fill).unwrap();
    assert_ne!(after, before);
    let SemanticValue::Number { current, .. } = runtime.ui().semantics.get(node).unwrap().value
    else {
        panic!("determinate progress must retain numeric semantics");
    };
    assert_eq!(current, 75.0);
}

#[test]
fn activity_style_resolves_state_density_and_reduced_motion_without_a_clock() {
    let style = ActivityIndicatorStyle::default();
    let compact_running = style.resolve(
        DensityClass::Compact,
        ActivityIndicatorState::Running,
        ActivityMotionPreference::Standard,
    );
    let touch_reduced = style.resolve(
        DensityClass::Touch,
        ActivityIndicatorState::Running,
        ActivityMotionPreference::Reduced,
    );
    let inactive = style.resolve(
        DensityClass::Standard,
        ActivityIndicatorState::Inactive,
        ActivityMotionPreference::Standard,
    );

    assert_eq!(compact_running.visual.indicator_size, 12.0);
    assert_eq!(touch_reduced.visual.indicator_size, 20.0);
    assert_eq!(
        compact_running.visual.motion,
        ActivityMotionStyle::Rotate { cycle_millis: 900 }
    );
    assert_eq!(touch_reduced.visual.motion, ActivityMotionStyle::Static);
    assert_eq!(inactive.visual.motion, ActivityMotionStyle::Static);
    assert_eq!(inactive.visual.marker.opacity, 0.0);
}

struct MountedActivity {
    initial: bool,
    density: DensityClass,
    motion_preference: ActivityMotionPreference,
    node: Rc<Cell<Option<UiNodeId>>>,
    indicator: Rc<Cell<Option<UiNodeId>>>,
    marker: Rc<Cell<Option<UiNodeId>>>,
    resolved: Rc<Cell<Option<ResolvedActivityIndicatorStyle>>>,
}

impl Component for MountedActivity {
    type State = State<bool>;
    type Action = bool;

    fn create(&self, context: &mut CreateContext<'_>) -> Self::State {
        context.state(self.initial)
    }

    fn mount(&self, state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
        let root = ui
            .foundation()
            .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
        assert!(matches!(
            ActivityIndicator::new(" ", state.read()),
            Err(ActivityIndicatorError::MissingAccessibleName)
        ));
        let reference = ActivityIndicator::new("Synchronizing", state.read())
            .unwrap()
            .density(self.density)
            .motion_preference(self.motion_preference)
            .mount(ui, root.0)
            .unwrap();
        self.node.set(Some(reference.node()));
        self.indicator.set(Some(reference.indicator_node()));
        self.marker.set(Some(reference.marker_node()));
        self.resolved.set(Some(reference.resolved_style()));
        root
    }

    fn action(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        context: &mut UpdateContext<'_, Self>,
    ) {
        context.set(*state, action).unwrap();
    }
}

fn mounted_activity(
    initial: bool,
    motion_preference: ActivityMotionPreference,
) -> (
    ViewRuntime<crate::runtime::ComponentRuntimeDriver<MountedActivity>>,
    UiNodeId,
    UiNodeId,
    UiNodeId,
    ResolvedActivityIndicatorStyle,
) {
    let node = Rc::new(Cell::new(None));
    let indicator = Rc::new(Cell::new(None));
    let marker = Rc::new(Cell::new(None));
    let resolved = Rc::new(Cell::new(None));
    let runtime = ViewRuntime::from_component(MountedActivity {
        initial,
        density: DensityClass::Touch,
        motion_preference,
        node: node.clone(),
        indicator: indicator.clone(),
        marker: marker.clone(),
        resolved: resolved.clone(),
    })
    .unwrap();
    (
        runtime,
        node.get().unwrap(),
        indicator.get().unwrap(),
        marker.get().unwrap(),
        resolved.get().unwrap(),
    )
}

#[test]
fn running_activity_mount_is_busy_nonnumeric_noninteractive_and_density_aware() {
    let (runtime, node, indicator, marker, resolved) =
        mounted_activity(true, ActivityMotionPreference::Standard);
    let semantic = runtime.ui().semantics.get(node).unwrap();
    assert_eq!(semantic.role, SemanticRole::ProgressIndicator);
    assert!(semantic.state.busy);
    assert_eq!(semantic.value, SemanticValue::None);
    assert!(semantic.actions.is_empty());
    assert!(!semantic.state.focusable);
    assert!(
        runtime
            .ui()
            .interactions
            .get(node)
            .is_none_or(|interaction| !interaction.focusable)
    );
    assert_eq!(resolved.state, ActivityIndicatorState::Running);
    assert_eq!(resolved.density, DensityClass::Touch);
    assert_eq!(resolved.visual.indicator_size, 20.0);
    assert_eq!(
        runtime.ui().box_styles.get(indicator).unwrap().width,
        SizeRule::Logical(20.0)
    );
    assert_eq!(
        runtime
            .ui()
            .box_styles
            .get(marker)
            .unwrap()
            .transform
            .translation
            .y,
        0.0
    );
}

#[test]
fn activity_state_patches_busy_semantics_and_marker_visual() {
    let (mut runtime, node, _, marker, _) =
        mounted_activity(true, ActivityMotionPreference::Standard);
    let before = *runtime.ui().box_styles.get(marker).unwrap();
    runtime.send_component_action(false).unwrap();
    let after = *runtime.ui().box_styles.get(marker).unwrap();
    assert!(!runtime.ui().semantics.get(node).unwrap().state.busy);
    assert_ne!(after, before);
    assert_eq!(after.opacity, 0.0);
}

#[test]
fn inactive_reduced_motion_mount_is_not_busy_and_has_static_visual_intent() {
    let (runtime, node, _indicator, marker, resolved) =
        mounted_activity(false, ActivityMotionPreference::Reduced);
    let semantic = runtime.ui().semantics.get(node).unwrap();
    assert!(!semantic.state.busy);
    assert_eq!(semantic.value, SemanticValue::None);
    assert!(semantic.actions.is_empty());
    assert_eq!(resolved.state, ActivityIndicatorState::Inactive);
    assert_eq!(
        resolved.motion_preference,
        ActivityMotionPreference::Reduced
    );
    assert_eq!(resolved.visual.motion, ActivityMotionStyle::Static);
    assert_eq!(runtime.ui().box_styles.get(marker).unwrap().opacity, 0.0);
}

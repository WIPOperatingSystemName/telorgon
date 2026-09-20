//! Stable application scroll viewport over a controller-owned metrics snapshot.

use std::fmt;

use crate::foundation::PointF;
use crate::runtime::{RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    BoxStyle, LayoutStyle, MountWriter, Property, ScrollHandle, SemanticAction, SemanticActions,
    SemanticName, SemanticNode, SemanticRelationship, SemanticRelationshipKind, SemanticRole,
    SemanticState, UiNodeId,
};

use super::{ScrollController, ScrollControllerCommand, ScrollInputSource, ScrollMetrics};

/// Logical axis used by forward and backward scroll requests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ScrollViewAxis {
    Horizontal,
    #[default]
    Vertical,
}

/// Directional request exposed by an application scroll view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScrollViewCommand {
    Forward,
    Backward,
}

impl ScrollViewCommand {
    pub const fn from_semantic_action(action: SemanticAction) -> Option<Self> {
        match action {
            SemanticAction::ScrollForward => Some(Self::Forward),
            SemanticAction::ScrollBackward => Some(Self::Backward),
            _ => None,
        }
    }
}

/// Read-only directional behavior over one controller metrics snapshot.
///
/// Requests are typed proposals for the controller. This value never mutates controller state,
/// applies layout, captures input, or schedules motion.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollViewBehavior {
    metrics: ScrollMetrics,
    axis: ScrollViewAxis,
    enabled: bool,
}

impl ScrollViewBehavior {
    pub fn from_controller(
        controller: &ScrollController,
        axis: ScrollViewAxis,
        enabled: bool,
    ) -> Self {
        Self {
            metrics: controller.metrics(),
            axis,
            enabled,
        }
    }

    pub const fn metrics(self) -> ScrollMetrics {
        self.metrics
    }

    pub const fn axis(self) -> ScrollViewAxis {
        self.axis
    }

    pub const fn enabled(self) -> bool {
        self.enabled
    }

    pub fn can_request(self, command: ScrollViewCommand) -> bool {
        if !self.enabled || self.page_extent() <= 0.0 {
            return false;
        }
        match (self.axis, command) {
            (ScrollViewAxis::Horizontal, ScrollViewCommand::Forward) => {
                self.metrics.can_scroll_right()
            }
            (ScrollViewAxis::Horizontal, ScrollViewCommand::Backward) => {
                self.metrics.can_scroll_left()
            }
            (ScrollViewAxis::Vertical, ScrollViewCommand::Forward) => {
                self.metrics.can_scroll_down()
            }
            (ScrollViewAxis::Vertical, ScrollViewCommand::Backward) => self.metrics.can_scroll_up(),
        }
    }

    pub fn is_scrollable(self) -> bool {
        self.can_request(ScrollViewCommand::Forward)
            || self.can_request(ScrollViewCommand::Backward)
    }

    /// Returns one viewport-sized semantic scroll proposal when that direction is applicable.
    pub fn request(self, command: ScrollViewCommand) -> Option<ScrollControllerCommand> {
        self.can_request(command).then(|| {
            let distance = match command {
                ScrollViewCommand::Forward => self.page_extent(),
                ScrollViewCommand::Backward => -self.page_extent(),
            };
            let delta = match self.axis {
                ScrollViewAxis::Horizontal => PointF {
                    x: distance,
                    y: 0.0,
                },
                ScrollViewAxis::Vertical => PointF {
                    x: 0.0,
                    y: distance,
                },
            };
            ScrollControllerCommand::ScrollBy {
                delta,
                source: ScrollInputSource::Semantic,
            }
        })
    }

    pub fn semantic_request(self, action: SemanticAction) -> Option<ScrollControllerCommand> {
        ScrollViewCommand::from_semantic_action(action).and_then(|command| self.request(command))
    }

    pub fn semantic_actions(self) -> SemanticActions {
        let mut actions = SemanticActions::NONE;
        if self.is_scrollable() {
            actions |= SemanticActions::FOCUS;
        }
        if self.can_request(ScrollViewCommand::Forward) {
            actions |= SemanticActions::SCROLL_FORWARD;
        }
        if self.can_request(ScrollViewCommand::Backward) {
            actions |= SemanticActions::SCROLL_BACKWARD;
        }
        actions
    }

    fn page_extent(self) -> f32 {
        match self.axis {
            ScrollViewAxis::Horizontal => self.metrics.viewport.width,
            ScrollViewAxis::Vertical => self.metrics.viewport.height,
        }
    }
}

/// Caller-owned visual and layout inputs for a scroll viewport and its content owner.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollViewStyle {
    pub viewport: BoxStyle,
    pub viewport_layout: LayoutStyle,
    pub content: BoxStyle,
    pub content_layout: LayoutStyle,
}

/// Mounted application scroll view over a mount-time controller snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct ScrollView {
    label: String,
    snapshot: ScrollMetrics,
    axis: ScrollViewAxis,
    enabled: bool,
    style: ScrollViewStyle,
}

impl ScrollView {
    pub fn new(
        label: impl Into<String>,
        controller: &ScrollController,
    ) -> Result<Self, ScrollViewError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(ScrollViewError::MissingAccessibleName);
        }
        Ok(Self {
            label,
            snapshot: controller.metrics(),
            axis: ScrollViewAxis::Vertical,
            enabled: true,
            style: ScrollViewStyle::default(),
        })
    }

    pub const fn axis(mut self, axis: ScrollViewAxis) -> Self {
        self.axis = axis;
        self
    }

    pub const fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub const fn style(mut self, style: ScrollViewStyle) -> Self {
        self.style = style;
        self
    }

    pub const fn snapshot(&self) -> ScrollMetrics {
        self.snapshot
    }

    pub const fn behavior(&self) -> ScrollViewBehavior {
        ScrollViewBehavior {
            metrics: self.snapshot,
            axis: self.axis,
            enabled: self.enabled,
        }
    }

    pub fn mount<'storage, Action, Content>(
        &self,
        ui: &mut Ui<'_, 'storage, Action>,
        host: UiNodeId,
        content: Content,
    ) -> RuntimeResult<ScrollViewRef>
    where
        Action: 'static,
        Content: FnOnce(&mut MountWriter<'storage, Action>),
    {
        let behavior = self.behavior();
        let viewport_layout = LayoutStyle {
            scroll_offset: self.snapshot.offset,
            ..self.style.viewport_layout
        };
        let mut content_owner = None;
        let viewport = ui
            .foundation()
            .scroll_node_under(
                host,
                self.style.viewport,
                viewport_layout,
                self.enabled,
                behavior.is_scrollable(),
                |writer| {
                    content_owner = Some(writer.container(
                        self.style.content,
                        self.style.content_layout,
                        content,
                    ));
                },
            )
            .ok_or_else(|| RuntimeError::new("application scroll-view parent is stale"))?;
        let content_owner = content_owner.expect("scroll view mounts its content owner");

        ui.foundation()
            .semantic_node(content_owner, SemanticNode::default())
            .map_err(semantic_runtime_error)?;
        let name = ui.foundation().intern(&self.label);
        ui.foundation()
            .semantic_node(
                viewport.node,
                SemanticNode {
                    role: SemanticRole::Region,
                    name: SemanticName::Text(name),
                    state: SemanticState {
                        disabled: !self.enabled,
                        focusable: behavior.is_scrollable(),
                        ..SemanticState::default()
                    },
                    actions: behavior.semantic_actions(),
                    relationships: vec![SemanticRelationship {
                        kind: SemanticRelationshipKind::Owns,
                        target: content_owner,
                    }],
                    ..SemanticNode::default()
                },
            )
            .map_err(semantic_runtime_error)?;
        if !self.enabled {
            ui.foundation().disabled(viewport.node, true);
        }

        Ok(ScrollViewRef {
            viewport,
            content_owner,
            behavior,
        })
    }
}

fn semantic_runtime_error(error: crate::ui::SemanticError) -> RuntimeError {
    RuntimeError::new(format!("invalid scroll-view semantics: {error:?}"))
}

/// Stable mounted identities and directional proposals for one scroll view.
#[derive(Clone, Copy, Debug)]
pub struct ScrollViewRef {
    viewport: ScrollHandle,
    content_owner: UiNodeId,
    behavior: ScrollViewBehavior,
}

impl ScrollViewRef {
    pub const fn node(self) -> UiNodeId {
        self.viewport.node
    }

    pub const fn content_node(self) -> UiNodeId {
        self.content_owner
    }

    pub const fn offset(self) -> Property<PointF> {
        self.viewport.offset
    }

    pub const fn style(self) -> Property<BoxStyle> {
        self.viewport.style
    }

    pub const fn snapshot(self) -> ScrollMetrics {
        self.behavior.metrics()
    }

    pub fn request(self, command: ScrollViewCommand) -> Option<ScrollControllerCommand> {
        self.behavior.request(command)
    }

    pub fn semantic_request(self, action: SemanticAction) -> Option<ScrollControllerCommand> {
        self.behavior.semantic_request(action)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollViewError {
    MissingAccessibleName,
}

impl fmt::Display for ScrollViewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid scroll view: {self:?}")
    }
}

impl std::error::Error for ScrollViewError {}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use crate::foundation::{ColorRgba8, SizeF};
    use crate::runtime::{Component, CreateContext, UpdateContext, ViewRuntime};
    use crate::ui::{Background, Flow, NodeKind, Overflow, SemanticAction, SizeRule, UiRoot};

    use super::*;

    fn size(width: f32, height: f32) -> SizeF {
        SizeF { width, height }
    }

    fn vertical_controller(offset: f32) -> ScrollController {
        let mut controller = ScrollController::new(size(100.0, 100.0), size(100.0, 500.0)).unwrap();
        controller
            .route(ScrollControllerCommand::ScrollTo {
                offset: PointF { x: 0.0, y: offset },
                source: ScrollInputSource::Programmatic,
            })
            .unwrap();
        controller
    }

    #[test]
    fn directional_behavior_returns_only_applicable_controller_commands() {
        let controller = vertical_controller(150.0);
        let behavior =
            ScrollViewBehavior::from_controller(&controller, ScrollViewAxis::Vertical, true);
        assert_eq!(
            behavior.request(ScrollViewCommand::Forward),
            Some(ScrollControllerCommand::ScrollBy {
                delta: PointF { x: 0.0, y: 100.0 },
                source: ScrollInputSource::Semantic,
            })
        );
        assert_eq!(
            behavior.semantic_request(SemanticAction::ScrollBackward),
            Some(ScrollControllerCommand::ScrollBy {
                delta: PointF { x: 0.0, y: -100.0 },
                source: ScrollInputSource::Semantic,
            })
        );
        assert!(
            behavior
                .semantic_actions()
                .contains(SemanticAction::ScrollForward)
        );
        assert!(
            behavior
                .semantic_actions()
                .contains(SemanticAction::ScrollBackward)
        );
        assert_eq!(behavior.semantic_request(SemanticAction::Activate), None);

        let at_end = vertical_controller(400.0);
        let behavior = ScrollViewBehavior::from_controller(&at_end, ScrollViewAxis::Vertical, true);
        assert_eq!(behavior.request(ScrollViewCommand::Forward), None);
        assert!(behavior.request(ScrollViewCommand::Backward).is_some());
        assert!(
            !behavior
                .semantic_actions()
                .contains(SemanticAction::ScrollForward)
        );
    }

    #[test]
    fn horizontal_and_disabled_snapshots_preserve_axis_and_availability() {
        let controller = ScrollController::new(size(80.0, 40.0), size(300.0, 40.0)).unwrap();
        let horizontal =
            ScrollViewBehavior::from_controller(&controller, ScrollViewAxis::Horizontal, true);
        assert_eq!(
            horizontal.request(ScrollViewCommand::Forward),
            Some(ScrollControllerCommand::ScrollBy {
                delta: PointF { x: 80.0, y: 0.0 },
                source: ScrollInputSource::Semantic,
            })
        );
        assert_eq!(horizontal.request(ScrollViewCommand::Backward), None);

        let disabled =
            ScrollViewBehavior::from_controller(&controller, ScrollViewAxis::Horizontal, false);
        assert!(!disabled.is_scrollable());
        assert!(disabled.semantic_actions().is_empty());
        assert_eq!(disabled.request(ScrollViewCommand::Forward), None);
    }

    #[test]
    fn rejects_a_missing_viewport_name() {
        let controller = vertical_controller(0.0);
        assert_eq!(
            ScrollView::new("  ", &controller),
            Err(ScrollViewError::MissingAccessibleName)
        );
    }

    struct Fixture {
        controller: ScrollController,
        reference: Rc<RefCell<Option<ScrollViewRef>>>,
        content_calls: Rc<Cell<usize>>,
    }

    impl Component for Fixture {
        type State = ();
        type Action = ();

        fn create(&self, _: &mut CreateContext<'_>) -> Self::State {}

        fn mount(&self, _: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            let style = ScrollViewStyle {
                viewport: BoxStyle {
                    width: SizeRule::Logical(320.0),
                    height: SizeRule::Logical(180.0),
                    overflow: Overflow::Clip,
                    ..BoxStyle::default()
                },
                viewport_layout: LayoutStyle {
                    flow: Flow::Overlay,
                    gap: 3.0,
                    contain: true,
                    scroll_offset: PointF { x: 99.0, y: 99.0 },
                    ..LayoutStyle::default()
                },
                content: BoxStyle {
                    decoration: crate::ui::BoxDecoration {
                        background: Background::Color(ColorRgba8::rgba(12, 24, 36, 255)),
                        ..crate::ui::BoxDecoration::default()
                    },
                    ..BoxStyle::default()
                },
                content_layout: LayoutStyle {
                    flow: Flow::Horizontal,
                    gap: 7.0,
                    contain: false,
                    scroll_offset: PointF { x: 2.0, y: 3.0 },
                    ..LayoutStyle::default()
                },
            };
            let reference = ScrollView::new("Document viewport", &self.controller)
                .unwrap()
                .style(style)
                .mount(ui, root.0, |writer| {
                    self.content_calls.set(self.content_calls.get() + 1);
                    writer.text("Document", ColorRgba8::rgba(255, 255, 255, 255), 12.0);
                })
                .unwrap();
            *self.reference.borrow_mut() = Some(reference);
            root
        }

        fn action(&self, _: &mut Self::State, _: Self::Action, _: &mut UpdateContext<'_, Self>) {}
    }

    #[test]
    fn mounted_viewport_and_content_keep_identity_offset_semantics_and_caller_layout() {
        let reference = Rc::new(RefCell::new(None));
        let content_calls = Rc::new(Cell::new(0));
        let runtime = ViewRuntime::from_component(Fixture {
            controller: vertical_controller(80.0),
            reference: reference.clone(),
            content_calls: content_calls.clone(),
        })
        .unwrap();
        assert_eq!(content_calls.get(), 1);

        let reference = reference.borrow().expect("scroll view reference");
        let viewport = reference.node();
        let content = reference.content_node();
        assert_eq!(runtime.ui().kinds.get(viewport), Some(&NodeKind::Scroll));
        assert_eq!(
            runtime.ui().nodes.core(content).unwrap().parent,
            Some(viewport)
        );
        assert_eq!(
            runtime.ui().layouts.get(viewport).unwrap().scroll_offset.y,
            80.0
        );
        assert_eq!(
            runtime.ui().layouts.get(viewport).unwrap().flow,
            Flow::Overlay
        );
        assert_eq!(runtime.ui().layouts.get(viewport).unwrap().gap, 3.0);
        assert_eq!(
            runtime.ui().layouts.get(content).unwrap(),
            &LayoutStyle {
                flow: Flow::Horizontal,
                gap: 7.0,
                contain: false,
                scroll_offset: PointF { x: 2.0, y: 3.0 },
                ..LayoutStyle::default()
            }
        );
        assert_eq!(
            runtime.ui().box_styles.get(viewport).unwrap().width,
            SizeRule::Logical(320.0)
        );
        assert_eq!(
            runtime
                .ui()
                .box_styles
                .get(content)
                .unwrap()
                .decoration
                .background,
            Background::Color(ColorRgba8::rgba(12, 24, 36, 255))
        );

        let interaction = runtime.ui().interactions.get(viewport).unwrap();
        assert!(interaction.enabled);
        assert!(interaction.focusable);
        let semantics = runtime.ui().semantics.get(viewport).unwrap();
        assert_eq!(semantics.role, SemanticRole::Region);
        assert!(semantics.state.focusable);
        assert!(semantics.actions.contains(SemanticAction::Focus));
        assert!(semantics.actions.contains(SemanticAction::ScrollForward));
        assert!(semantics.actions.contains(SemanticAction::ScrollBackward));
        assert_eq!(semantics.relationships.len(), 1);
        assert_eq!(semantics.relationships[0].target, content);
        assert_eq!(
            semantics.relationships[0].kind,
            SemanticRelationshipKind::Owns
        );
        assert_eq!(reference.snapshot().offset, PointF { x: 0.0, y: 80.0 });
        assert_eq!(
            reference.semantic_request(SemanticAction::ScrollForward),
            Some(ScrollControllerCommand::ScrollBy {
                delta: PointF { x: 0.0, y: 100.0 },
                source: ScrollInputSource::Semantic,
            })
        );
    }
}

//! Controlled two-pane split layout with a source-neutral resizable divider.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use crate::foundation::ColorRgba8;
use crate::input::{ChangeSource, GestureArenaRequest, GestureInput, WritingDirection};
use crate::runtime::{Read, RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    Background, BoxStyle, ControlHandle, CornerRadii, Flow, LayoutStyle, MountWriter, Property,
    SemanticActions, SemanticName, SemanticNode, SemanticRelationship, SemanticRelationshipKind,
    SemanticRole, SemanticState, SemanticValue, SizeRule, SizeRule2D, UiNodeId,
};

use crate::components::application::{
    ChangePhase, DensityMetrics, RangeModel, RangeModelError, SliderBehavior, SliderCommand,
    SliderError, SliderOrientation, SliderPointerOutcome, SliderTrackGeometry,
};

/// Stable panes owned by a split view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SplitViewPane {
    Primary,
    Secondary,
}

/// Axis along which the two panes are arranged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SplitViewOrientation {
    #[default]
    Horizontal,
    Vertical,
}

impl SplitViewOrientation {
    const fn flow(self) -> Flow {
        match self {
            Self::Horizontal => Flow::Horizontal,
            Self::Vertical => Flow::Vertical,
        }
    }

    const fn slider_orientation(self) -> SliderOrientation {
        match self {
            Self::Horizontal => SliderOrientation::Horizontal,
            Self::Vertical => SliderOrientation::Vertical,
        }
    }

    const fn slider_reversed(self) -> bool {
        matches!(self, Self::Vertical)
    }
}

/// Which pane, if any, the divider's collapse action controls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SplitViewCollapsePolicy {
    #[default]
    Disabled,
    Primary,
    Secondary,
}

impl SplitViewCollapsePolicy {
    pub const fn pane(self) -> Option<SplitViewPane> {
        match self {
            Self::Disabled => None,
            Self::Primary => Some(SplitViewPane::Primary),
            Self::Secondary => Some(SplitViewPane::Secondary),
        }
    }

    fn allows(self, pane: SplitViewPane) -> bool {
        self.pane() == Some(pane)
    }
}

/// Atomic parent-owned split state.
///
/// `divider` remains the last expanded position while a pane is collapsed, making restore a pure
/// controlled proposal rather than component-owned history.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitViewValue {
    divider: f32,
    collapsed: Option<SplitViewPane>,
}

impl SplitViewValue {
    pub const fn expanded(divider: f32) -> Self {
        Self {
            divider,
            collapsed: None,
        }
    }

    pub const fn collapsed(divider: f32, pane: SplitViewPane) -> Self {
        Self {
            divider,
            collapsed: Some(pane),
        }
    }

    pub const fn divider(self) -> f32 {
        self.divider
    }

    pub const fn collapsed_pane(self) -> Option<SplitViewPane> {
        self.collapsed
    }

    pub const fn is_expanded(self) -> bool {
        self.collapsed.is_none()
    }
}

/// Validated pane extent, minimum-size, and resize-step policy.
#[derive(Clone, Debug, PartialEq)]
pub struct SplitViewConstraints {
    pane_extent: f32,
    primary_minimum: f32,
    secondary_minimum: f32,
    divider_model: RangeModel<f32>,
}

impl SplitViewConstraints {
    pub fn new(
        pane_extent: f32,
        primary_minimum: f32,
        secondary_minimum: f32,
        step: f32,
        page_step: f32,
    ) -> Result<Self, SplitViewError> {
        if !pane_extent.is_finite() || pane_extent <= 0.0 {
            return Err(SplitViewError::InvalidPaneExtent);
        }
        if !primary_minimum.is_finite() || primary_minimum < 0.0 {
            return Err(SplitViewError::InvalidPrimaryMinimum);
        }
        if !secondary_minimum.is_finite() || secondary_minimum < 0.0 {
            return Err(SplitViewError::InvalidSecondaryMinimum);
        }
        let maximum = pane_extent - secondary_minimum;
        if primary_minimum >= maximum {
            return Err(SplitViewError::InsufficientResizableExtent);
        }
        let divider_model = RangeModel::new(primary_minimum, maximum, step, page_step)?;
        Ok(Self {
            pane_extent,
            primary_minimum,
            secondary_minimum,
            divider_model,
        })
    }

    pub const fn pane_extent(&self) -> f32 {
        self.pane_extent
    }

    pub const fn primary_minimum(&self) -> f32 {
        self.primary_minimum
    }

    pub const fn secondary_minimum(&self) -> f32 {
        self.secondary_minimum
    }

    pub const fn divider_model(&self) -> &RangeModel<f32> {
        &self.divider_model
    }

    pub fn effective_extents(&self, value: SplitViewValue) -> Result<(f32, f32), SplitViewError> {
        self.divider_model.format_value(value.divider)?;
        Ok(match value.collapsed {
            None => (value.divider, self.pane_extent - value.divider),
            Some(SplitViewPane::Primary) => (0.0, self.pane_extent),
            Some(SplitViewPane::Secondary) => (self.pane_extent, 0.0),
        })
    }
}

/// Keyboard, semantic, and programmatic divider requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SplitViewCommand {
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    PageUp,
    PageDown,
    Home,
    End,
    Increment,
    Decrement,
    Collapse,
    Restore,
}

impl SplitViewCommand {
    const fn slider_command(self) -> Option<SliderCommand> {
        Some(match self {
            Self::ArrowLeft => SliderCommand::ArrowLeft,
            Self::ArrowRight => SliderCommand::ArrowRight,
            Self::ArrowUp => SliderCommand::ArrowUp,
            Self::ArrowDown => SliderCommand::ArrowDown,
            Self::PageUp => SliderCommand::PageUp,
            Self::PageDown => SliderCommand::PageDown,
            Self::Home => SliderCommand::Home,
            Self::End => SliderCommand::End,
            Self::Increment => SliderCommand::Increment,
            Self::Decrement => SliderCommand::Decrement,
            Self::Collapse | Self::Restore => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SplitViewOperation {
    Resize,
    Collapse(SplitViewPane),
    Restore(SplitViewPane),
}

/// A non-mutating split-state proposal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitViewProposal {
    value: SplitViewValue,
    operation: SplitViewOperation,
    phase: ChangePhase,
    source: ChangeSource,
}

impl SplitViewProposal {
    pub const fn value(self) -> SplitViewValue {
        self.value
    }

    pub const fn operation(self) -> SplitViewOperation {
        self.operation
    }

    pub const fn phase(self) -> ChangePhase {
        self.phase
    }

    pub const fn source(self) -> ChangeSource {
        self.source
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SplitViewPointerOutcome {
    pub proposal: Option<SplitViewProposal>,
    pub arena: GestureArenaRequest,
}

/// Portable behavior composed over the shared one-thumb slider owner.
#[derive(Clone, Debug)]
pub struct SplitViewBehavior {
    constraints: SplitViewConstraints,
    collapse: SplitViewCollapsePolicy,
    enabled: bool,
    slider: SliderBehavior<f32>,
}

impl SplitViewBehavior {
    pub fn new(
        constraints: SplitViewConstraints,
        collapse: SplitViewCollapsePolicy,
        orientation: SplitViewOrientation,
        enabled: bool,
    ) -> Result<Self, SplitViewError> {
        let slider = SliderBehavior::new(
            constraints.divider_model.clone(),
            orientation.slider_orientation(),
            WritingDirection::LeftToRight,
            orientation.slider_reversed(),
            enabled,
        )?;
        Ok(Self {
            constraints,
            collapse,
            enabled,
            slider,
        })
    }

    pub const fn constraints(&self) -> &SplitViewConstraints {
        &self.constraints
    }

    pub const fn collapse_policy(&self) -> SplitViewCollapsePolicy {
        self.collapse
    }

    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn validate_value(&self, value: SplitViewValue) -> Result<(), SplitViewError> {
        self.constraints.divider_model.format_value(value.divider)?;
        if let Some(pane) = value.collapsed
            && !self.collapse.allows(pane)
        {
            return Err(SplitViewError::CollapsedPaneNotAllowed(pane));
        }
        Ok(())
    }

    pub fn request(
        &self,
        current: SplitViewValue,
        command: SplitViewCommand,
        source: ChangeSource,
    ) -> Result<Option<SplitViewProposal>, SplitViewError> {
        self.validate_value(current)?;
        if !self.enabled {
            return Ok(None);
        }
        match command {
            SplitViewCommand::Collapse => {
                let Some(pane) = self.collapse.pane() else {
                    return Ok(None);
                };
                if current.collapsed == Some(pane) {
                    return Ok(None);
                }
                Ok(Some(SplitViewProposal {
                    value: SplitViewValue::collapsed(current.divider, pane),
                    operation: SplitViewOperation::Collapse(pane),
                    phase: ChangePhase::Commit,
                    source,
                }))
            }
            SplitViewCommand::Restore => {
                let Some(pane) = current.collapsed else {
                    return Ok(None);
                };
                Ok(Some(SplitViewProposal {
                    value: SplitViewValue::expanded(current.divider),
                    operation: SplitViewOperation::Restore(pane),
                    phase: ChangePhase::Commit,
                    source,
                }))
            }
            resize => {
                if !current.is_expanded() {
                    return Ok(None);
                }
                let command = resize
                    .slider_command()
                    .expect("resize command has a slider equivalent");
                Ok(self
                    .slider
                    .request(current.divider, command, source)?
                    .map(|change| SplitViewProposal {
                        value: SplitViewValue::expanded(change.value),
                        operation: SplitViewOperation::Resize,
                        phase: change.phase,
                        source: change.source,
                    }))
            }
        }
    }

    pub fn propose_resize(
        &self,
        current: SplitViewValue,
        target: f32,
        phase: ChangePhase,
        source: ChangeSource,
    ) -> Result<SplitViewProposal, SplitViewError> {
        self.validate_value(current)?;
        if !current.is_expanded() {
            return Err(SplitViewError::CannotResizeCollapsed);
        }
        let divider = self.constraints.divider_model.normalize(target)?;
        Ok(SplitViewProposal {
            value: SplitViewValue::expanded(divider),
            operation: SplitViewOperation::Resize,
            phase,
            source,
        })
    }

    pub fn handle_pointer(
        &mut self,
        current: SplitViewValue,
        input: GestureInput,
        track: SliderTrackGeometry,
    ) -> Result<SplitViewPointerOutcome, SplitViewError> {
        self.validate_value(current)?;
        let interaction_enabled = self.enabled && current.is_expanded();
        let mut synchronized_arena = GestureArenaRequest::None;
        if self.slider.enabled() != interaction_enabled {
            synchronized_arena = self
                .slider
                .handle_pointer(
                    current.divider,
                    GestureInput::SetEnabled(interaction_enabled),
                    track,
                )?
                .arena;
        }
        if !interaction_enabled {
            return Ok(SplitViewPointerOutcome {
                proposal: None,
                arena: synchronized_arena,
            });
        }
        let SliderPointerOutcome { change, arena } =
            self.slider.handle_pointer(current.divider, input, track)?;
        Ok(SplitViewPointerOutcome {
            proposal: change.map(|change| SplitViewProposal {
                value: SplitViewValue::expanded(change.value),
                operation: SplitViewOperation::Resize,
                phase: change.phase,
                source: change.source,
            }),
            arena,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitViewStyle {
    pub container: BoxStyle,
    pub pane: BoxStyle,
    pub divider: BoxStyle,
    pub divider_grip: BoxStyle,
    pub pane_layout: LayoutStyle,
}

impl Default for SplitViewStyle {
    fn default() -> Self {
        Self {
            container: BoxStyle::default(),
            pane: BoxStyle::default(),
            divider: BoxStyle::default(),
            divider_grip: BoxStyle {
                decoration: crate::ui::BoxDecoration {
                    background: Background::Color(ColorRgba8::rgba(104, 116, 139, 255)),
                    corner_radii: CornerRadii::all(2.0),
                    ..crate::ui::BoxDecoration::default()
                },
                ..BoxStyle::default()
            },
            pane_layout: LayoutStyle::default(),
        }
    }
}

/// Mounted controlled split view. Caller content is mounted once under each stable pane owner.
#[derive(Clone, Debug, PartialEq)]
pub struct SplitView {
    label: String,
    primary_label: String,
    secondary_label: String,
    divider_label: String,
    value: Read<SplitViewValue>,
    constraints: SplitViewConstraints,
    collapse: SplitViewCollapsePolicy,
    orientation: SplitViewOrientation,
    enabled: bool,
    density: DensityMetrics,
    style: SplitViewStyle,
}

impl SplitView {
    pub fn new(
        label: impl Into<String>,
        primary_label: impl Into<String>,
        secondary_label: impl Into<String>,
        divider_label: impl Into<String>,
        value: Read<SplitViewValue>,
        constraints: SplitViewConstraints,
    ) -> Result<Self, SplitViewError> {
        let label = label.into();
        let primary_label = primary_label.into();
        let secondary_label = secondary_label.into();
        let divider_label = divider_label.into();
        for (name, text) in [
            (SplitViewName::Group, label.as_str()),
            (SplitViewName::PrimaryPane, primary_label.as_str()),
            (SplitViewName::SecondaryPane, secondary_label.as_str()),
            (SplitViewName::Divider, divider_label.as_str()),
        ] {
            if text.trim().is_empty() {
                return Err(SplitViewError::MissingAccessibleName(name));
            }
        }
        if primary_label == secondary_label {
            return Err(SplitViewError::DuplicatePaneNames);
        }
        Ok(Self {
            label,
            primary_label,
            secondary_label,
            divider_label,
            value,
            constraints,
            collapse: SplitViewCollapsePolicy::Disabled,
            orientation: SplitViewOrientation::Horizontal,
            enabled: true,
            density: DensityMetrics::baseline(
                crate::components::application::DensityClass::Standard,
            ),
            style: SplitViewStyle::default(),
        })
    }

    pub const fn collapse_policy(mut self, collapse: SplitViewCollapsePolicy) -> Self {
        self.collapse = collapse;
        self
    }

    pub const fn orientation(mut self, orientation: SplitViewOrientation) -> Self {
        self.orientation = orientation;
        self
    }

    pub const fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub const fn density(mut self, density: DensityMetrics) -> Self {
        self.density = density;
        self
    }

    pub const fn style(mut self, style: SplitViewStyle) -> Self {
        self.style = style;
        self
    }

    pub fn behavior(&self) -> Result<SplitViewBehavior, SplitViewError> {
        SplitViewBehavior::new(
            self.constraints.clone(),
            self.collapse,
            self.orientation,
            self.enabled,
        )
    }

    pub fn mount<'storage, Action, Content>(
        &self,
        ui: &mut Ui<'_, 'storage, Action>,
        host: UiNodeId,
        mut content: Content,
    ) -> RuntimeResult<SplitViewRef>
    where
        Action: 'static,
        Content: FnMut(SplitViewPane, &mut MountWriter<'storage, Action>),
    {
        let value = ui.read(self.value)?;
        let behavior = self
            .behavior()
            .map_err(|error| RuntimeError::new(format!("invalid split-view behavior: {error}")))?;
        behavior.validate_value(value).map_err(|error| {
            RuntimeError::new(format!("invalid controlled split view: {error}"))
        })?;
        let (primary_extent, secondary_extent) = self
            .constraints
            .effective_extents(value)
            .map_err(|error| RuntimeError::new(format!("invalid split-view extents: {error}")))?;
        let styles = resolve_styles(
            self.style,
            self.orientation,
            self.density,
            primary_extent,
            secondary_extent,
        );
        let primary_visible = value.collapsed != Some(SplitViewPane::Primary);
        let secondary_visible = value.collapsed != Some(SplitViewPane::Secondary);
        let mut primary = None;
        let mut divider = None;
        let mut secondary = None;
        let root = ui
            .foundation()
            .container_node_under(
                host,
                styles.container,
                LayoutStyle {
                    flow: self.orientation.flow(),
                    ..LayoutStyle::default()
                },
                |writer| {
                    primary = Some(writer.layer(
                        primary_visible,
                        styles.primary,
                        self.style.pane_layout,
                        |writer| content(SplitViewPane::Primary, writer),
                    ));
                    divider = Some(writer.action_node(styles.divider, self.enabled, |writer| {
                        writer.container(styles.divider_grip, LayoutStyle::default(), |_| {});
                    }));
                    secondary = Some(writer.layer(
                        secondary_visible,
                        styles.secondary,
                        self.style.pane_layout,
                        |writer| content(SplitViewPane::Secondary, writer),
                    ));
                },
            )
            .ok_or_else(|| RuntimeError::new("application split-view parent is stale"))?;
        let primary = SplitViewPaneRef {
            pane: SplitViewPane::Primary,
            control: primary.expect("split view mounts its primary pane"),
        };
        let divider = divider.expect("split view mounts its divider");
        let secondary = SplitViewPaneRef {
            pane: SplitViewPane::Secondary,
            control: secondary.expect("split view mounts its secondary pane"),
        };

        self.mount_pane_semantics(ui, &primary, primary_visible, &self.primary_label)?;
        self.mount_pane_semantics(ui, &secondary, secondary_visible, &self.secondary_label)?;
        let divider_name = ui.foundation().intern(&self.divider_label);
        let value_text = value.is_expanded().then(|| {
            ui.foundation()
                .intern(format!("{} logical pixels", value.divider))
        });
        ui.foundation()
            .semantic_node(
                divider.node,
                self.divider_semantics(divider_name, value_text, value),
            )
            .map_err(semantic_runtime_error)?;
        if !self.enabled {
            ui.foundation().disabled(divider.node, true);
        }

        let name = ui.foundation().intern(&self.label);
        ui.foundation()
            .semantic_node(
                root.node,
                SemanticNode {
                    role: SemanticRole::Generic,
                    name: SemanticName::Text(name),
                    relationships: [primary.node(), divider.node, secondary.node()]
                        .into_iter()
                        .map(|target| SemanticRelationship {
                            kind: SemanticRelationshipKind::Owns,
                            target,
                        })
                        .collect(),
                    ..SemanticNode::default()
                },
            )
            .map_err(semantic_runtime_error)?;

        Ok(SplitViewRef {
            root,
            primary,
            divider,
            secondary,
            value: self.value,
            behavior: Rc::new(RefCell::new(behavior)),
        })
    }

    fn mount_pane_semantics<Action>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        pane: &SplitViewPaneRef,
        visible: bool,
        label: &str,
    ) -> RuntimeResult<()> {
        let name = ui.foundation().intern(label);
        ui.foundation()
            .semantic_node(
                pane.node(),
                SemanticNode {
                    role: SemanticRole::Region,
                    name: SemanticName::Text(name),
                    state: SemanticState {
                        inert: !visible,
                        hidden: !visible,
                        ..SemanticState::default()
                    },
                    ..SemanticNode::default()
                },
            )
            .map_err(semantic_runtime_error)?;
        Ok(())
    }

    fn divider_semantics(
        &self,
        name: crate::ui::StringId,
        value_text: Option<crate::ui::StringId>,
        value: SplitViewValue,
    ) -> SemanticNode {
        let mut actions = SemanticActions::NONE;
        if self.enabled {
            actions |= SemanticActions::FOCUS;
            if value.is_expanded() {
                actions |= SemanticActions::INCREMENT
                    | SemanticActions::DECREMENT
                    | SemanticActions::SET_VALUE;
                if self.collapse.pane().is_some() {
                    actions |= SemanticActions::COLLAPSE;
                }
            } else {
                actions |= SemanticActions::EXPAND;
            }
        }
        let semantic_value = if value.is_expanded() {
            SemanticValue::Number {
                current: f64::from(value.divider),
                minimum: f64::from(self.constraints.divider_model.minimum()),
                maximum: f64::from(self.constraints.divider_model.maximum()),
                step: Some(f64::from(self.constraints.divider_model.step())),
                value_text,
            }
        } else {
            SemanticValue::None
        };
        SemanticNode {
            role: SemanticRole::Separator,
            name: SemanticName::Text(name),
            state: SemanticState {
                disabled: !self.enabled,
                focusable: self.enabled,
                expanded: (self.collapse.pane().is_some()).then_some(value.is_expanded()),
                ..SemanticState::default()
            },
            value: semantic_value,
            actions,
            ..SemanticNode::default()
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ResolvedSplitViewStyles {
    container: BoxStyle,
    primary: BoxStyle,
    secondary: BoxStyle,
    divider: BoxStyle,
    divider_grip: BoxStyle,
}

fn resolve_styles(
    style: SplitViewStyle,
    orientation: SplitViewOrientation,
    density: DensityMetrics,
    primary_extent: f32,
    secondary_extent: f32,
) -> ResolvedSplitViewStyles {
    let mut primary = style.pane;
    let mut secondary = style.pane;
    let mut divider = style.divider;
    let mut divider_grip = style.divider_grip;
    let target = density.effective_minimum();
    divider.min_size = SizeRule2D {
        width: SizeRule::Logical(target.width()),
        height: SizeRule::Logical(target.height()),
    };
    match orientation {
        SplitViewOrientation::Horizontal => {
            primary.width = SizeRule::Logical(primary_extent);
            primary.height = SizeRule::Fill(1.0);
            secondary.width = SizeRule::Logical(secondary_extent);
            secondary.height = SizeRule::Fill(1.0);
            divider.height = SizeRule::Fill(1.0);
            divider_grip.width = SizeRule::Logical(4.0);
            divider_grip.height = SizeRule::Logical(24.0);
        }
        SplitViewOrientation::Vertical => {
            primary.width = SizeRule::Fill(1.0);
            primary.height = SizeRule::Logical(primary_extent);
            secondary.width = SizeRule::Fill(1.0);
            secondary.height = SizeRule::Logical(secondary_extent);
            divider.width = SizeRule::Fill(1.0);
            divider_grip.width = SizeRule::Logical(24.0);
            divider_grip.height = SizeRule::Logical(4.0);
        }
    }
    ResolvedSplitViewStyles {
        container: style.container,
        primary,
        secondary,
        divider,
        divider_grip,
    }
}

fn semantic_runtime_error(error: crate::ui::SemanticError) -> RuntimeError {
    RuntimeError::new(format!("invalid split-view semantics: {error:?}"))
}

#[derive(Clone, Debug)]
pub struct SplitViewRef {
    root: ControlHandle,
    primary: SplitViewPaneRef,
    divider: ControlHandle,
    secondary: SplitViewPaneRef,
    value: Read<SplitViewValue>,
    behavior: Rc<RefCell<SplitViewBehavior>>,
}

impl SplitViewRef {
    pub const fn node(&self) -> UiNodeId {
        self.root.node
    }

    pub const fn style(&self) -> Property<BoxStyle> {
        self.root.style
    }

    pub const fn value(&self) -> Read<SplitViewValue> {
        self.value
    }

    pub const fn divider_node(&self) -> UiNodeId {
        self.divider.node
    }

    pub const fn divider_style(&self) -> Property<BoxStyle> {
        self.divider.style
    }

    pub const fn pane(&self, pane: SplitViewPane) -> &SplitViewPaneRef {
        match pane {
            SplitViewPane::Primary => &self.primary,
            SplitViewPane::Secondary => &self.secondary,
        }
    }

    pub fn request(
        &self,
        current: SplitViewValue,
        command: SplitViewCommand,
        source: ChangeSource,
    ) -> Result<Option<SplitViewProposal>, SplitViewError> {
        self.behavior.borrow().request(current, command, source)
    }

    pub fn propose_resize(
        &self,
        current: SplitViewValue,
        target: f32,
        phase: ChangePhase,
        source: ChangeSource,
    ) -> Result<SplitViewProposal, SplitViewError> {
        self.behavior
            .borrow()
            .propose_resize(current, target, phase, source)
    }

    pub fn handle_pointer(
        &self,
        current: SplitViewValue,
        input: GestureInput,
        track: SliderTrackGeometry,
    ) -> Result<SplitViewPointerOutcome, SplitViewError> {
        self.behavior
            .borrow_mut()
            .handle_pointer(current, input, track)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SplitViewPaneRef {
    pane: SplitViewPane,
    control: ControlHandle,
}

impl SplitViewPaneRef {
    pub const fn pane(self) -> SplitViewPane {
        self.pane
    }

    pub const fn node(self) -> UiNodeId {
        self.control.node
    }

    pub const fn style(self) -> Property<BoxStyle> {
        self.control.style
    }

    pub const fn visible(self) -> Property<bool> {
        self.control.visible
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitViewName {
    Group,
    PrimaryPane,
    SecondaryPane,
    Divider,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SplitViewError {
    MissingAccessibleName(SplitViewName),
    DuplicatePaneNames,
    InvalidPaneExtent,
    InvalidPrimaryMinimum,
    InvalidSecondaryMinimum,
    InsufficientResizableExtent,
    CollapsedPaneNotAllowed(SplitViewPane),
    CannotResizeCollapsed,
    Model(RangeModelError),
    Slider(SliderError),
}

impl From<RangeModelError> for SplitViewError {
    fn from(error: RangeModelError) -> Self {
        Self::Model(error)
    }
}

impl From<SliderError> for SplitViewError {
    fn from(error: SliderError) -> Self {
        Self::Slider(error)
    }
}

impl fmt::Display for SplitViewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid split view: {self:?}")
    }
}

impl std::error::Error for SplitViewError {}

#[cfg(test)]
mod tests;

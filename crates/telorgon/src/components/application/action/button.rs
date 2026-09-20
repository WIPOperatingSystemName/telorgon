//! Accessible Tier A push-button behavior, semantics, density, style, and mounting.

use crate::foundation::{ColorRgba8, EdgeInsets};
use crate::input::{Activation, ActivationInput, ActivationOutcome, ActivationStateMachine};
use crate::runtime::{RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    Background, BoxStyle, ControlHandle, CornerRadii, Property, SemanticActions, SemanticName,
    SemanticNode, SemanticRole, SemanticState, SizeRule, SizeRule2D, StringId, StylePropertyPatch,
    StyleSlotId, UiNodeId,
};

use crate::components::application::{ButtonStyleId, DensityClass, DensityMetrics};

/// Whether an enabled busy button accepts another completed activation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ButtonBusyPolicy {
    /// Keep the button focusable but suppress duplicate activation while work is pending.
    #[default]
    SuppressActivation,
    /// Continue accepting activation while busy.
    AllowActivation,
}

/// Component-visible inputs used by deterministic style and semantic resolution.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ButtonInteractionState {
    pub enabled: bool,
    pub busy: bool,
    pub hovered: bool,
    pub pressed: bool,
    pub focused: bool,
    pub focus_visible: bool,
}

impl ButtonInteractionState {
    pub const fn resting(enabled: bool, busy: bool) -> Self {
        Self {
            enabled,
            busy,
            hovered: false,
            pressed: false,
            focused: false,
            focus_visible: false,
        }
    }
}

/// Priority-selected visual state. Layout geometry is shared by every state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ButtonStyleState {
    Disabled,
    Busy,
    Pressed,
    Focused,
    Hovered,
    #[default]
    Resting,
}

impl ButtonStyleState {
    /// Resolves the shared button-family state priority independently of visual slots.
    pub const fn resolve(state: ButtonInteractionState) -> Self {
        if !state.enabled {
            Self::Disabled
        } else if state.busy {
            Self::Busy
        } else if state.pressed {
            Self::Pressed
        } else if state.focused && state.focus_visible {
            Self::Focused
        } else if state.hovered {
            Self::Hovered
        } else {
            Self::Resting
        }
    }
}

/// Named foundation slots for one resolved button visual state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ButtonVisualStyle {
    pub container: BoxStyle,
    pub label_color: ColorRgba8,
    pub label_size: f32,
}

/// Typed application-domain button style with deterministic state fallbacks.
///
/// Resolution priority is disabled, busy, pressed, focus-visible, hovered, then resting. Missing
/// variants fall back to `resting`. Density owns minimum hit geometry, so mounting replaces the
/// resolved container's minimum size with the effective density floor for every state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ButtonStyle {
    pub resting: ButtonVisualStyle,
    pub hovered: Option<ButtonVisualStyle>,
    pub focused: Option<ButtonVisualStyle>,
    pub pressed: Option<ButtonVisualStyle>,
    pub busy: Option<ButtonVisualStyle>,
    pub disabled: Option<ButtonVisualStyle>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedButtonStyle {
    pub state: ButtonStyleState,
    pub visual: ButtonVisualStyle,
}

impl ButtonStyle {
    pub const fn resolve(self, state: ButtonInteractionState) -> ResolvedButtonStyle {
        let resolved_state = ButtonStyleState::resolve(state);
        let visual = match resolved_state {
            ButtonStyleState::Disabled => self.disabled,
            ButtonStyleState::Busy => self.busy,
            ButtonStyleState::Pressed => self.pressed,
            ButtonStyleState::Focused => self.focused,
            ButtonStyleState::Hovered => self.hovered,
            ButtonStyleState::Resting => Some(self.resting),
        };
        ResolvedButtonStyle {
            state: resolved_state,
            visual: match visual {
                Some(visual) => visual,
                None => self.resting,
            },
        }
    }
}

impl Default for ButtonStyle {
    fn default() -> Self {
        let container = |color| BoxStyle {
            min_size: SizeRule2D {
                width: SizeRule::Logical(32.0),
                height: SizeRule::Logical(32.0),
            },
            padding: EdgeInsets {
                top: 7.0,
                right: 12.0,
                bottom: 7.0,
                left: 12.0,
            },
            decoration: crate::ui::BoxDecoration {
                background: Background::Color(color),
                corner_radii: CornerRadii::all(6.0),
                ..crate::ui::BoxDecoration::default()
            },
            ..BoxStyle::default()
        };
        let visual = |color, label_color| ButtonVisualStyle {
            container: container(color),
            label_color,
            label_size: 14.0,
        };
        Self {
            resting: visual(
                ColorRgba8::rgba(54, 60, 74, 255),
                ColorRgba8::rgba(248, 249, 252, 255),
            ),
            hovered: Some(visual(
                ColorRgba8::rgba(66, 74, 92, 255),
                ColorRgba8::rgba(255, 255, 255, 255),
            )),
            focused: Some(visual(
                ColorRgba8::rgba(61, 72, 101, 255),
                ColorRgba8::rgba(255, 255, 255, 255),
            )),
            pressed: Some(visual(
                ColorRgba8::rgba(42, 48, 61, 255),
                ColorRgba8::rgba(255, 255, 255, 255),
            )),
            busy: Some(visual(
                ColorRgba8::rgba(50, 55, 68, 255),
                ColorRgba8::rgba(214, 217, 225, 255),
            )),
            disabled: Some(visual(
                ColorRgba8::rgba(43, 46, 55, 180),
                ColorRgba8::rgba(153, 157, 168, 255),
            )),
        }
    }
}

/// Portable button behavior owner over the canonical neutral activation engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ButtonBehavior {
    activation: ActivationStateMachine,
    enabled: bool,
    busy: bool,
    busy_policy: ButtonBusyPolicy,
    hovered: bool,
    focused: bool,
    focus_visible: bool,
}

impl ButtonBehavior {
    pub const fn new(enabled: bool, busy: bool, busy_policy: ButtonBusyPolicy) -> Self {
        Self {
            activation: ActivationStateMachine::new(activation_allowed(enabled, busy, busy_policy)),
            enabled,
            busy,
            busy_policy,
            hovered: false,
            focused: false,
            focus_visible: false,
        }
    }

    pub fn handle(&mut self, input: ActivationInput) -> ActivationOutcome {
        match input {
            ActivationInput::SetEnabled(enabled) => self.set_enabled(enabled),
            _ => self.activation.handle(input),
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) -> ActivationOutcome {
        self.enabled = enabled;
        self.sync_eligibility()
    }

    pub fn set_busy(&mut self, busy: bool) -> ActivationOutcome {
        self.busy = busy;
        self.sync_eligibility()
    }

    pub fn set_busy_policy(&mut self, policy: ButtonBusyPolicy) -> ActivationOutcome {
        self.busy_policy = policy;
        self.sync_eligibility()
    }

    pub fn set_hovered(&mut self, hovered: bool) {
        self.hovered = hovered;
    }

    /// Applies focus-processor state. Losing focus cancels a Space-key arm through the canonical
    /// activation engine; pointer capture remains owned by the pointer route.
    pub fn set_focus(&mut self, focused: bool, focus_visible: bool) -> ActivationOutcome {
        let lost_focus = self.focused && !focused;
        self.focused = focused;
        self.focus_visible = focused && focus_visible;
        if lost_focus {
            self.activation.handle(ActivationInput::FocusLost)
        } else {
            ActivationOutcome::default()
        }
    }

    pub const fn interaction_state(&self) -> ButtonInteractionState {
        ButtonInteractionState {
            enabled: self.enabled,
            busy: self.busy,
            hovered: self.hovered,
            pressed: self.activation.is_visually_armed(),
            focused: self.focused,
            focus_visible: self.focus_visible,
        }
    }

    pub const fn activation_allowed(&self) -> bool {
        activation_allowed(self.enabled, self.busy, self.busy_policy)
    }

    fn sync_eligibility(&mut self) -> ActivationOutcome {
        self.activation
            .handle(ActivationInput::SetEnabled(self.activation_allowed()))
    }
}

/// Immutable mount configuration for one labelled application button.
#[derive(Clone, Debug, PartialEq)]
pub struct Button {
    label: String,
    enabled: bool,
    busy: bool,
    busy_policy: ButtonBusyPolicy,
    density: DensityMetrics,
    style: ButtonStyle,
    style_id: ButtonStyleId,
    style_override: StylePropertyPatch,
}

impl Button {
    pub fn new(label: impl Into<String>) -> Result<Self, ButtonError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(ButtonError::MissingAccessibleName);
        }
        Ok(Self {
            label,
            enabled: true,
            busy: false,
            busy_policy: ButtonBusyPolicy::SuppressActivation,
            density: DensityMetrics::baseline(DensityClass::Standard),
            style: ButtonStyle::default(),
            style_id: ButtonStyleId::DEFAULT,
            style_override: StylePropertyPatch::default(),
        })
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }

    pub fn busy_policy(mut self, policy: ButtonBusyPolicy) -> Self {
        self.busy_policy = policy;
        self
    }

    pub fn density(mut self, density: DensityMetrics) -> Self {
        self.density = density;
        self
    }

    pub fn style(mut self, style: ButtonStyle) -> Self {
        self.style = style;
        self
    }

    pub fn style_id(mut self, style_id: ButtonStyleId) -> Self {
        self.style_id = style_id;
        self
    }

    /// Adds a sparse local root-slot override after scoped theme resolution.
    pub fn style_override(mut self, style: StylePropertyPatch) -> Self {
        self.style_override = style;
        self
    }

    pub fn behavior(&self) -> ButtonBehavior {
        ButtonBehavior::new(self.enabled, self.busy, self.busy_policy)
    }

    pub(crate) const fn initial_interaction_state(&self) -> ButtonInteractionState {
        ButtonInteractionState::resting(self.enabled, self.busy)
    }

    pub(crate) const fn density_metrics(&self) -> DensityMetrics {
        self.density
    }

    pub(crate) fn label(&self) -> &str {
        &self.label
    }

    pub(crate) const fn accepts_activation(&self) -> bool {
        activation_allowed(self.enabled, self.busy, self.busy_policy)
    }

    pub fn semantic_node(&self, name: StringId, state: ButtonInteractionState) -> SemanticNode {
        let can_activate = activation_allowed(state.enabled, state.busy, self.busy_policy);
        let mut actions = SemanticActions::NONE;
        if state.enabled {
            actions |= SemanticActions::FOCUS;
        }
        if can_activate {
            actions |= SemanticActions::ACTIVATE;
        }
        SemanticNode {
            role: SemanticRole::Button,
            name: SemanticName::Text(name),
            state: SemanticState {
                disabled: !state.enabled,
                busy: state.busy,
                focusable: state.enabled,
                focused: state.focused,
                ..SemanticState::default()
            },
            actions,
            ..SemanticNode::default()
        }
    }

    pub(crate) fn attach_mounted_contract<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        node: UiNodeId,
    ) -> RuntimeResult<()> {
        self.attach_mounted_contract_with(ui, node, |_| {})
    }

    pub(crate) fn attach_mounted_contract_with<Action, Amend>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        node: UiNodeId,
        amend: Amend,
    ) -> RuntimeResult<()>
    where
        Action: 'static,
        Amend: FnOnce(&mut SemanticNode),
    {
        let state = self.initial_interaction_state();
        let name = ui.foundation().intern(&self.label);
        let mut semantic = self.semantic_node(name, state);
        amend(&mut semantic);
        ui.foundation()
            .semantic_node(node, semantic)
            .map_err(|error| RuntimeError::new(format!("invalid button semantics: {error:?}")))?;
        if !self.enabled {
            ui.foundation().disabled(node, true);
        }
        if self.busy {
            ui.foundation().busy(node, true);
        }
        Ok(())
    }

    pub(crate) fn route_mounted_activation<Action, Map>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        node: UiNodeId,
        map: Map,
    ) -> RuntimeResult<()>
    where
        Action: 'static,
        Map: Fn(Activation) -> Action + 'static,
    {
        if self.accepts_activation() {
            ui.route_activation(node, map)?;
        }
        Ok(())
    }

    /// Mounts one real foundation button and registers a source-preserving component action route.
    pub fn mount<Action, Map>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
        map: Map,
    ) -> RuntimeResult<ButtonRef>
    where
        Action: 'static,
        Map: Fn(Activation) -> Action + 'static,
    {
        self.mount_with_semantics(ui, host, |_| {}, move |_, activation| map(activation))
    }

    /// Mounts the canonical button while allowing a composed component to amend semantics and
    /// include the stable button identity in its routed action.
    pub(crate) fn mount_with_semantics<Action, Amend, Map>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
        amend: Amend,
        map: Map,
    ) -> RuntimeResult<ButtonRef>
    where
        Action: 'static,
        Amend: FnOnce(&mut SemanticNode),
        Map: Fn(UiNodeId, Activation) -> Action + 'static,
    {
        let state = self.initial_interaction_state();
        let mut visual = self.style.resolve(state).visual;
        let minimum = self.density_metrics().effective_minimum();
        visual.container.min_size = SizeRule2D {
            width: SizeRule::Logical(minimum.width()),
            height: SizeRule::Logical(minimum.height()),
        };

        let label = self.label.clone();
        let label_color = visual.label_color;
        let label_size = visual.label_size;
        let control = ui
            .foundation()
            .button_node_under(host, visual.container, move |writer| {
                writer.text(&label, label_color, label_size);
            })
            .ok_or_else(|| RuntimeError::new("application button host is stale"))?;

        ui.foundation().style_id(control.node, self.style_id.0);
        if self.style_override != StylePropertyPatch::default() {
            ui.foundation().style_override(
                control.node,
                StyleSlotId::named("root"),
                self.style_override,
            );
        }

        self.attach_mounted_contract_with(ui, control.node, amend)?;
        let node = control.node;
        self.route_mounted_activation(ui, node, move |activation| map(node, activation))?;

        Ok(ButtonRef { control })
    }
}

/// Focused advanced reference returned by button mounting.
#[derive(Clone, Copy, Debug)]
pub struct ButtonRef {
    control: ControlHandle,
}

impl ButtonRef {
    pub const fn node(self) -> UiNodeId {
        self.control.node
    }

    pub const fn enabled(self) -> Property<bool> {
        self.control.enabled
    }

    pub const fn style(self) -> Property<BoxStyle> {
        self.control.style
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonError {
    MissingAccessibleName,
}

impl std::fmt::Display for ButtonError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingAccessibleName => formatter.write_str("button accessible name is empty"),
        }
    }
}

impl std::error::Error for ButtonError {}

const fn activation_allowed(enabled: bool, busy: bool, busy_policy: ButtonBusyPolicy) -> bool {
    enabled && !(busy && matches!(busy_policy, ButtonBusyPolicy::SuppressActivation))
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use crate::input::{
        ActivationCancelReason, ActivationInput, ActivationTransition, ChangeSource, PointerButton,
        PointerCaptureRequest, PointerId,
    };
    use crate::runtime::{Component, CreateContext, UpdateContext, ViewRuntime};
    use crate::ui::{LayoutStyle, SemanticAction, UiRoot};

    use super::*;

    const POINTER: PointerId = PointerId::new(9);

    #[test]
    fn behavior_preserves_pointer_keyboard_semantic_and_cancellation_rules() {
        let mut behavior = ButtonBehavior::new(true, false, ButtonBusyPolicy::SuppressActivation);
        assert_eq!(
            behavior
                .handle(ActivationInput::PointerDown {
                    pointer: POINTER,
                    button: PointerButton::PRIMARY,
                })
                .capture,
            PointerCaptureRequest::Capture(POINTER)
        );
        behavior.handle(ActivationInput::PointerMoved {
            pointer: POINTER,
            inside: false,
        });
        assert!(!behavior.interaction_state().pressed);
        behavior.handle(ActivationInput::PointerMoved {
            pointer: POINTER,
            inside: true,
        });
        assert!(behavior.interaction_state().pressed);
        assert_eq!(
            behavior
                .handle(ActivationInput::PointerUp {
                    pointer: POINTER,
                    button: PointerButton::PRIMARY,
                    inside: true,
                })
                .transition,
            ActivationTransition::Activated(Activation {
                source: ChangeSource::Pointer,
            })
        );

        behavior.handle(ActivationInput::SpaceDown { repeat: false });
        assert!(behavior.interaction_state().pressed);
        assert!(matches!(
            behavior.set_focus(false, false).transition,
            ActivationTransition::Ignored
        ));
        behavior.set_focus(true, true);
        behavior.handle(ActivationInput::SpaceDown { repeat: false });
        assert!(matches!(
            behavior.set_focus(false, false).transition,
            ActivationTransition::Cancelled {
                reason: ActivationCancelReason::FocusLost,
                ..
            }
        ));
        assert_eq!(
            behavior
                .handle(ActivationInput::EnterDown { repeat: false })
                .transition,
            ActivationTransition::Activated(Activation {
                source: ChangeSource::Keyboard,
            })
        );
        assert_eq!(
            behavior
                .handle(ActivationInput::SemanticActivate)
                .transition,
            ActivationTransition::Activated(Activation {
                source: ChangeSource::Accessibility,
            })
        );
    }

    #[test]
    fn busy_policy_cancels_an_arm_but_keeps_focus_semantics() {
        let button = Button::new("Save").unwrap().busy(true);
        let behavior = button.behavior();
        assert!(!behavior.activation_allowed());
        let semantic =
            button.semantic_node(StringId(1), ButtonInteractionState::resting(true, true));
        assert!(semantic.state.busy);
        assert!(semantic.state.focusable);
        assert!(semantic.actions.contains(SemanticAction::Focus));
        assert!(!semantic.actions.contains(SemanticAction::Activate));

        let mut active = ButtonBehavior::new(true, false, ButtonBusyPolicy::SuppressActivation);
        active.handle(ActivationInput::PointerDown {
            pointer: POINTER,
            button: PointerButton::PRIMARY,
        });
        assert!(matches!(
            active.set_busy(true).transition,
            ActivationTransition::Cancelled {
                reason: ActivationCancelReason::Disabled,
                ..
            }
        ));
    }

    #[test]
    fn style_priority_is_deterministic_and_missing_variants_fall_back() {
        let style = ButtonStyle {
            busy: None,
            ..ButtonStyle::default()
        };
        let state = ButtonInteractionState {
            enabled: true,
            busy: true,
            hovered: true,
            pressed: true,
            focused: true,
            focus_visible: true,
        };
        let resolved = style.resolve(state);
        assert_eq!(resolved.state, ButtonStyleState::Busy);
        assert_eq!(resolved.visual, style.resting);

        let disabled = style.resolve(ButtonInteractionState {
            enabled: false,
            ..state
        });
        assert_eq!(disabled.state, ButtonStyleState::Disabled);
    }

    struct MountedButton {
        node: Rc<Cell<Option<UiNodeId>>>,
        received: Rc<RefCell<Vec<Activation>>>,
        button: Button,
    }

    impl Component for MountedButton {
        type State = ();
        type Action = Activation;

        fn create(&self, _context: &mut CreateContext<'_>) -> Self::State {}

        fn mount(&self, _state: &(), ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            let button = self
                .button
                .mount(ui, root.0, |activation| activation)
                .unwrap();
            self.node.set(Some(button.node()));
            root
        }

        fn action(
            &self,
            _state: &mut (),
            action: Self::Action,
            _context: &mut UpdateContext<'_, Self>,
        ) {
            self.received.borrow_mut().push(action);
        }
    }

    #[test]
    fn mounted_button_has_semantics_density_and_source_preserving_actions() {
        let node = Rc::new(Cell::new(None));
        let received = Rc::new(RefCell::new(Vec::new()));
        let density = DensityMetrics::baseline(DensityClass::Touch);
        let button = Button::new("Launch").unwrap().density(density);
        let mut runtime = ViewRuntime::from_component(MountedButton {
            node: node.clone(),
            received: received.clone(),
            button,
        })
        .unwrap();
        let node = node.get().unwrap();

        let semantic = runtime.ui().semantics.get(node).unwrap();
        assert_eq!(semantic.role, SemanticRole::Button);
        let SemanticName::Text(name) = semantic.name else {
            panic!("button must own a text accessible name");
        };
        assert_eq!(runtime.ui().string(name), Some("Launch"));
        assert!(semantic.actions.contains(SemanticAction::Activate));
        assert!(semantic.actions.contains(SemanticAction::Focus));
        let style = runtime.ui().box_styles.get(node).unwrap();
        assert_eq!(style.min_size.width, SizeRule::Logical(44.0));
        assert_eq!(style.min_size.height, SizeRule::Logical(44.0));

        assert!(runtime.dispatch_activation(node, ChangeSource::Pointer));
        assert!(runtime.dispatch_activation(node, ChangeSource::Accessibility));
        assert!(runtime.dispatch_action(node));
        assert_eq!(
            &*received.borrow(),
            &[
                Activation {
                    source: ChangeSource::Pointer,
                },
                Activation {
                    source: ChangeSource::Accessibility,
                },
                Activation {
                    source: ChangeSource::Programmatic,
                },
            ]
        );
    }

    #[test]
    fn disabled_mount_advertises_no_effective_action_and_has_no_route() {
        let node = Rc::new(Cell::new(None));
        let received = Rc::new(RefCell::new(Vec::new()));
        let mut runtime = ViewRuntime::from_component(MountedButton {
            node: node.clone(),
            received,
            button: Button::new("Unavailable").unwrap().enabled(false),
        })
        .unwrap();
        let node = node.get().unwrap();
        let semantic = runtime.ui().semantics.get(node).unwrap();
        assert!(semantic.state.disabled);
        assert!(semantic.effective_actions().is_empty());
        assert!(!runtime.dispatch_activation(node, ChangeSource::Pointer));
    }

    #[test]
    fn button_rejects_an_empty_accessible_name() {
        assert_eq!(
            Button::new("  ").unwrap_err(),
            ButtonError::MissingAccessibleName
        );
    }
}

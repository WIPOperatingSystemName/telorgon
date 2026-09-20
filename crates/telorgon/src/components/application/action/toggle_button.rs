//! Controlled Tier A toggle button built on the shared button-family contract.

use crate::foundation::ColorRgba8;
use crate::runtime::{Read, RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    Background, BoxStyle, ControlHandle, Property, SemanticNode, SizeRule, SizeRule2D, StringId,
    UiNodeId,
};

use crate::components::application::{
    Button, ButtonBehavior, ButtonBusyPolicy, ButtonError, ButtonInteractionState, ButtonStyle,
    ButtonStyleState, ButtonVisualStyle, DensityMetrics, ValueChange,
};

/// Typed styles for the controlled off and on values of a toggle button.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToggleButtonStyle {
    pub off: ButtonStyle,
    pub on: ButtonStyle,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedToggleButtonStyle {
    pub value: bool,
    pub state: ButtonStyleState,
    pub visual: ButtonVisualStyle,
}

impl ToggleButtonStyle {
    pub const fn resolve(
        self,
        value: bool,
        state: ButtonInteractionState,
    ) -> ResolvedToggleButtonStyle {
        let resolved = if value {
            self.on.resolve(state)
        } else {
            self.off.resolve(state)
        };
        ResolvedToggleButtonStyle {
            value,
            state: resolved.state,
            visual: resolved.visual,
        }
    }
}

impl Default for ToggleButtonStyle {
    fn default() -> Self {
        let off = ButtonStyle::default();
        let recolor = |mut visual: ButtonVisualStyle, color| {
            visual.container.decoration.background = Background::Color(color);
            visual
        };
        let map =
            |visual: Option<ButtonVisualStyle>, color| visual.map(|visual| recolor(visual, color));
        let on = ButtonStyle {
            resting: recolor(off.resting, ColorRgba8::rgba(54, 88, 166, 255)),
            hovered: map(off.hovered, ColorRgba8::rgba(65, 103, 190, 255)),
            focused: map(off.focused, ColorRgba8::rgba(67, 99, 177, 255)),
            pressed: map(off.pressed, ColorRgba8::rgba(43, 72, 140, 255)),
            busy: map(off.busy, ColorRgba8::rgba(52, 75, 127, 255)),
            disabled: map(off.disabled, ColorRgba8::rgba(48, 60, 87, 180)),
        };
        Self { off, on }
    }
}

/// Immutable configuration for a labelled, parent-controlled toggle button.
#[derive(Clone, Debug, PartialEq)]
pub struct ToggleButton {
    button: Button,
    value: Read<bool>,
    style: ToggleButtonStyle,
}

impl ToggleButton {
    pub fn new(label: impl Into<String>, value: Read<bool>) -> Result<Self, ToggleButtonError> {
        let button = Button::new(label).map_err(ToggleButtonError::from)?;
        Ok(Self {
            button,
            value,
            style: ToggleButtonStyle::default(),
        })
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.button = self.button.enabled(enabled);
        self
    }

    pub fn busy(mut self, busy: bool) -> Self {
        self.button = self.button.busy(busy);
        self
    }

    pub fn busy_policy(mut self, policy: ButtonBusyPolicy) -> Self {
        self.button = self.button.busy_policy(policy);
        self
    }

    pub fn density(mut self, density: DensityMetrics) -> Self {
        self.button = self.button.density(density);
        self
    }

    pub fn style(mut self, style: ToggleButtonStyle) -> Self {
        self.style = style;
        self
    }

    pub fn behavior(&self) -> ButtonBehavior {
        self.button.behavior()
    }

    pub fn semantic_node(
        &self,
        name: StringId,
        value: bool,
        state: ButtonInteractionState,
    ) -> SemanticNode {
        let mut semantic = self.button.semantic_node(name, state);
        semantic.state.pressed = Some(value);
        semantic
    }

    /// Mounts the current controlled value and routes each accepted activation to an inverse-value
    /// proposal based on the latest read. The route never writes the controlled value itself.
    pub fn mount<Action, Map>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
        map: Map,
    ) -> RuntimeResult<ToggleButtonRef>
    where
        Action: 'static,
        Map: Fn(ValueChange<bool>) -> Action + 'static,
    {
        let value = ui.read(self.value)?;
        let state = self.button.initial_interaction_state();
        let mut visual = self.style.resolve(value, state).visual;
        let minimum = self.button.density_metrics().effective_minimum();
        visual.container.min_size = SizeRule2D {
            width: SizeRule::Logical(minimum.width()),
            height: SizeRule::Logical(minimum.height()),
        };

        let label = self.button.label().to_owned();
        let label_color = visual.label_color;
        let label_size = visual.label_size;
        let control = ui
            .foundation()
            .button_node_under(host, visual.container, move |writer| {
                writer.text(&label, label_color, label_size);
            })
            .ok_or_else(|| RuntimeError::new("application toggle-button host is stale"))?;

        self.button
            .attach_mounted_contract_with(ui, control.node, |semantic| {
                semantic.state.pressed = Some(value);
            })?;
        if self.button.accepts_activation() {
            ui.route_activation_read(control.node, self.value, move |current, activation| {
                map(ValueChange::committed(!*current, activation.source))
            })?;
        }

        Ok(ToggleButtonRef {
            control,
            value: self.value,
        })
    }
}

/// Focused advanced reference returned by toggle-button mounting.
#[derive(Clone, Copy, Debug)]
pub struct ToggleButtonRef {
    control: ControlHandle,
    value: Read<bool>,
}

impl ToggleButtonRef {
    pub const fn node(self) -> UiNodeId {
        self.control.node
    }

    pub const fn value(self) -> Read<bool> {
        self.value
    }

    pub const fn enabled(self) -> Property<bool> {
        self.control.enabled
    }

    pub const fn style(self) -> Property<BoxStyle> {
        self.control.style
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToggleButtonError {
    MissingAccessibleName,
}

impl From<ButtonError> for ToggleButtonError {
    fn from(error: ButtonError) -> Self {
        match error {
            ButtonError::MissingAccessibleName => Self::MissingAccessibleName,
        }
    }
}

impl std::fmt::Display for ToggleButtonError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingAccessibleName => {
                formatter.write_str("toggle button accessible name is empty")
            }
        }
    }
}

impl std::error::Error for ToggleButtonError {}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use crate::input::ChangeSource;
    use crate::runtime::{Component, CreateContext, State, UpdateContext, ViewRuntime};
    use crate::ui::{LayoutStyle, SemanticAction, SemanticName, SemanticRole, UiRoot};

    use crate::components::application::{ChangePhase, DensityClass};

    use super::*;

    #[test]
    fn toggle_style_resolves_controlled_value_before_shared_interaction_priority() {
        let style = ToggleButtonStyle::default();
        let off = style.resolve(false, ButtonInteractionState::resting(true, false));
        let on = style.resolve(true, ButtonInteractionState::resting(true, false));
        assert!(!off.value);
        assert!(on.value);
        assert_ne!(
            off.visual.container.decoration.background,
            on.visual.container.decoration.background
        );

        let pressed = style.resolve(
            true,
            ButtonInteractionState {
                pressed: true,
                hovered: true,
                ..ButtonInteractionState::resting(true, false)
            },
        );
        assert_eq!(pressed.state, ButtonStyleState::Pressed);
    }

    struct ToggleState {
        value: State<bool>,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum ToggleAction {
        Requested(ValueChange<bool>),
        Publish(bool),
    }

    struct MountedToggle {
        node: Rc<Cell<Option<UiNodeId>>>,
        requests: Rc<RefCell<Vec<ValueChange<bool>>>>,
    }

    impl Component for MountedToggle {
        type State = ToggleState;
        type Action = ToggleAction;

        fn create(&self, context: &mut CreateContext<'_>) -> Self::State {
            ToggleState {
                value: context.state(true),
            }
        }

        fn mount(&self, state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            let toggle = ToggleButton::new("Pinned", state.value.read())
                .unwrap()
                .density(DensityMetrics::baseline(DensityClass::Touch))
                .mount(ui, root.0, ToggleAction::Requested)
                .unwrap();
            self.node.set(Some(toggle.node()));
            root
        }

        fn action(
            &self,
            state: &mut Self::State,
            action: Self::Action,
            context: &mut UpdateContext<'_, Self>,
        ) {
            match action {
                ToggleAction::Requested(change) => self.requests.borrow_mut().push(change),
                ToggleAction::Publish(value) => context.set(state.value, value).unwrap(),
            }
        }
    }

    #[test]
    fn mounted_toggle_reads_live_controlled_value_and_only_emits_inverse_requests() {
        let node = Rc::new(Cell::new(None));
        let requests = Rc::new(RefCell::new(Vec::new()));
        let mut runtime = ViewRuntime::from_component(MountedToggle {
            node: node.clone(),
            requests: requests.clone(),
        })
        .unwrap();
        let node = node.get().unwrap();

        let semantic = runtime.ui().semantics.get(node).unwrap();
        assert_eq!(semantic.role, SemanticRole::Button);
        assert_eq!(semantic.state.pressed, Some(true));
        assert!(semantic.actions.contains(SemanticAction::Activate));
        let SemanticName::Text(name) = semantic.name else {
            panic!("toggle button must be named by its stable label");
        };
        assert_eq!(runtime.ui().string(name), Some("Pinned"));
        assert_eq!(
            runtime.ui().box_styles.get(node).unwrap().min_size,
            SizeRule2D {
                width: SizeRule::Logical(44.0),
                height: SizeRule::Logical(44.0),
            }
        );

        assert!(runtime.dispatch_activation(node, ChangeSource::Pointer));
        assert!(runtime.dispatch_activation(node, ChangeSource::Accessibility));
        assert_eq!(
            &*requests.borrow(),
            &[
                ValueChange::new(false, ChangePhase::Commit, ChangeSource::Pointer),
                ValueChange::new(false, ChangePhase::Commit, ChangeSource::Accessibility),
            ]
        );

        runtime
            .send_component_action(ToggleAction::Publish(false))
            .unwrap();
        assert!(runtime.dispatch_action(node));
        assert_eq!(
            requests.borrow().last(),
            Some(&ValueChange::new(
                true,
                ChangePhase::Commit,
                ChangeSource::Programmatic,
            ))
        );
    }
}

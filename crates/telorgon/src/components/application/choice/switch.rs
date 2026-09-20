//! Controlled Tier A switch built on canonical activation and boolean value ownership.

use crate::foundation::{ColorRgba8, EdgeInsets, PointF, Transform2D};
use crate::runtime::{Read, RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    Background, Border, BoxStyle, ControlHandle, CornerRadii, Flow, LayoutStyle, Property,
    SemanticCheckState, SemanticNode, SemanticRole, SizeRule, SizeRule2D, StringId, UiNodeId,
};

use crate::components::application::{
    Button, ButtonBehavior, ButtonError, ButtonInteractionState, ButtonStyleState, DensityMetrics,
    ValueChange,
};

/// Visual slots for one switch value and interaction state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SwitchVisualStyle {
    pub container: BoxStyle,
    pub track: BoxStyle,
    pub thumb: BoxStyle,
    pub label_color: ColorRgba8,
    pub label_size: f32,
    pub gap: f32,
}

/// Interaction variants for one controlled switch value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SwitchStateStyle {
    pub resting: SwitchVisualStyle,
    pub hovered: Option<SwitchVisualStyle>,
    pub focused: Option<SwitchVisualStyle>,
    pub pressed: Option<SwitchVisualStyle>,
    pub disabled: Option<SwitchVisualStyle>,
}

impl SwitchStateStyle {
    const fn resolve(self, state: ButtonInteractionState) -> (ButtonStyleState, SwitchVisualStyle) {
        let resolved_state = ButtonStyleState::resolve(state);
        let visual = match resolved_state {
            ButtonStyleState::Disabled => self.disabled,
            ButtonStyleState::Pressed => self.pressed,
            ButtonStyleState::Focused => self.focused,
            ButtonStyleState::Hovered => self.hovered,
            ButtonStyleState::Busy | ButtonStyleState::Resting => Some(self.resting),
        };
        (
            resolved_state,
            match visual {
                Some(visual) => visual,
                None => self.resting,
            },
        )
    }
}

/// Typed off/on styles for a controlled switch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SwitchStyle {
    pub off: SwitchStateStyle,
    pub on: SwitchStateStyle,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedSwitchStyle {
    pub value: bool,
    pub state: ButtonStyleState,
    pub visual: SwitchVisualStyle,
}

impl SwitchStyle {
    pub const fn resolve(self, value: bool, state: ButtonInteractionState) -> ResolvedSwitchStyle {
        let (resolved_state, visual) = if value {
            self.on.resolve(state)
        } else {
            self.off.resolve(state)
        };
        ResolvedSwitchStyle {
            value,
            state: resolved_state,
            visual,
        }
    }
}

impl Default for SwitchStyle {
    fn default() -> Self {
        fn visual(
            value: bool,
            track_color: ColorRgba8,
            container_background: Background,
            opacity: u8,
        ) -> SwitchVisualStyle {
            let thumb_offset = if value { 16.0 } else { 0.0 };
            SwitchVisualStyle {
                container: BoxStyle {
                    min_size: SizeRule2D {
                        width: SizeRule::Logical(32.0),
                        height: SizeRule::Logical(32.0),
                    },
                    padding: EdgeInsets::all(5.0),
                    decoration: crate::ui::BoxDecoration {
                        background: container_background,
                        corner_radii: CornerRadii::all(4.0),
                        ..crate::ui::BoxDecoration::default()
                    },
                    ..BoxStyle::default()
                },
                track: BoxStyle {
                    width: SizeRule::Logical(38.0),
                    height: SizeRule::Logical(22.0),
                    padding: EdgeInsets::all(2.0),
                    decoration: crate::ui::BoxDecoration {
                        background: Background::Color(track_color),
                        border: Border::all(1.0, ColorRgba8::rgba(118, 127, 145, opacity)),
                        corner_radii: CornerRadii::all(11.0),
                        ..crate::ui::BoxDecoration::default()
                    },
                    ..BoxStyle::default()
                },
                thumb: BoxStyle {
                    width: SizeRule::Logical(16.0),
                    height: SizeRule::Logical(16.0),
                    decoration: crate::ui::BoxDecoration {
                        background: Background::Color(ColorRgba8::rgba(248, 249, 252, opacity)),
                        corner_radii: CornerRadii::all(8.0),
                        ..crate::ui::BoxDecoration::default()
                    },
                    transform: Transform2D {
                        translation: PointF {
                            x: thumb_offset,
                            y: 0.0,
                        },
                        ..Transform2D::default()
                    },
                    ..BoxStyle::default()
                },
                label_color: ColorRgba8::rgba(235, 238, 244, opacity),
                label_size: 14.0,
                gap: 8.0,
            }
        }

        fn state_style(value: bool, track_color: ColorRgba8) -> SwitchStateStyle {
            SwitchStateStyle {
                resting: visual(value, track_color, Background::None, 255),
                hovered: Some(visual(
                    value,
                    track_color,
                    Background::Color(ColorRgba8::rgba(69, 78, 96, 90)),
                    255,
                )),
                focused: Some(visual(
                    value,
                    track_color,
                    Background::Color(ColorRgba8::rgba(66, 91, 139, 110)),
                    255,
                )),
                pressed: Some(visual(
                    value,
                    track_color,
                    Background::Color(ColorRgba8::rgba(46, 55, 72, 140)),
                    255,
                )),
                disabled: Some(visual(
                    value,
                    ColorRgba8::rgba(track_color.r, track_color.g, track_color.b, 180),
                    Background::None,
                    180,
                )),
            }
        }

        Self {
            off: state_style(false, ColorRgba8::rgba(75, 84, 102, 255)),
            on: state_style(true, ColorRgba8::rgba(54, 104, 210, 255)),
        }
    }
}

/// Immutable configuration for a labelled, parent-controlled switch.
#[derive(Clone, Debug, PartialEq)]
pub struct Switch {
    button: Button,
    value: Read<bool>,
    style: SwitchStyle,
}

impl Switch {
    pub fn new(label: impl Into<String>, value: Read<bool>) -> Result<Self, SwitchError> {
        let button = Button::new(label).map_err(SwitchError::from)?;
        Ok(Self {
            button,
            value,
            style: SwitchStyle::default(),
        })
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.button = self.button.enabled(enabled);
        self
    }

    pub fn density(mut self, density: DensityMetrics) -> Self {
        self.button = self.button.density(density);
        self
    }

    pub fn style(mut self, style: SwitchStyle) -> Self {
        self.style = style;
        self
    }

    pub fn behavior(&self) -> ButtonBehavior {
        self.button.behavior()
    }

    pub const fn value(&self) -> Read<bool> {
        self.value
    }

    pub fn semantic_node(
        &self,
        name: StringId,
        value: bool,
        state: ButtonInteractionState,
    ) -> SemanticNode {
        let mut semantic = self.button.semantic_node(name, state);
        semantic.role = SemanticRole::Switch;
        semantic.state.checked = Some(semantic_check_state(value));
        semantic
    }

    /// Mounts the current controlled value and derives every proposal from the latest read.
    pub fn mount<Action, Map>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
        map: Map,
    ) -> RuntimeResult<SwitchRef>
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
        let track = visual.track;
        let thumb = visual.thumb;
        let label_color = visual.label_color;
        let label_size = visual.label_size;
        let row_layout = LayoutStyle {
            flow: Flow::Horizontal,
            gap: visual.gap,
            ..LayoutStyle::default()
        };
        let mut track_control = None;
        let mut thumb_control = None;
        let mut label_control = None;
        let control = ui
            .foundation()
            .toggle_node_under(host, visual.container, |writer| {
                writer.container(BoxStyle::default(), row_layout, |writer| {
                    track_control =
                        Some(
                            writer.container_handle(track, LayoutStyle::default(), |writer| {
                                thumb_control = Some(writer.container_handle(
                                    thumb,
                                    LayoutStyle::default(),
                                    |_| {},
                                ));
                            }),
                        );
                    label_control = Some(writer.text(&label, label_color, label_size));
                });
            })
            .ok_or_else(|| RuntimeError::new("application switch host is stale"))?;
        let track_control = track_control.expect("switch track mounts with its control");
        let thumb_control = thumb_control.expect("switch thumb mounts with its track");
        let label_control = label_control.expect("switch label mounts with its control");

        self.button
            .attach_mounted_contract_with(ui, control.node, |semantic| {
                semantic.role = SemanticRole::Switch;
                semantic.state.checked = Some(semantic_check_state(value));
            })?;
        let read = self.value;
        ui.bind_map(read, control.checked, |value| semantic_check_state(*value))?;
        let style = self.style;
        ui.bind_map(read, control.style, move |value| {
            let mut visual = style.resolve(*value, state).visual;
            visual.container.min_size = SizeRule2D {
                width: SizeRule::Logical(minimum.width()),
                height: SizeRule::Logical(minimum.height()),
            };
            visual.container
        })?;
        let style = self.style;
        ui.bind_map(read, track_control.style, move |value| {
            style.resolve(*value, state).visual.track
        })?;
        let style = self.style;
        ui.bind_map(read, thumb_control.style, move |value| {
            style.resolve(*value, state).visual.thumb
        })?;
        let style = self.style;
        ui.bind_map(read, label_control.color, move |value| {
            style.resolve(*value, state).visual.label_color
        })?;
        if self.button.accepts_activation() {
            ui.route_activation_read(control.node, self.value, move |current, activation| {
                map(ValueChange::committed(!*current, activation.source))
            })?;
        }

        Ok(SwitchRef {
            control,
            track: track_control,
            thumb: thumb_control,
            value: self.value,
        })
    }
}

/// Focused advanced reference returned by switch mounting.
#[derive(Clone, Copy, Debug)]
pub struct SwitchRef {
    control: ControlHandle,
    track: ControlHandle,
    thumb: ControlHandle,
    value: Read<bool>,
}

impl SwitchRef {
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

    pub const fn track_node(self) -> UiNodeId {
        self.track.node
    }

    pub const fn thumb_node(self) -> UiNodeId {
        self.thumb.node
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwitchError {
    MissingAccessibleName,
}

impl From<ButtonError> for SwitchError {
    fn from(error: ButtonError) -> Self {
        match error {
            ButtonError::MissingAccessibleName => Self::MissingAccessibleName,
        }
    }
}

impl std::fmt::Display for SwitchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingAccessibleName => formatter.write_str("switch accessible name is empty"),
        }
    }
}

impl std::error::Error for SwitchError {}

const fn semantic_check_state(value: bool) -> SemanticCheckState {
    if value {
        SemanticCheckState::Checked
    } else {
        SemanticCheckState::Unchecked
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use crate::input::{
        Activation, ActivationInput, ActivationTransition, ChangeSource, PointerButton, PointerId,
    };
    use crate::runtime::{Component, CreateContext, State, UpdateContext, ViewRuntime};
    use crate::ui::{LayoutStyle, SemanticAction, SemanticName, SemanticRole, UiRoot};

    use crate::components::application::{ChangePhase, DensityClass};

    use super::*;

    #[test]
    fn style_and_semantic_value_mapping_cover_both_values() {
        let interaction = ButtonInteractionState {
            pressed: true,
            hovered: true,
            ..ButtonInteractionState::resting(true, false)
        };
        let off = SwitchStyle::default().resolve(false, interaction);
        let on = SwitchStyle::default().resolve(true, interaction);
        assert_eq!(off.state, ButtonStyleState::Pressed);
        assert_eq!(on.state, ButtonStyleState::Pressed);
        assert_ne!(
            off.visual.track.decoration.background,
            on.visual.track.decoration.background
        );
        assert_ne!(off.visual.thumb.transform, on.visual.thumb.transform);

        for (value, expected) in [
            (false, SemanticCheckState::Unchecked),
            (true, SemanticCheckState::Checked),
        ] {
            assert_eq!(semantic_check_state(value), expected);
        }
    }

    struct SwitchOwner {
        node: Rc<Cell<Option<UiNodeId>>>,
        requests: Rc<RefCell<Vec<ValueChange<bool>>>>,
        enabled: bool,
    }

    struct OwnerState {
        value: State<bool>,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum OwnerAction {
        Requested(ValueChange<bool>),
        Publish(bool),
    }

    impl Component for SwitchOwner {
        type State = OwnerState;
        type Action = OwnerAction;

        fn create(&self, context: &mut CreateContext<'_>) -> Self::State {
            OwnerState {
                value: context.state(true),
            }
        }

        fn mount(&self, state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            assert_eq!(
                Switch::new(" ", state.value.read()),
                Err(SwitchError::MissingAccessibleName)
            );
            let switch = Switch::new("Wireless", state.value.read()).unwrap();
            if self.enabled {
                let mut behavior = switch.behavior();
                let pointer = PointerId::new(7);
                behavior.handle(ActivationInput::PointerDown {
                    pointer,
                    button: PointerButton::PRIMARY,
                });
                assert!(matches!(
                    behavior
                        .handle(ActivationInput::PointerUp {
                            pointer,
                            button: PointerButton::PRIMARY,
                            inside: false,
                        })
                        .transition,
                    ActivationTransition::Cancelled { .. }
                ));
                behavior.set_focus(true, true);
                behavior.handle(ActivationInput::SpaceDown { repeat: false });
                assert_eq!(
                    behavior.handle(ActivationInput::SpaceUp).transition,
                    ActivationTransition::Activated(Activation {
                        source: ChangeSource::Keyboard,
                    })
                );
            }
            let switch = switch
                .enabled(self.enabled)
                .density(DensityMetrics::baseline(DensityClass::Touch))
                .mount(ui, root.0, OwnerAction::Requested)
                .unwrap();
            self.node.set(Some(switch.node()));
            root
        }

        fn action(
            &self,
            state: &mut Self::State,
            action: Self::Action,
            context: &mut UpdateContext<'_, Self>,
        ) {
            match action {
                OwnerAction::Requested(change) => self.requests.borrow_mut().push(change),
                OwnerAction::Publish(value) => context.set(state.value, value).unwrap(),
            }
        }
    }

    #[test]
    fn mounted_switch_reads_latest_value_and_only_emits_inverse_proposals() {
        let node = Rc::new(Cell::new(None));
        let requests = Rc::new(RefCell::new(Vec::new()));
        let mut runtime = ViewRuntime::from_component(SwitchOwner {
            node: node.clone(),
            requests: requests.clone(),
            enabled: true,
        })
        .unwrap();
        let node = node.get().unwrap();

        let semantic = runtime.ui().semantics.get(node).unwrap();
        assert_eq!(semantic.role, SemanticRole::Switch);
        assert_eq!(semantic.state.checked, Some(SemanticCheckState::Checked));
        assert!(semantic.actions.contains(SemanticAction::Activate));
        let SemanticName::Text(name) = semantic.name else {
            panic!("switch must expose its stable visible label as its name");
        };
        assert_eq!(runtime.ui().string(name), Some("Wireless"));
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
            .send_component_action(OwnerAction::Publish(false))
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

    #[test]
    fn disabled_switch_has_checked_semantics_without_effective_actions_or_route() {
        let node = Rc::new(Cell::new(None));
        let requests = Rc::new(RefCell::new(Vec::new()));
        let mut runtime = ViewRuntime::from_component(SwitchOwner {
            node: node.clone(),
            requests: requests.clone(),
            enabled: false,
        })
        .unwrap();
        let node = node.get().unwrap();
        let semantic = runtime.ui().semantics.get(node).unwrap();
        assert_eq!(semantic.role, SemanticRole::Switch);
        assert_eq!(semantic.state.checked, Some(SemanticCheckState::Checked));
        assert!(semantic.state.disabled);
        assert!(semantic.effective_actions().is_empty());
        assert!(!runtime.dispatch_activation(node, ChangeSource::Pointer));
        assert!(requests.borrow().is_empty());
    }
}

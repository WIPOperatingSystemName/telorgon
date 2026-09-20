//! Accessible Tier A link action with typed navigation and context-command outputs.

use crate::foundation::{ColorRgba8, EdgeInsets};
use crate::input::ChangeSource;
use crate::runtime::{RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    Background, BoxStyle, ControlHandle, CornerRadii, Property, SemanticActions, SemanticNode,
    SemanticRole, SemanticValue, SizeRule, SizeRule2D, StringId, UiNodeId,
};

use crate::components::application::{
    Button, ButtonBehavior, ButtonError, ButtonInteractionState, ButtonStyleState, DensityMetrics,
};

/// Validated, host-interpreted destination carried by link requests.
///
/// Telorgon deliberately does not classify this string as a URI, route, or file path. The
/// application or navigation owner that receives a request interprets it and applies policy.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LinkDestination(String);

impl LinkDestination {
    pub fn new(value: impl Into<String>) -> Result<Self, LinkDestinationError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(LinkDestinationError::Empty);
        }
        if value.chars().any(char::is_control) {
            return Err(LinkDestinationError::ControlCharacter);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for LinkDestination {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for LinkDestination {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkDestinationError {
    Empty,
    ControlCharacter,
}

impl std::fmt::Display for LinkDestinationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => formatter.write_str("link destination is empty"),
            Self::ControlCharacter => {
                formatter.write_str("link destination contains a control character")
            }
        }
    }
}

impl std::error::Error for LinkDestinationError {}

/// Primary navigation request emitted after canonical activation succeeds.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LinkAction {
    pub destination: LinkDestination,
    pub source: ChangeSource,
}

impl LinkAction {
    pub const fn new(destination: LinkDestination, source: ChangeSource) -> Self {
        Self {
            destination,
            source,
        }
    }
}

/// Context operation offered by a link without performing a platform service inline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LinkCommandKind {
    CopyDestination,
    OpenInNewContext,
}

/// Typed context command for an application command/navigation owner.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LinkCommand {
    pub kind: LinkCommandKind,
    pub destination: LinkDestination,
    pub source: ChangeSource,
}

/// Named visual slots for one resolved link state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinkVisualStyle {
    pub container: BoxStyle,
    pub label_color: ColorRgba8,
    pub label_size: f32,
}

/// Typed application-domain link style with button-family interaction priority.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinkStyle {
    pub resting: LinkVisualStyle,
    pub hovered: Option<LinkVisualStyle>,
    pub focused: Option<LinkVisualStyle>,
    pub pressed: Option<LinkVisualStyle>,
    pub disabled: Option<LinkVisualStyle>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedLinkStyle {
    pub state: ButtonStyleState,
    pub visual: LinkVisualStyle,
}

impl LinkStyle {
    pub const fn resolve(self, state: ButtonInteractionState) -> ResolvedLinkStyle {
        let resolved_state = ButtonStyleState::resolve(state);
        let visual = match resolved_state {
            ButtonStyleState::Disabled => self.disabled,
            ButtonStyleState::Pressed => self.pressed,
            ButtonStyleState::Focused => self.focused,
            ButtonStyleState::Hovered => self.hovered,
            ButtonStyleState::Busy | ButtonStyleState::Resting => Some(self.resting),
        };
        ResolvedLinkStyle {
            state: resolved_state,
            visual: match visual {
                Some(visual) => visual,
                None => self.resting,
            },
        }
    }
}

impl Default for LinkStyle {
    fn default() -> Self {
        let container = BoxStyle {
            min_size: SizeRule2D {
                width: SizeRule::Logical(32.0),
                height: SizeRule::Logical(32.0),
            },
            padding: EdgeInsets {
                top: 5.0,
                right: 4.0,
                bottom: 5.0,
                left: 4.0,
            },
            decoration: crate::ui::BoxDecoration {
                corner_radii: CornerRadii::all(4.0),
                ..crate::ui::BoxDecoration::default()
            },
            ..BoxStyle::default()
        };
        let visual = |label_color, background| LinkVisualStyle {
            container: BoxStyle {
                decoration: crate::ui::BoxDecoration {
                    background,
                    ..crate::ui::BoxDecoration::default()
                },
                ..container
            },
            label_color,
            label_size: 14.0,
        };
        Self {
            resting: visual(ColorRgba8::rgba(77, 139, 255, 255), Background::None),
            hovered: Some(visual(
                ColorRgba8::rgba(111, 163, 255, 255),
                Background::Color(ColorRgba8::rgba(66, 91, 139, 80)),
            )),
            focused: Some(visual(
                ColorRgba8::rgba(129, 174, 255, 255),
                Background::Color(ColorRgba8::rgba(66, 91, 139, 105)),
            )),
            pressed: Some(visual(
                ColorRgba8::rgba(59, 116, 222, 255),
                Background::Color(ColorRgba8::rgba(41, 65, 109, 120)),
            )),
            disabled: Some(visual(
                ColorRgba8::rgba(133, 139, 151, 255),
                Background::None,
            )),
        }
    }
}

/// Immutable configuration for a labelled application link action.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    button: Button,
    destination: LinkDestination,
    style: LinkStyle,
}

impl Link {
    pub fn new(label: impl Into<String>, destination: LinkDestination) -> Result<Self, LinkError> {
        let button = Button::new(label).map_err(LinkError::from)?;
        Ok(Self {
            button,
            destination,
            style: LinkStyle::default(),
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

    pub fn style(mut self, style: LinkStyle) -> Self {
        self.style = style;
        self
    }

    pub fn behavior(&self) -> ButtonBehavior {
        self.button.behavior()
    }

    pub fn destination(&self) -> &LinkDestination {
        &self.destination
    }

    pub fn action(&self, source: ChangeSource) -> LinkAction {
        LinkAction::new(self.destination.clone(), source)
    }

    pub fn context_command(&self, kind: LinkCommandKind, source: ChangeSource) -> LinkCommand {
        LinkCommand {
            kind,
            destination: self.destination.clone(),
            source,
        }
    }

    pub fn semantic_node(
        &self,
        name: StringId,
        destination: StringId,
        state: ButtonInteractionState,
    ) -> SemanticNode {
        let mut semantic = self.button.semantic_node(name, state);
        semantic.role = SemanticRole::Link;
        semantic.value = SemanticValue::Text(destination);
        if state.enabled {
            semantic.actions |= SemanticActions::SHOW_CONTEXT_MENU;
        }
        semantic
    }

    /// Mounts a real foundation action node and routes successful activation as a typed request.
    pub fn mount<Action, Map>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
        map: Map,
    ) -> RuntimeResult<LinkRef>
    where
        Action: 'static,
        Map: Fn(LinkAction) -> Action + 'static,
    {
        let state = self.button.initial_interaction_state();
        let mut visual = self.style.resolve(state).visual;
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
            .ok_or_else(|| RuntimeError::new("application link host is stale"))?;

        let destination_id = ui.foundation().intern(self.destination.as_str());
        self.button
            .attach_mounted_contract_with(ui, control.node, |semantic| {
                semantic.role = SemanticRole::Link;
                semantic.value = SemanticValue::Text(destination_id);
                if state.enabled {
                    semantic.actions |= SemanticActions::SHOW_CONTEXT_MENU;
                }
            })?;

        let destination = self.destination.clone();
        self.button
            .route_mounted_activation(ui, control.node, move |activation| {
                map(LinkAction::new(destination.clone(), activation.source))
            })?;

        Ok(LinkRef { control })
    }
}

/// Focused advanced reference returned by link mounting.
#[derive(Clone, Copy, Debug)]
pub struct LinkRef {
    control: ControlHandle,
}

impl LinkRef {
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
pub enum LinkError {
    MissingAccessibleName,
}

impl From<ButtonError> for LinkError {
    fn from(error: ButtonError) -> Self {
        match error {
            ButtonError::MissingAccessibleName => Self::MissingAccessibleName,
        }
    }
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingAccessibleName => formatter.write_str("link accessible name is empty"),
        }
    }
}

impl std::error::Error for LinkError {}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use crate::input::{ActivationInput, ActivationTransition, PointerButton, PointerId};
    use crate::runtime::{Component, CreateContext, UpdateContext, ViewRuntime};
    use crate::ui::{
        LayoutStyle, SemanticAction, SemanticName, SemanticRole, SemanticValue, UiRoot,
    };

    use crate::components::application::DensityClass;

    use super::*;

    fn destination() -> LinkDestination {
        LinkDestination::new("docs/getting-started").unwrap()
    }

    #[test]
    fn destination_and_accessible_name_validation_are_independent() {
        assert_eq!(
            LinkDestination::new("  ").unwrap_err(),
            LinkDestinationError::Empty
        );
        assert_eq!(
            LinkDestination::new("docs\nsecret").unwrap_err(),
            LinkDestinationError::ControlCharacter
        );
        assert_eq!(
            Link::new(" ", destination()).unwrap_err(),
            LinkError::MissingAccessibleName
        );
    }

    #[test]
    fn headless_behavior_reuses_canonical_activation_and_preserves_destination() {
        let link = Link::new("Read docs", destination()).unwrap();
        let mut behavior = link.behavior();
        let pointer = PointerId::new(7);
        behavior.handle(ActivationInput::PointerDown {
            pointer,
            button: PointerButton::PRIMARY,
        });
        behavior.handle(ActivationInput::PointerMoved {
            pointer,
            inside: false,
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

        assert_eq!(
            link.action(ChangeSource::Keyboard),
            LinkAction::new(destination(), ChangeSource::Keyboard)
        );
    }

    #[test]
    fn semantics_expose_link_name_destination_and_context_capability() {
        let link = Link::new("Read docs", destination()).unwrap();
        let semantic = link.semantic_node(
            StringId(1),
            StringId(2),
            ButtonInteractionState::resting(true, false),
        );
        assert_eq!(semantic.role, SemanticRole::Link);
        assert_eq!(semantic.name, SemanticName::Text(StringId(1)));
        assert_eq!(semantic.value, SemanticValue::Text(StringId(2)));
        assert!(semantic.actions.contains(SemanticAction::Activate));
        assert!(semantic.actions.contains(SemanticAction::ShowContextMenu));

        let disabled = Link::new("Read docs", destination())
            .unwrap()
            .enabled(false)
            .semantic_node(
                StringId(1),
                StringId(2),
                ButtonInteractionState::resting(false, false),
            );
        assert!(disabled.effective_actions().is_empty());
    }

    #[test]
    fn context_operations_are_typed_commands_and_do_not_change_the_primary_action() {
        let link = Link::new("Read docs", destination()).unwrap();
        assert_eq!(
            link.context_command(LinkCommandKind::CopyDestination, ChangeSource::Pointer),
            LinkCommand {
                kind: LinkCommandKind::CopyDestination,
                destination: destination(),
                source: ChangeSource::Pointer,
            }
        );
        assert_eq!(
            link.context_command(
                LinkCommandKind::OpenInNewContext,
                ChangeSource::Accessibility,
            ),
            LinkCommand {
                kind: LinkCommandKind::OpenInNewContext,
                destination: destination(),
                source: ChangeSource::Accessibility,
            }
        );
    }

    struct MountedLink {
        node: Rc<Cell<Option<UiNodeId>>>,
        actions: Rc<RefCell<Vec<LinkAction>>>,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum MountedAction {
        Link(LinkAction),
    }

    impl Component for MountedLink {
        type State = ();
        type Action = MountedAction;

        fn create(&self, _context: &mut CreateContext<'_>) -> Self::State {}

        fn mount(&self, _state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            let link = Link::new("Read docs", destination())
                .unwrap()
                .density(DensityMetrics::baseline(DensityClass::Touch))
                .mount(ui, root.0, MountedAction::Link)
                .unwrap();
            self.node.set(Some(link.node()));
            root
        }

        fn action(
            &self,
            _state: &mut Self::State,
            action: Self::Action,
            _context: &mut UpdateContext<'_, Self>,
        ) {
            let MountedAction::Link(action) = action;
            self.actions.borrow_mut().push(action);
        }
    }

    #[test]
    fn mounted_link_routes_sources_and_mounts_link_semantics_at_touch_density() {
        let node = Rc::new(Cell::new(None));
        let actions = Rc::new(RefCell::new(Vec::new()));
        let mut runtime = ViewRuntime::from_component(MountedLink {
            node: node.clone(),
            actions: actions.clone(),
        })
        .unwrap();
        let node = node.get().unwrap();

        let semantic = runtime.ui().semantics.get(node).unwrap();
        assert_eq!(semantic.role, SemanticRole::Link);
        let SemanticName::Text(name) = semantic.name else {
            panic!("mounted link must expose its visible label as its name");
        };
        assert_eq!(runtime.ui().string(name), Some("Read docs"));
        let SemanticValue::Text(destination_id) = semantic.value else {
            panic!("mounted link must expose destination meaning");
        };
        assert_eq!(
            runtime.ui().string(destination_id),
            Some("docs/getting-started")
        );
        assert_eq!(
            runtime.ui().box_styles.get(node).unwrap().min_size,
            SizeRule2D {
                width: SizeRule::Logical(44.0),
                height: SizeRule::Logical(44.0),
            }
        );

        assert!(runtime.dispatch_activation(node, ChangeSource::Pointer));
        assert!(runtime.dispatch_activation(node, ChangeSource::Accessibility));
        assert!(runtime.dispatch_action(node));
        assert_eq!(
            &*actions.borrow(),
            &[
                LinkAction::new(destination(), ChangeSource::Pointer),
                LinkAction::new(destination(), ChangeSource::Accessibility),
                LinkAction::new(destination(), ChangeSource::Programmatic),
            ]
        );
    }

    #[test]
    fn style_resolves_shared_priority_without_changing_geometry() {
        let style = LinkStyle::default();
        let hovered = style.resolve(ButtonInteractionState {
            hovered: true,
            ..ButtonInteractionState::resting(true, false)
        });
        let pressed = style.resolve(ButtonInteractionState {
            hovered: true,
            pressed: true,
            ..ButtonInteractionState::resting(true, false)
        });
        assert_eq!(hovered.state, ButtonStyleState::Hovered);
        assert_eq!(pressed.state, ButtonStyleState::Pressed);
        assert_eq!(
            hovered.visual.container.min_size,
            pressed.visual.container.min_size
        );
    }
}

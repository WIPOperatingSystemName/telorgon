//! Mounted menu-level command view over the shared menu controller and command owners.
//!
//! This module owns menu rows, semantics, density, and typed input intents. `MenuController`
//! remains the sole owner of highlight and overlay-chain state, while `CommandSpec` remains the
//! sole owner of controlled command state and fresh action construction.

use std::collections::HashSet;
use std::fmt;
use std::hash::Hash;
use std::rc::Rc;

use crate::foundation::{ColorRgba8, EdgeInsets};
use crate::input::{ChangeSource, CompositeChange, CompositeNavigationCommand, WritingDirection};
use crate::runtime::{ComponentId, RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    Background, BoxStyle, ControlHandle, CornerRadii, LayoutStyle, OverlayId, Property,
    SemanticActions, SemanticCheckState, SemanticName, SemanticNode, SemanticRelationship,
    SemanticRelationshipKind, SemanticRole, SemanticState, SizeRule, SizeRule2D, UiNodeId,
};

use super::{
    CommandSpec, MenuActivationDismissal, MenuCommandIntent, MenuController, MenuControllerError,
    MenuLevelState, ResolvedCommandState,
};
use crate::components::application::{
    ApplicationOverlayController, CheckState, DensityClass, DensityMetrics,
};

/// Behavior of one menu row after accepted activation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MenuItemKind {
    #[default]
    Command,
    Submenu,
}

/// One command rendered in a menu level.
pub struct MenuItem<K: 'static, A: 'static> {
    command: CommandSpec<K, A>,
    kind: MenuItemKind,
    dismissal: MenuActivationDismissal,
}

impl<K: 'static, A: 'static> MenuItem<K, A> {
    pub fn command(command: CommandSpec<K, A>) -> Self {
        Self {
            command,
            kind: MenuItemKind::Command,
            dismissal: MenuActivationDismissal::Chain,
        }
    }

    pub fn submenu(command: CommandSpec<K, A>) -> Self {
        Self {
            command,
            kind: MenuItemKind::Submenu,
            dismissal: MenuActivationDismissal::Level,
        }
    }

    pub fn dismissal(mut self, dismissal: MenuActivationDismissal) -> Self {
        self.dismissal = dismissal;
        self
    }

    pub fn command_spec(&self) -> &CommandSpec<K, A> {
        &self.command
    }

    pub const fn kind(&self) -> MenuItemKind {
        self.kind
    }

    pub const fn activation_dismissal(&self) -> MenuActivationDismissal {
        self.dismissal
    }
}

impl<K: Clone + 'static, A: 'static> Clone for MenuItem<K, A> {
    fn clone(&self) -> Self {
        Self {
            command: self.command.clone(),
            kind: self.kind,
            dismissal: self.dismissal,
        }
    }
}

impl<K: fmt::Debug + 'static, A: 'static> fmt::Debug for MenuItem<K, A> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MenuItem")
            .field("command", &self.command)
            .field("kind", &self.kind)
            .field("dismissal", &self.dismissal)
            .finish()
    }
}

/// Typed visual slots for one mounted menu level.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MenuStyle {
    pub container: BoxStyle,
    pub item: BoxStyle,
    pub highlighted_item: BoxStyle,
    pub checked_item: BoxStyle,
    pub label_color: ColorRgba8,
    pub disabled_label_color: ColorRgba8,
    pub indicator_color: ColorRgba8,
    pub label_size: f32,
    pub gap: f32,
}

impl Default for MenuStyle {
    fn default() -> Self {
        let item = BoxStyle {
            padding: EdgeInsets::all(6.0),
            decoration: crate::ui::BoxDecoration {
                corner_radii: CornerRadii::all(4.0),
                ..crate::ui::BoxDecoration::default()
            },
            ..BoxStyle::default()
        };
        Self {
            container: BoxStyle {
                padding: EdgeInsets::all(4.0),
                decoration: crate::ui::BoxDecoration {
                    corner_radii: CornerRadii::all(6.0),
                    background: Background::Color(ColorRgba8::rgba(34, 37, 44, 255)),
                    ..crate::ui::BoxDecoration::default()
                },
                ..BoxStyle::default()
            },
            item,
            highlighted_item: BoxStyle {
                decoration: crate::ui::BoxDecoration {
                    background: Background::Color(ColorRgba8::rgba(66, 91, 139, 180)),
                    ..crate::ui::BoxDecoration::default()
                },
                ..item
            },
            checked_item: BoxStyle {
                decoration: crate::ui::BoxDecoration {
                    background: Background::Color(ColorRgba8::rgba(66, 91, 139, 90)),
                    ..crate::ui::BoxDecoration::default()
                },
                ..item
            },
            label_color: ColorRgba8::rgba(235, 238, 244, 255),
            disabled_label_color: ColorRgba8::rgba(145, 151, 164, 255),
            indicator_color: ColorRgba8::rgba(205, 211, 222, 255),
            label_size: 14.0,
            gap: 2.0,
        }
    }
}

/// Input route emitted by one mounted row. Applying it remains explicit and controller-owned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuRouteRequest<K> {
    Activate {
        command: K,
        source: ChangeSource,
        dismissal: MenuActivationDismissal,
    },
    Submenu {
        parent: OverlayId,
        command: K,
        source: ChangeSource,
    },
}

impl<K: Copy> MenuRouteRequest<K> {
    pub const fn command(self) -> K {
        match self {
            Self::Activate { command, .. } | Self::Submenu { command, .. } => command,
        }
    }

    pub const fn source(self) -> ChangeSource {
        match self {
            Self::Activate { source, .. } | Self::Submenu { source, .. } => source,
        }
    }
}

/// A submenu-opening request after row availability and active-level validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuSubmenuIntent<K> {
    pub parent: OverlayId,
    pub command: K,
    pub source: ChangeSource,
}

/// Result of applying a mounted row route to the existing controller owners.
#[derive(Debug)]
pub enum MenuDispatch<K, A> {
    Command(MenuCommandIntent<K, A>),
    Submenu(MenuSubmenuIntent<K>),
}

/// Caller-owned baseline typeahead match. It does not start a deadline or apply focus itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuTypeaheadIntent<K> {
    pub overlay: OverlayId,
    pub command: K,
    pub query: String,
}

/// Observable result of explicit menu navigation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuNavigation<K> {
    Highlight(CompositeChange<K>),
    Submenu(MenuSubmenuIntent<K>),
    DismissLevel(crate::ui::OverlayCloseOutcome),
    Ignored,
}

/// Immutable mounted menu-level configuration.
pub struct Menu<K: 'static, A: 'static> {
    label: String,
    items: Vec<MenuItem<K, A>>,
    density: DensityMetrics,
    style: MenuStyle,
}

impl<K, A> Menu<K, A>
where
    K: Copy + Eq + Hash + 'static,
    A: 'static,
{
    pub fn new(
        label: impl Into<String>,
        items: impl IntoIterator<Item = MenuItem<K, A>>,
    ) -> Result<Self, MenuError<K>> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(MenuError::MissingAccessibleName);
        }
        let items: Vec<_> = items.into_iter().collect();
        let Some(first) = items.first() else {
            return Err(MenuError::Empty);
        };
        let owner = first.command.owner();
        let mut commands = HashSet::with_capacity(items.len());
        for item in &items {
            let command = *item.command.id();
            if !commands.insert(command) {
                return Err(MenuError::DuplicateCommand(command));
            }
            if item.command.owner() != owner {
                return Err(MenuError::OwnerMismatch {
                    expected: owner,
                    actual: item.command.owner(),
                });
            }
        }
        Ok(Self {
            label,
            items,
            density: DensityMetrics::baseline(DensityClass::Standard),
            style: MenuStyle::default(),
        })
    }

    pub fn density(mut self, density: DensityMetrics) -> Self {
        self.density = density;
        self
    }

    pub fn style(mut self, style: MenuStyle) -> Self {
        self.style = style;
        self
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn items(&self) -> &[MenuItem<K, A>] {
        &self.items
    }

    /// Mounts one level snapshot. The matching `MenuController` remains the live highlight owner.
    pub fn mount<HostAction, Map>(
        &self,
        ui: &mut Ui<'_, '_, HostAction>,
        host: UiNodeId,
        level: MenuLevelState<K>,
        map: Map,
    ) -> RuntimeResult<MenuRef<K, A>>
    where
        HostAction: 'static,
        Map: Fn(MenuRouteRequest<K>) -> HostAction + 'static,
    {
        if level
            .active_command
            .is_some_and(|active| !self.items.iter().any(|item| *item.command.id() == active))
        {
            return Err(RuntimeError::new(
                "active menu command is absent from the mounted level",
            ));
        }

        let mut resolved = Vec::with_capacity(self.items.len());
        for item in &self.items {
            resolved.push(ResolvedMenuItem {
                item: item.clone(),
                state: item.command.resolve_state(ui)?,
            });
        }
        let display: Vec<_> = resolved
            .iter()
            .map(|item| {
                (
                    *item.item.command.id(),
                    item.item.command.label().to_owned(),
                    item.item.command.description_text().map(str::to_owned),
                    item.item.kind,
                    item.item.dismissal,
                    item.state,
                )
            })
            .collect();
        let minimum = self.density.effective_minimum();
        let mut mounted = Vec::with_capacity(display.len());
        let menu = ui
            .foundation()
            .button_node_under(host, self.style.container, |writer| {
                writer.container(
                    BoxStyle::default(),
                    LayoutStyle {
                        gap: self.style.gap,
                        ..LayoutStyle::default()
                    },
                    |writer| {
                        for (command, label, description, kind, dismissal, state) in display {
                            let highlighted = level.active_command == Some(command);
                            let checked = state
                                .checked()
                                .is_some_and(|checked| checked != CheckState::Unchecked);
                            let mut item_style = if highlighted {
                                self.style.highlighted_item
                            } else if checked {
                                self.style.checked_item
                            } else {
                                self.style.item
                            };
                            item_style.min_size = SizeRule2D {
                                width: SizeRule::Logical(minimum.width()),
                                height: SizeRule::Logical(minimum.height()),
                            };
                            let color = if state.enabled() {
                                self.style.label_color
                            } else {
                                self.style.disabled_label_color
                            };
                            let control = writer.action_node(item_style, false, |writer| {
                                writer.text(&label, color, self.style.label_size);
                                if kind == MenuItemKind::Submenu {
                                    writer.text(
                                        "›",
                                        self.style.indicator_color,
                                        self.style.label_size,
                                    );
                                }
                            });
                            mounted.push(MountedMenuItem {
                                command,
                                label,
                                description,
                                kind,
                                dismissal,
                                state,
                                control,
                            });
                        }
                    },
                );
            })
            .ok_or_else(|| RuntimeError::new("application menu host is stale"))?;

        let map: Rc<dyn Fn(MenuRouteRequest<K>) -> HostAction> = Rc::new(map);
        let mut item_refs = Vec::with_capacity(mounted.len());
        for item in &mounted {
            let name = ui.foundation().intern(&item.label);
            let description = item
                .description
                .as_deref()
                .map(|description| ui.foundation().intern(description));
            let checked = item.state.checked().map(check_state_semantic);
            let actions = if !item.state.enabled() {
                SemanticActions::NONE
            } else if item.kind == MenuItemKind::Submenu {
                SemanticActions::EXPAND
            } else {
                SemanticActions::ACTIVATE
            };
            ui.foundation()
                .semantic_node(
                    item.control.node,
                    SemanticNode {
                        role: SemanticRole::MenuItem,
                        name: SemanticName::Text(name),
                        description,
                        state: SemanticState {
                            disabled: !item.state.enabled(),
                            checked,
                            expanded: (item.kind == MenuItemKind::Submenu).then_some(false),
                            ..SemanticState::default()
                        },
                        actions,
                        ..SemanticNode::default()
                    },
                )
                .map_err(|error| {
                    RuntimeError::new(format!("invalid menu item semantics: {error:?}"))
                })?;
            if !item.state.enabled() {
                ui.foundation().disabled(item.control.node, true);
            }
            if checked.is_some_and(|checked| checked != SemanticCheckState::Unchecked) {
                ui.foundation().checked(item.control.node, true);
            }
            if level.active_command == Some(item.command) {
                ui.foundation().highlighted(item.control.node, true);
            }
            if item.state.enabled() {
                let request = route_request(level.overlay, item, ChangeSource::Programmatic);
                let route_map = map.clone();
                ui.route_activation(item.control.node, move |activation| {
                    route_map(with_source(request, activation.source))
                })?;
            }
            item_refs.push(MenuItemRef {
                command: item.command,
                control: item.control,
                kind: item.kind,
                enabled: item.state.enabled(),
                checked: item.state.checked(),
            });
        }

        let name = ui.foundation().intern(&self.label);
        let mut relationships: Vec<_> = item_refs
            .iter()
            .map(|item| SemanticRelationship {
                kind: SemanticRelationshipKind::Owns,
                target: item.control.node,
            })
            .collect();
        if let Some(active) = level.active_command
            && let Some(item) = item_refs.iter().find(|item| item.command == active)
        {
            relationships.push(SemanticRelationship {
                kind: SemanticRelationshipKind::ActiveDescendant,
                target: item.control.node,
            });
        }
        ui.foundation()
            .semantic_node(
                menu.node,
                SemanticNode {
                    role: SemanticRole::Menu,
                    name: SemanticName::Text(name),
                    state: SemanticState {
                        focusable: true,
                        ..SemanticState::default()
                    },
                    actions: SemanticActions::FOCUS,
                    relationships,
                    ..SemanticNode::default()
                },
            )
            .map_err(|error| RuntimeError::new(format!("invalid menu semantics: {error:?}")))?;

        Ok(MenuRef {
            menu,
            overlay: level.overlay,
            items: item_refs,
            commands: Rc::new(resolved),
        })
    }
}

impl<K: Clone + 'static, A: 'static> Clone for Menu<K, A> {
    fn clone(&self) -> Self {
        Self {
            label: self.label.clone(),
            items: self.items.clone(),
            density: self.density,
            style: self.style,
        }
    }
}

impl<K: fmt::Debug + 'static, A: 'static> fmt::Debug for Menu<K, A> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Menu")
            .field("label", &self.label)
            .field("items", &self.items)
            .field("density", &self.density)
            .field("style", &self.style)
            .finish()
    }
}

struct ResolvedMenuItem<K: 'static, A: 'static> {
    item: MenuItem<K, A>,
    state: ResolvedCommandState,
}

struct MountedMenuItem<K> {
    command: K,
    label: String,
    description: Option<String>,
    kind: MenuItemKind,
    dismissal: MenuActivationDismissal,
    state: ResolvedCommandState,
    control: ControlHandle,
}

/// Mounted menu-level handle retaining command snapshots but no duplicate highlight state.
pub struct MenuRef<K: 'static, A: 'static> {
    menu: ControlHandle,
    overlay: OverlayId,
    items: Vec<MenuItemRef<K>>,
    commands: Rc<Vec<ResolvedMenuItem<K, A>>>,
}

impl<K, A> MenuRef<K, A>
where
    K: Copy + Eq + Hash + 'static,
    A: 'static,
{
    pub const fn node(&self) -> UiNodeId {
        self.menu.node
    }

    pub const fn overlay(&self) -> OverlayId {
        self.overlay
    }

    pub fn items(&self) -> &[MenuItemRef<K>] {
        &self.items
    }

    pub const fn style(&self) -> Property<BoxStyle> {
        self.menu.style
    }

    pub fn active_command(&self, controller: &MenuController<K>) -> Option<K> {
        controller
            .level(self.overlay)
            .and_then(|level| level.active_command)
    }

    /// Applies arrows/Home/End through the existing controller. Inline-end opens submenus and
    /// inline-start closes only a submenu level; the mapping mirrors in RTL.
    pub fn navigate(
        &self,
        controller: &mut MenuController<K>,
        overlays: &mut ApplicationOverlayController,
        command: CompositeNavigationCommand,
        direction: WritingDirection,
    ) -> Result<MenuNavigation<K>, MenuInteractionError<K>> {
        self.validate_active_level(controller)?;
        let inline_end = matches!(
            (command, direction),
            (
                CompositeNavigationCommand::Right,
                WritingDirection::LeftToRight
            ) | (
                CompositeNavigationCommand::Left,
                WritingDirection::RightToLeft
            )
        );
        let inline_start = matches!(
            (command, direction),
            (
                CompositeNavigationCommand::Left,
                WritingDirection::LeftToRight
            ) | (
                CompositeNavigationCommand::Right,
                WritingDirection::RightToLeft
            )
        );
        if inline_end {
            let Some(command) = self.active_command(controller) else {
                return Ok(MenuNavigation::Ignored);
            };
            let item = self.item(command)?;
            if item.item.kind == MenuItemKind::Submenu && item.state.enabled() {
                return Ok(MenuNavigation::Submenu(MenuSubmenuIntent {
                    parent: self.overlay,
                    command,
                    source: ChangeSource::Directional,
                }));
            }
            return Ok(MenuNavigation::Ignored);
        }
        if inline_start {
            if controller
                .level(self.overlay)
                .is_some_and(|level| level.parent.is_some())
            {
                return controller
                    .dismiss_level(overlays, crate::ui::DismissReason::Cancelled)
                    .map(MenuNavigation::DismissLevel)
                    .map_err(MenuInteractionError::Controller);
            }
            return Ok(MenuNavigation::Ignored);
        }
        controller
            .navigate(command, direction)
            .map(MenuNavigation::Highlight)
            .map_err(MenuInteractionError::Controller)
    }

    /// Finds the next wrapping, Unicode-lowercased prefix match after the current highlight.
    pub fn typeahead(
        &self,
        controller: &MenuController<K>,
        query: impl Into<String>,
    ) -> Result<MenuTypeaheadIntent<K>, MenuInteractionError<K>> {
        self.validate_level(controller)?;
        let query = query.into();
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Err(MenuInteractionError::EmptyTypeahead);
        }
        if trimmed.chars().any(char::is_control) {
            return Err(MenuInteractionError::InvalidTypeahead);
        }
        let folded = trimmed.to_lowercase();
        let current = self.active_command(controller);
        let start = current
            .and_then(|current| {
                self.commands
                    .iter()
                    .position(|item| *item.item.command.id() == current)
            })
            .map_or(0, |index| index + 1);
        let matched = (0..self.commands.len())
            .map(|offset| (start + offset) % self.commands.len())
            .map(|index| &self.commands[index])
            .find(|item| {
                item.item
                    .command
                    .label()
                    .to_lowercase()
                    .starts_with(&folded)
            })
            .map(|item| *item.item.command.id())
            .ok_or(MenuInteractionError::NoTypeaheadMatch)?;
        Ok(MenuTypeaheadIntent {
            overlay: self.overlay,
            command: matched,
            query: trimmed.to_owned(),
        })
    }

    pub fn apply_typeahead(
        &self,
        controller: &mut MenuController<K>,
        intent: &MenuTypeaheadIntent<K>,
    ) -> Result<CompositeChange<K>, MenuInteractionError<K>> {
        self.validate_active_level(controller)?;
        if intent.overlay != self.overlay {
            return Err(MenuInteractionError::WrongOverlay {
                expected: self.overlay,
                actual: intent.overlay,
            });
        }
        self.item(intent.command)?;
        controller
            .set_highlight(intent.command)
            .map_err(MenuInteractionError::Controller)
    }

    /// Applies a row request. Command close effects complete before a fresh action is returned.
    pub fn dispatch(
        &self,
        controller: &mut MenuController<K>,
        overlays: &mut ApplicationOverlayController,
        request: MenuRouteRequest<K>,
    ) -> Result<MenuDispatch<K, A>, MenuInteractionError<K>> {
        self.validate_active_level(controller)?;
        let command = request.command();
        let item = self.item(command)?;
        if !item.state.enabled() {
            return Err(MenuInteractionError::DisabledCommand(command));
        }
        controller
            .set_highlight(command)
            .map_err(MenuInteractionError::Controller)?;
        match request {
            MenuRouteRequest::Activate {
                source, dismissal, ..
            } => {
                if item.item.kind != MenuItemKind::Command {
                    return Err(MenuInteractionError::ExpectedCommand(command));
                }
                controller
                    .activate(overlays, &item.item.command, item.state, source, dismissal)
                    .map(MenuDispatch::Command)
                    .map_err(MenuInteractionError::Controller)
            }
            MenuRouteRequest::Submenu { parent, source, .. } => {
                if item.item.kind != MenuItemKind::Submenu {
                    return Err(MenuInteractionError::ExpectedSubmenu(command));
                }
                if parent != self.overlay {
                    return Err(MenuInteractionError::WrongOverlay {
                        expected: self.overlay,
                        actual: parent,
                    });
                }
                Ok(MenuDispatch::Submenu(MenuSubmenuIntent {
                    parent,
                    command,
                    source,
                }))
            }
        }
    }

    fn item(&self, command: K) -> Result<&ResolvedMenuItem<K, A>, MenuInteractionError<K>> {
        self.commands
            .iter()
            .find(|item| *item.item.command.id() == command)
            .ok_or(MenuInteractionError::UnknownCommand(command))
    }

    fn validate_level(
        &self,
        controller: &MenuController<K>,
    ) -> Result<(), MenuInteractionError<K>> {
        if controller.level(self.overlay).is_none() {
            Err(MenuInteractionError::MissingLevel(self.overlay))
        } else {
            Ok(())
        }
    }

    fn validate_active_level(
        &self,
        controller: &MenuController<K>,
    ) -> Result<(), MenuInteractionError<K>> {
        self.validate_level(controller)?;
        let actual = controller.active_overlay();
        if actual == Some(self.overlay) {
            Ok(())
        } else {
            Err(MenuInteractionError::InactiveLevel {
                expected: self.overlay,
                actual,
            })
        }
    }
}

impl<K: Clone + 'static, A: 'static> Clone for MenuRef<K, A> {
    fn clone(&self) -> Self {
        Self {
            menu: self.menu,
            overlay: self.overlay,
            items: self.items.clone(),
            commands: self.commands.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MenuItemRef<K> {
    command: K,
    control: ControlHandle,
    kind: MenuItemKind,
    enabled: bool,
    checked: Option<CheckState>,
}

impl<K: Copy> MenuItemRef<K> {
    pub const fn command(self) -> K {
        self.command
    }

    pub const fn node(self) -> UiNodeId {
        self.control.node
    }

    pub const fn kind(self) -> MenuItemKind {
        self.kind
    }

    pub const fn is_enabled(self) -> bool {
        self.enabled
    }

    pub const fn checked(self) -> Option<CheckState> {
        self.checked
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuError<K> {
    MissingAccessibleName,
    Empty,
    DuplicateCommand(K),
    OwnerMismatch {
        expected: ComponentId,
        actual: ComponentId,
    },
}

impl<K: fmt::Debug> fmt::Display for MenuError<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid menu: {self:?}")
    }
}

impl<K: fmt::Debug> std::error::Error for MenuError<K> {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuInteractionError<K> {
    MissingLevel(OverlayId),
    InactiveLevel {
        expected: OverlayId,
        actual: Option<OverlayId>,
    },
    WrongOverlay {
        expected: OverlayId,
        actual: OverlayId,
    },
    UnknownCommand(K),
    DisabledCommand(K),
    ExpectedCommand(K),
    ExpectedSubmenu(K),
    EmptyTypeahead,
    InvalidTypeahead,
    NoTypeaheadMatch,
    Controller(MenuControllerError<K>),
}

impl<K: fmt::Debug> fmt::Display for MenuInteractionError<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "menu interaction failed: {self:?}")
    }
}

impl<K: fmt::Debug> std::error::Error for MenuInteractionError<K> {}

fn route_request<K: Copy>(
    parent: OverlayId,
    item: &MountedMenuItem<K>,
    source: ChangeSource,
) -> MenuRouteRequest<K> {
    match item.kind {
        MenuItemKind::Command => MenuRouteRequest::Activate {
            command: item.command,
            source,
            dismissal: item.dismissal,
        },
        MenuItemKind::Submenu => MenuRouteRequest::Submenu {
            parent,
            command: item.command,
            source,
        },
    }
}

const fn with_source<K: Copy>(
    request: MenuRouteRequest<K>,
    source: ChangeSource,
) -> MenuRouteRequest<K> {
    match request {
        MenuRouteRequest::Activate {
            command, dismissal, ..
        } => MenuRouteRequest::Activate {
            command,
            source,
            dismissal,
        },
        MenuRouteRequest::Submenu {
            parent, command, ..
        } => MenuRouteRequest::Submenu {
            parent,
            command,
            source,
        },
    }
}

const fn check_state_semantic(state: CheckState) -> SemanticCheckState {
    match state {
        CheckState::Unchecked => SemanticCheckState::Unchecked,
        CheckState::Checked => SemanticCheckState::Checked,
        CheckState::Mixed => SemanticCheckState::Mixed,
    }
}

#[cfg(test)]
mod tests;

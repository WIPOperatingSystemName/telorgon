//! Platform-neutral command-palette matching, highlight, and lifecycle coordination.
//!
//! `CommandSpec` remains the command and controlled-state owner, the neutral composite remains the
//! highlight owner, and the application overlay remains the lifecycle/focus owner. This module
//! mounts no rows, runs no search service, applies no focus effect, and enqueues no action.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::fmt;
use std::hash::Hash;

use crate::input::{
    ChangeSource, CompositeChange, CompositeEdgeBehavior, CompositeError, CompositeItem,
    CompositeNavigationCommand, CompositeNavigationPolicy, CompositeOrientation,
    CompositeSelectionBehavior, CompositeStateMachine, DisabledItemPolicy, WritingDirection,
};
use crate::runtime::{ComponentId, RuntimeResult, Ui};
use crate::ui::{
    DismissReason, MountedUi, OutsidePressPolicy, OverlayAnchor, OverlayCloseOutcome,
    OverlayDismissPolicy, OverlayDismissResult, OverlayFocusContainment, OverlayFocusLifecycle,
    OverlayFocusRequest, OverlayFocusRestoration, OverlayId, OverlayInitialFocus, OverlayModality,
    OverlayOpenRequest,
};

use super::{CommandSpec, ResolvedCommandState};
use crate::components::application::{
    ApplicationOverlayCommand, ApplicationOverlayController, ApplicationOverlayControllerError,
    ApplicationOverlayEffect, CheckState,
};

/// Bounded, deterministic query policy. No platform search or locale service is consulted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandPalettePolicy {
    pub max_results: usize,
    pub max_query_chars: usize,
}

impl Default for CommandPalettePolicy {
    fn default() -> Self {
        Self {
            max_results: 20,
            max_query_chars: 256,
        }
    }
}

impl CommandPalettePolicy {
    fn validate(self) -> Result<Self, CommandPaletteError<()>> {
        if self.max_results == 0 {
            return Err(CommandPaletteError::ZeroResultLimit);
        }
        if self.max_query_chars == 0 {
            return Err(CommandPaletteError::ZeroQueryLimit);
        }
        Ok(self)
    }
}

/// Which command metadata field supplied a match.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CommandPaletteMatchField {
    Label,
    Description,
}

/// Match strength inside a metadata field, ordered from strongest to weakest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CommandPaletteMatchKind {
    All,
    Exact,
    Prefix,
    WordPrefix,
    Substring,
}

/// Stable ranking intent exposed to a presenter without exposing normalized private text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CommandPaletteRank {
    pub field: CommandPaletteMatchField,
    pub kind: CommandPaletteMatchKind,
    pub match_offset: usize,
    pub declaration_order: usize,
}

impl Ord for CommandPaletteRank {
    fn cmp(&self, other: &Self) -> Ordering {
        (
            self.kind,
            self.field,
            self.match_offset,
            self.declaration_order,
        )
            .cmp(&(
                other.kind,
                other.field,
                other.match_offset,
                other.declaration_order,
            ))
    }
}

impl PartialOrd for CommandPaletteRank {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// One ranked command and its current parent-controlled state snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandPaletteResult<K> {
    pub command: K,
    pub enabled: bool,
    pub checked: Option<CheckState>,
    pub rank: CommandPaletteRank,
}

/// Lifecycle/focus intent returned after opening the palette overlay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandPaletteOpened<K> {
    pub overlay: OverlayId,
    pub focus: OverlayFocusRequest,
    pub highlighted: Option<K>,
}

/// Accepted invocation after the palette close effect has been produced.
#[derive(Debug, PartialEq, Eq)]
pub struct CommandPaletteInvocation<K, A> {
    command: K,
    action: A,
    source: ChangeSource,
    checked: Option<CheckState>,
    close: OverlayCloseOutcome,
}

impl<K, A> CommandPaletteInvocation<K, A> {
    pub fn command(&self) -> &K {
        &self.command
    }

    pub const fn source(&self) -> ChangeSource {
        self.source
    }

    pub const fn checked(&self) -> Option<CheckState> {
        self.checked
    }

    pub fn close_effect(&self) -> &OverlayCloseOutcome {
        &self.close
    }

    pub fn into_action(self) -> A {
        self.action
    }
}

/// One palette instance coordinating existing command, composite, and overlay owners.
pub struct CommandPalette<K: 'static, A: 'static> {
    accessible_name: String,
    commands: Vec<CommandSpec<K, A>>,
    policy: CommandPalettePolicy,
    query: String,
    resolved: Vec<ResolvedCommandState>,
    results: Vec<CommandPaletteResult<K>>,
    composite: CompositeStateMachine<K>,
    overlay: Option<OverlayId>,
    has_controlled_snapshot: bool,
}

struct RankedPalette<K>
where
    K: Copy + Eq + Hash,
{
    results: Vec<CommandPaletteResult<K>>,
    composite: CompositeStateMachine<K>,
    entry: CompositeChange<K>,
}

impl<K, A> CommandPalette<K, A>
where
    K: Copy + Eq + Hash + 'static,
    A: 'static,
{
    pub fn new(
        accessible_name: impl Into<String>,
        commands: impl IntoIterator<Item = CommandSpec<K, A>>,
    ) -> Result<Self, CommandPaletteError<K>> {
        Self::with_policy(accessible_name, commands, CommandPalettePolicy::default())
    }

    pub fn with_policy(
        accessible_name: impl Into<String>,
        commands: impl IntoIterator<Item = CommandSpec<K, A>>,
        policy: CommandPalettePolicy,
    ) -> Result<Self, CommandPaletteError<K>> {
        let accessible_name = accessible_name.into();
        if accessible_name.trim().is_empty() {
            return Err(CommandPaletteError::MissingAccessibleName);
        }
        let policy = policy.validate().map_err(|error| error.with_key())?;
        let commands: Vec<_> = commands.into_iter().collect();
        if commands.is_empty() {
            return Err(CommandPaletteError::EmptyCommands);
        }
        let owner = commands[0].owner();
        let mut ids = HashSet::with_capacity(commands.len());
        for command in &commands {
            if command.owner() != owner {
                return Err(CommandPaletteError::OwnerMismatch {
                    expected: owner,
                    actual: command.owner(),
                });
            }
            if !ids.insert(*command.id()) {
                return Err(CommandPaletteError::DuplicateCommand(*command.id()));
            }
        }
        Ok(Self {
            accessible_name,
            commands,
            policy,
            query: String::new(),
            resolved: Vec::new(),
            results: Vec::new(),
            composite: CompositeStateMachine::new(palette_navigation_policy()),
            overlay: None,
            has_controlled_snapshot: false,
        })
    }

    pub fn accessible_name(&self) -> &str {
        &self.accessible_name
    }

    pub const fn policy(&self) -> CommandPalettePolicy {
        self.policy
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn commands(&self) -> &[CommandSpec<K, A>] {
        &self.commands
    }

    pub fn results(&self) -> &[CommandPaletteResult<K>] {
        &self.results
    }

    pub fn highlighted_command(&self) -> Option<K> {
        self.composite.active_descendant()
    }

    pub const fn overlay(&self) -> Option<OverlayId> {
        self.overlay
    }

    /// Atomically reads one current controlled snapshot and re-ranks the current query.
    pub fn refresh<HostAction: 'static>(
        &mut self,
        ui: &mut Ui<'_, '_, HostAction>,
    ) -> Result<CompositeChange<K>, CommandPaletteRefreshError<K>> {
        let resolved = self
            .commands
            .iter()
            .map(|command| command.resolve_state(ui))
            .collect::<RuntimeResult<Vec<_>>>()?;
        let ranked = self
            .rank_and_enter(&self.query, &resolved)
            .map_err(CommandPaletteRefreshError::Palette)?;
        self.resolved = resolved;
        self.results = ranked.results;
        self.composite = ranked.composite;
        self.has_controlled_snapshot = true;
        Ok(ranked.entry)
    }

    /// Validates and applies a local query without invoking application or platform search code.
    pub fn set_query(
        &mut self,
        query: impl Into<String>,
    ) -> Result<CompositeChange<K>, CommandPaletteError<K>> {
        if !self.has_controlled_snapshot {
            return Err(CommandPaletteError::ControlledStateUnresolved);
        }
        let query = validate_query(query.into(), self.policy.max_query_chars)?;
        let ranked = self.rank_and_enter(&query, &self.resolved)?;
        self.query = query;
        self.results = ranked.results;
        self.composite = ranked.composite;
        Ok(ranked.entry)
    }

    pub fn navigate(
        &mut self,
        command: CompositeNavigationCommand,
        direction: WritingDirection,
    ) -> Result<CompositeChange<K>, CommandPaletteError<K>> {
        self.composite
            .navigate(command, direction)
            .map_err(CommandPaletteError::Composite)
    }

    pub fn set_highlight(
        &mut self,
        command: K,
    ) -> Result<CompositeChange<K>, CommandPaletteError<K>> {
        self.composite
            .set_active_descendant(command)
            .map_err(CommandPaletteError::Composite)
    }

    /// Opens one modal overlay and returns focus intent; it mounts no palette content.
    pub fn open(
        &mut self,
        overlays: &mut ApplicationOverlayController,
        ui: &MountedUi,
        anchor: OverlayAnchor,
    ) -> Result<CommandPaletteOpened<K>, CommandPaletteError<K>> {
        if !self.has_controlled_snapshot {
            return Err(CommandPaletteError::ControlledStateUnresolved);
        }
        if let Some(overlay) = self.overlay {
            return Err(CommandPaletteError::AlreadyOpen(overlay));
        }
        let restoration = match anchor {
            OverlayAnchor::Node(node) => OverlayFocusRestoration::TargetThenNearest(node),
            OverlayAnchor::Point(_) | OverlayAnchor::Rect(_) => OverlayFocusRestoration::None,
        };
        let request = OverlayOpenRequest {
            anchor,
            parent: None,
            modality: OverlayModality::Modal,
            dismissal: OverlayDismissPolicy {
                escape: true,
                outside_press: OutsidePressPolicy::DismissAndConsume,
                focus_lost: false,
                pointer_departure: false,
            },
            focus: OverlayFocusLifecycle {
                initial: OverlayInitialFocus::FirstFocusable,
                containment: OverlayFocusContainment::Contain,
                restoration,
            },
        };
        let ApplicationOverlayEffect::Opened(opened) = overlays
            .route(ApplicationOverlayCommand::Open { ui, request })
            .map_err(CommandPaletteError::Overlay)?
        else {
            unreachable!("open command has one effect variant")
        };
        self.overlay = Some(opened.id);
        Ok(CommandPaletteOpened {
            overlay: opened.id,
            focus: opened.focus,
            highlighted: self.highlighted_command(),
        })
    }

    /// Produces the overlay owner's typed close/focus effect without applying it.
    pub fn dismiss(
        &mut self,
        overlays: &mut ApplicationOverlayController,
        reason: DismissReason,
    ) -> Result<OverlayCloseOutcome, CommandPaletteError<K>> {
        let overlay = self.overlay.ok_or(CommandPaletteError::NotOpen)?;
        let ApplicationOverlayEffect::Dismissal(result) = overlays
            .route(ApplicationOverlayCommand::Dismiss {
                id: overlay,
                reason,
            })
            .map_err(CommandPaletteError::Overlay)?
        else {
            unreachable!("dismiss command has one effect variant")
        };
        match result {
            OverlayDismissResult::Dismissed(close) => {
                self.observe_close(&close);
                Ok(close)
            }
            OverlayDismissResult::Blocked { .. } => {
                Err(CommandPaletteError::DismissalBlocked { overlay, reason })
            }
        }
    }

    /// Closes first, then creates exactly one fresh moved action for the active enabled result.
    pub fn activate(
        &mut self,
        overlays: &mut ApplicationOverlayController,
        source: ChangeSource,
    ) -> Result<CommandPaletteInvocation<K, A>, CommandPaletteError<K>> {
        if self.overlay.is_none() {
            return Err(CommandPaletteError::NotOpen);
        }
        let request = self
            .composite
            .request_active_selection(source)
            .map_err(CommandPaletteError::Composite)?;
        let index = self
            .commands
            .iter()
            .position(|command| *command.id() == request.key)
            .expect("palette results originate from the validated command list");
        let state = self.resolved[index];
        if !state.enabled() {
            return Err(CommandPaletteError::DisabledCommand(request.key));
        }
        let close = self.dismiss(overlays, DismissReason::Accepted)?;
        let checked = state.checked();
        let action = self.commands[index]
            .invoke(state, source)
            .into_action()
            .expect("enabled palette snapshot was checked before invocation");
        Ok(CommandPaletteInvocation {
            command: request.key,
            action,
            source,
            checked,
            close,
        })
    }

    /// Reconciles a close produced by another overlay-controller route.
    pub fn observe_close(&mut self, close: &OverlayCloseOutcome) {
        if self.overlay.is_some_and(|overlay| {
            close
                .dismissed
                .iter()
                .any(|dismissed| dismissed.id == overlay)
        }) {
            self.overlay = None;
        }
    }

    fn rank_and_enter(
        &self,
        query: &str,
        states: &[ResolvedCommandState],
    ) -> Result<RankedPalette<K>, CommandPaletteError<K>> {
        let normalized = normalize(query);
        let mut results: Vec<_> = self
            .commands
            .iter()
            .zip(states.iter().copied())
            .enumerate()
            .filter_map(|(order, (command, state))| {
                rank_command(command, &normalized, order).map(|rank| CommandPaletteResult {
                    command: *command.id(),
                    enabled: state.enabled(),
                    checked: state.checked(),
                    rank,
                })
            })
            .collect();
        results.sort_by_key(|result| result.rank);
        results.truncate(self.policy.max_results);

        let mut composite = CompositeStateMachine::new(palette_navigation_policy());
        composite
            .update_items(results.iter().map(|result| CompositeItem {
                key: result.command,
                enabled: result.enabled,
            }))
            .map_err(CommandPaletteError::Composite)?;
        let change = composite
            .enter(None)
            .map_err(CommandPaletteError::Composite)?;
        Ok(RankedPalette {
            results,
            composite,
            entry: change,
        })
    }
}

fn rank_command<K: 'static, A: 'static>(
    command: &CommandSpec<K, A>,
    query: &str,
    declaration_order: usize,
) -> Option<CommandPaletteRank> {
    if query.is_empty() {
        return Some(CommandPaletteRank {
            field: CommandPaletteMatchField::Label,
            kind: CommandPaletteMatchKind::All,
            match_offset: 0,
            declaration_order,
        });
    }
    rank_text(
        command.label(),
        query,
        CommandPaletteMatchField::Label,
        declaration_order,
    )
    .or_else(|| {
        command.description_text().and_then(|description| {
            rank_text(
                description,
                query,
                CommandPaletteMatchField::Description,
                declaration_order,
            )
        })
    })
}

fn rank_text(
    text: &str,
    query: &str,
    field: CommandPaletteMatchField,
    declaration_order: usize,
) -> Option<CommandPaletteRank> {
    let text = normalize(text);
    let (kind, match_offset) = if text == query {
        (CommandPaletteMatchKind::Exact, 0)
    } else if text.starts_with(query) {
        (CommandPaletteMatchKind::Prefix, 0)
    } else {
        let offset = text.find(query)?;
        let word_prefix = offset == 0
            || text[..offset]
                .chars()
                .next_back()
                .is_none_or(|character| !character.is_alphanumeric());
        (
            if word_prefix {
                CommandPaletteMatchKind::WordPrefix
            } else {
                CommandPaletteMatchKind::Substring
            },
            offset,
        )
    };
    Some(CommandPaletteRank {
        field,
        kind,
        match_offset,
        declaration_order,
    })
}

fn validate_query<K>(query: String, max_chars: usize) -> Result<String, CommandPaletteError<K>> {
    if query.chars().count() > max_chars {
        return Err(CommandPaletteError::QueryTooLong {
            limit: max_chars,
            actual: query.chars().count(),
        });
    }
    if query.chars().any(char::is_control) {
        return Err(CommandPaletteError::QueryContainsControl);
    }
    Ok(query.trim().to_owned())
}

fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

const fn palette_navigation_policy() -> CompositeNavigationPolicy {
    CompositeNavigationPolicy {
        orientation: CompositeOrientation::Vertical,
        edge_behavior: CompositeEdgeBehavior::Wrap,
        disabled_items: DisabledItemPolicy::Skip,
        selection: CompositeSelectionBehavior::Independent,
    }
}

#[derive(Debug)]
pub enum CommandPaletteRefreshError<K> {
    Runtime(crate::runtime::RuntimeError),
    Palette(CommandPaletteError<K>),
}

impl<K> From<crate::runtime::RuntimeError> for CommandPaletteRefreshError<K> {
    fn from(error: crate::runtime::RuntimeError) -> Self {
        Self::Runtime(error)
    }
}

impl<K: fmt::Debug> fmt::Display for CommandPaletteRefreshError<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Runtime(error) => error.fmt(formatter),
            Self::Palette(error) => error.fmt(formatter),
        }
    }
}

impl<K: fmt::Debug> std::error::Error for CommandPaletteRefreshError<K> {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandPaletteError<K> {
    MissingAccessibleName,
    EmptyCommands,
    ZeroResultLimit,
    ZeroQueryLimit,
    DuplicateCommand(K),
    OwnerMismatch {
        expected: ComponentId,
        actual: ComponentId,
    },
    ControlledStateUnresolved,
    QueryTooLong {
        limit: usize,
        actual: usize,
    },
    QueryContainsControl,
    AlreadyOpen(OverlayId),
    NotOpen,
    DisabledCommand(K),
    DismissalBlocked {
        overlay: OverlayId,
        reason: DismissReason,
    },
    Composite(CompositeError<K>),
    Overlay(ApplicationOverlayControllerError),
}

impl CommandPaletteError<()> {
    fn with_key<K>(self) -> CommandPaletteError<K> {
        match self {
            Self::ZeroResultLimit => CommandPaletteError::ZeroResultLimit,
            Self::ZeroQueryLimit => CommandPaletteError::ZeroQueryLimit,
            _ => unreachable!("policy validation returns only policy errors"),
        }
    }
}

impl<K: fmt::Debug> fmt::Display for CommandPaletteError<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "command palette transition failed: {self:?}")
    }
}

impl<K: fmt::Debug> std::error::Error for CommandPaletteError<K> {}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use crate::runtime::{
        Component, ComponentRuntimeDriver, CreateContext, State, UpdateContext, ViewRuntime,
    };
    use crate::ui::{BoxStyle, LayoutStyle, OverlayFocusRequest, UiNodeId, UiRoot};

    use crate::components::application::ActionFactory;

    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    struct NonCloneAction {
        command: u32,
        source: ChangeSource,
    }

    struct Fixture {
        palette: Rc<RefCell<Option<CommandPalette<u32, NonCloneAction>>>>,
        overlays: Rc<RefCell<ApplicationOverlayController>>,
        anchor: Rc<Cell<Option<UiNodeId>>>,
    }

    struct FixtureState {
        _enabled: State<bool>,
        _disabled: State<bool>,
    }

    impl Component for Fixture {
        type State = FixtureState;
        type Action = ();

        fn create(&self, context: &mut CreateContext<'_>) -> Self::State {
            let enabled = context.state(true);
            let disabled = context.state(false);
            let owner = context.component();
            let command = |id, label, description: Option<&str>, availability| {
                let command = CommandSpec::new(
                    id,
                    label,
                    availability,
                    ActionFactory::new(owner, move |source| NonCloneAction {
                        command: id,
                        source,
                    }),
                )
                .unwrap();
                match description {
                    Some(description) => command.description(description).unwrap(),
                    None => command,
                }
            };
            self.palette.replace(Some(
                CommandPalette::with_policy(
                    "Commands",
                    [
                        command(1, "Open File", Some("Choose a document"), enabled.read()),
                        command(2, "Open Folder", None, disabled.read()),
                        command(3, "Reopen Closed", None, enabled.read()),
                        command(4, "Inspect", Some("Open developer tools"), enabled.read()),
                    ],
                    CommandPalettePolicy {
                        max_results: 3,
                        max_query_chars: 32,
                    },
                )
                .unwrap(),
            ));
            FixtureState {
                _enabled: enabled,
                _disabled: disabled,
            }
        }

        fn mount(&self, _state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            self.palette
                .borrow_mut()
                .as_mut()
                .unwrap()
                .refresh(ui)
                .unwrap();
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            self.overlays.borrow_mut().mount(ui, root.0).unwrap();
            self.anchor.set(Some(root.0));
            root
        }

        fn action(
            &self,
            _state: &mut Self::State,
            _action: Self::Action,
            _context: &mut UpdateContext<'_, Self>,
        ) {
        }
    }

    struct Harness {
        runtime: ViewRuntime<ComponentRuntimeDriver<Fixture>>,
        palette: Rc<RefCell<Option<CommandPalette<u32, NonCloneAction>>>>,
        overlays: Rc<RefCell<ApplicationOverlayController>>,
        anchor: UiNodeId,
    }

    fn harness() -> Harness {
        let palette = Rc::new(RefCell::new(None));
        let overlays = Rc::new(RefCell::new(ApplicationOverlayController::new()));
        let anchor = Rc::new(Cell::new(None));
        let runtime = ViewRuntime::from_component(Fixture {
            palette: palette.clone(),
            overlays: overlays.clone(),
            anchor: anchor.clone(),
        })
        .unwrap();
        Harness {
            runtime,
            palette,
            overlays,
            anchor: anchor.get().unwrap(),
        }
    }

    #[test]
    fn query_ranking_is_deterministic_and_bounded() {
        let harness = harness();
        let mut borrowed = harness.palette.borrow_mut();
        let palette = borrowed.as_mut().unwrap();
        palette.set_query("open").unwrap();
        assert_eq!(
            palette
                .results()
                .iter()
                .map(|result| (result.command, result.enabled, result.rank.kind))
                .collect::<Vec<_>>(),
            vec![
                (1, true, CommandPaletteMatchKind::Prefix),
                (2, false, CommandPaletteMatchKind::Prefix),
                (4, true, CommandPaletteMatchKind::Prefix),
            ]
        );
        assert_eq!(palette.highlighted_command(), Some(1));
        assert_eq!(
            palette.set_query("open\nfile"),
            Err(CommandPaletteError::QueryContainsControl)
        );
    }

    #[test]
    fn navigation_skips_disabled_results_and_home_end_are_owned_by_composite() {
        let harness = harness();
        let mut borrowed = harness.palette.borrow_mut();
        let palette = borrowed.as_mut().unwrap();
        palette.set_query("open").unwrap();
        palette
            .navigate(
                CompositeNavigationCommand::Down,
                WritingDirection::LeftToRight,
            )
            .unwrap();
        assert_eq!(palette.highlighted_command(), Some(4));
        palette
            .navigate(
                CompositeNavigationCommand::Home,
                WritingDirection::LeftToRight,
            )
            .unwrap();
        assert_eq!(palette.highlighted_command(), Some(1));
        assert!(matches!(
            palette.set_highlight(2),
            Err(CommandPaletteError::Composite(
                CompositeError::ItemNotNavigable(2)
            ))
        ));
    }

    #[test]
    fn modal_open_and_dismiss_return_focus_effects_without_mounting_rows() {
        let harness = harness();
        let node_count = harness.runtime.ui().nodes.alive().len();
        let mut borrowed = harness.palette.borrow_mut();
        let palette = borrowed.as_mut().unwrap();
        let opened = palette
            .open(
                &mut harness.overlays.borrow_mut(),
                harness.runtime.ui(),
                OverlayAnchor::Node(harness.anchor),
            )
            .unwrap();
        assert_eq!(
            opened.focus,
            OverlayFocusRequest::Initial(OverlayInitialFocus::FirstFocusable)
        );
        assert!(harness.overlays.borrow().state().background_is_inert);
        assert_eq!(harness.runtime.ui().nodes.alive().len(), node_count);

        let close = palette
            .dismiss(&mut harness.overlays.borrow_mut(), DismissReason::Escape)
            .unwrap();
        assert_eq!(close.dismissed[0].id, opened.overlay);
        assert!(matches!(
            close.focus,
            OverlayFocusRequest::Restore {
                target,
                nearest_fallback: true,
            } if target == harness.anchor
        ));
        assert_eq!(palette.overlay(), None);
    }

    #[test]
    fn activation_closes_before_returning_fresh_nonclone_action() {
        let harness = harness();
        let mut borrowed = harness.palette.borrow_mut();
        let palette = borrowed.as_mut().unwrap();
        palette.set_query("reopen").unwrap();
        let opened = palette
            .open(
                &mut harness.overlays.borrow_mut(),
                harness.runtime.ui(),
                OverlayAnchor::Node(harness.anchor),
            )
            .unwrap();
        let intent = palette
            .activate(&mut harness.overlays.borrow_mut(), ChangeSource::Keyboard)
            .unwrap();
        assert_eq!(intent.command(), &3);
        assert_eq!(intent.source(), ChangeSource::Keyboard);
        assert_eq!(intent.close_effect().dismissed[0].id, opened.overlay);
        assert_eq!(harness.overlays.borrow().state().entry_count, 0);
        assert_eq!(
            intent.into_action(),
            NonCloneAction {
                command: 3,
                source: ChangeSource::Keyboard,
            }
        );
    }
}

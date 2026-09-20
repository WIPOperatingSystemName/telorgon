//! Reusable command identity, metadata, controlled state, and fresh action construction.

use std::fmt;
use std::rc::Rc;

use crate::runtime::{ComponentId, Read, RuntimeResult, Ui};

use crate::components::application::{ChangeSource, CheckState, IconArtwork, ShortcutSet};

/// Owner-scoped callable that constructs one fresh action for each accepted invocation.
///
/// The factory is repeatable without requiring `A: Clone`. Constructing or cloning this value
/// never calls the supplied callback.
pub struct ActionFactory<A: 'static> {
    owner: ComponentId,
    create: Rc<dyn Fn(ChangeSource) -> A>,
}

impl<A: 'static> ActionFactory<A> {
    pub fn new(owner: ComponentId, create: impl Fn(ChangeSource) -> A + 'static) -> Self {
        Self {
            owner,
            create: Rc::new(create),
        }
    }

    pub const fn owner(&self) -> ComponentId {
        self.owner
    }

    fn create(&self, source: ChangeSource) -> A {
        (self.create)(source)
    }
}

impl<A: 'static> Clone for ActionFactory<A> {
    fn clone(&self) -> Self {
        Self {
            owner: self.owner,
            create: self.create.clone(),
        }
    }
}

impl<A: 'static> fmt::Debug for ActionFactory<A> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActionFactory")
            .field("owner", &self.owner)
            .finish_non_exhaustive()
    }
}

/// Stable command identity and presenter-neutral metadata with controlled state handles.
pub struct CommandSpec<Id: 'static, A: 'static> {
    id: Id,
    label: String,
    description: Option<String>,
    icon: Option<IconArtwork>,
    enabled: Read<bool>,
    checked: Option<Read<CheckState>>,
    shortcuts: ShortcutSet,
    invoke: ActionFactory<A>,
}

impl<Id: 'static, A: 'static> CommandSpec<Id, A> {
    pub fn new(
        id: Id,
        label: impl Into<String>,
        enabled: Read<bool>,
        invoke: ActionFactory<A>,
    ) -> Result<Self, CommandModelError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(CommandModelError::MissingLabel);
        }
        validate_owner(enabled.owner(), invoke.owner(), CommandOwnerField::Enabled)?;
        Ok(Self {
            id,
            label,
            description: None,
            icon: None,
            enabled,
            checked: None,
            shortcuts: ShortcutSet::default(),
            invoke,
        })
    }

    pub fn description(
        mut self,
        description: impl Into<String>,
    ) -> Result<Self, CommandModelError> {
        let description = description.into();
        if description.trim().is_empty() {
            return Err(CommandModelError::MissingDescription);
        }
        self.description = Some(description);
        Ok(self)
    }

    pub fn icon(mut self, icon: IconArtwork) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn checked(mut self, checked: Read<CheckState>) -> Result<Self, CommandModelError> {
        validate_owner(
            checked.owner(),
            self.invoke.owner(),
            CommandOwnerField::Checked,
        )?;
        self.checked = Some(checked);
        Ok(self)
    }

    pub fn shortcuts(mut self, shortcuts: ShortcutSet) -> Self {
        self.shortcuts = shortcuts;
        self
    }

    pub fn id(&self) -> &Id {
        &self.id
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn description_text(&self) -> Option<&str> {
        self.description.as_deref()
    }

    pub const fn icon_artwork(&self) -> Option<IconArtwork> {
        self.icon
    }

    pub const fn enabled(&self) -> Read<bool> {
        self.enabled
    }

    pub const fn checked_state(&self) -> Option<Read<CheckState>> {
        self.checked
    }

    pub fn shortcut_set(&self) -> &ShortcutSet {
        &self.shortcuts
    }

    pub const fn owner(&self) -> ComponentId {
        self.invoke.owner()
    }

    /// Reads the current controlled availability and optional check state during mount.
    pub fn resolve_state<HostAction: 'static>(
        &self,
        ui: &mut Ui<'_, '_, HostAction>,
    ) -> RuntimeResult<ResolvedCommandState> {
        Ok(ResolvedCommandState {
            enabled: ui.read(self.enabled)?,
            checked: match self.checked {
                Some(checked) => Some(ui.read(checked)?),
                None => None,
            },
        })
    }

    /// Constructs exactly one action when the supplied controlled snapshot is enabled.
    pub fn invoke(
        &self,
        state: ResolvedCommandState,
        source: ChangeSource,
    ) -> CommandInvocation<A> {
        if state.enabled {
            CommandInvocation::Invoked {
                action: self.invoke.create(source),
                source,
                checked: state.checked,
            }
        } else {
            CommandInvocation::Disabled {
                source,
                checked: state.checked,
            }
        }
    }
}

impl<Id: Clone + 'static, A: 'static> Clone for CommandSpec<Id, A> {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            label: self.label.clone(),
            description: self.description.clone(),
            icon: self.icon,
            enabled: self.enabled,
            checked: self.checked,
            shortcuts: self.shortcuts.clone(),
            invoke: self.invoke.clone(),
        }
    }
}

impl<Id: fmt::Debug + 'static, A: 'static> fmt::Debug for CommandSpec<Id, A> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CommandSpec")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("description", &self.description)
            .field("icon", &self.icon)
            .field("enabled", &self.enabled)
            .field("checked", &self.checked)
            .field("shortcuts", &self.shortcuts)
            .field("invoke", &self.invoke)
            .finish()
    }
}

/// One current snapshot of the command's parent-controlled state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ResolvedCommandState {
    enabled: bool,
    checked: Option<CheckState>,
}

impl ResolvedCommandState {
    pub const fn new(enabled: bool, checked: Option<CheckState>) -> Self {
        Self { enabled, checked }
    }

    pub const fn enabled(self) -> bool {
        self.enabled
    }

    pub const fn checked(self) -> Option<CheckState> {
        self.checked
    }
}

/// Typed result of attempting to invoke a resolved command.
#[derive(Debug, PartialEq, Eq)]
pub enum CommandInvocation<A> {
    Invoked {
        action: A,
        source: ChangeSource,
        checked: Option<CheckState>,
    },
    Disabled {
        source: ChangeSource,
        checked: Option<CheckState>,
    },
}

impl<A> CommandInvocation<A> {
    pub const fn source(&self) -> ChangeSource {
        match self {
            Self::Invoked { source, .. } | Self::Disabled { source, .. } => *source,
        }
    }

    pub const fn checked(&self) -> Option<CheckState> {
        match self {
            Self::Invoked { checked, .. } | Self::Disabled { checked, .. } => *checked,
        }
    }

    pub const fn is_invoked(&self) -> bool {
        matches!(self, Self::Invoked { .. })
    }

    pub fn into_action(self) -> Option<A> {
        match self {
            Self::Invoked { action, .. } => Some(action),
            Self::Disabled { .. } => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CommandOwnerField {
    Enabled,
    Checked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandModelError {
    MissingLabel,
    MissingDescription,
    OwnerMismatch {
        field: CommandOwnerField,
        expected: ComponentId,
        actual: ComponentId,
    },
}

impl fmt::Display for CommandModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingLabel => formatter.write_str("command label is empty"),
            Self::MissingDescription => formatter.write_str("command description is empty"),
            Self::OwnerMismatch {
                field,
                expected,
                actual,
            } => write!(
                formatter,
                "command {field:?} read belongs to {actual:?}, expected owner {expected:?}"
            ),
        }
    }
}

impl std::error::Error for CommandModelError {}

fn validate_owner(
    actual: ComponentId,
    expected: ComponentId,
    field: CommandOwnerField,
) -> Result<(), CommandModelError> {
    if actual == expected {
        Ok(())
    } else {
        Err(CommandModelError::OwnerMismatch {
            field,
            expected,
            actual,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use crate::runtime::{
        Component, ComponentRuntimeDriver, CreateContext, NoAction, State, UpdateContext,
        ViewRuntime,
    };
    use crate::ui::{BoxStyle, ImageId, LayoutStyle, UiRoot};

    use super::*;

    struct ReadCapture<T: Clone + 'static> {
        initial: T,
        captured: Rc<Cell<Option<Read<T>>>>,
    }

    impl<T: Clone + 'static> Component for ReadCapture<T> {
        type State = State<T>;
        type Action = NoAction;

        fn create(&self, context: &mut CreateContext<'_>) -> Self::State {
            let state = context.state(self.initial.clone());
            self.captured.set(Some(state.read()));
            state
        }

        fn mount(&self, _state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            ui.foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {})
        }

        fn action(
            &self,
            _state: &mut Self::State,
            action: Self::Action,
            _context: &mut UpdateContext<'_, Self>,
        ) {
            match action {}
        }
    }

    fn capture_read<T: Clone + 'static>(
        initial: T,
    ) -> (Read<T>, ViewRuntime<ComponentRuntimeDriver<ReadCapture<T>>>) {
        let captured = Rc::new(Cell::new(None));
        let runtime = ViewRuntime::from_component(ReadCapture {
            initial,
            captured: captured.clone(),
        })
        .unwrap();
        (captured.get().unwrap(), runtime)
    }

    #[test]
    fn validates_metadata_and_preserves_typed_identity() {
        let (enabled, _runtime) = capture_read(true);
        let factory = ActionFactory::new(enabled.owner(), |_| 1_u8);
        assert_eq!(
            CommandSpec::new("save", " ", enabled, factory.clone()).unwrap_err(),
            CommandModelError::MissingLabel
        );
        let icon = IconArtwork::from_image(ImageId(7));
        let command = CommandSpec::new("save", "Save", enabled, factory)
            .unwrap()
            .description("Persist the document")
            .unwrap()
            .icon(icon);
        assert_eq!(command.id(), &"save");
        assert_eq!(command.label(), "Save");
        assert_eq!(command.description_text(), Some("Persist the document"));
        assert_eq!(command.icon_artwork(), Some(icon));
        assert_eq!(
            command.clone().description(" ").unwrap_err(),
            CommandModelError::MissingDescription
        );
    }

    #[test]
    fn rejects_controlled_reads_from_another_owner() {
        let (enabled, _enabled_runtime) = capture_read(true);
        let (foreign, _foreign_runtime) = capture_read(CheckState::Checked);
        let foreign_factory = ActionFactory::new(foreign.owner(), |_| ());
        assert!(matches!(
            CommandSpec::new("save", "Save", enabled, foreign_factory),
            Err(CommandModelError::OwnerMismatch {
                field: CommandOwnerField::Enabled,
                ..
            })
        ));

        let local_factory = ActionFactory::new(enabled.owner(), |_| ());
        let command = CommandSpec::new("save", "Save", enabled, local_factory).unwrap();
        assert!(matches!(
            command.checked(foreign),
            Err(CommandModelError::OwnerMismatch {
                field: CommandOwnerField::Checked,
                ..
            })
        ));
    }

    #[derive(Debug, PartialEq, Eq)]
    struct NonCloneAction {
        sequence: usize,
        source: ChangeSource,
    }

    #[test]
    fn factory_is_lazy_repeatable_and_does_not_require_clone_actions() {
        let (enabled, _runtime) = capture_read(true);
        let calls = Rc::new(Cell::new(0));
        let callback_calls = calls.clone();
        let factory = ActionFactory::new(enabled.owner(), move |source| {
            let sequence = callback_calls.get() + 1;
            callback_calls.set(sequence);
            NonCloneAction { sequence, source }
        });
        let command = CommandSpec::new("run", "Run", enabled, factory).unwrap();
        let cloned = command.clone();
        assert_eq!(calls.get(), 0);

        let first = command
            .invoke(
                ResolvedCommandState::new(true, None),
                ChangeSource::Keyboard,
            )
            .into_action()
            .unwrap();
        let second = cloned
            .invoke(ResolvedCommandState::new(true, None), ChangeSource::Pointer)
            .into_action()
            .unwrap();
        assert_eq!(first.sequence, 1);
        assert_eq!(first.source, ChangeSource::Keyboard);
        assert_eq!(second.sequence, 2);
        assert_eq!(second.source, ChangeSource::Pointer);
    }

    #[test]
    fn disabled_snapshot_suppresses_action_construction_and_preserves_context() {
        let (enabled, _runtime) = capture_read(false);
        let calls = Rc::new(Cell::new(0));
        let callback_calls = calls.clone();
        let command = CommandSpec::new(
            "bold",
            "Bold",
            enabled,
            ActionFactory::new(enabled.owner(), move |_| {
                callback_calls.set(callback_calls.get() + 1)
            }),
        )
        .unwrap();
        let outcome = command.invoke(
            ResolvedCommandState::new(false, Some(CheckState::Mixed)),
            ChangeSource::Accessibility,
        );
        assert!(!outcome.is_invoked());
        assert_eq!(outcome.source(), ChangeSource::Accessibility);
        assert_eq!(outcome.checked(), Some(CheckState::Mixed));
        assert_eq!(calls.get(), 0);
    }

    struct MountedResolution {
        resolved: Rc<RefCell<Option<ResolvedCommandState>>>,
    }

    struct MountedResolutionState {
        command: CommandSpec<&'static str, ()>,
        _enabled: State<bool>,
        _checked: State<CheckState>,
    }

    impl Component for MountedResolution {
        type State = MountedResolutionState;
        type Action = NoAction;

        fn create(&self, context: &mut CreateContext<'_>) -> Self::State {
            let enabled = context.state(true);
            let checked = context.state(CheckState::Mixed);
            let command = CommandSpec::new(
                "selection",
                "Selection",
                enabled.read(),
                ActionFactory::new(context.component(), |_| ()),
            )
            .unwrap()
            .checked(checked.read())
            .unwrap();
            MountedResolutionState {
                command,
                _enabled: enabled,
                _checked: checked,
            }
        }

        fn mount(&self, state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            self.resolved
                .replace(Some(state.command.resolve_state(ui).unwrap()));
            ui.foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {})
        }

        fn action(
            &self,
            _state: &mut Self::State,
            action: Self::Action,
            _context: &mut UpdateContext<'_, Self>,
        ) {
            match action {}
        }
    }

    #[test]
    fn mounted_resolution_reads_controlled_availability_and_check_state() {
        let resolved = Rc::new(RefCell::new(None));
        let _runtime = ViewRuntime::from_component(MountedResolution {
            resolved: resolved.clone(),
        })
        .unwrap();
        assert_eq!(
            *resolved.borrow(),
            Some(ResolvedCommandState::new(true, Some(CheckState::Mixed)))
        );
    }
}

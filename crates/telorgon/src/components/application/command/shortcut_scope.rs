//! Application command declarations over the shared neutral shortcut matcher.

use std::fmt;
use std::hash::Hash;

use crate::input::{
    ActiveShortcutScope, KeyEvent, ShortcutBinding, ShortcutChord, ShortcutDiagnostics,
    ShortcutError, ShortcutMatcher, ShortcutRepeatPolicy, ShortcutResolution, ShortcutScopeId,
};
use crate::runtime::{Read, RuntimeError, Ui};

use crate::components::application::CommandSpec;

/// Caller-supplied localized presentation of one effective shortcut binding.
///
/// This is deliberately separate from [`ShortcutChord`], which remains physical matcher input.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ShortcutDisplayBinding(String);

impl ShortcutDisplayBinding {
    pub fn new(label: impl Into<String>) -> Result<Self, ShortcutDisplayBindingError> {
        let label = label.into();
        if label.trim().is_empty() {
            Err(ShortcutDisplayBindingError::Empty)
        } else {
            Ok(Self(label))
        }
    }

    pub fn label(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ShortcutDisplayBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShortcutDisplayBindingError {
    Empty,
}

impl fmt::Display for ShortcutDisplayBindingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("shortcut display binding is empty")
    }
}

impl std::error::Error for ShortcutDisplayBindingError {}

/// One presenter-neutral shortcut declared by a reusable command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandShortcut {
    chord: ShortcutChord,
    display: ShortcutDisplayBinding,
    priority: i16,
    repeat: ShortcutRepeatPolicy,
}

impl CommandShortcut {
    pub const fn new(chord: ShortcutChord, display: ShortcutDisplayBinding) -> Self {
        Self {
            chord,
            display,
            priority: 0,
            repeat: ShortcutRepeatPolicy::Suppress,
        }
    }

    pub const fn priority(mut self, priority: i16) -> Self {
        self.priority = priority;
        self
    }

    pub const fn repeat(mut self, repeat: ShortcutRepeatPolicy) -> Self {
        self.repeat = repeat;
        self
    }

    pub const fn chord(&self) -> ShortcutChord {
        self.chord
    }

    pub const fn priority_value(&self) -> i16 {
        self.priority
    }

    pub const fn repeat_policy(&self) -> ShortcutRepeatPolicy {
        self.repeat
    }

    pub const fn display_binding(&self) -> &ShortcutDisplayBinding {
        &self.display
    }
}

/// Validated alternative bindings for one command.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShortcutSet {
    shortcuts: Vec<CommandShortcut>,
}

impl ShortcutSet {
    pub fn new(
        shortcuts: impl IntoIterator<Item = CommandShortcut>,
    ) -> Result<Self, ShortcutSetError> {
        let shortcuts: Vec<_> = shortcuts.into_iter().collect();
        for (index, shortcut) in shortcuts.iter().enumerate() {
            if shortcuts[..index]
                .iter()
                .any(|prior| prior.chord == shortcut.chord)
            {
                return Err(ShortcutSetError::DuplicateChord(shortcut.chord));
            }
        }
        Ok(Self { shortcuts })
    }

    pub fn single(shortcut: CommandShortcut) -> Self {
        Self {
            shortcuts: vec![shortcut],
        }
    }

    pub fn len(&self) -> usize {
        self.shortcuts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.shortcuts.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<&CommandShortcut> {
        self.shortcuts.get(index)
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &CommandShortcut> {
        self.shortcuts.iter()
    }
}

impl IntoIterator for ShortcutSet {
    type Item = CommandShortcut;
    type IntoIter = std::vec::IntoIter<CommandShortcut>;

    fn into_iter(self) -> Self::IntoIter {
        self.shortcuts.into_iter()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShortcutSetError {
    DuplicateChord(ShortcutChord),
}

impl fmt::Display for ShortcutSetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateChord(chord) => {
                write!(formatter, "command shortcut set repeats chord {chord:?}")
            }
        }
    }
}

impl std::error::Error for ShortcutSetError {}

/// One command shortcut bound to a concrete scope and controlled availability read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandShortcutRegistration<K, C: 'static> {
    key: K,
    command: C,
    scope: ShortcutScopeId,
    enabled: Read<bool>,
    shortcut: CommandShortcut,
}

impl<K, C: 'static> CommandShortcutRegistration<K, C> {
    pub fn key(&self) -> &K {
        &self.key
    }

    pub fn command(&self) -> &C {
        &self.command
    }

    pub const fn scope(&self) -> ShortcutScopeId {
        self.scope
    }

    pub const fn enabled(&self) -> Read<bool> {
        self.enabled
    }

    pub const fn shortcut(&self) -> &CommandShortcut {
        &self.shortcut
    }

    /// Resolves this registration's current controlled availability during mount.
    pub fn resolve<HostAction: 'static>(
        self,
        ui: &mut Ui<'_, '_, HostAction>,
    ) -> Result<ResolvedCommandShortcut<K, C>, RuntimeError> {
        let enabled = ui.read(self.enabled)?;
        Ok(ResolvedCommandShortcut {
            key: self.key,
            command: self.command,
            scope: self.scope,
            enabled,
            shortcut: self.shortcut,
        })
    }
}

impl<C: Copy + 'static, A: 'static> CommandSpec<C, A> {
    /// Binds one declared shortcut to a concrete registration generation and active-scope ID.
    pub fn shortcut_registration<K>(
        &self,
        key: K,
        scope: ShortcutScopeId,
        shortcut_index: usize,
    ) -> Result<CommandShortcutRegistration<K, C>, CommandShortcutRegistrationError> {
        let Some(shortcut) = self.shortcut_set().get(shortcut_index) else {
            return Err(CommandShortcutRegistrationError::MissingShortcut {
                index: shortcut_index,
                len: self.shortcut_set().len(),
            });
        };
        Ok(CommandShortcutRegistration {
            key,
            command: *self.id(),
            scope,
            enabled: self.enabled(),
            shortcut: shortcut.clone(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandShortcutRegistrationError {
    MissingShortcut { index: usize, len: usize },
}

impl fmt::Display for CommandShortcutRegistrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingShortcut { index, len } => write!(
                formatter,
                "command shortcut index {index} is outside the declared set of length {len}"
            ),
        }
    }
}

impl std::error::Error for CommandShortcutRegistrationError {}

/// One registration after its controlled availability has been resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedCommandShortcut<K, C> {
    key: K,
    command: C,
    scope: ShortcutScopeId,
    enabled: bool,
    shortcut: CommandShortcut,
}

impl<K, C> ResolvedCommandShortcut<K, C> {
    pub const fn new(
        key: K,
        command: C,
        scope: ShortcutScopeId,
        enabled: bool,
        shortcut: CommandShortcut,
    ) -> Self {
        Self {
            key,
            command,
            scope,
            enabled,
            shortcut,
        }
    }

    pub fn key(&self) -> &K {
        &self.key
    }

    pub fn command(&self) -> &C {
        &self.command
    }

    pub const fn scope(&self) -> ShortcutScopeId {
        self.scope
    }

    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    pub const fn shortcut(&self) -> &CommandShortcut {
        &self.shortcut
    }
}

/// Application metadata attached to one unchanged neutral matcher result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandShortcutOutcome<K, C> {
    resolution: ShortcutResolution<K, C>,
    display: Option<ShortcutDisplayBinding>,
}

impl<K, C> CommandShortcutOutcome<K, C> {
    pub const fn resolution(&self) -> &ShortcutResolution<K, C> {
        &self.resolution
    }

    pub const fn display_binding(&self) -> Option<&ShortcutDisplayBinding> {
        self.display.as_ref()
    }

    pub fn into_resolution(self) -> ShortcutResolution<K, C> {
        self.resolution
    }
}

/// Application command shortcut adapter. Matching remains owned by [`ShortcutMatcher`].
#[derive(Clone, Debug)]
pub struct CommandShortcutScope<K, C> {
    matcher: ShortcutMatcher<K, C>,
    displays: Vec<(K, ShortcutDisplayBinding)>,
}

impl<K, C> Default for CommandShortcutScope<K, C> {
    fn default() -> Self {
        Self {
            matcher: ShortcutMatcher::default(),
            displays: Vec::new(),
        }
    }
}

impl<K, C> CommandShortcutScope<K, C>
where
    K: Copy + Eq + Hash,
    C: Copy,
{
    pub fn new() -> Self {
        Self::default()
    }

    pub fn binding_count(&self) -> usize {
        self.matcher.binding_count()
    }

    pub fn diagnostics(&self) -> ShortcutDiagnostics {
        self.matcher.diagnostics()
    }

    /// Atomically replaces bindings from already-resolved command availability snapshots.
    pub fn update_resolved(
        &mut self,
        bindings: impl IntoIterator<Item = ResolvedCommandShortcut<K, C>>,
    ) -> Result<(), CommandShortcutScopeError<K>> {
        let bindings: Vec<_> = bindings.into_iter().collect();
        let neutral_bindings = bindings.iter().map(|binding| {
            let mut neutral = ShortcutBinding::new(
                binding.key,
                binding.command,
                binding.scope,
                binding.shortcut.chord,
            );
            neutral.enabled = binding.enabled;
            neutral.priority = binding.shortcut.priority;
            neutral.repeat = binding.shortcut.repeat;
            neutral
        });
        self.matcher
            .update_bindings(neutral_bindings)
            .map_err(CommandShortcutScopeError::Matcher)?;
        self.displays = bindings
            .into_iter()
            .map(|binding| (binding.key, binding.shortcut.display))
            .collect();
        Ok(())
    }

    /// Reads every registration before atomically replacing the neutral matcher snapshot.
    pub fn update_controlled<HostAction: 'static>(
        &mut self,
        ui: &mut Ui<'_, '_, HostAction>,
        registrations: impl IntoIterator<Item = CommandShortcutRegistration<K, C>>,
    ) -> Result<(), CommandShortcutScopeError<K>>
    where
        C: 'static,
    {
        let resolved = registrations
            .into_iter()
            .map(|registration| registration.resolve(ui))
            .collect::<Result<Vec<_>, _>>()
            .map_err(CommandShortcutScopeError::Runtime)?;
        self.update_resolved(resolved)
    }

    /// Delegates matching unchanged and attaches display metadata only for one exact match.
    pub fn resolve(
        &mut self,
        event: KeyEvent,
        active_scopes: impl IntoIterator<Item = ActiveShortcutScope>,
    ) -> Result<CommandShortcutOutcome<K, C>, CommandShortcutScopeError<K>> {
        let resolution = self
            .matcher
            .resolve(event, active_scopes)
            .map_err(CommandShortcutScopeError::Matcher)?;
        let display = match &resolution {
            ShortcutResolution::Matched { binding, .. } => self
                .displays
                .iter()
                .find_map(|(key, display)| (*key == *binding).then(|| display.clone())),
            ShortcutResolution::NoMatch
            | ShortcutResolution::Ambiguous { .. }
            | ShortcutResolution::Blocked { .. } => None,
        };
        Ok(CommandShortcutOutcome {
            resolution,
            display,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandShortcutScopeError<K> {
    Runtime(RuntimeError),
    Matcher(ShortcutError<K>),
}

impl<K: fmt::Debug> fmt::Display for CommandShortcutScopeError<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Runtime(error) => error.fmt(formatter),
            Self::Matcher(error) => {
                write!(formatter, "command shortcut matcher rejected {error:?}")
            }
        }
    }
}

impl<K: fmt::Debug> std::error::Error for CommandShortcutScopeError<K> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Runtime(error) => Some(error),
            Self::Matcher(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use crate::input::{ButtonState, Modifiers, PhysicalKey, ShortcutScopePolicy, ShortcutTrigger};
    use crate::runtime::{Component, CreateContext, NoAction, State, UpdateContext, ViewRuntime};
    use crate::ui::{BoxStyle, LayoutStyle, UiRoot};

    use crate::components::application::ActionFactory;

    use super::*;

    const SAVE_KEY: PhysicalKey = PhysicalKey::new(10);

    fn scope(slot: u32) -> ShortcutScopeId {
        ShortcutScopeId::from_raw(slot, 1).unwrap()
    }

    fn event(repeat: bool) -> KeyEvent {
        KeyEvent {
            physical_key: SAVE_KEY,
            state: ButtonState::Pressed,
            repeat,
            modifiers: Modifiers::CONTROL,
            ..KeyEvent::new(SAVE_KEY, ButtonState::Pressed)
        }
    }

    fn shortcut(label: &str) -> CommandShortcut {
        CommandShortcut::new(
            ShortcutChord::pressed(SAVE_KEY, Modifiers::CONTROL),
            ShortcutDisplayBinding::new(label).unwrap(),
        )
    }

    fn resolved(
        key: u32,
        command: u32,
        scope: ShortcutScopeId,
        enabled: bool,
        label: &str,
        priority: i16,
    ) -> ResolvedCommandShortcut<u32, u32> {
        ResolvedCommandShortcut::new(
            key,
            command,
            scope,
            enabled,
            shortcut(label).priority(priority),
        )
    }

    #[test]
    fn display_and_set_validation_stay_separate_from_physical_chords() {
        assert_eq!(
            ShortcutDisplayBinding::new("  "),
            Err(ShortcutDisplayBindingError::Empty)
        );
        let declared = shortcut("Ctrl+S")
            .priority(7)
            .repeat(ShortcutRepeatPolicy::Allow);
        assert_eq!(declared.display_binding().label(), "Ctrl+S");
        assert_eq!(declared.priority_value(), 7);
        assert_eq!(declared.repeat_policy(), ShortcutRepeatPolicy::Allow);
        assert_eq!(declared.chord().trigger, ShortcutTrigger::Pressed);
        assert_eq!(
            ShortcutSet::new([declared.clone(), declared]),
            Err(ShortcutSetError::DuplicateChord(ShortcutChord::pressed(
                SAVE_KEY,
                Modifiers::CONTROL
            )))
        );
    }

    #[test]
    fn adapter_preserves_innermost_scope_priority_and_separate_display() {
        let inner = scope(2);
        let outer = scope(1);
        let mut commands = CommandShortcutScope::new();
        commands
            .update_resolved([
                resolved(1, 100, inner, true, "Inner Save", -10),
                resolved(2, 200, outer, true, "Outer Save", 100),
            ])
            .unwrap();

        let outcome = commands
            .resolve(
                event(false),
                [
                    ActiveShortcutScope::bubble(inner),
                    ActiveShortcutScope::bubble(outer),
                ],
            )
            .unwrap();
        assert!(matches!(
            outcome.resolution(),
            ShortcutResolution::Matched {
                binding: 1,
                command: 100,
                scope: selected,
                ..
            } if *selected == inner
        ));
        assert_eq!(outcome.display_binding().unwrap().label(), "Inner Save");
    }

    #[test]
    fn ambiguity_and_modal_blocking_remain_typed_neutral_outcomes() {
        let root = scope(1);
        let modal = scope(2);
        let mut commands = CommandShortcutScope::new();
        commands
            .update_resolved([
                resolved(1, 10, root, true, "First", 1),
                resolved(2, 20, root, true, "Second", 1),
            ])
            .unwrap();
        let ambiguous = commands
            .resolve(event(false), [ActiveShortcutScope::bubble(root)])
            .unwrap();
        assert_eq!(
            ambiguous.resolution(),
            &ShortcutResolution::Ambiguous {
                scope: root,
                chord: ShortcutChord::pressed(SAVE_KEY, Modifiers::CONTROL),
                bindings: vec![1, 2],
            }
        );
        assert_eq!(ambiguous.display_binding(), None);

        let blocked = commands
            .resolve(
                event(false),
                [
                    ActiveShortcutScope {
                        id: modal,
                        policy: ShortcutScopePolicy::Modal,
                    },
                    ActiveShortcutScope::bubble(root),
                ],
            )
            .unwrap();
        assert_eq!(
            blocked.into_resolution(),
            ShortcutResolution::Blocked { scope: modal }
        );
    }

    #[test]
    fn duplicate_update_is_atomic_and_repeat_policy_is_delegated() {
        let root = scope(1);
        let mut commands = CommandShortcutScope::new();
        commands
            .update_resolved([resolved(1, 10, root, true, "Original", 0)])
            .unwrap();
        assert_eq!(
            commands.update_resolved([
                resolved(2, 20, root, true, "Replacement", 0),
                resolved(2, 30, root, true, "Duplicate", 0),
            ]),
            Err(CommandShortcutScopeError::Matcher(
                ShortcutError::DuplicateBinding(2)
            ))
        );
        assert_eq!(commands.binding_count(), 1);
        let repeated = commands
            .resolve(event(true), [ActiveShortcutScope::bubble(root)])
            .unwrap();
        assert_eq!(repeated.into_resolution(), ShortcutResolution::NoMatch);
        let original = commands
            .resolve(event(false), [ActiveShortcutScope::bubble(root)])
            .unwrap();
        assert!(matches!(
            original.resolution(),
            ShortcutResolution::Matched { command: 10, .. }
        ));
        assert_eq!(original.display_binding().unwrap().label(), "Original");
        assert_eq!(commands.diagnostics().repeat_skips, 1);
    }

    struct MountedControlledScope {
        outcome: Rc<RefCell<Option<CommandShortcutOutcome<u32, u32>>>>,
        factory_calls: Rc<Cell<u32>>,
    }

    struct MountedControlledScopeState {
        inner: CommandSpec<u32, ()>,
        outer: CommandSpec<u32, ()>,
        _inner_enabled: State<bool>,
        _outer_enabled: State<bool>,
    }

    impl Component for MountedControlledScope {
        type State = MountedControlledScopeState;
        type Action = NoAction;

        fn create(&self, context: &mut CreateContext<'_>) -> Self::State {
            let inner_enabled = context.state(false);
            let outer_enabled = context.state(true);
            let inner_calls = self.factory_calls.clone();
            let outer_calls = self.factory_calls.clone();
            let inner = CommandSpec::new(
                100,
                "Inner",
                inner_enabled.read(),
                ActionFactory::new(context.component(), move |_| {
                    inner_calls.set(inner_calls.get() + 1)
                }),
            )
            .unwrap()
            .shortcuts(ShortcutSet::single(shortcut("Inner Save")));
            let outer = CommandSpec::new(
                200,
                "Outer",
                outer_enabled.read(),
                ActionFactory::new(context.component(), move |_| {
                    outer_calls.set(outer_calls.get() + 1)
                }),
            )
            .unwrap()
            .shortcuts(ShortcutSet::single(shortcut("Outer Save")));
            MountedControlledScopeState {
                inner,
                outer,
                _inner_enabled: inner_enabled,
                _outer_enabled: outer_enabled,
            }
        }

        fn mount(&self, state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let inner_scope = scope(2);
            let outer_scope = scope(1);
            let mut commands = CommandShortcutScope::new();
            commands
                .update_controlled(
                    ui,
                    [
                        state
                            .inner
                            .shortcut_registration(1, inner_scope, 0)
                            .unwrap(),
                        state
                            .outer
                            .shortcut_registration(2, outer_scope, 0)
                            .unwrap(),
                    ],
                )
                .unwrap();
            let outcome = commands
                .resolve(
                    event(false),
                    [
                        ActiveShortcutScope::bubble(inner_scope),
                        ActiveShortcutScope::bubble(outer_scope),
                    ],
                )
                .unwrap();
            self.outcome.replace(Some(outcome));
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
    fn mounted_adapter_reads_availability_and_never_invokes_action_factories() {
        let outcome = Rc::new(RefCell::new(None));
        let factory_calls = Rc::new(Cell::new(0));
        let _runtime = ViewRuntime::from_component(MountedControlledScope {
            outcome: outcome.clone(),
            factory_calls: factory_calls.clone(),
        })
        .unwrap();
        let outcome = outcome.borrow();
        assert!(matches!(
            outcome.as_ref().unwrap().resolution(),
            ShortcutResolution::Matched {
                binding: 2,
                command: 200,
                ..
            }
        ));
        assert_eq!(
            outcome.as_ref().unwrap().display_binding().unwrap().label(),
            "Outer Save"
        );
        assert_eq!(factory_calls.get(), 0);
    }
}

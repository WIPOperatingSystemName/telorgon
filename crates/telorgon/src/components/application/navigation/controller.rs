//! Typed application route-stack and restoration-key owner.
//!
//! This controller owns logical application navigation only. It does not mount route content,
//! invoke a URL/native navigation service, write platform history, or apply focus restoration.

use std::fmt;
use std::num::NonZeroU64;

use crate::components::application::ChangeSource;

/// Stable caller-chosen key used to restore route-local presentation state after returning.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NavigationRestorationKey(NonZeroU64);

impl NavigationRestorationKey {
    pub const fn new(value: NonZeroU64) -> Self {
        Self(value)
    }

    pub const fn from_raw(value: u64) -> Option<Self> {
        match NonZeroU64::new(value) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// One retained route and its optional route-local restoration identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavigationEntry<R> {
    route: R,
    restoration: Option<NavigationRestorationKey>,
}

impl<R> NavigationEntry<R> {
    pub const fn new(route: R, restoration: Option<NavigationRestorationKey>) -> Self {
        Self { route, restoration }
    }

    pub const fn route(&self) -> &R {
        &self.route
    }

    pub const fn restoration_key(&self) -> Option<NavigationRestorationKey> {
        self.restoration
    }

    pub fn into_route(self) -> R {
        self.route
    }
}

/// A component-produced request to select one route already retained by the controller.
///
/// Constructing this value does not mutate the route stack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavigationSelectionRequest<R> {
    route: R,
    source: ChangeSource,
}

impl<R> NavigationSelectionRequest<R> {
    pub const fn route(&self) -> &R {
        &self.route
    }

    pub const fn source(&self) -> ChangeSource {
        self.source
    }
}

/// Kind of accepted route-stack transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NavigationTransitionKind {
    Push,
    Replace,
    Pop,
    Select,
    Unchanged,
}

/// Accepted transition. Removed entries are reported from the former top toward the new current
/// route, matching teardown order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavigationTransition<R> {
    kind: NavigationTransitionKind,
    source: ChangeSource,
    previous: R,
    current: R,
    removed: Vec<NavigationEntry<R>>,
    restoration: Option<NavigationRestorationKey>,
    revision: u64,
}

impl<R> NavigationTransition<R> {
    pub const fn kind(&self) -> NavigationTransitionKind {
        self.kind
    }

    pub const fn source(&self) -> ChangeSource {
        self.source
    }

    pub const fn previous(&self) -> &R {
        &self.previous
    }

    pub const fn current(&self) -> &R {
        &self.current
    }

    pub fn removed(&self) -> &[NavigationEntry<R>] {
        &self.removed
    }

    /// Restoration identity for the route revealed by Pop or Select.
    pub const fn restoration_key(&self) -> Option<NavigationRestorationKey> {
        self.restoration
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }
}

/// Deterministic controller counters for later per-view diagnostics aggregation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NavigationDiagnostics {
    pub pushes: u64,
    pub replacements: u64,
    pub pops: u64,
    pub selection_requests: u64,
    pub selections: u64,
    pub unchanged_selections: u64,
    pub failures: u64,
}

/// One logical application-navigation owner.
#[derive(Clone, Debug)]
pub struct NavigationController<R> {
    entries: Vec<NavigationEntry<R>>,
    revision: u64,
    diagnostics: NavigationDiagnostics,
}

impl<R> NavigationController<R>
where
    R: Clone + Eq,
{
    pub fn new(root: R, restoration: Option<NavigationRestorationKey>) -> Self {
        Self {
            entries: vec![NavigationEntry::new(root, restoration)],
            revision: 1,
            diagnostics: NavigationDiagnostics::default(),
        }
    }

    pub fn entries(&self) -> &[NavigationEntry<R>] {
        &self.entries
    }

    pub fn current(&self) -> &R {
        &self
            .entries
            .last()
            .expect("navigation controller always retains its root")
            .route
    }

    pub fn current_entry(&self) -> &NavigationEntry<R> {
        self.entries
            .last()
            .expect("navigation controller always retains its root")
    }

    pub fn depth(&self) -> usize {
        self.entries.len()
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }

    pub const fn diagnostics(&self) -> NavigationDiagnostics {
        self.diagnostics
    }

    pub fn contains(&self, route: &R) -> bool {
        self.entries.iter().any(|entry| &entry.route == route)
    }

    pub fn push(
        &mut self,
        route: R,
        restoration: Option<NavigationRestorationKey>,
        source: ChangeSource,
    ) -> Result<NavigationTransition<R>, NavigationError<R>> {
        self.validate_route_and_restoration(&route, restoration, None)?;
        let revision = self.next_revision()?;
        let previous = self.current().clone();
        self.entries
            .push(NavigationEntry::new(route.clone(), restoration));
        self.revision = revision;
        self.diagnostics.pushes += 1;
        Ok(NavigationTransition {
            kind: NavigationTransitionKind::Push,
            source,
            previous,
            current: route,
            removed: Vec::new(),
            restoration: None,
            revision,
        })
    }

    pub fn replace(
        &mut self,
        route: R,
        restoration: Option<NavigationRestorationKey>,
        source: ChangeSource,
    ) -> Result<NavigationTransition<R>, NavigationError<R>> {
        let current_index = self.entries.len() - 1;
        self.validate_route_and_restoration(&route, restoration, Some(current_index))?;
        if self.current_entry().route == route && self.current_entry().restoration == restoration {
            return Ok(self.unchanged(source));
        }
        let revision = self.next_revision()?;
        let previous = self.current().clone();
        let removed = std::mem::replace(
            &mut self.entries[current_index],
            NavigationEntry::new(route.clone(), restoration),
        );
        self.revision = revision;
        self.diagnostics.replacements += 1;
        Ok(NavigationTransition {
            kind: NavigationTransitionKind::Replace,
            source,
            previous,
            current: route,
            removed: vec![removed],
            restoration: None,
            revision,
        })
    }

    pub fn pop(
        &mut self,
        source: ChangeSource,
    ) -> Result<NavigationTransition<R>, NavigationError<R>> {
        if self.entries.len() == 1 {
            self.diagnostics.failures += 1;
            return Err(NavigationError::CannotPopRoot);
        }
        let revision = self.next_revision()?;
        let previous = self.current().clone();
        let removed = self
            .entries
            .pop()
            .expect("root check proves one removable entry");
        let current = self.current().clone();
        let restoration = self.current_entry().restoration;
        self.revision = revision;
        self.diagnostics.pops += 1;
        Ok(NavigationTransition {
            kind: NavigationTransitionKind::Pop,
            source,
            previous,
            current,
            removed: vec![removed],
            restoration,
            revision,
        })
    }

    /// Validates a requested selection without changing the controller.
    pub fn request_selection(
        &mut self,
        route: R,
        source: ChangeSource,
    ) -> Result<NavigationSelectionRequest<R>, NavigationError<R>> {
        if !self.contains(&route) {
            self.diagnostics.failures += 1;
            return Err(NavigationError::UnknownRoute(route));
        }
        self.diagnostics.selection_requests += 1;
        Ok(NavigationSelectionRequest { route, source })
    }

    /// Applies a previously validated selection by revealing that retained stack entry.
    pub fn select(
        &mut self,
        request: NavigationSelectionRequest<R>,
    ) -> Result<NavigationTransition<R>, NavigationError<R>> {
        let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.route == request.route)
        else {
            self.diagnostics.failures += 1;
            return Err(NavigationError::UnknownRoute(request.route));
        };
        if index + 1 == self.entries.len() {
            self.diagnostics.unchanged_selections += 1;
            return Ok(self.unchanged(request.source));
        }
        let revision = self.next_revision()?;
        let previous = self.current().clone();
        let mut removed = self.entries.split_off(index + 1);
        removed.reverse();
        let current = self.current().clone();
        let restoration = self.current_entry().restoration;
        self.revision = revision;
        self.diagnostics.selections += 1;
        Ok(NavigationTransition {
            kind: NavigationTransitionKind::Select,
            source: request.source,
            previous,
            current,
            removed,
            restoration,
            revision,
        })
    }

    fn validate_route_and_restoration(
        &mut self,
        route: &R,
        restoration: Option<NavigationRestorationKey>,
        ignored_index: Option<usize>,
    ) -> Result<(), NavigationError<R>> {
        if self
            .entries
            .iter()
            .enumerate()
            .any(|(index, entry)| Some(index) != ignored_index && &entry.route == route)
        {
            self.diagnostics.failures += 1;
            return Err(NavigationError::DuplicateRoute(route.clone()));
        }
        if let Some(restoration) = restoration
            && self.entries.iter().enumerate().any(|(index, entry)| {
                Some(index) != ignored_index && entry.restoration == Some(restoration)
            })
        {
            self.diagnostics.failures += 1;
            return Err(NavigationError::DuplicateRestorationKey(restoration));
        }
        Ok(())
    }

    fn next_revision(&mut self) -> Result<u64, NavigationError<R>> {
        self.revision.checked_add(1).ok_or_else(|| {
            self.diagnostics.failures += 1;
            NavigationError::RevisionExhausted
        })
    }

    fn unchanged(&self, source: ChangeSource) -> NavigationTransition<R> {
        NavigationTransition {
            kind: NavigationTransitionKind::Unchanged,
            source,
            previous: self.current().clone(),
            current: self.current().clone(),
            removed: Vec::new(),
            restoration: None,
            revision: self.revision,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NavigationError<R> {
    DuplicateRoute(R),
    DuplicateRestorationKey(NavigationRestorationKey),
    UnknownRoute(R),
    CannotPopRoot,
    RevisionExhausted,
}

impl<R: fmt::Debug> fmt::Display for NavigationError<R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "navigation transition failed: {self:?}")
    }
}

impl<R: fmt::Debug> std::error::Error for NavigationError<R> {}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(value: u64) -> NavigationRestorationKey {
        NavigationRestorationKey::from_raw(value).unwrap()
    }

    #[test]
    fn push_replace_and_failures_are_atomic_and_source_preserving() {
        let mut navigation = NavigationController::new("home", Some(key(1)));
        let pushed = navigation
            .push("details", Some(key(2)), ChangeSource::Pointer)
            .unwrap();
        assert_eq!(pushed.kind(), NavigationTransitionKind::Push);
        assert_eq!(pushed.source(), ChangeSource::Pointer);
        assert_eq!(pushed.previous(), &"home");
        assert_eq!(pushed.current(), &"details");
        assert!(pushed.removed().is_empty());

        let revision = navigation.revision();
        assert_eq!(
            navigation.push("home", Some(key(3)), ChangeSource::Keyboard),
            Err(NavigationError::DuplicateRoute("home"))
        );
        assert_eq!(
            navigation.push("other", Some(key(2)), ChangeSource::Keyboard),
            Err(NavigationError::DuplicateRestorationKey(key(2)))
        );
        assert_eq!(navigation.revision(), revision);
        assert_eq!(navigation.current(), &"details");

        let replaced = navigation
            .replace("editor", Some(key(4)), ChangeSource::Programmatic)
            .unwrap();
        assert_eq!(replaced.kind(), NavigationTransitionKind::Replace);
        assert_eq!(replaced.source(), ChangeSource::Programmatic);
        assert_eq!(replaced.removed()[0].route(), &"details");
        assert_eq!(navigation.current(), &"editor");
    }

    #[test]
    fn pop_reveals_restoration_key_and_never_removes_root() {
        let mut navigation = NavigationController::new(1_u32, Some(key(11)));
        navigation
            .push(2, Some(key(12)), ChangeSource::Keyboard)
            .unwrap();
        navigation
            .push(3, Some(key(13)), ChangeSource::Keyboard)
            .unwrap();
        let popped = navigation.pop(ChangeSource::Accessibility).unwrap();
        assert_eq!(popped.kind(), NavigationTransitionKind::Pop);
        assert_eq!(popped.source(), ChangeSource::Accessibility);
        assert_eq!(popped.current(), &2);
        assert_eq!(popped.removed()[0].route(), &3);
        assert_eq!(popped.restoration_key(), Some(key(12)));
        navigation.pop(ChangeSource::Keyboard).unwrap();
        let revision = navigation.revision();
        assert_eq!(
            navigation.pop(ChangeSource::Keyboard),
            Err(NavigationError::CannotPopRoot)
        );
        assert_eq!(navigation.revision(), revision);
        assert_eq!(navigation.current(), &1);
    }

    #[test]
    fn selection_request_is_nonmutating_then_selects_with_top_first_teardown() {
        let mut navigation = NavigationController::new("root", Some(key(21)));
        navigation
            .push("one", Some(key(22)), ChangeSource::Programmatic)
            .unwrap();
        navigation
            .push("two", Some(key(23)), ChangeSource::Programmatic)
            .unwrap();
        navigation
            .push("three", Some(key(24)), ChangeSource::Programmatic)
            .unwrap();
        let revision = navigation.revision();
        let request = navigation
            .request_selection("one", ChangeSource::Directional)
            .unwrap();
        assert_eq!(navigation.current(), &"three");
        assert_eq!(navigation.revision(), revision);
        assert_eq!(request.source(), ChangeSource::Directional);

        let selected = navigation.select(request).unwrap();
        assert_eq!(selected.kind(), NavigationTransitionKind::Select);
        assert_eq!(selected.current(), &"one");
        assert_eq!(selected.restoration_key(), Some(key(22)));
        assert_eq!(
            selected
                .removed()
                .iter()
                .map(NavigationEntry::route)
                .copied()
                .collect::<Vec<_>>(),
            vec!["three", "two"]
        );
        assert_eq!(navigation.depth(), 2);
    }

    #[test]
    fn stale_selection_request_rejects_after_the_route_leaves() {
        let mut navigation = NavigationController::new(1_u8, None);
        navigation.push(2, None, ChangeSource::Keyboard).unwrap();
        let stale = navigation
            .request_selection(2, ChangeSource::Keyboard)
            .unwrap();
        navigation.pop(ChangeSource::Keyboard).unwrap();
        assert_eq!(
            navigation.select(stale),
            Err(NavigationError::UnknownRoute(2))
        );
    }
}

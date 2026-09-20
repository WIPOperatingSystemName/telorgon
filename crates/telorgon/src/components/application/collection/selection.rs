//! Stable-key application selection owner.
//!
//! This model owns selected keys, the range anchor, and focus-selection policy. Collection data,
//! row components, focus traversal, and platform services remain outside this package.

use std::fmt;

use crate::components::application::ChangeSource;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SelectionMode {
    None,
    Single,
    Multiple,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SelectionFollowsFocus {
    #[default]
    Disabled,
    Enabled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SelectionProposalKind {
    Clear,
    Select,
    Toggle,
    Extend,
    Focus,
    Set,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionProposal<K> {
    kind: SelectionProposalKind,
    source: ChangeSource,
    base_revision: u64,
    selected: Vec<K>,
    anchor: Option<K>,
}

impl<K> SelectionProposal<K> {
    pub const fn kind(&self) -> SelectionProposalKind {
        self.kind
    }

    pub const fn source(&self) -> ChangeSource {
        self.source
    }

    pub const fn base_revision(&self) -> u64 {
        self.base_revision
    }

    pub fn selected(&self) -> &[K] {
        &self.selected
    }

    pub const fn anchor(&self) -> Option<&K> {
        self.anchor.as_ref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionTransition<K> {
    kind: SelectionProposalKind,
    source: ChangeSource,
    previous: Vec<K>,
    selected: Vec<K>,
    previous_anchor: Option<K>,
    anchor: Option<K>,
    changed: bool,
    revision: u64,
}

impl<K> SelectionTransition<K> {
    pub const fn kind(&self) -> SelectionProposalKind {
        self.kind
    }

    pub const fn source(&self) -> ChangeSource {
        self.source
    }

    pub fn previous(&self) -> &[K] {
        &self.previous
    }

    pub fn selected(&self) -> &[K] {
        &self.selected
    }

    pub const fn previous_anchor(&self) -> Option<&K> {
        self.previous_anchor.as_ref()
    }

    pub const fn anchor(&self) -> Option<&K> {
        self.anchor.as_ref()
    }

    pub const fn changed(&self) -> bool {
        self.changed
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionItemsUpdate<K> {
    removed_selected: Vec<K>,
    previous_anchor: Option<K>,
    anchor: Option<K>,
    changed: bool,
    revision: u64,
}

impl<K> SelectionItemsUpdate<K> {
    pub fn removed_selected(&self) -> &[K] {
        &self.removed_selected
    }

    pub const fn previous_anchor(&self) -> Option<&K> {
        self.previous_anchor.as_ref()
    }

    pub const fn anchor(&self) -> Option<&K> {
        self.anchor.as_ref()
    }

    pub const fn changed(&self) -> bool {
        self.changed
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SelectionDiagnostics {
    pub proposals: u64,
    pub applied: u64,
    pub unchanged: u64,
    pub item_updates: u64,
    pub anchor_recoveries: u64,
    pub stale_proposals: u64,
    pub failures: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionModel<K> {
    mode: SelectionMode,
    follows_focus: SelectionFollowsFocus,
    items: Vec<K>,
    selected: Vec<K>,
    anchor: Option<K>,
    revision: u64,
    diagnostics: SelectionDiagnostics,
}

impl<K> SelectionModel<K>
where
    K: Clone + Eq,
{
    pub fn new(
        mode: SelectionMode,
        follows_focus: SelectionFollowsFocus,
        items: impl IntoIterator<Item = K>,
        selected: impl IntoIterator<Item = K>,
        anchor: Option<K>,
    ) -> Result<Self, SelectionError<K>> {
        let items: Vec<_> = items.into_iter().collect();
        validate_unique_items(&items)?;
        let selected: Vec<_> = selected.into_iter().collect();
        let selected = validate_selection(mode, &items, selected, anchor.as_ref())?;
        Ok(Self {
            mode,
            follows_focus,
            items,
            selected,
            anchor,
            revision: 1,
            diagnostics: SelectionDiagnostics::default(),
        })
    }

    pub const fn mode(&self) -> SelectionMode {
        self.mode
    }

    pub const fn selection_follows_focus(&self) -> SelectionFollowsFocus {
        self.follows_focus
    }

    pub fn items(&self) -> &[K] {
        &self.items
    }

    /// Selected keys in canonical current item order.
    pub fn selected(&self) -> &[K] {
        &self.selected
    }

    pub const fn anchor(&self) -> Option<&K> {
        self.anchor.as_ref()
    }

    pub fn is_selected(&self, key: &K) -> bool {
        self.selected.contains(key)
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }

    pub const fn diagnostics(&self) -> SelectionDiagnostics {
        self.diagnostics
    }

    pub fn propose_clear(&mut self, source: ChangeSource) -> SelectionProposal<K> {
        self.proposal(SelectionProposalKind::Clear, source, Vec::new(), None)
    }

    /// Proposes ordinary exclusive selection. In Multiple mode this is an explicit collapse.
    pub fn propose_select(
        &mut self,
        key: &K,
        source: ChangeSource,
    ) -> Result<SelectionProposal<K>, SelectionError<K>> {
        self.require_selectable(key)?;
        Ok(self.proposal(
            SelectionProposalKind::Select,
            source,
            vec![key.clone()],
            Some(key.clone()),
        ))
    }

    pub fn propose_toggle(
        &mut self,
        key: &K,
        source: ChangeSource,
    ) -> Result<SelectionProposal<K>, SelectionError<K>> {
        if self.mode != SelectionMode::Multiple {
            self.diagnostics.failures += 1;
            return Err(SelectionError::OperationRequiresMultiple(
                SelectionProposalKind::Toggle,
            ));
        }
        self.require_item(key)?;
        let mut selected = self.selected.clone();
        if let Some(index) = selected.iter().position(|selected| selected == key) {
            selected.remove(index);
        } else {
            selected.push(key.clone());
        }
        canonicalize(&self.items, &mut selected);
        let anchor = self.anchor.clone().or_else(|| Some(key.clone()));
        Ok(self.proposal(SelectionProposalKind::Toggle, source, selected, anchor))
    }

    pub fn propose_extend(
        &mut self,
        key: &K,
        source: ChangeSource,
    ) -> Result<SelectionProposal<K>, SelectionError<K>> {
        if self.mode != SelectionMode::Multiple {
            self.diagnostics.failures += 1;
            return Err(SelectionError::OperationRequiresMultiple(
                SelectionProposalKind::Extend,
            ));
        }
        self.require_item(key)?;
        let Some(anchor) = self.anchor.as_ref() else {
            self.diagnostics.failures += 1;
            return Err(SelectionError::MissingAnchor);
        };
        let anchor_index = self
            .items
            .iter()
            .position(|item| item == anchor)
            .expect("validated anchors remain in items until an atomic update");
        let key_index = self
            .items
            .iter()
            .position(|item| item == key)
            .expect("required item is present");
        let (start, end) = if anchor_index <= key_index {
            (anchor_index, key_index)
        } else {
            (key_index, anchor_index)
        };
        Ok(self.proposal(
            SelectionProposalKind::Extend,
            source,
            self.items[start..=end].to_vec(),
            Some(anchor.clone()),
        ))
    }

    /// Proposes policy-driven selection after focus moved. Multiple mode only adds and never
    /// collapses the existing selection.
    pub fn propose_focus(
        &mut self,
        key: &K,
        source: ChangeSource,
    ) -> Result<Option<SelectionProposal<K>>, SelectionError<K>> {
        self.require_item(key)?;
        if self.follows_focus == SelectionFollowsFocus::Disabled || self.mode == SelectionMode::None
        {
            return Ok(None);
        }
        let (selected, anchor) = match self.mode {
            SelectionMode::None => unreachable!("none mode returned above"),
            SelectionMode::Single => (vec![key.clone()], Some(key.clone())),
            SelectionMode::Multiple => {
                let mut selected = self.selected.clone();
                if !selected.contains(key) {
                    selected.push(key.clone());
                    canonicalize(&self.items, &mut selected);
                }
                (selected, self.anchor.clone().or_else(|| Some(key.clone())))
            }
        };
        Ok(Some(self.proposal(
            SelectionProposalKind::Focus,
            source,
            selected,
            anchor,
        )))
    }

    /// Proposes a complete controlled snapshot after validating it atomically.
    pub fn propose_set(
        &mut self,
        selected: impl IntoIterator<Item = K>,
        anchor: Option<K>,
        source: ChangeSource,
    ) -> Result<SelectionProposal<K>, SelectionError<K>> {
        let selected = validate_selection(
            self.mode,
            &self.items,
            selected.into_iter().collect(),
            anchor.as_ref(),
        )
        .inspect_err(|_| {
            self.diagnostics.failures += 1;
        })?;
        Ok(self.proposal(SelectionProposalKind::Set, source, selected, anchor))
    }

    pub fn apply(
        &mut self,
        proposal: SelectionProposal<K>,
    ) -> Result<SelectionTransition<K>, SelectionError<K>> {
        if proposal.base_revision != self.revision {
            self.diagnostics.stale_proposals += 1;
            return Err(SelectionError::StaleProposal {
                expected: self.revision,
                actual: proposal.base_revision,
            });
        }
        let selected = validate_selection(
            self.mode,
            &self.items,
            proposal.selected,
            proposal.anchor.as_ref(),
        )
        .inspect_err(|_| {
            self.diagnostics.failures += 1;
        })?;
        let changed = selected != self.selected || proposal.anchor != self.anchor;
        let revision = if changed {
            self.next_revision()?
        } else {
            self.revision
        };
        let previous = self.selected.clone();
        let previous_anchor = self.anchor.clone();
        if changed {
            self.selected = selected.clone();
            self.anchor = proposal.anchor.clone();
            self.revision = revision;
            self.diagnostics.applied += 1;
        } else {
            self.diagnostics.unchanged += 1;
        }
        Ok(SelectionTransition {
            kind: proposal.kind,
            source: proposal.source,
            previous,
            selected,
            previous_anchor,
            anchor: proposal.anchor,
            changed,
            revision,
        })
    }

    /// Atomically replaces canonical item order, preserves surviving selected keys, and recovers
    /// a removed anchor to the nearest surviving selected key in the former order.
    pub fn update_items(
        &mut self,
        items: impl IntoIterator<Item = K>,
    ) -> Result<SelectionItemsUpdate<K>, SelectionError<K>> {
        let items: Vec<_> = items.into_iter().collect();
        validate_unique_items(&items).inspect_err(|_| {
            self.diagnostics.failures += 1;
        })?;
        if items == self.items {
            return Ok(SelectionItemsUpdate {
                removed_selected: Vec::new(),
                previous_anchor: self.anchor.clone(),
                anchor: self.anchor.clone(),
                changed: false,
                revision: self.revision,
            });
        }

        let removed_selected: Vec<_> = self
            .selected
            .iter()
            .filter(|selected| !items.contains(selected))
            .cloned()
            .collect();
        let mut selected: Vec<_> = self
            .selected
            .iter()
            .filter(|selected| items.contains(selected))
            .cloned()
            .collect();
        canonicalize(&items, &mut selected);
        let previous_anchor = self.anchor.clone();
        let anchor = match self.anchor.as_ref() {
            Some(anchor) if items.contains(anchor) => Some(anchor.clone()),
            Some(anchor) => recover_anchor(&self.items, &selected, anchor),
            None => None,
        };
        let revision = self.next_revision()?;
        if previous_anchor.is_some() && anchor != previous_anchor {
            self.diagnostics.anchor_recoveries += 1;
        }
        self.items = items;
        self.selected = selected;
        self.anchor = anchor.clone();
        self.revision = revision;
        self.diagnostics.item_updates += 1;
        Ok(SelectionItemsUpdate {
            removed_selected,
            previous_anchor,
            anchor,
            changed: true,
            revision,
        })
    }

    fn proposal(
        &mut self,
        kind: SelectionProposalKind,
        source: ChangeSource,
        selected: Vec<K>,
        anchor: Option<K>,
    ) -> SelectionProposal<K> {
        self.diagnostics.proposals += 1;
        SelectionProposal {
            kind,
            source,
            base_revision: self.revision,
            selected,
            anchor,
        }
    }

    fn require_selectable(&mut self, key: &K) -> Result<(), SelectionError<K>> {
        if self.mode == SelectionMode::None {
            self.diagnostics.failures += 1;
            return Err(SelectionError::SelectionDisabled);
        }
        self.require_item(key)
    }

    fn require_item(&mut self, key: &K) -> Result<(), SelectionError<K>> {
        if !self.items.contains(key) {
            self.diagnostics.failures += 1;
            return Err(SelectionError::UnknownItem(key.clone()));
        }
        Ok(())
    }

    fn next_revision(&mut self) -> Result<u64, SelectionError<K>> {
        self.revision.checked_add(1).ok_or_else(|| {
            self.diagnostics.failures += 1;
            SelectionError::RevisionExhausted
        })
    }
}

fn validate_unique_items<K>(items: &[K]) -> Result<(), SelectionError<K>>
where
    K: Clone + Eq,
{
    for (index, item) in items.iter().enumerate() {
        if items[..index].contains(item) {
            return Err(SelectionError::DuplicateItem(item.clone()));
        }
    }
    Ok(())
}

fn validate_selection<K>(
    mode: SelectionMode,
    items: &[K],
    mut selected: Vec<K>,
    anchor: Option<&K>,
) -> Result<Vec<K>, SelectionError<K>>
where
    K: Clone + Eq,
{
    for (index, key) in selected.iter().enumerate() {
        if selected[..index].contains(key) {
            return Err(SelectionError::DuplicateSelection(key.clone()));
        }
        if !items.contains(key) {
            return Err(SelectionError::UnknownItem(key.clone()));
        }
    }
    if let Some(anchor) = anchor
        && !items.contains(anchor)
    {
        return Err(SelectionError::UnknownItem(anchor.clone()));
    }
    match mode {
        SelectionMode::None if !selected.is_empty() || anchor.is_some() => {
            return Err(SelectionError::SelectionDisabled);
        }
        SelectionMode::Single if selected.len() > 1 => {
            return Err(SelectionError::TooManySelected {
                mode,
                count: selected.len(),
            });
        }
        _ => {}
    }
    canonicalize(items, &mut selected);
    Ok(selected)
}

fn canonicalize<K>(items: &[K], selected: &mut [K])
where
    K: Eq,
{
    selected.sort_by_key(|key| {
        items
            .iter()
            .position(|item| item == key)
            .expect("validated selected keys exist in canonical items")
    });
}

fn recover_anchor<K>(old_items: &[K], surviving_selected: &[K], removed_anchor: &K) -> Option<K>
where
    K: Clone + Eq,
{
    let removed_index = old_items.iter().position(|item| item == removed_anchor)?;
    surviving_selected
        .iter()
        .filter_map(|key| {
            old_items
                .iter()
                .position(|item| item == key)
                .map(|index| (key, index))
        })
        .min_by_key(|(_, index)| {
            (
                index.abs_diff(removed_index),
                usize::from(*index < removed_index),
            )
        })
        .map(|(key, _)| key.clone())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectionError<K> {
    DuplicateItem(K),
    DuplicateSelection(K),
    UnknownItem(K),
    SelectionDisabled,
    TooManySelected { mode: SelectionMode, count: usize },
    MissingAnchor,
    OperationRequiresMultiple(SelectionProposalKind),
    StaleProposal { expected: u64, actual: u64 },
    RevisionExhausted,
}

impl<K: fmt::Debug> fmt::Display for SelectionError<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "selection-model operation failed: {self:?}")
    }
}

impl<K: fmt::Debug> std::error::Error for SelectionError<K> {}

#[cfg(test)]
mod tests {
    use super::*;

    fn multiple(selected: &[u8], anchor: Option<u8>) -> SelectionModel<u8> {
        SelectionModel::new(
            SelectionMode::Multiple,
            SelectionFollowsFocus::Enabled,
            [1, 2, 3, 4, 5],
            selected.iter().copied(),
            anchor,
        )
        .unwrap()
    }

    #[test]
    fn construction_validates_modes_keys_selection_and_anchor() {
        assert!(matches!(
            SelectionModel::new(
                SelectionMode::Single,
                SelectionFollowsFocus::Disabled,
                [1, 1],
                [],
                None,
            ),
            Err(SelectionError::DuplicateItem(1))
        ));
        assert!(matches!(
            SelectionModel::new(
                SelectionMode::Single,
                SelectionFollowsFocus::Disabled,
                [1, 2],
                [1, 2],
                Some(1),
            ),
            Err(SelectionError::TooManySelected { .. })
        ));
        assert_eq!(
            SelectionModel::new(
                SelectionMode::None,
                SelectionFollowsFocus::Enabled,
                [1, 2],
                [1],
                None,
            ),
            Err(SelectionError::SelectionDisabled)
        );
        assert_eq!(
            SelectionModel::new(
                SelectionMode::Multiple,
                SelectionFollowsFocus::Disabled,
                [1, 2],
                [],
                Some(3),
            ),
            Err(SelectionError::UnknownItem(3))
        );
    }

    #[test]
    fn proposals_are_nonmutating_source_preserving_revision_checked_and_atomic() {
        let mut model = SelectionModel::new(
            SelectionMode::Single,
            SelectionFollowsFocus::Enabled,
            [1, 2, 3],
            [1],
            Some(1),
        )
        .unwrap();
        let proposal = model
            .propose_select(&2, ChangeSource::Accessibility)
            .unwrap();
        assert_eq!(proposal.selected(), &[2]);
        assert_eq!(model.selected(), &[1]);
        let stale = proposal.clone();
        let transition = model.apply(proposal).unwrap();
        assert_eq!(transition.source(), ChangeSource::Accessibility);
        assert_eq!(transition.previous(), &[1]);
        assert_eq!(transition.selected(), &[2]);
        assert!(transition.changed());
        let revision = model.revision();
        assert_eq!(
            model.apply(stale),
            Err(SelectionError::StaleProposal {
                expected: revision,
                actual: 1,
            })
        );
        assert_eq!(model.selected(), &[2]);
    }

    #[test]
    fn multiple_focus_adds_without_collapsing_and_explicit_extend_uses_anchor() {
        let mut model = multiple(&[1, 3], Some(1));
        let focus = model
            .propose_focus(&4, ChangeSource::Directional)
            .unwrap()
            .unwrap();
        assert_eq!(focus.kind(), SelectionProposalKind::Focus);
        assert_eq!(focus.selected(), &[1, 3, 4]);
        model.apply(focus).unwrap();
        assert_eq!(model.selected(), &[1, 3, 4]);
        let extend = model.propose_extend(&3, ChangeSource::Keyboard).unwrap();
        assert_eq!(extend.selected(), &[1, 2, 3]);
        assert_eq!(extend.anchor(), Some(&1));
        model.apply(extend).unwrap();
        assert_eq!(model.selected(), &[1, 2, 3]);
    }

    #[test]
    fn reorder_preserves_keys_and_anchor_removal_recovers_nearest_selected_successor() {
        let mut model = multiple(&[1, 3, 5], Some(3));
        let update = model.update_items([5, 4, 2, 1]).unwrap();
        assert_eq!(update.removed_selected(), &[3]);
        assert_eq!(update.previous_anchor(), Some(&3));
        assert_eq!(update.anchor(), Some(&5));
        assert_eq!(model.items(), &[5, 4, 2, 1]);
        assert_eq!(model.selected(), &[5, 1]);
        assert_eq!(model.anchor(), Some(&5));
        let revision = model.revision();
        assert_eq!(
            model.update_items([5, 4, 4, 1]),
            Err(SelectionError::DuplicateItem(4))
        );
        assert_eq!(model.revision(), revision);
        assert_eq!(model.items(), &[5, 4, 2, 1]);
        assert_eq!(model.diagnostics().anchor_recoveries, 1);
    }
}

//! Stable-key hierarchical descriptors and the baseline application tree view.
//!
//! `TreeHierarchy` is the only hierarchy/expansion-snapshot owner, `SelectionModel` remains the
//! only selected-key owner, and `CompositeStateMachine` remains the only active-item owner.

use std::cell::RefCell;
use std::collections::HashSet;
use std::fmt;
use std::hash::Hash;
use std::rc::Rc;

use crate::foundation::ColorRgba8;
use crate::input::{
    ChangeSource, CompositeChange, CompositeEdgeBehavior, CompositeError, CompositeFocusTarget,
    CompositeItem, CompositeNavigationCommand, CompositeNavigationPolicy, CompositeOrientation,
    CompositeSelectionBehavior, CompositeStateMachine, DisabledItemPolicy, WritingDirection,
};
use crate::runtime::{RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    BoxStyle, ControlHandle, Flow, LayoutStyle, Property, SemanticActions, SemanticCollection,
    SemanticName, SemanticNode, SemanticRelationship, SemanticRelationshipKind, SemanticRole,
    SemanticState, SizeRule, SizeRule2D, UiNodeId,
};

use super::{
    SelectionError, SelectionMode, SelectionModel, SelectionProposal, SelectionTransition,
};
use crate::components::application::{DensityClass, DensityMetrics};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeItem<K> {
    key: K,
    label: String,
    parent: Option<K>,
    enabled: bool,
}

impl<K> TreeItem<K> {
    pub fn new(key: K, label: impl Into<String>, parent: Option<K>) -> Result<Self, TreeItemError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(TreeItemError::MissingAccessibleName);
        }
        Ok(Self {
            key,
            label,
            parent,
            enabled: true,
        })
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub const fn key(&self) -> &K {
        &self.key
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub const fn parent(&self) -> Option<&K> {
        self.parent.as_ref()
    }

    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeItemError {
    MissingAccessibleName,
}

impl fmt::Display for TreeItemError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("tree item accessible name is empty")
    }
}

impl std::error::Error for TreeItemError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeExpansionProposal<K> {
    key: K,
    expanded: bool,
    source: ChangeSource,
    base_revision: u64,
}

impl<K> TreeExpansionProposal<K> {
    pub const fn key(&self) -> &K {
        &self.key
    }

    pub const fn expanded(&self) -> bool {
        self.expanded
    }

    pub const fn source(&self) -> ChangeSource {
        self.source
    }

    pub const fn base_revision(&self) -> u64 {
        self.base_revision
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeExpansionTransition<K> {
    key: K,
    previous: bool,
    expanded: bool,
    source: ChangeSource,
    changed: bool,
    revision: u64,
}

impl<K> TreeExpansionTransition<K> {
    pub const fn key(&self) -> &K {
        &self.key
    }

    pub const fn previous(&self) -> bool {
        self.previous
    }

    pub const fn expanded(&self) -> bool {
        self.expanded
    }

    pub const fn source(&self) -> ChangeSource {
        self.source
    }

    pub const fn changed(&self) -> bool {
        self.changed
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TreeHierarchyDiagnostics {
    pub expansion_proposals: u64,
    pub expansion_applies: u64,
    pub unchanged_expansion_applies: u64,
    pub stale_expansion_proposals: u64,
    pub failures: u64,
}

/// Validated canonical-preorder hierarchy plus one controlled expansion snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeHierarchy<K> {
    items: Vec<TreeItem<K>>,
    expanded: Vec<K>,
    revision: u64,
    diagnostics: TreeHierarchyDiagnostics,
}

impl<K> TreeHierarchy<K>
where
    K: Copy + Eq + Hash,
{
    pub fn new(
        items: impl IntoIterator<Item = TreeItem<K>>,
        expanded: impl IntoIterator<Item = K>,
    ) -> Result<Self, TreeHierarchyError<K>> {
        let items: Vec<_> = items.into_iter().collect();
        validate_hierarchy(&items)?;
        let expanded = validate_expanded(&items, expanded.into_iter().collect())?;
        Ok(Self {
            items,
            expanded,
            revision: 1,
            diagnostics: TreeHierarchyDiagnostics::default(),
        })
    }

    pub fn items(&self) -> &[TreeItem<K>] {
        &self.items
    }

    pub fn keys(&self) -> Vec<K> {
        self.items.iter().map(|item| item.key).collect()
    }

    pub fn expanded(&self) -> &[K] {
        &self.expanded
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }

    pub const fn diagnostics(&self) -> TreeHierarchyDiagnostics {
        self.diagnostics
    }

    pub fn item(&self, key: &K) -> Option<&TreeItem<K>> {
        self.items.iter().find(|item| &item.key == key)
    }

    pub fn parent(&self, key: &K) -> Option<&TreeItem<K>> {
        self.item(key)
            .and_then(|item| item.parent.as_ref())
            .and_then(|parent| self.item(parent))
    }

    pub fn children(&self, key: &K) -> Vec<&TreeItem<K>> {
        self.items
            .iter()
            .filter(|item| item.parent.as_ref() == Some(key))
            .collect()
    }

    pub fn is_branch(&self, key: &K) -> bool {
        self.items
            .iter()
            .any(|item| item.parent.as_ref() == Some(key))
    }

    pub fn is_expanded(&self, key: &K) -> bool {
        self.expanded.contains(key)
    }

    pub fn is_visible(&self, key: &K) -> bool {
        let Some(mut item) = self.item(key) else {
            return false;
        };
        while let Some(parent) = item.parent.as_ref() {
            if !self.is_expanded(parent) {
                return false;
            }
            item = self
                .item(parent)
                .expect("validated tree parents remain in the hierarchy");
        }
        true
    }

    pub fn visible_keys(&self) -> Vec<K> {
        self.items
            .iter()
            .filter(|item| self.is_visible(&item.key))
            .map(|item| item.key)
            .collect()
    }

    pub fn level(&self, key: &K) -> Option<u32> {
        let mut item = self.item(key)?;
        let mut level = 1_u32;
        while let Some(parent) = item.parent.as_ref() {
            level = level.checked_add(1)?;
            item = self.item(parent)?;
        }
        Some(level)
    }

    pub fn sibling_position(&self, key: &K) -> Option<(u32, u32)> {
        let item = self.item(key)?;
        let siblings: Vec<_> = self
            .items
            .iter()
            .filter(|candidate| candidate.parent == item.parent)
            .collect();
        let position = siblings
            .iter()
            .position(|candidate| candidate.key == *key)?;
        Some((
            u32::try_from(position + 1).ok()?,
            u32::try_from(siblings.len()).ok()?,
        ))
    }

    pub fn propose_expansion(
        &mut self,
        key: K,
        expanded: bool,
        source: ChangeSource,
    ) -> Result<TreeExpansionProposal<K>, TreeHierarchyError<K>> {
        self.require_branch(&key)?;
        self.diagnostics.expansion_proposals += 1;
        Ok(TreeExpansionProposal {
            key,
            expanded,
            source,
            base_revision: self.revision,
        })
    }

    pub fn apply_expansion(
        &mut self,
        proposal: TreeExpansionProposal<K>,
    ) -> Result<TreeExpansionTransition<K>, TreeHierarchyError<K>> {
        if proposal.base_revision != self.revision {
            self.diagnostics.stale_expansion_proposals += 1;
            return Err(TreeHierarchyError::StaleExpansionProposal {
                expected: self.revision,
                actual: proposal.base_revision,
            });
        }
        self.require_branch(&proposal.key)?;
        let previous = self.is_expanded(&proposal.key);
        if previous == proposal.expanded {
            self.diagnostics.unchanged_expansion_applies += 1;
            return Ok(TreeExpansionTransition {
                key: proposal.key,
                previous,
                expanded: proposal.expanded,
                source: proposal.source,
                changed: false,
                revision: self.revision,
            });
        }
        let revision = self
            .revision
            .checked_add(1)
            .ok_or(TreeHierarchyError::RevisionExhausted)?;
        if proposal.expanded {
            self.expanded.push(proposal.key);
            self.expanded.sort_by_key(|key| {
                self.items
                    .iter()
                    .position(|item| item.key == *key)
                    .expect("expanded keys are validated branches")
            });
        } else {
            self.expanded.retain(|key| *key != proposal.key);
        }
        self.revision = revision;
        self.diagnostics.expansion_applies += 1;
        Ok(TreeExpansionTransition {
            key: proposal.key,
            previous,
            expanded: proposal.expanded,
            source: proposal.source,
            changed: true,
            revision,
        })
    }

    fn require_branch(&mut self, key: &K) -> Result<(), TreeHierarchyError<K>> {
        if self.item(key).is_none() {
            self.diagnostics.failures += 1;
            return Err(TreeHierarchyError::UnknownItem(*key));
        }
        if !self.is_branch(key) {
            self.diagnostics.failures += 1;
            return Err(TreeHierarchyError::LeafExpansion(*key));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeHierarchyError<K> {
    DuplicateKey(K),
    SelfParent(K),
    ParentMustPrecede { key: K, parent: K },
    NonContiguousSubtree(K),
    DuplicateExpandedKey(K),
    UnknownExpandedKey(K),
    LeafExpansion(K),
    UnknownItem(K),
    StaleExpansionProposal { expected: u64, actual: u64 },
    RevisionExhausted,
}

impl<K: fmt::Debug> fmt::Display for TreeHierarchyError<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "tree hierarchy operation failed: {self:?}")
    }
}

impl<K: fmt::Debug> std::error::Error for TreeHierarchyError<K> {}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TreeViewStyle {
    pub root: BoxStyle,
    pub item: BoxStyle,
    pub selected_item: Option<BoxStyle>,
    pub active_item: Option<BoxStyle>,
    pub disabled_item: Option<BoxStyle>,
    pub gap: f32,
    pub label_color: ColorRgba8,
    pub label_size: f32,
}

impl Default for TreeViewStyle {
    fn default() -> Self {
        Self {
            root: BoxStyle::default(),
            item: BoxStyle::default(),
            selected_item: None,
            active_item: None,
            disabled_item: None,
            gap: 0.0,
            label_color: ColorRgba8::rgba(255, 255, 255, 255),
            label_size: 14.0,
        }
    }
}

impl TreeViewStyle {
    fn resolve_item(self, selected: bool, active: bool, enabled: bool) -> BoxStyle {
        if !enabled {
            self.disabled_item.unwrap_or(self.item)
        } else if active {
            self.active_item
                .or(self.selected_item.filter(|_| selected))
                .unwrap_or(self.item)
        } else if selected {
            self.selected_item.unwrap_or(self.item)
        } else {
            self.item
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeViewNavigation<K> {
    command: CompositeNavigationCommand,
    change: CompositeChange<K>,
    expansion: Option<TreeExpansionProposal<K>>,
    selection: Option<SelectionProposal<K>>,
}

impl<K> TreeViewNavigation<K> {
    pub const fn command(&self) -> CompositeNavigationCommand {
        self.command
    }

    pub const fn change(&self) -> CompositeChange<K>
    where
        K: Copy,
    {
        self.change
    }

    pub const fn expansion(&self) -> Option<&TreeExpansionProposal<K>> {
        self.expansion.as_ref()
    }

    pub const fn selection(&self) -> Option<&SelectionProposal<K>> {
        self.selection.as_ref()
    }

    pub fn into_expansion(self) -> Option<TreeExpansionProposal<K>> {
        self.expansion
    }

    pub fn into_selection(self) -> Option<SelectionProposal<K>> {
        self.selection
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeViewActivation<K> {
    key: K,
    source: ChangeSource,
    selection: Option<SelectionProposal<K>>,
}

impl<K> TreeViewActivation<K> {
    pub const fn key(&self) -> &K {
        &self.key
    }

    pub const fn source(&self) -> ChangeSource {
        self.source
    }

    pub const fn selection(&self) -> Option<&SelectionProposal<K>> {
        self.selection.as_ref()
    }

    pub fn into_selection(self) -> Option<SelectionProposal<K>> {
        self.selection
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeViewExpansionTransition<K> {
    expansion: TreeExpansionTransition<K>,
    focus: CompositeChange<K>,
}

impl<K> TreeViewExpansionTransition<K> {
    pub const fn expansion(&self) -> &TreeExpansionTransition<K> {
        &self.expansion
    }

    pub const fn focus(&self) -> CompositeChange<K>
    where
        K: Copy,
    {
        self.focus
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TreeViewDiagnostics {
    pub navigation_requests: u64,
    pub boundaries: u64,
    pub activation_requests: u64,
    pub selection_requests: u64,
    pub selection_applies: u64,
    pub expansion_applies: u64,
    pub failures: u64,
}

#[derive(Clone, Debug)]
pub struct TreeView<K> {
    label: String,
    hierarchy: TreeHierarchy<K>,
    selection: SelectionModel<K>,
    composite: CompositeStateMachine<K>,
    density: DensityMetrics,
    style: TreeViewStyle,
    diagnostics: TreeViewDiagnostics,
}

impl<K> TreeView<K>
where
    K: Copy + Eq + Hash,
{
    pub fn new(
        label: impl Into<String>,
        hierarchy: TreeHierarchy<K>,
        selection: SelectionModel<K>,
    ) -> Result<Self, TreeViewError<K>> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(TreeViewError::MissingAccessibleName);
        }
        if selection.items() != hierarchy.keys() {
            return Err(TreeViewError::SelectionItemsMismatch);
        }
        let mut composite = CompositeStateMachine::new(standard_tree_policy());
        composite
            .update_items(visible_composite_items(&hierarchy))
            .map_err(TreeViewError::Composite)?;
        let selected = selection
            .selected()
            .iter()
            .copied()
            .find(|key| hierarchy.is_visible(key));
        composite
            .enter(selected)
            .map_err(TreeViewError::Composite)?;
        Ok(Self {
            label,
            hierarchy,
            selection,
            composite,
            density: DensityMetrics::baseline(DensityClass::Standard),
            style: TreeViewStyle::default(),
            diagnostics: TreeViewDiagnostics::default(),
        })
    }

    pub fn density(mut self, density: DensityMetrics) -> Self {
        self.density = density;
        self
    }

    pub fn style(mut self, style: TreeViewStyle) -> Self {
        self.style = style;
        self
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub const fn hierarchy(&self) -> &TreeHierarchy<K> {
        &self.hierarchy
    }

    pub const fn selection(&self) -> &SelectionModel<K> {
        &self.selection
    }

    pub fn active_item(&self) -> Option<K> {
        self.composite.active_descendant()
    }

    pub const fn diagnostics(&self) -> TreeViewDiagnostics {
        self.diagnostics
    }

    pub fn navigate(
        &mut self,
        command: CompositeNavigationCommand,
        direction: WritingDirection,
    ) -> Result<TreeViewNavigation<K>, TreeViewError<K>> {
        self.diagnostics.navigation_requests += 1;
        let opens = matches!(
            (command, direction),
            (
                CompositeNavigationCommand::Right,
                WritingDirection::LeftToRight
            ) | (
                CompositeNavigationCommand::Left,
                WritingDirection::RightToLeft
            )
        );
        let closes = matches!(
            (command, direction),
            (
                CompositeNavigationCommand::Left,
                WritingDirection::LeftToRight
            ) | (
                CompositeNavigationCommand::Right,
                WritingDirection::RightToLeft
            )
        );
        if opens || closes {
            return self.navigate_hierarchy(command, opens);
        }
        let change = self
            .composite
            .navigate(command, direction)
            .inspect_err(|_| self.diagnostics.failures += 1)
            .map_err(TreeViewError::Composite)?;
        self.navigation_from_change(command, change, None)
    }

    pub fn propose_item_activation(
        &mut self,
        key: K,
        source: ChangeSource,
    ) -> Result<TreeViewActivation<K>, TreeViewError<K>> {
        let item = self
            .hierarchy
            .item(&key)
            .ok_or(TreeViewError::UnknownItem(key))?;
        if !self.hierarchy.is_visible(&key) {
            self.diagnostics.failures += 1;
            return Err(TreeViewError::HiddenItem(key));
        }
        if !item.enabled {
            self.diagnostics.failures += 1;
            return Err(TreeViewError::DisabledItem(key));
        }
        self.composite
            .set_active_descendant(key)
            .inspect_err(|_| self.diagnostics.failures += 1)
            .map_err(TreeViewError::Composite)?;
        self.activation(key, source)
    }

    pub fn propose_active_activation(
        &mut self,
        source: ChangeSource,
    ) -> Result<TreeViewActivation<K>, TreeViewError<K>> {
        let key = self
            .composite
            .request_active_selection(source)
            .inspect_err(|_| self.diagnostics.failures += 1)
            .map_err(TreeViewError::Composite)?
            .key;
        self.activation(key, source)
    }

    pub fn propose_expansion(
        &mut self,
        key: K,
        expanded: bool,
        source: ChangeSource,
    ) -> Result<TreeExpansionProposal<K>, TreeViewError<K>> {
        self.hierarchy
            .propose_expansion(key, expanded, source)
            .map_err(TreeViewError::Hierarchy)
    }

    pub fn apply_expansion(
        &mut self,
        proposal: TreeExpansionProposal<K>,
    ) -> Result<TreeViewExpansionTransition<K>, TreeViewError<K>> {
        let expansion = self
            .hierarchy
            .apply_expansion(proposal)
            .inspect_err(|_| self.diagnostics.failures += 1)
            .map_err(TreeViewError::Hierarchy)?;
        let mut focus = CompositeChange::Unchanged;
        if let Some(active) = self.active_item()
            && !self.hierarchy.is_visible(&active)
            && let Some(ancestor) = self.nearest_visible_enabled_ancestor(active)
        {
            focus = self
                .composite
                .set_active_descendant(ancestor)
                .map_err(TreeViewError::Composite)?;
        }
        let update = self
            .composite
            .update_items(visible_composite_items(&self.hierarchy))
            .map_err(TreeViewError::Composite)?;
        if matches!(focus, CompositeChange::Unchanged) {
            focus = update;
        }
        if expansion.changed() {
            self.diagnostics.expansion_applies += 1;
        }
        Ok(TreeViewExpansionTransition { expansion, focus })
    }

    pub fn apply_selection(
        &mut self,
        proposal: SelectionProposal<K>,
    ) -> Result<SelectionTransition<K>, TreeViewError<K>> {
        let transition = self
            .selection
            .apply(proposal)
            .inspect_err(|_| self.diagnostics.failures += 1)
            .map_err(TreeViewError::Selection)?;
        self.diagnostics.selection_applies += 1;
        Ok(transition)
    }

    pub fn mount<Action, Map>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
        map: Map,
    ) -> RuntimeResult<TreeViewRef<K>>
    where
        K: 'static,
        Action: 'static,
        Map: Fn(TreeViewActivation<K>) -> Action + 'static,
    {
        let behavior = Rc::new(RefCell::new(self.clone()));
        let map = Rc::new(map);
        let visible: Vec<_> = self
            .hierarchy
            .items
            .iter()
            .filter(|item| self.hierarchy.is_visible(&item.key))
            .cloned()
            .collect();
        let total_count = u32::try_from(self.hierarchy.items.len())
            .map_err(|_| RuntimeError::new("tree exceeds semantic item capacity"))?;
        let root_count = u32::try_from(
            self.hierarchy
                .items
                .iter()
                .filter(|item| item.parent.is_none())
                .count(),
        )
        .map_err(|_| RuntimeError::new("tree exceeds semantic root capacity"))?;
        let active = self.active_item();
        let selectable = self.selection.mode() != SelectionMode::None;
        let any_enabled = visible.iter().any(|item| item.enabled);
        let minimum = self.density.effective_minimum();
        let mut mounted = Vec::with_capacity(visible.len());
        let root = ui
            .foundation()
            .button_node_under(host, self.style.root, |writer| {
                writer.container(
                    BoxStyle::default(),
                    LayoutStyle {
                        flow: Flow::Vertical,
                        gap: self.style.gap,
                        ..LayoutStyle::default()
                    },
                    |writer| {
                        for item in &visible {
                            let selected = self.selection.is_selected(&item.key);
                            let mut style = self.style.resolve_item(
                                selected,
                                active == Some(item.key),
                                item.enabled,
                            );
                            style.min_size = SizeRule2D {
                                width: SizeRule::Logical(minimum.width()),
                                height: SizeRule::Logical(minimum.height()),
                            };
                            let control = writer.action_node(style, false, |writer| {
                                writer.text(
                                    &item.label,
                                    self.style.label_color,
                                    self.style.label_size,
                                );
                            });
                            mounted.push((item.clone(), selected, control));
                        }
                    },
                );
            })
            .ok_or_else(|| RuntimeError::new("application tree host is stale"))?;
        if !any_enabled {
            ui.foundation().disabled(root.node, true);
        }

        let mut item_refs = Vec::with_capacity(mounted.len());
        for (item, selected, control) in mounted {
            let index = self
                .hierarchy
                .items
                .iter()
                .position(|candidate| candidate.key == item.key)
                .expect("mounted tree items belong to the hierarchy");
            let (position, set_size) = self
                .hierarchy
                .sibling_position(&item.key)
                .expect("mounted tree items have sibling metadata");
            let branch = self.hierarchy.is_branch(&item.key);
            let expanded = branch.then(|| self.hierarchy.is_expanded(&item.key));
            let mut actions = SemanticActions::NONE;
            if item.enabled && selectable {
                actions |= SemanticActions::SELECT;
            }
            if item.enabled && branch {
                actions |= if expanded == Some(true) {
                    SemanticActions::COLLAPSE
                } else {
                    SemanticActions::EXPAND
                };
            }
            let name = ui.foundation().intern(&item.label);
            ui.foundation()
                .semantic_node(
                    control.node,
                    SemanticNode {
                        role: SemanticRole::TreeItem,
                        name: SemanticName::Text(name),
                        state: SemanticState {
                            disabled: !item.enabled,
                            selected: selectable.then_some(selected),
                            expanded,
                            ..SemanticState::default()
                        },
                        actions,
                        collection: Some(SemanticCollection {
                            item_index: u32::try_from(index).ok(),
                            item_count: Some(total_count),
                            level: self.hierarchy.level(&item.key),
                            position_in_set: Some(position),
                            set_size: Some(set_size),
                        }),
                        ..SemanticNode::default()
                    },
                )
                .map_err(|error| {
                    RuntimeError::new(format!("invalid tree item semantics: {error:?}"))
                })?;
            if !item.enabled {
                ui.foundation().disabled(control.node, true);
            } else if selectable {
                let behavior = behavior.clone();
                let map = map.clone();
                let key = item.key;
                ui.route_activation_fallible(control.node, move |activation| {
                    let intent = behavior
                        .borrow_mut()
                        .propose_item_activation(key, activation.source)
                        .map_err(|_| RuntimeError::new("tree item activation failed"))?;
                    Ok(map(intent))
                })?;
            }
            item_refs.push(TreeItemRef {
                key: item.key,
                parent: item.parent,
                control,
                level: self
                    .hierarchy
                    .level(&item.key)
                    .expect("mounted tree items have levels"),
            });
        }

        let name = ui.foundation().intern(&self.label);
        let mut relationships: Vec<_> = item_refs
            .iter()
            .map(|item| owns(item.control.node))
            .collect();
        if let Some(active_node) = active.and_then(|key| {
            item_refs
                .iter()
                .find_map(|item| (item.key == key).then_some(item.control.node))
        }) {
            relationships.push(SemanticRelationship {
                kind: SemanticRelationshipKind::ActiveDescendant,
                target: active_node,
            });
        }
        ui.foundation()
            .semantic_node(
                root.node,
                SemanticNode {
                    role: SemanticRole::Tree,
                    name: SemanticName::Text(name),
                    state: SemanticState {
                        disabled: !any_enabled,
                        focusable: any_enabled,
                        ..SemanticState::default()
                    },
                    actions: if any_enabled {
                        SemanticActions::FOCUS
                            | if selectable {
                                SemanticActions::ACTIVATE
                            } else {
                                SemanticActions::NONE
                            }
                    } else {
                        SemanticActions::NONE
                    },
                    relationships,
                    collection: Some(SemanticCollection {
                        item_count: Some(total_count),
                        set_size: (root_count > 0).then_some(root_count),
                        ..SemanticCollection::default()
                    }),
                    ..SemanticNode::default()
                },
            )
            .map_err(|error| RuntimeError::new(format!("invalid tree semantics: {error:?}")))?;
        if any_enabled && selectable {
            let behavior = behavior.clone();
            let map = map.clone();
            ui.route_activation_fallible(root.node, move |activation| {
                let intent = behavior
                    .borrow_mut()
                    .propose_active_activation(activation.source)
                    .map_err(|_| RuntimeError::new("tree active activation failed"))?;
                Ok(map(intent))
            })?;
        }
        Ok(TreeViewRef {
            root,
            items: item_refs,
            behavior,
        })
    }

    fn navigate_hierarchy(
        &mut self,
        command: CompositeNavigationCommand,
        opens: bool,
    ) -> Result<TreeViewNavigation<K>, TreeViewError<K>> {
        let Some(active) = self.active_item() else {
            return Ok(self.boundary_navigation(command));
        };
        if opens {
            if !self.hierarchy.is_branch(&active) {
                return Ok(self.boundary_navigation(command));
            }
            if !self.hierarchy.is_expanded(&active) {
                let expansion = self
                    .hierarchy
                    .propose_expansion(active, true, ChangeSource::Directional)
                    .map_err(TreeViewError::Hierarchy)?;
                return Ok(TreeViewNavigation {
                    command,
                    change: CompositeChange::Unchanged,
                    expansion: Some(expansion),
                    selection: None,
                });
            }
            let child = self
                .hierarchy
                .children(&active)
                .into_iter()
                .find(|item| item.enabled)
                .map(|item| item.key);
            let Some(child) = child else {
                return Ok(self.boundary_navigation(command));
            };
            let change = self
                .composite
                .set_active_descendant(child)
                .map_err(TreeViewError::Composite)?;
            self.navigation_from_change(command, change, None)
        } else if self.hierarchy.is_branch(&active) && self.hierarchy.is_expanded(&active) {
            let expansion = self
                .hierarchy
                .propose_expansion(active, false, ChangeSource::Directional)
                .map_err(TreeViewError::Hierarchy)?;
            Ok(TreeViewNavigation {
                command,
                change: CompositeChange::Unchanged,
                expansion: Some(expansion),
                selection: None,
            })
        } else {
            let Some(parent) = self.nearest_visible_enabled_ancestor(active) else {
                return Ok(self.boundary_navigation(command));
            };
            let change = self
                .composite
                .set_active_descendant(parent)
                .map_err(TreeViewError::Composite)?;
            self.navigation_from_change(command, change, None)
        }
    }

    fn navigation_from_change(
        &mut self,
        command: CompositeNavigationCommand,
        change: CompositeChange<K>,
        expansion: Option<TreeExpansionProposal<K>>,
    ) -> Result<TreeViewNavigation<K>, TreeViewError<K>> {
        if matches!(change, CompositeChange::Boundary { .. }) {
            self.diagnostics.boundaries += 1;
        }
        let selection = match change {
            CompositeChange::Highlighted { current, .. } => self
                .selection
                .propose_focus(&current, ChangeSource::Directional)
                .map_err(TreeViewError::Selection)?,
            _ => None,
        };
        if selection.is_some() {
            self.diagnostics.selection_requests += 1;
        }
        Ok(TreeViewNavigation {
            command,
            change,
            expansion,
            selection,
        })
    }

    fn boundary_navigation(
        &mut self,
        command: CompositeNavigationCommand,
    ) -> TreeViewNavigation<K> {
        self.diagnostics.boundaries += 1;
        TreeViewNavigation {
            command,
            change: CompositeChange::Boundary {
                current: self
                    .composite
                    .active_target()
                    .unwrap_or(CompositeFocusTarget::Root),
                command,
            },
            expansion: None,
            selection: None,
        }
    }

    fn nearest_visible_enabled_ancestor(&self, key: K) -> Option<K> {
        let mut parent = self.hierarchy.item(&key)?.parent;
        while let Some(key) = parent {
            let item = self
                .hierarchy
                .item(&key)
                .expect("validated tree parents remain available");
            if item.enabled && self.hierarchy.is_visible(&key) {
                return Some(key);
            }
            parent = item.parent;
        }
        None
    }

    fn activation(
        &mut self,
        key: K,
        source: ChangeSource,
    ) -> Result<TreeViewActivation<K>, TreeViewError<K>> {
        self.diagnostics.activation_requests += 1;
        let selection = match self.selection.mode() {
            SelectionMode::None => None,
            SelectionMode::Single => Some(self.selection.propose_select(&key, source)),
            SelectionMode::Multiple => Some(self.selection.propose_toggle(&key, source)),
        }
        .transpose()
        .map_err(TreeViewError::Selection)?;
        if selection.is_some() {
            self.diagnostics.selection_requests += 1;
        }
        Ok(TreeViewActivation {
            key,
            source,
            selection,
        })
    }
}

pub const fn standard_tree_policy() -> CompositeNavigationPolicy {
    CompositeNavigationPolicy {
        orientation: CompositeOrientation::Vertical,
        edge_behavior: CompositeEdgeBehavior::Stop,
        disabled_items: DisabledItemPolicy::Skip,
        selection: CompositeSelectionBehavior::Independent,
    }
}

#[derive(Clone, Debug)]
pub struct TreeViewRef<K: 'static> {
    root: ControlHandle,
    items: Vec<TreeItemRef<K>>,
    behavior: Rc<RefCell<TreeView<K>>>,
}

impl<K> TreeViewRef<K>
where
    K: Copy + Eq + Hash + 'static,
{
    pub const fn node(&self) -> UiNodeId {
        self.root.node
    }

    pub fn items(&self) -> &[TreeItemRef<K>] {
        &self.items
    }

    pub fn active_item(&self) -> Option<K> {
        self.behavior.borrow().active_item()
    }

    pub fn navigate(
        &self,
        command: CompositeNavigationCommand,
        direction: WritingDirection,
    ) -> Result<TreeViewNavigation<K>, TreeViewError<K>> {
        self.behavior.borrow_mut().navigate(command, direction)
    }

    pub fn propose_active_activation(
        &self,
        source: ChangeSource,
    ) -> Result<TreeViewActivation<K>, TreeViewError<K>> {
        self.behavior.borrow_mut().propose_active_activation(source)
    }

    pub const fn style(&self) -> Property<BoxStyle> {
        self.root.style
    }
}

#[derive(Clone, Debug)]
pub struct TreeItemRef<K> {
    key: K,
    parent: Option<K>,
    control: ControlHandle,
    level: u32,
}

impl<K> TreeItemRef<K> {
    pub const fn key(&self) -> &K {
        &self.key
    }

    pub const fn parent(&self) -> Option<&K> {
        self.parent.as_ref()
    }

    pub const fn node(&self) -> UiNodeId {
        self.control.node
    }

    pub const fn level(&self) -> u32 {
        self.level
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeViewError<K> {
    MissingAccessibleName,
    SelectionItemsMismatch,
    UnknownItem(K),
    HiddenItem(K),
    DisabledItem(K),
    Hierarchy(TreeHierarchyError<K>),
    Selection(SelectionError<K>),
    Composite(CompositeError<K>),
}

impl<K: fmt::Debug> fmt::Display for TreeViewError<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "tree-view operation failed: {self:?}")
    }
}

impl<K: fmt::Debug> std::error::Error for TreeViewError<K> {}

fn validate_hierarchy<K>(items: &[TreeItem<K>]) -> Result<(), TreeHierarchyError<K>>
where
    K: Copy + Eq + Hash,
{
    let mut seen = HashSet::with_capacity(items.len());
    for item in items {
        if !seen.insert(item.key) {
            return Err(TreeHierarchyError::DuplicateKey(item.key));
        }
        if item.parent == Some(item.key) {
            return Err(TreeHierarchyError::SelfParent(item.key));
        }
        if let Some(parent) = item.parent
            && !seen.contains(&parent)
        {
            return Err(TreeHierarchyError::ParentMustPrecede {
                key: item.key,
                parent,
            });
        }
    }
    let mut expected = Vec::with_capacity(items.len());
    for (index, _item) in items
        .iter()
        .enumerate()
        .filter(|(_, item)| item.parent.is_none())
    {
        append_preorder(items, index, &mut expected);
    }
    if let Some((position, actual)) = expected
        .iter()
        .copied()
        .enumerate()
        .find(|(position, actual)| position != actual)
    {
        let _ = position;
        return Err(TreeHierarchyError::NonContiguousSubtree(items[actual].key));
    }
    Ok(())
}

fn append_preorder<K>(items: &[TreeItem<K>], index: usize, output: &mut Vec<usize>)
where
    K: Copy + Eq,
{
    output.push(index);
    let key = items[index].key;
    for (child_index, _) in items
        .iter()
        .enumerate()
        .filter(|(_, item)| item.parent == Some(key))
    {
        append_preorder(items, child_index, output);
    }
}

fn validate_expanded<K>(
    items: &[TreeItem<K>],
    mut expanded: Vec<K>,
) -> Result<Vec<K>, TreeHierarchyError<K>>
where
    K: Copy + Eq + Hash,
{
    let mut seen = HashSet::with_capacity(expanded.len());
    for key in &expanded {
        if !seen.insert(*key) {
            return Err(TreeHierarchyError::DuplicateExpandedKey(*key));
        }
        if !items.iter().any(|item| item.key == *key) {
            return Err(TreeHierarchyError::UnknownExpandedKey(*key));
        }
        if !items.iter().any(|item| item.parent == Some(*key)) {
            return Err(TreeHierarchyError::LeafExpansion(*key));
        }
    }
    expanded.sort_by_key(|key| {
        items
            .iter()
            .position(|item| item.key == *key)
            .expect("expanded keys are validated")
    });
    Ok(expanded)
}

fn visible_composite_items<K>(
    hierarchy: &TreeHierarchy<K>,
) -> impl Iterator<Item = CompositeItem<K>> + '_
where
    K: Copy + Eq + Hash,
{
    hierarchy
        .items
        .iter()
        .filter(|item| hierarchy.is_visible(&item.key))
        .map(|item| CompositeItem {
            key: item.key,
            enabled: item.enabled,
        })
}

fn owns(target: UiNodeId) -> SemanticRelationship {
    SemanticRelationship {
        kind: SemanticRelationshipKind::Owns,
        target,
    }
}

#[cfg(test)]
mod tests;

//! Stable-key listbox options with one composite focus entry.
//!
//! `SelectionModel` remains the only selection owner and `CompositeStateMachine` remains the only
//! active-descendant owner. `ListBox` validates and adapts their outputs without owning collection
//! data, scrolling, virtualization, input translation, or platform services.

use std::cell::RefCell;
use std::collections::HashSet;
use std::fmt;
use std::hash::Hash;
use std::rc::Rc;

use crate::foundation::ColorRgba8;
use crate::input::{
    ChangeSource, CompositeChange, CompositeEdgeBehavior, CompositeError, CompositeItem,
    CompositeNavigationCommand, CompositeNavigationPolicy, CompositeOrientation,
    CompositeSelectionBehavior, CompositeStateMachine, DisabledItemPolicy, WritingDirection,
};
use crate::runtime::{RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    BoxStyle, ControlHandle, Flow, LayoutStyle, Property, SemanticActions, SemanticCollection,
    SemanticName, SemanticNode, SemanticRelationship, SemanticRelationshipKind, SemanticRole,
    SemanticState, SizeRule, SizeRule2D, UiNodeId,
};

use super::{
    SelectionError, SelectionItemsUpdate, SelectionMode, SelectionModel, SelectionProposal,
    SelectionTransition,
};
use crate::components::application::{DensityClass, DensityMetrics};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListBoxOption<K> {
    key: K,
    label: String,
    enabled: bool,
}

impl<K> ListBoxOption<K> {
    pub fn new(key: K, label: impl Into<String>) -> Result<Self, ListBoxOptionError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(ListBoxOptionError::MissingAccessibleName);
        }
        Ok(Self {
            key,
            label,
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

    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListBoxOptionError {
    MissingAccessibleName,
}

impl fmt::Display for ListBoxOptionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("listbox option accessible name is empty")
    }
}

impl std::error::Error for ListBoxOptionError {}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ListBoxStyle {
    pub container: BoxStyle,
    pub option: BoxStyle,
    pub selected_option: Option<BoxStyle>,
    pub active_option: Option<BoxStyle>,
    pub disabled_option: Option<BoxStyle>,
    pub gap: f32,
    pub label_color: ColorRgba8,
    pub label_size: f32,
}

impl ListBoxStyle {
    fn resolve_option(self, selected: bool, active: bool, enabled: bool) -> BoxStyle {
        if !enabled {
            self.disabled_option.unwrap_or(self.option)
        } else if active {
            self.active_option
                .or(self.selected_option.filter(|_| selected))
                .unwrap_or(self.option)
        } else if selected {
            self.selected_option.unwrap_or(self.option)
        } else {
            self.option
        }
    }
}

impl Default for ListBoxStyle {
    fn default() -> Self {
        Self {
            container: BoxStyle::default(),
            option: BoxStyle::default(),
            selected_option: None,
            active_option: None,
            disabled_option: None,
            gap: 0.0,
            label_color: ColorRgba8::rgba(255, 255, 255, 255),
            label_size: 14.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListBoxTransition<K> {
    change: CompositeChange<K>,
    selection: Option<SelectionProposal<K>>,
}

impl<K> ListBoxTransition<K> {
    pub const fn change(&self) -> CompositeChange<K>
    where
        K: Copy,
    {
        self.change
    }

    pub const fn selection(&self) -> Option<&SelectionProposal<K>> {
        self.selection.as_ref()
    }

    pub fn into_selection(self) -> Option<SelectionProposal<K>> {
        self.selection
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListBoxSelectionRequest<K> {
    change: CompositeChange<K>,
    selection: SelectionProposal<K>,
}

impl<K> ListBoxSelectionRequest<K> {
    pub const fn change(&self) -> CompositeChange<K>
    where
        K: Copy,
    {
        self.change
    }

    pub const fn selection(&self) -> &SelectionProposal<K> {
        &self.selection
    }

    pub fn into_selection(self) -> SelectionProposal<K> {
        self.selection
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListBoxItemsUpdate<K> {
    selection: SelectionItemsUpdate<K>,
    focus: CompositeChange<K>,
    changed: bool,
    revision: u64,
}

impl<K> ListBoxItemsUpdate<K> {
    pub const fn selection(&self) -> &SelectionItemsUpdate<K> {
        &self.selection
    }

    pub const fn focus(&self) -> CompositeChange<K>
    where
        K: Copy,
    {
        self.focus
    }

    pub const fn changed(&self) -> bool {
        self.changed
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ListBoxDiagnostics {
    pub navigation_requests: u64,
    pub selection_requests: u64,
    pub selection_applies: u64,
    pub option_updates: u64,
    pub failures: u64,
}

#[derive(Clone, Debug)]
pub struct ListBox<K> {
    label: String,
    options: Vec<ListBoxOption<K>>,
    selection: SelectionModel<K>,
    composite: CompositeStateMachine<K>,
    density: DensityMetrics,
    style: ListBoxStyle,
    revision: u64,
    diagnostics: ListBoxDiagnostics,
}

impl<K> ListBox<K>
where
    K: Copy + Eq + Hash,
{
    pub fn new(
        label: impl Into<String>,
        options: impl IntoIterator<Item = ListBoxOption<K>>,
        selection: SelectionModel<K>,
    ) -> Result<Self, ListBoxError<K>> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(ListBoxError::MissingAccessibleName);
        }
        let options: Vec<_> = options.into_iter().collect();
        validate_options(&options)?;
        let keys: Vec<_> = options.iter().map(|option| option.key).collect();
        if selection.items() != keys {
            return Err(ListBoxError::SelectionItemsMismatch);
        }
        let mut composite = CompositeStateMachine::new(standard_listbox_policy());
        composite
            .update_items(options.iter().map(composite_item))
            .map_err(ListBoxError::Composite)?;
        composite
            .enter(selection.selected().first().copied())
            .map_err(ListBoxError::Composite)?;
        Ok(Self {
            label,
            options,
            selection,
            composite,
            density: DensityMetrics::baseline(DensityClass::Standard),
            style: ListBoxStyle::default(),
            revision: 1,
            diagnostics: ListBoxDiagnostics::default(),
        })
    }

    pub fn density(mut self, density: DensityMetrics) -> Self {
        self.density = density;
        self
    }

    pub fn style(mut self, style: ListBoxStyle) -> Self {
        self.style = style;
        self
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn options(&self) -> &[ListBoxOption<K>] {
        &self.options
    }

    pub const fn selection(&self) -> &SelectionModel<K> {
        &self.selection
    }

    pub fn active_descendant(&self) -> Option<K> {
        self.composite.active_descendant()
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }

    pub const fn diagnostics(&self) -> ListBoxDiagnostics {
        self.diagnostics
    }

    pub fn navigate(
        &mut self,
        command: CompositeNavigationCommand,
        direction: WritingDirection,
    ) -> Result<ListBoxTransition<K>, ListBoxError<K>> {
        let change = self
            .composite
            .navigate(command, direction)
            .inspect_err(|_| self.diagnostics.failures += 1)
            .map_err(ListBoxError::Composite)?;
        self.diagnostics.navigation_requests += 1;
        let selection = match change {
            CompositeChange::Highlighted { current, .. } => self
                .selection
                .propose_focus(&current, ChangeSource::Directional)
                .inspect_err(|_| self.diagnostics.failures += 1)
                .map_err(ListBoxError::Selection)?,
            _ => None,
        };
        if selection.is_some() {
            self.diagnostics.selection_requests += 1;
        }
        Ok(ListBoxTransition { change, selection })
    }

    pub fn propose_option_selection(
        &mut self,
        key: &K,
        source: ChangeSource,
    ) -> Result<ListBoxSelectionRequest<K>, ListBoxError<K>> {
        let option = self
            .options
            .iter()
            .find(|option| &option.key == key)
            .ok_or(ListBoxError::UnknownOption(*key))?;
        if !option.enabled {
            self.diagnostics.failures += 1;
            return Err(ListBoxError::DisabledOption(*key));
        }
        let change = self
            .composite
            .set_active_descendant(*key)
            .inspect_err(|_| self.diagnostics.failures += 1)
            .map_err(ListBoxError::Composite)?;
        let selection = self.propose_key_selection(key, source)?;
        Ok(ListBoxSelectionRequest { change, selection })
    }

    pub fn propose_active_selection(
        &mut self,
        source: ChangeSource,
    ) -> Result<SelectionProposal<K>, ListBoxError<K>> {
        let request = self
            .composite
            .request_active_selection(source)
            .inspect_err(|_| self.diagnostics.failures += 1)
            .map_err(ListBoxError::Composite)?;
        self.propose_key_selection(&request.key, request.source)
    }

    pub fn apply_selection(
        &mut self,
        proposal: SelectionProposal<K>,
    ) -> Result<SelectionTransition<K>, ListBoxError<K>> {
        let transition = self
            .selection
            .apply(proposal)
            .inspect_err(|_| self.diagnostics.failures += 1)
            .map_err(ListBoxError::Selection)?;
        self.diagnostics.selection_applies += 1;
        Ok(transition)
    }

    pub fn update_options(
        &mut self,
        options: impl IntoIterator<Item = ListBoxOption<K>>,
    ) -> Result<ListBoxItemsUpdate<K>, ListBoxError<K>> {
        let options: Vec<_> = options.into_iter().collect();
        validate_options(&options).inspect_err(|_| self.diagnostics.failures += 1)?;
        let changed = options != self.options;
        let revision = if changed {
            self.revision
                .checked_add(1)
                .ok_or(ListBoxError::RevisionExhausted)?
        } else {
            self.revision
        };
        let keys: Vec<_> = options.iter().map(|option| option.key).collect();
        let selection = self
            .selection
            .update_items(keys)
            .inspect_err(|_| self.diagnostics.failures += 1)
            .map_err(ListBoxError::Selection)?;
        let focus = self
            .composite
            .update_items(options.iter().map(composite_item))
            .unwrap_or_else(|_| unreachable!("validated listbox option keys are unique"));
        if changed {
            self.options = options;
            self.revision = revision;
            self.diagnostics.option_updates += 1;
        }
        Ok(ListBoxItemsUpdate {
            selection,
            focus,
            changed,
            revision,
        })
    }

    pub fn mount<Action, Map>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
        map: Map,
    ) -> RuntimeResult<ListBoxRef<K>>
    where
        K: 'static,
        Action: 'static,
        Map: Fn(SelectionProposal<K>) -> Action + 'static,
    {
        let item_count = u32::try_from(self.options.len())
            .map_err(|_| RuntimeError::new("listbox exceeds semantic item capacity"))?;
        let behavior = Rc::new(RefCell::new(self.clone()));
        let map = Rc::new(map);
        let active = self.active_descendant();
        let any_enabled = self.options.iter().any(ListBoxOption::is_enabled);
        let selectable = self.selection.mode() != SelectionMode::None;
        let minimum = self.density.effective_minimum();
        let mut mounted = Vec::with_capacity(self.options.len());
        let root = ui
            .foundation()
            .button_node_under(host, self.style.container, |writer| {
                writer.container(
                    BoxStyle::default(),
                    LayoutStyle {
                        flow: Flow::Vertical,
                        gap: self.style.gap,
                        ..LayoutStyle::default()
                    },
                    |writer| {
                        for (index, option) in self.options.iter().enumerate() {
                            let selected = self.selection.is_selected(&option.key);
                            let mut style = self.style.resolve_option(
                                selected,
                                active == Some(option.key),
                                option.enabled,
                            );
                            style.min_size = SizeRule2D {
                                width: SizeRule::Logical(minimum.width()),
                                height: SizeRule::Logical(minimum.height()),
                            };
                            let control = writer.action_node(style, false, |writer| {
                                writer.text(
                                    &option.label,
                                    self.style.label_color,
                                    self.style.label_size,
                                );
                            });
                            mounted.push((index, option.clone(), selected, control));
                        }
                    },
                );
            })
            .ok_or_else(|| RuntimeError::new("application listbox host is stale"))?;

        if !any_enabled {
            ui.foundation().disabled(root.node, true);
        }
        let mut option_refs = Vec::with_capacity(mounted.len());
        for (index, option, selected, control) in mounted {
            let name = ui.foundation().intern(&option.label);
            ui.foundation()
                .semantic_node(
                    control.node,
                    SemanticNode {
                        role: SemanticRole::Option,
                        name: SemanticName::Text(name),
                        state: SemanticState {
                            disabled: !option.enabled,
                            selected: Some(selected),
                            ..SemanticState::default()
                        },
                        actions: if option.enabled && selectable {
                            SemanticActions::SELECT
                        } else {
                            SemanticActions::NONE
                        },
                        collection: Some(SemanticCollection {
                            item_index: u32::try_from(index).ok(),
                            item_count: Some(item_count),
                            position_in_set: u32::try_from(index + 1).ok(),
                            set_size: Some(item_count),
                            ..SemanticCollection::default()
                        }),
                        ..SemanticNode::default()
                    },
                )
                .map_err(|error| {
                    RuntimeError::new(format!("invalid listbox option semantics: {error:?}"))
                })?;
            if !option.enabled {
                ui.foundation().disabled(control.node, true);
            } else if selectable {
                let behavior = behavior.clone();
                let map = map.clone();
                let key = option.key;
                ui.route_activation_fallible(control.node, move |activation| {
                    let proposal = behavior
                        .borrow_mut()
                        .propose_option_selection(&key, activation.source)
                        .map_err(|_| RuntimeError::new("listbox option selection failed"))?
                        .into_selection();
                    Ok(map(proposal))
                })?;
            }
            option_refs.push(ListBoxOptionRef {
                key: option.key,
                control,
                index,
                enabled: option.enabled,
            });
        }

        let name = ui.foundation().intern(&self.label);
        let mut relationships: Vec<_> = option_refs
            .iter()
            .map(|option| SemanticRelationship {
                kind: SemanticRelationshipKind::Owns,
                target: option.control.node,
            })
            .collect();
        if let Some(active) = active.and_then(|key| {
            option_refs
                .iter()
                .find_map(|option| (option.key == key).then_some(option.control.node))
        }) {
            relationships.push(SemanticRelationship {
                kind: SemanticRelationshipKind::ActiveDescendant,
                target: active,
            });
        }
        ui.foundation()
            .semantic_node(
                root.node,
                SemanticNode {
                    role: SemanticRole::ListBox,
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
                        item_count: Some(item_count),
                        set_size: (item_count > 0).then_some(item_count),
                        ..SemanticCollection::default()
                    }),
                    ..SemanticNode::default()
                },
            )
            .map_err(|error| RuntimeError::new(format!("invalid listbox semantics: {error:?}")))?;
        if any_enabled && selectable {
            let behavior = behavior.clone();
            let map = map.clone();
            ui.route_activation_fallible(root.node, move |activation| {
                let proposal = behavior
                    .borrow_mut()
                    .propose_active_selection(activation.source)
                    .map_err(|_| RuntimeError::new("listbox active selection failed"))?;
                Ok(map(proposal))
            })?;
        }
        Ok(ListBoxRef {
            root,
            options: option_refs,
            behavior,
        })
    }

    fn propose_key_selection(
        &mut self,
        key: &K,
        source: ChangeSource,
    ) -> Result<SelectionProposal<K>, ListBoxError<K>> {
        let proposal = match self.selection.mode() {
            SelectionMode::None => {
                self.diagnostics.failures += 1;
                return Err(ListBoxError::Selection(SelectionError::SelectionDisabled));
            }
            SelectionMode::Single => self.selection.propose_select(key, source),
            SelectionMode::Multiple => self.selection.propose_toggle(key, source),
        }
        .inspect_err(|_| self.diagnostics.failures += 1)
        .map_err(ListBoxError::Selection)?;
        self.diagnostics.selection_requests += 1;
        Ok(proposal)
    }
}

pub const fn standard_listbox_policy() -> CompositeNavigationPolicy {
    CompositeNavigationPolicy {
        orientation: CompositeOrientation::Vertical,
        edge_behavior: CompositeEdgeBehavior::Stop,
        disabled_items: DisabledItemPolicy::Skip,
        selection: CompositeSelectionBehavior::Independent,
    }
}

#[derive(Clone, Debug)]
pub struct ListBoxRef<K: 'static> {
    root: ControlHandle,
    options: Vec<ListBoxOptionRef<K>>,
    behavior: Rc<RefCell<ListBox<K>>>,
}

impl<K> ListBoxRef<K>
where
    K: Copy + Eq + Hash + 'static,
{
    pub const fn node(&self) -> UiNodeId {
        self.root.node
    }

    pub fn options(&self) -> &[ListBoxOptionRef<K>] {
        &self.options
    }

    pub fn active_descendant(&self) -> Option<K> {
        self.behavior.borrow().active_descendant()
    }

    pub fn navigate(
        &self,
        command: CompositeNavigationCommand,
        direction: WritingDirection,
    ) -> Result<ListBoxTransition<K>, ListBoxError<K>> {
        self.behavior.borrow_mut().navigate(command, direction)
    }

    pub fn propose_active_selection(
        &self,
        source: ChangeSource,
    ) -> Result<SelectionProposal<K>, ListBoxError<K>> {
        self.behavior.borrow_mut().propose_active_selection(source)
    }

    pub const fn style(&self) -> Property<BoxStyle> {
        self.root.style
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ListBoxOptionRef<K> {
    key: K,
    control: ControlHandle,
    index: usize,
    enabled: bool,
}

impl<K: Copy> ListBoxOptionRef<K> {
    pub const fn key(self) -> K {
        self.key
    }

    pub const fn node(self) -> UiNodeId {
        self.control.node
    }

    pub const fn index(self) -> usize {
        self.index
    }

    pub const fn is_enabled(self) -> bool {
        self.enabled
    }

    pub const fn style(self) -> Property<BoxStyle> {
        self.control.style
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListBoxError<K> {
    MissingAccessibleName,
    DuplicateKey(K),
    SelectionItemsMismatch,
    UnknownOption(K),
    DisabledOption(K),
    Selection(SelectionError<K>),
    Composite(CompositeError<K>),
    RevisionExhausted,
}

impl<K: fmt::Debug> fmt::Display for ListBoxError<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "listbox operation failed: {self:?}")
    }
}

impl<K: fmt::Debug> std::error::Error for ListBoxError<K> {}

fn validate_options<K>(options: &[ListBoxOption<K>]) -> Result<(), ListBoxError<K>>
where
    K: Copy + Eq + Hash,
{
    let mut keys = HashSet::with_capacity(options.len());
    for option in options {
        if !keys.insert(option.key) {
            return Err(ListBoxError::DuplicateKey(option.key));
        }
    }
    Ok(())
}

fn composite_item<K: Copy>(option: &ListBoxOption<K>) -> CompositeItem<K> {
    CompositeItem {
        key: option.key,
        enabled: option.enabled,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::runtime::{Component, CreateContext, UpdateContext, ViewRuntime};
    use crate::ui::{SemanticAction, SemanticRelationshipKind, UiRoot};

    use super::*;
    use crate::components::application::SelectionFollowsFocus;

    type RecordedProposals = Rc<RefCell<Vec<(Vec<u8>, ChangeSource)>>>;

    fn option(key: u8, enabled: bool) -> ListBoxOption<u8> {
        ListBoxOption::new(key, format!("Option {key}"))
            .unwrap()
            .enabled(enabled)
    }

    fn selection(
        mode: SelectionMode,
        follows: SelectionFollowsFocus,
        selected: impl IntoIterator<Item = u8>,
    ) -> SelectionModel<u8> {
        SelectionModel::new(mode, follows, [1, 2, 3], selected, None).unwrap()
    }

    #[test]
    fn construction_validates_names_unique_keys_and_selection_item_identity() {
        assert_eq!(
            ListBoxOption::new(1_u8, " "),
            Err(ListBoxOptionError::MissingAccessibleName)
        );
        assert!(matches!(
            ListBox::new(
                " ",
                [option(1, true), option(2, true), option(3, true)],
                selection(SelectionMode::Single, SelectionFollowsFocus::Disabled, [])
            ),
            Err(ListBoxError::MissingAccessibleName)
        ));
        assert!(matches!(
            ListBox::new(
                "Options",
                [option(1, true), option(1, true)],
                SelectionModel::new(
                    SelectionMode::Single,
                    SelectionFollowsFocus::Disabled,
                    [1, 2],
                    [],
                    None
                )
                .unwrap()
            ),
            Err(ListBoxError::DuplicateKey(1))
        ));
        assert!(matches!(
            ListBox::new(
                "Options",
                [option(1, true), option(2, true), option(3, true)],
                SelectionModel::new(
                    SelectionMode::Single,
                    SelectionFollowsFocus::Disabled,
                    [3, 2, 1],
                    [],
                    None
                )
                .unwrap()
            ),
            Err(ListBoxError::SelectionItemsMismatch)
        ));
    }

    #[test]
    fn navigation_skips_disabled_and_focus_selection_never_collapses_multiple() {
        let mut listbox = ListBox::new(
            "Options",
            [option(1, true), option(2, false), option(3, true)],
            selection(SelectionMode::Multiple, SelectionFollowsFocus::Enabled, [1]),
        )
        .unwrap();
        let moved = listbox
            .navigate(
                CompositeNavigationCommand::Down,
                WritingDirection::LeftToRight,
            )
            .unwrap();
        assert_eq!(listbox.active_descendant(), Some(3));
        assert_eq!(moved.selection().unwrap().selected(), &[1, 3]);
        assert_eq!(
            moved.selection().unwrap().source(),
            ChangeSource::Directional
        );
        assert_eq!(listbox.selection().selected(), &[1]);
        let boundary = listbox
            .navigate(
                CompositeNavigationCommand::End,
                WritingDirection::LeftToRight,
            )
            .unwrap();
        assert!(matches!(
            boundary.change(),
            CompositeChange::Boundary { .. }
        ));
        listbox
            .navigate(
                CompositeNavigationCommand::Home,
                WritingDirection::LeftToRight,
            )
            .unwrap();
        assert_eq!(listbox.active_descendant(), Some(1));
    }

    #[test]
    fn activation_is_source_preserving_and_updates_recover_focus_and_selection_by_key() {
        let mut listbox = ListBox::new(
            "Options",
            [option(1, true), option(2, false), option(3, true)],
            selection(SelectionMode::Single, SelectionFollowsFocus::Disabled, [1]),
        )
        .unwrap();
        let request = listbox
            .propose_option_selection(&3, ChangeSource::Accessibility)
            .unwrap();
        assert_eq!(request.selection().selected(), &[3]);
        assert_eq!(request.selection().source(), ChangeSource::Accessibility);
        listbox.apply_selection(request.into_selection()).unwrap();
        assert_eq!(listbox.selection().selected(), &[3]);
        assert_eq!(
            listbox.propose_option_selection(&2, ChangeSource::Pointer),
            Err(ListBoxError::DisabledOption(2))
        );
        let update = listbox
            .update_options([option(1, true), option(2, true)])
            .unwrap();
        assert_eq!(update.selection().removed_selected(), &[3]);
        assert!(listbox.selection().selected().is_empty());
        assert_eq!(listbox.active_descendant(), Some(2));
    }

    #[derive(Clone, Debug)]
    enum MountedAction {
        Proposed(SelectionProposal<u8>),
    }

    struct MountedListBox {
        mounted: Rc<RefCell<Option<ListBoxRef<u8>>>>,
        proposals: RecordedProposals,
    }

    impl Component for MountedListBox {
        type State = ListBox<u8>;
        type Action = MountedAction;

        fn create(&self, _context: &mut CreateContext<'_>) -> Self::State {
            ListBox::new(
                "Options",
                [option(1, true), option(2, false), option(3, true)],
                selection(SelectionMode::Single, SelectionFollowsFocus::Disabled, [1]),
            )
            .unwrap()
            .density(DensityMetrics::baseline(DensityClass::Touch))
        }

        fn mount(&self, state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            self.mounted.replace(Some(
                state.mount(ui, root.0, MountedAction::Proposed).unwrap(),
            ));
            root
        }

        fn action(
            &self,
            _state: &mut Self::State,
            action: Self::Action,
            _context: &mut UpdateContext<'_, Self>,
        ) {
            let MountedAction::Proposed(proposal) = action;
            self.proposals
                .borrow_mut()
                .push((proposal.selected().to_vec(), proposal.source()));
        }
    }

    #[test]
    fn mount_has_one_focus_entry_option_semantics_and_source_preserving_routes() {
        let mounted = Rc::new(RefCell::new(None));
        let proposals = Rc::new(RefCell::new(Vec::new()));
        let mut runtime = ViewRuntime::from_component(MountedListBox {
            mounted: mounted.clone(),
            proposals: proposals.clone(),
        })
        .unwrap();
        let mounted = mounted.borrow();
        let mounted = mounted.as_ref().unwrap();
        assert!(
            runtime
                .ui()
                .interactions
                .get(mounted.node())
                .unwrap()
                .focusable
        );
        assert!(mounted.options().iter().all(|option| {
            !runtime
                .ui()
                .interactions
                .get(option.node())
                .is_some_and(|interaction| interaction.focusable)
        }));
        let root = runtime.ui().semantics.get(mounted.node()).unwrap();
        assert_eq!(root.role, SemanticRole::ListBox);
        assert_eq!(root.collection.unwrap().item_count, Some(3));
        assert_eq!(
            root.relationships.last().unwrap().kind,
            SemanticRelationshipKind::ActiveDescendant
        );
        let first = runtime
            .ui()
            .semantics
            .get(mounted.options()[0].node())
            .unwrap();
        assert_eq!(first.role, SemanticRole::Option);
        assert_eq!(first.state.selected, Some(true));
        assert!(first.actions.contains(SemanticAction::Select));
        let disabled = runtime
            .ui()
            .semantics
            .get(mounted.options()[1].node())
            .unwrap();
        assert!(disabled.state.disabled);
        assert!(disabled.effective_actions().is_empty());
        assert_eq!(
            runtime
                .ui()
                .box_styles
                .get(mounted.options()[0].node())
                .unwrap()
                .min_size,
            SizeRule2D {
                width: SizeRule::Logical(44.0),
                height: SizeRule::Logical(44.0),
            }
        );
        assert!(
            runtime.dispatch_activation(mounted.options()[2].node(), ChangeSource::Accessibility)
        );
        assert!(!runtime.dispatch_activation(mounted.options()[1].node(), ChangeSource::Pointer));
        assert!(runtime.dispatch_action(mounted.node()));
        assert_eq!(
            &*proposals.borrow(),
            &[
                (vec![3], ChangeSource::Accessibility),
                (vec![3], ChangeSource::Programmatic),
            ]
        );
    }
}

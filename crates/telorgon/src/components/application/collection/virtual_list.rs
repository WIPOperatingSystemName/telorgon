//! Stable-key virtualized ordinary list rows.
//!
//! `VirtualListView` turns explicit viewport geometry and keyed extent observations into a bounded
//! materialization plan. The neutral layout owner remains responsible for extent indexing, and the
//! caller remains responsible for collection data, scroll state/physics, selection, and focus.

use std::fmt;
use std::ops::Range;

use crate::foundation::RectF;
use crate::ui::layout::{RevealAlignment, RevealRequest, VirtualCollection};
use crate::runtime::{RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    BoxStyle, ControlHandle, Flow, LayoutStyle, MountWriter, Property, SemanticCollection,
    SemanticName, SemanticNode, SemanticRelationship, SemanticRelationshipKind, SemanticRole,
    SizeRule, SizeRule2D, UiNodeId,
};

use super::{ListView, ListViewError, ListViewItem, ListViewUpdate};
use crate::components::application::{DensityClass, DensityMetrics};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VirtualListTotal {
    Known(usize),
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VirtualListPolicy {
    estimated_item_extent: f32,
    overscan_extent: f32,
    max_cached_items: usize,
}

impl VirtualListPolicy {
    pub fn new(
        estimated_item_extent: f32,
        overscan_extent: f32,
        max_cached_items: usize,
    ) -> Result<Self, VirtualListPolicyError> {
        if !estimated_item_extent.is_finite() || estimated_item_extent <= 0.0 {
            return Err(VirtualListPolicyError::InvalidEstimatedItemExtent);
        }
        if !overscan_extent.is_finite() || overscan_extent < 0.0 {
            return Err(VirtualListPolicyError::InvalidOverscanExtent);
        }
        Ok(Self {
            estimated_item_extent,
            overscan_extent,
            max_cached_items,
        })
    }

    pub const fn estimated_item_extent(self) -> f32 {
        self.estimated_item_extent
    }

    pub const fn overscan_extent(self) -> f32 {
        self.overscan_extent
    }

    pub const fn max_cached_items(self) -> usize {
        self.max_cached_items
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VirtualListPolicyError {
    InvalidEstimatedItemExtent,
    InvalidOverscanExtent,
}

impl fmt::Display for VirtualListPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid virtual-list policy: {self:?}")
    }
}

impl std::error::Error for VirtualListPolicyError {}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VirtualListViewport {
    offset: f32,
    extent: f32,
}

impl VirtualListViewport {
    pub fn new(offset: f32, extent: f32) -> Result<Self, VirtualListViewportError> {
        if !offset.is_finite() || offset < 0.0 || !extent.is_finite() || extent < 0.0 {
            return Err(VirtualListViewportError::InvalidGeometry);
        }
        Ok(Self { offset, extent })
    }

    pub const fn offset(self) -> f32 {
        self.offset
    }

    pub const fn extent(self) -> f32 {
        self.extent
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VirtualListViewportError {
    InvalidGeometry,
}

impl fmt::Display for VirtualListViewportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("virtual-list viewport geometry must be finite and nonnegative")
    }
}

impl std::error::Error for VirtualListViewportError {}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VirtualListStyle {
    pub container: BoxStyle,
    pub row: BoxStyle,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VirtualListPlan<K> {
    revision: u64,
    viewport: VirtualListViewport,
    total: VirtualListTotal,
    total_extent: f32,
    visible_range: Range<usize>,
    materialized_range: Range<usize>,
    visible_keys: Vec<K>,
    materialized_keys: Vec<K>,
    cached_keys: Vec<K>,
    leading_extent: f32,
    trailing_extent: f32,
}

impl<K> VirtualListPlan<K> {
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    pub const fn viewport(&self) -> VirtualListViewport {
        self.viewport
    }

    pub const fn total(&self) -> VirtualListTotal {
        self.total
    }

    pub const fn total_extent(&self) -> f32 {
        self.total_extent
    }

    pub fn visible_range(&self) -> Range<usize> {
        self.visible_range.clone()
    }

    pub fn materialized_range(&self) -> Range<usize> {
        self.materialized_range.clone()
    }

    pub fn visible_keys(&self) -> &[K] {
        &self.visible_keys
    }

    pub fn materialized_keys(&self) -> &[K] {
        &self.materialized_keys
    }

    /// Overscan rows retained outside the current visible range.
    pub fn cached_keys(&self) -> &[K] {
        &self.cached_keys
    }

    pub const fn leading_extent(&self) -> f32 {
        self.leading_extent
    }

    pub const fn trailing_extent(&self) -> f32 {
        self.trailing_extent
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VirtualListUpdate<K> {
    items: ListViewUpdate<K>,
    total_before: VirtualListTotal,
    total_after: VirtualListTotal,
    changed: bool,
    revision: u64,
}

impl<K> VirtualListUpdate<K> {
    pub const fn items(&self) -> &ListViewUpdate<K> {
        &self.items
    }

    pub const fn total_before(&self) -> VirtualListTotal {
        self.total_before
    }

    pub const fn total_after(&self) -> VirtualListTotal {
        self.total_after
    }

    pub const fn changed(&self) -> bool {
        self.changed
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VirtualListView<K> {
    list: ListView<K>,
    total: VirtualListTotal,
    policy: VirtualListPolicy,
    measured_extents: Vec<(K, f32)>,
    density: DensityMetrics,
    style: VirtualListStyle,
    revision: u64,
}

impl<K> VirtualListView<K>
where
    K: Clone + Eq,
{
    pub fn new(
        label: impl Into<String>,
        items: impl IntoIterator<Item = ListViewItem<K>>,
        total: VirtualListTotal,
        policy: VirtualListPolicy,
    ) -> Result<Self, VirtualListError<K>> {
        let items: Vec<_> = items.into_iter().collect();
        validate_total(total, items.len())?;
        Ok(Self {
            list: ListView::new(label, items).map_err(VirtualListError::Items)?,
            total,
            policy,
            measured_extents: Vec::new(),
            density: DensityMetrics::baseline(DensityClass::Standard),
            style: VirtualListStyle::default(),
            revision: 1,
        })
    }

    pub fn density(mut self, density: DensityMetrics) -> Self {
        self.density = density;
        self
    }

    pub fn style(mut self, style: VirtualListStyle) -> Self {
        self.style = style;
        self
    }

    pub fn label(&self) -> &str {
        self.list.label()
    }

    pub fn items(&self) -> &[ListViewItem<K>] {
        self.list.items()
    }

    pub const fn total(&self) -> VirtualListTotal {
        self.total
    }

    pub const fn policy(&self) -> VirtualListPolicy {
        self.policy
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }

    pub fn set_measured_extent(
        &mut self,
        key: &K,
        extent: f32,
    ) -> Result<bool, VirtualListError<K>> {
        if !extent.is_finite() || extent <= 0.0 {
            return Err(VirtualListError::InvalidMeasuredExtent);
        }
        if !self.list.items().iter().any(|item| item.key() == key) {
            return Err(VirtualListError::UnknownKey(key.clone()));
        }
        let revision = next_revision(self.revision)?;
        if let Some((_, current)) = self
            .measured_extents
            .iter_mut()
            .find(|(measured_key, _)| measured_key == key)
        {
            if *current == extent {
                return Ok(false);
            }
            *current = extent;
        } else {
            self.measured_extents.push((key.clone(), extent));
        }
        self.revision = revision;
        Ok(true)
    }

    pub fn update_items(
        &mut self,
        items: impl IntoIterator<Item = ListViewItem<K>>,
        total: VirtualListTotal,
    ) -> Result<VirtualListUpdate<K>, VirtualListError<K>> {
        let items: Vec<_> = items.into_iter().collect();
        validate_total(total, items.len())?;
        let changed = items.as_slice() != self.list.items() || self.total != total;
        let revision = if changed {
            next_revision(self.revision)?
        } else {
            self.revision
        };
        let item_update = self
            .list
            .update_items(items)
            .map_err(VirtualListError::Items)?;
        let total_before = self.total;
        if changed {
            self.total = total;
            self.measured_extents
                .retain(|(key, _)| self.list.items().iter().any(|item| item.key() == key));
            self.revision = revision;
        }
        Ok(VirtualListUpdate {
            items: item_update,
            total_before,
            total_after: total,
            changed,
            revision: self.revision,
        })
    }

    pub fn plan(&self, viewport: VirtualListViewport) -> VirtualListPlan<K> {
        let visible_collection = self.collection(0.0);
        let materialized_collection = self.collection(self.policy.overscan_extent);
        let visible_range = visible_collection.visible_range(viewport.offset, viewport.extent);
        let candidate = materialized_collection.visible_range(viewport.offset, viewport.extent);
        let before_available = visible_range.start.saturating_sub(candidate.start);
        let after_available = candidate.end.saturating_sub(visible_range.end);
        let mut before = before_available.min(self.policy.max_cached_items / 2);
        let mut after = after_available.min(self.policy.max_cached_items - before);
        let remaining = self.policy.max_cached_items - before - after;
        let extra_before = (before_available - before).min(remaining);
        before += extra_before;
        after += (after_available - after).min(remaining - extra_before);
        let materialized_range = (visible_range.start - before)..(visible_range.end + after);
        let visible_keys = self.keys(visible_range.clone());
        let materialized_keys = self.keys(materialized_range.clone());
        let cached_keys = materialized_range
            .clone()
            .filter(|index| !visible_range.contains(index))
            .filter_map(|index| self.list.items().get(index))
            .map(|item| item.key().clone())
            .collect();
        let total_extent = visible_collection.total_extent();
        let leading_extent = visible_collection
            .item_range(materialized_range.start)
            .map_or(total_extent, |range| range.start);
        let trailing_extent = visible_collection
            .item_range(materialized_range.end.saturating_sub(1))
            .map_or(0.0, |range| (total_extent - range.end).max(0.0));
        VirtualListPlan {
            revision: self.revision,
            viewport,
            total: self.total,
            total_extent,
            visible_range,
            materialized_range,
            visible_keys,
            materialized_keys,
            cached_keys,
            leading_extent,
            trailing_extent,
        }
    }

    /// Returns a reveal request for the current keyed extent without mutating scroll state.
    pub fn reveal_request(
        &self,
        key: &K,
        alignment: RevealAlignment,
    ) -> Result<RevealRequest, VirtualListError<K>> {
        if let RevealAlignment::Fraction(value) = alignment
            && (!value.is_finite() || !(0.0..=1.0).contains(&value))
        {
            return Err(VirtualListError::InvalidRevealAlignment);
        }
        let index = self
            .list
            .items()
            .iter()
            .position(|item| item.key() == key)
            .ok_or_else(|| VirtualListError::UnknownKey(key.clone()))?;
        let range = self
            .collection(0.0)
            .item_range(index)
            .expect("known item index has a virtual extent");
        Ok(RevealRequest {
            target: RectF {
                x: 0.0,
                y: range.start,
                width: 0.0,
                height: range.end - range.start,
            },
            horizontal: None,
            vertical: Some(alignment),
        })
    }

    pub fn mount<'storage, Action, Content>(
        &self,
        ui: &mut Ui<'_, 'storage, Action>,
        host: UiNodeId,
        plan: &VirtualListPlan<K>,
        mut content: Content,
    ) -> RuntimeResult<VirtualListViewRef<K>>
    where
        Action: 'static,
        Content: FnMut(&ListViewItem<K>, &mut MountWriter<'storage, Action>),
    {
        if plan.revision != self.revision
            || plan.total != self.total
            || plan.materialized_range.start > plan.materialized_range.end
            || plan.materialized_range.end > self.list.items().len()
            || plan.materialized_keys != self.keys(plan.materialized_range.clone())
        {
            return Err(RuntimeError::new(
                "virtual-list materialization plan is stale",
            ));
        }
        u32::try_from(self.list.items().len())
            .map_err(|_| RuntimeError::new("virtual list exceeds semantic item capacity"))?;
        let known_count =
            match self.total {
                VirtualListTotal::Known(count) => Some(u32::try_from(count).map_err(|_| {
                    RuntimeError::new("virtual list exceeds semantic item capacity")
                })?),
                VirtualListTotal::Unknown => None,
            };
        let minimum = self.density.effective_minimum();
        let collection = self.collection(0.0);
        let mut mounted = Vec::with_capacity(plan.materialized_range.len());
        let root = ui
            .foundation()
            .container_node_under(
                host,
                self.style.container,
                LayoutStyle {
                    flow: Flow::Vertical,
                    ..LayoutStyle::default()
                },
                |writer| {
                    if plan.leading_extent > 0.0 {
                        writer.layer(
                            true,
                            BoxStyle {
                                height: SizeRule::Logical(plan.leading_extent),
                                ..BoxStyle::default()
                            },
                            LayoutStyle::default(),
                            |_| {},
                        );
                    }
                    for index in plan.materialized_range.clone() {
                        let item = &self.list.items()[index];
                        let extent = collection
                            .item_range(index)
                            .map_or(minimum.height(), |range| range.end - range.start);
                        let mut style = self.style.row;
                        style.height = SizeRule::Logical(extent);
                        style.min_size = SizeRule2D {
                            width: SizeRule::Logical(minimum.width()),
                            height: SizeRule::Logical(minimum.height()),
                        };
                        let control = writer.layer(true, style, LayoutStyle::default(), |writer| {
                            content(item, writer)
                        });
                        mounted.push((index, item.clone(), control));
                    }
                    if plan.trailing_extent > 0.0 {
                        writer.layer(
                            true,
                            BoxStyle {
                                height: SizeRule::Logical(plan.trailing_extent),
                                ..BoxStyle::default()
                            },
                            LayoutStyle::default(),
                            |_| {},
                        );
                    }
                },
            )
            .ok_or_else(|| RuntimeError::new("application virtual-list host is stale"))?;

        let mut rows = Vec::with_capacity(mounted.len());
        for (index, item, control) in mounted {
            let semantic_index = u32::try_from(index)
                .map_err(|_| RuntimeError::new("virtual-list item index exceeds semantics"))?;
            let position = semantic_index
                .checked_add(1)
                .ok_or_else(|| RuntimeError::new("virtual-list item position exceeds semantics"))?;
            let name = ui.foundation().intern(item.label());
            ui.foundation()
                .semantic_node(
                    control.node,
                    SemanticNode {
                        role: SemanticRole::ListItem,
                        name: SemanticName::Text(name),
                        collection: Some(SemanticCollection {
                            item_index: Some(semantic_index),
                            item_count: known_count,
                            position_in_set: Some(position),
                            set_size: known_count,
                            ..SemanticCollection::default()
                        }),
                        ..SemanticNode::default()
                    },
                )
                .map_err(|error| {
                    RuntimeError::new(format!("invalid virtual-list row semantics: {error:?}"))
                })?;
            rows.push(VirtualListRowRef {
                key: item.key().clone(),
                control,
                index,
            });
        }

        let name = ui.foundation().intern(self.list.label());
        let relationships = rows
            .iter()
            .map(|row| SemanticRelationship {
                kind: SemanticRelationshipKind::Owns,
                target: row.control.node,
            })
            .collect();
        ui.foundation()
            .semantic_node(
                root.node,
                SemanticNode {
                    role: SemanticRole::List,
                    name: SemanticName::Text(name),
                    relationships,
                    collection: Some(SemanticCollection {
                        item_count: known_count,
                        set_size: known_count.filter(|count| *count > 0),
                        ..SemanticCollection::default()
                    }),
                    ..SemanticNode::default()
                },
            )
            .map_err(|error| {
                RuntimeError::new(format!("invalid virtual-list semantics: {error:?}"))
            })?;
        Ok(VirtualListViewRef { root, rows })
    }

    fn collection(&self, overscan: f32) -> VirtualCollection {
        let minimum = self.density.effective_minimum().height();
        let mut collection = VirtualCollection::new(
            self.list.items().len(),
            self.policy.estimated_item_extent.max(minimum),
            overscan,
        );
        for (index, item) in self.list.items().iter().enumerate() {
            if let Some((_, extent)) = self
                .measured_extents
                .iter()
                .find(|(key, _)| key == item.key())
            {
                collection.set_extent(index, extent.max(minimum));
            }
        }
        collection
    }

    fn keys(&self, range: Range<usize>) -> Vec<K> {
        self.list.items()[range]
            .iter()
            .map(|item| item.key().clone())
            .collect()
    }
}

#[derive(Clone, Debug)]
pub struct VirtualListViewRef<K> {
    root: ControlHandle,
    rows: Vec<VirtualListRowRef<K>>,
}

impl<K> VirtualListViewRef<K> {
    pub const fn node(&self) -> UiNodeId {
        self.root.node
    }

    pub fn rows(&self) -> &[VirtualListRowRef<K>] {
        &self.rows
    }

    pub const fn style(&self) -> Property<BoxStyle> {
        self.root.style
    }
}

#[derive(Clone, Debug)]
pub struct VirtualListRowRef<K> {
    key: K,
    control: ControlHandle,
    index: usize,
}

impl<K> VirtualListRowRef<K> {
    pub const fn key(&self) -> &K {
        &self.key
    }

    pub const fn node(&self) -> UiNodeId {
        self.control.node
    }

    pub const fn index(&self) -> usize {
        self.index
    }

    pub const fn style(&self) -> Property<BoxStyle> {
        self.control.style
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum VirtualListError<K> {
    Items(ListViewError<K>),
    KnownTotalMismatch { loaded: usize, total: usize },
    InvalidMeasuredExtent,
    InvalidRevealAlignment,
    UnknownKey(K),
    RevisionExhausted,
}

impl<K: fmt::Debug> fmt::Display for VirtualListError<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "virtual-list operation failed: {self:?}")
    }
}

impl<K: fmt::Debug> std::error::Error for VirtualListError<K> {}

fn validate_total<K>(total: VirtualListTotal, loaded: usize) -> Result<(), VirtualListError<K>> {
    if let VirtualListTotal::Known(total) = total
        && total != loaded
    {
        return Err(VirtualListError::KnownTotalMismatch { loaded, total });
    }
    Ok(())
}

fn next_revision<K>(revision: u64) -> Result<u64, VirtualListError<K>> {
    revision
        .checked_add(1)
        .ok_or(VirtualListError::RevisionExhausted)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::runtime::{Component, CreateContext, UpdateContext, ViewRuntime};
    use crate::ui::UiRoot;

    use super::*;

    fn item(key: u8) -> ListViewItem<u8> {
        ListViewItem::new(key, format!("Item {key}")).unwrap()
    }

    fn policy() -> VirtualListPolicy {
        VirtualListPolicy::new(20.0, 100.0, 4).unwrap()
    }

    #[test]
    fn policy_viewport_and_known_total_validate_at_the_boundary() {
        assert_eq!(
            VirtualListPolicy::new(0.0, 1.0, 0),
            Err(VirtualListPolicyError::InvalidEstimatedItemExtent)
        );
        assert_eq!(
            VirtualListPolicy::new(1.0, -1.0, 0),
            Err(VirtualListPolicyError::InvalidOverscanExtent)
        );
        assert_eq!(
            VirtualListViewport::new(f32::NAN, 10.0),
            Err(VirtualListViewportError::InvalidGeometry)
        );
        assert_eq!(
            VirtualListView::new("Items", [item(1)], VirtualListTotal::Known(2), policy()),
            Err(VirtualListError::KnownTotalMismatch {
                loaded: 1,
                total: 2
            })
        );
    }

    #[test]
    fn materialization_is_keyed_bounded_and_uses_preserved_measurements() {
        let mut list = VirtualListView::new(
            "Items",
            (0..20).map(item),
            VirtualListTotal::Known(20),
            policy(),
        )
        .unwrap();
        list.set_measured_extent(&5, 80.0).unwrap();
        let key_five = list.reveal_request(&5, RevealAlignment::Start).unwrap();
        let plan = list.plan(VirtualListViewport::new(key_five.target.y, 60.0).unwrap());
        assert!(plan.cached_keys().len() <= policy().max_cached_items());
        assert!(plan.materialized_keys().len() <= plan.visible_keys().len() + 4);
        assert!(plan.visible_keys().contains(&5));
        assert_eq!(plan.total_extent(), 688.0);

        list.update_items((0..20).rev().map(item), VirtualListTotal::Known(20))
            .unwrap();
        let request = list.reveal_request(&5, RevealAlignment::Center).unwrap();
        assert_eq!(request.target.height, 80.0);
        assert_eq!(request.vertical, Some(RevealAlignment::Center));
        assert_eq!(request.horizontal, None);
    }

    #[test]
    fn unknown_totals_stay_unknown_and_invalid_updates_are_atomic() {
        let mut list = VirtualListView::new(
            "Streaming results",
            (0..8).map(item),
            VirtualListTotal::Unknown,
            policy(),
        )
        .unwrap();
        let revision = list.revision();
        assert_eq!(
            list.update_items((0..8).map(item), VirtualListTotal::Known(9)),
            Err(VirtualListError::KnownTotalMismatch {
                loaded: 8,
                total: 9
            })
        );
        assert_eq!(list.revision(), revision);
        assert_eq!(list.total(), VirtualListTotal::Unknown);
        let plan = list.plan(VirtualListViewport::new(0.0, 40.0).unwrap());
        assert_eq!(plan.total(), VirtualListTotal::Unknown);
    }

    struct MountedVirtualList {
        mounted: Rc<RefCell<Option<VirtualListViewRef<u8>>>>,
    }

    impl Component for MountedVirtualList {
        type State = VirtualListView<u8>;
        type Action = ();

        fn create(&self, _context: &mut CreateContext<'_>) -> Self::State {
            VirtualListView::new(
                "Large list",
                (0..100).map(item),
                VirtualListTotal::Known(100),
                VirtualListPolicy::new(44.0, 44.0, 2).unwrap(),
            )
            .unwrap()
            .density(DensityMetrics::baseline(DensityClass::Touch))
        }

        fn mount(&self, state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            let plan = state.plan(VirtualListViewport::new(440.0, 88.0).unwrap());
            self.mounted
                .replace(Some(state.mount(ui, root.0, &plan, |_, _| {}).unwrap()));
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

    #[test]
    fn mount_emits_only_planned_rows_with_global_known_collection_metadata() {
        let mounted = Rc::new(RefCell::new(None));
        let runtime = ViewRuntime::from_component(MountedVirtualList {
            mounted: mounted.clone(),
        })
        .unwrap();
        let mounted = mounted.borrow();
        let mounted = mounted.as_ref().unwrap();
        let root = runtime.ui().semantics.get(mounted.node()).unwrap();
        assert_eq!(root.role, SemanticRole::List);
        assert_eq!(root.collection.unwrap().item_count, Some(100));
        assert!(mounted.rows().len() < 100);
        assert_eq!(root.relationships.len(), mounted.rows().len());
        for row in mounted.rows() {
            let semantic = runtime.ui().semantics.get(row.node()).unwrap();
            assert_eq!(semantic.role, SemanticRole::ListItem);
            assert_eq!(
                semantic.collection.unwrap().item_index,
                Some(row.index() as u32)
            );
            assert_eq!(semantic.collection.unwrap().set_size, Some(100));
            assert!(
                !runtime
                    .ui()
                    .interactions
                    .get(row.node())
                    .is_some_and(|interaction| interaction.focusable)
            );
        }
    }
}

//! Declarative mounting API and compact mounted UI components.
//!
//! Declarations execute once during mount. Runtime changes flow through typed properties and
//! coalesced transactions; no recursive widget tree exists in the active UI.

mod interaction;
mod styling;
mod transactions;

use std::{marker::PhantomData, sync::Arc};

use crate::foundation::{ColorRgba8, PointF};
use crate::graphics::scene::{DirtyFlags, NodeArena, NodeId, SparseSet};
pub use crate::input::EventPhase;
use crate::input::InputEvent;

use crate::ui::semantics::{
    SemanticCheckState, SemanticError, SemanticName, SemanticNode, SemanticRole, SemanticValue,
};

pub use crate::graphics::scene::NodeId as UiNodeId;

mod model;
pub use model::*;

#[derive(Clone, Debug)]
pub struct MountedUi {
    pub nodes: NodeArena,
    pub kinds: SparseSet<NodeKind>,
    pub box_styles: SparseSet<BoxStyle>,
    pub layouts: SparseSet<LayoutStyle>,
    pub interactions: SparseSet<InteractionSnapshot>,
    pub texts: SparseSet<TextVisual>,
    pub images: SparseSet<ImageVisual>,
    pub semantics: SparseSet<SemanticNode>,
    pub window_chrome_roles: SparseSet<crate::shell::window_chrome::WindowChromeRole>,
    pub window_chrome_hit_specs: SparseSet<crate::shell::window_chrome::WindowChromeHitSpec>,
    pub pointer_requests: SparseSet<crate::assets::PointerRequest>,
    style_bindings: Vec<StyleBinding>,
    style_binding_head_by_state: SparseSet<usize>,
    style_binding_next: Vec<Option<usize>>,
    dirty_style_bindings: Vec<usize>,
    dirty_style_binding_marks: Vec<bool>,
    keys: SparseSet<Option<u64>>,
    strings: Vec<String>,
    dynamic_text_strings: SparseSet<StringId>,
    free_dynamic_strings: Vec<u32>,
    root: Option<UiRoot>,
    patch_log: Vec<Patch>,
    structural_log: Vec<StructuralCommand>,
    pub diagnostics: UiDiagnostics,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct UiMemoryReport {
    pub mounted_nodes: usize,
    pub scene_bytes: usize,
    pub component_bytes: usize,
    pub string_bytes: usize,
    pub scratch_bytes: usize,
}
impl UiMemoryReport {
    pub fn total_bytes(self) -> usize {
        self.scene_bytes + self.component_bytes + self.string_bytes + self.scratch_bytes
    }
}

impl Default for MountedUi {
    fn default() -> Self {
        Self {
            nodes: NodeArena::default(),
            kinds: SparseSet::default(),
            box_styles: SparseSet::default(),
            layouts: SparseSet::default(),
            interactions: SparseSet::default(),
            texts: SparseSet::default(),
            images: SparseSet::default(),
            semantics: SparseSet::default(),
            window_chrome_roles: SparseSet::default(),
            window_chrome_hit_specs: SparseSet::default(),
            pointer_requests: SparseSet::default(),
            style_bindings: Vec::new(),
            style_binding_head_by_state: SparseSet::default(),
            style_binding_next: Vec::new(),
            dirty_style_bindings: Vec::new(),
            dirty_style_binding_marks: Vec::new(),
            keys: SparseSet::default(),
            strings: vec![String::new(), "sans-serif".to_owned()],
            dynamic_text_strings: SparseSet::default(),
            free_dynamic_strings: Vec::new(),
            root: None,
            patch_log: Vec::with_capacity(32),
            structural_log: Vec::with_capacity(4),
            diagnostics: UiDiagnostics::default(),
        }
    }
}

impl MountedUi {
    pub fn root(&self) -> Option<UiRoot> {
        self.root
    }
    pub fn memory_report(&self) -> UiMemoryReport {
        UiMemoryReport {
            mounted_nodes: self.nodes.alive().len(),
            scene_bytes: self.nodes.allocated_bytes(),
            component_bytes: self.kinds.allocated_bytes()
                + self.box_styles.allocated_bytes()
                + self.layouts.allocated_bytes()
                + self.interactions.allocated_bytes()
                + self.texts.allocated_bytes()
                + self.images.allocated_bytes()
                + self.semantics.allocated_bytes()
                + self.window_chrome_roles.allocated_bytes()
                + self.window_chrome_hit_specs.allocated_bytes()
                + self.pointer_requests.allocated_bytes()
                + self
                    .semantics
                    .values()
                    .iter()
                    .map(SemanticNode::allocated_bytes)
                    .sum::<usize>()
                + self.keys.allocated_bytes()
                + self.style_binding_head_by_state.allocated_bytes()
                + self.style_bindings.capacity() * std::mem::size_of::<StyleBinding>()
                + self.style_binding_next.capacity() * std::mem::size_of::<Option<usize>>()
                + self.dirty_style_bindings.capacity() * std::mem::size_of::<usize>()
                + self.dirty_style_binding_marks.capacity() * std::mem::size_of::<bool>()
                + self.dynamic_text_strings.allocated_bytes(),
            string_bytes: self.strings.capacity() * std::mem::size_of::<String>()
                + self.strings.iter().map(String::capacity).sum::<usize>()
                + self.free_dynamic_strings.capacity() * std::mem::size_of::<u32>(),
            scratch_bytes: self.patch_log.capacity() * std::mem::size_of::<Patch>()
                + self.structural_log.capacity() * std::mem::size_of::<StructuralCommand>(),
        }
    }
    pub fn string(&self, id: StringId) -> Option<&str> {
        self.strings.get(id.0 as usize).map(String::as_str)
    }
    pub fn intern(&mut self, text: impl AsRef<str>) -> StringId {
        let text = text.as_ref();
        if let Some(index) = self.strings.iter().position(|candidate| candidate == text) {
            return StringId(index as u32);
        }
        let id = StringId(self.strings.len() as u32);
        self.strings.push(text.to_owned());
        id
    }
    pub fn transaction<R>(
        &mut self,
        update: impl FnOnce(&mut UiTransaction<'_>) -> R,
    ) -> (R, TransactionResult) {
        self.patch_log.clear();
        self.structural_log.clear();
        let result = {
            let mut tx = UiTransaction { ui: self };
            update(&mut tx)
        };
        let committed = self.commit();
        (result, committed)
    }
    pub fn remove(&mut self, node: NodeId) -> Vec<NodeId> {
        let removed = self.nodes.remove_subtree(node);
        if self.root.is_some_and(|root| removed.contains(&root.0)) {
            self.root = None;
        }
        for id in &removed {
            if let Some(string) = self.dynamic_text_strings.remove(*id) {
                if let Some(slot) = self.strings.get_mut(string.0 as usize) {
                    slot.clear();
                    self.free_dynamic_strings.push(string.0);
                }
            }
            self.kinds.remove(*id);
            self.box_styles.remove(*id);
            self.layouts.remove(*id);
            self.interactions.remove(*id);
            self.texts.remove(*id);
            self.images.remove(*id);
            self.semantics.remove(*id);
            self.window_chrome_roles.remove(*id);
            self.window_chrome_hit_specs.remove(*id);
            self.pointer_requests.remove(*id);
            self.keys.remove(*id);
        }
        self.style_bindings.retain(|binding| {
            !removed.contains(&binding.state_root)
                && !binding
                    .slots
                    .iter()
                    .any(|slot| removed.contains(&slot.node))
        });
        self.rebuild_style_binding_index();
        removed
    }

    fn allocate_dynamic_text(&mut self, node: NodeId, value: String) -> StringId {
        let id = if let Some(index) = self.free_dynamic_strings.pop() {
            self.strings[index as usize] = value;
            StringId(index)
        } else {
            let id = StringId(self.strings.len() as u32);
            self.strings.push(value);
            id
        };
        self.dynamic_text_strings.insert(node, id);
        id
    }

    /// Replaces one composition-owned text buffer without growing the global string table.
    pub fn set_dynamic_text(&mut self, node: NodeId, value: impl AsRef<str>) -> bool {
        let value = value.as_ref();
        let Some(previous) = self.texts.get(node).copied() else {
            return false;
        };
        let id = match self.dynamic_text_strings.get(node).copied() {
            Some(id) => id,
            None => self.allocate_dynamic_text(node, value.to_owned()),
        };
        let content_changed = self.strings.get_mut(id.0 as usize).is_some_and(|current| {
            if current == value {
                false
            } else {
                current.clear();
                current.push_str(value);
                true
            }
        });
        let id_changed = previous.content != id;
        if id_changed {
            if let Some(text) = self.texts.get_mut(node) {
                text.content = id;
            }
        }
        if !content_changed && !id_changed {
            return false;
        }
        if let Some(text) = self.texts.get_mut(node) {
            text.revision = text.revision.wrapping_add(1).max(1);
        }
        if let Some(semantic) = self.semantics.get_mut(node) {
            if matches!(semantic.name, SemanticName::Text(_)) {
                semantic.name = SemanticName::Text(id);
            }
            if matches!(semantic.value, SemanticValue::Text(_)) {
                semantic.value = SemanticValue::Text(id);
            }
        }
        if let Some(core) = self.nodes.core_mut(node) {
            core.content_revision = core.content_revision.wrapping_add(1).max(1);
            core.semantic_revision = core.semantic_revision.wrapping_add(1).max(1);
        }
        self.nodes.mark_dirty(
            node,
            DirtyFlags::TEXT | DirtyFlags::MEASURE | DirtyFlags::PAINT | DirtyFlags::SEMANTICS,
        );
        true
    }

    /// Rebinds an image node to a retained image resource revision.
    pub fn set_image_visual(&mut self, node: NodeId, image: ImageId, content_version: u64) -> bool {
        let tint = self.images.get(node).and_then(|visual| visual.tint);
        self.set_image_visual_tinted(node, image, content_version, tint)
    }

    /// Rebinds an image and its optional alpha-mask tint as one paint update.
    pub fn set_image_visual_tinted(
        &mut self,
        node: NodeId,
        image: ImageId,
        content_version: u64,
        tint: Option<ColorRgba8>,
    ) -> bool {
        let Some(visual) = self.images.get_mut(node) else {
            return false;
        };
        let content_version = content_version.max(1);
        if visual.image == image && visual.content_version == content_version && visual.tint == tint
        {
            return false;
        }
        visual.image = image;
        visual.content_version = content_version;
        visual.tint = tint;
        if let Some(core) = self.nodes.core_mut(node) {
            core.content_revision = core.content_revision.wrapping_add(1).max(1);
        }
        self.nodes.mark_dirty(node, DirtyFlags::PAINT);
        true
    }
}

fn change_interaction<T: PartialEq + Copy>(
    store: &mut SparseSet<InteractionSnapshot>,
    node: NodeId,
    field: impl FnOnce(&mut InteractionSnapshot) -> &mut T,
    value: T,
) -> bool {
    store.get_mut(node).is_some_and(|item| {
        let slot = field(item);
        if *slot == value {
            false
        } else {
            *slot = value;
            item.revision = item.revision.wrapping_add(1).max(1);
            true
        }
    })
}

#[cfg(test)]
fn replace_value<T: PartialEq>(store: &mut SparseSet<T>, node: NodeId, value: T) -> bool {
    if store.get(node).is_some_and(|current| current == &value) {
        false
    } else {
        store.insert(node, value);
        true
    }
}

#[cfg(test)]
fn replace_default<T: Default + PartialEq>(
    store: &mut SparseSet<T>,
    node: NodeId,
    value: T,
) -> bool {
    if value == T::default() {
        store.remove(node).is_some_and(|previous| previous != value)
    } else {
        replace_value(store, node, value)
    }
}

#[cfg(test)]
fn replace_optional<T: PartialEq>(
    store: &mut SparseSet<T>,
    node: NodeId,
    value: Option<T>,
) -> bool {
    match value {
        Some(value) => replace_value(store, node, value),
        None => store.remove(node).is_some(),
    }
}
fn change_style<T: PartialEq>(
    store: &mut SparseSet<BoxStyle>,
    node: NodeId,
    field: impl FnOnce(&mut BoxStyle) -> &mut T,
    value: T,
) -> bool {
    store.get_mut(node).is_some_and(|item| {
        let slot = field(item);
        if *slot == value {
            false
        } else {
            *slot = value;
            true
        }
    })
}
fn change_layout<T: PartialEq>(
    store: &mut SparseSet<LayoutStyle>,
    node: NodeId,
    field: impl FnOnce(&mut LayoutStyle) -> &mut T,
    value: T,
) -> bool {
    store.get_mut(node).is_some_and(|item| {
        let slot = field(item);
        if *slot == value {
            false
        } else {
            *slot = value;
            true
        }
    })
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct UiRoot(pub NodeId);
mod writer;
pub use writer::MountWriter;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
enum PropertyKind {
    Enabled,
    Visible,
    Value,
    Opacity,
    Text,
    TextColor,
    Background,
    Translation,
    ScrollOffset,
    Style,
    Checked,
    Busy,
}
#[derive(Clone, Debug, PartialEq)]
pub enum PropertyValue {
    Bool(bool),
    Float(f32),
    String(StringId),
    Color(ColorRgba8),
    Point(PointF),
    Style(BoxStyle),
    Check(SemanticCheckState),
}
impl From<bool> for PropertyValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}
impl From<f32> for PropertyValue {
    fn from(value: f32) -> Self {
        Self::Float(value)
    }
}
impl From<StringId> for PropertyValue {
    fn from(value: StringId) -> Self {
        Self::String(value)
    }
}
impl From<ColorRgba8> for PropertyValue {
    fn from(value: ColorRgba8) -> Self {
        Self::Color(value)
    }
}
impl From<PointF> for PropertyValue {
    fn from(value: PointF) -> Self {
        Self::Point(value)
    }
}
impl From<BoxStyle> for PropertyValue {
    fn from(value: BoxStyle) -> Self {
        Self::Style(value)
    }
}
impl From<SemanticCheckState> for PropertyValue {
    fn from(value: SemanticCheckState) -> Self {
        Self::Check(value)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Property<T> {
    node: NodeId,
    kind: PropertyKind,
    marker: PhantomData<fn(T)>,
}
impl<T> Copy for Property<T> {}
impl<T> Clone for Property<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Property<T> {
    fn new(node: NodeId, kind: PropertyKind) -> Self {
        Self {
            node,
            kind,
            marker: PhantomData,
        }
    }
    pub fn node(self) -> NodeId {
        self.node
    }
}
#[derive(Copy, Clone, Debug)]
pub struct TextHandle {
    pub node: NodeId,
    pub text: Property<StringId>,
    pub color: Property<ColorRgba8>,
    pub enabled: Property<bool>,
    pub style: Property<BoxStyle>,
}
#[derive(Copy, Clone, Debug)]
pub struct ControlHandle {
    pub node: NodeId,
    pub enabled: Property<bool>,
    pub visible: Property<bool>,
    pub value: Property<f32>,
    pub opacity: Property<f32>,
    pub background: Property<ColorRgba8>,
    pub translation: Property<PointF>,
    pub style: Property<BoxStyle>,
    pub checked: Property<SemanticCheckState>,
    pub busy: Property<bool>,
}
impl ControlHandle {
    fn new(node: NodeId) -> Self {
        Self {
            node,
            enabled: Property::new(node, PropertyKind::Enabled),
            visible: Property::new(node, PropertyKind::Visible),
            value: Property::new(node, PropertyKind::Value),
            opacity: Property::new(node, PropertyKind::Opacity),
            background: Property::new(node, PropertyKind::Background),
            translation: Property::new(node, PropertyKind::Translation),
            style: Property::new(node, PropertyKind::Style),
            checked: Property::new(node, PropertyKind::Checked),
            busy: Property::new(node, PropertyKind::Busy),
        }
    }
}
#[derive(Copy, Clone, Debug)]
pub struct ScrollHandle {
    pub node: NodeId,
    pub offset: Property<PointF>,
    pub style: Property<BoxStyle>,
}

#[derive(Clone, Debug)]
struct Patch {
    node: NodeId,
    kind: PropertyKind,
    value: PropertyValue,
}
pub struct UiTransaction<'a> {
    ui: &'a mut MountedUi,
}
impl UiTransaction<'_> {
    pub fn set<T>(&mut self, property: Property<T>, value: T)
    where
        T: Into<PropertyValue>,
    {
        let value = value.into();
        if let Some(patch) = self
            .ui
            .patch_log
            .iter_mut()
            .find(|patch| patch.node == property.node && patch.kind == property.kind)
        {
            patch.value = value;
        } else {
            self.ui.patch_log.push(Patch {
                node: property.node,
                kind: property.kind,
                value,
            });
        }
    }
    pub fn remove(&mut self, node: NodeId) {
        self.ui.structural_log.push(StructuralCommand::Remove(node));
    }
    #[cfg(test)]
    fn reconcile_keyed(&mut self, parent: NodeId, children: Vec<NodeFixture>) {
        self.ui
            .structural_log
            .push(StructuralCommand::Reconcile { parent, children });
    }
}
#[derive(Clone, Debug)]
enum StructuralCommand {
    Remove(NodeId),
    #[cfg(test)]
    Reconcile {
        parent: NodeId,
        children: Vec<NodeFixture>,
    },
}
#[cfg(test)]
#[derive(Clone, Debug)]
struct NodeFixture {
    key: Option<u64>,
    kind: NodeKind,
    style: BoxStyle,
    layout: LayoutStyle,
    interaction: InteractionSnapshot,
    text: Option<TextVisual>,
    image: Option<ImageVisual>,
    semantic: Option<SemanticNode>,
    children: Vec<Self>,
}
#[cfg(test)]
impl NodeFixture {
    fn container(key: Option<u64>, style: BoxStyle, children: Vec<Self>) -> Self {
        Self {
            key,
            kind: NodeKind::Box,
            style,
            layout: LayoutStyle::default(),
            interaction: InteractionSnapshot::default(),
            text: None,
            image: None,
            semantic: None,
            children,
        }
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct TransactionResult {
    pub property_patches: usize,
    pub structural_mutations: usize,
}
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct UiDiagnostics {
    pub property_patches: u64,
    pub structural_mutations: u64,
    pub events_dispatched: u64,
    pub semantic_updates: u64,
    pub semantic_failures: u64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct UiEvent {
    /// The node selected by hit testing or focus dispatch.
    pub target: NodeId,
    /// The node whose listener is currently being invoked.
    pub current_target: NodeId,
    pub kind: UiEventKind,
    pub phase: EventPhase,
    pub timestamp: u64,
}
#[derive(Clone, Debug, PartialEq)]
pub enum UiEventKind {
    Input(InputEvent),
    Focus(bool),
    Text(StringId),
}

#[cfg(test)]
mod tests;

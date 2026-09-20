//! Nonadaptive named application scaffold.
//!
//! [`Scaffold`] owns only canonical slot identity, order, layout, and landmark relationships. Slot
//! content remains caller-mounted, and no routing, overlay, focus, or platform policy is executed.

use std::fmt;

use crate::runtime::{RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    BoxStyle, ControlHandle, LayoutStyle, MountWriter, Property, SemanticName, SemanticNode,
    SemanticRelationship, SemanticRelationshipKind, SemanticRole, UiNodeId,
};

/// Stable application regions in canonical semantic and visual order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ScaffoldSlot {
    Navigation,
    Top,
    Content,
    Secondary,
    Status,
    FloatingAction,
    Overlay,
}

impl ScaffoldSlot {
    pub const ALL: [Self; 7] = [
        Self::Navigation,
        Self::Top,
        Self::Content,
        Self::Secondary,
        Self::Status,
        Self::FloatingAction,
        Self::Overlay,
    ];

    pub const fn semantic_role(self) -> SemanticRole {
        match self {
            Self::Navigation => SemanticRole::Navigation,
            Self::Top => SemanticRole::Banner,
            Self::Content => SemanticRole::Main,
            Self::Secondary => SemanticRole::Complementary,
            Self::Status => SemanticRole::Status,
            Self::FloatingAction | Self::Overlay => SemanticRole::Region,
        }
    }
}

/// One present scaffold slot and its explicit landmark name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScaffoldSlotSpec {
    slot: ScaffoldSlot,
    label: String,
}

impl ScaffoldSlotSpec {
    pub fn new(
        slot: ScaffoldSlot,
        label: impl Into<String>,
    ) -> Result<Self, ScaffoldSlotSpecError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(ScaffoldSlotSpecError::MissingAccessibleName);
        }
        Ok(Self { slot, label })
    }

    pub const fn slot(&self) -> ScaffoldSlot {
        self.slot
    }

    pub fn label(&self) -> &str {
        &self.label
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScaffoldSlotSpecError {
    MissingAccessibleName,
}

impl fmt::Display for ScaffoldSlotSpecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("scaffold slot accessible name is empty")
    }
}

impl std::error::Error for ScaffoldSlotSpecError {}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScaffoldStyle {
    pub container: BoxStyle,
    pub slot: BoxStyle,
    pub layout: LayoutStyle,
    pub slot_layout: LayoutStyle,
}

/// Fixed-layout application structure over stable named slots.
#[derive(Clone, Debug, PartialEq)]
pub struct Scaffold {
    label: String,
    slots: Vec<ScaffoldSlotSpec>,
    style: ScaffoldStyle,
}

impl Scaffold {
    pub fn new(
        label: impl Into<String>,
        slots: impl IntoIterator<Item = ScaffoldSlotSpec>,
    ) -> Result<Self, ScaffoldError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(ScaffoldError::MissingAccessibleName);
        }
        let mut slots: Vec<_> = slots.into_iter().collect();
        slots.sort_by_key(ScaffoldSlotSpec::slot);
        for pair in slots.windows(2) {
            if pair[0].slot == pair[1].slot {
                return Err(ScaffoldError::DuplicateSlot(pair[0].slot));
            }
        }
        if !slots.iter().any(|spec| spec.slot == ScaffoldSlot::Content) {
            return Err(ScaffoldError::MissingContent);
        }
        Ok(Self {
            label,
            slots,
            style: ScaffoldStyle::default(),
        })
    }

    pub const fn style(mut self, style: ScaffoldStyle) -> Self {
        self.style = style;
        self
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn slots(&self) -> &[ScaffoldSlotSpec] {
        &self.slots
    }

    pub fn slot(&self, slot: ScaffoldSlot) -> Option<&ScaffoldSlotSpec> {
        self.slots.iter().find(|spec| spec.slot == slot)
    }

    pub fn mount<'storage, Action, Content>(
        &self,
        ui: &mut Ui<'_, 'storage, Action>,
        host: UiNodeId,
        mut content: Content,
    ) -> RuntimeResult<ScaffoldRef>
    where
        Action: 'static,
        Content: FnMut(ScaffoldSlot, &mut MountWriter<'storage, Action>),
    {
        let mut mounted = Vec::with_capacity(self.slots.len());
        let root = ui
            .foundation()
            .container_node_under(host, self.style.container, self.style.layout, |writer| {
                for spec in &self.slots {
                    let slot = spec.slot;
                    let control =
                        writer.container(self.style.slot, self.style.slot_layout, |writer| {
                            content(slot, writer)
                        });
                    mounted.push((spec.clone(), control));
                }
            })
            .ok_or_else(|| RuntimeError::new("application scaffold parent is stale"))?;

        let mut slot_refs = Vec::with_capacity(mounted.len());
        for (spec, control) in mounted {
            let name = ui.foundation().intern(&spec.label);
            ui.foundation()
                .semantic_node(
                    control,
                    SemanticNode::named(spec.slot.semantic_role(), name),
                )
                .map_err(|error| {
                    RuntimeError::new(format!("invalid scaffold slot semantics: {error:?}"))
                })?;
            slot_refs.push(ScaffoldSlotRef {
                slot: spec.slot,
                control,
            });
        }

        let name = ui.foundation().intern(&self.label);
        let relationships = slot_refs
            .iter()
            .map(|slot| SemanticRelationship {
                kind: SemanticRelationshipKind::Owns,
                target: slot.control,
            })
            .collect();
        ui.foundation()
            .semantic_node(
                root.node,
                SemanticNode {
                    role: SemanticRole::Application,
                    name: SemanticName::Text(name),
                    relationships,
                    ..SemanticNode::default()
                },
            )
            .map_err(|error| RuntimeError::new(format!("invalid scaffold semantics: {error:?}")))?;

        Ok(ScaffoldRef {
            root,
            slots: slot_refs,
        })
    }
}

#[derive(Clone, Debug)]
pub struct ScaffoldRef {
    root: ControlHandle,
    slots: Vec<ScaffoldSlotRef>,
}

impl ScaffoldRef {
    pub const fn node(&self) -> UiNodeId {
        self.root.node
    }

    pub const fn style(&self) -> Property<BoxStyle> {
        self.root.style
    }

    pub fn slots(&self) -> &[ScaffoldSlotRef] {
        &self.slots
    }

    pub fn slot(&self, slot: ScaffoldSlot) -> Option<&ScaffoldSlotRef> {
        self.slots.iter().find(|reference| reference.slot == slot)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScaffoldSlotRef {
    slot: ScaffoldSlot,
    control: UiNodeId,
}

impl ScaffoldSlotRef {
    pub const fn slot(&self) -> ScaffoldSlot {
        self.slot
    }

    pub const fn node(&self) -> UiNodeId {
        self.control
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScaffoldError {
    MissingAccessibleName,
    MissingContent,
    DuplicateSlot(ScaffoldSlot),
}

impl fmt::Display for ScaffoldError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid application scaffold: {self:?}")
    }
}

impl std::error::Error for ScaffoldError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{Component, CreateContext, UpdateContext, ViewRuntime};
    use crate::ui::{NodeKind, SemanticRelationshipKind, UiRoot};
    use std::cell::RefCell;
    use std::rc::Rc;

    fn slot(slot: ScaffoldSlot, label: &str) -> ScaffoldSlotSpec {
        ScaffoldSlotSpec::new(slot, label).unwrap()
    }

    #[test]
    fn construction_canonicalizes_slots_and_requires_exact_unique_content() {
        let scaffold = Scaffold::new(
            "Workspace",
            [
                slot(ScaffoldSlot::Status, "Sync status"),
                slot(ScaffoldSlot::Content, "Document"),
                slot(ScaffoldSlot::Navigation, "Sections"),
            ],
        )
        .unwrap();
        assert_eq!(
            scaffold
                .slots()
                .iter()
                .map(ScaffoldSlotSpec::slot)
                .collect::<Vec<_>>(),
            [
                ScaffoldSlot::Navigation,
                ScaffoldSlot::Content,
                ScaffoldSlot::Status
            ]
        );
        assert_eq!(
            Scaffold::new("Workspace", [slot(ScaffoldSlot::Status, "Status")]),
            Err(ScaffoldError::MissingContent)
        );
        assert_eq!(
            Scaffold::new(
                "Workspace",
                [
                    slot(ScaffoldSlot::Content, "Primary"),
                    slot(ScaffoldSlot::Content, "Duplicate")
                ]
            ),
            Err(ScaffoldError::DuplicateSlot(ScaffoldSlot::Content))
        );
    }

    struct Fixture {
        reference: Rc<RefCell<Option<ScaffoldRef>>>,
    }

    impl Component for Fixture {
        type State = ();
        type Action = ();

        fn create(&self, _: &mut CreateContext<'_>) -> Self::State {}

        fn mount(&self, _: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            let scaffold = Scaffold::new(
                "Workspace",
                [
                    slot(ScaffoldSlot::Content, "Document"),
                    slot(ScaffoldSlot::Navigation, "Sections"),
                    slot(ScaffoldSlot::Status, "Sync status"),
                ],
            )
            .unwrap();
            let reference = scaffold
                .mount(ui, root.0, |kind, writer| {
                    writer.text(format!("{kind:?}"), Default::default(), 12.0);
                })
                .unwrap();
            *self.reference.borrow_mut() = Some(reference.clone());
            root
        }

        fn action(&self, _: &mut Self::State, _: Self::Action, _: &mut UpdateContext<'_, Self>) {}
    }

    #[test]
    fn mounted_scaffold_publishes_named_landmarks_and_direct_ownership() {
        let reference = Rc::new(RefCell::new(None));
        let runtime = ViewRuntime::from_component(Fixture {
            reference: reference.clone(),
        })
        .unwrap();
        let reference = reference.borrow().clone().unwrap();
        assert_eq!(
            runtime.ui().kinds.get(reference.node()),
            Some(&NodeKind::Box)
        );
        let root = runtime.ui().semantics.get(reference.node()).unwrap();
        assert_eq!(root.role, SemanticRole::Application);
        assert_eq!(root.relationships.len(), 3);
        assert!(
            root.relationships
                .iter()
                .all(|relationship| relationship.kind == SemanticRelationshipKind::Owns)
        );
        for (slot, role) in [
            (ScaffoldSlot::Navigation, SemanticRole::Navigation),
            (ScaffoldSlot::Content, SemanticRole::Main),
            (ScaffoldSlot::Status, SemanticRole::Status),
        ] {
            let node = reference.slot(slot).unwrap().node();
            assert_eq!(runtime.ui().semantics.get(node).unwrap().role, role);
        }
    }
}

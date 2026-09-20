//! Ordered, platform-neutral overlay lifecycle records.
//!
//! Runtime portals remain responsible for visual ownership. Layout owns popup placement, input owns
//! routing, and focus owners apply the requests returned here. This module starts none of them.

use std::num::NonZeroU32;

use crate::foundation::{PointF, RectF};
use crate::graphics::scene::NodeId as UiNodeId;

use crate::ui::MountedUi;

/// Stable identity for one overlay-slot generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct OverlayId {
    slot: NonZeroU32,
    generation: NonZeroU32,
}

impl OverlayId {
    pub const fn new(slot: NonZeroU32, generation: NonZeroU32) -> Self {
        Self { slot, generation }
    }

    pub const fn from_raw(slot: u32, generation: u32) -> Option<Self> {
        match (NonZeroU32::new(slot), NonZeroU32::new(generation)) {
            (Some(slot), Some(generation)) => Some(Self { slot, generation }),
            _ => None,
        }
    }

    pub const fn slot(self) -> u32 {
        self.slot.get()
    }

    pub const fn generation(self) -> u32 {
        self.generation.get()
    }
}

/// Anchor whose geometry is resolved later by the placement owner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OverlayAnchor {
    Node(UiNodeId),
    Point(PointF),
    Rect(RectF),
}

impl OverlayAnchor {
    pub const fn node(self) -> Option<UiNodeId> {
        match self {
            Self::Node(node) => Some(node),
            Self::Point(_) | Self::Rect(_) => None,
        }
    }

    fn validate(self, ui: &MountedUi) -> Result<(), OverlayError> {
        match self {
            Self::Node(node) if !ui.nodes.contains(node) => Err(OverlayError::UnknownAnchor(node)),
            Self::Node(_) => Ok(()),
            Self::Point(point) if !point.x.is_finite() || !point.y.is_finite() => {
                Err(OverlayError::InvalidAnchorGeometry)
            }
            Self::Point(_) => Ok(()),
            Self::Rect(rect)
                if !rect.x.is_finite()
                    || !rect.y.is_finite()
                    || !rect.width.is_finite()
                    || !rect.height.is_finite()
                    || rect.width < 0.0
                    || rect.height < 0.0 =>
            {
                Err(OverlayError::InvalidAnchorGeometry)
            }
            Self::Rect(_) => Ok(()),
        }
    }
}

/// Whether lower content must become inert while this entry is active.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OverlayModality {
    #[default]
    NonModal,
    Modal,
}

/// Whether an outside press is ignored or closes before the remaining input route continues.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutsidePressPolicy {
    #[default]
    Ignore,
    DismissAndPropagate,
    DismissAndConsume,
}

impl OutsidePressPolicy {
    const fn dismisses(self) -> bool {
        !matches!(self, Self::Ignore)
    }

    const fn consumes(self) -> bool {
        matches!(self, Self::DismissAndConsume)
    }
}

/// Caller-selected user-dismissal policy. Forced lifecycle closure ignores these switches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OverlayDismissPolicy {
    pub escape: bool,
    pub outside_press: OutsidePressPolicy,
    pub focus_lost: bool,
    pub pointer_departure: bool,
}

impl OverlayDismissPolicy {
    const fn allows(self, reason: DismissReason) -> bool {
        match reason {
            DismissReason::Escape => self.escape,
            DismissReason::OutsidePress => self.outside_press.dismisses(),
            DismissReason::FocusLost => self.focus_lost,
            DismissReason::PointerDeparture => self.pointer_departure,
            DismissReason::Accepted
            | DismissReason::Cancelled
            | DismissReason::AnchorRemoved
            | DismissReason::Replaced
            | DismissReason::ViewLost
            | DismissReason::OwnerUnmounted => true,
        }
    }
}

/// Why an overlay closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DismissReason {
    Accepted,
    Cancelled,
    Escape,
    OutsidePress,
    AnchorRemoved,
    FocusLost,
    PointerDeparture,
    Replaced,
    ViewLost,
    OwnerUnmounted,
}

/// Initial focus intent emitted after visual content is mounted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OverlayInitialFocus {
    #[default]
    None,
    FirstFocusable,
    SelectedOrFirst,
    Explicit(UiNodeId),
}

/// Focus containment is a lifecycle record, not an implementation of focus traversal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OverlayFocusContainment {
    #[default]
    None,
    Contain,
}

/// Focus restoration intent after a close.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OverlayFocusRestoration {
    #[default]
    None,
    Target(UiNodeId),
    TargetThenNearest(UiNodeId),
}

/// Complete focus lifecycle recorded when the overlay opens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OverlayFocusLifecycle {
    pub initial: OverlayInitialFocus,
    pub containment: OverlayFocusContainment,
    pub restoration: OverlayFocusRestoration,
}

/// Request for the separate focus owner. No focus mutation happens in this module.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OverlayFocusRequest {
    #[default]
    None,
    Initial(OverlayInitialFocus),
    Restore {
        target: UiNodeId,
        nearest_fallback: bool,
    },
}

impl OverlayFocusLifecycle {
    const fn opening_request(self) -> OverlayFocusRequest {
        match self.initial {
            OverlayInitialFocus::None => OverlayFocusRequest::None,
            initial => OverlayFocusRequest::Initial(initial),
        }
    }

    const fn closing_request(self) -> OverlayFocusRequest {
        match self.restoration {
            OverlayFocusRestoration::None => OverlayFocusRequest::None,
            OverlayFocusRestoration::Target(target) => OverlayFocusRequest::Restore {
                target,
                nearest_fallback: false,
            },
            OverlayFocusRestoration::TargetThenNearest(target) => OverlayFocusRequest::Restore {
                target,
                nearest_fallback: true,
            },
        }
    }
}

/// Controlled values used to open one stack entry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverlayOpenRequest {
    pub anchor: OverlayAnchor,
    pub parent: Option<OverlayId>,
    pub modality: OverlayModality,
    pub dismissal: OverlayDismissPolicy,
    pub focus: OverlayFocusLifecycle,
}

impl OverlayOpenRequest {
    pub const fn anchored(node: UiNodeId) -> Self {
        Self {
            anchor: OverlayAnchor::Node(node),
            parent: None,
            modality: OverlayModality::NonModal,
            dismissal: OverlayDismissPolicy {
                escape: false,
                outside_press: OutsidePressPolicy::Ignore,
                focus_lost: false,
                pointer_departure: false,
            },
            focus: OverlayFocusLifecycle {
                initial: OverlayInitialFocus::None,
                containment: OverlayFocusContainment::None,
                restoration: OverlayFocusRestoration::None,
            },
        }
    }
}

/// One immutable stack record.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverlayEntry {
    pub id: OverlayId,
    pub anchor: OverlayAnchor,
    pub parent: Option<OverlayId>,
    pub modality: OverlayModality,
    pub dismissal: OverlayDismissPolicy,
    pub focus: OverlayFocusLifecycle,
}

/// Result of successfully opening one entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OverlayOpened {
    pub id: OverlayId,
    pub focus: OverlayFocusRequest,
}

/// One closed entry, emitted in topmost-to-bottommost order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OverlayDismissed {
    pub id: OverlayId,
    pub reason: DismissReason,
}

/// Atomic subtree-close outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverlayCloseOutcome {
    pub dismissed: Vec<OverlayDismissed>,
    pub focus: OverlayFocusRequest,
    /// Whether the triggering outside press must stop before normal routing can reopen the anchor.
    pub consume_input: bool,
}

/// A policy-gated dismissal either closes an overlay subtree or leaves it active.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OverlayDismissResult {
    Dismissed(OverlayCloseOutcome),
    Blocked {
        id: OverlayId,
        reason: DismissReason,
    },
}

/// Rejected operation. Rejection leaves stack order and generations unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayError {
    UnknownAnchor(UiNodeId),
    UnknownFocusTarget(UiNodeId),
    UnknownParent(OverlayId),
    StaleOverlay(OverlayId),
    InvalidAnchorGeometry,
    ModalRequiresInitialFocus,
    ModalRequiresFocusContainment,
    CapacityExhausted,
}

/// Deterministic counters for later per-view aggregation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OverlayDiagnostics {
    pub opened: u64,
    pub dismissed: u64,
    pub blocked_dismissals: u64,
    pub anchor_removals: u64,
    pub initial_focus_requests: u64,
    pub restoration_requests: u64,
    pub failures: u64,
}

#[derive(Clone, Copy, Debug)]
struct OverlaySlot {
    generation: u32,
    occupied: bool,
}

/// Pure ordered lifecycle owner for one view's overlays.
#[derive(Clone, Debug, Default)]
pub struct OverlayHost {
    entries: Vec<OverlayEntry>,
    slots: Vec<OverlaySlot>,
    free_slots: Vec<u32>,
    diagnostics: OverlayDiagnostics,
}

impl OverlayHost {
    pub fn diagnostics(&self) -> OverlayDiagnostics {
        self.diagnostics
    }

    /// Entries in painter/input order from bottommost to topmost.
    pub fn entries(&self) -> &[OverlayEntry] {
        &self.entries
    }

    pub fn entry(&self, id: OverlayId) -> Option<&OverlayEntry> {
        self.id_is_live(id)
            .then(|| self.entries.iter().find(|entry| entry.id == id))
            .flatten()
    }

    pub fn top(&self) -> Option<&OverlayEntry> {
        self.entries.last()
    }

    pub fn active_modal(&self) -> Option<OverlayId> {
        self.entries
            .iter()
            .rev()
            .find(|entry| entry.modality == OverlayModality::Modal)
            .map(|entry| entry.id)
    }

    pub fn background_is_inert(&self) -> bool {
        self.active_modal().is_some()
    }

    pub fn open(
        &mut self,
        ui: &MountedUi,
        request: OverlayOpenRequest,
    ) -> Result<OverlayOpened, OverlayError> {
        if let Err(error) = self.validate_open(ui, request) {
            self.diagnostics.failures += 1;
            return Err(error);
        }
        let id = match self.allocate_id() {
            Ok(id) => id,
            Err(error) => {
                self.diagnostics.failures += 1;
                return Err(error);
            }
        };
        let focus = request.focus.opening_request();
        self.entries.push(OverlayEntry {
            id,
            anchor: request.anchor,
            parent: request.parent,
            modality: request.modality,
            dismissal: request.dismissal,
            focus: request.focus,
        });
        self.diagnostics.opened += 1;
        self.diagnostics.initial_focus_requests += u64::from(focus != OverlayFocusRequest::None);
        Ok(OverlayOpened { id, focus })
    }

    /// Applies a policy-gated user or programmatic dismissal to an entry and its descendants.
    pub fn dismiss(
        &mut self,
        id: OverlayId,
        reason: DismissReason,
    ) -> Result<OverlayDismissResult, OverlayError> {
        let Some(entry) = self.entry(id).copied() else {
            self.diagnostics.failures += 1;
            return Err(OverlayError::StaleOverlay(id));
        };
        if !entry.dismissal.allows(reason) {
            self.diagnostics.blocked_dismissals += 1;
            return Ok(OverlayDismissResult::Blocked { id, reason });
        }
        let consume_input =
            reason == DismissReason::OutsidePress && entry.dismissal.outside_press.consumes();
        Ok(OverlayDismissResult::Dismissed(self.close_subtree(
            id,
            reason,
            consume_input,
        )))
    }

    /// Closes every overlay directly anchored to the removed node and their descendants.
    pub fn anchor_removed(&mut self, node: UiNodeId) -> Vec<OverlayCloseOutcome> {
        let direct: Vec<_> = self
            .entries
            .iter()
            .filter(|entry| entry.anchor.node() == Some(node))
            .map(|entry| entry.id)
            .collect();
        let mut roots: Vec<_> = direct
            .iter()
            .copied()
            .filter(|candidate| {
                !direct.iter().copied().any(|possible_parent| {
                    possible_parent != *candidate
                        && self.is_descendant_of(*candidate, possible_parent)
                })
            })
            .collect();
        roots.sort_by_key(|id| {
            std::cmp::Reverse(
                self.entries
                    .iter()
                    .position(|entry| entry.id == *id)
                    .unwrap_or(0),
            )
        });
        self.diagnostics.anchor_removals += u64::from(!roots.is_empty());
        roots
            .into_iter()
            .map(|id| self.close_subtree(id, DismissReason::AnchorRemoved, false))
            .collect()
    }

    /// Forced view/owner shutdown, returned in one top-to-bottom close record.
    pub fn close_all(&mut self, reason: DismissReason) -> OverlayCloseOutcome {
        let focus = self
            .entries
            .first()
            .map_or(OverlayFocusRequest::None, |entry| {
                entry.focus.closing_request()
            });
        let ids: Vec<_> = self.entries.iter().rev().map(|entry| entry.id).collect();
        for id in &ids {
            self.release_id(*id);
        }
        self.entries.clear();
        self.diagnostics.dismissed += ids.len() as u64;
        self.diagnostics.restoration_requests += u64::from(focus != OverlayFocusRequest::None);
        OverlayCloseOutcome {
            dismissed: ids
                .into_iter()
                .map(|id| OverlayDismissed { id, reason })
                .collect(),
            focus,
            consume_input: false,
        }
    }

    fn validate_open(
        &self,
        ui: &MountedUi,
        request: OverlayOpenRequest,
    ) -> Result<(), OverlayError> {
        request.anchor.validate(ui)?;
        if let Some(parent) = request.parent
            && self.entry(parent).is_none()
        {
            return Err(OverlayError::UnknownParent(parent));
        }
        for target in [
            match request.focus.initial {
                OverlayInitialFocus::Explicit(target) => Some(target),
                _ => None,
            },
            match request.focus.restoration {
                OverlayFocusRestoration::Target(target)
                | OverlayFocusRestoration::TargetThenNearest(target) => Some(target),
                OverlayFocusRestoration::None => None,
            },
        ]
        .into_iter()
        .flatten()
        {
            if !ui.nodes.contains(target) {
                return Err(OverlayError::UnknownFocusTarget(target));
            }
        }
        if request.modality == OverlayModality::Modal {
            if request.focus.initial == OverlayInitialFocus::None {
                return Err(OverlayError::ModalRequiresInitialFocus);
            }
            if request.focus.containment != OverlayFocusContainment::Contain {
                return Err(OverlayError::ModalRequiresFocusContainment);
            }
        }
        Ok(())
    }

    fn close_subtree(
        &mut self,
        root: OverlayId,
        reason: DismissReason,
        consume_input: bool,
    ) -> OverlayCloseOutcome {
        let focus = self.entry(root).map_or(OverlayFocusRequest::None, |entry| {
            entry.focus.closing_request()
        });
        let indices: Vec<_> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| self.is_descendant_of(entry.id, root))
            .map(|(index, _)| index)
            .collect();
        let mut dismissed = Vec::with_capacity(indices.len());
        for index in indices.into_iter().rev() {
            let entry = self.entries.remove(index);
            self.release_id(entry.id);
            dismissed.push(OverlayDismissed {
                id: entry.id,
                reason,
            });
        }
        self.diagnostics.dismissed += dismissed.len() as u64;
        self.diagnostics.restoration_requests += u64::from(focus != OverlayFocusRequest::None);
        OverlayCloseOutcome {
            dismissed,
            focus,
            consume_input,
        }
    }

    fn is_descendant_of(&self, mut candidate: OverlayId, ancestor: OverlayId) -> bool {
        loop {
            if candidate == ancestor {
                return true;
            }
            let Some(parent) = self
                .entries
                .iter()
                .find(|entry| entry.id == candidate)
                .and_then(|entry| entry.parent)
            else {
                return false;
            };
            candidate = parent;
        }
    }

    fn allocate_id(&mut self) -> Result<OverlayId, OverlayError> {
        if let Some(index) = self.free_slots.pop() {
            let slot = &mut self.slots[index as usize];
            slot.generation = slot.generation.wrapping_add(1).max(1);
            slot.occupied = true;
            return OverlayId::from_raw(index + 1, slot.generation)
                .ok_or(OverlayError::CapacityExhausted);
        }
        let index: u32 = self
            .slots
            .len()
            .try_into()
            .map_err(|_| OverlayError::CapacityExhausted)?;
        let slot = index
            .checked_add(1)
            .ok_or(OverlayError::CapacityExhausted)?;
        self.slots.push(OverlaySlot {
            generation: 1,
            occupied: true,
        });
        OverlayId::from_raw(slot, 1).ok_or(OverlayError::CapacityExhausted)
    }

    fn release_id(&mut self, id: OverlayId) {
        let index = id.slot() as usize - 1;
        if let Some(slot) = self.slots.get_mut(index)
            && slot.occupied
            && slot.generation == id.generation()
        {
            slot.occupied = false;
            self.free_slots.push(index as u32);
        }
    }

    fn id_is_live(&self, id: OverlayId) -> bool {
        self.slots
            .get(id.slot() as usize - 1)
            .is_some_and(|slot| slot.occupied && slot.generation == id.generation())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{BoxStyle, LayoutStyle, MountWriter};

    fn mounted_nodes() -> (MountedUi, Vec<UiNodeId>) {
        let mut ui = MountedUi::default();
        let mut nodes = Vec::new();
        MountWriter::<()>::new(&mut ui).root(
            BoxStyle::default(),
            LayoutStyle::default(),
            |writer| {
                nodes.push(writer.container(BoxStyle::default(), LayoutStyle::default(), |_| {}));
                nodes.push(writer.container(BoxStyle::default(), LayoutStyle::default(), |_| {}));
                nodes.push(writer.container(BoxStyle::default(), LayoutStyle::default(), |_| {}));
            },
        );
        (ui, nodes)
    }

    fn menu(anchor: UiNodeId) -> OverlayOpenRequest {
        let mut request = OverlayOpenRequest::anchored(anchor);
        request.dismissal.escape = true;
        request.dismissal.outside_press = OutsidePressPolicy::DismissAndConsume;
        request.focus.initial = OverlayInitialFocus::SelectedOrFirst;
        request.focus.restoration = OverlayFocusRestoration::TargetThenNearest(anchor);
        request
    }

    #[test]
    fn stack_order_and_parentage_are_explicit() {
        let (ui, nodes) = mounted_nodes();
        let mut host = OverlayHost::default();
        let parent = host.open(&ui, menu(nodes[0])).unwrap().id;
        let mut child_request = menu(nodes[1]);
        child_request.parent = Some(parent);
        let child = host.open(&ui, child_request).unwrap().id;
        assert_eq!(
            host.entries()
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            vec![parent, child]
        );
        assert_eq!(host.top().unwrap().id, child);
    }

    #[test]
    fn invalid_open_is_atomic_and_does_not_consume_an_identity() {
        let (ui, nodes) = mounted_nodes();
        let mut host = OverlayHost::default();
        let stale = UiNodeId::new(nodes[0].index(), nodes[0].generation() + 1);
        assert_eq!(
            host.open(&ui, OverlayOpenRequest::anchored(stale)),
            Err(OverlayError::UnknownAnchor(stale))
        );
        let opened = host.open(&ui, menu(nodes[0])).unwrap();
        assert_eq!((opened.id.slot(), opened.id.generation()), (1, 1));
        assert_eq!(host.entries().len(), 1);
    }

    #[test]
    fn modal_entries_require_focus_and_make_background_inert() {
        let (ui, nodes) = mounted_nodes();
        let mut host = OverlayHost::default();
        let mut modal = OverlayOpenRequest::anchored(nodes[0]);
        modal.modality = OverlayModality::Modal;
        assert_eq!(
            host.open(&ui, modal),
            Err(OverlayError::ModalRequiresInitialFocus)
        );
        modal.focus.initial = OverlayInitialFocus::FirstFocusable;
        assert_eq!(
            host.open(&ui, modal),
            Err(OverlayError::ModalRequiresFocusContainment)
        );
        modal.focus.containment = OverlayFocusContainment::Contain;
        let opened = host.open(&ui, modal).unwrap();
        assert_eq!(host.active_modal(), Some(opened.id));
        assert!(host.background_is_inert());
    }

    #[test]
    fn dismissal_policy_blocks_without_mutation_and_records_outside_consumption() {
        let (ui, nodes) = mounted_nodes();
        let mut host = OverlayHost::default();
        let mut request = menu(nodes[0]);
        request.dismissal.outside_press = OutsidePressPolicy::Ignore;
        let id = host.open(&ui, request).unwrap().id;
        assert_eq!(
            host.dismiss(id, DismissReason::OutsidePress).unwrap(),
            OverlayDismissResult::Blocked {
                id,
                reason: DismissReason::OutsidePress
            }
        );
        assert_eq!(host.entries().len(), 1);

        host.entries[0].dismissal.outside_press = OutsidePressPolicy::DismissAndConsume;
        let OverlayDismissResult::Dismissed(outcome) =
            host.dismiss(id, DismissReason::OutsidePress).unwrap()
        else {
            panic!("outside press should dismiss");
        };
        assert!(outcome.consume_input);
        assert_eq!(outcome.dismissed[0].id, id);
    }

    #[test]
    fn closing_a_parent_closes_descendants_top_first_and_restores_from_parent() {
        let (ui, nodes) = mounted_nodes();
        let mut host = OverlayHost::default();
        let parent = host.open(&ui, menu(nodes[0])).unwrap().id;
        let mut child_request = menu(nodes[1]);
        child_request.parent = Some(parent);
        let child = host.open(&ui, child_request).unwrap().id;
        let OverlayDismissResult::Dismissed(outcome) =
            host.dismiss(parent, DismissReason::Escape).unwrap()
        else {
            panic!("escape should dismiss");
        };
        assert_eq!(
            outcome.dismissed,
            vec![
                OverlayDismissed {
                    id: child,
                    reason: DismissReason::Escape,
                },
                OverlayDismissed {
                    id: parent,
                    reason: DismissReason::Escape,
                },
            ]
        );
        assert_eq!(
            outcome.focus,
            OverlayFocusRequest::Restore {
                target: nodes[0],
                nearest_fallback: true,
            }
        );
    }

    #[test]
    fn anchor_removal_forces_dependent_subtrees_closed() {
        let (ui, nodes) = mounted_nodes();
        let mut host = OverlayHost::default();
        let parent = host.open(&ui, menu(nodes[0])).unwrap().id;
        let mut child_request = menu(nodes[0]);
        child_request.parent = Some(parent);
        let child = host.open(&ui, child_request).unwrap().id;
        let unrelated = host.open(&ui, menu(nodes[2])).unwrap().id;

        let outcomes = host.anchor_removed(nodes[0]);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            outcomes[0].dismissed,
            vec![
                OverlayDismissed {
                    id: child,
                    reason: DismissReason::AnchorRemoved,
                },
                OverlayDismissed {
                    id: parent,
                    reason: DismissReason::AnchorRemoved,
                },
            ]
        );
        assert_eq!(
            host.entries()
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            vec![unrelated]
        );
    }

    #[test]
    fn released_slots_reject_stale_ids_and_advance_generation() {
        let (ui, nodes) = mounted_nodes();
        let mut host = OverlayHost::default();
        let old = host.open(&ui, menu(nodes[0])).unwrap().id;
        host.dismiss(old, DismissReason::Cancelled).unwrap();
        let replacement = host.open(&ui, menu(nodes[1])).unwrap().id;
        assert_eq!(replacement.slot(), old.slot());
        assert_ne!(replacement.generation(), old.generation());
        assert_eq!(
            host.dismiss(old, DismissReason::Cancelled),
            Err(OverlayError::StaleOverlay(old))
        );
        assert_eq!(host.top().unwrap().id, replacement);
        assert_eq!(
            host.diagnostics(),
            OverlayDiagnostics {
                opened: 2,
                dismissed: 1,
                restoration_requests: 1,
                initial_focus_requests: 2,
                failures: 1,
                ..OverlayDiagnostics::default()
            }
        );
    }

    #[test]
    fn coordinate_anchors_reject_nonfinite_and_negative_geometry() {
        let (ui, _) = mounted_nodes();
        let mut host = OverlayHost::default();
        let requests = [
            OverlayOpenRequest {
                anchor: OverlayAnchor::Point(PointF {
                    x: f32::NAN,
                    y: 0.0,
                }),
                ..OverlayOpenRequest::anchored(UiNodeId::new(1, 1))
            },
            OverlayOpenRequest {
                anchor: OverlayAnchor::Rect(RectF {
                    x: 0.0,
                    y: 0.0,
                    width: -1.0,
                    height: 2.0,
                }),
                ..OverlayOpenRequest::anchored(UiNodeId::new(1, 1))
            },
        ];
        for request in requests {
            assert_eq!(
                host.open(&ui, request),
                Err(OverlayError::InvalidAnchorGeometry)
            );
        }
    }
}

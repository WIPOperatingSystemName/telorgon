use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::integrations::wayland::compositor::{
    BufferDescriptor, ClientId, ClientLimits, ObjectRegistry, ObjectRegistryError, OutputState,
    ProtocolObjectId, SeatState, SerialLedger, SubsurfaceGraph, SurfaceState, WaylandBufferId,
    WaylandSurfaceId, WaylandWorld, WaylandWorldError, XdgSurfaceState,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompositorAction {
    PublishSurface(WaylandSurfaceId),
    /// Apply committed state and callbacks using content already owned by the host.
    UpdateSurface(WaylandSurfaceId),
    WithdrawSurface(WaylandSurfaceId),
    ImportBuffer(WaylandBufferId),
    ReleaseBuffer(WaylandBufferId),
    ActivateSurface {
        surface: WaylandSurfaceId,
        application_id: Option<String>,
        source_surface: Option<WaylandSurfaceId>,
    },
    SessionLockRequested(ProtocolObjectId),
    SessionLockCancelled(ProtocolObjectId),
    SessionUnlockRequested(ProtocolObjectId),
    MoveToplevel(WaylandSurfaceId),
    ResizeToplevel {
        surface: WaylandSurfaceId,
        edge: crate::integrations::wayland::compositor::ResizeEdge,
    },
    MaximizeToplevel {
        surface: WaylandSurfaceId,
        maximized: bool,
    },
    FullscreenToplevel {
        surface: WaylandSurfaceId,
        fullscreen: bool,
        output: Option<u32>,
    },
    MinimizeToplevel(WaylandSurfaceId),
    StartDrag {
        seat: u32,
        origin: WaylandSurfaceId,
        icon: Option<WaylandSurfaceId>,
    },
    FinishDrag {
        icon: Option<WaylandSurfaceId>,
    },
    RepaintOutput(u32),
    DisconnectClient(ClientId),
}

/// Protocol state owned by Telorgon. Native callbacks only decode requests into calls on this type.
#[derive(Debug)]
pub struct CompositorCore {
    pub world: WaylandWorld,
    pub objects: ObjectRegistry,
    pub serials: SerialLedger,
    pub subsurfaces: SubsurfaceGraph,
    pub data_devices: crate::integrations::wayland::compositor::DataDeviceState,
    pub buffer_uses: crate::integrations::wayland::compositor::BufferUseTracker,
    pub seats: BTreeMap<u32, SeatState>,
    pub outputs: BTreeMap<u32, OutputState>,
    buffers: BTreeMap<WaylandBufferId, (ClientId, BufferDescriptor)>,
    xdg_surfaces: BTreeMap<WaylandSurfaceId, XdgSurfaceState>,
    actions: Vec<CompositorAction>,
    // Buffer ownership is not coalesced with the latest image publication.
    queued_publications: BTreeMap<WaylandSurfaceId, QueuedPublication>,
    dispatched_attachments: BTreeMap<WaylandSurfaceId, u64>,
    superseded_publications: Vec<(WaylandSurfaceId, u64, WaylandBufferId)>,
}

#[derive(Debug)]
struct QueuedPublication {
    action_index: usize,
    buffer_use: Option<(u64, WaylandBufferId)>,
}

impl CompositorCore {
    pub fn new(limits: ClientLimits) -> Result<Self, CompositorCoreError> {
        Ok(Self {
            world: WaylandWorld::with_limits(limits)?,
            objects: ObjectRegistry::new(limits.maximum_protocol_objects)?,
            serials: SerialLedger::default(),
            subsurfaces: SubsurfaceGraph::default(),
            data_devices: crate::integrations::wayland::compositor::DataDeviceState::default(),
            buffer_uses: crate::integrations::wayland::compositor::BufferUseTracker::default(),
            seats: BTreeMap::new(),
            outputs: BTreeMap::new(),
            buffers: BTreeMap::new(),
            xdg_surfaces: BTreeMap::new(),
            actions: Vec::new(),
            queued_publications: BTreeMap::new(),
            dispatched_attachments: BTreeMap::new(),
            superseded_publications: Vec::new(),
        })
    }

    pub fn connect_client(&mut self, client: ClientId) -> Result<(), CompositorCoreError> {
        self.world.add_client(client)?;
        Ok(())
    }

    pub fn disconnect_client(&mut self, client: ClientId) -> Result<(), CompositorCoreError> {
        let surfaces = self.world.client_surfaces(client);
        self.world.remove_client(client)?;
        self.objects.remove_client(client);
        self.serials.remove_client(client);
        self.data_devices.remove_client(client);
        self.buffers.retain(|_, (owner, _)| *owner != client);
        self.xdg_surfaces
            .retain(|surface, _| self.world.surface(*surface).is_some());
        for seat in self.seats.values_mut() {
            seat.remove_client(client);
        }
        for surface in surfaces {
            self.queue_action(CompositorAction::WithdrawSurface(surface));
        }
        self.actions
            .push(CompositorAction::DisconnectClient(client));
        Ok(())
    }

    pub fn destroy_surface(
        &mut self,
        client: ClientId,
        surface: WaylandSurfaceId,
    ) -> Result<SurfaceState, CompositorCoreError> {
        let state = self.world.destroy_surface(client, surface)?;
        self.xdg_surfaces.remove(&surface);
        for seat in self.seats.values_mut() {
            seat.remove_surface(surface);
        }
        self.queue_action(CompositorAction::WithdrawSurface(surface));
        Ok(state)
    }

    pub fn register_buffer(
        &mut self,
        client: ClientId,
        buffer: WaylandBufferId,
        descriptor: BufferDescriptor,
    ) -> Result<(), CompositorCoreError> {
        if self.buffers.contains_key(&buffer) {
            return Err(CompositorCoreError::DuplicateBuffer);
        }
        self.buffers.insert(buffer, (client, descriptor));
        self.actions.push(CompositorAction::ImportBuffer(buffer));
        Ok(())
    }

    pub fn destroy_buffer(
        &mut self,
        client: ClientId,
        buffer: WaylandBufferId,
    ) -> Result<BufferDescriptor, CompositorCoreError> {
        if self.buffers.get(&buffer).map(|entry| entry.0) != Some(client) {
            return Err(CompositorCoreError::BufferOwnershipMismatch);
        }
        let (_, descriptor) = self.buffers.remove(&buffer).expect("checked above");
        self.actions.push(CompositorAction::ReleaseBuffer(buffer));
        Ok(descriptor)
    }

    pub fn buffer(&self, buffer: WaylandBufferId) -> Option<&BufferDescriptor> {
        self.buffers.get(&buffer).map(|(_, descriptor)| descriptor)
    }

    pub fn buffer_owner(&self, buffer: WaylandBufferId) -> Option<ClientId> {
        self.buffers.get(&buffer).map(|(owner, _)| *owner)
    }

    pub fn create_xdg_surface(
        &mut self,
        client: ClientId,
        surface: WaylandSurfaceId,
        object: ProtocolObjectId,
        version: u32,
    ) -> Result<&mut XdgSurfaceState, CompositorCoreError> {
        if self.world.surface(surface).is_none() {
            return Err(CompositorCoreError::UnknownSurface);
        }
        if self.world.surface_owner(surface) != Some(client) {
            return Err(CompositorCoreError::UnknownSurface);
        }
        self.objects.insert(
            object,
            crate::integrations::wayland::compositor::ObjectMetadata {
                owner: client,
                kind: crate::integrations::wayland::compositor::ProtocolObjectKind::XdgSurface,
                version,
            },
        )?;
        if self.xdg_surfaces.contains_key(&surface) {
            return Err(CompositorCoreError::DuplicateXdgSurface);
        }
        self.xdg_surfaces
            .insert(surface, XdgSurfaceState::new(surface));
        Ok(self.xdg_surfaces.get_mut(&surface).expect("inserted"))
    }

    pub fn xdg_surface_mut(&mut self, surface: WaylandSurfaceId) -> Option<&mut XdgSurfaceState> {
        self.xdg_surfaces.get_mut(&surface)
    }

    pub fn xdg_surface(&self, surface: WaylandSurfaceId) -> Option<&XdgSurfaceState> {
        self.xdg_surfaces.get(&surface)
    }

    pub fn queue_action(&mut self, action: CompositorAction) {
        if let CompositorAction::PublishSurface(surface)
        | CompositorAction::WithdrawSurface(surface) = action
        {
            let buffer_use = if matches!(action, CompositorAction::PublishSurface(_)) {
                self.world.surface(surface).and_then(|state| {
                    let snapshot = state.snapshot();
                    snapshot
                        .attachment
                        .filter(|_| {
                            self.dispatched_attachments.get(&surface)
                                != Some(&snapshot.attachment_revision)
                        })
                        .map(|attachment| (snapshot.attachment_revision, attachment.buffer))
                })
            } else {
                None
            };
            let previous = self.queued_publications.insert(
                surface,
                QueuedPublication {
                    action_index: self.actions.len(),
                    buffer_use,
                },
            );
            if let Some((revision, buffer)) = previous.and_then(|pending| pending.buffer_use)
                && Some((revision, buffer)) != buffer_use
            {
                self.superseded_publications
                    .push((surface, revision, buffer));
            }
        }
        // Appending is O(1). Avoid rescanning the whole batch for every commit;
        // drain_actions uses the recorded final position to preserve event order.
        self.actions.push(action);
    }

    /// Unsubmitted uses superseded before the host consumes its action batch. The host must
    /// finish their per-commit releases, and release storage only when no other use retains it.
    pub fn take_superseded_publications(
        &mut self,
    ) -> Vec<(WaylandSurfaceId, u64, WaylandBufferId)> {
        std::mem::take(&mut self.superseded_publications)
    }

    pub fn pending_publication_buffers(&self) -> BTreeSet<WaylandBufferId> {
        self.queued_publications
            .values()
            .filter_map(|pending| pending.buffer_use.map(|(_, buffer)| buffer))
            .collect()
    }

    pub fn drain_actions(&mut self) -> impl Iterator<Item = CompositorAction> + '_ {
        let publications = std::mem::take(&mut self.queued_publications);
        for (&surface, publication) in &publications {
            if let Some((revision, _)) = publication.buffer_use {
                self.dispatched_attachments.insert(surface, revision);
            }
            if matches!(
                self.actions[publication.action_index],
                CompositorAction::WithdrawSurface(_)
            ) {
                self.dispatched_attachments.remove(&surface);
            }
        }
        self.actions
            .drain(..)
            .enumerate()
            .filter_map(move |(index, action)| {
                if let CompositorAction::PublishSurface(surface)
                | CompositorAction::WithdrawSurface(surface) = action
                {
                    if publications
                        .get(&surface)
                        .is_some_and(|pending| pending.action_index != index)
                    {
                        return None;
                    }
                }
                if let CompositorAction::PublishSurface(surface) = action
                    && publications
                        .get(&surface)
                        .is_some_and(|pending| pending.buffer_use.is_none())
                {
                    return Some(CompositorAction::UpdateSurface(surface));
                }
                Some(action)
            })
    }
}

impl Default for CompositorCore {
    fn default() -> Self {
        Self::new(ClientLimits::default()).expect("default compositor limits are valid")
    }
}

#[derive(Debug)]
pub enum CompositorCoreError {
    World(WaylandWorldError),
    Objects(ObjectRegistryError),
    DuplicateBuffer,
    BufferOwnershipMismatch,
    UnknownSurface,
    DuplicateXdgSurface,
}

impl fmt::Display for CompositorCoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Wayland compositor core operation failed: {self:?}"
        )
    }
}

impl std::error::Error for CompositorCoreError {}

impl From<WaylandWorldError> for CompositorCoreError {
    fn from(value: WaylandWorldError) -> Self {
        Self::World(value)
    }
}

impl From<ObjectRegistryError> for CompositorCoreError {
    fn from(value: ObjectRegistryError) -> Self {
        Self::Objects(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::wayland::compositor::{CursorImage, KeyboardFocus, PointerFocus, SeatCapabilities};
    use crate::foundation::PointF;

    fn client(raw: u32) -> ClientId {
        ClientId::from_raw(raw).unwrap()
    }

    fn surface(raw: u32) -> WaylandSurfaceId {
        WaylandSurfaceId::from_raw(raw).unwrap()
    }

    #[test]
    fn destroying_a_surface_withdraws_it_and_clears_seat_references() {
        let client = client(1);
        let surface = surface(7);
        let mut core = CompositorCore::default();
        core.connect_client(client).unwrap();
        core.world.create_surface(client, surface).unwrap();
        let mut seat = SeatState::new("seat0", SeatCapabilities::default());
        seat.pointer_focus = Some(PointerFocus {
            client,
            surface,
            position: PointF::default(),
            enter_serial: 1,
        });
        seat.keyboard_focus = Some(KeyboardFocus {
            client,
            surface,
            enter_serial: 2,
        });
        seat.cursor = CursorImage::ClientSurface {
            surface,
            hotspot_x: 0,
            hotspot_y: 0,
        };
        core.seats.insert(1, seat);

        core.destroy_surface(client, surface).unwrap();

        let seat = core.seats.get(&1).unwrap();
        assert!(seat.pointer_focus.is_none());
        assert!(seat.keyboard_focus.is_none());
        assert_eq!(seat.cursor, CursorImage::TelorgonDefault);
        assert_eq!(
            core.drain_actions().collect::<Vec<_>>(),
            vec![CompositorAction::WithdrawSurface(surface)]
        );
    }

    #[test]
    fn disconnecting_a_client_withdraws_all_of_its_surfaces() {
        let client = client(1);
        let mut core = CompositorCore::default();
        core.connect_client(client).unwrap();
        for surface in [surface(7), surface(8)] {
            core.world.create_surface(client, surface).unwrap();
        }

        core.disconnect_client(client).unwrap();

        assert_eq!(
            core.drain_actions().collect::<Vec<_>>(),
            vec![
                CompositorAction::WithdrawSurface(surface(7)),
                CompositorAction::WithdrawSurface(surface(8)),
                CompositorAction::DisconnectClient(client),
            ]
        );
    }
}

#[cfg(test)]
mod publication_tests {
    use super::*;
    use crate::{
        compositor_wayland::{BufferAttachment, SurfaceRole},
        core::PointI,
    };

    fn setup() -> (CompositorCore, ClientId, WaylandSurfaceId) {
        let mut core = CompositorCore::default();
        let client = ClientId::from_raw(1).unwrap();
        let surface = WaylandSurfaceId::from_raw(2).unwrap();
        core.connect_client(client).unwrap();
        core.world.create_surface(client, surface).unwrap();
        core.world
            .surface_mut(surface)
            .unwrap()
            .assign_role(SurfaceRole::Xwayland)
            .unwrap();
        (core, client, surface)
    }
    fn publish(core: &mut CompositorCore, surface: WaylandSurfaceId, buffer: u32) -> u64 {
        let state = core.world.surface_mut(surface).unwrap();
        state.attach(Some(BufferAttachment {
            buffer: WaylandBufferId::from_raw(buffer).unwrap(),
            offset: PointI::default(),
        }));
        let revision = state.commit().unwrap().revision;
        core.queue_action(CompositorAction::PublishSurface(surface));
        revision
    }
    #[test]
    fn batched_images_coalesce_but_each_superseded_use_is_retired() {
        let (mut core, _, surface) = setup();
        let revision = publish(&mut core, surface, 10);
        publish(&mut core, surface, 11);
        assert_eq!(
            core.take_superseded_publications(),
            vec![(surface, revision, WaylandBufferId::from_raw(10).unwrap())]
        );
        assert!(
            core.pending_publication_buffers()
                .contains(&WaylandBufferId::from_raw(11).unwrap())
        );
        assert!(
            !core
                .pending_publication_buffers()
                .contains(&WaylandBufferId::from_raw(10).unwrap())
        );
        assert_eq!(
            core.drain_actions().collect::<Vec<_>>(),
            vec![CompositorAction::PublishSurface(surface)]
        );
        assert!(
            !core
                .pending_publication_buffers()
                .contains(&WaylandBufferId::from_raw(11).unwrap())
        );
        publish(&mut core, surface, 12);
        assert!(core.take_superseded_publications().is_empty()); // Previous batch belongs to the host.
    }
    #[test]
    fn duplicate_publication_is_not_a_second_buffer_use() {
        let (mut core, _, surface) = setup();
        publish(&mut core, surface, 10);
        core.queue_action(CompositorAction::PublishSurface(surface));
        assert!(core.take_superseded_publications().is_empty());
        assert_eq!(
            core.drain_actions().collect::<Vec<_>>(),
            vec![CompositorAction::PublishSurface(surface)]
        );
    }

    #[test]
    fn coalescing_preserves_other_actions_and_latest_publication_order() {
        let (mut core, _, surface) = setup();
        publish(&mut core, surface, 10);
        core.queue_action(CompositorAction::RepaintOutput(1));
        publish(&mut core, surface, 11);
        core.queue_action(CompositorAction::RepaintOutput(2));
        assert_eq!(
            core.drain_actions().collect::<Vec<_>>(),
            vec![
                CompositorAction::RepaintOutput(1),
                CompositorAction::PublishSurface(surface),
                CompositorAction::RepaintOutput(2),
            ]
        );
    }

    #[test]
    fn destruction_retires_unsubmitted_buffers_and_removes_stale_publication() {
        let (mut core, client, surface) = setup();
        let revision = publish(&mut core, surface, 10);
        core.destroy_surface(client, surface).unwrap();
        assert_eq!(
            core.take_superseded_publications(),
            vec![(surface, revision, WaylandBufferId::from_raw(10).unwrap())]
        );
        assert_eq!(
            core.drain_actions().collect::<Vec<_>>(),
            vec![CompositorAction::WithdrawSurface(surface)]
        );
    }
    #[test]
    fn reused_buffer_has_separate_commit_release_but_retains_latest_storage_use() {
        let (mut core, _, surface) = setup();
        let revision = publish(&mut core, surface, 10);
        publish(&mut core, surface, 10);
        assert_eq!(core.take_superseded_publications().len(), 1);
        assert!(
            core.pending_publication_buffers()
                .contains(&WaylandBufferId::from_raw(10).unwrap())
        );
        assert!(core.world.surface(surface).unwrap().snapshot().revision > revision);
    }
}

#[cfg(test)]
mod buffer_lifetime_tests {
    use super::*;
    use crate::integrations::wayland::compositor::BufferAttachment;

    #[test]
    fn state_commits_preserve_a_pending_use_and_never_reacquire_a_dispatched_use() {
        let mut core = CompositorCore::default();
        let client = ClientId::from_raw(1).unwrap();
        let surface = WaylandSurfaceId::from_raw(1).unwrap();
        let buffer = WaylandBufferId::from_raw(10).unwrap();
        core.connect_client(client).unwrap();
        core.world.create_surface(client, surface).unwrap();
        let state = core.world.surface_mut(surface).unwrap();
        state.attach(Some(BufferAttachment {
            buffer,
            offset: Default::default(),
        }));
        let attached = state.commit().unwrap().revision;
        core.queue_action(CompositorAction::PublishSurface(surface));
        // Callback/state commits arriving in the same batch must not retire the attachment.
        core.world.surface_mut(surface).unwrap().commit().unwrap();
        core.queue_action(CompositorAction::PublishSurface(surface));
        assert!(core.take_superseded_publications().is_empty());
        assert_eq!(core.pending_publication_buffers(), BTreeSet::from([buffer]));
        assert_eq!(
            core.drain_actions().collect::<Vec<_>>(),
            vec![CompositorAction::PublishSurface(surface)]
        );
        assert_eq!(
            core.world
                .surface(surface)
                .unwrap()
                .snapshot()
                .attachment_revision,
            attached
        );
        // Reproduce Firefox's frame-only commit after the host copied and released its buffer.
        for _ in 0..3 {
            core.world.surface_mut(surface).unwrap().commit().unwrap();
            core.queue_action(CompositorAction::PublishSurface(surface));
            assert!(core.pending_publication_buffers().is_empty());
            assert!(core.take_superseded_publications().is_empty());
            assert_eq!(
                core.drain_actions().collect::<Vec<_>>(),
                vec![CompositorAction::UpdateSurface(surface)]
            );
        }
        // An explicit reattachment of the same object is a new use, unlike a state-only commit.
        let state = core.world.surface_mut(surface).unwrap();
        state.attach(Some(BufferAttachment {
            buffer,
            offset: Default::default(),
        }));
        state.commit().unwrap();
        core.queue_action(CompositorAction::PublishSurface(surface));
        assert_eq!(
            core.drain_actions().collect::<Vec<_>>(),
            vec![CompositorAction::PublishSurface(surface)]
        );
    }

    #[test]
    fn state_only_commit_superseded_by_a_new_attachment_does_not_release_old_storage() {
        let mut core = CompositorCore::default();
        let client = ClientId::from_raw(1).unwrap();
        let surface = WaylandSurfaceId::from_raw(1).unwrap();
        core.connect_client(client).unwrap();
        core.world.create_surface(client, surface).unwrap();
        for buffer_id in [10, 11] {
            let state = core.world.surface_mut(surface).unwrap();
            state.attach(Some(BufferAttachment {
                buffer: WaylandBufferId::from_raw(buffer_id).unwrap(),
                offset: Default::default(),
            }));
            state.commit().unwrap();
            core.queue_action(CompositorAction::PublishSurface(surface));
            assert!(core.take_superseded_publications().is_empty());
            core.drain_actions().for_each(drop);
            core.world.surface_mut(surface).unwrap().commit().unwrap();
            core.queue_action(CompositorAction::PublishSurface(surface));
        }
        core.destroy_surface(client, surface).unwrap();
        assert!(core.take_superseded_publications().is_empty());
        assert_eq!(
            core.drain_actions().collect::<Vec<_>>(),
            vec![CompositorAction::WithdrawSurface(surface)]
        );
    }
}

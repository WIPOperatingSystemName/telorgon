use std::collections::BTreeMap;
use std::fmt;

use crate::integrations::wayland::compositor::{ClientId, SurfaceState, WaylandSurfaceId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientLimits {
    pub maximum_surfaces: usize,
    pub maximum_protocol_objects: usize,
    pub maximum_buffer_bytes: usize,
}

impl Default for ClientLimits {
    fn default() -> Self {
        Self {
            maximum_surfaces: 4_096,
            maximum_protocol_objects: 16_384,
            maximum_buffer_bytes: 512 * 1024 * 1024,
        }
    }
}

#[derive(Debug)]
struct ClientState {
    surfaces: BTreeMap<WaylandSurfaceId, SurfaceState>,
}

#[derive(Debug, Default)]
pub struct WaylandWorld {
    clients: BTreeMap<ClientId, ClientState>,
    surface_owner: BTreeMap<WaylandSurfaceId, ClientId>,
    limits: ClientLimits,
}

impl WaylandWorld {
    pub fn with_limits(limits: ClientLimits) -> Result<Self, WaylandWorldError> {
        if limits.maximum_surfaces == 0
            || limits.maximum_protocol_objects == 0
            || limits.maximum_buffer_bytes == 0
        {
            return Err(WaylandWorldError::InvalidLimits);
        }
        Ok(Self {
            clients: BTreeMap::new(),
            surface_owner: BTreeMap::new(),
            limits,
        })
    }

    pub fn add_client(&mut self, client: ClientId) -> Result<(), WaylandWorldError> {
        if self.clients.contains_key(&client) {
            return Err(WaylandWorldError::DuplicateClient);
        }
        self.clients.insert(
            client,
            ClientState {
                surfaces: BTreeMap::new(),
            },
        );
        Ok(())
    }

    pub fn remove_client(&mut self, client: ClientId) -> Result<usize, WaylandWorldError> {
        let state = self
            .clients
            .remove(&client)
            .ok_or(WaylandWorldError::UnknownClient)?;
        let removed = state.surfaces.len();
        for surface in state.surfaces.keys() {
            self.surface_owner.remove(surface);
        }
        Ok(removed)
    }

    pub fn create_surface(
        &mut self,
        client: ClientId,
        surface: WaylandSurfaceId,
    ) -> Result<&mut SurfaceState, WaylandWorldError> {
        if self.surface_owner.contains_key(&surface) {
            return Err(WaylandWorldError::DuplicateSurface);
        }
        let client_state = self
            .clients
            .get_mut(&client)
            .ok_or(WaylandWorldError::UnknownClient)?;
        if client_state.surfaces.len() >= self.limits.maximum_surfaces {
            return Err(WaylandWorldError::SurfaceLimitExceeded);
        }
        client_state
            .surfaces
            .insert(surface, SurfaceState::new(surface));
        self.surface_owner.insert(surface, client);
        Ok(client_state.surfaces.get_mut(&surface).unwrap())
    }

    pub fn surface(&self, surface: WaylandSurfaceId) -> Option<&SurfaceState> {
        let owner = self.surface_owner.get(&surface)?;
        self.clients.get(owner)?.surfaces.get(&surface)
    }

    pub fn surface_owner(&self, surface: WaylandSurfaceId) -> Option<ClientId> {
        self.surface_owner.get(&surface).copied()
    }

    pub fn client_surfaces(&self, client: ClientId) -> Vec<WaylandSurfaceId> {
        self.clients
            .get(&client)
            .map(|state| state.surfaces.keys().copied().collect())
            .unwrap_or_default()
    }

    pub fn surface_mut(&mut self, surface: WaylandSurfaceId) -> Option<&mut SurfaceState> {
        let owner = *self.surface_owner.get(&surface)?;
        self.clients.get_mut(&owner)?.surfaces.get_mut(&surface)
    }

    pub fn destroy_surface(
        &mut self,
        client: ClientId,
        surface: WaylandSurfaceId,
    ) -> Result<SurfaceState, WaylandWorldError> {
        if self.surface_owner.get(&surface).copied() != Some(client) {
            return Err(WaylandWorldError::SurfaceOwnershipMismatch);
        }
        let state = self
            .clients
            .get_mut(&client)
            .ok_or(WaylandWorldError::UnknownClient)?
            .surfaces
            .remove(&surface)
            .ok_or(WaylandWorldError::UnknownSurface)?;
        self.surface_owner.remove(&surface);
        Ok(state)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaylandWorldError {
    InvalidLimits,
    DuplicateClient,
    UnknownClient,
    DuplicateSurface,
    UnknownSurface,
    SurfaceOwnershipMismatch,
    SurfaceLimitExceeded,
}

impl fmt::Display for WaylandWorldError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Wayland world rejected an operation: {self:?}")
    }
}

impl std::error::Error for WaylandWorldError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(raw: u32) -> ClientId {
        ClientId::from_raw(raw).unwrap()
    }

    fn surface(raw: u32) -> WaylandSurfaceId {
        WaylandSurfaceId::from_raw(raw).unwrap()
    }

    #[test]
    fn surface_identity_is_unique_across_clients() {
        let mut world = WaylandWorld::default();
        world.add_client(client(1)).unwrap();
        world.add_client(client(2)).unwrap();
        world.create_surface(client(1), surface(10)).unwrap();
        assert_eq!(
            world.create_surface(client(2), surface(10)).unwrap_err(),
            WaylandWorldError::DuplicateSurface
        );
    }

    #[test]
    fn client_removal_atomically_removes_owned_surfaces() {
        let mut world = WaylandWorld::default();
        world.add_client(client(1)).unwrap();
        world.create_surface(client(1), surface(10)).unwrap();
        world.create_surface(client(1), surface(11)).unwrap();
        assert_eq!(world.remove_client(client(1)).unwrap(), 2);
        assert!(world.surface(surface(10)).is_none());
        assert!(world.surface(surface(11)).is_none());
    }
}

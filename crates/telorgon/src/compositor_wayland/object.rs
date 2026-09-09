use std::collections::BTreeMap;
use std::fmt;

use crate::compositor_wayland::{ClientId, ProtocolObjectId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProtocolObjectKind {
    Display,
    Registry,
    Callback,
    Compositor,
    Surface,
    Region,
    Shm,
    ShmPool,
    Buffer,
    Subcompositor,
    Subsurface,
    Output,
    Seat,
    Pointer,
    Keyboard,
    Touch,
    DataDeviceManager,
    DataDevice,
    DataSource,
    DataOffer,
    XdgWmBase,
    XdgPositioner,
    XdgSurface,
    XdgToplevel,
    XdgPopup,
    XwaylandShell,
    XwaylandSurface,
    DecorationManager,
    ToplevelDecoration,
    Viewporter,
    Viewport,
    Presentation,
    PresentationFeedback,
    LinuxDmaBuf,
    LinuxBufferParams,
    LinuxDmaBufFeedback,
    ExplicitSynchronization,
    SurfaceSynchronization,
    LinuxBufferRelease,
    CursorShapeManager,
    CursorShapeDevice,
    ToplevelIconManager,
    ToplevelIcon,
    FractionalScaleManager,
    FractionalScale,
    RelativePointerManager,
    RelativePointer,
    PointerConstraints,
    LockedPointer,
    ConfinedPointer,
    XdgOutputManager,
    XdgOutput,
    XwaylandKeyboardGrabManager,
    XwaylandKeyboardGrab,
    ShortcutInhibitManager,
    ShortcutInhibitor,
    IdleInhibitManager,
    IdleInhibitor,
    Activation,
    ActivationToken,
    SessionLockManager,
    SessionLock,
    SessionLockSurface,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjectMetadata {
    pub owner: ClientId,
    pub kind: ProtocolObjectKind,
    pub version: u32,
}

#[derive(Debug)]
pub struct ObjectRegistry {
    objects: BTreeMap<ProtocolObjectId, ObjectMetadata>,
    per_client: BTreeMap<ClientId, usize>,
    maximum_per_client: usize,
}

impl ObjectRegistry {
    pub fn new(maximum_per_client: usize) -> Result<Self, ObjectRegistryError> {
        if maximum_per_client == 0 {
            return Err(ObjectRegistryError::InvalidLimit);
        }
        Ok(Self {
            objects: BTreeMap::new(),
            per_client: BTreeMap::new(),
            maximum_per_client,
        })
    }

    pub fn insert(
        &mut self,
        id: ProtocolObjectId,
        metadata: ObjectMetadata,
    ) -> Result<(), ObjectRegistryError> {
        if metadata.version == 0 {
            return Err(ObjectRegistryError::InvalidVersion);
        }
        if self.objects.contains_key(&id) {
            return Err(ObjectRegistryError::DuplicateObject);
        }
        let count = self.per_client.get(&metadata.owner).copied().unwrap_or(0);
        if count >= self.maximum_per_client {
            return Err(ObjectRegistryError::ClientLimitExceeded);
        }
        self.objects.insert(id, metadata);
        self.per_client.insert(metadata.owner, count + 1);
        Ok(())
    }

    pub fn get(&self, id: ProtocolObjectId) -> Option<ObjectMetadata> {
        self.objects.get(&id).copied()
    }

    pub fn require(
        &self,
        owner: ClientId,
        id: ProtocolObjectId,
        kind: ProtocolObjectKind,
    ) -> Result<ObjectMetadata, ObjectRegistryError> {
        let object = self
            .objects
            .get(&id)
            .copied()
            .ok_or(ObjectRegistryError::UnknownObject)?;
        if object.owner != owner {
            return Err(ObjectRegistryError::OwnershipMismatch);
        }
        if object.kind != kind {
            return Err(ObjectRegistryError::KindMismatch);
        }
        Ok(object)
    }

    pub fn remove(
        &mut self,
        owner: ClientId,
        id: ProtocolObjectId,
    ) -> Result<ObjectMetadata, ObjectRegistryError> {
        let object = self
            .objects
            .get(&id)
            .copied()
            .ok_or(ObjectRegistryError::UnknownObject)?;
        if object.owner != owner {
            return Err(ObjectRegistryError::OwnershipMismatch);
        }
        self.objects.remove(&id);
        decrement(&mut self.per_client, owner);
        Ok(object)
    }

    pub fn remove_client(&mut self, owner: ClientId) -> usize {
        let before = self.objects.len();
        self.objects.retain(|_, object| object.owner != owner);
        self.per_client.remove(&owner);
        before - self.objects.len()
    }

    pub fn client_len(&self, owner: ClientId) -> usize {
        self.per_client.get(&owner).copied().unwrap_or(0)
    }
}

fn decrement(counts: &mut BTreeMap<ClientId, usize>, client: ClientId) {
    let Some(count) = counts.get_mut(&client) else {
        return;
    };
    *count -= 1;
    if *count == 0 {
        counts.remove(&client);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectRegistryError {
    InvalidLimit,
    InvalidVersion,
    DuplicateObject,
    UnknownObject,
    OwnershipMismatch,
    KindMismatch,
    ClientLimitExceeded,
}

impl fmt::Display for ObjectRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Wayland object registry rejected an operation: {self:?}"
        )
    }
}

impl std::error::Error for ObjectRegistryError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(raw: u32) -> ClientId {
        ClientId::from_raw(raw).unwrap()
    }

    fn object(raw: u32) -> ProtocolObjectId {
        ProtocolObjectId::from_raw(raw).unwrap()
    }

    #[test]
    fn ownership_and_client_limits_are_enforced() {
        let mut objects = ObjectRegistry::new(1).unwrap();
        objects
            .insert(
                object(2),
                ObjectMetadata {
                    owner: client(1),
                    kind: ProtocolObjectKind::Surface,
                    version: 6,
                },
            )
            .unwrap();
        assert_eq!(
            objects.insert(
                object(3),
                ObjectMetadata {
                    owner: client(1),
                    kind: ProtocolObjectKind::Region,
                    version: 1,
                }
            ),
            Err(ObjectRegistryError::ClientLimitExceeded)
        );
        assert_eq!(
            objects.remove(client(2), object(2)),
            Err(ObjectRegistryError::OwnershipMismatch)
        );
    }
}

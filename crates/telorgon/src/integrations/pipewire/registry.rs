use super::{MediaError, ParameterValue};
use std::collections::BTreeMap;

/// Connection-scoped identity, including a local incarnation for recycled global IDs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectHandle {
    pub(crate) epoch: u64,
    pub(crate) incarnation: u64,
    pub(crate) id: u32,
}
impl ObjectHandle {
    /// Informational native ID; use the complete handle in requests.
    pub fn id(self) -> u32 {
        self.id
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectKind {
    Device,
    Node,
    Port,
    Link,
    Metadata,
}
#[derive(Clone, Debug, PartialEq)]
pub struct RegistryObject {
    pub handle: ObjectHandle,
    pub kind: ObjectKind,
    /// PipeWire permission bits, advisory until the server accepts a request.
    pub permissions: u32,
    pub properties: BTreeMap<String, String>,
    /// Advanced SPA parameters keyed by (parameter type, enumeration index).
    pub parameters: BTreeMap<(u32, u32), ParameterValue>,
    pub readable_parameters: Vec<u32>,
    pub writable_parameters: Vec<u32>,
    pub metadata: BTreeMap<(u32, String), String>,
}
impl RegistryObject {
    pub fn can_write(&self) -> bool {
        let required = (pipewire::permissions::PermissionFlags::W
            | pipewire::permissions::PermissionFlags::X)
            .bits();
        self.permissions & required == required
    }
    pub fn serial(&self) -> Option<u64> {
        self.properties.get("object.serial")?.parse().ok()
    }
    pub fn media_class(&self) -> Option<&str> {
        self.properties.get("media.class").map(String::as_str)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectionState {
    Connecting,
    Ready,
    Failed(MediaError),
    Stopping,
    Stopped,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConnectionDiagnostics {
    pub connection_failures: u64,
    pub protocol_errors: u64,
    pub event_overflows: u64,
    pub registry_overflows: u64,
}
#[derive(Clone, Debug)]
pub struct RegistrySnapshot {
    pub epoch: u64,
    pub revision: u64,
    pub state: ConnectionState,
    pub objects: BTreeMap<u32, RegistryObject>,
    pub diagnostics: ConnectionDiagnostics,
    pub server_version: Option<String>,
    /// True means only the portal-authorized subset can be discovered.
    pub restricted: bool,
}
impl RegistrySnapshot {
    pub fn resolve(&self, handle: ObjectHandle) -> Result<&RegistryObject, MediaError> {
        if handle.epoch != self.epoch {
            return Err(MediaError::StaleHandle);
        }
        self.objects
            .get(&handle.id)
            .filter(|o| o.handle == handle)
            .ok_or(MediaError::StaleHandle)
    }
    pub fn objects_of_kind(&self, kind: ObjectKind) -> impl Iterator<Item = &RegistryObject> {
        self.objects.values().filter(move |o| o.kind == kind)
    }
}

pub(crate) struct RegistryState {
    pub snapshot: RegistrySnapshot,
    next_incarnation: u64,
    limit: usize,
}
impl RegistryState {
    pub fn new(epoch: u64, restricted: bool, limit: usize) -> Self {
        Self {
            snapshot: RegistrySnapshot {
                epoch,
                revision: 0,
                state: ConnectionState::Connecting,
                objects: BTreeMap::new(),
                diagnostics: ConnectionDiagnostics::default(),
                server_version: None,
                restricted,
            },
            next_incarnation: 0,
            limit,
        }
    }
    pub fn insert(
        &mut self,
        id: u32,
        kind: ObjectKind,
        permissions: u32,
        properties: BTreeMap<String, String>,
    ) -> Result<ObjectHandle, MediaError> {
        if self.snapshot.objects.len() >= self.limit && !self.snapshot.objects.contains_key(&id) {
            self.snapshot.diagnostics.registry_overflows += 1;
            return Err(MediaError::ResourceLimit("registry objects"));
        }
        self.next_incarnation = self
            .next_incarnation
            .checked_add(1)
            .ok_or(MediaError::ResourceLimit("object generations"))?;
        let handle = ObjectHandle {
            epoch: self.snapshot.epoch,
            incarnation: self.next_incarnation,
            id,
        };
        self.snapshot.objects.insert(
            id,
            RegistryObject {
                handle,
                kind,
                permissions,
                properties,
                parameters: BTreeMap::new(),
                readable_parameters: Vec::new(),
                writable_parameters: Vec::new(),
                metadata: BTreeMap::new(),
            },
        );
        self.snapshot.revision += 1;
        Ok(handle)
    }
    pub fn remove(&mut self, id: u32) -> Option<ObjectHandle> {
        let removed = self.snapshot.objects.remove(&id)?;
        self.snapshot.revision += 1;
        Some(removed.handle)
    }
}

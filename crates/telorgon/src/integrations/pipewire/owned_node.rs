//! Control-thread identity for an application-created filter node. Never used in processing.
use super::*;
use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};
static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);
pub(crate) const OWNER_KEY: &str = "telorgon.owner";
pub(crate) struct OwnedNode {
    token: String,
    handle: Mutex<Option<ObjectHandle>>,
}
impl OwnedNode {
    pub fn new() -> Result<Self, MediaError> {
        let id = NEXT_OWNER
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| MediaError::ResourceLimit("native node owner identities"))?;
        Ok(Self {
            token: format!("{}.{}", std::process::id(), id),
            handle: Mutex::new(None),
        })
    }
    pub fn token(&self) -> &str {
        &self.token
    }
    /// Initial identification uses a unique process-local property, never a display name.
    /// Once pinned, only that incarnation can be returned. The property is not authority
    /// or a security credential; permissions still come from the connection/server.
    pub fn update(&self, snapshot: &RegistrySnapshot, native_id: u32) -> Result<(), MediaError> {
        let mut pinned = self.handle.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(handle) = *pinned {
            snapshot.resolve(handle)?;
            if native_id != handle.id() {
                return Err(MediaError::StaleHandle);
            }
        } else if let Some(object) = snapshot.objects.get(&native_id) {
            if object.kind == ObjectKind::Node
                && object
                    .properties
                    .get(OWNER_KEY)
                    .is_some_and(|token| token == &self.token)
            {
                *pinned = Some(object.handle);
            }
        }
        Ok(())
    }
    pub fn get(&self, connection: &ConnectionHandle) -> Option<ObjectHandle> {
        let handle = (*self.handle.lock().unwrap_or_else(|e| e.into_inner()))?;
        let snapshot = connection.snapshot();
        (snapshot.state == ConnectionState::Ready && snapshot.resolve(handle).is_ok())
            .then_some(handle)
    }
}

use super::{DataError, DataResult};
use std::{
    collections::BTreeSet,
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeOrigin {
    Registration,
    Update,
    Restore,
    Mixed,
}

/// Notifications coalesce: keys are unioned and revision is the newest committed revision.
/// Read a registry snapshot when several values must be observed together.
#[derive(Clone, Debug)]
pub struct ChangeBatch {
    pub revision: u64,
    pub keys: BTreeSet<String>,
    pub origin: ChangeOrigin,
}
#[derive(Default)]
pub(crate) struct Mailbox {
    pending: Mutex<Option<ChangeBatch>>,
    ready: Condvar,
}
impl Mailbox {
    pub fn publish(&self, change: &ChangeBatch) {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        match pending.as_mut() {
            Some(batch) => {
                batch.revision = batch.revision.max(change.revision);
                batch.keys.extend(change.keys.iter().cloned());
                if batch.origin != change.origin {
                    batch.origin = ChangeOrigin::Mixed;
                }
            }
            None => *pending = Some(change.clone()),
        }
        self.ready.notify_all();
    }
}

/// A bounded, coalescing mailbox; slow consumers do not allocate an unbounded event queue.
pub struct Subscription(pub(crate) Arc<Mailbox>);
impl Subscription {
    pub fn try_recv(&self) -> DataResult<Option<ChangeBatch>> {
        Ok(self
            .0
            .pending
            .lock()
            .map_err(|_| DataError::Poisoned)?
            .take())
    }
    pub fn recv_timeout(&self, timeout: Duration) -> DataResult<Option<ChangeBatch>> {
        let pending = self.0.pending.lock().map_err(|_| DataError::Poisoned)?;
        let (mut pending, _) = self
            .0
            .ready
            .wait_timeout_while(pending, timeout, |p| p.is_none())
            .map_err(|_| DataError::Poisoned)?;
        Ok(pending.take())
    }
}

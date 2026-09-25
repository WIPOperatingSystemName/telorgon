//! Mode-1 authorization never reaches disk. The trusted portal frontend owns the original
//! application's D-Bus lifetime and sender/token binding; backend calls expose only its own
//! unique bus name and app ID. Scope cached grants to that broker and clear on reconnect.
use super::{
    grant_worker,
    grants::random_id,
    restore::{RestoreData, RestoreSource},
};
use std::sync::{Arc, Mutex};

const LIMIT: usize = 128;
#[derive(Default)]
pub(super) struct Store {
    entries: Vec<(String, RestoreData)>,
}
impl Store {
    pub fn retain_owner(&mut self, owner: Option<&str>) {
        self.entries
            .retain(|(broker, _)| Some(broker.as_str()) == owner);
    }
    pub fn resolve(
        &self,
        owner: &str,
        hint: &RestoreData,
        app: &str,
        types: u32,
        multiple: bool,
    ) -> Option<RestoreData> {
        (hint.valid_for(app, types, multiple)
            && self
                .entries
                .iter()
                .any(|(broker, grant)| broker == owner && grant == hint))
        .then(|| hint.clone())
    }
    pub fn revoke(&mut self, app: &str, id: Option<&str>) -> bool {
        let before = self.entries.len();
        self.entries
            .retain(|(_, grant)| grant.app_id != app || id.is_some_and(|id| id != grant.grant_id));
        self.entries.len() != before
    }
    pub fn prepare(
        store: &Arc<Mutex<Self>>,
        owner: String,
        app: String,
        sources: Vec<RestoreSource>,
    ) -> Result<Delivery, u32> {
        let mut record = RestoreData {
            app_id: app,
            grant_id: "0".repeat(64),
            sources,
        };
        if owner.is_empty() || owner.len() > 255 || !record.valid_for(&record.app_id, 7, true) {
            return Err(2);
        }
        let mut locked = store.lock().unwrap_or_else(|e| e.into_inner());
        if locked.entries.len() == LIMIT {
            return Err(2);
        }
        record.grant_id = random_id().map_err(|_| 2u32)?;
        if locked
            .entries
            .iter()
            .any(|(_, grant)| grant.grant_id == record.grant_id)
        {
            return Err(2);
        }
        locked.entries.push((owner, record.clone()));
        Ok(Delivery::Transient {
            record: Some(record),
            store: store.clone(),
        })
    }
}

/// Both kinds retain cancellation ownership until the final synchronous response commit.
pub(super) enum Delivery {
    Durable(Option<grant_worker::Delivery>),
    Transient {
        record: Option<RestoreData>,
        store: Arc<Mutex<Store>>,
    },
}
impl Delivery {
    pub fn record(&self) -> Option<&RestoreData> {
        match self {
            Self::Durable(grant) => grant.as_ref().and_then(|grant| grant.record.as_ref()),
            Self::Transient { record, .. } => record.as_ref(),
        }
    }
    pub fn claim(mut self) -> RestoreData {
        match &mut self {
            Self::Durable(grant) => grant.take().unwrap().claim(),
            Self::Transient { record, .. } => record.take().unwrap(),
        }
    }
}
impl Drop for Delivery {
    fn drop(&mut self) {
        if let Self::Transient {
            record: Some(record),
            store,
        } = self
        {
            store
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .revoke(&record.app_id, Some(&record.grant_id));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sources() -> Vec<RestoreSource> {
        vec![RestoreSource {
            kind: 2,
            identity: "window:epoch:1".into(),
        }]
    }
    #[test]
    fn cancellation_revocation_and_broker_lifetime() {
        let store = Arc::new(Mutex::new(Store::default()));
        let pending = Store::prepare(&store, ":1.2".into(), "app".into(), sources()).unwrap();
        let abandoned = pending.record().unwrap().clone();
        drop(pending);
        assert!(
            store
                .lock()
                .unwrap()
                .resolve(":1.2", &abandoned, "app", 2, false)
                .is_none()
        );
        let record = Store::prepare(&store, ":1.2".into(), "app".into(), sources())
            .unwrap()
            .claim();
        let other = Store::prepare(&store, ":1.2".into(), "other".into(), sources())
            .unwrap()
            .claim();
        let mut store = store.lock().unwrap();
        assert_eq!(
            store.resolve(":1.2", &record, "app", 2, false),
            Some(record.clone())
        );
        for (owner, app, types) in [(":1.3", "app", 2), (":1.2", "other", 2), (":1.2", "app", 1)] {
            assert!(store.resolve(owner, &record, app, types, false).is_none());
        }
        let mut forged = record.clone();
        forged.sources[0].identity = "other window".into();
        assert!(store.resolve(":1.2", &forged, "app", 2, false).is_none());
        assert!(store.revoke("app", Some(&record.grant_id)));
        assert!(store.resolve(":1.2", &record, "app", 2, false).is_none());
        assert!(store.resolve(":1.2", &other, "other", 2, false).is_some());
        store.retain_owner(Some(":1.3"));
        assert!(store.resolve(":1.2", &other, "other", 2, false).is_none());
    }
    #[test]
    fn bounded_without_eviction_and_cleared_on_reconnection() {
        let store = Arc::new(Mutex::new(Store::default()));
        let first = Store::prepare(&store, ":1.2".into(), "app".into(), sources())
            .unwrap()
            .claim();
        for _ in 1..LIMIT {
            Store::prepare(&store, ":1.2".into(), "app".into(), sources())
                .unwrap()
                .claim();
        }
        assert!(Store::prepare(&store, ":1.2".into(), "app".into(), sources()).is_err());
        assert!(
            store
                .lock()
                .unwrap()
                .resolve(":1.2", &first, "app", 2, false)
                .is_some()
        );
        store.lock().unwrap().retain_owner(None);
        assert!(
            store
                .lock()
                .unwrap()
                .resolve(":1.2", &first, "app", 2, false)
                .is_none()
        );
        assert!(Store::prepare(&store, ":1.2".into(), "app".into(), sources()).is_ok());
    }
}

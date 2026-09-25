use super::{
    autosave::Worker,
    subscription::Mailbox,
    value::{Erased, Typed},
    *,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, Weak},
    time::Instant,
};

#[derive(Clone)]
pub(crate) struct Entry {
    pub value: Arc<dyn Erased>,
    pub default: Arc<dyn Erased>,
}

pub(crate) struct State {
    pub entries: BTreeMap<String, Entry>,
    pub unknown: BTreeMap<String, toml::Value>,
    pub revision: u64,
    pub dirty_revision: u64,
    pub saved_revision: u64,
    pub first_change: Option<Instant>,
    pub last_change: Option<Instant>,
    pub subscribers: Vec<Weak<Mailbox>>,
    pub last_error: Option<String>,
    pub saving: bool,
    pub stop: bool,
    pub autosave_path: Option<std::path::PathBuf>,
}
impl State {
    pub fn dirty(&self) -> bool {
        self.dirty_revision > self.saved_revision
    }
    pub fn changed(&mut self, keys: BTreeSet<String>, origin: ChangeOrigin, dirty: bool) {
        self.revision += 1;
        if dirty {
            self.dirty_revision = self.revision;
            let now = Instant::now();
            self.first_change.get_or_insert(now);
            self.last_change = Some(now);
        }
        let change = ChangeBatch {
            revision: self.revision,
            keys,
            origin,
        };
        // Mailboxes never run user code. Publish in commit order under the state lock.
        self.subscribers.retain(|weak| {
            if let Some(mailbox) = weak.upgrade() {
                mailbox.publish(&change);
                true
            } else {
                false
            }
        });
    }
}

pub(crate) struct Core {
    pub name: String,
    pub version: u32,
    pub state: Mutex<State>,
    pub wake: Condvar,
    pub io: Mutex<()>,
}
impl Core {
    pub fn lock(&self) -> DataResult<MutexGuard<'_, State>> {
        self.state.lock().map_err(|_| DataError::Poisoned)
    }
}

/// Owns registrations and the optional autosave worker. Variables share storage, not worker ownership.
pub struct Registry {
    pub(crate) core: Arc<Core>,
    pub(crate) worker: Mutex<Option<Worker>>,
}
impl Registry {
    pub fn new(name: impl Into<String>, version: u32) -> Self {
        Self {
            core: Arc::new(Core {
                name: name.into(),
                version,
                state: Mutex::new(State {
                    entries: BTreeMap::new(),
                    unknown: BTreeMap::new(),
                    revision: 0,
                    dirty_revision: 0,
                    saved_revision: 0,
                    first_change: None,
                    last_change: None,
                    subscribers: Vec::new(),
                    last_error: None,
                    saving: false,
                    stop: false,
                    autosave_path: None,
                }),
                wake: Condvar::new(),
                io: Mutex::new(()),
            }),
            worker: Mutex::new(None),
        }
    }

    pub fn create<T: DataValue>(
        &self,
        key: impl Into<String>,
        default: T,
    ) -> DataResult<Variable<T>> {
        self.register(EntrySpec::new(key, default))
    }

    pub fn register<T: DataValue>(&self, spec: EntrySpec<T>) -> DataResult<Variable<T>> {
        let key = spec.key;
        if key.is_empty() || self.core.name.is_empty() {
            return Err(DataError::EmptyName);
        }
        let typed = Typed {
            value: spec.default,
            validator: spec.validator,
        };
        let default = typed.checked(&key, typed.value.clone())?;
        // Check serialization now, and decode it to enforce custom deserialization invariants.
        let encoded = default.encode(&key)?;
        default.decode(&key, encoded)?;
        let (revision, saved) = {
            let state = self.core.lock()?;
            if state.entries.contains_key(&key) {
                return Err(DataError::Duplicate(key));
            }
            (state.revision, state.unknown.get(&key).cloned())
        };
        let value = match &saved {
            Some(value) => default.decode(&key, value.clone())?,
            None => default.clone(),
        };
        let mut state = self.core.lock()?;
        if state.revision != revision {
            return Err(DataError::Conflict);
        }
        state.entries.insert(key.clone(), Entry { value, default });
        state.unknown.remove(&key);
        state.changed(
            BTreeSet::from([key.clone()]),
            ChangeOrigin::Registration,
            saved.is_none(),
        );
        drop(state);
        self.core.wake.notify_all();
        Ok(Variable::new(self.core.clone(), key))
    }

    pub fn get<T: DataValue>(&self, key: &str) -> DataResult<Variable<T>> {
        let state = self.core.lock()?;
        let entry = state
            .entries
            .get(key)
            .ok_or_else(|| DataError::Missing(key.into()))?;
        if !entry.value.any().is::<Typed<T>>() {
            return Err(DataError::TypeMismatch(key.into()));
        }
        Ok(Variable::new(self.core.clone(), key.into()))
    }

    pub fn subscribe(&self) -> DataResult<Subscription> {
        let mailbox = Arc::new(Mailbox::default());
        self.core.lock()?.subscribers.push(Arc::downgrade(&mailbox));
        Ok(Subscription(mailbox))
    }

    /// Read several values from one immutable snapshot; the closure holds no registry locks.
    pub fn read<R>(&self, read: impl FnOnce(&Transaction) -> DataResult<R>) -> DataResult<R> {
        read(&Transaction::snapshot(self.core.clone())?)
    }

    /// Optimistic transaction. Concurrent commits return Conflict; user closures are never retried.
    pub fn transaction<R>(
        &self,
        update: impl FnOnce(&mut Transaction) -> DataResult<R>,
    ) -> DataResult<R> {
        let mut transaction = Transaction::snapshot(self.core.clone())?;
        let result = update(&mut transaction)?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn reset_all(&self) -> DataResult<()> {
        self.transaction(|tx| {
            for (key, entry) in &mut tx.entries {
                entry.value = entry.default.clone();
                tx.changed.insert(key.clone());
            }
            Ok(())
        })
    }
}

/// Immutable starting snapshot plus staged replacements, committed by Registry::transaction.
pub struct Transaction {
    core: Arc<Core>,
    revision: u64,
    entries: BTreeMap<String, Entry>,
    changed: BTreeSet<String>,
}
impl Transaction {
    fn snapshot(core: Arc<Core>) -> DataResult<Self> {
        let state = core.lock()?;
        let transaction = Self {
            core: core.clone(),
            revision: state.revision,
            entries: state.entries.clone(),
            changed: BTreeSet::new(),
        };
        Ok(transaction)
    }
    pub fn get<T: DataValue>(&self, key: &str) -> DataResult<T> {
        let entry = self
            .entries
            .get(key)
            .ok_or_else(|| DataError::Missing(key.into()))?;
        let typed = entry
            .value
            .any()
            .downcast_ref::<Typed<T>>()
            .ok_or_else(|| DataError::TypeMismatch(key.into()))?;
        Ok(typed.value.clone())
    }
    pub fn set<T: DataValue>(&mut self, variable: &Variable<T>, value: T) -> DataResult<()> {
        if !Arc::ptr_eq(&self.core, &variable.core) {
            return Err(DataError::WrongRegistry);
        }
        let entry = self
            .entries
            .get_mut(&variable.key)
            .ok_or_else(|| DataError::Missing(variable.key.clone()))?;
        let typed = entry
            .value
            .any()
            .downcast_ref::<Typed<T>>()
            .ok_or_else(|| DataError::TypeMismatch(variable.key.clone()))?;
        let candidate = typed.checked(&variable.key, value)?;
        // Round-trip decoding checks intrinsic invariants in custom Deserialize implementations.
        candidate.decode(&variable.key, candidate.encode(&variable.key)?)?;
        entry.value = candidate;
        self.changed.insert(variable.key.clone());
        Ok(())
    }
    fn commit(self) -> DataResult<()> {
        if self.changed.is_empty() {
            return Ok(());
        }
        let mut state = self.core.lock()?;
        if state.revision != self.revision {
            return Err(DataError::Conflict);
        }
        for key in &self.changed {
            state.entries.insert(key.clone(), self.entries[key].clone());
        }
        state.changed(self.changed, ChangeOrigin::Update, true);
        drop(state);
        self.core.wake.notify_all();
        Ok(())
    }
}

static GLOBAL: OnceLock<Registry> = OnceLock::new();
pub struct Settings;
impl Settings {
    pub fn install_global(registry: Registry) -> DataResult<()> {
        GLOBAL
            .set(registry)
            .map_err(|_| DataError::AlreadyInitialized)
    }
    pub fn global() -> DataResult<&'static Registry> {
        GLOBAL.get().ok_or(DataError::NotInitialized)
    }
}

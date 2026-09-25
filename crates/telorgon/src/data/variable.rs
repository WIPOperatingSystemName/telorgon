use super::{registry::Core, value::Typed, *};
use std::{collections::BTreeSet, marker::PhantomData, sync::Arc};

pub struct Variable<T: DataValue> {
    pub(crate) core: Arc<Core>,
    pub(crate) key: String,
    marker: PhantomData<fn() -> T>,
}
impl<T: DataValue> Clone for Variable<T> {
    fn clone(&self) -> Self {
        Self::new(self.core.clone(), self.key.clone())
    }
}
impl<T: DataValue> Variable<T> {
    pub(crate) fn new(core: Arc<Core>, key: String) -> Self {
        Self {
            core,
            key,
            marker: PhantomData,
        }
    }
    pub fn key(&self) -> &str {
        &self.key
    }
    pub fn get(&self) -> DataResult<T> {
        self.with(Clone::clone)
    }

    /// The immutable snapshot remains alive during the closure; no registry lock is held.
    pub fn with<R>(&self, read: impl FnOnce(&T) -> R) -> DataResult<R> {
        let value = self
            .core
            .lock()?
            .entries
            .get(&self.key)
            .ok_or_else(|| DataError::Missing(self.key.clone()))?
            .value
            .clone();
        let typed = value
            .any()
            .downcast_ref::<Typed<T>>()
            .ok_or_else(|| DataError::TypeMismatch(self.key.clone()))?;
        Ok(read(&typed.value))
    }

    pub fn set(&self, value: T) -> DataResult<()> {
        self.replace(|_| value, false)
    }

    /// Does not replay the closure if another writer wins; returns Conflict instead.
    pub fn update(&self, update: impl FnOnce(&mut T)) -> DataResult<()> {
        self.replace(
            |mut value| {
                update(&mut value);
                value
            },
            true,
        )
    }
    fn replace(&self, update: impl FnOnce(T) -> T, check_conflict: bool) -> DataResult<()> {
        let original = self
            .core
            .lock()?
            .entries
            .get(&self.key)
            .ok_or_else(|| DataError::Missing(self.key.clone()))?
            .value
            .clone();
        let typed = original
            .any()
            .downcast_ref::<Typed<T>>()
            .ok_or_else(|| DataError::TypeMismatch(self.key.clone()))?;
        let candidate = typed.checked(&self.key, update(typed.value.clone()))?;
        candidate.decode(&self.key, candidate.encode(&self.key)?)?;
        let mut state = self.core.lock()?;
        let entry = state
            .entries
            .get_mut(&self.key)
            .ok_or_else(|| DataError::Missing(self.key.clone()))?;
        if check_conflict && !Arc::ptr_eq(&original, &entry.value) {
            return Err(DataError::Conflict);
        }
        entry.value = candidate;
        state.changed(
            BTreeSet::from([self.key.clone()]),
            ChangeOrigin::Update,
            true,
        );
        drop(state);
        self.core.wake.notify_all();
        Ok(())
    }
    pub fn reset(&self) -> DataResult<()> {
        let mut state = self.core.lock()?;
        let entry = state
            .entries
            .get_mut(&self.key)
            .ok_or_else(|| DataError::Missing(self.key.clone()))?;
        entry.value = entry.default.clone();
        state.changed(
            BTreeSet::from([self.key.clone()]),
            ChangeOrigin::Update,
            true,
        );
        drop(state);
        self.core.wake.notify_all();
        Ok(())
    }
}

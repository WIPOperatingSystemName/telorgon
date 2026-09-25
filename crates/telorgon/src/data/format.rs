use super::{registry::Core, *};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    registry: String,
    version: u32,
    values: BTreeMap<String, toml::Value>,
}

impl Core {
    pub(crate) fn serialize(&self) -> DataResult<(u64, String)> {
        if self.name.is_empty() {
            return Err(DataError::EmptyName);
        }
        let (revision, entries, mut values) = {
            let state = self.lock()?;
            (state.revision, state.entries.clone(), state.unknown.clone())
        };
        for (key, entry) in entries {
            values.insert(key.clone(), entry.value.encode(&key)?);
        }
        let document = Document {
            registry: self.name.clone(),
            version: self.version,
            values,
        };
        let text =
            toml::to_string_pretty(&document).map_err(|e| DataError::Format(e.to_string()))?;
        Ok((revision, text))
    }
}

impl Registry {
    pub fn to_toml(&self) -> DataResult<String> {
        Ok(self.core.serialize()?.1)
    }

    /// Restore is a new clean baseline, not a user edit. It does not trigger autosave.
    /// Missing entries keep current values. Concurrent edits cause Conflict, never silent overwrite.
    pub fn restore_toml(&self, source: &str) -> DataResult<()> {
        let _io = self.core.io.lock().map_err(|_| DataError::Poisoned)?;
        self.restore_source(source)
    }

    fn restore_source(&self, source: &str) -> DataResult<()> {
        let document: Document =
            toml::from_str(source).map_err(|e| DataError::Format(e.to_string()))?;
        if document.registry != self.core.name || document.version != self.core.version {
            return Err(DataError::SchemaMismatch);
        }
        let (revision, mut entries) = {
            let state = self.core.lock()?;
            (state.revision, state.entries.clone())
        };
        let mut unknown = BTreeMap::new();
        let mut changed = BTreeSet::new();
        for (key, value) in document.values {
            if let Some(entry) = entries.get_mut(&key) {
                entry.value = entry.value.decode(&key, value)?;
                changed.insert(key);
            } else {
                unknown.insert(key, value);
            }
        }
        let mut state = self.core.lock()?;
        if state.revision != revision {
            return Err(DataError::Conflict);
        }
        state.entries = entries;
        state.unknown = unknown;
        state.changed(changed, ChangeOrigin::Restore, false);
        state.saved_revision = state.revision;
        state.first_change = None;
        state.last_change = None;
        state.last_error = None;
        drop(state);
        self.core.wake.notify_all();
        Ok(())
    }

    pub fn load(&self, path: impl AsRef<Path>) -> DataResult<()> {
        self.load_file(path.as_ref(), false).map(|_| ())
    }
    /// Returns false only when the file does not exist; other errors remain visible.
    pub fn load_if_exists(&self, path: impl AsRef<Path>) -> DataResult<bool> {
        self.load_file(path.as_ref(), true)
    }
    fn load_file(&self, path: &Path, optional: bool) -> DataResult<bool> {
        let _io = self.core.io.lock().map_err(|_| DataError::Poisoned)?;
        let source = match std::fs::read_to_string(path) {
            Ok(source) => source,
            Err(e) if optional && e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(source) => {
                return Err(DataError::Io {
                    path: path.into(),
                    source,
                });
            }
        };
        self.restore_source(&source)?;
        Ok(true)
    }
}

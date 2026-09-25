use super::{DataError, DataResult};
use serde::{Serialize, de::DeserializeOwned};
use std::{any::Any, sync::Arc};

/// Serialization must round-trip the value in TOML. Unsupported representations fail explicitly.
/// Clone is required for isolated updates; Send + Sync permit global and worker-thread access.
pub trait DataValue: Serialize + DeserializeOwned + Clone + Send + Sync + 'static {}
impl<T: Serialize + DeserializeOwned + Clone + Send + Sync + 'static> DataValue for T {}

type Validator<T> = Arc<dyn Fn(&T) -> Result<(), String> + Send + Sync>;

pub struct EntrySpec<T: DataValue> {
    pub(crate) key: String,
    pub(crate) default: T,
    pub(crate) validator: Validator<T>,
}
impl<T: DataValue> EntrySpec<T> {
    pub fn new(key: impl Into<String>, default: T) -> Self {
        Self {
            key: key.into(),
            default,
            validator: Arc::new(|_| Ok(())),
        }
    }

    /// Validators must be deterministic and free of side effects. They run without registry locks.
    pub fn validate(
        mut self,
        validate: impl Fn(&T) -> Result<(), String> + Send + Sync + 'static,
    ) -> Self {
        self.validator = Arc::new(validate);
        self
    }
}

pub(crate) trait Erased: Send + Sync {
    fn any(&self) -> &dyn Any;
    fn encode(&self, key: &str) -> DataResult<toml::Value>;
    fn decode(&self, key: &str, value: toml::Value) -> DataResult<Arc<dyn Erased>>;
}

pub(crate) struct Typed<T: DataValue> {
    pub value: T,
    pub validator: Validator<T>,
}
impl<T: DataValue> Typed<T> {
    pub fn checked(&self, key: &str, value: T) -> DataResult<Arc<dyn Erased>> {
        (self.validator)(&value).map_err(|message| DataError::Value {
            key: key.into(),
            message,
        })?;
        Ok(Arc::new(Self {
            value,
            validator: self.validator.clone(),
        }))
    }
}
impl<T: DataValue> Erased for Typed<T> {
    fn any(&self) -> &dyn Any {
        self
    }
    fn encode(&self, key: &str) -> DataResult<toml::Value> {
        toml::Value::try_from(&self.value).map_err(|e| DataError::Value {
            key: key.into(),
            message: e.to_string(),
        })
    }
    fn decode(&self, key: &str, value: toml::Value) -> DataResult<Arc<dyn Erased>> {
        let value = value.try_into::<T>().map_err(|e| DataError::Value {
            key: key.into(),
            message: e.to_string(),
        })?;
        self.checked(key, value)
    }
}

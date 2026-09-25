use super::DataValue;

/// Export a persistent description, never raw pointers or transient native handles.
pub trait SaveState {
    type Saved: DataValue;
    type Error;
    fn save_state(&self) -> Result<Self::Saved, Self::Error>;
}

/// Implement on the resource-owning service. Async services can expose their own async operation.
/// Restoring registry data alone never invokes this trait or opens a device.
pub trait RestoreState<S: DataValue> {
    type Resource;
    type Error;
    fn restore_state(&self, saved: &S) -> Result<Self::Resource, Self::Error>;
}

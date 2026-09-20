use crate::foundation::{ColorRgba8, RectI};

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RenderRequest {
    pub force: bool,
    pub load: TargetLoad,
    pub store: TargetStore,
    /// Optional target-space damage/render clip. This does not replace the target's viewport
    /// mapping in [`crate::graphics::render::RenderTargetInfo::region`].
    pub region: Option<RectI>,
}

impl Default for RenderRequest {
    fn default() -> Self {
        Self {
            force: false,
            load: TargetLoad::Preserve,
            store: TargetStore::Store,
            region: None,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TargetLoad {
    Preserve,
    Clear(ColorRgba8),
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum TargetStore {
    #[default]
    Store,
    Discard,
}

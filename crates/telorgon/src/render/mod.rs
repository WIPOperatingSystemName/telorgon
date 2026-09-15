//! Backend-neutral retained render scene and execution contract.

mod backend;
mod compiler;
mod error;
mod readback;
mod request;
mod rounded_clip;
mod scene;
mod stats;
mod target;

pub use crate::layout::{ClipId, SpatialId};
pub use crate::ui::{Border, ImageId, MaterialId, Outline, Shadow, ShadowList};
pub use backend::RenderBackend;
pub use compiler::{CompileStats, SceneCompiler};
pub use error::{RenderError, RenderErrorKind, RenderResult};
pub use readback::{ReadbackFormat, ReadbackImage, ReadbackRequest, RenderReadback};
pub use request::{RenderRequest, TargetLoad, TargetStore};
pub use rounded_clip::RoundedClip;
#[doc(hidden)]
pub use scene::apply_patches;
pub use scene::{
    BatchKey, BlendMode, BoxInstance, DamageRegion, DenseInstances, DirtyRanges, DrawItem,
    GlyphInstance, ImageAlphaMode, ImageColorEncoding, ImageInstance, ImagePixelFormat,
    ImageResource, ImageResourceDelta, ImageResourceUpdate, LiquidGlassMaterial, MaterialInstance,
    MaterialKind, MaterialResource, MaterialResourceDelta, PipelineKind, PrimitiveKind, RangePatch, RenderClip,
    RenderScene, RenderSceneDelta, RenderSpatialNode,
};
pub use stats::{RenderStats, SceneUpdateStats};
pub use target::{AlphaMode, ColorSpace, RenderTargetInfo};

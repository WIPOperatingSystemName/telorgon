//! Deterministic CPU reference renderer and headless framebuffer backend.

mod renderer;

#[cfg(any(target_os = "linux", test))]
pub(crate) use renderer::SoftwareCompositeLayer;
pub use renderer::{
    SoftwareFrameContext, SoftwareReadback, SoftwareRenderer, SoftwareScene, SoftwareSurface,
    SoftwareTarget,
};

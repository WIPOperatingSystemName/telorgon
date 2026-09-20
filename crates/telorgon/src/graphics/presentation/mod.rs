//! Renderer- and platform-neutral native-presentation contracts.
//!
//! Concrete backends retain strongly typed render targets and completion proofs. This crate only
//! defines the lifecycle and ownership vocabulary shared by application hosts and managed
//! presentation assemblies.

use std::fmt;

use crate::foundation::{SizeF, SizeI};
pub use crate::graphics::render::{AlphaMode, ColorSpace};
use thiserror::Error;

/// Monotonically increasing identity for one set of host-observed surface metrics.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SurfaceRevision(u64);

impl SurfaceRevision {
    pub const INITIAL: Self = Self(0);

    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    pub fn advance(&mut self) -> Result<Self, PresentationError> {
        self.0 = self.0.checked_add(1).ok_or_else(|| {
            PresentationError::new(
                PresentationErrorKind::InvalidState,
                "surface metrics revision exhausted",
            )
        })?;
        Ok(*self)
    }
}

impl From<u64> for SurfaceRevision {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<SurfaceRevision> for u64 {
    fn from(value: SurfaceRevision) -> Self {
        value.0
    }
}

/// A coherent logical and physical view of a native drawable.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SurfaceMetrics {
    pub revision: SurfaceRevision,
    pub logical_extent: SizeF,
    pub physical_extent: SizeI,
    pub scale_factor: f64,
    pub color_space: ColorSpace,
    pub alpha_mode: AlphaMode,
}

impl SurfaceMetrics {
    pub fn validate(self) -> Result<Self, PresentationError> {
        if !self.scale_factor.is_finite() || self.scale_factor <= 0.0 {
            return Err(PresentationError::new(
                PresentationErrorKind::InvalidState,
                "surface scale factor must be finite and positive",
            ));
        }
        if !self.logical_extent.width.is_finite()
            || !self.logical_extent.height.is_finite()
            || self.logical_extent.width < 0.0
            || self.logical_extent.height < 0.0
        {
            return Err(PresentationError::new(
                PresentationErrorKind::InvalidState,
                "logical surface extent must be finite and non-negative",
            ));
        }
        Ok(self)
    }

    pub const fn drawable(self) -> bool {
        self.physical_extent.width > 0 && self.physical_extent.height > 0
    }
}

/// Observable lifecycle of one native presentation session.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PresentationState {
    Unconfigured,
    Ready,
    NeedsReconfigure,
    Suspended,
    SurfaceLost,
    DeviceLost,
    Shutdown,
}

/// Backend-neutral lifecycle bookkeeping for one presentation resource generation.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PresentationRecovery {
    #[doc(hidden)]
    pub state: PresentationState,
    #[doc(hidden)]
    pub requested_extent: SizeI,
    #[doc(hidden)]
    pub generation: u64,
    #[doc(hidden)]
    pub retired_generations: u64,
}

impl PresentationRecovery {
    pub fn new(extent: SizeI) -> Self {
        Self {
            state: if is_zero_extent(extent) {
                PresentationState::Suspended
            } else {
                PresentationState::Unconfigured
            },
            requested_extent: extent,
            generation: 0,
            retired_generations: 0,
        }
    }

    pub const fn state(&self) -> PresentationState {
        self.state
    }

    pub const fn requested_extent(&self) -> SizeI {
        self.requested_extent
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }

    pub const fn retired_generations(&self) -> u64 {
        self.retired_generations
    }

    pub fn resize(&mut self, extent: SizeI) -> bool {
        let extent_changed = self.requested_extent != extent;
        if matches!(
            self.state,
            PresentationState::SurfaceLost
                | PresentationState::DeviceLost
                | PresentationState::Shutdown
        ) {
            self.requested_extent = extent;
            return extent_changed;
        }
        if self.requested_extent == extent
            && matches!(
                (self.state, is_zero_extent(extent)),
                (
                    PresentationState::Ready | PresentationState::NeedsReconfigure,
                    false
                ) | (PresentationState::Suspended, true)
            )
        {
            return false;
        }
        self.requested_extent = extent;
        self.state = if is_zero_extent(extent) {
            PresentationState::Suspended
        } else {
            PresentationState::NeedsReconfigure
        };
        true
    }

    pub fn mark_reconfigured(&mut self) -> PresentationResult<u64> {
        self.generation = self.generation.checked_add(1).ok_or_else(|| {
            PresentationError::new(
                PresentationErrorKind::InvalidState,
                "presentation resource generation exhausted",
            )
        })?;
        self.state = PresentationState::Ready;
        Ok(self.generation)
    }

    pub fn mark_retired(&mut self) {
        self.retired_generations = self.retired_generations.saturating_add(1);
    }
}

pub const fn is_zero_extent(extent: SizeI) -> bool {
    extent.width <= 0 || extent.height <= 0
}

/// Renderer-independent class of presentation failure.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PresentationErrorKind {
    Unsupported,
    SurfaceLost,
    DeviceLost,
    OutOfMemory,
    InvalidState,
    Native,
}

/// Presentation failure with an optional backend diagnostic code.
#[derive(Debug, Error)]
#[error("{context}")]
pub struct PresentationError {
    kind: PresentationErrorKind,
    context: String,
    backend_code: Option<i64>,
}

impl PresentationError {
    pub fn new(kind: PresentationErrorKind, context: impl Into<String>) -> Self {
        Self {
            kind,
            context: context.into(),
            backend_code: None,
        }
    }

    pub fn with_backend_code(
        kind: PresentationErrorKind,
        context: impl Into<String>,
        backend_code: i64,
    ) -> Self {
        Self {
            kind,
            context: context.into(),
            backend_code: Some(backend_code),
        }
    }

    pub const fn kind(&self) -> PresentationErrorKind {
        self.kind
    }

    pub const fn backend_code(&self) -> Option<i64> {
        self.backend_code
    }
}

pub type PresentationResult<T> = Result<T, PresentationError>;

/// Result of trying to acquire without blocking the native event loop.
#[derive(Debug)]
pub enum AcquireDisposition<F> {
    Ready(F),
    Suspended,
    NotReady,
    NeedsReconfigure,
}

/// Native disposition of a consumed frame.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PresentDisposition {
    Presented,
    PresentedSuboptimal,
    NeedsReconfigure,
    SurfaceLost,
}

/// The latest externally meaningful stage reached by a frame.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CompletionStage {
    Render,
    Transport,
    Present,
    Display,
}

/// Backend-neutral progress reported to the application host.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct PresentationProgress {
    pub reconfigure_pending: bool,
    pub maintenance_pending: bool,
    pub retired_generations: u64,
}

/// Capabilities relevant to orchestration without exposing native handles or API names.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct PresentationCapabilities {
    pub damage_tracking: bool,
    pub transport_completion: bool,
    pub present_completion: bool,
    pub display_completion: bool,
}

/// Stable identity attached to a linearly acquired frame.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct FrameIdentity {
    pub device_id: u64,
    pub frame_id: u64,
    pub surface_revision: SurfaceRevision,
    pub surface_generation: u64,
}

impl FrameIdentity {
    pub fn validate_for(
        self,
        device_id: u64,
        surface_revision: SurfaceRevision,
        surface_generation: u64,
    ) -> PresentationResult<()> {
        if self.device_id != device_id {
            return Err(PresentationError::new(
                PresentationErrorKind::InvalidState,
                "presentable frame belongs to another rendering device",
            ));
        }
        if self.surface_revision != surface_revision
            || self.surface_generation != surface_generation
        {
            return Err(PresentationError::new(
                PresentationErrorKind::InvalidState,
                "presentable frame belongs to a stale surface generation",
            ));
        }
        Ok(())
    }
}

/// One acquired frame which must be submitted or explicitly discarded exactly once.
///
/// The target is a generic associated type so Vulkan targets may borrow presenter-owned images
/// while software targets may borrow CPU memory. The trait consumes `self` for both terminal paths,
/// making accidental frame reuse impossible in safe Rust.
pub trait PresentableFrame<R>: Sized {
    type Target<'frame>
    where
        Self: 'frame;
    type Submission;
    type Receipt;

    fn identity(&self) -> FrameIdentity;
    fn target(&mut self) -> Self::Target<'_>;
    fn submit_and_present(
        self,
        renderer: &mut R,
        submission: Self::Submission,
    ) -> PresentationResult<Self::Receipt>;
    fn discard(self, renderer: &mut R) -> PresentationResult<()>;
}

/// Lifecycle contract implemented by a concrete, known-compatible managed assembly.
pub trait PresentationSession<R> {
    type Frame<'session>: PresentableFrame<R>
    where
        Self: 'session;

    fn state(&self) -> PresentationState;
    fn configure(&mut self, metrics: SurfaceMetrics) -> PresentationResult<()>;
    fn acquire(&mut self) -> PresentationResult<AcquireDisposition<Self::Frame<'_>>>;
    fn poll(&mut self) -> PresentationResult<PresentationProgress>;
    fn suspend(&mut self) -> PresentationResult<()>;
    fn shutdown(&mut self) -> PresentationResult<()>;
}

/// Opaque backend completion proof tagged with the semantic stage it demonstrates.
#[derive(Copy, Clone, PartialEq, Eq)]
pub struct CompletionProof<P> {
    stage: CompletionStage,
    proof: P,
}

impl<P> CompletionProof<P> {
    pub const fn new(stage: CompletionStage, proof: P) -> Self {
        Self { stage, proof }
    }

    pub const fn stage(&self) -> CompletionStage {
        self.stage
    }

    pub const fn proof(&self) -> &P {
        &self.proof
    }

    pub fn into_inner(self) -> P {
        self.proof
    }
}

impl<P: fmt::Debug> fmt::Debug for CompletionProof<P> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CompletionProof")
            .field("stage", &self.stage)
            .field("proof", &self.proof)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics(revision: u64, width: i32, height: i32) -> SurfaceMetrics {
        SurfaceMetrics {
            revision: revision.into(),
            logical_extent: SizeF {
                width: width.max(0) as f32,
                height: height.max(0) as f32,
            },
            physical_extent: SizeI { width, height },
            scale_factor: 1.0,
            color_space: ColorSpace::Srgb,
            alpha_mode: AlphaMode::Opaque,
        }
    }

    #[test]
    fn zero_extent_is_suspended_instead_of_fabricated() {
        assert!(!metrics(1, 0, 720).drawable());
        assert!(!metrics(2, 1280, 0).drawable());
        assert!(metrics(3, 1280, 720).drawable());
    }

    #[test]
    fn stale_frame_identity_is_rejected() {
        let identity = FrameIdentity {
            device_id: 7,
            frame_id: 11,
            surface_revision: 3.into(),
            surface_generation: 2,
        };
        assert!(identity.validate_for(7, 3.into(), 2).is_ok());
        assert_eq!(
            identity.validate_for(7, 4.into(), 2).unwrap_err().kind(),
            PresentationErrorKind::InvalidState
        );
        assert_eq!(
            identity.validate_for(8, 3.into(), 2).unwrap_err().kind(),
            PresentationErrorKind::InvalidState
        );
    }

    #[test]
    fn revisions_advance_monotonically() {
        let mut revision = SurfaceRevision::INITIAL;
        assert_eq!(revision.advance().unwrap().get(), 1);
        assert_eq!(revision.advance().unwrap().get(), 2);
    }

    #[test]
    fn completion_stages_do_not_collapse() {
        assert!(CompletionStage::Render < CompletionStage::Transport);
        assert!(CompletionStage::Transport < CompletionStage::Present);
        assert!(CompletionStage::Present < CompletionStage::Display);
    }
}

#[cfg(all(feature = "application-vulkan-windows", target_os = "windows"))]
pub mod dxgi;
#[cfg(feature = "application-software")]
pub mod softbuffer;
#[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
pub mod kms;
#[cfg(feature = "application-vulkan-windows")]
pub mod wsi;

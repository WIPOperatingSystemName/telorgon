//! Native `softbuffer` presentation for software-renderer framebuffers.

use std::num::NonZeroU32;
use std::sync::Arc;

use crate::foundation::{RectF, SizeI};
use crate::graphics::presentation::{
    PresentationError, PresentationErrorKind, PresentationRecovery, PresentationResult,
    PresentationState, SurfaceMetrics,
};
use crate::graphics::renderers::software::SoftwareSurface;
use softbuffer::{Context, Rect as SoftbufferRect, Surface};
use winit::event_loop::OwnedDisplayHandle;
use winit::window::Window;

/// Owns the native software surface and transfers completed CPU framebuffers into it.
pub struct SoftbufferPresenter {
    context: Context<OwnedDisplayHandle>,
    surface: Option<Surface<OwnedDisplayHandle, Arc<Window>>>,
    presented: bool,
    damage_scratch: Vec<SoftbufferRect>,
    recovery: PresentationRecovery,
}

impl SoftbufferPresenter {
    pub fn from_display(display: OwnedDisplayHandle) -> PresentationResult<Self> {
        let context = Context::new(display).map_err(|error| {
            PresentationError::new(
                PresentationErrorKind::Native,
                format!("failed to create native display context: {error}"),
            )
        })?;
        Ok(Self {
            context,
            surface: None,
            presented: false,
            damage_scratch: Vec::with_capacity(8),
            recovery: PresentationRecovery::new(SizeI::default()),
        })
    }

    pub fn attach(&mut self, window: Arc<Window>) -> PresentationResult<()> {
        if self.recovery.state() == PresentationState::Shutdown {
            return Err(PresentationError::new(
                PresentationErrorKind::InvalidState,
                "cannot attach a shut down software presenter",
            ));
        }
        self.surface = Some(Surface::new(&self.context, window).map_err(|error| {
            PresentationError::new(
                PresentationErrorKind::Native,
                format!("software surface creation failed: {error}"),
            )
        })?);
        self.presented = false;
        Ok(())
    }

    pub const fn has_presented(&self) -> bool {
        self.presented
    }

    pub fn is_attached(&self) -> bool {
        self.surface.is_some()
    }

    pub const fn state(&self) -> PresentationState {
        self.recovery.state()
    }

    pub fn configure(&mut self, metrics: SurfaceMetrics) -> PresentationResult<()> {
        if self.recovery.state() == PresentationState::Shutdown {
            return Err(PresentationError::new(
                PresentationErrorKind::InvalidState,
                "cannot configure a shut down software presenter",
            ));
        }
        let metrics = metrics.validate()?;
        self.recovery.resize(metrics.physical_extent);
        if metrics.drawable() {
            if self.recovery.state() != PresentationState::Ready {
                self.recovery.mark_reconfigured()?;
            }
        } else {
            self.recovery.state = PresentationState::Suspended;
        }
        Ok(())
    }

    pub fn present(&mut self, framebuffer: &SoftwareSurface) -> PresentationResult<()> {
        #[cfg(feature = "instrumentation")]
        let _span = crate::runtime::instrumentation::span!("surface.copy");
        let extent = framebuffer.framebuffer_extent();
        let pixels_rgba8 = framebuffer.pixels_rgba8();
        let damage = framebuffer.presented_damage();
        let width = NonZeroU32::new(extent.width.max(1) as u32).expect("clamped width is non-zero");
        let height =
            NonZeroU32::new(extent.height.max(1) as u32).expect("clamped height is non-zero");
        let expected_len = width.get() as usize * height.get() as usize * 4;
        if pixels_rgba8.len() < expected_len {
            return Err(PresentationError::new(
                PresentationErrorKind::InvalidState,
                "software framebuffer is smaller than its declared extent",
            ));
        }
        let (surface, damage_scratch) = match (&mut self.surface, &mut self.damage_scratch) {
            (Some(surface), damage_scratch) => (surface, damage_scratch),
            (None, _) => {
                return Err(PresentationError::new(
                    PresentationErrorKind::InvalidState,
                    "native software surface is unavailable",
                ));
            }
        };
        surface.resize(width, height).map_err(|error| {
            PresentationError::new(
                PresentationErrorKind::Native,
                format!("software surface resize failed: {error}"),
            )
        })?;
        let mut buffer = surface.buffer_mut().map_err(|error| {
            PresentationError::new(
                PresentationErrorKind::Native,
                format!("software surface buffer acquisition failed: {error}"),
            )
        })?;
        let needs_full_copy = !self.presented || damage.full || buffer.age() != 1;
        damage_scratch.clear();
        if needs_full_copy {
            damage_scratch.push(SoftbufferRect {
                x: 0,
                y: 0,
                width,
                height,
            });
        } else {
            damage_scratch.extend(
                damage
                    .rects
                    .iter()
                    .filter_map(|rect| softbuffer_damage_rect(*rect, extent)),
            );
        }
        for rect in damage_scratch.iter().copied() {
            copy_softbuffer_rect(&mut buffer, pixels_rgba8, width.get() as usize, rect);
        }
        if damage_scratch.is_empty() {
            return Ok(());
        }
        buffer
            .present_with_damage(damage_scratch)
            .map_err(|error| {
                PresentationError::new(
                    PresentationErrorKind::Native,
                    format!("software surface presentation failed: {error}"),
                )
            })?;
        self.presented = true;
        self.recovery.state = PresentationState::Ready;
        Ok(())
    }

    pub fn suspend(&mut self) {
        if self.recovery.state() == PresentationState::Shutdown {
            return;
        }
        self.surface = None;
        self.presented = false;
        self.recovery.state = PresentationState::Suspended;
    }

    pub fn shutdown(&mut self) {
        self.surface = None;
        self.presented = false;
        self.recovery.state = PresentationState::Shutdown;
    }
}

fn softbuffer_pixel(rgba: &[u8]) -> u32 {
    u32::from(rgba[2]) | (u32::from(rgba[1]) << 8) | (u32::from(rgba[0]) << 16)
}

fn softbuffer_damage_rect(rect: RectF, extent: SizeI) -> Option<SoftbufferRect> {
    let bounds = RectF {
        x: 0.0,
        y: 0.0,
        width: extent.width.max(1) as f32,
        height: extent.height.max(1) as f32,
    };
    let clipped = rect.intersection(bounds)?;
    let left = clipped.x.floor().max(0.0) as u32;
    let top = clipped.y.floor().max(0.0) as u32;
    let right = clipped.right().ceil().min(bounds.width) as u32;
    let bottom = clipped.bottom().ceil().min(bounds.height) as u32;
    Some(SoftbufferRect {
        x: left,
        y: top,
        width: NonZeroU32::new(right.saturating_sub(left))?,
        height: NonZeroU32::new(bottom.saturating_sub(top))?,
    })
}

fn copy_softbuffer_rect(
    target: &mut [u32],
    source_rgba8: &[u8],
    stride: usize,
    rect: SoftbufferRect,
) {
    let left = rect.x as usize;
    let top = rect.y as usize;
    let right = left + rect.width.get() as usize;
    let bottom = top + rect.height.get() as usize;
    for y in top..bottom {
        for x in left..right {
            let pixel = y * stride + x;
            target[pixel] = softbuffer_pixel(&source_rgba8[pixel * 4..pixel * 4 + 4]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_rgba_readback_to_softbuffer_rgb() {
        assert_eq!(softbuffer_pixel(&[0x12, 0x34, 0x56, 0x78]), 0x0012_3456);
    }

    #[test]
    fn clips_fractional_damage_to_the_surface() {
        let rect = softbuffer_damage_rect(
            RectF {
                x: -1.5,
                y: 3.2,
                width: 6.0,
                height: 8.0,
            },
            SizeI {
                width: 8,
                height: 8,
            },
        )
        .unwrap();
        assert_eq!((rect.x, rect.y), (0, 3));
        assert_eq!((rect.width.get(), rect.height.get()), (5, 5));
    }
}

//! Protocol-neutral screen capture values. These values confer no capture authority.

use super::{OutputId, WindowId};
use std::num::NonZeroU32;

/// A host-owned source. Window identity includes its incarnation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CaptureSource {
    Output(OutputId),
    /// An isolated host-owned display, distinct from a physical monitor.
    VirtualOutput(OutputId),
    Window(WindowId),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CaptureCursorMode {
    #[default]
    Hidden,
    Embedded,
    /// Cursor pixels are excluded from the image and delivered as stream metadata.
    Metadata,
}

/// Validated stream preferences. The host may negotiate a lower frame rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureOptions {
    cursor: CaptureCursorMode,
    max_frame_rate: NonZeroU32,
}

impl CaptureOptions {
    pub const fn new(cursor: CaptureCursorMode, max_frame_rate: NonZeroU32) -> Self {
        Self {
            cursor,
            max_frame_rate,
        }
    }

    pub const fn cursor(self) -> CaptureCursorMode {
        self.cursor
    }
    pub const fn max_frame_rate(self) -> NonZeroU32 {
        self.max_frame_rate
    }
}

impl Default for CaptureOptions {
    fn default() -> Self {
        Self::new(CaptureCursorMode::Hidden, NonZeroU32::new(30).unwrap())
    }
}

/// Validated packed-RGB frame layout. Byte accounting includes row padding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureLayout {
    width: NonZeroU32,
    height: NonZeroU32,
    stride: u32,
    byte_len: usize,
}

impl CaptureLayout {
    pub fn rgba8(width: NonZeroU32, height: NonZeroU32, stride: u32) -> Option<Self> {
        if stride < width.get().checked_mul(4)? || stride % 4 != 0 {
            return None;
        }
        let byte_len = usize::try_from(stride)
            .ok()?
            .checked_mul(usize::try_from(height.get()).ok()?)?;
        Some(Self {
            width,
            height,
            stride,
            byte_len,
        })
    }

    pub const fn width(self) -> u32 {
        self.width.get()
    }
    pub const fn height(self) -> u32 {
        self.height.get()
    }
    pub const fn stride(self) -> u32 {
        self.stride
    }
    pub const fn byte_len(self) -> usize {
        self.byte_len
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureStopReason {
    Requested,
    Denied,
    Revoked,
    SessionLocked,
    SourceUnavailable,
    RequesterDisconnected,
    StreamFailed,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn nz(value: u32) -> NonZeroU32 {
        NonZeroU32::new(value).unwrap()
    }

    #[test]
    fn padded_rows_are_accounted_and_invalid_layouts_rejected() {
        let layout = CaptureLayout::rgba8(nz(3), nz(2), 16).unwrap();
        assert_eq!(layout.byte_len(), 32);
        assert!(CaptureLayout::rgba8(nz(3), nz(2), 8).is_none());
        assert!(CaptureLayout::rgba8(nz(3), nz(2), 13).is_none());
        assert!(CaptureLayout::rgba8(nz(u32::MAX), nz(1), u32::MAX).is_none());
    }
}

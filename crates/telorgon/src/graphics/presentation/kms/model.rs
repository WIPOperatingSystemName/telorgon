use std::fmt;
use std::num::NonZeroU32;

use crate::foundation::SizeI;

const fn drm_fourcc(a: u8, b: u8, c: u8, d: u8) -> u32 {
    a as u32 | ((b as u32) << 8) | ((c as u32) << 16) | ((d as u32) << 24)
}

pub const DRM_FORMAT_ARGB8888: u32 = drm_fourcc(b'A', b'R', b'2', b'4');
pub const DRM_FORMAT_XRGB8888: u32 = drm_fourcc(b'X', b'R', b'2', b'4');
pub const DRM_FORMAT_MOD_LINEAR: u64 = 0;
pub const DRM_FORMAT_MOD_INVALID: u64 = 0x00ff_ffff_ffff_ffff;
pub const DRM_PLANE_TYPE_PRIMARY: u64 = 1;
pub const DRM_PLANE_TYPE_CURSOR: u64 = 2;

macro_rules! id {
    ($name:ident) => {
        #[repr(transparent)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(NonZeroU32);

        impl $name {
            pub const fn from_raw(value: u32) -> Option<Self> {
                match NonZeroU32::new(value) {
                    Some(value) => Some(Self(value)),
                    None => None,
                }
            }

            pub const fn get(self) -> u32 {
                self.0.get()
            }
        }
    };
}

id!(KmsConnectorId);
id!(KmsCrtcId);
id!(KmsPlaneId);
id!(KmsPropertyId);
id!(KmsFramebufferId);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScanoutFormat {
    pub fourcc: u32,
    pub modifier: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KmsMode {
    pub size: SizeI,
    pub refresh_millihertz: u32,
    pub blob_id: u32,
}

impl KmsMode {
    pub fn validate(self) -> Result<Self, FrameSlotError> {
        if self.size.width <= 0
            || self.size.height <= 0
            || self.refresh_millihertz == 0
            || self.blob_id == 0
        {
            Err(FrameSlotError::InvalidMode)
        } else {
            Ok(self)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AtomicProperty {
    pub object: u32,
    pub property: KmsPropertyId,
    pub value: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FrameSlotState {
    #[default]
    Available,
    Rendering,
    GpuSubmitted,
    ReadyForScanout,
    FlipQueued,
    ScanningOut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameSlot {
    pub index: usize,
    pub framebuffer: KmsFramebufferId,
    pub state: FrameSlotState,
    pub frame_id: u64,
}

impl FrameSlot {
    pub fn new(index: usize, framebuffer: KmsFramebufferId) -> Self {
        Self {
            index,
            framebuffer,
            state: FrameSlotState::Available,
            frame_id: 0,
        }
    }

    pub fn begin_render(&mut self, frame_id: u64) -> Result<(), FrameSlotError> {
        if self.state != FrameSlotState::Available || frame_id == 0 {
            return Err(FrameSlotError::InvalidTransition);
        }
        self.state = FrameSlotState::Rendering;
        self.frame_id = frame_id;
        Ok(())
    }

    pub fn gpu_submitted(&mut self) -> Result<(), FrameSlotError> {
        self.transition(FrameSlotState::Rendering, FrameSlotState::GpuSubmitted)
    }

    pub fn gpu_completed(&mut self) -> Result<(), FrameSlotError> {
        self.transition(
            FrameSlotState::GpuSubmitted,
            FrameSlotState::ReadyForScanout,
        )
    }

    pub fn page_flip_submitted(&mut self) -> Result<(), FrameSlotError> {
        self.transition(FrameSlotState::ReadyForScanout, FrameSlotState::FlipQueued)
    }

    pub fn page_flip_completed(&mut self) -> Result<(), FrameSlotError> {
        self.transition(FrameSlotState::FlipQueued, FrameSlotState::ScanningOut)
    }

    pub fn page_flip_replaced(&mut self) -> Result<(), FrameSlotError> {
        self.transition(FrameSlotState::ScanningOut, FrameSlotState::Available)
    }

    pub fn cancel_render(&mut self) -> Result<(), FrameSlotError> {
        self.transition(FrameSlotState::Rendering, FrameSlotState::Available)
    }

    pub fn discard_ready(&mut self) -> Result<(), FrameSlotError> {
        self.transition(FrameSlotState::ReadyForScanout, FrameSlotState::Available)
    }

    fn transition(
        &mut self,
        expected: FrameSlotState,
        next: FrameSlotState,
    ) -> Result<(), FrameSlotError> {
        if self.state != expected {
            return Err(FrameSlotError::InvalidTransition);
        }
        self.state = next;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameSlotError {
    InvalidMode,
    InvalidTransition,
}

impl fmt::Display for FrameSlotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "KMS frame slot operation failed: {self:?}")
    }
}

impl std::error::Error for FrameSlotError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scanout_buffer_reuse_requires_page_flip_retirement() {
        let mut slot = FrameSlot::new(0, KmsFramebufferId::from_raw(5).unwrap());
        slot.begin_render(1).unwrap();
        slot.gpu_submitted().unwrap();
        slot.gpu_completed().unwrap();
        slot.page_flip_submitted().unwrap();
        assert_eq!(slot.begin_render(2), Err(FrameSlotError::InvalidTransition));
        slot.page_flip_completed().unwrap();
        assert_eq!(slot.begin_render(2), Err(FrameSlotError::InvalidTransition));
        slot.page_flip_replaced().unwrap();
        slot.begin_render(2).unwrap();
    }

    #[test]
    fn completed_but_unpresented_frame_can_be_discarded_for_mailbox_scheduling() {
        let mut slot = FrameSlot::new(1, KmsFramebufferId::from_raw(6).unwrap());
        slot.begin_render(1).unwrap();
        slot.gpu_submitted().unwrap();
        slot.gpu_completed().unwrap();
        slot.discard_ready().unwrap();
        slot.begin_render(2).unwrap();
    }
}

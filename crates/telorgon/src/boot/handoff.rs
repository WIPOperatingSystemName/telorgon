use core::fmt;

pub const SPLASH_HANDOFF_MAGIC: [u8; 8] = *b"TLGSPLSH";
pub const SPLASH_HANDOFF_SIZE: usize = 96;
pub const SPLASH_THEME_DISKS: u32 = 1;
pub const SPLASH_THEME_VOXEL: u32 = 2;
const FRAMEBUFFER_PRESENT: u32 = 1;

/// Fixed metadata for a cooperating loader. Encoding is explicitly little endian;
/// no Rust component, allocator, pointer ownership, or rendering code is transferred.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SplashHandoff {
    pub magic: [u8; 8],
    pub major_version: u16,
    pub minor_version: u16,
    pub total_size: u32,
    pub theme_id: u32,
    pub flags: u32,
    pub elapsed_millis: u64,
    pub framebuffer_address: u64,
    pub framebuffer_size: u64,
    pub width: u32,
    pub height: u32,
    pub stride_bytes: u32,
    pub pixel_bytes: u32,
    pub red_mask: u32,
    pub green_mask: u32,
    pub blue_mask: u32,
    pub reserved_mask: u32,
    pub reserved: [u64; 2],
}

const _: [(); SPLASH_HANDOFF_SIZE] = [(); core::mem::size_of::<SplashHandoff>()];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SplashFramebuffer {
    pub address: u64,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    pub stride_bytes: u32,
    pub pixel_bytes: u32,
    pub red_mask: u32,
    pub green_mask: u32,
    pub blue_mask: u32,
    pub reserved_mask: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplashHandoffError {
    InvalidHeader,
    UnsupportedVersion,
    UnknownFlags,
    InvalidFramebuffer,
    InvalidMasks,
}

impl fmt::Display for SplashHandoffError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidHeader => "invalid splash handoff header",
            Self::UnsupportedVersion => "unsupported splash handoff version",
            Self::UnknownFlags => "unknown splash handoff flags or reserved fields",
            Self::InvalidFramebuffer => "invalid splash framebuffer bounds or dimensions",
            Self::InvalidMasks => "invalid splash framebuffer channel masks",
        })
    }
}

impl core::error::Error for SplashHandoffError {}

impl SplashHandoff {
    pub const fn new(theme_id: u32, elapsed_millis: u64) -> Self {
        Self {
            magic: SPLASH_HANDOFF_MAGIC,
            major_version: 1,
            minor_version: 0,
            total_size: SPLASH_HANDOFF_SIZE as u32,
            theme_id,
            flags: 0,
            elapsed_millis,
            framebuffer_address: 0,
            framebuffer_size: 0,
            width: 0,
            height: 0,
            stride_bytes: 0,
            pixel_bytes: 0,
            red_mask: 0,
            green_mask: 0,
            blue_mask: 0,
            reserved_mask: 0,
            reserved: [0; 2],
        }
    }

    pub fn with_framebuffer(
        mut self,
        framebuffer: SplashFramebuffer,
    ) -> Result<Self, SplashHandoffError> {
        self.flags |= FRAMEBUFFER_PRESENT;
        self.framebuffer_address = framebuffer.address;
        self.framebuffer_size = framebuffer.size;
        self.width = framebuffer.width;
        self.height = framebuffer.height;
        self.stride_bytes = framebuffer.stride_bytes;
        self.pixel_bytes = framebuffer.pixel_bytes;
        self.red_mask = framebuffer.red_mask;
        self.green_mask = framebuffer.green_mask;
        self.blue_mask = framebuffer.blue_mask;
        self.reserved_mask = framebuffer.reserved_mask;
        self.validate()?;
        Ok(self)
    }

    pub fn framebuffer(self) -> Option<SplashFramebuffer> {
        if self.flags & FRAMEBUFFER_PRESENT == 0 {
            return None;
        }
        Some(SplashFramebuffer {
            address: self.framebuffer_address,
            size: self.framebuffer_size,
            width: self.width,
            height: self.height,
            stride_bytes: self.stride_bytes,
            pixel_bytes: self.pixel_bytes,
            red_mask: self.red_mask,
            green_mask: self.green_mask,
            blue_mask: self.blue_mask,
            reserved_mask: self.reserved_mask,
        })
    }

    pub fn validate(&self) -> Result<(), SplashHandoffError> {
        if self.magic != SPLASH_HANDOFF_MAGIC
            || self.total_size != SPLASH_HANDOFF_SIZE as u32
            || self.theme_id == 0
        {
            return Err(SplashHandoffError::InvalidHeader);
        }
        if self.major_version != 1 || self.minor_version != 0 {
            return Err(SplashHandoffError::UnsupportedVersion);
        }
        if self.flags & !FRAMEBUFFER_PRESENT != 0 || self.reserved != [0; 2] {
            return Err(SplashHandoffError::UnknownFlags);
        }
        let Some(framebuffer) = self.framebuffer() else {
            if *self != Self::new(self.theme_id, self.elapsed_millis) {
                return Err(SplashHandoffError::InvalidFramebuffer);
            }
            return Ok(());
        };
        framebuffer.validate()
    }

    pub fn to_bytes(self) -> [u8; SPLASH_HANDOFF_SIZE] {
        let mut bytes = [0; SPLASH_HANDOFF_SIZE];
        bytes[..8].copy_from_slice(&self.magic);
        bytes[8..10].copy_from_slice(&self.major_version.to_le_bytes());
        bytes[10..12].copy_from_slice(&self.minor_version.to_le_bytes());
        bytes[12..16].copy_from_slice(&self.total_size.to_le_bytes());
        bytes[16..20].copy_from_slice(&self.theme_id.to_le_bytes());
        bytes[20..24].copy_from_slice(&self.flags.to_le_bytes());
        for (index, value) in [
            self.elapsed_millis,
            self.framebuffer_address,
            self.framebuffer_size,
        ]
        .into_iter()
        .enumerate()
        {
            let offset = 24 + index * 8;
            bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        }
        for (index, value) in [
            self.width,
            self.height,
            self.stride_bytes,
            self.pixel_bytes,
            self.red_mask,
            self.green_mask,
            self.blue_mask,
            self.reserved_mask,
        ]
        .into_iter()
        .enumerate()
        {
            let offset = 48 + index * 4;
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[80..88].copy_from_slice(&self.reserved[0].to_le_bytes());
        bytes[88..96].copy_from_slice(&self.reserved[1].to_le_bytes());
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SplashHandoffError> {
        if bytes.len() != SPLASH_HANDOFF_SIZE {
            return Err(SplashHandoffError::InvalidHeader);
        }
        let word = |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        let long = |offset| u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
        let result = Self {
            magic: bytes[..8].try_into().unwrap(),
            major_version: u16::from_le_bytes(bytes[8..10].try_into().unwrap()),
            minor_version: u16::from_le_bytes(bytes[10..12].try_into().unwrap()),
            total_size: word(12),
            theme_id: word(16),
            flags: word(20),
            elapsed_millis: long(24),
            framebuffer_address: long(32),
            framebuffer_size: long(40),
            width: word(48),
            height: word(52),
            stride_bytes: word(56),
            pixel_bytes: word(60),
            red_mask: word(64),
            green_mask: word(68),
            blue_mask: word(72),
            reserved_mask: word(76),
            reserved: [long(80), long(88)],
        };
        result.validate()?;
        Ok(result)
    }
}

impl SplashFramebuffer {
    pub fn validate(&self) -> Result<(), SplashHandoffError> {
        if self.address == 0
            || self.width == 0
            || self.height == 0
            || !(1..=4).contains(&self.pixel_bytes)
        {
            return Err(SplashHandoffError::InvalidFramebuffer);
        }
        let row_bytes = u64::from(self.width)
            .checked_mul(u64::from(self.pixel_bytes))
            .ok_or(SplashHandoffError::InvalidFramebuffer)?;
        let required_bytes = u64::from(self.stride_bytes)
            .checked_mul(u64::from(self.height))
            .ok_or(SplashHandoffError::InvalidFramebuffer)?;
        if u64::from(self.stride_bytes) < row_bytes
            || self.size < required_bytes
            || self.address.checked_add(self.size).is_none()
        {
            return Err(SplashHandoffError::InvalidFramebuffer);
        }
        let masks = [
            self.red_mask,
            self.green_mask,
            self.blue_mask,
            self.reserved_mask,
        ];
        if masks[..3].contains(&0) {
            return Err(SplashHandoffError::InvalidMasks);
        }
        let limit = 1u64 << (self.pixel_bytes * 8);
        for index in 0..masks.len() {
            if u64::from(masks[index]) >= limit
                || masks[index + 1..]
                    .iter()
                    .any(|other| masks[index] & *other != 0)
            {
                return Err(SplashHandoffError::InvalidMasks);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn framebuffer() -> SplashFramebuffer {
        SplashFramebuffer {
            address: 0x1234_0000,
            size: 128,
            width: 3,
            height: 2,
            stride_bytes: 64,
            pixel_bytes: 4,
            red_mask: 0xff,
            green_mask: 0xff00,
            blue_mask: 0xff0000,
            reserved_mask: 0xff000000,
        }
    }

    #[test]
    fn fixed_wire_roundtrip_and_unknown_version_rejection() {
        let handoff = SplashHandoff::new(SPLASH_THEME_VOXEL, 1234)
            .with_framebuffer(framebuffer())
            .unwrap();
        let mut bytes = handoff.to_bytes();
        assert_eq!(&bytes[24..32], &1234u64.to_le_bytes());
        assert_eq!(SplashHandoff::from_bytes(&bytes), Ok(handoff));
        bytes[8..10].copy_from_slice(&2u16.to_le_bytes());
        assert_eq!(
            SplashHandoff::from_bytes(&bytes),
            Err(SplashHandoffError::UnsupportedVersion)
        );
        assert_eq!(
            SplashHandoff::from_bytes(&bytes[..95]),
            Err(SplashHandoffError::InvalidHeader)
        );
    }

    #[test]
    fn dimensions_overflow_and_masks_fail_validation() {
        let mut framebuffer = framebuffer();
        framebuffer.address = u64::MAX - 16;
        assert_eq!(
            framebuffer.validate(),
            Err(SplashHandoffError::InvalidFramebuffer)
        );
        framebuffer.address = 0x1000;
        framebuffer.stride_bytes = 8;
        assert_eq!(
            framebuffer.validate(),
            Err(SplashHandoffError::InvalidFramebuffer)
        );
        framebuffer.stride_bytes = 64;
        framebuffer.height = 0;
        assert_eq!(
            framebuffer.validate(),
            Err(SplashHandoffError::InvalidFramebuffer)
        );
        framebuffer.height = 2;
        framebuffer.green_mask = framebuffer.red_mask;
        assert_eq!(
            framebuffer.validate(),
            Err(SplashHandoffError::InvalidMasks)
        );
    }

    #[test]
    fn metadata_without_a_framebuffer_and_unknown_flags() {
        let mut handoff = SplashHandoff::new(SPLASH_THEME_DISKS, 1);
        assert_eq!(SplashHandoff::from_bytes(&handoff.to_bytes()), Ok(handoff));
        handoff.width = 1;
        assert_eq!(
            handoff.validate(),
            Err(SplashHandoffError::InvalidFramebuffer)
        );
        handoff.width = 0;
        handoff.flags = 2;
        assert_eq!(handoff.validate(), Err(SplashHandoffError::UnknownFlags));
    }
}

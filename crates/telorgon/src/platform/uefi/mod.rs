mod allocator;
mod clock;
mod context;
mod handoff;
mod image;
mod input;

pub use allocator::FirmwareAllocator;
pub use clock::UefiClock;
pub use context::UefiContext;
pub use handoff::{InstalledSplashHandoff, SPLASH_HANDOFF_GUID};
pub use image::{EfiExit, LoadedEfiImage};
pub use input::{UefiKey, WaitOutcome};

pub use r_efi::efi::{Handle, Status, SystemTable};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UefiError {
    Firmware {
        operation: &'static str,
        status: usize,
    },
    InvalidFirmwareTable,
    BootServicesEnded,
    InvalidPath,
    ResourceLimit,
    InvalidImage,
    UnsupportedArchitecture,
    NotApplication,
    InvalidFramebuffer,
    InvalidPixelFormat,
    SurfaceMismatch,
    InvalidSplashHandoff,
    SplashHandoffAlreadyInstalled,
}

impl core::fmt::Display for UefiError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if let Self::Firmware { operation, status } = self {
            return write!(formatter, "{operation} failed with EFI status {status:#x}");
        }
        formatter.write_str(match self {
            Self::Firmware { .. } => unreachable!(),
            Self::InvalidFirmwareTable => "invalid firmware table or protocol",
            Self::BootServicesEnded => "firmware boot services have ended",
            Self::InvalidPath => "invalid absolute EFI path",
            Self::ResourceLimit => "firmware resource limit exceeded",
            Self::InvalidImage => "invalid EFI image",
            Self::UnsupportedArchitecture => "EFI image architecture does not match this launcher",
            Self::NotApplication => "EFI launch target must be an application",
            Self::InvalidFramebuffer => "invalid firmware framebuffer bounds",
            Self::InvalidPixelFormat => "unsupported or invalid firmware pixel format",
            Self::SurfaceMismatch => "rendered surface does not match the firmware framebuffer",
            Self::InvalidSplashHandoff => "invalid or unsupported splash handoff metadata",
            Self::SplashHandoffAlreadyInstalled => {
                "a splash handoff is already installed by this context"
            }
        })
    }
}

impl core::error::Error for UefiError {}

pub type UefiResult<T> = core::result::Result<T, UefiError>;

pub(crate) fn check(operation: &'static str, status: Status) -> UefiResult<()> {
    if status.is_error() {
        Err(UefiError::Firmware {
            operation,
            status: status.as_usize(),
        })
    } else {
        Ok(())
    }
}

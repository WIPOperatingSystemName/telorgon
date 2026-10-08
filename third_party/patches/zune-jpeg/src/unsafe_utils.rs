#[cfg(all(all(feature = "x86", not(target_os = "uefi")), any(target_arch = "x86", target_arch = "x86_64")))]
pub use crate::unsafe_utils_avx2::*;
#[cfg(all(all(feature = "neon", not(target_os = "uefi")), target_arch = "aarch64"))]
pub use crate::unsafe_utils_neon::*;

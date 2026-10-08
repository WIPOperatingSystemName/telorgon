#[cfg(not(target_os = "uefi"))]
pub mod avx2;
#[cfg(not(target_os = "uefi"))]
pub mod avx512;
#[cfg(not(target_os = "uefi"))]
pub mod neon;
pub mod scalar;
#[cfg(not(target_os = "uefi"))]
pub mod sse2;
#[cfg(not(target_os = "uefi"))]
pub mod ssse3;
#[cfg(not(target_os = "uefi"))]
pub mod wasm;

// UEFI's soft-float ABI cannot compile the architecture-specific SIMD functions.
// Retain the module API while keeping both intrinsics and runtime detection out of firmware.
#[cfg(target_os = "uefi")]
macro_rules! unavailable_simd {
  ($($name:ident),*) => {
    $(pub mod $name {
      pub fn get_imp() -> Option<super::Adler32Imp> { None }
    })*
  };
}
#[cfg(target_os = "uefi")]
unavailable_simd!(avx2, avx512, neon, sse2, ssse3, wasm);

pub type Adler32Imp = fn(u16, u16, &[u8]) -> (u16, u16);

#[inline]
#[allow(non_snake_case)]
pub const fn _MM_SHUFFLE(z: u32, y: u32, x: u32, w: u32) -> i32 {
  ((z << 6) | (y << 4) | (x << 2) | w) as i32
}

#[cfg(not(target_os = "uefi"))]
pub fn get_imp() -> Adler32Imp {
  avx512::get_imp()
    .or_else(neon::get_imp)
    .or_else(avx2::get_imp)
    .or_else(ssse3::get_imp)
    .or_else(sse2::get_imp)
    .or_else(wasm::get_imp)
    .unwrap_or(scalar::update)
}

#[cfg(target_os = "uefi")]
pub fn get_imp() -> Adler32Imp { scalar::update }

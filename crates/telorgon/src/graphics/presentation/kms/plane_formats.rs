//! Checked decoding of the kernel's native-endian IN_FORMATS blob.
use std::os::fd::AsRawFd;
use std::ptr::NonNull;

use super::{KmsDevice, KmsError, KmsErrorKind, KmsObjectProperties, ScanoutFormat, ffi};

const MAX_BLOB: usize = 1024 * 1024;
const MAX_TUPLES: usize = 16_384;

/// None means legacy implicit layout support, not a LINEAR guarantee.
pub fn plane_formats(
    device: &KmsDevice,
    properties: &KmsObjectProperties,
) -> Result<Option<Vec<ScanoutFormat>>, KmsError> {
    let Some(property) = properties.named("IN_FORMATS") else {
        return Ok(None);
    };
    if property.value == 0 {
        return Ok(None);
    }
    let id = u32::try_from(property.value).map_err(|_| malformed())?;
    let raw = NonNull::new(unsafe { ffi::drmModeGetPropertyBlob(device.fd().as_raw_fd(), id) })
        .ok_or_else(|| {
            KmsError::last_os_error(KmsErrorKind::Native, "DRM IN_FORMATS query failed")
        })?;
    struct Blob(NonNull<ffi::drmModePropertyBlobRes>);
    impl Drop for Blob {
        fn drop(&mut self) {
            unsafe { ffi::drmModeFreePropertyBlob(self.0.as_ptr()) };
        }
    }
    let guard = Blob(raw);
    let native = unsafe { guard.0.as_ref() };
    if native.data.is_null() || native.length as usize > MAX_BLOB || native.length < 24 {
        return Err(malformed());
    }
    let bytes = unsafe { std::slice::from_raw_parts(native.data.cast(), native.length as usize) };
    parse(bytes).map(Some)
}

fn malformed() -> KmsError {
    KmsError::new(
        KmsErrorKind::InvalidState,
        "malformed or oversized DRM IN_FORMATS blob",
    )
}

fn parse(bytes: &[u8]) -> Result<Vec<ScanoutFormat>, KmsError> {
    let u32_at = |offset: usize| -> Result<u32, KmsError> {
        let end = offset.checked_add(4).ok_or_else(malformed)?;
        Ok(u32::from_ne_bytes(
            bytes
                .get(offset..end)
                .ok_or_else(malformed)?
                .try_into()
                .unwrap(),
        ))
    };
    if bytes.len() > MAX_BLOB || u32_at(0)? != 1 || u32_at(4)? != 0 {
        return Err(malformed());
    }
    let count = u32_at(8)? as usize;
    let formats_start = u32_at(12)? as usize;
    let modifier_count = u32_at(16)? as usize;
    let modifiers_start = u32_at(20)? as usize;
    let range =
        |start: usize, count: usize, stride: usize| -> Result<std::ops::Range<usize>, KmsError> {
            let end = count
                .checked_mul(stride)
                .and_then(|size| start.checked_add(size))
                .ok_or_else(malformed)?;
            if start < 24 || end > bytes.len() {
                return Err(malformed());
            }
            Ok(start..end)
        };
    let formats = range(formats_start, count, 4)?;
    let modifiers = range(modifiers_start, modifier_count, 24)?;
    if !formats.is_empty()
        && !modifiers.is_empty()
        && formats.start < modifiers.end
        && modifiers.start < formats.end
    {
        return Err(malformed());
    }
    let mut tuples = Vec::new();
    for entry in bytes[modifiers].as_chunks::<24>().0 {
        let mask = u64::from_ne_bytes(entry[..8].try_into().unwrap());
        let offset = u32::from_ne_bytes(entry[8..12].try_into().unwrap()) as usize;
        let modifier = u64::from_ne_bytes(entry[16..24].try_into().unwrap());
        for bit in 0..64 {
            if mask & (1_u64 << bit) == 0 {
                continue;
            }
            let index = offset.checked_add(bit).ok_or_else(malformed)?;
            if index >= count || tuples.len() >= MAX_TUPLES {
                return Err(malformed());
            }
            tuples.push(ScanoutFormat {
                fourcc: u32_at(formats_start + index * 4)?,
                modifier,
            });
        }
    }
    tuples.sort_by_key(|f| (f.fourcc, f.modifier));
    tuples.dedup();
    Ok(tuples)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Vec<u8> {
        let mut bytes = Vec::new();
        for n in [1_u32, 0, 66, 24, 1, 24 + 66 * 4] {
            bytes.extend(n.to_ne_bytes());
        }
        for n in 0..66_u32 {
            bytes.extend(n.to_ne_bytes());
        }
        bytes.extend(3_u64.to_ne_bytes());
        bytes.extend(64_u32.to_ne_bytes());
        bytes.extend(0_u32.to_ne_bytes());
        bytes.extend(0x0300000000606014_u64.to_ne_bytes());
        bytes
    }
    #[test]
    fn modifier_mask_offsets_are_format_indices_not_bytes() {
        let formats = parse(&fixture()).unwrap();
        assert_eq!(
            formats.iter().map(|f| f.fourcc).collect::<Vec<_>>(),
            [64, 65]
        );
        assert_eq!(formats[0].modifier, 0x0300000000606014);
    }
    #[test]
    fn rejects_truncation_out_of_range_masks_and_overlapping_arrays() {
        let data = fixture();
        for length in 0..data.len() {
            assert!(parse(&data[..length]).is_err());
        }
        let mut invalid = data.clone();
        invalid[20..24].copy_from_slice(&24_u32.to_ne_bytes());
        assert!(parse(&invalid).is_err());
        let mut invalid = data;
        let offset = invalid.len() - 16;
        invalid[offset..offset + 4].copy_from_slice(&65_u32.to_ne_bytes());
        assert!(parse(&invalid).is_err());
    }
    #[test]
    fn invalid_modifier_matches_linux_abi_and_vulkan() {
        assert_eq!(super::super::DRM_FORMAT_MOD_INVALID, 0x00ff_ffff_ffff_ffff);
        assert_ne!(
            super::super::DRM_FORMAT_MOD_INVALID,
            super::super::DRM_FORMAT_MOD_LINEAR
        );
        assert_eq!(super::super::ffi::GBM_BO_TRANSFER_WRITE, 2);
        assert_eq!(super::super::ffi::GBM_BO_TRANSFER_READ_WRITE, 3);
    }
}

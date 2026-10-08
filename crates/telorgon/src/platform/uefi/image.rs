use alloc::vec::Vec;
use core::ffi::c_void;
use core::ptr::{self, NonNull};
use r_efi::{
    efi,
    protocols::{device_path, file, loaded_image, simple_file_system},
};

use super::{UefiContext, UefiError, UefiResult, check};

pub struct EfiExit {
    pub status: efi::Status,
    /// Bounded UTF-16 diagnostic text; never interpreted as configuration.
    pub exit_data: Vec<u16>,
}

pub struct LoadedEfiImage<'a> {
    context: &'a UefiContext,
    handle: efi::Handle,
    options: Vec<u64>,
    owns_unstarted_image: bool,
    image_base: *mut c_void,
    image_size: u64,
}

struct OpenFile<'a> {
    context: &'a UefiContext,
    protocol: NonNull<file::Protocol>,
}

impl Drop for OpenFile<'_> {
    fn drop(&mut self) {
        if self.context.boot_services_active() {
            unsafe { (self.protocol.as_ref().close)(self.protocol.as_ptr()) };
        }
    }
}

impl UefiContext {
    fn boot_device(&self) -> UefiResult<efi::Handle> {
        let loaded = self
            .handle_protocol::<loaded_image::Protocol>(self.image, loaded_image::PROTOCOL_GUID)?;
        let handle = unsafe { loaded.as_ref().device_handle };
        if handle.is_null() {
            return Err(UefiError::InvalidFirmwareTable);
        }
        Ok(handle)
    }

    fn open_boot_file(&self, path: &str) -> UefiResult<OpenFile<'_>> {
        let mut path = encode_path(path)?;
        let filesystem = self.handle_protocol::<simple_file_system::Protocol>(
            self.boot_device()?,
            simple_file_system::PROTOCOL_GUID,
        )?;
        let mut root = ptr::null_mut();
        check("open boot volume", unsafe {
            (filesystem.as_ref().open_volume)(filesystem.as_ptr(), &mut root)
        })?;
        let root = OpenFile {
            context: self,
            protocol: NonNull::new(root).ok_or(UefiError::InvalidFirmwareTable)?,
        };
        let mut opened = ptr::null_mut();
        check("open boot file", unsafe {
            (root.protocol.as_ref().open)(
                root.protocol.as_ptr(),
                &mut opened,
                path.as_mut_ptr(),
                file::MODE_READ,
                0,
            )
        })?;
        let opened = OpenFile {
            context: self,
            protocol: NonNull::new(opened).ok_or(UefiError::InvalidFirmwareTable)?,
        };
        drop(root);
        Ok(opened)
    }

    /// Reads up to `max_bytes` from an offset without reading the rest of an EFI
    /// executable. Short reads indicate EOF, so payload callers can reject truncation.
    pub fn read_file_range(
        &self,
        path: &str,
        offset: u64,
        max_bytes: usize,
    ) -> UefiResult<Vec<u8>> {
        if max_bytes > 512 * 1024 * 1024 || offset.checked_add(max_bytes as u64).is_none() {
            return Err(UefiError::ResourceLimit);
        }
        let opened = self.open_boot_file(path)?;
        check("seek boot file", unsafe {
            (opened.protocol.as_ref().set_position)(opened.protocol.as_ptr(), offset)
        })?;
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 8192];
        while bytes.len() < max_bytes {
            let mut count = chunk.len().min(max_bytes - bytes.len());
            let requested = count;
            check("read boot file range", unsafe {
                (opened.protocol.as_ref().read)(
                    opened.protocol.as_ptr(),
                    &mut count,
                    chunk.as_mut_ptr().cast(),
                )
            })?;
            if count == 0 {
                break;
            }
            if count > requested {
                return Err(UefiError::ResourceLimit);
            }
            bytes
                .try_reserve(count)
                .map_err(|_| UefiError::ResourceLimit)?;
            bytes.extend_from_slice(&chunk[..count]);
        }
        Ok(bytes)
    }

    pub fn read_file(&self, path: &str, max_bytes: usize) -> UefiResult<Vec<u8>> {
        let opened = self.open_boot_file(path)?;
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let mut count = chunk
                .len()
                .min(max_bytes.saturating_sub(bytes.len()).saturating_add(1));
            check("read boot file", unsafe {
                (opened.protocol.as_ref().read)(
                    opened.protocol.as_ptr(),
                    &mut count,
                    chunk.as_mut_ptr().cast(),
                )
            })?;
            if count == 0 {
                break;
            }
            if count > chunk.len()
                || bytes
                    .len()
                    .checked_add(count)
                    .filter(|n| *n <= max_bytes)
                    .is_none()
            {
                return Err(UefiError::ResourceLimit);
            }
            bytes
                .try_reserve(count)
                .map_err(|_| UefiError::ResourceLimit)?;
            bytes.extend_from_slice(&chunk[..count]);
        }
        Ok(bytes)
    }

    pub fn load_image<'a>(
        &'a self,
        path: &str,
        load_options: &[u8],
        max_image_bytes: usize,
    ) -> UefiResult<LoadedEfiImage<'a>> {
        if load_options.len() > 64 * 1024 {
            return Err(UefiError::ResourceLimit);
        }
        let mut bytes = self.read_file(path, max_image_bytes)?;
        validate_application(&bytes)?;
        let mut path = self.image_device_path(path)?;
        let mut handle = ptr::null_mut();
        let services = self.services()?;
        let status = unsafe {
            (services.load_image)(
                false.into(),
                self.image,
                path.as_mut_ptr().cast(),
                bytes.as_mut_ptr().cast(),
                bytes.len(),
                &mut handle,
            )
        };
        if let Err(error) = check("load EFI image", status) {
            // SECURITY_VIOLATION is explicitly allowed to return a loaded handle.
            // Other errors do not provide a valid output handle.
            if status == efi::Status::SECURITY_VIOLATION && !handle.is_null() {
                unsafe { (services.unload_image)(handle) };
            }
            return Err(error);
        }
        if handle.is_null() {
            return Err(UefiError::InvalidFirmwareTable);
        }
        let mut image = LoadedEfiImage {
            context: self,
            handle,
            options: Vec::new(),
            owns_unstarted_image: true,
            image_base: ptr::null_mut(),
            image_size: 0,
        };
        let loaded =
            self.handle_protocol::<loaded_image::Protocol>(handle, loaded_image::PROTOCOL_GUID)?;
        image.image_base = unsafe { loaded.as_ref().image_base };
        image.image_size = unsafe { loaded.as_ref().image_size };
        let words = load_options.len().div_ceil(core::mem::size_of::<u64>());
        image
            .options
            .try_reserve_exact(words)
            .map_err(|_| UefiError::ResourceLimit)?;
        image.options.resize(words, 0);
        if !load_options.is_empty() {
            unsafe {
                ptr::copy_nonoverlapping(
                    load_options.as_ptr(),
                    image.options.as_mut_ptr().cast::<u8>(),
                    load_options.len(),
                )
            };
        }
        unsafe {
            (*loaded.as_ptr()).load_options_size = load_options.len() as u32;
            (*loaded.as_ptr()).load_options = if image.options.is_empty() {
                ptr::null_mut()
            } else {
                image.options.as_mut_ptr().cast()
            };
        }
        Ok(image)
    }

    fn image_device_path(&self, path: &str) -> UefiResult<Vec<u8>> {
        let encoded = encode_path(path)?;
        let device = self.handle_protocol::<device_path::Protocol>(
            self.boot_device()?,
            device_path::PROTOCOL_GUID,
        )?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve(4096)
            .map_err(|_| UefiError::ResourceLimit)?;
        let mut offset = 0usize;
        loop {
            if offset > 4096 - 4 {
                return Err(UefiError::InvalidPath);
            }
            // EFI device path memory is firmware-owned and valid by the unsafe
            // context contract; cap its traversal and reject malformed nodes.
            let header = unsafe {
                ptr::read_unaligned(
                    device
                        .as_ptr()
                        .cast::<u8>()
                        .add(offset)
                        .cast::<device_path::Protocol>(),
                )
            };
            let length = usize::from(u16::from_le_bytes(header.length));
            if length < 4 || offset.checked_add(length).filter(|n| *n <= 4096).is_none() {
                return Err(UefiError::InvalidPath);
            }
            if header.r#type == device_path::TYPE_END {
                if header.sub_type != device_path::End::SUBTYPE_ENTIRE || length != 4 {
                    return Err(UefiError::InvalidPath);
                }
                break;
            }
            let node = unsafe {
                core::slice::from_raw_parts(device.as_ptr().cast::<u8>().add(offset), length)
            };
            bytes.extend_from_slice(node);
            offset += length;
        }
        let node_length = encoded
            .len()
            .checked_mul(2)
            .and_then(|n| n.checked_add(4))
            .ok_or(UefiError::ResourceLimit)?;
        let node_length = u16::try_from(node_length).map_err(|_| UefiError::ResourceLimit)?;
        bytes
            .try_reserve(usize::from(node_length) + 4)
            .map_err(|_| UefiError::ResourceLimit)?;
        bytes.extend_from_slice(&[
            device_path::TYPE_MEDIA,
            device_path::Media::SUBTYPE_FILE_PATH,
        ]);
        bytes.extend_from_slice(&node_length.to_le_bytes());
        for unit in encoded {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes.extend_from_slice(&[
            device_path::TYPE_END,
            device_path::End::SUBTYPE_ENTIRE,
            4,
            0,
        ]);
        Ok(bytes)
    }
}

impl LoadedEfiImage<'_> {
    pub fn start(mut self) -> UefiResult<EfiExit> {
        let services = self.context.services()?;
        let mut exit_size = 0usize;
        let mut exit_data = ptr::null_mut();
        self.owns_unstarted_image = false;
        self.context.invalidate_presentation();
        let status = unsafe { (services.start_image)(self.handle, &mut exit_size, &mut exit_data) };
        if !self.context.boot_services_active() {
            // A compliant OS never comes back here. Do not run allocator-backed
            // destruction or firmware cleanup if an exit transition was observed.
            core::mem::forget(self);
            return Err(UefiError::BootServicesEnded);
        }
        // Application return frees its handle. A refused original may remain,
        // while a child may have reused that address for another image. Match
        // the original identity before attempting any cleanup.
        if self.original_image_is_loaded() {
            self.clear_options();
            unsafe { (services.unload_image)(self.handle) };
        }
        let mut diagnostic = Vec::new();
        if !exit_data.is_null() {
            let count = (exit_size / 2).min(4096);
            if diagnostic.try_reserve_exact(count).is_err() {
                unsafe { (services.free_pool)(exit_data.cast()) };
                return Err(UefiError::ResourceLimit);
            }
            for index in 0..count {
                let unit = unsafe { ptr::read_unaligned(exit_data.add(index)) };
                if unit == 0 {
                    break;
                }
                diagnostic.push(unit);
            }
            unsafe { (services.free_pool)(exit_data.cast()) };
        }
        Ok(EfiExit {
            status,
            exit_data: diagnostic,
        })
    }

    fn clear_options(&self) {
        if !self.original_image_is_loaded() {
            return;
        }
        if let Ok(loaded) = self
            .context
            .handle_protocol::<loaded_image::Protocol>(self.handle, loaded_image::PROTOCOL_GUID)
        {
            unsafe {
                (*loaded.as_ptr()).load_options = ptr::null_mut();
                (*loaded.as_ptr()).load_options_size = 0;
            }
        }
    }

    fn original_image_is_loaded(&self) -> bool {
        let Ok(loaded) = self
            .context
            .handle_protocol::<loaded_image::Protocol>(self.handle, loaded_image::PROTOCOL_GUID)
        else {
            return false;
        };
        let loaded = unsafe { loaded.as_ref() };
        loaded.parent_handle == self.context.image
            && loaded.image_base == self.image_base
            && loaded.image_size == self.image_size
    }
}

impl Drop for LoadedEfiImage<'_> {
    fn drop(&mut self) {
        if self.owns_unstarted_image {
            if let Ok(services) = self.context.services() {
                // A failure before identity capture occurs immediately after
                // LoadImage, before another image can reuse this handle.
                if self.image_base.is_null() || self.original_image_is_loaded() {
                    self.clear_options();
                    unsafe { (services.unload_image)(self.handle) };
                }
            }
        }
    }
}

fn encode_path(path: &str) -> UefiResult<Vec<u16>> {
    if !path.starts_with('\\') || path.contains('\0') || path.contains('/') || path.len() > 4096 {
        return Err(UefiError::InvalidPath);
    }
    if path.split('\\').any(|part| part == "." || part == "..") {
        return Err(UefiError::InvalidPath);
    }
    let mut result = Vec::new();
    result
        .try_reserve(path.len() + 1)
        .map_err(|_| UefiError::ResourceLimit)?;
    result.extend(path.encode_utf16());
    result.push(0);
    Ok(result)
}

fn validate_application(image: &[u8]) -> UefiResult<()> {
    fn word(bytes: &[u8], offset: usize) -> Option<u16> {
        Some(u16::from_le_bytes(
            bytes.get(offset..offset.checked_add(2)?)?.try_into().ok()?,
        ))
    }
    if image.get(..2) != Some(b"MZ") {
        return Err(UefiError::InvalidImage);
    }
    let offset = u32::from_le_bytes(
        image
            .get(0x3c..0x40)
            .ok_or(UefiError::InvalidImage)?
            .try_into()
            .unwrap(),
    ) as usize;
    let pe = image.get(offset..).ok_or(UefiError::InvalidImage)?;
    if pe.get(..4) != Some(b"PE\0\0")
        || word(pe, 24) != Some(0x20b)
        || word(pe, 20).unwrap_or(0) < 70
    {
        return Err(UefiError::InvalidImage);
    }
    #[cfg(target_arch = "x86_64")]
    let expected_machine = 0x8664;
    #[cfg(target_arch = "aarch64")]
    let expected_machine = 0xaa64;
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    let expected_machine = 0xffff;
    if word(pe, 4) != Some(expected_machine) {
        return Err(UefiError::UnsupportedArchitecture);
    }
    if word(pe, 24 + 68) != Some(10) {
        return Err(UefiError::NotApplication);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_image_and_driver_are_not_launch_recipes() {
        assert_eq!(validate_application(b"MZ"), Err(UefiError::InvalidImage));
        let mut image = alloc::vec![0; 256];
        image[..2].copy_from_slice(b"MZ");
        image[0x3c..0x40].copy_from_slice(&64u32.to_le_bytes());
        image[64..68].copy_from_slice(b"PE\0\0");
        image[68..70].copy_from_slice(&0x8664u16.to_le_bytes());
        image[84..86].copy_from_slice(&240u16.to_le_bytes());
        image[88..90].copy_from_slice(&0x20bu16.to_le_bytes());
        image[156..158].copy_from_slice(&11u16.to_le_bytes());
        #[cfg(target_arch = "x86_64")]
        assert_eq!(validate_application(&image), Err(UefiError::NotApplication));
        image[0x3c..0x40].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(validate_application(&image), Err(UefiError::InvalidImage));
    }
}

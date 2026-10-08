use core::ffi::c_void;
use core::ptr::{self, NonNull};
use r_efi::efi;

use super::{UefiContext, UefiError, UefiResult, check};
use crate::boot::{SPLASH_HANDOFF_SIZE, SplashHandoff};

pub const SPLASH_HANDOFF_GUID: efi::Guid = efi::Guid::from_fields(
    0x63c3876c,
    0x3d44,
    0x4a8b,
    0xb9,
    0xc7,
    &[0x1f, 0x02, 0x76, 0x4a, 0x7c, 0xee],
);

/// Keep this guard alive across StartImage. A cooperating child must copy this
/// metadata before exit and supply framebuffer mapping/reservation itself.
pub struct InstalledSplashHandoff<'a> {
    context: &'a UefiContext,
    allocation: NonNull<u8>,
    previous: *mut c_void,
}

impl UefiContext {
    pub fn install_splash_handoff<'a>(
        &'a self,
        handoff: &SplashHandoff,
    ) -> UefiResult<InstalledSplashHandoff<'a>> {
        handoff
            .validate()
            .map_err(|_| UefiError::InvalidSplashHandoff)?;
        if self.splash_handoff_installed.get() {
            return Err(UefiError::SplashHandoffAlreadyInstalled);
        }
        let previous = self.splash_table()?.unwrap_or(ptr::null_mut());
        let services = self.services()?;
        let mut allocation = ptr::null_mut();
        check("allocate splash handoff", unsafe {
            (services.allocate_pool)(efi::LOADER_DATA, SPLASH_HANDOFF_SIZE, &mut allocation)
        })?;
        let allocation =
            NonNull::new(allocation.cast::<u8>()).ok_or(UefiError::InvalidFirmwareTable)?;
        let bytes = handoff.to_bytes();
        unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), allocation.as_ptr(), bytes.len()) };
        let mut guid = SPLASH_HANDOFF_GUID;
        let status = unsafe {
            (services.install_configuration_table)(&mut guid, allocation.as_ptr().cast())
        };
        if let Err(error) = check("install splash handoff", status) {
            unsafe { (services.free_pool)(allocation.as_ptr().cast()) };
            return Err(error);
        }
        self.splash_handoff_installed.set(true);
        Ok(InstalledSplashHandoff {
            context: self,
            allocation,
            previous,
        })
    }

    /// Returns a validated value, never a borrowed reference into loader memory.
    pub fn splash_handoff(&self) -> UefiResult<Option<SplashHandoff>> {
        let Some(table) = self.splash_table()? else {
            return Ok(None);
        };
        self.validate_metadata_span(table, SPLASH_HANDOFF_SIZE)?;
        let mut bytes = [0; SPLASH_HANDOFF_SIZE];
        unsafe { ptr::copy_nonoverlapping(table.cast::<u8>(), bytes.as_mut_ptr(), bytes.len()) };
        SplashHandoff::from_bytes(&bytes)
            .map(Some)
            .map_err(|_| UefiError::InvalidSplashHandoff)
    }

    fn splash_table(&self) -> UefiResult<Option<*mut c_void>> {
        let table = self.table()?;
        if table.number_of_table_entries > 4096
            || (table.number_of_table_entries != 0 && table.configuration_table.is_null())
        {
            return Err(UefiError::InvalidFirmwareTable);
        }
        let mut result = None;
        for index in 0..table.number_of_table_entries {
            let entry = unsafe { ptr::read(table.configuration_table.add(index)) };
            if entry.vendor_guid == SPLASH_HANDOFF_GUID {
                if result.is_some() || entry.vendor_table.is_null() {
                    return Err(UefiError::InvalidSplashHandoff);
                }
                result = Some(entry.vendor_table);
            }
        }
        Ok(result)
    }

    fn validate_metadata_span(&self, address: *mut c_void, length: usize) -> UefiResult<()> {
        let span_start = address as usize as u64;
        let span_end = span_start
            .checked_add(length as u64)
            .ok_or(UefiError::InvalidSplashHandoff)?;
        let services = self.services()?;
        let mut map_size = 0usize;
        let mut map_key = 0usize;
        let mut descriptor_size = 0usize;
        let mut descriptor_version = 0u32;
        let status = unsafe {
            (services.get_memory_map)(
                &mut map_size,
                ptr::null_mut(),
                &mut map_key,
                &mut descriptor_size,
                &mut descriptor_version,
            )
        };
        if status != efi::Status::BUFFER_TOO_SMALL {
            return Err(UefiError::InvalidSplashHandoff);
        }
        // GetMemoryMap can grow when its buffer is allocated. Bound both growth
        // and retries; this reads a memory map, and never exits boot services.
        for _ in 0..4 {
            let slack = descriptor_size
                .max(core::mem::size_of::<efi::MemoryDescriptor>())
                .checked_mul(16)
                .ok_or(UefiError::ResourceLimit)?;
            let capacity = map_size
                .checked_add(slack)
                .filter(|size| *size <= 1024 * 1024)
                .ok_or(UefiError::ResourceLimit)?;
            let mut buffer = ptr::null_mut();
            check("allocate handoff memory map", unsafe {
                (services.allocate_pool)(efi::LOADER_DATA, capacity, &mut buffer)
            })?;
            if buffer.is_null() {
                return Err(UefiError::InvalidFirmwareTable);
            }
            map_size = capacity;
            let status = unsafe {
                (services.get_memory_map)(
                    &mut map_size,
                    buffer.cast(),
                    &mut map_key,
                    &mut descriptor_size,
                    &mut descriptor_version,
                )
            };
            if status == efi::Status::BUFFER_TOO_SMALL {
                unsafe { (services.free_pool)(buffer) };
                continue;
            }
            let mut readable = false;
            if !status.is_error()
                && map_size <= capacity
                && descriptor_size >= core::mem::size_of::<efi::MemoryDescriptor>()
                && map_size.is_multiple_of(descriptor_size)
                && descriptor_version == efi::MEMORY_DESCRIPTOR_VERSION
            {
                for offset in (0..map_size).step_by(descriptor_size) {
                    let descriptor = unsafe {
                        ptr::read_unaligned(
                            buffer
                                .cast::<u8>()
                                .add(offset)
                                .cast::<efi::MemoryDescriptor>(),
                        )
                    };
                    let memory_end = descriptor
                        .number_of_pages
                        .checked_mul(4096)
                        .and_then(|bytes| descriptor.physical_start.checked_add(bytes));
                    let ordinary_data = matches!(
                        descriptor.r#type,
                        efi::LOADER_DATA
                            | efi::BOOT_SERVICES_DATA
                            | efi::RUNTIME_SERVICES_DATA
                            | efi::ACPI_RECLAIM_MEMORY
                    );
                    if ordinary_data
                        && descriptor.physical_start <= span_start
                        && memory_end.is_some_and(|end| span_end <= end)
                    {
                        readable = true;
                        break;
                    }
                }
            }
            unsafe { (services.free_pool)(buffer) };
            check("read handoff memory map", status)?;
            return if readable {
                Ok(())
            } else {
                Err(UefiError::InvalidSplashHandoff)
            };
        }
        Err(UefiError::ResourceLimit)
    }
}

impl Drop for InstalledSplashHandoff<'_> {
    fn drop(&mut self) {
        if let Ok(services) = self.context.services() {
            let mut guid = SPLASH_HANDOFF_GUID;
            let status =
                unsafe { (services.install_configuration_table)(&mut guid, self.previous) };
            // Restore first: configuration tables must never retain freed data.
            // Failure leaves this allocation installed and owned by firmware.
            if !status.is_error() {
                self.context.splash_handoff_installed.set(false);
                unsafe { (services.free_pool)(self.allocation.as_ptr().cast()) };
            }
        }
    }
}

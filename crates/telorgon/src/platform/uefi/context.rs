use alloc::vec::Vec;
use core::cell::Cell;
use core::ffi::c_void;
use core::marker::PhantomData;
use core::ptr::{self, NonNull};
use r_efi::efi;

use super::{FirmwareAllocator, UefiError, UefiResult, check};

struct ExitObservation {
    exited: Cell<bool>,
    allocator: Option<&'static FirmwareAllocator>,
}

pub struct UefiContext {
    pub(crate) image: efi::Handle,
    table: NonNull<efi::SystemTable>,
    services: NonNull<efi::BootServices>,
    observation: NonNull<ExitObservation>,
    exit_event: efi::Event,
    presentation_epoch: Cell<u64>,
    pub(crate) splash_handoff_installed: Cell<bool>,
    _boot_cpu: PhantomData<*mut ()>,
}

unsafe extern "efiapi" fn observe_exit(_event: efi::Event, context: *mut c_void) {
    let observation = unsafe { &*context.cast::<ExitObservation>() };
    observation.exited.set(true);
    if let Some(allocator) = observation.allocator {
        allocator.disable();
    }
}

impl UefiContext {
    /// # Safety
    /// The handle and table must come from this application's EFI entry point.
    /// Call on the boot CPU at application TPL and create only one context.
    /// Receivers must follow the EFI contract and never return after exiting
    /// boot services or attempting an exit that partially shuts firmware down.
    pub unsafe fn from_raw(image: efi::Handle, table: *mut efi::SystemTable) -> UefiResult<Self> {
        unsafe { Self::create(image, table, None) }
    }

    /// # Safety
    /// Same contract as `from_raw`; allocator must already be initialized for
    /// this firmware session. Exit notification disables it automatically.
    pub unsafe fn from_raw_with_allocator(
        image: efi::Handle,
        table: *mut efi::SystemTable,
        allocator: &'static FirmwareAllocator,
    ) -> UefiResult<Self> {
        unsafe { Self::create(image, table, Some(allocator)) }
    }

    unsafe fn create(
        image: efi::Handle,
        table: *mut efi::SystemTable,
        allocator: Option<&'static FirmwareAllocator>,
    ) -> UefiResult<Self> {
        let table = NonNull::new(table).ok_or(UefiError::InvalidFirmwareTable)?;
        if image.is_null() || unsafe { table.as_ref().hdr.signature } != efi::SYSTEM_TABLE_SIGNATURE
        {
            return Err(UefiError::InvalidFirmwareTable);
        }
        let services = NonNull::new(unsafe { table.as_ref().boot_services })
            .ok_or(UefiError::InvalidFirmwareTable)?;
        if unsafe { services.as_ref().hdr.signature } != efi::BOOT_SERVICES_SIGNATURE {
            return Err(UefiError::InvalidFirmwareTable);
        }
        let mut storage = ptr::null_mut();
        check("allocate exit observation", unsafe {
            (services.as_ref().allocate_pool)(
                efi::LOADER_DATA,
                core::mem::size_of::<ExitObservation>(),
                &mut storage,
            )
        })?;
        let observation = NonNull::new(storage.cast::<ExitObservation>())
            .ok_or(UefiError::InvalidFirmwareTable)?;
        unsafe {
            observation.as_ptr().write(ExitObservation {
                exited: Cell::new(false),
                allocator,
            })
        };
        let mut exit_event = ptr::null_mut();
        let status = unsafe {
            (services.as_ref().create_event)(
                efi::EVT_SIGNAL_EXIT_BOOT_SERVICES,
                efi::TPL_NOTIFY,
                Some(observe_exit),
                storage,
                &mut exit_event,
            )
        };
        if let Err(error) = check("observe boot services exit", status) {
            unsafe { (services.as_ref().free_pool)(storage) };
            return Err(error);
        }
        Ok(Self {
            image,
            table,
            services,
            observation,
            exit_event,
            presentation_epoch: Cell::new(0),
            splash_handoff_installed: Cell::new(false),
            _boot_cpu: PhantomData,
        })
    }

    pub fn boot_services_active(&self) -> bool {
        !unsafe { self.observation.as_ref().exited.get() }
    }

    pub fn ensure_active(&self) -> UefiResult<()> {
        if self.boot_services_active() {
            Ok(())
        } else {
            Err(UefiError::BootServicesEnded)
        }
    }

    pub(crate) fn presentation_epoch(&self) -> u64 {
        self.presentation_epoch.get()
    }

    pub(crate) fn invalidate_presentation(&self) {
        self.presentation_epoch
            .set(self.presentation_epoch.get().wrapping_add(1));
    }

    pub(crate) fn services(&self) -> UefiResult<&efi::BootServices> {
        self.ensure_active()?;
        Ok(unsafe { self.services.as_ref() })
    }

    pub(crate) fn table(&self) -> UefiResult<&efi::SystemTable> {
        self.ensure_active()?;
        Ok(unsafe { self.table.as_ref() })
    }

    pub(crate) fn handle_protocol<T>(
        &self,
        handle: efi::Handle,
        mut guid: efi::Guid,
    ) -> UefiResult<NonNull<T>> {
        let mut interface = ptr::null_mut();
        check("open protocol", unsafe {
            (self.services()?.handle_protocol)(handle, &mut guid, &mut interface)
        })?;
        NonNull::new(interface.cast()).ok_or(UefiError::InvalidFirmwareTable)
    }

    pub(crate) fn locate_protocol<T>(&self, mut guid: efi::Guid) -> UefiResult<NonNull<T>> {
        let mut interface = ptr::null_mut();
        check("locate protocol", unsafe {
            (self.services()?.locate_protocol)(&mut guid, ptr::null_mut(), &mut interface)
        })?;
        NonNull::new(interface.cast()).ok_or(UefiError::InvalidFirmwareTable)
    }

    pub fn set_watchdog(&self, seconds: usize) -> UefiResult<()> {
        check("set watchdog", unsafe {
            (self.services()?.set_watchdog_timer)(seconds, 0x10000, 0, ptr::null_mut())
        })
    }

    pub fn write_text(&self, text: &str) -> UefiResult<()> {
        let output = NonNull::new(self.table()?.con_out).ok_or(UefiError::InvalidFirmwareTable)?;
        let mut encoded = Vec::new();
        encoded
            .try_reserve(
                text.len()
                    .checked_mul(2)
                    .and_then(|n| n.checked_add(1))
                    .ok_or(UefiError::ResourceLimit)?,
            )
            .map_err(|_| UefiError::ResourceLimit)?;
        for character in text.chars() {
            if character == '\0' {
                return Err(UefiError::InvalidPath);
            }
            if character == '\n' {
                encoded.push('\r' as u16);
            }
            let mut units = [0; 2];
            encoded.extend_from_slice(character.encode_utf16(&mut units));
        }
        encoded.push(0);
        check("write text", unsafe {
            (output.as_ref().output_string)(output.as_ptr(), encoded.as_mut_ptr())
        })
    }

    pub fn clear_text(&self) -> UefiResult<()> {
        let output = NonNull::new(self.table()?.con_out).ok_or(UefiError::InvalidFirmwareTable)?;
        check("clear text", unsafe {
            (output.as_ref().clear_screen)(output.as_ptr())
        })
    }
}

impl Drop for UefiContext {
    fn drop(&mut self) {
        if self.boot_services_active() {
            unsafe {
                let status = (self.services.as_ref().close_event)(self.exit_event);
                // Failed event closure must retain callback context. Leaking this
                // tiny allocation is safer than leaving a dangling notification.
                if !status.is_error() {
                    (self.services.as_ref().free_pool)(self.observation.as_ptr().cast());
                }
            }
        }
    }
}

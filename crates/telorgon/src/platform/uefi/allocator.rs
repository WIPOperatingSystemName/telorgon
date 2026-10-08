use core::alloc::{GlobalAlloc, Layout};
use core::ffi::c_void;
use core::ptr;
use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use r_efi::efi;

use super::{UefiError, UefiResult};

/// Firmware-backed allocation, explicitly initialized by the EFI entry point.
pub struct FirmwareAllocator {
    services: AtomicPtr<efi::BootServices>,
    used: AtomicUsize,
    limit: usize,
}

#[derive(Clone, Copy)]
struct AllocationHeader {
    original: *mut c_void,
    bytes: usize,
}

impl FirmwareAllocator {
    pub const fn new(limit_bytes: usize) -> Self {
        Self {
            services: AtomicPtr::new(ptr::null_mut()),
            used: AtomicUsize::new(0),
            limit: limit_bytes,
        }
    }

    /// # Safety
    /// Call once on the firmware boot CPU at application TPL, before allocation.
    /// The table must remain valid until `disable`; no allocation may span a
    /// transition to another address space or outlive boot services.
    pub unsafe fn initialize(&self, table: *mut efi::SystemTable) -> UefiResult<()> {
        if table.is_null() {
            return Err(UefiError::InvalidFirmwareTable);
        }
        let services = unsafe { (*table).boot_services };
        if services.is_null() {
            return Err(UefiError::InvalidFirmwareTable);
        }
        self.services
            .compare_exchange(
                ptr::null_mut(),
                services,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .map_err(|_| UefiError::InvalidFirmwareTable)?;
        Ok(())
    }

    pub fn disable(&self) {
        self.services.store(ptr::null_mut(), Ordering::SeqCst);
    }

    pub fn allocated_bytes(&self) -> usize {
        self.used.load(Ordering::Relaxed)
    }
}

// UEFI hosts must use this allocator only on the boot CPU at application TPL.
// Atomics support the exit notification, not concurrent firmware allocation.
unsafe impl GlobalAlloc for FirmwareAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let services = self.services.load(Ordering::SeqCst);
        if services.is_null() {
            return ptr::null_mut();
        }
        let Some(bytes) = layout
            .size()
            .max(1)
            .checked_add(layout.align() - 1)
            .and_then(|n| n.checked_add(core::mem::size_of::<AllocationHeader>()))
        else {
            return ptr::null_mut();
        };
        if self
            .used
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |used| {
                used.checked_add(bytes).filter(|value| *value <= self.limit)
            })
            .is_err()
        {
            return ptr::null_mut();
        }
        let mut original = ptr::null_mut();
        let status = unsafe { ((*services).allocate_pool)(efi::LOADER_DATA, bytes, &mut original) };
        if status.is_error() || original.is_null() {
            self.used.fetch_sub(bytes, Ordering::SeqCst);
            return ptr::null_mut();
        }
        let start = original as usize + core::mem::size_of::<AllocationHeader>();
        let aligned = (start + layout.align() - 1) & !(layout.align() - 1);
        let header = (aligned - core::mem::size_of::<AllocationHeader>()) as *mut AllocationHeader;
        unsafe { ptr::write_unaligned(header, AllocationHeader { original, bytes }) };
        aligned as *mut u8
    }

    unsafe fn dealloc(&self, allocation: *mut u8, _layout: Layout) {
        let services = self.services.load(Ordering::SeqCst);
        // Once exited, loader allocations belong to the receiving OS. Calling
        // FreePool would be invalid; deliberately leave reclamation to that OS.
        if services.is_null() || allocation.is_null() {
            return;
        }
        let header = unsafe {
            ptr::read_unaligned(
                allocation
                    .sub(core::mem::size_of::<AllocationHeader>())
                    .cast::<AllocationHeader>(),
            )
        };
        let status = unsafe { ((*services).free_pool)(header.original) };
        if !status.is_error() {
            self.used.fetch_sub(header.bytes, Ordering::SeqCst);
        }
    }
}

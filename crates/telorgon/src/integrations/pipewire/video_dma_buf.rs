//! Linux DMA-BUF descriptor and synchronization boundary, independent of compositor features.
use super::MediaError;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
#[repr(C)]
struct SyncFile {
    flags: u32,
    fd: i32,
}
/// Snapshot all producer write fences for a read. The caller must retain the native buffer
/// lease until its GPU reads complete; exporting fences does not pin content ownership.
pub(crate) fn read_fence(fd: BorrowedFd<'_>) -> Result<OwnedFd, MediaError> {
    export_fence(fd, 1)
}
pub(crate) fn write_fence(fd: BorrowedFd<'_>) -> Result<OwnedFd, MediaError> {
    export_fence(fd, 3)
}
fn export_fence(fd: BorrowedFd<'_>, flags: u32) -> Result<OwnedFd, MediaError> {
    let mut sync = SyncFile { flags, fd: -1 };
    // SAFETY: Linux DMA-BUF ioctl with exact UAPI layout and borrowed live FD. libc computes
    // the ioctl number for the target architecture instead of assuming asm-generic layout.
    if unsafe {
        libc::ioctl(
            fd.as_raw_fd(),
            libc::_IOWR::<SyncFile>(b'b' as u32, 2),
            &mut sync,
        )
    } != 0
    {
        return Err(super::connection::native(std::io::Error::last_os_error()));
    }
    if sync.fd < 0 {
        return Err(MediaError::InvalidArgument("DMA-BUF read fence"));
    }
    // SAFETY: successful ioctl transferred a new owning sync-file FD.
    Ok(unsafe { OwnedFd::from_raw_fd(sync.fd) })
}
pub(crate) fn allocation_size(fd: BorrowedFd<'_>) -> Result<u64, MediaError> {
    // DMA-BUF UAPI explicitly supports SEEK_END(0) for allocation size. It has no stream
    // data/file offset used by consumers. Never trust SPA chunk.size/maxsize for DMA-BUF.
    let size = unsafe { libc::lseek(fd.as_raw_fd(), 0, libc::SEEK_END) };
    if size <= 0 {
        return Err(super::connection::native(std::io::Error::last_os_error()));
    }
    Ok(size as u64)
}

/// Wait only on a native worker. No output overwrite is permitted before all previous
/// implicit readers/writers complete; EINTR must not release that ownership obligation.
pub(crate) fn wait_fence(fd: BorrowedFd<'_>) -> Result<(), MediaError> {
    let mut event = libc::pollfd {
        fd: fd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    loop {
        let result = unsafe { libc::poll(&mut event, 1, -1) };
        if result < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(super::connection::native(error));
        }
        if event.revents & (libc::POLLERR | libc::POLLNVAL | libc::POLLHUP) != 0 {
            return Err(MediaError::Native("DMA-BUF consumer fence failed".into()));
        }
        if event.revents & libc::POLLIN != 0 {
            return Ok(());
        }
    }
}

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
pub(super) struct Events(OwnedFd);
impl Events {
    pub fn open() -> Option<Self> {
        // SAFETY: scalar socket arguments; success creates an owned fd.
        let raw = unsafe {
            libc::socket(
                libc::AF_NETLINK,
                libc::SOCK_DGRAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
                libc::NETLINK_KOBJECT_UEVENT,
            )
        };
        if raw < 0 {
            return None;
        }
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let mut address: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        address.nl_family = libc::AF_NETLINK as u16;
        address.nl_groups = 1;
        // SAFETY: initialized sockaddr and correct byte size; no borrowed buffer is retained.
        if unsafe {
            libc::bind(
                fd.as_raw_fd(),
                (&address as *const libc::sockaddr_nl).cast(),
                std::mem::size_of_val(&address) as libc::socklen_t,
            )
        } != 0
        {
            return None;
        }
        Some(Self(fd))
    }
    pub fn changed(&self) -> bool {
        let mut changed = false;
        for _ in 0..32 {
            let mut message = [0_u8; 4096];
            let mut source: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
            let mut length = std::mem::size_of_val(&source) as libc::socklen_t;
            // SAFETY: owned fd and valid output buffers live throughout recvfrom.
            let count = unsafe {
                libc::recvfrom(
                    self.0.as_raw_fd(),
                    message.as_mut_ptr().cast(),
                    message.len(),
                    libc::MSG_DONTWAIT | libc::MSG_TRUNC,
                    (&mut source as *mut libc::sockaddr_nl).cast(),
                    &mut length,
                )
            };
            if count < 0 {
                if std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOBUFS) {
                    changed = true;
                }
                break;
            }
            if source.nl_pid != 0 {
                continue;
            }
            if count as usize > message.len() {
                changed = true;
                continue;
            }
            changed |= message[..count as usize]
                .split(|b| *b == 0)
                .any(|field| field == b"SUBSYSTEM=backlight");
        }
        changed
    }
}

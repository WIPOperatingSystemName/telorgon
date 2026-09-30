//! Read-only kernel uevents. Payloads are refresh hints, never paths or battery readings.

use std::{
    io,
    mem::{self, size_of, size_of_val},
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    sync::Arc,
};

const KERNEL_GROUP: u32 = 1;
const MESSAGE_BYTES: usize = 8192;
const DRAIN_BUDGET: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::services::battery) enum Notification {
    Changed,
    Resync,
    Stopped,
}

pub(in crate::services::battery) struct ShutdownWake(OwnedFd);

impl ShutdownWake {
    fn new() -> io::Result<Self> {
        // SAFETY: eventfd takes only scalar arguments; a successful fd is newly owned.
        owned_fd(unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) }).map(Self)
    }

    pub(in crate::services::battery) fn notify(&self) {
        let value = 1_u64;
        loop {
            // SAFETY: the Arc-held fd stays open; value is a valid 8-byte input buffer.
            let written = unsafe {
                libc::write(
                    self.0.as_raw_fd(),
                    (&value as *const u64).cast(),
                    size_of::<u64>(),
                )
            };
            if written >= 0 || io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                // EAGAIN means the counter is already readable. Shutdown is also recorded
                // in the owner's atomic flag; repeated stop requests do not write again.
                return;
            }
        }
    }
}

pub(in crate::services::battery) struct Notifications {
    socket: OwnedFd,
    shutdown: Arc<ShutdownWake>,
}

impl Notifications {
    #[cfg(test)]
    pub(in crate::services::battery) fn test_socket(socket: OwnedFd) -> Self {
        Self {
            socket,
            shutdown: Arc::new(ShutdownWake::new().unwrap()),
        }
    }

    pub(in crate::services::battery) fn new() -> io::Result<Self> {
        // SAFETY: socket takes scalar arguments; ownership transfers only on success.
        let socket = owned_fd(unsafe {
            libc::socket(
                libc::AF_NETLINK,
                libc::SOCK_DGRAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
                libc::NETLINK_KOBJECT_UEVENT,
            )
        })?;
        let enabled: libc::c_int = 1;
        // SAFETY: enabled points to an initialized int for exactly the supplied length.
        let result = unsafe {
            libc::setsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PASSCRED,
                (&enabled as *const libc::c_int).cast(),
                size_of::<libc::c_int>() as libc::socklen_t,
            )
        };
        syscall_result(result)?;
        // SAFETY: sockaddr_nl is a plain C struct whose zero representation is valid.
        let mut address: libc::sockaddr_nl = unsafe { mem::zeroed() };
        address.nl_family = libc::AF_NETLINK as libc::sa_family_t;
        // Let the kernel allocate a unique port, including for multiple monitors in one process.
        address.nl_pid = 0;
        address.nl_groups = KERNEL_GROUP;
        // SAFETY: address is an initialized sockaddr_nl with its exact size; fd is live.
        let result = unsafe {
            libc::bind(
                socket.as_raw_fd(),
                (&address as *const libc::sockaddr_nl).cast(),
                size_of::<libc::sockaddr_nl>() as libc::socklen_t,
            )
        };
        syscall_result(result)?;
        Ok(Self {
            socket,
            shutdown: Arc::new(ShutdownWake::new()?),
        })
    }

    pub(in crate::services::battery) fn shutdown_wake(&self) -> Arc<ShutdownWake> {
        self.shutdown.clone()
    }

    pub(in crate::services::battery) fn wait(
        &mut self,
        stopped: impl Fn() -> bool,
    ) -> io::Result<Notification> {
        loop {
            if stopped() {
                return Ok(Notification::Stopped);
            }
            match wait_readable(&self.socket, &self.shutdown) {
                Ok(false) => return Ok(Notification::Stopped),
                Ok(true) => {}
                // Return to the stop check on EINTR, including under repeated signals.
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
            let mut refresh = None;
            // A finite drain budget coalesces bursts while keeping stop checks responsive.
            for _ in 0..DRAIN_BUDGET {
                if stopped() {
                    return Ok(Notification::Stopped);
                }
                match receive(&self.socket) {
                    Ok(Some(Notification::Resync)) => refresh = Some(Notification::Resync),
                    Ok(Some(Notification::Changed)) if refresh.is_none() => {
                        refresh = Some(Notification::Changed);
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) if error.raw_os_error() == Some(libc::ENOBUFS) => {
                        refresh = Some(Notification::Resync);
                    }
                    Err(error) => return Err(error),
                }
            }
            if let Some(refresh) = refresh {
                return Ok(refresh);
            }
        }
    }
}

fn wait_readable(socket: &OwnedFd, shutdown: &ShutdownWake) -> io::Result<bool> {
    let mut descriptors = [
        libc::pollfd {
            fd: socket.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd: shutdown.0.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    loop {
        // SAFETY: both fds remain owned throughout poll; the array has two writable entries.
        // No periodic timeout: shutdown readiness or a kernel notification wakes the worker.
        let result = unsafe { libc::poll(descriptors.as_mut_ptr(), 2, -1) };
        if result < 0 {
            let error = io::Error::last_os_error();
            return Err(error);
        }
        // Shutdown wins when both fds are readable. Treat unexpected wake-fd failure as stop.
        if descriptors[1].revents != 0 {
            return Ok(false);
        }
        if descriptors[0].revents & (libc::POLLHUP | libc::POLLNVAL) != 0 {
            return Err(io::Error::other("battery notification socket closed"));
        }
        // POLLERR can represent ENOBUFS: recvmsg must consume it and request a resync.
        if descriptors[0].revents & (libc::POLLIN | libc::POLLERR) != 0 {
            return Ok(true);
        }
    }
}

fn receive(socket: &OwnedFd) -> io::Result<Option<Notification>> {
    let mut payload = [0_u8; MESSAGE_BYTES];
    // Native-word alignment is sufficient for cmsghdr. The buffer holds one ucred + padding.
    let mut control = [0_usize; 8];
    // SAFETY: these C structs have valid all-zero representations; output buffers stay live.
    let mut address: libc::sockaddr_nl = unsafe { mem::zeroed() };
    let mut message: libc::msghdr = unsafe { mem::zeroed() };
    let mut vector = libc::iovec {
        iov_base: payload.as_mut_ptr().cast(),
        iov_len: payload.len(),
    };
    message.msg_name = (&mut address as *mut libc::sockaddr_nl).cast();
    message.msg_namelen = size_of::<libc::sockaddr_nl>() as libc::socklen_t;
    message.msg_iov = &mut vector;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = size_of_val(&control);
    // SAFETY: every pointer refers to writable storage of the supplied size. MSG_DONTWAIT
    // prevents a receive race from blocking shutdown; the fd remains owned by this worker.
    let received = unsafe { libc::recvmsg(socket.as_raw_fd(), &mut message, libc::MSG_DONTWAIT) };
    if received < 0 {
        return Err(io::Error::last_os_error());
    }
    if message.msg_controllen > size_of_val(&control) {
        return Ok(None);
    }
    // SAFETY: control is initialized storage; the kernel-reported length was bounded above.
    let control = unsafe {
        std::slice::from_raw_parts(control.as_ptr().cast::<u8>(), message.msg_controllen)
    };
    let flags = if received as usize > payload.len() {
        message.msg_flags | libc::MSG_TRUNC
    } else {
        message.msg_flags
    };
    Ok(classify_message(
        &address,
        message.msg_namelen,
        flags,
        control,
        &payload[..(received as usize).min(payload.len())],
    ))
}

fn classify_message(
    address: &libc::sockaddr_nl,
    length: libc::socklen_t,
    flags: libc::c_int,
    control: &[u8],
    payload: &[u8],
) -> Option<Notification> {
    if !kernel_sender(address, length, flags, control) {
        return None;
    }
    if flags & libc::MSG_TRUNC != 0 {
        // Authenticate before reacting to loss, so forged/truncated user packets cannot resync.
        return Some(Notification::Resync);
    }
    power_supply_event(payload).then_some(Notification::Changed)
}

fn kernel_sender(
    address: &libc::sockaddr_nl,
    length: libc::socklen_t,
    flags: libc::c_int,
    control: &[u8],
) -> bool {
    if length as usize != size_of::<libc::sockaddr_nl>()
        || address.nl_family != libc::AF_NETLINK as libc::sa_family_t
        || address.nl_pid != 0
        || address.nl_groups != KERNEL_GROUP
        || flags & libc::MSG_CTRUNC != 0
    {
        return false;
    }
    let mut offset = 0;
    let mut authenticated = false;
    while control.len().saturating_sub(offset) >= size_of::<libc::cmsghdr>() {
        // SAFETY: a full header fits in the bounded slice; read_unaligned avoids alignment
        // assumptions for the byte slice. Lengths are checked before reading credential data.
        let header = unsafe {
            control
                .as_ptr()
                .add(offset)
                .cast::<libc::cmsghdr>()
                .read_unaligned()
        };
        let length = header.cmsg_len as usize;
        if length < size_of::<libc::cmsghdr>() || length > control.len() - offset {
            return false;
        }
        if header.cmsg_level == libc::SOL_SOCKET && header.cmsg_type == libc::SCM_CREDENTIALS {
            if authenticated || length != size_of::<libc::cmsghdr>() + size_of::<libc::ucred>() {
                return false;
            }
            // SAFETY: header and credential sizes were bounded against this control message.
            let credentials = unsafe {
                control
                    .as_ptr()
                    .add(offset + size_of::<libc::cmsghdr>())
                    .cast::<libc::ucred>()
                    .read_unaligned()
            };
            // Kernel-generated uevents carry zero credentials. Checking pid also excludes
            // privileged userspace events forwarded by the kernel with nl_pid rewritten to 0.
            if credentials.pid != 0 || credentials.uid != 0 || credentials.gid != 0 {
                return false;
            }
            authenticated = true;
        }
        let Some(aligned) = length.checked_add(size_of::<usize>() - 1) else {
            return false;
        };
        offset += aligned & !(size_of::<usize>() - 1);
    }
    authenticated
        && control
            .get(offset..)
            .is_none_or(|tail| tail.iter().all(|byte| *byte == 0))
}

fn power_supply_event(payload: &[u8]) -> bool {
    let Some(payload) = payload.strip_suffix(&[0]) else {
        return false;
    };
    let mut fields = payload.split(|byte| *byte == 0);
    let Some(header) = fields.next() else {
        return false;
    };
    let Some(separator) = header.iter().position(|byte| *byte == b'@') else {
        return false;
    };
    let action = &header[..separator];
    let path = &header[separator + 1..];
    if !matches!(action, b"add" | b"remove" | b"change") || !path.starts_with(b"/devices/") {
        return false;
    }
    let mut found_action = false;
    let mut found_path = false;
    let mut found_subsystem = false;
    for field in fields {
        if let Some(value) = field.strip_prefix(b"ACTION=") {
            if found_action || value != action {
                return false;
            }
            found_action = true;
        } else if let Some(value) = field.strip_prefix(b"DEVPATH=") {
            if found_path || value != path {
                return false;
            }
            found_path = true;
        } else if let Some(value) = field.strip_prefix(b"SUBSYSTEM=") {
            if found_subsystem || value != b"power_supply" {
                return false;
            }
            found_subsystem = true;
        } else if field.is_empty() || !field.contains(&b'=') {
            return false;
        }
    }
    found_action && found_path && found_subsystem
}

fn owned_fd(fd: libc::c_int) -> io::Result<OwnedFd> {
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        // SAFETY: callers pass only newly-created successful descriptors, transferring ownership.
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }
}

fn syscall_result(result: libc::c_int) -> io::Result<()> {
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;

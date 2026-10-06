use crate::screen_brightness::ScreenBrightnessError as Error;
use std::{
    io::{Read, Write},
    os::{fd::AsRawFd, unix::net::UnixStream},
    time::Instant,
};

// Nonblocking socket I/O and an absolute deadline bound the whole frame, including
// partial reads/writes. Per-syscall socket timeouts would let trickle traffic extend it.
fn wait(channel: &UnixStream, events: i16, deadline: Option<Instant>) -> Result<(), Error> {
    loop {
        let timeout = match deadline {
            Some(deadline) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(Error::Transport);
                }
                remaining.as_millis().clamp(1, i32::MAX as u128) as i32
            }
            None => -1,
        };
        let mut descriptor = libc::pollfd {
            fd: channel.as_raw_fd(),
            events,
            revents: 0,
        };
        // SAFETY: one live socket and one correctly sized writable pollfd record.
        let result = unsafe { libc::poll(&mut descriptor, 1, timeout) };
        if result > 0 && descriptor.revents & libc::POLLNVAL == 0 {
            return Ok(());
        }
        if result < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        return Err(Error::Transport);
    }
}
pub(super) fn read(
    channel: &mut UnixStream,
    mut bytes: &mut [u8],
    deadline: Option<Instant>,
    idle_eof: bool,
) -> Result<bool, Error> {
    let mut first = true;
    while !bytes.is_empty() {
        if deadline.is_some_and(|end| Instant::now() >= end) {
            return Err(Error::Transport);
        }
        match channel.read(bytes) {
            Ok(0) if first && idle_eof => return Ok(false),
            Ok(0) => return Err(Error::Transport),
            Ok(count) => {
                first = false;
                bytes = &mut bytes[count..];
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                wait(channel, libc::POLLIN, deadline)?
            }
            Err(_) => return Err(Error::Transport),
        }
    }
    Ok(true)
}
pub(super) fn write(
    channel: &mut UnixStream,
    mut bytes: &[u8],
    deadline: Instant,
) -> Result<(), Error> {
    while !bytes.is_empty() {
        if Instant::now() >= deadline {
            return Err(Error::Transport);
        }
        match channel.write(bytes) {
            Ok(0) => return Err(Error::Transport),
            Ok(count) => bytes = &bytes[count..],
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                wait(channel, libc::POLLOUT, Some(deadline))?
            }
            Err(_) => return Err(Error::Transport),
        }
    }
    Ok(())
}

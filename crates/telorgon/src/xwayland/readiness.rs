//! Bounded nonblocking parser for Xwayland's dedicated -displayfd pipe.
//! This establishes only the server half of the lifecycle readiness barrier.
use super::{Error, Result};
use std::{
    fs::File,
    io::{self, Read},
    os::fd::{AsRawFd, OwnedFd, RawFd},
};

pub struct DisplayNotification {
    pipe: File,
    expected: Vec<u8>,
    received: usize,
    failed: bool,
}

impl DisplayNotification {
    pub fn new(pipe: OwnedFd, display: u16) -> Result<Self> {
        let flags = unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_GETFL) };
        if flags < 0
            || unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
        {
            return Err(io::Error::last_os_error().into());
        }
        Ok(Self {
            pipe: pipe.into(),
            expected: format!("{display}\n").into_bytes(),
            received: 0,
            failed: false,
        })
    }

    pub fn fd(&self) -> RawFd {
        self.pipe.as_raw_fd()
    }

    /// True only for the exact reserved display number and newline. Partial
    /// reads return false; EOF, mismatches and extra bytes fail without logging
    /// the received contents. The lifecycle owns the startup timeout.
    pub fn dispatch(&mut self) -> Result<bool> {
        if self.failed {
            return Err(Error("Xwayland display notification has failed".into()));
        }
        if self.received == self.expected.len() {
            return Ok(true);
        }
        let mut bytes = [0u8; 8];
        // A single bounded read per turn; EINTR simply reschedules via readiness.
        match self.pipe.read(&mut bytes) {
            Ok(n)
                if n > 0
                    && self.expected.get(self.received..self.received + n) == Some(&bytes[..n]) =>
            {
                self.received += n;
                Ok(self.received == self.expected.len())
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                Ok(false)
            }
            _ => {
                self.failed = true;
                Err(Error(
                    "invalid or closed Xwayland display notification".into(),
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, os::unix::net::UnixStream};
    #[test]
    fn fragmented_notification_does_not_establish_early_readiness() {
        let (read, mut write) = UnixStream::pair().unwrap();
        let mut notification = DisplayNotification::new(read.into(), 123).unwrap();
        assert!(!notification.dispatch().unwrap());
        write.write_all(b"12").unwrap();
        assert!(!notification.dispatch().unwrap());
        write.write_all(b"3\n").unwrap();
        assert!(notification.dispatch().unwrap());
        drop(write);
        assert!(notification.dispatch().unwrap());
    }
    #[test]
    fn mismatches_extra_bytes_and_premature_eof_are_terminal() {
        for input in [b"12\n".as_slice(), b"1\nx", b"01\n", b"", b"1"] {
            let (read, mut write) = UnixStream::pair().unwrap();
            let mut notification = DisplayNotification::new(read.into(), 1).unwrap();
            write.write_all(input).unwrap();
            drop(write);
            let result = notification.dispatch();
            if matches!(result, Ok(false)) {
                assert!(notification.dispatch().is_err());
            } else {
                assert!(result.is_err());
            }
            assert!(notification.dispatch().is_err());
        }
    }
}

use super::*;
use std::{
    io::{Read, Write},
    os::fd::FromRawFd,
    time::{Duration, Instant},
};
const TIMEOUT: Duration = Duration::from_secs(5);
pub(super) fn pipe() -> std::io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [-1; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}
fn ready(fd: i32, events: i16, cancel: &AtomicBool, deadline: Instant) -> service::Result<()> {
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err(ClipboardError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(ClipboardError::Timeout);
        }
        let mut poll = libc::pollfd {
            fd,
            events,
            revents: 0,
        };
        let result = unsafe { libc::poll(&mut poll, 1, 50) };
        if result > 0 {
            return Ok(());
        }
        if result < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            return Err(ClipboardError::TransferFailed);
        }
    }
}
pub(super) fn read_pipe(fd: OwnedFd, max: usize, reply: ReadResponse) {
    read_pipe_result(fd, max, reply, None);
}
fn read_pipe_result(
    fd: OwnedFd,
    max: usize,
    reply: ReadResponse,
    provider: Option<std::sync::mpsc::Receiver<service::Result<()>>>,
) {
    std::thread::spawn(move || {
        let mut file = std::fs::File::from(fd);
        let deadline = Instant::now() + TIMEOUT;
        let result = (|| {
            let mut data = Vec::new();
            let mut buffer = vec![0; reply.chunk_size()];
            let mut count_total = 0;
            loop {
                ready(file.as_raw_fd(), libc::POLLIN, &reply.cancel(), deadline)?;
                match file.read(&mut buffer) {
                    Ok(0) => {
                        if let Some(provider) = &provider {
                            loop {
                                if reply.cancel().load(Ordering::Acquire) {
                                    return Err(ClipboardError::Cancelled);
                                }
                                if Instant::now() >= deadline {
                                    return Err(ClipboardError::Timeout);
                                }
                                match provider.recv_timeout(Duration::from_millis(50)) {
                                    Ok(result) => {
                                        result?;
                                        break;
                                    }
                                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                                    Err(_) => return Err(ClipboardError::TransferFailed),
                                }
                            }
                        }
                        return Ok(data);
                    }
                    Ok(count) => {
                        if count_total + count > max {
                            return Err(ClipboardError::TooLarge);
                        }
                        reply.chunk(&buffer[..count], &mut data)?;
                        count_total += count;
                    }
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                        ) => {}
                    Err(_) => return Err(ClipboardError::TransferFailed),
                }
            }
        })();
        reply.finish(result);
    });
}
struct BoundedWriter<'a> {
    output: Box<dyn Write>,
    fd: Option<i32>,
    max: usize,
    written: usize,
    cancel: &'a AtomicBool,
    deadline: Instant,
    failure: Option<ClipboardError>,
}
impl Write for BoundedWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.cancel.load(Ordering::Acquire) || Instant::now() >= self.deadline {
            self.failure = Some(if self.cancel.load(Ordering::Acquire) {
                ClipboardError::Cancelled
            } else {
                ClipboardError::Timeout
            });
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        if bytes.len() > self.max.saturating_sub(self.written) {
            self.failure = Some(ClipboardError::TooLarge);
            return Err(std::io::ErrorKind::FileTooLarge.into());
        }
        loop {
            if let Some(fd) = self.fd {
                ready(fd, libc::POLLOUT, self.cancel, self.deadline)
                    .map_err(std::io::Error::other)?;
            }
            match self.output.write(bytes) {
                Ok(count) => {
                    self.written += count;
                    return Ok(count);
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) => {}
                Err(e) => return Err(e),
            }
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub(super) fn write_pipe(
    fd: OwnedFd,
    content: ClipboardContent,
    format: DataFormat,
    cancel: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        let raw = fd.as_raw_fd();
        let flags = unsafe { libc::fcntl(raw, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(raw, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return;
        }
        let mut writer = BoundedWriter {
            output: Box::new(std::fs::File::from(fd)),
            fd: Some(raw),
            max: service::MAX_BYTES,
            written: 0,
            cancel: &cancel,
            deadline: Instant::now() + TIMEOUT,
            failure: None,
        };
        let _ = content.provider.write(&format, &mut writer, &cancel);
    });
}
pub(super) fn read_provider(
    content: ClipboardContent,
    format: DataFormat,
    max: usize,
    reply: ReadResponse,
) {
    match pipe() {
        Ok((reader, writer)) => {
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            let cancel = reply.cancel();
            std::thread::spawn(move || {
                let raw = writer.as_raw_fd();
                let mut writer = BoundedWriter {
                    output: Box::new(std::fs::File::from(writer)),
                    fd: Some(raw),
                    max,
                    written: 0,
                    cancel: &cancel,
                    deadline: Instant::now() + TIMEOUT,
                    failure: None,
                };
                let result = content.provider.write(&format, &mut writer, &cancel);
                let result = writer.failure.clone().map_or(result, Err);
                let _ = tx.send(result);
            });
            read_pipe_result(reader, max, reply, Some(rx));
        }
        Err(_) => reply.finish(Err(ClipboardError::TransferFailed)),
    }
}

use std::io;
use std::os::fd::{AsRawFd, BorrowedFd};
use std::time::Duration;

use super::{
    LinuxSplashConfig, LinuxSplashExit, LinuxSplashResult, MAX_MILESTONE_LINE_BYTES,
    MilestoneError, SplashMilestone, SplashRuntime,
};
use crate::authoring::compose::Component;
use crate::boot::BootSession;

const FRAME_INTERVAL: Duration = Duration::from_millis(33);

/// Animates the shared OS splash on one thread while polling an exclusively read milestone fd.
/// Input is accumulated into bounded records, so an incomplete line never blocks animation.
/// The caller retains fd ownership and must not read it concurrently. This does not change its
/// shared file flags. An EOF returns `InputClosed`, preserves the last frame, and leaves OS
/// completion unconfirmed; only an explicit `ready` milestone publishes completion.
pub fn run_fd<C: Component>(
    session: BootSession<C>,
    config: LinuxSplashConfig,
    milestones: BorrowedFd<'_>,
) -> LinuxSplashResult<LinuxSplashExit> {
    let mut host = SplashRuntime::new(session, config)?;
    let result = (|| {
        host.paint()?;
        let mut schedule = FrameSchedule::new(FRAME_INTERVAL);
        let mut decoder = Decoder::new();
        let mut input = [0u8; 4096];
        loop {
            if schedule.take_due(host.elapsed()) {
                host.paint()?;
            }
            if !wait_input(milestones, schedule.timeout_ms(host.elapsed()))
                .map_err(MilestoneError::from)?
            {
                continue;
            }
            // poll established readability or a hangup. With an exclusive reader, a blocking
            // fd returns currently available bytes; partial records are held in our own buffer.
            let read = unsafe {
                libc::read(
                    milestones.as_raw_fd(),
                    input.as_mut_ptr().cast(),
                    input.len(),
                )
            };
            if read < 0 {
                let error = io::Error::last_os_error();
                if matches!(
                    error.kind(),
                    io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                ) {
                    continue;
                }
                return Err(MilestoneError::Io(error).into());
            }
            if read == 0 {
                let decoded = decoder.finish(|milestone| host.apply(milestone))?;
                if decoded.updated {
                    host.paint()?;
                }
                return Ok(decoded.exit.unwrap_or(LinuxSplashExit::InputClosed));
            }
            let decoded =
                decoder.feed(&input[..read as usize], |milestone| host.apply(milestone))?;
            if decoded.updated {
                host.paint()?;
            }
            if let Some(exit) = decoded.exit {
                return Ok(exit);
            }
        }
    })();
    host.finish(result)
}

fn wait_input(input: BorrowedFd<'_>, timeout_ms: i32) -> io::Result<bool> {
    let mut descriptor = libc::pollfd {
        fd: input.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    // The stack descriptor remains alive for poll, and no fd is created, closed or reconfigured.
    let count = unsafe { libc::poll(&mut descriptor, 1, timeout_ms) };
    if count < 0 {
        let error = io::Error::last_os_error();
        return if error.kind() == io::ErrorKind::Interrupted {
            Ok(false)
        } else {
            Err(error)
        };
    }
    if descriptor.revents & libc::POLLNVAL != 0 {
        return Err(io::Error::from_raw_os_error(libc::EBADF));
    }
    let readable = descriptor.revents & (libc::POLLIN | libc::POLLHUP) != 0;
    if descriptor.revents & libc::POLLERR != 0 && !readable {
        return Err(io::Error::from_raw_os_error(libc::EIO));
    }
    Ok(readable)
}

struct FrameSchedule {
    next: Duration,
    interval: Duration,
}

impl FrameSchedule {
    fn new(interval: Duration) -> Self {
        Self {
            next: interval,
            interval,
        }
    }

    fn take_due(&mut self, now: Duration) -> bool {
        if now < self.next {
            return false;
        }
        // Skip missed frames rather than queuing bursts after a slow render or delayed input.
        self.next = now.saturating_add(self.interval);
        true
    }

    fn timeout_ms(&self, now: Duration) -> i32 {
        let nanos = self.next.saturating_sub(now).as_nanos();
        i32::try_from(nanos.div_ceil(1_000_000)).unwrap_or(i32::MAX)
    }
}

#[derive(Default)]
struct Decoded {
    updated: bool,
    exit: Option<LinuxSplashExit>,
}

struct Decoder {
    pending: Vec<u8>,
}

impl Decoder {
    fn new() -> Self {
        Self {
            pending: Vec::with_capacity(MAX_MILESTONE_LINE_BYTES),
        }
    }

    fn feed(
        &mut self,
        bytes: &[u8],
        mut emit: impl FnMut(SplashMilestone) -> LinuxSplashResult<Option<LinuxSplashExit>>,
    ) -> LinuxSplashResult<Decoded> {
        let mut result = Decoded::default();
        for byte in bytes {
            if self.pending.len() == MAX_MILESTONE_LINE_BYTES {
                return Err(MilestoneError::Invalid("line exceeds 1024 bytes").into());
            }
            self.pending.push(*byte);
            if *byte == b'\n' {
                if let Some(milestone) = self.record()? {
                    result.updated = true;
                    if let Some(exit) = emit(milestone)? {
                        result.exit = Some(exit);
                        return Ok(result);
                    }
                }
            }
        }
        Ok(result)
    }

    fn finish(
        &mut self,
        mut emit: impl FnMut(SplashMilestone) -> LinuxSplashResult<Option<LinuxSplashExit>>,
    ) -> LinuxSplashResult<Decoded> {
        if self.pending.is_empty() {
            return Ok(Decoded::default());
        }
        match self.record()? {
            Some(milestone) => Ok(Decoded {
                updated: true,
                exit: emit(milestone)?,
            }),
            None => Ok(Decoded::default()),
        }
    }

    fn record(&mut self) -> Result<Option<SplashMilestone>, MilestoneError> {
        let line = std::str::from_utf8(&self.pending)
            .map_err(|_| MilestoneError::Invalid("line is not UTF-8"))?;
        let milestone = SplashMilestone::parse(line)?;
        self.pending.clear();
        Ok(milestone)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boot::{BootApplication, BootPhase, BootTarget};

    #[test]
    fn partial_input_does_not_emit_or_prevent_frame_deadlines() {
        let mut decoder = Decoder::new();
        let mut records = Vec::new();
        let partial = decoder
            .feed(b"status Mount", |milestone| {
                records.push(milestone);
                Ok(None)
            })
            .unwrap();
        assert!(!partial.updated);
        assert!(records.is_empty());
        let mut schedule = FrameSchedule::new(FRAME_INTERVAL);
        assert_eq!(schedule.timeout_ms(Duration::from_millis(10)), 23);
        assert!(schedule.take_due(Duration::from_millis(33)));
        assert_eq!(schedule.timeout_ms(Duration::from_millis(33)), 33);
        assert!(schedule.take_due(Duration::from_secs(1)));
        assert!(!schedule.take_due(Duration::from_secs(1)));
        decoder
            .feed(b"ing root\nprogress 1 3\n", |milestone| {
                records.push(milestone);
                Ok(None)
            })
            .unwrap();
        assert_eq!(
            records,
            [
                SplashMilestone::Status("Mounting root".into()),
                SplashMilestone::Progress {
                    completed: 1,
                    total: 3
                }
            ]
        );
    }

    #[test]
    fn ready_stops_before_trailing_records_and_decoder_bounds_unterminated_input() {
        let mut decoder = Decoder::new();
        let result = decoder
            .feed(b"ready\ninvalid trailing data\n", |milestone| {
                assert_eq!(milestone, SplashMilestone::Ready);
                Ok(Some(LinuxSplashExit::Ready))
            })
            .unwrap();
        assert_eq!(result.exit, Some(LinuxSplashExit::Ready));
        let mut decoder = Decoder::new();
        assert!(
            decoder
                .feed(&vec![b'x'; MAX_MILESTONE_LINE_BYTES + 1], |_| Ok(None))
                .is_err()
        );
    }

    #[test]
    fn eof_flushes_status_without_fabricating_os_completion() {
        let session = BootApplication::new()
            .target(BootTarget::linux("linux", "Linux").unwrap())
            .build()
            .unwrap()
            .into_splash_session("linux")
            .unwrap();
        let request = session.active_request().unwrap();
        let mut decoder = Decoder::new();
        decoder
            .feed(b"status Mounting root", |_| {
                panic!("partial record emitted")
            })
            .unwrap();
        let result = decoder
            .finish(|milestone| {
                super::super::apply_milestone(&session.controller, request, milestone)
            })
            .unwrap();
        assert!(result.updated);
        assert_eq!(
            result.exit.unwrap_or(LinuxSplashExit::InputClosed),
            LinuxSplashExit::InputClosed
        );
        assert_eq!(session.controller.snapshot().phase, BootPhase::OsStarting);
        assert_eq!(session.controller.snapshot().status, "Mounting root");
    }
}

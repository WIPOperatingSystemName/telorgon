//! A trusted launcher supplies a private socket fd and an explicit connector allowlist.
//! This process is deliberately not installed, elevated, or started by the SDK.
use std::{
    os::{fd::FromRawFd, unix::net::UnixStream},
    time::Duration,
};
use telorgon::{
    platform::linux::screen_brightness::ddc::*, screen_brightness::ScreenBrightnessLevel,
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let fd: i32 = arguments
        .next()
        .ok_or("expected private socket fd, session PID, seat, and DRM connectors")?
        .parse()?;
    let session_pid = arguments.next().ok_or("missing session PID")?.parse()?;
    let seat = arguments.next().ok_or("missing seat")?;
    if fd < 3 {
        return Err("broker socket must be a dedicated inherited fd".into());
    }
    // SAFETY: scalar descriptor inspection; verify it is live before taking ownership.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err("broker socket must be a live descriptor".into());
    }
    let monitors = arguments
        .map(|connector| ScreenBrightnessDdcMonitorConfig {
            label: connector.clone(),
            connector,
        })
        .collect();
    // SAFETY: the launcher transfers unique ownership of the dedicated inherited Unix fd.
    // SO_PEERCRED and protocol/session checks run before any client operation is dispatched.
    let channel = unsafe { UnixStream::from_raw_fd(fd) };
    serve_screen_brightness_ddc(
        channel,
        ScreenBrightnessDdcBrokerConfig {
            session_pid,
            seat,
            monitors,
            timeout: Duration::from_secs(5),
            minimum: ScreenBrightnessLevel::percent(1.0)?,
            allow_zero: false,
        },
    )?;
    Ok(())
}

//! Explicit admission of one user's Wayland clients by a persistent system shell.
use std::{collections::VecDeque, fmt, io, os::{fd::AsRawFd, unix::net::UnixListener}, sync::{Arc, Mutex, mpsc}};

pub(crate) struct Request {
    pub listener: Option<(UnixListener, u32)>,
    pub reply: mpsc::Sender<io::Result<()>>,
}
#[derive(Default)]
struct State {
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
    pending: VecDeque<Request>,
}
#[derive(Clone, Default)]
pub struct WaylandSessionControl(Arc<Mutex<State>>);
impl fmt::Debug for WaylandSessionControl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("WaylandSessionControl").finish_non_exhaustive()
    }
}
impl PartialEq for WaylandSessionControl {
    fn eq(&self, other: &Self) -> bool { Arc::ptr_eq(&self.0, &other.0) }
}
impl WaylandSessionControl {
    /// Call from a controller thread, never the compositor owner thread.
    /// The socket's filesystem policy belongs to the caller; the host additionally
    /// checks kernel peer credentials and disconnects the previous session's clients.
    pub fn attach(&self, listener: UnixListener, uid: u32) -> io::Result<()> {
        if uid == 0 { return Err(io::Error::new(io::ErrorKind::PermissionDenied, "A user session must be unprivileged")); }
        let mut listening: libc::c_int = 0;
        let mut length = size_of_val(&listening) as libc::socklen_t;
        // Received descriptors may originate in an unprivileged helper. Reject
        // non-listening descriptors before registering them with the event loop.
        if unsafe { libc::getsockopt(listener.as_raw_fd(), libc::SOL_SOCKET, libc::SO_ACCEPTCONN,
            std::ptr::from_mut(&mut listening).cast(), &mut length) } != 0 {
            return Err(io::Error::last_os_error());
        }
        if listening != 1 { return Err(io::Error::new(io::ErrorKind::InvalidInput, "Expected a listening socket")); }
        listener.set_nonblocking(true)?;
        self.request(Some((listener, uid)))
    }
    /// Returns after the host has stopped admission and destroyed all session clients.
    /// Before host startup there are no clients to revoke.
    pub fn detach(&self) -> io::Result<()> { self.request(None) }
    fn request(&self, listener: Option<(UnixListener, u32)>) -> io::Result<()> {
        let (reply, receive) = mpsc::channel();
        let wake = {
            let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
            let Some(wake) = state.wake.clone() else {
                return if listener.is_none() { Ok(()) } else { Err(io::Error::new(io::ErrorKind::NotConnected, "Wayland host is not running")) };
            };
            if state.pending.len() >= 8 { return Err(io::Error::new(io::ErrorKind::WouldBlock, "Too many session changes")); }
            state.pending.push_back(Request { listener, reply });
            wake
        };
        wake();
        receive.recv().unwrap_or_else(|_| Err(io::Error::new(io::ErrorKind::BrokenPipe, "Wayland host stopped")))
    }
    pub(crate) fn bind(&self, wake: Arc<dyn Fn() + Send + Sync>) {
        self.0.lock().unwrap_or_else(|error| error.into_inner()).wake = Some(wake);
    }
    pub(crate) fn take(&self) -> VecDeque<Request> {
        std::mem::take(&mut self.0.lock().unwrap_or_else(|error| error.into_inner()).pending)
    }
    pub(crate) fn unbind(&self) {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        state.wake = None;
        state.pending.clear();
    }
}

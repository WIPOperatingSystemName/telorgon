use super::{event_source::{EventNotifier, mark_external_fd_ready}, *};
use crate::host::application::WaylandSessionControl;
use crate::integrations::wayland::server::{EventSource, OwnedClient};
use std::{io, os::unix::net::{UnixListener, UnixStream}};

struct Active {
    // Remove the event source before closing its FD or freeing callback storage.
    _source: EventSource,
    listener: UnixListener,
    ready: Box<AtomicBool>,
    uid: u32,
    clients: Vec<OwnedClient>,
}
pub(super) struct ClientSessions {
    control: WaylandSessionControl,
    active: Option<Active>,
}
impl ClientSessions {
    pub fn new(control: WaylandSessionControl, wake: EventNotifier) -> Self {
        control.bind(Arc::new(move || wake.notify()));
        Self { control, active: None }
    }
    pub fn apply(&mut self, display: &Display) {
        for request in self.control.take() {
            let replacement = request.listener.map(|(listener, uid)| {
                let ready = Box::new(AtomicBool::new(false));
                // Callback storage is boxed and outlives the registered event source.
                let source = unsafe { display.event_loop().add_fd(
                    listener.as_raw_fd(), 1, Some(mark_external_fd_ready),
                    std::ptr::from_ref(ready.as_ref()).cast_mut().cast(),
                ) }.map_err(|error| io::Error::other(error.to_string()))?;
                Ok(Active { _source: source, listener, ready, uid, clients: Vec::new() })
            }).transpose();
            match replacement {
                Ok(active) => { self.active = active; let _ = request.reply.send(Ok(())); }
                Err(error) => { let _ = request.reply.send(Err(error)); }
            }
        }
    }
    pub fn accept(&mut self, display: &Display) {
        let Some(active) = &mut self.active else { return };
        active.clients.retain(OwnedClient::is_alive);
        if !active.ready.swap(false, Ordering::AcqRel) { return; }
        // Bound work and live connections so one user's connection storm cannot
        // monopolize the compositor. Level-triggered readiness preserves the rest.
        for _ in 0..64 {
            let socket = match active.listener.accept() {
                Ok((socket, _)) => socket,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => { eprintln!("telorgon-session: accept failed: {error}"); break; }
            };
            if peer_uid(&socket).ok() != Some(active.uid) || active.clients.len() >= 256 { continue; }
            match display.create_client(socket) {
                Ok(client) => active.clients.push(client),
                Err(error) => eprintln!("telorgon-session: client creation failed: {error}"),
            }
        }
    }
}
impl Drop for ClientSessions {
    fn drop(&mut self) {
        self.control.unbind();
        self.active.take();
    }
}
fn peer_uid(socket: &UnixStream) -> io::Result<u32> {
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of_val(&credentials) as libc::socklen_t;
    // Linux fills the fixed-size ucred from the connected socket's kernel identity.
    if unsafe { libc::getsockopt(socket.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED,
        std::ptr::from_mut(&mut credentials).cast(), &mut length) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if length as usize != std::mem::size_of_val(&credentials) { return Err(io::Error::other("Invalid peer credentials")); }
    Ok(credentials.uid)
}

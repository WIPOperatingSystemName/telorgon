use std::cell::{Cell, UnsafeCell};
use std::ffi::{CStr, CString, c_int, c_void};
use std::marker::PhantomData;
use std::os::{
    fd::{AsRawFd, IntoRawFd},
    unix::net::UnixStream,
};
use std::ptr::NonNull;
use std::rc::Rc;
use std::time::Duration;

use crate::wayland_server::ffi;
use crate::wayland_server::{WaylandServerError, WaylandServerErrorKind};

type ServerResult<T> = Result<T, WaylandServerError>;

type FilterFn = dyn Fn(usize, *const ffi::wl_interface) -> bool;
struct GlobalFilter(Box<FilterFn>);

unsafe extern "C" fn filter_global(
    client: *const ffi::wl_client,
    global: *const ffi::wl_global,
    data: *mut c_void,
) -> bool {
    let filter = unsafe { &*data.cast::<GlobalFilter>() };
    let interface = unsafe { ffi::wl_global_get_interface(global) };
    // A policy panic must not unwind through C or accidentally grant access.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        (filter.0)(client as usize, interface)
    }))
    .unwrap_or(false)
}

pub struct Display {
    raw: NonNull<ffi::wl_display>,
    filter: Option<Box<GlobalFilter>>,
    marker: PhantomData<Rc<()>>,
}

impl Display {
    /// Install an owned owner-thread policy before creating protocol globals.
    /// libwayland applies it both to registry advertisements and bind requests.
    /// Interface pointers are opaque identities; this function grants no access
    /// to their storage. Panicking policies deny access. Native clients should
    /// remain allowed for every unrestricted interface.
    pub fn set_global_filter(
        &mut self,
        filter: impl Fn(usize, *const ffi::wl_interface) -> bool + 'static,
    ) {
        let mut filter = Box::new(GlobalFilter(Box::new(filter)));
        unsafe {
            ffi::wl_display_set_global_filter(
                self.raw.as_ptr(),
                Some(filter_global),
                (&mut *filter as *mut GlobalFilter).cast(),
            );
        }
        self.filter = Some(filter);
    }
    /// Register a compositor-created connection. The returned owner tracks
    /// libwayland destruction and is safe to retain beyond display teardown.
    /// Dropping it disconnects only this client. No credentials imply privilege;
    /// authorization must compare the live identity of this dedicated handle.
    pub fn create_client(&self, socket: UnixStream) -> ServerResult<OwnedClient> {
        let lifetime = Box::new(ClientLifetime {
            listener: UnsafeCell::new(ffi::wl_listener {
                link: ffi::wl_list {
                    prev: std::ptr::null_mut(),
                    next: std::ptr::null_mut(),
                },
                notify: Some(client_destroyed),
            }),
            raw: Cell::new(None),
        });
        let raw = unsafe { ffi::wl_client_create(self.raw.as_ptr(), socket.as_raw_fd()) };
        let raw = NonNull::new(raw).ok_or_else(|| {
            WaylandServerError::new(
                WaylandServerErrorKind::Allocation,
                "libwayland could not create the private client",
            )
        })?;
        // Official API: ownership transfers only on success. Failure leaves the
        // Rust socket responsible for closing the endpoint.
        let _ = socket.into_raw_fd();
        lifetime.raw.set(Some(raw));
        unsafe {
            ffi::wl_client_add_destroy_listener(raw.as_ptr(), lifetime.listener.get());
        }
        Ok(OwnedClient {
            lifetime,
            marker: PhantomData,
        })
    }
    pub fn new() -> ServerResult<Self> {
        let raw = unsafe { ffi::wl_display_create() };
        let raw = NonNull::new(raw).ok_or_else(|| {
            WaylandServerError::new(
                WaylandServerErrorKind::Allocation,
                "libwayland could not allocate the server display",
            )
        })?;
        Ok(Self {
            raw,
            filter: None,
            marker: PhantomData,
        })
    }

    pub fn add_socket_auto(&self) -> ServerResult<String> {
        let name = unsafe { ffi::wl_display_add_socket_auto(self.raw.as_ptr()) };
        let name = NonNull::new(name.cast_mut()).ok_or_else(|| {
            WaylandServerError::new(
                WaylandServerErrorKind::Socket,
                "libwayland could not create a display socket",
            )
        })?;
        let name = unsafe { CStr::from_ptr(name.as_ptr()) };
        Ok(name.to_string_lossy().into_owned())
    }

    pub fn add_socket(&self, name: &str) -> ServerResult<()> {
        let name = CString::new(name).map_err(|_| {
            WaylandServerError::new(
                WaylandServerErrorKind::Socket,
                "Wayland socket name contains an interior NUL",
            )
        })?;
        let result = unsafe { ffi::wl_display_add_socket(self.raw.as_ptr(), name.as_ptr()) };
        native_zero(
            result,
            WaylandServerErrorKind::Socket,
            "libwayland could not create the requested display socket",
        )
    }

    /// Bind in a validated runtime directory without mutating XDG_RUNTIME_DIR. Libwayland owns
    /// socket locking, stale-socket handling and unlinking for each successful absolute-path bind.
    pub fn add_socket_in(
        &self,
        directory: &std::path::Path,
        name: Option<&str>,
    ) -> ServerResult<String> {
        let candidates: Vec<String> = match name {
            Some(name) => vec![name.to_owned()],
            None => (0..=32).map(|index| format!("wayland-{index}")).collect(),
        };
        let mut last_error = None;
        for name in candidates {
            if name.is_empty()
                || name.contains(['/', '\\'])
                || name == "."
                || name == ".."
                || !directory.is_absolute()
            {
                return Err(WaylandServerError::new(
                    WaylandServerErrorKind::Socket,
                    "invalid runtime directory or socket name",
                ));
            }
            let path = directory.join(&name);
            let path = path.to_str().ok_or_else(|| {
                WaylandServerError::new(
                    WaylandServerErrorKind::Socket,
                    "Wayland socket path is not UTF-8",
                )
            })?;
            match self.add_socket(path) {
                Ok(()) => return Ok(name),
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.expect("socket candidates are nonempty"))
    }

    pub fn event_loop(&self) -> EventLoopRef<'_> {
        let raw = unsafe { ffi::wl_display_get_event_loop(self.raw.as_ptr()) };
        EventLoopRef {
            raw: NonNull::new(raw).expect("a live Wayland display always owns an event loop"),
            marker: PhantomData,
        }
    }

    pub fn flush_clients(&self) {
        unsafe { ffi::wl_display_flush_clients(self.raw.as_ptr()) };
    }

    /// Flushes queued events, dispatches one event-loop turn, and immediately flushes replies.
    ///
    /// Keeping both flushes at this boundary prevents the loop from blocking with events queued
    /// by either the previous owner turn or newly dispatched client requests.
    pub fn dispatch_and_flush(&self, timeout: Option<Duration>) -> ServerResult<()> {
        self.flush_clients();
        self.event_loop().dispatch(timeout)?;
        self.flush_clients();
        Ok(())
    }

    pub fn current_serial(&self) -> u32 {
        unsafe { ffi::wl_display_get_serial(self.raw.as_ptr()) }
    }

    pub fn next_serial(&self) -> u32 {
        unsafe { ffi::wl_display_next_serial(self.raw.as_ptr()) }
    }

    /// Native display identity for Telorgon-owned protocol layers that must allocate serials from
    /// the same libwayland sequence while handling a C callback.
    #[doc(hidden)]
    pub fn native_handle(&self) -> NonNull<ffi::wl_display> {
        self.raw
    }

    pub fn terminate(&self) {
        unsafe { ffi::wl_display_terminate(self.raw.as_ptr()) };
    }

    /// Creates a native global whose callback data and interface descriptor are borrowed from the
    /// caller.
    ///
    /// # Safety
    ///
    /// `interface`, `data`, and all storage reachable by `bind` must remain valid until the
    /// returned global is destroyed. The callback must obey the libwayland server ABI and must not
    /// unwind.
    pub unsafe fn create_global<'display>(
        &'display self,
        interface: &ffi::wl_interface,
        version: u32,
        data: *mut c_void,
        bind: ffi::wl_global_bind_func_t,
    ) -> ServerResult<Global<'display>> {
        if version == 0 || version > interface.version as u32 || version > c_int::MAX as u32 {
            return Err(WaylandServerError::new(
                WaylandServerErrorKind::InvalidVersion,
                "Wayland global version is outside the interface contract",
            ));
        }
        let raw = unsafe {
            ffi::wl_global_create(self.raw.as_ptr(), interface, version as c_int, data, bind)
        };
        Ok(Global {
            raw: NonNull::new(raw).ok_or_else(|| {
                WaylandServerError::new(
                    WaylandServerErrorKind::Allocation,
                    "libwayland could not allocate a global",
                )
            })?,
            marker: PhantomData,
        })
    }
}

impl Drop for Display {
    fn drop(&mut self) {
        unsafe {
            ffi::wl_display_destroy_clients(self.raw.as_ptr());
            ffi::wl_display_destroy(self.raw.as_ptr());
        }
    }
}

pub struct EventLoopRef<'display> {
    raw: NonNull<ffi::wl_event_loop>,
    marker: PhantomData<&'display Display>,
}

impl EventLoopRef<'_> {
    pub fn dispatch(&self, timeout: Option<Duration>) -> ServerResult<()> {
        let timeout_ms = match timeout {
            None => -1,
            Some(duration) => i32::try_from(duration.as_millis()).map_err(|_| {
                WaylandServerError::new(
                    WaylandServerErrorKind::InvalidTimeout,
                    "Wayland event-loop timeout exceeds i32 milliseconds",
                )
            })?,
        };
        let result = unsafe { ffi::wl_event_loop_dispatch(self.raw.as_ptr(), timeout_ms) };
        if result < 0 {
            Err(WaylandServerError::new(
                WaylandServerErrorKind::Dispatch,
                "libwayland event-loop dispatch failed",
            ))
        } else {
            Ok(())
        }
    }

    pub fn dispatch_idle(&self) {
        unsafe { ffi::wl_event_loop_dispatch_idle(self.raw.as_ptr()) };
    }

    /// Registers an externally owned file descriptor with this event loop.
    ///
    /// # Safety
    ///
    /// `fd` must remain valid until the returned source is removed. `data` must remain valid for
    /// every callback, and `callback` must obey the libwayland ABI and must not unwind.
    pub unsafe fn add_fd(
        &self,
        fd: c_int,
        mask: u32,
        callback: ffi::wl_event_loop_fd_func_t,
        data: *mut c_void,
    ) -> ServerResult<EventSource> {
        let raw = unsafe { ffi::wl_event_loop_add_fd(self.raw.as_ptr(), fd, mask, callback, data) };
        EventSource::from_raw(raw)
    }

    /// Registers a timer callback with this event loop.
    ///
    /// # Safety
    ///
    /// `data` must remain valid until the returned source is removed. `callback` must obey the
    /// libwayland ABI and must not unwind.
    pub unsafe fn add_timer(
        &self,
        callback: ffi::wl_event_loop_timer_func_t,
        data: *mut c_void,
    ) -> ServerResult<EventSource> {
        let raw = unsafe { ffi::wl_event_loop_add_timer(self.raw.as_ptr(), callback, data) };
        EventSource::from_raw(raw)
    }
}

pub struct EventSource {
    raw: NonNull<ffi::wl_event_source>,
    marker: PhantomData<Rc<()>>,
}

impl EventSource {
    fn from_raw(raw: *mut ffi::wl_event_source) -> ServerResult<Self> {
        Ok(Self {
            raw: NonNull::new(raw).ok_or_else(|| {
                WaylandServerError::new(
                    WaylandServerErrorKind::Allocation,
                    "libwayland could not allocate an event source",
                )
            })?,
            marker: PhantomData,
        })
    }

    pub fn update_fd_mask(&self, mask: u32) -> ServerResult<()> {
        let result = unsafe { ffi::wl_event_source_fd_update(self.raw.as_ptr(), mask) };
        native_zero(
            result,
            WaylandServerErrorKind::NativeFailure,
            "libwayland could not update the event-source mask",
        )
    }

    pub fn arm_timer(&self, delay: Duration) -> ServerResult<()> {
        let delay_ms = i32::try_from(delay.as_millis()).map_err(|_| {
            WaylandServerError::new(
                WaylandServerErrorKind::InvalidTimeout,
                "Wayland timer delay exceeds i32 milliseconds",
            )
        })?;
        let result = unsafe { ffi::wl_event_source_timer_update(self.raw.as_ptr(), delay_ms) };
        native_zero(
            result,
            WaylandServerErrorKind::NativeFailure,
            "libwayland could not arm the event-source timer",
        )
    }
}

impl Drop for EventSource {
    fn drop(&mut self) {
        let result = unsafe { ffi::wl_event_source_remove(self.raw.as_ptr()) };
        debug_assert_eq!(result, 0, "libwayland event-source removal failed");
    }
}

pub struct Global<'display> {
    raw: NonNull<ffi::wl_global>,
    marker: PhantomData<&'display Display>,
}

impl Global<'_> {
    pub fn remove(&self) {
        unsafe { ffi::wl_global_remove(self.raw.as_ptr()) };
    }
}

impl Drop for Global<'_> {
    fn drop(&mut self) {
        unsafe { ffi::wl_global_destroy(self.raw.as_ptr()) };
    }
}

// Listener is first so the C callback can recover this stable allocation.
// UnsafeCell permits libwayland's intrusive-list mutations through a shared owner.
#[repr(C)]
struct ClientLifetime {
    listener: UnsafeCell<ffi::wl_listener>,
    raw: Cell<Option<NonNull<ffi::wl_client>>>,
}

unsafe extern "C" fn client_destroyed(listener: *mut ffi::wl_listener, _: *mut c_void) {
    let lifetime = unsafe { &*listener.cast::<ClientLifetime>() };
    lifetime.raw.set(None);
    // Detach while the signal/list still exists; never touch the link later.
    unsafe {
        ffi::wl_list_remove(&mut (*listener).link);
    }
}

/// Owner-thread client lifetime, distinct from a callback-scoped ClientRef.
/// identity() becomes None on disconnect, protocol error or display destruction;
/// an old handle can never authenticate a newly allocated client at the same address.
pub struct OwnedClient {
    lifetime: Box<ClientLifetime>,
    marker: PhantomData<Rc<()>>,
}

impl OwnedClient {
    pub fn identity(&self) -> Option<usize> {
        self.lifetime.raw.get().map(|raw| raw.as_ptr() as usize)
    }
    pub fn is_alive(&self) -> bool {
        self.lifetime.raw.get().is_some()
    }
    pub fn matches(&self, client: ClientRef<'_>) -> bool {
        self.identity() == Some(client.identity())
    }
    pub fn disconnect(&self) {
        if let Some(raw) = self.lifetime.raw.get() {
            unsafe {
                ffi::wl_client_destroy(raw.as_ptr());
            }
        }
    }
}

impl Drop for OwnedClient {
    fn drop(&mut self) {
        self.disconnect();
    }
}

#[derive(Clone, Copy)]
pub struct ClientRef<'callback> {
    raw: NonNull<ffi::wl_client>,
    marker: PhantomData<&'callback mut ffi::wl_client>,
}

impl<'callback> ClientRef<'callback> {
    /// Borrows a client pointer supplied by a live libwayland callback.
    ///
    /// # Safety
    ///
    /// `raw` must either be null or identify a live `wl_client` for the entire `'callback`
    /// lifetime. The caller must prevent destruction while a returned reference is in use.
    pub unsafe fn from_raw(raw: *mut ffi::wl_client) -> Option<Self> {
        NonNull::new(raw).map(|raw| Self {
            raw,
            marker: PhantomData,
        })
    }

    pub fn credentials(self) -> ClientCredentials {
        let mut pid = 0;
        let mut uid = 0;
        let mut gid = 0;
        unsafe { ffi::wl_client_get_credentials(self.raw.as_ptr(), &mut pid, &mut uid, &mut gid) };
        ClientCredentials { pid, uid, gid }
    }

    pub fn identity(self) -> usize {
        self.raw.as_ptr() as usize
    }

    /// Creates a resource owned by this client.
    ///
    /// # Safety
    ///
    /// `interface` and every descriptor/string/type pointer reachable from it must remain valid
    /// until the resource is destroyed. The client must still be live.
    pub unsafe fn create_resource(
        self,
        interface: &ffi::wl_interface,
        version: u32,
        id: u32,
    ) -> ServerResult<ResourceRef<'callback>> {
        if version == 0 || version > interface.version as u32 || version > c_int::MAX as u32 {
            return Err(WaylandServerError::new(
                WaylandServerErrorKind::InvalidVersion,
                "Wayland resource version is outside the interface contract",
            ));
        }
        let raw =
            unsafe { ffi::wl_resource_create(self.raw.as_ptr(), interface, version as c_int, id) };
        unsafe { ResourceRef::from_raw(raw) }.ok_or_else(|| {
            WaylandServerError::new(
                WaylandServerErrorKind::Allocation,
                "libwayland could not allocate a resource",
            )
        })
    }

    pub fn object(self, id: u32) -> Option<ResourceRef<'callback>> {
        let raw = unsafe { ffi::wl_client_get_object(self.raw.as_ptr(), id) };
        unsafe { ResourceRef::from_raw(raw) }
    }

    pub fn flush(self) {
        unsafe { ffi::wl_client_flush(self.raw.as_ptr()) };
    }

    pub fn post_no_memory(self) {
        unsafe { ffi::wl_client_post_no_memory(self.raw.as_ptr()) };
    }

    pub fn disconnect(self) {
        unsafe { ffi::wl_client_destroy(self.raw.as_ptr()) };
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClientCredentials {
    pub pid: i32,
    pub uid: u32,
    pub gid: u32,
}

#[derive(Clone, Copy)]
pub struct ResourceRef<'callback> {
    raw: NonNull<ffi::wl_resource>,
    marker: PhantomData<&'callback mut ffi::wl_resource>,
}

impl<'callback> ResourceRef<'callback> {
    /// Borrows a resource pointer supplied by a live libwayland callback or lookup.
    ///
    /// # Safety
    ///
    /// `raw` must either be null or identify a live `wl_resource` for the entire `'callback`
    /// lifetime. The caller must prevent destruction while a returned reference is in use.
    pub unsafe fn from_raw(raw: *mut ffi::wl_resource) -> Option<Self> {
        NonNull::new(raw).map(|raw| Self {
            raw,
            marker: PhantomData,
        })
    }

    pub fn id(self) -> u32 {
        unsafe { ffi::wl_resource_get_id(self.raw.as_ptr()) }
    }

    pub fn identity(self) -> usize {
        self.raw.as_ptr() as usize
    }

    pub fn version(self) -> u32 {
        unsafe { ffi::wl_resource_get_version(self.raw.as_ptr()) as u32 }
    }

    pub fn client(self) -> ClientRef<'callback> {
        let raw = unsafe { ffi::wl_resource_get_client(self.raw.as_ptr()) };
        unsafe { ClientRef::from_raw(raw) }.expect("a live Wayland resource always has a client")
    }

    pub fn user_data(self) -> *mut c_void {
        unsafe { ffi::wl_resource_get_user_data(self.raw.as_ptr()) }
    }

    /// Replaces the opaque callback data associated with this resource.
    ///
    /// # Safety
    ///
    /// `data` must remain valid for every native access until it is replaced or the resource is
    /// destroyed.
    pub unsafe fn set_user_data(self, data: *mut c_void) {
        unsafe { ffi::wl_resource_set_user_data(self.raw.as_ptr(), data) };
    }

    /// Installs the native request dispatcher and destruction callback for this resource.
    ///
    /// # Safety
    ///
    /// The function pointers must obey the libwayland ABI and must not unwind. `implementation`
    /// and `data` must remain valid until destruction, and `destroy` must release them exactly once
    /// when ownership requires it.
    pub unsafe fn set_dispatcher(
        self,
        dispatcher: ffi::wl_dispatcher_func_t,
        implementation: *const c_void,
        data: *mut c_void,
        destroy: ffi::wl_resource_destroy_func_t,
    ) {
        unsafe {
            ffi::wl_resource_set_dispatcher(
                self.raw.as_ptr(),
                dispatcher,
                implementation,
                data,
                destroy,
            )
        };
    }

    pub fn post_error(self, code: u32, message: &str) {
        let message = CString::new(message).unwrap_or_else(|_| {
            CString::new("Telorgon rejected a malformed Wayland request").unwrap()
        });
        const STRING_FORMAT: &[u8] = b"%s\0";
        unsafe {
            ffi::wl_resource_post_error(
                self.raw.as_ptr(),
                code,
                STRING_FORMAT.as_ptr().cast(),
                message.as_ptr(),
            )
        };
    }

    /// Posts one protocol event using the argument layout for `opcode`.
    ///
    /// # Safety
    ///
    /// `opcode` must select an event on this resource's negotiated interface and `arguments` must
    /// exactly match its native signature. Every pointer/FD referenced by the argument union must
    /// satisfy libwayland's lifetime and ownership rules for the duration of the call.
    pub unsafe fn post_event(self, opcode: u32, arguments: &mut [ffi::wl_argument]) {
        unsafe {
            ffi::wl_resource_post_event_array(self.raw.as_ptr(), opcode, arguments.as_mut_ptr())
        };
    }

    pub fn post_no_memory(self) {
        unsafe { ffi::wl_resource_post_no_memory(self.raw.as_ptr()) };
    }

    /// Destroys this resource through libwayland.
    ///
    /// # Safety
    ///
    /// No further use may be made of this value or any alias after the call. The resource must not
    /// already have been destroyed by client teardown or a reentrant callback.
    pub unsafe fn destroy(self) {
        unsafe { ffi::wl_resource_destroy(self.raw.as_ptr()) };
    }
}

fn native_zero(
    result: c_int,
    kind: WaylandServerErrorKind,
    message: &'static str,
) -> ServerResult<()> {
    if result == 0 {
        Ok(())
    } else {
        Err(WaylandServerError::new(kind, message))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn restricted_global_is_hidden_and_guessed_bind_is_rejected() {
        let mut display = Display::new().unwrap();
        let (mut allowed_peer, socket) = UnixStream::pair().unwrap();
        let allowed = Rc::new(display.create_client(socket).unwrap());
        let (mut denied_peer, socket) = UnixStream::pair().unwrap();
        let denied = display.create_client(socket).unwrap();
        let interface = unsafe { &ffi::wl_compositor_interface as *const _ };
        let policy_client = allowed.clone();
        display.set_global_filter(move |client, candidate| {
            candidate != interface || policy_client.identity() == Some(client)
        });
        let _global = unsafe {
            display.create_global(
                &ffi::wl_compositor_interface,
                1,
                std::ptr::null_mut(),
                Some(ignore_global_bind),
            )
        }
        .unwrap();
        for peer in [&mut allowed_peer, &mut denied_peer] {
            peer.set_read_timeout(Some(Duration::from_millis(250)))
                .unwrap();
            peer.write_all(&display_request(1, 1, 2)).unwrap();
            peer.write_all(&display_request(1, 0, 3)).unwrap();
        }
        display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
        let mut visible = [0; 48];
        allowed_peer.read_exact(&mut visible).unwrap();
        assert_eq!(u32::from_ne_bytes(visible[..4].try_into().unwrap()), 2);
        let global_name = u32::from_ne_bytes(visible[8..12].try_into().unwrap());
        let mut hidden = [0; 12];
        denied_peer.read_exact(&mut hidden).unwrap();
        // Only sync completion, with no registry advertisement preceding it.
        assert_eq!(u32::from_ne_bytes(hidden[..4].try_into().unwrap()), 3);
        let mut bind = [0u8; 40];
        bind[..4].copy_from_slice(&2u32.to_ne_bytes());
        bind[4..8].copy_from_slice(&(40u32 << 16).to_ne_bytes());
        bind[8..12].copy_from_slice(&global_name.to_ne_bytes());
        bind[12..16].copy_from_slice(&14u32.to_ne_bytes());
        bind[16..30].copy_from_slice(b"wl_compositor\0");
        bind[32..36].copy_from_slice(&1u32.to_ne_bytes());
        bind[36..40].copy_from_slice(&4u32.to_ne_bytes());
        denied_peer.write_all(&bind).unwrap();
        display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
        assert!(!denied.is_alive());
        assert!(allowed.is_alive());
        allowed.disconnect();
        assert_eq!(allowed.identity(), None);
        let (mut replacement_peer, socket) = UnixStream::pair().unwrap();
        let _replacement = display.create_client(socket).unwrap();
        replacement_peer
            .write_all(&display_request(1, 1, 2))
            .unwrap();
        replacement_peer
            .write_all(&display_request(1, 0, 3))
            .unwrap();
        display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
        replacement_peer
            .set_read_timeout(Some(Duration::from_millis(250)))
            .unwrap();
        replacement_peer.read_exact(&mut hidden).unwrap();
        assert_eq!(u32::from_ne_bytes(hidden[..4].try_into().unwrap()), 3);
    }

    #[test]
    fn global_filter_policy_panic_fails_closed() {
        let mut display = Display::new().unwrap();
        display.set_global_filter(|_, _| panic!("policy fixture"));
        let _global = unsafe {
            display.create_global(
                &ffi::wl_compositor_interface,
                1,
                std::ptr::null_mut(),
                Some(ignore_global_bind),
            )
        }
        .unwrap();
        let (mut peer, socket) = UnixStream::pair().unwrap();
        let _client = display.create_client(socket).unwrap();
        peer.write_all(&display_request(1, 1, 2)).unwrap();
        peer.write_all(&display_request(1, 0, 3)).unwrap();
        display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
        peer.set_read_timeout(Some(Duration::from_millis(250)))
            .unwrap();
        let mut reply = [0; 12];
        peer.read_exact(&mut reply).unwrap();
        assert_eq!(u32::from_ne_bytes(reply[..4].try_into().unwrap()), 3);
    }
    #[test]
    fn owned_client_tracks_peer_and_display_destruction() {
        let display = Display::new().unwrap();
        let (peer, socket) = UnixStream::pair().unwrap();
        let client = display.create_client(socket).unwrap();
        assert!(client.identity().is_some());
        drop(peer);
        display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
        assert_eq!(client.identity(), None);
        client.disconnect();
        let (_peer, socket) = UnixStream::pair().unwrap();
        let replacement = display.create_client(socket).unwrap();
        assert!(replacement.is_alive());
        assert!(!client.is_alive());
        drop(display);
        assert!(!replacement.is_alive());
        replacement.disconnect();
    }

    #[test]
    fn dropping_owned_client_closes_only_its_connection() {
        let display = Display::new().unwrap();
        let (mut peer, socket) = UnixStream::pair().unwrap();
        let client = display.create_client(socket).unwrap();
        let (_other_peer, other_socket) = UnixStream::pair().unwrap();
        let other = display.create_client(other_socket).unwrap();
        drop(client);
        peer.set_read_timeout(Some(Duration::from_millis(250)))
            .unwrap();
        assert_eq!(peer.read(&mut [0]).unwrap(), 0);
        assert!(other.is_alive());
    }
    use super::*;

    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    fn display_request(object: u32, opcode: u16, new_id: u32) -> [u8; 12] {
        let mut bytes = [0_u8; 12];
        bytes[0..4].copy_from_slice(&object.to_ne_bytes());
        bytes[4..8].copy_from_slice(&(((12_u32) << 16) | u32::from(opcode)).to_ne_bytes());
        bytes[8..12].copy_from_slice(&new_id.to_ne_bytes());
        bytes
    }

    unsafe extern "C" fn ignore_global_bind(
        _client: *mut ffi::wl_client,
        _data: *mut c_void,
        _version: u32,
        _id: u32,
    ) {
    }

    #[test]
    fn timeout_conversion_rejects_values_beyond_native_range_without_dispatching() {
        let oversized = Duration::from_millis(i32::MAX as u64 + 1);
        assert!(i32::try_from(oversized.as_millis()).is_err());
    }

    #[test]
    fn wrapper_types_remain_owner_thread_values() {
        fn assert_not_copy<T>() {}
        assert_not_copy::<Display>();
        assert_not_copy::<EventSource>();
    }

    #[test]
    fn late_client_registry_sync_is_flushed_without_an_external_wake() {
        let display = Display::new().unwrap();
        let _global = unsafe {
            display.create_global(
                &ffi::wl_compositor_interface,
                1,
                std::ptr::null_mut(),
                Some(ignore_global_bind),
            )
        }
        .unwrap();
        let (mut client_socket, server_socket) = UnixStream::pair().unwrap();
        let client = display.create_client(server_socket).unwrap();
        assert!(client.is_alive());

        client_socket.write_all(&display_request(1, 1, 2)).unwrap();
        client_socket.write_all(&display_request(1, 0, 3)).unwrap();

        display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();

        client_socket
            .set_read_timeout(Some(Duration::from_millis(250)))
            .unwrap();
        let mut replies = [0_u8; 48];
        client_socket.read_exact(&mut replies).unwrap();

        let registry_object = u32::from_ne_bytes(replies[0..4].try_into().unwrap());
        let registry_header = u32::from_ne_bytes(replies[4..8].try_into().unwrap());
        let interface_length = u32::from_ne_bytes(replies[12..16].try_into().unwrap());
        assert_eq!(registry_object, 2);
        assert_eq!(registry_header & 0xffff, 0);
        assert_eq!(registry_header >> 16, 36);
        assert_eq!(interface_length, 14);
        assert_eq!(&replies[16..29], b"wl_compositor");

        let callback_object = u32::from_ne_bytes(replies[36..40].try_into().unwrap());
        let callback_header = u32::from_ne_bytes(replies[40..44].try_into().unwrap());
        assert_eq!(callback_object, 3);
        assert_eq!(callback_header & 0xffff, 0);
        assert_eq!(callback_header >> 16, 12);
    }
}

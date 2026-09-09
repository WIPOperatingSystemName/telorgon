//! Managed private-server ownership. Rendering/readiness publication is a separate
//! barrier: an initialized XWM alone must not expose DISPLAY to applications.
use super::*;
use crate::{
    compositor_wayland::XwaylandAccess,
    wayland_server::{EventSource, OwnedClient},
    xwayland::{
        process::{Command, PreparedServer, Status, Supervisor},
        readiness::DisplayNotification,
        xwm::Xwm,
    },
};
use std::{
    rc::Rc,
    sync::mpsc::{self, Receiver, TryRecvError},
};

pub(super) struct Compatibility {
    // Remove callbacks before dropping their data and owned descriptors.
    sources: Vec<EventSource>,
    ready: Box<AtomicBool>,
    preparation: Option<Receiver<crate::xwayland::Result<PreparedServer>>>,
    helper: Option<Supervisor>,
    client: Option<Rc<OwnedClient>>,
    xwm: Option<Xwm>,
    notification: Option<DisplayNotification>,
    access: Rc<XwaylandAccess>,
    deadline: Option<Instant>,
    failed: bool,
    initialized: bool,
    environment: Option<(std::ffi::OsString, std::ffi::OsString)>,
    serials: BTreeMap<u64, (u64, bool)>,
    identities: BTreeSet<crate::xwayland::association::XWindow>,
    descendants: BTreeSet<WaylandSurfaceId>,
    desktop: super::x11_windows::X11Windows,
    policy_repaint: bool,
    closing: BTreeMap<crate::xwayland::association::XWindow, Option<u16>>,
    pending_focus: Option<(
        WaylandSurfaceId,
        crate::xwayland::association::XWindow,
        u16,
        Instant,
        Option<u32>,
    )>,
    focus_result: Option<(
        WaylandSurfaceId,
        crate::xwayland::association::XWindow,
        Instant,
    )>,
}
impl Compatibility {
    #[cfg_attr(not(feature = "desktop-xwayland-embedded"), allow(dead_code))]
    pub(super) fn prepare(
        access: Rc<XwaylandAccess>,
        bytes: &'static [u8],
        cache: std::path::PathBuf,
        runtime: std::path::PathBuf,
        environment: crate::session::Environment,
        wake: EventNotifier,
    ) -> AppResult<Self> {
        let (send, receive) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("telorgon-xwayland-prepare".into())
            .spawn(move || {
                let result = (|| {
                    let payload = Arc::new(crate::xwayland::payload::extract(bytes, &cache)?);
                    let reservation = Arc::new(
                        crate::xwayland::resources::DisplayReservation::reserve(0, 255)?,
                    );
                    let runtime = Arc::new(crate::xwayland::resources::RuntimeFiles::create(
                        &runtime,
                        reservation.number(),
                    )?);
                    Command::embedded(payload, reservation, runtime, environment.0)
                })();
                // If startup was cancelled, dropping the unsent command cleans its resources.
                let _ = send.send(result);
                wake.notify();
            })
            .map_err(app_error)?;
        Ok(Self {
            sources: Vec::new(),
            ready: Box::new(AtomicBool::new(true)),
            preparation: Some(receive),
            helper: None,
            client: None,
            xwm: None,
            notification: None,
            access,
            deadline: Some(Instant::now() + Duration::from_secs(10)),
            failed: false,
            initialized: false,
            environment: None,
            serials: BTreeMap::new(),
            identities: BTreeSet::new(),
            descendants: BTreeSet::new(),
            desktop: Default::default(),
            policy_repaint: false,
            closing: BTreeMap::new(),
            pending_focus: None,
            focus_result: None,
        })
    }
    pub(super) fn committed(&mut self, surface: WaylandSurfaceId, serial: u64) {
        if self.failed {
            return;
        }
        let id = u64::from(surface.get());
        if self.serials.get(&id).is_some_and(|(old, _)| *old == serial) {
            return;
        }
        if self.serials.len() >= 4096 && !self.serials.contains_key(&id) {
            eprintln!("telorgon-xwayland: committed surface bound exceeded");
            self.stop();
            return;
        }
        self.serials.insert(id, (serial, false));
        self.ready.store(true, Ordering::Release);
    }
    pub(super) fn destroyed(&mut self, surface: WaylandSurfaceId) {
        let id = u64::from(surface.get());
        if self.serials.remove(&id).is_none() {
            return;
        }
        if let Some(xwm) = &mut self.xwm {
            if xwm.windows().is_some() && xwm.destroy_surface(1, id).is_err() {
                self.stop();
            }
        }
    }
    /// Apply protocol geometry/eligibility after image completions and before
    /// scene assembly. An unassociated image is retained but never visible.
    pub(super) fn sync_presentation(
        &mut self,
        windows: &mut BTreeMap<WaylandSurfaceId, ClientWindow>,
        identities: &mut WindowIdentities,
        config: &LinuxDesktopConfig,
    ) -> AppResult<bool> {
        let registry = self.xwm.as_ref().and_then(Xwm::windows);
        let mut placements = BTreeMap::new();
        let mut live = BTreeSet::new();
        if let Some(registry) = registry {
            for window in registry.iter() {
                live.insert(window.id);
                let desktop_id = identities.ensure(window.id)?;
                if let Some(surface) = registry
                    .presentable_surface(window.id)
                    .and_then(|raw| u32::try_from(raw).ok())
                    .and_then(WaylandSurfaceId::from_raw)
                {
                    placements.insert(
                        surface,
                        (
                            window.geometry,
                            desktop_id,
                            window.id,
                            window.override_redirect,
                        ),
                    );
                }
            }
        }
        for dead in self.identities.difference(&live) {
            identities.destroy(*dead);
        }
        self.identities = live.clone();
        let mut changed = std::mem::take(&mut self.policy_repaint);
        for (surface, window) in windows
            .iter_mut()
            .filter(|(_, window)| window.role == SurfaceRole::Xwayland)
        {
            if let Some((geometry, id, xwindow, unmanaged)) = placements.get(surface) {
                let before = (
                    window.position,
                    window.requested_size,
                    window.minimized,
                    window.server_decorated,
                    window.desktop_id,
                );
                self.desktop
                    .attach(*xwindow, *surface, *geometry, *unmanaged, window, config);
                window.desktop_id = Some(*id);
                if let Some(xwm) = &self.xwm {
                    let title = xwm.window_title(*xwindow).map(str::to_owned);
                    changed |= window.frame_title != title;
                    window.frame_title = title;
                }
                changed |= before
                    != (
                        window.position,
                        window.requested_size,
                        window.minimized,
                        window.server_decorated,
                        window.desktop_id,
                    );
            } else {
                changed |= !window.minimized || window.desktop_id.is_some();
                window.minimized = true;
                window.desktop_id = None;
                window.backend = None;
                window.resize_preview = Default::default();
                window.server_decorated = false;
            }
        }
        self.desktop.retain(&live);
        // Subsurfaces share their X11 root's eligibility and coordinate space.
        // Compute first, then mutate, so sibling traversal order is irrelevant.
        let children: Vec<_> = windows
            .iter()
            .filter_map(|(surface, window)| {
                if window.role != SurfaceRole::Subsurface {
                    return None;
                }
                let mut candidate = window;
                let mut offset = PointI::default();
                for _ in 0..windows.len() {
                    offset.x = offset.x.saturating_add(candidate.offset.x);
                    offset.y = offset.y.saturating_add(candidate.offset.y);
                    let Some(parent) = candidate.parent.and_then(|parent| windows.get(&parent))
                    else {
                        return self.descendants.contains(surface).then_some((
                            *surface,
                            true,
                            window.position,
                        ));
                    };
                    candidate = parent;
                    if candidate.role == SurfaceRole::Xwayland {
                        return Some((
                            *surface,
                            candidate.minimized,
                            PointI {
                                x: candidate
                                    .position
                                    .x
                                    .saturating_add(window_content_offset(candidate, config).x)
                                    .saturating_add(offset.x),
                                y: candidate
                                    .position
                                    .y
                                    .saturating_add(window_content_offset(candidate, config).y)
                                    .saturating_add(offset.y),
                            },
                        ));
                    }
                    if candidate.role != SurfaceRole::Subsurface {
                        return None;
                    }
                }
                None
            })
            .collect();
        self.descendants = children.iter().map(|(surface, _, _)| *surface).collect();
        for (surface, hidden, position) in children {
            let window = windows.get_mut(&surface).unwrap();
            changed |= window.minimized != hidden || window.position != position;
            window.minimized = hidden;
            window.position = position;
        }
        Ok(changed)
    }
    /// Keep shutdown open while ordinary X11 windows exist, including unmapped
    /// windows. Unknown/unsupported WM_DELETE_WINDOW never authorizes killing.
    pub(super) fn startup_complete(&self) -> bool {
        self.initialized || self.failed
    }
    pub(super) fn environment(&self) -> Option<(std::ffi::OsString, std::ffi::OsString)> {
        if self.initialized && !self.failed {
            self.environment.clone()
        } else {
            None
        }
    }
    pub(super) fn flush_windows(
        &mut self,
        windows: &mut BTreeMap<WaylandSurfaceId, ClientWindow>,
        config: &LinuxDesktopConfig,
    ) {
        if let Some(xwm) = &mut self.xwm {
            match self.desktop.flush(xwm, windows, config) {
                Err(error) => {
                    eprintln!("telorgon-xwayland: window update failed: {error}");
                    self.stop();
                }
                Ok(changed) => {
                    self.policy_repaint |= changed;
                    if changed || xwm.wants_write() {
                        self.ready.store(true, Ordering::Release);
                    }
                }
            }
        }
    }

    pub(super) fn close_surface(&mut self, surface: WaylandSurfaceId) {
        let Some(xwm) = &mut self.xwm else {
            return;
        };
        let Some(window) = xwm.windows().and_then(|registry| {
            registry
                .iter()
                .find(|window| {
                    !window.override_redirect
                        && registry.presentable_surface(window.id) == Some(u64::from(surface.get()))
                })
                .map(|window| window.id)
        }) else {
            return;
        };
        match xwm.request_focus_timestamp(window, Instant::now()) {
            Ok(sequence) => {
                self.closing.insert(window, Some(sequence));
                self.ready.store(true, Ordering::Release);
            }
            Err(error) => eprintln!("telorgon-xwayland: close request failed: {error}"),
        }
    }

    pub(super) fn request_close(&mut self) -> bool {
        let Some(xwm) = &mut self.xwm else {
            return false;
        };
        let live: BTreeSet<_> = xwm
            .windows()
            .map(|registry| {
                registry
                    .iter()
                    .filter(|window| !window.override_redirect)
                    .map(|window| window.id)
                    .collect()
            })
            .unwrap_or_default();
        self.closing.retain(|window, _| live.contains(window));
        let mut waiting = self
            .closing
            .values()
            .filter(|sequence| sequence.is_some())
            .count();
        for window in &live {
            if waiting >= 4 {
                break;
            }
            if self.closing.contains_key(window)
                || !xwm
                    .protocols(*window)
                    .is_some_and(|protocols| protocols.delete_window)
            {
                continue;
            }
            if let Ok(sequence) = xwm.request_focus_timestamp(*window, Instant::now()) {
                self.closing.insert(*window, Some(sequence));
                waiting += 1;
                self.ready.store(true, Ordering::Release);
            }
        }
        !live.is_empty()
    }
    pub(super) fn cancel_close(&mut self) {
        self.closing.clear();
    }
    pub(super) fn request_focus(
        &mut self,
        surface: Option<WaylandSurfaceId>,
        windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
    ) {
        self.pending_focus = None;
        self.focus_result = None;
        let Some(mut surface) = surface else {
            return;
        };
        for _ in 0..windows.len() {
            let Some(image) = windows.get(&surface) else {
                return;
            };
            if image.minimized {
                return;
            }
            if image.role == SurfaceRole::Xwayland {
                break;
            }
            let Some(parent) = image.parent else {
                return;
            };
            surface = parent;
        }
        let Some(xwm) = &mut self.xwm else {
            return;
        };
        let target = xwm.windows().and_then(|registry| {
            registry
                .iter()
                .find(|window| {
                    registry.presentable_surface(window.id) == Some(u64::from(surface.get()))
                })
                .map(|window| window.id)
        });
        let Some(target) = target else {
            return;
        };
        let now = Instant::now();
        match xwm.request_focus_timestamp(target, now) {
            Ok(sequence) => {
                self.pending_focus = Some((
                    surface,
                    target,
                    sequence,
                    now + Duration::from_secs(2),
                    None,
                ));
                self.ready.store(true, Ordering::Release);
            }
            Err(error) => eprintln!("telorgon-xwayland: focus request failed: {error}"),
        }
    }
    pub(super) fn apply_focus(
        &mut self,
        display: &Display,
        wayland: &mut NativeCompositor<'_>,
        windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
        stacking: &mut Vec<WaylandSurfaceId>,
        locked: bool,
    ) -> AppResult<bool> {
        let current = wayland
            .core()
            .seats
            .get(&1)
            .and_then(|seat| seat.keyboard_focus)
            .map(|focus| focus.surface);
        if let Some(surface) = current {
            if windows
                .get(&surface)
                .is_some_and(|image| image.role == SurfaceRole::Xwayland && image.minimized)
            {
                wayland
                    .set_keyboard_focus(1, None, display.next_serial())
                    .map_err(app_error)?;
            }
        }
        if locked || current.is_some() {
            self.pending_focus = None;
            self.focus_result = None;
            return Ok(false);
        }
        let Some((surface, target, deadline)) = self.focus_result else {
            return Ok(false);
        };
        if Instant::now() >= deadline {
            self.focus_result = None;
            return Ok(false);
        }
        if self
            .xwm
            .as_ref()
            .is_none_or(|xwm| xwm.commands_pending(target))
        {
            return Ok(false);
        }
        self.focus_result = None;
        if self
            .xwm
            .as_ref()
            .and_then(Xwm::windows)
            .and_then(|registry| registry.presentable_surface(target))
            != Some(u64::from(surface.get()))
        {
            return Ok(false);
        }
        if !windows
            .get(&surface)
            .is_some_and(|image| image.role == SurfaceRole::Xwayland && !image.minimized)
        {
            return Ok(false);
        }
        wayland
            .set_keyboard_focus(1, Some(surface), display.next_serial())
            .map_err(app_error)?;
        let family: Vec<_> = stacking
            .iter()
            .copied()
            .filter(|candidate| {
                let mut candidate = *candidate;
                for _ in 0..=windows.len() {
                    if candidate == surface {
                        return true;
                    }
                    let Some(parent) = windows.get(&candidate).and_then(|image| image.parent)
                    else {
                        return false;
                    };
                    candidate = parent;
                }
                false
            })
            .collect();
        stacking.retain(|candidate| !family.contains(candidate));
        stacking.extend(family);
        Ok(true)
    }
    pub(super) fn wait(&self, wait: Option<Duration>) -> Option<Duration> {
        if self.ready.load(Ordering::Acquire) {
            return Some(Duration::ZERO);
        }
        let deadline = self
            .deadline
            .into_iter()
            .chain(self.xwm.as_ref().and_then(Xwm::deadline))
            .chain(self.pending_focus.map(|(_, _, _, deadline, _)| deadline))
            .chain(self.focus_result.map(|(_, _, deadline)| deadline))
            .min();
        match (wait, deadline) {
            (wait, Some(deadline)) => Some(
                wait.unwrap_or(Duration::MAX)
                    .min(deadline.saturating_duration_since(Instant::now())),
            ),
            (wait, None) => wait,
        }
    }
    pub(super) fn dispatch(&mut self, display: &Display) {
        if self.failed {
            return;
        }
        if let Err(error) = self.advance(display) {
            eprintln!("telorgon-xwayland: {error}; native desktop remains active");
            self.stop();
        }
    }
    fn advance(&mut self, display: &Display) -> AppResult<()> {
        self.ready.store(false, Ordering::Release);
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Err(AppError::new("private Xwayland startup deadline expired"));
        }
        if let Some(receiver) = &self.preparation {
            match receiver.try_recv() {
                Ok(result) => {
                    self.preparation = None;
                    let PreparedServer {
                        command,
                        environment,
                        wayland,
                        xwm,
                        readiness,
                    } = result.map_err(app_error)?;
                    self.environment = Some(environment);
                    let client = Rc::new(display.create_client(wayland).map_err(app_error)?);
                    self.access
                        .set_client(client.clone(), 1)
                        .map_err(app_error)?;
                    self.client = Some(client);
                    self.xwm = Some(Xwm::new(xwm, 1, Instant::now()).map_err(app_error)?);
                    self.notification = Some(readiness);
                    self.helper = Some(Supervisor::spawn(command).map_err(app_error)?);
                }
                Err(TryRecvError::Empty) => return Ok(()),
                Err(TryRecvError::Disconnected) => {
                    return Err(AppError::new("Xwayland preparation worker disconnected"));
                }
            }
        }
        // Dispatch may close a failed XWM FD. Unregister before permitting that.
        self.sources.clear();
        if let Some(helper) = &mut self.helper {
            match helper.snapshot().status {
                Status::Starting | Status::Running { .. } => {}
                Status::Cancelled => {
                    return Err(AppError::new("private Xwayland spawn was cancelled"));
                }
                Status::Failed(error) => {
                    return Err(AppError::new(format!(
                        "private Xwayland supervision failed: {error}"
                    )));
                }
                Status::Exited { code, signal } => {
                    return Err(AppError::new(format!(
                        "private Xwayland exited (code {code:?}, signal {signal:?})"
                    )));
                }
            }
        }
        if self
            .client
            .as_ref()
            .is_some_and(|client| !client.is_alive())
        {
            return Err(AppError::new("private Xwayland Wayland connection closed"));
        }
        if let Some(notification) = &mut self.notification {
            if notification.dispatch().map_err(app_error)? {
                self.notification = None;
            }
        }
        let mut writable = false;
        if let Some(xwm) = &mut self.xwm {
            let turn = xwm.dispatch(Instant::now()).map_err(app_error)?;
            self.ready.store(turn.reschedule, Ordering::Release);
            writable = turn.writable_interest;
            if turn.enumerated && self.notification.is_none() {
                if !self.initialized {
                    if let Some((display, _)) = &self.environment {
                        eprintln!(
                            "telorgon-xwayland: private server and XWM initialized on {}",
                            display.to_string_lossy()
                        );
                    }
                }
                self.initialized = true;
                self.deadline = None;
            }
            if let Some((surface, target, sequence, deadline, mut timestamp)) = self.pending_focus {
                if Instant::now() >= deadline {
                    self.pending_focus = None;
                } else {
                    timestamp = timestamp.or_else(|| {
                        turn.events
                            .iter()
                            .find_map(|event| xwm.focus_timestamp(event, sequence))
                    });
                    self.pending_focus = Some((surface, target, sequence, deadline, timestamp));
                    if let Some(timestamp) = timestamp.filter(|_| {
                        xwm.input_hints(target).is_some() && xwm.protocols(target).is_some()
                    }) {
                        self.pending_focus = None;
                        match xwm.focus_window(target, timestamp, Instant::now()) {
                            Ok(crate::xwayland::properties::FocusModel::NoInput) => {}
                            Ok(_) => {
                                xwm.raise_window(target, Instant::now())
                                    .map_err(app_error)?;
                                self.focus_result = Some((surface, target, deadline));
                            }
                            Err(error) => {
                                eprintln!("telorgon-xwayland: focus not applied: {error}")
                            }
                        }
                    }
                }
            }
            for (window, sequence) in &mut self.closing {
                let Some(expected) = *sequence else {
                    continue;
                };
                if let Some(timestamp) = turn
                    .events
                    .iter()
                    .find_map(|event| xwm.focus_timestamp(event, expected))
                {
                    *sequence = None;
                    if let Err(error) = xwm.close_window(*window, timestamp, Instant::now()) {
                        eprintln!("telorgon-xwayland: cooperative close failed: {error}");
                    }
                }
            }
            for action in turn.actions {
                use crate::xwayland::window::Action;
                match action {
                    Action::CommandFailed(window) => {
                        if let Some(sequence) = self.closing.get_mut(&window) {
                            *sequence = None;
                        }
                        if self
                            .focus_result
                            .is_some_and(|(_, target, _)| target == window)
                        {
                            self.focus_result = None;
                        }
                        if self
                            .pending_focus
                            .is_some_and(|(_, target, _, _, _)| target == window)
                        {
                            self.pending_focus = None;
                        }
                    }
                    Action::Created(window)
                        if xwm
                            .windows()
                            .and_then(|windows| windows.get(window.xid))
                            .is_some_and(|live| live.id == window) =>
                    {
                        xwm.refresh_protocols(window, Instant::now())
                            .map_err(app_error)?;
                        xwm.refresh_input_hints(window).map_err(app_error)?;
                        xwm.refresh_normal_hints(window).map_err(app_error)?;
                        xwm.refresh_title(window).map_err(app_error)?;
                    }
                    Action::RequestMap(window)
                        if xwm
                            .windows()
                            .and_then(|windows| windows.get(window.xid))
                            .is_some_and(|live| live.id == window) =>
                    {
                        xwm.map_window(window, Instant::now()).map_err(app_error)?
                    }
                    Action::RequestConfigure(window, requested) => {
                        self.desktop.configure_requested(window, requested);
                    }
                    _ => {}
                }
            }
            if xwm.windows().is_some() {
                for (surface, (serial, delivered)) in &mut self.serials {
                    if !*delivered {
                        xwm.committed_surface(1, *surface, *serial)
                            .map_err(app_error)?;
                        *delivered = true;
                    }
                }
            }
            // Commands queued above must get an owner turn even if the incoming
            // batch did not itself request rescheduling.
            writable |= xwm.wants_write();
        }
        let mask = crate::wayland_server::ffi::WL_EVENT_READABLE
            | crate::wayland_server::ffi::WL_EVENT_HANGUP
            | crate::wayland_server::ffi::WL_EVENT_ERROR;
        let mut fds = Vec::new();
        if let Some(helper) = &self.helper {
            fds.push((helper.fd(), mask));
        }
        if let Some(notification) = &self.notification {
            fds.push((notification.fd(), mask));
        }
        if let Some(fd) = self.xwm.as_ref().and_then(Xwm::fd) {
            fds.push((
                fd,
                mask | if writable {
                    crate::wayland_server::ffi::WL_EVENT_WRITABLE
                } else {
                    0
                },
            ));
        }
        for (fd, mask) in fds {
            self.sources.push(
                unsafe {
                    display.event_loop().add_fd(
                        fd,
                        mask,
                        Some(mark_external_fd_ready),
                        std::ptr::from_ref(self.ready.as_ref()).cast_mut().cast(),
                    )
                }
                .map_err(app_error)?,
            );
        }
        Ok(())
    }
    fn stop(&mut self) {
        self.sources.clear();
        self.preparation = None;
        self.xwm = None;
        self.desktop = Default::default();
        self.notification = None;
        if let Some(client) = self.client.take() {
            client.disconnect();
        }
        self.helper = None; // owned supervisor requests TERM, then bounded KILL/reap
        self.deadline = None;
        self.serials.clear();
        self.closing.clear();
        self.pending_focus = None;
        self.focus_result = None;
        self.ready.store(false, Ordering::Release);
        self.failed = true;
        self.initialized = false;
        self.environment = None;
    }
}
impl Drop for Compatibility {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_preparation_or_deadline_keeps_unrelated_wayland_client_alive() {
        for timeout in [false, true] {
            let mut display = Display::new().unwrap();
            let access = XwaylandAccess::configure_display(&mut display).unwrap();
            let (socket, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
            let native_client = display.create_client(socket).unwrap();
            let (send, receive) = mpsc::sync_channel(1);
            send.send(Err(crate::xwayland::Error(
                "fixture preparation failure".into(),
            )))
            .unwrap();
            let now = Instant::now();
            let mut host = Compatibility {
                sources: Vec::new(),
                ready: Box::new(AtomicBool::new(true)),
                preparation: Some(receive),
                helper: None,
                client: None,
                xwm: None,
                notification: None,
                access,
                deadline: Some(if timeout {
                    now
                } else {
                    now + Duration::from_secs(10)
                }),
                failed: false,
                initialized: false,
                environment: None,
                serials: BTreeMap::new(),
                identities: BTreeSet::new(),
                descendants: BTreeSet::new(),
                desktop: Default::default(),
                policy_repaint: false,
                closing: BTreeMap::new(),
                pending_focus: None,
                focus_result: None,
            };
            host.environment = Some((":123".into(), "/private/fixture-auth".into()));
            assert_eq!(host.environment(), None);
            host.initialized = true;
            assert!(host.environment().is_some());
            assert_eq!(host.wait(None), Some(Duration::ZERO));
            host.dispatch(&display);
            assert!(host.failed);
            assert_eq!(host.environment(), None);
            let root = WaylandSurfaceId::from_raw(1).unwrap();
            let child = WaylandSurfaceId::from_raw(2).unwrap();
            let mut image = super::super::client::maximize_preview_tests::test_window(
                SizeI {
                    width: 30,
                    height: 20,
                },
                PointI { x: 10, y: 10 },
            );
            image.role = SurfaceRole::Xwayland;
            let mut sub = super::super::client::maximize_preview_tests::test_window(
                SizeI {
                    width: 5,
                    height: 5,
                },
                PointI::default(),
            );
            sub.role = SurfaceRole::Subsurface;
            sub.parent = Some(root);
            sub.offset = PointI { x: 2, y: 3 };
            let mut windows = BTreeMap::from([(root, image), (child, sub)]);
            let mut identities = WindowIdentities::default();
            assert!(
                host.sync_presentation(
                    &mut windows,
                    &mut identities,
                    &LinuxDesktopConfig::default()
                )
                .unwrap()
            );
            assert!(windows[&root].minimized);
            assert!(windows[&child].minimized);
            assert_eq!(windows[&child].position, PointI { x: 12, y: 13 });
            assert!(
                !host
                    .sync_presentation(
                        &mut windows,
                        &mut identities,
                        &LinuxDesktopConfig::default()
                    )
                    .unwrap()
            );

            windows.remove(&root);
            windows.get_mut(&child).unwrap().minimized = false;
            assert!(
                host.sync_presentation(
                    &mut windows,
                    &mut identities,
                    &LinuxDesktopConfig::default()
                )
                .unwrap()
            );
            assert!(windows[&child].minimized);
            assert!(host.preparation.is_none());
            assert_eq!(host.wait(None), None);
            assert!(native_client.is_alive());
            host.dispatch(&display);
            drop(host);
            assert!(native_client.is_alive());
        }
    }
}

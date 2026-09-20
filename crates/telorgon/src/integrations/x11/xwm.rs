//! Owner-loop adapter for XWM initialization and window observation. Tracking is
//! not desktop readiness: output state and executable window policy remain required.
use super::{
    Error, Result,
    association::XWindow,
    commands::Commands,
    discovery::{Discovered, Discovery},
    inspection::Inspector,
    manager::Manager,
    properties::{FocusModel, InputHints, PropertyReader, Protocols},
    requests::{Completion, Requests},
    transport::Transport,
    window::{Action, Geometry, Windows},
};
use std::{
    borrow::Cow,
    collections::{BTreeMap, VecDeque},
    os::{fd::RawFd, unix::net::UnixStream},
    time::{Duration, Instant},
};
use x11rb_protocol::{
    id_allocator::IdAllocator,
    protocol::xproto,
    x11_utils::{Request, TryParse},
};
const CAPACITY: usize = 4096;
const BUDGET: Duration = Duration::from_millis(1);
#[cfg(test)]
#[path = "xwm_decoration_tests.rs"]
pub(crate) mod decoration_tests;
#[cfg(test)]
#[path = "xwm_sync_tests.rs"]
mod resize_sync_tests;
enum Phase {
    Discovery(Discovery),
    Manager(Manager),
    Tracking(Tracking),
}
struct Tracking {
    selection_checks: BTreeMap<u64, bool>,
    selection_watches: Vec<(super::selection::Selection, u32)>,
    transport: Transport,
    requests: Requests,
    discovered: Discovered,
    _ids: IdAllocator,
    manager: u32,
    incoming: VecDeque<Vec<u8>>,
    commands: Commands,
    protocols: PropertyReader,
    sync_counter: PropertyReader,
    resize_sync: super::resize_sync::ResizeSync,
    server_time: u32,
    hints: PropertyReader,
    normal_hints: PropertyReader,
    decorations: PropertyReader,
    frame_extents: BTreeMap<XWindow, [u32; 4]>,
    move_resize_requests: Vec<(XWindow, u32, u32)>,
    maximize_requests: Vec<(XWindow, u32)>,
    minimize_requests: Vec<XWindow>,
    maximized: BTreeMap<XWindow, bool>,
    title: PropertyReader,
    legacy_title: PropertyReader,
    app_class: PropertyReader,
    app_icon: PropertyReader,
    last_focus_request: Option<(u32, Instant)>,
}
impl Tracking {
    fn selection_completion(&mut self, completion: &Completion) -> Result<bool> {
        let id = match completion {
            Completion::Reply(id, _)
            | Completion::Checked(id)
            | Completion::Error(id, _)
            | Completion::TimedOut(id) => *id,
            Completion::Event(_) => return Ok(false),
        };
        if id.generation != self.requests.generation() {
            return Ok(false);
        }
        let Some(barrier) = self.selection_checks.get(&id.sequence).copied() else {
            return Ok(false);
        };
        match completion {
            Completion::Checked(_) if !barrier => {}
            Completion::Reply(_, bytes) if barrier => {
                xproto::GetInputFocusReply::try_parse(bytes)
                    .map_err(|_| Error("malformed selection subscription barrier".into()))?;
            }
            _ => return Err(Error("XWM selection subscription failed".into())),
        }
        self.selection_checks.remove(&id.sequence);
        Ok(true)
    }
}
pub struct Turn {
    pub actions: Vec<Action>,
    /// Events for subsequent property/focus/selection policy handlers.
    pub events: Vec<Vec<u8>>,
    pub reschedule: bool,
    pub writable_interest: bool,
    pub enumerated: bool,
}
pub struct Xwm {
    pub(crate) root_cursor: Option<super::root_cursor::RootCursor>,
    phase: Option<Phase>,
    windows: Option<Windows>,
    inspector: Inspector,
    early_events: VecDeque<Vec<u8>>,
    deadline: Instant,
}
impl Xwm {
    pub fn new(socket: UnixStream, generation: u64, now: Instant) -> Result<Self> {
        let discovery = Discovery::new(socket, generation, now)?;
        let deadline = discovery.deadline();
        Ok(Self {
            root_cursor: None,
            phase: Some(Phase::Discovery(discovery)),
            windows: None,
            inspector: Inspector::new(),
            early_events: VecDeque::new(),
            deadline,
        })
    }
    pub fn set_root_cursor(&mut self, cursor: super::root_cursor::RootCursor) -> Result<()> {
        if !matches!(self.phase, Some(Phase::Discovery(_))) {
            return Err(Error(
                "root cursor must be configured before manager initialization".into(),
            ));
        }
        self.root_cursor = Some(cursor);
        Ok(())
    }
    pub fn fd(&self) -> Option<RawFd> {
        self.phase.as_ref().map(|p| match p {
            Phase::Discovery(p) => p.fd(),
            Phase::Manager(p) => p.fd(),
            Phase::Tracking(p) => p.transport.fd(),
        })
    }
    /// Refresh writable interest after policy queues requests outside dispatch.
    pub fn wants_write(&self) -> bool {
        self.phase.as_ref().is_some_and(|phase| match phase {
            Phase::Discovery(p) => p.wants_write(),
            Phase::Manager(p) => p.wants_write(),
            Phase::Tracking(p) => p.transport.wants_write(),
        })
    }
    pub fn deadline(&self) -> Option<Instant> {
        match self.phase.as_ref()? {
            Phase::Tracking(p) => p
                .requests
                .deadline()
                .into_iter()
                .chain(p.resize_sync.deadline())
                .min(),
            _ => Some(self.deadline),
        }
    }
    /// Enable one selection watch on the XWM's owned manager window. Returned
    /// context routes raw Turn events into the host's ownership ledger. Wait for
    /// selection_watches_ready before relying on the subscription. Failure after queueing
    /// terminates this driver to contain partial setup. Rejection before queueing
    /// leaves the existing driver and subscriptions intact.
    pub fn watch_selection(
        &mut self,
        selection: super::selection::Selection,
        atom: u32,
        proxy_window: u32,
        now: Instant,
    ) -> Result<super::selection::Subscription> {
        let mut queued = false;
        let result = (|| {
            let Some(Phase::Tracking(tracking)) = self.phase.as_mut() else {
                return Err(Error("XWM is not tracking windows".into()));
            };
            if tracking
                .selection_watches
                .iter()
                .any(|(watched, watched_atom)| *watched == selection || *watched_atom == atom)
                || tracking.selection_watches.len() >= 2
                || tracking.requests.available_slots() < 2
            {
                return Err(Error("XWM selection subscription bound reached".into()));
            }
            let extension = &tracking.discovered.extensions["XFIXES"];
            let subscription = super::selection::Subscription {
                generation: tracking.requests.generation(),
                selection,
                atom,
                window: tracking.manager,
                proxy_window,
                first_event: extension.first_event,
            };
            let deadline = now + Duration::from_secs(10);
            let id = subscription.queue(
                extension.major_opcode,
                true,
                &mut tracking.transport,
                &mut tracking.requests,
                deadline,
            )?;
            queued = true;
            tracking.selection_checks.insert(id.sequence, false);
            let (bytes, _) = Request::serialize(xproto::GetInputFocusRequest, 0);
            let barrier = tracking.requests.queue(
                &mut tracking.transport,
                bytes,
                super::requests::ReplyKind::Reply,
                super::requests::Importance::Optional,
                deadline,
            )?;
            tracking.selection_checks.insert(barrier.sequence, true);
            tracking.selection_watches.push((selection, atom));
            Ok(subscription)
        })();
        if result.is_err() && queued {
            self.phase = None;
            self.windows = None;
            self.early_events.clear();
        }
        result
    }
    /// Watch the standard selection using the atom resolved on this connection.
    pub fn watch_standard_selection(
        &mut self,
        selection: super::selection::Selection,
        proxy_window: u32,
        now: Instant,
    ) -> Result<super::selection::Subscription> {
        let atom = self
            .selection_atoms()
            .ok_or_else(|| Error("XWM is not tracking windows".into()))?
            .selection(selection);
        self.watch_selection(selection, atom, proxy_window, now)
    }
    /// Connection-scoped IDs for ownership, target negotiation and INCR routing.
    /// Unavailable before tracking or after compatibility teardown.
    pub fn selection_atoms(&self) -> Option<super::selection::Atoms> {
        let Phase::Tracking(tracking) = self.phase.as_ref()? else {
            return None;
        };
        super::selection::Atoms::discovered(
            tracking.requests.generation(),
            &tracking.discovered.atoms,
        )
        .ok()
    }
    pub fn selection_watches_ready(&self) -> bool {
        matches!(self.phase.as_ref(), Some(Phase::Tracking(t)) if !t.selection_watches.is_empty() && t.selection_checks.is_empty())
    }
    pub fn windows(&self) -> Option<&Windows> {
        self.windows.as_ref()
    }
    fn command(&mut self, window: XWindow, request: impl Request, now: Instant) -> Result<()> {
        if !self
            .windows
            .as_ref()
            .and_then(|w| w.get(window.xid))
            .is_some_and(|w| w.id == window)
        {
            return Err(Error("stale XWM command window".into()));
        }
        let Some(Phase::Tracking(tracking)) = self.phase.as_mut() else {
            return Err(Error("XWM policy commands require tracking phase".into()));
        };
        if tracking.requests.available_slots() < 2 || tracking.commands.available_groups() == 0 {
            return Err(Error("XWM command queue is busy".into()));
        }
        let result = tracking.commands.queue(
            request,
            window,
            &mut tracking.transport,
            &mut tracking.requests,
            now + Duration::from_secs(10),
        );
        // A partially queued command cannot be reported as an atomic rejection.
        // Terminate this compatibility connection on queue/backpressure failure.
        if result.is_err() {
            self.phase = None;
            self.windows = None;
        }
        result
    }
    pub fn command_capacity(&self) -> usize {
        match &self.phase {
            Some(Phase::Tracking(t)) => t
                .commands
                .available_groups()
                .min(t.requests.available_slots() / 2),
            _ => 0,
        }
    }
    /// Queue a policy-approved map; only MapNotify changes confirmed map state.
    pub fn map_window(&mut self, window: XWindow, now: Instant) -> Result<()> {
        self.command(window, xproto::MapWindowRequest { window: window.xid }, now)
    }
    /// Queue actual geometry selected by policy, independent of xdg configure/ack.
    /// Stacking and focus are separate policy operations.
    pub fn configure_window(
        &mut self,
        window: XWindow,
        geometry: Geometry,
        now: Instant,
    ) -> Result<()> {
        if geometry.width == 0 || geometry.height == 0 {
            return Err(Error("zero XWM configure dimensions".into()));
        }
        let values = xproto::ConfigureWindowAux::new()
            .x(i32::from(geometry.x))
            .y(i32::from(geometry.y))
            .width(u32::from(geometry.width))
            .height(u32::from(geometry.height))
            .border_width(u32::from(geometry.border));
        self.command(
            window,
            xproto::ConfigureWindowRequest {
                window: window.xid,
                value_list: Cow::Owned(values),
            },
            now,
        )
    }
    /// Configure with optional client repaint acknowledgement. False means the
    /// previous resize/initialization is still pending; retry on the XWM deadline.
    pub fn configure_window_synced(
        &mut self,
        window: XWindow,
        geometry: Geometry,
        now: Instant,
    ) -> Result<bool> {
        let actual = self
            .windows
            .as_ref()
            .and_then(|w| w.get(window.xid))
            .filter(|w| w.id == window)
            .ok_or_else(|| Error("stale resize sync window".into()))?
            .geometry;
        let resized = actual.width != geometry.width || actual.height != geometry.height;
        if self.command_capacity() == 0 {
            return Ok(false);
        }
        if resized {
            let Some(Phase::Tracking(t)) = self.phase.as_mut() else {
                return Err(Error("resize requires tracking".into()));
            };
            if t.resize_sync.waiting(window) {
                return Ok(false);
            }
            // Reserve room for both checked commands before changing the sync target.
            if t.requests.available_slots() < 4 || t.commands.available_groups() < 2 {
                return Ok(false);
            }
            if let Some(value) = t.resize_sync.begin(window, now) {
                let event: [u8; 32] = xproto::ClientMessageEvent {
                    response_type: xproto::CLIENT_MESSAGE_EVENT,
                    format: 32,
                    sequence: 0,
                    window: window.xid,
                    type_: t.discovered.atoms["WM_PROTOCOLS"],
                    data: [
                        t.discovered.atoms["_NET_WM_SYNC_REQUEST"],
                        t.server_time,
                        value as u32,
                        (value >> 32) as u32,
                        0,
                    ]
                    .into(),
                }
                .into();
                self.command(
                    window,
                    xproto::SendEventRequest {
                        propagate: false,
                        destination: window.xid,
                        event_mask: 0u32.into(),
                        event: Cow::Owned(event),
                    },
                    now,
                )?;
            }
        }
        // Same X connection: SendEvent precedes the real ConfigureNotify generated here.
        self.configure_window(window, geometry, now)?;
        Ok(true)
    }
    pub fn resize_sync_status(&self, window: XWindow) -> super::resize_sync::ResizeSyncStatus {
        match &self.phase {
            Some(Phase::Tracking(t)) => t.resize_sync.status(window),
            _ => super::resize_sync::ResizeSyncStatus::Unsupported,
        }
    }
    pub fn repaint_pending(&self, window: XWindow) -> bool {
        matches!(
            self.resize_sync_status(window),
            super::resize_sync::ResizeSyncStatus::Waiting
        )
    }
    pub fn raise_window(&mut self, window: XWindow, now: Instant) -> Result<()> {
        self.command(
            window,
            xproto::ConfigureWindowRequest {
                window: window.xid,
                value_list: Cow::Owned(
                    xproto::ConfigureWindowAux::new().stack_mode(xproto::StackMode::ABOVE),
                ),
            },
            now,
        )
    }
    /// Configure an ordinary client only if current normal hints accept the size.
    /// Fullscreen policy may deliberately use configure_window instead.
    pub fn configure_window_with_hints(
        &mut self,
        window: XWindow,
        geometry: Geometry,
        now: Instant,
    ) -> Result<()> {
        let hints = self
            .normal_hints(window)
            .ok_or_else(|| Error("XWM normal hints unavailable".into()))?;
        if !hints.accepts_size(crate::foundation::SizeI {
            width: geometry.width.into(),
            height: geometry.height.into(),
        }) {
            return Err(Error("XWM configure size violates normal hints".into()));
        }
        self.configure_window(window, geometry, now)
    }
    /// Report policy-selected geometry to a client after an adjusted/no-op request.
    /// Supply root coordinates adjusted for the client's requested border, and
    /// that requested border width, as required by ICCCM 4.1.5. This notification
    /// does not change server-confirmed geometry in the window registry.
    pub fn notify_configured(
        &mut self,
        window: XWindow,
        root_geometry: Geometry,
        above_sibling: Option<XWindow>,
        now: Instant,
    ) -> Result<()> {
        if root_geometry.width == 0 || root_geometry.height == 0 {
            return Err(Error("zero XWM configure notification dimensions".into()));
        }
        let target = self
            .windows
            .as_ref()
            .and_then(|w| w.get(window.xid))
            .filter(|w| w.id == window)
            .ok_or_else(|| Error("stale XWM configure notification window".into()))?;
        if let Some(sibling) = above_sibling {
            if sibling == window
                || !self
                    .windows
                    .as_ref()
                    .and_then(|w| w.get(sibling.xid))
                    .is_some_and(|w| w.id == sibling && w.parent == target.parent)
            {
                return Err(Error("stale XWM configure notification sibling".into()));
            }
        }
        let event: [u8; 32] = xproto::ConfigureNotifyEvent {
            response_type: xproto::CONFIGURE_NOTIFY_EVENT,
            sequence: 0,
            event: window.xid,
            window: window.xid,
            above_sibling: above_sibling.map_or(0, |w| w.xid),
            x: root_geometry.x,
            y: root_geometry.y,
            width: root_geometry.width,
            height: root_geometry.height,
            border_width: root_geometry.border,
            override_redirect: target.override_redirect,
        }
        .into();
        self.command(
            window,
            xproto::SendEventRequest {
                propagate: false,
                destination: window.xid,
                event_mask: xproto::EventMask::STRUCTURE_NOTIFY,
                event: Cow::Owned(event),
            },
            now,
        )
    }
    pub fn protocols(&self, window: XWindow) -> Option<Protocols> {
        let Phase::Tracking(tracking) = self.phase.as_ref()? else {
            return None;
        };
        tracking.protocols.get(window)
    }
    /// Schedule an initial WM_PROTOCOLS read for a policy-managed window. Later
    /// authoritative PropertyNotify events refresh it automatically.
    pub fn refresh_protocols(&mut self, window: XWindow, now: Instant) -> Result<()> {
        if !self
            .windows
            .as_ref()
            .and_then(|w| w.get(window.xid))
            .is_some_and(|w| w.id == window)
        {
            return Err(Error("stale XWM protocol window".into()));
        }
        let Some(Phase::Tracking(tracking)) = self.phase.as_mut() else {
            return Err(Error("XWM protocol reads require tracking phase".into()));
        };
        let _ = tracking;
        self.command(
            window,
            xproto::ChangeWindowAttributesRequest {
                window: window.xid,
                value_list: Cow::Owned(
                    xproto::ChangeWindowAttributesAux::new()
                        .event_mask(xproto::EventMask::PROPERTY_CHANGE),
                ),
            },
            now,
        )?;
        let Some(Phase::Tracking(tracking)) = self.phase.as_mut() else {
            unreachable!()
        };
        tracking.protocols.refresh(window)
    }
    pub fn input_hints(&self, window: XWindow) -> Option<InputHints> {
        let Phase::Tracking(tracking) = self.phase.as_ref()? else {
            return None;
        };
        tracking.hints.input_hints(window)
    }
    /// Read focus hints after the property subscription established by refresh_protocols.
    pub fn refresh_input_hints(&mut self, window: XWindow) -> Result<()> {
        if !self
            .windows
            .as_ref()
            .and_then(|w| w.get(window.xid))
            .is_some_and(|w| w.id == window)
        {
            return Err(Error("stale XWM input-hint window".into()));
        }
        let Some(Phase::Tracking(tracking)) = self.phase.as_mut() else {
            return Err(Error("XWM input hints require tracking phase".into()));
        };
        if !tracking.protocols.tracked(window) {
            return Err(Error(
                "XWM input hints require a property subscription".into(),
            ));
        }
        tracking.hints.refresh(window)
    }
    pub fn window_icon(&self, window: XWindow) -> Option<&crate::graphics::render::ImageResource> {
        let Some(Phase::Tracking(tracking)) = &self.phase else {
            return None;
        };
        tracking.app_icon.icon(window)
    }
    pub fn window_application_class(&self, window: XWindow) -> Option<&str> {
        let Some(Phase::Tracking(tracking)) = &self.phase else {
            return None;
        };
        tracking.app_class.text(window)
    }
    pub fn window_title(&self, window: XWindow) -> Option<&str> {
        let Some(Phase::Tracking(tracking)) = &self.phase else {
            return None;
        };
        tracking
            .title
            .text(window)
            .filter(|text| !text.is_empty())
            .or_else(|| tracking.legacy_title.text(window))
    }
    pub fn refresh_title(&mut self, window: XWindow) -> Result<()> {
        if !self
            .windows
            .as_ref()
            .and_then(|windows| windows.get(window.xid))
            .is_some_and(|live| live.id == window)
        {
            return Err(Error("stale XWM title window".into()));
        }
        let Some(Phase::Tracking(tracking)) = &mut self.phase else {
            return Err(Error("XWM not initialized".into()));
        };
        tracking.app_icon.refresh(window)?;
        tracking.app_class.refresh(window)?;
        tracking.title.refresh(window)?;
        tracking.legacy_title.refresh(window)
    }

    /// Resolve queued requests against live window incarnations and surface associations.
    pub fn take_move_resize_requests(&mut self) -> Vec<(u64, u32, u32)> {
        let Some(Phase::Tracking(tracking)) = &mut self.phase else {
            return Vec::new();
        };
        let Some(windows) = &self.windows else {
            return Vec::new();
        };
        std::mem::take(&mut tracking.move_resize_requests)
            .into_iter()
            .filter_map(|(id, direction, button)| {
                windows
                    .presentable_surface(id)
                    .map(|surface| (surface, direction, button))
            })
            .collect()
    }

    /// Mapped managed clients request remove (0), add (1), or toggle (2).
    /// Resolve incarnations at consumption so stale requests cannot affect reused XIDs.
    pub fn take_maximize_requests(&mut self) -> Vec<(u64, u32)> {
        let Some(Phase::Tracking(tracking)) = &mut self.phase else {
            return Vec::new();
        };
        let Some(windows) = &self.windows else {
            return Vec::new();
        };
        std::mem::take(&mut tracking.maximize_requests)
            .into_iter()
            .filter_map(|(id, action)| {
                windows
                    .get(id.xid)
                    .filter(|w| w.id == id && !w.override_redirect)?;
                windows
                    .presentable_surface(id)
                    .map(|surface| (surface, action))
            })
            .collect()
    }

    /// Consume authenticated ICCCM iconify requests for live managed windows.
    pub fn take_minimize_requests(&mut self) -> Vec<u64> {
        let Some(Phase::Tracking(tracking)) = &mut self.phase else {
            return Vec::new();
        };
        let Some(windows) = &self.windows else {
            return Vec::new();
        };
        std::mem::take(&mut tracking.minimize_requests)
            .into_iter()
            .filter_map(|id| {
                windows
                    .get(id.xid)
                    .filter(|w| w.id == id && w.mapped && !w.override_redirect)?;
                windows.presentable_surface(id)
            })
            .collect()
    }

    /// Publish the shell's actual maximize state, including changes from compositor controls.
    /// Telorgon currently has one maximize mode, covering both axes.
    pub fn set_maximized_state(
        &mut self,
        window: XWindow,
        maximized: bool,
        now: Instant,
    ) -> Result<bool> {
        if !self
            .windows
            .as_ref()
            .and_then(|w| w.get(window.xid))
            .is_some_and(|w| w.id == window && w.mapped && !w.override_redirect)
        {
            return Err(Error("stale or unmanaged XWM state window".into()));
        }
        let Some(Phase::Tracking(tracking)) = &mut self.phase else {
            return Err(Error("XWM not initialized".into()));
        };
        if tracking.maximized.get(&window) == Some(&maximized) {
            return Ok(true);
        }
        if tracking.commands.available_groups() == 0 || tracking.requests.available_slots() < 2 {
            return Ok(false);
        }
        let atoms = if maximized {
            vec![
                tracking.discovered.atoms["_NET_WM_STATE_MAXIMIZED_VERT"],
                tracking.discovered.atoms["_NET_WM_STATE_MAXIMIZED_HORZ"],
            ]
        } else {
            Vec::new()
        };
        tracking.commands.queue(
            xproto::ChangePropertyRequest {
                mode: xproto::PropMode::REPLACE,
                window: window.xid,
                property: tracking.discovered.atoms["_NET_WM_STATE"],
                type_: xproto::AtomEnum::ATOM.into(),
                format: 32,
                data_len: atoms.len() as u32,
                data: Cow::Owned(atoms.into_iter().flat_map(u32::to_ne_bytes).collect()),
            },
            window,
            &mut tracking.transport,
            &mut tracking.requests,
            now + Duration::from_secs(10),
        )?;
        tracking.maximized.insert(window, maximized);
        Ok(true)
    }

    pub fn decorations(&self, window: XWindow) -> bool {
        match &self.phase {
            Some(Phase::Tracking(tracking)) => {
                tracking.decorations.decorations(window).unwrap_or(true)
            }
            _ => true,
        }
    }
    pub fn refresh_decorations(&mut self, window: XWindow) -> Result<()> {
        if !self
            .windows
            .as_ref()
            .and_then(|w| w.get(window.xid))
            .is_some_and(|w| w.id == window)
        {
            return Err(Error("stale XWM decoration window".into()));
        }
        let Some(Phase::Tracking(tracking)) = &mut self.phase else {
            return Err(Error("XWM not initialized".into()));
        };
        if !tracking.protocols.tracked(window) {
            return Err(Error(
                "decoration hints require property subscription".into(),
            ));
        }
        tracking.decorations.refresh(window)
    }
    /// Publish left/right/top/bottom frame widths in X11 pixels. Repeated values coalesce.
    /// Returns false on backpressure; the owner retries on its next presentation sync.
    pub fn set_frame_extents(
        &mut self,
        window: XWindow,
        extents: [u32; 4],
        now: Instant,
    ) -> Result<bool> {
        if !self
            .windows
            .as_ref()
            .and_then(|w| w.get(window.xid))
            .is_some_and(|w| w.id == window)
        {
            return Err(Error("stale XWM frame extents window".into()));
        }
        let Some(Phase::Tracking(tracking)) = &mut self.phase else {
            return Err(Error("XWM not initialized".into()));
        };
        if tracking.frame_extents.get(&window) == Some(&extents) {
            return Ok(true);
        }
        if tracking.commands.available_groups() == 0 || tracking.requests.available_slots() < 2 {
            return Ok(false);
        }
        tracking.commands.queue(
            xproto::ChangePropertyRequest {
                mode: xproto::PropMode::REPLACE,
                window: window.xid,
                property: tracking.discovered.atoms["_NET_FRAME_EXTENTS"],
                type_: xproto::AtomEnum::CARDINAL.into(),
                format: 32,
                data_len: 4,
                data: Cow::Owned(extents.into_iter().flat_map(u32::to_ne_bytes).collect()),
            },
            window,
            &mut tracking.transport,
            &mut tracking.requests,
            now + Duration::from_secs(10),
        )?;
        tracking.frame_extents.insert(window, extents);
        Ok(true)
    }

    pub fn normal_hints(&self, window: XWindow) -> Option<super::normal_hints::NormalHints> {
        let Phase::Tracking(tracking) = self.phase.as_ref()? else {
            return None;
        };
        tracking.normal_hints.normal_hints(window)
    }
    /// Request size constraints after refresh_protocols establishes property subscription.
    pub fn refresh_normal_hints(&mut self, window: XWindow) -> Result<()> {
        if !self
            .windows
            .as_ref()
            .and_then(|w| w.get(window.xid))
            .is_some_and(|w| w.id == window)
        {
            return Err(Error("stale XWM normal-hint window".into()));
        }
        let Some(Phase::Tracking(tracking)) = self.phase.as_mut() else {
            return Err(Error("XWM normal hints require tracking phase".into()));
        };
        if !tracking.protocols.tracked(window) {
            return Err(Error(
                "XWM normal hints require a property subscription".into(),
            ));
        }
        tracking.normal_hints.refresh(window)
    }
    /// True until all policy command barriers for this window have resolved.
    pub fn commands_pending(&self, window: XWindow) -> bool {
        match self.phase.as_ref() {
            Some(Phase::Tracking(tracking)) => tracking.commands.pending_for(window),
            _ => true,
        }
    }
    /// Obtain server time asynchronously for an already-authorized user focus action.
    pub fn request_focus_timestamp(&mut self, window: XWindow, now: Instant) -> Result<u16> {
        let Some(Phase::Tracking(tracking)) = self.phase.as_ref() else {
            return Err(Error("XWM focus clock unavailable".into()));
        };
        let request = xproto::ChangePropertyRequest {
            mode: xproto::PropMode::REPLACE,
            window: tracking.manager,
            property: tracking.discovered.atoms["MANAGER"],
            type_: u32::from(xproto::AtomEnum::INTEGER),
            format: 32,
            data_len: 1,
            data: Cow::Owned(0u32.to_ne_bytes().to_vec()),
        };
        self.command(window, request, now)?;
        let Some(Phase::Tracking(tracking)) = self.phase.as_ref() else {
            unreachable!()
        };
        // command queues the ChangeProperty immediately followed by its barrier.
        Ok(tracking.requests.last_sequence().wrapping_sub(1) as u16)
    }
    pub fn focus_timestamp(&self, bytes: &[u8], sequence: u16) -> Option<u32> {
        let Phase::Tracking(tracking) = self.phase.as_ref()? else {
            return None;
        };
        if bytes.len() != 32 || bytes[0] != xproto::PROPERTY_NOTIFY_EVENT {
            return None;
        }
        let (event, _) = xproto::PropertyNotifyEvent::try_parse(bytes).ok()?;
        (event.window == tracking.manager
            && event.atom == tracking.discovered.atoms["MANAGER"]
            && event.sequence == sequence
            && event.state == xproto::Property::NEW_VALUE
            && event.time != 0)
            .then_some(event.time)
    }
    /// Execute only a compositor-authorized focus decision after lock/seat policy
    /// checks. The timestamp must come from the initiating X-server interaction.
    /// This does not grant native Wayland focus or implement activation policy.
    pub fn focus_window(
        &mut self,
        window: XWindow,
        timestamp: u32,
        now: Instant,
    ) -> Result<FocusModel> {
        if timestamp == 0
            || self
                .windows
                .as_ref()
                .and_then(|w| w.presentable_surface(window))
                .is_none()
        {
            return Err(Error(
                "XWM focus requires a mapped associated window and timestamp".into(),
            ));
        }
        if let Some(Phase::Tracking(tracking)) = self.phase.as_ref()
            && !focus_timestamp_is_current(tracking.last_focus_request, timestamp, now)
        {
            return Err(Error("stale or ambiguous XWM focus timestamp".into()));
        }
        let hints = self
            .input_hints(window)
            .ok_or_else(|| Error("XWM input hints unavailable".into()))?;
        let protocols = self
            .protocols(window)
            .ok_or_else(|| Error("XWM protocols unavailable".into()))?;
        let model = FocusModel::from_hints(hints, protocols);
        let command_count = usize::from(matches!(
            model,
            FocusModel::Passive | FocusModel::LocallyActive
        )) + usize::from(protocols.take_focus);
        let Some(Phase::Tracking(tracking)) = self.phase.as_ref() else {
            unreachable!()
        };
        if tracking.requests.available_slots() < command_count * 2
            || tracking.commands.available_groups() < command_count
        {
            return Err(Error("XWM focus command queue is busy".into()));
        }

        if matches!(model, FocusModel::Passive | FocusModel::LocallyActive) {
            self.command(
                window,
                xproto::SetInputFocusRequest {
                    revert_to: xproto::InputFocus::PARENT,
                    focus: window.xid,
                    time: timestamp,
                },
                now,
            )?;
        }
        if protocols.take_focus {
            let Some(Phase::Tracking(tracking)) = self.phase.as_ref() else {
                unreachable!()
            };
            let event: [u8; 32] = xproto::ClientMessageEvent {
                response_type: xproto::CLIENT_MESSAGE_EVENT,
                format: 32,
                sequence: 0,
                window: window.xid,
                type_: tracking.discovered.atoms["WM_PROTOCOLS"],
                data: [
                    tracking.discovered.atoms["WM_TAKE_FOCUS"],
                    timestamp,
                    0,
                    0,
                    0,
                ]
                .into(),
            }
            .into();
            self.command(
                window,
                xproto::SendEventRequest {
                    propagate: false,
                    destination: window.xid,
                    event_mask: 0u32.into(),
                    event: Cow::Owned(event),
                },
                now,
            )?;
        }
        if model != FocusModel::NoInput
            && let Some(Phase::Tracking(tracking)) = self.phase.as_mut()
        {
            tracking.last_focus_request = Some((timestamp, now));
        }
        Ok(model)
    }
    /// Request cooperative close only when current validated metadata supports it.
    /// Unsupported/unknown metadata returns an error; no process or X client is killed.
    pub fn close_window(&mut self, window: XWindow, timestamp: u32, now: Instant) -> Result<()> {
        if timestamp == 0 || !self.protocols(window).is_some_and(|p| p.delete_window) {
            return Err(Error("XWM cooperative close unavailable".into()));
        }
        let Some(Phase::Tracking(tracking)) = self.phase.as_ref() else {
            return Err(Error("XWM close requires tracking phase".into()));
        };
        let event: [u8; 32] = xproto::ClientMessageEvent {
            response_type: xproto::CLIENT_MESSAGE_EVENT,
            format: 32,
            sequence: 0,
            window: window.xid,
            type_: tracking.discovered.atoms["WM_PROTOCOLS"],
            data: [
                tracking.discovered.atoms["WM_DELETE_WINDOW"],
                timestamp,
                0,
                0,
                0,
            ]
            .into(),
        }
        .into();
        self.command(
            window,
            xproto::SendEventRequest {
                propagate: false,
                destination: window.xid,
                event_mask: 0u32.into(),
                event: Cow::Owned(event),
            },
            now,
        )
    }
    /// Legacy fallback for an explicit user close, never session shutdown or timeout.
    /// KillClient closes the X connection owning this live window, not an OS PID.
    pub fn close_legacy_window(&mut self, window: XWindow, now: Instant) -> Result<()> {
        if !self
            .windows
            .as_ref()
            .and_then(|w| w.get(window.xid))
            .is_some_and(|w| w.id == window && w.mapped && !w.override_redirect)
            || !self.protocols(window).is_some_and(|p| !p.delete_window)
        {
            return Err(Error(
                "legacy close requires a live managed window with known protocols".into(),
            ));
        }
        self.command(
            window,
            xproto::KillClientRequest {
                resource: window.xid,
            },
            now,
        )
    }
    /// The caller must authenticate the dedicated Wayland client and pass only a
    /// newly committed serial. This does not authorize arbitrary Wayland clients.
    pub fn committed_surface(
        &mut self,
        generation: u64,
        surface: u64,
        serial: u64,
    ) -> Result<Option<Action>> {
        self.windows
            .as_mut()
            .ok_or_else(|| Error("XWM window registry not initialized".into()))?
            .committed_surface(generation, surface, serial)
    }
    pub fn destroy_surface(&mut self, generation: u64, surface: u64) -> Result<()> {
        self.windows
            .as_mut()
            .ok_or_else(|| Error("XWM window registry not initialized".into()))?
            .destroy_surface(generation, surface)
    }
    fn manager(
        &mut self,
        transport: Transport,
        requests: Requests,
        discovered: Discovered,
    ) -> Result<Phase> {
        let generation = requests.generation();
        let root = discovered.setup.roots[0].root;
        let serial = discovered.atoms["WL_SURFACE_SERIAL"];
        let mut manager = Manager::new(transport, requests, discovered, self.deadline)?;
        manager.root_cursor = self.root_cursor.take();
        self.windows = Some(Windows::new(
            generation,
            CAPACITY,
            root,
            manager.window_id(),
            serial,
        )?);
        Ok(Phase::Manager(manager))
    }
    fn event(
        &mut self,
        bytes: Vec<u8>,
        actions: &mut Vec<Action>,
        events: &mut Vec<Vec<u8>>,
    ) -> Result<()> {
        let windows = self
            .windows
            .as_mut()
            .ok_or_else(|| Error("XWM event before registry initialization".into()))?;
        match windows.event(&bytes)? {
            Some(Action::Inspect(xid)) => self.inspector.enqueue(xid, windows)?,
            Some(action) => actions.push(action),
            None => events.push(bytes),
        }
        Ok(())
    }
    /// Register writable interest and the returned deadline with the existing
    /// owner loop. Errors drop the XWM transport; they never terminate the desktop.
    pub fn dispatch(&mut self, now: Instant) -> Result<Turn> {
        let result = self.dispatch_inner(now);
        if result.is_err() {
            self.phase = None;
            self.windows = None;
            self.early_events.clear();
        }
        result
    }
    fn dispatch_inner(&mut self, now: Instant) -> Result<Turn> {
        let phase = self
            .phase
            .take()
            .ok_or_else(|| Error("XWM driver has failed".into()))?;
        if !matches!(phase, Phase::Tracking(_)) && now >= self.deadline {
            return Err(Error("XWM initialization timed out".into()));
        }
        let started = Instant::now();
        let mut actions = vec![];
        let mut events = vec![];
        let mut reschedule = false;
        let phase = if self.windows.is_some() && !self.early_events.is_empty() {
            for _ in 0..256 {
                if started.elapsed() >= BUDGET {
                    break;
                }
                let Some(event) = self.early_events.pop_front() else {
                    break;
                };
                self.event(event, &mut actions, &mut events)?;
            }
            reschedule = true; // Resume initialization even when the backlog just drained.
            phase
        } else {
            match phase {
                Phase::Discovery(mut discovery) => {
                    let turn = discovery.dispatch(now)?;
                    self.early_events.extend(turn.events);
                    reschedule |= turn.reschedule;
                    if turn.complete {
                        let (transport, requests, discovered) = discovery.finish()?;
                        reschedule = true;
                        self.manager(transport, requests, discovered)?
                    } else {
                        Phase::Discovery(discovery)
                    }
                }
                Phase::Manager(mut manager) => {
                    let turn = manager.dispatch(now)?;
                    self.early_events.extend(turn.events);
                    reschedule |= turn.reschedule;
                    if turn.complete {
                        let (mut transport, mut requests, discovered, ids, manager) =
                            manager.finish()?;
                        self.inspector.enumerate(
                            self.windows.as_ref().unwrap(),
                            &mut transport,
                            &mut requests,
                            self.deadline,
                        )?;
                        reschedule = true;
                        let protocols = PropertyReader::new(
                            discovered.atoms["WM_PROTOCOLS"],
                            discovered.atoms["WM_DELETE_WINDOW"],
                            discovered.atoms["WM_TAKE_FOCUS"],
                        )
                        .with_sync_request(discovered.atoms["_NET_WM_SYNC_REQUEST"]);
                        let sync_counter = PropertyReader::new_counter(
                            discovered.atoms["_NET_WM_SYNC_REQUEST_COUNTER"],
                        );
                        let title = PropertyReader::new_text(
                            discovered.atoms["_NET_WM_NAME"],
                            discovered.atoms["UTF8_STRING"],
                            true,
                        );
                        let decorations =
                            PropertyReader::new_decorations(discovered.atoms["_MOTIF_WM_HINTS"]);
                        let app_icon = PropertyReader::new_icon(discovered.atoms["_NET_WM_ICON"]);
                        Phase::Tracking(Tracking {
                            selection_checks: BTreeMap::new(),
                            selection_watches: Vec::new(),
                            transport,
                            requests,
                            discovered,
                            _ids: ids,
                            manager,
                            incoming: VecDeque::new(),
                            commands: Commands::new(),
                            protocols,
                            sync_counter,
                            resize_sync: Default::default(),
                            server_time: 0,
                            hints: PropertyReader::new_hints(),
                            normal_hints: PropertyReader::new_normal_hints(),
                            decorations,
                            frame_extents: BTreeMap::new(),
                            move_resize_requests: Vec::new(),
                            maximize_requests: Vec::new(),
                            minimize_requests: Vec::new(),
                            maximized: BTreeMap::new(),
                            title,
                            app_icon,
                            app_class: PropertyReader::new_text(
                                xproto::AtomEnum::WM_CLASS.into(),
                                xproto::AtomEnum::STRING.into(),
                                false,
                            ),
                            legacy_title: PropertyReader::new_text(
                                xproto::AtomEnum::WM_NAME.into(),
                                xproto::AtomEnum::STRING.into(),
                                false,
                            ),

                            last_focus_request: None,
                        })
                    } else {
                        Phase::Manager(manager)
                    }
                }
                Phase::Tracking(mut tracking) => {
                    for completion in tracking.requests.expire(now)? {
                        if !tracking.resize_sync.completion(&completion, now)
                            && !tracking.sync_counter.completion(
                                &completion,
                                self.windows.as_ref().unwrap(),
                                &mut actions,
                            )?
                            && !tracking.selection_completion(&completion)?
                            && !tracking.commands.completion(
                                &completion,
                                self.windows.as_ref().unwrap(),
                                &mut actions,
                            )?
                            && !tracking.protocols.completion(
                                &completion,
                                self.windows.as_ref().unwrap(),
                                &mut actions,
                            )?
                            && !tracking.hints.completion(
                                &completion,
                                self.windows.as_ref().unwrap(),
                                &mut actions,
                            )?
                            && !tracking.decorations.completion(
                                &completion,
                                self.windows.as_ref().unwrap(),
                                &mut actions,
                            )?
                            && !tracking.normal_hints.completion(
                                &completion,
                                self.windows.as_ref().unwrap(),
                                &mut actions,
                            )?
                            && !tracking.title.completion(
                                &completion,
                                self.windows.as_ref().unwrap(),
                                &mut actions,
                            )?
                            && !tracking.legacy_title.completion(
                                &completion,
                                self.windows.as_ref().unwrap(),
                                &mut actions,
                            )?
                            && !tracking.app_class.completion(
                                &completion,
                                self.windows.as_ref().unwrap(),
                                &mut actions,
                            )?
                            && !tracking.app_icon.completion(
                                &completion,
                                self.windows.as_ref().unwrap(),
                                &mut actions,
                            )?
                            && !self.inspector.completion(
                                &completion,
                                self.windows.as_mut().unwrap(),
                                &mut actions,
                            )?
                        {
                            return Err(Error("unhandled XWM timeout".into()));
                        }
                    }
                    if tracking.incoming.is_empty() {
                        let turn = tracking.transport.dispatch()?;
                        if turn.setup.is_some() {
                            return Err(Error("duplicate XWM setup".into()));
                        }
                        tracking.incoming.extend(turn.packets);
                        reschedule |= turn.reschedule;
                    }
                    for _ in 0..256 {
                        if started.elapsed() >= BUDGET {
                            break;
                        }
                        let Some(packet) = tracking.incoming.pop_front() else {
                            break;
                        };
                        for completion in tracking.requests.ingest(packet)? {
                            if let Completion::Event(bytes) = completion {
                                if bytes[0] == xproto::PROPERTY_NOTIFY_EVENT {
                                    let (e, _) = xproto::PropertyNotifyEvent::try_parse(&bytes)
                                        .map_err(|_| {
                                            Error("malformed XWM property event".into())
                                        })?;
                                    tracking.server_time = e.time;
                                    if let Some(w) = self.windows.as_ref().unwrap().get(e.window) {
                                        if e.atom == tracking.sync_counter.property() {
                                            tracking.resize_sync.forget(w.id);
                                            tracking.sync_counter.refresh(w.id)?;
                                        }
                                        if e.atom == tracking.protocols.property() {
                                            tracking.resize_sync.forget(w.id);
                                        }
                                    }
                                    if e.atom == tracking.protocols.property()
                                        && let Some(w) =
                                            self.windows.as_ref().unwrap().get(e.window)
                                    {
                                        tracking.protocols.refresh(w.id)?;
                                    }
                                    if e.atom == tracking.hints.property()
                                        && let Some(w) =
                                            self.windows.as_ref().unwrap().get(e.window)
                                    {
                                        tracking.hints.refresh(w.id)?;
                                    }
                                    if let Some(w) = self.windows.as_ref().unwrap().get(e.window) {
                                        if e.atom == tracking.decorations.property() {
                                            tracking.decorations.refresh(w.id)?;
                                        }
                                        if e.atom == tracking.title.property() {
                                            tracking.title.refresh(w.id)?;
                                        }
                                        if e.atom == tracking.legacy_title.property() {
                                            tracking.legacy_title.refresh(w.id)?;
                                        }
                                        if e.atom == tracking.app_class.property() {
                                            tracking.app_class.refresh(w.id)?;
                                        }
                                        if e.atom == tracking.app_icon.property() {
                                            tracking.app_icon.refresh(w.id)?;
                                        }
                                    }
                                    if e.atom == tracking.normal_hints.property()
                                        && let Some(w) =
                                            self.windows.as_ref().unwrap().get(e.window)
                                    {
                                        tracking.normal_hints.refresh(w.id)?;
                                    }
                                }
                                if bytes[0] & 0x7f == xproto::CLIENT_MESSAGE_EVENT {
                                    let (e, _) = xproto::ClientMessageEvent::try_parse(&bytes)
                                        .map_err(|_| {
                                            Error("malformed frame extents request".into())
                                        })?;
                                    if e.format == 32
                                        && e.type_ == tracking.discovered.atoms["WM_CHANGE_STATE"]
                                        && e.data.as_data32()[0] == 3
                                        && tracking.minimize_requests.len() < 256
                                    {
                                        if let Some(w) = self
                                            .windows
                                            .as_ref()
                                            .unwrap()
                                            .get(e.window)
                                            .filter(|w| w.mapped && !w.override_redirect)
                                        {
                                            tracking.minimize_requests.push(w.id);
                                        }
                                    }
                                    if e.format == 32
                                        && e.type_ == tracking.discovered.atoms["_NET_WM_STATE"]
                                    {
                                        let data = e.data.as_data32();
                                        let is_maximize = data[1..=2].iter().any(|atom| {
                                            *atom == tracking.discovered.atoms["_NET_WM_STATE_MAXIMIZED_VERT"]
                                                || *atom == tracking.discovered.atoms["_NET_WM_STATE_MAXIMIZED_HORZ"]
                                        });
                                        if data[0] <= 2
                                            && data[3] <= 2
                                            && is_maximize
                                            && tracking.maximize_requests.len() < 256
                                        {
                                            if let Some(w) = self
                                                .windows
                                                .as_ref()
                                                .unwrap()
                                                .get(e.window)
                                                .filter(|w| w.mapped && !w.override_redirect)
                                            {
                                                // Process a two-atom toggle once, not once per axis.
                                                tracking.maximize_requests.push((w.id, data[0]));
                                            }
                                        }
                                    }
                                    if e.format == 32
                                        && e.type_
                                            == tracking.discovered.atoms["_NET_WM_MOVERESIZE"]
                                    {
                                        let data = e.data.as_data32();
                                        if matches!(data[2], 0..=8 | 11)
                                            && data[4] <= 2
                                            && tracking.move_resize_requests.len() < 256
                                        {
                                            if let Some(w) = self
                                                .windows
                                                .as_ref()
                                                .unwrap()
                                                .get(e.window)
                                                .filter(|w| w.mapped && !w.override_redirect)
                                            {
                                                tracking
                                                    .move_resize_requests
                                                    .push((w.id, data[2], data[3]));
                                            }
                                        }
                                    }
                                    if e.format == 32
                                        && e.type_
                                            == tracking.discovered.atoms["_NET_REQUEST_FRAME_EXTENTS"]
                                    {
                                        if let Some(w) =
                                            self.windows.as_ref().unwrap().get(e.window)
                                        {
                                            tracking.frame_extents.remove(&w.id);
                                        }
                                    }
                                }
                                if bytes[0] == xproto::SELECTION_CLEAR_EVENT {
                                    let (e, _) = xproto::SelectionClearEvent::try_parse(&bytes)
                                        .map_err(|_| {
                                            Error("malformed manager selection loss".into())
                                        })?;
                                    if e.owner == tracking.manager
                                        && [
                                            tracking.discovered.atoms["WM_S0"],
                                            tracking.discovered.atoms["_NET_WM_CM_S0"],
                                        ]
                                        .contains(&e.selection)
                                    {
                                        return Err(Error(
                                            "XWM manager selection ownership lost".into(),
                                        ));
                                    }
                                }
                                self.event(bytes, &mut actions, &mut events)?;
                            } else if !tracking.resize_sync.completion(&completion, now)
                                && !tracking.sync_counter.completion(
                                    &completion,
                                    self.windows.as_ref().unwrap(),
                                    &mut actions,
                                )?
                                && !tracking.selection_completion(&completion)?
                                && !tracking.commands.completion(
                                    &completion,
                                    self.windows.as_ref().unwrap(),
                                    &mut actions,
                                )?
                                && !tracking.protocols.completion(
                                    &completion,
                                    self.windows.as_ref().unwrap(),
                                    &mut actions,
                                )?
                                && !tracking.hints.completion(
                                    &completion,
                                    self.windows.as_ref().unwrap(),
                                    &mut actions,
                                )?
                                && !tracking.decorations.completion(
                                    &completion,
                                    self.windows.as_ref().unwrap(),
                                    &mut actions,
                                )?
                                && !tracking.normal_hints.completion(
                                    &completion,
                                    self.windows.as_ref().unwrap(),
                                    &mut actions,
                                )?
                                && !tracking.title.completion(
                                    &completion,
                                    self.windows.as_ref().unwrap(),
                                    &mut actions,
                                )?
                                && !tracking.legacy_title.completion(
                                    &completion,
                                    self.windows.as_ref().unwrap(),
                                    &mut actions,
                                )?
                                && !tracking.app_class.completion(
                                    &completion,
                                    self.windows.as_ref().unwrap(),
                                    &mut actions,
                                )?
                                && !tracking.app_icon.completion(
                                    &completion,
                                    self.windows.as_ref().unwrap(),
                                    &mut actions,
                                )?
                                && !self.inspector.completion(
                                    &completion,
                                    self.windows.as_mut().unwrap(),
                                    &mut actions,
                                )?
                            {
                                return Err(Error("unhandled XWM reply".into()));
                            }
                        }
                    }
                    for action in &actions {
                        if let Action::CommandFailed(window) = action {
                            tracking.frame_extents.remove(window);
                            tracking.maximized.remove(window);
                        }
                        if let Action::Destroyed(window) = action {
                            tracking.resize_sync.forget(*window);
                            tracking.sync_counter.forget(*window);
                            tracking.protocols.forget(*window);
                            tracking.hints.forget(*window);
                            tracking.normal_hints.forget(*window);
                            tracking.decorations.forget(*window);
                            tracking.frame_extents.remove(window);
                            tracking.maximized.remove(window);
                            tracking.title.forget(*window);
                            tracking.legacy_title.forget(*window);
                            tracking.app_class.forget(*window);
                            tracking.app_icon.forget(*window);
                        }
                    }
                    if let Some(extension) = tracking.discovered.extensions.get("SYNC") {
                        for action in &actions {
                            let window = match action {
                                Action::Changed(w) | Action::ProtocolsChanged(w) => *w,
                                _ => continue,
                            };
                            if self
                                .windows
                                .as_ref()
                                .unwrap()
                                .get(window.xid)
                                .is_some_and(|w| w.id == window && !w.override_redirect)
                            {
                                if matches!(action, Action::ProtocolsChanged(_))
                                    && tracking
                                        .protocols
                                        .get(window)
                                        .is_some_and(|p| p.sync_request)
                                {
                                    tracking.sync_counter.refresh(window)?;
                                }
                                if !self
                                    .windows
                                    .as_ref()
                                    .unwrap()
                                    .get(window.xid)
                                    .unwrap()
                                    .mapped
                                {
                                    tracking.resize_sync.cancel(window);
                                }
                                let counter = tracking
                                    .protocols
                                    .get(window)
                                    .filter(|p| p.sync_request)
                                    .and_then(|_| tracking.sync_counter.counter(window));
                                tracking.resize_sync.capability(window, counter, now)?;
                            }
                        }
                        reschedule |= tracking.resize_sync.schedule(
                            extension.major_opcode,
                            &mut tracking.transport,
                            &mut tracking.requests,
                            now,
                            started + BUDGET,
                        )?;
                    }
                    reschedule |= tracking.sync_counter.schedule(
                        &mut tracking.transport,
                        &mut tracking.requests,
                        now + Duration::from_secs(1),
                        started + BUDGET,
                    )?;
                    reschedule |= tracking.protocols.schedule(
                        &mut tracking.transport,
                        &mut tracking.requests,
                        now + Duration::from_secs(10),
                        started + BUDGET,
                    )?;
                    reschedule |= tracking.hints.schedule(
                        &mut tracking.transport,
                        &mut tracking.requests,
                        now + Duration::from_secs(10),
                        started + BUDGET,
                    )?;
                    reschedule |= tracking.decorations.schedule(
                        &mut tracking.transport,
                        &mut tracking.requests,
                        now + Duration::from_secs(10),
                        started + BUDGET,
                    )?;
                    reschedule |= tracking.normal_hints.schedule(
                        &mut tracking.transport,
                        &mut tracking.requests,
                        now + Duration::from_secs(10),
                        started + BUDGET,
                    )?;
                    reschedule |= tracking.title.schedule(
                        &mut tracking.transport,
                        &mut tracking.requests,
                        now + Duration::from_secs(10),
                        started + BUDGET,
                    )?;
                    reschedule |= tracking.legacy_title.schedule(
                        &mut tracking.transport,
                        &mut tracking.requests,
                        now + Duration::from_secs(10),
                        started + BUDGET,
                    )?;
                    reschedule |= tracking.app_class.schedule(
                        &mut tracking.transport,
                        &mut tracking.requests,
                        now + Duration::from_secs(10),
                        started + BUDGET,
                    )?;
                    reschedule |= tracking.app_icon.schedule(
                        &mut tracking.transport,
                        &mut tracking.requests,
                        now + Duration::from_secs(10),
                        started + BUDGET,
                    )?;
                    // Event processing and snapshot scheduling occupy separate turns,
                    // preserving connection event order before newly queued reads.
                    if started.elapsed() < BUDGET && tracking.incoming.is_empty() {
                        reschedule |= self.inspector.schedule_until(
                            self.windows.as_mut().unwrap(),
                            &mut tracking.transport,
                            &mut tracking.requests,
                            now + Duration::from_secs(10),
                            started + BUDGET,
                        )?;
                    } else {
                        reschedule = true;
                    }
                    Phase::Tracking(tracking)
                }
            }
        };
        if self.early_events.len() > CAPACITY {
            return Err(Error("XWM initialization event bound reached".into()));
        }
        reschedule |= self.windows.is_some() && !self.early_events.is_empty();
        let writable_interest = match &phase {
            Phase::Discovery(p) => p.wants_write(),
            Phase::Manager(p) => p.wants_write(),
            Phase::Tracking(p) => p.transport.wants_write(),
        };
        self.phase = Some(phase);
        Ok(Turn {
            actions,
            events,
            reschedule,
            writable_interest,
            enumerated: self.inspector.enumeration_complete(),
        })
    }
}

// Compare serial times only within their unambiguous half-range. This tracks
// compositor requests, not server-confirmed focus or proof of user interaction.
fn focus_timestamp_is_current(
    previous: Option<(u32, Instant)>,
    timestamp: u32,
    now: Instant,
) -> bool {
    if timestamp == 0 {
        return false;
    }
    let Some((last, observed)) = previous else {
        return true;
    };
    if now
        .checked_duration_since(observed)
        .is_some_and(|age| age >= Duration::from_millis(1u64 << 31))
    {
        return true; // Old serial history cannot order a newly authorized interaction.
    }
    timestamp.wrapping_sub(last) < (1u32 << 31)
}

#[cfg(test)]
mod tests;

//! Asynchronous manager selection acquisition after discovery. Ownership is only
//! an initialization phase, never sufficient to publish desktop readiness.
use super::{
    Error, Result,
    discovery::Discovered,
    requests::{Completion, Importance, ReplyKind, Requests},
    transport::Transport,
};
use std::{
    borrow::Cow,
    collections::{BTreeMap, VecDeque},
    os::fd::RawFd,
    time::{Duration, Instant},
};
use x11rb_protocol::{
    id_allocator::IdAllocator,
    protocol::{composite, render, xproto},
    x11_utils::{Request, Serialize, TryParse},
};

#[derive(Clone, Copy)]
enum Pending {
    Checked,
    CompositeBarrier,
    CursorExtension,
    CursorVersion(u8),
    CursorFormats(u8),
    Existing(usize),
    Verify(usize),
    Barrier,
}
pub struct Manager {
    pub(super) root_cursor: Option<super::root_cursor::RootCursor>,
    announced: bool,
    transport: Transport,
    requests: Requests,
    discovered: Discovered,
    ids: IdAllocator,
    window: u32,
    selections: [u32; 2],
    timestamp: Option<u32>,
    existing: [bool; 2],
    verified: [bool; 2],
    claiming: bool,
    redirected: bool,
    complete: bool,
    failed: bool,
    pending: BTreeMap<u64, Pending>,
    incoming: VecDeque<Vec<u8>>,
    deadline: Instant,
}
impl Manager {
    /// Keep the original startup deadline when transitioning from discovery.
    pub fn new(
        transport: Transport,
        requests: Requests,
        discovered: Discovered,
        deadline: Instant,
    ) -> Result<Self> {
        let setup = &discovered.setup;
        if setup.roots.len() != 1
            || setup.resource_id_base == 0
            || setup.resource_id_base & setup.resource_id_mask != 0
            || (setup.resource_id_base | setup.resource_id_mask) >= 0x80000000
        {
            return Err(Error("invalid XWM resource ID range".into()));
        }
        let mask = setup.resource_id_mask;
        let shifted = mask >> mask.trailing_zeros().min(31);
        if mask == 0 || shifted & shifted.wrapping_add(1) != 0 {
            return Err(Error("noncontiguous XWM resource ID mask".into()));
        }
        let mut ids = IdAllocator::new(setup.resource_id_base, setup.resource_id_mask)
            .map_err(|_| Error("invalid XWM resource ID mask".into()))?;
        let window = ids
            .generate_id()
            .ok_or_else(|| Error("exhausted XWM resource IDs".into()))?;
        let atom = |name| {
            discovered
                .atoms
                .get(name)
                .copied()
                .ok_or_else(|| Error("missing manager atom".into()))
        };
        let selections = [atom("WM_S0")?, atom("_NET_WM_CM_S0")?];
        atom("_NET_WM_NAME")?;
        atom("UTF8_STRING")?;
        atom("MANAGER")?;
        atom("_NET_SUPPORTING_WM_CHECK")?;
        atom("_NET_SUPPORTED")?;
        let root = setup.roots[0].root;
        let mut this = Self {
            root_cursor: None,
            announced: false,
            transport,
            requests,
            discovered,
            ids,
            window,
            selections,
            timestamp: None,
            existing: [false; 2],
            verified: [false; 2],
            claiming: false,
            redirected: false,
            complete: false,
            failed: false,
            pending: BTreeMap::new(),
            incoming: VecDeque::new(),
            deadline,
        };
        this.send(
            xproto::CreateWindowRequest {
                depth: 0,
                wid: window,
                parent: root,
                x: 0,
                y: 0,
                width: 1,
                height: 1,
                border_width: 0,
                class: xproto::WindowClass::INPUT_OUTPUT,
                visual: 0,
                value_list: Cow::Owned(
                    xproto::CreateWindowAux::new().event_mask(xproto::EventMask::PROPERTY_CHANGE),
                ),
            },
            ReplyKind::Void,
            Pending::Checked,
        )?;
        this.send(
            xproto::ChangePropertyRequest {
                mode: xproto::PropMode::REPLACE,
                window,
                property: this.discovered.atoms["_NET_WM_NAME"],
                type_: this.discovered.atoms["UTF8_STRING"],
                format: 8,
                data_len: 8,
                data: Cow::Borrowed(b"Telorgon"),
            },
            ReplyKind::Void,
            Pending::Checked,
        )?;
        for (index, selection) in selections.into_iter().enumerate() {
            this.send(
                xproto::GetSelectionOwnerRequest { selection },
                ReplyKind::Reply,
                Pending::Existing(index),
            )?;
        }
        let composite = this
            .discovered
            .extensions
            .get("Composite")
            .filter(|extension| {
                extension.major_opcode >= 128
                    && extension.version.0 == 0
                    && extension.version.1 >= 4
            })
            .ok_or_else(|| Error("Composite 0.4 was not negotiated".into()))?;
        this.send_at(
            composite::RedirectSubwindowsRequest {
                window: root,
                update: composite::Redirect::MANUAL,
            },
            composite.major_opcode,
            ReplyKind::Void,
            Pending::Checked,
        )?;
        this.send(
            xproto::GetInputFocusRequest,
            ReplyKind::Reply,
            Pending::CompositeBarrier,
        )?;
        Ok(this)
    }
    /// Set only the inherited root default; never override an application's cursor.
    fn initialize_root_cursor(&mut self) -> Result<()> {
        let font = self
            .ids
            .generate_id()
            .ok_or_else(|| Error("exhausted XWM resource IDs".into()))?;
        let cursor = self
            .ids
            .generate_id()
            .ok_or_else(|| Error("exhausted XWM resource IDs".into()))?;
        self.send(
            xproto::OpenFontRequest {
                fid: font,
                name: Cow::Borrowed(b"cursor"),
            },
            ReplyKind::Void,
            Pending::Checked,
        )?;
        self.send(
            xproto::CreateGlyphCursorRequest {
                cid: cursor,
                source_font: font,
                mask_font: font,
                source_char: 68,
                mask_char: 69,
                fore_red: 0,
                fore_green: 0,
                fore_blue: 0,
                back_red: u16::MAX,
                back_green: u16::MAX,
                back_blue: u16::MAX,
            },
            ReplyKind::Void,
            Pending::Checked,
        )?;
        self.send(
            xproto::ChangeWindowAttributesRequest {
                window: self.discovered.setup.roots[0].root,
                value_list: Cow::Owned(xproto::ChangeWindowAttributesAux::new().cursor(cursor)),
            },
            ReplyKind::Void,
            Pending::Checked,
        )?;
        // The window retains the cursor; the cursor retains the glyph data.
        self.send(
            xproto::FreeCursorRequest { cursor },
            ReplyKind::Void,
            Pending::Checked,
        )?;
        self.send(
            xproto::CloseFontRequest { font },
            ReplyKind::Void,
            Pending::Checked,
        )?;
        Ok(())
    }
    fn cursor_barrier(&mut self) -> Result<()> {
        self.send(
            xproto::GetInputFocusRequest,
            ReplyKind::Reply,
            Pending::Barrier,
        )
    }
    fn upload_root_cursor(&mut self, opcode: u8, format: u32) -> Result<()> {
        let image = self
            .root_cursor
            .take()
            .ok_or_else(|| Error("missing root cursor pixels".into()))?;
        let setup = &self.discovered.setup;
        if !setup
            .pixmap_formats
            .iter()
            .any(|f| f.depth == 32 && f.bits_per_pixel == 32 && f.scanline_pad == 32)
        {
            return Err(Error(
                "X11 cursor requires a 32-bit pixmap upload format".into(),
            ));
        }
        let bytes = image.bytes(setup.image_byte_order == xproto::ImageOrder::LSB_FIRST);
        let row_bytes = usize::from(image.width) * 4;
        let max_bytes = usize::from(setup.maximum_request_length) * 4;
        let rows = max_bytes.saturating_sub(24) / row_bytes;
        if rows == 0 {
            return Err(Error("X11 request limit cannot fit a cursor row".into()));
        }
        let root = setup.roots[0].root;
        let mut allocate = || {
            self.ids
                .generate_id()
                .ok_or_else(|| Error("exhausted cursor resource IDs".into()))
        };
        let pixmap = allocate()?;
        let gc = allocate()?;
        let picture = allocate()?;
        let cursor = allocate()?;
        self.send(
            xproto::CreatePixmapRequest {
                depth: 32,
                pid: pixmap,
                drawable: root,
                width: image.width,
                height: image.height,
            },
            ReplyKind::Void,
            Pending::Checked,
        )?;
        self.send(
            xproto::CreateGCRequest {
                cid: gc,
                drawable: pixmap,
                value_list: Cow::Owned(xproto::CreateGCAux::new()),
            },
            ReplyKind::Void,
            Pending::Checked,
        )?;
        for y in (0..usize::from(image.height)).step_by(rows) {
            let height = rows.min(usize::from(image.height) - y);
            self.send(
                xproto::PutImageRequest {
                    format: xproto::ImageFormat::Z_PIXMAP,
                    drawable: pixmap,
                    gc,
                    width: image.width,
                    height: height as u16,
                    dst_x: 0,
                    dst_y: y as i16,
                    left_pad: 0,
                    depth: 32,
                    data: Cow::Owned(bytes[y * row_bytes..(y + height) * row_bytes].to_vec()),
                },
                ReplyKind::Void,
                Pending::Checked,
            )?;
        }
        self.send_at(
            render::CreatePictureRequest {
                pid: picture,
                drawable: pixmap,
                format,
                value_list: Cow::Owned(render::CreatePictureAux::new()),
            },
            opcode,
            ReplyKind::Void,
            Pending::Checked,
        )?;
        self.send_at(
            render::CreateCursorRequest {
                cid: cursor,
                source: picture,
                x: image.x,
                y: image.y,
            },
            opcode,
            ReplyKind::Void,
            Pending::Checked,
        )?;
        self.send(
            xproto::ChangeWindowAttributesRequest {
                window: root,
                value_list: Cow::Owned(xproto::ChangeWindowAttributesAux::new().cursor(cursor)),
            },
            ReplyKind::Void,
            Pending::Checked,
        )?;
        self.send(
            xproto::FreeCursorRequest { cursor },
            ReplyKind::Void,
            Pending::Checked,
        )?;
        self.send_at(
            render::FreePictureRequest { picture },
            opcode,
            ReplyKind::Void,
            Pending::Checked,
        )?;
        self.send(
            xproto::FreeGCRequest { gc },
            ReplyKind::Void,
            Pending::Checked,
        )?;
        self.send(
            xproto::FreePixmapRequest { pixmap },
            ReplyKind::Void,
            Pending::Checked,
        )?;
        Ok(())
    }
    pub fn wants_write(&self) -> bool {
        self.transport.wants_write()
    }
    pub fn fd(&self) -> RawFd {
        self.transport.fd()
    }
    pub fn window_id(&self) -> u32 {
        self.window
    }
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
    pub fn is_complete(&self) -> bool {
        self.complete && !self.failed && self.incoming.is_empty()
    }
    /// Preserve the allocated manager window and allocator for subsequent XWM work.
    /// The next phase must continue rejecting SelectionClear for both selections.
    pub fn finish(self) -> Result<(Transport, Requests, Discovered, IdAllocator, u32)> {
        if !self.is_complete() {
            return Err(Error("XWM manager acquisition is not complete".into()));
        }
        Ok((
            self.transport,
            self.requests,
            self.discovered,
            self.ids,
            self.window,
        ))
    }
    fn send(&mut self, request: impl Request, kind: ReplyKind, pending: Pending) -> Result<()> {
        self.send_at(request, 0, kind, pending)
    }
    fn send_at(
        &mut self,
        request: impl Request,
        opcode: u8,
        kind: ReplyKind,
        pending: Pending,
    ) -> Result<()> {
        let (bytes, fds) = Request::serialize(request, opcode);
        if !fds.is_empty() {
            return Err(Error("unexpected manager request descriptors".into()));
        }
        let id = self.requests.queue(
            &mut self.transport,
            bytes,
            kind,
            Importance::Essential,
            self.deadline,
        )?;
        self.pending.insert(id.sequence, pending);
        Ok(())
    }
    fn property32(
        &mut self,
        window: u32,
        property: u32,
        type_: xproto::AtomEnum,
        values: &[u32],
    ) -> Result<()> {
        let data = values
            .iter()
            .flat_map(|value| value.to_ne_bytes())
            .collect();
        self.send(
            xproto::ChangePropertyRequest {
                mode: xproto::PropMode::REPLACE,
                window,
                property,
                type_: type_.into(),
                format: 32,
                data_len: values.len() as u32,
                data: Cow::Owned(data),
            },
            ReplyKind::Void,
            Pending::Checked,
        )
    }
    /// Publish only the EWMH metadata actually maintained by this phase. Extend
    /// this list alongside implemented handlers, never from a desired feature list.
    fn publish_metadata(&mut self) -> Result<()> {
        let root = self.discovered.setup.roots[0].root;
        if let Some(image) = &self.root_cursor {
            let resources = format!(
                "Xft.dpi: {}\nXcursor.size: {}\n",
                96 * u32::from(image.density),
                24 * u32::from(image.density)
            );
            self.send(
                xproto::ChangePropertyRequest {
                    mode: xproto::PropMode::REPLACE,
                    window: root,
                    property: xproto::AtomEnum::RESOURCE_MANAGER.into(),
                    type_: xproto::AtomEnum::STRING.into(),
                    format: 8,
                    data_len: resources.len() as u32,
                    data: Cow::Owned(resources.into_bytes()),
                },
                ReplyKind::Void,
                Pending::Checked,
            )?;
        }
        let check = self.discovered.atoms["_NET_SUPPORTING_WM_CHECK"];
        let supported = self.discovered.atoms["_NET_SUPPORTED"];
        self.property32(self.window, check, xproto::AtomEnum::WINDOW, &[self.window])?;
        self.property32(root, check, xproto::AtomEnum::WINDOW, &[self.window])?;
        let mut capabilities = vec![
            supported,
            check,
            self.discovered.atoms["_NET_WM_NAME"],
            self.discovered.atoms["_NET_FRAME_EXTENTS"],
            self.discovered.atoms["_NET_REQUEST_FRAME_EXTENTS"],
            self.discovered.atoms["_NET_WM_MOVERESIZE"],
            self.discovered.atoms["_NET_WM_STATE"],
            self.discovered.atoms["_NET_WM_STATE_MAXIMIZED_VERT"],
            self.discovered.atoms["_NET_WM_STATE_MAXIMIZED_HORZ"],
        ];
        if self.discovered.extensions.contains_key("SYNC") {
            capabilities.extend([
                self.discovered.atoms["_NET_WM_SYNC_REQUEST"],
                self.discovered.atoms["_NET_WM_SYNC_REQUEST_COUNTER"],
            ]);
        }
        self.property32(root, supported, xproto::AtomEnum::ATOM, &capabilities)
    }
    fn completion(&mut self, completion: Completion, events: &mut Vec<Vec<u8>>) -> Result<()> {
        match completion {
            Completion::Event(bytes) => {
                if bytes.first() == Some(&xproto::SELECTION_CLEAR_EVENT) {
                    let (event, _) = xproto::SelectionClearEvent::try_parse(&bytes)
                        .map_err(|_| Error("invalid manager selection event".into()))?;
                    if event.owner == self.window && self.selections.contains(&event.selection) {
                        return Err(Error("XWM manager selection ownership lost".into()));
                    }
                }
                if bytes.first() == Some(&xproto::PROPERTY_NOTIFY_EVENT) {
                    let (event, _) = xproto::PropertyNotifyEvent::try_parse(&bytes)
                        .map_err(|_| Error("invalid manager timestamp event".into()))?;
                    if self.timestamp.is_none()
                        && event.window == self.window
                        && event.atom == self.discovered.atoms["_NET_WM_NAME"]
                        && event.state == xproto::Property::NEW_VALUE
                        && event.time != 0
                    {
                        self.timestamp = Some(event.time);
                    } else {
                        events.push(bytes);
                    }
                } else {
                    events.push(bytes);
                }
            }
            Completion::Reply(id, bytes) => {
                match self
                    .pending
                    .remove(&id.sequence)
                    .ok_or_else(|| Error("unexpected manager reply".into()))?
                {
                    Pending::Existing(index) | Pending::Verify(index) => {
                        let (reply, _) = xproto::GetSelectionOwnerReply::try_parse(&bytes)
                            .map_err(|_| Error("invalid manager ownership reply".into()))?;
                        if self.claiming {
                            if reply.owner != self.window {
                                return Err(Error(
                                    "XWM manager ownership verification failed".into(),
                                ));
                            }
                            self.verified[index] = true;
                        } else {
                            if reply.owner != 0 {
                                return Err(Error("XWM manager selection already owned".into()));
                            }
                            self.existing[index] = true;
                        }
                    }
                    Pending::CompositeBarrier => {
                        xproto::GetInputFocusReply::try_parse(&bytes).map_err(|_| {
                            Error("invalid Composite redirection barrier reply".into())
                        })?;
                        self.redirected = true;
                    }
                    Pending::Barrier => {
                        xproto::GetInputFocusReply::try_parse(&bytes)
                            .map_err(|_| Error("invalid manager barrier reply".into()))?;
                        self.complete = true;
                    }
                    Pending::CursorExtension => {
                        let (reply, _) = xproto::QueryExtensionReply::try_parse(&bytes)
                            .map_err(|_| Error("invalid cursor RENDER extension reply".into()))?;
                        if !reply.present || reply.major_opcode < 128 {
                            return Err(Error("themed X11 cursor requires RENDER".into()));
                        }
                        self.send_at(
                            render::QueryVersionRequest {
                                client_major_version: 0,
                                client_minor_version: 5,
                            },
                            reply.major_opcode,
                            ReplyKind::Reply,
                            Pending::CursorVersion(reply.major_opcode),
                        )?;
                    }
                    Pending::CursorVersion(opcode) => {
                        let (reply, _) = render::QueryVersionReply::try_parse(&bytes)
                            .map_err(|_| Error("invalid cursor RENDER version reply".into()))?;
                        if (reply.major_version, reply.minor_version) < (0, 5) {
                            return Err(Error("themed X11 cursor requires RENDER 0.5".into()));
                        }
                        self.send_at(
                            render::QueryPictFormatsRequest,
                            opcode,
                            ReplyKind::Reply,
                            Pending::CursorFormats(opcode),
                        )?;
                    }
                    Pending::CursorFormats(opcode) => {
                        let (reply, _) = render::QueryPictFormatsReply::try_parse(&bytes)
                            .map_err(|_| Error("invalid cursor picture formats".into()))?;
                        let format = reply
                            .formats
                            .iter()
                            .find(|f| {
                                f.type_ == render::PictType::DIRECT
                                    && f.depth == 32
                                    && f.direct.red_shift == 16
                                    && f.direct.red_mask == 255
                                    && f.direct.green_shift == 8
                                    && f.direct.green_mask == 255
                                    && f.direct.blue_shift == 0
                                    && f.direct.blue_mask == 255
                                    && f.direct.alpha_shift == 24
                                    && f.direct.alpha_mask == 255
                            })
                            .ok_or_else(|| Error("X11 cursor ARGB32 format unavailable".into()))?
                            .id;
                        self.upload_root_cursor(opcode, format)?;
                        self.cursor_barrier()?;
                    }
                    Pending::Checked => return Err(Error("unexpected manager void reply".into())),
                }
            }
            Completion::Checked(id) => {
                self.pending
                    .remove(&id.sequence)
                    .ok_or_else(|| Error("unexpected manager checked request".into()))?;
            }
            Completion::Error(_, _) | Completion::TimedOut(_) => {
                return Err(Error("manager request failed".into()));
            }
        }
        if !self.claiming
            && self.redirected
            && self.existing == [true; 2]
            && let Some(time) = self.timestamp
        {
            self.claiming = true;
            for (index, selection) in self.selections.into_iter().enumerate() {
                self.send(
                    xproto::SetSelectionOwnerRequest {
                        owner: self.window,
                        selection,
                        time,
                    },
                    ReplyKind::Void,
                    Pending::Checked,
                )?;
                self.send(
                    xproto::GetSelectionOwnerRequest { selection },
                    ReplyKind::Reply,
                    Pending::Verify(index),
                )?;
            }
        }
        if self.verified == [true; 2] && !self.announced {
            self.announced = true;
            self.publish_metadata()?;
            let root = self.discovered.setup.roots[0].root;
            for selection in self.selections {
                let event = xproto::ClientMessageEvent {
                    response_type: xproto::CLIENT_MESSAGE_EVENT,
                    format: 32,
                    sequence: 0,
                    window: root,
                    type_: self.discovered.atoms["MANAGER"],
                    data: [self.timestamp.unwrap(), selection, self.window, 0, 0].into(),
                }
                .serialize();
                self.send(
                    xproto::SendEventRequest {
                        propagate: false,
                        destination: root,
                        event_mask: xproto::EventMask::STRUCTURE_NOTIFY,
                        event: Cow::Owned(event),
                    },
                    ReplyKind::Void,
                    Pending::Checked,
                )?;
            }
            if self.root_cursor.is_some() {
                self.send(
                    xproto::QueryExtensionRequest {
                        name: Cow::Borrowed(b"RENDER"),
                    },
                    ReplyKind::Reply,
                    Pending::CursorExtension,
                )?;
            } else {
                self.initialize_root_cursor()?;
                self.cursor_barrier()?;
            }
        }
        Ok(())
    }
    /// Use both the returned reschedule flag and writable interest in the owner loop.
    pub fn dispatch(&mut self, now: Instant) -> Result<super::discovery::Turn> {
        let result = self.dispatch_inner(now);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn dispatch_inner(&mut self, now: Instant) -> Result<super::discovery::Turn> {
        if self.failed || (!self.complete && now >= self.deadline) {
            return Err(Error("XWM manager acquisition failed or timed out".into()));
        }
        self.requests.expire(now)?;
        let started = Instant::now();
        let mut reschedule = false;
        if self.incoming.is_empty() {
            let turn = self.transport.dispatch()?;
            if turn.setup.is_some() {
                return Err(Error("duplicate XWM setup".into()));
            }
            reschedule = turn.reschedule;
            self.incoming.extend(turn.packets);
        }
        let mut events = vec![];
        for _ in 0..256 {
            if started.elapsed() >= Duration::from_millis(1) {
                break;
            }
            let Some(packet) = self.incoming.pop_front() else {
                break;
            };
            for completion in self.requests.ingest(packet)? {
                self.completion(completion, &mut events)?;
            }
        }
        Ok(super::discovery::Turn {
            events,
            reschedule: reschedule || !self.incoming.is_empty(),
            writable_interest: self.transport.wants_write(),
            complete: self.is_complete(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::discovery::tests::{ready, request, write_reply};
    use super::*;
    use std::os::unix::net::UnixStream;
    fn start() -> (Manager, UnixStream, Instant) {
        let (transport, requests, discovered, mut peer, now) = ready();
        let mut manager = Manager::new(
            transport,
            requests,
            discovered,
            now + Duration::from_secs(10),
        )
        .unwrap();
        manager.dispatch(now).unwrap();
        for opcode in [1, 18, 23, 23] {
            assert_eq!(request(&mut peer)[0], opcode);
        }
        let bytes = request(&mut peer);
        assert_eq!(&bytes[..2], &[128, 2]); // Negotiated Composite, RedirectSubwindows.
        assert_eq!(u32::from_ne_bytes(bytes[4..8].try_into().unwrap()), 1);
        assert_eq!(bytes[8], 1); // Manual; the root itself is not redirected.
        assert_eq!(request(&mut peer)[0], 43);
        (manager, peer, now)
    }
    fn owners(peer: &mut UnixStream, first: u16, owner: u32) {
        for sequence in [first, first + 1] {
            write_reply(
                peer,
                &xproto::GetSelectionOwnerReply {
                    sequence,
                    owner,
                    ..Default::default()
                }
                .serialize(),
            );
        }
        write_reply(
            peer,
            &xproto::GetInputFocusReply {
                sequence: 46,
                ..Default::default()
            }
            .serialize(),
        );
    }
    fn timestamp(manager: &Manager, peer: &mut UnixStream, synthetic: bool) {
        write_reply(
            peer,
            &xproto::PropertyNotifyEvent {
                response_type: xproto::PROPERTY_NOTIFY_EVENT | if synthetic { 128 } else { 0 },
                sequence: 46,
                window: manager.window,
                atom: manager.discovered.atoms["_NET_WM_NAME"],
                time: 123,
                state: xproto::Property::NEW_VALUE,
            }
            .serialize(),
        );
    }
    fn pump(manager: &mut Manager, now: Instant) {
        for _ in 0..10 {
            manager.dispatch(now).unwrap();
        }
    }
    fn check_metadata(manager: &Manager, peer: &mut UnixStream) {
        let check = manager.discovered.atoms["_NET_SUPPORTING_WM_CHECK"];
        let supported = manager.discovered.atoms["_NET_SUPPORTED"];
        for (window, property, type_, values) in [
            (manager.window, check, 33, vec![manager.window]),
            (1, check, 33, vec![manager.window]),
            (
                1,
                supported,
                4,
                vec![
                    supported,
                    check,
                    manager.discovered.atoms["_NET_WM_NAME"],
                    manager.discovered.atoms["_NET_FRAME_EXTENTS"],
                    manager.discovered.atoms["_NET_REQUEST_FRAME_EXTENTS"],
                    manager.discovered.atoms["_NET_WM_MOVERESIZE"],
                    manager.discovered.atoms["_NET_WM_STATE"],
                    manager.discovered.atoms["_NET_WM_STATE_MAXIMIZED_VERT"],
                    manager.discovered.atoms["_NET_WM_STATE_MAXIMIZED_HORZ"],
                    manager.discovered.atoms["_NET_WM_SYNC_REQUEST"],
                    manager.discovered.atoms["_NET_WM_SYNC_REQUEST_COUNTER"],
                ],
            ),
        ] {
            let bytes = request(peer);
            assert_eq!(&bytes[..2], &[18, 0]); // ChangeProperty, Replace
            assert_eq!(u32::from_ne_bytes(bytes[4..8].try_into().unwrap()), window);
            assert_eq!(
                u32::from_ne_bytes(bytes[8..12].try_into().unwrap()),
                property
            );
            assert_eq!(u32::from_ne_bytes(bytes[12..16].try_into().unwrap()), type_);
            assert_eq!(bytes[16], 32);
            assert_eq!(
                u32::from_ne_bytes(bytes[20..24].try_into().unwrap()) as usize,
                values.len()
            );
            assert_eq!(
                bytes[24..]
                    .chunks_exact(4)
                    .map(|b| u32::from_ne_bytes(b.try_into().unwrap()))
                    .collect::<Vec<_>>(),
                values
            );
        }
    }
    #[test]
    fn themed_cursor_upload_uses_argb_hotspot_and_checked_resource_lifetimes() {
        let (mut manager, mut peer, now) = start();
        manager.discovered.setup.pixmap_formats = vec![xproto::Format {
            depth: 32,
            bits_per_pixel: 32,
            scanline_pad: 32,
        }];
        manager.discovered.setup.maximum_request_length = 65535;
        manager.discovered.setup.image_byte_order = xproto::ImageOrder::LSB_FIRST;
        manager.root_cursor = Some(
            super::super::root_cursor::RootCursor::new(
                2,
                1,
                1,
                0,
                &[200, 100, 50, 128, 0, 0, 0, 0],
                false,
            )
            .unwrap(),
        );
        manager.upload_root_cursor(140, 77).unwrap();
        let failed_sequence = manager.requests.last_sequence() - 5; // RENDER CreateCursor
        manager.cursor_barrier().unwrap();
        pump(&mut manager, now);
        assert!(!manager.is_complete());
        let pixmap = request(&mut peer);
        assert_eq!(&pixmap[..2], &[53, 32]);
        assert_eq!(request(&mut peer)[0], 55);
        let image = request(&mut peer);
        assert_eq!(image[0], 72);
        assert_eq!(&image[24..], &[25, 50, 100, 128, 0, 0, 0, 0]);
        let picture = request(&mut peer);
        assert_eq!(&picture[..2], &[140, 4]);
        assert_eq!(u32::from_ne_bytes(picture[12..16].try_into().unwrap()), 77);
        let cursor = request(&mut peer);
        assert_eq!(&cursor[..2], &[140, 27]);
        assert_eq!(u16::from_ne_bytes(cursor[12..14].try_into().unwrap()), 1);
        let root = request(&mut peer);
        assert_eq!(root[0], 2);
        assert_eq!(u32::from_ne_bytes(root[4..8].try_into().unwrap()), 1);
        assert_eq!(&root[12..16], &cursor[4..8]);
        for opcode in [95, 140, 60, 54, 43] {
            assert_eq!(request(&mut peer)[0], opcode);
        }
        let mut error = [0u8; 32];
        error[1] = 2;
        error[2..4].copy_from_slice(&(failed_sequence as u16).to_ne_bytes());
        error[10] = 140;
        write_reply(&mut peer, &error);
        assert!(manager.dispatch(now).is_err());
        assert!(!manager.is_complete());
    }
    #[test]
    fn composite_barrier_is_required_even_with_timestamp_and_free_selections() {
        let (mut manager, mut peer, now) = start();
        for sequence in [43, 44] {
            write_reply(
                &mut peer,
                &xproto::GetSelectionOwnerReply {
                    sequence,
                    owner: 0,
                    ..Default::default()
                }
                .serialize(),
            );
        }
        timestamp(&manager, &mut peer, false);
        pump(&mut manager, now);
        assert_eq!(manager.existing, [true; 2]);
        assert_eq!(manager.timestamp, Some(123));
        assert!(!manager.redirected);
        assert!(!manager.claiming);
        assert!(!manager.transport.wants_write());
        write_reply(
            &mut peer,
            &xproto::GetInputFocusReply {
                sequence: 46,
                ..Default::default()
            }
            .serialize(),
        );
        pump(&mut manager, now);
        assert!(manager.redirected);
        assert!(manager.claiming);
        assert_eq!(request(&mut peer)[0], 22);
    }
    #[test]
    fn composite_conflict_fails_before_selection_acquisition() {
        let (mut manager, mut peer, now) = start();
        for sequence in [43, 44] {
            write_reply(
                &mut peer,
                &xproto::GetSelectionOwnerReply {
                    sequence,
                    owner: 0,
                    ..Default::default()
                }
                .serialize(),
            );
        }
        let mut error = [0u8; 32];
        error[1] = 10; // BadAccess: another client owns manual redirection.
        error[2..4].copy_from_slice(&45u16.to_ne_bytes());
        error[8..10].copy_from_slice(&2u16.to_ne_bytes());
        error[10] = 128;
        write_reply(&mut peer, &error);
        let failure = manager.dispatch(now).err().unwrap();
        assert!(failure.to_string().contains("essential XWM request failed"));
        assert!(!manager.claiming);
        assert!(!manager.is_complete());
        assert!(manager.dispatch(now).is_err());
    }
    #[test]
    fn metadata_error_prevents_completion() {
        let (mut manager, mut peer, now) = start();
        owners(&mut peer, 43, 0);
        timestamp(&manager, &mut peer, false);
        pump(&mut manager, now);
        for (index, opcode) in [22, 23, 22, 23].into_iter().enumerate() {
            assert_eq!(request(&mut peer)[0], opcode);
            if opcode == 23 {
                write_reply(
                    &mut peer,
                    &xproto::GetSelectionOwnerReply {
                        sequence: 47 + index as u16,
                        owner: manager.window,
                        ..Default::default()
                    }
                    .serialize(),
                );
            }
        }
        pump(&mut manager, now);
        check_metadata(&manager, &mut peer);
        assert!(!manager.is_complete());
        let mut error = [0u8; 32];
        error[1] = 3; // BadWindow on the first metadata write.
        error[2..4].copy_from_slice(&50u16.to_ne_bytes());
        error[10] = 18;
        write_reply(&mut peer, &error);
        assert!(manager.dispatch(now).is_err());
        assert!(manager.finish().is_err());
    }
    #[test]
    fn ownership_requires_timestamp_verification_and_checked_announcements() {
        let (mut manager, mut peer, now) = start();
        owners(&mut peer, 43, 0);
        pump(&mut manager, now);
        assert!(!manager.claiming);
        timestamp(&manager, &mut peer, true);
        pump(&mut manager, now);
        assert!(!manager.claiming);
        timestamp(&manager, &mut peer, false);
        pump(&mut manager, now);
        for (index, selection) in manager.selections.into_iter().enumerate() {
            let bytes = request(&mut peer);
            assert_eq!(bytes[0], 22);
            assert_eq!(
                u32::from_ne_bytes(bytes[4..8].try_into().unwrap()),
                manager.window
            );
            assert_eq!(
                u32::from_ne_bytes(bytes[8..12].try_into().unwrap()),
                selection
            );
            assert_eq!(u32::from_ne_bytes(bytes[12..16].try_into().unwrap()), 123);
            assert_eq!(request(&mut peer)[0], 23);
            write_reply(
                &mut peer,
                &xproto::GetSelectionOwnerReply {
                    sequence: 48 + index as u16 * 2,
                    owner: manager.window,
                    ..Default::default()
                }
                .serialize(),
            );
        }
        pump(&mut manager, now);
        assert!(!manager.is_complete());
        check_metadata(&manager, &mut peer);
        for selection in manager.selections {
            let bytes = request(&mut peer);
            assert_eq!(bytes[0], 25);
            let (event, _) = xproto::ClientMessageEvent::try_parse(&bytes[12..]).unwrap();
            assert_eq!(
                event.data.as_data32(),
                [123, selection, manager.window, 0, 0]
            );
            assert_eq!(event.window, 1);
            assert_eq!(event.type_, manager.discovered.atoms["MANAGER"]);
        }
        let font = request(&mut peer);
        assert_eq!(font[0], 45);
        assert_eq!(&font[12..18], b"cursor");
        let glyph = request(&mut peer);
        assert_eq!(glyph[0], 94);
        assert_eq!(u16::from_ne_bytes(glyph[16..18].try_into().unwrap()), 68);
        assert_eq!(u16::from_ne_bytes(glyph[18..20].try_into().unwrap()), 69);
        let attributes = request(&mut peer);
        assert_eq!(attributes[0], 2);
        assert_eq!(u32::from_ne_bytes(attributes[4..8].try_into().unwrap()), 1);
        assert_eq!(
            u32::from_ne_bytes(attributes[8..12].try_into().unwrap()),
            1 << 14
        );
        assert_eq!(&attributes[12..16], &glyph[4..8]);
        assert_eq!(request(&mut peer)[0], 95);
        assert_eq!(request(&mut peer)[0], 46);
        assert!(!manager.is_complete());
        assert_eq!(request(&mut peer)[0], 43);
        write_reply(
            &mut peer,
            &xproto::GetInputFocusReply {
                sequence: 61,
                ..Default::default()
            }
            .serialize(),
        );
        pump(&mut manager, now);
        assert!(manager.is_complete());
        assert_eq!(manager.requests.outstanding(), 0);
        write_reply(
            &mut peer,
            &xproto::SelectionClearEvent {
                response_type: xproto::SELECTION_CLEAR_EVENT,
                sequence: 61,
                time: 124,
                owner: manager.window,
                selection: manager.selections[0],
            }
            .serialize(),
        );
        assert!(
            manager
                .dispatch(now)
                .err()
                .unwrap()
                .to_string()
                .contains("ownership lost")
        );
        assert!(!manager.is_complete());
    }
    #[test]
    fn existing_owner_is_not_replaced_and_failure_is_terminal() {
        let (mut manager, mut peer, now) = start();
        owners(&mut peer, 43, 999);
        assert!(manager.dispatch(now).is_err());
        assert!(!manager.claiming);
        assert!(manager.dispatch(now).is_err());
        assert!(manager.finish().is_err());
    }
    #[test]
    fn missing_timestamp_hits_original_startup_deadline() {
        let (mut manager, mut peer, now) = start();
        owners(&mut peer, 43, 0);
        pump(&mut manager, now);
        assert!(manager.dispatch(manager.deadline()).is_err());
        assert!(!manager.is_complete());
    }
    #[test]
    fn lost_claim_cannot_reach_announcements() {
        let (mut manager, mut peer, now) = start();
        owners(&mut peer, 43, 0);
        timestamp(&manager, &mut peer, false);
        pump(&mut manager, now);
        for opcode in [22, 23, 22, 23] {
            assert_eq!(request(&mut peer)[0], opcode);
        }
        write_reply(
            &mut peer,
            &xproto::GetSelectionOwnerReply {
                sequence: 48,
                owner: 999,
                ..Default::default()
            }
            .serialize(),
        );
        assert!(manager.dispatch(now).is_err());
        assert!(!manager.is_complete());
    }
}

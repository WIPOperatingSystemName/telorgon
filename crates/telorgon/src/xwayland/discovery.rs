//! First XWM initialization phase: asynchronous atom/extension discovery and
//! checked root redirection. Completion is NOT the lifecycle Ready barrier:
//! manager ownership, root properties, output state and window handling follow.
use super::{
    Error, Result,
    requests::{Completion, Importance, ReplyKind, Requests},
    transport::Transport,
};
use std::{
    borrow::Cow,
    collections::{BTreeMap, VecDeque},
    os::{fd::RawFd, unix::net::UnixStream},
    time::{Duration, Instant},
};
use x11rb_protocol::{
    protocol::{composite, randr, shape, sync, xfixes, xproto},
    x11_utils::{Request, TryParse},
};

const ATOMS: &[&str] = &[
    "WM_S0",
    "_NET_WM_CM_S0",
    "MANAGER",
    "WL_SURFACE_SERIAL",
    "WM_PROTOCOLS",
    "WM_DELETE_WINDOW",
    "WM_TAKE_FOCUS",
    "WM_STATE",
    "_NET_SUPPORTED",
    "_NET_SUPPORTING_WM_CHECK",
    "_NET_WM_NAME",
    "UTF8_STRING",
    "CLIPBOARD",
    "TARGETS",
    "TIMESTAMP",
    "INCR",
    "TEXT",
    "COMPOUND_TEXT",
    "_NET_WM_SYNC_REQUEST",
    "_NET_WM_SYNC_REQUEST_COUNTER",
];
const EXTENSIONS: &[&str] = &["Composite", "XFIXES", "SHAPE", "RANDR", "SYNC"];
const DEADLINE: Duration = Duration::from_secs(10);
const TURN: Duration = Duration::from_millis(1);

// Required subsets: redirection, selections/regions, input shape, monitor objects.
fn required_version(name: &str) -> (u32, u32) {
    match name {
        "Composite" => (0, 4),
        "XFIXES" => (2, 0),
        "SHAPE" => (1, 1),
        "RANDR" => (1, 5),
        "SYNC" => (3, 1),
        _ => (0, 0),
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Extension {
    pub major_opcode: u8,
    pub first_event: u8,
    pub first_error: u8,
    pub version: (u32, u32),
}
pub struct Discovered {
    pub setup: xproto::Setup,
    pub atoms: BTreeMap<&'static str, u32>,
    /// Verified presence, opcode ranges and negotiated versions.
    pub extensions: BTreeMap<&'static str, Extension>,
}
enum Pending {
    Atom(&'static str),
    Extension(&'static str),
    Version(&'static str),
    Redirect,
    Barrier,
}
pub struct Turn {
    pub events: Vec<Vec<u8>>,
    pub reschedule: bool,
    pub writable_interest: bool,
    pub complete: bool,
}
pub struct Discovery {
    transport: Transport,
    requests: Requests,
    pending: BTreeMap<u64, Pending>,
    incoming: VecDeque<Vec<u8>>,
    discovered: Option<Discovered>,
    deadline: Instant,
    failed: bool,
}
impl Discovery {
    pub fn new(socket: UnixStream, generation: u64, now: Instant) -> Result<Self> {
        Ok(Self {
            transport: Transport::new(socket)?,
            requests: Requests::new(generation)?,
            pending: BTreeMap::new(),
            incoming: VecDeque::new(),
            discovered: None,
            deadline: now + DEADLINE,
            failed: false,
        })
    }
    pub fn wants_write(&self) -> bool {
        self.transport.wants_write()
    }
    pub fn fd(&self) -> RawFd {
        self.transport.fd()
    }
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
    pub fn result(&self) -> Option<&Discovered> {
        if self.failed || !self.pending.is_empty() {
            None
        } else {
            self.discovered.as_ref()
        }
    }
    /// Transfer the same connection and sequence tracker into the next XWM phase.
    /// No packets may be dropped at that boundary; finish draining this phase first.
    pub fn finish(self) -> Result<(Transport, Requests, Discovered)> {
        if self.result().is_none() || !self.incoming.is_empty() {
            return Err(Error("XWM discovery is not complete".into()));
        }
        Ok((self.transport, self.requests, self.discovered.unwrap()))
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
            return Err(Error("unexpected FD-bearing discovery request".into()));
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
    fn query_version(&mut self, name: &'static str, opcode: u8) -> Result<()> {
        let (major, minor) = required_version(name);
        match name {
            "Composite" => self.send_at(
                composite::QueryVersionRequest {
                    client_major_version: major,
                    client_minor_version: minor,
                },
                opcode,
                ReplyKind::Reply,
                Pending::Version(name),
            ),
            "XFIXES" => self.send_at(
                xfixes::QueryVersionRequest {
                    client_major_version: major,
                    client_minor_version: minor,
                },
                opcode,
                ReplyKind::Reply,
                Pending::Version(name),
            ),
            "SHAPE" => self.send_at(
                shape::QueryVersionRequest,
                opcode,
                ReplyKind::Reply,
                Pending::Version(name),
            ),
            "SYNC" => self.send_at(
                sync::InitializeRequest {
                    desired_major_version: 3,
                    desired_minor_version: 1,
                },
                opcode,
                ReplyKind::Reply,
                Pending::Version(name),
            ),
            "RANDR" => self.send_at(
                randr::QueryVersionRequest {
                    major_version: major,
                    minor_version: minor,
                },
                opcode,
                ReplyKind::Reply,
                Pending::Version(name),
            ),
            _ => Err(Error("unknown XWM extension version query".into())),
        }
    }
    fn begin(&mut self, setup: xproto::Setup) -> Result<()> {
        if self.discovered.is_some()
            || setup.roots.len() != 1
            || setup.roots[0].root == 0
            || setup.maximum_request_length < 16
        {
            return Err(Error(
                "unsupported XWM setup screen/request contract".into(),
            ));
        }
        let root = setup.roots[0].root;
        self.discovered = Some(Discovered {
            setup,
            atoms: BTreeMap::new(),
            extensions: BTreeMap::new(),
        });
        for &name in ATOMS {
            self.send(
                xproto::InternAtomRequest {
                    only_if_exists: false,
                    name: Cow::Borrowed(name.as_bytes()),
                },
                ReplyKind::Reply,
                Pending::Atom(name),
            )?;
        }
        for &name in EXTENSIONS {
            self.send(
                xproto::QueryExtensionRequest {
                    name: Cow::Borrowed(name.as_bytes()),
                },
                ReplyKind::Reply,
                Pending::Extension(name),
            )?;
        }
        let attributes = xproto::ChangeWindowAttributesAux::new().event_mask(
            xproto::EventMask::SUBSTRUCTURE_REDIRECT
                | xproto::EventMask::SUBSTRUCTURE_NOTIFY
                | xproto::EventMask::PROPERTY_CHANGE,
        );
        self.send(
            xproto::ChangeWindowAttributesRequest {
                window: root,
                value_list: Cow::Owned(attributes),
            },
            ReplyKind::Void,
            Pending::Redirect,
        )?;
        self.send(
            xproto::GetInputFocusRequest {},
            ReplyKind::Reply,
            Pending::Barrier,
        )
    }
    fn completion(&mut self, completion: Completion, events: &mut Vec<Vec<u8>>) -> Result<()> {
        match completion {
            Completion::Reply(id, bytes) => {
                let pending = self
                    .pending
                    .remove(&id.sequence)
                    .ok_or_else(|| Error("untracked discovery reply".into()))?;
                let discovered = self
                    .discovered
                    .as_mut()
                    .ok_or_else(|| Error("discovery reply before setup".into()))?;
                match pending {
                    Pending::Atom(name) => {
                        let (reply, _) = xproto::InternAtomReply::try_parse(&bytes)
                            .map_err(|_| Error("invalid discovery atom reply".into()))?;
                        if reply.atom == 0
                            || discovered.atoms.values().any(|atom| *atom == reply.atom)
                        {
                            return Err(Error("invalid or duplicate discovery atom".into()));
                        }
                        discovered.atoms.insert(name, reply.atom);
                    }
                    Pending::Extension(name) => {
                        let (reply, _) = xproto::QueryExtensionReply::try_parse(&bytes)
                            .map_err(|_| Error("invalid discovery extension reply".into()))?;
                        if name == "SYNC" && !reply.present {
                            return Ok(());
                        }
                        if !reply.present
                            || reply.major_opcode < 128
                            || discovered
                                .extensions
                                .values()
                                .any(|ext| ext.major_opcode == reply.major_opcode)
                        {
                            return Err(Error(format!(
                                "required XWM extension unavailable or invalid: {name}"
                            )));
                        }
                        discovered.extensions.insert(
                            name,
                            Extension {
                                major_opcode: reply.major_opcode,
                                first_event: reply.first_event,
                                first_error: reply.first_error,
                                version: (0, 0),
                            },
                        );
                        self.query_version(name, reply.major_opcode)?;
                    }
                    Pending::Version(name) => {
                        let parse_error = |_| Error("invalid XWM extension version reply".into());
                        let version = match name {
                            "Composite" => {
                                let (r, _) = composite::QueryVersionReply::try_parse(&bytes)
                                    .map_err(parse_error)?;
                                (r.major_version, r.minor_version)
                            }
                            "XFIXES" => {
                                let (r, _) = xfixes::QueryVersionReply::try_parse(&bytes)
                                    .map_err(parse_error)?;
                                (r.major_version, r.minor_version)
                            }
                            "SHAPE" => {
                                let (r, _) = shape::QueryVersionReply::try_parse(&bytes)
                                    .map_err(parse_error)?;
                                (u32::from(r.major_version), u32::from(r.minor_version))
                            }
                            "SYNC" => {
                                let (r, _) = sync::InitializeReply::try_parse(&bytes)
                                    .map_err(parse_error)?;
                                (u32::from(r.major_version), u32::from(r.minor_version))
                            }
                            "RANDR" => {
                                let (r, _) = randr::QueryVersionReply::try_parse(&bytes)
                                    .map_err(parse_error)?;
                                (r.major_version, r.minor_version)
                            }
                            _ => return Err(Error("unknown XWM extension version reply".into())),
                        };
                        let required = required_version(name);
                        if version.0 != required.0 || version.1 < required.1 {
                            return Err(Error(format!(
                                "unsupported XWM extension version: {name}"
                            )));
                        }
                        discovered
                            .extensions
                            .get_mut(name)
                            .ok_or_else(|| Error("version before extension discovery".into()))?
                            .version = version;
                    }
                    Pending::Barrier => {
                        xproto::GetInputFocusReply::try_parse(&bytes)
                            .map_err(|_| Error("invalid discovery barrier reply".into()))?;
                    }
                    Pending::Redirect => {
                        return Err(Error("unexpected root redirection reply".into()));
                    }
                }
            }
            Completion::Checked(id) => {
                if !matches!(self.pending.remove(&id.sequence), Some(Pending::Redirect)) {
                    return Err(Error("unexpected discovery checked request".into()));
                }
            }
            Completion::Event(bytes) => events.push(bytes),
            _ => return Err(Error("essential XWM discovery request failed".into())),
        }
        Ok(())
    }
    pub fn dispatch(&mut self, now: Instant) -> Result<Turn> {
        if self.failed {
            return Err(Error("XWM discovery has failed".into()));
        }
        let result = self.dispatch_inner(now);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn dispatch_inner(&mut self, now: Instant) -> Result<Turn> {
        if now >= self.deadline && self.result().is_none() {
            return Err(Error("XWM discovery timed out".into()));
        }
        self.requests.expire(now)?;
        let started = Instant::now();
        let mut reschedule = false;
        if self.incoming.is_empty() {
            let turn = self.transport.dispatch()?;
            reschedule = turn.reschedule;
            if let Some(setup) = turn.setup {
                self.begin(setup)?;
                reschedule = true; // Newly queued discovery requests need a first write.
            }
            self.incoming.extend(turn.packets);
        }
        let mut events = vec![];
        for _ in 0..256 {
            if started.elapsed() >= TURN {
                break;
            }
            let Some(packet) = self.incoming.pop_front() else {
                break;
            };
            for completion in self.requests.ingest(packet)? {
                self.completion(completion, &mut events)?;
            }
        }
        let writable_interest = self.transport.wants_write();
        Ok(Turn {
            events,
            reschedule: reschedule || !self.incoming.is_empty(),
            writable_interest,
            complete: self.result().is_some() && self.incoming.is_empty(),
        })
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use std::io::{Read, Write};
    use x11rb_protocol::x11_utils::Serialize;
    fn start() -> (Discovery, UnixStream, Instant) {
        let now = Instant::now();
        let (socket, mut peer) = UnixStream::pair().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        let mut discovery = Discovery::new(socket, 1, now).unwrap();
        flush_requests(&mut discovery.transport);
        peer.read_exact(&mut [0u8; 12]).unwrap();
        let setup = xproto::Setup {
            status: 1,
            protocol_major_version: 11,
            maximum_request_length: 65535,
            resource_id_base: 0x20000000,
            resource_id_mask: 0x1fffff,
            roots: vec![xproto::Screen {
                root: 1,
                width_in_pixels: 800,
                height_in_pixels: 600,
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut bytes = setup.serialize();
        let length = ((bytes.len() - 8) / 4) as u16;
        bytes[6..8].copy_from_slice(&length.to_ne_bytes());
        peer.write_all(&bytes[..5]).unwrap();
        assert!(!discovery.dispatch(now).unwrap().complete);
        peer.write_all(&bytes[5..]).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while discovery.discovered.is_none() || discovery.transport.wants_write() {
            assert!(
                Instant::now() < deadline,
                "discovery fixture did not finish initial requests"
            );
            discovery.dispatch(now).unwrap();
        }
        (discovery, peer, now)
    }
    /// Fixture-only draining before reading requests from the peer on this same thread.
    /// A production dispatch intentionally may leave writes queued after its turn budget.
    pub(in crate::xwayland) fn flush_requests(transport: &mut Transport) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while transport.wants_write() {
            assert!(
                Instant::now() < deadline,
                "fixture request writes did not drain"
            );
            let turn = transport.dispatch().unwrap();
            assert!(
                turn.setup.is_none() && turn.packets.is_empty(),
                "fixture flush would discard input"
            );
        }
    }
    pub(in crate::xwayland) fn request(peer: &mut UnixStream) -> Vec<u8> {
        let mut header = [0; 4];
        peer.read_exact(&mut header).unwrap();
        let len = u16::from_ne_bytes([header[2], header[3]]) as usize * 4;
        let mut bytes = vec![0; len];
        bytes[..4].copy_from_slice(&header);
        peer.read_exact(&mut bytes[4..]).unwrap();
        bytes
    }
    pub(in crate::xwayland) fn write_reply(peer: &mut UnixStream, fields: &[u8]) {
        // Generated reply serialization omits unused trailing wire padding.
        let mut bytes = fields.to_vec();
        bytes.resize(32, 0);
        peer.write_all(&bytes).unwrap();
    }
    fn discovery_replies(peer: &mut UnixStream, missing: bool) -> u16 {
        let mut sequence = 0;
        for (index, name) in ATOMS.iter().enumerate() {
            sequence += 1;
            let bytes = request(peer);
            assert_eq!(bytes[0], 16);
            assert_eq!(&bytes[8..8 + name.len()], name.as_bytes());
            let reply = xproto::InternAtomReply {
                sequence,
                atom: 100 + index as u32,
                ..Default::default()
            };
            write_reply(peer, &reply.serialize());
        }
        for (index, name) in EXTENSIONS.iter().enumerate() {
            sequence += 1;
            let bytes = request(peer);
            assert_eq!(bytes[0], 98);
            assert_eq!(&bytes[8..8 + name.len()], name.as_bytes());
            let reply = xproto::QueryExtensionReply {
                sequence,
                present: !(missing && index == 0),
                major_opcode: 128 + index as u8,
                first_event: if index == 1 { 80 } else { 0 },
                ..Default::default()
            };
            write_reply(peer, &reply.serialize());
        }
        let redirect = request(peer);
        assert_eq!(redirect[0], 2);
        assert_eq!(u32::from_ne_bytes(redirect[4..8].try_into().unwrap()), 1);
        assert_eq!(request(peer)[0], 43);
        sequence + 1 // Root redirection sequence; the barrier is one later.
    }
    fn version_replies(
        discovery: &mut Discovery,
        peer: &mut UnixStream,
        now: Instant,
        old_randr: bool,
    ) {
        for _ in 0..100 {
            discovery.dispatch(now).unwrap();
            if discovery.discovered.as_ref().unwrap().extensions.len() == EXTENSIONS.len() {
                break;
            }
        }
        flush_requests(&mut discovery.transport); // All version requests have been queued.
        assert!(discovery.result().is_none());
        for (index, name) in EXTENSIONS.iter().enumerate() {
            let bytes = request(peer);
            assert_eq!(bytes[0], 128 + index as u8);
            assert_eq!(bytes[1], 0);
            let (major, minor) = required_version(name);
            if *name == "SYNC" {
                assert_eq!(&bytes[4..6], &[3, 1]);
            } else if *name != "SHAPE" {
                assert_eq!(u32::from_ne_bytes(bytes[4..8].try_into().unwrap()), major);
                assert_eq!(u32::from_ne_bytes(bytes[8..12].try_into().unwrap()), minor);
            } else {
                assert_eq!(bytes.len(), 4);
            }
            let sequence = (ATOMS.len() + EXTENSIONS.len() + 3 + index) as u16;
            match *name {
                "Composite" => write_reply(
                    peer,
                    &composite::QueryVersionReply {
                        sequence,
                        major_version: major,
                        minor_version: minor,
                        ..Default::default()
                    }
                    .serialize(),
                ),
                "XFIXES" => write_reply(
                    peer,
                    &xfixes::QueryVersionReply {
                        sequence,
                        major_version: major,
                        minor_version: minor,
                        ..Default::default()
                    }
                    .serialize(),
                ),
                "SHAPE" => write_reply(
                    peer,
                    &shape::QueryVersionReply {
                        sequence,
                        major_version: major as u16,
                        minor_version: minor as u16,
                        ..Default::default()
                    }
                    .serialize(),
                ),
                "SYNC" => write_reply(
                    peer,
                    &sync::InitializeReply {
                        sequence,
                        major_version: 3,
                        minor_version: 1,
                        ..Default::default()
                    }
                    .serialize(),
                ),
                "RANDR" => write_reply(
                    peer,
                    &randr::QueryVersionReply {
                        sequence,
                        major_version: major,
                        minor_version: if old_randr { 4 } else { minor },
                        ..Default::default()
                    }
                    .serialize(),
                ),
                _ => unreachable!(),
            }
        }
    }
    pub(in crate::xwayland) fn ready() -> (Transport, Requests, Discovered, UnixStream, Instant) {
        let (mut discovery, mut peer, now) = start();
        let redirect = discovery_replies(&mut peer, false);
        write_reply(
            &mut peer,
            &xproto::GetInputFocusReply {
                sequence: redirect + 1,
                ..Default::default()
            }
            .serialize(),
        );
        version_replies(&mut discovery, &mut peer, now, false);
        for _ in 0..100 {
            if discovery.dispatch(now).unwrap().complete {
                break;
            }
        }
        let (transport, requests, discovered) = discovery.finish().unwrap();
        (transport, requests, discovered, peer, now)
    }
    #[test]
    fn fixture_drains_requests_across_the_transport_operation_budget() {
        let (mut transport, _, _, mut peer, _) = ready();
        let (bytes, _) = Request::serialize(xproto::GetInputFocusRequest, 0);
        // A single transport turn permits at most 256 write operations.
        for _ in 0..260 {
            transport.queue(bytes.clone()).unwrap();
        }
        transport.dispatch().unwrap();
        assert!(transport.wants_write());
        flush_requests(&mut transport);
        assert!(!transport.wants_write());
        for _ in 0..260 {
            assert_eq!(request(&mut peer), bytes);
        }
    }
    #[test]
    fn discovery_completes_only_after_checked_redirection_and_versions() {
        let (mut discovery, mut peer, now) = start();
        let redirect = discovery_replies(&mut peer, false);
        assert!(!discovery.dispatch(now).unwrap().complete);
        assert!(discovery.result().is_none());
        let reply = xproto::GetInputFocusReply {
            sequence: redirect + 1,
            ..Default::default()
        };
        write_reply(&mut peer, &reply.serialize());
        version_replies(&mut discovery, &mut peer, now, false);
        for _ in 0..100 {
            if discovery.dispatch(now).unwrap().complete {
                break;
            }
        }
        let (_, requests, result) = discovery.finish().unwrap();
        assert_eq!(requests.outstanding(), 0);
        assert_eq!(result.atoms.len(), ATOMS.len());
        assert_eq!(result.extensions.len(), EXTENSIONS.len());
        for &name in EXTENSIONS {
            assert_eq!(result.extensions[name].version, required_version(name));
        }
        assert_eq!(result.setup.roots[0].root, 1);
    }
    #[test]
    fn missing_extension_or_root_conflict_is_terminal() {
        for missing in [true, false] {
            let (mut discovery, mut peer, now) = start();
            let redirect = discovery_replies(&mut peer, missing);
            if !missing {
                let mut error = [0u8; 32];
                error[1] = 10; // BadAccess from another root redirect owner.
                error[2..4].copy_from_slice(&redirect.to_ne_bytes());
                peer.write_all(&error).unwrap();
            }
            let mut failed = false;
            for _ in 0..100 {
                if let Err(error) = discovery.dispatch(now) {
                    assert!(error.to_string().contains(if missing {
                        "required XWM extension"
                    } else {
                        "essential XWM request failed"
                    }));
                    failed = true;
                    break;
                }
            }
            assert!(failed);
            assert!(discovery.result().is_none());
            assert!(discovery.dispatch(now).is_err());
        }
    }
    #[test]
    fn setup_timeout_does_not_report_completion() {
        let now = Instant::now();
        let (socket, _peer) = UnixStream::pair().unwrap();
        let mut discovery = Discovery::new(socket, 1, now).unwrap();
        assert!(discovery.dispatch(now + DEADLINE).is_err());
        assert!(discovery.finish().is_err());
    }
    #[test]
    fn insufficient_version_does_not_satisfy_discovery_barrier() {
        let (mut discovery, mut peer, now) = start();
        let redirect = discovery_replies(&mut peer, false);
        write_reply(
            &mut peer,
            &xproto::GetInputFocusReply {
                sequence: redirect + 1,
                ..Default::default()
            }
            .serialize(),
        );
        version_replies(&mut discovery, &mut peer, now, true);
        let mut failed = false;
        for _ in 0..100 {
            if let Err(error) = discovery.dispatch(now) {
                assert!(
                    error
                        .to_string()
                        .contains("unsupported XWM extension version: RANDR")
                );
                failed = true;
                break;
            }
        }
        assert!(failed);
        assert!(discovery.result().is_none());
    }
}

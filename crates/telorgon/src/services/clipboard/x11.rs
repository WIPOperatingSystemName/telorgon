//! Clipboard bridge on a separate connection to the host's private Xwayland.
//! X replies and clipboard content transfers never block the compositor owner.
use super::*;
use std::sync::Arc;
use std::{
    collections::HashMap,
    os::unix::net::UnixStream,
    sync::mpsc,
    time::{Duration, Instant},
};
use x11rb::{
    connection::Connection,
    protocol::{
        Event,
        xfixes::{self, ConnectionExt as _},
        xproto::*,
    },
    rust_connection::{DefaultStream, RustConnection},
    wrapper::ConnectionExt as _,
};
const CHUNK: usize = 64 * 1024;
const TIMEOUT: Duration = Duration::from_secs(5);
type BridgeResult<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
pub(crate) struct Bridge {
    stop: Arc<AtomicBool>,
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}
impl Bridge {
    pub fn start(
        display: std::ffi::OsString,
        authority: std::ffi::OsString,
        clipboard: Clipboard,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        std::thread::spawn(move || {
            if let Err(error) = run(display, authority, clipboard, worker_stop) {
                eprintln!("telorgon-clipboard: Xwayland bridge stopped: {error}");
            }
        });
        Self { stop }
    }
}
// Read only the explicitly supplied private-server authority, never ambient DISPLAY.
fn cookie(path: &std::path::Path, display: &str) -> BridgeResult<(Vec<u8>, Vec<u8>)> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(65537)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 65536 {
        return Err("authority file too large".into());
    }
    let mut offset = 0;
    fn field(bytes: &[u8], offset: &mut usize) -> Option<Vec<u8>> {
        let len = u16::from_be_bytes(bytes.get(*offset..*offset + 2)?.try_into().ok()?) as usize;
        *offset += 2;
        let field = bytes.get(*offset..*offset + len)?.to_vec();
        *offset += len;
        Some(field)
    }
    while offset + 2 <= bytes.len() {
        offset += 2;
        let _address = field(&bytes, &mut offset).ok_or("invalid authority")?;
        let number = field(&bytes, &mut offset).ok_or("invalid authority")?;
        let name = field(&bytes, &mut offset).ok_or("invalid authority")?;
        let data = field(&bytes, &mut offset).ok_or("invalid authority")?;
        if number == display.as_bytes() && name == b"MIT-MAGIC-COOKIE-1" {
            return Ok((name, data));
        }
    }
    Err("private Xwayland cookie absent".into())
}
struct ReadJob {
    selection: Atom,
    owner: Window,
    target: Atom,
    reply: mpsc::SyncSender<Result<Vec<u8>>>,
}
struct XProvider {
    tx: mpsc::SyncSender<ReadJob>,
    selection: Atom,
    owner: Window,
    formats: Vec<(DataFormat, Atom)>,
}
impl ClipboardProvider for XProvider {
    fn write(
        &self,
        format: &DataFormat,
        output: &mut dyn std::io::Write,
        cancel: &AtomicBool,
    ) -> Result<()> {
        let target = self
            .formats
            .iter()
            .find(|(f, _)| f == format)
            .ok_or(ClipboardError::InvalidFormat)?
            .1;
        let (tx, rx) = mpsc::sync_channel(1);
        self.tx
            .try_send(ReadJob {
                selection: self.selection,
                owner: self.owner,
                target,
                reply: tx,
            })
            .map_err(|_| ClipboardError::Busy)?;
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if cancel.load(Ordering::Acquire) {
                return Err(ClipboardError::Cancelled);
            }
            if Instant::now() >= deadline {
                return Err(ClipboardError::Timeout);
            }
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(result) => {
                    output
                        .write_all(&result?)
                        .map_err(|_| ClipboardError::TransferFailed)?;
                    return Ok(());
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return Err(ClipboardError::Unavailable),
            }
        }
    }
}
enum Destination {
    Formats(usize, Window, u64),
    Read(mpsc::SyncSender<Result<Vec<u8>>>),
}
struct Incoming {
    selection: Atom,
    target: Atom,
    data: Vec<u8>,
    incr: bool,
    completed: Option<Result<()>>,
    destination: Destination,
    deadline: Instant,
}
struct Outgoing {
    target: Atom,
    data: Vec<u8>,
    offset: usize,
    deadline: Instant,
}
struct Export {
    event: SelectionRequestEvent,
    snapshot: ClipboardSnapshot,
    future: ClipboardRequest<Vec<u8>>,
}
fn notify(
    conn: &RustConnection,
    event: &SelectionRequestEvent,
    property: Atom,
) -> BridgeResult<()> {
    conn.send_event(
        false,
        event.requestor,
        EventMask::NO_EVENT,
        SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: event.time,
            requestor: event.requestor,
            selection: event.selection,
            target: event.target,
            property,
        },
    )?;
    Ok(())
}
fn run(
    display: std::ffi::OsString,
    authority: std::ffi::OsString,
    clipboard: Clipboard,
    stop: Arc<AtomicBool>,
) -> BridgeResult<()> {
    let display = display
        .to_str()
        .ok_or("non-text display")?
        .strip_prefix(':')
        .ok_or("non-local display")?
        .split('.')
        .next()
        .unwrap();
    if !display.bytes().all(|b| b.is_ascii_digit()) {
        return Err("invalid display".into());
    }
    let auth = cookie(std::path::Path::new(&authority), display)?;
    let (stream, _) = DefaultStream::from_unix_stream(UnixStream::connect(format!(
        "/tmp/.X11-unix/X{display}"
    ))?)?;
    let conn = RustConnection::connect_to_stream_with_auth_info(stream, 0, auth.0, auth.1)?;
    let root = conn.setup().roots[0].root;
    let window = conn.generate_id()?;
    conn.create_window(
        0,
        window,
        root,
        0,
        0,
        1,
        1,
        0,
        WindowClass::INPUT_ONLY,
        0,
        &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
    )?
    .check()?;
    let atom =
        |name: &[u8]| -> BridgeResult<Atom> { Ok(conn.intern_atom(false, name)?.reply()?.atom) };
    let selections = [atom(b"CLIPBOARD")?, AtomEnum::PRIMARY.into()];
    let targets = atom(b"TARGETS")?;
    let utf8 = atom(b"UTF8_STRING")?;
    let incr = atom(b"INCR")?;
    let mut properties = Vec::new();
    for i in 0..16 {
        properties.push(atom(format!("_TELORGON_CLIPBOARD_{i}").as_bytes())?);
    }
    conn.xfixes_query_version(5, 0)?.reply()?;
    for selection in selections {
        conn.xfixes_select_selection_input(
            window,
            selection,
            xfixes::SelectionEventMask::SET_SELECTION_OWNER
                | xfixes::SelectionEventMask::SELECTION_WINDOW_DESTROY
                | xfixes::SelectionEventMask::SELECTION_CLIENT_CLOSE,
        )?
        .check()?;
    }
    let (tx, rx) = mpsc::sync_channel::<ReadJob>(16);
    let mut incoming = HashMap::<Atom, Incoming>::new();
    let mut outgoing = HashMap::<(Window, Atom), Outgoing>::new();
    let mut exports = Vec::<Export>::new();
    let mut mirrored = [0u64; 2];
    let mut owners = [0u32; 2];
    let mut publications: [Option<ClipboardRequest<()>>; 2] = [None, None];
    let mut publication_revisions = [0; 2];
    let kinds = [ClipboardKind::System, ClipboardKind::Selection];
    let mut initial = true;
    while !stop.load(Ordering::Acquire) {
        if clipboard.snapshot(ClipboardKind::System).is_err() {
            outgoing.clear();
            exports.clear();
        }
        let mut changes = Vec::new();
        if initial {
            for slot in 0..2 {
                changes.push((
                    slot,
                    conn.get_selection_owner(selections[slot])?.reply()?.owner,
                ));
            }
            initial = false;
        }
        for _ in 0..256 {
            let Some(event) = conn.poll_for_event()? else {
                break;
            };
            match event {
                Event::XfixesSelectionNotify(event) => {
                    if let Some(slot) = selections
                        .iter()
                        .position(|selection| *selection == event.selection)
                    {
                        changes.push((slot, event.owner));
                    }
                }
                Event::SelectionRequest(event)
                    if selections.contains(&event.selection) && event.owner == window =>
                {
                    let slot = selections
                        .iter()
                        .position(|id| *id == event.selection)
                        .unwrap();
                    let property = if event.property == 0 {
                        event.target
                    } else {
                        event.property
                    };
                    let Ok(snapshot) = clipboard.snapshot(kinds[slot]) else {
                        notify(&conn, &event, 0)?;
                        continue;
                    };
                    let mut formats = Vec::new();
                    for format in &snapshot.formats {
                        formats.push((atom(format.identifier().as_bytes())?, format.clone()));
                        if format.identifier() == "text/plain;charset=utf-8" {
                            formats.push((utf8, format.clone()));
                        }
                    }
                    if event.target == targets {
                        let mut atoms = vec![targets];
                        atoms.extend(formats.iter().map(|(atom, _)| *atom));
                        conn.change_property32(
                            PropMode::REPLACE,
                            event.requestor,
                            property,
                            AtomEnum::ATOM,
                            &atoms,
                        )?;
                        notify(&conn, &event, property)?;
                    } else if let Some((_, format)) =
                        formats.iter().find(|(atom, _)| *atom == event.target)
                    {
                        if exports.len() + outgoing.len() >= 16 {
                            notify(&conn, &event, 0)?;
                            continue;
                        }
                        exports.push(Export {
                            future: clipboard.read(snapshot.clone(), format.clone(), MAX_BYTES),
                            snapshot,
                            event,
                        });
                    } else {
                        notify(&conn, &event, 0)?;
                    }
                }
                Event::SelectionNotify(event) if event.requestor == window => {
                    if event.property == 0 {
                        let failed: Vec<_> = incoming
                            .iter()
                            .filter_map(|(id, pending)| {
                                (pending.selection == event.selection
                                    && pending.target == event.target)
                                    .then_some(*id)
                            })
                            .collect();
                        for id in failed {
                            if let Some(pending) = incoming.remove(&id) {
                                if let Destination::Read(reply) = pending.destination {
                                    let _ = reply.try_send(Err(ClipboardError::TransferFailed));
                                }
                            }
                        }
                        continue;
                    }
                    if let Some(pending) = incoming.get_mut(&event.property) {
                        let reply = conn
                            .get_property(
                                true,
                                window,
                                event.property,
                                AtomEnum::ANY,
                                0,
                                (MAX_BYTES / 4 + 1) as u32,
                            )?
                            .reply()?;
                        if reply.type_ == incr && reply.format == 32 && reply.value.len() == 4 {
                            pending.incr = true;
                        } else if !valid_property(&reply, pending.target == targets) {
                            pending.completed = Some(Err(ClipboardError::TransferFailed));
                        } else if reply.bytes_after == 0 && reply.value.len() <= MAX_BYTES {
                            pending.data = reply.value;
                            pending.completed = Some(Ok(()));
                        } else {
                            pending.data.clear();
                            pending.completed = Some(Err(ClipboardError::TooLarge));
                        }
                    }
                }
                Event::PropertyNotify(event) => {
                    if event.window == window && event.state == Property::NEW_VALUE {
                        if let Some(pending) = incoming.get_mut(&event.atom).filter(|p| p.incr) {
                            let reply = conn
                                .get_property(
                                    true,
                                    window,
                                    event.atom,
                                    AtomEnum::ANY,
                                    0,
                                    (MAX_BYTES / 4 + 1) as u32,
                                )?
                                .reply()?;
                            if !valid_property(&reply, pending.target == targets) {
                                pending.completed = Some(Err(ClipboardError::TransferFailed));
                            } else if reply.bytes_after != 0
                                || pending.data.len() + reply.value.len() > MAX_BYTES
                            {
                                pending.data.clear();
                                pending.completed = Some(Err(ClipboardError::TooLarge));
                            } else if reply.value.is_empty() {
                                pending.completed = Some(Ok(()));
                            } else {
                                pending.data.extend(reply.value);
                            }
                        }
                    } else if event.state == Property::DELETE {
                        if let Some(pending) = outgoing.get_mut(&(event.window, event.atom)) {
                            let end = (pending.offset + CHUNK).min(pending.data.len());
                            conn.change_property8(
                                PropMode::REPLACE,
                                event.window,
                                event.atom,
                                pending.target,
                                &pending.data[pending.offset..end],
                            )?;
                            let finished = pending.offset == pending.data.len();
                            pending.offset = end;
                            if finished {
                                outgoing.remove(&(event.window, event.atom));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        for (slot, owner) in changes {
            owners[slot] = owner;
            if owner == window {
                continue;
            }
            publications[slot] = None;
            let Ok(snapshot) = clipboard.snapshot(kinds[slot]) else {
                continue;
            };
            if owner == 0 {
                // Do not erase a simultaneous Wayland publication.
                if snapshot.revision == mirrored[slot] {
                    publications[slot] =
                        Some(clipboard.clear(kinds[slot], Some(snapshot.revision)));
                    publication_revisions[slot] = snapshot.revision + 1;
                }
                continue;
            }
            if let Some(property) = properties
                .iter()
                .find(|property| !incoming.contains_key(property))
                .copied()
            {
                conn.convert_selection(
                    window,
                    selections[slot],
                    targets,
                    property,
                    x11rb::CURRENT_TIME,
                )?;
                incoming.insert(
                    property,
                    Incoming {
                        selection: selections[slot],
                        target: targets,
                        data: vec![],
                        incr: false,
                        completed: None,
                        destination: Destination::Formats(slot, owner, snapshot.revision),
                        deadline: Instant::now() + TIMEOUT,
                    },
                );
            }
        }
        while let Ok(job) = rx.try_recv() {
            let slot = selections
                .iter()
                .position(|id| *id == job.selection)
                .unwrap();
            if owners[slot] != job.owner {
                let _ = job.reply.try_send(Err(ClipboardError::Stale));
                continue;
            }
            if let Some(property) = properties
                .iter()
                .find(|property| !incoming.contains_key(property))
                .copied()
            {
                conn.convert_selection(
                    window,
                    job.selection,
                    job.target,
                    property,
                    x11rb::CURRENT_TIME,
                )?;
                incoming.insert(
                    property,
                    Incoming {
                        selection: job.selection,
                        target: job.target,
                        data: vec![],
                        incr: false,
                        completed: None,
                        destination: Destination::Read(job.reply),
                        deadline: Instant::now() + TIMEOUT,
                    },
                );
            } else {
                let _ = job.reply.try_send(Err(ClipboardError::Busy));
            }
        }
        let finished: Vec<_> = incoming
            .iter()
            .filter_map(|(id, pending)| {
                (pending.completed.is_some() || Instant::now() >= pending.deadline).then_some(*id)
            })
            .collect();
        for id in finished {
            let pending = incoming.remove(&id).unwrap();
            conn.delete_property(window, id)?;
            match pending.destination {
                Destination::Read(reply) => {
                    let _ = reply.try_send(
                        pending
                            .completed
                            .unwrap_or(Err(ClipboardError::Timeout))
                            .map(|()| pending.data),
                    );
                }
                Destination::Formats(slot, owner, revision)
                    if owner == owners[slot] && pending.completed == Some(Ok(())) =>
                {
                    let mut formats = Vec::new();
                    for bytes in pending.data.chunks_exact(4).take(256) {
                        let target = u32::from_ne_bytes(bytes.try_into().unwrap());
                        let name = if target == utf8 {
                            "text/plain;charset=utf-8".to_owned()
                        } else {
                            let Ok(reply) = conn.get_atom_name(target)?.reply() else {
                                continue;
                            };
                            String::from_utf8_lossy(&reply.name).into_owned()
                        };
                        if let Ok(format) = DataFormat::mime(&name) {
                            if !formats.iter().any(|(f, _)| *f == format) && formats.len() < 64 {
                                formats.push((format, target));
                            }
                        }
                    }
                    if !formats.is_empty() {
                        let content = ClipboardContent::provider(
                            formats.iter().map(|(f, _)| f.clone()).collect(),
                            Arc::new(XProvider {
                                tx: tx.clone(),
                                selection: selections[slot],
                                owner,
                                formats,
                            }),
                        )?;
                        publications[slot] =
                            Some(clipboard.publish(kinds[slot], content, Some(revision)));
                        publication_revisions[slot] = revision + 1;
                    }
                }
                _ => {}
            }
        }
        exports.retain_mut(|export| {
            let Some(result) = export.future.try_take() else {
                return true;
            };
            let event = &export.event;
            let property = if event.property == 0 {
                event.target
            } else {
                event.property
            };
            let valid = clipboard
                .snapshot(export.snapshot.kind)
                .is_ok_and(|s| s.revision == export.snapshot.revision);
            let result: BridgeResult<()> = (|| {
                let Ok(bytes) = result else {
                    return notify(&conn, event, 0);
                };
                if !valid {
                    return notify(&conn, event, 0);
                }
                if bytes.len() <= CHUNK {
                    conn.change_property8(
                        PropMode::REPLACE,
                        event.requestor,
                        property,
                        event.target,
                        &bytes,
                    )?;
                } else {
                    conn.change_window_attributes(
                        event.requestor,
                        &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
                    )?;
                    conn.change_property32(
                        PropMode::REPLACE,
                        event.requestor,
                        property,
                        incr,
                        &[bytes.len() as u32],
                    )?;
                    outgoing.insert(
                        (event.requestor, property),
                        Outgoing {
                            target: event.target,
                            data: bytes,
                            offset: 0,
                            deadline: Instant::now() + TIMEOUT,
                        },
                    );
                }
                notify(&conn, event, property)
            })();
            if result.is_err() {
                let _ = notify(&conn, event, 0);
            }
            false
        });
        outgoing.retain(|_, pending| Instant::now() < pending.deadline);
        for slot in 0..2 {
            if let Some(request) = &mut publications[slot] {
                if let Some(result) = request.try_take() {
                    publications[slot] = None;
                    if result.is_ok() {
                        mirrored[slot] = publication_revisions[slot];
                    }
                }
            }
            if publications[slot].is_some()
                || incoming
                    .values()
                    .any(|p| matches!(p.destination, Destination::Formats(s, _, _) if s == slot))
            {
                continue;
            }
            let Ok(snapshot) = clipboard.snapshot(kinds[slot]) else {
                if owners[slot] == window {
                    conn.set_selection_owner(0u32, selections[slot], x11rb::CURRENT_TIME)?;
                    owners[slot] = 0;
                    mirrored[slot] = 0;
                }
                continue;
            };
            if mirrored[slot] == snapshot.revision {
                continue;
            }
            mirrored[slot] = snapshot.revision;
            if snapshot.formats.is_empty() {
                if owners[slot] == window {
                    conn.set_selection_owner(0u32, selections[slot], x11rb::CURRENT_TIME)?;
                    owners[slot] = 0;
                }
                continue;
            }
            conn.set_selection_owner(window, selections[slot], x11rb::CURRENT_TIME)?;
            if conn.get_selection_owner(selections[slot])?.reply()?.owner == window {
                owners[slot] = window;
            }
        }
        conn.flush()?;
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

fn valid_property(reply: &GetPropertyReply, targets: bool) -> bool {
    if targets {
        reply.type_ == u32::from(AtomEnum::ATOM) && reply.format == 32
    } else {
        reply.type_ != 0 && reply.format == 8
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clipboard_x11_rejects_wrong_property_representation() {
        let mut reply = GetPropertyReply {
            format: 32,
            type_: AtomEnum::ATOM.into(),
            ..Default::default()
        };
        assert!(valid_property(&reply, true));
        assert!(!valid_property(&reply, false));
        reply.format = 8;
        assert!(!valid_property(&reply, true));
        assert!(valid_property(&reply, false));
        reply.type_ = 0;
        assert!(!valid_property(&reply, false));
    }
}

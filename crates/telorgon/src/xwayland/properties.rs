//! Bounded asynchronous WM_PROTOCOLS, WM_HINTS and WM_NORMAL_HINTS metadata. Invalidated reads never restore
//! older capabilities; property storms coalesce while one read is outstanding.
use super::{
    Error, Result,
    association::XWindow,
    requests::{Completion, Importance, ReplyKind, Requests},
    transport::Transport,
    window::{Action, Windows},
};
use std::{collections::BTreeMap, time::Instant};
use x11rb_protocol::{
    protocol::xproto,
    x11_utils::{Request, TryParse},
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Protocols {
    pub delete_window: bool,
    pub take_focus: bool,
    pub sync_request: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputHints {
    pub accepts_input: bool,
    pub urgent: bool,
}
impl Default for InputHints {
    fn default() -> Self {
        Self {
            accepts_input: true,
            urgent: false,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusModel {
    NoInput,
    Passive,
    LocallyActive,
    GloballyActive,
}
impl FocusModel {
    pub fn from_hints(hints: InputHints, protocols: Protocols) -> Self {
        match (hints.accepts_input, protocols.take_focus) {
            (false, false) => Self::NoInput,
            (true, false) => Self::Passive,
            (true, true) => Self::LocallyActive,
            (false, true) => Self::GloballyActive,
        }
    }
}
#[derive(Clone)]
enum Value {
    Protocols(Protocols),
    Hints(InputHints),
    Normal(super::normal_hints::NormalHints),
    Text(String),
    Counter(u32),
}
#[derive(Clone, Copy)]
enum PropertyKind {
    Protocols,
    Counter,
    Input,
    Normal,
    Text { utf8: bool },
}
struct Entry {
    revision: u64,
    dirty: bool,
    inflight: bool,
    value: Option<Value>,
}
pub(crate) struct PropertyReader {
    property: u32,
    kind: PropertyKind,
    delete: u32,
    take_focus: u32,
    sync_request: u32,
    entries: BTreeMap<XWindow, Entry>,
    pending: BTreeMap<u64, (XWindow, u64)>,
}
impl PropertyReader {
    pub fn new(property: u32, delete: u32, take_focus: u32) -> Self {
        Self {
            property,
            kind: PropertyKind::Protocols,
            delete,
            take_focus,
            sync_request: 0,
            entries: BTreeMap::new(),
            pending: BTreeMap::new(),
        }
    }
    pub fn with_sync_request(mut self, atom: u32) -> Self {
        self.sync_request = atom;
        self
    }
    pub fn new_counter(property: u32) -> Self {
        let mut reader = Self::new(property, 0, 0);
        reader.kind = PropertyKind::Counter;
        reader
    }
    pub fn counter(&self, window: XWindow) -> Option<u32> {
        match self.entries.get(&window)?.value.as_ref()? {
            Value::Counter(counter) => Some(*counter),
            _ => None,
        }
    }
    fn parse_counter(bytes: &[u8]) -> Option<u32> {
        if bytes.len() != 36 && bytes.len() != 40 {
            return None;
        }
        let (reply, _) = xproto::GetPropertyReply::try_parse(bytes).ok()?;
        // An optional second counter is the extended frame protocol. Use only
        // the basic counter and never claim the extended protocol here.
        if reply.type_ != u32::from(xproto::AtomEnum::CARDINAL)
            || reply.format != 32
            || reply.bytes_after != 0
            || !(1..=2).contains(&reply.value_len)
        {
            return None;
        }
        let counter = u32::from_ne_bytes(reply.value.get(..4)?.try_into().ok()?);
        (counter != 0).then_some(counter)
    }
    pub fn new_hints() -> Self {
        let mut reader = Self::new(xproto::AtomEnum::WM_HINTS.into(), 0, 0);
        reader.kind = PropertyKind::Input;
        reader
    }
    pub fn new_normal_hints() -> Self {
        let mut reader = Self::new(xproto::AtomEnum::WM_NORMAL_HINTS.into(), 0, 0);
        reader.kind = PropertyKind::Normal;
        reader
    }
    pub fn new_text(property: u32, type_: u32, utf8: bool) -> Self {
        let mut reader = Self::new(property, type_, 0);
        reader.kind = PropertyKind::Text { utf8 };
        reader
    }
    pub fn text(&self, window: XWindow) -> Option<&str> {
        match self.entries.get(&window)?.value.as_ref()? {
            Value::Text(text) => Some(text),
            _ => None,
        }
    }
    fn parse_text(&self, bytes: &[u8], utf8: bool) -> Option<String> {
        if bytes.len() > 32 + 4096 {
            return None;
        }
        let (reply, _) = xproto::GetPropertyReply::try_parse(bytes).ok()?;
        if reply.type_ != self.delete
            || reply.format != 8
            || reply.bytes_after != 0
            || reply.value.len() > 4096
        {
            return None;
        }
        let value = if utf8 {
            std::str::from_utf8(&reply.value).ok()?.to_owned()
        } else {
            reply.value.iter().map(|byte| char::from(*byte)).collect()
        };
        Some(value.chars().filter(|c| !c.is_control()).collect())
    }
    pub fn normal_hints(&self, window: XWindow) -> Option<super::normal_hints::NormalHints> {
        match self.entries.get(&window)?.value.as_ref()? {
            Value::Normal(hints) => Some(*hints),
            _ => None,
        }
    }
    pub fn input_hints(&self, window: XWindow) -> Option<InputHints> {
        match self.entries.get(&window)?.value.as_ref()? {
            Value::Hints(hints) => Some(*hints),
            _ => None,
        }
    }
    pub fn tracked(&self, window: XWindow) -> bool {
        self.entries.contains_key(&window)
    }
    pub fn property(&self) -> u32 {
        self.property
    }
    pub fn get(&self, window: XWindow) -> Option<Protocols> {
        match self.entries.get(&window)?.value.as_ref()? {
            Value::Protocols(protocols) => Some(*protocols),
            _ => None,
        }
    }
    pub fn forget(&mut self, window: XWindow) {
        self.entries.remove(&window);
    }
    pub fn refresh(&mut self, window: XWindow) -> Result<()> {
        if !self.entries.contains_key(&window) && self.entries.len() >= 4096 {
            return Err(Error("XWM protocol metadata bound reached".into()));
        }
        let entry = self.entries.entry(window).or_insert(Entry {
            revision: 0,
            dirty: false,
            inflight: false,
            value: None,
        });
        entry.revision = entry
            .revision
            .checked_add(1)
            .ok_or_else(|| Error("XWM property revision exhausted".into()))?;
        entry.dirty = true;
        entry.value = None;
        Ok(())
    }
    pub fn schedule(
        &mut self,
        transport: &mut Transport,
        requests: &mut Requests,
        deadline: Instant,
        work_deadline: Instant,
    ) -> Result<bool> {
        let mut count = 0;
        for (&window, entry) in &mut self.entries {
            if self.pending.len() >= 128
                || requests.available_slots() == 0
                || count == 16
                || Instant::now() >= work_deadline
            {
                break;
            }
            if !entry.dirty || entry.inflight {
                continue;
            }
            let (bytes, _) = Request::serialize(
                xproto::GetPropertyRequest {
                    delete: false,
                    window: window.xid,
                    property: self.property,
                    type_: match self.kind {
                        PropertyKind::Counter => xproto::AtomEnum::CARDINAL.into(),
                        PropertyKind::Input => self.property,
                        PropertyKind::Normal => xproto::AtomEnum::WM_SIZE_HINTS.into(),
                        PropertyKind::Protocols => xproto::AtomEnum::ATOM.into(),
                        PropertyKind::Text { .. } => self.delete,
                    },
                    long_offset: 0,
                    long_length: match self.kind {
                        PropertyKind::Counter => 2,
                        PropertyKind::Input => 9,
                        PropertyKind::Normal => 18,
                        PropertyKind::Protocols => 256,
                        PropertyKind::Text { .. } => 1024,
                    },
                },
                0,
            );
            let id = requests.queue(
                transport,
                bytes,
                ReplyKind::Reply,
                Importance::Optional,
                deadline,
            )?;
            self.pending.insert(id.sequence, (window, entry.revision));
            entry.inflight = true;
            entry.dirty = false;
            count += 1;
        }
        Ok(self.pending.len() < 128
            && requests.available_slots() > 0
            && self.entries.values().any(|e| e.dirty && !e.inflight))
    }
    fn parse(&self, bytes: &[u8]) -> Option<Protocols> {
        if bytes.len() > 32 + 1024 {
            return None;
        }
        let (reply, _) = xproto::GetPropertyReply::try_parse(bytes).ok()?;
        if reply.type_ == 0 && reply.format == 0 && reply.value_len == 0 && reply.bytes_after == 0 {
            return Some(Protocols::default());
        }
        if reply.type_ != u32::from(xproto::AtomEnum::ATOM)
            || reply.format != 32
            || reply.bytes_after != 0
            || reply.value_len > 256
        {
            return None;
        }
        let mut result = Protocols::default();
        for bytes in reply.value.chunks_exact(4) {
            let atom = u32::from_ne_bytes(bytes.try_into().ok()?);
            result.delete_window |= atom == self.delete;
            result.take_focus |= atom == self.take_focus;
            result.sync_request |= self.sync_request != 0 && atom == self.sync_request;
        }
        Some(result)
    }
    fn parse_hints(&self, bytes: &[u8]) -> Option<InputHints> {
        if bytes.len() > 68 {
            return None;
        }
        let (reply, _) = xproto::GetPropertyReply::try_parse(bytes).ok()?;
        if reply.type_ == 0 && reply.format == 0 && reply.value_len == 0 && reply.bytes_after == 0 {
            return Some(InputHints::default());
        }
        if reply.type_ != u32::from(xproto::AtomEnum::WM_HINTS)
            || reply.format != 32
            || reply.bytes_after != 0
            || reply.value_len != 9
        {
            return None;
        }
        let flags = u32::from_ne_bytes(reply.value[0..4].try_into().ok()?);
        let input = u32::from_ne_bytes(reply.value[4..8].try_into().ok()?);
        if flags & 1 != 0 && input > 1 {
            return None;
        }
        Some(InputHints {
            accepts_input: flags & 1 == 0 || input != 0,
            urgent: flags & 256 != 0,
        })
    }
    pub fn completion(
        &mut self,
        completion: &Completion,
        windows: &Windows,
        actions: &mut Vec<Action>,
    ) -> Result<bool> {
        let id = match completion {
            Completion::Reply(id, _) | Completion::Error(id, _) | Completion::TimedOut(id) => id,
            _ => return Ok(false),
        };
        let Some(&(window, revision)) = self.pending.get(&id.sequence) else {
            return Ok(false);
        };
        if id.generation != window.generation {
            return Ok(false);
        }
        self.pending.remove(&id.sequence);
        let value = match completion {
            Completion::Reply(_, bytes) => match self.kind {
                PropertyKind::Counter => Self::parse_counter(bytes).map(Value::Counter),
                PropertyKind::Input => self.parse_hints(bytes).map(Value::Hints),
                PropertyKind::Normal => super::normal_hints::parse(bytes).map(Value::Normal),
                PropertyKind::Protocols => self.parse(bytes).map(Value::Protocols),
                PropertyKind::Text { utf8 } => self.parse_text(bytes, utf8).map(Value::Text),
            },
            _ => None,
        };
        if let Some(entry) = self.entries.get_mut(&window) {
            entry.inflight = false;
            if entry.revision == revision && windows.get(window.xid).is_some_and(|w| w.id == window)
            {
                entry.value = value;
                actions.push(match self.kind {
                    PropertyKind::Counter => Action::Changed(window),
                    PropertyKind::Input => Action::HintsChanged(window),
                    PropertyKind::Normal => Action::NormalHintsChanged(window),
                    PropertyKind::Protocols => Action::ProtocolsChanged(window),
                    PropertyKind::Text { .. } => Action::Changed(window),
                });
            }
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{discovery::tests::ready, requests::RequestId};
    use super::*;
    use std::time::Duration;
    use x11rb_protocol::x11_utils::Serialize;
    fn window() -> (Windows, XWindow) {
        let mut w = Windows::new(1, 16, 1, 2, 100).unwrap();
        let event: [u8; 32] = xproto::CreateNotifyEvent {
            response_type: xproto::CREATE_NOTIFY_EVENT,
            parent: 1,
            window: 10,
            width: 1,
            height: 1,
            ..Default::default()
        }
        .into();
        w.event(&event).unwrap();
        let id = w.get(10).unwrap().id;
        (w, id)
    }
    fn reply(sequence: u64, atoms: &[u32]) -> Completion {
        let value = atoms.iter().flat_map(|atom| atom.to_ne_bytes()).collect();
        Completion::Reply(
            RequestId {
                generation: 1,
                sequence,
            },
            xproto::GetPropertyReply {
                sequence: sequence as u16,
                format: 32,
                length: atoms.len() as u32,
                type_: 4,
                bytes_after: 0,
                value_len: atoms.len() as u32,
                value,
            }
            .serialize(),
        )
    }
    #[test]
    fn basic_sync_counter_requires_bounded_cardinal_and_advertised_protocol() {
        let p = PropertyReader::new(104, 105, 106).with_sync_request(120);
        let Completion::Reply(_, bytes) = reply(1, &[120]) else {
            unreachable!()
        };
        assert!(p.parse(&bytes).unwrap().sync_request);
        let make = |values: &[u32]| {
            xproto::GetPropertyReply {
                format: 32,
                length: values.len() as u32,
                type_: xproto::AtomEnum::CARDINAL.into(),
                value_len: values.len() as u32,
                value: values.iter().flat_map(|v| v.to_ne_bytes()).collect(),
                ..Default::default()
            }
            .serialize()
        };
        assert_eq!(PropertyReader::parse_counter(&make(&[42])), Some(42));
        assert_eq!(PropertyReader::parse_counter(&make(&[42, 43])), Some(42));
        assert_eq!(PropertyReader::parse_counter(&make(&[0])), None);
        assert_eq!(PropertyReader::parse_counter(&make(&[42, 43, 44])), None);
        let mut malformed = make(&[42]);
        malformed[8..12].copy_from_slice(&u32::from(xproto::AtomEnum::ATOM).to_ne_bytes());
        assert_eq!(PropertyReader::parse_counter(&malformed), None);
        let mut partial = make(&[42]);
        partial[12..16].copy_from_slice(&4u32.to_ne_bytes());
        assert_eq!(PropertyReader::parse_counter(&partial), None);
    }
    #[test]
    fn frame_titles_are_bounded_and_encoding_checked() {
        let reader = PropertyReader::new_text(200, 201, true);
        let make_reply = |value: Vec<u8>, bytes_after: u32, type_: u32| {
            let mut bytes = xproto::GetPropertyReply {
                format: 8,
                sequence: 1,
                length: value.len().div_ceil(4) as u32,
                type_,
                bytes_after,
                value_len: value.len() as u32,
                value,
            }
            .serialize();
            bytes.resize(bytes.len().div_ceil(4) * 4, 0);
            bytes
        };
        assert_eq!(
            reader.parse_text(
                &make_reply("Title — 日本語".as_bytes().to_vec(), 0, 201),
                true
            ),
            Some("Title — 日本語".into())
        );
        assert!(
            reader
                .parse_text(&make_reply(vec![0xff], 0, 201), true)
                .is_none()
        );
        assert!(
            reader
                .parse_text(&make_reply(vec![b'a'; 4097], 0, 201), true)
                .is_none()
        );
        assert!(
            reader
                .parse_text(&make_reply(b"title".to_vec(), 1, 201), true)
                .is_none()
        );
        assert!(
            reader
                .parse_text(&make_reply(b"title".to_vec(), 0, 202), true)
                .is_none()
        );
        let legacy = PropertyReader::new_text(
            xproto::AtomEnum::WM_NAME.into(),
            xproto::AtomEnum::STRING.into(),
            false,
        );
        assert_eq!(
            legacy.parse_text(
                &make_reply(vec![0xe9], 0, xproto::AtomEnum::STRING.into()),
                false
            ),
            Some("é".into())
        );
    }

    #[test]
    fn normal_hints_reads_are_bounded_and_stale_replies_cannot_restore_constraints() {
        use std::io::Read;
        let (mut transport, mut requests, _, mut peer, now) = ready();
        let (windows, id) = window();
        let mut reader = PropertyReader::new_normal_hints();
        reader.refresh(id).unwrap();
        let deadline = now + Duration::from_secs(10);
        reader
            .schedule(
                &mut transport,
                &mut requests,
                deadline,
                Instant::now() + Duration::from_secs(1),
            )
            .unwrap();
        transport.dispatch().unwrap();
        let mut wire = [0; 24];
        peer.read_exact(&mut wire).unwrap();
        assert_eq!(wire[0], 20);
        assert_eq!(
            u32::from_ne_bytes(wire[8..12].try_into().unwrap()),
            u32::from(xproto::AtomEnum::WM_NORMAL_HINTS)
        );
        assert_eq!(
            u32::from_ne_bytes(wire[12..16].try_into().unwrap()),
            u32::from(xproto::AtomEnum::WM_SIZE_HINTS)
        );
        assert_eq!(u32::from_ne_bytes(wire[20..24].try_into().unwrap()), 18);
        let response = |sequence| {
            let mut completion = reply(
                sequence,
                &[16, 0, 0, 0, 0, 80, 60, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            );
            if let Completion::Reply(_, bytes) = &mut completion {
                bytes[8..12]
                    .copy_from_slice(&u32::from(xproto::AtomEnum::WM_SIZE_HINTS).to_ne_bytes());
            }
            completion
        };
        reader.refresh(id).unwrap();
        let mut actions = Vec::new();
        reader
            .completion(&response(33), &windows, &mut actions)
            .unwrap();
        assert!(actions.is_empty());
        assert_eq!(reader.normal_hints(id), None);
        reader
            .schedule(
                &mut transport,
                &mut requests,
                deadline,
                Instant::now() + Duration::from_secs(1),
            )
            .unwrap();
        reader
            .completion(&response(34), &windows, &mut actions)
            .unwrap();
        assert!(
            matches!(actions.as_slice(), [Action::NormalHintsChanged(window)] if *window == id)
        );
        assert_eq!(
            reader.normal_hints(id).unwrap().minimum_size(),
            crate::core::SizeI {
                width: 80,
                height: 60
            }
        );
        reader.refresh(id).unwrap();
        assert_eq!(reader.normal_hints(id), None);
        reader
            .schedule(
                &mut transport,
                &mut requests,
                deadline,
                Instant::now() + Duration::from_secs(1),
            )
            .unwrap();
        reader.forget(id);
        actions.clear();
        reader
            .completion(&response(35), &windows, &mut actions)
            .unwrap();
        assert!(actions.is_empty());
        assert_eq!(reader.normal_hints(id), None);
    }
    #[test]
    fn property_reads_wait_for_shared_request_capacity() {
        let (mut transport, mut requests, _, _peer, now) = ready();
        let (_, id) = window();
        let mut reader = PropertyReader::new(104, 105, 106);
        reader.refresh(id).unwrap();
        let (bytes, _) = Request::serialize(xproto::GetInputFocusRequest, 0);
        while requests.available_slots() != 0 {
            requests
                .queue(
                    &mut transport,
                    bytes.clone(),
                    ReplyKind::Reply,
                    Importance::Optional,
                    now + Duration::from_secs(1),
                )
                .unwrap();
        }
        assert!(
            !reader
                .schedule(
                    &mut transport,
                    &mut requests,
                    now,
                    Instant::now() + Duration::from_secs(1)
                )
                .unwrap()
        );
        assert!(reader.pending.is_empty());
        let mut response = xproto::GetInputFocusReply {
            sequence: 33,
            ..Default::default()
        }
        .serialize()
        .to_vec();
        response.resize(32, 0);
        requests.ingest(response).unwrap();
        assert_eq!(requests.available_slots(), 1);
        reader
            .schedule(
                &mut transport,
                &mut requests,
                now,
                Instant::now() + Duration::from_secs(1),
            )
            .unwrap();
        assert_eq!(reader.pending.len(), 1);
        assert_eq!(requests.available_slots(), 0);
    }
    #[test]
    fn input_hints_validate_layout_flags_and_default() {
        let reader = PropertyReader::new_hints();
        let make = |flags: u32, input: u32| {
            xproto::GetPropertyReply {
                format: 32,
                sequence: 1,
                length: 9,
                type_: 35,
                bytes_after: 0,
                value_len: 9,
                value: [flags, input, 0, 0, 0, 0, 0, 0, 0]
                    .into_iter()
                    .flat_map(u32::to_ne_bytes)
                    .collect(),
            }
            .serialize()
        };
        assert_eq!(
            reader.parse_hints(&make(257, 0)),
            Some(InputHints {
                accepts_input: false,
                urgent: true
            })
        );
        assert_eq!(reader.parse_hints(&make(1, 1)), Some(InputHints::default()));
        assert_eq!(
            reader.parse_hints(&make(0, 999)),
            Some(InputHints::default())
        );
        assert_eq!(reader.parse_hints(&make(1, 2)), None);
        assert_eq!(reader.parse_hints(&make(1, 1)[..64]), None);
        let absent = xproto::GetPropertyReply {
            format: 0,
            sequence: 1,
            length: 0,
            type_: 0,
            bytes_after: 0,
            value_len: 0,
            value: vec![],
        }
        .serialize();
        assert_eq!(reader.parse_hints(&absent), Some(InputHints::default()));
    }
    #[test]
    fn icccm_focus_models_follow_input_and_take_focus_independently() {
        for (input, take, expected) in [
            (false, false, FocusModel::NoInput),
            (true, false, FocusModel::Passive),
            (true, true, FocusModel::LocallyActive),
            (false, true, FocusModel::GloballyActive),
        ] {
            assert_eq!(
                FocusModel::from_hints(
                    InputHints {
                        accepts_input: input,
                        urgent: false
                    },
                    Protocols {
                        delete_window: false,
                        take_focus: take,
                        sync_request: false
                    }
                ),
                expected
            );
        }
    }
    #[test]
    fn property_storm_coalesces_and_old_reply_cannot_restore_capabilities() {
        let (mut t, mut r, _, _peer, now) = ready();
        let (w, id) = window();
        let mut p = PropertyReader::new(104, 105, 106);
        p.refresh(id).unwrap();
        p.schedule(
            &mut t,
            &mut r,
            now + Duration::from_secs(1),
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap();
        for _ in 0..100 {
            p.refresh(id).unwrap();
        }
        p.schedule(
            &mut t,
            &mut r,
            now + Duration::from_secs(1),
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(p.pending.len(), 1);
        let mut actions = vec![];
        p.completion(&reply(33, &[105, 106]), &w, &mut actions)
            .unwrap();
        assert_eq!(p.get(id), None);
        assert!(actions.is_empty());
        p.schedule(
            &mut t,
            &mut r,
            now + Duration::from_secs(1),
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap();
        p.completion(&reply(34, &[]), &w, &mut actions).unwrap();
        assert_eq!(p.get(id), Some(Protocols::default()));
        assert_eq!(actions, vec![Action::ProtocolsChanged(id)]);
    }
    #[test]
    fn invalid_or_incomplete_properties_grant_no_capabilities() {
        let p = PropertyReader::new(104, 105, 106);
        let Completion::Reply(_, mut bytes) = reply(33, &[105]) else {
            unreachable!()
        };
        assert_eq!(
            p.parse(&bytes),
            Some(Protocols {
                delete_window: true,
                take_focus: false,
                sync_request: false
            })
        );
        bytes[12..16].copy_from_slice(&4u32.to_ne_bytes());
        assert_eq!(p.parse(&bytes), None);
        assert_eq!(p.parse(&vec![0; 1057]), None);
        bytes[1] = 8;
        assert_eq!(p.parse(&bytes), None);
    }
    #[test]
    fn discarded_window_reply_cannot_publish_metadata() {
        let (mut t, mut r, _, _peer, now) = ready();
        let (w, id) = window();
        let mut p = PropertyReader::new(104, 105, 106);
        p.refresh(id).unwrap();
        p.schedule(&mut t, &mut r, now, Instant::now() + Duration::from_secs(1))
            .unwrap();
        p.forget(id);
        let mut actions = vec![];
        assert!(p.completion(&reply(33, &[105]), &w, &mut actions).unwrap());
        assert!(actions.is_empty());
        assert_eq!(p.get(id), None);
    }
}

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
    Icon(crate::graphics::render::ImageResource),
    Counter(u32),
    Decorations(bool),
}
#[derive(Clone, Copy)]
enum PropertyKind {
    Icon,
    Protocols,
    Counter,
    Input,
    Decorations,
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
    pub fn new_decorations(property: u32) -> Self {
        let mut reader = Self::new(property, 0, 0);
        reader.kind = PropertyKind::Decorations;
        reader
    }
    pub fn decorations(&self, window: XWindow) -> Option<bool> {
        match self.entries.get(&window)?.value.as_ref()? {
            Value::Decorations(decorated) => Some(*decorated),
            _ => None,
        }
    }
    fn parse_decorations(&self, bytes: &[u8]) -> Option<bool> {
        if bytes.len() != 52 {
            return None;
        }
        let (reply, _) = xproto::GetPropertyReply::try_parse(bytes).ok()?;
        if reply.type_ != self.property
            || reply.format != 32
            || reply.value_len != 5
            || reply.bytes_after != 0
        {
            return None;
        }
        let flags = u32::from_ne_bytes(reply.value.get(..4)?.try_into().ok()?);
        if flags & 2 == 0 {
            return Some(true);
        }
        let decorations = u32::from_ne_bytes(reply.value.get(8..12)?.try_into().ok()?);
        if decorations & !0x7f != 0 {
            return None;
        }
        // Motif ALL inverts the remaining decoration bits. Telorgon's shell has one
        // complete frame, so any requested standard decoration selects that frame.
        let mask = if decorations & 1 != 0 {
            !decorations & 0x7e
        } else {
            decorations & 0x7e
        };
        Some(mask != 0)
    }
    pub fn new_icon(property: u32) -> Self {
        let mut reader = Self::new(property, 0, 0);
        reader.kind = PropertyKind::Icon;
        reader
    }
    pub fn icon(&self, window: XWindow) -> Option<&crate::graphics::render::ImageResource> {
        match self.entries.get(&window)?.value.as_ref()? {
            Value::Icon(icon) => Some(icon),
            _ => None,
        }
    }
    fn parse_icon(bytes: &[u8]) -> Option<crate::graphics::render::ImageResource> {
        if bytes.len() > 32 + 1024 * 1024 {
            return None;
        }
        let (reply, _) = xproto::GetPropertyReply::try_parse(bytes).ok()?;
        if reply.format != 32
            || reply.type_ != u32::from(xproto::AtomEnum::CARDINAL)
            || reply.bytes_after != 0
        {
            return None;
        }
        let words: Vec<u32> = reply
            .value
            .chunks_exact(4)
            .map(|v| u32::from_ne_bytes(v.try_into().unwrap()))
            .collect();
        let mut offset = 0usize;
        let mut best: Option<(u32, u32, usize)> = None;
        while offset < words.len() {
            let width = *words.get(offset)?;
            let height = *words.get(offset + 1)?;
            offset += 2;
            if width == 0 || height == 0 || width > 4096 || height > 4096 {
                return None;
            }
            let count = (width as usize).checked_mul(height as usize)?;
            let end = offset.checked_add(count)?;
            if end > words.len() {
                return None;
            }
            if best.is_none_or(|(w, h, _)| width.min(height) > w.min(h)) {
                best = Some((width, height, offset));
            }
            offset = end;
        }
        let (width, height, start) = best?;
        let pixels: Vec<u8> = words[start..start + (width * height) as usize]
            .iter()
            .flat_map(|argb| {
                [
                    (argb >> 16) as u8,
                    (argb >> 8) as u8,
                    *argb as u8,
                    (argb >> 24) as u8,
                ]
            })
            .collect();
        Some(crate::graphics::render::ImageResource {
            image: crate::ui::ImageId(
                crate::authoring::compose::applications::NEXT_ICON
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            ),
            content_version: 1,
            extent: crate::SizeI {
                width: width as i32,
                height: height as i32,
            },
            pixels: pixels.into(),
            color_encoding: crate::graphics::render::ImageColorEncoding::Srgb,
            alpha_mode: crate::graphics::render::ImageAlphaMode::Straight,
            pixel_format: crate::graphics::render::ImagePixelFormat::Rgba8,
        })
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
        if self.property == u32::from(xproto::AtomEnum::WM_CLASS) {
            let mut parts = reply.value.split(|b| *b == 0);
            let instance = parts.next()?;
            let class = parts.next()?;
            let value = if class.is_empty() { instance } else { class };
            return Some(
                value
                    .iter()
                    .map(|b| char::from(*b))
                    .filter(|c| !c.is_control())
                    .collect(),
            );
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
        if !matches!(self.kind, PropertyKind::Decorations) {
            entry.value = None;
        }
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
                        PropertyKind::Counter | PropertyKind::Icon => {
                            xproto::AtomEnum::CARDINAL.into()
                        }
                        PropertyKind::Input | PropertyKind::Decorations => self.property,
                        PropertyKind::Normal => xproto::AtomEnum::WM_SIZE_HINTS.into(),
                        PropertyKind::Protocols => xproto::AtomEnum::ATOM.into(),
                        PropertyKind::Text { .. } => self.delete,
                    },
                    long_offset: 0,
                    long_length: match self.kind {
                        PropertyKind::Icon => 262144,
                        PropertyKind::Counter => 2,
                        PropertyKind::Input => 9,
                        PropertyKind::Decorations => 5,
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
                PropertyKind::Icon => Self::parse_icon(bytes).map(Value::Icon),
                PropertyKind::Counter => Self::parse_counter(bytes).map(Value::Counter),
                PropertyKind::Input => self.parse_hints(bytes).map(Value::Hints),
                PropertyKind::Decorations => Some(Value::Decorations(
                    self.parse_decorations(bytes).unwrap_or(true),
                )),
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
                    PropertyKind::Icon | PropertyKind::Counter | PropertyKind::Decorations => {
                        Action::Changed(window)
                    }
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
mod tests;

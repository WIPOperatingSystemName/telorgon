//! Immutable, build-generated protocol metadata used by the dispatcher.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtocolSchema {
    pub name: &'static str,
    pub interfaces: &'static [InterfaceSchema],
}

impl ProtocolSchema {
    pub fn interface(&self, name: &str) -> Option<&InterfaceSchema> {
        self.interfaces
            .iter()
            .find(|interface| interface.name == name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InterfaceSchema {
    pub name: &'static str,
    pub version: u32,
    pub requests: &'static [MessageSchema],
    pub events: &'static [MessageSchema],
}

impl InterfaceSchema {
    pub fn request(&self, opcode: u32) -> Option<&MessageSchema> {
        self.requests.get(opcode as usize)
    }

    pub fn request_named(&self, name: &str) -> Option<(u32, &MessageSchema)> {
        self.requests
            .iter()
            .enumerate()
            .find(|(_, message)| message.name == name)
            .map(|(opcode, message)| (opcode as u32, message))
    }

    pub fn event_named(&self, name: &str) -> Option<(u32, &MessageSchema)> {
        self.events
            .iter()
            .enumerate()
            .find(|(_, message)| message.name == name)
            .map(|(opcode, message)| (opcode as u32, message))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageKind {
    Request,
    Event,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageSchema {
    pub name: &'static str,
    pub since: u32,
    pub(crate) signature: &'static str,
    pub destructor: bool,
    pub kind: MessageKind,
    pub arguments: &'static [ArgumentSchema],
}

impl MessageSchema {
    /// Precomputed libwayland wire signature (including version and nullability).
    pub const fn native_signature(&self) -> &'static str {
        self.signature
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArgumentSchema {
    pub name: &'static str,
    pub argument_type: ArgumentType,
    pub interface: Option<&'static str>,
    pub allow_null: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArgumentType {
    Int,
    Uint,
    Fixed,
    String,
    Object,
    NewId,
    Array,
    Fd,
}

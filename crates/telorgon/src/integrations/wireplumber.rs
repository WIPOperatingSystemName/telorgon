//! WirePlumber policy through default metadata. Policy can reject requests; default requests
//! complete only when the effective default changes. Stream-move requests confirm metadata
//! acceptance; graph links must be inspected for routing completion by the session manager.
use super::pipewire::{self as transport, *};
#[derive(Clone, Copy, Debug)]
pub enum DefaultKind {
    Output,
    Input,
    Camera,
}
impl DefaultKind {
    fn suffix(self) -> &'static str {
        match self {
            Self::Output => "audio.sink",
            Self::Input => "audio.source",
            Self::Camera => "video.source",
        }
    }
}
#[derive(Clone)]
pub struct SessionPolicy {
    connection: ConnectionHandle,
}
impl SessionPolicy {
    pub(crate) fn new(connection: ConnectionHandle) -> Self {
        Self { connection }
    }
    pub fn default_device(&self, kind: DefaultKind) -> Option<ObjectHandle> {
        let s = self.connection.snapshot();
        let m = s.objects.values().find(|o| {
            o.kind == ObjectKind::Metadata
                && o.properties
                    .get("metadata.name")
                    .is_some_and(|s| s == "default")
        })?;
        let value = m.metadata.get(&(0, format!("default.{}", kind.suffix())))?;
        let json: serde_json::Value = serde_json::from_str(value).ok()?;
        let name = json.get("name")?.as_str()?;
        s.objects
            .values()
            .find(|o| o.properties.get("node.name").is_some_and(|n| n == name))
            .map(|o| o.handle)
    }
    pub fn set_default(
        &self,
        kind: DefaultKind,
        target: ObjectHandle,
    ) -> Result<Request, MediaError> {
        let s = self.connection.snapshot();
        let object = s.resolve(target)?;
        let class = match kind {
            DefaultKind::Output => "Audio/Sink",
            DefaultKind::Input => "Audio/Source",
            DefaultKind::Camera => "Video/Source",
        };
        if object.media_class() != Some(class) {
            return Err(MediaError::InvalidArgument("default device media class"));
        }
        let name = object
            .properties
            .get("node.name")
            .ok_or(MediaError::Unsupported("named node"))?;
        self.metadata(
            None,
            format!("default.configured.{}", kind.suffix()),
            serde_json::json!({"name":name}).to_string(),
            format!("default.{}", kind.suffix()),
        )
    }
    pub fn move_stream(
        &self,
        stream: ObjectHandle,
        target: ObjectHandle,
    ) -> Result<Request, MediaError> {
        let s = self.connection.snapshot();
        let source = s.resolve(stream)?;
        let target = s.resolve(target)?;
        let expected = match source.media_class() {
            Some("Stream/Output/Audio") => "Audio/Sink",
            Some("Stream/Input/Audio") => "Audio/Source",
            _ => return Err(MediaError::InvalidArgument("audio stream")),
        };
        if target.media_class() != Some(expected) {
            return Err(MediaError::InvalidArgument("routing direction"));
        }
        let serial = target
            .serial()
            .ok_or(MediaError::Unsupported("target serial"))?;
        self.metadata(
            Some(stream),
            "target.object".into(),
            serial.to_string(),
            "target.object".into(),
        )
    }
    fn metadata(
        &self,
        subject: Option<ObjectHandle>,
        key: String,
        value: String,
        observed_key: String,
    ) -> Result<Request, MediaError> {
        let snapshot = self.connection.snapshot();
        let target = snapshot
            .objects
            .values()
            .find(|o| {
                o.kind == ObjectKind::Metadata
                    && o.properties
                        .get("metadata.name")
                        .is_some_and(|s| s == "default")
            })
            .ok_or(MediaError::Unsupported("WirePlumber default metadata"))?
            .handle;
        self.connection
            .mutate(transport::control::Mutation::Metadata {
                target,
                subject,
                key,
                value,
                observed_key,
            })
    }
}

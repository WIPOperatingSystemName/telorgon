use super::{BootError, BootResult};
use crate::graphics::render::ImageResource;
use alloc::{string::String, vec::Vec};
use core::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TargetId(String);

impl TargetId {
    pub fn new(id: &str) -> BootResult<Self> {
        if id.is_empty()
            || id.len() > 64
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        {
            return Err(BootError::Configuration(
                "target IDs must use 1..=64 letters, numbers, dots, hyphens, or underscores".into(),
            ));
        }
        Ok(Self(id.into()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootTargetKind {
    Linux,
    Windows,
    Custom,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootTarget {
    pub id: TargetId,
    pub name: String,
    pub kind: BootTargetKind,
    pub detail: String,
    pub source: Option<BootSource>,
    pub icon: Option<ImageResource>,
    pub splash: Option<ImageResource>,
    pub icon_path: Option<String>,
    pub splash_path: Option<String>,
    pub embedded_splash: bool,
}

impl BootTarget {
    pub fn new(id: &str, name: &str, kind: BootTargetKind) -> BootResult<Self> {
        if name.trim().is_empty() || name.len() > 80 || name.chars().any(char::is_control) {
            return Err(BootError::Configuration(
                "target names must use 1..=80 bytes without control characters".into(),
            ));
        }
        Ok(Self {
            id: TargetId::new(id)?,
            name: name.into(),
            kind,
            detail: String::new(),
            source: None,
            icon: None,
            splash: None,
            icon_path: None,
            splash_path: None,
            embedded_splash: true,
        })
    }
    pub fn linux(id: &str, name: &str) -> BootResult<Self> {
        Self::new(id, name, BootTargetKind::Linux)
    }
    pub fn windows(id: &str, name: &str) -> BootResult<Self> {
        Self::new(id, name, BootTargetKind::Windows)
    }
    pub fn custom(id: &str, name: &str) -> BootResult<Self> {
        Self::new(id, name, BootTargetKind::Custom)
    }
    pub fn unknown(id: &str, name: &str) -> BootResult<Self> {
        Self::new(id, name, BootTargetKind::Unknown)
    }
    pub fn icon(mut self, image: ImageResource) -> Self {
        self.icon = Some(image);
        self
    }
    pub fn splash(mut self, image: ImageResource) -> Self {
        self.splash = Some(image);
        self
    }
    pub fn icon_file(mut self, path: &str) -> BootResult<Self> {
        self.icon_path = Some(BootSource::efi(path)?.path().into());
        Ok(self)
    }
    pub fn splash_file(mut self, path: &str) -> BootResult<Self> {
        self.splash_path = Some(BootSource::efi(path)?.path().into());
        Ok(self)
    }
    pub fn detail(mut self, detail: &str) -> Self {
        self.detail = detail.into();
        self
    }
    pub fn source(mut self, source: BootSource) -> Self {
        self.source = Some(source);
        self
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BootTheme {
    #[default]
    Disks,
    Voxel,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BootSelectionMode {
    /// Launch a single target directly; show the selector when there is a choice.
    #[default]
    Auto,
    /// Show the selector even when only one target is configured.
    Always,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootPhase {
    Selecting,
    Loading,
    OsStarting,
    Complete,
    Failed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PreviewScenario {
    #[default]
    Selecting,
    /// Follow the application's initial selection policy.
    Startup,
    Loading,
    OsStarting,
    Complete,
    Failed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BootSnapshot {
    pub theme: BootTheme,
    pub phase: BootPhase,
    pub selected: usize,
    pub targets: Vec<BootTarget>,
    /// Normalized measured fraction; inspect `progress_kind` before displaying a percentage.
    pub progress: f32,
    pub status: String,
    pub frame: u64,
    pub elapsed: Duration,
    pub paused: bool,
    pub simulation: bool,
    pub progress_kind: BootProgress,
}

pub type PreviewTheme = BootTheme;
pub type PreviewPhase = BootPhase;
pub type PreviewSnapshot = BootSnapshot;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BootVolume {
    #[default]
    Current,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BootSource {
    EfiImage {
        volume: BootVolume,
        path: String,
        options: Vec<u16>,
    },
}

impl BootSource {
    pub fn path(&self) -> &str {
        let Self::EfiImage { path, .. } = self;
        path
    }
    pub fn efi(path: &str) -> BootResult<Self> {
        let source = Self::EfiImage {
            volume: BootVolume::Current,
            path: path.replace('/', "\\"),
            options: Vec::new(),
        };
        source.validate()?;
        Ok(source)
    }
    pub fn arguments(mut self, arguments: &str) -> BootResult<Self> {
        let Self::EfiImage { options, .. } = &mut self;
        *options = arguments
            .encode_utf16()
            .chain(core::iter::once(0))
            .collect();
        self.validate()?;
        Ok(self)
    }
    pub fn load_options(mut self, load_options: Vec<u16>) -> BootResult<Self> {
        let Self::EfiImage { options, .. } = &mut self;
        *options = load_options;
        self.validate()?;
        Ok(self)
    }
    pub(crate) fn validate(&self) -> BootResult<()> {
        let Self::EfiImage { path, options, .. } = self;
        if !path.starts_with('\\')
            || path.encode_utf16().count() > 512
            || path.chars().any(char::is_control)
            || path.contains('/')
            || path[1..]
                .split('\\')
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(BootError::Configuration("EFI image paths must be absolute volume paths without empty, dot, or parent components".into()));
        }
        if options.len() > 32768 {
            return Err(BootError::Configuration(
                "EFI load options must fit in 64 KiB".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BootProgress {
    #[default]
    Unknown,
    Measured {
        completed: u64,
        total: u64,
    },
}

impl BootProgress {
    pub fn fraction(self) -> Option<f32> {
        match self {
            Self::Unknown => None,
            Self::Measured { completed, total } if total > 0 && completed <= total => {
                Some(completed as f32 / total as f32)
            }
            _ => None,
        }
    }
    pub(crate) fn validate(self) -> BootResult<()> {
        if matches!(self, Self::Unknown) || self.fraction().is_some() {
            Ok(())
        } else {
            Err(BootError::Host(
                "measured progress requires a positive total and completed <= total".into(),
            ))
        }
    }
}

#[derive(Clone, Debug)]
pub enum BootCommand {
    Select(usize),
    MoveSelection(i32),
    Launch,
    Boot(TargetId),
    Retry,
    Reset,
    SetTheme(BootTheme),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BootRequestId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootRequest {
    pub id: BootRequestId,
    pub target: BootTarget,
    pub source: BootSource,
}

#[derive(Clone, Debug)]
pub enum BootHostEvent {
    Loading {
        request: BootRequestId,
        status: String,
        progress: BootProgress,
    },
    Handoff {
        request: BootRequestId,
    },
    OsStarting {
        request: BootRequestId,
        status: String,
        progress: BootProgress,
    },
    Complete {
        request: BootRequestId,
    },
    Failed {
        request: BootRequestId,
        message: String,
    },
}

#[derive(Clone, Debug)]
pub enum PreviewCommand {
    Select(usize),
    MoveSelection(i32),
    Launch,
    Boot(TargetId),
    Retry,
    Reset,
    SetTheme(PreviewTheme),
    TogglePause,
    Fail,
    ContinueOs,
    SetScenario(PreviewScenario),
}

#[doc(hidden)]
#[derive(Clone, Debug)]
pub enum BootControl {
    Boot(BootCommand),
    #[cfg(feature = "boot-preview")]
    Preview(PreviewCommand),
}

impl From<BootCommand> for BootControl {
    fn from(command: BootCommand) -> Self {
        Self::Boot(command)
    }
}

#[cfg(feature = "boot-preview")]
impl From<PreviewCommand> for BootControl {
    fn from(command: PreviewCommand) -> Self {
        Self::Preview(command)
    }
}

#[cfg(feature = "boot-preview")]
impl From<BootCommand> for PreviewCommand {
    fn from(command: BootCommand) -> Self {
        match command {
            BootCommand::Select(index) => Self::Select(index),
            BootCommand::MoveSelection(delta) => Self::MoveSelection(delta),
            BootCommand::Launch => Self::Launch,
            BootCommand::Boot(id) => Self::Boot(id),
            BootCommand::Retry => Self::Retry,
            BootCommand::Reset => Self::Reset,
            BootCommand::SetTheme(theme) => Self::SetTheme(theme),
        }
    }
}

//! Visible title presentation and capability-checked begin-move intentions.

use std::fmt;
use std::sync::Arc;

use crate::foundation::ColorRgba8;
use crate::runtime::{RuntimeError, Ui};
use crate::shell::{
    ClientSurfaceSnapshot, InputSource, OutputId, ShellCapabilities, ShellGrantToken,
    SurfaceCapabilities, SurfaceId, SurfaceInputContact, SurfaceRequest, SurfaceRevision,
};
use crate::components::shell::primitives::ShellRootRef;
use crate::ui::{
    BoxStyle, ControlHandle, Flow, LayoutStyle, Property, SemanticName, SemanticNode,
    SemanticParticipation, SemanticRole, TextHandle, TextStyle, TextVisual, UiNodeId,
};

use super::WindowFrameRef;

/// Caller-owned titlebar visuals. The title remains visible text and is never inferred from pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowTitlebarStyle {
    pub container: BoxStyle,
    pub title: BoxStyle,
    pub controls: BoxStyle,
    pub layout: LayoutStyle,
    pub title_layout: LayoutStyle,
    pub controls_layout: LayoutStyle,
    pub title_color: ColorRgba8,
    pub title_size: f32,
    pub title_line_height: f32,
    pub title_family: String,
    pub title_weight: u16,
}

impl Default for WindowTitlebarStyle {
    fn default() -> Self {
        Self {
            container: BoxStyle::default(),
            title: BoxStyle::default(),
            controls: BoxStyle::default(),
            layout: LayoutStyle {
                flow: Flow::Horizontal,
                ..LayoutStyle::default()
            },
            title_layout: LayoutStyle::default(),
            controls_layout: LayoutStyle {
                flow: Flow::Horizontal,
                ..LayoutStyle::default()
            },
            title_color: ColorRgba8::rgba(245, 247, 250, 255),
            title_size: 14.0,
            title_line_height: 17.5,
            title_family: "sans-serif".to_owned(),
            title_weight: 500,
        }
    }
}

/// One exact surface titlebar. Move requests are returned as values and never executed here.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowTitlebar {
    label: String,
    snapshot: Arc<ClientSurfaceSnapshot>,
    style: WindowTitlebarStyle,
}

impl WindowTitlebar {
    pub fn new(
        label: impl Into<String>,
        snapshot: ClientSurfaceSnapshot,
    ) -> Result<Self, WindowTitlebarError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(WindowTitlebarError::MissingTitle);
        }
        Ok(Self {
            label,
            snapshot: Arc::new(snapshot),
            style: WindowTitlebarStyle::default(),
        })
    }

    pub fn from_snapshot_title(
        snapshot: ClientSurfaceSnapshot,
    ) -> Result<Self, WindowTitlebarError> {
        let title = snapshot
            .title()
            .ok_or(WindowTitlebarError::MissingSurfaceTitle)?
            .as_str()
            .to_owned();
        Self::new(title, snapshot)
    }

    pub fn style(mut self, style: WindowTitlebarStyle) -> Result<Self, WindowTitlebarError> {
        validate_style(&style)?;
        self.style = style;
        Ok(self)
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn snapshot(&self) -> &ClientSurfaceSnapshot {
        &self.snapshot
    }

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        frame: &WindowFrameRef,
    ) -> Result<WindowTitlebarRef, WindowTitlebarMountError> {
        validate_style(&self.style)?;
        if self.snapshot.as_ref() != frame.snapshot() {
            return Err(WindowTitlebarError::SurfaceSnapshotMismatch.into());
        }

        let root = ui
            .foundation()
            .container_node_under(
                frame.chrome_node(),
                self.style.container,
                self.style.layout,
                |_| {},
            )
            .ok_or_else(|| RuntimeError::new("window-titlebar frame is stale"))?;
        let content = ui.foundation().intern(&self.label);
        let family = ui.foundation().intern(&self.style.title_family);
        let title = ui
            .foundation()
            .text_node_under(
                root.node,
                TextVisual {
                    content,
                    style: TextStyle {
                        color: self.style.title_color,
                        size: self.style.title_size,
                        line_height: self.style.title_line_height,
                        family,
                        weight: self.style.title_weight,
                        align: crate::ui::TextAlign::Start,
                    },
                    revision: self.snapshot.revision().get(),
                },
                self.style.title,
                self.style.title_layout,
                true,
                false,
            )
            .ok_or_else(|| RuntimeError::new("window-titlebar title parent is stale"))?;
        let controls = ui
            .foundation()
            .container_node_under(
                root.node,
                self.style.controls,
                self.style.controls_layout,
                |_| {},
            )
            .ok_or_else(|| RuntimeError::new("window-titlebar controls parent is stale"))?;

        ui.foundation()
            .semantic_node(
                title.node,
                SemanticNode {
                    role: SemanticRole::Text,
                    name: SemanticName::Text(content),
                    ..SemanticNode::default()
                },
            )
            .map_err(semantic_runtime_error)?;
        for node in [root.node, controls.node] {
            ui.foundation()
                .semantic_node(
                    node,
                    SemanticNode {
                        participation: SemanticParticipation::MergeDescendants,
                        ..SemanticNode::default()
                    },
                )
                .map_err(semantic_runtime_error)?;
        }

        Ok(WindowTitlebarRef {
            root,
            title,
            controls,
            output: frame.output(),
            grant: frame.grant(),
            snapshot: Arc::clone(&self.snapshot),
        })
    }
}

fn validate_style(style: &WindowTitlebarStyle) -> Result<(), WindowTitlebarError> {
    if !style.title_size.is_finite() || style.title_size <= 0.0 {
        return Err(WindowTitlebarError::InvalidTitleSize);
    }
    if !style.title_line_height.is_finite() || style.title_line_height <= 0.0 {
        return Err(WindowTitlebarError::InvalidTitleLineHeight);
    }
    if style.title_family.trim().is_empty() {
        return Err(WindowTitlebarError::MissingTitleFamily);
    }
    if !(1..=1000).contains(&style.title_weight) {
        return Err(WindowTitlebarError::InvalidTitleWeight);
    }
    Ok(())
}

fn semantic_runtime_error(error: crate::ui::SemanticError) -> RuntimeError {
    RuntimeError::new(format!("invalid window-titlebar semantics: {error:?}"))
}

#[derive(Clone, Debug)]
pub struct WindowTitlebarRef {
    root: ControlHandle,
    title: TextHandle,
    controls: ControlHandle,
    output: OutputId,
    grant: ShellGrantToken,
    snapshot: Arc<ClientSurfaceSnapshot>,
}

impl WindowTitlebarRef {
    pub const fn node(&self) -> UiNodeId {
        self.root.node
    }

    pub const fn title_node(&self) -> UiNodeId {
        self.title.node
    }

    pub const fn controls_node(&self) -> UiNodeId {
        self.controls.node
    }

    pub const fn output(&self) -> OutputId {
        self.output
    }

    pub const fn grant(&self) -> ShellGrantToken {
        self.grant
    }

    pub fn snapshot(&self) -> &ClientSurfaceSnapshot {
        &self.snapshot
    }

    pub fn surface(&self) -> SurfaceId {
        self.snapshot.id()
    }

    pub fn revision(&self) -> SurfaceRevision {
        self.snapshot.revision()
    }

    pub const fn style(&self) -> Property<BoxStyle> {
        self.root.style
    }

    pub fn begin_move_intent(
        &self,
        root: ShellRootRef,
        contact: SurfaceInputContact,
    ) -> Result<TitlebarMoveIntent, TitlebarMoveError> {
        if root.output() != self.output {
            return Err(TitlebarMoveError::OutputMismatch);
        }
        if root.grant().token() != self.grant {
            return Err(TitlebarMoveError::GrantMismatch);
        }
        if !root.grant().permits(ShellCapabilities::MOVE_SURFACE) {
            return Err(TitlebarMoveError::NotAuthorized);
        }
        if !self
            .snapshot
            .capabilities()
            .contains(SurfaceCapabilities::MOVE)
        {
            return Err(TitlebarMoveError::SurfaceNotCapable);
        }
        Ok(TitlebarMoveIntent {
            request: SurfaceRequest::BeginMove {
                surface: self.surface(),
                contact: contact.contact(),
            },
            revision: self.revision(),
            contact,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TitlebarMoveIntent {
    request: SurfaceRequest,
    revision: SurfaceRevision,
    contact: SurfaceInputContact,
}

impl TitlebarMoveIntent {
    pub const fn request(self) -> SurfaceRequest {
        self.request
    }

    pub const fn revision(self) -> SurfaceRevision {
        self.revision
    }

    pub const fn contact(self) -> SurfaceInputContact {
        self.contact
    }

    pub const fn source(self) -> InputSource {
        self.contact.source()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowTitlebarError {
    MissingTitle,
    MissingSurfaceTitle,
    InvalidTitleSize,
    InvalidTitleLineHeight,
    MissingTitleFamily,
    InvalidTitleWeight,
    SurfaceSnapshotMismatch,
}

impl fmt::Display for WindowTitlebarError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid window titlebar: {self:?}")
    }
}

impl std::error::Error for WindowTitlebarError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitlebarMoveError {
    OutputMismatch,
    GrantMismatch,
    NotAuthorized,
    SurfaceNotCapable,
}

impl fmt::Display for TitlebarMoveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "window titlebar cannot begin move: {self:?}")
    }
}

impl std::error::Error for TitlebarMoveError {}

#[derive(Debug)]
pub enum WindowTitlebarMountError {
    Titlebar(WindowTitlebarError),
    Runtime(RuntimeError),
}

impl fmt::Display for WindowTitlebarMountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Titlebar(error) => error.fmt(formatter),
            Self::Runtime(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for WindowTitlebarMountError {}

impl From<WindowTitlebarError> for WindowTitlebarMountError {
    fn from(value: WindowTitlebarError) -> Self {
        Self::Titlebar(value)
    }
}

impl From<RuntimeError> for WindowTitlebarMountError {
    fn from(value: RuntimeError) -> Self {
        Self::Runtime(value)
    }
}

// `TitleBar` is the canonical spelling in the composable-window API. Keep the earlier
// `Titlebar` spellings as source-compatible names for imperative shell components.
pub type WindowTitleBar = WindowTitlebar;
pub type WindowTitleBarStyle = WindowTitlebarStyle;
pub type WindowTitleBarRef = WindowTitlebarRef;
pub type WindowTitleBarError = WindowTitlebarError;
pub type WindowTitleBarMountError = WindowTitlebarMountError;
pub type TitleBarMoveIntent = TitlebarMoveIntent;
pub type TitleBarMoveError = TitlebarMoveError;

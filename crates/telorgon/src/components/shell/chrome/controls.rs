//! Capability-derived window controls that emit typed, revision-bound requests.

use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

use crate::foundation::ColorRgba8;
use crate::input::Activation;
use crate::runtime::{RuntimeError, Ui};
use crate::shell::{
    ClientSurfaceSnapshot, SurfaceCapabilities, SurfaceId, SurfaceRequest, SurfaceRevision,
    SurfaceStates,
};
use crate::components::shell::primitives::ShellRootRef;
use crate::ui::{
    BoxStyle, ControlHandle, Flow, LayoutStyle, SemanticActions, SemanticName, SemanticNode,
    SemanticRole, SemanticState, StylePropertyPatch, StyleSlotId, UiNodeId,
};

use super::WindowTitlebarRef;
use crate::components::shell::WindowControlsStyleId;

/// Canonical control identities. Maximize and Restore are mutually exclusive presentations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WindowControl {
    Minimize,
    Maximize,
    Restore,
    Close,
}

impl WindowControl {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Minimize => "Minimize window",
            Self::Maximize => "Maximize window",
            Self::Restore => "Restore window",
            Self::Close => "Close window",
        }
    }

    pub const fn required_surface_capability(self) -> SurfaceCapabilities {
        match self {
            Self::Minimize => SurfaceCapabilities::MINIMIZE,
            Self::Maximize | Self::Restore => SurfaceCapabilities::MAXIMIZE,
            Self::Close => SurfaceCapabilities::CLOSE,
        }
    }

    pub const fn request(self, surface: SurfaceId) -> SurfaceRequest {
        match self {
            Self::Minimize => SurfaceRequest::SetMinimized {
                surface,
                minimized: true,
            },
            Self::Maximize => SurfaceRequest::SetMaximized {
                surface,
                maximized: true,
            },
            Self::Restore => SurfaceRequest::SetMaximized {
                surface,
                maximized: false,
            },
            Self::Close => SurfaceRequest::Close { surface },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowControlsStyle {
    pub container: BoxStyle,
    pub control: BoxStyle,
    pub layout: LayoutStyle,
    pub label_color: ColorRgba8,
    pub label_size: f32,
}

impl Default for WindowControlsStyle {
    fn default() -> Self {
        Self {
            container: BoxStyle::default(),
            control: BoxStyle::default(),
            layout: LayoutStyle {
                flow: Flow::Horizontal,
                ..LayoutStyle::default()
            },
            label_color: ColorRgba8::rgba(245, 247, 250, 255),
            label_size: 12.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowControls {
    snapshot: Arc<ClientSurfaceSnapshot>,
    style: WindowControlsStyle,
    style_id: WindowControlsStyleId,
    style_override: StylePropertyPatch,
}

impl WindowControls {
    pub fn new(snapshot: ClientSurfaceSnapshot) -> Self {
        Self {
            snapshot: Arc::new(snapshot),
            style: WindowControlsStyle::default(),
            style_id: WindowControlsStyleId::DEFAULT,
            style_override: StylePropertyPatch::default(),
        }
    }

    pub fn style(mut self, style: WindowControlsStyle) -> Result<Self, WindowControlsError> {
        if !style.label_size.is_finite() || style.label_size <= 0.0 {
            return Err(WindowControlsError::InvalidLabelSize);
        }
        self.style = style;
        Ok(self)
    }

    pub fn style_id(mut self, style_id: WindowControlsStyleId) -> Self {
        self.style_id = style_id;
        self
    }

    pub fn style_override(mut self, style: StylePropertyPatch) -> Self {
        self.style_override = style;
        self
    }

    pub fn snapshot(&self) -> &ClientSurfaceSnapshot {
        &self.snapshot
    }

    /// Controls available in stable leading-to-trailing order from the exact surface snapshot.
    pub fn available(&self) -> Vec<WindowControl> {
        let capabilities = self.snapshot.capabilities();
        let mut controls = Vec::with_capacity(3);
        if capabilities.contains(SurfaceCapabilities::MINIMIZE) {
            controls.push(WindowControl::Minimize);
        }
        if capabilities.contains(SurfaceCapabilities::MAXIMIZE) {
            controls.push(
                if self.snapshot.states().contains(SurfaceStates::MAXIMIZED) {
                    WindowControl::Restore
                } else {
                    WindowControl::Maximize
                },
            );
        }
        if capabilities.contains(SurfaceCapabilities::CLOSE) {
            controls.push(WindowControl::Close);
        }
        controls
    }

    pub fn mount<Action, Map>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        titlebar: &WindowTitlebarRef,
        root: ShellRootRef,
        map: Map,
    ) -> Result<WindowControlsRef, WindowControlsMountError>
    where
        Action: 'static,
        Map: Fn(WindowControlIntent) -> Action + 'static,
    {
        if !self.style.label_size.is_finite() || self.style.label_size <= 0.0 {
            return Err(WindowControlsError::InvalidLabelSize.into());
        }
        if self.snapshot.as_ref() != titlebar.snapshot() {
            return Err(WindowControlsError::SurfaceSnapshotMismatch.into());
        }
        if root.output() != titlebar.output() {
            return Err(WindowControlsError::OutputMismatch.into());
        }
        if root.grant().token() != titlebar.grant() {
            return Err(WindowControlsError::GrantMismatch.into());
        }

        let container = ui
            .foundation()
            .container_node_under(
                titlebar.controls_node(),
                self.style.container,
                self.style.layout,
                |_| {},
            )
            .ok_or_else(|| RuntimeError::new("window-controls titlebar is stale"))?;
        ui.foundation().style_id(container.node, self.style_id.0);
        if self.style_override != StylePropertyPatch::default() {
            ui.foundation().style_override(
                container.node,
                StyleSlotId::named("root"),
                self.style_override,
            );
        }
        let controls_name = ui.foundation().intern("Window controls");
        ui.foundation()
            .semantic_node(
                container.node,
                SemanticNode::named(SemanticRole::Toolbar, controls_name),
            )
            .map_err(semantic_runtime_error)?;

        let map = Rc::new(map);
        let mut mounted = Vec::new();
        for kind in self.available() {
            let request = kind.request(self.snapshot.id());
            let enabled = root.grant().permits(request.required_shell_capability());
            let label = kind.label();
            let node = ui
                .foundation()
                .action_node_under(
                    container.node,
                    self.style.control,
                    enabled,
                    true,
                    |writer| {
                        writer.text(label, self.style.label_color, self.style.label_size);
                    },
                )
                .ok_or_else(|| RuntimeError::new("window-controls container is stale"))?;
            let name = ui.foundation().intern(label);
            let actions = if enabled {
                SemanticActions::FOCUS | SemanticActions::ACTIVATE
            } else {
                SemanticActions::NONE
            };
            ui.foundation()
                .semantic_node(
                    node.node,
                    SemanticNode {
                        role: SemanticRole::Button,
                        name: SemanticName::Text(name),
                        state: SemanticState {
                            disabled: !enabled,
                            focusable: enabled,
                            ..SemanticState::default()
                        },
                        actions,
                        ..SemanticNode::default()
                    },
                )
                .map_err(semantic_runtime_error)?;
            if !enabled {
                ui.foundation().disabled(node.node, true);
            } else {
                let map = Rc::clone(&map);
                let revision = self.snapshot.revision();
                ui.route_activation(node.node, move |activation| {
                    map(WindowControlIntent {
                        control: kind,
                        request,
                        revision,
                        activation,
                    })
                })?;
            }
            mounted.push(WindowControlRef {
                control: kind,
                node,
                enabled,
            });
        }

        Ok(WindowControlsRef {
            container,
            controls: mounted,
            snapshot: Arc::clone(&self.snapshot),
        })
    }
}

fn semantic_runtime_error(error: crate::ui::SemanticError) -> RuntimeError {
    RuntimeError::new(format!("invalid window-controls semantics: {error:?}"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowControlIntent {
    control: WindowControl,
    request: SurfaceRequest,
    revision: SurfaceRevision,
    activation: Activation,
}

impl WindowControlIntent {
    pub const fn control(self) -> WindowControl {
        self.control
    }

    pub const fn request(self) -> SurfaceRequest {
        self.request
    }

    pub const fn revision(self) -> SurfaceRevision {
        self.revision
    }

    pub const fn activation(self) -> Activation {
        self.activation
    }
}

#[derive(Clone, Debug)]
pub struct WindowControlsRef {
    container: ControlHandle,
    controls: Vec<WindowControlRef>,
    snapshot: Arc<ClientSurfaceSnapshot>,
}

impl WindowControlsRef {
    pub const fn node(&self) -> UiNodeId {
        self.container.node
    }

    pub fn controls(&self) -> &[WindowControlRef] {
        &self.controls
    }

    pub fn control(&self, control: WindowControl) -> Option<WindowControlRef> {
        self.controls
            .iter()
            .copied()
            .find(|reference| reference.control == control)
    }

    pub fn snapshot(&self) -> &ClientSurfaceSnapshot {
        &self.snapshot
    }
}

#[derive(Clone, Copy, Debug)]
pub struct WindowControlRef {
    control: WindowControl,
    node: ControlHandle,
    enabled: bool,
}

impl WindowControlRef {
    pub const fn control(self) -> WindowControl {
        self.control
    }

    pub const fn node(self) -> UiNodeId {
        self.node.node
    }

    pub const fn enabled(self) -> bool {
        self.enabled
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowControlsError {
    InvalidLabelSize,
    SurfaceSnapshotMismatch,
    OutputMismatch,
    GrantMismatch,
}

impl fmt::Display for WindowControlsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid window controls: {self:?}")
    }
}

impl std::error::Error for WindowControlsError {}

#[derive(Debug)]
pub enum WindowControlsMountError {
    Controls(WindowControlsError),
    Runtime(RuntimeError),
}

impl fmt::Display for WindowControlsMountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Controls(error) => error.fmt(formatter),
            Self::Runtime(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for WindowControlsMountError {}

impl From<WindowControlsError> for WindowControlsMountError {
    fn from(value: WindowControlsError) -> Self {
        Self::Controls(value)
    }
}

impl From<RuntimeError> for WindowControlsMountError {
    fn from(value: RuntimeError) -> Self {
        Self::Runtime(value)
    }
}

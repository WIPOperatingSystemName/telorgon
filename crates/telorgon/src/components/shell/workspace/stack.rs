//! Exact back-to-front workspace surface reconciliation and frame mounting.

use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

use crate::foundation::PointF;
use crate::runtime::{RuntimeError, Ui};
use crate::shell::{
    ClientSurfaceSnapshot, OutputId, OutputRevision, ShellGrantToken, SurfaceId, WorkspaceId,
    WorkspaceRevision, WorkspaceSnapshot, WorkspaceSurface,
};
use crate::components::shell::primitives::OutputViewRef;
use crate::ui::{
    BoxStyle, ControlHandle, Flow, LayoutStyle, Property, SemanticNode, SemanticParticipation,
    SizeRule, UiNodeId,
};

use crate::components::shell::{WindowFrame, WindowFrameRef, WindowFrameStyle, WorkspaceViewRef};

pub(crate) mod sealed {
    pub trait Sealed {}
}

/// A standard workspace container that can host one exact [`WindowStack`].
pub trait WindowStackHost: sealed::Sealed {
    fn content_node(&self) -> UiNodeId;
    fn workspace(&self) -> WorkspaceId;
    fn workspace_revision(&self) -> WorkspaceRevision;
    fn output(&self) -> OutputId;
    fn output_revision(&self) -> OutputRevision;
    fn grant(&self) -> ShellGrantToken;
    fn global_origin(&self) -> PointF;
    fn placements(&self) -> &[WorkspaceSurface];
}

impl sealed::Sealed for WorkspaceViewRef {}

impl WindowStackHost for WorkspaceViewRef {
    fn content_node(&self) -> UiNodeId {
        self.content_node()
    }

    fn workspace(&self) -> WorkspaceId {
        self.workspace()
    }

    fn workspace_revision(&self) -> WorkspaceRevision {
        self.revision()
    }

    fn output(&self) -> OutputId {
        self.output()
    }

    fn output_revision(&self) -> OutputRevision {
        self.output_revision()
    }

    fn grant(&self) -> ShellGrantToken {
        self.grant()
    }

    fn global_origin(&self) -> PointF {
        self.output_origin()
    }

    fn placements(&self) -> &[WorkspaceSurface] {
        self.placements()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowStackEntry {
    label: String,
    snapshot: ClientSurfaceSnapshot,
    frame_style: WindowFrameStyle,
}

impl WindowStackEntry {
    pub fn new(
        label: impl Into<String>,
        snapshot: ClientSurfaceSnapshot,
    ) -> Result<Self, WindowStackError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(WindowStackError::MissingWindowName);
        }
        Ok(Self {
            label,
            snapshot,
            frame_style: WindowFrameStyle::default(),
        })
    }

    pub const fn frame_style(mut self, style: WindowFrameStyle) -> Self {
        self.frame_style = style;
        self
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub const fn snapshot(&self) -> &ClientSurfaceSnapshot {
        &self.snapshot
    }

    pub fn surface(&self) -> SurfaceId {
        self.snapshot.id()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowStackStyle {
    pub container: BoxStyle,
    pub layout: LayoutStyle,
}

impl Default for WindowStackStyle {
    fn default() -> Self {
        Self {
            container: BoxStyle {
                width: SizeRule::Fill(1.0),
                height: SizeRule::Fill(1.0),
                ..BoxStyle::default()
            },
            layout: LayoutStyle {
                flow: Flow::Overlay,
                ..LayoutStyle::default()
            },
        }
    }
}

/// Reconciled top-level surfaces in the workspace snapshot's exact painter order.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowStack {
    workspace: Arc<WorkspaceSnapshot>,
    output: OutputId,
    placements: Arc<[WorkspaceSurface]>,
    entries: Arc<[WindowStackEntry]>,
    style: WindowStackStyle,
}

impl WindowStack {
    pub fn new(
        workspace: WorkspaceSnapshot,
        output: OutputId,
        entries: Vec<WindowStackEntry>,
    ) -> Result<Self, WindowStackError> {
        let mut seen = HashSet::with_capacity(entries.len());
        for entry in &entries {
            if !seen.insert(entry.surface()) {
                return Err(WindowStackError::DuplicateSurface {
                    surface: entry.surface(),
                });
            }
        }
        let placements: Vec<_> = workspace
            .surfaces()
            .iter()
            .copied()
            .filter(|placement| placement.output() == output && seen.contains(&placement.surface()))
            .collect();
        if entries.len() != placements.len() {
            return Err(WindowStackError::SurfaceSetMismatch);
        }
        for (index, (placement, entry)) in placements.iter().zip(&entries).enumerate() {
            if entry.snapshot.parent().is_some() {
                return Err(WindowStackError::SubsurfaceEntry {
                    surface: entry.surface(),
                });
            }
            if placement.surface() != entry.surface() {
                return Err(WindowStackError::PainterOrderMismatch { index });
            }
            if placement.bounds() != entry.snapshot.geometry().logical_bounds() {
                return Err(WindowStackError::GeometryMismatch {
                    surface: entry.surface(),
                });
            }
        }
        Ok(Self {
            workspace: Arc::new(workspace),
            output,
            placements: placements.into(),
            entries: entries.into(),
            style: WindowStackStyle::default(),
        })
    }

    pub const fn style(mut self, style: WindowStackStyle) -> Self {
        self.style = style;
        self
    }

    pub fn workspace(&self) -> &WorkspaceSnapshot {
        &self.workspace
    }

    pub const fn output(&self) -> OutputId {
        self.output
    }

    pub fn entries(&self) -> &[WindowStackEntry] {
        &self.entries
    }

    pub fn mount<Action: 'static, Host: WindowStackHost>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: &Host,
        output: OutputViewRef,
    ) -> Result<WindowStackRef, WindowStackMountError> {
        if host.workspace() != self.workspace.id()
            || host.workspace_revision() != self.workspace.revision()
            || host.placements() != self.placements.as_ref()
        {
            return Err(WindowStackError::WorkspaceSnapshotMismatch.into());
        }
        if host.output() != self.output || output.output() != self.output {
            return Err(WindowStackError::OutputMismatch.into());
        }
        if host.output_revision() != output.revision() {
            return Err(WindowStackError::OutputRevisionMismatch.into());
        }

        let root = ui
            .foundation()
            .container_node_under(
                host.content_node(),
                self.style.container,
                self.style.layout,
                |_| {},
            )
            .ok_or_else(|| RuntimeError::new("window-stack host is stale"))?;
        ui.foundation()
            .semantic_node(
                root.node,
                SemanticNode {
                    participation: SemanticParticipation::MergeDescendants,
                    ..SemanticNode::default()
                },
            )
            .map_err(|error| {
                RuntimeError::new(format!("invalid window-stack semantics: {error:?}"))
            })?;

        let mut frames = Vec::with_capacity(self.entries.len());
        for entry in self.entries.iter() {
            let frame = WindowFrame::new(entry.label.clone(), entry.snapshot.clone())
                .expect("window-stack entries were validated")
                .style(entry.frame_style)
                .mount_under(
                    ui,
                    root.node,
                    self.output,
                    output.revision(),
                    host.grant(),
                    host.global_origin(),
                )?;
            frames.push(frame);
        }
        Ok(WindowStackRef {
            root,
            workspace: Arc::clone(&self.workspace),
            output: self.output,
            output_revision: output.revision(),
            placements: Arc::clone(&self.placements),
            frames,
        })
    }
}

#[derive(Clone, Debug)]
pub struct WindowStackRef {
    root: ControlHandle,
    workspace: Arc<WorkspaceSnapshot>,
    output: OutputId,
    output_revision: OutputRevision,
    placements: Arc<[WorkspaceSurface]>,
    frames: Vec<WindowFrameRef>,
}

impl WindowStackRef {
    pub const fn node(&self) -> UiNodeId {
        self.root.node
    }

    pub fn workspace(&self) -> &WorkspaceSnapshot {
        &self.workspace
    }

    pub const fn output(&self) -> OutputId {
        self.output
    }

    pub const fn output_revision(&self) -> OutputRevision {
        self.output_revision
    }

    pub fn placements(&self) -> &[WorkspaceSurface] {
        &self.placements
    }

    pub fn frames(&self) -> &[WindowFrameRef] {
        &self.frames
    }

    pub fn frame(&self, surface: SurfaceId) -> Option<&WindowFrameRef> {
        self.frames.iter().find(|frame| frame.surface() == surface)
    }

    pub const fn style(&self) -> Property<BoxStyle> {
        self.root.style
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WindowStackError {
    MissingWindowName,
    SurfaceSetMismatch,
    DuplicateSurface { surface: SurfaceId },
    SubsurfaceEntry { surface: SurfaceId },
    PainterOrderMismatch { index: usize },
    GeometryMismatch { surface: SurfaceId },
    WorkspaceSnapshotMismatch,
    OutputMismatch,
    OutputRevisionMismatch,
}

impl fmt::Display for WindowStackError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid window stack: {self:?}")
    }
}

impl std::error::Error for WindowStackError {}

#[derive(Debug)]
pub enum WindowStackMountError {
    Stack(WindowStackError),
    Runtime(RuntimeError),
}

impl fmt::Display for WindowStackMountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stack(error) => error.fmt(formatter),
            Self::Runtime(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for WindowStackMountError {}

impl From<WindowStackError> for WindowStackMountError {
    fn from(value: WindowStackError) -> Self {
        Self::Stack(value)
    }
}

impl From<RuntimeError> for WindowStackMountError {
    fn from(value: RuntimeError) -> Self {
        Self::Runtime(value)
    }
}

//! Legacy noninteractive frame-shadow adapter.
//!
//! New composition code should put the reusable [`Shadow`] value in
//! [`crate::ui::BoxDecoration::shadows`]. This adapter remains available for shell code built on
//! the older imperative `WindowFrameRef` mounting API.

use std::fmt;
use std::sync::Arc;

use crate::runtime::{RuntimeError, Ui};
use crate::shell::{ClientSurfaceSnapshot, OutputId, SurfaceId, SurfaceRevision};
use crate::ui::{
    BoxStyle, ControlHandle, LayoutStyle, SemanticNode, SemanticParticipation, Shadow, ShadowList,
    SizeRule, UiNodeId,
};

use super::WindowFrameRef;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowFrameStyle {
    pub container: BoxStyle,
    pub layout: LayoutStyle,
    pub shadow: Shadow,
}

impl ShadowFrameStyle {
    pub fn new(shadow: Shadow) -> Result<Self, ShadowFrameError> {
        validate_shadow(shadow)?;
        Ok(Self {
            container: BoxStyle {
                width: SizeRule::Fill(1.0),
                height: SizeRule::Fill(1.0),
                ..BoxStyle::default()
            },
            layout: LayoutStyle::default(),
            shadow,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShadowFrame {
    snapshot: Arc<ClientSurfaceSnapshot>,
    style: ShadowFrameStyle,
}

impl ShadowFrame {
    pub fn new(snapshot: ClientSurfaceSnapshot, shadow: Shadow) -> Result<Self, ShadowFrameError> {
        Ok(Self {
            snapshot: Arc::new(snapshot),
            style: ShadowFrameStyle::new(shadow)?,
        })
    }

    pub fn style(mut self, style: ShadowFrameStyle) -> Result<Self, ShadowFrameError> {
        validate_shadow(style.shadow)?;
        self.style = style;
        Ok(self)
    }

    pub fn snapshot(&self) -> &ClientSurfaceSnapshot {
        &self.snapshot
    }

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        frame: &WindowFrameRef,
    ) -> Result<ShadowFrameRef, ShadowFrameMountError> {
        validate_shadow(self.style.shadow)?;
        if self.snapshot.as_ref() != frame.snapshot() {
            return Err(ShadowFrameError::SurfaceSnapshotMismatch.into());
        }
        let mut style = self.style.container;
        style.decoration.shadows = ShadowList::one(self.style.shadow);
        let control = ui
            .foundation()
            .container_node_under(frame.decoration_node(), style, self.style.layout, |_| {})
            .ok_or_else(|| RuntimeError::new("shadow-frame decoration host is stale"))?;
        ui.foundation()
            .semantic_node(
                control.node,
                SemanticNode {
                    participation: SemanticParticipation::Exclude,
                    ..SemanticNode::default()
                },
            )
            .map_err(|error| {
                RuntimeError::new(format!("invalid shadow-frame semantics: {error:?}"))
            })?;
        Ok(ShadowFrameRef {
            control,
            output: frame.output(),
            snapshot: Arc::clone(&self.snapshot),
        })
    }
}

fn validate_shadow(shadow: Shadow) -> Result<(), ShadowFrameError> {
    if !shadow.offset.x.is_finite() || !shadow.offset.y.is_finite() {
        return Err(ShadowFrameError::NonFiniteOffset);
    }
    if !shadow.blur.is_finite() || shadow.blur < 0.0 {
        return Err(ShadowFrameError::InvalidBlur);
    }
    if !shadow.spread.is_finite() || shadow.spread < 0.0 {
        return Err(ShadowFrameError::InvalidSpread);
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct ShadowFrameRef {
    control: ControlHandle,
    output: OutputId,
    snapshot: Arc<ClientSurfaceSnapshot>,
}

impl ShadowFrameRef {
    pub const fn node(&self) -> UiNodeId {
        self.control.node
    }

    pub const fn output(&self) -> OutputId {
        self.output
    }

    pub fn surface(&self) -> SurfaceId {
        self.snapshot.id()
    }

    pub fn revision(&self) -> SurfaceRevision {
        self.snapshot.revision()
    }

    pub fn snapshot(&self) -> &ClientSurfaceSnapshot {
        &self.snapshot
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadowFrameError {
    NonFiniteOffset,
    InvalidBlur,
    InvalidSpread,
    SurfaceSnapshotMismatch,
}

impl fmt::Display for ShadowFrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid shadow frame: {self:?}")
    }
}

impl std::error::Error for ShadowFrameError {}

#[derive(Debug)]
pub enum ShadowFrameMountError {
    Shadow(ShadowFrameError),
    Runtime(RuntimeError),
}

impl fmt::Display for ShadowFrameMountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Shadow(error) => error.fmt(formatter),
            Self::Runtime(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ShadowFrameMountError {}

impl From<ShadowFrameError> for ShadowFrameMountError {
    fn from(value: ShadowFrameError) -> Self {
        Self::Shadow(value)
    }
}

impl From<RuntimeError> for ShadowFrameMountError {
    fn from(value: RuntimeError) -> Self {
        Self::Runtime(value)
    }
}

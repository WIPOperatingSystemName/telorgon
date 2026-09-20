//! Authorized, canonically ordered shell layer content hosts.

use std::fmt;

use crate::runtime::{RuntimeError, Ui};
use crate::shell::{LayerAuthority, OutputId, ShellLayerKind};
use crate::ui::{
    BoxStyle, ControlHandle, LayoutStyle, Property, SemanticNode, SemanticParticipation, UiNodeId,
};

use crate::components::shell::primitives::OutputViewRef;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShellLayerStyle {
    pub container: BoxStyle,
    pub layout: LayoutStyle,
}

/// Mount-time order owner for one output. Layers may be omitted but never duplicated or reversed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShellLayerOrder {
    output: OutputId,
    last: Option<ShellLayerKind>,
}

impl ShellLayerOrder {
    pub const fn new(output: OutputId) -> Self {
        Self { output, last: None }
    }

    pub const fn output(self) -> OutputId {
        self.output
    }

    pub const fn last(self) -> Option<ShellLayerKind> {
        self.last
    }

    fn validate(&self, authority: LayerAuthority) -> Result<(), ShellLayerError> {
        if authority.output() != self.output {
            return Err(ShellLayerError::OutputMismatch {
                expected: self.output,
                actual: authority.output(),
            });
        }
        if self.last.is_some_and(|last| authority.layer() <= last) {
            return Err(ShellLayerError::NonCanonicalOrder {
                previous: self.last.expect("checked as present"),
                requested: authority.layer(),
            });
        }
        Ok(())
    }

    fn commit(&mut self, layer: ShellLayerKind) {
        self.last = Some(layer);
    }
}

/// One layer authorized by a narrowed host grant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShellLayer {
    authority: LayerAuthority,
    style: ShellLayerStyle,
}

impl ShellLayer {
    pub fn new(authority: LayerAuthority) -> Self {
        Self {
            authority,
            style: ShellLayerStyle::default(),
        }
    }

    pub const fn style(mut self, style: ShellLayerStyle) -> Self {
        self.style = style;
        self
    }

    pub const fn authority(self) -> LayerAuthority {
        self.authority
    }

    pub const fn kind(self) -> ShellLayerKind {
        self.authority.layer()
    }

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        output: OutputViewRef,
        order: &mut ShellLayerOrder,
    ) -> Result<ShellLayerRef, ShellLayerMountError> {
        if output.output() != self.authority.output() {
            return Err(ShellLayerError::OutputMismatch {
                expected: output.output(),
                actual: self.authority.output(),
            }
            .into());
        }
        order.validate(self.authority)?;
        let content = ui
            .foundation()
            .container_node_under(
                output.content_node(),
                self.style.container,
                self.style.layout,
                |_| {},
            )
            .ok_or_else(|| RuntimeError::new("output view is stale"))?;
        ui.foundation()
            .semantic_node(
                content.node,
                SemanticNode {
                    participation: SemanticParticipation::MergeDescendants,
                    ..SemanticNode::default()
                },
            )
            .map_err(|error| {
                RuntimeError::new(format!("invalid shell layer semantics: {error:?}"))
            })?;
        order.commit(self.kind());
        Ok(ShellLayerRef {
            content,
            authority: self.authority,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ShellLayerRef {
    content: ControlHandle,
    authority: LayerAuthority,
}

impl ShellLayerRef {
    pub const fn node(self) -> UiNodeId {
        self.content.node
    }

    pub const fn content_node(self) -> UiNodeId {
        self.content.node
    }

    pub const fn output(self) -> OutputId {
        self.authority.output()
    }

    pub const fn kind(self) -> ShellLayerKind {
        self.authority.layer()
    }

    pub const fn authority(self) -> LayerAuthority {
        self.authority
    }

    pub const fn style(self) -> Property<BoxStyle> {
        self.content.style
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellLayerError {
    OutputMismatch {
        expected: OutputId,
        actual: OutputId,
    },
    NonCanonicalOrder {
        previous: ShellLayerKind,
        requested: ShellLayerKind,
    },
}

impl fmt::Display for ShellLayerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutputMismatch { .. } => {
                formatter.write_str("shell layer output does not match its output view")
            }
            Self::NonCanonicalOrder { .. } => {
                formatter.write_str("shell layers must mount once in canonical back-to-front order")
            }
        }
    }
}

impl std::error::Error for ShellLayerError {}

#[derive(Debug)]
pub enum ShellLayerMountError {
    Layer(ShellLayerError),
    Runtime(RuntimeError),
}

impl fmt::Display for ShellLayerMountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Layer(error) => error.fmt(formatter),
            Self::Runtime(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ShellLayerMountError {}

impl From<ShellLayerError> for ShellLayerMountError {
    fn from(value: ShellLayerError) -> Self {
        Self::Layer(value)
    }
}

impl From<RuntimeError> for ShellLayerMountError {
    fn from(value: RuntimeError) -> Self {
        Self::Runtime(value)
    }
}

#[cfg(test)]
mod tests {
    use crate::shell::{OutputId, ShellCapabilities, ShellCapabilityGrant, ShellGrantToken};

    use super::*;

    fn grant(output: u64) -> ShellCapabilityGrant {
        ShellCapabilityGrant::from_host(
            ShellGrantToken::from_raw(output).unwrap(),
            OutputId::from_raw(output).unwrap(),
            ShellCapabilities::BACKGROUND_LAYER
                | ShellCapabilities::WORKSPACE_LAYER
                | ShellCapabilities::PANEL_LAYER,
        )
    }

    #[test]
    fn order_accepts_skips_but_rejects_duplicates_and_backtracking() {
        let grant = grant(1);
        let mut order = ShellLayerOrder::new(grant.output());
        let background = grant.authorize_layer(ShellLayerKind::Background).unwrap();
        let panel = grant.authorize_layer(ShellLayerKind::Panel).unwrap();

        order.validate(background).unwrap();
        order.commit(background.layer());
        order.validate(panel).unwrap();
        order.commit(panel.layer());
        assert_eq!(
            order.validate(background),
            Err(ShellLayerError::NonCanonicalOrder {
                previous: ShellLayerKind::Panel,
                requested: ShellLayerKind::Background,
            })
        );
        assert_eq!(order.last(), Some(ShellLayerKind::Panel));
    }

    #[test]
    fn order_rejects_cross_output_authority() {
        let order = ShellLayerOrder::new(OutputId::from_raw(1).unwrap());
        let authority = grant(2).authorize_layer(ShellLayerKind::Panel).unwrap();
        assert!(matches!(
            order.validate(authority),
            Err(ShellLayerError::OutputMismatch { .. })
        ));
    }
}

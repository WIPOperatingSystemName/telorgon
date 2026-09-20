//! Authorized shell semantic scope and caller-content host for one output.

use std::fmt;

use crate::runtime::{RuntimeError, RuntimeResult, Ui};
use crate::shell::{
    LayerAuthority, LayerAuthorityError, OutputId, ShellCapabilityGrant, ShellLayerKind,
};
use crate::ui::{
    BoxStyle, ControlHandle, LayoutStyle, Property, SemanticName, SemanticNode,
    SemanticParticipation, SemanticRelationship, SemanticRelationshipKind, SemanticRole, UiNodeId,
};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShellRootStyle {
    pub container: BoxStyle,
    pub content: BoxStyle,
    pub layout: LayoutStyle,
    pub content_layout: LayoutStyle,
}

/// One explicitly named shell scope retaining a host-issued grant for exactly one output.
#[derive(Clone, Debug, PartialEq)]
pub struct ShellRoot {
    label: String,
    grant: ShellCapabilityGrant,
    style: ShellRootStyle,
}

impl ShellRoot {
    pub fn new(
        label: impl Into<String>,
        grant: ShellCapabilityGrant,
    ) -> Result<Self, ShellRootError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(ShellRootError::MissingAccessibleName);
        }
        Ok(Self {
            label,
            grant,
            style: ShellRootStyle::default(),
        })
    }

    pub const fn style(mut self, style: ShellRootStyle) -> Self {
        self.style = style;
        self
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub const fn output(&self) -> OutputId {
        self.grant.output()
    }

    pub const fn grant(&self) -> ShellCapabilityGrant {
        self.grant
    }

    pub const fn root_style(&self) -> ShellRootStyle {
        self.style
    }

    pub fn authorize_layer(
        &self,
        layer: ShellLayerKind,
    ) -> Result<LayerAuthority, LayerAuthorityError> {
        self.grant.authorize_layer(layer)
    }

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
    ) -> RuntimeResult<ShellRootRef> {
        let root = ui
            .foundation()
            .container_node_under(host, self.style.container, self.style.layout, |_| {})
            .ok_or_else(|| RuntimeError::new("shell root parent is stale"))?;
        let content = ui
            .foundation()
            .container_node_under(
                root.node,
                self.style.content,
                self.style.content_layout,
                |_| {},
            )
            .ok_or_else(|| RuntimeError::new("shell root content parent is stale"))?;

        ui.foundation()
            .semantic_node(
                content.node,
                SemanticNode {
                    participation: SemanticParticipation::MergeDescendants,
                    ..SemanticNode::default()
                },
            )
            .map_err(|error| {
                RuntimeError::new(format!("invalid shell root content semantics: {error:?}"))
            })?;
        let name = ui.foundation().intern(&self.label);
        ui.foundation()
            .semantic_node(
                root.node,
                SemanticNode {
                    role: SemanticRole::Region,
                    name: SemanticName::Text(name),
                    relationships: vec![SemanticRelationship {
                        kind: SemanticRelationshipKind::Owns,
                        target: content.node,
                    }],
                    ..SemanticNode::default()
                },
            )
            .map_err(|error| {
                RuntimeError::new(format!("invalid shell root semantics: {error:?}"))
            })?;

        Ok(ShellRootRef {
            root,
            content,
            grant: self.grant,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ShellRootRef {
    root: ControlHandle,
    content: ControlHandle,
    grant: ShellCapabilityGrant,
}

impl ShellRootRef {
    pub const fn node(self) -> UiNodeId {
        self.root.node
    }

    pub const fn content_node(self) -> UiNodeId {
        self.content.node
    }

    pub const fn output(self) -> OutputId {
        self.grant.output()
    }

    pub const fn grant(self) -> ShellCapabilityGrant {
        self.grant
    }

    pub const fn style(self) -> Property<BoxStyle> {
        self.root.style
    }

    pub const fn content_style(self) -> Property<BoxStyle> {
        self.content.style
    }

    pub fn authorize_layer(
        self,
        layer: ShellLayerKind,
    ) -> Result<LayerAuthority, LayerAuthorityError> {
        self.grant.authorize_layer(layer)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellRootError {
    MissingAccessibleName,
}

impl fmt::Display for ShellRootError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("shell root accessible name is empty")
    }
}

impl std::error::Error for ShellRootError {}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use crate::runtime::{Component, CreateContext, UpdateContext, ViewRuntime};
    use crate::shell::{OutputId, ShellCapabilities, ShellGrantToken};
    use crate::ui::{BoxStyle, LayoutStyle, SemanticRelationshipKind, UiRoot};

    use super::*;

    fn grant() -> ShellCapabilityGrant {
        ShellCapabilityGrant::from_host(
            ShellGrantToken::from_raw(1).unwrap(),
            OutputId::from_raw(2).unwrap(),
            ShellCapabilities::WORKSPACE_LAYER | ShellCapabilities::PANEL_LAYER,
        )
    }

    struct Fixture(Rc<Cell<Option<ShellRootRef>>>);

    impl Component for Fixture {
        type State = ();
        type Action = ();

        fn create(&self, _: &mut CreateContext<'_>) -> Self::State {}

        fn mount(&self, _: &Self::State, ui: &mut Ui<'_, '_, ()>) -> UiRoot {
            let host = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            self.0.set(Some(
                ShellRoot::new("System shell", grant())
                    .unwrap()
                    .mount(ui, host.0)
                    .unwrap(),
            ));
            host
        }

        fn action(&self, _: &mut (), _: (), _: &mut UpdateContext<'_, Self>) {}
    }

    #[test]
    fn root_is_named_owned_and_narrows_only_granted_layers() {
        assert_eq!(
            ShellRoot::new(" ", grant()),
            Err(ShellRootError::MissingAccessibleName)
        );
        let reference = Rc::new(Cell::new(None));
        let runtime = ViewRuntime::from_component(Fixture(reference.clone())).unwrap();
        let reference = reference.get().unwrap();
        let semantic = runtime.ui().semantics.get(reference.node()).unwrap();

        assert_eq!(reference.output().get(), 2);
        assert_eq!(semantic.role, SemanticRole::Region);
        assert_eq!(
            semantic.relationships[0].kind,
            SemanticRelationshipKind::Owns
        );
        assert_eq!(semantic.relationships[0].target, reference.content_node());
        assert!(reference.authorize_layer(ShellLayerKind::Panel).is_ok());
        assert!(reference.authorize_layer(ShellLayerKind::Lock).is_err());
    }
}

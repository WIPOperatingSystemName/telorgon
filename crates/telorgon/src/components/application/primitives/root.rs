//! Stable application semantic scope and caller-content host.

use std::fmt;

use crate::runtime::{RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    BoxStyle, ControlHandle, LayoutStyle, Property, SemanticName, SemanticNode,
    SemanticParticipation, SemanticRelationship, SemanticRelationshipKind, SemanticRole, UiNodeId,
};

/// Complete visual and layout inputs for an application scope and its content host.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ApplicationRootStyle {
    pub container: BoxStyle,
    pub content: BoxStyle,
    pub layout: LayoutStyle,
    pub content_layout: LayoutStyle,
}

/// One explicitly named application semantic scope.
#[derive(Clone, Debug, PartialEq)]
pub struct ApplicationRoot {
    label: String,
    style: ApplicationRootStyle,
}

impl ApplicationRoot {
    pub fn new(label: impl Into<String>) -> Result<Self, ApplicationRootError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(ApplicationRootError::MissingAccessibleName);
        }
        Ok(Self {
            label,
            style: ApplicationRootStyle::default(),
        })
    }

    pub const fn style(mut self, style: ApplicationRootStyle) -> Self {
        self.style = style;
        self
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub const fn root_style(&self) -> ApplicationRootStyle {
        self.style
    }

    /// Mounts an empty stable scope and content host. Callers mount regions or content under
    /// [`ApplicationRootRef::content_node`] after this returns.
    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
    ) -> RuntimeResult<ApplicationRootRef> {
        let root = ui
            .foundation()
            .container_node_under(host, self.style.container, self.style.layout, |_| {})
            .ok_or_else(|| RuntimeError::new("application root parent is stale"))?;
        let content = ui
            .foundation()
            .container_node_under(
                root.node,
                self.style.content,
                self.style.content_layout,
                |_| {},
            )
            .ok_or_else(|| RuntimeError::new("application root content parent is stale"))?;

        ui.foundation()
            .semantic_node(
                content.node,
                SemanticNode {
                    participation: SemanticParticipation::MergeDescendants,
                    ..SemanticNode::default()
                },
            )
            .map_err(|error| {
                RuntimeError::new(format!("invalid application content semantics: {error:?}"))
            })?;
        let name = ui.foundation().intern(&self.label);
        ui.foundation()
            .semantic_node(
                root.node,
                SemanticNode {
                    role: SemanticRole::Application,
                    name: SemanticName::Text(name),
                    relationships: vec![SemanticRelationship {
                        kind: SemanticRelationshipKind::Owns,
                        target: content.node,
                    }],
                    ..SemanticNode::default()
                },
            )
            .map_err(|error| {
                RuntimeError::new(format!("invalid application root semantics: {error:?}"))
            })?;

        Ok(ApplicationRootRef { root, content })
    }
}

/// Stable mounted application scope and its caller-content host.
#[derive(Clone, Copy, Debug)]
pub struct ApplicationRootRef {
    root: ControlHandle,
    content: ControlHandle,
}

impl ApplicationRootRef {
    pub const fn node(self) -> UiNodeId {
        self.root.node
    }

    pub const fn content_node(self) -> UiNodeId {
        self.content.node
    }

    pub const fn style(self) -> Property<BoxStyle> {
        self.root.style
    }

    pub const fn content_style(self) -> Property<BoxStyle> {
        self.content.style
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationRootError {
    MissingAccessibleName,
}

impl fmt::Display for ApplicationRootError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("application root accessible name is empty")
    }
}

impl std::error::Error for ApplicationRootError {}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use crate::runtime::{Component, CreateContext, UpdateContext, ViewRuntime};
    use crate::ui::{NodeKind, SemanticAction, UiRoot};

    use super::*;

    #[test]
    fn construction_requires_an_explicit_name() {
        assert_eq!(
            ApplicationRoot::new("  "),
            Err(ApplicationRootError::MissingAccessibleName)
        );
        assert_eq!(
            ApplicationRoot::new("Workspace").unwrap().label(),
            "Workspace"
        );
    }

    struct Fixture {
        reference: Rc<Cell<Option<ApplicationRootRef>>>,
        style: ApplicationRootStyle,
    }

    impl Component for Fixture {
        type State = ();
        type Action = ();

        fn create(&self, _: &mut CreateContext<'_>) -> Self::State {}

        fn mount(&self, _: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let host = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            let reference = ApplicationRoot::new("Workspace")
                .unwrap()
                .style(self.style)
                .mount(ui, host.0)
                .unwrap();
            self.reference.set(Some(reference));
            host
        }

        fn action(&self, _: &mut Self::State, _: Self::Action, _: &mut UpdateContext<'_, Self>) {}
    }

    #[test]
    fn mount_publishes_one_named_application_scope_and_stable_owned_content() {
        let reference = Rc::new(Cell::new(None));
        let style = ApplicationRootStyle {
            container: BoxStyle {
                opacity: 0.75,
                ..BoxStyle::default()
            },
            content: BoxStyle {
                opacity: 0.5,
                ..BoxStyle::default()
            },
            layout: LayoutStyle {
                gap: 4.0,
                ..LayoutStyle::default()
            },
            content_layout: LayoutStyle {
                gap: 5.0,
                ..LayoutStyle::default()
            },
        };
        let runtime = ViewRuntime::from_component(Fixture {
            reference: reference.clone(),
            style,
        })
        .unwrap();
        let reference = reference.get().unwrap();
        assert_ne!(reference.node(), reference.content_node());
        assert_eq!(
            runtime.ui().kinds.get(reference.node()),
            Some(&NodeKind::Box)
        );
        assert_eq!(
            runtime.ui().box_styles.get(reference.node()),
            Some(&style.container)
        );
        assert_eq!(
            runtime.ui().box_styles.get(reference.content_node()),
            Some(&style.content)
        );
        assert_eq!(
            runtime.ui().layouts.get(reference.node()),
            Some(&style.layout)
        );
        assert_eq!(
            runtime.ui().layouts.get(reference.content_node()),
            Some(&style.content_layout)
        );
        let semantic = runtime.ui().semantics.get(reference.node()).unwrap();
        assert_eq!(semantic.role, SemanticRole::Application);
        let SemanticName::Text(name) = semantic.name else {
            panic!("application root must have a text name");
        };
        assert_eq!(runtime.ui().string(name), Some("Workspace"));
        assert!(semantic.actions.is_empty());
        assert!(!semantic.state.focusable);
        assert_eq!(semantic.relationships.len(), 1);
        assert_eq!(semantic.relationships[0].target, reference.content_node());
        assert_eq!(
            semantic.relationships[0].kind,
            SemanticRelationshipKind::Owns
        );
        let content = runtime
            .ui()
            .semantics
            .get(reference.content_node())
            .unwrap();
        assert_eq!(
            content.participation,
            SemanticParticipation::MergeDescendants
        );
        assert!(!content.effective_actions().contains(SemanticAction::Focus));
    }
}

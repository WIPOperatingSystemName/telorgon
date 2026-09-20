//! Typed content, navigation, and status application landmarks.

use std::fmt;

use crate::runtime::{RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    BoxStyle, ControlHandle, LayoutStyle, Property, SemanticNode, SemanticRole, UiNodeId,
};

/// Application landmark kind independent of Scaffold slot policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ApplicationRegionKind {
    Content,
    Navigation,
    Status,
}

impl ApplicationRegionKind {
    pub const fn semantic_role(self) -> SemanticRole {
        match self {
            Self::Content => SemanticRole::Main,
            Self::Navigation => SemanticRole::Navigation,
            Self::Status => SemanticRole::Status,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ApplicationRegionStyle {
    pub container: BoxStyle,
    pub layout: LayoutStyle,
}

/// One named application landmark that serves as a stable caller-content host.
#[derive(Clone, Debug, PartialEq)]
pub struct ApplicationRegion {
    kind: ApplicationRegionKind,
    label: String,
    style: ApplicationRegionStyle,
}

impl ApplicationRegion {
    pub fn new(
        kind: ApplicationRegionKind,
        label: impl Into<String>,
    ) -> Result<Self, ApplicationRegionError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(ApplicationRegionError::MissingAccessibleName);
        }
        Ok(Self {
            kind,
            label,
            style: ApplicationRegionStyle::default(),
        })
    }

    pub fn content(label: impl Into<String>) -> Result<Self, ApplicationRegionError> {
        Self::new(ApplicationRegionKind::Content, label)
    }

    pub fn navigation(label: impl Into<String>) -> Result<Self, ApplicationRegionError> {
        Self::new(ApplicationRegionKind::Navigation, label)
    }

    pub fn status(label: impl Into<String>) -> Result<Self, ApplicationRegionError> {
        Self::new(ApplicationRegionKind::Status, label)
    }

    pub const fn style(mut self, style: ApplicationRegionStyle) -> Self {
        self.style = style;
        self
    }

    pub const fn kind(&self) -> ApplicationRegionKind {
        self.kind
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub const fn region_style(&self) -> ApplicationRegionStyle {
        self.style
    }

    /// Mounts an empty named landmark. Callers mount content under [`ApplicationRegionRef::node`].
    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
    ) -> RuntimeResult<ApplicationRegionRef> {
        let control = ui
            .foundation()
            .container_node_under(host, self.style.container, self.style.layout, |_| {})
            .ok_or_else(|| RuntimeError::new("application region parent is stale"))?;
        let name = ui.foundation().intern(&self.label);
        ui.foundation()
            .semantic_node(
                control.node,
                SemanticNode::named(self.kind.semantic_role(), name),
            )
            .map_err(|error| {
                RuntimeError::new(format!("invalid application region semantics: {error:?}"))
            })?;
        Ok(ApplicationRegionRef {
            kind: self.kind,
            control,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ApplicationRegionRef {
    kind: ApplicationRegionKind,
    control: ControlHandle,
}

impl ApplicationRegionRef {
    pub const fn kind(self) -> ApplicationRegionKind {
        self.kind
    }

    pub const fn node(self) -> UiNodeId {
        self.control.node
    }

    pub const fn style(self) -> Property<BoxStyle> {
        self.control.style
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationRegionError {
    MissingAccessibleName,
}

impl fmt::Display for ApplicationRegionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("application region accessible name is empty")
    }
}

impl std::error::Error for ApplicationRegionError {}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::runtime::{Component, CreateContext, UpdateContext, ViewRuntime};
    use crate::ui::{SemanticAction, SemanticName, UiRoot};

    use crate::components::application::primitives::ApplicationRoot;

    use super::*;

    #[test]
    fn constructors_are_typed_named_and_independent_of_slot_policy() {
        assert_eq!(
            ApplicationRegion::content(" "),
            Err(ApplicationRegionError::MissingAccessibleName)
        );
        for (region, kind, role) in [
            (
                ApplicationRegion::content("Document").unwrap(),
                ApplicationRegionKind::Content,
                SemanticRole::Main,
            ),
            (
                ApplicationRegion::navigation("Sections").unwrap(),
                ApplicationRegionKind::Navigation,
                SemanticRole::Navigation,
            ),
            (
                ApplicationRegion::status("Sync status").unwrap(),
                ApplicationRegionKind::Status,
                SemanticRole::Status,
            ),
        ] {
            assert_eq!(region.kind(), kind);
            assert_eq!(region.kind().semantic_role(), role);
        }
    }

    struct Fixture {
        references: Rc<RefCell<Vec<ApplicationRegionRef>>>,
    }

    impl Component for Fixture {
        type State = ();
        type Action = ();

        fn create(&self, _: &mut CreateContext<'_>) -> Self::State {}

        fn mount(&self, _: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let host = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            let root = ApplicationRoot::new("Workspace")
                .unwrap()
                .mount(ui, host.0)
                .unwrap();
            let style = ApplicationRegionStyle {
                container: BoxStyle {
                    opacity: 0.625,
                    ..BoxStyle::default()
                },
                layout: LayoutStyle {
                    gap: 7.0,
                    ..LayoutStyle::default()
                },
            };
            for region in [
                ApplicationRegion::navigation("Sections").unwrap(),
                ApplicationRegion::content("Document").unwrap(),
                ApplicationRegion::status("Sync status").unwrap(),
            ] {
                self.references
                    .borrow_mut()
                    .push(region.style(style).mount(ui, root.content_node()).unwrap());
            }
            host
        }

        fn action(&self, _: &mut Self::State, _: Self::Action, _: &mut UpdateContext<'_, Self>) {}
    }

    #[test]
    fn mount_preserves_style_and_publishes_action_free_landmarks() {
        let references = Rc::new(RefCell::new(Vec::new()));
        let runtime = ViewRuntime::from_component(Fixture {
            references: references.clone(),
        })
        .unwrap();
        let references = references.borrow();
        assert_eq!(references.len(), 3);
        for (reference, role, name) in [
            (references[0], SemanticRole::Navigation, "Sections"),
            (references[1], SemanticRole::Main, "Document"),
            (references[2], SemanticRole::Status, "Sync status"),
        ] {
            assert_eq!(
                runtime
                    .ui()
                    .box_styles
                    .get(reference.node())
                    .unwrap()
                    .opacity,
                0.625
            );
            assert_eq!(runtime.ui().layouts.get(reference.node()).unwrap().gap, 7.0);
            let semantic = runtime.ui().semantics.get(reference.node()).unwrap();
            assert_eq!(semantic.role, role);
            let SemanticName::Text(label) = semantic.name else {
                panic!("application region must have a text name");
            };
            assert_eq!(runtime.ui().string(label), Some(name));
            assert!(semantic.actions.is_empty());
            assert!(!semantic.state.focusable);
            assert!(!semantic.effective_actions().contains(SemanticAction::Focus));
        }
    }
}

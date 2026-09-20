//! Stable identity for revisioned, opaque host-owned render-target content.

use std::fmt;
use std::num::NonZeroU64;

use crate::runtime::{RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    BoxStyle, ControlHandle, LayoutStyle, Property, SemanticName, SemanticNode,
    SemanticParticipation, SemanticRole, UiNodeId,
};

/// Opaque host identity. It is not a GPU handle, image ID, or ownership transfer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RenderTargetToken(NonZeroU64);

impl RenderTargetToken {
    pub const fn new(raw: u64) -> Option<Self> {
        match NonZeroU64::new(raw) {
            Some(raw) => Some(Self(raw)),
            None => None,
        }
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// Host token plus the revision that changes when its visible content changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RenderTargetViewContent {
    target: RenderTargetToken,
    content_version: NonZeroU64,
}

impl RenderTargetViewContent {
    pub fn new(
        target: RenderTargetToken,
        content_version: u64,
    ) -> Result<Self, RenderTargetViewError> {
        let content_version =
            NonZeroU64::new(content_version).ok_or(RenderTargetViewError::ZeroContentVersion)?;
        Ok(Self {
            target,
            content_version,
        })
    }

    pub const fn target(self) -> RenderTargetToken {
        self.target
    }

    pub const fn content_version(self) -> u64 {
        self.content_version.get()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RenderTargetViewSemanticPolicy {
    #[default]
    Decorative,
    Described,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RenderTargetViewStyle {
    pub container: BoxStyle,
    pub layout: LayoutStyle,
}

/// Immutable mount configuration for one opaque host render target.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderTargetView {
    content: RenderTargetViewContent,
    accessible_description: Option<String>,
    style: RenderTargetViewStyle,
}

impl RenderTargetView {
    pub fn decorative(content: RenderTargetViewContent) -> Self {
        Self {
            content,
            accessible_description: None,
            style: RenderTargetViewStyle::default(),
        }
    }

    pub fn described(
        content: RenderTargetViewContent,
        accessible_description: impl Into<String>,
    ) -> Result<Self, RenderTargetViewError> {
        let accessible_description = accessible_description.into();
        if accessible_description.trim().is_empty() {
            return Err(RenderTargetViewError::MissingAccessibleDescription);
        }
        Ok(Self {
            content,
            accessible_description: Some(accessible_description),
            style: RenderTargetViewStyle::default(),
        })
    }

    pub const fn content(&self) -> RenderTargetViewContent {
        self.content
    }

    pub fn semantic_policy(&self) -> RenderTargetViewSemanticPolicy {
        if self.accessible_description.is_some() {
            RenderTargetViewSemanticPolicy::Described
        } else {
            RenderTargetViewSemanticPolicy::Decorative
        }
    }

    pub fn accessible_description(&self) -> Option<&str> {
        self.accessible_description.as_deref()
    }

    pub const fn style(mut self, style: RenderTargetViewStyle) -> Self {
        self.style = style;
        self
    }

    pub const fn view_style(&self) -> RenderTargetViewStyle {
        self.style
    }

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
    ) -> RuntimeResult<RenderTargetViewRef> {
        let control = ui
            .foundation()
            .container_node_under(host, self.style.container, self.style.layout, |_| {})
            .ok_or_else(|| RuntimeError::new("render-target view parent is stale"))?;
        let semantic = if let Some(description) = &self.accessible_description {
            let name = ui.foundation().intern(description);
            SemanticNode {
                role: SemanticRole::Image,
                name: SemanticName::Text(name),
                ..SemanticNode::default()
            }
        } else {
            SemanticNode {
                role: SemanticRole::Image,
                participation: SemanticParticipation::Exclude,
                ..SemanticNode::default()
            }
        };
        ui.foundation()
            .semantic_node(control.node, semantic)
            .map_err(|error| {
                RuntimeError::new(format!("invalid render-target semantics: {error:?}"))
            })?;
        Ok(RenderTargetViewRef {
            control,
            content: self.content,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RenderTargetViewRef {
    control: ControlHandle,
    content: RenderTargetViewContent,
}

impl RenderTargetViewRef {
    pub const fn node(self) -> UiNodeId {
        self.control.node
    }

    pub const fn style(self) -> Property<BoxStyle> {
        self.control.style
    }

    pub const fn content(self) -> RenderTargetViewContent {
        self.content
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderTargetViewError {
    ZeroContentVersion,
    MissingAccessibleDescription,
}

impl fmt::Display for RenderTargetViewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid render-target view: {self:?}")
    }
}

impl std::error::Error for RenderTargetViewError {}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::runtime::{Component, CreateContext, UpdateContext, ViewRuntime};
    use crate::ui::{NodeKind, SemanticActions, UiRoot};

    use super::*;

    fn content(raw: u64, version: u64) -> RenderTargetViewContent {
        RenderTargetViewContent::new(RenderTargetToken::new(raw).unwrap(), version).unwrap()
    }

    #[test]
    fn opaque_identity_revision_and_semantic_policy_are_explicit() {
        assert_eq!(RenderTargetToken::new(0), None);
        assert_eq!(
            RenderTargetViewContent::new(RenderTargetToken::new(4).unwrap(), 0),
            Err(RenderTargetViewError::ZeroContentVersion)
        );
        let decorative = RenderTargetView::decorative(content(4, 2));
        assert_eq!(
            decorative.semantic_policy(),
            RenderTargetViewSemanticPolicy::Decorative
        );
        assert_eq!(decorative.content().target().get(), 4);
        assert_eq!(decorative.content().content_version(), 2);
        assert_eq!(
            RenderTargetView::described(content(4, 2), " "),
            Err(RenderTargetViewError::MissingAccessibleDescription)
        );
    }

    struct Fixture {
        references: Rc<RefCell<Option<(RenderTargetViewRef, RenderTargetViewRef)>>>,
    }

    impl Component for Fixture {
        type State = ();
        type Action = ();

        fn create(&self, _: &mut CreateContext<'_>) -> Self::State {}

        fn mount(&self, _: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let host = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            let style = RenderTargetViewStyle {
                container: BoxStyle {
                    opacity: 0.65,
                    ..BoxStyle::default()
                },
                layout: LayoutStyle {
                    gap: 6.0,
                    ..LayoutStyle::default()
                },
            };
            let decorative = RenderTargetView::decorative(content(8, 12))
                .style(style)
                .mount(ui, host.0)
                .unwrap();
            let described = RenderTargetView::described(content(9, 13), "Editor viewport")
                .unwrap()
                .mount(ui, host.0)
                .unwrap();
            *self.references.borrow_mut() = Some((decorative, described));
            host
        }

        fn action(&self, _: &mut Self::State, _: Self::Action, _: &mut UpdateContext<'_, Self>) {}
    }

    #[test]
    fn mount_retains_host_content_without_fabricating_an_image_or_input_route() {
        let references = Rc::new(RefCell::new(None));
        let runtime = ViewRuntime::from_component(Fixture {
            references: references.clone(),
        })
        .unwrap();
        let (decorative, described) = references.borrow().unwrap();
        assert_eq!(
            runtime.ui().kinds.get(decorative.node()),
            Some(&NodeKind::Box)
        );
        assert!(runtime.ui().images.get(decorative.node()).is_none());
        assert_eq!(decorative.content(), content(8, 12));
        assert_eq!(
            runtime
                .ui()
                .box_styles
                .get(decorative.node())
                .unwrap()
                .opacity,
            0.65
        );
        assert_eq!(
            runtime.ui().layouts.get(decorative.node()).unwrap().gap,
            6.0
        );
        let decorative_semantic = runtime.ui().semantics.get(decorative.node()).unwrap();
        assert_eq!(
            decorative_semantic.participation,
            SemanticParticipation::Exclude
        );
        assert_eq!(decorative_semantic.actions, SemanticActions::NONE);
        let described_semantic = runtime.ui().semantics.get(described.node()).unwrap();
        assert_eq!(described_semantic.role, SemanticRole::Image);
        assert!(matches!(described_semantic.name, SemanticName::Text(_)));
        for node in [decorative.node(), described.node()] {
            assert!(
                runtime
                    .ui()
                    .interactions
                    .get(node)
                    .is_none_or(|interaction| {
                        !interaction.focusable && interaction.listener_mask == 0
                    })
            );
        }
    }
}

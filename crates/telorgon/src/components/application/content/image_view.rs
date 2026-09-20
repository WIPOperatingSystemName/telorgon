//! Stable decorative or described retained image content.

use std::fmt;

use crate::runtime::{RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    BoxStyle, ControlHandle, ImageId, LayoutStyle, Property, SemanticName, SemanticNode,
    SemanticParticipation, SemanticRole, UiNodeId,
};

/// Caller-owned retained image identity and content revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImageViewContent {
    image: ImageId,
    content_version: u64,
}

impl ImageViewContent {
    pub const fn new(image: ImageId, content_version: u64) -> Self {
        Self {
            image,
            content_version,
        }
    }

    pub const fn image(self) -> ImageId {
        self.image
    }

    pub const fn content_version(self) -> u64 {
        self.content_version
    }
}

impl From<ImageId> for ImageViewContent {
    fn from(image: ImageId) -> Self {
        Self::new(image, 1)
    }
}

/// Whether an image is absent from semantics or described for assistive consumers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ImageViewSemanticPolicy {
    #[default]
    Decorative,
    Described,
}

/// Caller-owned image visual and layout inputs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ImageViewStyle {
    pub image: BoxStyle,
    pub layout: LayoutStyle,
}

/// Immutable mount configuration for one retained application image.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageView {
    content: ImageViewContent,
    accessible_description: Option<String>,
    style: ImageViewStyle,
}

impl ImageView {
    pub fn decorative(content: impl Into<ImageViewContent>) -> Self {
        Self {
            content: content.into(),
            accessible_description: None,
            style: ImageViewStyle::default(),
        }
    }

    pub fn described(
        content: impl Into<ImageViewContent>,
        accessible_description: impl Into<String>,
    ) -> Result<Self, ImageViewError> {
        let accessible_description = accessible_description.into();
        if accessible_description.trim().is_empty() {
            return Err(ImageViewError::MissingAccessibleDescription);
        }
        Ok(Self {
            content: content.into(),
            accessible_description: Some(accessible_description),
            style: ImageViewStyle::default(),
        })
    }

    pub const fn content(&self) -> ImageViewContent {
        self.content
    }

    pub fn semantic_policy(&self) -> ImageViewSemanticPolicy {
        if self.accessible_description.is_some() {
            ImageViewSemanticPolicy::Described
        } else {
            ImageViewSemanticPolicy::Decorative
        }
    }

    pub fn accessible_description(&self) -> Option<&str> {
        self.accessible_description.as_deref()
    }

    pub const fn style(mut self, style: ImageViewStyle) -> Self {
        self.style = style;
        self
    }

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
    ) -> RuntimeResult<ImageViewRef> {
        let control = ui
            .foundation()
            .image_node_under(
                host,
                self.content.image(),
                self.content.content_version(),
                self.style.image,
                self.style.layout,
            )
            .ok_or_else(|| RuntimeError::new("application image-view host is stale"))?;
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
            .map_err(semantic_runtime_error)?;
        Ok(ImageViewRef {
            control,
            content: self.content,
        })
    }
}

fn semantic_runtime_error(error: crate::ui::SemanticError) -> RuntimeError {
    RuntimeError::new(format!("invalid image-view semantics: {error:?}"))
}

/// Stable mounted identity and style property for one image view.
#[derive(Clone, Copy, Debug)]
pub struct ImageViewRef {
    control: ControlHandle,
    content: ImageViewContent,
}

impl ImageViewRef {
    pub const fn node(self) -> UiNodeId {
        self.control.node
    }

    pub const fn content(self) -> ImageViewContent {
        self.content
    }

    pub const fn style(self) -> Property<BoxStyle> {
        self.control.style
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageViewError {
    MissingAccessibleDescription,
}

impl fmt::Display for ImageViewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid image view: {self:?}")
    }
}

impl std::error::Error for ImageViewError {}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::foundation::{ColorRgba8, PointF};
    use crate::runtime::{Component, CreateContext, UpdateContext, ViewRuntime};
    use crate::ui::{
        Background, Border, CornerRadii, Flow, NodeKind, SemanticActions, SemanticValue, SizeRule,
        UiRoot,
    };

    use super::*;

    #[test]
    fn constructors_keep_content_version_and_semantic_policy_explicit() {
        let content = ImageViewContent::new(ImageId(41), 9);
        assert_eq!(content.image(), ImageId(41));
        assert_eq!(content.content_version(), 9);

        let decorative = ImageView::decorative(content);
        assert_eq!(
            decorative.semantic_policy(),
            ImageViewSemanticPolicy::Decorative
        );
        assert_eq!(decorative.accessible_description(), None);
        assert_eq!(decorative.content(), content);

        let described = ImageView::described(content, "Quarterly revenue chart").unwrap();
        assert_eq!(
            described.semantic_policy(),
            ImageViewSemanticPolicy::Described
        );
        assert_eq!(
            described.accessible_description(),
            Some("Quarterly revenue chart")
        );
        assert_eq!(
            ImageView::described(content, " "),
            Err(ImageViewError::MissingAccessibleDescription)
        );
        assert_eq!(ImageViewContent::from(ImageId(7)).content_version(), 1);
    }

    struct Fixture {
        references: Rc<RefCell<Option<(ImageViewRef, ImageViewRef)>>>,
    }

    impl Component for Fixture {
        type State = ();
        type Action = ();

        fn create(&self, _: &mut CreateContext<'_>) -> Self::State {}

        fn mount(&self, _: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            let style = ImageViewStyle {
                image: BoxStyle {
                    width: SizeRule::Logical(320.0),
                    height: SizeRule::Logical(180.0),
                    decoration: crate::ui::BoxDecoration {
                        background: Background::Color(ColorRgba8::rgba(10, 20, 30, 255)),
                        border: Border::all(2.0, ColorRgba8::rgba(40, 50, 60, 255)),
                        corner_radii: CornerRadii::all(8.0),
                        ..crate::ui::BoxDecoration::default()
                    },
                    opacity: 0.75,
                    ..BoxStyle::default()
                },
                layout: LayoutStyle {
                    flow: Flow::Overlay,
                    gap: 3.0,
                    contain: true,
                    scroll_offset: PointF { x: 4.0, y: 5.0 },
                    ..LayoutStyle::default()
                },
            };
            let decorative = ImageView::decorative(ImageViewContent::new(ImageId(17), 44))
                .style(style)
                .mount(ui, root.0)
                .unwrap();
            let described = ImageView::described(
                ImageViewContent::new(ImageId(23), 91),
                "A mountain reflected in a lake",
            )
            .unwrap()
            .mount(ui, root.0)
            .unwrap();
            *self.references.borrow_mut() = Some((decorative, described));
            root
        }

        fn action(&self, _: &mut Self::State, _: Self::Action, _: &mut UpdateContext<'_, Self>) {}
    }

    #[test]
    fn mount_preserves_visual_layout_content_and_semantic_exclusion() {
        let references = Rc::new(RefCell::new(None));
        let runtime = ViewRuntime::from_component(Fixture {
            references: references.clone(),
        })
        .unwrap();
        let (decorative, described) = references.borrow().expect("image-view references");

        assert_ne!(decorative.node(), described.node());
        assert_eq!(
            runtime.ui().kinds.get(decorative.node()),
            Some(&NodeKind::Image)
        );
        assert_eq!(
            runtime.ui().kinds.get(described.node()),
            Some(&NodeKind::Image)
        );
        assert_eq!(
            runtime.ui().images.get(decorative.node()).unwrap(),
            &crate::ui::ImageVisual {
                image: ImageId(17),
                tint: None,
                content_version: 44,
            }
        );
        assert_eq!(decorative.content(), ImageViewContent::new(ImageId(17), 44));

        let style = runtime.ui().box_styles.get(decorative.node()).unwrap();
        assert_eq!(style.width, SizeRule::Logical(320.0));
        assert_eq!(style.height, SizeRule::Logical(180.0));
        assert_eq!(style.opacity, 0.75);
        assert_eq!(style.decoration.corner_radii, CornerRadii::all(8.0));
        assert_eq!(
            runtime.ui().layouts.get(decorative.node()).unwrap(),
            &LayoutStyle {
                flow: Flow::Overlay,
                gap: 3.0,
                contain: true,
                scroll_offset: PointF { x: 4.0, y: 5.0 },
                ..LayoutStyle::default()
            }
        );

        let decorative_semantics = runtime.ui().semantics.get(decorative.node()).unwrap();
        assert_eq!(decorative_semantics.role, SemanticRole::Image);
        assert_eq!(
            decorative_semantics.participation,
            SemanticParticipation::Exclude
        );
        assert_eq!(decorative_semantics.actions, SemanticActions::NONE);

        let described_visual = runtime.ui().images.get(described.node()).unwrap();
        assert_eq!(described_visual.image, ImageId(23));
        assert_eq!(described_visual.content_version, 91);
        let described_semantics = runtime.ui().semantics.get(described.node()).unwrap();
        assert_eq!(described_semantics.role, SemanticRole::Image);
        assert!(matches!(described_semantics.name, SemanticName::Text(_)));
        assert_eq!(
            described_semantics.participation,
            SemanticParticipation::Node
        );
        assert!(!described_semantics.state.focusable);
        assert_eq!(described_semantics.value, SemanticValue::None);
        assert_eq!(described_semantics.actions, SemanticActions::NONE);

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

//! Revisioned opaque host video-surface content and presentation metadata.

use std::fmt;
use std::num::NonZeroU64;

use crate::foundation::SizeI;
use crate::runtime::{RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    BoxStyle, ControlHandle, LayoutStyle, Property, SemanticName, SemanticNode,
    SemanticParticipation, SemanticRole, UiNodeId,
};

/// Opaque host media identity; not a decoder, native handle, or ownership transfer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VideoSurfaceToken(NonZeroU64);

impl VideoSurfaceToken {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VideoFit {
    Contain,
    Cover,
    Fill,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VideoColorPrimaries {
    Srgb,
    DisplayP3,
    Bt709,
    Bt2020,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VideoTransferFunction {
    Srgb,
    Linear,
    Bt709,
    Pq,
    Hlg,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VideoColorRange {
    Full,
    Limited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VideoColorMetadata {
    pub primaries: VideoColorPrimaries,
    pub transfer: VideoTransferFunction,
    pub range: VideoColorRange,
}

impl Default for VideoColorMetadata {
    fn default() -> Self {
        Self {
            primaries: VideoColorPrimaries::Bt709,
            transfer: VideoTransferFunction::Bt709,
            range: VideoColorRange::Limited,
        }
    }
}

/// Host assertion only; enforcement remains an explicit host/backend responsibility.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum VideoProtection {
    #[default]
    Unprotected,
    Protected,
}

/// Complete revisioned host media input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoSurfaceContent {
    surface: VideoSurfaceToken,
    content_version: NonZeroU64,
    frame_size: SizeI,
    color: VideoColorMetadata,
    protection: VideoProtection,
}

impl VideoSurfaceContent {
    pub fn new(
        surface: VideoSurfaceToken,
        content_version: u64,
        frame_size: SizeI,
        color: VideoColorMetadata,
        protection: VideoProtection,
    ) -> Result<Self, VideoSurfaceError> {
        let content_version =
            NonZeroU64::new(content_version).ok_or(VideoSurfaceError::ZeroContentVersion)?;
        if frame_size.width <= 0 || frame_size.height <= 0 {
            return Err(VideoSurfaceError::InvalidFrameSize);
        }
        Ok(Self {
            surface,
            content_version,
            frame_size,
            color,
            protection,
        })
    }

    pub const fn surface(self) -> VideoSurfaceToken {
        self.surface
    }

    pub const fn content_version(self) -> u64 {
        self.content_version.get()
    }

    pub const fn frame_size(self) -> SizeI {
        self.frame_size
    }

    pub const fn color(self) -> VideoColorMetadata {
        self.color
    }

    pub const fn protection(self) -> VideoProtection {
        self.protection
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum VideoSurfaceSemanticPolicy {
    #[default]
    Decorative,
    Described,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VideoSurfaceStyle {
    pub container: BoxStyle,
    pub layout: LayoutStyle,
}

/// Immutable mount snapshot for one host media surface.
#[derive(Clone, Debug, PartialEq)]
pub struct VideoSurface {
    content: VideoSurfaceContent,
    fit: VideoFit,
    accessible_description: Option<String>,
    style: VideoSurfaceStyle,
}

impl VideoSurface {
    pub fn decorative(content: VideoSurfaceContent, fit: VideoFit) -> Self {
        Self {
            content,
            fit,
            accessible_description: None,
            style: VideoSurfaceStyle::default(),
        }
    }

    pub fn described(
        content: VideoSurfaceContent,
        fit: VideoFit,
        accessible_description: impl Into<String>,
    ) -> Result<Self, VideoSurfaceError> {
        let accessible_description = accessible_description.into();
        if accessible_description.trim().is_empty() {
            return Err(VideoSurfaceError::MissingAccessibleDescription);
        }
        Ok(Self {
            content,
            fit,
            accessible_description: Some(accessible_description),
            style: VideoSurfaceStyle::default(),
        })
    }

    pub const fn content(&self) -> VideoSurfaceContent {
        self.content
    }

    pub const fn fit(&self) -> VideoFit {
        self.fit
    }

    pub fn semantic_policy(&self) -> VideoSurfaceSemanticPolicy {
        if self.accessible_description.is_some() {
            VideoSurfaceSemanticPolicy::Described
        } else {
            VideoSurfaceSemanticPolicy::Decorative
        }
    }

    pub fn accessible_description(&self) -> Option<&str> {
        self.accessible_description.as_deref()
    }

    pub const fn style(mut self, style: VideoSurfaceStyle) -> Self {
        self.style = style;
        self
    }

    pub const fn surface_style(&self) -> VideoSurfaceStyle {
        self.style
    }

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
    ) -> RuntimeResult<VideoSurfaceRef> {
        let control = ui
            .foundation()
            .container_node_under(host, self.style.container, self.style.layout, |_| {})
            .ok_or_else(|| RuntimeError::new("video surface parent is stale"))?;
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
                RuntimeError::new(format!("invalid video-surface semantics: {error:?}"))
            })?;
        Ok(VideoSurfaceRef {
            control,
            content: self.content,
            fit: self.fit,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct VideoSurfaceRef {
    control: ControlHandle,
    content: VideoSurfaceContent,
    fit: VideoFit,
}

impl VideoSurfaceRef {
    pub const fn node(self) -> UiNodeId {
        self.control.node
    }

    pub const fn style(self) -> Property<BoxStyle> {
        self.control.style
    }

    pub const fn content(self) -> VideoSurfaceContent {
        self.content
    }

    pub const fn fit(self) -> VideoFit {
        self.fit
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoSurfaceError {
    ZeroContentVersion,
    InvalidFrameSize,
    MissingAccessibleDescription,
}

impl fmt::Display for VideoSurfaceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid video surface: {self:?}")
    }
}

impl std::error::Error for VideoSurfaceError {}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::runtime::{Component, CreateContext, UpdateContext, ViewRuntime};
    use crate::ui::{NodeKind, SemanticActions, UiRoot};

    use super::*;

    fn content(protection: VideoProtection) -> VideoSurfaceContent {
        VideoSurfaceContent::new(
            VideoSurfaceToken::new(12).unwrap(),
            7,
            SizeI {
                width: 1920,
                height: 1080,
            },
            VideoColorMetadata::default(),
            protection,
        )
        .unwrap()
    }

    #[test]
    fn host_content_validates_revision_and_frame_size_and_retains_metadata() {
        assert_eq!(VideoSurfaceToken::new(0), None);
        assert_eq!(
            VideoSurfaceContent::new(
                VideoSurfaceToken::new(2).unwrap(),
                0,
                SizeI {
                    width: 1,
                    height: 1,
                },
                VideoColorMetadata::default(),
                VideoProtection::Unprotected,
            ),
            Err(VideoSurfaceError::ZeroContentVersion)
        );
        assert_eq!(
            VideoSurfaceContent::new(
                VideoSurfaceToken::new(2).unwrap(),
                1,
                SizeI {
                    width: -1,
                    height: 1,
                },
                VideoColorMetadata::default(),
                VideoProtection::Unprotected,
            ),
            Err(VideoSurfaceError::InvalidFrameSize)
        );
        let protected = content(VideoProtection::Protected);
        assert_eq!(protected.protection(), VideoProtection::Protected);
        assert_eq!(protected.frame_size().width, 1920);
        assert_eq!(protected.color(), VideoColorMetadata::default());
    }

    struct Fixture {
        references: Rc<RefCell<Option<(VideoSurfaceRef, VideoSurfaceRef)>>>,
    }

    impl Component for Fixture {
        type State = ();
        type Action = ();

        fn create(&self, _: &mut CreateContext<'_>) -> Self::State {}

        fn mount(&self, _: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let host = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            let style = VideoSurfaceStyle {
                container: BoxStyle {
                    opacity: 0.72,
                    ..BoxStyle::default()
                },
                layout: LayoutStyle {
                    gap: 5.0,
                    ..LayoutStyle::default()
                },
            };
            let decorative =
                VideoSurface::decorative(content(VideoProtection::Protected), VideoFit::Cover)
                    .style(style)
                    .mount(ui, host.0)
                    .unwrap();
            let described = VideoSurface::described(
                content(VideoProtection::Unprotected),
                VideoFit::Contain,
                "Conference camera",
            )
            .unwrap()
            .mount(ui, host.0)
            .unwrap();
            *self.references.borrow_mut() = Some((decorative, described));
            host
        }

        fn action(&self, _: &mut Self::State, _: Self::Action, _: &mut UpdateContext<'_, Self>) {}
    }

    #[test]
    fn mount_retains_metadata_without_claiming_decoding_import_or_protection_enforcement() {
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
        assert_eq!(
            decorative.content().protection(),
            VideoProtection::Protected
        );
        assert_eq!(decorative.fit(), VideoFit::Cover);
        assert_eq!(
            runtime
                .ui()
                .box_styles
                .get(decorative.node())
                .unwrap()
                .opacity,
            0.72
        );
        assert_eq!(
            runtime.ui().layouts.get(decorative.node()).unwrap().gap,
            5.0
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

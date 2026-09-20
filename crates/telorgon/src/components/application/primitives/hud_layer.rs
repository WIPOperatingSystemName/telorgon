//! Application/game HUD coordinate layer with explicit pointer and semantic policy.

use std::fmt;

use crate::foundation::{PointF, SizeF};
use crate::runtime::{RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    BoxStyle, ControlHandle, LayoutStyle, Property, SemanticNode, SemanticParticipation, UiNodeId,
};

/// Coordinate system used by caller-mounted HUD content.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HudCoordinateSpace {
    /// Coordinates already use the host view's logical units.
    HostLogical,
    /// Coordinates use a fixed reference extent and resolve independently along each host axis.
    Reference(SizeF),
}

impl HudCoordinateSpace {
    pub fn resolve_point(self, point: PointF, viewport: SizeF) -> Result<PointF, HudLayerError> {
        validate_point(point)?;
        validate_size(viewport, HudLayerError::InvalidViewportSize)?;
        let resolved = match self {
            Self::HostLogical => point,
            Self::Reference(reference) => {
                validate_size(reference, HudLayerError::InvalidReferenceSize)?;
                PointF {
                    x: point.x * viewport.width / reference.width,
                    y: point.y * viewport.height / reference.height,
                }
            }
        };
        validate_point(resolved)?;
        Ok(resolved)
    }
}

/// Whether the host may consider this layer while resolving pointer targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HudHitTestPolicy {
    PassThrough,
    Content,
}

impl HudHitTestPolicy {
    pub const fn accepts_content_hits(self) -> bool {
        matches!(self, Self::Content)
    }
}

/// Semantic-tree participation independent of pointer hit behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HudSemanticPolicy {
    IncludeContent,
    Exclude,
}

impl HudSemanticPolicy {
    const fn participation(self) -> SemanticParticipation {
        match self {
            Self::IncludeContent => SemanticParticipation::MergeDescendants,
            Self::Exclude => SemanticParticipation::Exclude,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HudLayerStyle {
    pub container: BoxStyle,
    pub layout: LayoutStyle,
}

/// Immutable mount policy for one caller-populated HUD layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HudLayer {
    coordinates: HudCoordinateSpace,
    hit_test: HudHitTestPolicy,
    semantics: HudSemanticPolicy,
    style: HudLayerStyle,
}

impl HudLayer {
    pub fn new(
        coordinates: HudCoordinateSpace,
        hit_test: HudHitTestPolicy,
        semantics: HudSemanticPolicy,
    ) -> Result<Self, HudLayerError> {
        if let HudCoordinateSpace::Reference(size) = coordinates {
            validate_size(size, HudLayerError::InvalidReferenceSize)?;
        }
        Ok(Self {
            coordinates,
            hit_test,
            semantics,
            style: HudLayerStyle::default(),
        })
    }

    pub const fn style(mut self, style: HudLayerStyle) -> Self {
        self.style = style;
        self
    }

    pub const fn coordinate_space(&self) -> HudCoordinateSpace {
        self.coordinates
    }

    pub const fn hit_test_policy(&self) -> HudHitTestPolicy {
        self.hit_test
    }

    pub const fn semantic_policy(&self) -> HudSemanticPolicy {
        self.semantics
    }

    pub const fn layer_style(&self) -> HudLayerStyle {
        self.style
    }

    /// Mounts an empty stable host. The returned hit policy is consumed by the host's hit tester;
    /// this primitive does not install an input route or capture pointer input itself.
    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
    ) -> RuntimeResult<HudLayerRef> {
        let control = ui
            .foundation()
            .container_node_under(host, self.style.container, self.style.layout, |_| {})
            .ok_or_else(|| RuntimeError::new("HUD layer parent is stale"))?;
        ui.foundation()
            .semantic_node(
                control.node,
                SemanticNode {
                    participation: self.semantics.participation(),
                    ..SemanticNode::default()
                },
            )
            .map_err(|error| RuntimeError::new(format!("invalid HUD semantics: {error:?}")))?;
        Ok(HudLayerRef {
            control,
            coordinates: self.coordinates,
            hit_test: self.hit_test,
            semantics: self.semantics,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct HudLayerRef {
    control: ControlHandle,
    coordinates: HudCoordinateSpace,
    hit_test: HudHitTestPolicy,
    semantics: HudSemanticPolicy,
}

impl HudLayerRef {
    pub const fn node(self) -> UiNodeId {
        self.control.node
    }

    pub const fn style(self) -> Property<BoxStyle> {
        self.control.style
    }

    pub const fn coordinate_space(self) -> HudCoordinateSpace {
        self.coordinates
    }

    pub const fn hit_test_policy(self) -> HudHitTestPolicy {
        self.hit_test
    }

    pub const fn semantic_policy(self) -> HudSemanticPolicy {
        self.semantics
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HudLayerError {
    InvalidReferenceSize,
    InvalidViewportSize,
    NonFinitePoint,
}

impl fmt::Display for HudLayerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid HUD layer input: {self:?}")
    }
}

impl std::error::Error for HudLayerError {}

fn validate_size(size: SizeF, error: HudLayerError) -> Result<(), HudLayerError> {
    if !size.width.is_finite()
        || !size.height.is_finite()
        || size.width <= 0.0
        || size.height <= 0.0
    {
        return Err(error);
    }
    Ok(())
}

fn validate_point(point: PointF) -> Result<(), HudLayerError> {
    if !point.x.is_finite() || !point.y.is_finite() {
        return Err(HudLayerError::NonFinitePoint);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use crate::runtime::{Component, CreateContext, UpdateContext, ViewRuntime};
    use crate::ui::{SemanticParticipation, UiRoot};

    use super::*;

    #[test]
    fn coordinate_resolution_is_explicit_validated_and_renderer_free() {
        assert_eq!(
            HudLayer::new(
                HudCoordinateSpace::Reference(SizeF {
                    width: 0.0,
                    height: 1080.0,
                }),
                HudHitTestPolicy::PassThrough,
                HudSemanticPolicy::Exclude,
            ),
            Err(HudLayerError::InvalidReferenceSize)
        );
        let coordinates = HudCoordinateSpace::Reference(SizeF {
            width: 1920.0,
            height: 1080.0,
        });
        assert_eq!(
            coordinates
                .resolve_point(
                    PointF { x: 960.0, y: 540.0 },
                    SizeF {
                        width: 1280.0,
                        height: 720.0,
                    },
                )
                .unwrap(),
            PointF { x: 640.0, y: 360.0 }
        );
        assert_eq!(
            coordinates.resolve_point(
                PointF {
                    x: f32::NAN,
                    y: 0.0
                },
                SizeF {
                    width: 1280.0,
                    height: 720.0,
                },
            ),
            Err(HudLayerError::NonFinitePoint)
        );
    }

    struct Fixture {
        excluded: Rc<Cell<Option<HudLayerRef>>>,
        included: Rc<Cell<Option<HudLayerRef>>>,
    }

    impl Component for Fixture {
        type State = ();
        type Action = ();

        fn create(&self, _: &mut CreateContext<'_>) -> Self::State {}

        fn mount(&self, _: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let host = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            self.excluded.set(Some(
                HudLayer::new(
                    HudCoordinateSpace::HostLogical,
                    HudHitTestPolicy::PassThrough,
                    HudSemanticPolicy::Exclude,
                )
                .unwrap()
                .mount(ui, host.0)
                .unwrap(),
            ));
            self.included.set(Some(
                HudLayer::new(
                    HudCoordinateSpace::HostLogical,
                    HudHitTestPolicy::Content,
                    HudSemanticPolicy::IncludeContent,
                )
                .unwrap()
                .style(HudLayerStyle {
                    container: BoxStyle {
                        opacity: 0.7,
                        ..BoxStyle::default()
                    },
                    layout: LayoutStyle {
                        gap: 3.0,
                        ..LayoutStyle::default()
                    },
                })
                .mount(ui, host.0)
                .unwrap(),
            ));
            host
        }

        fn action(&self, _: &mut Self::State, _: Self::Action, _: &mut UpdateContext<'_, Self>) {}
    }

    #[test]
    fn mount_keeps_hit_and_semantic_policy_independent_without_routes() {
        let excluded = Rc::new(Cell::new(None));
        let included = Rc::new(Cell::new(None));
        let runtime = ViewRuntime::from_component(Fixture {
            excluded: excluded.clone(),
            included: included.clone(),
        })
        .unwrap();
        let excluded = excluded.get().unwrap();
        let included = included.get().unwrap();
        assert_eq!(
            runtime
                .ui()
                .semantics
                .get(excluded.node())
                .unwrap()
                .participation,
            SemanticParticipation::Exclude
        );
        assert_eq!(
            runtime
                .ui()
                .semantics
                .get(included.node())
                .unwrap()
                .participation,
            SemanticParticipation::MergeDescendants
        );
        assert!(!excluded.hit_test_policy().accepts_content_hits());
        assert!(included.hit_test_policy().accepts_content_hits());
        assert_eq!(
            runtime
                .ui()
                .box_styles
                .get(included.node())
                .unwrap()
                .opacity,
            0.7
        );
        assert_eq!(runtime.ui().layouts.get(included.node()).unwrap().gap, 3.0);
        assert_eq!(
            runtime
                .ui()
                .interactions
                .get(included.node())
                .map_or(0, |interaction| interaction.listener_mask),
            0
        );
    }
}

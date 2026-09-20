//! Caller content positioned relative to a host-supplied viewport rectangle.

use std::fmt;

use crate::foundation::{PointF, RectF, Transform2D};
use crate::runtime::{RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    BoxStyle, ControlHandle, LayoutStyle, Property, SemanticNode, SemanticParticipation, UiNodeId,
};

/// Validated host viewport, normalized anchor, and logical offset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportOverlayPlacement {
    viewport: RectF,
    normalized_anchor: PointF,
    offset: PointF,
    resolved_anchor: PointF,
}

impl ViewportOverlayPlacement {
    pub fn new(
        viewport: RectF,
        normalized_anchor: PointF,
        offset: PointF,
    ) -> Result<Self, ViewportOverlayPlacementError> {
        if !viewport.x.is_finite()
            || !viewport.y.is_finite()
            || !viewport.width.is_finite()
            || !viewport.height.is_finite()
        {
            return Err(ViewportOverlayPlacementError::NonFiniteViewport);
        }
        if viewport.width <= 0.0 || viewport.height <= 0.0 {
            return Err(ViewportOverlayPlacementError::NonPositiveViewport);
        }
        if !normalized_anchor.x.is_finite() || !normalized_anchor.y.is_finite() {
            return Err(ViewportOverlayPlacementError::NonFiniteAnchor);
        }
        if !(0.0..=1.0).contains(&normalized_anchor.x)
            || !(0.0..=1.0).contains(&normalized_anchor.y)
        {
            return Err(ViewportOverlayPlacementError::AnchorOutOfBounds);
        }
        if !offset.x.is_finite() || !offset.y.is_finite() {
            return Err(ViewportOverlayPlacementError::NonFiniteOffset);
        }
        let resolved_anchor = PointF {
            x: viewport.x + viewport.width * normalized_anchor.x + offset.x,
            y: viewport.y + viewport.height * normalized_anchor.y + offset.y,
        };
        if !resolved_anchor.x.is_finite() || !resolved_anchor.y.is_finite() {
            return Err(ViewportOverlayPlacementError::NonFiniteResolvedAnchor);
        }
        Ok(Self {
            viewport,
            normalized_anchor,
            offset,
            resolved_anchor,
        })
    }

    pub const fn viewport(self) -> RectF {
        self.viewport
    }

    pub const fn normalized_anchor(self) -> PointF {
        self.normalized_anchor
    }

    pub const fn offset(self) -> PointF {
        self.offset
    }

    pub const fn resolved_anchor(self) -> PointF {
        self.resolved_anchor
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewportOverlayStyle {
    pub content: BoxStyle,
    pub content_layout: LayoutStyle,
}

/// Immutable viewport-relative positioning primitive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportOverlay {
    placement: ViewportOverlayPlacement,
    style: ViewportOverlayStyle,
}

impl ViewportOverlay {
    pub fn new(placement: ViewportOverlayPlacement) -> Self {
        Self {
            placement,
            style: ViewportOverlayStyle {
                content: BoxStyle::default(),
                content_layout: LayoutStyle::default(),
            },
        }
    }

    pub const fn style(mut self, style: ViewportOverlayStyle) -> Self {
        self.style = style;
        self
    }

    pub const fn placement(&self) -> ViewportOverlayPlacement {
        self.placement
    }

    pub const fn overlay_style(&self) -> ViewportOverlayStyle {
        self.style
    }

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
    ) -> RuntimeResult<ViewportOverlayRef> {
        let position_style = BoxStyle {
            transform: Transform2D {
                translation: self.placement.resolved_anchor,
                ..Transform2D::default()
            },
            ..BoxStyle::default()
        };
        let position = ui
            .foundation()
            .container_node_under(host, position_style, LayoutStyle::default(), |_| {})
            .ok_or_else(|| RuntimeError::new("viewport overlay parent is stale"))?;
        let content = ui
            .foundation()
            .container_node_under(
                position.node,
                self.style.content,
                self.style.content_layout,
                |_| {},
            )
            .ok_or_else(|| RuntimeError::new("viewport overlay content parent is stale"))?;
        ui.foundation()
            .semantic_node(
                position.node,
                SemanticNode {
                    participation: SemanticParticipation::MergeDescendants,
                    ..SemanticNode::default()
                },
            )
            .map_err(|error| {
                RuntimeError::new(format!("invalid viewport overlay semantics: {error:?}"))
            })?;
        Ok(ViewportOverlayRef {
            position,
            content,
            placement: self.placement,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ViewportOverlayRef {
    position: ControlHandle,
    content: ControlHandle,
    placement: ViewportOverlayPlacement,
}

impl ViewportOverlayRef {
    pub const fn node(self) -> UiNodeId {
        self.position.node
    }

    pub const fn content_node(self) -> UiNodeId {
        self.content.node
    }

    pub const fn position_style(self) -> Property<BoxStyle> {
        self.position.style
    }

    pub const fn content_style(self) -> Property<BoxStyle> {
        self.content.style
    }

    pub const fn placement(self) -> ViewportOverlayPlacement {
        self.placement
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewportOverlayPlacementError {
    NonFiniteViewport,
    NonPositiveViewport,
    NonFiniteAnchor,
    AnchorOutOfBounds,
    NonFiniteOffset,
    NonFiniteResolvedAnchor,
}

impl fmt::Display for ViewportOverlayPlacementError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid viewport overlay placement: {self:?}")
    }
}

impl std::error::Error for ViewportOverlayPlacementError {}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use crate::runtime::{Component, CreateContext, UpdateContext, ViewRuntime};
    use crate::ui::{SemanticParticipation, UiRoot};

    use super::*;

    fn placement() -> ViewportOverlayPlacement {
        ViewportOverlayPlacement::new(
            RectF {
                x: 100.0,
                y: 50.0,
                width: 800.0,
                height: 600.0,
            },
            PointF { x: 0.5, y: 1.0 },
            PointF { x: 8.0, y: -12.0 },
        )
        .unwrap()
    }

    #[test]
    fn placement_validates_host_geometry_and_resolves_normalized_coordinates() {
        assert_eq!(placement().resolved_anchor(), PointF { x: 508.0, y: 638.0 });
        assert_eq!(
            ViewportOverlayPlacement::new(
                RectF {
                    width: 0.0,
                    height: 10.0,
                    ..RectF::ZERO
                },
                PointF::default(),
                PointF::default(),
            ),
            Err(ViewportOverlayPlacementError::NonPositiveViewport)
        );
        assert_eq!(
            ViewportOverlayPlacement::new(
                RectF {
                    width: 10.0,
                    height: 10.0,
                    ..RectF::ZERO
                },
                PointF { x: 1.1, y: 0.0 },
                PointF::default(),
            ),
            Err(ViewportOverlayPlacementError::AnchorOutOfBounds)
        );
    }

    struct Fixture {
        reference: Rc<Cell<Option<ViewportOverlayRef>>>,
    }

    impl Component for Fixture {
        type State = ();
        type Action = ();

        fn create(&self, _: &mut CreateContext<'_>) -> Self::State {}

        fn mount(&self, _: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let host = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            self.reference.set(Some(
                ViewportOverlay::new(placement())
                    .style(ViewportOverlayStyle {
                        content: BoxStyle {
                            opacity: 0.6,
                            ..BoxStyle::default()
                        },
                        content_layout: LayoutStyle {
                            gap: 9.0,
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
    fn mount_positions_an_outer_owner_and_preserves_caller_content_inputs() {
        let reference = Rc::new(Cell::new(None));
        let runtime = ViewRuntime::from_component(Fixture {
            reference: reference.clone(),
        })
        .unwrap();
        let reference = reference.get().unwrap();
        assert_ne!(reference.node(), reference.content_node());
        assert_eq!(
            runtime
                .ui()
                .box_styles
                .get(reference.node())
                .unwrap()
                .transform
                .translation,
            placement().resolved_anchor()
        );
        assert_eq!(
            runtime
                .ui()
                .box_styles
                .get(reference.content_node())
                .unwrap()
                .opacity,
            0.6
        );
        assert_eq!(
            runtime
                .ui()
                .layouts
                .get(reference.content_node())
                .unwrap()
                .gap,
            9.0
        );
        assert_eq!(
            runtime
                .ui()
                .semantics
                .get(reference.node())
                .unwrap()
                .participation,
            SemanticParticipation::MergeDescendants
        );
        assert_eq!(
            runtime
                .ui()
                .interactions
                .get(reference.node())
                .map_or(0, |interaction| interaction.listener_mask),
            0
        );
    }
}

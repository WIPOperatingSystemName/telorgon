//! Stable caller content under a host-provided projected transform and visibility/depth hint.

use std::fmt;

use crate::foundation::Transform2D;
use crate::runtime::{RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    BoxStyle, ControlHandle, LayoutStyle, Property, SemanticNode, SemanticParticipation,
    SemanticState, UiNodeId,
};

/// Host classification retained independently of renderer or camera conventions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WorldAnchorVisibility {
    Visible,
    Occluded,
    OutsideViewport,
}

impl WorldAnchorVisibility {
    pub const fn is_visible(self) -> bool {
        matches!(self, Self::Visible)
    }
}

/// Complete host-projected input consumed by a world anchor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldAnchorProjection {
    transform: Transform2D,
    visibility: WorldAnchorVisibility,
    depth_hint: f32,
}

impl WorldAnchorProjection {
    pub fn new(
        transform: Transform2D,
        visibility: WorldAnchorVisibility,
        depth_hint: f32,
    ) -> Result<Self, WorldAnchorProjectionError> {
        if !transform.translation.x.is_finite() || !transform.translation.y.is_finite() {
            return Err(WorldAnchorProjectionError::NonFiniteTranslation);
        }
        if !transform.scale.x.is_finite() || !transform.scale.y.is_finite() {
            return Err(WorldAnchorProjectionError::NonFiniteScale);
        }
        if transform.scale.x <= 0.0 || transform.scale.y <= 0.0 {
            return Err(WorldAnchorProjectionError::NonPositiveScale);
        }
        if !depth_hint.is_finite() {
            return Err(WorldAnchorProjectionError::NonFiniteDepth);
        }
        Ok(Self {
            transform,
            visibility,
            depth_hint,
        })
    }

    pub const fn transform(self) -> Transform2D {
        self.transform
    }

    pub const fn visibility(self) -> WorldAnchorVisibility {
        self.visibility
    }

    pub const fn depth_hint(self) -> f32 {
        self.depth_hint
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WorldAnchorStyle {
    pub content: BoxStyle,
    pub content_layout: LayoutStyle,
}

/// Immutable mount snapshot of one host-projected world anchor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldAnchor {
    projection: WorldAnchorProjection,
    style: WorldAnchorStyle,
}

impl WorldAnchor {
    pub fn new(projection: WorldAnchorProjection) -> Self {
        Self {
            projection,
            style: WorldAnchorStyle {
                content: BoxStyle::default(),
                content_layout: LayoutStyle::default(),
            },
        }
    }

    pub const fn style(mut self, style: WorldAnchorStyle) -> Self {
        self.style = style;
        self
    }

    pub const fn projection(&self) -> WorldAnchorProjection {
        self.projection
    }

    pub const fn anchor_style(&self) -> WorldAnchorStyle {
        self.style
    }

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
    ) -> RuntimeResult<WorldAnchorRef> {
        let anchor_style = BoxStyle {
            transform: self.projection.transform,
            ..BoxStyle::default()
        };
        let anchor = ui
            .foundation()
            .layer_node_under(
                host,
                self.projection.visibility.is_visible(),
                anchor_style,
                LayoutStyle::default(),
                |_| {},
            )
            .ok_or_else(|| RuntimeError::new("world anchor parent is stale"))?;
        let content = ui
            .foundation()
            .container_node_under(
                anchor.node,
                self.style.content,
                self.style.content_layout,
                |_| {},
            )
            .ok_or_else(|| RuntimeError::new("world anchor content parent is stale"))?;
        ui.foundation()
            .semantic_node(
                anchor.node,
                SemanticNode {
                    state: SemanticState {
                        hidden: !self.projection.visibility.is_visible(),
                        ..SemanticState::default()
                    },
                    participation: SemanticParticipation::MergeDescendants,
                    ..SemanticNode::default()
                },
            )
            .map_err(|error| {
                RuntimeError::new(format!("invalid world anchor semantics: {error:?}"))
            })?;
        Ok(WorldAnchorRef {
            anchor,
            content,
            projection: self.projection,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct WorldAnchorRef {
    anchor: ControlHandle,
    content: ControlHandle,
    projection: WorldAnchorProjection,
}

impl WorldAnchorRef {
    pub const fn node(self) -> UiNodeId {
        self.anchor.node
    }

    pub const fn content_node(self) -> UiNodeId {
        self.content.node
    }

    pub const fn projection_style(self) -> Property<BoxStyle> {
        self.anchor.style
    }

    pub const fn visible(self) -> Property<bool> {
        self.anchor.visible
    }

    pub const fn content_style(self) -> Property<BoxStyle> {
        self.content.style
    }

    pub const fn projection(self) -> WorldAnchorProjection {
        self.projection
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorldAnchorProjectionError {
    NonFiniteTranslation,
    NonFiniteScale,
    NonPositiveScale,
    NonFiniteDepth,
}

impl fmt::Display for WorldAnchorProjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid world anchor projection: {self:?}")
    }
}

impl std::error::Error for WorldAnchorProjectionError {}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::foundation::PointF;
    use crate::runtime::{Component, CreateContext, UpdateContext, ViewRuntime};
    use crate::ui::UiRoot;

    use super::*;

    fn projection(visibility: WorldAnchorVisibility) -> WorldAnchorProjection {
        WorldAnchorProjection::new(
            Transform2D {
                translation: PointF { x: 320.0, y: 180.0 },
                scale: PointF { x: 0.75, y: 0.75 },
                ..Transform2D::default()
            },
            visibility,
            -4.5,
        )
        .unwrap()
    }

    #[test]
    fn projection_requires_finite_positive_host_inputs_without_constraining_depth_convention() {
        assert_eq!(
            projection(WorldAnchorVisibility::Visible).depth_hint(),
            -4.5
        );
        assert_eq!(
            WorldAnchorProjection::new(
                Transform2D {
                    scale: PointF { x: 0.0, y: 1.0 },
                    ..Transform2D::default()
                },
                WorldAnchorVisibility::Visible,
                0.0,
            ),
            Err(WorldAnchorProjectionError::NonPositiveScale)
        );
        assert_eq!(
            WorldAnchorProjection::new(
                Transform2D::default(),
                WorldAnchorVisibility::Visible,
                f32::INFINITY,
            ),
            Err(WorldAnchorProjectionError::NonFiniteDepth)
        );
    }

    struct Fixture {
        references: Rc<RefCell<Vec<WorldAnchorRef>>>,
    }

    impl Component for Fixture {
        type State = ();
        type Action = ();

        fn create(&self, _: &mut CreateContext<'_>) -> Self::State {}

        fn mount(&self, _: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let host = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            for visibility in [
                WorldAnchorVisibility::Visible,
                WorldAnchorVisibility::Occluded,
                WorldAnchorVisibility::OutsideViewport,
            ] {
                self.references.borrow_mut().push(
                    WorldAnchor::new(projection(visibility))
                        .style(WorldAnchorStyle {
                            content: BoxStyle {
                                opacity: 0.55,
                                ..BoxStyle::default()
                            },
                            content_layout: LayoutStyle {
                                gap: 2.0,
                                ..LayoutStyle::default()
                            },
                        })
                        .mount(ui, host.0)
                        .unwrap(),
                );
            }
            host
        }

        fn action(&self, _: &mut Self::State, _: Self::Action, _: &mut UpdateContext<'_, Self>) {}
    }

    #[test]
    fn mount_uses_host_projection_visibility_and_depth_without_camera_or_renderer_state() {
        let references = Rc::new(RefCell::new(Vec::new()));
        let runtime = ViewRuntime::from_component(Fixture {
            references: references.clone(),
        })
        .unwrap();
        let references = references.borrow();
        assert_eq!(references.len(), 3);
        for (index, reference) in references.iter().copied().enumerate() {
            let visible = index == 0;
            assert_eq!(
                runtime
                    .ui()
                    .interactions
                    .get(reference.node())
                    .is_none_or(|interaction| interaction.visible),
                visible
            );
            assert_eq!(
                runtime
                    .ui()
                    .semantics
                    .get(reference.node())
                    .unwrap()
                    .state
                    .hidden,
                !visible
            );
            assert_eq!(
                runtime
                    .ui()
                    .box_styles
                    .get(reference.node())
                    .unwrap()
                    .transform,
                reference.projection().transform()
            );
            assert_eq!(reference.projection().depth_hint(), -4.5);
            assert_eq!(
                runtime
                    .ui()
                    .box_styles
                    .get(reference.content_node())
                    .unwrap()
                    .opacity,
                0.55
            );
            assert_eq!(
                runtime
                    .ui()
                    .layouts
                    .get(reference.content_node())
                    .unwrap()
                    .gap,
                2.0
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
}

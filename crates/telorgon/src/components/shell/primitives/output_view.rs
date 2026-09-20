//! Output-local logical coordinate mapping and mounted caller-content host.

use std::fmt;

use crate::foundation::{PointF, RectF};
use crate::runtime::{RuntimeError, RuntimeResult, Ui};
use crate::shell::{OutputId, OutputRevision, OutputSnapshot, OutputTransform};
use crate::ui::{
    BoxStyle, ControlHandle, LayoutStyle, Property, SemanticNode, SemanticParticipation, UiNodeId,
};

use crate::components::shell::primitives::ShellRootRef;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OutputViewStyle {
    pub container: BoxStyle,
    pub layout: LayoutStyle,
}

/// One immutable output snapshot adapted into output-local logical coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutputView {
    snapshot: OutputSnapshot,
    style: OutputViewStyle,
}

impl OutputView {
    pub fn new(snapshot: OutputSnapshot) -> Self {
        Self {
            snapshot,
            style: OutputViewStyle::default(),
        }
    }

    pub const fn style(mut self, style: OutputViewStyle) -> Self {
        self.style = style;
        self
    }

    pub const fn snapshot(self) -> OutputSnapshot {
        self.snapshot
    }

    pub const fn output(self) -> OutputId {
        self.snapshot.id()
    }

    pub const fn revision(self) -> OutputRevision {
        self.snapshot.revision()
    }

    pub const fn scale(self) -> f32 {
        self.snapshot.geometry().scale()
    }

    pub const fn transform(self) -> OutputTransform {
        self.snapshot.geometry().transform()
    }

    pub fn local_logical_bounds(self) -> RectF {
        let logical = self.snapshot.geometry().logical_bounds();
        RectF {
            x: 0.0,
            y: 0.0,
            width: logical.width,
            height: logical.height,
        }
    }

    pub fn local_usable_bounds(self) -> RectF {
        self.to_local_rect(self.snapshot.geometry().usable_bounds())
    }

    pub fn local_safe_bounds(self) -> RectF {
        self.to_local_rect(self.snapshot.geometry().safe_bounds())
    }

    pub fn to_local(self, point: PointF) -> Result<PointF, OutputViewMappingError> {
        if !point.x.is_finite() || !point.y.is_finite() {
            return Err(OutputViewMappingError::NonFinitePoint);
        }
        let logical = self.snapshot.geometry().logical_bounds();
        Ok(PointF {
            x: point.x - logical.x,
            y: point.y - logical.y,
        })
    }

    pub fn to_global(self, point: PointF) -> Result<PointF, OutputViewMappingError> {
        if !point.x.is_finite() || !point.y.is_finite() {
            return Err(OutputViewMappingError::NonFinitePoint);
        }
        let logical = self.snapshot.geometry().logical_bounds();
        let mapped = PointF {
            x: point.x + logical.x,
            y: point.y + logical.y,
        };
        if !mapped.x.is_finite() || !mapped.y.is_finite() {
            return Err(OutputViewMappingError::NonFinitePoint);
        }
        Ok(mapped)
    }

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        root: ShellRootRef,
    ) -> RuntimeResult<OutputViewRef> {
        if root.output() != self.output() {
            return Err(RuntimeError::new(
                "output view does not match shell root output",
            ));
        }
        let content = ui
            .foundation()
            .container_node_under(
                root.content_node(),
                self.style.container,
                self.style.layout,
                |_| {},
            )
            .ok_or_else(|| RuntimeError::new("shell root content is stale"))?;
        ui.foundation()
            .semantic_node(
                content.node,
                SemanticNode {
                    participation: SemanticParticipation::MergeDescendants,
                    ..SemanticNode::default()
                },
            )
            .map_err(|error| {
                RuntimeError::new(format!("invalid output view semantics: {error:?}"))
            })?;
        Ok(OutputViewRef {
            content,
            snapshot: self.snapshot,
        })
    }

    fn to_local_rect(self, rect: RectF) -> RectF {
        let logical = self.snapshot.geometry().logical_bounds();
        RectF {
            x: rect.x - logical.x,
            y: rect.y - logical.y,
            width: rect.width,
            height: rect.height,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct OutputViewRef {
    content: ControlHandle,
    snapshot: OutputSnapshot,
}

impl OutputViewRef {
    pub const fn node(self) -> UiNodeId {
        self.content.node
    }

    pub const fn content_node(self) -> UiNodeId {
        self.content.node
    }

    pub const fn output(self) -> OutputId {
        self.snapshot.id()
    }

    pub const fn revision(self) -> OutputRevision {
        self.snapshot.revision()
    }

    pub const fn snapshot(self) -> OutputSnapshot {
        self.snapshot
    }

    pub const fn style(self) -> Property<BoxStyle> {
        self.content.style
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputViewMappingError {
    NonFinitePoint,
}

impl fmt::Display for OutputViewMappingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("output coordinate must be finite")
    }
}

impl std::error::Error for OutputViewMappingError {}

#[cfg(test)]
mod tests {
    use crate::foundation::{EdgeInsets, SizeI};
    use crate::shell::{OutputColorCapabilities, OutputGeometry};

    use super::*;

    fn view() -> OutputView {
        OutputView::new(OutputSnapshot::new(
            OutputId::from_raw(2).unwrap(),
            OutputRevision::from_raw(3).unwrap(),
            OutputGeometry::new(
                RectF {
                    x: -100.0,
                    y: 20.0,
                    width: 100.0,
                    height: 80.0,
                },
                RectF {
                    x: -100.0,
                    y: 30.0,
                    width: 100.0,
                    height: 70.0,
                },
                SizeI {
                    width: 200,
                    height: 160,
                },
                2.0,
                OutputTransform::Normal,
                EdgeInsets {
                    top: 5.0,
                    right: 0.0,
                    bottom: 0.0,
                    left: 0.0,
                },
                OutputColorCapabilities::SRGB,
            )
            .unwrap(),
        ))
    }

    #[test]
    fn mapping_preserves_host_geometry_in_output_local_coordinates() {
        let view = view();
        assert_eq!(
            view.local_logical_bounds(),
            RectF {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 80.0,
            }
        );
        assert_eq!(view.local_usable_bounds().y, 10.0);
        assert_eq!(view.local_safe_bounds().y, 5.0);
        assert_eq!(
            view.to_local(PointF { x: -90.0, y: 25.0 }).unwrap(),
            PointF { x: 10.0, y: 5.0 }
        );
        assert_eq!(
            view.to_global(PointF { x: 10.0, y: 5.0 }).unwrap(),
            PointF { x: -90.0, y: 25.0 }
        );
        assert_eq!(view.scale(), 2.0);
    }

    #[test]
    fn nonfinite_coordinates_are_rejected() {
        assert_eq!(
            view().to_local(PointF {
                x: f32::NAN,
                y: 0.0,
            }),
            Err(OutputViewMappingError::NonFinitePoint)
        );
    }
}

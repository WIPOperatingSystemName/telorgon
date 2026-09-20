//! Stable decorative or meaningful application separator.

use std::fmt;

use crate::foundation::ColorRgba8;
use crate::runtime::{RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    Background, BoxStyle, ControlHandle, LayoutStyle, Property, SemanticName, SemanticNode,
    SemanticParticipation, SemanticRole, SizeRule, UiNodeId,
};

/// Visual axis occupied by a separator's length.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SeparatorOrientation {
    #[default]
    Horizontal,
    Vertical,
}

/// Whether a separator is absent from semantics or represents a meaningful division.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SeparatorSemanticPolicy {
    #[default]
    Decorative,
    Named,
}

/// Validated logical length and cross-axis thickness.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeparatorGeometry {
    length: f32,
    thickness: f32,
}

impl SeparatorGeometry {
    pub fn new(length: f32, thickness: f32) -> Result<Self, SeparatorError> {
        if !length.is_finite() || length <= 0.0 {
            return Err(SeparatorError::InvalidLength);
        }
        if !thickness.is_finite() || thickness <= 0.0 {
            return Err(SeparatorError::InvalidThickness);
        }
        Ok(Self { length, thickness })
    }

    pub const fn length(self) -> f32 {
        self.length
    }

    pub const fn thickness(self) -> f32 {
        self.thickness
    }
}

/// Caller-owned visual and internal layout inputs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeparatorStyle {
    pub line: BoxStyle,
    pub layout: LayoutStyle,
}

impl Default for SeparatorStyle {
    fn default() -> Self {
        Self {
            line: BoxStyle {
                decoration: crate::ui::BoxDecoration {
                    background: Background::Color(ColorRgba8::rgba(104, 116, 139, 255)),
                    ..crate::ui::BoxDecoration::default()
                },
                ..BoxStyle::default()
            },
            layout: LayoutStyle::default(),
        }
    }
}

/// Noninteractive mounted separator with explicit semantic participation.
#[derive(Clone, Debug, PartialEq)]
pub struct Separator {
    orientation: SeparatorOrientation,
    geometry: SeparatorGeometry,
    accessible_name: Option<String>,
    style: SeparatorStyle,
}

impl Separator {
    pub fn decorative(orientation: SeparatorOrientation, geometry: SeparatorGeometry) -> Self {
        Self {
            orientation,
            geometry,
            accessible_name: None,
            style: SeparatorStyle::default(),
        }
    }

    pub fn named(
        label: impl Into<String>,
        orientation: SeparatorOrientation,
        geometry: SeparatorGeometry,
    ) -> Result<Self, SeparatorError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(SeparatorError::MissingAccessibleName);
        }
        Ok(Self {
            orientation,
            geometry,
            accessible_name: Some(label),
            style: SeparatorStyle::default(),
        })
    }

    pub const fn orientation(&self) -> SeparatorOrientation {
        self.orientation
    }

    pub const fn geometry(&self) -> SeparatorGeometry {
        self.geometry
    }

    pub fn semantic_policy(&self) -> SeparatorSemanticPolicy {
        if self.accessible_name.is_some() {
            SeparatorSemanticPolicy::Named
        } else {
            SeparatorSemanticPolicy::Decorative
        }
    }

    pub fn accessible_name(&self) -> Option<&str> {
        self.accessible_name.as_deref()
    }

    pub const fn style(mut self, style: SeparatorStyle) -> Self {
        self.style = style;
        self
    }

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
    ) -> RuntimeResult<SeparatorRef> {
        let line = resolve_line_style(self.style.line, self.orientation, self.geometry);
        let control = ui
            .foundation()
            .container_node_under(host, line, self.style.layout, |_| {})
            .ok_or_else(|| RuntimeError::new("application separator host is stale"))?;
        let semantic = if let Some(label) = &self.accessible_name {
            let name = ui.foundation().intern(label);
            SemanticNode {
                role: SemanticRole::Separator,
                name: SemanticName::Text(name),
                ..SemanticNode::default()
            }
        } else {
            SemanticNode {
                role: SemanticRole::Separator,
                participation: SemanticParticipation::Exclude,
                ..SemanticNode::default()
            }
        };
        ui.foundation()
            .semantic_node(control.node, semantic)
            .map_err(semantic_runtime_error)?;
        Ok(SeparatorRef { control })
    }
}

fn resolve_line_style(
    mut style: BoxStyle,
    orientation: SeparatorOrientation,
    geometry: SeparatorGeometry,
) -> BoxStyle {
    match orientation {
        SeparatorOrientation::Horizontal => {
            style.width = SizeRule::Logical(geometry.length());
            style.height = SizeRule::Logical(geometry.thickness());
        }
        SeparatorOrientation::Vertical => {
            style.width = SizeRule::Logical(geometry.thickness());
            style.height = SizeRule::Logical(geometry.length());
        }
    }
    style
}

fn semantic_runtime_error(error: crate::ui::SemanticError) -> RuntimeError {
    RuntimeError::new(format!("invalid separator semantics: {error:?}"))
}

/// Stable mounted identity and style property for one separator.
#[derive(Clone, Copy, Debug)]
pub struct SeparatorRef {
    control: ControlHandle,
}

impl SeparatorRef {
    pub const fn node(self) -> UiNodeId {
        self.control.node
    }

    pub const fn style(self) -> Property<BoxStyle> {
        self.control.style
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeparatorError {
    MissingAccessibleName,
    InvalidLength,
    InvalidThickness,
}

impl fmt::Display for SeparatorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid separator: {self:?}")
    }
}

impl std::error::Error for SeparatorError {}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::foundation::PointF;
    use crate::runtime::{Component, CreateContext, UpdateContext, ViewRuntime};
    use crate::ui::{Border, CornerRadii, Flow, NodeKind, SemanticActions, SemanticValue, UiRoot};

    use super::*;

    #[test]
    fn geometry_and_named_policy_reject_invalid_inputs() {
        assert_eq!(
            SeparatorGeometry::new(0.0, 1.0),
            Err(SeparatorError::InvalidLength)
        );
        assert_eq!(
            SeparatorGeometry::new(f32::NAN, 1.0),
            Err(SeparatorError::InvalidLength)
        );
        assert_eq!(
            SeparatorGeometry::new(40.0, -1.0),
            Err(SeparatorError::InvalidThickness)
        );
        let geometry = SeparatorGeometry::new(40.0, 1.0).unwrap();
        assert_eq!(
            Separator::named(" ", SeparatorOrientation::Horizontal, geometry),
            Err(SeparatorError::MissingAccessibleName)
        );
    }

    #[test]
    fn constructors_make_decorative_and_named_semantics_explicit() {
        let geometry = SeparatorGeometry::new(80.0, 2.0).unwrap();
        let decorative = Separator::decorative(SeparatorOrientation::Horizontal, geometry);
        assert_eq!(
            decorative.semantic_policy(),
            SeparatorSemanticPolicy::Decorative
        );
        assert_eq!(decorative.accessible_name(), None);
        assert_eq!(decorative.geometry(), geometry);

        let named = Separator::named(
            "Primary and secondary",
            SeparatorOrientation::Vertical,
            geometry,
        )
        .unwrap();
        assert_eq!(named.semantic_policy(), SeparatorSemanticPolicy::Named);
        assert_eq!(named.accessible_name(), Some("Primary and secondary"));
        assert_eq!(named.orientation(), SeparatorOrientation::Vertical);
    }

    struct Fixture {
        references: Rc<RefCell<Option<(SeparatorRef, SeparatorRef)>>>,
    }

    impl Component for Fixture {
        type State = ();
        type Action = ();

        fn create(&self, _: &mut CreateContext<'_>) -> Self::State {}

        fn mount(&self, _: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            let decorative_style = SeparatorStyle {
                line: BoxStyle {
                    decoration: crate::ui::BoxDecoration {
                        background: Background::Color(ColorRgba8::rgba(10, 20, 30, 255)),
                        border: Border::all(2.0, ColorRgba8::rgba(40, 50, 60, 255)),
                        corner_radii: CornerRadii::all(3.0),
                        ..crate::ui::BoxDecoration::default()
                    },
                    ..BoxStyle::default()
                },
                layout: LayoutStyle {
                    flow: Flow::Overlay,
                    gap: 4.0,
                    contain: true,
                    scroll_offset: PointF { x: 2.0, y: 3.0 },
                    ..LayoutStyle::default()
                },
            };
            let decorative = Separator::decorative(
                SeparatorOrientation::Horizontal,
                SeparatorGeometry::new(120.0, 2.0).unwrap(),
            )
            .style(decorative_style)
            .mount(ui, root.0)
            .unwrap();
            let named = Separator::named(
                "Editor sections",
                SeparatorOrientation::Vertical,
                SeparatorGeometry::new(80.0, 3.0).unwrap(),
            )
            .unwrap()
            .mount(ui, root.0)
            .unwrap();
            *self.references.borrow_mut() = Some((decorative, named));
            root
        }

        fn action(&self, _: &mut Self::State, _: Self::Action, _: &mut UpdateContext<'_, Self>) {}
    }

    #[test]
    fn mounted_geometry_preserves_visual_inputs_and_semantic_exclusion_policy() {
        let references = Rc::new(RefCell::new(None));
        let runtime = ViewRuntime::from_component(Fixture {
            references: references.clone(),
        })
        .unwrap();
        let (decorative, named) = references.borrow().expect("separator references");
        let decorative_node = decorative.node();
        let named_node = named.node();
        assert_eq!(
            runtime.ui().kinds.get(decorative_node),
            Some(&NodeKind::Box)
        );
        assert_eq!(runtime.ui().kinds.get(named_node), Some(&NodeKind::Box));

        let decorative_style = runtime.ui().box_styles.get(decorative_node).unwrap();
        assert_eq!(decorative_style.width, SizeRule::Logical(120.0));
        assert_eq!(decorative_style.height, SizeRule::Logical(2.0));
        assert_eq!(
            decorative_style.decoration.background,
            Background::Color(ColorRgba8::rgba(10, 20, 30, 255))
        );
        assert_eq!(
            decorative_style.decoration.corner_radii,
            CornerRadii::all(3.0)
        );
        assert_eq!(
            runtime.ui().layouts.get(decorative_node).unwrap(),
            &LayoutStyle {
                flow: Flow::Overlay,
                gap: 4.0,
                contain: true,
                scroll_offset: PointF { x: 2.0, y: 3.0 },
                ..LayoutStyle::default()
            }
        );
        let named_style = runtime.ui().box_styles.get(named_node).unwrap();
        assert_eq!(named_style.width, SizeRule::Logical(3.0));
        assert_eq!(named_style.height, SizeRule::Logical(80.0));

        let decorative_semantics = runtime.ui().semantics.get(decorative_node).unwrap();
        assert_eq!(decorative_semantics.role, SemanticRole::Separator);
        assert_eq!(
            decorative_semantics.participation,
            SemanticParticipation::Exclude
        );
        assert_eq!(decorative_semantics.actions, SemanticActions::NONE);
        assert!(decorative_semantics.effective_actions().is_empty());

        let named_semantics = runtime.ui().semantics.get(named_node).unwrap();
        assert_eq!(named_semantics.role, SemanticRole::Separator);
        assert!(matches!(named_semantics.name, SemanticName::Text(_)));
        assert_eq!(named_semantics.participation, SemanticParticipation::Node);
        assert!(!named_semantics.state.focusable);
        assert_eq!(named_semantics.value, SemanticValue::None);
        assert_eq!(named_semantics.actions, SemanticActions::NONE);
        for node in [decorative_node, named_node] {
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

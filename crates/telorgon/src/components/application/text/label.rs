//! Stable visible application label text.

use std::fmt;

use crate::foundation::ColorRgba8;
use crate::runtime::{RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    BoxStyle, LayoutStyle, Property, SemanticName, SemanticNode, SemanticRelationship,
    SemanticRelationshipKind, SemanticRole, StringId, TextHandle, TextStyle, TextVisual, UiNodeId,
};

/// Validated visible text plus caller-owned retained content revision.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LabelContent {
    text: String,
    revision: u64,
}

impl LabelContent {
    pub fn new(text: impl Into<String>, revision: u64) -> Result<Self, LabelError> {
        let text = text.into();
        if text.trim().is_empty() {
            return Err(LabelError::MissingText);
        }
        Ok(Self { text, revision })
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }
}

/// Validated renderer-neutral text visual inputs.
#[derive(Clone, Debug, PartialEq)]
pub struct LabelTextStyle {
    color: ColorRgba8,
    size: f32,
    line_height: f32,
    family: String,
    weight: u16,
}

impl LabelTextStyle {
    pub fn new(
        color: ColorRgba8,
        size: f32,
        line_height: f32,
        family: impl Into<String>,
        weight: u16,
    ) -> Result<Self, LabelTextStyleError> {
        if !size.is_finite() || size <= 0.0 {
            return Err(LabelTextStyleError::InvalidSize);
        }
        if !line_height.is_finite() || line_height <= 0.0 {
            return Err(LabelTextStyleError::InvalidLineHeight);
        }
        let family = family.into();
        if family.trim().is_empty() {
            return Err(LabelTextStyleError::MissingFamily);
        }
        if !(1..=1000).contains(&weight) {
            return Err(LabelTextStyleError::InvalidWeight);
        }
        Ok(Self {
            color,
            size,
            line_height,
            family,
            weight,
        })
    }

    pub const fn color(&self) -> ColorRgba8 {
        self.color
    }

    pub const fn size(&self) -> f32 {
        self.size
    }

    pub const fn line_height(&self) -> f32 {
        self.line_height
    }

    pub fn family(&self) -> &str {
        &self.family
    }

    pub const fn weight(&self) -> u16 {
        self.weight
    }

    pub(crate) fn resolve(&self, family: StringId) -> TextStyle {
        TextStyle {
            color: self.color,
            size: self.size,
            line_height: self.line_height,
            family,
            weight: self.weight,
            align: crate::ui::TextAlign::Start,
            vertical_align: crate::ui::TextAlign::Start,
            fit_height: false,
        }
    }
}

impl Default for LabelTextStyle {
    fn default() -> Self {
        Self {
            color: ColorRgba8::rgba(27, 31, 40, 255),
            size: 14.0,
            line_height: 17.5,
            family: "sans-serif".to_owned(),
            weight: 400,
        }
    }
}

/// Caller-owned label text, box, and layout inputs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LabelStyle {
    pub text: LabelTextStyle,
    pub container: BoxStyle,
    pub layout: LayoutStyle,
}

/// Immutable mount configuration for one visible application label.
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    content: LabelContent,
    style: LabelStyle,
}

impl Label {
    pub fn new(text: impl Into<String>) -> Result<Self, LabelError> {
        Ok(Self::from_content(LabelContent::new(text, 1)?))
    }

    pub fn from_content(content: LabelContent) -> Self {
        Self {
            content,
            style: LabelStyle::default(),
        }
    }

    pub const fn content(&self) -> &LabelContent {
        &self.content
    }

    pub fn style(mut self, style: LabelStyle) -> Self {
        self.style = style;
        self
    }

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
    ) -> RuntimeResult<LabelRef> {
        let content = ui.foundation().intern(self.content.text());
        let family = ui.foundation().intern(self.style.text.family());
        let text = ui
            .foundation()
            .text_node_under(
                host,
                TextVisual {
                    content,
                    style: self.style.text.resolve(family),
                    revision: self.content.revision(),
                },
                self.style.container,
                self.style.layout,
                true,
                false,
            )
            .ok_or_else(|| RuntimeError::new("application label host is stale"))?;
        ui.foundation()
            .semantic_node(
                text.node,
                SemanticNode {
                    role: SemanticRole::Text,
                    name: SemanticName::Text(content),
                    ..SemanticNode::default()
                },
            )
            .map_err(semantic_runtime_error)?;
        Ok(LabelRef { text })
    }
}

fn semantic_runtime_error(error: crate::ui::SemanticError) -> RuntimeError {
    RuntimeError::new(format!("invalid label semantics: {error:?}"))
}

/// Stable mounted text identity suitable for `LabelledBy` relationships.
#[derive(Clone, Copy, Debug)]
pub struct LabelRef {
    text: TextHandle,
}

impl LabelRef {
    pub const fn node(self) -> UiNodeId {
        self.text.node
    }

    pub const fn text(self) -> Property<StringId> {
        self.text.text
    }

    pub const fn color(self) -> Property<ColorRgba8> {
        self.text.color
    }

    pub const fn labelled_by(self) -> SemanticRelationship {
        SemanticRelationship {
            kind: SemanticRelationshipKind::LabelledBy,
            target: self.text.node,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelError {
    MissingText,
}

impl fmt::Display for LabelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid label: {self:?}")
    }
}

impl std::error::Error for LabelError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelTextStyleError {
    InvalidSize,
    InvalidLineHeight,
    MissingFamily,
    InvalidWeight,
}

impl fmt::Display for LabelTextStyleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid label text style: {self:?}")
    }
}

impl std::error::Error for LabelTextStyleError {}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use crate::foundation::PointF;
    use crate::runtime::{Component, CreateContext, UpdateContext, ViewRuntime};
    use crate::ui::{
        Background, Flow, NodeKind, SemanticActions, SemanticParticipation, SemanticValue,
        SizeRule, UiRoot,
    };

    use super::*;

    #[test]
    fn content_and_text_style_reject_invalid_inputs() {
        assert_eq!(Label::new(" "), Err(LabelError::MissingText));
        let content = LabelContent::new("Account name", 27).unwrap();
        assert_eq!(content.text(), "Account name");
        assert_eq!(content.revision(), 27);

        let color = ColorRgba8::rgba(1, 2, 3, 255);
        assert_eq!(
            LabelTextStyle::new(color, 0.0, 16.0, "sans-serif", 400),
            Err(LabelTextStyleError::InvalidSize)
        );
        assert_eq!(
            LabelTextStyle::new(color, 14.0, f32::NAN, "sans-serif", 400),
            Err(LabelTextStyleError::InvalidLineHeight)
        );
        assert_eq!(
            LabelTextStyle::new(color, 14.0, 16.0, " ", 400),
            Err(LabelTextStyleError::MissingFamily)
        );
        assert_eq!(
            LabelTextStyle::new(color, 14.0, 16.0, "sans-serif", 1001),
            Err(LabelTextStyleError::InvalidWeight)
        );
    }

    struct Fixture {
        reference: Rc<Cell<Option<LabelRef>>>,
    }

    impl Component for Fixture {
        type State = ();
        type Action = ();

        fn create(&self, _: &mut CreateContext<'_>) -> Self::State {}

        fn mount(&self, _: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            let text = LabelTextStyle::new(
                ColorRgba8::rgba(80, 90, 100, 255),
                18.0,
                24.0,
                "application-ui",
                600,
            )
            .unwrap();
            let label = Label::from_content(LabelContent::new("Deployment status", 42).unwrap())
                .style(LabelStyle {
                    text,
                    container: BoxStyle {
                        width: SizeRule::Logical(240.0),
                        decoration: crate::ui::BoxDecoration {
                            background: Background::Color(ColorRgba8::rgba(10, 20, 30, 255)),
                            ..crate::ui::BoxDecoration::default()
                        },
                        opacity: 0.8,
                        ..BoxStyle::default()
                    },
                    layout: LayoutStyle {
                        flow: Flow::Overlay,
                        gap: 2.0,
                        contain: true,
                        scroll_offset: PointF { x: 3.0, y: 4.0 },
                        ..LayoutStyle::default()
                    },
                })
                .mount(ui, root.0)
                .unwrap();
            self.reference.set(Some(label));
            root
        }

        fn action(&self, _: &mut Self::State, _: Self::Action, _: &mut UpdateContext<'_, Self>) {}
    }

    #[test]
    fn mount_preserves_content_revision_styles_layout_and_text_semantics() {
        let reference = Rc::new(Cell::new(None));
        let runtime = ViewRuntime::from_component(Fixture {
            reference: reference.clone(),
        })
        .unwrap();
        let label = reference.get().expect("label reference");
        assert_eq!(runtime.ui().kinds.get(label.node()), Some(&NodeKind::Text));

        let visual = runtime.ui().texts.get(label.node()).unwrap();
        assert_eq!(
            runtime.ui().string(visual.content),
            Some("Deployment status")
        );
        assert_eq!(
            runtime.ui().string(visual.style.family),
            Some("application-ui")
        );
        assert_eq!(visual.revision, 42);
        assert_eq!(visual.style.color, ColorRgba8::rgba(80, 90, 100, 255));
        assert_eq!(visual.style.size, 18.0);
        assert_eq!(visual.style.line_height, 24.0);
        assert_eq!(visual.style.weight, 600);

        let box_style = runtime.ui().box_styles.get(label.node()).unwrap();
        assert_eq!(box_style.width, SizeRule::Logical(240.0));
        assert_eq!(box_style.opacity, 0.8);
        assert_eq!(
            runtime.ui().layouts.get(label.node()).unwrap(),
            &LayoutStyle {
                flow: Flow::Overlay,
                gap: 2.0,
                contain: true,
                scroll_offset: PointF { x: 3.0, y: 4.0 },
                ..LayoutStyle::default()
            }
        );

        let semantics = runtime.ui().semantics.get(label.node()).unwrap();
        assert_eq!(semantics.role, SemanticRole::Text);
        assert_eq!(semantics.name, SemanticName::Text(visual.content));
        assert_eq!(semantics.participation, SemanticParticipation::Node);
        assert!(!semantics.state.focusable);
        assert_eq!(semantics.value, SemanticValue::None);
        assert_eq!(semantics.actions, SemanticActions::NONE);
        assert_eq!(
            label.labelled_by(),
            SemanticRelationship {
                kind: SemanticRelationshipKind::LabelledBy,
                target: label.node(),
            }
        );
        assert!(
            runtime
                .ui()
                .interactions
                .get(label.node())
                .is_none_or(|interaction| {
                    !interaction.focusable && interaction.listener_mask == 0
                })
        );
    }
}

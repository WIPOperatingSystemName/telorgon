use crate::ui::SemanticCheckState;

use crate::authoring::compose::ComponentCallback;

#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToggleKind {
    Checkbox,
    Switch,
}

#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct ToggleElement {
    pub kind: ToggleKind,
    pub label: String,
    pub accessible_label: Option<String>,
    pub value: SemanticCheckState,
    pub enabled: bool,
    pub style: crate::ui::BoxStyle,
    pub(crate) effects: super::interaction::InteractionEffects,
    pub on_change: Option<ComponentCallback>,
}

impl ToggleElement {
    pub(crate) fn style_id(&self) -> crate::ui::ComponentStyleId {
        crate::ui::ComponentStyleId::named(
            crate::ui::ThemeDomainId::APPLICATION,
            match self.kind {
                ToggleKind::Checkbox => "checkbox",
                ToggleKind::Switch => "switch",
            },
            "default",
        )
    }

    pub(crate) fn local_style(
        &self,
    ) -> Option<std::sync::Arc<crate::theme::CompiledComponentStyle>> {
        self.effects
            .compile_for(self.style_id(), &self.style, Default::default(), None)
    }
}

pub(super) fn default_box_style() -> crate::ui::BoxStyle {
    crate::ui::BoxStyle {
        min_size: crate::ui::SizeRule2D {
            width: crate::ui::SizeRule::Logical(32.0),
            height: crate::ui::SizeRule::Logical(32.0),
        },
        padding: crate::foundation::EdgeInsets::all(5.0),
        ..Default::default()
    }
}

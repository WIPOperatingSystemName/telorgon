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
    pub value: SemanticCheckState,
    pub enabled: bool,
    pub width: Option<f32>,
    pub on_change: Option<ComponentCallback>,
}

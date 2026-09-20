//! Mounted, renderer- and platform-neutral semantic inputs.
//!
//! This module describes component-authored meaning. A later `telorgon-accessibility` owner combines
//! these inputs with mounted identity and layout geometry to build revisioned semantic trees.
//! Platform adapters map those trees and enqueue validated actions; they are not implemented here.

use std::mem::size_of;
use std::ops::{BitOr, BitOrAssign};

use crate::graphics::scene::NodeId as UiNodeId;

use crate::ui::mounted::StringId;

/// Platform-neutral role supplied by a component.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SemanticRole {
    #[default]
    Generic,
    Application,
    Window,
    Banner,
    Navigation,
    Main,
    Complementary,
    Region,
    Text,
    Button,
    Checkbox,
    Radio,
    RadioGroup,
    Switch,
    Slider,
    ScrollBar,
    Separator,
    ProgressIndicator,
    Meter,
    TextInput,
    SearchBox,
    Image,
    Link,
    List,
    ListItem,
    ListBox,
    Option,
    Menu,
    MenuItem,
    Toolbar,
    Tab,
    TabPanel,
    Dialog,
    Alert,
    Tooltip,
    Status,
    Heading,
    Table,
    Row,
    Cell,
    ColumnHeader,
    RowHeader,
    Tree,
    TreeItem,
    Grid,
    TreeGrid,
}

impl SemanticRole {
    /// Whether this role needs a name unless its component explicitly derives one from contents.
    pub const fn requires_accessible_name(self) -> bool {
        matches!(
            self,
            Self::Application
                | Self::Window
                | Self::Banner
                | Self::Navigation
                | Self::Main
                | Self::Complementary
                | Self::Region
                | Self::Button
                | Self::Checkbox
                | Self::Radio
                | Self::RadioGroup
                | Self::Switch
                | Self::Slider
                | Self::ScrollBar
                | Self::Separator
                | Self::ProgressIndicator
                | Self::Meter
                | Self::TextInput
                | Self::SearchBox
                | Self::Image
                | Self::Link
                | Self::List
                | Self::ListBox
                | Self::Option
                | Self::Menu
                | Self::MenuItem
                | Self::Toolbar
                | Self::Tab
                | Self::Dialog
                | Self::Table
                | Self::ColumnHeader
                | Self::RowHeader
                | Self::Tree
                | Self::TreeItem
                | Self::Grid
                | Self::TreeGrid
        )
    }
}

/// Source for an accessible name.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SemanticName {
    /// No name is supplied by this node.
    #[default]
    Unspecified,
    /// The later tree owner derives the name from eligible semantic descendants.
    Contents,
    /// A component-supplied interned string, normally matching its visible label.
    Text(StringId),
}

/// Tri-state check value used by checkboxes and similar components.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SemanticCheckState {
    Unchecked,
    Checked,
    Mixed,
}

/// Component- and processor-owned state presented to accessibility consumers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SemanticState {
    pub disabled: bool,
    pub read_only: bool,
    /// The text input accepts line breaks and exposes multiline navigation semantics.
    pub multiline: bool,
    pub required: bool,
    pub invalid: bool,
    pub busy: bool,
    pub inert: bool,
    pub hidden: bool,
    pub focusable: bool,
    pub focused: bool,
    pub checked: Option<SemanticCheckState>,
    pub pressed: Option<bool>,
    pub selected: Option<bool>,
    pub expanded: Option<bool>,
}

/// Current semantic value. Text is an interned, caller-redacted value rather than platform text.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum SemanticValue {
    #[default]
    None,
    Text(StringId),
    Number {
        current: f64,
        minimum: f64,
        maximum: f64,
        step: Option<f64>,
        value_text: Option<StringId>,
    },
}

/// One operation an assistive consumer may request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum SemanticAction {
    Activate,
    Focus,
    Increment,
    Decrement,
    Expand,
    Collapse,
    Select,
    Dismiss,
    SetValue,
    SetText,
    SetSelection,
    ScrollForward,
    ScrollBackward,
    ShowContextMenu,
}

impl SemanticAction {
    const fn mask(self) -> u32 {
        1_u32 << self as u8
    }
}

/// Compact advertised semantic-action capability set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SemanticActions(u32);

impl SemanticActions {
    pub const NONE: Self = Self(0);
    pub const ACTIVATE: Self = Self(SemanticAction::Activate.mask());
    pub const FOCUS: Self = Self(SemanticAction::Focus.mask());
    pub const INCREMENT: Self = Self(SemanticAction::Increment.mask());
    pub const DECREMENT: Self = Self(SemanticAction::Decrement.mask());
    pub const EXPAND: Self = Self(SemanticAction::Expand.mask());
    pub const COLLAPSE: Self = Self(SemanticAction::Collapse.mask());
    pub const SELECT: Self = Self(SemanticAction::Select.mask());
    pub const DISMISS: Self = Self(SemanticAction::Dismiss.mask());
    pub const SET_VALUE: Self = Self(SemanticAction::SetValue.mask());
    pub const SET_TEXT: Self = Self(SemanticAction::SetText.mask());
    pub const SET_SELECTION: Self = Self(SemanticAction::SetSelection.mask());
    pub const SCROLL_FORWARD: Self = Self(SemanticAction::ScrollForward.mask());
    pub const SCROLL_BACKWARD: Self = Self(SemanticAction::ScrollBackward.mask());
    pub const SHOW_CONTEXT_MENU: Self = Self(SemanticAction::ShowContextMenu.mask());

    pub const fn from_action(action: SemanticAction) -> Self {
        Self(action.mask())
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, action: SemanticAction) -> bool {
        self.0 & action.mask() != 0
    }

    pub fn insert(&mut self, action: SemanticAction) {
        self.0 |= action.mask();
    }

    pub fn remove(&mut self, action: SemanticAction) {
        self.0 &= !action.mask();
    }
}

impl BitOr for SemanticActions {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for SemanticActions {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// How this mounted input participates in the later semantic tree.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SemanticParticipation {
    #[default]
    Node,
    MergeDescendants,
    Exclude,
}

/// Meaning of a relationship to another mounted node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SemanticRelationshipKind {
    LabelledBy,
    DescribedBy,
    Help,
    ErrorMessage,
    Controls,
    Owns,
    ActiveDescendant,
}

/// One generation-checked mounted relationship input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SemanticRelationship {
    pub kind: SemanticRelationshipKind,
    pub target: UiNodeId,
}

/// Optional collection position supplied independently of visual materialization.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SemanticCollection {
    pub item_index: Option<u32>,
    pub item_count: Option<u32>,
    pub level: Option<u32>,
    pub position_in_set: Option<u32>,
    pub set_size: Option<u32>,
}

/// One component-authored semantic record attached to a mounted UI node.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SemanticNode {
    pub role: SemanticRole,
    pub name: SemanticName,
    pub description: Option<StringId>,
    pub state: SemanticState,
    pub value: SemanticValue,
    pub actions: SemanticActions,
    pub participation: SemanticParticipation,
    pub relationships: Vec<SemanticRelationship>,
    pub collection: Option<SemanticCollection>,
}

impl SemanticNode {
    pub fn new(role: SemanticRole) -> Self {
        Self {
            role,
            ..Self::default()
        }
    }

    pub fn named(role: SemanticRole, name: StringId) -> Self {
        Self {
            role,
            name: SemanticName::Text(name),
            ..Self::default()
        }
    }

    /// Advertised operations after disabled, hidden, inert, and excluded state is applied.
    pub const fn effective_actions(&self) -> SemanticActions {
        if self.state.disabled
            || self.state.inert
            || self.state.hidden
            || matches!(self.participation, SemanticParticipation::Exclude)
        {
            SemanticActions::NONE
        } else {
            self.actions
        }
    }

    pub fn allocated_bytes(&self) -> usize {
        self.relationships.capacity() * size_of::<SemanticRelationship>()
    }

    /// Every interned string referenced by this record, for mounted-pool validation and export.
    pub fn referenced_strings(&self) -> impl Iterator<Item = StringId> {
        let name = match self.name {
            SemanticName::Text(name) => Some(name),
            SemanticName::Unspecified | SemanticName::Contents => None,
        };
        let value = match self.value {
            SemanticValue::Text(value) => Some(value),
            SemanticValue::Number { value_text, .. } => value_text,
            SemanticValue::None => None,
        };
        [name, self.description, value].into_iter().flatten()
    }

    /// Validates invariants that do not require access to the complete mounted tree.
    pub fn validate(&self, source: UiNodeId) -> Result<(), SemanticError> {
        if self.participation != SemanticParticipation::Exclude {
            match self.name {
                SemanticName::Text(StringId(0)) => return Err(SemanticError::EmptyExplicitName),
                SemanticName::Unspecified if self.role.requires_accessible_name() => {
                    return Err(SemanticError::MissingAccessibleName);
                }
                _ => {}
            }
        }

        if let SemanticValue::Number {
            current,
            minimum,
            maximum,
            step,
            ..
        } = self.value
        {
            if !current.is_finite() || !minimum.is_finite() || !maximum.is_finite() {
                return Err(SemanticError::NonFiniteRangeValue);
            }
            if minimum > maximum {
                return Err(SemanticError::InvalidRangeBounds);
            }
            if current < minimum || current > maximum {
                return Err(SemanticError::RangeValueOutOfBounds);
            }
            if step.is_some_and(|step| !step.is_finite() || step <= 0.0) {
                return Err(SemanticError::InvalidRangeStep);
            }
        }

        if let Some(collection) = self.collection {
            if collection.level == Some(0) {
                return Err(SemanticError::InvalidCollectionLevel);
            }
            if let (Some(index), Some(count)) = (collection.item_index, collection.item_count)
                && index >= count
            {
                return Err(SemanticError::InvalidCollectionIndex);
            }
            if collection.position_in_set == Some(0) || collection.set_size == Some(0) {
                return Err(SemanticError::InvalidCollectionPosition);
            }
            if let (Some(position), Some(size)) = (collection.position_in_set, collection.set_size)
                && position > size
            {
                return Err(SemanticError::InvalidCollectionPosition);
            }
        }

        for (index, relationship) in self.relationships.iter().enumerate() {
            if relationship.target == source {
                return Err(SemanticError::SelfRelationship {
                    kind: relationship.kind,
                });
            }
            if self.relationships[..index].contains(relationship) {
                return Err(SemanticError::DuplicateRelationship(*relationship));
            }
        }
        Ok(())
    }
}

/// Invalid mounted semantic input. Rejection leaves the prior record unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SemanticError {
    UnknownNode(UiNodeId),
    UnknownRelationshipTarget(UiNodeId),
    UnknownString(StringId),
    MissingAccessibleName,
    EmptyExplicitName,
    NonFiniteRangeValue,
    InvalidRangeBounds,
    RangeValueOutOfBounds,
    InvalidRangeStep,
    InvalidCollectionLevel,
    InvalidCollectionIndex,
    InvalidCollectionPosition,
    SelfRelationship { kind: SemanticRelationshipKind },
    DuplicateRelationship(SemanticRelationship),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(index: u32) -> UiNodeId {
        UiNodeId::new(index, 1)
    }

    #[test]
    fn roles_require_explicit_or_content_derived_names_without_naming_generic_nodes() {
        assert_eq!(
            SemanticNode::new(SemanticRole::Button).validate(node(1)),
            Err(SemanticError::MissingAccessibleName)
        );
        assert_eq!(
            SemanticNode::new(SemanticRole::Window).validate(node(1)),
            Err(SemanticError::MissingAccessibleName)
        );
        assert_eq!(
            SemanticNode::new(SemanticRole::Meter).validate(node(1)),
            Err(SemanticError::MissingAccessibleName)
        );
        assert_eq!(
            SemanticNode::new(SemanticRole::Separator).validate(node(1)),
            Err(SemanticError::MissingAccessibleName)
        );
        assert_eq!(
            SemanticNode::new(SemanticRole::ScrollBar).validate(node(1)),
            Err(SemanticError::MissingAccessibleName)
        );
        assert_eq!(
            SemanticNode::new(SemanticRole::Toolbar).validate(node(1)),
            Err(SemanticError::MissingAccessibleName)
        );
        assert_eq!(
            SemanticNode::new(SemanticRole::ListBox).validate(node(1)),
            Err(SemanticError::MissingAccessibleName)
        );
        assert_eq!(
            SemanticNode::new(SemanticRole::Option).validate(node(1)),
            Err(SemanticError::MissingAccessibleName)
        );
        assert_eq!(
            SemanticNode::new(SemanticRole::ColumnHeader).validate(node(1)),
            Err(SemanticError::MissingAccessibleName)
        );
        assert_eq!(
            SemanticNode::new(SemanticRole::RowHeader).validate(node(1)),
            Err(SemanticError::MissingAccessibleName)
        );
        for role in [
            SemanticRole::Tree,
            SemanticRole::TreeItem,
            SemanticRole::Grid,
            SemanticRole::TreeGrid,
        ] {
            assert_eq!(
                SemanticNode::new(role).validate(node(1)),
                Err(SemanticError::MissingAccessibleName)
            );
        }
        let mut button = SemanticNode::new(SemanticRole::Button);
        button.name = SemanticName::Contents;
        assert_eq!(button.validate(node(1)), Ok(()));
        assert_eq!(
            SemanticNode::named(SemanticRole::Button, StringId(0)).validate(node(1)),
            Err(SemanticError::EmptyExplicitName)
        );
        assert_eq!(
            SemanticNode::new(SemanticRole::Generic).validate(node(1)),
            Ok(())
        );
    }

    #[test]
    fn action_capabilities_are_typed_and_suppressed_by_unavailable_states() {
        let mut semantic = SemanticNode::named(SemanticRole::Button, StringId(1));
        semantic.actions = SemanticActions::ACTIVATE | SemanticActions::FOCUS;
        assert!(
            semantic
                .effective_actions()
                .contains(SemanticAction::Activate)
        );
        semantic.state.disabled = true;
        assert!(semantic.effective_actions().is_empty());
        semantic.state.disabled = false;
        semantic.state.inert = true;
        assert!(semantic.effective_actions().is_empty());
        semantic.state.inert = false;
        semantic.participation = SemanticParticipation::Exclude;
        assert!(semantic.effective_actions().is_empty());
    }

    #[test]
    fn numeric_values_reject_nonfinite_reversed_out_of_range_and_invalid_steps() {
        let mut slider = SemanticNode::named(SemanticRole::Slider, StringId(1));
        slider.value = SemanticValue::Number {
            current: 5.0,
            minimum: 0.0,
            maximum: 10.0,
            step: Some(1.0),
            value_text: None,
        };
        assert_eq!(slider.validate(node(1)), Ok(()));

        let invalid_values = [
            (
                SemanticValue::Number {
                    current: f64::NAN,
                    minimum: 0.0,
                    maximum: 10.0,
                    step: None,
                    value_text: None,
                },
                SemanticError::NonFiniteRangeValue,
            ),
            (
                SemanticValue::Number {
                    current: 5.0,
                    minimum: 10.0,
                    maximum: 0.0,
                    step: None,
                    value_text: None,
                },
                SemanticError::InvalidRangeBounds,
            ),
            (
                SemanticValue::Number {
                    current: 11.0,
                    minimum: 0.0,
                    maximum: 10.0,
                    step: None,
                    value_text: None,
                },
                SemanticError::RangeValueOutOfBounds,
            ),
            (
                SemanticValue::Number {
                    current: 5.0,
                    minimum: 0.0,
                    maximum: 10.0,
                    step: Some(0.0),
                    value_text: None,
                },
                SemanticError::InvalidRangeStep,
            ),
        ];
        for (value, expected) in invalid_values {
            slider.value = value;
            assert_eq!(slider.validate(node(1)), Err(expected));
        }
    }

    #[test]
    fn relationships_reject_self_and_exact_duplicates() {
        let relation = SemanticRelationship {
            kind: SemanticRelationshipKind::DescribedBy,
            target: node(2),
        };
        let mut semantic = SemanticNode {
            relationships: vec![relation, relation],
            ..SemanticNode::default()
        };
        assert_eq!(
            semantic.validate(node(1)),
            Err(SemanticError::DuplicateRelationship(relation))
        );
        semantic.relationships = vec![SemanticRelationship {
            kind: SemanticRelationshipKind::Controls,
            target: node(1),
        }];
        assert_eq!(
            semantic.validate(node(1)),
            Err(SemanticError::SelfRelationship {
                kind: SemanticRelationshipKind::Controls
            })
        );
    }

    #[test]
    fn collection_metadata_preserves_unknown_counts_and_validates_known_bounds() {
        let mut item = SemanticNode::new(SemanticRole::ListItem);
        item.collection = Some(SemanticCollection {
            item_index: Some(400),
            item_count: None,
            level: Some(1),
            position_in_set: Some(401),
            set_size: None,
        });
        assert_eq!(item.validate(node(1)), Ok(()));
        item.collection.as_mut().unwrap().item_count = Some(100);
        assert_eq!(
            item.validate(node(1)),
            Err(SemanticError::InvalidCollectionIndex)
        );
        item.collection = Some(SemanticCollection {
            position_in_set: Some(3),
            set_size: Some(2),
            ..SemanticCollection::default()
        });
        assert_eq!(
            item.validate(node(1)),
            Err(SemanticError::InvalidCollectionPosition)
        );
    }
}

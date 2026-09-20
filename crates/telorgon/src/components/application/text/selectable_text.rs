//! Parent-controlled, noneditable selectable application text.

use std::fmt;

use crate::input::ChangeSource;
use crate::runtime::{Read, RuntimeError, RuntimeResult, Ui};
use crate::ui::text::{TextAffinity, TextOffset, TextRangeError, TextSelection};
use crate::ui::{
    BoxStyle, Property, SemanticActions, SemanticName, SemanticNode, SemanticRole, SemanticState,
    SemanticValue, TextHandle, TextVisual, UiNodeId,
};

use crate::components::application::{LabelContent, LabelStyle, ValueChange};

/// Portable selection behavior over one immutable visible-text revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectableTextBehavior {
    content: LabelContent,
    enabled: bool,
}

impl SelectableTextBehavior {
    pub fn new(content: LabelContent) -> Result<Self, SelectableTextError> {
        u32::try_from(content.text().len()).map_err(|_| SelectableTextError::TextTooLong)?;
        Ok(Self {
            content,
            enabled: true,
        })
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub const fn content(&self) -> &LabelContent {
        &self.content
    }

    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn validate_current(
        &self,
        selection: TextSelection,
    ) -> Result<TextSelection, SelectableTextError> {
        selection
            .validate(self.content.text())
            .map_err(SelectableTextError::InvalidCurrentSelection)
    }

    /// Returns a committed proposal without changing the caller-owned selection.
    pub fn request(
        &self,
        current: TextSelection,
        requested: TextSelection,
        source: ChangeSource,
    ) -> Result<Option<ValueChange<TextSelection>>, SelectableTextError> {
        self.validate_current(current)?;
        let requested = requested
            .validate(self.content.text())
            .map_err(SelectableTextError::InvalidRequestedSelection)?;
        if !self.enabled || requested == current {
            return Ok(None);
        }
        Ok(Some(ValueChange::committed(requested, source)))
    }

    pub fn select_all(
        &self,
        current: TextSelection,
        source: ChangeSource,
    ) -> Result<Option<ValueChange<TextSelection>>, SelectableTextError> {
        let end = TextOffset(
            u32::try_from(self.content.text().len())
                .map_err(|_| SelectableTextError::TextTooLong)?,
        );
        self.request(
            current,
            TextSelection {
                anchor: TextOffset::ZERO,
                active: end,
                affinity: TextAffinity::Downstream,
            },
            source,
        )
    }
}

/// Immutable mount configuration for parent-controlled selectable text.
#[derive(Clone, Debug, PartialEq)]
pub struct SelectableText {
    behavior: SelectableTextBehavior,
    selection: Read<TextSelection>,
    style: LabelStyle,
}

impl SelectableText {
    pub fn new(
        content: LabelContent,
        selection: Read<TextSelection>,
    ) -> Result<Self, SelectableTextError> {
        Ok(Self {
            behavior: SelectableTextBehavior::new(content)?,
            selection,
            style: LabelStyle::default(),
        })
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.behavior = self.behavior.enabled(enabled);
        self
    }

    pub fn style(mut self, style: LabelStyle) -> Self {
        self.style = style;
        self
    }

    pub const fn behavior(&self) -> &SelectableTextBehavior {
        &self.behavior
    }

    pub const fn selection(&self) -> Read<TextSelection> {
        self.selection
    }

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
    ) -> RuntimeResult<SelectableTextRef> {
        let selection = ui.read(self.selection)?;
        self.behavior.validate_current(selection).map_err(|error| {
            RuntimeError::new(format!(
                "invalid initial selectable-text selection: {error}"
            ))
        })?;

        let content = ui.foundation().intern(self.behavior.content().text());
        let family = ui.foundation().intern(self.style.text.family());
        let enabled = self.behavior.is_enabled();
        let text = ui
            .foundation()
            .text_node_under(
                host,
                TextVisual {
                    content,
                    style: self.style.text.resolve(family),
                    revision: self.behavior.content().revision(),
                },
                self.style.container,
                self.style.layout,
                enabled,
                true,
            )
            .ok_or_else(|| RuntimeError::new("application selectable-text host is stale"))?;

        let actions = if enabled {
            SemanticActions::FOCUS | SemanticActions::SET_SELECTION
        } else {
            SemanticActions::NONE
        };
        ui.foundation()
            .semantic_node(
                text.node,
                SemanticNode {
                    role: SemanticRole::Text,
                    name: SemanticName::Text(content),
                    state: SemanticState {
                        disabled: !enabled,
                        read_only: true,
                        focusable: enabled,
                        ..SemanticState::default()
                    },
                    value: SemanticValue::Text(content),
                    actions,
                    ..SemanticNode::default()
                },
            )
            .map_err(semantic_runtime_error)?;

        Ok(SelectableTextRef {
            text,
            selection: self.selection,
            content_revision: self.behavior.content().revision(),
        })
    }
}

fn semantic_runtime_error(error: crate::ui::SemanticError) -> RuntimeError {
    RuntimeError::new(format!("invalid selectable-text semantics: {error:?}"))
}

/// Stable mounted identity and caller-owned selection read.
#[derive(Clone, Copy, Debug)]
pub struct SelectableTextRef {
    text: TextHandle,
    selection: Read<TextSelection>,
    content_revision: u64,
}

impl SelectableTextRef {
    pub const fn node(self) -> UiNodeId {
        self.text.node
    }

    pub const fn selection(self) -> Read<TextSelection> {
        self.selection
    }

    pub const fn content_revision(self) -> u64 {
        self.content_revision
    }

    pub const fn enabled(self) -> Property<bool> {
        self.text.enabled
    }

    pub const fn style(self) -> Property<BoxStyle> {
        self.text.style
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectableTextError {
    TextTooLong,
    InvalidCurrentSelection(TextRangeError),
    InvalidRequestedSelection(TextRangeError),
}

impl fmt::Display for SelectableTextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid selectable text: {self:?}")
    }
}

impl std::error::Error for SelectableTextError {}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use crate::input::ChangeSource;
    use crate::runtime::{Component, CreateContext, State, UpdateContext, ViewRuntime};
    use crate::ui::{
        InteractionFlags, LayoutStyle, NodeKind, SemanticAction, SemanticParticipation, UiRoot,
    };

    use super::*;

    fn selection(anchor: u32, active: u32) -> TextSelection {
        TextSelection {
            anchor: TextOffset(anchor),
            active: TextOffset(active),
            affinity: TextAffinity::Downstream,
        }
    }

    #[test]
    fn behavior_validates_scalar_boundaries_preserves_source_and_never_mutates() {
        let behavior = SelectableTextBehavior::new(LabelContent::new("aé中", 8).unwrap()).unwrap();
        let current = selection(0, 1);
        let proposal = behavior
            .request(current, selection(1, 6), ChangeSource::Pointer)
            .unwrap()
            .unwrap();
        assert_eq!(proposal.value, selection(1, 6));
        assert_eq!(proposal.source, ChangeSource::Pointer);
        assert_eq!(current, selection(0, 1));
        assert_eq!(
            behavior.request(current, selection(1, 2), ChangeSource::Accessibility),
            Err(SelectableTextError::InvalidRequestedSelection(
                TextRangeError::NotCharBoundary {
                    offset: TextOffset(2)
                }
            ))
        );
        assert_eq!(
            behavior.request(selection(0, 7), current, ChangeSource::Keyboard),
            Err(SelectableTextError::InvalidCurrentSelection(
                TextRangeError::OutOfBounds {
                    offset: TextOffset(7),
                    len_bytes: 6,
                }
            ))
        );
        assert!(
            behavior
                .request(current, current, ChangeSource::Programmatic)
                .unwrap()
                .is_none()
        );
        assert!(
            behavior
                .clone()
                .enabled(false)
                .select_all(current, ChangeSource::Keyboard)
                .unwrap()
                .is_none()
        );
    }

    struct Fixture {
        reference: Rc<Cell<Option<SelectableTextRef>>>,
        mount_error: Rc<RefCell<Option<String>>>,
    }

    impl Component for Fixture {
        type State = (State<TextSelection>, State<TextSelection>);
        type Action = ();

        fn create(&self, context: &mut CreateContext<'_>) -> Self::State {
            (
                context.state(selection(1, 3)),
                context.state(selection(0, 2)),
            )
        }

        fn mount(&self, state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            let selectable =
                SelectableText::new(LabelContent::new("aé status", 75).unwrap(), state.0.read())
                    .unwrap()
                    .mount(ui, root.0)
                    .unwrap();
            self.reference.set(Some(selectable));

            let invalid =
                SelectableText::new(LabelContent::new("aé status", 75).unwrap(), state.1.read())
                    .unwrap()
                    .mount(ui, root.0);
            if let Err(error) = invalid {
                *self.mount_error.borrow_mut() = Some(error.to_string());
            }
            root
        }

        fn action(&self, _: &mut Self::State, _: Self::Action, _: &mut UpdateContext<'_, Self>) {}
    }

    #[test]
    fn mounted_text_is_focusable_read_only_and_advertises_only_selection_actions() {
        let reference = Rc::new(Cell::new(None));
        let mount_error = Rc::new(RefCell::new(None));
        let runtime = ViewRuntime::from_component(Fixture {
            reference: reference.clone(),
            mount_error: mount_error.clone(),
        })
        .unwrap();
        let selectable = reference.get().expect("selectable-text reference");
        assert!(mount_error.borrow().is_some());
        assert_eq!(selectable.content_revision(), 75);
        assert_eq!(
            runtime.ui().kinds.get(selectable.node()),
            Some(&NodeKind::Text)
        );

        let visual = runtime.ui().texts.get(selectable.node()).unwrap();
        assert_eq!(runtime.ui().string(visual.content), Some("aé status"));
        assert_eq!(visual.revision, 75);
        let semantic = runtime.ui().semantics.get(selectable.node()).unwrap();
        assert_eq!(semantic.role, SemanticRole::Text);
        assert_eq!(semantic.name, SemanticName::Text(visual.content));
        assert_eq!(semantic.value, SemanticValue::Text(visual.content));
        assert_eq!(semantic.participation, SemanticParticipation::Node);
        assert!(semantic.state.read_only);
        assert!(semantic.state.focusable);
        assert!(!semantic.state.disabled);
        assert!(semantic.actions.contains(SemanticAction::Focus));
        assert!(semantic.actions.contains(SemanticAction::SetSelection));
        assert!(!semantic.actions.contains(SemanticAction::SetText));
        assert_eq!(
            runtime
                .ui()
                .interactions
                .get(selectable.node())
                .unwrap()
                .flags,
            InteractionFlags::default()
        );
        assert!(
            runtime
                .ui()
                .interactions
                .get(selectable.node())
                .unwrap()
                .focusable
        );
    }
}

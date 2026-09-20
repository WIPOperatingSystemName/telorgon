//! Controlled keyed Tier A radio group and item behavior.

use std::cell::RefCell;
use std::collections::HashSet;
use std::hash::Hash;
use std::rc::Rc;

use crate::foundation::{ColorRgba8, EdgeInsets};
use crate::input::{
    ChangeSource, CompositeChange, CompositeEdgeBehavior, CompositeError, CompositeItem,
    CompositeNavigationCommand, CompositeNavigationPolicy, CompositeOrientation,
    CompositeSelectionBehavior, CompositeStateMachine, DisabledItemPolicy, WritingDirection,
};
use crate::runtime::{Read, RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    Background, Border, BoxStyle, ControlHandle, CornerRadii, Flow, LayoutStyle, Property,
    SemanticActions, SemanticName, SemanticNode, SemanticRelationship, SemanticRelationshipKind,
    SemanticRole, SemanticState, SizeRule, SizeRule2D, UiNodeId,
};

use crate::components::application::{
    ButtonInteractionState, ButtonStyleState, DensityClass, DensityMetrics, ValueChange,
};

/// One stable-keyed labelled option in canonical group order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RadioItem<K> {
    key: K,
    label: String,
    enabled: bool,
}

impl<K> RadioItem<K> {
    pub fn new(key: K, label: impl Into<String>) -> Result<Self, RadioItemError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(RadioItemError::MissingAccessibleName);
        }
        Ok(Self {
            key,
            label,
            enabled: true,
        })
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn key(&self) -> &K {
        &self.key
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RadioItemError {
    MissingAccessibleName,
}

impl std::fmt::Display for RadioItemError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("radio item accessible name is empty")
    }
}

impl std::error::Error for RadioItemError {}

/// One neutral composite transition plus any controlled selection proposal it produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RadioGroupTransition<K> {
    pub change: CompositeChange<K>,
    pub selection: Option<ValueChange<Option<K>>>,
}

/// Radio-specific wrapper over the single neutral composite navigation owner.
#[derive(Clone, Debug)]
pub struct RadioGroupBehavior<K> {
    composite: CompositeStateMachine<K>,
}

impl<K> RadioGroupBehavior<K>
where
    K: Copy + Eq + Hash,
{
    pub fn new(
        items: impl IntoIterator<Item = CompositeItem<K>>,
        selected: Option<K>,
        policy: CompositeNavigationPolicy,
    ) -> Result<Self, CompositeError<K>> {
        let mut composite = CompositeStateMachine::new(policy);
        composite.update_items(items)?;
        composite.enter(selected)?;
        Ok(Self { composite })
    }

    pub fn standard(
        items: impl IntoIterator<Item = CompositeItem<K>>,
        selected: Option<K>,
    ) -> Result<Self, CompositeError<K>> {
        Self::new(items, selected, standard_radio_policy())
    }

    pub fn active_descendant(&self) -> Option<K> {
        self.composite.active_descendant()
    }

    pub fn update_items(
        &mut self,
        items: impl IntoIterator<Item = CompositeItem<K>>,
    ) -> Result<CompositeChange<K>, CompositeError<K>> {
        self.composite.update_items(items)
    }

    pub fn navigate(
        &mut self,
        command: CompositeNavigationCommand,
        direction: WritingDirection,
    ) -> Result<RadioGroupTransition<K>, CompositeError<K>> {
        let change = self.composite.navigate(command, direction)?;
        let selection = match change {
            CompositeChange::Highlighted {
                selection_request: Some(request),
                ..
            } => Some(ValueChange::committed(Some(request.key), request.source)),
            _ => None,
        };
        Ok(RadioGroupTransition { change, selection })
    }

    pub fn request_active_selection(
        &mut self,
        source: ChangeSource,
    ) -> Result<ValueChange<Option<K>>, CompositeError<K>> {
        let request = self.composite.request_active_selection(source)?;
        Ok(ValueChange::committed(Some(request.key), request.source))
    }
}

pub const fn standard_radio_policy() -> CompositeNavigationPolicy {
    CompositeNavigationPolicy {
        orientation: CompositeOrientation::Both,
        edge_behavior: CompositeEdgeBehavior::Wrap,
        disabled_items: DisabledItemPolicy::Skip,
        selection: CompositeSelectionBehavior::FollowsHighlight,
    }
}

/// Visual slots for one radio item value and interaction state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RadioItemVisualStyle {
    pub container: BoxStyle,
    pub indicator: BoxStyle,
    pub dot: BoxStyle,
    pub label_color: ColorRgba8,
    pub label_size: f32,
    pub gap: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RadioItemStateStyle {
    pub resting: RadioItemVisualStyle,
    pub hovered: Option<RadioItemVisualStyle>,
    pub focused: Option<RadioItemVisualStyle>,
    pub pressed: Option<RadioItemVisualStyle>,
    pub disabled: Option<RadioItemVisualStyle>,
}

impl RadioItemStateStyle {
    const fn resolve(
        self,
        state: ButtonInteractionState,
    ) -> (ButtonStyleState, RadioItemVisualStyle) {
        let resolved_state = ButtonStyleState::resolve(state);
        let visual = match resolved_state {
            ButtonStyleState::Disabled => self.disabled,
            ButtonStyleState::Pressed => self.pressed,
            ButtonStyleState::Focused => self.focused,
            ButtonStyleState::Hovered => self.hovered,
            ButtonStyleState::Busy | ButtonStyleState::Resting => Some(self.resting),
        };
        (
            resolved_state,
            match visual {
                Some(visual) => visual,
                None => self.resting,
            },
        )
    }
}

/// Typed group and selected/unselected item styles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RadioStyle {
    pub group: BoxStyle,
    pub flow: Flow,
    pub group_gap: f32,
    pub unselected: RadioItemStateStyle,
    pub selected: RadioItemStateStyle,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedRadioItemStyle {
    pub selected: bool,
    pub state: ButtonStyleState,
    pub visual: RadioItemVisualStyle,
}

impl RadioStyle {
    pub const fn resolve_item(
        self,
        selected: bool,
        state: ButtonInteractionState,
    ) -> ResolvedRadioItemStyle {
        let (resolved_state, visual) = if selected {
            self.selected.resolve(state)
        } else {
            self.unselected.resolve(state)
        };
        ResolvedRadioItemStyle {
            selected,
            state: resolved_state,
            visual,
        }
    }
}

impl Default for RadioStyle {
    fn default() -> Self {
        fn visual(selected: bool, background: Background, opacity: u8) -> RadioItemVisualStyle {
            RadioItemVisualStyle {
                container: BoxStyle {
                    min_size: SizeRule2D {
                        width: SizeRule::Logical(32.0),
                        height: SizeRule::Logical(32.0),
                    },
                    padding: EdgeInsets::all(5.0),
                    decoration: crate::ui::BoxDecoration {
                        background,
                        corner_radii: CornerRadii::all(4.0),
                        ..crate::ui::BoxDecoration::default()
                    },
                    ..BoxStyle::default()
                },
                indicator: BoxStyle {
                    width: SizeRule::Logical(18.0),
                    height: SizeRule::Logical(18.0),
                    decoration: crate::ui::BoxDecoration {
                        border: Border::all(1.0, ColorRgba8::rgba(109, 119, 139, opacity)),
                        corner_radii: CornerRadii::all(9.0),
                        ..crate::ui::BoxDecoration::default()
                    },
                    ..BoxStyle::default()
                },
                dot: BoxStyle {
                    width: SizeRule::Logical(if selected { 10.0 } else { 0.0 }),
                    height: SizeRule::Logical(if selected { 10.0 } else { 0.0 }),
                    decoration: crate::ui::BoxDecoration {
                        background: if selected {
                            Background::Color(ColorRgba8::rgba(76, 132, 235, opacity))
                        } else {
                            Background::None
                        },
                        corner_radii: CornerRadii::all(5.0),
                        ..crate::ui::BoxDecoration::default()
                    },
                    ..BoxStyle::default()
                },
                label_color: ColorRgba8::rgba(235, 238, 244, opacity),
                label_size: 14.0,
                gap: 8.0,
            }
        }

        fn state_style(selected: bool) -> RadioItemStateStyle {
            RadioItemStateStyle {
                resting: visual(selected, Background::None, 255),
                hovered: Some(visual(
                    selected,
                    Background::Color(ColorRgba8::rgba(69, 78, 96, 90)),
                    255,
                )),
                focused: Some(visual(
                    selected,
                    Background::Color(ColorRgba8::rgba(66, 91, 139, 110)),
                    255,
                )),
                pressed: Some(visual(
                    selected,
                    Background::Color(ColorRgba8::rgba(46, 55, 72, 140)),
                    255,
                )),
                disabled: Some(visual(selected, Background::None, 180)),
            }
        }

        Self {
            group: BoxStyle::default(),
            flow: Flow::Vertical,
            group_gap: 4.0,
            unselected: state_style(false),
            selected: state_style(true),
        }
    }
}

/// Immutable configuration for a parent-controlled radio group.
#[derive(Clone, Debug, PartialEq)]
pub struct RadioGroup<K: 'static> {
    label: String,
    selected: Read<Option<K>>,
    items: Vec<RadioItem<K>>,
    policy: CompositeNavigationPolicy,
    density: DensityMetrics,
    style: RadioStyle,
}

impl<K> RadioGroup<K>
where
    K: Copy + Eq + Hash + 'static,
{
    pub fn new(
        label: impl Into<String>,
        selected: Read<Option<K>>,
        items: impl IntoIterator<Item = RadioItem<K>>,
    ) -> Result<Self, RadioGroupError<K>> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(RadioGroupError::MissingAccessibleName);
        }
        let items: Vec<_> = items.into_iter().collect();
        let mut keys = HashSet::with_capacity(items.len());
        for item in &items {
            if !keys.insert(item.key) {
                return Err(RadioGroupError::DuplicateKey(item.key));
            }
        }
        Ok(Self {
            label,
            selected,
            items,
            policy: standard_radio_policy(),
            density: DensityMetrics::baseline(DensityClass::Standard),
            style: RadioStyle::default(),
        })
    }

    pub fn policy(mut self, policy: CompositeNavigationPolicy) -> Self {
        self.policy = policy;
        self
    }

    pub fn density(mut self, density: DensityMetrics) -> Self {
        self.density = density;
        self
    }

    pub fn style(mut self, style: RadioStyle) -> Self {
        self.style = style;
        self
    }

    pub fn behavior(
        &self,
        selected: Option<K>,
    ) -> Result<RadioGroupBehavior<K>, CompositeError<K>> {
        RadioGroupBehavior::new(
            self.items.iter().map(|item| CompositeItem {
                key: item.key,
                enabled: item.enabled,
            }),
            selected,
            self.policy,
        )
    }

    pub fn mount<Action, Map>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
        map: Map,
    ) -> RuntimeResult<RadioGroupRef<K>>
    where
        K: 'static,
        Action: 'static,
        Map: Fn(ValueChange<Option<K>>) -> Action + 'static,
    {
        let selected = ui.read(self.selected)?;
        let behavior =
            Rc::new(RefCell::new(self.behavior(selected).map_err(|_| {
                RuntimeError::new("invalid radio group composite state")
            })?));
        let active = behavior.borrow().active_descendant();
        let minimum = self.density.effective_minimum();
        let group_layout = LayoutStyle {
            flow: self.style.flow,
            gap: self.style.group_gap,
            ..LayoutStyle::default()
        };
        let items = self.items.clone();
        let mut mounted = Vec::with_capacity(items.len());
        let group = ui
            .foundation()
            .button_node_under(host, self.style.group, |writer| {
                writer.container(BoxStyle::default(), group_layout, |writer| {
                    for item in items {
                        let state = ButtonInteractionState::resting(item.enabled, false);
                        let mut visual = self
                            .style
                            .resolve_item(selected == Some(item.key), state)
                            .visual;
                        visual.container.min_size = SizeRule2D {
                            width: SizeRule::Logical(minimum.width()),
                            height: SizeRule::Logical(minimum.height()),
                        };
                        let label = item.label.clone();
                        let control = writer.action_node(visual.container, false, move |writer| {
                            writer.container(
                                BoxStyle::default(),
                                LayoutStyle {
                                    flow: Flow::Horizontal,
                                    gap: visual.gap,
                                    ..LayoutStyle::default()
                                },
                                move |writer| {
                                    writer.container(
                                        visual.indicator,
                                        LayoutStyle::default(),
                                        move |writer| {
                                            if selected == Some(item.key) {
                                                writer.container(
                                                    visual.dot,
                                                    LayoutStyle::default(),
                                                    |_| {},
                                                );
                                            }
                                        },
                                    );
                                    writer.text(&label, visual.label_color, visual.label_size);
                                },
                            );
                        });
                        mounted.push((item, control));
                    }
                });
            })
            .ok_or_else(|| RuntimeError::new("application radio-group host is stale"))?;

        let map: Rc<dyn Fn(ValueChange<Option<K>>) -> Action> = Rc::new(map);
        let mut item_refs = Vec::with_capacity(mounted.len());
        for (item, control) in &mounted {
            let name = ui.foundation().intern(&item.label);
            let mut actions = SemanticActions::NONE;
            if item.enabled {
                actions |= SemanticActions::ACTIVATE;
            }
            ui.foundation()
                .semantic_node(
                    control.node,
                    SemanticNode {
                        role: SemanticRole::Radio,
                        name: SemanticName::Text(name),
                        state: SemanticState {
                            disabled: !item.enabled,
                            selected: Some(selected == Some(item.key)),
                            ..SemanticState::default()
                        },
                        actions,
                        ..SemanticNode::default()
                    },
                )
                .map_err(|error| {
                    RuntimeError::new(format!("invalid radio item semantics: {error:?}"))
                })?;
            if !item.enabled {
                ui.foundation().disabled(control.node, true);
            } else {
                let key = item.key;
                let map = map.clone();
                ui.route_activation(control.node, move |activation| {
                    map(ValueChange::committed(Some(key), activation.source))
                })?;
            }
            item_refs.push(RadioItemRef {
                key: item.key,
                control: *control,
                enabled: item.enabled,
            });
        }

        let group_name = ui.foundation().intern(&self.label);
        let any_enabled = mounted.iter().any(|(item, _)| item.enabled);
        let mut relationships: Vec<_> = mounted
            .iter()
            .map(|(_, control)| SemanticRelationship {
                kind: SemanticRelationshipKind::Owns,
                target: control.node,
            })
            .collect();
        if let Some(active) = active.and_then(|key| {
            mounted
                .iter()
                .find_map(|(item, control)| (item.key == key).then_some(control.node))
        }) {
            relationships.push(SemanticRelationship {
                kind: SemanticRelationshipKind::ActiveDescendant,
                target: active,
            });
        }
        ui.foundation()
            .semantic_node(
                group.node,
                SemanticNode {
                    role: SemanticRole::RadioGroup,
                    name: SemanticName::Text(group_name),
                    state: SemanticState {
                        disabled: !any_enabled,
                        focusable: any_enabled,
                        ..SemanticState::default()
                    },
                    actions: if any_enabled {
                        SemanticActions::FOCUS | SemanticActions::ACTIVATE
                    } else {
                        SemanticActions::NONE
                    },
                    relationships,
                    ..SemanticNode::default()
                },
            )
            .map_err(|error| {
                RuntimeError::new(format!("invalid radio group semantics: {error:?}"))
            })?;

        if any_enabled {
            let route_behavior = behavior.clone();
            let route_map = map.clone();
            ui.route_activation_fallible(group.node, move |activation| {
                let change = route_behavior
                    .borrow_mut()
                    .request_active_selection(activation.source)
                    .map_err(|_| RuntimeError::new("radio group activation failed"))?;
                Ok(route_map(change))
            })?;
        }

        Ok(RadioGroupRef {
            group,
            items: item_refs,
            selected: self.selected,
            behavior,
        })
    }
}

#[derive(Clone, Debug)]
pub struct RadioGroupRef<K: 'static> {
    group: ControlHandle,
    items: Vec<RadioItemRef<K>>,
    selected: Read<Option<K>>,
    behavior: Rc<RefCell<RadioGroupBehavior<K>>>,
}

impl<K> RadioGroupRef<K>
where
    K: Copy + Eq + Hash + 'static,
{
    pub fn node(&self) -> UiNodeId {
        self.group.node
    }

    pub fn selected(&self) -> Read<Option<K>> {
        self.selected
    }

    pub fn items(&self) -> &[RadioItemRef<K>] {
        &self.items
    }

    pub fn active_descendant(&self) -> Option<K> {
        self.behavior.borrow().active_descendant()
    }

    pub fn navigate(
        &self,
        command: CompositeNavigationCommand,
        direction: WritingDirection,
    ) -> Result<RadioGroupTransition<K>, CompositeError<K>> {
        self.behavior.borrow_mut().navigate(command, direction)
    }

    pub fn request_active_selection(
        &self,
        source: ChangeSource,
    ) -> Result<ValueChange<Option<K>>, CompositeError<K>> {
        self.behavior.borrow_mut().request_active_selection(source)
    }

    pub const fn style(&self) -> Property<BoxStyle> {
        self.group.style
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RadioItemRef<K> {
    key: K,
    control: ControlHandle,
    enabled: bool,
}

impl<K: Copy> RadioItemRef<K> {
    pub const fn key(self) -> K {
        self.key
    }

    pub const fn node(self) -> UiNodeId {
        self.control.node
    }

    pub const fn is_enabled(self) -> bool {
        self.enabled
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RadioGroupError<K> {
    MissingAccessibleName,
    DuplicateKey(K),
}

impl<K: std::fmt::Debug> std::fmt::Display for RadioGroupError<K> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingAccessibleName => {
                formatter.write_str("radio group accessible name is empty")
            }
            Self::DuplicateKey(key) => write!(formatter, "duplicate radio item key: {key:?}"),
        }
    }
}

impl<K: std::fmt::Debug> std::error::Error for RadioGroupError<K> {}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use crate::runtime::{Component, CreateContext, State, UpdateContext, ViewRuntime};
    use crate::ui::{SemanticAction, SemanticRelationshipKind, UiRoot};

    use crate::components::application::ChangePhase;

    use super::*;

    fn item(key: u32, enabled: bool) -> RadioItem<u32> {
        RadioItem::new(key, format!("Option {key}"))
            .unwrap()
            .enabled(enabled)
    }

    #[test]
    fn names_and_duplicate_keys_are_rejected_before_mount() {
        assert_eq!(
            RadioItem::new(1_u32, " ").unwrap_err(),
            RadioItemError::MissingAccessibleName
        );

        struct Validate {
            checked: Rc<Cell<bool>>,
        }
        impl Component for Validate {
            type State = State<Option<u32>>;
            type Action = ();

            fn create(&self, context: &mut CreateContext<'_>) -> Self::State {
                context.state(None)
            }

            fn mount(&self, state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
                self.checked.set(matches!(
                    RadioGroup::new(" ", state.read(), [item(1, true)]),
                    Err(RadioGroupError::MissingAccessibleName)
                ));
                assert!(matches!(
                    RadioGroup::new("Mode", state.read(), [item(1, true), item(1, false)]),
                    Err(RadioGroupError::DuplicateKey(1))
                ));
                ui.foundation()
                    .root(BoxStyle::default(), LayoutStyle::default(), |_| {})
            }

            fn action(
                &self,
                _state: &mut Self::State,
                _action: Self::Action,
                _context: &mut UpdateContext<'_, Self>,
            ) {
            }
        }
        let checked = Rc::new(Cell::new(false));
        ViewRuntime::from_component(Validate {
            checked: checked.clone(),
        })
        .unwrap();
        assert!(checked.get());
    }

    #[test]
    fn neutral_composite_owns_directional_focus_and_emits_controlled_selection() {
        let mut behavior = RadioGroupBehavior::standard(
            [
                CompositeItem {
                    key: 1_u32,
                    enabled: true,
                },
                CompositeItem {
                    key: 2,
                    enabled: false,
                },
                CompositeItem {
                    key: 3,
                    enabled: true,
                },
            ],
            Some(1),
        )
        .unwrap();
        assert_eq!(behavior.active_descendant(), Some(1));
        let moved = behavior
            .navigate(
                CompositeNavigationCommand::Right,
                WritingDirection::LeftToRight,
            )
            .unwrap();
        assert_eq!(behavior.active_descendant(), Some(3));
        assert_eq!(
            moved.selection,
            Some(ValueChange::new(
                Some(3),
                ChangePhase::Commit,
                ChangeSource::Directional,
            ))
        );
        assert_eq!(
            behavior
                .request_active_selection(ChangeSource::Accessibility)
                .unwrap(),
            ValueChange::committed(Some(3), ChangeSource::Accessibility)
        );
    }

    #[test]
    fn reorder_preserves_active_key_and_removal_uses_surviving_successor() {
        let mut behavior = RadioGroupBehavior::standard(
            [1_u32, 2, 3].map(|key| CompositeItem { key, enabled: true }),
            Some(2),
        )
        .unwrap();
        behavior
            .update_items([3_u32, 2, 1].map(|key| CompositeItem { key, enabled: true }))
            .unwrap();
        assert_eq!(behavior.active_descendant(), Some(2));
        behavior
            .update_items([3_u32, 1].map(|key| CompositeItem { key, enabled: true }))
            .unwrap();
        assert_eq!(behavior.active_descendant(), Some(1));
    }

    struct MountedRadio {
        group: Rc<RefCell<Option<RadioGroupRef<u32>>>>,
        requests: Rc<RefCell<Vec<ValueChange<Option<u32>>>>>,
    }

    struct MountedState {
        selected: State<Option<u32>>,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum MountedAction {
        Requested(ValueChange<Option<u32>>),
    }

    impl Component for MountedRadio {
        type State = MountedState;
        type Action = MountedAction;

        fn create(&self, context: &mut CreateContext<'_>) -> Self::State {
            MountedState {
                selected: context.state(Some(1)),
            }
        }

        fn mount(&self, state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            let group = RadioGroup::new(
                "Mode",
                state.selected.read(),
                [item(1, true), item(2, false), item(3, true)],
            )
            .unwrap()
            .density(DensityMetrics::baseline(DensityClass::Touch))
            .mount(ui, root.0, MountedAction::Requested)
            .unwrap();
            *self.group.borrow_mut() = Some(group);
            root
        }

        fn action(
            &self,
            _state: &mut Self::State,
            action: Self::Action,
            _context: &mut UpdateContext<'_, Self>,
        ) {
            match action {
                MountedAction::Requested(change) => self.requests.borrow_mut().push(change),
            }
        }
    }

    #[test]
    fn mounted_group_has_one_tab_stop_semantics_and_source_preserving_item_routes() {
        let group = Rc::new(RefCell::new(None));
        let requests = Rc::new(RefCell::new(Vec::new()));
        let mut runtime = ViewRuntime::from_component(MountedRadio {
            group: group.clone(),
            requests: requests.clone(),
        })
        .unwrap();
        let borrowed = group.borrow();
        let group = borrowed.as_ref().unwrap();
        let group_node = group.node();
        let items = group.items();

        assert!(runtime.ui().interactions.get(group_node).unwrap().focusable);
        assert!(items.iter().all(|item| {
            !runtime
                .ui()
                .interactions
                .get(item.node())
                .is_some_and(|interaction| interaction.focusable)
        }));
        let semantics = runtime.ui().semantics.get(group_node).unwrap();
        assert_eq!(semantics.role, SemanticRole::RadioGroup);
        assert_eq!(semantics.relationships.len(), 4);
        assert_eq!(
            semantics.relationships.last().unwrap().kind,
            SemanticRelationshipKind::ActiveDescendant
        );
        assert!(semantics.actions.contains(SemanticAction::Activate));

        let first = runtime.ui().semantics.get(items[0].node()).unwrap();
        assert_eq!(first.role, SemanticRole::Radio);
        assert_eq!(first.state.selected, Some(true));
        let disabled = runtime.ui().semantics.get(items[1].node()).unwrap();
        assert!(disabled.state.disabled);
        assert!(disabled.effective_actions().is_empty());
        assert_eq!(
            runtime
                .ui()
                .box_styles
                .get(items[0].node())
                .unwrap()
                .min_size,
            SizeRule2D {
                width: SizeRule::Logical(44.0),
                height: SizeRule::Logical(44.0),
            }
        );

        assert!(runtime.dispatch_activation(items[2].node(), ChangeSource::Pointer));
        assert!(runtime.dispatch_activation(items[2].node(), ChangeSource::Accessibility));
        assert!(!runtime.dispatch_activation(items[1].node(), ChangeSource::Pointer));
        assert!(runtime.dispatch_action(group_node));
        assert_eq!(
            &*requests.borrow(),
            &[
                ValueChange::committed(Some(3), ChangeSource::Pointer),
                ValueChange::committed(Some(3), ChangeSource::Accessibility),
                ValueChange::committed(Some(1), ChangeSource::Programmatic),
            ]
        );
    }

    #[test]
    fn mounted_ref_exposes_neutral_navigation_without_committing_selection() {
        let group = Rc::new(RefCell::new(None));
        let requests = Rc::new(RefCell::new(Vec::new()));
        let runtime = ViewRuntime::from_component(MountedRadio {
            group: group.clone(),
            requests: requests.clone(),
        })
        .unwrap();
        let moved = group
            .borrow()
            .as_ref()
            .unwrap()
            .navigate(
                CompositeNavigationCommand::Down,
                WritingDirection::LeftToRight,
            )
            .unwrap();
        assert_eq!(
            moved.selection,
            Some(ValueChange::committed(Some(3), ChangeSource::Directional))
        );
        assert!(requests.borrow().is_empty());
        assert_eq!(
            runtime
                .ui()
                .semantics
                .get(group.borrow().as_ref().unwrap().items()[0].node())
                .unwrap()
                .state
                .selected,
            Some(true)
        );
    }
}

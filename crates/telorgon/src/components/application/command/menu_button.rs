//! Canonical button that emits an unapplied root-menu open request.

use std::collections::HashSet;
use std::fmt;
use std::hash::Hash;

use crate::input::{Activation, ChangeSource, CompositeItem};
use crate::runtime::{RuntimeResult, Ui};
use crate::ui::{OverlayAnchor, Property, SemanticNode, StringId, UiNodeId};

use super::{MenuOpenRequest, MenuOpeningFocus};
use crate::components::application::{
    Button, ButtonBehavior, ButtonBusyPolicy, ButtonError, ButtonInteractionState, ButtonRef,
    ButtonStyle, DensityMetrics,
};

/// Root-menu request plus the exact activation source that produced it.
#[derive(Clone, Debug)]
pub struct MenuButtonOpenRequest<K> {
    source: ChangeSource,
    menu: MenuOpenRequest<K>,
}

impl<K> MenuButtonOpenRequest<K> {
    pub const fn source(&self) -> ChangeSource {
        self.source
    }

    pub const fn menu(&self) -> &MenuOpenRequest<K> {
        &self.menu
    }

    pub fn into_menu(self) -> MenuOpenRequest<K> {
        self.menu
    }
}

/// Immutable configuration for one labelled root-menu trigger.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuButton<K> {
    button: Button,
    items: Vec<CompositeItem<K>>,
    selected: Option<K>,
    opening_focus: MenuOpeningFocus,
    expanded: bool,
}

impl<K> MenuButton<K>
where
    K: Copy + Eq + Hash,
{
    pub fn new(
        label: impl Into<String>,
        items: impl IntoIterator<Item = CompositeItem<K>>,
    ) -> Result<Self, MenuButtonError<K>> {
        let button = Button::new(label).map_err(MenuButtonError::from)?;
        let items = items.into_iter().collect::<Vec<_>>();
        if items.is_empty() {
            return Err(MenuButtonError::EmptyItems);
        }
        let mut keys = HashSet::with_capacity(items.len());
        for item in &items {
            if !keys.insert(item.key) {
                return Err(MenuButtonError::DuplicateItem(item.key));
            }
        }
        Ok(Self {
            button,
            items,
            selected: None,
            opening_focus: MenuOpeningFocus::SelectedOrFirst,
            expanded: false,
        })
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.button = self.button.enabled(enabled);
        self
    }

    pub fn busy(mut self, busy: bool) -> Self {
        self.button = self.button.busy(busy);
        self
    }

    pub fn busy_policy(mut self, policy: ButtonBusyPolicy) -> Self {
        self.button = self.button.busy_policy(policy);
        self
    }

    pub fn density(mut self, density: DensityMetrics) -> Self {
        self.button = self.button.density(density);
        self
    }

    pub fn style(mut self, style: ButtonStyle) -> Self {
        self.button = self.button.style(style);
        self
    }

    pub fn selected(mut self, selected: K) -> Result<Self, MenuButtonError<K>> {
        if !self.items.iter().any(|item| item.key == selected) {
            return Err(MenuButtonError::UnknownSelected(selected));
        }
        self.selected = Some(selected);
        Ok(self)
    }

    pub fn opening_focus(mut self, opening_focus: MenuOpeningFocus) -> Self {
        self.opening_focus = opening_focus;
        self
    }

    /// Caller-published snapshot of the existing menu controller's root state.
    pub fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = expanded;
        self
    }

    pub fn behavior(&self) -> ButtonBehavior {
        self.button.behavior()
    }

    pub fn items(&self) -> &[CompositeItem<K>] {
        &self.items
    }

    pub const fn selected_key(&self) -> Option<K> {
        self.selected
    }

    pub const fn opening_focus_policy(&self) -> MenuOpeningFocus {
        self.opening_focus
    }

    pub const fn is_expanded(&self) -> bool {
        self.expanded
    }

    pub fn semantic_node(&self, name: StringId, state: ButtonInteractionState) -> SemanticNode {
        let mut semantic = self.button.semantic_node(name, state);
        semantic.state.expanded = Some(self.expanded);
        semantic
    }

    pub fn open_request(&self, anchor: UiNodeId, source: ChangeSource) -> MenuButtonOpenRequest<K> {
        build_open_request(
            anchor,
            self.items.clone(),
            self.selected,
            self.opening_focus,
            source,
        )
    }

    pub fn mount<Action, Map>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
        map: Map,
    ) -> RuntimeResult<MenuButtonRef>
    where
        Action: 'static,
        K: 'static,
        Map: Fn(MenuButtonOpenRequest<K>) -> Action + 'static,
    {
        let expanded = self.expanded;
        let items = self.items.clone();
        let selected = self.selected;
        let opening_focus = self.opening_focus;
        let button = self.button.mount_with_semantics(
            ui,
            host,
            move |semantic| semantic.state.expanded = Some(expanded),
            move |anchor, activation: Activation| {
                map(build_open_request(
                    anchor,
                    items.clone(),
                    selected,
                    opening_focus,
                    activation.source,
                ))
            },
        )?;
        Ok(MenuButtonRef { button })
    }
}

fn build_open_request<K>(
    anchor: UiNodeId,
    items: Vec<CompositeItem<K>>,
    selected: Option<K>,
    opening_focus: MenuOpeningFocus,
    source: ChangeSource,
) -> MenuButtonOpenRequest<K> {
    let mut menu =
        MenuOpenRequest::root(OverlayAnchor::Node(anchor), items).opening_focus(opening_focus);
    if let Some(selected) = selected {
        menu = menu.selected(selected);
    }
    MenuButtonOpenRequest { source, menu }
}

/// Focused advanced reference returned by menu-button mounting.
#[derive(Clone, Copy, Debug)]
pub struct MenuButtonRef {
    button: ButtonRef,
}

impl MenuButtonRef {
    pub const fn node(self) -> UiNodeId {
        self.button.node()
    }

    pub const fn enabled(self) -> Property<bool> {
        self.button.enabled()
    }

    pub const fn style(self) -> Property<crate::ui::BoxStyle> {
        self.button.style()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuButtonError<K> {
    MissingAccessibleName,
    EmptyItems,
    DuplicateItem(K),
    UnknownSelected(K),
}

impl<K> From<ButtonError> for MenuButtonError<K> {
    fn from(error: ButtonError) -> Self {
        match error {
            ButtonError::MissingAccessibleName => Self::MissingAccessibleName,
        }
    }
}

impl<K: fmt::Debug> fmt::Display for MenuButtonError<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid menu button: {self:?}")
    }
}

impl<K: fmt::Debug> std::error::Error for MenuButtonError<K> {}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use crate::input::{ActivationInput, ActivationTransition};
    use crate::runtime::{Component, CreateContext, UpdateContext, ViewRuntime};
    use crate::ui::{BoxStyle, LayoutStyle, SemanticAction, SemanticName, UiRoot};

    use crate::components::application::{DensityClass, DensityMetrics};

    use super::*;

    fn item(key: u8, enabled: bool) -> CompositeItem<u8> {
        CompositeItem { key, enabled }
    }

    #[test]
    fn construction_validates_name_items_and_selected_key() {
        assert_eq!(
            MenuButton::new(" ", [item(1, true)]),
            Err(MenuButtonError::MissingAccessibleName)
        );
        assert_eq!(
            MenuButton::<u8>::new("Actions", []),
            Err(MenuButtonError::EmptyItems)
        );
        assert_eq!(
            MenuButton::new("Actions", [item(1, true), item(1, false)]),
            Err(MenuButtonError::DuplicateItem(1))
        );
        assert_eq!(
            MenuButton::new("Actions", [item(1, true)])
                .unwrap()
                .selected(2),
            Err(MenuButtonError::UnknownSelected(2))
        );
    }

    #[test]
    fn behavior_and_request_reuse_button_activation_and_preserve_open_inputs() {
        let button = MenuButton::new("Actions", [item(1, true), item(2, false)])
            .unwrap()
            .selected(2)
            .unwrap()
            .opening_focus(MenuOpeningFocus::None)
            .expanded(true);
        let mut behavior = button.behavior();
        assert_eq!(
            behavior.handle(ActivationInput::EnterDown { repeat: false }),
            crate::input::ActivationOutcome {
                transition: ActivationTransition::Activated(Activation {
                    source: ChangeSource::Keyboard,
                }),
                ..crate::input::ActivationOutcome::default()
            }
        );
        let anchor = UiNodeId::new(8, 1);
        let request = button.open_request(anchor, ChangeSource::Accessibility);
        assert_eq!(request.source(), ChangeSource::Accessibility);
        assert_eq!(request.menu().anchor, OverlayAnchor::Node(anchor));
        assert_eq!(request.menu().items, [item(1, true), item(2, false)]);
        assert_eq!(request.menu().selected, Some(2));
        assert_eq!(request.menu().opening_focus, MenuOpeningFocus::None);
        let semantic =
            button.semantic_node(StringId(1), ButtonInteractionState::resting(true, false));
        assert_eq!(semantic.state.expanded, Some(true));
    }

    struct Fixture {
        node: Rc<Cell<Option<UiNodeId>>>,
        received: Rc<RefCell<Vec<MenuButtonOpenRequest<u8>>>>,
    }

    impl Component for Fixture {
        type State = ();
        type Action = MenuButtonOpenRequest<u8>;

        fn create(&self, _: &mut CreateContext<'_>) -> Self::State {}

        fn mount(&self, _: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            let button = MenuButton::new("More actions", [item(4, true), item(5, false)])
                .unwrap()
                .selected(4)
                .unwrap()
                .expanded(true)
                .density(DensityMetrics::baseline(DensityClass::Touch))
                .mount(ui, root.0, |request| request)
                .unwrap();
            self.node.set(Some(button.node()));
            root
        }

        fn action(
            &self,
            _: &mut Self::State,
            action: Self::Action,
            _: &mut UpdateContext<'_, Self>,
        ) {
            self.received.borrow_mut().push(action);
        }
    }

    #[test]
    fn mounted_button_routes_stable_anchor_and_source_without_opening_a_menu() {
        let node = Rc::new(Cell::new(None));
        let received = Rc::new(RefCell::new(Vec::new()));
        let mut runtime = ViewRuntime::from_component(Fixture {
            node: node.clone(),
            received: received.clone(),
        })
        .unwrap();
        let node = node.get().unwrap();
        let mounted_count = runtime.ui().nodes.alive().len();

        let semantic = runtime.ui().semantics.get(node).unwrap();
        let SemanticName::Text(name) = semantic.name else {
            panic!("menu button must publish a text semantic name");
        };
        assert_eq!(runtime.ui().string(name), Some("More actions"));
        assert_eq!(semantic.state.expanded, Some(true));
        assert!(semantic.actions.contains(SemanticAction::Activate));
        assert_eq!(
            runtime.ui().box_styles.get(node).unwrap().min_size.width,
            crate::ui::SizeRule::Logical(44.0)
        );

        assert!(runtime.dispatch_activation(node, ChangeSource::Pointer));
        assert!(runtime.dispatch_activation(node, ChangeSource::Accessibility));
        assert_eq!(runtime.ui().nodes.alive().len(), mounted_count);
        let received = received.borrow();
        assert_eq!(received.len(), 2);
        assert_eq!(received[0].source(), ChangeSource::Pointer);
        assert_eq!(received[1].source(), ChangeSource::Accessibility);
        for request in received.iter() {
            assert_eq!(request.menu().anchor, OverlayAnchor::Node(node));
            assert_eq!(request.menu().parent, None);
            assert_eq!(request.menu().selected, Some(4));
            assert_eq!(request.menu().items, [item(4, true), item(5, false)]);
        }
    }
}

//! Accessible Tier A icon-only button built on the shared button contract.

use crate::assets::{Icon, IconAsset};
use crate::foundation::{ColorRgba8, EdgeInsets};
use crate::input::Activation;
use crate::runtime::{RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    Background, BoxStyle, ControlHandle, CornerRadii, ImageId, Property, SemanticNode, SizeRule,
    SizeRule2D, StringId, UiNodeId,
};

use crate::components::application::{
    Button, ButtonBehavior, ButtonBusyPolicy, ButtonError, ButtonInteractionState,
    ButtonStyleState, DensityMetrics,
};

/// Decorative icon artwork. It deliberately carries no accessible name or action behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IconArtwork {
    image: ImageId,
    tint: Option<ColorRgba8>,
}

impl IconArtwork {
    pub const fn from_image(image: ImageId) -> Self {
        Self { image, tint: None }
    }

    pub const fn image(self) -> ImageId {
        self.image
    }

    /// Recolors the artwork from its alpha mask while preserving transparency.
    pub const fn tint(mut self, color: ColorRgba8) -> Self {
        self.tint = Some(color);
        self
    }

    pub const fn without_tint(mut self) -> Self {
        self.tint = None;
        self
    }

    pub const fn tint_color(self) -> Option<ColorRgba8> {
        self.tint
    }
}

impl From<ImageId> for IconArtwork {
    fn from(image: ImageId) -> Self {
        Self::from_image(image)
    }
}

impl From<IconAsset> for IconArtwork {
    fn from(icon: IconAsset) -> Self {
        Self::from_image(icon.image_id())
    }
}

impl From<Icon> for IconArtwork {
    fn from(icon: Icon) -> Self {
        let artwork = Self::from_image(icon.image_id());
        match icon.tint_color() {
            Some(color) => artwork.tint(color),
            None => artwork,
        }
    }
}

/// Validated geometry and opacity for the decorative icon slot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IconSlotStyle {
    logical_size: f32,
    opacity: f32,
}

impl IconSlotStyle {
    pub fn new(logical_size: f32, opacity: f32) -> Result<Self, IconSlotStyleError> {
        if !logical_size.is_finite() || logical_size <= 0.0 {
            return Err(IconSlotStyleError::InvalidLogicalSize);
        }
        if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
            return Err(IconSlotStyleError::InvalidOpacity);
        }
        Ok(Self {
            logical_size,
            opacity,
        })
    }

    pub const fn logical_size(self) -> f32 {
        self.logical_size
    }

    pub const fn opacity(self) -> f32 {
        self.opacity
    }
}

impl Default for IconSlotStyle {
    fn default() -> Self {
        Self {
            logical_size: 18.0,
            opacity: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconSlotStyleError {
    InvalidLogicalSize,
    InvalidOpacity,
}

impl std::fmt::Display for IconSlotStyleError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "invalid icon slot style: {self:?}")
    }
}

impl std::error::Error for IconSlotStyleError {}

/// Named foundation slots for one resolved icon-button visual state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IconButtonVisualStyle {
    pub container: BoxStyle,
    pub icon: IconSlotStyle,
}

/// Typed icon-button style. State priority is owned by [`ButtonStyleState`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IconButtonStyle {
    pub resting: IconButtonVisualStyle,
    pub hovered: Option<IconButtonVisualStyle>,
    pub focused: Option<IconButtonVisualStyle>,
    pub pressed: Option<IconButtonVisualStyle>,
    pub busy: Option<IconButtonVisualStyle>,
    pub disabled: Option<IconButtonVisualStyle>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedIconButtonStyle {
    pub state: ButtonStyleState,
    pub visual: IconButtonVisualStyle,
}

impl IconButtonStyle {
    pub const fn resolve(self, state: ButtonInteractionState) -> ResolvedIconButtonStyle {
        let resolved_state = ButtonStyleState::resolve(state);
        let visual = match resolved_state {
            ButtonStyleState::Disabled => self.disabled,
            ButtonStyleState::Busy => self.busy,
            ButtonStyleState::Pressed => self.pressed,
            ButtonStyleState::Focused => self.focused,
            ButtonStyleState::Hovered => self.hovered,
            ButtonStyleState::Resting => Some(self.resting),
        };
        ResolvedIconButtonStyle {
            state: resolved_state,
            visual: match visual {
                Some(visual) => visual,
                None => self.resting,
            },
        }
    }
}

impl Default for IconButtonStyle {
    fn default() -> Self {
        let container = |color| BoxStyle {
            min_size: SizeRule2D {
                width: SizeRule::Logical(32.0),
                height: SizeRule::Logical(32.0),
            },
            padding: EdgeInsets::all(7.0),
            decoration: crate::ui::BoxDecoration {
                background: Background::Color(color),
                corner_radii: CornerRadii::all(6.0),
                ..crate::ui::BoxDecoration::default()
            },
            ..BoxStyle::default()
        };
        let visual = |color, opacity| IconButtonVisualStyle {
            container: container(color),
            icon: IconSlotStyle {
                logical_size: 18.0,
                opacity,
            },
        };
        Self {
            resting: visual(ColorRgba8::rgba(54, 60, 74, 255), 1.0),
            hovered: Some(visual(ColorRgba8::rgba(66, 74, 92, 255), 1.0)),
            focused: Some(visual(ColorRgba8::rgba(61, 72, 101, 255), 1.0)),
            pressed: Some(visual(ColorRgba8::rgba(42, 48, 61, 255), 1.0)),
            busy: Some(visual(ColorRgba8::rgba(50, 55, 68, 255), 0.75)),
            disabled: Some(visual(ColorRgba8::rgba(43, 46, 55, 180), 0.5)),
        }
    }
}

/// Immutable mount configuration for an icon-only application action.
#[derive(Clone, Debug, PartialEq)]
pub struct IconButton {
    artwork: IconArtwork,
    button: Button,
    style: IconButtonStyle,
}

impl IconButton {
    /// Creates an icon-only action. `accessible_name` is mandatory and independent of artwork.
    pub fn new(
        artwork: impl Into<IconArtwork>,
        accessible_name: impl Into<String>,
    ) -> Result<Self, IconButtonError> {
        let button = Button::new(accessible_name).map_err(IconButtonError::from)?;
        Ok(Self {
            artwork: artwork.into(),
            button,
            style: IconButtonStyle::default(),
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

    pub fn style(mut self, style: IconButtonStyle) -> Self {
        self.style = style;
        self
    }

    pub fn behavior(&self) -> ButtonBehavior {
        self.button.behavior()
    }

    pub fn semantic_node(&self, name: StringId, state: ButtonInteractionState) -> SemanticNode {
        self.button.semantic_node(name, state)
    }

    /// Mounts decorative icon artwork under the shared semantic/action button root.
    pub fn mount<Action, Map>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
        map: Map,
    ) -> RuntimeResult<IconButtonRef>
    where
        Action: 'static,
        Map: Fn(Activation) -> Action + 'static,
    {
        let state = self.button.initial_interaction_state();
        let mut visual = self.style.resolve(state).visual;
        let minimum = self.button.density_metrics().effective_minimum();
        visual.container.min_size = SizeRule2D {
            width: SizeRule::Logical(minimum.width()),
            height: SizeRule::Logical(minimum.height()),
        };
        let image = self.artwork.image();
        let tint = self.artwork.tint_color();
        let icon = visual.icon;
        let control = ui
            .foundation()
            .button_node_under(host, visual.container, move |writer| {
                writer.dynamic_image_tinted(
                    image,
                    1,
                    tint,
                    BoxStyle {
                        width: SizeRule::Logical(icon.logical_size()),
                        height: SizeRule::Logical(icon.logical_size()),
                        opacity: icon.opacity(),
                        ..BoxStyle::default()
                    },
                    Default::default(),
                );
            })
            .ok_or_else(|| RuntimeError::new("application icon-button host is stale"))?;

        self.button.attach_mounted_contract(ui, control.node)?;
        self.button
            .route_mounted_activation(ui, control.node, map)?;
        Ok(IconButtonRef { control })
    }
}

/// Focused advanced reference returned by icon-button mounting.
#[derive(Clone, Copy, Debug)]
pub struct IconButtonRef {
    control: ControlHandle,
}

impl IconButtonRef {
    pub const fn node(self) -> UiNodeId {
        self.control.node
    }

    pub const fn enabled(self) -> Property<bool> {
        self.control.enabled
    }

    pub const fn style(self) -> Property<BoxStyle> {
        self.control.style
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconButtonError {
    MissingAccessibleName,
}

impl From<ButtonError> for IconButtonError {
    fn from(error: ButtonError) -> Self {
        match error {
            ButtonError::MissingAccessibleName => Self::MissingAccessibleName,
        }
    }
}

impl std::fmt::Display for IconButtonError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingAccessibleName => {
                formatter.write_str("icon button accessible name is empty")
            }
        }
    }
}

impl std::error::Error for IconButtonError {}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use crate::input::{
        ActivationInput, ActivationTransition, ChangeSource, PointerButton, PointerId,
    };
    use crate::runtime::{Component, CreateContext, UpdateContext, ViewRuntime};
    use crate::ui::{LayoutStyle, SemanticAction, SemanticName, SemanticRole, UiRoot};

    use crate::components::application::DensityClass;

    use super::*;

    #[test]
    fn accessible_name_is_required_and_not_stored_in_artwork() {
        let artwork = IconArtwork::from_image(ImageId(17));
        assert_eq!(artwork.image(), ImageId(17));
        assert_eq!(
            IconButton::new(artwork, " ").unwrap_err(),
            IconButtonError::MissingAccessibleName
        );
    }

    #[test]
    fn icon_artwork_preserves_icon_tint() {
        const ICON: IconAsset = IconAsset::new(crate::assets::AssetKey::new("icons/test.svg"));
        let white = ColorRgba8::rgba(255, 255, 255, 255);
        let artwork = IconArtwork::from(Icon::new(ICON).tint(white));

        assert_eq!(artwork.image(), ICON.image_id());
        assert_eq!(artwork.tint_color(), Some(white));
        assert_eq!(artwork.without_tint().tint_color(), None);
    }

    #[test]
    fn icon_slot_geometry_and_opacity_are_validated() {
        assert_eq!(
            IconSlotStyle::new(0.0, 1.0),
            Err(IconSlotStyleError::InvalidLogicalSize)
        );
        assert_eq!(
            IconSlotStyle::new(16.0, f32::NAN),
            Err(IconSlotStyleError::InvalidOpacity)
        );
        assert_eq!(IconSlotStyle::new(20.0, 0.5).unwrap().logical_size(), 20.0);
    }

    #[test]
    fn icon_style_reuses_button_family_priority_and_fallback() {
        let style = IconButtonStyle {
            pressed: None,
            ..IconButtonStyle::default()
        };
        let state = ButtonInteractionState {
            enabled: true,
            pressed: true,
            hovered: true,
            ..ButtonInteractionState::resting(true, false)
        };
        let resolved = style.resolve(state);
        assert_eq!(resolved.state, ButtonStyleState::Pressed);
        assert_eq!(resolved.visual, style.resting);
    }

    #[test]
    fn behavior_is_the_shared_button_activation_owner() {
        let mut behavior = IconButton::new(ImageId(4), "Refresh").unwrap().behavior();
        let pointer = PointerId::new(3);
        behavior.handle(ActivationInput::PointerDown {
            pointer,
            button: PointerButton::PRIMARY,
        });
        assert_eq!(
            behavior
                .handle(ActivationInput::PointerUp {
                    pointer,
                    button: PointerButton::PRIMARY,
                    inside: true,
                })
                .transition,
            ActivationTransition::Activated(Activation {
                source: ChangeSource::Pointer,
            })
        );
    }

    struct MountedIconButton {
        node: Rc<Cell<Option<UiNodeId>>>,
        received: Rc<RefCell<Vec<Activation>>>,
        button: IconButton,
    }

    impl Component for MountedIconButton {
        type State = ();
        type Action = Activation;

        fn create(&self, _context: &mut CreateContext<'_>) -> Self::State {}

        fn mount(&self, _state: &(), ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            let button = self
                .button
                .mount(ui, root.0, |activation| activation)
                .unwrap();
            self.node.set(Some(button.node()));
            root
        }

        fn action(
            &self,
            _state: &mut (),
            action: Self::Action,
            _context: &mut UpdateContext<'_, Self>,
        ) {
            self.received.borrow_mut().push(action);
        }
    }

    #[test]
    fn mounted_icon_is_decorative_under_named_semantic_button_and_preserves_source() {
        let node = Rc::new(Cell::new(None));
        let received = Rc::new(RefCell::new(Vec::new()));
        let mut runtime = ViewRuntime::from_component(MountedIconButton {
            node: node.clone(),
            received: received.clone(),
            button: IconButton::new(ImageId(23), "Open navigation")
                .unwrap()
                .density(DensityMetrics::baseline(DensityClass::Touch)),
        })
        .unwrap();
        let node = node.get().unwrap();

        let semantic = runtime.ui().semantics.get(node).unwrap();
        assert_eq!(semantic.role, SemanticRole::Button);
        let SemanticName::Text(name) = semantic.name else {
            panic!("icon button must own an explicit text name");
        };
        assert_eq!(runtime.ui().string(name), Some("Open navigation"));
        assert!(semantic.actions.contains(SemanticAction::Activate));
        assert_eq!(runtime.ui().nodes.children(node).count(), 1);
        let icon = runtime.ui().nodes.children(node).next().unwrap();
        assert_eq!(runtime.ui().images.get(icon).unwrap().image, ImageId(23));
        assert!(runtime.ui().semantics.get(icon).is_none());
        assert_eq!(
            runtime.ui().box_styles.get(node).unwrap().min_size,
            SizeRule2D {
                width: SizeRule::Logical(44.0),
                height: SizeRule::Logical(44.0),
            }
        );
        assert_eq!(
            runtime.ui().box_styles.get(icon).unwrap().width,
            SizeRule::Logical(18.0)
        );

        assert!(runtime.dispatch_activation(node, ChangeSource::Accessibility));
        assert_eq!(
            &*received.borrow(),
            &[Activation {
                source: ChangeSource::Accessibility,
            }]
        );
    }

    #[test]
    fn busy_icon_button_stays_focusable_without_advertising_or_routing_activation() {
        let node = Rc::new(Cell::new(None));
        let received = Rc::new(RefCell::new(Vec::new()));
        let mut runtime = ViewRuntime::from_component(MountedIconButton {
            node: node.clone(),
            received: received.clone(),
            button: IconButton::new(ImageId(8), "Synchronizing")
                .unwrap()
                .busy(true),
        })
        .unwrap();
        let node = node.get().unwrap();
        let semantic = runtime.ui().semantics.get(node).unwrap();
        assert!(semantic.state.busy);
        assert!(semantic.state.focusable);
        assert!(semantic.actions.contains(SemanticAction::Focus));
        assert!(!semantic.actions.contains(SemanticAction::Activate));
        assert!(!runtime.dispatch_activation(node, ChangeSource::Pointer));
        assert!(received.borrow().is_empty());
    }
}

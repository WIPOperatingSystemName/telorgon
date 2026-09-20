//! Controlled Tier A checkbox built on canonical activation and check-cycle owners.

use crate::foundation::{ColorRgba8, EdgeInsets, PointF, Transform2D};
use crate::runtime::{Read, RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    Background, Border, BoxSizing, BoxStyle, ControlHandle, CornerRadii, Flow, LayoutStyle,
    Property, SemanticCheckState, SemanticNode, SemanticRole, SizeRule, SizeRule2D, StringId,
    UiNodeId,
};

use crate::components::application::{
    Button, ButtonBehavior, ButtonError, ButtonInteractionState, ButtonStyleState,
    CheckCyclePolicy, CheckState, DensityMetrics, ValueChange,
};

/// Visual slots for one checkbox value and interaction state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CheckboxVisualStyle {
    pub container: BoxStyle,
    pub indicator: BoxStyle,
    pub mark_color: ColorRgba8,
    pub mark_size: f32,
    pub label_color: ColorRgba8,
    pub label_size: f32,
    pub gap: f32,
}

const MARK_VIEW_BOX: f32 = 24.0;
const MARK_STROKE_WIDTH: f32 = 2.0;
const CHECK_PATH: [PointF; 3] = [
    PointF { x: 20.0, y: 6.0 },
    PointF { x: 9.0, y: 17.0 },
    PointF { x: 4.0, y: 12.0 },
];
const MIXED_PATH: [PointF; 2] = [PointF { x: 5.0, y: 12.0 }, PointF { x: 19.0, y: 12.0 }];

#[derive(Clone, Copy, Debug, PartialEq)]
struct CheckboxMarkStyles {
    root: BoxStyle,
    check_first: BoxStyle,
    check_second: BoxStyle,
    mixed: BoxStyle,
}

fn checkbox_mark_styles(visual: CheckboxVisualStyle, value: CheckState) -> CheckboxMarkStyles {
    let mark_size = if visual.mark_size.is_finite() {
        visual.mark_size.max(0.0)
    } else {
        0.0
    };
    let indicator_width = indicator_content_extent(visual.indicator, true, mark_size);
    let indicator_height = indicator_content_extent(visual.indicator, false, mark_size);
    let scale = mark_size / MARK_VIEW_BOX;
    let offset = PointF {
        x: (indicator_width - mark_size) * 0.5,
        y: (indicator_height - mark_size) * 0.5,
    };
    let check_background = mark_background(value == CheckState::Checked, visual.mark_color);
    let mixed_background = mark_background(value == CheckState::Mixed, visual.mark_color);

    CheckboxMarkStyles {
        root: BoxStyle {
            width: SizeRule::Logical(indicator_width),
            height: SizeRule::Logical(indicator_height),
            ..BoxStyle::default()
        },
        check_first: mark_segment(
            CHECK_PATH[0],
            CHECK_PATH[1],
            scale,
            offset,
            check_background,
        ),
        check_second: mark_segment(
            CHECK_PATH[1],
            CHECK_PATH[2],
            scale,
            offset,
            check_background,
        ),
        mixed: mark_segment(
            MIXED_PATH[0],
            MIXED_PATH[1],
            scale,
            offset,
            mixed_background,
        ),
    }
}

fn mark_segment(
    start: PointF,
    end: PointF,
    scale: f32,
    offset: PointF,
    background: Background,
) -> BoxStyle {
    let dx = (end.x - start.x) * scale;
    let dy = (end.y - start.y) * scale;
    let length = dx.hypot(dy);
    let stroke_width = MARK_STROKE_WIDTH * scale;
    BoxStyle {
        width: SizeRule::Logical(length),
        height: SizeRule::Logical(stroke_width),
        decoration: crate::ui::BoxDecoration {
            background,
            corner_radii: CornerRadii::all(stroke_width * 0.5),
            ..crate::ui::BoxDecoration::default()
        },
        transform: Transform2D {
            translation: PointF {
                x: offset.x + start.x * scale,
                y: offset.y + start.y * scale - stroke_width * 0.5,
            },
            rotation: dy.atan2(dx),
            origin: PointF { x: 0.0, y: 0.5 },
            ..Transform2D::default()
        },
        ..BoxStyle::default()
    }
}

const fn fixed_extent(rule: SizeRule) -> Option<f32> {
    match rule {
        SizeRule::Logical(value) => Some(value),
        SizeRule::Percent(_) | SizeRule::Fill(_) | SizeRule::Shrink => None,
    }
}

fn indicator_content_extent(style: BoxStyle, horizontal: bool, fallback: f32) -> f32 {
    let specified = fixed_extent(if horizontal {
        style.width
    } else {
        style.height
    })
    .unwrap_or(fallback)
    .max(0.0);
    if style.sizing == BoxSizing::ContentBox {
        return specified;
    }

    let border = if horizontal {
        style.decoration.border.left.width.max(0.0) + style.decoration.border.right.width.max(0.0)
    } else {
        style.decoration.border.top.width.max(0.0) + style.decoration.border.bottom.width.max(0.0)
    };
    let padding = if horizontal {
        style.padding.horizontal()
    } else {
        style.padding.vertical()
    };
    (specified - border - padding).max(0.0)
}

const fn mark_background(visible: bool, color: ColorRgba8) -> Background {
    if visible {
        Background::Color(color)
    } else {
        Background::None
    }
}

/// Interaction variants for one controlled checkbox value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CheckboxStateStyle {
    pub resting: CheckboxVisualStyle,
    pub hovered: Option<CheckboxVisualStyle>,
    pub focused: Option<CheckboxVisualStyle>,
    pub pressed: Option<CheckboxVisualStyle>,
    pub disabled: Option<CheckboxVisualStyle>,
}

impl CheckboxStateStyle {
    const fn resolve(
        self,
        state: ButtonInteractionState,
    ) -> (ButtonStyleState, CheckboxVisualStyle) {
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

/// Typed styles for unchecked, checked, and mixed controlled values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CheckboxStyle {
    pub unchecked: CheckboxStateStyle,
    pub checked: CheckboxStateStyle,
    pub mixed: CheckboxStateStyle,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedCheckboxStyle {
    pub value: CheckState,
    pub state: ButtonStyleState,
    pub visual: CheckboxVisualStyle,
}

impl CheckboxStyle {
    pub const fn resolve(
        self,
        value: CheckState,
        state: ButtonInteractionState,
    ) -> ResolvedCheckboxStyle {
        let (resolved_state, visual) = match value {
            CheckState::Unchecked => self.unchecked.resolve(state),
            CheckState::Checked => self.checked.resolve(state),
            CheckState::Mixed => self.mixed.resolve(state),
        };
        ResolvedCheckboxStyle {
            value,
            state: resolved_state,
            visual,
        }
    }
}

impl Default for CheckboxStyle {
    fn default() -> Self {
        fn visual(
            indicator_color: ColorRgba8,
            indicator_background: Background,
            container_background: Background,
            opacity: u8,
        ) -> CheckboxVisualStyle {
            CheckboxVisualStyle {
                container: BoxStyle {
                    min_size: SizeRule2D {
                        width: SizeRule::Logical(32.0),
                        height: SizeRule::Logical(32.0),
                    },
                    padding: EdgeInsets::all(5.0),
                    decoration: crate::ui::BoxDecoration {
                        background: container_background,
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
                        background: indicator_background,
                        corner_radii: CornerRadii::all(4.0),
                        ..crate::ui::BoxDecoration::default()
                    },
                    ..BoxStyle::default()
                },
                mark_color: indicator_color,
                mark_size: 14.0,
                label_color: ColorRgba8::rgba(235, 238, 244, opacity),
                label_size: 14.0,
                gap: 8.0,
            }
        }

        fn state_style(
            indicator_background: Background,
            mark_color: ColorRgba8,
        ) -> CheckboxStateStyle {
            CheckboxStateStyle {
                resting: visual(mark_color, indicator_background, Background::None, 255),
                hovered: Some(visual(
                    mark_color,
                    indicator_background,
                    Background::Color(ColorRgba8::rgba(69, 78, 96, 90)),
                    255,
                )),
                focused: Some(visual(
                    mark_color,
                    indicator_background,
                    Background::Color(ColorRgba8::rgba(66, 91, 139, 110)),
                    255,
                )),
                pressed: Some(visual(
                    mark_color,
                    indicator_background,
                    Background::Color(ColorRgba8::rgba(46, 55, 72, 140)),
                    255,
                )),
                disabled: Some(visual(
                    ColorRgba8::rgba(186, 190, 199, 180),
                    indicator_background,
                    Background::None,
                    180,
                )),
            }
        }

        Self {
            unchecked: state_style(Background::None, ColorRgba8::rgba(255, 255, 255, 255)),
            checked: state_style(
                Background::Color(ColorRgba8::rgba(54, 104, 210, 255)),
                ColorRgba8::rgba(255, 255, 255, 255),
            ),
            mixed: state_style(
                Background::Color(ColorRgba8::rgba(75, 96, 150, 255)),
                ColorRgba8::rgba(255, 255, 255, 255),
            ),
        }
    }
}

/// Immutable configuration for a labelled, parent-controlled checkbox.
#[derive(Clone, Debug, PartialEq)]
pub struct Checkbox {
    button: Button,
    value: Read<CheckState>,
    cycle: CheckCyclePolicy,
    style: CheckboxStyle,
}

impl Checkbox {
    pub fn new(label: impl Into<String>, value: Read<CheckState>) -> Result<Self, CheckboxError> {
        let button = Button::new(label).map_err(CheckboxError::from)?;
        Ok(Self {
            button,
            value,
            cycle: CheckCyclePolicy::two_state(),
            style: CheckboxStyle::default(),
        })
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.button = self.button.enabled(enabled);
        self
    }

    pub fn cycle(mut self, cycle: CheckCyclePolicy) -> Self {
        self.cycle = cycle;
        self
    }

    pub fn density(mut self, density: DensityMetrics) -> Self {
        self.button = self.button.density(density);
        self
    }

    pub fn style(mut self, style: CheckboxStyle) -> Self {
        self.style = style;
        self
    }

    pub fn behavior(&self) -> ButtonBehavior {
        self.button.behavior()
    }

    pub const fn value(&self) -> Read<CheckState> {
        self.value
    }

    pub const fn cycle_policy(&self) -> CheckCyclePolicy {
        self.cycle
    }

    pub fn semantic_node(
        &self,
        name: StringId,
        value: CheckState,
        state: ButtonInteractionState,
    ) -> SemanticNode {
        let mut semantic = self.button.semantic_node(name, state);
        semantic.role = SemanticRole::Checkbox;
        semantic.state.checked = Some(semantic_check_state(value));
        semantic
    }

    /// Mounts the current controlled value and derives every proposal from the latest read.
    pub fn mount<Action, Map>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
        map: Map,
    ) -> RuntimeResult<CheckboxRef>
    where
        Action: 'static,
        Map: Fn(ValueChange<CheckState>) -> Action + 'static,
    {
        let value = ui.read(self.value)?;
        self.cycle.next(value).map_err(|error| {
            RuntimeError::new(format!("invalid initial checkbox cycle state: {error}"))
        })?;

        let state = self.button.initial_interaction_state();
        let mut visual = self.style.resolve(value, state).visual;
        let minimum = self.button.density_metrics().effective_minimum();
        visual.container.min_size = SizeRule2D {
            width: SizeRule::Logical(minimum.width()),
            height: SizeRule::Logical(minimum.height()),
        };

        let label = self.button.label().to_owned();
        let indicator = visual.indicator;
        let mark_styles = checkbox_mark_styles(visual, value);
        let label_color = visual.label_color;
        let label_size = visual.label_size;
        let row_layout = LayoutStyle {
            flow: Flow::Horizontal,
            gap: visual.gap,
            ..LayoutStyle::default()
        };
        let mut indicator_control = None;
        let mut mark_control = None;
        let mut check_first_control = None;
        let mut check_second_control = None;
        let mut mixed_control = None;
        let mut label_control = None;
        let control = ui
            .foundation()
            .toggle_node_under(host, visual.container, |writer| {
                writer.container(BoxStyle::default(), row_layout, |writer| {
                    indicator_control = Some(writer.container_handle(
                        indicator,
                        LayoutStyle {
                            flow: Flow::Overlay,
                            ..LayoutStyle::default()
                        },
                        |writer| {
                            mark_control = Some(writer.container_handle(
                                mark_styles.root,
                                LayoutStyle {
                                    flow: Flow::Overlay,
                                    ..LayoutStyle::default()
                                },
                                |writer| {
                                    check_first_control = Some(writer.container_handle(
                                        mark_styles.check_first,
                                        LayoutStyle::default(),
                                        |_| {},
                                    ));
                                    check_second_control = Some(writer.container_handle(
                                        mark_styles.check_second,
                                        LayoutStyle::default(),
                                        |_| {},
                                    ));
                                    mixed_control = Some(writer.container_handle(
                                        mark_styles.mixed,
                                        LayoutStyle::default(),
                                        |_| {},
                                    ));
                                },
                            ));
                        },
                    ));
                    label_control = Some(writer.text(&label, label_color, label_size));
                });
            })
            .ok_or_else(|| RuntimeError::new("application checkbox host is stale"))?;
        let indicator_control =
            indicator_control.expect("checkbox indicator mounts with its control");
        let mark_control = mark_control.expect("checkbox mark mounts with its indicator");
        let check_first_control =
            check_first_control.expect("checkbox first check segment mounts with its mark");
        let check_second_control =
            check_second_control.expect("checkbox second check segment mounts with its mark");
        let mixed_control = mixed_control.expect("checkbox mixed segment mounts with its mark");
        let label_control = label_control.expect("checkbox label mounts with its control");

        self.button
            .attach_mounted_contract_with(ui, control.node, |semantic| {
                semantic.role = SemanticRole::Checkbox;
                semantic.state.checked = Some(semantic_check_state(value));
            })?;
        let read = self.value;
        ui.bind_map(read, control.checked, |value| semantic_check_state(*value))?;
        let style = self.style;
        ui.bind_map(read, control.style, move |value| {
            let mut visual = style.resolve(*value, state).visual;
            visual.container.min_size = SizeRule2D {
                width: SizeRule::Logical(minimum.width()),
                height: SizeRule::Logical(minimum.height()),
            };
            visual.container
        })?;
        let style = self.style;
        ui.bind_map(read, indicator_control.style, move |value| {
            style.resolve(*value, state).visual.indicator
        })?;
        let style = self.style;
        ui.bind_map(read, mark_control.style, move |value| {
            let visual = style.resolve(*value, state).visual;
            checkbox_mark_styles(visual, *value).root
        })?;
        let style = self.style;
        ui.bind_map(read, check_first_control.style, move |value| {
            let visual = style.resolve(*value, state).visual;
            checkbox_mark_styles(visual, *value).check_first
        })?;
        let style = self.style;
        ui.bind_map(read, check_second_control.style, move |value| {
            let visual = style.resolve(*value, state).visual;
            checkbox_mark_styles(visual, *value).check_second
        })?;
        let style = self.style;
        ui.bind_map(read, mixed_control.style, move |value| {
            let visual = style.resolve(*value, state).visual;
            checkbox_mark_styles(visual, *value).mixed
        })?;
        let style = self.style;
        ui.bind_map(read, label_control.color, move |value| {
            style.resolve(*value, state).visual.label_color
        })?;
        if self.button.accepts_activation() {
            let cycle = self.cycle;
            ui.route_activation_read_fallible(
                control.node,
                self.value,
                move |current, activation| {
                    let next = cycle.next(*current).map_err(|error| {
                        RuntimeError::new(format!("invalid live checkbox cycle state: {error}"))
                    })?;
                    Ok(map(ValueChange::committed(next, activation.source)))
                },
            )?;
        }

        Ok(CheckboxRef {
            control,
            indicator: indicator_control,
            mark: mark_control.node,
            value: self.value,
        })
    }
}

/// Focused advanced reference returned by checkbox mounting.
#[derive(Clone, Copy, Debug)]
pub struct CheckboxRef {
    control: ControlHandle,
    indicator: ControlHandle,
    mark: UiNodeId,
    value: Read<CheckState>,
}

impl CheckboxRef {
    pub const fn node(self) -> UiNodeId {
        self.control.node
    }

    pub const fn value(self) -> Read<CheckState> {
        self.value
    }

    pub const fn enabled(self) -> Property<bool> {
        self.control.enabled
    }

    pub const fn style(self) -> Property<BoxStyle> {
        self.control.style
    }

    pub const fn indicator_node(self) -> UiNodeId {
        self.indicator.node
    }

    pub const fn mark_node(self) -> UiNodeId {
        self.mark
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckboxError {
    MissingAccessibleName,
}

impl From<ButtonError> for CheckboxError {
    fn from(error: ButtonError) -> Self {
        match error {
            ButtonError::MissingAccessibleName => Self::MissingAccessibleName,
        }
    }
}

impl std::fmt::Display for CheckboxError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingAccessibleName => formatter.write_str("checkbox accessible name is empty"),
        }
    }
}

impl std::error::Error for CheckboxError {}

const fn semantic_check_state(value: CheckState) -> SemanticCheckState {
    match value {
        CheckState::Unchecked => SemanticCheckState::Unchecked,
        CheckState::Checked => SemanticCheckState::Checked,
        CheckState::Mixed => SemanticCheckState::Mixed,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use crate::input::{
        ActivationInput, ActivationTransition, ChangeSource, PointerButton, PointerId,
    };
    use crate::runtime::{Component, CreateContext, State, UpdateContext, ViewRuntime};
    use crate::ui::{LayoutStyle, SemanticAction, SemanticName, SemanticRole, UiRoot};

    use crate::components::application::{ChangePhase, DensityClass};

    use super::*;

    #[test]
    fn accessible_name_and_initial_binary_value_are_validated() {
        struct InvalidInitial {
            missing_name_rejected: Rc<Cell<bool>>,
            initial_error: Rc<RefCell<Option<String>>>,
        }

        impl Component for InvalidInitial {
            type State = State<CheckState>;
            type Action = ();

            fn create(&self, context: &mut CreateContext<'_>) -> Self::State {
                context.state(CheckState::Mixed)
            }

            fn mount(&self, state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
                let root =
                    ui.foundation()
                        .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
                self.missing_name_rejected.set(matches!(
                    Checkbox::new(" ", state.read()),
                    Err(CheckboxError::MissingAccessibleName)
                ));
                let result =
                    Checkbox::new("Aggregate", state.read())
                        .unwrap()
                        .mount(ui, root.0, |_| ());
                if let Err(error) = result {
                    *self.initial_error.borrow_mut() = Some(error.to_string());
                }
                root
            }

            fn action(
                &self,
                _state: &mut Self::State,
                _action: Self::Action,
                _context: &mut UpdateContext<'_, Self>,
            ) {
            }
        }

        let missing_name_rejected = Rc::new(Cell::new(false));
        let initial_error = Rc::new(RefCell::new(None));
        ViewRuntime::from_component(InvalidInitial {
            missing_name_rejected: missing_name_rejected.clone(),
            initial_error: initial_error.clone(),
        })
        .unwrap();
        assert!(missing_name_rejected.get());
        assert!(
            initial_error
                .borrow()
                .as_deref()
                .unwrap()
                .contains("two-state check cycle cannot consume Mixed")
        );
    }

    #[test]
    fn style_and_semantics_resolve_each_controlled_value_before_interaction() {
        let style = CheckboxStyle::default();
        let state = ButtonInteractionState {
            pressed: true,
            hovered: true,
            ..ButtonInteractionState::resting(true, false)
        };
        let unchecked = style.resolve(CheckState::Unchecked, state);
        let checked = style.resolve(CheckState::Checked, state);
        let mixed = style.resolve(CheckState::Mixed, state);
        assert_eq!(unchecked.state, ButtonStyleState::Pressed);
        assert_eq!(checked.state, ButtonStyleState::Pressed);
        assert_eq!(mixed.state, ButtonStyleState::Pressed);
        assert_ne!(
            unchecked.visual.indicator.decoration.background,
            checked.visual.indicator.decoration.background
        );
        assert_ne!(
            checked.visual.indicator.decoration.background,
            mixed.visual.indicator.decoration.background
        );

        for (value, expected) in [
            (CheckState::Unchecked, SemanticCheckState::Unchecked),
            (CheckState::Checked, SemanticCheckState::Checked),
            (CheckState::Mixed, SemanticCheckState::Mixed),
        ] {
            assert_eq!(semantic_check_state(value), expected);
        }
    }

    #[test]
    fn lucide_marks_are_rounded_vector_segments_not_font_glyphs() {
        assert_eq!(
            CHECK_PATH,
            [
                PointF { x: 20.0, y: 6.0 },
                PointF { x: 9.0, y: 17.0 },
                PointF { x: 4.0, y: 12.0 },
            ]
        );
        assert_eq!(
            MIXED_PATH,
            [PointF { x: 5.0, y: 12.0 }, PointF { x: 19.0, y: 12.0 }]
        );

        let style = CheckboxStyle::default();
        let checked = checkbox_mark_styles(style.checked.resting, CheckState::Checked);
        assert_eq!(checked.root.width, SizeRule::Logical(16.0));
        assert_eq!(checked.root.height, SizeRule::Logical(16.0));
        assert!(matches!(
            checked.check_first.decoration.background,
            Background::Color(_)
        ));
        assert!(matches!(
            checked.check_second.decoration.background,
            Background::Color(_)
        ));
        assert_eq!(checked.mixed.decoration.background, Background::None);
        assert_eq!(
            checked.check_first.transform.origin,
            PointF { x: 0.0, y: 0.5 }
        );
        assert_eq!(
            checked.check_first.decoration.corner_radii,
            CornerRadii::all(
                MARK_STROKE_WIDTH * style.checked.resting.mark_size / MARK_VIEW_BOX / 2.0
            )
        );

        let mixed = checkbox_mark_styles(style.mixed.resting, CheckState::Mixed);
        assert_eq!(mixed.check_first.decoration.background, Background::None);
        assert_eq!(mixed.check_second.decoration.background, Background::None);
        assert!(matches!(
            mixed.mixed.decoration.background,
            Background::Color(_)
        ));
        assert_eq!(mixed.mixed.transform.rotation, 0.0);
        assert_eq!(
            mixed.mixed.transform.translation.y
                + MARK_STROKE_WIDTH * style.mixed.resting.mark_size / MARK_VIEW_BOX * 0.5,
            8.0
        );
    }

    struct CheckboxStateOwner {
        node: Rc<Cell<Option<UiNodeId>>>,
        requests: Rc<RefCell<Vec<ValueChange<CheckState>>>>,
        initial: CheckState,
        cycle: CheckCyclePolicy,
    }

    struct OwnerState {
        value: State<CheckState>,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum OwnerAction {
        Requested(ValueChange<CheckState>),
        Publish(CheckState),
        Noop,
    }

    impl Component for CheckboxStateOwner {
        type State = OwnerState;
        type Action = OwnerAction;

        fn create(&self, context: &mut CreateContext<'_>) -> Self::State {
            OwnerState {
                value: context.state(self.initial),
            }
        }

        fn mount(&self, state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            let checkbox = Checkbox::new("Pinned", state.value.read())
                .unwrap()
                .cycle(self.cycle)
                .density(DensityMetrics::baseline(DensityClass::Touch));
            let mut behavior = checkbox.behavior();
            let pointer = PointerId::new(3);
            behavior.handle(ActivationInput::PointerDown {
                pointer,
                button: PointerButton::PRIMARY,
            });
            assert!(matches!(
                behavior
                    .handle(ActivationInput::PointerUp {
                        pointer,
                        button: PointerButton::PRIMARY,
                        inside: false,
                    })
                    .transition,
                ActivationTransition::Cancelled { .. }
            ));
            let checkbox = checkbox.mount(ui, root.0, OwnerAction::Requested).unwrap();
            self.node.set(Some(checkbox.node()));
            root
        }

        fn action(
            &self,
            state: &mut Self::State,
            action: Self::Action,
            context: &mut UpdateContext<'_, Self>,
        ) {
            match action {
                OwnerAction::Requested(change) => self.requests.borrow_mut().push(change),
                OwnerAction::Publish(value) => context.set(state.value, value).unwrap(),
                OwnerAction::Noop => {}
            }
        }
    }

    #[test]
    fn mounted_tri_state_checkbox_reads_latest_value_and_emits_only_proposals() {
        let node = Rc::new(Cell::new(None));
        let requests = Rc::new(RefCell::new(Vec::new()));
        let cycle = CheckCyclePolicy::tri_state([
            CheckState::Unchecked,
            CheckState::Checked,
            CheckState::Mixed,
        ])
        .unwrap();
        let mut runtime = ViewRuntime::from_component(CheckboxStateOwner {
            node: node.clone(),
            requests: requests.clone(),
            initial: CheckState::Checked,
            cycle,
        })
        .unwrap();
        let node = node.get().unwrap();

        let semantic = runtime.ui().semantics.get(node).unwrap();
        assert_eq!(semantic.role, SemanticRole::Checkbox);
        assert!(semantic.actions.contains(SemanticAction::Activate));
        let SemanticName::Text(name) = semantic.name else {
            panic!("checkbox must expose its stable visible label as its name");
        };
        assert_eq!(runtime.ui().string(name), Some("Pinned"));
        assert!(
            runtime.ui().texts.values().iter().all(|visual| {
                !matches!(runtime.ui().string(visual.content), Some("✓" | "−"))
            })
        );
        assert_eq!(semantic.state.checked, Some(SemanticCheckState::Checked));
        assert_eq!(
            runtime.ui().box_styles.get(node).unwrap().min_size,
            SizeRule2D {
                width: SizeRule::Logical(44.0),
                height: SizeRule::Logical(44.0),
            }
        );

        assert!(runtime.dispatch_activation(node, ChangeSource::Pointer));
        assert!(runtime.dispatch_activation(node, ChangeSource::Accessibility));
        assert_eq!(
            &*requests.borrow(),
            &[
                ValueChange::new(
                    CheckState::Mixed,
                    ChangePhase::Commit,
                    ChangeSource::Pointer
                ),
                ValueChange::new(
                    CheckState::Mixed,
                    ChangePhase::Commit,
                    ChangeSource::Accessibility,
                ),
            ]
        );

        runtime
            .send_component_action(OwnerAction::Publish(CheckState::Mixed))
            .unwrap();
        assert!(runtime.dispatch_action(node));
        assert_eq!(
            requests.borrow().last(),
            Some(&ValueChange::new(
                CheckState::Unchecked,
                ChangePhase::Commit,
                ChangeSource::Programmatic,
            ))
        );
    }

    #[test]
    fn live_mixed_value_under_binary_policy_is_rejected_without_an_action() {
        let node = Rc::new(Cell::new(None));
        let requests = Rc::new(RefCell::new(Vec::new()));
        let mut runtime = ViewRuntime::from_component(CheckboxStateOwner {
            node: node.clone(),
            requests: requests.clone(),
            initial: CheckState::Unchecked,
            cycle: CheckCyclePolicy::two_state(),
        })
        .unwrap();
        let node = node.get().unwrap();

        runtime
            .send_component_action(OwnerAction::Publish(CheckState::Mixed))
            .unwrap();
        assert!(runtime.dispatch_activation(node, ChangeSource::Keyboard));
        assert!(requests.borrow().is_empty());
        let error = runtime
            .send_component_action(OwnerAction::Noop)
            .unwrap_err()
            .to_string();
        assert!(error.contains("invalid live checkbox cycle state"));
        assert!(requests.borrow().is_empty());
    }
}

//! Controlled Tier A slider behavior, semantics, styles, and mounting.

use std::cell::RefCell;
use std::rc::Rc;

use crate::foundation::{ColorRgba8, EdgeInsets, PointF, Transform2D};
use crate::input::{
    ChangeSource, DragAxis, DragRecognizer, GestureArenaRequest, GestureInput,
    GestureRecognizerError, GestureTransition, PointerButton, WritingDirection,
};
use crate::runtime::{Read, RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    Background, Border, BoxStyle, ControlHandle, CornerRadii, Flow, LayoutStyle, Property,
    SemanticActions, SemanticName, SemanticNode, SemanticRole, SemanticState, SemanticValue,
    SizeRule, SizeRule2D, UiNodeId, ValueAxis,
};

use crate::components::application::{
    ChangePhase, DensityMetrics, RangeModel, RangeModelError, RangeScalar, ValueChange,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SliderOrientation {
    #[default]
    Horizontal,
    Vertical,
}

/// Logical and physical navigation requests accepted by slider behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SliderCommand {
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    PageUp,
    PageDown,
    Home,
    End,
    Increment,
    Decrement,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SliderTrackGeometry {
    start: f32,
    length: f32,
}

impl SliderTrackGeometry {
    pub fn new(start: f32, length: f32) -> Result<Self, SliderError> {
        if !start.is_finite() || !length.is_finite() || length <= 0.0 {
            return Err(SliderError::InvalidTrackGeometry);
        }
        Ok(Self { start, length })
    }

    pub const fn start(self) -> f32 {
        self.start
    }

    pub const fn length(self) -> f32 {
        self.length
    }
}

/// Portable controlled-value behavior for one slider thumb.
#[derive(Clone, Debug)]
pub struct SliderBehavior<T> {
    model: RangeModel<T>,
    orientation: SliderOrientation,
    writing_direction: WritingDirection,
    reversed: bool,
    enabled: bool,
    drag: DragRecognizer,
    drag_start: Option<T>,
    last_requested: Option<T>,
}

/// Complete pointer handoff from slider behavior.
#[derive(Clone, Debug, PartialEq)]
pub struct SliderPointerOutcome<T> {
    pub change: Option<ValueChange<T>>,
    pub arena: GestureArenaRequest,
}

impl<T> SliderPointerOutcome<T> {
    fn new(change: Option<ValueChange<T>>, arena: GestureArenaRequest) -> Self {
        Self { change, arena }
    }
}

impl<T> SliderBehavior<T>
where
    T: RangeScalar,
{
    pub fn new(
        model: RangeModel<T>,
        orientation: SliderOrientation,
        writing_direction: WritingDirection,
        reversed: bool,
        enabled: bool,
    ) -> Result<Self, SliderError> {
        let axis = match orientation {
            SliderOrientation::Horizontal => DragAxis::Horizontal,
            SliderOrientation::Vertical => DragAxis::Vertical,
        };
        Ok(Self {
            model,
            orientation,
            writing_direction,
            reversed,
            enabled,
            drag: DragRecognizer::new(axis, 6.0, enabled).map_err(SliderError::Gesture)?,
            drag_start: None,
            last_requested: None,
        })
    }

    pub const fn model(&self) -> &RangeModel<T> {
        &self.model
    }

    pub const fn orientation(&self) -> SliderOrientation {
        self.orientation
    }

    pub const fn writing_direction(&self) -> WritingDirection {
        self.writing_direction
    }

    pub const fn reversed(&self) -> bool {
        self.reversed
    }

    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    /// Derives one committed keyboard, directional, accessibility, or programmatic request.
    /// Commands that do not apply to the configured axis, or that cannot change a bound value,
    /// return `None`.
    pub fn request(
        &self,
        current: T,
        command: SliderCommand,
        source: ChangeSource,
    ) -> Result<Option<ValueChange<T>>, SliderError> {
        if !self.enabled {
            return Ok(None);
        }
        let current = self.model.normalize(current)?;
        let Some(target) = self.command_target(current, command)? else {
            return Ok(None);
        };
        if target == current {
            return Ok(None);
        }
        Ok(Some(ValueChange::new(target, ChangePhase::Commit, source)))
    }

    /// Applies one source-neutral pointer input through the shared drag recognizer.
    pub fn handle_pointer(
        &mut self,
        current: T,
        input: GestureInput,
        track: SliderTrackGeometry,
    ) -> Result<SliderPointerOutcome<T>, SliderError> {
        if self.enabled
            && matches!(
                input,
                GestureInput::PointerDown {
                    button: PointerButton::PRIMARY,
                    ..
                }
            )
        {
            self.drag_start = Some(self.model.normalize(current)?);
            self.last_requested = None;
        }
        if let GestureInput::SetEnabled(enabled) = input {
            self.enabled = enabled;
        }
        let outcome = self.drag.handle(input).map_err(SliderError::Gesture)?;
        let arena = outcome.arena;
        let change = match outcome.transition {
            GestureTransition::DragStarted { position, .. } => {
                let value = self.value_from_position(position, track)?;
                self.last_requested = Some(value);
                Some(ValueChange::new(
                    value,
                    ChangePhase::Begin,
                    ChangeSource::Pointer,
                ))
            }
            GestureTransition::DragUpdated { position, .. } => {
                let value = self.value_from_position(position, track)?;
                if self.last_requested == Some(value) {
                    return Ok(SliderPointerOutcome::new(None, arena));
                }
                self.last_requested = Some(value);
                Some(ValueChange::new(
                    value,
                    ChangePhase::Update,
                    ChangeSource::Pointer,
                ))
            }
            GestureTransition::DragEnded { position, .. } => {
                let value = self.value_from_position(position, track)?;
                self.drag_start = None;
                self.last_requested = None;
                Some(ValueChange::new(
                    value,
                    ChangePhase::Commit,
                    ChangeSource::Pointer,
                ))
            }
            GestureTransition::Cancelled { .. } => {
                let started = self.last_requested.take().is_some();
                let start = self.drag_start.take();
                if started {
                    start.map(|value| {
                        ValueChange::new(value, ChangePhase::Cancel, ChangeSource::Pointer)
                    })
                } else {
                    None
                }
            }
            GestureTransition::None
            | GestureTransition::Possible { .. }
            | GestureTransition::TapRecognized { .. }
            | GestureTransition::LongPressStarted { .. }
            | GestureTransition::LongPressUpdated { .. }
            | GestureTransition::LongPressEnded { .. } => None,
        };
        Ok(SliderPointerOutcome::new(change, arena))
    }

    fn command_target(&self, current: T, command: SliderCommand) -> Result<Option<T>, SliderError> {
        let direction = match command {
            SliderCommand::Increment => Some(1_i64),
            SliderCommand::Decrement => Some(-1),
            SliderCommand::Home => return Ok(Some(self.model.minimum())),
            SliderCommand::End => return Ok(Some(self.model.maximum())),
            SliderCommand::PageUp => {
                return self
                    .model
                    .page_by(current, if self.reversed { -1 } else { 1 })
                    .map(Some)
                    .map_err(SliderError::Model);
            }
            SliderCommand::PageDown => {
                return self
                    .model
                    .page_by(current, if self.reversed { 1 } else { -1 })
                    .map(Some)
                    .map_err(SliderError::Model);
            }
            SliderCommand::ArrowLeft | SliderCommand::ArrowRight
                if self.orientation == SliderOrientation::Vertical =>
            {
                None
            }
            SliderCommand::ArrowUp | SliderCommand::ArrowDown
                if self.orientation == SliderOrientation::Horizontal =>
            {
                None
            }
            SliderCommand::ArrowLeft => Some(if self.horizontal_increases_at_end() {
                -1
            } else {
                1
            }),
            SliderCommand::ArrowRight => Some(if self.horizontal_increases_at_end() {
                1
            } else {
                -1
            }),
            SliderCommand::ArrowUp => Some(if self.reversed { -1 } else { 1 }),
            SliderCommand::ArrowDown => Some(if self.reversed { 1 } else { -1 }),
        };
        direction
            .map(|direction| self.model.step_by(current, direction))
            .transpose()
            .map_err(SliderError::Model)
    }

    fn horizontal_increases_at_end(&self) -> bool {
        let left_to_right = self.writing_direction == WritingDirection::LeftToRight;
        left_to_right != self.reversed
    }

    fn value_from_position(
        &self,
        position: PointF,
        track: SliderTrackGeometry,
    ) -> Result<T, SliderError> {
        let coordinate = match self.orientation {
            SliderOrientation::Horizontal => position.x,
            SliderOrientation::Vertical => position.y,
        };
        let mut fraction = ((coordinate - track.start) / track.length).clamp(0.0, 1.0);
        match self.orientation {
            SliderOrientation::Horizontal if !self.horizontal_increases_at_end() => {
                fraction = 1.0 - fraction;
            }
            SliderOrientation::Vertical if !self.reversed => {
                fraction = 1.0 - fraction;
            }
            SliderOrientation::Horizontal | SliderOrientation::Vertical => {}
        }
        let minimum = self.model.minimum().to_f64();
        let maximum = self.model.maximum().to_f64();
        let value = minimum + f64::from(fraction) * (maximum - minimum);
        let value = T::from_f64(value).ok_or(SliderError::UnrepresentablePosition)?;
        self.model.normalize(value).map_err(SliderError::Model)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SliderInteractionState {
    pub enabled: bool,
    pub hovered: bool,
    pub focused: bool,
    pub dragging: bool,
}

impl SliderInteractionState {
    pub const fn resting(enabled: bool) -> Self {
        Self {
            enabled,
            hovered: false,
            focused: false,
            dragging: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SliderStyleState {
    Resting,
    Hovered,
    Focused,
    Dragging,
    Disabled,
}

impl SliderStyleState {
    pub const fn resolve(state: SliderInteractionState) -> Self {
        if !state.enabled {
            Self::Disabled
        } else if state.dragging {
            Self::Dragging
        } else if state.focused {
            Self::Focused
        } else if state.hovered {
            Self::Hovered
        } else {
            Self::Resting
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SliderVisualStyle {
    pub container: BoxStyle,
    pub track: BoxStyle,
    pub fill: BoxStyle,
    pub thumb: BoxStyle,
    pub label_color: ColorRgba8,
    pub label_size: f32,
    pub gap: f32,
    pub track_length: f32,
    pub track_thickness: f32,
    pub thumb_size: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SliderStyle {
    pub resting: SliderVisualStyle,
    pub hovered: Option<SliderVisualStyle>,
    pub focused: Option<SliderVisualStyle>,
    pub dragging: Option<SliderVisualStyle>,
    pub disabled: Option<SliderVisualStyle>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedSliderStyle {
    pub state: SliderStyleState,
    pub visual: SliderVisualStyle,
}

impl SliderStyle {
    pub const fn resolve(self, state: SliderInteractionState) -> ResolvedSliderStyle {
        let resolved_state = SliderStyleState::resolve(state);
        let visual = match resolved_state {
            SliderStyleState::Disabled => self.disabled,
            SliderStyleState::Dragging => self.dragging,
            SliderStyleState::Focused => self.focused,
            SliderStyleState::Hovered => self.hovered,
            SliderStyleState::Resting => Some(self.resting),
        };
        ResolvedSliderStyle {
            state: resolved_state,
            visual: match visual {
                Some(visual) => visual,
                None => self.resting,
            },
        }
    }
}

impl Default for SliderStyle {
    fn default() -> Self {
        fn visual(
            container_background: Background,
            accent: ColorRgba8,
            opacity: u8,
        ) -> SliderVisualStyle {
            SliderVisualStyle {
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
                track: BoxStyle {
                    decoration: crate::ui::BoxDecoration {
                        background: Background::Color(ColorRgba8::rgba(78, 87, 105, opacity)),
                        corner_radii: CornerRadii::all(3.0),
                        ..crate::ui::BoxDecoration::default()
                    },
                    ..BoxStyle::default()
                },
                fill: BoxStyle {
                    decoration: crate::ui::BoxDecoration {
                        background: Background::Color(accent),
                        corner_radii: CornerRadii::all(3.0),
                        ..crate::ui::BoxDecoration::default()
                    },
                    ..BoxStyle::default()
                },
                thumb: BoxStyle {
                    decoration: crate::ui::BoxDecoration {
                        background: Background::Color(ColorRgba8::rgba(245, 247, 251, opacity)),
                        border: Border::all(1.0, accent),
                        corner_radii: CornerRadii::all(9.0),
                        ..crate::ui::BoxDecoration::default()
                    },
                    ..BoxStyle::default()
                },
                label_color: ColorRgba8::rgba(235, 238, 244, opacity),
                label_size: 14.0,
                gap: 8.0,
                track_length: 160.0,
                track_thickness: 6.0,
                thumb_size: 18.0,
            }
        }

        Self {
            resting: visual(Background::None, ColorRgba8::rgba(54, 104, 210, 255), 255),
            hovered: Some(visual(
                Background::Color(ColorRgba8::rgba(69, 78, 96, 90)),
                ColorRgba8::rgba(65, 116, 224, 255),
                255,
            )),
            focused: Some(visual(
                Background::Color(ColorRgba8::rgba(66, 91, 139, 110)),
                ColorRgba8::rgba(76, 128, 236, 255),
                255,
            )),
            dragging: Some(visual(
                Background::Color(ColorRgba8::rgba(46, 55, 72, 140)),
                ColorRgba8::rgba(43, 91, 194, 255),
                255,
            )),
            disabled: Some(visual(
                Background::None,
                ColorRgba8::rgba(99, 111, 139, 180),
                180,
            )),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Slider<T: 'static> {
    label: String,
    value: Read<T>,
    model: RangeModel<T>,
    enabled: bool,
    orientation: SliderOrientation,
    writing_direction: WritingDirection,
    reversed: bool,
    density: DensityMetrics,
    style: SliderStyle,
}

impl<T> Slider<T>
where
    T: RangeScalar,
{
    pub fn new(
        label: impl Into<String>,
        value: Read<T>,
        model: RangeModel<T>,
    ) -> Result<Self, SliderError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(SliderError::MissingAccessibleName);
        }
        Ok(Self {
            label,
            value,
            model,
            enabled: true,
            orientation: SliderOrientation::Horizontal,
            writing_direction: WritingDirection::LeftToRight,
            reversed: false,
            density: DensityMetrics::baseline(
                crate::components::application::DensityClass::Standard,
            ),
            style: SliderStyle::default(),
        })
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn orientation(mut self, orientation: SliderOrientation) -> Self {
        self.orientation = orientation;
        self
    }

    pub fn writing_direction(mut self, direction: WritingDirection) -> Self {
        self.writing_direction = direction;
        self
    }

    pub fn reversed(mut self, reversed: bool) -> Self {
        self.reversed = reversed;
        self
    }

    pub fn density(mut self, density: DensityMetrics) -> Self {
        self.density = density;
        self
    }

    pub fn style(mut self, style: SliderStyle) -> Self {
        self.style = style;
        self
    }

    pub fn behavior(&self) -> Result<SliderBehavior<T>, SliderError> {
        SliderBehavior::new(
            self.model.clone(),
            self.orientation,
            self.writing_direction,
            self.reversed,
            self.enabled,
        )
    }

    pub fn semantic_node(
        &self,
        name: crate::ui::StringId,
        value_text: crate::ui::StringId,
        value: T,
    ) -> Result<SemanticNode, SliderError> {
        let current = value.to_f64();
        self.model.format_value(value)?;
        let actions = if self.enabled {
            SemanticActions::FOCUS
                | SemanticActions::INCREMENT
                | SemanticActions::DECREMENT
                | SemanticActions::SET_VALUE
        } else {
            SemanticActions::NONE
        };
        Ok(SemanticNode {
            role: SemanticRole::Slider,
            name: SemanticName::Text(name),
            state: SemanticState {
                disabled: !self.enabled,
                focusable: self.enabled,
                ..SemanticState::default()
            },
            value: SemanticValue::Number {
                current,
                minimum: self.model.minimum().to_f64(),
                maximum: self.model.maximum().to_f64(),
                step: Some(self.model.step().to_f64()),
                value_text: Some(value_text),
            },
            actions,
            ..SemanticNode::default()
        })
    }

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
    ) -> RuntimeResult<SliderRef<T>> {
        let value = ui.read(self.value)?;
        let value_text = self.model.format_value(value).map_err(|error| {
            RuntimeError::new(format!("invalid controlled slider value: {error}"))
        })?;
        let state = SliderInteractionState::resting(self.enabled);
        let minimum = self.density.effective_minimum();
        let visual_context = SliderVisualContext {
            state,
            minimum,
            orientation: self.orientation,
            reversed: self.reversed,
            writing_direction: self.writing_direction,
        };
        let visual = resolved_visual(self.style, visual_context, &self.model, value);

        let label = self.label.clone();
        let label_color = visual.label_color;
        let label_size = visual.label_size;
        let content_flow = match self.orientation {
            SliderOrientation::Horizontal => Flow::Horizontal,
            SliderOrientation::Vertical => Flow::Vertical,
        };
        let content_layout = LayoutStyle {
            flow: content_flow,
            gap: visual.gap,
            ..LayoutStyle::default()
        };
        let content_style = match self.orientation {
            SliderOrientation::Horizontal => BoxStyle {
                width: SizeRule::Fill(1.0),
                height: SizeRule::Logical((visual.label_size * 1.25).max(visual.thumb_size)),
                ..BoxStyle::default()
            },
            SliderOrientation::Vertical => BoxStyle {
                width: SizeRule::Logical((visual.label_size * 4.0).max(visual.thumb_size)),
                height: SizeRule::Logical(
                    visual.track_length + visual.gap + visual.label_size * 1.25,
                ),
                ..BoxStyle::default()
            },
        };
        let mut fill_control = None;
        let mut thumb_control = None;
        let mut label_control = None;
        let mut track_control = None;
        let control = ui
            .foundation()
            .slider_node_under(host, visual.container, |writer| {
                writer.container(content_style, content_layout, |writer| {
                    label_control = Some(writer.text(&label, label_color, label_size));
                    track_control = Some(writer.container_handle(
                        visual.track,
                        LayoutStyle {
                            flow: Flow::Overlay,
                            ..LayoutStyle::default()
                        },
                        |writer| {
                            fill_control = Some(writer.container_handle(
                                visual.fill,
                                LayoutStyle::default(),
                                |_| {},
                            ));
                            thumb_control = Some(writer.container_handle(
                                visual.thumb,
                                LayoutStyle::default(),
                                |_| {},
                            ));
                        },
                    ));
                });
            })
            .ok_or_else(|| RuntimeError::new("application slider host is stale"))?;
        let track_control = track_control.expect("slider track mounts with its control");
        let fill_control = fill_control.expect("slider fill mounts with its track");
        let thumb_control = thumb_control.expect("slider thumb mounts with its track");
        let label_control = label_control.expect("slider label mounts with its control");

        let name = ui.foundation().intern(&self.label);
        let value_text = ui.foundation().intern(&value_text);
        let semantic = self
            .semantic_node(name, value_text, value)
            .map_err(|error| RuntimeError::new(format!("invalid slider semantics: {error}")))?;
        ui.foundation()
            .semantic_node(control.node, semantic)
            .map_err(|error| RuntimeError::new(format!("invalid slider semantics: {error:?}")))?;
        let value_axis = match self.orientation {
            SliderOrientation::Horizontal => ValueAxis::Horizontal {
                inverted: (self.writing_direction == WritingDirection::LeftToRight)
                    == self.reversed,
            },
            SliderOrientation::Vertical => ValueAxis::Vertical {
                inverted: !self.reversed,
            },
        };
        ui.foundation()
            .value_track(control.node, track_control.node, value_axis);
        if !self.enabled {
            ui.foundation().disabled(control.node, true);
        }
        let read = self.value;
        ui.bind_map(read, control.value, |value| value.to_f64() as f32)?;
        let style = self.style;
        let model = self.model.clone();
        ui.bind_map(read, control.style, move |value| {
            resolved_visual(style, visual_context, &model, *value).container
        })?;
        let style = self.style;
        let model = self.model.clone();
        ui.bind_map(read, fill_control.style, move |value| {
            resolved_visual(style, visual_context, &model, *value).fill
        })?;
        let style = self.style;
        let model = self.model.clone();
        ui.bind_map(read, thumb_control.style, move |value| {
            resolved_visual(style, visual_context, &model, *value).thumb
        })?;
        let style = self.style;
        ui.bind_map(read, label_control.color, move |_| {
            style.resolve(state).visual.label_color
        })?;

        Ok(SliderRef {
            control,
            track: track_control,
            fill: fill_control,
            thumb: thumb_control,
            value: self.value,
            behavior: Rc::new(RefCell::new(self.behavior().map_err(|error| {
                RuntimeError::new(format!("invalid slider behavior: {error}"))
            })?)),
        })
    }

    /// Mounts an interactive slider route. Activation advances one normalized step while richer
    /// pointer-drag owners may continue to use [`SliderRef::handle_pointer`].
    pub fn mount_interactive<Action, Map>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
        map: Map,
    ) -> RuntimeResult<SliderRef<T>>
    where
        Action: 'static,
        Map: Fn(ValueChange<T>) -> Action + 'static,
    {
        let reference = self.mount(ui, host)?;
        if self.enabled {
            let map: Rc<dyn Fn(ValueChange<T>) -> Action> = Rc::new(map);
            let model = self.model.clone();
            let activation_map = map.clone();
            ui.route_activation_read_fallible(
                reference.node(),
                self.value,
                move |current, activation| {
                    let value = model.step_by(*current, 1).map_err(|error| {
                        RuntimeError::new(format!("invalid slider step request: {error}"))
                    })?;
                    Ok(activation_map(ValueChange::committed(
                        value,
                        activation.source,
                    )))
                },
            )?;
            let model = self.model.clone();
            ui.route_value(reference.node(), move |fraction, phase, source| {
                let minimum = model.minimum().to_f64();
                let extent = model.maximum().to_f64() - minimum;
                let raw = T::from_f64(minimum + f64::from(fraction) * extent)
                    .expect("a normalized value must be representable by a valid range model");
                let value = model
                    .normalize(raw)
                    .expect("a normalized value must remain inside a valid range model");
                map(ValueChange::new(value, phase, source))
            })?;
        }
        Ok(reference)
    }
}

#[derive(Clone, Copy)]
struct SliderVisualContext {
    state: SliderInteractionState,
    minimum: crate::components::application::InteractiveTargetSize,
    orientation: SliderOrientation,
    reversed: bool,
    writing_direction: WritingDirection,
}

fn resolved_visual<T: RangeScalar>(
    style: SliderStyle,
    context: SliderVisualContext,
    model: &RangeModel<T>,
    value: T,
) -> SliderVisualStyle {
    let fraction = ((value.to_f64() - model.minimum().to_f64())
        / (model.maximum().to_f64() - model.minimum().to_f64()))
    .clamp(0.0, 1.0) as f32;
    let mut visual = style.resolve(context.state).visual;
    visual.container.min_size = SizeRule2D {
        width: SizeRule::Logical(context.minimum.width()),
        height: SizeRule::Logical(context.minimum.height()),
    };
    configure_visual_geometry(
        &mut visual,
        context.orientation,
        fraction,
        context.reversed,
        context.writing_direction,
    );
    visual
}

fn configure_visual_geometry(
    visual: &mut SliderVisualStyle,
    orientation: SliderOrientation,
    fraction: f32,
    reversed: bool,
    writing_direction: WritingDirection,
) {
    let horizontal_forward = writing_direction == WritingDirection::LeftToRight;
    let visual_fraction = match orientation {
        SliderOrientation::Horizontal if horizontal_forward == reversed => 1.0 - fraction,
        SliderOrientation::Vertical if !reversed => 1.0 - fraction,
        SliderOrientation::Horizontal | SliderOrientation::Vertical => fraction,
    };
    match orientation {
        SliderOrientation::Horizontal => {
            let cross_axis_offset = (visual.track_thickness - visual.thumb_size) * 0.5;
            visual.track.width = SizeRule::Logical(visual.track_length);
            visual.track.height = SizeRule::Logical(visual.track_thickness);
            visual.fill.width = SizeRule::Logical(visual.track_length * visual_fraction);
            visual.fill.height = SizeRule::Logical(visual.track_thickness);
            visual.thumb.width = SizeRule::Logical(visual.thumb_size);
            visual.thumb.height = SizeRule::Logical(visual.thumb_size);
            visual.thumb.max_size = SizeRule2D {
                width: SizeRule::Logical(visual.thumb_size),
                height: SizeRule::Logical(visual.thumb_size),
            };
            visual.thumb.transform = Transform2D {
                translation: PointF {
                    x: (visual.track_length - visual.thumb_size) * visual_fraction,
                    y: cross_axis_offset,
                },
                ..Transform2D::default()
            };
        }
        SliderOrientation::Vertical => {
            let cross_axis_offset = (visual.track_thickness - visual.thumb_size) * 0.5;
            visual.track.width = SizeRule::Logical(visual.track_thickness);
            visual.track.height = SizeRule::Logical(visual.track_length);
            visual.fill.width = SizeRule::Logical(visual.track_thickness);
            visual.fill.height = SizeRule::Logical(visual.track_length * fraction);
            visual.thumb.width = SizeRule::Logical(visual.thumb_size);
            visual.thumb.height = SizeRule::Logical(visual.thumb_size);
            visual.thumb.max_size = SizeRule2D {
                width: SizeRule::Logical(visual.thumb_size),
                height: SizeRule::Logical(visual.thumb_size),
            };
            visual.thumb.transform = Transform2D {
                translation: PointF {
                    x: cross_axis_offset,
                    y: (visual.track_length - visual.thumb_size) * visual_fraction,
                },
                ..Transform2D::default()
            };
        }
    }
}

#[derive(Clone, Debug)]
pub struct SliderRef<T: 'static> {
    control: ControlHandle,
    track: ControlHandle,
    fill: ControlHandle,
    thumb: ControlHandle,
    value: Read<T>,
    behavior: Rc<RefCell<SliderBehavior<T>>>,
}

impl<T> SliderRef<T>
where
    T: RangeScalar,
{
    pub const fn node(&self) -> UiNodeId {
        self.control.node
    }

    pub const fn value(&self) -> Read<T> {
        self.value
    }

    pub const fn enabled(&self) -> Property<bool> {
        self.control.enabled
    }

    pub const fn style(&self) -> Property<BoxStyle> {
        self.control.style
    }

    pub const fn fill_node(&self) -> UiNodeId {
        self.fill.node
    }

    pub const fn track_node(&self) -> UiNodeId {
        self.track.node
    }

    pub const fn thumb_node(&self) -> UiNodeId {
        self.thumb.node
    }

    pub fn request(
        &self,
        current: T,
        command: SliderCommand,
        source: ChangeSource,
    ) -> Result<Option<ValueChange<T>>, SliderError> {
        self.behavior.borrow().request(current, command, source)
    }

    pub fn handle_pointer(
        &self,
        current: T,
        input: GestureInput,
        track: SliderTrackGeometry,
    ) -> Result<SliderPointerOutcome<T>, SliderError> {
        self.behavior
            .borrow_mut()
            .handle_pointer(current, input, track)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SliderError {
    MissingAccessibleName,
    InvalidTrackGeometry,
    UnrepresentablePosition,
    Model(RangeModelError),
    Gesture(GestureRecognizerError),
}

impl From<RangeModelError> for SliderError {
    fn from(error: RangeModelError) -> Self {
        Self::Model(error)
    }
}

impl std::fmt::Display for SliderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingAccessibleName => formatter.write_str("slider accessible name is empty"),
            Self::InvalidTrackGeometry => {
                formatter.write_str("slider track geometry must be finite and positive")
            }
            Self::UnrepresentablePosition => {
                formatter.write_str("slider pointer position cannot be represented")
            }
            Self::Model(error) => write!(formatter, "invalid slider range value: {error}"),
            Self::Gesture(error) => write!(formatter, "invalid slider gesture: {error:?}"),
        }
    }
}

impl std::error::Error for SliderError {}

#[cfg(test)]
mod tests;

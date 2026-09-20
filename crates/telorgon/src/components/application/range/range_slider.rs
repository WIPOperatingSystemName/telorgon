//! Controlled two-thumb range slider composed over the shared range model and slider behavior.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use crate::foundation::{PointF, Transform2D};
use crate::input::{ChangeSource, GestureArenaRequest, GestureInput, WritingDirection};
use crate::runtime::{Read, RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    BoxStyle, ControlHandle, Flow, LayoutStyle, Property, SemanticActions, SemanticName,
    SemanticNode, SemanticRelationship, SemanticRelationshipKind, SemanticRole, SemanticState,
    SemanticValue, SizeRule, SizeRule2D, UiNodeId,
};

use crate::components::application::{
    ChangePhase, DensityMetrics, RangeModel, RangeModelError, RangeScalar, SliderBehavior,
    SliderCommand, SliderError, SliderInteractionState, SliderOrientation, SliderStyle,
    SliderTrackGeometry,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RangeSliderValue<T> {
    lower: T,
    upper: T,
}

impl<T> RangeSliderValue<T> {
    pub const fn new(lower: T, upper: T) -> Self {
        Self { lower, upper }
    }

    pub const fn lower(&self) -> &T {
        &self.lower
    }

    pub const fn upper(&self) -> &T {
        &self.upper
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RangeSliderThumb {
    Lower,
    Upper,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RangeSliderCrossingPolicy {
    #[default]
    Clamp,
    Swap,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RangeSliderProposal<T> {
    value: RangeSliderValue<T>,
    requested_thumb: RangeSliderThumb,
    active_thumb: RangeSliderThumb,
    role_swapped: bool,
    phase: ChangePhase,
    source: ChangeSource,
}

impl<T> RangeSliderProposal<T> {
    pub const fn value(&self) -> &RangeSliderValue<T> {
        &self.value
    }

    pub const fn requested_thumb(&self) -> RangeSliderThumb {
        self.requested_thumb
    }

    pub const fn active_thumb(&self) -> RangeSliderThumb {
        self.active_thumb
    }

    pub const fn role_swapped(&self) -> bool {
        self.role_swapped
    }

    pub const fn phase(&self) -> ChangePhase {
        self.phase
    }

    pub const fn source(&self) -> ChangeSource {
        self.source
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RangeSliderPointerOutcome<T> {
    pub proposal: Option<RangeSliderProposal<T>>,
    pub arena: GestureArenaRequest,
}

#[derive(Clone, Debug)]
pub struct RangeSliderBehavior<T> {
    model: RangeModel<T>,
    crossing: RangeSliderCrossingPolicy,
    lower: SliderBehavior<T>,
    upper: SliderBehavior<T>,
}

impl<T> RangeSliderBehavior<T>
where
    T: RangeScalar,
{
    pub fn new(
        model: RangeModel<T>,
        crossing: RangeSliderCrossingPolicy,
        orientation: SliderOrientation,
        writing_direction: WritingDirection,
        reversed: bool,
        enabled: bool,
    ) -> Result<Self, RangeSliderError> {
        let lower = SliderBehavior::new(
            model.clone(),
            orientation,
            writing_direction,
            reversed,
            enabled,
        )?;
        let upper = SliderBehavior::new(
            model.clone(),
            orientation,
            writing_direction,
            reversed,
            enabled,
        )?;
        Ok(Self {
            model,
            crossing,
            lower,
            upper,
        })
    }

    pub const fn model(&self) -> &RangeModel<T> {
        &self.model
    }

    pub const fn crossing_policy(&self) -> RangeSliderCrossingPolicy {
        self.crossing
    }

    pub fn validate_value(&self, value: RangeSliderValue<T>) -> Result<(), RangeSliderError> {
        self.model.format_value(value.lower)?;
        self.model.format_value(value.upper)?;
        if value.lower.to_f64() > value.upper.to_f64() {
            return Err(RangeSliderError::UnorderedControlledValue);
        }
        Ok(())
    }

    pub fn request(
        &self,
        current: RangeSliderValue<T>,
        thumb: RangeSliderThumb,
        command: SliderCommand,
        source: ChangeSource,
    ) -> Result<Option<RangeSliderProposal<T>>, RangeSliderError> {
        self.validate_value(current)?;
        let selected = selected_value(current, thumb);
        let change = self.behavior(thumb).request(selected, command, source)?;
        let Some(change) = change else {
            return Ok(None);
        };
        let proposal =
            self.resolve_target(current, thumb, change.value, change.phase, change.source)?;
        if proposal.value == current {
            Ok(None)
        } else {
            Ok(Some(proposal))
        }
    }

    pub fn propose(
        &self,
        current: RangeSliderValue<T>,
        thumb: RangeSliderThumb,
        target: T,
        phase: ChangePhase,
        source: ChangeSource,
    ) -> Result<RangeSliderProposal<T>, RangeSliderError> {
        self.validate_value(current)?;
        let target = self.model.normalize(target)?;
        self.resolve_target(current, thumb, target, phase, source)
    }

    pub fn handle_pointer(
        &mut self,
        current: RangeSliderValue<T>,
        thumb: RangeSliderThumb,
        input: GestureInput,
        track: SliderTrackGeometry,
    ) -> Result<RangeSliderPointerOutcome<T>, RangeSliderError> {
        self.validate_value(current)?;
        let selected = selected_value(current, thumb);
        let outcome = self
            .behavior_mut(thumb)
            .handle_pointer(selected, input, track)?;
        let proposal = outcome
            .change
            .map(|change| {
                self.resolve_target(current, thumb, change.value, change.phase, change.source)
            })
            .transpose()?;
        Ok(RangeSliderPointerOutcome {
            proposal,
            arena: outcome.arena,
        })
    }

    fn behavior(&self, thumb: RangeSliderThumb) -> &SliderBehavior<T> {
        match thumb {
            RangeSliderThumb::Lower => &self.lower,
            RangeSliderThumb::Upper => &self.upper,
        }
    }

    fn behavior_mut(&mut self, thumb: RangeSliderThumb) -> &mut SliderBehavior<T> {
        match thumb {
            RangeSliderThumb::Lower => &mut self.lower,
            RangeSliderThumb::Upper => &mut self.upper,
        }
    }

    fn resolve_target(
        &self,
        current: RangeSliderValue<T>,
        thumb: RangeSliderThumb,
        target: T,
        phase: ChangePhase,
        source: ChangeSource,
    ) -> Result<RangeSliderProposal<T>, RangeSliderError> {
        let target = self.model.normalize(target)?;
        let (value, active_thumb, role_swapped) = match (thumb, self.crossing) {
            (RangeSliderThumb::Lower, RangeSliderCrossingPolicy::Clamp) => (
                RangeSliderValue::new(
                    if target.to_f64() > current.upper.to_f64() {
                        current.upper
                    } else {
                        target
                    },
                    current.upper,
                ),
                RangeSliderThumb::Lower,
                false,
            ),
            (RangeSliderThumb::Upper, RangeSliderCrossingPolicy::Clamp) => (
                RangeSliderValue::new(
                    current.lower,
                    if target.to_f64() < current.lower.to_f64() {
                        current.lower
                    } else {
                        target
                    },
                ),
                RangeSliderThumb::Upper,
                false,
            ),
            (RangeSliderThumb::Lower, RangeSliderCrossingPolicy::Swap)
                if target.to_f64() > current.upper.to_f64() =>
            {
                (
                    RangeSliderValue::new(current.upper, target),
                    RangeSliderThumb::Upper,
                    true,
                )
            }
            (RangeSliderThumb::Upper, RangeSliderCrossingPolicy::Swap)
                if target.to_f64() < current.lower.to_f64() =>
            {
                (
                    RangeSliderValue::new(target, current.lower),
                    RangeSliderThumb::Lower,
                    true,
                )
            }
            (RangeSliderThumb::Lower, RangeSliderCrossingPolicy::Swap) => (
                RangeSliderValue::new(target, current.upper),
                RangeSliderThumb::Lower,
                false,
            ),
            (RangeSliderThumb::Upper, RangeSliderCrossingPolicy::Swap) => (
                RangeSliderValue::new(current.lower, target),
                RangeSliderThumb::Upper,
                false,
            ),
        };
        Ok(RangeSliderProposal {
            value,
            requested_thumb: thumb,
            active_thumb,
            role_swapped,
            phase,
            source,
        })
    }
}

fn selected_value<T: Copy>(value: RangeSliderValue<T>, thumb: RangeSliderThumb) -> T {
    match thumb {
        RangeSliderThumb::Lower => value.lower,
        RangeSliderThumb::Upper => value.upper,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RangeSlider<T: 'static> {
    label: String,
    lower_label: String,
    upper_label: String,
    value: Read<RangeSliderValue<T>>,
    model: RangeModel<T>,
    crossing: RangeSliderCrossingPolicy,
    enabled: bool,
    orientation: SliderOrientation,
    writing_direction: WritingDirection,
    reversed: bool,
    density: DensityMetrics,
    style: SliderStyle,
}

impl<T> RangeSlider<T>
where
    T: RangeScalar,
{
    pub fn new(
        label: impl Into<String>,
        lower_label: impl Into<String>,
        upper_label: impl Into<String>,
        value: Read<RangeSliderValue<T>>,
        model: RangeModel<T>,
    ) -> Result<Self, RangeSliderError> {
        let label = label.into();
        let lower_label = lower_label.into();
        let upper_label = upper_label.into();
        for (name, value) in [
            (RangeSliderName::Group, label.as_str()),
            (RangeSliderName::LowerThumb, lower_label.as_str()),
            (RangeSliderName::UpperThumb, upper_label.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(RangeSliderError::MissingAccessibleName(name));
            }
        }
        if lower_label == upper_label {
            return Err(RangeSliderError::DuplicateThumbNames);
        }
        Ok(Self {
            label,
            lower_label,
            upper_label,
            value,
            model,
            crossing: RangeSliderCrossingPolicy::Clamp,
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

    pub const fn crossing_policy(mut self, crossing: RangeSliderCrossingPolicy) -> Self {
        self.crossing = crossing;
        self
    }

    pub const fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub const fn orientation(mut self, orientation: SliderOrientation) -> Self {
        self.orientation = orientation;
        self
    }

    pub const fn writing_direction(mut self, writing_direction: WritingDirection) -> Self {
        self.writing_direction = writing_direction;
        self
    }

    pub const fn reversed(mut self, reversed: bool) -> Self {
        self.reversed = reversed;
        self
    }

    pub const fn density(mut self, density: DensityMetrics) -> Self {
        self.density = density;
        self
    }

    pub const fn style(mut self, style: SliderStyle) -> Self {
        self.style = style;
        self
    }

    pub fn behavior(&self) -> Result<RangeSliderBehavior<T>, RangeSliderError> {
        RangeSliderBehavior::new(
            self.model.clone(),
            self.crossing,
            self.orientation,
            self.writing_direction,
            self.reversed,
            self.enabled,
        )
    }

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
    ) -> RuntimeResult<RangeSliderRef<T>> {
        let value = ui.read(self.value)?;
        self.behavior()
            .and_then(|behavior| behavior.validate_value(value))
            .map_err(|error| {
                RuntimeError::new(format!("invalid controlled range-slider value: {error}"))
            })?;
        let lower_text = self
            .model
            .format_value(value.lower)
            .map_err(range_runtime_error)?;
        let upper_text = self
            .model
            .format_value(value.upper)
            .map_err(range_runtime_error)?;
        let minimum = self.model.minimum().to_f64();
        let extent = self.model.maximum().to_f64() - minimum;
        let lower_fraction = ((value.lower.to_f64() - minimum) / extent).clamp(0.0, 1.0) as f32;
        let upper_fraction = ((value.upper.to_f64() - minimum) / extent).clamp(0.0, 1.0) as f32;
        let mut visual = self
            .style
            .resolve(SliderInteractionState::resting(self.enabled))
            .visual;
        let target = self.density.effective_minimum();
        visual.container.min_size = SizeRule2D {
            width: SizeRule::Logical(target.width()),
            height: SizeRule::Logical(target.height()),
        };
        let (lower_position, upper_position) = visual_positions(
            lower_fraction,
            upper_fraction,
            self.orientation,
            self.reversed,
            self.writing_direction,
        );
        configure_track(
            &mut visual.track,
            self.orientation,
            visual.track_length,
            visual.track_thickness,
        );
        let mut fill = visual.fill;
        let mut lower_thumb = visual.thumb;
        let mut upper_thumb = visual.thumb;
        configure_range_visual(
            &mut fill,
            &mut lower_thumb,
            &mut upper_thumb,
            self.orientation,
            visual.track_length,
            visual.track_thickness,
            visual.thumb_size,
            lower_position,
            upper_position,
            target.width(),
            target.height(),
        );

        let label = self.label.clone();
        let content_layout = LayoutStyle {
            flow: match self.orientation {
                SliderOrientation::Horizontal => Flow::Horizontal,
                SliderOrientation::Vertical => Flow::Vertical,
            },
            gap: visual.gap,
            ..LayoutStyle::default()
        };
        let mut thumbs = None;
        let root = ui
            .foundation()
            .container_node_under(host, visual.container, content_layout, |writer| {
                writer.text(label, visual.label_color, visual.label_size);
                writer.container(
                    visual.track,
                    LayoutStyle {
                        flow: Flow::Overlay,
                        ..LayoutStyle::default()
                    },
                    |writer| {
                        writer.container(fill, LayoutStyle::default(), |_| {});
                        let lower = writer.action_node(lower_thumb, self.enabled, |_| {});
                        let upper = writer.action_node(upper_thumb, self.enabled, |_| {});
                        thumbs = Some((lower, upper));
                    },
                );
            })
            .ok_or_else(|| RuntimeError::new("application range-slider host is stale"))?;
        let (lower, upper) = thumbs
            .ok_or_else(|| RuntimeError::new("application range-slider thumbs were not mounted"))?;

        let lower_name = ui.foundation().intern(&self.lower_label);
        let upper_name = ui.foundation().intern(&self.upper_label);
        let lower_text = ui.foundation().intern(&lower_text);
        let upper_text = ui.foundation().intern(&upper_text);
        ui.foundation()
            .semantic_node(
                lower.node,
                self.thumb_semantics(RangeSliderThumb::Lower, value, lower_name, lower_text),
            )
            .map_err(semantic_runtime_error)?;
        ui.foundation()
            .semantic_node(
                upper.node,
                self.thumb_semantics(RangeSliderThumb::Upper, value, upper_name, upper_text),
            )
            .map_err(semantic_runtime_error)?;
        let root_name = ui.foundation().intern(&self.label);
        ui.foundation()
            .semantic_node(
                root.node,
                SemanticNode {
                    role: SemanticRole::Generic,
                    name: SemanticName::Text(root_name),
                    state: SemanticState {
                        disabled: !self.enabled,
                        ..SemanticState::default()
                    },
                    relationships: vec![
                        SemanticRelationship {
                            kind: SemanticRelationshipKind::Owns,
                            target: lower.node,
                        },
                        SemanticRelationship {
                            kind: SemanticRelationshipKind::Owns,
                            target: upper.node,
                        },
                    ],
                    ..SemanticNode::default()
                },
            )
            .map_err(semantic_runtime_error)?;
        if !self.enabled {
            ui.foundation().disabled(root.node, true);
            ui.foundation().disabled(lower.node, true);
            ui.foundation().disabled(upper.node, true);
        }
        Ok(RangeSliderRef {
            root,
            lower,
            upper,
            value: self.value,
            behavior: Rc::new(RefCell::new(self.behavior().map_err(|error| {
                RuntimeError::new(format!("invalid range-slider behavior: {error}"))
            })?)),
        })
    }

    fn thumb_semantics(
        &self,
        thumb: RangeSliderThumb,
        value: RangeSliderValue<T>,
        name: crate::ui::StringId,
        value_text: crate::ui::StringId,
    ) -> SemanticNode {
        let (current, minimum, maximum) = match (thumb, self.crossing) {
            (RangeSliderThumb::Lower, RangeSliderCrossingPolicy::Clamp) => {
                (value.lower, self.model.minimum(), value.upper)
            }
            (RangeSliderThumb::Upper, RangeSliderCrossingPolicy::Clamp) => {
                (value.upper, value.lower, self.model.maximum())
            }
            (RangeSliderThumb::Lower, RangeSliderCrossingPolicy::Swap) => {
                (value.lower, self.model.minimum(), self.model.maximum())
            }
            (RangeSliderThumb::Upper, RangeSliderCrossingPolicy::Swap) => {
                (value.upper, self.model.minimum(), self.model.maximum())
            }
        };
        SemanticNode {
            role: SemanticRole::Slider,
            name: SemanticName::Text(name),
            state: SemanticState {
                disabled: !self.enabled,
                focusable: self.enabled,
                ..SemanticState::default()
            },
            value: SemanticValue::Number {
                current: current.to_f64(),
                minimum: minimum.to_f64(),
                maximum: maximum.to_f64(),
                step: Some(self.model.step().to_f64()),
                value_text: Some(value_text),
            },
            actions: if self.enabled {
                SemanticActions::FOCUS
                    | SemanticActions::INCREMENT
                    | SemanticActions::DECREMENT
                    | SemanticActions::SET_VALUE
            } else {
                SemanticActions::NONE
            },
            ..SemanticNode::default()
        }
    }
}

fn range_runtime_error(error: RangeModelError) -> RuntimeError {
    RuntimeError::new(format!("invalid controlled range-slider value: {error}"))
}

fn semantic_runtime_error(error: crate::ui::SemanticError) -> RuntimeError {
    RuntimeError::new(format!("invalid range-slider semantics: {error:?}"))
}

fn configure_track(
    track: &mut BoxStyle,
    orientation: SliderOrientation,
    length: f32,
    thickness: f32,
) {
    match orientation {
        SliderOrientation::Horizontal => {
            track.width = SizeRule::Logical(length);
            track.height = SizeRule::Logical(thickness);
        }
        SliderOrientation::Vertical => {
            track.width = SizeRule::Logical(thickness);
            track.height = SizeRule::Logical(length);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn configure_range_visual(
    fill: &mut BoxStyle,
    lower_thumb: &mut BoxStyle,
    upper_thumb: &mut BoxStyle,
    orientation: SliderOrientation,
    track_length: f32,
    track_thickness: f32,
    thumb_size: f32,
    lower_position: f32,
    upper_position: f32,
    target_width: f32,
    target_height: f32,
) {
    let start = lower_position.min(upper_position);
    let end = lower_position.max(upper_position);
    match orientation {
        SliderOrientation::Horizontal => {
            fill.width = SizeRule::Logical(track_length * (end - start));
            fill.height = SizeRule::Logical(track_thickness);
            fill.transform = translation(track_length * start, 0.0);
            for (thumb, position) in [(lower_thumb, lower_position), (upper_thumb, upper_position)]
            {
                thumb.min_size = SizeRule2D {
                    width: SizeRule::Logical(target_width),
                    height: SizeRule::Logical(target_height),
                };
                thumb.transform = translation((track_length - thumb_size) * position, 0.0);
            }
        }
        SliderOrientation::Vertical => {
            fill.width = SizeRule::Logical(track_thickness);
            fill.height = SizeRule::Logical(track_length * (end - start));
            fill.transform = translation(0.0, track_length * start);
            for (thumb, position) in [(lower_thumb, lower_position), (upper_thumb, upper_position)]
            {
                thumb.min_size = SizeRule2D {
                    width: SizeRule::Logical(target_width),
                    height: SizeRule::Logical(target_height),
                };
                thumb.transform = translation(0.0, (track_length - thumb_size) * position);
            }
        }
    }
}

fn translation(x: f32, y: f32) -> Transform2D {
    Transform2D {
        translation: PointF { x, y },
        ..Transform2D::default()
    }
}

fn visual_positions(
    lower: f32,
    upper: f32,
    orientation: SliderOrientation,
    reversed: bool,
    writing_direction: WritingDirection,
) -> (f32, f32) {
    let map = |fraction: f32| match orientation {
        SliderOrientation::Horizontal
            if (writing_direction == WritingDirection::LeftToRight) == reversed =>
        {
            1.0 - fraction
        }
        SliderOrientation::Vertical if !reversed => 1.0 - fraction,
        SliderOrientation::Horizontal | SliderOrientation::Vertical => fraction,
    };
    (map(lower), map(upper))
}

#[derive(Clone, Debug)]
pub struct RangeSliderRef<T: 'static> {
    root: ControlHandle,
    lower: ControlHandle,
    upper: ControlHandle,
    value: Read<RangeSliderValue<T>>,
    behavior: Rc<RefCell<RangeSliderBehavior<T>>>,
}

impl<T> RangeSliderRef<T>
where
    T: RangeScalar,
{
    pub const fn node(&self) -> UiNodeId {
        self.root.node
    }

    pub const fn thumb_node(&self, thumb: RangeSliderThumb) -> UiNodeId {
        match thumb {
            RangeSliderThumb::Lower => self.lower.node,
            RangeSliderThumb::Upper => self.upper.node,
        }
    }

    pub const fn value(&self) -> Read<RangeSliderValue<T>> {
        self.value
    }

    pub const fn style(&self) -> Property<BoxStyle> {
        self.root.style
    }

    pub fn request(
        &self,
        current: RangeSliderValue<T>,
        thumb: RangeSliderThumb,
        command: SliderCommand,
        source: ChangeSource,
    ) -> Result<Option<RangeSliderProposal<T>>, RangeSliderError> {
        self.behavior
            .borrow()
            .request(current, thumb, command, source)
    }

    pub fn propose(
        &self,
        current: RangeSliderValue<T>,
        thumb: RangeSliderThumb,
        target: T,
        phase: ChangePhase,
        source: ChangeSource,
    ) -> Result<RangeSliderProposal<T>, RangeSliderError> {
        self.behavior
            .borrow()
            .propose(current, thumb, target, phase, source)
    }

    pub fn handle_pointer(
        &self,
        current: RangeSliderValue<T>,
        thumb: RangeSliderThumb,
        input: GestureInput,
        track: SliderTrackGeometry,
    ) -> Result<RangeSliderPointerOutcome<T>, RangeSliderError> {
        self.behavior
            .borrow_mut()
            .handle_pointer(current, thumb, input, track)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeSliderName {
    Group,
    LowerThumb,
    UpperThumb,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RangeSliderError {
    MissingAccessibleName(RangeSliderName),
    DuplicateThumbNames,
    UnorderedControlledValue,
    Model(RangeModelError),
    Slider(SliderError),
}

impl From<RangeModelError> for RangeSliderError {
    fn from(error: RangeModelError) -> Self {
        Self::Model(error)
    }
}

impl From<SliderError> for RangeSliderError {
    fn from(error: SliderError) -> Self {
        Self::Slider(error)
    }
}

impl fmt::Display for RangeSliderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid range slider: {self:?}")
    }
}

impl std::error::Error for RangeSliderError {}

#[cfg(test)]
mod tests;

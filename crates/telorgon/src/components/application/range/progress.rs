//! Read-only Tier A progress/activity values, semantics, styles, and mounting.

use crate::foundation::{ColorRgba8, EdgeInsets, PointF, Transform2D};
use crate::runtime::{Read, RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    Background, BoxStyle, ControlHandle, CornerRadii, Flow, LayoutStyle, Property, SemanticActions,
    SemanticName, SemanticNode, SemanticRole, SemanticState, SemanticValue, SizeRule, StyleBinding,
    StyleSlotId, ThemeScopeId, UiNodeId,
};

use crate::components::application::{
    ActivityIndicatorStyleId, DensityClass, RangeModel, RangeModelError, RangeScalar,
};

/// Parent-owned progress mode and value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ProgressValue<T> {
    Determinate(T),
    Indeterminate,
}

impl<T> ProgressValue<T> {
    pub const fn mode(&self) -> ProgressMode {
        match self {
            Self::Determinate(_) => ProgressMode::Determinate,
            Self::Indeterminate => ProgressMode::Indeterminate,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgressMode {
    Determinate,
    Indeterminate,
}

/// Visual slots for one density and progress mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProgressVisualStyle {
    pub container: BoxStyle,
    pub track: BoxStyle,
    pub fill: BoxStyle,
    pub label_color: ColorRgba8,
    pub label_size: f32,
    pub gap: f32,
    pub track_length: f32,
    pub track_thickness: f32,
    pub indeterminate_segment_fraction: f32,
}

/// Determinate and indeterminate variants for one density class.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProgressDensityStyle {
    pub determinate: ProgressVisualStyle,
    pub indeterminate: ProgressVisualStyle,
}

/// Explicit Compact/Standard/Touch visual variants for progress indicators.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProgressStyle {
    pub compact: ProgressDensityStyle,
    pub standard: ProgressDensityStyle,
    pub touch: ProgressDensityStyle,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedProgressStyle {
    pub density: DensityClass,
    pub mode: ProgressMode,
    pub visual: ProgressVisualStyle,
}

impl ProgressStyle {
    pub const fn resolve(self, density: DensityClass, mode: ProgressMode) -> ResolvedProgressStyle {
        let density_style = match density {
            DensityClass::Compact => self.compact,
            DensityClass::Standard => self.standard,
            DensityClass::Touch => self.touch,
        };
        let visual = match mode {
            ProgressMode::Determinate => density_style.determinate,
            ProgressMode::Indeterminate => density_style.indeterminate,
        };
        ResolvedProgressStyle {
            density,
            mode,
            visual,
        }
    }
}

impl Default for ProgressStyle {
    fn default() -> Self {
        fn visual(
            label_size: f32,
            gap: f32,
            track_thickness: f32,
            fill: ColorRgba8,
        ) -> ProgressVisualStyle {
            ProgressVisualStyle {
                container: BoxStyle {
                    padding: EdgeInsets::all(gap),
                    ..BoxStyle::default()
                },
                track: BoxStyle {
                    decoration: crate::ui::BoxDecoration {
                        background: Background::Color(ColorRgba8::rgba(76, 84, 101, 255)),
                        corner_radii: CornerRadii::all(track_thickness * 0.5),
                        ..crate::ui::BoxDecoration::default()
                    },
                    ..BoxStyle::default()
                },
                fill: BoxStyle {
                    decoration: crate::ui::BoxDecoration {
                        background: Background::Color(fill),
                        corner_radii: CornerRadii::all(track_thickness * 0.5),
                        ..crate::ui::BoxDecoration::default()
                    },
                    ..BoxStyle::default()
                },
                label_color: ColorRgba8::rgba(235, 238, 244, 255),
                label_size,
                gap,
                track_length: 160.0,
                track_thickness,
                indeterminate_segment_fraction: 0.3,
            }
        }

        fn density(label_size: f32, gap: f32, thickness: f32) -> ProgressDensityStyle {
            ProgressDensityStyle {
                determinate: visual(
                    label_size,
                    gap,
                    thickness,
                    ColorRgba8::rgba(54, 104, 210, 255),
                ),
                indeterminate: visual(
                    label_size,
                    gap,
                    thickness,
                    ColorRgba8::rgba(93, 132, 218, 255),
                ),
            }
        }

        Self {
            compact: density(12.0, 4.0, 3.0),
            standard: density(14.0, 6.0, 4.0),
            touch: density(16.0, 8.0, 6.0),
        }
    }
}

/// Parent-controlled lifecycle state for an activity indicator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivityIndicatorState {
    Running,
    Inactive,
}

impl From<bool> for ActivityIndicatorState {
    fn from(active: bool) -> Self {
        if active {
            Self::Running
        } else {
            Self::Inactive
        }
    }
}

/// Motion preference supplied by the neutral application environment owner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ActivityMotionPreference {
    #[default]
    Standard,
    Reduced,
}

/// Declarative motion intent for a scheduling owner; this component never advances a clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivityMotionStyle {
    Static,
    Rotate { cycle_millis: u32 },
}

/// Visual slots for one activity state, density, and motion preference.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActivityIndicatorVisualStyle {
    pub container: BoxStyle,
    pub track: BoxStyle,
    pub marker: BoxStyle,
    pub label_color: ColorRgba8,
    pub label_size: f32,
    pub gap: f32,
    pub indicator_size: f32,
    pub marker_size: f32,
    pub motion: ActivityMotionStyle,
}

/// Running/inactive and standard/reduced-motion variants for one density class.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActivityIndicatorDensityStyle {
    pub running: ActivityIndicatorVisualStyle,
    pub inactive: ActivityIndicatorVisualStyle,
    pub reduced_motion_running: ActivityIndicatorVisualStyle,
    pub reduced_motion_inactive: ActivityIndicatorVisualStyle,
}

/// Explicit Compact/Standard/Touch visual variants for activity indicators.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActivityIndicatorStyle {
    pub compact: ActivityIndicatorDensityStyle,
    pub standard: ActivityIndicatorDensityStyle,
    pub touch: ActivityIndicatorDensityStyle,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedActivityIndicatorStyle {
    pub density: DensityClass,
    pub state: ActivityIndicatorState,
    pub motion_preference: ActivityMotionPreference,
    pub visual: ActivityIndicatorVisualStyle,
}

impl ActivityIndicatorStyle {
    pub const fn resolve(
        self,
        density: DensityClass,
        state: ActivityIndicatorState,
        motion_preference: ActivityMotionPreference,
    ) -> ResolvedActivityIndicatorStyle {
        let density_style = match density {
            DensityClass::Compact => self.compact,
            DensityClass::Standard => self.standard,
            DensityClass::Touch => self.touch,
        };
        let visual = match (state, motion_preference) {
            (ActivityIndicatorState::Running, ActivityMotionPreference::Standard) => {
                density_style.running
            }
            (ActivityIndicatorState::Inactive, ActivityMotionPreference::Standard) => {
                density_style.inactive
            }
            (ActivityIndicatorState::Running, ActivityMotionPreference::Reduced) => {
                density_style.reduced_motion_running
            }
            (ActivityIndicatorState::Inactive, ActivityMotionPreference::Reduced) => {
                density_style.reduced_motion_inactive
            }
        };
        ResolvedActivityIndicatorStyle {
            density,
            state,
            motion_preference,
            visual,
        }
    }
}

impl Default for ActivityIndicatorStyle {
    fn default() -> Self {
        fn visual(
            label_size: f32,
            gap: f32,
            indicator_size: f32,
            marker_size: f32,
            track_opacity: f32,
            marker_opacity: f32,
            motion: ActivityMotionStyle,
        ) -> ActivityIndicatorVisualStyle {
            ActivityIndicatorVisualStyle {
                container: BoxStyle {
                    padding: EdgeInsets::all(gap),
                    ..BoxStyle::default()
                },
                track: BoxStyle {
                    decoration: crate::ui::BoxDecoration {
                        background: Background::Color(ColorRgba8::rgba(76, 84, 101, 255)),
                        corner_radii: CornerRadii::all(indicator_size * 0.5),
                        ..crate::ui::BoxDecoration::default()
                    },
                    opacity: track_opacity,
                    ..BoxStyle::default()
                },
                marker: BoxStyle {
                    decoration: crate::ui::BoxDecoration {
                        background: Background::Color(ColorRgba8::rgba(93, 132, 218, 255)),
                        corner_radii: CornerRadii::all(marker_size * 0.5),
                        ..crate::ui::BoxDecoration::default()
                    },
                    opacity: marker_opacity,
                    ..BoxStyle::default()
                },
                label_color: ColorRgba8::rgba(235, 238, 244, 255),
                label_size,
                gap,
                indicator_size,
                marker_size,
                motion,
            }
        }

        fn density(
            label_size: f32,
            gap: f32,
            indicator_size: f32,
        ) -> ActivityIndicatorDensityStyle {
            let marker_size = indicator_size * 0.3;
            ActivityIndicatorDensityStyle {
                running: visual(
                    label_size,
                    gap,
                    indicator_size,
                    marker_size,
                    0.45,
                    1.0,
                    ActivityMotionStyle::Rotate { cycle_millis: 900 },
                ),
                inactive: visual(
                    label_size,
                    gap,
                    indicator_size,
                    marker_size,
                    0.25,
                    0.0,
                    ActivityMotionStyle::Static,
                ),
                reduced_motion_running: visual(
                    label_size,
                    gap,
                    indicator_size,
                    marker_size,
                    0.7,
                    1.0,
                    ActivityMotionStyle::Static,
                ),
                reduced_motion_inactive: visual(
                    label_size,
                    gap,
                    indicator_size,
                    marker_size,
                    0.25,
                    0.0,
                    ActivityMotionStyle::Static,
                ),
            }
        }

        Self {
            compact: density(12.0, 4.0, 12.0),
            standard: density(14.0, 6.0, 16.0),
            touch: density(16.0, 8.0, 20.0),
        }
    }
}

/// Immutable configuration for a named, parent-controlled progress indicator.
#[derive(Clone, Debug, PartialEq)]
pub struct ProgressIndicator<T: 'static> {
    label: String,
    value: Read<ProgressValue<T>>,
    model: RangeModel<T>,
    density: DensityClass,
    style: ProgressStyle,
}

impl<T> ProgressIndicator<T>
where
    T: RangeScalar,
{
    pub fn new(
        label: impl Into<String>,
        value: Read<ProgressValue<T>>,
        model: RangeModel<T>,
    ) -> Result<Self, ProgressError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(ProgressError::MissingAccessibleName);
        }
        Ok(Self {
            label,
            value,
            model,
            density: DensityClass::Standard,
            style: ProgressStyle::default(),
        })
    }

    pub fn density(mut self, density: DensityClass) -> Self {
        self.density = density;
        self
    }

    pub fn style(mut self, style: ProgressStyle) -> Self {
        self.style = style;
        self
    }

    pub const fn value(&self) -> Read<ProgressValue<T>> {
        self.value
    }

    pub const fn model(&self) -> &RangeModel<T> {
        &self.model
    }

    pub fn semantic_node(
        &self,
        name: crate::ui::StringId,
        value_text: Option<crate::ui::StringId>,
        value: ProgressValue<T>,
    ) -> Result<SemanticNode, ProgressError> {
        let (busy, semantic_value) = match value {
            ProgressValue::Determinate(value) => {
                self.model.format_value(value)?;
                (
                    false,
                    SemanticValue::Number {
                        current: value.to_f64(),
                        minimum: self.model.minimum().to_f64(),
                        maximum: self.model.maximum().to_f64(),
                        step: Some(self.model.step().to_f64()),
                        value_text,
                    },
                )
            }
            ProgressValue::Indeterminate => (true, SemanticValue::None),
        };
        Ok(SemanticNode {
            role: SemanticRole::ProgressIndicator,
            name: SemanticName::Text(name),
            state: SemanticState {
                busy,
                ..SemanticState::default()
            },
            value: semantic_value,
            actions: SemanticActions::NONE,
            ..SemanticNode::default()
        })
    }

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
    ) -> RuntimeResult<ProgressRef<T>> {
        let value = ui.read(self.value)?;
        let value_text = match value {
            ProgressValue::Determinate(value) => {
                Some(self.model.format_value(value).map_err(|error| {
                    RuntimeError::new(format!("invalid controlled progress value: {error}"))
                })?)
            }
            ProgressValue::Indeterminate => None,
        };
        let visual = resolved_progress_visual(self.style, self.density, value, &self.model);
        let label = self.label.clone();
        let label_color = visual.label_color;
        let label_size = visual.label_size;
        let row = LayoutStyle {
            flow: Flow::Horizontal,
            gap: visual.gap,
            ..LayoutStyle::default()
        };
        let mut fill_control = None;
        let mut label_control = None;
        let control = ui
            .foundation()
            .container_node_under(host, visual.container, row, |writer| {
                label_control = Some(writer.text(&label, label_color, label_size));
                writer.container_handle(
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
                    },
                );
            })
            .ok_or_else(|| RuntimeError::new("application progress host is stale"))?;
        let fill_control = fill_control.expect("progress fill mounts with its track");
        let label_control = label_control.expect("progress label mounts with its control");

        let name = ui.foundation().intern(&self.label);
        let value_text = value_text.map(|value| ui.foundation().intern(value));
        let semantic = self
            .semantic_node(name, value_text, value)
            .map_err(|error| RuntimeError::new(format!("invalid progress semantics: {error}")))?;
        ui.foundation()
            .semantic_node(control.node, semantic)
            .map_err(|error| RuntimeError::new(format!("invalid progress semantics: {error:?}")))?;
        let read = self.value;
        let style = self.style;
        let density = self.density;
        let model = self.model.clone();
        ui.bind_map(read, control.style, move |value| {
            resolved_progress_visual(style, density, *value, &model).container
        })?;
        let style = self.style;
        let model = self.model.clone();
        ui.bind_map(read, fill_control.style, move |value| {
            resolved_progress_visual(style, density, *value, &model).fill
        })?;
        let style = self.style;
        ui.bind_map(read, label_control.color, move |value| {
            style.resolve(density, value.mode()).visual.label_color
        })?;
        ui.bind_map(read, control.value, |value| match value {
            ProgressValue::Determinate(value) => value.to_f64() as f32,
            ProgressValue::Indeterminate => 0.0,
        })?;
        ui.bind_map(read, control.busy, |value| {
            matches!(value, ProgressValue::Indeterminate)
        })?;

        Ok(ProgressRef {
            control,
            fill: fill_control,
            value: self.value,
        })
    }
}

fn resolved_progress_visual<T: RangeScalar>(
    style: ProgressStyle,
    density: DensityClass,
    value: ProgressValue<T>,
    model: &RangeModel<T>,
) -> ProgressVisualStyle {
    let mut visual = style.resolve(density, value.mode()).visual;
    configure_progress_geometry(&mut visual, value, model);
    visual
}

fn configure_progress_geometry<T: RangeScalar>(
    visual: &mut ProgressVisualStyle,
    value: ProgressValue<T>,
    model: &RangeModel<T>,
) {
    visual.track.width = SizeRule::Logical(visual.track_length);
    visual.track.height = SizeRule::Logical(visual.track_thickness);
    visual.fill.height = SizeRule::Logical(visual.track_thickness);
    let (fraction, offset) = match value {
        ProgressValue::Determinate(value) => {
            let fraction = ((value.to_f64() - model.minimum().to_f64())
                / (model.maximum().to_f64() - model.minimum().to_f64()))
            .clamp(0.0, 1.0) as f32;
            (fraction, 0.0)
        }
        ProgressValue::Indeterminate => (
            visual.indeterminate_segment_fraction.clamp(0.0, 1.0),
            visual.track_length * 0.1,
        ),
    };
    visual.fill.width = SizeRule::Logical(visual.track_length * fraction);
    visual.fill.transform = Transform2D {
        translation: PointF { x: offset, y: 0.0 },
        ..Transform2D::default()
    };
}

/// Focused advanced reference returned by progress mounting.
#[derive(Clone, Copy, Debug)]
pub struct ProgressRef<T: 'static> {
    control: ControlHandle,
    fill: ControlHandle,
    value: Read<ProgressValue<T>>,
}

impl<T: 'static> ProgressRef<T> {
    pub const fn node(self) -> UiNodeId {
        self.control.node
    }

    pub const fn value(self) -> Read<ProgressValue<T>> {
        self.value
    }

    pub const fn style(self) -> Property<BoxStyle> {
        self.control.style
    }

    pub const fn fill_node(self) -> UiNodeId {
        self.fill.node
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgressError {
    MissingAccessibleName,
    Model(RangeModelError),
}

impl From<RangeModelError> for ProgressError {
    fn from(error: RangeModelError) -> Self {
        Self::Model(error)
    }
}

impl std::fmt::Display for ProgressError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingAccessibleName => {
                formatter.write_str("progress indicator accessible name is empty")
            }
            Self::Model(error) => write!(formatter, "invalid progress range value: {error}"),
        }
    }
}

impl std::error::Error for ProgressError {}

/// Immutable configuration for a named, parent-controlled activity indicator.
#[derive(Clone, Debug, PartialEq)]
pub struct ActivityIndicator {
    label: String,
    active: Read<bool>,
    density: DensityClass,
    motion_preference: ActivityMotionPreference,
    style: ActivityIndicatorStyle,
    style_id: ActivityIndicatorStyleId,
}

impl ActivityIndicator {
    pub fn new(
        label: impl Into<String>,
        active: Read<bool>,
    ) -> Result<Self, ActivityIndicatorError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(ActivityIndicatorError::MissingAccessibleName);
        }
        Ok(Self {
            label,
            active,
            density: DensityClass::Standard,
            motion_preference: ActivityMotionPreference::Standard,
            style: ActivityIndicatorStyle::default(),
            style_id: ActivityIndicatorStyleId::DEFAULT,
        })
    }

    pub fn density(mut self, density: DensityClass) -> Self {
        self.density = density;
        self
    }

    pub fn motion_preference(mut self, preference: ActivityMotionPreference) -> Self {
        self.motion_preference = preference;
        self
    }

    pub fn style(mut self, style: ActivityIndicatorStyle) -> Self {
        self.style = style;
        self
    }

    pub fn style_id(mut self, style_id: ActivityIndicatorStyleId) -> Self {
        self.style_id = style_id;
        self
    }

    pub const fn active(&self) -> Read<bool> {
        self.active
    }

    pub fn semantic_node(&self, name: crate::ui::StringId, active: bool) -> SemanticNode {
        SemanticNode {
            role: SemanticRole::ProgressIndicator,
            name: SemanticName::Text(name),
            state: SemanticState {
                busy: active,
                ..SemanticState::default()
            },
            value: SemanticValue::None,
            actions: SemanticActions::NONE,
            ..SemanticNode::default()
        }
    }

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
    ) -> RuntimeResult<ActivityIndicatorRef> {
        let active = ui.read(self.active)?;
        let state = ActivityIndicatorState::from(active);
        let resolved = self
            .style
            .resolve(self.density, state, self.motion_preference);
        let visual =
            resolved_activity_visual(self.style, self.density, self.motion_preference, active);
        let row = LayoutStyle {
            flow: Flow::Horizontal,
            gap: visual.gap,
            ..LayoutStyle::default()
        };
        let mut indicator_control = None;
        let mut marker_control = None;
        let mut label_control = None;
        let control = ui
            .foundation()
            .container_node_under(host, visual.container, row, |writer| {
                label_control =
                    Some(writer.text(&self.label, visual.label_color, visual.label_size));
                indicator_control = Some(writer.container_handle(
                    visual.track,
                    LayoutStyle {
                        flow: Flow::Overlay,
                        ..LayoutStyle::default()
                    },
                    |writer| {
                        marker_control = Some(writer.container_handle(
                            visual.marker,
                            LayoutStyle::default(),
                            |_| {},
                        ));
                    },
                ));
            })
            .ok_or_else(|| RuntimeError::new("application activity indicator host is stale"))?;
        let indicator_control =
            indicator_control.expect("activity indicator track is always mounted");
        let marker_control = marker_control.expect("activity indicator marker is always mounted");
        let label_control = label_control.expect("activity indicator label is always mounted");

        ui.foundation().style_binding(
            StyleBinding::new(control.node, ThemeScopeId::new(0, 1), self.style_id.0)
                .slot(StyleSlotId::named("root"), control.node)
                .slot(StyleSlotId::named("track"), indicator_control.node)
                .slot(StyleSlotId::named("marker"), marker_control.node)
                .slot(StyleSlotId::named("label"), label_control.node),
        );

        let name = ui.foundation().intern(&self.label);
        ui.foundation()
            .semantic_node(control.node, self.semantic_node(name, active))
            .map_err(|error| {
                RuntimeError::new(format!("invalid activity indicator semantics: {error:?}"))
            })?;
        ui.foundation().busy(control.node, active);
        let read = self.active;
        ui.bind_map(read, control.busy, |active| *active)?;
        let style = self.style;
        let density = self.density;
        let preference = self.motion_preference;
        ui.bind_map(read, control.style, move |active| {
            resolved_activity_visual(style, density, preference, *active).container
        })?;
        let style = self.style;
        ui.bind_map(read, indicator_control.style, move |active| {
            resolved_activity_visual(style, density, preference, *active).track
        })?;
        let style = self.style;
        ui.bind_map(read, marker_control.style, move |active| {
            resolved_activity_visual(style, density, preference, *active).marker
        })?;
        let style = self.style;
        ui.bind_map(read, label_control.color, move |active| {
            resolved_activity_visual(style, density, preference, *active).label_color
        })?;

        Ok(ActivityIndicatorRef {
            control,
            indicator: indicator_control.node,
            marker: marker_control.node,
            active: self.active,
            resolved,
        })
    }
}

fn resolved_activity_visual(
    style: ActivityIndicatorStyle,
    density: DensityClass,
    preference: ActivityMotionPreference,
    active: bool,
) -> ActivityIndicatorVisualStyle {
    let mut visual = style
        .resolve(density, ActivityIndicatorState::from(active), preference)
        .visual;
    configure_activity_geometry(&mut visual);
    visual
}

fn configure_activity_geometry(visual: &mut ActivityIndicatorVisualStyle) {
    visual.track.width = SizeRule::Logical(visual.indicator_size);
    visual.track.height = SizeRule::Logical(visual.indicator_size);
    visual.marker.width = SizeRule::Logical(visual.marker_size);
    visual.marker.height = SizeRule::Logical(visual.marker_size);
    let centered = (visual.indicator_size - visual.marker_size) * 0.5;
    let y = match visual.motion {
        ActivityMotionStyle::Rotate { .. } => 0.0,
        ActivityMotionStyle::Static => centered,
    };
    visual.marker.transform = Transform2D {
        translation: PointF { x: centered, y },
        ..Transform2D::default()
    };
}

/// Focused reference exposing declarative motion intent without owning a scheduler.
#[derive(Clone, Copy, Debug)]
pub struct ActivityIndicatorRef {
    control: ControlHandle,
    indicator: UiNodeId,
    marker: UiNodeId,
    active: Read<bool>,
    resolved: ResolvedActivityIndicatorStyle,
}

impl ActivityIndicatorRef {
    pub const fn node(self) -> UiNodeId {
        self.control.node
    }

    pub const fn indicator_node(self) -> UiNodeId {
        self.indicator
    }

    pub const fn marker_node(self) -> UiNodeId {
        self.marker
    }

    pub const fn active(self) -> Read<bool> {
        self.active
    }

    pub const fn resolved_style(self) -> ResolvedActivityIndicatorStyle {
        self.resolved
    }

    pub const fn motion(self) -> ActivityMotionStyle {
        self.resolved.visual.motion
    }

    pub const fn style(self) -> Property<BoxStyle> {
        self.control.style
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivityIndicatorError {
    MissingAccessibleName,
}

impl std::fmt::Display for ActivityIndicatorError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingAccessibleName => {
                formatter.write_str("activity indicator accessible name is empty")
            }
        }
    }
}

impl std::error::Error for ActivityIndicatorError {}

#[cfg(test)]
mod tests;

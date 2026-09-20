//! Read-only Tier A meter bands, semantics, styles, and mounting.

use crate::foundation::{ColorRgba8, EdgeInsets};
use crate::runtime::{Read, RuntimeError, RuntimeResult, Ui};
use crate::ui::{
    Background, BoxStyle, ControlHandle, CornerRadii, Flow, LayoutStyle, Property, SemanticActions,
    SemanticName, SemanticNode, SemanticRole, SemanticValue, SizeRule, UiNodeId,
};

use crate::components::application::{DensityClass, RangeModel, RangeModelError, RangeScalar};

/// Typed meaning assigned to a validated meter interval.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MeterLevel {
    #[default]
    Neutral,
    Positive,
    Caution,
    Critical,
}

/// Inclusive upper boundary and meaning for one meter interval.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeterBand<T> {
    upper_bound: T,
    level: MeterLevel,
}

impl<T> MeterBand<T> {
    pub const fn new(upper_bound: T, level: MeterLevel) -> Self {
        Self { upper_bound, level }
    }

    pub const fn upper_bound(&self) -> &T {
        &self.upper_bound
    }

    pub const fn level(&self) -> MeterLevel {
        self.level
    }
}

/// Nonempty, ordered bands that cover one complete range model.
#[derive(Clone, Debug, PartialEq)]
pub struct MeterBands<T> {
    minimum: T,
    maximum: T,
    bands: Vec<MeterBand<T>>,
}

impl<T> MeterBands<T>
where
    T: RangeScalar,
{
    pub fn new(
        model: &RangeModel<T>,
        bands: impl IntoIterator<Item = MeterBand<T>>,
    ) -> Result<Self, MeterError> {
        let bands: Vec<_> = bands.into_iter().collect();
        validate_bands(model, &bands)?;
        Ok(Self {
            minimum: model.minimum(),
            maximum: model.maximum(),
            bands,
        })
    }

    pub fn as_slice(&self) -> &[MeterBand<T>] {
        &self.bands
    }

    pub fn level_for(&self, value: T) -> Result<MeterLevel, MeterError> {
        let value = value.to_f64();
        if !value.is_finite() {
            return Err(MeterError::Model(RangeModelError::NonFinite(
                crate::components::application::RangeNumber::Value,
            )));
        }
        if value < self.minimum.to_f64() || value > self.maximum.to_f64() {
            return Err(MeterError::Model(RangeModelError::ValueOutOfBounds));
        }
        self.bands
            .iter()
            .find(|band| value <= band.upper_bound.to_f64())
            .map(MeterBand::level)
            .ok_or(MeterError::BandsDoNotReachMaximum)
    }

    fn matches(&self, model: &RangeModel<T>) -> bool {
        self.minimum == model.minimum() && self.maximum == model.maximum()
    }
}

fn validate_bands<T: RangeScalar>(
    model: &RangeModel<T>,
    bands: &[MeterBand<T>],
) -> Result<(), MeterError> {
    if bands.is_empty() {
        return Err(MeterError::EmptyBands);
    }
    let minimum = model.minimum().to_f64();
    let maximum = model.maximum().to_f64();
    let mut previous = minimum;
    for (index, band) in bands.iter().enumerate() {
        let upper = band.upper_bound.to_f64();
        if !upper.is_finite() {
            return Err(MeterError::NonFiniteBand { index });
        }
        if upper <= minimum || upper > maximum {
            return Err(MeterError::BandOutOfBounds { index });
        }
        if upper <= previous {
            return Err(MeterError::BandsNotStrictlyIncreasing { index });
        }
        previous = upper;
    }
    if previous != maximum {
        return Err(MeterError::BandsDoNotReachMaximum);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeterLevelColors {
    pub neutral: ColorRgba8,
    pub positive: ColorRgba8,
    pub caution: ColorRgba8,
    pub critical: ColorRgba8,
}

impl MeterLevelColors {
    pub const fn for_level(self, level: MeterLevel) -> ColorRgba8 {
        match level {
            MeterLevel::Neutral => self.neutral,
            MeterLevel::Positive => self.positive,
            MeterLevel::Caution => self.caution,
            MeterLevel::Critical => self.critical,
        }
    }
}

/// Visual slots for one meter density.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeterVisualStyle {
    pub container: BoxStyle,
    pub track: BoxStyle,
    pub fill: BoxStyle,
    pub colors: MeterLevelColors,
    pub label_color: ColorRgba8,
    pub label_size: f32,
    pub gap: f32,
    pub track_length: f32,
    pub track_thickness: f32,
}

/// Explicit Compact/Standard/Touch meter variants.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeterStyle {
    pub compact: MeterVisualStyle,
    pub standard: MeterVisualStyle,
    pub touch: MeterVisualStyle,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedMeterStyle {
    pub density: DensityClass,
    pub level: MeterLevel,
    pub visual: MeterVisualStyle,
}

impl MeterStyle {
    pub fn resolve(self, density: DensityClass, level: MeterLevel) -> ResolvedMeterStyle {
        let mut visual = match density {
            DensityClass::Compact => self.compact,
            DensityClass::Standard => self.standard,
            DensityClass::Touch => self.touch,
        };
        visual.fill.decoration.background = Background::Color(visual.colors.for_level(level));
        ResolvedMeterStyle {
            density,
            level,
            visual,
        }
    }
}

impl Default for MeterStyle {
    fn default() -> Self {
        fn visual(label_size: f32, gap: f32, track_thickness: f32) -> MeterVisualStyle {
            MeterVisualStyle {
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
                        corner_radii: CornerRadii::all(track_thickness * 0.5),
                        ..crate::ui::BoxDecoration::default()
                    },
                    ..BoxStyle::default()
                },
                colors: MeterLevelColors {
                    neutral: ColorRgba8::rgba(93, 132, 218, 255),
                    positive: ColorRgba8::rgba(55, 153, 104, 255),
                    caution: ColorRgba8::rgba(217, 153, 47, 255),
                    critical: ColorRgba8::rgba(211, 72, 76, 255),
                },
                label_color: ColorRgba8::rgba(235, 238, 244, 255),
                label_size,
                gap,
                track_length: 160.0,
                track_thickness,
            }
        }

        Self {
            compact: visual(12.0, 4.0, 3.0),
            standard: visual(14.0, 6.0, 4.0),
            touch: visual(16.0, 8.0, 6.0),
        }
    }
}

/// Immutable configuration for a named, parent-controlled meter.
#[derive(Clone, Debug, PartialEq)]
pub struct Meter<T: 'static> {
    label: String,
    value: Read<T>,
    model: RangeModel<T>,
    bands: MeterBands<T>,
    density: DensityClass,
    style: MeterStyle,
}

impl<T> Meter<T>
where
    T: RangeScalar,
{
    pub fn new(
        label: impl Into<String>,
        value: Read<T>,
        model: RangeModel<T>,
        bands: MeterBands<T>,
    ) -> Result<Self, MeterError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(MeterError::MissingAccessibleName);
        }
        if !bands.matches(&model) {
            return Err(MeterError::BandModelMismatch);
        }
        Ok(Self {
            label,
            value,
            model,
            bands,
            density: DensityClass::Standard,
            style: MeterStyle::default(),
        })
    }

    pub fn density(mut self, density: DensityClass) -> Self {
        self.density = density;
        self
    }

    pub fn style(mut self, style: MeterStyle) -> Self {
        self.style = style;
        self
    }

    pub const fn value(&self) -> Read<T> {
        self.value
    }

    pub const fn model(&self) -> &RangeModel<T> {
        &self.model
    }

    pub const fn bands(&self) -> &MeterBands<T> {
        &self.bands
    }

    pub fn semantic_node(
        &self,
        name: crate::ui::StringId,
        value_text: crate::ui::StringId,
        value: T,
    ) -> Result<SemanticNode, MeterError> {
        self.model.format_value(value)?;
        Ok(SemanticNode {
            role: SemanticRole::Meter,
            name: SemanticName::Text(name),
            value: SemanticValue::Number {
                current: value.to_f64(),
                minimum: self.model.minimum().to_f64(),
                maximum: self.model.maximum().to_f64(),
                step: Some(self.model.step().to_f64()),
                value_text: Some(value_text),
            },
            actions: SemanticActions::NONE,
            ..SemanticNode::default()
        })
    }

    pub fn mount<Action: 'static>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        host: UiNodeId,
    ) -> RuntimeResult<MeterRef<T>> {
        let value = ui.read(self.value)?;
        let value_text = self.model.format_value(value).map_err(|error| {
            RuntimeError::new(format!("invalid controlled meter value: {error}"))
        })?;
        let level = self
            .bands
            .level_for(value)
            .map_err(|error| RuntimeError::new(format!("invalid meter band value: {error}")))?;
        let resolved = self.style.resolve(self.density, level);
        let mut visual = resolved.visual;
        configure_meter_geometry(&mut visual, value, &self.model);
        let row = LayoutStyle {
            flow: Flow::Horizontal,
            gap: visual.gap,
            ..LayoutStyle::default()
        };
        let mut fill_node = None;
        let control = ui
            .foundation()
            .container_node_under(host, visual.container, row, |writer| {
                writer.text(&self.label, visual.label_color, visual.label_size);
                writer.container(
                    visual.track,
                    LayoutStyle {
                        flow: Flow::Overlay,
                        ..LayoutStyle::default()
                    },
                    |writer| {
                        fill_node =
                            Some(writer.container(visual.fill, LayoutStyle::default(), |_| {}));
                    },
                );
            })
            .ok_or_else(|| RuntimeError::new("application meter host is stale"))?;

        let name = ui.foundation().intern(&self.label);
        let value_text = ui.foundation().intern(value_text);
        let semantic = self
            .semantic_node(name, value_text, value)
            .map_err(|error| RuntimeError::new(format!("invalid meter semantics: {error}")))?;
        ui.foundation()
            .semantic_node(control.node, semantic)
            .map_err(|error| RuntimeError::new(format!("invalid meter semantics: {error:?}")))?;

        Ok(MeterRef {
            control,
            fill: fill_node.expect("meter fill is always mounted"),
            value: self.value,
            level,
        })
    }
}

fn configure_meter_geometry<T: RangeScalar>(
    visual: &mut MeterVisualStyle,
    value: T,
    model: &RangeModel<T>,
) {
    visual.track.width = SizeRule::Logical(visual.track_length);
    visual.track.height = SizeRule::Logical(visual.track_thickness);
    visual.fill.height = SizeRule::Logical(visual.track_thickness);
    let fraction = ((value.to_f64() - model.minimum().to_f64())
        / (model.maximum().to_f64() - model.minimum().to_f64()))
    .clamp(0.0, 1.0) as f32;
    visual.fill.width = SizeRule::Logical(visual.track_length * fraction);
}

/// Focused advanced reference returned by meter mounting.
#[derive(Clone, Copy, Debug)]
pub struct MeterRef<T: 'static> {
    control: ControlHandle,
    fill: UiNodeId,
    value: Read<T>,
    level: MeterLevel,
}

impl<T: 'static> MeterRef<T> {
    pub const fn node(self) -> UiNodeId {
        self.control.node
    }

    pub const fn fill_node(self) -> UiNodeId {
        self.fill
    }

    pub const fn value(self) -> Read<T> {
        self.value
    }

    pub const fn level(self) -> MeterLevel {
        self.level
    }

    pub const fn style(self) -> Property<BoxStyle> {
        self.control.style
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeterError {
    MissingAccessibleName,
    EmptyBands,
    NonFiniteBand { index: usize },
    BandOutOfBounds { index: usize },
    BandsNotStrictlyIncreasing { index: usize },
    BandsDoNotReachMaximum,
    BandModelMismatch,
    Model(RangeModelError),
}

impl From<RangeModelError> for MeterError {
    fn from(error: RangeModelError) -> Self {
        Self::Model(error)
    }
}

impl std::fmt::Display for MeterError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingAccessibleName => formatter.write_str("meter accessible name is empty"),
            Self::EmptyBands => formatter.write_str("meter bands are empty"),
            Self::NonFiniteBand { index } => {
                write!(formatter, "meter band {index} upper bound is not finite")
            }
            Self::BandOutOfBounds { index } => {
                write!(
                    formatter,
                    "meter band {index} upper bound is outside the range"
                )
            }
            Self::BandsNotStrictlyIncreasing { index } => write!(
                formatter,
                "meter band {index} is not strictly greater than its predecessor"
            ),
            Self::BandsDoNotReachMaximum => {
                formatter.write_str("meter bands do not cover the range maximum")
            }
            Self::BandModelMismatch => {
                formatter.write_str("meter bands were validated for different range bounds")
            }
            Self::Model(error) => write!(formatter, "invalid meter range value: {error}"),
        }
    }
}

impl std::error::Error for MeterError {}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use crate::runtime::{Component, CreateContext, State, UpdateContext, ViewRuntime};
    use crate::ui::{LayoutStyle, UiRoot};

    use crate::components::application::RangeFormat;

    use super::*;

    fn model() -> RangeModel<f64> {
        RangeModel::new(0.0, 100.0, 1.0, 10.0)
            .unwrap()
            .with_format(RangeFormat::new(0).unwrap().suffix("%").unwrap())
    }

    fn bands() -> MeterBands<f64> {
        MeterBands::new(
            &model(),
            [
                MeterBand::new(60.0, MeterLevel::Positive),
                MeterBand::new(85.0, MeterLevel::Caution),
                MeterBand::new(100.0, MeterLevel::Critical),
            ],
        )
        .unwrap()
    }

    #[test]
    fn bands_are_nonempty_ordered_bounded_covering_and_select_inclusive_boundaries() {
        assert_eq!(MeterBands::new(&model(), []), Err(MeterError::EmptyBands));
        assert_eq!(
            MeterBands::new(&model(), [MeterBand::new(f64::NAN, MeterLevel::Neutral)]),
            Err(MeterError::NonFiniteBand { index: 0 })
        );
        assert_eq!(
            MeterBands::new(&model(), [MeterBand::new(101.0, MeterLevel::Neutral)]),
            Err(MeterError::BandOutOfBounds { index: 0 })
        );
        assert_eq!(
            MeterBands::new(
                &model(),
                [
                    MeterBand::new(60.0, MeterLevel::Positive),
                    MeterBand::new(60.0, MeterLevel::Caution),
                    MeterBand::new(100.0, MeterLevel::Critical),
                ]
            ),
            Err(MeterError::BandsNotStrictlyIncreasing { index: 1 })
        );
        assert_eq!(
            MeterBands::new(&model(), [MeterBand::new(90.0, MeterLevel::Positive)]),
            Err(MeterError::BandsDoNotReachMaximum)
        );

        let bands = bands();
        assert_eq!(bands.level_for(0.0), Ok(MeterLevel::Positive));
        assert_eq!(bands.level_for(60.0), Ok(MeterLevel::Positive));
        assert_eq!(bands.level_for(60.1), Ok(MeterLevel::Caution));
        assert_eq!(bands.level_for(85.0), Ok(MeterLevel::Caution));
        assert_eq!(bands.level_for(100.0), Ok(MeterLevel::Critical));
        assert_eq!(
            bands.level_for(101.0),
            Err(MeterError::Model(RangeModelError::ValueOutOfBounds))
        );
    }

    #[test]
    fn style_resolution_selects_density_and_typed_level_color() {
        let style = MeterStyle::default();
        let compact = style.resolve(DensityClass::Compact, MeterLevel::Positive);
        let touch = style.resolve(DensityClass::Touch, MeterLevel::Critical);
        assert_eq!(compact.visual.track_thickness, 3.0);
        assert_eq!(touch.visual.track_thickness, 6.0);
        assert_eq!(compact.visual.label_size, 12.0);
        assert_eq!(touch.visual.label_size, 16.0);
        assert_eq!(
            touch.visual.fill.decoration.background,
            Background::Color(touch.visual.colors.critical)
        );
    }

    struct MountedMeter {
        initial: f64,
        node: Rc<Cell<Option<UiNodeId>>>,
        fill: Rc<Cell<Option<UiNodeId>>>,
        level: Rc<Cell<Option<MeterLevel>>>,
        error: Rc<RefCell<Option<String>>>,
    }

    impl Component for MountedMeter {
        type State = State<f64>;
        type Action = ();

        fn create(&self, context: &mut CreateContext<'_>) -> Self::State {
            context.state(self.initial)
        }

        fn mount(&self, state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            assert!(matches!(
                Meter::new(" ", state.read(), model(), bands()),
                Err(MeterError::MissingAccessibleName)
            ));
            assert!(matches!(
                Meter::new(
                    "Load",
                    state.read(),
                    RangeModel::new(0.0, 200.0, 1.0, 10.0).unwrap(),
                    bands(),
                ),
                Err(MeterError::BandModelMismatch)
            ));
            match Meter::new("Battery", state.read(), model(), bands())
                .unwrap()
                .density(DensityClass::Touch)
                .mount(ui, root.0)
            {
                Ok(reference) => {
                    self.node.set(Some(reference.node()));
                    self.fill.set(Some(reference.fill_node()));
                    self.level.set(Some(reference.level()));
                }
                Err(error) => *self.error.borrow_mut() = Some(error.to_string()),
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

    type MountedMeterResult = (
        ViewRuntime<crate::runtime::ComponentRuntimeDriver<MountedMeter>>,
        Rc<Cell<Option<UiNodeId>>>,
        Rc<Cell<Option<UiNodeId>>>,
        Rc<Cell<Option<MeterLevel>>>,
        Rc<RefCell<Option<String>>>,
    );

    fn mounted(initial: f64) -> MountedMeterResult {
        let node = Rc::new(Cell::new(None));
        let fill = Rc::new(Cell::new(None));
        let level = Rc::new(Cell::new(None));
        let error = Rc::new(RefCell::new(None));
        let runtime = ViewRuntime::from_component(MountedMeter {
            initial,
            node: node.clone(),
            fill: fill.clone(),
            level: level.clone(),
            error: error.clone(),
        })
        .unwrap();
        (runtime, node, fill, level, error)
    }

    #[test]
    fn mount_reports_formatted_meter_semantics_and_typed_band_visual() {
        let (runtime, node, fill, level, error) = mounted(75.0);
        assert!(error.borrow().is_none());
        let node = node.get().unwrap();
        let semantic = runtime.ui().semantics.get(node).unwrap();
        assert_eq!(semantic.role, SemanticRole::Meter);
        assert!(semantic.actions.is_empty());
        assert!(!semantic.state.focusable);
        let SemanticValue::Number {
            current,
            minimum,
            maximum,
            step,
            value_text,
        } = semantic.value
        else {
            panic!("meter must expose a numeric value");
        };
        assert_eq!(
            (current, minimum, maximum, step),
            (75.0, 0.0, 100.0, Some(1.0))
        );
        assert_eq!(runtime.ui().string(value_text.unwrap()), Some("75%"));
        assert_eq!(level.get(), Some(MeterLevel::Caution));
        let fill = runtime.ui().box_styles.get(fill.get().unwrap()).unwrap();
        assert_eq!(fill.width, SizeRule::Logical(120.0));
        assert_eq!(fill.height, SizeRule::Logical(6.0));
        assert_eq!(
            fill.decoration.background,
            Background::Color(MeterStyle::default().touch.colors.caution)
        );
    }

    #[test]
    fn out_of_range_controlled_value_is_rejected_before_semantic_attachment() {
        let (runtime, node, _fill, _level, error) = mounted(120.0);
        assert!(node.get().is_none());
        assert!(
            error
                .borrow()
                .as_deref()
                .unwrap()
                .contains("outside the bounds")
        );
        assert_eq!(runtime.ui().semantics.len(), 0);
    }
}

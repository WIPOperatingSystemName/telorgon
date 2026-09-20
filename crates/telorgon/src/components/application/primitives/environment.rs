//! Validated, platform-neutral environment records for one application view.

use std::fmt;
use std::num::NonZeroU64;
use std::sync::Arc;

use crate::foundation::{EdgeInsets, RectF, SizeF};
use crate::input::{FocusIndicatorPolicy, PointerDeviceKind, WritingDirection};

/// Monotonic identity of an accepted environment value set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EnvironmentRevision(NonZeroU64);

impl EnvironmentRevision {
    pub const INITIAL: Self = Self(NonZeroU64::MIN);

    pub const fn get(self) -> u64 {
        self.0.get()
    }

    fn next(self) -> Option<Self> {
        self.get()
            .checked_add(1)
            .and_then(NonZeroU64::new)
            .map(Self)
    }
}

/// Constraint for one logical axis. `None` means that the maximum is unbounded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxisConstraints {
    pub min: f32,
    pub max: Option<f32>,
}

impl Default for AxisConstraints {
    fn default() -> Self {
        Self {
            min: 0.0,
            max: None,
        }
    }
}

/// Local logical constraints supplied to an application component.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LogicalConstraints {
    pub horizontal: AxisConstraints,
    pub vertical: AxisConstraints,
}

/// Coarse logical density selected for the current view.
///
/// Component target metrics remain owned by the application component package.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LogicalDensityClass {
    Compact,
    #[default]
    Standard,
    Touch,
}

/// A nonempty, segmented ASCII locale tag retained without platform types.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocaleTag(Box<str>);

impl LocaleTag {
    pub const MAX_BYTES: usize = 63;

    pub fn parse(value: impl AsRef<str>) -> Result<Self, EnvironmentError> {
        let value = value.as_ref();
        let valid_length = !value.is_empty() && value.len() <= Self::MAX_BYTES;
        let valid_segments = value.split('-').all(|segment| {
            !segment.is_empty()
                && segment.len() <= 8
                && segment.bytes().all(|byte| byte.is_ascii_alphanumeric())
        });
        if !valid_length || !valid_segments {
            return Err(EnvironmentError::InvalidLocaleTag);
        }
        Ok(Self(value.into()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for LocaleTag {
    fn default() -> Self {
        Self("und".into())
    }
}

impl fmt::Display for LocaleTag {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl TryFrom<&str> for LocaleTag {
    type Error = EnvironmentError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

/// Preferred traversal of two-dimensional reading content.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PreferredReadingOrder {
    #[default]
    RowsFirst,
    ColumnsFirst,
}

/// Capabilities that may all be present at the same time.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct InputCapabilities(u16);

impl InputCapabilities {
    pub const NONE: Self = Self(0);
    pub const MOUSE: Self = Self(1 << 0);
    pub const TOUCH: Self = Self(1 << 1);
    pub const PEN: Self = Self(1 << 2);
    pub const ERASER: Self = Self(1 << 3);
    pub const HOVER: Self = Self(1 << 4);
    pub const KEYBOARD: Self = Self(1 << 5);
    pub const DIRECTIONAL_CONTROLLER: Self = Self(1 << 6);
    pub const TEXT_INPUT: Self = Self(1 << 7);
    const ALL_BITS: u16 = (1 << 8) - 1;

    pub const fn from_bits(bits: u16) -> Option<Self> {
        if bits & !Self::ALL_BITS == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn with_pointer(self, kind: PointerDeviceKind) -> Self {
        let pointer = match kind {
            PointerDeviceKind::Mouse => Self::MOUSE,
            PointerDeviceKind::Touch => Self::TOUCH,
            PointerDeviceKind::Pen => Self::PEN,
            PointerDeviceKind::Eraser => Self::ERASER,
            PointerDeviceKind::Unknown => Self::NONE,
        };
        self.union(pointer)
    }
}

impl fmt::Debug for InputCapabilities {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InputCapabilities")
            .field("bits", &format_args!("{:#010b}", self.bits()))
            .finish()
    }
}

impl std::ops::BitOr for InputCapabilities {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        self.union(rhs)
    }
}

impl std::ops::BitOrAssign for InputCapabilities {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = self.union(rhs);
    }
}

/// Resolved color-scheme preference, without a platform or theme type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ColorSchemePreference {
    #[default]
    NoPreference,
    Light,
    Dark,
}

/// User accessibility and presentation preferences relevant to components.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct EnvironmentPreferences {
    pub reduced_motion: bool,
    pub increased_contrast: bool,
    pub color_scheme: ColorSchemePreference,
    pub focus_indicators: FocusIndicatorPolicy,
}

/// Current lifecycle facts for the view that owns the environment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EnvironmentViewState {
    pub active: bool,
    pub focused: bool,
    pub visible: bool,
}

impl Default for EnvironmentViewState {
    fn default() -> Self {
        Self {
            active: true,
            focused: true,
            visible: true,
        }
    }
}

/// Complete platform-neutral environment value set for one application view.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvironmentValues {
    pub available_size: SizeF,
    pub constraints: LogicalConstraints,
    pub device_scale: f32,
    pub density: LogicalDensityClass,
    pub text_scale: f32,
    pub locale: LocaleTag,
    pub writing_direction: WritingDirection,
    pub reading_order: PreferredReadingOrder,
    pub safe_area: EdgeInsets,
    pub occlusions: Vec<RectF>,
    pub input_capabilities: InputCapabilities,
    pub preferences: EnvironmentPreferences,
    pub view: EnvironmentViewState,
}

impl Default for EnvironmentValues {
    fn default() -> Self {
        Self {
            available_size: SizeF::default(),
            constraints: LogicalConstraints::default(),
            device_scale: 1.0,
            density: LogicalDensityClass::default(),
            text_scale: 1.0,
            locale: LocaleTag::default(),
            writing_direction: WritingDirection::default(),
            reading_order: PreferredReadingOrder::default(),
            safe_area: EdgeInsets::ZERO,
            occlusions: Vec::new(),
            input_capabilities: InputCapabilities::NONE,
            preferences: EnvironmentPreferences::default(),
            view: EnvironmentViewState::default(),
        }
    }
}

/// Groups of dependency-tracked reads invalidated by an accepted update.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct EnvironmentChangeSet(u8);

impl EnvironmentChangeSet {
    pub const NONE: Self = Self(0);
    pub const GEOMETRY: Self = Self(1 << 0);
    pub const SCALE_AND_DENSITY: Self = Self(1 << 1);
    pub const LANGUAGE_AND_DIRECTION: Self = Self(1 << 2);
    pub const INPUT_CAPABILITIES: Self = Self(1 << 3);
    pub const PREFERENCES: Self = Self(1 << 4);
    pub const VIEW_STATE: Self = Self(1 << 5);

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl fmt::Debug for EnvironmentChangeSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EnvironmentChangeSet")
            .field("bits", &format_args!("{:#08b}", self.bits()))
            .finish()
    }
}

/// Immutable view of an accepted environment revision.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvironmentSnapshot {
    revision: EnvironmentRevision,
    values: Arc<EnvironmentValues>,
}

impl EnvironmentSnapshot {
    pub const fn revision(&self) -> EnvironmentRevision {
        self.revision
    }

    pub fn values(&self) -> &EnvironmentValues {
        &self.values
    }
}

/// Result of submitting a valid environment value set.
#[derive(Clone, Debug)]
pub struct EnvironmentUpdate {
    pub snapshot: EnvironmentSnapshot,
    pub changed: EnvironmentChangeSet,
}

impl EnvironmentUpdate {
    pub const fn changed(&self) -> bool {
        !self.changed.is_empty()
    }
}

/// Deterministic work and rejection counters for this owner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EnvironmentDiagnostics {
    pub accepted_updates: u64,
    pub unchanged_updates: u64,
    pub rejected_updates: u64,
}

/// Validation or revision failure that leaves the accepted environment untouched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvironmentError {
    InvalidAvailableSize,
    InvalidHorizontalConstraints,
    InvalidVerticalConstraints,
    InvalidDeviceScale,
    InvalidTextScale,
    InvalidSafeArea,
    InvalidOcclusion { index: usize },
    DuplicateOcclusion { first: usize, duplicate: usize },
    InvalidLocaleTag,
    RevisionExhausted,
}

impl fmt::Display for EnvironmentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid application environment: {self:?}")
    }
}

impl std::error::Error for EnvironmentError {}

/// Atomic owner of the current environment and its immutable snapshots.
#[derive(Debug)]
pub struct EnvironmentState {
    revision: EnvironmentRevision,
    values: Arc<EnvironmentValues>,
    diagnostics: EnvironmentDiagnostics,
}

impl EnvironmentState {
    pub fn new(values: EnvironmentValues) -> Result<Self, EnvironmentError> {
        validate_values(&values)?;
        Ok(Self {
            revision: EnvironmentRevision::INITIAL,
            values: Arc::new(values),
            diagnostics: EnvironmentDiagnostics::default(),
        })
    }

    pub fn snapshot(&self) -> EnvironmentSnapshot {
        EnvironmentSnapshot {
            revision: self.revision,
            values: Arc::clone(&self.values),
        }
    }

    pub const fn revision(&self) -> EnvironmentRevision {
        self.revision
    }

    pub fn values(&self) -> &EnvironmentValues {
        &self.values
    }

    pub const fn diagnostics(&self) -> EnvironmentDiagnostics {
        self.diagnostics
    }

    pub fn update(
        &mut self,
        values: EnvironmentValues,
    ) -> Result<EnvironmentUpdate, EnvironmentError> {
        if let Err(error) = validate_values(&values) {
            self.diagnostics.rejected_updates += 1;
            return Err(error);
        }

        let changed = changes_between(&self.values, &values);
        if changed.is_empty() {
            self.diagnostics.unchanged_updates += 1;
            return Ok(EnvironmentUpdate {
                snapshot: self.snapshot(),
                changed,
            });
        }

        let Some(revision) = self.revision.next() else {
            self.diagnostics.rejected_updates += 1;
            return Err(EnvironmentError::RevisionExhausted);
        };
        self.revision = revision;
        self.values = Arc::new(values);
        self.diagnostics.accepted_updates += 1;
        Ok(EnvironmentUpdate {
            snapshot: self.snapshot(),
            changed,
        })
    }
}

fn validate_values(values: &EnvironmentValues) -> Result<(), EnvironmentError> {
    if !is_nonnegative_finite(values.available_size.width)
        || !is_nonnegative_finite(values.available_size.height)
    {
        return Err(EnvironmentError::InvalidAvailableSize);
    }
    validate_axis(values.constraints.horizontal)
        .map_err(|()| EnvironmentError::InvalidHorizontalConstraints)?;
    validate_axis(values.constraints.vertical)
        .map_err(|()| EnvironmentError::InvalidVerticalConstraints)?;
    if !is_positive_finite(values.device_scale) {
        return Err(EnvironmentError::InvalidDeviceScale);
    }
    if !is_positive_finite(values.text_scale) {
        return Err(EnvironmentError::InvalidTextScale);
    }
    if ![
        values.safe_area.top,
        values.safe_area.right,
        values.safe_area.bottom,
        values.safe_area.left,
    ]
    .into_iter()
    .all(is_nonnegative_finite)
        || !values.safe_area.horizontal().is_finite()
        || !values.safe_area.vertical().is_finite()
    {
        return Err(EnvironmentError::InvalidSafeArea);
    }

    for (index, occlusion) in values.occlusions.iter().copied().enumerate() {
        if ![occlusion.x, occlusion.y, occlusion.width, occlusion.height]
            .into_iter()
            .all(f32::is_finite)
            || occlusion.width < 0.0
            || occlusion.height < 0.0
            || !occlusion.right().is_finite()
            || !occlusion.bottom().is_finite()
        {
            return Err(EnvironmentError::InvalidOcclusion { index });
        }
        if let Some(first) = values.occlusions[..index]
            .iter()
            .position(|candidate| *candidate == occlusion)
        {
            return Err(EnvironmentError::DuplicateOcclusion {
                first,
                duplicate: index,
            });
        }
    }
    Ok(())
}

fn validate_axis(axis: AxisConstraints) -> Result<(), ()> {
    if !is_nonnegative_finite(axis.min)
        || axis
            .max
            .is_some_and(|max| !is_nonnegative_finite(max) || max < axis.min)
    {
        return Err(());
    }
    Ok(())
}

fn is_nonnegative_finite(value: f32) -> bool {
    value.is_finite() && value >= 0.0
}

fn is_positive_finite(value: f32) -> bool {
    value.is_finite() && value > 0.0
}

pub(crate) fn changes_between(
    before: &EnvironmentValues,
    after: &EnvironmentValues,
) -> EnvironmentChangeSet {
    let mut changes = EnvironmentChangeSet::NONE;
    if before.available_size != after.available_size
        || before.constraints != after.constraints
        || before.safe_area != after.safe_area
        || before.occlusions != after.occlusions
    {
        changes = changes.union(EnvironmentChangeSet::GEOMETRY);
    }
    if before.device_scale != after.device_scale
        || before.density != after.density
        || before.text_scale != after.text_scale
    {
        changes = changes.union(EnvironmentChangeSet::SCALE_AND_DENSITY);
    }
    if before.locale != after.locale
        || before.writing_direction != after.writing_direction
        || before.reading_order != after.reading_order
    {
        changes = changes.union(EnvironmentChangeSet::LANGUAGE_AND_DIRECTION);
    }
    if before.input_capabilities != after.input_capabilities {
        changes = changes.union(EnvironmentChangeSet::INPUT_CAPABILITIES);
    }
    if before.preferences != after.preferences {
        changes = changes.union(EnvironmentChangeSet::PREFERENCES);
    }
    if before.view != after.view {
        changes = changes.union(EnvironmentChangeSet::VIEW_STATE);
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values() -> EnvironmentValues {
        EnvironmentValues {
            available_size: SizeF {
                width: 1280.0,
                height: 720.0,
            },
            constraints: LogicalConstraints {
                horizontal: AxisConstraints {
                    min: 240.0,
                    max: Some(1280.0),
                },
                vertical: AxisConstraints {
                    min: 160.0,
                    max: None,
                },
            },
            locale: LocaleTag::parse("en-US").unwrap(),
            input_capabilities: InputCapabilities::MOUSE
                | InputCapabilities::TOUCH
                | InputCapabilities::HOVER
                | InputCapabilities::KEYBOARD
                | InputCapabilities::TEXT_INPUT,
            ..EnvironmentValues::default()
        }
    }

    #[test]
    fn snapshot_retains_an_accepted_revision() {
        let mut state = EnvironmentState::new(values()).unwrap();
        let old = state.snapshot();
        let mut next = values();
        next.available_size.width = 900.0;
        let update = state.update(next).unwrap();

        assert_eq!(old.revision(), EnvironmentRevision::INITIAL);
        assert_eq!(old.values().available_size.width, 1280.0);
        assert_eq!(update.snapshot.revision().get(), 2);
        assert_eq!(update.snapshot.values().available_size.width, 900.0);
        assert_eq!(state.diagnostics().accepted_updates, 1);
    }

    #[test]
    fn capabilities_are_a_simultaneous_set() {
        let capabilities = InputCapabilities::KEYBOARD
            | InputCapabilities::TOUCH
            | InputCapabilities::MOUSE.with_pointer(PointerDeviceKind::Pen);
        assert!(capabilities.contains(InputCapabilities::KEYBOARD | InputCapabilities::TOUCH));
        assert!(capabilities.intersects(InputCapabilities::MOUSE | InputCapabilities::PEN));
        assert_eq!(
            InputCapabilities::from_bits(capabilities.bits()),
            Some(capabilities)
        );
        assert_eq!(InputCapabilities::from_bits(1 << 15), None);
    }

    #[test]
    fn invalid_updates_are_atomic() {
        let mut state = EnvironmentState::new(values()).unwrap();
        let before = state.snapshot();
        let mut invalid = values();
        invalid.device_scale = f32::NAN;

        assert!(matches!(
            state.update(invalid),
            Err(EnvironmentError::InvalidDeviceScale)
        ));
        assert_eq!(state.revision(), before.revision());
        assert_eq!(state.values(), before.values());
        assert_eq!(state.diagnostics().rejected_updates, 1);
    }

    #[test]
    fn constraints_and_geometry_reject_nonfinite_or_inverted_values() {
        let mut invalid = values();
        invalid.available_size.height = -1.0;
        assert!(matches!(
            EnvironmentState::new(invalid),
            Err(EnvironmentError::InvalidAvailableSize)
        ));

        let mut invalid = values();
        invalid.constraints.horizontal.max = Some(100.0);
        assert!(matches!(
            EnvironmentState::new(invalid),
            Err(EnvironmentError::InvalidHorizontalConstraints)
        ));

        let mut invalid = values();
        invalid.safe_area.left = f32::INFINITY;
        assert!(matches!(
            EnvironmentState::new(invalid),
            Err(EnvironmentError::InvalidSafeArea)
        ));
    }

    #[test]
    fn locale_tags_are_nonempty_segmented_ascii_values() {
        assert_eq!(
            LocaleTag::parse("zh-Hant-TW").unwrap().as_str(),
            "zh-Hant-TW"
        );
        for invalid in ["", "-en", "en-", "en--US", "language9-tag", "fr_ÇA"] {
            assert_eq!(
                LocaleTag::parse(invalid),
                Err(EnvironmentError::InvalidLocaleTag)
            );
        }
    }

    #[test]
    fn occlusion_validation_is_ordered_and_atomic() {
        let rect = RectF {
            x: 300.0,
            y: 0.0,
            width: 12.0,
            height: 720.0,
        };
        let mut invalid = values();
        invalid.occlusions = vec![rect, rect];
        assert!(matches!(
            EnvironmentState::new(invalid),
            Err(EnvironmentError::DuplicateOcclusion {
                first: 0,
                duplicate: 1
            })
        ));

        let mut invalid = values();
        invalid.occlusions = vec![RectF {
            width: f32::INFINITY,
            ..RectF::ZERO
        }];
        assert!(matches!(
            EnvironmentState::new(invalid),
            Err(EnvironmentError::InvalidOcclusion { index: 0 })
        ));
    }

    #[test]
    fn unchanged_updates_do_no_revision_or_dirty_work() {
        let mut state = EnvironmentState::new(values()).unwrap();
        let update = state.update(values()).unwrap();
        assert!(!update.changed());
        assert_eq!(update.snapshot.revision(), EnvironmentRevision::INITIAL);
        assert_eq!(state.diagnostics().unchanged_updates, 1);
        assert_eq!(state.diagnostics().accepted_updates, 0);
    }

    #[test]
    fn updates_report_exact_change_groups() {
        let mut state = EnvironmentState::new(values()).unwrap();
        let mut next = values();
        next.text_scale = 1.5;
        next.writing_direction = WritingDirection::RightToLeft;
        next.preferences.reduced_motion = true;
        let update = state.update(next).unwrap();

        assert!(
            update
                .changed
                .contains(EnvironmentChangeSet::SCALE_AND_DENSITY)
        );
        assert!(
            update
                .changed
                .contains(EnvironmentChangeSet::LANGUAGE_AND_DIRECTION)
        );
        assert!(update.changed.contains(EnvironmentChangeSet::PREFERENCES));
        assert!(!update.changed.contains(EnvironmentChangeSet::GEOMETRY));
        assert!(!update.changed.contains(EnvironmentChangeSet::VIEW_STATE));
    }

    #[test]
    fn view_state_and_input_have_independent_dirty_groups() {
        let mut state = EnvironmentState::new(values()).unwrap();
        let mut next = values();
        next.view.focused = false;
        next.input_capabilities |= InputCapabilities::DIRECTIONAL_CONTROLLER;
        let update = state.update(next).unwrap();

        assert!(update.changed.contains(EnvironmentChangeSet::VIEW_STATE));
        assert!(
            update
                .changed
                .contains(EnvironmentChangeSet::INPUT_CAPABILITIES)
        );
        assert!(!update.changed.contains(EnvironmentChangeSet::PREFERENCES));
    }
}

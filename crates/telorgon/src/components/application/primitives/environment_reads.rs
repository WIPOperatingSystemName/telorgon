//! Dependency-tracked runtime reads over one validated application environment.

use std::fmt;

use crate::foundation::{EdgeInsets, RectF, SizeF};
use crate::input::WritingDirection;
use crate::runtime::{
    Component, CreateContext, Read, RuntimeError, RuntimeResult, State, UpdateContext,
};

use crate::components::application::primitives::environment::{
    EnvironmentChangeSet, EnvironmentPreferences, EnvironmentSnapshot, EnvironmentUpdate,
    EnvironmentViewState, InputCapabilities, LocaleTag, LogicalConstraints, LogicalDensityClass,
    PreferredReadingOrder, changes_between,
};

/// Geometry fields that invalidate layout and view-relative placement together.
///
/// The value retains the immutable shared environment snapshot rather than copying the occlusion
/// list. Equality deliberately considers only geometry fields, so an unrelated environment update
/// does not advance this derived read.
#[derive(Clone)]
pub struct EnvironmentGeometryAspect {
    snapshot: EnvironmentSnapshot,
}

impl EnvironmentGeometryAspect {
    fn from_snapshot(snapshot: &EnvironmentSnapshot) -> Self {
        Self {
            snapshot: snapshot.clone(),
        }
    }

    pub fn available_size(&self) -> SizeF {
        self.snapshot.values().available_size
    }

    pub fn constraints(&self) -> LogicalConstraints {
        self.snapshot.values().constraints
    }

    pub fn safe_area(&self) -> EdgeInsets {
        self.snapshot.values().safe_area
    }

    pub fn occlusions(&self) -> &[RectF] {
        &self.snapshot.values().occlusions
    }
}

impl PartialEq for EnvironmentGeometryAspect {
    fn eq(&self, other: &Self) -> bool {
        self.available_size() == other.available_size()
            && self.constraints() == other.constraints()
            && self.safe_area() == other.safe_area()
            && self.occlusions() == other.occlusions()
    }
}

impl fmt::Debug for EnvironmentGeometryAspect {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EnvironmentGeometryAspect")
            .field("available_size", &self.available_size())
            .field("constraints", &self.constraints())
            .field("safe_area", &self.safe_area())
            .field("occlusions", &self.occlusions())
            .finish()
    }
}

/// Device scale, logical density, and user text scale for one view.
#[derive(Clone)]
pub struct EnvironmentScaleAndDensityAspect {
    snapshot: EnvironmentSnapshot,
}

impl EnvironmentScaleAndDensityAspect {
    fn from_snapshot(snapshot: &EnvironmentSnapshot) -> Self {
        Self {
            snapshot: snapshot.clone(),
        }
    }

    pub fn device_scale(&self) -> f32 {
        self.snapshot.values().device_scale
    }

    pub fn density(&self) -> LogicalDensityClass {
        self.snapshot.values().density
    }

    pub fn text_scale(&self) -> f32 {
        self.snapshot.values().text_scale
    }
}

impl PartialEq for EnvironmentScaleAndDensityAspect {
    fn eq(&self, other: &Self) -> bool {
        self.device_scale() == other.device_scale()
            && self.density() == other.density()
            && self.text_scale() == other.text_scale()
    }
}

impl fmt::Debug for EnvironmentScaleAndDensityAspect {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EnvironmentScaleAndDensityAspect")
            .field("device_scale", &self.device_scale())
            .field("density", &self.density())
            .field("text_scale", &self.text_scale())
            .finish()
    }
}

/// Locale and directional fields that affect text and navigation interpretation together.
#[derive(Clone)]
pub struct EnvironmentLanguageAndDirectionAspect {
    snapshot: EnvironmentSnapshot,
}

impl EnvironmentLanguageAndDirectionAspect {
    fn from_snapshot(snapshot: &EnvironmentSnapshot) -> Self {
        Self {
            snapshot: snapshot.clone(),
        }
    }

    pub fn locale(&self) -> &LocaleTag {
        &self.snapshot.values().locale
    }

    pub fn writing_direction(&self) -> WritingDirection {
        self.snapshot.values().writing_direction
    }

    pub fn reading_order(&self) -> PreferredReadingOrder {
        self.snapshot.values().reading_order
    }
}

impl PartialEq for EnvironmentLanguageAndDirectionAspect {
    fn eq(&self, other: &Self) -> bool {
        self.locale() == other.locale()
            && self.writing_direction() == other.writing_direction()
            && self.reading_order() == other.reading_order()
    }
}

impl fmt::Debug for EnvironmentLanguageAndDirectionAspect {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EnvironmentLanguageAndDirectionAspect")
            .field("locale", &self.locale())
            .field("writing_direction", &self.writing_direction())
            .field("reading_order", &self.reading_order())
            .finish()
    }
}

/// Simultaneous input capabilities currently available to the view.
#[derive(Clone)]
pub struct EnvironmentInputAspect {
    snapshot: EnvironmentSnapshot,
}

impl EnvironmentInputAspect {
    fn from_snapshot(snapshot: &EnvironmentSnapshot) -> Self {
        Self {
            snapshot: snapshot.clone(),
        }
    }

    pub fn capabilities(&self) -> InputCapabilities {
        self.snapshot.values().input_capabilities
    }
}

impl PartialEq for EnvironmentInputAspect {
    fn eq(&self, other: &Self) -> bool {
        self.capabilities() == other.capabilities()
    }
}

impl fmt::Debug for EnvironmentInputAspect {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EnvironmentInputAspect")
            .field("capabilities", &self.capabilities())
            .finish()
    }
}

/// Accessibility and presentation preferences relevant to application components.
#[derive(Clone)]
pub struct EnvironmentPreferencesAspect {
    snapshot: EnvironmentSnapshot,
}

impl EnvironmentPreferencesAspect {
    fn from_snapshot(snapshot: &EnvironmentSnapshot) -> Self {
        Self {
            snapshot: snapshot.clone(),
        }
    }

    pub fn preferences(&self) -> EnvironmentPreferences {
        self.snapshot.values().preferences
    }
}

impl PartialEq for EnvironmentPreferencesAspect {
    fn eq(&self, other: &Self) -> bool {
        self.preferences() == other.preferences()
    }
}

impl fmt::Debug for EnvironmentPreferencesAspect {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EnvironmentPreferencesAspect")
            .field("preferences", &self.preferences())
            .finish()
    }
}

/// Independent active, focused, and visible lifecycle facts for one view.
#[derive(Clone)]
pub struct EnvironmentViewAspect {
    snapshot: EnvironmentSnapshot,
}

impl EnvironmentViewAspect {
    fn from_snapshot(snapshot: &EnvironmentSnapshot) -> Self {
        Self {
            snapshot: snapshot.clone(),
        }
    }

    pub fn view_state(&self) -> EnvironmentViewState {
        self.snapshot.values().view
    }
}

impl PartialEq for EnvironmentViewAspect {
    fn eq(&self, other: &Self) -> bool {
        self.view_state() == other.view_state()
    }
}

impl fmt::Debug for EnvironmentViewAspect {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EnvironmentViewAspect")
            .field("view_state", &self.view_state())
            .finish()
    }
}

/// Copyable dependency-tracked inputs for one mounted view environment.
///
/// Components may retain only the reads they consume. Each read advances only when its matching
/// aspect changes, even though all six aspects are committed from one atomic snapshot source.
#[derive(Clone, Copy, Debug)]
pub struct EnvironmentReads {
    geometry: Read<EnvironmentGeometryAspect>,
    scale_and_density: Read<EnvironmentScaleAndDensityAspect>,
    language_and_direction: Read<EnvironmentLanguageAndDirectionAspect>,
    input: Read<EnvironmentInputAspect>,
    preferences: Read<EnvironmentPreferencesAspect>,
    view: Read<EnvironmentViewAspect>,
}

impl EnvironmentReads {
    pub const fn geometry(self) -> Read<EnvironmentGeometryAspect> {
        self.geometry
    }

    pub const fn scale_and_density(self) -> Read<EnvironmentScaleAndDensityAspect> {
        self.scale_and_density
    }

    pub const fn language_and_direction(self) -> Read<EnvironmentLanguageAndDirectionAspect> {
        self.language_and_direction
    }

    pub const fn input(self) -> Read<EnvironmentInputAspect> {
        self.input
    }

    pub const fn preferences(self) -> Read<EnvironmentPreferencesAspect> {
        self.preferences
    }

    pub const fn view(self) -> Read<EnvironmentViewAspect> {
        self.view
    }
}

/// Runtime-side publication binding for a canonical [`crate::components::application::primitives::EnvironmentState`].
///
/// The binding stores only the last immutable snapshot and its derived read handles. It does not
/// duplicate or mutate the validating environment owner. A host or root assembly first updates its
/// canonical environment, then delivers the resulting [`EnvironmentUpdate`] as a typed component
/// action and publishes it through this binding.
#[derive(Clone, Copy, Debug)]
pub struct EnvironmentReadBinding {
    snapshot: State<EnvironmentSnapshot>,
    reads: EnvironmentReads,
}

impl EnvironmentReadBinding {
    /// Creates one binding in the current component scope from an already validated snapshot.
    pub fn new(
        context: &mut CreateContext<'_>,
        snapshot: EnvironmentSnapshot,
    ) -> RuntimeResult<Self> {
        let snapshot = context.state(snapshot);
        let source = snapshot.read();
        let reads = EnvironmentReads {
            geometry: context.map(source, EnvironmentGeometryAspect::from_snapshot)?,
            scale_and_density: context
                .map(source, EnvironmentScaleAndDensityAspect::from_snapshot)?,
            language_and_direction: context
                .map(source, EnvironmentLanguageAndDirectionAspect::from_snapshot)?,
            input: context.map(source, EnvironmentInputAspect::from_snapshot)?,
            preferences: context.map(source, EnvironmentPreferencesAspect::from_snapshot)?,
            view: context.map(source, EnvironmentViewAspect::from_snapshot)?,
        };
        Ok(Self { snapshot, reads })
    }

    pub const fn reads(self) -> EnvironmentReads {
        self.reads
    }

    /// Atomically stages one contiguous accepted environment update.
    ///
    /// Stale updates, skipped revisions, and change-set mismatches are rejected before the source
    /// state is staged. Runtime state ownership also prevents a binding from being published by a
    /// different component or view.
    pub fn publish<C: Component>(
        &self,
        update: &EnvironmentUpdate,
        context: &mut UpdateContext<'_, C>,
    ) -> RuntimeResult<EnvironmentChangeSet> {
        let current = context.get(self.snapshot)?;
        let actual = changes_between(current.values(), update.snapshot.values());
        if actual != update.changed {
            return Err(RuntimeError::new(
                "environment update change set does not match the bound snapshot",
            ));
        }

        let expected_revision = expected_revision(current.revision(), update.changed)?;
        if update.snapshot.revision().get() != expected_revision {
            return Err(RuntimeError::new(format!(
                "environment update revision {} does not continue bound revision {}",
                update.snapshot.revision().get(),
                current.revision().get(),
            )));
        }

        context.set(self.snapshot, update.snapshot.clone())?;
        Ok(update.changed)
    }
}

fn expected_revision(
    current: crate::components::application::primitives::EnvironmentRevision,
    changed: EnvironmentChangeSet,
) -> RuntimeResult<u64> {
    if changed.is_empty() {
        return Ok(current.get());
    }
    current
        .get()
        .checked_add(1)
        .ok_or_else(|| RuntimeError::new("environment read binding revision is exhausted"))
}

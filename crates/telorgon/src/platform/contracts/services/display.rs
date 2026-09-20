//! Platform-neutral display enumeration and per-view display facts.
//!
//! A display service publishes retained immutable observations. It does not poll a native display
//! source, retain a monitor handle, choose a display mode, move a view, or own a callback, queue,
//! executor, thread, or event loop. Service-wide descriptors reuse the canonical metrics scale,
//! transform, color, and HDR types. Per-view association retains the exact canonical
//! [`ViewMetricsSnapshot`] instead of defining another safe-area or avoidance model.

use std::error::Error;
use std::fmt;
use std::num::{NonZeroU16, NonZeroU64};
use std::rc::Rc;
use std::sync::Arc;

use crate::foundation::RectF;

use super::ServiceKey;
use crate::platform::contracts::{
    AvoidRegion, CapabilityDescriptor, CapabilityLimit, CoordinateSpace, DisplayId,
    DisplayProperties, ExecutionRequirement, MetricInsets, MetricsRevision, PermissionState,
    PhysicalExtent, PlatformError, ScaleFactor, Support, UnavailableReason, UserGestureRequirement,
    ViewId, ViewMetricsSnapshot, ViewRevision, ViewSnapshot,
};

/// Maximum number of connected displays in one neutral enumeration snapshot.
pub const MAX_DISPLAYS: usize = 64;

/// Accuracy claimed for one family of observed display facts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DisplayAccuracy {
    /// The adapter reports the platform's authoritative value without approximation.
    Exact,
    /// The adapter reports a documented approximation or synthesized value.
    Estimated,
    /// The adapter cannot make a stable accuracy claim for this fact.
    #[default]
    Unknown,
}

/// Accuracy dimensions advertised by a display service.
///
/// Safe-area accuracy covers both canonical safe drawing and safe gesture insets. Avoidance
/// accuracy applies to the complete canonical avoid-region list in a per-view metrics snapshot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct DisplayAccuracyProfile {
    logical_bounds: DisplayAccuracy,
    scale_factor: DisplayAccuracy,
    transform: DisplayAccuracy,
    color_space: DisplayAccuracy,
    hdr: DisplayAccuracy,
    safe_areas: DisplayAccuracy,
    avoid_regions: DisplayAccuracy,
}

impl DisplayAccuracyProfile {
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        logical_bounds: DisplayAccuracy,
        scale_factor: DisplayAccuracy,
        transform: DisplayAccuracy,
        color_space: DisplayAccuracy,
        hdr: DisplayAccuracy,
        safe_areas: DisplayAccuracy,
        avoid_regions: DisplayAccuracy,
    ) -> Self {
        Self {
            logical_bounds,
            scale_factor,
            transform,
            color_space,
            hdr,
            safe_areas,
            avoid_regions,
        }
    }

    pub const fn logical_bounds(self) -> DisplayAccuracy {
        self.logical_bounds
    }

    pub const fn scale_factor(self) -> DisplayAccuracy {
        self.scale_factor
    }

    pub const fn transform(self) -> DisplayAccuracy {
        self.transform
    }

    pub const fn color_space(self) -> DisplayAccuracy {
        self.color_space
    }

    pub const fn hdr(self) -> DisplayAccuracy {
        self.hdr
    }

    pub const fn safe_areas(self) -> DisplayAccuracy {
        self.safe_areas
    }

    pub const fn avoid_regions(self) -> DisplayAccuracy {
        self.avoid_regions
    }
}

/// Independently discoverable display observation operations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct DisplayOperations {
    snapshot: bool,
    change_notifications: bool,
    view_association: bool,
}

impl DisplayOperations {
    pub const fn new(snapshot: bool, change_notifications: bool, view_association: bool) -> Self {
        Self {
            snapshot,
            change_notifications,
            view_association,
        }
    }

    pub const fn supports_snapshot(self) -> bool {
        self.snapshot
    }

    pub const fn supports_change_notifications(self) -> bool {
        self.change_notifications
    }

    pub const fn supports_view_association(self) -> bool {
        self.view_association
    }
}

/// Host-advertised enumeration bound, capped by the neutral hard limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DisplayLimits {
    maximum_displays: CapabilityLimit<NonZeroU16>,
}

impl DisplayLimits {
    pub const fn new(
        maximum_displays: CapabilityLimit<NonZeroU16>,
    ) -> Result<Self, DisplayLimitError> {
        if let CapabilityLimit::Bounded(maximum) = maximum_displays
            && maximum.get() as usize > MAX_DISPLAYS
        {
            return Err(DisplayLimitError::DisplayLimitTooLarge);
        }
        Ok(Self { maximum_displays })
    }

    pub const fn maximum_displays(self) -> CapabilityLimit<NonZeroU16> {
        self.maximum_displays
    }
}

impl Default for DisplayLimits {
    fn default() -> Self {
        Self {
            maximum_displays: CapabilityLimit::Bounded(
                NonZeroU16::new(MAX_DISPLAYS as u16)
                    .expect("display enumeration hard bound is nonzero"),
            ),
        }
    }
}

/// Invalid display limit metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DisplayLimitError {
    DisplayLimitTooLarge,
}

impl fmt::Display for DisplayLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("display count limit exceeds the neutral hard bound")
    }
}

impl Error for DisplayLimitError {}

/// Complete display-service capability and its accuracy dimensions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DisplayCapability {
    descriptor: CapabilityDescriptor<DisplayOperations, DisplayLimits>,
    accuracy: DisplayAccuracyProfile,
}

impl DisplayCapability {
    /// Creates observation capability metadata.
    ///
    /// Display observation never requires a recent user gesture. Permission remains explicit for
    /// hosts that restrict topology observation.
    pub const fn new(
        operations: DisplayOperations,
        limits: DisplayLimits,
        accuracy: DisplayAccuracyProfile,
        permission: PermissionState,
        execution: ExecutionRequirement,
    ) -> Self {
        Self {
            descriptor: CapabilityDescriptor::new(
                operations,
                limits,
                permission,
                execution,
                UserGestureRequirement::NotRequired,
            ),
            accuracy,
        }
    }

    pub const fn descriptor(&self) -> &CapabilityDescriptor<DisplayOperations, DisplayLimits> {
        &self.descriptor
    }

    pub const fn operations(self) -> DisplayOperations {
        *self.descriptor.operations()
    }

    pub const fn limits(self) -> DisplayLimits {
        *self.descriptor.limits()
    }

    pub const fn accuracy(self) -> DisplayAccuracyProfile {
        self.accuracy
    }

    pub const fn permission(self) -> PermissionState {
        self.descriptor.permission()
    }

    pub const fn execution(self) -> ExecutionRequirement {
        self.descriptor.execution()
    }
}

/// Monotonic revision of one service's complete display enumeration.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DisplayRevision(NonZeroU64);

impl DisplayRevision {
    pub const INITIAL: Self = Self(NonZeroU64::MIN);

    pub const fn new(revision: NonZeroU64) -> Self {
        Self(revision)
    }

    pub const fn from_raw(revision: u64) -> Option<Self> {
        match NonZeroU64::new(revision) {
            Some(revision) => Some(Self(revision)),
            None => None,
        }
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }

    pub const fn checked_next(self) -> Option<Self> {
        match self.get().checked_add(1) {
            Some(next) => Self::from_raw(next),
            None => None,
        }
    }
}

impl fmt::Display for DisplayRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(formatter)
    }
}

/// Validated display-logical bounds in the adapter's shared desktop coordinate space.
///
/// Origins may be negative. Width and height are finite and strictly positive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayLogicalBounds(RectF);

impl DisplayLogicalBounds {
    pub fn new(bounds: RectF) -> Result<Self, DisplayDescriptorError> {
        if [bounds.x, bounds.y, bounds.width, bounds.height]
            .into_iter()
            .any(|component| !component.is_finite())
        {
            return Err(DisplayDescriptorError::NonFiniteLogicalBounds);
        }
        if bounds.width <= 0.0 || bounds.height <= 0.0 {
            return Err(DisplayDescriptorError::EmptyLogicalBounds);
        }
        Ok(Self(bounds))
    }

    pub const fn bounds(self) -> RectF {
        self.0
    }

    pub const fn coordinate_space(self) -> CoordinateSpace {
        CoordinateSpace::DisplayLogical
    }
}

/// One connected display generation in a retained enumeration snapshot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayDescriptor {
    id: DisplayId,
    logical_bounds: DisplayLogicalBounds,
    physical_extent: PhysicalExtent,
    scale_factor: ScaleFactor,
    properties: DisplayProperties,
}

impl DisplayDescriptor {
    pub fn new(
        id: DisplayId,
        logical_bounds: DisplayLogicalBounds,
        physical_extent: PhysicalExtent,
        scale_factor: ScaleFactor,
        properties: DisplayProperties,
    ) -> Result<Self, DisplayDescriptorError> {
        if !physical_extent.is_renderable() {
            return Err(DisplayDescriptorError::EmptyPhysicalExtent);
        }
        Ok(Self {
            id,
            logical_bounds,
            physical_extent,
            scale_factor,
            properties,
        })
    }

    pub const fn id(self) -> DisplayId {
        self.id
    }

    pub const fn logical_bounds(self) -> DisplayLogicalBounds {
        self.logical_bounds
    }

    pub const fn physical_extent(self) -> PhysicalExtent {
        self.physical_extent
    }

    pub const fn scale_factor(self) -> ScaleFactor {
        self.scale_factor
    }

    pub const fn properties(self) -> DisplayProperties {
        self.properties
    }
}

/// Invalid service-wide display geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DisplayDescriptorError {
    NonFiniteLogicalBounds,
    EmptyLogicalBounds,
    EmptyPhysicalExtent,
}

impl fmt::Display for DisplayDescriptorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFiniteLogicalBounds => "display logical bounds must be finite",
            Self::EmptyLogicalBounds => "display logical bounds must be nonempty",
            Self::EmptyPhysicalExtent => "connected display physical extent must be nonempty",
        })
    }
}

impl Error for DisplayDescriptorError {}

/// Complete immutable display enumeration at one service revision.
#[derive(Clone, Debug, PartialEq)]
pub struct DisplaySnapshot {
    revision: DisplayRevision,
    displays: Arc<[DisplayDescriptor]>,
    primary: Option<DisplayId>,
}

impl DisplaySnapshot {
    /// Constructs a bounded snapshot in stable adapter order.
    ///
    /// An empty snapshot represents an observed headless or disconnected state and therefore
    /// cannot name a primary display.
    pub fn new(
        revision: DisplayRevision,
        displays: Vec<DisplayDescriptor>,
        primary: Option<DisplayId>,
    ) -> Result<Self, DisplaySnapshotError> {
        if displays.len() > MAX_DISPLAYS {
            return Err(DisplaySnapshotError::TooManyDisplays {
                supplied: displays.len(),
                maximum: MAX_DISPLAYS,
            });
        }
        for (index, display) in displays.iter().enumerate() {
            if displays[..index]
                .iter()
                .any(|existing| existing.id == display.id)
            {
                return Err(DisplaySnapshotError::DuplicateDisplay {
                    display: display.id,
                });
            }
        }
        if let Some(primary) = primary
            && !displays.iter().any(|display| display.id == primary)
        {
            return Err(DisplaySnapshotError::PrimaryDisplayMissing { display: primary });
        }
        Ok(Self {
            revision,
            displays: displays.into(),
            primary,
        })
    }

    pub const fn revision(&self) -> DisplayRevision {
        self.revision
    }

    pub fn displays(&self) -> &[DisplayDescriptor] {
        &self.displays
    }

    pub const fn primary(&self) -> Option<DisplayId> {
        self.primary
    }

    pub fn primary_display(&self) -> Option<&DisplayDescriptor> {
        self.primary.and_then(|primary| self.display(primary))
    }

    pub fn display(&self, id: DisplayId) -> Option<&DisplayDescriptor> {
        self.displays.iter().find(|display| display.id == id)
    }

    pub fn len(&self) -> usize {
        self.displays.len()
    }

    pub fn is_empty(&self) -> bool {
        self.displays.is_empty()
    }
}

/// Invalid complete display enumeration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DisplaySnapshotError {
    TooManyDisplays { supplied: usize, maximum: usize },
    DuplicateDisplay { display: DisplayId },
    PrimaryDisplayMissing { display: DisplayId },
}

impl fmt::Display for DisplaySnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyDisplays { supplied, maximum } => {
                write!(
                    formatter,
                    "display snapshot contains {supplied} displays; maximum is {maximum}"
                )
            }
            Self::DuplicateDisplay { display } => {
                write!(formatter, "display snapshot repeats display {display}")
            }
            Self::PrimaryDisplayMissing { display } => {
                write!(
                    formatter,
                    "primary display {display} is absent from the snapshot"
                )
            }
        }
    }
}

impl Error for DisplaySnapshotError {}

/// One immutable display-topology change payload carrying the complete new snapshot.
///
/// A first publication may omit `previous`. Otherwise the service-wide revision must strictly
/// advance. Ordering time belongs to the host's event boundary rather than this payload.
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayChange {
    previous: Option<DisplayRevision>,
    current: DisplaySnapshot,
}

impl DisplayChange {
    pub fn new(
        previous: Option<DisplayRevision>,
        current: DisplaySnapshot,
    ) -> Result<Self, DisplayChangeError> {
        if let Some(previous) = previous
            && current.revision <= previous
        {
            return Err(DisplayChangeError::RevisionDidNotAdvance {
                previous,
                current: current.revision,
            });
        }
        Ok(Self { previous, current })
    }

    pub const fn previous(&self) -> Option<DisplayRevision> {
        self.previous
    }

    pub const fn current(&self) -> &DisplaySnapshot {
        &self.current
    }
}

/// Invalid display change history.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DisplayChangeError {
    RevisionDidNotAdvance {
        previous: DisplayRevision,
        current: DisplayRevision,
    },
}

impl fmt::Display for DisplayChangeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RevisionDidNotAdvance { previous, current } => write!(
                formatter,
                "display revision {current} did not advance after {previous}"
            ),
        }
    }
}

impl Error for DisplayChangeError {}

/// Exact display association and canonical metrics for one immutable view publication.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewDisplaySnapshot {
    view: ViewId,
    view_revision: ViewRevision,
    display_revision: DisplayRevision,
    display: Option<DisplayId>,
    metrics: ViewMetricsSnapshot,
}

impl ViewDisplaySnapshot {
    /// Captures association metadata against one exact enumeration and view publication.
    ///
    /// A present association must name a display in the cited enumeration. Metrics are retained
    /// unchanged and no coordinate conversion occurs.
    pub fn new(
        snapshot: &ViewSnapshot,
        displays: &DisplaySnapshot,
        display: Option<DisplayId>,
    ) -> Result<Self, ViewDisplayError> {
        if let Some(display) = display
            && displays.display(display).is_none()
        {
            return Err(ViewDisplayError::DisplayMissing {
                display,
                revision: displays.revision(),
            });
        }
        Ok(Self {
            view: snapshot.view(),
            view_revision: snapshot.revision(),
            display_revision: displays.revision(),
            display,
            metrics: snapshot.metrics().clone(),
        })
    }

    pub const fn view(&self) -> ViewId {
        self.view
    }

    pub const fn view_revision(&self) -> ViewRevision {
        self.view_revision
    }

    pub const fn display(&self) -> Option<DisplayId> {
        self.display
    }

    pub const fn display_revision(&self) -> DisplayRevision {
        self.display_revision
    }

    pub const fn metrics_revision(&self) -> MetricsRevision {
        self.metrics.revision()
    }

    pub const fn metrics(&self) -> &ViewMetricsSnapshot {
        &self.metrics
    }

    pub fn scale_factor(&self) -> ScaleFactor {
        self.metrics.metrics().scale_factor()
    }

    pub fn display_properties(&self) -> DisplayProperties {
        self.metrics.metrics().display()
    }

    pub fn safe_drawing_insets(&self) -> MetricInsets {
        self.metrics.metrics().safe_drawing_insets()
    }

    pub fn safe_gesture_insets(&self) -> MetricInsets {
        self.metrics.metrics().safe_gesture_insets()
    }

    pub fn avoid_regions(&self) -> &[AvoidRegion] {
        self.metrics.metrics().avoid_regions()
    }
}

/// Invalid association between an exact view publication and display enumeration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ViewDisplayError {
    DisplayMissing {
        display: DisplayId,
        revision: DisplayRevision,
    },
}

impl fmt::Display for ViewDisplayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DisplayMissing { display, revision } => write!(
                formatter,
                "display {display} is absent from enumeration revision {revision}"
            ),
        }
    }
}

impl Error for ViewDisplayError {}

/// Result of querying the current retained display enumeration.
#[derive(Clone, Debug, PartialEq)]
#[must_use = "display snapshot absence and failure must be handled explicitly"]
pub enum DisplaySnapshotStatus {
    Current(DisplaySnapshot),
    Unavailable(UnavailableReason),
    Failed(PlatformError),
}

impl DisplaySnapshotStatus {
    pub const fn current(&self) -> Option<&DisplaySnapshot> {
        match self {
            Self::Current(snapshot) => Some(snapshot),
            Self::Unavailable(_) | Self::Failed(_) => None,
        }
    }

    pub const fn unavailable_reason(&self) -> Option<UnavailableReason> {
        match self {
            Self::Unavailable(reason) => Some(*reason),
            Self::Current(_) | Self::Failed(_) => None,
        }
    }

    pub const fn failure(&self) -> Option<PlatformError> {
        match self {
            Self::Failed(error) => Some(*error),
            Self::Current(_) | Self::Unavailable(_) => None,
        }
    }
}

/// Result of associating an exact view publication with a retained display generation.
#[derive(Clone, Debug, PartialEq)]
#[must_use = "view/display association absence and failure must be handled explicitly"]
pub enum ViewDisplayStatus {
    Current(ViewDisplaySnapshot),
    Unavailable(UnavailableReason),
    Failed(PlatformError),
}

impl ViewDisplayStatus {
    pub const fn current(&self) -> Option<&ViewDisplaySnapshot> {
        match self {
            Self::Current(snapshot) => Some(snapshot),
            Self::Unavailable(_) | Self::Failed(_) => None,
        }
    }

    pub const fn unavailable_reason(&self) -> Option<UnavailableReason> {
        match self {
            Self::Unavailable(reason) => Some(*reason),
            Self::Current(_) | Self::Failed(_) => None,
        }
    }

    pub const fn failure(&self) -> Option<PlatformError> {
        match self {
            Self::Failed(error) => Some(*error),
            Self::Current(_) | Self::Unavailable(_) => None,
        }
    }
}

/// Narrow retained-observation surface for display topology and exact view association.
pub trait DisplayService {
    fn capability(&self) -> Support<DisplayCapability>;

    /// Returns the latest retained complete enumeration without forcing native polling.
    fn current_snapshot(&self) -> DisplaySnapshotStatus;

    /// Associates one exact immutable view publication with its retained display generation.
    fn for_view(&self, view: &ViewSnapshot) -> ViewDisplayStatus;
}

/// Type-level registry key for an owner-local display service handle.
pub enum DisplayServiceKey {}

impl ServiceKey for DisplayServiceKey {
    type Handle = Rc<dyn DisplayService>;
}

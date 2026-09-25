//! Platform-neutral scroll offset, extent, reveal, and physics transitions.
//!
//! This module owns no input routing, scheduler, animation clock, component state, or rendering.
//! Callers deliver logical deltas and elapsed time, then apply the returned transition through the
//! runtime/layout owners.

use std::num::NonZeroU64;
use std::time::Duration;

use crate::foundation::{PointF, RectF, SizeF};

const OFFSET_EPSILON: f32 = 0.0001;

/// Validated scroll geometry in logical coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollMetrics {
    pub offset: PointF,
    pub viewport: SizeF,
    pub content: SizeF,
}

impl ScrollMetrics {
    pub fn max_offset(self) -> PointF {
        PointF {
            x: (self.content.width - self.viewport.width).max(0.0),
            y: (self.content.height - self.viewport.height).max(0.0),
        }
    }

    pub fn visible_rect(self) -> RectF {
        RectF {
            x: self.offset.x,
            y: self.offset.y,
            width: self.viewport.width,
            height: self.viewport.height,
        }
    }

    pub fn can_scroll_left(self) -> bool {
        self.offset.x > OFFSET_EPSILON
    }

    pub fn can_scroll_right(self) -> bool {
        self.offset.x + OFFSET_EPSILON < self.max_offset().x
    }

    pub fn can_scroll_up(self) -> bool {
        self.offset.y > OFFSET_EPSILON
    }

    pub fn can_scroll_down(self) -> bool {
        self.offset.y + OFFSET_EPSILON < self.max_offset().y
    }
}

/// How an offset responds when content or viewport extents change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScrollAnchorMode {
    /// Preserve the current offset when possible, otherwise clamp it into the new bounds.
    #[default]
    Clamp,
    /// Preserve the current distance from the content end on this axis.
    PreserveEndDistance,
}

/// Independent horizontal and vertical extent-correction policy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScrollExtentAnchor {
    pub horizontal: ScrollAnchorMode,
    pub vertical: ScrollAnchorMode,
}

impl ScrollExtentAnchor {
    pub const END: Self = Self {
        horizontal: ScrollAnchorMode::PreserveEndDistance,
        vertical: ScrollAnchorMode::PreserveEndDistance,
    };
}

/// Alignment used to calculate a reveal target on one axis.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum RevealAlignment {
    /// Move only far enough to expose the nearest obscured edge.
    #[default]
    Nearest,
    Start,
    Center,
    End,
    /// Align within the free viewport space, where zero is start and one is end.
    Fraction(f32),
}

/// Content-space rectangle and axes requested for reveal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RevealRequest {
    pub target: RectF,
    pub horizontal: Option<RevealAlignment>,
    pub vertical: Option<RevealAlignment>,
}

impl RevealRequest {
    pub const fn nearest(target: RectF) -> Self {
        Self {
            target,
            horizontal: Some(RevealAlignment::Nearest),
            vertical: Some(RevealAlignment::Nearest),
        }
    }
}

/// One caller-owned ballistic motion generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ScrollMotionId(NonZeroU64);

impl ScrollMotionId {
    pub const fn from_raw(generation: u64) -> Option<Self> {
        match NonZeroU64::new(generation) {
            Some(generation) => Some(Self(generation)),
            None => None,
        }
    }

    pub const fn generation(self) -> u64 {
        self.0.get()
    }
}

/// Current scroll activity. The runtime remains responsible for capture and frame scheduling.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScrollActivity {
    #[default]
    Idle,
    Dragging,
    Ballistic(ScrollMotionId),
}

/// Motion handoff emitted for the caller's frame scheduler.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScrollMotionRequest {
    #[default]
    None,
    Start(ScrollMotionId),
    Continue(ScrollMotionId),
    Stop(ScrollMotionId),
}

/// Source for a discrete, non-drag scroll mutation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollInputSource {
    Pointer,
    Wheel,
    Keyboard,
    Semantic,
    Programmatic,
}

/// Reason attached to a completed scroll transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollChangeSource {
    ExtentCorrection,
    Input(ScrollInputSource),
    Reveal,
    Drag,
    Ballistic,
    Activity,
    Cancellation(ScrollCancelReason),
}

/// Lifecycle causes that stop a drag or ballistic motion without committing more movement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollCancelReason {
    GestureCancelled,
    CaptureLost,
    ViewDeactivated,
    Disabled,
    Unmounted,
    Replaced,
}

/// Clamping ballistic policy. Elapsed time is always supplied by the caller.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "SavedScrollPhysics", into = "SavedScrollPhysics")]
pub struct ScrollPhysics {
    deceleration: f32,
    stop_velocity: f32,
}

impl ScrollPhysics {
    pub fn new(deceleration: f32, stop_velocity: f32) -> Result<Self, ScrollError> {
        if !deceleration.is_finite()
            || deceleration <= 0.0
            || !stop_velocity.is_finite()
            || stop_velocity < 0.0
        {
            return Err(ScrollError::InvalidPhysics);
        }
        Ok(Self {
            deceleration,
            stop_velocity,
        })
    }

    pub const fn deceleration(self) -> f32 {
        self.deceleration
    }

    pub const fn stop_velocity(self) -> f32 {
        self.stop_velocity
    }
}

impl Default for ScrollPhysics {
    fn default() -> Self {
        Self {
            deceleration: 2_500.0,
            stop_velocity: 5.0,
        }
    }
}

/// Complete observable result of one accepted scroll transition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollUpdate {
    pub before: ScrollMetrics,
    pub after: ScrollMetrics,
    pub requested_delta: PointF,
    pub consumed_delta: PointF,
    pub unconsumed_delta: PointF,
    pub source: ScrollChangeSource,
    pub activity_before: ScrollActivity,
    pub activity_after: ScrollActivity,
    pub motion: ScrollMotionRequest,
}

impl ScrollUpdate {
    pub fn changed(self) -> bool {
        self.before != self.after || self.activity_before != self.activity_after
    }
}

/// Deterministic counters for later per-view aggregation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScrollDiagnostics {
    pub extent_updates: u64,
    pub offset_updates: u64,
    pub reveal_requests: u64,
    pub reveal_noops: u64,
    pub drag_starts: u64,
    pub drag_ends: u64,
    pub ballistics_started: u64,
    pub motion_steps: u64,
    pub cancellations: u64,
    pub boundary_hits: u64,
    pub invalid_requests: u64,
    pub stale_motion_steps: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollError {
    InvalidExtent,
    InvalidDelta,
    InvalidOffset,
    InvalidRevealTarget,
    InvalidRevealAlignment,
    ViewportUnavailable,
    InvalidPhysics,
    InvalidVelocity,
    NotDragging,
    NoActiveMotion,
    StaleMotion {
        expected: ScrollMotionId,
        received: ScrollMotionId,
    },
    InvalidElapsedTime,
}

#[derive(Clone, Copy, Debug)]
struct BallisticMotion {
    id: ScrollMotionId,
    velocity: PointF,
    physics: ScrollPhysics,
}

/// Pure state owner for one two-dimensional scroll position.
#[derive(Clone, Debug)]
pub struct ScrollState {
    metrics: ScrollMetrics,
    activity: ScrollActivity,
    motion: Option<BallisticMotion>,
    next_motion_generation: u64,
    diagnostics: ScrollDiagnostics,
}

impl Default for ScrollState {
    fn default() -> Self {
        Self {
            metrics: ScrollMetrics::default(),
            activity: ScrollActivity::Idle,
            motion: None,
            next_motion_generation: 0,
            diagnostics: ScrollDiagnostics::default(),
        }
    }
}

impl ScrollState {
    pub fn new(viewport: SizeF, content: SizeF) -> Result<Self, ScrollError> {
        validate_extents(viewport, content)?;
        Ok(Self {
            metrics: ScrollMetrics {
                offset: PointF::default(),
                viewport,
                content,
            },
            ..Self::default()
        })
    }

    pub const fn metrics(&self) -> ScrollMetrics {
        self.metrics
    }

    pub const fn activity(&self) -> ScrollActivity {
        self.activity
    }

    pub const fn diagnostics(&self) -> ScrollDiagnostics {
        self.diagnostics
    }

    pub fn velocity(&self) -> PointF {
        self.motion
            .map(|motion| motion.velocity)
            .unwrap_or_default()
    }

    pub fn set_extents(
        &mut self,
        viewport: SizeF,
        content: SizeF,
        anchor: ScrollExtentAnchor,
    ) -> Result<ScrollUpdate, ScrollError> {
        if let Err(error) = validate_extents(viewport, content) {
            self.diagnostics.invalid_requests += 1;
            return Err(error);
        }

        let before = self.metrics;
        let activity_before = self.activity;
        let old_max = before.max_offset();
        let mut after = ScrollMetrics {
            offset: before.offset,
            viewport,
            content,
        };
        let new_max = after.max_offset();
        after.offset = PointF {
            x: anchored_offset(before.offset.x, old_max.x, new_max.x, anchor.horizontal),
            y: anchored_offset(before.offset.y, old_max.y, new_max.y, anchor.vertical),
        };

        self.metrics = after;
        self.diagnostics.extent_updates += 1;
        let changed_offset = subtract(after.offset, before.offset);
        if !point_is_zero(changed_offset) {
            self.diagnostics.offset_updates += 1;
        }
        let motion = if !point_is_zero(changed_offset) {
            self.stop_ballistic_for_replacement()
        } else {
            ScrollMotionRequest::None
        };

        Ok(ScrollUpdate {
            before,
            after: self.metrics,
            requested_delta: changed_offset,
            consumed_delta: changed_offset,
            unconsumed_delta: PointF::default(),
            source: ScrollChangeSource::ExtentCorrection,
            activity_before,
            activity_after: self.activity,
            motion,
        })
    }

    pub fn scroll_by(
        &mut self,
        delta: PointF,
        source: ScrollInputSource,
    ) -> Result<ScrollUpdate, ScrollError> {
        if !point_is_finite(delta) {
            self.diagnostics.invalid_requests += 1;
            return Err(ScrollError::InvalidDelta);
        }
        let activity_before = self.activity;
        let motion = self.stop_activity_for_replacement();
        Ok(self.apply_delta(
            delta,
            ScrollChangeSource::Input(source),
            activity_before,
            motion,
        ))
    }

    pub fn scroll_to(
        &mut self,
        offset: PointF,
        source: ScrollInputSource,
    ) -> Result<ScrollUpdate, ScrollError> {
        if !point_is_finite(offset) {
            self.diagnostics.invalid_requests += 1;
            return Err(ScrollError::InvalidOffset);
        }
        self.scroll_by(subtract(offset, self.metrics.offset), source)
    }

    pub fn begin_drag(&mut self) -> ScrollUpdate {
        let before = self.metrics;
        let activity_before = self.activity;
        if self.activity == ScrollActivity::Dragging {
            return self.stationary_update(
                before,
                ScrollChangeSource::Activity,
                activity_before,
                ScrollMotionRequest::None,
            );
        }
        let motion = self.stop_activity_for_replacement();
        self.activity = ScrollActivity::Dragging;
        self.diagnostics.drag_starts += 1;
        self.stationary_update(
            before,
            ScrollChangeSource::Activity,
            activity_before,
            motion,
        )
    }

    pub fn drag_by(&mut self, delta: PointF) -> Result<ScrollUpdate, ScrollError> {
        if !point_is_finite(delta) {
            self.diagnostics.invalid_requests += 1;
            return Err(ScrollError::InvalidDelta);
        }
        if self.activity != ScrollActivity::Dragging {
            self.diagnostics.invalid_requests += 1;
            return Err(ScrollError::NotDragging);
        }
        Ok(self.apply_delta(
            delta,
            ScrollChangeSource::Drag,
            ScrollActivity::Dragging,
            ScrollMotionRequest::None,
        ))
    }

    pub fn end_drag(
        &mut self,
        velocity: PointF,
        physics: ScrollPhysics,
        reduced_motion: bool,
    ) -> Result<ScrollUpdate, ScrollError> {
        if self.activity != ScrollActivity::Dragging {
            self.diagnostics.invalid_requests += 1;
            return Err(ScrollError::NotDragging);
        }
        if !point_is_finite(velocity) {
            self.diagnostics.invalid_requests += 1;
            return Err(ScrollError::InvalidVelocity);
        }
        if ScrollPhysics::new(physics.deceleration, physics.stop_velocity).is_err() {
            self.diagnostics.invalid_requests += 1;
            return Err(ScrollError::InvalidPhysics);
        }

        let before = self.metrics;
        let activity_before = self.activity;
        self.diagnostics.drag_ends += 1;
        if reduced_motion || velocity_below_threshold(velocity, physics.stop_velocity) {
            self.activity = ScrollActivity::Idle;
            return Ok(self.stationary_update(
                before,
                ScrollChangeSource::Activity,
                activity_before,
                ScrollMotionRequest::None,
            ));
        }

        let id = self.next_motion_id();
        self.motion = Some(BallisticMotion {
            id,
            velocity,
            physics,
        });
        self.activity = ScrollActivity::Ballistic(id);
        self.diagnostics.ballistics_started += 1;
        Ok(self.stationary_update(
            before,
            ScrollChangeSource::Activity,
            activity_before,
            ScrollMotionRequest::Start(id),
        ))
    }

    pub fn step_motion(
        &mut self,
        id: ScrollMotionId,
        elapsed: Duration,
    ) -> Result<ScrollUpdate, ScrollError> {
        let Some(motion) = self.motion else {
            self.diagnostics.invalid_requests += 1;
            return Err(ScrollError::NoActiveMotion);
        };
        if motion.id != id {
            self.diagnostics.invalid_requests += 1;
            self.diagnostics.stale_motion_steps += 1;
            return Err(ScrollError::StaleMotion {
                expected: motion.id,
                received: id,
            });
        }
        if elapsed.is_zero() {
            self.diagnostics.invalid_requests += 1;
            return Err(ScrollError::InvalidElapsedTime);
        }

        let seconds = elapsed.as_secs_f32();
        let (delta_x, mut velocity_x) = integrate_axis(
            motion.velocity.x,
            motion.physics.deceleration,
            motion.physics.stop_velocity,
            seconds,
        );
        let (delta_y, mut velocity_y) = integrate_axis(
            motion.velocity.y,
            motion.physics.deceleration,
            motion.physics.stop_velocity,
            seconds,
        );
        let mut update = self.apply_delta(
            PointF {
                x: delta_x,
                y: delta_y,
            },
            ScrollChangeSource::Ballistic,
            ScrollActivity::Ballistic(id),
            ScrollMotionRequest::Continue(id),
        );
        self.diagnostics.motion_steps += 1;

        if update.unconsumed_delta.x.abs() > OFFSET_EPSILON {
            velocity_x = 0.0;
        }
        if update.unconsumed_delta.y.abs() > OFFSET_EPSILON {
            velocity_y = 0.0;
        }
        let velocity = PointF {
            x: velocity_x,
            y: velocity_y,
        };
        if point_is_zero(velocity) {
            self.motion = None;
            self.activity = ScrollActivity::Idle;
            update.activity_after = self.activity;
            update.motion = ScrollMotionRequest::Stop(id);
        } else {
            self.motion = Some(BallisticMotion { velocity, ..motion });
        }
        Ok(update)
    }

    pub fn cancel(&mut self, reason: ScrollCancelReason) -> ScrollUpdate {
        let before = self.metrics;
        let activity_before = self.activity;
        let motion = match self.activity {
            ScrollActivity::Ballistic(id) => ScrollMotionRequest::Stop(id),
            ScrollActivity::Idle | ScrollActivity::Dragging => ScrollMotionRequest::None,
        };
        if self.activity != ScrollActivity::Idle {
            self.diagnostics.cancellations += 1;
        }
        self.motion = None;
        self.activity = ScrollActivity::Idle;
        self.stationary_update(
            before,
            ScrollChangeSource::Cancellation(reason),
            activity_before,
            motion,
        )
    }

    pub fn reveal_target(&self, request: RevealRequest) -> Result<PointF, ScrollError> {
        validate_reveal(request)?;
        let max = self.metrics.max_offset();
        let x = match request.horizontal {
            Some(alignment) => reveal_axis(
                self.metrics.offset.x,
                self.metrics.viewport.width,
                request.target.x,
                request.target.width,
                max.x,
                alignment,
            )?,
            None => self.metrics.offset.x,
        };
        let y = match request.vertical {
            Some(alignment) => reveal_axis(
                self.metrics.offset.y,
                self.metrics.viewport.height,
                request.target.y,
                request.target.height,
                max.y,
                alignment,
            )?,
            None => self.metrics.offset.y,
        };
        Ok(PointF { x, y })
    }

    pub fn reveal(&mut self, request: RevealRequest) -> Result<ScrollUpdate, ScrollError> {
        self.diagnostics.reveal_requests += 1;
        let target = match self.reveal_target(request) {
            Ok(target) => target,
            Err(error) => {
                self.diagnostics.invalid_requests += 1;
                return Err(error);
            }
        };
        let delta = subtract(target, self.metrics.offset);
        if point_is_zero(delta) {
            self.diagnostics.reveal_noops += 1;
            return Ok(self.stationary_update(
                self.metrics,
                ScrollChangeSource::Reveal,
                self.activity,
                ScrollMotionRequest::None,
            ));
        }
        let activity_before = self.activity;
        let motion = self.stop_activity_for_replacement();
        Ok(self.apply_delta(delta, ScrollChangeSource::Reveal, activity_before, motion))
    }

    fn apply_delta(
        &mut self,
        delta: PointF,
        source: ScrollChangeSource,
        activity_before: ScrollActivity,
        motion: ScrollMotionRequest,
    ) -> ScrollUpdate {
        let before = self.metrics;
        let max = before.max_offset();
        let requested = PointF {
            x: before.offset.x + delta.x,
            y: before.offset.y + delta.y,
        };
        self.metrics.offset = PointF {
            x: requested.x.clamp(0.0, max.x),
            y: requested.y.clamp(0.0, max.y),
        };
        let consumed = subtract(self.metrics.offset, before.offset);
        let unconsumed = subtract(delta, consumed);
        if !point_is_zero(consumed) {
            self.diagnostics.offset_updates += 1;
        }
        if !point_is_zero(unconsumed) {
            self.diagnostics.boundary_hits += 1;
        }
        ScrollUpdate {
            before,
            after: self.metrics,
            requested_delta: delta,
            consumed_delta: consumed,
            unconsumed_delta: unconsumed,
            source,
            activity_before,
            activity_after: self.activity,
            motion,
        }
    }

    fn stationary_update(
        &self,
        before: ScrollMetrics,
        source: ScrollChangeSource,
        activity_before: ScrollActivity,
        motion: ScrollMotionRequest,
    ) -> ScrollUpdate {
        ScrollUpdate {
            before,
            after: self.metrics,
            requested_delta: PointF::default(),
            consumed_delta: PointF::default(),
            unconsumed_delta: PointF::default(),
            source,
            activity_before,
            activity_after: self.activity,
            motion,
        }
    }

    fn stop_activity_for_replacement(&mut self) -> ScrollMotionRequest {
        let request = match self.activity {
            ScrollActivity::Ballistic(id) => ScrollMotionRequest::Stop(id),
            ScrollActivity::Idle | ScrollActivity::Dragging => ScrollMotionRequest::None,
        };
        if self.activity != ScrollActivity::Idle {
            self.diagnostics.cancellations += 1;
        }
        self.motion = None;
        self.activity = ScrollActivity::Idle;
        request
    }

    fn stop_ballistic_for_replacement(&mut self) -> ScrollMotionRequest {
        let ScrollActivity::Ballistic(id) = self.activity else {
            return ScrollMotionRequest::None;
        };
        self.motion = None;
        self.activity = ScrollActivity::Idle;
        self.diagnostics.cancellations += 1;
        ScrollMotionRequest::Stop(id)
    }

    fn next_motion_id(&mut self) -> ScrollMotionId {
        self.next_motion_generation = self.next_motion_generation.wrapping_add(1);
        if self.next_motion_generation == 0 {
            self.next_motion_generation = 1;
        }
        ScrollMotionId(NonZeroU64::new(self.next_motion_generation).expect("generation is nonzero"))
    }
}

fn validate_extents(viewport: SizeF, content: SizeF) -> Result<(), ScrollError> {
    if !viewport.width.is_finite()
        || !viewport.height.is_finite()
        || !content.width.is_finite()
        || !content.height.is_finite()
        || viewport.width < 0.0
        || viewport.height < 0.0
        || content.width < 0.0
        || content.height < 0.0
    {
        return Err(ScrollError::InvalidExtent);
    }
    Ok(())
}

fn validate_reveal(request: RevealRequest) -> Result<(), ScrollError> {
    if !request.target.x.is_finite()
        || !request.target.y.is_finite()
        || !request.target.width.is_finite()
        || !request.target.height.is_finite()
        || !request.target.right().is_finite()
        || !request.target.bottom().is_finite()
        || request.target.width < 0.0
        || request.target.height < 0.0
    {
        return Err(ScrollError::InvalidRevealTarget);
    }
    for alignment in [request.horizontal, request.vertical].into_iter().flatten() {
        if let RevealAlignment::Fraction(value) = alignment
            && (!value.is_finite() || !(0.0..=1.0).contains(&value))
        {
            return Err(ScrollError::InvalidRevealAlignment);
        }
    }
    Ok(())
}

fn anchored_offset(current: f32, old_max: f32, new_max: f32, mode: ScrollAnchorMode) -> f32 {
    let candidate = match mode {
        ScrollAnchorMode::Clamp => current,
        ScrollAnchorMode::PreserveEndDistance => new_max - (old_max - current).max(0.0),
    };
    candidate.clamp(0.0, new_max)
}

fn reveal_axis(
    current: f32,
    viewport: f32,
    target_start: f32,
    target_size: f32,
    max_offset: f32,
    alignment: RevealAlignment,
) -> Result<f32, ScrollError> {
    if viewport <= 0.0 {
        return Err(ScrollError::ViewportUnavailable);
    }
    let target_end = target_start + target_size;
    let visible_end = current + viewport;
    let candidate = match alignment {
        RevealAlignment::Nearest => {
            let fully_visible = target_start >= current && target_end <= visible_end;
            let spans_viewport = target_start < current && target_end > visible_end;
            if fully_visible || spans_viewport {
                return Ok(current);
            }
            let align_start = target_start;
            let align_end = target_end - viewport;
            if (align_start - current).abs() < (align_end - current).abs() {
                align_start
            } else {
                align_end
            }
        }
        RevealAlignment::Start => target_start,
        RevealAlignment::Center => target_start - (viewport - target_size) * 0.5,
        RevealAlignment::End => target_end - viewport,
        RevealAlignment::Fraction(fraction) => target_start - (viewport - target_size) * fraction,
    };
    Ok(candidate.clamp(0.0, max_offset))
}

fn integrate_axis(
    velocity: f32,
    deceleration: f32,
    stop_velocity: f32,
    seconds: f32,
) -> (f32, f32) {
    if velocity.abs() <= stop_velocity {
        return (0.0, 0.0);
    }
    let velocity = f64::from(velocity);
    let deceleration = f64::from(deceleration);
    let stop_velocity = f64::from(stop_velocity);
    let sign = velocity.signum();
    let active_seconds = f64::from(seconds).min(velocity.abs() / deceleration);
    let delta =
        velocity * active_seconds - sign * 0.5 * deceleration * active_seconds * active_seconds;
    let delta = delta.clamp(f64::from(f32::MIN), f64::from(f32::MAX)) as f32;
    let remaining = (velocity.abs() - deceleration * active_seconds).max(0.0);
    let next_velocity = if remaining <= stop_velocity {
        0.0
    } else {
        (sign * remaining) as f32
    };
    (delta, next_velocity)
}

fn velocity_below_threshold(velocity: PointF, threshold: f32) -> bool {
    velocity.x.abs() <= threshold && velocity.y.abs() <= threshold
}

fn point_is_finite(point: PointF) -> bool {
    point.x.is_finite() && point.y.is_finite()
}

fn point_is_zero(point: PointF) -> bool {
    point.x.abs() <= OFFSET_EPSILON && point.y.abs() <= OFFSET_EPSILON
}

fn subtract(left: PointF, right: PointF) -> PointF {
    PointF {
        x: left.x - right.x,
        y: left.y - right.y,
    }
}

#[cfg(test)]
mod tests;

#[derive(serde::Serialize, serde::Deserialize)]
struct SavedScrollPhysics {
    deceleration: f32,
    stop_velocity: f32,
}
impl From<ScrollPhysics> for SavedScrollPhysics {
    fn from(value: ScrollPhysics) -> Self {
        Self {
            deceleration: value.deceleration(),
            stop_velocity: value.stop_velocity(),
        }
    }
}
impl TryFrom<SavedScrollPhysics> for ScrollPhysics {
    type Error = &'static str;
    fn try_from(value: SavedScrollPhysics) -> Result<Self, Self::Error> {
        Self::new(value.deceleration, value.stop_velocity).map_err(|_| "invalid scroll physics")
    }
}

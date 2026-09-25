//! Presentation policy owns the frame mailbox, outstanding commit and refresh pacing.
//! Resource transitions and DRM calls remain in the host; planning never waits or submits IO.
use super::frame_pacer::FramePacer;
use std::{collections::VecDeque, time::Duration};

pub(super) trait Commit {
    fn primary_slot(&self) -> Option<usize>;
}

#[derive(Clone, Copy, Default)]
pub(super) struct Work {
    pub primary_damage: bool,
    pub primary_animation: bool,
    pub cursor_dirty: bool,
    pub scanout_available: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Submission {
    Primary,
    Cursor,
}

#[derive(Debug, Default)]
pub(super) struct Plan {
    pub render: bool,
    /// Active animation must produce a successor even when this sample has no delta.
    pub force_frame: bool,
    pub submit: Option<Submission>,
    pub deadline_ns: Option<u64>,
}
impl Plan {
    /// IO readiness always wins over pacing. The caller may shorten this timeout for
    /// other services, but must never extend it or sleep while an event is pending.
    pub fn wait(
        &self,
        io_ready: bool,
        runtime_deadline_ns: Option<u64>,
        now_ns: u64,
    ) -> Option<Duration> {
        if io_ready || self.render || self.submit.is_some() {
            return Some(Duration::ZERO);
        }
        self.deadline_ns
            .into_iter()
            .chain(runtime_deadline_ns)
            .min()
            .map(|deadline| Duration::from_nanos(deadline.saturating_sub(now_ns)))
    }
}

#[derive(Clone, Copy)]
enum Waiting {
    Commit,
    Pacing(u64),
    Dispatch,
}
#[derive(Clone, Copy)]
struct Frame {
    slot: usize,
    target: u64,
    observed: u64,
    ready_at: u64,
    waiting: Waiting,
    waits: [u64; 3],
}
impl Frame {
    fn advance(&mut self, now: u64) {
        let elapsed = now.saturating_sub(self.observed);
        match self.waiting {
            Waiting::Commit => self.waits[0] += elapsed,
            Waiting::Dispatch => self.waits[2] += elapsed,
            Waiting::Pacing(deadline) => {
                let pacing = deadline.saturating_sub(self.observed).min(elapsed);
                self.waits[1] += pacing;
                self.waits[2] += elapsed - pacing;
            }
        }
        self.observed = now;
    }
}

/// Translate a validated monotonic DRM timestamp by event age, never by assuming
/// that Rust's Instant epoch equals CLOCK_MONOTONIC's epoch.
pub(super) fn flip_time(
    timestamp_us: u64,
    monotonic_us: Option<u64>,
    observed_ns: u64,
) -> Option<u64> {
    if timestamp_us == 0 {
        return None;
    }
    let age = monotonic_us?.checked_sub(timestamp_us)?.checked_mul(1000)?;
    observed_ns.checked_sub(age)
}

pub(super) struct Scheduler<C> {
    pacer: FramePacer,
    ready: VecDeque<Frame>,
    pending: Option<C>,
    rendering: Option<Frame>,
    selected_target: Option<u64>,
    pending_target: Option<u64>,
    refresh_ns: u64,
    last_scanout_ns: Option<u64>,
    background_due_ns: u64,
    background_pending: bool,
    background_cost_ns: u64,
}
impl<C: Commit> Scheduler<C> {
    pub fn new(period: Duration) -> Self {
        Self {
            pacer: FramePacer::new(period),
            ready: VecDeque::new(),
            pending: None,
            rendering: None,
            selected_target: None,
            pending_target: None,
            refresh_ns: period.as_nanos().min(u128::from(u64::MAX)) as u64,
            last_scanout_ns: None,
            background_due_ns: 0,
            background_pending: false,
            background_cost_ns: 4_000_000,
        }
    }
    pub fn request_background(&mut self, pending: bool) {
        self.background_pending = pending;
    }
    fn background_turn(&self, now: u64) -> bool {
        self.background_pending && now >= self.background_due_ns && self.primary_pending()
    }
    /// Admit background work only after display work is prepared. Reserve measured
    /// completion cost plus submission margin; allow progress every four refreshes
    /// when a continuously busy display leaves no slack. Never queue behind rendering.
    pub fn background_allowed(&self, now: u64, display_work: bool) -> bool {
        if self.rendering.is_some() {
            return false;
        }
        if !display_work {
            return true;
        }
        if self.ready.is_empty() && !self.primary_pending() {
            return false;
        }
        now >= self.background_due_ns
            || self.last_scanout_ns.is_some_and(|last| {
                last.saturating_add(self.refresh_ns).saturating_sub(now)
                    > self.background_cost_ns.saturating_add(3_000_000)
            })
    }
    pub fn background_submitted(&mut self, now: u64) {
        self.background_due_ns = now.saturating_add(self.refresh_ns.saturating_mul(4));
    }
    pub fn background_completed(&mut self, elapsed: Duration) {
        let cost = elapsed.as_nanos().min(u128::from(u64::MAX)) as u64;
        self.background_cost_ns = cost.max(self.background_cost_ns - self.background_cost_ns / 32);
    }
    /// A successor queued behind an outstanding primary flip targets the following
    /// refresh. Bound prediction after idle/missed refreshes instead of replaying old time.
    pub fn animation_sample_time(&self, now: u64) -> u64 {
        if let Some(frame) = self.rendering {
            return frame.target;
        }
        let period = self.refresh_ns.max(1);
        let next = self
            .last_scanout_ns
            .map_or(now.saturating_add(period), |last| {
                let intervals = now.saturating_sub(last) / period + 1;
                last.saturating_add(intervals.saturating_mul(period))
            });
        if self.primary_pending() {
            next.max(self.pending_target.unwrap_or(next).saturating_add(period))
        } else {
            next
        }
    }

    pub fn pending(&self) -> Option<&C> {
        self.pending.as_ref()
    }
    pub fn primary_pending(&self) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|commit| commit.primary_slot().is_some())
    }
    pub fn ready_len(&self) -> usize {
        self.ready.len()
    }
    pub fn has_ready(&self) -> bool {
        !self.ready.is_empty()
    }
    pub fn render_budget(&self) -> bool {
        self.rendering.is_none() && self.ready.is_empty()
    }
    pub fn plan(&self, work: Work, now_ns: u64) -> Plan {
        let primary_work = work.primary_damage || work.primary_animation;
        // Keep one successor, only for animation, while KMS owns the previous frame.
        // GPU work remains serial and a ready successor prevents any deeper queue.
        let render_candidate = primary_work
            && work.scanout_available
            && self.render_budget()
            && (!self.primary_pending() || work.primary_animation)
            && !self.background_turn(now_ns);
        let cursor_candidate = self.pending.is_none()
            && work.cursor_dirty
            && !primary_work
            && self.rendering.is_none()
            && self.ready.is_empty();
        let render_deadline = render_candidate
            .then(|| self.pacer.deadline(now_ns))
            .flatten();
        // Rendering and cursor sampling have different deadlines. Reserving a refresh
        // immediately after the previous flip samples an almost unchanged pointer;
        // a later menu frame then samples a much larger movement. Use the same late
        // sampling point for cursor-only and combined commits, leaving submission margin.
        let candidate = if self.pending.is_some() {
            None
        } else if self.has_ready() {
            Some(Submission::Primary)
        } else if cursor_candidate {
            Some(Submission::Cursor)
        } else {
            None
        };
        let cursor_deadline = if work.cursor_dirty {
            match candidate {
                Some(Submission::Primary) => self.primary_deadline(now_ns),
                Some(Submission::Cursor) => self.cursor_deadline(now_ns),
                None => None,
            }
        } else {
            None
        };
        Plan {
            render: render_candidate && render_deadline.is_none(),
            force_frame: work.primary_animation,
            submit: candidate.filter(|_| cursor_deadline.is_none()),
            deadline_ns: render_deadline.into_iter().chain(cursor_deadline).min(),
        }
    }
    fn primary_deadline(&self, now: u64) -> Option<u64> {
        self.last_scanout_ns?;
        let deadline = self
            .ready
            .back()?
            .target
            .saturating_sub(3_000_000.min(self.refresh_ns / 2));
        (deadline > now).then_some(deadline)
    }
    pub fn observe_ready_wait(&mut self, now: u64, cursor_dirty: bool) {
        let waiting = if self.pending.is_some() {
            Waiting::Commit
        } else if cursor_dirty {
            self.primary_deadline(now)
                .map_or(Waiting::Dispatch, Waiting::Pacing)
        } else {
            Waiting::Dispatch
        };
        for frame in &mut self.ready {
            frame.advance(now);
            frame.waiting = waiting;
        }
    }
    pub fn ready_wait_sample(&self, now: u64) -> Option<(usize, u64, [u64; 3])> {
        let mut frame = *self.ready.back()?;
        frame.advance(now);
        Some((frame.slot, frame.target, frame.waits))
    }
    fn cursor_deadline(&self, now_ns: u64) -> Option<u64> {
        // Three ms cover timer rounding, owner work and the display latch deadline.
        // Cap the margin for high-refresh displays. Never roll an expired deadline
        // forward: resuming from idle or a missed refresh must remain immediate.
        let margin = 3_000_000.min(self.refresh_ns / 2);
        let deadline = self
            .last_scanout_ns?
            .saturating_add(self.refresh_ns)
            .saturating_sub(margin);
        (deadline > now_ns).then_some(deadline)
    }
    pub fn begin_render(&mut self, slot: usize, now_ns: u64) {
        assert!(
            self.render_budget(),
            "primary render exceeded presentation budget"
        );
        self.rendering = Some(Frame {
            slot,
            target: self.animation_sample_time(now_ns),
            observed: now_ns,
            ready_at: now_ns,
            waiting: Waiting::Dispatch,
            waits: [0; 3],
        });
        self.pacer.begin_render(now_ns);
    }
    pub fn cancel_render(&mut self) {
        self.rendering = None;
        self.pacer.cancel_render();
    }
    pub fn frame_ready(&mut self, slot: usize, now_ns: u64) -> Result<(), &'static str> {
        if self.rendering.as_ref().map(|frame| frame.slot) != Some(slot) {
            return Err("GPU completion did not match the scheduled frame");
        }
        let mut frame = self.rendering.take().unwrap();
        frame.observed = now_ns;
        frame.ready_at = now_ns;
        frame.waiting = if self.pending.is_some() {
            Waiting::Commit
        } else {
            Waiting::Dispatch
        };
        self.pacer.completed(now_ns);
        self.ready.push_back(frame);
        Ok(())
    }
    pub fn take_stale_frame(&mut self) -> Option<usize> {
        (self.ready.len() > 1).then(|| self.ready.pop_front().unwrap().slot)
    }
    pub fn take_ready_frame(&mut self) -> Option<usize> {
        self.ready.pop_back().map(|frame| {
            self.selected_target = Some(frame.target);
            frame.slot
        })
    }
    pub fn submitted(&mut self, commit: C) -> Result<(), &'static str> {
        if self.pending.is_some() {
            return Err("display commit submitted while another commit is pending");
        }
        self.pending_target = if commit.primary_slot().is_some() {
            self.selected_target.take()
        } else {
            None
        };
        self.pending = Some(commit);
        Ok(())
    }
    #[cfg(test)]
    pub fn completed(&mut self, now_ns: u64) -> Option<C> {
        self.completed_at(now_ns, now_ns)
    }
    pub fn completed_at(&mut self, now_ns: u64, observed_ns: u64) -> Option<C> {
        let commit = self.pending.take()?;
        self.pending_target = None;
        for frame in &mut self.ready {
            frame.advance(observed_ns);
            // Completion may have happened before the owner dispatched its event.
            // Reclassify that interval instead of blaming the display commit for it.
            let dispatch = observed_ns
                .saturating_sub(now_ns.max(frame.ready_at))
                .min(frame.waits[0]);
            frame.waits[0] -= dispatch;
            frame.waits[2] += dispatch;
            frame.waiting = Waiting::Dispatch;
        }
        self.last_scanout_ns = Some(now_ns);
        if commit.primary_slot().is_some() {
            self.pacer.presented(now_ns);
        }
        Some(commit)
    }
    pub fn modeset_completed(&mut self, now_ns: u64) {
        self.selected_target = None;
        self.last_scanout_ns = Some(now_ns);
        self.pacer.presented(now_ns);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const MS: u64 = 1_000_000;
    impl Commit for Option<usize> {
        fn primary_slot(&self) -> Option<usize> {
            *self
        }
    }
    fn scheduler() -> Scheduler<Option<usize>> {
        Scheduler::new(Duration::from_millis(16))
    }
    fn damage() -> Work {
        Work {
            primary_damage: true,
            cursor_dirty: true,
            scanout_available: true,
            ..Work::default()
        }
    }
    fn pointer() -> Work {
        Work {
            cursor_dirty: true,
            scanout_available: true,
            ..Work::default()
        }
    }
    #[test]
    fn delayed_flip_dispatch_cannot_move_a_ready_frames_target() {
        let mut s = scheduler();
        s.modeset_completed(0);
        s.begin_render(0, 4 * MS);
        s.frame_ready(0, 6 * MS).unwrap();
        s.take_ready_frame();
        s.submitted(Some(0)).unwrap();
        s.begin_render(1, 8 * MS);
        assert_eq!(s.animation_sample_time(12 * MS), 32 * MS);
        s.frame_ready(1, 10 * MS).unwrap();
        s.completed_at(16 * MS, 20 * MS).unwrap();
        assert_eq!(s.plan(damage(), 22 * MS).deadline_ns, Some(29 * MS));
        s.observe_ready_wait(22 * MS, true);
        assert_eq!(s.plan(damage(), 29 * MS).submit, Some(Submission::Primary));
        s.observe_ready_wait(30 * MS, true);
        let (_, target, waits) = s.ready_wait_sample(30 * MS).unwrap();
        assert_eq!(target, 32 * MS);
        assert_eq!(waits, [6 * MS, 7 * MS, 7 * MS]);
        assert_eq!(waits.into_iter().sum::<u64>(), 20 * MS);
        assert_eq!(s.plan(damage(), 40 * MS).submit, Some(Submission::Primary));
    }

    #[test]
    fn drm_timestamp_conversion_rejects_incompatible_or_invalid_clocks() {
        assert_eq!(flip_time(10_000, Some(12_000), 5 * MS), Some(3 * MS));
        assert_eq!(flip_time(13_000, Some(12_000), 5 * MS), None);
        assert_eq!(flip_time(1, Some(12_000), 5 * MS), None);
        assert_eq!(flip_time(0, Some(12_000), 5 * MS), None);
        assert_eq!(flip_time(10_000, None, 5 * MS), None);
    }

    #[test]
    fn background_admission_reserves_display_work_and_bounds_deferral() {
        let mut s = scheduler();
        s.modeset_completed(0);
        assert!(!s.background_allowed(0, true)); // prepare display first
        s.begin_render(0, 0);
        assert!(!s.background_allowed(0, false)); // no background queue behind rendering
        s.frame_ready(0, 2 * MS).unwrap();
        assert!(s.background_allowed(2 * MS, true));
        s.background_submitted(2 * MS);
        s.background_completed(Duration::from_millis(20));
        assert!(!s.background_allowed(3 * MS, true));
        assert!(!s.background_allowed(65 * MS, true));
        assert!(s.background_allowed(66 * MS, true)); // bounded progress under animation
        assert!(s.background_allowed(3 * MS, false)); // idle display does not throttle capture
        s.take_ready_frame();
        assert!(!s.background_allowed(66 * MS, true)); // a due capture cannot jump fresh display work
    }
    #[test]
    fn overdue_capture_gets_a_turn_before_the_next_animation_successor() {
        let mut s = scheduler();
        let animation = Work {
            primary_animation: true,
            ..damage()
        };
        s.request_background(true);
        assert!(s.plan(animation, 0).render); // first display frame has priority
        s.begin_render(0, 0);
        s.frame_ready(0, MS).unwrap();
        s.take_ready_frame();
        s.submitted(Some(0)).unwrap();
        assert!(!s.plan(animation, 2 * MS).render);
        assert!(s.background_allowed(2 * MS, true));
        s.background_submitted(2 * MS);
        assert!(s.plan(animation, 3 * MS).render);
        s.request_background(false);
        assert!(s.plan(animation, 100 * MS).render); // no artificial pauses without demand
    }

    #[test]
    fn animation_sampling_targets_successor_refresh_and_recovers_from_idle() {
        let mut s = scheduler();
        s.modeset_completed(0);
        assert_eq!(s.animation_sample_time(4 * MS), 16 * MS);
        s.submitted(Some(0)).unwrap();
        assert_eq!(s.animation_sample_time(4 * MS), 32 * MS);
        s.completed(16 * MS);
        assert_eq!(s.animation_sample_time(20 * MS), 32 * MS);
        assert_eq!(s.animation_sample_time(1000 * MS), 1008 * MS);
    }

    #[test]
    fn cursor_waits_for_render_and_combines_with_ready_frame() {
        let mut s = scheduler();
        assert!(s.plan(damage(), 0).render);
        assert_eq!(s.plan(damage(), 0).submit, None);
        s.begin_render(0, 0);
        assert_eq!(s.plan(pointer(), MS).submit, None);
        s.frame_ready(0, 2 * MS).unwrap();
        assert_eq!(s.plan(damage(), 2 * MS).submit, Some(Submission::Primary));
        assert_eq!(s.take_ready_frame(), Some(0));
        s.submitted(Some(0)).unwrap();
        assert!(!s.plan(damage(), 3 * MS).render);
        assert_eq!(s.plan(pointer(), 3 * MS).submit, None);
        assert_eq!(s.completed(16 * MS), Some(Some(0)));
        assert!(s.render_budget());
    }
    #[test]
    fn animation_overlaps_pending_flip_with_exactly_one_successor() {
        let mut s = scheduler();
        let animation = Work {
            primary_animation: true,
            cursor_dirty: false,
            ..damage()
        };
        s.begin_render(0, 0);
        s.frame_ready(0, 2 * MS).unwrap();
        assert_eq!(s.take_ready_frame(), Some(0));
        s.submitted(Some(0)).unwrap();
        assert!(s.plan(animation, 3 * MS).render);
        s.begin_render(1, 3 * MS);
        assert!(!s.plan(animation, 4 * MS).render);
        s.frame_ready(1, 10 * MS).unwrap();
        let waiting = s.plan(animation, 11 * MS);
        assert!(!waiting.render);
        assert_eq!(waiting.submit, None);
        assert_eq!(waiting.wait(false, None, 11 * MS), None);
        assert_eq!(s.completed(16 * MS), Some(Some(0)));
        assert_eq!(s.plan(animation, 16 * MS).submit, Some(Submission::Primary));
        assert_eq!(s.take_ready_frame(), Some(1));
        s.submitted(Some(1)).unwrap();
        assert!(!s.plan(damage(), 20 * MS).render);
        assert_eq!(s.completed(32 * MS), Some(Some(1)));
    }

    #[test]
    fn pending_flip_completion_does_not_release_a_successor_gpu_slot() {
        let mut s = scheduler();
        s.begin_render(0, 0);
        s.frame_ready(0, MS).unwrap();
        s.take_ready_frame();
        s.submitted(Some(0)).unwrap();
        s.begin_render(1, 2 * MS);
        assert_eq!(s.completed(16 * MS), Some(Some(0)));
        assert!(!s.render_budget());
        assert!(s.frame_ready(0, 17 * MS).is_err());
        s.frame_ready(1, 18 * MS).unwrap();
        assert_eq!(s.take_ready_frame(), Some(1));
        assert!(s.render_budget());
    }

    #[test]
    fn cursor_in_flight_does_not_stop_frame_preparation_or_input() {
        let mut s = scheduler();
        s.submitted(None).unwrap();
        assert!(s.plan(damage(), 0).render);
        s.begin_render(1, 0);
        s.frame_ready(1, MS).unwrap();
        let plan = s.plan(damage(), MS);
        assert_eq!(plan.submit, None);
        assert_eq!(plan.wait(true, None, MS), Some(Duration::ZERO));
        assert_eq!(plan.wait(false, None, MS), None); // sleep on completion FD, no spin
        s.completed(16 * MS).unwrap();
        // This frame targeted the just-completed refresh. Do not postpone it again
        // to sample the cursor for the following refresh.
        assert_eq!(s.plan(damage(), 16 * MS).submit, Some(Submission::Primary));
        assert_eq!(s.plan(damage(), 29 * MS).submit, Some(Submission::Primary));
    }
    #[test]
    fn input_wakes_during_pacing_and_repeated_motion_does_not_extend_deadline() {
        let mut s = scheduler();
        s.modeset_completed(16 * MS);
        let deadline = s.plan(pointer(), 16 * MS).deadline_ns.unwrap();
        for now in [16 * MS, 17 * MS, 18 * MS] {
            let plan = s.plan(pointer(), now);
            assert_eq!(plan.deadline_ns, Some(deadline));
            assert_eq!(plan.wait(true, None, now), Some(Duration::ZERO));
            assert_eq!(
                plan.wait(false, None, now),
                Some(Duration::from_nanos(deadline - now))
            );
        }
        assert_eq!(s.plan(pointer(), deadline).submit, Some(Submission::Cursor));
    }
    #[test]
    fn cancellation_releases_cursor_and_bad_completions_preserve_ownership() {
        let mut s = scheduler();
        s.begin_render(2, 0);
        assert!(s.frame_ready(3, MS).is_err());
        assert!(!s.render_budget());
        s.cancel_render();
        assert_eq!(s.plan(pointer(), MS).submit, Some(Submission::Cursor));
        s.submitted(None).unwrap();
        assert!(s.submitted(Some(2)).is_err());
        assert_eq!(s.completed(2 * MS), Some(None));
        assert_eq!(s.completed(3 * MS), None);
    }
    #[test]
    fn animation_reserves_primary_but_idle_cursor_can_progress() {
        let s = scheduler();
        let work = Work {
            primary_animation: true,
            ..pointer()
        };
        assert_eq!(s.plan(work, 0).submit, None);
        assert!(s.plan(work, 0).render);
        assert_eq!(s.plan(pointer(), 0).submit, Some(Submission::Cursor));
    }
    #[test]
    fn moving_cursor_is_sampled_at_the_same_refresh_phase_with_and_without_menu_frames() {
        let mut s = scheduler();
        s.modeset_completed(0);
        // A finished menu frame must not freeze a cursor position early in the refresh.
        s.begin_render(0, 4 * MS);
        s.frame_ready(0, 6 * MS).unwrap();
        for now in [6 * MS, 8 * MS, 12 * MS] {
            let plan = s.plan(damage(), now);
            assert_eq!(plan.submit, None);
            assert_eq!(plan.deadline_ns, Some(13 * MS));
            assert_eq!(plan.wait(true, None, now), Some(Duration::ZERO));
        }
        assert_eq!(s.plan(damage(), 13 * MS).submit, Some(Submission::Primary));
        s.take_ready_frame();
        s.submitted(Some(0)).unwrap();
        s.completed(16 * MS).unwrap();
        assert_eq!(s.plan(pointer(), 16 * MS).deadline_ns, Some(29 * MS));
        assert_eq!(s.plan(pointer(), 29 * MS).submit, Some(Submission::Cursor));
        s.submitted(None).unwrap();
        s.completed(32 * MS).unwrap();
        // Cursor-only flips also advance the clock, unlike the render cost estimator.
        assert_eq!(s.plan(pointer(), 32 * MS).deadline_ns, Some(45 * MS));
        assert_eq!(s.plan(pointer(), 45 * MS).submit, Some(Submission::Cursor));
    }

    #[test]
    fn stationary_cursor_does_not_delay_desktop_and_idle_motion_does_not_wait() {
        let mut s = scheduler();
        s.modeset_completed(0);
        s.begin_render(0, 4 * MS);
        s.frame_ready(0, 6 * MS).unwrap();
        let work = Work {
            cursor_dirty: false,
            ..damage()
        };
        assert_eq!(s.plan(work, 6 * MS).submit, Some(Submission::Primary));
        s.take_ready_frame();
        assert_eq!(
            s.plan(pointer(), 1000 * MS).submit,
            Some(Submission::Cursor)
        );
    }
}

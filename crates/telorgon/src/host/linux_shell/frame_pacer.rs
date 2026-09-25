//! Owner-observed refresh pacing. Delays render preparation, never input/protocol dispatch.
//! GPU completion observations include scheduling delay, making the estimate conservative.
pub(super) struct FramePacer {
    period_ns: u64,
    budget_ns: u64,
    last_flip_ns: Option<u64>,
    render_start_ns: Option<u64>,
    recent_costs_ns: [u64; 32],
    next_cost: usize,
}

impl FramePacer {
    pub(super) fn new(period: std::time::Duration) -> Self {
        let period_ns = period.as_nanos().min(u128::from(u64::MAX)) as u64;
        let period_ns = period_ns.max(1);
        Self {
            period_ns,
            budget_ns: period_ns - period_ns / 4,
            last_flip_ns: None,
            render_start_ns: None,
            recent_costs_ns: [0; 32],
            next_cost: 0,
        }
    }

    pub(super) fn presented(&mut self, now_ns: u64) {
        self.last_flip_ns = Some(now_ns);
    }

    /// Do not roll an expired deadline forward: idle/recovering outputs render immediately.
    /// Keep a fixed deadline through input wakeups so high-rate input cannot postpone rendering.
    pub(super) fn deadline(&self, now_ns: u64) -> Option<u64> {
        let deadline = self
            .last_flip_ns?
            .saturating_add(self.period_ns)
            .saturating_sub(self.budget_ns);
        (deadline > now_ns).then_some(deadline)
    }

    pub(super) fn begin_render(&mut self, now_ns: u64) {
        self.render_start_ns = Some(now_ns);
    }

    pub(super) fn cancel_render(&mut self) {
        self.render_start_ns = None;
    }

    pub(super) fn completed(&mut self, now_ns: u64) {
        let Some(start) = self.render_start_ns.take() else {
            return;
        };
        let sample = now_ns.saturating_sub(start);
        self.recent_costs_ns[self.next_cost] = sample.min(self.period_ns);
        self.next_cost = (self.next_cost + 1) % self.recent_costs_ns.len();
        // Large client frames vary substantially in cost. Preserve the recent high-water
        // mark for 32 completed frames, then decay gradually when that load has passed.
        // Reserve at least half a refresh; three ms cover millisecond timer rounding,
        // submission and the display's latch deadline before the observed page flip.
        let recent_peak = self.recent_costs_ns.iter().copied().max().unwrap_or(0);
        let target = recent_peak.saturating_add(3_000_000)
            .max(self.period_ns.div_ceil(2)).min(self.period_ns);
        self.budget_ns = target.max(self.budget_ns - self.budget_ns / 64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const MS: u64 = 1_000_000;

    #[test]
    fn incoming_client_buffers_fit_before_the_same_refresh_without_moving_the_deadline() {
        let mut pacer = FramePacer::new(std::time::Duration::from_millis(16));
        assert_eq!(pacer.deadline(0), None); // first frame never waits
        pacer.presented(16 * MS);
        // Callback at refresh, client replies two ms later. Render four ms after refresh,
        // instead of locking in old pixels before that reply. Repeated input cannot defer it.
        assert_eq!(pacer.deadline(16 * MS), Some(20 * MS));
        assert_eq!(pacer.deadline(18 * MS), Some(20 * MS));
        assert_eq!(pacer.deadline(20 * MS), None);
        let render_at = pacer.deadline(18 * MS).unwrap();
        assert!(18 * MS <= render_at); // includes the new client buffer
        let completed_at = render_at + 5 * MS;
        assert!(completed_at < 32 * MS); // ready before the same refresh
        pacer.begin_render(render_at);
        pacer.completed(completed_at);
        assert_eq!(pacer.deadline(200 * MS), None); // idle output remains responsive
    }

    #[test]
    fn intermittent_heavy_frames_keep_room_for_the_next_gpu_spike() {
        let mut pacer = FramePacer::new(std::time::Duration::from_millis(16));
        pacer.begin_render(0);
        pacer.completed(11 * MS);
        for frame in 1..=24 {
            let flip = frame * 16 * MS;
            pacer.presented(flip);
            // A one-ms wakeup overshoot and an 11-ms frame still fit this refresh.
            let render = pacer.deadline(flip).unwrap_or(flip) + MS;
            assert!(render + 11 * MS < flip + 16 * MS);
            pacer.begin_render(render);
            pacer.completed(render + 5 * MS);
        }
        // Sustained lighter work eventually releases the old peak, without allowing
        // rendering to drift into the final half of a refresh again.
        for frame in 25..=400 {
            let flip = frame * 16 * MS;
            pacer.presented(flip);
            let render = pacer.deadline(flip).unwrap_or(flip);
            pacer.begin_render(render);
            pacer.completed(render + 4 * MS);
        }
        assert_eq!(pacer.budget_ns, 8 * MS);
    }

    #[test]
    fn expensive_frames_remove_deferral_and_idle_time_does_not_pollute_gpu_budget() {
        let mut pacer = FramePacer::new(std::time::Duration::from_millis(16));
        pacer.presented(0);
        pacer.begin_render(4 * MS);
        pacer.completed(24 * MS);
        pacer.presented(32 * MS);
        assert_eq!(pacer.deadline(32 * MS), None);
        pacer.begin_render(32 * MS);
        pacer.cancel_render(); // scene unchanged: no GPU completion expected
        pacer.completed(900 * MS);
        assert_eq!(pacer.budget_ns, 16 * MS);
        pacer.begin_render(1000 * MS);
        pacer.completed(1001 * MS);
        assert!(pacer.budget_ns > 15 * MS); // recover gradually, without oscillation
    }
}

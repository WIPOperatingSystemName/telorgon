//! Owner-observed transition timing. These are dispatch times, not GPU or display timestamps.
use super::motion::WindowState;
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

#[path = "gpu_correlation.rs"]
mod gpu_correlation;

struct Transition {
    generation: u64,
    kind: &'static str,
    started: Instant,
    last_flip: Option<Instant>,
    frames: u64,
    gaps: u64,
    details: u64,
    max_gap: Duration,
    first_flip: Option<Duration>,
    // Preparation, render call, completion dispatch, ready queue, KMS completion,
    // render return to worker completion, and worker completion to owner dispatch.
    maxima: [Duration; 7],
}
impl Transition {
    fn report(&self, window: u32, reason: &str) {
        eprintln!(
            "telorgon-transition-summary: window={window} id={} kind={} reason={reason} elapsed_ms={:.3} frames={} late_gaps={} suppressed={} first_flip_ms={:?} max_gap_ms={:.3} prepare_max_ms={:.3} render_call_max_ms={:.3} render_return_to_ready_max_ms={:.3} ready_to_submit_max_ms={:.3} submit_to_flip_max_ms={:.3} render_return_to_worker_max_ms={:.3} worker_to_owner_max_ms={:.3}",
            self.generation,
            self.kind,
            ms(self.started.elapsed()),
            self.frames,
            self.gaps,
            self.gaps.saturating_sub(self.details),
            self.first_flip.map(ms),
            ms(self.max_gap),
            ms(self.maxima[0]),
            ms(self.maxima[1]),
            ms(self.maxima[2]),
            ms(self.maxima[3]),
            ms(self.maxima[4]),
            ms(self.maxima[5]),
            ms(self.maxima[6])
        );
    }
}
struct Frame {
    correlation: gpu_correlation::Details,
    windows: Vec<(u32, u64, bool)>,
    started: Instant,
    prepared: Instant,
    rendered: Option<Instant>,
    ready: Option<Instant>,
    worker_completed: Option<Instant>,
    submitted: Option<Instant>,
}
pub(super) struct Probe {
    enabled: bool,
    refresh: Duration,
    previous: BTreeMap<u32, (bool, bool)>,
    transitions: BTreeMap<u32, Transition>,
    frames: BTreeMap<usize, Frame>,
    generation: u64,
    gpu_history: std::collections::VecDeque<gpu_correlation::Completed>,
}
fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}
impl Probe {
    pub(super) fn new(refresh: Duration) -> Self {
        Self {
            enabled: super::stall_probe::enabled(),
            refresh,
            previous: BTreeMap::new(),
            transitions: BTreeMap::new(),
            frames: BTreeMap::new(),
            generation: 0,
            gpu_history: Default::default(),
        }
    }
    pub(super) fn observe_states(&mut self, states: &BTreeMap<u32, WindowState>) {
        if !self.enabled {
            return;
        }
        self.previous.retain(|id, _| states.contains_key(id));
        self.transitions.retain(|id, transition| {
            if states.contains_key(id) {
                true
            } else {
                transition.report(*id, "withdrawn");
                false
            }
        });
        for (&id, state) in states {
            let current = (state.maximized, state.minimized);
            let Some(previous) = self.previous.insert(id, current) else {
                continue;
            };
            let kind = if previous.1 != current.1 {
                if current.1 { "minimize" } else { "unminimize" }
            } else if previous.0 != current.0 {
                if current.0 { "maximize" } else { "unmaximize" }
            } else {
                continue;
            };
            if let Some(old) = self.transitions.remove(&id) {
                old.report(id, "retargeted");
            }
            self.generation = self.generation.wrapping_add(1);
            self.transitions.insert(
                id,
                Transition {
                    generation: self.generation,
                    kind,
                    started: Instant::now(),
                    last_flip: None,
                    frames: 0,
                    gaps: 0,
                    details: 0,
                    max_gap: Duration::ZERO,
                    first_flip: None,
                    maxima: [Duration::ZERO; 7],
                },
            );
            eprintln!(
                "telorgon-transition: window={id} id={} kind={kind} event=state-observed refresh_ms={:.3} bounds={:?} veiled={} clock=owner-observed",
                self.generation,
                ms(self.refresh),
                state.bounds,
                state.veiled
            );
        }
    }
    pub(super) fn prepared(
        &mut self,
        slot: usize,
        shell_frame: u64,
        started: Option<Instant>,
        pending: impl Fn(u32) -> bool,
    ) {
        if !self.enabled {
            return;
        }
        self.frames.remove(&slot);
        if self.transitions.is_empty() {
            return;
        }
        let now = Instant::now();
        self.frames.insert(
            slot,
            Frame {
                correlation: gpu_correlation::Details {
                    shell_frame,
                    ..Default::default()
                },
                windows: self
                    .transitions
                    .iter()
                    .map(|(&id, t)| (id, t.generation, !pending(id)))
                    .collect(),
                started: started.unwrap_or(now),
                prepared: now,
                rendered: None,
                ready: None,
                worker_completed: None,
                submitted: None,
            },
        );
    }
    pub(super) fn rendered(&mut self, slot: usize) {
        if let Some(frame) = self.frames.get_mut(&slot) {
            frame.rendered = Some(Instant::now());
        }
    }
    pub(super) fn ready(&mut self, slot: usize) {
        if let Some(frame) = self.frames.get_mut(&slot) {
            frame.ready = Some(Instant::now());
        }
    }
    pub(super) fn gpu_ready(&mut self, slot: usize, completed_at: Instant) {
        self.ready(slot);
        if let Some(frame) = self.frames.get_mut(&slot) {
            frame.worker_completed = Some(completed_at);
        }
        super::stall_probe::observe("gpu_completion_dispatch", completed_at.elapsed(), || {
            format!("slot={slot}")
        });
    }
    pub(super) fn gpu_submission(&mut self, completion: &super::renderer::VulkanCompletion) {
        // Timeouts may return the same capture receipt for retry; they are not
        // completed GPU work and must not enter the neighboring-submission history.
        if !self.enabled || completion.result.is_err() {
            return;
        }
        let sample = gpu_correlation::Completed {
            capture: completion.capture.is_some(),
            timing: completion.timing.clone(),
            done: completion.completed_at,
        };
        if !sample.capture
            && let Some(frame) = self.frames.get_mut(&completion.slot_index)
        {
            frame.correlation.submission = Some(sample.timing.clone());
        }
        gpu_correlation::retain(&mut self.gpu_history, sample);
    }
    pub(super) fn presentation_wait(&mut self, slot: usize, target: u64, waits: [u64; 3]) {
        if let Some(frame) = self.frames.get_mut(&slot) {
            frame.correlation.target_ns = Some(target);
            frame.correlation.waits = waits;
        }
    }
    pub(super) fn flip(&mut self, slot: usize, sequence: u32, hardware_ns: Option<u64>) {
        if let Some(frame) = self.frames.get_mut(&slot) {
            frame.correlation.flip = Some((sequence, hardware_ns));
        }
        self.completed(slot);
    }
    pub(super) fn submitted(&mut self, slot: usize) {
        if let Some(frame) = self.frames.get_mut(&slot) {
            frame.submitted = Some(Instant::now());
        }
    }
    pub(super) fn completed(&mut self, slot: usize) {
        self.completed_at(slot, Instant::now());
    }
    fn completed_at(&mut self, slot: usize, now: Instant) {
        let Some(frame) = self.frames.remove(&slot) else {
            return;
        };
        let (Some(rendered), Some(ready), Some(submitted)) =
            (frame.rendered, frame.ready, frame.submitted)
        else {
            return;
        };
        let stages = [
            frame.prepared.saturating_duration_since(frame.started),
            rendered.saturating_duration_since(frame.prepared),
            ready.saturating_duration_since(rendered),
            submitted.saturating_duration_since(ready),
            now.saturating_duration_since(submitted),
            frame.worker_completed.map_or(Duration::ZERO, |done| {
                done.saturating_duration_since(rendered)
            }),
            frame
                .worker_completed
                .map_or(Duration::ZERO, |done| ready.saturating_duration_since(done)),
        ];
        for &(id, generation, settled) in &frame.windows {
            let Some(t) = self
                .transitions
                .get_mut(&id)
                .filter(|t| t.generation == generation)
            else {
                continue;
            };
            // The first interval is state-observation-to-first-flip, not a frame gap.
            if let Some(last) = t.last_flip.replace(now) {
                let gap = now.saturating_duration_since(last);
                t.max_gap = t.max_gap.max(gap);
                if gap > self.refresh + self.refresh / 2 {
                    t.gaps += 1;
                    if t.details < 4 {
                        t.details += 1;
                        gpu_correlation::report(
                            &frame,
                            id,
                            generation,
                            gap,
                            now,
                            &self.gpu_history,
                        );
                        eprintln!(
                            "telorgon-transition: window={id} id={generation} kind={} event=late-frame gap_ms={:.3} refresh_ms={:.3} prepare_ms={:.3} render_call_ms={:.3} render_return_to_ready_ms={:.3} ready_to_submit_ms={:.3} submit_to_flip_ms={:.3} render_return_to_worker_ms={:.3} worker_to_owner_ms={:.3}",
                            t.kind,
                            ms(gap),
                            ms(self.refresh),
                            ms(stages[0]),
                            ms(stages[1]),
                            ms(stages[2]),
                            ms(stages[3]),
                            ms(stages[4]),
                            ms(stages[5]),
                            ms(stages[6])
                        );
                    }
                }
            } else {
                t.first_flip = Some(now.saturating_duration_since(t.started));
            }
            t.frames += 1;
            for (max, sample) in t.maxima.iter_mut().zip(stages) {
                *max = (*max).max(sample);
            }
            if settled {
                self.transitions
                    .remove(&id)
                    .unwrap()
                    .report(id, "settled-flip");
            }
        }
    }
}
impl Drop for Probe {
    fn drop(&mut self) {
        for (&id, transition) in &self.transitions {
            transition.report(id, "session-exit");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tracks_state_changes_but_not_initial_windows_and_retires_withdrawals() {
        let state = WindowState {
            bounds: crate::foundation::RectI {
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            },
            maximized: false,
            minimized: false,
            tiled: None,
            interactive: false,
            move_pointer: None,
            veiled: false,
            style: crate::WindowMotion::smooth(),
            client_decorated: false,
            corner_radii: Default::default(),
            shadows: Default::default(),
        };
        let mut states = BTreeMap::from([(1, state)]);
        let mut probe = Probe::new(Duration::from_millis(10));
        probe.enabled = true;
        probe.observe_states(&states);
        assert!(probe.transitions.is_empty());
        for (maximized, minimized, kind) in [
            (true, false, "maximize"),
            (true, true, "minimize"),
            (true, false, "unminimize"),
            (false, false, "unmaximize"),
        ] {
            states.get_mut(&1).unwrap().maximized = maximized;
            states.get_mut(&1).unwrap().minimized = minimized;
            probe.observe_states(&states);
            assert_eq!(probe.transitions[&1].kind, kind);
            let generation = probe.transitions[&1].generation;
            probe.observe_states(&states);
            assert_eq!(probe.transitions[&1].generation, generation);
        }
        probe.observe_states(&BTreeMap::new());
        assert!(probe.transitions.is_empty());
        assert!(probe.previous.is_empty());
    }

    #[test]
    fn gaps_follow_presented_frames_and_ignore_superseded_generations() {
        let start = Instant::now();
        let mut probe = Probe::new(Duration::from_millis(10));
        probe.transitions.insert(
            1,
            Transition {
                generation: 2,
                kind: "maximize",
                started: start,
                last_flip: None,
                frames: 0,
                gaps: 0,
                details: 0,
                max_gap: Duration::ZERO,
                first_flip: None,
                maxima: [Duration::ZERO; 7],
            },
        );
        for (generation, offset, settled) in
            [(1, 5, true), (2, 10, false), (2, 20, false), (2, 50, false)]
        {
            probe.frames.insert(
                0,
                Frame {
                    correlation: Default::default(),
                    windows: vec![(1, generation, settled)],
                    started: start,
                    prepared: start,
                    rendered: Some(start + Duration::from_millis(1)),
                    ready: Some(start + Duration::from_millis(4)),
                    worker_completed: Some(start + Duration::from_millis(3)),
                    submitted: Some(start + Duration::from_millis(4)),
                },
            );
            probe.completed_at(0, start + Duration::from_millis(offset));
        }
        let t = &probe.transitions[&1];
        assert_eq!(t.maxima[5], Duration::from_millis(2));
        assert_eq!(t.maxima[6], Duration::from_millis(1));
        assert_eq!(t.frames, 3);
        assert_eq!(t.gaps, 1);
        assert_eq!(t.first_flip, Some(Duration::from_millis(10)));
        assert_eq!(t.max_gap, Duration::from_millis(30));
        probe.frames.insert(
            0,
            Frame {
                correlation: Default::default(),
                windows: vec![(1, 2, true)],
                started: start,
                prepared: start,
                rendered: Some(start),
                ready: Some(start),
                worker_completed: None,
                submitted: Some(start),
            },
        );
        probe.completed_at(0, start + Duration::from_millis(60));
        assert!(probe.transitions.is_empty());
    }
}

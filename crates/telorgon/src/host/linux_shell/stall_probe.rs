//! Bounded stall details plus lifetime totals, independent of the rolling flight recorder.
use std::{
    cell::RefCell,
    collections::BTreeMap,
    time::{Duration, Instant},
};

const SLOW: Duration = Duration::from_millis(8);
const WINDOW: Duration = Duration::from_secs(2);
const DETAIL_LIMIT: u32 = 4;

#[derive(Default)]
struct Metric {
    count: u64,
    slow: u64,
    total: Duration,
    max: Duration,
    worst: String,
    logged: u32,
    suppressed: u64,
}
impl Metric {
    fn add(&mut self, duration: Duration) -> (bool, bool) {
        self.count += 1;
        self.total += duration;
        let worst = duration > self.max;
        self.max = self.max.max(duration);
        let log = duration >= SLOW && self.logged < DETAIL_LIMIT;
        if duration >= SLOW {
            self.slow += 1;
            if log {
                self.logged += 1;
            } else {
                self.suppressed += 1;
            }
        }
        (worst, log)
    }
}
struct Commit {
    submitted: Instant,
    primary: bool,
    blocked_since: Option<Instant>,
    cursor_input: Option<Instant>,
}
struct State {
    start: Instant,
    window: Instant,
    report: Instant,
    metrics: BTreeMap<&'static str, Metric>,
    commit: Option<Commit>,
    cursor_input: Option<Instant>,
}
impl State {
    fn summary(&self, reason: &str) {
        for (stage, m) in &self.metrics {
            eprintln!(
                "telorgon-stall-summary: reason={reason} elapsed_ms={:.3} stage={stage} count={} slow={} suppressed={} total_ms={:.3} max_ms={:.3} worst=[{}]",
                self.start.elapsed().as_secs_f64() * 1000.0,
                m.count,
                m.slow,
                m.suppressed,
                m.total.as_secs_f64() * 1000.0,
                m.max.as_secs_f64() * 1000.0,
                m.worst
            );
        }
    }
}
thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

pub(super) struct Session;
impl Session {
    pub(super) fn from_env() -> Self {
        if std::env::var("TELORGON_STALL_PROBES").as_deref() == Ok("1") {
            let now = Instant::now();
            STATE.with(|state| {
                *state.borrow_mut() = Some(State {
                    start: now,
                    window: now,
                    report: now,
                    metrics: BTreeMap::new(),
                    commit: None,
                    cursor_input: None,
                })
            });
            eprintln!(
                "telorgon-stall: enabled threshold_ms=8 detail_limit=4/stage/2s summaries=10s-and-exit clock=owner-observed; commit completion is observed dispatch, not a hardware timestamp"
            );
        }
        Self
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        STATE.with(|state| {
            if let Some(state) = state.borrow_mut().take() {
                state.summary("exit");
                if let Some(commit) = state.commit {
                    eprintln!(
                        "telorgon-stall: pending-at-exit primary={} age_ms={:.3}",
                        commit.primary,
                        commit.submitted.elapsed().as_secs_f64() * 1000.0
                    );
                }
            }
        });
    }
}
pub(super) fn enabled() -> bool {
    STATE.with(|state| state.borrow().is_some())
}
pub(super) fn begin() -> Option<Instant> {
    enabled().then(Instant::now)
}
pub(super) fn finish(
    stage: &'static str,
    start: Option<Instant>,
    context: impl FnOnce() -> String,
) {
    if let Some(start) = start {
        observe(stage, start.elapsed(), context);
    }
}
pub(super) fn observe(stage: &'static str, duration: Duration, context: impl FnOnce() -> String) {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let Some(state) = state.as_mut() else {
            return;
        };
        let now = Instant::now();
        if now.duration_since(state.window) >= WINDOW {
            for metric in state.metrics.values_mut() {
                metric.logged = 0;
            }
            state.window = now;
        }
        let metric = state.metrics.entry(stage).or_default();
        let (worst, log) = metric.add(duration);
        if worst || log {
            let context = context();
            if worst {
                metric.worst = context.clone();
            }
            if log {
                eprintln!(
                    "telorgon-stall: elapsed_ms={:.3} stage={stage} duration_ms={:.3} {context}",
                    now.duration_since(state.start).as_secs_f64() * 1000.0,
                    duration.as_secs_f64() * 1000.0
                );
            }
        }
        if now.duration_since(state.report) >= Duration::from_secs(10) {
            state.summary("periodic");
            state.report = now;
        }
    });
}
/// Timestamp the latest dispatched motion separately from commit duration. This
/// measures owner-observed cursor latency; it is not an input-to-photon timestamp.
pub(super) fn cursor_moved() {
    STATE.with(|state| {
        if let Some(state) = state.borrow_mut().as_mut() {
            state.cursor_input = Some(Instant::now());
        }
    });
}
pub(super) fn submitted(primary: bool, cursor_updated: bool) {
    let sample = STATE.with(|state| {
        let mut state = state.borrow_mut();
        let state = state.as_mut()?;
        let now = Instant::now();
        let input = if cursor_updated {
            state.cursor_input.take()
        } else {
            None
        };
        state.commit = Some(Commit {
            submitted: now,
            primary,
            blocked_since: None,
            cursor_input: input,
        });
        input.map(|input| now.duration_since(input))
    });
    if let Some(duration) = sample {
        observe("cursor_dispatch_to_submit", duration, || {
            format!("primary={primary}")
        });
    }
}
pub(super) fn blocked(ready: bool) {
    if ready {
        STATE.with(|state| {
            if let Some(state) = state.borrow_mut().as_mut()
                && let Some(commit) = state.commit.as_mut()
            {
                commit.blocked_since.get_or_insert_with(Instant::now);
            }
        });
    }
}
pub(super) fn completed() {
    let sample = STATE.with(|state| {
        let mut state = state.borrow_mut();
        let state = state.as_mut()?;
        let commit = state.commit.take()?;
        Some((state.start, commit))
    });
    if let Some((start, commit)) = sample {
        let now = Instant::now();
        if let Some(input) = commit.cursor_input {
            observe("cursor_dispatch_to_flip", now.duration_since(input), || {
                format!("primary={}", commit.primary)
            });
        }
        let stage = if commit.primary {
            "primary_commit"
        } else {
            "cursor_commit"
        };
        let context = || {
            format!(
                "submitted_ms={:.3} completed_ms={:.3} ready_frame_blocked_ms={:.3}",
                commit.submitted.duration_since(start).as_secs_f64() * 1000.0,
                now.duration_since(start).as_secs_f64() * 1000.0,
                commit
                    .blocked_since
                    .map_or(0.0, |since| now.duration_since(since).as_secs_f64()
                        * 1000.0)
            )
        };
        observe(stage, now.duration_since(commit.submitted), context);
        if let Some(since) = commit.blocked_since {
            observe(
                if commit.primary {
                    "frame_blocked_by_primary"
                } else {
                    "frame_blocked_by_cursor"
                },
                now.duration_since(since),
                context,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cursor_block_is_recorded_once_at_completion() {
        let now = Instant::now();
        STATE.with(|state| {
            *state.borrow_mut() = Some(State {
                start: now,
                window: now,
                report: now,
                metrics: BTreeMap::new(),
                commit: None,
                cursor_input: None,
            })
        });
        cursor_moved();
        submitted(false, true);
        cursor_moved(); // New motion belongs to the next commit, not this completion.
        blocked(true);
        let first = STATE.with(|state| {
            state
                .borrow()
                .as_ref()
                .unwrap()
                .commit
                .as_ref()
                .unwrap()
                .blocked_since
        });
        blocked(true);
        STATE.with(|state| {
            assert_eq!(
                state
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .commit
                    .as_ref()
                    .unwrap()
                    .blocked_since,
                first
            )
        });
        completed();
        completed();
        STATE.with(|state| {
            let state = state.borrow_mut().take().unwrap();
            assert!(state.commit.is_none());
            assert!(state.cursor_input.is_some());
            assert_eq!(state.metrics["cursor_dispatch_to_submit"].count, 1);
            assert_eq!(state.metrics["cursor_dispatch_to_flip"].count, 1);
            assert_eq!(state.metrics["cursor_commit"].count, 1);
            assert_eq!(state.metrics["frame_blocked_by_cursor"].count, 1);
        });
    }

    #[test]
    fn suppressed_details_preserve_lifetime_counts_and_worst() {
        let mut metric = Metric::default();
        for _ in 0..10 {
            metric.add(Duration::from_millis(9));
        }
        assert_eq!(
            (metric.count, metric.slow, metric.logged, metric.suppressed),
            (10, 10, 4, 6)
        );
        assert_eq!(metric.add(Duration::from_millis(50)), (true, false));
        metric.logged = 0;
        assert_eq!(metric.add(Duration::from_millis(10)), (false, true));
        assert_eq!(metric.count, 12);
        assert_eq!(metric.max, Duration::from_millis(50));
        assert_eq!(metric.total, Duration::from_millis(150));
    }
}

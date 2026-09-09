//! Owner-loop startup policy, independent of process creation and XWM dispatch.
//! Deadlines use the caller's monotonic clock. A retry cannot start until the
//! previous child has been reaped; stale generation notifications do nothing.
use std::time::{Duration, Instant};

const STARTUP: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Disabled,
    Preparing,
    Starting,
    Initializing,
    Ready,
    Stopping,
    Backoff,
    Failed,
}

/// Work requested from the host. Environment publication belongs only to Ready;
/// Withdraw also suppresses application recovery for the lost X11 generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Prepare { generation: u64 },
    Spawn { generation: u64 },
    Publish { generation: u64 },
    Withdraw { generation: u64 },
    Stop { generation: u64 },
}

#[derive(Clone, Copy, Debug)]
pub enum Event {
    Prepared,
    Spawned,
    /// A validated displayfd notification, not merely any bytes on the pipe.
    ServerReady,
    /// Root redirection, manager ownership, properties and outputs initialized.
    XwmReady,
    Failed,
    /// Sent only once supervision has reaped the owned child, or confirmed that
    /// spawn failed/was cancelled without creating one. A failure notification
    /// alone is not evidence that all child ownership has ended.
    Reaped,
}

pub struct Lifecycle {
    state: State,
    generation: u64,
    retries: u8,
    deadline: Option<Instant>,
    child: bool,
    server_ready: bool,
    xwm_ready: bool,
    disabled: bool,
}

impl Default for Lifecycle {
    fn default() -> Self {
        Self {
            state: State::Disabled,
            generation: 0,
            retries: 0,
            deadline: None,
            child: false,
            server_ready: false,
            xwm_ready: false,
            disabled: true,
        }
    }
}

impl Lifecycle {
    pub fn state(&self) -> State {
        self.state
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    /// Explicitly enable a disabled or terminally failed instance.
    pub fn start(&mut self, now: Instant) -> Vec<Action> {
        if !matches!(self.state, State::Disabled | State::Failed) {
            return vec![];
        }
        self.retries = 0;
        self.disabled = false;
        self.prepare(now)
    }

    fn prepare(&mut self, now: Instant) -> Vec<Action> {
        let Some(generation) = self.generation.checked_add(1) else {
            self.state = State::Failed;
            self.deadline = None;
            return vec![];
        };
        self.generation = generation;
        self.server_ready = false;
        self.xwm_ready = false;
        self.child = false;
        self.state = State::Preparing;
        self.deadline = Some(now + STARTUP);
        vec![Action::Prepare { generation }]
    }

    pub fn event(&mut self, generation: u64, event: Event, now: Instant) -> Vec<Action> {
        if generation != self.generation {
            return vec![];
        }
        // Reaping is terminal evidence, even when delivered at a deadline.
        // Never discard it in favor of a request to stop an already-dead child.
        if matches!(event, Event::Reaped) && self.child {
            self.child = false;
            let actions = if self.state == State::Ready {
                vec![Action::Withdraw { generation }]
            } else {
                vec![]
            };
            self.after_stop(now);
            return actions;
        }
        // Deadline wins over readiness delivered after expiry.
        let actions = self.tick(now);
        if !actions.is_empty() {
            return actions;
        }
        match (self.state, event) {
            (State::Preparing, Event::Prepared) => {
                self.state = State::Starting;
                // Once Spawn is issued, even a late spawn completion must be
                // stopped and reaped before this generation can be replaced.
                self.child = true;
                vec![Action::Spawn { generation }]
            }
            (State::Starting, Event::Spawned) => {
                self.state = State::Initializing;
                vec![]
            }
            (State::Initializing, Event::ServerReady | Event::XwmReady) => {
                match event {
                    Event::ServerReady => self.server_ready = true,
                    Event::XwmReady => self.xwm_ready = true,
                    _ => unreachable!(),
                }
                if self.server_ready && self.xwm_ready {
                    self.state = State::Ready;
                    self.deadline = None;
                    vec![Action::Publish { generation }]
                } else {
                    vec![]
                }
            }
            (
                State::Preparing | State::Starting | State::Initializing | State::Ready,
                Event::Failed,
            ) => self.fail(now),
            _ => vec![],
        }
    }

    /// Host must explicitly authorize loss of existing connections before calling.
    pub fn stop(&mut self) -> Vec<Action> {
        self.disabled = true;
        self.deadline = None;
        let mut actions = vec![];
        if self.state == State::Ready {
            actions.push(Action::Withdraw {
                generation: self.generation,
            });
        }
        if self.child {
            if self.state != State::Stopping {
                actions.push(Action::Stop {
                    generation: self.generation,
                });
            }
            self.state = State::Stopping;
        } else {
            self.state = State::Disabled;
        }
        actions
    }

    pub fn tick(&mut self, now: Instant) -> Vec<Action> {
        if !self.deadline.is_some_and(|deadline| now >= deadline) {
            return vec![];
        }
        match self.state {
            State::Backoff => self.prepare(now),
            State::Preparing | State::Starting | State::Initializing => self.fail(now),
            _ => vec![],
        }
    }

    fn fail(&mut self, now: Instant) -> Vec<Action> {
        let mut actions = vec![];
        if self.state == State::Ready {
            actions.push(Action::Withdraw {
                generation: self.generation,
            });
        }
        self.deadline = None;
        if self.child {
            self.state = State::Stopping;
            actions.push(Action::Stop {
                generation: self.generation,
            });
        } else {
            self.after_stop(now);
        }
        actions
    }

    fn after_stop(&mut self, now: Instant) {
        if self.disabled {
            self.state = State::Disabled;
            self.deadline = None;
        } else if self.retries == 3 {
            self.state = State::Failed;
            self.deadline = None;
        } else {
            self.deadline = Some(now + Duration::from_secs(1 << self.retries));
            self.retries += 1;
            self.state = State::Backoff;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn initializing(l: &mut Lifecycle, now: Instant) -> u64 {
        l.start(now);
        let g = l.generation();
        assert_eq!(
            l.event(g, Event::Prepared, now),
            vec![Action::Spawn { generation: g }]
        );
        l.event(g, Event::Spawned, now);
        g
    }
    #[test]
    fn readiness_requires_both_barriers_in_either_order() {
        for first in [Event::ServerReady, Event::XwmReady] {
            let now = Instant::now();
            let mut l = Lifecycle::default();
            let g = initializing(&mut l, now);
            assert!(l.event(g, first, now).is_empty());
            assert_eq!(l.state(), State::Initializing);
            let other = match first {
                Event::ServerReady => Event::XwmReady,
                _ => Event::ServerReady,
            };
            assert_eq!(
                l.event(g, other, now),
                vec![Action::Publish { generation: g }]
            );
            assert!(l.event(g, other, now).is_empty());
            assert_eq!(
                l.stop(),
                vec![
                    Action::Withdraw { generation: g },
                    Action::Stop { generation: g }
                ]
            );
            l.event(g, Event::Reaped, now);
            assert_eq!(l.state(), State::Disabled);
        }
    }
    #[test]
    fn timeout_waits_for_reaping_and_exhausts_exactly_three_retries() {
        let mut now = Instant::now();
        let mut l = Lifecycle::default();
        initializing(&mut l, now);
        for delay in [1, 2, 4] {
            let g = l.generation();
            now += STARTUP;
            assert_eq!(
                l.event(g, Event::XwmReady, now),
                vec![Action::Stop { generation: g }]
            );
            assert_eq!(l.state(), State::Stopping);
            assert!(l.tick(now + Duration::from_secs(100)).is_empty());
            l.event(g, Event::Reaped, now);
            assert_eq!(l.deadline(), Some(now + Duration::from_secs(delay)));
            now += Duration::from_secs(delay);
            assert_eq!(l.tick(now), vec![Action::Prepare { generation: g + 1 }]);
            assert!(l.event(g, Event::Prepared, now).is_empty());
            l.event(g + 1, Event::Prepared, now);
            l.event(g + 1, Event::Spawned, now);
        }
        l.event(l.generation(), Event::Reaped, now);
        assert_eq!(l.state(), State::Failed);
        assert!(l.tick(now + STARTUP).is_empty());
    }
    #[test]
    fn cancellation_during_spawn_cannot_leave_late_child_unowned() {
        let now = Instant::now();
        let mut l = Lifecycle::default();
        l.start(now);
        l.event(1, Event::Prepared, now);
        assert_eq!(l.stop(), vec![Action::Stop { generation: 1 }]);
        assert!(l.event(1, Event::Spawned, now).is_empty());
        assert!(l.start(now).is_empty());
        l.event(1, Event::Reaped, now);
        assert_eq!(l.state(), State::Disabled);
        assert_eq!(l.start(now), vec![Action::Prepare { generation: 2 }]);
    }
    #[test]
    fn ready_crash_withdraws_once_and_cannot_publish_old_generation() {
        let now = Instant::now();
        let mut l = Lifecycle::default();
        let g = initializing(&mut l, now);
        l.event(g, Event::ServerReady, now);
        l.event(g, Event::XwmReady, now);
        assert_eq!(
            l.event(g, Event::Reaped, now),
            vec![Action::Withdraw { generation: g }]
        );
        assert!(l.event(g, Event::Reaped, now).is_empty());
        l.tick(now + Duration::from_secs(1));
        assert!(l.event(g, Event::XwmReady, now).is_empty());
        assert_eq!(l.state(), State::Preparing);
    }

    #[test]
    fn reap_at_deadline_does_not_request_stop_or_wait_for_second_reap() {
        let now = Instant::now();
        let mut l = Lifecycle::default();
        let g = initializing(&mut l, now);
        assert!(l.event(g, Event::Reaped, now + STARTUP).is_empty());
        assert_eq!(l.state(), State::Backoff);
        assert_eq!(l.deadline(), Some(now + STARTUP + Duration::from_secs(1)));
    }
}

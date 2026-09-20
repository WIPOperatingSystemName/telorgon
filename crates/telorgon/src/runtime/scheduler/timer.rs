use std::any::{Any, TypeId};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::foundation::MonotonicInstant;

use crate::runtime::{ComponentId, RuntimeError, RuntimeResult, routed_action::RoutedAction};

#[derive(Clone)]
struct TimerWake {
    state: Arc<TimerWakeState>,
}

struct TimerWakeState {
    pending: AtomicBool,
    notify: Box<dyn Fn() + Send + Sync>,
}

impl TimerWake {
    fn new(notify: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            state: Arc::new(TimerWakeState {
                pending: AtomicBool::new(false),
                notify: Box::new(notify),
            }),
        }
    }

    fn signal(&self) {
        if !self.state.pending.swap(true, Ordering::AcqRel) {
            (self.state.notify)();
        }
    }

    fn begin_turn(&self) {
        self.state.pending.store(false, Ordering::Release);
    }
}

struct TimerControl {
    cancelled: AtomicBool,
    wake: Mutex<Option<TimerWake>>,
}

impl TimerControl {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            cancelled: AtomicBool::new(false),
            wake: Mutex::new(None),
        })
    }

    fn attach_wake(&self, wake: TimerWake) {
        *lock(&self.wake) = Some(wake);
    }

    fn cancel(&self) {
        if !self.cancelled.swap(true, Ordering::AcqRel)
            && let Some(wake) = lock(&self.wake).as_ref()
        {
            wake.signal();
        }
    }

    fn cancel_without_wake(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

/// Cloneable cancellation authority for one component-scoped timer.
#[derive(Clone)]
pub struct TimerHandle {
    control: Arc<TimerControl>,
}

impl TimerHandle {
    pub fn cancel(&self) {
        self.control.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.control.cancelled.load(Ordering::Acquire)
    }
}

enum TimerAction {
    Once(Option<Box<dyn Any>>),
    Repeating(Box<dyn FnMut() -> Box<dyn Any>>),
}

pub(crate) struct PendingTimerStart {
    deadline: MonotonicInstant,
    period_nanos: Option<u64>,
    type_id: TypeId,
    action: TimerAction,
    control: Arc<TimerControl>,
}

impl PendingTimerStart {
    pub(crate) fn once<Action: 'static>(
        deadline: MonotonicInstant,
        action: Action,
    ) -> (Self, TimerHandle) {
        let control = TimerControl::new();
        let handle = TimerHandle {
            control: control.clone(),
        };
        (
            Self {
                deadline,
                period_nanos: None,
                type_id: TypeId::of::<Action>(),
                action: TimerAction::Once(Some(Box::new(action))),
                control,
            },
            handle,
        )
    }

    pub(crate) fn repeating<Action, F>(
        deadline: MonotonicInstant,
        period: Duration,
        mut action: F,
    ) -> RuntimeResult<(Self, TimerHandle)>
    where
        Action: 'static,
        F: FnMut() -> Action + 'static,
    {
        let period_nanos = u64::try_from(period.as_nanos())
            .ok()
            .filter(|period| *period > 0)
            .ok_or_else(|| {
                RuntimeError::new(
                    "component timer period must fit in u64 nanoseconds and be nonzero",
                )
            })?;
        let control = TimerControl::new();
        let handle = TimerHandle {
            control: control.clone(),
        };
        Ok((
            Self {
                deadline,
                period_nanos: Some(period_nanos),
                type_id: TypeId::of::<Action>(),
                action: TimerAction::Repeating(Box::new(move || Box::new(action()))),
                control,
            },
            handle,
        ))
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
struct TimerKey {
    index: u32,
    generation: u32,
}

struct TimerSlot {
    generation: u32,
    owner: Option<ComponentId>,
    deadline: MonotonicInstant,
    period_nanos: Option<u64>,
    type_id: TypeId,
    action: Option<TimerAction>,
    control: Option<Arc<TimerControl>>,
}

pub(crate) enum TimerStart {
    Started,
    Cancelled,
}

pub(crate) struct TimerDrain {
    pub(crate) actions: VecDeque<RoutedAction>,
    pub(crate) fired: usize,
    pub(crate) cancelled: usize,
    pub(crate) stale: usize,
    pub(crate) missed_intervals: u64,
    pub(crate) queue_depth: usize,
}

pub(crate) struct TimerArena {
    slots: Vec<TimerSlot>,
    free: Vec<u32>,
    deadlines: BinaryHeap<Reverse<(MonotonicInstant, u32, u32)>>,
    live: usize,
    wake: TimerWake,
}

impl TimerArena {
    pub(crate) fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            deadlines: BinaryHeap::new(),
            live: 0,
            wake: TimerWake::new(wake),
        }
    }

    pub(crate) fn start(&mut self, owner: ComponentId, start: PendingTimerStart) -> TimerStart {
        let key = self.allocate(owner, start);
        let slot = &self.slots[key.index as usize];
        let control = slot.control.as_ref().expect("live timer has control");
        control.attach_wake(self.wake.clone());
        if control.cancelled.load(Ordering::Acquire) {
            self.release(key);
            TimerStart::Cancelled
        } else {
            let slot = &self.slots[key.index as usize];
            self.deadlines
                .push(Reverse((slot.deadline, key.index, key.generation)));
            TimerStart::Started
        }
    }

    pub(crate) fn begin_turn(&self) {
        self.wake.begin_turn();
    }

    pub(crate) fn finish_turn(&self, now: MonotonicInstant) {
        if self.is_ready(now) {
            self.wake.signal();
        }
    }

    pub(crate) fn is_ready(&self, now: MonotonicInstant) -> bool {
        self.slots.iter().any(|slot| {
            slot.owner.is_some()
                && (slot.deadline <= now
                    || slot
                        .control
                        .as_ref()
                        .is_some_and(|control| control.cancelled.load(Ordering::Acquire)))
        })
    }

    pub(crate) fn next_deadline(&self) -> Option<MonotonicInstant> {
        self.slots
            .iter()
            .filter(|slot| slot.owner.is_some())
            .map(|slot| slot.deadline)
            .min()
    }

    pub(crate) fn drain_due(&mut self, now: MonotonicInstant, limit: usize) -> TimerDrain {
        let queue_depth = self.live;
        let mut actions = VecDeque::new();
        let mut fired = 0;
        let mut cancelled = 0;
        let mut stale = 0;
        let mut missed_intervals = 0_u64;

        let requested = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| {
                (slot.owner.is_some()
                    && slot
                        .control
                        .as_ref()
                        .is_some_and(|control| control.cancelled.load(Ordering::Acquire)))
                .then_some(TimerKey {
                    index: index as u32,
                    generation: slot.generation,
                })
            })
            .take(limit)
            .collect::<Vec<_>>();
        for key in requested {
            self.release(key);
            cancelled += 1;
        }

        while fired + cancelled + stale < limit {
            let Some(Reverse((deadline, index, generation))) = self.deadlines.peek().copied()
            else {
                break;
            };
            if deadline > now {
                break;
            }
            self.deadlines.pop();
            let key = TimerKey { index, generation };
            let Some(slot) = self.slots.get(index as usize) else {
                stale += 1;
                continue;
            };
            if slot.generation != generation || slot.owner.is_none() || slot.deadline != deadline {
                stale += 1;
                continue;
            }

            let (target, type_id, value, period_nanos) = {
                let slot = &mut self.slots[index as usize];
                let value = match slot.action.as_mut().expect("live timer has action") {
                    TimerAction::Once(value) => value.take().expect("one-shot timer fires once"),
                    TimerAction::Repeating(action) => action(),
                };
                (
                    slot.owner.expect("live timer has owner"),
                    slot.type_id,
                    value,
                    slot.period_nanos,
                )
            };
            actions.push_back(RoutedAction {
                target,
                type_id,
                value,
            });
            fired += 1;

            if let Some(period_nanos) = period_nanos {
                let elapsed = now.as_nanos().saturating_sub(deadline.as_nanos());
                let periods = elapsed / period_nanos + 1;
                missed_intervals = missed_intervals.saturating_add(periods.saturating_sub(1));
                let advance = period_nanos.checked_mul(periods);
                let next = advance.and_then(|advance| deadline.as_nanos().checked_add(advance));
                if let Some(next) = next {
                    let next = MonotonicInstant::from_nanos(next);
                    self.slots[index as usize].deadline = next;
                    self.deadlines.push(Reverse((next, index, generation)));
                } else {
                    self.release(key);
                }
            } else {
                self.release(key);
            }
        }

        TimerDrain {
            actions,
            fired,
            cancelled,
            stale,
            missed_intervals,
            queue_depth,
        }
    }

    pub(crate) fn remove_owner(&mut self, owner: ComponentId) -> usize {
        let timers = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| {
                (slot.owner == Some(owner)).then_some(TimerKey {
                    index: index as u32,
                    generation: slot.generation,
                })
            })
            .collect::<Vec<_>>();
        for timer in &timers {
            self.release(*timer);
        }
        timers.len()
    }

    pub(crate) fn live(&self) -> usize {
        self.live
    }

    fn allocate(&mut self, owner: ComponentId, start: PendingTimerStart) -> TimerKey {
        let (index, generation) = if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.owner = Some(owner);
            slot.deadline = start.deadline;
            slot.period_nanos = start.period_nanos;
            slot.type_id = start.type_id;
            slot.action = Some(start.action);
            slot.control = Some(start.control);
            (index, slot.generation)
        } else {
            let index = self.slots.len() as u32;
            self.slots.push(TimerSlot {
                generation: 1,
                owner: Some(owner),
                deadline: start.deadline,
                period_nanos: start.period_nanos,
                type_id: start.type_id,
                action: Some(start.action),
                control: Some(start.control),
            });
            (index, 1)
        };
        self.live += 1;
        TimerKey { index, generation }
    }

    fn release(&mut self, key: TimerKey) {
        let Some(slot) = self.slots.get_mut(key.index as usize) else {
            return;
        };
        if slot.generation != key.generation || slot.owner.is_none() {
            return;
        }
        if let Some(control) = slot.control.take() {
            control.cancel_without_wake();
        }
        slot.owner = None;
        slot.action = None;
        slot.period_nanos = None;
        slot.generation = slot.generation.wrapping_add(1).max(1);
        self.free.push(key.index);
        self.live -= 1;
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

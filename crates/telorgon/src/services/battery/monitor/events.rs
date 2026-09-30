use std::{
    collections::VecDeque,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};

use super::BatteryEvent;

struct Queue {
    events: VecDeque<BatteryEvent>,
    capacity: usize,
    overflow: bool,
    closed: bool,
    waker: Option<Waker>,
}

pub(super) struct Subscriber {
    queue: Mutex<Queue>,
}

impl Subscriber {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            queue: Mutex::new(Queue {
                events: VecDeque::new(),
                capacity,
                overflow: false,
                closed: false,
                waker: None,
            }),
        }
    }

    pub(super) fn emit(&self, events: &[BatteryEvent]) {
        if events.is_empty() {
            return;
        }
        let waker = {
            let mut queue = self.queue.lock().expect("battery event lock poisoned");
            if queue.closed {
                return;
            }
            for event in events {
                if queue.overflow {
                    break;
                }
                if queue.events.len() == queue.capacity {
                    queue.events.clear();
                    queue.overflow = true;
                } else {
                    queue.events.push_back(event.clone());
                }
            }
            queue.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    pub(super) fn close(&self) {
        let waker = {
            let mut queue = self.queue.lock().expect("battery event lock poisoned");
            queue.closed = true;
            queue.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    fn take(queue: &mut Queue) -> Option<BatteryEvent> {
        if queue.overflow {
            queue.overflow = false;
            queue.events.clear();
            Some(BatteryEvent::ResyncRequired)
        } else {
            queue.events.pop_front()
        }
    }
}

/// FIFO observed transitions until overflow replaces pending events with `ResyncRequired`.
/// Dropping this receiver unsubscribes. Temporary observation failures do not close it;
/// shutdown drains pending events and then `next()` returns `None`.
pub struct BatteryEvents {
    subscriber: Arc<Subscriber>,
}

impl BatteryEvents {
    pub(super) fn new(subscriber: Arc<Subscriber>) -> Self {
        Self { subscriber }
    }

    pub fn try_recv(&mut self) -> Option<BatteryEvent> {
        Subscriber::take(
            &mut self
                .subscriber
                .queue
                .lock()
                .expect("battery event lock poisoned"),
        )
    }

    /// True after observation stops. Pending events can still be drained.
    pub fn is_closed(&self) -> bool {
        self.subscriber
            .queue
            .lock()
            .expect("battery event lock poisoned")
            .closed
    }

    /// Waits using standard Rust wakers; no Tokio dependency. Cancelling this future leaves
    /// pending events intact and unregisters its waker.
    pub fn next(&mut self) -> impl Future<Output = Option<BatteryEvent>> + '_ {
        NextEvent {
            subscriber: &self.subscriber,
        }
    }
}

struct NextEvent<'a> {
    subscriber: &'a Subscriber,
}

impl Future for NextEvent<'_> {
    type Output = Option<BatteryEvent>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut queue = self
            .subscriber
            .queue
            .lock()
            .expect("battery event lock poisoned");
        if let Some(event) = Subscriber::take(&mut queue) {
            return Poll::Ready(Some(event));
        }
        if queue.closed {
            return Poll::Ready(None);
        }
        if queue
            .waker
            .as_ref()
            .is_none_or(|waker| !waker.will_wake(cx.waker()))
        {
            queue.waker = Some(cx.waker().clone());
        }
        Poll::Pending
    }
}

impl Drop for NextEvent<'_> {
    fn drop(&mut self) {
        self.subscriber
            .queue
            .lock()
            .expect("battery event lock poisoned")
            .waker = None;
    }
}

#[cfg(test)]
mod tests;

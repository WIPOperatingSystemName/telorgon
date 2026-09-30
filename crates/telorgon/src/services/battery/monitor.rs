use std::{
    sync::{
        Arc, Condvar, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use super::{
    BatteryAvailability, BatteryError, BatteryEvent, BatteryMonitorState, BatteryProvider,
    BatterySnapshot, SystemBatteryProvider,
};
use crate::authoring::compose::{Signal, SignalSnapshot, SignalWriter};

mod events;
mod observation;
pub use events::BatteryEvents;
use events::Subscriber;
use observation::Observation;

const MAX_SUBSCRIPTIONS: usize = 64;

#[derive(Clone, Debug)]
pub struct BatteryMonitorConfig {
    /// Polling latency, not a guarantee of observing every native transition. Default: 2 seconds.
    pub refresh_interval: Duration,
    /// Bounded queue size per subscription. Default: 32; valid range: 1..=4096.
    pub event_capacity: usize,
}

impl Default for BatteryMonitorConfig {
    fn default() -> Self {
        Self {
            refresh_interval: Duration::from_secs(2),
            event_capacity: 32,
        }
    }
}

impl BatteryMonitorConfig {
    fn validate(&self) -> Result<(), BatteryError> {
        if self.refresh_interval.is_zero() || self.refresh_interval > Duration::from_secs(3600) {
            return Err(BatteryError::InvalidConfig(
                "refresh interval must be positive and at most one hour",
            ));
        }
        if !(1..=4096).contains(&self.event_capacity) {
            return Err(BatteryError::InvalidConfig(
                "event capacity must be between 1 and 4096",
            ));
        }
        Ok(())
    }
}

/// Starts native observation explicitly. No Tokio or GUI session is required.
pub async fn monitor(config: BatteryMonitorConfig) -> Result<BatteryMonitor, BatteryError> {
    BatteryMonitor::start(config).await
}

/// Owns the refresh worker. Handles and subscriptions do not prolong observation after owner drop.
/// Drop requests shutdown without joining; an in-flight native read may finish before shutdown.
pub struct BatteryMonitor {
    handle: BatteryMonitorHandle,
    worker: Option<JoinHandle<()>>,
}

impl BatteryMonitor {
    pub async fn start(config: BatteryMonitorConfig) -> Result<Self, BatteryError> {
        Self::start_with_provider(SystemBatteryProvider, config).await
    }

    /// Supplies a worker-owned source for custom or embedded hardware. It needs `Send`, not `Sync`.
    /// The first read runs off the caller's thread and initializes a baseline without events.
    /// Startup errors return directly; subsequent read failures publish unavailability and retry.
    pub async fn start_with_provider<P>(
        provider: P,
        config: BatteryMonitorConfig,
    ) -> Result<Self, BatteryError>
    where
        P: BatteryProvider + Send + 'static,
    {
        config.validate()?;
        blocking::unblock(move || {
            // The startup executor waits only for the worker's baseline. All provider reads
            // happen on one worker, including the first read, preserving source ownership.
            let (ready, initial) = mpsc::sync_channel(1);
            let worker = thread::Builder::new()
                .name("telorgon-battery".into())
                .spawn(move || {
                    let first = match provider.read_snapshot() {
                        Ok(snapshot) => snapshot,
                        Err(error) => {
                            let _ = ready.send(Err(error));
                            return;
                        }
                    };
                    let worker_shared = Arc::new(Shared::new(first, config.event_capacity));
                    let _completion = WorkerCompletion(worker_shared.clone());
                    if ready.send(Ok(worker_shared.clone())).is_err() {
                        return;
                    }
                    while !worker_shared.stop.wait(config.refresh_interval) {
                        let result = provider.read_snapshot();
                        if worker_shared.stop.requested.load(Ordering::Acquire) {
                            break;
                        }
                        worker_shared.observe(result);
                    }
                })
                .map_err(BatteryError::Start)?;
            let shared = match initial.recv() {
                Ok(Ok(shared)) => shared,
                Ok(Err(error)) => {
                    let _ = worker.join();
                    return Err(error);
                }
                Err(_) => {
                    let _ = worker.join();
                    return Err(BatteryError::WorkerPanicked);
                }
            };
            Ok(Self {
                handle: BatteryMonitorHandle { shared },
                worker: Some(worker),
            })
        })
        .await
    }

    pub fn handle(&self) -> BatteryMonitorHandle {
        self.handle.clone()
    }

    /// Requests shutdown and interrupts the polling wait without blocking on a native read.
    pub fn stop(&self) {
        self.handle.shared.stop.request();
    }

    /// Requests shutdown and awaits worker completion off the caller's thread.
    /// This cannot cancel a provider's in-flight read; providers must eventually return.
    pub async fn shutdown(mut self) -> Result<(), BatteryError> {
        self.stop();
        if let Some(worker) = self.worker.take() {
            blocking::unblock(move || worker.join())
                .await
                .map_err(|_| BatteryError::WorkerPanicked)?;
        }
        Ok(())
    }
}

impl Drop for BatteryMonitor {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Clone)]
pub struct BatteryMonitorHandle {
    shared: Arc<Shared>,
}

impl PartialEq for BatteryMonitorHandle {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.shared, &other.shared)
    }
}
impl Eq for BatteryMonitorHandle {}

impl BatteryMonitorHandle {
    /// Watch this in a component with `self.watch(&handle.signal())`.
    pub fn signal(&self) -> Signal<BatteryMonitorState> {
        self.shared.signal.clone()
    }

    pub fn state(&self) -> SignalSnapshot<BatteryMonitorState> {
        self.shared.signal.snapshot()
    }

    /// Latest successful full reading, including its timestamp. During failure or shutdown this
    /// remains available but stale; consult `state().availability` for availability.
    pub fn snapshot(&self) -> Arc<BatterySnapshot> {
        self.shared
            .data
            .lock()
            .expect("battery monitor lock poisoned")
            .observation
            .latest
            .clone()
    }

    /// Starts with an empty queue; startup state is obtained through the signal or snapshot.
    /// Up to 64 subscriptions are retained. Dropping a subscription releases its queue.
    pub fn subscribe(&self) -> Result<BatteryEvents, BatteryError> {
        let mut data = self
            .shared
            .data
            .lock()
            .expect("battery monitor lock poisoned");
        if self.shared.stop.requested.load(Ordering::Acquire)
            || data.observation.status.availability == BatteryAvailability::Stopped
        {
            return Err(BatteryError::Stopped);
        }
        data.subscribers.retain(|s| s.strong_count() != 0);
        if data.subscribers.len() == MAX_SUBSCRIPTIONS {
            return Err(BatteryError::SubscriberLimitReached);
        }
        let subscriber = Arc::new(Subscriber::new(self.shared.event_capacity));
        data.subscribers.push(Arc::downgrade(&subscriber));
        Ok(BatteryEvents::new(subscriber))
    }
}

struct Data {
    observation: Observation,
    subscribers: Vec<Weak<Subscriber>>,
}

struct Shared {
    data: Mutex<Data>,
    signal: Signal<BatteryMonitorState>,
    publish: SignalWriter<BatteryMonitorState>,
    event_capacity: usize,
    stop: Stop,
}

impl Shared {
    fn new(initial: BatterySnapshot, event_capacity: usize) -> Self {
        let observation = Observation::new(initial);
        let (signal, publish) = Signal::new(observation.status.clone());
        Self {
            data: Mutex::new(Data {
                observation,
                subscribers: Vec::new(),
            }),
            signal,
            publish,
            event_capacity,
            stop: Stop::default(),
        }
    }

    // The worker is the only publisher. Signals/wakers run after releasing the state lock.
    fn observe(&self, result: Result<BatterySnapshot, BatteryError>) {
        let (status, events, subscribers) = {
            let mut data = self.data.lock().expect("battery monitor lock poisoned");
            let events = data.observation.update(result);
            data.subscribers.retain(|s| s.strong_count() != 0);
            (
                data.observation.status.clone(),
                events,
                data.subscribers
                    .iter()
                    .filter_map(Weak::upgrade)
                    .collect::<Vec<_>>(),
            )
        };
        self.publish.publish_if_changed(status);
        for subscriber in subscribers {
            subscriber.emit(&events);
        }
    }

    fn finish(&self) {
        let (status, previous, subscribers) = {
            let mut data = self.data.lock().expect("battery monitor lock poisoned");
            let previous = data.observation.status.availability;
            data.observation.status.availability = BatteryAvailability::Stopped;
            (
                data.observation.status.clone(),
                previous,
                data.subscribers
                    .iter()
                    .filter_map(Weak::upgrade)
                    .collect::<Vec<_>>(),
            )
        };
        self.publish.publish_if_changed(status);
        for subscriber in subscribers {
            subscriber.emit(&[BatteryEvent::AvailabilityChanged {
                previous,
                current: BatteryAvailability::Stopped,
            }]);
            subscriber.close();
        }
    }
}

struct WorkerCompletion(Arc<Shared>);
impl Drop for WorkerCompletion {
    fn drop(&mut self) {
        self.0.finish();
    }
}

#[derive(Default)]
struct Stop {
    requested: AtomicBool,
    lock: Mutex<()>,
    wake: Condvar,
}

impl Stop {
    fn request(&self) {
        let _guard = self.lock.lock().expect("battery stop lock poisoned");
        self.requested.store(true, Ordering::Release);
        self.wake.notify_all();
    }

    fn wait(&self, interval: Duration) -> bool {
        let guard = self.lock.lock().expect("battery stop lock poisoned");
        let _wait = self
            .wake
            .wait_timeout_while(guard, interval, |_| !self.requested.load(Ordering::Acquire))
            .expect("battery stop lock poisoned");
        self.requested.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests;

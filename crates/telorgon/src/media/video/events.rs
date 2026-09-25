//! Coalesced readiness notifications; frame payloads remain in the bounded stream queue.
use super::*;
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicU8, Ordering},
};
pub(crate) const STATE: u8 = 1;
pub(crate) const FORMAT: u8 = 2;
pub(crate) const FRAME: u8 = 4;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoUpdate {
    pub state_changed: bool,
    pub format_changed: bool,
    pub frame_available: bool,
}
struct Watcher {
    pending: AtomicU8,
    wake: Box<dyn Fn() + Send + Sync>,
}
impl Watcher {
    fn notify(&self, flags: u8) {
        if self.pending.fetch_or(flags, Ordering::AcqRel) == 0 {
            // Wake only asks the host to service this subscription. User panics may not
            // cross native callbacks; retained pending bits remain observable by polling.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (self.wake)()));
        }
    }
}
/// One coalesced update word, never an unbounded event queue. After wake, take_update and
/// inspect stream snapshots/frames on the UI thread. Intermediate states may be coalesced.
/// Drop unregisters lazily; a wake already in flight may finish. The callback runs on the
/// native control worker (initial wake on the subscribing thread): it must not block, join
/// the media connection, or call UI APIs directly. Send an event to the host's event loop.
pub struct VideoSubscription {
    watcher: Arc<Watcher>,
}
impl VideoSubscription {
    pub fn take_update(&self) -> Option<VideoUpdate> {
        let flags = self.watcher.pending.swap(0, Ordering::AcqRel);
        (flags != 0).then_some(VideoUpdate {
            state_changed: flags & STATE != 0,
            format_changed: flags & FORMAT != 0,
            frame_available: flags & FRAME != 0,
        })
    }
}
#[derive(Default)]
pub(crate) struct Notifications {
    watchers: Mutex<Vec<Weak<Watcher>>>,
}
impl Notifications {
    pub fn subscribe(
        &self,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<VideoSubscription, MediaError> {
        let watcher = Arc::new(Watcher {
            pending: AtomicU8::new(0),
            wake: Box::new(wake),
        });
        {
            let mut watchers = self.watchers.lock().unwrap_or_else(|e| e.into_inner());
            watchers.retain(|w| w.strong_count() > 0);
            if watchers.len() >= 8 {
                return Err(MediaError::ResourceLimit("video subscriptions"));
            }
            watchers.push(Arc::downgrade(&watcher));
        }
        watcher.notify(STATE | FORMAT | FRAME);
        Ok(VideoSubscription { watcher })
    }
    pub fn notify(&self, flags: u8) {
        let mut recipients: [Option<Arc<Watcher>>; 8] = std::array::from_fn(|_| None);
        {
            let mut watchers = self.watchers.lock().unwrap_or_else(|e| e.into_inner());
            watchers.retain(|w| w.strong_count() > 0);
            for (slot, watcher) in recipients.iter_mut().zip(watchers.iter()) {
                *slot = watcher.upgrade();
            }
        }
        for watcher in recipients.into_iter().flatten() {
            watcher.notify(flags);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    #[test]
    fn repeated_frames_coalesce_and_wake_without_holding_subscription_lock() {
        let notifications = Arc::new(Notifications::default());
        let wakes = Arc::new(AtomicUsize::new(0));
        let count = wakes.clone();
        let other = notifications.clone();
        let subscription = notifications
            .subscribe(move || {
                assert!(other.watchers.try_lock().is_ok());
                count.fetch_add(1, Ordering::Relaxed);
            })
            .unwrap();
        assert!(subscription.take_update().unwrap().format_changed);
        for _ in 0..1000 {
            notifications.notify(FRAME);
        }
        notifications.notify(STATE);
        assert_eq!(wakes.load(Ordering::Relaxed), 2);
        assert_eq!(
            subscription.take_update(),
            Some(VideoUpdate {
                state_changed: true,
                format_changed: false,
                frame_available: true
            })
        );
        assert!(subscription.take_update().is_none());
        drop(subscription);
        notifications.notify(FRAME);
        assert_eq!(wakes.load(Ordering::Relaxed), 2);
    }
}

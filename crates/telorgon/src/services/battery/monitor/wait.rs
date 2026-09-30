use std::{sync::atomic::Ordering, time::Duration};

use super::{BatteryUpdateMode, Stop};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Refresh {
    Changed,
    Resync,
    Stopped,
}

pub(super) enum WaitSource {
    Polling,
    #[cfg(test)]
    Controlled {
        events: std::sync::mpsc::Receiver<Refresh>,
        stop: std::sync::mpsc::Sender<Refresh>,
    },
    #[cfg(target_os = "linux")]
    Native(crate::services::battery::linux::notifications::Notifications),
}

impl WaitSource {
    pub(super) fn use_polling(&mut self, stop: &Stop) {
        *self = Self::Polling;
        stop.detach_native_wake();
    }

    pub(super) fn system(mode: BatteryUpdateMode) -> Self {
        #[cfg(target_os = "linux")]
        if mode == BatteryUpdateMode::Auto {
            if let Ok(notifications) =
                crate::services::battery::linux::notifications::Notifications::new()
            {
                return Self::Native(notifications);
            }
        }
        #[cfg(not(target_os = "linux"))]
        let _ = mode;
        Self::Polling
    }

    pub(super) fn stop(&self) -> Stop {
        #[cfg(test)]
        if let Self::Controlled { stop, .. } = self {
            return Stop {
                test_wake: Some(stop.clone()),
                ..Stop::default()
            };
        }
        #[cfg(target_os = "linux")]
        if let Self::Native(notifications) = self {
            return Stop {
                native_wake: std::sync::Mutex::new(Some(notifications.shutdown_wake())),
                ..Stop::default()
            };
        }
        Stop::default()
    }

    pub(super) fn wait(&mut self, stop: &Stop, interval: Duration) -> Refresh {
        if stop.requested.load(Ordering::Acquire) {
            return Refresh::Stopped;
        }
        #[cfg(test)]
        if let Self::Controlled { events, .. } = self {
            return events.recv().unwrap_or(Refresh::Stopped);
        }
        #[cfg(target_os = "linux")]
        if let Self::Native(notifications) = self {
            match notifications.wait(|| stop.requested.load(Ordering::Acquire)) {
                Ok(crate::services::battery::linux::notifications::Notification::Changed) => {
                    return Refresh::Changed;
                }
                Ok(crate::services::battery::linux::notifications::Notification::Resync) => {
                    return Refresh::Resync;
                }
                Ok(crate::services::battery::linux::notifications::Notification::Stopped) => {
                    return Refresh::Stopped;
                }
                Err(_) => {
                    // Drop the failed socket; immediately refresh without synthesizing sounds.
                    self.use_polling(stop);
                    return Refresh::Resync;
                }
            }
        }
        if stop.wait(interval) {
            Refresh::Stopped
        } else {
            Refresh::Changed
        }
    }
}

//! Process-wide requests, owned and consumed by managed host runs.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

static HOSTS: ExitRegistry = ExitRegistry(Mutex::new(Vec::new()));

/// Requests a clean shutdown of every currently running Telorgon managed GUI or compositor host.
///
/// This thread-safe function returns immediately. It wakes idle hosts; each host exits its event
/// loop and performs normal cleanup after control returns from the current callback. It does not
/// terminate the process, interrupt callbacks, or stop an embedded/headless runtime. With no
/// managed host running it does nothing, and requests never carry over to a later host run.
/// Repeated requests for the same run are coalesced.
///
/// ```
/// use telorgon::app::{Compositor, KeyBindings, KeyChord, ShortcutKey};
/// let shortcuts = KeyBindings::new().bind(
///     KeyChord::new(ShortcutKey::Q).control().shift(),
///     telorgon::request_exit,
/// );
/// let compositor = Compositor::new().keybindings(shortcuts);
/// ```
pub fn request_exit() {
    HOSTS.request();
}

struct ExitRegistry(Mutex<Vec<Weak<HostExit>>>);

impl ExitRegistry {
    fn register(&self, wake: impl Fn() + Send + Sync + 'static) -> Arc<HostExit> {
        let host = Arc::new(HostExit {
            requested: AtomicBool::new(false),
            wake: Box::new(wake),
        });
        let mut hosts = self.0.lock().unwrap_or_else(|error| error.into_inner());
        hosts.retain(|host| host.strong_count() != 0);
        hosts.push(Arc::downgrade(&host));
        host
    }

    fn request(&self) {
        let hosts: Vec<_> = {
            let mut hosts = self.0.lock().unwrap_or_else(|error| error.into_inner());
            hosts.retain(|host| host.strong_count() != 0);
            hosts.iter().filter_map(Weak::upgrade).collect()
        };
        // Never call platform wake code under the registry lock.
        for host in hosts {
            if !host.requested.swap(true, Ordering::AcqRel) {
                (host.wake)();
            }
        }
    }
}

pub(crate) struct HostExit {
    requested: AtomicBool,
    wake: Box<dyn Fn() + Send + Sync>,
}

impl HostExit {
    pub(crate) fn cancel(&self) {
        self.requested.store(false, Ordering::Release);
    }
    pub(crate) fn register(wake: impl Fn() + Send + Sync + 'static) -> Arc<Self> {
        HOSTS.register(wake)
    }

    pub(crate) fn requested(&self) -> bool {
        self.requested.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn requests_wake_all_active_hosts_once_from_another_thread() {
        let registry = Arc::new(ExitRegistry(Mutex::new(Vec::new())));
        let wakes = Arc::new(AtomicUsize::new(0));
        let first = registry.register({
            let wakes = wakes.clone();
            move || {
                wakes.fetch_add(1, Ordering::SeqCst);
            }
        });
        let second = registry.register({
            let wakes = wakes.clone();
            move || {
                wakes.fetch_add(1, Ordering::SeqCst);
            }
        });
        assert!(!first.requested());
        std::thread::spawn(move || {
            registry.request();
            registry.request();
        })
        .join()
        .unwrap();
        assert!(first.requested());
        assert!(second.requested());
        assert_eq!(wakes.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn inactive_requests_do_not_leak_to_later_runs() {
        let registry = ExitRegistry(Mutex::new(Vec::new()));
        registry.request();
        let old = registry.register(|| {});
        assert!(!old.requested());
        registry.request();
        assert!(old.requested());
        drop(old);
        registry.request();
        let next = registry.register(|| {});
        assert!(!next.requested());
        assert_eq!(registry.0.lock().unwrap().len(), 1);
    }
}

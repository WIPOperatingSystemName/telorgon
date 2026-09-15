//! Reactive window service for shell components. Native objects remain host-owned.
use super::{Signal, SignalWriter};
use crate::shell::WindowId;
use std::{cell::RefCell, collections::VecDeque, rc::Rc};
#[derive(Clone, Default)]
pub struct ShellServiceRegistry(std::collections::HashMap<std::any::TypeId, Rc<dyn std::any::Any>>);
impl ShellServiceRegistry {
    pub fn insert<T: 'static>(&mut self, service: T) {
        self.0.insert(std::any::TypeId::of::<T>(), Rc::new(service));
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct ShellWindow {
    pub id: WindowId,
    pub title: String,
    pub active: bool,
    pub minimized: bool,
    pub maximized: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellWindowAction {
    Activate,
    SetMinimized(bool),
    SetMaximized(bool),
    Close,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellServiceError {
    Unavailable,
    Stale,
    Busy,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellRequestOutcome {
    Dispatched,
    Stale,
    Denied,
    Failed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShellRequestCompletion {
    pub id: u64,
    pub outcome: ShellRequestOutcome,
}
#[derive(Clone)]
pub struct ShellServices {
    windows: Signal<Vec<ShellWindow>>,
    completions: Signal<Vec<ShellRequestCompletion>>,
    queue: Rc<RefCell<ServiceQueue>>,
    extensions: ShellServiceRegistry,
}
struct ServiceQueue {
    alive: bool,
    next: u64,
    requests: VecDeque<WindowCommand>,
}
#[derive(Clone, Copy)]
pub(crate) struct WindowCommand {
    pub id: u64,
    pub window: WindowId,
    pub action: ShellWindowAction,
}
pub(crate) struct ShellServiceHost {
    pub services: ShellServices,
    windows: SignalWriter<Vec<ShellWindow>>,
    completions: SignalWriter<Vec<ShellRequestCompletion>>,
}
impl ShellServices {
    /// Resolves an environment-installed service without giving the widget ownership of the host.
    pub fn service<T: 'static>(&self) -> Result<Rc<T>, ShellServiceError> {
        if !self.queue.borrow().alive {
            return Err(ShellServiceError::Unavailable);
        }
        self.extensions
            .0
            .get(&std::any::TypeId::of::<T>())
            .cloned()
            .and_then(|s| s.downcast::<T>().ok())
            .ok_or(ShellServiceError::Unavailable)
    }
    pub fn windows(&self) -> &Signal<Vec<ShellWindow>> {
        &self.windows
    }
    pub fn completions(&self) -> &Signal<Vec<ShellRequestCompletion>> {
        &self.completions
    }
    /// Admission only; observable window state comes from subsequent publications.
    pub fn request(
        &self,
        window: WindowId,
        action: ShellWindowAction,
    ) -> Result<u64, ShellServiceError> {
        let mut q = self.queue.borrow_mut();
        if !q.alive {
            return Err(ShellServiceError::Unavailable);
        }
        if !self.windows.snapshot().iter().any(|w| w.id == window) {
            return Err(ShellServiceError::Stale);
        }
        if q.requests.len() >= 256 {
            return Err(ShellServiceError::Busy);
        }
        let id = q.next;
        q.next = q.next.checked_add(1).ok_or(ShellServiceError::Busy)?;
        q.requests.push_back(WindowCommand { id, window, action });
        Ok(id)
    }
}
impl ShellServiceHost {
    #[cfg(test)]
    pub fn new() -> Self {
        Self::with_registry(ShellServiceRegistry::default())
    }
    pub fn with_registry(extensions: ShellServiceRegistry) -> Self {
        let (windows, writer) = Signal::new(Vec::new());
        let (completions, cwriter) = Signal::new(Vec::new());
        Self {
            services: ShellServices {
                windows,
                completions,
                extensions,
                queue: Rc::new(RefCell::new(ServiceQueue {
                    alive: true,
                    next: 1,
                    requests: VecDeque::new(),
                })),
            },
            windows: writer,
            completions: cwriter,
        }
    }
    pub fn publish(&self, windows: Vec<ShellWindow>) {
        self.windows.publish_if_changed(windows);
    }
    pub fn drain(&self) -> Vec<WindowCommand> {
        self.services
            .queue
            .borrow_mut()
            .requests
            .drain(..)
            .collect()
    }
    pub fn complete(&self, id: u64, outcome: ShellRequestOutcome) {
        let mut values = self.services.completions.snapshot().to_vec();
        if values.len() >= 256 {
            values.remove(0);
        }
        values.push(ShellRequestCompletion { id, outcome });
        self.completions.publish_if_changed(values);
    }
}
impl Drop for ShellServiceHost {
    fn drop(&mut self) {
        let mut q = self.services.queue.borrow_mut();
        q.alive = false;
        q.requests.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroU32;
    fn window(generation: u32) -> ShellWindow {
        ShellWindow {
            id: WindowId::new(
                NonZeroU32::new(1).unwrap(),
                NonZeroU32::new(generation).unwrap(),
            ),
            title: "A".into(),
            active: false,
            minimized: false,
            maximized: false,
        }
    }
    #[test]
    fn requests_do_not_change_snapshot_and_retired_generations_fail() {
        let host = ShellServiceHost::new();
        host.publish(vec![window(1)]);
        let service = host.services.clone();
        let id = service
            .request(window(1).id, ShellWindowAction::SetMinimized(true))
            .unwrap();
        assert!(!service.windows().snapshot()[0].minimized);
        let commands = host.drain();
        assert_eq!(commands[0].id, id);
        host.publish(vec![window(2)]);
        assert_eq!(
            service.request(window(1).id, ShellWindowAction::Close),
            Err(ShellServiceError::Stale)
        );
        drop(host);
        assert_eq!(
            service.request(window(2).id, ShellWindowAction::Close),
            Err(ShellServiceError::Unavailable)
        );
    }
}

impl std::fmt::Debug for ShellServiceRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShellServiceRegistry")
            .field("count", &self.0.len())
            .finish()
    }
}

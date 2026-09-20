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
    /// Retained client content size in logical units, for aspect-ratio-aware previews.
    pub preview_size: Option<crate::foundation::SizeF>,
    pub application_id: Option<super::ApplicationId>,
    /// Raw Wayland app ID or X11 WM_CLASS, used for late metadata association.
    pub application_identity: String,
    pub(crate) icon: Option<crate::graphics::render::ImageResource>,
    pub(crate) icon_name: Option<String>,
    pub active: bool,
    pub minimized: bool,
    pub maximized: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellWindowAction {
    Snap(super::TileTarget),
    Float,
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
    output_size: Signal<crate::foundation::SizeF>,
    windows: Signal<Vec<ShellWindow>>,
    completions: Signal<Vec<ShellRequestCompletion>>,
    queue: Rc<RefCell<ServiceQueue>>,
    extensions: ShellServiceRegistry,
    applications: super::ApplicationCatalogHandle,
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
    output_size: SignalWriter<crate::foundation::SizeF>,
    _catalog_worker: super::applications::CatalogWorker,
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
        let mut registry = ShellServiceRegistry::default();
        registry.insert(super::ApplicationCatalog::in_memory(Vec::new()));
        Self::with_registry(registry)
    }
    pub fn with_registry(extensions: ShellServiceRegistry) -> Self {
        Self::with_registry_and_scale(extensions, 32)
    }
    pub fn with_registry_and_scale(extensions: ShellServiceRegistry, size: u32) -> Self {
        let config = extensions
            .0
            .get(&std::any::TypeId::of::<super::ApplicationCatalog>())
            .and_then(|v| v.downcast_ref::<super::ApplicationCatalog>())
            .cloned()
            .unwrap_or_default();
        let (applications, worker) = super::applications::start(config, size);
        let (output_size, output_writer) = Signal::new(crate::foundation::SizeF {
            width: 1.0,
            height: 1.0,
        });
        let (windows, writer) = Signal::new(Vec::new());
        let (completions, cwriter) = Signal::new(Vec::new());
        Self {
            output_size: output_writer,
            _catalog_worker: worker,
            services: ShellServices {
                output_size,
                applications,
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
    pub fn publish_output_size(&self, size: crate::foundation::SizeF) {
        self.output_size.publish_if_changed(size);
    }
    pub fn publish(&self, mut windows: Vec<ShellWindow>) {
        for window in &mut windows {
            // Protocol focus may lag title-bar minimization. A hidden window must
            // never be advertised as the active taskbar target.
            window.active &= !window.minimized;
            window.application_id = self
                .services
                .applications
                .identify(&window.application_identity);
        }
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
            preview_size: None,
            id: WindowId::new(
                NonZeroU32::new(1).unwrap(),
                NonZeroU32::new(generation).unwrap(),
            ),
            title: "A".into(),
            application_id: None,
            application_identity: String::new(),
            icon: None,
            icon_name: None,
            active: false,
            minimized: false,
            maximized: false,
        }
    }
    #[test]
    fn minimized_windows_are_inactive_even_when_protocol_focus_lags() {
        let host = ShellServiceHost::new();
        let mut value = window(1);
        value.active = true;
        host.publish(vec![value.clone()]);
        assert!(host.services.windows().snapshot()[0].active);
        value.minimized = true;
        host.publish(vec![value.clone()]);
        let snapshot = host.services.windows().snapshot();
        assert!(snapshot[0].minimized);
        assert!(!snapshot[0].active);
        // Restoring is admitted immediately, without a preceding minimize request.
        let windows = ShellContext::new(host.services.clone()).windows();
        windows.activate(value.id).unwrap();
        let commands = host.drain();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].action, ShellWindowAction::Activate);
        value.minimized = false;
        host.publish(vec![value]);
        assert!(windows.open()[0].active);
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

/// Services owned by the enclosing shell environment, inherited by its components.
#[derive(Clone)]
pub struct ShellContext {
    services: ShellServices,
}
impl ShellContext {
    pub(crate) fn new(services: ShellServices) -> Self {
        Self { services }
    }
    /// Selected output extent in logical units. Reads in view are reactive.
    pub fn output_size(&self) -> crate::foundation::SizeF {
        *super::context::observe(&self.services.output_size)
    }
    pub fn windows(&self) -> ShellWindows {
        ShellWindows(self.services.clone())
    }
    pub fn applications(&self) -> super::ApplicationCatalogHandle {
        self.services.applications.clone()
    }
}
#[derive(Clone)]
pub struct ShellWindows(ShellServices);
impl ShellWindows {
    /// Cached managed windows, including minimized windows. Reads in view are reactive.
    pub fn open(&self) -> Vec<ShellWindow> {
        super::context::observe(&self.0.windows)
            .iter()
            .cloned()
            .map(|mut w| {
                w.application_id = self.0.applications.identify(&w.application_identity);
                w
            })
            .collect()
    }
    pub fn icon(&self, id: WindowId) -> crate::assets::ImageSource {
        self.resolve_icon(id, super::applications::IconRequest::new())
    }
    /// Resolve catalog artwork at the rendered logical size and output density.
    pub fn resolve_icon(
        &self,
        id: WindowId,
        request: super::applications::IconRequest,
    ) -> crate::assets::ImageSource {
        let windows = super::context::observe(&self.0.windows);
        if let Some(w) = windows.iter().find(|w| w.id == id) {
            if let Some(icon) = &w.icon {
                return super::context::bind_image(icon.clone());
            }
            if let Some(name) = &w.icon_name {
                let icon = self.0.applications.resolve_named_icon(name, request);
                if icon.image_id() != super::applications::fallback_image().image {
                    return icon;
                }
            }
            if let Some(app) = self.0.applications.identify(&w.application_identity) {
                return self.0.applications.resolve_icon(&app, request);
            }
        }
        super::context::bind_image(super::applications::fallback_image())
    }
    pub fn snap(&self, id: WindowId, target: super::TileTarget) -> Result<u64, ShellServiceError> {
        self.0.request(id, ShellWindowAction::Snap(target))
    }
    pub fn float(&self, id: WindowId) -> Result<u64, ShellServiceError> {
        self.0.request(id, ShellWindowAction::Float)
    }
    pub fn activate(&self, id: WindowId) -> Result<u64, ShellServiceError> {
        self.0.request(id, ShellWindowAction::Activate)
    }
    pub fn set_minimized(&self, id: WindowId, minimized: bool) -> Result<u64, ShellServiceError> {
        self.0
            .request(id, ShellWindowAction::SetMinimized(minimized))
    }
    pub fn close(&self, id: WindowId) -> Result<u64, ShellServiceError> {
        self.0.request(id, ShellWindowAction::Close)
    }
}

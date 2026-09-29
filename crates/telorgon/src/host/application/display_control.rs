//! Requests are executed by the KMS owner, never by an IPC or UI thread.
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayMode {
    pub width: i32,
    pub height: i32,
    pub refresh_millihertz: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DisplayConfiguration {
    /// Empty selects the connected output chosen by the host.
    pub connector: String,
    /// None selects the monitor's preferred mode.
    pub mode: Option<DisplayMode>,
    /// None selects automatic density; fixed values are 1.0–4.0.
    pub scale: Option<f32>,
}
impl DisplayConfiguration {
    pub fn validate(&self) -> Result<(), String> {
        if self.connector.len() > 128 {
            return Err("Monitor name is too long".into());
        }
        if self
            .scale
            .is_some_and(|s| !s.is_finite() || !(1.0..=4.0).contains(&s))
        {
            return Err("Display scale must be between 100% and 400%".into());
        }
        if self
            .mode
            .is_some_and(|m| m.width <= 0 || m.height <= 0 || m.refresh_millihertz == 0)
        {
            return Err("Invalid display mode".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DisplaySnapshot {
    pub ready: bool,
    pub connector: String,
    #[serde(default)]
    pub connector_name: Option<String>,
    #[serde(default)]
    pub monitor_name: Option<String>,
    #[serde(default)]
    pub manufacturer: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub product_code: Option<u16>,
    #[serde(default)]
    pub serial_number: Option<String>,
    #[serde(default)]
    pub physical_millimeters: Option<crate::foundation::SizeI>,
    pub modes: Vec<DisplayMode>,
    pub preferred_mode: Option<DisplayMode>,
    pub current: DisplayConfiguration,
    pub resolved_scale: f32,
    pub preview: Option<u64>,
    pub seconds_remaining: u64,
    pub error: Option<String>,
}

pub(crate) enum DisplayCommand {
    Preview(DisplayConfiguration),
    Confirm(u64),
    Revert(u64),
}
pub(crate) struct DisplayRequest {
    pub command: DisplayCommand,
    pub expires: Instant,
    pub reply: mpsc::SyncSender<Result<u64, String>>,
}
struct Inner {
    snapshot: DisplaySnapshot,
    queue: VecDeque<DisplayRequest>,
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
    deadline: Option<Instant>,
}

/// A bounded, thread-safe handle; blocking methods must run outside the UI/owner thread.
#[derive(Clone)]
pub struct DisplayControl {
    initial: DisplayConfiguration,
    inner: Arc<Mutex<Inner>>,
}
impl std::fmt::Debug for DisplayControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DisplayControl")
            .field("initial", &self.initial)
            .finish_non_exhaustive()
    }
}
impl PartialEq for DisplayControl {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}
impl DisplayControl {
    pub fn new(initial: DisplayConfiguration) -> Result<Self, String> {
        initial.validate()?;
        Ok(Self {
            initial,
            inner: Arc::new(Mutex::new(Inner {
                snapshot: DisplaySnapshot::default(),
                queue: VecDeque::new(),
                wake: None,
                deadline: None,
            })),
        })
    }
    pub fn snapshot(&self) -> DisplaySnapshot {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut snapshot = inner.snapshot.clone();
        snapshot.seconds_remaining = inner.deadline.map_or(0, |d| {
            d.saturating_duration_since(Instant::now())
                .as_secs()
                .saturating_add(1)
        });
        snapshot
    }
    pub fn preview(&self, configuration: DisplayConfiguration) -> Result<u64, String> {
        configuration.validate()?;
        self.request(DisplayCommand::Preview(configuration))
    }
    pub fn confirm(&self, token: u64) -> Result<(), String> {
        self.request(DisplayCommand::Confirm(token)).map(|_| ())
    }
    pub fn revert(&self, token: u64) -> Result<(), String> {
        self.request(DisplayCommand::Revert(token)).map(|_| ())
    }
    fn request(&self, command: DisplayCommand) -> Result<u64, String> {
        let (reply, result) = mpsc::sync_channel(1);
        let wake = {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if !inner.snapshot.ready {
                return Err("Display controller is not ready".into());
            }
            if inner.queue.len() >= 8 {
                return Err("Display controller is busy".into());
            }
            inner.queue.push_back(DisplayRequest {
                command,
                reply,
                expires: Instant::now() + Duration::from_secs(10),
            });
            inner.wake.clone()
        };
        if let Some(wake) = wake {
            wake();
        }
        result
            .recv_timeout(Duration::from_secs(12))
            .map_err(|_| "Display request timed out".to_owned())?
    }
    pub(crate) fn initial(&self) -> &DisplayConfiguration {
        &self.initial
    }
    pub(crate) fn has_requests(&self) -> bool {
        !self
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .queue
            .is_empty()
    }
    pub(crate) fn take_request(&self) -> Option<DisplayRequest> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .queue
            .pop_front()
    }
    pub(crate) fn connect(&self, wake: Arc<dyn Fn() + Send + Sync>) {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).wake = Some(wake);
    }
    pub(crate) fn publish(&self, snapshot: DisplaySnapshot, deadline: Option<Instant>) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.snapshot = snapshot;
        inner.deadline = deadline;
    }
    pub(crate) fn disconnect(&self) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.snapshot.ready = false;
        inner.queue.clear();
        inner.wake = None;
    }
}

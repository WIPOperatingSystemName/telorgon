//! Audio policy is shell-owned; delivery requires an explicitly negotiated client adapter.
use crate::shell::capture::CaptureSource;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioScope {
    SelectedApplication,
    Desktop,
}

#[derive(Clone)]
pub struct ShareAudio {
    pub(crate) window: AudioScope,
    pub(crate) monitor: AudioScope,
    pub(crate) exclude_requester: bool,
    pub(crate) microphone: bool,
    pub(crate) delivery: Option<Arc<dyn PortalAudioDelivery>>,
}
impl Default for ShareAudio {
    fn default() -> Self {
        Self::new()
    }
}
impl ShareAudio {
    pub const fn new() -> Self {
        Self {
            window: AudioScope::SelectedApplication,
            monitor: AudioScope::Desktop,
            exclude_requester: true,
            microphone: false,
            delivery: None,
        }
    }
    pub fn window_mode(mut self, scope: AudioScope) -> Self {
        self.window = scope;
        self
    }
    pub fn monitor_mode(mut self, scope: AudioScope) -> Self {
        self.monitor = scope;
        self
    }
    pub fn exclude_requesting_application(mut self, exclude: bool) -> Self {
        self.exclude_requester = exclude;
        self
    }
    pub fn include_microphone(mut self, include: bool) -> Self {
        self.microphone = include;
        self
    }
    /// Install a trusted adapter only for clients with a negotiated audio handoff. Ordinary
    /// ScreenCast clients do not negotiate audio and must return false from `available`.
    pub fn delivery(mut self, adapter: impl PortalAudioDelivery) -> Self {
        self.delivery = Some(Arc::new(adapter));
        self
    }
    pub fn scope(&self, source: CaptureSource) -> AudioScope {
        match source {
            CaptureSource::Window(_) => self.window,
            _ => self.monitor,
        }
    }
    pub fn excludes_requester(&self) -> bool {
        self.exclude_requester
    }
    pub fn includes_microphone(&self) -> bool {
        self.microphone
    }
    pub(crate) fn available(&self, request: u64, app: &str) -> bool {
        self.delivery
            .as_ref()
            .is_some_and(|adapter| adapter.available(request, app))
    }
}

#[derive(Clone, Debug)]
pub struct PortalAudioRequest {
    pub request: u64,
    pub application_id: String,
    pub sources: Vec<(CaptureSource, u64)>,
}
/// Trusted shell adapter: authenticate the client and resolve its playback identity through
/// the negotiated transport. Application display names alone are not proof of identity.
/// All methods run on the compositor owner thread and must return without blocking.
pub trait PortalAudioDelivery: Send + Sync + 'static {
    fn available(&self, request: u64, application_id: &str) -> bool;
    /// Called only after source consent has been validated. The returned owner must include
    /// the audio routes and client handoff; creating an unconsumed mix is not success.
    fn start(
        &self,
        request: &PortalAudioRequest,
        policy: &ShareAudio,
    ) -> Result<Box<dyn PortalAudioSession>, String>;
}
pub trait PortalAudioSession: 'static {
    /// True only when routing and client delivery are ready; errors revoke the whole share.
    fn poll(&mut self) -> Result<bool, String>;
    fn stop(&mut self);
}

pub(crate) struct PortalAudioLease {
    session: Box<dyn PortalAudioSession>,
    deadline: std::time::Instant,
    ready: bool,
}
impl PortalAudioLease {
    pub fn start(policy: &ShareAudio, request: PortalAudioRequest) -> Result<Self, String> {
        let adapter = policy
            .delivery
            .as_ref()
            .filter(|_| policy.available(request.request, &request.application_id))
            .ok_or_else(|| {
                "This application has no negotiated screen-share audio delivery".to_owned()
            })?;
        Ok(Self {
            session: adapter.start(&request, policy)?,
            deadline: std::time::Instant::now() + std::time::Duration::from_secs(5),
            ready: false,
        })
    }
    pub fn poll(&mut self) -> Result<bool, String> {
        let ready = self.session.poll()?;
        if !ready && (self.ready || std::time::Instant::now() >= self.deadline) {
            return Err("Screen-share audio delivery became unavailable".into());
        }
        self.ready |= ready;
        Ok(ready)
    }
}
impl Drop for PortalAudioLease {
    fn drop(&mut self) {
        self.session.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    struct Adapter {
        negotiated: Arc<AtomicBool>,
        stops: Arc<AtomicUsize>,
    }
    struct Session(Arc<AtomicUsize>);
    impl PortalAudioSession for Session {
        fn poll(&mut self) -> Result<bool, String> {
            Ok(true)
        }
        fn stop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    impl PortalAudioDelivery for Adapter {
        fn available(&self, id: u64, app: &str) -> bool {
            id == 7 && app == "recorder" && self.negotiated.load(Ordering::SeqCst)
        }
        fn start(
            &self,
            _: &PortalAudioRequest,
            _: &ShareAudio,
        ) -> Result<Box<dyn PortalAudioSession>, String> {
            Ok(Box::new(Session(self.stops.clone())))
        }
    }
    #[test]
    fn audio_requires_negotiation_and_is_stopped_when_its_share_owner_drops() {
        let negotiated = Arc::new(AtomicBool::new(true));
        let stops = Arc::new(AtomicUsize::new(0));
        let policy = ShareAudio::new().delivery(Adapter {
            negotiated: negotiated.clone(),
            stops: stops.clone(),
        });
        let request = PortalAudioRequest {
            request: 7,
            application_id: "recorder".into(),
            sources: vec![(CaptureSource::Output(crate::shell::OutputId::MIN), 1)],
        };
        assert!(!ShareAudio::new().available(7, "recorder"));
        assert!(!policy.available(8, "recorder"));
        assert!(!policy.available(7, "other"));
        {
            let mut lease = PortalAudioLease::start(&policy, request.clone()).unwrap();
            assert_eq!(lease.poll(), Ok(true));
        }
        assert_eq!(stops.load(Ordering::SeqCst), 1);
        negotiated.store(false, Ordering::SeqCst);
        assert!(PortalAudioLease::start(&policy, request).is_err());
        assert_eq!(stops.load(Ordering::SeqCst), 1);
    }
}

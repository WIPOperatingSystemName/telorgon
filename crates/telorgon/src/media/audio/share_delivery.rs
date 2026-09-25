//! Explicit handoff registration for clients that negotiate PipeWire audio input ports.
use super::{AudioShareMix, AudioShareResolver};
use crate::host::application::{
    PortalAudioDelivery, PortalAudioRequest, PortalAudioSession, ShareAudio,
};
use crate::integrations::pipewire::{ConnectionHandle, ConnectionState, MediaError};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

struct Handoff {
    alive: Arc<AtomicBool>,
    resolver: Option<Box<dyn AudioShareResolver>>,
}
/// Cloneable adapter installed with `ShareAudio::delivery`. Register a handoff only after
/// authenticating a client's session and negotiating its stereo capture ports. This API does
/// not invent an audio extension to the standard XDG ScreenCast protocol.
#[derive(Clone)]
pub struct PipeWireAudioDelivery {
    connection: ConnectionHandle,
    handoffs: Arc<Mutex<BTreeMap<(u64, String), Handoff>>>,
}
/// Owned by the client's negotiated audio transport. Drop revokes pending and active delivery.
pub struct AudioShareHandoff(Arc<AtomicBool>);
impl Drop for AudioShareHandoff {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
impl PipeWireAudioDelivery {
    pub fn new(connection: ConnectionHandle) -> Self {
        Self {
            connection,
            handoffs: Arc::default(),
        }
    }
    /// Registration enables the option, never capture itself. Source consent is still required.
    /// The resolver must identify all requesting-app playback nodes on each registry update,
    /// and return only the input ports negotiated with this authenticated client.
    pub fn register_handoff(
        &self,
        request: u64,
        application_id: String,
        resolver: impl AudioShareResolver,
    ) -> Result<AudioShareHandoff, MediaError> {
        if request == 0 || application_id.is_empty() || application_id.len() > 255 {
            return Err(MediaError::InvalidArgument("audio share handoff identity"));
        }
        let mut handoffs = self.handoffs.lock().map_err(|_| MediaError::Disconnected)?;
        handoffs.retain(|_, handoff| handoff.alive.load(Ordering::Acquire));
        let key = (request, application_id);
        if handoffs.contains_key(&key) || handoffs.len() >= 8 {
            return Err(MediaError::ResourceLimit("audio share handoffs"));
        }
        let alive = Arc::new(AtomicBool::new(true));
        handoffs.insert(
            key,
            Handoff {
                alive: alive.clone(),
                resolver: Some(Box::new(resolver)),
            },
        );
        Ok(AudioShareHandoff(alive))
    }
}
impl PortalAudioDelivery for PipeWireAudioDelivery {
    fn available(&self, request: u64, application_id: &str) -> bool {
        self.connection.state() == ConnectionState::Ready
            && self.handoffs.lock().is_ok_and(|handoffs| {
                handoffs
                    .get(&(request, application_id.to_owned()))
                    .is_some_and(|handoff| {
                        handoff.alive.load(Ordering::Acquire) && handoff.resolver.is_some()
                    })
            })
    }
    fn start(
        &self,
        request: &PortalAudioRequest,
        policy: &ShareAudio,
    ) -> Result<Box<dyn PortalAudioSession>, String> {
        let (resolver, alive) = {
            let mut handoffs = self
                .handoffs
                .lock()
                .map_err(|_| "audio handoff lock poisoned")?;
            let handoff = handoffs
                .get_mut(&(request.request, request.application_id.clone()))
                .filter(|handoff| handoff.alive.load(Ordering::Acquire))
                .ok_or("audio handoff was revoked")?;
            (
                handoff
                    .resolver
                    .take()
                    .ok_or("audio handoff already consumed")?,
                handoff.alive.clone(),
            )
        };
        let mix = AudioShareMix::open(
            self.connection.clone(),
            policy.clone(),
            request.clone(),
            BoxedResolver(resolver),
        )
        .map_err(|error| {
            alive.store(false, Ordering::Release);
            error.to_string()
        })?;
        Ok(Box::new(DeliverySession { mix, alive }))
    }
}
struct BoxedResolver(Box<dyn AudioShareResolver>);
impl AudioShareResolver for BoxedResolver {
    fn resolve(
        &mut self,
        snapshot: &crate::integrations::pipewire::RegistrySnapshot,
        request: &PortalAudioRequest,
    ) -> Result<super::AudioShareEndpoints, MediaError> {
        self.0.resolve(snapshot, request)
    }
}
struct DeliverySession {
    mix: AudioShareMix,
    alive: Arc<AtomicBool>,
}
impl PortalAudioSession for DeliverySession {
    fn poll(&mut self) -> Result<bool, String> {
        if !self.alive.load(Ordering::Acquire) {
            self.mix.stop();
            return Err("audio handoff was revoked".into());
        }
        self.mix.poll()
    }
    fn stop(&mut self) {
        self.alive.store(false, Ordering::Release);
        self.mix.stop();
    }
}
impl Drop for DeliverySession {
    fn drop(&mut self) {
        self.stop();
    }
}

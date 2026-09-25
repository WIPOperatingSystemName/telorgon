//! Session-owned audio mix for a negotiated portal delivery adapter. No default-device routing.
use super::{AudioFilter, FilterConfig, FilterCycle, FilterProcessor, FilterState};
use crate::host::application::{AudioScope, PortalAudioRequest, PortalAudioSession, ShareAudio};
use crate::integrations::pipewire::graph::{Feedback, Graph, LinkLease, LinkState};
use crate::integrations::pipewire::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

const MAX_STREAMS: usize = 32;

/// A trusted adapter resolves application/window identity and its negotiated consumer against
/// each fresh registry snapshot. Node names and application labels are not authentication.
/// Return an error if a selected window or requesting application's identity is ambiguous.
pub trait AudioShareResolver: Send + 'static {
    fn resolve(
        &mut self,
        snapshot: &RegistrySnapshot,
        request: &PortalAudioRequest,
    ) -> Result<AudioShareEndpoints, MediaError>;
}
pub struct AudioShareEndpoints {
    /// All playback nodes belonging to the requesting application, including helper processes.
    pub requesting_playback: BTreeSet<ObjectHandle>,
    /// All playback nodes belonging to the selected windows' applications.
    pub selected_playback: BTreeSet<ObjectHandle>,
    /// Explicit microphone node, required only if the shell opted into microphone capture.
    pub microphone: Option<ObjectHandle>,
    /// Authenticated client's negotiated left/right input ports. Never its playback ports.
    pub consumer: [ObjectHandle; 2],
}

struct Route {
    slot: usize,
    ports: [ObjectHandle; 2],
    links: Vec<LinkLease>,
}
pub struct AudioShareMix {
    connection: ConnectionHandle,
    policy: ShareAudio,
    request: PortalAudioRequest,
    resolver: Box<dyn AudioShareResolver>,
    filter: AudioFilter,
    routes: BTreeMap<ObjectHandle, Route>,
    retiring: Vec<Route>,
    delivery: Vec<LinkLease>,
    consumer: Option<[ObjectHandle; 2]>,
    audible: Arc<AtomicU32>,
    stopped: bool,
}
impl AudioShareMix {
    /// Open only after portal consent, on a control thread. Does not alter any existing links.
    pub fn open(
        connection: ConnectionHandle,
        policy: ShareAudio,
        request: PortalAudioRequest,
        resolver: impl AudioShareResolver,
    ) -> Result<Self, MediaError> {
        if request.sources.is_empty() || request.sources.len() > 8 {
            return Err(MediaError::InvalidArgument("audio share sources"));
        }
        let audible = Arc::new(AtomicU32::new(0));
        let filter = AudioFilter::open(
            connection.clone(),
            FilterConfig {
                name: format!("Telorgon share {}", request.request),
                inputs: (0..MAX_STREAMS * 2).map(|i| format!("input_{i}")).collect(),
                outputs: vec!["output_FL".into(), "output_FR".into()],
                max_quantum: 8192,
                latency_frames: 0,
                start_paused: false,
            },
            MixProcessor(audible.clone()),
        )?;
        Ok(Self {
            connection,
            policy,
            request,
            resolver: Box::new(resolver),
            filter,
            routes: BTreeMap::new(),
            retiring: Vec::new(),
            delivery: Vec::new(),
            consumer: None,
            audible,
            stopped: false,
        })
    }
    fn advance(&mut self) -> Result<bool, MediaError> {
        if self.stopped {
            return Err(MediaError::Disconnected);
        }
        let snapshot = self.connection.snapshot();
        if snapshot.state != ConnectionState::Ready {
            return Err(MediaError::Disconnected);
        }
        if let FilterState::Failed(error) = self.filter.state() {
            return Err(error);
        }
        let endpoints = self.resolver.resolve(&snapshot, &self.request)?;
        let desired = selected_nodes(&snapshot, &self.policy, &self.request, &endpoints)?;
        let Some(node) = self.filter.node() else {
            return Ok(false);
        };
        let find = |name: &str, direction: &str| {
            snapshot
                .objects_of_kind(ObjectKind::Port)
                .find(|port| {
                    port.properties
                        .get("node.id")
                        .and_then(|s| s.parse::<u32>().ok())
                        == Some(node.id())
                        && port.properties.get("port.name").map(String::as_str) == Some(name)
                        && port.properties.get("port.direction").map(String::as_str)
                            == Some(direction)
                })
                .map(|port| port.handle)
        };
        let Some(left) = find("output_FL", "out") else {
            return Ok(false);
        };
        let Some(right) = find("output_FR", "out") else {
            return Ok(false);
        };
        if self.consumer.is_some_and(|old| old != endpoints.consumer) {
            return Err(MediaError::StaleHandle); // Re-negotiate, never silently redirect a running share.
        }
        let graph = Graph::new(self.connection.clone());
        for port in endpoints.consumer {
            validate_port(&snapshot, port, "in")?;
        }
        if self.consumer.is_none() {
            self.delivery
                .push(graph.link(left, endpoints.consumer[0], Feedback::Reject)?);
            self.delivery
                .push(graph.link(right, endpoints.consumer[1], Feedback::Reject)?);
            self.consumer = Some(endpoints.consumer);
        }
        let mut ports = BTreeMap::new();
        for source in &desired {
            if let Some(pair) = stereo_ports(&snapshot, *source)? {
                ports.insert(*source, pair);
            }
        }
        let obsolete: Vec<_> = self
            .routes
            .iter()
            .filter(|(source, route)| ports.get(source) != Some(&route.ports))
            .map(|(source, _)| *source)
            .collect();
        for source in obsolete {
            let route = self.routes.remove(&source).unwrap();
            // Silence first: link destruction is asynchronous on the PipeWire control worker.
            self.audible.fetch_and(!(1 << route.slot), Ordering::AcqRel);
            for link in &route.links {
                link.remove();
            }
            self.retiring.push(route);
        }
        self.retiring.retain(|route| {
            route
                .links
                .iter()
                .any(|link| !matches!(link.state(), LinkState::Removed | LinkState::Failed(_)))
        });
        let used: BTreeSet<_> = self
            .routes
            .values()
            .chain(self.retiring.iter())
            .map(|r| r.slot)
            .collect();
        let mut free = (0..MAX_STREAMS).filter(|slot| !used.contains(slot));
        for (source, ports) in ports {
            if self.routes.contains_key(&source) {
                continue;
            }
            let Some(slot) = free.next() else {
                break;
            };
            let Some(left) = find(&format!("input_{}", slot * 2), "in") else {
                return Ok(false);
            };
            let Some(right) = find(&format!("input_{}", slot * 2 + 1), "in") else {
                return Ok(false);
            };
            let links = vec![
                graph.link(ports[0], left, Feedback::Reject)?,
                graph.link(ports[1], right, Feedback::Reject)?,
            ];
            self.routes.insert(source, Route { slot, ports, links });
        }
        let mut mask = 0;
        let ready = links_ready(&self.delivery)?;
        for route in self.routes.values() {
            let connected = links_ready(&route.links)?;
            if connected {
                mask |= 1 << route.slot;
            }
        }
        // Do not emit audio until the negotiated consumer links exist.
        self.audible
            .store(if ready { mask } else { 0 }, Ordering::Release);
        Ok(ready)
    }
}
impl PortalAudioSession for AudioShareMix {
    fn poll(&mut self) -> Result<bool, String> {
        match self.advance() {
            Ok(ready) => Ok(ready),
            Err(error) => {
                self.stop();
                Err(error.to_string())
            }
        }
    }
    fn stop(&mut self) {
        self.audible.store(0, Ordering::Release);
        self.filter.stop();
        self.delivery.clear();
        self.routes.clear();
        self.retiring.clear();
        self.stopped = true;
    }
}
impl Drop for AudioShareMix {
    fn drop(&mut self) {
        self.stop();
    }
}

fn links_ready(links: &[LinkLease]) -> Result<bool, MediaError> {
    let mut ready = true;
    for link in links {
        match link.state() {
            LinkState::Active | LinkState::Paused => {}
            LinkState::Connecting => ready = false,
            LinkState::Failed(error) => return Err(error),
            LinkState::Removed => return Err(MediaError::StaleHandle),
        }
    }
    Ok(ready)
}
fn validate_port(
    snapshot: &RegistrySnapshot,
    handle: ObjectHandle,
    direction: &str,
) -> Result<(), MediaError> {
    let port = snapshot.resolve(handle)?;
    if port.kind != ObjectKind::Port
        || port.properties.get("port.direction").map(String::as_str) != Some(direction)
    {
        return Err(MediaError::InvalidArgument("audio share port direction"));
    }
    Ok(())
}
fn stereo_ports(
    snapshot: &RegistrySnapshot,
    node: ObjectHandle,
) -> Result<Option<[ObjectHandle; 2]>, MediaError> {
    snapshot.resolve(node)?;
    let mut channels = BTreeMap::new();
    for port in snapshot.objects_of_kind(ObjectKind::Port).filter(|port| {
        port.properties
            .get("node.id")
            .and_then(|s| s.parse::<u32>().ok())
            == Some(node.id())
            && port.properties.get("port.direction").map(String::as_str) == Some("out")
    }) {
        let channel = port
            .properties
            .get("audio.channel")
            .ok_or(MediaError::Unsupported(
                "audio share port has no channel identity",
            ))?;
        if !matches!(channel.as_str(), "FL" | "FR" | "MONO")
            || channels.insert(channel.as_str(), port.handle).is_some()
        {
            return Err(MediaError::Unsupported(
                "audio share supports mono or stereo playback",
            ));
        }
    }
    if channels.contains_key("MONO") && channels.len() != 1 {
        return Err(MediaError::Unsupported(
            "ambiguous audio share channel layout",
        ));
    }
    if let Some(mono) = channels.get("MONO") {
        return Ok(Some([*mono, *mono]));
    }
    match (channels.get("FL"), channels.get("FR")) {
        (Some(left), Some(right)) => Ok(Some([*left, *right])),
        _ if channels.is_empty()
            || channels
                .keys()
                .all(|channel| matches!(*channel, "FL" | "FR")) =>
        {
            Ok(None)
        }
        _ => Err(MediaError::Unsupported(
            "audio share requires identifiable mono or stereo ports",
        )),
    }
}
fn selected_nodes(
    snapshot: &RegistrySnapshot,
    policy: &ShareAudio,
    request: &PortalAudioRequest,
    endpoints: &AudioShareEndpoints,
) -> Result<BTreeSet<ObjectHandle>, MediaError> {
    // Validate incarnation-bearing identities even when the corresponding node is excluded.
    for handle in endpoints
        .requesting_playback
        .iter()
        .chain(&endpoints.selected_playback)
    {
        if snapshot.resolve(*handle)?.media_class() != Some("Stream/Output/Audio") {
            return Err(MediaError::InvalidArgument(
                "audio share application playback identity",
            ));
        }
    }
    let desktop = request
        .sources
        .iter()
        .any(|(source, _)| policy.scope(*source) == AudioScope::Desktop);
    let mut selected: BTreeSet<_> = snapshot
        .objects_of_kind(ObjectKind::Node)
        .filter(|node| {
            node.media_class() == Some("Stream/Output/Audio")
                && (desktop || endpoints.selected_playback.contains(&node.handle))
                && (!policy.excludes_requester()
                    || !endpoints.requesting_playback.contains(&node.handle))
        })
        .map(|node| node.handle)
        .collect();
    if policy.includes_microphone() {
        let mic = endpoints.microphone.ok_or(MediaError::Unsupported(
            "microphone requires explicit selection",
        ))?;
        if snapshot.resolve(mic)?.media_class() != Some("Audio/Source") {
            return Err(MediaError::InvalidArgument("microphone source"));
        }
        selected.insert(mic);
    }
    if selected.len() > MAX_STREAMS {
        return Err(MediaError::ResourceLimit("audio share mix inputs"));
    }
    Ok(selected)
}
struct MixProcessor(Arc<AtomicU32>);
impl FilterProcessor for MixProcessor {
    fn process(&mut self, cycle: FilterCycle<'_>) {
        let mask = self.0.load(Ordering::Acquire);
        let FilterCycle {
            inputs,
            mut outputs,
            ..
        } = cycle;
        for channel in 0..2 {
            let Some(output) = outputs.channel(channel) else {
                continue;
            };
            output.fill(0.0);
            for slot in 0..MAX_STREAMS {
                if mask & (1 << slot) == 0 {
                    continue;
                }
                if let Some(input) = inputs.channel(slot * 2 + channel) {
                    for (out, sample) in output.iter_mut().zip(input) {
                        if sample.is_finite() {
                            *out += sample;
                        }
                    }
                }
            }
            for sample in output {
                *sample = sample.clamp(-1.0, 1.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::pipewire::RegistryState;
    fn registry() -> (RegistrySnapshot, AudioShareEndpoints, PortalAudioRequest) {
        let mut registry = RegistryState::new(1, false, 32);
        let mut insert = |id, class: &str| {
            registry
                .insert(
                    id,
                    ObjectKind::Node,
                    7,
                    BTreeMap::from([("media.class".into(), class.into())]),
                )
                .unwrap()
        };
        let discord = insert(1, "Stream/Output/Audio");
        let game = insert(2, "Stream/Output/Audio");
        insert(3, "Stream/Output/Audio");
        let microphone = insert(4, "Audio/Source");
        insert(5, "Audio/Sink");
        insert(6, "Stream/Input/Audio");
        let endpoints = AudioShareEndpoints {
            requesting_playback: BTreeSet::from([discord]),
            selected_playback: BTreeSet::from([game]),
            microphone: Some(microphone),
            consumer: [discord; 2],
        };
        let request = PortalAudioRequest {
            request: 7,
            application_id: "discord".into(),
            sources: vec![(
                crate::shell::capture::CaptureSource::Output(crate::shell::OutputId::MIN),
                1,
            )],
        };
        (registry.snapshot, endpoints, request)
    }
    #[test]
    fn desktop_mix_excludes_call_microphone_sinks_and_capture_streams() {
        let (snapshot, endpoints, request) = registry();
        let selected = selected_nodes(&snapshot, &ShareAudio::new(), &request, &endpoints).unwrap();
        assert_eq!(selected.iter().map(|h| h.id()).collect::<Vec<_>>(), [2, 3]);
        let selected = selected_nodes(
            &snapshot,
            &ShareAudio::new().include_microphone(true),
            &request,
            &endpoints,
        )
        .unwrap();
        assert_eq!(
            selected.iter().map(|h| h.id()).collect::<Vec<_>>(),
            [2, 3, 4]
        );
    }
    #[test]
    fn application_scope_never_falls_back_and_rejects_stale_identity() {
        let (mut snapshot, mut endpoints, request) = registry();
        let policy = ShareAudio::new().monitor_mode(AudioScope::SelectedApplication);
        assert_eq!(
            selected_nodes(&snapshot, &policy, &request, &endpoints).unwrap(),
            endpoints.selected_playback
        );
        endpoints.selected_playback.clear();
        assert!(
            selected_nodes(&snapshot, &policy, &request, &endpoints)
                .unwrap()
                .is_empty()
        );
        snapshot.objects.remove(&1);
        assert_eq!(
            selected_nodes(&snapshot, &policy, &request, &endpoints),
            Err(MediaError::StaleHandle)
        );
    }
    #[test]
    fn channel_routing_waits_for_ports_and_rejects_silent_surround_truncation() {
        let mut registry = RegistryState::new(1, false, 16);
        let node = registry
            .insert(
                1,
                ObjectKind::Node,
                7,
                BTreeMap::from([("media.class".into(), "Stream/Output/Audio".into())]),
            )
            .unwrap();
        assert_eq!(stereo_ports(&registry.snapshot, node).unwrap(), None);
        let mut port = |id, channel: &str| {
            registry
                .insert(
                    id,
                    ObjectKind::Port,
                    7,
                    BTreeMap::from([
                        ("node.id".into(), "1".into()),
                        ("port.direction".into(), "out".into()),
                        ("audio.channel".into(), channel.into()),
                    ]),
                )
                .unwrap()
        };
        let left = port(2, "FL");
        let right = port(3, "FR");
        assert_eq!(
            stereo_ports(&registry.snapshot, node).unwrap(),
            Some([left, right])
        );
        registry
            .insert(
                4,
                ObjectKind::Port,
                7,
                BTreeMap::from([
                    ("node.id".into(), "1".into()),
                    ("port.direction".into(), "out".into()),
                    ("audio.channel".into(), "FC".into()),
                ]),
            )
            .unwrap();
        assert!(stereo_ports(&registry.snapshot, node).is_err());
    }
    #[test]
    fn realtime_mix_sums_stereo_and_silences_revoked_routes_before_unlink() {
        use super::super::{FilterClock, FilterInputs, FilterOutputs};
        let mask = Arc::new(AtomicU32::new(3));
        let mut processor = MixProcessor(mask.clone());
        let input = [0.2, 0.3, 0.4, 0.1]; // two stereo sources, one frame
        let mut output = [0.0; 2];
        processor.process(FilterCycle {
            clock: FilterClock::default(),
            inputs: FilterInputs {
                storage: &input,
                stride: 1,
                frames: 1,
            },
            outputs: FilterOutputs {
                storage: &mut output,
                stride: 1,
                frames: 1,
            },
        });
        assert!((output[0] - 0.6).abs() < 0.0001);
        assert!((output[1] - 0.4).abs() < 0.0001);
        mask.store(1, Ordering::Release);
        processor.process(FilterCycle {
            clock: FilterClock::default(),
            inputs: FilterInputs {
                storage: &input,
                stride: 1,
                frames: 1,
            },
            outputs: FilterOutputs {
                storage: &mut output,
                stride: 1,
                frames: 1,
            },
        });
        assert_eq!(output, [0.2, 0.3]);
    }
}

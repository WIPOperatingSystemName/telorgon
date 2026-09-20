use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use crate::runtime::instrumentation::{Event, EventKind, LaneInfo, ViewInfo};
use serde::Serialize;

use crate::services::profiler::SessionMetadata;
use crate::services::profiler::protocol::{
    PROTOCOL_MAJOR, PROTOCOL_MINOR, StoredEvent, StoredLabel, describe_event, describe_parent_span,
    encode_capture, encode_events, encode_labels,
};

pub(crate) const DEFAULT_SESSION_BYTES: usize = 32 * 1024 * 1024;
pub(crate) const DEFAULT_RETAINED_FRAMES: usize = 600;
const MAX_LABELS: usize = 4_096;
const STORED_EVENT_BYTES: usize = std::mem::size_of::<StoredEvent>();

#[derive(Clone)]
pub(crate) struct LiveBatch {
    pub metadata_json: Option<Arc<str>>,
    pub labels: Option<Arc<[u8]>>,
    pub events: Arc<[u8]>,
}

pub(crate) struct Snapshot {
    pub metadata_json: String,
    pub labels: Vec<u8>,
    pub events: Vec<u8>,
}

#[derive(Serialize)]
struct LaneMetadata<'a> {
    id: u32,
    name: &'a str,
}

#[derive(Serialize)]
struct ViewMetadata<'a> {
    id: u64,
    role: &'a str,
}

#[derive(Serialize)]
struct MetadataEnvelope<'a> {
    message: &'static str,
    protocol_major: u16,
    protocol_minor: u16,
    session: &'a SessionMetadata,
    lanes: Vec<LaneMetadata<'a>>,
    views: Vec<ViewMetadata<'a>>,
    retained_events: usize,
    retained_bytes: usize,
    retained_frame_limit: usize,
    event_byte_limit: usize,
}

pub(crate) struct SessionStore {
    metadata: SessionMetadata,
    labels: HashMap<
        (
            &'static str,
            crate::services::profiler::protocol::LabelDescriptor,
        ),
        StoredLabel,
    >,
    labels_by_id: Vec<StoredLabel>,
    lanes: Vec<LaneInfo>,
    views: Vec<ViewInfo>,
    events: VecDeque<StoredEvent>,
    completed_frames: VecDeque<(u64, u64)>,
    byte_limit: usize,
    frame_limit: usize,
}

impl SessionStore {
    pub fn new(metadata: SessionMetadata) -> Self {
        Self {
            metadata,
            labels: HashMap::new(),
            labels_by_id: Vec::new(),
            lanes: Vec::new(),
            views: Vec::new(),
            events: VecDeque::new(),
            completed_frames: VecDeque::new(),
            byte_limit: DEFAULT_SESSION_BYTES,
            frame_limit: DEFAULT_RETAINED_FRAMES,
        }
    }

    pub fn append(
        &mut self,
        incoming: &[Event],
        lanes: Vec<LaneInfo>,
        views: Vec<ViewInfo>,
    ) -> Option<LiveBatch> {
        let lanes_changed = self.lanes != lanes;
        let views_changed = self.views != views;
        self.lanes = lanes;
        self.views = views;
        if incoming.is_empty() {
            return None;
        }
        let mut newly_interned = Vec::new();
        let mut stored = Vec::with_capacity(incoming.len());
        let mut metadata_changed = false;
        for event in incoming.iter().copied() {
            metadata_changed |= self.update_metadata_for_event(event);
            let label_id = self.intern(event.label, describe_event(event), &mut newly_interned);
            let parent_label_id = event.parent_label.map_or(0, |label| {
                self.intern(label, describe_parent_span(label), &mut newly_interned)
            });
            let stored_event = StoredEvent {
                event,
                label_id,
                parent_label_id,
            };
            if event.kind == EventKind::FrameEnd
                && let Some(frame) = event.frame
            {
                self.completed_frames
                    .push_back((frame.get(), event.sequence));
            }
            self.events.push_back(stored_event);
            stored.push(stored_event);
        }
        self.enforce_bounds();
        Some(LiveBatch {
            metadata_json: (lanes_changed || views_changed || metadata_changed)
                .then(|| Arc::<str>::from(self.metadata_json())),
            labels: (!newly_interned.is_empty())
                .then(|| Arc::<[u8]>::from(encode_labels(&newly_interned))),
            events: Arc::<[u8]>::from(encode_events(&stored)),
        })
    }

    pub fn snapshot(&self) -> Snapshot {
        let labels = self.labels_by_id.to_vec();
        let events = self.events.iter().copied().collect::<Vec<_>>();
        Snapshot {
            metadata_json: self.metadata_json(),
            labels: encode_labels(&labels),
            events: encode_events(&events),
        }
    }

    pub fn metadata_json(&self) -> String {
        serde_json::to_string(&MetadataEnvelope {
            message: "metadata",
            protocol_major: PROTOCOL_MAJOR,
            protocol_minor: PROTOCOL_MINOR,
            session: &self.metadata,
            lanes: self
                .lanes
                .iter()
                .map(|lane| LaneMetadata {
                    id: lane.id,
                    name: &lane.name,
                })
                .collect(),
            views: self
                .views
                .iter()
                .map(|view| ViewMetadata {
                    id: view.id.get(),
                    role: view.role,
                })
                .collect(),
            retained_events: self.events.len(),
            retained_bytes: self.events.len() * STORED_EVENT_BYTES,
            retained_frame_limit: self.frame_limit,
            event_byte_limit: self.byte_limit,
        })
        .expect("profiler metadata contains only serializable bounded fields")
    }

    pub fn capture(&self) -> Vec<u8> {
        let snapshot = self.snapshot();
        encode_capture(
            snapshot.metadata_json.as_bytes(),
            &snapshot.labels,
            &snapshot.events,
        )
    }

    fn intern(
        &mut self,
        label: &'static str,
        descriptor: crate::services::profiler::protocol::LabelDescriptor,
        newly_interned: &mut Vec<StoredLabel>,
    ) -> u32 {
        if let Some(stored) = self.labels.get(&(label, descriptor)) {
            return stored.id;
        }
        if self.labels_by_id.len() >= MAX_LABELS || label.len() > usize::from(u16::MAX) {
            return 0;
        }
        let id = (self.labels_by_id.len() + 1) as u32;
        let stored = StoredLabel {
            id,
            label,
            descriptor,
        };
        self.labels.insert((label, descriptor), stored);
        self.labels_by_id.push(stored);
        newly_interned.push(stored);
        id
    }

    fn enforce_bounds(&mut self) {
        while self.completed_frames.len() > self.frame_limit {
            let Some((_, end_sequence)) = self.completed_frames.pop_front() else {
                break;
            };
            self.remove_through(end_sequence);
        }
        while self.events.len() * STORED_EVENT_BYTES > self.byte_limit {
            if let Some((_, end_sequence)) = self.completed_frames.pop_front() {
                self.remove_through(end_sequence);
            } else {
                self.events.pop_front();
            }
        }
        let oldest_sequence = self
            .events
            .front()
            .map_or(u64::MAX, |event| event.event.sequence);
        while self
            .completed_frames
            .front()
            .is_some_and(|(_, sequence)| *sequence < oldest_sequence)
        {
            self.completed_frames.pop_front();
        }
    }

    fn remove_through(&mut self, sequence: u64) {
        while self
            .events
            .front()
            .is_some_and(|event| event.event.sequence <= sequence)
        {
            self.events.pop_front();
        }
    }

    fn update_metadata_for_event(&mut self, event: Event) -> bool {
        match event.label {
            "renderer.vulkan" => replace_if_different(&mut self.metadata.renderer, "Vulkan"),
            "renderer.software" | "renderer.software_fallback" => {
                replace_if_different(&mut self.metadata.renderer, "software")
            }
            "gpu.timestamps.available" => {
                let mut changed =
                    push_unique(&mut self.metadata.capabilities, "gpu-relative-timestamps");
                let before = self.metadata.unavailable_metrics.len();
                self.metadata
                    .unavailable_metrics
                    .retain(|metric| metric != "gpu-relative-timestamps");
                changed |= before != self.metadata.unavailable_metrics.len();
                changed
            }
            "gpu.timestamps.unavailable" => push_unique(
                &mut self.metadata.unavailable_metrics,
                "gpu-relative-timestamps",
            ),
            _ => false,
        }
    }
}

fn replace_if_different(destination: &mut String, value: &str) -> bool {
    if destination == value {
        return false;
    }
    value.clone_into(destination);
    true
}

fn push_unique(values: &mut Vec<String>, value: &str) -> bool {
    if values.iter().any(|candidate| candidate == value) {
        return false;
    }
    values.push(value.to_owned());
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::instrumentation::{EventKind, ProfileFrameId, ProfileViewId, TimingDomain, ViewInfo};

    fn event(sequence: u64, label: &'static str) -> Event {
        Event {
            sequence,
            timestamp_ns: sequence,
            duration_ns: 1,
            lane: 1,
            view: None,
            frame: None,
            presentation: None,
            kind: EventKind::Span,
            timing_domain: TimingDomain::Cpu,
            label,
            parent_label: None,
            value: 0,
            auxiliary: 0,
        }
    }

    #[test]
    fn new_labels_are_sent_once_and_snapshots_retain_them() {
        let mut store = SessionStore::new(SessionMetadata::for_tests());
        let first = store
            .append(&[event(1, "frame.total")], Vec::new(), Vec::new())
            .unwrap();
        assert!(first.labels.is_some());
        let second = store
            .append(&[event(2, "frame.total")], Vec::new(), Vec::new())
            .unwrap();
        assert!(second.labels.is_none());
        let snapshot = store.snapshot();
        assert!(snapshot.labels.len() > 16);
        assert_eq!(
            snapshot.events.len(),
            16 + 2 * crate::services::profiler::protocol::EVENT_WIRE_BYTES
        );
    }

    #[test]
    fn capture_is_built_only_from_memory() {
        let mut store = SessionStore::new(SessionMetadata::for_tests());
        store.append(&[event(1, "frame.total")], Vec::new(), Vec::new());
        assert_eq!(&store.capture()[..4], b"LTPC");
    }

    #[test]
    fn frame_bound_evicts_complete_oldest_frames() {
        let mut store = SessionStore::new(SessionMetadata::for_tests());
        store.frame_limit = 2;
        for sequence in 1..=3 {
            let mut frame_end = event(sequence, "frame.total");
            frame_end.kind = EventKind::FrameEnd;
            frame_end.frame = ProfileFrameId::from_raw(sequence);
            store.append(&[frame_end], Vec::new(), Vec::new());
        }
        assert_eq!(store.events.len(), 2);
        assert_eq!(store.events.front().unwrap().event.sequence, 2);
    }

    #[test]
    fn capability_and_renderer_events_update_session_metadata() {
        let mut metadata = SessionMetadata::for_tests();
        metadata
            .unavailable_metrics
            .push("gpu-relative-timestamps".to_owned());
        let mut store = SessionStore::new(metadata);
        let batch = store
            .append(
                &[
                    event(1, "renderer.vulkan"),
                    event(2, "gpu.timestamps.available"),
                ],
                Vec::new(),
                Vec::new(),
            )
            .unwrap();
        assert!(batch.metadata_json.is_some());
        let metadata = store.metadata_json();
        assert!(metadata.contains("\"renderer\":\"Vulkan\""));
        assert!(metadata.contains("gpu-relative-timestamps"));
        assert!(!metadata.contains("\"unavailable_metrics\":[\"gpu-relative-timestamps\"]"));
    }

    #[test]
    fn registered_views_are_published_as_bounded_metadata() {
        let mut store = SessionStore::new(SessionMetadata::for_tests());
        let batch = store
            .append(
                &[event(1, "frame.total")],
                Vec::new(),
                vec![ViewInfo {
                    id: ProfileViewId::PRIMARY,
                    role: "Application window",
                }],
            )
            .unwrap();
        let metadata = batch.metadata_json.unwrap();
        assert!(metadata.contains("\"views\":[{\"id\":1,\"role\":\"Application window\"}]"));
    }
}

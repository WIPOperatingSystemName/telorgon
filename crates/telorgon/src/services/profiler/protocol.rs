use crate::runtime::instrumentation::{Event, EventKind, TimingDomain};

pub(crate) const PROTOCOL_MAJOR: u16 = 3;
pub(crate) const PROTOCOL_MINOR: u16 = 0;
pub(crate) const EVENT_WIRE_BYTES: usize = 80;

const STREAM_MAGIC: &[u8; 4] = b"LTPR";
const CAPTURE_MAGIC: &[u8; 4] = b"LTPC";
const MESSAGE_LABELS: u8 = 1;
const MESSAGE_EVENTS: u8 = 2;
const MESSAGE_VIEWER_GAP: u8 = 3;

#[derive(Clone, Copy)]
pub(crate) struct StoredEvent {
    pub event: Event,
    pub label_id: u32,
    pub parent_label_id: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub(crate) enum MetricCategory {
    Other = 0,
    Runtime = 1,
    Input = 2,
    Theme = 3,
    Layout = 4,
    Scene = 5,
    Renderer = 6,
    Gpu = 7,
    Wait = 8,
    Diagnostic = 9,
    Presentation = 10,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub(crate) enum MetricUnit {
    None = 0,
    Duration = 1,
    Count = 2,
    Bytes = 3,
    Nanoseconds = 4,
    Area = 5,
    Scalar = 6,
    Identifier = 7,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub(crate) enum MetricAggregation {
    Event = 0,
    Gauge = 1,
    Sum = 2,
    Cumulative = 3,
}

pub(crate) const LABEL_FLAG_RESOURCE: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct LabelDescriptor {
    pub category: MetricCategory,
    pub unit: MetricUnit,
    pub aggregation: MetricAggregation,
    pub flags: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StoredLabel {
    pub id: u32,
    pub label: &'static str,
    pub descriptor: LabelDescriptor,
}

pub(crate) fn describe_event(event: Event) -> LabelDescriptor {
    describe_label(event.label, event.kind, event.timing_domain)
}

pub(crate) fn describe_parent_span(label: &'static str) -> LabelDescriptor {
    describe_label(label, EventKind::Span, TimingDomain::Cpu)
}

fn describe_label(label: &'static str, kind: EventKind, domain: TimingDomain) -> LabelDescriptor {
    let category = metric_category(label, kind, domain);
    let unit = metric_unit(label, kind);
    let aggregation = metric_aggregation(label, kind);
    let flags = if kind == EventKind::Counter && is_resource_metric(label, category, unit) {
        LABEL_FLAG_RESOURCE
    } else {
        0
    };
    LabelDescriptor {
        category,
        unit,
        aggregation,
        flags,
    }
}

fn metric_category(label: &str, kind: EventKind, domain: TimingDomain) -> MetricCategory {
    if domain == TimingDomain::GpuRelative || label.starts_with("gpu.") {
        MetricCategory::Gpu
    } else if matches!(kind, EventKind::Diagnostic | EventKind::Gap) {
        MetricCategory::Diagnostic
    } else if matches!(
        kind,
        EventKind::PresentationBegin | EventKind::PresentationEnd
    ) || [
        "presentation.",
        "responsiveness.",
        "presenter.",
        "swapchain.",
        "surface.",
    ]
    .iter()
    .any(|prefix| label.starts_with(prefix))
        || matches!(label, "queue.submit_present" | "worker.submit")
    {
        MetricCategory::Presentation
    } else if label.contains("wait")
        || label.contains("acquire")
        || label.contains("retry")
        || label.contains("backpressure")
        || label.contains("unavailable")
    {
        MetricCategory::Wait
    } else if label.starts_with("input.") {
        MetricCategory::Input
    } else if label.starts_with("theme.") {
        MetricCategory::Theme
    } else if label.starts_with("layout.") {
        MetricCategory::Layout
    } else if label.starts_with("scene.") || label.starts_with("delta.") {
        MetricCategory::Scene
    } else if [
        "render.",
        "renderer.",
        "vulkan.",
        "software.",
        "framebuffer.",
        "presenter.",
        "swapchain.",
        "surface.",
        "command.",
        "queue.",
        "uploads.",
        "descriptors.",
        "draws.",
        "barriers.",
        "transport.",
        "worker.",
    ]
    .iter()
    .any(|prefix| label.starts_with(prefix))
    {
        MetricCategory::Renderer
    } else if [
        "runtime.",
        "component.",
        "element.",
        "signals.",
        "tasks.",
        "host.",
        "frame.",
        "presentation.",
        "commands.",
        "embedded.",
    ]
    .iter()
    .any(|prefix| label.starts_with(prefix))
    {
        MetricCategory::Runtime
    } else {
        MetricCategory::Other
    }
}

fn metric_unit(label: &str, kind: EventKind) -> MetricUnit {
    if matches!(kind, EventKind::Span | EventKind::GpuSpanResolved) {
        MetricUnit::Duration
    } else if label.ends_with("_ns") {
        MetricUnit::Nanoseconds
    } else if kind != EventKind::Counter {
        MetricUnit::None
    } else if label.ends_with("_bytes") || label.ends_with(".bytes") {
        MetricUnit::Bytes
    } else if label.ends_with("_area") || label.ends_with(".area") {
        MetricUnit::Area
    } else if label.ends_with(".epoch")
        || label.ends_with(".revision")
        || matches!(
            label,
            "gpu.adapter.vendor_id"
                | "gpu.adapter.device_id"
                | "gpu.adapter.driver_version"
                | "gpu.adapter.api_version"
        )
    {
        MetricUnit::Identifier
    } else if label.ends_with(".period") {
        MetricUnit::Scalar
    } else {
        MetricUnit::Count
    }
}

fn metric_aggregation(label: &str, kind: EventKind) -> MetricAggregation {
    if kind != EventKind::Counter {
        return MetricAggregation::Event;
    }
    if label.starts_with("host.")
        || matches!(
            label,
            "theme.bindings.evaluated"
                | "theme.bindings.skipped"
                | "theme.entries.invalidated"
                | "theme.retargets"
        )
    {
        MetricAggregation::Cumulative
    } else if matches!(
        label,
        "frame.refresh_interval_ns"
            | "framebuffer.bytes"
            | "theme.animations.active"
            | "scene.delta.queue.high_water"
            | "scene.epoch"
            | "presenter.retired_swapchains"
    ) || label.starts_with("gpu.memory.")
        || label.starts_with("gpu.adapter.")
        || label.starts_with("gpu.timestamp.")
    {
        MetricAggregation::Gauge
    } else {
        MetricAggregation::Sum
    }
}

fn is_resource_metric(label: &str, category: MetricCategory, unit: MetricUnit) -> bool {
    unit == MetricUnit::Bytes
        || matches!(
            category,
            MetricCategory::Layout
                | MetricCategory::Scene
                | MetricCategory::Renderer
                | MetricCategory::Gpu
        )
        || label.starts_with("presenter.")
}

pub(crate) fn encode_labels(labels: &[StoredLabel]) -> Vec<u8> {
    let mut output = header(MESSAGE_LABELS);
    push_u32(&mut output, labels.len() as u32);
    for stored in labels {
        let bytes = stored.label.as_bytes();
        let len = u16::try_from(bytes.len()).unwrap_or(u16::MAX);
        push_u32(&mut output, stored.id);
        output.push(stored.descriptor.category as u8);
        output.push(stored.descriptor.unit as u8);
        output.push(stored.descriptor.aggregation as u8);
        output.push(stored.descriptor.flags);
        push_u16(&mut output, len);
        output.extend_from_slice(&bytes[..usize::from(len)]);
    }
    output
}

pub(crate) fn encode_events(events: &[StoredEvent]) -> Vec<u8> {
    let mut output = Vec::with_capacity(16 + events.len() * EVENT_WIRE_BYTES);
    output.extend_from_slice(&header(MESSAGE_EVENTS));
    push_u32(&mut output, events.len() as u32);
    for stored in events {
        let event = stored.event;
        push_u64(&mut output, event.sequence);
        push_u64(&mut output, event.timestamp_ns);
        push_u64(&mut output, event.duration_ns);
        push_u64(&mut output, event.view.map_or(0, |id| id.get()));
        push_u64(&mut output, event.frame.map_or(0, |id| id.get()));
        push_u64(&mut output, event.presentation.map_or(0, |id| id.get()));
        push_u64(&mut output, event.value);
        push_u64(&mut output, event.auxiliary);
        push_u32(&mut output, event.lane);
        push_u32(&mut output, stored.label_id);
        push_u32(&mut output, stored.parent_label_id);
        output.push(event_kind(event.kind));
        output.push(timing_domain(event.timing_domain));
        push_u16(&mut output, 0);
    }
    output
}

pub(crate) fn encode_viewer_gap(dropped_batches: u64) -> Vec<u8> {
    let mut output = header(MESSAGE_VIEWER_GAP);
    push_u64(&mut output, dropped_batches);
    output
}

pub(crate) fn encode_capture(metadata: &[u8], labels: &[u8], events: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(20 + metadata.len() + labels.len() + events.len());
    output.extend_from_slice(CAPTURE_MAGIC);
    push_u16(&mut output, PROTOCOL_MAJOR);
    push_u16(&mut output, PROTOCOL_MINOR);
    push_u32(&mut output, metadata.len() as u32);
    push_u32(&mut output, labels.len() as u32);
    push_u32(&mut output, events.len() as u32);
    output.extend_from_slice(metadata);
    output.extend_from_slice(labels);
    output.extend_from_slice(events);
    output
}

fn header(kind: u8) -> Vec<u8> {
    let mut output = Vec::with_capacity(16);
    output.extend_from_slice(STREAM_MAGIC);
    push_u16(&mut output, PROTOCOL_MAJOR);
    push_u16(&mut output, PROTOCOL_MINOR);
    output.push(kind);
    output.extend_from_slice(&[0; 3]);
    output
}

const fn event_kind(kind: EventKind) -> u8 {
    kind as u8
}

const fn timing_domain(domain: TimingDomain) -> u8 {
    domain as u8
}

fn push_u16(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(output: &mut Vec<u8>, value: u64) {
    output.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::instrumentation::ProfileViewId;

    fn event() -> Event {
        Event {
            sequence: 7,
            timestamp_ns: 11,
            duration_ns: 13,
            lane: 2,
            view: None,
            frame: None,
            presentation: None,
            kind: EventKind::Span,
            timing_domain: TimingDomain::Cpu,
            label: "layout.measure",
            parent_label: None,
            value: 0,
            auxiliary: 0,
        }
    }

    #[test]
    fn event_batches_have_a_fixed_record_width() {
        let bytes = encode_events(&[StoredEvent {
            event: event(),
            label_id: 3,
            parent_label_id: 0,
        }]);
        assert_eq!(&bytes[..4], STREAM_MAGIC);
        assert_eq!(u16::from_le_bytes(bytes[4..6].try_into().unwrap()), 3);
        assert_eq!(bytes[8], MESSAGE_EVENTS);
        assert_eq!(bytes.len(), 16 + EVENT_WIRE_BYTES);
    }

    #[test]
    fn event_batches_preserve_view_identity() {
        let mut event = event();
        event.view = Some(ProfileViewId::PRIMARY);
        let bytes = encode_events(&[StoredEvent {
            event,
            label_id: 3,
            parent_label_id: 0,
        }]);
        assert_eq!(u64::from_le_bytes(bytes[40..48].try_into().unwrap()), 1);
    }

    #[test]
    fn capture_contains_sized_sections() {
        let capture = encode_capture(b"{}", b"labels", b"events");
        assert_eq!(&capture[..4], CAPTURE_MAGIC);
        assert_eq!(u16::from_le_bytes(capture[4..6].try_into().unwrap()), 3);
        assert_eq!(u32::from_le_bytes(capture[8..12].try_into().unwrap()), 2);
        assert_eq!(u32::from_le_bytes(capture[12..16].try_into().unwrap()), 6);
        assert_eq!(u32::from_le_bytes(capture[16..20].try_into().unwrap()), 6);
    }

    #[test]
    fn label_batches_include_machine_readable_metric_metadata() {
        let bytes = encode_labels(&[StoredLabel {
            id: 9,
            label: "host.redraw_requests",
            descriptor: describe_label(
                "host.redraw_requests",
                EventKind::Counter,
                TimingDomain::Cpu,
            ),
        }]);
        assert_eq!(u32::from_le_bytes(bytes[12..16].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(bytes[16..20].try_into().unwrap()), 9);
        assert_eq!(bytes[20], MetricCategory::Runtime as u8);
        assert_eq!(bytes[21], MetricUnit::Count as u8);
        assert_eq!(bytes[22], MetricAggregation::Cumulative as u8);
        assert_eq!(u16::from_le_bytes(bytes[24..26].try_into().unwrap()), 20);
    }

    #[test]
    fn resource_and_frame_counter_descriptors_are_distinct() {
        let resource = describe_label("render.upload_bytes", EventKind::Counter, TimingDomain::Cpu);
        assert_eq!(resource.unit, MetricUnit::Bytes);
        assert_eq!(resource.aggregation, MetricAggregation::Sum);
        assert_ne!(resource.flags & LABEL_FLAG_RESOURCE, 0);

        let frame_interval = describe_label(
            "frame.refresh_interval_ns",
            EventKind::Counter,
            TimingDomain::Cpu,
        );
        assert_eq!(frame_interval.unit, MetricUnit::Nanoseconds);
        assert_eq!(frame_interval.aggregation, MetricAggregation::Gauge);
        assert_eq!(frame_interval.flags & LABEL_FLAG_RESOURCE, 0);

        let driver = describe_label(
            "gpu.adapter.driver_version",
            EventKind::Counter,
            TimingDomain::Cpu,
        );
        assert_eq!(driver.unit, MetricUnit::Identifier);
        assert_eq!(driver.aggregation, MetricAggregation::Gauge);
        assert_ne!(driver.flags & LABEL_FLAG_RESOURCE, 0);
    }

    #[test]
    fn libinput_event_age_instants_keep_their_nanosecond_unit() {
        let descriptor = describe_label(
            "input.libinput.pointer_motion.relative.queue_age_ns",
            EventKind::Instant,
            TimingDomain::Cpu,
        );
        assert_eq!(descriptor.category, MetricCategory::Input);
        assert_eq!(descriptor.unit, MetricUnit::Nanoseconds);
        assert_eq!(descriptor.aggregation, MetricAggregation::Event);

        let pipeline = describe_label(
            "input.libinput.pointer_motion.pipeline.freshest_event_to_cursor_submit_ns",
            EventKind::Instant,
            TimingDomain::Cpu,
        );
        assert_eq!(pipeline.category, MetricCategory::Input);
        assert_eq!(pipeline.unit, MetricUnit::Nanoseconds);
        assert_eq!(pipeline.aggregation, MetricAggregation::Event);
    }

    #[test]
    fn presentation_work_has_a_distinct_semantic_category() {
        for label in [
            "presentation.vulkan",
            "presentation.retry.acquire_not_ready",
            "responsiveness.resize.updating",
            "presentation.worker.queue_age_ns",
            "presenter.retirement_wait",
            "swapchain.acquire",
            "queue.submit_present",
            "worker.submit",
        ] {
            assert_eq!(
                describe_label(label, EventKind::Span, TimingDomain::Cpu).category,
                MetricCategory::Presentation,
                "{label}"
            );
        }
    }
}

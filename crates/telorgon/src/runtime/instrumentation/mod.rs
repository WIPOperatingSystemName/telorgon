//! Compile-time optional, bounded event production for the Telorgon development profiler.
//!
//! This crate deliberately contains no socket, browser, filesystem, executor, or application
//! lifecycle integration. A managed host activates one session and transfers its collector to the
//! separately owned profiler service.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Instant;

use rtrb::{Consumer, Producer, RingBuffer};

/// Default record capacity for one registered producer.
pub const DEFAULT_PRODUCER_CAPACITY: usize = 4_096;
/// Maximum supported nested CPU span depth on one producer lane.
pub const MAX_SPAN_DEPTH: usize = 64;
/// Maximum number of independently rendered views registered in one session.
pub const MAX_PROFILE_VIEWS: usize = 1_024;

static ACTIVE_GENERATION: AtomicU64 = AtomicU64::new(0);
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
static ACTIVE_SESSION: Mutex<Option<Weak<SessionInner>>> = Mutex::new(None);
static INPUT_RECORDING_ENABLED: AtomicU32 = AtomicU32::new(0);

/// Opt-in native input streams whose event-rate can materially perturb a profiling session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub(crate) enum InputRecordingSource {
    PointerMotion,
    PointerButton,
    Scroll,
    Keyboard,
    TouchMotion,
    TouchContact,
    DeviceChange,
}

impl InputRecordingSource {
    const fn mask(self) -> u32 {
        1 << self as u32
    }
}

thread_local! {
    static LOCAL_PRODUCER: RefCell<LocalProducer> = const {
        RefCell::new(LocalProducer::inactive())
    };
    static RECORDING_SUPPRESSION_DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// Stable identity for one host-scheduled frame in a profiler session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProfileFrameId(u64);

impl ProfileFrameId {
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    #[must_use]
    pub const fn from_raw(value: u64) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }
}

/// Stable identity for one independently rendered view or surface in a profiler session.
///
/// Hosts allocate these identities. The primary view used by a single-window application is
/// [`ProfileViewId::PRIMARY`]; desktop, widget, compositor, and embedded hosts can assign one
/// identity per independently scheduled output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProfileViewId(u64);

impl ProfileViewId {
    pub const PRIMARY: Self = Self(1);

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    #[must_use]
    pub const fn from_raw(value: u64) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }
}

/// Stable identity for one presentation attempt in a profiler session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProfilePresentationId(u64);

impl ProfilePresentationId {
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    #[must_use]
    pub const fn from_raw(value: u64) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }
}

/// Fixed event vocabulary shared by collectors and transports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum EventKind {
    Span = 1,
    Instant = 2,
    Counter = 3,
    FrameBegin = 4,
    FrameEnd = 5,
    PresentationBegin = 6,
    PresentationEnd = 7,
    Diagnostic = 8,
    Gap = 9,
    GpuSpanResolved = 10,
}

/// Timing domain used by an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TimingDomain {
    Cpu = 1,
    GpuRelative = 2,
}

/// Severity for a bounded structured diagnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum DiagnosticSeverity {
    Information = 1,
    Warning = 2,
    Error = 3,
}

/// One fixed-size producer record. Labels are static program data and are interned by the
/// collector, never by a renderer or runtime hot path.
#[derive(Clone, Copy, Debug)]
pub struct Event {
    pub sequence: u64,
    pub timestamp_ns: u64,
    pub duration_ns: u64,
    pub lane: u32,
    pub view: Option<ProfileViewId>,
    pub frame: Option<ProfileFrameId>,
    pub presentation: Option<ProfilePresentationId>,
    pub kind: EventKind,
    pub timing_domain: TimingDomain,
    pub label: &'static str,
    pub parent_label: Option<&'static str>,
    pub value: u64,
    pub auxiliary: u64,
}

impl Event {
    #[must_use]
    pub fn counter_value(self) -> Option<f64> {
        if matches!(self.kind, EventKind::Counter) {
            Some(f64::from_bits(self.value))
        } else {
            None
        }
    }

    #[must_use]
    pub const fn dropped_count(self) -> Option<u64> {
        if matches!(self.kind, EventKind::Gap) {
            Some(self.value)
        } else {
            None
        }
    }
}

/// Immutable information about one registered producer lane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaneInfo {
    pub id: u32,
    pub name: String,
}

/// Immutable, content-free display metadata for one independently rendered view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewInfo {
    pub id: ProfileViewId,
    pub role: &'static str,
}

/// Bounds chosen when a profiler session is activated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionConfig {
    pub producer_capacity: usize,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            producer_capacity: DEFAULT_PRODUCER_CAPACITY,
        }
    }
}

/// Error returned when a session cannot be activated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionStartError(&'static str);

impl fmt::Display for SessionStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for SessionStartError {}

struct ConsumerSlot {
    lane: LaneInfo,
    consumer: Consumer<Event>,
    lost: Arc<AtomicU64>,
}

struct SessionInner {
    generation: u64,
    origin: Instant,
    producer_capacity: usize,
    sequence: Arc<AtomicU64>,
    next_lane: AtomicU32,
    next_view: AtomicU64,
    next_frame: Arc<AtomicU64>,
    next_presentation: Arc<AtomicU64>,
    views: Mutex<Vec<ViewInfo>>,
    consumers: Mutex<Vec<ConsumerSlot>>,
}

/// Activation owner for one process-local profiling session.
pub struct Session {
    inner: Arc<SessionInner>,
}

impl Session {
    /// Activates a single session and returns its exclusive collector.
    pub fn start(config: SessionConfig) -> Result<(Self, Collector), SessionStartError> {
        if config.producer_capacity == 0 {
            return Err(SessionStartError("producer capacity must be non-zero"));
        }
        let mut active = ACTIVE_SESSION
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if active.as_ref().and_then(Weak::upgrade).is_some() {
            return Err(SessionStartError(
                "a Telorgon profiler session is already active",
            ));
        }
        let generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed).max(1);
        INPUT_RECORDING_ENABLED.store(0, Ordering::Release);
        let inner = Arc::new(SessionInner {
            generation,
            origin: Instant::now(),
            producer_capacity: config.producer_capacity,
            sequence: Arc::new(AtomicU64::new(1)),
            next_lane: AtomicU32::new(1),
            next_view: AtomicU64::new(2),
            next_frame: Arc::new(AtomicU64::new(1)),
            next_presentation: Arc::new(AtomicU64::new(1)),
            views: Mutex::new(Vec::new()),
            consumers: Mutex::new(Vec::new()),
        });
        *active = Some(Arc::downgrade(&inner));
        ACTIVE_GENERATION.store(generation, Ordering::Release);
        Ok((
            Self {
                inner: Arc::clone(&inner),
            },
            Collector { inner },
        ))
    }

    #[must_use]
    pub fn elapsed_ns(&self) -> u64 {
        elapsed_ns(self.inner.origin)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = ACTIVE_GENERATION.compare_exchange(
            self.inner.generation,
            0,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        let mut active = ACTIVE_SESSION
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if active
            .as_ref()
            .and_then(Weak::upgrade)
            .is_some_and(|inner| inner.generation == self.inner.generation)
        {
            *active = None;
            INPUT_RECORDING_ENABLED.store(0, Ordering::Release);
        }
    }
}

/// Single consumer for every registered profiler producer.
pub struct Collector {
    inner: Arc<SessionInner>,
}

impl Collector {
    /// Drains all currently available records without waiting for producers.
    pub fn drain_into(&mut self, events: &mut Vec<Event>) {
        let mut consumers = self
            .inner
            .consumers
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for slot in consumers.iter_mut() {
            while let Ok(event) = slot.consumer.pop() {
                events.push(event);
            }
            let dropped = slot.lost.swap(0, Ordering::AcqRel);
            if dropped > 0 {
                events.push(Event {
                    sequence: self.inner.sequence.fetch_add(1, Ordering::Relaxed),
                    timestamp_ns: elapsed_ns(self.inner.origin),
                    duration_ns: 0,
                    lane: slot.lane.id,
                    view: None,
                    frame: None,
                    presentation: None,
                    kind: EventKind::Gap,
                    timing_domain: TimingDomain::Cpu,
                    label: "profiler.producer_gap",
                    parent_label: None,
                    value: dropped,
                    auxiliary: 0,
                });
            }
        }
        events.sort_unstable_by_key(|event| event.sequence);
    }

    /// Copies the current bounded lane dictionary.
    pub fn lanes(&self) -> Vec<LaneInfo> {
        self.inner
            .consumers
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .map(|slot| slot.lane.clone())
            .collect()
    }

    /// Copies the current bounded view dictionary.
    pub fn views(&self) -> Vec<ViewInfo> {
        self.inner
            .views
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
}

struct LocalProducer {
    generation: u64,
    lane: u32,
    origin: Option<Instant>,
    producer: Option<Producer<Event>>,
    lost: Option<Arc<AtomicU64>>,
    sequence: Option<Arc<AtomicU64>>,
    next_frame: Option<Arc<AtomicU64>>,
    next_presentation: Option<Arc<AtomicU64>>,
    view: Option<ProfileViewId>,
    frame: Option<ProfileFrameId>,
    presentation: Option<ProfilePresentationId>,
    parents: [Option<&'static str>; MAX_SPAN_DEPTH],
    depth: usize,
}

impl LocalProducer {
    const fn inactive() -> Self {
        Self {
            generation: 0,
            lane: 0,
            origin: None,
            producer: None,
            lost: None,
            sequence: None,
            next_frame: None,
            next_presentation: None,
            view: None,
            frame: None,
            presentation: None,
            parents: [None; MAX_SPAN_DEPTH],
            depth: 0,
        }
    }

    fn register(generation: u64) -> Option<Self> {
        let inner = ACTIVE_SESSION
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .and_then(Weak::upgrade)?;
        if inner.generation != generation {
            return None;
        }
        let lane = inner.next_lane.fetch_add(1, Ordering::Relaxed);
        let (producer, consumer) = RingBuffer::new(inner.producer_capacity);
        let lost = Arc::new(AtomicU64::new(0));
        let lane_name = std::thread::current()
            .name()
            .unwrap_or("unnamed")
            .chars()
            .take(64)
            .collect::<String>();
        inner
            .consumers
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(ConsumerSlot {
                lane: LaneInfo {
                    id: lane,
                    name: lane_name,
                },
                consumer,
                lost: Arc::clone(&lost),
            });
        Some(Self {
            generation,
            lane,
            origin: Some(inner.origin),
            producer: Some(producer),
            lost: Some(lost),
            sequence: Some(Arc::clone(&inner.sequence)),
            next_frame: Some(Arc::clone(&inner.next_frame)),
            next_presentation: Some(Arc::clone(&inner.next_presentation)),
            view: None,
            frame: None,
            presentation: None,
            parents: [None; MAX_SPAN_DEPTH],
            depth: 0,
        })
    }

    fn push(&mut self, mut event: Event) {
        let Some(sequence) = self.sequence.as_ref() else {
            return;
        };
        event.sequence = sequence.fetch_add(1, Ordering::Relaxed);
        event.lane = self.lane;
        event.view = event.view.or(self.view);
        event.frame = event.frame.or(self.frame);
        event.presentation = event.presentation.or(self.presentation);
        if self
            .producer
            .as_mut()
            .is_some_and(|producer| producer.push(event).is_err())
            && let Some(lost) = self.lost.as_ref()
        {
            lost.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn now_ns(&self) -> u64 {
        self.origin.map_or(0, elapsed_ns)
    }

    fn parent(&self) -> Option<&'static str> {
        self.depth
            .checked_sub(1)
            .and_then(|index| self.parents[index])
    }
}

fn elapsed_ns(origin: Instant) -> u64 {
    u64::try_from(origin.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

fn with_producer<R>(operation: impl FnOnce(&mut LocalProducer) -> R) -> Option<R> {
    if recording_suppressed() {
        return None;
    }
    let generation = ACTIVE_GENERATION.load(Ordering::Acquire);
    if generation == 0 {
        return None;
    }
    LOCAL_PRODUCER.with(|producer| {
        let mut producer = producer.borrow_mut();
        if producer.generation != generation {
            *producer = LocalProducer::register(generation)?;
        }
        Some(operation(&mut producer))
    })
}

fn recording_suppressed() -> bool {
    RECORDING_SUPPRESSION_DEPTH.with(|depth| depth.get() != 0)
}

/// Temporarily prevents the current thread from producing profiler records.
///
/// This is intended for high-rate host work that remains functionally necessary but has been
/// explicitly excluded from the active capture. Suppression is thread-local and nestable, so
/// independently correlated renderer or worker lanes continue recording normally.
pub fn suppress_current_thread() -> RecordingSuppressionGuard {
    RECORDING_SUPPRESSION_DEPTH.with(|depth| depth.set(depth.get().saturating_add(1)));
    RecordingSuppressionGuard { active: true }
}

/// RAII restoration of current-thread profiler recording.
#[must_use]
pub struct RecordingSuppressionGuard {
    active: bool,
}

impl Drop for RecordingSuppressionGuard {
    fn drop(&mut self) {
        if self.active {
            RECORDING_SUPPRESSION_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
        }
    }
}

/// Reports whether native pointer-movement details should be recorded for the active session.
#[inline]
#[must_use]
pub fn pointer_move_events_enabled() -> bool {
    input_recording_enabled(InputRecordingSource::PointerMotion)
}

/// Changes whether future native pointer-movement details are recorded.
///
/// The managed profiler viewer owns this session preference. It defaults to disabled whenever a
/// new session starts; changing it never fabricates or restores records from the excluded period.
#[inline]
pub fn set_pointer_move_events_enabled(enabled: bool) {
    set_input_recording_enabled(InputRecordingSource::PointerMotion, enabled);
}

/// Reports whether one opt-in native input stream should publish individual profiler records.
#[inline]
#[must_use]
pub(crate) fn input_recording_enabled(source: InputRecordingSource) -> bool {
    INPUT_RECORDING_ENABLED.load(Ordering::Acquire) & source.mask() != 0
}

/// Changes whether future events from one native input stream publish profiler records.
///
/// The managed viewer owns this session preference. Every stream defaults to disabled, and
/// changing a preference never fabricates records from the excluded period.
#[inline]
pub(crate) fn set_input_recording_enabled(source: InputRecordingSource, enabled: bool) {
    if enabled {
        INPUT_RECORDING_ENABLED.fetch_or(source.mask(), Ordering::AcqRel);
    } else {
        INPUT_RECORDING_ENABLED.fetch_and(!source.mask(), Ordering::AcqRel);
    }
}

/// Returns whether a managed profiler session is currently active.
#[inline]
#[must_use]
pub fn is_active() -> bool {
    ACTIVE_GENERATION.load(Ordering::Relaxed) != 0 && !recording_suppressed()
}

/// Current correlated frame on this thread, when one has been established.
#[must_use]
pub fn current_frame_id() -> Option<ProfileFrameId> {
    with_producer(|producer| producer.frame).flatten()
}

/// Current independently rendered view on this thread, when one has been established.
#[must_use]
pub fn current_view_id() -> Option<ProfileViewId> {
    with_producer(|producer| producer.view).flatten()
}

/// Allocates a session-unique view identity. The primary single-window identity remains reserved.
#[must_use]
pub fn allocate_view_id() -> Option<ProfileViewId> {
    let generation = ACTIVE_GENERATION.load(Ordering::Acquire);
    if generation == 0 {
        return None;
    }
    ACTIVE_SESSION
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()
        .and_then(Weak::upgrade)
        .filter(|inner| inner.generation == generation)
        .and_then(|inner| {
            let id = inner.next_view.fetch_add(1, Ordering::Relaxed);
            ProfileViewId::from_raw(id)
        })
}

/// Registers content-free display metadata for a host-owned view.
///
/// Registration is a lifecycle operation, not a hot-path event. Duplicate registration succeeds
/// only when the role is unchanged; the bounded registry never replaces an existing identity.
pub fn register_view(view: ProfileViewId, role: &'static str) -> bool {
    let generation = ACTIVE_GENERATION.load(Ordering::Acquire);
    if generation == 0 || role.is_empty() || role.len() > 64 {
        return false;
    }
    let Some(inner) = ACTIVE_SESSION
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()
        .and_then(Weak::upgrade)
        .filter(|inner| inner.generation == generation)
    else {
        return false;
    };
    let mut views = inner
        .views
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(existing) = views.iter().find(|existing| existing.id == view) {
        return existing.role == role;
    }
    if views.len() >= MAX_PROFILE_VIEWS {
        return false;
    }
    views.push(ViewInfo { id: view, role });
    views.sort_unstable_by_key(|view| view.id);
    true
}

/// Temporarily correlates work on this thread with an independently rendered view.
pub fn enter_view(view: Option<ProfileViewId>) -> ViewScopeGuard {
    let generation = ACTIVE_GENERATION.load(Ordering::Acquire);
    let previous = with_producer(|producer| std::mem::replace(&mut producer.view, view)).flatten();
    ViewScopeGuard {
        generation,
        previous,
        active: generation != 0,
    }
}

#[must_use]
pub struct ViewScopeGuard {
    generation: u64,
    previous: Option<ProfileViewId>,
    active: bool,
}

impl Drop for ViewScopeGuard {
    fn drop(&mut self) {
        if self.active && ACTIVE_GENERATION.load(Ordering::Acquire) == self.generation {
            let _ = with_producer(|producer| producer.view = self.previous);
        }
    }
}

/// Starts one completed CPU span. Prefer [`span!`] at instrumentation sites.
pub fn start_span(label: &'static str) -> SpanGuard {
    let generation = ACTIVE_GENERATION.load(Ordering::Acquire);
    let Some((start_ns, parent, pushed)) = with_producer(|producer| {
        let start_ns = producer.now_ns();
        let parent = producer.parent();
        let pushed = producer.depth < MAX_SPAN_DEPTH;
        if pushed {
            producer.parents[producer.depth] = Some(label);
            producer.depth += 1;
        }
        (start_ns, parent, pushed)
    }) else {
        return SpanGuard::inactive();
    };
    SpanGuard {
        generation,
        start_ns,
        label,
        parent,
        pushed,
        active: true,
    }
}

/// RAII completion of one CPU span.
#[must_use]
pub struct SpanGuard {
    generation: u64,
    start_ns: u64,
    label: &'static str,
    parent: Option<&'static str>,
    pushed: bool,
    active: bool,
}

impl SpanGuard {
    const fn inactive() -> Self {
        Self {
            generation: 0,
            start_ns: 0,
            label: "",
            parent: None,
            pushed: false,
            active: false,
        }
    }
}

impl Drop for SpanGuard {
    fn drop(&mut self) {
        if !self.active || ACTIVE_GENERATION.load(Ordering::Acquire) != self.generation {
            return;
        }
        let _ = with_producer(|producer| {
            let end_ns = producer.now_ns();
            if self.pushed && producer.depth > 0 {
                producer.depth -= 1;
                producer.parents[producer.depth] = None;
            }
            producer.push(Event {
                sequence: 0,
                timestamp_ns: self.start_ns,
                duration_ns: end_ns.saturating_sub(self.start_ns),
                lane: 0,
                view: None,
                frame: None,
                presentation: None,
                kind: EventKind::Span,
                timing_domain: TimingDomain::Cpu,
                label: self.label,
                parent_label: self.parent,
                value: 0,
                auxiliary: 0,
            });
        });
    }
}

/// Starts one host-scheduled frame and makes its identity current on this thread.
pub fn start_frame(label: &'static str) -> FrameGuard {
    let generation = ACTIVE_GENERATION.load(Ordering::Acquire);
    let Some((id, previous, start_ns)) = with_producer(|producer| {
        let raw = producer
            .next_frame
            .as_ref()
            .map_or(0, |next| next.fetch_add(1, Ordering::Relaxed));
        let id = ProfileFrameId(raw.max(1));
        let previous = producer.frame.replace(id);
        let start_ns = producer.now_ns();
        producer.push(Event {
            sequence: 0,
            timestamp_ns: start_ns,
            duration_ns: 0,
            lane: 0,
            view: None,
            frame: Some(id),
            presentation: None,
            kind: EventKind::FrameBegin,
            timing_domain: TimingDomain::Cpu,
            label,
            parent_label: None,
            value: 0,
            auxiliary: 0,
        });
        (id, previous, start_ns)
    }) else {
        return FrameGuard::inactive();
    };
    FrameGuard {
        generation,
        id: Some(id),
        previous,
        start_ns,
        label,
    }
}

/// RAII host-frame boundary.
#[must_use]
pub struct FrameGuard {
    generation: u64,
    id: Option<ProfileFrameId>,
    previous: Option<ProfileFrameId>,
    start_ns: u64,
    label: &'static str,
}

impl FrameGuard {
    const fn inactive() -> Self {
        Self {
            generation: 0,
            id: None,
            previous: None,
            start_ns: 0,
            label: "",
        }
    }

    #[must_use]
    pub const fn id(&self) -> Option<ProfileFrameId> {
        self.id
    }
}

impl Drop for FrameGuard {
    fn drop(&mut self) {
        let Some(id) = self.id else { return };
        if ACTIVE_GENERATION.load(Ordering::Acquire) != self.generation {
            return;
        }
        let _ = with_producer(|producer| {
            let end_ns = producer.now_ns();
            producer.frame = self.previous;
            producer.push(Event {
                sequence: 0,
                timestamp_ns: self.start_ns,
                duration_ns: end_ns.saturating_sub(self.start_ns),
                lane: 0,
                view: None,
                frame: Some(id),
                presentation: None,
                kind: EventKind::FrameEnd,
                timing_domain: TimingDomain::Cpu,
                label: self.label,
                parent_label: None,
                value: 0,
                auxiliary: 0,
            });
        });
    }
}

/// Temporarily correlates work on another thread with an existing host frame.
pub fn enter_frame(frame: Option<ProfileFrameId>) -> FrameScopeGuard {
    let generation = ACTIVE_GENERATION.load(Ordering::Acquire);
    let previous =
        with_producer(|producer| std::mem::replace(&mut producer.frame, frame)).flatten();
    FrameScopeGuard {
        generation,
        previous,
        active: generation != 0,
    }
}

#[must_use]
pub struct FrameScopeGuard {
    generation: u64,
    previous: Option<ProfileFrameId>,
    active: bool,
}

impl Drop for FrameScopeGuard {
    fn drop(&mut self) {
        if self.active && ACTIVE_GENERATION.load(Ordering::Acquire) == self.generation {
            let _ = with_producer(|producer| producer.frame = self.previous);
        }
    }
}

/// Starts a correlated presentation attempt on this thread.
pub fn start_presentation(label: &'static str) -> PresentationGuard {
    let generation = ACTIVE_GENERATION.load(Ordering::Acquire);
    let Some((id, previous, start_ns)) = with_producer(|producer| {
        let raw = producer
            .next_presentation
            .as_ref()
            .map_or(0, |next| next.fetch_add(1, Ordering::Relaxed));
        let id = ProfilePresentationId(raw.max(1));
        let previous = producer.presentation.replace(id);
        let start_ns = producer.now_ns();
        producer.push(Event {
            sequence: 0,
            timestamp_ns: start_ns,
            duration_ns: 0,
            lane: 0,
            view: None,
            frame: None,
            presentation: Some(id),
            kind: EventKind::PresentationBegin,
            timing_domain: TimingDomain::Cpu,
            label,
            parent_label: None,
            value: 0,
            auxiliary: 0,
        });
        (id, previous, start_ns)
    }) else {
        return PresentationGuard::inactive();
    };
    PresentationGuard {
        generation,
        id: Some(id),
        previous,
        start_ns,
        label,
    }
}

#[must_use]
pub struct PresentationGuard {
    generation: u64,
    id: Option<ProfilePresentationId>,
    previous: Option<ProfilePresentationId>,
    start_ns: u64,
    label: &'static str,
}

impl PresentationGuard {
    const fn inactive() -> Self {
        Self {
            generation: 0,
            id: None,
            previous: None,
            start_ns: 0,
            label: "",
        }
    }
}

impl Drop for PresentationGuard {
    fn drop(&mut self) {
        let Some(id) = self.id else { return };
        if ACTIVE_GENERATION.load(Ordering::Acquire) != self.generation {
            return;
        }
        let _ = with_producer(|producer| {
            let end_ns = producer.now_ns();
            producer.presentation = self.previous;
            producer.push(Event {
                sequence: 0,
                timestamp_ns: self.start_ns,
                duration_ns: end_ns.saturating_sub(self.start_ns),
                lane: 0,
                view: None,
                frame: None,
                presentation: Some(id),
                kind: EventKind::PresentationEnd,
                timing_domain: TimingDomain::Cpu,
                label: self.label,
                parent_label: None,
                value: 0,
                auxiliary: 0,
            });
        });
    }
}

/// Records one numeric counter observation.
pub fn record_counter(label: &'static str, value: f64) {
    let _ = with_producer(|producer| {
        producer.push(Event {
            sequence: 0,
            timestamp_ns: producer.now_ns(),
            duration_ns: 0,
            lane: 0,
            view: None,
            frame: None,
            presentation: None,
            kind: EventKind::Counter,
            timing_domain: TimingDomain::Cpu,
            label,
            parent_label: producer.parent(),
            value: value.to_bits(),
            auxiliary: 0,
        });
    });
}

/// Records a bounded event with no dynamic payload.
pub fn record_instant(label: &'static str) {
    record_value(EventKind::Instant, label, 0, 0);
}

/// Records a bounded event and one integer value.
pub fn record_instant_value(label: &'static str, value: u64) {
    record_value(EventKind::Instant, label, value, 0);
}

/// Records a sanitized structured diagnostic kind.
pub fn record_diagnostic(label: &'static str, severity: DiagnosticSeverity, count: u64) {
    record_value(EventKind::Diagnostic, label, count, severity as u64);
}

/// Records a completed GPU-relative interval after backend completion has made its query results
/// available. The timestamp is relative to that GPU frame's first timestamp, never a CPU time.
pub fn record_gpu_span(
    label: &'static str,
    frame: Option<ProfileFrameId>,
    start_ns: u64,
    duration_ns: u64,
) {
    let _ = with_producer(|producer| {
        producer.push(Event {
            sequence: 0,
            timestamp_ns: start_ns,
            duration_ns,
            lane: 0,
            view: None,
            frame,
            presentation: None,
            kind: EventKind::GpuSpanResolved,
            timing_domain: TimingDomain::GpuRelative,
            label,
            parent_label: None,
            value: 0,
            auxiliary: 0,
        });
    });
}

fn record_value(kind: EventKind, label: &'static str, value: u64, auxiliary: u64) {
    let _ = with_producer(|producer| {
        producer.push(Event {
            sequence: 0,
            timestamp_ns: producer.now_ns(),
            duration_ns: 0,
            lane: 0,
            view: None,
            frame: None,
            presentation: None,
            kind,
            timing_domain: TimingDomain::Cpu,
            label,
            parent_label: producer.parent(),
            value,
            auxiliary,
        });
    });
}

/// Creates an RAII span from a static label.
#[macro_export]
macro_rules! span {
    ($label:literal) => {{ $crate::runtime::instrumentation::start_span($label) }};
}

/// Records a counter lazily; `$value` is not evaluated while the session is inactive.
#[macro_export]
macro_rules! counter {
    ($label:literal, $value:expr) => {{
        if $crate::runtime::instrumentation::is_active() {
            $crate::runtime::instrumentation::record_counter($label, ($value) as f64);
        }
    }};
}

/// Records an instant event while a session is active.
#[macro_export]
macro_rules! instant {
    ($label:literal) => {{
        if $crate::runtime::instrumentation::is_active() {
            $crate::runtime::instrumentation::record_instant($label);
        }
    }};
    ($label:literal, $value:expr) => {{
        if $crate::runtime::instrumentation::is_active() {
            $crate::runtime::instrumentation::record_instant_value($label, ($value) as u64);
        }
    }};
}

// `#[macro_export]` places these macros at the monolithic crate root. Re-export them from the
// profiler module so existing instrumentation paths remain stable after crate consolidation.
pub use crate::{counter, instant, span};

#[cfg(test)]
mod tests;

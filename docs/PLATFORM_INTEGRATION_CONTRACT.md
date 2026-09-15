# Telorgon Platform Integration Contract

Status: **Gate 9 canonical architecture contract**

This document freezes the boundary between Telorgon's portable UI/runtime/renderer packages and the
operating system or embedding host. It is authoritative for lifecycle, views, input translation,
IME, accessibility, clipboard and data transfer, platform services, native handles, and shell-host
integration. Earlier documents remain authoritative for their own domains, but must defer to this
contract when they describe a platform boundary.

Gate 9 closes the numbered architecture-planning sequence. It does not claim that these interfaces
are implemented. Implementation proceeds through the milestones in
[Platform implementation order](PLATFORM_IMPLEMENTATION_ORDER.md), with qualification defined by
[Acceptance and qualification](ACCEPTANCE_AND_QUALIFICATION.md).

## 1. Decisions fixed by this gate

1. `telorgon-platform` contains neutral values and interfaces. It does not depend on Winit, Vulkan,
   Metal, DirectX, Wayland, an Activity type, AppKit, or UIKit.
2. Platform access is a set of narrow, capability-checked service handles. There is no global
   platform singleton and no all-powerful `Platform` trait.
3. Lifecycle, activity, visibility, native-surface availability, and GPU presentation are separate
   state axes. A single enum must not pretend that all five change together.
4. The portable runtime is a single-writer state machine. Platform callbacks enqueue immutable
   events; they never re-enter component code.
5. Managed applications and embedded/engine use are sibling assemblies over the same portable
   runtime. Embedded mode creates no event loop, window, surface, presenter, queue, or thread.
6. Input preserves the information supplied by the platform. It does not invent pixel conversions,
   sentinel coordinates, fake timestamps, or collapse physical and logical keys.
7. Gate 7's UTF-8 text model remains canonical. Platform adapters perform revision-checked UTF-16
   or native-range conversion for IME and accessibility.
8. Accessibility is a first-class per-view semantic export. It is not derived from pixels and is
   not an optional afterthought for an operational application or shell profile.
9. Clipboard and drag/drop are typed multi-format, asynchronous, bounded data-offer protocols.
   Plain text is a convenience operation, not the entire abstraction.
10. Raw native handles never enter portable scene or component state. Backend/platform adapter
    packages use short-lived borrowed handles or typed owning import payloads.
11. Protocol implementations remain outside portable Telorgon contracts. A shell host adapts protocol
    objects, serials, seats, imported buffers, and release notifications to those neutral contracts;
    the host may be external or the Linux-only first-party Wayland compositor assembly.
12. Unsupported, denied, unavailable, stale, and failed are different outcomes and remain visible.
    No adapter silently succeeds or substitutes a no-op service.

## 2. Dependency and ownership boundary

The intended dependency direction is:

```text
application/shell components
          |
          v
telorgon-runtime ----> telorgon-input / telorgon-text / telorgon-accessibility
          |                         |
          +------> telorgon-platform <+
                         ^
                         |
        platform adapters and host-provided service implementations

telorgon-app   = managed platform + runtime + selected presenter/backend
telorgon-embed = host-owned platform services + runtime + selected backend
```

The neutral runtime may retain service handles supplied when a view is mounted. Those handles are
object-safe, thread behavior is documented per method, and their implementations belong to the
host or an adapter package. They do not give components direct access to native objects.

The renderer boundary stays separate:

- platform adapters own event-loop and OS-service integration;
- presenters own native surfaces, swapchains/drawables, acquisition, and presentation;
- render backends own device resources and command recording;
- the host owns queue submission and composition in embedded mode;
- the runtime owns component state, scheduling, input dispatch, semantics, and scene production.

## 3. Common vocabulary and wire values

### 3.1 Identity

All externally observable objects use opaque, nonzero, generation-aware identities:

```rust
pub struct ViewId(/* opaque slot + generation */);
pub struct RequestId(/* opaque sequence */);
pub struct TextSessionId(/* opaque slot + generation */);
pub struct DataOfferId(/* opaque slot + generation */);
pub struct AccessibilityNodeId(/* per-view stable identity */);
```

An ID is meaningful only in the owner and generation that issued it. IDs are not native window
handles, pointers, protocol object IDs, file descriptors, array indices, or globally stable database
keys. Reusing a closed object's numeric bits must increment its generation.

### 3.2 Time and ordering

Every platform-to-runtime event carries:

```rust
pub struct EventStamp {
    pub sequence: u64,
    pub received_at: MonotonicInstant,
    pub source_at: Option<MonotonicInstant>,
}
```

`sequence` is strictly increasing for one host event stream. `received_at` uses the injected host
monotonic clock. `source_at` exists only when the source timestamp can be mapped reliably into the
same clock domain. Wall-clock time is never used for input ordering, animation, gesture thresholds,
or frame deadlines.

### 3.3 Capabilities and availability

Feature support is queried at the relevant scope, normally host, view, or data offer:

```rust
pub enum Support<T> {
    Unavailable(UnavailableReason),
    Available(T),
}

pub enum PermissionState {
    NotRequired,
    Unknown,
    PromptRequired,
    Granted,
    Denied,
    Restricted,
}
```

A capability descriptor states supported operations, limits, permissions, execution/thread
requirements, and whether an operation needs a recent user gesture. Compile-time features decide
which adapters are present; runtime capability queries decide what the current environment allows.

### 3.4 Requests and observed truth

Platform mutation is request/observation based. Enqueueing a request returns a `RequestId` or an
immediate validation error. Completion produces exactly one terminal result:

```rust
pub enum RequestOutcome<T> {
    Applied(T),
    Denied,
    Unsupported,
    Cancelled,
    Stale,
    Failed(PlatformError),
}
```

The next revisioned snapshot is the truth about window state, focus, metrics, or service state.
`Applied` means the platform accepted and completed an operation; it does not permit the runtime to
invent a snapshot before the OS reports one. Closing a view cancels its outstanding requests.

Errors are structured, source-preserving, and redact sensitive payloads. Portable code must not
branch on display strings.

## 4. View and lifecycle model

### 4.1 Independent state axes

Each view exposes one atomic `ViewSnapshot` containing at least:

```rust
pub struct ViewSnapshot {
    pub view: ViewId,
    pub revision: u64,
    pub lifetime: ViewLifetime,
    pub activity: ActivityState,
    pub visibility: VisibilityState,
    pub focus: FocusSnapshot,
    pub metrics: ViewMetricsSnapshot,
    pub surface: NativeSurfaceState,
    pub environment: EnvironmentSnapshot,
}
```

The axes are:

- `ViewLifetime`: `Declared`, `Live`, `Closing`, `Closed`;
- `ActivityState`: `Active`, `Inactive`, `Background`, `Suspended`;
- `VisibilityState`: `Visible`, `Hidden`, `Occluded`;
- `NativeSurfaceState`: `Unavailable` or `Available { generation }`;
- presenter state, defined by the selected presenter contract rather than `telorgon-platform`.

Transitions may be redundant and are idempotent. One view may be suspended or occluded while another
remains active. `Closed` is terminal. A forced native destruction is distinct from a cancellable
`CloseRequested` event.

For a managed Winit host, windows and graphics surfaces are created only after the event loop first
resumes. On platforms where suspension invalidates native surfaces, the old surface generation is
retired before the suspended state is delivered to renderer-facing code. Resuming creates a new
generation; portable objects do not assume native-handle continuity.

### 4.2 Close protocol

`CloseRequested { reason }` enters the normal routed event path. The application may accept, reject,
or defer it. A host-enforced `Destroying`/`Destroyed` notification cannot be rejected and cancels
input captures, text sessions, data transfers, platform requests, frame requests, and presenter work
for that view. Managed `telorgon-app` exits only when its configured application policy says that the
last relevant view closed; an arbitrary close request must not always terminate the process.

### 4.3 Metrics and safe areas

`ViewMetricsSnapshot` is revisioned and changed atomically. It includes:

- physical pixel extent;
- logical extent and logical-to-physical transform;
- scale factor and display orientation/transform;
- safe drawing insets;
- safe gesture insets;
- IME occlusion and other avoid regions;
- display/color/HDR properties needed by the renderer contract;
- the coordinate spaces in which every field is expressed.

An extent with either dimension zero is not renderable. It is not clamped to `1x1`. The runtime may
continue layout/state work, but the presenter neither acquires nor presents until a nonzero current
extent exists.

Input events cite the metrics revision used for coordinate conversion. If a host cannot map an event
against that revision, it reports a conversion failure or queues a corrected event; it does not
guess using whichever scale factor is newest.

## 5. Managed and embedded entry points

### 5.1 Managed application

The primary application path remains intentionally small:

```rust
telorgon::application(NotesApplication::default())
    .window(WindowOptions::new("Notes").size(960, 640))
    .renderer(RendererPreference::Auto)
    .run()
```

`run()` selects compiled platform/presenter adapters, creates the event loop, constructs windows only
when lifecycle permits, and blocks on the calling thread until the application exits. It does not
start a hidden server or unmanaged background process. Multi-window APIs create additional declared
views through the same managed host.

`telorgon::request_exit()` requests clean shutdown of all currently running managed GUI and Linux
compositor hosts in the process. It is thread-safe and wakes idle hosts through their existing
event-loop wake mechanisms. Calls return immediately; they do not interrupt callbacks or terminate
the process. Hosts return through their normal shutdown paths. Repeated requests are coalesced,
calls without a running host do nothing, and requests do not carry over to subsequent runs.
Embedded and headless runtimes remain under their caller's lifecycle control.

Managed GUI and DE entrypoints initialize one process-wide session launch service before callbacks.
Child environments are explicit snapshots; the host never needs to mutate global display exports.
DE exit quiesces launching and requests normal client close while continuing event dispatch for save
prompts. A client-close timeout cancels logout; after windows close, supervised children receive a
bounded graceful termination request without automatic force-kill. SIGTERM/SIGINT use the same DE
exit request. Recovery offers saved launch specifications, not arbitrary application memory. See
[Managed sessions and process launching](SESSION_AND_PROCESS_LAUNCHING.md) for the current API,
platform restrictions, persistence contract, and qualification gaps.

The builder exposes policy and preferences, never backend-native objects. An advanced managed API
may accept explicitly constructed service/presenter factories, but the normal API must not require
knowledge of Vulkan swapchains or Winit event types.

### 5.2 Embedded/engine host

The host-driven path is windowless and presentation-neutral:

```rust
let mut ui = UiHost::new(ui_device, platform_services);
let view = ui.create_view(initial_metrics, root_component)?;

let prepared = ui.prepare(view, frame_input)?;
prepared.record(&mut command_recorder, RenderArea::new(origin, extent))?;
```

The exact renderer arguments are finalized with their backend package, but these ownership rules are
fixed:

- the caller owns event loops, windows, devices, queues, render targets, submission, synchronization,
  presentation, frame cadence, and worker threads;
- Telorgon records only in the declared render area and declared render-graph phase;
- multiple Telorgon views may render into different areas or targets in one engine frame;
- the caller supplies view metrics, platform events, a monotonic clock, wake scheduling, optional
  executor hooks, and any desired platform service implementations;
- absent services report `Unavailable`; `UiHost` does not create Winit or native service objects as
  a fallback;
- dropping a view retires its work through the renderer's completion protocol rather than waiting
  for global device idle.

A test/headless host is an embedded profile with a fake clock, deterministic services, and an
explicit software or GPU offscreen renderer. It is not part of `telorgon-app`.

## 6. Scheduling, threading, and reentrancy

The UI runtime has one logical writer. The host chooses which thread owns it, but all calls that can
mutate components, focus, text, semantics, layout, or scene state occur serially on that owner.

Platform callbacks follow this path:

```text
native callback -> validate/copy minimal immutable data -> event queue -> runtime owner -> dispatch
```

They never call user components synchronously. Service completions and worker tasks return through a
host event-loop proxy or the embedded host's completion queue. Locks held by native callback code are
released before a completion enters the runtime.

After processing events, the runtime returns a scheduling decision containing:

- whether update/layout/semantics/scene work remains;
- views needing redraw;
- the next monotonic deadline, if any;
- pending host wakes or service completions.

A managed Winit adapter requests redraw for dirty views and chooses `Wait` or `WaitUntil` in
`about_to_wait`. `Poll` is used only for an explicit continuous-frame profile. Drawing occurs in a
redraw/frame callback, not on every event. An embedded host maps the same decision into its frame
scheduler.

No package may spawn a thread or install an async runtime merely because a dependency can do so.
Threaded/executor-backed adapters declare that requirement in their capability descriptor and are
enabled by the managed host or explicitly supplied by an embedding host. Blocking OS calls must not
run on the runtime owner; if no allowed executor exists, the operation is unsupported.

## 7. Input contract

`telorgon-input` owns neutral input values. Platform packages translate native events at the boundary.
All input carries `ViewId`, `EventStamp`, device identity/class where known, modifiers, and explicit
coordinate-space metadata.

### 7.1 Pointer, touch, and pen

- IDs are stable for the contact lifetime and generation-safe across device reconnects.
- Phases are explicit: enter/leave, hover/move, down, move, up, cancel, and capture change as
  applicable.
- Events preserve changed button, complete button state, pressure, tilt, twist, contact geometry,
  primary-contact status, and tool class when supplied.
- View-space positions are canonical logical coordinates. Physical coordinates and the metrics
  revision are retained where available.
- An absent position is `None`; cursor leave never uses `(-1, -1)` or another sentinel.
- Focus loss, suspension, or forced destruction produces cancellation/reset for active contacts,
  captures, hover, and pressed-button state.

Mouse emulation synthesized from touch is marked synthesized so gesture recognizers do not handle
one action twice.

### 7.2 Scrolling

Scroll preserves the source unit (`Pixels`, `Lines`, or `Pages`), both axes, precision flag when
known, and gesture phase/momentum when available. A platform adapter never multiplies line deltas by
a magic pixel constant. Component behavior or theme policy decides how logical line/page deltas map
to content movement.

### 7.3 Keyboard

Keyboard events preserve:

- physical key identity;
- logical key meaning;
- produced text, if the platform supplies it outside an active IME composition;
- key location, pressed/released state, repeat, synthetic flag, and modifiers;
- native/unidentified key data only in a diagnostic extension, not as the portable primary key.

Key bindings choose physical or logical matching explicitly. While an IME session is composing,
IME commit is the only text mutation path; `KeyEvent.text` must not insert duplicate characters.

### 7.4 Raw device input

Unfocused `DeviceEvent`-style raw motion is not ordinary UI input because it is not associated with a
view and may arrive without focus. It is available only through an explicit raw-input/game-host
capability with separate routing and permission policy. `telorgon-embed` can coexist with an engine's
raw input without intercepting it.

### 7.5 Mapping invariants

Each platform adapter maintains a conformance table from every relevant native event to a neutral
event, an explicit ignored reason, or a documented unsupported capability. Translation tests cover
scale changes, reordered focus events, cancellation, repeat, synthetic input, and stale view IDs.
Unknown enum variants are handled defensively and do not panic the host.

## 8. Text input, IME, and virtual keyboard

Gate 7's revisioned text buffer, UTF-8 byte `TextOffset`, selection, composition, and edit-batch
contracts remain canonical. A valid `TextOffset` is a UTF-8 character boundary. Native adapters own
conversion to native indexing conventions.

### 8.1 Session model

A focused editable opens a generation-aware `TextSessionId` and publishes a revisioned
`TextInputSnapshot` containing:

- editable revision and selection;
- composition/marked range, if any;
- bounded surrounding text with its base offset;
- caret and selection geometry in view coordinates;
- input purpose, capitalization, correction/spelling policy, secure-entry state, multiline policy,
  and return-key action;
- virtual-keyboard visibility preference.

Native callbacks cite the session and text revision they observed. The adapter converts native
ranges to Gate 7 offsets against that snapshot. A stale or invalid range yields `Stale` and requests
a full resynchronization; it is never applied to a newer buffer by coincidence.

Composition start/update/commit/cancel is one ordered edit protocol. Selection and marked-range
changes that belong to one native callback are applied atomically. Secure-entry surrounding text and
committed contents are redacted from logs, traces, crash metadata, semantics, and clipboard unless
the application performs an explicit allowed operation.

### 8.2 Platform profiles

- Desktop Winit's IME enablement, preedit, commit, cursor area, and purpose hints form the minimum
  desktop adapter. The adapter enables IME only while a session is active and disables duplicate
  keyboard text insertion during preedit.
- Android's operational adapter implements the `InputConnection` behavior required by the selected
  view/Activity integration, including UTF-16 range conversion, batch edits, extracted/surrounding
  text, selection, composing regions, editor actions, and restart/resync behavior.
- iOS implements the required `UITextInput`/`UIKeyInput` bridge, including marked text, selected
  ranges, tokenizer/direction queries, and coordinate conversion.
- macOS uses the AppKit text-input client bridge required for marked text and candidate-window
  placement.

Winit preedit/commit alone is not evidence for an operational mobile GUI profile. If a platform API
cannot express the full session contract, that target remains bring-up/experimental.

## 9. Accessibility contract

`telorgon-accessibility` owns a renderer- and platform-neutral semantics model. The model is generated
from component behavior/state and layout geometry, never reconstructed from pixels.

Each live view has a semantic tree with:

- stable per-view node IDs and a monotonically increasing tree revision;
- role, state, value, label/description, actions, relationships, live-region data, and text ranges;
- bounds/transforms in a declared coordinate space;
- keyboard focus and assistive-technology focus as distinct values;
- imported semantic attachments for externally owned shell content when the host supplies them.

Activation sends a complete snapshot. Later updates are revisioned deltas computed only after the
corresponding behavior and layout passes. Closing or replacing a view invalidates its tree generation.
Actions from assistive technology carry view/node generation and observed revision, enter through the
platform event queue, and are validated before semantic-action dispatch.

### 9.1 AccessKit adapter

The initial desktop implementation uses `accesskit` in a neutral mapping package and
`accesskit_winit` in the Winit adapter. The native adapter is created before its window becomes
visible. The Winit event is offered to the adapter before Telorgon translates it. The initial-tree
callback returns a precomputed immutable snapshot synchronously; it must not re-enter the runtime.
Action and deactivation callbacks may arrive from another thread and therefore enqueue through the
event-loop proxy.

Embedded hosts can provide an `AccessibilitySink` and action source, or report accessibility
unavailable. A shell host may attach a client's semantic tree under a namespaced node and transform
its geometry. In the absence of an imported semantic channel, Telorgon exposes the shell-owned surface
container/chrome semantics but does not fabricate client controls with OCR.

### 9.2 Mobile qualification

Android Winit/AccessKit support currently aligns with a GameActivity integration. Therefore:

- NativeActivity remains an allowed renderer and packaging bring-up target;
- the first operational managed Android GUI profile uses GameActivity, unless a separately
  qualified direct `accesskit_android`/native-view adapter demonstrates equivalent lifecycle, IME,
  and accessibility behavior;
- no Android profile is called operational while screen-reader navigation, text editing, focus, and
  action routing are absent.

iOS/macOS adapters map the neutral tree and actions to UIKit/AppKit accessibility objects. Platform
objects cache identity but not independent component truth.

## 10. Clipboard, data offers, and drag/drop

Clipboard and native drag/drop share a neutral data-transfer vocabulary:

```rust
pub struct DataOfferDescriptor {
    pub id: DataOfferId,
    pub formats: Vec<DataFormat>,
    pub source: DataSourceKind,
    pub trust: TrustLevel,
    pub size_hints: Vec<SizeHint>,
}
```

An offer can expose multiple MIME/UTI/native-format mappings. Reads are asynchronous, cancellable,
size-bounded, and may stream. A provider may produce data lazily, but the platform adapter owns it
until the native paste/drag operation releases it. Reads validate the offer generation and requested
format. File paths/URIs, HTML, images, and custom data are never coerced into text silently.

Convenience text and image operations are layered over this protocol. Platform clipboards may
change ownership at any time and publish a new snapshot/change notification when supported. Sensitive
data is not logged, and untrusted client offers are bounded before allocation or decoding.

Internal widget drag/drop remains a runtime gesture/behavior protocol. Crossing the OS boundary is a
separate `DataTransferService` operation with native policy and user-gesture requirements. Winit's
file hover/drop events can implement `InboundFilesOnly`; full native outbound drag, rich inbound
offers, hover actions, and promised/lazy data require a native platform adapter.

`arboard` may implement a Tier-A desktop text/image bridge with target-selected features. It is not
the neutral model and cannot silently fall back to a no-op clipboard. A failed platform clipboard
initialization makes the capability unavailable and surfaces a diagnostic.

## 11. Platform service set

The initial service registry contains narrow typed handles. It is extensible without adding a
variant to a global command enum.

| Service | Core responsibility | Key capability dimensions |
| --- | --- | --- |
| `WindowService` | title, state, size constraints, attention, close | per-view operations, user-gesture and policy limits |
| `ClipboardService` | clipboard offers and publish/clear | formats, change events, selection clipboard, permissions |
| `DataTransferService` | native drag/drop and share-style transfer | inbound/outbound, formats, gestures, streaming |
| `TextInputService` | native IME and virtual keyboard session bridge | composition, surrounding text, purposes, geometry |
| `AccessibilityService` | publish semantics and receive actions | tree/actions/text/range features |
| `CursorService` | standard/custom cursors, visibility, and position policy | custom images, animation, confinement/lock |
| `DisplayService` | displays, scale, transforms, color/HDR, safe/avoid regions | change notifications and accuracy |
| `UriService` | open an external URI under policy | supported schemes, gesture/permission policy |
| `FileDialogService` | open/save/folder selection | async, filters, multiple selection, sandbox tokens |
| `MenuService` | native menus/tray surfaces where applicable | roles, accelerators, validation, status/tray support |
| `NotificationService` | system notifications and response events | authorization, actions, badges |
| `HapticsService` | semantic haptic effects | device support, user settings, intensity control |
| `PowerService` | optional idle/sleep inhibition scoped by lease | operation and policy support |
| `RestorationService` | optional platform state restoration tokens | view/session scope and size limits |

Service methods take typed request structures and return request/completion outcomes. Long-lived
effects such as cursor confinement or sleep inhibition use RAII leases and are released on drop,
view close, suspension, or host revocation.

Haptics use semantic effects rather than device-specific waveforms in portable component code and
respect platform/user accessibility settings. File dialogs, menus, notifications, and power policy
are optional adapters; they are not dependencies of `telorgon-runtime` or `telorgon-app`'s minimal
profile. Blocking dialog libraries and heavyweight desktop toolkit features are never baseline
dependencies.

## 12. Native handles and external GPU resources

### 12.1 Borrowed window/display handles

`raw-window-handle` is confined to presenter and platform-adapter boundaries. A handle is borrowed
only for the duration of the operation that creates/recreates a surface and is never retained in
portable runtime, scene, component, or renderer-neutral resource state. Handle acquisition may fail
while a view is inactive or its native surface is unavailable.

The pair `(ViewId, NativeSurfaceGeneration)` guards all surface work. Work referring to an older
generation is cancelled or retired; it never resolves a fresh handle and assumes it is the same
surface.

### 12.2 External content in scenes

Portable shell/scene data stores an opaque `ExternalContentId` plus neutral metadata: logical extent,
transform, sampling/color/alpha intent, damage, and acquire/release requirements. The selected
backend's resolver turns that ID into a backend-specific typed import lease. Core packages contain no
generic `NativeHandle(u64)` and no universal enum of Vulkan/Metal/D3D resources.

An import lease is linear (`!Clone`) unless the backend explicitly implements reference-counted
duplication. It identifies resource generation, acquire synchronization, allowed access, and release
obligation. Dropping an unsubmitted lease follows the adapter's cancellation path; submission
transfers it to a completion-tracked retirement path.

### 12.3 Vulkan Linux external image profile

The first Linux shell interop profile uses a backend-specific payload equivalent to:

```rust
pub struct VulkanDmaBufImport {
    pub planes: Vec<DmaBufPlane>, // each owns its fd and plane metadata
    pub drm_fourcc: u32,
    pub drm_modifier: u64,
    pub extent: PhysicalExtent,
    pub color: ExternalColorDescription,
    pub alpha: AlphaMode,
    pub acquire: Option<OwnedSyncFd>,
    pub release: ReleaseRequest,
}

pub struct DmaBufPlane {
    pub memory: OwnedFd,
    pub memory_index: u32,
    pub offset: u64,
    pub row_pitch: u32,
}
```

The real type uses platform-gated owning FD types. Import validates device extension support, format
and modifier properties, plane count, extent, offsets, row pitches, allocation sizes, dedicated-only
requirements, usage, and color/alpha interpretation before Vulkan object creation. A successful
Vulkan FD import consumes the transferred FD according to the relevant extension rules; failed or
cancelled paths close every still-owned FD exactly once.

Acquire synchronization is imported and waited before the first GPU access. A release fence/sync FD
is exported or signaled only after the final GPU read for that generation. Cache reuse does not defer
the host-visible release obligation. Device loss, view destruction, rejected buffers, and partial
multi-plane failure all have explicit close/release paths. DRM modifier and explicit-plane layout are
never inferred from pixel format alone.

Metal, Direct3D, Nintendo, and PlayStation adapters define equally typed native payloads inside their
backend packages, with the same linear ownership, generation, validation, acquire, release, and
retirement invariants. Closed SDK details do not leak into public portable packages.

## 13. Shell and protocol-host integration

`telorgon-shell` remains protocol-neutral. A compositor/protocol host owns Wayland/X11/native protocol
objects, security policy, client process management, seats, protocol serials, buffer lifetimes, and
output publication. It translates those into Gate 8 snapshots and requests plus the contracts here.

Shell operations that require causality from a recent native input event carry an opaque, scoped,
single-use grant:

```rust
pub struct ShellPlatformRequest<T> {
    pub view: ViewId,
    pub observed_snapshot_revision: u64,
    pub gesture: Option<UserGestureGrant>,
    pub request: T,
}
```

`UserGestureGrant` may encode a seat/serial/token internally, but portable shell code cannot inspect,
clone, persist, synthesize, compare, or log it. The host validates scope, age, generation, focus, and
single use. A stale snapshot or invalid grant yields a typed denial rather than a protocol call.

Imported client buffers use `ExternalContentId` and the backend import lease. Damage, transforms,
safe areas, input regions, presentation feedback, and release completion cross explicit adapter
interfaces. Ordinary shell UI never acquires Wayland dependencies merely to render a taskbar or
window chrome; the explicit Linux shell-environment assembly owns those dependencies and adapts
them at the host boundary.

## 14. Rust dependency baseline

Versions are fixed in the workspace and changed only by an explicit dependency-review commit. The
initial line is:

| Dependency | Initial role and policy |
| --- | --- |
| `winit = 0.30.13` | managed desktop window/event loop and initial platform events; adapter-only |
| `raw-window-handle = 0.6.2` | short-lived presenter/native-handle borrowing; adapter/backend-only |
| `accesskit = 0.24.1` | neutral semantic mapping implementation; not required by core semantics |
| `accesskit_winit = 0.33.2` | Winit accessibility adapter, `default-features = false`, target features selected explicitly |
| `accesskit_android = 0.7.5` | optional direct Android view bridge if the chosen Activity path needs it |
| `arboard = 3.6.1` | optional Tier-A desktop clipboard bridge, `default-features = false` |
| `gilrs = 0.11.2` | optional `telorgon-input-gilrs` gamepad adapter, never required by UI input |
| `muda = 0.19.3` | optional native-menu adapter; default Linux GTK/libxdo stack is not baseline |
| `rfd = 0.17.2` | prototype/opt-in dialog adapter only; never the baseline service contract |
| `windows` | Windows adapter only, one compatible workspace version and minimum feature set |
| `objc2`, `objc2-app-kit`, `objc2-ui-kit` | Apple adapters only, one compatible version family and minimum framework features |
| `jni` | Android adapter only and aligned with the selected Activity glue |

Target features are specified under target-specific dependency tables. Platform crates are not
optional dependencies of neutral crates. `accesskit_unix` executor/thread behavior is reviewed and
declared by the Unix adapter; a no-thread embedded build must not pull it transitively. `arboard` image
and Wayland data-control features are enabled only for profiles that qualify those operations.

The Vulkan renderer continues to use Gate 3's `ash`-first stack. No CPU renderer is inserted into the
Vulkan application path. Software rendering remains an explicit test/fallback backend and does not
mask a failed Vulkan requirement.

## 15. Package and file blueprint

Files have one primary responsibility. This is the target layout, not permission to move every file
in one unreviewable change:

```text
crates/telorgon/src/platform/
  lib.rs                 curated exports only
  id.rs                  opaque/generational platform identities
  stamp.rs               monotonic event stamps and sequencing
  capability.rs          support, permissions, limits
  lifecycle.rs           lifetime/activity/visibility state
  view.rs                view snapshots and close protocol
  metrics.rs             scale, extent, insets, transforms, environment
  event.rs               platform event envelope
  request.rs             request IDs and terminal outcomes
  clock.rs               injected monotonic clock
  schedule.rs            wakes, deadlines, redraw decisions
  error.rs               structured platform errors
  services/
    mod.rs               registry and curated exports
    window.rs
    clipboard.rs
    data_transfer.rs
    text_input.rs
    accessibility.rs
    cursor.rs
    display.rs
    uri.rs
    file_dialog.rs
    menu.rs
    notification.rs
    haptics.rs
    power.rs
    restoration.rs

crates/telorgon/src/platform_winit/
  lib.rs
  application_handler.rs lifecycle callback orchestration only
  view_registry.rs        Winit WindowId <-> ViewId and generations
  event_proxy.rs          cross-thread immutable completion delivery
  translate/
    mod.rs
    keyboard.rs
    pointer.rs
    touch.rs
    ime.rs
    window.rs
  schedule.rs             about_to_wait/control-flow/redraw mapping
  handles.rs              scoped raw-window-handle access
  error.rs

crates/telorgon/src/accessibility/
  lib.rs
  id.rs
  node.rs
  tree.rs
  update.rs
  action.rs
  text.rs

crates/telorgon-accessibility-accesskit/src/
  lib.rs
  role.rs
  node.rs
  update.rs
  action.rs

crates/telorgon-accessibility-accesskit-winit/src/
  lib.rs
  adapter.rs
  callbacks.rs

crates/telorgon-platform-clipboard-arboard/src/
  lib.rs
  formats.rs
  service.rs

crates/telorgon-platform-windows/src/   Windows service adapters only
crates/telorgon/src/platform_linux/     Linux desktop service adapters only
crates/telorgon-platform-apple/src/     AppKit/UIKit service adapters only
crates/telorgon-platform-android/src/   Activity/view/Android services only

crates/telorgon/src/embed/
  lib.rs
  host.rs
  view.rs
  input.rs
  schedule.rs
  services.rs
  error.rs

crates/telorgon/src/application_host/
  lib.rs
  builder.rs
  options.rs
  profile.rs
  assembly.rs
  error.rs

crates/telorgon/src/platform_conformance/
  lib.rs
  fake_clock.rs
  fake_services.rs
  lifecycle.rs
  input.rs
  text.rs
  accessibility.rs
  transfer.rs
```

Backend-specific external import types live under their renderer packages, for example
`crates/telorgon/src/renderer_vulkan/external/{mod.rs,dma_buf.rs,sync_fd.rs,import.rs,release.rs}`. Presenter
surface-generation code stays in the selected presenter package. Optional service adapter packages
can remain workspace-private until their API satisfies the contract.

## 16. Ordered implementation slices

These slices refine, but do not replace, Gate 5's P1–P8 platform order:

1. **Neutral spine:** create identities, stamps, lifecycle axes, metrics, scheduling, request outcomes,
   service registry, fake clock, and deterministic conformance host.
2. **Runtime extraction:** move neutral runtime/input ownership out of current `telorgon-app`; preserve a
   temporary compatibility facade.
3. **Winit view host:** add resumed/suspended, multi-view registry, close protocol, zero-extent and
   scale handling, proxy queue, deadlines, and complete window-event translation.
4. **Managed Vulkan P1:** assemble Windows Winit + Vulkan presenter/backend; no software fallback in
   the qualification profile.
5. **Hosted P2:** implement `UiHost`, explicit render areas, fake services, multi-view scheduling, and
   Vulkan host-device/submission contracts.
6. **Text and accessibility:** connect Gate 7 text sessions, Winit IME, semantic snapshots/deltas,
   AccessKit, and keyboard/pointer focus without reentrancy.
7. **Desktop services/P3:** clipboard/data offers, cursor/display, file/URI adapters, Linux Winit
   variants, and platform-specific service capabilities.
8. **Shell interop/P4:** user-gesture grants, imported semantic attachments, DMA-BUF/modifier and
   acquire/release synchronization, real protocol-host integration outside Telorgon.
9. **Apple/P5:** Metal presenter/backend plus AppKit services; qualify hosted and managed macOS.
10. **Mobile foundation/P6:** touch/pen, safe/IME areas, lifecycle restoration, virtual keyboard,
    accessibility, gestures, and haptics using fake platform adapters first.
11. **Android/P7:** renderer/packaging NativeActivity bring-up, then operational GameActivity or a
    separately qualified native-view bridge with complete IME/accessibility.
12. **iOS/P8:** Metal layer/presenter plus UIKit lifecycle, `UITextInput`, accessibility, pasteboard,
    safe areas, drag/drop, and haptics.
13. **Additional APIs:** implement Direct3D/console backends only through the proven RHI, presenter,
    external-resource, and service boundaries, with vendor SDK code isolated.

Every slice updates its ownership ledger, feature matrix, conformance fixtures, evidence manifests,
and migration shims. A slice does not delete a compatibility API until its replacement is exercised.

## 17. Diagnostics, performance, and security

Diagnostics expose counters and structured spans for queued platform events, queue high-water marks,
event-to-dispatch latency, coalesced motion/metrics events, redraw reasons, missed deadlines, service
latency/outcomes, stale generations/revisions, IME resyncs, accessibility snapshot/delta sizes, data
transfer bytes, external import/release latency, and leaked/unretired leases.

The following are correctness failures, not optimizations:

- unbounded platform event, transfer, or completion queues;
- rendering continuously while clean, hidden, zero-sized, or suspended;
- blocking the UI/runtime owner on clipboard, dialogs, protocol clients, or GPU idle;
- re-entering components from accessibility, IME, window, or service callbacks;
- logging secure text, clipboard contents, file payloads, protocol serials, native pointers, or FDs;
- accepting stale view, text, semantic, surface, or external-resource generations;
- permitting an untrusted external extent/stride/count to overflow allocation arithmetic;
- allowing a denied/unsupported operation to appear successful.

High-frequency pointer and metrics events may be coalesced only when order relative to buttons,
focus, scale, enter/leave, composition, or close cannot change observable behavior. Coalescing retains
the newest source stamp and records the number collapsed.

## 18. Acceptance contract

Gate 9 is implemented only when the applicable profile proves all of the following:

1. lifecycle traces include redundant resume/suspend, close cancellation, forced destruction,
   zero extent, scale change, surface regeneration, and two independently changing views;
2. the deterministic host reproduces identical event/action/scene/semantics output under a fake
   clock;
3. input mapping preserves units and identities and resets captures/presses on focus loss;
4. text fixtures cover ASCII, combining marks, emoji/surrogate pairs, bidirectional text, stale
   revisions, composition cancellation, and secure redaction;
5. accessibility fixtures cover initial activation, deltas, focus distinction, text ranges, actions
   from another thread, stale node IDs, and imported shell attachments;
6. clipboard/transfer fixtures cover multiple formats, lazy reads, cancellation, ownership loss,
   oversized/untrusted data, and adapter unavailability;
7. scheduling traces show no idle polling or redraw and no hidden dependency-owned background work
   outside the selected profile;
8. embedded tests prove no window/event loop/presenter/queue/thread creation, correct render-area
   clipping, multiple views, and host-owned synchronization;
9. Vulkan shell tests prove FD ownership on success/failure, multi-plane/modifier validation,
   acquire-before-read, release-after-final-read, rejection, device loss, and no global idle wait;
10. unsupported/denied/stale/failure outcomes appear in API results and evidence rather than only
    logs;
11. operational profiles include real assistive-technology and IME smoke evidence on the target OS;
12. package-feature checks show neutral and embedded-minimal builds do not pull Winit, AccessKit
    platform adapters, native dialogs, a software presenter, or unrequested async runtimes.

The full evidence names, artifact hashes, hardware/driver metadata, and tier rules remain owned by
[Acceptance and qualification](ACCEPTANCE_AND_QUALIFICATION.md).

## 19. Current-to-target migration ledger

The present `crates/telorgon/src/application_host/native.rs` is a useful Winit/Softbuffer proof, not the target
platform architecture. Its responsibilities migrate as follows:

| Current behavior | Target owner and rule |
| --- | --- |
| Winit `ApplicationHandler` plus window registry | `telorgon-platform-winit`; multi-view and generation-aware |
| elapsed-millisecond timestamps | injected monotonic clock and `EventStamp` |
| physical cursor position treated as logical | metrics-revision coordinate translator |
| cursor leave at `(-1, -1)` | optional position plus explicit leave event |
| line scroll multiplied by `24` | preserved `ScrollUnit::Lines` |
| numeric physical key only | physical + logical + text + location + repeat mapping |
| close request always exits | cancellable close protocol and application exit policy |
| zero extent ignored/clamped | explicit non-renderable metrics state |
| clipboard commands ignored | typed unavailable/terminal outcome or real service |
| no IME/touch/accessibility/services | dedicated adapters and qualification suites |
| runtime generic over renderer in native host | renderer-free runtime; managed assembly selects presenter/backend |
| Softbuffer surface owned beside event translation | explicit software presenter; Winit platform package owns no renderer |

Migration preserves any still-used public facade until callers and examples move to the replacement.
The proof must not be expanded into a larger monolith.

## 20. Reference and specification audit

The contract was checked against local, revision-pinned source in `../other-rendering-libs`:

| Source | Revision | Applied lesson |
| --- | --- | --- |
| Xilem/Masonry | `ce7b04d2ba2d9d7a8c364f2ab109e2083121e144` | event-loop proxying, input conversion, AccessKit-before-visible, semantics passes, IME signals |
| Slint | `69ecb713f5c62d1b6fe986ff822a57f22152b4d9` | platform/window-adapter separation, event-loop deadlines, clipboard and AccessKit adapter boundaries |
| Flutter engine | `51fd9afadf309ba5337320bd3653f5345c156cb9` | embedder task runners, multi-view metrics, semantics/actions, platform messages, external textures/compositor ownership |
| Qt Declarative | `3e2d6bd456a8e850bcf641de77d1d5d8bc8419ef` | cross-platform input-method, accessibility, window-system, and scene integration concerns |
| React Native | `2d427ba77bbf17bc487e25bef4d011097ba4fff5` | native service/module boundaries and mobile lifecycle constraints |
| Android platform | `1cdfff555f4a21f71ccc978290e2e212e2f8b168` | native lifecycle/input/accessibility service behavior |
| AndroidX support | `491d5b9a1de8225097e39684c3412f40f227a0f7` | compatibility-layer behavior and mobile integration pitfalls |

These are engineering references, not upstream code to copy. Implementation agents must inspect the
pinned source and the current official specification for the exact subsystem being implemented,
record files/symbols/revision in the source ledger, and preserve Telorgon's ownership model. Local
source does not override a platform specification or license.

Primary specifications to consult per slice include Winit's `ApplicationHandler`, `WindowEvent`,
keyboard and window-handle contracts; AccessKit adapter lifecycle; Android `InputConnection` and
accessibility node-provider contracts; UIKit/AppKit text, accessibility, pasteboard, lifecycle, and
drag/drop contracts; Windows UI Automation and clipboard-provider contracts; and Vulkan external
memory, semaphore/sync-FD, DMA-BUF, and DRM-modifier extension specifications.

Primary starting links:

- [Winit 0.30.13 `ApplicationHandler`](https://docs.rs/winit/0.30.13/winit/application/trait.ApplicationHandler.html),
  [`WindowEvent`](https://docs.rs/winit/0.30.13/winit/event/enum.WindowEvent.html), and
  [`KeyEvent`](https://docs.rs/winit/0.30.13/winit/event/struct.KeyEvent.html);
- [`raw-window-handle` borrowed handle contract](https://docs.rs/raw-window-handle/0.6.2/raw_window_handle/trait.HasWindowHandle.html);
- [`accesskit_winit::Adapter`](https://docs.rs/accesskit_winit/0.33.2/accesskit_winit/struct.Adapter.html)
  and [AccessKit source/documentation](https://github.com/AccessKit/accesskit);
- [Android `InputConnection`](https://developer.android.com/reference/android/view/inputmethod/InputConnection),
  [`AccessibilityNodeProvider`](https://developer.android.com/reference/android/view/accessibility/AccessibilityNodeProvider),
  and [Activity lifecycle](https://developer.android.com/guide/components/activities/activity-lifecycle);
- [UIKit `UITextInput`](https://developer.apple.com/documentation/uikit/uitextinput),
  [`UIAccessibilityElement`](https://developer.apple.com/documentation/uikit/uiaccessibilityelement),
  and [`UIPasteboard`](https://developer.apple.com/documentation/uikit/uipasteboard);
- [Windows UI Automation provider overview](https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-providersoverview)
  and [clipboard operations](https://learn.microsoft.com/en-us/windows/win32/dataxchg/clipboard-operations);
- Vulkan [`VK_KHR_external_memory_fd`](https://docs.vulkan.org/refpages/latest/refpages/source/VK_KHR_external_memory_fd.html),
  [`VK_KHR_external_semaphore_fd`](https://docs.vulkan.org/refpages/latest/refpages/source/VK_KHR_external_semaphore_fd.html),
  [`VK_EXT_external_memory_dma_buf`](https://docs.vulkan.org/refpages/latest/refpages/source/VK_EXT_external_memory_dma_buf.html),
  and [`VK_EXT_image_drm_format_modifier`](https://docs.vulkan.org/refpages/latest/refpages/source/VK_EXT_image_drm_format_modifier.html).

## 21. Work remaining after Gate 9

There is no undefined Gate 10. The architecture planning gates are closed. Remaining work is tracked
as implementation/product packages with explicit qualification:

- exact public Rust spelling may change during the neutral-spine compile prototype without changing
  the invariants here;
- theme syntax and authoring/editor Tier B/C work continues under the theme plan;
- renderer/RHI details are validated by Vulkan first and Metal second before additional APIs;
- platform adapters, operational mobile profiles, and real compositor-host interop follow P1–P8;
- dependency versions are updated only through review and evidence;
- closed console SDK adapters require authorized environments and cannot be qualified from public
  desktop implementations alone.

Any proposed design that changes ownership, lifecycle axes, error visibility, text indexing,
accessibility obligations, embedded-mode non-ownership, or external-resource synchronization must
amend this contract and the affected acceptance tests before implementation.

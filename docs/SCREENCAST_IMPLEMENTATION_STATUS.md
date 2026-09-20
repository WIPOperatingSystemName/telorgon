# Screencast implementation work log

Latest UI addition: [visual picker, output previews and verification](SCREENCAST_PICKER.md).

This records partial implementation of `SCREENCAST_IMPLEMENTATION_PLAN.md`. The full plan remains
incomplete. The monitor/window portal path is connected in source, but no live compositor, portal,
PipeWire, browser or Discord run has qualified it. Do not equate compilation with working sharing.

## Explicit capture declaration

The public [capture API](CAPTURE_API.md) now gates portal startup: a Cargo feature alone no longer
starts the backend. `Capture::desktop()` opts into monitor/window portal sharing. Source masks
control backend advertisement, selection and host admission. Owned-session mode uses existing
environment publication and D-Bus frontend activation; external sessions do not take over shared
services. Direct Wayland and internal capture declarations are accepted as configuration values but
return explicit unsupported errors at startup. They do not advertise unfinished interfaces.

## Implemented owner and renderer path

- `shell/capture.rs`: output/window identities, cursor/frame-rate preferences, checked packed-RGBA
  layout and typed stop reasons. Values do not confer authority.
- `application_host/shell_wayland/capture.rs`: owner-local authorization, source reconciliation,
  requester/source binding, lock revocation, cancellation and terminal resource retirement.
- Limits: eight retained sessions, two per requester, 8192 pixels per dimension, 60 FPS and 512 MiB
  nominal capture storage. Default negotiation reserves eight frame-sized allocations: three producer
  buffers, three PipeWire buffers, staging and target. Exact native allocation alignment/metadata
  overhead is still an outstanding accounting gate.
- Buffer generations now bind frame tickets to their negotiated layout. The owner engine can
  reserve a replacement while retaining the old generation, rejects old-frame delivery, and requires
  explicit GPU/consumer retirement before resuming. The host now compares output layouts and sends
  a bounded replacement request to the existing PipeWire stream. It retains the node ID, waits for
  matching format negotiation and removal of every old transport buffer, then allocates the new
  capture pool after any old GPU frame completes. Intermediate size changes wait for that retirement
  before another generation is admitted. This does not add output hotplug/mode switching to the host;
  its selected KMS mode remains fixed, and live resize behavior is not yet qualified.
- Session identities never recycle. Reservations remain until the owner reports retirement and no
  admitted frame remains outstanding. Scheduler tests cover initial frames, revision coalescing,
  rate limiting, late completion and duplicate completion.
- `renderer_vulkan/readback.rs`: reusable staging, exact receipt validation, mapped copy into caller
  storage and cancelled-frame discard. `target.rs` provides an independent sRGB capture target.
- `compositor_render/capture.rs`: records validated placements plus readback into an explicitly
  submitted host frame. It pins the target and staging through GPU completion.
- The Vulkan shell renderer retains final monitor placements after motion/glass resolution. Capture
  reuses those scene resources, excludes the cursor for hidden mode and submits into its own target.
  The existing GPU completion worker performs the delivery copy. A completion timeout preserves the
  exact receipt and pending job for retry; it is not treated as completed work.
- `capture_streams.rs`: connects approved monitor sessions to renderer slots and PipeWire. At most one
  capture GPU job is outstanding globally, with round-robin session selection. Transport buffers are
  bounded and independent of GPU completion. This is a frame-rate ceiling, not qualified throughput.

The scoped reusable-readback review is recorded in `SCREENCAST_READBACK_AUDIT.md`. Upstream Flutter
and wgpu sources were inspected because the adjacent source library was absent. DMA-BUF export has
not been audited or implemented by this work.

Cursor embedding now uses a separate capture cursor scene, including when the desktop uses a
hardware cursor plane. Hidden streams exclude the desktop cursor; embedded streams append the
capture cursor and track cursor-only revisions. Three composition tests pass for fractional-scale
hotspots, motion without pixel uploads, and off-screen hide/reappearance retirement. A revision
test checks that capture-only cursor movement leaves hidden-stream revisions unchanged. Actual
hardware-plane rendering and cursor pixels remain unqualified.

## Window capture integration

`capture_window.rs` now builds isolated placements from raw client image layers before desktop
motion, glass and culling. It resolves the exact managed WindowId incarnation, includes native
owned subsurfaces/popups, excludes separate toplevels and unmanaged X11 associations, and computes
a bounded physical target spanning client content and popups. Existing rectangular client clips
are retained; server decoration rounded clips are omitted. The capture target clears to opaque
black, so transparent client pixels do not reveal the desktop.

The renderer accepts this separate view and translates the embedded cursor into its coordinates.
The host refreshes snapshots only for active window sessions and tracks their content/geometry
revisions independently of desktop damage. Source composition and receipt lifetime reuse the
existing image scenes and capture target; no external-memory or GPU synchronization API was added.
Five headless composition tests (including the XWayland feature) cover subtree isolation, popup bounds, fractional scale, stale or
unavailable roots, parent cycles, foreign scene references, coordinate overflow and off-screen
content revisions and managed X11 roots excluding unverified override-redirect associations.

The default visual picker now shows eligible managed native/X11 windows and the monitor in separate
categories, filtered by the application's requested source types. It pages at most six thumbnails
and requires selection followed by an explicit Share activation. See [picker details](SCREENCAST_PICKER.md). Discovery snapshots
bind the WindowId to a separate capture mapping epoch: desktop window IDs deliberately survive
unmap/remap, while capture approval epochs do not. Both stale buttons and host admission reject an
old epoch. Source disappearance/minimize stops streams; temporary image unavailability during resize
pauses new capture jobs. Per-window layout changes use the existing PipeWire renegotiation path.
After successful delivery, captured surface revisions receive pacing callbacks without claiming KMS
presentation. The selected source remains fixed for the session. Portal Start metadata is resolved from the current
session layout and output scale at reply time, rather than the window size cached at consent.
A headless regression covers changed window dimensions, fractional scale, missing layout and
monitor logical dimensions; live resizing remains unqualified.

The backend now advertises MONITOR | WINDOW. This is connected implementation, not live qualification.
X11 override-redirect popup ownership remains deliberately untrusted and those surfaces are excluded;
full popup interoperability and actual pixel isolation still need qualification. Exact native storage accounting and the direct capture/DMA-BUF phases remain outstanding.

## Portal, consent and PipeWire integration

The optional `shell-screencast-linux` feature includes the Wayland host and pinned PipeWire bindings.
The managed Vulkan host starts a backend worker and installs the default consent/indicator widgets.
The software host does not start this backend. Compilation alone starts no services.

| Boundary | Current selection |
| --- | --- |
| Backend ScreenCast | Version 3; MONITOR | WINDOW with hidden or embedded cursor |
| Backend Request/Session | Close objects bound to frontend unique owner; Session version 1 and Closed signal |
| Frontend ScreenCast | Supplied by existing xdg-desktop-portal, including OpenPipeWireRemote |
| D-Bus | Existing zbus 5.19 dependency; session bus, dedicated backend name |
| PipeWire binding | Pinned Rust pipewire 0.10.1, API feature v0_3_34 |
| Build evidence | Extracted PipeWire/SPA 1.6.2 development headers; system runtime 1.6.2 |
| Delivery | Packed sRGB RGBA, full range/BT.709 primaries, three negotiated MemFd buffers |
| Capture protocols | Three modern capture XML profiles and wire contracts pinned; no capture globals advertised yet |

`portal_linux` implements CreateSession, SelectSources and asynchronous Start, with bounded session
and request tables. Only the current owner of `org.freedesktop.portal.Desktop` may call those methods.
Session/request Close checks the initiating frontend's unique name. Frontend loss, bus reconnect,
lock and user stop revoke leases. Channel closure carries cancellation independently of queue
capacity. Owner comparisons also cover creation racing a frontend-owner change.

`capture_portal.rs` serializes consent dialogs, binds decisions to the exact request incarnation and
starts the selected source only after approval. Choices and approval messages carry an exact
CaptureSource and mapping epoch, and the original portal source-type mask reaches host validation.
Source buttons are keyed by identity and epoch. It returns the actual PipeWire node when ready.
The default modal chooser and persistent stop controls live in `shell_components/capture/` and
submit intentions; they own no transport/GPU resources. Custom chooser factories and the public CaptureUi decision handle are implemented; see
`SCREENCAST_CUSTOM_CHOOSER.md` for their host authority and bounded-queue contract.

`screencast_linux` owns a PipeWire worker per admitted stream, validates negotiated format, writes
only dequeued delivery buffers, supplies stride/size and monotonic PTS/sequence metadata, and returns
each buffer once. A bounded mailbox retains current/latest frames and returns reusable producer
storage. Missing first-frame data and invalid planes are marked corrupt rather than exposing
uninitialized delivery bytes. Node readiness, errors and stop wake the host. No compositor objects
are accessed from PipeWire callbacks. A worker-exit guard clears queued storage and wakes the host
on normal return or Rust unwinding, including when the mailbox mutex is poisoned. This does not
claim recovery from process aborts or a panic across a non-unwinding native callback boundary.

Portal discovery/configuration inputs and manual qualification instructions are in
`packaging/portal/`. There is no activation command that starts another compositor. The frontend
provides restricted PipeWire remotes; live restricted-node visibility still requires qualification
with the distribution's access-control policy. This is not a security claim about unsandboxed clients
with unrestricted access to the user's PipeWire socket.

The public `Compositor::capture_chooser` hook replaces the default consent widget while retaining
standard sharing/stop controls. The one-shot factory executes on the owner thread and receives a
CaptureUi handle with a read-only snapshot signal and bounded approve/deny/stop methods. The
host revalidates every decision. Public API, deferred-factory and queue-backpressure fixtures run
without starting native services; custom widget focus and live integration remain unqualified.

## Verification evidence

- Thirteen capture owner/scheduler tests pass without native features, including reservation retirement,
  stale source identity, lock and cancelled GPU work. Resize tests cover simultaneous old/new
  reservations, stale completions, retirement ordering, atomic quota rejection and lock revocation
  during negotiation. These tests exercise owner state, not live transport resizing.
- Eleven portal tests pass with `shell-screencast-linux`: lifecycle/quotas/cancellation, registration
  races, unique-owner Close checks, stream signatures, requested source-type preservation, window
  metadata without monitor coordinates, and generated D-Bus wire contracts.
- Five consent UI tests pass, including public decision identity, queue backpressure and inert handles.
  Four mounted regressions verify: replacing a request or a window incarnation retires
  its old activation target, which cannot approve the replacement even when source labels match.
  Remapping the same desktop ID also retires its button; pagination reaches sources beyond the
  first page without reusing old activation targets. A discovery test binds snapshots to mapping
  epochs so registry changes cannot make cached metadata appear current. This exposed a shared button accessibility-name bug;
  buttons now reference their own label storage rather than another control's mutable string.
- All 15 portable `composition_api` tests pass, including equal-label ownership and existing
  control reconciliation, action routing and keyed component coverage.
- Eight PipeWire tests pass: format POD negotiation, bounded mailbox replacement, buffer writes
  with timestamps/stride/sequence plus undersized-plane rejection, and worker-exit cleanup. The
  exit tests inject Rust unwinding with a poisoned mailbox and verify terminal failure, storage
  release, rejection of late frames and an owner wake outside the mutex. These use local data,
  no daemon. Three additional generation tests cover equal-byte-count layout changes, old buffer
  removal after format acknowledgement, duplicate/over-limit buffer events, one pending replacement
  and rejection of frames before acknowledgement.
- The public custom chooser integration fixture and deferred factory unit test pass; the
  shell-wayland-linux build also passes with the screencast feature disabled.
- Full screencast library/test compilation passes. The generated-wire test caught and fixed a tuple
  alias that exposed a struct reply instead of the portal's two reply fields, and explicitly checks
  the lowercase `version` property.
- `embedded-vulkan` and `shell-wayland-linux` compilation were rechecked after portal integration
  and passed without enabling PipeWire dependencies.
  The reusable-readback extent test passed; hardware receipt/reuse/cancellation fixtures compile and
  remain ignored. No live GPU test was run.
- An earlier broad native `capture` filter selected an unrelated XWayland private-client test that
  could not create a client in this environment. It is not recorded as a green native suite.
- Existing feature-specific dead-code warnings remain. No services, applications or background
  native test processes were launched by the agent.

Development headers were downloaded and extracted under `/tmp/telorgon-pipewire-dev`; they were not
installed into the system. This checkout's native build evidence uses:

```sh
PKG_CONFIG_PATH=/tmp/telorgon-pipewire-dev/root/usr/lib/x86_64-linux-gnu/pkgconfig \
BINDGEN_EXTRA_CLANG_ARGS=-I/usr/lib/gcc/x86_64-linux-gnu/15/include \
cargo test -p telorgon --offline --no-default-features --features shell-screencast-linux --lib portal_linux
```

A normal development installation should use its system pkg-config and libclang configuration.
The `/tmp` paths above document this environment only; they are not project build requirements.

## Remaining plan gates

1. Complete exact native resource accounting and capture performance instrumentation; qualify the
   connected resize/buffer-generation path. Qualify real monitor
   frames, color/scale/orientation, cancellation and bounded slow-consumer behavior.
2. Qualify custom chooser focus/accessibility and live backend/frontend reconnect, stream-disconnect,
   consent/focus, restricted-remote and packaging validation. Verify a second portal consumer before
   diagnosing Discord-specific failures. The first usable milestone is not yet proven.
3. Qualify the connected native/XWayland window path for occlusion, move/resize, source replacement,
   callback pacing and pixel isolation. Complete the X11 popup ownership review and associated
   capture behavior. Window capture must never crop the monitor or inherit desktop effects.
4. Implement modern source/toplevel/image-copy capture protocol adapters with explicit trusted-client
   access policy. XML profiles and generated wire checks are now pinned; runtime handlers remain absent.
5. Complete the separate DMA-BUF export/synchronization reference review, negotiated DMA-BUF transport,
   shared-memory fallback and wlr-screencopy compatibility.
6. Run the full release qualification matrix. Repository guidance reserves live GUI, services and
   hardware-presenting runs for the user; compile/state tests cannot satisfy those gates.

No release completion, universal client compatibility, DMA-BUF export, hardware qualification or
production-ready screen-sharing capability is claimed.

## Direct capture protocol build foundation

The desktop protocol catalog now includes version 1 of ext-foreign-toplevel-list,
ext-image-capture-source and ext-image-copy-capture. The machine-readable profile and sorted wire
contract cover their complete interface families, including cross-protocol object arguments and
cursor capture session descriptors. This introduces no runtime globals: native registration still
uses its explicit IMPLEMENTED_GLOBALS table. Trusted-client authorization and capture dispatch are
not implemented by loading XML.

Validation: 12 protocol-generation tests pass, including independent XML field comparison,
static native descriptor references, malformed input and wire-contract mutation rejection.
The separate ignored official wayland-scanner comparison was explicitly run and passed for all
catalog signatures. No display, server or service was launched. The added source data must exist
under the configured wayland-protocols directory; missing files use the existing actionable build
error. No XML source was copied into the repository.

Inspected installed XML SHA-256 values:

- ext-foreign-toplevel-list-v1: `402d97291a4041e377d9cc3930af7507bd9752edfc19810c2b4c8cf34aac3a3d`
- ext-image-capture-source-v1: `eaa9e6c3b9c254762c2d8cbb691192bea777e221f18fbf0ba78c0feb02d54e77`
- ext-image-copy-capture-v1: `41a446653f788fabb404cab3168c0bd667c1ff4b54f1f0bb1e18810a1f47d73f`

These hashes record audit inputs, not an exact-file build restriction: the wire contract checks
compatible message definitions and permits supported future XML additions. This step changes
protocol descriptors only and introduces no graphics allocation or synchronization mechanism.

The pure `compositor_wayland/capture.rs` frame validator implements the version-1 request
ordering and numeric frame errors from the installed ext-image-copy-capture XML. Attachment
replaces the previous buffer before submission; capture requires a buffer and occurs once;
post-capture mutations are rejected. Valid damage is accepted without accumulating client-sized
region lists because initial delivery will copy the whole buffer. Destruction suppresses late
ready/failed delivery without asserting GPU retirement. Three no-default-feature tests pass for
buffer replacement, error precedence, repeated completion, cancellation and extreme damage values.
This state is not yet connected to native request dispatch and does not advertise capture support.

Access integration inspection found that `XwaylandAccess::configure_display` installs the display's
single global filter. Capture authorization must compose with this existing filter rather than
replace it, and bind/request dispatch must revalidate authorization independently of registry
visibility. Native runtime global creation is an explicit table, separate from XML availability.
No graphics ownership mechanism was changed by the frame validator.

Capture session validation now retains its single-frame slot until frame resource destruction,
including after ready/failed completion. Explicit destruction releases the slot once; later Rust
cleanup of that old frame cannot release a replacement frame's slot. Dropping the session leaves
existing frames usable, as required by the XML session destroy request. The frame constructor is
private so adapters must allocate through the session. Five portable tests cover these session
rules plus the frame request checks above. This is protocol bookkeeping only: native resource
callbacks, source authorization, buffer validation and renderer completion routing remain pending.

`Display::add_global_filter` now composes owner-installed restrictions conjunctively. XWayland
configuration uses it, preserving any independently installed restriction. The callback retains a
stable boxed policy address while the internal list grows, and policy panics deny access. The
existing replacement API explicitly documents its replacement semantics. Installation is intended
before globals/clients; adding registry restrictions does not revoke already-bound resources.
Capture bind/request authorization and revocation must still be enforced by its native adapter.
The focused policy test uses only Rust predicates and opaque pointer comparisons (no display or
client creation), covering both independent denials and panic handling.

`compositor_wayland::CaptureAccess` supplies an explicit owner-thread connection capability for
modern capture/discovery globals. It installs an additive deny-by-default filter, creates at most
eight host-provided privileged socket connections on its original display, and checks live
OwnedClient identities. Weak handles avoid extending client ownership; expired/disconnected entries
cannot authenticate reused native client addresses. Display identity is an Rc token, not a reusable
native address. Revocation removes authorization before disconnecting the client, outside the
policy's RefCell borrow. No PID, credential or portal approval implicitly authorizes these sockets.

Feature compilation and a pure unknown/expired-client denial test pass. Native socket creation,
registry bind behavior and revocation callbacks remain unqualified. This public capability is not
installed by the managed host yet, and no capture globals or native capture handlers are enabled.
Adapters must share the policy and recheck it at bind/request and delivery boundaries.

Native output capture-source dispatch now creates registered ext_image_capture_source_v1 resources
bound to the requested native output ID. It validates object kind and client ownership. Source
objects reuse normal resource destruction and carry their output independently of manager lifetime.
Native bind and request entry points deny capture kinds unless the shared CaptureAccess policy
allows the live client. The native state defaults to no access policy. No manager is added to the
runtime global table, and there is not yet a host enablement API: session creation, source liveness,
buffer delivery and completion still need integration. The shell-wayland-linux feature compiles;
this is compile evidence only, not a native protocol interaction test.

Native image-copy manager/session/frame request dispatch is now connected to the pure lifecycle
validator. Session creation validates the paint-cursor option and captures the source output ID;
frame creation enforces the one-live-frame rule with the protocol's duplicate_frame error. Buffer
attachment verifies native object type and same-client ownership. Damage/capture requests use the
numeric frame errors, and successful capture records the pending buffer ID once. Registered native
resource destruction removes session/frame bookkeeping; existing frames retain their copied source
and lifetime slot after session destruction. Authorization checks cover all these new resource kinds.

This compiles with shell-wayland-linux. These globals remain unregistered: initial/updated buffer
constraints, pointer-cursor sessions, buffer storage retention and validation, source liveness,
host scheduling and completion events are not yet connected. A pending buffer ID is not an assertion
that its storage is retained or that a capture was delivered. Full native interaction remains
unqualified; the portable lifecycle tests validate only the shared state rules.

Native output session creation now snapshots the enabled output's current physical mode dimensions
and emits buffer_size, mandatory wl_shm ARGB8888 format, and done in order. Missing/disabled outputs
produce stopped instead; a capture submitted against that stopped snapshot produces failed(stopped)
once and clears pending delivery. Frame snapshots preserve the session's constraint dimensions after
session destruction. ARGB8888 delivery requires conversion from renderer RGBA on little-endian hosts;
that conversion is not yet connected. Output changes after creation still require constraint updates
and source-liveness handling. Runtime globals remain disabled pending that and buffer/delivery work.

Direct output capture constraints now refresh after committed output updates and before/after frame
requests. Existing sessions stop at lock admission, before the locked event, and remain stopped on
unlock. Missing/disabled outputs also stop sessions; a stopped session cannot retarget a replacement.
Pending frames fail once with stopped or buffer_constraints when their source disappears or saved
physical dimensions change. Frames surviving session destruction are checked independently. A
configured but unsubmitted frame records source loss, so later capture fails rather than hanging.
Native compilation validates this integration; runtime resize/lock event ordering remains unqualified.
The host's existing output update API still rejects hotplug/global reconstruction, so this does not
claim new multi-output/hotplug support or completed direct capture delivery.

Direct frame submission now validates a matching ARGB8888 SHM buffer and retains a duplicated backing
FD with checked dimensions, stride and extent. Invalid destinations fail with buffer_constraints;
source invalidation and resource destruction release retained destinations. The new owned destination
writer accepts exact packed RGBA input, converts to native-endian ARGB8888 a row at a time, and leaves
client padding untouched. It uses positioned file writes rather than client-controlled mmap access;
backing size is checked again before writing. Concurrent client truncation can still race that check,
so this is not a claim of immutable backing storage. Write errors must produce failed, never ready.
The writer is not invoked by native dispatch and still needs a delivery worker and completion route.

Retained SHM destinations now share an atomic cancellation flag with an owner-side guard. Dropping
the native frame cancels the flag even after the destination moves to a worker; source invalidation
also cancels before releasing the destination. Writers check before work, before each row write, and
before reporting success. This does not interrupt a positioned write already entered, revoke bytes
already copied, or replace the required owner-side completion authorization check. The destination
fixture now also verifies that a writer surviving its dropped owner returns an error without
modifying the backing file. Worker scheduling and ready-event delivery are still pending.

A crate-private direct-capture worker handoff now takes one admitted request at a time and transfers
its owned SHM destination, source dimensions and cursor mode without native pointers. The host must
call it only when its shared scheduler/resource limits admit work and return either write or failure
completion. Completion carries an Arc identity for the originating native compositor, preventing a
late completion from matching reused object numbers in another compositor instance. Owner routing
rechecks source state and access, ignores destroyed/already-terminal frames, and emits transform,
full damage and split monotonic presentation timestamp before ready. Write failures emit failed.
No wl_buffer.release is used for capture destinations. These methods are not yet called by the host;
the worker/renderer scheduler integration and runtime qualification remain incomplete.

The shared renderer CaptureJob now optionally carries a direct destination with its source timestamp
and transform. The existing Vulkan completion worker retains this payload through GPU wait timeouts,
then performs SHM writing after successful readback (or constructs failure after terminal GPU error).
The resulting opaque direct completion travels in VulkanCompletion and the host routes it to native
completion validation. Portal jobs set the optional payload to None. This reuses existing submission
receipts/readback storage; no new GPU allocation/synchronization primitive was introduced. Host direct
request admission and shared session scheduling still need to populate these jobs, and no direct
capture globals have been enabled. This is connected worker plumbing, not an end-to-end stream.

Shared capture requester identity now explicitly distinguishes Portal(u64) from Direct(ClientId).
Direct worker jobs carry the registered native client ID for host admission. The focused portable
regression confirms that equal numeric IDs cannot stop one another's sessions, consume one another's
per-requester quota, or revoke one another on disconnect. The optional portal feature build passes
after updating its requester construction. This prepares shared scheduler admission; direct requests
are still not scheduled by the host.

GPU capture admission is now serialized by CaptureSessions across all requester domains, replacing
the portal stream table's local in-flight check. A cancelled session keeps its pending slot until
GPU completion retirement; stopping alone cannot let another transport submit work over it. The new
portable regression interleaves portal and direct requests, verifies cancellation/retirement ordering,
and checks that denied admission consumes neither source revision nor pacing eligibility. All 15
capture-owner tests pass. This shared admission invariant precedes direct host scheduler integration.

Direct frames/jobs now carry their originating session object ID. The host-facing liveness query
refreshes source state and includes surviving frame resources after session-object destruction,
allowing reusable host capture storage to remain associated with the correct session. It does not
replace GPU retirement accounting. A regression checks orphan-frame identity and source-loss
termination without creating a native display. Jobs also provide an opaque failure completion before
ownership transfer so renderer submission errors can terminate the protocol frame instead of leaving
it pending. Actual host scheduler admission/reuse remains the next integration step.

`capture_direct.rs` adds host-owned reusable storage keyed by native capture session, with typed
requester admission through CaptureSessions and the existing nominal memory quota. Preparation leaves
a request owned by the caller when pacing or shared GPU admission defers it; accepted work uses the
same CaptureJob and output rendering path. The managed host now instantiates this owner, routes its
GPU completions back to reusable buffers, and retires source-dead sessions only after their buffer
returns from GPU work. This currently supports the managed host's existing native output 1 mapping.
Direct request intake, resized-buffer renegotiation, cursor preparation, scheduler fairness and
submission-failure retirement still need connection before runtime enablement. No end-to-end direct
capture capability is claimed by this partial host integration.

Direct host admission now handles changed frame dimensions using CaptureSessions generation
renegotiation. It defers while the previous GPU buffer is outstanding, reserves old plus new nominal
storage before replacement allocation, retires the old host buffer, and resumes the same authorized
session. Allocation/transition errors drop owned buffers and stop/retire the idle host session;
quota rejection occurs before modifying its existing generation. Feature compilation and all 15
shared-owner tests pass, including their existing generation accounting/retirement cases. These tests
do not execute Vulkan replacement allocation. Exact native/deferred allocation accounting and live
resize qualification remain open gates, as does scheduler request intake.

Capture-only cursor preparation is now part of the base Wayland host rather than gated on the
PipeWire/portal feature. Direct session/frame cursor requirements and optional portal stream
requirements feed the same composition path. Cursor preparation errors stop direct embedded-cursor
sessions and invalidate their frames, preserving hidden-cursor sessions; lock cleanup runs with or
without the portal feature. All three cursor geometry/resource-retirement tests pass under
shell-wayland-linux alone. This removes a dependency obstacle to direct request scheduling but does
not enable capture globals or qualify hardware cursor pixels in a live stream.

The managed loop now takes direct protocol requests, admits/reuses their host session, prepares and
submits shared renderer jobs, and retains pacing-deferred requests. A pending direct request bounds
idle waiting by the output refresh period. Direct and portal scheduling alternate priority after
successful GPU admission while the common owner permits only one pending capture. Admission and
preparation failures return protocol failure; timestamps currently use CLOCK_MONOTONIC at capture
scheduling. Matching the protocol's source-presentation timestamp semantics remains a qualification
and refinement gate. Renderer submission errors still propagate through the host's fatal renderer
error path because post-submission retirement must be resolved safely; independent stream recovery
is not claimed. Globals remain disabled, and stationary-scene wake paths, session fairness, native
interaction and actual pixels remain unqualified.

The host's unchanged-scene branch no longer continues the entire event loop before capture
scheduling. It exits only the primary-render section, allowing explicit/initial capture requests to
use retained scene placements on a stationary desktop. Capture submission and deferred direct
pacing wakeups are gated on an enabled seat, so a suspended seat does not submit new capture work or
poll its pacing timer. Compile checks cover the control-flow change; actual idle-desktop start and
seat suspend/resume remain live qualification cases. This fixes the identified skipped-scheduler
branch but does not establish complete idle/liveness qualification for every transport.

Pacing-deferred direct jobs are now revalidated before shared admission, including while another
capture occupies the GPU slot. Validation refreshes source state and checks compositor identity,
frame existence, pending state, session identity, dimensions, cancellation and live connection access.
Stale jobs are dropped rather than admitted for unnecessary GPU work. Source refresh owns terminal
failure events; destroyed resources do not receive late events. The SHM fixture verifies cancellation
visibility as well as rejected writes after owner destruction. The native feature test build passes;
end-to-end deferred-request races still require runtime qualification.

Capture submission now distinguishes recording failures before queue submission from submission or
completion-worker failures. Only the former return the complete CaptureJob to its owner. Direct
captures fail that protocol frame, finish its host ticket unsuccessfully, and recover reusable
storage without exiting the desktop. Portal captures restore returned storage before stopping the
failed stream. Neither adapter now claims completion/retirement for an uncertain post-submission
failure; those failures still use the renderer recovery path. The old error-flattening submit API was
removed. This changes host error ownership around the existing VulkanRecordedFrame/receipt contract,
not native synchronization. Both feature builds compiled during integration; actual injected GPU
recording/device/worker failures remain unqualified, including reusable readback after partial recording.

Follow-up inspection found that record_capture_readback marks staging pending before frame.finish,
so a recording failure can return an unsubmitted job whose slot is not reusable. Direct recording
rejection now finishes the host ticket unsuccessfully, drops that job and its host stream storage,
and retires the stopped host session; the next protocol request can admit fresh storage. It does not
clear a readback pending flag without a receipt or infer GPU completion. Portal rejection already
stops its stream and drops the restored slot during retirement. This supersedes the earlier claim
that a rejected direct job immediately recovers reusable storage. Native allocation/deferred
retirement accounting remains the separately documented open gate.

Native image-copy session creation now enforces eight retained session incarnations total and two
per authorized native client before creating another session resource. Surviving frame resources
retain their original requester/session identity and count once with their parent; deleting session
parents therefore cannot bypass the bound. Exhaustion posts the standard Wayland no-memory error,
consistent with resource-admission failure, rather than allocating unbounded native bookkeeping.
This native bound supplements host GPU/session/byte limits. The pure quota fixture covers parent/frame
deduplication, orphan retention, requester separation and aggregate exhaustion. Direct frame dequeue
already selects monotonically allocated frame IDs in oldest-first order; no selection change was
needed for the reviewed repeated-frame case.

Source invalidation no longer sends a frame's failed event while its destination is handed off to
host/worker code. It cancels writing and records the failure reason, retaining the pending terminal
transition until completion returns. This prevents failed from authorizing client buffer reuse
while a positioned write may still be finishing. Stopped takes precedence over a prior dimensions
mismatch. Pacing-deferred jobs explicitly return failure completion when discarded, acknowledging
that destination ownership has ended. Destroyed resources still suppress events. The focused delivery
state fixture checks deferred notification and failure precedence; concurrent native client/worker
buffer reuse remains a live qualification case. This supersedes earlier immediate source-failure
notification for handed-off frames.

Foreign-toplevel discovery now has pure mapped-window catalog and per-list announcement state,
following the installed ext-foreign-toplevel-list-v1 XML. Mapping epochs produce nonempty printable
identifiers under 32 bytes; metadata updates preserve them, while unmap/remap allocates a fresh epoch
and exhaustion fails closed. Per-list seen epochs prevent client-destroyed handles from being
reannounced during the same mapping. Stop is idempotent; unmap pruning bounds remembered identities.
The catalog bounds mapped entries and UTF-8 metadata storage. Two portable tests pass for independent
lists, remap, title changes, stop, metadata bounds and epoch exhaustion. Native event dispatch and
host catalog synchronization are still pending. The catalog must be populated from mapped-window
truth (including minimized mapped windows), not the narrower portal capture eligibility registry.

Native foreign-toplevel dispatch now creates separate handles per list, sends initial identifier,
title/app_id and done events, updates metadata, closes unmapped handles once, and acknowledges stop
without stopping existing handles. Resource destruction removes native bookkeeping without clearing
the list's mapping tombstones. Discovery binds/requests and outbound metadata use capture access
policy. Host reconciliation includes minimized mapped windows and explicitly retires native unmaps
before end-of-loop reconciliation, preserving remap identities. Collection is skipped while capture
access is unconfigured. Capture globals remain disabled; direct window source binding is still pending.
The base Wayland build and two portable catalog tests pass. These tests do not exercise native wire
events. A fresh combined portal/XWayland check was blocked by unavailable PipeWire development
pkg-config inputs in the previously used temporary header directory; no successful full-feature check
is claimed for this change. Native client interaction and X11 unmap/remap batching need qualification.

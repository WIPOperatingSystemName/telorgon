# Telorgon Development Profiler

## Status

The initial profiler implementation is present. The workspace now has compile-optional event and
server packages, a target-neutral managed-host activation contract, application-host activation,
project-local `cargo profile` aliases for Gallery and Theme Studio, bounded CPU instrumentation,
completion-delayed Vulkan timestamp queries, stable per-view correlation/filtering, an embedded
vanilla HTML/CSS/JavaScript viewer, and in-memory capture export. Unit, compile, feature, and
protocol checks exercise these paths without launching an application or server. The ordinary GUI
host and Linux shell-environment/compositor host automatically own the managed profiler session;
widget hosts can select the same contract when their native host implementations become operational.

This is not production-qualified performance tooling yet. Its overhead budgets, browser behavior,
and Vulkan timestamp behavior still need the named manual and hardware evidence in Sections 15 and
17. Current capability claims remain controlled by
[IMPLEMENTATION_STATUS.md](IMPLEMENTATION_STATUS.md).

The profiler is development tooling. Its measurements are diagnostic evidence, not production
qualification by themselves. Machine-dependent performance reports remain governed by
[PERFORMANCE.md](PERFORMANCE.md) and
[ACCEPTANCE_AND_QUALIFICATION.md](ACCEPTANCE_AND_QUALIFICATION.md).

## 1. Product contract

From an application project directory, the ordinary managed-development workflow is:

```powershell
cargo profile
```

That command must:

1. build the application with Telorgon profiler instrumentation and an optimized profiling build;
2. start the application normally;
3. start one application-owned loopback profiler service;
4. print the authenticated local URL;
5. leave browser navigation under explicit user control; and
6. stop the service synchronously when the application exits.

The browser opens to the live Performance workspace. The application remains the source of truth for
events and counters; the browser is a disposable viewer. Closing or refreshing the browser does not
change application behavior.

### 1.1 Generated project configuration

Telorgon application templates should add an application feature that forwards to the umbrella
profiler feature:

```toml
[features]
telorgon-profiler = ["telorgon/profiler"]

[profile.telorgon-profile]
inherits = "release"
debug = 1
incremental = true
strip = "none"
```

The project-local `.cargo/config.toml` should define:

```toml
[alias]
profile = [
    "run",
    "--profile", "telorgon-profile",
    "--features", "telorgon-profiler",
    "--",
    "--telorgon-profile",
]
```

Cargo profiles control optimization and debug-information settings, while Cargo features control
whether profiler code and dependencies are compiled. The alias deliberately selects both. The
reserved application argument activates the already-compiled tooling.

### 1.2 Build and activation matrix

| Build | Profiler code | Runtime service | Hot-path cost |
| --- | --- | --- | --- |
| `cargo run` | Not compiled | Impossible | Zero |
| `cargo build --release` | Not compiled | Impossible | Zero |
| Profiler feature, flag absent | Compiled | Off | One predictable disabled check only where unavoidable |
| `cargo profile` | Compiled | On | Bounded instrumentation cost |

Passing `--telorgon-profile` to a build without the feature must fail with a clear message. It must not
silently run without profiling. Ordinary builds must contain no HTTP server, WebSocket, browser
assets, timestamp-query pools, profiling labels, or profiling clock reads.

### 1.3 Frozen implementation decisions

The first implementation uses the recommended choices throughout, with the browser stack kept
entirely Rust-hosted and framework-free:

- `telorgon/profiler` fans out compile-time `profiler` features to every selected instrumented crate;
- the exact reserved `--telorgon-profile` process argument activates a compiled managed profiler;
- `telorgon-profiler-server` owns an Axum HTTP/WebSocket service on one current-thread Tokio runtime;
- the viewer is semantic HTML, one CSS file, and one vanilla JavaScript file embedded in the Rust
  binary, with no TypeScript, Node, bundler, frontend framework, CDN, or filesystem asset root;
- live metadata and retained history are followed by versioned binary event batches over one
  authenticated WebSocket; and
- the balanced overhead and memory gates in Section 15 control acceptance.

These choices are implemented. The Status section and `IMPLEMENTATION_STATUS.md` continue to
control qualification claims.

## 2. Goals

- Make one-command live renderer diagnosis available to ordinary managed applications.
- Correlate component/runtime, layout, scene compilation, renderer, GPU, and presentation work by
  stable view and frame identities.
- Reuse Telorgon's truthful counters rather than inventing estimated GPU behavior.
- Keep event production nonblocking, allocation-free after producer registration, and bounded.
- Exclude the complete facility from ordinary builds.
- Preserve a backend-neutral CPU/event protocol while allowing backend-specific timing payloads.
- Show unavailable, dropped, delayed, and uncalibrated measurements explicitly.
- Export a captured session for offline comparison without granting filesystem access to the server.
- Keep the first implementation small enough to validate overhead before adding broad coverage.

## 3. Non-goals

The first profiler is not:

- a production telemetry or crash-reporting service;
- a remote administration endpoint;
- a system-wide sampling profiler or debugger;
- a replacement for native GPU capture and validation tools;
- permission to read widget text, clipboard data, paths, URIs, secure-field contents, or user data;
- a way for the browser to mutate application state, trigger actions, or issue renderer commands;
- a universal performance score or a hardware-independent millisecond promise; or
- an automatic service in embedded or hosted-renderer integrations.

## 4. Package and ownership boundaries

The target split is:

```text
instrumented Telorgon crates
        |
        | fixed events through an optional dependency
        v
telorgon-profiler
event schema | static labels | producer rings | bounded session store
        |
        | snapshots and batches
        v
telorgon-profiler-server
managed lifecycle | loopback HTTP/WebSocket | embedded web bundle
        |
        v
browser Performance/Resources/Diagnostics/Session workspaces
```

`telorgon-profiler` contains no HTTP implementation, browser launcher, Winit type, graphics API type,
or process-global logger. Instrumented runtime, layout, render, and backend crates depend only on
this small optional package.

`telorgon-profiler-server` is enabled only through a managed-host profiler feature. It owns the
collector session, one named service thread, static browser assets, connection lifecycle, and final
join. Its session metadata identifies application, desktop, widget, or compositor target
without changing the event protocol. It does not own renderer resources or call application code.

The umbrella `telorgon/profiler` feature fans out to every selected package that has instrumentation.
Backend features remain independent: enabling the profiler does not select Vulkan or software, and
selecting a renderer does not enable the profiler.

Every instrumented crate exposes a `profiler` feature whose only profiler dependency is optional.
The umbrella feature forwards to the runtime, theme, layout, render, application, and each already
selected renderer/presenter feature. A managed host feature additionally selects
`telorgon-profiler-server`; embedded hosts continue to own sinks explicitly. This fan-out must use weak optional-dependency feature forwarding where a
backend is optional, so profiling never changes renderer selection. Feature-off dependency-tree and
symbol fixtures enforce complete exclusion.

The selected implementation packages are:

- `rtrb` for fixed-capacity, single-producer/single-consumer rings between instrumented threads and
  the collector;
- `axum`, with its WebSocket feature, for the loopback routes and upgrade;
- `tokio` for network I/O, timers, and connection tasks on a current-thread runtime owned by the
  service thread;
- `serde` and `serde_json` only for bounded metadata and explicit HTTP/protocol errors; and
- a Telorgon-owned binary encoder/decoder for event batches and captures rather than a
  dependency-specific serialization format.

Implementation must commit resolved versions to `Cargo.lock` and record their licenses through the
repository dependency-review process before merge.

### 4.1 Managed and embedded ownership

An operational managed application, desktop, widget, or compositor runner may start the service
because `cargo profile` is an explicit user request. Embedded and hosted modes must instead receive
an optional host-owned event sink and must
not create a thread, socket, browser process, or independent clock without an explicit host grant.
This preserves Telorgon's embedded-host interference contract.

## 5. Lifecycle

### 5.1 Startup

Before the Winit event loop starts, the managed runner:

1. validates the reserved profiler arguments;
2. creates one monotonic session clock origin;
3. creates the bounded session store and producer registry;
4. binds a stable executable-derived port on `127.0.0.1` so an already-open viewer can find the
   same application after restart;
5. creates a cryptographically random session token;
6. starts one named `telorgon-profiler` service thread;
7. prints the complete URL without launching a browser.

Socket binding or collector initialization failure is fatal to `cargo profile`, because the user
explicitly requested a profiling run. The user opens the printed URL when a viewer is not already
waiting; subsequent runs reconnect that loaded viewer and do not create additional browser windows.

The managed runner recognizes the exact `--telorgon-profile` OS argument; it does not treat prefixes,
environment variables, or the custom Cargo profile name as activation. It does not remove or
reinterpret application arguments. An application that uses a strict argument parser before
calling the managed runner must allow this reserved Telorgon argument. With the feature compiled but
the argument absent, producer registration and the service remain inactive.

The named service thread constructs a Tokio current-thread runtime with I/O and time enabled and
drives it until shutdown. It must not create a hidden Tokio worker pool or use `spawn_blocking`.
Connection tasks, the Axum router, collector draining, and viewer back-pressure all remain on this
single service thread; instrumented application and renderer threads only write their producer
rings.

### 5.2 Active session

Collection starts before application mounting so startup can be inspected. The session retains a
rolling window even when no browser is connected. Connecting a viewer transfers the metadata and
current retained window, then streams later batches.

Disconnect, browser refresh, slow network reads, or a paused browser must never block an application
or renderer thread. A slow viewer loses viewer batches; the application session continues and
reports the gap.

When the application exits, the already-loaded viewer clears its session data, displays **App not
connected**, and retries the same stable loopback origin with bounded backoff. A later
`cargo profile` run for that executable renews the prior same-origin profiler cookie and the viewer
accepts the new metadata, label dictionary, sequence space, and retained frame window as a new
session. `TELORGON_PROFILER_PORT` can provide an explicit nonzero stable port when the derived port
collides with another local service.

### 5.3 Shutdown

The managed runner closes producers, publishes a terminal session event, closes listener and client
sockets, wakes the service thread, and joins it before returning. Dropping or forgetting the server
handle is forbidden. Application shutdown does not wait for a browser acknowledgement.

## 6. Event model

Every event has a fixed header:

```text
protocol version
monotonic sequence
session-relative timestamp in nanoseconds
lane identity
optional view identity
optional frame identity
event kind
payload length
```

The initial event vocabulary is:

- `Span`: start plus duration for one completed CPU operation;
- `Instant`: a recovery, loss, fallback, wake, retry, overflow, or lifecycle event;
- `Counter`: one named numeric observation;
- `FrameBegin` and `FrameEnd`: CPU frame correlation boundaries;
- `PresentationAttempt`: acquire and presentation result with an optional source frame;
- `GpuSpanResolved`: completed backend timing relative to one GPU timestamp domain;
- `Diagnostic`: bounded structured severity, kind, and static context; and
- `Gap`: producer, session-store, or viewer loss with an exact dropped count.

Labels are static author-written strings registered once and referenced by compact IDs. Dynamic
labels, formatted strings, application data, and source paths are not accepted by the hot-path API.
Hash collisions must be detected during registration rather than silently merging labels.

### 6.1 Identities and correlation

`ProfileViewId` identifies one independently scheduled rendered view or surface. A single-window
GUI application uses the primary identity; shell-environment and embedded hosts allocate one
stable identity per independently scheduled output. Every frame, presentation, span, counter, and
diagnostic inherits the current view scope. Worker transfers carry both view and frame identity so
unframed presentation attempts remain attributable without inferring ownership from thread names.
Hosts register a bounded, static, content-free role for each view (for example, `GUI window` or
`Desktop environment output`); dynamic titles and shell-widget content are not profiler metadata.
The profiler supplies session-unique view IDs after the reserved primary identity, so target hosts
do not derive correlation IDs from native handles or pointer values.

`ProfileFrameId` is issued by the managed host for every requested frame on any entrypoint. It is distinct
from the retained-scene epoch because idle, resize-preview, recovery, and maintenance presentation
attempts do not have a one-to-one relationship with scene changes.

The frame ID is passed through `PreparedFrame`, scene-delta transfer, `VulkanWork`, command
recording, and presentation. A presentation-only attempt may have no source frame but still receives
its own `PresentationAttemptId`.

Lanes are stable for the session and include at least:

- managed UI/event thread;
- Vulkan presentation worker or software presentation lane;
- managed component task worker when present; and
- one GPU lane per backend queue that supplies timestamps.

## 7. Producer requirements

Each participating thread registers once and owns a single-producer ring. The collector is the only
consumer. After registration, recording a CPU span performs only:

- a profiler-enabled check;
- monotonic clock reads at scope entry and exit;
- construction of one fixed-size record; and
- a bounded ring write.

It performs no heap allocation, label formatting, mutex acquisition, system call, channel wait,
network I/O, file I/O, or subscriber callback. Ring saturation increments an atomic lost-event
counter and returns immediately.

Each registered producer owns the `rtrb::Producer` half of a fixed-capacity ring and the collector
owns its `Consumer` half. Ring storage is allocated at registration and never resized. A full ring
causes the new record to be dropped immediately and increments that producer's loss count; no
producer retries, spins, blocks, or wakes a network task directly.

Instrumentation macros compile to no expression and evaluate no arguments when the profiler feature
is absent. Expensive diagnostic values must be supplied lazily so a disabled recording path does not
compute them.

## 8. Initial instrumentation map

| Layer | CPU spans | Existing facts sampled at frame boundaries |
| --- | --- | --- |
| Managed host | event dispatch, command flush, redraw decision | redraw reason, refresh interval, pending resize, idle/submitted |
| Component runtime | external updates, action rounds, task/timer turn | component/state/read/action/task diagnostics |
| Application runtime | theme update, layout, scene compilation, delta creation | `FrameDiagnostics`, scene epoch, delta queue high-water |
| Scene transport | enqueue, dequeue, coalescing | delta bytes, coalesced count, queue depth |
| Software renderer | scene apply, raster, surface copy, present | `RenderStats`, damage, framebuffer reuse |
| Vulkan worker | scene apply, acquire, command record, finish, submit/present, recovery | `RenderStats`, acquire duration/breaches, resize generation/metrics revision/phase, acquire/present disposition, retries |
| Vulkan device | allocation, upload, pipeline/descriptor creation | memory, scene, frame-slot, cache, validation diagnostics |

The first slice instruments frame-scale operations. It must not emit one event per node, primitive,
glyph, descriptor, or draw. Existing aggregate counters explain that work with much lower observer
effect.

The managed host publishes cumulative turn-boundary counters for total and clean input turns, issued
and duplicate-suppressed redraw requests, redraw callbacks, idle versus submitted presentations, and
each stable redraw reason. Input flushing separately publishes received/dispatched, non-pointer
input, and pointer/scroll/resize coalescing totals. A pending owner turn that contains only pointer
movement is processed normally but produces no profiler records by default, including no empty
`input.flush`/`commands.flush` spans or repeated host-counter snapshots. The persistent Input
recording sidebar can enable collection of future pointer-movement details when an investigation
needs them. Frames
triggered only by pointer movement and their Responsiveness input signals are likewise excluded from
Performance views while that preference is off. Frames that also carry animation, timer, command,
resize, or other input work remain visible.

The sidebar is target-aware session metadata rather than a hard-coded universal device list. GUI
sessions advertise their supported Winit pointer, button, scroll, and keyboard streams. Linux
shell-environment sessions advertise their supported libinput pointer-motion, button, axis,
keyboard, touch-motion, touch-contact, and device-change streams. Every stream is independently
opt-in and resets off for a new profiler session. The Inputs performance view charts each retained
sample on a source lane, summarizes event counts and queue-age percentiles, and keeps a bounded event
log. Libinput samples carry the monotonic delay from the native event timestamp to the compositor's
post-dispatch observation, exposing the backlog behind libinput's client-lag warning. Input payloads,
coordinates, key values or text, touch positions, and device identities are never recorded.

### 8.1 Frame-timeline probe catalog

The raw Frame Work timeline is deliberately more granular than the initial map while remaining
bounded. One changed application frame may emit at most 48 completed CPU spans and 32 counter
observations across its source frame and presentation attempt. Conditional spans exist only when
their operation runs. Idle frames do not manufacture empty work.

| Timeline lane | Stable span labels | Primary source boundaries |
| --- | --- | --- |
| UI turn | `input.drain`, `input.dispatch`, `tasks.process`, `timers.process`, `signals.ready`, `frame.gate`, `commands.flush`, `redraw.decide` | Managed Winit host and `AppRuntime` turn boundaries |
| Composition | `signals.drain`, `component.evaluate`, `element.reconcile`, `children.reconcile`, `dependencies.commit` | `ViewRuntime::process_external_updates` and `CompositionDriver` evaluation/reconciliation functions |
| Theme + layout | `theme.resolve`, `theme.motion`, `layout.dirty_scan`, `layout.measure`, `layout.arrange`, `layout.spatial_clip` | `ThemeRuntime::update_styles` and `LayoutEngine::update` phases |
| Scene compile | `scene.dirty_scan`, `scene.primitives.patch`, `scene.text.prepare`, `scene.atlas.collect`, `scene.draw_order`, `scene.delta.take`, `scene.delta.enqueue` | `SceneCompiler::compile`, `RenderScene::take_delta`, and the application delta queue |
| Scene transport | `transport.coalesce`, `transport.dequeue`, `worker.wake`; `presentation.worker.queue_age_ns` counter | Managed renderer mailbox and `VulkanWorkerState::process` |
| Vulkan scene | `delta.validate`, `delta.apply`, `scene.rebatch`, `retained.retire` | `VulkanDevice::apply_scene_delta`, retained-scene batch rebuild, and completion maintenance |
| Vulkan frame | `frame.maintain`, `frame.slot.begin`, `swapchain.acquire`, `presenter.acquire.raw_dispatch`, `uploads.plan`, `uploads.stage`, `descriptors.bind`, `barriers.record`, `draws.record`, `command.finish`, `queue.submit`, `queue.present`, `presenter.retirement_wait` | `VulkanWorkerState::render_once`, the exact `vkAcquireNextImageKHR` dispatch, `VulkanDevice::render`, frame-slot finish, presenter submission, and bounded retired-swapchain fallback |
| GPU relative | `gpu.total`, `gpu.upload_copy`, `gpu.render_pass` | Frame-slot-owned Vulkan timestamp-query boundaries after capability qualification |

The browser groups nested probes under these stable lanes. A span carries its parent label ID, but
the producer never constructs a dynamic hierarchy string. The timeline may collapse very short
spans visually; their duration and label remain present in selection details and saved captures.

### 8.2 Probe-versus-counter rule

A probe is warranted when it identifies a schedulable or independently actionable interval. Work
inside a tight loop remains a counter:

- component evaluations and reconciliation rounds are spans; individual component instances are
  counts and maximum-depth facts;
- layout measure/arrange and spatial/clip phases are spans; individual nodes, cache hits, and
  intrinsic passes are counters;
- scene primitive, text/atlas, draw-order, and delta phases are spans; patched boxes, glyphs,
  images, dirty ranges, bytes, and damage rectangles are counters;
- upload planning/staging, descriptor binding, barrier recording, and draw recording are spans;
  individual allocations, copies, writes, barriers, batches, and draws are counters; and
- acquire, submit, and present are spans or presentation attempts; retry, suboptimal, recovery, and
  loss outcomes are structured instants.

Per-node, per-glyph, per-resource, per-descriptor, per-barrier, per-batch, and per-draw spans are not
part of the live profiler. Native GPU capture tools remain the correct path for command-level
inspection. This rule keeps timeline detail actionable and preserves a fixed observer-effect bound.

## 9. GPU timing

GPU timing is a separately accepted capability, not part of the initial CPU-profiler milestone.

For Vulkan, a supported device path must:

1. check the selected queue family's nonzero `timestampValidBits`;
2. retain the device `timestampPeriod`;
3. allocate a bounded timestamp query range per reusable frame slot;
4. reset and write only queries owned by that slot;
5. record selected phase boundaries with `vkCmdWriteTimestamp2`;
6. retrieve results only after the existing completion proof says the slot has retired;
7. avoid `VK_QUERY_RESULT_WAIT_BIT` and every device/queue idle wait;
8. handle timestamp wrap using the reported valid bit width; and
9. publish timing as unavailable when any prerequisite is absent.

The initial GPU regions are upload/copy, main render pass, and total recorded GPU work. Additional
markers require measured justification because timestamp operations themselves have cost.

Ordinary timestamp queries provide GPU-relative durations. The browser must not place GPU events on
an absolute CPU time axis unless a later backend implements and validates calibrated CPU/GPU time
domains. Until then, the UI labels the lane `GPU relative` and displays it separately from absolute
CPU scheduling.

## 10. Bounded storage and transport

The default session retains the newer of:

- the latest 600 completed application frames; or
- the configured hard byte ceiling, initially 32 MiB.

When the ceiling is reached, complete oldest frames are evicted first. Startup events and immutable
session metadata have a separate small bound. No browser control may increase an application-owned
bound beyond its configured maximum.

The same-origin server exposes only:

- static versioned browser assets;
- one metadata document;
- one authenticated WebSocket event stream; and
- one authenticated in-memory capture download.

The wire protocol uses versioned binary label and event batches. Each interned label record carries
its stable name plus a bounded category, unit, aggregation mode, and display flags. Presentation and
responsiveness probes have their own semantic category while the browser also maps every event into
one of three exclusive performance classes: CPU work, GPU work, or presentation/responsiveness.
Counter aggregation is explicit: gauges use the latest observation, per-turn/frame samples sum within their
correlated frame, and cumulative counters are converted to deltas from the preceding observation.
The browser therefore never infers units, resource membership, or counter behavior from a label
regular expression. Protocol major version 3 adds an explicit view identity to every event record;
the descriptor-bearing label layout introduced by version 2 remains unchanged. A stale viewer
therefore cannot silently interpret view-less records as multi-view telemetry. Metadata and explicit error
responses may be JSON. The browser must reject an unsupported major version rather than trying to
interpret unknown events.

### 10.1 Rust server and embedded viewer

The initial service uses Axum for both HTTP and WebSocket handling. It serves only fixed routes for
the session HTML, `profiler.css`, `profiler.js`, bounded metadata, capture download, and the live
upgrade. HTML, CSS, and JavaScript are embedded with Rust `include_bytes!`/`include_str!` inputs and
returned with fixed content types. There is no directory-serving fallback, path-derived file access,
template execution, or runtime asset compilation.

The HTML loads the separate stylesheet and deferred script from the authenticated same-origin
service. A restrictive content-security policy allows resources and WebSocket connections only from
that origin; no inline executable code or third-party resource is required.

The vanilla JavaScript client opens one WebSocket, sets `binaryType = "arraybuffer"`, and decodes
the Telorgon batch header and fixed event records with `DataView`; 64-bit sequence, timestamp, and
identity values remain `BigInt` until formatted or converted through an explicitly checked range.
On connection it receives bounded session metadata, the label dictionary, and the retained snapshot
before incremental batches. The server may coalesce records into a batch, but it never exposes a
producer ring or waits for browser acknowledgement. The client coalesces rendering work to
`requestAnimationFrame` and caps live repaints at ten per second so WebSocket message frequency does
not cause an equivalent number of DOM layout passes. It invalidates only the visible workspace;
completed pointer-move-only frames do not repaint the Performance graph while pointer movement is
excluded, and the Inputs view caps its source-lane chart and event table independently.

DOM updates use semantic tables, buttons, navigation, plots, and inspector regions already defined
by the visual language in Section 12. The browser owns selection, zoom, pause, and display state.
Only explicit Input recording checkboxes change source collection: the authenticated WebSocket
enables or disables future records for one advertised native source without replaying an excluded
interval. While
paused through the top-bar control, the viewer discards incoming event and viewer-gap batches,
freezes its current dataset, and establishes
new cumulative-counter baselines when resumed. The bounded application-side session continues so a
browser interaction cannot perturb the target being measured.

## 11. Security and privacy

- Bind loopback only by default and never silently widen to `0.0.0.0`.
- Use a stable executable-derived port so a loaded viewer can reconnect after process restart; fail
  clearly on collision rather than silently changing the endpoint.
- Put an unguessable token in the initial URL and require its port-qualified HTTP-only cookie for
  every request and upgrade.
- Permit token renewal only for an existing correctly shaped profiler cookie on an exact
  same-origin POST; rotate it to the new process token before reconnecting.
- Check WebSocket origin against the profiler origin.
- Set a restrictive content-security policy and serve no user-supplied HTML.
- Expose no filesystem routes, arbitrary proxy, command execution, application RPC, or object
  inspection.
- Never record widget text, secure text, clipboard content, drag payloads, paths, URIs, notification
  content, native handles, pointers, file descriptors, or dynamic validation payloads by default.
- Sanitize backend diagnostics before they enter the protocol; unbounded native messages remain in
  the existing local diagnostic owner and are represented by bounded kind/count information.
- Treat remote binding as a separate future security design, not a hidden flag.

## 12. Browser presentation

The product name is **Telorgon Profiler**. It is a dense development tool, not an application
dashboard. The initial page uses one persistent shell:

- top bar: application, live/paused state, backend, connection state, an accessible pause/play icon
  toggle, Clear, and Save trace;
- left navigation: Performance, Resources, Diagnostics, and Session, followed by a persistent
  target-aware Input recording checklist;
- main workspace: the selected navigation surface; and
- resizable right inspector: selected frame or event details.

### 12.1 Performance workspace

Performance is the default and primary workspace. It retains one correlated selection and
event-ordered frame range across three views:

1. **Overview** plots one host-turn duration point per completed frame in recorded order, then
   reports the CPU distribution, equal-per-frame correlated category statistics, and the most
   important budget misses. Host-turn duration is not mislabeled as GPU completion or physical
   display time.
2. **Frame Work** plots one CPU, GPU, and presentation value per recorded frame, then provides the
   hot-spot table, counter summaries, and expandable raw CPU/GPU flame chart.
3. **Responsiveness** plots only correlated active-frame, input-to-present, and successful-present
   latency samples. It aggregates presentation attempts by terminal outcome and lists actionable
   over-budget incidents; it does not rasterize raw event occupancy or draw full-height markers.
   Pointer-movement input is excluded from these latency signals by default and can be included with
   its Input recording checkbox.
4. **Inputs** charts opt-in native events in stable source lanes, summarizes counts and libinput
   queue-age percentiles, and provides a bounded newest-first event log linked to event inspection.

The pointer-movement checkbox controls source collection for future pointer-only owner turns and the
visibility of completed frames whose only scheduling trigger was a pointer move. Every input source
is off by default and resets when the profiler session restarts. Disabling pointer movement does not
hide a frame when pointer movement overlaps any other meaningful trigger.

The header, tables, and summaries analyze the frames currently visible in the graph. The shared
selector provides an all-views or one-view filter, while the graph-window selector shows the latest
30, 60, 120, 300, or all retained frames. Right-drag zoom narrows both the visible graph and its
default analysis scope. A left-drag custom range temporarily overrides that scope; changing the graph
limit, zooming, navigating zoom history, resetting the view, selecting an individual point, or using
Clear range returns analysis to the visible graph. Gap calculations are partitioned by view before
aggregation. Plot positions use recorded frame order, while hover,
persistent point selection, and frame details expose session-relative timestamps. A long idle
interval therefore remains visible in the timestamps and latency calculations but does not consume
horizontal plot space or compress existing points when no frame was recorded.

The raw flame chart is collapsed by default so event volume cannot displace the range summaries.
Hot-spot percentiles are computed from one value per frame: repeated spans with the same timing
domain and stable label are summed within a frame before the range percentile is calculated. This
prevents a frame that emits many animation or state-update spans from weighting the distribution
more heavily merely because it contains more events. Hot spots rank by self time so nested spans do
not double-count work. Low-cost stages below the visible threshold collapse into one `Other` row;
their original events remain selectable in the raw flame chart and present in saved captures.

Hovering a plotted point gives it a visible ring and previews its label, value, frame, and timestamp.
Left-clicking a point gives it a persistent marker, one full-height white guide, and every series
value displayed at that frame or sample beneath the graph; clicking elsewhere selects the nearest
frame without drawing a short bottom tick. Left-dragging draws a full-height translucent marquee and
leaves the custom analysis range visibly shaded between two full-height white boundary guides. Point
and manual-range selection are mutually exclusive: selecting either clears the other. A multi-frame
manual selection changes the right inspector to a Range summary containing its frame and timestamp
boundaries, host-turn distribution, budget misses, grouped CPU/GPU/presentation work, and dominant
stages. Right-dragging draws a separate marquee, clears any custom analysis range, and zooms the graph
and default analysis to that recorded-frame interval. Back view restores nested graph views one step
at a time and Reset view returns directly to the graph-window limit. Selecting a frame, point, event, or hot spot
never changes the live/paused state; hot spots open their worst frame and highlight matching spans
without hiding surrounding causal work. Arrow keys move frame selection by one frame while new data
continues to arrive. Only the top-bar Pause/Resume control changes that state. Every frame selection
refreshes the right-hand summary in all three Performance views; selecting a frame also returns from
selected-event details without a separate summary button. Escape resumes the live tail, returns
selection to the newest frame, and clears the hot-spot filter. Hover is supplementary; all values
remain reachable by selection and keyboard.

The session indicator reads **Live** beside a green dot while collection is active and **Paused**
beside an orange dot while the viewer is explicitly paused. The pause/play icon toggle has no visible
border or background; its tooltip and accessible label name the action it will perform.

Selecting any timeline span replaces the frame summary in the right inspector with stable label,
lane, start offset, duration, self time, timing domain, frame/presentation correlation, source
boundary, and bounded phase facts. Selecting any graph frame returns to its frame summary. The
inspector's left separator can be dragged between 220 and 760 pixels, or adjusted with arrow, Home,
and End keys. Its effective maximum responds to the desktop width so at least 360 pixels remain for
the main workspace, whose flex/grid toolbars reflow around it. Pointer movement is coalesced to one
sidebar width update per animation frame, layout limits are measured once per drag, and the graph is
redrawn only after release rather than reconstructed at every intermediate width. Sequential events
at the same nesting depth share one flame row rather than each claiming a vertical row. Spans do not expand inline because
changing lane geometry would make aligned timing comparisons unstable. On narrow layouts the
inspector follows the timeline at full width and the resize separator is hidden.

Every table column heading is an ascending/descending sort control. A table retains one active sort
column at a time, marks only that heading with `aria-sort` and an up/down indicator, and clears the
previous heading when another column is chosen. Sort state is local to each table and is reapplied
after live rows or inspector tables are rebuilt. Duration, byte, percentage, frame-ID, ratio, count,
and text values use type-aware comparison; unavailable values remain at the end in either direction.
Tables that classify events or measurements expose CPU work, GPU work, or
presentation/responsiveness as a dedicated sortable Category column instead of relying on the swatch
inside another field.

Blue always denotes CPU work, green GPU-relative work, and orange presentation/responsiveness work.
The same labeled swatches appear in summary tables, raw event bars, and inspector details, so color is
never the only category signal. Red remains reserved for errors or explicit threshold failures.

`presentation.presented` means the platform present operation accepted the image; it is not a claim
about physical scan-out. The viewer reports that distinction and leaves actual display timing
unavailable unless a future qualified display-timing source supplies it.

### 12.2 Resources workspace

Resources shows retained values over time rather than a one-time allocation list:

- device-local reserved and budget bytes;
- staging high-water and upload bytes;
- live/capacity counts for retained scene records;
- images, atlas pages, pipelines, descriptors, frame slots, and retired generations; and
- allocation, growth, eviction, replacement, and recovery events.

Backend-unavailable values show `N/A`, never zero. The page does not infer GPU residency from CPU
scene ownership.

### 12.3 Diagnostics workspace

Diagnostics presents bounded structured events grouped by kind and first/last frame. It includes
validation error/warning counts, device/surface loss and recovery, selected fallbacks, dropped
profiler events, protocol gaps, and service errors. It must not become a raw unbounded log viewer.

The Vulkan presenter separately measures the raw `vkAcquireNextImageKHR` call made with timeout
zero. A dispatch lasting at least 100 ms records
`presentation.vulkan_wsi.zero_timeout_acquire_stall` and the
`presentation.acquire.raw_dispatch_duration_ns` counter. The selected-frame inspector and
Diagnostics workspace then show a likely GPU-driver/Vulkan-WSI warning, raw Vulkan vendor/device/
driver identifiers, and driver update/rollback guidance. This classification is intentionally not
conclusive: a persistent incident still requires capture export and Vulkan validation to rule out
application synchronization misuse. The originating AMD/Windows incident disappeared after a GPU
driver update, which is the empirical basis for the guidance.

### 12.4 Session workspace

Session shows the application/build identifier, Git revision when supplied by the application,
compiler and profile, operating system, CPU architecture, renderer/backend, adapter/driver, output
properties, present mode, profiler protocol, buffer limits, active capabilities, and unavailable
metrics. This metadata accompanies every saved capture.

### 12.5 Visual language

The primary task is finding a slow frame and identifying which pipeline stage caused it. The
interface therefore gives visual priority to the frame history, selected-frame timeline, and
selected-frame measurements. Session controls and metadata remain quiet; Resources, Diagnostics,
and Session reuse the same page-header, row, table, divider, and numeric-text treatments.

The profiler uses one restrained interface system:

- a `4/8/12/16/24 px` spacing scale, with denser spacing reserved for timeline marks and tables;
- square-to-soft `2/4 px` radii for data marks and controls, with no pill treatment for ordinary
  status, navigation, metadata, or actions;
- flat opaque major surfaces, subtle separators between major regions, and no decorative shadows,
  gradients, glass effects, nested cards, or borders around every section;
- one 32 px text-button treatment, with a single filled primary action per toolbar and invariant
  geometry across hover, focus, current, paused, and selected states;
- four practical type levels: compact plot labels, ordinary interface text, section headings, and
  the selected primary measurement; and
- plain text navigation and labels unless an icon communicates something text cannot.

Alignment, proximity, type weight, and whitespace establish hierarchy before background color or
container chrome. Components with the same function must share markup and styling. A different
treatment requires a functional distinction, such as primary action, current workspace, selected
frame, warning, or error.

- neutral structure and text for the application shell;
- blue for CPU work;
- green for measured GPU-relative work;
- orange for presentation, waits, back-pressure, retries, and responsiveness delay; and
- red only for over-budget frames and errors.

Color is always paired with lane labels, icons, shapes, or text. Frame bars preserve a minimum
interactive hit target even when visually narrow. Durations use tabular numerals and explicit units.

## 13. Capture export

Save trace downloads one self-contained, versioned capture assembled from the in-memory session. The
capture contains metadata, label dictionary, events, counters, gaps, and availability facts. It does
not expose a server-side path selector.

The canonical Telorgon capture format is the lossless source. A later optional exporter may produce
Chrome Trace Event JSON or another interchange format, but the live protocol and internal model must
not be constrained to the least expressive interchange format.

## 14. Failure behavior

| Failure | Required behavior |
| --- | --- |
| Feature absent, flag present | Exit before application startup with remediation command |
| Loopback bind or collector initialization fails | Fail the profiling run clearly |
| No viewer is open | Keep bounded capture and print the URL for the user |
| Browser disconnects | Continue bounded capture |
| Producer ring fills | Drop new producer events, increment exact loss count |
| Session store fills | Evict complete oldest frames, publish exact gap |
| Viewer falls behind | Drop viewer batches only, publish viewer gap |
| GPU timestamp unsupported | Show `N/A`; continue CPU/counter profiling |
| GPU result not ready | Defer; never wait |
| Device/surface loss | Publish structured event; follow existing recovery path |
| Server thread fails after startup | Disable streaming, report once, never abort renderer work |

## 15. Performance budgets

The balanced acceptance gates are:

- feature absent: exact zero code generation in instrumentation fixtures, no profiler dependency in
  the resolved ordinary-build graph, and no labels, browser assets, service thread, or socket in an
  ordinary binary;
- feature compiled but inactive: no producer rings or service resources and no more than
  `max(0.5% of baseline, 10 us)` additional p95 CPU frame time;
- CPU profiling active: no more than `max(3% of baseline, 0.20 ms)` additional p95 CPU frame time
  on a representative changed-frame workload;
- one completed CPU span: at most 500 ns p95 recording cost after producer registration;
- Vulkan GPU timing active: no more than `max(1% of baseline, 0.10 ms)` additional p95 frame time
  on each named qualified device profile;
- memory: at most 32 MiB for retained session events plus 1 MiB per registered producer by default;
- event volume: at most 48 completed CPU spans and 32 counter observations per correlated
  frame/presentation attempt by default; and
- every producer write remains allocation-free, lock-free, wait-free, system-call-free, and bounded,
  with immediate loss accounting on saturation.

For these formulas, the allowed absolute overhead is the greater of the relative allowance and the
absolute noise floor. Reports must publish the baseline and instrumented p50/p95/p99, sample count,
build revision, compiler, operating system, CPU, renderer, adapter/driver, profiler buffer sizes,
and every unavailable measurement. The initial acceptance machine and Vulkan devices must be named
in the qualification report rather than implied by these cross-machine targets.

Profiler overhead must be visible in Session metadata and qualification reports. The tool must never
subtract its own estimated cost from measurements.

## 16. Implementation slices and current state

### P0: Protocol and build exclusion — implemented

- Add the two package boundaries and umbrella feature fan-out.
- Add feature-off compile fixtures and macro argument non-evaluation tests.
- Define session, identity, event, gap, and availability types.
- Add generated-project Cargo profile and alias fixtures without starting applications in CI.

### P1: Managed CPU profiler and Performance UI — implemented, qualification pending

- Start and join the Axum service and current-thread Tokio runtime through managed runner ownership.
- Embed the semantic HTML, CSS, and vanilla JavaScript assets in the server crate.
- Instrument the bounded stable-label CPU catalog from host turn through queue presentation.
- Publish resize phases and bounded Vulkan presentation outcomes for successful, suboptimal,
  frame-slot retry, acquire-not-ready, reconfigure, preview, preview-suppression, zero-timeout
  acquire breach, and surface-loss attempts. Swapchain generation, extent, image count, selected
  present mode, and maintenance capability are bounded counters.
- Stream existing `FrameDiagnostics` and `RenderStats`.
- Implement the rolling Performance workspace, three correlated views, selection, pause, clear, and
  capture download.
- Prove bounded slow-viewer and disconnect behavior.

### P2: Vulkan GPU-relative timing — implemented, hardware qualification pending

- Extend Vulkan capabilities with timestamp support facts.
- Add frame-slot-owned query pools and completion-delayed retrieval.
- Add upload, main-pass, and total GPU regions.
- Prove no wait/idle path and correct unavailable/wrap handling.

### P3: Resources and diagnostics — implemented, coverage expansion pending

- Publish actual memory, cache, retirement, recovery, and validation counts.
- Implement Resources, Diagnostics, and Session workspaces.
- Add capture compatibility and regression fixtures.

### P4: Multi-view and explicit embedded sinks — partial

- Stable view identity is encoded in protocol v3, transferred to the Vulkan worker, and filterable
  in the viewer; shared-device GPU-lane qualification remains pending.
- Add host-supplied sinks without service ownership.
- Keep all automatic network/thread behavior confined to explicitly activated managed hosts.

`telorgon-embed` now exposes an explicit host-owned session/collector path with no server, browser,
polling thread, or implicit runtime. Native desktop, widget, and compositor activation plus
shared-device GPU-lane qualification remain open.

## 17. Acceptance tests derived from the design

- Ordinary dev/release builds compile without profiler packages, labels, assets, and symbols.
- Profiler macros do not evaluate arguments when excluded or disabled.
- Exact reserved-argument fixtures distinguish active, compiled-inactive, and feature-absent runs.
- Managed startup binds only loopback, uses a stable executable-derived port and random process
  token, and joins its one service thread on every exit path.
- An existing same-origin viewer clears stale session state on disconnect, renews its cookie after a
  process restart, and treats reused sequence/label identities as a new session.
- The service uses one current-thread runtime and creates no Tokio worker pool or blocking-task pool.
- Invalid token, origin, protocol version, path, and WebSocket upgrade requests are rejected.
- Embedded-asset route fixtures serve only the fixed HTML/CSS/JavaScript resources and reject path
  traversal and filesystem fallback.
- Producer, session, and viewer saturation preserve bounds and report exact independent gaps.
- Browser disconnect and slow-reader fixtures cannot delay an instrumented producer.
- One app frame keeps the same `ProfileFrameId` through runtime, delta, Vulkan work, and presentation.
- Presentation-only maintenance attempts remain distinguishable from app frames.
- Timeline spans are properly nested within one lane, carry only registered static labels, and never
  exceed the default per-frame/attempt event budget.
- Per-node, per-glyph, per-resource, per-descriptor, per-barrier, per-batch, and per-draw work is
  represented by existing counters rather than unbounded live spans.
- Counter snapshots match the existing source structs exactly.
- GPU queries are unavailable on a zero-`timestampValidBits` queue and never replaced by CPU timing.
- Query reuse occurs only after frame-slot completion and results are retrieved without wait flags.
- Timestamp wrap and `timestampPeriod` conversion have deterministic tests.
- Captures reject incompatible major versions and retain gaps/unavailable facts.
- Redaction fixtures prove no dynamic application or secure content enters labels, events, metadata,
  diagnostics, or exports.
- Vanilla JavaScript protocol fixtures decode retained and incremental binary batches, preserve
  64-bit identities without numeric truncation, and surface unsupported versions and gaps.
- Performance UI keyboard selection, paused/live state, unavailable metrics, gaps, and narrow layouts have
  browser fixtures.
- Qualification fixtures enforce the CPU, GPU, memory, and feature-exclusion gates in Section 15 on
  named profiles without starting a server in ordinary CI.

## 18. Reference and specification audit

### Concern

Compile-excluded development instrumentation, a live local viewer, bounded nonblocking event
delivery, stable multi-view correlation, statistically equal per-frame aggregation, and Vulkan
timing that respects asynchronous query and frame-resource lifetimes.

### Sources inspected

- Flutter tool and DDS integration:
  `../other-rendering-libs/flutter/packages/flutter_tools/lib/src/devtools_launcher.dart`,
  `../other-rendering-libs/flutter/packages/flutter_tools/lib/src/base/dds.dart`, and
  `../other-rendering-libs/flutter/packages/flutter_tools/lib/src/resident_runner.dart`.
- Flutter revision `51fd9afadf309ba5337320bd3653f5345c156cb9` multi-view embedder
  contracts in `engine/src/flutter/shell/platform/embedder/embedder.h`
  (`FlutterViewId`, `FlutterPresentViewInfo`, `present_view_callback`) and
  `embedder_external_view_embedder.cc` (`SubmitFlutterView`, per-view render-target caches, and
  view-bearing presentation callback invocation).
- Slint revision `69ecb713f5c62d1b6fe986ff822a57f22152b4d9` Winit routing in
  `internal/backends/winit/event_loop.rs`: native `WindowId` lookup before input, resize, and redraw
  dispatch plus per-active-window animation redraw requests.
- Egui/Puffin opt-in local profiling example:
  `../other-rendering-libs/egui/examples/puffin_profiler/src/main.rs`.
- Wgpu profiling and Vulkan query implementation:
  `../other-rendering-libs/wgpu/wgpu-hal/src/vulkan/command.rs`,
  `../other-rendering-libs/wgpu/wgpu-hal/src/vulkan/device.rs`, and
  `../other-rendering-libs/wgpu/wgpu-hal/src/vulkan/mod.rs`.
- Khronos Vulkan query specification and timestamp-query sample:
  <https://docs.vulkan.org/spec/latest/chapters/queries.html> and
  <https://docs.vulkan.org/samples/latest/samples/api/timestamp_queries/README.html>.
- Khronos `vkQueuePresentKHR` reference, which specifies that the call queues presentation work,
  may block for finite time, and does not include actual presentation-engine processing in the
  queue operation's scope:
  <https://docs.vulkan.org/refpages/latest/refpages/source/vkQueuePresentKHR.html>.
- Cargo custom profiles, run options, and aliases:
  <https://doc.rust-lang.org/cargo/reference/profiles.html>,
  <https://doc.rust-lang.org/cargo/commands/cargo-run.html>, and
  <https://doc.rust-lang.org/cargo/reference/config.html#alias>.
- Axum WebSocket upgrade and stream support:
  <https://docs.rs/axum/latest/axum/extract/ws/>.
- Tokio current-thread runtime construction and I/O/time drivers:
  <https://docs.rs/tokio/latest/tokio/runtime/>.
- `rtrb` bounded real-time-safe SPSC ring behavior:
  <https://docs.rs/rtrb/latest/rtrb/>.

### Invariants extracted

- The viewer, target connection information, and application lifecycle are separate concerns.
- Profiling is explicitly enabled and local service lifetime has an owner.
- Hot-path scopes are small static-label events, not formatted logging.
- The profiler connection cannot be allowed to block target execution.
- GPU query pools and result storage are real resources with reuse and completion lifetimes.
- Timestamp support and conversion come from reported device/queue properties.
- Query results are asynchronous; absence or lateness is not zero duration.
- A convenient command must select compiler profile, feature inclusion, and runtime activation.
- View identity must be carried at dispatch and presentation boundaries; lane/thread identity is
  not a substitute because one host or worker can serve several outputs.
- Range percentiles use one combined value per frame. Repeated events within one animation frame
  must not receive more statistical weight than a frame with fewer events.
- A successful queue-present return is not evidence that the presentation engine displayed the
  image; display timing stays explicitly unavailable without an independent qualified source.

### Failure and recovery cases extracted

- Server startup, absent viewer, browser disconnect, and application exit have distinct outcomes.
- A stable endpoint is required for reconnect; derived-port collision fails startup with an explicit
  override path rather than falling back to a different origin.
- Slow or absent viewers require bounded target-side retention.
- Query results may be unavailable and old available values can be mistaken for a new generation if
  reset/reuse ordering is wrong.
- Timestamp counters can wrap at the queue's valid bit width.
- CPU and GPU clock domains are not automatically comparable.
- Combining consecutive-frame or present gaps across views fabricates latency that no view
  experienced; sequences must be partitioned by view before gap calculation.

### Approaches rejected

- `cfg(debug_assertions)` alone: it prevents an optimized profiling build and does not express
  dependency inclusion as clearly as a feature.
- `cargo run --profile` without a name: Cargo owns that incomplete option syntax.
- A custom Cargo profile alone: profiles do not select dependency features.
- Always compiling the server into applications: it violates ordinary-build exclusion.
- A process-global tracing subscriber installed by a library: it takes ownership from hosts and
  conflicts with Telorgon's diagnostics policy.
- Unbounded channels or event vectors: a disconnected browser could consume application memory.
- Network writes from render/runtime threads: viewer behavior could perturb the measured target.
- Blocking GPU query reads or queue/device idle: measurement would change frame scheduling.
- Aligning ordinary Vulkan timestamps to CPU wall time: the domains are not calibrated.
- Building the profiler UI with Telorgon itself in the first version: Telorgon has no web backend and the
  viewer should not bootstrap through the renderer it is diagnosing.
- A TypeScript or frontend-framework build: the initial viewer is small, needs no Node toolchain,
  and is more auditable as directly embedded HTML, CSS, and JavaScript.
- A filesystem static-file service: it expands the route and path surface and makes the viewer
  depend on the application's working directory instead of the compiled profiler version.
- Automatic browser launching: it creates duplicate windows or tabs across application restarts and
  takes navigation control away from the user; the console URL and stable reconnect endpoint are
  sufficient.
- JSON for the live event stream: repeated field names and parsing allocations work against the
  bounded high-frequency transport; JSON remains appropriate for bounded metadata and errors.
- Copying Flutter DevTools, Puffin, or wgpu APIs: Telorgon needs its own event, ownership, privacy, and
  backend contracts.
- Inferring a view from lane names or frame ordering: workers can serve several views and globally
  issued frames can interleave.
- Painting every raw event into the overview/responsiveness canvas: dense animation traffic becomes
  an occupancy wall and makes isolated latency incidents less readable.

### Telorgon-specific decision

Use an optional dependency-light event package, a managed-host server package, bounded per-thread
`rtrb` producer rings, a versioned Telorgon binary session protocol, an Axum WebSocket service on one
current-thread Tokio runtime, a browser Performance workspace built from embedded vanilla HTML/CSS/JS,
existing truthful frame/render counters, and completion-delayed backend timing. Protocol v3 carries
a view ID independently of frame and presentation identity. The viewer applies view filters before
range and gap analysis, plots completed frames in recorded order with their session-relative
timestamps, aggregates outcomes, and keeps raw spans behind an explicit expansion. Expose the
complete workflow as the generated project-local `cargo profile` alias.

### Known gaps

- Resolved dependency licenses still require the repository's merge-time review.
- The managed server/browser path has compile and in-memory router evidence but still needs a manual
  end-to-end run on supported desktop platforms.
- The UI still needs automated browser interaction and visual-regression fixtures.
- CPU/inactive instrumentation overhead and Vulkan timestamp overhead need named machine and
  hardware evidence against the Section 15 budgets.
- Shared-device GPU-lane qualification remains open.
- Calibrated CPU/GPU time, native sampling, remote connections, and non-Vulkan GPU timing remain
  separate future designs.

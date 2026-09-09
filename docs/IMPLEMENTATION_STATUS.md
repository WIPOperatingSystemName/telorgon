# Telorgon Implementation Status

> X11 compatibility: the optional embedded feature is now connected to managed startup,
> dedicated Wayland-client access, XWM dispatch, serial association, retained surface
> composition, ordinary pointer-click focus, keyboard delivery and cooperative shutdown.
> Initial direct managed launches wait asynchronously for readiness/failure. Ready endpoints
> enter child environments; helper loss withdraws them and suppresses incident retries.
> Native, compatibility and downstream embedded builds have compile coverage; state/socket/
> subprocess fixtures cover components. On 2026-09-09 the real patched payload was rebuilt
> and embedded in the consuming compositor; the user reported that xmessage displays
> and accepts interaction. Managed X11 windows now use the same composed frame factory,
> titlebar hit tests and desktop geometry/state actions as native windows, with backend
> dispatch for focus/close and a separate X11 geometry adapter. The temporary Alt-drag
> gesture was removed. X11 titles use bounded UTF-8/legacy property readers; popups remain
> unframed. The user confirmed the framed window rendering. Resize previews now use the
> shared RGBA veil and subtree suppression. X11 defers resizing until release and settles
> against checked server geometry plus matching published content, without xdg acknowledgements.
> No-op resizes reuse matching content; delayed content and new grabs cannot settle stale targets.
> The resize-veil change awaits live validation.
> This user-reported smoke result does not establish the wider hardware/application matrix.
> Shared activation-environment updates, explicit X11-required launch errors, restarts,
> broader desktop policy, accelerated capabilities and application/hardware qualification
> remain outstanding. See the [manual smoke test](X11_SMOKE_TEST.md),
> [compatibility details](X11_COMPATIBILITY.md) and [qualification ledger](X11_QUALIFICATION.md).
> W01–W13 are not closed by this integration milestone.

> Managed session launching: both managed entrypoints initialize the global session API. Literal
> command execution, bounded pipe capture/input, child supervision, crash retries, and persistent
> recovery operate in real subprocess unit fixtures. Automatic DE socket selection has native
> socket coverage. TTY/KMS logout, real-client save prompts, multi-GPU selection, and optional
> shared D-Bus/systemd integration are implemented but await manual qualification. See
> [Managed sessions and process launching](SESSION_AND_PROCESS_LAUNCHING.md) for ownership and
> recovery limits; these additions are not production-qualified.

> Linux desktop HiDPI: logical layout/input, output-density text/client images, and physical KMS
> composition are integrated with CPU regression coverage. Automatic boot scaling uses reported
> monitor dimensions with a fixed override. Native client events and hardware presentation still
> await manual qualification; dynamic scale changes and multiple outputs are not implemented here.
> See [Logical units and output scaling](LOGICAL_UNITS_AND_OUTPUT_SCALING.md).

## Status vocabulary

This document records what the repository currently demonstrates. It is deliberately separate from
the target design in [PROJECT_SCOPE_AND_ARCHITECTURE.md](PROJECT_SCOPE_AND_ARCHITECTURE.md).

> Current packaging: all framework implementation formerly split among `telorgon-*` library crates
> is consolidated into named modules of the `telorgon` package. Historical gate entries retain old
> crate names as milestone vocabulary, but they do not describe current Cargo packages. The only
> other publishable package is `telorgon-macros`; `telorgon-shader-build` remains unpublished. See
> [Code layout](CODE_LAYOUT.md) for the authoritative mapping.

| Status | Meaning |
| --- | --- |
| Planned | Defined as target scope but not represented by a working implementation |
| Modeled | Data structures or tests model intended behavior without operational integration |
| Operational | Executes its intended function through at least one real integration path |
| Production-qualified | Meets documented conformance, platform, failure, and performance gates |

Passing a model-level test does not promote a subsystem to operational. Operational does not imply
production-qualified.

> Current Windows resize experiment: managed presentation prefers a same-adapter D3D11/DXGI HWND
> flip swapchain with `DXGI_SCALING_NONE`. Vulkan renders into D3D11-owned shared textures imported
> through Win32 external memory; an imported shared fence orders the Vulkan render, D3D11 GPU copy,
> and `Present1(1, 0)`. A keyed mutex owns each shared texture across the Vulkan/D3D11 boundary,
> and the Win32 subclass prepares expansions before the native target grows while deferring
> contractions through a coalesced private message until the native transaction unwinds.
> Capability failure retains the Vulkan WSI path, which defaults to FIFO and enables
> `VK_KHR_present_id` plus `VK_KHR_present_wait` when both extensions and features are available.
> The Windows resize barrier prefers the exact swapchain-local presentation ID, while maintenance
> fences remain responsible for semaphore reuse and retired-generation lifetime. When
> `VK_KHR_get_surface_capabilities2` and `VK_EXT_surface_maintenance1` are available, each
> swapchain with the enabled `swapchainMaintenance1` feature also requests one-to-one presentation
> scaling with supported deterministic gravity; unsupported drivers and present modes retain the
> legacy create path.
> The implementation is now separated into `telorgon-presenter-vulkan-wsi`,
> `telorgon-presenter-dxgi`, `telorgon-bridge-vulkan-dxgi`, and
> `telorgon-presenter-softbuffer`, with neutral lifecycle/metrics/completion contracts in
> `telorgon-presentation`. The former `telorgon-presenter-vulkan-winit` compatibility facade has been
> removed.

> Current widget closeout: mounted switch and checkbox fixtures use semantic toggle nodes,
> slider pointer input emits begin/update/commit value changes from mounted track geometry, and
> switch, checkbox, slider, progress, and activity visuals/semantics update through live bindings.
> Enter and Space activate focused controls, and Tab traverses the mounted focus order. Any older
> matrix wording below that lists these specific capabilities as planned is superseded by this note.

> Unified UI pipeline closeout: per-view generational interaction routing now isolates pointer,
> capture, focus-visible, keyboard activation, cancellation, drag, and controlled semantic states.
> Theme v4 catalogs, typed tokens, generation-safe scopes, dependency-limited replacement, sampled
> motion, rounded asymmetric borders, outside outlines, two shadows, primitive-local opacity, and
> affine spatial hit testing/rendering execute in the retained application pipeline. Older entries
> below describing Theme v3, renderer-side state guesses, rectangular-only borders, unrendered
> shadows, or disconnected raw input are superseded by this note.

> Composition interface closeout: `telorgon-compose` and `#[component]` now provide short-lived
> non-generic builders over persistent annotated component structs, owner-bound events, keyed
> reconciliation, lifecycle updates, dynamic retained text, and revisioned signal subscriptions
> with native host wakes. `Application::gui(...)` and
> `Application::desktop_environment(...)` are the only process entrypoint constructors. Windows,
> compositor backgrounds, and shell widgets are completed through strict mode-specific builders. The
> ordinary GUI host and the feature-gated Linux Wayland/KMS desktop host have
> operational integration paths; neither is implied to be production-qualified.
> The older mount/action runtime remains an advanced/internal compatibility layer, not the ordinary
> `Component` API.

> Linux scanout startup now negotiates renderer-specific buffers with exact
> Vulkan/KMS modifier matching, verified DRM adapter identity, CPU GBM/dumb-buffer
> fallback, bounded rollback and native allocation diagnostics. Both the invalid
> modifier sentinel and GBM transfer-write ABI are corrected. Portable regression
> tests and compilation are covered; this is not hardware qualification. See
> [Linux scanout negotiation](LINUX_SCANOUT_NEGOTIATION.md).
>
> Linux Wayland compositor closeout: Telorgon now owns bounded build-time official-protocol XML parsing, compiled static native
> `libwayland-server` descriptors and dispatch, surface/xdg/subsurface/seat/output state, SHM
> presentation, capability-gated DMA-BUF and explicit/implicit sync bridges, libseat/libinput/XKB input, and
> atomic libdrm/GBM KMS presentation. Compositor backgrounds, default window frames, pointers, semantic
> icons, and shell widgets are ordinary Telorgon compositions. The managed path now supports mapped
> software scanout and owned Vulkan rendering into explicit-modifier GBM scanout targets. Both
> paths consume the same renderer-neutral retained-scene deltas and ordered placements. Vulkan owns
> only Vulkan scenes and composites them directly into GBM in one render pass/submission instead of
> uploading a CPU-flattened output. Software owns only software scenes, rasterizes directly into one
> retained output framebuffer, and copies only accumulated output damage. Window-frame inputs reconcile through persistent component
> runtimes, and direct SHM client damage remains native-order regional data through retained scene
> and renderer updates. Full SHM reads/conversions use a single bounded FIFO worker with owner-thread
> completion and buffer release; independent retained/scene snapshots are prepared there so the
> completion path performs no whole-image copy on the input owner. Full-copy work is latest-wins per
> surface rather than an accumulating commit FIFO. Interactive resize uses a configurable RGBA solid-color
> preview with pointer-driven frame geometry, a start-state configure, and a final-size configure on
> release. New SHM copies for the veiled surface tree pause in a latest-wins mailbox during the drag;
> committed/desired geometry stays separate, configure acknowledgement and window
> geometry are latched to the applying surface commit, configure acknowledgement survives
> latest-wins image-copy replacement, window presentation feedback uses frame-owned revisions,
> and client content returns at native size only after the final applying publication.
> Post-release frame-callback hints permit redraw progress without claiming hidden content was shown;
> actual presentation later discards superseded feedback. Easy-frame designs can override preview RGBA
> and independently configure transparent content backing. Frame decoration is cut out beneath the
> content slot so client alpha reveals lower layers; opaque client buffers remain opaque.
> Client surfaces, subsurfaces, backing, and resize previews now share analytic inner-border rounded
> clipping anchored at the window top rather than the app/title-bar seam. Disjoint outline and
> inverse-clipped frame-fill patches preserve the rim and wider corner wedges without backing
> transparent content. Both renderers support the placement clips; Vulkan's shader bundle is GPU
> ABI 4 (the view record remains 192 bytes; flags now include per-clip inversion).
> Easy frames derive content bounds/rounding directly from title-bar height and border width,
> replacing the separate content-margin/radius configuration. Their resize targets follow the
> rounded border, exclude the interior, and support outward-only tolerance beyond the window bounds;
> custom templates retain their independently authored layout and hit regions.
> See [solid resize preview](WAYLAND_RESIZE_PREVIEW.md). Capability-gated Vulkan DMA-BUF commits are materialized into
> retained compositor textures with acquire/release sync-FD handling and no CPU pixel copy. A protocol
> acquire fence is preferred; otherwise the host exports the DMA-BUF reservation object's implicit
> writer fences into the same Vulkan wait path. The host also supports secure
> session lock, interactive window-management policy, activation, constraints, and pointer/touch
> drag-and-drop. It remains single-output; direct long-lived client DMA-BUF sampling, newer DMA-BUF
> feedback/syncobj protocols, input methods, hotplug/session recovery, and Linux hardware/conformance
> qualification remain incomplete. See
> [WAYLAND_COMPOSITOR_ARCHITECTURE.md](WAYLAND_COMPOSITOR_ARCHITECTURE.md).

> V3DV compatibility: owned and hosted Vulkan devices identify the driver and use a V3DV-only
> storage-buffer upload-completion workaround; all other drivers retain the precise default mask.
> CPU regression coverage does not establish that the reported Raspberry Pi diagonal resize artifact
> is resolved. The before/after Pi comparison and cross-driver hardware qualification remain user-run.
> See [V3DV geometry-upload workaround](V3DV_GEOMETRY_UPLOAD_WORKAROUND.md).

## Current classification

`telorgon::request_exit()` now requests clean shutdown of running managed GUI and Linux compositor
hosts, including from a plain shortcut function or another thread. GUI hosts receive a Winit user
event; compositor hosts reuse their runtime notifier and check the request around dispatch and
after shortcut handlers. Requests are coalesced and scoped to active runs; embedded/headless hosts
are unaffected. Registry tests cover cross-thread wakes, multiple hosts, repeat requests, and
run isolation. Live GUI/compositor shutdown qualification remains manual.

The Linux compositor supports `Compositor::keybindings(KeyBindings)` with named `fn()` handlers,
exact modifier matching, and layout-resolved `ShortcutKey` named/ASCII/raw-XKB symbols. Named
`A`–`Z` letters match either ASCII case; `.shift()` alone controls the required Shift modifier.
`Digit0`–`Digit9`, `F1`–`F12`, existing named keys, and ASCII/raw-XKB constructors match exact
symbols. Binding validation rejects overlapping named/exact letter chords in either order. The
adapter uses existing fresh-press, repeat/release capture, and session-lock routing; duplicate
chords panic during declaration, and the last builder shortcut configuration replaces the prior
one. This is separate from physical application command scopes. Portable tests cover dispatch,
exact matching, duplicate aliases, builder installation, capture, and locked-session isolation;
live seat/display qualification remains manual. See the usage example in
`WAYLAND_COMPOSITOR_ARCHITECTURE.md`.

| Area | Status | Current evidence and boundary |
| --- | --- | --- |
| Platform conformance host | Modeled, limited | `telorgon-platform-conformance` now provides an explicit manual monotonic clock, hard-bounded ownership-returning capture, a bounded multi-view canonical lifecycle driver, a deterministic stamped event host, and object-safe fake haptics/restoration service adapters. Replaying the same two-view observations and event inputs produces identical ordered updates and events. Capture saturation rejects before view mutation or stamp advancement; fake restoration keeps admission distinct from explicitly observed current truth and retains consumed opaque tokens by request identity until terminal completion. The crate has no Winit, native API, renderer, ambient clock, background thread, executor, timer, event loop, automatic dispatch, or fallback service. Broader fake services, input/text/accessibility/transfer conformance modules, runtime/scene/semantics replay comparison, scheduling assertions, and every native qualification remain planned |
| Mounted node storage | Operational | Nongeneric `MountedUi` owns the generational node arena, sparse neutral components, typed properties, transactions, and private test-only keyed node fixtures; application actions and public structural blueprints no longer reside in low-level UI storage |
| Component authoring | Partial operational | `telorgon-runtime::ViewRuntime` mounts one normal root `Component` through a `ComponentRuntimeDriver` lifecycle owner. Generational component/state/read arenas, atomic action transactions, derived reads, bindings, observers, and cycle diagnostics execute in portable tests. Slice 7 demonstrates `when`, homogeneous `for_each_keyed`, keyed `switch`, and portal child lifecycles with direct and derived structural-input validation, retained identity, branch type replacement, visual-host movement, and child-first teardown. Private generation-targeted routes deliver a child's own non-`Clone` observer, foundation-button, or neutral-listener action to that child or explicitly map, consume, or convert it to a runtime command at conditional, keyed-collection, switch, and portal boundaries. Slice 8 has injected executor-neutral local/send task hosts, transaction-staged starts, later-turn typed results, per-turn result budgets, cancellation handles, bounded progress senders with lossless backpressure, coalesced host wakes, unsupported-host rejection, executor-shutdown closure, and unmount cancellation. Runtime-owned one-shot and repeating monotonic timers stage typed actions, expose the earliest scheduler deadline, coalesce missed intervals, honor bounded turns, wake on cancellation, and close with the component generation. `telorgon-app::ManagedComponentRuntime` now supplies the explicit managed capability: local futures are polled only on the constructing owner thread, `Send` futures run on one named worker, host wakes are coalesced, and synchronous shutdown closes capabilities and joins that worker. The managed `run` host mounts root components and maps task wakes through its Winit user-event proxy; application-scoped tasks and editable-text component/session routing remain unimplemented |
| Layout | Operational, limited | Incremental box/flex-style layout, canonical geometry, scrolling, hit testing, focus order, and virtual collection behavior execute. Text intrinsic and constrained sizes now come from the same caller-owned, constraint-keyed shaped-run cache used by scene compilation; the former character-count width estimate is removed. A separate platform-neutral scroll transition owner now validates two-dimensional viewport/content extents, clamps offsets while reporting consumed and unconsumed deltas, preserves optional end-distance anchors across extent changes, computes nearest/start/center/end/fraction reveal targets, models drag/cancel/reduced-motion activity, and advances clamping ballistic motion only from caller-supplied elapsed time under generational motion IDs. A separate pure popup solver validates anchor/content/safe/occlusion geometry, resolves above/below/inline-start/inline-end alignment through canonical writing direction, tries exact ordered candidates before explicit reject/shift/resize/scroll fallback, subtracts occlusions into deterministic usable regions, and reports typed final geometry without mutating layout. These owners include no input routing, scheduler, animation clock, component/controller state, overscroll effects, snapping, nested-scroll policy, portal/lifecycle state, rendering, or platform conversion; the complete target primitive and adaptive layout vocabulary is not implemented |
| Input | Operational, limited | `telorgon-input` owns the native-type-free button, pointer/contact/tool, standardized physical/logical-key, location, bounded redacted key-text, repeat/synthetic, modifier/lock-state, input-event, phase, propagation, and default-response values used by the current app/UI route. Its source-neutral activation state machine covers primary-pointer capture, leave/re-enter arming, matching release, Space/Enter, semantic activation, cancellation, and competing-gesture suppression. A separate neutral focus owner now consumes generational targets in canonical order, validates candidate updates atomically, traverses forward/reverse with explicit scope edges, restores nested scopes, selects surviving successors/predecessors after removal without focusing a recycled generation, tracks focus-visible modality/preferences, and reports deterministic diagnostics in portable fixtures. The neutral composite owner consumes keyed canonical order, keeps active-descendant history distinct from controlled selection, emits typed selection requests, supports stop/wrap, Home/End, orientation and RTL-aware arrows, applies an explicit disabled-item discovery policy, and recovers only to surviving generations after removal. A portable gesture arena and tap/long-press/drag recognizers now model exactly-once competition, caller-owned deadlines, logical slop/axis policy, phased drag/long-press changes, and explicit loss/cancellation handoffs without timers, capture, or callbacks. A source-neutral shortcut matcher consumes atomic controlled binding snapshots, matches exact physical chords over generational innermost-first scopes, ignores disabled or repeat-suppressed bindings, supports modal blocking and explicit priority, and reports equal-priority ambiguity by returning typed command IDs rather than invoking commands. Desktop mouse movement/buttons, wheel, basic keyboard delivery, capture, focus, neutral ancestry propagation, and separately routed typed actions execute. The first Gate 8 button preserves `Pointer`, `Accessibility`, and `Programmatic` sources after completed activation validation and directly reuses the neutral activation owner in portable behavior fixtures. Raw key/focus/capture/gesture routing is not yet connected to that component behavior, and revisioned view/event metadata, stateful keyboard integration/event-envelope dispatch, unified multi-pointer behavior, touch, pen details, double/context gestures, drag and drop, localized shortcut display, and multi-stroke sequences remain planned |
| Semantics | Operational, limited | `telorgon-ui::semantics` owns mounted component-authored roles, names, state, values, advertised actions, participation, relationships, and virtual collection inputs. The new neutral `telorgon-accessibility` owner combines only validated exported nodes with mounted generational identity, resolved redacted strings, view-logical layout geometry/transforms, distinct keyboard and assistive focus, a nonzero tree generation/revision, complete immutable snapshots, and atomically revalidated node/string/focus deltas. It rejects malformed topology, unresolved participation, missing nodes/strings/relationships/focus, invalid geometry, hard-bound violations, stale delta bases, stale action generations/revisions, unknown targets, unadvertised actions, and incompatible or oversized action data. Live-region data, accessible text runs/ranges, imported attachment merging, runtime tree production/action dispatch, AccessKit mapping, and every native accessibility adapter remain planned |
| Overlays | Operational, limited | `telorgon-ui::overlay` now owns one platform-neutral ordered overlay lifecycle engine with generational entry identity, node/point/rectangle anchors, explicit parentage, modality, typed dismissal and outside-input policy, focus containment/initial/restoration records, forced anchor cleanup, top-first subtree closure, active-barrier queries, and deterministic diagnostics. Open validation rejects stale nodes/parents, invalid geometry, and incomplete modal focus policy before mutation; stale overlay generations cannot close reused slots. The application-domain `ApplicationOverlayHost` now binds one such neutral owner to one noninteractive, semantic-free full-view portal target, validates its exact mounted-UI generation before opening, delegates lifecycle records unchanged, and explicitly closes owned entries on view loss or unmount. `ApplicationOverlayController` now owns one such host and maps typed open/dismiss/anchor/view/owner commands to unchanged neutral focus/input/close effects while deriving inspection state from that same host. The neutral layout owner supplies pure writing-direction-aware exact/shift/resize/scroll popup geometry over safe and occluded bounds. The application package now supplies environment-aware placement defaults, a typed nonmodal Popup, and a named modal Dialog with mandatory initial focus, containment/restoration, and barrier/inert intent. None of these owners mounts portal content, routes input, applies focus/inert semantics, creates native windows, or executes callbacks |
| Text shaping, storage, editing, and glyph cache | Operational, limited | `cosmic-text` now produces constraint-aware retained geometry (advance, height, baseline, line count, and physical glyph positions) without mutating the atlas. Layout and paint share those cache entries; paint lazily rasterizes glyphs, and atlas generations invalidate only placement-dependent data after an atlas clear. Cache identity includes content, typography, scale, and width/height constraints. `TextBuffer` provides opaque contiguous UTF-8 storage, cheap immutable revisioned `TextSnapshot` values, and range-bounded chunks. Atomic edit batches reject stale revisions, invalid scalar boundaries, unsorted/overlapping ranges, oversized results, and invalid resulting selection/composition before mutation; successful batches publish text, selection, composition, one revision, precise old/new changed ranges, and an immutable snapshot together. Explicit neutral composition start/update/commit/cancel commands enforce revision and active-state ordering through that same atomic edit path. Logical grapheme/word boundaries, word lookup, and move/extend selection helpers use pinned Unicode 17.0.0 default UAX #29 segmentation; visual bidi/wrapped-line movement remains a layout/controller responsibility. A generation-aware neutral text-input session now emits bounded revisioned open/update/close snapshots and accepts revision-citing edit, composition, and semantic action deltas with explicit resynchronization and secure surrounding-text redaction. Runtime focus routing, every native IME/UTF-16 adapter, locale-tailored word breaking, virtual-keyboard execution, noncontiguous large-document storage, and qualified fallback behavior remain planned |
| Themes | Operational | Strict Theme v4 parsing, typed tokens and references, application/shell component catalogs and stable IDs, slot/variant/state contracts, deterministic precedence, immutable compiled snapshots, generation-safe roots/previews, atomic dependency-limited replacement, sparse local overrides, per-view sampled motion, reduced-motion policy, diagnostics, and deterministic `LTH4` encoding execute in the application frame pipeline. Theme v2/v3 and `LTH2`/`LTH3` inputs are rejected; archive decoding, external assets, and inheritance remain outside this cutover |
| Retained render scene | Operational | Dense box/glyph/image/material instances, ordered draw items, clip/spatial tables, scene deltas, dirty ranges, damage, glyph-atlas pages, versioned native RGBA/BGRA full and regional image writes/removals, and built-in material resources are applied independently by the software and Vulkan backends |
| Software renderer | Operational, limited | `telorgon-renderer-software` is the CPU reference implementation. It rasterizes ordered boxes, antialiased rounded asymmetric border rings, outside focus outlines, two shadows, primitive-local opacity, glyph masks, native RGBA/BGRA images, materials, affine spatial transforms, rounded clips, and opaque/alpha batches with retained damage and explicit readback. Broader effects and production color-management qualification remain incomplete |
| Native desktop host | Operational, limited | A Winit/Softbuffer desktop window path composes the renderer/platform-free `telorgon-runtime` view owner with layout, scene compilation, software backend orchestration, and presentation in `telorgon-app`; it is not yet the separated platform-service architecture and lacks production platform coverage |
| Linux Wayland desktop host | Operational, limited | With `desktop-wayland-linux`, `Application::desktop_environment(...)` owns one Linux-only thread that combines official `libwayland-server` transport, Telorgon protocol state, libseat/libinput/XKB pointer/keyboard/touch delivery, persistent reconciled background/frame/pointer/icon/widget layers, native-order damage-aware and transformed/scaled/viewporter SHM client images, capability-gated single-plane DMA-BUF GPU materialization with acquire/release sync FDs, data-device selection and pointer/touch drag-and-drop with MIME FD/action negotiation, activation, session lock, constraints, live compositor-scaled interactive window management, commit-scoped XDG state and presentation feedback, triple-buffered GBM primary scanout, completion-retired atomic ARGB8888 cursor-plane commits with coalesced cursor-only motion and composited fallback, strict startup-selected direct Vulkan or direct software composition, and atomic libdrm KMS. The renderer-neutral shell frame contains only retained deltas, ordered placements, clips, and output damage. Vulkan and software own disjoint scene/output maps; neither invokes the other. Compile and portable state tests pass without launching the host. The path is single-output, has no Linux hardware/conformance run, and does not yet provide direct long-lived client DMA-BUF sampling, hotplug/session reconstruction, input methods, DMA-BUF feedback v4/v5, DRM syncobj, or the rest of the unadvertised extension set |
| Development profiler | Partial operational | `telorgon-profiler` supplies compile-optional bounded per-thread event rings, static-label spans/counters, exact producer gaps, frame/presentation correlation, and an explicit embedded collector. `telorgon-profiler-server` supplies a managed-only authenticated loopback Axum/WebSocket service, fixed embedded vanilla assets, bounded retained history, capture export, and synchronous shutdown ownership. Its executable-derived stable loopback endpoint lets an already-loaded viewer clear stale content on disconnect, renew a prior exact-origin profiler cookie, and attach to a restarted app; live batches are deduplicated per event so late cross-thread sequence delivery cannot suppress newer frames. Label batches carry explicit category, unit, aggregation, and resource-display descriptors, including presentation/responsiveness. The Performance viewer shares one wall-clock range across Overview, Frame Work, and Responsiveness views; uses consistent labeled colors for CPU, GPU, and presentation events; supports plot-drag selection, per-frame hot-spot percentiles, grouped low-cost work, frame-correlated counters, presentation outcomes/retries/gaps, grouped selected-frame costs, and a depth-based raw flame chart. A raw zero-timeout Vulkan acquire lasting at least 100 ms now raises a bounded likely-driver/WSI diagnostic with measured duration, raw adapter/driver identifiers, update/rollback guidance, and a validation caveat. Compatible applications can forward the profiler feature and define a local `cargo profile` alias; the managed runtime activates only for the exact reserved flag. Runtime, layout, theme, render, software, Vulkan, presenter, and application paths publish correlated probes, and owned Vulkan frame slots implement completion-delayed no-wait timestamp queries. Unit, protocol, compile, feature-matrix, and timestamp wrap/conversion checks pass without starting the service. Manual browser/end-to-end behavior, automated UI fixtures, feature-off binary-symbol evidence, overhead budgets, and named Vulkan hardware qualification remain open; this tooling is not production-qualified |
| Platform integration contract | Modeled, limited | The native-free `telorgon-platform` neutral spine now includes typed nonzero identities; strict injected-domain event sequencing; generic typed capability descriptors; independent lifecycle axes; coherent immutable view and metrics publications; generic immutable event envelopes; structured redaction-safe failures; linear typed request completions; an injected monotonic-clock boundary; immutable bounded post-turn schedules; an extensible typed service registry; an opaque non-cloneable user-gesture grant boundary; and neutral window, data-transfer, clipboard, text-input, accessibility, cursor, display, external-URI, file-dialog, menu, notification, haptics, power, and restoration service contracts. Metrics preserve named spaces, zero extent, safe/avoid regions, display transform, color, and HDR facts under atomic revisions. Events retain view, stamp, metrics citation, coalescing evidence, and typed payload without translating or dispatching. Admitted requests bind one identity to one terminal result without fabricating observed truth. Window operations are per-view; data offers preserve exact formats and hard read bounds; clipboard snapshots distinguish system/selection ownership; text-input synchronization reuses canonical revisioned `telorgon-text` session values; accessibility publication reuses canonical `telorgon-accessibility` snapshots/deltas while action-event construction requires exact current tree state; cursor appearance, metrics-cited logical positioning, and scoped constraint leases remain distinct request families; display enumeration is bounded and revisioned while per-view association retains the exact canonical metrics publication; external URI intents are lexical, bounded, scheme-governed, and content-redacted; file-dialog intentions preserve bounded typed filters and return redacted URI-located resources with optional opaque sandbox grants; menu publications preserve bounded exact-revision application/view/status trees while source-qualified actions require an enabled visible action in the cited current snapshot; notification post/update/removal, authorization, and badge intentions use bounded redacted content and stable revisions while body/action/reply/dismissal events validate against an exact current notification; haptic intentions select only a semantic effect and bounded normalized intensity against explicit current device and user-setting observations; power intentions acquire explicit application/view-scoped idle or system-sleep inhibition leases against observed host policy; restoration transports hard-bounded opaque redacted tokens through exact revisioned application/view/session publication, consumption, and clearing. Cancellable close remains distinct from forced destruction, and user dismissal remains distinct from request cancellation. These models own no native query or handle, renderer, presenter, native range conversion, callback, queue, event loop, cleanup execution, network operation, handler launch, dialog object, filesystem operation, native menu or notification object, command execution, notification scheduling, haptic waveform or hardware control, native power inhibitor or policy decision, restoration serializer/storage/native object/persistence policy, or application-exit policy. The Winit application host now operationally consumes the neutral post-turn schedule and generation-checked view registry for bounded owner wakes, monotonic timer deadlines, and typed redraw resolution. Focus/environment snapshot fields, neutral event translation, and all native service/IME/accessibility/cursor/display/URI/file-dialog/menu/notification/haptics/power/restoration adapters remain planned; the deterministic conformance host remains modeled in its first bounded slice |
| Platform services | Modeled, limited | The neutral crate now models capability-checked per-view window operations, bounded exact-format data offers and asynchronous read admissions, system/selection clipboard snapshots plus publish/clear admissions, per-view text-input synchronization plus canonical converted delta envelopes, per-view accessibility capability/tree publication plus exact-snapshot action admission, per-view standard/custom cursor appearance and scoped constraints, bounded revisioned display enumeration plus exact per-view display/metrics association, bounded redacted external-URI open admission with scheme-specific policy, asynchronous open/save/folder dialog admission with bounded filters, selection limits, redacted resource metadata, and opaque sandbox grants, bounded revisioned application/view/status native-menu tree publication with roles, accelerators, checked state, and exact-snapshot action admission, authorization-aware system-notification post/update/removal, badge, bounded-action, inline-reply, and exact-response admission, semantic haptic-effect admission with explicit output-device effect support, observed user-setting state, and fixed-point normalized intensity, policy-aware idle/system-sleep inhibition through application/view-scoped observable RAII leases, and bounded opaque restoration-token publication/consumption/clearing over exact application/view/session histories. Custom cursors and power inhibitors have explicit scoped leases; custom cursors have hard geometry/animation limits; display capabilities expose accuracy without alternate metrics; URI, dialog, haptics, and power requests consume opaque adapter-validated gesture grants when supplied. These contracts execute no native operation and have no Winit, AccessKit, arboard, input-method, virtual-keyboard, monitor/URL handle, external handler, native dialog, filesystem-path, native menu/notification object, command invocation, delivery scheduler, haptic waveform/timing/hardware owner, native power inhibitor/timer/policy owner, restoration serializer/storage/native object/persistence owner, or fallback implementation. All native adapters and operational qualification remain planned |
| Managed cross-platform presentation entry point | Partial operational | `Application::gui(...)` and `Application::desktop_environment(...)` are the only process entrypoint constructors, and both own the runtime `Renderer` policy. The GUI mode requires one content-bearing `Window`; the Linux desktop-environment mode requires one compositor background; composed `ShellWidget`s are optional. The operational GUI host's default `Auto` policy tries the real Windows Vulkan surface/swapchain/acquire/submit/present assembly and falls back to Winit/Softbuffer before mounting if Vulkan startup is unavailable; exact Vulkan and software policies are also available without consumer Cargo-feature selection. The application host coalesces native input/resize work and owns bounded presentation scheduling. The Linux desktop mode is a separate feature-gated bare-metal Wayland/libseat/KMS owner-thread path: exact `Vulkan` imports explicit-modifier GBM targets into Telorgon's owned Vulkan renderer, `Auto` tries that path then falls back, and exact `Software` uses mapped scanout. The post-Slice5 Windows developer-hardware E5 regression passed with six presented frames, resize, suspend/resume, surface replacement, shutdown, and zero validation errors. The Linux desktop graph has compile and portable state evidence but no hardware launch or production qualification |
| Host-provided render-area entry point | Operational, limited | `telorgon-embed` provides window-system-free `UiDevice`/`UiHost`/independent-view assembly, target-to-local input-coordinate mapping, prepare-before-record scheduling, and a zero-allocation/zero-record idle return. `telorgon-renderer-vulkan` imports host-owned instances/devices/queues, validates host targets and subregions, appends dynamic-rendering commands to an already-recording primary command buffer, and returns linear resource-pin receipts without begin/end/reset/submit/present operations. The accepted E6 developer-hardware run recorded two subregions with host-owned submission and command-buffer lifecycle, retired two completion receipts, and reported zero validation errors. Custom allocator callbacks, render-graph adapters, and non-Vulkan hosted backends remain incomplete |
| Shared-device multi-view composition | Operational, limited | One `UiDevice` shares Vulkan pipelines, layouts, samplers, and allocator state while `UiHost` retains independent application runtime and `VulkanScene` state per view. The accepted E6 fixture recorded two independent scenes into host-selected left/right subregions of one host target, committed both receipts to one host completion point, performed zero Telorgon submissions or command-buffer begin/end operations, and reported zero validation errors |
| Rendering hardware interface/backend SPI | Operational, limited | The internal `RenderBackend` device/per-view-scene/frame/typed-target contract executes through software, owned offscreen Vulkan with explicit submission/readback, and borrowed command-only Vulkan with host completion. The software reference follows the same linear-premultiplied compositing contract, sRGB/linear image decoding, and linear glyph/image sampling as Vulkan while returning target-encoded RGBA8 readback. Vulkan scenes resolve logical image IDs to linear same-device external-image leases without a pixel upload and return native acquire/release requirements through hosted receipts; a platform-gated Linux adapter now additionally owns narrow DMA-BUF/modifier and sync-FD import/export. Complete effect plans, broader OS-handle profiles, and other vendor backends remain planned |
| Render-plan trace backend | Planned | No independent trace backend currently validates cross-backend resource lifetimes, dependencies, bindings, or usage transitions |
| Vulkan renderer | Operational, limited | Dynamic `ash` loading, validation/debug reporting, non-CPU adapter selection, Vulkan 1.3 owned rendering, mixed retained primitives, regional native RGBA/BGRA image staging, in-place idle-image patches, completion-pinned on-GPU preservation and reusable copy-on-write images, and Slice 6 hosted command-only recording execute on developer hardware with accepted evidence. The accepted same-device E8 run sampled one host-owned image with zero external upload, real acquire/release semaphores, one completion receipt, and zero validation errors. The Linux P4 adapter now cross-compiles with explicit enabled-extension declarations, deterministic single-plane RGBA/BGRA fourcc/format/modifier negotiation for an exact requested usage, strict DRM modifier/layout validation, owning DMA-BUF and acquire sync-FD consumption, implicit reservation-fence export when no protocol acquire fence is supplied, imported Vulkan image/memory/view lifetime, foreign queue-family transfer, one-shot release sync-FD export, and commit-time release enforcement. Its Linux hardware fixture selects a jointly importable/exportable negotiated tuple and cross-compiles but has not run, so that profile remains unqualified; multi-plane/YUV, protected content, custom host allocator callbacks, shadows/effect passes, and production qualification remain incomplete |
| Windows DXGI bridge and native presenters | Operational, limited | Managed Windows now prefers a D3D11/DXGI HWND flip-sequential swapchain with `DXGI_SCALING_NONE` when the selected Vulkan adapter exposes the required LUID and Win32 external-memory/fence profile. Native D3D11/DXGI device, swapchain, copy, resize, present, and fence-wait ownership is isolated in `telorgon-presenter-dxgi`; it has no Vulkan dependency. D3D11-owned BGRA sRGB texture creation, Vulkan import, keyed-mutex transfer, imported fence/timeline sequencing, LUID validation, and dual-API generation retirement are isolated in `telorgon-bridge-vulkan-dxgi`. Failure to signal D3D completion remains a distinct lifetime-unproven device-loss path. The bridge deliberately serializes cross-API access and retains the bounded display-interval/DWM resize barrier because present success and D3D copy completion are not exact display proof. Because default `WM_WINDOWPOSCHANGED` processing emits the nested `WM_SIZE`, the thread-affine Win32 subclass uses direction-aware ordering: an expansion prepares the exact larger buffer before native processing exposes the larger target, while a contraction retains the old larger buffer for clipping and posts one coalesced private HWND message to commit after the native transaction unwinds. Both directions receive a post-transaction synchronization pass. Capability failure retains `telorgon-presenter-vulkan-wsi`, including queried sRGB format selection, FIFO-by-default or optional mailbox, exact present ID/wait, maintenance fences, one-to-one scaling when supported, responsive exact-extent resize, and the explicit deferred-preview compatibility policy. Software native transfer is separately owned by `telorgon-presenter-softbuffer`. The Winit host prepares runtime state once and hands a revisioned neutral metrics/delta packet to the selected assembly; it no longer hands `AppRuntimeCore` to a presenter. Portable tests cover direction selection, repeated post-boundary expansion synchronization, the scaling-none flip descriptor, bridge texture flags, monotonic cross-API fence pairs, neutral lifecycle/revision rules, and Softbuffer conversion/damage clipping. A targeted user run on the affected hardware confirms that the corrected sharing/keyed-mutex route renders and removes the original stretch/squash artifact; the broader resize matrix remains unqualified. The earlier Vulkan E5 regression passed six presented frames, resize, suspend/resume, surface replacement, shutdown, and zero validation errors; Linux Vulkan surface selection only has compile evidence. |
| GPU ABI and Vulkan shader bundle | Operational, limited | GPU ABI 1.0 POD records have exhaustive size/alignment/offset tests. The offline tool deterministically compiles box, glyph-mask, ordinary-image, and built-in material vertex/fragment variants for Vulkan 1.3/SPIR-V 1.6, validates them with SPIR-V Tools, checks the focused interface and CPU-matching instance-array strides with rspirv, and packages hash-checked artifacts. Clip-mask/effect/final-encode variants and a complete reflection manifest remain unimplemented |
| Other GPU backends | Planned | Metal, Direct3D, and portability-backed renderers are not implemented |
| External/compositor surfaces | Operational, limited | Surface identity, permanent roles, double-buffered state, damage, xdg/subsurface relationships, frame-after-presentation callbacks, SHM copies, buffer releases, and retained image composition are integrated with the Linux Wayland host. The Vulkan same-device zero-copy resolver is E8-qualified with real binary acquire/release requirements and completion-pinned linear leases. The Linux bridge now converts exact Vulkan device capabilities into protocol DMA-BUF tuples, imports a validated single-plane generation with an acquire sync FD sourced from either the explicit-sync protocol or the buffer's implicit reservation fences, and binds it into a Vulkan scene; explicit protocol releases are revision-scoped. The managed KMS path deliberately advertises SHM only until that Vulkan bridge is selected end to end. Multi-output qualification, multi-plane/YUV, protected content, and Linux hardware execution remain incomplete |
| Materials | Operational, limited | Backend-neutral versioned solid and horizontal/vertical two-color linear-gradient resources execute in both the software reference and Vulkan material pipeline. The broader material graph, shadows, filters, intermediate targets, and cached effect passes remain planned |
| Application primitive/component domain | Partial operational | `telorgon-primitives-application` owns validated, revisioned, platform-neutral per-view environment snapshots plus one atomic runtime publication binding with six aspect-specific dependency-tracked reads. `telorgon-components-application` owns controlled change phases/value proposals, Compact/Standard/Touch target metrics, the four Tier A actions, checkbox/radio/switch Tier A choice controls, a validated generic range model, the Tier A slider, the read-only progress and activity indicators, the read-only meter, the application text-controller plus bounded edit-history foundations, the basic single-line text field plus multiline, search, numeric, and secure companions, the mounted application overlay-host plus typed command-controller seams, the application popup-placement policy adapter, the typed nonmodal popup open seam, and the typed modal dialog seam. The labelled button provides canonical activation behavior, explicit busy policy, deterministic typed state styles, button/name/enabled/busy semantics, density-floor mounting, and source-preserving runtime actions. The icon button reuses those owners, requires an explicit accessible name independent of its decorative `IconArtwork`, validates icon-slot size/opacity, and keeps artwork smaller than the enforced hit target. The toggle button consumes a parent-owned `Read<bool>`, exposes semantic `pressed`, and derives a committed inverse proposal from the latest read on every activation without mutating it. The link owns a validated opaque destination plus separate navigation and context-command outputs, exposes link/name/destination semantics, and performs no navigation, clipboard, or platform service inline. The checkbox consumes parent-owned `CheckState`, uses explicit binary/tri-state cycle policy, exposes checked/mixed semantics, and emits only source-preserving committed proposals derived from the latest read; incompatible binary `Mixed` values reject without an action. The keyed radio group wraps the neutral composite owner for one-tab-stop active-descendant movement, keeps focus distinct from parent-owned `Read<Option<K>>` selection, skips disabled items, and emits selection proposals without committing them. The switch consumes a parent-owned `Read<bool>`, exposes switch checked semantics, and derives a source-preserving committed inverse proposal from the latest value without mutating it. The generic range model validates finite ordered bounds, positive step/page-step values, deterministic formatting, ordered bounded marks, clamping, nearest-step normalization, and discrete movement for `f32`, `f64`, or an explicitly implemented scalar. The controlled slider maps orientation, writing direction, reversal, steps, pages, bounds, and cancellable drag-arena transitions into source-preserving proposals; its mounted node exposes numeric slider semantics and density-aware typed visuals. The progress indicator consumes a parent-owned determinate/indeterminate value, reports bounded formatted numeric semantics only for determinate input, reports busy without a fabricated number for indeterminate input, and mounts noninteractive Compact/Standard/Touch typed visuals. The activity indicator consumes parent-owned active state, always leaves its semantic value empty, reports busy only while running, and resolves typed running/inactive plus standard/reduced-motion visuals without owning a clock. The meter consumes a parent-owned bounded value, validates typed bands covering the shared range model, exposes distinct formatted numeric meter semantics, and mounts the selected band through density-aware styles. The local application `TextController` delegates revisioned storage, atomic edit/composition validation, immutable snapshots, and generational input-session deltas to `telorgon-text`, while publishing typed text/selection/composition/submission/rejection results without native IME behavior. The controller optionally owns the bounded edit history, records explicitly classified direct/session edits at caller-owned monotonic times, groups committed composition as one unit, exposes typed undo/redo availability and commands, drops plaintext history for secure sessions, and retains mutation solely in the neutral buffer. The basic field owns one controller, validates Editable/ReadOnly/Disabled/Secure single-line policy, routes typed edit/selection/submit/history commands, exposes mode-correct TextInput semantics and availability, and mounts density-aware visuals with secure redaction. The multiline area composes that owner, routes line-break edits and explicit newline-versus-submit Return policy, exposes multiline input/semantic state, and mounts a validated visible-line floor. The search field composes the single-line owner with Search-purpose configuration, SearchBox semantics, revision-checked undoable clear, content-derived clear availability, and mutation-free Search submission. The generic numeric field preserves intermediate decimal editing grammar, checks complete f32/f64 values through the shared RangeModel constraints/formatting owner, publishes typed parse and mutation-free commit results, and mounts numeric or invalid semantics. The dedicated secure field fixes omitted diagnostic and semantic content, bullet-redacted mounted content, disabled plaintext history/copy/cut, capability-gated paste availability, and snapshot-free typed outputs. The application overlay host binds exactly one neutral lifecycle owner to one live full-view portal target, rejects duplicate/stale/cross-view associations, delegates ordered modal/dismissal/focus effects unchanged, and closes entries explicitly on unmount. Its controller owns that one host, routes typed open/dismiss/anchor/view/owner commands, derives inspection state without copying the stack, and returns effects without applying them. The application placement adapter supplies a stable ordinary-popup candidate/Shift policy, derives logical safe bounds from the environment, forwards occlusions and writing direction, and preserves the neutral solver result plus scale-sensitive recomputation inputs. The typed Popup pairs lifecycle and resolved anchor geometry, opens only as nonmodal through that controller after placement succeeds, and returns the generational lifecycle/focus output with its placement without mounting content or applying effects. The typed Dialog requires a name and initial focus, fixes modal containment/opener restoration, returns barrier/inert intent, and requires explicit outside-dismissal opt-in while preserving placement-before-open atomicity. Headless/mounted fixtures demonstrate canonical action behavior, cancellation, disabled/busy policy, semantic actions, shared state priority, controlled rejection/acceptance, 44-pixel Touch mounting, mounted source preservation, portable check-cycle invariants, checkbox tri-state/rejection behavior, radio reorder/removal/directional/group-item semantics, switch pointer/Space/controlled-value behavior, portable range boundary/error cases, slider directional/phased/no-duplicate behavior, determinate/indeterminate progress semantics, activity busy/reduced-motion semantics, meter band/numeric semantics, controller edit/session/rejection behavior, owned edit-history grouping/budget/composition/secure-session/undo-redo behavior, text-field validation/mode/command/semantic/mount/redaction behavior, text-area multiline/Return/semantic/mount behavior, search configuration/clear/submit/semantic/redaction behavior, numeric intermediate/constraint/commit/semantic behavior, secure privacy/context/output/mount behavior, overlay-host mount/lifecycle/unmount behavior, overlay-controller command/effect behavior, application safe-area/RTL/occlusion/default/custom placement behavior, popup nonmodal/default/nested/atomic-open behavior, and dialog modal/focus/containment/barrier/atomic-open behavior. The application primitive domain now provides named Application roots, typed content/navigation/status regions, primitive-only `ApplicationUiExt` mount conveniences, explicit HUD coordinate/hit/semantic policy, viewport-relative content placement, host-projected world anchors, opaque revisioned render-target/video identities with metadata-only protection, and bounded payload-free diagnostics. Both domain crates have no shell, native, or backend dependency. Dynamic mounted style/semantic publication, automatic mounted slider input and semantic-action routing, high-frequency progress coalescing/live announcements, component-catalog extension conveniences, sheet, tooltip, toast, remaining overlay components, later Tier A controls, and gallery specimens remain planned |
| Shell primitive/component domain | Partial operational | `telorgon-shell` now owns one atomic bounded cross-model snapshot and the executor-neutral `ShellHost` request boundary plus fixed diagnostics and structured errors. `telorgon-primitives-shell` owns a named output-scoped root, host-grant layer narrowing, output-local logical mapping, canonical authorized layer mounting, immutable client-surface metadata/tree assembly, explicit content placeholders, host-authorized retained snapshot metadata, revision-bound reservation proposals, exclusive lower-layer hit decisions, exact client input eligibility mapping, capability-checked move/resize intentions, output edge/corner geometry with non-pointer alternatives, fixed diagnostics, and mount conveniences. `telorgon-components-shell` now owns the exact-snapshot window frame/titlebar/control/shadow/snap-preview catalog, policy-owned workspace view/stack/tiling/floating/switcher/overview presentations, output-edge panel/taskbar/dock facilities, a clock-free auto-hide transition boundary, launcher/list/grid/start-menu presentations over exact host application entries, status-area/clock/indicator/media/quick-settings/extension views over exact host system-status snapshots, privacy/lifecycle-aware notification host/center/dialog/OSD presentations, controlled lock/system-modal content hosts, and fixed payload-free diagnostics. Mounted fixtures demonstrate Window semantics, stable chrome/client hosts, capability-derived typed requests without optimistic mutation, visual-only decoration/previews, exact back-to-front reconciliation and region geometry, source-qualified workspace/application/status/notification intentions, unapplied revision-bound reservation requests, explicit caller-timed panel reveal transitions, privacy-aware status/notification presentation, disabled unavailable/unauthorized actions, and exact authorized secure-layer composition. Dynamic host reads, policy enforcement/integration, event forwarding, external-image import, and native protocol adapters remain planned |
| Application/shell domain isolation | Partial operational | Application and shell primitive/component crates enforce sibling dependency isolation, and the shared theme engine now has distinct application/shell root and preview namespaces. Independent umbrella build profiles, domain-specific typed style registries/default assets, and split application/shell galleries remain incomplete |
| Mobile/touch platform host | Planned | No Android, iOS, or other mobile lifecycle and presentation path is operational |
| Accessibility adapters | Planned | UI Automation, AT-SPI, Android Accessibility, and Apple accessibility export are not implemented |
| Bundled developer applications | Removed | Gallery, Theme Studio, and the theme build/create command wrappers are no longer workspace packages. Theme parsing, compilation, archives, previews, and public components remain available as library APIs. |

No subsystem is currently classified here as production-qualified.

## Active integration checklist

- [x] The Gate 8 foundation transition owners have portable direct-package fixtures.
- [x] The neutral application environment has one source owner and no shell/native dependency.
- [x] Its public records compile through the application primitive crate and umbrella seam.
- [x] One runtime binding publishes contiguous validated environment snapshots atomically and six
  aspect reads suppress downstream work for unrelated or unchanged updates.
- [x] Stale, skipped-revision, mismatched-change-set, and cross-owner environment publications leave
  all mounted aspect reads unchanged in portable fixtures.
- [x] The Gate 9 neutral platform crate depends on `telorgon-core` plus canonical `telorgon-text` and
  `telorgon-accessibility` protocol values and exposes compact typed nonzero view, data-offer,
  request, and native-surface identities without native or operating-system protocol values.
- [x] Reusing a platform object slot requires a different generation, and stale identities remain
  unequal through ordered owner maps in portable fixtures.
- [x] Per-host-stream event stamps assign strict nonwrapping sequences over caller-injected shared
  monotonic instants, preserve optional same-domain source time, and reject time regressions without
  reading a clock or owning a queue, timer, event loop, or native conversion.
- [x] Generic platform capability descriptors keep support, unavailability, permission, execution,
  recent-user-gesture policy, typed operations, and typed limits separate without probing or
  invoking a service.
- [x] The neutral accessibility owner publishes immutable bounded complete trees and exact-base
  deltas with stable mounted IDs, resolved redacted strings, layout geometry, distinct focus, and
  full cross-node validation without pixels, native types, callbacks, queues, or runtime dispatch.
- [x] Per-view accessibility service capability/publication and canonical assistive-action event
  admission preserve tree generation/revision and reject stale, missing, or unadvertised targets
  before a portable event exists.
- [x] Per-view cursor capability keeps appearance, visibility, metrics-cited logical positioning,
  and confinement/lock admission distinct; custom images and animations enforce hard geometry,
  frame, duration, and byte limits while diagnostics omit pixel content.
- [x] Successful cursor confinement or lock completes with a non-cloneable adapter-owned lease whose
  generational identity and revocation status are inspectable and whose concrete drop releases the
  long-lived effect.
- [x] Display enumeration publishes bounded immutable snapshots with generational identities,
  validated display-logical/physical geometry, optional referentially valid primary identity, and
  strictly advancing complete change payloads.
- [x] Display capability exposes explicit accuracy dimensions, while per-view association rejects
  identities absent from its cited `DisplayRevision` and retains the exact `ViewSnapshot` and
  canonical `ViewMetricsSnapshot` rather than duplicating scale, transform, color/HDR, safe-area,
  or avoidance vocabulary.
- [x] External URIs validate a hard-bounded absolute ASCII lexical envelope, normalized scheme,
  RFC-style characters, and percent escapes while URI content stays absent from diagnostics and
  applied receipts.
- [x] URI capability preserves a bounded unique ordered supported-scheme set with independent
  permission, execution, recent-gesture, and byte-limit policy; open admission remains linear.
- [x] The generic `UserGestureGrant` is an opaque non-cloneable adapter value moved into a request,
  omitted from diagnostics, and available only for private adapter downcast plus
  view/age/generation/focus/scope/single-use validation before a native call.
- [x] File-dialog open/save/folder intentions validate bounded typed format/extension filters,
  save-name policy, multiple-selection limits, per-view capability, and optional gesture evidence
  before linear asynchronous admission.
- [x] Selected dialog resources use redacted absolute URI locators rather than assumed filesystem
  paths and preserve kind, access intent, bounded redacted display metadata, and optional opaque
  non-cloneable sandbox grants whose concrete lifetime remains adapter-owned.
- [x] Applied user dismissal remains a typed file-dialog result distinct from cancellation of an
  admitted request.
- [x] Application, per-view, and status/tray menu scopes publish bounded immutable trees with
  stable nonzero item identities, exact monotonic revisions, hard depth/item/accelerator limits,
  redacted labels, and validated separator topology.
- [x] Menu roles are kind-checked and unique, while physical shortcut chords retain separate
  bounded localized display labels and reject ambiguous duplicates before publication.
- [x] Native menu actions cite the exact current scope, revision, and item and preserve pointer,
  keyboard, accelerator, assistive, platform-role, or status source without invoking a command;
  stale, absent, submenu, separator, hidden, disabled, or unadvertised actions reject first.
- [x] System-notification titles, bodies, action labels, and inline replies have separate hard and
  adapter-narrowed bounds and remain redacted from descriptor, request, response, and error
  diagnostics; action identities and default/dismiss roles are validated before publication.
- [x] Notification post/update histories retain stable nonzero identity and exact successor
  revisions, while exact-current removal, authorization options/results, and bounded numeric badge
  updates remain separate linear request families with no scheduled-delivery field.
- [x] Body, visible-action, inline-reply, user-dismissal, system-dismissal, and expiry responses
  cite the exact current notification revision and validate their advertised action/reply
  relationship before becoming source-qualified portable events that invoke no command.
- [x] Haptic requests select one semantic effect and exact fixed-point normalized intensity without
  accepting a waveform, frequency, duration, vendor identifier, or native output handle.
- [x] Haptic capability snapshots distinguish temporary device absence, the exact supported
  semantic-effect set, intensity-control support, adapter-narrowed intensity, and enabled,
  disabled, or unknown user-setting state before linear request admission.
- [x] Haptic service registration and completion remain object-safe and linear; opaque optional
  gesture evidence can be consumed only by the adapter and no neutral owner drives hardware.
- [x] Power-inhibition requests distinguish idle response from system sleep, use explicit
  application or generational-view scope, and carry one semantic policy reason rather than a
  duration, native token, arbitrary reason string, or platform command.
- [x] Power capabilities independently expose inhibition kinds, supported scopes, observed
  Allowed/Denied/Unknown policy, permission/execution/gesture requirements, and a hard-bounded
  adapter-narrowed concurrent-lease limit before admission.
- [x] Applied power inhibition is owned by one non-cloneable adapter RAII lease with stable
  generational identity, observable active/revoked state, and deterministic release on drop.
- [x] Restoration tokens are nonempty, capped at 64 KiB, single-owner across publication and
  consumption, and content-redacted from token, record, request, result, and error diagnostics.
- [x] Application, generational-view, and generational-session restoration histories use exact
  nonzero revisions; initial publication requires revision one, updates require the immediate
  successor, and clearing cites the exact current snapshot.
- [x] Restoration capability and linear admissions keep publish/update/consume/clear operations,
  supported scopes, adapter token-size limits, unavailable scope, stale revision, and returned
  consumed-token ownership explicit without serializing state or accessing storage.
- [x] The deterministic conformance clock advances only through explicit nonregressing input, while
  bounded captures preserve insertion order and return rejected payload/completion ownership on
  saturation rather than dropping, reallocating, coalescing, or dispatching entries.
- [x] The conformance view driver retains up to 64 independent canonical `ViewState` owners and
  preflights update capacity so rejected observations cannot mutate view truth; redundant
  observations remain visible in the ordered trace.
- [x] The deterministic event host reproduces identical two-view stamped traces from identical
  inputs and leaves sequence state unchanged when a view is absent, capture is full, or source time
  is invalid.
- [x] Fake haptics and restoration services implement the actual object-safe traits, revalidate
  capabilities and exact revisions, issue deterministic request identities, retain only bounded
  payload-free invocation metadata, and preserve opaque consumed-token ownership until completion.
- [x] The application component crate has one controlled-change/density owner, a curated component
  prelude, direct fixtures, and no shell/native/backend dependency.
- [x] Component density policy cannot reduce the 24/32/44 logical target baselines or a stricter
  accessibility/platform minimum.
- [x] The first Tier A button mounts a real semantic/focusable foundation node, enforces its density
  floor, resolves typed state styles deterministically, and routes completed activation sources.
- [x] The Tier A icon button reuses the button-family behavior/state priority, requires a separate
  accessible name, keeps its image child decorative, and validates icon-slot geometry/opacity.
- [x] The Tier A toggle button reads the latest parent-controlled boolean at activation time and
  emits only a committed inverse proposal with the preserved source; rejection leaves it unchanged.
- [x] The Tier A link action validates and preserves an opaque destination, emits navigation and
  context operations as typed values, and mounts link/name/destination semantics without invoking a
  platform service.
- [x] The canonical `CheckState` owner validates explicit tri-state order, while its binary policy
  rejects `Mixed` input and cannot produce `Mixed` output.
- [x] The Tier A checkbox reads the latest controlled `CheckState`, emits only a cycle-derived
  committed proposal, exposes checked/mixed semantics, and rejects incompatible binary `Mixed`
  values without fabricating an action.
- [x] The keyed Tier A radio group reuses the neutral composite owner, contributes one mounted tab
  stop, preserves active keys across reorder/removal policy, skips disabled items, and emits typed
  selection proposals independently from controlled selection.
- [x] The Tier A switch reads the latest controlled boolean, reuses canonical pointer/Space
  activation, emits only a source-preserving committed inverse proposal, exposes switch checked
  semantics, and retains its 44-pixel Touch floor.
- [x] The generic range model validates finite ordered bounds, positive step/page-step values,
  deterministic formatting, strictly increasing bounded marks, clamping, and step normalization
  without owning a component value or input route.
- [x] The controlled Tier A slider maps orientation/direction/reversal-aware navigation and shared
  drag-arena transitions into phased source-preserving proposals, suppresses duplicate updates,
  and mounts named numeric semantics plus a 44-pixel Touch floor.
- [x] The read-only Tier A progress indicator consumes controlled determinate/indeterminate input,
  reports bounded formatted numeric semantics or busy-without-number semantics respectively, and
  mounts nonfocusable Compact/Standard/Touch typed visuals.
- [x] The read-only Tier A activity indicator consumes controlled active state, reports busy without
  a numeric value only while running, and resolves deterministic density and reduced-motion visual
  intent without owning a clock, task, or timer.
- [x] The read-only Tier A meter validates ordered typed bands covering its shared range, reports a
  formatted numeric value through a distinct meter role, and mounts the selected band color without
  exposing actions or focus.
- [x] The application `TextController` locally wraps the revisioned neutral buffer/composition/session
  owners, publishes typed text/selection/composition/submission/rejection results, and preserves
  atomic stale/invalid-edit rejection without duplicating text storage or platform IME behavior.
- [x] Bounded application edit history groups only adjacent compatible typing/deletion, preserves
  edit selections across undo/redo, enforces unit/byte budgets, and delegates every restoration to
  the revisioned `TextController` mutation path.
- [x] `TextController` optionally owns that history, records explicitly classified direct/session
  edits with caller-supplied monotonic times, groups committed composition as one unit, publishes
  typed undo/redo availability and commands, and drops plaintext history for secure sessions.
- [x] The basic single-line `TextField` owns one controller, validates label/mode/return policy,
  routes typed edit/selection/submit/history commands, mounts a neutral `TextInput` with density and
  semantic state, and never mounts or reports secure plaintext.
- [x] The multiline `TextArea` composes that field owner, accepts revisioned line-break edits,
  separates newline insertion from configured submission, reports multiline semantics, and mounts
  a validated visible-line floor without duplicating controller/history/security policy.
- [x] `SearchField` composes the single-line field owner, reports Search-purpose and `SearchBox`
  semantics, publishes revision-checked undoable clear separately from ordinary edits, and submits
  Search without mutating text or calling a platform service.
- [x] Generic `NumericField<f32/f64>` preserves locale-neutral intermediate editing text, publishes
  typed valid/intermediate/invalid state after edits/history, commits only finite in-range values,
  and mounts formatted numeric semantics without rewriting controller text.
- [x] `SecureField` fixes omitted diagnostic and semantic content, bullet-redacted mounted content,
  disabled plaintext history/copy/cut, capability-gated paste availability, and snapshot-free typed
  outputs without invoking a clipboard or claiming protected storage/capture.
- [x] `ApplicationOverlayHost` binds exactly one neutral overlay lifecycle owner to one mounted,
  noninteractive full-view portal target, rejects duplicate/stale/cross-view mounting, delegates
  ordered modal/dismissal/focus effects unchanged, and closes owned entries explicitly on unmount.
- [x] `ApplicationOverlayController` owns that single host, routes typed open/dismiss/anchor/view/
  owner commands, derives inspection state from the host rather than copying it, and returns neutral
  focus/input/close effects without executing them or exposing mounted-UI contents in diagnostics.
- [x] `CommandSpec` owns reusable typed identity/metadata plus same-component controlled enabled and
  optional checked reads; its lazy owner-scoped `ActionFactory` creates a fresh moved action only
  for an enabled resolved invocation and preserves `ChangeSource` without requiring `A: Clone`.
- [x] The application command shortcut adapter resolves validated command-declared shortcut sets
  through the existing neutral matcher, atomically reads controlled availability, preserves scoped
  ambiguity/modal/repeat results, and reports display bindings separately without invoking actions.
- [x] The baseline command `Toolbar` reuses the neutral composite and shared `CommandSpec` owners,
  contributes one named focus stop with owned button-item semantics, includes disabled items for
  directional discovery without activation, and creates fresh source-preserving typed actions.
- [x] `MenuController` coordinates one explicit root-to-leaf chain over the existing application
  overlay and neutral composite owners, preserves top-first level/chain close ordering, rejects
  disabled activation, and returns focus, action, and caller-owned submenu scheduling intents.
- [x] The baseline `Menu` mounts one named composite focus entry and non-tab-stop `MenuItem` rows,
  publishes owned/active-descendant and controlled disabled/checked semantics, and routes command
  or submenu intent through the existing menu controller without duplicating its highlight state.
- [x] `ContextMenu` maps pointer, keyboard, and programmatic openings to neutral anchors, source-
  specific focus/restoration intent, one shared menu chain, and explicit level/chain dismissal
  without invoking a native platform menu service.
- [x] The focused Tier B `CommandPalette` model reads shared command snapshots, ranks bounded local
  label/description matches, skips disabled results during composite navigation, and closes its
  modal overlay before returning a fresh typed action; it remains outside the Tier A prelude.
- [x] `NavigationController` owns one nonempty typed route stack with unique optional restoration
  keys, validates push/replace/pop/selection atomically, and returns source-preserving transitions
  without mounting route content or calling URL, native-navigation, or platform-history services.
- [x] The baseline `Tabs` view reads selected route only from `NavigationController`, retains
  transient focused tab only in the neutral composite, exposes explicit automatic-local/manual
  activation, and mounts related Tab/TabPanel semantics without route content or keep-alive state.
- [x] `Breadcrumb` validates one labelled root-to-current trail against `NavigationController`,
  mounts ordered ancestor links plus a nonactivatable current item, excludes visual separators from
  semantics, and returns source-preserving ancestor proposals without owning route history.
- [x] `NavigationRail` reads selected route from `NavigationController`, delegates its single
  vertical focus entry to the neutral composite, skips disabled destinations, and returns
  source-preserving route proposals without owning history or route content.
- [x] `NavigationBar` reads the same controlled route owner, delegates one horizontal focus entry
  to the neutral composite, applies writing direction to Left/Right navigation, and returns
  source-preserving proposals without owning adaptive switching, history, or route content.
- [x] The focused Tier B `RouteHost` validates stable route-content registrations, derives an
  explicit count-and-byte-bounded inactive-content plan, exposes restoration identity, and mounts
  retained content hidden/inert without becoming another navigation or restoration-state owner.
- [x] `SelectionModel<K>` owns stable-key None/Single/Multiple selection and its anchor, returns
  revision-checked source-preserving proposals, prevents focus from collapsing multiple selection,
  and preserves surviving keys with deterministic anchor recovery across item updates.
- [x] The baseline `ListView<K>` validates named stable-key row descriptors, reports atomic
  insert/remove/move/label-update work, and mounts ordered density-aware ListItem semantics while
  leaving row selection, focus, actions, and business data to independent descendants/owners.
- [x] The baseline `VirtualListView<K>` adapts explicit viewport geometry and keyed extents through
  the neutral `VirtualCollection`, bounds overscan cache rows, distinguishes known/unknown totals,
  emits reveal intent, and mounts only planned density-aware ListItem rows and extent spacers.
- [x] The baseline `ListBox<K>` composes `SelectionModel<K>` with one vertical neutral composite,
  skips disabled options, keeps highlight distinct from selection, returns controlled proposals,
  and mounts one focus entry with density-aware ListBox/Option semantics.
- [x] The baseline noninteractive `Table<R, C>` validates stable rectangular row/column descriptors,
  addresses cells by key pair, and mounts density-aware Table/Row/Cell/header semantics with
  explicit row-and-column header relationships and no focus or actions.
- [x] The baseline `DataGrid<R, C>` composes that rectangular `Table` with `SelectionModel` and one
  neutral composite, keeps its active cell distinct from controlled selection, maps two-dimensional
  writing-direction-aware navigation, and mounts one focus entry with Grid/Row/Cell/header semantics.
- [x] The baseline `TreeView<K>` composes one validated canonical-preorder `TreeHierarchy`, one
  `SelectionModel`, and one vertical neutral composite, while keeping expansion, selection, and the
  active item distinct and mounting visible TreeItem hierarchy metadata under one focus entry.
- [x] The baseline `TreeGrid<R, C>` composes that hierarchy with one `DataGrid` cell owner, validates
  visible row identity/labels, maps disclosure-column open/descend/close/ascend behavior, and mounts
  one TreeGrid focus entry with hierarchical row plus tabular cell metadata.
- [x] `FieldMetadata<K>` owns stable field identity plus validated label/help, required, read-only,
  and enabled inputs without owning a value, layout, form order, focus, or submission policy.
- [x] `FieldValidation<K>` associates typed Valid/Warning/Invalid/Pending results with that exact
  stable identity; every non-valid result requires visible text, and field semantics use explicit
  Help/DescribedBy/ErrorMessage relationships plus invalid/busy state rather than color alone.
- [x] `Form<K>` owns one revisioned canonical unique field order and exact controlled validation
  snapshot, rejects incomplete/mismatched updates atomically, and returns acceptance metadata or
  first-invalid focus plus nearest-reveal intent without applying either effect.
- [x] `ValidationSummary<K>` derives canonical ordered non-valid entries from one form revision,
  mounts visible Alert/Status and actionable Link semantics, and returns source-preserving field-
  focus plus nearest-reveal intents without applying either effect.
- [x] `Scaffold` validates and canonicalizes stable navigation/top/content/secondary/status/floating-
  action/overlay slots, requires one primary content region, and mounts caller content under named
  application landmarks with direct ownership relationships and no routing or platform policy.
- [x] `AdaptiveScaffold` resolves validated environment snapshots into text-scale-adjusted compact,
  medium, or expanded plans with input-aware navigation and typed secondary presentations; mounted
  reconciliation reports changes against the same slot node identities without applying policy.
- [x] The focused `RangeSlider<T>` consumes one atomic controlled lower/upper pair, composes two
  shared slider behaviors under stable thumb identities, returns clamp-or-role-swap phased
  proposals, and mounts independent slider semantics and density targets without applying state.
- [x] The focused `SplitView` validates pane extent and both minimum sizes, retains one controlled
  restore position across collapse, composes shared slider resize behavior, and mounts stable named
  pane owners plus one density-aware Separator without applying layout or caller state changes.
- [x] `ScrollController` wraps exactly one neutral `ScrollState`, routes typed extent/offset/reveal/
  drag/motion/cancel commands, and returns unchanged consumed/unconsumed, activity, diagnostics,
  and scheduler-handoff records without owning layout, input, time, or background work.
- [x] `ScrollView` mounts one named stable scroll viewport and caller content owner over a
  `ScrollController` metrics snapshot, advertises only applicable forward/backward semantic actions,
  and returns viewport-sized typed controller commands without mutating offset or owning layout,
  input capture, motion scheduling, or transition state.
- [x] `ScrollBar` projects one controller snapshot into axis-specific thumb geometry, returns
  applicable line/page/bound/semantic/pointer-offset commands with unchanged source and boundary
  handoff, and mounts one named range-semantic control with stable density-aware track/thumb nodes
  without mutating the controller or owning capture, motion, or viewport layout.
- [x] `Separator` validates finite positive length and thickness, mounts one stable noninteractive
  horizontal or vertical line while preserving caller visual and layout inputs, explicitly excludes
  decorative instances, and exposes only named meaningful divisions as action-free semantics.
- [x] `ImageView` retains the caller's image ID and content version, mounts one stable noninteractive
  Image while preserving caller visual and layout inputs, explicitly excludes decorative instances,
  and exposes only nonempty described content as action-free Image semantics.
- [x] `Label` validates nonempty visible content and renderer-neutral text metrics, mounts one stable
  noninteractive Text with the caller's revision and complete visual/layout inputs, reuses visible
  content for action-free semantics, and exposes a generation-checked `LabelledBy` target.
- [x] `SelectableText` reuses label content/visuals, validates controlled selections at UTF-8 scalar
  boundaries, returns only source-preserving committed proposals, and mounts one focusable read-only
  Text semantic owner with Focus/SetSelection but no editing or clipboard execution.
- [x] `MenuButton` validates one nonempty keyed item snapshot, reuses canonical button activation
  and density, emits source-preserving root-menu requests anchored to its stable mounted identity,
  and reflects caller-published expanded state without owning menu or overlay lifecycle.
- [x] `ApplicationRoot` mounts one named Application semantic scope plus one stable merge-only
  caller-content host, preserving complete visual/layout inputs and explicit semantic ownership.
- [x] `ApplicationRegion` mounts named action-free Content, Navigation, or Status landmarks as
  stable caller-content hosts without importing Scaffold slot or layout policy.
- [x] Primitive `ApplicationUiExt<A>` conveniences mount those same root/region owners through the
  component runtime while preserving the application-to-runtime/UI dependency direction.
- [x] `HudLayer` validates host-logical or reference coordinates, keeps pointer hit and semantic
  participation policies independent, and mounts one stable route-free caller-content layer.
- [x] `ViewportOverlay` validates a host viewport plus normalized anchor and logical offset, mounts
  positioning separately from exact caller content inputs, and owns no viewport renderer.
- [x] `WorldAnchor` consumes only a validated host projection, visibility classification, and
  convention-free finite depth hint, reflecting hidden state without reading a camera or GPU.
- [x] `RenderTargetView` retains a nonzero opaque host token and content revision, mounts an honest
  noninteractive Box identity with explicit decorative/described semantics, and fabricates no image.
- [x] `VideoSurface` retains validated revision/frame size plus fit, color, and protection metadata
  without claiming decoding, import, playback, synchronization, or protection enforcement.
- [x] Application primitive diagnostics provide fixed typed saturating counters without retaining
  host tokens, descriptions, coordinates, media metadata, or an unbounded event log.
- [x] Shell output, surface, workspace, and application identities are distinct opaque nonzero
  host values with no native handle, protocol object, allocator, or application-domain dependency.
- [x] Shell capability grants are output-scoped host assertions that narrow only granted layer bits
  into typed layer authority while preserving canonical back-to-front layer ordering.
- [x] Shell output snapshots atomically retain validated logical/usable/physical geometry, scale,
  transform, safe insets, color capabilities, stable identity, and a nonzero host revision.
- [x] Client surface snapshots retain validated parentage, geometry, bounded logical regions and
  buffer damage, opaque content/synchronization references, color/protection, state, and capability.
- [x] Workspace snapshots retain a named revision and unique surfaces in exact host painter order,
  with explicit output-relative placements and no layout or membership policy.
- [x] Application entries retain bounded host labels/assets/state and unique typed action IDs with a
  referentially valid optional primary action, without process discovery or execution.
- [x] Notification snapshots retain bounded debug-redacted presentation, priority, privacy,
  lifecycle, and unique typed actions without delivery, persistence, dismissal, or service access.
- [x] System-status snapshots retain ordered bounded indicator/media/session/extension summaries
  with globally unique typed actions and debug-redacted labels/values.
- [x] Imported accessibility attachments retain only opaque identity, namespace/revision, validated
  affine mapping, distinct focus facts, and privacy metadata; they contain no fabricated tree/OCR.
- [x] Immediate shell request results distinguish accepted identity from denied, stale, and
  unsupported admission without claiming later platform completion or mutating a snapshot.
- [x] Client-input requests retain validated surface-local pointer/touch/pen lifecycle plus typed
  seat/contact/source identity without protocol serials, native grabs, or client dispatch.
- [x] Surface requests retain typed activate/close/move/resize/minimize/maximize/fullscreen intent
  and exact shell/surface capability requirements without applying optimistic state.
- [x] Workspace requests cite observed revisions for selection, membership movement, reorder, and
  removal while creation retains only validated name/order and fabricates no host identity.
- [x] Output requests retain revisioned reserved-area proposals/releases and opaque host appearance/
  mode actions with distinct reservation/configuration authority and no invented display policy.
- [x] System requests bind application, notification, and status action IDs to their exact parent
  identities/revisions and input source without invoking a service or applying optimistic state.
- [x] `ShellHost` transports one immutable bounded cross-model publication and five typed request
  families with immediate results, while deterministic trace fixtures prove category preservation.
- [x] Shell diagnostics use fixed saturating counters, and structured shell errors retain only a
  closed kind plus static redaction-safe context without event or content payloads.
- [x] `ShellRoot` mounts one named noninteractive output scope and owned content host while retaining
  one host grant that can narrow only its explicitly authorized layer kinds.
- [x] `OutputView` maps global host logical/usable/safe geometry into one output-local coordinate
  space while preserving output identity, revision, scale, transform, style, and noninteraction.
- [x] `ShellLayerOrder` permits omitted layers but mounts authorized layers only once in canonical
  back-to-front order for the same output, without applying lock/focus/input policy.
- [x] `ClientSurface` mounts one exact immutable surface revision only under an authorized workspace
  layer, retaining geometry/regions/opacity/color/protection/damage without importing an image.
- [x] `SurfaceTree` validates one bounded parent-before-child tree and mounts its exact host order
  without sorting, reparenting policy, protocol release, input forwarding, or renderer work.
- [x] `SurfacePlaceholder` presents typed unavailable/protected/lost state without retaining stale
  external content, while `SurfaceSnapshot` requires exact revision/grant/protection authorization.
- [x] `ReservedArea` binds proposal/release records to one authorized root and observed output
  revision without changing usable geometry or invoking the host.
- [x] `ExclusiveRegion` validates bounded output-logical geometry and returns explicit half-open
  lower-layer block/pass decisions without claiming focus, capture, or protocol authority.
- [x] `SurfaceInputRegion` retains one mounted surface/output/revision and maps finite output points
  into exact half-open surface-local input eligibility without constructing or forwarding events.
- [x] `DragRegion` and `ResizeRegion` validate bounded surface-local geometry and require matching
  output, shell grant, surface capability, contact, and hit before returning typed request intents.
- [x] `OutputEdgeRegion` derives stable output-local edge/corner geometry from one observed output
  revision and supports pointer/touch hits plus coordinate-free directional/accessibility intents.
- [x] Shell primitive diagnostics use fifteen fixed saturating counters without retaining identities,
  coordinates, contacts, content, errors, requests, or an unbounded log.
- [x] Primitive `ShellUiExt` methods delegate all current mountable shell owners without creating a
  second lifecycle, semantics, geometry, request, policy, or package-dependency owner.
- [x] The neutral popup solver validates finite anchor/content/safe/occlusion geometry, tries exact
  writing-direction-aware candidates first, subtracts occlusions, and applies only the explicit
  reject/shift/resize/scroll policy with deterministic typed placement results.
- [x] Managed pointer release labels completed dispatch as `ChangeSource::Pointer`; mounted
  component-route fixtures preserve explicit Pointer/Accessibility/Programmatic dispatch sources.
- [x] Invalid environment publications are atomic and unchanged publications produce no revision or
  dirty-aspect work.
- [x] Capability claims and the exact next eligible package are recorded in this status document.
- [ ] Mount environment aspects as dependency-tracked per-view reads after their component consumers
  exist.
- [ ] Route Gate 8 components through activation, focus, gesture, semantics, overlay, scroll, and
  environment owners.
- [ ] Bind button-family hover/press/focus/busy/enabled style and semantic changes, including the
  toggle's controlled `pressed` state, to dependency-tracked runtime updates when the standard
  application extension mounts ordinary child components.
- [ ] Host root components in the managed application assembly so raw input reaches the standard
  button route rather than only the compatibility application path.
- [ ] Translate native view/input/accessibility facts into these records only in Gate 9 adapters.

## Current implementation direction

Slice 5 visual coverage is implemented for text, ordinary images, clips, built-in materials,
scroll-resolved geometry, opacity, and the current compiler-owned interaction/focus visuals. Its
portable/reference suite and compile-only hardware checks pass. The Gallery hardware reference is
accepted at 1.506 mean channel error, a 0.0034 large-error ratio, and zero validation errors; the
focused mixed-scene hardware run also passes four draws at a maximum channel error of 3 with zero
validation errors. The post-Slice5 managed Windows regression passes six frames through resize,
suspend/resume, surface replacement, and shutdown with zero validation errors. Slice 5 is fully
qualified. Slice 6 hosted device/render-area mode is now implemented and E6-qualified: borrowed
native objects remain host-owned, command recording and resource retirement are separate, two views
share one device and record into separate subregions, the idle view path returns before GPU
allocation, Telorgon performs no hosted submission or command-buffer begin/end, both receipts retire
through host completion, and validation reports zero errors. Renderer Slice 7's same-device external
image path is E8-qualified: logical image IDs bind linear host-image leases, sampling uses the host
view directly with zero external pixel upload, command-only receipts return real binary acquire waits
and release signals, host completion retires the use generation, and validation reports zero errors.
The next Linux P4 subprofile is compile-complete for a narrow single-plane RGBA/BGRA DMA-BUF/modifier
and sync-FD contract. It exposes deterministic fourcc/format/modifier records for exact requested
usage, rejects invalid modifier sentinels and malformed plane/damage metadata before FD import,
imports a protocol acquire fence or exports implicit reservation fences for the Vulkan wait, performs
explicit foreign ownership transfer, and exports release synchronization once. Its ignored
Linux hardware fixture selects a jointly importable/exportable negotiated tuple rather than assuming
linear tiling. Linux execution is deliberately deferred and remains the qualification step. The existing
software renderer remains
operational for deterministic tests, headless use, temporary desktop tools, and the default managed
profile. Slice 2 has real offscreen device, command, shader, submission, and readback evidence;
Slice 3 has managed Windows presentation/recovery evidence; and Slice 4's device-local residency,
dirty uploads, ordered batching, descriptor reuse, budget enforcement, measured counters, reusable
frame slots, and post-rewrite E4/E5 validation now pass on developer hardware.
Epoch D has begun with three single-owner package moves. The deterministic software backend lives in
`telorgon-renderer-software`; `telorgon-render` no longer owns or exports that concrete backend, and the
Vulkan application profile does not select it. Neutral button/key state and input events now live in
`telorgon-input`; Linux/Wayland conversion methods no longer live in `telorgon-core`, current app input
queues and UI routes consume the neutral vocabulary, and Winit remains the native translator.
`telorgon-runtime` owns the current mount-once application adapter, normal root-component lifecycle,
nongeneric mounted UI, private generational component/state/action storage, atomic state
transactions, direct mounted-property bindings, application contexts/events, command queue, neutral
ancestry routing, and frame scheduler through one renderer/platform-free `ViewRuntime`;
`telorgon-app::AppRuntime` composes it with layout, text, scene compilation, and delta transport while
preserving the existing application facade. Gate 7 Slice 6's source/derived `Read<T>`, explicit
`map`/`zip`/conditional `select` dependency graph, iterative invalidation, inactive-edge
replacement, demanded bindings, post-commit observers, bounded action rounds, and cycle-path
diagnostics are operational in portable tests. Slice 7's first structural package is operational:
the runtime-owned component tree mounts conditional children and keyed homogeneous collections,
preserves component/node identity across moves and item updates, and tears children down in reverse
child-first order. Direct structural state is now validated before publication: duplicate collection
keys and missing switch branches reject the originating transaction, while a valid keyed switch
performs an explicit component-type replacement. Child observer and foundation-input actions now
use private generation-targeted type erasure: the child may handle its own action, while
conditional, keyed-collection, and keyed-switch boundaries can explicitly map, consume, or
command-route it without requiring `Clone`. Repeatable foundation button factories and neutral
listener maps are runtime-owned and removed with the component generation; an old branch node is
rejected after keyed replacement. A read-only staged preview now validates structural `map`, `zip`,
and `select` outputs before publication without changing live read caches, revisions, dependencies,
or diagnostics. Portals now retain logical component ownership while placing child roots under an
explicit visual host; host movement preserves component/node identity, routes use the same four
destinations, stale initial hosts fail construction, and teardown remains reverse child-first.
Slice 7 is complete: public `NodeBlueprint` insertion, replacement, and reconciliation have been
removed, while the old node-level keyed behavior remains a private test fixture. Gate 7 Slice 8's
first task-scope package is operational: component transactions stage local or worker-safe futures,
an explicitly injected host schedules them, completions enqueue generation-targeted actions for a
later bounded UI turn, and unmount cancels the scope. The second package adds cancellable handles,
bounded local/worker progress senders that return unsent actions on backpressure or closure,
coalesced cross-thread host wakes, stale-sender rejection, and explicit executor shutdown. Scheduler
deadline/timer action routing is now implemented without a sleeping thread: the host supplies
absolute monotonic instants, component transactions stage one-shot or repeating typed actions,
`FrameScheduler` exposes the earliest deadline, delayed intervals coalesce, and per-turn budgets,
cancellation wakes, generation closure, and diagnostics execute in portable tests. Managed
task-host wiring now completes Slice 8's executor package: `telorgon-app` has one separate managed
task-host implementation and a `ManagedComponentRuntime` wrapper over the existing `ViewRuntime`.
Local futures are owner-thread-only and fairness-budgeted, worker-safe futures use one explicitly
constructed named worker, the adapter reports that threading model through
`ManagedTaskCapabilities`, and cancellation/shutdown tests prove that capabilities close and the
worker is joined rather than detached. No executor dependency or logic was added to the monotelorgon
native host. Winit user-event integration waits for the component-aware managed assembly/Gate 9
extraction. Slice 9's first package is now complete: the former `crates/telorgon/src/text/system.rs` owner is
split into declaration-only exports plus focused error, resolved-style, shaping-engine, glyph,
atlas, retained-run, and cache modules without changing the retained glyph pipeline. The documented
`TextEngine` and `ResolvedTextStyle` names are primary, with deprecated aliases preserving the old
surface. Slice 9's second package now adds the opaque `TextBuffer`, immutable shared
`TextSnapshot`, `TextRevision`, scalar-validated UTF-8 offsets/ranges, directional selection and
affinity, and range-bounded chunks. Slice 9's third package now adds `edit.rs`: sorted,
nonoverlapping multi-edits cite their base revision; every old range and resulting selection/
composition is validated before mutation; success commits the complete editing value under one new
revision and returns precise old/new ranges plus a snapshot. Failures retain the prior text,
selection, composition, revision, and snapshots. Slice 9's fourth package now adds the explicit
neutral composition-state owner: revisioned start/update/commit/cancel commands validate active-
state ordering and delegate their text, selection, and composing-range changes to the atomic edit
engine. Invalid transitions, stale revisions, and invalid edits leave the complete editing value
unchanged; cancellation rollback is explicit edit data rather than hidden backup storage. Slice 9's
fifth package now adds `navigation.rs`: `unicode-segmentation` 1.13.2 is pinned to
Unicode 17.0.0 default UAX29-C1-1 extended-grapheme and UAX29-C2-1 word behavior; logical boundary,
word-range, and move/extend selection helpers reject invalid UTF-8 offsets, keep combining and emoji
ZWJ sequences indivisible, normalize platform scalar-range collapses, preserve anchor direction, and
take affinity explicitly rather than inventing visual bidi or wrapped-line behavior. Slice 9's sixth
package now adds `session.rs`: nonzero slot/generation identities guard an ordered
created/open/closed lifecycle; neutral input configuration, caret/selection geometry, and bounded
surrounding UTF-8 publish through redacted open/update/close requests; callbacks cite the session and
last issued text revision; valid edit/composition deltas delegate to the existing atomic owners;
wrong generations and closed sessions are rejected; stale or invalid callbacks return a typed full-
state resynchronization request without mutation; and semantic return-key actions do not mutate text.
Secure sessions publish no surrounding contents and none of the new debug paths expose text. This
completes Gate 7 Slice 9's neutral text foundation without implementing a platform adapter. Gate 8's
first ordered slice is the foundation behavior seam. Its first package adds
`crates/telorgon/src/input/activation.rs`: a pure transition owner retains
pointer capture while leave/re-enter toggles visual arming, completes only a matching primary
release, treats Space release, nonrepeat Enter, and semantic activation distinctly, and cancels
without action on outside release, pointer cancellation, capture loss, focus/view loss, disable,
unmount, or a claimed long/double/context/drag gesture. Outcomes carry typed change sources and
capture handoffs but never invoke component code, translate native keys, recognize gestures, or own
runtime capture. The second package now adds `crates/telorgon/src/input/focus.rs`: generational scope IDs and
generic generational target keys keep identity with the mounted/runtime owner; atomic candidate
updates consume canonical order; forward/reverse traversal skips ineligible targets and obeys
explicit stop/wrap edges; nested scopes restore the exact surviving parent target and indicator
state; removal selects an old surviving successor then predecessor but never a new recycled
generation; and pointer/keyboard/directional/accessibility modality plus an always-visible preference
drive focus indicators. Rejections do not mutate focus and deterministic move/restore/failure
counters are exposed for later view aggregation. The third package now adds
`crates/telorgon/src/input/composite.rs`: keyed canonical item updates are atomic; transient active-descendant
history stays separate from caller-controlled selection and produces typed selection requests;
re-entry restores the last valid key, then the selected key, then the first enabled key; directional,
Home/End, stop/wrap, orientation, and RTL policies execute in portable fixtures; disabled items are
either skipped or discoverable but never selectable; and removal chooses an old surviving successor,
then predecessor, then the composite root without targeting a recycled generation. The next eligible
package in the same foundation slice is the arena, cancellation, drag, long-press, and tap transition
engine in `crates/telorgon/src/input/gesture.rs`. The fourth package now adds that owner: per-pointer arenas
accept ordered participants, eager/last/swept winners notify every remaining participant exactly
once, holds defer sweeps, and cancellation rejects the unresolved set. Tap recognition waits for
pointer-up and arena victory while enforcing slop; long press emits generation-aware schedule/cancel
requests and cannot recognize before its caller-delivered deadline; drag claims only after explicit
axis slop and then reports begin/update/end deltas. Arena loss, pointer/capture loss, view loss,
disable, and unmount cancel without a later recognition, and no recognizer owns timers, capture,
native conversion, or callbacks. The fifth package now adds `crates/telorgon/src/input/shortcut.rs`: exact
physical chords match controlled registration snapshots through generational innermost-to-outermost
scopes; disabled and repeat-suppressed entries are skipped; scope proximity precedes explicit
per-scope priority; modal scopes block ancestor lookup; and equal-priority winners report
deterministic ambiguity. The owner returns typed binding and command IDs but does not execute
actions, translate native keys, or invent localized/platform shortcut policy. With the neutral input
owners complete, the sixth package now adds `crates/telorgon/src/ui/semantics.rs`: mounted records keep role,
name source, description, controlled state, text/range value, advertised actions, tree participation,
generational relationships, and virtual collection metadata separate from later tree and platform
owners. Local invariants and live relationship targets are validated before atomic replacement;
semantic revisions, dirty work, memory, and success/failure counters update only after accepted
changes. Disabled, inert, hidden, and excluded nodes expose no effective action set, but this owner
never dispatches an action or constructs a platform accessibility object. The seventh package now
adds `crates/telorgon/src/ui/overlay.rs`: one pure host maintains bottom-to-top entries with generational IDs,
validated anchors and parentage, modal barrier state, explicit outside-input disposition, and focus
lifecycle records. Policy-gated dismissal is nonmutating when blocked; accepted parent closure emits
descendants top-first; anchor removal is forced; released slots reject stale generations; and focus
and input effects are returned for their named owners rather than executed inline. Existing runtime
portals remain the only visual-placement owner. The eighth package now adds
`crates/telorgon/src/layout/scroll.rs`: one pure two-dimensional state owner validates extent and reveal
geometry before mutation, reports exact consumed/unconsumed deltas at clamped boundaries, supports
per-axis end-distance anchoring, leaves already-satisfied nearest reveals undisturbed, and exposes
drag/cancel/reduced-motion plus caller-timed ballistic transitions through generational motion IDs.
It starts no timer or task and performs no input routing, layout mutation, or rendering. The ninth
package now creates `telorgon-primitives-application` and adds
`crates/telorgon/src/application_primitives/environment.rs`: validated logical geometry/constraints, device
and text scale, logical density class, locale/direction/reading order, safe-area and occlusion data,
simultaneous input capability sets, accessibility/color/focus preferences, and active/focused/
visible state publish atomically under immutable revisions. Accepted updates report exact dirty
aspect groups, unchanged updates do no revision work, rejected values preserve the prior snapshot,
and the crate depends only on `telorgon-core` and `telorgon-input`. It performs no platform detection,
native conversion, runtime read mounting, component styling, or shell work. This completes Gate 8
Slice 1's foundation behavior seam. Gate 8 Slice 2 now begins with the tenth package: the new
`telorgon-components-application` crate gives `change.rs` sole ownership of `ChangePhase` and
controlled `ValueChange<T>` proposals while re-exporting the canonical input-owned `Activation` and
`ChangeSource`; `density.rs` reuses the environment-owned logical density class and resolves exact
24/32/44 Compact/Standard/Touch baselines against optional theme and stricter accessibility/platform
floors. Target assessment rejects invalid geometry and reports undersized hit bounds without
requiring visible artwork to fill them. The crate has a curated current-value prelude and depends
only on its matching primitive package plus shared foundations. The eleventh Gate 8 package now
adds the Tier A button in `crates/telorgon/src/application_components/action/button.rs`. It reuses the
input-owned activation machine, keeps focus-visible and hover inputs externally owned, defines an
explicit suppress/allow busy policy, resolves typed visual slots in
disabled/busy/pressed/focused/hovered/resting priority, attaches a validated button semantic node
and density floor during real runtime mounting, and routes completed activations without erasing
their neutral source. The runtime's legacy source-free button entry remains compatible, while the
managed pointer route now identifies pointer activation explicitly. This does not claim
dependency-tracked live restyling/semantic patching, keyboard default routing,
touch/directional/accessibility adapters, the application extension trait, a gallery specimen, or
any sibling control. The twelfth Gate 8 package now adds the Tier A icon button in
`crates/telorgon/src/application_components/action/icon_button.rs`. It wraps foundation `ImageId` in
application-domain `IconArtwork` without giving paint semantic meaning, requires an explicit
nonempty accessible name at construction, reuses the button behavior/busy/semantic/density/action
contract and its single state-priority owner, validates icon logical size and opacity, and mounts
the image as one semantically empty child under the named button root. Direct fixtures demonstrate
shared pointer activation, missing-name rejection, style priority/fallback, invalid icon-slot
rejection, 44-pixel Touch hit geometry around 18-pixel artwork, accessibility-source preservation,
and mounted busy suppression. This does not add an icon registry, resource loader, tint pipeline,
tooltip, dynamic state binding, gallery specimen, or sibling control. The exact next eligible
Slice 2 package is `crates/telorgon/src/application_components/action/toggle_button.rs`: it must reuse the
button-family activation and focus/density contracts, accept a controlled boolean value, emit only
the requested inverse with its activation source, expose button semantics with controlled
`pressed`, define a typed toggle style contract, and land headless controlled-value/semantic/mount
fixtures without adding choice or range controls in the same change.
The thirteenth Gate 8 package now adds the Tier A toggle button in
`crates/telorgon/src/application_components/action/toggle_button.rs`. A narrow runtime read-aware activation
route evaluates the latest validated `Read<T>` before mapping an action, so the toggle emits
`ValueChange<bool>::Commit` for the inverse of the current parent value without owning a write.
The component reuses button-family behavior, busy/focus/density/state priority, preserves its stable
label, exposes semantic `pressed`, resolves separate typed off/on button styles, and returns a
focused reference retaining the controlled read. Fixtures prove repeated rejected requests do not
drift, an accepted parent publication changes the next requested inverse, sources remain exact,
Touch density mounts at 44 pixels, and the on/off style dimension resolves before interaction
priority. Initial semantics/style are mounted from the controlled value; dependency-tracked live
semantic/style patching remains open and is not claimed. The exact next eligible Slice 2 package is
`crates/telorgon/src/application_components/action/link.rs`: it must define a typed destination/action
contract, reuse neutral activation and accessible naming, expose link role plus destination meaning
without performing navigation inline, define typed link styles, and land context-operation and
mounted/headless fixtures without adding choice or range controls in the same change.
The fourteenth Gate 8 package now adds the Tier A link action in
`crates/telorgon/src/application_components/action/link.rs`. `LinkDestination` validates a nonempty,
control-character-free opaque destination without pretending to parse or authorize a URI, route,
or file path. Canonical completed activation produces a source-preserving `LinkAction`; copy and
open-in-new-context remain separate `LinkCommand` values for an application owner to execute.
Mounting reuses button-family activation, density, accessible naming, and state priority while
installing link role and interned destination value semantics. Fixtures demonstrate destination and
name rejection, pointer cancellation, typed context commands, state-style priority, 44-pixel Touch
mounting, semantic name/destination/context capability, and exact
Pointer/Accessibility/Programmatic sources. The component does not navigate, touch a clipboard,
call a platform service, parse destinations, mount a context menu, dynamically patch styles, or add
choice/range controls. The exact next eligible Slice 2 package is
`crates/telorgon/src/application_components/choice/check_state.rs`: it must give `CheckState` and explicit
two-state/tri-state cycle policy one owner, guarantee that a two-state cycle never produces
`Mixed`, and land portable cycle-policy fixtures without mounting the checkbox early.
The fifteenth Gate 8 package now adds the choice-value prerequisite in
`crates/telorgon/src/application_components/choice/check_state.rs`. `CheckState` is the single
`Unchecked`/`Checked`/`Mixed` value owner. `CheckCyclePolicy::two_state` toggles the two binary
values, rejects incompatible `Mixed` input, and cannot produce `Mixed`; `tri_state` requires a
caller-ordered permutation containing all three states exactly once, so Telorgon does not invent
either transition around `Mixed`. Direct portable fixtures prove binary output closure, mixed-input
rejection, duplicate tri-state rejection, both mixed-transition choices, complete visitation, and
wraparound. This package owns no component state, parent write, semantic mapping, style, mounting,
or input route. The exact next eligible Slice 2 package is
`crates/telorgon/src/application_components/choice/checkbox.rs`: it must consume `Read<CheckState>` plus the
explicit cycle policy, reuse canonical activation/density/accessibility naming, emit only a
source-preserving committed proposal from the latest controlled value, expose checkbox
checked/mixed semantics, define typed checkbox styles, and land mounted/headless controlled-value
fixtures without adding radio or switch controls.
The sixteenth Gate 8 package now adds the Tier A checkbox in
`crates/telorgon/src/application_components/choice/checkbox.rs`. It consumes `Read<CheckState>` and a
validated `CheckCyclePolicy`, reuses the button-family activation/name/density/state-priority owners,
maps all three values to checkbox checked/mixed semantics, mounts a typed indicator/mark/label style,
and emits only `ValueChange<CheckState>::Commit` proposals derived from the latest parent value.
The runtime's narrow read-aware activation seam now also supports fallible derivation, allowing a
later binary `Mixed` publication to record an error and emit no action rather than panic, guess, or
silently retain a fake value. Fixtures prove initial name/cycle rejection, canonical pointer
cancellation, per-value style/semantic mapping, repeated rejected proposals without drift,
tri-state publication followed by latest-value derivation, exact Pointer/Accessibility/Programmatic
sources, 44-pixel Touch mounting, and live incompatible-value rejection. Initial checked semantics,
mark content, and styles remain mount-time snapshots until dependency-tracked component patching is
implemented. The exact next eligible Slice 2 package is
`crates/telorgon/src/application_components/choice/radio.rs`: it must define keyed radio-group/item inputs,
reuse the neutral composite owner for one tab stop and directional active-descendant movement,
separate focus from parent-controlled `Read<Option<K>>` selection, emit source-preserving selection
proposals only for enabled items, expose group/item selected semantics, define typed styles, and
land reorder/removal/disabled/mounted fixtures without adding the switch.
The seventeenth Gate 8 package now adds keyed Tier A radio groups/items in
`crates/telorgon/src/application_components/choice/radio.rs`. `RadioGroupBehavior<K>` delegates canonical
order, entry, active-descendant history, disabled skipping, wrapping directional navigation,
selection-following proposals, and survivor-only removal recovery to the input-owned
`CompositeStateMachine<K>`. The component keeps `Read<Option<K>>` selection parent-owned, mounts one
focusable group root plus non-tab-stop item action nodes, validates names and duplicate keys, emits
source-preserving `ValueChange<Option<K>>::Commit` proposals only from enabled items, and attaches
named group/item selected semantics with owns/active-descendant relationships. A focused foundation
action-node constructor expresses explicit tab-stop participation, and the existing fallible action
route handles group activation without moving composite policy into runtime storage. Fixtures prove
duplicate/name rejection, disabled skipping, directional selection source, independent controlled
selection, reorder retention, successor recovery, one tab stop, selected/disabled semantics,
44-pixel Touch items, and Pointer/Accessibility/Programmatic activation sources. Mounted raw-key
translation and dependency-tracked active-descendant/selected/style semantic patches remain open;
the returned focused ref exposes the neutral transition result but does not commit it. The exact
next eligible Slice 2 package is `crates/telorgon/src/application_components/choice/switch.rs`: it must
consume `Read<bool>`, reuse canonical tap/Space activation and density/naming behavior, emit only a
source-preserving committed inverse from the latest value, expose switch checked semantics, define
typed off/on switch styles, and land controlled/mounted fixtures without starting range controls.
The eighteenth Gate 8 package now adds the Tier A switch in
`crates/telorgon/src/application_components/choice/switch.rs`. It consumes a parent-owned `Read<bool>`,
reuses the button-family activation/name/density/state-priority owners, exposes switch role plus
checked/unchecked semantics, resolves typed off/on track/thumb/label styles, and emits only a
source-preserving `ValueChange<bool>::Commit` inverse derived from the latest parent value. Fixtures
prove missing-name rejection, pointer cancellation, Space activation, deterministic off/on style
resolution, disabled action suppression, 44-pixel Touch mounting, and exact
Pointer/Accessibility/Programmatic sources after controlled publication. The optional drag
enhancement is not implemented, and initial checked semantics, thumb position, and styles remain
mount-time snapshots until dependency-tracked component patching exists. The exact next eligible
Slice 2 package is `crates/telorgon/src/application_components/range/model.rs`: it must give finite ordered
range bounds, positive step/page-step validation, clamping/step normalization, formatting, and
optional validated marks one application-domain owner, with portable boundary/error fixtures and
without mounting the slider early.
The nineteenth Gate 8 package now adds the range-value prerequisite in
`crates/telorgon/src/application_components/range/model.rs`. `RangeModel<T>` is the single owner for finite
ordered minimum/maximum values, positive step and page-step values, deterministic typed formatting,
and optional finite, bounded, strictly increasing labelled marks. `RangeScalar` supplies the narrow
conversion contract with built-in `f32` and `f64` implementations; clamping preserves authored
bounds, while normalization chooses the nearest step or explicit endpoint without hiding nonfinite
inputs. Formatting rejects out-of-range values instead of silently changing them. Portable fixtures
prove invalid numeric inputs, both scalar implementations, nonzero-bound normalization, endpoint
reachability, precision/affix validation, deterministic negative-zero formatting, and mark
validation. This package owns no `Read<T>`, component state, input transition, semantic node, style,
mount, or renderer dependency. The exact next eligible Slice 2 package is
`crates/telorgon/src/application_components/range/slider.rs`: it must consume `Read<T>` plus `RangeModel<T>`,
define orientation/writing-direction/reversal-aware step, page, Home/End, and cancellable phased
pointer behavior, emit source-preserving `Begin`/`Update`/`Commit`/`Cancel` proposals without owning
the value, expose named slider range semantics and typed density-aware styles, and land portable and
mounted fixtures without adding range-slider or progress controls.
The twentieth Gate 8 package now adds the controlled Tier A slider in
`crates/telorgon/src/application_components/range/slider.rs`. `SliderBehavior<T>` keeps the parent-owned
value separate from transient drag protocol state, delegates slop/cancellation/arena ownership to
the input-owned `DragRecognizer`, and exposes arena requests alongside optional typed proposals.
Arrow movement follows orientation, writing direction, and explicit reversal; page, Home/End, and
semantic increment/decrement requests use the range model's discrete movement and emit no duplicate
bound request. Pointer sequences emit `Begin` only after arena win, changed `Update` values only,
then `Commit`, or the starting value as `Cancel`, all with the exact source. The component consumes
`Read<T>`, requires an accessible name, validates the current value, mounts deterministic typed
track/fill/thumb/label visuals at the density floor, and exposes slider current/minimum/maximum/step/
formatted-text semantics with disabled action suppression. Fixtures prove LTR/RTL, horizontal/
vertical/reversed commands, pages/bounds, accessibility and directional sources, arena handoff,
phase ordering, duplicate suppression, cancellation restoration, disabled behavior, style priority,
numeric semantics, and 44-pixel Touch mounting. Mounted raw input and semantic-action adapters,
dependency-tracked value/style/semantic patching, tap-to-position policy, and gallery specimens
remain open; the focused ref returns proposals for the owning component to map and never writes the
controlled value. The exact next eligible Slice 2 package is
`crates/telorgon/src/application_components/range/progress.rs`: it must define determinate bounded and
indeterminate progress inputs without fabricating a percentage, expose progress/busy semantics,
define typed density-aware determinate/indeterminate styles, and land controlled/mounted fixtures
without adding activity-indicator, meter, or range-slider controls.
The twenty-first Gate 8 package now adds the read-only Tier A progress indicator in
`crates/telorgon/src/application_components/range/progress.rs`. `ProgressValue<T>` keeps determinate values
and indeterminate mode parent-owned through one `Read`, while `ProgressIndicator<T>` validates
determinate values against the shared range model and never emits an action. Determinate semantics
contain current/minimum/maximum/step plus the model-formatted value text; indeterminate semantics
set busy and leave the semantic value empty, so no percentage is fabricated. `ProgressStyle`
defines separate determinate/indeterminate visuals for each Compact/Standard/Touch density rather
than applying an interactive hit-target floor to a read-only node. A focused hidden foundation
container-under-host constructor mounts that node without button identity or tab-stop participation.
Fixtures prove density/mode style selection, required naming, bounded formatted numeric semantics,
indeterminate busy-without-number semantics, zero effective actions/focus, and rejection of an
out-of-range controlled value before semantic attachment. Mounted values, semantics, and visuals
remain mount-time snapshots; dependency-tracked high-frequency coalescing, live-announcement rate
policy, progress animation/reduced-motion behavior, and gallery specimens remain open. The exact next
eligible Slice 2 package is the activity-indicator companion in
`crates/telorgon/src/application_components/range/progress.rs`: it must consume controlled active state,
report busy without any numeric semantic value, define deterministic running/inactive and
reduced-motion-capable density styles without owning a clock, and land portable/mounted fixtures
without adding the meter or range slider.
The twenty-second Gate 8 package now adds the read-only Tier A activity-indicator companion in that
same `range/progress.rs` owner. `ActivityIndicator` consumes a parent-owned `Read<bool>`, requires an
accessible name, never emits an action, and always exposes `SemanticValue::None`; only active input
sets semantic busy. `ActivityIndicatorStyle` resolves running/inactive and standard/reduced-motion
variants independently for Compact/Standard/Touch density. Standard running style exposes a typed
rotation intent to the scheduling owner, while reduced-motion and inactive styles are static; the
component creates no clock, timer, task, or scheduler. Its focused reference identifies the mounted
track and marker plus the resolved declarative motion intent. Fixtures prove required naming,
state/density/motion resolution, Touch geometry, active busy-without-number semantics, inactive
non-busy semantics, static reduced-motion intent, and zero actions/focus. Values, semantics, and
visuals remain mount-time snapshots, and no animation scheduler or gallery specimen is claimed. The
exact next eligible Slice 2 package is `crates/telorgon/src/application_components/range/meter.rs`: it must
consume a parent-controlled bounded value through the shared range model, define validated typed
meter bands and Compact/Standard/Touch styles, expose formatted read-only numeric semantics, and land
portable/mounted fixtures without adding the range slider or another component family.
The twenty-third Gate 8 package now adds the read-only Tier A meter in
`crates/telorgon/src/application_components/range/meter.rs`. `MeterBands<T>` rejects empty, nonfinite,
out-of-range, non-increasing, incomplete, and range-model-mismatched band sets before use; inclusive
upper bounds deterministically select typed Neutral/Positive/Caution/Critical levels. `Meter<T>`
consumes a parent-owned `Read<T>` and shared `RangeModel<T>`, validates and formats the controlled
value, never emits an action, and mounts the selected typed level color through explicit
Compact/Standard/Touch styles. `telorgon-ui` remains the sole semantic vocabulary owner and now adds a
named `Meter` role; the mounted record contains current/minimum/maximum/step/formatted text and no
effective action or focus. Fixtures prove band rejection and boundary selection, density/level style
resolution, required naming and model matching, 75-percent Touch geometry, formatted numeric meter
semantics, and rejection of out-of-range controlled input before semantic attachment. Mounted value,
semantics, and visuals remain mount-time snapshots, and no live update, announcement, or gallery
integration is claimed. This completes the Tier A range/status family. The exact next eligible Slice
2 package is `crates/telorgon/src/application_components/text/controller.rs`: it must wrap the existing
revisioned `telorgon-text` buffer, selection, composition, and session owners as one application-domain
editing controller, expose revision-checked typed edits and outputs without duplicating the text
engine, and land portable controller fixtures without adding edit history or field components.
The twenty-fourth Gate 8 package now adds the application-domain `TextController` in
`crates/telorgon/src/application_components/text/controller.rs`. The local non-`Send` controller owns exactly
one neutral `TextBuffer` and at most one generational `TextInputSession`; all UTF-8 range checks,
revision increments, atomic edits, selection/composition state, immutable snapshots, surrounding
text, secure redaction, and session resynchronization remain delegated to `telorgon-text`.
Revision-checked direct edits and explicit whole-text synchronization publish typed `TextChanged`,
`SelectionChanged`, and `CompositionChanged` records. Session deltas publish either an accepted
update plus neutral host request, a separate typed `Submitted` action, or `EditRejected` with the
unchanged redacted snapshot and optional resynchronization request. The controller admits only one
open session, exposes configuration/geometry updates and explicit close, and contains no native IME,
clipboard, widget, mounting, background task, or persistence behavior. Fixtures prove immutable old
snapshots, explicit programmatic revision checks, atomic stale/invalid UTF-8 rejection, directional
selection and composition transitions, generational session mismatch, accepted session edits,
return-action separation, stale-session resynchronization, and debug redaction. Runtime-owned
controller handles/reads, controller-owned optional history, field components, command availability,
and platform adapters remain open. The exact next eligible Slice 3 package is
`crates/telorgon/src/application_components/text/edit_history.rs`: it must define a bounded application edit
history policy and deterministic undo/redo grouping transitions over revision-citing neutral edit
records, without duplicating buffer mutation or adding a text field in the same package.
The twenty-fifth Gate 8 package now adds bounded deterministic application edit history in
`crates/telorgon/src/application_components/text/edit_history.rs`. `EditHistoryPolicy` validates positive
unit and retained-byte budgets and accepts a caller-owned monotonic merge deadline. The history
keeps paste, cut, drop, replacement, committed composition, and programmatic replacement as separate
units; only adjacent matching typing or directional deletion may merge, and explicit or
selection-only boundaries close that group. Undo and redo verify current controller continuity,
reject active composition or divergent text without moving either stack, restore the edit's
selection, and delegate the actual revisioned whole-buffer mutation to `TextController`. New edits
after undo clear redo, discontinuous histories reset safely, and over-budget edits retain no
plaintext history. Fixtures prove grouping/deadline/boundary behavior, selection-only suppression,
separate edit kinds, oldest-first pruning, oversized reset including an oversized merged unit,
redo invalidation, explicit reset, divergence rejection, active-composition rejection, and
undo/redo restoration. The store intentionally retains plaintext for ordinary fields; secure fields
must disable it until protected retention exists. Controller-owned optional history/availability
reads, mounted fields, clipboard/platform commands, rich-document transactions, multi-cursor, and
collaborative history remain open. The exact next eligible Slice 3 package is the focused
`crates/telorgon/src/application_components/text/controller.rs` history integration: it must let the
controller optionally own the bounded history, record accepted direct/session edits with explicit
edit kinds and caller-supplied monotonic times, and expose typed undo/redo availability and commands
without adding a text field, clock, native IME, or clipboard service.
The twenty-sixth Gate 8 package now integrates optional bounded history into the application
`TextController`. Ordinary controllers may enable one `EditHistoryPolicy`; recorded direct and
session edit paths require an explicit `EditHistoryKind` plus caller-supplied `MonotonicInstant`,
validate timestamp/composition preconditions before mutation, and automatically update the owned
undo/redo stacks after acceptance. Legacy untracked mutation paths remain available but clear
history after text changes or close compatible grouping after selection/composition changes, so
they cannot leave a stale undo chain. Recorded composition retains one redacted-debug origin
snapshot from start through update and publishes only the committed result as one undo unit; cancel
retains history only when it restores the origin text. Session open/close and submission close merge
groups. `EditHistoryAvailability` reports enabled/undo/redo state and `EditHistoryCommand` executes
typed traversal through the controller's existing revisioned mutation path. Entering or configuring
a secure input session drops owned plaintext history and prevents re-enabling it until secure entry
ends. Fixtures prove direct merging and traversal, availability transitions, pre-mutation rejection
of backward timestamps, safe invalidation by untracked edits, selection boundaries, recorded session
edits with rejected-delta preservation, one-unit committed composition, and secure-session disposal.
Runtime-owned controller handles/reads, mounted fields, clipboard/platform commands, rich-document
transactions, multi-cursor, and collaborative history remain open. The exact next eligible Slice 3
package is `crates/telorgon/src/application_components/text/field.rs`: it must define the basic single-line
field's validated label/mode configuration, renderer-free text semantics, and typed
edit/selection/submit/history-command boundary over one `TextController`, with portable and mounted
fixtures, without adding text-area, search, native IME, or clipboard-service implementations.
The twenty-seventh Gate 8 package now adds the basic single-line application `TextField` in
`crates/telorgon/src/application_components/text/field.rs`. One field owns exactly one `TextController` plus a
nonempty label and explicit Editable/ReadOnly/Disabled/Secure mode. Its command boundary routes
revisioned edit batches, directional selection, submission, and typed history traversal; rejects
newline edits and newline return actions before mutation; preserves selection in read-only mode;
suppresses every command while disabled; and keeps submission separate from text mutation. Optional
controller history is used when enabled for ordinary edits, while secure mode discards history and
uses only unretained edits. Mode-derived availability reports edit/select/submit/undo/redo without
calling a platform service. Renderer-neutral semantics expose the `TextInput` role, explicit name,
required/invalid/read-only/disabled/focus state, SetText/SetSelection capabilities, and ordinary
text value; secure semantics contain no value. Compact/Standard/Touch typed visuals mount through a
real neutral `NodeKind::TextInput` foundation node, with the Touch field enforcing a 44-logical-pixel
minimum and secure display containing only bullets. Custom field/command debug paths redact edit
contents. Fixtures prove validation, mode/action/value semantics, atomic multiline rejection,
read-only selection, disabled suppression, history undo/redo availability, separate submission,
mounted identity/semantics/Touch geometry, and secure mounted redaction. Mounted values and
semantics remain mount-time snapshots; raw pointer/key/semantic-action routing, caret/selection
painting and reveal, runtime-owned controller handles, neutral session attachment, native IME,
clipboard/context services, help/error relationship nodes, text transformation, and filtering remain
open. The exact next eligible Slice 3 package is
`crates/telorgon/src/application_components/text/area.rs`: it must define the multiline field companion over
one `TextController`, including explicit newline-versus-submit policy, multiline semantics and typed
command routing, and portable/mounted fixtures, without adding search, numeric, secure-policy
extensions, native IME, or clipboard services.
The twenty-eighth Gate 8 package now adds the multiline application `TextArea` in
`crates/telorgon/src/application_components/text/area.rs`. It composes the existing `TextField`, leaving that
owner solely responsible for controller storage, mode checks, bounded history, secure redaction,
shared styles, and neutral text-input mounting. Area commands accept ordinary multiline edit batches,
selection/history operations, and an explicit Return request. The default Return policy validates
and applies a caller-supplied revisioned newline batch; a configured non-newline submission action
instead publishes `Submitted` without mutating text. Its neutral input configuration and semantic
state both identify multiline editing, while a validated two-or-more visible-line floor defaults to
three lines. Command debug output remains content-redacted. Portable and mounted fixtures prove
invalid-policy rejection, multiline edit/Return acceptance, mutation-free submit Return, input
configuration, availability, semantics, neutral `TextInput` identity, and multiline height. Mounted
values and semantics remain mount-time snapshots; raw pointer/key/semantic-action routing,
caret/selection painting and reveal, runtime-owned controller handles, neutral session attachment,
native IME, clipboard/context services, help/error relationship nodes, text transformation, and
filtering remain open. The exact next eligible Slice 3 package is
`crates/telorgon/src/application_components/text/search.rs`: it must compose the basic field with typed clear
and Search submission behavior plus portable/mounted fixtures, without adding numeric parsing,
secure-policy extensions, native IME, clipboard services, or platform navigation.
The twenty-ninth Gate 8 package now adds `SearchField` in
`crates/telorgon/src/application_components/text/search.rs`. It composes the existing single-line
`TextField`, configuring only Search input purpose, Search return action, `SearchBox` semantics, and
typed clear behavior. Ordinary edit, selection, history, mode, security, style, and mounting policy
remain delegated to the basic field. `SearchFieldCommand::Clear` cites the source revision, rejects
stale or already-empty requests without mutation, replaces nonempty content through the controller's
normal edit path, records as an undoable replacement when bounded history is enabled, and publishes a
distinct `Cleared` result. Submit publishes Search-flavored `Submitted` at the current revision
without changing text. Content-derived availability adds `can_clear`; secure diagnostics, mounted
display, and semantics retain the field's redaction. Portable and mounted fixtures prove Search
configuration/role, clear availability and undo, stale/empty/newline rejection, mutation-free Search
submission, Touch geometry, neutral `TextInput` identity, and secure redaction. A mounted clear
button, dynamic mounted updates, raw input/action routing, native IME, clipboard/context services,
search execution/results, and platform navigation remain open. The exact next eligible Slice 3
package is `crates/telorgon/src/application_components/text/numeric.rs`: it must compose the basic field with
the numeric parsing/formatting contract and typed valid/intermediate/invalid states plus
portable/mounted fixtures, without adding secure-policy extensions, native IME, clipboard services,
or platform locale adapters.
The thirtieth Gate 8 package now adds generic `NumericField<T>` in
`crates/telorgon/src/application_components/text/numeric.rs` for `f32`, `f64`, and explicitly implemented
`NumericFieldScalar` values. It composes the basic single-line field with a locale-neutral ASCII
decimal grammar and the existing generic `RangeModel<T>` constraint/format owner. Empty text, signs,
decimal points, trailing decimal points such as `1.`, and incomplete exponents remain typed
`NumericIntermediate` states; malformed, nonfinite, unrepresentable, and out-of-bounds values remain
distinct `NumericInvalid` states. Accepted edits and history traversal publish the recomputed state
without rewriting the controller text. Commit is mutation-free and publishes a typed numeric value
plus deterministic formatted text only for a complete finite value accepted by the model; otherwise
it publishes typed `CommitRejected`. Decimal input purpose, content-derived commit availability,
numeric min/max/step/value semantics, semantic invalid state, and mounted error state are explicit.
Secure mode is rejected because exposing parsed numeric state would violate the existing secure
redaction baseline; secure policy remains with the next focused package. Fixtures prove intermediate
grammar, both scalar implementations, syntax/nonfinite/constraint distinctions, nonrewriting edits,
valid and rejected commit, history recomputation, secure rejection, numeric/invalid mounted semantics,
Touch geometry, and public compile paths. Locale-specific separators/grouping, platform locale
parsers, spinner/stepper affordances, automatic normalization, dynamic mounted updates, raw input
routing, native IME, and clipboard services remain open. The exact next eligible Slice 3 package is
`crates/telorgon/src/application_components/text/secure.rs`: it must define the dedicated secure-field policy
surface over the basic field's secure baseline, including explicit diagnostics/semantics/capture and
history guarantees plus portable/mounted fixtures, without adding native credential storage,
platform autofill, IME, clipboard services, or protected-memory claims not actually demonstrated.
The thirty-first Gate 8 package now adds `SecureField` in
`crates/telorgon/src/application_components/text/secure.rs`. It wraps one basic field forced into Secure mode
and fixes an inspectable baseline policy: diagnostic and semantic content are omitted, mounted
visual content is bullet-redacted, plaintext edit history is disabled, and copy/cut are unavailable.
Paste availability is derived from an explicit external plain-text-read capability without invoking
a clipboard service. Secure command outputs publish only revision and change flags rather than a
plaintext-bearing snapshot or changed ranges, and the command vocabulary structurally omits history
traversal. Edit-command debug formatting reports only the base revision and edit count. The secure
input configuration disables correction and spelling; submission carries only revision and return
action. Portable and mounted fixtures prove construction-time history disposal, redacted command and
output diagnostics, capability-gated context availability, omitted semantic values, bullet-only
mounted content, Touch geometry, and public compile paths. Bullet redaction still exposes character
count and does not prevent operating-system screenshots. Protected memory, native credential
storage, autofill, native IME, clipboard execution, platform capture prevention, and dynamic mounted
updates remain open. This completes the dedicated text-component sequence in Slice 3. The exact next
eligible Slice 3 package is `crates/telorgon/src/application_components/overlay/host.rs`: it must provide the
application-domain mounted overlay-host seam over the existing neutral `telorgon-ui::OverlayHost`
lifecycle owner, with one host per view and portable/mounted fixtures, without duplicating neutral
overlay identity/dismissal/focus policy or adding native windows, platform input routing, popup
placement, or later overlay components.
The thirty-second Gate 8 package now adds `ApplicationOverlayHost` in
`crates/telorgon/src/application_components/overlay/host.rs`. One wrapper owns exactly one existing neutral
`telorgon-ui::OverlayHost` and binds it once to a full-view, contained, overlay-flow foundation node
that is noninteractive and absent from semantics. The returned generational node is the explicit
runtime portal target. Opening first proves that exact mount generation belongs to the supplied
`MountedUi`; not-mounted, duplicate-mount, stale-parent, stale-mount/cross-view, and neutral
lifecycle errors remain typed and distinct. Entry identity, bottom-to-top ordering, parentage,
modality, barrier state, dismissal policy, anchor cleanup, focus requests, outside-input disposition,
and diagnostics delegate unchanged to the neutral owner. View loss closes the current stack while
retaining the live mount; explicit owner unmount closes entries with `OwnerUnmounted` and releases
the association. Portable/mounted fixtures prove the portal-host geometry and nonparticipation,
duplicate and cross-view rejection, modal state/focus effects, nested top-first dismissal with input
consumption, owner-unmount closure, and public compile paths. The wrapper does not mount overlay
content, create portals, route input, apply focus/inert semantics, solve placement, create native
windows, or enforce global uniqueness between unrelated wrapper instances; the application root is
still responsible for owning one wrapper per view. The exact next eligible Slice 3 package is
`crates/telorgon/src/application_components/overlay/controller.rs`: it must define the application command
and typed effect boundary over the mounted host and neutral lifecycle records, including open,
dismiss, anchor-removal, view-loss, and owner-unmount transitions with portable fixtures, without
duplicating lifecycle state, mounting overlay components, routing platform input, applying focus,
or solving popup placement.
The thirty-third Gate 8 package now adds `ApplicationOverlayController` in
`crates/telorgon/src/application_components/overlay/controller.rs`. The controller owns exactly one
`ApplicationOverlayHost`, delegates mounting to it, and exposes one typed command route for open,
dismiss, anchor removal, view loss, and owner unmount. Only `Open` carries the `MountedUi` reference
needed by neutral anchor/focus and exact-host-generation validation; its custom debug output omits
that UI's retained contents. Effects preserve `OverlayOpened`, policy-blocked or accepted dismissal,
top-first close outcomes, focus requests, and outside-input consumption without executing them.
Controller inspection state is recomputed from the host's entries, top entry, modal entry, and mount
association rather than retaining a second stack. View loss closes entries but retains the mount;
owner unmount closes entries and releases it. Portable mounted fixtures prove open effects and
derived state, nonmutating blocked dismissal, top-first anchor-removal closure, view-loss reopen,
terminal owner cleanup, diagnostic omission, and public compile paths. Portal content, focus/inert
application, input routing, placement, native windows, and callbacks remain absent. The exact next
eligible Slice 3 prerequisite is `crates/telorgon/src/layout/popup_placement.rs`: it must implement the pure
anchor/safe-and-occluded-bounds candidate placement solver required before
`crates/telorgon/src/application_components/overlay/placement.rs`, with deterministic geometry fixtures and
no component, overlay-lifecycle, runtime, platform, native-window, or rendering ownership.
The thirty-fourth Gate 8 package now adds the neutral popup solver in
`crates/telorgon/src/layout/popup_placement.rs`. It validates finite anchor rectangles, positive content and
safe bounds, bounded finite occlusions, nonnegative gap, unique ordered candidates, and valid
minimum resize/scroll viewports before deriving geometry. Above, below, inline-start, and inline-end
candidates carry start/center/end alignment; horizontal start/end and inline sides resolve through
the canonical writing direction. The solver tries all candidates exactly in declared order before
using the explicit Reject, Shift, Resize, or Scroll policy. Occlusions are deterministically
subtracted from safe bounds into free rectangles, so neither exact nor adjusted placements overlap a
known occlusion. Shift preserves content size, Resize returns a constrained layout rectangle, and
Scroll returns an explicitly scroll-required viewport; no policy silently changes into another.
Results report the selected candidate, original content size, final rectangle, containing usable
region, adjustment/delta, and deterministic candidate-attempt count. Unit and public fixtures prove
RTL start alignment, flip-before-fallback ordering, preferred-candidate shifting, distinct resize
and scroll results, occlusion avoidance, malformed input rejection, no-fit behavior, and umbrella
compile paths. The solver owns no cache, overlay entry, component/controller state, portal, layout
mutation, focus, input route, platform conversion, native window, or renderer. Callers remain
responsible for recomputing only when anchor/content/scale/usable-bound inputs change. The exact next
eligible Slice 3 package is `crates/telorgon/src/application_components/overlay/placement.rs`: it must define
the application candidate/default overflow policy and request/result boundary over this neutral
solver, including environment safe/occluded geometry and writing direction with portable fixtures,
without duplicating solver geometry, mutating overlay lifecycle, mounting popup content, routing
input, or adding platform/native-window behavior.
The thirty-fifth Gate 8 package now adds the application popup-placement boundary in
`crates/telorgon/src/application_components/overlay/placement.rs`. Its stable ordinary-popup policy tries
below-start, above-start, inline-end-start, and inline-start-start in order, then uses size-preserving
Shift only after every exact candidate fails; the default gap remains zero so component/theme
spacing is not invented by policy. Custom candidates, gap, and every neutral overflow policy remain
explicit. Each request derives positive view-local logical safe bounds from available size and safe
insets, forwards environment occlusions and canonical writing direction unchanged, and delegates all
candidate, occlusion, shift, resize, and scroll geometry to `telorgon-layout`. Results preserve the
neutral placement while recording the safe bounds, writing direction, and device scale relevant to
caller-owned recomputation. Invalid scale, exhausted safe bounds, and the original neutral solver
failure remain typed and distinct. Unit, direct-public, and umbrella fixtures prove stable defaults,
safe-area derivation, RTL alignment, flip-before-shift behavior, occlusion forwarding without
environment mutation, custom scroll results, malformed input rejection, and public compile paths.
The adapter owns no cache, overlay lifecycle mutation, portal or popup content, focus/inert
application, input routing, platform conversion, native window, or rendering behavior. The exact
next eligible Slice 3 package is `crates/telorgon/src/application_components/overlay/popup.rs`: it must compose
the existing application controller and placement boundary into the typed nonmodal popup
configuration/open-result seam with portable and mounted fixtures, without duplicating lifecycle or
solver state, applying focus/input effects inline, creating native windows, or implementing later
dialog, sheet, tooltip, or toast policy.
The thirty-sixth Gate 8 package now adds `Popup` in
`crates/telorgon/src/application_components/overlay/popup.rs`. `PopupAnchor` pairs a node identity with its
resolved logical bounds, or carries a point/rectangle directly, so lifecycle validation and
placement consume one application configuration without sharing mutable state. The ordinary popup
is structurally nonmodal, closes on Escape, consuming outside press, or focus loss, and restores a
node anchor with nearest-live fallback; dismissal, focus, parent-overlay, and placement policies
remain explicit overrides without exposing a modality switch. Opening first delegates environment-
aware geometry to the application placement adapter, then issues exactly one controller Open
command. Placement rejection and lifecycle rejection are typed separately and leave the host stack
unchanged. The successful result pairs the generational neutral overlay/focus output with the
application placement. Unit, direct-public mounted, and umbrella fixtures prove standard nonmodal
policy, exact placement, focus restoration, flip/scroll preservation, nested parentage, atomic
placement and lifecycle rejection, no inert background, no content mount, and public compile paths.
The popup owns no second lifecycle or geometry state, portal content, focus/input effect execution,
native window, platform adapter, or rendering behavior. The exact next eligible Slice 3 package is
`crates/telorgon/src/application_components/overlay/dialog.rs`: it must define the typed modal dialog policy
and open-result seam over the same controller, including mandatory initial focus, containment,
restoration, barrier/inert intent, and safe placement with portable and mounted fixtures, without
applying focus/input/semantics effects inline, creating native windows, or implementing sheet,
tooltip, or toast policy.
The thirty-seventh Gate 8 package now adds `Dialog` in
`crates/telorgon/src/application_components/overlay/dialog.rs`. Construction requires a nonempty accessible
name, opener identity, resolved logical placement anchor/content size, and a typed non-optional
initial-focus choice. Every dialog opens as Modal with focus containment and opener-then-nearest
restoration fixed structurally; no builder can weaken those invariants. Standard, Destructive, and
Critical meaning is typed. The barrier always reports background-inert intent and blocks outside
press by default; dismissal-and-consume requires an explicit opt-in and is never silently enabled
for destructive or critical dialogs. Escape remains explicitly configurable while focus loss and
pointer departure never dismiss a modal dialog. Opening preserves placement-before-lifecycle
atomicity, environment safe/occluded geometry, parent overlay identity, and distinct placement versus
controller errors. The result pairs generational lifecycle/focus output, safe placement, dialog
kind, and barrier intent without applying any effect. Unit, direct-public mounted, and umbrella
fixtures prove name validation, mandatory focus, containment/restoration, modal/inert state,
destructive barrier opt-in, explicit-focus validation, nested parentage, scroll-constrained safe
geometry, atomic rejection, no content mount, and public compile paths. The dialog owns no portal
content, focus/input/semantics effect application, native window, platform adapter, or rendering
behavior. The exact next eligible Slice 3 package is
`crates/telorgon/src/application_components/overlay/sheet.rs`: it must define the typed edge-attached sheet
policy and open-result seam over the same controller and placement owners, including explicit
modal/nonmodal mode, edge, constrained scrolling, focus/restoration, barrier intent, and safe-area
behavior with portable and mounted fixtures, without applying effects inline, creating native
windows, or implementing tooltip or toast policy.
The thirty-eighth Gate 8 package now adds `Sheet` in
`crates/telorgon/src/application_components/overlay/sheet.rs`. Construction requires a nonempty accessible
name, opener identity, logical attachment edge, desired content/minimum scroll viewport, and an
explicit nonmodal or modal mode. Block and inline edges resolve against writing direction while the
application placement owner remains the single source of safe-area arithmetic, occlusion handling,
and typed scroll-constrained geometry. Nonmodal sheets take no initial focus, contain no focus, and
report no inert barrier. Modal sheets require typed initial focus, contain focus, report background
inertness, and make block-versus-dismiss-and-consume barrier behavior explicit. Both modes restore
to the opener or nearest live fallback, never dismiss on focus loss or pointer departure, and keep
Escape explicitly configurable. Opening preserves placement-before-lifecycle atomicity, parent
overlay identity, distinct placement/controller errors, and a direction-resolved physical edge in
the result without mounting content or applying any effect. Unit, direct-public mounted, and
umbrella fixtures prove name validation, LTR/RTL edge resolution, safe-area attachment, modal and
nonmodal lifecycle policy, barrier intent, constrained scrolling, parentage, atomic rejection, no
content mount, and public compile paths. The sheet owns no portal content, scroll controller,
focus/input/semantics effect application, native window, platform adapter, or rendering behavior.
The exact next eligible Slice 3 package is
`crates/telorgon/src/application_components/overlay/tooltip.rs`: it must define the typed nonfocusable
tooltip policy and open-result seam over the same controller and placement owners, including
hover/sustained-focus trigger intent with caller-owned deadlines, Escape/pointer-departure/focus-loss
dismissal, accessible-description-only policy, text-scale-aware constrained placement, and portable
and mounted fixtures, without running timers, applying effects inline, creating native windows, or
implementing toast policy.
The thirty-ninth Gate 8 package now adds `Tooltip` in
`crates/telorgon/src/application_components/overlay/tooltip.rs`. A tooltip requires nonempty supplemental
description text, a generational node anchor with resolved bounds, scale-one desired/minimum reflow
geometry, and a validated hover and/or sustained-focus trigger policy. Trigger policy returns
host-clock deadline intents from caller-supplied monotonic instants, rejects zero/out-of-range
delays and clock overflow, and never starts a timer. Opening requires an enabled trigger, scales
desired/minimum geometry and logical gap by the validated environment text scale, then delegates
safe-area, occlusion, writing-direction, candidate, and typed resize behavior to the application
placement owner before lifecycle mutation. Every tooltip is structurally NonModal, takes and
restores no focus, has no containment or outside-press behavior, and never makes the background
inert. Escape, pointer departure, and focus-loss dismissal are typed and explicitly configurable.
The semantic result fixes Tooltip role, anchor `DescribedBy` relationship, and DescriptionOnly
contribution so the API cannot promote tooltip text to the anchor's accessible name. Unit,
direct-public mounted, and umbrella fixtures prove trigger/deadline validation, description-only
semantics, nonfocusable lifecycle, default/custom dismissal, nested parentage, text-scale-resolved
constrained placement, atomic rejection, no content mount, and public compile paths. The tooltip
owns no timer handle, portal content, text measurement/reflow engine, semantic-tree mutation,
focus/input effect application, native window, platform adapter, or rendering behavior. The exact
next eligible Slice 3 package is
`crates/telorgon/src/application_components/overlay/toast.rs`: it must define the typed nonfocusable toast
policy and open-result seam over the same overlay owner, including explicit live-announcement
priority, coalescing/redaction intent, caller-owned expiry deadlines, safe application placement,
dismissal intent, and portable and mounted fixtures, without running timers, stealing focus,
applying effects inline, creating native windows, or implementing a platform notification service.
The fortieth Gate 8 package now adds `Toast` in
`crates/telorgon/src/application_components/overlay/toast.rs`. A toast requires nonempty visible message
text, a logical safe-area corner, scale-one desired/minimum reflow geometry, typed announcement
policy, and a validated persistent or expiring lifetime. Polite and Assertive priorities map to
Status and Alert intent without mutating a semantic tree. Independent or opaque-key replacement
coalescing and None/Diagnostics/AnnouncementAndDiagnostics redaction are explicit outputs; redacted
toast debug output omits its message. Expiring lifetimes return overflow-checked host-clock intents
from caller-supplied monotonic instants and never schedule a timer. Opening scales geometry and gap
by environment text scale, resolves logical corners in LTR/RTL, delegates safe-area/occlusion and
typed resize behavior to the application placement owner, then opens one structurally NonModal
entry. Toasts take and restore no focus, have no containment, outside-press, focus-loss, or pointer-
departure behavior, and never make lower content inert. Escape/manual/expiry dismissal intent is
returned without applying it. Unit, direct-public mounted, and umbrella fixtures prove lifetime and
deadline validation, announcement priority, keyed coalescing, redaction, polite/assertive roles,
LTR/RTL safe placement, text-scale constraints, nested parentage, atomic rejection, no content
mount, and public compile paths. The toast owns no timer handle, coalescing queue, live-region or
semantic-tree mutation, portal content, focus/input effect application, native notification
service, platform adapter, or rendering behavior. Together with the existing neutral text-session
and explicit clipboard-capability inputs, this completes Gate 8 Slice 3's ordered text/overlay
baseline without claiming a native IME or clipboard implementation. The exact next eligible Gate 8
Slice 4 package is `crates/telorgon/src/application_components/command/model.rs`: it must define reusable
typed command identity/metadata, controlled availability/check state, and an owner-scoped fresh-
action factory that preserves `ChangeSource` without requiring `A: Clone`, invoking callbacks
inline during construction, implementing shortcut resolution, or adding menus/toolbars/navigation.
The forty-first Gate 8 package now adds the reusable command model in
`crates/telorgon/src/application_components/command/model.rs`. `CommandSpec<Id, A>` retains the caller's
typed identity, validated nonempty label and optional description, decorative icon metadata, and
same-component controlled enabled plus optional `CheckState` reads. Construction rejects read/
factory owner mismatches before a presenter can observe mixed generations. Mount-time resolution
reads one explicit controlled snapshot; invoking that snapshot returns a typed enabled/disabled
outcome carrying its check state and `ChangeSource`. Disabled outcomes do not call application code.
`ActionFactory<A>` shares an owner-scoped repeatable callable, stays lazy through construction and
clone, and constructs one fresh moved action per accepted invocation without requiring `A: Clone`.
Unit, direct-public mounted, and umbrella compile fixtures prove metadata validation, owner
rejection, controlled state resolution, disabled suppression, source preservation, lazy repeated
fresh actions, non-Clone action support, and curated public paths. This package owns no shortcut
matching, localized binding display, menu/toolbar/palette/navigation presentation, platform key
mapping, state mutation, input route, callback scheduling, or service behavior. The exact next
eligible Gate 8 Slice 4 package is
`crates/telorgon/src/application_components/command/shortcut_scope.rs`: it must adapt command shortcut
declarations and current controlled availability to the existing neutral `ShortcutMatcher`, retain
innermost-first scope/modal/priority/repeat and typed ambiguity outcomes, and keep display bindings
separate from physical matching without invoking an `ActionFactory`, duplicating the neutral
matcher, mapping platform keys, or adding menus/toolbars/navigation.
The forty-second Gate 8 package now adds
`crates/telorgon/src/application_components/command/shortcut_scope.rs`. `ShortcutSet` stores validated
alternative command bindings and is now part of `CommandSpec`; each declaration keeps its exact
physical `ShortcutChord`, caller-supplied nonempty localized display binding, priority, and repeat
policy separate. A command produces a scope/generation registration only for a declared shortcut
index, carrying its typed command ID and controlled enabled read but no action factory. The
application `CommandShortcutScope` resolves all registration reads before atomically replacing its
snapshot, delegates duplicate-key validation, innermost-first scope ordering, modal blocking,
priority, repeat suppression, ambiguity, and diagnostics to the existing neutral
`ShortcutMatcher`, and attaches display metadata only after one exact match. Failed controlled or
matcher updates retain the prior matcher/display snapshot. Unit, direct-public mounted, and umbrella
fixtures prove display/set validation, scope precedence over priority, disabled inner fallthrough,
typed ambiguity and modal outcomes, repeat suppression, atomic duplicate rejection, current
controlled availability, public compile paths, and zero `ActionFactory` invocation during matching.
This package does not map native/platform keys, infer localized labels, invoke commands, mutate
command state, register a runtime keyboard route, or add menus, toolbars, palettes, navigation, or
platform services. The exact next eligible Gate 8 Slice 4 package is
`crates/telorgon/src/application_components/command/toolbar.rs`: it must define the baseline typed toolbar
command view over shared `CommandSpec` values and the neutral composite owner, including one focus
entry, orientation/direction-aware arrows plus Home/End, explicit disabled-item discovery,
controlled availability, toolbar/item semantics, and source-preserving invocation proposals, with
portable and mounted fixtures but without menus, overflow policy, drag customization, platform
services, or a second focus/shortcut/command implementation.
The forty-third Gate 8 package now adds
`crates/telorgon/src/application_components/command/toolbar.rs` and the neutral `Toolbar` semantic role. A
toolbar requires a nonempty accessible name, at least one uniquely identified same-component
`CommandSpec`, explicit horizontal/vertical navigation policy, density metrics, and typed visual
slots. `ToolbarBehavior` wraps the existing neutral composite with independent selection,
orientation- and writing-direction-aware arrows, Home/End, explicit stop/wrap edges, and fixed
disabled-item discovery; attempting to invoke a highlighted disabled command returns the neutral
typed rejection. Mounting reads each command's controlled enabled/checked state, creates one named
focusable Toolbar root, mounts its command buttons as non-tab-stop owned descendants, reports the
initial active descendant, maps checked/mixed state, enforces the density floor, and installs routes
only for enabled items. Accepted pointer, keyboard, accessibility, programmatic, or explicit active
invocations call the existing command factory exactly once, preserve `ChangeSource`, and move a
fresh action without requiring `A: Clone`. Unit, direct-public mounted, neutral-semantics, and
umbrella fixtures prove construction/owner rejection, horizontal/vertical and RTL navigation,
disabled discovery/suppression, one-focus-stop semantics, Touch geometry, mixed checked state,
source preservation, root/item routes, non-Clone actions, and public compile paths. Dynamic
availability/style/semantic/active-descendant patching after mount, raw keyboard translation,
overflow, customization drag, menus, platform services, and platform accessibility export remain
open. The exact next eligible Gate 8 Slice 4 package is
`crates/telorgon/src/application_components/command/menu_controller.rs`: it must define one typed menu-chain
lifecycle/highlight controller over the existing application overlay and neutral composite owners,
including explicit parentage, opening focus intent, one-level versus chain dismissal ordering,
disabled-command rejection, source-preserving command intent after close effects, and caller-owned
submenu deadline/cancellation intent, with portable and mounted fixtures but without mounting menu
rows, running timers, applying focus/input effects, invoking platform services, or duplicating
overlay/composite/command state.
The forty-fourth Gate 8 package now adds
`crates/telorgon/src/application_components/command/menu_controller.rs`. `MenuController<K>` retains only a
linear root-to-leaf association between live generational overlay entries and one existing neutral
vertical composite per level. Root and submenu opens validate explicit leaf parentage before routing
one nonmodal overlay open, return selected-or-first focus intent without applying it, and preserve
node-anchor restoration. Highlight movement uses the neutral wrap, Home/End, writing-direction, and
disabled-discovery policy; disabled active commands cannot activate. Escape-style level dismissal
targets only the current leaf, chain dismissal targets the root, and both preserve the neutral
topmost-first close effect. Activation validates the supplied shared `CommandSpec` and controlled
snapshot, completes the configured close first, then returns one source-preserving fresh moved action
without enqueueing it or requiring `A: Clone`. Submenu hover produces only absolute monotonic deadline
and matching cancellation values for the caller's scheduler. Unit, direct-public mounted, and
umbrella fixtures prove opening focus, explicit parentage, disabled discovery/rejection, leaf versus
chain ordering, source/check-state preservation, non-Clone action support, no mounted menu rows, and
curated public paths. Menu row mounting, typeahead text matching, dynamic controlled-state patching,
raw key/pointer/focus routing, timer execution, focus-effect application, context menus, palettes,
platform services, and accessibility export remain open. The exact next eligible Gate 8 Slice 4
package is `crates/telorgon/src/application_components/command/menu.rs`: it must define the baseline mounted
typed menu-level command view over shared `CommandSpec` and `MenuController` owners, including named
Menu/MenuItem semantics, one composite focus entry with active-descendant relationships, controlled
enabled/checked snapshots, density/style slots, direction-aware arrows plus Home/End, baseline
typeahead highlight intent, and source-preserving activation/submenu routes, with portable and mounted
fixtures but without running submenu timers, applying focus/input effects, adding context-menu or
palette policy, invoking platform services, or duplicating command/composite/overlay state.
The forty-fifth Gate 8 package now adds `crates/telorgon/src/application_components/command/menu.rs`.
`Menu<K, A>` validates one accessible name, a nonempty unique same-owner `MenuItem` set, typed
density metrics, and deterministic visual slots. Mounting reads each shared command's controlled
enabled/checked snapshot, creates one named `Menu` focus entry plus non-tab-stop `MenuItem` rows,
publishes Owns and ActiveDescendant relationships, suppresses disabled actions, preserves mixed
checked state, and enforces the active density floor. `MenuRef` delegates Up/Down/Home/End and
direction-aware inline submenu open/close behavior to the existing `MenuController`; baseline
Unicode-lowercase prefix typeahead returns and applies typed highlight intent, including disabled
items for discovery. Accepted command rows close through the controller before returning a fresh
source-preserving non-Clone action, while submenu rows return typed parent/source intent. Unit,
direct-public mounted, semantic, density, route, and umbrella fixtures pass. Dynamic post-mount
availability/style/semantic updates, locale-tailored typeahead, raw input/focus-effect application,
submenu timers, platform services, and accessibility export remain open. The exact next eligible
Gate 8 Slice 4 package is `crates/telorgon/src/application_components/command/context_menu.rs`: it must add a
narrow application context-menu policy over the existing menu/controller/overlay owners, including
typed pointer, keyboard, and programmatic anchors and sources, source-specific opening focus,
one-level Escape versus consuming chain dismissal, and close-before-command action behavior, with
portable/direct fixtures but without native platform menus, mounted duplicate rows, timers, or
applied input/focus effects.
The forty-sixth Gate 8 package now adds
`crates/telorgon/src/application_components/command/context_menu.rs`. `ContextMenu<K>` owns exactly one
`MenuController<K>` and maps secondary-pointer coordinates, keyboard node anchors, or explicit
platform-neutral anchors into root menu opens. Keyboard opening requests selected-or-first focus;
pointer/programmatic opening defaults to no focus request, and callers may explicitly override that
intent. Escape closes only the current leaf, whereas outside press, cancellation, and replacement
close the chain; the existing overlay preserves focus restoration and outside-input consumption.
Activation rejects unavailable commands and closes the complete chain before returning one fresh
source-preserving non-Clone action. Unit and direct-public fixtures prove anchors, sources, focus,
restoration, one-level/chain ordering, disabled rejection, and zero mounted duplicate rows. Native
menu APIs, gesture recognition, raw key/pointer routes, placement rendering, timer execution, and
effect application remain outside this package. The exact next eligible Gate 8 package is the
focused Tier B `crates/telorgon/src/application_components/command/palette.rs`: it must define a bounded local
command-palette model over shared `CommandSpec`, composite, and overlay owners, including validated
query policy, deterministic typed ranking, controlled availability/check snapshots, disabled-result
navigation policy, modal focus/close intent, and close-before-fresh-action ordering, while remaining
outside the Tier A prelude and adding no mounted rows, platform search, services, or `A: Clone` bound.
The forty-seventh Gate 8 package now adds
`crates/telorgon/src/application_components/command/palette.rs` on the focused command-module path.
`CommandPalette<K, A>` validates its name, nonempty unique same-owner command set, bounded result and
query limits, and control-free query input. Mount-time refresh reads one controlled enabled/checked
snapshot; local Unicode-lowercase label/description matching ranks exact, prefix, word-prefix, then
substring results deterministically and truncates them to policy. The existing neutral composite
owns vertical wrap/highlight behavior and skips disabled results. Opening returns a contained modal
focus intent without mounting content; dismissal returns the overlay close/restoration effect; and
activation closes first before creating one fresh source-preserving non-Clone action. Unit and
direct-public fixtures prove ranking, bounds, disabled navigation, modal lifecycle, focus
restoration, close ordering, and the focused public path. This is a Tier B model/controller, not a
complete semantic palette component: mounted query/results, TextField composition, locale-tailored
or fuzzy matching, live post-mount controlled refresh, raw input routing, platform search/services,
and accessibility export remain open. The exact next eligible Gate 8 Slice 4 package is
`crates/telorgon/src/application_components/navigation/controller.rs`: it must define one typed application
route-stack/current-route/restoration-key owner with explicit push, replace, pop, and selection
requests plus source-preserving outcomes, portable fixtures, and no URL/native navigation service,
mounted route host, tabs, duplicated component state, or platform history integration.
The forty-eighth Gate 8 package now adds `NavigationController<R>` in
`crates/telorgon/src/application_components/navigation/controller.rs`. It retains one nonempty authoritative
stack of unique typed routes and unique optional restoration keys; validates push, replace, pop,
and retained-route selection atomically; and increments revisions only for accepted mutations.
Selection construction is nonmutating, stale requests are rejected, and accepted transitions retain
their `ChangeSource`, previous/current route, top-first removed entries, and the restoration key of a
revealed route. Unit, direct-public, and umbrella compile fixtures cover mutation ordering, source
preservation, root retention, restoration, duplicate rejection, and stale selection. The controller
is a logical in-memory owner: it does not mount route content, retain tab focus, apply restoration,
or integrate URL/native navigation, platform history, or accessibility export. The exact next
eligible Gate 8 Slice 4 package is `crates/telorgon/src/application_components/navigation/tabs.rs`: it must
define the baseline controlled tab/tab-panel view over this navigation owner and the existing
neutral composite, keep focused tab distinct from selected route, support direction-aware arrows
plus Home/End and explicit automatic/manual activation policy, preserve selection sources, and add
portable/mounted fixtures without adding a route-host/keep-alive engine, URL/platform service, or a
second route/focus/selection owner.
The forty-ninth Gate 8 package now adds `Tabs<R>` in
`crates/telorgon/src/application_components/navigation/tabs.rs`. It validates one named, nonempty,
stable-route tab set; reads the selected route directly from `NavigationController`; and delegates
the sole transient focused-tab state to the existing neutral composite. Horizontal/vertical,
writing-direction-aware arrows plus Home/End use explicit stop/wrap policy. `AutomaticLocal`
navigation emits a directional selection proposal only for caller-promised local latency-free
panels, while `Manual` navigation moves focus without selection until Enter/Space, pointer, or
semantic activation. All proposals preserve `ChangeSource` and never mutate navigation. Mounting
creates one focus entry, density-aware non-tab-stop Tab nodes, empty per-route TabPanel targets,
selected/hidden state, and Controls/LabelledBy/Owns/ActiveDescendant relationships. Unit,
direct-public, and umbrella compile fixtures cover validation, focus/selection separation,
automatic/manual activation, RTL/Home/End, density, semantics, and activation sources. Panel
content, state keep-alive/budgets, dynamic post-mount route/style/semantic patching, raw keyboard
routing, route hosting, URL/native navigation, platform history, and accessibility export remain
open. The exact next eligible Gate 8 Slice 4 package is
`crates/telorgon/src/application_components/navigation/breadcrumb.rs`: it must define the baseline typed
ancestor/current-route trail over the same navigation owner, with named ordered semantics,
decorative separators, a nonactivatable current item, source-preserving ancestor route proposals,
and portable/mounted fixtures without mounting route content, owning history/selection, or invoking
URL/native navigation or platform services.
The fiftieth Gate 8 package now adds `Breadcrumb<R>` in
`crates/telorgon/src/application_components/navigation/breadcrumb.rs`. It validates a named, nonempty,
unique-route label trail against the exact root-to-current `NavigationController` entry order and
rejects length or typed route mismatches before mounting. Ancestors emit nonmutating typed route
proposals with their original `ChangeSource`; the current route rejects direct proposals and has no
mounted action route. The mounted trail exposes one named ordered List, independently focusable
ancestor Link nodes with density floors, a selected noninteractive current ListItem, semantic
position/count metadata, and owned relationships. Visual separators are explicitly excluded from
semantics. Unit, direct-public, and umbrella compile fixtures cover construction, controller-trail
validation, nonmutation, controller selection handoff, source preservation, ordered semantics,
decorative separators, current-route rejection, and touch density. Overflow/collapse, dynamic
post-mount trail/style/semantic patching, route content, URL/native navigation, platform history,
and accessibility export remain open. The exact next eligible Gate 8 Slice 4 package is
`crates/telorgon/src/application_components/navigation/rail.rs`: it must define the baseline controlled
vertical navigation-destination view over `NavigationController` and the neutral composite, with a
named one-focus-entry group, selected-route semantics, Up/Down plus Home/End behavior,
source-preserving proposals, density/style slots, and portable/mounted fixtures without owning
route history, mounting route content, or invoking URL/native navigation or platform services.
The fifty-first Gate 8 package now adds `NavigationRail<R>` in
`crates/telorgon/src/application_components/navigation/rail.rs`. It validates one named, nonempty,
unique-route destination set; reads selected route only from `NavigationController`; and delegates
the sole transient focused destination to the existing neutral composite. Up/Down plus Home/End
use explicit stop/wrap policy, disabled destinations are skipped, and selection proposals retain
their `ChangeSource` without mutating navigation. Mounting creates one named focus entry,
density-aware non-tab-stop Link destinations, controlled selected/disabled and collection
semantics, plus Owns/ActiveDescendant relationships. Unit, direct-public, and umbrella compile
fixtures cover validation, focus/selection separation, disabled discovery, source preservation,
controller handoff, density, semantics, and root/destination activation. Icons, badges,
collapsed/expanded presentation, dynamic post-mount route/style/semantic patching, raw keyboard
routing, route content, URL/native navigation, platform history, and accessibility export remain
open. The exact next eligible Gate 8 Slice 4 package is
`crates/telorgon/src/application_components/navigation/bar.rs`: it must define the baseline controlled
horizontal compact navigation-destination view over the same navigation owner and neutral
composite, with a named one-focus-entry group, selected-route semantics, writing-direction-aware
Left/Right plus Home/End behavior, source-preserving proposals, density/style slots, and
portable/mounted fixtures without owning route history, mounting route content, or invoking
URL/native navigation or platform services.
The fifty-second Gate 8 package now adds `NavigationBar<R>` in
`crates/telorgon/src/application_components/navigation/bar.rs`. It validates one named, nonempty,
unique-route compact destination set; reads selected route only from `NavigationController`; and
delegates the sole transient focused destination to the existing neutral composite. Left/Right
navigation follows explicit writing direction, Home/End and stop/wrap policy remain neutral-owner
behavior, disabled destinations are skipped, and selection proposals retain their `ChangeSource`
without mutating navigation. Mounting creates one named focus entry, horizontal density-aware
non-tab-stop Link destinations, controlled selected/disabled and collection semantics, plus
Owns/ActiveDescendant relationships. Unit, direct-public, and umbrella compile fixtures cover
validation, LTR/RTL navigation, focus/selection separation, disabled discovery, source
preservation, controller handoff, density, semantics, and root/destination activation. Icons,
labels-at-narrow-width policy, overflow, automatic rail/bar adaptation, dynamic post-mount
route/style/semantic patching, raw keyboard routing, route content, URL/native navigation,
platform history, and accessibility export remain open. The exact next eligible Gate 8 Slice 4
package is `crates/telorgon/src/application_components/navigation/route_host.rs`: it must define a typed
controlled current-route content boundary over `NavigationController`, with explicit stable route
registrations, missing-route diagnostics, current-content visibility semantics, and bounded
keep-alive/restoration intents plus portable/mounted fixtures, without duplicating route history,
executing platform restoration, or invoking URL/native navigation or platform services.
The fifty-third Gate 8 package now adds the focused Tier B `RouteHost<R>` in
`crates/telorgon/src/application_components/navigation/route_host.rs`. It validates one named, nonempty,
unique-route content registry against every retained `NavigationController` entry and returns a
typed missing-route diagnostic before mounting. Its default has no inactive cache; its explicit
bounded policy considers nearest inactive stack entries first and retains them only while both the
route-count limit and caller-estimated byte budget permit. The plan separates current, kept-alive,
and evicted routes, totals inactive retained bytes, and exposes each controller restoration key as
an unapplied Fresh/Restore intent. Snapshot mounting creates content only for the current and
budgeted routes in stable registration order, marks inactive content visually hidden and
semantically hidden/inert, and publishes named owned content relationships. Unit, direct-public,
and focused umbrella compile fixtures cover validation, missing routes, no-cache defaults,
count/byte eviction, nearest-first retention, restoration intent, content construction, and
visibility semantics. This is not yet live keyed reconciliation: navigation changes require a new
mount snapshot, byte estimates are supplied rather than measured, and route-local component
lifecycle preservation, focus/scroll restoration application, transitions, loading/error content,
URL/native navigation, platform history, and accessibility export remain open. The exact next
eligible Gate 8 Slice 4 package is
`crates/telorgon/src/application_components/collection/selection.rs`: it must define the stable-key
`SelectionModel<K>` owner with None/Single/Multiple modes, explicit anchor and
selection-follows-focus policy, atomic controlled updates, source-preserving selection proposals,
reorder/removal recovery, diagnostics, and portable/direct-public fixtures without mounting rows,
owning collection data, or invoking platform services.
The fifty-fourth Gate 8 package now adds `SelectionModel<K>` in
`crates/telorgon/src/application_components/collection/selection.rs`. It validates one unique canonical key
order plus mode-compatible initial selected keys and anchor, owns those selection values without
owning collection data, and exposes None/Single/Multiple modes with an explicit
selection-follows-focus policy. Clear, exclusive select, multiple toggle/range extension,
focus-driven, and complete-set operations return nonmutating revision-stamped proposals that retain
`ChangeSource`; application rejects stale proposals atomically and advances revision only when
selection or anchor changes. Focus following replaces Single selection but only adds in Multiple
mode, so it cannot silently collapse an existing multi-selection. Atomic item snapshots preserve
surviving keys by identity in new canonical order, report removed selections, reject duplicate
updates unchanged, and recover a removed anchor to the nearest surviving selected key with a
deterministic successor tie-break. Unit, direct-public, and umbrella compile fixtures cover all
modes, invalid keys/anchors, source preservation, nonmutation, stale rejection, explicit range
selection, multi-focus preservation, reorder/removal, anchor recovery, and diagnostics. Row
mounting, collection storage, focus traversal, offscreen reveal, virtualization, semantic export,
drag selection, and platform services remain open. The exact next eligible Gate 8 Slice 4 package
is `crates/telorgon/src/application_components/collection/list.rs`: it must define the baseline named
stable-key `ListView<K>` for ordinary rows with independent interactive descendants, controlled
item snapshots, ordered List/ListItem semantics and collection metadata, density/style slots,
reorder/removal diagnostics, and portable/mounted/direct-public fixtures without adding composite
selection/focus, virtualization, collection-data ownership, or platform services.
The fifty-fifth Gate 8 package now adds `ListView<K>` in
`crates/telorgon/src/application_components/collection/list.rs`. It validates a named controlled snapshot of
uniquely keyed, accessibly named ordinary row descriptors while allowing an explicitly empty list.
Atomic snapshot updates retain no business data: they report inserted keys in new order, removed
keys in old order, per-key old/new moves, accessible-label updates, reuse count, revision, and
aggregated diagnostics; duplicate snapshots reject without changing rows or revision. Mounting
creates one named vertical List with known item count, owned nonfocusable ListItem rows in canonical
order, exact position/count metadata, density floors, and caller-provided contents that may mount
their own independently focusable controls. Empty lists report a known zero count without
fabricating a set position. Unit, mounted, direct-public, and umbrella compile fixtures cover
validation, empty state, atomic keyed diffs, diagnostics, semantic order/relationships, Touch
density, nonfocusable rows, and independent focusable descendants. Live keyed runtime
reconciliation, collection storage, selection/composite focus, row activation policy,
virtualization, offscreen reveal, unknown totals, drag/reorder behavior, and accessibility export
remain open. The exact next eligible Gate 8 Slice 4 package is
`crates/telorgon/src/application_components/collection/virtual_list.rs`: it must define the baseline
stable-key `VirtualListView<K>` over the existing neutral virtualization owner, with explicit
viewport/overscan inputs, bounded keyed materialization/cache outcomes, known-versus-unknown total
semantic metadata, reveal intent, and portable/mounted/direct-public fixtures without owning
collection data, selection/focus, scrolling physics, background work, or platform services.
The fifty-sixth Gate 8 package now adds `VirtualListView<K>` in
`crates/telorgon/src/application_components/collection/virtual_list.rs` and the neutral per-item extent query
it requires in `crates/telorgon/src/layout/engine.rs`. The view composes `ListView<K>` for validated
stable-key descriptor snapshots and preserves measured extents by key across reorder/removal.
Validated viewport and policy values produce separate visible and materialized ranges; overscan
rows outside the visible range are explicitly identified as cache rows and remain bounded by the
configured item budget even when the visible set exceeds that budget. Plans expose keyed ranges,
content extent, and leading/trailing extent spacers, while known totals must exactly match the
descriptor snapshot and unknown totals remain unknown in semantic metadata. Keyed reveal emits an
existing content-space `RevealRequest` without changing scroll state. Mounting rejects stale plans,
builds only planned nonfocusable ListItem rows, preserves global positions and known set sizes, and
applies density floors without mounting collection data or independently interactive descendants.
Unit, mounted, direct-public, neutral-layout, and umbrella compile fixtures cover invalid geometry,
total mismatch atomicity, bounded overscan, keyed measurement preservation, unknown totals, reveal
alignment, semantic positions, and partial materialization. Live keyed runtime reconciliation,
incremental extent indexing, collection/data loading, selection/composite focus, row activation,
scroll offset/physics, background prefetch, and platform accessibility export remain open. The exact
next eligible Gate 8 Slice 4 package is
`crates/telorgon/src/application_components/collection/listbox.rs`: it must define the baseline stable-key
`ListBox<K>` by composing the existing `SelectionModel<K>` with one neutral composite focus entry,
accessibly named enabled/disabled options, controlled selection proposals, directional/Home/End
navigation, density/style slots, Option/ListBox semantics, and portable/mounted/direct-public
fixtures without owning collection data, virtualization, scroll physics, background work, or
platform services.
The fifty-seventh Gate 8 package now adds `ListBox<K>` in
`crates/telorgon/src/application_components/collection/listbox.rs` and the neutral `ListBox`/`Option`
semantic roles it requires in `crates/telorgon/src/ui/semantics.rs`. Construction requires named uniquely
keyed options whose canonical keys exactly match the supplied `SelectionModel<K>`, leaving that
model as the only selected-key/anchor owner. One vertical `CompositeStateMachine` owns transient
active-descendant state, skips disabled options, stops at boundaries, handles directional/Home/End
navigation, and recovers a removed active key to its nearest surviving successor then predecessor.
Navigation can return policy-driven focus-selection proposals without applying them; Multiple mode
adds focused keys without collapsing existing selection. Pointer, accessibility, directional, and
programmatic option requests retain `ChangeSource`; Single mode proposes exclusive selection,
Multiple proposes toggle, and None mode remains focusable for browsing without advertising select.
Atomic option updates preserve selection and highlight by key, while disabled options cannot emit
selection. Mounting creates one focusable named ListBox root, nonfocusable named Option descendants,
global collection positions, selected/disabled state, Select actions only for enabled selectable
options, an active-descendant relationship, density floors, and text-only option descendants that
cannot become independent controls. Unit, mounted, semantic-foundation, direct-public, and umbrella
compile fixtures cover validation, selection-item identity, disabled skipping, Home/End boundaries,
multiple-selection preservation, source retention, apply/reorder/removal behavior, focus recovery,
semantic actions/relationships, and Touch density. Typeahead, range-modifier input translation,
virtualization, collection data, scrolling, drag selection, live mounted reconciliation, and
platform accessibility export remain open. The exact next eligible Gate 8 Slice 4 package is
`crates/telorgon/src/application_components/collection/table.rs`: it must define the baseline stable-key
noninteractive `Table` model/view with accessibly named row/column headers, validated rectangular
cell relationships, ordered Table/Row/Cell/ColumnHeader/RowHeader semantics, density/style slots,
and portable/mounted/direct-public fixtures without adding selection, composite focus, editing,
sorting, resizing, virtualization, collection-data ownership, or platform services.
The fifty-eighth Gate 8 package now adds `Table<R, C>` in
`crates/telorgon/src/application_components/collection/table.rs` and strengthens neutral semantic validation
so ColumnHeader and RowHeader roles require names. Construction requires a named table, at least one
uniquely keyed named column, uniquely keyed named rows, and exactly one cell for every column in
canonical column order. A cell carries only its stable column key and presentation text; keyed row,
column, and row-column lookup expose the validated descriptor snapshot without taking business-data
ownership. Empty cell text and an empty table body remain valid, while duplicate keys, missing
columns, ragged rows, and misordered/mismatched cell-column relationships reject before mounting.
Mounting creates one named Table, one structural header Row, ordered named ColumnHeader nodes,
ordered data Rows with named RowHeader nodes, and ordered Cells. Every Cell is labelled by both its
mounted row and column header; Table and Row ownership relationships preserve canonical structure,
known row/column positions are attached, and a known-zero body retains its column headers without a
fabricated row position. Density floors and separate table/header/row/cell style and text slots are
applied uniformly. No mounted table node is focusable, actionable, or runtime-routed, and there is no
caller content slot that could introduce interactive descendants. Unit, mounted, semantic-
foundation, direct-public, and umbrella compile fixtures cover name/key validation, rectangularity,
column order, keyed lookup, empty bodies/cells, semantic roles/counts/header relationships, Touch
density, and noninteraction. Controlled descriptor reconciliation, intrinsic shared-column sizing,
selection, focus/navigation, editing, sorting, resizing, virtualization, collection data, and
platform accessibility export remain open. The exact next eligible Gate 8 Slice 4 package is
`crates/telorgon/src/application_components/collection/data_grid.rs`: it must define the baseline stable-key
`DataGrid<R, C>` behavior/view over validated rectangular row/column descriptors, with one composite
focus entry, a distinct active cell coordinate, directional/Home/End cell navigation, controlled
selection/activation intents, ordered Grid/Row/Cell/header semantics, density/style slots, and
portable/mounted/direct-public fixtures without yet owning editing data, sorting, column resizing,
virtualization, scrolling physics, background work, or platform services.
The fifty-ninth Gate 8 package now adds `DataGrid<R, C>` in
`crates/telorgon/src/application_components/collection/data_grid.rs`. It accepts the already validated
`Table<R, C>` as the sole rectangular descriptor owner and requires the supplied
`SelectionModel<DataGridCell<R, C>>` canonical keys to match its row-major cell coordinates exactly.
The selection model remains the only selected-key and anchor owner, while one neutral composite owns
the distinct active cell. Up/Down move by row, writing-direction-aware Left/Right move by column,
Home/End move within the current row, and Previous/Next traverse row-major order through that same
composite without adding a second focus or selection state machine. Activation preserves its source,
returns a controlled selection proposal appropriate to None/Single/Multiple mode, and does not
mutate selection until the caller applies the revision-checked proposal. Mounting wraps the composed
table in one focusable named Grid root, excludes the inner Table semantic root, retains ordered
header and row ownership, patches cells with controlled selected/action state, adds the active-
descendant relationship, and routes root or cell activation to the current or addressed coordinate.
Unit, mounted, direct-public, and umbrella compile fixtures cover canonical-key validation,
two-dimensional and RTL navigation, active/selection separation, source preservation, controlled
application, one focus entry, semantic ownership/selection/actions, activation routes, and density
composition. Editing values and focus handoff, sorting, column resizing, live descriptor/style/
semantic reconciliation, virtualization, scrolling and reveal behavior, background collection work,
and platform accessibility export remain open. The exact next eligible Gate 8 Slice 4 package is
`crates/telorgon/src/application_components/collection/tree.rs`: it must define the baseline stable-key
`TreeView<K>` over validated hierarchical descriptors, with one vertical composite focus entry,
caller-controlled expanded keys and selection, level/parent/set metadata, open-or-descend and
close-or-ascend directional behavior, density/style slots, and portable/mounted/direct-public
fixtures without lazy collection data, virtualization, drag/reorder behavior, background work, or
platform services.
The sixtieth Gate 8 package now adds `TreeHierarchy<K>` and `TreeView<K>` in
`crates/telorgon/src/application_components/collection/tree.rs`. The hierarchy validates unique stable keys,
parent-before-child canonical preorder, contiguous subtrees, named items, and unique branch-only
expanded keys; it derives visibility, parent/children, one-based levels, and direct-sibling
position/set metadata without owning collection data. Expansion requests retain source and base
revision and remain nonmutating until explicit application. `TreeView` requires its sole
`SelectionModel<K>` canonical order to match every hierarchy key and feeds only visible enabled keys
to one vertical neutral composite. Up/Down/Previous/Next/Home/End traverse visible preorder; logical
open emits expansion or descends to an enabled child, logical close emits collapse or ascends, and
Left/Right mirror under RTL. Selection-follows-focus and activation return controlled proposals
without collapsing existing multiple selection. Mounting creates one named focusable Tree, only
visible nonfocusable TreeItems, total logical counts, level and sibling metadata, controlled
selected/expanded/disabled state, Expand/Collapse/Select actions, ordered ownership, one active-
descendant relationship, density floors, and source-preserving item/root activation routes. Unit,
mounted, semantic-foundation, direct-public, and umbrella compile fixtures cover invalid hierarchy
and expansion snapshots, visibility/metadata, LTR/RTL navigation, proposal atomicity, active/
selection separation, one focus entry, semantics, density, and routing. Live descriptor/style/
semantic reconciliation, independently routed semantic Expand/Collapse actions, lazy children,
virtualization, reveal/scroll behavior, drag/reorder, background data work, and platform
accessibility export remain open. The exact next eligible Gate 8 Slice 4 package is
`crates/telorgon/src/application_components/collection/tree_grid.rs`: it must reuse `TreeHierarchy<R>` as
the only hierarchy/expansion owner and `DataGrid<R, C>` as the only table/cell-selection/composite
owner, validate visible row identity, add disclosure-column hierarchical navigation and TreeGrid row
metadata, and land mounted/direct-public fixtures without a second selection/focus owner.
The sixty-first Gate 8 package now adds `TreeGrid<R, C>` in
`crates/telorgon/src/application_components/collection/tree_grid.rs` plus the neutral named `TreeGrid`
semantic role. Construction requires one disclosure column, enabled hierarchy items, exact
canonical visible-row identity, and row labels that agree with the shared hierarchy. It composes one
`TreeHierarchy<R>` with one `DataGrid<R, C>` rather than retaining a second tree or grid composite:
ordinary cell movement delegates to DataGrid, while logical open/close in the disclosure column
emits a controlled expansion proposal or asks that same DataGrid cell owner to descend/ascend.
Expansion application is atomic only alongside a caller-supplied replacement DataGrid whose rows
match the resulting visible hierarchy, so neither collection data nor hidden-row cell state is
fabricated. Mounting reuses the DataGrid root as the single focus entry, replaces its Grid meaning
with TreeGrid, retains the excluded structural Table, and adds total logical row counts, levels,
sibling positions, and branch expanded/actions to visible Rows while preserving cell selection,
header labelling, activation routes, column metadata, and active descendant. Unit, mounted,
semantic-foundation, direct-public, and umbrella compile fixtures cover disclosure/row validation,
controlled atomic expansion, LTR/RTL descend/ascend, delegated grid movement, multiple-selection
nonmutation, one focus entry, hierarchy metadata, semantic-root replacement, density composition,
and public paths. Disabled hierarchical rows are rejected until DataGrid has a disabled-cell/row
contract; editing handoff, sorting/resizing, live reconciliation, hidden-row state retention,
virtualization, reveal/scroll behavior, background work, semantic Expand/Collapse dispatch, and
platform accessibility export remain open. This completes Gate 8 Slice 4.
The sixty-second Gate 8 batch begins Slice 5 with stable `FieldMetadata<K>` in
`crates/telorgon/src/application_components/form/field.rs` and typed `FieldValidation<K>` plus
`ValidationResult` in `form/validation.rs`. Metadata validates visible label and optional help text
while retaining required, read-only, and enabled inputs independently from any field value or
component type. Every Warning, Invalid, and Pending result carries construction-validated nonempty
visible text and remains explicitly associated with one field key. Semantic decoration rejects
mismatched keys and missing, unexpected, or duplicate support nodes before publication; it merges
required/read-only/disabled plus invalid/pending state, removes mutation actions for read-only
fields, and associates mounted help and validation text through typed Help, DescribedBy, and
ErrorMessage relationships. Portable, mounted, direct-public, and umbrella compile fixtures cover
metadata/message validation, stable association, read-only action policy, generation-checked
relationships, and public paths. No form ordering, value ownership, validation execution,
submission, focus/reveal application, summary, layout, platform service, or accessibility export is
claimed.
The sixty-third Gate 8 package adds `Form<K>` in
`crates/telorgon/src/application_components/form/form.rs`. It validates unique stable field keys plus exactly
one controlled validation per field, canonicalizes arbitrarily ordered validation inputs to field
order, and replaces metadata/order/validation snapshots atomically under one revision. Submission
inspection returns the first Invalid field in canonical order with typed field-focus and nearest-
edge reveal intents, or an accepted result that retains ordered Warning and Pending keys for
explicit caller policy. Diagnostics distinguish updates, unchanged snapshots, accepted/invalid
submissions, and rejected updates. Unit, direct-public, and umbrella compile fixtures cover
duplicate/unknown/missing validation rejection, canonicalization, atomic updates, revision behavior,
first-invalid priority, intents, and accepted warning/pending visibility. This owner does not retain
field values, run validation, execute focus/scrolling, mount layout, invoke submission callbacks, or
create a validation summary.
The sixty-fourth Gate 8 package adds `ValidationSummary<K>` in
`crates/telorgon/src/application_components/form/summary.rs`. It derives Warning, Invalid, and Pending entries
from one `Form<K>` revision in canonical field order, retaining the form's validated labels and
messages without copying validation ownership. Its mounted representation exposes visible heading
and entry text, Alert semantics whenever an Invalid entry exists (Status otherwise), actionable Link
semantics, known collection positions, and density-aware targets. Entry activation returns a typed,
source-preserving action that reuses `FormFocusIntent` and `FormRevealIntent`; it does not execute
focus or scrolling. Unit, mounted, direct-public, and umbrella compile fixtures cover filtering,
ordering, revision identity, roles, relationships, density, and typed actions. The exact next
eligible Gate 8 Slice 5 package is
`crates/telorgon/src/application_components/structure/scaffold.rs`: it must establish the nonadaptive named
application slots and landmark relationships that `AdaptiveScaffold` can later reposition without
remounting slot owners, while leaving application policy and platform behavior outside the owner.
The sixty-fifth Gate 8 package adds `Scaffold` in
`crates/telorgon/src/application_components/structure/scaffold.rs` and the neutral Application, Banner,
Navigation, Main, Complementary, and Region roles required to express its landmark contract.
Construction validates the application name, nonempty slot names, unique slot kinds, and one
required primary Content slot, then canonicalizes the optional Navigation, Top, Secondary, Status,
FloatingAction, and Overlay slots independently from caller order. Mounting invokes caller content
once per present slot, publishes each slot under its typed landmark, and relates the named
Application root directly to every slot without owning route selection, overlays, focus, platform
services, or business state. Unit, mounted, direct-public, semantic, and umbrella compile fixtures
cover validation, canonical order, role mapping, ownership, and public paths. The exact next
eligible Gate 8 Slice 5 package is
`crates/telorgon/src/application_components/structure/adaptive_scaffold.rs`: it must derive an explicit
constraint/environment policy that repositions the same stable scaffold slot owners, reporting
typed presentation choices without remounting business content or taking over navigation, overlay,
or platform authority.
The sixty-sixth Gate 8 package adds `AdaptiveScaffold` in
`crates/telorgon/src/application_components/structure/adaptive_scaffold.rs`. Its validated breakpoint policy
classifies the accepted environment revision by constraint-bounded width divided by text scale,
distinguishes touch-primary input, and derives typed navigation rail/bar plus secondary
alongside/route/sheet presentations for the existing canonical Scaffold slots. Initial mounting
chooses an explicit compact or wider layout while delegating each slot's content exactly once to the
same named landmark owner. Later plan reconciliation validates an unchanged slot set and reports
only presentation changes with the existing mounted node identities and before/after environment
revisions; navigation component replacement, overlay execution, route mutation, and platform
adaptation remain caller-owned. Unit, mounted, direct-public, environment-revision, and umbrella
compile fixtures cover breakpoint validation, text scale, input capability selection, presentation
mapping, slot identity preservation, and public paths. The exact next eligible Gate 8 Slice 5
package is `crates/telorgon/src/application_components/range/range_slider.rs`: it must add two stable thumb
identities over one controlled bounded range, explicit clamp-versus-role-swap policy, independent
thumb semantics and keyboard/semantic actions, phased source-preserving proposals, and mounted plus
direct-public fixtures without duplicating the shared range model or applying caller state.
The sixty-seventh Gate 8 package adds focused `RangeSlider<T>` support in
`crates/telorgon/src/application_components/range/range_slider.rs`. One `Read<RangeSliderValue<T>>` supplies
an atomic ordered lower/upper snapshot, while two composed `SliderBehavior<T>` owners preserve the
existing normalization, direction, command, gesture-arena, and pointer-lifecycle rules under stable
Lower and Upper identities. Clamp policy restricts the requested thumb to its peer; Swap policy
keeps the ordered pair while explicitly reporting the new active logical thumb. Both paths return
Begin/Update/Commit/Cancel proposals with the originating change source and never mutate caller
state. The mounted group owns two independently named Slider nodes with separate focus,
increment/decrement/set-value actions, numeric values, clamp-aware semantic bounds, and
Compact/Standard/Touch targets. Unit, mounted, direct-public, and focused umbrella compile fixtures
cover crossing policy, source and phase preservation, cancellation, nonmutation, stable ownership,
semantics, density, and public paths. Mounted values, semantics, and visuals remain mount-time
snapshots; dependency-tracked controlled updates and platform semantic dispatch remain open. The
exact next eligible Gate 8 Slice 5 package is
`crates/telorgon/src/application_components/scroll/split_view.rs`: it must add a controlled, phased divider
with validated minimum sizes, keyboard/semantic resize and collapse/restore proposals, explicit
cancel restoration, stable pane/divider ownership, and mounted plus direct-public fixtures without
coupling the divider to `ScrollController` or applying caller state.
The sixty-eighth Gate 8 package adds focused `SplitView` support in
`crates/telorgon/src/application_components/scroll/split_view.rs` and the neutral named `Separator` role used
by its divider. `SplitViewConstraints` converts one finite pane extent plus primary and secondary
minimums into the shared bounded `RangeModel<f32>`; `SplitViewValue` atomically retains the last
expanded divider position and optional collapsed pane so restoration requires no component-owned
history. Horizontal and vertical behavior composes the existing `SliderBehavior<f32>` for stepped
keyboard, semantic, gesture-arena, and Begin/Update/Commit/Cancel resize proposals. An explicit
single-pane collapse policy returns source-preserving Collapse/Restore proposals while preventing
resize until the parent accepts restoration. Mounting invokes each caller content owner exactly
once under stable named Region layers, hides and marks only the controlled collapsed pane inert,
and gives the focusable named Separator numeric bounds plus increment/decrement/set-value and
collapse-or-expand actions at Compact/Standard/Touch density. Unit, mounted, direct-public, and
focused umbrella compile fixtures cover constraint rejection, axis mapping, nonmutation, collapse
restoration, pointer cancellation, ownership, semantics, density, and public paths. Mounted value,
semantics, visibility, and geometry remain mount-time snapshots; dependency-tracked controlled
updates, routed semantic action payloads, and platform dispatch remain open. The exact next eligible
Gate 8 Slice 5 package is `crates/telorgon/src/application_components/scroll/controller.rs`: it must wrap one
shared neutral scroll transition owner with application-domain offset, extent, anchor, activity,
reveal, drag, and caller-timed motion commands and typed outcomes, preserving atomic validation and
generational motion continuity without owning layout, input routing, a scheduler, or a background
animation thread.
The sixty-ninth Gate 8 package adds `ScrollController` in
`crates/telorgon/src/application_components/scroll/controller.rs`. It owns exactly one shared neutral
`ScrollState` and routes typed SetExtents, ScrollBy/ScrollTo, Reveal, BeginDrag/DragBy/EndDrag,
StepMotion, and Cancel commands into that owner. Typed controller outcomes retain each neutral
`ScrollUpdate` unchanged, including requested, consumed, and unconsumed deltas; before/after
metrics and activity; source; and Start/Continue/Stop scheduler handoff. The application path also
exposes immutable metrics, activity, velocity, diagnostics, and reveal-target queries while the
neutral owner continues to enforce finite geometry, per-axis end anchoring, reveal alignment,
reduced motion, boundary clamping, atomic rejection, and generational stale-motion checks. Unit,
direct-public prelude, and focused umbrella compile fixtures cover boundary handoff, source
preservation, invalid-request atomicity, extent anchoring, reveal, drag, cancellation, reduced
motion, scheduler intent, and motion-generation continuity. The controller mounts nothing and owns
no input route, capture, layout mutation, timer, frame scheduler, callback, task, or thread.
The seventieth Gate 8 package adds `ScrollView` in
`crates/telorgon/src/application_components/scroll/view.rs`. It captures one immutable metrics snapshot from
the application `ScrollController`, maps axis-specific forward and backward requests to unapplied
viewport-sized `ScrollBy` commands with Semantic source, and advertises only the directions that
the snapshot can consume. Mounting creates one focusable named Region on a stable Scroll node, one
stable Generic caller-content owner, an explicit Owns relationship, and one persistent offset
property initialized from the snapshot. Caller viewport/content styles and layout inputs are kept
unchanged except for that initial viewport offset, and caller content is invoked exactly once. Unit,
mounted, direct-public prelude, and focused umbrella compile fixtures cover vertical/horizontal
mapping, disabled and boundary availability, semantic action filtering, mount identity, accessible
ownership, initial offset, and caller layout preservation. The view owns no mutable scroll state,
input route, capture, gesture recognizer, layout application, timer, frame scheduler, callback,
task, or thread. Mounted metrics, semantics, and geometry remain mount-time snapshots; dependency-
tracked controller publication and platform semantic dispatch remain open.
The seventy-first Gate 8 package adds `ScrollBar` in
`crates/telorgon/src/application_components/scroll/scrollbar.rs` and the neutral named `ScrollBar` role plus
the discrete Pointer scroll source used by its drag-to-offset handoff. `ScrollBarModel` projects one
immutable controller metrics snapshot into axis offset, maximum, viewport/content extent, visible thumb
fraction, and normalized thumb position. Validated track geometry resolves the minimum-clamped
thumb extent, travel, and origin, while stateless thumb-origin mapping returns an absolute Pointer-
sourced `ScrollTo` command without retaining a grab offset or gesture lifecycle. Behavior returns
only applicable line, page, start/end, increment/decrement, scroll-forward/backward, set-value, and
absolute commands; full requested deltas remain intact so the existing controller continues to
report consumed and unconsumed boundary distance. Mounting creates one named focusable `ScrollBar`
numeric semantic node and stable track/thumb visual nodes, applies the 24/32/44 density floor to
the interaction owner, and preserves caller visual inputs around resolved axis geometry. Unit,
mounted, direct-public prelude, neutral-role, and focused umbrella compile fixtures cover both axes,
boundary handoff, semantic availability/value validation, pointer source, geometry clamping,
density, identity, and nonmutation. The component owns no mutable scroll state, input route,
capture, gesture recognizer, viewport layout mutation, timer, frame scheduler, task, or thread.
Mounted metrics, semantics, and geometry remain mount-time snapshots; dependency-tracked controller
publication and platform semantic dispatch remain open.
The seventy-second Gate 8 package adds `Separator` in
`crates/telorgon/src/application_components/structure/separator.rs`. `SeparatorGeometry` rejects nonfinite or
nonpositive length and thickness, while the separate decorative and named constructors prevent a
meaningful divider from carrying an empty accessible name. Horizontal and vertical mounting resolve
only the line's width and height, preserve all other caller `BoxStyle` and `LayoutStyle` inputs, and
create one stable Box identity with no focus, actions, listeners, input routes, or component state.
Decorative instances attach an explicitly excluded `Separator` semantic record; named instances
attach a nonfocusable, action-free `Separator` record. Unit, mounted, direct-public prelude, and
focused umbrella compile fixtures cover validation, both policies and orientations, semantic
participation, caller style/layout preservation, stable identity, and public paths. The component
owns no adjacent layout, interaction lifecycle, navigation, scheduler, task, or thread. The exact
next eligible Gate 8 Slice 5 package is
`crates/telorgon/src/application_components/content/image_view.rs`: it must mount one stable caller-supplied
image identity with explicit decorative-versus-described semantic policy, preserve caller sizing
and visual style, and expose described content as nonfocusable action-free image semantics without
owning asset loading, decoding, caching, network/platform services, or renderer fit policy.
The seventy-third Gate 8 package adds `ImageView` in
`crates/telorgon/src/application_components/content/image_view.rs` and one narrow retained-image-under-host
foundation entry point. `ImageViewContent` carries the caller's `ImageId` and opaque `u64` content
version unchanged; converting directly from `ImageId` retains the existing initial-version `1`
convenience without hiding the explicit constructor. Separate decorative and described constructors
make semantic participation deliberate and reject an empty accessible description. Mounting creates
one stable Image identity with the exact `ImageVisual`, `BoxStyle`, and `LayoutStyle` inputs, no
focus, actions, listeners, routes, or component state. Decorative images attach an explicitly
excluded Image semantic record, while described images attach a nonfocusable, action-free named
Image record. Unit, mounted, direct-public prelude, and focused umbrella compile fixtures cover
version identity, both semantic policies, description validation, style/layout preservation,
noninteraction, stable ownership, and public paths. Neither the component nor its foundation seam
loads, decodes, caches, fits, uploads, fetches, or presents image resources. The exact next eligible
Gate 8 package is `crates/telorgon/src/application_components/text/label.rs`: it must mount one stable visible
text identity with validated nonempty content, preserve caller text revision, visual style, and
layout inputs, expose nonfocusable action-free Text semantics derived from that visible content,
and provide a stable label node suitable for semantic relationships without owning wrapping,
selection, editing, localization, shaping, or font loading.
The seventy-fourth Gate 8 package adds `Label` in
`crates/telorgon/src/application_components/text/label.rs` and one narrow retained-text-under-host foundation
entry point. `LabelContent` rejects whitespace-only text and carries the caller's opaque `u64`
content revision; the `Label::new` convenience uses initial revision `1`. `LabelTextStyle`
validates finite positive size and line height, a nonempty family identifier, and weight in
`1..=1000`, then resolves that family through the existing mounted string pool without loading a
font. Mounting creates one stable Text identity with the exact `TextVisual`, `BoxStyle`, and
`LayoutStyle` inputs and reuses the same interned visible-content ID as its nonfocusable, action-free
Text semantic name. `LabelRef::labelled_by` returns the generation-checked relationship target for
consumer semantics without adding a route or second ownership model. Unit, mounted, direct-public
prelude, and focused umbrella compile fixtures cover validation, revision identity, text metrics,
family/content interning, box/layout preservation, semantic identity, noninteraction, and public
paths. The component owns no wrapping, selection, editing, localization, shaping, font loading,
task, or thread. The exact next eligible Gate 8 package is
`crates/telorgon/src/application_components/text/selectable_text.rs`: it must reuse `LabelContent` and label
visuals, validate a caller-controlled `TextSelection` at UTF-8 scalar boundaries, emit only
source-preserving selection proposals without mutating the caller value, and expose one focusable
Text semantic owner with set-selection capability while leaving editing, clipboard execution,
wrapping geometry, platform selection handles, shaping, and font loading outside the component.
The seventy-fifth Gate 8 package adds `SelectableText` in
`crates/telorgon/src/application_components/text/selectable_text.rs`. `SelectableTextBehavior` reuses one
validated `LabelContent`, rejects content outside the shared `u32` text-offset space, validates both
current and requested `TextSelection` endpoints at UTF-8 scalar boundaries, suppresses unchanged or
disabled requests, and returns only committed `ValueChange<TextSelection>` values with the exact
caller source. Its select-all convenience uses the same validation and never changes the caller's
selection. The mounted component consumes `Read<TextSelection>`, rejects an invalid initial value
before creating its node, reuses `LabelStyle` and the exact visible-content revision, and mounts one
enabled/focusable retained Text identity. Its read-only Text semantics reuse the visible content as
name and value and advertise only Focus plus SetSelection; disabled instances advertise neither.
The focused reference exposes the controlled read and patchable enabled/box-style properties, while
text-layout hit testing and semantic payload dispatch remain caller/platform seams. Unit, mounted,
direct-public prelude, and focused umbrella compile fixtures cover multibyte boundaries, invalid
current/requested values, no-op/disabled behavior, select-all, source and nonmutation, initial
atomicity, semantic capabilities, focusability, revision identity, and public paths. The component
owns no editing buffer, clipboard action, wrapping geometry, selection handle, shaping, font load,
task, or thread. The exact next eligible Gate 8 package is
`crates/telorgon/src/application_components/command/menu_button.rs`: it must reuse canonical Button
activation and density behavior, derive a source-preserving typed menu-open request against one
explicit stable anchor, expose expanded/collapsed named Button semantics without creating a second
menu lifecycle owner, and leave popup placement, overlay execution, command dispatch, and platform
menu services to the existing menu/overlay controllers.
The seventy-sixth Gate 8 package adds `MenuButton` in
`crates/telorgon/src/application_components/command/menu_button.rs`. It validates a nonempty item snapshot
with unique keys and a known optional selected key, then composes the canonical `Button` for
enabled/busy policy, state-style resolution, density-floor mounting, and pointer/keyboard/semantic
activation. `MenuButtonOpenRequest<K>` retains the exact activation source beside an unapplied root
`MenuOpenRequest<K>` whose node anchor is the stable mounted button identity. A caller-published
expanded snapshot amends the same named Button semantic record; the component does not open or
mutate a menu, allocate an overlay, place a popup, dispatch a command, or invoke platform menu
services. Unit, mounted, direct-public prelude, and focused umbrella compile fixtures cover
validation, selected/opening-focus preservation, activation source, stable anchoring, expanded
semantics, density reuse, nonmutation, and public paths. Mounted inputs and expanded semantics
remain mount-time snapshots pending dependency-tracked publication. The exact next eligible Gate 8
package is `crates/telorgon/src/application_primitives/root.rs`: it must establish one explicit named
Application semantic scope and stable caller-content ownership for a view, preserve caller visual
and layout inputs, and expose the scope identity needed by later content/navigation/status region
primitives without owning window creation, theme/environment state, overlay lifecycle, focus
policy, platform services, business state, or Scaffold slot policy.
The seventy-seventh Gate 8 package adds `ApplicationRoot` in
`crates/telorgon/src/application_primitives/root.rs`. It validates one explicit accessible name, mounts a
stable noninteractive Application semantic node and a separate merge-only caller-content host,
and relates the application directly to that host through one generation-checked Owns
relationship. Both nodes preserve their complete caller-supplied box and layout inputs and expose
focused property/identity references. Unit, mounted, direct-public prelude, and umbrella compile
fixtures cover validation, naming, semantics, ownership, identity, noninteraction, and style/layout
preservation. The primitive does not create a window, own environment/theme/overlay/focus/business
state, enforce global root uniqueness, or define Scaffold slots. The exact next eligible Gate 8
package is `crates/telorgon/src/application_primitives/region.rs`: it must provide explicitly named stable
Content, Navigation, and Status semantic landmarks under an application content host, preserve
caller visual/layout inputs, and leave slot ordering, adaptive presentation, focus, routing, and
business content ownership to callers and higher-level components.
The seventy-eighth Gate 8 package adds `ApplicationRegion` in
`crates/telorgon/src/application_primitives/region.rs`. Its closed `ApplicationRegionKind` maps Content,
Navigation, and Status to the existing Main, Navigation, and Status semantic roles, while separate
convenience constructors still share one nonempty-name validation path. Mounting creates one stable
empty caller-content host with exact box/layout inputs and named, nonfocusable, action-free landmark
semantics. It has no Scaffold slot ordering, required-content, adaptive presentation, layout,
routing, selection, or business-state policy. Unit, mounted, direct-public prelude, and umbrella
compile fixtures cover all kind/role mappings, name rejection, stable composition under an
Application root, noninteraction, style/layout preservation, and public paths. The exact next
eligible Gate 8 package is `crates/telorgon/src/application_primitives/ext.rs`: it must expose mount-only
application root and region conveniences over the existing component-runtime `Ui` without moving
domain implementations into the runtime or reversing the components-to-primitives dependency.
The seventy-ninth Gate 8 package adds primitive `ApplicationUiExt<Action>` conveniences in
`crates/telorgon/src/application_primitives/ext.rs` plus the primitive-domain prelude. Its root and region
methods delegate to the same validated primitive owners and return the same stable focused
references; no alternative node, semantics, layout, action, or lifecycle owner is introduced. The
trait remains primitive-only so `telorgon-components-application` continues to depend on primitives
rather than creating a package cycle. Direct-public mounted and umbrella compile fixtures cover
trait discovery, nested root/region composition, stable identities, and semantic-role preservation.
The exact next eligible Gate 8 package is `crates/telorgon/src/application_primitives/hud_layer.rs`: it must
define validated application-domain HUD coordinate and pointer pass-through policy, mount one
stable caller-content layer with explicit semantic participation, and leave projection, hit
testing, capture, frame scheduling, scene submission, and game-engine state to their existing
owners.
The eightieth Gate 8 package adds `HudLayer` in
`crates/telorgon/src/application_primitives/hud_layer.rs`. `HudCoordinateSpace` distinguishes host-logical
coordinates from a validated finite positive reference extent and resolves reference points against
an explicit host viewport without reading layout or renderer state. `HudHitTestPolicy` keeps
pass-through versus content eligibility separate from `HudSemanticPolicy` include-versus-exclude
participation. Mounting creates one stable empty caller-content host with exact box/layout inputs
and the selected semantic participation, but installs no listener, route, recognizer, capture, hit
tester, frame work, scene submission, or game-state poll. Unit, mounted, direct-public prelude, and
umbrella compile fixtures cover invalid geometry, coordinate resolution, independent hit/semantic
policy, identity, style/layout preservation, noninteraction, and public paths. The exact next
eligible Gate 8 package is `crates/telorgon/src/application_primitives/viewport_overlay.rs`: it must validate
one host-provided viewport region and viewport-relative anchor, mount stable caller content at the
resolved logical position without altering its visual/layout inputs, and leave viewport rendering,
clipping, hit testing, focus, capture, and scene scheduling outside the primitive.
The eighty-first Gate 8 package adds `ViewportOverlay` in
`crates/telorgon/src/application_primitives/viewport_overlay.rs`. `ViewportOverlayPlacement` atomically
validates a finite positive host `RectF`, normalized anchor in `0..=1` on both axes, finite logical
offset, and finite resolved anchor. Mounting applies that position only to an implementation-owned
outer transform node, preserves the caller's complete content box/layout inputs on a separate
stable host, and merges descendant semantics without adding focus, actions, listeners, routes, or
capture. The primitive does not own the viewport renderer, clip, overflow solver, hit testing,
layout measurement, frame scheduling, or scene submission. Unit, mounted, direct-public prelude,
and umbrella compile fixtures cover geometry rejection, edge/center resolution, stable identity,
position/content separation, semantics, noninteraction, and public paths. The exact next eligible
Gate 8 package is `crates/telorgon/src/application_primitives/world_anchor.rs`: it must consume one validated
host-projected transform, visibility classification, and depth hint, mount stable caller content
with matching visual/semantic visibility, and never read a game camera, world transform, GPU, depth
buffer, renderer, or frame loop.
The eighty-second Gate 8 package adds `WorldAnchor` in
`crates/telorgon/src/application_primitives/world_anchor.rs` plus one narrow retained visibility-layer-under-
host foundation entry point. `WorldAnchorProjection` retains a finite translation, finite positive
2D scale, explicit Visible/Occluded/OutsideViewport classification, and finite depth hint without
constraining the host's depth convention. Mounting applies only the host projection to an outer
stable layer, reflects nonvisible classifications in both retained visibility and hidden merged
semantics, and preserves exact caller content box/layout inputs on a separate host. The focused
reference exposes visibility and style properties plus the unchanged projection snapshot. Unit,
mounted, direct-public prelude, foundation, and umbrella compile fixtures cover invalid projection
inputs, all visibility states, transform/depth retention, semantic hiding, content preservation,
noninteraction, and public paths. The primitive reads no camera, world state, depth buffer, GPU,
renderer, frame clock, scheduler, or scene submission. The exact next eligible Gate 8 package is
`crates/telorgon/src/application_primitives/render_target_view.rs`: it must retain an opaque revisioned
host-owned render-target content token, mount one stable noninteractive application content identity
with explicit semantic policy and caller sizing, and leave target allocation/import,
synchronization, rendering, submission, presentation, and platform/backend ownership to the host.
The eighty-third Gate 8 package adds `RenderTargetView` in
`crates/telorgon/src/application_primitives/render_target_view.rs`. `RenderTargetToken` is an opaque nonzero
host identity rather than an `ImageId`, GPU/native handle, or ownership transfer, and
`RenderTargetViewContent` requires a nonzero content version. Separate decorative and described
constructors make Image semantic participation explicit and reject an empty description. Mounting
creates one stable action-free Box with exact caller box/layout inputs and deliberately does not
insert an image visual or renderer resource; the typed focused reference retains the unchanged host
content for later integration. Unit, mounted, direct-public prelude, and umbrella compile fixtures
cover token/version validation, semantic policies, identity, style/layout preservation,
noninteraction, absence of fabricated images, extension mounting, and public paths. The primitive
does not allocate, import, synchronize, render, submit, present, release, or inspect a target. The
exact next eligible Gate 8 package is `crates/telorgon/src/application_primitives/video_surface.rs`: it must
retain one revisioned opaque host media token with validated frame size and explicit fit, color, and
protection metadata, mount a stable noninteractive identity with deliberate semantics, and leave
decoding, playback, import, synchronization, protection enforcement, and presentation to the host.
The eighty-fourth Gate 8 package adds `VideoSurface` in
`crates/telorgon/src/application_primitives/video_surface.rs`. Its opaque nonzero `VideoSurfaceToken` and
nonzero content version are paired with a positive integer frame size, typed color primaries,
transfer function and range, explicit Contain/Cover/Fill/None fit, and Unprotected/Protected host
metadata. Protection is retained as an assertion only; no capability or enforcement claim follows.
Decorative and described constructors mount one stable action-free Box with exact caller box/layout
inputs and no fabricated image visual. Unit, mounted, direct-public prelude, and umbrella compile
fixtures cover invalid revision/size, fit/color/protection retention, semantics, identity,
noninteraction, absence of image resources, extension mounting, and public paths. The primitive
owns no decoder, demuxer, playback clock, queue, native handle, import, synchronization, protected
memory, capture prevention, renderer, submission, or presentation. The exact next eligible Gate 8
package is `crates/telorgon/src/application_primitives/diagnostics.rs`: it must provide bounded typed
diagnostics for primitive validation and host-content availability without retaining sensitive
descriptions, opaque host tokens, coordinates, media metadata, or unbounded event payloads.
The eighty-fifth Gate 8 package adds `ApplicationPrimitiveDiagnosticCollector` and immutable
`ApplicationPrimitiveDiagnostics` in `crates/telorgon/src/application_primitives/diagnostics.rs`. A closed
eight-kind vocabulary covers environment, HUD, viewport, world projection, render-target, video,
protected-video availability, and stale-host-content boundaries. Fixed `u64` counters saturate,
support typed error conversion, iterate in deterministic order, snapshot by value, and clear
without allocating or retaining any event payload. Unit and direct-public prelude/umbrella fixtures
cover conversion, counting, saturation, deterministic bounded size, clearing, and public paths. The
collector owns no logger, telemetry transport, callback, host resource, content description,
coordinate, media metadata, task, or thread. This completes the currently specified
`telorgon-primitives-application` source set. The exact next eligible Gate 8 package is
`crates/telorgon/src/shell/id.rs`: it must establish protocol-neutral, nonzero, strongly typed output,
surface, workspace, and application identities in the new shell model crate without embedding
native handles, protocol objects, renderer/runtime ownership, allocation policy, or application-
domain dependencies.
The eighty-sixth Gate 8 package creates the protocol-neutral `telorgon-shell` model crate and adds
`id.rs`. `OutputId`, `SurfaceId`, `WorkspaceId`, and `ApplicationId` are distinct transparent
nonzero host values with checked raw construction, deterministic ordering/hashing/display, and no
sentinel default. Unit, direct-public, and umbrella fixtures cover zero rejection, value retention,
domain-distinct paths, and the nonzero option niche. These values embed no index/generation layout,
native handle, display-protocol object, renderer/runtime owner, allocation policy, or application-
domain dependency. The exact next eligible Gate 8 package is `crates/telorgon/src/shell/capability.rs`: it
must represent host-granted request/layer capabilities and make a privileged layer require a typed,
output-scoped authority derived from an applicable grant rather than a layer enum alone.
The eighty-seventh Gate 8 package adds `ShellCapabilities`, `ShellCapabilityGrant`, and
`LayerAuthority` in `crates/telorgon/src/shell/capability.rs`. The bounded validated bit set distinguishes
surface/workspace/output/system operations from seven canonical back-to-front layer classes. One
opaque nonzero host grant is scoped to one output and can narrow only a granted layer bit into a
privately constructed authority token; a layer enum alone is insufficient. Unit, direct-public,
and umbrella fixtures cover unknown-bit rejection, set operations, canonical order, output/grant
retention, accepted narrowing, and denied privileged layers. Grants remain host assertions rather
than a security boundary: the host must still validate every identity, revision, session, and
policy precondition. The package owns no policy engine, protocol permission, renderer access,
request execution, layer content, event loop, task, or thread. The exact next eligible Gate 8
package is `crates/telorgon/src/shell/model/output.rs`: it must publish immutable revisioned output identity,
logical/usable/physical geometry, scale, transform, safe insets, and color capabilities while
rejecting invalid or contradictory geometry before shell consumers observe it.
The eighty-eighth Gate 8 package adds `OutputSnapshot` in
`crates/telorgon/src/shell/model/output.rs`. A nonzero `OutputRevision` accompanies one stable `OutputId` and
validated `OutputGeometry`: finite positive logical and usable rectangles, containment of usable
space, positive physical extent, finite positive scale, the eight orientation/reflection cases,
finite nonnegative safe insets that leave drawable space, and a closed color-capability bit set.
The model also derives safe logical bounds without rewriting the host's independent usable area or
asserting exact physical/logical rounding. Unit, direct-public, and umbrella fixtures cover
identity/revision retention, multi-output coordinates, usable containment, scale/inset rejection,
color sets, transform axis classification, immutability, and public paths. It owns no display
enumeration, mode setting, native/protocol object, color conversion, compositor policy, renderer,
window, event loop, task, or thread. The exact next eligible Gate 8 package is
`crates/telorgon/src/shell/model/surface.rs`: it must define an immutable revisioned host surface snapshot
with explicit parentage, logical/buffer geometry, regions, opacity/transform, opaque external-
content and synchronization references, color/protection metadata, and bounded damage, without
importing protocol or backend-native ownership.
The eighty-ninth Gate 8 package adds `ClientSurfaceSnapshot` in
`crates/telorgon/src/shell/model/surface.rs`. Separate nonzero snapshot/content revisions accompany stable
surface/application parentage, finite positive logical and buffer geometry, scale, the eight buffer
transforms, bounded opacity, typed state/capability sets, and an optional bounded title. Logical
clip/opaque/input regions use immutable bounded arrays and must stay within surface-local bounds;
buffer-pixel damage is independently bounded and must stay within the buffer. `SurfaceContent`
retains redacted-debug opaque external-content/synchronization identities plus sampling, alpha,
color, and protection metadata without exposing a native semaphore or import handle. Unit,
direct-public, and umbrella fixtures cover host fact retention, self-parent rejection, region and
damage validation, bounded arrays, state/capability sets, transform/color/protection values, and
public paths. It owns no protocol object, mutable surface world, import lease, buffer lifetime,
renderer, submission, release signaling, focus/window policy, task, or thread. The exact next
eligible Gate 8 package is `crates/telorgon/src/shell/model/workspace.rs`: it must retain immutable named,
ordered workspace truth with unique host-provided surface membership and explicit output-relative
geometry, without choosing membership, tiling, stacking, focus, or activation policy.
The ninetieth Gate 8 package adds `WorkspaceSnapshot` in
`crates/telorgon/src/shell/model/workspace.rs`. A stable workspace identity, nonzero revision, explicit host
order, bounded nonempty name, and active flag accompany an immutable bounded back-to-front surface
list. Every `WorkspaceSurface` supplies a stable surface/output identity and finite positive bounds;
duplicate surface membership is rejected without sorting or rewriting host order. Empty workspaces
remain valid. Unit, direct-public, and umbrella fixtures cover name/geometry validation, identity,
revision/order/active retention, exact surface order, lookup, duplicate rejection, and public paths.
The package owns no workspace creation/removal, membership decision, tiling/floating algorithm,
focus, activation, layout solver, protocol object, task, or thread. The exact next eligible Gate 8
package is `crates/telorgon/src/shell/model/application.rs`: it must retain bounded host-described
application/launcher labels, approved logical assets, state, and typed action identities without
discovering processes, resolving executables, or invoking an action.
The ninety-first Gate 8 package adds `ApplicationEntry` in
`crates/telorgon/src/shell/model/application.rs`. Stable application identity and a nonzero revision retain a
bounded nonempty label, optional bounded description, opaque logical icon identity, closed
running/active/urgent/pinned state bits, and at most 32 ordered typed actions. Each action has a
nonzero host identity, Launch/Activate/NewInstance/Custom kind, bounded label, and enabled fact;
duplicates and an optional primary action absent from that list are rejected. Unit, direct-public,
and umbrella fixtures cover text bounds, state, asset and action retention, order, uniqueness,
primary referential integrity, and public paths. The package owns no process/application discovery,
executable path, launch mechanism, icon loading, search ranking, request dispatch, optimistic state,
platform service, task, or thread. The exact next eligible Gate 8 package is
`crates/telorgon/src/shell/model/notification.rs`: it must retain bounded revisioned host notification
identity, privacy-safe presentation fields, priority/lifecycle facts, and typed action IDs without
owning service discovery, delivery, persistence, dismissal, or sensitive diagnostic payloads.
The ninety-second Gate 8 package adds `NotificationSnapshot` in
`crates/telorgon/src/shell/model/notification.rs`. Stable notification identity and nonzero revision retain
an optional source application, bounded title/body, optional logical icon, Low/Normal/High/Critical
priority, Public/Sensitive/Secret privacy, transient/persistent and new/presented/acknowledged
lifecycle facts, and at most 16 ordered typed actions. Notification and action text always redacts
from `Debug`; duplicate action identities are rejected. Unit, direct-public, and umbrella fixtures
cover bounds, revision/source metadata, privacy/priority/lifecycle retention, action ordering and
uniqueness, lookup, and redaction. The package owns no notification service, delivery, storage,
expiry clock, dismissal, reply editor, action dispatch, platform integration, task, or thread. The
exact next eligible Gate 8 package is `crates/telorgon/src/shell/model/status.rs`: it must retain an ordered,
bounded revisioned snapshot of host-approved indicators, media/session summaries, extension
entries, privacy facts, and typed actions without querying or mutating a system service.
The ninety-third Gate 8 package adds `SystemStatusSnapshot` in
`crates/telorgon/src/shell/model/status.rs`. Its nonzero revision retains at most 128 exact-order entries for
clock, connectivity, power, audio, input, media, session, privacy, and approved extension kinds.
Each stable entry carries bounded debug-redacted label/value text, an optional logical icon,
Normal/Attention/Critical/Unavailable severity, Public/Sensitive/Secret privacy, active state, and
at most 16 unique typed actions with a referentially valid optional primary action. Entry IDs and
action IDs are globally unique within a snapshot so later system requests remain unambiguous. Unit,
direct-public, and umbrella fixtures cover ordering, bounds, kinds, state/privacy/severity, lookup,
duplicate rejection, primary integrity, and redaction. It owns no clock, network/power/audio/media
query, session manager, extension runtime, service mutation, action dispatch, task, or thread. The
exact next eligible Gate 8 package is `crates/telorgon/src/shell/model/accessibility.rs`: it must identify an
optional host-provided semantic attachment for a surface with an explicit namespace, revision,
coordinate mapping, keyboard/assistive focus ownership, and privacy without copying a platform
tree, fabricating semantics from pixels, or exporting it directly.
The ninety-fourth Gate 8 package adds `ImportedAccessibilityAttachment` in
`crates/telorgon/src/shell/model/accessibility.rs`. Opaque nonzero attachment identity, nonzero revision,
surface identity, namespace, and imported root node accompany a finite invertible six-coefficient
affine mapping, distinct optional keyboard and assistive-technology focus nodes, and
Ordinary/Redacted privacy. The attachment ID redacts from `Debug`; actual role/string/action/tree
payload stays with the later `telorgon-accessibility` owner and host channel. Unit, direct-public, and
umbrella fixtures cover identity/namespace/revision retention, independent focus, translation/
scale and determinant preservation, nonfinite/singular rejection, privacy, redaction, and public
paths. The package owns no OCR, semantic-tree storage/merge, platform accessibility object, export,
action dispatch, focus mutation, protocol object, task, or thread. The exact next eligible Gate 8
package is `crates/telorgon/src/shell/request/result.rs`: it must define typed immediate request acceptance,
denied/stale/unsupported outcomes and opaque accepted request identity without optimistically
mutating any shell snapshot or conflating later Gate 9 terminal platform completion.
The ninety-fifth Gate 8 package adds `ShellRequestResult` and `AcceptedRequestId` in
`crates/telorgon/src/shell/request/result.rs`. The closed immediate result distinguishes Accepted, Denied,
Stale, and Unsupported admission; accepted values retain one nonzero typed host identity for later
correlation. Admission never reports Applied/Failed/Cancelled terminal platform completion and does
not authorize optimistic shell-snapshot mutation. Unit, direct-public, and umbrella fixtures cover
zero rejection, accepted identity retention, and the three identity-free rejection outcomes. The
package owns no request queue, terminal completion, platform error, callback, event loop, task, or
thread. The exact next eligible Gate 8 package is `crates/telorgon/src/shell/request/input.rs`: it must retain
validated neutral pointer/touch/pen lifecycle in surface-local logical coordinates plus stable typed
seat/contact/source identity without embedding a native event, protocol serial, grab, client
dispatch, or user-gesture authority.
The ninety-sixth Gate 8 package adds neutral client-input requests in
`crates/telorgon/src/shell/request/input.rs`. Nonzero `SeatId` and `ContactId` plus a closed `InputSource`
identify validated pointer-like streams. `SurfaceInputEvent` retains Entered/Moved/Button/Scrolled/
Left/Cancelled lifecycle with finite surface-local coordinates and scroll deltas, rejects noncontact
sources and the zero button code, and `ClientInputRequest` addresses exactly one stable surface.
Unit, direct-public, and umbrella fixtures cover identity/source retention, local button delivery,
nonfinite rejection, and coordinate-free leave/cancellation. The package owns no native event,
protocol serial, user-gesture grant, implicit grab, routing policy, client delivery, callback, task,
or thread. The exact next eligible Gate 8 package is `crates/telorgon/src/shell/request/surface.rs`: it must
retain typed surface activate/close/move/resize/minimize/maximize/fullscreen intentions and expose
their exact shell-grant and per-surface capability requirements without applying policy or changing
a surface snapshot.
The ninety-seventh Gate 8 package adds `SurfaceRequest` in
`crates/telorgon/src/shell/request/surface.rs`. Seven typed variants preserve stable target identity,
activation source, move/resize contact, all eight logical resize edges, and explicit requested
boolean state. Each maps to both its host-grant `ShellCapabilities` bit and advertised
`SurfaceCapabilities` bit; the shell grant vocabulary now includes the previously reserved
maximize/fullscreen operation bits. Unit, direct-public, and umbrella fixtures cover exact target,
authority, capability, source, contact, and edge retention. Constructing a request performs no
optimistic snapshot mutation, capability validation, focus/geometry/state change, protocol call,
native grab, event dispatch, task, or thread. The exact next eligible Gate 8 package is
`crates/telorgon/src/shell/request/workspace.rs`: it must retain typed workspace selection, surface movement,
reordering, and capability-gated creation/removal intentions with stable IDs and host-provided
ordering/revisions, without applying membership, focus, layout, or lifecycle policy.
The ninety-eighth Gate 8 package adds `WorkspaceRequest` in
`crates/telorgon/src/shell/request/workspace.rs`. Select cites stable identity, observed revision, and input
source under `SELECT_WORKSPACE`; surface movement retains both source and destination workspace
identities/revisions; reorder retains the observed revision and requested `u32` order; removal cites
the exact observed workspace; and creation retains only a validated bounded `WorkspaceName` and
requested order without fabricating a host identity. All management variants require
`MANAGE_WORKSPACES`. Unit, direct-public, and umbrella fixtures cover authority separation, both
move revisions, selection source, order, and identity-free creation. The package owns no membership,
focus, layout, lifecycle, workspace allocator, policy engine, protocol object, task, or thread. The
exact next eligible Gate 8 package is `crates/telorgon/src/shell/request/output.rs`: it must retain validated
revisioned reserved-area proposals/releases plus opaque host-exposed appearance/mode actions and
their exact authority without rewriting usable geometry or inventing display policy.
The ninety-ninth Gate 8 package adds `OutputRequest` in
`crates/telorgon/src/shell/request/output.rs`. Stable nonzero `ReservedAreaId` identifies proposal updates;
`OutputEdge` fixes four logical edges; and `ReservedAreaExtent` rejects nonfinite/nonpositive values.
Propose/release variants cite one exact `OutputRevision` and require `RESERVE_OUTPUT_AREA` while
opaque typed appearance/mode action IDs require `CONFIGURE_OUTPUT`. Unit, direct-public, and
umbrella fixtures cover extent rejection, identity/revision/edge retention, and authority mapping.
The package owns no accepted usable geometry, display enumeration, mode/color choice, reservation
policy, protocol object, platform call, task, or thread. The exact next eligible Gate 8 package is
`crates/telorgon/src/shell/request/system.rs`: it must bind launcher/application, notification, and status
action IDs to their exact parent snapshot identities/revisions and source without invoking services
or optimistically changing any entry.
The one-hundredth Gate 8 package adds `SystemRequest` in
`crates/telorgon/src/shell/request/system.rs`. Application and notification actions retain parent identity,
revision, typed action ID, and input source; status actions retain the complete status revision,
entry identity, globally unique action ID, and source. Every variant requires
`INVOKE_SYSTEM_ACTION`, leaving enabled-state, staleness, session/lock, and causality validation to
the host. Unit, direct-public, and umbrella fixtures cover exact typed associations, revisions,
sources, and authority. The package owns no application launch, notification dismissal/reply,
status mutation, service query, protocol call, callback, task, or thread. This completes the six
currently specified shell request source files. The exact next eligible Gate 8 package is
`crates/telorgon/src/shell/host.rs`: it must define the protocol-neutral snapshot/request transport boundary
over the existing typed models, grants, requests, and immediate results, with deterministic fake/
trace fixtures and no native types, hidden execution, callback reentrancy, policy implementation,
task, or thread.
The one-hundred-first Gate 8 package adds `ShellSnapshot` and `ShellHost` in
`crates/telorgon/src/shell/host.rs`. One nonzero publication revision atomically retains bounded immutable
grants, outputs, surfaces, workspaces, applications, notifications, system status, and imported
accessibility attachments. Validation rejects duplicate top-level identities, unknown grant
outputs, missing/cyclic surface parents, invalid workspace surface/output references, and dangling
accessibility surfaces before publication. The executor-neutral trait returns cloned immutable
snapshots and preserves five separate typed request methods with immediate results. Unit,
direct-public, and umbrella fixtures cover empty publication, uniqueness, deterministic trace
category/order, and trait paths. The package owns no reactive runtime read, subscription callback,
policy validation, request queue/execution, native/protocol type, event loop, task, or thread. The
exact next eligible Gate 8 package is `crates/telorgon/src/shell/diagnostics.rs`: it must count bounded stable
snapshot/request/outcome categories without retaining identities, labels, coordinates, content,
errors, event payloads, or an unbounded log.
The one-hundred-second Gate 8 package adds `ShellDiagnosticCollector` and immutable
`ShellDiagnostics` in `crates/telorgon/src/shell/diagnostics.rs`. Twelve closed categories cover snapshot
publication/rejection, each request family, each immediate result, and host errors. Fixed `u64`
counters saturate, iterate deterministically, snapshot by value, and clear without retaining any
payload. Unit, direct-public, and umbrella fixtures cover category/result mapping, bounded order,
counting, and clearing. The package owns no logger, telemetry transport, host identity, request,
error payload, callback, task, or thread. The exact next eligible Gate 8 package is
`crates/telorgon/src/shell/error.rs`: it must provide structured redaction-safe shell boundary errors whose
closed kind is safe for control flow and whose diagnostic context cannot retain dynamic host/user
content.
The one-hundred-third Gate 8 package adds `ShellError`, `ShellErrorKind`, and `ShellResult<T>` in
`crates/telorgon/src/shell/error.rs`. Seven closed kinds distinguish invalid snapshots, denied/stale/
unsupported requests, host unavailability, capacity, and invariant failures; context is restricted
to `&'static str`. Immediate rejections map without display-string branching, accepted requests do
not fabricate errors, and snapshot validation converts without retaining its identity payload.
Unit, direct-public, and umbrella fixtures cover typed mapping and static context. The package owns
no platform error object, dynamic payload, retry policy, logging, callback, task, or thread. This
completes the currently specified `telorgon-shell` source set. The exact next eligible Gate 8 package
is `crates/telorgon/src/shell_primitives/root.rs`: it must establish one explicitly named output-scoped shell
semantic owner and stable caller-content host, retain one host grant, and narrow only granted layer
authority without creating a window, policy host, overlay/focus owner, native object, task, or
thread.
The one-hundred-fourth Gate 8 package creates `telorgon-primitives-shell` and adds `ShellRoot` in
`src/root.rs` with its crate/prelude scaffolding. It mounts one named nonfocusable Region semantic
scope plus one merge-only content host related through generational Owns, preserves complete caller
box/layout inputs, retains exactly one output-scoped host grant, and exposes only the existing
grant's checked layer narrowing. Unit, mounted direct-public, and umbrella fixtures cover naming,
output/grant retention, semantic ownership, stable identities, granted/denied layers, and public
paths. The primitive owns no window, global root registry, theme/environment state, overlay/focus
domain, policy, protocol object, task, or thread. The exact next eligible Gate 8 package is
`crates/telorgon/src/shell_primitives/output_view.rs`: it must map one immutable output snapshot into
output-local logical, usable, and safe coordinates under the matching shell root while preserving
scale/transform/revision and leaving display configuration, layout policy, rendering, and native
conversion outside the primitive.
The one-hundred-fifth Gate 8 package adds `OutputView` in
`crates/telorgon/src/shell_primitives/output_view.rs`. It requires its mounted shell root to target the same
output, mounts one stable merge-only caller-content host, preserves box/layout inputs, and retains
the exact output identity/revision/scale/transform. Pure finite mapping subtracts/adds the host
logical origin and derives local logical/usable/safe rectangles without applying physical-pixel or
rotation conversions. Unit, direct-public, mounted layer, and umbrella fixtures cover negative and
positive multi-output origins, safe/usable mapping, round trips, nonfinite rejection, root matching,
identity, and public paths. The primitive owns no display enumeration/configuration, layout policy,
renderer transform, hit route, platform conversion, task, or thread. The exact next eligible Gate 8
package is `crates/telorgon/src/shell_primitives/layer.rs`: it must mount only narrowed layer authority for
the same output in canonical back-to-front order, permit omitted layers, reject duplicates/reversal,
and leave lock/modal inertness, focus/input routing, painting, and security enforcement outside the
primitive.
The one-hundred-sixth Gate 8 package adds `ShellLayer`, `ShellLayerOrder`, and focused references in
`crates/telorgon/src/shell_primitives/layer.rs`. Each stable merge-only caller-content host retains one
privately constructed `LayerAuthority`, exact output/kind, and caller box/layout inputs.
`ShellLayerOrder` is scoped to one output, permits skipped kinds, commits only after successful
mount, and rejects cross-output authority, duplicates, and backtracking against the canonical seven
kind order. Unit, mounted direct-public prelude, and umbrella fixtures cover grant narrowing,
background-to-panel composition, skipped order, duplicate/reverse rejection, identity, style, and
public paths. The primitive owns no barrier/inert state, lock/session policy, focus/input routing,
painting order engine, security boundary, protocol object, task, or thread. The exact next eligible
Gate 8 package is `crates/telorgon/src/shell_primitives/client_surface.rs`: it must mount one exact immutable
client-surface revision as renderer-neutral external-content metadata under a matching authorized
workspace layer, preserve geometry/regions/opacity/color/protection/damage, and emit no input or
surface request yet.
The one-hundred-seventh Gate 8 package adds `ClientSurface` in
`crates/telorgon/src/shell_primitives/client_surface.rs`. It mounts one exact immutable
`ClientSurfaceSnapshot` only beneath an authorized workspace layer, retains its identity/revision,
geometry, clip/opaque/input regions, opacity, content/color/protection/synchronization, and damage,
and exposes an excluded noninteractive Box identity without fabricating a retained image. Mounted
direct-public and umbrella fixtures cover metadata preservation, workspace authority, stable node
identity, and public paths. The primitive owns no image import, synchronization wait/release,
renderer work, input route, surface request, protocol object, task, or thread. The exact next
eligible Gate 8 package is `crates/telorgon/src/shell_primitives/surface_tree.rs`: it must validate and mount
one bounded parent/subsurface tree in exact host painter order without inventing membership,
stacking, reparenting, protocol release, input, or rendering policy.
The one-hundred-eighth Gate 8 package adds `SurfaceTree` in
`crates/telorgon/src/shell_primitives/surface_tree.rs`. One nonempty bounded immutable tree rejects duplicate
identities, a parented first root, additional roots, and any child whose parent does not precede it;
successful mounting retains the supplied order and nests each surface beneath its declared parent
under one authorized workspace layer. Direct-public fixtures cover exact order, nested mounted
ancestry, invalid parentage, identity lookup, and metadata retention. The primitive owns no sorting,
surface membership/stacking policy, reparent request, protocol release, input route, renderer work,
task, or thread. The exact next eligible Gate 8 package is
`crates/telorgon/src/shell_primitives/placeholder.rs`: it must present explicit unavailable/protected/lost
client content without retaining stale external image metadata or fabricating client semantics.
The one-hundred-ninth Gate 8 package adds `SurfacePlaceholder` in
`crates/telorgon/src/shell_primitives/placeholder.rs`. It retains only the surface identity, observed
revision, closed unavailable/protected/lost reason, and caller style, requires an authorized
workspace layer, mounts a fixed redaction-safe Image description, and never creates or retains an
image/external-content record. Mounted direct-public and umbrella fixtures cover typed reasons,
identity/revision, safe semantics, route-free interaction, and absence of stale image state. The
primitive owns no retry/import behavior, client content, host request, OCR, protocol object, task,
or thread. The exact next eligible Gate 8 package is
`crates/telorgon/src/shell_primitives/snapshot.rs`: it must retain a client visual only through exact
host-issued surface/source-revision/output-grant authorization and explicit protected-content policy.
The one-hundred-tenth Gate 8 package adds `SurfaceSnapshot`, opaque authorization/revision tokens,
and explicit `SurfaceSnapshotPolicy` in `crates/telorgon/src/shell_primitives/snapshot.rs`. Authorization is
admitted only from a grant carrying `RETAIN_SURFACE_SNAPSHOT`; construction rejects mismatched
surface/source revisions and protected content not explicitly allowed; mounting rejects the wrong
layer, output, or grant while retaining renderer-neutral source metadata. Direct-public fixtures
cover capability, revision, protection, output/grant binding, and absence of fabricated image/input
state. The primitive owns no capture, pixel copy, import, protection enforcement, synchronization
execution, protocol object, task, or thread. The exact next eligible Gate 8 package is
`crates/telorgon/src/shell_primitives/reserved_area.rs`: it must bind propose/release intentions to one
authorized root and exact observed output revision without changing accepted usable geometry.
The one-hundred-eleventh Gate 8 package adds `ReservedArea` and `ReservedAreaRef` in
`crates/telorgon/src/shell_primitives/reserved_area.rs`. Binding requires a matching root/output view and an
output grant carrying `RESERVE_OUTPUT_AREA`; the result retains reservation ID, edge, extent, grant,
output, and observed revision and constructs exact propose/release `OutputRequest` values. Mounted
direct-public and umbrella fixtures cover identity, revision, authority, and both request variants.
The primitive owns no accepted reservation, geometry mutation, host invocation, display policy,
protocol object, callback, task, or thread. The exact next eligible Gate 8 package is
`crates/telorgon/src/shell_primitives/exclusive_region.rs`: it must validate bounded output-logical geometry
and deterministically block lower shell hit routes inside it without claiming native exclusivity.
The one-hundred-twelfth Gate 8 package adds `ExclusiveRegionGeometry`, `ExclusiveRegion`, and
`ExclusiveHitDecision` in `crates/telorgon/src/shell_primitives/exclusive_region.rs`. Nonempty bounded finite
positive rectangles use deterministic half-open containment; mounted metadata retains its exact
output/layer and returns explicit pass/block decisions while remaining nonfocusable, listener-free,
and excluded from semantics. Direct-public and umbrella fixtures cover invalid geometry, finite
queries, boundary behavior, mounted authority, and absence of input callbacks. The primitive owns no
global hit-test engine, focus/capture, protocol exclusive zone, security boundary, callback, task,
or thread. The exact next eligible Gate 8 package is
`crates/telorgon/src/shell_primitives/surface_input_region.rs`: it must map finite output pointer coordinates
through one exact surface geometry revision into eligible surface-local regions without forwarding
an event, minting causality, or invoking the host.
The one-hundred-thirteenth Gate 8 package adds `SurfaceInputRegion` and `SurfaceInputMapping` in
`crates/telorgon/src/shell_primitives/surface_input_region.rs`. Construction retains the mounted output,
surface identity/revision, exact geometry, and host input region; mapping subtracts only the logical
origin, rejects nonfinite inputs/results, uses deterministic half-open bounds, and distinguishes
outside-surface, outside-input-region, and eligible local points. Direct-public and umbrella
fixtures cover exact local mapping, region exclusion, identity/revision/output retention, and public
paths. The primitive owns no event, seat/contact, forwarding request, transform into buffer pixels,
hit-test engine, callback, task, or thread. The exact next eligible Gate 8 package is
`crates/telorgon/src/shell_primitives/drag_region.rs`: it must validate explicit surface-local drag geometry
and return a begin-move intention only after exact output/grant/surface-capability/contact/hit checks.
The one-hundred-fourteenth Gate 8 package adds `DragRegion` and `DragRegionIntent` in
`crates/telorgon/src/shell_primitives/drag_region.rs`. A nonempty bounded region must fit the exact mounted
surface revision; an intent requires the same output root, `MOVE_SURFACE` grant, host-declared
surface MOVE capability, finite point inside the region, and a validated contact. The result retains
the observed revision, complete seat/contact/source, output/local point, and exact typed BeginMove
request; misses return no request. Direct-public fixtures cover capability/authority, coordinate
mapping, source retention, hit/miss, and request identity. The primitive owns no movement, grab,
capture, gesture arbitration, host invocation, protocol serial, callback, task, or thread. The exact
next eligible Gate 8 package is `crates/telorgon/src/shell_primitives/resize_region.rs`: it must apply the same
fail-closed boundary to one explicit resize edge/corner without changing surface geometry.
The one-hundred-fifteenth Gate 8 package adds `ResizeRegion` and `ResizeRegionIntent` in
`crates/telorgon/src/shell_primitives/resize_region.rs`. It validates nonempty surface-local geometry, retains
one exact `ResizeEdge`, and requires matching output, `RESIZE_SURFACE` grant, host RESIZE capability,
finite in-region point, and validated contact before returning a BeginResize intention with exact
revision/source/coordinates. Direct-public fixtures cover edge identity, capability/authority,
source retention, mapping, hit/miss, and typed requests. The primitive owns no resize application,
edge precedence policy, native grab, capture, gesture arena, host invocation, callback, task, or
thread. The exact next eligible Gate 8 package is
`crates/telorgon/src/shell_primitives/output_edge.rs`: it must derive stable edge/corner geometry from one
observed output revision and keep pointer/touch activation equivalent to directional/accessibility
alternatives without invoking reveal, snap, or reservation policy.
The one-hundred-sixteenth Gate 8 package adds eight-way `OutputEdgeKind`, validated
`OutputEdgeThickness`, `OutputEdgeRegion`, and typed activation intents in
`crates/telorgon/src/shell_primitives/output_edge.rs`. Exact output-local strips/squares reject invalid or
oversized thickness, retain output/revision, use half-open pointer/touch hits, and provide explicit
coordinate-free directional/accessibility alternatives; only four straight edges map to reservation
edges. Direct-public and umbrella fixtures cover corner geometry, boundary hits, source/coordinate
rules, revision identity, and alternatives. The primitive owns no reveal/snap/reservation action,
timer, gesture, accessibility adapter, display configuration, callback, task, or thread. The exact
next eligible Gate 8 package is `crates/telorgon/src/shell_primitives/diagnostics.rs`: it must count stable
primitive failure categories without retaining identities, geometry, contacts, request/content/error
payloads, or an unbounded log.
The one-hundred-seventeenth Gate 8 package adds `ShellPrimitiveDiagnosticCollector` and immutable
`ShellPrimitiveDiagnostics` in `crates/telorgon/src/shell_primitives/diagnostics.rs`. Fifteen closed ordered
categories cover every current primitive boundary plus stale mount state; fixed `u64` counters
saturate, snapshot by value, iterate deterministically, and clear without payload retention. Unit,
direct-public, and umbrella fixtures cover typed error mapping, category order, counting, and
clearing. The package owns no logger, telemetry transport, identity, coordinate, contact, request,
error payload, callback, task, or thread. The exact next eligible Gate 8 package is
`crates/telorgon/src/shell_primitives/ext.rs`: it must expose mount-only conveniences for existing shell
primitive owners without introducing an alternate lifecycle, policy path, or dependency reversal.
The one-hundred-eighteenth Gate 8 package adds primitive `ShellUiExt<Action>` conveniences in
`crates/telorgon/src/shell_primitives/ext.rs` and the curated prelude. Root, output view, ordered layer,
client surface/tree, placeholder, retained snapshot, and exclusive-region methods delegate to the
same validated owners and return their exact focused references/errors. Direct-public mounted and
umbrella fixtures cover trait discovery, complete nested composition, stable identities, and public
paths. The trait introduces no alternate node, semantics, geometry, action, request, lifecycle,
policy, task, or thread. This completes the currently specified `telorgon-primitives-shell` source
set. The exact next eligible Gate 8 package is
`crates/telorgon/src/shell_components/chrome/frame.rs`: it must create the shell-component crate/prelude
scaffolding and mount one named noninteractive Window semantic owner with separate stable chrome and
client-content hosts derived from an exact surface snapshot, without controls, input, or requests yet.
The one-hundred-nineteenth Gate 8 package creates `telorgon-components-shell`, its curated prelude,
chrome/workspace module scaffolding, and `WindowFrame` in `chrome/frame.rs`. One exact immutable
surface revision mounts only on a matching authorized workspace/output pair, maps host-global bounds
into output-local geometry, and owns stable decoration, client-content, and chrome hosts beneath one
named nonfocusable `Window` semantic node. Direct-public and umbrella fixtures cover role/name,
ownership, exact identity/revision/geometry, output mapping, and public paths. The component owns no
control, input route, request, surface mutation, policy, callback, task, or thread. The exact next
eligible Gate 8 package is `crates/telorgon/src/shell_components/chrome/titlebar.rs`: it must present one safe
visible title for the exact framed revision and expose only a capability-checked begin-move intent.
The one-hundred-twentieth Gate 8 package adds `WindowTitlebar`, stable title/control hosts, and
`TitlebarMoveIntent` in `chrome/titlebar.rs`. Explicit or host-supplied nonempty title text is mounted
as ordinary Text semantics; begin-move requires the same output/grant, `MOVE_SURFACE` authority, the
surface MOVE capability, and a validated contact, and retains revision/seat/contact/source in the
typed request seam. Fixtures cover visible title identity, source preservation, and request values.
The component owns no drag recognition, capture, movement, focus transfer, host invocation, native
serial, callback, task, or thread. The exact next eligible Gate 8 package is
`crates/telorgon/src/shell_components/chrome/controls.rs`: it must derive the canonical minimize,
maximize/restore, and close set from the exact surface snapshot and route only authorized typed
requests without changing controlled state.
The one-hundred-twenty-first Gate 8 package adds `WindowControls`, four-way `WindowControl`, and
source-preserving `WindowControlIntent` in `chrome/controls.rs`. Snapshot capabilities determine the
stable control set, MAXIMIZED state chooses Maximize versus Restore, root authority determines
enabled routing, and ordinary named Button/Toolbar semantics expose only effective actions. Mounted
fixtures dispatch keyboard and accessibility activations, preserve the observed revision, and prove
that request emission does not optimistically change surface state. The component owns no host
execution, maximize/minimize/close policy, focus transfer to the client, callback, task, or thread.
The exact next eligible Gate 8 package is `crates/telorgon/src/shell_components/chrome/shadow.rs`: it must
mount validated visual-only frame shadow metadata without becoming an input or semantic target.
The one-hundred-twenty-second Gate 8 package adds `ShadowFrame` in `chrome/shadow.rs`. Finite offset,
nonnegative blur/spread, exact surface revision matching, and the frame's stable decoration host are
validated before mounting a listener-free semantics-excluded shadow node. Fixtures cover invalid
geometry, retained output/surface/revision, exact shadow values, and absence of semantic/input
participation. The component owns no hit region, resize policy, renderer resource, callback, task,
or thread. The exact next eligible Gate 8 package is
`crates/telorgon/src/shell_components/chrome/snap_preview.rs`: it must present one host-provided snap proposal
on the matching overlay/output revision without choosing a target or emitting a snap request.
The one-hundred-twenty-third Gate 8 package adds `SnapPreview` in `chrome/snap_preview.rs`. A finite
positive output-local rectangle is bound to exact surface/output revisions, rejected outside the
observed output, and mounted only on its matching Overlay layer as a noninteractive,
semantics-excluded visual. Fixtures cover bounds/revision retention, output-local placement, layer
authority, and absence of input. The component owns no snap candidates, tiling algorithm, request,
surface geometry mutation, callback, task, or thread. The exact next eligible Gate 8 package is
`crates/telorgon/src/shell_components/workspace/view.rs`: it must present one exact named workspace revision
for an output while preserving host membership, painter order, geometry, and active visibility.
The one-hundred-twenty-fourth Gate 8 package adds `WorkspaceView` in `workspace/view.rs`. It mounts
only on a matching Workspace layer/output view, derives visible/hidden and inert semantics from the
host-owned active flag, retains the exact workspace revision, filters membership only by output
without reordering it, and exposes translation of exact global placements into output-local bounds.
Direct-public and umbrella fixtures cover name/identity/revision, active state, output filtering,
back-to-front order, and coordinate mapping. The component owns no tiling/floating choice, surface
creation, focus stealing, workspace selection, host request, callback, task, or thread. The exact
next eligible Gate 8 package is `crates/telorgon/src/shell_components/workspace/stack.rs`: it must reconcile
the workspace view's exact ordered placements with matching surface snapshots and mount stable
back-to-front window hosts without inventing membership, geometry, focus, or stacking policy.
The one-hundred-twenty-fifth Gate 8 package adds `WindowStack`, validated `WindowStackEntry`, and a
sealed standard-host seam in `workspace/stack.rs`. Entries must cite unique top-level surface
snapshots whose exact geometry and order match a selected subset of the workspace's output
placements; mounting preserves that back-to-front order and composes ordinary `WindowFrame` owners
relative to the selected host's global origin. Fixtures cover missing/duplicate/subsurface/order/
geometry rejection, direct workspace and region hosts, stable frame identity, and public paths. The
component owns no membership reconciliation policy, restacking, focus transfer, surface mutation,
callback, task, or thread. The exact next eligible Gate 8 package is
`crates/telorgon/src/shell_components/workspace/tiling_region.rs`: it must mount one named host-classified
tiled region whose exact member placements fit its bounds without overlap, without running a tiling
algorithm.
The one-hundred-twenty-sixth Gate 8 package adds `TilingRegion` in `workspace/tiling_region.rs`.
Explicit surface membership is resolved back through the exact workspace revision and canonical
painter order; duplicate, cross-output, out-of-region, and overlapping placements fail closed before
mount. The named region exposes a stable `WindowStackHost` while retaining exact global geometry and
output-local placement. Fixtures cover overlap rejection, order preservation, nested stack mapping,
and semantics. The component owns no split tree, allocation algorithm, resize balancing, layout
mutation, request, callback, task, or thread. The exact next eligible Gate 8 package is
`crates/telorgon/src/shell_components/workspace/floating_region.rs`: it must preserve the same exact
host-classified membership and bounds while explicitly allowing overlapping painter-order geometry.
The one-hundred-twenty-seventh Gate 8 package adds `FloatingRegion` in
`workspace/floating_region.rs`. It shares the exact workspace/output/member validation and stable
stack-host boundary, preserves canonical back-to-front order even when caller membership order
differs, and permits overlapping placements without reordering them. Fixtures contrast permitted
floating overlap with tiled rejection and cover nested frame coordinates. The component owns no
placement, cascade, z-order, focus, movement, request, callback, task, or thread. The exact next
eligible Gate 8 package is `crates/telorgon/src/shell_components/workspace/switcher.rs`: it must validate one
bounded ordered workspace catalog and route controlled selection intents without locally changing
the active workspace.
The one-hundred-twenty-eighth Gate 8 package adds bounded `WorkspaceCatalog`,
`WorkspaceSwitcher`, and `WorkspaceSelectionIntent` in `workspace/switcher.rs`. Catalog validation
requires unique identities, strict host order, and at most one active workspace. A matching Overlay
layer mounts named Toolbar/Button semantics; the active item remains controlled, unavailable
selection has no route, and selectable items require `SELECT_WORKSPACE` authority. Selection keeps
the activation source and requires an exact compatible shell source before constructing the typed
revision-bound `WorkspaceRequest`. Fixtures cover catalog ambiguity, keyboard selection, active
state, semantic availability, and nonmutation. The component owns no workspace activation, focus
stealing, contact fabrication, host invocation, callback, task, or thread. The exact next eligible
Gate 8 package is `crates/telorgon/src/shell_components/workspace/overview.rs`: it must reuse that catalog and
selection seam while presenting stable per-workspace preview hosts derived only from exact output
placements.
The one-hundred-twenty-ninth Gate 8 package adds `WorkspaceOverview` and stable workspace/surface
preview references in `workspace/overview.rs`. The component reuses catalog validation and
source-qualified selection, filters exact placements by output without reordering, and applies only
a validated caller-selected visual preview scale while retaining both source and projected bounds.
Preview nodes remain listener-free and excluded from semantics beneath ordinary named workspace
buttons. Fixtures cover active/selectable cards, exact surface membership, projection, semantic
exclusion, and typed selection. The component owns no screenshot, surface readback, workspace
layout, policy request execution, callback, task, or thread. The exact next eligible Gate 8 package
is `crates/telorgon/src/shell_components/panel/panel.rs`: it must mount one named output-edge panel on an
authorized Panel layer and expose a revision-bound reservation proposal without changing accepted
usable geometry.
The one-hundred-thirtieth Gate 8 package adds `Panel` in `panel/panel.rs`. A validated edge/extent
maps to exact output-local panel geometry, a stable named Region/content host, and the existing
authorized `ReservedAreaRef`; propose/release return unchanged typed `OutputRequest` values tied to
the observed output revision. Oversized extents and layer/output/grant mismatches fail closed.
Direct-public and umbrella fixtures cover geometry, semantics, reservation identity/revision, and
unapplied request values. The component owns no accepted reservation state, protocol exclusive
zone, auto-hide state, animation, input blocking, host invocation, callback, task, or thread. The
exact next eligible Gate 8 package is `crates/telorgon/src/shell_components/panel/auto_hide.rs`: it must model
the explicit Hidden/RevealArmed/Revealing/Shown/Hiding transition boundary from caller-owned times
and reveal inputs without owning a clock, task, or continuous reservation negotiation.
The one-hundred-thirty-first Gate 8 package adds the pure `PanelAutoHidePolicy` transition boundary
in `panel/auto_hide.rs`. Validated caller-owned snapshots carry exactly one of Hidden, RevealArmed,
Revealing, Shown, or Hiding plus the deadline shape required by that state. Mouse, touch, pen,
eraser, keyboard, directional, accessibility, and programmatic reveal paths enter the same rules;
caller-supplied `MonotonicInstant` values drive every deadline and reject backward/early/overflowing
time. The package starts no clock or task and emits no reservation request. The exact next eligible
Gate 8 package is `crates/telorgon/src/shell_components/panel/taskbar.rs`: it must present caller-ordered host
application entries inside an exact panel and emit only revision/source-bound system intentions.
The one-hundred-thirty-second Gate 8 package adds `Taskbar` in `panel/taskbar.rs`. It mounts a named
Toolbar beneath a grant/output-matching `PanelRef`, preserves a bounded unique `ApplicationCatalog`
without discovery or sorting, derives availability only from the exact enabled primary action and
system-action grant, and returns source-qualified `ApplicationActionIntent` values without changing
application state. The exact next eligible Gate 8 package is
`crates/telorgon/src/shell_components/panel/dock.rs`: it must expose the same host-truth boundary through a
named navigation presentation without owning pinning, process, focus, or window policy.
The one-hundred-thirty-third Gate 8 package adds `Dock` in `panel/dock.rs`. Dock entries mount under
the exact panel content host in caller order, retain application identity/revision/state/action, and
advertise activation only when that exact primary action is enabled and authorized. It performs no
pinning, launching, activation, focus transfer, reservation negotiation, or host invocation. The
exact next eligible Gate 8 package is `crates/telorgon/src/shell_components/launcher/launcher.rs`: it must
define the shared bounded application catalog and typed activation boundary on an authorized
output overlay.
The one-hundred-thirty-fourth Gate 8 package adds `Launcher`, `ApplicationCatalog`, stable item
references, and `ApplicationActionIntent` in `launcher/launcher.rs`. Duplicate identities fail
closed, caller order and complete immutable entry snapshots are retained, disabled/missing primary
actions remain visible but noninteractive, pointer requests require the caller's exact contact
source, and non-pointer activations infer only their canonical source. The exact next eligible Gate
8 package is `crates/telorgon/src/shell_components/launcher/application_grid.rs`: it must present the same
catalog with bounded caller-selected column addressing and Grid semantics.
The one-hundred-thirty-fifth Gate 8 package adds `ApplicationGrid` and stable row/column item
references in `launcher/application_grid.rs`. Column counts are bounded and nonzero, index-to-cell
addressing follows the unchanged caller order, and Grid/button semantics plus typed application
intent routes reuse the single launcher catalog owner. The component owns no wrapping/layout
policy beyond its explicit logical column count. The exact next eligible Gate 8 package is
`crates/telorgon/src/shell_components/launcher/start_menu.rs`: it must expose a named Menu presentation over
the same exact application/action boundary.
The one-hundred-thirty-sixth Gate 8 package adds `StartMenu` in `launcher/start_menu.rs`. It mounts a
named Menu with MenuItem entries on the grant/output-matching Overlay layer, retains disabled and
missing-action entries without inventing fallbacks, and routes only enabled primary actions as
revision/source-bound intentions. Direct, mounted, and umbrella fixtures cover the five-state
auto-hide sequence, deadline validation, duplicate catalogs, panel binding, exact entry order,
disabled actions, application state, list/grid/menu semantics, grid addressing, and typed requests.
These six packages own no clock, task, process enumeration, search index, app execution, optimistic
state, host callback, thread, or native service. The exact next eligible Gate 8 package is
`crates/telorgon/src/shell_components/status/area.rs`: it must mount a named status collection over the exact
bounded host status snapshot and emit only authorized revision-bound status action intentions.
The one-hundred-thirty-seventh Gate 8 package adds `StatusArea`, stable status-entry references, and
the source-qualified `StatusActionIntent` in `status/area.rs`. The named Status collection mounts
under an exact grant/output-matching panel, preserves snapshot order and revision, uses only the
host-selected primary action, disables unavailable or unauthorized entries, presents Public values,
omits Sensitive values, and excludes Secret entries from text and semantics while retaining their
opaque identity in the component reference. The exact next eligible Gate 8 package is
`crates/telorgon/src/shell_components/status/clock.rs`: it must bind only a host-authored Clock entry without
reading a clock, formatting time, or owning update scheduling.
The one-hundred-thirty-eighth Gate 8 package adds `StatusClock` in `status/clock.rs`. It rejects
missing and wrong-kind identities, wraps the already-mounted stable status node, and exposes only
the value permitted by the area privacy policy. It performs no wall/monotonic time read, timezone or
locale formatting, timer creation, or snapshot mutation. The exact next eligible Gate 8 package is
`crates/telorgon/src/shell_components/status/indicator.rs`: it must type-check ordinary host indicator kinds
and preserve their severity/privacy boundary without querying a system service.
The one-hundred-thirty-ninth Gate 8 package adds `StatusIndicator` in `status/indicator.rs` for the
Connectivity, Power, Audio, Input, Session, and Privacy kinds. Typed references retain the original
node, kind, severity, and privacy-filtered presented value; Clock, Media, and Extension entries fail
closed. The package queries no network, power, audio, input, session, or privacy service. The exact
next eligible Gate 8 package is `crates/telorgon/src/shell_components/status/media.rs`: it must expose one
exact Media summary and its opaque host actions without playback/session integration.
The one-hundred-fortieth Gate 8 package adds `MediaStatus` in `status/media.rs`. Only an exact Media
entry binds, its summary remains subject to the status-area privacy policy, and its ordered typed
actions remain immutable host descriptions. No metadata discovery, playback, session mutation,
media key handling, or service invocation occurs. The exact next eligible Gate 8 package is
`crates/telorgon/src/shell_components/status/quick_settings.rs`: it must mount every exact status action in
canonical entry/action order on an authorized overlay and return only typed intentions.
The one-hundred-forty-first Gate 8 package adds `QuickSettings` and stable action references in
`status/quick_settings.rs`. A named Menu/MenuItem presentation flattens the validated snapshot
without sorting, preserves entry/revision/action identity, disables unavailable and unauthorized
actions, excludes Secret entries from text and semantics, and routes enabled actions through the
single source-qualified status intention owner. The exact next eligible Gate 8 package is
`crates/telorgon/src/shell_components/status/extension_slot.rs`: it must bind only an approved Extension
entry without granting code, layer, native-surface, model-enumeration, or unrestricted action access.
The one-hundred-forty-second Gate 8 package adds `StatusExtensionSlot` in
`status/extension_slot.rs`. The typed wrapper rejects every non-Extension identity and exposes only
the already-approved immutable label/value/icon/action model through the same mounted status node;
it cannot execute extension code, mint grants, import native content, or bypass the area privacy and
action boundaries. Direct, mounted, and umbrella fixtures cover kind rejection, exact ordering,
clock non-ownership, public/sensitive/secret presentation, unavailable severity, semantic exclusion,
primary and non-primary actions, source qualification, and public paths. These six packages own no
system service, clock, timer, media session, extension runtime, optimistic state, callback, task, or
thread. The exact next eligible Gate 8 package is
`crates/telorgon/src/shell_components/notification/host.rs`: it must present an exact bounded notification
snapshot with priority/privacy/lifecycle behavior and typed actions without implementing delivery,
storage, expiry, or a platform notification service.
The one-hundred-forty-third Gate 8 package adds `NotificationHost`, the bounded
`NotificationCatalog`, stable notification/action references, and the source-qualified
`NotificationActionIntent` in `notification/host.rs`. Caller order, notification/revision/action
identity, priority roles, and lifecycle state remain exact: acknowledged entries retain stable
references but are hidden and inert in the transient host. Public title/body content is presented,
Sensitive body content is omitted, Secret content/actions are excluded, and only enabled actions
authorized by the root grant receive routes. The package performs no delivery, storage, expiry,
dismissal, or platform notification work. The exact next eligible Gate 8 package is
`crates/telorgon/src/shell_components/notification/center.rs`: it must retain the same bounded catalog and
privacy/action boundary while presenting acknowledged history as a named List.
The one-hundred-forty-fourth Gate 8 package adds `NotificationCenter` in
`notification/center.rs`. It reuses the single catalog, privacy, authorization, and typed-action
owner, preserves exact caller order as ListItem semantics, and keeps acknowledged entries presented
without mutating their host lifecycle. It performs no history persistence, read-state mutation,
delivery, expiry, grouping, sorting, or platform service work. The exact next eligible Gate 8
package is `crates/telorgon/src/shell_components/notification/system_dialog.rs`: it must mount only an
unacknowledged Critical notification on an exact authorized SystemModal layer.
The one-hundred-forty-fifth Gate 8 package adds `SystemDialog` in
`notification/system_dialog.rs`. It rejects non-Critical and acknowledged snapshots, requires an
output/grant-matching SystemModal layer, exposes Dialog semantics under an explicit safe name, and
routes only exact enabled authorized actions. Public bodies are presented, Sensitive bodies are
withheld, and Secret notification content/actions are excluded; the reference reports the host
policy intent that lower layers become inert but does not apply focus, input, or security policy.
The exact next eligible Gate 8 package is `crates/telorgon/src/shell_components/notification/osd.rs`: it
must provide a controlled noninteractive overlay presentation without owning a clock or expiry.
The one-hundred-forty-sixth Gate 8 package adds `OnScreenDisplay` in `notification/osd.rs`. It
requires an exact authorized Overlay layer and an action-free notification snapshot, exposes
controlled visibility with Status semantics, and applies the same Public/Sensitive/Secret content
policy. It starts no clock, timer, animation, task, delivery path, or system-service query. The
exact next eligible Gate 8 package is
`crates/telorgon/src/shell_components/secure/lock_composition.rs`: it must host controlled content only on an
exact authorized Lock layer and surface lower-layer inertness as unapplied host-policy intent.
The one-hundred-forty-seventh Gate 8 package adds `LockComposition` in
`secure/lock_composition.rs`. It mounts a full-view named Application composition with a distinct
content host on the exact output/grant-matching Lock layer, makes inactive compositions hidden and
inert, and preserves output/grant/active identity in its reference. It does not authenticate,
authorize a session, capture input, move focus, hide lower semantics, or enforce security policy.
The exact next eligible Gate 8 package is
`crates/telorgon/src/shell_components/secure/system_modal.rs`: it must provide the corresponding controlled
generic content host on an exact authorized SystemModal layer.
The one-hundred-forty-eighth Gate 8 package adds `SystemModalHost` in
`secure/system_modal.rs`. It mounts a named Dialog composition plus an owned content host on the
exact output/grant-matching SystemModal layer, exposes controlled active/hidden/inert state, and
retains only the intent that lower layers become inert. Direct, mounted, and umbrella fixtures for
these six packages cover catalog rejection, canonical order, priority roles, lifecycle behavior,
Public/Sensitive/Secret presentation, exact source-qualified actions, OSD noninteraction, secure
layer authority, controlled state, semantic roles, and public paths. They own no delivery service,
storage, clock, timer, authentication, security enforcement, host callback, task, or thread. The
exact next eligible Gate 8 package is `crates/telorgon/src/shell_components/diagnostics.rs`: it must expose
bounded payload-free component counters without retaining notification/secure content, invoking a
host, or owning a logging service.
The one-hundred-forty-ninth Gate 8 package adds `ShellComponentDiagnostics`, its fixed twelve-kind
taxonomy, and a caller-owned saturating collector in `crates/telorgon/src/shell_components/diagnostics.rs`.
Family validation and action-source errors collapse into payload-free categories; manual categories
cover authorization suppression, lifecycle suppression, privacy redaction, and stale mounts without
retaining labels, IDs, grants, notification content, snapshots, or error payloads. Recording performs
no allocation, logging, callback, host invocation, task, or thread work. This completes the currently
specified `telorgon-components-shell` source tree. The exact next eligible Gate 8 domain-split package
is `crates/telorgon/src/theme/source.rs`: it must extract the serializable source records and parsing boundary
from the monotelorgon theme runtime without changing source compatibility.
The one-hundred-fiftieth Gate 8 package extracts `ThemeSource`, `ThemeFormat`, `StyleSource`, and
`PaintSource` into `crates/telorgon/src/theme/source.rs`. TOML parsing remains one deterministic error-mapped
boundary, authored styles retain canonical `BTreeMap` order, and the existing public records remain
source compatible. It adds no file I/O, watcher, registry, inheritance, asset loading, or platform
service. The exact next eligible Gate 8 domain-split package is `crates/telorgon/src/theme/compiler.rs`: it
must become the single source-to-compiled-table transformation owner.
The one-hundred-fifty-first Gate 8 package extracts deterministic theme compilation into
`crates/telorgon/src/theme/compiler.rs`. Default-style insertion, validated style names, v2 radius
normalization, state-slot mapping, color parsing, and numeric box/paint compilation preserve the
existing results and diagnostics without owning a runtime registry or archive transport. The exact
next eligible Gate 8 domain-split package is `crates/telorgon/src/theme/compiled.rs`: it must own immutable
compiled style/theme tables and their borrowed lookup surface.
The one-hundred-fifty-second Gate 8 package extracts `PaintStyle`, `CompiledStyle`, and
`CompiledTheme` into `crates/telorgon/src/theme/compiled.rs`. Immutable tables preserve scope/style identity,
borrowed name and numeric lookup, deterministic state resolution, diagnostics access, and the
existing compatibility archive encoding until its focused owner lands. The exact next eligible
Gate 8 domain-split package is `crates/telorgon/src/theme/resolver.rs`: it must provide one shared engine with
isolated application, shell, and preview registries rather than duplicating the compiler.
The one-hundred-fifty-third Gate 8 package adds `ThemeRegistry` in
`crates/telorgon/src/theme/resolver.rs` and uses `ThemeRegistry` as the sole public registry spelling. One registry
owns separate application and shell root tables, domain-qualified preview tables, replacement,
discard, and borrowed resolution. Replacing or previewing one domain cannot mutate the other root;
the existing application-only preview methods continue to compile for first-party tools. The exact
next eligible Gate 8 domain-split package is `crates/telorgon/src/theme/scope.rs`: it must make domain and
root-versus-preview identity explicit at the resolver boundary.
The one-hundred-fifty-fourth Gate 8 package adds `ThemeDomain`, `ThemeScopeKind`, and the opaque
domain-qualified `ThemeScope` in `crates/telorgon/src/theme/scope.rs`. Application and shell roots have
distinct raw IDs, typed lookups reject a mismatched scope record, root scopes cannot be discarded as
previews, and focused plus umbrella fixtures cover source order, compilation/state resolution, v2
normalization, cross-domain absence, preview isolation/discard, compatibility paths, and public
exports. These five theme packages split one engine; they do not duplicate compiler state, load
assets, watch files, mutate component semantics, or start a task/thread. The exact next eligible
Gate 8 domain-split package is `crates/telorgon/src/theme/archive.rs`: it must extract deterministic archive
encoding from the compiled model without adding file I/O or claiming a stable decode format.
The one-hundred-fifty-fifth Gate 8 package extracts the current `LTH3` in-memory encoder and magic
into `crates/telorgon/src/theme/archive.rs`. Encoding preserves canonical compiled-table/name order and the
existing `CompiledTheme::encode` compatibility method, with deterministic byte fixtures. The module
opens no file, performs no decoding, and does not claim a stable interchange format. The exact next
eligible Gate 8 domain-split package is `crates/telorgon/src/theme/diagnostics.rs`: it must become the single
owner of structured compiler diagnostics without coupling them to registry resolution.
The one-hundred-fifty-sixth Gate 8 package extracts `ThemeDiagnostic` into
`crates/telorgon/src/theme/diagnostics.rs`. The existing style/message compatibility fields and equality
remain intact, and a focused constructor supports compiler-owned normalization reports. Diagnostics
do not own registry state, emit logs, write files, invoke callbacks, or capture component/runtime
content. The exact next eligible Gate 8 domain-split package is `crates/telorgon/src/theme/error.rs`: it must
own the theme error/result boundary and finish the declared theme source split.
The one-hundred-fifty-seventh Gate 8 package extracts `ThemeError` and `ThemeResult` into
`crates/telorgon/src/theme/error.rs`. Display, cloning, equality, and standard-error behavior remain source
compatible, and the former compatibility `runtime.rs` implementation file is removed. This
completes the declared `source/compiler/compiled/resolver/scope/archive/diagnostics/error` theme
layout with one implementation owner per concern. The exact next eligible Gate 8 domain-split
package is `telorgon-ui-gallery/src/model.rs`: it must extract the stable specimen, preview-state, and
action vocabulary without changing the gallery's public names or behavior.
The one-hundred-fifty-eighth Gate 8 package extracts `Specimen`, `PreviewState`, and `GalleryAction`
into `telorgon-ui-gallery/src/model.rs`. The closed seven-specimen order is explicit, all existing
action variants remain unchanged, and crate-root compatibility re-exports preserve callers. The
model owns no mounted UI, renderer, theme registry, application lifecycle, or platform host. The
exact next eligible Gate 8 domain-split package is `telorgon-ui-gallery/src/diagnostics.rs`: it must
separate the gallery's tool-local performance/debug records from application state.
The one-hundred-fifty-ninth Gate 8 package extracts `PerformanceSnapshot` and `DebugOverlay` into
`telorgon-ui-gallery/src/diagnostics.rs`. Their public fields/default/equality behavior remain intact,
and focused fixtures prove empty defaults without treating predicted counters or rectangles as
qualified performance evidence. The records perform no sampling, rendering, logging, scheduling,
or background work. The exact next eligible Gate 8 domain-split package is
`telorgon-ui-gallery/src/application.rs`: it must become the single mounted-gallery application owner
while preserving the existing root export and binary entry point.
The one-hundred-sixtieth Gate 8 package moves `GalleryApp`, its mount/update implementation,
tool-local visual helpers, and existing behavioral fixtures into
`telorgon-ui-gallery/src/application.rs`. The crate root is now declaration/re-export only; focused
and root public paths compile, and all eight existing mounted/software behavioral fixtures continue
to pass without launching the application during verification. These six packages add no server,
watcher, application launch, task, or thread. The exact next eligible Gate 8 domain-split package is
`telorgon-ui-gallery/src/state.rs`: it must extract the gallery's tool-local editable specimen and
interaction state from the mounted application owner without changing behavior.
The one-hundred-sixty-first Gate 8 package extracts `GalleryState`, per-specimen editable
parameters, stable specimen/state indexing, defaults, and reset behavior into
`telorgon-ui-gallery/src/state.rs`. The state remains tool-local, preserves the current specimen and
session action count across an inspector reset, and owns no mounted handles, theme registry,
renderer, or platform host. The exact next eligible Gate 8 domain-split package is
`telorgon-ui-gallery/src/fixtures.rs`: it must separate mounted/software behavioral evidence from the
application implementation without weakening private test access.
The one-hundred-sixty-second Gate 8 package moves all eight gallery application fixtures into
`telorgon-ui-gallery/src/fixtures.rs` as a test-only child of the application owner. Native window
identity, headless rendering, retained specimen navigation, live slider/toggle behavior, all seven
specimen actions, isolated theme preview, per-specimen edits, and inspector controls continue to
execute without launching a window. The exact next eligible Gate 8 domain-split package is
`telorgon-theme-studio/src/document.rs`: it must become the single owner of non-destructive tool-local
theme document parsing, validation, loading, replacement, and saving.
The one-hundred-sixty-third Gate 8 package extracts `ThemeDocument` and `DocumentError` into
`telorgon-theme-studio/src/document.rs`. Parsing occurs before construction or replacement commits,
validation delegates to the shared compiler, and explicit load/save retain their existing file-I/O
semantics; the document owner does not mount UI, manage preview scopes, or start background work.
The exact next eligible Gate 8 domain-split package is `telorgon-theme-studio/src/model.rs`: it must
extract the stable Theme Studio action vocabulary.
The one-hundred-sixty-fourth Gate 8 package extracts `StudioAction` into
`telorgon-theme-studio/src/model.rs`. Validate, apply-preview, and reset-preview variants remain source
compatible and own no document, runtime, UI, renderer, or platform behavior. The exact next
eligible Gate 8 domain-split package is `telorgon-theme-studio/src/application.rs`: it must become the
single owner of the mounted tool and isolated preview lifecycle.
The one-hundred-sixty-fifth Gate 8 package moves `ThemeStudio`, its mounted handles, preview-scope
lifecycle, action handling, and visual helpers into `telorgon-theme-studio/src/application.rs`.
Existing crate-root names and the binary entry point remain compatible, while the library root is
declaration/re-export only. The exact next eligible Gate 8 domain-split package is
`telorgon-theme-studio/src/fixtures.rs`: it must separate document/application evidence from the
implementation owners.
The one-hundred-sixty-sixth Gate 8 package adds test-only
`telorgon-theme-studio/src/fixtures.rs`, preserving the non-destructive invalid replacement fixture
and adding valid dirty/compile plus headless mounted-render evidence. Focused module paths and
existing crate-root exports compile through an integration fixture, and the Theme Studio binary is
compiled but not launched. These six packages add no server, watcher, application launch, task, or
thread. The exact next eligible Gate 8
domain-split package is `telorgon-ui-gallery/src/style.rs`: it must extract tool-local visual constants
and style construction from the mounted application without changing rendered behavior.
The one-hundred-sixty-seventh Gate 8 package extracts the gallery palette, accent names, color
derivation, theme-source color formatting, and all `BoxStyle`/`LayoutStyle` constructors into
`telorgon-ui-gallery/src/style.rs`. The owner remains private to the tool, and focused fixtures prove
debug-bound isolation plus deterministic state tones while every mounted/software gallery fixture
continues to pass. The exact next eligible Gate 8 domain-split package is
`telorgon-theme-build/src/model.rs`: it must define reusable build input and completion records without
performing compilation or file I/O.
The one-hundred-sixty-eighth Gate 8 package adds `BuildRequest` and `BuildReport` in
`telorgon-theme-build/src/model.rs`. Requests retain the caller's input and optional output, default
archive naming remains `<input>.lthm`, and reports retain resolved source/output paths, style count,
and archive byte count. The model performs no parsing, compilation, I/O, printing, or process
control. The exact next eligible Gate 8 domain-split package is
`telorgon-theme-build/src/error.rs`: it must replace the erased CLI error boundary with typed usage,
I/O, and theme failures.
The one-hundred-sixty-ninth Gate 8 package adds `BuildError` in
`telorgon-theme-build/src/error.rs`. Usage failures preserve the command syntax, I/O failures retain
their operation/path/source, and parser/compiler failures retain `ThemeError`; display and standard
error chaining require no logging or process exit. The exact next eligible Gate 8 domain-split
package is `telorgon-theme-build/src/builder.rs`: it must own explicit source-to-archive execution
through the shared theme compiler.
The one-hundred-seventieth Gate 8 package adds `compile_source` and `build_theme` in
`telorgon-theme-build/src/builder.rs`. The builder resolves file-versus-directory input, reads
`theme.toml`, delegates parsing/compilation to `telorgon-theme`, encodes the current deterministic
`LTH3` bytes, explicitly creates the output parent, writes once, and returns a report. It introduces
no second compiler, decoder, watcher, cache, or background work. The exact next eligible Gate 8
domain-split package is `telorgon-theme-build/src/cli.rs`: it must parse command arguments without
owning file I/O, printing, or process control.
The one-hundred-seventy-first Gate 8 package adds the stable usage text and `parse_arguments` in
`telorgon-theme-build/src/cli.rs`. One required input and one optional output are accepted from
OS-native strings, all other arities return the typed usage failure, and `main.rs` is now a thin
parse/build/report/exit adapter. The exact next eligible Gate 8 domain-split package is
`telorgon-theme-build/src/fixtures.rs`: it must cover argument cardinality, deterministic shared
compilation, directory input, archive output, and focused public paths.
The one-hundred-seventy-second Gate 8 package adds test-only
`telorgon-theme-build/src/fixtures.rs` plus a focused public-path integration fixture. Tests cover CLI
cardinality, default output naming, deterministic compilation, real directory-to-archive execution,
reported byte counts, and the `LTH3` compatibility magic. The binary is compiled but not executed;
these six packages add no server, watcher, application launch, task, or thread. The exact next
eligible Gate 8 domain-split package is `telorgon-theme-create/src/model.rs`: it must define the
creation request/report boundary without performing file I/O.
The one-hundred-seventy-third Gate 8 package adds `CreateRequest` and `CreateReport` in
`telorgon-theme-create/src/model.rs`. Requests retain the exact destination directory and derive only
its `theme.toml` path; reports retain the created directory/source path and source byte count. The
model performs no existence check, creation, write, printing, or process control. The exact next
eligible Gate 8 domain-split package is `telorgon-theme-create/src/error.rs`: it must replace the
erased CLI boundary with typed usage, existing-target, and I/O failures.
The one-hundred-seventy-fourth Gate 8 package adds `CreateError` in
`telorgon-theme-create/src/error.rs`. Usage failures preserve the command syntax, existing-target
failures retain the exact path, and I/O failures retain operation/path/source with standard error
chaining. The owner performs no logging, cleanup, or process exit. The exact next eligible Gate 8
domain-split package is `telorgon-theme-create/src/creator.rs`: it must own explicit minimal Theme v3
directory creation while preserving the current template and non-overwrite rule.
The one-hundred-seventy-fifth Gate 8 package adds `DEFAULT_THEME_SOURCE` and `create_theme` in
`telorgon-theme-create/src/creator.rs`. Creation rejects an existing destination before mutation,
creates missing parent directories, writes the exact existing `theme.toml` template once, and
returns a completion report. It does not invoke the build tool, start a watcher, overwrite content,
or perform rollback deletion. The exact next eligible Gate 8 domain-split package is
`telorgon-theme-create/src/cli.rs`: it must parse one OS-native directory argument without owning file
I/O, printing, or process control.
The one-hundred-seventy-sixth Gate 8 package adds the stable usage text and `parse_arguments` in
`telorgon-theme-create/src/cli.rs`. Exactly one OS-native directory is accepted, every other arity
returns the typed usage failure, and `main.rs` is now a thin parse/create/report/exit adapter. The
exact next eligible Gate 8 domain-split package is `telorgon-theme-create/src/fixtures.rs`: it must
cover argument cardinality, template validity, nested creation, non-overwrite behavior, and focused
public paths.
The one-hundred-seventy-seventh Gate 8 package adds test-only
`telorgon-theme-create/src/fixtures.rs` plus a focused public-path integration fixture. Tests prove
exact CLI cardinality, compiler-parseable Theme v3 template content, real nested-directory/source
creation, exact report bytes/paths, and rejection of an existing destination without overwriting its
source. The binary is compiled but not executed. This completes the focused source split for both
theme command-line tools; it adds no server, watcher, application launch, task, or thread. The exact
next eligible Gate 8 integration package is
`crates/telorgon/src/application_primitives/environment_reads.rs`: it must expose validated per-view
environment aspect groups as dependency-tracked runtime reads without importing platform types or
duplicating `EnvironmentState`.
The one-hundred-seventy-eighth Gate 8 integration package adds `EnvironmentReadBinding`,
`EnvironmentReads`, and six focused aspect values in
`crates/telorgon/src/application_primitives/environment_reads.rs`. A component-created binding stores one
immutable `EnvironmentSnapshot`, derives geometry, scale/density, language/direction, input,
preference, and view-state reads, and publishes only contiguous `EnvironmentUpdate` values from the
canonical external `EnvironmentState`. Aspect equality ignores unrelated fields, so downstream
observers remain quiet while one source state preserves cross-aspect atomicity. Stale, skipped,
change-set-mismatched, and cross-owner publications are rejected before staging; unchanged updates
perform no read work. Direct-package and umbrella compile fixtures cover getters, coherent combined
reads, selective invalidation, ownership, revision continuity, public exports, and the absence of
platform/native types. The exact next eligible Gate 9 neutral-spine package is
`crates/telorgon/src/platform/id.rs`: it must establish opaque generational view and platform object
identities without embedding native handles, pointer values, protocol IDs, or runtime ownership.
The one-hundred-seventy-ninth Gate 9 integration package creates the dependency-free
`telorgon-platform` crate and adds `ViewId`, `DataOfferId`, `RequestId`, and
`NativeSurfaceGeneration` in
`crates/telorgon/src/platform/id.rs`. View and offer identities retain separate nonzero owner-local slot and
generation values; request admission and native-surface continuity use distinct nonzero sequence
types. All representations are private, compact, `Send + Sync`, and strongly typed rather than
aliases for Winit IDs, pointers, protocol serials, file descriptors, renderer handles, runtime
owners, text sessions, or accessibility nodes. Unit, direct public-path, rustdoc, strict-lint, and
umbrella compile fixtures cover zero rejection, value preservation, namespace separation, stale
generation mismatch, ordered-map lookup, and niche-preserving optional layout. The exact next
eligible Gate 9 neutral-spine package is `crates/telorgon/src/platform/stamp.rs`: it must define strictly
ordered host-stream event stamps over injected monotonic time without reading wall time or owning a
clock, queue, event loop, timer, or native timestamp conversion.
The one-hundred-eightieth Gate 9 integration package moves the existing host-injected
`MonotonicInstant` value from the runtime timer implementation to `crates/telorgon/src/core/time.rs`, while
preserving the compatible `telorgon-runtime` export, and adds `EventStamp`, `EventStampError`, and the
non-cloneable `EventStampStream` in `crates/telorgon/src/platform/stamp.rs`. One stream assigns strict
sequences starting at one, accepts equal-resolution receipt instants, retains optional mapped source
time, rejects backward receipt time and source-after-receipt mappings without mutation, and reports
sequence exhaustion instead of wrapping. The owner reads no clock and contains no queue, event
loop, timer, native timestamp conversion, callback, task, or thread. Unit, direct public-path,
strict-lint, and umbrella compile fixtures cover ordering, clock ties, mapped/absent source time,
atomic rejection, exhaustion, thread-transferable stamp values, and the shared runtime/platform
instant representation. The exact next eligible Gate 9 neutral-spine package is
`crates/telorgon/src/platform/capability.rs`: it must define typed support availability, unavailable reasons,
permission state, limits, and execution/user-gesture requirements without probing a platform,
requesting permission, spawning work, or importing an adapter.
The one-hundred-eighty-first Gate 9 integration package adds `Support<T>`, seven closed
`UnavailableReason` values, `PermissionState`, `ExecutionRequirement`,
`UserGestureRequirement`, `CapabilityLimit<T>`, `NoCapabilityLimits`, and generic
`CapabilityDescriptor<Operations, Limits>` in `crates/telorgon/src/platform/capability.rs`. An unavailable
query carries no stale descriptor; a supported capability remains available when permission is
unknown, prompt-required, denied, or restricted; execution and recent-user-gesture requirements are
explicit; and an unspecified maximum makes no unlimited-resource claim. Service-specific operation
and limit records remain typed generic payloads. The package performs no platform query, adapter
initialization, permission prompt, request execution, callback, allocation, thread/executor/event-
loop creation, or native import. Unit, direct public-path, strict-lint, and umbrella compile fixtures
cover every separation, bounded/unspecified limits, unavailable mapping, payload preservation,
thread-transferable values, and focused/root exports. The exact next eligible Gate 9 neutral-spine
package is `crates/telorgon/src/platform/lifecycle.rs`: it must model independent view lifetime, application
activity, visibility, and native-surface availability axes plus legal idempotent/terminal
transitions without owning a view registry, presenter, native handle, event loop, or application
exit policy.
The one-hundred-eighty-second Gate 9 integration package adds `ViewLifetime`, `ActivityState`,
`VisibilityState`, `NativeSurfaceState`, `LifecycleTransition<T>`, `LifecycleAxis`,
`LifecycleError`, and the non-cloneable `ViewLifecycle` transition owner in
`crates/telorgon/src/platform/lifecycle.rs`. The four axes remain separately observable: a hidden,
occluded, background, or suspended view may still retain an available surface. Equal observations
are idempotent, legal lifetime movement is forward-only, `Closed` is terminal, and failed
observations leave every axis unchanged. A surface becomes available only for a live view, closing
requires its current surface to be retired first, and the latest accepted generation remains
recorded after retirement so it cannot reappear or move backward. The package owns no view ID,
registry, snapshot revision, native handle, presenter, event loop, callback, or exit policy. Unit,
direct public-path, strict-lint, rustdoc, and umbrella compile fixtures cover initial state,
independent axes, redundant callbacks, invalid transition atomicity, surface continuity,
close ordering, terminal behavior, and public exports. The exact next eligible Gate 9 neutral-spine
package is `crates/telorgon/src/platform/view.rs`: it must combine a typed view identity, lifecycle facts,
and a revision into immutable atomic view snapshots while distinguishing cancellable close
requests from forced destruction, without owning the view registry, native object, presenter,
event loop, callback execution, or application exit policy.
The one-hundred-eighty-third Gate 9 integration package adds nonzero `ViewRevision`, immutable
`ViewSnapshot`, before/after `ViewUpdate`, structured `ViewStateError`, and the non-cloneable
`ViewState` publication owner in `crates/telorgon/src/platform/view.rs`. The initial coherent publication is
revision one; each accepted changed lifecycle observation advances exactly once; a redundant
observation retains the exact snapshot; and invalid lifecycle movement or revision exhaustion
leaves every fact unchanged. Snapshot fields are private so consumers cannot construct invented
host truth and the later metrics, focus, and environment packages can extend the publication.
`CloseRequest` captures one view/revision and a typed source for the normal routed path, with
explicit accept, reject, and defer decisions. `ForcedDestruction` instead captures an unanswerable
destroying/destroyed notification and exposes no decision API. Neither value executes close,
cleanup, cancellation, native destruction, callbacks, or process exit. Unit, direct public-path,
strict-lint, rustdoc, and umbrella compile fixtures cover coherent initial state, one-revision
publication, redundant callbacks, lifecycle and exhaustion atomicity, independent axes, surface
retirement, view/revision-scoped close facts, and focused/root exports. The exact next eligible
Gate 9 neutral-spine package is `crates/telorgon/src/platform/metrics.rs`: it must define validated,
revisioned physical/logical extent, scale, transform, inset, avoidance, display, and renderability
facts with explicit coordinate spaces and zero-extent behavior, without owning a native window,
display query, renderer, presenter, event loop, or coordinate conversion policy.
The one-hundred-eighty-fourth Gate 9 integration package adds `PhysicalExtent`, validated
`ScaleFactor`, explicit `LogicalToPhysicalTransform`, four `CoordinateSpace` values, mirrored and
rotated `DisplayTransform`/`DisplayOrientation`, neutral `DisplayColorSpace`/`HdrState`, typed
`MetricInsets` and bounded `AvoidRegion` values, immutable `ViewMetrics`, nonzero
`MetricsRevision`, retained `ViewMetricsSnapshot`, and the non-cloneable `ViewMetricsState` owner in
`crates/telorgon/src/platform/metrics.rs`. Logical extent is derived from unsigned physical pixels and a
finite positive scale; a derived overflow is rejected. Safe drawing and gesture insets must be
finite, nonnegative, view-relative, and fit their cited logical or physical extent. Nonempty finite
IME/system/cutout/host avoidance rectangles retain their own coordinate space and are capped at 32
per publication. Either zero physical dimension remains exactly zero and non-renderable. Equal
metrics reuse the retained publication, changed metrics advance once, and revision exhaustion is
atomic. `ViewSnapshot` now includes the exact retained metrics snapshot; `ViewState` preflights both
revision owners so a metrics change advances both or neither. No type queries a display, converts
native coordinates, clamps extent, creates a target, acquires/presents, or imports renderer/native
types. Unit, direct public-path, strict-lint, rustdoc, and umbrella fixtures cover scale/overflow,
zero extent, inset bounds/spaces, avoidance validation/bounds, display transform/color/HDR,
redundancy, both exhaustion boundaries, retained snapshots, and atomic view integration. The exact
next eligible Gate 9 neutral-spine package is `crates/telorgon/src/platform/event.rs`: it must define the
immutable view-scoped platform event envelope over `ViewId`, `EventStamp`, typed payloads, cited
metrics revisions where coordinate conversion occurred, and explicit coalescing metadata without
owning a queue, native translator, callback, clock, scheduler, or dispatch policy.
The one-hundred-eighty-fifth Gate 9 integration package adds generic immutable `PlatformEvent<T>`
values in `crates/telorgon/src/platform/event.rs`. Every envelope retains one generation-safe `ViewId`, the
newest complete `EventStamp`, an explicit `MetricsCitation`, adapter-produced
`CoalescingMetadata`, and its typed payload. `CollapsedEventCount` excludes the retained event and
cannot represent zero. Payload mapping preserves every platform fact. The module records an
already-reviewed coalescing result but deliberately supplies no merge operation or compatibility
policy, so it cannot combine different views, payload meanings, or metrics citations. It owns no
queue, native translator, coordinate conversion, callback, clock, scheduler, or dispatch. Unit,
direct-path, strict-lint, rustdoc, and umbrella fixtures cover unconverted and exact-revision
citations, newest source/receipt stamp retention, nonzero collapsed counts, typed payload access,
and evidence-preserving mapping. The dependency prerequisite before terminal request outcomes is
`crates/telorgon/src/platform/error.rs`: it must expose structured failures that preserve sanitized causal
classification without admitting sensitive native payloads or display-string branching.
The one-hundred-eighty-sixth Gate 9 integration package adds eight closed `PlatformErrorKind`
categories, redaction-safe `PlatformErrorSource`, `PlatformError`, and `PlatformResult<T>` in
`crates/telorgon/src/platform/error.rs`. Errors carry only an author-written static operation context and an
optional sanitized typed cause; arbitrary platform strings, error codes, pointers, handles, paths,
transfer contents, and user data cannot enter the portable record. Portable code branches on the
closed kind rather than rendered diagnostics, while the standard error source chain retains the
sanitized cause. Unit, direct-path, strict-lint, rustdoc, and umbrella fixtures cover every kind,
structured causality, diagnostic rendering, compact thread-transferable records, and root exports.
The exact next eligible Gate 9 neutral-spine package is `crates/telorgon/src/platform/request.rs`: it must
bind admitted `RequestId` values to exactly one typed terminal `RequestOutcome<T>` of applied,
denied, unsupported, cancelled, stale, or failed-with-`PlatformError`, without executing a request,
inventing observed host truth, owning callbacks, or introducing a completion queue.
The one-hundred-eighty-seventh Gate 9 integration package adds `RequestAdmission<T, E>`, the
non-cloneable typed `AdmittedRequest<T>`, six-way `RequestOutcome<T>`, and non-cloneable immutable
`RequestCompletion<T>` in `crates/telorgon/src/platform/request.rs`. Immediate validation rejection creates
no request identity. Successful admission carries only the issued `RequestId` and expected result
type; consuming the token binds that identity to one applied, denied, unsupported, cancelled,
stale, or structured failed outcome. Applied data mapping preserves identity and all non-applied
terminal classifications. `Applied` explicitly does not fabricate a view or service snapshot.
Compile-fail coverage proves one token cannot complete twice, while unit, direct-path,
strict-lint, rustdoc, and umbrella fixtures cover admission separation, terminal distinctions,
structured failure, identity preservation, and typed mapping. The module owns no executor, queue,
callback, service, native object, clock, or cancellation side effect. The next dependency-safe
neutral package is `crates/telorgon/src/platform/clock.rs`: it must expose a host-injected monotonic clock
without an ambient or wall-time fallback.
The one-hundred-eighty-eighth Gate 9 integration package adds object-safe `MonotonicClock`, typed
`MonotonicClockError`, and non-cloneable `MonotonicClockState<C>` in
`crates/telorgon/src/platform/clock.rs`. One state owner binds one managed, embedded, or deterministic host
source and its clock-domain assumption. Sampling accepts equal-resolution observations, advances
only for a valid nondecreasing value, and rejects regression with previous/observed instants while
retaining the last accepted observation. The trait deliberately has no `Send`/`Sync` requirement;
the host selects and serializes its runtime owner. No implementation reads wall time or ambient
`std::time::Instant`, and the package has no sleep, timer, scheduling, thread, callback, event loop,
or global state. Unit, direct-path, strict-lint, rustdoc, and umbrella fixtures cover ties,
advancement, atomic regression, object safety, and manually controlled deterministic sources. The
exact next eligible Gate 9 neutral-spine package is `crates/telorgon/src/platform/schedule.rs`: it must define
the immutable post-turn scheduling decision over remaining work, typed redraw views, the next
optional monotonic deadline, and pending host wake/completion facts without selecting Winit control
flow, requesting redraw, sleeping, polling continuously, spawning work, or owning a queue.
The one-hundred-eighty-ninth Gate 9 integration package adds `RemainingWork`, `PendingHostFacts`,
`PostTurnSchedule`, `ScheduleError`, and the 1,024-view `MAX_REDRAW_VIEWS` bound in
`crates/telorgon/src/platform/schedule.rs`. Construction rejects an oversized input before copying, then
sorts and deduplicates generation-safe `ViewId` values for deterministic equality and lookup. A
policy-free merge ORs independent runtime/host facts, forms a bounded sorted redraw union, and
retains the earliest present injected-domain deadline. No type reads a clock, chooses native
control flow, requests redraw, sleeps, polls, spawns work, invokes callbacks, renders, or owns an
event/completion queue. Unit, direct-path, strict-lint, rustdoc, and umbrella fixtures cover fact
independence, discovery-order normalization, bound rejection without truncation, unique union,
deadline selection, empty decisions, and thread-transferable publications. The next neutral-spine
package is `crates/telorgon/src/platform/services/mod.rs`: it must provide extensible typed handle storage
without a global command enum, native fallback, or cross-thread requirement.
The one-hundred-ninetieth Gate 9 integration package adds type-level `ServiceKey`, local-owner
`ServiceRegistry`, explicit `ServiceLookup`/`ServiceUnavailable`, and ownership-preserving
registration, replacement, and removal outcomes in `crates/telorgon/src/platform/services/mod.rs`. Registry
identity is the concrete key type rather than the handle representation, so independent service
families can use identical `Rc<dyn Trait>`-style handles without downcast ambiguity. Lookup borrows
without cloning; duplicate registration preserves the old entry and returns the rejected handle;
replacement/removal return displaced ownership. Absence never fabricates a service. The registry
imposes no `Send`/`Sync` bound and owns no operation vocabulary, request, completion, permission,
native API, thread, executor, callback, queue, or blocking call. Unit, direct-path, strict-lint,
rustdoc, and umbrella fixtures cover absent lookup, duplicate atomicity, same-handle key
disambiguation, deterministic replacement/removal, owner-thread `Rc` handles, and redacted debug
output. The one-hundred-ninety-first Gate 9 integration package adds the typed per-view
`WindowService` contract in `crates/telorgon/src/platform/services/window.rs`. It exposes per-operation
capability queries, bounded debug-redacted titles, validated view-logical size constraints, state,
attention, and close intentions, separate immutable request/applied records, and linear typed
admission. Applied receipts never become observed `ViewSnapshot` truth. The owner-local registry key
retains `Rc<dyn WindowService>` without a cross-thread requirement. Unit, direct-path, strict-lint,
rustdoc, and umbrella fixtures cover view generations, operation capability, bounds, redaction,
admission, completion, and explicit stale-view rejection without native calls or exit policy.
The one-hundred-ninety-second Gate 9 integration package adds the shared neutral data-transfer
vocabulary in `crates/telorgon/src/platform/services/data_transfer.rs`. Exact bounded MIME, UTI, and named
native-format identifiers form multi-format generation-aware offers with source, trust, and aligned
size hints. Buffered or streamed reads select one offered format, require a hard byte bound, retain
request identity in payload-free progress/completion metadata, and revalidate the exact offer
generation. Capability limits bound formats, reads, and chunks before admission. The service handle
only admits/cancels typed reads; it owns no content, native offer, allocation, I/O, queue, executor,
or fallback. Unit, direct-path, strict-lint, rustdoc, and umbrella fixtures cover exact formats,
bounds, stale offers, streaming metadata, redaction, capability advertisement, and linear admission.
The one-hundred-ninety-third Gate 9 integration package adds the typed `ClipboardService` contract
in `crates/telorgon/src/platform/services/clipboard.rs`. System and selection clipboards have independent
capability and availability facts, separate read/write permission, bounded exact formats, monotonic
snapshot identities, payload-free current/change publications, and optimistic-concurrency-aware
publish/clear requests with separate applied receipts. Absence and snapshot failure are explicit;
no in-process or no-op clipboard is fabricated. Unit, direct-path, strict-lint, rustdoc, and umbrella
fixtures cover scope separation, format capability, snapshot generations, redaction, publish/clear
admission, completion, and registry lookup. None of these three packages invokes Winit, arboard, or
a native platform, mutates view or clipboard observation, performs a synchronous content read, or
adds a global service command enum. The one-hundred-ninety-fourth Gate 9 integration package adds
the object-safe `TextInputService` contract in `crates/telorgon/src/platform/services/text_input.rs` and
depends directly on canonical `telorgon-text` protocol values rather than duplicating session IDs,
revisions, offsets, configuration, geometry, requests, or deltas. Per-view capability separates
input-method, virtual-keyboard, surrounding-text, selection, composition, and editor-action support
under a hard 64-KiB surrounding-text limit. A synchronization wrapper validates that surrounding
text is bounded, secure snapshots expose no plaintext, the active cursor is an in-range UTF-8
boundary, and view-logical caret/selection geometry is finite and nonnegative before linear
admission. Applied metadata remains distinct from text or platform observation. Platform-to-runtime
delta envelopes accept only canonical, already-converted `TextSessionDelta` values, cite the view,
session generation, and observed revision, and redact edit/composition content from diagnostics;
the owning `TextInputSession` remains responsible for stale-session/revision validation and actual
editing. Unit, direct-path, strict-lint, rustdoc, and umbrella fixtures cover capability limits,
session identity, secure and diagnostic redaction, UTF-8/range/geometry rejection, delta identity,
registry lookup, admission, and terminal completion. The package owns no text buffer, native range
conversion, Winit/input-method/virtual-keyboard object, callback, queue, executor, thread, or global
command enum. The one-hundred-ninety-fifth Gate 9 integration package first supplies the missing
neutral `telorgon-accessibility` owner promised by the architecture and then adds
`crates/telorgon/src/platform/services/accessibility.rs` around it. The canonical owner retains nonzero tree
generation/revision, mounted generational node identity, validated parent/child topology, resolved
redacted strings, view-logical layout geometry/transforms, distinct keyboard/assistive focus,
complete activation snapshots, exact-base node/string/focus deltas, and typed assistive action
data. Whole-tree construction and delta application reject unresolved merge/exclude inputs,
malformed/disconnected topology, missing relationships/strings/focus, invalid geometry, duplicate
or oversized records, and stale generation/revision before publication. Action requests preserve
the observed generation/revision and target, redact text from diagnostics, and cannot become a
platform event unless the exact current snapshot advertises the action. The platform service adds
per-view tree/action capability, hard limits, linear publication admission/completion, explicit
stale view/tree errors, and an owner-local object-safe registry key without duplicating any
semantic record. Unit, direct-public, umbrella, strict-lint, and rustdoc fixtures cover complete
trees, atomic deltas, cross-node validation, focus distinction, redaction, typed action data, stale
action rejection, capability, registry lookup, publication admission, and completion. Neither
package reconstructs semantics from pixels, invokes AccessKit or another native API, dispatches an
action, or owns an adapter, callback, queue, executor, event loop, or platform semantic source of
truth. Live regions, accessible text runs/ranges, imported-tree merging, runtime tree production,
native mapping, and platform qualification remain explicit later work. The one-hundred-ninety-sixth
Gate 9 integration package adds the object-safe per-view `CursorService` contract in
`crates/telorgon/src/platform/services/cursor.rs`. Appearance requests keep standard and custom selection
separate from visibility, while custom straight-alpha sRGB RGBA8 images validate nonzero physical
dimensions, an in-bounds hotspot, exact byte length, and hard dimension/byte limits. Custom
animation additionally validates multiple identical-geometry frames, bounded frame and cycle
durations, and hard frame-count/aggregate-byte limits. Pixel content is omitted from diagnostics.
Position requests can be constructed only from a coherent `ViewSnapshot`, cite its exact metrics
revision and canonical view-logical coordinate space, and reject nonfinite or out-of-view points;
the service neither reads a cursor position nor fabricates a leave sentinel. Confinement and lock
remain separate admitted requests whose success completes with a non-cloneable adapter-owned RAII
lease carrying a generational identity, view, kind, and active/revoked status. The concrete adapter
lease releases its long-lived effect on drop. Direct-public, umbrella, drop-behavior, strict-lint,
and rustdoc fixtures cover image/animation validation, redacted diagnostics, host-specific limits,
metrics citation, logical-position rejection, registry lookup, admission, terminal completion,
duplicate constraint rejection, and lease release. The package owns no Winit or native cursor,
physical-position conversion, native handle, callback, queue, executor, thread, event loop, clock,
or fallback policy. The one-hundred-ninety-seventh Gate 9 integration package adds the object-safe
retained-observation `DisplayService` contract in `crates/telorgon/src/platform/services/display.rs`.
Connected displays use owner-local generational `DisplayId` values so a reconnected display cannot
silently satisfy a stale identity. Complete service snapshots are hard-bounded to 64 records,
retain stable adapter order and one optional referentially valid primary identity, accept an
observed empty/headless state, and reject duplicate identities. Each descriptor validates finite,
nonempty display-logical bounds with negative desktop origins allowed, a nonzero physical extent,
and reuses the canonical `ScaleFactor` and `DisplayProperties` transform/color/HDR vocabulary.
Monotonic `DisplayRevision` values and complete immutable `DisplayChange` payloads reject
nonadvancing history. Capability separates current snapshots, change notifications, and exact-view
association, caps host display-count claims at the neutral hard bound, and exposes independent
accuracy claims for logical bounds, scale, transform, color, HDR, safe areas, and avoid regions.
`ViewDisplaySnapshot` rejects an associated identity absent from its exact cited display revision,
binds that optional generation to one exact `ViewSnapshot`, and retains its existing
`ViewMetricsSnapshot`; all scale, transform, color/HDR, safe-drawing, safe-gesture, and avoid-region
access delegates to that canonical publication. Explicit current,
unavailable, and structured-failure statuses prevent fallback observations. Direct-public,
generation, umbrella, object-safety, registry, strict-lint, and rustdoc fixtures cover invalid
geometry, hard bounds, duplicate/primary validation, empty topology, reconnection generations,
revision advancement, accuracy, canonical metrics identity, and retained service queries. The
package polls no native display source, retains no monitor handle, chooses no mode, and owns no
callback, queue, executor, thread, event loop, or fallback policy. The one-hundred-ninety-eighth
Gate 9 integration package adds the object-safe `UriService` contract in
`crates/telorgon/src/platform/services/uri.rs` plus the contract's general opaque `UserGestureGrant`
boundary. `UriScheme` validates the RFC 3986 ASCII scheme grammar, enforces a 64-byte hard bound,
and normalizes case for exact capability matching. `ExternalUri` accepts only an absolute ASCII
lexical envelope, enforces an 8-KiB hard bound, rejects whitespace/control/non-URI characters and
malformed percent escapes, and deliberately leaves scheme-specific authority/path/query meaning to
application and adapter policy. Its content is available only through explicit access and is
omitted from `Debug`, errors, request diagnostics, and applied receipts. Capabilities preserve up to
32 unique normalized schemes in adapter preference order, with independent permission, execution,
recent-gesture, and host URI-byte-limit facts per scheme. Open requests are view-scoped and complete
through the existing linear typed request owner; immediate errors distinguish unavailable views,
unsupported or changed scheme capability, denied permission, missing/invalid gesture evidence,
host-limit overflow, and admission capacity. `UserGestureGrantHandle` is a non-cloneable boxed
adapter value exposing no native token or scope data and permitting only private `Any` consumption.
Attaching it moves it into the request and redacts it from diagnostics; the issuing adapter then
validates concrete type, view, age, generation, focus, scope, and single use before any platform
call. Direct-public, lexical, redaction, hard-limit, capability, gesture,
object-safety, registry, umbrella, admission/completion, strict-lint, and rustdoc fixtures cover
those boundaries. The package performs no navigation or network I/O, launches no handler, exposes
no native URL/serial/token value, and owns no callback, queue, executor, thread, or event loop. The
one-hundred-ninety-ninth Gate 9 integration package adds the object-safe `FileDialogService`
contract in `crates/telorgon/src/platform/services/file_dialog.rs`. It models view-scoped asynchronous
open-file, save-file, and folder-selection intentions with independently discoverable operation,
multiple-selection, sandbox-grant, permission, execution, recent-gesture, and host-limit facts.
Filters combine normalized bounded file extensions with canonical `DataFormat` values; labels,
save-name suggestions, rule/filter counts, and selection counts all have neutral hard limits plus
narrower adapter-advertised limits. Applied selections contain one or more redacted `ExternalUri`
locators with exact file/folder kind, read/write intent, optional bounded redacted display names,
and optional opaque non-cloneable adapter grants. Result construction rejects empty or duplicate
selections, request-limit overflow, kind/access mismatches, and missing required grants. A normal
user dismissal is an applied typed result, not a cancelled admitted request. Requests optionally
move the same opaque adapter-validated gesture evidence used by URI intents and complete through
the existing linear typed request owner. Focused and umbrella fixtures cover bounds, normalization,
redaction, cross-field validation, grant destruction, capability discovery, private gesture
downcast, registry object safety, and admission/completion. The package opens no dialog, performs
no filesystem I/O, assumes no locator is a path, retains no native dialog/token object, and owns no
callback, queue, executor, thread, or event loop. The two-hundredth Gate 9 integration package adds
the object-safe `MenuService` contract in `crates/telorgon/src/platform/services/menu.rs`. Application,
per-view, and status/tray scopes own independent exact revision histories. Complete immutable trees
retain stable nonzero item identities across revisions and enforce hard 1,024-item, 16-level, and
512-accelerator bounds; global item, semantic-role, and accelerator identities are unique, and
sibling separator topology rejects leading, trailing, or adjacent separators. Action/submenu roles
are kind-checked, while action state keeps enabled, visible, and not-checkable/unchecked/checked/
mixed facts explicit. Labels and localized accelerator presentations are independently bounded and
redacted. Accelerators reuse the neutral physical `ShortcutChord`, require a pressed-key trigger,
and remain separate from their localized display text. Initial and advancing publications require
the exact scope and monotonic successor revision, with an empty complete tree serving as explicit
removal rather than a native-object command. Capabilities independently expose application, view,
status, native-role, accelerator, mixed-state, and action-event support plus adapter-narrowed tree
limits. Native action candidates become portable events only after validating the exact current
scope/revision/item, action kind, visibility, enabled state, and source-specific accelerator/role/
status advertisement. The event returns identity, role, and source only; it never invokes or owns a
command. Focused and umbrella fixtures cover redaction, kind/role constraints, topology, depth,
identity and accelerator ambiguity, exact revision succession, empty-tree removal, stale/invalid
actions, capability discovery, registry object safety, and linear publication completion. The
package retains no native menu or status object and owns no command callback, queue, executor,
thread, or event loop. The two-hundred-first Gate 9 integration package adds the object-safe
`NotificationService` contract in `crates/telorgon/src/platform/services/notification.rs`. Stable nonzero
notification and action identities plus exact per-notification revisions distinguish initial posts,
successor updates, exact-current removals, and responses without retaining native identifiers.
Titles, bodies, visible action labels, and inline replies have distinct 256-byte/4-KiB/256-byte/
4-KiB hard limits, reject empty/control text, and redact contents from all generic diagnostics.
Complete descriptors preserve priority, Public/Sensitive/Secret privacy, up to 16 unique actions,
and at most one default and dismiss role. Action-kind construction requires the unlabeled default
or a labeled visible Open/Reply/Dismiss/Custom action. Initial and update requests require the
initial or exact successor revision while preserving stable notification identity; removal cites
one exact current snapshot. Authorization requests validate nonempty alert/badge/sound/critical
dimensions and return observed permission through the existing linear completion owner. Numeric
badges use explicit clear/count intentions capped at 999,999. Capabilities independently expose
authorization, post, update, remove, action, inline-reply, badge, and response support with
adapter-narrowed action/body/reply/badge limits. Native response candidates become portable events
only after exact notification/revision validation and source-specific checks: body activation needs
the default action, visible action activation needs an advertised nondefault action, inline reply
needs an advertised Reply action plus bounded text, and user/system/expiry dismissal carries no
action or reply. Focused and umbrella fixtures cover bounds, redaction, action relationships,
identity/revision succession, removal, authorization, badges, stale/unknown response rejection,
registry object safety, and linear completions. The package has no delivery time or schedule,
retains no native notification or command object, invokes no action, and owns no callback, queue,
executor, thread, timer, or event loop. The two-hundred-second Gate 9 integration package adds the
object-safe `HapticsService` contract in `crates/telorgon/src/platform/services/haptics.rs`. Portable callers
select one of nine user-facing Selection/Activation/Toggle, impact, or notification effects rather
than supplying a waveform. `HapticEffectSupport` advertises an exact semantic subset, while
`HapticDeviceSupport` distinguishes a temporarily unavailable output from an available output and
states whether portable intensity control exists. Capability snapshots retain that device fact,
Enabled/Disabled/Unknown user-setting state, common permission/execution/gesture policy, and an
adapter-narrowed maximum intensity. Intensity uses exact fixed-point thousandths over the closed
normalized 0.0 through 1.0 range, rejecting nonfinite, out-of-range, above-hard-bound, and silent
maximum values; fixed-intensity devices must advertise the full normalized maximum. Requests carry
only effect, intensity, and optional opaque single-use gesture evidence. Applied results preserve
the admitted semantic intention, while typed immediate errors distinguish unsupported operation or
effect, device absence, disabled/unknown setting, permission/prompt/gesture policy, unsupported or
excessive intensity, capability change, and capacity exhaustion. Focused and umbrella fixtures
cover intensity bounds and quantization, effect-set composition, device/capability relationships,
setting enforcement, gesture evidence, unsupported effects, temporary absence, registry object
safety, and identity-bound linear completion. The package accepts no waveform, frequency, duration,
vendor identifier, or native device handle and owns no hardware driver, callback, queue, executor,
thread, timer, or event loop. The exact next eligible Gate 9 neutral-spine package is
`crates/telorgon/src/platform/services/power.rs`: it must model optional policy-aware idle and sleep
inhibition as explicit scoped linear leases with observable revocation and deterministic release,
without invoking power APIs, retaining native handles, choosing application policy, or owning a
callback, queue, executor, thread, timer, or event loop. The two-hundred-third Gate 9 integration
package adds the object-safe `PowerService` contract in
`crates/telorgon/src/platform/services/power.rs`. Requests distinguish Idle response from SystemSleep
inhibition and bind each intention to either application scope or one exact generational view.
InteractiveActivity, MediaPlayback, Presentation, and UserInitiatedWork are closed semantic policy
reasons; portable callers provide no arbitrary reason string, duration, deadline, or native token.
Capabilities independently expose both inhibition kinds, application/view scopes, common
permission/execution/gesture policy, observed Allowed/Denied/Unknown host policy, and an
adapter-narrowed concurrent-lease capacity capped by a 64-lease neutral hard bound. Optional opaque
single-use gesture evidence remains adapter-only. A successful linear completion owns one boxed,
non-cloneable `PowerInhibitionLease` with stable generational identity and exact scope/kind/reason.
Lease status distinguishes Active from ScopeClosed, ScopeSuspended, PermissionChanged,
PolicyChanged, or HostRevoked, while the concrete adapter destructor performs deterministic native
release. Typed immediate errors keep unsupported kinds/scopes, unavailable views, denied/unknown
policy, permission/prompt/gesture policy, lease-limit exhaustion, capability change, and admission
capacity distinct. Focused and umbrella fixtures cover capability dimensions, hard limits, query
scope, opaque gesture ownership, unsupported operation, capacity exhaustion, stable lease identity,
observable revocation, object-safe registry lookup, identity-bound completion, and exactly-once
drop release. The package invokes no power API, retains no native inhibitor, chooses no application
policy or timing, and owns no callback, queue, executor, thread, timer, or event loop. The exact next
eligible Gate 9 neutral-spine package is `crates/telorgon/src/platform/services/restoration.rs`: it must
model bounded opaque platform-restoration tokens with explicit application/view/session scope,
revision-safe publication and consumption, and redacted diagnostics without serializing runtime
state, reading storage, retaining native objects, choosing persistence policy, or owning a callback,
queue, executor, thread, timer, or event loop. The two-hundred-fourth Gate 9 integration package
adds the object-safe `RestorationService` contract in
`crates/telorgon/src/platform/services/restoration.rs`. Application scope, exact generational view scope,
and a new exact generational `RestorationSessionId` own independent histories. Every snapshot cites
one nonzero monotonic revision; initial publication requires revision one, advancing publication
requires the immediate successor in the identical scope, and exact-current clearing remains a
separate request family. `RestorationToken` owns a nonempty opaque byte sequence capped at 64 KiB,
is intentionally non-cloneable, and redacts contents from token, record, publication, consumption,
result, and error diagnostics. Consumption moves one complete exact record into admission and an
applied completion returns that same single owned token to portable code. Capability independently
exposes publish, update, consume, clear, application, view, and session support with an
adapter-narrowed token-size limit plus common permission/execution policy. Typed immediate errors
distinguish unsupported operations/scopes, unavailable views/sessions, permission/prompt state,
token limit, unavailable snapshot, stale revision, capability change, and admission capacity.
Focused and umbrella fixtures cover token bounds/redaction, generational scopes, revision
initialization/succession/exhaustion, cross-scope rejection, exact-current clearing, single-owner
consumption return, capability narrowing, stale requests, object-safe registry lookup, and
identity-bound completions. The package transports but never interprets tokens, serializes runtime
state, accesses storage, retains native restoration objects, or chooses persistence policy, and it
owns no callback, queue, executor, thread, timer, or event loop. This completes the planned neutral
service-family files. The exact next eligible Gate 9 package is the new
`telorgon-platform-conformance` crate: it must provide a deterministic fake clock, lifecycle/view
driver, bounded event/completion capture, and fake service adapters that exercise the neutral
contracts without native APIs, ambient time, background threads, renderer ownership, or fallback
behavior. The two-hundred-fifth Gate 9 integration package adds the new
`telorgon-platform-conformance` workspace crate as a one-way consumer of `telorgon-platform`.
`FakeClock` samples without advancing and changes only through explicit nonregressing set/advance
operations with typed regression and overflow rejection. Generic `BoundedCapture<T>` caps one
ordered trace at 4,096 entries, rejects before growth, returns rejected item ownership, and omits
generic payloads from saturation diagnostics; event and terminal-completion aliases preserve the
same behavior. `ViewDriver` owns up to 64 independent canonical `ViewState` instances, rejects
duplicate/missing/capacity cases, retains redundant observations, and preflights update-trace
capacity so saturation cannot mutate view truth. Generic `DeterministicHost<T>` combines that
driver with one manual clock, `EventStampStream`, and bounded `PlatformEvent<T>` capture. Missing
views, full capture, and invalid source time return the untouched redacted payload before sequence
or trace advancement; identical two-view input scripts reproduce identical revisions, stamps, and
events. `FakeHapticsService` and `FakeRestorationService` implement the actual object-safe neutral
traits, revalidate current capability, issue deterministic request IDs, and retain bounded
payload-free invocation metadata. Restoration admission remains distinct from explicit observed
snapshot changes, and an admitted consumed token remains single-owner in a bounded request-ID map
until terminal completion. Focused fixtures cover manual time ties/advance/regression/overflow,
capture saturation and completion identity, deterministic two-view replay, atomic full-trace
rejection, service-registry installation, haptic effect/capacity rejection, restoration
publication/observation separation, stale consumption, returned opaque token ownership, and exact
clear observation. The crate has no Winit, native API, renderer, ambient clock, executor, thread,
timer, event loop, automatic dispatch, or fallback service. Broader input/text/accessibility/
transfer/service fakes and end-to-end runtime/scene/semantics replay remain planned. The
two-hundred-sixth Gate 9 integration package adds the isolated `telorgon-platform-winit` workspace
crate as a direct consumer of only `telorgon-platform` and Winit. Its `ViewRegistry` has an explicit
configurable simultaneous-view capacity capped at 1,024 and owns both directions of the Winit
`WindowId`/neutral `ViewId` relationship. Registration rejects duplicate native identity and full
capacity before mutation, deterministically reuses the lowest eligible retired owner slot, and
increments its nonzero generation; permanently exhausted generation slots are skipped and terminal
identity-space exhaustion is typed. Native-window replacement requires the exact current view and
expected window plus a distinct unowned replacement, changes both lookup directions atomically,
and preserves the logical `ViewId`. Retirement likewise requires the exact current pair, removes
both lookup directions, and leaves stale view generations and old native identities unable to
target a later occupant. Ordered iteration exposes active mappings in owner-slot order. Focused
fixtures cover multi-view lookup and bounds, duplicate/conflicting mutation atomicity, replacement,
stale native retirement, generation-advancing reuse, stale callback isolation, and terminal
generation exhaustion. The crate creates no window or event loop and owns no `ApplicationHandler`,
renderer, presenter, runtime dispatch, or fallback. The two-hundred-seventh Gate 9 integration
package adds `crates/telorgon/src/platform_winit/event_proxy.rs`. `CompletionEvent<T>` privately owns one
typed completion, exposes only shared borrowing or ownership-consuming extraction, and is
intentionally neither Clone nor Copy so a linear neutral `RequestCompletion` cannot be duplicated
at this boundary. `CompletionEventProxy<T>` consumes a caller-created Winit
`EventLoopProxy<CompletionEvent<T>>`, requires a cross-thread `Send + 'static` payload, and is
itself Clone/Send/Sync without requiring `T: Clone`; cloning copies only Winit's wake/send
capability. Sending moves the completion exactly once. Winit's closed-loop return is converted into
a typed `CompletionSendError<T>` that returns the original undelivered completion intact, reports
an explicit `EventLoopClosed` kind, and redacts the generic payload from Debug and Display. It does
not retry, dispatch, or create a fallback path. Pure and compile-only fixtures cover an actual
non-cloneable neutral terminal completion, shared/consuming envelope access, closed-loop ownership
return, redacted diagnostics, non-Clone payload proxy bounds, cross-thread proxy bounds, and the
public constructor signature without constructing or running an event loop. The package owns no
queue, executor, thread, event-loop execution, runtime dispatch, or fallback. The
two-hundred-eighth Gate 9 integration package adds
`crates/telorgon/src/platform_winit/schedule.rs`. `WinitClockObservation` pairs an explicit neutral
`MonotonicInstant` with one caller-observed native `Instant` and never samples or infers a clock
origin. `interpret_schedule` resolves every normalized bounded redraw `ViewId` through the exact
current `ViewRegistry` generation into an owned `(ViewId, WindowId)` target; stale, retired, or
unknown generations reject the whole plan before any Winit action, while native-window replacement
under the same logical view resolves to the current Winit identity. Idle schedules select `Wait`;
future neutral deadlines use checked relative-domain arithmetic to select `WaitUntil`, with typed
native-instant overflow. Due deadlines, remaining update/layout/semantics/scene work, and pending
host/service work remain independent from Winit control flow: they select `Wait` plus an explicit
coalescible `RequestWake`, or `WakeAlreadyPending` when the neutral schedule reports an existing
wake. The interpreter never selects `Poll`; that remains reserved for a separately explicit
continuous-frame profile. Focused fixtures cover idle/future/deadline-due plans, unfinished work,
pending-wake coalescing, two-view redraw ordering, current native replacement, and stale-generation
slot-reuse isolation. The slice calls no `set_control_flow`, `request_redraw`, window, event-loop, or
clock API and owns no callback, queue, executor, thread, runtime dispatch, or fallback. The
two-hundred-ninth Gate 9 integration package adds
`crates/telorgon/src/platform_winit/translate/window.rs`. `WinitWindowFact::from_event` copies only supported
data from a
borrowed Winit event; scale-factor selection deliberately ignores the borrowed `InnerSizeWriter`,
and input, IME, data-transfer, theme, and redraw variants return `None` for their dedicated owners.
Contextual translation requires the callback's current `WindowId` to resolve through
`ViewRegistry` to the identical generational `ViewId` cited by a caller-supplied `ViewSnapshot`.
Stale native identity and cross-view snapshot mismatches are typed failures before an observation
is produced. Resize maps Winit's unsigned physical client extent directly into `PhysicalExtent`,
retaining either zero dimension as explicitly non-renderable rather than clamping. Scale retains
the original Winit `f64` observation for diagnostics while validating its narrowed neutral
`ScaleFactor`, including nonfinite, nonpositive, overflow, and underflow rejection. Focus and
occlusion remain distinct copied booleans for later input/lifecycle owners rather than causing
state mutation. Winit `CloseRequested` becomes a cancellable user `CloseRequest` citing the exact
snapshot revision, while `Destroyed` becomes an unanswerable `ForcedDestruction` at the distinct
Destroyed phase. Focused fixtures cover physical zero extent, valid/invalid scale, focus/occlusion
separation, unsupported-event nonconsumption, revision-bound close/destruction, stale replaced
window identity, and snapshot/view mismatch. The slice mutates no `ViewState`, invokes no
`InnerSizeWriter`, window, event-loop, dispatch, close, or exit method, and owns no callback, policy,
queue, renderer, presenter, or fallback. The two-hundred-tenth Gate 9 integration prerequisite
expands `crates/telorgon/src/input/keyboard.rs` into the complete adapter-ready neutral keyboard vocabulary.
`PhysicalKeyCode` owns all 194 standardized physical positions and maps them into stable nonzero
`PhysicalKey` identities while reserving zero for `UNIDENTIFIED`; the existing application-assigned
numeric constructor remains only as the compatibility shortcut seam. `NamedKey` owns all 306
standardized non-character meanings, while `LogicalKey` keeps character, named, dead-key, and
unidentified meanings distinct. `KeyLocation` preserves standard, left, right, and numpad
locations. `KeyText` owns exact UTF-8 behind a hard 4-KiB bound and redacts content from Debug while
retaining byte length. `KeyEvent` now carries independent physical and logical meanings, optional
produced text, location, state, repeat, synthetic origin, and a fourteen-flag modifier/lock-state
snapshot. Its constructor supplies explicit unidentified/absent/standard/nonrepeat/nonsynthetic
defaults. Because bounded text is owned, `KeyEvent` and `InputEvent` are Clone rather than Copy;
cloning text shares immutable storage. Existing physical shortcut chord and matcher signatures,
scope ordering, exact-modifier behavior, trigger behavior, and repeat policy remain unchanged and
focused fixtures prove logical meaning, text, location, and synthetic origin do not alter a
physical chord match. The crate imports no Winit or native keyboard enum and retains no native
numeric unidentified value. The two-hundred-eleventh Gate 9 integration package adds the pure
`crates/telorgon/src/platform_winit/translate/keyboard.rs` boundary. Its explicit tables map all 194 current
Winit `KeyCode` values and all 306 current Winit `NamedKey` values into the neutral owner; future
non-exhaustive values and native unidentified physical/logical payloads collapse defensively to
neutral `Unidentified` instead of becoming portable native codes. `WinitKeyboardInput` borrows only
callback-scoped physical, logical, produced-text, location, state, repeat, and synthetic fields;
its Debug representation redacts character, dead-key, and produced text. Contextual translation
first resolves the callback's exact current `WindowId` generation through `ViewRegistry`, then
constructs one owned `WinitKeyboardObservation` with the logical `ViewId` and neutral `KeyEvent`.
Stale/replaced windows reject before text conversion. The caller supplies the exact current neutral
modifier snapshot plus an explicit produced-text policy: text is hard-bounded and retained outside
composition, while active IME composition suppresses only `KeyEvent.text` so commit remains the
single mutation path. Logical character meaning remains present. Aggregate Winit modifier changes
map only Shift, Control, Alt, and Super; the adapter does not invent unavailable side or lock state.
Oversized logical/produced text returns a typed, field-specific, view-scoped, content-redacted
failure. Focused fixtures prove complete table cardinality/uniqueness, named/character/dead/unknown
distinction, location/transition/repeat/synthetic preservation, caller modifier preservation, IME
suppression, hard bounds, redaction, stale-window rejection, and unsupported-event nonconsumption.
The slice mutates no state, retains no event/native text reference, invokes no Winit method, and
owns no callback, queue, event loop, runtime dispatch, renderer, presenter, or fallback. The
two-hundred-twelfth Gate 9 integration package completes the neutral pointer/scroll prerequisite
in `crates/telorgon/src/input/pointer.rs` and `event.rs`. Generation-aware optional device identity remains
separate from stable contact identity. Validated optional positions retain canonical view-logical
coordinates and, when supplied, the original physical observation without sentinels. Canonically
ordered, bounded pressed-button snapshots accompany explicit enter, leave, hover, move, button,
cancel, and capture lifecycle changes; button edges must agree with the complete snapshot and
cancellation must release every button. Pressure, tilt, twist, positive contact geometry, primary
contact, native/synthesized source, and modifier facts remain independently optional or explicit.
Scroll observations retain both axes and their original Pixels, Lines, or Pages meaning plus
gesture phase, independent momentum phase, precision classification, source, modifiers, identity,
and optional position. Only logical pixel deltas may retain an additional physical-pixel source,
so the value layer never invents line/page conversion constants. A separate `PointerInputEvent`
admits the complete canonical records while the mounted runtime's existing `InputEvent` pointer
variants remain an explicit compatibility seam; this package does not silently change routing
behavior. Focused unit and public-path fixtures cover identity generations, bounds, sorting,
coordinate/property rejection, edge/snapshot consistency, coordinate-free leave/cancel,
two-axis scroll semantics, momentum/precision, physical-source restrictions, and umbrella/app
reexports. The package imports no Winit/native type and owns no adapter state, capture policy,
callback, queue, event loop, runtime dispatch, renderer, presenter, or fallback. The
two-hundred-thirteenth Gate 9 integration package adds the pure Winit cursor/mouse/scroll boundary
in `crates/telorgon/src/platform_winit/translate/pointer.rs`. It selects callback-scoped `CursorEntered`,
`CursorLeft`, `CursorMoved`, `MouseInput`, and `MouseWheel` facts without consuming unrelated
events. `WinitPointerContext` pairs the callback's native device with one complete caller-owned
neutral mouse snapshot and the exact metrics revision under which its optional current position
was converted. Translation first resolves the current `WindowId` generation, requires the supplied
`ViewSnapshot`, context metrics, and event device to agree, and rejects a non-mouse context before
converting payload data. Physical cursor positions become unclamped view-logical positions while
retaining the original physical observation. Standard buttons map explicitly; Winit `Other(u16)`
buttons use a distinct neutral namespace so native code 1 cannot collide with Primary. Press and
release edges atomically derive a new bounded complete button snapshot without mutating the caller,
and repeated edges remain idempotent. Pixel wheel deltas are divided by the exact retained scale,
retain their original physical delta, and cite that metrics revision; line deltas remain two-axis
Lines with no invented multiplier or metrics citation when no retained position needs one. All
four Winit wheel phases map explicitly, pixel/line precision remains Precise/Discrete, and Winit's
absence of momentum data remains neutral None. Typed failures cover stale windows, view/metrics/
device mismatches, wrong context class, nonfinite coordinates/deltas, button-bound failure, and
neutral invariant failure. Focused fixtures cover every selected callback class, unsupported-event
nonconsumption, optional enter/leave position, exact scale conversion/citation, outside positions
without clamping, complete press/release state, platform-other namespacing, hard bounds, pixel/line
axes/units/phase/precision, stale-first rejection, and caller-state nonmutation. The slice retains
no event reference or native device identity in its neutral payload, invokes no Winit method, and
owns no callback, mutable device/contact registry, queue, coalescing policy, capture policy, event
loop, runtime dispatch, renderer, presenter, or fallback. The exact next eligible Gate 9 package is
the pure Winit touch translator in `crates/telorgon/src/platform_winit/translate/touch.rs`: it must select
`WindowEvent::Touch`, require a caller-owned generation-safe `(DeviceId, touch id)` contact mapping,
convert physical location with an exact metrics citation, preserve normalized pressure when Winit
supplies force, map Started/Moved/Ended/Cancelled into complete touch-contact state without native
identity leakage, reject stale views/metrics/contact mismatches, and mutate no contact registry.
Selected dependencies, code boundaries, implementation slices, and operational exit criteria are in
[VULKAN_IMPLEMENTATION_PLAN.md](VULKAN_IMPLEMENTATION_PLAN.md). Current crates and APIs must reach
that direction through the staged, single-owner moves in [MIGRATION_PLAN.md](MIGRATION_PLAN.md).
The complete replacement scene/GPU records and shader bundle are specified in
[SCENE_GPU_ABI_AND_SHADERS.md](SCENE_GPU_ABI_AND_SHADERS.md); the implemented ABI records and box
bundle do not promote the current dense scene or unimplemented shader variants. The planned platform sequence is specified
in [PLATFORM_IMPLEMENTATION_ORDER.md](PLATFORM_IMPLEMENTATION_ORDER.md); it likewise does not imply
that Windows Vulkan, hosted rendering, Linux, Metal/macOS, Android, or iOS is operational.
The planned acceptance suites and production report model are specified in
[ACCEPTANCE_AND_QUALIFICATION.md](ACCEPTANCE_AND_QUALIFICATION.md); documenting a test ID, profile,
golden, or device matrix does not imply that the suite exists or passed.
The planned mount-once reusable component runtime, owner-scoped state/reactivity, typed action
routing, keyed containers, task scopes, and revisioned text-editing model are specified in
[AUTHORING_AND_COMPONENT_RUNTIME.md](AUTHORING_AND_COMPONENT_RUNTIME.md); this does not promote the
current one-root `Application::mount`/`UiBuilder` path to that target capability.
The planned foundation/primitive/component split, Tier A accessible controls, later application/tool/
game catalogs, and protocol-neutral shell UI are specified in
[APPLICATION_AND_SHELL_PRIMITIVES.md](APPLICATION_AND_SHELL_PRIMITIVES.md); documenting those names
or behaviors does not make the current shared builder controls either domain implementation.
The planned lifecycle, managed/embedded host, input/IME/accessibility, service, data-transfer, and
native-resource contract is specified in
[PLATFORM_INTEGRATION_CONTRACT.md](PLATFORM_INTEGRATION_CONTRACT.md); the current Winit/Softbuffer
proof does not satisfy it merely because it opens a window and delivers basic input.

## Composable window chrome, assets, icons, and pointers

This package is operational at the portable/unit and compile-integration evidence layers. A typed
`asset_catalog!` embeds project media and supplies bounded raster/SVG decoding shared by GUI and
desktop runtimes. `BoxDecoration` is the common background/border/outline/corner/shadow vocabulary
for ordinary boxes and window frames. Managed windows may hide system decorations and mount a
type-state `WindowFrame`; the Wayland compositor builds one frame per `WindowChromeModel`. Both
hosts derive title, app-icon, drag, resize, content, and action regions from computed layout.
`WindowFrameTemplate` accepts named templates while preserving closure compatibility.
`EasyWindowFrame` resolves active/inactive palettes, normal/maximized/tiled/fullscreen geometry,
client/fallback icons, capability-filtered controls, maximize/restore artwork, all eight resize
directions, border-confined L-shaped corner targets, outward-only hit slop, and
resting/hovered/pressed/focus-visible/disabled control visuals from one code-defined
`WindowChromeDesign`. The low-level type-state frame retains unrestricted normal
composition, explicit hit priority/cursor requests, tiled-edge control, and declaration-authorized
custom shell actions. `Compositor::background` is the normalized visual name; `.policy` is a
deprecated compatibility alias.

Focus palette changes reconcile the retained frame runtime and emit only that frame's neutral scene
delta. The selected backend applies the delta to its own retained scene: Vulkan records the changed
frame directly in the shared desktop pass, while software rasterizes it directly into the damaged
output region. No composed layer is rasterized through software for use by Vulkan.

`AppIconProfile` feeds native window metadata and compositor fallback art. The Linux server also
implements staging `xdg_toplevel_icon_v1` name/SHM-buffer assignment with commit synchronization,
immutability, reset, replacement, and lifetime tracking. Semantic pointer requests, code-local
overrides, registered TOML themes, hotspots, different per-state sizes, and animation frames are
resolved by both managed Winit and the Linux desktop runtime. This work has portable tests and
Windows/Linux compilation evidence but is not production-qualified on a physical compositor or
desktop hardware matrix. The complete API and invariants are documented in
[Custom windows, assets, icons, and pointers](CUSTOM_WINDOWS_ASSETS_AND_POINTERS.md).

## Documentation rule

Architecture documents describe the intended end state. Capability and release documentation must
use the status vocabulary above. In particular:

- shader assets do not prove shader execution;
- synthetic resource identifiers do not prove graphics-resource lifetime;
- predicted batch counts do not prove submitted draw calls;
- CPU byte counts do not prove GPU transfer behavior;
- retained semantic data does not prove platform accessibility; and
- a desktop mouse path does not prove touch or mobile support; and
- a mount-once application test does not prove reusable components, reactive dependency tracking,
  scoped tasks, or editable-text/IME behavior; and
- a painted/current control does not prove Gate 8 keyboard, touch, directional, semantic, adaptive,
  cancellation, controller, tier, or domain-isolation behavior.

Status changes require an operational integration test or a documented production qualification
report appropriate to the subsystem. The evidence layer and report rules are controlled by
[Acceptance and qualification](ACCEPTANCE_AND_QUALIFICATION.md); `skip`, `unsupported`, `waived`,
modeled, and offscreen results are not interchangeable with `pass` at a required higher layer.

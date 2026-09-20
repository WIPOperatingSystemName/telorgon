# Screen sharing implementation plan

## Status and scope

This is a proposed implementation sequence, not a current capability claim. No capture,
portal, or PipeWire implementation is introduced by this document. Current capability remains
governed by `IMPLEMENTATION_STATUS.md`.

Implementation progress and remaining gates are tracked separately in
[`SCREENCAST_IMPLEMENTATION_STATUS.md`](SCREENCAST_IMPLEMENTATION_STATUS.md).

Goal: a Telorgon desktop can share its monitor or a selected native Wayland/XWayland window
through the XDG ScreenCast portal and PipeWire, with modern Wayland capture protocols and
optional wlr-screencopy compatibility backed by the same capture engine.

The first milestone is one real monitor stream through the portal using shared-memory video
buffers. GPU rendering remains GPU rendering; readback is an explicit export operation, not a
software-renderer fallback. Audio sharing, remote input, virtual outputs, persistent grants,
HDR streaming, multi-plane video formats, and simultaneous multi-source selection are deferred.
Output identities should allow future multi-output support, but this project does not undertake
the host's outstanding multi-output/hotplug reconstruction work.

## Architecture decisions

Keep implementation in the existing `telorgon` package. Proposed new modules and filenames below
are internal organization, not new published crates or promises of public API stability.

| Location under `crates/telorgon/src` | Responsibility |
| --- | --- |
| `shell/capture.rs` | Neutral source identities, metadata, options, capability and lifecycle values |
| `application_host/shell_wayland/capture/` | Session authorization, scheduling, source resolution and completion routing on the existing owner thread |
| `compositor_render/capture.rs` | Validated capture jobs and destination-buffer bridge |
| `render/readback.rs`, `renderer_vulkan/readback.rs` | Reuse/extend explicit asynchronous readback contracts |
| `renderer_vulkan/` | GPU capture target execution and later DMA-BUF interoperability |
| `compositor_wayland/capture.rs`, `foreign_toplevel.rs`, `native/` | Protocol resource state, request validation and event dispatch |
| `wayland_server/protocol.rs`, package `build/` | XML profile and pinned wire contract |
| `portal_linux/` | Backend D-Bus requests, request/session objects and cancellation |
| `screencast_linux/` | PipeWire negotiation, stream buffers and delivery |
| `shell_components/` | Default chooser and sharing indicator; no authority or transport ownership |
| `application_host/declaration.rs`, `lib.rs`, Cargo features | Configuration, assembly and curated exports |
| `packaging/portal/` | Backend discovery, desktop portal selection and integration instructions |

The host owns live capture sessions. Neutral shell values own no protocol objects, file
descriptors or GPU resources. Renderer modules know nothing about D-Bus, user consent or PipeWire.
Each transport adapter uses the same host source selection and capture execution.

For the first implementation, assemble the portal backend in the managed compositor process.
Use a dedicated backend bus name and register the backend only after the host is ready. Do not
install a D-Bus activation command that starts a second compositor. Document host-started service
ownership and portal discovery; a separate helper process is a future packaging option, not a
requirement for this implementation. D-Bus/PipeWire execution uses explicit bounded bridges to
the owner thread. Their callbacks never mutate compositor or renderer state directly.

## Phase 0 — Freeze contracts and integration choices

Before changing GPU ownership, complete the required reference audit in
`REFERENCE_IMPLEMENTATIONS.md`. The adjacent `../other-rendering-libs` directory was absent
during planning. No independent implementation review is claimed. Restore/access the references
and inspect at least two implementations for capture buffer lifetime and synchronization; record
paths, revisions, extracted invariants, rejected alternatives and derived tests. Cross-check
Vulkan external-memory/synchronization requirements before selecting the DMA-BUF design.

Inspect the current shell frame, readback and Linux DMA-BUF paths end to end. Existing import
support is not evidence of capture export capability. Confirm the baseline build and distinguish
pre-existing failures in the active working tree from failures introduced by this work.

Freeze these initial choices:

- Sources reuse `OutputId` and generational `WindowId`; capture session/request IDs are separate.
- The host resolves identity to current presentation; remaps cannot silently retarget a grant.
- First release supports one source per session, multiple bounded sessions, SDR packed RGB,
  hidden/embedded cursors and no persistent grants. Publish only implemented capabilities.
- Capture limits include sessions per requester, buffers in flight, dimensions, aggregate bytes
  and frame rate. Choose explicit defaults after estimating worst-case allocation sizes.
- Request admission is asynchronous. Stop/revoke is idempotent. Every admitted frame completes
  once or fails; late completion after cancellation only retires resources.
- Locking stops sessions and prevents lock-screen capture. Source destruction stops its stream.
  Initially stop a window stream on unmap/minimize rather than promise fresh hidden content.
- No restore token or capture permission is inferred from a title, PID or reused numeric ID.

Pin the supported portal interface version and PipeWire binding/API versions in the implementation
review. Reuse the existing zbus dependency where suitable. Pick dependencies compatible with
Telorgon's feature isolation and native ABI policy; do not introduce a compositor framework.

Exit: reviewed contracts, concrete resource limits, protocol-version matrix, dependency decision
and reference audit. Fine-grained GPU implementation remains gated on that audit.

## Phase 1 — Neutral API and owner-thread session engine

Add source descriptors, cursor modes, options, typed failure/stop reasons, capability snapshots
and capture IDs. Keep buffer and transport types private to their execution layers. Use existing
shell request/result conventions instead of introducing a second general service framework.

Create host session states for requested, authorized, negotiating, streaming and stopped.
Maintain one source registry derived from existing output/window truth. Reuse
`shell_wayland/window_identity.rs` for native and X11 identity mapping.

Expose owner-local operations to enumerate eligible sources, request selection, accept/reject,
start, stop and revoke. A grant binds requester, source and session; adapters cannot substitute
another source after approval. Add bounded event-loop messages and wakeups for external adapters.

Tests: invalid/stale identities; recycled XIDs; source removal; cancellation in every state;
requester disconnect; quota exhaustion; duplicate stop; late completion; denied authorization.

Exit: deterministic lifecycle tests pass with fake frame completion and no native service run.

## Phase 2 — Real monitor capture into reusable shared-memory buffers

Add capture job preparation to the host's existing frame orchestration and a destination bridge
in `compositor_render`. Capture the composed output into a safe target or copy from a valid
render target before its lifetime ends. Do not retain an acquired KMS target for a slow consumer.

Reuse asynchronous Vulkan readback after auditing its staging reuse and allocation behavior.
Use a bounded reusable staging/delivery pool rather than allocating a frame-sized vector every
frame. Keep GPU completion and downstream buffer availability as distinct states. No GPU waits,
PipeWire work or blocking pixel copies belong in native protocol dispatch.

Account for output transform, scale, stride, orientation and target encoding. Start with explicit
SDR format/color metadata. Include hardware-plane cursor pixels when embedded mode is requested,
even if those pixels were absent from the primary render target. A stationary desktop must still
produce an initial frame; cursor movement must update an embedded-cursor stream.

When consumers are slow, skip frames while preserving the next frame's required damage. Resizing
creates a new buffer generation; old buffers retire after their outstanding uses finish.

Tests: known image patterns, alpha/format conversion, transform/scale, initial frame with no damage,
cursor-only motion, bounded memory under backpressure, resize with outstanding work and cancellation
after GPU submission. Use existing compile-only GPU fixtures plus a documented user-run GPU check.

Exit: real monitor frames can be delivered to a test sink with correct lifetime and bounded storage.

## Phase 3 — Portal, PipeWire and chooser: first usable milestone

Add `portal_linux` and `screencast_linux`, gated by a proposed `shell-screencast-linux` feature
which includes `shell-wayland-linux`. Capture protocol support should remain separable from the
PipeWire/portal dependency feature. Wire module declarations and optional native dependencies so
ordinary applications and embedded builds do not acquire the new dependency requirements.

Implement the backend `org.freedesktop.impl.portal.ScreenCast` contract, including request/session
objects, source selection, Start responses, cancellation and caller loss. The existing
xdg-desktop-portal service owns the application-facing API, including OpenPipeWireRemote; do not
duplicate it inside Telorgon. Confirm restricted PipeWire visibility/access using the portal's
integration rules rather than simply exposing a globally accessible video node.

Implement format negotiation and shared-memory PipeWire buffers, timestamps, stride metadata,
stream errors and disconnect cleanup. Start succeeds only when a usable stream is ready. Failure
stops that sharing session without stopping the desktop. Use the PipeWire buffer recycle lifecycle
before writing a delivery buffer again.

Provide a default source chooser and persistent sharing indicator with a stop action. A custom
chooser submits decisions to the same host authority. Previews, if added later, are capture
consumers with the same accounting. Backend startup/readiness must tolerate the frontend portal
starting before the compositor is ready.

Add backend `.portal` discovery data, a desktop-specific `*-portals.conf` example, environment
integration instructions, required packages, startup ordering and diagnostics under
`packaging/portal/`. Scope backend selection to the Telorgon session; do not overwrite unrelated
desktop configuration. Reuse session environment machinery for WAYLAND_DISPLAY and
XDG_CURRENT_DESKTOP and document the session-bus ownership assumptions.

Tests: backend method/state contract with a fake bus/stream adapter, user denial, cancellation
during chooser/negotiation, backend reconnect and feature isolation. Provide an opt-in live test
procedure using the portal frontend and a recorder/browser, followed by Discord.

Exit: a user can select the monitor, see moving frames in a portal consumer, stop sharing and
repeat the operation without restarting the compositor. This is the first product milestone.

## Phase 4 — Individual native and XWayland windows

Resolve a selected WindowId to its current client subtree and render it into an independent target.
Never implement window capture by cropping the output. Capture must exclude unrelated windows and
background pixels even while the selected window is occluded or moved.

Define one explicit initial composition policy: client content and owned subsurfaces/popups,
without server decorations. Preserve client-side decorations as client content. Audit background
sampling effects so a window stream cannot accidentally reveal other desktop content. Treat
unmanaged X11 windows separately; do not advertise them as selectable windows until ownership and
association are understood. Do not share the main desktop's culling decision for window capture.

Test native and X11 sources identically: occlusion, move, resize, transient popup association,
surface replacement, XWayland loss, recycled IDs and unmap/minimize termination. Verify the stream
never changes source silently. Advertise WINDOW only after these paths work.

Exit: both window types can be selected and shared with correct isolation and lifecycle.

## Phase 5 — Modern Wayland capture adapters

Extend the build-time XML profile and `build/protocol-wire-contract.txt` for
`ext-foreign-toplevel-list-v1`, `ext-image-capture-source-v1` and `ext-image-copy-capture-v1`.
Implement protocol object lifetime/state separately from the host engine and translate requests
into existing source resolution and capture jobs. Implement required constraints, frame events,
damage, timestamps and terminal failure behavior for the advertised versions.

Define explicit capture-global access policy before advertising these globals. Initially allow
only host-authorized trusted connections; deny ordinary clients by default. A public enablement
option must describe its access implications. Portal consent alone does not protect direct capture
protocols. Apply appropriate policy to window discovery as well as frame access.

Tests: generated wire-contract validation; malformed requests; wrong buffer dimensions/formats;
destroyed resources; source disappearance; generation change; repeated/invalid frame operations;
and unauthorized binds. Verify there is still only one capture renderer and source authority.

Exit: an authorized external capture client can record outputs and windows through these protocols.

## Phase 6 — DMA-BUF performance path and wlr compatibility

After the phase-0 graphics audit, add DMA-BUF delivery with exact format/modifier/usage negotiation,
device compatibility checks and an explicit fallback to shared-memory delivery. Do not assume
importable client images are exportable capture targets. Specify FD ownership and acquire/release
synchronization across renderer, PipeWire and protocol consumers. GPU completion alone does not
authorize recycling a consumer-held image.

Implement wlr-screencopy v3 as another adapter for outputs/regions, using the same authorization,
capture scheduler and renderer bridge. Obtain its XML as an explicitly pinned build input; it is
not part of the official wayland-protocols source tree. Apply existing license/provenance rules.

Measure end-to-end frame latency, copies, bandwidth, CPU time, dropped frames and retained bytes.
Test shared-memory fallback, modifier rejection, consumer-held buffers, device/stream failure,
fractional scaling and cursor behavior. Do not claim universal zero-copy support.

Exit: qualified DMA-BUF operation on documented hardware and compatibility capture without a
second execution pipeline.

## Verification and release handoff

For each phase, run formatting, the relevant unit/integration tests and feature-specific compile
checks. Maintain no-default-feature and embedded/application build coverage. Update current-state
documentation only for behavior actually implemented and validated; distinguish portable state
tests, GPU compilation and real hardware evidence.

Repository guidance forbids the agent from launching GUI applications, services or background
processes. Leave actual compositor, portal, PipeWire and Discord runs to the user. Supply commands,
expected observations and diagnostics in a screencast qualification document as implementation
lands. Record client versions, graphics device/driver, renderer, source type, delivery format,
cursor mode and result. A Discord failure must be isolated against a second portal consumer.

Release requires monitor and native/X11 window sharing, repeatable start/stop, denial/revocation,
session-lock behavior, source/consumer disconnect, bounded slow-consumer behavior and no unrelated
desktop pixels in window streams. Deferred features must remain unadvertised.

## Standards and further design inputs

- [Application-facing ScreenCast portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html)
- [Backend ScreenCast interface](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.impl.portal.ScreenCast.html)
- [Portal system integration](https://flatpak.github.io/xdg-desktop-portal/docs/system-integration.html)
- [PipeWire documentation](https://docs.pipewire.org/)
- Official capture XML already present in the adjacent Wayland build source tree; select and pin
  supported versions through Telorgon's existing protocol-profile machinery.

Rejected directions: patching XWayland for desktop capture; desktop cropping for window capture;
placing session policy in the renderer; treating DMA-BUF import as export qualification; installing
a second compositor through portal activation; or giving each protocol its own capture pipeline.

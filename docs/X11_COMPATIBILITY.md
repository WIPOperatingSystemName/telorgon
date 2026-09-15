# X11 compatibility implementation

Status: **integrated basic display/input; broader compatibility remains incomplete**.
The real patched payload is built and the user reported an interactive xmessage
window. Managed startup publishes X11 endpoints after readiness. Normal X11 windows
now use the same composed frame factory, titlebar/edge hit tests and desktop policy
as native windows. A typed `WindowBackend` dispatches focus/close/configure work;
the separate X11 adapter owns root-coordinate conversion and bounded request
coalescing. Native xdg configure/ack state stays in the native adapter. Alt-drag was
removed. Popups remain unframed and cannot click through to underlying controls.
The shared frame/control integration awaits a live retest; full EWMH state,
iconification, application icon hints and release qualification remain
outstanding. Minimize currently uses the same local visibility policy as native
windows. See [the smoke test](X11_SMOKE_TEST.md).

The managed Vulkan host now advertises linux-dmabuf v4 allocation feedback, including its matched
DRM device and exact importable format/modifier table. Xwayland 24.1.13 requires this device discovery
when `wl_drm` is absent; the previous v3-only advertisement forced its Glamor/DRI3 initialization to
fall back to software. Default and per-surface feedback share the existing Vulkan sampling policy;
the buffer import, materialization, and acquire/release synchronization paths are unchanged.
Wire tests cover feedback transactions and FD lifetimes. Actual Glamor/Firefox acceleration awaits
the hardware check in [the smoke test](X11_SMOKE_TEST.md#gpu-acceleration).

### Host EGL linkage failure found by the latency capture (2026-09-13)

The user-run release capture `20260913-015221-6aw3tras` reached a 60 Hz Vulkan desktop but recorded
566 X11 SHM publications and **zero** X11 DMA-BUF publications. Xwayland stderr explicitly reported
`glamor_egl_get_display() failed` and a Glamor software fallback. Across captured clients, observed
publication-to-first-flip p95 was 80.86 ms (maximum 97.98 ms); the oldest event-to-flush age per input
batch reached 39.15 ms. Long input ages overlapped surface processing, Vulkan scene updates and
command recording. These measurements are evidence from the pre-fix run, not proof that input
dispatch itself, the GPU, or every other app independently causes those delays.

The payload's private build-root `libwayland-client.so.0` was loaded before the host Mesa EGL
vendor. Host `libEGL_mesa.so.0` (Mesa 26.0.8) requires `wl_fixes_interface`, absent from the private
copy. A loader-only regression reproduced that unresolved symbol with the old stage, while the
same host vendor linked successfully after the corrected stage's complete direct Xwayland
dependencies. The runtime policy now leaves the Wayland client with the host graphics stack.
The packer rejects stale stages shadowing host runtime dependencies, and a dedicated
`packaging/xwayland/check_egl_linkage.py` checks selected vendor linkage without a GUI or GPU.
The local real payload was restaged, repacked and embedded again; the old artifact was preserved.

Audit: inspected `packaging/xwayland/{runtime-policy.toml,stage.py,pack.py}`,
`xwayland/process.rs::server_environment`, staged helper/library ELF DT_NEEDED and symbol tables,
host Mesa's symbol table and GLVND vendor JSON. Independent upstream paths inspected were bundled
Xwayland 24.1.13 `hw/xwayland/xwayland-glamor-gbm.c::xwl_glamor_gbm_init_egl`,
`glamor/glamor_egl.h::glamor_egl_get_display`, and upstream
[libglvnd vendor loading](https://github.com/NVIDIA/libglvnd/blob/master/src/EGL/libeglvendor.c)
and its [ICD enumeration contract](https://github.com/NVIDIA/libglvnd/blob/master/src/EGL/icd_enumeration.md).
The [official Wayland client API](https://wayland.freedesktop.org/docs/html/apb.html) documents the
library interfaces; current host vendor requirements cannot be inferred just from the helper's
older link-time symbol floor. The adjacent reference library is absent; this narrow loader-policy
fix changes no GPU synchronization, import ownership or compositor protocol implementation.

Invariants: keep host driver dependencies ABI-compatible in the helper's process; do not substitute
a fixed older Wayland client ahead of the host vendor; reject stale stages before packing; preserve
literal child-local environments and content-addressed extraction. Rejected alternatives: forcing
software rendering, bundling a replacement Mesa driver, adding global `LD_PRELOAD` overrides, or
changing input scheduling without first resolving the confirmed fallback. Tests cover stale-stage
rejection and policy separation, old-failure/new-success vendor linkage, relocated keymap helpers,
and the real embedded release build. The latency harness now surfaces the startup failure and
offers `--require-x11-dmabuf` (no X11 samples means inconclusive). Actual post-fix acceleration and
latency still require the next user-run capture; remaining SHM/native-app costs are not declared fixed.

Decoration ownership now follows bounded `_MOTIF_WM_HINTS` reads on creation and property changes.
A valid decoration flag with no requested decorations removes Telorgon's title bar and controls
while retaining the outer frame styling and normal window management. Missing, deleted, malformed or failed properties select normal chrome;
refreshes retain the last resolved policy until the new reply arrives, and stale replies are ignored.
Motif ALL/exclusion masks are decoded. The shell exposes a complete-frame policy: a nonempty
standard decoration mask selects Telorgon's title bar and controls, rather than separate Motif-style
menu/button combinations. Outer border/radius/colors are independently supplied by the frame template. Motif function restrictions are not implemented by this hint.

`_NET_FRAME_EXTENTS` and `_NET_REQUEST_FRAME_EXTENTS` are advertised. Unmapped windows receive a
configuration-based estimate; associated windows report measured chrome margins in X11 pixels,
with border-only extents for client-decorated windows and zero extents for override-redirect and
fullscreen windows.
Changes and explicit requests cause bounded checked property writes, repeated values coalesce,
and destroyed window generations retire the cache. Normal title-bar ownership changes preserve client root
coordinates; maximized changes preserve outer dimensions. Composed-frame measurements replace
estimated offsets without moving normal X11 client content. No resize delay is introduced.

Implementation audit: read `xwayland/properties.rs`, `xwm.rs`, `manager.rs`, and the desktop
`compatibility.rs`, `x11_windows.rs`, `geometry.rs`, `input.rs`, and `layers.rs` paths. The adjacent
`../other-rendering-libs` sources are unavailable; no independent compositor comparison is claimed.
This CPU protocol/policy change follows [GTK's Motif decoration contract](https://docs.gtk.org/gdk3/method.Window.set_decorations.html)
and [EWMH frame extents and pre-map estimates](https://specifications.freedesktop.org/wm/latest-single/).
Rejected approaches: treating an undecorated window as unmanaged, dropping validation, blocking the
owner loop for properties, or treating GTK's own client-side margins as compositor frame extents.
Tests cover malformed/missing/deleted/stale properties, live notifications, wire extents and request
replies, property coalescing, normal/maximized/fullscreen geometry, custom chrome and integer density.
Live Firefox decoration switching remains manually qualified.

The approved target is a rootless Xwayland 24.1.13 helper with a Telorgon-owned
XWM, modern committed-serial association, and one delivered desktop executable
containing privately extracted helpers/data/libraries. Native application builds
must not carry that payload. The first platform is x86-64 Linux/glibc; glibc 2.39
is the tested helper build floor, not a portability result.

Native host groundwork now includes a protocol-neutral generational `WindowId`.
The identity table uses a native/X11 protocol discriminator and one shared slot
pool. Native entries survive buffer withdrawal/remap and retire on surface
destruction. X11 keys include server generation and XID incarnation, independently
of associated surfaces; the managed X11 adapter is connected. Native resize anchors and terminal configure/ack
transactions now reside in a dedicated `NativeConfigureState` record. Requested
geometry and window policy stay outside that record. `SurfacePresentation` now
owns retained pixels, logical/raster extents, alpha/format metadata, commit revision
and pending image updates. Composition, cursor imagery and frame reporting use
that presentation record. `ClientWindow` still owns it; independent storage/lifetimes
remains open; shared frame action routing is connected. Native configure semantics are unchanged.

Implemented behind `desktop-xwayland`:

- An XWM driver assembles discovery, checked manager acquisition, root enumeration,
  window events and inspection scheduling over one connection. It exposes the FD,
  write interest, next timeout, policy actions and unhandled events to the managed
  desktop-loop owner. Initialization events are retained in a bounded queue and
  consumed before later replies. Tracking combines event work and inspection
  scheduling under a shared turn deadline. Selection loss or transport failure
  closes the driver and revokes its window registry. Enumeration completion is
  not compatibility readiness; output state and executable window policy remain
  required before environment publication. Policy can queue map and geometry
  commands against live generational window identities. Asynchronous reply barriers
  check them without changing confirmed state; only server notifications change
  map/geometry records. Command failures are reported once, and stale failures
  cannot attach to a reused XID. Configure requests expose only masked fields;
  requests received during inspection retain their order within bounded queues.
  A checked synthetic ConfigureNotify command reports caller-supplied root geometry
  adjusted for the client's requested border, without changing confirmed state.
  Policy still decides when to configure or notify; no new EWMH capability is
  advertised by these APIs. Policy can subscribe to per-window property changes
  and read bounded WM_PROTOCOLS metadata. Property revisions invalidate older
  replies, and changes coalesce behind an in-flight read. Cooperative close sends
  WM_DELETE_WINDOW only when current validated metadata supports it; unknown or
  unsupported metadata returns an error without killing a process or X client.
  Bounded WM_HINTS reads provide input and urgency hints. The focus-command API
  selects no-input, passive, locally active or globally active behavior from
  validated hints/protocols, using SetInputFocus and/or WM_TAKE_FOCUS as appropriate.
  It requires a mapped associated window and nonzero interaction timestamp.
  Desktop focus authorization, lock/seat checks and activation policy remain
  caller responsibilities awaiting managed-host integration.

- A bounded X11 window registry consumes server create/map/unmap/configure/reparent/
  destroy notifications and modern `WL_SURFACE_SERIAL` messages. Map requests become
  policy actions rather than confirmed map state. Window identity survives surface
  loss/unmap; destroyed XIDs receive fresh incarnations. Only mapped root children
  with committed associations are presentation/input candidates, including
  override-redirect surfaces. Synthetic lifecycle events cannot mutate server state.
  Unobserved root children request asynchronous inspection. The inspection adapter
  queues attributes, geometry and parent reads with optional deadlines, admitting
  a snapshot only after all three complete. Lifecycle events invalidate inspection
  tokens, preventing stale replies from restoring destroyed or reused XIDs. Root
  enumeration validates and bounds child lists, excludes the manager window and
  feeds inspection in batches of at most 16 windows or one millisecond. At the
  384-request inspection bound it waits for replies, then resumes queued work.
  Unknown-window serials trigger inspection and are retained until metadata is
  available: at most sixteen unique serials per window and the configured window
  capacity in total. Timeouts preserve them for retry; destruction, new creation,
  non-root/input-only results and definitive read failures discard them. Replayed
  serials still require authenticated committed Wayland surfaces. Event-driven
  timeout retry policy, property policy and managed host wiring remain outstanding.

- A first XWM initialization phase that consumes server setup, interns core
  manager/association atoms, queries required extension presence/opcode ranges,
  negotiates Composite 0.4, XFixes 2.0, Shape 1.1 and RandR 1.5,
  and checks root redirection with an asynchronous reply barrier. It preserves
  the connection and sequence tracker for subsequent phases. Discovery completion
  is not compatibility readiness: manager acquisition, root properties,
  output state and window-event handling remain required. Version replies use
  their generated protocol parsers, including Shape's 16-bit version fields;
  missing, insufficient or incompatible versions fail discovery.
- A subsequent manager-acquisition phase creates an unmapped private owner
  window, requests manual Composite redirection for root children, waits for a
  checked redirection barrier, obtains a server PropertyNotify timestamp, refuses existing WM/CM
  selection owners, claims both selections, verifies ownership, and sends checked
  MANAGER announcements. Synthetic timestamp events cannot authorize acquisition.
  Selection loss and the original startup deadline fail this phase. The connection,
  request tracker and advanced XID allocator survive the phase boundary. After
  verified ownership it publishes matching root/owner `_NET_SUPPORTING_WM_CHECK`
  properties and a minimal `_NET_SUPPORTED` list containing only that property,
  `_NET_SUPPORTED` and `_NET_WM_NAME`. All metadata writes share the checked
  announcement barrier. Output state, window handling and owner-loop integration remain open;
  acquisition completion does not publish compatibility readiness.
- Asynchronous request accounting using the locked `x11rb-protocol` sans-I/O
  connection. Every queued request gets a generation/64-bit sequence identity;
  up to 1,024 unresolved requests are retained. Reply barriers confirm void
  requests without synchronous checks. Essential errors/timeouts fail the tracker;
  optional timeouts notify once and discard late contents while retaining their
  slots until resolved. Framing, sequence-span and response-identity checks bound
  internal queues. Only single-reply, non-FD-bearing requests are supported.
- On glibc, pinned embedded-server command preparation from a verified payload
  lease, retained display reservation and matching runtime files. It creates new
  private Wayland/XWM socketpairs and a displayfd pipe, fixes inherited slots 3–7,
  passes the inspected rootless/listener/auth/keymap/font arguments, and sets
  private compiler/scratch/parent-identity variables in a child-only snapshot.
  Preparation does not spawn. The caller must register the dedicated Wayland
  client before handing the command to supervision.
- Display-number reservation using exclusive conventional PID locks and retained
  filesystem plus Linux abstract Unix listeners. Both namespaces are bound before
  accepting a candidate; occupied/stale locks and sockets are never repaired or
  deleted. Reservations expose child descriptor duplicates and accept no clients.
- Private per-instance authority/keymap directories, a fresh 128-bit cookie from
  the kernel random source, and direct FamilyLocal MIT-MAGIC-COOKIE-1 records.
  Authority files are mode 0600 and private directories mode 0700. No xauth tool
  or process-global environment/umask changes are used. Helper commands can retain
  these resources through reaping, with display-number mismatch rejection.
- A generation-aware startup controller with a ten-second deadline, independent
  displayfd/XWM readiness barriers, three retries with 1/2/4-second backoff, and
  environment publication/withdrawal actions. Retries wait for child reaping;
  cancellation and late notifications cannot revive an old generation. These
  actions are not yet connected to the session environment or recovery manager.
- On glibc, an asynchronous private-helper process primitive using `posix_spawn`
  with explicit argv/environment and inherited FD slots 3–7. It relocates source
  descriptors before dup2, closes unrelated descriptors, resets child signal
  dispositions/masks, and supplies null stdin/stdout. A dedicated worker drains
  stderr without retaining contents, retains supplied payload leases, and reaps
  only its owned PID. Stop/drop requests TERM, followed by KILL after two seconds;
  the desktop thread never waits for process exit. A readable notification FD and
  durable status snapshot provide the future event-loop integration seam.
- Bounded nonblocking displayfd parsing that requires the reserved display number
  and newline. This establishes only the server half of readiness.
- Bounded manifest/DEFLATE validation, SHA-256 integrity checks and private
  directory-relative extraction with atomic publication and usage leases.
- A nonblocking private-socket transport using `x11rb-protocol`'s sans-I/O setup
  parser. Packet framing, output queues, ancillary FD rejection and per-turn
  byte/event/time budgets are enforced. The current request set has no FD-bearing
  replies; received descriptors are closed and rejected.
- Association bookkeeping that separates Xwayland generations, XID incarnations,
  stable compositor surface identities and committed serials. Either channel can
  arrive first. A delayed old commit cannot replace a newer association. Destroyed
  surfaces revoke their association; XID reuse cannot inherit stale state. History
  is bounded and exhaustion fails rather than evicting identity tombstones.

`desktop-xwayland-embedded` additionally validates a supplied payload at build
time and provides embedded bytes. See the [packaging instructions](../packaging/xwayland/README.md).
These component APIs are not a claim that the complete compatibility modes or
their managed startup defaults have been implemented.

The pinned server, compiler, libXfont2 and keyboard data have been built in an
isolated Ubuntu 24.04 root. Source patches implement argv-based keymap compilation,
bounded input/time, private scratch paths and helper parent-death handling. The
staged ELF closure contains 21 private libraries with explicit host allowlisting.
The real 4.47 MB archive has passed Cargo embedding and cold/warm extraction tests.
Relocated compiler tests cover multiple layouts and literal special-character
paths. None of these checks starts an operational X11 desktop.

The native Wayland wrapper now provides `Display::create_client` and `OwnedClient`.
It consumes a private socket, tracks libwayland destruction through a stable
listener allocation, and revokes the live identity on peer/display teardown.
Dropping the handle disconnects that client. This supplies registration mechanics;
the managed host must still retain and use the dedicated handle for authorization.
`Display::set_global_filter` now owns a policy for both registry advertisement and
bind-time rejection. Install it before creating globals and consult live client
identity, not UID or a cached pointer. Unwinding policy panics deny access; builds
configured to abort on panic retain their normal abort behavior. Tests prove a
hidden global cannot be bound by guessing its name and that a replacement client
does not inherit the disconnected client's grant.

The protocol generator now pins xwayland-shell v1's two interfaces and four
requests against the installed XML. Loading those descriptors does not register
a global. Surface state now has a permanent Xwayland role and an independent
pending/committed association serial. Zero serials, role conflicts and a second
committed association are rejected. Later buffer commits retain the committed
serial. Opt-in native dispatch now enforces generation-wide strict serial order,
surface role ownership, dedicated-client identity and protocol error codes.
Destroying the role object preserves committed surface association state.

To assemble this native protocol path, first call
`XwaylandAccess::configure_display(&mut display)` before creating globals. Register
the owned private client with its increasing generation via `set_client`, then use
`NativeCompositor::new_with_xwayland(&display, limits, access)`. The access slot
allows a new generation only after its previous client is dead, resetting serial
tracking at that boundary. The registry filter stays installed across replacement.
This setup takes ownership of the display filter policy. Ordinary
`NativeCompositor::new` still creates no Xwayland-shell global. The managed desktop
does not yet select the compatibility constructor or route commits into an XWM.

Still required: real-server launch/authentication qualification, managed use of
the dedicated Wayland client, supervision/readiness/session
integration, remaining XWM initialization/window handling using the bounded
request tracker, and committed-serial association routing,
protocol-neutral desktop windows, ICCCM/EWMH policy,
DRM-syncobj support, multiple outputs, clipboard/PRIMARY/Xdnd bridges, capture
policy and emergency release, managed keyboard replacement and settings publication.
The extraction API does not yet collect unused generations or repair partial
preparations. The source/build input inventory still needs independent
reproducibility and complete provenance/license review, plus dynamic dependency tracing.

The transport's one-millisecond budget is checked between bounded nonblocking I/O
operations. Its real owner-loop scheduling and latency remain unmeasured. It does
not invoke blocking `RustConnection::flush`, synchronous reply waits or Xlib.
The association API requires an already-authenticated caller; its future host
integration must consume the authenticated native dispatch state. Destroying an association protocol
object must not call surface destruction on this bookkeeping table.

The process primitive is intentionally separate from ordinary managed application
supervision: dropping its handle stops the owned helper. It does not restart or
signal applications, publish environments, reserve display sockets, or validate a
custom server contract. Its worker polls child exit every 10 ms and bounds each
diagnostic drain to 256 KiB. Only state changes notify the host; diagnostic byte
counts are content-free snapshots. The glibc closefrom spawn action requires
glibc 2.34 or newer, within the selected 2.39 payload baseline. Actual server
parent-death behavior still depends on the payload patch and remains unqualified.

Resource preparation runs off the desktop thread. The conventional `/tmp/.X11-unix`
directory must already exist with safe ownership/permissions; missing shared
infrastructure fails explicitly. Directory walks reject symlinks. Cleanup holds
inode references and compares current names before unlinking, preserving replaced
entries. It removes the pinned helper's regular, single-link `server-N.xkm` crash
remnant, but leaves unexpected scratch entries for inspection. This is not
isolation from other processes of the same user. The selected server's same-user
local-access policy remains alongside cookie authentication; rejection of other
users without a cookie still requires a real-server test.

See [qualification evidence](X11_QUALIFICATION.md) for executed checks and the
remaining gates. Steam, Wine/Proton, 32-bit application runtimes, drivers, fonts and
IME daemons remain external. Horizon desktop/published-app tests are unavailable,
and no Horizon support claim is made. No Xorg session/backend, external WM or
global toolkit backend override is part of this implementation.

## Startup keyboard configuration

`application_host::LinuxDesktopConfig::keyboard` accepts `KeyboardConfig` with
optional XKB rules, model, layout, variant and options. The managed native seat
compiles and publishes these names at startup. `None` keeps existing defaults;
explicit empty options remain distinct from unspecified options. For example:

```rust
use telorgon::application_host::{KeyboardConfig, LinuxDesktopConfig};
let config = LinuxDesktopConfig {
    keyboard: KeyboardConfig {
        layout: Some("us,de".into()),
        options: Some("grp:alt_shift_toggle".into()),
        ..Default::default()
    },
    ..Default::default()
};
```

Interior NULs fail declaration validation; other invalid XKB names fail keymap
compilation. With `include_root: None`, compilation uses the current host XKB search path.
Setting `include_root` to an absolute directory makes it the exclusive data root,
with no fallback to default/home/environment include paths. This can point to a
leased payload’s `share/X11/xkb` directory; the caller must keep that payload valid
while compiling. Automatic payload selection, live keymap replacement, managed
Xwayland startup and IME services remain open. Downstream exhaustive config
literals must supply the new field or use `..Default::default()`.

## Emergency pointer release

The managed desktop reserves **Ctrl+Alt+Shift+Escape** ahead of configurable
shortcut handlers, including while locked. Physical Escape is used so changing
its keysym cannot remove the escape path. Its press, repeats and release are
consumed; keyboard focus and session-lock state are unchanged.

The chord deactivates pointer constraints and blocks new or persistent constraints
on that surface until pointer focus leaves. A later enter may reactivate persistent
constraints; one-shot constraints remain finished. `DesktopKeyAction::ReleaseCapture`
also exposes this action to compositor shortcut policy. Downstream exhaustive
matches on that enum must handle the new variant.

The chord also revokes native shortcut inhibition and accepted Xwayland keyboard
grabs. Real Xwayland application/capture qualification remains open.

Native keyboard delivery now rejects duplicate presses and unmatched releases
before issuing input serials or sending protocol events. This prevents invalid
edges from becoming activation evidence. The managed host now suspends/drains libinput before acknowledging seat loss,
clears protocol input state, and resumes input on seat enable. Physical VT/device
reopening and KMS recovery still need hardware qualification.

## Seat input suspension

Managed seat disable is explicitly acknowledged after libinput closes its devices
and discards queued events. The host cancels drag/touch delivery, clears pressed
keys/buttons and pointer grabs, sends keyboard/pointer leave, releases held XKB
keys, clears shortcut ownership and ends interactive resize. Resume reopens
libinput devices and restores a still-present, non-minimized keyboard target only
when unlocked and no newer focus has been chosen. Pointer focus is reacquired by
normal pointer policy. Existing direct users of `LinuxSeat::open()` keep automatic
disable acknowledgement; the managed host uses the explicit deferred API.

This is input lifecycle integration with unit/protocol coverage. It does not
establish working KMS device reconstruction or real hardware suspend/resume
qualification.

Lock, unlock and cancelled-lock transitions explicitly revoke keyboard focus and
clear delivered pressed-key state. Held physical keys stay suppressed through
repeat and release, so a later keyboard enter cannot inherit keys from across the
lock boundary. Normal application-to-application focus semantics are unchanged.

## Native shortcut inhibition

`zwp_keyboard_shortcuts_inhibit_manager_v1` v1 is implemented and used by the
managed host. A mapped surface with keyboard focus may inhibit configurable
shortcuts for its seat. Focus loss or unmap restores normal routing silently,
as the protocol requires; focus return can activate an eligible inhibitor again.
Lock surfaces are ineligible, and lock/seat cleanup removes keyboard focus.

Ctrl+Alt+Shift+Escape remains reserved. User release sends `inactive` and revokes
the grant for that surface's lifetime, including replacement inhibitor objects.
There is currently no user-facing re-enable command. Duplicate seat/surface
inhibitors raise the protocol's error. Surface/object destruction and client
teardown remove their state. Hardware application qualification remains open.

## Dedicated Xwayland keyboard grabs

The Xwayland-enabled compositor constructor exposes
`zwp_xwayland_keyboard_grab_manager_v1` v1 only to its dedicated client. The global
filter hides it from other clients and bind/request checks enforce the same actual
client identity. A request is honored only for an already-focused, mapped Xwayland
role with a committed modern surface serial. It bypasses configurable shortcuts
through the same host routing as native inhibition, retaining the emergency chord.
It never changes focus to satisfy a request.

Focus loss/unmap cancels the grant permanently for that object; a new eligible
request is required. Lock and seat cleanup therefore cancel it through focus loss.
Emergency release blocks replacement requests for the surface lifetime. Object,
surface and client destruction remove grant state. These are tested protocol
components; the managed host still does not launch Xwayland or route X11 windows,
and real server/application capture qualification remains outstanding.

## Logical output descriptions

`zxdg_output_manager_v1` v3 publishes each registered output's logical position,
transformed/scaled size, and version-appropriate name/description. Output names
must be unique ASCII alphanumeric/dash strings. v1/v2 descriptions finish with
xdg-output done; v3 uses the associated wl_output done when that object supports it.
For a legacy wl_output v1 binding, the deprecated xdg-output done event remains
the wire-compatible completion fallback.

`NativeCompositor::update_output` publishes changed position, scale, transform,
current mode and description to existing core/xdg-output objects. Names stay
immutable, v2 xdg descriptions stay immutable, and core done events follow logical
metadata. Identical snapshots emit nothing. Renaming, changing the available mode
list or toggling enabled state requires future global reconstruction and is
rejected without replacing the stored snapshot.

Changes to the first enabled output's scale refresh existing surface scale
preferences under the current single-output policy. The managed KMS loop does not
yet drive this API from hotplug/layout changes; atomic whole-layout revisions,
per-surface multi-output scale selection and output reconstruction remain open.

## Transactional output descriptions

`update_outputs` validates an entire batch before changing state or publishing
metadata. Changed batches advance one output-layout revision; empty or unchanged
batches do not. Duplicate IDs, unknown outputs, invalid replacements and revision
exhaustion leave the previous state intact. `update_output` uses the same path.
`output_snapshot` returns detached output state with that revision, so retained
snapshots remain stable across later updates. Output registration also advances
the revision and rolls back its state if global creation fails.

Revision tracking requires use of these APIs rather than direct writes through
`core_mut().outputs`. Rendering, input and XWM consumers are not yet driven by the
snapshot; the managed host still has its existing single-output configuration.
This does not implement physical multi-output scheduling or reconstruction.

### Checked desktop/root geometry

`OutputLayoutSnapshot::root_geometry` derives a revision-bearing root mapping from
all enabled output rectangles, including transformed fractional extents and negative
positions. Empty layouts return no root; invalid modes, overflowing extents and
unrepresentable unions return errors. Conversion preserves offscreen positions.
`Geometry::from_desktop` and `desktop_rect` convert root-child geometry once at the
X11 boundary and reject core coordinate/dimension overflow. They do not apply to
child-relative geometry. The owner must check the retained revision before sending
a configure; these helpers do not establish Xwayland RandR monitor ordering or
wire the XWM into the managed host. The native host now derives its initial logical
extent from the same selected output state it publishes. Physical multi-output
scheduling and live shared-snapshot consumption remain outstanding.

### Normal size hints

The XWM exposes asynchronous `refresh_normal_hints`/`normal_hints` after its property
subscription is established. Reads request at most 18 32-bit words and share the
existing request capacity, metadata bounds and per-turn deadline. Changes revoke
cached constraints immediately; revision and window-lifetime checks reject stale
replies. Deletion produces default hints; malformed metadata remains unavailable.
The reader supports 15-word legacy and 18-word ICCCM records, flagged min/max sizes,
increments, aspect ranges, explicit base size and gravity. It preserves the distinct
base-size fallback rules for increments and aspect ratios. Resize policy enforcement,
gravity placement and actual client qualification are still outstanding.

Wire layout and fallback semantics were checked against the
[ICCCM normal-hints specification](https://xorg.freedesktop.org/archive/X11R7.7/doc/xorg-docs/icccm/icccm.html).

### Focus while seat access is suspended

`suspend_seat_input` now keeps focus delivery suspended until `resume_seat_input`.
Keyboard/pointer focus requests during that interval replace bounded pending state
instead of sending enter events or reactivating capture. Resume discards destroyed
targets and uses separate fresh serials for keyboard and pointer enters. Its return
value identifies an explicit pending keyboard decision, including clearing focus,
so the managed host does not override it with the pre-suspend focus. Repeated
suspension preserves pending policy and the host's original fallback focus.
Hardware input remains owned by the host's libinput suspend/resume gate. Actual
seat/VT and lock-during-suspension qualification remains outstanding.

### Selection transfer socket pump

`xwayland::transfer::Transfer` forwards compositor-owned Unix socket endpoints
without blocking. It buffers at most 256 KiB, limits each turn to 64 KiB of I/O or
1 ms, exposes readiness interests and a ten-second inactivity deadline, and accepts
a configurable total-byte limit (64 MiB default constant). EOF completes only after
buffered output drains. Cancellation, timeout, oversize input and I/O errors close
the owned endpoints and release buffering. Oversize/failure may occur after a prefix
was delivered; callers must report failure rather than treating EOF as success.
Socket peers can be supplied as Wayland transfer FDs. `Transfers` enforces a
16-entry limit, allocates non-reused registry IDs, cancels by ownership generation
and dispatches ready/expired entries round-robin under one shared millisecond budget.
The owner must register readiness/timer sources, preserve returned reschedule IDs,
unregister sources before teardown and remove observed terminal entries.
`xwayland::incr` supplies bounded send/receive state machines for property deletion
handshakes, including final empty-property acknowledgement. Senders support 8-, 16-
and 32-bit formats and reject chunks that split wire items. Receivers expose the
validated property atom/format while retaining raw connection-order bytes. Target
encoding and conversion remain the caller's responsibility.
`xwayland::incr_wire` queues generation-scoped ChangeProperty/DeleteProperty
requests through the asynchronous request tracker, checking padded writes against
the server's setup request limit. State advances only after successful queueing;
backpressure leaves pending chunks available. Returned request IDs still need
owner routing, reply barriers and cancellation on asynchronous errors/timeouts.
The announcement helper writes the INCR atom with one 32-bit zero lower bound and
rejects repeated announcements; the owner must still send SelectionNotify afterward.
The notification helper serializes success/refusal responses with an empty event
mask, preserving request fields and accepting an explicit property for legacy
requests that name none. Calling it in order, validating ownership/timestamps and
handling its asynchronous result remain selection-owner responsibilities.
`property_read::PropertyRead` assembles non-deleting GetProperty replies in 16-KiB
parts with a 256-KiB total bound, one outstanding request, generation/sequence
matching, and consistent type/size checks. It returns a complete property for the
INCR receiver; the owner still routes replies and enforces cancellation/deadlines.
Its completion adapter distinguishes unrelated requests, another required fragment,
and complete data. Matching errors/timeouts or malformed wire replies cancel the
read and release its buffer; callers must contain those errors to the transfer.
The INCR receiver accepts owned property replies, moving the assembled allocation
into its pending chunk without a second payload copy. The borrowed API remains
available, but the selection bridge should use ownership transfer for reader output.
`selection::Ownerships` retains independent clipboard/PRIMARY ownership records,
with server-generation-scoped revisions for each accepted content change, including
same-owner replacement. It returns revoked snapshots for transfer/offer cancellation.
Proxy echo filtering, timestamps, wire ownership and cancellation routing remain
bridge work; this ledger neither advertises PRIMARY nor publishes a selection.
The transfer registry accepts live ownership tokens, enumerates their transfer IDs,
and cancels by the complete token. These scopes are distinct from its legacy opaque
generation scopes. Owners must unregister readiness sources before cancellation;
server teardown must revoke every ownership, not just a legacy generation scope.
Server-scoped enumeration/cancellation includes superseded selection ownerships
still registered for teardown, even after their ledger entries have been cleared.
Conditional ledger revocation requires the exact current ownership token, so
delayed events cannot clear replacement owners. Unconditional clear remains for
explicit compositor policy such as lock or teardown.
`acquisition::Acquisition` queues SetSelectionOwner with an explicit timestamp and
checks GetSelectionOwner asynchronously. Acquired requires both the checked set
request and a matching owner reply. A mismatch rejects the attempt; errors/timeouts
fail it. Publication, SelectionClear routing and acquisition scheduling remain host work.
`acquisition::Publication` connects confirmation to native-source ledger publication
and token-checked revocation. The host must synchronize after completions/errors,
process returned snapshots for transfer cleanup, and map selection atoms correctly.
Acquisition handles matching server SelectionClear events (including pending claims);
synthetic/stale events are ignored and malformed clear events invalidate authority.
`Xwm::watch_selection` now queues an explicitly configured selection subscription
using discovered XFixes metadata and the owned manager window. Its dispatch loop
accounts for subscription/barrier completions and deadlines; readiness requires all
checks to finish. Duplicate selection kinds or atoms and invalid inputs are rejected
before queueing without disrupting existing watches. Failure after partial setup or
an asynchronous subscription failure closes that XWM driver. Returned context
routes raw Turn events to the host ledger; managed desktop setup remains unwired.
X11 request/event wiring, selection generations, MIME conversion and actual
native/X11 endpoint integration remain outstanding; this is not a working bridge.

Startup discovery now interns CLIPBOARD, TARGETS, TIMESTAMP, INCR, TEXT and
COMPOUND_TEXT on the private XWM connection. `watch_standard_selection` resolves
CLIPBOARD from that discovery and PRIMARY from its predefined X11 atom; both use
the same asynchronous subscription barrier. Interning transfer atoms does not
advertise implemented conversions or selection targets. Transfer scheduling and
native endpoint integration remain outstanding.

`Xwm::selection_atoms()` exposes the discovered selection/transfer IDs with their
connection generation while tracking. Its selection decoder rejects stale
generations and distinguishes PRIMARY/CLIPBOARD from unrelated target atoms.
The accessor returns no context before tracking or after teardown; retained
contexts must still be checked against the current generation.

`conversion::Conversion` queues asynchronous ConvertSelection and matches
SelectionNotify by connection generation, requestor, selection, target, timestamp
and property. Synthetic notifications are accepted as required by SendEvent.
A checked request does not end the independent response deadline. Refusal,
timeout and cancellation terminate the attempt; success exposes a PropertyTarget
for bounded property reading and subsequent direct/INCR classification. The host
must allocate distinct live endpoints, cancel on ownership loss, and schedule
reads/transfers; this component is not yet attached to native clipboard endpoints.

## Managed launch and association integration

Embedded builds now start preparation from the managed desktop unless
`LinuxDesktopConfig::xwayland_enabled` is false. `xwayland_cache` overrides the
private cache under XDG_CACHE_HOME (or HOME/.cache). Preparation/extraction runs
on a worker; the owner inserts the private Wayland connection and registers its
identity before spawning. XWM/helper/readiness FDs wake libwayland, writable
interest follows queued requests, and startup has a ten-second deadline.
Map/configure requests and committed surface serials now reach the XWM. Actual
surface destruction revokes associations; ordinary buffer commits do not repeat
serial admission. Failure/drop unregisters callbacks, disconnects the private
client and requests supervised helper shutdown without blocking native dispatch.

This integration currently performs one startup attempt. Bounded lifecycle restart,
X11 composition/input routing, session launch gating and DISPLAY/XAUTHORITY
publication remain to be connected. No live X11 desktop claim follows from it.

Managed presentation now admits Xwayland images through the existing SHM/DMA-BUF
publication path, initially hidden. Before input processing and scene assembly,
the host applies mapped/associated XWM geometry and stable desktop identities;
unassociated images and their subsurfaces remain hidden. No GPU ownership or
release mechanism changed. Keyboard focus routing and readiness/environment
publication remain unfinished; accelerated capability gaps are unchanged.

Managed pointer-button focus now schedules a private-manager property change to
obtain X-server time, waits for WM_HINTS/WM_PROTOCOLS, and applies the existing
ICCCM focus adapter. Wayland keyboard focus is granted only after command barriers
resolve and the mapped association is rechecked. X11 stacking and the compositor
window family are raised together. Native focus changes, hidden windows and
lock/seat transitions prevent pending grants; timestamp replies are sequence- and
manager-scoped and synthetic notifications are rejected. Initial metadata reads
are attached to live XWM window creation. This covers the ordinary pointer-click
path; task switching/touch policy, session environment publication and full X11
shutdown integration remain outstanding. No live input qualification is claimed.

Managed shutdown now counts ordinary X11 windows independently of native
wl_surface toplevels. It requests server timestamps for up to four outstanding
cooperative closes, then sends WM_DELETE_WINDOW only where validated protocols
support it. Closed/reused window identities are pruned; cancellation drops pending
close intents so later timestamp replies cannot initiate closes. Refusal or missing
WM_DELETE_WINDOW keeps shutdown open until the existing cancellation timeout;
there is no fallback KillClient or application PID signalling. The helper is
stopped only when desktop teardown actually proceeds. Runtime save-dialog and
mixed native/X11 shutdown acceptance remain unrun.

Managed child environment publication is now connected: after displayfd and XWM
initialization, future managed launches inherit the private DISPLAY/XAUTHORITY
while preserving WAYLAND_DISPLAY. Withdrawal restores the session's native base
snapshot without process-global mutation. Existing managed children that inherited
that display have automatic failure retries suppressed; recoverable failures remain
available for explicit recovery review. Explicit launches targeting another DISPLAY
are excluded from this incident scope. Initial launches do not yet wait for
compatibility readiness, and opt-in shared D-Bus/systemd environment publication
still uses the native startup snapshot; both remain integration work.

Initial direct managed launches are now queued while embedded compatibility starts.
Success releases them with the private endpoint; terminal startup failure releases
them with the native snapshot. The process worker drains at most four launches per
turn. Quiescing holds queued work, closing cancels it without spawning, and recovery
journals retain recoverable queued commands without claiming an existing PID.
`ManagedChild::id()` is zero until a deferred process is created. Shared activation
service updates and explicit X11-required launch errors remain outstanding.

The outer-frame policy uses `window_has_frame`, separately from title-bar ownership. Managed X11
windows retain frame composition and clipping when Motif requests client decorations. Fullscreen
and unmanaged popups are excluded; native Wayland decoration policy is unchanged. The frame model's
`title_bar_visible` flag tells templates whether to omit compositor title/drag/control elements.
`EasyWindowFrame` honors it while retaining the selected active/inactive palette, border, state radius,
shadow and resize regions. The content slot gets all remaining inner space, including the application's
own header. Scene clipping, size limits, hit testing and frame extents use the outer-frame policy;
only title-bar controls depend on server-decoration ownership. This uses existing frame geometry and
rounded clipping rather than adding a renderer path or reading/replacing application UI pixels.
The prior source/specification audit applies; adjacent reference sources remain unavailable.

Application-owned headers can initiate pointer moves and resizes using `_NET_WM_MOVERESIZE`, now
advertised in `_NET_SUPPORTED`. The XWM queues at most 256 requests per owner drain, accepts only
mapped managed windows, and resolves their live incarnation/association again at consumption.
The desktop requires a matching implicit pointer grab and held physical button, uses its own pointer
position, and delegates to the existing shared interaction implementation. Cancellation is processed
in order and ends only the matching window's interaction. Late requests after button release,
unknown/unmanaged windows, malformed formats and unsupported directions are ignored. Keyboard
move/resize directions are not implemented. The existing release and session-lock paths end grabs.

Audit: inspected XWM client-message dispatch, `SeatState` press ownership, native move/resize actions,
and the shared desktop interaction loop; checked the official EWMH `_NET_WM_MOVERESIZE` contract.
The adjacent reference library remains unavailable. Treating arbitrary clicks in application content
as title-bar drags was rejected: the application chooses its own draggable regions. Tests cover wire
request parsing and rejection plus held-button/surface ownership checks. Live Firefox dragging awaits
manual confirmation.

Mapped X11 maximize requests now use `_NET_WM_STATE` add/remove/toggle messages and
Telorgon's existing work-area maximize/restore policy. Either maximize atom selects
that single, both-axis policy; a paired request is processed once. Separate horizontal
or vertical maximization and pre-map initial state hints remain unsupported. The XWM
advertises the state/maximize atoms and publishes the actual shell state as ATOM/32,
including changes from compositor controls. Writes coalesce, use checked requests,
and retry after backpressure or command failure. Request queues are bounded and resolve
live mapped, managed window incarnations to committed surfaces before desktop dispatch.
Requests during a locked session or active move/resize are ignored.

Maximize audit: inspected `xwayland/{discovery,manager,xwm,window}.rs` and desktop
`{compatibility,x11_windows,interaction}.rs`; the adjacent reference tree is still
absent. This CPU protocol bridge follows the official
[EWMH state request/property contract](https://specifications.freedesktop.org/wm/latest-single/).
The shell remains authoritative; no client-reported geometry or pointer grab is required
for maximize. Rejected a second geometry implementation and per-atom double toggles.
Socket tests cover request validation, paired atom ordering, association retirement,
state publication and redundant-write coalescing. Existing desktop tests cover X11
maximize/restore geometry and resize previews. Live Firefox button behavior remains user-run.

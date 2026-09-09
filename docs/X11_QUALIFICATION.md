# X11 qualification evidence

This is an implementation ledger, not a release qualification report. Starting
revision: `efec5fb0cd92495c4cbe4ddc01d49b48edc61cc9`. No GUI/server or hardware
session was launched during these checks. No workstation package was installed
and no desktop session was altered. Build packages were installed only inside a
private Ubuntu root under `/tmp`, with service startup disabled.

## Executed native baseline

```sh
cargo check -p telorgon --all-targets --no-default-features \
  --features desktop-wayland-linux --frozen \
  --target-dir /tmp/telorgon-x11-plan-check
cargo test -p telorgon --lib --no-default-features \
  --features desktop-wayland-linux --offline \
  --target-dir /tmp/telorgon-x11-plan-check compositor_wayland:: -- --test-threads=1
```

The planning-revision check passed with three pre-existing dead-code warnings.
The implementation baseline ran 37 compositor state tests: all passed. Coverage
includes surface/xdg commits, ownership, focus/key/button edges, subsurfaces,
selection cancellation, and presentation/buffer lifetime accounting. It does not
establish rendered pixels, native desktop interaction or shutdown on hardware.

The DMA-BUF hardware integration test also requires `application-software` and a
Vulkan-capable feature combination. An ordinary `desktop-wayland-linux`
`--all-targets` check does not exercise that test's implementation. Hardware test
compilation and selected-device execution must be reported separately.

The hardware test was subsequently **compiled, not executed**, with its actual
feature gates enabled:

```sh
cargo test -p telorgon --no-default-features \
  --features desktop-wayland-linux,application-software \
  --test renderer_vulkan__linux_dma_buf_hardware --no-run --offline \
  --target-dir /tmp/telorgon-x11-plan-check
```

Compilation passed. The native `--all-targets` check also passed again after the
changes. Neither result is GPU/KMS runtime evidence.

## Component checks

```sh
cargo test -p telorgon --lib --no-default-features \
  --features desktop-xwayland --offline \
  --target-dir /tmp/telorgon-x11-plan-check xwayland:: -- --test-threads=1
python3 -m unittest discover -s packaging/xwayland -p 'test_*.py'
```

The full library suite with `desktop-xwayland` passed: **1,001 passed, one
ignored**. This includes 19 new component tests and existing native/session
regressions. Python archive tests: **five passed**.

With the real built archive supplied through `TELORGON_XWAYLAND_PAYLOAD`,
`desktop-xwayland-embedded` component tests passed: **20 passed**, including
cold/warm extraction of the actual payload embedded in the Rust test executable.
An embedded build with no supplied payload was also checked: it failed at build
time with the documented missing-input error.

One attempted embedded test run was blocked by the tool sandbox's unmapped root
ownership for `/tmp` and denied Unix socket writes. The exact tests passed outside
that sandbox; ownership checks were not weakened. No server was started by them.

Rust component checks cover synthetic archive corruption/truncation/bounds, unsafe
paths, cached symlinks/hardlinks/unexpected files, concurrent publication and usage
leases, association order/reuse/destruction, partial socket reads, bounded stalled
writes, oversized replies, event-budget yielding and content-free setup failures.
Synthetic fixtures contain artificial ELF headers and are never executed. The
separate real-payload embedding test writes and verifies built binaries but does
not execute them. Actual compiler execution is covered separately below.

## Private process and lifecycle follow-up

The subsequent implementation chunk ran the `desktop-xwayland` component command
above again: **32 passed**, including 13 additional lifecycle, process and
readiness tests. Six process tests execute short-lived `/usr/bin/python3` unit
fixtures, not Xwayland or a desktop application. Coverage includes literal argv,
explicit child-only environment, null standard input, all FD destinations 3–7,
closure of an intentionally non-CLOEXEC unrelated FD, large diagnostic draining,
exec failure, ignored TERM followed by KILL, and reaping after handle drop.

Lifecycle tests cover either readiness order, stale generations, cancellation
during spawn, exact startup-deadline/reap ordering, withdrawal on crash, and the
three bounded backoffs. Displayfd tests cover partial notification, mismatched
display numbers, extra bytes and premature EOF. These are component results, not
real-server startup, authentication, FD inventory, or native-survival acceptance.

The native `desktop-wayland-linux --all-targets --frozen` check also passed again
with the same three existing warnings. Formatting and `git diff --check` passed.

## Display reservation and authority follow-up

The next component run passed **40 tests**, including eight resource tests added
in this chunk. Tests use randomized private temporary filesystem/abstract socket
namespaces and do not bind conventional session display sockets. Coverage includes
concurrent reservations, existing stale/symlink locks, filesystem and abstract
conflicts, rollback after conflict, replacement-lock preservation, descriptor
duplication, authority record fields/fresh cookies/modes, unsafe runtime roots,
regular versus symlink keymap cleanup, and command resource retention/display
mismatch rejection. Record encoding was cross-checked against the locked
`x11rb-protocol 0.13.2` `src/xauth.rs` FamilyLocal and big-endian parser.
The native all-targets frozen check passed again with the same three existing
warnings; scoped formatting and whitespace checks passed.

No live server accepted or rejected authentication in these tests. No Xwayland,
GUI application, active desktop, conventional display socket, or shared activation
environment was changed. The earlier `/tmp` payload build artifact is not present
in the current environment; its build results below remain historical evidence,
not a newly built or currently available distribution artifact.

## Pinned command preparation follow-up

The subsequent component run passed **42 tests**. Added coverage checks the exact
pinned argv/FD-number contract, replacement of inherited private-helper settings,
removal of stale display/one-shot grants, preservation of unrelated environment
values, literal special-character paths, and endpoint/resource cleanup when a
prepared generation is discarded. The synthetic payload is never executed.
Actual process execution remains covered by the separate short-lived subprocess
fixtures. Dedicated Wayland-client registration and real server startup remain
outstanding; preparation alone cannot publish compatibility readiness.

## Owned Wayland client follow-up

Native server component tests passed (**8 tests**) with private-client creation,
peer disconnect, display teardown and handle-drop isolation coverage. The registry
sync test now uses the owned registration wrapper. The FD transfer and destruction
listener contracts were checked against the [official server API](https://wayland.freedesktop.org/docs/html/apc.html).
This verifies the wrapper mechanics, not Xwayland privileged-global filtering or
managed restart integration. No GUI or X server was launched.
The full `desktop-xwayland` library regression suite subsequently passed:
**1,026 passed, one ignored**, including the updated registry-sync test and native
compositor/session state tests. Scoped formatting and whitespace checks passed.

## Registry filter follow-up

The native server component suite passed **10 tests** after adding an owned
display-global filter. Private socket fixtures verify authorized advertisement,
unauthorized invisibility, bind rejection despite a guessed global name, and
denial after the trusted client disconnects and a replacement connects. A separate
test exercises panic containment under the test profile's unwind strategy. The
expected unauthorized-bind protocol error is emitted by libwayland during this
test. These checks do not implement or qualify Xwayland-shell dispatch itself.

## Xwayland-shell descriptors and committed surface state

The xwayland-shell descriptor/surface-state follow-up passed the compatibility
all-targets check with the same three existing warnings. Its source is the
installed `staging/xwayland-shell/xwayland-shell-v1.xml`; the generated build
validated the four pinned request signatures against that XML. Surface tests
cover commit-only visibility, retention over later commits, second-association
rejection, native-role conflicts and failure before commit publication. No global
is registered by these changes; wire dispatch and per-generation validation remain
outstanding. The native compositor state suite passed **40 tests**, including
three new Xwayland surface tests; formatting and whitespace checks passed.

## Authenticated Xwayland-shell dispatch

The subsequent authenticated-dispatch slice adds an opt-in Xwayland-shell global
and a persistent dedicated-client access slot. Two private wire tests pass:
surface serials latch on commit and survive role-object destruction; role reuse
is rejected; serial order spans surfaces; stale/live generation replacement is
rejected; a dead generation can be replaced without rebuilding the compositor.
The test also checks that an ordinary same-user client sees native globals but
cannot discover or bind the Xwayland shell. Expected protocol-error messages are
part of the rejection tests. No X server or interactive session was launched.
The full `desktop-xwayland` library suite then passed **1,033 tests, one ignored**.
The compatibility all-targets check also passed with the same three existing
warnings; scoped formatting and whitespace checks passed.

## Bounded XWM request accounting

The XWM request-accounting follow-up uses the inspected
`x11rb-protocol 0.13.2/src/connection/mod.rs` helpers. Component tests cover void
request barriers, asynchronous errors, essential and optional timeouts, retained
capacity under a stalled peer, late-response discard, 65,537 successive request
identities crossing the wire sequence wrap, unexpected response order, and failed
queue attempts. KeymapNotify's SendEvent variant is normalized for the helper's
sequence extractor and restored for delivery, with a dedicated regression test.
The complete Xwayland component run passed **48 tests**; scoped formatting and
whitespace checks passed. XWM initialization and owner-loop budget measurements
remain outstanding.

## Initial XWM discovery

The first XWM discovery phase passed **51 Xwayland component tests**, including
three new controlled-peer tests. They send a fragmented setup, verify generated
atom/extension/root-redirection request shapes, withhold the final barrier to
prove discovery stays incomplete, and test a missing extension, root BadAccess,
and startup timeout. An initial test failure exposed omitted trailing padding in
the fixture's generated reply serialization; the fixture now supplies the required
32-byte wire replies and asserts the specific intended failure messages.
No real X server was used. Compilation, scoped formatting and whitespace checks
passed. Selection ownership, extension-version negotiation, root metadata, output
state, and owner-loop latency qualification remain outstanding.

## XWM extension-version negotiation

The version-negotiation follow-up passed **52 Xwayland component tests**. The
controlled peer checks the discovered extension opcodes and requested versions,
returns the four protocol-specific version replies, and proves the earlier root
barrier alone cannot complete discovery. A RandR 1.4 response is rejected before
the RandR 1.5 monitor path can be used. Compilation, scoped formatting and
whitespace checks passed. These are protocol-fixture results, not live-server or
hardware qualification; manager ownership, root metadata and output initialization
remain outstanding.

## XWM manager-selection acquisition

The manager-acquisition follow-up passed **56 Xwayland component tests**.
The new socket fixtures exercise existing-owner rejection, waiting for a real
server timestamp (ignoring synthetic PropertyNotify), owner verification,
checked MANAGER announcements, selection loss after acquisition, and expiration
at the original startup deadline. Generated events are padded to the full
32-byte wire size. No X server or interactive application was launched.

The all-target `desktop-xwayland` compile check, scoped formatting and
`git diff --check` also passed. The phase preserves its connection, sequence
tracker and allocated XID range for subsequent initialization. This is component
coverage for W04, not completed XWM readiness: root metadata, output state,
window handling and desktop-loop integration remain outstanding. Hardware and
application qualification, including Horizon, remain unrun.

## Checked EWMH manager metadata

The metadata follow-up passed **57 Xwayland component tests**. The controlled
peer checks both supporting-WM pointers, WINDOW/32 versus ATOM/32 wire types,
replacement mode and the exact three-entry supported-property list. The existing
barrier test now covers metadata writes as well as manager announcements; an
injected BadWindow on a metadata write prevents phase completion. No unsupported
window-state or workspace handler is advertised.

The all-target `desktop-xwayland` compile check, scoped formatting and
`git diff --check` passed. Publication follows the self-referencing owner-window
contract in [EWMH sections 3.1 and 3.10](https://specifications.freedesktop.org/wm/latest-single/).
This remains component evidence: real Xwayland startup, output initialization,
window routing and the managed owner loop are not qualified by these tests.

## Checked Composite redirection

Manager initialization now requests manual Composite redirection for all current
and future root children using the negotiated extension opcode. It waits for a
reply barrier before claiming manager selections, even when both selections are
free and a server timestamp has arrived. A Composite BadAccess fails the instance
before selection acquisition. This is separate from the core SubstructureRedirect
window-management event mask already checked during discovery.

The follow-up passed **59 Xwayland component tests**, the all-target
`desktop-xwayland` compile check, scoped formatting and `git diff --check`.
The socket fixtures verify the target root, extension/minor opcode, manual mode,
withheld-barrier behavior and conflict handling. The intended wire behavior follows
[X.Org's Composite contract](https://xorg.freedesktop.org/archive/X11R7.5/doc/man/man3/Xcomposite.3.html).
This does not establish correct pixels or real Xwayland surface creation; those
remain runtime acceptance gates alongside window association and host integration.

## X11 window-event and association routing

The follow-up passed **64 Xwayland component tests**, the all-target
`desktop-xwayland` compile check, scoped formatting and `git diff --check`.
The window-registry fixtures cover native-order and reversed-order X11/Wayland
serial arrival (including high serial bits), map request versus confirmed mapping,
override-redirect windows, unmap/reparent/surface-loss presentation revocation,
destroy/XID reuse, stale server generations, synthetic lifecycle rejection,
malformed serial messages and bounded window creation. Window metadata survives
surface loss. No desktop policy action is executed by this registry.

Initial QueryTree/attribute inspection, bounded window properties, configuration
requests and native-host event routing remain outstanding. These tests are W04
component evidence and do not close W02 or establish real desktop compatibility.

## Asynchronous window inspection

The inspection adapter queues GetWindowAttributes, GetGeometry and QueryTree
without waiting on the owner thread. Optional errors/timeouts cancel only the
snapshot; late responses remain accounted by the request tracker. Windows admits
completed snapshots through generation/lifetime tokens invalidated by subsequent
server lifecycle observations. Input-only windows are excluded from presentation
records. The adapter bounds outstanding inspection requests to 384.

The follow-up passed **67 Xwayland component tests**, the all-target
`desktop-xwayland` compile check, scoped formatting and `git diff --check`.
Socket fixtures verify exact request targets, no premature snapshot after one or
two replies, complete mapped/override-redirect geometry, destruction plus XID reuse
before replies, and timeout/late-response cleanup. Initial root inventory and
retry scheduling after invalidation still need owner-loop integration. Unknown
window surface-serial messages during inspection also need bounded retention
before enabling this path in the managed desktop.

## Root enumeration and bounded inspection scheduling

The initial QueryTree root read is essential and checked for the expected root,
root parent, duplicate/invalid child IDs and configured window bound. The private
manager window is excluded. Its child list feeds asynchronous inspection in
batches limited to sixteen windows or one millisecond, with at most 384 unresolved
inspection requests. A saturated adapter reports no immediate reschedule demand;
replies free slots for remaining children. Enumeration completion waits for both
queued and outstanding inspections and is not desktop readiness.

The follow-up passed **70 Xwayland component tests**, the all-target
`desktop-xwayland` compile check, scoped formatting and `git diff --check`.
Socket tests cover manager exclusion, completion after a full child snapshot,
invalid/excessive trees, a 129-child queue reaching the 128-inspection limit and
resumption after a disappearing child's error replies. Managed-loop scheduling,
retry after event invalidation and retaining surface serials during inspection
remain integration work; no real Xwayland session was run.

## Surface serial retention during window inspection

Unknown-window WL_SURFACE_SERIAL messages now request inspection and retain a
bounded ordered set of serials until metadata is available. Duplicate messages
consume no additional storage. Retention is limited to sixteen serials per window
and the configured window capacity in total, with explicit overflow failure.
Successful inspection emits creation followed by an association change where
available; association still requires an authenticated committed Wayland surface.

The follow-up passed **74 Xwayland component tests**, the all-target
`desktop-xwayland` compile check, scoped formatting and `git diff --check`.
Tests cover both commit/inspection orders, high serial bits, duplicate retention,
timeout retry, obsolete token cancellation, destruction before metadata followed
by XID reuse, and bound exhaustion/reclamation. Definitive inspection errors and
non-presentation windows discard retained serials; ordinary timeouts retain them
for a future retry. Managed-loop retry scheduling remains outstanding.

## Assembled nonblocking XWM driver

The driver now connects discovery, manager acquisition, enumeration, window event
routing and inspection scheduling. Early initialization events remain bounded and
ordered across phase changes. Inspection scheduling shares the tracking turn's
one-millisecond deadline. The owner-facing interface exposes read/write readiness,
request deadlines, policy actions and unhandled events without publishing DISPLAY.
Manager selection loss closes the driver transport and clears presentation state.

The follow-up passed **76 Xwayland component tests**, the all-target
`desktop-xwayland` compile check, scoped formatting and `git diff --check`.
A socket fixture progresses from completed discovery through manager acquisition
and root enumeration, injects window creation before manager completion, then
maps/associates that same window and verifies teardown on selection loss. A
separate fixture verifies setup-deadline failure. This is component integration,
not native desktop-loop or real-server evidence. Output publication, configuration
and window policy, session readiness, helper lifecycle integration and runtime
qualification remain outstanding.

## Asynchronous map and geometry commands

The driver accepts policy-approved map and geometry commands for live XWindow
identities. Each void request has an asynchronous reply barrier; at most 256
command groups are tracked. Optional command/barrier failure reports one action
for the original live window. Late failures cannot target a reused XID. Stale
identities and zero dimensions are rejected before queuing; partial queue failure
closes the compatibility connection rather than pretending the command was atomic.

The follow-up passed **78 Xwayland component tests**, the all-target
`desktop-xwayland` compile check, scoped formatting and `git diff --check`.
The driver socket fixture checks map/configure opcodes and negative-position/size
wire values, confirms that successful barriers leave map/geometry state unchanged,
and then verifies state updates from MapNotify/ConfigureNotify. Further tests cover
duplicate timeout suppression and stale-window failure isolation. These are command
primitives: configure-request policy, synthetic ICCCM notifications, focus/stacking,
WM_STATE and managed desktop integration remain outstanding.

## Configure-request routing and synthetic notifications

ConfigureRequest decoding preserves only fields selected by its value mask,
including border and stacking hints; it does not apply xdg acknowledgement rules.
Unknown-window map/configure requests are retained in order through inspection,
with a sixteen-request per-window limit and configured-capacity total limit.
Requests are not merged because each needs a policy response. Definitive discard
or destruction clears retained requests before XID reuse.

The driver can queue a checked synthetic ConfigureNotify with root coordinates
adjusted for the client's requested border and that requested border width. The
caller supplies this geometry according to [ICCCM 4.1.5](https://www.x.org/releases/X11R7.7/doc/xorg-docs/icccm/icccm.pdf).
The API checks target/sibling lifetimes and preserves override-redirect metadata.
The follow-up passed **81 Xwayland component tests**, the all-target
`desktop-xwayland` compile check, scoped formatting and `git diff --check`.
Tests verify masked fields without actual-geometry mutation, ordered inspection
replay, queue overflow/destruction cleanup, notification target/mask/geometry and
unchanged server-confirmed geometry after notification. Window policy deciding
when to honor, adjust or reject requests remains outstanding.

## WM_PROTOCOLS metadata and cooperative close

Policy can subscribe to window PropertyNotify events and request WM_PROTOCOLS.
Reads are limited to 256 atoms/1024 payload bytes, with 128 outstanding reads,
4096 window records and sixteen requests per scheduling turn. Incomplete or
wrong-type/format properties grant no capabilities. Authoritative property changes
invalidate cached capabilities and coalesce while a read is outstanding; older
revisions and discarded-window replies cannot restore capabilities.

Cooperative close sends WM_DELETE_WINDOW through a checked ClientMessage with
WM_PROTOCOLS type, format 32, the supplied nonzero timestamp and empty event mask,
following [ICCCM 4.2.8](https://www.x.org/archive/current/doc/xorg-docs/icccm/icccm.pdf).
No application process or X client is killed. Unknown/unsupported protocol metadata
returns a structured compatibility error. WM_TAKE_FOCUS is parsed but its policy
and input-focus behavior remain unimplemented.

The follow-up passed **84 Xwayland component tests**, the all-target
`desktop-xwayland` compile check, scoped formatting and `git diff --check`.
The driver fixture verifies the property-change subscription, bounded property
read, protocol publication, unavailable-close rejection and close message wire
fields. Reader tests cover property storms, stale revisions, incomplete/invalid
values, and discarded-window replies. Initial policy-managed subscriptions remain
an explicit driver call; native desktop window policy and runtime qualification
remain outstanding.

## WM_HINTS and ICCCM focus commands

The shared property reader now supports WM_HINTS with a nine-word/36-byte bound,
strict type/format/completeness validation, InputHint flag handling and urgency.
Telorgon defaults absent/unflagged input hints to accepting input. Invalid hints
remain unavailable. Initial hints reads require the property subscription already
established by protocol reads; PropertyNotify invalidates and refreshes hints.

The driver selects all four [ICCCM focus models](https://www.x.org/releases/X11R7.7/doc/xorg-docs/icccm/icccm.pdf).
Passive/locally active clients receive SetInputFocus with Parent reversion;
locally/globally active clients receive WM_TAKE_FOCUS with the supplied interaction
timestamp. No-input clients receive neither. Commands require a mapped associated
window, current metadata and a nonzero timestamp. This API executes a trusted
compositor decision; it does not implement activation authorization, timestamp
freshness policy, native focus transfer, lock layering or seat routing.

Shared property/inspection scheduling now also checks connection-wide request
capacity. Saturation waits for replies, and policy commands reject as busy before
queuing a command/barrier pair. A regression fills the shared request table and
verifies property reads resume only after a slot is released.

The follow-up passed **87 Xwayland component tests**, the all-target
`desktop-xwayland` compile check, scoped formatting and `git diff --check`.
Tests cover the four-model matrix, hint validation/defaults, locally active wire
requests, globally active message-only behavior, property invalidation and stale
hint rejection. Input/IME and real desktop focus qualification remain open.

## Native desktop window identities (W02)

The shell now exposes a generational WindowId independent of Wayland surface IDs.
The native host allocates it for published xdg toplevels through a separate identity
table shared by synchronous and asynchronous image-publication paths. Surface
withdrawal removes presentation but retains the identity while the surface still
exists; destruction/client disconnect retires it. Recycled slots increment their
generation, and exhausted generations are permanently retired.

The inspected lifetime boundary is CompositorCore::destroy_surface (which always
queues WithdrawSurface) and the desktop owner's WithdrawSurface handler (which can
check whether the surface remains in WaylandWorld). Tests cover stable repeated
publication identity, duplicate destruction, slot reuse and generation exhaustion.
The adjacent reference library remains absent; this bounded bookkeeping change
uses Telorgon's existing lifecycle and does not change GPU/resource retirement.

The full `desktop-xwayland` library suite passed **1080 tests, one ignored**.
The first parallel run exposed an existing FD-number reuse race in the unimported
DMA-BUF ownership test: after drop, another test can reuse the same `/proc/self/fd`
number. The fixture now checks EOF on a retained Unix-socket peer, which proves
endpoint closure independently of FD-number reuse. GPU implementation is unchanged.
The native-only and compatibility all-target compile checks, scoped formatting and
`git diff --check` passed; the three existing dead-code warnings remain.

This starts W02 but does not complete it. Policy geometry still resides in
ClientWindow, native configure/ack handling is unchanged, and shell snapshot/action
routing plus X11 identity allocation must still join the shared desktop model.

## Native configure transaction separation (W02)

`ClientWindow` now stores resize anchors and terminal configure/acknowledgement
transactions in `NativeConfigureState`. Interactive resize, configure emission,
acknowledgement retention, publication reconciliation, maximize/fullscreen reset,
geometry placement and occluded-frame handling all use that native record. General
requested geometry, restore geometry, window state and retained imagery remain
outside it. This is a data-layout refactor with unchanged native scheduling and
acknowledgement rules; the future X11 adapter still needs separate routing.

Validation: the full `desktop-xwayland` library suite passed **1080 tests, one
ignored**. Existing regression coverage includes terminal configure acknowledgement,
serial wraparound, superseded publication, resize anchors, maximize/restore and
same-turn resize coalescing. Both native-only and compatibility all-target compile
checks passed, with the existing three dead-code warnings. Scoped formatting and
`git diff --check` passed. No GUI, compositor server or hardware presentation ran.
The adjacent reference library is still absent; this bounded state-layout change
uses the existing native contracts and changes no GPU ownership or retirement.
W02 remains partial: presentation storage and protocol-neutral policy/action routing
still need separation and integration.

## Retained surface presentation record (W02)

`SurfacePresentation` owns the retained image revision, surface logical extent,
raster extent, pixel format/alpha metadata, retained pixels and pending full,
regional or external image update. Image application and update consumption now
operate on this record without access to window policy or configure state.
Composition, drag icons, cursor rendering, placement, SHM patch eligibility and
presentation/occluded-frame reporting read the same record. Existing image update
coalescing, hidden-content consumption rules and GPU retirement are unchanged.

The full compatibility library suite passed **1080 tests, one ignored**, including
retained revision and hidden-image regressions. Native-only and compatibility
all-target checks passed with the existing three dead-code warnings; scoped
formatting and `git diff --check` passed. No interactive or hardware run occurred.
The adjacent reference sources remain unavailable; this mechanical record split
preserves existing renderer ownership and changes no synchronization mechanism.

The record is still owned by `ClientWindow`; separate storage, independent
window/presentation lifetimes and native/X11 action routing remain required.
This evidence does not close W02 or qualify X11 desktop operation.

## Shared native/X11 desktop identity pool (W02)

The host identity table now keys entries by `ProtocolWindow`: a native permanent
surface or an X11 server generation/XID/incarnation. Both use the same generational
slot allocator, preventing cross-protocol identity collisions. Existing native
callers keep their lifecycle behavior; X11 admission remains the responsibility of
the future managed adapter and must use live protocol-registry entries.

All **three identity tests passed**. The mixed-protocol regression uses the actual
association state machine to destroy and replace an associated surface while
retaining desktop identity, then verifies XID reuse, stale destruction, server
restart and simultaneous native identity isolation. Native-only and compatibility
all-target compile checks passed, with the existing three dead-code warnings.
Scoped formatting and `git diff --check` passed. This turn did not rerun the full
library suite or launch a compositor. The new key type does not itself establish
managed X11 lifecycle integration, neutral actions or complete W02.

## Managed startup keyboard names (W09)

The managed desktop now passes configured rules/model/layout/variant/options to
`XkbKeyboard::from_names` and publishes the resulting keymap through its existing
seat path. Defaults preserve unspecified names. Declaration validation rejects
interior NULs in all five fields while retaining explicit empty options.

Both targeted tests passed: configuration validation and real libxkbcommon US/DE
compilation with different text for the same physical evdev key. Native-only and
compatibility all-target checks passed with the existing three dead-code warnings;
scoped formatting and `git diff --check` passed. No GUI/server or hardware test ran,
and the full suite was not rerun in this slice. The test uses host keyboard data;
it does not qualify private payload include paths, live replacement, Xwayland
capture or IME operation. W09 remains partial.

## Exclusive compositor keyboard-data root (W10)

`KeyboardConfig::include_root` now reaches the managed seat compiler. An explicit
absolute root creates an XKB context with `XKB_CONTEXT_NO_DEFAULT_INCLUDES` and
appends only that path. Relative or NUL-containing paths and inaccessible roots
fail; failed compilation does not retry against host data. The existing
`from_names` API delegates with no root and preserves its search behavior.

Reference inspected: installed `/usr/include/xkbcommon/xkbcommon.h`, context flags
and `xkb_context_include_path_append` return/ownership contract. The context is
unreferenced on append or compile failure. No graphics mechanism changed.

Two targeted tests passed: an empty exclusive directory cannot compile despite a
working host keymap, and an explicit root with spaces/metacharacters compiles
literally. The latter uses a fixture symlink to host XKB data and does not establish
payload extraction safety or clean-machine qualification. Native-only and
compatibility all-target checks passed, with the existing warnings. Scoped
formatting and `git diff --check` passed. No GUI/server ran; full-suite tests were
not rerun. Automatic payload-root selection and live keymap replacement remain
open, so this does not close W10.

## Reserved emergency pointer release (W09)

The host now reserves Ctrl+Alt+Shift+physical Escape before user shortcut handlers.
The initiating press and subsequent repeat/release edges remain compositor-owned
across modifier and lock changes. The host calls the native pointer-release API
without changing keyboard focus or lock state. Per-seat suppression prevents a
persistent or newly-created constraint from immediately recapturing the same
surface; suppression ends when pointer focus leaves. Destroying a surface also
removes its suppression record.

Two targeted tests passed: shortcut ownership/lock precedence and a real Wayland
socket fixture exercising persistent deactivation, same-focus updates, protocol
object replacement, re-entry and one-shot retirement. The full compatibility
library suite then passed **1087 tests, one ignored**. Native-only and compatibility
all-target checks, scoped formatting and `git diff --check` passed with existing
warnings. No GUI, helper server or hardware run occurred.

The installed pointer-constraints XML was checked for compositor deactivation and
persistent/one-shot lifetime semantics. This preserves its unlocked/unconfined
notification path. W09 remains partial: Xwayland keyboard grabs, shortcut inhibition,
seat-transition capture cleanup and real pointer-warp qualification remain open.

## Matched keyboard event delivery (W09/native regression)

Inspection of seat-transition handling found that native `keyboard_key` ignored
the pressed-key tracker's rejection of duplicate presses and unmatched releases.
A Wayland wire fixture reproduced five delivered events for one valid press/release
pair. The fix returns before serial issuance and protocol delivery when no key
state transition occurred. The regression now requires exactly the two valid wire
events and verifies no keyboard-input serial exists for any rejected edge.

The reproducer failed before the change and passed after it. The full compatibility
library suite passed **1088 tests, one ignored**. Native-only and compatibility
all-target checks passed with existing warnings; scoped formatting and
`git diff --check` passed. No interactive or hardware run occurred.

Seat transitions remain open: the managed host currently gates input dispatch on
libseat's enabled state, but has no coordinated libinput suspend/resume and held
key/button reset. This matched-edge fix is a prerequisite, not evidence that stale
physical state has been cleared on suspend. W09 and the seat portion of W07 remain
partial.

## Managed seat input suspension (W09/W07)

The managed host uses deferred libseat disable acknowledgement. A pending-disable
flag survives even if enabled state changes before owner cleanup. The owner
suspends libinput (closing devices), drains queued input/removal events, acknowledges
disable, cancels drag/touch and clears protocol physical state/focus. Shortcut-held
keys are released in XKB and interactive resize is finalized. Resume calls
libinput_resume and restores only a still-present, non-minimized keyboard target
when unlocked and no newer protocol focus exists. Input reopen errors remain
explicit host errors. The existing public automatic-acknowledgement seat API keeps
its behavior; deferred users must promptly acknowledge pending disables.

Contracts inspected locally: `/usr/include/libseat.h` disable callback/acknowledgement
and `/usr/include/libinput.h` suspend/resume device closure and return values.
No graphics retirement or KMS reconstruction mechanism was changed.

The full compatibility library suite passed **1090 tests, one ignored**. Added
coverage verifies pending disable survives an intervening enable, shortcut reset
allows fresh ownership, and the existing Wayland capture fixture now verifies
held keyboard/button state and focus clear idempotently on suspension. Native-only
and compatibility all-target checks passed with existing warnings. Scoped formatting
and `git diff --check` passed.

No real libseat session, libinput device reopen, VT switch or KMS recovery was run.
These remain mandatory hardware gates; keyboard-grab/inhibition teardown is still
open. W07/W09 remain partial despite the new input lifecycle integration.

## Keyboard ownership at lock boundaries (W09)

Lock requests already cancelled pointer buttons and pointer focus, but retained
pressed keyboard keys could appear in a later lock-client enter. The host now
suppresses held physical keys and calls an explicit keyboard-delivery reset on
lock, unlock and pending-lock cancellation. This sends keyboard leave and clears
pressed keys/modifiers in the protocol seat. It does not reset ordinary focus
transitions or synthesize a fresh key press. Physical key ownership remains until
release, preventing repeats or release edges from crossing the boundary.

The shortcut regression covers a held key through lock and unlock before release.
The keyboard wire fixture now verifies a subsequent enter has an empty key array
and a stale release is absent. The full compatibility library suite passed
**1091 tests, one ignored**. Native-only and compatibility all-target checks passed
with existing warnings; scoped formatting and `git diff --check` passed. No real
lock-screen, hardware or Xwayland keyboard-grab test ran; W09 remains partial.

## Native shortcut-inhibition protocol (W09)

Added exact v1 descriptors, wire contract and runtime global for the installed
`keyboard-shortcuts-inhibit-unstable-v1.xml`. Mapped keyboard focus gates activation;
normal focus/unmap loss is silent as specified. The managed host bypasses its
configurable shortcut handler while active, retaining reserved emergency handling.
User release sends inactive and retains a per-surface revocation across object
replacement. Lock surfaces are ineligible; existing focus cleanup handles lock
and seat loss. Object/surface/client destruction removes corresponding state.

The wire fixture passed, covering unmapped denial, mapped focus activation,
silent focus loss, reactivation, explicit inactive, replacement after revocation,
duplicate rejection and disconnect cleanup. Mapping is modeled directly in the
surface state; requests/events use actual local Wayland sockets. This is not a
rendered-client or hardware test. The full compatibility library suite passed
**1092 tests, one ignored**. The protocol-generation integration suite passed
**12 tests, one ignored**. Its first run exposed a pre-existing positional assumption
that core Wayland occupied the first catalog entry; the fixture now selects the
core profile by name. Native-only and compatibility all-target compilation passed
with existing warnings. Scoped formatting and `git diff --check` passed.

There is no user-facing re-enable action for an explicitly revoked surface.
Xwayland keyboard-grab protocol and real input/capture qualification remain open;
W09 is not complete.

## Dedicated Xwayland keyboard-grab protocol (W09)

Added v1 protocol/profile/wire descriptors using the installed upstream XML and
restricted both registry visibility and bind/request admission to XwaylandAccess's
actual dedicated client. Grants require existing mapped keyboard focus on a modern
Xwayland surface, bypass configurable shortcuts, and never steal focus. Focus loss
or unmap cancels the object without automatic reactivation; emergency release
revokes replacement requests for that surface lifetime. Existing lock/seat focus
cleanup and object/surface/client destruction revoke grants.

The socket fixture passed for private visibility, hostile guessed bind, unfocused
denial, focused grant, focus-loss cancellation, fresh grant, emergency revocation,
replacement denial and surface teardown. It models buffer mapping directly but
uses actual wire requests for roles/serials/grabs. No Xwayland server or GUI ran.
The full library suite passed **1093 tests, one ignored**; protocol descriptor
tests passed **12 tests, one ignored**. Native-only and compatibility all-target
checks passed with the existing warnings. Scoped formatting and `git diff --check`
passed.

This closes the missing grab-protocol component, not W09: managed Xwayland window
routing, pointer-warp behavior and real input/capture application qualification are
still required. No runtime game or remote-client support is claimed.

## Initial xdg-output descriptions (W07)

Added xdg-output v3 descriptors/profile/runtime creation. Logical extent applies
rotation/reflection axis exchange before the existing ScaleFactor logical-size
conversion; position comes from the same OutputState. Names/descriptions respect
v2 introduction and names are validated/unique at output registration. Completion
uses xdg-output done below v3 and wl_output done at v3 with wl_output v2+. A mixed
v3/legacy wl_output v1 binding receives deprecated xdg-output done rather than an
opcode unsupported by its wl_output version.

The installed upstream XML supplied event/version and transformed-size semantics.
The adjacent reference tree remains unavailable. A narrow read of
[wlroots xdg-output implementation](https://raw.githubusercontent.com/swaywm/wlroots/master/types/wlr_xdg_output_v1.c)
confirmed separation of description events and output completion; Telorgon does
not copy its implementation or send wl_output done to a v1 resource.

The wire fixture passed for versions 1/2/3, negative position, 150% scale, 90-degree
rotation and completion ordering, including the legacy output-version combination.
The full library suite passed **1094 tests, one ignored**; descriptor tests passed
**12 tests, one ignored**. Native-only and compatibility all-target checks passed
with the existing three warnings. Scoped formatting and `git diff --check` passed.
No output hardware or Xwayland server ran.

This slice publishes initial descriptions only. Live layout updates, atomic layout
revision publication, hotplug/removal and KMS reconstruction remain W07 work. The
legacy completion fallback still requires real-client qualification.

## Output-state update publication (W07)

`NativeCompositor::update_output` validates and replaces one existing output
snapshot, publishes core properties without an early done, updates attached
xdg-output descriptions and then emits core done. Xdg objects retain their
original wl_output protocol-object identity, avoiding raw-pointer reuse when a
parent binding is released. Names are initial-only; xdg v2 descriptions are
initial-only. Renames, mode-list changes and enabled changes require future global
reconstruction and are rejected before mutation. Invalid initial mode indices are
also rejected during output registration. Equal updates emit no events.

A scale change on the first enabled output refreshes fractional-scale and
wl_surface v6 preferences using the existing single-output policy. It does not
invent output membership or change renderer/KMS state.

The extended wire fixture passed for all four version combinations, checking
new negative position, 200% scale after rotation, immutable names/descriptions,
exactly one core completion, no-op deduplication and rejection without mutation.
The full compatibility library suite passed **1094 tests, one ignored**. Native-only
and compatibility all-target checks passed with the existing warnings. Scoped
formatting and `git diff --check` passed. No hardware output change ran. Whole-layout revisions, physical topology changes,
per-surface output membership and managed host integration remain W07 work.

## Revisioned output-description batches (W07)

`update_outputs` validates all replacements and duplicate IDs before mutation,
advances a checked layout revision once per changed batch, and emits core/logical
metadata before core completions. Single-output updates delegate to it. Detached
OutputLayoutSnapshot values preserve prior descriptions/revision; output registration
also advances revision and rolls back the inserted state if global creation fails.
Direct mutation of core.outputs bypasses this contract and is not a supported
publication path.

The new state regression covers two-output replacement, invalid later entries,
duplicates, no-op batches, retained snapshots and revision exhaustion. The full
library suite passed **1095 tests, one ignored**. The extended wire fixture checks
that a rejected later entry produces no partial metadata and passed in its targeted
run. Native-only and compatibility all-target checks passed with the existing
warnings; scoped formatting and `git diff --check` passed. No physical output or
GUI ran.

The adjacent rendering references remain absent. This change is owner-thread
metadata validation/publication and does not modify GPU or KMS lifetimes. Shared
snapshot consumption by rendering/input/XWM, output removal/topology reconstruction
and hardware qualification remain open. W07 is still partial.

## Checked root-coordinate mapping slice

Implemented revision-bearing root bounds from enabled output snapshots and checked
bidirectional desktop/root translation. Root-child X11 geometry conversion validates
signed coordinates and positive unsigned dimensions without scaling or clamping.
The native startup extent now comes from its published output state.

Three new tests cover negative origins, rotated 150% scaling, offscreen roundtrips,
empty/disabled layouts, invalid mode indexes, extent/union/translation overflow,
and X11 coordinate/dimension boundaries. Full desktop-xwayland library suite:
**1,098 passed, 1 ignored** (`/tmp/telorgon-root-full-tests.log`). Native-only
all-target compilation passed (`/tmp/telorgon-root-native-check.log`).

This is a metadata/coordinate boundary change; GPU submissions, resource ownership
and KMS scheduling are unchanged. It follows existing Telorgon logical-size rounding,
rejecting the float-to-integer saturation boundary instead of silently clipping.
Adjacent rendering references remain unavailable. Real multi-monitor/RandR alignment,
shared live snapshot ownership and hardware qualification remain unrun/outstanding.

## Asynchronous normal-size-hints slice

Added bounded WM_NORMAL_HINTS reads to the XWM tracking phase, with property-change
invalidation, lifetime/revision checks, timeout routing and destruction cleanup.
The parser covers 15/18-word records, selected min/max/increment/aspect/base/gravity
fields and absent properties. Invalid flagged dimensions, aspect ratios, gravity,
wire types, lengths and trailing data do not become usable policy metadata.

Three new tests cover parsing and asynchronous stale/destruction behavior; the
existing XWM socket fixture now checks actual normal-hint requests, replies,
invalidation and deletion. All-target desktop-xwayland compilation passed
(`/tmp/telorgon-normal-hints-check.log`). Full serial library regression:
**1,101 passed, 1 ignored** (`/tmp/telorgon-normal-hints-serial.log`). The initial
parallel run had 1,100 pass and one existing inspection-fixture failure:
`discovery.rs`'s test request reader received WouldBlock. Serial rerun passed that
test; parallel fixture timing remains an unresolved test-harness limitation, not
proof of runtime failure or a fully green parallel run.

The wire/reference source is linked in X11_COMPATIBILITY.md. This slice supplies
validated metadata; actual resize constraint enforcement, placement/gravity policy,
and real legacy-client acceptance remain outstanding. No GUI/server was launched.

## XWM fixture dispatch-budget correction

Follow-up to the normal-hints parallel-suite failure: discovery and initial window
inspection fixtures previously called bounded dispatch once before blocking on the
same-thread peer's request read. A turn can legitimately retain queued writes after
its time/operation budget. Fixtures now drain known request-only writes with a
two-second failure deadline, checking `wants_write` and asserting that no inbound
setup/packets are discarded. Discovery setup waits for actual initialization and
request-drain state; extension discovery drains dynamically queued version requests.
Production transport budgets and blocking behavior are unchanged.

The parallel full library suite passed **1,101 tests, 1 ignored** after this change
(`/tmp/telorgon-fixture-drain-tests.log`). An additional deterministic regression
queues 260 requests, proves that one dispatch leaves work after the 256-operation
write limit, then checks that the fixture drains and preserves every request.
The discovery test family including that regression passed
(`/tmp/telorgon-fixture-budget-tests.log`). This addresses the observed one-turn
assumption; it does not establish that every unrelated timing-sensitive fixture is
free of scheduling assumptions. No compositor or real server was launched.

## Real helper build and relocation checks

The workstation preflight lacked Meson, Ninja, patchelf, Bison/byacc and several
development libraries. This was resolved for compilation by preparing an isolated
Ubuntu 24.04/glibc 2.39 build environment. Canonical's Ubuntu Base 24.04.4 checksum
signature verified with the installed Ubuntu archive keyring, fingerprint
`843938DF228D22F7B3742BC0D94AA3F0EFE21092`. No system package changes were made.

All five planned source archives matched their locked hashes. Building exposed
one additional input: X.Org protocol headers 2024.1 are required for Present 1.4.
That archive is now pinned too. `sources.lock.toml`, `build-debs.lock.json` and
`build-packages.lock.tsv` record the base, 191 additional package archives and
installed package inventory. X.Org release signature/source-license review and
independent reproducibility remain open.

The checked-in `build.py --source-cache ... --compile ...` recipe passed from a
fresh tree, applying both patches with zero fuzz. It built the pinned Xwayland,
xkbcomp, libXfont2 and keyboard data with networking disabled. Static ELF staging
found **21 private libraries**; it checked symbol floors and assigned private
RUNPATHs. Host drivers and the ordinary C runtime were not bundled.

`test_helpers.py` passed **nine checks** using the actual relocated compiler and
the exact patched Xwayland keymap functions compiled into a C harness:

- US, German, French, US Dvorak, and US/Russian with group switching.
- Parent identity mismatch rejection, the 4 MiB input bound, five-second helper
  timeout/reaping, and scratch symlink rejection.

The relocation directory contained spaces, a semicolon and literal shell
substitution syntax. No shell substitution ran. No external xkbcomp, xauth, xrdb,
or system XKB data was used by these compiler tests. This does not yet prove a
real Xwayland server has no additional runtime dependency.

Initial feasibility artifact (not a release):

| Property | Recorded value |
|---|---|
| Local archive | `/tmp/telorgon-xwayland-build/xwayland.payload` |
| SHA-256 | `03e6f25563de92c3dcc2e2525202fdd5864bfefcd5a53a2fcc05a4389aed8599` |
| Compressed archive | 4,475,074 bytes |
| Extracted entries | 346 files, 12,433,953 bytes |
| Component/input records | 28 |

The archive is not the final compositor executable. Runtime dlopen/file tracing,
font queries/rendering, server FD/authentication/parent-death cycles, supervised
launch, clean images, complete SBOM/license obligations and release qualification
remain outstanding.

## Work-item disposition

| Work | Current disposition |
|---|---|
| W01 native baseline | Partial: compile and selected state evidence; real native desktop baseline outstanding |
| W02 window/presentation split | Partial: native generational identity survives presentation withdrawal and native configure transactions/retained imagery have separate records; independent presentation lifetimes and neutral action routing remain open |
| W03 packaging/private launch | Partial: payload, extraction, subprocess supervision, lifecycle/readiness, display reservation, Xauthority and pinned command preparation tested; dedicated client registration, real server/authentication qualification and managed integration outstanding |
| W04 XWM and association | Partial: socket framing, association state, dedicated-client filtering and authenticated xwayland-shell dispatch tested; XWM and managed commit routing outstanding |
| W05 desktop policy | Not implemented |
| W06 graphics/synchronization | Not implemented; no capability advertisements changed |
| W07 multiple outputs | Not implemented |
| W08 transfers | Not implemented |
| W09 capture/keymaps | Not implemented |
| W10 helpers/data/settings | Partial: private compiler/data patches and static library closure tested; settings/runtime tracing outstanding |
| W11 Steam/Wine/legacy clients | Not run |
| W12 Horizon/remoting | Validation-blocked: no real Horizon infrastructure; no SPICE/FreeRDP runs |
| W13 release | Not qualified; no final single-executable artifact |

All T01–T13 acceptance families remain open. In particular, no Intel/AMD/NVIDIA
GPU, monitor/hotplug, native-survival-on-Xwayland-crash, authentication, 32/64-bit
application, clean-image, noexec/LSM, resource-cycle, performance or Horizon
acceptance results are claimed. Real Horizon desktop and published-application
sessions cannot be replaced by these synthetic tests.

## Normal-hint size validation

Added constant-work integer validation of client sizes against minimum/maximum,
increment and aspect constraints. Explicit base size is subtracted for aspect
checks; minimum size substitutes only for the increment base. The XWM provides
`configure_window_with_hints`, which rejects unavailable or violated hints before
queueing a configure. Policy can deliberately use the existing configure operation
for overrides. Automatic nearest-size selection and managed-host resize integration
remain outstanding.

The targeted regression passed (`/tmp/telorgon-size-constraints.log`), covering
base/minimum distinction, exact increments, limits, invalid increments, zero sizes
and large aspect products. This is unit evidence, not real-client qualification.

## Hint-aware configure wire regression

Extended the XWM socket fixture to verify that a below-minimum configure emits no
request, an accepted configure emits the expected dimensions and checked-request
barrier, and neither queuing nor successful barrier completion changes the registry's
server-confirmed geometry. The fixture also rejects configure while normal hints
are invalidated and the replacement property reply remains pending. Both XWM driver
tests passed (`/tmp/telorgon-hint-configure-wire.log`). This validates the command
boundary; interactive resize selection and managed host integration remain open.

## Focus timestamp ordering

The XWM now tracks its last successfully queued focus decision per tracking
instance and rejects older/half-range-ambiguous timestamps before issuing either
SetInputFocus or WM_TAKE_FOCUS. Equal timestamps remain valid; zero remains
forbidden. Wrapping subtraction handles the 32-bit clock rollover. History older
than the half-range interval is no longer used to order newly authorized requests.
NoInput decisions do not advance history. This is local request ordering, not
server-confirmed focus state, future-time validation or authorization evidence.
The caller must still enforce trusted interaction, lock and seat policy.

Three XWM tests passed (`/tmp/telorgon-focus-time-tests.log`), including rollover,
zero, equality, stale/ambiguous values, old history and wire-sequence preservation
when stale focus requests are rejected. Reference:
[X protocol SetInputFocus](https://www.x.org/archive/current/doc/xproto/x11protocol.pdf).
Full focus-stealing prevention and real-client qualification remain outstanding.

## Compound focus backpressure

Focus now checks request slots and command-group capacity for its complete ICCCM
focus model before queueing anything. This closes a partial-queue case where three
remaining request slots allowed SetInputFocus plus its barrier, then rejected
WM_TAKE_FOCUS's checked command while retaining the first command. Ordinary commands
also reject exhausted group capacity before entering the mutation path. Transport
queue failures still follow the existing compatibility-connection teardown path.

The XWM socket regression injects a three-slot request tracker and verifies rejection
preserves its slots, command groups, tracking connection and focus timestamp, then
restores capacity and checks the usual focus wire sequence. All three XWM tests
passed (`/tmp/telorgon-focus-capacity.log`). Real input/focus qualification remains open.

## Combined XWM policy regression checkpoint

After normal-hint validation, hint-aware configure, timestamp ordering and compound
focus capacity changes, native-only and desktop-xwayland all-target checks passed
(`/tmp/telorgon-policy-native.log`, `/tmp/telorgon-policy-compat.log`). Full serial
library regression passed **1,104 tests, 1 ignored**
(`/tmp/telorgon-policy-serial.log`).

The parallel run passed 1,103 tests and failed
`payload::tests::cold_and_warm_cache_share_immutable_generation` at the immediate
exclusive-lock assertion after dropping both leases (`/tmp/telorgon-policy-full.log`).
The same test passed serially. Lease files are opened with O_CLOEXEC; transient
inheritance by concurrent subprocess fixtures is a hypothesis, not an established
cause. No production lease semantics or test assertion were weakened. Parallel
lease-test reliability remains an investigation item. This checkpoint does not
qualify actual desktop, GPU, application or clean-machine behavior.

## Host capture cleanup on seat suspension

The managed desktop seat-disable path now clears host touch-slot routing and
cancels primary-pointer state in every decoration runtime after native seat
cancellation. It requests repaint so held decoration visuals do not survive
resume. Previously the native touch/focus/button state was reset while these host
records remained. New device activity must establish new interaction state.

Native desktop all-target compilation passed (`/tmp/telorgon-seat-host-check.log`).
Two existing pointer-cancellation component regressions passed
(`/tmp/telorgon-seat-cancel-tests.log`); these verify underlying cancellation behavior,
not the actual libseat disable/resume sequence. Real VT/device and KMS reconstruction
qualification remains unrun. No graphics resource ownership or lease behavior changed.

## Deferred focus during seat suspension

Added a persistent per-seat suspension gate for keyboard and pointer focus. Pending
policy requests replace bounded state without protocol enters; resume applies valid
remaining targets with distinct generated serials. The host preserves explicit
pending keyboard clears and does not overwrite them with its fallback focus.
Repeated disable notification no longer loses that fallback. The socket regression
covers deferred keyboard/pointer focus, repeated suspension, resume, explicit clear,
idempotent resume and unknown-seat errors. A serial collision found by the new test
was fixed before completion.

Targeted fixture passed (`/tmp/telorgon-suspended-focus.log`); full serial library
suite passed 1,104 tests with 1 ignored (`/tmp/telorgon-suspended-focus-full.log`).
Native-only all-target compilation passed (`/tmp/telorgon-suspended-focus-check.log`).
This does not qualify physical seat/VT transitions or lock-screen behavior on hardware.

## Deferred focus target revocation

Unmapped commits (including released synchronized child commits) and surface
resource destruction now revoke matching suspended keyboard/pointer targets.
Revocation retains an explicit keyboard clear, preventing fallback restoration by
the host. This closes the case where an unmapped but surviving surface remained a
pending resume target. The existing local wire fixture now sends an unmapped
wl_surface commit and then destroys the surface during separate suspensions; both
resume with no keyboard or pointer focus. The fixture passed
(`/tmp/telorgon-deferred-targets.log`). Real suspended-seat behavior remains unqualified.

## Late input during suspension

Native seat input delivery now ignores keyboard keys/modifiers, pointer buttons,
motion/relative motion/axes and touch down/motion/up while focus delivery is
suspended. This prevents late queued events from rebuilding pressed state or
reporting unknown touch identities after cancellation. The host's device gate
remains in place. The local seat fixture injects late key/button presses, modifiers
and a touch sequence, verifies empty pressed/touch state, then resumes focus.
It passed (`/tmp/telorgon-suspended-events.log`). Hardware event ordering remains
unqualified; protocol cancellation and focus clearing still run during suspension.

## Drag requests during suspension

`wl_data_device.start_drag` now ignores requests while its seat is suspended,
before serial consumption or drag-source/ownership changes. The local wire fixture
binds a data device and sends a drag request using a pre-suspension button serial;
it verifies that the client remains connected, no drag starts and the request does
not consume the serial. The fixture passed (`/tmp/telorgon-suspended-drag.log`).
This closes suspension-time recapture; comprehensive implicit-grab serial matching
and real drag-and-drop qualification remain separate work.

## Live pointer-grab requirement for drag initiation

Pointer-driven start_drag now requires an active implicit grab on the requesting
client's origin surface, before consuming the authorization serial. A retained
button serial alone cannot start a drag after suspension clears all button owners.
The wire fixture checks a stale post-resume request is ignored, then delivers a
fresh press and verifies drag starts and subsequent suspension cancels it. It
passed (`/tmp/telorgon-drag-live-grab.log`). Matching each serial to its exact held
button/press generation remains outstanding; this check establishes current grab
ownership, not that stronger correspondence.

## Pointer drag serial-to-press matching

Native pointer delivery records each client press's serial and focus identity by
seat/button. Release, suspension and surface destruction remove the corresponding
records. Pointer drag initiation requires the supplied serial to match a still-held
button owned by the current implicit grab, including its enter identity. A new grab
therefore cannot rehabilitate a retained serial from before suspension or focus
replacement. The local fixture tries the old serial after a fresh press, verifies
no drag starts, then verifies the fresh serial starts a drag. It passed
(`/tmp/telorgon-drag-press-serial.log`). X11 selection/DnD bridging and actual client
qualification remain outstanding.

## Combined seat and drag regression checkpoint

Following suspended focus, late-input suppression, deferred-target revocation and
pointer press serial matching, the full serial library suite passed **1,104 tests,
1 ignored** (`/tmp/telorgon-seat-drag-full.log`). Both native-only and compatibility
all-target checks passed (`/tmp/telorgon-seat-drag-native.log`,
`/tmp/telorgon-seat-drag-compat.log`). The socket fixture was then extended to assert
that button release removes its press-authorization record; that targeted fixture
passed (`/tmp/telorgon-press-release-cleanup.log`). No production changes followed
the full run. Physical input, lock/VT transitions and actual Xwayland application
qualification remain unrun. The previously recorded parallel payload lease-test
failure is not resolved by this serial checkpoint.

## Legacy drag source action fallback

Offer construction now applies the legacy copy fallback when either the source
or target offer predates version 3. Previously only target version was considered,
so a version-1/2 source could advertise an empty action mask to a version-3 target
and fail negotiation. Modern source/target pairs retain the source's exact actions.
The version-matrix unit regression passed (`/tmp/telorgon-drag-versions.log`).
Offer source_actions events use the same selected mask as internal negotiation.
Mixed-version end-to-end transfers and X11 bridge qualification remain outstanding.

## Data-source action lifecycle

DataSource now owns checked action assignment: empty masks, repeated assignment
and assignment after use fail without changing its actions. Native set_actions
uses that method. A source with configured drag actions cannot become the clipboard
selection, preserving the protocol's drag-only source contract. The prior selection
cleanup fixture was corrected to create a selection source with no drag actions.
Four data-device state tests passed (`/tmp/telorgon-source-actions.log`), including
repeated/late assignment and failed selection leaving the source available for drag.
The local Wayland XML set_actions contract was inspected. Wire error-code coverage
and actual application transfers remain outstanding.

## Empty action mask correction

Follow-up protocol review corrected the preceding lifecycle implementation: an
empty declared action mask must be distinguished from an unset mask. DataSource
now records `actions_set`; even an empty declaration consumes the one allowed call
and reserves the source for drag. Native v3 drag startup checks that declaration
rather than requiring nonempty actions. Unknown bits remain rejected by mask parsing.
The four data-device tests passed (`/tmp/telorgon-empty-actions.log`), including
empty declaration, repeated-call rejection and clipboard exclusion. This supersedes
the prior ledger statement that empty masks should fail. DataSource struct literals
now require the actions_set field. Actual mixed-version transfers remain unqualified.

## Action declaration wire errors

Added a local v3 data-source wire test: the first empty set_actions keeps the client
connected and records an unused, explicitly configured source. A repeated call
produces wl_data_source.invalid_source (1); unsupported mask bits produce
invalid_action_mask (0). Native dispatch now explicitly posts invalid_source for
repeated/late action assignment instead of the generic error code. The fixture
checks the offending resource and exact wl_display.error payload and passed
(`/tmp/telorgon-source-actions-wire.log`). It handles registry callback delete_id
events before the error. Application transfer qualification remains outstanding.

## Distinct MIME offer bounds

An unused source at its 128-type limit now accepts a duplicate MIME offer without
allocating or changing the type list. Previously the capacity check ran before
deduplication and rejected that request. A 129th distinct type still fails, and
used sources remain immutable even for duplicates. All five data-device tests
passed (`/tmp/telorgon-mime-bounds.log`), including these boundary cases. Transfer
bridging and application qualification remain outstanding.

## Data object registration atomicity

Duplicate source/offer registration now uses map entry checks, preserving live
records on rejection. Previously insert replaced the record before returning
DuplicateObject. The regression retains an active selected source and its used
state, rejects different-owner replacement, rejects offer-target/finished-state
replacement, and verifies later source removal still revokes the original offer.
All six data-device tests passed (`/tmp/telorgon-data-registration.log`). Native
wire object allocation already provides a separate identity guard; this correction
also makes the state API safe for future internal bridge endpoints.

## Data-device regression checkpoint

After mixed-version source action fallback, explicit action-declaration state,
wire error handling, distinct MIME bounds and duplicate-registration preservation,
the full serial library suite passed **1,109 tests, 1 ignored**
(`/tmp/telorgon-data-device-full.log`). Native-only and desktop-xwayland all-target
compilation passed (`/tmp/telorgon-data-device-native.log`,
`/tmp/telorgon-data-device-compat.log`). Existing three dead-code warnings remain.

This validates the accumulated native data-device and XWM regressions at library
scope. It does not complete W08: bidirectional X11 selection endpoints, PRIMARY,
bounded asynchronous INCR, conversion, loop prevention and Xdnd remain missing.
No actual desktop, application or hardware acceptance runs were performed. The
separate parallel payload lease-test timing failure remains unresolved.

## Bounded selection transfer pump

Added a nonblocking owned-socket byte pump with bounded buffering, per-turn I/O/time
budgets, configurable total limit, inactivity expiry and explicit terminal states.
Three local socket tests passed (`/tmp/telorgon-transfer-tests.log`): binary fidelity
and EOF/drain ordering; oversize/timeout/cancel/disconnect cleanup; stalled receiver
backpressure reaching the buffer limit and expiring without false completion.
Terminal cleanup also releases the buffer allocation. The pump is not registered
with the managed event loop or connected to X11 INCR; concurrent-transfer admission,
selection ownership, conversion and full transfer acceptance remain outstanding.

## Transfer recovery under backpressure

Added a one-MiB patterned-byte socket regression that deliberately stops receiver
reads until the pump reaches its buffer limit, then drains in non-aligned chunks
across many dispatch turns. It verifies exact byte order, bounds, readiness at
capacity and successful EOF/drain completion after backpressure clears. All four
transfer tests passed (`/tmp/telorgon-transfer-recovery.log`). This tests the byte
pump only; selection ownership and X11 INCR handshakes remain unimplemented.

## Transfer admission and generation teardown

Added a transfer registry with a 16-entry concurrency limit, monotonically allocated
non-reused IDs, earliest deadline query and ownership-generation cancellation.
Rejection drops supplied endpoints; identity exhaustion fails instead of wrapping.
The owner must unregister readiness sources before cancelling/removing entries and
remove terminal transfers after observing their result. Generation tokens are
supplied by the future selection owner; the registry does not implement ownership
or loop prevention itself.

Five transfer tests passed (`/tmp/telorgon-transfer-registry.log`). Registry coverage
includes the 17th admission failing, cancellation closing matching peer sockets,
other-generation survival, stale ID rejection, repeated removal and ID exhaustion.
Event-loop dispatch scheduling, selection generations and X11 INCR remain outstanding.

## Shared selection-transfer dispatch budget

The transfer registry now services ready and expired entries under one shared
one-millisecond work deadline, passing the remaining deadline into each pump.
Round-robin ordering resumes after the last serviced ID. Deferred/locally yielding
IDs are returned for rescheduling without requiring a new readiness edge. Per-entry
errors do not abort the batch. Callers still must register FD/timer sources, retain
reschedule IDs and remove terminal entries after processing results.

Six transfer tests passed (`/tmp/telorgon-transfer-scheduling.log`). New coverage
uses an expired work budget to prove no work is lost, checks rotated service order,
and verifies an expired transfer fails independently while another forwards bytes.
This supplies scheduling mechanics, not managed-host or X11 selection integration.

## Transfer foundation regression checkpoint

The accumulated transfer pump, recovery, registry and shared-budget scheduler
passed the full serial library suite: **1,115 tests, 1 ignored**
(`/tmp/telorgon-transfer-full.log`). Native-only and desktop-xwayland all-target
checks passed (`/tmp/telorgon-transfer-native.log`, `/tmp/telorgon-transfer-compat.log`).
Three existing dead-code warnings remain. These results cover library behavior;
no managed readiness registration, X11 INCR or cross-protocol transfer was exercised.
Those implementation and acceptance gates remain open, as does the separately
recorded parallel payload lease-test issue.

## INCR receive state machine

Added `xwayland::incr::Receiver` for receiving ICCCM incremental properties. It
validates the announcement's lower bound and bounded property layout, retains at
most one 256-KiB chunk, withholds property deletion until downstream consumption,
preserves property type/format, checks total limits and inactivity, and completes
only after final empty-property deletion is queued. Failed/cancelled receivers
release buffered data. The owner must route replies by requestor/property/ownership
generation, queue checked deletions and cancel on asynchronous errors or ownership
loss. No XWM requests or transfer endpoints are wired by this state machine.

Three tests passed (`/tmp/telorgon-incr-receiver.log`), covering partial consumption,
acknowledgement order, binary bytes, final deletion, type change, malformed lengths,
remaining property bytes, lower-bound checks, empty transfer, limits and timeout.
Reference: [ICCCM INCR properties](https://www.x.org/releases/X11R7.7/doc/xorg-docs/icccm/icccm.pdf).
Actual X11/native bridging, INCR sender and wire-level acceptance remain outstanding.

## INCR sender state machine

Added streaming byte-format INCR send flow control. The owner announces a zero
lower bound, queues SelectionNotify, supplies data only after property deletion,
and acknowledges each successfully queued ChangeProperty. Final nonempty data must
be deleted before the empty terminator is exposed; completion waits for that final
property's deletion. Per-chunk/total limits, timeout and cancellation bound state.
The owner must cap chunks to the negotiated X request size and cancel on wire errors
or ownership loss. Typed target conversion is outside this byte sender.

All five INCR tests passed (`/tmp/telorgon-incr-sender.log`), including a paired
sender/receiver state-machine roundtrip, early-data rejection, oversized chunks,
stall expiry and cancellation. This is not a real X11 transaction: requestor/property
routing, SelectionNotify, checked requests and endpoint wiring remain outstanding.

## INCR terminal outcome stability

Late sender/receiver callbacks now return errors without overwriting completed or
cancelled outcomes with Failed. Active protocol violations still fail and release
buffering. The regression exercises late deletion and data callbacks plus repeated
cancellation after both normal completion and explicit cancellation; terminal
states and absent deadlines remain unchanged. All six INCR tests passed
(`/tmp/telorgon-incr-terminal.log`). Generation-aware wire routing remains necessary
and is not replaced by this terminal-state guard.

## INCR cumulative limits and inactivity evidence

Added adversarial state tests showing that several individually valid chunks cannot
exceed either sender or receiver total-byte limits. A separate stalled-receiver test
shows consume(0) does not postpone expiry, positive partial consumption does, and
expiry clears remaining buffered bytes. All eight INCR tests passed
(`/tmp/telorgon-incr-limits.log`). Production behavior already satisfied these cases;
this closes a gap in state-level evidence, not the pending XWM wire integration.

## INCR regression checkpoint and typed properties

Before the typed-property extension, the serial compatibility library suite passed
1,123 tests with one ignored hardware test (`/tmp/telorgon-incr-full.log`). Native
and compatibility all-target checks passed (`/tmp/telorgon-incr-native.log` and
`/tmp/telorgon-incr-compat.log`), with the three existing dead-code warnings.

The sender now supports 8-, 16- and 32-bit properties, retaining the byte-format
constructor. Typed chunks must contain whole wire items; invalid formats and
chunks smaller than one item are rejected. Receivers expose validated atom/format
metadata for the eventual conversion adapter. Raw bytes are not converted.
All ten focused INCR tests passed after this change
(`/tmp/telorgon-incr-typed.log`), including typed roundtrips, partial downstream
consumption, terminator format, invalid formats and split-item rejection. These are
state tests; XWM wire integration and real clipboard qualification remain pending.

## INCR property request adapter

Added `incr_wire` helpers that serialize ChangeProperty and DeleteProperty through
the nonblocking transport and optional asynchronous request tracker. Endpoints are
scoped to the Xwayland generation. Writes validate the padded request length against
the setup limit, use item counts for typed properties, and preserve chunks when
queueing is rejected. Deletion requires the receiver's acknowledgement-ready state.
Returned request IDs are the selection owner's responsibility: barriers, error
handling, event routing, ownership-incarnation checks and SelectionNotify remain
unimplemented integration work. Queue success is not server confirmation.

Two local socket fixtures passed (`/tmp/telorgon-incr-wire.log`), checking serialized
typed data and empty terminators, padded-length rejection, stale generations,
partial-drain deletion rejection and saturated request accounting. The initial
sandbox run could not create socket pairs; the same tests passed with permission
for local fixtures. No X server, desktop or application was launched.

## INCR announcement serialization

Added the initial ChangeProperty helper with INCR type, format 32, one zero
lower-bound item, and a seven-unit minimum server request limit. Invalid type,
undersized request budget and repeated announcements are rejected before queueing;
the sender advances only after the request tracker accepts the write. SelectionNotify
must still be queued by the future selection owner, with cancellation on failure.
All three property adapter socket tests passed (`/tmp/telorgon-incr-announce.log`),
including exact header/item serialization and unchanged state/request count after
invalid announcement attempts. This does not establish a working selection bridge.

## Post-adapter regression checkpoint

After typed INCR support and the announcement/data/deletion wire helpers, the full
serial compatibility library suite passed 1,128 tests with one ignored hardware
test (`/tmp/telorgon-incr-wire-full.log`). Native and compatibility all-target checks
both passed (`/tmp/telorgon-incr-wire-native.log` and
`/tmp/telorgon-incr-wire-compat.log`), retaining the three existing dead-code warnings.
This checkpoint covers the library and compilation configurations, not a desktop
session or GPU/KMS presentation. SelectionNotify, bounded property reads, reply
barriers, ownership/event routing and native selection endpoint integration remain
necessary before W08 can claim an operational bridge. Hardware, application and
Horizon acceptance gates remain unexecuted.

## SelectionNotify request serialization

Added success/refusal notification serialization through the asynchronous request
tracker. The helper preserves requestor, selection, target and timestamp, sends
with an empty event mask, and reports refusal with property None. Success must name
the requested property, or an explicit nonzero property when the legacy request
specified None. Stale generations and mismatched success properties are rejected.
The selection owner still must validate ownership/time, order replies to equivalent
requests, confirm property writes and handle asynchronous errors.
Reference: [ICCCM selection responses](https://www.x.org/docs/ICCCM/icccm.pdf).

All four adapter socket tests passed (`/tmp/telorgon-selection-notify.log`). The new
fixture decodes success, refusal and legacy-property events, checks the destination,
empty mask and echoed fields, and verifies invalid requests consume no sequence
slots. No real selection bridge or desktop session was exercised.

## Bounded selection property reads

Added a non-deleting GetProperty assembler using one outstanding 16-KiB request
and at most 256 KiB of accumulated data. This keeps individual requested replies
below the transport's 256-KiB packet ceiling including the reply header. Subsequent
offsets are four-byte units. Replies validate item lengths, type/format consistency,
remaining byte counts, progress and total bounds. Wrong generation/sequence replies
do not consume the pending request. Cancellation and malformed replies discard the
accumulator; completion moves the assembled bytes into one GetPropertyReply.
Same-size concurrent rewrites are not detectable by these checks; selection-owner
handshake/event routing, deadlines and asynchronous-error cancellation remain required.

The compatibility library check passed (`/tmp/telorgon-property-read-check.log`).
All three focused tests passed (`/tmp/telorgon-property-read.log`), covering fragmented
assembly, stale identities, malformed/changing/oversized replies and actual request
bytes for offsets, read bounds and delete=false. The first test compile used an
unavailable generated request-parser method; the fixture now inspects wire fields
directly. No runtime clipboard support is claimed.

## Property-read completion dispatch

The bounded reader now accepts request-tracker completions directly. Only matching
generation/sequence IDs are consumed. Valid replies produce More or Complete;
events and unrelated IDs produce Unrelated. Matching errors, timeouts, unexpected
void acknowledgements and malformed replies terminate the read and release its
buffer. Wire lengths are checked before parsing. The owner must contain returned
errors, dispatch timeout completions and cancel on selection ownership loss.

All four focused reader tests passed (`/tmp/telorgon-property-completion.log`). New
coverage checks failure cleanup, unrelated and late completions, and two-fragment
wire-reply assembly. The fixture was corrected to include X11 four-byte reply
padding; the generated value serializer does not append that padding itself.
This is request-reader integration, not completed native/X11 selection routing.

## Property payload-length validation

A reproducer showed the generated GetProperty parser permits payload bytes beyond
the declared items. The outer packet-length check alone therefore accepted a reply
with eight payload bytes but one declared byte item. The completion adapter now
requires exactly the item bytes rounded to the four-byte wire boundary; malformed
replies terminate the read and release buffering.
The reproducer failed before the fix (`/tmp/telorgon-property-extra-before.log`),
and all five reader tests passed afterward (`/tmp/telorgon-property-extra-after.log`).
The existing fragmented three-byte final reply still passes with its legitimate
one-byte padding. This is protocol-reader evidence, not desktop qualification.

## Property reader transport integration evidence

Added a local socket fixture that queues a real GetProperty request, splits its
reply across socket writes, feeds framed packets through Requests::ingest and
delivers the resulting completion to PropertyRead. Partial wire data produces no
premature completion. The next fragment is deliberately unanswered; Requests::expire
then cancels the reader and releases accumulated bytes while allowing an unrelated
reader to queue. Request sequence accounting remains retained after expiry.
All six reader tests passed (`/tmp/telorgon-property-transport.log`). This exercises
the actual transport/request-reader chain, but no X server or clipboard endpoint.

## Post-property-reader regression checkpoint

The full serial compatibility library suite passed 1,135 tests with one ignored
hardware test (`/tmp/telorgon-property-full.log`). Native and compatibility
all-target checks passed (`/tmp/telorgon-property-native.log` and
`/tmp/telorgon-property-compat.log`), retaining the three existing dead-code warnings.
This includes the notification, fragmented-read, malformed-payload and timeout
fixtures. It does not prove an operational selection bridge: ownership generations,
event routing, checked-write barriers, native endpoints, PRIMARY and Xdnd remain
integration work. No desktop, GPU/KMS, application or Horizon qualification ran.

## Owned property handoff

Added Receiver::accept_owned_property with the same validation and state transitions
as the borrowed API. It moves the property payload into the receiver instead of
cloning it, avoiding a second maximum-sized allocation when handing off an assembled
PropertyRead result. The borrowed API remains compatible and explicitly copies.
All eleven INCR state tests passed (`/tmp/telorgon-incr-owned.log`); the new test
checks allocation identity for a 256-KiB payload, partial consumption and allocation
release on cancellation. The actual selection-owner handoff is still to be wired.

## Maximum-sized reader-to-INCR handoff

Added a component-chain test assembling sixteen 16-KiB fragments into a 256-KiB
property, moving that allocation into Receiver, checking every fragment's bytes,
and withholding deletion until the last downstream byte drains. Completion still
requires the empty terminator and its deletion acknowledgement. Another test
rejects cumulative growth beyond the property limit before retaining new bytes.
All eight reader tests passed (`/tmp/telorgon-property-handoff.log`). This proves
the component handoff at the configured bound, not an operational clipboard bridge.

## Selection ownership revisions

Added bounded ownership bookkeeping with independent clipboard and PRIMARY slots.
Each accepted content/source replacement receives a new revision scoped to the
Xwayland generation, even for the same owner identity. Replacements return the
previous snapshot; clear/clear_all invalidate tokens without allocating revisions.
Invalid owners and revision exhaustion preserve current state, while revocation
remains possible after exhaustion. Native IDs must be compositor-assigned; X11
selection owners need not have managed desktop windows.

Both state tests passed (`/tmp/telorgon-selection-ownership.log`), covering same-owner
replacement, PRIMARY independence, restart, invalid IDs, exhaustion and revocation.
The host must create one ledger per unique server generation and route returned
snapshots to transfer cancellation. Wire timestamps, proxy echoes, PRIMARY protocol
and endpoint integration remain outstanding; no selection is published by this code.

## Ownership-scoped transfer admission and cancellation

Connected the ownership ledger to Transfers admission: insert_owned rejects stale
tokens and records the full server-generation/selection/revision identity. Exact
ownership enumeration and cancellation close only matching endpoints. Legacy
opaque generation scopes remain disjoint, even when their integer matches an
ownership revision. The bridge must unregister the enumerated readiness sources
before cancellation and revoke all ownerships during server teardown.

The compatibility library check passed (`/tmp/telorgon-owned-transfers-check.log`).
All seven transfer tests passed (`/tmp/telorgon-owned-transfers.log`). The new socket
fixture replaces the same X11 clipboard owner, rejects stale admission with peer
EOF, cancels old endpoints, and retains PRIMARY, replacement and legacy-scoped
transfers. Desktop event-source and selection-event routing remain unimplemented.

## Selection server teardown

Added selection-server enumeration and cancellation independent of the live
ownership ledger. This includes superseded tokens awaiting cleanup and remains
usable after clear_all, while preserving other Xwayland generations and legacy
opaque scopes. Callers must unregister enumerated readiness sources first.
All eight transfer tests passed (`/tmp/telorgon-transfer-server-teardown.log`),
including peer EOF for old and replacement ownerships after server teardown,
idempotent cancellation and retention of another server's transfers. Host teardown
integration remains outstanding.

## Ownership/transfer regression checkpoint

The full serial compatibility library suite passed 1,142 tests with one ignored
hardware test (`/tmp/telorgon-selection-full.log`). Native and compatibility
all-target checks passed (`/tmp/telorgon-selection-native.log` and
`/tmp/telorgon-selection-compat.log`), with the three existing dead-code warnings.
This checkpoint includes owned payload handoff, ownership revisions, typed transfer
admission and server-scoped cancellation. It does not complete W08: selection-event
and endpoint routing, proxy loop prevention, timestamp policy, checked-write
barriers, PRIMARY protocol and Xdnd integration remain. Runtime desktop, hardware,
Steam/Wine and Horizon acceptance were not executed.

## Conditional ownership revocation

Added token-checked revocation for asynchronous selection failures/events. A stale
revision or server generation cannot clear a current replacement, while a matching
token returns the revoked snapshot for transfer cleanup. Repeated revocation is
idempotent; unconditional clear remains available for compositor policy.
All three ownership tests passed (`/tmp/telorgon-selection-revoke.log`), including
same-XID replacement, restart, PRIMARY isolation and repeated revocation. Wire
event-to-token routing remains to be implemented.

## Selection request timestamp intervals

Added a modular interval check for explicit server acquisition/current timestamps,
accepting interval endpoints and CurrentTime requests while rejecting requests
before acquisition or after the supplied current bound. Zero bounds and intervals
with ambiguous half-range ordering are rejected. The caller must establish an
interval shorter than half the server clock range and independently verify the live
ownership token; extended history cannot be inferred from wrapped timestamps alone.
This is a policy primitive, not acquisition verification or a server clock tracker.
Reference: [ICCCM selection owner responsibilities](https://www.x.org/docs/ICCCM/icccm.pdf).
All four ownership tests passed (`/tmp/telorgon-selection-time.log`), including clock
wrap, endpoints, CurrentTime, reversed/ambiguous bounds and zero-bound rejection.

## Selection policy regression checkpoint

After conditional revocation and timestamp interval checks, the full serial
compatibility library suite passed 1,144 tests with one ignored hardware test
(`/tmp/telorgon-selection-policy-full.log`). Native and compatibility all-target
checks passed (`/tmp/telorgon-selection-policy-native.log` and
`/tmp/telorgon-selection-policy-compat.log`), retaining three existing dead-code
warnings. Selection acquisition confirmation, server-clock tracking, proxy echo
prevention, event/endpoint routing and checked-write integration remain outstanding.
These checks do not establish runtime clipboard or PRIMARY support, nor any
hardware, application or Horizon acceptance result.

## Asynchronous selection acquisition confirmation

Added SetSelectionOwner/GetSelectionOwner acquisition state. Begin requires nonzero
owner, selection and server timestamp; queue_check can retry backpressure without
repeating the set request. Acquired requires a checked set and matching owner reply.
Mismatches reject the attempt; errors/timeouts fail it and late replies cannot
revive it. The owner must schedule checks/timeouts and independently handle later
SelectionClear events; no ledger or native selection is published automatically.

The library check passed (`/tmp/telorgon-acquisition-check.log`). Both focused socket
tests passed (`/tmp/telorgon-acquisition.log`), checking serialized set/query requests,
request-tracker checked completion, matching/different/absent owners and late replies
after errors/timeouts. Fixtures supply the protocol's 32-byte reply padding, which
the generated fixed-field serializer omits. Host integration remains outstanding.

Acquisition follow-up: backpressure retries preserve one set request; confirmed
metadata is available only after verification; cancellation invalidates pending or
confirmed attempts and ignores late replies. Four acquisition fixtures pass
(`/tmp/telorgon-acquisition-cancel.log`). The consolidated serial suite passed
1,148 tests with one ignored hardware test (`/tmp/telorgon-acquisition-full.log`),
and both all-target feature checks passed (`/tmp/telorgon-acquisition-native.log`,
`/tmp/telorgon-acquisition-compat.log`), with three existing dead-code warnings.
Wire ownership-loss routing, ledger publication and the desktop selection bridge
remain outstanding; no runtime or hardware qualification is implied.

Publication follow-up: confirmed acquisitions publish one native-source ledger
revision; cancellation/loss revokes only that token. Captured per-selection state
and retained replacement revisions reject delayed publication after replacement
or replace/clear races. SelectionClear fixtures cover pending/confirmed claims,
timestamp wrap, synthetic/stale events, late replies and malformed packet lengths.
The consolidated serial suite passed 1,152 tests with one ignored hardware test
(`/tmp/telorgon-publication-full.log`); native and compatibility all-target checks
passed (`/tmp/telorgon-publication-native.log`, `/tmp/telorgon-publication-compat.log`)
with three existing warnings. Host event routing, transfer cleanup scheduling,
proxy loop prevention and native selection endpoints remain unfinished.

Publication cleanup/echo follow-up: completion dispatch returns revocation alongside
errors; explicit cancellation synchronizes the ledger; old publications preserve
replacement clipboard and PRIMARY owners. Proxy notification classification separates
pending confirmation, current mirrored ownership, stale echoes and unrelated owners.
Actual XFixes subscription/event routing remains unimplemented. The full serial
suite passed 1,154 tests with one ignored hardware test (`/tmp/telorgon-proxy-full.log`);
both all-target checks passed (`/tmp/telorgon-proxy-native.log`,
`/tmp/telorgon-proxy-compat.log`) with three existing warnings. This remains component
evidence, not an operational clipboard bridge or runtime qualification.

XFixes follow-up: subscription requests cover owner changes, owner-window destruction
and owner-client closure, with unsubscribe support. Scoped event decoding rejects
synthetic, malformed and unknown-subtype packets. Proxy packet classification uses
the selection timestamp rather than the general event timestamp and checks server
generation/subscription window. The serial suite passed 1,157 tests with one ignored
hardware test (`/tmp/telorgon-xfixes-full.log`); both all-target feature checks passed
(`/tmp/telorgon-xfixes-native.log`, `/tmp/telorgon-xfixes-compat.log`), retaining three
existing warnings. Actual host subscription setup, event dispatch and clipboard
endpoint wiring remain outstanding.

Subscription-ledger follow-up: external owners create new revisions, owner=None
clears the selection, and the compositor's proxy is excluded. Subscription context
combines generation/atom/window filtering with ledger updates and preserves PRIMARY
when processing clipboard changes. The full serial suite passed 1,159 tests with
one ignored hardware test (`/tmp/telorgon-subscription-full.log`); both all-target
checks passed (`/tmp/telorgon-subscription-native.log`,
`/tmp/telorgon-subscription-compat.log`) with three existing warnings. Host dispatch,
native endpoints and real clipboard qualification remain unfinished.

XWM integration: watch_selection queues at most two distinct selection watches and
reply barriers. XWM dispatch now handles their checked completions/timeouts, exposes
subscription readiness, and contains setup failure to the compatibility driver.
The startup socket fixture verifies actual subscription bytes and readiness after
the barrier. All 143 Xwayland component tests passed
(`/tmp/telorgon-xwm-selection-all.log`). The shared discovery fixture now supplies a
valid XFixes event base; barrier replies use the existing padded reply helper.
The managed desktop still must supply selection atoms/proxy ownership and route
returned subscription contexts into its selection ledger and native endpoints.

The XWM startup fixture additionally verifies an actual XFixes owner packet reaches
Turn.events and updates the ledger through the returned subscription context.
The consolidated serial suite passed 1,159 tests with one ignored hardware test
(`/tmp/telorgon-xwm-watch-full.log`); both all-target checks passed
(`/tmp/telorgon-xwm-watch-native.log`, `/tmp/telorgon-xwm-watch-compat.log`) with three
existing warnings. This is socket-fixture integration, not managed desktop or real
clipboard qualification.

Selection subscription rejection regression: duplicate kinds, duplicate server
atoms across CLIPBOARD/PRIMARY, and zero atoms leave the existing XWM connection
and confirmed watch intact. The three focused XWM tests passed serially with
`desktop-xwayland` (offline); local Unix socket fixtures required execution outside
the socket-restricting sandbox. No compositor or Xwayland session was launched.
This does not qualify the still-unwired desktop selection bridge.

Selection atom discovery: all 143 Xwayland component tests passed serially with
`desktop-xwayland`, offline, after adding six selection atoms. The startup fixture
checks both PRIMARY and discovered CLIPBOARD watch requests and independent
readiness barriers. Mock-server sequence numbers were updated for the additional
InternAtom requests. These are socket fixtures, not a real clipboard session.

Connection-scoped selection atom access: three focused XWM tests passed serially
with `desktop-xwayland` offline. The startup fixture verifies PRIMARY/CLIPBOARD
decoding, unrelated INCR exclusion, stale-generation rejection, and removal of
atom access on teardown. No live selection transfer was executed.

ConvertSelection fixture passed with `desktop-xwayland`, offline and serial. It
checks the outgoing request fields, synthetic SelectionNotify matching, wrong
generation/target rejection, successful property access, owner refusal, timeout
after checked request, and cancellation. This is a mock-server test, not an
end-to-end clipboard transfer.

## Managed launch integration checks

Native and embedded-feature all-target cargo checks passed offline. The embedded
check used a **synthetic compile-only archive**, not a runnable/redistributable
Xwayland payload. The focused owner fixture passed: preparation failure and the
startup deadline preserve an unrelated live Wayland client and remove pending
compatibility work. Only the three pre-existing dead-code warnings remain in
those feature checks. No helper, desktop, GPU or GUI runtime was launched.

Runtime acceptance remains unrun: with a real validated embedded payload, start
the consuming compositor on an authorized test seat, verify the private client's
registry restriction and XWM initialization, then crash the owned helper while a
native window remains active. Rendering/input acceptance must follow after those
adapters are wired; setting DISPLAY manually does not establish that milestone.

Managed X11 presentation integration: 85 desktop state/socket tests passed
serially with desktop-xwayland; the native all-target check passed. The failure
fixture verifies hidden X11 roots/children, inherited child placement, and orphan
child hiding. Mapping/association has component coverage, but no actual pixels,
X11 input, DMA-BUF or GPU/KMS session was tested in this change.

Managed click-focus integration: the serial desktop-xwayland library suite passed
1161 tests with one ignored. The XWM startup socket fixture checks the outgoing
server-time marker, matching PropertyNotify timestamp, wrong-sequence/synthetic
rejection and command-barrier completion. This is compile/state/wire evidence,
not an executed X11 keyboard or pointer session.

Shutdown integration: the compatibility all-target check and three focused XWM
socket tests passed. The wire fixture chains a validated asynchronous server-time
marker into WM_DELETE_WINDOW and checks the timestamp and reply barrier. No live
application exit/save-dialog flow was executed. Session environment publication
and initial launch readiness remain the next integration gap.

Session X11 environment transition fixture passed: a captured subprocess receives
only the fixture endpoint plus preserved WAYLAND_DISPLAY; withdrawal removes the
endpoint for subsequent launches and prevents an inherited-display child's planned
failure retry while retaining its recovery entry. All 23 tests selected by the
session::tests:: filter passed serially (including text-session tests matched by
that filter). No Xwayland or GUI was launched.

Initial launch gating: 24 selected session/text-session tests passed, followed by
one additional deferred-recovery fixture. Coverage includes success, native
fallback, cancellation before spawn, PID transition, and journal recovery without
an original process identity. Native and embedded all-target checks passed; the
embedded check used the synthetic compile-only payload. No GUI/Xwayland server
or live initial application launch was executed.

Downstream integration check: `cargo check --manifest-path
/home/aku/CompositorStuff/test-compositor/Cargo.toml --features
telorgon/desktop-xwayland-embedded --offline --locked` passed. It used the
synthetic compile-only payload and a separate target directory; no executable was
run. The [manual smoke test](X11_SMOKE_TEST.md) records the consuming project's
verified bindings and installed xmessage options. Helper failure diagnostics now
include exit code/signal or supervisor errors without helper output contents.

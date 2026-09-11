# Telorgon Wayland Compositor Architecture

## Document role

This document describes the **current implementation** of Telorgon's Linux-only Wayland desktop
runtime and records the remaining qualification work. It is not a claim that every protocol in
`wayland-protocols` is implemented or that the compositor is production-qualified.

The implementation deliberately uses the official C ABIs and protocol XML. It does not use Winit,
Smithay, wlroots, an X11 compatibility host, or a Rust compositor framework.

Optional [X11 compatibility components](X11_COMPATIBILITY.md) now connect embedded
payload preparation, private helper supervision, dedicated Wayland-client access,
XWM readiness/dispatch, map/configure requests and committed serial association to
the managed loop. Preparation runs off the owner thread; helper/XWM descriptors
wake libwayland and errors stop only the compatibility instance. Associated X11 surfaces now enter the retained image/composition path with
XWM geometry and identity; unassociated/unmapped images and their subsurfaces
remain hidden. Ordinary pointer-click focus now uses asynchronous X-server timestamps and ICCCM
focus commands before granting Wayland keyboard focus. Broader focus policy remains
incomplete. DISPLAY/XAUTHORITY now enter future managed child environments after
server notification and XWM initialization; helper loss withdraws them and suppresses
incident retries. Initial managed launches wait asynchronously for startup completion or failure.
Shared activation-environment updates remain outstanding. The user has reported a working
xmessage display/input smoke test; this is not full X11 qualification.

The managed desktop uses a shared frame/policy model with a typed `WindowBackend`.
Frame construction, hit testing, move/resize geometry, maximize/restore and local
minimize reuse the native implementation. `window_backend.rs` dispatches focus
and close, while `x11_windows.rs` translates frame-content coordinates into bounded,
coalesced X11 configure commands. Only native windows use xdg configure/ack state.
X11 server notifications cannot overwrite an in-progress compositor drag.
Normal X11 windows receive the configured frame template; override-redirect
windows remain unmanaged. Title metadata is bounded and encoding-checked.
There is no Alt-drag special case. The shared resize-veil predicate now accepts
either native configure state or X11 resize-completion state; both use the same
RGBA solid layer, content clip and client-subtree/input suppression. X11 does not
resize the client for intermediate drag sizes. After release it waits for checked
server geometry and a newer published image matching the final size; a no-op can
reuse an already matching image. Superseding grabs discard old targets, measured
maximized chrome can restart the veil, and timeout is never treated as image
completion. The existing image/GPU retirement paths are unchanged. Full EWMH state/iconification, decoration hints,
X11 application icon metadata and the broader compatibility matrix remain separate work.

Reference review for this change reused Telorgon's existing frame, geometry and
XWM request lifetimes. The adjacent reference library is absent; no independent
graphics-source review is claimed and no GPU ownership or renderer implementation
changed. The protocol boundary follows the
[ICCCM](https://xorg.freedesktop.org/archive/current/doc/xorg-docs/icccm/icccm.html)
and the repository's xdg-shell protocol contract: X11 root/content geometry and
checked requests remain separate from xdg configure acknowledgements. Tests cover
frame-template parity, content/input offsets, stale geometry during dragging,
native transaction isolation, unmanaged windows, size hints and bounded titles.

## Public assembly

`Application::desktop_environment` is the only process entrypoint for this mode. Compositor-owned
pixels are normal Telorgon `Component` values:

```rust,ignore
Application::desktop_environment("Telorgon")
    .linux(LinuxDesktopConfig::default())
    .renderer(Renderer::Auto)
    .assets(assets::bundle())
    .app_icon(app_icons())
    .cursor_theme(assets::cursors::DEFAULT)
    .compositor(
        Compositor::new()
            .window_frame(easy_window_frame(MY_CHROME))
            .background(MyDesktopBackground),
    )
    .shell_widget(ShellWidget::new("panel").content(MyPanel))
    .run()?;
```

The frame template receives a fresh `WindowChromeModel` for each server-decorated toplevel and is
rendered at that window's outer extent. The model carries title, activation, state, capabilities,
tiling metadata, and application-icon metadata. `EasyWindowFrame` resolves a complete
`WindowChromeDesign` without a factory closure. A named `WindowFrameTemplate` implementation or a
legacy closure can instead compose a fully custom frame. Visuals are normal Telorgon composition using `BoxDecoration` plus
explicit title, icon, drag, resize, content-slot, and action roles. Final retained layout defines
the client-content offset and hit regions; no fixed control placement is imposed by the host.
`Compositor::background` is likewise a visual component behind clients, not a policy object.

The typed project asset bundle is shared by frames, normal shell composition, pointer themes, and
the desktop fallback `AppIconProfile`. Client-provided `xdg_toplevel_icon_v1` name/buffer snapshots
override that fallback. A permitted client-provided `wl_pointer` cursor surface overrides the
configured pointer theme, and a client-side xdg-decoration request suppresses Telorgon's frame.
See [Custom windows, assets, icons, and pointers](CUSTOM_WINDOWS_ASSETS_AND_POINTERS.md) for the
complete authoring API.

Output scaling now uses logical desktop units with automatic density selection at boot or
`OutputScale::Fixed(factor)`. See [Logical units and output scaling](LOGICAL_UNITS_AND_OUTPUT_SCALING.md)
for coordinate contracts, protocol announcements, configuration migration, and current limits.

`LinuxDesktopConfig` selects the DRM device, seat, optional Wayland socket name, output scale,
frame dimensions, and pointer extent. The umbrella exposes this mode through the
`desktop-wayland-linux` Cargo feature; it remains target plumbing rather than a renderer-selection
API or a second process entrypoint. The mode has no Windows or macOS implementation.
Top/right/bottom/left `ShellWidget::reserve_space` declarations reduce the maximized work area;
floating widgets remain overlays and fullscreen windows continue to use the complete output.

## Ownership and dependency layers

```text
official wayland.xml + wayland-protocols XML
                    |
                    v
telorgon-wayland-server
  build-time XML validation -> compiled wl_interface descriptors,
  libwayland-server display/global/resource/event-loop ABI
                    |
                    v
telorgon-compositor-wayland
  clients, resources, double-buffered surfaces, roles, xdg-shell,
  seats, outputs, SHM, DMA-BUF descriptors, explicit sync, events
              /                         \
             v                           v
telorgon-compositor-render          telorgon-platform-linux
  SHM -> Telorgon images            libseat, libinput, XKB, keymap FD
  DMA-BUF -> Vulkan leases                 |
             \                           /
              v                         v
             telorgon-app desktop_wayland owner thread
             Telorgon composition + scene/render orchestration
                              |
                              v
             telorgon-presenter-vulkan-kms
             libdrm atomic KMS + GBM scanout buffers
```

No layer above relies on external generated Rust protocol packages. The `telorgon` build script
parses official XML and emits immutable Rust schema and `wl_interface`/`wl_message` tables.
`wayland_server` compiles those tables into the library and passes decoded requests to Telorgon-owned
state. `libwayland-server` remains the mature transport, resource, client, socket, and event-loop
implementation.

Within the managed host, `desktop_wayland.rs` is the single-owner orchestration loop. Its sibling
modules isolate client publication, cursor-plane/KMS lifetime tracking, event sources and input
profiling, geometry and damage math, pointer routing and hit testing, resize transactions, composed
layer preparation, pointer visuals, renderer-neutral desktop-scene synchronization, bounded
full-SHM copying, and separate Vulkan/software renderer assemblies. These boundaries are internal
and do not create additional owners. Two
explicit blocking operations are isolated: the Vulkan completion waiter and a single bounded SHM
copy worker. The latter receives only duplicated FDs plus immutable commit metadata; it never
accesses Wayland objects or compositor state.

## Protocol source and advertisement rules

The `desktop-wayland-linux` feature on a Linux target requires protocol XML **at build time only**.
`crates/telorgon/build.rs` reads `/usr/share/wayland/wayland.xml` and the 14 extension paths under
`/usr/share/wayland-protocols` listed in `wayland_server/protocol.rs`. Install the Wayland development
data and `wayland-protocols` packages with the interface versions required by that profile (including
xdg-shell v7, cursor-shape v2, presentation-time v2, and linux-dmabuf v5). Package names vary by distro.
The existing native link dependencies, including `libwayland-server`, remain required.

For custom installations and cross compilation, set build environment variables:

```sh
TELORGON_WAYLAND_XML=/path/to/wayland.xml \
TELORGON_WAYLAND_PROTOCOLS_DIR=/path/to/wayland-protocols \
cargo build -p telorgon --no-default-features --features desktop-wayland-linux
```

These are host-readable build inputs; select definitions compatible with the target profile.
Other features and non-Linux targets do not open or require these files. The optional `roxmltree`
build dependency parses XML; no scanner executable or C compilation is required for generation.
The official `wayland-scanner` is used only by an explicitly selected reference-comparison test.
Cargo watches each selected XML file, both override variables, the build modules, the Rust profile,
and the checked-in wire contract. Changes, removals, or path overrides cause generation to rerun.
The generator and wire contract are inside the published Cargo package.

Generation rejects malformed/misnested XML, invalid names and numeric ranges, excessive source or
collection sizes (including libwayland’s 20 wire-argument limit), missing/old profile interfaces, duplicate interfaces/messages/arguments, invalid
argument types/nullability/interface annotations, invalid message versions, and unresolved typed
references. `build/protocol-wire-contract.txt` pins request/event opcodes, names, signatures,
destructor flags, and argument interface names through each profile source version. Future versions
may append messages without changing this ABI. Intentional profile changes require an explicit
review of this contract; builds never silently regenerate the checked-in baseline. Documentation and
enumeration values are not dispatcher descriptor inputs and are not part of this compatibility check.

`NativeProtocol::desktop()` returns a zero-allocation handle. Rust metadata uses static strings and
slices; native tables, C strings, and per-wire-argument type pointers are immutable statics with
process lifetime. Only private generated-data wrappers implement `Sync`, leaving arbitrary FFI
values untouched. There is no lazy initialization, pointer patching, XML parsing, or `/usr/share`
protocol access at runtime. The old `ProtocolCatalog`, `ProtocolSourcePaths`, parsing API, and native
allocation/error API have been removed; `NativeCompositor::new` no longer takes a catalog.

The cursor-shape `get_tablet_tool_v2.tablet_tool` argument retains its previous opaque/null native
type entry because Telorgon does not load or advertise tablet protocols and rejects that request.
This exact exception is checked by the generator; other unresolved interface references fail the
build. Generic `new_id` expands to the three wire arguments `string, uint, new_id`, correcting the
old unused `wl_registry.bind` metadata from `un` to the official `usun`. Registry dispatch remains
owned by libwayland; Telorgon's supported requests and advertised versions are unchanged.

Compiling a protocol descriptor does **not** advertise its globals. A global is created only where
`NativeCompositor` has a dispatcher. Request and event `since` versions are checked against each
resource's negotiated version. The machine-readable profile is
[`protocols/telorgon-wayland-profile.toml`](../protocols/telorgon-wayland-profile.toml).

| Global | Maximum advertised | Availability |
| --- | ---: | --- |
| `wl_compositor` | 6 | Always |
| `wl_shm` | 1 | Always; ARGB8888/XRGB8888 plus accepted Telorgon formats |
| `wl_subcompositor` | 1 | Always |
| `wl_data_device_manager` | 3 | Always; selection plus pointer/touch drag-and-drop with MIME FD and action negotiation |
| `xdg_wm_base` | 7 | Always |
| `zxdg_decoration_manager_v1` | 1 | Always |
| `wp_cursor_shape_manager_v1` | 1 | Always |
| `xdg_toplevel_icon_manager_v1` | 1 | Always; commit-synchronized named or square SHM icon snapshots |
| `wp_fractional_scale_manager_v1` | 1 | Always |
| `wp_viewporter` | 1 | Always; commit-synchronized source crop and destination scale |
| `wp_presentation` | 2 | Always; commit-scoped monotonic feedback after KMS commit |
| `xdg_activation_v1` | 1 | Always; fresh-input-authorized, opaque, one-shot tokens |
| `ext_session_lock_manager_v1` | 1 | Always; blank-first secure KMS transition and input isolation |
| `zwp_relative_pointer_manager_v1` | 1 | Always; focused accelerated and unaccelerated deltas |
| `zwp_pointer_constraints_v1` | 1 | Always; focused pointer lock and region confinement |
| `zwp_idle_inhibit_manager_v1` | 1 | Always; scoped inhibitors exposed to shell power policy |
| `wl_output` | 4 | Per configured KMS output |
| `wl_seat` | 9 | Per configured libseat seat |
| `zwp_linux_dmabuf_v1` | 3 | Only after exact Vulkan importable format/modifier tuples are supplied |
| `zwp_linux_explicit_synchronization_v1` | 2 | Only when the selected render/present path accepts acquire and release fences |

Linux-dmabuf feedback v4/v5 and DRM syncobj are not advertised. Their XML metadata is retained as
an implementation roadmap, not as a support claim.

## Surface and shell behavior

The compositor enforces per-client object limits and ownership, permanent surface roles, xdg
configure/ack ordering, buffer-before-configure rejection, lossless retention of every
unacknowledged configure, SHM pool bounds, DMA-BUF plane/tuple validation, and client-scoped
single-use input serials. Interactive resize separately coalesces raw pointer motion to the latest
scheduled state before it enters that protocol queue and budgets resizing configures to one per
presented frame; a newer acknowledged serial validly supersedes older queued configures. Surface state is
double-buffered. Damage, opaque/input regions, scale, transform, offset, frame callbacks, buffer
attachments, and subsurface relationships flow through the commit model.

Implemented shell roles are xdg-toplevel, xdg-popup, wl-subsurface, session-lock, drag icon, and
pointer cursor. Toplevel icons accept bounded square SHM buffers and optional desktop icon names,
become immutable when assigned, and latch to the target surface's next commit. Popups retain
positioner anchor, gravity, offset, and constraint flags and receive
configure/repositioned events. Synchronized subsurfaces cache their state until the parent commit.
Server-side decoration is the default; explicit client-side negotiation is respected. The managed
policy loop maintains explicit stacking, keyboard activation state, titlebar and client-requested
move grabs, border/corner and client-requested resize grabs, maximize/restore, fullscreen/restore,
minimize, and close events. Multi-workspace policy and output-aware placement remain shell policy
extensions rather than protocol-server concerns.

Frame callbacks and presentation-time feedback are associated with commit revisions and emitted
only through the image revision included in a successful KMS commit, rather than at
`wl_surface.commit`. A presented revision completes frame callbacks from superseded commits while
superseded presentation feedback is discarded. Presentation feedback uses `CLOCK_MONOTONIC`; the
blocking presenter reports no hardware timing flags until page-flip timestamp proof is integrated.
Copied SHM buffers are released after the compositor has taken its copy. Explicit-sync acquire FDs
are attached to a specific surface revision and release objects are completed only by the render
path that consumed that revision.

## Input, output, and session

`telorgon-platform-linux` owns narrow bindings to libseat, libinput, and xkbcommon. The runtime opens
the requested seat, obtains the DRM FD through libseat, publishes one `wl_seat`, creates an
NUL-terminated XKB keymap in a memfd, and delivers pointer motion/focus/buttons/axes plus keyboard
focus/keys/modifiers and slot-stable touch down/motion/up/frame/cancel streams. Newly mapped normal
toplevels receive keyboard focus, and keyboard delivery follows that focus independently from
pointer hover. Pointer and touch coordinates are transformed into surface-local coordinates.

Pointer button routing retains physical presses separately from their recipients. A client press
keeps an implicit grab until its final button release; decoration and unfocused presses cannot
deliver a release into a newly entered client. Explicit move/resize grabs, focus replacement,
surface/client destruction, and session locking cancel client delivery while retaining physical
state until release. Session locking also cancels armed frame controls. Pointer leave events are
terminated with a frame, including when no new surface receives focus.

Client hover focus and decoration cursor selection share `WindowChromeSnapshot::hit_test_content`.
Published chrome targets take precedence over the rectangular content slot, including rounded
corner resize bands that overlap that slot. Crossing from a corner handle into content therefore
produces a fresh client enter instead of leaving a compositor cursor installed under unchanged
client focus. The window-frame regression walks inward in subpixel steps at every corner (with
and without a title bar where content is adjacent) and also checks the straight-edge transitions.

Applications can declare global desktop shortcuts with named functions:

```rust
use telorgon::app::{Compositor, KeyBindings, KeyChord, ShortcutKey};

fn open_launcher() { /* Request the application's launcher. */ }
fn open_terminal() { /* Request the application's terminal. */ }

let shortcuts = KeyBindings::new()
    .bind(KeyChord::new(ShortcutKey::Q).control().shift(), telorgon::request_exit)
    .bind(KeyChord::new(ShortcutKey::Space).super_key(), open_launcher)
    .bind(KeyChord::new(ShortcutKey::T).control(), open_terminal);
let compositor = Compositor::new().keybindings(shortcuts);
```

`KeyBindings` stores plain `fn()` pointers; it requires no action enum or handler trait.
`KeyChord` supports `.control()`, `.shift()`, `.alt()`, and `.super_key()` and requires exact
modifier matching. `ShortcutKey::A`–`Z` match either ASCII case, including when Caps Lock changes
the resolved case. For example, `KeyChord::new(ShortcutKey::Q).control().shift()` binds Ctrl+Shift+Q;
without `.shift()` it binds Ctrl+Q. `Digit0`–`Digit9`, `F1`–`F12`, and the existing `Space`, `Enter`,
`Escape`, `Tab`, and `Backspace` constants match exact symbols. Digits refer to layout-resolved
digits, not physical number-row positions; shifted punctuation needs its own symbol binding.
`ShortcutKey::ascii('q')` and `ShortcutKey::from_keysym` retain exact, case-sensitive matching.
Symbol constants follow the
[XKB keysym definitions](https://github.com/xkbcommon/libxkbcommon/blob/master/include/xkbcommon/xkbcommon-keysyms.h).
These are layout-resolved symbols, so this adapter does not reinterpret them as physical chords
for the separate neutral `ShortcutMatcher`. Overlapping chords (including named letters paired
with exact upper/lowercase symbols under the same modifiers) panic
during binding construction. Matched functions run once per fresh press and consume the key
through release; unmatched keys forward. Functions must remain short and nonblocking.
The adapter reuses the existing host capture and session-lock routing. Calling `.keybindings()`
or `.keyboard_shortcut_handler()` replaces the previous shortcut configuration: the last call wins.
These function bindings have no captured application state or return disposition. To quit cleanly,
bind `telorgon::request_exit` directly or call it from a named handler. It wakes running managed GUI
and compositor hosts and requests normal event-loop shutdown after the callback returns. It is
thread-safe, coalesces repeated requests, and does nothing when no host is running. For custom key
forwarding behavior, use the raw handler below.

Applications may also register `Compositor::keyboard_shortcut_handler` for global desktop shortcuts.
The callback receives a fresh `DesktopKeyEvent` before client delivery, with its evdev code,
XKB symbol, and effective Control/Shift/Alt/Logo modifiers. `Forward` preserves normal delivery;
`Consume` reserves that key's press, repeats, and release; `Quit` returns normally from the host.
Modifier events continue updating XKB and the Wayland seat even for consumed keys. The handler is
not called while a session lock is active and must remain short and nonblocking. Shortcut actions
and process launching belong to the application, not the framework. Effective modifier lookup
uses [XKB's named modifier API](https://xkbcommon.org/doc/current/group__state.html).
Portable routing tests cover matched and ordinary keys, repeats, modifier changes before release,
locked-session isolation, and quit propagation. Live seat/display qualification remains manual.

Both cursor requests authorize against the focused client and its current pointer-enter serial,
independently of the bounded general serial ledger. Nonmatching requests are ignored before cursor
state or surface roles change; invalid cursor shapes and role conflicts remain protocol errors.
This follows the [core pointer protocol](https://wayland.freedesktop.org/docs/html/apa.html#protocol-spec-wl_pointer)
and [cursor-shape protocol](https://wayland.app/protocols/cursor-shape-v1).

The input ownership review inspected `../other-rendering-libs/slint/internal/core/input.rs`
(`handle_mouse_grab`) and `../other-rendering-libs/xilem/masonry_core/src/passes/event.rs`
(`get_pointer_target` and pointer-up/cancel handling). Their shared invariants are that capture
precedes hover targeting, invalid owners lose capture, and release/cancellation terminates an
interaction. Telorgon implements these rules with seat-owned button recipients, without copying
reference code. Filtering only unmatched releases in Foot or accepting every cursor serial was
rejected: neither establishes compositor-side ownership. Portable seat tests cover the captured
166/167 cursor mismatch, serial-history eviction, decoration reentry, multiple buttons, duplicate
edges, destruction, focus replacement, explicit grabs, and session cancellation. Linux host wiring
is compile-checked; interactive compositor qualification remains a user-run check.

Relative-pointer and pointer-constraints v1 are native-dispatched. A constraint is unique per
pointer/surface pair, follows pointer focus, implements persistent and one-shot lifetimes, and
emits the required locked/unlocked or confined/unconfined transitions. Locked pointers suppress
absolute motion while continuing relative deltas; confined pointers are clipped to the nearest
point in the surface-local constraint region.

XDG activation v1 issues cryptographically opaque, bounded, one-shot tokens. Tokens backed by a
fresh client-scoped input/focus serial are authorized; tokens without valid interaction metadata
are intentionally ineffective, as allowed by the protocol. Successful activation raises the
target in Telorgon's explicit stacking order and transfers keyboard focus. Tokens may be passed
between clients and unknown or consumed handles are ignored.

Session-lock v1 assigns permanent lock-surface roles, configures each output at its exact logical
size, rejects pre-configure/null/wrong-size commits, and isolates normal surfaces from both input
and rendering. A lock request first switches composition to opaque black plus lock surfaces. The
`locked` event is emitted only after that frame successfully completes Vulkan/software scanout and
the blocking atomic KMS commit. If the lock client dies after that event, Telorgon remains locked on
black; a later lock client may take responsibility for recovery. Unlock is accepted only from the
active lock object after `locked` was sent.

Core data-device v3 is native-dispatched. Only the keyboard-focused client may install a selection
using a fresh input serial; offers carry bounded validated MIME types, access is revoked on focus
loss, and `receive` forwards the supplied FD to the source. Replaced sources are cancelled.
Pointer- and touch-origin drag grabs validate the initiating serial, assign and render the drag-icon
role, hit-test client targets, issue per-data-device offers, negotiate v3 copy/move/ask actions,
preserve legacy copy behavior, and deliver enter/motion/leave/drop plus source completion or
cancellation. Entering secure session-lock mode cancels any active drag before isolating input.

The managed path currently publishes one connected KMS output and advertises pointer, keyboard, and
touch. Multi-output layout, hotplug, seat disable/re-enable KMS reconstruction, repeat scheduling,
input-method support, and touch-shape/orientation metadata remain qualification gaps.

Compose icon declarations are semantic and active. `window.close`, `window.maximize`, and
`window.minimize` are rendered into the matching server-decoration controls and define their hit
targets. Cursor-shape requests resolve through `cursor.default`, `cursor.pointer`, `cursor.text`,
`cursor.grab`, the resize-direction names, and the other CSS-compatible cursor semantic names; an
unconfigured shape falls back to the compositor's composed pointer component. The entire frame,
control, pointer, and cursor-shape visual path therefore remains ordinary Telorgon composition.

## Rendering and presentation

Primary scanout uses [startup negotiation](LINUX_SCANOUT_NEGOTIATION.md): Vulkan
selects a matching DRM adapter and a jointly supported explicit layout; software
uses CPU-mappable GBM or DRM dumb buffers. Both paths validate a complete buffer
pool with KMS before selection, and Auto rebuilds a separate software pool after
a compatible Vulkan startup failure. Linux hardware qualification remains open.

The operational managed path is entirely Telorgon-rendered:

1. The compositor copies a committed SHM buffer with checked offset/stride/extent arithmetic. Once
   a direct, same-size surface has a retained image, `wl_surface` damage limits positional I/O to
   tightly packed damaged rows. New images, metadata changes, transformed/viewported surfaces, and
   full-surface damage take a bounded FIFO worker path so full pixel I/O and conversion do not stall
   the input/protocol owner. That worker also prepares independent client-retained and scene-owned
   snapshots, so accepting a completed full image does not perform another whole-buffer copy on the
   owner. Each surface has at most one submitted full copy and one replaceable latest deferred
   copy. Draining free worker capacity skips deferred surfaces whose earlier copy is still in
   flight, while continuing past them so unrelated surfaces can make progress. Superseded deferred
   revisions are retired immediately, while `wl_buffer` release remains delayed until every
   submitted read of that buffer is done. Different surfaces can still occupy the bounded worker
   queue concurrently.
2. `telorgon-compositor-render` preserves little-endian ARGB/XRGB as native BGRA and ABGR/XBGR as
   native RGBA, with explicit alpha/color metadata; only RGB565 and geometry transformations need
   pixel conversion. Buffer transform/scale and viewporter crop/destination use bounded
   deterministic sampling.
3. Client images, the composed background, composed server frames, shell widgets, popups, drag
   icons, and a composited cursor become a renderer-neutral frame containing retained-scene deltas,
   ordered placements, clips, and output damage. This layer contains no backend scene, pixel
   surface, rasterizer, or Vulkan handle. Scene identity is separate from placement identity, so one
   retained control can appear in several windows while movement still damages the correct old and
   new bounds. Hidden/minimized producers keep their retained identity without contributing a draw,
   and a hidden client revision is not consumed before its queued pixels are delivered. Focus-state
   changes reconcile the existing window-frame component root instead of recreating its runtime. A
   client resize uses an RGBA solid-color veil (opaque by default) instead of stretching stale content.
   Easy-frame designs override preview color and independently set normal content backing. Frame fill
   and shadow are excluded from the content rectangle by non-overlapping clipped placements; normal
   backing draws separately, and neither it nor stale client pixels draw beneath a translucent veil.
   Client alpha can reveal lower layers when the backing is transparent; opaque buffers stay opaque.
   Content and subsurfaces intersect the frame's inset rounded-border contour with the content slot;
   popups remain independent. Both contours start at the window top, not the app/title-bar seam.
   Separate outline and inverse-clipped frame-fill placements restore the curved rim and wider
   corner wedges inside the rectangular cutout. Rounded-clip changes damage placements without rewriting client images. Vulkan
   applies analytic coverage through per-placement view uniforms; software uses the same distance
   rule. Composed easy-frame descendants clip at their overflow bounds, excluding the frame's shadow.
   Easy-frame content bounds derive from title-bar height and border width, without extra content
   margins or independent aperture radii. The same inset contour excludes client/title pixels from
   rounded resize targets; wider grab tolerance extends outward and is tested before the host rejects
   outside-window points. Only resize actions may activate outside the window. Custom templates can
   still define separate aperture rounding and rectangular hit regions.
   All eight edges track the pointer using compositor-owned frame/veil geometry;
   the client receives its current committed extent with the initial resizing hint and its final
   requested extent on release, not a stream of intermediate sizes. The client surface tree is hidden
   and new SHM copies are paused behind a bounded latest-wins mailbox during the drag. Work submitted
   before the drag may finish. After release, copies and frame callbacks resume; the veil remains
   until the applying client publication acknowledges the final configure. Hidden content is never
   reported as presented. Client images use native surface-coordinate sizes with commit-latched XDG
   geometry offsets and clipping; client-side shadow margins stay out of resize-size, hit-test, and
   fixed-edge calculations. A final
   configure acknowledgement is captured before asynchronous image work can be superseded. The
   applying publication retires the resize transaction at the client's committed window extent;
   cell- and aspect-constrained clients may legally choose an extent below the configure maximum.
   See [solid resize preview](WAYLAND_RESIZE_PREVIEW.md) for configuration, buffer ownership,
   callback pacing, reference audit, and remaining hardware qualification.
4. Backend selection happens once in the desktop renderer assembly. The selected implementation
   owns its scene map and output state for the remainder of the run; neither backend calls the
   other. Vulkan applies deltas to `VulkanScene`, stages changed rows and per-placement uniforms,
   and records all ordered placements directly into the imported GBM target in one dynamic render
   pass, command buffer, and submission. There is no `SoftwareScene`, software raster, intermediate
   layer surface, CPU-flattened desktop, or full-screen texture upload in that path. Software applies
   the same neutral deltas to `SoftwareScene` and rasterizes every placement directly into one
   retained output framebuffer, clearing and copying only accumulated output damage; it creates no
   Vulkan object. CPU work that remains in Vulkan mode is protocol ingestion—bounds checking,
   optional Wayland SHM format/geometry conversion, and construction of staging bytes—not rendering.
   The composed pointer uses three
   completion-retired ARGB8888 GBM cursor buffers when an atomic cursor plane is available;
   otherwise it remains an ordinary retained desktop-scene layer.
5. Three linear GBM scanout buffers receive the primary output. Primary and cursor state are
   submitted through one serialized libdrm atomic-commit scheduler to a connector-compatible CRTC.
   Cursor-only motion coalesces to the newest position and commits without repainting the primary
   plane; it does not use the legacy cursor ioctls or asynchronous page-flip flag.

`telorgon-compositor-render::DmaBufImporter` is the Vulkan client-buffer import bridge. It exposes
only exact single-plane format/modifier tuples queried from the selected `VulkanDevice`, validates
allocation bounds, consumes an acquire sync FD, creates a generation-scoped
external-image lease, and binds it into `VulkanScene`. The external-image path can export the
matching release requirement.

The managed KMS host advertises those tuples only when the owned Vulkan device also supports the
complete sync-FD contract. A committed client DMA-BUF is sampled once into a compositor-owned
retained Vulkan texture in the same submission as desktop composition. That submission waits on
the acquire fence, signals and exports the per-commit release fence, and keeps `wl_buffer` busy until
GPU completion. If the commit supplies a protocol explicit-sync fence, the host consumes it. Because
`linux-dmabuf` otherwise uses implicit synchronization, the host snapshots the DMA-BUF reservation
object's writer fences with `DMA_BUF_IOCTL_EXPORT_SYNC_FILE(DMA_BUF_SYNC_READ)` and imports that sync
file into the same Vulkan wait path. Subsequent scanout-buffer updates sample only the retained
texture, so the linear client lease is never reused and no client pixels cross the CPU.

The managed KMS path also has an owned Vulkan scanout route. All three primary GBM buffers are
imported with their explicit modifier and row layout as Vulkan color targets. Telorgon renders the
ordered retained-scene placements into the selected target, transfers queue ownership back to
`VK_QUEUE_FAMILY_FOREIGN_EXT`, waits for GPU completion, and only then makes that frame eligible for
the serialized atomic KMS scheduler. `Renderer::Vulkan` requires this route and fails startup if it
cannot be created. `Renderer::Auto` attempts it before any frame is rendered and otherwise constructs
the separate mapped software assembly; it never switches or combines backends mid-run.
`Renderer::Software` constructs only the mapped software route. SHM content is copied into
backend-owned retained image resources. Vulkan additionally accepts the capability-gated DMA-BUF
materialization route described above; direct long-lived sampling of client-owned buffers is not
used because one client generation may need to update several retained scanout targets.

The GBM/KMS bindings are Telorgon-owned and use the original libgbm/libdrm ABIs. Scanout allocation,
mapping, modifier-aware framebuffer creation, connector/encoder/CRTC/plane discovery, primary and
cursor-plane filtering, mode blobs, atomic test commits, initial modesets, nonblocking page-flip
events, primary/cursor buffer retirement, and mailbox scheduling are implemented. Direct scanout,
general overlays, color management, VRR, HDR, and hardware qualification are still open.

## FD, synchronization, and lifetime invariants

- Incoming protocol FDs become `OwnedFd` immediately; duplicated FDs have one documented owner.
- SHM reads use positional I/O and never mutate a client's shared file offset.
- The SHM worker owns only duplicated files and immutable snapshots. Wayland state/application and
  `wl_buffer.release` remain owner-thread operations; the submitted queue and per-surface latest
  mailbox have explicit hard bounds. Full client and scene snapshots are prepared on that worker
  and transferred to the owner without another whole-image copy.
- A direct SHM regional update carries tightly packed native-order rows through the desktop scene;
  a renderer may not reinterpret damage as permission to discard pixels outside that rectangle.
- Renderer-neutral scene identity is distinct from placement identity. Removing a placement may not
  discard a still-live shared scene, and a hidden revision may not advance image content until its
  queued pixel update is consumed.
- Vulkan image replacement while an older submission is in flight preserves the old contents with
  a transfer-source/transfer-destination image copy before applying regional staging bytes. Both
  images remain completion-pinned, and only an unpinned retired image may re-enter the pool.
- A DMA-BUF buffer owns all plane FDs until duplicated into a generation-scoped renderer lease.
  The owned renderer consumes that lease exactly once to populate a retained texture, exports its
  release sync FD after submission, and delays core buffer release until completion.
- Absence of a protocol explicit-sync fence is not proof that a DMA-BUF is ready. The default
  implicit writer fences are exported as a read sync file before Vulkan samples the buffer.
- Explicit acquire/release state is keyed by `(surface, commit revision)`, not merely by surface.
- Vulkan explicit modifier imports use `VkSubresourceLayout::size == 0`, as required by the Vulkan
  specification, while allocation size is retained separately for bounds checking.
- A GBM buffer outlives its KMS framebuffer, and both outlive the atomic request that references it.
- A cursor buffer is never mapped while it is current or named by the one pending CRTC commit. A
  composited fallback retains an active cursor framebuffer until an atomic plane-disable completion.
- Only one nonblocking atomic commit is outstanding per CRTC. Pointer events received while it is
  outstanding replace desired cursor position rather than enqueueing additional commits.
- A libseat-managed DRM FD remains owned by the seat; KMS receives a duplicate.
- Connected Wayland clients and their resources are destroyed while native protocol state is
  still alive; globals are then destroyed before bind contexts and protocol descriptors.
- No callback is allowed to unwind across a C ABI boundary.

## Source audit

The implementation is based on the following primary specifications and source audits:

- [Wayland server API](https://wayland.freedesktop.org/docs/html/apc.html), [wire/XML rules](https://wayland.freedesktop.org/docs/book/Message_XML.html), and [core protocol](https://wayland.freedesktop.org/docs/html/apa.html) define transport/resource and core object behavior.
- [xdg-shell](https://wayland.app/protocols/xdg-shell), [xdg-decoration](https://wayland.app/protocols/xdg-decoration-unstable-v1), [xdg-toplevel-icon](https://wayland.app/protocols/xdg-toplevel-icon-v1), [xdg-activation](https://wayland.app/protocols/xdg-activation-v1), [session-lock](https://wayland.app/protocols/ext-session-lock-v1), [cursor-shape](https://wayland.app/protocols/cursor-shape-v1), [linux-dmabuf](https://wayland.app/protocols/linux-dmabuf-v1), and [explicit synchronization](https://wayland.app/protocols/linux-explicit-synchronization-unstable-v1) define the implemented extension contracts.
- The Linux kernel [DMA-BUF synchronization documentation](https://docs.kernel.org/driver-api/dma-buf.html) defines reservation-fence export as a sync file for explicit APIs such as Vulkan.
- [DRM KMS documentation](https://docs.kernel.org/gpu/drm-kms.html) and the official libdrm [`xf86drmMode.h`](https://cgit.freedesktop.org/drm/libdrm/tree/xf86drmMode.h) define atomic presentation and exact ABI layouts. The source audit caught the legacy coordinate fields that precede `possible_crtcs` in `drmModePlane`.
- The [Vulkan explicit DRM modifier structure](https://registry.khronos.org/vulkan/specs/latest/man/html/VkImageDrmFormatModifierExplicitCreateInfoEXT.html) requires each explicit plane layout's `size` to be zero.
- wgpu commit `d99c241a3b9dcc0f6674d990d007d79e94d39862` was inspected for DMA-BUF import capability and ownership invariants; Flutter commit `51fd9afadf309ba5337320bd3653f5345c156cb9` was inspected for sync-FD ownership and frame-slot reuse. Those projects are references only; no framework code or abstraction was copied.

### DMA-BUF acquire-synchronization audit

The concern was diagonal, triangle-shaped corruption during aggressive interactive resize. KMS frame
eligibility already follows compositor Vulkan completion, and scanout slots remain pinned through
page-flip retirement. The missing ordering was earlier: commits without the legacy explicit-sync
extension entered Vulkan without waiting for the client producer recorded in the DMA-BUF reservation
object. That allowed a retained-texture copy to observe a partially rendered client frame.

The `linux-dmabuf` contract was checked for its default implicit synchronization rule and the kernel
DMA-BUF contract for `DMA_BUF_IOCTL_EXPORT_SYNC_FILE`. wgpu's Vulkan DMA-BUF import was inspected to
confirm that image-memory import does not itself wait for a producer, while Flutter's Vulkan sync-FD
path was inspected for temporary binary-semaphore import and successful-import FD ownership. The
Telorgon decision is to prefer a revision-scoped protocol acquire fence and otherwise export the
single supported plane's implicit write fences for a read operation. Both sources feed the existing
one-shot Vulkan semaphore wait. Export failure rejects publication rather than presenting an image
whose producer completion is unknown. The ABI layout is compile-time checked, and the full Wayland
feature graph is compile-checked for both supported Linux architectures; hardware resize remains a
manual qualification step. No reference source was copied.

### Direct retained-composition audit

The concern was focus-triggered whole-frame work and the accidental use of software-rasterized layer
surfaces as Vulkan inputs. Telorgon's component, render-scene, software, Vulkan, and Wayland
ownership documents were read first. Slint commit
`69ecb713f5c62d1b6fe986ff822a57f22152b4d9` was inspected at
`internal/core/window.rs::draw_contents` and `internal/renderers/anyrender/lib.rs` for walking
multiple component scenes through one selected renderer. Egui commit
`fd54387eac03f57ca772a8fb590ceaadf780f31c` was inspected at
`crates/egui-wgpu/src/renderer.rs::render` and `epaint/src/textures.rs::TexturesDelta` for ordered
clipped draws in an existing render pass, retained texture identity, and partial writes. Qt commit
`3e2d6bd456a8e850bcf641de77d1d5d8bc8419ef` was inspected at
`src/quick/scenegraph/coreapi/qsgrendernode.cpp` and the batch renderer's
prepare/begin/record/end-pass flow for explicit render-state ownership. Xilem/Masonry commit
`ce7b04d2ba2d9d7a8c364f2ab109e2083121e144` was inspected at
`xilem_core/src/views/any_view.rs::dyn_rebuild` and
`masonry/src/properties/content_color.rs` for same-type in-place reconciliation and property-scoped
invalidations. No reference code or abstraction was copied.

The [Wayland `wl_surface.damage_buffer` contract](https://wayland.freedesktop.org/docs/html/apa.html#protocol-spec-wl_surface-request-damage_buffer)
defines damage in buffer coordinates as the area where pending buffer contents differ from current
surface contents. The Vulkan [copy-command rules](https://docs.vulkan.org/spec/latest/chapters/copies.html)
and [format rules](https://docs.vulkan.org/spec/latest/chapters/formats.html) govern the regional
buffer-to-image writes, image preservation copy, row length, and native BGRA/RGBA formats.
The Vulkan [dynamic-rendering rules](https://docs.vulkan.org/spec/latest/chapters/renderpass.html),
[descriptor-pool lifetime rules](https://docs.vulkan.org/spec/latest/chapters/descriptorsets.html),
[synchronization rules](https://docs.vulkan.org/spec/latest/chapters/synchronization.html), and
[viewport rules](https://docs.vulkan.org/spec/latest/chapters/vertexpostproc.html) govern the single
pass, per-completed-slot descriptor reset, explicit transfer/draw dependencies, and placement
viewports used by direct composition.

Adopted invariants are: a same-type root update preserves component/runtime identity; shell/runtime
layers emit only neutral deltas; scene identity is independent of placement; a partial write
preserves all pixels outside its rectangle; native four-channel SHM order remains explicit; Vulkan
and software own disjoint scene/output types; and resources named by an incomplete Vulkan submission
remain pinned. Rebuilding every frame on focus, flattening any layer through software before Vulkan,
creating per-layer CPU surfaces, replacing a sampled image with a full CPU re-upload, and issuing a
Vulkan pass/submission per desktop layer were rejected because each scales work with unaffected
state or crosses the backend boundary. Portable tests cover root reconciliation, disjoint regional
desktop writes, hidden-revision retention, shared-scene placement, native SHM channel order,
software BGRA sampling, and shared-staging alignment. Linux checks compile both direct compositor
assemblies, and portable source-boundary tests reject backend types in neutral desktop modules or
cross-references between the Vulkan and software assemblies. Hardware timing and visual
qualification remain user-run gaps; profiler spans distinguish Vulkan composite work from software
raster work and distinguish worker-full from owner-regional SHM copies.

### Live-resize scheduling audit

The inherited X11 root cursor now accepts a bounded static snapshot from the same
`render_cursor_image(TelorgonDefault)` asset/theme path used by native pointer rendering.
It is rasterized at logical resolution, preserving tint, alpha and hotspot. Animated
assets use their first frame; live theme changes and arbitrary composed pointer components
are not exported. Composed components or oversized images retain a diagnosed built-in
arrow fallback. An explicitly hidden default exports a transparent cursor.

`xwayland/root_cursor.rs` owns protocol-neutral RGBA-to-premultiplied-ARGB conversion
and a 128x128 bound. Manager startup asynchronously discovers RENDER 0.5 and an exact
ARGB32 picture format, checks the server pixmap layout/byte order, splits PutImage by
the negotiated request size, creates the cursor and assigns only the root attribute.
Temporary pixmap/GC/picture/cursor resources are freed in request order; the root retains
its cursor reference. Readiness waits for the checked upload/assignment barrier. Client
cursor overrides and intentional hiding are not intercepted.

Review used the official [RENDER protocol](https://www.x.org/releases/current/doc/renderproto/renderproto.txt),
the pinned Xwayland `render/render.c:ProcRenderCreateCursor` implementation and existing
Telorgon cursor rasterization/request accounting. The adjacent reference-library tree
remains unavailable. No GPU submission or retirement contract changes. Rejected approaches
were replacing every Xwayland cursor, monochrome conversion, and a runtime xsetroot/helper
dependency. Tests cover alpha, byte order, invalid bounds/hotspots, upload ordering and
checked failure; exact visual matching remains a user-run test.

Hidden final-size content receives pacing callbacks through the shared window predicate
for both native and X11 adapters. Otherwise Xwayland Present can wait for a callback
behind the very veil waiting for its replacement image. These callbacks do not report
hidden content as presented or release GPU ownership. The pinned Xwayland 24.1.13
`xwayland-present.c` frame-callback path and Telorgon's occluded-frame path were inspected;
state tests cover settling, stale revisions and completion. Live GLX validation remains
separate from these tests.

Explicit user close uses WM_DELETE_WINDOW where supported. For a live managed window
whose known protocols omit it, the XWM sends KillClient for that window resource, as
permitted by ICCCM deletion policy. This disconnects the owning X connection, potentially
closing its other windows; it never uses a claimed PID. Session shutdown remains
cooperative. Unknown protocols and stale window generations cannot trigger this fallback.

The optional X11 adapter also uses the EWMH basic
[`_NET_WM_SYNC_REQUEST` protocol](https://specifications.freedesktop.org/wm/latest-single/#idm45446104416128).
Bounded property readers require both protocol advertisement and a CARDINAL/32 counter
property. SYNC 3.1 is negotiated when present. The XWM initializes the basic counter to
zero, then sends a nonzero, monotonically increasing request before the real configure on
the same connection. It does not request the extended frame-counter protocol. Shared
resize, maximize and restore previews require the client counter acknowledgement as
well as the existing checked geometry and published-buffer conditions. Acknowledgement
is client-reported repaint progress, not a replacement for GPU acquire/retirement rules.

Counter queries use the existing sans-I/O request tracker, with one current query per
window (superseded queries retain bounded sequence accounting), 16 queries per turn,
128 tracked sync requests, and a 16 ms polling
interval. The owner-loop deadline includes polling and the one-second acknowledgement
deadline; no thread or XSync Await request blocks dispatch. Timeout or invalid-counter
states explicitly fall back to buffer checks and disable further handshakes for that
capability instance. Property replacement/removal and window destruction invalidate
epochs; cancellation and subsequent requests cannot accept an older request number.
Diagnostics contain window/generation IDs and failure reasons, never application content.

This change reuses Telorgon's request accounting and existing presentation gate without
changing GPU resource ownership. The adjacent `other-rendering-libs` tree was unavailable;
the EWMH contract and local wire/state code were inspected instead. Rejected alternatives
were blocking XSync Await, an arbitrary repaint delay, and treating a server barrier or
timeout as application acknowledgement. Tests cover ordered client-message/configure
traffic, counter initialization, delayed/stale replies, property bounds and explicit
fallback; real-client visual qualification remains user-run.

Android platform/base commit `1cdfff555f4a21f71ccc978290e2e212e2f8b168` was inspected at
`FluidResizeTaskPositioner`, `VeiledResizeTaskPositioner`, `ResizeVeil`, and `SurfaceControl` for the
separation between pointer-driven container geometry and application buffer production. Flutter
commit `51fd9afadf309ba5337320bd3653f5345c156cb9` was inspected for acquire-latest external-texture
replacement, import, and bounded reuse. Qt Declarative commit
`3e2d6bd456a8e850bcf641de77d1d5d8bc8419ef` was inspected at `QQuickWindow` and the threaded scene
graph loop for independent UI/event and render progress. wgpu commit
`d99c241a3b9dcc0f6674d990d007d79e94d39862` was inspected for owned Vulkan presentation and
DMA-BUF lifetime boundaries. The resulting Telorgon rule is that pointer motion updates placement
immediately, protocol configures and full copies coalesce, and no focus transition is permitted to
stand in for an XDG commit acknowledgement. No reference code or abstraction was copied.

### Atomic cursor-plane audit

```text
Concern:
Tear-free hardware-cursor positioning, atomic primary/cursor scheduling, and completion-safe cursor
framebuffer reuse.

Telorgon files/contracts affected:
crates/telorgon/src/application_host/desktop_wayland.rs;
crates/telorgon/src/presenter_vulkan_kms/{ffi.rs,kms.rs,model.rs}; this document.

Reference revisions, paths, and symbols inspected:
Android platform/base 1cdfff555f4a21f71ccc978290e2e212e2f8b168,
core/java/android/view/SurfaceControl.java Transaction, apply, setPosition, setBuffer, and fences;
Flutter 51fd9afadf309ba5337320bd3653f5345c156cb9,
engine/src/flutter/shell/platform/embedder/embedder_external_view_embedder.{h,cc} layer collection,
single present callback, and post-present recycling;
wlroots 0855cdacb2eeeff35849e2e9c4db0aa996d78d10, backend/drm/drm.c cursor desired-state
updates and atomic CRTC commit;
Smithay e3d461a057ba244d213a8498ec372b0799cca103,
src/backend/drm/compositor/mod.rs and src/backend/drm/surface/atomic.rs plane assignment, test-only
validation, atomic commit, and vblank completion.

Official specification sections checked:
Linux DRM KMS standard cursor-plane properties and legacy/atomic non-mixing rule; DRM client atomic,
universal-plane, and cursor-hotspot capabilities; DRM_MODE_PAGE_FLIP_EVENT,
DRM_MODE_ATOMIC_NONBLOCK, DRM_MODE_PAGE_FLIP_ASYNC, and test-only atomic commits; libdrm
drmModeAddFB2 and drmModeAtomicCommit declarations.

Invariants extracted:
All visible plane changes form one explicit atomic state; one CRTC commit is outstanding at a time;
new pointer events coalesce to newest desired state; nonblocking does not mean asynchronous scanout;
current or pending cursor buffers are immutable; replacement is reusable only after flip completion;
cursor-only commits do not imply client-surface presentation.

Failure and recovery cases extracted:
Missing cursor plane/ARGB8888/properties, unsupported modifier allocation, cursor image larger than
the reported hardware extent, rejected test or runtime commit, a cursor update arriving during an
outstanding primary flip, and composited fallback while a cursor framebuffer remains active.

Approaches rejected and why:
Legacy drmModeMoveCursor/drmModeSetCursor2 is an untracked driver-dependent update beside the atomic
primary path; composited cursors force primary damage on every move; DRM_MODE_PAGE_FLIP_ASYNC may
tear; one commit per raw input event ignores CRTC back-pressure and loses mailbox coalescing.

Telorgon-specific decision:
Auto-select an atomic ARGB8888 cursor plane, use three GBM/DRM framebuffer slots, append the newest
cursor generation to a primary or cursor-only vblank commit, and disable that plane atomically in
the same primary commit that introduces the composited cursor fallback.

Tests/diagnostics derived:
Pure cursor-state tests cover in-flight motion coalescing, current/pending buffer exclusion, and
fallback retirement after disable completion. Linux feature and profiler feature cross-target
checks compile the complete path. Profiler events distinguish atomic cursor submit, scanout
completion latency, image-stage failure, and atomic-commit failure.

Known gaps requiring hardware or vendor validation:
No physical/vGPU matrix has yet qualified cursor-plane format/modifier restrictions, negative edge
coordinates, hotspot properties, visual tear behavior, latency, runtime fallback, or driver-specific
atomic commit behavior.
```

Smithay and wlroots were rejected as implementation dependencies because Telorgon owns protocol
state, policy, composition, and rendering. They remain useful external compatibility references.

## Verification and qualification boundary

Portable state tests cover surface commits, roles, ownership, serials, subsurface cycles, buffer
release tracking, SHM/DMA-BUF validation, xdg configure ordering, XML schema bounds, and KMS frame
slot reuse. Windows-hosted checks cover the non-Linux declarations; an
`x86_64-unknown-linux-gnu` compile check covers the complete Linux feature graph without starting
the compositor. Per repository policy, no server or application is launched by automated work.

The current path is **operational by integration and compile evidence, but not
production-qualified**. A Linux TTY/hardware run is still required for socket/client conformance,
atomic modesetting and page-flip behavior, libseat transitions, input devices, multiple GPUs,
multiple outputs, failure recovery, and performance. The largest remaining gaps are
multi-output/hotplug, direct retained client DMA-BUF sampling, page-flip timestamps,
input-method/text-input integration, and newer DMA-BUF feedback/syncobj protocol generations.

## Build-time descriptor reference audit (2026-09-08)

The adjacent `../other-rendering-libs` checkout was unavailable. The relevant replacements inspected
were the local Cargo registry's `wayland-scanner-0.31.11/src/c_interfaces.rs` (static native graph,
`gen_messages`, `message_signature`) and the upstream C scanner's
[`src/scanner.c`](https://raw.githubusercontent.com/wayland-mirror/wayland/main/src/scanner.c)
(`emit_types`, `emit_messages`, `emit_code`, nullability validation). The installed official scanner
1.24.0 also generated comparison descriptors for all selected XML files. No reference code was
copied or added as a runtime dependency.

Both implementations preserve XML opcode order, encode `since`/nullable arguments in signatures,
expand untyped `new_id`, and retain immutable interface/message/type storage. These invariants agree
with the official [message format](https://wayland.freedesktop.org/docs/book/Message_XML.html) and
[server ABI](https://wayland.freedesktop.org/docs/html/apc.html). Telorgon adopts immutable static
tables with a narrowly scoped `Sync` guarantee. Embedding XML for startup parsing, rebuilding an
owned graph on each startup, adding a compositor framework, and requiring an external scanner for
ordinary builds were rejected as unnecessary runtime work or build dependencies.

`wayland_protocol_generation` compares every generated metadata field against independently read
XML, traverses native signatures and type pointers after temporary handles disappear, exercises
malformed/bounded input and ABI incompatibility rejection, and checks generic `new_id` wire slots.
The explicitly selected `signatures_match_official_c_scanner` test compares all message signatures
against the independent C generator. The self-contained descriptor test runs without reading XML
and can be run directly from the compiled test binary with missing protocol paths. These tests do
not create a display, socket, compositor server, GUI, or GPU/KMS output. Hardware and full client
conformance qualification remain manual.

Validation for this refactor passed on Linux:

```sh
cargo check -p telorgon --all-targets --no-default-features --features desktop-wayland-linux
cargo test -p telorgon --no-default-features --features desktop-wayland-linux --test wayland_protocol_generation -- --include-ignored
cargo test -p telorgon --lib --no-default-features --features desktop-wayland-linux compositor_wayland::
cargo fmt --all -- --check
```

The descriptor suite passed 13 tests (including the optional C scanner comparison); 37 existing
compositor state tests passed. Default, no-feature, and `embedded-vulkan` compilation also passed
with both XML overrides set to nonexistent paths. Separate Cargo checks verified custom inputs,
fresh unchanged builds, XML-edit rebuilds, rejection of a same-version ABI change, missing-file
errors, and environment-change rebuilds. Package listing includes all build inputs. A filesystem
trace of the descriptor-only test contained no protocol XML access. The non-Linux build-script
branch was exercised with missing inputs, but no non-Linux Rust target was installed for a full
cross-target check. Existing unrelated dead-code warnings remain. No GUI, server, or hardware run
was performed.

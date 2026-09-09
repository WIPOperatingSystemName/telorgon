# Wayland solid resize preview

Status: implemented with CPU regression and compile-time verification; interactive Linux visual and
performance qualification remains user-run. This replaces the earlier stretched live-client preview.

## Behavior and configuration

By default, interactive resize shows an opaque dark slate content veil (`#262a30`) while the composed server frame
and veil follow the pointer. This is a solid fill, not a blur or a scaled screenshot. The client image,
including client-drawn decoration, popups, and subsurfaces, is hidden until the final redraw is ready.
Server-drawn title text and controls remain ordinary composed chrome. The first version switches
directly; it does not add a fade, blur pass, readback, image resampling, or intermediate render target.

Set `LinuxDesktopConfig::resize_preview_color` to a `ColorRgba8` and pass the configuration via
the desktop declaration's `.linux(config)` method. All alpha values are supported. Easy frames can
override it with `WindowChromeDesign::resize_preview_color: Some(color)`; `None` inherits the host
setting. `content_background` independently configures normal backing beneath the app. Set that
backing's alpha to zero to let app-supplied transparency reveal lower desktop layers; opaque app
pixels and XRGB buffers remain opaque. See the [easy-frame example](CUSTOM_WINDOWS_ASSETS_AND_POINTERS.md).

The compositor cuts frame decoration out of the client rectangle with four non-overlapping clipped
placements sharing one retained scene. Easy frames paint their content backing separately in normal
use; custom templates can opt in through `WindowFrameTemplate::content_style`. During resize neither
the backing nor any client pixels draw beneath the preview, even for custom templates. Thus a
translucent preview reveals the desktop/lower windows, and alpha zero gives a frame-only resize.
Client pixels, backing, and preview share the frame's inner rounded
border clip, intersected with content-slot bounds and optional custom-template aperture rounding. Both rounded
contours start at the window's inner top edge; the title-bar seam does not get separate top corners.
An outline placement restores the curved rim, and for custom apertures an inverse-clipped fill placement restores wider
frame-colored corner wedges inside the cutout without backing the content aperture or double-blending
the frame strips. Subsurfaces inherit the clip; popups remain independent.
Color scenes have per-window identity and native-sized analytic geometry for both
backends. Resizing changes a box delta and placement, never client pixels or a CPU image allocation;
translation-only movement reuses the scene. Rounded clipping uses per-placement analytic shader
coverage (GPU ABI 4) and matching software coverage, without new extensions or offscreen passes.

On press, the compositor sends the resizing state with the client's committed window extent. Motion
only changes desired geometry; it does not issue intermediate size requests. Release sends the final
size without the resizing state. The veil persists until an image publication acknowledges that final
configure or a later superseding configure. Cell/aspect-constrained clients may choose a smaller legal
extent; final geometry reconciles around the fixed opposite edges. A new resize supersedes an older
pending final transaction. Unmap removes its preview and pending requests; maximize/fullscreen reset
the transaction. A nonresponding client can leave its veil visible, but does not block input, other
clients, closing the window, or a subsequent resize. No timeout pretends that client content is ready.

## Work and ownership

- Each window retains a native-sized analytic color scene for its veil. Resizing updates that box
  and its placement/damage without uploading client pixels; translation reuses the scene. Composed
  chrome can still require layout/scene updates. Vulkan and software consume the same neutral placements.
- A veiled tree retains its image scenes and queued pixel deltas without drawing or uploading them.
  Source and destination image extents remain equal; XDG margins and visible clips are separate.
- New SHM reads/conversions during an active drag enter the existing bounded, replaceable per-surface
  mailbox. Replacement retires an unused request; the latest request retains its buffer until copied
  or superseded. Already submitted work is allowed to finish. Paused requests cannot prevent unrelated
  surfaces from entering the worker, and the bounded queue scan never spins on paused entries.
- The first publication replacing an unread request takes a full copy, even for same-sized buffers
  with partial damage. Otherwise earlier deferred damage would be lost. Replacing a request with the
  same `wl_buffer` must not release it before the replacement read finishes. Explicit release remains
  per commit, while ordinary buffer release still respects other pending reads.
- Client frame callbacks are withheld during the active drag. After release, callbacks for accepted
  hidden intermediate revisions may resume without a new KMS flip, so callback-paced clients can
  reach their final redraw. Callback-only wakes leave presentation feedback pending; an actual
  displayed frame later reports its own revision and discards superseded hidden revisions. Each
  scanout slot carries the window/drag-image revisions in its rendered frame, not the latest client cache at flip
  completion. Future revisions stay queued for their own presentation. Damage-free commits can
  explicitly reuse delivered pixels while advancing revision/callback progress without a texture
  upload. The final image and preview removal enter the
  same desktop frame; no SHM callback alone removes a preview early.
  The separate cursor-plane callback/presentation path is unchanged by this window-resize policy.
- DMA-BUF acquire/release and materialization rules are unchanged. Its client resize requests are
  likewise reduced and hidden content does not draw, but this does not promise zero DMA-BUF import
  work for clients that keep committing independently of frame callbacks. Likewise, Telorgon cannot
  prevent independent client-side animation or work. Performance savings are not hardware-measured.

The profiler's existing `compositor.shm.copy_bytes` records actual reads. The new
`compositor.resize.shm_copy_deferred` instant records requests paused by an active resize; it is not an
estimate of saved bytes. Expect no new drag-triggered SHM reads after preexisting work drains.

## Reference and protocol audit

Concern: hide intermediate resize content and reduce producer work without blocking the Wayland
owner, losing skipped damage, releasing a leased buffer, or fabricating presentation feedback.

Read-only references inspected (paths relative to `../other-rendering-libs`):

- Android platform/base `1cdfff555f4a21f71ccc978290e2e212e2f8b168`,
  `base/libs/WindowManager/Shell/src/com/android/wm/shell/windowdecor/FluidResizeTaskPositioner.java`,
  `VeiledResizeTaskPositioner.java`, and `ResizeVeil.kt`: compare per-motion application bounds with
  veil-only movement, final bounds submission, fixed geometry, and a color-surface preview.
- Zed `f4178619acd0d47ea1f76a2025c42962c6d6638c`,
  `zed/crates/gpui_linux/src/linux/wayland/window.rs`: `handle_xdg_surface_event`, `frame`,
  `set_size_and_scale`, and `resize`; client configure acknowledgement, vblank resize throttling,
  geometry, viewport, renderer resize, and callback-driven progress are separate operations.

Official protocol contracts checked: the XDG-shell XML's `xdg_surface.set_window_geometry`,
`ack_configure`, and `xdg_toplevel.resize/configure/state.resizing`
([XML rendering](https://wayland.app/protocols/xdg-shell)); core
[`wl_surface.frame` and `wl_buffer.release`](https://wayland.freedesktop.org/docs/html/apa.html);
and presentation-time's
[`presented` versus `discarded`](https://wayland.app/protocols/presentation-time).
No graphics API ownership/barrier contract changes. No reference source or public abstraction copied.

Extracted invariants: desired frame geometry is not committed client image geometry; only the client
can redraw; callback pacing is not presentation proof; hidden and superseded work must retain/release
the same buffer ownership as visible work. Rejected alternatives: stretched snapshots distort text;
live clipping/padding exposes dead space; continuous configure/copy loops pay for unseen intermediate
sizes; synchronous waiting blocks unrelated clients; CPU blur/readback diverges between SHM and
GPU-backed content; early buffer release permits mutation during the delayed read.

## RGBA and content-backing reference audit

Read-only comparisons for the transparency follow-up:

- egui `fd54387eac03f57ca772a8fb590ceaadf780f31c`,
  `egui/crates/egui-wgpu/src/renderer.rs`: premultiplied source-over blending retains destination
  contribution when source alpha is below one.
- Flutter `51fd9afadf309ba5337320bd3653f5345c156cb9`,
  `flutter/engine/src/flutter/impeller/entity/contents/solid_color_contents.cc`: color/opacity,
  premultiplication, geometry coverage, and opaque classification remain separate concerns.

Official checks: [Vulkan blend factors](https://docs.vulkan.org/spec/latest/chapters/framebuffer.html#framebuffer-blending)
and [half-open scissor coverage](https://docs.vulkan.org/spec/latest/chapters/fragops.html#fragops-scissor);
the [core Wayland SHM format contract](https://wayland.freedesktop.org/docs/html/apa.html) distinguishes
ARGB from XRGB and specifies premultiplied alpha. No client-format or blend-equation changes are made.

Invariants: an alpha-zero preview cannot reveal the hidden client; frame root fill/shadow cannot
obscure transparent content; each frame pixel blends at most once; shared frame resources receive
deltas once; every color scene keeps an independent window identity; empty cuts retain scene ownership;
native-sized analytic geometry works without image scaling in either renderer. Rejected alternatives:
changing only the color leaves an opaque backing; fading the whole window also fades controls;
overlapping border strips double-blend corners; a vendor-exclusive scissor or shader hole would add
unnecessary GPU requirements; changing image formats or forcing XRGB transparent would violate client
semantics. The software compositor's existing native-extent contract also rules out a scaled 1x1
solid scene. Reference sources were neither modified nor copied.

## Rounded-frame clipping audit

Concern: preserve the curved border rim while clipping external content, including opaque XRGB
images, to the same inner contour. The outer radius alone cannot define the inner edge; border
thickness and content-slot offsets matter. Transparent content must not regain an opaque backing.

Read-only references inspected:

- Flutter `51fd9afadf309ba5337320bd3653f5345c156cb9`,
  `flutter/packages/flutter/lib/src/painting/rounded_rectangle_border.dart`, `getInnerPath`,
  `getOuterPath`, and `paint`: derive an inset inner contour and paint the difference between outer
  and inner bounds. Telorgon's uniform easy-frame border follows that geometry; custom asymmetric
  borders retain the existing analytic-box renderer's per-corner circular inset convention.
- Android platform/base `1cdfff555f4a21f71ccc978290e2e212e2f8b168`,
  `base/core/java/android/view/SurfaceControl.java`, `Transaction.setCrop` and `setCornerRadius`:
  clipping is surface-composition metadata, not a request to rewrite the producer buffer; child
  surfaces and independent overlays have different clipping ownership.

Official checks: [CSS corner shaping](https://www.w3.org/TR/css-backgrounds-3/#corner-shaping)
for uniform border inset geometry and [Vulkan fragment operations](https://docs.vulkan.org/spec/latest/chapters/fragops.html)
for rectangular scissor bounds versus fragment coverage. No vendor-specific exclusive-scissor,
stencil attachment, mask texture, imported-image mutation, or new synchronization feature is needed.

Implemented invariants: at most two output-space rounded bounds per composite placement (frame
interior and window-top-anchored content aperture), intersected with rectangular scissors; premultiplied color and
alpha both receive coverage; clipped opaque batches use source-over for their edge; shader/CPU
coverage use the same one-output-pixel distance rule; radius-only changes damage old/new placement
bounds without touching image resources; frame-node replacement cannot accumulate stale border
geometry. The border-only patch and four frame strips have disjoint scissors. The existing
slot-fenced GPU upload/driver workarounds, image ownership, callback pacing, and input hit regions
remain unchanged. The easy frame clips composed descendants, not its own shadow.

Rejected alternatives: merely rounding the background leaves square client pixels; an opaque corner
cover breaks transparency; a radius-only content setting ignores border thickness and title-bar
offsets; CPU image masks/readbacks add resize work and diverge for DMA-BUF; shader-space hard discard
alone creates jagged edges. General arbitrary clip stacks remain outside this bounded compositor
feature. No reference code was copied or modified.

Vulkan shader assets are regenerated offline, SPIR-V validated, and reflected against the 192-byte
view-block offsets in all eight stages. This is compilation/verification, not an application run.

## Verification and manual qualification

### Corner-gap correction audit

The initial rounded-frame implementation rounded the app rectangle independently below the title
bar and restored only the thin outline, leaving desktop-visible wedges when content margins were
wider than the outline. The correction retains the frame's root fill color and paints its interior
only outside the full-height content aperture. The original border-only patch still owns the outline;
the four rectangular frame strips remain disjoint from both patches. No fill is restored inside the
aperture, even for alpha-zero clients/previews. Easy-frame slot decoration also keeps its top corners
square when a title bar is visible. Changing title-bar height cannot shift the window-top contour or
shrink bottom radii merely by shortening the app rectangle.

Re-inspected references: Flutter `51fd9afadf309ba5337320bd3653f5345c156cb9`,
`packages/flutter/lib/src/painting/rounded_rectangle_border.dart` (`getInnerPath`, `paint`), and
Android/base `1cdfff555f4a21f71ccc978290e2e212e2f8b168`,
`core/java/android/view/SurfaceControl.java` (`setCrop`, `setCornerRadius`). The former distinguishes
frame fill/outline and the interior curve; the latter keeps cropping at composition rather than
modifying client buffers. Official checks remain [CSS corner shaping/clipping](https://www.w3.org/TR/css-backgrounds-3/#corner-shaping)
and [Vulkan scissor bounds](https://docs.vulkan.org/spec/latest/chapters/fragops.html#fragops-scissor).
Rejected: rounding the app's top corners, outlining without filling wider corner wedges, and restoring
an opaque full backing. No reference code was copied. Image lifetime, SHM copying, resize/configure
pacing, and V3DV upload workarounds are unchanged.

Neutral `RoundedClip::inverse` supplies complementary coverage in software and four Vulkan fragment
stages. GPU ABI 4 keeps the 192-byte view layout but assigns bits 1/2 of view flags to clip inversion;
the offline bundle is regenerated, validated, and reflected. No new GPU extension, mask image,
offscreen pass, or presenting run is required. Tests cover bottom wedges with margins wider than the
outline, the square app seam versus rounded window top, translucent frame fill without duplicate
blending, client/preview alpha 0/128/255, title-bar-height independence, empty/antialiased inverse
coverage, inversion-only placement damage without image uploads, and per-placement GPU flag reset.

### Border-derived easy-frame follow-up

Easy frames now derive content bounds from `title_bar.height` and the root border, removing the
redundant `WindowChromeStateStyle::content_margin` and `content_radius` fields. They publish
`WindowContentStyle::corner_radius = 0`, selecting only the full-window inset border contour, and
skip the unnecessary corner-fill placement. Custom templates retain aperture support. The outline,
client/backing/veil clip, and resize exclusion all use the same normalized `RoundedClip::inset`
geometry. Root layout already reserves the border width, so only title-bar height is added to the
content slot. The composed slot's bottom radius is derived rather than independently configured.

Resize regions opt into `WindowChromeHitSpec::frame_border_outset`; snapshots derive geometric hit
clips from the root's border style. A minimum grab thickness can exceed the painted outline, but
only outward; its per-side extent is `max(resize_edge - frame_border_width, 0) + resize_hit_slop`.
Corner bounds cover the outer radius and are constrained to their quadrant, then exclude the inner
curve just like edge regions. The host evaluates these targets before the outside-frame rejection,
allows only resize actions outside, and retains fractional pointer coordinates. This changes resize
hit geometry, not client pixel alpha or general transparent-pixel click-through behavior.

Read-only audit: Flutter `51fd9afadf309ba5337320bd3653f5345c156cb9`,
`packages/flutter/lib/src/painting/rounded_rectangle_border.dart` (`getInnerPath`, `paint`,
`hitTest`); Android/base `1cdfff555f4a21f71ccc978290e2e212e2f8b168`,
`libs/WindowManager/Shell/src/com/android/wm/shell/windowdecor/DragResizeWindowGeometry.java`
(`calculateCtrlType`, `calculateEdgeResizeCtrlType`, `checkDistanceFromCenter`). Invariants:
derive the inner contour from the border; input tolerance and visual geometry remain separate;
corners use geometry rather than a square that steals interior input. Rejected: independently
configured margins/radii, double-counting layout borders, inward expansion over app controls, and
unbounded overlapping corner handles. Official checks are the CSS corner-shaping/clipping and Vulkan
scissor references above. No reference code was copied. GPU ABI 4 and shaders, buffer ownership,
resize configure pacing, and V3DV workarounds remain unchanged.

New CPU tests cover automatic bar/border insets, zero/thin/thick borders, all curved resize corners,
outside reach, interior exclusion, hidden bars, disabled/tiled edges, shared inset/outset geometry,
and omission of extra fill/clip for easy frames. Native input routing is additionally inspected and
Linux-test-compiled; interactive pointer/visual qualification remains user-run.

### Checks

CPU tests cover native coordinate/clip mapping, shadow exclusion, all eight anchored edges and
cell-snapped final sizes, paused mailbox fairness/resume, image-free solid previews, hidden image
retention, one final image publication, monotonic solid-scene epochs, full-range RGBA validation, final
configure serial/ack supersession, and hidden versus visible feedback with future revisions retained.
Source-boundary tests include the new geometry/state policy; Linux test compilation checks native
owner integration. No compositor, GUI, server, or GPU-presenting test is run by the agent.

The transparency follow-up adds CPU framebuffer tests for alpha 0/128/255, hidden client/backing
exclusion, premultiplied/straight/opaque client modes, non-overlapping translucent frame strips,
rounded-to-square backing transitions, and resized coverage repaint. Neutral tests cover partially
offscreen/full-frame cutouts, retained ownership, independent preview colors, and easy/template API
inheritance. Software composite tests also run on non-Linux development hosts without opening a device.

Rounded-frame tests additionally raster-check both bottom rims and outside corners, zero/oversized
radii, zero/thick borders, preview alpha 0/128/255, radius changes without image uploads, and border
scene reuse across source-node replacement. Geometry tests check inset centers and empty clips.

Verification for the current implementation, including the rounded-frame follow-up:

- `cargo test -p telorgon --lib --quiet`: 938 passed.
- `cargo test -p telorgon --lib desktop_wayland --features embedded-vulkan,profiler --quiet`:
  40 passed.
- `cargo test -p telorgon --lib clip_tests --features embedded-vulkan,profiler --quiet`:
  1 passed (CPU-only view-uniform encoding).
- `cargo test -p telorgon --test window_frame_api --quiet`: 6 passed.
- `cargo check -p telorgon --tests --target aarch64-unknown-linux-gnu --no-default-features
  --features desktop-wayland-linux,embedded-vulkan,profiler`: passed.
- `cargo build -p telorgon --lib --release --target aarch64-unknown-linux-gnu --no-default-features
  --features desktop-wayland-linux,embedded-vulkan,profiler`: passed.
- Formatting, whitespace, and changed-document relative links checked. Existing platform-dependent
  dead-code warnings remain; these results are not hardware performance or visual evidence.

User-run checks: resize a terminal aggressively from all eight edges; hold still during a drag and
release; repeat before the previous redraw finishes; try a slow client, no-motion click/release,
client-side decorations, popups/subsurfaces, partial-damage SHM animation, and a DMA-BUF client.
Check that chrome remains usable, the default veil covers all content, the final content is sharp,
and no stale patches or unintended background gaps appear. For RGBA, try 0/128/255 preview alpha,
overlapping windows, an alpha-capable SHM/GPU client, and an opaque client; verify lower layers move
behind transparency while stale client pixels never appear during resize. Compare actual SHM-copy counters during a held drag and
after release. Repeat with the software backend and the target Raspberry Pi/Mesa, AMD, Intel, and
NVIDIA hardware as available; build success alone does not qualify that matrix.

For rounded frames, test contrasting desktop/client/border colors with thin, zero-width, and thick
borders; custom aperture radius zero and nonzero; normal/maximized/tiled transitions; translucent previews;
and child surfaces near corners. The visible rim must remain curved while content never extends
beyond its inner edge. Popups may extend beyond the window, and shadows remain outside its outer edge.
Check resize cursors and press/drag both on the curved outline and just outside all eight edges;
content immediately inside the border must remain interactive. Confirm hidden bars reserve no title
height, thicker borders move the content inset once, and tiled/non-resizable edges stay disabled.

### Outer frame corner containment

Retained frame strips now carry the root frame's outer rounded contour as a compositor-space
clip, including the title-bar strip and side/bottom strips. Content already uses the inner contour;
rectangular frame allocations must not expose backing paint beyond the outer contour. Clip changes
use the existing placement-damage path, including rounded/square state transitions.

This deliberately contains all frame paint, including shadows, inside the outline. Proper exterior
shadows require separate expanded placements; the current frame-sized rectangular placement is not
suitable for them. Merely changing the content background color was rejected because it conceals,
rather than bounds, escaping frame paint. The CPU framebuffer regression uses an opaque blue
rectangular backing and checks all four corners through rounded/square/rounded transitions.

The adjacent reference library is unavailable in this checkout. This bounded correction reuses the
existing rounded-placement contract and its software/Vulkan implementations; it introduces no new
GPU mechanism, synchronization, or resource-lifetime changes. Live confirmation of the reported
artifact remains a user-run check.

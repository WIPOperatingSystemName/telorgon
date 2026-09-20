# Visual capture picker

Status: implemented source and CPU verification; live Vulkan/Discord qualification is pending.
Physical multi-output presentation/hotplug remains unimplemented in the host.

## Ownership and files

- `shell_components/capture/mod.rs`: the host-issued `CaptureUi` decision handle and snapshot.
- `capture/selection.rs`: request/source/epoch selection and paging state. No capture resources.
- `capture/layout.rs`: adaptive logical dimensions shared by controls and preview slots.
- `capture/chooser.rs`: public `CapturePicker`, Screens/Windows categories, cards, explicit Share,
  Cancel, keyboard arrows, normal button focus/activation and bounded paging/wheel navigation.
- `capture/indicator.rs`: content-sized starting/sharing/stop controls and dismissible failure text.
- `compose/shell_widget.rs`: visual-only `ShellOutputPreview`, alongside `ShellWindowPreview`.
- `application_host/shell_wayland/widgets/output_previews.rs`: validation, aspect fit, cropping and
  retained desktop scene reuse. The ordinary producer remains the sole scene publisher.
- `application_host/shell_wayland/capture_portal.rs`: consent validation, actual output labels,
  stream readiness and fixed user-facing failure reports.
- The test compositor explicitly installs `CapturePicker::new` through `capture_chooser`.
  It is also the default portal chooser. There are no Discord-specific branches.

## Behavior

Selecting a card changes local selection only. Share submits the exact request, source identity
and mapping epoch; the owner revalidates them. A request/source incarnation change retires the old
card and Share control and disables sharing until another explicit selection. Cancel/Escape reject
the pending request. Source availability follows the existing host registry (mapped, non-minimized,
ready windows); sources disallowed by the app are never offered. Window names no longer have a
redundant "Share window:" prefix. Output names include the real connector name and pixel dimensions.

The chooser displays at most six cards at once. Source categories and geometry adapt to the source
count and available logical output size. Previous/next buttons and the wheel move through pages;
arrow keys move selection through the current category, including across page boundaries. Tab and
Shift-Tab use the normal button focus system; Enter/Space activate the focused control. Escape
cancels. Only the Share button approves. The status line identifies the selected source.

The sharing indicator uses shrink dimensions for both container axes and its rows. It distinguishes
PipeWire startup from ready sharing, offers Stop while starting or active, and retains a dismissible
message on startup/stream/source failure. Detailed native errors remain in the compositor log.

## Preview implementation and limits

Window cards reuse the existing retained window/subsurface preview implementation. Output cards
reuse visible background, client and panel scenes as scaled placements. They do not copy the
framebuffer, read pixels back to the CPU, allocate new capture pools, create PipeWire streams or
start a preview timer. Damage to a producer updates every placement of its scene, including a
thumbnail. Static scenes generate no extra frames. Each widget may request at most eight output
slots; the default picker displays at most six sources total.

Output previews exclude overlay widgets (including the chooser and sharing indicator), other
preview copies, cursors, drag icons, transient tiling/resize veils and glass-producing layers.
Exclusion prevents recursive previews and avoids rebinding a backdrop producer to a second target.
Thumbnails show base desktop geometry rather than compositor motion/glass effects. These are source
selection previews, not a guarantee that every desktop effect exactly matches the outgoing stream.

All slots must be finite, positive and inside their widget. Cropping and rounded clips map into the
same logical coordinate system before the existing physical-scale conversion. Hidden widgets and
session lock suppress previews. An unknown output ID renders nothing, never the primary display.
The ordinary scene producer owns uploads, versioning and GPU lifetime; copies publish no image or
retained-scene update and use no new GPU synchronization or external-image mechanism.

## Multiple monitors: prerequisite still outstanding

The picker preserves arbitrary OutputIds and invalidates removed output selections. This is not
physical multi-monitor support. `shell_wayland.rs` still chooses one connected KMS connector, one
CRTC/plane, one scanout pool, one renderer/scene and one scale, advertises that output to Wayland,
and uses it for input/window coordinates and capture. Adding cards for other connected connectors
would falsely advertise streams with no rendered content.

Completing multiple-monitor sharing requires a separate host-level change:

1. Own a presentation/scheduling/scene/scale state per active output and allocate compatible,
   non-conflicting connector/CRTC/plane assignments.
2. Maintain output identity and mapping epochs through hotplug/mode changes; publish matching
   Wayland globals, logical desktop layout and window membership.
3. Route pointer, window, shell-widget and cursor geometry across those outputs.
4. Route each capture source to its own renderer/extent/scale and report that output's position;
   preserve in-flight GPU and PipeWire retirement when an output disappears.
5. Validate at least two physical outputs, mixed scaling, disconnect/reconnect and active sharing.

None of these native display changes are claimed by this picker implementation. Live Discord video
delivery also remains unqualified; registration and unit tests alone do not establish it.

## Reference and specification review

The adjacent `../other-rendering-libs` directory was absent. The scoped change reuses existing
rendering ownership; no KMS, external-memory, frame-submission or GPU resource-lifetime contract was
changed. Upstream sources inspected on 2026-09-19:

- [Flutter TransformLayer](https://github.com/flutter/flutter/blob/master/engine/src/flutter/flow/layers/transform_layer.cc):
  `Diff`, `Preroll`, `Paint`; finite transforms, transformed child bounds and invalidation when
  geometry changes. Retrieved from upstream master; no pinned commit was available in the response.
- [Smithay render-element utilities](https://docs.rs/smithay/latest/src/smithay/backend/renderer/element/utils/elements.rs.html):
  `RescaleRenderElement` and `CropRenderElement`; origin-relative mapping, source crop preservation,
  scaled damage and zero-intersection rejection. Retrieved from the published latest source page.
- [Vulkan render-pass specification](https://docs.vulkan.org/spec/latest/chapters/renderpass.html):
  attachment read/write feedback restrictions; avoid sampling the target being rendered.
- [Portal ScreenCast contract](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.impl.portal.ScreenCast.html):
  source-type admission, Start consent and compositor-coordinate metadata. The backend's existing
  version-3 wire profile remains unchanged.

Invariants derived: preserve source identity and painter order; transform geometry and clipping
consistently; do not consume a producer update twice; damage all copies; exclude recursive content;
never substitute a primary output for an unavailable one; never approve on rendering or selection.
Rejected: framebuffer sampling/readback for thumbnails, a second PipeWire preview pipeline,
independent preview timers, and advertising connected but unhosted monitors. No reference code was
copied. CPU tests exercise these boundaries; native GPU pixels and input require live qualification.

## Automated verification

- Capture + XWayland library suite: 1,517 passed, 8 hardware tests ignored.
- Public capture chooser/configuration integration tests: 4 passed.
- Test compositor: normal and embedded-Xwayland/instrumentation builds checked successfully.
- Library without default features: checked successfully.
- Mounted CPU tests cover compact indicator size, real button bounds, thumbnail pointer selection
  followed by explicit Share, plus normal/narrow reference renders.
- Consent tests cover source/request/epoch replacement, disappearing sources, paging, distinct
  output identities, inert handles and bounded-queue backpressure.
- Composition tests cover hidden/locked/invalid output slots, recursive-content exclusion,
  single scene publication, idle unchanged frames and widget opacity grouping.
- Both repositories pass `git diff --check`. Existing unrelated compiler warnings remain.

The first private Wayland socket fixture was blocked by sandbox allocation; the isolated test and
full suite passed outside the sandbox. No GUI application, display service or Discord was launched.

## Manual qualification

Run the test compositor normally, launch Discord in its session, and share:

- Screens: check the real output thumbnail, select it, then Share; confirm video from another client.
- Windows: check several native and XWayland thumbnails and select each independently.
- Cancel and Escape: neither should create a stream.
- Close/minimize/remap the selected window before Share: the old selection must become unusable.
- Resize the shared window and stop sharing from both Discord and the compact indicator.
- Repeat at different display scales; inspect preview aspect ratio, button focus, and clipping.
- Verify source/stream failure produces a dismissible error and a subsequent request still works.

Repository `AGENTS.md` leaves GUI/hardware-presenting runs to the user. The automated tests are
headless; `TELORGON_CAPTURE_TEST_IMAGES=/tmp/directory` optionally writes CPU picker renders from
the mounted layout regression test. The directory must already exist. They are not live captures.

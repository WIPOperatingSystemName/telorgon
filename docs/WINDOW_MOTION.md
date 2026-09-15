# Desktop window motion

Status: initial implementation, with CPU framebuffer/controller tests and compile validation.
Live Vulkan appearance and performance remain user-qualified. The desktop host animates composed
windows; native OS window animation is outside this API's control.

Fluid geometry is implemented alongside the original smooth tween geometry. Both run through the
same desktop motion controller, renderer, placeholder sequencing, and Wayland/Xwayland readiness
adapters. The consuming test compositor currently selects the editable fluid preset.

## Authoring

Set one field on `WindowChromeDesign`:

```rust,ignore
motion: WindowMotion::smooth(),
```

Use `WindowMotion::fluid()` for spring geometry, or customize every parameter with const builders:

```rust,ignore
motion: WindowMotion::fluid()
    .maximize_spring(Spring::new()
        .initial_velocity(3.2)
        .damping_ratio(0.87)
        .angular_frequency(20.5)
        .settle_within_ms(450))
    .restore_spring(Spring::new()
        .initial_velocity(2.8)
        .damping_ratio(0.86)
        .angular_frequency(19.5)
        .settle_within_ms(475))
    .maximize_content(ContentFade::new(50, 130)),
```

These are the fluid preset's defaults. `Spring::new()` independently defaults to velocity 3,
damping 0.86, angular frequency 20, and a 460 ms cutoff. Velocity is normalized displacement per
second; frequency is radians per second. Damping must be strictly between zero and one. Velocity
is bounded to [-100, 100] and frequency to [0.1, 1000]; invalid/nonfinite builder inputs panic
(including during const evaluation). This version implements the specification's underdamped
regime, not critical or overdamped springs. `settle_within_ms(0)` is immediate.

The cutoff is measured after the entry fade and forces exact destination geometry and zero velocity.
It does not stretch the spring curve to fit a duration. Short cutoffs or highly oscillatory settings
can therefore snap at the end. The default entry fade adds 50 ms before movement; app readiness can
add a wait before the 130 ms reveal. Cosmetic opacity/radii retain their existing constraints; spring
geometry permits overshoot while keeping dimensions at least one pixel.

`maximize_spring` sets the paired direction unless an explicit restore override exists, just like
`maximize(tween_ms(...))`. `fluid()` supplies an explicit restore spring with its own settings.
`WindowMotion::fluid().restore(tween_ms(240, Easing::EaseOut))` mixes spring maximize with tween
restore. Each setter selects one geometry type; it does not layer competing geometry animations.
`maximize_transition` now returns `GeometryMotion::{Tween, Spring}`; use `.duration_ms()` to inspect
either variant's time limit. `WindowMotion` supports `PartialEq`, not `Eq`, because spring settings
contain floating-point values. Manual edge resizing remains pointer-driven; spring geometry applies
to maximize/restore and title-bar drag-to-restore. Optional velocity-modulated surface effects from
the proposal are not included; existing rounded outlines and analytic shadows remain in use.

### Fluid sampling and performance

`desktop_wayland/motion/geometry.rs` holds four floating-point positions and velocities. It solves
the damped oscillator analytically from monotonic elapsed time, without a keyframe table, fixed-step
integration, or per-sample heap allocation. Decay/frequency and per-coordinate coefficients are
prepared once per target change. One exponential and one `sin_cos` pair serve all four coordinates;
position and derivative share their intermediates. A timestamp cache avoids repeated calculations
within one compositor frame. Coordinates are rounded only when producing render rectangles.

Spring interruption carries position and velocity into the new target without restarting the entry
delay. Tween-to-spring interruption estimates the tween derivative locally. Changing a target can
change acceleration; acceleration continuity across retargets is not claimed. Drag translations move
both ends without resetting spring time, and drag-to-restore constrains horizontal velocity to the
grab anchor while preserving size velocity. Measured target corrections retain the remaining cutoff.
Reduced motion, final-frame repaint, snapshot reuse, and app-ready withholding remain shared.

The scalar spring matches an independent RK4 integration of the governing differential equation in
CPU tests. Tests also cover overshoot, precise cutoff, vector velocity continuity, refresh-rate
independence, translation, parameter validation, zero duration, const public API use, and both smooth
and fluid drag/held-placeholder behavior. The explicit release-mode `spring_sampler_cpu_throughput`
microbenchmark measured 33.2 ns per rectangle sample over 500,000 samples on this host. This is one
CPU microbenchmark result, not a GPU/frame-latency measurement or a universal optimum claim.
Final fluid validation: 1,284 library tests passed (four ignored), 13 public window-frame API tests
passed, and both core-only and embedded-Xwayland test-compositor compile checks passed. The ignored
CPU sampler microbenchmark was run separately in release mode; GPU tests were not executed.

Reference audit: the adjacent library remains unavailable. The upstream Flutter
[`spring_simulation.dart`](https://raw.githubusercontent.com/flutter/flutter/master/packages/flutter/lib/src/physics/spring_simulation.dart)
spring solutions and AndroidX
[`SpringForce.java`](https://raw.githubusercontent.com/androidx/androidx/androidx-main/dynamicanimation/dynamicanimation/src/main/java/androidx/dynamicanimation/animation/SpringForce.java)
`init`/`updateValues` were inspected on their moving branches. They independently model displacement
and velocity, precompute spring constants, and distinguish damping regimes. Telorgon uses the
supplied specification's underdamped equation with a shared rectangle sampler and explicit cutoff;
no source was copied and no dependency added. Keyframe tables and Euler stepping were rejected
because they introduce sampling/interruption complexity or integration error. No GPU API contract
changes, shaders, allocations, or synchronization primitives are introduced by the spring sampler.

Customize just the effects you want. These builders are const-compatible, including in a complete
`const TEST_CHROME: WindowChromeDesign` declaration:

```rust,ignore
motion: WindowMotion::smooth()
    .maximize(tween_ms(240, Easing::EaseOut))
    .maximize_content(ContentFade::new(50, 130))
    .minimize(Minimize::shrink_and_fade(180))
    .resize_content(ContentFade::new(90, 130)),
```

All durations above are milliseconds. The two content durations are entry into the placeholder and
reveal of ready client content. The placeholder's color and alpha remain in `resize_preview`.
`WindowMotion::none()` disables window effects. `Default` chooses `smooth()`.

`maximize` sets both maximize and unmaximize timing. Add `.restore(tween_ms(...))` to override only
unmaximize. `minimize` also supplies the entrance effect from minimized; `.unminimize(...)` overrides
that direction. Overrides survive subsequent paired setters, so builder order cannot accidentally
clear an explicit override. Zero-duration transitions settle immediately. Motion never repeats.

Custom templates can implement
`WindowFrameTemplate::motion(&self, &WindowChromeModel) -> Option<WindowMotion>`. Returning `None`
inherits `LinuxDesktopConfig::window_motion`, whose default is `none()` for compatibility.
`Some(WindowMotion::none())` explicitly disables motion. Easy frames return their design's style.
`LinuxDesktopConfig::motion_preference = MotionPreference::Reduced` settles visual motion centrally
while preserving client-content readiness gates.

The consuming `test-compositor/src/main.rs` opts into the editable configuration above. Its normal
`start.sh` rebuilds the dependency. Change timings there, restart, and exercise maximize/restore,
minimize/activation, and drag resize.

## Behavior and implementation boundaries

The neutral style lives in `window_chrome/motion.rs`; the desktop controller lives in
`application_host/desktop_wayland/motion.rs`. The controller samples geometry, visibility, and
content independently using the existing frame clock. It never sends protocol configurations.
Wayland and X11 retain their existing final-frame readiness and configure scheduling rules.
Both backends also share a presentation latch for maximize/restore (including title-bar restore).
It keeps the first placeholder visible to composition even if protocol redraw completion occurs
before the next frame. After that frame the latch is consumed and another repaint is requested;
the common controller handles entry, movement, and ready-content reveal. This prevents fast X11
clients from skipping the placeholder without delaying or falsifying their protocol completion.
There is no Xwayland-specific frame renderer or animation controller. Only configure submission,
acknowledgement, and redraw evidence remain specialized to each protocol.
Motion-enabled Wayland restore operations now install their own terminal configure gate, including
drag-to-restore, so the restored content cannot be revealed from an older maximize transaction.
Ordinary direct manipulation cancels geometry easing and follows the pointer. Dragging a maximized
title bar is the exception: crossing the existing drag threshold starts restore while retaining
the pointer grab. The window translates immediately with pointer motion as its size animates,
keeping the horizontal title-bar grab fraction and vertical offset attached to the pointer.
Movement does not restart the animation deadline, and releasing the pointer lets an unfinished
restore complete. An edge resize still cancels easing. Wayland/Xwayland submission and readiness
gates are unchanged. CPU regression covers the initial size, moving grab anchor, release mid-restore,
fixed completion deadline, and hidden content while awaiting the app. This is a controller/input
classification correction using the existing rendering contract, with no new GPU mechanism.
When an early app redraw is held during drag-to-restore, the retained placeholder's bounds are
rebased from the previous destination window to the current one before mapping to animated geometry.
Its pixels are reused; only the placement changes. Previously those bounds retained the old screen
position while the outer clip tracked the pointer, producing an inset/misaligned rectangle. A CPU
regression fails with that ordering and verifies snapshot/clip alignment, filled edge pixels, and
capture-free reuse while dragging in both directions. Resize submission/readiness is unchanged.

Maximize/restore use a separate `maximize_content` fade (50 ms entry, 130 ms reveal by default).
They fade at the starting geometry before moving the placeholder. Movement retains the restored
frame's physical corner radii rather than scaling them with a snapshot; maximized square corners
take effect at the destination. A ready image arriving during movement stays hidden until movement
finishes. A late image leaves a stationary placeholder without animation wakeups. Both directions
continue to use the existing Wayland final-configure/publication gate and Xwayland synchronized
configure/redraw gate. The final size is submitted early, allowing the app to redraw concurrently;
animation ticks never send intermediate sizes.
During staged handoffs and manual resize, the snapshot group is clipped to the outer window and
an independent analytic shadow follows the displayed outer bounds. Blur, spread, and offset stay
fixed in physical pixels, with an inverse rounded clip keeping shadow paint outside the window.
Maximize movement retains the starting shadow; restore uses the destination shadow. At the final
maximized geometry the maximized style takes effect (normally no shadow). The shadow is stacked
immediately before its window, and opacity follows the window's visibility. Normal resting
composition resumes on the final fully damaged frame without double-painting the shadow.
This reuses `DesktopLayer::frame_shadow` and `DesktopComposition` with a separate retained scene
identity; there are no new shadow textures, shaders, or GPU synchronization mechanisms. CPU tests
cover expansion, restoration, manual growth/shrink, fixed shadow reach, inverse clipping, stack
order, and image-free analytic updates. The earlier exterior-shadow reference note in
`WAYLAND_RESIZE_PREVIEW.md` describes the reused rendering contract. Scaling a captured shadow
was rejected because it stretches blur and offset; leaving the final-size shadow under a moving
window was rejected because it detaches from the visible outline. Live appearance remains user-run.
Completion removes this temporary clip before emitting the final, fully damaged animation frame.
Previously it cleared the handoff only after emitting that frame, so the next partial hover redraw
could expose the shadow without invalidating its pixels in every scanout slot. The CPU regression
failed with the old ordering and now verifies an unclipped completion placement, full damage, and
unchanged clipping on the following hover update. This fixes a neutral presentation-state ordering
bug using existing damage-history contracts; it changes no GPU resource or synchronization mechanism.
The adjacent reference library remains unavailable; no new external review is claimed for this
bounded correction. Live confirmation of the reported restore/hover flicker is still user-run.

Entry mixes use the starting extent instead of the maximized extent. Pure movement reuses the
placeholder content capture without allocating a mix target per tick. Glass optics use the live
resolve described in [Resize-preview glass](RESIZE_GLASS.md). Vulkan output scenes retain stable
per-window identity and existing buffers/bindings; unchanged sampled outputs keep their epochs. Measured target corrections preserve the remaining animation deadline instead of
restarting a full-duration tween. These remove identifiable redundant work, but the reported live
hitch still needs hardware timing/visual confirmation. This uses existing scene-delta validation,
materialized-image pins, and buffer update contracts; no allocator, shader, or synchronization
mechanism is added. The prior reference review applies. Repeated fresh output scene allocation,
scaled corner radii, and early-ready interruption of movement were rejected. CPU regressions cover
entry timing, fixed radii, capture-free movement, early/late redraws, and both animation directions.

The resize placeholder covers the complete outer window, including its title bar and border area,
clipped to the frame's outer rounded contour. Chrome and client pixels are hidden underneath;
the separate exterior shadow remains. Both fade endpoints therefore represent a whole window.
The existing final-configure/image-publication gate still controls the return from the placeholder:
mouse release alone never reveals old content. The same rule applies with animations disabled.

Motion-enabled windows are composed into immutable per-window snapshots, including their chrome,
content, shadow and associated surfaces. Vulkan glass is excluded from those pixels and retained
as a weighted optical recipe, resolved against the desktop at the current animated geometry. A source capture is reused when its local placements and
source scenes do not change. Translation alone reuses captured pixels. Content transitions blend
composed endpoints; a new handoff uses the last sampled mix as its starting image. This keeps
interruptions bounded rather than retaining an ever-growing tree of earlier transitions.

The initial implementation scales snapshots during geometry transitions, including briefly scaling
text and borders. Interactive geometry follows pointer updates; the outgoing image can scale during
the short placeholder-entry fade. This refines the original plan's proposed live-chrome layout:
per-tick chrome relayout and native-extent outgoing content are not implemented. At rest rendering
returns to native-size sampling. The API leaves room to improve this rendering strategy later.

A minimize exit uses centered scaling to 92% plus group opacity. Its semantic/input eligibility ends
immediately, while its visual finishes at its retained stack position. Restoration can reverse from
current opacity. Completed exits release snapshots. Unmap, leaving the output, fullscreen, or session
lock removes the old animation state. Dock destinations are not implemented.

Hit testing inverse-maps displayed window geometry, and client input is suppressed during geometry,
visibility, and content handoffs. New source frames may be rendered into a capture before they
contribute visible pixels; zero-weight endpoints are withheld from presentation feedback while
remaining eligible for GPU materialization. Replaying an old snapshot does not report a newer client
revision as presented. Existing hidden-resize callback pacing remains separate.

## Rendering and storage

Both renderers consume the same snapshot commands. Crossfades add complementary premultiplied RGBA
contributions into a transparent intermediate, then source-over the resulting window onto the
desktop. `BlendMode::Add` supplies that isolated accumulation. This avoids desktop leakage between
two opaque endpoints and avoids fading overlapping chrome/content primitives independently.

Vulkan uses the existing sRGB `VulkanMaterializationTarget`, image binding, and composite recording
paths. Capture, mix, and output execute in the normal frame submission. Each destination is pinned
in that submission; sampled images retain their own pins. The existing color-attachment-to-sampled
barriers apply between passes. No client DMA-BUF lease is extended for animation, and Vulkan never
uses CPU readback or the software renderer to create its snapshots.

Only targets with matching extent and no remaining scene/submission pins can be reused. The spare
pool is limited to 16 targets and 64 MiB. Admission estimates six RGBA snapshots for transitioning
families and two for stationary families, with a
64-million-pixel aggregate limit and a 16-million-pixel per-image limit; oversized groups use direct
presentation. Actual allocator overhead and other compositor resources remain separately budgeted.
Admission prioritizes state changes and running transitions over idle windows, independently of
surface ID or backend. Previously reserving six images for every idle window in ID order could
exclude later-created windows (including Xwayland clients) before their transition started. The
CPU regression covers a later surface maximizing alongside a large idle window. This is a
presentation admission estimate, not a hard bound on all in-flight GPU memory. Resource allocation,
retirement, and synchronization primitives are unchanged.
Animation-target allocation failure disables motion for the session and restores ordinary output
placements and input mapping; device/render errors retain existing host error handling.

Window captures add rendering/storage work for enabled windows. Intermediate target allocation can
occur on the owner when no safe spare exists. This initial implementation does not claim a latency
or memory improvement. Active transitions conservatively damage the full output; settled windows
retain ordinary damage tracking. The final sample is requested even after a timer expires, then
animation redraw requests stop. Waiting for client content alone does not run an animation loop.

## Reference review and validation

The adjacent reference library was unavailable; upstream source was inspected instead:

- Flutter `engine/src/flutter/flow/layers/opacity_layer.cc` on `master`: `Diff`, `Preroll`, and
  `Paint` distinguish subtree opacity inheritance, cached composition, and transform/opacity damage.
  [Source](https://github.com/flutter/flutter/blob/master/engine/src/flutter/flow/layers/opacity_layer.cc).
- Qt `src/quick/scenegraph/coreapi/qsgnode.cpp` on `dev`: `markDirty`, removal propagation, and
  `setOpaqueMaterial` separate opacity/matrix invalidation from geometry and opaque-material use.
  [Source](https://github.com/qt/qtdeclarative/blob/dev/src/quick/scenegraph/coreapi/qsgnode.cpp).

These were source snapshots retrieved during this task, not pinned dependency revisions. No code
was copied. Their distinction between inherited opacity and isolated composition led to explicit
window snapshots rather than per-child fades. Qt's invalidation rules and Flutter's old paint-region
tracking informed the final-frame and opacity-only damage checks.

Official contracts checked: [Vulkan blending](https://docs.vulkan.org/spec/latest/chapters/framebuffer.html#framebuffer-blending)
and [synchronization/access scopes](https://docs.vulkan.org/spec/latest/chapters/synchronization.html),
especially color attachment writes followed by fragment sampled reads. Existing Telorgon
`VulkanMaterializationTarget::{target,can_recycle}`, `VulkanScene::bind_materialized_image`, composite
recording and frame image pins provide the resource-lifetime mechanism; no new unsafe GPU boundary
or queue is introduced.

CPU coverage includes paired override precedence, finite tracks, early-ready interruption, opaque
crossfades without background leakage, transparent-placeholder endpoints, group opacity with
occluding children, minimize reversal and retirement, geometry/input mapping, reduced motion, and
final-frame scheduling. Existing configure/readiness tests continue to cover the protocol gates.
The public frame fixture exercises const builders and template motion export. No compositor or
hardware-presenting application was launched by the agent.

User-run checks: repeat maximize/restore during motion; minimize and reactivate quickly; resize from
all edges with a delayed client; test preview alpha 0/128/255, transparent clients, subsurfaces and
popups, Wayland and X11, and reduced motion. Watch for clipping, transient text scaling, stacking,
input alignment, and capture-allocation latency on the target GPU.

Validation for this implementation:

- Library tests with `--no-default-features --features desktop-xwayland -- --test-threads=1`:
  1,271 passed, two ignored. The sandbox initially denied socket fixtures; the complete suite
  passed when run with socket access.
- `window_frame_api` integration fixture: 12 passed.
- Test compositor checked both the ordinary desktop configuration and the embedded-Xwayland
  configuration using the real local payload and offline dependencies.
- Core library checked without default features; formatting and document links checked.

These are CPU/build results, not Vulkan visual or performance qualification.

The first user-run compositor log exposed snapshot scene construction applying its draw delta
before binding materialized images. Vulkan's retained-scene validator correctly rejected the
missing resource. Snapshot output and mix scenes now bind all inputs before applying the delta,
following the existing desktop DMA-BUF preparation order. This is a correction to use of the
existing binding contract; allocation, barriers, and image lifetimes are unchanged. Disabling
validation or adding dummy uploaded images was rejected. An ignored Vulkan regression exercises
both output and two-input mix scene construction; it is compile-checked, pending a user-run
hardware test. The reference review above remains applicable to the unchanged GPU mechanism.

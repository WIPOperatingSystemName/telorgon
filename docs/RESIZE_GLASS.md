# Resize-preview glass

Status: implemented Vulkan path with portable tests and compile integration; visual quality,
Vulkan validation on hardware, timing, and battery impact require user-run qualification.

## API

`Fill` is the shared preview material: `None`, `Color(ColorRgba8)`, or `Glass(GlassStyle)`.
It lives in `telorgon::fill` and is exported by `telorgon::app::*`. Ordinary widget
`Background` remains separate; arbitrary widget glass is not implemented by this refactor.

`LinuxShellConfig::resize_preview` takes `ResizePreviewDesign { fill, border }`.
The optional override in `WindowChromeDesign` and `WindowContentStyle` inherits the
complete host design when `None`. Borders use `Border` with finite, nonnegative logical
widths, paint inward over the fill, and follow the window's outer corner contour without
changing layout or hit regions. Fill and border fade together.

```rust,ignore
resize_preview: Some(ResizePreviewDesign {
    fill: Fill::Glass(GlassStyle {
        tint: ColorRgba8::rgba(23, 27, 37, 32),
        blur_radius: 4.0,
        ..GlassStyle::liquid()
    }),
    border: Border::all(1.0, ColorRgba8::rgba(160, 200, 255, 220)),
}),
```

Migration: replace `ResizePreview::Color/Glass` with `Fill::Color/Glass`. For resize
settings, wrap the fill in `ResizePreviewDesign::new(fill)` to preserve the previous
borderless appearance. Tile previews accept the fill directly. `Fill::None` paints no
interior but still permits a preview border; it does not disable resize readiness handling.
`ResizePreviewDesign::default()` is empty; the default host design keeps its opaque slate fill.

Color alpha controls transparency. Glass alpha controls **tint strength**; the result is opaque
before rounded-edge coverage, even with zero tint alpha. The software renderer and Vulkan
allocation/budget fallback use the flat tint. The default host appearance remains opaque slate.
The test compositor opts into glass.

`GlassStyle::liquid()` (also the default) provides a mild blur, a smooth lens matched to the rounded window outline,
RGB dispersion and Fresnel reflection. Distances use logical pixels and scale once for output
density. `blur_radius` is a Gaussian diameter (sigma is half the value), filtered at native resolution for small kernels and half resolution for broad kernels (radius ≥ 8 physical pixels).
0 disables filtering, finite logical values clamp to 0–64 and non-finite values become 4.
`bevel_width` clamps to 1–128, `refraction` to 0–64 and `dispersion` to 0–4. Zero dispersion
uses one sharp rim sample instead of three. `fresnel` clamps to 0–1. Directional rim lighting and specular controls are removed. The shader does not
add a shadow. Optical parameters are normalized before caching. `bevel_width` controls the smooth
inward falloff measured from the actual rounded window outline. A separate smoothed direction
field blends adjacent edges, including when the band exceeds the corner radius. The
old thin highlight stroke is removed so rim light blends across the lens profile. The squared
quintic edge falloff concentrates bending near the rim and smoothly returns to the undistorted
blurred interior. The test compositor uses an 8-pixel band, refraction 10, dispersion 0.2, and
full-surface blur 10; these controls use logical pixels.
A static four-degree twist at the outer rim adds subtle swirl, fading inward without changing
refraction strength. Tune the offline `EDGE_TWIST_TANGENT` constant in the liquid fragment shader
(zero disables, negative reverses), then regenerate the shader bundle. It adds no texture samples
or animation-driven redraws.

Use `blend_softness: 16.0` to soften the inner boundary independently of `bevel_width`.
It adds a gentle tail into the center while keeping the main rim strong. `0.0` (the default)
restores the previous edge-only profile; values clamp to 0–128 logical pixels. The tail extent
is capped by the smaller window half-size. Increasing it does not change backdrop blur or require
extra texture samples. The test compositor explicitly enables 16.

For the cheapest liquid preset use `GlassStyle { blur_radius: 0.0, dispersion: 0.0,
..GlassStyle::liquid() }`. Flat `Fill::Color` remains cheaper still.
See [Liquid-glass analysis](LIQUID_GLASS.md) for equations, reference comparisons, packed parameters,
source provenance, exact versus approximate optimizations, and remaining performance limits.

## Rendering and work

The Vulkan desktop captures ordered layers below each veil, excluding its own shadow.
Independent previews capture a screen-clipped rectangle padded for Gaussian support, refraction,
dispersion and reconstruction; outward 64-pixel quantization reduces allocation churn. Only
placements intersecting that region are dependencies. Participating tiles in a coordinated resize
instead share a stable full-output desktop beneath the group, grouped by blur radius. They do not
refract one another's animated previews. Membership persists through the content-return fade.

Sharp refraction remains native resolution. Blur radii below 8 physical pixels use two native
Gaussian passes. Broader kernels use a linear prefilter followed by horizontal and vertical
half-resolution passes; per-axis sigma scales with actual texture dimensions, including odd sizes.
This is a visual approximation, not pixel-equivalent to the native filter. Rounded coverage,
borders and optical displacement remain native resolution. See [optimization audit](GLASS_PIPELINE_OPTIMIZATION.md).

Cache signatures include output extent, capture region, normalized blur radius, intersecting lower
placements, scene epochs and lower glass revisions. Radius-only changes reuse the sharp capture;
tint/optics changes reuse prepared pixels. Geometry reuses pixels while the capture region remains
unchanged. Lower-source changes recapture and refilter. Filter scenes and bindings are retained.
One RGBA8 target is needed without blur; small kernels need three native targets; broad kernels
need one native plus three half-resolution targets. The existing 64–512 MiB admission limit and
flat-tint allocation fallback remain. Idle glass adds no frames.

Window motion captures now exclude glass pixels. A small optical recipe travels alongside each
immutable content snapshot and its weighted mixes. Maximize/restore, early-ready holding, and
minimizing an active preview or reveal resolve that recipe at the **current displayed position and
size**. The background is not a cropped image stretched with the animation. Physical bevel/radius
controls remain independent of snapshot scaling.

Resolve windows from bottom to top, using lower windows' already animated placements as backdrop
inputs. A window-sized intermediate adds the weighted content and live glass in premultiplied linear
color; the final draw applies visibility once. This preserves the existing content fade algebra,
including interrupted fades, without applying source-over separately to each weighted endpoint.
Glass-only snapshots use a 1x1 transparent body. A fully weighted single lens with no body draws
directly into the resolve, avoiding the body copy and bordered weighting intermediate.
Rounded placement coverage promotes only opaque draws to source-over; additive draws keep addition.
The motion resolve itself does not change the shader interface or GPU ABI.

A resolve is reused while content, optical recipe, displayed placement and lower sources match.
Lower Motion scene epochs now stay stable on unchanged frames. Native lens buffers and image
bindings survive resize; resolve and bordered targets use 64-pixel capacity buckets with shrink hysteresis. Sampling
crops the active rectangle rather than stretching padding; admission charges retained capacity.
The existing completion-safe spare pool is preserved. Resolved live-glass targets have an output-sized estimated pixel budget: four full-output
RGBA8 surfaces, clamped to 64–256 MiB, separate from the 64–512 MiB backdrop budget and existing snapshot/spare budgets. In-flight pins may temporarily
exceed those owner budgets. At most 16 distinct glass endpoints are admitted per motion snapshot;
resource exhaustion takes the existing immediate-presentation fallback. This correction adds GPU
composition work during glass motion; it is not a claim of unchanged GPU time or power.

## Engineering audit

The new offline liquid shader uses GPU ABI 4.4 and the existing four descriptor sets, materialization
ownership, synchronization and submission mechanisms. Reference review, mathematical derivation
and packed parameter contract are documented in [LIQUID_GLASS.md](LIQUID_GLASS.md).

Inspected local paths:

- `src/renderer_vulkan/target.rs`: filterable/blendable sRGB target checks, initialized layout,
  resource ownership and completion pins.
- `src/renderer_vulkan/scene.rs`: materialized-image binding and retained sampled-image ownership.
- `src/renderer_vulkan/composite.rs`: attachment transitions, sampled image pins, pass recording.
- `src/application_host/shell_wayland/renderer/vulkan/motion.rs`: snapshot recording and sampling.
- `src/application_host/shell_wayland/scene.rs`: placement damage and one-time output scaling.
- `crates/telorgon-shader-build/shaders/vulkan/box/image.frag`: linear sampled color and clipping.

Paths starting with `src/` are relative to `crates/telorgon/`.
The official [Vulkan synchronization examples](https://docs.vulkan.org/guide/latest/synchronization_examples.html)
and [sampler specification](https://docs.vulkan.org/spec/latest/chapters/samplers.html) were checked.
Adopted invariants: never sample the active destination; attachment writes become visible to
fragment sampling; preserve read-to-write ordering when reusing a target; pin every destination
and sampled image through submission completion; filter/tint in linear color using sRGB targets;
keep immutable motion captures independent of subsequent backdrop writes.

Rejected alternatives: CPU readback/blur/upload; full-resolution neighborhood blur per glass pixel;
a single poorly filtered 1/16 reduction; continuously rebuilding unchanged backdrops; introducing a
second native GPU resource owner. Portable tests cover filtered reduction extents, tiny outputs,
invalid radius bounds, shadow exclusion, live motion-coordinate/scissor mapping, API propagation,
output scaling, color/glass switching, idle suppression and lower-versus-upper damage.

## User-run qualification

Rebuild and run the test compositor normally. Check a patterned wallpaper and overlapping windows;
resize from every edge and corner, including partially offscreen positions. Verify the background
stays aligned, the window's own content/shadow is absent, rounded edges remain clean, and moving or
updating lower windows refreshes glass. Check fractional output scales, maximize/restore and
resize-content fades, multiple windows awaiting resize completion, and switching back to Color.
Measure complete-frame GPU time and power for idle, active resizing, and video behind the preview.
No battery-life or hardware performance result is claimed by compilation or portable tests.

## Validation

- Test compositor `cargo check --offline`: passed.
- Outline-matched lens with tight edge falloff and static twist: 21 glass unit tests, two compiled-shader contract tests, and the
  packaged-bundle hash test passed. Two glass hardware fixtures compiled but were not run.
- Desktop and Vulkan renderer unit suites for the motion correction: 224 passed. Four hardware
  fixtures and the opt-in CPU sampler benchmark compile but remain ignored. Two existing private-socket fixtures required local socket
  access outside the sandbox and passed on rerun.
- Window frame public API suite: 13 passed.
- Offline shader builder: SPIR-V compilation, validation and interface reflection passed;
  two compiled-shader contract tests passed.

CPU tests cover equal insets on independently constructed corner arcs and straight edges,
continuous bending through diagonal joins and asymmetric axes, finite bounded refraction,
finite defaults, exact output scaling,
single-lens upload payloads, atomic material
validation, stacked-glass invalidation, backdrop filtering and placement/motion behavior. The
offline shader builder validates SPIR-V and reflects the new bindings. Its tests inspect the
compiled sampling instructions and reject a corrupted binding.

The ignored `glass_records_reuses_and_refreshes_real_gpu_targets` fixture compiles. It records the
real lens draw and backdrop reuse/refresh path, then drops host owners before completion. It was
not executed. No GUI, compositor session, or GPU hardware test was launched.

The motion correction adds CPU regressions for a fixed desktop pixel's UVs through resize,
current physical corner radii and shadow padding, glass-free captures, interrupted mix weights, and early-ready
maximize followed by minimize with no protocol veil metadata. The ignored
`live_motion_glass_keeps_stripes_aligned_and_refreshes_without_recapture` hardware fixture records
real captures/resolves and reads an offscreen result: it checks a stationary stripe boundary while
moving/resizing, backdrop refresh, visible tint, and idle cache/scene-epoch reuse. It was compiled,
not executed.

Reference audit for the correction: the adjacent library is still absent. Reviewed Flutter's
[`BackdropFilterLayer::Diff/Paint`](https://github.com/flutter/engine/blob/main/flow/layers/backdrop_filter_layer.cc)
(screen-coordinate filter inputs and backdrop dependencies) and Qt's
[`QQuickShaderEffectSource`](https://github.com/qt/qtdeclarative/blob/dev/src/quick/items/qquickshadereffectsource.cpp)
(cached source textures, live invalidation, grouped opacity, render-thread cleanup). These were
read on their moving upstream branches; no code was copied. The
[Vulkan framebuffer blending contract](https://docs.vulkan.org/spec/latest/chapters/framebuffer.html)
was checked for weighted addition and premultiplied source-over. Local target/scene/composite
ownership and queue ordering remain as audited above. Rejected alternatives: freezing glass inside
window captures, recapturing hidden client content while waiting for readiness, applying visibility
twice, and allocating new lens buffers every animation tick.

## Shared-fill and preview-border refactor audit

The adjacent `../other-rendering-libs` directory was unavailable in this checkout; no
external implementation review is claimed. This refactor reuses the existing analytic box,
retained scene, glass cache, and motion materialization paths. Inspected local sources:
`src/application_host/shell_wayland/{scene,layers,motion}.rs`,
`src/application_host/shell_wayland/renderer/vulkan/{motion,motion_glass,glass}.rs`, and
`src/renderer_vulkan/pipeline.rs` (paths relative to `crates/telorgon`).
The [Vulkan framebuffer blending specification](https://docs.vulkan.org/spec/latest/chapters/framebuffer.html)
was checked for source/destination blend semantics.

Invariant: a border uses source-over on its own glass endpoint **before** weighted endpoint
addition. Capturing the border in the ordinary body and adding glass over it was rejected:
that would brighten the border and produce incorrect alpha during interrupted fades.
Bordered optical endpoints use a reusable intermediate charged to the output-sized
motion-resolve budget; allocation failure preserves immediate-presentation fallback.
Borders travel with immutable optical recipes through mixes, while the backdrop stays live.
No shader ABI, synchronization, or external-image ownership changes are introduced.

Headless coverage checks transparent/color/fallback interiors, inward border placement,
public fill sharing, invalid widths, border recipe retention through interrupted fades,
and preservation of borders in ordinary fallback captures. Live Vulkan visual qualification
remains user-run.

Validation for this refactor: 199 shell tests, 15 public API tests, and 10 easy-frame
tests passed; host border-validation tests and the downstream compositor compile check
also passed. Five hardware fixtures (including the bordered-glass fade readback fixture)
compiled but were not run. The broader shell filter additionally finds an existing
`neutral_desktop_modules_do_not_depend_on_a_renderer` failure: a pre-existing test fixture
in `scene.rs` names `renderer_vulkan`. That unrelated fixture was left unchanged.

## Motion-budget recovery

Bordered glass at 3840×2400 needs about 70.3 MiB for its resolve plus border intermediate,
which exceeded the former fixed 64 MiB budget. Admission now includes all live resolves and
border intermediates, with a budget of four physical-output RGBA8 surfaces (64 MiB floor,
256 MiB ceiling). Saturating arithmetic prevents oversized estimates from wrapping.
Snapshot/spare and backdrop budgets remain separate; completion pins may temporarily retain
retired allocations beyond live-owner accounting.

A budget or target-allocation fallback affects the current transition. The renderer presents
ordinary layers with full damage, drops motion snapshot/effect owners, and reports the failure
to the host. The host acknowledges it, clears obsolete input transforms and captures, and keeps
logical window geometry, monotonically increasing snapshot IDs, and retained shadow epochs.
The next transition can therefore animate even if no idle redraw occurs first. Surface revisions
hidden only by the discarded animation are credited to the actual fallback presentation.

Reference audit: the adjacent source library remains unavailable. Local paths inspected were
`renderer/vulkan/{motion,motion_glass}.rs`, `renderer/vulkan.rs`, `motion.rs`, and the shell host
under `src/application_host/shell_wayland`, plus `src/renderer_vulkan/{frame,composite}.rs`.
The [Vulkan memory specification](https://docs.vulkan.org/spec/latest/chapters/memory.html)
was consulted. This change retains existing submission-completion pins and target retirement;
it does not change synchronization or treat device loss as recoverable allocation pressure.
Rejected alternatives: unbounded budget increases, permanently disabling effects after one
failure, and resetting scene/snapshot identities during recovery. Regression coverage checks
native 4K admission, multiple border endpoints, the upper cap/overflow, and immediate restore
then maximize following capture reset. Hardware presentation remains unqualified.

### Four-quadrant backdrop admission

A user-run 3840×2400 log showed repeated snapshot/effect fallback while resizing quadrants.
The old 256 MiB backdrop cap admitted only two full-output, three-target glass caches.
The budget now scales to four such caches plus 25% allocation headroom, with a
64 MiB floor and 512 MiB hard ceiling.
At that output four previews require about 422 MiB of backdrop target pixels (previously
only about 211 MiB for two); snapshots, resolves, allocator overhead and in-flight pins are
additional. This earlier admission correction preserved native sharp and blurred samples. The later
[shared/reduced pipeline](GLASS_PIPELINE_OPTIMIZATION.md) reduces their actual cost. Outputs exceeding the cap still use the existing flat fallback.

Motion now retires cache owners for no-longer-referenced lenses before admitting replacements;
existing submission image pins continue to protect in-flight resources. No allocation or barrier
primitive changes. The existing reference audit applies; adjacent reference sources remain absent.
CPU tests verify four-lens admission, the hard ceiling, overflow rejection, and quadrant divider
membership. Hardware appearance and frame-time improvement require a user-run retry.

The subsequent profiling log contained 207 generic motion fallbacks. It also exposed an
admission-test gap: existing caches are charged by Vulkan memory requirements, while new-cache
estimates use raw RGBA pixels. An exact four-lens budget rejects the fourth when earlier images
include padding. The same 512 MiB hard cap now includes output-scaled 25% headroom. The regression
charges padding on existing targets. Individual budget/allocation/optical-endpoint failures now
identify their stage and relevant sizes in the log; the previous log cannot distinguish these
causes, so confirming this user's precise failure and GPU frame times still requires a live run.

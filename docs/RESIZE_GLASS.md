# Resize-preview glass

Status: implemented Vulkan path with portable tests and compile integration; visual quality,
Vulkan validation on hardware, timing, and battery impact require user-run qualification.

## API

`LinuxDesktopConfig::resize_preview` takes `ResizePreview`. The optional override in
`WindowChromeDesign` and `WindowContentStyle` inherits that host setting when `None`.
This replaces `resize_preview_color`; migrate existing colors by wrapping them in
`ResizePreview::Color(color)`.

```rust,ignore
resize_preview: Some(ResizePreview::Glass(GlassStyle {
    tint: ColorRgba8::rgba(23, 27, 37, 32),
    blur_radius: 4.0,
    ..GlassStyle::liquid()
})),
```

Use `Some(ResizePreview::Color(ColorRgba8::rgba(23, 27, 37, 150)))` for the old behavior.
Color alpha controls transparency. Glass alpha controls **tint strength**; the result is opaque
before rounded-edge coverage, even with zero tint alpha. The software renderer and Vulkan
allocation/budget fallback use the flat tint. The default host appearance remains opaque slate.
The test compositor opts into glass.

`GlassStyle::liquid()` (also the default) provides a mild blur, a smooth lens matched to the rounded window outline,
RGB dispersion and Fresnel reflection. Distances use logical pixels and scale once for output
density. `blur_radius` is a Gaussian diameter (sigma is half the value), filtered at full resolution.
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
..GlassStyle::liquid() }`. Flat `ResizePreview::Color` remains cheaper still.
See [Liquid-glass analysis](LIQUID_GLASS.md) for equations, reference comparisons, packed parameters,
source provenance, exact versus approximate optimizations, and remaining performance limits.

## Rendering and work

The Vulkan desktop captures the ordered layers below each veil into a full-output opaque target,
excluding the resizing window's own shadow. The window's hidden client/chrome and higher layers
are absent. Two full-resolution Gaussian passes produce the blurred interior source. The liquid
lens blends toward the original sharp capture at its refracted rim, then applies tint and Fresnel reflection.
Sampling uses desktop coordinates through the original scissor and rounded contour.

The cache key includes output extent, exact normalized blur radius, lower placements, lower scene
epochs and lower glass backdrop revisions. Radius-only changes rerun filtering without recapture;
tint, optics and lens movement reuse prepared pixels. Lower-source changes recapture and refilter.
Targets remain full resolution and are reused while their extent/count match. Idle glass adds no
frames. Three RGBA8 targets are needed with blur, one without. The 256 MiB live-pixel budget
and flat-tint allocation fallback remain. This costs more GPU work and memory than downsampling.

Window motion captures now exclude glass pixels. A small optical recipe travels alongside each
immutable content snapshot and its weighted mixes. Maximize/restore, early-ready holding, and
minimizing an active preview or reveal resolve that recipe at the **current displayed position and
size**. The background is not a cropped image stretched with the animation. Physical bevel/radius
controls remain independent of snapshot scaling.

Resolve windows from bottom to top, using lower windows' already animated placements as backdrop
inputs. A window-sized intermediate adds the weighted content and live glass in premultiplied linear
color; the final draw applies visibility once. This preserves the existing content fade algebra,
including interrupted fades, without applying source-over separately to each weighted endpoint.
Rounded placement coverage promotes only opaque draws to source-over; additive draws keep addition.
The motion resolve itself does not change the shader interface or GPU ABI.

A resolve is reused while content, optical recipe, displayed placement and lower sources match.
Lower Motion scene epochs now stay stable on unchanged frames. Native lens buffers and image
bindings survive resize; the window-sized target changes size when necessary and uses the existing
spare pool. Resolved live-glass targets have an additional 64 MiB estimated pixel budget, separate
from the 256 MiB backdrop budget and existing snapshot/spare budgets. In-flight pins may temporarily
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
- `src/application_host/desktop_wayland/renderer/vulkan/motion.rs`: snapshot recording and sampling.
- `src/application_host/desktop_wayland/scene.rs`: placement damage and one-time output scaling.
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

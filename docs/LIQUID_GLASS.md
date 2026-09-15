# Liquid glass: implementation and optimization audit

Status: implemented for Vulkan resize previews. CPU regression tests, offline shader validation,
and compile checks are available. GPU appearance, validation-layer execution, timing and battery
impact are not yet qualified. See [Resize-preview glass](RESIZE_GLASS.md) for configuration and limits.

## Reference study

Studied `ybouane/liquidglass` at revision `00aafe50202e916951d6f30d49afa1197ca236a7`:

- [`src/shaders.ts`](https://github.com/ybouane/liquidglass/blob/00aafe50202e916951d6f30d49afa1197ca236a7/src/shaders.ts):
  rounded-rectangle distance, circular bevel, finite-difference normals, refraction, dispersion,
  sharp/blur mixing, highlights and shadow.
- [`src/GlassRenderer.ts`](https://github.com/ybouane/liquidglass/blob/00aafe50202e916951d6f30d49afa1197ca236a7/src/GlassRenderer.ts):
  canvas upload, FBO allocation, blur passes, individual uniform updates and binding lifetime.

The glass interior evaluates five shape distances and five bevel heights to estimate its normal.
It samples sharp and blurred RGB separately: six texture instructions. Positive blur uses six
horizontal/vertical iterations with nine taps per pass, or 108 filtering samples per blur pixel,
plus copies. Those targets use the full **padded panel** size, not the desktop size. Its biconvex
mode combines height-gradient displacements rather than tracing a ray through a closed volume.
These are source-level counts, not measured GPU instructions or frame times.

The package declares MIT licensing. This change studies its design and writes an original Telorgon
lens implementation; it does not vendor the project or copy its shader. Shared clipping and ABI
boilerplate comes from Telorgon's own existing shaders.

The required adjacent `../other-rendering-libs` directory is absent. A second independent source
review used [`egui-wgpu/src/renderer.rs`](https://github.com/emilk/egui/blob/master/crates/egui-wgpu/src/renderer.rs),
particularly `UniformBuffer`, `Texture`, `Renderer::update_buffers` and the sampler map. It compares
packed uniform contents before uploading, and retains texture bindings and samplers. The adopted
invariants are to separate parameter changes from texture ownership, reuse bindings, and avoid
uploads for unchanged state. No egui abstraction or dependency was added.

## Outline-matched lens (current)

The optical band now follows the actual rounded window outline and its four corner radii.
The former fixed squircle field remains a design reference, not the shape used for falloff.
Direction smoothing is separate from band geometry to avoid the original nearest-edge normal jump.

Reviewed OverShifted/LiquidGlass on `master` on 2026-09-14:
[`BatchRenderer2D.glsl`](https://github.com/OverShifted/LiquidGlass/blob/master/assets/shaders/BatchRenderer2D.glsl),
[`Blur.glsl`](https://github.com/OverShifted/LiquidGlass/blob/master/assets/shaders/Blur.glsl),
[`BlurPass.h`](https://github.com/OverShifted/LiquidGlass/blob/master/src/BlurPass.h), and
[`LiquidGlass.cpp`](https://github.com/OverShifted/LiquidGlass/blob/master/src/LiquidGlass.cpp).
Its shader approximates distance with an implicit superellipse divided by its gradient magnitude,
then scales coordinates about the panel center. It uses adjustable powers, exponential falloff,
noise and glow; the final lens has one texture sample. Its blur uses repeated horizontal/vertical
passes, seven texture instructions per pass, invoked from the update path. The project has an
[MIT license](https://github.com/OverShifted/LiquidGlass/blob/master/LICENSE), copyright 2026
Sepehr Kalanaki. This is an original implementation of the mathematical approach, without vendored
code. The prior ybouane shader was rechecked as the independent contrasting implementation.

The shader uses the same rounded-rectangle distance construction as Telorgon's existing window
coverage. For centered pixel position `p`, half-size `a`, the applicable corner radius `r`, band
width `B`, and refraction strength `R`:

```text
q = abs(p) - a + r
s = length(max(q, 0)) + min(max(q.x, q.y), 0) - r
t = clamp(1 + s/B, 0, 1)
e = t*t*t*(t*(6*t - 15) + 10)
w = e*e
```

`w` is one at the actual outline and zero at depth `B`. Straight sides and circular corner arcs
share the same physical inset profile, including asymmetric radii. The outline and optical band
no longer require matching a separately chosen squircle. Physical units avoid aspect-dependent
band stretching. Existing corner-radius and band-width clamps are retained.

The squared quintic falloff concentrates bending at the rim and has zero first and second
endpoint derivatives. It replaces the cubic fade to soften the visible transition into the
undistorted, uniformly blurred interior. At half the band width its weight is 0.25; at three
quarters it is about 0.0107. The test compositor uses blur 10, band width 8, refraction 10 and
RGB separation 0.2 logical pixels. Blur remains a separate, full-surface cached backdrop operation;
no additional blur pass, texture, or interior distortion is introduced. This is tuning toward the
user-described appearance, not a claim to implement Apple's proprietary material.

### Independent inner blend softness

`GlassStyle::blend_softness` adds a low-strength tail beyond `bevel_width`, in logical pixels.
It defaults to zero for the previous profile; finite values clamp to 0–128 and non-finite values
become zero. The test compositor selects 16 without changing its other user-tuned values.

Let `B` be core width, `S` softness, and `H` the smaller window half-size. The vertex stage computes
`O=max(B,min(B+S,H))`, `inverse_outer=1/O`, and `alpha=0.25*(O-B)/O`.
For the squared quintic profile `P`, core weight `c=P(clamp(1-depth/B,0,1))` and tail weight
`l=P(clamp(1-depth/O,0,1))`, the combined weight is `c+(1-c)*alpha*l`.
It preserves full strength at the rim, returns to the old formula when softness is zero, and
stays bounded and monotonic inward. The tail is deliberately weaker than the main edge band.
The direction field uses outer width O too, so it does not disappear at the old core boundary.
Both squared quintic profiles join with zero endpoint derivatives. Tiny windows cap the outer
width to keep the center clear. Increasing softness therefore does not necessarily extend the
fade on windows whose core band already reaches the smaller half-size.

The new scalar is appended to the existing raw uint parameter array (four-byte word stride),
not a new std140 record or descriptor. ABI 4.2 updates both CPU word count and offline bundle.
The vertex shader prepares two flat values; no per-fragment division for outer width is needed.
CPU tests check positive tail displacement, unchanged rim strength, monotonicity, the old inner
join, density scaling, invalid values, and a softness-only 68-byte parameter update with zero
geometry/image uploads. Backdrop caching and motion recipes retain their existing ownership.
The prior raw-parameter/interface audit applies; no new resource or synchronization mechanism
is introduced. Offline shader reflection and bundle matching validate the changed interface.

Differentiating this exact SDF would reintroduce discontinuous nearest-edge normals on internal
medial axes. Instead, the direction field uses overlapping edge influences:

```text
r_optical = min(max(r/O, 1), min(a.x, a.y)/O)
v = max((abs(p)-a)/O + r_optical, 0)
v = v*v / (v + 0.25)
direction = sign(p) * v / sqrt(max(dot(v,v), 1e-8))
slope = direction * w
bend_pixels = -slope * R
```

The enlarged radius affects only direction, never the window mask or band distance. Squaring the
positive influence gives a zero derivative where each edge joins a corner, so the bend does not
jump from a horizontal axis to a vertical one. A guarded normalization handles the flat center.
The direction guard only attenuates negligible near-zero influences; the quintic falloff suppresses
the inner field where influences disappear. This is an artistic direction blend, not the gradient
of a physical concave surface. Exact distance itself still has non-unique derivatives on medial
axes; we claim continuous bending there, not globally continuous derivatives of the complete warp.

The CPU already supplies inverse bevel width, so calculations use bevel units without a new
per-fragment reciprocal for B. No powers, exponentials, finite-difference samples, new uniforms,
blur passes, or texture uploads are introduced. The four radius words and one flat radius varying
are used again within the 68-byte optics payload in ABI 4.2. Displacement stays bounded by `R` because
both direction magnitude and `w` are at most one. Refraction zero removes displacement.

A static edge twist rotates only refraction and RGB separation, leaving the lighting normal
unchanged. `EDGE_TWIST_TANGENT` in `liquid.frag` is `tan(4 degrees)` (0.06992681194).
For edge weight `w`, let `k = EDGE_TWIST_TANGENT*w`; rotate the slope with
`(slope + perpendicular(slope)*k) / sqrt(1+k*k)`. This is exactly a rotation by `atan(k)`:
it preserves the displacement bound, reaches four degrees at the rim, and fades to zero inward.
It uses one inverse square root and multiply/adds, without trigonometric instructions or extra
texture samples. Zero disables it; a negative value reverses it. This is an offline shader
constant, not a new `GlassStyle` field; changing it requires regenerating the shader bundle.
No timer, runtime parameter storage, or additional invalidation is introduced. A CPU regression
compares the expression to an independent sin/cos rotation across signed strengths, edge weights,
and zero vectors, checking direction and magnitude. The existing reference/sampling audit applies.

RGB separation uses the same smooth direction and falloff. Sampling remains one cached backdrop
lookup without dispersion, or three where dispersion is active. Explicit LOD zero remains inside
varying control flow, per the
[Khronos textureLod contract](https://github.com/KhronosGroup/OpenGL-Refpages/blob/main/gl4/textureLod.xml).
The clamped quintic is evaluated directly with multiplications; unlike the earlier cubic
[Khronos smoothstep](https://github.com/KhronosGroup/OpenGL-Refpages/blob/main/gl4/smoothstep.xml),
it also has zero second derivatives at the endpoints. Its square preserves those endpoint properties.
The existing lighting proxy, subdued rim profile without a narrow stroke, tint, backdrop caching,
and motion resolves are retained. GPU and battery costs remain unmeasured.

The reference audit above still applies: ybouane provides the contrasting rounded-distance /
height-normal approach and OverShifted the independent optical-direction approach. No upstream
code was copied. The adjacent source library remains absent; this change touches only the lens
math and restores an existing varying, without a new rendering or ownership mechanism.

CPU regressions construct circle points independently and check equal insets against straight
edges for four different corner radii; test continuity through diagonal ties and asymmetric axes;
sweep tiny, wide, square and zero-radius windows for bounded finite displacement; and verify
band width and fractional output scaling. Offline compilation validates the actual shaders,
descriptor/sample contracts and bundle hashes. Live appearance remains user-qualified.

## Backdrop work and its limits

Backdrop capture stays entirely on the GPU. The existing full-output capture and successive
half-size filtered reductions are retained. The former separate tint target/pass is removed;
tint now belongs to the final lens shader. Zero blur performs no reductions. The default logical
blur footprint of four produces one half-size reduction at scale 1.

For output area `A` and `L` reductions, their texture-instruction count is approximately:

```text
A * (1/4 + 1/16 + ... + 1/4^L) < A/3
```

This excludes the initial lower-layer composition, final lens draw, normal scene work, and motion
snapshots. Odd dimensions add rounding. Linear texture instructions can fetch multiple texels.
The filter is a low-cost pyramid, not an equivalent Gaussian; a single filtered backdrop also
forgoes separate sharp-rim/blur-interior mixing. Mild blur is intentional for clear liquid glass.

The full-output cache favors reuse while a panel moves. It can cost more than a padded panel crop
for a tiny preview over a frequently changing desktop. Every changed lower scene currently
invalidates the whole cache even if that change falls outside the visible lens. Multiple glass
panels may each rebuild a backdrop. Therefore the reference's local blur cost and this output-wide
cost cannot be turned into a valid whole-frame speedup ratio.

The cache excludes optics, lens position and size; it includes output extent, quantized pyramid
depth, lower placements, lower scene epochs and lower glass backdrop revisions. Tracking both
lower lens epochs and backdrop revisions prevents stale stacked glass after optics-only changes.
An unchanged lens skips CPU material reconstruction as well. Idle glass adds no timer or frame
requests. Targets retain the existing 96 MiB estimated live-pixel budget and in-flight ownership.

## Shader data and ABI

GPU ABI **4.2** retains material variant 3 and `PipelineKind::LiquidGlass` without enlarging the
64-byte material instance or adding descriptor sets. Set 2 binding 1 becomes visible to vertices
as well as fragments. The optics block is 17 `f32` words, packed as bits:

| Bytes | Values |
|---|---|
| 0–15 | Four actual window corner radii, TL/TR/BR/BL |
| 16–31 | Inverse output width/height, inverse bevel width, refraction pixels |
| 32–47 | Dispersion pixels, rim, Fresnel, specular |
| 48–63 | Premultiplied linear tint RGB, remaining transmission |
| 64–67 | Additional inner blend softness in physical pixels |

Geometry stays in the existing material record. The vertex shader reads the optical words and
passes flat values, including the window corner radii, to fragments. This removes optical storage-buffer fetches from the fragment
path, at the cost of flat varying/register traffic; profiling must confirm the tradeoff on target
GPUs. No dynamic indexing of sampled-image arrays, push constants, runtime compiler or new
per-property uniform calls are introduced. The existing sampled-image descriptor carries the
owned backdrop separately. Rebinding the same native image is an early return.

Portable retained-scene tests show the following **queued payloads for a single lens**, after its
initial upload is committed:

| Change | Geometry bytes | Optics bytes | CPU image upload |
|---|---:|---:|---:|
| Unchanged state | 0 | 0 | 0 |
| Move/resize with unchanged radii and bevel limit | 64 | 0 | 0 |
| Tint, optical strength, or blend softness only | 0 | 68 | 0 |

These are not total frame traffic: view data, staging alignment, draw metadata, descriptor work,
backdrop rendering and optional motion snapshots are additional. Resizing across a radius/bevel
clamp boundary updates optics too. Output scaling applies once to all pixel-distance controls.

Validation rejects missing/foreign raw external backdrops, non-finite parameters and mismatched
pipelines before mutating a scene. Offline reflection checks the descriptor pairs, view layout,
64-byte instance stride and prohibition on push constants. A compiled-SPIR-V test checks explicit
LOD sampling and rejects a deliberately misbound optics buffer. The descriptor change follows
the [Vulkan shader interface requirements](https://docs.vulkan.org/spec/latest/chapters/interfaces.html).

## Ownership, alternatives and qualification

Inspected Telorgon paths (relative to `crates/telorgon/src/`): `renderer_vulkan/{target,scene,
composite,sync,descriptor,executor}.rs` and `application_host/desktop_wayland/{scene.rs,
renderer/vulkan/glass.rs,renderer/vulkan/motion.rs}`. The existing buffer barrier already makes
storage reads visible to both shader stages. Backdrop passes never sample their destination and
keep destination/sampled-image pins through completion. Attachment-to-sampling and reuse ordering
remain with the existing materialization machinery, checked against the
[Vulkan synchronization examples](https://docs.vulkan.org/guide/latest/synchronization_examples.html)
and [sampler specification](https://docs.vulkan.org/spec/latest/chapters/samplers.html).

Rejected alternatives include CPU readback/upload, full-resolution neighborhood blur in each lens
fragment, retaining every historical panel size, continuous noise-driven redraw, and a second GPU
resource owner. Gaussian nine-to-five tap pairing is only an exact bilinear reduction for compatible
adjacent texel offsets/alignment; arbitrary spread-scaled taps do not justify that substitution.
A cropped/shared backdrop atlas is deferred because it changes invalidation and sampling coverage.

Before making a battery claim, measure whole-frame GPU time and system power on the target device
for Color versus liquid with dispersion 0/0.65 and blur 0/4/24, at idle, during resize, and with video
underneath. Include several simultaneous previews and fractional output scaling. Confirm clipping,
color, refraction direction, source refresh, and validation-layer cleanliness with the ignored
hardware fixture. The repository prohibits launching GUI/hardware sessions in this task, so those
checks remain user-run. See [RESIZE_GLASS.md](RESIZE_GLASS.md) for the live motion resolve: glass
now travels as a weighted optical recipe outside content snapshots and samples lower windows at
their displayed geometry. It adds a bounded window-sized resolve target during glass motion.

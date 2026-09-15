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

For wide bevels, the exact distance alone is insufficient: inside a corner's circle center,
`max(q.x,q.y)` has a diagonal first-derivative crease even though its values are continuous.
The optical falloff now rounds that interior maximum with a compact polynomial smooth maximum.
For both q components negative, let `D=-(q.x+q.y)`, `L=O/2`, and
`K=L*D/(L+D)*smoothstep(0,L,O-r-D/2)`. Add
`K/4 * max(1-abs(q.x-q.y)/K,0)^2` to the maximum when K is positive.
The width depends smoothly on the sum, not on the creased maximum. Its overlap vanishes before
reaching the circular arc or straight-edge joins and fades out at the inner band boundary.
Thus coverage, the actual rounded outline and its full-strength rim remain exact; deeper optical
insets intentionally deviate from exact distance to remove the diagonal crease. The direction
field remains separate. Both refraction strength and sharp/blur blending consume this smooth
falloff. This adds arithmetic but no texture reads, descriptors, targets or ABI changes.

The corner regression compares one-sided first derivatives of displacement and blend weight at
all four diagonals with the user's 52-pixel bevel and 64-pixel softness. Earlier value-continuity
tests could not detect this crease. Existing circular-outline, scaling, boundedness and clear-center
tests remain applicable. This is a local mathematical correction using the preceding SDF/reference
audit; it changes no Vulkan resource or synchronization contract. Live appearance remains unqualified.

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
join, density scaling, invalid values, and a softness-only 60-byte parameter update with zero
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

Backdrop capture stays entirely on the GPU at full output resolution. Positive blur uses two
same-resolution Gaussian passes, horizontal then vertical. Zero blur uses only the sharp capture.
`blur_radius` remains bounded to 0–64 logical pixels; after output scaling its physical diameter
is capped at 256 pixels. Gaussian sigma is half that diameter, with support truncated at three
sigma and normalized. This changes the appearance from the former approximate reduction blur.
Adjacent discrete taps are paired using weighted fractional offsets and linear sampling. The
kernel has at most 192 positive pairs: at most 385 texture instructions per pixel per pass.

The final lens samples the blurred image in its interior and blends toward the original sharp
capture using the existing rim falloff. The rim uses one additional sharp sample, or three for
RGB dispersion. Thus magnification no longer enlarges a reduced-resolution backdrop. Tint and
highlights remain in the final lens draw.

Three full-output RGBA8 targets cost approximately `12 * output_width * output_height` bytes
with positive blur, versus one target with zero blur. The 256 MiB estimated live-pixel
budget remains; allocation/budget failure uses the flat tint. In-flight pins can temporarily
exceed the owner budget. Full-resolution filtering costs more GPU work and memory, especially
with wide blur or multiple glass windows; no live GPU timing claim is made.

The cache includes output extent, exact normalized radius, lower placements, lower scene epochs
and lower glass backdrop revisions. Radius-only changes reuse the capture and targets and rerun
the filters. Lens position, size, tint and optics alone reuse the backdrop. Lower-source changes
recapture and refilter. Idle glass requests no extra frames. Both sharp and blurred images use
the existing completion pins, barriers and owned-image lifetime, including during window motion.

## Shader data and ABI

GPU ABI **4.4** retains material variant 3 and `PipelineKind::LiquidGlass` without enlarging the
64-byte material instance or adding descriptor sets. Set 2 binding 1 becomes visible to vertices
as well as fragments. The optics block is 15 `f32` words, packed as bits:

| Bytes | Values |
|---|---|
| 0–15 | Four actual window corner radii, TL/TR/BR/BL |
| 16–31 | Inverse output width/height, inverse bevel width, refraction pixels |
| 32–39 | Dispersion pixels, Fresnel |
| 40–55 | Premultiplied linear tint RGB, remaining transmission |
| 56–59 | Additional inner blend softness in physical pixels |

Geometry stays in the existing material record. The vertex shader reads the optical words and
passes flat values, including the window corner radii, to fragments. This removes optical storage-buffer fetches from the fragment
path, at the cost of flat varying/register traffic; profiling must confirm the tradeoff on target
GPUs. No dynamic indexing of sampled-image arrays, push constants, runtime compiler or new
per-property uniform calls are introduced. Set 3 binding 0 carries the blurred backdrop and binding 1 carries the sharp backdrop.
Texture slot identity includes both images. Rebinding the same native image is an early return.

Portable retained-scene tests show the following **queued payloads for a single lens**, after its
initial upload is committed:

| Change | Geometry bytes | Optics bytes | CPU image upload |
|---|---:|---:|---:|
| Unchanged state | 0 | 0 | 0 |
| Move/resize with unchanged radii and bevel limit | 64 | 0 | 0 |
| Tint, optical strength, or blend softness only | 0 | 60 | 0 |

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
composite,sync,descriptor,executor}.rs` and `application_host/shell_wayland/{scene.rs,
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

## Full-resolution refactor audit (2026-09-15)

The adjacent `../other-rendering-libs` library was unavailable. Upstream review used Flutter's
[`gaussian_blur_filter_contents.cc`](https://github.com/flutter/engine/blob/main/impeller/entity/contents/filters/gaussian_blur_filter_contents.cc)
(kernel generation, paired linear taps and filter subpasses) and Qt's
[`qquickshadereffectsource.cpp`](https://github.com/qt/qtdeclarative/blob/dev/src/quick/items/qquickshadereffectsource.cpp)
(source invalidation and render-thread resource release). These are moving upstream references,
not pinned local revisions. No external code or dependency was imported.

The invariants are normalized symmetric filtering, independent source/destination targets,
source changes invalidating cached pixels, and resource release after rendering completes.
Vulkan's [synchronization examples](https://docs.vulkan.org/guide/latest/synchronization_examples.html)
and [resource interface contract](https://docs.vulkan.org/spec/latest/chapters/interfaces.html#interfaces-resources)
support retaining the existing attachment-to-sampling transitions, fragment storage visibility,
and two-sampler descriptor layout. Local review covered `renderer_vulkan/{scene,frame,executor,
composite,target}.rs` and desktop Vulkan `glass.rs`/`motion_glass.rs`.

Rejected alternatives: more reduction levels still lose detail; nearest filtering increases
blockiness; sharpening the reduced image cannot restore the missing rim detail. Validation covers
normalized kernels, paired taps versus direct convolution of alternating fine detail, fractional
radius cache invalidation, constant target extents, missing sharp images, shader reflection and
explicit LOD. Live GPU appearance and performance remain user-qualified.

### High-resolution budget regression

The initial full-resolution refactor retained the old 96 MiB pyramid budget. A 3840×2400
output requires 110,592,000 bytes (105.5 MiB) for one three-target backdrop, so admission failed
before the lens shader ran and the compositor displayed the flat tint fallback. The budget is
now 256 MiB, allowing two such cached backdrops while retaining the bounded allocation fallback.
A CPU admission regression covers that physical output, two simultaneous lenses, rejection of a
third, the exact budget boundary and overflow. Output scaling is already reflected in the physical
extent and is not applied again. This adjusts only admission; the preceding resource lifetime
and Vulkan synchronization audit still applies. Shader optics and user settings are unchanged.

## Lighting removal and exact optimizations (ABI 4.4)

Directional rim lighting and specular highlights, including their public `GlassStyle` and material
fields, are removed. Fresnel reflection remains. Existing callers must delete `rim` and `specular`
initializers; the test compositor is updated. Its previous values were both zero, so this removal
preserves its appearance. The packed optics range shrinks from 17 words/68 bytes to 15 words/60 bytes.

The full optical calculation is skipped when `max(q.x,q.y) <= min(0,r-O)`: these pixels are inside
the rectangular interior, beyond the outer fade, and the corner smoothing cannot reactivate them.
A dense independent reference-profile test verifies exactly zero weight and displacement for all
skipped points across tiny/wide windows, corner radii, bevel widths and softness. Zero softness
skips the tail polynomial; zero Fresnel skips the square root. Outer width and half-width move to
vertex calculations using the varying lanes freed by removed lighting parameters. The central sharp
texture sample is shared between dispersed and non-dispersed paths (four static sample sites,
unchanged one/two/four executed samples). Gaussian parameter ranges now use the actual packed
length instead of regenerating the Gaussian kernel merely to count its words.

These are algebraic/control-flow and CPU packing optimizations, not a new approximation to the
lens or blur. Full-resolution targets, paired Gaussian taps and the corner fix remain. The preceding
reference and Vulkan resource audit applies unchanged: the descriptor layout, synchronization and ownership remain unchanged. No imported source was used. Sampling work in a dispersive edge is unchanged;
the inactive specular branch was already skipped, so removal alone is not a large measured speedup.
Real GPU timing/power and visual equivalence require user-run qualification; no percentage claim
is supported by offline compilation or operation counts.

Clip identity and clamped opacity now also travel as flat vertex outputs. Consequently the liquid
fragment shader no longer reads the material instance SSBO; reflection requires the absence of
set 2 bindings in that stage. The vertex still validates the 64-byte instance stride and supplies
unchanged per-primitive values. This trades one additional flat scalar for fragment storage reads;
the two removed lighting lanes are reused by precomputed fade dimensions. Target profiling must
confirm the balance between register/varying traffic and avoided storage accesses.

Validation for the optimization: the library run passed 1,164 tests; 20 unrelated Wayland wire/socket
tests failed while creating private clients or display sockets in the execution environment, and
six hardware fixtures remained ignored. Focused glass tests and shader reflection are checked
separately. No GUI, compositor or hardware presentation was launched by the agent.

# Glass pipeline optimization

## Implemented behavior

The divider-resize bottleneck was structurally amplified by independent lower-window backdrops:
updating a lower preview could invalidate every preview above it. Four matching tiles now sample
one desktop below their resize group, sharing capture and filtering by normalized blur radius.
Tint, refraction, dispersion, border and fade weights remain per window. The group survives release
until its content fade finishes. Different radii remain separate. Interrupted recipes containing
conflicting radii for one scene key disable aliasing. Group previews intentionally do not refract
one another; ordinary overlapping windows retain bottom-to-top backdrop semantics.

On a static desktop, changing only the divider geometry can reuse that shared capture and filter.
This removes the cascading dependency rather than merely increasing the cache budget. A changing
wallpaper or lower window still invalidates the backdrop. Reduced motion and immediate fallback
retain their existing direct rendering path.

Independent lenses use a padded, screen-clipped capture. The conservative margin in physical pixels
is `ceil(3*sigma + abs(refraction) + abs(dispersion) + 4)`, with `sigma = blur_radius/2`.
The analytic optical displacement has magnitude at most one before scaling, including its
normalized twist. Four additional pixels cover reduction/reconstruction support. Edges round
outward to 64-pixel boundaries; source placements outside the crop do not invalidate it. Scene
revision changes inside the crop remain conservative: there is no per-pixel damage dependency graph.

Broad blur (radius >= 8 physical pixels) retains a native sharp capture and uses three half-sized
targets: prefilter, horizontal Gaussian, vertical Gaussian. A zero-radius linear sample averages a
2x2 footprint at exact 2:1 reduction; odd dimensions use the actual normalized mapping. Each axis
scales sigma by source/native size. Gaussian tap pairing remains exact for its discrete kernel:
`w0*T(x) + w1*T(x+1) = (w0+w1)*linearT(x+w1/(w0+w1))`.
The reduction and reconstruction are an approximation and add a small amount of smoothing;
they do not change full-resolution borders, contours or sharp refraction. Small kernels stay native.
The threshold can produce a small visual change when animating blur radius across 8; live visual
qualification is still needed. No adaptive quality or time-dependent quality switching is enabled.

At even dimensions the broad-blur cache contains 1.75 native image equivalents rather than 3.
Four matching 3840x2400 previews can share about 61.5 MiB of raw RGBA pixels, versus about 421.9 MiB
for four independent three-image native caches. These figures exclude allocation padding,
motion resolves, snapshots, spare images and in-flight pins; they are not measured GPU savings.

Filter scenes and descriptors persist across captures. Resolve and bordered intermediate targets
round capacity to 64 pixels, reuse it while both dimensions fit, and shrink when a dimension exceeds
twice the requested size (with a 64-pixel minimum). Published image geometry maps active pixels
one-to-one and clips padding. Budget admission charges retained allocations, including their padding,
rather than the smaller logical size. Existing queue ordering, image pins and completion-safe pool
checkout remain unchanged.

Empty captured bodies now use a 1x1 transparent image. Scaling zero premultiplied pixels or mixing
only zero bodies remains zero, so glass-only resize snapshots need no large empty attachment.
Nonempty body snapshots now reuse capacity-bucketed storage with separate active extents. When the body is empty and there is exactly one
fully weighted lens, glass and border render directly into the resolve target: no body copy,
weighted border intermediate, or final intermediate copy is needed. Interrupted fades, fractional
weights and multiple lenses keep the general premultiplied composition path.

## Measurements

`TELORGON_FRAME_STATS=1 ./start.sh` in `test-compositor` now enables the instrumentation feature.
The existing CPU/input reports are joined by `telorgon-gpu-stats` when the graphics queue supports
timestamps. Reports include capture, prefilter, blur, motion snapshots, motion/divider resolve and
final desktop composition. Each stage reports total time divided by submitted-frame count,
calls per frame and maximum call duration over the reporting interval. Stages absent from an
interval performed no recorded work. Stage scopes nest: do not add resolve and its capture/blur
children together. Limits allow 256 scope pairs per frame; excess scopes are omitted.

`gpu.total` now ends when command recording finishes, rather than after the first render pass.
Queries are read on completed frame-slot reuse, without a new wait or submission, and use the
queue's valid-bit mask and timestamp period. They measure submitted GPU commands, not input,
presentation latency or CPU queueing. Enabling timestamps itself has a cost. Hosted rendering does
not acquire this standalone query pool. No GUI, compositor session or hardware GPU test was run.

For user-run qualification, drag horizontal and vertical dividers in a four-tile layout, pause,
then release. Use matching and different glass styles, patterned backgrounds, odd/fractional output
sizes, translucent borders, and an updating lower window. Confirm that native edges remain aligned,
all previews appear, content returns together, and static-background drag intervals have no repeated
capture/blur work. Compare whole-frame and divider-resolve GPU times against CPU/input intervals.

## Reference audit and invariants

The adjacent `../other-rendering-libs` library was unavailable. Existing Telorgon ownership and
synchronization mechanisms were retained. The source/document review used:

- Telorgon `renderer/vulkan/glass.rs`, `motion_glass.rs`, `motion.rs`, and
  `renderer_vulkan/{frame,composite,executor,target}.rs`: immutable snapshot ownership,
  sampled-image pins, render-target transitions, completion-safe recycling and publication epochs.
- [egui-wgpu renderer.rs](https://raw.githubusercontent.com/emilk/egui/master/crates/egui-wgpu/src/renderer.rs):
  persistent resources/bindings and recording work through the caller's encoder.
- [LiquidGlass GlassRenderer.ts](https://raw.githubusercontent.com/ybouane/liquidglass/00aafe50202e916951d6f30d49afa1197ca236a7/src/GlassRenderer.ts):
  separate prepared background textures and lens composition; this is not a Vulkan lifetime model.
- [Vulkan queries specification](https://docs.vulkan.org/spec/latest/chapters/queries.html):
  reset/reuse, timestamp stage semantics, availability, timestamp period and valid bits.
- Existing [resize glass audit](RESIZE_GLASS.md#engineering-audit) and
  [optics audit](LIQUID_GLASS.md): Vulkan synchronization/sampling invariants and paired-filter math.

Never sample the active destination, overwrite a checked-out pooled image before completion, or
release image owners while submissions still hold them. Keep capture support larger than the
maximum optical/filter neighborhood. Apply fades in premultiplied linear color. Different blur
radii cannot alias a prepared filter. Instrumentation must not force GPU completion or introduce
an additional submission.

Headless regressions cover shared group membership through release, four-member stable dependency
planning, different/conflicting radii, crop/support/clip mapping, disjoint source rejection, odd
half extents and physical kernel scaling, capacity cropping/shrink behavior, admission bounds, and
existing fade/motion/fallback cases. Existing Gaussian tests independently compare bilinear tap
pairs against discrete convolution. Hardware framebuffer tests remain compile-only/ignored.

Rejected for this change: CPU readback, sparse stretched blur taps without prefiltering, additional
GPU submissions, unbounded caches, changing visible quality from frame to frame, and speculative
rim-only draws. Beyond the proven empty-body fast path, unconditional border/fade pass fusion is not algebraically safe: applying border
source-over after adding weighted body content attenuates that content again. A future fused shader
must compute the premultiplied weighted sum directly and preserve interrupted multi-endpoint fades;
profile that cost before extending the material ABI. Further pass fusion remains an optimization avenue, not a completed optimization or a claim of maximal hardware performance.

Validation for this change: 255 headless shell/Wayland/X11 tests passed with instrumentation
(5 hardware tests remained ignored), plus 3 Gaussian kernel tests and 2 timestamp arithmetic tests.
The test compositor compiled with and without instrumentation; launcher shell syntax and diff
whitespace checks passed. Existing unrelated compiler warnings remain. These checks do not qualify
GPU visual output or establish a measured speedup.


## Removing interactive setup work

System font discovery now initializes one process-wide database seed. Each text engine clones its
metadata and shared immutable sources, retaining independent shaping caches and glyph atlases.
Embedded static font faces are parsed once per asset pointer/length and shared across engines;
the metadata cache retains at most 64 entries. Dynamic owned font loading remains available.
Installed-font discovery reflects the first engine's environment for the process lifetime.
This avoids repeated filesystem discovery when constructing chrome without sharing mutable atlas
state. Local cosmic-text `font/system.rs` and fontdb `lib.rs` were inspected for database cloning,
source ownership, and face-ID reassignment in `push_face_info`.

Nonempty motion snapshots round allocation capacity up to 64 pixels and reuse compatible spare
textures only when `can_recycle()` confirms their previous consumers have released them. The
existing spare-count/byte limits remain in force. Active dimensions travel with each immutable
snapshot. Published sampling instances use active/capacity UV bounds; the image shader clamps to
half-texel centers within those bounds. This is equivalent to clamp-to-edge on an exact-sized
texture, including enlarged snapshots, and prevents transparent allocation padding from leaking
into the visible edge. Ordinary full-image sampling retains its existing edge behavior. Binding
and scene publication precede the crop override, which is reapplied on every publication.

Delta validation borrows unchanged instance arrays (including truncated prefixes), while patched
arrays remain private copies. Existing reference, growth, and range validation still completes
before retained state is mutated. Material resource validation retains its existing map copy.

The shell renderer prewarms all six pipeline kinds with opaque, alpha, and additive blending for
its initial output formats and the offscreen materialization format. The device's existing locked
pipeline map owns and reuses these objects. This moves compilation into renderer initialization;
it does not remove startup cost or promise that a driver performs no later internal work.

The adjacent source library remains unavailable. The egui and LiquidGlass sources cited above
provide the retained-resource and initialization/composition comparisons; native image lifetime
continues to use Telorgon's existing completion and submission pins. Official
[Vulkan sampler modes](https://docs.vulkan.org/refpages/latest/refpages/source/VkSamplerAddressMode.html)
and [pipeline creation](https://docs.vulkan.org/refpages/latest/refpages/source/vkCreateGraphicsPipelines.html)
were checked. Rejected alternatives include sharing mutable atlases, overwriting busy snapshots,
stretching allocation padding into the image, and removing transactional validation.

Validation: 1,229 headless library tests passed with shell-wayland-linux; seven hardware tests
remained ignored. New regressions verify font sharing and atlas isolation, borrowed versus patched
validation, and snapshot texel mapping at smaller, equal, and enlarged output sizes. The existing
ignored Vulkan fixture also compiles pipeline-prewarm reuse checks. Hardware visuals and latency
improvement still require a compositor run on the target system.

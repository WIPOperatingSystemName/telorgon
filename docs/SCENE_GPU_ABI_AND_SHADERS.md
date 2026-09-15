# Scene-to-GPU ABI and Shader Contract

Status: **Gate 4 — accepted implementation contract**

This document freezes Telorgon's first retained-scene-to-GPU boundary. It defines the scene records,
delta rules, upload model, GPU transfer records, shader interfaces, batching, color handling, and
validation work that implementations must share. It is intentionally more exact than the backend
architecture: a Vulkan implementer should be able to create the files and tests described here
without inventing data layouts or synchronization behavior.

This gate does not make Vulkan types part of the public UI API. Vulkan is the first and reference
backend; Metal, Direct3D, and console backends translate the same semantic contract into their own
resource and pipeline models.

Gate 6 assigns this contract's cases to portable, trace, artifact, and hardware evidence and controls
visual comparison and reports in
[ACCEPTANCE_AND_QUALIFICATION.md](ACCEPTANCE_AND_QUALIFICATION.md).

Normative terms such as **must**, **must not**, **should**, and **may** are requirements at this gate.

## 1. Decisions frozen by this gate

1. The retained scene is backend-neutral and is not the GPU ABI.
2. UI identities and authoring types do not cross into `telorgon-scene` or GPU records.
3. Scene tables use typed generational slots. Removing one item must not relocate unrelated items.
4. Scene deltas are epoch-ordered, atomic, range-based updates with snapshot recovery.
5. Painter order is authoritative. Only adjacent compatible paint items may be batched.
6. The baseline GPU path is vertexless instanced drawing with a draw-index indirection buffer.
7. GPU records live in a small `telorgon-gpu-abi` package and have explicit C layouts, padding, and
   compile-time offset checks.
8. The Vulkan baseline uses four descriptor sets, no push constants, no descriptor indexing, and no
   scalar-block-layout or 16-bit-storage requirement.
9. Shaders consume linear data and emit linear premultiplied RGBA. Blending is premultiplied alpha.
10. Shaders are compiled, validated, reflected, and packaged offline. Runtime shader compilation and
    shader compiler discovery in `build.rs` are prohibited.
11. Vulkan synchronization is derived from declared resource use. Upload helpers may not hide queue
    submission, ownership transfer, or completion from a hosted renderer.
12. The software renderer remains a scene-level reference renderer. It does not consume the GPU ABI.

Anything that would violate one of these decisions requires an architecture review and a gate
revision, not a local backend workaround.

## 2. Boundary and package ownership

The data flow is:

```text
telorgon-ui / telorgon-shell
        |
        | retained authoring state, layout, style, text shaping
        v
telorgon-render compiler
        |
        | scene-native records + epoch delta
        v
telorgon-scene
        |
        | immutable snapshot or validated SceneDelta
        v
telorgon-render planner
        |
        | RenderPlan: uploads, passes, ordered batches, semantic usages
        v
backend adapter (Vulkan first)
        |
        | calls canonical conversion for changed slots; may lower further for a native API
        v
backend resources, pipelines, command recording, submission
```

Package responsibilities are deliberately narrow:

| Package | Owns | Must not own |
|---|---|---|
| `telorgon-ui` / `telorgon-shell` | UI nodes, components, styles, shell primitives | GPU records or native graphics handles |
| `telorgon-render` | UI-to-scene compilation, paint planning, batching, damage, semantic usage declarations, canonical scene-to-GPU-record conversion | Queue submission or native allocation |
| `telorgon-scene` | Backend-neutral retained records, typed IDs, generational tables, deltas and snapshots | UI `NodeId`, pipeline selection, native handles, shader layouts |
| `telorgon-gpu-abi` | Exact portable POD transfer records and layout assertions | Scene ownership, allocation, rendering, or Vulkan types |
| `telorgon-renderer-vulkan` | Vulkan conversion, resources, descriptors, pipelines, commands, synchronization | UI interpretation or runtime shader compilation |
| `telorgon-shader-build` | Offline compile, validate, reflect, manifest generation, generated Rust metadata | Runtime dependency of applications or backends |

`telorgon-gpu-abi` is not a general graphics abstraction. It is a shared byte contract between the
planner/backend conversion code, shader artifacts, and conformance tests. A Metal or Direct3D
backend may consume these layouts directly when suitable or convert them into an equivalent native
layout; observable scene behavior must remain the same.

## 3. Scene-native records

### 3.1 Identity and storage

Every retained table has its own typed ID:

- `BoxId`, `ShadowId`, `GlyphId`, `ImageId`, and `MaterialId`;
- `ClipId` and `SpatialId`;
- `ImageResourceId` and `SamplerId`.

An ID contains a slot index and generation. IDs from different tables are not interchangeable.
Generation zero is reserved for invalid/default IDs. Slot reuse increments the generation and stale
IDs must fail validation.

Tables are slot-indexed generational tables. Live slots need not be contiguous. Deletion records a
vacancy but must not swap the last item into the vacancy. This gives the backend a stable mapping
from a scene slot to a GPU-buffer element and prevents one removal from dirtying an unrelated tail.
Compaction is an explicit snapshot-level maintenance operation and may not occur inside an ordinary
delta.

The compiler keeps its `NodeId -> scene IDs` association privately. `telorgon-scene` records must not
depend on `telorgon-ui`, CSS-like authoring types, layout nodes, or shell nodes.

### 3.2 Shared value rules

- Geometry is finite `f32` in logical pixels with a top-left origin and positive Y downward.
- NaN and infinity are rejected at compilation boundaries.
- Rectangles use `(x, y, width, height)`; negative sizes are rejected or normalized before entering
  the scene.
- Colors use explicit semantic types. `Srgba8` means straight-alpha, nonlinear sRGB authoring bytes.
  Linear colors and premultiplied colors use distinct types.
- Opacity is finite and clamped to `[0, 1]` at the authoring-to-scene boundary.
- Transforms use a full 2D affine transform. Translation-and-scale-only records are insufficient.
- Corner radii are stored in top-left, top-right, bottom-right, bottom-left order and normalized to
  the rectangle before rendering.
- Optional identities use a typed sentinel or `Option<T>` in CPU records; raw magic integers do not
  leak into the public scene API.

### 3.3 Primitive records

The first implementation must provide one source file per record family.

`BoxPrimitive` contains a local rectangle, fill color, four normalized radii, four border widths,
four border colors, opacity, `SpatialId`, and `ClipId`. Shadows are not embedded in it.

`ShadowPrimitive` contains a local rectangle, radii, offset, blur sigma, spread, color, opacity,
`SpatialId`, and `ClipId`. Keeping shadows separate allows a planner to create an intermediate pass
without changing the box record.

`GlyphPrimitive` contains a local rectangle, atlas-page identity, an integer atlas texel rectangle,
color, opacity, `SpatialId`, and `ClipId`. UVs are stored as texels rather than normalized values so
that growing an atlas page does not rewrite every glyph on that page.

`ImagePrimitive` contains an `ImageResourceId`, local destination rectangle, normalized source UV
rectangle, tint, opacity, `SamplerId`, `SpatialId`, and `ClipId`.

`MaterialPrimitive` contains a material schema/variant identity, local rectangle, a parameter-block
reference, an ordered resource-reference range, opacity, `SpatialId`, and `ClipId`. Material layouts
are shader-bundle contracts, not arbitrary pointers or public backend handles.

`SpatialNode` contains a full local-to-parent affine transform and an optional parent. The compiler
resolves and caches local-to-view transforms for a snapshot. Cycles, invalid parents, and non-finite
results are errors.

`ClipNode` contains a local rectangle, optional radii, `SpatialId`, and an optional parent clip.
Complex paths are a later ABI extension; they may initially compile to a mask-producing material
only through an explicitly supported path. Clip cycles are errors.

`ImageResource` declares logical extent, semantic format, transfer/color encoding, alpha type,
sampling constraints, generation, and content version. Its payload is supplied by typed create and
update deltas, never by a raw GPU handle. Imported native images are governed by the import contract
in [GPU ownership and synchronization](GPU_OWNERSHIP_AND_SYNCHRONIZATION.md), not smuggled into this
record.

### 3.4 Paint order

`ScenePaintItem` contains only a typed primitive reference:

```rust
pub enum ScenePaintItem {
    Box(BoxId),
    Shadow(ShadowId),
    Glyph(GlyphId),
    Image(ImageId),
    Material(MaterialId),
}
```

It does not contain a pipeline, blend mode, target, descriptor, or batch key. The ordered list is the
semantic painter order. The planner derives rendering machinery from the referenced primitive,
target, capability profile, and shader bundle.

## 4. Snapshots and deltas

### 4.1 Epoch rules

Each scene has a monotonically increasing 64-bit epoch. A snapshot establishes an epoch and complete
table state. A delta declares `base_epoch` and `new_epoch`, where `new_epoch == base_epoch + 1`.

The consumer must:

- reject a stale delta;
- reject an epoch gap;
- apply all table, resource, and paint-order changes atomically;
- leave the previous state intact if validation or application fails;
- request or accept a complete snapshot to recover from a gap.

Several valid consecutive deltas may be coalesced before rendering, but diagnostics must preserve the
source epoch span.

### 4.2 Table updates

Each table uses the equivalent of:

```rust
pub struct SceneTableDelta<T> {
    pub writes: Vec<RangePatch<T>>,
    pub removals: Vec<SlotRemoval>,
    pub high_water_slots: u32,
}

pub struct RangePatch<T> {
    pub first_slot: u32,
    pub values: Vec<VersionedSlot<T>>,
}
```

Write ranges are sorted, non-overlapping, and bounded by `high_water_slots`. A versioned slot carries
its generation and value. A removal carries the expected current generation. Duplicate writes,
overlapping ranges, stale removals, and values referring to nonexistent dependencies invalidate the
entire delta.

The delta may replace paint order only when paint order changed. A property-only update must not
force a draw-index upload. A resource or clip change may force the planner to repartition adjacent
batches, but it must not rewrite unchanged instance records.

### 4.3 Image and atlas updates

Resource deltas have explicit create, rectangular update, and destroy operations. An update declares:

- resource ID and expected generation/content version;
- texel rectangle and mip/layer (initially mip 0, layer 0 only);
- semantic pixel format;
- `row_bytes` and payload bytes.

Bounds, row length, and byte count are validated before state changes. Atlas updates use the same
form. Reallocation of a glyph atlas page changes page resource metadata, not stored glyph texel
rectangles. Destruction is deferred by the backend until the last submission using the native
resource completes.

## 5. Render plan and batching

`telorgon-render` turns a validated scene state and target description into an immutable `RenderPlan`.
It contains uploads, ordered passes, per-pass resource-use declarations, adjacent draw batches, load
and final-target intent, damage, and diagnostic estimates. It contains no native handles.

Painter order is exact. The baseline planner may merge only adjacent items with identical pipeline
and binding keys. It must not globally sort by texture or pipeline. It must not add an opaque-item
reordering pass. A future opaque optimization requires a separately reviewed proof that overlap,
clips, filters, destination reads, and accessibility/debug overlays preserve results.

`PipelineKey` contains at least:

- shader interface version and variant;
- target class and backend-native format class;
- sample count;
- blend mode;
- clip implementation;
- binding strategy.

`BindingKey` contains at least the generations of the primary sampled image, sampler, optional clip
mask, and material resource group. A non-bindless baseline merges adjacent image draws only when this
key is compatible. Optional descriptor indexing may change Vulkan's backend-local batching but must
not change scene identity or the public ABI.

### 5.1 Draw indirection

Each paint item contributes its primitive table slot to a `u32` draw-index buffer. The vertex shader
uses `gl_InstanceIndex` to load the table slot, then loads the primitive record. The canonical quad
is generated from `gl_VertexIndex` as four triangle-strip vertices. Culling is disabled.

The Vulkan baseline records the equivalent of:

```text
vkCmdDraw(vertex_count = 4,
          instance_count = batch.count,
          first_vertex = 0,
          first_instance = batch.draw_index_start)
```

This needs no vertex buffer and keeps primitive slots stable while paint order changes independently.
Backends lacking an equivalent base-instance facility may supply a batch offset through a compatible
constant mechanism; shader-visible semantics are unchanged.

## 6. `telorgon-gpu-abi` byte layouts

The package starts at `GPU_ABI_MAJOR = 1`, `GPU_ABI_MINOR = 0`. Any incompatible field meaning,
offset, width, binding, or stage-interface change increments the major version. Compatible additive
shader variants may increment the minor version.

Every record is `#[repr(C, align(16))]`, `bytemuck::Pod`, and `bytemuck::Zeroable`. Records must not
contain Rust `bool`, enums without a fixed representation, `usize`, references, implicit padding, or
three-element vector fields. Reserved words are written as zero and ignored when read. Layout tests
use `size_of`, `align_of`, and `offset_of!` for every field.

The following offsets and sizes are normative. All offsets are bytes.

### 6.1 Frame/view record

`GpuView`, alignment 16, size 192 in the implemented GPU ABI 4.4:

| Offset | Field | Meaning |
|---:|---|---|
| 0 | `clip_from_view_0: [f32; 4]` | clip matrix row 0 |
| 16 | `clip_from_view_1: [f32; 4]` | clip matrix row 1 |
| 32 | `clip_from_view_2: [f32; 4]` | clip matrix row 2 |
| 48 | `clip_from_view_3: [f32; 4]` | clip matrix row 3 |
| 64 | `view_size_scale: [f32; 4]` | logical width/height, scale X/Y |
| 80 | `target_size_origin: [f32; 4]` | target pixel width/height, render-area X/Y |
| 96 | `render_size_inverse: [f32; 4]` | render pixel width/height, inverse target width/height |
| 112 | `epoch_flags: [u32; 4]` | epoch low/high, target color mode, flags |
| 128 | `placement_clip_rects: [[f32; 4]; 2]` | optional output-pixel X/Y/width/height bounds |
| 160 | `placement_clip_radii: [[f32; 4]; 2]` | corresponding TL/TR/BR/BL circular radii |

`GpuView` uses uniform-buffer-compatible base alignment. Matrices are stored as explicit rows so host
layout does not depend on language matrix-major defaults.

ABI 3 extends the view record for bounded compositor clipping; all stages declare the same block.
A negative rectangle width disables a clip slot, and the two enabled rounded bounds intersect.
ABI 4 assigns flags bits 1/2 to invert coverage for clip slots 0/1 respectively, allowing frame fill
outside an aperture without painting behind transparent clients. A zero-extent normal clip rejects
everything; its inverse accepts everything. Ordinary non-composite views explicitly disable both slots. Composite
fragment stages use output-space signed-distance coverage with a one-pixel antialias band and
apply coverage to both premultiplied RGB and alpha; opaque batches select source-over when these
clips are enabled. Analytic box bodies intersect their outer/inner coverage with scene and placement
coverage before assigning material alpha, so a matching rounded clip cannot attenuate an already
antialiased edge twice. Other sampled content retains multiplicative masking. Box edges and rounded
scene clips derive their pixel width from the analytic edge normal and coordinate derivatives,
using Euclidean gradient length to keep the antialias band independent of edge angle. Existing
scene clips and scissors still apply. Clip metadata is placement-owned, preserving
shared scene resources and per-frame lifetime; there is no client image rewrite or mask attachment.
See [rounded-frame audit](WAYLAND_RESIZE_PREVIEW.md#rounded-frame-clipping-audit).

### 6.2 Spatial and clip records

`GpuSpatial`, alignment 16, size 32:

| Offset | Field | Meaning |
|---:|---|---|
| 0 | `local_to_view_0: [f32; 4]` | `m00, m01, tx, 0` |
| 16 | `local_to_view_1: [f32; 4]` | `m10, m11, ty, 0` |

`GpuClip`, alignment 16, size 128:

| Offset | Field | Meaning |
|---:|---|---|
| 0 | `view_bounds: [f32; 4]` | conservative min X/Y, max X/Y in view space |
| 16 | `local_rect: [f32; 4]` | X, Y, width, height |
| 32 | `local_from_view_0: [f32; 4]` | inverse affine row 0 |
| 48 | `local_from_view_1: [f32; 4]` | inverse affine row 1 |
| 64 | `radii: [f32; 4]` | TL, TR, BR, BL |
| 80 | `mask_uv_from_view_0: [f32; 4]` | mask UV affine row 0 |
| 96 | `mask_uv_from_view_1: [f32; 4]` | mask UV affine row 1 |
| 112 | `mode_mask_flags: [u32; 4]` | clip mode, mask slot, flags, reserved |

Clip modes are `0 none`, `1 scissor`, `2 analytic rounded rectangle`, and `3 sampled mask`. Unknown
modes are rejected before command recording.

### 6.3 Primitive records

`GpuBoxInstance`, alignment 16, size 96:

| Offset | Field |
|---:|---|
| 0 | `rect: [f32; 4]` |
| 16 | `radii: [f32; 4]` |
| 32 | `border_widths: [f32; 4]` |
| 48 | `fill_border_t_r_b: [u32; 4]` |
| 64 | `border_l_spatial_clip_flags: [u32; 4]` |
| 80 | `opacity: f32` |
| 84 | `reserved: [u32; 3]` |

The packed colors at offset 48 are fill, top, right, and bottom. Offset 64 contains left color,
spatial slot, clip slot, and flags.

`GpuShadowInstance`, alignment 16, size 64:

| Offset | Field |
|---:|---|
| 0 | `rect: [f32; 4]` |
| 16 | `radii: [f32; 4]` |
| 32 | `offset_blur_spread: [f32; 4]` |
| 48 | `color_spatial_clip_flags: [u32; 4]` |

The conversion step folds `ShadowPrimitive.opacity` into the packed straight-alpha color's alpha
byte. RGB remains straight sRGB until the fragment shader performs linearization and
premultiplication.

`GpuGlyphInstance`, alignment 16, size 64:

| Offset | Field |
|---:|---|
| 0 | `rect: [f32; 4]` |
| 16 | `uv_texels: [f32; 4]` |
| 32 | `color_spatial_clip_page: [u32; 4]` |
| 48 | `opacity: f32` |
| 52 | `flags: u32` |
| 56 | `reserved: [u32; 2]` |

`GpuImageInstance`, alignment 16, size 64:

| Offset | Field |
|---:|---|
| 0 | `rect: [f32; 4]` |
| 16 | `uv_normalized: [f32; 4]` |
| 32 | `tint_spatial_clip_texture: [u32; 4]` |
| 48 | `opacity: f32` |
| 52 | `sampler_key: u32` |
| 56 | `flags: u32` |
| 60 | `reserved: u32` |

`GpuMaterialInstance`, alignment 16, size 64:

| Offset | Field |
|---:|---|
| 0 | `rect: [f32; 4]` |
| 16 | `params_spatial_clip: [u32; 4]` | parameter word offset/length, spatial, clip |
| 32 | `opacity: f32` |
| 36 | `material_variant: u32` |
| 40 | `flags: u32` |
| 44 | `reserved: u32` |
| 48 | `resource_range_reserved: [u32; 4]` | resource base/count, two reserved words |

Implemented ABI 4.4 retains liquid-glass material variant 3 (`PipelineKind::LiquidGlass`). Its parameter
range is 15 float-bit words (60 bytes): four radii; inverse output size, inverse bevel and refraction;
dispersion/Fresnel; premultiplied linear tint RGB and transmission; inner blend softness.
The softness scalar occupies byte 56 in the raw uint array, with a four-byte word stride. The vertex stage
loads set 2 binding 1 and forwards flat optics; set 3 bindings 0 and 1 sample owned blurred and sharp backdrops. Geometry
and optics retain separate dirty ranges. The existing material instance layout remains unchanged.
See [Liquid-glass analysis](LIQUID_GLASS.md) for validation and ownership details.

Gaussian blur uses material variant 4 and `PipelineKind::GaussianBlur`. Its raw parameter range
contains six float-bit header words (inverse source size XY, UV step XY, center weight, positive
pair count), followed by up to 192 offset/weight pairs. Negative taps are symmetric. The fragment
stage reads set 2 binding 1 and samples its owned source at set 3 binding 0. Material instances
remain 64 bytes. Texture slots are keyed by the primary image and optional secondary image;
liquid sharp-image identity is resource metadata, not an extra packed optics word.

The draw-index buffer is a base-aligned storage-buffer array of `u32`. It has no wrapper stride
beyond four bytes. Shaders must bounds-check indirectly through validated batch ranges; production
shaders do not add divergent per-vertex recovery for an invalid plan.

### 6.4 Word encodings and flags

`NO_GPU_SLOT` is `0xffff_ffff`. GPU indices are table slot indices, not packed generational IDs; the
CPU plan has already validated generations. Optional clip and mask slots use `NO_GPU_SLOT`.

The initial flag and mode words are normative:

| Word | Values |
|---|---|
| `GpuView.epoch_flags[2]` | `0 linear attachment`, `1 hardware sRGB attachment`, `2 linear intermediate requiring final encode` |
| `GpuView.epoch_flags[3]` | bit 0 target is opaque; bits 1/2 invert placement clips 0/1; remaining bits zero |
| `GpuBoxInstance` flags | bit 0 fill present; bit 1 border present; every other bit is zero |
| `GpuShadowInstance` flags | zero; inset or alternative shadow algorithms require a later variant |
| `GpuGlyphInstance.flags` | bit 0 color glyph; every other bit is zero; it must agree with the selected pipeline variant |
| `GpuImageInstance.flags` bits 0–1 | `0 linear`, `1 sRGB`; values 2–3 invalid in ABI 1 |
| `GpuImageInstance.flags` bits 2–3 | `0 straight alpha`, `1 premultiplied alpha`, `2 opaque`; value 3 invalid |
| `GpuImageInstance.flags` bit 4 | texture index is meaningful for a capability-selected indexed binding path; zero in the baseline |
| `GpuMaterialInstance.flags` | bundle-manifest-defined; zero for all Gate 4 built-in variants |

Unknown or nonzero reserved bits are rejected by plan validation. The non-indexed baseline writes
zero to image `texture_index`; the bound descriptor determines the image. A capability-selected
indexed path writes the validated descriptor-table index and sets bit 4.

`sampler_key` is a stable packed sampler class used by planning and diagnostics: bit 0 minification
linear, bit 1 magnification linear, bits 2–3 U address mode, bits 4–5 V address mode, and bit 6 linear
mipmap filtering. Address modes are `0 clamp-to-edge`, `1 repeat`, `2 mirrored-repeat`; value 3 is
invalid. Mipmap filtering is zero while the initial image contract exposes only mip 0. Backend
sampler handles never enter this word.

### 6.5 Color packing

GPU color packing uses these numeric bit positions within a `u32`; it is not defined by transmuting
a four-byte host array:

```text
bits  0..=7   R
bits  8..=15  G
bits 16..=23  B
bits 24..=31  A
```

Helpers use shifts and masks. `ColorRgba8::to_ne_u32` must not be used at this boundary. Initial SDR
packed colors represent straight-alpha sRGBA authoring values; the fragment shader converts their
RGB channels to linear and premultiplies them.

## 7. Shader descriptor and stage ABI

The baseline fits Vulkan's required minimum of four bound descriptor sets and orders the least
frequently changing data first.

| Set | Binding | Resource | Stages | Change frequency |
|---:|---:|---|---|---|
| 0 | 0 | `GpuView` uniform buffer | vertex, fragment | per target/frame |
| 1 | 0 | readonly `GpuSpatial[]` storage buffer | vertex | per scene upload |
| 1 | 1 | readonly `GpuClip[]` storage buffer | fragment | per scene upload |
| 1 | 2 | readonly `u32 draw_indices[]` storage buffer | vertex | paint-order change |
| 2 | 0 | pipeline-specific readonly instance storage buffer | vertex, fragment | primitive table/buffer change |
| 2 | 1 | readonly material parameter words | vertex/fragment | material use only |
| 3 | 0 | primary combined image/sampler | fragment | adjacent binding batch |
| 3 | 1 | clip-mask combined image/sampler | fragment | masked clip batch |

Set 0 binding 0 is declared `std140`. All storage blocks are declared `std430`; array stride equals
the record size in section 6, and the draw-index stride is four. The bundle must not require scalar
block layout, relaxed block layout, 8/16-bit storage, or buffer device address for these records.

All Gate 4 built-in graphics pipelines use compatible set 0–3 layouts with the descriptor types and
stage visibility above. A descriptor need not be bound when it is not statically used by either
stage, so an ordinary box draw does not create or bind dummy sampled images. This keeps frame and
scene sets compatible across pipeline switches without making an unused texture part of box shader
behavior. The initial ABI has no push constants and no vertex attributes.

The built-in vertex-to-fragment interface is also fixed:

| Location | Vertex output / fragment input | Qualifier | Meaning |
|---:|---|---|---|
| 0 | `vec2 unit_position` | `noperspective` | canonical quad coordinate in `[0, 1]` |
| 1 | `vec2 view_position` | `noperspective` | logical view-space position used by analytic coverage/clips |
| 2 | `uint instance_slot` | `flat` | slot loaded from `draw_indices[gl_InstanceIndex]` |

The fragment output is location 0 `vec4`, linear premultiplied RGBA. The shader must not write
backend-specific encoded output except in the dedicated final-encode pass. Any material variant that
needs additional varyings declares them at locations 3 and above in its versioned manifest and does
not change locations 0–2.

The canonical triangle-strip vertex order is `(0,0), (1,0), (0,1), (1,1)`. The vertex shader loads
the record at `draw_indices[gl_InstanceIndex]`, constructs local position from the record rectangle,
applies `GpuSpatial`, and then applies `GpuView`. Fragment shaders reload fragment-needed record
fields through the flat slot and derive UVs from `unit_position`. Stage I/O locations and
interpolation qualifiers are explicit in source and reflection-checked.

## 8. Coordinate, clipping, and antialiasing contract

Scene coordinates are logical pixels, top-left origin, positive Y downward. Primitive rectangles are
local; `GpuSpatial` maps local coordinates to view coordinates. `GpuView` maps view coordinates into
the backend clip convention. The Vulkan backend may implement the Y convention with a negative
viewport height or with the generated matrix, but it must pass the same coordinate conformance
images and must choose one approach consistently.

Logical texture UV `(0, 0)` is the top-left of the imported image. A backend normalizes native image
origin during upload/import rather than changing widget geometry. Glyph UVs are converted from texel
coordinates using the current atlas extent in the shader or a stable per-page parameter.

Clips are selected as follows:

1. Axis-aligned rectangular clip chains are intersected by the planner and emitted as a scissor.
2. One representable transformed rounded rectangle uses `GpuClip` analytic coverage.
3. Nested, path, filtered, or otherwise non-analytic clips render a mask intermediate and sample it
   through set 3 binding 1.
4. If the required mask path is unavailable under the active capability profile, planning returns a
   typed unsupported-feature error. It must not silently ignore the clip.

Scissors intersect the pass render area. Conservative conversion floors the minimum edge and ceils
the maximum edge before clamping to the target extent. Non-invertible transforms used by analytic
clips are rejected or routed through a supported mask path.

Edge antialiasing uses analytic coverage derived from fragment-space derivatives where supported.
Pixel snapping is opt-in component behavior; the renderer must not silently round all geometry.

## 9. Color, alpha, and target encoding

All normal blend passes operate in linear light and emit premultiplied alpha:

```text
linear_rgb = srgb_decode(authoring_rgb)
effective_alpha = authoring_alpha * opacity * coverage
output_rgb = linear_rgb * effective_alpha
output_alpha = effective_alpha
```

The blend state is additive with source factor `ONE` and destination factor
`ONE_MINUS_SRC_ALPHA` for both color and alpha. The glyph atlas baseline is `R8_UNORM` linear
coverage. A glyph shader distinguishes mask glyphs from color glyphs through an explicit shader
variant, not by guessing from a texture.

Every image declares both color encoding (`sRGB` or `linear`) and alpha representation (`straight`,
`premultiplied`, or `opaque`). For a premultiplied sRGB image, “premultiplied” means its encoded RGB
decodes to RGB already premultiplied in linear light; gamma-space-premultiplied assets require an
explicit conversion on import and must not be mislabeled. Sampling uses an sRGB native view when
format support permits or an interface-equivalent shader decode fallback. Straight input is decoded
and then multiplied by sampled alpha; premultiplied input is decoded without multiplying RGB again;
opaque input forces alpha to one. Tint is decoded to linear straight RGBA, then its alpha and
primitive opacity scale both premultiplied RGB and alpha. Unsupported or contradictory import
metadata is a typed error.

Target handling is:

- A hardware sRGB attachment accepts linear shader output and performs the final sRGB encoding.
- A linear floating-point or UNORM attachment accepts linear output directly.
- A nonlinear presentation target without an sRGB attachment format uses a linear intermediate and
  one final encode pass.
- Individual primitives are never sRGB-encoded before blending.

The Vulkan backend queries required sampled-image, filtering, color-attachment, blend, and transfer
features for every selected native format. Format names alone do not establish support. Opaque target
alpha handling is an explicit target policy.

## 10. Upload and residency contract

The backend keeps a CPU mirror of the validated scene tables. Only changed live slots are converted
into `telorgon-gpu-abi` records and uploaded. Consecutive byte ranges are coalesced. Removals need not
clear device memory, because no valid draw index may reference the removed generation.

Uploads use a persistently mapped staging ring where supported. Non-coherent writes are flushed to
the implementation's atom-size alignment. Buffer-copy offsets and sizes satisfy Vulkan's required
alignment, and texture rows satisfy the selected texel format and copy constraints. Multiple copies
are batched before the transition to drawing.

Device-local scene buffers grow geometrically. A grow operation allocates a replacement, uploads the
complete live CPU mirror, updates descriptors only after the copy is scheduled correctly, and
defers destruction of the old buffer until its last submission token completes. The allocator must
not reuse staging or device ranges still referenced by in-flight work.

A partial overwrite of a resource read by an earlier submission still needs the appropriate
read-to-write ordering. Hosted mode must record uploads and draws in the host-provided command stream
or return explicit work and usage declarations. Convenience functions equivalent to a hidden
`queue.write_*` submission are prohibited in hosted rendering.

Diagnostics report actual converted bytes, uploaded bytes, buffer and image copy counts, descriptor
updates, barriers, batches, and draw calls. Estimated counts are labeled as estimates.

## 11. Semantic use to Vulkan synchronization

The planner speaks backend-neutral use categories. The initial Vulkan synchronization2 translation
must cover at least these transitions:

| Semantic transition | Vulkan synchronization intent |
|---|---|
| Host staging write -> transfer source | Flush non-coherent ranges; queue submission makes prior host writes available, so no synthetic pre-copy pipeline barrier is added |
| Transfer buffer write -> vertex/fragment storage read | `COPY / TRANSFER_WRITE` -> applicable vertex or fragment shader / `SHADER_STORAGE_READ` |
| Prior shader read -> transfer overwrite | applicable shader / storage or sampled read -> `COPY / TRANSFER_WRITE` |
| Sampled image -> transfer update | fragment sampled read -> `COPY / TRANSFER_WRITE`, shader-read-only -> transfer-destination layout |
| Transfer image update -> sampled | `COPY / TRANSFER_WRITE` -> fragment / sampled read, transfer-destination -> shader-read-only layout |
| Target initial state -> color attachment | transition from declared acquire/load state to attachment write, honoring preserve/clear/discard policy |
| Color attachment intermediate -> sampled | color-attachment-output/write -> fragment/sampled read, attachment -> shader-read-only layout |
| Final target -> release/present | transition to the final state promised by `TargetFinalState` and return the completion token from Gate 3 |
| Color result -> readback copy | color attachment -> transfer source; copy write -> host read is completed through the Gate 3 readback token |

Use actual synchronization2 stage and access flags in the Vulkan implementation. The table is a
semantic specification, not permission to use blanket `ALL_COMMANDS` barriers. Whole-resource
tracking is acceptable for the first scene buffers; image mip/layer and atlas update rectangles need
the range/subresource precision required to avoid conflicting use. Same-queue work across submissions
still requires correct memory dependencies where a resource changes use.

The implemented Vulkan renderer has one driver-scoped exception: V3DV uses `SHADER_READ` for the
destination access of retained storage-buffer upload completion. It includes storage reads while
avoiding a driver-side access-mask truncation. Other drivers, shader stages, ranges, pre-upload
dependencies, and image barriers are unchanged. See the
[V3DV workaround audit and acceptance procedure](V3DV_GEOMETRY_UPLOAD_WORKAROUND.md).

## 12. Offline shader bundle

GLSL 450 is the authoritative source language only for the first Vulkan bundle. It is not a promised
cross-backend shader language. Later backends may use generated or separately maintained native
sources as long as their artifacts pass the same semantic conformance suite.

`telorgon-shader-build` uses tool-only dependencies:

- `shaderc = "0.10"` to compile GLSL;
- `spirv-tools = "0.13"` to validate against the Vulkan 1.3 target environment;
- `rspirv = "0.13"` to inspect SPIR-V entry points, decorations, types, and member offsets;
- `sha2 = "0.11"`, `serde = "1"`, and a workspace-pinned TOML implementation for deterministic
  manifests.

These packages are never linked into the runtime renderer. Compilation targets Vulkan 1.3 and
SPIR-V 1.6 with fixed warning, optimization, and debug-info policies. All descriptor sets, bindings,
stage I/O locations, block layouts, and member offsets are explicit. Warnings fail CI.

The build tool performs this deterministic sequence:

1. discover only manifest-listed shader sources;
2. normalize options and hash sources plus compiler configuration;
3. compile every entry point;
4. validate every module with SPIR-V Tools for Vulkan 1.3;
5. inspect SPIR-V with `rspirv`;
6. compare descriptors, stages, locations, strides, offsets, push-constant absence, and capability
   requirements with the checked-in interface declaration and `telorgon-gpu-abi` constants;
7. write artifacts, a human-readable TOML manifest, and generated Rust constants;
8. hash every artifact and the complete bundle.

The manifest contains schema version, bundle/interface versions, source and artifact hashes,
compiler target/options, entry points, stages, descriptors, record member offsets/strides, stage I/O,
specialization constants, and required features. The runtime uses generated Rust metadata and
packaged SPIR-V; it does not parse arbitrary shader source or reflect interfaces at startup. It
verifies bundle/interface versions and artifact hashes before creating pipelines.

Initial variants are:

- box/border fill;
- glyph mask and color glyph;
- image input normalization;
- clip-mask generation/use;
- final target encode.

Material variants are added through versioned manifests. The current Vulkan crate shaders and its
compiler-probing `build.rs`, and the material crate's ad hoc build script, are replacement targets;
they are not the source of truth for the new bundle.

## 13. Required file layout

The implementation should create these narrowly scoped files rather than one ABI or shader module:

```text
crates/telorgon/src/gpu_abi/
  Cargo.toml
  src/
    lib.rs                 # versions and deliberate public exports
    color.rs               # bit-exact packing and decode helpers used by tests
    view.rs                # GpuView
    spatial.rs             # GpuSpatial
    clip.rs                # GpuClip and clip constants
    box_instance.rs        # GpuBoxInstance
    shadow_instance.rs     # GpuShadowInstance
    glyph_instance.rs      # GpuGlyphInstance
    image_instance.rs      # GpuImageInstance
    material_instance.rs   # GpuMaterialInstance
    layout.rs              # size/alignment/offset compile assertions

crates/telorgon/src/scene/
  id.rs
  table.rs
  snapshot.rs
  delta.rs
  paint.rs
  spatial.rs
  clip.rs
  primitive/
    mod.rs
    box_primitive.rs
    shadow.rs
    glyph.rs
    image.rs
    material.rs
  resource/
    mod.rs
    image.rs
    sampler.rs

crates/telorgon/src/render/
  compiler/              # authoring-to-scene conversion
  plan/                  # RenderPlan, passes, batches, use declarations
  gpu_convert/           # scene record -> telorgon-gpu-abi conversion

crates/telorgon-shader-build/
  Cargo.toml
  src/
    main.rs
    compile.rs
    validate.rs
    reflect.rs
    manifest.rs
    generate_rust.rs
  shaders/vulkan/
    common/
    box/
    glyph/
    image/
    clip/
    encode/
  bundle.toml
```

Files may be split further, but responsibilities must not be collapsed into a renderer-wide module.

## 14. Migration from the current prototype

The existing prototype is evidence and test input, not an ABI to preserve.

- Move and refine the retained records in `crates/telorgon/src/render/scene.rs` into `telorgon-scene`.
- Remove `NodeId` and UI border/image/material types from scene records.
- Replace `DenseInstances` swap removal with typed generational slot tables.
- Replace translation/scale spatial records with full affine records.
- Replace normalized glyph atlas coordinates with atlas-page plus texel rectangles.
- Replace `DrawItem` pipeline state with `ScenePaintItem`; generate `PipelineKey` and `BindingKey` in
  the planner.
- Keep the current compiler's painter order as a behavior reference, while making paint-order deltas
  independent from primitive-table deltas.
- Do not use `ColorRgba8::to_ne_u32` for GPU upload.
- Adapt the software renderer to scene-native records and full transforms. It remains useful for
  semantic tests but is not a fallback implementation for the Vulkan-first milestone.
- Delete and replace the current push-constant, one-primitive-per-draw shaders and compiler-probing
  build scripts after their behavior is captured in tests.
- Remove public material constants that expose shader file names or backend binding assumptions.

Compatibility re-exports may exist for one migration slice, must be marked deprecated immediately,
and must not be added to the umbrella prelude.

## 15. Acceptance tests

Gate 4 is implemented only when the following tests exist and pass. Gate 6 assigns these cases to
portable, trace, shader/ABI, and real-GPU evidence layers and controls visual tolerances and reports
in [Acceptance and qualification](ACCEPTANCE_AND_QUALIFICATION.md).

### ABI and build tests

- size, alignment, and every field offset for every GPU record;
- `Pod`/`Zeroable` derivation and zeroed reserved words;
- stable color pack/unpack golden values independent of host byte order;
- shader regeneration produces no repository diff;
- SPIR-V validation succeeds for every module;
- reflected descriptors, stages, I/O, record offsets, and strides match the manifest and Rust ABI;
- corrupt hashes, wrong interface versions, unexpected capabilities, and interface drift are rejected.

### Scene and planning tests

- stale IDs, stale removals, epoch gaps, overlapping patches, and cyclic dependencies are rejected
  without partial application;
- slot reuse increments generation and removing a slot does not relocate other primitives;
- property-only changes upload only affected instance ranges;
- paint-order-only changes upload only draw indices;
- atlas growth does not rewrite unchanged glyph records;
- painter order is preserved and only adjacent compatible items batch;
- complex clips either produce a mask pass or a typed unsupported error;
- buffer growth copies the live mirror and defers old-buffer destruction.

### Math and rendering tests

- CPU golden tests cover sRGB decode, straight-to-premultiplied conversion, opacity, coverage, and
  premultiplied blending;
- coordinate tests cover DPI scaling, render-area origin, Y orientation, affine transforms, scissor
  rounding, and texture origin;
- a trace backend checks upload/use/barrier/binding plans without a GPU;
- E4 hardware conformance images cover boxes/borders, glyphs, images of every alpha/encoding class,
  analytic and mask clips, overlapping translucent colors, each target mode, and final encode under
  Gate 6's canonical linear-premultiplied comparison policy.

Performance results must include actual upload bytes, staging occupancy, draw/batch counts, descriptor
updates, barrier counts, and CPU planning/recording time. A screenshot alone is not ABI acceptance.

## 16. Reference audit

This contract was checked against independently designed renderers and official specifications. The
references inform pitfalls and patterns; Telorgon does not copy their public APIs.

| Reference | Revision studied | Relevant findings adopted or rejected |
|---|---|---|
| Egui / epaint + egui-wgpu | `fd54387eac03f57ca772a8fb590ceaadf780f31c` | Compact explicit vertices, ordered clipped paint jobs, partial texture deltas, buffer growth, stable/texture binding separation, premultiplied blending, and explicit sRGB target handling. Telorgon adopts ordered adjacent batching, partial uploads, and explicit color handling, but uses retained stable slots and GPU draw indirection instead of flattening all meshes each frame. |
| Flutter Impeller | `51fd9afadf309ba5337320bd3653f5345c156cb9` | Explicit buffer views, aligned/recycled transient host buffers, reflected shader metadata, pipeline descriptors keyed by attachment/state, resource binding wrappers, and premultiplied content output. Telorgon adopts explicit lifetimes, manifests, pipeline keys, and shader-visible record validation while retaining a smaller UI-specific ABI. |
| wgpu | `d99c241a3b9dcc0f6674d990d007d79e94d39862` | Backend capability translation and explicit shader/pipeline layout concepts were cross-checked. Telorgon does not put wgpu in the Vulkan reference backend or expose its API as Telorgon's abstraction. |

Primary technical sources:

- [Vulkan shader interfaces](https://docs.vulkan.org/spec/latest/chapters/interfaces.html)
- [Vulkan descriptor sets and pipeline layouts](https://docs.vulkan.org/spec/latest/chapters/descriptorsets.html)
- [Vulkan formats and format feature queries](https://docs.vulkan.org/spec/latest/chapters/formats.html)
- [Vulkan framebuffer operations and blending](https://docs.vulkan.org/spec/latest/chapters/framebuffer.html)
- [Vulkan SPIR-V environment](https://docs.vulkan.org/spec/latest/appendices/spirvenv.html)
- [Vulkan synchronization examples](https://docs.vulkan.org/guide/latest/synchronization_examples.html)
- [Vulkan shader memory layout guide](https://docs.vulkan.org/guide/latest/shader_memory_layout.html)
- [SPIR-V specification](https://registry.khronos.org/SPIR-V/specs/unified1/SPIRV.html)

The adjacent source audit is also recorded in [Reference implementations](REFERENCE_IMPLEMENTATIONS.md).

## 17. Deferred, not undefined

The following are deliberately deferred to later gates or versioned extensions:

- descriptor indexing and backend-specific bindless batching;
- path primitives and general vector tessellation;
- HDR authoring color spaces and HDR presentation policy;
- subpixel text rendering;
- multisample policy beyond the pipeline key and target description;
- material parameter schema details beyond the versioned manifest mechanism;
- cross-backend shader-source generation;
- opaque reordering, GPU-driven culling, and indirect multidraw;
- vendor-specific imported-image payload spelling after Gate 9's portable identity and linear
  ownership/acquire/release invariants; the Linux Vulkan DMA-BUF profile is already fixed there.

Deferral means the first ABI reserves a clean extension point or returns an explicit unsupported
result. It never means silently approximating behavior or exposing Vulkan objects through the UI.

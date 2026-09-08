# Logical units and output scaling

The Linux Wayland desktop authors and lays out UI in logical units. Its selected KMS mode,
framebuffers, image allocations, glyph atlas, and hardware cursor plane remain physical pixels.
A 32-unit title bar occupies 32 pixels at 100% and 64 pixels at 200%. A 3840×2160 output at 200%
has a 1920×1080 logical desktop. Existing application layout constants do not need resolution checks.

This is integrated into the desktop host and both its software and Vulkan composition paths, with
CPU regression coverage and compilation checks. Interactive KMS/Wayland and GPU presentation still
need manual qualification. This change does not retrofit automatic DPI selection into the separate
managed GUI hosts or implement multiple simultaneous desktop outputs.

## Fixed layout dimensions

Use `Dimension::Logical(value)` in composition builders and `SizeRule::Logical(value)` in
retained styles for fixed logical sizes. For example, `Dimension::Logical(38.0)` occupies
38 physical pixels at 100% and 76 at 200% in the desktop host. Numeric builder arguments
such as `.width(38.0)` continue to convert into logical dimensions.

API migration: replace `Dimension::Pixels` with `Dimension::Logical` and `SizeRule::Px`
with `SizeRule::Logical`, including match patterns. The old variants have been removed.
This is a naming change only; layout values and output scaling behavior are unchanged.
Neither variant specifies a fixed physical-pixel size.

## Selecting scale at boot

`LinuxDesktopConfig::default()` uses `OutputScale::Auto`. After selecting the connector's preferred
mode, the host reads the physical width and height reported by KMS (usually derived from EDID):

```text
horizontal DPI = pixel width  × 25.4 / width in millimeters
vertical DPI   = pixel height × 25.4 / height in millimeters
scale = round(sqrt(horizontal DPI × vertical DPI) / 96 × 4) / 4
```

The policy rounds to the nearest 25%, clamped to 100–400%. Halfway values round upward.
Both physical dimensions must be 50–3000 mm, both densities 50–500 DPI, and the horizontal/vertical
DPI ratio 0.9–1.1. Unknown or implausible dimensions select 100%. The startup diagnostic prints
physical pixels, reported millimeters, chosen percentage, logical extent, and policy.

| Example monitor | Approximate density | Automatic scale |
| --- | --- | --- |
| 24-inch 1920×1080 | 92 DPI | 100% |
| 24-inch 3840×2160 | 184 DPI | 200% |
| 27-inch 3840×2160 | 163 DPI | 175% |
| 43-inch 3840×2160 | 104 DPI | 100% |

96 DPI is a desktop policy baseline, not a measurement of the user's preferred text size or viewing
distance. Physical display metadata can be incorrect even when it passes these checks. In
particular, resolution alone cannot establish the right scale for a TV versus a laptop. An explicit
preference takes precedence and also works when physical dimensions are absent:

```rust
use telorgon::app::{LinuxDesktopConfig, OutputScale};

let config = LinuxDesktopConfig {
    output_scale: OutputScale::Fixed(2.0), // 200%; use 1.0 for 100%, 1.5 for 150%
    ..LinuxDesktopConfig::default()
};
```

Fixed values must be finite and within 1.0–4.0; they are rounded to Wayland's 1/120 increments so
rendering and client announcements agree. This replaces the former integer `output_scale` field.
The current setting is selected once at boot. Persistent per-monitor preferences, runtime scale
changes, monitor hotplug, and moving surfaces between differently scaled outputs remain future work.

## Coordinate and resource contracts

- Desktop window geometry, decorations, widget placement, reservations, hit tests, Wayland configure
  sizes, pointer focus, and cursor hotspots are logical. Historical text layout names such as
  `font_size_px` denote logical dimensions in this desktop host. Cursor builders accept
  `logical_units`, and `PointerGraphic::logical_size()` / `PointerTheme::logical_size()` expose
  the nominal logical size (replacing the former `physical_size()` names). `pointer_extent`,
  title-bar height, and border width are logical.
- `platform::ScaleFactor` is the validated conversion boundary. Floating-point points preserve
  fractional positions. Integer desktop bounds use `ceil(physical / scale)` to cover the final pixel.
  Placement rectangles round shared endpoints, rather than origin and size separately. Damage uses
  floor/ceil to conservatively cover touched pixels. The physical output clips any final partial unit.
- Libinput's normalized accelerated mouse movement is applied directly in logical units, preserving
  density-independent pointer speed. Unaccelerated relative motion retains its device units.
  Absolute devices map their normalized positions into the logical desktop. Hardware cursor positions
  and hotspots convert to physical pixels at the KMS boundary.
- `DesktopComposition` retains logical geometry and damage. `DesktopFrame::into_physical` converts
  placements, rectangular/rounded clips, corner radii, and damage exactly once before either backend.
  The existing `ViewMapping` maps each retained scene into that physical target. KMS mode sizes and
  framebuffer allocations never use the logical desktop extent.
- Text wrapping, advances, and line spacing remain logical. Glyphs rasterize at output density;
  glyph instances separately carry logical rectangles and physical atlas texel dimensions. Software
  sampling and Vulkan uploads consume those separate dimensions. Changing a runtime's raster scale
  clears cached runs/atlas placements and rebuilds its compiled scene. GPU struct/shader layout is
  unchanged. Larger raster glyphs still consume the existing bounded atlas capacity.
- Client windows store both logical surface size and retained image pixel size. SHM transforms sample
  the original buffer directly into an output-density image, avoiding a 1x intermediate that would
  discard HiDPI detail. Untransformed, uncropped SHM images already at the target raster extent reuse
  their original pixels, including integer HiDPI and fractional-scale viewporter clients; logical
  geometry is still computed independently. The simple untransformed scale-1 path also keeps its
  damage patches. DMA-BUF materialization also targets output density. Buffer scale, transform, and viewport
  validation remain in force. SHM allocations retain the 512 MiB bound; Vulkan targets retain device
  extent validation. Existing asynchronous copy, acquire/release, and KMS retirement ownership remain
  intact; logical conversions do not shorten image or scene lifetimes.
- Cursor assets rasterize at output density. Client SHM cursor images retain separate logical size
  and pixel size. Hardware cursor resizing checks plane limits before allocation; unsupported sizes
  fall back to composition. DMA-BUF-only client cursors still lack a CPU cursor-image path and are
  not displayed by this path. Fractional snapping may produce a one-pixel difference when switching
  hardware/composited cursor paths; active logical hotspots remain unchanged.

The native protocol publishes `ceil(scale)` through integer `wl_output.scale` and
`wl_surface.preferred_buffer_scale` (surface version 6+). Mapped surfaces receive `wl_surface.enter`
for their client's enabled output bindings, including late output binds; unmapping emits `leave`.
Tracking uses object IDs, is removed on destruction, and does not retain raw resource pointers.
Fractional-scale clients receive the actual factor in 1/120 units. Clients supporting that protocol
can submit density-sized buffers with buffer scale 1 and a logical viewporter destination. Older
integer-scale clients render at the next integer scale and are resampled to the selected density.
A client that ignores scaling can still appear blurry when enlarged.

## Reference review and derived checks

Resize-latency follow-up: the identity SHM mapping now skips allocation and per-pixel sampling after
the existing scale, source, destination, and allocation-limit validation. Equal raster extents alone
are insufficient: crops and rotations must still sample. No native-buffer lifetime, configure,
callback, or placeholder-release rules change. The adjacent reference library remains unavailable;
this bounded CPU optimization follows the existing coordinate contract, cross-checked against
the official [surface buffer-scale specification](https://raw.githubusercontent.com/wayland-mirror/wayland/main/protocol/wayland.xml)
and [viewporter transformation order](https://raw.githubusercontent.com/wayland-mirror/wayland-protocols/main/stable/viewporter/viewporter.xml).
Tests assert shared pixel allocation for integer/fractional identity mappings and correct pixels
for same-size cropped/rotated mappings. Returning early solely on matching dimensions was rejected
because it would ignore those transformations.

The CPU-only `native_density_image_preparation_timing` ignored test measures just this mapping step
over ten preparations of a 3840×2400 image at 300%. In a local unoptimized test build it changed from
296.7 ms to 2.3 microseconds per call. This excludes SHM reads, client drawing, retained-image copies,
GPU upload, and presentation; it is not an end-to-end resize latency claim. Run explicitly with:

```sh
cargo test -p telorgon --lib --no-default-features --features desktop-wayland-linux \
  native_density_image_preparation_timing --offline -- --ignored --nocapture
```

The adjacent `../other-rendering-libs` library was absent in this checkout. The routing in
[Reference implementations](REFERENCE_IMPLEMENTATIONS.md) was followed with available upstream
sources and locally installed dependency sources instead:

- Winit 0.30.13: `src/platform_impl/linux/wayland/window/state.rs` (logical inner size, scale changes,
  cursor coordinate conversion) and `src/platform_impl/linux/wayland/output.rs`; dpi 0.1.2:
  `src/lib.rs` (logical/physical separation and rounding). These are installed under the Cargo
  registry source tree. Invariant: output pixels and client logical geometry have distinct owners.
- Flutter engine: [`shell/platform/windows/flutter_windows_view.cc`](https://github.com/flutter/engine/blob/main/shell/platform/windows/flutter_windows_view.cc),
  `SendWindowMetrics` and `GetDpiScale`. Independent check: physical bounds and pixel ratio are
  delivered separately; UI size is not inferred from framebuffer width alone.
- Cosmic Text 0.19.0: `src/layout.rs`, `LayoutGlyph::physical`. The offset argument is already
  physical, so the logical line baseline must also be multiplied when rasterizing at higher density.
- Official [Wayland protocol](https://wayland.freedesktop.org/docs/html/apa.html), plus installed
  `staging/fractional-scale/fractional-scale-v1.xml`, `stable/viewporter/viewporter.xml`, and
  `unstable/relative-pointer/relative-pointer-unstable-v1.xml`: surface/buffer coordinate separation,
  scale announcements, positive half-away rounding of fractional buffer sizes, and relative motion.
- Official [libinput pointer API](https://wayland.freedesktop.org/libinput/doc/latest/api/group__event__pointer.html):
  normalized accelerated movement and device-space unaccelerated movement must not both be treated
  as KMS pixels. Official [Vulkan viewport specification](https://docs.vulkan.org/refpages/latest/refpages/source/VkViewport.html):
  framebuffer viewport coordinates/limits stay physical; the existing scene mapping performs scaling.

Rejected alternatives: classify every 4K display as 200%; render the entire desktop at 1080p then
upscale; divide all input vectors by output scale; downsample client buffers to logical resolution;
change font layout metrics to obtain sharper glyphs; scale each backend independently. These either
lose physical-size consistency/detail or let input, clipping, and geometry disagree.

CPU tests cover density selection and invalid overrides; invalid EDID fallback; fractional shared
edges, conservative damage and pointer conversions; placement/clip/radius conversion while
preserving presented revisions; unchanged text wrapping and doubled physical line spacing; glyph
atlas sampling/upload dimensions; SHM pixel preservation, fractional viewport sizing and invalid
buffer scales; and hardware cursor sizing/hotspots/fallback bounds. Existing desktop, text, software
renderer, compiler and compositor-image tests are also run. GPU tests are compiled only.

Manual qualification should compare the same shell at Fixed(1.0), Fixed(1.5), and Fixed(2.0), then
Auto: text sharpness, panel and window geometry, pointer/touch alignment, drag/resize, cursor fallback,
SHM and DMA-BUF applications, fractional/integer Wayland clients, and session-lock coverage. Monitor
EDID and client responses make these necessary in addition to CPU and compilation checks.

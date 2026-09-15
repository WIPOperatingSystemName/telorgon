# Telorgon Current Code Layout

## Status

This document describes the current consolidated workspace. It does not promote modeled or
operational subsystems to production-qualified status; current capability claims remain controlled
by [Implementation status](IMPLEMENTATION_STATUS.md).

Telorgon deliberately uses one public framework package. Former crate boundaries remain visible as
cohesive source modules so ownership, dependency direction, platform isolation, and test coverage
do not disappear merely because registry packaging was consolidated.

## Cargo packages

| Package | Published | Responsibility |
| --- | --- | --- |
| `telorgon` | Yes | Complete framework library, subsystem modules, facade, assets, tests, and benchmarks |
| `telorgon-macros` | Yes | Procedural `#[component]` implementation re-exported by `telorgon` |
| `telorgon-shader-build` | No | Offline shader compiler, validator, reflector, bundle hasher, and generated-source writer |

Applications add only `telorgon` to `Cargo.toml`. The macro companion is an implementation
dependency and the shader tool is not in the runtime dependency graph.

## Main package modules

All framework implementation source is under `crates/telorgon/src`:

| Module | Source directory | Responsibility |
| --- | --- | --- |
| `core` | `src/core` | Geometry, color, time, and shared values |
| `scene` | `src/scene` | Generational retained CPU scene storage |
| `input` | `src/input` | Platform-neutral pointer, keyboard, gesture, focus, and routing values |
| `ui` | `src/ui` | Mounted nodes, properties, semantics, overlays, and foundation storage |
| `compose` | `src/compose` | Component authoring, builders, signals, elements, and field plumbing |
| `runtime` | `src/runtime` | Persistent component ownership, state, reads, transactions, tasks, and reconciliation |
| `text` | `src/text` | Shaping, retained runs, editing, selection, sessions, glyph cache, and atlas data |
| `layout` | `src/layout` | Incremental layout, spatial state, scrolling, hit testing, and popup placement |
| `theme` | `src/theme` | Theme v4 parsing, compilation, resolution, scopes, archives, and motion |
| `accessibility` | `src/accessibility` | Semantic-tree validation, deltas, publication, and assistive actions |
| `application_primitives` | `src/application_primitives` | Application/game/embedded foundation primitives |
| `shell_primitives` | `src/shell_primitives` | Shell, output, surface, workspace, and system foundation primitives |
| `application_components` | `src/application_components` | Standard accessible application controls and adaptive components |
| `shell_components` | `src/shell_components` | System widgets and composed shell facilities |
| `shell` | `src/shell` | Protocol-neutral shell host truth, authority, requests, and results |
| `platform` | `src/platform` | Platform-neutral lifecycle, metrics, scheduling, capabilities, and services |
| `platform_conformance` | `src/platform_conformance` | Deterministic platform fixtures and fake services |
| `platform_winit` | `src/platform_winit` | Winit registry, scheduling, and event translation adapters |
| `platform_linux` | `src/platform_linux` | Linux session, input, and keymap bindings |
| `render` | `src/render` | Scene compilation, render planning, deltas, batches, and backend contract |
| `gpu_abi` | `src/gpu_abi` | Exact shader-visible POD records and layout constants |
| `material` | `src/material` | Logical material and pass descriptions |
| `renderer_software` | `src/renderer_software` | Deterministic CPU reference renderer |
| `renderer_vulkan` | `src/renderer_vulkan` | Vulkan device, resources, execution, hosting, interop, and readback |
| `presentation` | `src/presentation` | Neutral acquire/present/recovery and completion contracts |
| `presenter_softbuffer` | `src/presenter_softbuffer` | Native software transfer and presentation |
| `presenter_vulkan_wsi` | `src/presenter_vulkan_wsi` | Vulkan surfaces, swapchains, synchronization, and recovery |
| `presenter_dxgi` | `src/presenter_dxgi` | Windows D3D11/DXGI presentation |
| `bridge_vulkan_dxgi` | `src/bridge_vulkan_dxgi` | Windows Vulkan-to-DXGI texture and synchronization bridge |
| `presenter_vulkan_kms` | `src/presenter_vulkan_kms` | Linux DRM/GBM atomic KMS presentation |
| `wayland_server` | `src/wayland_server` | Bounded native Wayland protocol and server transport bindings |
| `compositor_wayland` | `src/compositor_wayland` | Wayland compositor state, roles, dispatch, input, and surface commits |
| `compositor_render` | `src/compositor_render` | SHM and DMA-BUF compositor-to-renderer bridge |
| `profiler` | `src/profiler` | Compile-optional bounded event production and capture ownership |
| `profiler_server` | `src/profiler_server` | Managed-only loopback profiler service and embedded viewer assets |
| `session` | `src/session` | Child environments, managed command/application launches, supervision, and recovery journals |
| `application_host` | `src/application_host` | Managed application preparation, orchestration, and backend assembly |
| `assets` | `src/assets.rs`, `src/assets` | Typed embedded catalogs, bounded raster/SVG media, icon profiles, and pointer themes |
| `window_chrome` | `src/window_chrome.rs` | Per-window metadata, semantic frame roles/actions, and layout-derived hit regions |
| `embed` | `src/embed` | Window-system-free host-driven Vulkan embedding |

`src/lib.rs` declares these modules, re-exports the curated public facade, and contains no
independent duplicate runtime. `telorgon::app::*` remains the ordinary authoring entry point.

## Feature isolation

The consolidated package retains focused build profiles:

- `application-software` enables Winit and Softbuffer managed presentation;
- `application-vulkan-windows` enables managed Vulkan WSI and the Windows DXGI bridge;
- `shell-wayland-linux` enables the Linux Wayland/KMS compositor assembly and is rejected on
  non-Linux targets;
- `embedded-vulkan` enables host-driven Vulkan without a managed window loop;
- `profiler` enables instrumentation plus the managed profiler service;
- `embedded-profiler` enables instrumentation without forcing the managed profiler service; and
- the private plumbing feature `instrumentation` is shared by the two public profiling profiles.

Optional native, renderer, profiler, and server dependencies remain absent when their profiles are
disabled. Platform-specific modules are guarded at their declarations as well as internally.

## Assets and generated source

- Checked-in Vulkan artifacts and their manifest live under
  `crates/telorgon/src/renderer_vulkan/shaders/vulkan`.
- Generated Rust shader metadata lives at
  `crates/telorgon/src/renderer_vulkan/generated_shader_bundle.rs`.
- The unpublished shader tool reads authored GLSL under `crates/telorgon-shader-build/shaders` and
  writes directly to those main-package paths.
- Profiler HTML, CSS, and JavaScript live under
  `crates/telorgon/src/profiler_server/assets` and are embedded with `include_str!`.
- Repository themes and protocol profiles remain under `themes` and `protocols`.

Downstream builds consume checked-in shader artifacts. They do not compile GLSL or depend on
`telorgon-shader-build`.

## Tests and benchmarks

Unit tests remain beside their implementation modules. Former per-crate integration tests were
moved into `crates/telorgon/tests` and prefixed with their owning module, for example
`platform__platform_metrics.rs`. This keeps each test as an independent integration-test target
while avoiding filename collisions.

Feature-specific integration tests carry crate-level `cfg` guards so `--no-default-features`
remains a valid build. Hardware-presenting Vulkan fixtures remain ignored unless the documented
developer hardware environment explicitly opts in.

The umbrella facade tests, component tests, and benchmarks remain directly under
`crates/telorgon/tests` and `crates/telorgon/benches`.

## Dependency direction

Although Cargo no longer enforces every former library edge as a package boundary, the source
architecture retains the same downward direction:

```text
core / scene
      |
      v
input / ui / text / theme / platform values
      |
      v
compose / runtime / layout / accessibility
      |
      v
application and shell primitives/components
      |
      v
render / material / presentation contracts
      |
      v
software and Vulkan renderer modules
      |
      v
platform adapters / presenters / compositor bridges
      |
      v
application_host / embed / public facade
```

Lower modules must not import managed hosts, concrete presenters, or shell policy. The application
and shell design domains remain siblings. Platform and renderer-specific dependencies stay behind
features and target conditions. Architecture tests and review enforce these rules now that Cargo
package edges no longer provide the boundary automatically.

## Publication boundary

The main package must not contain path dependencies on unpublished runtime crates. The only local
library dependency is the exact-version `telorgon-macros` companion required by Rust's procedural
macro crate rules. See [Cargo publishing](PUBLISHING.md) for validation and release order.

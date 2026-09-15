# Telorgon Documentation

See [Shell application catalog](APPLICATION_CATALOG.md) for inherited shell context, installed-app
metadata/search, asynchronous icons, and the test taskbar.

See [Resize-preview glass](RESIZE_GLASS.md) for the color/glass API, Vulkan cache, and qualification
limits, and the [liquid-glass optimization audit](LIQUID_GLASS.md) for the shader mathematics.

Telorgon's documentation deliberately separates the intended product from the code that exists today.
Start here so architectural proposals are not mistaken for implemented or production-qualified
features.

> Packaging consolidation: the framework implementation now lives in named modules of the single
> `telorgon` package. `telorgon-macros` is the required published procedural-macro companion and
> `telorgon-shader-build` is an unpublished repository tool. Historical milestone text may still
> use former `telorgon-*` crate names to identify ownership; [Code layout](CODE_LAYOUT.md) controls
> their current module paths and [Cargo publishing](PUBLISHING.md) controls the registry surface.

See [Logical units and output scaling](LOGICAL_UNITS_AND_OUTPUT_SCALING.md) for Linux desktop
HiDPI behavior, boot density selection, configuration migration, and qualification limits.

See [Desktop window motion](WINDOW_MOTION.md) for chrome motion presets, maximize/minimize and
resize-content transitions, implementation limits, and user-run qualification. The
[original API plan](WINDOW_MOTION_API_PLAN.md) records the design rationale.

See [Component-owned shell widgets](SHELL_WIDGETS.md) for shell registration, surface descriptors,
geometry animation, live window services, and migration from compositor backgrounds.

## Reading order

1. [Project scope and architecture](PROJECT_SCOPE_AND_ARCHITECTURE.md) defines Telorgon's mission,
   product boundaries, two public UI domains, presentation modes, and target module architecture.
2. [Implementation status](IMPLEMENTATION_STATUS.md) classifies current capabilities as planned,
   modeled, operational, or production-qualified.
3. [Render backend architecture](RENDER_BACKEND_ARCHITECTURE.md) specifies the proposed
   Vulkan-first, cross-API rendering boundary.
4. [Presentation-pipeline restructuring plan](PRESENTATION_PIPELINE_RESTRUCTURING_PLAN.md) defines
   the staged separation of the currently implemented renderers, presentation bridges, native
   presenters, and application orchestration.
5. [GPU-first Vulkan implementation plan](VULKAN_IMPLEMENTATION_PLAN.md) selects the Rust packages,
   dependency boundaries, interface corrections, and ordered milestones for real GPU rendering.
6. [File and API implementation blueprint](IMPLEMENTATION_BLUEPRINT.md) fixes the concrete crate,
   file, ownership, and API boundaries for the first three implementation slices.
7. [Current-to-target migration plan](MIGRATION_PLAN.md) assigns every current crate and public type
   a destination, compatibility rule, move order, and deletion condition.
8. [GPU ownership and synchronization](GPU_OWNERSHIP_AND_SYNCHRONIZATION.md) fixes device, frame,
   target, completion, swapchain, hosted-recording, destruction, readback, and recovery lifetimes.
   [Reactive resize and presentation](REACTIVE_RESIZE_AND_PRESENTATION.md) records the implemented
   logical-metrics/surface-commit split and its remaining hardware qualification.
9. [Scene-to-GPU ABI and shader contract](SCENE_GPU_ABI_AND_SHADERS.md) fixes retained-scene
   identity and deltas, GPU byte layouts, descriptor bindings, shaders, batching, uploads, color,
   and resource-use lowering.
10. [Platform implementation order](PLATFORM_IMPLEMENTATION_ORDER.md) fixes the Windows, hosted,
   Linux, shell/compositor, Metal/macOS, Android, and iOS delivery sequence and package isolation.
11. [Acceptance and qualification](ACCEPTANCE_AND_QUALIFICATION.md) fixes evidence layers, shared
    test matrices, hardware-run policy, visual tolerances, validation, device profiles, waivers, and
    production reports.
12. [Authoring and component runtime](AUTHORING_AND_COMPONENT_RUNTIME.md) documents the current
    builder/component/signal composition API, annotated input/state ownership, keyed
    reconciliation, lifecycle, events, and sealed runtime roots.
13. [Application and shell primitives](APPLICATION_AND_SHELL_PRIMITIVES.md) fixes the foundation,
    primitive, component, and facility cut; accessible control behavior; adaptive input/layout;
    catalog tiers; and protocol-neutral shell UI boundary.
14. [Platform integration contract](PLATFORM_INTEGRATION_CONTRACT.md) fixes lifecycle axes, managed
    and embedded host ownership, input/IME/accessibility mapping, platform services, data transfer,
    native handles, and shell-host integration.
15. [Reference-implementation study guide](REFERENCE_IMPLEMENTATIONS.md) routes graphics and UI work
   to relevant sources in the adjacent `other-rendering-libs` library and defines the required audit.
16. [Code layout](CODE_LAYOUT.md) maps the crates and source organization that exist today to the
   intended package structure.
17. [UI widgets](UI_WIDGETS.md) documents the current retained UI authoring path and points toward
   the separate application and shell domains.
18. [Theme runtime](THEME_RUNTIME.md) documents current styling behavior and the planned theme-domain
   split.
19. [Performance](PERFORMANCE.md) defines current CPU evidence and modeled-renderer caveats, with
    Gate 6 controlling future hardware qualification.
20. [Development profiler](PROFILER.md) defines and tracks the compile-excluded instrumentation,
    `cargo profile` workflow, bounded local service, event protocol, GPU timing, browser UI, and
    remaining qualification gates.
21. [Wayland compositor architecture](WAYLAND_COMPOSITOR_ARCHITECTURE.md) documents the implemented
    Linux-only protocol/server, Telorgon composition, input/session, rendering, DMA-BUF, and atomic
    KMS path, including the exact advertised protocol profile and remaining qualification gaps.
22. [Cargo publishing](PUBLISHING.md) records the registry prerequisites, validation commands, and
    dependency-ordered first-release sequence.
23. [Custom windows, assets, icons, and pointers](CUSTOM_WINDOWS_ASSETS_AND_POINTERS.md) documents
    the implemented composable frame contract, reusable box decoration, typed project media,
    application icons, pointer themes, Wayland invariants, and complete API examples.

24. [Managed sessions and process launching](SESSION_AND_PROCESS_LAUNCHING.md) documents automatic
    entrypoint setup, the global command/application API, shutdown, recovery, and qualification limits.
25. [X11 compatibility](X11_COMPATIBILITY.md) documents the optional managed integration and
    remaining gaps; [X11 qualification](X11_QUALIFICATION.md) records actual evidence, and the
    [manual smoke test](X11_SMOKE_TEST.md) defines the first live acceptance procedure.

## Document roles

| Role | Documents | Meaning |
|---|---|---|
| Product and target design | `PROJECT_SCOPE_AND_ARCHITECTURE.md`, `RENDER_BACKEND_ARCHITECTURE.md`, `PROFILER.md` | Working specifications. APIs and package names are proposals until implemented and validated. |
| Active implementation direction | `PRESENTATION_PIPELINE_RESTRUCTURING_PLAN.md`, `VULKAN_IMPLEMENTATION_PLAN.md`, `IMPLEMENTATION_BLUEPRINT.md`, `MIGRATION_PLAN.md`, `GPU_OWNERSHIP_AND_SYNCHRONIZATION.md`, `SCENE_GPU_ABI_AND_SHADERS.md`, `PLATFORM_IMPLEMENTATION_ORDER.md`, `ACCEPTANCE_AND_QUALIFICATION.md`, `AUTHORING_AND_COMPONENT_RUNTIME.md`, `APPLICATION_AND_SHELL_PRIMITIVES.md`, `PLATFORM_INTEGRATION_CONTRACT.md` | Concrete presentation separation, GPU-first phase policy, implementation/platform order, exact initial file/API/lifetime/data/shader/component/control/platform boundaries, current-to-target moves, acceptance evidence, and exit criteria. |
| Reference engineering | `REFERENCE_IMPLEMENTATIONS.md` | Required read-only comparison workflow, concern-to-source routing, pitfall checklist, and provenance rules. |
| Current implementation | `IMPLEMENTATION_STATUS.md`, `CODE_LAYOUT.md`, `UI_WIDGETS.md`, `THEME_RUNTIME.md`, `WAYLAND_COMPOSITOR_ARCHITECTURE.md` | Describes what is present in the repository now, with links to the target design where useful. |
| Evidence and qualification | `ACCEPTANCE_AND_QUALIFICATION.md`, `PERFORMANCE.md` | Gate 6 controls future test/qualification evidence and reports; Performance describes current evidence and performance principles. |

## Source-of-truth rules

- If a target document and the implementation-status table appear to disagree about availability,
  `IMPLEMENTATION_STATUS.md` controls the current-state claim.
- A CPU model of GPU resource or frame behavior is labeled **modeled**. It is not described as a
  Vulkan implementation or Vulkan performance result.
- During the current rendering phase, new renderer engineering goes to real Vulkan. The software
  renderer remains a deterministic reference and temporary host path, not an implementation stage
  that Vulkan must follow.
- For implementation slices 1–3, `IMPLEMENTATION_BLUEPRINT.md` controls file responsibilities,
  ownership boundaries, and initial API shape. Later planning gates may refine signatures before
  implementation begins, but changes must be reflected there explicitly.
- For device/frame/target/presentation/hosted lifetimes, completion proof, and destruction,
  `GPU_OWNERSHIP_AND_SYNCHRONIZATION.md` controls and the Blueprint must agree with it.
- For retained-scene identity and deltas, GPU records, descriptor and shader interfaces, batching,
  uploads, color/alpha, and semantic use lowering, `SCENE_GPU_ABI_AND_SHADERS.md` controls.
- For the managed, hosted, desktop, shell/compositor, Metal, and mobile implementation sequence and
  platform feature isolation, `PLATFORM_IMPLEMENTATION_ORDER.md` controls.
- For evidence layers, test outcomes, shared conformance suites, hardware skips, goldens, validation,
  performance profiles, device matrices, waivers, and production reports,
  `ACCEPTANCE_AND_QUALIFICATION.md` controls.
- For component lifecycle, owner-scoped state/reactivity, transactions, typed actions, keyed
  reconciliation, scoped tasks, and neutral text-editing storage,
  `AUTHORING_AND_COMPONENT_RUNTIME.md` controls.
- For the foundation/primitive/component cut, application and shell catalogs, standard control
  behavior, adaptive input/layout, semantics, overlays/controllers, and protocol-neutral shell UI,
  `APPLICATION_AND_SHELL_PRIMITIVES.md` controls.
- For lifecycle axes, view metrics, managed and embedded host behavior, event translation, native
  IME, accessibility export, platform services, data transfer, native handles, and shell-host
  integration, `PLATFORM_INTEGRATION_CONTRACT.md` controls.
- `MIGRATION_PLAN.md` controls the disposition of existing crates, types, import paths, temporary
  bridges, and deletions. Target package tables do not authorize duplicating an implementation or
  preserving a dependency cycle for compatibility.
- `PRESENTATION_PIPELINE_RESTRUCTURING_PLAN.md` controls the staged separation of the implemented
  renderer, cross-API bridge, native presenter, and application-orchestration responsibilities. It
  does not authorize speculative backend crates.
- `PROFILER.md` is the development-tool contract and implementation record. Its implemented paths
  do not become production-qualified until `IMPLEMENTATION_STATUS.md` and the acceptance reports
  record the required manual, performance, browser, and hardware evidence.
- A retained description of external surfaces is not described as an operational compositor.
- Closed graphics APIs are adapter targets, not claimed platform support. Support requires a real
  backend built and tested with the relevant vendor SDK.
- Adjacent source projects are read-only references. Their invariants and failure cases should inform
  Telorgon tests, but their code and abstractions are not copied or treated as specifications.
- New documents must say whether they describe current behavior, a target contract, or qualification
  evidence.

## Retired documents

Milestone-specific migration notes and superseded runtime plans are intentionally not part of the
active documentation set. Their completed work is represented by the current-state documents above;
historical details remain available through version-control history. The former compositor runtime
and scene-model documents were retired with `telorgon-compositor`; protocol-neutral surface contracts
now belong to `telorgon-shell`, and external-image execution belongs to the renderer/host boundary.

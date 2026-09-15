# Telorgon Project Scope and High-Level Architecture

## Document status

This document is the working scope and architecture draft for Telorgon. It records current product
goals and proposed architectural consequences so they can be challenged before they become stable
public contracts. It is not a claim that every subsystem described here is implemented, and it does
not freeze the illustrative API names. Implementation status must be recorded separately, and tests
must distinguish modeled behavior from operational platform or GPU integration.

The current capability classification is maintained in
[IMPLEMENTATION_STATUS.md](IMPLEMENTATION_STATUS.md). Acceptance evidence and production
qualification are controlled separately by
[ACCEPTANCE_AND_QUALIFICATION.md](ACCEPTANCE_AND_QUALIFICATION.md).

## 1. Mission

Telorgon is a lightweight, high-performance Rust GUI toolkit for native desktop, touchscreen, and
mobile applications. It uses a retained, data-oriented UI and scene architecture to minimize CPU
work, memory use, GPU uploads, and redraws while providing consistent custom-rendered interfaces
across platforms.

Telorgon is designed to support:

- ordinary application GUIs, from small utilities to editor-scale desktop software;
- touch-first and mobile applications;
- desktop shells, panels, launchers, window chrome, and system overlays;
- compositor or host-owned surfaces embedded into a Telorgon interface; and
- deterministic headless rendering for tests, export, and tooling.

Desktop-shell and compositor integration is an important use case, but it does not define or own the
core toolkit. A shell uses a dedicated public shell primitive/component domain over the same
runtime, layout, input, text, accessibility, scene, and rendering foundations used by the public
application domain.

The central performance promise is:

> Application authors describe persistent interfaces, and Telorgon performs work proportional to what
> changed.

An idle interface performs no frame work. A paint-only change does not trigger structural or layout
work. A local layout change does not rebuild the application. Unchanged text is not reshaped.
Unchanged GPU resources are not recreated or uploaded.

### 1.1 Ordered product goals

These goals are ordered. When two choices conflict, an earlier goal normally wins unless a written
architecture decision explains why the tradeoff is necessary.

#### Goal 1: Be easy to learn, use, and understand

An application author should be able to create a window and compose a useful interface without
understanding scene storage, GPU synchronization, render graphs, or platform event loops. The
default path must have sensible behavior and a small vocabulary. Advanced control remains
available through progressively lower layers rather than parameters on every common component.

This requires:

- one umbrella package with a batteries-included starting path;
- a short application entry point and one normal component model;
- conventional Rust ownership and typed actions without application-facing unsafe code;
- consistent layout, styling, state, and event conventions across all components;
- standard accessible components that do not require users to assemble basic behavior manually;
- errors and diagnostics that identify the component, property, and violated constraint;
- examples that progress from a single window to an embedded UI and a shell; and
- escape hatches that do not complicate the common path.

Ease of use is a performance feature as well as an API feature. Applications should receive good
retention, batching, accessibility, and idle behavior by default rather than through expert-only
configuration.

Acceptance for this goal includes a small complete application that needs only the umbrella
package, an understandable component lifecycle, API documentation with runnable examples, and user
testing that can identify concepts developers repeatedly misunderstand.

#### Goal 2: Make desktop shells and compositor GUIs straightforward

Telorgon should make it practical to build panels, launchers, workspaces, task switchers, system
overlays, window chrome, and complete graphical shells. It should also provide the visual scene and
external-surface integration needed by a compositor.

Telorgon's portable shell, scene, renderer-contract, and component crates do not depend on Wayland,
X11, input-device, or console protocol types. An external protocol/policy host may supply outputs,
seats, client surfaces, damage, synchronization, metadata, and commands through typed interfaces.
For the Linux-only shell-environment application type, focused first-party crates implement a
Wayland server over the official XML and `libwayland-server`, Linux input/session integration, and
atomic KMS presentation while adapting all of that state into the same neutral Telorgon contracts.

This boundary allows a shell to use a Wayland library, an existing compositor core, a mobile system
service, or a proprietary platform layer without forking Telorgon UI. Zero-copy external image import
and explicit synchronization are renderer capabilities, not protocol implementations.

Acceptance for this goal includes building a multi-output shell UI from ordinary Telorgon components,
embedding changing client surfaces without CPU readback, and replacing the protocol host with a
test fixture while preserving shell behavior.

#### Goal 3: Work as a standalone toolkit or a well-behaved embedded library

Telorgon must support three first-class deployment profiles:

- **managed application:** Telorgon owns the desktop window and event-loop convenience layer;
- **hosted application:** a mobile, embedded, or platform host owns lifecycle and presentation; and
- **embedded UI:** another renderer or engine owns the graphics device, queues, frame loop, render
  graph, and target.

The application domain therefore has two equally deliberate entry points:

- **request presentation:** give Telorgon a root component and window/view options; Telorgon requests a
  native presentation surface, selects the configured renderer, manages resize and presentation,
  and drives frames on demand; and
- **provide a render area:** give Telorgon a root component, host renderer context, target attachment
  or texture, rectangular viewport, scale, input mapping, and frame context; Telorgon records only the
  UI work for that area and never presents it independently.

Both entry points use the same application primitives and components. Moving a UI between them must
not require rewriting its component tree.

Game engines and other render hosts must not be forced to create a second graphics device, run a
second event loop, accept hidden background threads, or surrender frame scheduling. An embedded UI
can consume host input and time, update on demand, record into a host-provided frame context, and
render into a host-provided target. Resource creation, destruction, synchronization, and failures
are explicit.

Telorgon must not install global allocators, global loggers, panic hooks, or process-wide platform
state as a side effect of constructing an embedded runtime. Optional worker threads and render
threads are host-configured. Idle embedded UIs do not add work to the host frame beyond a cheap
needs-update query.

Acceptance for this goal includes the same component running under the managed desktop host and a
test embedding host, sharing an existing Vulkan device and frame schedule, and reporting measurable
CPU work, allocations, uploads, and passes attributable to Telorgon. It also includes multiple
independent UI views sharing one device/cache context while recording into separate host-selected
targets or target regions in one frame.

#### Goal 4: Be Vulkan-first and graphics-API adaptable

Vulkan is the intended reference production graphics backend and the first backend against which
resource lifetime, synchronization, batching, memory, failure recovery, and profiling will be made
operational.
The architecture is nevertheless designed so Metal, Direct3D, Nintendo, PlayStation, and other
modern explicit graphics APIs can be implemented without changing UI, layout, text, theme, or scene
packages.

Telorgon does not expose Vulkan types in its general UI or scene APIs. Its internal rendering hardware
interface is based on capabilities and explicit GPU concepts rather than a promise that every API
looks identical. Backends may expose narrowly scoped native interop through backend-specific
extension traits.

Console backends can live in private vendor packages and implement the same backend contracts. The
open repository does not assume access to proprietary SDKs, headers, shader compilers, or protocol
details.

Acceptance for this goal begins with a real Vulkan device, swapchain, command submission, shader
execution, synchronization, and presentation path. Portability is not proven until a materially
different backend or a conformance test backend implements the abstraction without changes to the
retained scene contract.

The current implementation phase is GPU-first: preserve the existing software renderer as a
deterministic reference, but direct new renderer engineering immediately to real Vulkan. Package
selection, interface corrections, and ordered exit criteria are defined in
[VULKAN_IMPLEMENTATION_PLAN.md](VULKAN_IMPLEMENTATION_PLAN.md).

#### Goal 5: Remain modular in packages and source structure

Telorgon is a family of focused packages with explicit dependency direction, not one framework crate
containing every subsystem. The umbrella package provides convenience by re-exporting selected
packages; it does not erase their boundaries or force every feature into every application.

Within a package, each source file owns one cohesive concept or closely related type family. Crate
roots primarily declare modules and public exports. Platform code, backend code, protocol adapters,
standard components, and tools do not accumulate in shared runtime files.

Modularity must improve comprehension, testing, compilation, feature selection, and replacement.
It must not become hundreds of tiny files that split one algorithm across arbitrary boundaries.

Acceptance for this goal includes an acyclic package graph, minimal dependency packages that build
independently, backend and platform selection through packages/features, and code-review checks that
reject new monotelorgon modules or unrelated responsibilities added to existing files.

## 2. Product boundaries

### 2.1 Telorgon owns

- application and component lifecycle;
- persistent UI identity and reactive state propagation;
- layout, hit testing, focus, input routing, and gesture arbitration;
- text editing, shaping, font fallback, selection, and IME integration;
- accessibility semantics and platform accessibility export;
- themes, visual states, animation, images, icons, and materials;
- backend-neutral retained scenes, damage, batching metadata, and resource deltas;
- software reference rendering and production GPU renderer integrations;
- desktop and mobile platform adapters;
- embedding contracts for native, external, and compositor-owned surfaces; and
- first-party components, adaptive controls, development tools, and shell-building components.

### 2.2 Telorgon does not own

- operating-system policy, permissions, sessions, or process management;
- compositor protocols, window-placement policy, or page-flip policy;
- application business logic, persistence, networking, or service architecture;
- a JavaScript, Dart, or other managed-language runtime;
- wrappers around platform-native widget hierarchies; or
- an unrestricted browser document and CSS compatibility engine.

Platform and shell integrations may connect Telorgon to these systems through explicit interfaces.
They must not leak platform policy into the core UI runtime.

## 3. Design principles

1. **Retain identity, not reconstruction work.** Components mount stable nodes once. Ordinary state
   updates patch typed properties. Dynamic structure is reconciled explicitly and by stable keys.
2. **One canonical result per concern.** Layout owns geometry. Semantics owns accessibility meaning.
   The scene owns render order. Other systems consume those results rather than recomputing them.
3. **Separate authoring ergonomics from runtime representation.** A friendly component API compiles
   or mounts into compact node and component storage. Runtime storage is not required to resemble
   the authoring syntax.
4. **Make invalidation precise.** Structure, measure, arrange, spatial transform, text shaping,
   semantics, paint, and compositing are separate dirty domains.
5. **Use Vulkan as the reference, not as leaked public policy.** Vulkan is intended to drive the
   first operational GPU renderer and validate explicit resource and synchronization design. UI and
   scene packages do not depend on Vulkan, Metal, Direct3D, console APIs, or a particular
   portability library.
6. **Use persistent resources.** Frame slots, pipelines, buffers, descriptors, textures, atlases,
   intermediate targets, and imported surfaces survive across ordinary frames.
7. **Demand-driven by default.** Input, animation deadlines, external-surface damage, and explicit
   requests schedule frames. Merely owning a window does not create a render loop.
8. **Desktop and touch are peers.** Mouse, keyboard, touch, pen, IME, accessibility, safe areas, and
   density scaling are architectural inputs, not later compatibility layers.
9. **Small core, layered capability.** Advanced widgets, shell facilities, vector effects, and
   platform services build above a stable set of primitives.
10. **Measure operational behavior.** A modeled backend cannot satisfy a GPU performance gate.
    Release claims require end-to-end measurements on real backends and devices.
11. **Respect host ownership.** Embedded use does not seize the event loop, graphics device, frame
    schedule, threads, logging, allocation policy, or process-wide state.
12. **Make boundaries visible in code.** Packages and files reflect subsystem ownership and
    dependency direction. Convenience is provided by composition and re-exports, not monoliths.

## 4. System architecture

```text
Application or desktop shell
        |
        v
Components, actions, reactive state, and tasks
        |
        v
Mounted UI runtime
  stable nodes | typed properties | keyed structure | semantics
        |
        +--------------------------+
        |                          |
        v                          v
Layout and spatial state      Input and interaction
measure | arrange | clips     pointer | touch | pen | focus | IME
        |                          |
        +-------------+------------+
                      |
                      v
Text, images, themes, animation, and materials
                      |
                      v
Backend-neutral retained scene
instances | order | damage | resource and range deltas
                      |
                      v
Scene renderer
software reference | Vulkan | Metal | Direct3D | selected portability backend
                      |
                      v
Platform presentation
desktop window | mobile view | embedded target | compositor target
```

The architecture is divided into contracts so that application authoring, runtime storage, scene
compilation, rendering, and platform presentation can evolve independently.

### 4.1 Operating profiles

All profiles use the same component runtime, layout, input, semantics, text, theme infrastructure,
and scene systems. A profile selects the application domain, shell domain, or both. Profiles also
differ in who supplies platform services, GPU ownership, scheduling, and presentation.

| Profile | Telorgon owns | Host owns |
| --- | --- | --- |
| Managed desktop | Components, runtime, renderer, windows, event loop, and presentation convenience | Application state and optional platform services |
| Hosted/mobile | Components, runtime, and normally the renderer | Platform lifecycle, native view, system services, and frame callbacks |
| Embedded/game engine | Components, runtime, scene compilation, and Telorgon GPU resources | Device, queues, frame loop, render graph, target, input translation, and synchronization boundary |
| Shell/compositor | Components, shell scene, UI interaction, and rendering of UI plus imported surfaces | Protocols, client lifecycle, outputs/seats, policy, surface metadata, and system commands |
| Headless | Runtime, deterministic layout, semantics, scene, and optional software output | Test clock, inputs, fixtures, and artifact handling |

### 4.2 Progressive API layers

The framework exposes three deliberate levels:

1. **Application API:** `run`, components, state, standard controls, themes, and common platform
   services. This is the documented default.
2. **Host API:** explicit lifecycle, input, time, viewport, accessibility, and frame scheduling for
   mobile, embedded, engine, and shell hosts.
3. **Renderer/backend API:** scene deltas, render plans, graphics capabilities, external resources,
   command recording, and synchronization for renderer and platform implementers.

A developer should not need renderer types to build an application, and a renderer implementer
should not need application component types to consume a scene.

An embedded host should be able to drive Telorgon without hidden control flow:

```rust,ignore
let mut ui_host = UiHost::new(hosted_ui_device, platform_services);
let view = ui_host.create_view(root_component);

view.handle_input(host_input);
view.set_viewport(viewport);

if view.needs_update(host_time) {
    let prepared = ui_host.prepare(host_time)?;
    host_render_graph.add_pass("telorgon-ui", |frame, target| {
        prepared.record(view, frame, RenderArea::from_target(target, viewport))
    });
}
```

This is an illustrative ownership contract. Exact names remain a design decision.

### 4.3 Surface and view terminology

The word “surface” is overloaded in graphics and compositor systems, so Telorgon distinguishes these
concepts:

| Term | Meaning |
| --- | --- |
| `UiView` | One mounted component root with its own viewport, layout, focus, input routing, semantics, scene, and damage |
| `PresentationSurface` | A native window/view presentation object and its swapchain or equivalent image sequence |
| `RenderTarget` | A host- or Telorgon-owned image/attachment usable for one frame |
| `RenderArea` | A rectangle, transform, scale, and clip within a `RenderTarget` into which one `UiView` records |
| `UiDevice` | Shared renderer/device resources such as pipelines, samplers, glyph/image atlases, and upload infrastructure |
| `ExternalSurface` | Content supplied to Telorgon for composition, such as a game viewport, video, or compositor client surface |

Rendering a `UiView` into a host `RenderArea` is Telorgon-inside-host embedding. Displaying an
`ExternalSurface` inside a Telorgon component is host-content-inside-Telorgon embedding. The APIs and
resource ownership for these opposite directions remain distinct.

## 5. Application and component model

Gate 7 fixes the implementation-level semantics for this section in
[Authoring and component runtime](AUTHORING_AND_COMPONENT_RUNTIME.md). The names below summarize that
contract; [Gate 8](APPLICATION_AND_SHELL_PRIMITIVES.md) fixes application/shell foundations,
primitives, components, facilities, and constructors without creating a second runtime.

### 5.1 Application

The normal entry point accepts a root `Component`. A lightweight default adapter supplies application
lifecycle behavior, while an advanced `Application` scope owns multi-view roots, application-scoped
tasks/commands, and explicit long-lived models. Platform suspension retains component state unless
application policy closes the view.

The normal entry point should be small:

```rust,ignore
fn main() -> telorgon::Result<()> {
    telorgon::run(NotesApplication::default())
}
```

Platform-specific configuration is optional and additive:

```rust,ignore
telorgon::ApplicationHost::new(NotesApplication::default())
    .title("Notes")
    .initial_size((960, 640))
    .minimum_size((480, 320))
    .run()
```

Mobile hosts provide the same application lifecycle through the platform entry point rather than
requiring application code to own an event loop.

### 5.2 Component

A component has retained immutable construction configuration, one runtime-owned state record, a
typed action, lifecycle hooks, and a mounted subtree. `create` and `mount` each run once for a mounted
identity. Ordinary state changes reevaluate only demanded reactive reads and patch mounted
properties; they do not rerun the component declaration or construct a transient widget tree.

```rust,ignore
struct Counter;

struct CounterState {
    count: State<i32>,
}

enum CounterAction {
    Increment,
}

impl Component for Counter {
    type State = CounterState;
    type Action = CounterAction;

    fn create(&self, cx: &mut CreateContext<'_>) -> CounterState {
        CounterState { count: cx.state(0) }
    }

    fn mount(&self, state: &CounterState, ui: &mut Ui<'_, Self::Action>) -> UiRoot {
        ui.column(|ui| {
            ui.text(state.count.read().map(|count| format!("Count: {count}")));
            ui.button("Increment", |_| CounterAction::Increment);
        })
    }

    fn action(
        &self,
        state: &mut CounterState,
        action: Self::Action,
        cx: &mut UpdateContext<'_, Self>,
    ) {
        match action {
            CounterAction::Increment => {
                let next = cx.get(state.count) + 1;
                cx.set(state.count, next);
            }
        }
    }
}
```

Gate 8 owns the `column`, `text`, and `button` authoring constructors in this example. Gate 7 fixes
the component/state/action shape and these properties:

- component declarations are concise and strongly typed;
- nodes and components receive separate stable identities when mounted;
- state writes occur in transactions and coalesce;
- derived values track dependencies;
- actions are moved typed values and cross component boundaries only through an explicit map; and
- mounting, updating, and structural reconciliation are observable as different operations.

### 5.3 State and binding

The state layer contains four concepts:

- `State<T>` owns mutable application or component state;
- `Read<T>` exposes a read-only reactive value;
- `Property<T>` is a stable handle to a mounted node property; and
- `Transaction` applies and coalesces related changes atomically.

State handles are generational, view-local, and writable only through an authorized owner context.
Derived reads register dependencies while evaluated. When a source changes, only dependent reads
become dirty, and equal output suppresses downstream work. Generic implicit two-way binding is not
part of the model: children receive `Read<T>` inputs and return typed actions.

Observers map committed read changes into an action for a later transaction. They never mutate the
tree during dependency traversal. Transactions stage state, structure, commands, and task starts,
then validate and commit atomically.

Async work never receives mutable UI references. It returns typed messages or actions to the UI
thread through a host executor. Unmount closes the component generation and cancels scoped work;
late task results are discarded. No concrete async runtime appears in the component API.

### 5.4 Dynamic structure

Most updates are property updates. Structure changes through explicit primitives:

- `when` mounts or unmounts conditional content;
- `switch` selects one keyed branch;
- `for_each_keyed` reconciles collections by stable application keys;
- `portal` mounts content into another visual layer while preserving logical ownership; and
- imperative insert, remove, replace, and move operations support advanced hosts.

Keys are local to one structural container and include their Rust key type in identity. Duplicate
keys reject the transaction. Unkeyed collection reconciliation must never silently preserve
incorrect identity. Keyed moves preserve component state, focus, accessibility identity, tasks,
nodes, and animation state without visiting child subtrees.

### 5.5 Application presentation entry points

#### Telorgon-owned presentation

For a complete cross-platform application, the entry point requests a native presentation surface
and hides ordinary platform and renderer setup:

```rust,ignore
fn main() -> telorgon::Result<()> {
    telorgon::application(NotesApplication::default())
        .window(WindowOptions::new("Notes").size(960, 640))
        .renderer(RendererPreference::Auto)
        .run()
}
```

The managed host creates the platform window/view, requests the selected renderer's compatible
presentation object (a Vulkan surface or Metal drawable path), chooses a device/queue, manages
presentation resources, translates platform input, schedules only needed frames, and presents them.
Defaults make `.renderer(...)` optional. `Auto` selects only a compiled, operational GPU backend for
the active Gate 5 profile; it does not silently select software.

Multiple application windows create multiple `UiView` and `PresentationSurface` pairs while sharing
one compatible `UiDevice` where the platform and backend permit it.

#### Host-provided render area

For a game engine, editor, media engine, or existing renderer, the entry point accepts a host-owned
device context and creates one or more UI views without creating a window or swapchain:

```rust,ignore
let ui_device = telorgon_vulkan::UiDevice::from_host(vulkan_context, host_allocator)?;
let mut ui_host = telorgon::embed::UiHost::new(ui_device, platform_services);

let hud = ui_host.create_view(hud_metrics, GameHud::new())?;
let tools = ui_host.create_view(tool_metrics, EditorTools::new())?;

ui_host.send_input(hud, input_for_hud)?;
ui_host.send_input(tools, input_for_tools)?;

let hud_work = ui_host.prepare(hud, hud_frame_input)?;
let tool_work = ui_host.prepare(tools, tool_frame_input)?;
render_graph.add_pass("hud", |frame| {
    hud_work.record(frame, RenderArea::new(main_target, hud_rect, scale))
});
render_graph.add_pass("tools", |frame| {
    tool_work.record(frame, RenderArea::new(editor_target, tools_rect, scale))
});
```

This is illustrative target API. The ownership requirements are normative:

- one `UiDevice` shares pipelines, shader modules, samplers, atlases, upload storage, and caches
  across compatible views;
- each `UiView` independently retains component state, layout, input/focus state, scene instances,
  animation deadlines, and damage;
- `prepare` performs required CPU work once and exposes explicit work for the host frame;
- `record` writes into a host-approved command/frame context and does not submit or present;
- the host chooses target ordering and composes Telorgon with world rendering, post-processing, video,
  and other UI systems;
- views that did not change do not add upload or recording work unless the host target requires a
  redraw; and
- destroying a view releases its per-view resources without destroying shared device resources.

#### Render-area contract

A `RenderArea` describes enough information to render without guessing host state:

- target image or attachment identity and lifetime for the current frame;
- pixel rectangle, logical size, scale factor, coordinate transform, and clip;
- format, color space, alpha mode, sample count, and origin convention;
- load/store intent and whether existing target contents must be preserved;
- initial and final resource usage expected by the host;
- frame slot and synchronization context;
- optional target damage or redraw constraints; and
- debug and accounting labels.

The host can request direct recording into a target region or ask Telorgon to maintain an offscreen UI
texture that the host composites later. Offscreen mode has an explicit cache and update policy; it
is not silently selected for every view.

Input delivered to a view includes the mapping from host coordinates into that view's logical
coordinates. Telorgon clips hit testing to the render area and does not consume input intended for
another engine layer unless the host routes it to that view.

## 6. UI foundation and domain vocabulary

Gate 8 fixes the implementation-level catalog, behavior, accessibility, controller, adaptive, and
package boundaries for sections 6–9 in
[Application and shell primitives](APPLICATION_AND_SHELL_PRIMITIVES.md). The lists here are a
high-level product summary; the Gate 8 layer classification controls when an older name below could
otherwise be mistaken for a low-level primitive.

Telorgon exposes exactly two public design domains:

1. the **application domain** for desktop applications, mobile applications, tools, games, game
   editors, HUDs, and embedded product interfaces; and
2. the **shell domain** for desktop environments, compositor scenes, system chrome, taskbars,
   launchers, workspaces, notifications, and system overlays.

The domains share internal foundation atoms: mounted identity, box and text scene instances,
layout, input, semantics, state, animation, themes, and rendering. Those atoms prevent duplicated
engines, but they are not a third public design domain.

The two domains have separate public preludes, primitive packages, component packages, themes, and
documentation:

```rust,ignore
use telorgon::application::prelude::*;
// or
use telorgon::shell::prelude::*;
```

An ordinary application does not compile shell surfaces, output composition, system panels, or
external synchronization. A shell does not automatically compile editor docking, property grids,
game HUDs, or application-navigation components. An advanced host may deliberately enable both.

Domain primitives are the smallest stable public building blocks for their design vocabulary. They
have layout, interaction, semantics, and rendering behavior, but do not contain protocol or
application business policy.

### 6.1 Shared foundation atoms

These concepts support both public domains and are exposed through the selected domain API rather
than requiring most users to import a third primitive catalog.

#### Structure and layout

| Foundation mechanism | Responsibility |
| --- | --- |
| `fragment` | Groups children without adding visual geometry |
| `spacer` | Consumes configurable free space without adding visual content |
| `box` | Box model, background, border, radius, padding, clip, and one child group |
| `row` / `column` | One-dimensional flex layout with gap, alignment, and distribution |
| `flex` | General flex container when row/column convenience is insufficient |
| `grid` | Explicit and intrinsic two-dimensional tracks |
| `stack` | Overlapping children with alignment and controlled paint order |
| `positioned` | Anchors a child within a stack or containing block |
| `aspect_ratio` | Constrains one dimension from another |
| `scroll_view` | Clipped scroll transform, wheel/touch physics, and scroll semantics |
| `virtual_list` / `virtual_grid` | Materializes only visible and cached collection ranges |
| `safe_area` | Applies platform display cutout and system-inset constraints |
| `splitter` | Low-level resizable-region geometry used by domain `SplitView` components |

Layout values support fixed logical pixels, content/intrinsic size, proportional fill, percentages,
minimum and maximum constraints, and device-independent density scaling. Layout never reads renderer
state.

#### Visual content

| Foundation mechanism | Responsibility |
| --- | --- |
| `text` | Shaped single- or multi-line text with selectable semantic content |
| `rich_text` | Styled spans, inline content, selection, and links |
| `editable_text` | Editing buffer, caret, selection, IME, and text-input semantics |
| `image` | Raster image with fit, sampling, tint, and content description |
| `icon` | Theme-sized symbolic image or vector glyph |
| `path` | Filled or stroked vector path for bounded custom graphics |
| `canvas` | Explicit custom drawing isolated behind a cache and damage boundary |
| `material` | Named compiled visual effect or multi-pass material |
| `external_content` | Backend-neutral slot mechanics wrapped by domain render/video/client-surface primitives |
| `separator` | Efficient semantic dividing line |

Common controls should lower to analytic scene records where possible. A rounded button background
does not become an arbitrary vector path or offscreen layer unless its effects require one.

#### Interaction

| Foundation mechanism | Responsibility |
| --- | --- |
| `pointer_region` | Pointer enter, leave, move, button, wheel, capture, and cursor behavior |
| `gesture_region` | Tap, long press, drag, scale, rotate, and gesture-arena participation |
| `focusable` | Keyboard focus, focus traversal, and focus-visible state |
| `focus_scope` | Local traversal, restoration, and modal focus containment |
| `shortcut` | Key chord to typed action mapping |
| `action_region` | Semantic activation independent of mouse or touch source |
| `drop_target` | Typed drag-and-drop negotiation and delivery |
| `text_input` | Platform IME and virtual-keyboard connection for an editor client |

All pointing devices enter the runtime as pointer events with stable pointer identities and a device
kind: mouse, touch, pen, eraser, or unknown. Gesture recognition consumes pointer streams but cannot
erase the underlying pointer lifecycle. Cancellation is first-class.

#### Semantics and accessibility

Every interactive component produces semantics independently of its paint implementation. The
semantics mechanism supports role, label, value, state, actions, relationships, live regions,
collection position, text ranges, and bounds derived from canonical layout.

Platform adapters export retained semantic deltas to UI Automation, AT-SPI, Android Accessibility,
and Apple accessibility APIs. Accessibility is not implemented by reverse-engineering the draw
list.

#### Overlays and layers

`overlay`, `portal`, `popup_anchor`, and `modal_scope` support menus, tooltips, dialogs, drag
previews, notifications, and shell overlays. Logical component/event ownership remains stable even
when visual content is presented in a separate layer.

Layers are created only for a reason: independent compositing, effects, caching, clipping, native
surface integration, or overlay ordering. A layout container does not automatically allocate an
offscreen target.

### 6.2 Application primitive domain

The application domain covers conventional desktop/mobile UI and real-time or game-development UI.
Its irreducible domain primitives are deliberately narrow:

| Primitive family | Includes |
| --- | --- |
| Application regions | `application_root`, `content_region`, `navigation_region`, and `status_region` |
| Game/viewport coordinates | `hud_layer`, `viewport_overlay`, and `world_anchor` |
| Host content | `render_target_view` and `video_surface` |

Menus, tabs, collections, fields, forms, docks, editor/game controls, overlays, and scaffolds are
standard components/facilities in section 7, not low-level primitives. Image, path, canvas, text,
layout, semantics, focus, and scrolling mechanics remain shared foundations.

A `render_target_view` displays a host-owned game/editor target without forcing Telorgon to own the
game renderer. A `hud_layer` makes coordinate spaces and hit behavior convenient; it does not
introduce an always-running immediate-mode UI path.

The application domain can adapt between mouse/keyboard, touch, controller, and mixed input. It does
not import compositor client-surface or operating-system policy concepts.

### 6.3 Shell primitive domain

The shell domain provides a narrow compositor-scene/chrome-geometry vocabulary:

| Primitive family | Includes |
| --- | --- |
| Outputs and layers | `shell_root`, `output_view`, and authorized `shell_layer` values |
| Client content | `client_surface`, `surface_tree`, `surface_placeholder`, and `surface_snapshot` |
| Shell input geometry | `reserved_area`, `exclusive_region`, `surface_input_region`, `drag_region`, `resize_region`, and `output_edge` |

Window frames/titlebars, workspace facilities, panels/taskbars, launchers/status widgets,
notifications, system dialogs, and secure overlays are shell components in section 9. Shell
primitives describe presentation and interaction, not protocol messages. For example,
`client_surface` consumes a stable host-provided surface model; it does not know whether that model
came from Wayland, a test compositor, a console shell service, or another protocol provider.

Shell primitives are suitable for both a complete desktop environment and smaller system-owned
surfaces such as login screens, launchers, virtual-keyboard shells, television interfaces, or
in-device dashboards.

### 6.4 Domain boundary rules

- Application primitives may depend on shared UI/runtime packages but not shell packages.
- Shell primitives may depend on shared UI/runtime and protocol-neutral external-surface contracts,
  but not application tool/editor components.
- Shared foundation packages contain no taskbar, window-policy, HUD, editor, or application
  navigation concepts.
- Application and shell theme identifiers use separate namespaces and scopes.
- Accessibility roles are shared when their meaning is universal; shell-only semantic actions live
  in the shell domain.
- Cross-domain composition is explicit. A shell can host a Telorgon application surface without
  mounting the application's component tree inside the shell, and a system settings application can
  use the application domain without depending on shell UI.
- Domain packages have independent compile, documentation, gallery, screenshot, accessibility, and
  performance tests.

## 7. Application components

Components are styled, accessible behavior assembled from primitives. The standard library should
include:

- button, icon button, link, toggle button, checkbox, radio, and switch;
- slider, range slider, progress indicator, and activity indicator;
- text field, search field, text area, editable document surface, and validation messages;
- list, table, tree, grid, tabs, breadcrumb, and pagination components;
- menu bar, menu, context menu, command palette, toolbar, and status bar;
- dialog, sheet, popover, tooltip, toast, and notification host;
- navigation rail, navigation bar, sidebar, split view, and adaptive scaffold;
- application scaffold, toolbar regions, content regions, and responsive window structure;
- date, time, and selection controls where platform-independent behavior is practical; and
- game/editor controls such as dock panels, inspectors, viewport overlays, property grids, and
  performance graphs.

Components expose typed values and actions rather than string event names. Interaction states are
shared state bits with theme resolution, not separate widget implementations.

Desktop and touch variants share semantics and state but may change density, minimum target size,
hover affordances, navigation pattern, and gesture behavior. Adaptive behavior follows input
capabilities and available space rather than assuming a platform from the executable name.

### 7.1 Application composition

Common application structure should not require manual stacks, portals, or window-service wiring.
An adaptive scaffold provides named regions while remaining ordinary component composition:

```rust,ignore
ui.application_scaffold()
    .navigation(ui.navigation_rail(routes, selected_route))
    .toolbar(ui.toolbar((
        ui.icon_button(icons::MENU, |_| Action::ToggleNavigation),
        ui.title("Documents"),
        ui.spacer(),
        ui.search_field(search_query),
    )))
    .content(ui.split_view(
        ui.virtual_list(documents, document_row),
        ui.document_editor(active_document),
    ))
    .overlay(ui.notification_host(notifications))
```

On a compact touch display the same scaffold may replace the rail with a navigation bar, collapse
the split view into a route, enlarge interaction targets, and request the virtual keyboard for the
editor. Those changes preserve component state and semantics rather than mounting a separate
application implementation.

## 8. Styling, themes, and design workflow

The theme system separates design-time documents from runtime tables:

- design tokens describe color, typography, spacing, radius, elevation, motion, density, and icon
  metrics;
- component styles map tokens and interaction states to primitive properties;
- compilation resolves names and inheritance into immutable numeric tables;
- runtime selection uses numeric identifiers and compact state masks; and
- theme scopes allow embedded content, previews, and shell regions to use isolated themes.

The Rust authoring API supports fluent local styling for application-specific composition. Repeated
or branded styles belong in compiled themes rather than cloned per-node style maps.

Application and shell themes are separate compiled domains. They may share low-level token value
types and an organization's brand palette, but they have independent component identifiers,
density policies, state tables, defaults, and theme scopes. A shell theme change cannot mutate an
embedded application's styles, and an application theme cannot restyle trusted system chrome.

Theme packages may deliberately derive both domains from one source design system at build time.
The resulting runtime tables remain separate and are loaded only when their domain is selected.

Theme Studio provides application and shell workspaces. The galleries are split into application
and shell catalogs so developers do not mistake domain-specific behavior for a universal control.
Both catalogs operate on the same component runtime, layout, text, and renderer paths as production
consumers. A preview backend or modeled renderer cannot qualify the production renderer.

## 9. Desktop-shell facilities

Shell support is a sibling component library and host integration layer above the shared
UI/runtime foundations. It does not depend on or specialize the application component domain.
It should make the common visual structure of a shell concise without owning window-management
policy.

### 9.1 Shell components

- `shell_root` establishes background, workspace, panel, overlay, and system-modal layers;
- `workspace_view` lays out host-provided surfaces according to policy-owned geometry;
- `client_surface` presents an external surface with transform, clip, opacity, damage, and sync;
- `panel` and `dock` provide edge placement, reservation, auto-hide, and adaptive layout hooks;
- `launcher` and `command_palette` present searchable host-provided actions;
- `status_area` hosts clock, connectivity, power, media, and extension-provided indicators;
- `notification_center` and `notification_host` present system notifications;
- `window_chrome` provides shared titlebar and resize affordances for decorated surfaces; and
- `shell_overlay` provides lock, task switching, overview, volume, brightness, and similar modes.

### 9.2 Shell host contract

The shell receives typed models and commands from a platform-specific policy host:

```rust,ignore
trait ShellHost {
    fn surfaces(&self) -> Read<SurfaceCollection>;
    fn workspaces(&self) -> Read<WorkspaceCollection>;
    fn applications(&self) -> Read<ApplicationCollection>;
    fn notifications(&self) -> Read<NotificationCollection>;
    fn system_status(&self) -> Read<SystemStatus>;
    fn request_client_input(&mut self, request: ClientInputRequest) -> RequestResult;
    fn request_surface(&mut self, request: SurfaceRequest) -> RequestResult;
    fn request_workspace(&mut self, request: WorkspaceRequest) -> RequestResult;
    fn request_output(&mut self, request: OutputRequest) -> RequestResult;
    fn request_system(&mut self, request: SystemRequest) -> RequestResult;
}
```

Telorgon renders and interacts with these models. Requests are divided by authority and the host
validates identity, input source, capability, lock/session state, and policy before returning an
accepted/denied/stale/unsupported result. The host decides whether a surface/workspace/system
request maps to Wayland, a mobile system service, a test fixture, or another environment. Visual
truth changes through the next host snapshot rather than optimistic component mutation.

An illustrative shell declaration should remain ordinary UI composition:

```rust,ignore
ui.shell_root(
    ui.workspace_view(host.surfaces()),
    ui.panel(
        Edge::Top,
        ui.row((
            ui.launcher(host.applications()),
            ui.workspace_switcher(host.workspaces()),
            ui.spacer(),
            ui.status_area(host.system_status()),
        )),
    ),
    ui.notification_host(host.notifications()),
)
```

### 9.3 Compositor integration boundary

A compositor host supplies an output target and a stream of stable external-surface records. Each
record contains only the information needed for composition:

- stable surface identity and parent/subsurface relationship;
- logical bounds, buffer size, scale, transform, opacity, and z-order;
- opaque, input, clip, and damage regions;
- content version and color-space/HDR metadata;
- an opaque external-image import token understood by the selected renderer backend; and
- acquire and release synchronization understood by the platform/backend interop package.

Telorgon converts these records into retained `client_surface` scene instances, combines them with shell
UI, records the output render work, and returns completion synchronization to the host. Client
buffers are not read back to the CPU in the normal path.

The protocol provider remains responsible for interpreting Wayland or another protocol, validating
client state, importing protocol buffer types into backend-compatible handles, choosing window and
focus policy, and delivering shell commands. That provider may be external or Telorgon's Linux-only
`telorgon-compositor-wayland` assembly. No Wayland object, event, or naming convention belongs in
`telorgon-shell`, `telorgon-scene`, or the general component API.

This boundary lets the portable Telorgon layers remain reusable even though the optional Linux
shell-environment assembly is itself a display-server protocol implementation.

## 10. Platform architecture

The implementation and qualification sequence is fixed in
[PLATFORM_IMPLEMENTATION_ORDER.md](PLATFORM_IMPLEMENTATION_ORDER.md): Windows Vulkan, hosted Vulkan,
separate Linux Wayland/X11 Vulkan profiles, protocol-neutral Linux shell integration, direct
Metal/macOS, shared mobile foundations, Android Vulkan, then iOS Metal.

The exact lifecycle axes, scheduling, input translation, native IME/accessibility, capability-
checked services, data transfer, native handle, managed/embedded, and protocol-host boundaries are
fixed by [PLATFORM_INTEGRATION_CONTRACT.md](PLATFORM_INTEGRATION_CONTRACT.md). In particular,
activity, visibility, native-surface availability, and presenter state are independent; platform
callbacks enqueue events and never re-enter component code.

The platform boundary is split into services rather than one all-powerful backend:

- event-loop and application lifecycle;
- window or view creation and surface lifecycle;
- pointer, keyboard, touch, pen, and gamepad input where supported;
- clipboard, drag and drop, cursor, and system commands;
- IME, virtual keyboard, and text-editing protocol;
- accessibility bridge;
- monitors, density, orientation, safe areas, and appearance;
- timers, frame callbacks, and power-aware scheduling; and
- renderer surface creation and presentation.

Desktop hosts may expose multiple windows. Mobile hosts may expose one platform view plus lifecycle
events. Embedded hosts may provide no window API at all and supply a render target directly.

The core runtime cannot call a desktop event loop directly. Convenience hosts may combine the
contracts for ordinary applications.

## 11. Rendering architecture

The detailed portability and backend service-provider contract is specified in
[RENDER_BACKEND_ARCHITECTURE.md](RENDER_BACKEND_ARCHITECTURE.md).

The rendering architecture has four boundaries:

```text
Retained UI scene
       |
       v
Render planner and resource cache
       |
       v
Telorgon rendering hardware interface
       |
       v
Vulkan, Metal, Direct3D, console, or portability backend
       |
       v
Owned surface or host render graph
```

The retained scene describes UI meaning and painter order. The render planner selects concrete
passes, batches, caches, and fallbacks. The rendering hardware interface expresses explicit GPU
work. A backend maps that work to one graphics API and its host interop.

### 11.1 Vulkan reference backend

The planned Vulkan backend will be implemented first and will serve as the architectural reference.
To be classified as operational, it must use real Vulkan objects and operations for:

- instance, physical-device, logical-device, queue, and surface selection;
- swapchain creation, resizing, suspension, and loss;
- persistent device-local buffers and staging allocations;
- images, views, samplers, descriptors, pipelines, and pipeline caches;
- command pools, command buffers, barriers, semaphores, timelines, and fences;
- external memory and synchronization import where platform support permits;
- render and compute passes used by Telorgon materials and vector features;
- timestamp queries, debug labels, validation, and device diagnostics; and
- device loss reporting and retained-scene recovery.

It supports both owned and hosted modes. Owned mode creates and manages the device and presentation
surface for a normal application. Hosted mode uses a host-approved Vulkan device, queues, allocator,
frame context, formats, synchronization rules, and render target. Hosted mode must not call
`vkDeviceWaitIdle` during ordinary work or submit independently when the host requests command-only
integration.

### 11.2 Rendering hardware interface

`telorgon-rhi` is an internal, backend-implementer-facing contract for modern explicit graphics APIs.
It is not a public application drawing API and does not mirror Vulkan function for function.

Its concepts include:

- adapter and device capabilities;
- owned and host-borrowed device contexts;
- queues and frame contexts;
- buffers, textures, texture views, samplers, and memory classes;
- shader modules, bind layouts, pipelines, render passes, and compute passes;
- command encoders and explicit resource usage transitions;
- fences, timeline values, external synchronization, and deferred destruction;
- presentation surfaces and host-provided render targets; and
- debug markers, timestamps, memory statistics, and device-loss errors.

The RHI defines a small required baseline for normal UI and capability flags for optional features
such as descriptor indexing, compute paths, advanced blending, subgroups, HDR, external memory, and
timeline synchronization. Render planning selects a fallback rather than pretending unsupported
features exist.

The abstraction is not designed around the lowest common denominator. Metal or Direct3D backends
may combine or emulate concepts internally. Console backends may provide private extensions and
precompiled shader artifacts. Backend-specific native handles are available only through typed
interop interfaces in that backend package.

Vulkan SPIR-V is intended to be the first operational GPU shader artifact. The material/shader build
system owns the logical shader interface and can generate or consume backend-appropriate artifacts.
The core does not assume proprietary console shader compilers can be redistributed or invoked
outside their SDKs.

A portability library such as wgpu may be implemented as one backend adapter, but it is not the
definition of Telorgon's rendering abstraction.

The RHI remains internal and explicitly unstable while only Vulkan implements it. Vulkan provides
the concrete semantics; a second materially different backend tests which concepts are genuinely
portable before the RHI is stabilized for external backend implementers.

The initial implementation does not create a broad public RHI first. It uses the narrow
`telorgon-render` backend contract and direct Vulkan package defined by the implementation Blueprint;
`telorgon-rhi` is extracted only after Vulkan plus a trace or materially different backend prove the
shared boundary. The exact first scene/GPU/shader contract is
[Gate 4](SCENE_GPU_ABI_AND_SHADERS.md).

### 11.3 Backend-neutral scene

The retained scene contains stable instances for:

- analytic boxes and borders;
- glyphs and text decorations;
- images and atlas regions;
- vector paths;
- clips, masks, and spatial transforms;
- opacity and compositing layers;
- compiled materials and intermediate targets; and
- external or native surfaces.

Painter order is explicit. Scene storage uses typed generational slots whose indices remain stable
across ordinary removal; UI node identity stays private to the compiler. Updates produce
epoch-ordered range patches, resource changes, optional order replacement, and damage rather than
rebuilding or flattening all scene content each frame.

### 11.4 Scene renderer contract

A production render backend operates against an owned or host-supplied real graphics device and owns
Telorgon's persistent resources within that device. A separate presenter or embedding host owns target
acquisition, submission policy, and presentation. The backend contract includes:

- capability discovery and negotiated fallbacks;
- applying scene and resource deltas;
- importing external textures and synchronization objects;
- scheduling uploads into persistent staging storage;
- recording real GPU work into an owned or host-provided frame;
- returning recorded-work statistics and explicit completion/readback tokens; and
- explicit readback for tests, export, and diagnostics.

The owned presenter separately handles surface creation, resize, suspension, acquisition,
submission, presentation, and surface loss. Hosted command-only mode performs none of those actions
on Telorgon's initiative.

Backend selection is a build and host decision. The scene cannot expose backend handles except
through typed external-resource interop contracts. Applications that do not request native interop
remain source-independent from the selected backend.

### 11.5 Batching and mutability

Batching preserves painter order. Adjacent compatible instances form batches by pipeline, resource,
clip, blend, target, and material state. Static and frequently changing content must not be merged
into one large upload unit merely to reduce draw count.

The compiler identifies retained batch roots and mutability groups. Transform-only scrolling and
animation update root transforms when possible. Clips and effects explicitly report when they break
batching or require intermediate targets.

### 11.6 Software reference renderer

The software renderer defines deterministic scene semantics, supports headless tests, and can be an
explicitly selected fallback. It is not an internal fallback for a GPU backend and is not evidence
that a GPU backend works. It is feature-frozen during the current Vulkan implementation phase except
for correctness and test maintenance. Software and GPU outputs share conformance fixtures with
documented antialiasing tolerances.

## 12. Frame lifecycle

A scheduled frame performs only the required phases:

1. collect and coalesce platform input;
2. route pointer, gesture, keyboard, focus, and text-input events;
3. commit application transactions and evaluate dirty bindings;
4. reconcile explicitly changed structure;
5. measure and arrange dirty layout roots;
6. update spatial transforms, clips, visibility, and hit indices;
7. update semantic nodes and emit platform accessibility deltas;
8. shape changed text and update dirty atlas pages;
9. compile changed UI nodes into retained scene instances and damage;
10. transport scene/resource deltas to the renderer;
11. record and submit real backend work; and
12. present damage and schedule the next required deadline.

The UI runtime is single-writer. A renderer may operate on another thread using ordered immutable
deltas and bounded queues. Background application work communicates through typed tasks and
messages.

## 13. Performance and resource model

The project-wide acceptance layers, structural invariants, hardware timing profiles, device matrices,
and production reports are fixed in
[Acceptance and qualification](ACCEPTANCE_AND_QUALIFICATION.md). This section states the product
principles those gates enforce.

Portable performance gates measure work rather than universal frame time:

- nodes visited and properties evaluated;
- measured and arranged layout nodes;
- text runs shaped and atlas bytes uploaded;
- scene instances patched and bytes transported;
- actual GPU bytes uploaded, passes, batches, draws, and submissions;
- damage area and intermediate-target allocation;
- allocations after warm-up; and
- retained CPU and GPU memory high-water marks.

Hardware timing reports include device, driver, backend, surface format, scale, resolution, and
power mode. Modeled resource identifiers and CPU vectors cannot satisfy actual GPU resource or
timing gates.

### 13.1 Embedded-host interference gates

An embedded or engine integration is qualified separately from a standalone application. After
warm-up, an unchanged embedded UI must:

- require no update, scene compilation, upload, command recording, or submission;
- allocate no general heap memory during a needs-update query;
- create no threads and schedule no background work unless the host enabled it;
- perform no readback, device-wide idle, or implicit queue wait;
- submit no work independently when operating in command-only mode; and
- retain resources within a host-configurable CPU and GPU budget.

When the UI changes, diagnostics report the CPU time, allocations, uploaded bytes, render/compute
passes, barriers, draw/dispatch counts, and target damage introduced by Telorgon. The host can provide
frame allocators, pipeline caches, shader caches, and debug labels without transferring ownership of
the rest of its renderer.

Passing the offscreen renderer suite does not qualify embedding: hosted operation separately proves
the absence of hidden submission, waits, readback, threads, or target-state interference.

## 14. Target module boundaries

The module graph is part of the architecture. Telorgon is distributed as one public framework
package so users do not need to coordinate dozens of internal versions, but consolidation does not
authorize an all-purpose implementation file or erase subsystem ownership.

Rust requires the `#[component]` procedural attribute to be compiled by a dedicated proc-macro
crate. The workspace therefore has exactly two publishable packages:

- `telorgon`, containing the complete framework and its named subsystem modules; and
- `telorgon-macros`, an exact-version implementation companion re-exported by `telorgon`.

The offline `telorgon-shader-build` package remains repository-only with `publish = false`. It is
not linked into downstream applications. No other Telorgon runtime package is published.

| Module | Responsibility |
| --- | --- |
| `core` | Geometry, identifiers, time, color, capability-neutral values, and shared errors |
| `runtime` | Components, state, bindings, transactions, typed actions, and tasks |
| `compose` | Declarative component builders, fields, signals, events, and elements |
| `ui` | Mounted nodes, properties, structure, foundation atoms, overlays, and semantics |
| `layout` | Incremental measure/arrange, spatial state, clips, hit indices, scrolling, and virtualization |
| `input` | Unified pointers, keyboard, gestures, focus, shortcuts, drag and drop, and routing |
| `text` | Shaping, fallback, editing, selection, IME client state, glyph cache, and atlas data |
| `accessibility` | Semantic-tree processing and platform-neutral accessibility deltas |
| `theme` | Compiled tokens/styles, scopes, state resolution, archives, and motion values |
| `application_primitives` | Public application, tool, game, and embedded-product primitive domain |
| `shell_primitives` | Public shell, chrome, output, workspace, panel, and system primitive domain |
| `application_components` | Accessible standard controls and adaptive application/game components |
| `shell_components` | System widgets and composed shell facilities built from shell primitives |
| `scene` | Retained scene storage, hierarchy, dirty flags, and sparse data |
| `gpu_abi` | Exact versioned POD transfer records and shader-visible layout constants |
| `material` | Effect definitions, logical passes, intermediates, and fallback descriptions |
| `render` | Scene-to-render planning, batching, caches, uploads, and backend lifecycle contract |
| `renderer_software` | Deterministic CPU reference renderer and headless output |
| `renderer_vulkan` | Vulkan backend for owned, hosted, external-image, and readback paths |
| `presentation` | Neutral surface, acquire, present, completion, recovery, and lifecycle contracts |
| `presenter_*` | Native WSI, DXGI, Softbuffer, or KMS presentation adapters |
| `bridge_vulkan_dxgi` | Windows cross-API Vulkan-to-DXGI transport and synchronization |
| `platform` | Platform-neutral view/lifecycle snapshots, scheduling, capabilities, and services |
| `platform_conformance` | Deterministic neutral platform fixtures and fake services |
| `platform_*` | Winit or native Linux adapters with no renderer ownership |
| `shell` | Protocol-neutral surface/output/workspace models, authority, and host transport |
| `wayland_server` | Linux Wayland protocol parsing and native server transport |
| `compositor_wayland` | Wayland compositor state, protocol dispatch, and surface/input ownership |
| `compositor_render` | Compositor SHM and DMA-BUF rendering bridge |
| `profiler` | Compile-optional bounded event production and explicit session collection |
| `profiler_server` | Managed-only bounded loopback profiler service and embedded viewer |
| `embed` | Host-driven multi-view rendering and render-area integration conveniences |
| `application_host` | Managed application convenience host and selected backend assembly |
| `app` | Curated ordinary-authoring facade; it re-exports rather than reimplements |
| crate root | Stable top-level values and documented compatibility re-exports |

Dependencies point downward and remain conceptually acyclic. The two primitive domains are siblings
and cannot depend on one another. Shell components may depend on shell primitives and
protocol-neutral external-surface contracts. Application components may depend only on application
primitives and shared foundation modules. Platform adapters depend on platform traits. Core runtime,
UI, layout, and scene modules cannot depend on a design domain, shell policy, managed host, concrete
presenter, or concrete graphics backend.

`application_host` and `embed` are sibling conveniences over the same runtime and renderer
contracts. The managed module cannot sit underneath the embedded module because embedding must not
pull in window or event-loop ownership. The facade exposes both under distinct namespaces.

### 14.1 Build and feature profiles

The single package provides curated profiles rather than compiling every platform and tool:

- `application-software` selects application authoring, Winit hosting, the software renderer, and
  Softbuffer presentation;
- `application-vulkan-windows` selects application authoring, Winit hosting, Vulkan WSI, and the
  target-gated Windows Vulkan/DXGI path;
- `embedded-vulkan` selects host-driven runtime and Vulkan hosted mode without a window loop;
- `shell-wayland-linux` selects the target-gated shell/compositor, Linux platform, Vulkan,
  software-reference, and KMS modules;
- `profiler` selects shared instrumentation and the managed profiler service; and
- `embedded-profiler` selects shared instrumentation without forcing the managed service.

The internal `instrumentation` feature is plumbing shared by the two public profiler profiles.
Adding a backend must not add its SDK or dependencies to builds that did not select it.

The package exports the application and shell domains under distinct modules and keeps the compact
ordinary application facade:

```rust,ignore
use telorgon::app::*;
use telorgon::application_components::prelude::*;
use telorgon::shell_components::prelude::*;
```

Enabling one platform profile does not silently enable unrelated platform adapters or launch any
host. Feature selection only controls compilation.

### 14.2 Source-file structure

Module boundaries are reinforced inside the package:

- `lib.rs` declares modules, curated exports, and facade policy; it is not the primary
  implementation file;
- every former crate root is now a declaration-oriented `mod.rs` under its named module directory;
- each file owns one cohesive concept or tightly coupled type family;
- public traits and their backend/platform implementations live in separate files;
- platform-specific code lives in target- and feature-gated modules;
- GPU device setup, resources, pipelines, frames, external interop, presentation, and diagnostics
  remain distinct renderer files;
- layout algorithms, input routing, component definitions, and rasterization do not share a file;
- unit tests remain beside implementations, while integration fixtures live under
  `crates/telorgon/tests` with module-qualified filenames; and
- generated bindings, shaders, theme tables, profiler assets, and platform glue remain visibly
  separated from authored runtime code.

There is no universal line-count limit. A file is split when it owns multiple lifecycle domains,
requires unrelated dependency groups, cannot be described by one responsibility, or repeatedly
causes unrelated changes to collide.

### 14.3 Boundary enforcement

Consolidation replaces Cargo package-edge enforcement with explicit module and build checks:

- reject imports from foundation modules into managed hosts, platform adapters, or concrete
  renderers;
- report package features and unexpected optional dependencies;
- keep the no-default-features core profile buildable without Winit, Softbuffer, Vulkan, platform
  SDKs, profiler transport, or shader compiler dependencies;
- compile each supported feature profile independently;
- prohibit concrete backend types in scene and ordinary application public APIs;
- run renderer conformance against software and every operational backend available to the selected
  test environment;
- keep hardware-presenting tests opt-in; and
- require an architecture note when a change introduces a new cross-layer dependency.

## 15. Delivery stages

The architecture should be delivered in evidence-producing stages. Delivery order reflects
technical dependencies and does not change the ordered product goals:

1. **Scope and usability prototypes:** validate the application, component, embedded, and shell APIs
   in small examples before treating their names and ownership model as stable.
2. **Modular runtime foundation:** establish package boundaries, reactive state, keyed structure,
   canonical layout, input routing, semantics storage, the existing software conformance reference,
   and status reporting. This stage does not require further CPU-rasterizer feature expansion.
3. **Operational Vulkan and embedding:** implement real Vulkan offscreen and Windows-owned mode,
   hosted render areas, then separate Linux Wayland/X11 profiles, resource reuse, synchronization,
   GPU timing, recovery, and conformance. This is the current renderer priority; it does not wait for
   new CPU-rasterizer feature work.
4. **Desktop application toolkit:** standard accessible controls, editing and IME, adaptive layout,
   desktop services, gallery, theming workflow, and application usability qualification.
5. **Shell and compositor integration:** external-image import and synchronization, output targets,
   shell host contracts, panels, workspaces, overlays, chrome, and protocol-host test fixtures.
6. **Direct Metal and macOS:** implement the materially different direct Metal backend, hosted Metal,
   and managed macOS arm64 before extracting a broad RHI.
7. **Touch and mobile:** implement shared multi-pointer gestures, virtual keyboard, safe areas,
   lifecycle, accessibility bridges, mobile platform hosts, and device qualification.
   Android arm64 Vulkan is first; iOS arm64 Metal is second.
8. **Additional graphics backends:** use D3D12 or another API to refine the proven boundary, then
   support private console adapters without changing UI and scene contracts.

Each stage must label subsystems as planned, modeled, operational, or production-qualified. Public
documentation must not collapse those states into one claim.

## 16. Architectural decision summary

Telorgon is one general-purpose GUI toolkit with multiple hosts, not separate application and shell
runtimes. It uses one mounted component model, one canonical layout, one semantics model, one
backend-neutral retained scene, and one renderer contract. Desktop, mobile, embedded, and shell
integrations supply platform capabilities around that core. Vulkan is intended to become the first
operational GPU backend and the reference used to shape an internal RHI that other explicit APIs can
implement.

The authoring API is declarative and compact. The runtime representation is persistent and
data-oriented. Managed applications are convenient; embedded hosts retain ownership; shell hosts
provide protocols and policy; renderer backends remain replaceable. Convenience must not require
rebuilding every frame, modularity must remain visible in packages and files, and performance must
not make ordinary application development difficult.

## 17. Implementation-validation decisions

Gates 3–9 resolved the high-level ownership, RHI, threading, compositor, package, multi-view,
component, primitive, and platform questions that were previously listed here. There is no further
undefined architecture gate. The following deliberately bounded details still require compile or
hardware prototypes before their public Rust spelling is stabilized:

1. **Styling syntax:** the exact relationship between fluent Rust styling, compiled themes, design
   tokens, responsive variants, and live design tooling, without changing Gate 7/8 ownership.
2. **Borrowed backend spelling:** the exact safe Rust types for hosted device/frame/encoder/target
   borrowing and completion tokens while preserving Gate 3 and Gate 9 non-ownership rules.
3. **RHI stabilization:** capability and extension details refined by real Vulkan and direct Metal
   implementations before promising a public vendor-backend API.
4. **Optional render-worker policy:** the queue and cancellation implementation for an explicitly
   selected render worker; platform callbacks still cannot re-enter the single-writer UI runtime or
   spawn hidden threads.
5. **Per-target view variants:** thresholds and cache policy for recording one logical view at
   multiple scales/targets while preserving independent metrics revisions and bounded retained data.

Each item is an implementation work package with an architecture note, tests, and acceptance
evidence. A prototype may refine signatures, but changing the invariants fixed by Gates 3–9 requires
amending the controlling contract first.

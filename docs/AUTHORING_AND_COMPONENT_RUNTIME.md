# Telorgon Composition and Component Runtime

## Current status

This document describes the programming interface implemented by `telorgon-compose`,
`telorgon-macros`, and `telorgon-runtime::CompositionDriver`. It supersedes the former public
mount-once authoring proposal. The retained foundation runtime remains available to framework and
advanced component authors, but ordinary application composition uses the interface below.

## Minimal application

```rust
use telorgon::app::*;

#[component]
struct Counter {
    #[input]
    title: String,
    #[state]
    count: u32,
}

impl Component for Counter {
    fn view(&self) -> impl View {
        column()
            .padding(24.0)
            .gap(16.0)
            .child(text(&self.title).size(24.0))
            .child(text(format!("Count: {}", self.count)))
            .child(
                button("Increment")
                    .on_press(|this: &mut Self| this.count += 1),
            )
    }
}

fn main() -> Result<()> {
    Application::gui("Counter")
        .window(
            Window::new("Counter")
                .size(480, 320)
                .content(Counter {
                    title: "Counter".into(),
                    count: 0,
                }),
        )
        .run()
}
```

`run` exists only on a complete GUI declaration; a component or incomplete builder cannot
accidentally be used as a process root. The application owns its initial window and the window
owns its component composition.

## The three authoring values

Telorgon follows one rule:

```text
builders describe things
component structs represent things that persist
signals represent values owned elsewhere
```

Builder values and `Element` are short-lived descriptions. Component objects and mounted UI nodes
are persistent and generational. A view evaluation does not recreate a retained button, reset its
focus, or share its interaction state with a sibling when identity and type still match.

Pure helpers can remain functions:

```rust,ignore
fn heading(value: &str) -> impl View {
    text(value).size(24.0).weight(650)
}
```

Use a component struct when local state, lifecycle, signal dependencies, or stable child identity
is needed.

## Inputs and state

Every named component field is explicitly classified:

- `#[input]` is parent-owned configuration. The default mode requires `Clone + PartialEq` and
  rerenders the retained child only when the incoming value changes.
- `#[input(always)]` accepts a value that should always notify the child.
- `#[input(compare_with = path)]` uses a named comparison function.
- `#[state]` remains owned by the retained component instance. An incoming child description can
  never overwrite it.

Event callbacks receive mutable access to the persistent component. Input fields are snapshotted
before a callback and restored afterward, so child code cannot acquire authority over parent-owned
inputs by mutating the struct. Diagnostics count rejected input mutations.

The `#[component]` macro generates the field-classification plumbing and the default implementation
described below. Lifecycle and view behavior remain ordinary trait methods. Direct users normally
write `#[component]`; internal crates that depend on `telorgon-compose` without the umbrella crate may use
`#[component(crate_path = telorgon_compose)]`.

Components derive `Default` automatically, so a component whose fields implement `Default` can be
constructed with `Counter::default()` without an additional derive. Components with custom
initialization or fields that do not implement `Default` opt out with `#[component(no_default)]`
and provide a constructor or manual `Default` implementation. Internal crates can combine the
options as `#[component(crate_path = telorgon_compose, no_default)]`.

## View evaluation and reconciliation

`Component::view` runs:

- for the initial mount;
- after a successfully delivered handler mutates its owner;
- when parent inputs for that retained child change;
- after a watched signal publishes a new revision; and
- after lifecycle code explicitly requests an update.

It does not run once per display frame. Rendering frames with no dirty component performs no view
evaluation.

The runtime validates each component's complete returned element description before reconciling it.
Containers match unkeyed children by local position and type. Keyed children match by key and type,
may move without remounting, and retain component state and control identity. Duplicate sibling
keys and callbacks typed for a different component are errors. Replaced or removed generations
close their handlers, signal subscriptions, lifecycle, and mounted nodes; stale events are rejected.

Common builders erase each child as it is appended, so user-facing container types do not grow a
large nested generic signature. Implemented primitives include `row`, `column`, `stack`, `text`,
`button`, `checkbox`, `switch`, `slider`, `spacer`, and `card`. `Dimension` and `Insets` allow
compact calls such as `.width(320.0)`, `.width(Dimension::FILL)`, `.padding(12.0)`, and
`.padding((6.0, 12.0))`.

Composition containers (`row`, `column`, `stack`, and container-based conveniences such as
`card`) fill their available width and height by default. Opt into content-sized behavior with
`.width(Dimension::Shrink)`, `.height(Dimension::Shrink)`, or both. Passing `.style(BoxStyle)`
replaces the complete style, including these size rules.

Containers accept a complete `BoxStyle` through `.style(...)` and expose concise field setters such
as `.background(...)`, `.padding(...)`, and `.corner_radius(...)`. Text follows the same model with a
sparse, reusable `TextStyle`:

```rust,ignore
let heading = TextStyle::new()
    .size(24.0)
    .weight(600)
    .color(ColorRgba8::rgba(240, 242, 247, 255));

column()
    .child(text("First").style(heading))
    .child(text("Second").style(heading).size(20.0));
```

Unspecified text fields inherit the current defaults. A fluent setter after `.style(...)` overrides
only that field; an unspecified line height remains automatic at `1.25 × size`. The runtime resolves
the sparse authoring style into retained text metrics before mounting or patching the text node.

## Events

Simple handlers pass an annotated closure directly to the control:

```rust,ignore
button("Open").on_press(|this: &mut Self| this.open = true)
```

Function items use the same interface: `button("Open").on_press(Self::open)`. A closure may capture
owned values with `move`. Callback descriptions remain owner-independent until the component's view
is validated; mount or reconciliation then binds them to that component's generational
`ComponentInstanceId`.

Common value controls pass their typed value directly:

```rust,ignore
slider("Volume", self.volume)
    .on_change(|this: &mut Self, value| this.volume = value)

checkbox("Enabled", self.enabled)
    .on_change(|this: &mut Self, checked| this.enabled = checked)
```

`on_change_event` exposes the complete normalized `EventContext` when source or change-phase
metadata is needed.

Handlers are bound to a generational `ComponentInstanceId`. The interaction router resolves child
label/icon hits to the registered control root and owns hover, press, capture, focus-visible,
keyboard activation, drag, and cancellation independently per control.

## External values

`Signal<T>` is for data whose owner is outside the component:

```rust,ignore
let tasks = self.watch(&self.tasks);
```

Telorgon evaluates `view` inside a scoped runtime frame. `watch` records the signal revision against
that evaluating component without storing runtime identity inside the component struct.
Publications coalesce by component, wake the managed event loop, rerender only current subscribers,
and safely discard stale generation notifications. `publish_if_changed` produces no invalidation
for an equal value. Contextual component methods reject calls outside view evaluation.

Local component state should remain a normal `#[state]` field; signals are not a replacement for
ordinary ownership.

## Lifecycle

Components may implement `mounted`, `inputs_changed`, and `unmounted`. A lifecycle callback that
changes state calls `cx.request_update()`. Mount remains non-reentrant: requested work is coalesced
and reconciled in the following runtime turn. Unmount runs before the component generation is
retired.

## Entry roots and capabilities

`Application` has exactly two entrypoint constructors:

- `Application::gui(name)` builds an ordinary managed GUI application and requires one `Window`
  with content before `.run()` is available.
- `Application::shell_environment(name)` builds a Linux shell environment with a cursor-configured
  compositor. Components implementing `ShellWidget` are registered directly through `.widget(...)`;
  zero widgets is valid. Wallpaper is a widget, not compositor configuration.

Both modes select rendering on the application builder. `Window::content(...)` completes GUI
content. A compositor is ready after cursor configuration and has no background/content component.
Shell widgets define reactive `surface()` descriptors beside their ordinary `view()` methods.
See [Shell widgets](SHELL_WIDGETS.md) for placement, animation, child surfaces, services, migration,
and current qualification limits.

`self.runtime_target()` reports `Application`, `ShellWidget`, or `Compositor` according to the
composition's role. The GUI native host is operational. With the Linux-only
`shell-wayland-linux` feature, the shell-environment declaration, validation, renderer
selection, background/frame composition, shell widgets, and bare-metal Wayland/KMS runtime are
operational but not production-qualified. Other targets return a clear unsupported-host error; the
declaration never falls back to an ordinary application window because doing so would falsely
grant shell/compositor capabilities.

## Theme and renderer path

Composition primitives mount retained foundation nodes and named component slot bindings. Button
labels and checkbox/switch/slider indicators, tracks, fills, thumbs, marks, and labels participate
in their component style contracts. Interaction flags resolve through Theme v4, sampled motion,
atomic property patches, layout/spatial updates, and scene compilation. Renderers consume only the
resolved sampled scene and never infer hover, focus, disabled, or sibling state.

## Advanced retained API

The previous mount/action runtime remains in lower-level crates for existing first-party component
catalog coverage and custom foundation work. It is exported as `MountedComponent` rather than
`Component`; ordinary application code should start with `use telorgon::app::*` and the composition
API. The UI Gallery and Theme Studio both use the new composition runtime and serve as first-party
examples.

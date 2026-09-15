# Telorgon component UI

## Current authoring model

Telorgon applications use one root `Component`. Configuration remains immutable, runtime-owned
`State<T>` values belong to one component instance, and typed actions commit updates atomically.
Foundation nodes are written through `telorgon-runtime::Ui` and its short-lived
`telorgon-ui::MountWriter`; application controls come from `telorgon-components-application`.

```rust,ignore
use telorgon::{
    Button, Component, CreateContext, State, Ui, UpdateContext,
    ui::{BoxStyle, LayoutStyle, UiRoot},
};

struct Editor;
struct EditorState {
    saves: State<u32>,
}
enum EditorAction {
    Save,
}

impl Component for Editor {
    type State = EditorState;
    type Action = EditorAction;

    fn create(&self, cx: &mut CreateContext<'_>) -> Self::State {
        EditorState { saves: cx.state(0) }
    }

    fn mount(&self, _state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
        let root = ui
            .foundation()
            .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
        Button::new("Save")
            .unwrap()
            .mount(ui, root.0, |_| EditorAction::Save)
            .unwrap();
        root
    }

    fn action(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        cx: &mut UpdateContext<'_, Self>,
    ) {
        match action {
            EditorAction::Save => {
                let count = cx.get(state.saves).unwrap();
                cx.set(state.saves, count + 1).unwrap();
            }
        }
    }
}
```

`mount` runs once per component generation. Ordinary input does not remount a component. Direct
state reads, derived `Read<T>` graphs, property bindings, observers, timers, and tasks all retain
the creating component as owner. Stale component, state, action-route, timer, and task generations
are rejected.

## Structure and input

`Ui::when`, `Ui::for_each_keyed`, `Ui::switch`, and `Ui::portal` own child component lifecycles.
Structural changes validate before commit, preserve stable keyed identity, and unmount replaced
children child-first. Child actions stay local unless a structural boundary explicitly maps,
consumes, or converts them to a runtime command.

Neutral input preserves capture, target, and bubble phases. Application components layer typed
behavior and semantic contracts over that neutral path. Parent-controlled controls such as
checkboxes, switches, sliders, progress indicators, and activity indicators consume `Read<T>` and
emit typed proposals instead of mutating caller state implicitly.

Mounted switches and checkboxes use toggle nodes, keep their checked/mixed semantics synchronized,
and patch their track, thumb, indicator, and mark visuals from the live controlled read. Sliders use
slider nodes, route pointer positions through their mounted track geometry, emit begin/update/commit
phases, and patch fill/thumb geometry without remounting. Progress and activity indicators remain
noninteractive while their fill, marker, numeric value, and busy semantics follow live state.
Focused buttons and toggles activate with Enter or Space, and Tab/Shift+Tab use the runtime focus
order.

## Current catalog

The application catalog includes actions, choices, range controls, text and editing controls,
collections, navigation, structure, scrolling, command surfaces, and overlays. Shell-specific
chrome, workspaces, panels, launchers, status, notifications, and secure surfaces live in the
sibling `telorgon-components-shell` package.

The component catalog is exercised directly by package-level behavior and compile-path tests. Telorgon
does not bundle a gallery application; consumers can build project-specific catalogs from the public
application components without pulling an example executable into the runtime workspace.

## Presentation

Both application modes select presentation in Rust rather than in the consuming project's Cargo
features. `Application::gui(...)` and `Application::shell_environment(...)` default to
`Renderer::Auto`; an operational host tries the direct Vulkan presenter first and falls back to the
software/Softbuffer presenter if Vulkan cannot initialize before component mounting.
`Renderer::Vulkan` and `Renderer::Software` request an exact backend. The GUI host is the currently
operational native entrypoint. The Linux shell-environment declaration retains the selected
policy for its future bare-metal host; its compositor and shell widgets do not select renderers
independently. `HeadlessRuntime` continues to use the software backend for deterministic tests.

See [AUTHORING_AND_COMPONENT_RUNTIME.md](AUTHORING_AND_COMPONENT_RUNTIME.md) for ownership rules,
[APPLICATION_AND_SHELL_PRIMITIVES.md](APPLICATION_AND_SHELL_PRIMITIVES.md) for domain boundaries,
and [PLATFORM_INTEGRATION_CONTRACT.md](PLATFORM_INTEGRATION_CONTRACT.md) for host integration.

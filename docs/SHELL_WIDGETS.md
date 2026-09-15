# Component-owned shell widgets

## Status

This document describes the implemented component API and Linux host integration. Portable geometry,
reactivity, lifecycle, service-admission, and headless host tests provide automated evidence. Native
KMS/input behavior still needs user-run qualification. This is not a production qualification claim.

## Ownership and migration

`ShellEnvironment` owns widgets. `Compositor` owns windowing/rendering configuration and window-frame
templates; it no longer accepts `.background(...)` or `.policy(...)`. Configure its cursor theme and
pass it directly to `.compositor(...)`. `ReadyCompositor` is a compatibility type alias for a
cursor-configured compositor, not another component owner.

The former `ShellWidget::new(...).content(component)` wrapper and `.shell_widget(...)` have been
removed. Implement `ShellWidget: Component` on the component and register the constructed instance
with `.widget(...)`. Zero widgets is valid. Each registration is one persistent instance on the
host's selected output; nothing implicitly clones component state across monitors.

```rust,ignore
Application::shell_environment("My shell")
    .compositor(Compositor::new()
        .cursor_theme(cursor_theme())
        .window_frame(easy_window_frame(chrome)))
    .widget(Wallpaper::new(wallpaper))
    .widget(Taskbar::new())
    .widget(Search::new())
    .run()
```

Both `view()` and `surface()` execute in the same component evaluation scope, so `self.watch(...)`
in either subscribes the persistent component to the signal. Local state remains ordinary
`#[state]` data. A descriptor change does not remount the root or reset its button state.

```rust,ignore
impl ShellWidget for Taskbar {
    fn surface(&self) -> ShellSurfaceSpec {
        ShellSurfaceSpec::new()
            .placement(WidgetPlacement::edge(ShellEdge::Bottom).height(48.0))
            .layer(ShellSurfaceLayer::Panel)
            .reserve_space(ShellReservation::WhenVisible)
            .movement(GeometryMotion::Spring(Spring::new()))
            .enter_from(ShellEdge::Bottom)
            .exit_to(ShellEdge::Bottom)
    }
}
```

## Geometry and presentation

`WidgetPlacement` supports fill, edge, normalized alignment, center, explicit output-local position,
and anchor rectangles. Dimensions use logical units, with fill/content/fixed extents, minimum and
maximum constraints, margins, offsets, work-area selection, and optional on-screen containment.
Anchored popups flip to the opposite side when their preferred side has insufficient space.
`attached(edge)` follows the owning child surface's parent; `attached_to(rect, edge)` narrows the
anchor to a parent-local rectangle. The parent component must publish updated local geometry when
that rectangle changes. There is not yet an automatic retained-element anchor handle.

Background surfaces precede client windows; panel and overlay surfaces follow them. Explicit layer
and order, followed by a stable host identity, determine stacking. Ordinary widget surfaces never
paint above session lock content. The current Linux host drives its one selected output; `output`
can select its neutral `OutputId::MIN`, and an unavailable explicit output hides the widget. This
is not a multi-output host implementation or hotplug qualification.

`visible` is requested state. Exit motion retains presentation until the transition ends, while
hidden widgets retain their component state. Pointer pass-through, content-only hit testing,
full-surface hit testing, and modal pointer blocking are independent of reservation/focus.
Keyboard policy is None, OnClick, or OnOpen. Focus returns to a surviving client when the shell
focus group closes. Escape and outside presses call `dismissed(reason)`; the component accepts
by changing its state or an owner-provided visibility signal. The host does not overwrite it.
The raw `input(event)` hook can implement selection gestures or simple keyboard-driven overlays;
return true only when component state changed. Normal controls still receive retained UI input.
Native IME and full touch/pen routing are not provided by this hook.

## Motion and work area

Surface motion reuses the existing `GeometryMotion`, `WindowTween`, `Spring`, and analytic
`GeometryTrack` used by window motion. The same host loop schedules it; no timer thread or second
animation engine is introduced. Retargeting samples current position and velocity. Reduced-motion
policy snaps placement to its target. Layout resolves target size, while the sampled rectangle
scales the retained scene and maps pointer coordinates back into that scene.

Reservations apply only to edge panels. None, WhenVisible, Always, and fixed extents are supported.
They use target layout extents, not the intermediate animation rectangle, so sliding does not
reconfigure maximized windows every frame. Changes in accepted work area update maximized windows.

## Owned child surfaces

`children()` returns keyed `ShellChild::new(key, component)` entries. Same-key/same-type children
keep component state. Removal closes descendants before their owner; type replacement creates a
new surface identity. Children can extend outside the parent's rectangle and attach to its sampled
geometry. Hiding an owner hides its descendants. Keys must be unique and bounded; each owner is
limited to 64 children and the environment to 256 surfaces. Virtual shells/workspaces are outside
this API change.

## Services

`connected(ShellServices)` runs before mounting the widget. Store the handle in component state.
`windows()` is a reactive view of current generational `WindowId`, title, active/minimized/maximized
state. `request(id, action)` admits activate, minimize, maximize, or close requests to the host.
Requests are bounded, stale identities reject, and session lock denies execution. Completion
`Dispatched` confirms dispatch, not client closure or a new snapshot. Snapshots remain authoritative.
Application launching continues through `session::application(...)` or `session::command(...)`.

The environment's `.service(adapter)` installs additional typed services, resolved with
`services.service::<Adapter>()`. This permits notification, audio, brightness, and capture adapters
without putting those operations in a surface descriptor. Missing services return Unavailable.
This change does not implement native audio/brightness adapters, a notification daemon, tray
protocols, or screenshot readback. Capture adapters must explicitly define whether shell overlays
are included. The test compositor demonstrates window search, not an application index or IME editor.

## Reference audit

The prescribed adjacent `../other-rendering-libs` tree was absent. Corresponding upstream sources
were inspected instead, without copying code:

- Flutter `packages/flutter/lib/src/widgets/overlay.dart`, `OverlayEntry` and `OverlayPortal`:
  https://raw.githubusercontent.com/flutter/flutter/master/packages/flutter/lib/src/widgets/overlay.dart
- Qt `src/quicktemplates/qquickpopup.cpp`, exit transition/focus cleanup:
  https://raw.githubusercontent.com/qt/qtdeclarative/dev/src/quicktemplates/qquickpopup.cpp
- Authoritative wlr layer-shell XML, committed geometry, exclusive-zone and interactivity rules:
  https://raw.githubusercontent.com/swaywm/wlr-protocols/master/unstable/wlr-layer-shell-unstable-v1.xml

Extracted invariants: overlay lifetime follows ownership; focus release differs from the end of
presentation; attachment, geometry and input agree; reservation is independent of keyboard focus.
Rejected alternatives: remounting on movement, frame-by-frame reservation animation, a second
animation scheduler, and a taskbar-specific compositor builder. Derived tests cover retained
identity, exit presentation/input separation, interruption continuity, constrained popup flipping,
child removal, stale service requests, and reduced motion. This work does not change GPU resource
ownership, barriers, shaders, or graphics API contracts; no graphics specification change is needed.

## Manual verification

Run the test compositor yourself on the normal test setup. Verify wallpaper under windows, taskbar
buttons activating/restoring/minimizing windows, maximized windows respecting the panel, Move panel
animating between edges, Search windows opening with keyboard focus and filtering titles, and Escape
or outside press restoring client focus. Check click geometry during motion and session lock hiding
all ordinary widgets. Hardware-presenting applications are intentionally not launched by the agent.

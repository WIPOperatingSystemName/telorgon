# Component-owned shell widgets

## Status

This document describes the implemented component API and Linux host integration. Portable geometry,
reactivity, lifecycle, service-admission, and headless host tests provide automated evidence. Native
KMS/input behavior still needs user-run qualification. This is not a production qualification claim.

See [Application catalog and taskbar icons](APPLICATION_CATALOG.md) for environment-owned
application discovery and `self.context::<ShellContext>()` access from widgets and descendants.

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
are included. The test compositor demonstrates grouped taskbar icons and a hover window picker.

## Window preview slots and hover popups

`ShellContext::output_size()` returns the selected host output's logical extent. Views subscribe to
it just like cached window metadata, so responsive widget layout does not read native monitor handles.
The current host still selects one output; this is not a multi-monitor extension.

A widget can return `Vec<ShellWindowPreview>` from `window_previews()`. Each entry contains a live
managed `WindowId` and a rectangle in the widget's local logical coordinates. The rectangle must fit
entirely within the surface and contain finite positive dimensions. The host presents at most the
first 64 entries, aspect-fits the client's retained scene and subsurfaces into each slot, and clips to
the slot and surface. Slots follow the surface's sampled geometry during animation. They draw above
that widget's UI; leave the preview area clear of captions and controls. Buttons under the slots
remain the input targets. No input is forwarded to the depicted application.

```rust,ignore
fn window_previews(&self) -> Vec<ShellWindowPreview> {
    vec![ShellWindowPreview::new(self.window, RectF {
        x: 8.0, y: 40.0, width: 216.0, height: 128.0,
    })]
}
```

The normal client layer remains the only scene/image producer. Previews add placements referencing
its admitted content, without screenshot readback, a second import, or a second upload. Visible
clients update their previews as their retained content changes. Minimized clients show the last
retained content; they are not resumed just for hover. Stale generations, destroyed windows, and
unready content produce no preview draw. Widget slots never draw while session-locked. Server-side
frames and separate popup menus are not included; the widget supplies its own caption/header.

`dismiss_on_pointer_leave(true)` requests `PointerLeft` when the pointer leaves a popup and its
attachment anchor/connecting gap. Hover does not take keyboard focus. Escape can dismiss the topmost
visible surface requesting Escape dismissal even without keyboard focus. Mouse-wheel input is now
routed to the hit shell widget before clients. `ShellWidget::input(InputEvent::Scroll { .. })` can
implement a bounded list, while retained UI still receives scroll input.

The test taskbar groups by resolved application ID, falling back to the raw application identity;
windows with neither identity remain separate. Hover or clicking a multiple-window icon opens the
picker. Its previews target a shared height of 128 logical units, with each card's width derived from
`ShellWindow::preview_size` and capped at 320 units. Very wide content scales down proportionally
within that cap; unknown sizes use a 216-unit fallback. Cards have no outer panel padding or gaps.
The panel grows up to 92% of the
output width, then switches to a title list with wheel scrolling and up/down controls. Selecting a
card/list entry restores and activates it; the close button requests closing just that window.
Outside press, Escape, and leaving the hover region dismiss the picker. A single-window icon keeps
the existing minimize/restore toggle.

### Preview implementation audit

The adjacent `../other-rendering-libs` directory was unavailable for this change. No comparison of
its source implementations is claimed. This change safely reuses Telorgon's existing retained scene
placement and lifetime contracts: `shell_wayland/scene.rs` (`ImageScene`, `ShellComposition`, damage
fanout), `renderer/vulkan.rs` (retained scene/materialization ownership), and `widgets.rs` (child
surface lifetime and geometry). No GPU synchronization, external-image import, shader, or backend
resource-lifetime code changed. A new graphics mechanism was deliberately avoided.

The visual reference was the [Windows 11 thumbnail screenshot](https://image.itmedia.co.jp/ait/articles/2209/09/wi-win11taskbarlist01.png)
from [this taskbar article](https://atmarkit.itmedia.co.jp/ait/articles/2209/09/news027.html).
[Microsoft's DWM thumbnail contract](https://learn.microsoft.com/en-us/windows/win32/dwm/thumbnail-ovw)
describes source/destination thumbnail relationships. It informed the visual-only slot boundary;
Telorgon does not adopt DWM's API. Rejected alternatives were CPU screenshot copies and waking
minimized applications to draw previews. Headless tests cover shared scene reuse without another
upload, no unready-image draws, retained Vulkan delta validity, generational IDs, subsurface mapping,
lock hiding, hover handoff, scroll routing, fixed card sizing and overflow thresholds. Hardware
appearance and native pointer feel still require the user-run checks below.

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

Run the test compositor yourself on the normal test setup. Open several windows from one application
and one from another. Verify one icon per application, hover thumbnails, aspect-ratio preservation,
and single-click restoration after using the window's minimize button. Move from the icon across the
gap into the picker; verify it stays open and closes after leaving, outside-clicking, or Escape.
Open enough windows to exceed the 92% threshold, scroll the title list and select its last entry.
Close windows while the picker is open. Repeat with native Wayland and Xwayland applications and at
HiDPI. Check that session lock hides every ordinary widget and preview. Hardware-presenting
applications are intentionally not launched by the agent.

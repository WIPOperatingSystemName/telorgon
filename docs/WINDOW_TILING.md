# WindowTiling shell widget

Status: implemented for the Linux desktop's selected output, with headless tests. Native pointer
feel, appearance, Vulkan validation, and Wayland/Xwayland interoperability still require user-run
qualification. This does not add multi-monitor hosting.

Register the widget on the shell environment:

```rust,ignore
Application::shell_environment("My desktop")
    .compositor(compositor)
    .widget(WindowTiling::snap())
    .run()
```

The preset enables left/right halves, four corners, a destination preview, and shared-divider
resizing. Corners take priority over side edges. The pointer enters snap zones at output edges;
destination rectangles partition the work area excluding reserved panels. Release commits the
actual target, independently of preview animation progress. Title-bar movement beyond four logical
units restores saved floating geometry and rebases the move grab. Escape cancels a pending snap.
Dropping a window into a quadrant occupied by a half-screen tile moves the existing window into
the complementary quadrant on that side. Both use the normal tile transition and resize handoff;
the existing window keeps its original floating restore geometry. If it cannot fit the smaller
quadrant, it returns to floating geometry. Other conflicting destinations retain the existing
displacement behavior.

## Appearance and motion

```rust,ignore
.widget(WindowTiling::snap()
    .edge_threshold(16.0)
    .corner_threshold(48.0)
    .divider_hit_width(8.0)
    .preview(TilePreviewDesign {
        fill: Fill::Glass(GlassStyle {
            tint: ColorRgba8::rgba(70, 110, 180, 48),
            ..GlassStyle::liquid()
        }),
        border: Border::all(1.0, ColorRgba8::rgba(160, 200, 255, 220)),
        corner_radius: 12.0,
        padding: Insets::all(8.0),
        motion: TilePreviewMotion::smooth()
            .appear(tween_ms(120, Easing::EaseOut))
            .relocate(tween_ms(180, Easing::EaseOut))
            .disappear(tween_ms(90, Easing::EaseOut)),
    }))
```

`fill` reuses the shared `Fill::None` / `Fill::Color` / `Fill::Glass` material and `GlassStyle`.
Its tint-alpha and software/allocation fallback semantics are the same as
[resize glass](RESIZE_GLASS.md). Border uses `ui::Border`; distances are logical units. Invalid
threshold, border-width, and radius inputs are rejected at construction. The default preview is a
translucent blue box with a thin border and a 12-unit radius.

`padding` defaults to `Insets::ZERO` and affects preview geometry only. Use `Insets::all(8.0)`
for an outer gap or `Insets::new(top, right, bottom, left)` for per-side values. Insets apply
only where a tile touches the usable work-area boundary (after panel reservations), never
along an internal split. Adjacent halves and quadrants retain matching shared-edge coordinates.
The whole preview, including fill and border, uses the padded rectangle and its existing motion.
Snap activation, committed window geometry, and shared-divider resizing are unchanged.
Values must be finite and nonnegative; they round to logical integer coordinates before output
scaling and clamp to leave at least one logical unit of preview width and height. If vertical
padding cannot fit in a full-height tile, top padding takes precedence.

Appearance and disappearance fade the whole preview. Relocation animates its position and size.
`TilePreviewMotion::none()` disables all three effects; `.movement(GeometryMotion::Spring(...))`
provides the existing spring option. Retargeting samples the current geometry/opacity. Reduced
motion settles immediately. The placeholder remains below the dragged window and above lower
windows, takes no pointer or keyboard input, and is hidden under session lock. Glass uses live lower
layers, including during fades, through the existing bounded backdrop/snapshot machinery.

`WindowTiling` is an ordinary persistent `ShellWidget`; its surface descriptor registers policy and
presents the preview. The host owns authoritative membership, split coordinates, pointer grabs,
constraints and client configuration. Register at most one active manager on the selected output;
a duplicate returns an error. Removing its owner restores managed tiles to floating placement.

## Window transitions

Snapping a window into a tile or switching to another tile uses its existing
`WindowMotion::maximize` tween or `maximize_spring`. Returning to floating uses `restore`
or `restore_spring`, including a displaced occupant or title-bar drag. Moving between a
maximized window and a tile uses the maximize settings for the destination arrangement.
The same maximize-content fade and client-readiness handoff apply to these transitions;
no separate tiling motion settings are required. During a title-bar drag restore, position
follows the pointer while size animates. Shared-divider resizing continues to track the
pointer directly while each affected window uses its `WindowMotion::resize_content`
fade to enter the resize placeholder and return to ready content. Custom-frame measurement
preserves the active grab; releasing the divider waits for the final client image before
starting the return fade. After a shared-divider release, all surviving affected members
keep their placeholders until every member has final-size content, then start their configured
resize-content return fades on the same frame. A slow client delays this shared reveal; closed,
minimized, or untiled members leave the group. A new divider grab replaces the pending group. `WindowMotion::none()` and reduced-motion preferences still disable effects.

`TilePreviewDesign::motion` controls the snap-target preview widget only; the actual window
uses the chrome/host `WindowMotion` above. Preview padding still does not inset snapped windows.

## Shared resizing and commands

One vertical split divides left and right columns; each column has an independent horizontal split.
A half occupies a whole column, and two quadrants share that column's horizontal split. Hovering a
shared border shows the horizontal/vertical resize cursor. Dragging changes the affected members
together. Integer rectangles share the same rounded split coordinate, including odd work-area sizes.
Size limits constrain the split; incompatible snap destinations are declined. X11 grid/aspect hints
must also accept the destination; divider movement finds a compatible integer split. A floating
window above a divider occludes its hit region.

`.halves(false)`, `.quadrants(false)`, and `.shared_resize(false)` customize behavior. Disabled shared
resizing removes inner resize authority; output edges do not become ordinary floating resize grips.
Closing or minimizing a member leaves split positions intact. Maximize/fullscreen leave the tile
while retaining the original floating restore geometry. Work-area changes recalculate tile rectangles;
windows whose constraints no longer fit return to floating placement. Lock/input suspension ends
active shared grabs without leaving clients permanently in interactive-resize state.

Shell controls and keyboard handlers can use the same policy:

```rust,ignore
services.request(window, ShellWindowAction::Snap(TileTarget::TopLeft))?;
services.request(window, ShellWindowAction::Float)?;
// Equivalently, ShellContext::windows().snap(window, target) / .float(window).
```

Targets are `Left`, `Right`, `TopLeft`, `TopRight`, `BottomLeft`, and `BottomRight`.
Requests retain the existing stale-ID and lock checks. Disabled or infeasible snaps complete as
`Denied`. This feature does not add a window suggestion picker or bind desktop shortcuts implicitly.

The client resize/configure and readiness paths remain shared with ordinary resizing: geometry
follows the pointer, final sizes get terminal requests, and late client content cannot establish
readiness for a newer transaction. Custom frames measure the tiled outer rectangle to derive their
content slot. XDG clients receive tiled edge state; X11 uses its existing checked geometry pipeline.

## Reference audit

The adjacent `../other-rendering-libs` directory was unavailable. Inspected upstream sources:

- [KWin `src/tiles/tile.cpp`](https://raw.githubusercontent.com/KDE/kwin/master/src/tiles/tile.cpp),
  relative geometry, work-area mapping, size constraints and `resizeFromGravity`.
- [Sway `sway/tree/view.c`](https://raw.githubusercontent.com/swaywm/sway/master/sway/tree/view.c),
  `view_autoconfigure`, frame/content geometry and fullscreen separation.
- [Official XDG shell XML](https://raw.githubusercontent.com/wayland-mirror/wayland-protocols/main/stable/xdg-shell/xdg-shell.xml),
  tiled edges, interactive resizing and configure/acknowledgement ordering.

Adopted invariants: one authoritative shared boundary; work-area geometry includes decorations;
client publication is asynchronous; restore geometry survives state changes. Rejected independent
window rectangles, widget-owned native handles, timer-based readiness, and a second animation engine.
No external code was copied. Existing render targets, barriers, cache budgets, and retirement rules
are reused; renderer changes give preview glass distinct cache identities and permit the existing
live optical recipe on widget opacity groups. No new GPU synchronization or lifetime mechanism is
introduced.

Headless coverage includes odd-size partitions, conflicting slots, min/max clamping, independent
horizontal dividers, tiled protocol state, drag restoration, floating-window occlusion, manager
removal, preview retargeting/fades/input exclusion, and the public glass/style API. Existing frame,
resize, widget, and glass tests provide regression coverage. Run the test compositor manually to
check corner-to-half transitions, glass against moving windows, fast interrupted previews, divider
cursors, slow clients, panels, HiDPI, and both native Wayland and Xwayland windows. Agent-run GUI or
hardware-presenting tests are prohibited by repository guidance.

Divider updates skip unchanged rectangles and pointer samples that resolve to the current split
pixel. These remove redundant host-side work; they are not a measured GPU performance result.
For user-run lag investigation, `TELORGON_FRAME_STATS=1` records render CPU time, frame cadence,
and input queue age in the compositor log. Live glass and snapshot costs remain hardware-qualified.

## Preview owner lifetime

The snap widget outlives the window that last displayed it. Desktop composition must keep its
retained scene live, and deliver pending deltas, even before the first owner exists or after that
owner closes. Without that invisible ownership layer, the backend discards the scene while the
widget runtime keeps its incremental baseline. A later edge preview at a different size then sends
partial glyph/spatial updates into an empty backend scene and fails Vulkan growth validation.

Unowned previews remain invisible and do not publish a glass material. Owned previews retain their
existing placement immediately below the owning window. Ownership is released when the widget
itself unmounts, rather than when its last window disappears. The headless regression drives the
real snap widget through owner closure and differently sized reentry, applies its scene deltas to
the Vulkan CPU mirror, and checks that no orphan preview is drawn.

Audit: inspected `layers.rs::prepare_desktop_layers`, `widgets.rs::WidgetLayer::scene`,
`tiling.rs` candidate publication, `scene.rs` live-scene retirement, and
`renderer_vulkan/executor.rs` growth validation. This preserves the existing retained-scene
contract documented in REFERENCE_IMPLEMENTATIONS.md; no Vulkan allocation, synchronization or
shader mechanism changes. The adjacent reference-source library remains unavailable. Rejected
alternatives: bypassing growth validation, recreating every preview from a full snapshot, or
keeping every retired shell scene indefinitely. The regression uses no GPU device or GUI.

Divider ratios belong to the current tiled layout. Once no windows remain tiled, all three
dividers reset to 50%, including before a preview or a snap in the same input batch. Existing
nonempty layouts retain their ratios so newly filled slots stay aligned with their neighbors.

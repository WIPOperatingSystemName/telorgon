# Maximized custom window geometry

Maximized custom frames use the logical shell work area as their outer extent. The host lays
out the frame in its maximized state and configures the client to the resulting content-slot
size. It does not grow or shrink that outer extent to preserve an earlier client request.
The measured configure is flushed before presenting the updated frame. The initial legacy
fallback configure may be superseded by this measured size through the existing scheduler.
Client-decorated windows receive the full work-area size without server decoration deductions.

The easy frame suppresses its palette border in maximized and fullscreen states. Maximization
retains the authored title bar; normal and tiled states retain their authored border. Restoring
continues to use the saved client size and position. True fullscreen keeps its existing separate
output-sized, undecorated host path. No physical/logical conversion is added here.

## Content placeholder

Maximizing uses the same `resize_preview_color` placeholder as interactive resizing. It hides
the existing client content until the terminal configure is acknowledged and a corresponding
surface publication is applied. Custom frame measurement replaces the pending transaction when
it supersedes the fallback size, so an acknowledgement for the fallback cannot reveal old pixels.
The title bar remains visible, and maximizing does not set the protocol's interactive `resizing`
state. Restoring before completion cancels the pending maximize placeholder.

The existing resize scene/alpha/commit regressions also cover the shared rendering path. Additional
CPU checks verify maximize starts the veil without a pointer grab, restore cancels it and preserves
saved geometry, and the measured configure requires its own acknowledgement.

## Reference audit

The adjacent reference checkout is unavailable. Following the reference guide's bounded-change
fallback, these upstream paths were inspected:

- [Sway `sway/tree/view.c`](https://github.com/swaywm/sway/blob/master/sway/tree/view.c),
  `view_autoconfigure`: derive content geometry from the container's actual border/title-bar policy;
  fullscreen derives geometry from the output independently.
- [KWin `src/xdgshellwindow.cpp`](https://github.com/KDE/kwin/blob/master/src/xdgshellwindow.cpp),
  `XdgSurfaceWindow::moveResizeInternal`: convert requested frame dimensions into client dimensions
  before scheduling configuration; acknowledgement/commit tracking remains separate.
- [XDG shell protocol](https://github.com/wayland-mirror/wayland-protocols/blob/main/stable/xdg-shell/xdg-shell.xml),
  `set_maximized` / `unset_maximized`: the compositor chooses the region and communicates the state
  and geometry by configure; restoring may recover previous geometry.

Invariants: work area owns maximized outer geometry; the composed content slot owns custom client
size; configure delivery and client commit remain asynchronous; normal restore geometry survives.
Rejected alternatives: copy test-app constants into LinuxDesktopConfig, subtract the normal-state
border from maximized geometry, or use physical scanout dimensions. Those couple policy to legacy
metrics, retain unwanted insets, or break output scaling. No reference source was copied.

CPU tests exercise custom title heights, multiple raster scales, a work area with reserved panel
space, preserved restore client size, and borderless maximized/fullscreen content reaching the
right and bottom frame edges. Interactive compositor validation is left to the user.

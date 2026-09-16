# Shell application catalog and taskbar icons

## Implemented API

The shell environment owns the catalog and its worker. There is no process-global catalog instance.
Components and ordinary descendant components access the enclosing environment through a driver-scoped
provider. Access is available while their views, lifecycle hooks, and event handlers execute.

```rust,ignore
Application::shell_environment("My shell")
    .applications(ApplicationCatalog::system().watch_changes(true).icon_theme("hicolor"))
    .compositor(Compositor::new().cursor_theme(cursor_theme()))
    .widget(Taskbar::new())
    .run()
```

Omitting `.applications(...)` selects the system catalog. `.locale("fr-CA")` overrides the process
message locale. `.data_directories(...)` replaces the ordered XDG data roots, and
`ApplicationCatalog::in_memory(entries)` provides deterministic test metadata. Merely constructing
configuration starts no worker.

```rust,ignore
let shell = self.context::<ShellContext>();
let windows = shell.windows();
let applications = shell.applications();
let matches = applications.search(ApplicationQuery::new("editor").limit(20));

for window in windows.open() {
    let icon = windows.icon(window.id);
    // button(window.title).icon(icon).key(stable_window_key)
}
```

These are cached snapshot reads. They register signal dependencies automatically during `view()`;
reads from input handlers do not register subscriptions. `try_context::<ShellContext>()` returns
`None` outside a shell provider. `context()` reports a missing-provider diagnostic. Providers are
inherited by every component in the mounted shell widget and isolated between environments, including
nested execution and panic unwinding. Existing `ShellWidget::connected`/`ShellServices` code remains
compatible; new components need not store service handles in state.

`windows.open()` returns managed windows, including minimized windows. Each has a generational
`WindowId`, optional `ApplicationId`, title, and active/minimized/maximized state. Closing removes the
entry. Unmanaged X11 surfaces and Wayland popups are excluded. Association uses the Wayland app ID or
X11 WM_CLASS, matching desktop-entry IDs and unambiguous StartupWMClass aliases; it never guesses from
a window title. Identity can arrive later and trigger an update. `ApplicationId` is a desktop-entry
ID, not an executable or package-manager identifier.

`windows.activate(id)`, `set_minimized(id, bool)`, and `close(id)` submit requests and return a request
ID or admission error. Activation restores minimized windows. Stale windows, shutdown, queue limits,
and the existing session-lock policy retain their previous handling. Admission is not client completion.

## Metadata and discovery

The worker searches `$XDG_DATA_HOME/applications` and each `$XDG_DATA_DIRS` application directory in
precedence order, with XDG defaults for unset/empty variables and rejection of relative data roots.
Nested desktop-entry paths become IDs using the specified slash-to-hyphen transformation. A user
`Hidden=true` entry masks lower-priority entries. Discovery preserves launcher-hidden metadata for
exact lookup and window association. Launcher lists and search respect NoDisplay, OnlyShowIn,
NotShowIn, TryExec, and the presence of Exec or D-Bus activation metadata.

Metadata includes localized name, generic name, description, keywords, categories, icon reference,
launcher visibility, and StartupWMClass. Localization follows POSIX locale fallback, and desktop-entry
string/list escapes are decoded. Search ranks exact names, prefixes, substrings, then keyword/generic
name/description matches, with deterministic ordering and a result limit. Search scans cached metadata;
it does not perform filesystem work. `status()` distinguishes Loading, Ready, and Failed, preserving
the last successful snapshot on scan failure.

Change watching currently uses a two-second worker-side rescan, not inotify. Traversal is bounded to
16 directory levels and 16,384 desktop-entry IDs, with a 1 MiB per-file read limit. Directory symlinks
are not traversed; exported desktop-file symlinks are read. No package-manager subprocesses are run.

## Icons

`windows.icon(id)` resolves client-supplied pixel data, a client icon name, associated application
metadata, or a built-in generic window icon. Wayland uses xdg_toplevel_icon_v1; X11 asynchronously reads
WM_CLASS and _NET_WM_ICON through the existing bounded, revision-checked property reader. Invalid or
late X11 replies cannot repopulate retired window generations.

Application icons can be rendered as `image(metadata.icon.unwrap_or_default())` inside shell
components, or obtained through `applications.icon(&id)`. For explicit sizing:

```rust,ignore
applications.resolve_icon(&id, IconRequest::new().logical_size(48).scale(output_scale))
```

For window icons, `windows.resolve_icon(id, IconRequest::new().logical_size(44))` requests
catalog artwork at the rendered size and output scale. Client-supplied X11 and Wayland icons
retain the largest available variant so larger taskbar buttons do not upscale a 32-pixel image.

The default application-icon raster request is 32 logical units at the host's output scale. Explicit
requests are capped at 256 physical pixels. Named icon lookup honors the configured theme's directories,
size ranges, scale, inheritance, hicolor fallback, and unthemed icon/pixmap paths. The default theme is
hicolor; automatic theme selection from other desktops' settings is not implemented. PNG and SVG are
supported; XPM is not. SVG external/data image references are disabled. Reads are capped at 4 MiB,
raster source dimensions at 4096, and decoder allocation at 64 MiB. Aspect ratio is preserved.

Named icon lookup and decoding happen on the catalog worker. A fallback is returned immediately while
loading. At most 512 distinct name/size requests are retained per environment, with a bounded request
queue. Requested icons are checked for path/mtime/length changes during watching; failures can recover
when files change. Each component runtime retains the exact resources selected during view evaluation.
After compilation, it registers those resources in the same delta as their first draw, and retires
resources only after mounted image nodes and retained draws no longer reference them. This prevents
an asynchronous icon completion from publishing a draw before its image data. Image replacement also
rebuilds the draw batch when the texture or clip identity changes. CPU Vulkan admission tests exercise
this transition on a persistent button. The worker is stopped
and joined at environment shutdown.

## Test compositor

The taskbar renders one icon per resolved application (raw identity fallback, isolated unknown
windows). Hover opens fixed-size client previews and switches to a scrollable title list when the
row would exceed 92% of the output width. See [Shell widgets](SHELL_WIDGETS.md) for the preview API.
App names/window titles remain accessible labels; active groups have a distinct icon background.
Clicking a single-window icon toggles minimize/restore; clicking a grouped icon opens the picker.
Selecting a picker entry restores and activates that window. There are no Move panel or Search
buttons or window-search component. Pinning, application launching, and virtual shells remain
outside this change.

## Evidence and limits

Headless tests cover provider isolation/unwinding, inherited descendant access, reactive dependencies,
existing windows at mount, open/minimized/closed entries, delayed icon resources, click requests,
XDG overrides/localization/visibility, theme inheritance and image bounds, async discovery/icon loading,
change polling, and X11 icon/class parsing. X11 mock-socket regression tests require permission to
create local Unix sockets. Compile checks cover the Linux/Xwayland library, core feature configuration,
and consuming test compositor. Live KMS/Wayland/Xwayland appearance remains user-qualified.

The host still has one active output. Per-element automatic icon size inference, automatic external
desktop theme changes, full fuzzy search, application actions/launching, and native filesystem watches
are not implemented. X11 client-icon variant selection currently prefers 32 pixels; application catalog
icons and Wayland taskbar icons account for the output scale.

Reference audit: the adjacent `other-rendering-libs` source library was unavailable. This change uses
existing Telorgon component signal tracking, scoped driver execution, async X11 PropertyReader
lifetimes, and image-resource registration; it introduces no graphics-backend contract. Reviewed
[Desktop Entry](https://specifications.freedesktop.org/desktop-entry/latest-single/),
[XDG Base Directory](https://specifications.freedesktop.org/basedir/latest/),
[Icon Theme](https://specifications.freedesktop.org/icon-theme/latest/), and
[EWMH](https://specifications.freedesktop.org/wm/latest-single/) specifications. The derived invariants
are ordered overrides, hidden-entry masking, locale fallback, theme inheritance, bounded image parsing,
and rejecting stale property replies. Package-manager-specific lookup and blocking work in views were
rejected. No external implementation code was copied.

# Telorgon ScreenCast portal integration

This is the in-progress monitor/window-sharing backend. See
[`SCREENCAST_IMPLEMENTATION_STATUS.md`](../../docs/SCREENCAST_IMPLEMENTATION_STATUS.md) for actual
verification and remaining implementation. Live portal/PipeWire/Discord interoperability has not
yet been qualified. Do not package it as a finished screen-sharing feature.

## Build and runtime dependencies

Enable `telorgon/shell-screencast-linux` in the compositor's Cargo features and add
`.capture(Capture::desktop())` to its shell-environment declaration (see
[the capture API](../../docs/CAPTURE_API.md)). This includes
`shell-wayland-linux`, pinned `pipewire` Rust bindings 0.10.1, and optional bounded-channel/timer
dependencies. The binding enables the PipeWire 0.3.34 API level; build against matching or newer
PipeWire/SPA development headers with pkg-config and libclang available for bindgen. Runtime needs
libpipewire, the user's PipeWire daemon/session manager, the existing user D-Bus, and
xdg-desktop-portal. Existing application/embedded builds do not enable these native dependencies.

The managed Vulkan compositor starts the backend worker after renderer/widget setup. The software
host does not advertise it. The consent controls require two free slots within the shell's 256-surface
limit; insufficient initial capacity disables the backend with a diagnostic. The worker owns
`org.freedesktop.impl.portal.desktop.telorgon` and
exports `/org/freedesktop/portal/desktop`. It retries bus connection failures and revokes sessions
before reconnecting. It does not start D-Bus, PipeWire, xdg-desktop-portal, or another compositor.

## Distribution files and startup

- Install `telorgon.portal` under the distribution's `share/xdg-desktop-portal/portals/` directory.
- Install `telorgon-portals.conf` under `share/xdg-desktop-portal/`, or use the desktop-specific
  `~/.config/xdg-desktop-portal/telorgon-portals.conf` for local qualification. Merge the ScreenCast
  entry into an existing Telorgon config rather than replacing its other portal choices. Configure
  another installed backend for file chooser and other interfaces as appropriate for the desktop.
- Use `XDG_CURRENT_DESKTOP=telorgon` for this configuration. The managed session's default identity
  is `telorgon`; if the downstream compositor changes it, name the desktop-specific config after
  that identity (ASCII lowercase). Preserve the existing desktop's portal configuration.
- Publish the managed session's `WAYLAND_DISPLAY`, `XDG_CURRENT_DESKTOP` and user-bus environment
  through the existing Telorgon session environment integration before activating the frontend.
  Read `SESSION_AND_PROCESS_LAUNCHING.md` for that integration's single-session assumptions.
- Start the compositor before the frontend resolves its backend. If the frontend was already
  started with stale desktop configuration, restart it from the Telorgon session during manual
  qualification. Backend bus reconnection does not guarantee an old frontend reloads config.

There is intentionally no D-Bus `.service` activation file: an activation `Exec` must not launch a
second compositor. These files are packaging inputs, not an installer; no system/user service
configuration is changed by compiling this feature.

## Contract and access control

The implemented backend advertises ScreenCast version 3, MONITOR | WINDOW with hidden and embedded cursor modes.
Virtual-output and persistent-grant capabilities remain unadvertised. Unverified X11
override-redirect surfaces are excluded from window capture. Every Start
requires the host's explicit consent dialog, followed by actual PipeWire node readiness. A persistent
indicator lets the user stop each session. Lock, cancellation, frontend disconnect and stream failure
revoke the corresponding capture session. Public Wayland capture globals are not enabled by this feature.

Only the current owner of `org.freedesktop.portal.Desktop` can create/select/start backend sessions.
Close objects bind to that owner's unique bus name. Application IDs are labels and quota keys, not
proof of authorization. The frontend validates application ownership and provides the public
`OpenPipeWireRemote` method; Telorgon does not duplicate that method or pass an unrestricted remote FD
to applications. The distribution must retain xdg-desktop-portal's PipeWire access-control setup.
Ordinary unsandboxed clients with unrestricted access to the user's PipeWire socket remain subject
to that session's PipeWire policy; portal consent is not a sandbox for those clients.

References: [backend contract](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.impl.portal.ScreenCast.html),
[backend discovery](https://flatpak.github.io/xdg-desktop-portal/docs/writing-a-new-backend.html),
[desktop configuration](https://flatpak.github.io/xdg-desktop-portal/docs/portals.conf.html), and
[PipeWire access integration](https://flatpak.github.io/xdg-desktop-portal/docs/pipewire.html).

## User-run qualification

Repository policy leaves live applications and services to the user. Inside the running Telorgon
session, inspect registration without invoking private backend methods directly:

```sh
busctl --user introspect org.freedesktop.impl.portal.desktop.telorgon /org/freedesktop/portal/desktop
busctl --user get-property org.freedesktop.impl.portal.desktop.telorgon /org/freedesktop/portal/desktop org.freedesktop.impl.portal.ScreenCast AvailableSourceTypes
busctl --user get-property org.freedesktop.impl.portal.desktop.telorgon /org/freedesktop/portal/desktop org.freedesktop.impl.portal.ScreenCast AvailableCursorModes
journalctl --user -u xdg-desktop-portal.service -b
```

Expected capability values are currently `3` and `3`. An ordinary application's direct private
backend request must be rejected. Use a portal-enabled browser's `getDisplayMedia` workflow or an
OBS PipeWire capture source for the first actual video test, then test Discord. Confirm:

1. Deny the chooser: no live video node survives; repeat and approve to obtain moving monitor pixels.
2. Leave the desktop stationary before starting: an initial frame must still arrive.
3. Stop using the shell indicator and repeat ten times without restarting the compositor.
4. Close the consumer during the chooser, negotiation and active delivery; each must clean up.
5. Lock during sharing: the stream stops and no lock-screen content is delivered. Unlocking must
   not silently restore permission. Restarting the frontend must also revoke old sessions.
6. Start two consumers; stop one and verify the other remains functional. Slow a consumer and
   observe bounded retained storage and a responsive desktop.
7. Request hidden and embedded cursor modes separately. With a hardware cursor enabled, verify
   embedded capture shows exactly one pointer and hidden capture shows none. Move only the pointer
   over a stationary desktop; check embedded frames update. Repeat at fractional scale, switch cursor
   shapes, and move outside the output and back to check hotspot and source retirement.
8. Inspect the remote obtained through OpenPipeWireRemote: unrelated capture nodes must not be
   visible through that restricted remote. Report the session manager and policy configuration.

Record compositor revision, frontend/PipeWire/client versions, GPU/driver, scale, pixel format,
source, cursor mode and observations. Monitor color, orientation, memory bounds, buffer reuse,
and actual Discord compatibility remain qualification gates. DMA-BUF and direct capture protocol tests will be added with those implementations;
the checklist above does not establish their completion.

### Window qualification

Repeat with a native Wayland application and a managed XWayland application. Request WINDOW-only
and confirm no monitor choice appears; request both kinds and select an individual window. Cover
it with another application and move it off-screen: only its client content and verified owned
subsurfaces/popups may appear, with unused target pixels black. Check that animation continues
while obscured, cursor coordinates follow the captured target, and resize keeps the PipeWire node.
Minimize, unmap or close the source: sharing must stop. Remapping the same desktop window ID must
not resume an old stream or accept an old chooser activation. Test native popup bounds and record
X11 popup omissions explicitly; override-redirect ownership is not yet qualified. Exercise more
than five sources to check the chooser's next/previous controls. Record failures separately from
monitor capture and do not infer isolation from the headless placement tests alone.

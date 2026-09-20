# Compositor decoration policy

`Compositor::decoration_policy(DecorationPolicy)` selects startup policy for managed Wayland and
Xwayland windows. It is independent of `window_frame(...)`, which supplies the visual template.

```rust
use telorgon::app::*;

const DECORATIONS: DecorationPolicy = DecorationPolicy {
    negotiation: DecorationNegotiation::ClientPreference,
    title_bar: TitleBarPolicy::Automatic,
    outer_frame: OuterFramePolicy {
        border: FramePartPolicy::Automatic,
        rounded_clip: FramePartPolicy::Automatic,
        shadow: FramePartPolicy::Automatic,
    },
    interaction: FrameInteractionPolicy {
        resize_regions: ResizeRegionPolicy::Automatic,
    },
};

let compositor = Compositor::new().decoration_policy(DECORATIONS);
```

This is also `DecorationPolicy::default()` / `DecorationPolicy::DEFAULT`: Telorgon chooses server
decorations unless the client explicitly requests its own. Native clients use xdg-decoration;
managed X11 clients use the existing Motif decoration hints. `PreferServer` overrides those requests.
A client that never negotiates may still draw embedded controls: no policy can remove its pixels.
This preserves Telorgon's existing server-by-default behavior for non-negotiating Wayland clients;
it does not imply that the Wayland protocol requires such clients to stop drawing decorations.

`TitleBarPolicy::{Automatic, Always, Never}` controls Telorgon's title bar only. Individual icons,
buttons, layout, colors, sizes, and interaction styles remain in the existing chrome template.
`FramePartPolicy::{Automatic, Always, Never}` independently controls border, rounded clipping, and
shadow. Automatic follows decoration ownership, not title-bar visibility. `Always` makes a part
available even when the client owns its header. Fullscreen exclusion, fixed-size capabilities,
and normal/maximized/tiled frame styles still apply; Always does not force nonzero style metrics.
`ResizeRegionPolicy::{Automatic, Enabled, Disabled}` controls compositor frame hit regions, not
client requests, keyboard actions, or general permission to resize.

For a Firefox-owned header with Telorgon outer styling, keep negotiation and title bar automatic,
set the three outer-frame parts to Always, and optionally enable resize regions. No additional
server title bar is added when Firefox explicitly selects client decorations. Client shadow pixels
are not stripped; the existing declared window geometry and content placement remain authoritative.

The policy is copied into the shell at startup and has no live mutation API or per-app override.
Unmanaged X11 windows, popups, and fullscreen windows are excluded. Under the new default, X11 CSD
windows no longer implicitly retain a server border; choose Always to retain the previous outer look.

## Frame-template contract

The host resolves title ownership into `WindowChromeModel::title_bar_visible` and the four independent
parts into `WindowChromeModel::frame_parts: WindowFrameParts`. `easy_window_frame` honors both,
removes disabled borders from layout, suppresses disabled shadows and resize targets, and uses the
resolved corner shape for frame and client clipping. Custom templates must honor these fields too;
the host cannot identify arbitrary user-authored title controls or decorative nodes in a template.
A policy does not generate visuals absent from a custom template. Window action capabilities remain
separate from whether buttons are visible.

Wayland decoration changes emit a decoration event and an xdg configure, preserving queued size and
state. Effective ownership changes only when the client acknowledges and commits that configure.
Frame removal clears cached frame geometry. X11 hint changes preserve client placement and update
extents using the same resolved policy, including when server preference overrides client hints.

## Implementation evidence and review

The adjacent `../other-rendering-libs` library is absent in this checkout, so no independent reference
implementation review is claimed. This CPU policy change reuses existing frame composition,
geometry, clipping, and input mechanisms; GPU resource/synchronization contracts are unchanged.
Reviewed Telorgon paths: `application_host/declaration.rs`, `shell_wayland/{client,geometry,layers,
input,x11_windows}.rs`, `compose/components/easy_window_frame.rs`, and `compositor_wayland/{native,
xdg}.rs`. The upstream Wayland protocol source was reviewed from the local wayland-protocols-1.47
source at `../xwayland-build/work/build-run/sources/wayland-protocols-1.47/unstable/xdg-decoration/`:
`set_mode` permits server choice; `configure` requires acknowledgement and commit before application.

Rejected: one server-decorated boolean controlling every visual; applying decoration changes before
client commit; changing client content dimensions to hide borders; overriding unrelated client or
keyboard resize permissions. Tests cover wire negotiation/default/unset/server override and commit
ordering, independent visual combinations and resize hit regions, backend parity and fullscreen/
popup exclusion, stale geometry removal, X11 hint transitions, and public builder propagation.
Live Firefox/Discord appearance remains user-run; no GUI or compositor was launched during this work.

Validation: `cargo test -p telorgon --offline --no-default-features --features shell-xwayland --lib`
passed 1,468 tests (8 ignored). The `decoration_policy_api` and `window_frame_api` integration suites
passed 16 tests. The test compositor passed `cargo check --offline` and the embedded-Xwayland check
with `TELORGON_XWAYLAND_PAYLOAD` pointing to the workspace payload and
`--features telorgon/shell-xwayland-embedded`. Existing unused-code/import warnings remain.

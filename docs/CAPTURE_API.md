# Capture configuration

See [the visual sharing picker](SCREENCAST_PICKER.md) for thumbnails, selection, stop controls,
verification and the outstanding physical multi-output prerequisite.

The shell environment explicitly opts into capture:

```rust
use telorgon::app::*;

let desktop = Application::shell_environment("My Desktop")
    .capture(Capture::desktop());
```

Continue the declaration with Linux settings, a compositor, widgets, and `run()` as usual.
The test compositor uses this preset in `src/main.rs`. `Capture::desktop()` enables portal capture
of monitors and windows with automatic session integration and the default chooser. An existing
`Compositor::capture_chooser` replaces that chooser while retaining host consent validation and the
sharing indicator. The PipeWire/portal code still requires the `shell-screencast-linux` Cargo feature.

`Capture::new()` and an omitted `.capture(...)` enable **no interfaces**, even when that feature is
compiled. This replaces the earlier implicit portal startup based solely on a Cargo feature.
`.capture(...)` is available before or after `.compositor(...)` and after `.widget(...)`; each call
replaces the complete previous capture configuration. Builders and getters do not launch services.

## Expanded configuration

```rust
use telorgon::app::*;

let capture = Capture::new()
    .sources(CaptureSources::MONITORS | CaptureSources::WINDOWS)
    .portal(
        PortalCapture::new()
            .session_integration(PortalSessionIntegration::Automatic),
    );
```

`CaptureSources::MONITORS`, `WINDOWS`, `all()`, and `empty()` support `|` and `union()` (also usable in
const expressions). Enabled capture requires at least one source type. The same selection controls
portal capability advertisement, the source types admitted by SelectSources, the chooser, and the
host's available source registry. A mixed request is restricted to enabled source types; a request
with no enabled types fails and closes its session. Virtual monitors are not implemented.

## Session integration

The installation still provides `telorgon.portal` and a configuration named after the session's
identity. Building a Rust application does not install or overwrite system/user portal files.

- `Automatic` registers the backend. If `SessionConfig::publish_user_service_environment` is true,
  the host's existing session guard publishes the environment before backend startup, and the
  backend worker asks D-Bus to activate the frontend after backend registration. If that flag is
  false, shared frontend and environment management remain external.
- `DesktopSession` requires that ownership flag to be true; otherwise declaration validation fails
  before opening display devices or creating a desktop session. It uses the same publication and
  frontend activation path as owned Automatic mode.
- `External` registers the backend without activating or restarting the frontend. Independently
  configured session-wide environment publication is not overridden by this capture setting.

No mode infers ownership from environment strings, restarts an already-running frontend, or takes
over another desktop's portal configuration. D-Bus activation leaves an existing frontend running;
if it loaded old configuration, an explicit user/session-manager restart is still necessary.
Activation failures are reported by the worker and do not block rendering. Backend reconnection
retains the existing cancellation and lease-revocation behavior. Portal/PipeWire interoperability
with Discord remains a live qualification task; registration alone does not prove video delivery.

## Reserved interfaces

The complete declaration syntax also includes:

```rust
use telorgon::app::*;

let future_interfaces = Capture::new()
    .wayland(
        WaylandCapture::new()
            .protocols(CaptureProtocols::ImageCopyCapture)
            .access(DirectCaptureAccess::Ask),
    )
    .internal(InternalCapture::new());
```

These values are representable and inspectable, but **enabling either currently returns a clear
startup error**. Direct Wayland consent and component-facing delivery are unfinished; no capture
global or unrestricted access is enabled by these builders. This is intentional, not a silent
fallback to portal capture. The low-level direct-protocol code remains separate and restricted.

## Validation and implementation review

An enabled portal requires Linux with the screencast Cargo feature, Vulkan (including successful
Auto selection), and two free shell widget slots. Missing prerequisites are explicit errors rather
than silently disabling a requested interface. Internal/direct declarations, empty source sets,
and unowned DesktopSession mode are rejected before device/session startup.

The adjacent `../other-rendering-libs` reference library was unavailable. This change is restricted
to declaration propagation, CPU source policy, and existing D-Bus session integration; it changes
no GPU capture, external-image, rendering or synchronization machinery. Reviewed existing paths:
`application_host/declaration.rs`, `shell_wayland.rs`, `shell_wayland/capture_portal.rs`,
`portal_linux/{mod,state}.rs`, `compositor_wayland/capture_access.rs`, and the direct capture scheduler.

The upstream [ScreenCast backend contract](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.impl.portal.ScreenCast.html)
and [session integration rules](https://flatpak.github.io/xdg-desktop-portal/docs/system-integration.html)
were checked. Invariants: advertise only admitted source types; consent binds the selected source;
registration precedes frontend activation; declaration alone does not access a bus or launch a
service. Rejected alternatives: automatic exposure of privileged direct capture globals, treating
unsupported interfaces as no-ops, installing files at runtime, inferring shared-session ownership,
and unconditionally restarting the frontend. Tests cover these CPU policy and declaration boundaries;
GUI/service execution is left to the user under repository policy.

Validation: the capture-enabled library suite (`--no-default-features --features
shell-screencast-linux,shell-xwayland --lib`) passed 1,507 tests with 8 ignored hardware tests.
The public `capture_config_api` and `capture_chooser_api` fixtures passed three tests. Four
configuration tests also passed without default features or capture dependencies. The test compositor
passed both its normal check and the embedded-Xwayland check with the workspace payload. No live
compositor, portal, PipeWire, or Discord qualification was performed.

# Telorgon

Telorgon is a Rust SDK for building desktop applications, embedded interfaces, Linux desktop
shells/compositors, and boot interfaces. It brings UI authoring, rendering, application hosting,
media, system integration, and persistent data together through one public Rust package.

It provides the shared foundation for the desktop, applications, and boot experience in our
[Linux distribution](https://github.com/WIPOperatingSystemName/distro). Applications and shell
hosts can select the SDK capabilities they need and reuse the same components and runtime.

## What the SDK includes

- **UI and input:** retained components, declarative composition, layout, text editing, themes,
  animation, accessibility semantics, focus, gestures, and keyboard/pointer routing.
- **Graphics and hosting:** software and Vulkan renderers, assets and bundled fonts, managed
  application windows, headless rendering, and embedded hosts with caller-owned scheduling.
- **Desktop shells:** Linux Wayland/KMS compositor hosting, shell components, window chrome,
  workspaces, application catalogs, session setup, and process supervision.
- **Media and system services:** Linux PipeWire audio, MIDI and video; desktop audio controls,
  clipboard, battery, brightness, NetworkManager, tray, and portal integration.
- **Data and settings:** typed registries, transactional updates, change notifications, TOML
  persistence, and optional autosave, independent of the graphical host.
- **Boot interfaces:** shared boot-selection and splash components, previews, UEFI hosting,
  and Linux framebuffer splash hosting.
- **Development tools:** profiling, native source/cache helpers, PipeWire and XWayland preparation
  and packaging, and a shader-generation tool.

## Using the SDK

Applications depend on the `telorgon` package. Published releases can be added through Cargo:

```toml
[dependencies]
telorgon = "0.1"
```

This README describes the current repository; published releases may lag its APIs. For source
development, use the SDK crate at `crates/telorgon` in your checkout.

Managed application authoring starts with `telorgon::app::*`, which includes the component macro:

```rust,ignore
use telorgon::app::*;

#[component]
struct Counter {
    #[input]
    label: String,
    #[state]
    count: usize,
}
```

Other entry points include `telorgon::host::embedded`, `telorgon::components::shell`,
`telorgon::shell`, `telorgon::media`, `telorgon::services`, `telorgon::data`, and `telorgon::boot`.
Use the [examples](crates/telorgon/examples) and [Cargo features](crates/telorgon/Cargo.toml)
to find the relevant API and capability selection.

Default features compile managed software and Vulkan application backends and bundled fonts;
application declarations select the renderer at runtime. Embedded, shell, media, and boot
capabilities have separate features and runtime requirements.

## Platforms and native inputs

Managed application backends include Linux and Windows. Linux compositor hosting, PipeWire media,
desktop services, and hardware controls depend on the appropriate features, native libraries,
and available runtime services. A portable API does not imply identical capability support on
every target.

Linux builds with `shell-wayland-linux` require compatible Wayland development XML and
`wayland-protocols` data during compilation. Generated protocol descriptors do not require those
XML files at runtime. Native dependency helpers live in [tools/sdk](tools/sdk), with source pins
and patches under [third_party](third_party) and installation metadata under [packaging](packaging).
Screen-cast portal interoperability remains unqualified.

## Repository

The repository contains three Cargo packages:

- `telorgon` is the public SDK package, organized into focused subsystem modules.
- `telorgon-macros` is the required procedural-macro companion that `telorgon` re-exports. Users do
  not add it directly.
- `telorgon-shader-build` is an unpublished maintainer tool that regenerates the checked-in Vulkan
  shader bundle.

See [the development specification](DEVELOPMENT_SPECIFICATION.md) for the target architecture and
engineering standards. It describes the direction of the SDK; current capabilities are established
by the implementation and relevant tests.

## Shell terminology

Telorgon-owned environment and rendering APIs use `Shell`: `Application::shell_environment`,
`ShellEnvironment`, `LinuxShellConfig`, `ShellKeyEvent`, and `ShellKeyAction`. Cargo features are
`shell-wayland-linux`, `shell-xwayland`, and `shell-xwayland-embedded`. This is a breaking rename;
consumers must update the former `Desktop` names and `desktop-*` feature selections.
Freedesktop `.desktop` application files, `[Desktop Entry]`, and XDG environment keys retain their
standard names.

## License

Telorgon-owned source code, documentation, themes, protocols, tests, and tools in this repository
are licensed under the GNU General Public License, version 3 or any later version, identified by the
SPDX expression `GPL-3.0-or-later`. See [LICENSE](LICENSE).

Third-party dependencies and tools retain their respective licenses.

# Telorgon

Telorgon is a retained, component-based Rust UI framework with software and Vulkan rendering,
managed application hosting, embedded rendering, and shell/compositor building blocks.

Applications depend on one public package and can use the component attribute through its facade:

```toml
[dependencies]
telorgon = "0.1"
```

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

The repository contains three Cargo packages:

- `telorgon` is the single user-facing framework package. Former subsystem crates are preserved as
  focused modules inside this package.
- `telorgon-macros` is the required procedural-macro companion that `telorgon` re-exports. Users do
  not add it directly.
- `telorgon-shader-build` is an unpublished maintainer tool that regenerates the checked-in Vulkan
  shader bundle.

See [the documentation index](docs/README.md) for architecture, implementation status, and
qualification boundaries. Features described as operational are not necessarily
production-qualified.

## Shell terminology

Telorgon-owned environment and rendering APIs use `Shell`: `Application::shell_environment`,
`ShellEnvironment`, `LinuxShellConfig`, `ShellKeyEvent`, and `ShellKeyAction`. Cargo features are
`shell-wayland-linux`, `shell-xwayland`, and `shell-xwayland-embedded`. This is a breaking rename;
consumers must update the former `Desktop` names and `desktop-*` feature selections.
Freedesktop `.desktop` application files, `[Desktop Entry]`, and XDG environment keys retain their
standard names.

## Linux shell build dependencies

Linux builds with `shell-wayland-linux` require compatible Wayland development XML and
`wayland-protocols` data during compilation. Protocol descriptors are generated as Rust tables;
installed applications do not need protocol XML files. Other builds do not require that data.
See [protocol build inputs and overrides](docs/WAYLAND_COMPOSITOR_ARCHITECTURE.md#protocol-source-and-advertisement-rules)
for required versions, custom paths, and cross compilation. Native library dependencies still apply.

## License

Telorgon-owned source code, documentation, themes, protocols, tests, and tools in this repository
are licensed under the GNU General Public License, version 3 or any later version, identified by the
SPDX expression `GPL-3.0-or-later`. See [LICENSE](LICENSE).

Third-party dependencies and tools retain their respective licenses.

# Publishing Telorgon with Cargo

Cargo registry releases are immutable distribution artifacts. Use Git commits and tags to save
recoverable project states; publish to crates.io only for versions intended for downstream users.

No publish command is part of ordinary validation.

The optional Xwayland archive feature is a downstream desktop distribution input,
not a payload bundled with ordinary GUI applications. Cargo does not download or
build Xwayland. Embedded builds require an explicitly supplied validated archive;
the helper build has local evidence, while full notices/SBOM and clean-image release gates remain open.
See [X11 packaging](../packaging/xwayland/README.md) and
[qualification](X11_QUALIFICATION.md) before enabling it in a distributed desktop.

## Registry surface

Telorgon has one user-facing package and one required implementation companion:

| Package | Registry status | Purpose |
| --- | --- | --- |
| `telorgon` | Published | The complete public framework, its subsystem modules, managed and embedded hosts, renderers, assets, tests, and facade |
| `telorgon-macros` | Published | Procedural implementation of `#[component]`, re-exported by `telorgon`; users do not add it directly |
| `telorgon-shader-build` | Never published | Repository-only shader compilation, validation, reflection, manifest, and generated-source tool |

Every former runtime, UI, platform, component, renderer, presenter, profiler, shell, and compositor
crate is now a module of `telorgon`. No other registry package is required.

## Owner decisions required before the first release

1. Confirm that `https://github.com/TempShellEnvNameOrg/telorgon` is the permanent public
   repository URL, then update the workspace metadata and local remote if necessary.
2. Recheck registry package-name availability immediately before release. Availability is not
   reserved until publication.
3. Review the exact packaged file lists, especially generated SPIR-V, profiler web assets, the
   repository README, and GPL metadata.

The repository and all three packages declare `GPL-3.0-or-later`. The complete license text is in
the repository-root `LICENSE` file.

## Validation without publishing

The following commands compile and test but do not upload anything or launch an application,
service, server, or hardware-presenting test:

```powershell
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo check -p telorgon --all-targets --no-default-features
cargo check -p telorgon --all-targets --no-default-features --features application-software
cargo check -p telorgon --all-targets --no-default-features --features application-vulkan-windows
cargo check -p telorgon --all-targets --no-default-features --features embedded-vulkan,embedded-profiler
cargo test --workspace
cargo package -p telorgon-macros --allow-dirty
```

Before `telorgon-macros` exists in the registry, Cargo may be unable to verify the packaged
`telorgon` archive because registry dependency resolution intentionally ignores its local path. That
is not a reason to weaken or remove the exact macro dependency.

## First-release order

1. Inspect and publish `telorgon-macros`.
2. Wait for the registry index to resolve the exact companion version.
3. Package, inspect, dry-run, and publish `telorgon`.

For each published package, inspect before uploading:

```powershell
cargo package -p <package>
cargo publish -p <package> --dry-run
cargo publish -p <package>
```

Do not run the final `cargo publish` command until the release owner explicitly authorizes it.

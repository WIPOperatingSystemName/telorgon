# Telorgon UEFI boot API design

Telorgon should provide `telorgon::boot` for building graphical UEFI boot selectors and
`telorgon::splash` for loading screens shared with a cooperating OS. Authors choose boot targets,
provide a theme or ordinary Telorgon views, and let a firmware host manage input, software
rendering, loading, and transfer of control. Linux, Windows, and custom kernels share the selector
API; continuation of the loading screen is an explicit capability of each target.

Status: proposed API, October 6, 2026. The Rust sketches and Cargo features below require
implementation; they are not available in the current SDK.

## Creating a boot selector

The default path should require no GOP handling, raw framebuffer access, EFI handles, executor,
or application window. This example assumes the three images are installed on the same EFI
System Partition as this launcher. Volume selection is explicit when that is not true.

```rust,ignore
#![no_std]
#![no_main]

use core::time::Duration;
use telorgon::boot::*;

#[boot_entry]
fn main(env: BootEnvironment) -> BootResult<BootReturn> {
    BootApplication::new()
        .target(BootTarget::linux_efi(
            "linux", "Linux",
            EfiImage::on_boot_volume(r"\EFI\Linux\linux.efi")?,
        )?)
        .target(BootTarget::windows(
            "windows", "Windows",
            EfiImage::on_boot_volume(r"\EFI\Microsoft\Boot\bootmgfw.efi")?,
        )?)
        .target(BootTarget::custom_efi(
            "my-os", "My OS",
            EfiImage::on_boot_volume(r"\EFI\MyOS\loader.efi")?,
        )?)
        .default_target("linux")
        .timeout(Duration::from_secs(5))
        .theme(BootTheme::dark())
        .loading_style(LoadingStyle::dots())
        .run(env)
}
```

`#[boot_entry]` supplies the EFI entry ABI, initializes a bounded firmware allocator and panic
handler, creates the non-cloneable `BootEnvironment`, and maps a normal return to firmware status.
It is incompatible with a second entry macro, global allocator, or panic handler; advanced hosts
use the explicit adapter instead. A successful OS launch normally never returns. `BootReturn`
means the user chose to return to firmware, not that the OS finished booting.

Defaults: keyboard navigation, current usable graphics mode, bundled fonts, no automatic launch
unless a timeout is explicitly requested, no persistent writes, no automatic boot of another target after
a failure, and a loading screen limited to launcher-owned work. With no timeout the menu waits.
Graphical failure falls back to a built-in text selector unless graphics are explicitly required.

## What each target can customize

| Target | Selector and launcher loading screen | Loading screen after transfer |
| --- | --- | --- |
| Linux EFI stub or EFI application containing Linux | Full Telorgon software UI | Shared theme and animation through an installed initramfs companion; continuity depends on the display drivers |
| Windows Boot Manager | Full Telorgon software UI before Windows starts | No general Telorgon spinner integration in this design; optional platform integration for a static startup logo |
| Custom kernel with an EFI loader | Full Telorgon software UI | Shared splash renderer when the receiving loader and kernel implement the handoff contract |
| Another EFI application | Full Telorgon software UI | Determined by the receiving application |

`StartImage()` transfers execution to the selected EFI image. The launcher does not keep an
independent rendering loop running afterward. Boot services are unavailable after successful
`ExitBootServices()`; attempted exit can partially shut them down even on failure.
[UEFI image services](https://uefi.org/specs/UEFI/2.10/07_Services_Boot_Services.html?highlight=exitbootservice).

Linux provides an EFI stub entry point and an initrd delivery protocol. The proposed Linux adapter
uses that entry point rather than the deprecated EFI handover protocol.
[Linux PE and COFF entry point](https://docs.kernel.org/arch/x86/boot.html#pe-coff-entry-point).
An OS companion must acquire its own display access; Linux documents an EFI framebuffer driver,
but the selected kernel configuration determines which early display path exists.
[Linux EFI framebuffer](https://docs.kernel.org/fb/efifb.html).

Microsoft documents BGRT as the interface for a static startup logo. Unbranded Boot suppresses
startup UI on supported Enterprise, Education, and IoT Enterprise editions. Neither establishes
an API for an EFI launcher to replace the Windows spinner with an arbitrary Telorgon animation.
The API therefore reports such a request as unsupported.
[Windows boot screen components](https://learn.microsoft.com/en-us/windows-hardware/drivers/bringup/boot-screen-components),
[Windows Unbranded Boot](https://learn.microsoft.com/en-us/windows/configuration/unbranded-boot/).

## Reusing the current SDK

The implementation should reuse existing scene, layout, input, and software rendering contracts.
It should add firmware assembly and presentation, rather than another UI toolkit or rasterizer.

| Existing interface | Intended reuse or prerequisite |
| --- | --- |
| [RenderBackend](crates/telorgon/src/graphics/render/backend.rs) | Keep scene updates and rendering separate from presentation |
| [SoftwareRenderer and SoftwareSurface](crates/telorgon/src/graphics/renderers/software/mod.rs) | Render to the owned RGBA8 backbuffer, then convert dirty regions for GOP |
| [AppRuntimeCore](crates/telorgon/src/host/application/runtime.rs) | Extract reusable view advancement and scene preparation from desktop assembly |
| [MonotonicInstant](crates/telorgon/src/foundation/time.rs) | Accept host-supplied time for selection deadlines and animation |
| [PlatformInput](crates/telorgon/src/host/application/input.rs) | Preserve portable input meaning behind a firmware adapter |
| [TextEngine](crates/telorgon/src/ui/text/shaping.rs) | Use bundled-only fonts and inject text configuration into the runtime |

The existing [embedded host](crates/telorgon/src/host/embedded/mod.rs) requires externally owned
Vulkan resources. It is not a firmware software host. Current [dependencies and features](crates/telorgon/Cargo.toml)
also require `std`; disabling defaults alone does not make Telorgon usable on UEFI.

The required portability work is a `core + alloc` slice of foundation, authoring, UI, text, input,
scene compilation, and the software backend. Desktop services, process helpers, native windowing,
filesystem persistence, and unrelated integrations must be gated out. Host-scoped state must
replace relevant thread-local context and OS-dependent caches. The text engine needs an audited
firmware-compatible shaping path, a fixed locale, bundled fonts, and explicit memory budgets.
Preserve the normal desktop defaults during this extraction.

The first boot profile should support text, images, boxes, layout, selection, and inexpensive
animation. Full software effects need separate work: the current software backend substitutes a
color for liquid glass and Gaussian blur. Report effect capabilities and use theme fallbacks;
reject a screen requiring unavailable effects.

## Subsystem ownership

Follow the [SDK development specification](DEVELOPMENT_SPECIFICATION.md). Keep one curated package
and add directories only when their implementations exist.

| Owner | Responsibility |
| --- | --- |
| `boot` | Target descriptions, selection policy, attempt state, actions, errors, immutable snapshots |
| `splash` | Portable splash design, animation state, progress values, asset identity, handoff encoding |
| `host::boot` | Assemble the runtime, poll preparation work, render views, own one launch attempt |
| `platform::uefi` | EFI entry, scoped allocation, image and filesystem protocols, input, timers, optional variables |
| `graphics::presentation::uefi` | GOP discovery, mode selection, format conversion, presentation, graphics capabilities |
| `host::splash` | Assemble the same portable splash engine with a receiving OS or kernel host |
| `api` | Re-export the curated `telorgon::boot` and `telorgon::splash` authoring surfaces |

Dependencies flow from hosts to portable engines and concrete adapters. Rendering does not depend
on boot selection or EFI. Splash appearance does not load executables. Views request typed actions;
only the boot host may execute them. The selected image owns the actual OS boot protocol.

## Targets and application configuration

```rust,ignore
// These are focused proposed declarations, not complete module listings.
pub type BootResult<T> = core::result::Result<T, BootError>;
pub struct TargetId { /* validated stable string */ }
pub struct BootTarget { /* identity, label, launch recipe, splash request */ }
pub struct EfiFile { /* volume selector and validated absolute EFI path */ }
pub struct EfiImage { /* volume selector and validated absolute EFI path */ }
pub struct InitrdSource { /* file plus required payload authorization */ }
pub struct PayloadAuthorization { /* authenticated manifest or embedded authorization */ }

pub enum VolumeSelector {
    BootVolume,
    PartitionGuid(PartitionGuid),
}

impl EfiImage {
    pub fn on_boot_volume(path: &str) -> BootResult<Self>;
    pub fn on_partition(partition: PartitionGuid, path: &str) -> BootResult<Self>;
}
impl EfiFile {
    pub fn on_boot_volume(path: &str) -> BootResult<Self>;
    pub fn on_partition(partition: PartitionGuid, path: &str) -> BootResult<Self>;
}
impl InitrdSource {
    pub fn authorized(file: EfiFile, trust: PayloadAuthorization) -> BootResult<Self>;
}

impl BootTarget {
    pub fn efi(id: &str, label: &str, image: EfiImage) -> BootResult<Self>;
    pub fn linux_efi(id: &str, label: &str, image: EfiImage) -> BootResult<Self>;
    pub fn windows(id: &str, label: &str, image: EfiImage) -> BootResult<Self>;
    pub fn custom_efi(id: &str, label: &str, image: EfiImage) -> BootResult<Self>;
    pub fn load_options(self, options: LoadOptions) -> BootResult<Self>;
    pub fn linux_initrd(self, initrd: InitrdSource) -> BootResult<Self>;
    pub fn splash(self, request: SplashRequest) -> Self;
    pub fn id(&self) -> &TargetId;
    pub fn icon(self, icon: AssetKey) -> Self;
}

pub struct BootApplication<S = DefaultBootScreens> { /* owned configuration */ }
impl BootApplication<DefaultBootScreens> {
    pub fn new() -> Self;
    pub fn theme(self, theme: BootTheme) -> Self;
    pub fn loading_style(self, style: LoadingStyle) -> Self;
}
impl<S: BootScreens> BootApplication<S> {
    pub fn target(self, target: BootTarget) -> Self;
    pub fn default_target(self, id: &str) -> Self;
    pub fn timeout(self, duration: core::time::Duration) -> Self;
    pub fn screens<T: BootScreens>(self, screens: T) -> BootApplication<T>;
    pub fn assets(self, assets: BootAssets) -> Self;
    pub fn limits(self, limits: BootLimits) -> Self;
    pub fn presentation(self, policy: PresentationPolicy) -> Self;
    pub fn build(self) -> BootResult<ReadyBootApplication<S>>;
    pub fn run(self, env: BootEnvironment) -> BootResult<BootReturn>;
}
```

`run` first performs `build` validation. Reject duplicate IDs, an absent default, no targets,
oversized strings/options/assets, invalid deadlines, and an incompatible target recipe before
claiming graphics. Zero duration means launch immediately after validation and readiness checks;
an unavailable default leaves the menu open with a failure. `TargetId` is independent of list order.

`LoadOptions` has separate constructors for UTF-16 text and opaque bytes, with explicit size limits.
It is not a shell command. Linux initrd support installs a scoped `EFI_LOAD_FILE2` provider using
`LINUX_EFI_INITRD_MEDIA_GUID`. Keep it and the child's options alive for the entire `StartImage`
call, and remove them on a normal return. An EFI Linux application containing its own initrd needs
no external initrd. Preserve that application's signed command-line policy; never assume added
options can override it.
[Linux EFI initrd delivery](https://docs.kernel.org/arch/x86/boot.html#pe-coff-entry-point).

`EfiFile` describes data without implying an executable image. `InitrdSource` binds that file to
authorization from a signed manifest with a configured trust root, or an expected digest embedded
in the launcher's authenticated image. Preparation checks authorization and verifies the content;
the provider serves those same verified bytes, never a later unverified reread. A signed EFI kernel
does not authorize an arbitrary external initrd. Without applicable authorization, preparation
returns `TrustRejected`; the firmware host does not silently weaken this rule.

`BootVolume` means the device carrying the launcher's loaded image, not the first FAT disk.
Partition resolution must reject ambiguity. `EfiImage` names a file; discovery and loading remain
separate, fallible operations. A missing image remains a visible unavailable entry.

Optional discovery is explicit: `TargetCatalog::from_firmware(&mut env)` reads `BootOrder` and
`Boot####` descriptions without changing them. It accepts supported EFI device paths, bounds all
variable parsing, preserves opaque options, and reports unsupported entries. It does not scan
every filesystem or guess installations. Author-supplied targets work without discovery.

Custom kernels should initially supply their own `.efi` loader. It may read an ELF kernel and
perform architecture-specific setup, but that logic is owned by the receiving loader. Direct raw
kernel loading inside Telorgon's launcher is a later, explicit architecture adapter; an arbitrary
function pointer is not a boot recipe.

## Custom selector and loading views

Simple authors use `BootTheme` and `LoadingStyle`. Advanced authors implement a selector view and
optionally override loading and failure views using ordinary Telorgon composition.
The firmware host supplies state and owns navigation,
focus, countdowns, and launch authority.

```rust,ignore
use telorgon::{column, text, ColorRgba8, Element, View};

pub trait BootScreens {
    fn selector(&self, state: &SelectionSnapshot) -> Element;
    fn loading(&self, state: &LoadSnapshot) -> Element {
        default_loading_screen(state)
    }
    fn failure(&self, state: &FailureSnapshot) -> Element {
        default_failure_screen(state)
    }
}

pub enum BootAction {
    Select(TargetId),
    Boot(TargetId),
    BootSelected,
    CancelPreparation,
    DismissFailure,
    ReturnToFirmware,
}

impl BootScreens for MyScreens {
    fn selector(&self, state: &SelectionSnapshot) -> Element {
        column()
            .child(text("Choose an operating system"))
            .child(BootChoices::new(state).item(|target, selected| {
                text(target.label()).color(if selected {
                    ColorRgba8::rgba(100, 180, 255, 255)
                } else {
                    ColorRgba8::rgba(255, 255, 255, 255)
                })
            }))
            .child(BootActionButton::new(BootAction::BootSelected)
                .child(text("Start")))
            .into_element()
    }

    fn loading(&self, state: &LoadSnapshot) -> Element {
        column()
            .child(text(state.target_label()))
            .child(text(state.stage_label()))
            .child(LoadingIndicator::new(state.progress()))
            .into_element()
    }

    fn failure(&self, state: &FailureSnapshot) -> Element {
        column()
            .child(text(state.message()))
            .child(BootActionButton::new(BootAction::DismissFailure)
                .child(text("Back to selection")))
            .into_element()
    }
}
```

`BootChoices`, `BootActionButton`, and `LoadingIndicator` are proposed boot view helpers implementing
the existing `View` contract. The boot host binds their actions to its root component. They do not
pretend that today's component-specific `Button::on_press` directly accepts a boot action.
`BootChoices::item` replaces each choice's appearance while retaining its identity, disabled state,
navigation, and focus behavior. Its proposed `layout(ChoiceLayout)` method supports a vertical
list, horizontal row, or grid with a validated column count and gap. Left/Right navigation applies
to a horizontal row; a grid maps arrow navigation to its actual item positions. The default list
uses Up/Down. Changing appearance never changes which target a choice refers to.
`BootActionButton::new` accepts an action and exposes `child` like
the existing button. Layout can be completely custom; an advanced `BootActionBinding` attaches
the same typed actions to authored hit regions and keyboard bindings.

Helpers copy or retain bounded owned snapshot data when built. They never retain references to a
view callback's borrowed snapshot; the existing `View` contract requires `'static` ownership.

Snapshots contain an attempt ID and revision. Action bindings retain their originating snapshot;
the host rejects stale activation from a prior attempt or catalog revision. A view callback cannot
perform I/O, enter firmware, or recursively launch a target. Accepted actions are applied after
the current UI dispatch turn. Work queues and retained snapshots have explicit bounds.

`SelectionSnapshot` includes entries, availability, selected ID, focused ID, remaining timeout,
and graphics/input capabilities. Any accepted user interaction cancels automatic launch; only an
explicit policy can restart its timeout. Enter starts the selected available target, Escape
cancels preparation or dismisses a failure, and arrow keys navigate the configured layout. Returning to firmware
has its own action. A failed attempt always disables automatic launch until explicitly restarted.

Firmware keyboard protocols do not provide desktop key-release semantics universally. The adapter
must expose activation pulses or bounded synthetic press/release pairs without inventing held-key
state. Pointer and touch are optional capabilities; all default actions remain keyboard reachable.

## Visual pickers and themed menus

Custom authoring covers the entire screen: background, typography, icons, placement, target
controls, focus and hover styling, transition animation, loading artwork, and error presentation.
The API must make both a graphical disk picker and a game-style title menu ordinary boot
applications. `BootTheme` provides a convenient default; `BootScreens` supplies complete views.

| Interface | Composition and interaction |
| --- | --- |
| Boot Camp style disk picker | Centered horizontal disk or OS cards, target icons and labels, a selected-card highlight, Left/Right navigation, Enter or a visible start control |
| Minecraft style title menu | Author-supplied background and pixel font, a title, vertically stacked textured buttons such as Boot Linux and Boot My OS, selected/hovered button styling |
| Custom loading screen | Shared `SplashDesign` with a logo, spinner, animated character, or a stylized progress display, driven by actual status from the current host |

For a disk picker, `BootChoices::new(state).layout(ChoiceLayout::Row { gap })` supplies navigation and
selection while its `item` callback draws each card. `BootTarget::icon` attaches a prepared
asset key; the target snapshot exposes that optional key. Missing optional artwork uses the
theme's fallback icon. Asset admission occurs before rendering, through the existing Telorgon
resource path adapted for the boot host.

For title-menu buttons, expose direct activation alongside selection. `BootAction::Boot(id)`
asks the host to select and launch one available target in a single validated action. It uses
the same preparation, cancellation, authentication, and failure path as `BootSelected`. A button
does not load an executable inside a view callback. Authors can arrange these controls without
using `BootChoices`:

```rust,ignore
// Inside BootScreens::selector; both IDs are validated and retained by the screen.
column()
    .gap(12.0)
    .child(text("MY BOOT MENU"))
    .child(BootActionButton::new(BootAction::Boot(self.linux.clone()))
        .child(text("Boot Linux")))
    .child(BootActionButton::new(BootAction::Boot(self.custom_os.clone()))
        .child(text("Boot My OS")))
    .child(BootActionButton::new(BootAction::ReturnToFirmware)
        .child(text("Return to firmware")))
    .into_element()
```

`BootActionButton` exposes the existing button's portable appearance and interaction styling,
including background, borders, dimensions, focus indication, and hover/press effects. Custom
fonts, textures, and images are prepared in `BootAssets`. A game-like screen uses the software
backend's supported effects; a live 3D panorama requires a separately supported renderer or
bounded prerendered frames. Frame animation and larger backgrounds count toward `BootLimits`.

Selecting a card changes selection by default; it does not immediately launch. Direct menu
buttons explicitly use `Boot(id)` when that is the intended interaction.

The same authored splash can be reused by both the UEFI loader and a receiving OS:

```rust,ignore
fn loading(&self, state: &LoadSnapshot) -> Element {
    self.splash.view(state.splash())
}

// In the Linux initramfs companion or custom kernel's own rendering host.
let splash = SplashSession::new(MySplash, prepared_assets)?;
```

The companion compiles `MySplash` from the same author-owned library and supplies its own clock,
progress, and display adapter. This carries custom appearance into OS startup without leaving
firmware drawing callbacks active. Linux continuation starts when the configured early userspace
display is available; a custom kernel can adopt the framebuffer and start its own renderer sooner.
An animation does not imply a percentage: use an indeterminate indicator until the current host
has measurable work.

Provide `ReadyBootApplication::preview(PreviewScenario)` behind `boot-preview`. It consumes the
same configured application and opens an ordinary desktop preview with simulated targets and
status. Scenarios cover selection, success transitions, missing targets, failure, absent pointer
input, and OS continuation. It presents the actual software-rendered views, so authors can iterate
on either interface without rebuilding and booting a firmware image for every visual change.
Firmware resolution, timing, and driver transitions still require separate qualification.
Keep screen code and configuration in a shared author library, with separate small UEFI and
desktop entry binaries; the firmware binary's `no_std` entry setup does not run inside the desktop
preview. The desktop entry calls `configure_boot_ui()?.build()?.preview(scenario)` using the shared
configuration function and an explicitly constructed `PreviewScenario`.

## Loading progress and launch lifecycle

```rust,ignore
pub enum LoadStage { Resolve, Read, Verify, Prepare, Transfer }
pub enum Progress {
    Indeterminate,
    Bytes { completed: u64, total: core::num::NonZeroU64 },
    Fraction(ProgressFraction), // Validated 0..=10_000 basis points.
}
pub struct LoadSnapshot { /* attempt, target, stage, progress, cancellation */ }
pub enum BootReturn { ReturnToFirmware }
```

Progress describes the current named operation. Byte progress applies only when the host performs
measurable reads; it is not overall OS startup progress. Firmware image loading is synchronous
and supplies no generic percentage callback. Render an indeterminate final frame before blocking
firmware calls, and resume animation only if control returns. Do not draw from timer callbacks.

The owned lifecycle is:

```text
Ready -> Selecting -> Preparing -> Transferring
            ^             |              |
            |         Cancel/Failure     +-> receiving EFI loader and OS
            +-------------+              +-> normal return -> failure or selection
```

`BootEnvironment` owns firmware access. `ReadyBootApplication` owns configuration, assets, UI state,
and one preparation at a time. Internally, non-cloneable `PreparedEfiImage` owns the loaded image,
options, and temporary protocols. `QuiescedUi` consumes live UI scheduling and finalizes the visible
frame before launch. Raw firmware lifetimes do not escape into view snapshots.

Preparation validates capabilities, resolves the target, loads the image, installs any scoped
providers, and creates the final splash state. Chunked host work yields to input and frame
deadlines. Synchronous firmware calls cannot guarantee responsive animation or cancellation;
`can_cancel` reflects that. A cancelled preparation unloads its unstarted image and returns to
selection. It never leaves a child protocol pointing into freed memory.

The launcher calls `LoadImage`/`StartImage` while boot services are active. Linux's EFI stub,
Windows Boot Manager, or the custom receiving loader owns `ExitBootServices`; the launcher must
not call it first. Before `StartImage`, pause input and graphics timers and release unnecessary
protocol borrows. Keep only the launch resources and optional handoff payload needed by the child.

On a compliant normal child return with boot services still active, remove temporary protocols,
release launch resources, re-query graphics state, and recreate presentation if necessary.
Retained selection state seeds a fresh UI runtime; explicitly rearm its input and timers.
Return status is not proof that an OS booted or that no failed exit attempt occurred. Restoring
the menu requires a receiving image that returns without attempting exit. A preallocated exit
notification marker forbids rendering and firmware-backed cleanup once an exit transition is
observed; it cannot prove every third-party failed attempt was detected. Custom receiver helpers
make exit attempts terminal, with no menu return, UI work, or ordinary destruction during retry.
No user callback runs between final memory-map capture and exit.
[UEFI image execution and exit](https://uefi.org/specs/UEFI/2.10/07_Services_Boot_Services.html?highlight=exitbootservice).

The firmware event loop is single-threaded and deadline driven. Input and timers wake the host;
callbacks only mark work ready. The host advances the view, compiles scene changes, renders, then
presents. Animation advances from supplied monotonic time. Slow frames skip intermediate animation
steps rather than accumulating a frame queue. The host explicitly controls the firmware watchdog
for menu waiting and restores the appropriate launch policy before starting a child.

## Continuing a splash inside an OS

Appearance and ownership are separate contracts:

```rust,ignore
pub struct SplashRequest {
    pub theme: SplashThemeId,
    pub continuation: SplashContinuation,
    pub requirement: CapabilityRequirement,
}
pub enum SplashContinuation {
    LauncherOnly,
    LeaveLastFrame,
    LinuxInitramfs(LinuxSplashContract),
    CustomKernel(CustomSplashContract),
}
pub enum CapabilityRequirement { Required, BestEffort }
pub enum SplashSupport {
    LauncherOnly,
    StaticFrameBestEffort,
    ReceiverConfigured { protocol_version: u16 },
}

pub trait SplashDesign {
    fn view(&self, state: &SplashSnapshot) -> Element;
}
pub struct SplashSession<D: SplashDesign> { /* owns animation and view state */ }
impl<D: SplashDesign> SplashSession<D> {
    pub fn new(design: D, assets: SplashAssets) -> SplashResult<Self>;
    pub fn resume(design: D, assets: SplashAssets,
                  checkpoint: SplashCheckpoint) -> SplashResult<Self>;
    pub fn update(&mut self, status: SplashStatus) -> SplashResult<()>;
    pub fn advance(&mut self, now: MonotonicInstant) -> SplashResult<SplashFrame>;
    pub fn checkpoint(&self) -> SplashCheckpoint;
    pub fn finish(self) -> SplashCompletion;
}
```

`SplashFrame` is a prepared scene update consumed by a host's renderer; it owns no firmware target.
Each environment creates its own `SplashSession` and presentation owner. `SplashCheckpoint`
contains bounded serializable appearance state, such as animation phase and theme identity,
rather than a live scene or allocator. A receiver may restart a spinner when clock continuity is
unavailable. Status includes a stage label, optional progress, and failure/completion; only the
current owning environment publishes it. OS startup milestones require explicit OS events.

`SplashDesign` is ordinary portable Telorgon view code, compiled into each cooperating binary;
it is not a Rust object serialized across the handoff. Share that code in an author-owned library
along with its theme registration. `LoadSnapshot::splash()` exposes the corresponding
`SplashSnapshot`, so `BootScreens::loading` can delegate to the exact same design's `view` method
that the initramfs or custom kernel uses. Built-in dots and progress bars use this same path.
The receiving binary resolves `SplashThemeId` through its own registered designs and rejects an
unknown required theme.

`SplashSnapshot` exposes the logical viewport, elapsed animation time, normalized animation
phase, motion preference, and current status. Shared view code derives its visual state from those
values. Each host binds theme IDs to compiled designs explicitly; IDs never load executable code
from a theme bundle or imply that a live closure can cross the handoff.

Capability negotiation distinguishes observed graphics support from a configured receiver
contract. `ReceiverConfigured` is not proof that the OS companion actually started. A required
contract must match a versioned manifest bound to the selected image and asset bundle; a missing
or incompatible manifest fails before launch. Best effort records the resolved fallback in the
snapshot. Neither setting promises uninterrupted pixels during a driver mode switch.

For Linux, package the same design and asset bundle into the initramfs. The companion starts at
the earliest configured display availability, renders through an OS-owned framebuffer or DRM
adapter, receives boot milestones locally, and stops when the graphical session claims the output.
A Linux-specific target recipe selects a declared theme through agreed command-line options
when the target accepts them; a signed embedded configuration may select it instead. Default Linux
support transfers theme identity, not an arbitrary EFI configuration table assumed readable from
userspace. Exact animation-phase preservation is optional and needs a separate documented kernel
transport. Kernel console output and driver transitions may interrupt the image.

For custom kernels, provide `splash::handoff::v1` and a receiver helper. The EFI loader reads a
Telorgon-specific configuration table while services are active, copies or reserves its payload,
then supplies it through its own documented kernel entry ABI. The launcher does not create a
generic kernel stack, page table, or entry convention.

Each attempt publishes a fresh payload. Installing the table preserves any prior entry; on a
compliant child return, restore that entry before freeing the payload. No table may retain a
reference to a freed prior attempt. Terminal exit transfers payload preservation to the receiver.

| Handoff field | Contract |
| --- | --- |
| Header | Magic, major/minor version, header size, total length, required feature flags |
| Target and assets | Stable target identity, theme identity, asset bundle digest |
| Display | Optional physical address, mapped size, width, height, stride, pixel size and channel masks |
| Appearance | Bounded checkpoint and optional final-image description |
| Payloads | Explicit physical spans, lengths, alignment, encoding, and digest for each asset payload |

Use fixed-width integers, a specified byte order, and offsets for variable records; Rust struct
layout is not the wire ABI. A checksum detects damage, not trust. Unknown required flags or major
versions fail validation. The receiver checks arithmetic, overlaps, bounds, asset digests, and
display suitability before mapping anything. Physical addresses are descriptions, not safe Rust
references. The receiving kernel must reserve or copy payload allocations before reclaiming
loader memory. It must establish its own device mappings and memory attributes.

No handles, protocol pointers, function callbacks, heap ownership, or firmware event loop crosses
the handoff. A framebuffer is optional. Without a transferable framebuffer, a required custom
early framebuffer splash fails before launch; best effort waits for the kernel's display driver.
An EFI child may continue its own splash before exiting services, but it must freeze that renderer
before its exit attempt and resume only through kernel-owned facilities afterward.

For Windows, keep `SplashContinuation::LauncherOnly` or best-effort last-frame behavior. A future
`WindowsStartupLogo` platform integration may prepare BGRT-compatible static artwork only when
the platform explicitly supports installation. It is separate from the shared animation API and
disabled by default. Never rewrite Windows binaries, BCD, ACPI tables, or Secure Boot policy as
an incidental consequence of applying a Telorgon theme.

## Firmware graphics and resources

The GOP presenter converts Telorgon's RGBA8 backbuffer to RGB-reserved, BGR-reserved, or validated
bitmask pixels using actual `PixelsPerScanLine` stride. Reserved channels are not alpha. It uses
firmware `Blt` for `PixelBltOnly`, which has no transferable linear framebuffer. Select a mode
before drawing the splash; `SetMode` clears the visible screen.
[UEFI graphics output protocol](https://uefi.org/specs/UEFI/2.10/12_Protocols_Console_Support.html).

Validate geometry, stride, framebuffer length, masks, and all offset arithmetic. Use one bounded
RGBA8 surface initially; do not reinterpret it as GOP memory. Render with `TargetLoad::Clear`,
which the current software backend supports, and copy only reported damage when valid. Software
rendering does not imply tear-free or synchronized presentation. `BootCapabilities` reports
graphics availability, direct framebuffer transfer, pointer input, supported effects, and limits.

`BootLimits` bounds heap use, surface size, asset bytes, text/glyph caches, target count, option
length, and pending actions. Resolution selection must account for the framebuffer budget.
`PresentationPolicy` chooses graphical-required, graphics-with-text-fallback, or text-only behavior.
Boot targets and errors remain usable if the custom graphical view or font setup fails.

Reserve text fallback resources before graphical setup. Recoverable SDK budget and initialization
errors use that reserve. A hard allocator ceiling does not make arbitrary author callbacks
recoverable: infallible allocation or panic can terminate the launcher. The entry panic path
emits bounded diagnostics only while firmware access remains valid; it never promises a menu
after terminal exit. Use fallible reservation at SDK-owned allocation boundaries.

`BootAssets` uses an embedded, versioned bundle by default. Host build tooling prepares images,
fonts, and optional vector rasterizations; runtime frame callbacks do not read files or decode
assets. Complex scripts require the audited shaping support and fonts, not silent replacement
with ASCII. External theme files are explicitly loaded and bounded before entering selection.
Shared splash assets should be embedded in authenticated images or covered by an authenticated
manifest; a digest alone does not authenticate a theme.

## Security and failure contracts

EFI execution uses firmware image validation. Respect `EFI_SECURITY_VIOLATION` and
`EFI_ACCESS_DENIED`, including a rejected image for which firmware returned a handle; never start
that handle. Firmware trust of an EFI loader does not itself authenticate raw kernels, initrds,
or external assets loaded separately. Their loader must enforce the required trust policy.
[UEFI image validation](https://uefi.org/specs/UEFI/2.10/07_Services_Boot_Services.html?highlight=exitbootservice).

The firmware host enforces `InitrdSource` authorization before serving an external initrd. Under
Secure Boot, continuation bundles likewise require an authenticated manifest or trusted embedded
configuration. A custom loader must enforce equivalent authentication for its raw kernel/modules.

Public errors distinguish configuration, unavailable target, ambiguous volume, unsupported image
architecture, malformed options, I/O, trust rejection, missing continuation capability, resource
limit, and firmware failure. Retain a stage and optional firmware status for diagnosis. Failure
screens show a safe message and the target; detailed logs use an explicitly configured console.

`build` errors have no boot side effects. Discovery reports entry availability separately from
execution failure. Preparation failures return to selection only while boot services remain
active. There is no `Result` promising menu recovery after attempted `ExitBootServices`.
Do not automatically disable validation, change targets, or retry indefinitely.

Selection and themes are read-only by default. Optional remembered selection is a separate
explicit persistence policy, with bounded writes to an application-owned variable and visible
write failure. It never overwrites `BootOrder` or `BootNext`. Installation, firmware registration,
signing, and Windows branding are separate operations outside `run`.

## Build profiles and implementation sequence

The proposed firmware dependency declaration is:

```toml
[dependencies]
telorgon = { version = "0.1", default-features = false, features = ["boot-uefi", "font-inter"] }
```

`boot-uefi` includes the portable boot UI and software renderer plus UEFI assembly. `splash-core`
enables the portable receiving splash engine without firmware or desktop hosts. `boot-preview`
provides a desktop host for the same screens with simulated targets, progress, capabilities, and
failures; it never launches real images. These features are proposed compatibility commitments.
Reject incompatible firmware/native combinations rather than silently linking desktop services.
Initially qualify `x86_64-unknown-uefi`; other architectures need explicit adapter qualification.

Implement in this order:

1. Isolate and compile the portable software UI slice, including injected text configuration and
   bounded resources. Preserve existing desktop interfaces.
2. Implement portable target selection, snapshot/action rules, default screens, and desktop preview.
3. Implement the UEFI entry, input, GOP, filesystem, and image adapters with recoverable failures.
4. Add Linux EFI initrd delivery and the shared splash assets/renderer in an initramfs companion.
5. Version the custom-kernel handoff and qualify a receiver. Treat Windows static logo support as
   a separate platform integration when there is a concrete supported platform.

Implementation checks should establish the contracts: portable target compilation without native
dependencies; stride/channel conversion and bounds; stale action rejection; cancellation cleanup;
real byte progress versus indeterminate phases; capability failure/fallback; and handoff parser
validation. Desktop preview checks visual composition. User-run OVMF and hardware qualification
checks actual Linux, Windows, and custom-loader launch, Secure Boot rejection, returning EFI
applications, GOP modes, and display ownership transitions. Compilation does not establish splash
continuity or real firmware behavior.

# Telorgon SDK Architecture and Engineering Standards

## Status and scope

This specification defines the proposed framework for organizing the SDK, designing reusable code,
and keeping future development straightforward. It records the agreed general standards and target
architecture; it does not claim that the current repository already conforms.

Adoption is incremental. This specification does not require an immediate refactor or complete
documentation of existing code. Establish current behavior from the implementation, relevant tests,
and observed results. Existing code contracts must be reconciled deliberately as changes are adopted.

## 1. One cohesive SDK

Telorgon should offer:

- One curated programmer-facing Rust package.
- Reusable internals for applications, embedded interfaces, and Linux shells.
- Explicit platform and capability profiles.
- Managed native dependencies and unified build tooling.
- Selectable default graphical and sound resources.

“One SDK” means a coherent development experience. It does not mean every build includes every
backend, service, or resource.

## 2. Repository organization

Separate implementation, third-party inputs, build tooling, resources, and documentation.

```text
telorgon/
  crates/
    telorgon/
      src/
        lib.rs
        api/
        foundation/
        data/
        authoring/
        runtime/
        ui/
        input/
        components/
          application/
          shell/
        shell/
        services/
        media/
        assets/
        theme/
        graphics/
          scene/
          render/
          renderers/
            vulkan/
              shaders/
                source/
                generated/
          presentation/
          bridges/
        integrations/
          wayland/
          x11/
          pipewire/
          portals/
        platform/
          contracts/
          linux/
          windows/
          macos/
        host/
          application/
          embedded/
          linux_shell/
      tests/
      benches/

    telorgon-macros/
    telorgon-shader-build/

  tools/
    sdk/
    assets/

  sdk/
    profiles/
    toolchains/

  third_party/
    manifest.toml
    sources.lock.toml
    recipes/
    patches/
    licenses/

  protocols/                   Telorgon-authored protocol definitions, when needed
  resources/
  packaging/                   Installation metadata and distribution assembly rules
  examples/
  vendor/
  target/
  dist/
```

This is a conceptual layout. Introduce directories when they have real responsibilities and content.
It does not imply a separate crate for every directory.

## 3. Public API separated from implementation

The public API reflects how programmers use Telorgon. Internal organization reflects how Telorgon
works.

- Curate public exports through deliberate entry points.
- Keep implementation modules private by default.
- Define public types beside their owning subsystem and re-export them where appropriate.
- Avoid duplicate forwarding layers when a re-export is sufficient.
- Keep backend details out of ordinary public signatures.
- Provide explicit advanced extension interfaces.
- Preserve public import paths during internal reorganization where practical.

Public types, Cargo features, configuration formats, and resource identifiers are all compatibility
commitments.

## 4. Clear subsystem ownership

Every subsystem must have an identifiable:

- Purpose and scope.
- Owner of authoritative state.
- Set of permitted dependencies.
- Interface to other subsystems.
- Resource lifecycle.
- Failure and recovery model.

Other subsystems interact through focused requests, results, events, or snapshots. Hosts coordinate
subsystems rather than accumulating their implementation details.

## 5. Directed dependencies

Portable behavior depends on narrow contracts. Concrete adapters implement those contracts. Hosts
assemble implementations.

Rules include:

- No subsystem dependency cycles.
- No OS handles or protocol bindings in portable UI and shell models.
- No renderer dependency on application-host orchestration.
- No unrestricted object exposing all framework state and services.
- No generic shared module used to conceal misplaced responsibilities.
- Internal code uses subsystem interfaces rather than depending on the user-facing facade.

Use Rust visibility and eventual architecture checks to reinforce these boundaries.

## 6. Reuse across entry points

Managed applications, Linux compositors, embedded interfaces, and headless execution should reuse
engines wherever their semantics match.

| Entry point | Supplies |
| --- | --- |
| Managed application | Windows, event-loop integration, platform services |
| Linux compositor | Device access, client management, display and session integration |
| Embedded interface | Externally owned scheduling, input, rendering targets and resources |
| Headless execution | Explicit advancement, synthetic inputs, offscreen execution |

Reusable engines must not inherently require ownership of the process, event loop, or graphics
device.

Future entry points should primarily add assembly and adapters rather than duplicate subsystems.

## 7. Policy separated from mechanism

Keep decisions separate from execution:

- Shell policy chooses window placement; protocol integration communicates it.
- Routing selects input destinations; device adapters acquire events.
- Capture policy authorizes access; graphics and media machinery deliver frames.
- System actions express intent; services perform the operation.

Platform adapters should not accumulate product policy.

## 8. Platform and capability isolation

Separate portable contracts from Windows, Linux, macOS, and other implementations.

- Features determine which implementations are compiled.
- Runtime configuration selects and initializes available implementations.
- Enabling a feature must not automatically start services or claim devices.
- Build scripts, generated bindings, and dependencies follow the same isolation.
- Shared algorithms contain minimal platform-specific branching.
- Unsupported configurations fail clearly.

Check actual capabilities rather than assuming availability from an OS name. Define required
capabilities, fallbacks, unsupported results, and behavior when capabilities disappear.

## 9. Protocol and integration organization

Give each technology a clear integration boundary.

Distinguish client, server, provider, producer/consumer, and process-supervision roles where relevant.

Examples:

- Wayland application clients and compositor servers have separate responsibilities.
- XWayland process supervision differs from X11 window management.
- PipeWire producers and consumers normally connect to an existing server.
- Portal clients request capabilities; portal backends provide them.

Shared transport infrastructure may be reused, but unrelated services retain separate ownership.
Avoid a universal protocol abstraction without a concrete need.

## 10. First-class input architecture

Input is a portable subsystem covering device identities, capabilities, events, state,
interpretation, and routing.

```text
Platform acquisition
        ↓
Portable events and state
        ↓
Routing and shell policy
        ├── Telorgon UI
        └── External client delivery
```

Rules:

- Device acquisition does not choose the destination window.
- Routing does not depend on native device handles.
- Focus, grabs, and pressed-state ownership are explicit.
- Widget behavior remains separate from global shell policy.
- Preserve device identity, timestamps, and coordinate-space meaning.
- Handle removal and session loss without stuck interactions.
- Give synthetic input an explicit entry point.

Controllers, touch, tablets, and future devices retain meaningful capabilities rather than being
forced into keyboard-and-mouse abstractions.

## 11. System actions and hardware controls

Introduce typed system actions between input bindings and capability services.

Supported action families may include:

- Speaker volume and mute.
- Microphone mute.
- Display brightness.
- Keyboard backlighting.
- Screenshots and recording.
- Camera privacy.
- Media playback controls.

```text
Key, widget, command, or accessibility request
                    ↓
Typed action and applicable policy
                    ↓
Owning service and platform adapter
                    ↓
Observed state and feedback
```

Rules:

- Input adapters do not directly change system settings.
- Different callers reuse the same service operations.
- Services coordinate existing implementations rather than duplicate media or capture machinery.
- Each action defines its target, repeat behavior, permission requirements, and availability.
- Firmware-handled operations must not be applied twice.
- Feedback reflects confirmed state or clearly pending work.
- Hardware-enforced privacy restrictions remain authoritative.
- Ordinary applications do not automatically receive compositor privileges.

## 12. Explicit ownership, lifecycle, and concurrency

Every connection, task, device, stream, and graphics resource needs defined creation, ownership,
cancellation, and destruction.

Specify:

- Owned versus borrowed resources.
- Thread affinity and callback context.
- Blocking and reentrancy behavior.
- Ordering guarantees.
- Queue bounds and overload behavior.
- Disconnect and shutdown handling.
- Treatment of stale asynchronous results.

Distinguish accepting a request from completing it. Shared ownership through `Arc` does not remove
the need for a shutdown owner.

## 13. Rust design conventions

Use Rust’s type system to express important contracts.

- Newtypes distinguish identifiers, units, and coordinate spaces.
- Validated constructors preserve invariants.
- Enums represent meaningful alternatives and lifecycle states.
- Public error types expose actionable failure categories.
- Traits represent real implementation or extension boundaries.
- Sealed traits are appropriate when external implementation is intentionally restricted.
- Builders serve genuinely complex configuration.
- `#[non_exhaustive]` and `#[must_use]` are used deliberately.
- Typestate is reserved for transitions where it materially improves correctness.

Avoid abstraction for its own sake. Similar code is not automatically evidence of shared semantics.

## 14. Unsafe and native boundaries

Keep unsafe operations concentrated in focused integration modules.

- Define the safety contract.
- Explain the invariants justifying unsafe operations.
- Wrap native resources with clear ownership and cleanup.
- Expose safe interfaces where invariants can be enforced.
- Provide explicit completion or shutdown when destruction alone cannot express the lifecycle.
- Expose raw native handles only through intentional interoperability contracts.

## 15. Cohesive files and size limits

Handwritten source files should remain below 1,000 lines.

| Size | Expectation |
| --- | --- |
| Below 500 lines | Preferred working range where natural |
| 500–799 lines | Review accumulating responsibilities |
| 800–999 lines | Review before substantial additions |
| 1,000+ lines | Restructure or document an exception |

Split by behavior, ownership, lifecycle, or interface—not arbitrary numbered sections.

Keep entry-point files small. Avoid vague helper collections and large state objects spread across
many files with unrestricted access.

## 16. Narrow file-size exceptions

Exceptions are justified when splitting a cohesive definition would reduce clarity, weaken
invariants, or introduce artificial indirection.

Possible examples include exhaustive enums, standardized tables, and tightly coupled
implementations.

Record:

- The qualifying file and content.
- Why it is cohesive.
- Why splitting would hurt.
- The permitted scope.
- The review trigger.

An exception permits length, not unrelated responsibilities. Generated output receives a separate
classification.

## 17. Managed dependencies and reproducible builds

Distinguish:

| Dependency category | Treatment |
| --- | --- |
| Rust packages | Cargo manifests, locks, optional vendoring |
| Native build inputs | Pinned sources, headers, libraries, protocol data and recipes |
| Runtime helpers | Versioned, validated payloads |
| Host facilities | Explicit runtime and compatibility requirements |

Record versions, source identities, patches, licenses, targets, build tools, and linking or
packaging policies.

Separate build-machine tools from target libraries. Use private build directories and explicit
discovery paths instead of arbitrary sibling directories.

## 18. Unified SDK tooling

A supported SDK command interface should prepare dependencies, select profiles, invoke Cargo,
verify outputs, and package distributions.

- Preparation occurs before dependency build scripts need native inputs.
- Cargo build scripts focus on local generation and linking.
- Optional profiles avoid unnecessary dependencies.
- Release environments are pinned as well as source inputs.
- Offline builds work once required inputs are present.
- Offline SDK distributions include dependency sources and necessary build inputs.

A unified workflow does not eliminate legitimate host SDK, driver, or runtime requirements.

## 19. Default graphical and sound resources

Store first-party assets under `resources/`, separate from Rust implementation and third-party
build inputs.

- Use stable semantic identifiers.
- Support application overrides, selected packs, and SDK defaults.
- Scope resource selection to the relevant host or application.
- Support selective embedded or external packaging.
- Record provenance, licenses, and required metadata.
- Generate runtime variants reproducibly.
- Avoid decoding and file access in latency-sensitive callbacks.

Resources supply content; themes select presentation; components consume references; the asset
subsystem handles loading and lifetimes.

## 20. Documentation-ready architecture

Define the documentation standard now; complete documentation later.

Code must make its structure easy to explain:

- Each module has one explainable purpose.
- Nesting communicates ownership and responsibility.
- Names reveal roles.
- Interfaces expose meaningful inputs, outputs, constraints, and ownership.
- Dependencies and initialization are traceable.
- Each concept has a canonical home.
- Entry points provide a clear reading path.
- Types make lifecycle and failure behavior explicit.

Future documentation follows consistent conventions for purpose, usage, ownership, dependencies,
lifecycle, errors, and related modules. Rustdoc should present a coherent public view and a useful
internal view.

Documentation completeness requirements are introduced gradually. Documentation should describe a
clear design rather than compensate for an unclear one.

Do not recreate the removed documentation directory or generate a replacement documentation tree.
Keep general standards in this root specification and agent guidance; use source comments and
rustdoc for local contracts. Add standalone documents only when explicitly requested.

## 21. Verification and enforcement

Use appropriate checks for:

- Public API usability.
- State transitions and resource lifetimes.
- Backend contracts.
- Feature and target isolation.
- Dependency boundaries.
- File sizes and exceptions.
- Resource catalogs.
- Native dependency preparation and packaging.
- Documentation links and examples as documentation is added.

Keep compilation, deterministic testing, live interoperability, hardware qualification, and
reproducibility evidence distinct.

## 22. Incremental adoption

Implement changes in their owning source repository. Generated build snapshots
and temporary experiment directories are outputs; they cannot be the only copy
of a fix. When Telorgon is developed inside Custom Distro, use the distro Python
pipeline to snapshot edited source, build versioned packages and install them
through guest pacman in a private development VM. Rebuild affected applications
for framework changes and use release builds for rendering measurements. Record
the actual source/package identities and interaction results; compilation and
installation alone do not qualify desktop behavior.

Start with an inventory of current modules, interfaces, dependencies, and violations.

Establish a baseline, prevent new violations, and migrate one subsystem at a time. Separate
structural and behavioral changes where practical.

Preserve compatibility deliberately, reconcile existing architectural contracts, and remove
temporary bridges when migrations finish.

## 23. Shader source and generated artifacts

Shader source is renderer implementation code. Keep it beside the backend that owns it, separate
from default graphical assets and from the tool that compiles it.

Illustrative target layout:

```text
src/graphics/renderers/vulkan/
  shaders/
    source/
      common/          Shared shader functions and declarations
      ui/              UI rendering
      effects/         Blur, shadows, glass
      composition/     Surface composition
    generated/
      spirv/           Compiled shader binaries
      metadata.rs      Generated bindings and layout metadata
      manifest.json    Source and build identities
```

The separate `telorgon-shader-build` tool compiles and validates these sources; it does not own them.
Under the existing module layout, the corresponding source location is
`crates/telorgon/src/renderer_vulkan/shaders/source/`. Adoption does not require a broader graphics
directory migration first.

- Group shader sources by rendering responsibility.
- Keep authored sources distinct from generated artifacts; do not manually edit generated output.
- Keep Rust/shader interface definitions and their validation closely connected.
- Share source across backends only when they actually consume the same source.
- Make compilation and artifact generation reproducible through SDK tooling.
- Retain generated release artifacts where needed so downstream users do not require shader
  compilers. Keep those artifacts synchronized with their source and build inputs.
- Apply source modularity and file-size rules to handwritten shaders, with the same narrow exception
  policy used for other implementation code.

This section defines organization rules; it does not itself move sources or generated artifacts.

## 24. Protocol definitions and packaging boundaries

Keep protocol implementation, authored definitions, upstream inputs, and distribution assembly in
distinct locations according to ownership.

| Content | Location |
| --- | --- |
| Protocol handlers, bindings, and translation code | `src/integrations/<technology>/` |
| Telorgon-authored protocol definitions | `protocols/<technology>/` |
| Supported protocol selections and generation settings | Beside the owning integration |
| Upstream protocol sources | Managed through `third_party/` |
| Install metadata, launchers, service files, and portal registration | `packaging/` |
| Native dependency compilation recipes | `third_party/recipes/` |
| Build and packaging orchestration | `tools/sdk/` |
| Produced archives and installers | `dist/` |

The root `protocols/` directory is reserved for definitions Telorgon authors and maintains. It is
not a general collection of downloaded protocol specifications or runtime implementation code.
Create it only when custom definitions exist. Protocol-selection profiles belong beside their
integration. Upstream definitions follow the dependency source and versioning rules; generated
bindings remain distinct from handwritten code and use the appropriate build output location or
deliberately shipped generated-artifact location.

Keep `packaging/` at the repository root because distribution assembly can span several subsystems.
Organize it by deployment target and responsibility, introducing directories only when needed:

```text
packaging/
  linux/
    session/          Session registration and launcher metadata
    portals/          Portal registration and configuration
    xwayland/         Runtime payload composition and staging rules
  windows/
  macos/
  sdk/                SDK distribution manifests
```

For dependencies such as XWayland, compilation recipes belong in `third_party/recipes/`; rules
describing which binaries, libraries, data, and metadata ship belong in `packaging/`. The SDK tool
coordinates these responsibilities without duplicating their recipes. Produced payloads and
installers belong in `dist/`, not among authored packaging inputs.

These are target ownership rules. Relocate existing files only when a migration is in scope, and
update their consumers as part of that migration.

## 25. Registered data, live settings, and persistence

The `data` subsystem owns typed registered storage, transactional replacement, change
notifications, serialization, file persistence, and opt-in autosave. Its public entry point is
`telorgon::data`; it does not depend on a renderer, window, native device, or application host.

```text
src/data/
  mod.rs             Curated subsystem exports
  value.rs           DataValue, entry specifications, validators, erased value adapters
  variable.rs        Typed shared handles
  registry.rs        Registration, snapshot transactions, optional global access
  subscription.rs    Bounded/coalescing change mailboxes
  persistence.rs     Resource export/restoration contracts
  format.rs          Versioned TOML snapshots and transactional restore
  file.rs            Serialized file writes and safe replacement
  autosave.rs        Debounce worker, status, flushing, and shutdown
  error.rs           Actionable failure categories
```

### Ownership and value contracts

- `Registry` retains entries identified by stable application-defined string keys. `Variable<T>`
  handles share storage and remain valid across reloads and after the registry owner is dropped.
  The owner, not the variable handles, controls the autosave worker lifetime.
- `DataValue` requires `Serialize + DeserializeOwned + Clone + Send + Sync + 'static`.
  Values must support a faithful TOML round trip. Unsupported representations are errors, not
  silently converted strings. Encoding, decoding, cloning, and validation must be side-effect-free.
- Basic colors and geometry remain owned by `foundation`. Theme, motion, media, and graphics
  configuration retain their respective owners. Persistence support alone does not move them into
  `data`, introduce new crates, or justify a broad repository migration.
- Duplicate keys, missing lookups, incorrect types, invalid values, and conflicting transactions
  produce explicit errors. Defaults are validated at registration and retained for reset operations.
- Intrinsic invariants must be enforced during deserialization. Registration validators add
  application-specific restrictions. Validation and user closures run without registry locks.

### Live replacement and concurrency

- Values are immutable shared snapshots replaced under one registry lock. Reads release that lock
  before cloning or inspecting values. Multi-value reads use one snapshot.
- Transactions stage and validate replacements before a single commit. Concurrent registry changes
  produce `Conflict`; arbitrary user closures are not retried automatically. Single-variable updates
  similarly detect conflicting replacement rather than overwriting a concurrent update.
- Subscribers receive coalescing mailboxes with the latest revision and union of changed keys.
  Internal mailbox publication preserves commit order; it invokes no application callbacks.
  Consumers read current snapshots and perform UI invalidation or subsystem reconfiguration themselves.
- The optional `Settings` global is installed once per process. Independent registries remain
  supported. Access before installation is fallible; no hidden initialization or cross-process
  sharing occurs. Real-time code receives prepared updates through its subsystem instead of locking
  the registry.

### Persistence and autosave

- Manual saving is the default. Serialization captures a consistent snapshot and writes without
  holding the registry state lock. A registry serializes its file operations so an older snapshot
  cannot overwrite a newer completed write. Sharing a destination across independent registries or
  processes requires coordination outside this subsystem.
- TOML documents contain a registry identity, schema version, and keyed values. Identity/version
  mismatch is an error. Loading decodes and validates all matching entries before committing;
  failure leaves all values unchanged. Missing keys retain current values. Unknown keys survive
  saving and may be consumed by later registration.
- Explicit restore establishes a clean baseline and does not schedule autosave. It discards pending
  edits for keys present in the document; callers choose when to reload. Existing handles observe
  restored values. File watching and schema migration are extension points, not implicit behavior.
- File saving uses an exclusively created temporary file beside the destination, flushes its
  contents, and renames it over the destination. Unix also syncs the parent directory. Failures are
  returned; unsupported replacement never falls back to deleting the existing file first. Power-loss
  guarantees still depend on the platform and filesystem.
- Optional runtime autosave uses one sleeping worker per registry, a quiet-period debounce, and a
  maximum delay under continuous changes. The maximum bounds scheduling, not completion of slow or
  failing I/O. Failures remain observable and retry with backoff. Dirty revisions arriving during a
  write remain pending. Exporting another destination does not acknowledge the autosave file.
- `flush` synchronously saves to the configured autosave destination. Quiesce producers, then call
  `shutdown` to stop the worker and save its final snapshot. Dropping the owner stops and joins its
  worker without an implicit final save. Global registries require explicit shutdown because static
  storage is not dropped at process exit. Abrupt exit can lose changes not yet persisted.

### Resource preferences and application

`SaveState` exports persistent data from a runtime object; `RestoreState<S>` is implemented by a
service capable of recreating a resource. Loading a registry never invokes restoration or opens a
device. Async services may expose their own asynchronous restoration operations.

Saved preferences contain persistent selectors and fallback policy, never raw pointers or
connection-scoped handles. Desired configuration remains separate from observed hardware state.
Fallback does not overwrite the preference automatically. A successful storage transaction does not
promise successful or atomic hardware reconfiguration.

The initial audio adapter persists configured PipeWire sink/source names and resolves them against
current discovery. These names identify configured endpoints, not universally stable physical
hardware across machine reconfiguration. Ambiguous names and invalid directions are rejected.
Restoration creates an ordinary buffered stream; actual readiness and negotiated format remain
observable on that stream. It does not automatically replace an existing stream or authorize access
beyond the supplied connection.

### Adoption scope

The initial implementation includes the registry, live updates, manual persistence, global access,
and runtime autosave; basic color/geometry, scroll, and motion value integration; and a buffered
audio preference adapter. It does not claim every existing configuration is serializable. Additional
keyboard, renderer, GPU, MIDI, camera, and display preferences should adopt these contracts through
focused subsystem changes. Settings editors, automatic hardware switching, file watching, and schema
migrations remain later work.

## Governing standard

> Every capability has a clear owner, every interface has a purpose, every resource has a lifecycle,
> and every implementation has an understandable place in the SDK.

# Telorgon screen brightness API design

This proposal gives shell widgets, brightness keys, and trusted commands one asynchronous service
for changing physical display brightness. Telorgon owns policy and request ordering; a native
adapter performs device I/O. Linux laptop panels use the kernel backlight interface through
logind by default. External monitors use an optional, narrowly scoped DDC/CI broker.

Status: design contract with an initial implementation in `services::screen_brightness` and
`platform::linux::screen_brightness`, September 30, 2026. The sketches below describe intended
contracts; consult the Rust declarations for exact signatures. Linux panels require
`shell-screen-brightness-linux`; DDC requires `shell-screen-brightness-ddc-linux` and a trusted
launcher for the private broker channel. No device settings or system configuration were changed.

## Fit with the current SDK

The canonical owner should be `services::screen_brightness`, publicly re-exported as
`telorgon::screen_brightness` and from the shell programmer's prelude. It owns values, requests,
snapshots, and the controller lifecycle. Linux adapters belong under `platform::linux::screen_brightness`;
`host::linux_shell` supplies seat authority, output associations, lock policy, and startup/shutdown.

Relevant existing code: [shell service registry](crates/telorgon/src/authoring/compose/shell_services.rs),
[audio control handle](crates/telorgon/src/host/application/desktop_audio.rs),
[display control](crates/telorgon/src/host/application/display_control.rs), and
[seat lifecycle](crates/telorgon/src/platform/linux/session.rs). The ownership rules follow the
[development specification](DEVELOPMENT_SPECIFICATION.md).

Use the existing `ShellServiceRegistry` and `ShellServices::service<T>()` to deliver the handle to
widgets. Follow `DesktopAudioHandle` for nonblocking admission, immutable `Signal` snapshots, and
explicit worker ownership. Keep brightness independent of the existing `DisplayControl`, which
controls display modes and scale, and the neutral `DisplayService`, which publishes topology facts.

The existing `shell::SystemRequest` invokes host-published action identities and has no arbitrary
parameter payload. Fixed brightness-up/down status actions may delegate to this service. A slider
needs the typed service API below. If a remote shell transport is later required, add a focused
brightness request to that transport with host-side authorization; do not encode levels in action
IDs, strings, or a Wayland protocol automatically exposed to every client.

## Programmer API

The proposed public surface is deliberately small:

```rust,ignore
use telorgon::{screen_brightness::*, shell::OutputId};

// Private fields; validated construction and deserialization.
pub struct ScreenBrightnessLevel(u16); // 0..=10_000 basis points of device maximum.
pub struct ScreenBrightnessDelta(i16); // -10_000..=10_000; percentage POINTS.
pub struct ScreenBrightnessDeviceHandle { /* controller identity, ID, incarnation */ }

impl ScreenBrightnessLevel {
    pub fn percent(value: f32) -> Result<Self, ScreenBrightnessError>;
    pub fn basis_points(value: u16) -> Result<Self, ScreenBrightnessError>;
    pub fn as_percent(self) -> f32;
}
impl ScreenBrightnessDelta {
    pub fn percentage_points(value: f32) -> Result<Self, ScreenBrightnessError>;
}

pub enum ScreenBrightnessTarget {
    Device(ScreenBrightnessDeviceHandle),
    Output(OutputId),
    DefaultInternal,
}
pub enum ScreenBrightnessAction {
    Set(ScreenBrightnessLevel),
    Adjust(ScreenBrightnessDelta),
}
pub enum ScreenBrightnessWriteMode {
    Ordered,
    ReplacePendingSet,
}

// Cloneable, revocable write authority; construction is host controlled.
pub struct ScreenBrightnessHandle { /* ... */ }
impl ScreenBrightnessHandle {
    pub fn observer(&self) -> ScreenBrightnessObserver;
    pub fn signal(&self) -> Signal<ScreenBrightnessSnapshot>;
    pub fn execute(
        &self,
        target: ScreenBrightnessTarget,
        action: ScreenBrightnessAction,
    ) -> Result<ScreenBrightnessRequest, ScreenBrightnessError>; // Ordered by default.
    pub fn execute_with_mode(
        &self,
        target: ScreenBrightnessTarget,
        action: ScreenBrightnessAction,
        mode: ScreenBrightnessWriteMode,
    ) -> Result<ScreenBrightnessRequest, ScreenBrightnessError>;
}

// Read-only views can be distributed without write authority.
pub struct ScreenBrightnessObserver { /* ... */ }
impl ScreenBrightnessObserver {
    pub fn signal(&self) -> Signal<ScreenBrightnessSnapshot>;
}

#[must_use]
pub struct ScreenBrightnessRequest { /* request ID and bounded completion cell */ }
impl ScreenBrightnessRequest {
    pub fn id(&self) -> ScreenBrightnessRequestId;
    pub fn state(&self) -> ScreenBrightnessRequestState;
    pub fn cancel(&self) -> ScreenBrightnessCancellation;
    pub async fn completion(self) -> ScreenBrightnessOutcome;
}

// A non-cloneable owner, moved into the shell host.
pub struct ScreenBrightnessController { /* ... */ }
impl ScreenBrightnessController {
    pub fn new(config: ScreenBrightnessConfig) -> Result<Self, ScreenBrightnessError>;
    pub fn handle(&self) -> ScreenBrightnessHandle;
    pub async fn shutdown(&mut self) -> Result<(), ScreenBrightnessError>;
}
```

`Signal` is the SDK's existing reactive snapshot type. Constructors reject NaN, infinity, and
out-of-range values; fractional percent input rounds to the nearest basis point. `Adjust(+5)` means
five percentage points, so 40% becomes 45%. These are normalized device control values, not nits or
guarantees about perceived luminance. Perceptual slider curves belong in shell presentation policy.

Example widget or command handler, using the proposed additions:

```rust,ignore
let brightness = shell_services.service::<ScreenBrightnessHandle>()?;

// Enqueue immediately; execute() does no filesystem, D-Bus, or I2C I/O.
let request = brightness.execute(
    ScreenBrightnessTarget::Output(output),
    ScreenBrightnessAction::Set(ScreenBrightnessLevel::percent(60.0)?),
)?;

// Await in a task. Paint observed state from brightness.signal(); expose pending separately.
match request.completion().await {
    ScreenBrightnessOutcome::Applied(applied) => show_confirmed(applied.level),
    other => show_brightness_result(other),
}

let request = brightness.execute(
    ScreenBrightnessTarget::DefaultInternal,
    ScreenBrightnessAction::Adjust(ScreenBrightnessDelta::percentage_points(-5.0)?),
)?;
```

Add `ShellEnvironment::screen_brightness(controller)` to move ownership into the host and install its
handle in the existing registry. Construction only validates configuration and creates inert
queues/signals. The host initializes the compiled adapter after obtaining its seat, starts
observation, and enables writes only while authority is current. Advanced embedded hosts can
inject an explicit provider through a separate extension interface; provider installation is a
trusted host operation. There is no process-global privileged brightness setter.

## Device identity and snapshots

Target a physical control endpoint, because one actuator can affect several mirrored outputs.
`ScreenBrightnessDeviceHandle` contains an opaque controller identity and device incarnation. A removed
device, restarted adapter, or reused kernel name cannot make an old handle valid for new hardware.
Handles are not persistent selectors and are not proof of authorization. This protects admission
and dispatch within Telorgon; name-based native transports have a replacement race described below.

Each complete `ScreenBrightnessSnapshot` has a revision, service state, pending count, and a bounded
list of endpoints. Each endpoint publishes:

| Field | Meaning |
| --- | --- |
| `device` and `name` | Current opaque handle and bounded display label |
| `kind` | Internal backlight or external monitor control |
| `association` | Confirmed output IDs, trusted host override, unknown, or ambiguous |
| `capability` | Existing `Support` and `PermissionState` concepts, kept separate |
| `range` | Effective minimum, maximum, and native quantization |
| `configured` | Latest observed control setting, if readable |
| `reported_actual` | Optional device-reported hardware value, with accuracy metadata |
| `pending` | Desired level and request ID, explicitly pending |
| `observed_at` and `health` | Freshness and current device/transport health |

`Output` and `DefaultInternal` resolve when admitted, and the request retains that device's
incarnation, mapping generation, and authority generation. Later topology changes invalidate the
request instead of redirecting it. `DefaultInternal` is a configured host default or the sole
unambiguous internal endpoint; multiple candidates produce `AmbiguousTarget`. Virtual/headless
outputs produce `Unsupported`. There is no implicit all-monitor operation in the first version.

Linux discovery enumerates only `backlight` devices for panels. Associate them using available
kernel topology and host knowledge of the selected GPU/connector. Shared GPU ancestry alone does
not prove a panel mapping. Where topology is insufficient, retain an unknown association or require
a trusted host override. Deduplicate alternate interfaces for the same actuator before exposing
them. Never select the first directory, assume an `intel_backlight` name, or apply a universal
firmware/raw preference as if it worked on every machine.

For external monitors, use connector DDC-adapter topology plus EDID identity where available.
Missing/duplicate EDID identities and ambiguous DisplayPort MST routes require an explicit mapping.
Neither a bus number nor a connector ordinal is a durable physical monitor identity.

## Levels and completion

Map a normalized level to native units using checked `u64` arithmetic:

```text
raw = (basis_points * native_max + 5_000) / 10_000
```

Reject zero/invalid native maxima. Unless zero is explicitly authorized, compute
`native_min = max(1, ceil(host_floor_basis_points * native_max / 10_000))`, also respecting any
known device minimum. Clamp converted native values to that effective range and expose its
quantized limits in the snapshot. This keeps coarse devices from rounding a nonzero floor to zero.
Native readback maps back to the nearest basis
point and can differ from the request through quantization. Very coarse devices expose their
quantization; a nonzero adjustment that would round back to the same setting moves at least one native
step in its requested direction, unless already at the applicable limit.

Proposed default shell policy sets a 1% floor and never writes native zero. This is a usability
precaution, not a guarantee that every panel remains visible. A host may choose a different floor
and explicitly authorize zero. Absolute values below the advertised floor return `BelowMinimum`;
relative adjustments clamp at the floor/maximum and report the effective value. Screen power,
DPMS, and session blanking remain separate operations; brightness does not unblank a screen.

`execute()` returns admission or an immediate typed error. Every admitted request gets exactly one
terminal result, even if its handle is dropped. The proposed outcomes are:

| Outcome | Contract |
| --- | --- |
| `Applied(ScreenBrightnessApplied)` | The operation completed and matching control readback was observed; includes level, revision, and verification source |
| `Unconfirmed { reason, observed }` | A command may have applied, but readback/transport did not establish completion; includes optional fresh state |
| `Superseded` | A pending absolute set was replaced before dispatch |
| `Cancelled` | Cancellation prevented native dispatch |
| `Denied` | Authority or host policy denied execution |
| `Stale` | Endpoint, mapping, or authority generation changed |
| `Failed(PlatformError)` | A definite failure with a classified, sanitized cause |

`ScreenBrightnessApplied` records `DriverSetting` or `MonitorVcp` verification and optional hardware
readback. Matching driver state is not optical measurement. A successful bus reply or `write()`
alone never becomes `Applied`; missing, mismatched, or failed readback yields `Unconfirmed`.
Cancellation reports whether it prevented dispatch, arrived too late, or found a terminal request.
Dropping a request abandons observation without promising cancellation.

`Cancelled`, `Superseded`, `Denied`, and `Stale` mean native dispatch was prevented. Authority or
device loss after dispatch produces `Unconfirmed` if completion cannot be established. Suppressing
a stale observation never suppresses its request's terminal result. Publish confirmed state before
waking an `Applied` completion, so the caller can read a snapshot at least as new as the cited revision.

The panel's `brightness` attribute represents its requested setting; `actual_brightness` can
differ during blanking or power saving. Relative changes use a fresh configured setting so a
blanked panel reporting actual zero does not erase the user's setting. Missing hardware readback
stays unknown. [Linux backlight ABI](https://www.kernel.org/doc/Documentation/ABI/stable/sysfs-class-backlight)

Immediate errors include `InvalidLevel`, `BelowMinimum`, `Unsupported`, `Unavailable`,
`PermissionDenied`, `SessionInactive`, `Locked`, `StaleDevice`, `AmbiguousTarget`, and `QueueFull`.
Native messages, filenames, bus identifiers, and errno details stay in restricted adapter
diagnostics; portable errors reuse the SDK's redaction-safe `PlatformError` conventions.

## Ordering and responsiveness

Use one bounded controller scheduler and serialize operations per physical endpoint and shared DDC
adapter. Start with 32 outstanding requests and at most 64 endpoints. Each admitted request owns
one completion cell; there is no accumulating completion log. Bound discovery strings, reads,
protocol buffers, and process messages as well as commands. Quarantined in-flight operations retain
their capacity reservation until recovery, even after their caller receives a terminal result.

`Ordered` preserves arrival order for sets and adjustments. An adjustment reads the current
configured setting immediately before computing its absolute target, after earlier operations
finish. Several held-key steps therefore accumulate correctly. It is serialized within Telorgon,
but cannot be atomic with unrelated programs changing the same device; a concurrent external change
may race the read/write. Subsequent observations always remain authoritative.

`ReplacePendingSet` is valid only for `Set`. It replaces only an adjacent undispatched set from
the same authority and target, never crosses an ordered action or adjustment, and completes the
replaced request as `Superseded`. It does not replace an in-flight write. This lets sliders retain
the newest intent without hiding queue overflow or losing completion accounting.

Perform native I/O off the compositor/input/render thread, with no shared-state lock held across
I/O. Publish immutable snapshots through the existing thread-safe `Signal`. It invokes framework
notification callbacks synchronously on the publishing thread and does not cap subscriptions;
reuse its existing owner-lifecycle subscriptions and add no arbitrary user-callback stream.
Framework-installed notifications must remain short and wake the UI owner. Service queue bounds
do not imply a new limit on the runtime's signal subscriptions. Native events trigger refresh; polling with
backoff supplements missing notifications. Poll DDC only while needed, at a low device-appropriate
rate, rather than per frame. Do not introduce brightness animations in the first implementation.

Use finite backend deadlines and bounded retry budgets. A deadline ends the caller's wait, not a
kernel operation. Keep a timed-out operation occupying its endpoint until the transport is drained
or reset; do not issue overlapping retries. Retry safe reads where appropriate. Do not retry an
uncertain relative change or silently replay queued work after resume/reconnect.

## Linux panel backend

### Preferred logind call

Connect to the system bus with the existing `zbus` dependency. Resolve the host's actual session
through `org.freedesktop.login1.Manager.GetSessionByPID`, or the host's explicit validated session
identity where its launcher owns that relationship. Obtain the returned object path instead of
constructing one from an untrusted environment variable.

```text
service:   org.freedesktop.login1
interface: org.freedesktop.login1.Session
object:    the validated session object
method:    SetBrightness
signature: ssu
arguments: ("backlight", enumerated_device_sysname, raw_level_u32)
```

Current upstream logind checks foreground seat state, caller UID against the session owner (or
root), and device seat membership. This method does not require session `TakeControl` and does
not invoke a polkit check. It does not enforce Telorgon component permissions or session lock
policy. These conclusions come from the method implementation, so runtime compatibility must also
be checked against supported deployments. [logind session implementation](https://github.com/systemd/systemd/blob/main/src/login/logind-session-dbus.c)

Read panel attributes through pinned sysfs descriptors and validate their bounded decimal values.
Track system-bus owner changes, session activity, seat removal, and service disconnects. A denied
logind request must stay denied; do not evade it with a more privileged direct-write fallback.

Logind receives only a sysname, not Telorgon's incarnation or a pinned device descriptor. Removal
and replacement under the same sysname between validation and server execution can redirect an
already-issued call. Observed generations reject stale queued work but cannot close that native
race. Deployments requiring strict device binding need a descriptor-bound writer/broker contract.

### Explicit direct sysfs adapter

For a deployment without usable logind, support an explicitly configured direct adapter only when
the process already has appropriately scoped write access or a narrow broker supplies it. Missing
permissions remain observable. Do not change file ownership, chmod sysfs, invoke sudo, spawn a
shell command, or run the compositor as root to make brightness work.

The adapter uses `openat2`/`openat`, `fstatfs`/`fstat`, bounded `read` or offset-zero `pread`, a single
`write`, and `close` through owned file descriptors:

1. Pin a trusted `/sys` directory and verify the expected sysfs mount. Enumerate kernel-owned
   `class/backlight` entries; accept no caller-supplied path.
2. Resolve an enumerated entry relative to that root with `RESOLVE_BENEATH | RESOLVE_NO_MAGICLINKS`
   and a same-mount constraint. Class entries are legitimate symlinks into `devices`, so globally
   rejecting symlinks would break ordinary sysfs.
3. Pin the resolved device directory. Open only fixed `brightness`, `max_brightness`,
   `actual_brightness`, and `type` attributes beneath it, with `O_CLOEXEC`, no final symlink, and
   traversal/mount restrictions. Validate the current device incarnation before dispatch.
4. Encode a checked native value as short ASCII decimal plus newline. Send the complete buffer in
   one `write()` and require an exact count. A short write must not trigger a suffix write, which
   would be a second independent sysfs command. Read back on success and on an uncertain failure.

These path constraints are a proposed adapter policy built on
[openat2 resolution controls](https://man7.org/linux/man-pages/man2/openat2.2.html).
When unavailable, a vetted descriptor-walk implementation must preserve those invariants; otherwise
report the direct adapter unavailable. [sysfs documentation](https://docs.kernel.org/filesystems/sysfs.html)
documents class symlinks and the complete-buffer write contract.

Subscribe to backlight change/hotplug events using a udev adapter; treat events as refresh hints
and reopen/revalidate device state. Kernel backlight changes generate uevents, including firmware
changes when the driver reports them. [Kernel backlight documentation](https://docs.kernel.org/gpu/backlight.html)
Polling remains a bounded fallback when a driver misses notifications.

### Kernel path and required support

```text
Telorgon request
  -> host authorization and native range conversion
  -> logind SetBrightness, or explicitly authorized sysfs write
  -> sysfs attribute store
  -> backlight core updates requested brightness
  -> driver's backlight_ops.update_status()
  -> existing GPU, ACPI, PWM, or panel hardware implementation
```

Backlight drivers supply the update callback and may supply `get_brightness`; userspace does not
call those functions directly. Supported panels need an existing driver registering a backlight
device and a usable sysfs mount. No Telorgon kernel module or custom brightness syscall is required.
[Kernel backlight driver contract](https://docs.kernel.org/gpu/backlight.html)

Do not assume DRM master ownership, a libseat device grant, a render node, or gamma-LUT access
authorizes a backlight write. A future connector-native brightness adapter must negotiate an
actually available kernel interface; this design does not depend on proposed DRM luminance APIs.
Software color dimming and HDR luminance policy are separate features with separate semantics.

## External monitor backend

Probe DDC/CI brightness VCP feature `0x10` and its reported maximum/current value for an explicitly
mapped monitor. A monitor's maximum need not be 100. DDC may be unavailable, disabled in the
monitor menu, or inaccessible through a dock; expose that capability result instead of simulating
hardware brightness. [ddcutil brightness example](https://www.ddcutil.com/getvcp_known_u3011_output/)

Prefer a vetted, version-qualified DDC implementation such as `libddcutil` inside the broker.
Its public API includes display selection/open/close and non-table VCP get/set operations; explicitly
read feature `0x10` back after setting it. Do not parse command output or execute `ddcutil` through
a shell. [ddcutil API](https://www.ddcutil.com/api_main/),
[public function declarations](https://github.com/rockowitz/ddcutil/blob/master/src/public/ddcutil_c_api.h)

At the transport boundary, Linux uses `i2c-dev`: a validated `/dev/i2c-N` character device opened
with `O_RDWR | O_CLOEXEC`, `ioctl(I2C_FUNCS)` for supported operations, and protocol-appropriate
`read`/`write` or `ioctl(I2C_RDWR)` transfers. DDC/CI messages use the 7-bit monitor control address
`0x37` (wire address bytes `0x6e`/`0x6f`).
[ddcutil transport explanation](https://www.ddcutil.com/bibliography/)
Separate request, device-required delay, and response phases follow the protocol. Do not
substitute arbitrary SMBus calls, force an occupied I2C address, or invent one combined transaction
for every monitor. Kernel adapter numbers can change across boots.
[Linux userspace I2C interface](https://docs.kernel.org/i2c/dev-interface.html)

Access to a raw I2C descriptor permits much more than brightness. Keep it inside a small broker
process whose only client operations are list/read/set for allowlisted monitor identities and
VCP `0x10`. The broker derives device paths, addresses, message lengths, and protocol bytes itself;
clients supply none of them. Never pass a raw descriptor or arbitrary-I2C RPC to the shell or
extensions. Restrict discovery to mapped display adapters; do not probe every system I2C bus.

If elevated access is required, install the broker separately as distribution packaging under
`packaging/linux/brightness`. Its authenticated endpoint checks actual peer identity, a registered
session, active seat, monitor ownership, and a restricted caller role on every dispatch. A private
host channel or deployment-enforced service identity is needed to distinguish the authorized shell
from arbitrary processes sharing its UID. A UID comparison or a caller-asserted PID alone is
insufficient. Drop unnecessary capabilities, restrict accessible devices, and sandbox the broker;
an ioctl allowlist alone cannot restrict the address or payload inside `I2C_RDWR`.

DDC calls can stall inside a driver. A broker process isolates that stall from the compositor and
can be replaced for recovery. Killing it does not retract an already-issued kernel transaction.
Quarantine its endpoint and shared adapter until the old worker has exited and been reaped with
transport drained, or an authoritative device reset establishes recovery. Rediscovery/readback
alone does not establish that a previous transaction finished. A replacement worker can serve
unaffected adapters meanwhile; the affected endpoint resumes only after recovery and rediscovery.

## Authorization and lifecycle

The host mints revocable handle scopes for observation and adjustment of specific endpoints.
Registry installation and Rust type safety are useful capability plumbing, but in-process
untrusted native code is not sandboxed by a handle. Host boundaries validate their live grant
registry; process boundaries also authenticate callers. `InputSource` and request metadata are
not evidence of a real gesture or authorization.

Revalidate scope, configured minimum, session activity, lock policy, device incarnation, and mapping
at admission and immediately before native dispatch. Default policy permits trusted shell controls
while the seat is active and unlocked. Optional brightness keys on the lock screen require an
explicitly narrower trusted-host rule; arbitrary widget/programmatic requests remain denied.
Ordinary applications do not receive a write handle automatically. A narrow, explicitly authorized
settings IPC endpoint can forward the same typed operations without exposing a generic privileged
service registry.

This secures Telorgon's own API. Logind's session-owner UID authorization can still let a separate
unsandboxed process with that UID call `SetBrightness` directly. Preventing that requires deployment
sandbox/bus policy; a Rust handle cannot impose that restriction on the operating system.

On seat loss, lock-policy revocation, disconnect, suspend, or host shutdown, advance authority
generation, reject unsent work, stop repeats, and mark snapshots unavailable or restricted. Suppress
stale asynchronous results. Resume requires fresh authority and device observation; never replay
old deltas. An operation already issued may finish during a concurrent VT switch; generation checks
cannot undo it. State this limitation instead of claiming atomic revocation across kernel I/O.

The controller owns workers and descriptors; cloned handles cannot keep them running. Explicit
shutdown closes admission, resolves queued requests, and drains or isolates in-flight operations
without blocking the UI thread. Drop immediately revokes admission and requests cleanup; it never
waits indefinitely on a blocked native call. The host coordinates explicit shutdown before teardown.
Do not restore saved brightness on shutdown or reconnection unless the host explicitly requests it.

Persist desired preferences only through the existing `data` contracts, using physical selectors
and fallback policy. Never persist controller handles, incarnations, paths, file descriptors, or
I2C bus numbers, and never treat loading preferences as permission to mutate devices.

## ScreenBrightness keys

Add `ShortcutKey::ScreenBrightnessUp` and `ScreenBrightnessDown` and a typed
`KeyBindings::screen_brightness_keys(handle, target, config)` helper. Existing shortcuts run once per
fresh press and consume repeats; brightness repetition must be added deliberately without changing
all shortcut handlers. Use the same queued `Adjust` operations as widgets, with an explicit initial
delay, bounded repeat rate, configured step, and teardown on release/seat loss.

`ScreenBrightnessKeyHandling` should distinguish `ShellAdjusts`, `FirmwareAdjusts`, and `Disabled`.
In firmware mode the host observes and displays confirmed changes but emits no second write.
There is no reliable universal heuristic that an input key proves firmware did or did not handle
the change. Keep an explicit host setting and support device-qualified quirks. Firmware-generated
observation may arrive asynchronously; refresh feedback follows observed state.

## Implementation sequence and verification

1. Implement portable values, snapshot/capability records, request state, bounded queue, cancellation,
   incarnation checks, and a fake provider. Reuse existing `Signal`, permission, and error types.
2. Add logind and sysfs observation adapters, then host authority/topology integration and the
   registry-installed handle. Direct writes remain an explicit deployment option.
3. Add typed brightness key bindings, including press/repeat/firmware/lock policy.
4. Add the optional DDC broker after its protocol, packaging, authentication, and device allowlist
   have been reviewed. Keep the public API identical across adapters.

Portable declarations remain available without native features. Introduce a focused
`shell-screen-brightness-linux` feature for Linux shell assembly and a separate optional DDC feature;
compilation never starts a worker or claims devices. Other native platforms initially report
unsupported, while a trusted embedded provider can implement the same contract.

Focused deterministic checks should establish level validation/quantization, repeated adjustments,
coalescing barriers and terminal accounting, forged/revoked scope rejection, stale output/device
rejection, cancellation timing, readback mismatch, queue bounds, partial sysfs writes, and broker
rejection of arbitrary paths/addresses/features. Compile the portable surface and Linux feature
entry points separately. Hardware qualification then covers a real panel, firmware-handled keys,
blanking, VT/lock transitions, hotplug, and an external monitor with delayed or failed DDC readback.

The design was checked against existing source and primary Linux/systemd/DDC documentation.
The implementation has focused portable and Linux feature tests. Hardware qualification remains
a user-run check. The initial broker implements brightness-only DDC packets directly; the launcher
is responsible for process isolation, permissions, channel setup, and stalled-worker recovery.

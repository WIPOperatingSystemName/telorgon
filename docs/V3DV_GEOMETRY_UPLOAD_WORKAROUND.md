# V3DV geometry-upload synchronization workaround

## Status and scope

Implemented driver-scoped compatibility behavior; the reported Raspberry Pi diagonal resize
corruption still requires a user-run before/after comparison. Finding the driver defect does not
by itself prove it caused the observed artifact. Compilation and CPU tests are not GPU qualification.

The Vulkan renderer normally completes retained storage-buffer uploads with
`COPY / TRANSFER_WRITE -> VERTEX_SHADER | FRAGMENT_SHADER / SHADER_STORAGE_READ`.
On `VK_DRIVER_ID_MESA_V3DV` only, the destination access becomes `SHADER_READ`.
All other drivers, including unknown IDs, retain the precise existing mask.

Both owned and hosted device construction query `VkPhysicalDeviceDriverProperties`, select the same
internal `DriverWorkarounds` policy, and retain it on the device. Both single-scene execution and
Linux compositor recording pass this policy into the shared upload recorder. No public configuration,
dependency, device feature requirement, or Vulkan API baseline changes.

The exception currently applies to every V3DV version: no fixed upstream release has been verified.
Driver versions are diagnostic data, not a guessed cutoff. A version restriction or removal requires
verification of the upstream fix, downstream packaging, and a hardware regression test. Do not match
GPU names, assume every Mesa driver is affected, or interpret all vendors' raw versions as Mesa
versions.

## Evidence and invariants

The source package for Raspberry Pi `mesa-vulkan-drivers:arm64 25.0.7-2+rpt2` contains a narrowing from
`VkAccessFlags2` to `VkAccessFlags` in V3DV's `cmd_buffer_binning_sync_required`. The storage-read flag
is above bit 31 and is lost there. The package's downstream patches do not change this function.
The same narrowing was also inspected in later upstream source; a later version number alone is not
proof that the issue has been fixed.

Primary sources:

- [Raspberry Pi source-package manifest](https://archive.raspberrypi.com/debian/pool/main/m/mesa/mesa_25.0.7-2+rpt2.dsc)
  identifies the original source and downstream patch archives.
- [Inspected V3DV implementation](https://chromium.googlesource.com/external/gitlab.freedesktop.org/mesa/mesa/+/0ae28c9056ae57242fc5ca43fc6b6e57b58c23ea/src/broadcom/vulkan/v3dv_cmd_buffer.c)
  contains `cmd_buffer_binning_sync_required` and the binning synchronization decision.
- [Vulkan access flags](https://docs.vulkan.org/refpages/latest/refpages/source/VkAccessFlagBits2.html)
  specify that `SHADER_READ` includes storage reads. Its low bit survives the driver's narrowing.
- [Vulkan driver properties](https://docs.vulkan.org/refpages/latest/refpages/source/VkPhysicalDeviceDriverProperties.html)
  define the driver ID and informational strings queried through the physical-device property chain.

The original source archive's SHA-256 is
`592272df3cf01e85e7db300c449df5061092574d099da275d19e97ef0510f8a6`;
the `25.0.7-2+rpt2` downstream archive's is
`a1bfabf1f94d98ee42b8e88e62d12cef43c0cd373ecfb0dbb20799a6b8a62498`.
Both were checked against the manifest during diagnosis.

Invariants preserved:

- Access masks remain `VkAccessFlags2`; Telorgon does not truncate them.
- Upload-completion source stages/access, destination shader stages, buffer ranges, and queue-family
  fields are unchanged. The default path produces the original dependency.
- Newly initialized buffers still have no preceding shader read. Existing buffers retain the
  original shader-read-to-copy overwrite dependency.
- Image sampling, layouts, DMA-BUF acquire/release, presentation, frame lifetimes, and resize policy
  do not change.
- There are no new submissions, idle waits, per-frame property queries, or background workers.
- The policy remains backend-internal and is selected independently for each device.

## Diagnostics

Successful owned and hosted construction append one informational message to
`VulkanDevice::diagnostics().messages()` (shared with the instance diagnostics). The message includes
the adapter name, driver ID, driver name/info, raw driver version, and
`v3dv_geometry_upload_read=enabled` or `disabled`. This is not a warning or a claim that the resize
problem has been resolved. It is collected even when validation layers are disabled, and does not
print to the terminal automatically.

With instrumentation active, initialization also emits the counters `gpu.adapter.driver_id` and
`gpu.workaround.v3dv_geometry_upload_read` (1 for enabled, 0 for disabled). There is no per-frame
diagnostic work. Managed applications may use the existing profiler workflow; starting its service
remains a user action.

## Reference implementation audit

Concern: driver-scoped compatibility for storage-buffer upload visibility without weakening the
cross-driver synchronization contract.

Inspected adjacent references, relative to the repository root:

- wgpu `d99c241a3b9dcc0f6674d990d007d79e94d39862`:
  `../other-rendering-libs/wgpu/wgpu-hal/src/vulkan/adapter.rs` (driver-property query and workaround
  selection), `conv.rs::map_buffer_usage_to_barrier` (storage reads map to broad shader reads), and
  `command.rs::transition_buffers` (stage/access pairing and buffer-barrier recording).
- Flutter `51fd9afadf309ba5337320bd3653f5345c156cb9`:
  `../other-rendering-libs/flutter/engine/src/flutter/impeller/renderer/backend/vulkan/workarounds_vk.h`,
  `workarounds_vk.cc::GetWorkaroundsFromDriverInfo`, and
  `compute_pass_vk.cc::AddBufferMemoryBarrier` (separate driver policy and broad shader-read
  dependencies). The routed `barrier_vk.cc` is only an empty translation unit in this revision.

These implementations inform policy placement and synchronization invariants; their APIs and code
were not copied. Telorgon retains synchronization2 and its existing precise stage/range contract.

Rejected alternatives: globally replacing access flags would unnecessarily affect unrelated drivers;
device-wide idle or forced single-frame operation would change scheduling and hide the dependency;
GPU-name/vendor-only matching would conflate distinct driver implementations; an arbitrary version
cutoff would assume an unverified fix. This patch does not modify the system driver.

## Verification and user-run acceptance

CPU regressions cover V3DV selection, unaffected AMD/NVIDIA/Intel/other Mesa/unknown IDs, default
behavior, the modeled 32-bit narrowing, informational diagnostics, exact upload dependency fields,
new-versus-reused buffer handling, and unchanged sampled-image access. Buffer handles in these tests
are inert test values; the tests do not load Vulkan or execute GPU commands.

Build verification covers native Windows Vulkan, standalone embedded Vulkan without profiling,
ARM64 Linux Wayland with embedded Vulkan and the profiler, and compile-only hardware test targets.
Keep GPU and presentation tests opt-in. Do not run applications or services as part of this check.

Local verification recorded on 2026-09-03:

- `cargo test -p telorgon --lib --quiet`: 906 tests passed, including six new regressions.
- `cargo test -p telorgon --lib renderer_vulkan:: --features embedded-vulkan,profiler`:
  38 tests passed.
- `cargo test -p telorgon --no-run --features embedded-vulkan,profiler`: native Windows test
  executables, including hardware fixtures, compiled without execution.
- `cargo check -p telorgon --lib --no-default-features --features embedded-vulkan`:
  standalone embedded Vulkan compiled without instrumentation.
- `cargo check -p telorgon --tests --target aarch64-unknown-linux-gnu --no-default-features --features shell-wayland-linux,embedded-vulkan,profiler`:
  ARM64 Linux library and test targets checked without execution.
- `cargo build -p telorgon --lib --release --target aarch64-unknown-linux-gnu --no-default-features --features shell-wayland-linux,embedded-vulkan,profiler`:
  optimized ARM64 Linux library built.
- Formatting, `git diff --check`, and changed-document local Markdown links checked successfully.

These builds report non-fatal dead-code warnings in platform-specific code. No compositor,
application, server, or GPU test was launched. Pi visual behavior and cross-driver hardware behavior
remain unverified.

User-run acceptance:

1. Keep the Pi, Mesa `25.0.7-2+rpt2`, compositor settings, and client application the same between
   the baseline and patched builds. Ensure Cargo actually uses the patched Telorgon revision.
2. Confirm the compositor selected Vulkan/V3DV and the workaround diagnostic is enabled (or the
   profiler counter is 1). A software fallback is not a successful Vulkan test.
3. Repeat the aggressive width/height and corner resizing from the original clip. Check title bars,
   borders, content, and desktop exposure; repeat several times and compare with the baseline.
4. Compare frame timing and memory use under the same workload. A disappearance of the artifact
   supports the hypothesis; a baseline reproduction on switching back strengthens that evidence.
5. Exercise the normal path on available AMD, NVIDIA, and Intel hardware with the workaround disabled
   automatically. Record each exact driver/device; unavailable hardware remains untested.

If the Pi artifact persists, record that result and continue diagnosis of geometry/resource lifetime
and presentation. Do not label the artifact fixed or broaden this exception solely because this
targeted test did not resolve it.

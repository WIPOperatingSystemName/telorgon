# DMA-BUF rendering rework

> The follow-up [smoothness repairs](RENDERING_SMOOTHNESS_REPAIRS.md) supersede this document's
> original global target-readiness gate, owner-side spare destruction and full external output
> damage. The historical measurements and qualification entries below describe their original runs.

Implementation scope: shared Linux desktop Vulkan client composition, used by native Wayland and
Xwayland. The measured owner allocation stalls are the primary target. Existing client acquire /
release synchronization and scanout completion remain mandatory.

## Ownership and scheduling decisions

- Retain one materialization target and source scene per live surface geometry. Updates to that
  target are ordered on the same owned graphics queue with shader-read → attachment-write →
  shader-read barriers. This orders earlier GPU reads before later writes without a CPU wait.
- A bounded spare-target pool only transfers an image to another surface when no scene or submitted
  frame pins it. Size/format and content validity are distinct: reassignment requires full redraw.
- Allocate target misses on a bounded host worker. The owner keeps servicing input/protocol/KMS
  while waiting for its eventfd wake. Do not advance frame revisions or consume scene deltas until
  required targets exist. Never synchronously wait on an allocation result in the input loop.
- Cache client imports only with the immutable descriptor and DMA-BUF allocation identity. Re-arm
  a cached generation only after GPU completion, exported release resolution and exclusive CPU
  ownership. Every generation still gets its own acquire payload and one-shot release state.
- Preserve surface and buffer damage coordinate spaces separately. Coalesce damage across queued
  replacements, map outward through committed geometry, and force full redraw when geometry or
  content history is incompatible. A gap between observed surface revisions also forces full redraw:
  publish actions refer to the latest snapshot and can omit intermediate damage. Clear only the
  damaged render area before alpha composition.
- Bound render backpressure independently of scanout-buffer count; do not accumulate redundant
  primary renders while one is still executing. Client callbacks remain paced by existing host
  completion/presentation rules, and superseded unsubmitted buffers are retired normally.

## Direct sampling evaluation

Direct long-lived sampling cannot use the current one-submission foreign-image lease unchanged:
subsequent desktop repaints would reuse a consumed lease, and releasing it after the first frame
would allow Xwayland to overwrite retained content. Adopting that route requires a different
multi-submission producer ownership contract and a policy for clients that wait for buffer release
before their next commit. This rework retains the owned copy, removes its steady-state allocation
and applies damage; it does not silently bypass release/acquire or enable unqualified direct sampling.

## Reference audit

The adjacent `../other-rendering-libs` checkout is absent. Inspected upstream equivalents:

- [wgpu resource ownership](https://raw.githubusercontent.com/gfx-rs/wgpu/trunk/wgpu-core/src/resource.rs)
  and [submission lifetime tracking](https://raw.githubusercontent.com/gfx-rs/wgpu/trunk/wgpu-core/src/device/life.rs):
  resources remain pinned by active submissions; completion and user reference lifetime differ.
- [wlroots buffer ownership and damaged updates](https://raw.githubusercontent.com/swaywm/wlroots/master/types/wlr_buffer.c):
  client buffers retain locks, release when uses finish, and damaged updates require compatible
  storage and valid preceding contents. Its SHM update code is a lifetime/damage reference, not
  a DMA-BUF partial-copy implementation to copy.
- [Vulkan synchronization](https://docs.vulkan.org/spec/latest/chapters/synchronization.html),
  [kernel DMA-BUF synchronization](https://www.kernel.org/doc/html/latest/driver-api/dma-buf.html),
  and [Wayland buffer ownership](https://wayland.freedesktop.org/docs/html/apa.html#protocol-spec-wl_buffer):
  queue order alone is insufficient without memory dependencies; completed uses and one-shot
  semaphore payloads must not be conflated with buffer identity.

Rejected: reusing storage based only on raw FD numbers, overwriting images without barriers,
recycling binary semaphore payloads before completion, partial redraw into unknown pooled contents,
and blocking the owner on a worker result. Tests must cover those boundaries, coalescing, geometry
changes and bounded backpressure. Hardware tests are compiled but left for explicit user execution.

## Implemented limits and measurements

The spare-target pool holds at most 8 entries / 128 MiB; a live surface retains its own target outside
that pool. The import cache holds at most 64 entries / 512 MiB, including entries still pinned by a
submission. Eviction drops the cache reference and never overrides GPU pins. Allocation requests and
results each have capacity one, with at most one outstanding request. An allocation failure remains
an explicit renderer error rather than a silent software fallback.

Only target creation moves to the worker. Small scene setup, cache misses, resource destruction,
command recording and submission still run on the owner and can still contend with driver or
allocator work. Source scenes retain their GPU buffers, and unchanged owned texture bindings retain
their identity. A primary render can overlap an outstanding page flip, but cannot be queued behind
an executing primary render or an already-ready primary frame. This deliberately limits work ahead
of fresh input; it requires workload/refresh-rate qualification rather than assuming a throughput win.

`dmabuf_alloc_{format,image,requirements,lock_wait,memory,bind,view}` carry worker durations as
numeric instant-event values, reported when the owner receives the result. They must not be counted
as owner stalls or reconstructed as synchronized worker timeline spans. `dmabuf_target_allocated`,
`dmabuf_import_cache` and `dmabuf_materialized_pixels` expose allocation bytes/counts, hits/misses and
planned updated/full pixel area. Pixel area is not a GPU bandwidth measurement. Existing input,
owner-phase and presentation observations remain available in the same capture.

## Qualification

Portable regressions cover separate damage coordinate spaces, filter expansion, rotated/clipped
damage, missing revision history, coalesced pending updates, cache completion/release/pin guards,
and primary render backpressure. Harness tests separate worker durations from owner overlap and
reject missing evidence for explicitly requested allocation/cache budgets.

Two opt-in hardware regressions cover this rework:

- `retained_srgb_target_preserves_undamaged_pixels_across_submissions`: stable owned storage, queued
  partial updates, preservation outside the render area, transparent clearing, sRGB gray and pins.
- `cached_dma_buf_rearms_acquire_and_release_across_three_generations`: stable imported image and
  semaphore handles across completed uses, fresh acquire imports, one-shot releases and readback.

Both request Vulkan validation and assert zero reported validation errors. They are compiled only
during this task. No live performance gain, visual correctness or driver conformance is claimed
without a user-run capture. Repeat the existing no-argument `capture-input-latency.sh`, launch
`firefox-latency-telorgon.sh` inside that session, and keep the Ubuntu/Telorgon viewport, DPR,
animation setting and capture duration equal. Cold starts and resizes legitimately allocate;
steady-size repeated updates should reuse targets and recurring client buffer imports.

Build/test result for this rework: 1,228 library tests passed, two optional tests ignored; 19 harness
and browser-comparison Python tests passed. The Linux DMA-BUF and Vulkan hardware integration test
binaries compile, including the new import-cache regression. The consuming compositor's optimized
release build succeeds offline with the real embedded Xwayland payload. Rust formatting and diff
whitespace checks pass. Local socket fixtures required running the unit suite outside the socket-
restricted sandbox; no compositor, browser, hardware test or external service was launched.

## Firefox launch crash follow-up

The first user run of the rework exited with `Vulkan retained external image content version is
stale` in `test-compositor/compositor.log`. Source GPU scenes were retained, but their temporary CPU
scene builders restarted delta epochs at 1. The device's existing stale-delta gate therefore skipped
the second update while the external binding advanced to the new buffer content version. Retained
image validation correctly rejected that mismatch and the host shut down.

Source deltas now use the host's globally increasing publication content version as their epoch,
including across pooled source-scene reassignment. The backend keeps both stale-delta and external
content-version validation; mismatch errors now include the relevant epoch and versions. Its CPU
delta admission is shared with a portable regression that failed on publication 2 before the fix.
The regression covers successive publications, skipped version numbers, geometry updates and the
continued rejection of stale deltas. The three-generation hardware fixture now also advances its
scene epochs and asserts admission, rather than recreating epoch-1 updates.

This is a correction to Telorgon's existing CPU retained-scene ordering contract, with the reference
and ownership audit above still applicable; no GPU synchronization or release rule is relaxed.
The adjacent reference checkout remains unavailable. The crash log is preserved; launching Firefox
again and qualifying live latency/color remain user-run work.

Fix validation: the regression failed before the epoch correction and passes afterward; all 1,229
library tests pass with two optional tests ignored. The Linux DMA-BUF hardware binary compiles,
and the consuming compositor's release build succeeds offline with the real embedded payload.
Rust formatting and diff checks pass. No compositor or hardware test was launched for this fix.

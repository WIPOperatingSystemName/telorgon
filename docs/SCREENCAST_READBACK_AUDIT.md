# Capture readback reference audit

Scope: reusable same-queue RGBA readback, not DMA-BUF export or complete screencast qualification.
The adjacent reference library was unavailable. Upstream files were fetched read-only into `/tmp`
on 2026-09-18 instead; no upstream implementation was copied or added as a dependency.

## Inspected sources

Flutter repository, `engine/src/flutter/impeller/renderer/backend/vulkan/`:

- `blit_pass_vk.cc`: `OnCopyTextureToBufferCommand`, texture transition, tracked source/destination
  and transfer-to-host dependency.
- `device_buffer_vk.cc`: allocation-backed mapping, `Invalidate` and coherent-memory distinction.
- `command_buffer_vk.cc`: resource tracking, end-recording and error propagation.
- `command_queue_vk.cc`: submission failure, fence callback and tracked-resource retirement.
- `tracked_objects_vk.cc`: command-buffer/resource collection and destruction.

wgpu repository, `wgpu-hal/src/vulkan/`:

- `command.rs`: buffer/texture transitions, source layout selection and texture-to-buffer copies.
- `device.rs`: map/unmap, mapped-range invalidation and completion query/wait APIs.

The downloaded source content is identified by the SHA-256 values below rather than an unverified
upstream commit claim. Flutter was fetched from `flutter/flutter/master`; wgpu from
`gfx-rs/wgpu/trunk` on raw.githubusercontent.com.

| File | SHA-256 |
| --- | --- |
| Flutter blit_pass_vk.cc | f0d4efbbdc8db2873b2c015aad6c82aea9850a082fcb003a3ce2592fb2a5b8fd |
| Flutter device_buffer_vk.cc | 1174955460fa1442a12c2e5f8a395a06a662e8af0f94392a426c6cad0e39b5ab |
| Flutter command_buffer_vk.cc | be0aa4d84d79a38f832eac47b57f75a28deb729c0e2d6fef0681eeac2f9043ae |
| Flutter command_queue_vk.cc | 444d6388602f0d737f09f34a4b141c18a5f480f207b095c3f2315b62e168752e |
| Flutter tracked_objects_vk.cc | 45ceabfe51849e5dbdf2aef9c6c3b6b55dba8794b795bdb1a8692357691c98b0 |
| wgpu command.rs | b004b40c9df77458401e4e00218746479ca158c600ed97eb31033905c96c8d24 |
| wgpu device.rs | b8b565228a8f1d41c79413b94f45432de7ca89a07c4f1bc64024009d7e96f65f |

## Invariants and Telorgon decisions

Both implementations separate command recording from host access and resource retirement. Flutter
retains referenced resources through fence completion; wgpu makes layout, memory visibility and
completion explicit at the HAL boundary. Telorgon retains its existing frame-resource pins and
submission receipt, rather than adopting either project's ownership abstraction.

The source must support transfer reads, use the recorded copy layout and cover valid bounds; the
destination must support transfer writes and have sufficient storage. Copy completion and host
visibility are distinct obligations. Noncoherent mapping requires invalidation after completion.
See the official [copy command](https://docs.vulkan.org/refpages/latest/refpages/source/vkCmdCopyImageToBuffer.html),
[mapped-memory invalidation](https://docs.vulkan.org/refpages/latest/refpages/source/vkInvalidateMappedMemoryRanges.html)
and [synchronization specification](https://docs.vulkan.org/spec/latest/chapters/synchronization.html).

Initial streaming staging accepts only matching same-device, same-queue RGBA targets left in
color-attachment state, and restores that layout after copying. It does not acquire a foreign
queue or infer transfer usage for a KMS image. Dedicated capture targets must be created with
transfer-source usage. Nonblocking readback accepts only the exact frame/device receipt that
recorded it, and copies into caller-owned delivery storage. One pending use prevents re-recording.
Cancelled submitted work can discard pixels after completion; dropping storage earlier is safe
because the frame/receipt retains its allocation. Aborted, unsubmitted work retires the staging
object instead of manufacturing a completion receipt.

Rejected alternatives: waiting in the owner loop, allocating a pixel vector per streamed frame,
reusing storage merely because the session stopped, capturing an externally owned KMS image without
usage/ownership qualification, or treating GPU completion as consumer release of a delivery buffer.

## Derived verification

- Portable tests reject invalid/overflowing extents and preserve source incarnation semantics.
- A compile-only opt-in hardware test must render two different frames through the same staging
  allocation, verify pixels, reject reuse while pending and reject a receipt from another frame.
- Cancellation/discard must permit reuse only after completion, without exposing cancelled pixels.
- Actual Vulkan validation, noncoherent memory, device-loss and delivery-consumer tests remain
  hardware qualification work. This audit does not certify them or the later DMA-BUF path.

## Capture cursor scope

The embedded cursor path reuses ShellComposition image updates, VulkanScene resource ownership,
and the existing capture submission resource pins. It adds no native allocation or synchronization
primitive and does not alter hardware cursor plane ownership. Capture geometry matches the existing
hardware cursor rounding at output scale. Off-screen source removal explicitly emits a retirement
frame even without pixel damage, so a reappearing source cannot reuse a stale GPU scene epoch.
Portable tests cover these transitions; this is not hardware cursor or DMA-BUF qualification.

## PipeWire resize ownership review

Reviewed on 2026-09-18: PipeWire's official
[video-src-reneg.c example](https://docs.pipewire.org/video-src-reneg_8c-example.html)
(documentation version 1.6.8), especially `on_reneg_timeout`, `on_stream_param_changed`,
`on_stream_add_buffer` and `on_stream_remove_buffer`; and xdg-desktop-portal-wlr's
[`src/screencast/pipewire_screencast.c`](https://github.com/emersion/xdg-desktop-portal-wlr/blob/master/src/screencast/pipewire_screencast.c),
particularly format negotiation, add/remove callbacks and current-buffer invalidation. The latter
was read from upstream master and is not claimed as a pinned revision. Cross-checked the official
[stream API](https://docs.pipewire.org/group__pw__stream.html) for parameter updates and callback
ownership, and the installed pipewire-rs 0.10.1 callback signatures. No source was copied.

Both implementations separate format selection from buffer allocation/removal. Telorgon updates
EnumFormat on its existing stream and tracks opaque buffer addresses with generation identities.
A matching format event alone never releases the old generation's reservation. Old buffer removal
and GPU completion are independent requirements; late GPU results are discarded by the owner.
The producer has a single pending replacement slot, and each generation has at most three delivery
buffers. The same-size-in-bytes case still requires a new generation. Callback state is unborrowed
across native parameter updates and buffer queue calls. Existing Vulkan capture allocation and
receipt retirement contracts are reused without changing native GPU synchronization.

Rejected: replacing the PipeWire node after portal Start, accepting buffers by byte length alone,
releasing old memory on format acknowledgement, or allowing an unbounded resize command queue.
Initial portal readiness remains distinct from format readiness: clients need the published node
before they can connect and negotiate. During resizing that node stays available to the portal.
Tests cover delayed buffer removal, equal-byte-count layouts, stale/duplicate events, bounded
replacement admission and generation acknowledgement. Live graph renegotiation and application
behavior remain user-run qualification gates.

## Isolated window capture assembly

The window path reuses the audited capture target, scene-image ownership and submission receipts.
It selects raw Surface image scenes before desktop motion/glass substitution, preserves their
rectangular client clips and excludes decoration scenes and unverified X11 override-redirect
associations. Popup/subsurface parent walks are bounded; separate toplevels cannot enter a grant
through parent metadata. Checked physical bounds and coordinate subtraction reject overflow.
The existing capture target clears to opaque black before rendering these placements.

Source permission uses both the desktop WindowId and a capture availability epoch. Inspection of
`window_identity.rs` showed that desktop identity intentionally survives unmapping, so it cannot
alone identify a chooser entry's mapping lifetime. Withdrawal/minimize invalidates the capture
epoch without changing desktop identity semantics. Discovery snapshots carry their original epoch;
reading a newer registry epoch must not relabel an old snapshot as current. Tests exercise this
transition and retirement of same-label UI controls.

Captured revisions reuse `surface_occluded_frame_ready` after successful GPU completion and pixel
delivery. That existing API advances frame callbacks without sending KMS presentation feedback.
No new release/fence operation was introduced. Headless composition tests establish source-list
isolation; actual pixels, off-screen animation pacing, external-image lifetime and driver behavior
still require the user-run qualification described in `packaging/portal/README.md`.

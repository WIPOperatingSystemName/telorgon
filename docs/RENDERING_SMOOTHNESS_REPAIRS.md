# Rendering smoothness repairs

Status: implemented and CPU-tested; GPU execution and live Firefox/KMS performance remain
user-run qualification. This change addresses the September 13 follow-up review of the current
DMA-BUF rendering rework, rather than the historical synchronous-per-frame allocation path.

## Implemented changes

1. **Independent readiness.** A missing texture no longer prevents primary composition. The
   renderer prepares eligible surfaces independently and leaves unready pixel updates in each
   window's mailbox. Desktop composition retains the previous image extent and displayed revision;
   placement changes can continue using that image. A first unready image has no output placement.
   Old geometry can therefore be temporarily scaled into the latest placement, without claiming the
   new client revision was presented. Parent/child window families share a conservative readiness
   gate; unrelated roots remain independent.
2. **Acquire admission.** Each pending DMA-BUF owns a one-shot libwayland FD watch. Zero-timeout
   readiness checks and event-loop wakeups replace submitting unready producer work into the
   shared desktop batch. Watches own duplicated descriptors and stable callback data, unregister
   before either is freed, and are removed when superseded, ready or withdrawn. Existing imported
   GPU acquire waits, barriers and release fences remain intact. Allocation completion also requests
   a repaint, including when the preceding composition had no visible changes.
3. **Visible-generation materialization.** The backend materializes only pending revisions selected
   for output placements. Veiled, minimized and fully clipped content retains its pending update.
   Hidden ordinary window and chrome scenes remain retained even when absent from stacking order.
   Replacement and withdrawal continue through existing unsubmitted-buffer retirement accounting.
4. **End-to-end damage.** External-image updates now carry raster-space damage into the retained
   scene, logical placement mapping and physical output damage. Compatibility checks include
   transform, scale, alpha, geometry and contiguous history. Same-size transform changes still
   invalidate the whole image. Allocation reassignment/new image metadata also requires a full copy;
   the existing scanout-buffer-age damage accumulation remains in use.
5. **Early preparation culling.** Placement intersection with output damage and rectangular clipping
   is checked before scene validation, upload preparation and descriptor allocation. Culled scenes
   retain their pending uploads for a later exposure. This does not add opaque-region occlusion
   culling or weaken transparent/rounded-clip rendering.
6. **Deferred texture retirement.** Spare eviction transfers ownership to a retirement worker rather
   than dropping the final image on the input owner. The worker retains a target pin until scene
   and submission pins end, then runs image destruction and allocator freeing. Its transfer channel
   holds 64 entries; saturation retains resources in an owner backlog without waiting on the
   allocator. Backlog/held allocations remain charged to the device's memory budget. The worker
   sleeps indefinitely when empty and periodically checks outstanding pins when needed. Shutdown
   joins GPU completion and drops scene pins before joining retirement.
7. **Staging and chrome churn.** Composite staging appends only each pass's suffix while maintaining
   absolute, aligned GPU offsets; it no longer allocates/zeroes previous passes' prefixes. Protocol
   icon pixels are borrowed until their revision changes. Unchanged chrome snapshots are reused;
   model, size, state, runtime and animation changes invalidate the shortcut. The redundant second
   frame preparation call was removed.
8. **Cumulative descriptors.** Each owned frame slot has a reusable descriptor arena. It accounts for
   placements and textures across all passes and moves to another pool page before capacity is
   exceeded. Pages are retained with the slot and reset only after its completion proof. Each page
   keeps the existing 256-placement/2,048-texture limits; a frame is bounded to 64 pages. Exceeding
   that explicit bound returns an error rather than silently overrunning a pool.

Allocation requests remain bounded to one in progress. Pending surface generations coalesce to the
latest publication; an allocation already executing inside the driver is allowed to finish, and a
stale-size result can become a spare. It does not hold up unrelated desktop composition. Resource
import misses, new scene buffers and descriptor-page growth still have owner-side costs; eliminating
every driver call from the owner is outside this repair.

## Regression evidence

CPU regressions exercise delayed and ready producers independently; one-shot watch wakeup/removal;
family admission; old-image movement while a resized generation is unready; first-image suppression;
partial output damage at 300%; same-size transform/history invalidation; damage/clip culling;
absolute staging offsets without prefixes; cumulative descriptor-page limits; and final-pin
destruction on the retirement thread.

The opt-in hardware regression
`composite_culling_and_descriptor_pages_preserve_multipass_uploads` first culls a cold scene, then
records 257 visible passes in one owned frame. It checks that the first visible pass uploads the
previously deferred data, later passes retain it, descriptor allocation crosses a page boundary,
and final readback is correct with zero Vulkan validation errors. It is compiled, not executed,
during automated work in this checkout.

Validation completed on September 13:

- Embedded-XWayland/software library suite: **1,296 passed, 0 failed, 2 ignored**.
- Native-only `shell-wayland-linux` compilation: passed.
- Instrumented Vulkan and Linux DMA-BUF hardware test-binary compilation: passed; not executed.
- Consuming `test-compositor` optimized build with embedded XWayland: passed.

Tests requiring local Unix sockets ran outside the socket-restricted sandbox; this was not an
interactive compositor run. Historical latency captures predate these repairs and establish no
measured gain.

## Reference and ownership audit

The adjacent `../other-rendering-libs` source library is unavailable. Relevant upstream source was
inspected instead; no reference project was modified or added as a dependency:

- [Smithay `anvil/src/shell/mod.rs`](https://raw.githubusercontent.com/Smithay/smithay/master/anvil/src/shell/mod.rs),
  `new_surface` pre-commit hooks: producer readiness blockers wake through event sources before a
  generation is admitted. Telorgon keeps its existing retained-image and protocol owners.
- [wlroots `types/scene/wlr_scene.c`](https://raw.githubusercontent.com/swaywm/wlroots/master/types/scene/wlr_scene.c),
  disabled-node traversal and `render_texture`: retained lifetime differs from visible drawing;
  damage is intersected with destination coverage. This is the inspected upstream snapshot, not a
  claim that every wlroots backend has the same implementation.
- [wgpu `wgpu-core/src/device/life.rs`](https://raw.githubusercontent.com/gfx-rs/wgpu/trunk/wgpu-core/src/device/life.rs),
  `ActiveSubmission` and `triage_submissions`: command resources remain pinned through completion,
  and releasing retained resources can itself be expensive. Moving cleanup must preserve that proof.
- [Vulkan queue-submit semantics](https://docs.vulkan.org/refpages/latest/refpages/source/vkQueueSubmit2.html)
  explain why a producer wait affects other fragment work in the same batch. Readiness admission
  supplements rather than removes the acquire dependency.
- [Vulkan descriptor-pool reset](https://docs.vulkan.org/refpages/latest/refpages/source/vkResetDescriptorPool.html)
  requires submitted uses to finish before reset. All arena pages follow the existing frame-slot
  completion boundary.

Rejected shortcuts: sampling before acquire readiness, resetting a pool between passes in the same
command buffer, freeing a pinned texture, returning a new client revision while showing old pixels,
dropping hidden pending updates, or removing the retained copy without a new producer ownership
contract. The CPU and hardware regressions above derive from these invariants.

## Remaining qualification

Use the normal optimized compositor build for a fresh Firefox X11 capture. Separately exercise
unchanged-content dragging, animated Firefox, resize/maximize/restore, minimize/restore, and one
resizing client beside another animated client. Match scale, viewport and refresh rate when comparing
Ubuntu. Measure owner phase time, producer readiness, allocation misses, updated pixel area and
presentation gaps; keep browser/compositor clock domains separate unless explicitly synchronized.
The documented X11 deferred-resize preview policy remains unchanged.

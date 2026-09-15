# XWayland correctness repairs

This change repairs the six concrete defects from the XWayland review and removes several
avoidable owner-loop allocations. It does **not** complete the broader feature implementation plan.
Clipboard/PRIMARY/Xdnd integration, expanded ICCCM/EWMH policy, physical multi-output scheduling,
dynamic output reconstruction and DRM syncobj support remain open.

## Implemented behavior

1. **Publication ownership.** The core records each superseded surface revision and buffer use
   separately from the latest image action. The host retires unsubmitted acquire/release state and
   sends a storage release only when neither the final publication nor a queued SHM/GPU use retains
   that buffer. Publication coalescing preserves the final action's position relative to unrelated
   actions. It appends without repeatedly scanning the batch; final positions filter the drain.
   Committed explicit releases survive destruction of their wl_surface and finish through normal
   renderer/discard completion. Destroying a surface synchronization object preserves independent
   release objects, as required by the protocol.
2. **Damage continuity.** The SHM partial-copy path requires the incoming revision to immediately
   follow the retained image revision. A skipped, duplicate or wrapped revision requires a full
   image copy, including commits whose last damage region is empty. Existing DMA-BUF continuity
   and queue-coalesced damage checks remain in force.
3. **Density.** DMA-BUF materialization and transformed SHM images use output scale divided by the
   dedicated X11 client's coordinate density. At 300%, a 900×600 X11 surface materializes to 900×600,
   instead of 2700×1800. Native Wayland's coordinate density remains one. Direct SHM images keep
   their existing original-pixel fast path. Fractional density, transform, viewport and damage
   calculations retain their separate surface-coordinate and raster-coordinate meanings.
4. **Focus.** Clicking the existing X11 keyboard owner, or its Wayland child, retains its seat focus
   and enter serial. Raising its image family uses a separate checked XWM raise operation when
   needed. A genuine focus change still uses timestamp and checked ICCCM focus completion.
5. **Configure replies.** Denied/no-op requests receive synthetic ConfigureNotify with actual
   geometry and the most recently requested border width. Replies wait for outstanding checked
   commands, without waiting for repaint acknowledgement. Geometry requests also progress before
   a Wayland image is associated, so a client can wait for negotiation before allocating that image.
   Client-requested stacking remains denied by desktop click-to-raise policy; its geometry reply
   still completes. Saturated command queues defer configuration instead of failing compatibility.
6. **Association churn.** Live association limits remain bounded independently of historical IDs.
   Consecutive retired serials and surface IDs compress into lossless intervals, preserving replay
   rejection, delayed-event handling and XID incarnation checks. A regression covers 100,000 window
   lifetimes with live capacity two. Sparse adversarial histories still have a separate 1,048,576
   interval bound; no IDs are silently forgotten or accepted through an approximate filter.
7. **Owner-loop work.** Unchanged event sources persist across dispatch; only changed interest masks
   are updated. Sources are removed during teardown before callback data is destroyed. Titles are
   copied only when changed, and the live identity set is moved instead of cloned. No GPU copy or
   synchronization mechanism is bypassed, and no measured latency improvement is claimed.

## Reference audit

The adjacent `../other-rendering-libs` checkout is absent. The review used upstream sources and
specifications; no reference code or new dependency was copied into Telorgon.

- [wlroots types/wlr_surface.c](https://raw.githubusercontent.com/swaywm/wlroots/master/types/wlr_surface.c):
  examined pending/current state replacement, buffer unlocking and damage propagation. Buffer
  ownership must survive state replacement independently of the selected visible image.
- [Smithay src/wayland/compositor/handlers.rs](https://raw.githubusercontent.com/Smithay/smithay/master/src/wayland/compositor/handlers.rs):
  examined SurfaceAttributes commit/merge, buffer replacement, damage accumulation, callbacks and
  destruction. Cache/image coalescing does not cancel individual lifetime obligations.
- [Wayland core specification](https://wayland.freedesktop.org/docs/html/apa.html): buffer release,
  double-buffered surface state and synchronized subsurface cache semantics.
- [Explicit synchronization protocol source](https://raw.githubusercontent.com/wayland-mirror/wayland-protocols/main/unstable/linux-explicit-synchronization/linux-explicit-synchronization-unstable-v1.xml):
  a release belongs to one commit and its object is independent of the synchronization object.
- [Vulkan synchronization specification](https://docs.vulkan.org/spec/latest/chapters/synchronization.html):
  submitted image uses continue to retire through existing completion and foreign-image ownership
  paths. Only uses never submitted may be immediately discarded.
- [ICCCM, section 4.1.5](https://xorg.freedesktop.org/archive/X11R7.7/doc/xorg-docs/icccm/icccm.html):
  synthetic geometry notifications and requested border coordinates.
- Local pinned Xwayland 24.1.13 `hw/xwayland/xwayland-screen.c` and `xwayland-input.c`:
  rootless integer-density handling and keyboard enter/leave behavior informed the density and
  focus boundaries. The payload was not modified by this repair.

Rejected alternatives: forgetting historical IDs after 4096 lifetimes; releasing a buffer still
retained by another pending use; clearing committed release objects on surface destruction;
patching only the newest damage region after a revision gap; dividing buffer_scale a second time;
restarting the focus handshake on every click; and sending speculative geometry before checked
server configuration completes.

## Verification and remaining work

CPU and local-socket regressions cover publication replacement/destruction/reuse/action order,
release object lifetime, contiguous SHM damage, fractional/integer density, repeated focus, hidden
and cyclic ancestry, configure denial/border persistence/pre-association replies, long association
churn and replay protection. The discovery fixture now drives the owning protocol state machine
while flushing, preserving replies that can arrive when a dispatch budget yields.

Live rendering, GPU bandwidth, focus latency and multi-client interoperability are not qualified
by these tests. Repository AGENTS.md reserves interactive applications and hardware-presenting runs
for the user. Existing GPU tests are compile-only during this repair.

The larger plan still requires:

- Connecting XFixes ownership, conversion, INCR and bounded transfer components to native clipboard
  endpoints; adding native PRIMARY endpoints; implementing bidirectional Xdnd negotiation and
  cancellation. Existing transfer components alone do not constitute those features.
- Initial EWMH state, independent maximize axes, X11 fullscreen, ICCCM iconify/restore and transient
  policy, followed by accurate advertised capability updates.
- Per-output render targets, KMS scheduling, input/scene output mapping, hotplug reconstruction and
  dynamic density changes using the existing transactional output snapshot.
- DRM syncobj protocol validation, timeline import, deferred acquire-point availability and release
  signaling, with capability-gated advertisement and the current fence fallback.
- Further native synchronized-subsurface cache ownership work: cached replacement and independent
  explicit-sync/viewport state need dedicated transaction coverage. X11 rootless publications use
  the ordinary surface path repaired here.
- User-run real-client/GPU qualification and profiling before making throughput or latency claims.

Automated results for this repair:

- Embedded-payload library suite: **1,247 passed, 0 failed, 2 ignored**, serial execution.
- After the final unmapped-window configure correction: **130 desktop regressions passed**.
- Native-only `shell-wayland-linux` library check passed.
- Linux DMA-BUF and Vulkan hardware test binaries compiled with `application-software` and
  `shell-xwayland` enabled (the required test bodies were included); neither was executed.
- The consuming `test-compositor` optimized release build passed offline with the real
  `/home/aku/CompositorStuff/xwayland.payload` embedded.
- Rust formatting and diff whitespace checks passed. Unit socket fixtures required execution
  outside the socket-restricted sandbox; no desktop, browser or hardware workload was launched.

For user-run acceptance, check 100/150/200/300% raster sizes and small text, type while repeatedly
clicking the focused X11 client, maximize then submit client geometry requests, unmap/configure/remap,
and create/destroy more than 4096 windows over one private-server lifetime. Compare profiling
captures under identical output scale, window sizes, refresh rate and client workload. Existing
retained-image hardware fixtures cover GPU ownership and partial redraw; the new CPU density tests
do not establish driver behavior or image quality on physical outputs.

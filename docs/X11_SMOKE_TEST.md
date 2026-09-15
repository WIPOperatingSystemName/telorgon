# Integrated X11 smoke test

The real patched Xwayland 24.1.13 payload was built on 2026-09-09 at
`/home/aku/CompositorStuff/xwayland.payload`. The consuming project's `start.sh`
uses that path by default and enables the embedded compatibility feature.
Build logs and the helper-test results are in `../xwayland-build/` relative to
the Telorgon repository. On 2026-09-09 the user reported that the fixture spawns
and is interactable, and subsequently confirmed the integrated frame/rendering.
The resize veil was missing and has now been connected. This is user-reported smoke evidence,
not completion of every test below.
**Do not execute an artifact built with `/tmp/telorgon-host-COMPILE-ONLY.payload`.**

## Build a real test artifact

For the normal development profile, run `./start.sh` from `test-compositor` using
your usual test-session procedure. To build a separate release artifact with the
real pinned archive produced by [the payload recipe](../packaging/xwayland/README.md):

```sh
TELORGON_XWAYLAND_PAYLOAD=/home/aku/CompositorStuff/xwayland.payload \
  cargo build --release \
  --manifest-path /home/aku/CompositorStuff/test-compositor/Cargo.toml \
  --features telorgon/desktop-xwayland-embedded \
  --target-dir /tmp/telorgon-x11-real-test
```

Run `/tmp/telorgon-x11-real-test/release/telorgon-compositor` using your normal
Telorgon test-seat/session procedure. It uses the consuming project's configured
Vulkan renderer. Do not replace an active production session for this test.
Repository guidance reserves interactive and hardware-presenting runs for the user.

## GPU acceleration

From the consuming project's usual test-session starting point, use:

```sh
cd /home/aku/CompositorStuff/test-compositor
TELORGON_WAYLAND_ERROR_LOG=1 TELORGON_PROFILE=release ./start.sh
```

Confirm startup reports `telorgon-dmabuf: v4 feedback enabled` with the matched DRM device and
nonzero format/modifier count. Xwayland stderr must not report `GBM Wayland interfaces not available`,
`Failed to initialize glamor`, or `falling back to sw`. Readiness alone does not prove acceleration.
Inside the session, `glxinfo -B` (when installed) should name the hardware renderer rather than
llvmpipe/softpipe. Launch a fresh X11 Firefox process (`MOZ_ENABLE_WAYLAND=0 firefox --no-remote`)
and check `about:support`: Window Protocol should be X11, Compositing should use WebRender,
and the graphics details must not indicate a software renderer. Close other instances using the
same profile before launching this fresh process.

Exercise scrolling, animation, resize and maximize. The existing `test-frame-pacing.sh` can capture
timings with the same environment variables. Test native Wayland clients and the software
compositor separately: the latter must keep SHM and must not advertise DMA-BUF v4. This check
qualifies a particular GPU/driver/session; unit tests do not establish live acceleration.

## Presentation and input

1. Press **Ctrl+T** to launch Foot through Telorgon's managed session. If pressed
   during Xwayland preparation, the launch should wait and then proceed.
2. Confirm the compositor reports `private server and XWM initialized on :N`.
   In Foot, run `printenv DISPLAY WAYLAND_DISPLAY`. Both should be present. Do not
   manually set DISPLAY or use a global toolkit-backend override: that would bypass
   the launch-readiness test.
3. The workstation has `/usr/bin/xmessage` (manual version 1.0.7). Run:

   ```sh
   /usr/bin/xmessage -xrm '*font: fixed' -geometry 500x200+80+80 \
     -buttons 'Close:0' -default Close \
     'Telorgon X11 test: text must be visible. Click this window, then press Return.'
   ```

4. Confirm legible text and a button appear. Click the text area, then press Return;
   the window should close and the command should exit with status zero. Repeat,
   this time clicking the Close button. This separately exercises keyboard and
   pointer delivery. The `-default`/Return behavior is documented in the installed
   xmessage manual.
5. Confirm the fixture uses the same custom frame as Foot, with its X11 title.
   Drag the titlebar normally (no modifier), then resize from edges/corners.
   While dragging, check that the same configured RGBA resize veil as Foot covers
   the content. On release, it should remain until the final-sized client content
   arrives, then reveal that content. A click/release without changing size must
   not leave a stuck veil. Repeat from top/left edges and with a rapid second resize.
   Check that content remains clickable at its new location. Maximize, restore,
   and close with the frame controls; compare appearance and behavior with Foot.
   X11 maximize and restore (including titlebar-drag restore) must show the RGBA
   veil until matching content arrives; late maximized content must not reveal a
   restoring window early.
   For a client advertising `_NET_WM_SYNC_REQUEST` in `WM_PROTOCOLS`, the veil also
   waits for its basic XSync repaint acknowledgement. The xmessage fixture need not
   implement this protocol and therefore cannot validate that handshake. A deliberately
   delayed acknowledgement should keep the veil visible; an absent acknowledgement
   should produce a content-free timeout diagnostic after one second and use the
   existing buffer-only checks. Verify Foot remains responsive throughout.
   Check minimize in a separate run (the existing desktop's local visibility policy
   applies; a complete X11 task-switcher/iconification contract is still outstanding).
   Alt+left-drag is no longer a compositor gesture. Override-redirect menus and
   tooltips must stay unframed. These frame/control changes await a live retest.
6. Launch two instances at different positions; alternate clicks between them and
   Foot. Check raising, text visibility, keyboard delivery, and that input does not
   continue reaching the previously focused client. With overlapping windows,
   hover a foreground resize border, then move into the exposed background window
   without clicking. Its client cursor must replace the resize cursor. Repeat with
   both Foot and the X11 fixture in front; keyboard focus must not move on hover.

## Making a fast X11 resize veil visible

The user confirmed that fresh-start root-cursor initialization restores cursor visibility
in glxgears. Temporary cursor tracing and the inheritance probe have been removed.
The XWM retains the checked root arrow initialization; application cursor overrides and
intentional hiding remain supported. Retest visibility after a fresh start without probes.

The inherited arrow now uses the configured Telorgon asset cursor (including tint and
hotspot), captured at startup. Restart and launch glxgears without a probe; compare the
arrow over content with the default arrow outside the window. Check click position and
leave/re-enter behavior. Application-specific cursors remain unchanged. This exports a
static first frame; composed component cursors and images larger than 256x256 use a
logged built-in fallback. Live X11 visual validation is still outstanding.

For an accelerated client regression, launch `glxgears` from the compositor's terminal.
Check the pointer stays visible over content, then resize from every edge and verify
animation resumes after the veil clears. Repeat maximize/restore and rapid consecutive
resizes while Foot remains usable. Close using the frame button: legacy clients without
WM_DELETE_WINDOW are disconnected, so a nonzero process exit is acceptable here.
At 300%, X11 now uses three pixels per logical desktop unit. Compare the cursor with
the native arrow and resize glxgears to a similar on-screen size as before: content
should have more detail. Check click alignment, every resize edge, maximize/restore,
and leave/re-enter. Fixed-pixel apps can initially appear smaller; Xft DPI resources
help supporting clients but do not make every legacy toolkit scale its controls.
This high-density path needs live validation, especially on fractional outputs.

Ordinary windows now prefer a 300x200 logical minimum, configurable with
`LinuxDesktopConfig::preferred_window_minimum`. Launch glxgears without a geometry
override: its fixed pixel request should be enlarged to at least that logical size,
unless client constraints or available space require an exception. Test shrinking
from every edge and verify the opposite edge stays anchored. Test a fixed-size dialog
and a terminal using resize increments: the dialog should retain its permitted size;
the terminal should land on a valid grid size. Standard capability-aware frames hide
secondary buttons when narrow and disable resize/maximize for fixed-size clients.
Fullscreen/maximized sizing is separate. This policy does not magnify legacy text or
controls; a per-window magnification control is not included.

For visual diagnosis, restart the consuming compositor with:

```sh
TELORGON_X11_PREVIEW_HOLD_MS=750 ./start.sh
```

This opt-in diagnostic holds each final resize/maximize/restore veil for at least
750 ms and logs its start/clear transitions. Client acknowledgement and buffer
checks still apply after the hold. The normal default is zero; values are capped
at 2,000 ms. Run `./start.sh` without this variable to return to normal timing.
If maximize/restore still show no veil with the diagnostic enabled, report the
matching `resize veil` lines from `compositor.log`; a fast client alone would no
longer explain the absence. This does not prove that xmessage supports resize sync.

### Unsynchronized resize completion

Normal `start.sh` uses immediate buffer-based completion once server geometry,
content size/revision, commands, and chrome agree. There is no artificial delay
or extra-update requirement. The 750 ms workaround was removed at the user's
request; the user accepts the intermittent visual artifact. Explicitly advertised
client resize acknowledgement still gates completion where supported.

The old `test-x11-resize-extra-update.sh` remains a compatibility entry point for
normal behavior. `test-x11-resize-delay.sh` is solely an opt-in diagnostic launcher
that intentionally adds a hold; do not use it for normal startup.

### Image/revision trace for intermittent flashes

Launch with `test-compositor/test-x11-resize-trace.sh` instead of the earlier
launcher. It enables `TELORGON_X11_RESIZE_TRACE=1` alongside the normal fallback.
The separate diagnostic hold override is disabled. Inside the compositor, run glxgears in one terminal
and `record-x11-resize-test.py` in another. Arm a trial, resize glxgears once, wait
one second after the veil clears, and label it good or bad. Reports preserve both
completion reasons and trace lines. Capture at least one of each outcome.

Each trace line identifies the resize, surface, event order, and elapsed microseconds.
Stages record incoming buffer ID/extent/revision, prepared-image update type,
veil removal, retained scene image/content version, DMA-BUF materialization buffer
and lease, successful Vulkan submission with retained binding extent/generation,
and presentation feedback. `render-request` links the host frame and scanout slot
to the submitted revision. A GPU submission is not proof of scanout; older
already-queued frames may complete after veil removal. The recorder compares the
first traced GPU submission after reveal, not the first presentation callback.

Tracing lasts up to five seconds before reveal and one second afterward, capped
at 1,000 events per capture and 64 tracked surfaces. It is disabled by default and
captures metadata only; matching revisions cannot prove that the pixels contain
a finished application redraw. Logging can perturb timing. The trace adds no
GPU readback, waits, or changes to image ownership. Inspected paths include
`client.rs:apply_surface_publication`, `scene.rs:ImageScene::synchronize`,
`renderer/vulkan.rs:prepare_dma_bufs`/`render`, and
`renderer_vulkan/scene.rs:bind_materialized_image`; the read-only binding query
reports the actual retained resource's extent and generation. The adjacent
reference library remains unavailable; this diagnostic preserves the previously
reviewed synchronization contract rather than introducing a new one.

Validation: all 108 desktop-host tests passed serially; a parallel run hit an
existing private-client allocation failure (107 passed). Recorder classification,
surface-ID matching, shell syntax, and a mocked launcher/interactive-report flow
passed. No compositor was launched by the agent. Pixel capture remains a possible
next diagnostic if revision/binding correlation does not explain a bad frame.

## Shutdown and failure containment

For an unexplained Firefox channel error or private Xwayland disconnect, launch
the consuming compositor with `test-compositor/start-x11-error-log.sh`, then run
`test-compositor/firefox-x11-log.sh` inside its terminal. The first enables
`TELORGON_WAYLAND_ERROR_LOG=1`: rejected dispatch requests log PID, object,
interface, opcode and request name, while posted protocol errors log their code
and escaped message. Global-bind failures also retain their error. Xwayland
stderr is escaped and forwarded up to 64 KiB per helper process; remaining output
is still drained. Normal launches retain the previous discard behavior.
The compositor output is in `compositor.log`; Firefox console/sandbox output is
in `firefox-x11.log`. Launchers overwrite those logs on their next invocation.
No request arguments, environment contents, or authentication cookies are added
to protocol logs. These diagnostics do not change protocol acceptance or repair
a disconnected client. The existing helper-output flood test exercises draining
with diagnostics enabled; live Firefox reproduction remains user-run.

With Foot and xmessage open, press **Ctrl+Q** (the consuming project's exit binding).
Ordinary X11 windows should participate in cooperative shutdown. A client that
refuses close must lead to shutdown cancellation at the configured timeout, not
forced application termination. Repeat with an application that has an actual
unsaved-work dialog before claiming that scenario passes.

For a separate helper-loss test, identify the actual compositor-owned Xwayland
child in the process tree. Terminate only that owned helper while keeping a native
window open. Verify native interaction survives, new managed terminals have no
DISPLAY, and affected managed apps are not automatically restarted. Do not use a
broad `pkill Xwayland`, claimed `_NET_WM_PID`, or an unrelated session's PID.

Record pass/fail for each step, the real payload hash, executable build revision,
GPU/driver/kernel, and compositor exit/signal diagnostics. No pixels, typed text,
clipboard content, credentials, or authority-cookie bytes are needed in the report.
This smoke test does not qualify acceleration, multiple outputs, clipboard/DnD,
Steam/Wine, Horizon, or release portability.

### Firefox decoration ownership

Launch Firefox inside Telorgon using the existing X11 launcher. In Firefox's Customize Toolbar
screen, toggle **Title Bar**. Client decorations should retain Telorgon's border/radius/colors but have no extra Telorgon title bar; enabling
the native title bar should restore Telorgon's chrome. Normal client content should retain its root
position, and clicking the removed title-bar area should no longer invoke compositor controls.
Repeat while maximized, then restore, and check at the configured X11 scale.

From a terminal inside Telorgon, run `xprop _MOTIF_WM_HINTS _NET_FRAME_EXTENTS` and select Firefox.
Client-decorated windows should report the measured outer border extents, with no added title-bar height. Decorated windows should
report left/right/top/bottom margins in X11 pixels (including the title bar in the top margin).
Fullscreen should report zeros. Firefox's own tabs/header are client content, not frame extents.

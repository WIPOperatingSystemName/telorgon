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
   Check minimize in a separate run (the existing desktop's local visibility policy
   applies; a complete X11 task-switcher/iconification contract is still outstanding).
   Alt+left-drag is no longer a compositor gesture. Override-redirect menus and
   tooltips must stay unframed. These frame/control changes await a live retest.
6. Launch two instances at different positions; alternate clicks between them and
   Foot. Check raising, text visibility, keyboard delivery, and that input does not
   continue reaching the previously focused client.

## Shutdown and failure containment

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

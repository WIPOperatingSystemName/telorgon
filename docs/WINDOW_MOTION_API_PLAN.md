# Window motion API and implementation plan

Status: original design plan. The [initial implementation](WINDOW_MOTION.md) now provides the API
and transitions, with snapshot scaling in place of the planned live-chrome relayout. This records
the agreed authoring API for animated
maximize/restore, minimize/unminimize, and content-to-resize-placeholder-to-ready-content fades.
[Implementation status](IMPLEMENTATION_STATUS.md) remains authoritative for working features.

## Decision: one motion object on the chrome design

Add `motion: WindowMotion` to `WindowChromeDesign`. Keep `WindowChromeStateStyle` focused on the
appearance of normal, maximized, tiled, and fullscreen windows. Programmers should configure a
coherent set of animations in one place, without repeating transitions in each state.

The common case is one field:

```rust,ignore
WindowChromeDesign {
    motion: WindowMotion::smooth(),
    // Existing appearance fields...
}
```

Customize only what differs from the preset:

```rust,ignore
WindowChromeDesign {
    motion: WindowMotion::smooth()
        .maximize(tween_ms(240, Easing::EaseOut))
        .minimize(Minimize::shrink_and_fade(180))
        .resize_content(ContentFade::new(90, 130)),
    // Existing appearance fields...
}
```

All names above are proposed API. Timing arguments are milliseconds; `tween_ms` returns a
non-repeating `WindowTween` using the existing `Easing`. Do not add a general `.ms()` extension
trait solely for these examples. `ContentFade::new` takes entry and exit durations, respectively.
The placeholder color stays in the existing appearance settings.

### Presets and overrides

- `WindowMotion::smooth()` supplies the complete coordinated effect set.
- `WindowMotion::none()` supplies immediate transitions for every effect.
- `WindowMotion::default()` equals `smooth()`.
- `.maximize(tween)` configures both maximize and unmaximize by default.
- `.restore(tween)` overrides only unmaximize, not restoration from minimized.
- `.minimize(effect)` configures minimize and its corresponding entrance effect.
- `.unminimize(effect)` overrides only the entrance from minimized.
- `.resize_content(fade)` configures the shared placeholder handoff for resize and maximize/restore.

Use private fields and consuming builder methods. Store paired defaults and optional directional
overrides separately: setting `.maximize(...)` must not clear an explicit `.restore(...)`, regardless
of builder order. Apply the same rule to minimize/unminimize. This avoids configuring both directions
just to get a complete result. A zero-duration tween is immediate; transitions cannot repeat.

Initial smooth timings: 240 ms maximize/restore, 180 ms minimize/unminimize, 90 ms entering the
placeholder and 130 ms revealing ready content. These are tuning defaults, not qualified UX results.
Start minimize with a small centered shrink and fade. Dock destinations and additional effect kinds
can be added later without changing the basic API.

## Ownership and integration

Place the neutral public motion types beside the window chrome contracts, for example in
`src/window_chrome/motion.rs`, re-exported from `window_chrome` and the existing authoring exports.
Reuse `Easing` and `MotionPreference`; do not make the window controller depend on mounted theme
slots or allow repeating theme tracks to govern finite window transitions.

Add an optional `WindowFrameTemplate::motion(&self, model: &WindowChromeModel) -> Option<WindowMotion>`
method with a default of `None`. The easy template returns `Some(design.motion)`. Forward it through
`WindowFrameFactory` alongside `content_style`. `None` inherits the desktop default; an explicit
`Some(WindowMotion::none())` disables effects. This also gives custom frames the same functionality
without requiring them to use the easy-frame appearance structs.

The desktop owns the fallback for windows without a motion-providing template, including
client-decorated windows. Expose the same value type there; do not create a second animation DSL.
Initially preserve existing fallback behavior with `WindowMotion::none()`. Existing in-repository
design literals receive an explicit `none()` during migration; opt the example compositor into
`smooth()` once rendering support is complete. Adding a public struct field requires downstream
literal updates and a migration note.

Chrome declares motion; a compositor-owned `WindowTransitionController` executes it. It consumes
accepted window changes, matching content-publication events, and caller-supplied monotonic time.
It produces sampled geometry, visibility, and content-mix values. GPU resources remain renderer-owned.
There are no public animation handles, per-frame callbacks, timers, or protocol serials in the
programmer-facing style API. Native hosted windows remain subject to the outer OS compositor;
this feature initially governs windows composed by Telorgon's desktop host.

## Runtime behavior

Geometry and content readiness are independent tracks:

1. Resize entry freezes the outgoing visible content and fades it into the placeholder. Interactive
   geometry follows the pointer immediately, without easing lag. Keep outgoing content at its native
   extent with anchored clipping during the short entry fade; the placeholder covers the target slot.
2. Release uses the existing final-size request and readiness rules. The placeholder remains until
   matching content is publishable; elapsed time never establishes readiness.
3. Ready content fades in. If it arrives during entry, retarget the current content mix without an
   obligatory full-placeholder frame. Keep a bounded representation of the interrupted mix.
4. Maximize/restore resolves authoritative destination layout and sends the necessary configuration
   through existing policy. Only presentation geometry is sampled during motion; it does not emit a
   configure per animation frame. Animate frame layout so text and borders remain crisp. Final chrome
   measurement and its configure may supersede an initial estimate, as today.
5. Minimize immediately changes semantic/input eligibility, while a separately retained visual exits
   at its original stack location. Restoring reverses from the sampled appearance. Unmap, destruction,
   and session lock cancel visuals and retire resources safely.

Every content handoff carries a generation, so stale acknowledgements/publications cannot satisfy a
new resize. Wayland and X11 retain their distinct readiness adapters. A minimized or veiled window
must not regain input merely because a retained image is visible. Route chrome input using displayed
geometry; suppress client input during the placeholder handoff. Subsurfaces follow content clips;
popup visibility remains an explicit window-family policy, preserving current resize suppression.

Retarget from the current sampled values. Reduced motion settles visual tracks immediately while
preserving readiness gates. Sample from the existing frame clock, damage old and new visual bounds
including shadows and opacity-only changes, and stop requesting frames when only client readiness
is pending. Presentation feedback continues to describe the revisions actually displayed; replaying
an outgoing snapshot does not manufacture presentation of a newer client commit.

## Renderer contract and implementation gates

Introduce bounded composition grouping with transform and group opacity, plus a content crossfade.
Fading each overlapping primitive independently is not equivalent to fading the completed window.
Similarly, source-over drawing two opaque endpoints at complementary opacity exposes background at
the midpoint. Define content crossfade as linear interpolation of premultiplied endpoint RGBA,
then composite that result over the desktop. Alpha-zero placeholders must reveal lower layers once
entry finishes, without retained client pixels or content backing beneath them.

Allow direct composition only where equivalent. Otherwise use a bounded temporary target for the
window/content group. Vulkan must produce its own targets, without software rendering or CPU
readback. Retain outgoing content immutably: an image handle alone does not prevent external producer
reuse. The implementation audit must resolve snapshot capture, acquire/release synchronization,
completion-based retirement, memory bounds, allocation failure, and cancellation before GPU coding.
When capture is unavailable, skip the affected outgoing animation while preserving readiness and
visibility correctness. Do not publish an operational smooth preset backed by inert configuration.

## Delivery sequence and acceptance

1. **Style contract and controller:** implement presets, builders, template forwarding, exports, and
   a pure controller. Test paired inheritance, builder-order independence, explicit disabling,
   zero duration, interruption, stale generations, early/late readiness, and reduced motion.
2. **Composition support:** finish the resource/reference audit, then implement Vulkan and software
   group opacity/crossfade semantics. Test overlapping children, translucent clients, placeholder
   alpha 0/128/255, clipping, shadow bounds, unchanged-geometry opacity damage, and resource retirement.
3. **Content handoffs:** integrate both readiness adapters, retain the existing deferred-copy and
   callback pacing policy, and verify new drags supersede pending fades without stale content.
4. **Window motion:** integrate maximize/restore and minimize/unminimize. Verify no animation-driven
   configure stream, retained exit visuals, aligned input, cancellation, and no idle redraw loop.
5. **Authoring and qualification:** opt the consuming example into the one-line preset, update
   documentation/status, and run CPU tests and compile checks. User-run visual qualification covers
   rapid reversals, slow clients, transparent windows, subsurfaces/popups, X11/Wayland, and reduced
   motion. Repository rules prohibit agent-run GUI/compositor and GPU-presenting applications.

## Reference audit and rejected alternatives

Local code inspected: `compose/components/easy_window_frame.rs`, `window_chrome.rs`,
`application_host/declaration.rs`, `application_host/desktop_wayland/{state,interaction,layers,scene}.rs`,
the desktop software adapter, `renderer_vulkan/composite.rs`, and `theme/{motion,processor}.rs`.
Existing behavior is documented in [resize preview](WAYLAND_RESIZE_PREVIEW.md) and
[maximized geometry](MAXIMIZED_WINDOW_GEOMETRY.md).

The adjacent `../other-rendering-libs` library is unavailable. Upstream Flutter
[`engine/src/flutter/flow/layers/opacity_layer.cc`](https://github.com/flutter/flutter/blob/master/engine/src/flutter/flow/layers/opacity_layer.cc)
was inspected for opacity inheritance, cache use, and opacity/transform damage. Attempts to fetch
Android's `libs/WindowManager/Shell/src/com/android/wm/shell/windowdecor/ResizeVeil.kt` and a second
independent scene implementation failed. The required two-implementation comparison is incomplete;
this document authorizes no unreviewed GPU lifetime mechanism. The official
[Vulkan blending contract](https://docs.vulkan.org/spec/latest/chapters/framebuffer.html#framebuffer-blending)
was checked for the blend-semantics distinction above. No external source was copied.

Extracted invariants become the alpha, damage, readiness, and lifetime acceptance tests above.
Rejected alternatives: per-state animation fields scatter paired behavior; desktop-only configuration
makes window design incomplete; a general scripting API adds unnecessary authoring and lifecycle
machinery; timer-based readiness reveals stale content; independent child fades break group alpha;
and per-frame client resizing couples animation smoothness to application redraw speed.

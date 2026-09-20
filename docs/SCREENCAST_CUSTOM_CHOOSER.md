# Custom screen-sharing chooser

The default is now the visual `CapturePicker`; see [picker implementation and limits](SCREENCAST_PICKER.md).
It can also be installed explicitly with `.capture_chooser(telorgon::CapturePicker::new)`.
Custom widgets may provide `ShellWidget::output_previews()` and `window_previews()` slots.
`CaptureUiSnapshot::starting` distinguishes pending stream startup; `failure` supplies a dismissible
error notification (`CaptureUi::dismiss_failure`).

Enable `shell-screencast-linux`, opt in with `.capture(Capture::desktop())` on the shell environment,
and configure `Compositor::capture_chooser` with an owner-thread
factory returning a `ShellWidget`:

```rust,ignore
Compositor::new()
    .capture_chooser(|capture: telorgon::CaptureUi| MyChooser::new(capture))
    .cursor_theme(my_cursor_theme)
```

The factory runs once when the managed Vulkan host assembles capture controls. Declaring the
compositor does not run the factory or start services. The default chooser is replaced; the standard
persistent sharing indicator and stop buttons remain installed. A default-constructed `CaptureUi`
is inert and cannot authorize anything. Ordinary and embedded builds do not acquire this API's
PipeWire dependencies unless the screencast feature is enabled.

A custom component stores the supplied `CaptureUi` as an input and watches its signal:

```rust,ignore
let snapshot = self.watch(self.capture.snapshot());
let mut choices = column();
if let Some((request, _)) = &snapshot.pending {
    let request = *request;
    for (source, epoch, source_label) in &snapshot.sources {
        let (source, epoch) = (*source, *epoch);
        choices = choices.child(
            button(source_label.clone())
                .on_press(move |this: &mut Self| {
                    let _ = this.capture.approve(request, source, epoch);
                })
                .key(format!("{request}:{source:?}:{epoch}")),
        );
    }
}
choices
```

The second fragment shows the data flow, not a complete widget: call `approve` only from an
explicit user activation handler, never while rendering `view`. Treat application/source labels as
presentation text. Preserve request IDs, source identities and mapping epochs unchanged. In
particular, a WindowId can survive unmap/remap while its capture epoch changes. Replace the old
control and any armed activation when its request/source/epoch changes.

`approve(request, source, epoch)`, `deny(request)` and `stop(sharing_id)` return whether the decision
was queued. A `true` return does not mean permission was granted. A full or closed queue returns
`false` without waking the host. The host independently checks the current request, requested
source types, mapping epoch, source availability, lock state and capture limits. It resolves the
source and owns all GPU/PipeWire resources. Custom components cannot supply pixels or substitute a
source after approval.

Implement `ShellWidget::surface` so the chooser is visible only while `snapshot.pending` is present.
Use an overlay, modal pointer policy and focus-on-open behavior, and submit `deny` on Escape/dismissal.
Keep every choice reachable with paging or another bounded layout. The default implementation in
`crates/telorgon/src/shell_components/capture/` demonstrates source keys, paging, dismissal and
request replacement. `snapshot.sharing` supplies IDs accepted by `stop` if the desktop also wants
its own sharing controls.

Validation: the public integration fixture `tests/capture_chooser_api.rs` compiles a downstream
component and confirms declaration starts no backend. Unit tests verify deferred one-shot factory
invocation, factory preservation across cursor-theme configuration, queue backpressure, inert
handles and stale chooser activation retirement. These do not qualify a custom widget's actual
focus, accessibility or consent behavior; exercise those in the compositor session.

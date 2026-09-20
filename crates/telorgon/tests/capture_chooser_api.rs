#![cfg(all(feature = "shell-screencast-linux", target_os = "linux"))]
use telorgon::app::*;
use telorgon::{CaptureUi, ShellSurfaceSpec, ShellWidget};

#[component]
struct CustomChooser {
    #[input]
    capture: CaptureUi,
}
impl Component for CustomChooser {
    fn view(&self) -> impl View {
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
            choices = choices.child(button("Cancel capture").on_press(move |this: &mut Self| {
                let _ = this.capture.deny(request);
            }));
        }
        choices
    }
}
impl ShellWidget for CustomChooser {
    fn surface(&self) -> ShellSurfaceSpec {
        ShellSurfaceSpec::new().visible(self.capture.snapshot().snapshot().pending.is_some())
    }
}
#[test]
fn public_custom_chooser_declaration_starts_no_backend() {
    let called = std::rc::Rc::new(std::cell::Cell::new(false));
    let invocation = called.clone();
    let _compositor = Compositor::new()
        .capture_chooser(move |capture| {
            invocation.set(true);
            CustomChooser { capture }
        })
        .cursor_theme(telorgon::CursorTheme::new());
    assert!(!called.get());
    let inert = CaptureUi::default();
    assert!(inert.snapshot().snapshot().pending.is_none());
    assert!(!inert.stop(1));
}

#[test]
fn visual_picker_and_output_preview_are_public_declarations() {
    use telorgon::{CapturePicker, ShellOutputPreview};
    let _compositor = Compositor::new().capture_chooser(CapturePicker::new);
    let preview = ShellOutputPreview::new(
        telorgon::shell::OutputId::MIN,
        telorgon::RectF {
            x: 12.0,
            y: 12.0,
            width: 240.0,
            height: 135.0,
        },
    );
    assert_eq!(preview.output, telorgon::shell::OutputId::MIN);
    let snapshot = telorgon::CaptureUiSnapshot::default();
    assert!(snapshot.starting.is_empty());
    assert!(snapshot.failure.is_none());
    assert!(!CaptureUi::default().dismiss_failure());
}

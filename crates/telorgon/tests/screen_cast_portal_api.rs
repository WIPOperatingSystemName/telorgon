#![cfg(all(feature = "shell-screencast-linux", target_os = "linux"))]
use telorgon::app::*;
use telorgon::{ScreenCastPortalContext, ShellSurfaceSpec, ShellWidget};

#[component]
struct CustomChooser {
    #[input]
    capture: ScreenCastPortalContext,
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
                    button()
                        .child(
                            text(source_label.clone())
                                .color(telorgon::ColorRgba8::rgba(248, 249, 252, 255)),
                        )
                        .on_press(move |this: &mut Self| {
                            let _ = this.capture.approve(request, source, epoch);
                        })
                        .key(format!("{request}:{source:?}:{epoch}")),
                );
            }
            choices = choices.child(
                button()
                    .child(
                        text("Cancel capture")
                            .color(telorgon::ColorRgba8::rgba(248, 249, 252, 255)),
                    )
                    .on_press(move |this: &mut Self| {
                        let _ = this.capture.deny(request);
                    }),
            );
        }
        choices
    }
}
impl ShellWidget for CustomChooser {
    fn surface(&self) -> ShellSurfaceSpec {
        ShellSurfaceSpec::new().visible(self.capture.snapshot().snapshot().pending.is_some())
    }
}
#[component]
struct CustomSharing {
    #[input]
    capture: ScreenCastPortalContext,
}
impl Component for CustomSharing {
    fn view(&self) -> impl View {
        let snapshot = self.watch(self.capture.snapshot());
        let mut controls = column();
        for (id, label) in &snapshot.sharing {
            let id = *id;
            controls = controls.child(
                button()
                    .child(
                        text(format!("Stop {label}"))
                            .color(telorgon::ColorRgba8::rgba(248, 249, 252, 255)),
                    )
                    .on_press(move |this: &mut Self| {
                        this.capture.stop(id);
                    }),
            );
        }
        controls
    }
}
impl ShellWidget for CustomSharing {
    fn surface(&self) -> ShellSurfaceSpec {
        ShellSurfaceSpec::new().visible(!self.capture.snapshot().snapshot().sharing.is_empty())
    }
}

#[test]
fn public_portal_design_starts_no_backend() {
    let called = std::rc::Rc::new(std::cell::Cell::new(false));
    let invocation = called.clone();
    let _compositor = Compositor::new()
        .screen_cast_portal(
            ScreenCastPortal::new()
                .picker(move |capture| {
                    invocation.set(true);
                    CustomChooser { capture }
                })
                .sharing(|capture| CustomSharing { capture }),
        )
        .cursor_theme(telorgon::CursorTheme::new());
    assert!(!called.get());
    let inert = ScreenCastPortalContext::default();
    assert!(inert.snapshot().snapshot().pending.is_none());
    assert!(!inert.stop(1));
}

#[test]
fn output_preview_is_a_public_declaration() {
    use telorgon::ShellOutputPreview;
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
    let snapshot = telorgon::ScreenCastPortalSnapshot::default();
    assert!(snapshot.starting.is_empty());
    assert!(snapshot.failure.is_none());
    assert!(!ScreenCastPortalContext::default().dismiss_failure());
}

#[test]
fn saved_permission_controls_are_public_and_inert_without_host() {
    let ui = ScreenCastPortalContext::default();
    let summary = telorgon::SavedCapturePermission {
        app_id: "org.example.Recorder".into(),
        grants: 2,
    };
    assert!(!ui.refresh_saved_permissions());
    assert!(!ui.forget_saved_permissions(&summary.app_id));
    assert!(!ui.forget_saved_permissions(""));
}

use super::*;
use crate::{Background, ColorRgba8};

#[crate::component]
pub(crate) struct CaptureIndicator {
    #[input]
    ui: CaptureUi,
}
impl CaptureIndicator {
    pub fn new(ui: CaptureUi) -> Self {
        Self { ui }
    }
}
impl Component for CaptureIndicator {
    fn view(&self) -> impl View {
        let snapshot = self.watch(&self.ui.snapshot);
        let compact = self
            .try_context::<ShellContext>()
            .is_some_and(|c| c.output_size().width < 620.0);
        let mut content = column()
            .width(Dimension::Shrink)
            .height(Dimension::Shrink)
            .padding(10.0)
            .gap(8.0)
            .background(Background::Color(ColorRgba8::rgba(76, 28, 28, 255)));
        for (id, app) in &snapshot.sharing {
            let id = *id;
            let app = application_label(app);
            let app: String = app.chars().take(if compact { 16 } else { 40 }).collect();
            content = content.child(
                row()
                    .width(Dimension::Shrink)
                    .height(Dimension::Shrink)
                    .key(id)
                    .gap(14.0)
                    .child(
                        text(format!(
                            "{}: {app}",
                            if snapshot.starting.contains(&id) {
                                if compact {
                                    "Starting"
                                } else {
                                    "Starting screen sharing"
                                }
                            } else {
                                if compact { "Sharing" } else { "Screen sharing" }
                            }
                        ))
                        .size(14.0)
                        .color(ColorRgba8::rgba(255, 255, 255, 255)),
                    )
                    .child(button("Stop sharing").on_press(move |this: &mut Self| {
                        this.ui.decide(CaptureDecision::Stop(id))
                    })),
            );
        }
        if let Some(message) = &snapshot.failure {
            content = content.child(
                row()
                    .width(Dimension::Shrink)
                    .height(Dimension::Shrink)
                    .gap(12.0)
                    .child(
                        text(if compact {
                            "Sharing failed. Try again.".to_owned()
                        } else {
                            message.clone()
                        })
                        .size(14.0)
                        .color(ColorRgba8::rgba(255, 255, 255, 255)),
                    )
                    .child(button("Dismiss").on_press(|this: &mut Self| {
                        let _ = this.ui.dismiss_failure();
                    })),
            );
        }
        content
    }
}
impl ShellWidget for CaptureIndicator {
    fn surface(&self) -> ShellSurfaceSpec {
        ShellSurfaceSpec::new()
            .layer(ShellSurfaceLayer::Overlay)
            .order(i32::MAX - 1)
            .placement(WidgetPlacement::aligned(0.5, 0.0).margin(8.0))
            .visible(
                !self.ui.snapshot.snapshot().sharing.is_empty()
                    || self.ui.snapshot.snapshot().failure.is_some(),
            )
            .focus(ShellFocus::OnClick)
    }
}

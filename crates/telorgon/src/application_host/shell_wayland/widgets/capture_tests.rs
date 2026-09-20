//! Mounted CPU layout and reference-renderer tests; no display, portal or PipeWire is started.
use super::*;
use crate::shell_components::capture::{
    CaptureIndicator, CapturePicker, CaptureUi, CaptureUiSnapshot,
};
use std::sync::{Arc, mpsc::sync_channel};

fn mount<W: crate::ShellWidget>(widget: W, output: SizeI) -> WidgetLayer {
    let host = crate::compose::shell_services::ShellServiceHost::new();
    host.publish_output_size(SizeF {
        width: output.width as f32,
        height: output.height as f32,
    });
    WidgetLayer::new(
        42,
        crate::application_host::declaration::RegisteredShellWidget::new(widget),
        output,
        AssetBundle::default(),
        crate::platform::ScaleFactor::new(1.0).unwrap(),
        &EventNotifier::new("capture layout test").unwrap(),
        host.services.clone(),
    )
    .unwrap()
}
fn ui(snapshot: CaptureUiSnapshot) -> CaptureUi {
    let (snapshot, _) = crate::compose::Signal::new(snapshot);
    let (decisions, _) = sync_channel(16);
    CaptureUi {
        snapshot,
        decisions,
        wake: Arc::new(|| {}),
    }
}
#[test]
fn sharing_indicator_hugs_content_instead_of_filling_the_output() {
    for output in [
        SizeI {
            width: 1920,
            height: 1200,
        },
        SizeI {
            width: 800,
            height: 600,
        },
    ] {
        let mut widget = mount(
            CaptureIndicator::new(ui(CaptureUiSnapshot {
                sharing: vec![(1, "Discord".into())],
                ..Default::default()
            })),
            output,
        );
        widget
            .prepare(output, shell_work_area_for_spec(output), 1)
            .unwrap();
        assert!(widget.spec.visible);
        assert!(
            widget.sampled.width < 500,
            "indicator width {:?}",
            widget.sampled
        );
        assert!(
            widget.sampled.height < 90,
            "indicator height {:?}",
            widget.sampled
        );
        assert_eq!(widget.sampled.y, 8);
    }
}
#[test]
fn picker_controls_and_preview_slots_fit_real_layout() {
    for output in [
        SizeI {
            width: 1920,
            height: 1200,
        },
        SizeI {
            width: 800,
            height: 600,
        },
        SizeI {
            width: 360,
            height: 360,
        },
    ] {
        let mut widget = mount(
            CapturePicker::new(ui(CaptureUiSnapshot {
                pending: Some((1, "Discord".into())),
                sources: vec![(
                    crate::shell::capture::CaptureSource::Output(crate::shell::OutputId::MIN),
                    1,
                    "Built-in display".into(),
                )],
                ..Default::default()
            })),
            output,
        );
        widget
            .prepare(output, shell_work_area_for_spec(output), 1)
            .unwrap();
        let extent = widget.layer.runtime.extent();
        assert!(extent.width <= output.width as f32 && extent.height <= output.height as f32);
        for (node, semantic) in widget.layer.runtime.ui().semantics.iter() {
            if semantic.role != crate::SemanticRole::Button {
                continue;
            }
            let rect = widget
                .layer
                .runtime
                .layout()
                .computed(node)
                .unwrap()
                .border_rect;
            assert!(
                rect.x >= 0.0
                    && rect.y >= 0.0
                    && rect.right() <= extent.width + 0.1
                    && rect.bottom() <= extent.height + 0.1,
                "button outside panel: {rect:?}, {extent:?}"
            );
        }
        assert_eq!(widget.binding.3.borrow().len(), 1);
        let slot = widget.binding.3.borrow()[0].rect;
        assert!(slot.right() <= extent.width && slot.bottom() <= extent.height);
        if let Ok(directory) = std::env::var("TELORGON_CAPTURE_TEST_IMAGES") {
            render_panel(
                &mut widget,
                &format!("{directory}/picker-{}.png", output.width),
            );
        }
    }
}
fn render_panel(widget: &mut WidgetLayer, path: &str) {
    use crate::render::RenderBackend;
    use crate::renderer_software::{SoftwareCompositeLayer, SoftwareRenderer, SoftwareSurface};
    let renderer = SoftwareRenderer;
    let mut scene = renderer.create_scene().unwrap();
    for delta in widget.layer.take_deltas() {
        renderer.apply_scene_delta(&mut scene, &delta).unwrap();
    }
    let size = widget.layer.runtime.extent();
    let extent = SizeI {
        width: size.width as i32,
        height: size.height as i32,
    };
    let mut surface = SoftwareSurface::default();
    renderer
        .render_composite(
            &mut surface,
            &[SoftwareCompositeLayer {
                scene: &scene,
                target: RectI {
                    x: 0,
                    y: 0,
                    width: extent.width,
                    height: extent.height,
                },
                clip: None,
                rounded_clips: [None, None],
            }],
            extent,
            None,
            crate::ColorRgba8::rgba(0, 0, 0, 255),
        )
        .unwrap();
    image::save_buffer(
        path,
        surface.pixels_rgba8(),
        extent.width as u32,
        extent.height as u32,
        image::ColorType::Rgba8,
    )
    .unwrap();
}

#[test]
fn pointer_click_inside_thumbnail_selects_then_share_button_approves() {
    let (signal, _) = crate::compose::Signal::new(CaptureUiSnapshot {
        pending: Some((12, "Discord".into())),
        sources: vec![(
            crate::shell::capture::CaptureSource::Output(crate::shell::OutputId::MIN),
            7,
            "Screen 1".into(),
        )],
        ..Default::default()
    });
    let (decisions, receive) = sync_channel(16);
    let output = SizeI {
        width: 1920,
        height: 1200,
    };
    let mut widget = mount(
        CapturePicker::new(CaptureUi {
            snapshot: signal,
            decisions,
            wake: Arc::new(|| {}),
        }),
        output,
    );
    let slot = widget.binding.3.borrow()[0].rect;
    let now = MonotonicInstant::from_nanos(10);
    widget.layer.pointer_motion(
        PointF {
            x: slot.x + slot.width / 2.0,
            y: slot.y + slot.height / 2.0,
        },
        now,
    );
    widget.layer.pointer_button(true, now);
    widget.layer.pointer_button(false, now);
    widget
        .prepare(output, shell_work_area_for_spec(output), 11)
        .unwrap();
    assert!(receive.try_recv().is_err());
    let node = widget
        .layer
        .runtime
        .ui()
        .semantics
        .iter()
        .find_map(|(node, semantic)| {
            let crate::SemanticName::Text(label) = semantic.name else {
                return None;
            };
            (semantic.role == crate::SemanticRole::Button
                && widget.layer.runtime.ui().string(label) == Some("Share"))
            .then_some(node)
        })
        .unwrap();
    let button = widget
        .layer
        .runtime
        .layout()
        .computed(node)
        .unwrap()
        .border_rect;
    widget.layer.pointer_motion(
        PointF {
            x: button.x + button.width / 2.0,
            y: button.y + button.height / 2.0,
        },
        now,
    );
    widget.layer.pointer_button(true, now);
    widget.layer.pointer_button(false, now);
    assert_eq!(
        receive.try_recv().unwrap(),
        crate::shell_components::capture::CaptureDecision::Approve(
            12,
            crate::shell::capture::CaptureSource::Output(crate::shell::OutputId::MIN),
            7
        )
    );
}

use telorgon::app::{Alignment, View, text};
use telorgon::compose::ElementKind;
use telorgon::foundation::{ColorRgba8, SizeF};
use telorgon::graphics::render::{RenderScene, SceneCompiler};
use telorgon::ui::layout::LayoutEngine;
use telorgon::ui::text::RetainedTextSystem;
use telorgon::ui::{BoxStyle, LayoutStyle, MountWriter, MountedUi, SizeRule};

fn render(height: f32, fit: bool, alignment: Alignment) -> (RenderScene, f32) {
    let element = text("Hello")
        .width(200.0)
        .height(height)
        .padding(4.0)
        .size(16.0)
        .fit_height(fit)
        .vertical_align(alignment)
        .into_element();
    let ElementKind::Text(props) = element.kind() else {
        panic!("expected text")
    };
    let mut ui = MountedUi::default();
    let mut node = None;
    MountWriter::<()>::new(&mut ui).root(
        BoxStyle {
            width: SizeRule::Fill(1.0),
            height: SizeRule::Fill(1.0),
            ..BoxStyle::default()
        },
        LayoutStyle::default(),
        |writer| {
            node = Some(
                writer
                    .dynamic_text(
                        props.content.clone(),
                        props.style.resolve(),
                        props.box_style,
                        props.layout,
                    )
                    .node,
            );
        },
    );
    let extent = SizeF {
        width: 300.0,
        height: 200.0,
    };
    let mut layout = LayoutEngine::default();
    let mut text = RetainedTextSystem::new(4096).unwrap();
    layout.update(&mut ui, &mut text, extent, 1.0);
    let computed = layout.computed(node.unwrap()).unwrap();
    assert_eq!(computed.local_border_rect.height, height);
    assert_eq!(computed.local_content_rect.height, height - 8.0);
    let baseline = computed.baseline;
    let shaped = text.stats().shaped;
    let mut scene = RenderScene::default();
    SceneCompiler::default().compile(
        &mut ui,
        &layout,
        &mut text,
        &mut scene,
        extent,
        ColorRgba8::default(),
    );
    assert_eq!(
        text.stats().shaped,
        shaped,
        "layout and paint must share shaped metrics"
    );
    assert!(!scene.glyphs.is_empty());
    (scene, baseline)
}

#[test]
fn height_fit_scales_glyphs_with_the_padded_content_height() {
    let (small, _) = render(28.0, true, Alignment::Start);
    let (large, _) = render(88.0, true, Alignment::Start);
    assert!(large.glyphs[0].rect.height > small.glyphs[0].rect.height * 2.0);
    let (normal, _) = render(88.0, false, Alignment::Start);
    assert_eq!(small.glyphs[0].rect.height, normal.glyphs[0].rect.height);
}

#[test]
fn vertical_alignment_moves_glyphs_and_layout_baseline_together() {
    let (start, start_baseline) = render(88.0, false, Alignment::Start);
    let (center, center_baseline) = render(88.0, false, Alignment::Center);
    let (end, end_baseline) = render(88.0, false, Alignment::End);
    assert_eq!(center.glyphs[0].rect.y - start.glyphs[0].rect.y, 30.0);
    assert_eq!(end.glyphs[0].rect.y - start.glyphs[0].rect.y, 60.0);
    assert!((center_baseline - start_baseline - 30.0).abs() < 0.01);
    assert!((end_baseline - start_baseline - 60.0).abs() < 0.01);
}

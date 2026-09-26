use telorgon::foundation::{ColorRgba8, SizeF};
use telorgon::graphics::render::{RenderScene, SceneCompiler};
use telorgon::ui::layout::LayoutEngine;
use telorgon::ui::text::RetainedTextSystem;
use telorgon::ui::{
    Border, BoxStyle, LayoutStyle, MountWriter, MountedUi, SizeRule, StylePropertyPatch,
};

#[test]
fn animated_subtree_leaves_unrelated_glyphs_and_draw_order_unchanged() {
    let mut ui = MountedUi::default();
    let mut branches = Vec::new();
    let mut labels = Vec::new();
    MountWriter::<()>::new(&mut ui).root(BoxStyle::default(), LayoutStyle::default(), |writer| {
        for _ in 0..20 {
            let branch = writer.container(
                BoxStyle {
                    width: SizeRule::Logical(200.0),
                    height: SizeRule::Logical(30.0),
                    decoration: telorgon::BoxDecoration::new()
                        .border(Border::all(1.0, ColorRgba8::rgba(10, 10, 10, 255))),
                    ..Default::default()
                },
                LayoutStyle::default(),
                |writer| {
                    labels.push(
                        writer
                            .dynamic_text(
                                "Hello",
                                telorgon::compose::TextStyle::new().resolve(),
                                BoxStyle::default(),
                                LayoutStyle::default(),
                            )
                            .node,
                    );
                },
            );
            branches.push(branch);
        }
    });
    let extent = SizeF {
        width: 400.0,
        height: 1000.0,
    };
    let mut layout = LayoutEngine::default();
    let mut text = RetainedTextSystem::new(4096).unwrap();
    let mut compiler = SceneCompiler::default();
    let mut scene = RenderScene::default();
    layout.update(&mut ui, &mut text, extent, 1.0);
    compiler.compile(
        &mut ui,
        &layout,
        &mut text,
        &mut scene,
        extent,
        ColorRgba8::default(),
    );
    let mut history = vec![scene.take_delta().unwrap()];
    let original_glyphs = scene.glyphs.clone();
    let shaped = text.stats().shaped;

    ui.apply_style_patch(
        branches[0],
        StylePropertyPatch {
            border_color: Some(ColorRgba8::rgba(200, 100, 50, 255)),
            ..Default::default()
        },
    );
    let diagnostics = layout.update(&mut ui, &mut text, extent, 1.0);
    assert_eq!(
        diagnostics.measured, 0,
        "border color must not trigger layout"
    );
    let stats = compiler.compile(
        &mut ui,
        &layout,
        &mut text,
        &mut scene,
        extent,
        ColorRgba8::default(),
    );
    assert_eq!(stats.glyphs_patched, 0);
    assert_eq!(text.stats().shaped, shaped);
    let delta = scene.take_delta().unwrap();
    assert!(delta.draw_order.is_none());
    assert!(delta.glyphs.is_empty());
    history.push(delta);

    ui.apply_style_patch(
        branches[0],
        StylePropertyPatch {
            translation_x: Some(3.5),
            scale_x: Some(1.02),
            scale_y: Some(1.02),
            ..Default::default()
        },
    );
    let diagnostics = layout.update(&mut ui, &mut text, extent, 1.0);
    assert_eq!(diagnostics.measured, 0);
    assert_eq!(
        diagnostics.spatial_updated, 2,
        "only the parent and its text move"
    );
    let stats = compiler.compile(
        &mut ui,
        &layout,
        &mut text,
        &mut scene,
        extent,
        ColorRgba8::default(),
    );
    assert!(stats.glyphs_patched > 0 && stats.glyphs_patched < scene.glyphs.len() as u64);
    assert_eq!(text.stats().shaped, shaped);
    let delta = scene.take_delta().unwrap();
    assert!(delta.draw_order.is_none());
    assert!(
        delta
            .glyphs
            .iter()
            .flat_map(|range| range.values.iter())
            .all(|glyph| glyph.node == labels[0])
    );
    history.push(delta);
    let unaffected: Vec<_> = scene
        .glyphs
        .iter()
        .filter(|g| g.node != labels[0])
        .collect();
    let original: Vec<_> = original_glyphs
        .iter()
        .filter(|g| g.node != labels[0])
        .collect();
    assert_eq!(unaffected, original);

    let mut full = RenderScene::default();
    SceneCompiler::default().compile(
        &mut ui,
        &layout,
        &mut text,
        &mut full,
        extent,
        ColorRgba8::default(),
    );
    assert_eq!(
        scene.glyphs, full.glyphs,
        "incremental glyphs must match a fresh compiler"
    );
    assert_eq!(scene.draw_order, full.draw_order);
    let mut reference = full.take_delta().unwrap();
    // Reuse the initial atlas upload, then apply the fresh scene at a newer epoch.
    reference.epoch = history.last().unwrap().epoch + 1;
    assert!(
        render_pixels(&history) == render_pixels(&[history[0].clone(), reference]),
        "incremental software output must match a complete redraw"
    );

    ui.set_dynamic_text(labels[0], "A much longer label");
    layout.update(&mut ui, &mut text, extent, 1.0);
    compiler.compile(
        &mut ui,
        &layout,
        &mut text,
        &mut scene,
        extent,
        ColorRgba8::default(),
    );
    let mut full = RenderScene::default();
    SceneCompiler::default().compile(
        &mut ui,
        &layout,
        &mut text,
        &mut full,
        extent,
        ColorRgba8::default(),
    );
    assert_eq!(
        scene.glyphs, full.glyphs,
        "glyph count changes must repack consistently"
    );
    assert_eq!(scene.draw_order, full.draw_order);
}

fn render_pixels(deltas: &[telorgon::graphics::render::RenderSceneDelta]) -> Vec<u8> {
    use telorgon::graphics::render::{
        RenderBackend, RenderRequest, RenderTargetInfo, TargetLoad, TargetStore,
    };
    use telorgon::graphics::renderers::software::{
        SoftwareRenderer, SoftwareSurface, SoftwareTarget,
    };
    let renderer = SoftwareRenderer;
    let mut scene = renderer.create_scene().unwrap();
    let mut surface = SoftwareSurface::default();
    let target = SoftwareTarget::new(RenderTargetInfo::full(telorgon::SizeI {
        width: 400,
        height: 1000,
    }));
    for delta in deltas {
        renderer.apply_scene_delta(&mut scene, delta).unwrap();
        let mut frame = surface.begin_frame();
        renderer
            .render(
                &mut scene,
                &mut frame,
                &target,
                &RenderRequest {
                    force: true,
                    load: TargetLoad::Clear(ColorRgba8::rgba(0, 0, 0, 255)),
                    store: TargetStore::Store,
                    region: None,
                },
            )
            .unwrap();
    }
    surface.pixels_rgba8().to_vec()
}

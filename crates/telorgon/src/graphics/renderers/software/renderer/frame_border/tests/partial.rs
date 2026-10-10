use super::*;

#[test]
fn partial_hover_resolves_match_fresh_frames_and_reuse_pixel_storage() {
    let extent = SizeI { width: 128, height: 96 };
    let rect = RectF { x: 0.0, y: 0.0, width: 128.0, height: 96.0 };
    let border = BoxInstance {
        node: NodeId::new(0, 1), rect, view_bounds: rect,
        background: Some(ColorRgba8::rgba(30, 50, 70, 145)),
        border: Border::all(1.5, ColorRgba8::rgba(180, 100, 50, 210)),
        outline: Default::default(), corner_radii: CornerRadii::all(8.0),
        shadows: Default::default(), opacity: 1.0,
        clip: ClipId(0), spatial: SpatialId(0),
    };
    let mut button = border.clone();
    button.node = NodeId::new(1, 1);
    button.rect = RectF { x: 10.25, y: 12.75, width: 32.0, height: 20.0 };
    button.view_bounds = button.rect;
    button.border = Border::default();
    button.corner_radii = CornerRadii::all(4.0);
    let mut input = RenderScene::default();
    input.extent = SizeF { width: 128.0, height: 96.0 };
    input.boxes.upsert(border.node, border.clone());
    input.boxes.upsert(button.node, button.clone());
    input.set_draw_order((0..2).map(|index| DrawItem {
        kind: PrimitiveKind::Box, index,
        batch: BatchKey { pipeline: PipelineKind::AnalyticBox, resource: 0,
            clip: ClipId(0), blend: BlendMode::Alpha, target: 0 },
    }).collect());
    let mut source = SoftwareRenderer.create_scene().unwrap();
    let mut delta = input.take_delta().unwrap();
    frame_border::prepare_interior(&mut delta, &border);
    SoftwareRenderer.apply_scene_delta(&mut source, &delta).unwrap();
    source.set_frame_border(Some(border.clone()));
    let render = |scene: &SoftwareScene| {
        let mut output = SoftwareSurface::default();
        SoftwareRenderer.render_composite(&mut output, &[SoftwareCompositeLayer {
            scene, target: RectI { x: 0, y: 0, width: 128, height: 96 },
            clip: None, rounded_clips: [None; 2],
        }], extent, None, ColorRgba8::rgba(0, 0, 0, 0)).unwrap();
        output.pixels_rgba8().to_vec()
    };
    render(&source);
    source.discard_pending_damage();
    fn allocations(source: &SoftwareScene) -> (*const u8, *const u8) {
        let guard = source.frame_cache.lock().unwrap();
        let cache = guard.as_ref().unwrap();
        (cache.interior.image_resources[&frame_border::IMAGE].pixels.as_ptr(),
            cache.output.image_resources[&frame_border::IMAGE].pixels.as_ptr())
    }
    let initial_storage = allocations(&source);
    // Multiple deltas, old/new movement damage, fractional AA edges, alpha changes
    // and a later full invalidation must all agree with a fresh frame resolve.
    for (index, alpha) in [0, 90, 255, 160].into_iter().enumerate() {
        for offset in [1.0, 2.5] {
            input.damage.add(button.view_bounds, input.extent);
            button.rect.x += offset;
            button.view_bounds = button.rect;
            button.background = Some(ColorRgba8::rgba(90, 150, 200, alpha));
            input.boxes.upsert(button.node, button.clone());
            input.damage.add(button.view_bounds, input.extent);
            let mut delta = input.take_delta().unwrap();
            frame_border::prepare_interior(&mut delta, &border);
            SoftwareRenderer.apply_scene_delta(&mut source, &delta).unwrap();
        }
        if index == 3 { source.pending_damage.full = true; }
        let mut fresh = source.clone();
        fresh.frame_cache = Default::default();
        assert_eq!(render(&source), render(&fresh), "hover alpha {alpha}");
        assert_eq!(allocations(&source), initial_storage, "hover reallocated full-frame pixels");
        source.discard_pending_damage();
    }
    // Changing the border itself invalidates both images and preserves the new ring.
    let mut changed = border;
    changed.border = Border::all(2.0, ColorRgba8::rgba(20, 220, 80, 255));
    source.set_frame_border(Some(changed));
    let mut fresh = source.clone();
    fresh.frame_cache = Default::default();
    assert_eq!(render(&source), render(&fresh));
}

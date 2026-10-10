use super::*;
use crate::graphics::render::{BatchKey, ClipId, PipelineKind, RenderScene};
use crate::graphics::scene::NodeId;
use crate::ui::CornerRadii;
use std::time::Instant;

#[test]
#[ignore = "manual headless renderer timing probe; use release mode"]
fn hover_refresh_cost() {
    for (width, height) in [(640, 480), (1280, 900)] {
        for mode in ["unbordered-hover", "bordered-cached", "bordered-hover"] {
            let extent = SizeI { width, height };
            let rect = RectF { x: 0.0, y: 0.0, width: width as f32, height: height as f32 };
            let border = BoxInstance {
                node: NodeId::new(0, 1), rect, view_bounds: rect,
                background: Some(ColorRgba8::rgba(40, 40, 40, 255)),
                border: Border::all(1.0, ColorRgba8::rgba(100, 100, 100, 255)),
                outline: Default::default(), corner_radii: CornerRadii::all(8.0),
                shadows: Default::default(), opacity: 1.0, clip: ClipId(0), spatial: SpatialId(0),
            };
            let mut button = border.clone();
            button.node = NodeId::new(1, 1);
            button.rect = RectF { x: 100.0, y: 100.0, width: 100.0, height: 36.0 };
            button.view_bounds = button.rect;
            button.border = Border::default();
            button.corner_radii = CornerRadii::all(4.0);
            let mut input = RenderScene::default();
            input.extent = SizeF { width: width as f32, height: height as f32 };
            input.background = ColorRgba8::rgba(0, 0, 0, 0);
            input.boxes.upsert(border.node, border.clone());
            input.boxes.upsert(button.node, button.clone());
            input.set_draw_order((0..2).map(|index| DrawItem {
                kind: PrimitiveKind::Box, index,
                batch: BatchKey { pipeline: PipelineKind::AnalyticBox, resource: 0,
                    clip: ClipId(0), blend: BlendMode::Alpha, target: 0 },
            }).collect());
            let mut scene = SoftwareRenderer.create_scene().unwrap();
            let mut initial = input.take_delta().unwrap();
            if mode != "unbordered-hover" {
                frame_border::prepare_interior(&mut initial, &border);
                scene.set_frame_border(Some(border.clone()));
            }
            SoftwareRenderer.apply_scene_delta(&mut scene, &initial).unwrap();
            let mut surface = SoftwareSurface::default();
            let draw = |scene: &SoftwareScene, surface: &mut SoftwareSurface, damage| {
                SoftwareRenderer.render_composite(surface, &[SoftwareCompositeLayer {
                    scene, target: RectI { x: 0, y: 0, width, height }, clip: None,
                    rounded_clips: [None; 2],
                }], extent, damage, ColorRgba8::rgba(0, 0, 0, 255)).unwrap();
            };
            draw(&scene, &mut surface, None);
            scene.discard_pending_damage();
            let damage = RectI { x: 100, y: 100, width: 100, height: 36 };
            let mut prepare_ms = Vec::new();
            let mut composite_ms = Vec::new();
            for n in 0..12 {
                if mode != "bordered-cached" {
                    button.background = Some(ColorRgba8::rgba(if n % 2 == 0 { 70 } else { 100 }, 80, 90, 255));
                    input.boxes.upsert(button.node, button.clone());
                    input.damage.add(button.rect, input.extent);
                    let mut delta = input.take_delta().unwrap();
                    assert!(!delta.damage.full);
                    if mode != "unbordered-hover" { frame_border::prepare_interior(&mut delta, &border); }
                    SoftwareRenderer.apply_scene_delta(&mut scene, &delta).unwrap();
                }
                let start = Instant::now();
                scene.prepare_frame_border().unwrap();
                prepare_ms.push(start.elapsed().as_secs_f64() * 1000.0);
                let start = Instant::now();
                draw(&scene, &mut surface, Some(damage));
                composite_ms.push(start.elapsed().as_secs_f64() * 1000.0);
                scene.discard_pending_damage();
            }
            println!("HOVER_PROBE extent={}x{} mode={} damage_pixels=3600 prepare_avg_ms={:.3} prepare_max_ms={:.3} composite_avg_ms={:.3}", width, height, mode,
                prepare_ms.iter().sum::<f64>() / 12.0, prepare_ms.iter().copied().fold(0.0, f64::max), composite_ms.iter().sum::<f64>() / 12.0);
        }
    }
}

use super::*;
use crate::host::linux_shell::scene::{ShellComposition, ShellLayer};
use crate::foundation::SizeI;

#[test]
fn rotating_scanout_buffers_catch_up_without_losing_retained_pixels() {
    let mut extent = SizeI { width: 24, height: 18 };
    let mut composition = ShellComposition::new(extent);
    let mut renderer = SoftwareShellRenderer::new(3);
    let mut targets = vec![Vec::new(); 3];
    for step in 0..18 {
        if step == 12 {
            extent = SizeI { width: 28, height: 20 };
            renderer.invalidate_targets();
        }
        let layers = vec![
            ShellLayer::solid(ShellLayerKey::Background, ShellSceneKey::Background,
                ColorRgba8::rgba(19, 80, 130, 255), full_rect(extent)),
            ShellLayer::solid(ShellLayerKey::Widget(1), ShellSceneKey::Widget(1),
                ColorRgba8::rgba(200, 20, 30, 145), RectI { x: 2 + step % 8, y: 3, width: 4, height: 5 }),
        ];
        let frame = composition.synchronize(extent, layers).unwrap();
        let target = step as usize % 3;
        let damage = renderer.render(target, frame.clone()).unwrap();
        let mut reference = SoftwareSurface::default();
        let retained: Vec<_> = frame.placements.iter().map(|p| SoftwareCompositeLayer {
            scene: &renderer.scenes[&p.scene], target: p.target, clip: p.clip, rounded_clips: p.rounded_clips,
        }).collect();
        SoftwareRenderer.render_composite(&mut reference, &retained, extent, None,
            ColorRgba8::rgba(0, 0, 0, 255)).unwrap();
        assert_eq!(renderer.pixels(), reference.pixels_rgba8(), "composition step {step}");
        targets[target].resize(renderer.pixels().len(), 0);
        for y in damage.y..damage.y + damage.height as i32 {
            let start = (y as usize * extent.width as usize + damage.x as usize) * 4;
            let end = start + damage.width as usize * 4;
            targets[target][start..end].copy_from_slice(&renderer.pixels()[start..end]);
        }
        renderer.mark_copied(target);
        assert_eq!(targets[target], reference.pixels_rgba8(), "scanout step {step}");
    }
}

#[test]
fn cached_backdrop_restores_closed_menu_and_invalidates_on_wallpaper_change() {
    let extent = SizeI { width: 12, height: 8 };
    let bounds = crate::foundation::RectI { x: 3, y: 2, width: 4, height: 3 };
    let layers = |color, menu| {
        let mut layers = vec![ShellLayer::solid(ShellLayerKey::Background,
            ShellSceneKey::Background, color, full_rect(extent))];
        if menu {
            layers.push(ShellLayer::solid(ShellLayerKey::Widget(1), ShellSceneKey::Widget(1),
                ColorRgba8::rgba(200, 20, 30, 245), bounds));
        }
        layers
    };
    let original = ColorRgba8::rgba(19, 80, 130, 255);
    let replacement = ColorRgba8::rgba(90, 100, 110, 255);
    let mut composition = ShellComposition::new(extent);
    let mut renderer = SoftwareShellRenderer::new(2);
    let initial = composition.synchronize(extent, layers(original, false)).unwrap();
    renderer.render(0, initial).unwrap();
    renderer.mark_copied(0);
    let untouched = renderer.pixels().to_vec();
    let open = composition.synchronize(extent, layers(original, true)).unwrap();
    renderer.render(0, open.clone()).unwrap();
    renderer.mark_copied(0);
    // Compare the cached composition to the ordinary rasterizer byte for byte.
    let retained: Vec<_> = open.placements.iter().map(|p| SoftwareCompositeLayer {
        scene: &renderer.scenes[&p.scene], target: p.target, clip: p.clip,
        rounded_clips: p.rounded_clips,
    }).collect();
    let mut reference = SoftwareSurface::default();
    SoftwareRenderer.render_composite(&mut reference, &retained, extent, None,
        ColorRgba8::rgba(0, 0, 0, 255)).unwrap();
    assert_eq!(renderer.pixels(), reference.pixels_rgba8());
    let close = composition.synchronize(extent, layers(original, false)).unwrap();
    assert_eq!(renderer.render(0, close).unwrap(), bounds);
    renderer.mark_copied(0);
    assert_eq!(renderer.pixels(), untouched);
    let changed = composition.synchronize(extent, layers(replacement, false)).unwrap();
    renderer.render(1, changed).unwrap();
    assert!(renderer.pixels().chunks_exact(4).all(|p| p == [90, 100, 110, 255]));
}

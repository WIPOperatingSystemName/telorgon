use super::*;

fn base(extent: SizeI) -> ShellLayer {
    ShellLayer::solid(ShellLayerKey::Background, ShellSceneKey::Background,
        ColorRgba8::rgba(13, 25, 39, 255), full_rect(extent))
}
fn popup(id: u32, bounds: RectI) -> ShellLayer {
    ShellLayer::solid(ShellLayerKey::Widget(id), ShellSceneKey::Widget(id),
        ColorRgba8::rgba(210, 220, 230, 255), bounds)
}

#[test]
fn adding_moving_and_removing_popup_only_damages_its_bounds() {
    let extent = SizeI { width: 200, height: 120 };
    let bounds = RectI { x: 20, y: 30, width: 50, height: 40 };
    let moved = RectI { x: 30, ..bounds };
    let mut composition = ShellComposition::new(extent);
    composition.synchronize(extent, vec![base(extent)]).unwrap();
    let added = composition.synchronize(extent, vec![base(extent), popup(1, bounds)]).unwrap();
    assert_eq!(added.damage, Some(bounds));
    let shifted = composition.synchronize(extent, vec![base(extent), popup(1, moved)]).unwrap();
    assert_eq!(shifted.damage, Some(RectI { width: 60, ..bounds }));
    let removed = composition.synchronize(extent, vec![base(extent)]).unwrap();
    assert_eq!(removed.damage, Some(moved));
}

#[test]
fn swapping_existing_overlapping_layers_still_repaints_the_output() {
    let extent = SizeI { width: 200, height: 120 };
    let bounds = RectI { x: 20, y: 30, width: 50, height: 40 };
    let mut composition = ShellComposition::new(extent);
    composition.synchronize(extent, vec![base(extent), popup(1, bounds), popup(2, bounds)]).unwrap();
    let frame = composition.synchronize(extent, vec![base(extent), popup(2, bounds), popup(1, bounds)]).unwrap();
    assert_eq!(frame.damage, None);
}

#[test]
fn quantized_hover_frame_advances_without_full_output_damage() {
    let extent = SizeI { width: 200, height: 120 };
    let mut composition = ShellComposition::new(extent);
    composition.synchronize(extent, vec![base(extent)]).unwrap();
    assert!(composition.synchronize(extent, vec![base(extent)]).is_none());
    let frame = composition.synchronize_with_force(extent, vec![base(extent)], true).unwrap();
    assert!(frame.updates.is_empty());
    assert_eq!(frame.damage, Some(RectI { x: 0, y: 0, width: 1, height: 1 }));
}

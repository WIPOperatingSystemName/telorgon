use super::*;
#[test]
fn widget_and_previews_share_one_opacity_group_without_capturing_other_layers() {
    let bounds = RectI {
        x: 100,
        y: 200,
        width: 200,
        height: 100,
    };
    let placement = |key, scene| ShellPlacement {
        key,
        scene,
        target: bounds,
        clip: None,
        rounded_clips: [None; 2],
    };
    let mut frame = ShellFrame {
        glass_changed: Default::default(),
        glass: Default::default(),
        preview_borders: Default::default(),
        motion: Default::default(),
        extent: SizeI {
            width: 800,
            height: 600,
        },
        live_scenes: Default::default(),
        updates: vec![],
        placements: vec![
            placement(ShellLayerKey::Surface(9), ShellSceneKey::Surface(9)),
            placement(ShellLayerKey::TilePreview(1), ShellSceneKey::TilePreview(1)),
            placement(ShellLayerKey::Widget(1), ShellSceneKey::Widget(1)),
            placement(
                ShellLayerKey::WindowPreview(1, 0, 9),
                ShellSceneKey::Surface(9),
            ),
            placement(ShellLayerKey::OutputPreview(1, 0, 0), ShellSceneKey::Background),
            placement(ShellLayerKey::Widget(2), ShellSceneKey::Widget(2)),
        ],
        surface_revisions: vec![],
        damage: None,
    };
    WindowMotionController::default().apply_widget_visibility(&mut frame, &[(1, 0.5)]);
    assert_eq!(frame.placements.len(), 3);
    assert_eq!(frame.placements[0].key, ShellLayerKey::Surface(9));
    assert_eq!(frame.placements[1].key, ShellLayerKey::Widget(1));
    assert_eq!(frame.placements[2].key, ShellLayerKey::Widget(2));
    assert_eq!(frame.motion.outputs.len(), 1);
    assert_eq!(frame.motion.outputs[0].opacity, 0.5);
    let SnapshotContent::Capture(group) = &frame.motion.snapshots[0].content else {
        panic!()
    };
    assert_eq!(group.len(), 4);
    assert_eq!(group[0].target.x, 0);
    assert_eq!(group[1].target.y, 0);
}
#[test]
fn later_surface_gets_animation_budget_before_idle_windows() {
    use super::super::scene::{ShellComposition, ShellLayer};
    let extent = SizeI {
        width: 3840,
        height: 2400,
    };
    let initial = WindowState {
        bounds: RectI {
            x: 0,
            y: 0,
            width: 3000,
            height: 2000,
        },
        maximized: false,
        tiled: None,
        interactive: false,
        move_pointer: None,
        minimized: false,
        veiled: false,
        style: WindowMotion::smooth(),
        corner_radii: Default::default(),
        shadows: Default::default(),
    };
    let mut states = BTreeMap::from([(1, initial), (2, initial)]);
    let owners = BTreeMap::from([(1, 1), (2, 2)]);
    let mut composition = ShellComposition::new(extent);
    let mut controller = WindowMotionController::default();
    for now in [0, 1] {
        if now == 1 {
            let state = states.get_mut(&2).unwrap();
            state.maximized = true;
            state.veiled = true;
            state.bounds.width = extent.width;
            state.bounds.height = extent.height;
        }
        let layers = states
            .iter()
            .map(|(id, state)| {
                ShellLayer::solid(
                    ShellLayerKey::ResizeVeil(*id),
                    ShellSceneKey::ResizeVeil(*id),
                    crate::foundation::ColorRgba8::rgba(20, 30, 40, 255),
                    state.bounds,
                )
            })
            .collect();
        let mut frame = composition
            .synchronize_with_force(extent, layers, true)
            .unwrap();
        controller.apply(
            &mut frame,
            states.clone(),
            &owners,
            now,
            MotionPreference::Full,
        );
        assert!(frame.motion.outputs.iter().any(|o| o.id == 2));
        if now == 1 {
            let output = frame
                .placements
                .iter()
                .find(|p| p.key == ShellLayerKey::Motion(2))
                .unwrap();
            assert_eq!(
                output.target.width, 3000,
                "later surface must animate from its previous geometry"
            );
        }
    }
}
#[test]
fn retarget_continues_from_sample_and_zero_duration_settles() {
    let mut t = Track::fixed(0.0);
    t.retarget(1.0, crate::tween_ms(100, crate::Easing::Linear), 0);
    assert_eq!(t.sample(50_000_000), 0.5);
    t.retarget(0.0, crate::tween_ms(100, crate::Easing::Linear), 50_000_000);
    assert_eq!(t.sample(50_000_000), 0.5);
    assert_eq!(t.sample(100_000_000), 0.25);
    t.retarget(1.0, crate::tween_ms(0, crate::Easing::Linear), 100_000_000);
    assert_eq!(t.sample(100_000_000), 1.0);
    assert!(!t.active(100_000_000));
}

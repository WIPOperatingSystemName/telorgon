use super::*;
fn layer(revision: u64, extent: SizeI, update: ShellImageUpdate) -> ShellLayer {
    ShellLayer::image(
        ShellLayerKey::Surface(1),
        ShellSceneKey::Surface(1),
        revision,
        update,
        extent,
        RectI {
            x: 20,
            y: 30,
            width: extent.width,
            height: extent.height,
        },
        None,
        ImageAlphaMode::Opaque,
        ImagePixelFormat::Rgba8,
        true,
    )
}
fn external(revision: u64, damage: Option<RectI>) -> ShellImageUpdate {
    ShellImageUpdate::External {
        image: ImageId(1),
        content_version: revision,
        damage,
    }
}
#[test]
fn external_damage_reaches_output_at_hidpi() {
    let output = SizeI {
        width: 1280,
        height: 800,
    };
    let size = SizeI {
        width: 1000,
        height: 700,
    };
    let mut composition = ShellComposition::new(output);
    composition
        .synchronize(output, vec![layer(1, size, external(1, None))])
        .unwrap();
    let frame = composition
        .synchronize(
            output,
            vec![layer(
                2,
                size,
                external(
                    2,
                    Some(RectI {
                        x: 5,
                        y: 7,
                        width: 10,
                        height: 12,
                    }),
                ),
            )],
        )
        .unwrap();
    assert_eq!(frame.surface_revisions, [(1, 2)]);
    assert_eq!(
        frame.damage,
        Some(RectI {
            x: 25,
            y: 37,
            width: 10,
            height: 12
        })
    );
    let physical = frame.into_physical(
        crate::platform::contracts::ScaleFactor::new(3.0).unwrap(),
        SizeI {
            width: 3840,
            height: 2400,
        },
    );
    assert_eq!(
        physical.damage,
        Some(RectI {
            x: 75,
            y: 111,
            width: 30,
            height: 36
        })
    );
}
#[test]
fn blocked_resize_keeps_old_revision_while_other_placements_move() {
    let output = SizeI {
        width: 1280,
        height: 800,
    };
    let size = SizeI {
        width: 100,
        height: 80,
    };
    let mut composition = ShellComposition::new(output);
    composition
        .synchronize(output, vec![layer(1, size, external(1, None))])
        .unwrap();
    let mut blocked = layer(
        2,
        SizeI {
            width: 200,
            height: 160,
        },
        ShellImageUpdate::Unchanged,
    );
    blocked.target.x += 50;
    let frame = composition.synchronize(output, vec![blocked]).unwrap();
    assert_eq!(frame.surface_revisions, [(1, 1)]);
    assert!(frame.updates.is_empty());
    assert_eq!(frame.placements[0].target.x, 70);
    assert_eq!(
        composition.image_scenes[&ShellSceneKey::Surface(1)].extent,
        size
    );
    let ready = composition
        .synchronize(
            output,
            vec![layer(
                2,
                SizeI {
                    width: 200,
                    height: 160,
                },
                external(2, None),
            )],
        )
        .unwrap();
    assert_eq!(ready.surface_revisions, [(1, 2)]);
}
#[test]
fn first_unready_image_is_not_presented_or_acknowledged() {
    let output = SizeI {
        width: 1280,
        height: 800,
    };
    let mut composition = ShellComposition::new(output);
    let frame = composition.synchronize(
        output,
        vec![layer(
            1,
            SizeI {
                width: 100,
                height: 80,
            },
            ShellImageUpdate::Unchanged,
        )],
    );
    assert!(frame.is_none_or(|f| f.placements.is_empty() && f.surface_revisions.is_empty()));
}

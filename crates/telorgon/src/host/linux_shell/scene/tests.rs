use super::*;

#[test]
fn glass_is_idle_until_lower_content_or_its_appearance_changes() {
    let extent = SizeI {
        width: 160,
        height: 120,
    };
    let target = RectI {
        x: 30,
        y: 20,
        width: 80,
        height: 70,
    };
    let layers = |background, foreground, glass: bool| {
        let mut preview = veil(target, ColorRgba8::rgba(23, 27, 37, 150));
        preview.glass = glass.then_some(crate::GlassStyle::default());
        vec![
            ShellLayer::solid(
                ShellLayerKey::Background,
                ShellSceneKey::Background,
                background,
                full_rect(extent),
            ),
            preview,
            ShellLayer::solid(
                ShellLayerKey::Cursor,
                ShellSceneKey::CursorImage,
                foreground,
                RectI {
                    x: 140,
                    y: 100,
                    width: 5,
                    height: 5,
                },
            ),
        ]
    };
    let black = ColorRgba8::rgba(0, 0, 0, 255);
    let white = ColorRgba8::rgba(255, 255, 255, 255);
    let mut composition = ShellComposition::new(extent);
    let first = composition
        .synchronize(extent, layers(black, black, true))
        .unwrap();
    assert!(first.glass_changed.contains(&ShellSceneKey::ResizeVeil(9)));
    assert!(
        composition
            .synchronize(extent, layers(black, black, true))
            .is_none()
    );
    let cursor = composition
        .synchronize(extent, layers(black, white, true))
        .unwrap();
    assert!(
        cursor.glass_changed.is_empty(),
        "foreground changes must not rebuild glass snapshots"
    );
    assert_eq!(
        cursor.damage,
        Some(RectI {
            x: 140,
            y: 100,
            width: 5,
            height: 5
        })
    );
    let lower = composition
        .synchronize(extent, layers(white, white, true))
        .unwrap();
    assert!(lower.glass_changed.contains(&ShellSceneKey::ResizeVeil(9)));
    let flat = composition
        .synchronize(extent, layers(white, white, false))
        .unwrap();
    assert!(flat.glass.is_empty());
    assert_eq!(
        flat.damage,
        Some(target),
        "switching to the identical flat tint must still repaint"
    );
}

#[test]
fn glass_distances_cross_output_scale_once() {
    let extent = SizeI {
        width: 100,
        height: 100,
    };
    let mut preview = veil(full_rect(extent), ColorRgba8::rgba(0, 0, 0, 0));
    preview.glass = Some(crate::GlassStyle {
        blend_softness: 16.0,
        ..crate::GlassStyle::default()
    });
    let mut composition = ShellComposition::new(extent);
    let frame = composition
        .synchronize(extent, vec![preview])
        .unwrap()
        .into_physical(
            crate::platform::contracts::ScaleFactor::new(2.0).unwrap(),
            SizeI {
                width: 200,
                height: 200,
            },
        );
    let physical = frame.glass[&ShellSceneKey::ResizeVeil(9)];
    let logical = crate::GlassStyle::liquid();
    assert_eq!(physical.blur_radius, logical.blur_radius * 2.0);
    assert_eq!(physical.bevel_width, logical.bevel_width * 2.0);
    assert_eq!(physical.blend_softness, 32.0);
    assert_eq!(physical.refraction, logical.refraction * 2.0);
    assert_eq!(physical.dispersion, logical.dispersion * 2.0);

    assert_eq!(physical.fresnel, logical.fresnel);

    assert_eq!(physical.tint, logical.tint);
}

#[test]
fn output_mapping_scales_geometry_clips_and_damage_once_and_preserves_revisions() {
    let logical = RectI {
        x: 1,
        y: 3,
        width: 100,
        height: 40,
    };
    let mut rounded = RoundedClip::new(
        RectF {
            x: 1.0,
            y: 3.0,
            width: 100.0,
            height: 40.0,
        },
        crate::ui::CornerRadii::all(4.0),
    );
    rounded.inverted = true;
    let frame = ShellFrame {
        glass_changed: BTreeSet::new(),
        glass: BTreeMap::new(),
        preview_borders: BTreeMap::new(),
        motion: Default::default(),
        extent: SizeI {
            width: 1280,
            height: 720,
        },
        live_scenes: BTreeSet::new(),
        updates: Vec::new(),
        placements: vec![ShellPlacement {
            key: ShellLayerKey::Surface(1),
            scene: ShellSceneKey::Surface(1),
            target: logical,
            clip: Some(logical),
            rounded_clips: [Some(rounded), None],
        }],
        surface_revisions: vec![(1, 9)],
        damage: Some(logical),
    };
    for factor in [1.0, 1.5, 2.0] {
        let scale = crate::platform::contracts::ScaleFactor::new(factor).unwrap();
        let pixels = SizeI {
            width: (1280.0 * factor) as i32,
            height: (720.0 * factor) as i32,
        };
        let physical = frame.clone().into_physical(scale, pixels);
        assert_eq!(physical.extent, pixels);
        assert_eq!(physical.placements[0].target, scale.physical_rect(logical));
        assert_eq!(
            physical.placements[0].clip,
            Some(scale.physical_rect(logical))
        );
        assert_eq!(physical.damage, Some(scale.physical_damage(logical)));
        let clip = physical.placements[0].rounded_clips[0].unwrap();
        assert_eq!(clip.radii.top_left, 4.0 * factor);
        assert_eq!(clip.rect.y, 3.0 * factor);
        assert!(clip.inverted);
        assert_eq!(physical.surface_revisions, [(1, 9)]);
    }
}

#[test]
fn frame_cutouts_cover_only_the_complement_even_at_output_edges() {
    let extent = SizeI {
        width: 20,
        height: 16,
    };
    let position = PointI { x: -2, y: -1 };
    let outer = RectI {
        x: -2,
        y: -1,
        width: 20,
        height: 16,
    };
    for hole in [
        outer,
        RectI {
            x: 4,
            y: 4,
            width: 8,
            height: 6,
        },
        RectI {
            x: -10,
            y: -10,
            width: 15,
            height: 15,
        },
        RectI {
            x: 40,
            y: 40,
            width: 4,
            height: 4,
        },
    ] {
        let layers =
            ShellLayer::retained_frame(9, Vec::new(), extent, position, true, Some(hole));
        for y in -3..19 {
            for x in -4..22 {
                let contains =
                    |r: RectI| x >= r.x && y >= r.y && x < r.right() && y < r.bottom();
                let count = layers
                    .iter()
                    .filter(|layer| contains(layer.target) && layer.clip.is_none_or(contains))
                    .count();
                assert_eq!(count, usize::from(contains(outer) && !contains(hole)));
            }
        }
    }
    let mut composition = ShellComposition::new(extent);
    let hidden = composition
        .synchronize(
            SizeI {
                width: 24,
                height: 20,
            },
            ShellLayer::retained_frame(9, Vec::new(), extent, position, true, Some(outer)),
        )
        .unwrap();
    assert!(hidden.placements.is_empty());
    assert!(hidden.live_scenes.contains(&ShellSceneKey::Frame(9)));
}

#[test]
fn per_window_preview_colors_do_not_alias_a_shared_scene() {
    let extent = SizeI {
        width: 100,
        height: 80,
    };
    let mut composition = ShellComposition::new(extent);
    let colors = [
        ColorRgba8::rgba(200, 20, 30, 128),
        ColorRgba8::rgba(10, 150, 250, 255),
    ];
    let layers = colors
        .into_iter()
        .enumerate()
        .map(|(i, color)| {
            ShellLayer::solid(
                ShellLayerKey::ResizeVeil(i as u32),
                ShellSceneKey::ResizeVeil(i as u32),
                color,
                RectI {
                    x: i as i32 * 40,
                    y: 0,
                    width: 40,
                    height: 40,
                },
            )
        })
        .collect();
    let frame = composition.synchronize(extent, layers).unwrap();
    assert_eq!(frame.updates.len(), 2);
    for (update, color) in frame.updates.iter().zip(colors) {
        assert_eq!(update.deltas[0].boxes[0].values[0].background, Some(color));
    }
}

fn veil(target: RectI, color: ColorRgba8) -> ShellLayer {
    ShellLayer::solid(
        ShellLayerKey::ResizeVeil(9),
        ShellSceneKey::ResizeVeil(9),
        color,
        target,
    )
}

#[test]
fn resize_veil_moves_without_client_uploads_and_reveals_only_the_final_image() {
    let extent = SizeI {
        width: 800,
        height: 600,
    };
    let position = PointI { x: 100, y: 90 };
    let color = ColorRgba8 {
        r: 38,
        g: 42,
        b: 48,
        a: 255,
    };
    let mut composition = ShellComposition::new(extent);
    composition
        .synchronize(
            extent,
            vec![image_layer(
                position,
                ShellImageUpdate::Full(vec![255; 100 * 80 * 4].into()),
            )],
        )
        .unwrap();
    let preview = |width, height| RectI {
        x: position.x,
        y: position.y,
        width,
        height,
    };
    let hidden = || {
        let mut layer = image_layer(position, ShellImageUpdate::Unchanged);
        layer.visible = false;
        layer
    };
    let first = composition
        .synchronize(extent, vec![veil(preview(100, 80), color), hidden()])
        .unwrap();
    assert_eq!(first.updates.len(), 1);
    assert_eq!(first.updates[0].key, ShellSceneKey::ResizeVeil(9));
    let delta = &first.updates[0].deltas[0];
    assert!(delta.image_resources.is_empty());
    assert_eq!(delta.boxes[0].values[0].background, Some(color));
    assert!(first.live_scenes.contains(&ShellSceneKey::Surface(9)));
    assert_eq!(first.placements.len(), 1);
    assert!(first.surface_revisions.is_empty());

    let moved = composition
        .synchronize(extent, vec![veil(preview(150, 110), color), hidden()])
        .unwrap();
    assert_eq!(moved.updates.len(), 1);
    assert_eq!(moved.updates[0].key, ShellSceneKey::ResizeVeil(9));
    assert!(moved.updates[0].deltas[0].image_resources.is_empty());
    assert_eq!(
        moved.updates[0].deltas[0].extent,
        SizeF {
            width: 150.0,
            height: 110.0
        }
    );
    assert_eq!(moved.placements[0].target, preview(150, 110));
    assert!(
        composition
            .synchronize(extent, vec![veil(preview(150, 110), color), hidden()])
            .is_none()
    );

    let final_extent = SizeI {
        width: 144,
        height: 104,
    }; // cell-snapped legal response
    let final_layer = ShellLayer::image(
        ShellLayerKey::Surface(9),
        ShellSceneKey::Surface(9),
        2,
        ShellImageUpdate::Full(vec![128; 144 * 104 * 4].into()),
        final_extent,
        preview(144, 104),
        None,
        ImageAlphaMode::Opaque,
        ImagePixelFormat::Rgba8,
        true,
    );
    let final_frame = composition.synchronize(extent, vec![final_layer]).unwrap();
    assert_eq!(final_frame.placements.len(), 1);
    assert_eq!(final_frame.surface_revisions, vec![(9, 2)]);
    assert_eq!(final_frame.placements[0].scene, ShellSceneKey::Surface(9));
    assert_eq!(final_frame.updates.len(), 1);
    assert_eq!(final_frame.updates[0].deltas[0].image_resources.len(), 1);
    assert!(
        !final_frame
            .live_scenes
            .contains(&ShellSceneKey::ResizeVeil(9))
    );
}

#[test]
fn solid_color_changes_keep_scene_epochs_monotonic() {
    let extent = SizeI {
        width: 200,
        height: 150,
    };
    let target = RectI {
        x: 0,
        y: 0,
        width: 100,
        height: 80,
    };
    let mut composition = ShellComposition::new(extent);
    let first = composition
        .synchronize(
            extent,
            vec![veil(
                target,
                ColorRgba8 {
                    r: 30,
                    g: 40,
                    b: 50,
                    a: 255,
                },
            )],
        )
        .unwrap();
    let second = composition
        .synchronize(
            extent,
            vec![veil(
                target,
                ColorRgba8 {
                    r: 60,
                    g: 70,
                    b: 80,
                    a: 255,
                },
            )],
        )
        .unwrap();
    assert!(second.updates[0].deltas[0].epoch > first.updates[0].deltas[0].epoch);
    assert!(second.updates[0].deltas[0].image_resources.is_empty());
}

#[test]
fn damage_free_publication_advances_displayed_revision_without_uploading_pixels() {
    let extent = SizeI {
        width: 800,
        height: 600,
    };
    let position = PointI { x: 20, y: 30 };
    let mut composition = ShellComposition::new(extent);
    let first = composition
        .synchronize(
            extent,
            vec![image_layer(
                position,
                ShellImageUpdate::Full(vec![255; 100 * 80 * 4].into()),
            )],
        )
        .unwrap();
    let mut reused = image_layer(position, ShellImageUpdate::Reused);
    if let ShellLayerContent::Image {
        content_version, ..
    } = &mut reused.content
    {
        *content_version = 2;
    }
    let second = composition.synchronize(extent, vec![reused]).unwrap();
    assert_eq!(
        first.surface_revisions,
        vec![(9, 1)],
        "queued frames must retain their own revision"
    );
    assert_eq!(second.surface_revisions, vec![(9, 2)]);
    assert!(
        second
            .updates
            .iter()
            .flat_map(|update| &update.deltas)
            .all(|delta| delta.image_resources.is_empty())
    );
}

fn image_layer(position: PointI, update: ShellImageUpdate) -> ShellLayer {
    image_layer_at(
        RectI {
            x: position.x,
            y: position.y,
            width: 100,
            height: 80,
        },
        update,
    )
}

fn image_layer_at(target: RectI, update: ShellImageUpdate) -> ShellLayer {
    ShellLayer::image(
        ShellLayerKey::Surface(9),
        ShellSceneKey::Surface(9),
        1,
        update,
        SizeI {
            width: 100,
            height: 80,
        },
        target,
        None,
        ImageAlphaMode::Opaque,
        ImagePixelFormat::Rgba8,
        true,
    )
}

#[test]
fn thumbnail_reuses_hidden_scene_without_upload_and_retires_independently() {
    let extent = SizeI {
        width: 800,
        height: 600,
    };
    let mut composition = ShellComposition::new(SizeI {
        width: 800,
        height: 600,
    });
    let original = || image_layer(PointI { x: 10, y: 10 }, ShellImageUpdate::Unchanged);
    let first = composition
        .synchronize(
            extent,
            vec![image_layer(
                PointI { x: 10, y: 10 },
                ShellImageUpdate::Full(vec![255; 100 * 80 * 4].into()),
            )],
        )
        .unwrap();
    assert_eq!(first.updates.len(), 1);
    let mut gpu_scene = crate::graphics::renderers::vulkan::VulkanScene::default();
    for delta in &first.updates[0].deltas {
        gpu_scene.apply_delta_checked(delta).unwrap();
    }

    let preview = || {
        let mut layer = image_layer_at(
            RectI {
                x: 400,
                y: 450,
                width: 100,
                height: 80,
            },
            ShellImageUpdate::Unchanged,
        );
        layer.key = ShellLayerKey::WindowPreview(7, 0, 9);
        layer
    };
    let mut hidden = original();
    hidden.visible = false;
    let frame = composition
        .synchronize(extent, vec![hidden, preview()])
        .unwrap();
    assert!(frame.updates.is_empty());
    assert_eq!(frame.placements.len(), 1);
    assert_eq!(frame.placements[0].scene, ShellSceneKey::Surface(9));
    assert_eq!(frame.placements[0].target.x, 400);
    let restored = composition.synchronize(extent, vec![original()]).unwrap();
    assert!(restored.updates.is_empty());
    assert_eq!(restored.placements.len(), 1);
    assert!(restored.live_scenes.contains(&ShellSceneKey::Surface(9)));
    let closed = composition.synchronize(extent, Vec::new()).unwrap();
    assert!(closed.live_scenes.is_empty());
    assert!(closed.placements.is_empty());
}
#[test]
fn thumbnail_without_a_ready_producer_does_not_emit_missing_image_draws() {
    let mut composition = ShellComposition::new(SizeI {
        width: 800,
        height: 600,
    });
    let mut preview = image_layer(PointI::default(), ShellImageUpdate::Unchanged);
    preview.key = ShellLayerKey::WindowPreview(7, 0, 9);
    let frame = composition
        .synchronize_with_force(
            SizeI {
                width: 800,
                height: 600,
            },
            vec![preview],
            true,
        )
        .unwrap();
    assert!(frame.updates.is_empty());
    assert!(frame.placements.is_empty());
}
#[test]
fn explicit_placement_scaling_does_not_require_a_content_update() {
    let extent = SizeI {
        width: 800,
        height: 600,
    };
    let mut composition = ShellComposition::new(extent);
    let _ = composition.synchronize(
        extent,
        vec![image_layer(
            PointI { x: 300, y: 220 },
            ShellImageUpdate::Full(vec![255; 100 * 80 * 4].into()),
        )],
    );

    let target = RectI {
        x: 260,
        y: 190,
        width: 140,
        height: 110,
    };
    let frame = composition
        .synchronize(
            extent,
            vec![image_layer_at(target, ShellImageUpdate::Unchanged)],
        )
        .unwrap();

    assert!(frame.updates.is_empty());
    assert_eq!(frame.placements[0].target, target);
    assert_eq!(
        frame.damage,
        Some(RectI {
            x: 260,
            y: 190,
            width: 140,
            height: 110,
        })
    );
}

#[test]
fn movement_damages_old_and_new_bounds_without_rebuilding_content() {
    let extent = SizeI {
        width: 800,
        height: 600,
    };
    let mut composition = ShellComposition::new(extent);
    let _ = composition.synchronize(
        extent,
        vec![image_layer(
            PointI { x: 10, y: 20 },
            ShellImageUpdate::Full(vec![255; 100 * 80 * 4].into()),
        )],
    );
    let frame = composition
        .synchronize(
            extent,
            vec![image_layer(
                PointI { x: 40, y: 20 },
                ShellImageUpdate::Unchanged,
            )],
        )
        .unwrap();
    assert_eq!(
        frame.damage,
        Some(RectI {
            x: 10,
            y: 20,
            width: 130,
            height: 80,
        })
    );
    assert!(frame.updates.is_empty());
}

#[test]
fn disjoint_image_updates_remain_disjoint_in_the_scene_delta() {
    let extent = SizeI {
        width: 800,
        height: 600,
    };
    let position = PointI { x: 20, y: 30 };
    let mut composition = ShellComposition::new(extent);
    let _ = composition.synchronize(
        extent,
        vec![image_layer(
            position,
            ShellImageUpdate::Full(vec![255; 100 * 80 * 4].into()),
        )],
    );
    let rects = [
        RectI {
            x: 4,
            y: 5,
            width: 8,
            height: 6,
        },
        RectI {
            x: 82,
            y: 67,
            width: 7,
            height: 5,
        },
    ];
    let mut layer = image_layer(
        position,
        ShellImageUpdate::Regions(
            rects
                .into_iter()
                .map(|rect| ShellImageRegion {
                    rect,
                    row_bytes: rect.width as usize * 4,
                    pixels: vec![128; rect.width as usize * rect.height as usize * 4].into(),
                })
                .collect(),
        ),
    );
    if let ShellLayerContent::Image {
        content_version, ..
    } = &mut layer.content
    {
        *content_version = 2;
    }
    let frame = composition.synchronize(extent, vec![layer]).unwrap();
    let delta = &frame.updates[0].deltas[0];
    let writes = delta
        .image_resources
        .iter()
        .filter_map(|update| match update {
            crate::graphics::render::ImageResourceDelta::Write(update) => Some(update.rect),
            crate::graphics::render::ImageResourceDelta::Remove(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(writes, rects);
}

#[test]
fn one_retained_scene_can_be_placed_more_than_once() {
    let extent = SizeI {
        width: 800,
        height: 600,
    };
    let mut composition = ShellComposition::new(extent);
    let layers = [10, 50]
        .into_iter()
        .enumerate()
        .map(|(index, x)| {
            ShellLayer::retained(
                ShellLayerKey::LegacyControl(index as u32, 0),
                ShellSceneKey::LegacyControl(0),
                Vec::new(),
                SizeI {
                    width: 24,
                    height: 24,
                },
                PointI { x, y: 10 },
                true,
            )
        })
        .collect();
    let frame = composition.synchronize(extent, layers).unwrap();
    assert_eq!(frame.live_scenes.len(), 1);
    assert_eq!(frame.placements.len(), 2);
    assert_eq!(frame.placements[0].scene, frame.placements[1].scene);
}

#[test]
fn hidden_image_revision_is_not_consumed_before_its_pixels_arrive() {
    let extent = SizeI {
        width: 800,
        height: 600,
    };
    let position = PointI { x: 20, y: 30 };
    let mut composition = ShellComposition::new(extent);
    let _ = composition.synchronize(
        extent,
        vec![image_layer(
            position,
            ShellImageUpdate::Full(vec![255; 100 * 80 * 4].into()),
        )],
    );

    let mut hidden = image_layer(position, ShellImageUpdate::Unchanged);
    hidden.visible = false;
    if let ShellLayerContent::Image {
        content_version, ..
    } = &mut hidden.content
    {
        *content_version = 2;
    }
    let _ = composition.synchronize(extent, vec![hidden]);

    let rect = RectI {
        x: 4,
        y: 5,
        width: 8,
        height: 6,
    };
    let mut visible = image_layer(
        position,
        ShellImageUpdate::Regions(vec![ShellImageRegion {
            rect,
            row_bytes: rect.width as usize * 4,
            pixels: vec![128; rect.width as usize * rect.height as usize * 4].into(),
        }]),
    );
    if let ShellLayerContent::Image {
        content_version, ..
    } = &mut visible.content
    {
        *content_version = 2;
    }
    let frame = composition.synchronize(extent, vec![visible]).unwrap();
    assert_eq!(frame.updates.len(), 1);
    assert_eq!(frame.updates[0].deltas[0].image_resources.len(), 1);
}

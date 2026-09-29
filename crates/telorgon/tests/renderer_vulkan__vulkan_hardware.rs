#![cfg(all(
    feature = "application-software",
    any(
        feature = "application-vulkan",
        feature = "shell-wayland-linux",
        feature = "embedded-vulkan"
    )
))]

use std::sync::Arc;
use std::time::Duration;

use telorgon::core::{ColorRgba8, RectF, RectI, SizeF, SizeI};
use telorgon::gpu_abi::{GpuBoxInstance, GpuSpatial};
use telorgon::layout::{ClipId, SpatialId};
use telorgon::render::{
    BatchKey, BlendMode, BoxInstance, DrawItem, GlyphInstance, ImageAlphaMode, ImageColorEncoding,
    ImageId, ImageInstance, ImageResource, MaterialInstance, MaterialKind, MaterialResource,
    PipelineKind, PrimitiveKind, ReadbackFormat, ReadbackRequest, RenderBackend, RenderClip,
    RenderRequest, RenderScene, RenderSpatialNode, TargetLoad, TargetStore,
};
use telorgon::renderer_software::{SoftwareRenderer, SoftwareSurface, SoftwareTarget};
use telorgon::renderer_vulkan::{
    DeviceSelection, OffscreenVulkanTarget, VulkanConfig, VulkanDevice, VulkanInstance,
};
use telorgon::scene::NodeId;
use telorgon::text::AtlasPageUpdate;
use telorgon::ui::MaterialId;

#[test]
#[ignore = "requires TELORGON_TEST_MODE=developer-hardware and a non-CPU Vulkan 1.3 adapter"]
fn box_shader_executes_on_real_vulkan_hardware() {
    assert_eq!(
        std::env::var("TELORGON_TEST_MODE").as_deref(),
        Ok("developer-hardware"),
        "hardware test must be selected explicitly"
    );
    let config = VulkanConfig {
        enable_validation: true,
        ..VulkanConfig::default()
    };
    let instance = VulkanInstance::load(&config, &[]).expect("load Vulkan 1.3 with validation");
    let adapters = instance.adapters().expect("enumerate Vulkan adapters");
    let selection = DeviceSelection::best(&adapters)
        .unwrap_or_else(|| panic!("no eligible non-CPU Vulkan adapter; reports: {adapters:#?}"));
    let device = VulkanDevice::create_owned(instance.clone(), &config, &selection, None)
        .expect("create Vulkan device");
    let target = OffscreenVulkanTarget::new(
        &device,
        SizeI {
            width: 16,
            height: 16,
        },
    )
    .expect("create offscreen target");

    let node = NodeId::new(0, 1);
    let mut source = RenderScene::default();
    source.extent = SizeF {
        width: 16.0,
        height: 16.0,
    };
    source.spatial_nodes.upsert(
        node,
        RenderSpatialNode {
            id: SpatialId(0),
            transform: telorgon::core::Affine2D::IDENTITY,
        },
    );
    let box_instance = BoxInstance {
        node,
        rect: RectF {
            x: 4.0,
            y: 4.0,
            width: 8.0,
            height: 8.0,
        },
        view_bounds: RectF {
            x: 4.0,
            y: 4.0,
            width: 8.0,
            height: 8.0,
        },
        background: Some(ColorRgba8::rgba(255, 0, 0, 255)),
        border: Default::default(),
        outline: Default::default(),
        corner_radii: Default::default(),
        shadows: Default::default(),
        opacity: 1.0,
        clip: ClipId(0),
        spatial: SpatialId(0),
    };
    source.boxes.upsert(node, box_instance.clone());
    source.set_draw_order(vec![DrawItem {
        kind: PrimitiveKind::Box,
        index: 0,
        batch: BatchKey {
            pipeline: PipelineKind::AnalyticBox,
            resource: 0,
            clip: ClipId(0),
            blend: BlendMode::Alpha,
            target: 0,
        },
    }]);
    let delta = source.take_delta().expect("box scene delta");
    let mut scene = device.create_scene().expect("create Vulkan scene");
    device
        .apply_scene_delta(&mut scene, &delta)
        .expect("apply Vulkan scene delta");

    let mut frame = device.begin_owned_frame().expect("begin Vulkan frame");
    let (initial_stats, pending) = {
        let mut context = frame.context_mut();
        let stats = device
            .render(
                &mut scene,
                &mut context,
                &target.target(),
                &RenderRequest {
                    force: true,
                    load: TargetLoad::Clear(ColorRgba8::rgba(0, 0, 0, 255)),
                    store: TargetStore::Store,
                    region: None,
                },
            )
            .expect("record Vulkan box rendering");
        let pending = context
            .record_readback(
                &target.target(),
                &ReadbackRequest {
                    region: RectI {
                        x: 0,
                        y: 0,
                        width: 16,
                        height: 16,
                    },
                    format: ReadbackFormat::Rgba8,
                },
            )
            .expect("record Vulkan readback");
        (stats, pending)
    };
    assert_eq!(initial_stats.buffer_allocations, 3);
    assert_eq!(initial_stats.buffer_copies, 3);
    assert_eq!(initial_stats.descriptor_writes, 3);
    assert_eq!(
        initial_stats.upload_bytes_recorded,
        (size_of::<GpuBoxInstance>() + size_of::<GpuSpatial>() + size_of::<u32>()) as u64
    );
    assert_eq!(initial_stats.batches, 1);
    assert_eq!(initial_stats.draws, 1);
    assert_eq!(scene.metrics().buffer_allocations, 3);
    assert_eq!(scene.metrics().buffer_growths, 0);
    assert!(device.memory_metrics().device_local_reserved_bytes > 0);
    let receipt = frame
        .finish()
        .expect("finish Vulkan frame")
        .submit()
        .expect("submit Vulkan frame");
    let image = pending
        .bind_to_submission(receipt)
        .expect("bind readback completion")
        .wait(Duration::from_secs(10))
        .expect("complete Vulkan readback");

    assert_eq!(
        &image.pixels[(8 * 16 + 8) * 4..(8 * 16 + 8) * 4 + 4],
        &[255, 0, 0, 255]
    );
    assert_eq!(&image.pixels[0..4], &[0, 0, 0, 255]);

    let mut changed_box = box_instance;
    changed_box.opacity = 0.5;
    source.boxes.upsert(node, changed_box);
    let changed_delta = source.take_delta().expect("single box property delta");
    let update = device
        .apply_scene_delta(&mut scene, &changed_delta)
        .expect("apply single box property delta");
    assert_eq!(
        update.upload_bytes_queued,
        size_of::<GpuBoxInstance>() as u64
    );
    assert_eq!(update.descriptor_writes_queued, 0);

    let mut changed_frame = device
        .begin_owned_frame()
        .expect("begin changed retained frame");
    let changed_stats = {
        let mut context = changed_frame.context_mut();
        device
            .render(
                &mut scene,
                &mut context,
                &target.target(),
                &RenderRequest {
                    force: true,
                    load: TargetLoad::Clear(ColorRgba8::rgba(0, 0, 0, 255)),
                    store: TargetStore::Store,
                    region: None,
                },
            )
            .expect("record single-property retained update")
    };
    assert_eq!(
        changed_stats.upload_bytes_recorded,
        size_of::<GpuBoxInstance>() as u64
    );
    assert_eq!(changed_stats.buffer_copies, 1);
    assert_eq!(changed_stats.buffer_allocations, 0);
    assert_eq!(changed_stats.descriptor_writes, 0);
    let mut changed_receipt = changed_frame
        .finish()
        .expect("finish changed retained frame")
        .submit()
        .expect("submit changed retained frame");
    changed_receipt
        .wait(Duration::from_secs(10))
        .expect("complete changed retained frame");

    let mut warm_frame = device
        .begin_owned_frame()
        .expect("begin warm retained frame");
    let warm_stats = {
        let mut context = warm_frame.context_mut();
        device
            .render(
                &mut scene,
                &mut context,
                &target.target(),
                &RenderRequest {
                    force: true,
                    load: TargetLoad::Clear(ColorRgba8::rgba(0, 0, 0, 255)),
                    store: TargetStore::Store,
                    region: None,
                },
            )
            .expect("record warm retained frame")
    };
    assert_eq!(warm_stats.upload_bytes_recorded, 0);
    assert_eq!(warm_stats.buffer_copies, 0);
    assert_eq!(warm_stats.buffer_allocations, 0);
    assert_eq!(warm_stats.descriptor_writes, 0);
    assert_eq!(scene.metrics().buffer_allocations, 3);
    assert_eq!(scene.metrics().buffer_growths, 0);
    let mut warm_receipt = warm_frame
        .finish()
        .expect("finish warm retained frame")
        .submit()
        .expect("submit warm retained frame");
    warm_receipt
        .wait(Duration::from_secs(10))
        .expect("complete warm retained frame");

    let deferred = device
        .begin_owned_frame()
        .expect("begin deferred-retirement frame")
        .finish()
        .expect("finish deferred-retirement frame")
        .submit()
        .expect("submit deferred-retirement frame");
    assert!(deferred.completion().value() > 0);
    drop(deferred);
    drop(
        device
            .begin_owned_frame()
            .expect("begin frame while prior resources retire asynchronously"),
    );
    assert_eq!(
        instance.diagnostics().error_count(),
        0,
        "validation messages: {:#?}",
        instance.diagnostics().messages()
    );
    assert_eq!(
        instance.diagnostics().warning_count(),
        0,
        "validation messages: {:#?}",
        instance.diagnostics().messages()
    );
    println!(
        "TELORGON_EVIDENCE case=vulkan.retained-box-dirty-upload-and-warm-reuse layer=E4 outcome=pass changed_upload_bytes={} changed_buffer_copies={} changed_allocations={} warm_upload_bytes={} warm_allocations={} validation_errors=0",
        changed_stats.upload_bytes_recorded,
        changed_stats.buffer_copies,
        changed_stats.buffer_allocations,
        warm_stats.upload_bytes_recorded,
        warm_stats.buffer_allocations,
    );
}

#[test]
#[ignore = "requires TELORGON_TEST_MODE=developer-hardware and a non-CPU Vulkan 1.3 adapter"]
fn mixed_scene_matches_the_software_reference_on_real_vulkan_hardware() {
    assert_eq!(
        std::env::var("TELORGON_TEST_MODE").as_deref(),
        Ok("developer-hardware"),
        "hardware test must be selected explicitly"
    );
    let config = VulkanConfig {
        enable_validation: true,
        ..VulkanConfig::default()
    };
    let instance = VulkanInstance::load(&config, &[]).expect("load Vulkan 1.3 with validation");
    let adapters = instance.adapters().expect("enumerate Vulkan adapters");
    let selection = DeviceSelection::best(&adapters)
        .unwrap_or_else(|| panic!("no eligible non-CPU Vulkan adapter; reports: {adapters:#?}"));
    let device = VulkanDevice::create_owned(instance.clone(), &config, &selection, None)
        .expect("create Vulkan device");
    let extent_i = SizeI {
        width: 16,
        height: 8,
    };
    let target = OffscreenVulkanTarget::new(&device, extent_i).expect("create offscreen target");

    let mut source = RenderScene::default();
    source.extent = SizeF {
        width: 16.0,
        height: 8.0,
    };
    let box_node = NodeId::new(0, 1);
    let image_node = NodeId::new(1, 1);
    let glyph_node = NodeId::new(2, 1);
    let material_node = NodeId::new(3, 1);
    source.spatial_nodes.upsert(
        box_node,
        RenderSpatialNode {
            id: SpatialId(0),
            transform: telorgon::core::Affine2D::IDENTITY,
        },
    );
    source.clips.upsert(
        material_node,
        RenderClip {
            id: ClipId(1),
            rect: RectF {
                x: 7.0,
                y: 1.0,
                width: 7.0,
                height: 2.0,
            },
            corner_radii: telorgon::ui::CornerRadii::all(1.0),
        },
    );
    source.boxes.upsert(
        box_node,
        BoxInstance {
            node: box_node,
            rect: RectF {
                x: 0.0,
                y: 0.0,
                width: 16.0,
                height: 8.0,
            },
            view_bounds: RectF {
                x: 0.0,
                y: 0.0,
                width: 16.0,
                height: 8.0,
            },
            background: Some(ColorRgba8::rgba(96, 96, 96, 255)),
            border: Default::default(),
            outline: Default::default(),
            corner_radii: telorgon::ui::CornerRadii::all(2.0),
            shadows: Default::default(),
            opacity: 1.0,
            clip: ClipId(0),
            spatial: SpatialId(0),
        },
    );
    source
        .set_image_resource(ImageResource {
            image: ImageId(9),
            content_version: 1,
            extent: SizeI {
                width: 2,
                height: 2,
            },
            color_encoding: ImageColorEncoding::Srgb,
            alpha_mode: ImageAlphaMode::Opaque,
            pixel_format: telorgon::render::ImagePixelFormat::Rgba8,
            pixels: Arc::from([
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            ]),
        })
        .unwrap();
    source.images.upsert(
        image_node,
        ImageInstance {
            node: image_node,
            image: ImageId(9),
            tint: None,
            rect: RectF {
                x: 1.0,
                y: 1.0,
                width: 2.0,
                height: 2.0,
            },
            view_bounds: RectF {
                x: 1.0,
                y: 1.0,
                width: 2.0,
                height: 2.0,
            },
            content_version: 1,
            opacity: 1.0,
            clip: ClipId(0),
            spatial: SpatialId(0),
        },
    );
    source.set_atlas_updates(
        SizeI {
            width: 2,
            height: 2,
        },
        vec![AtlasPageUpdate {
            page: 0,
            x: 0,
            y: 0,
            width: 2,
            height: 2,
            pixels_a8: Arc::from([255, 255, 255, 255]),
        }],
    );
    source.set_glyphs(vec![GlyphInstance {
        node: glyph_node,
        rect: RectF {
            x: 4.0,
            y: 1.0,
            width: 2.0,
            height: 2.0,
        },
        view_bounds: RectF {
            x: 4.0,
            y: 1.0,
            width: 2.0,
            height: 2.0,
        },
        atlas_x: 0,
        atlas_y: 0,
        atlas_size: SizeI {
            width: 2,
            height: 2,
        },
        color: ColorRgba8::rgba(255, 255, 0, 255),
        opacity: 1.0,
        clip: ClipId(0),
        spatial: SpatialId(0),
    }]);
    source.set_material_resource(MaterialResource {
        material: MaterialId(5),
        content_version: 1,
        kind: MaterialKind::LinearGradientHorizontal,
        colors: [
            ColorRgba8::rgba(255, 0, 0, 255),
            ColorRgba8::rgba(0, 0, 255, 255),
        ],
    });
    source.materials.upsert(
        material_node,
        MaterialInstance {
            node: material_node,
            material: MaterialId(5),
            rect: RectF {
                x: 7.0,
                y: 1.0,
                width: 8.0,
                height: 2.0,
            },
            view_bounds: RectF {
                x: 7.0,
                y: 1.0,
                width: 8.0,
                height: 2.0,
            },
            opacity: 1.0,
            clip: ClipId(1),
            spatial: SpatialId(0),
        },
    );
    source.set_draw_order(vec![
        draw(
            PrimitiveKind::Box,
            PipelineKind::AnalyticBox,
            0,
            0,
            ClipId(0),
        ),
        draw(PrimitiveKind::Image, PipelineKind::Image, 0, 9, ClipId(0)),
        draw(PrimitiveKind::Glyph, PipelineKind::Glyph, 0, 0, ClipId(0)),
        draw(
            PrimitiveKind::Material,
            PipelineKind::Material,
            0,
            5,
            ClipId(1),
        ),
    ]);
    let delta = source.take_delta().expect("mixed scene delta");

    let software = SoftwareRenderer;
    let mut software_scene = software.create_scene().unwrap();
    software
        .apply_scene_delta(&mut software_scene, &delta)
        .unwrap();
    let mut software_surface = SoftwareSurface::default();
    let software_target = SoftwareTarget::new(telorgon::render::RenderTargetInfo::full(extent_i));
    {
        let mut frame = software_surface.begin_frame();
        software
            .render(
                &mut software_scene,
                &mut frame,
                &software_target,
                &RenderRequest {
                    force: true,
                    load: TargetLoad::Clear(ColorRgba8::rgba(0, 0, 0, 255)),
                    store: TargetStore::Store,
                    region: None,
                },
            )
            .unwrap();
    }

    let mut scene = device.create_scene().expect("create Vulkan scene");
    device
        .apply_scene_delta(&mut scene, &delta)
        .expect("apply mixed Vulkan delta");
    let mut frame = device
        .begin_owned_frame()
        .expect("begin mixed Vulkan frame");
    let pending = {
        let mut context = frame.context_mut();
        let stats = device
            .render(
                &mut scene,
                &mut context,
                &target.target(),
                &RenderRequest {
                    force: true,
                    load: TargetLoad::Clear(ColorRgba8::rgba(0, 0, 0, 255)),
                    store: TargetStore::Store,
                    region: None,
                },
            )
            .expect("record mixed Vulkan rendering");
        assert_eq!(stats.draws, 4);
        context
            .record_readback(
                &target.target(),
                &ReadbackRequest {
                    region: RectI {
                        x: 0,
                        y: 0,
                        width: 16,
                        height: 8,
                    },
                    format: ReadbackFormat::Rgba8,
                },
            )
            .expect("record mixed Vulkan readback")
    };
    let receipt = frame.finish().unwrap().submit().unwrap();
    let hardware = pending
        .bind_to_submission(receipt)
        .unwrap()
        .wait(Duration::from_secs(10))
        .unwrap();
    let converted = linear_rgba_to_srgba(&hardware.pixels);
    let reference = software_surface.pixels_rgba8();
    let maximum_error = converted
        .iter()
        .zip(reference)
        .map(|(actual, expected)| actual.abs_diff(*expected))
        .max()
        .unwrap_or(0);
    assert!(
        maximum_error <= 3,
        "maximum mixed-scene channel error was {maximum_error}"
    );
    assert_eq!(
        instance.diagnostics().error_count(),
        0,
        "validation messages: {:#?}",
        instance.diagnostics().messages()
    );
    println!(
        "TELORGON_EVIDENCE case=vulkan.mixed-box-glyph-image-material-clip-spatial-reference layer=E4 outcome=pass draws=4 max_channel_error={maximum_error} validation_errors=0"
    );
}

#[test]
#[ignore = "requires TELORGON_TEST_MODE=developer-hardware and a non-CPU Vulkan 1.3 adapter"]
fn flush_controls_have_no_diagonal_or_edge_seams_under_rounded_clipping() {
    assert_eq!(
        std::env::var("TELORGON_TEST_MODE").as_deref(),
        Ok("developer-hardware"),
        "hardware test must be selected explicitly"
    );
    let config = VulkanConfig {
        enable_validation: true,
        ..VulkanConfig::default()
    };
    let instance = VulkanInstance::load(&config, &[]).unwrap();
    let adapters = instance.adapters().unwrap();
    let selection = DeviceSelection::best(&adapters)
        .unwrap_or_else(|| panic!("no eligible non-CPU Vulkan adapter; reports: {adapters:#?}"));
    let device = VulkanDevice::create_owned(instance.clone(), &config, &selection, None).unwrap();

    // The bright backing exposes even a single partially covered pixel along the two
    // triangles' shared diagonal. Odd extents and fractional origins vary helper coverage.
    for width in [200, 201] {
        for scale in [1.0_f32, 1.25, 1.5, 2.0, 3.0] {
            for inset in [1.0, 1.5] {
                for radius in [0.0, 12.0] {
                    let extent = SizeI {
                        width: (width as f32 * scale).round() as i32,
                        height: (100.0 * scale).round() as i32,
                    };
                    let logical = SizeF {
                        width: width as f32,
                        height: 100.0,
                    };
                    let target = OffscreenVulkanTarget::new(&device, extent).unwrap();
                    let mut source = RenderScene::default();
                    source.extent = logical;
                    let root = NodeId::new(0, 1);
                    source.spatial_nodes.upsert(
                        root,
                        RenderSpatialNode {
                            id: SpatialId(0),
                            transform: telorgon::core::Affine2D::IDENTITY,
                        },
                    );
                    let clip_rect = RectF {
                        x: inset,
                        y: inset,
                        width: logical.width - 2.0 * inset,
                        height: logical.height - 2.0 * inset,
                    };
                    source.clips.upsert(
                        root,
                        RenderClip {
                            id: ClipId(1),
                            rect: clip_rect,
                            corner_radii: telorgon::ui::CornerRadii::all(radius),
                        },
                    );
                    let mut controls = Vec::new();
                    for index in 0..3 {
                        let node = NodeId::new(index + 1, 1);
                        let rect = RectF {
                            x: logical.width - inset - 40.0 - (2 - index) as f32 * 40.0,
                            y: inset,
                            width: 40.0,
                            height: 31.0,
                        };
                        source.boxes.upsert(
                            node,
                            BoxInstance {
                                node,
                                rect,
                                view_bounds: rect,
                                background: Some(ColorRgba8::rgba(0, 0, 255, 255)),
                                border: Default::default(),
                                outline: Default::default(),
                                corner_radii: Default::default(),
                                shadows: Default::default(),
                                opacity: 1.0,
                                clip: ClipId(1),
                                spatial: SpatialId(0),
                            },
                        );
                        controls.push(rect);
                    }
                    source.set_draw_order(
                        (0..3)
                            .map(|index| {
                                draw(
                                    PrimitiveKind::Box,
                                    PipelineKind::AnalyticBox,
                                    index,
                                    0,
                                    ClipId(1),
                                )
                            })
                            .collect(),
                    );
                    let mut scene = device.create_scene().unwrap();
                    device
                        .apply_scene_delta(&mut scene, &source.take_delta().unwrap())
                        .unwrap();
                    let mut frame = device.begin_owned_frame().unwrap();
                    let pending = {
                        let mut context = frame.context_mut();
                        device
                            .render(
                                &mut scene,
                                &mut context,
                                &target.target(),
                                &RenderRequest {
                                    force: true,
                                    load: TargetLoad::Clear(ColorRgba8::rgba(0, 255, 0, 255)),
                                    store: TargetStore::Store,
                                    region: None,
                                },
                            )
                            .unwrap();
                        context
                            .record_readback(
                                &target.target(),
                                &ReadbackRequest {
                                    region: RectI {
                                        x: 0,
                                        y: 0,
                                        width: extent.width,
                                        height: extent.height,
                                    },
                                    format: ReadbackFormat::Rgba8,
                                },
                            )
                            .unwrap()
                    };
                    let receipt = frame.finish().unwrap().submit().unwrap();
                    let pixels = pending
                        .bind_to_submission(receipt)
                        .unwrap()
                        .wait(Duration::from_secs(10))
                        .unwrap()
                        .pixels;
                    let sx = extent.width as f32 / logical.width;
                    let sy = extent.height as f32 / logical.height;
                    let mut checked = [0; 3];
                    for y in 0..extent.height {
                        for x in 0..extent.width {
                            let px = x as f32 + 0.5;
                            let py = y as f32 + 0.5;
                            for (index, rect) in controls.iter().enumerate() {
                                // Include fully covered top/bottom rows and every interior pixel,
                                // including the shared diagonal; leave the curved AA band alone.
                                if px < rect.x * sx + 0.5
                                    || px > rect.right() * sx - 0.5
                                    || py < rect.y * sy + 0.5
                                    || py > rect.bottom() * sy - 0.5
                                    || (px > (clip_rect.right() - radius) * sx
                                        && py < (clip_rect.y + radius) * sy)
                                {
                                    continue;
                                }
                                let offset = ((y * extent.width + x) * 4) as usize;
                                assert_eq!(
                                    &pixels[offset..offset + 4],
                                    &[0, 0, 255, 255],
                                    "seam in control {index} at ({x},{y}): width={width}, scale={scale}, inset={inset}, radius={radius}"
                                );
                                checked[index] += 1;
                            }
                        }
                    }
                    assert!(checked.into_iter().all(|count| count > 500));
                }
            }
        }
    }
    assert_eq!(
        instance.diagnostics().error_count(),
        0,
        "validation messages: {:#?}",
        instance.diagnostics().messages()
    );
}

#[test]
#[ignore = "requires TELORGON_TEST_MODE=developer-hardware and a non-CPU Vulkan 1.3 adapter"]
fn rounded_border_coverage_matches_its_physical_width_with_a_matching_clip() {
    assert_eq!(
        std::env::var("TELORGON_TEST_MODE").as_deref(),
        Ok("developer-hardware")
    );
    let config = VulkanConfig {
        enable_validation: true,
        ..VulkanConfig::default()
    };
    let instance = VulkanInstance::load(&config, &[]).unwrap();
    let selection = DeviceSelection::best(&instance.adapters().unwrap()).unwrap();
    let device = VulkanDevice::create_owned(instance.clone(), &config, &selection, None).unwrap();
    for scale in [1.0, 1.25, 1.5, 2.0] {
        for width in [1.0, 2.0, 2.5] {
            for opacity in [0.5, 1.0] {
                let extent = SizeI {
                    width: (40.0 * scale) as i32,
                    height: (40.0 * scale) as i32,
                };
                let target = OffscreenVulkanTarget::new(&device, extent).unwrap();
                let mut source = RenderScene::default();
                source.extent = SizeF {
                    width: 40.0,
                    height: 40.0,
                };
                source.background = ColorRgba8::rgba(0, 0, 0, 0);
                let node = NodeId::new(0, 1);
                let rect = RectF {
                    x: 2.25,
                    y: 2.25,
                    width: 35.5,
                    height: 35.5,
                };
                let radii = telorgon::ui::CornerRadii::all(12.0);
                source.spatial_nodes.upsert(
                    node,
                    RenderSpatialNode {
                        id: SpatialId(0),
                        transform: telorgon::core::Affine2D::IDENTITY,
                    },
                );
                source.clips.upsert(
                    node,
                    RenderClip {
                        id: ClipId(1),
                        rect,
                        corner_radii: radii,
                    },
                );
                source.boxes.upsert(
                    node,
                    BoxInstance {
                        node,
                        rect,
                        view_bounds: rect,
                        background: None,
                        border: telorgon::render::Border::all(
                            width,
                            ColorRgba8::rgba(255, 0, 0, 255),
                        ),
                        outline: Default::default(),
                        corner_radii: radii,
                        shadows: Default::default(),
                        opacity,
                        clip: ClipId(1),
                        spatial: SpatialId(0),
                    },
                );
                source.set_draw_order(vec![draw(
                    PrimitiveKind::Box,
                    PipelineKind::AnalyticBox,
                    0,
                    0,
                    ClipId(1),
                )]);
                let mut scene = device.create_scene().unwrap();
                device
                    .apply_scene_delta(&mut scene, &source.take_delta().unwrap())
                    .unwrap();
                let mut frame = device.begin_owned_frame().unwrap();
                let pending = {
                    let mut context = frame.context_mut();
                    device
                        .render(
                            &mut scene,
                            &mut context,
                            &target.target(),
                            &RenderRequest {
                                force: true,
                                load: TargetLoad::Clear(ColorRgba8::rgba(0, 0, 0, 0)),
                                store: TargetStore::Store,
                                region: None,
                            },
                        )
                        .unwrap();
                    context
                        .record_readback(
                            &target.target(),
                            &ReadbackRequest {
                                region: RectI {
                                    x: 0,
                                    y: 0,
                                    width: extent.width,
                                    height: extent.height,
                                },
                                format: ReadbackFormat::Rgba8,
                            },
                        )
                        .unwrap()
                };
                let receipt = frame.finish().unwrap().submit().unwrap();
                let pixels = pending
                    .bind_to_submission(receipt)
                    .unwrap()
                    .wait(Duration::from_secs(10))
                    .unwrap()
                    .pixels;
                let outer = telorgon::render::RoundedClip::new(
                    RectF {
                        x: rect.x * scale,
                        y: rect.y * scale,
                        width: rect.width * scale,
                        height: rect.height * scale,
                    },
                    telorgon::ui::CornerRadii::all(12.0 * scale),
                );
                let inner = outer.inset(telorgon::render::Border::all(
                    width * scale,
                    ColorRgba8::rgba(255, 0, 0, 255),
                ));
                let mut fractional = 0;
                for y in 0..extent.height {
                    for x in 0..extent.width {
                        let point = telorgon::core::PointF {
                            x: x as f32 + 0.5,
                            y: y as f32 + 0.5,
                        };
                        let coverage = (outer.coverage(point) - inner.coverage(point)).max(0.0);
                        fractional += usize::from(coverage > 0.0 && coverage < 1.0);
                        let expected = (255.0 * opacity * coverage).round() as u8;
                        let actual = pixels[((y * extent.width + x) * 4 + 3) as usize];
                        assert!(
                            actual.abs_diff(expected) <= 2,
                            "border coverage at {x},{y}: {actual} != {expected}, scale={scale}, width={width}, opacity={opacity}"
                        );
                    }
                }
                assert!(fractional > 20, "fixture must exercise the curved AA bands");
            }
        }
    }
    assert_eq!(instance.diagnostics().error_count(), 0);
}

fn draw(
    kind: PrimitiveKind,
    pipeline: PipelineKind,
    index: u32,
    resource: u32,
    clip: ClipId,
) -> DrawItem {
    DrawItem {
        kind,
        index,
        batch: BatchKey {
            pipeline,
            resource,
            clip,
            blend: BlendMode::Alpha,
            target: 0,
        },
    }
}

fn linear_rgba_to_srgba(pixels: &[u8]) -> Vec<u8> {
    let mut converted = pixels.to_vec();
    for pixel in converted.chunks_exact_mut(4) {
        for channel in &mut pixel[..3] {
            let linear = f32::from(*channel) / 255.0;
            let srgb = if linear <= 0.003_130_8 {
                linear * 12.92
            } else {
                1.055 * linear.powf(1.0 / 2.4) - 0.055
            };
            *channel = (srgb * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    converted
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires TELORGON_TEST_MODE=developer-hardware and Vulkan validation"]
fn composite_culling_and_descriptor_pages_preserve_multipass_uploads() {
    use telorgon::renderer_vulkan::{VulkanCompositePlacement, VulkanCompositeScene};
    assert_eq!(
        std::env::var("TELORGON_TEST_MODE").as_deref(),
        Ok("developer-hardware")
    );
    let config = VulkanConfig {
        enable_validation: true,
        ..VulkanConfig::default()
    };
    let instance = VulkanInstance::load(&config, &[]).unwrap();
    let selection = DeviceSelection::best(&instance.adapters().unwrap()).unwrap();
    let device = VulkanDevice::create_owned(instance.clone(), &config, &selection, None).unwrap();
    let size = SizeI {
        width: 16,
        height: 16,
    };
    let targets = (0..258)
        .map(|_| OffscreenVulkanTarget::new(&device, size).unwrap())
        .collect::<Vec<_>>();
    let node = NodeId::new(0, 1);
    let mut source = RenderScene::default();
    source.extent = SizeF {
        width: 16.0,
        height: 16.0,
    };
    source.spatial_nodes.upsert(
        node,
        RenderSpatialNode {
            id: SpatialId(0),
            transform: telorgon::core::Affine2D::IDENTITY,
        },
    );
    let rect = RectF {
        x: 0.0,
        y: 0.0,
        width: 16.0,
        height: 16.0,
    };
    source.boxes.upsert(
        node,
        BoxInstance {
            node,
            rect,
            view_bounds: rect,
            background: Some(ColorRgba8::rgba(255, 0, 0, 255)),
            border: Default::default(),
            outline: Default::default(),
            corner_radii: Default::default(),
            shadows: Default::default(),
            opacity: 1.0,
            clip: ClipId(0),
            spatial: SpatialId(0),
        },
    );
    source.set_draw_order(vec![DrawItem {
        kind: PrimitiveKind::Box,
        index: 0,
        batch: BatchKey {
            pipeline: PipelineKind::AnalyticBox,
            resource: 0,
            clip: ClipId(0),
            blend: BlendMode::Opaque,
            target: 0,
        },
    }]);
    let mut scene = device.create_scene().unwrap();
    device
        .apply_scene_delta(&mut scene, &source.take_delta().unwrap())
        .unwrap();
    let mut recording = device.begin_owned_frame().unwrap();
    let pending = {
        let mut context = recording.context_mut();
        for (index, target) in targets.iter().enumerate() {
            let placement = VulkanCompositePlacement {
                scene_index: 0,
                target: RectI {
                    x: if index == 0 { 32 } else { 0 },
                    y: 0,
                    width: 16,
                    height: 16,
                },
                clip: None,
                rounded_clips: [None; 2],
            };
            let stats = device
                .render_composite(
                    &mut [VulkanCompositeScene { scene: &mut scene }],
                    &[placement],
                    &mut context,
                    &target.target(),
                    &RenderRequest {
                        force: true,
                        load: TargetLoad::Clear(ColorRgba8::rgba(0, 0, 0, 255)),
                        store: TargetStore::Store,
                        region: None,
                    },
                )
                .unwrap();
            if index == 0 {
                assert_eq!(stats.descriptor_writes, 0);
                assert_eq!(stats.upload_bytes_recorded, 0);
            } else if index == 1 {
                assert!(
                    stats.upload_bytes_recorded > 0,
                    "culled scene must retain its pending uploads"
                );
            } else {
                assert_eq!(stats.upload_bytes_recorded, 0);
            }
        }
        context
            .record_readback(
                &targets.last().unwrap().target(),
                &ReadbackRequest {
                    region: RectI {
                        x: 0,
                        y: 0,
                        width: 16,
                        height: 16,
                    },
                    format: ReadbackFormat::Rgba8,
                },
            )
            .unwrap()
    };
    let receipt = recording.finish().unwrap().submit().unwrap();
    let image = pending
        .bind_to_submission(receipt)
        .unwrap()
        .wait(Duration::from_secs(10))
        .unwrap();
    assert_eq!(&image.pixels[0..4], &[255, 0, 0, 255]);
    assert_eq!(
        instance.diagnostics().error_count(),
        0,
        "{:?}",
        instance.diagnostics().messages()
    );
}

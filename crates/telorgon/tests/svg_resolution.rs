use telorgon::app::*;
use telorgon::graphics::render::{ImageResourceDelta, ImageResourceUpdate};
use telorgon::platform::contracts::ScaleFactor;
use telorgon::{
    AssetEntry, AssetKind, AssetMediaCache, AssetRasterSize, ComposedAppRuntime, MonotonicInstant,
};

const ICON: IconAsset = IconAsset::new(AssetKey::new("icons/test.svg"));
static ENTRIES: [AssetEntry; 1] = [AssetEntry::embedded(ICON.key(), AssetKind::Icon, "image/svg+xml",
    br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24"><path d="M2 21 L12 3 L22 21 Z" fill="none" stroke="white" stroke-width="1.5"/></svg>"##)];

#[component]
struct Icons {}
impl Component for Icons {
    fn view(&self) -> impl View {
        row()
            .child(image(ICON).width(20.0).height(20.0))
            .child(image(ICON).height(Dimension::FILL).aspect_ratio(1.0))
    }
}

fn uploads(runtime: &mut ComposedAppRuntime) -> Vec<ImageResourceUpdate> {
    let mut updates = Vec::new();
    while let Some(delta) = runtime.pop_scene_delta() {
        for resource in delta.image_resources {
            if let ImageResourceDelta::Write(update) = resource {
                updates.push(update);
            }
        }
    }
    updates
}

#[test]
fn svg_tracks_largest_visible_size_density_and_resize_without_idle_uploads() {
    let bundle = AssetBundle::new(&ENTRIES);
    let mut runtime = ComposedAppRuntime::from_composed_with_extent(
        Icons::default(),
        SizeI {
            width: 240,
            height: 40,
        },
    )
    .unwrap();
    runtime.register_assets(bundle).unwrap();
    let mut expected = AssetMediaCache::new(bundle).unwrap();
    let mut version = 0;
    for (height, scale, pixels) in [(40, 1.0, 40), (40, 2.0, 80), (80, 2.0, 160), (40, 1.0, 40)] {
        runtime.resize(SizeI { width: 240, height }).unwrap();
        runtime.set_raster_scale(ScaleFactor::new(scale).unwrap());
        runtime.prepare_frame(MonotonicInstant::ZERO, true).unwrap();
        let changes = uploads(&mut runtime);
        let upload = changes
            .last()
            .expect("resolution changes must upload a raster");
        assert_eq!(
            upload.extent,
            SizeI {
                width: pixels,
                height: pixels
            }
        );
        assert!(upload.content_version > version);
        version = upload.content_version;
        let reference = expected
            .icon(
                ICON,
                Some(AssetRasterSize::new(pixels as u32, pixels as u32).unwrap()),
            )
            .unwrap();
        assert_eq!(
            upload.pixels.as_ref(),
            reference.render_resource().pixels.as_ref(),
            "must rasterize vectors at the requested size, not enlarge the intrinsic bitmap"
        );
        runtime.prepare_frame(MonotonicInstant::ZERO, true).unwrap();
        assert!(
            uploads(&mut runtime).is_empty(),
            "unchanged images must reuse their textures"
        );
    }
    runtime.register_assets(bundle).unwrap();
    runtime.prepare_frame(MonotonicInstant::ZERO, true).unwrap();
    assert!(uploads(&mut runtime).last().unwrap().content_version > version);
}

#[test]
fn transformed_svg_uses_basis_scale_not_rotated_bounding_box() {
    let mut runtime = ComposedAppRuntime::from_composed_with_extent(
        Icons::default(),
        SizeI {
            width: 240,
            height: 40,
        },
    )
    .unwrap();
    runtime.register_assets(AssetBundle::new(&ENTRIES)).unwrap();
    runtime.prepare_frame(MonotonicInstant::ZERO, true).unwrap();
    uploads(&mut runtime);
    let node = runtime.ui().images.iter().last().unwrap().0;
    let mut style = *runtime.ui().box_styles.get(node).unwrap();
    style.transform.scale.x = 2.0;
    style.transform.scale.y = 1.5;
    style.transform.rotation = std::f32::consts::FRAC_PI_2;
    runtime.ui_mut().set_box_style(node, style);
    runtime.prepare_frame(MonotonicInstant::ZERO, true).unwrap();
    let changes = uploads(&mut runtime);
    assert_eq!(
        changes.last().unwrap().extent,
        SizeI {
            width: 80,
            height: 60
        }
    );
}

#[test]
fn svg_edge_coverage_is_preserved_in_linear_blending_without_changing_native_pixels() {
    let mut cache = AssetMediaCache::new(AssetBundle::new(&ENTRIES)).unwrap();
    let decoded = cache
        .icon(ICON, Some(AssetRasterSize::new(40, 40).unwrap()))
        .unwrap();
    let resource = decoded.render_resource();
    let mut edges = 0;
    for (native, rendered) in decoded
        .pixels_rgba8
        .chunks_exact(4)
        .zip(resource.pixels.chunks_exact(4))
    {
        assert_eq!(native[3], rendered[3]);
        if native[3] > 10 && native[3] < 245 {
            edges += 1;
            let alpha = native[3] as f32 / 255.0;
            // The fixture is white: native premultiplied RGB equals coverage.
            assert_eq!(native[0], native[3]);
            let encoded = rendered[0] as f32 / 255.0;
            let linear = if encoded <= 0.04045 {
                encoded / 12.92
            } else {
                ((encoded + 0.055) / 1.055).powf(2.4)
            };
            assert!(
                (linear - alpha).abs() < 0.005,
                "white edge intensity must equal alpha coverage"
            );
        }
    }
    assert!(edges > 0);
}

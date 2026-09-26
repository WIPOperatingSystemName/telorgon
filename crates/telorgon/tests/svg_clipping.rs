use telorgon::app::*;
use telorgon::graphics::render::{
    RenderBackend, RenderRequest, RenderTargetInfo, TargetLoad, TargetStore,
};
use telorgon::graphics::renderers::software::{SoftwareRenderer, SoftwareSurface, SoftwareTarget};
use telorgon::{AssetEntry, AssetKind, ComposedAppRuntime, MonotonicInstant};

const ICON: IconAsset = IconAsset::new(AssetKey::new("audio.svg"));
static ASSETS: [AssetEntry; 1] = [AssetEntry::embedded(ICON.key(), AssetKind::Icon, "image/svg+xml",
    br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M2 10v3"/><path d="M6 6v11"/><path d="M10 3v18"/><path d="M14 8v7"/><path d="M18 5v13"/><path d="M22 10v3"/></svg>"##)];

#[component]
struct Nested {}
impl Component for Nested {
    fn view(&self) -> impl View {
        row().padding(16.0).child(
            column()
                .width(240.0)
                .padding(20.0)
                .corner_radius(12.0)
                .child(
                    button()
                        .height(48.0)
                        .width(Dimension::FILL)
                        .background(Background::None)
                        .child(
                            row()
                                .padding(4.0)
                                .align_items(Alignment::Center)
                                .child(image(ICON).height(Dimension::FILL).aspect_ratio(1.0)),
                        ),
                ),
        )
    }
}
#[component]
struct Unclipped {}
impl Component for Unclipped {
    fn view(&self) -> impl View {
        image(ICON).width(40.0).height(40.0)
    }
}

fn pixels(runtime: &mut ComposedAppRuntime, extent: SizeI) -> Vec<u8> {
    runtime.register_assets(AssetBundle::new(&ASSETS)).unwrap();
    runtime.prepare_frame(MonotonicInstant::ZERO, true).unwrap();
    let renderer = SoftwareRenderer;
    let mut scene = renderer.create_scene().unwrap();
    renderer
        .apply_scene_delta(&mut scene, &runtime.scene_snapshot())
        .unwrap();
    let mut surface = SoftwareSurface::default();
    let mut frame = surface.begin_frame();
    renderer
        .render(
            &mut scene,
            &mut frame,
            &SoftwareTarget::new(RenderTargetInfo::full(extent)),
            &RenderRequest {
                force: true,
                load: TargetLoad::Clear(ColorRgba8::rgba(255, 255, 255, 255)),
                store: TargetStore::Store,
                region: None,
            },
        )
        .unwrap();
    drop(frame);
    surface.pixels_rgba8().to_vec()
}

#[test]
fn rounded_svg_caps_survive_button_and_container_clips_pixel_for_pixel() {
    let extent = SizeI {
        width: 300,
        height: 160,
    };
    let mut nested =
        ComposedAppRuntime::from_composed_with_extent(Nested::default(), extent).unwrap();
    let output = pixels(&mut nested, extent);
    let image = nested.ui().images.iter().next().unwrap().0;
    let rect = nested.layout().computed(image).unwrap().content_rect;
    assert_eq!((rect.width, rect.height), (40.0, 40.0));
    let mut reference = ComposedAppRuntime::from_composed_with_extent(
        Unclipped::default(),
        SizeI {
            width: 40,
            height: 40,
        },
    )
    .unwrap();
    let expected = pixels(
        &mut reference,
        SizeI {
            width: 40,
            height: 40,
        },
    );
    for y in 0..40 {
        let start = ((rect.y as usize + y) * 300 + rect.x as usize) * 4;
        assert_eq!(
            &output[start..start + 160],
            &expected[y * 160..(y + 1) * 160],
            "SVG row {y} changed under ancestor clipping"
        );
    }
}

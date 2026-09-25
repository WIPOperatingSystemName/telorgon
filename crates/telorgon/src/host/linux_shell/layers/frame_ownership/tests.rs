use super::*;
use crate::authoring::compose::{Component, View, window_content_slot, window_frame};

#[crate::component]
struct BorderFrame {
    #[input]
    title_bar_visible: bool,
}

impl Component for BorderFrame {
    fn view(&self) -> impl View {
        window_frame()
            .decoration(crate::BoxDecoration::new()
                .border(crate::Border::all(4.0, crate::ColorRgba8::rgba(120, 80, 200, 255)))
                .corner_radius(20.0))
            .content_slot(window_content_slot().margin(crate::authoring::compose::Insets::new(
                if self.title_bar_visible { 37.0 } else { 0.0 }, 0.0, 0.0, 0.0,
            )))
    }
}

#[test]
fn tiled_client_styling_gets_a_clipped_frame_without_a_second_header() {
    use super::super::super::client::maximize_preview_tests::test_window;
    let declaration = crate::host::application::Compositor::new()
        .cursor_theme(crate::CursorTheme::new())
        .window_frame(|model: WindowChromeModel| BorderFrame {
            title_bar_visible: model.title_bar_visible,
        });
    let display = Display::new().unwrap();
    let mut wayland = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
    wayland.set_decoration_policy(crate::DecorationPolicy {
        tiled_client_decorations: true,
        ..crate::DecorationPolicy::DEFAULT
    });
    let surface = WaylandSurfaceId::from_raw(1).unwrap();
    let mut window = test_window(SizeI { width: 640, height: 480 }, PointI { x: 100, y: 80 });
    window.server_decorated = false;
    let mut windows = BTreeMap::from([(surface, window)]);
    let mut frames = BTreeMap::new();
    let config = LinuxShellConfig::default();
    let wake = EventNotifier::new("client border test").unwrap();
    let mut scheduler = ConfigureScheduler::default();
    let world = test_input_world(&[surface]);
    for fullscreen in [false, true, false] {
        windows.get_mut(&surface).unwrap().fullscreen = fullscreen;
        refresh_window_frames(
            declaration.frame_template(), &mut frames, &mut windows, &wayland, &config,
            &LayerAssets::new(AssetBundle::default()).unwrap(), &crate::AppIconProfile::default(), None, &wake, 0,
            crate::platform::contracts::ScaleFactor::new(1.0).unwrap(),
            RectI { x: 0, y: 0, width: 1280, height: 800 }, &mut scheduler,
        ).unwrap();
        if fullscreen {
            assert!(frames.is_empty());
            assert!(windows[&surface].chrome.is_none());
            continue;
        }
        let frame = &frames[&surface];
        assert!(!frame.model.title_bar_visible);
        assert_eq!(windows[&surface].chrome_content_offset, Some(PointI { x: 4, y: 4 }));
        assert_eq!(frame.outer, SizeI { width: 648, height: 488 });
        let layers = prepare_desktop_layers(
            false, SizeI { width: 1280, height: 800 }, 0, false,
            &mut frames, &mut windows, &[surface], &mut [], &mut [], &mut None,
            None, PointF::default(), None, PointF::default(), &config,
        ).unwrap();
        let client = layers.iter().find(|layer| layer.key == ShellLayerKey::Surface(surface.get())).unwrap();
        let clip = client.rounded_clips[0].expect("the app pixels must inherit the frame curve");
        assert_eq!(clip.radii, crate::ui::CornerRadii::all(16.0));
        assert!(hit_test_decoration(&world, &windows, &[surface], PointF { x: 140.0, y: 90.0 }, &config, &[]).is_none(),
            "the client header must remain client input");
    }
    synchronize(&mut windows, false, true);
    assert!(!window_has_frame(&windows[&surface]));
    assert!(windows[&surface].chrome.is_none());
}

use super::*;
use crate::authoring::compose::{Component, RuntimeTarget, View, window_content_slot, window_frame};

#[crate::component]
struct TestFrame {
    #[input]
    title_height: f32,
}

impl Component for TestFrame {
    fn view(&self) -> impl View {
        window_frame().content_slot(window_content_slot().margin(crate::authoring::compose::Insets::new(
            self.title_height,
            0.0,
            0.0,
            0.0,
        )))
    }
}

#[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
#[test]
fn decoration_policy_selects_native_and_x11_frame_parts_and_clears_stale_geometry() {
    use super::super::client::maximize_preview_tests::test_window;
    use crate::{DecorationNegotiation, FramePartPolicy, ResizeRegionPolicy};
    let declaration = crate::host::application::Compositor::new()
        .cursor_theme(crate::CursorTheme::new())
        .window_frame(|model: WindowChromeModel| TestFrame {
            title_height: if model.title_bar_visible { 37.0 } else { 0.0 },
        });
    let display = Display::new().unwrap();
    let wayland = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
    let config = LinuxShellConfig::default();
    let wake = EventNotifier::new("decoration policy test").unwrap();
    let area = RectI {
        x: 0,
        y: 0,
        width: 1280,
        height: 800,
    };
    let mut windows = BTreeMap::new();
    for index in 1..=3 {
        let mut window = test_window(
            SizeI {
                width: 640,
                height: 480,
            },
            PointI::default(),
        );
        if index == 2 {
            window.role = SurfaceRole::Xwayland;
            window.backend = Some(WindowBackend::X11(crate::integrations::x11::association::XWindow {
                generation: 1,
                xid: 10,
                incarnation: 1,
            }));
        } else if index == 3 {
            window.role = SurfaceRole::XdgPopup;
            window.backend = None;
        }
        window.server_decorated = false;
        windows.insert(WaylandSurfaceId::from_raw(index).unwrap(), window);
    }
    let mut frames = BTreeMap::new();
    let mut scheduler = ConfigureScheduler::default();
    for phase in 0..5 {
        for window in windows.values_mut() {
            window.decoration_policy = crate::DecorationPolicy::DEFAULT;
            if phase == 1 {
                window.decoration_policy.outer_frame.rounded_clip = FramePartPolicy::Always;
                window.decoration_policy.interaction.resize_regions =
                    ResizeRegionPolicy::Enabled;
            } else if phase == 2 {
                window.decoration_policy.negotiation = DecorationNegotiation::PreferServer;
            }
            window.fullscreen = phase == 3;
        }
        refresh_window_frames(
            declaration.frame_template(),
            &mut frames,
            &mut windows,
            &wayland,
            &config,
            AssetBundle::default(),
            &crate::AppIconProfile::default(),
            None,
            &wake,
            phase,
            crate::platform::contracts::ScaleFactor::new(1.0).unwrap(),
            area,
            &mut scheduler,
        )
        .unwrap();
        assert!(!frames.contains_key(&WaylandSurfaceId::from_raw(3).unwrap()));
        if phase == 1 || phase == 2 {
            assert_eq!(frames.len(), 2);
            for frame in frames.values() {
                assert_eq!(frame.model.title_bar_visible, phase == 2);
                assert_eq!(frame.model.frame_parts.border, phase == 2);
                assert!(frame.model.frame_parts.rounded_clip);
                assert!(frame.model.frame_parts.resize_regions);
                assert_eq!(
                    frame.snapshot.as_ref().unwrap().content.bounds.y,
                    if phase == 2 { 37.0 } else { 0.0 }
                );
            }
        } else {
            assert!(frames.is_empty());
            for window in windows.values() {
                assert!(window.chrome.is_none());
                assert!(window.chrome_outer.is_none());
                assert_eq!(window_content_offset(window, &config), PointI::default());
            }
        }
    }
}

#[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
#[test]
fn both_backends_draw_the_same_rgba_veil_and_hide_client_subtrees() {
    use super::super::client::maximize_preview_tests::test_window;
    use super::super::scene::ShellLayerContent;
    let root = WaylandSurfaceId::from_raw(20).unwrap();
    let child = WaylandSurfaceId::from_raw(21).unwrap();
    let color = crate::foundation::ColorRgba8::rgba(20, 40, 60, 96);
    let config = LinuxShellConfig {
        resize_preview: crate::ResizePreviewDesign::new(crate::Fill::Color(color)),
        ..Default::default()
    };
    for backend in [
        WindowBackend::Wayland,
        WindowBackend::X11(crate::integrations::x11::association::XWindow {
            generation: 1,
            xid: 10,
            incarnation: 1,
        }),
    ] {
        let mut image = test_window(
            SizeI {
                width: 640,
                height: 480,
            },
            PointI { x: 100, y: 100 },
        );
        image.backend = Some(backend);
        if matches!(backend, WindowBackend::X11(_)) {
            image.role = SurfaceRole::Xwayland;
        }
        let mut sub = test_window(
            SizeI {
                width: 30,
                height: 20,
            },
            PointI { x: 110, y: 140 },
        );
        sub.role = SurfaceRole::Subsurface;
        sub.backend = None;
        sub.server_decorated = false;
        sub.parent = Some(root);
        sub.offset = PointI { x: 10, y: 10 };
        let mut windows = BTreeMap::from([(root, image), (child, sub)]);
        let mut scheduler = ConfigureScheduler::default();
        let declaration = crate::host::application::Compositor::new()
            .cursor_theme(crate::CursorTheme::new())
            .window_frame(|_: WindowChromeModel| TestFrame { title_height: 37.0 });
        let display = Display::new().unwrap();
        let wayland = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
        let mut frames = BTreeMap::new();
        refresh_window_frames(
            declaration.frame_template(),
            &mut frames,
            &mut windows,
            &wayland,
            &config,
            AssetBundle::default(),
            &crate::AppIconProfile::default(),
            None,
            &EventNotifier::new("whole window preview").unwrap(),
            0,
            crate::platform::contracts::ScaleFactor::new(1.0).unwrap(),
            RectI {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            &mut scheduler,
        )
        .unwrap();
        assert!(frames.contains_key(&root));
        let interaction = WindowInteraction::begin_resize(
            &mut windows,
            &mut scheduler,
            root,
            ResizeEdge::BottomRight,
            PointF::default(),
        )
        .unwrap();
        for finishing in [false, true] {
            if finishing {
                finish_window_interaction(&mut windows, &mut scheduler, interaction);
            }
            assert_eq!(resize_veil_owner(&windows, child), Some(root));
            let layers = prepare_desktop_layers(
                false,
                SizeI {
                    width: 1920,
                    height: 1080,
                },
                0,
                true,
                &mut frames,
                &mut windows,
                &[root, child],
                &mut [],
                &mut [],
                &mut None,
                None,
                PointF::default(),
                None,
                PointF::default(),
                &config,
            )
            .unwrap();
            let veil = layers
                .iter()
                .find(|layer| layer.key == ShellLayerKey::ResizeVeil(root.get()))
                .unwrap();
            assert!(veil.visible);
            let outer = windows[&root]
                .chrome_outer
                .unwrap_or_else(|| legacy_window_outer(&windows[&root], &config));
            assert_eq!(
                veil.target,
                RectI {
                    x: windows[&root].position.x,
                    y: windows[&root].position.y,
                    width: outer.width,
                    height: outer.height,
                }
            );
            assert!(layers.iter().all(|layer| {
                !matches!(
                    layer.key,
                    ShellLayerKey::Frame(_, _) | ShellLayerKey::LegacyControl(_, _)
                ) || !layer.visible
            }));
            assert!(
                matches!(veil.content, ShellLayerContent::Solid { color: actual, .. } if actual == color)
            );
            for surface in [root, child] {
                let image = layers
                    .iter()
                    .find(|layer| layer.key == ShellLayerKey::Surface(surface.get()))
                    .unwrap();
                assert!(!image.visible);
                assert!(matches!(
                    image.content,
                    ShellLayerContent::Image {
                        update: ShellImageUpdate::Unchanged,
                        ..
                    }
                ));
            }
            assert_eq!(
                layers
                    .iter()
                    .filter(|layer| matches!(layer.key, ShellLayerKey::ResizeVeil(_)))
                    .count(),
                1
            );
        }
    }
}

#[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
#[test]
fn native_and_x11_windows_use_the_same_composed_frame_template() {
    use super::super::client::maximize_preview_tests::test_window;
    let declaration = crate::host::application::Compositor::new()
        .cursor_theme(crate::CursorTheme::new())
        .window_frame(|_: WindowChromeModel| TestFrame { title_height: 37.0 });
    let display = Display::new().unwrap();
    let wayland = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
    let mut registry = crate::integrations::x11::window::Windows::new(1, 16, 1, 2, 100).unwrap();
    let token = registry.begin_inspection(10).unwrap().unwrap();
    registry
        .finish_inspection(
            token,
            1,
            crate::integrations::x11::window::Geometry {
                x: 100,
                y: 100,
                width: 640,
                height: 480,
                border: 0,
            },
            false,
            true,
        )
        .unwrap();
    let id = registry.get(10).unwrap().id;
    let native = WaylandSurfaceId::from_raw(20).unwrap();
    let x11 = WaylandSurfaceId::from_raw(21).unwrap();
    let popup = WaylandSurfaceId::from_raw(22).unwrap();
    let mut windows = BTreeMap::new();
    for (surface, backend, role) in [
        (
            native,
            Some(WindowBackend::Wayland),
            SurfaceRole::XdgToplevel,
        ),
        (x11, Some(WindowBackend::X11(id)), SurfaceRole::Xwayland),
        (popup, None, SurfaceRole::Xwayland),
    ] {
        let mut window = test_window(
            SizeI {
                width: 640,
                height: 480,
            },
            PointI { x: 100, y: 100 },
        );
        window.backend = backend;
        window.role = role;
        window.server_decorated = backend.is_some();
        window.frame_title = Some("Same application title".into());
        windows.insert(surface, window);
    }
    let mut frames = BTreeMap::new();
    refresh_window_frames(
        declaration.frame_template(),
        &mut frames,
        &mut windows,
        &wayland,
        &LinuxShellConfig::default(),
        AssetBundle::default(),
        &crate::AppIconProfile::default(),
        None,
        &EventNotifier::new("frame parity").unwrap(),
        0,
        crate::platform::contracts::ScaleFactor::new(1.5).unwrap(),
        RectI {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        },
        &mut ConfigureScheduler::default(),
    )
    .unwrap();
    assert_eq!(frames.len(), 2);
    assert!(!frames.contains_key(&popup));
    assert_eq!(frames[&native].outer, frames[&x11].outer);
    assert_eq!(
        windows[&native].chrome_content_offset,
        windows[&x11].chrome_content_offset
    );
    assert_eq!(windows[&x11].chrome_content_offset.unwrap().y, 37);
    assert_eq!(
        frames[&native].snapshot.as_ref().unwrap().content.bounds,
        frames[&x11].snapshot.as_ref().unwrap().content.bounds
    );

    // A completed dense X11 buffer must not uncover the old retained frame.
    let target = SizeI {
        width: 800,
        height: 550,
    };
    let pixels = SizeI {
        width: 2400,
        height: 1650,
    };
    let window = windows.get_mut(&x11).unwrap();
    window.surface_scale = 3;
    window.resize_preview.begin(
        window.position,
        window.requested_size,
        ResizeEdge::BottomRight,
    );
    window.resize_preview.finish();
    window.resize_preview.submitted(pixels, 7, false);
    window.requested_size = target;
    window.presentation.size = pixels;
    window.presentation.revision = 8;
    assert!(!super::super::x11_windows::settle_resize(
        window, pixels, false
    ));
    assert!(window.resize_veil_active());

    refresh_window_frames(
        declaration.frame_template(),
        &mut frames,
        &mut windows,
        &wayland,
        &LinuxShellConfig::default(),
        AssetBundle::default(),
        &crate::AppIconProfile::default(),
        None,
        &EventNotifier::new("resize frame").unwrap(),
        1,
        crate::platform::contracts::ScaleFactor::new(1.5).unwrap(),
        RectI {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        },
        &mut ConfigureScheduler::default(),
    )
    .unwrap();
    let window = windows.get_mut(&x11).unwrap();
    assert!(super::super::x11_windows::settle_resize(
        window, pixels, false
    ));
    assert!(!window.resize_veil_active());
    let content = frames[&x11].snapshot.as_ref().unwrap().content.bounds;
    assert_eq!((content.width, content.height), (800.0, 550.0));
}

#[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
#[test]
fn measured_maximized_x11_frame_restarts_the_resize_veil() {
    use super::super::client::maximize_preview_tests::test_window;
    let declaration = crate::host::application::Compositor::new()
        .cursor_theme(crate::CursorTheme::new())
        .window_frame(|_: WindowChromeModel| TestFrame { title_height: 53.0 });
    let display = Display::new().unwrap();
    let wayland = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
    let surface = WaylandSurfaceId::from_raw(20).unwrap();
    let mut window = test_window(
        SizeI {
            width: 640,
            height: 480,
        },
        PointI::default(),
    );
    window.role = SurfaceRole::Xwayland;
    window.backend = Some(WindowBackend::X11(crate::integrations::x11::association::XWindow {
        generation: 1,
        xid: 10,
        incarnation: 1,
    }));
    let mut windows = BTreeMap::from([(surface, window)]);
    let mut scheduler = ConfigureScheduler::default();
    let area = RectI {
        x: 0,
        y: 0,
        width: 1280,
        height: 800,
    };
    let config = LinuxShellConfig::default();
    set_window_maximized(&mut windows, &mut scheduler, surface, true, area, &config).unwrap();
    // Simulate the fallback-sized image arriving before the frame is measured.
    windows.get_mut(&surface).unwrap().resize_preview = Default::default();
    refresh_window_frames(
        declaration.frame_template(),
        &mut BTreeMap::new(),
        &mut windows,
        &wayland,
        &config,
        AssetBundle::default(),
        &crate::AppIconProfile::default(),
        None,
        &EventNotifier::new("maximized veil").unwrap(),
        0,
        crate::platform::contracts::ScaleFactor::new(1.0).unwrap(),
        area,
        &mut scheduler,
    )
    .unwrap();
    assert_eq!(windows[&surface].requested_size.height, 747);
    assert_eq!(resize_veil_owner(&windows, surface), Some(surface));
    assert!(windows[&surface].native_configure.resize_final.is_none());
}

#[test]
fn tile_outer_geometry_measures_the_actual_custom_content_slot() {
    let backends = [
        WindowBackend::Wayland,
        #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
        WindowBackend::X11(crate::integrations::x11::association::XWindow {
            generation: 1,
            xid: 10,
            incarnation: 1,
        }),
    ];
    for backend in backends {
        for interactive in [false, true] {
            use super::super::client::maximize_preview_tests::test_window;
            let declaration = crate::host::application::Compositor::new()
                .cursor_theme(crate::CursorTheme::new())
                .window_frame(|model: WindowChromeModel| {
                    assert_eq!(model.state, WindowChromeState::Tiled);
                    assert!(model.tiling.is_some());
                    TestFrame { title_height: 53.0 }
                });
            let display = Display::new().unwrap();
            let wayland = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
            let surface = WaylandSurfaceId::from_raw(20).unwrap();
            let rect = RectI {
                x: 0,
                y: 40,
                width: 600,
                height: 380,
            };
            let mut window = test_window(
                SizeI {
                    width: 600,
                    height: 350,
                },
                PointI { x: 0, y: 40 },
            );
            window.tile = Some(super::super::tiling::TilePlacement {
                target: crate::TileTarget::TopLeft,
                rect,
                shared_resize: true,
            });
            window.backend = Some(backend);
            if interactive && backend == WindowBackend::Wayland {
                window.native_configure.resize_anchor = Some(ResizeAnchor::new(
                    window.position,
                    window.requested_size,
                    ResizeEdge::BottomRight,
                ));
            }
            #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
            if interactive && matches!(backend, WindowBackend::X11(_)) {
                window.resize_preview.begin(
                    window.position,
                    window.requested_size,
                    ResizeEdge::BottomRight,
                );
            }
            let mut windows = BTreeMap::from([(surface, window)]);
            let mut frames = BTreeMap::new();
            refresh_window_frames(
                declaration.frame_template(),
                &mut frames,
                &mut windows,
                &wayland,
                &LinuxShellConfig::default(),
                AssetBundle::default(),
                &crate::AppIconProfile::default(),
                None,
                &EventNotifier::new("tiled frame").unwrap(),
                0,
                crate::platform::contracts::ScaleFactor::new(1.5).unwrap(),
                RectI {
                    x: 0,
                    y: 40,
                    width: 1200,
                    height: 760,
                },
                &mut ConfigureScheduler::default(),
            )
            .unwrap();
            assert_eq!(
                windows[&surface].chrome_outer,
                Some(SizeI {
                    width: 600,
                    height: 380
                })
            );
            assert_eq!(
                windows[&surface].requested_size,
                SizeI {
                    width: 600,
                    height: 327
                }
            );
            assert_eq!(
                windows[&surface].native_configure.resize_final.is_some(),
                !interactive && backend == WindowBackend::Wayland
            );
            assert_eq!(windows[&surface].resizing(), interactive);
            assert!(windows[&surface].resize_veil_active());
        }
    }
}

#[test]
fn maximize_uses_work_area_instead_of_legacy_client_size_and_restore_keeps_client_size() {
    for title_height in [24.0, 32.0, 48.0] {
        for scale in [1.0, 1.5, 3.0] {
            let original = SizeI {
                width: 600,
                height: 400,
            };
            let mut outer = original;
            let mut layer = Layer::new(
                CompositionDriver::for_target(
                    TestFrame { title_height },
                    RuntimeTarget::Compositor,
                ),
                outer,
                AssetBundle::default(),
                crate::platform::contracts::ScaleFactor::new(scale).unwrap(),
            )
            .unwrap();
            let area = RectI {
                x: 0,
                y: 42,
                width: 1280,
                height: 758,
            };
            let legacy = SizeI {
                width: 1272,
                height: 718,
            };
            let snapshot =
                layout_window_frame(&mut layer, &mut outer, legacy, Some(area), 0, true)
                    .unwrap();
            assert_eq!(
                outer,
                SizeI {
                    width: 1280,
                    height: 758
                }
            );
            assert_eq!(snapshot.content.bounds.width, 1280.0);
            assert_eq!(snapshot.content.bounds.height, 758.0 - title_height);
            assert_eq!(snapshot.content.bounds.bottom(), 758.0);
            let restored =
                layout_window_frame(&mut layer, &mut outer, original, None, 1, true).unwrap();
            assert_eq!(restored.content.bounds.width, original.width as f32);
            assert_eq!(restored.content.bounds.height, original.height as f32);
        }
    }
}

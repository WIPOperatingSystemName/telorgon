use telorgon::app::*;
use telorgon::application_host::AppRuntimeCore;
use telorgon::{AssetKey, MonotonicInstant};

const fn icon(path: &'static str) -> IconAsset {
    IconAsset::new(AssetKey::new(path))
}

const CONTROL_RESTING: WindowControlVisual = WindowControlVisual {
    decoration: BoxDecoration::new()
        .background(Background::Color(ColorRgba8::rgba(35, 40, 52, 255)))
        .corner_radius(7.0),
    icon_tint: ColorRgba8::rgba(225, 230, 242, 255),
};

const CONTROL_HOVERED: WindowControlVisual = WindowControlVisual {
    decoration: BoxDecoration::new()
        .background(Background::Color(ColorRgba8::rgba(60, 70, 96, 255)))
        .corner_radius(7.0),
    icon_tint: ColorRgba8::rgba(255, 255, 255, 255),
};

const CONTROL_PRESSED: WindowControlVisual = WindowControlVisual {
    decoration: BoxDecoration::new()
        .background(Background::Color(ColorRgba8::rgba(82, 96, 132, 255)))
        .corner_radius(7.0),
    icon_tint: ColorRgba8::rgba(255, 255, 255, 255),
};

const CONTROL_FOCUSED: WindowControlVisual = WindowControlVisual {
    decoration: BoxDecoration::new()
        .uniform_border(2.0, ColorRgba8::rgba(130, 155, 255, 255))
        .corner_radius(7.0),
    icon_tint: ColorRgba8::rgba(255, 255, 255, 255),
};

const CONTROL_DISABLED: WindowControlVisual = WindowControlVisual {
    decoration: BoxDecoration::new().corner_radius(7.0),
    icon_tint: ColorRgba8::rgba(120, 126, 142, 255),
};

const STANDARD_BUTTON: WindowControlButtonStyle = WindowControlButtonStyle {
    width: Dimension::Logical(38.0),
    height: Dimension::Logical(30.0),
    icon_size: 15.0,
    resting: CONTROL_RESTING,
    hovered: Some(CONTROL_HOVERED),
    pressed: Some(CONTROL_PRESSED),
    focused: Some(CONTROL_FOCUSED),
    disabled: Some(CONTROL_DISABLED),
    transition: Some(telorgon::TransitionSpec {
        duration_ms: 90,
        easing: telorgon::Easing::EaseOut,
        repeat: false,
    }),
};

const CLOSE_BUTTON: WindowControlButtonStyle = WindowControlButtonStyle {
    resting: WindowControlVisual {
        decoration: BoxDecoration::new()
            .background(Background::Color(ColorRgba8::rgba(77, 35, 45, 255)))
            .corner_radius(7.0),
        icon_tint: ColorRgba8::rgba(255, 220, 225, 255),
    },
    hovered: Some(WindowControlVisual {
        decoration: BoxDecoration::new()
            .background(Background::Color(ColorRgba8::rgba(196, 52, 72, 255)))
            .corner_radius(7.0),
        icon_tint: ColorRgba8::rgba(255, 255, 255, 255),
    }),
    ..STANDARD_BUTTON
};

const NORMAL: WindowChromeStateStyle = WindowChromeStateStyle {
    title_bar_visible: true,
    frame_radius: 14.0,
    shadow: Some(Shadow {
        offset: PointF { x: 0.0, y: 12.0 },
        blur: 30.0,
        spread: 0.0,
        color: ColorRgba8::rgba(0, 0, 0, 128),
    }),
    resize_regions: true,
    resize_edge: 6.0,
    resize_hit_slop: Insets::all(3.0),
};

const TEST_CHROME: WindowChromeDesign = WindowChromeDesign {
    motion: telorgon::WindowMotion::none(),
    active: WindowChromePalette {
        frame_background: ColorRgba8::rgba(23, 27, 38, 255),
        frame_border: ColorRgba8::rgba(101, 119, 184, 255),
        frame_border_width: 1.0,
        title_color: ColorRgba8::rgba(245, 247, 255, 255),
        title_weight: 650,
    },
    inactive: WindowChromePalette {
        frame_background: ColorRgba8::rgba(31, 34, 43, 255),
        frame_border: ColorRgba8::rgba(65, 70, 85, 255),
        frame_border_width: 1.0,
        title_color: ColorRgba8::rgba(174, 179, 193, 255),
        title_weight: 450,
    },
    normal: NORMAL,
    maximized: WindowChromeStateStyle {
        frame_radius: 0.0,
        shadow: None,
        resize_regions: false,
        resize_edge: 0.0,
        resize_hit_slop: Insets::ZERO,
        ..NORMAL
    },
    tiled: WindowChromeStateStyle {
        frame_radius: 0.0,
        shadow: None,
        ..NORMAL
    },
    fullscreen: WindowChromeStateStyle {
        title_bar_visible: false,
        frame_radius: 0.0,
        shadow: None,
        resize_regions: false,
        resize_edge: 0.0,
        resize_hit_slop: Insets::ZERO,
    },
    title_bar: WindowTitleBarStyle {
        height: 44.0,
        padding: Insets::symmetric(7.0, 8.0),
        gap: 8.0,
        title_size: 14.0,
        app_icon_region_size: 30.0,
        app_icon_size: 20.0,
        show_client_icon: true,
        fallback_app_icon: Some(icon("icons/app.svg")),
        app_icon_opens_system_menu: true,
    },
    controls: WindowControlsDesign {
        minimize: WindowControlDesign {
            icon: icon("icons/minimize.svg"),
            style: STANDARD_BUTTON,
        },
        maximize: WindowControlDesign {
            icon: icon("icons/maximize.svg"),
            style: STANDARD_BUTTON,
        },
        restore: WindowControlDesign {
            icon: icon("icons/restore.svg"),
            style: STANDARD_BUTTON,
        },
        close: WindowControlDesign {
            icon: icon("icons/close.svg"),
            style: CLOSE_BUTTON,
        },
        gap: 6.0,
    },
    content_background: ColorRgba8::rgba(15, 18, 26, 255),
    resize_preview: None,
};

const PIN_WINDOW: ShellActionId = ShellActionId::named("window.pin");

#[component]
struct DesktopBackground {}

impl Component for DesktopBackground {
    fn view(&self) -> impl View {
        stack().background(ColorRgba8::rgba(10, 12, 18, 255))
    }
}

#[component]
struct Panel {}

impl Component for Panel {
    fn view(&self) -> impl View {
        row().child(text("Telorgon"))
    }
}

fn pin_window(_window: WindowChromeModel) {}

#[test]
fn easy_frame_is_a_closure_free_complete_desktop_declaration() {
    TEST_CHROME.validate().unwrap();

    let desktop = Application::desktop_environment("Telorgon")
        .compositor(
            Compositor::new()
                .cursor_theme(telorgon::CursorTheme::new())
                .window_frame(easy_window_frame(TEST_CHROME))
                .shell_action(PIN_WINDOW, pin_window)
                .background(DesktopBackground::default()),
        )
        .shell_widget(
            ShellWidget::new("Panel")
                .anchor(ShellWidgetAnchor::Top)
                .reserve_space(40.0)
                .content(Panel::default()),
        );

    assert!(format!("{desktop:?}").contains("has_compositor: true"));
}

#[test]
fn easy_frame_publishes_controls_and_all_resize_directions() {
    let component =
        easy_window_frame(TEST_CHROME).compose(WindowChromeModel::new(7, "Editor").active(true));
    let mut runtime = AppRuntimeCore::from_composed_with_extent(
        component,
        SizeI {
            width: 640,
            height: 480,
        },
    )
    .unwrap();
    runtime.prepare_frame(MonotonicInstant::ZERO, true).unwrap();
    let snapshot = telorgon::WindowChromeSnapshot::derive(runtime.ui(), runtime.layout()).unwrap();

    let root = &runtime
        .ui()
        .box_styles
        .get(snapshot.frame.node)
        .unwrap()
        .decoration;
    let slot = &runtime
        .ui()
        .box_styles
        .get(snapshot.content.node)
        .unwrap()
        .decoration;
    assert_eq!(root.corner_radii, CornerRadii::all(NORMAL.frame_radius));
    assert_eq!(
        slot.corner_radii.top_left, 0.0,
        "title-bar seam is not a window corner"
    );
    assert_eq!(slot.corner_radii.top_right, 0.0);
    assert_eq!(
        slot.corner_radii.bottom_left,
        NORMAL.frame_radius - TEST_CHROME.active.frame_border_width
    );
    assert_eq!(
        slot.corner_radii.bottom_right,
        NORMAL.frame_radius - TEST_CHROME.active.frame_border_width
    );

    assert!(
        snapshot
            .regions
            .iter()
            .any(|region| { region.role == WindowChromeRole::Action(WindowAction::Close) })
    );
    assert_eq!(
        snapshot
            .regions
            .iter()
            .filter(|region| matches!(
                region.role,
                WindowChromeRole::Action(WindowAction::BeginResize(_))
            ))
            .count(),
        8
    );
    assert_eq!(snapshot.hit_test(320.0, 240.0), None);
    assert_eq!(
        snapshot.hit_test(320.0, 24.0),
        Some(WindowChromeRole::DragRegion)
    );
    assert_eq!(
        snapshot.hit_test(636.0, 4.0),
        Some(WindowChromeRole::Action(WindowAction::BeginResize(
            WindowResizeEdge::TopRight,
        )))
    );
    assert_eq!(snapshot.hit_test(630.0, 470.0), None);
    for region in snapshot.regions.iter().filter(|region| {
        matches!(
            region.role,
            WindowChromeRole::Action(WindowAction::BeginResize(_))
        )
    }) {
        match region.role {
            WindowChromeRole::Action(WindowAction::BeginResize(
                WindowResizeEdge::Top | WindowResizeEdge::Bottom,
            )) => assert_eq!(region.bounds.height, 0.0),
            WindowChromeRole::Action(WindowAction::BeginResize(
                WindowResizeEdge::Left | WindowResizeEdge::Right,
            )) => assert_eq!(region.bounds.width, 0.0),
            WindowChromeRole::Action(WindowAction::BeginResize(_)) => {
                assert!((region.bounds.width, region.bounds.height) == (14.0, 14.0));
            }
            _ => unreachable!("filtered to resize regions"),
        }
    }
    let close = snapshot
        .regions
        .iter()
        .find(|region| region.role == WindowChromeRole::Action(WindowAction::Close))
        .expect("easy frame must publish its close control");
    let close_right_inset = snapshot.frame.bounds.x + snapshot.frame.bounds.width
        - (close.bounds.x + close.bounds.width);
    assert_eq!(close_right_inset, 9.0);

    let tiled = easy_window_frame(TEST_CHROME).compose(WindowChromeModel::new(8, "Tiled").tiling(
        WindowTilingState::new(
            WindowEdgeMask::TOP | WindowEdgeMask::RIGHT,
            WindowEdgeMask::TOP | WindowEdgeMask::RIGHT,
        ),
    ));
    let mut runtime = AppRuntimeCore::from_composed_with_extent(
        tiled,
        SizeI {
            width: 640,
            height: 480,
        },
    )
    .unwrap();
    runtime.prepare_frame(MonotonicInstant::ZERO, true).unwrap();
    let snapshot = telorgon::WindowChromeSnapshot::derive(runtime.ui(), runtime.layout()).unwrap();
    assert_eq!(
        snapshot
            .regions
            .iter()
            .filter(|region| matches!(
                region.role,
                WindowChromeRole::Action(WindowAction::BeginResize(_))
            ))
            .count(),
        3
    );
}

fn chrome_snapshot(
    design: WindowChromeDesign,
    model: WindowChromeModel,
) -> telorgon::WindowChromeSnapshot {
    let mut runtime = AppRuntimeCore::from_composed_with_extent(
        easy_window_frame(design).compose(model),
        SizeI {
            width: 640,
            height: 480,
        },
    )
    .unwrap();
    runtime.prepare_frame(MonotonicInstant::ZERO, true).unwrap();
    telorgon::WindowChromeSnapshot::derive(runtime.ui(), runtime.layout()).unwrap()
}

#[test]
fn easy_frame_resize_hitboxes_track_the_border_without_spilling_inward() {
    for (border, grab) in [(0.0_f32, 6.0_f32), (1.0, 6.0), (4.0, 2.0), (18.0, 0.0)] {
        let state = WindowChromeStateStyle {
            resize_edge: grab,
            ..NORMAL
        };
        let palette = WindowChromePalette {
            frame_border_width: border,
            ..TEST_CHROME.active
        };
        let design = WindowChromeDesign {
            active: palette,
            inactive: palette,
            normal: state,
            ..TEST_CHROME
        };
        let snapshot = chrome_snapshot(design, WindowChromeModel::new(10, "Border"));
        let frame = snapshot.frame.bounds;
        let content = snapshot.content.bounds;
        assert_eq!(content.x, border);
        assert_eq!(content.y, border + design.title_bar.height);
        assert_eq!(content.right(), frame.right() - border);
        assert_eq!(content.bottom(), frame.bottom() - border);
        let outside = (grab - border).max(0.0) + 3.0; // NORMAL's outward hit slop.
        let resize = |edge| Some(WindowChromeRole::Action(WindowAction::BeginResize(edge)));
        assert_eq!(
            snapshot.hit_test(-outside + 0.25, 240.0),
            resize(WindowResizeEdge::Left)
        );
        assert_eq!(snapshot.hit_test(-outside - 0.25, 240.0), None);
        assert_eq!(
            snapshot.hit_test(640.0 + outside - 0.25, 240.0),
            resize(WindowResizeEdge::Right)
        );
        assert_eq!(
            snapshot.hit_test(320.0, -outside + 0.25),
            resize(WindowResizeEdge::Top)
        );
        assert_eq!(
            snapshot.hit_test(320.0, 480.0 + outside - 0.25),
            resize(WindowResizeEdge::Bottom)
        );
        assert_eq!(
            snapshot.hit_test(border + 0.5, 240.0),
            None,
            "client edge is not a resize target"
        );
        assert_eq!(snapshot.hit_test(640.0 - border - 0.5, 240.0), None);
        assert_eq!(snapshot.hit_test(320.0, 480.0 - border - 0.5), None);
        assert_eq!(
            snapshot.hit_test(320.0, border + 0.5),
            Some(WindowChromeRole::DragRegion)
        );
        if border > 0.0 {
            assert_eq!(
                snapshot.hit_test(border * 0.5, 240.0),
                resize(WindowResizeEdge::Left)
            );
            let r = state.frame_radius;
            let offset = (r - border * 0.5).max(0.0) / 2.0_f32.sqrt();
            for (edge, x, y) in [
                (WindowResizeEdge::TopLeft, r - offset, r - offset),
                (WindowResizeEdge::TopRight, 640.0 - r + offset, r - offset),
                (WindowResizeEdge::BottomLeft, r - offset, 480.0 - r + offset),
                (
                    WindowResizeEdge::BottomRight,
                    640.0 - r + offset,
                    480.0 - r + offset,
                ),
            ] {
                assert_eq!(
                    snapshot.hit_test(x, y),
                    resize(edge),
                    "curved border itself must be grabbable"
                );
            }
        }
    }
}

#[test]
fn title_bar_height_and_visibility_derive_content_bounds_automatically() {
    for visible in [false, true] {
        for height in [44.0, 72.0] {
            let design = WindowChromeDesign {
                normal: WindowChromeStateStyle {
                    title_bar_visible: visible,
                    ..NORMAL
                },
                title_bar: WindowTitleBarStyle {
                    height,
                    ..TEST_CHROME.title_bar
                },
                ..TEST_CHROME
            };
            let snapshot =
                chrome_snapshot(design, WindowChromeModel::new(11, "Layout").active(true));
            let border = design.active.frame_border_width;
            assert_eq!(snapshot.content.bounds.x, border);
            assert_eq!(
                snapshot.content.bounds.y,
                border + if visible { height } else { 0.0 }
            );
            assert_eq!(snapshot.content.bounds.width, 640.0 - border * 2.0);
            assert_eq!(
                snapshot.content.bounds.height,
                480.0 - border * 2.0 - if visible { height } else { 0.0 }
            );
        }
    }
}

#[test]
fn corner_resize_to_content_hover_changes_pointer_ownership() {
    for title_bar_visible in [false, true] {
        let design = WindowChromeDesign {
            normal: WindowChromeStateStyle {
                title_bar_visible,
                ..NORMAL
            },
            ..TEST_CHROME
        };
        let snapshot = chrome_snapshot(design, WindowChromeModel::new(17, "Hover").active(true));
        let radius = NORMAL.frame_radius;
        let border = design.active.frame_border_width;
        let start = radius - (radius - border * 0.5) / 2.0_f32.sqrt();
        for (edge, right, bottom) in [
            (WindowResizeEdge::TopLeft, false, false),
            (WindowResizeEdge::TopRight, true, false),
            (WindowResizeEdge::BottomLeft, false, true),
            (WindowResizeEdge::BottomRight, true, true),
        ] {
            // With a title bar, the upper corners lead into chrome, not client content.
            if title_bar_visible && !bottom {
                continue;
            }
            let point = |inset: f32| PointF {
                x: if right { 640.0 - inset } else { inset },
                y: if bottom { 480.0 - inset } else { inset },
            };
            let corner = point(start);
            assert!(
                snapshot.content.bounds.contains(corner),
                "the old rectangular focus test would enter here"
            );
            assert_eq!(
                snapshot.hit_test(corner.x, corner.y),
                Some(WindowChromeRole::Action(WindowAction::BeginResize(edge)))
            );
            assert!(!snapshot.hit_test_content(corner.x, corner.y));

            let mut entered = false;
            let mut transitions = 0;
            for step in 0..=80 {
                let p = point(start + (radius - start) * step as f32 / 80.0);
                let content = snapshot.hit_test_content(p.x, p.y);
                if content != entered {
                    transitions += 1;
                }
                if content {
                    assert_eq!(snapshot.hit_test(p.x, p.y), None);
                }
                entered = content;
            }
            assert!(entered);
            assert_eq!(
                transitions, 1,
                "slow corner-to-content motion must hand focus back exactly once"
            );
        }
        for (outside, inside) in [
            (
                PointF {
                    x: border * 0.5,
                    y: 240.0,
                },
                PointF {
                    x: border + 0.5,
                    y: 240.0,
                },
            ),
            (
                PointF {
                    x: 640.0 - border * 0.5,
                    y: 240.0,
                },
                PointF {
                    x: 640.0 - border - 0.5,
                    y: 240.0,
                },
            ),
            (
                PointF {
                    x: 320.0,
                    y: 480.0 - border * 0.5,
                },
                PointF {
                    x: 320.0,
                    y: 480.0 - border - 0.5,
                },
            ),
        ] {
            assert!(!snapshot.hit_test_content(outside.x, outside.y));
            assert!(snapshot.hit_test_content(inside.x, inside.y));
        }
    }
}

#[test]
fn resize_contours_do_not_capture_client_pixels_or_enable_disabled_edges() {
    let snapshot = chrome_snapshot(
        TEST_CHROME,
        WindowChromeModel::new(12, "Round").active(true),
    );
    let border = TEST_CHROME.active.frame_border_width;
    let outer = telorgon::render::RoundedClip::new(
        snapshot.frame.bounds,
        CornerRadii::all(NORMAL.frame_radius),
    );
    let inner = outer.inset(telorgon::render::Border::all(
        border,
        ColorRgba8::rgba(0, 0, 0, 0),
    ));
    for y in (0..480).step_by(3) {
        for x in (0..640).step_by(3) {
            let point = PointF {
                x: x as f32 + 0.5,
                y: y as f32 + 0.5,
            };
            if inner.contains(point) {
                assert!(
                    !matches!(
                        snapshot.hit_test(point.x, point.y),
                        Some(WindowChromeRole::Action(WindowAction::BeginResize(_)))
                    ),
                    "interior stolen at {point:?}"
                );
            }
        }
    }
    for model in [
        WindowChromeModel::new(13, "Disabled").state(WindowChromeState::Maximized),
        WindowChromeModel::new(14, "Disabled").state(WindowChromeState::Fullscreen),
        WindowChromeModel::new(15, "Disabled").capabilities(WindowChromeCapabilities {
            resize: false,
            ..WindowChromeCapabilities::MANAGED_TOPLEVEL
        }),
    ] {
        let snapshot = chrome_snapshot(TEST_CHROME, model);
        assert!(!snapshot.regions.iter().any(|r| matches!(
            r.role,
            WindowChromeRole::Action(WindowAction::BeginResize(_))
        )));
    }
    let tiled = chrome_snapshot(
        TEST_CHROME,
        WindowChromeModel::new(16, "Tiled").tiling(WindowTilingState::new(
            WindowEdgeMask::TOP,
            WindowEdgeMask::TOP,
        )),
    );
    assert_eq!(tiled.hit_test(-1.0, 240.0), None);
    assert_eq!(
        tiled.hit_test(320.0, -1.0),
        Some(WindowChromeRole::Action(WindowAction::BeginResize(
            WindowResizeEdge::Top
        )))
    );
}

#[component(no_default)]
struct AdvancedFrame {
    #[input]
    model: WindowChromeModel,
}

impl Component for AdvancedFrame {
    fn view(&self) -> impl View {
        window_frame()
            .gap(4.0)
            .child(
                row()
                    .height(42.0)
                    .child(text(&self.model.title).window_title())
                    .child(spacer())
                    .child(
                        button("Pin")
                            .window_shell_action(PIN_WINDOW)
                            .pointer_icon(telorgon::PointerIcon::Pointer),
                    )
                    .window_drag_region(),
            )
            .child(
                stack()
                    .height(5.0)
                    .window_resize(WindowResizeEdge::Top)
                    .window_hit_slop(Insets::all(4.0))
                    .window_hit_priority(500),
            )
            .content_slot(window_content_slot().margin((42.0, 5.0, 5.0, 5.0)))
    }
}

struct AdvancedFrameTemplate;

impl WindowFrameTemplate for AdvancedFrameTemplate {
    type Component = AdvancedFrame;

    fn compose(&self, model: WindowChromeModel) -> Self::Component {
        AdvancedFrame { model }
    }
}

#[test]
fn low_level_template_retains_full_composition_and_authorized_action_freedom() {
    let compositor = Compositor::new()
        .cursor_theme(telorgon::CursorTheme::new())
        .window_frame(AdvancedFrameTemplate)
        .shell_action(PIN_WINDOW, pin_window)
        .background(DesktopBackground::default());

    assert!(compositor.window_frame().is_some());
    assert!(compositor.authorizes_shell_action(PIN_WINDOW));

    let component = AdvancedFrameTemplate.compose(WindowChromeModel::new(9, "Advanced"));
    let mut runtime = AppRuntimeCore::from_composed_with_extent(
        component,
        SizeI {
            width: 500,
            height: 320,
        },
    )
    .unwrap();
    runtime.prepare_frame(MonotonicInstant::ZERO, true).unwrap();
    let snapshot = telorgon::WindowChromeSnapshot::derive(runtime.ui(), runtime.layout()).unwrap();
    assert!(
        snapshot
            .regions
            .iter()
            .any(|region| region.role == WindowChromeRole::ShellAction(PIN_WINDOW))
    );
    let resize = snapshot
        .regions
        .iter()
        .find(|region| {
            region.role
                == WindowChromeRole::Action(WindowAction::BeginResize(WindowResizeEdge::Top))
        })
        .unwrap();
    assert_eq!(resize.priority, 500);
    assert!(resize.hit_bounds.height > resize.bounds.height);
}

#[test]
fn control_dimensions_follow_the_padded_bar_and_preserve_hit_targets() {
    for bar_height in [24.0, 31.0, 31.5, 40.0, 60.0] {
        for padding in [0.0, 3.0] {
            for height in [
                Dimension::FILL,
                Dimension::Percent(0.5),
                Dimension::Logical(16.0),
                Dimension::Shrink,
            ] {
                for maximized in [false, true] {
                    let mut design = TEST_CHROME;
                    design.title_bar.height = bar_height;
                    design.title_bar.padding = Insets::symmetric(padding, 8.0);
                    design.title_bar.show_client_icon = false;
                    for control in [
                        &mut design.controls.minimize,
                        &mut design.controls.maximize,
                        &mut design.controls.restore,
                        &mut design.controls.close,
                    ] {
                        control.style.height = height;
                    }
                    let mut model = WindowChromeModel::new(42, "Sizing").active(true);
                    if maximized {
                        model.state = WindowChromeState::Maximized;
                    }
                    let snapshot = chrome_snapshot(design, model);
                    let available = bar_height - 2.0 * padding;
                    for action in [
                        WindowAction::Minimize,
                        WindowAction::ToggleMaximize,
                        WindowAction::Close,
                    ] {
                        let role = WindowChromeRole::Action(action);
                        let bounds = snapshot
                            .regions
                            .iter()
                            .find(|r| r.role == role)
                            .unwrap()
                            .bounds;
                        let expected = match height {
                            Dimension::Fill(_) => available,
                            Dimension::Percent(fraction) => available * fraction,
                            Dimension::Logical(value) => value,
                            Dimension::Shrink => {
                                assert!(bounds.height > 0.0 && bounds.height <= available);
                                bounds.height
                            }
                        };
                        assert_eq!(
                            bounds.height, expected,
                            "{height:?}, bar={bar_height}, padding={padding}"
                        );
                        assert_eq!(
                            bounds.y,
                            if maximized { 0.0 } else { 1.0 }
                                + padding
                                + (available - expected) / 2.0
                        );
                        assert_eq!(bounds.width, 38.0);
                        assert_eq!(
                            snapshot.hit_test(bounds.x + bounds.width / 2.0, bounds.y + 0.5),
                            Some(role)
                        );
                        assert_eq!(
                            snapshot.hit_test(bounds.x + bounds.width / 2.0, bounds.bottom() - 0.5),
                            Some(role)
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn control_widths_support_shrink_percent_and_weighted_fill() {
    for width in [Dimension::Shrink, Dimension::Percent(0.2), Dimension::FILL] {
        let mut design = TEST_CHROME;
        design.controls.minimize.style.width = width;
        design.controls.maximize.style.width = width;
        design.controls.close.style.width = if width == Dimension::FILL {
            Dimension::Fill(2.0)
        } else {
            width
        };
        let snapshot = chrome_snapshot(design, WindowChromeModel::new(43, "Widths").active(true));
        let bounds = |action| {
            snapshot
                .regions
                .iter()
                .find(|r| r.role == WindowChromeRole::Action(action))
                .unwrap()
                .bounds
        };
        let first = bounds(WindowAction::Minimize);
        let second = bounds(WindowAction::ToggleMaximize);
        let last = bounds(WindowAction::Close);
        assert!(first.width > 0.0);
        // Bounds arithmetic can differ by a few f32 rounding steps.
        assert!((first.width - second.width).abs() < 0.001);
        assert!((second.x - (first.right() + design.controls.gap)).abs() < 0.001);
        assert!((last.x - (second.right() + design.controls.gap)).abs() < 0.001);
        if width == Dimension::FILL {
            assert!((last.width - first.width * 2.0).abs() < 0.001);
        }
        assert!(last.right() <= 631.0);
    }
}

#[test]
fn control_dimension_validation_rejects_invalid_values() {
    for dimension in [
        Dimension::Logical(-1.0),
        Dimension::Logical(0.0),
        Dimension::Logical(f32::NAN),
        Dimension::Fill(0.0),
        Dimension::Fill(f32::INFINITY),
        Dimension::Percent(-0.1),
        Dimension::Percent(1.1),
    ] {
        for height in [false, true] {
            let mut design = TEST_CHROME;
            if height {
                design.controls.restore.style.height = dimension;
            } else {
                design.controls.restore.style.width = dimension;
            }
            assert_eq!(
                design.validate(),
                Err(WindowChromeDesignError::InvalidControlMetric)
            );
        }
    }
    for dimension in [
        Dimension::Shrink,
        Dimension::FILL,
        Dimension::Percent(0.5),
        Dimension::Logical(24.0),
    ] {
        let mut design = TEST_CHROME;
        design.controls.close.style.height = dimension;
        design.controls.close.style.width = dimension;
        assert!(design.validate().is_ok());
    }
}

#[test]
fn maximized_and_fullscreen_frames_fill_the_outer_bounds_without_a_border() {
    for border in [1.0, 2.0, 6.0] {
        for title_height in [24.0, 32.0, 48.0] {
            let mut design = TEST_CHROME;
            design.active.frame_border_width = border;
            design.title_bar.height = title_height;
            for state in [WindowChromeState::Maximized, WindowChromeState::Fullscreen] {
                let snapshot = chrome_snapshot(
                    design,
                    WindowChromeModel::new(99, "Maximized")
                        .active(true)
                        .state(state),
                );
                let content = snapshot.content.bounds;
                let frame = snapshot.frame.bounds;
                assert_eq!(content.x, frame.x);
                assert_eq!(content.right(), frame.right());
                assert_eq!(content.bottom(), frame.bottom());
                let top = if state == WindowChromeState::Fullscreen {
                    0.0
                } else {
                    title_height
                };
                assert_eq!(content.y, frame.y + top);
                assert_eq!(content.height, frame.height - top);
            }
            let normal = chrome_snapshot(design, WindowChromeModel::new(99, "Normal").active(true));
            assert_eq!(normal.content.bounds.x, border);
        }
    }
}

#[test]
fn easy_frame_exports_the_same_const_motion_style_as_custom_templates() {
    const MOTION: WindowMotion = WindowMotion::smooth()
        .maximize(tween_ms(210, telorgon::Easing::EaseOut))
        .restore(tween_ms(170, telorgon::Easing::Linear))
        .minimize(Minimize::shrink_and_fade(150))
        .resize_content(ContentFade::new(60, 110));
    let design = WindowChromeDesign {
        motion: MOTION,
        ..TEST_CHROME
    };
    let template = easy_window_frame(design);
    let model = WindowChromeModel::new(1, "Motion");
    assert_eq!(template.motion(&model), Some(MOTION));
    assert_eq!(MOTION.maximize_transition(false).duration_ms(), 170);
    assert_eq!(MOTION.minimize_transition(false).duration_ms, 150);
    assert_eq!(
        easy_window_frame(TEST_CHROME).motion(&model),
        Some(WindowMotion::none())
    );
}

#[test]
fn fluid_motion_is_const_customizable_through_the_public_app_api() {
    use telorgon::app::{GeometryMotion, Spring};
    const MOTION: WindowMotion = WindowMotion::fluid()
        .maximize_spring(
            Spring::new()
                .initial_velocity(3.2)
                .damping_ratio(0.87)
                .angular_frequency(20.5)
                .settle_within_ms(450),
        )
        .restore_spring(
            Spring::new()
                .initial_velocity(2.8)
                .damping_ratio(0.86)
                .angular_frequency(19.5)
                .settle_within_ms(475),
        )
        .maximize_content(ContentFade::new(50, 130));
    let template = easy_window_frame(WindowChromeDesign {
        motion: MOTION,
        ..TEST_CHROME
    });
    assert_eq!(
        template.motion(&WindowChromeModel::new(1, "Fluid")),
        Some(MOTION)
    );
    assert!(matches!(
        MOTION.maximize_transition(true),
        GeometryMotion::Spring(_)
    ));
    assert_eq!(MOTION.maximize_transition(false).duration_ms(), 475);
}

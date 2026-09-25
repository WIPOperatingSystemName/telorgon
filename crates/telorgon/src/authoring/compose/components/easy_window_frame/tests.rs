use super::*;
use crate::assets::AssetKey;
use crate::theme::Easing;

#[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
#[test]
fn shell_catalog_icon_is_centered_and_focus_changes_shadow_color() {
    use crate::host::application::AppRuntimeCore;
    use crate::foundation::{MonotonicInstant, SizeI};
    use crate::runtime::CompositionDriver;
    use crate::shell::window_chrome::{WindowChromeRole, WindowChromeSnapshot};
    let host = crate::authoring::compose::shell_services::ShellServiceHost::new();
    let mut model = WindowChromeModel::new(42, "Editor");
    model.desktop_window_id = Some(crate::shell::WindowId::new(
        std::num::NonZeroU32::new(42).unwrap(),
        std::num::NonZeroU32::new(1).unwrap(),
    ));
    let mut design = DESIGN;
    design.normal.shadow = Some(Shadow {
        offset: crate::PointF { x: 0.0, y: 4.0 },
        blur: 12.0,
        spread: 2.0,
        color: ColorRgba8::rgba(0, 0, 0, 130),
    });
    design.active.shadow_color = Some(ColorRgba8::rgba(0, 0, 0, 210));
    design.inactive.shadow_color = Some(ColorRgba8::rgba(0, 0, 0, 90));
    let mut driver = CompositionDriver::new(easy_window_frame(design).compose(model.clone()));
    driver.connect_shell(host.services.clone());
    let mut runtime = AppRuntimeCore::from_composition_driver(
        driver,
        SizeI {
            width: 640,
            height: 480,
        },
    )
    .unwrap();
    for (index, (active, state)) in [
        (false, WindowChromeState::Normal),
        (true, WindowChromeState::Normal),
        (true, WindowChromeState::Maximized),
    ]
    .into_iter()
    .enumerate()
    {
        model.active = active;
        model.state = state;
        runtime
            .update_composition_root(Box::new(easy_window_frame(design).compose(model.clone())))
            .unwrap();
        runtime
            .prepare_frame(MonotonicInstant::from_nanos(index as u64), true)
            .unwrap();
        let snapshot = WindowChromeSnapshot::derive(runtime.ui(), runtime.layout()).unwrap();
        let icon = snapshot
            .regions
            .iter()
            .find(|r| r.role == WindowChromeRole::AppIcon)
            .expect("catalog fallback icon is visible");
        let border = if state == WindowChromeState::Maximized {
            0.0
        } else {
            design.inactive.frame_border.top.width
        };
        assert!(
            (icon.bounds.y + icon.bounds.height / 2.0
                - (border + design.title_bar.height / 2.0))
                .abs()
                < 0.01
        );
        let decoration = runtime
            .ui()
            .box_styles
            .get(snapshot.frame.node)
            .unwrap()
            .decoration;
        let expected = if state == WindowChromeState::Maximized {
            crate::ui::ShadowList::default()
        } else {
            let mut shadow = design.normal.shadow.unwrap();
            shadow.color = design.palette(active).shadow_color.unwrap();
            crate::ui::ShadowList::one(shadow)
        };
        assert_eq!(decoration.shadows, expected);
    }
}

#[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
#[test]
fn server_content_slot_is_square_with_or_without_titlebar() {
    use crate::foundation::{MonotonicInstant, SizeI};
    use crate::host::application::AppRuntimeCore;
    use crate::runtime::CompositionDriver;
    use crate::shell::window_chrome::WindowChromeSnapshot;

    for title_bar_visible in [true, false] {
        let mut model = WindowChromeModel::new(42, "Browser");
        model.title_bar_visible = title_bar_visible;
        let driver = CompositionDriver::new(easy_window_frame(DESIGN).compose(model));
        let mut runtime = AppRuntimeCore::from_composition_driver(
            driver, SizeI { width: 640, height: 480 },
        ).unwrap();
        runtime.prepare_frame(MonotonicInstant::from_nanos(0), true).unwrap();
        let snapshot = WindowChromeSnapshot::derive(runtime.ui(), runtime.layout()).unwrap();
        let content = runtime.ui().box_styles.get(snapshot.content.node).unwrap();
        let frame = runtime.ui().box_styles.get(snapshot.frame.node).unwrap();
        assert_eq!(content.decoration.corner_radii, crate::ui::CornerRadii::default());
        assert_eq!(frame.decoration.corner_radii, crate::ui::CornerRadii::all(STATE.frame_radius));
    }
}

const fn icon(path: &'static str) -> IconAsset {
    IconAsset::new(AssetKey::new(path))
}

const VISUAL: WindowControlVisual = WindowControlVisual {
    decoration: BoxDecoration::new(),
    icon_tint: ColorRgba8::rgba(255, 255, 255, 255),
};
const BUTTON: WindowControlButtonStyle = WindowControlButtonStyle {
    width: Dimension::Logical(38.0),
    height: Dimension::Logical(30.0),
    icon_size: 15.0,
    resting: VISUAL,
    hovered: None,
    pressed: None,
    focused: None,
    disabled: None,
    transition: None,
};
const STATE: WindowChromeStateStyle = WindowChromeStateStyle {
    title_bar_visible: true,
    frame_radius: 12.0,
    shadow: None,
    resize_regions: true,
    resize_edge: 6.0,
    resize_hit_slop: Insets::all(2.0),
};
const DESIGN: WindowChromeDesign = WindowChromeDesign {
    motion: crate::WindowMotion::none(),
    active: WindowChromePalette {
        frame_background: ColorRgba8::rgba(20, 24, 32, 255),
        frame_border: Border::all(1.0, ColorRgba8::rgba(80, 90, 120, 255)),
        title_color: ColorRgba8::rgba(255, 255, 255, 255),
        title_weight: 600,
        shadow_color: None,
    },
    inactive: WindowChromePalette {
        frame_background: ColorRgba8::rgba(30, 34, 42, 255),
        frame_border: Border::all(1.0, ColorRgba8::rgba(60, 65, 80, 255)),
        title_color: ColorRgba8::rgba(180, 180, 190, 255),
        title_weight: 400,
        shadow_color: None,
    },
    normal: STATE,
    maximized: WindowChromeStateStyle {
        frame_radius: 0.0,
        shadow: None,
        resize_regions: false,
        resize_edge: 0.0,
        resize_hit_slop: Insets::ZERO,
        ..STATE
    },
    tiled: STATE,
    fullscreen: WindowChromeStateStyle {
        title_bar_visible: false,
        frame_radius: 0.0,
        shadow: None,
        resize_regions: false,
        resize_edge: 0.0,
        resize_hit_slop: Insets::ZERO,
    },
    title_bar: WindowTitleBarStyle {
        font_family: "sans-serif",
        height: 42.0,
        padding: Insets::symmetric(6.0, 8.0),
        gap: 6.0,
        title_size: 14.0,
        app_icon_region_size: 30.0,
        app_icon_size: 20.0,
        show_client_icon: true,
        fallback_app_icon: None,
        app_icon_opens_system_menu: true,
    },
    controls: WindowControlsDesign {
        minimize: WindowControlDesign {
            icon: icon("icons/minimize.svg"),
            style: BUTTON,
        },
        maximize: WindowControlDesign {
            icon: icon("icons/maximize.svg"),
            style: BUTTON,
        },
        restore: WindowControlDesign {
            icon: icon("icons/restore.svg"),
            style: BUTTON,
        },
        close: WindowControlDesign {
            icon: icon("icons/close.svg"),
            style: BUTTON,
        },
        gap: 6.0,
    },
    content_background: ColorRgba8::rgba(10, 12, 18, 255),
    resize_preview: None,
};

#[test]
fn design_validation_accepts_finite_complete_chrome() {
    assert_eq!(DESIGN.validate(), Ok(DESIGN));
}

#[test]
fn resize_preview_border_validation_is_independent_of_frame_border() {
    for width in [-1.0, f32::NAN, f32::INFINITY] {
        let mut preview = crate::ResizePreviewDesign::new(crate::Fill::None);
        preview.border.left.width = width;
        let design = WindowChromeDesign {
            resize_preview: Some(preview),
            ..DESIGN
        };
        assert_eq!(
            design.validate(),
            Err(WindowChromeDesignError::InvalidResizePreviewBorder)
        );
    }
}

#[test]
fn content_style_preserves_alpha_preview_inheritance_and_state_radius() {
    for alpha in [0, 128, 255] {
        for preview in [
            None,
            Some(crate::ResizePreviewDesign::new(crate::Fill::Color(
                ColorRgba8::rgba(30, 40, 50, alpha),
            ))),
            Some(crate::ResizePreviewDesign::new(crate::Fill::Glass(
                crate::GlassStyle {
                    tint: ColorRgba8::rgba(30, 40, 50, alpha),
                    blur_radius: 24.0,
                    ..crate::GlassStyle::liquid()
                },
            ))),
        ] {
            let design = WindowChromeDesign {
                content_background: ColorRgba8::rgba(0, 0, 0, alpha),
                resize_preview: preview,
                ..DESIGN
            };
            assert_eq!(design.validate(), Ok(design));
            for state in [
                WindowChromeState::Normal,
                WindowChromeState::Maximized,
                WindowChromeState::Tiled,
                WindowChromeState::Fullscreen,
            ] {
                let style = easy_window_frame(design)
                    .content_style(&WindowChromeModel::new(7, "Editor").state(state))
                    .unwrap();
                assert_eq!(style.background, design.content_background);
                assert_eq!(style.resize_preview, preview);
                assert_eq!(style.corner_radius, 0.0);
            }
        }
    }
}

#[test]
fn template_composes_a_distinct_model_owned_component() {
    let model = WindowChromeModel::new(7, "Editor").active(true);
    let component = easy_window_frame(DESIGN).compose(model.clone());
    assert_eq!(component.model, model);
    assert_eq!(component.design, DESIGN);
}

#[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
#[test]
fn fractional_content_background_does_not_leak_into_frame_strips() {
    use crate::host::application::AppRuntimeCore;
    use crate::foundation::{MonotonicInstant, RectI, SizeI};
    use crate::graphics::render::RenderBackend;
    use crate::graphics::renderers::software::{SoftwareCompositeLayer, SoftwareRenderer, SoftwareSurface};
    use crate::shell::window_chrome::WindowChromeSnapshot;

    let extent = SizeI {
        width: 240,
        height: 120,
    };
    for border in [1.0, 1.25, 1.5, 1.75, 2.5] {
        for title_height in [32.0, 32.5] {
            for title_bar_visible in [true, false] {
                let mut reference = None;
                for color in [
                    ColorRgba8::rgba(0, 0, 0, 255),
                    ColorRgba8::rgba(255, 255, 255, 255),
                ] {
                    let mut design = DESIGN;
                    design.active.frame_border =
                        Border::all(border, design.active.frame_border.top.color);
                    design.title_bar.height = title_height;
                    design.content_background = color;
                    let mut runtime = AppRuntimeCore::from_composed_with_extent(
                        easy_window_frame(design).compose(
                            WindowChromeModel::new(7, "")
                                .active(true)
                                .title_bar_visible(title_bar_visible),
                        ),
                        extent,
                    )
                    .unwrap();
                    runtime
                        .prepare_frame(MonotonicInstant::from_nanos(0), true)
                        .unwrap();
                    let snapshot =
                        WindowChromeSnapshot::derive(runtime.ui(), runtime.layout()).unwrap();
                    let content = snapshot.content.bounds;
                    // Match the host's integer client placement and measured size.
                    let cutout = RectI {
                        x: content.x.round() as i32,
                        y: content.y.round() as i32,
                        width: content.width.round() as i32,
                        height: content.height.round() as i32,
                    };
                    let target = RectI {
                        x: 0,
                        y: 0,
                        width: extent.width,
                        height: extent.height,
                    };
                    let mut scene = SoftwareRenderer.create_scene().unwrap();
                    SoftwareRenderer
                        .apply_scene_delta(&mut scene, &runtime.scene_snapshot())
                        .unwrap();
                    let mut surface = SoftwareSurface::default();
                    SoftwareRenderer
                        .render_composite(
                            &mut surface,
                            &[SoftwareCompositeLayer {
                                scene: &scene,
                                target,
                                clip: None,
                                rounded_clips: [None; 2],
                            }],
                            SizeI {
                                width: target.width,
                                height: target.height,
                            },
                            Some(target),
                            ColorRgba8::rgba(0, 255, 0, 255),
                        )
                        .unwrap();
                    let mut strips = Vec::new();
                    for y in 0..target.height {
                        for x in 0..target.width {
                            if x < cutout.x
                                || x >= cutout.right()
                                || y < cutout.y
                                || y >= cutout.bottom()
                            {
                                let offset = ((y * target.width + x) * 4) as usize;
                                strips.extend_from_slice(
                                    &surface.pixels_rgba8()[offset..offset + 4],
                                );
                            }
                        }
                    }
                    if let Some(expected) = &reference {
                        assert!(
                            expected == &strips,
                            "content backing leaked outside client bounds: border={border}, title={title_height}, title_bar_visible={title_bar_visible}"
                        );
                    } else {
                        reference = Some(strips);
                    }
                }
            }
        }
    }
}

#[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
#[test]
fn server_template_can_hide_title_controls_without_changing_outer_style() {
    use crate::host::application::AppRuntimeCore;
    use crate::foundation::{MonotonicInstant, SizeI};
    use crate::shell::window_chrome::{WindowAction, WindowChromeRole, WindowChromeSnapshot};
    for active in [false, true] {
        let mut runtime = AppRuntimeCore::from_composed_with_extent(
            easy_window_frame(DESIGN).compose(
                WindowChromeModel::new(42, "Firefox")
                    .active(active)
                    .title_bar_visible(false),
            ),
            SizeI {
                width: 640,
                height: 480,
            },
        )
        .unwrap();
        runtime
            .prepare_frame(MonotonicInstant::from_nanos(0), true)
            .unwrap();
        let snapshot = WindowChromeSnapshot::derive(runtime.ui(), runtime.layout()).unwrap();
        assert_eq!(snapshot.content.bounds.x, 1.0);
        assert_eq!(snapshot.content.bounds.y, 1.0);
        assert_eq!(snapshot.content.bounds.width, 638.0);
        assert_eq!(snapshot.content.bounds.height, 478.0);
        assert!(snapshot.regions.iter().any(|region| matches!(
            region.role,
            WindowChromeRole::Action(WindowAction::BeginResize(_))
        )));
        assert!(!snapshot.regions.iter().any(|region| matches!(
            region.role,
            WindowChromeRole::Title
                | WindowChromeRole::DragRegion
                | WindowChromeRole::Action(
                    WindowAction::Close
                        | WindowAction::Minimize
                        | WindowAction::ToggleMaximize
                        | WindowAction::BeginMove
                )
        )));
        let style = runtime.ui().box_styles.get(snapshot.frame.node).unwrap();
        assert_eq!(
            style.decoration.corner_radii,
            crate::ui::CornerRadii::all(12.0)
        );
    }
}

#[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
#[test]
fn controls_remain_centered_after_frame_state_updates_without_hover() {
    use crate::host::application::AppRuntimeCore;
    use crate::foundation::{MonotonicInstant, SizeI};

    let mut design = DESIGN;
    design.title_bar.height = 24.0;
    design.title_bar.padding = Insets::ZERO;
    for control in [
        &mut design.controls.minimize,
        &mut design.controls.maximize,
        &mut design.controls.restore,
        &mut design.controls.close,
    ] {
        control.style.height = Dimension::FILL;
        control.style.icon_size = 12.0;
    }
    let template = easy_window_frame(design);
    let extent = SizeI {
        width: 640,
        height: 480,
    };
    let mut runtime = AppRuntimeCore::from_composed_with_extent(
        template.compose(WindowChromeModel::new(42, "Controls").active(true)),
        extent,
    )
    .unwrap();
    for (iteration, state) in [
        WindowChromeState::Normal,
        WindowChromeState::Maximized,
        WindowChromeState::Normal,
        WindowChromeState::Maximized,
        WindowChromeState::Normal,
    ]
    .into_iter()
    .enumerate()
    {
        runtime
            .update_composition_root(Box::new(
                template.compose(
                    WindowChromeModel::new(42, "Controls")
                        .active(true)
                        .state(state),
                ),
            ))
            .unwrap();
        runtime
            .resize(if state == WindowChromeState::Maximized {
                SizeI {
                    width: 1280,
                    height: 800,
                }
            } else {
                extent
            })
            .unwrap();
        runtime
            .prepare_frame(
                MonotonicInstant::from_nanos(iteration as u64 * 1_000_000),
                true,
            )
            .unwrap();
        let controls: Vec<_> = runtime
            .ui()
            .style_bindings()
            .iter()
            .filter(|b| {
                b.local_style.is_some()
                    && b.slots.iter().any(|s| s.slot == StyleSlotId::named("icon"))
            })
            .collect();
        assert_eq!(controls.len(), 3);
        for binding in controls {
            let button = runtime
                .layout()
                .computed(binding.state_root)
                .unwrap()
                .border_rect;
            assert_eq!(button.height, 24.0, "button height in {state:?}");
            let icon_node = binding
                .slots
                .iter()
                .find(|s| s.slot == StyleSlotId::named("icon"))
                .unwrap()
                .node;
            let icon = runtime.layout().computed(icon_node).unwrap().border_rect;
            assert!(
                (icon.y + icon.height / 2.0 - (button.y + button.height / 2.0)).abs() < 0.001,
                "icon not centered in {state:?}: icon={icon:?}, button={button:?}"
            );
        }
    }
}

#[test]
fn control_design_resolves_interaction_visuals_without_a_theme_catalog_entry() {
    let hovered = WindowControlVisual {
        decoration: BoxDecoration::new()
            .background(Background::Color(ColorRgba8::rgba(40, 50, 60, 255))),
        icon_tint: ColorRgba8::rgba(10, 20, 30, 255),
    };
    let style = WindowControlButtonStyle {
        hovered: Some(hovered),
        transition: Some(TransitionSpec {
            duration_ms: 90,
            easing: Easing::EaseOut,
            repeat: false,
        }),
        ..BUTTON
    };
    let compiled = compiled_control_style(style);
    let root = compiled
        .resolve_slot(&[], InteractionFlags::HOVERED, StyleSlotId::named("root"))
        .unwrap();
    let icon = compiled
        .resolve_slot(&[], InteractionFlags::HOVERED, StyleSlotId::named("icon"))
        .unwrap();

    assert_eq!(root.patch.background, Some(hovered.decoration.background));
    assert_eq!(icon.patch.image_tint, Some(Some(hovered.icon_tint)));
    assert_eq!(root.transition.duration_ms, 90);
}
#[test]
fn rounded_controls_keep_their_shape_during_hover_and_interrupted_transitions() {
    use crate::host::application::AppRuntimeCore;
    use crate::foundation::{MonotonicInstant, SizeI};
    use crate::ui::CornerRadii;

    let mut design = DESIGN;
    let radius = CornerRadii::all(7.0);
    for control in [
        &mut design.controls.minimize,
        &mut design.controls.maximize,
        &mut design.controls.restore,
        &mut design.controls.close,
    ] {
        control.style.resting.decoration = BoxDecoration::new()
            .background(Background::Color(ColorRgba8::rgba(30, 40, 50, 255)))
            .corner_radii(radius);
        control.style.hovered = Some(WindowControlVisual {
            decoration: control
                .style
                .resting
                .decoration
                .background(Background::Color(ColorRgba8::rgba(120, 140, 160, 255))),
            ..control.style.resting
        });
        control.style.transition = Some(TransitionSpec {
            duration_ms: 100,
            easing: Easing::Linear,
            repeat: false,
        });
    }
    let mut runtime = AppRuntimeCore::from_composed_with_extent(
        easy_window_frame(design).compose(WindowChromeModel::new(42, "Rounded")),
        SizeI {
            width: 640,
            height: 480,
        },
    )
    .unwrap();
    runtime.prepare_frame(MonotonicInstant::ZERO, true).unwrap();
    let nodes: Vec<_> = runtime
        .ui()
        .style_bindings()
        .iter()
        .filter(|binding| binding.local_style.is_some())
        .map(|binding| binding.state_root)
        .collect();
    assert_eq!(nodes.len(), 3);
    let mut saw_intermediate_color = false;
    for ms in 1..=350 {
        if let Some(hovered) = match ms {
            1 | 61 => Some(true),
            41 | 201 => Some(false),
            _ => None,
        } {
            for &node in &nodes {
                runtime.ui_mut().route_interaction_flag(
                    node,
                    InteractionFlags::HOVERED,
                    hovered,
                );
            }
        }
        runtime
            .prepare_frame(MonotonicInstant::from_nanos(ms * 1_000_000), true)
            .unwrap();
        for &node in &nodes {
            let decoration = runtime.ui().box_styles.get(node).unwrap().decoration;
            assert_eq!(decoration.corner_radii, radius, "radius changed at {ms}ms");
            if let Background::Color(color) = decoration.background {
                saw_intermediate_color |= color.r > 30 && color.r < 120;
            }
        }
    }
    assert!(
        saw_intermediate_color,
        "the check must sample an active color transition"
    );
}
#[cfg(feature = "application-software")]
#[test]
fn square_flush_controls_preserve_the_rounded_window_border() {
    use crate::host::application::AppRuntimeCore;
    use crate::foundation::{MonotonicInstant, RectI, SizeI};
    use crate::graphics::render::RenderBackend;
    use crate::graphics::renderers::software::{SoftwareCompositeLayer, SoftwareRenderer, SoftwareSurface};

    let extent = SizeI {
        width: 200,
        height: 100,
    };
    for (radius, border) in [(14.0, 2.0), (14.0, 0.0), (0.0, 2.0)] {
        let mut design = DESIGN;
        design.active.frame_border = Border::all(border, ColorRgba8::rgba(255, 0, 0, 255));
        design.active.frame_background = ColorRgba8::rgba(0, 255, 0, 255);
        design.normal.frame_radius = radius;
        design.normal.shadow = None;
        design.title_bar.height = 32.0;
        design.title_bar.padding = Insets::ZERO;
        design.title_bar.show_client_icon = false;
        design.controls.gap = 0.0;
        design.controls.close.style.height = Dimension::FILL;
        design.controls.close.style.resting.decoration = BoxDecoration::new()
            .background(Background::Color(ColorRgba8::rgba(0, 0, 255, 255)));
        let mut runtime = AppRuntimeCore::from_composed_with_extent(
            easy_window_frame(design).compose(WindowChromeModel::new(42, "").active(true)),
            extent,
        )
        .unwrap();
        runtime.prepare_frame(MonotonicInstant::ZERO, true).unwrap();
        let mut scene = SoftwareRenderer.create_scene().unwrap();
        SoftwareRenderer
            .apply_scene_delta(&mut scene, &runtime.scene_snapshot())
            .unwrap();
        let mut surface = SoftwareSurface::default();
        SoftwareRenderer
            .render_composite(
                &mut surface,
                &[SoftwareCompositeLayer {
                    scene: &scene,
                    target: RectI {
                        x: 0,
                        y: 0,
                        width: extent.width,
                        height: extent.height,
                    },
                    clip: None,
                    rounded_clips: [None; 2],
                }],
                extent,
                None,
                ColorRgba8::rgba(0, 0, 0, 0),
            )
            .unwrap();
        let pixel = |x: usize, y: usize| {
            let index = (y * extent.width as usize + x) * 4;
            &surface.pixels_rgba8()[index..index + 4]
        };
        assert_eq!(pixel(195, 20), [0, 0, 255, 255], "button interior");
        if radius > 0.0 {
            assert_eq!(
                pixel(199, 0),
                [0, 0, 0, 0],
                "outer corner must stay transparent"
            );
            if border > 0.0 {
                assert_eq!(
                    pixel(195, 6)[3],
                    255,
                    "inner antialias seam must remain opaque"
                );
                assert!(
                    pixel(195, 6)[2] > 0,
                    "partially covered inner corner must contain button color, not just frame fill"
                );
                assert_eq!(
                    pixel(196, 6),
                    [255, 0, 0, 255],
                    "button painted over curved border"
                );
                assert_eq!(
                    pixel(194, 6),
                    [0, 0, 255, 255],
                    "button should follow inner curve"
                );
            }
        }
        if border > 0.0 {
            assert_eq!(pixel(199, 20), [255, 0, 0, 255], "right border");
        }
    }
}

#[test]
fn server_content_backing_is_independent_of_titlebar_visibility() {
    for alpha in [255, 128, 0] {
        let design = WindowChromeDesign {
            content_background: ColorRgba8::rgba(15, 18, 26, alpha),
            ..DESIGN
        };
        let template = easy_window_frame(design);
        for title_bar_visible in [false, true] {
            let model = WindowChromeModel::new(7, "Client decorations")
                .title_bar_visible(title_bar_visible);
            let style = template.content_style(&model).unwrap();
            assert_eq!(style.background, design.content_background,
                "header ownership must not punch holes in the window backing");
            assert_eq!(style.resize_preview, design.resize_preview);
        }
    }
}

#[cfg(all(feature = "application-software", target_os = "linux"))]
mod hover_border;

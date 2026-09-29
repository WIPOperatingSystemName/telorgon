    use super::*;
    use crate::test_alloc;
    use crate::theme::CompiledTheme;
    use crate::ui::{Background, BoxStyle, LayoutStyle, MountWriter};

    #[test]
    fn color_interpolation_is_linear_premultiplied_and_endpoints_are_exact() {
        let transparent = ColorRgba8::rgba(255, 0, 0, 0);
        let opaque = ColorRgba8::rgba(0, 0, 255, 255);
        assert_eq!(
            interpolate_color(transparent, opaque, 0.0),
            ColorRgba8::default()
        );
        assert_eq!(interpolate_color(transparent, opaque, 1.0), opaque);
        let middle = interpolate_color(transparent, opaque, 0.5);
        assert_eq!(middle.a, 128);
        assert!(middle.b > 240 && middle.r < 10);
    }

    fn motion_theme(hovered: &str, pressed: &str, repeat: bool) -> CompiledTheme {
        let source = format!(
            r##"
format = "v4"
domain = "application"
[components.button.default]
transition = {{ duration = 100, easing = "linear", repeat = {repeat} }}
[components.button.default.states.hovered.slots.root]
background = "{hovered}"
opacity = 0.2
translation_x = 10
[components.button.default.states.pressed.slots.root]
background = "{pressed}"
opacity = 0.5
translation_x = 4
"##
        );
        CompiledTheme::compile(
            &crate::theme::ThemeSource::parse(&source).unwrap(),
            &crate::theme::application_catalog(),
        )
        .unwrap()
    }

    fn themed_button() -> (MountedUi, NodeId) {
        let mut ui = MountedUi::default();
        let button = {
            let mut writer = MountWriter::<()>::new(&mut ui);
            let mut button = None;
            writer.root(BoxStyle::default(), LayoutStyle::default(), |writer| {
                button = Some(writer.button_node(
                    BoxStyle {
                        decoration: crate::ui::BoxDecoration {
                            background: Background::Color(ColorRgba8::rgba(20, 30, 40, 255)),
                            ..crate::ui::BoxDecoration::default()
                        },
                        ..BoxStyle::default()
                    },
                    |_| {},
                ));
            });
            button.unwrap().node
        };
        (ui, button)
    }

    #[test]
    fn fake_clock_samples_retargets_theme_swaps_and_settles_without_work() {
        let mut runtime = ThemeRuntime::default();
        runtime
            .replace_theme(
                ThemeRuntime::root_scope(crate::theme::ThemeDomain::Application),
                motion_theme("#ff0000ff", "#0000ffff", false),
            )
            .unwrap();
        let (mut ui, button) = themed_button();
        runtime.update_styles(
            &mut ui,
            MonotonicInstant::from_nanos(0),
            MotionPreference::Full,
        );
        ui.route_interaction_flag(button, crate::ui::InteractionFlags::HOVERED, true);
        let start = runtime.update_styles(
            &mut ui,
            MonotonicInstant::from_nanos(0),
            MotionPreference::Full,
        );
        assert!(start.active_animations);
        assert!(!start.changed);

        let middle = runtime.update_styles(
            &mut ui,
            MonotonicInstant::from_nanos(50_000_000),
            MotionPreference::Full,
        );
        assert!(middle.changed);
        let halfway = *ui.box_styles.get(button).unwrap();
        assert!((halfway.opacity - 0.6).abs() < 0.02);
        assert!((halfway.transform.translation.x - 5.0).abs() < 0.02);

        ui.route_interaction_flag(button, crate::ui::InteractionFlags::PRESSED, true);
        runtime.update_styles(
            &mut ui,
            MonotonicInstant::from_nanos(50_000_000),
            MotionPreference::Full,
        );
        assert!(runtime.diagnostics().retargets >= 1);

        runtime
            .replace_theme(
                ThemeRuntime::root_scope(crate::theme::ThemeDomain::Application),
                motion_theme("#00ff00ff", "#ffffffff", false),
            )
            .unwrap();
        runtime.update_styles(
            &mut ui,
            MonotonicInstant::from_nanos(75_000_000),
            MotionPreference::Full,
        );
        assert!(runtime.diagnostics().retargets >= 2);
        let settled = runtime.update_styles(
            &mut ui,
            MonotonicInstant::from_nanos(200_000_000),
            MotionPreference::Full,
        );
        assert!(!settled.active_animations);
        let idle = runtime.update_styles(
            &mut ui,
            MonotonicInstant::from_nanos(201_000_000),
            MotionPreference::Full,
        );
        assert!(!idle.changed);
        assert!(!idle.active_animations);
    }

    #[test]
    fn reduced_motion_caps_opacity_and_suspends_invisible_repeating_tracks() {
        let mut runtime = ThemeRuntime::default();
        runtime
            .replace_theme(
                ThemeRuntime::root_scope(crate::theme::ThemeDomain::Application),
                motion_theme("#ff0000ff", "#0000ffff", false),
            )
            .unwrap();
        let (mut ui, button) = themed_button();
        runtime.update_styles(
            &mut ui,
            MonotonicInstant::from_nanos(0),
            MotionPreference::Reduced,
        );
        ui.route_interaction_flag(button, crate::ui::InteractionFlags::HOVERED, true);
        runtime.update_styles(
            &mut ui,
            MonotonicInstant::from_nanos(0),
            MotionPreference::Reduced,
        );
        runtime.update_styles(
            &mut ui,
            MonotonicInstant::from_nanos(1),
            MotionPreference::Reduced,
        );
        assert_eq!(
            ui.box_styles.get(button).unwrap().transform.translation.x,
            10.0
        );
        runtime.update_styles(
            &mut ui,
            MonotonicInstant::from_nanos(100_000_000),
            MotionPreference::Reduced,
        );
        assert!((ui.box_styles.get(button).unwrap().opacity - 0.2).abs() < 0.01);

        let mut repeating = ThemeRuntime::default();
        repeating
            .replace_theme(
                ThemeRuntime::root_scope(crate::theme::ThemeDomain::Application),
                motion_theme("#ff0000ff", "#0000ffff", true),
            )
            .unwrap();
        let (mut ui, button) = themed_button();
        repeating.update_styles(
            &mut ui,
            MonotonicInstant::from_nanos(0),
            MotionPreference::Full,
        );
        ui.route_interaction_flag(button, crate::ui::InteractionFlags::HOVERED, true);
        assert!(
            repeating
                .update_styles(
                    &mut ui,
                    MonotonicInstant::from_nanos(0),
                    MotionPreference::Full
                )
                .active_animations
        );
        ui.interactions.get_mut(button).unwrap().visible = false;
        assert!(
            !repeating
                .update_styles(
                    &mut ui,
                    MonotonicInstant::from_nanos(20_000_000),
                    MotionPreference::Full,
                )
                .active_animations
        );
    }

    #[test]
    fn warmed_single_binding_interaction_avoids_scans_and_allocations() {
        let mut runtime = ThemeRuntime::default();
        runtime
            .replace_theme(
                ThemeRuntime::root_scope(crate::theme::ThemeDomain::Application),
                motion_theme("#ff0000ff", "#0000ffff", false),
            )
            .unwrap();
        let (mut ui, button) = themed_button();
        runtime.update_styles(
            &mut ui,
            MonotonicInstant::from_nanos(0),
            MotionPreference::Full,
        );

        for (hovered, start, end) in [
            (true, 1_000_000, 201_000_000),
            (false, 202_000_000, 402_000_000),
        ] {
            ui.route_interaction_flag(button, crate::ui::InteractionFlags::HOVERED, hovered);
            runtime.update_styles(
                &mut ui,
                MonotonicInstant::from_nanos(start),
                MotionPreference::Full,
            );
            runtime.update_styles(
                &mut ui,
                MonotonicInstant::from_nanos(end),
                MotionPreference::Full,
            );
        }

        let before = runtime.diagnostics().bindings_evaluated;
        test_alloc::begin();
        ui.route_interaction_flag(button, crate::ui::InteractionFlags::HOVERED, true);
        runtime.update_styles(
            &mut ui,
            MonotonicInstant::from_nanos(403_000_000),
            MotionPreference::Full,
        );
        assert_eq!(test_alloc::finish(), 0);
        assert_eq!(runtime.diagnostics().bindings_evaluated - before, 1);
    }

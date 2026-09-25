use telorgon::assets::{builtin, fonts};
use telorgon::{AssetBundle, AssetCatalogError, AssetEntry, AssetKey, AssetKind, AssetResolver};

#[test]
#[cfg(feature = "font-inter")]
fn inter_is_available_by_typed_handle_without_a_project_catalog() {
    let entry = fonts::INTER.resolve(AssetBundle::EMPTY).unwrap();
    assert_eq!(entry.kind, AssetKind::Font);
    assert_eq!(fonts::INTER.resolve(builtin::bundle()).unwrap(), entry);
    let mut text = telorgon::ui::text::TextEngine::without_system_fonts().unwrap();
    let run = text
        .prepare_text(telorgon::ui::text::TextLayoutRequest {
            text: "Settings",
            style: telorgon::ui::text::ResolvedTextStyle::new(
                telorgon::foundation::ColorRgba8::rgba(255, 255, 255, 255),
                16,
            ),
            max_width_px: None,
            max_height_px: None,
        })
        .unwrap();
    assert!(!run.glyphs.is_empty());
}

#[test]
fn sdk_catalog_coexists_with_project_assets_and_reserves_its_namespace() {
    static PROJECT: &[AssetEntry] = &[AssetEntry::embedded(
        AssetKey::new("icons/custom"),
        AssetKind::Icon,
        "image/svg+xml",
        b"<svg/>",
    )];
    let resolver = AssetResolver::new(AssetBundle::new(PROJECT)).unwrap();
    assert!(resolver.get(AssetKey::new("icons/custom")).is_some());
    for entry in builtin::bundle().iter() {
        assert_eq!(resolver.get(entry.key), Some(entry));
    }
    static COLLISION: &[AssetEntry] = &[AssetEntry::embedded(
        AssetKey::new("telorgon/icons/future"),
        AssetKind::Icon,
        "image/svg+xml",
        b"<svg/>",
    )];
    assert!(matches!(
        AssetResolver::new(AssetBundle::new(COLLISION)),
        Err(AssetCatalogError::ReservedKey(_))
    ));
}

#[test]
fn every_enabled_font_parses_and_has_an_original_license() {
    for entry in builtin::bundle().iter() {
        let mut db = cosmic_text::fontdb::Database::new();
        db.load_font_data(entry.bytes.to_vec());
        assert!(!db.is_empty(), "{}", entry.key);
        for face in db.faces() {
            assert!(
                fonts::LICENSES.iter().any(|notice| {
                    face.families
                        .iter()
                        .any(|(family, _)| family == notice.family)
                        && notice.text.contains("SIL OPEN FONT LICENSE")
                }),
                "{}: {:?}",
                entry.key,
                face.families
            );
        }
    }
}

#[cfg(feature = "bundled-fonts")]
mod rendering {
    use super::*;
    use telorgon::foundation::ColorRgba8;
    use telorgon::ui::text::{ResolvedTextStyle, TextEngine, TextLayoutRequest, Typography};

    #[telorgon::component]
    struct FontPreview {}

    impl telorgon::app::Component for FontPreview {
        fn view(&self) -> impl telorgon::app::View {
            telorgon::app::text("Settings Il1 O0 0123456789")
        }
    }

    #[test]
    fn runtime_remeasures_after_changing_default_font() {
        let mut runtime =
            telorgon::ComposedAppRuntime::from_composed(FontPreview::default()).unwrap();
        runtime
            .prepare_frame(telorgon::MonotonicInstant::ZERO, true)
            .unwrap();
        runtime.set_typography(
            Typography::default().ui_font(fonts::ATKINSON_HYPERLEGIBLE_NEXT_FAMILY),
        );
        let frame = runtime
            .prepare_frame(telorgon::MonotonicInstant::ZERO, false)
            .unwrap();
        assert!(frame.diagnostics.layout.measured > 0);
    }

    fn render(family: &str, weight: u16, typography: Typography, system: bool) -> (f32, Vec<u8>) {
        let mut engine = if system {
            TextEngine::new()
        } else {
            TextEngine::without_system_fonts()
        }
        .unwrap();
        engine.set_typography(typography);
        let result = engine
            .prepare_text(TextLayoutRequest {
                text: "Settings Il1 O0 0123456789",
                style: ResolvedTextStyle::new(ColorRgba8::rgba(255, 255, 255, 255), 24)
                    .typography(family, weight, 32),
                max_width_px: None,
                max_height_px: None,
            })
            .unwrap();
        assert!(!result.glyphs.is_empty());
        assert!(engine.atlas().pixels_a8.iter().any(|p| *p != 0));
        (result.advance_width_px, engine.atlas().pixels_a8.to_vec())
    }

    #[test]
    fn generic_defaults_and_accessibility_override_work_without_system_fonts() {
        for (generic, family) in [
            ("sans-serif", fonts::INTER_FAMILY),
            ("serif", fonts::SOURCE_SERIF_4_FAMILY),
            ("monospace", fonts::JETBRAINS_MONO_FAMILY),
        ] {
            assert_eq!(
                render(generic, 400, Typography::default(), false),
                render(family, 400, Typography::default(), false)
            );
        }
        assert_eq!(
            render(
                "sans-serif",
                400,
                Typography::default().ui_font(fonts::ATKINSON_HYPERLEGIBLE_NEXT_FAMILY),
                false
            ),
            render(
                fonts::ATKINSON_HYPERLEGIBLE_NEXT_FAMILY,
                400,
                Typography::default(),
                false
            )
        );
        render(
            fonts::INSTRUMENT_SANS_FAMILY,
            500,
            Typography::default(),
            false,
        );
    }

    #[test]
    fn weights_change_rasterization_and_system_fonts_do_not_change_inter() {
        let normal = render("Inter", 400, Typography::default(), false);
        let medium = render("Inter", 500, Typography::default(), false);
        let semibold = render("Inter", 600, Typography::default(), false);
        assert_ne!(normal.1, medium.1);
        assert_ne!(medium.1, semibold.1);
        assert_eq!(normal, render("Inter", 400, Typography::default(), true));
    }

    #[test]
    fn changing_typography_invalidates_retained_measurements() {
        use telorgon::ui::text::{RetainedTextRequest, RetainedTextSystem, TextRunKey};
        let mut text = RetainedTextSystem::new(1024).unwrap();
        let request = RetainedTextRequest {
            key: TextRunKey::new(1, 1, "sans-serif", 24.0, 400, 32.0, None, None, 1.0),
            text: "Settings Il1 O0 0123456789",
            family: "sans-serif",
            font_size_px: 24,
            line_height_px: 32,
            max_width_px: None,
            max_height_px: None,
        };
        let id = text.measure(request.clone()).unwrap();
        let before = text.run(id).unwrap().advance_width_px;
        text.set_typography(
            Typography::default().ui_font(fonts::ATKINSON_HYPERLEGIBLE_NEXT_FAMILY),
        );
        let id = text.measure(request).unwrap();
        let after = text.run(id).unwrap().advance_width_px;
        assert_ne!(before, after);
        assert_eq!(
            after,
            render(
                fonts::ATKINSON_HYPERLEGIBLE_NEXT_FAMILY,
                400,
                Typography::default(),
                false
            )
            .0
        );
    }
}

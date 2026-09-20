use super::*;

#[test]
fn missing_roles_and_size_are_reported_together() {
    let error = CursorTheme::new()
        .prepare(AssetBundle::EMPTY, ClientCursorMode::Allow)
        .unwrap_err()
        .to_string();
    for role in REQUIRED_CURSOR_ROLES {
        assert!(error.contains(role.name()), "{error}");
    }
    assert!(error.contains("logical size"));
}

#[test]
fn every_role_is_required_without_implicit_aliasing() {
    for missing in REQUIRED_CURSOR_ROLES {
        let mut theme = cursor_test_theme();
        theme.graphics.remove(&missing);
        let error = theme
            .prepare(cursor_test_bundle(), ClientCursorMode::Allow)
            .unwrap_err()
            .to_string();
        assert!(error.contains(&format!("missing cursor `{}`", missing.name())));
    }
}

#[test]
fn complete_theme_scales_artwork_and_preserves_explicit_proportions() {
    let theme = cursor_test_theme()
        .text(
            cursor(CursorAsset::new(AssetKey::new("cursor.svg")))
                .size(30.0)
                .hotspot(16, 16),
        )
        .cursor_size(32.0);
    let prepared = theme
        .prepare(cursor_test_bundle(), ClientCursorMode::ThemeOnly)
        .unwrap();
    for role in REQUIRED_CURSOR_ROLES {
        let graphic = prepared.pointer_overrides().graphic(role).unwrap();
        let expected = if role == PointerIcon::Text { 40 } else { 32 };
        assert_eq!(graphic.logical_size(), Some(expected));
        assert_eq!(
            graphic.pointer_hotspot(),
            PointerHotspot::new(expected / 2, expected / 2)
        );
    }
    assert!(matches!(
        resolve_pointer(
            PointerRequest::ClientSurface,
            prepared.client_cursor_mode(),
            prepared.pointer_overrides(),
            None
        ),
        PointerResolution::Graphic(_)
    ));
    assert_eq!(
        resolve_pointer(
            PointerRequest::Hidden,
            ClientCursorMode::Allow,
            prepared.pointer_overrides(),
            None
        ),
        PointerResolution::Hidden
    );
    assert_eq!(
        resolve_pointer(
            PointerRequest::ClientSurface,
            ClientCursorMode::Allow,
            prepared.pointer_overrides(),
            None
        ),
        PointerResolution::ClientSurface
    );
}

#[test]
fn asset_themes_require_every_role_and_keep_fractional_sizes() {
    use crate::assets::AssetEntry;
    let manifest =
        REQUIRED_CURSOR_ROLES
            .iter()
            .fold(String::from("size = 24.5\n"), |mut text, role| {
                text.push_str(&format!(
                    "[{}]\nasset = \"cursor.svg\"\nhotspot = [16, 16]\n",
                    role.name()
                ));
                text
            });
    let source = Box::leak(manifest.into_bytes().into_boxed_slice());
    let mut entries = cursor_test_bundle().iter().copied().collect::<Vec<_>>();
    let key = AssetKey::new("theme.toml");
    entries.push(AssetEntry::embedded(
        key,
        AssetKind::CursorTheme,
        "application/toml",
        source,
    ));
    let bundle = AssetBundle::new(Box::leak(entries.into_boxed_slice()));
    let theme = CursorTheme::from_asset(CursorThemeAsset::new(key));
    let prepared = theme.prepare(bundle, ClientCursorMode::Allow).unwrap();
    assert_eq!(
        prepared
            .pointer_overrides()
            .graphic(PointerIcon::Default)
            .unwrap()
            .exact_logical_size(),
        Some(24.5)
    );
    let prepared = theme
        .size(32.0)
        .prepare(bundle, ClientCursorMode::Allow)
        .unwrap();
    assert_eq!(
        prepared
            .pointer_overrides()
            .graphic(PointerIcon::Default)
            .unwrap()
            .exact_logical_size(),
        Some(32.0)
    );
}

#[test]
fn invalid_metadata_and_assets_fail_preparation() {
    for size in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert!(
            cursor_test_theme()
                .size(size)
                .prepare(cursor_test_bundle(), ClientCursorMode::Allow)
                .is_err()
        );
    }
    let invalid =
        cursor_test_theme().text(cursor(CursorAsset::new(AssetKey::new("missing.svg"))));
    assert!(
        invalid
            .prepare(cursor_test_bundle(), ClientCursorMode::Allow)
            .unwrap_err()
            .to_string()
            .contains("missing.svg")
    );
    let invalid = cursor_test_theme()
        .text(cursor(CursorAsset::new(AssetKey::new("cursor.svg"))).hotspot(32, 0));
    assert!(
        invalid
            .prepare(cursor_test_bundle(), ClientCursorMode::Allow)
            .unwrap_err()
            .to_string()
            .contains("hotspot")
    );
}

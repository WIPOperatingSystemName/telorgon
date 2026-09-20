use super::*;
use crate::assets::AssetEntry;

const THEME: CursorThemeAsset = CursorThemeAsset::new(AssetKey::new("cursors/theme.toml"));
const POINTER: CursorAsset = CursorAsset::new(AssetKey::new("cursors/pointer.svg"));
static ENTRIES: [AssetEntry; 2] = [
    AssetEntry::embedded(
        THEME.key(),
        AssetKind::CursorTheme,
        "application/toml",
        b"fallback = \"system\"\nsize = 24\n[pointer]\nasset = \"cursors/pointer.svg\"\nhotspot = [3, 2]\n",
    ),
    AssetEntry::embedded(
        POINTER.key(),
        AssetKind::Cursor,
        "image/svg+xml",
        b"<svg/>",
    ),
];

#[test]
fn pointer_graphics_retain_code_defined_tint() {
    let white = ColorRgba8::rgba(255, 255, 255, 255);
    let graphic = CursorGraphic::new(POINTER)
        .size(32)
        .hotspot(3, 2)
        .tint(white);

    assert_eq!(graphic.tint_color(), Some(white));
    assert_eq!(graphic.without_tint().tint_color(), None);
}

#[test]
fn manifest_and_override_use_fixed_precedence() {
    let bundle = AssetBundle::new(&ENTRIES);
    let theme = PointerTheme::from_asset(THEME, bundle).unwrap();
    let override_graphic = CursorGraphic::new(POINTER).hotspot(1, 1);
    let overrides = PointerThemeOverrides::new().pointer(override_graphic.clone());
    assert_eq!(
        resolve_pointer(
            PointerRequest::Semantic(PointerIcon::Pointer),
            ClientCursorMode::Allow,
            &overrides,
            Some(&theme)
        ),
        PointerResolution::Graphic(&override_graphic)
    );
    assert_eq!(
        theme
            .graphic(PointerIcon::Pointer)
            .unwrap()
            .pointer_hotspot(),
        PointerHotspot::new(3, 2)
    );
}

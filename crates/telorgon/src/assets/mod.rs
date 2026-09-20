//! Target-neutral, typed references to project-owned embedded media.

use std::{fmt, slice};

use crate::foundation::ColorRgba8;
use crate::ui::ImageId;

mod cursor_theme;
mod media;
pub use cursor_theme::{
    ClientCursorMode, CursorGraphic, CursorTheme, CursorThemeError, PointerConfiguration,
    PointerFrame, PointerHotspot, PointerRequest, PointerResolution, PointerTheme,
    PointerThemeFallback, PointerThemeOverrides, REQUIRED_CURSOR_ROLES, cursor, resolve_pointer,
};
pub use media::{AssetMediaCache, AssetMediaError, AssetRasterSize, DecodedAssetImage};

/// Stable, slash-separated identity of one asset inside a catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AssetKey(&'static str);

impl AssetKey {
    pub const fn new(path: &'static str) -> Self {
        Self(path)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for AssetKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AssetKind {
    Font,
    Icon,
    Image,
    Cursor,
    CursorTheme,
}

/// One immutable file embedded by an `asset_catalog!` declaration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AssetEntry {
    pub key: AssetKey,
    pub kind: AssetKind,
    pub media_type: &'static str,
    pub bytes: &'static [u8],
}

impl AssetEntry {
    pub const fn embedded(
        key: AssetKey,
        kind: AssetKind,
        media_type: &'static str,
        bytes: &'static [u8],
    ) -> Self {
        Self {
            key,
            kind,
            media_type,
            bytes,
        }
    }
}

/// Cheap, copyable view over all entries generated for one project asset directory.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AssetBundle {
    entries: &'static [AssetEntry],
}

impl AssetBundle {
    pub const EMPTY: Self = Self { entries: &[] };

    pub const fn new(entries: &'static [AssetEntry]) -> Self {
        Self { entries }
    }

    pub const fn len(self) -> usize {
        self.entries.len()
    }

    pub const fn is_empty(self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(self) -> slice::Iter<'static, AssetEntry> {
        self.entries.iter()
    }

    pub fn get(self, key: AssetKey) -> Option<&'static AssetEntry> {
        self.entries.iter().find(|entry| entry.key == key)
    }

    pub fn validate(self) -> Result<Self, AssetCatalogError> {
        for (index, entry) in self.entries.iter().enumerate() {
            validate_key(entry.key)?;
            if entry.media_type.trim().is_empty() {
                return Err(AssetCatalogError::MissingMediaType(entry.key));
            }
            if entry.bytes.is_empty() {
                return Err(AssetCatalogError::EmptyAsset(entry.key));
            }
            if self.entries[..index]
                .iter()
                .any(|candidate| candidate.key == entry.key)
            {
                return Err(AssetCatalogError::DuplicateKey(entry.key));
            }
        }
        Ok(self)
    }
}

impl IntoIterator for AssetBundle {
    type Item = &'static AssetEntry;
    type IntoIter = slice::Iter<'static, AssetEntry>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Implemented by the marker type generated inside each `asset_catalog!` module.
pub trait AssetCatalog {
    const BUNDLE: AssetBundle;
}

macro_rules! typed_asset {
    ($name:ident, $kind:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(AssetKey);

        impl $name {
            pub const fn new(key: AssetKey) -> Self {
                Self(key)
            }

            pub const fn key(self) -> AssetKey {
                self.0
            }

            pub fn resolve(self, bundle: AssetBundle) -> Result<&'static AssetEntry, AssetError> {
                let entry = bundle.get(self.0).ok_or(AssetError::NotFound(self.0))?;
                if entry.kind != AssetKind::$kind {
                    return Err(AssetError::KindMismatch {
                        key: self.0,
                        expected: AssetKind::$kind,
                        actual: entry.kind,
                    });
                }
                Ok(entry)
            }
        }

        impl From<$name> for AssetKey {
            fn from(value: $name) -> Self {
                value.key()
            }
        }
    };
}

typed_asset!(IconAsset, Icon);
typed_asset!(FontAsset, Font);
typed_asset!(ImageAsset, Image);
typed_asset!(CursorAsset, Cursor);
typed_asset!(CursorThemeAsset, CursorTheme);

impl IconAsset {
    pub const fn image_id(self) -> ImageId {
        asset_image_id(self.key())
    }
}

impl ImageAsset {
    pub const fn image_id(self) -> ImageId {
        asset_image_id(self.key())
    }
}

/// Target-neutral icon value accepted by ordinary controls and native window metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Icon {
    source: IconAsset,
    tint: Option<ColorRgba8>,
}

impl Icon {
    pub const fn new(source: IconAsset) -> Self {
        Self { source, tint: None }
    }

    pub const fn source(self) -> IconAsset {
        self.source
    }

    /// Recolors the decoded artwork from its alpha mask while preserving transparency.
    pub const fn tint(mut self, color: ColorRgba8) -> Self {
        self.tint = Some(color);
        self
    }

    pub const fn without_tint(mut self) -> Self {
        self.tint = None;
        self
    }

    pub const fn tint_color(self) -> Option<ColorRgba8> {
        self.tint
    }

    pub const fn image_id(self) -> ImageId {
        self.source.image_id()
    }
}

impl From<IconAsset> for Icon {
    fn from(value: IconAsset) -> Self {
        Self::new(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AppIconVariant {
    pub icon: Icon,
    /// Logical square edge this source is optimized for. `None` denotes a scalable source.
    pub size: Option<u16>,
}

/// Cross-target application/window icon declaration.
///
/// A stock icon name can participate in desktop-theme lookup while asset variants provide the
/// deterministic fallback used by managed GUI windows and compositor-owned chrome.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppIconProfile {
    name: Option<String>,
    variants: Vec<AppIconVariant>,
}

impl AppIconProfile {
    pub const fn new() -> Self {
        Self {
            name: None,
            variants: Vec::new(),
        }
    }

    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Adds a scalable icon source.
    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        self.variants.push(AppIconVariant {
            icon: icon.into(),
            size: None,
        });
        self
    }

    /// Adds a source optimized for one logical square edge size.
    pub fn icon_at(mut self, size: u16, icon: impl Into<Icon>) -> Self {
        self.variants.push(AppIconVariant {
            icon: icon.into(),
            size: Some(size),
        });
        self
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn variants(&self) -> &[AppIconVariant] {
        &self.variants
    }

    pub fn preferred(&self, size: u16) -> Option<Icon> {
        self.variants
            .iter()
            .find(|variant| variant.size == Some(size))
            .or_else(|| self.variants.iter().find(|variant| variant.size.is_none()))
            .or_else(|| {
                self.variants.iter().min_by_key(|variant| {
                    variant.size.expect("sized variants remain").abs_diff(size)
                })
            })
            .map(|variant| variant.icon)
    }

    pub fn validate(&self) -> Result<(), AppIconProfileError> {
        if self
            .name
            .as_ref()
            .is_some_and(|name| name.trim().is_empty() || name.len() > 4_096 || name.contains('\0'))
        {
            return Err(AppIconProfileError::InvalidName);
        }
        if self.variants.len() > 32 {
            return Err(AppIconProfileError::TooManyVariants);
        }
        let mut sizes = std::collections::BTreeSet::new();
        for variant in &self.variants {
            if variant.size == Some(0) {
                return Err(AppIconProfileError::ZeroSize);
            }
            if !sizes.insert(variant.size) {
                return Err(AppIconProfileError::DuplicateSize(variant.size));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AppIconProfileError {
    #[error("application icon name is empty or invalid")]
    InvalidName,
    #[error("application icon profiles support at most 32 source variants")]
    TooManyVariants,
    #[error("application icon variant size must be positive")]
    ZeroSize,
    #[error("application icon profile contains duplicate size {0:?}")]
    DuplicateSize(Option<u16>),
}

/// Source accepted by image composition without exposing renderer resource allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImageSource {
    Registered(ImageId),
    Asset(ImageAsset),
    Tinted { image: ImageId, color: ColorRgba8 },
}

impl ImageSource {
    pub const fn image_id(self) -> ImageId {
        match self {
            Self::Registered(image) => image,
            Self::Asset(asset) => asset.image_id(),
            Self::Tinted { image, .. } => image,
        }
    }

    /// Recolors sampled artwork from its alpha mask. This is especially useful for monochrome SVG
    /// icons, while remaining deterministic for any registered image resource.
    pub const fn tint(self, color: ColorRgba8) -> Self {
        Self::Tinted {
            image: self.image_id(),
            color,
        }
    }

    pub const fn without_tint(self) -> Self {
        Self::Registered(self.image_id())
    }

    pub const fn tint_color(self) -> Option<ColorRgba8> {
        match self {
            Self::Tinted { color, .. } => Some(color),
            Self::Registered(_) | Self::Asset(_) => None,
        }
    }
}

impl From<ImageId> for ImageSource {
    fn from(value: ImageId) -> Self {
        Self::Registered(value)
    }
}

impl From<ImageAsset> for ImageSource {
    fn from(value: ImageAsset) -> Self {
        Self::Asset(value)
    }
}

impl From<IconAsset> for ImageSource {
    fn from(value: IconAsset) -> Self {
        Self::Registered(value.image_id())
    }
}

impl From<Icon> for ImageSource {
    fn from(value: Icon) -> Self {
        let source = Self::Registered(value.image_id());
        match value.tint_color() {
            Some(color) => source.tint(color),
            None => source,
        }
    }
}

/// Deterministic resource identity used to connect declarative asset references to render data.
pub const fn asset_image_id(key: AssetKey) -> ImageId {
    let bytes = key.as_str().as_bytes();
    let mut hash = 2_166_136_261_u32;
    let mut index = 0;
    while index < bytes.len() {
        hash ^= bytes[index] as u32;
        hash = hash.wrapping_mul(16_777_619);
        index += 1;
    }
    ImageId(hash | (1 << 31))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetError {
    NotFound(AssetKey),
    KindMismatch {
        key: AssetKey,
        expected: AssetKind,
        actual: AssetKind,
    },
}

impl fmt::Display for AssetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(key) => write!(formatter, "asset `{key}` was not registered"),
            Self::KindMismatch {
                key,
                expected,
                actual,
            } => write!(
                formatter,
                "asset `{key}` has kind {actual:?}, expected {expected:?}"
            ),
        }
    }
}

impl std::error::Error for AssetError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetCatalogError {
    EmptyKey,
    InvalidKey(AssetKey),
    DuplicateKey(AssetKey),
    MissingMediaType(AssetKey),
    EmptyAsset(AssetKey),
}

impl fmt::Display for AssetCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid asset catalog: {self:?}")
    }
}

impl std::error::Error for AssetCatalogError {}

fn validate_key(key: AssetKey) -> Result<(), AssetCatalogError> {
    let path = key.as_str();
    if path.is_empty() {
        return Err(AssetCatalogError::EmptyKey);
    }
    if path.starts_with('/')
        || path.ends_with('/')
        || path.contains('\\')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(AssetCatalogError::InvalidKey(key));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ICON: IconAsset = IconAsset::new(AssetKey::new("icons/app.svg"));
    static ENTRIES: [AssetEntry; 1] = [AssetEntry::embedded(
        ICON.key(),
        AssetKind::Icon,
        "image/svg+xml",
        b"<svg/>",
    )];

    #[test]
    fn typed_assets_resolve_only_their_declared_kind() {
        let bundle = AssetBundle::new(&ENTRIES).validate().unwrap();
        assert_eq!(ICON.resolve(bundle).unwrap().bytes, b"<svg/>");
        assert!(matches!(
            ImageAsset::new(ICON.key()).resolve(bundle),
            Err(AssetError::KindMismatch { .. })
        ));
    }

    #[test]
    fn bundles_reject_unsafe_keys() {
        static BAD: [AssetEntry; 1] = [AssetEntry::embedded(
            AssetKey::new("../secret.svg"),
            AssetKind::Icon,
            "image/svg+xml",
            b"x",
        )];
        assert!(matches!(
            AssetBundle::new(&BAD).validate(),
            Err(AssetCatalogError::InvalidKey(_))
        ));
    }

    #[test]
    fn application_icon_profiles_prefer_exact_then_scalable_then_nearest() {
        let exact = IconAsset::new(AssetKey::new("icons/exact.png"));
        let scalable = IconAsset::new(AssetKey::new("icons/scalable.svg"));
        let large = IconAsset::new(AssetKey::new("icons/large.png"));
        let profile = AppIconProfile::new()
            .icon_at(128, large)
            .icon(scalable)
            .icon_at(32, exact);

        assert_eq!(profile.preferred(32), Some(Icon::new(exact)));
        assert_eq!(profile.preferred(48), Some(Icon::new(scalable)));
        assert!(profile.validate().is_ok());

        let nearest = AppIconProfile::new().icon_at(16, exact).icon_at(128, large);
        assert_eq!(nearest.preferred(96), Some(Icon::new(large)));
    }

    #[test]
    fn application_icon_profiles_reject_invalid_metadata() {
        assert_eq!(
            AppIconProfile::new().named(" ").validate(),
            Err(AppIconProfileError::InvalidName)
        );
        assert_eq!(
            AppIconProfile::new().icon_at(0, ICON).validate(),
            Err(AppIconProfileError::ZeroSize)
        );
        assert_eq!(
            AppIconProfile::new()
                .icon_at(32, ICON)
                .icon_at(32, ICON)
                .validate(),
            Err(AppIconProfileError::DuplicateSize(Some(32)))
        );
    }

    #[test]
    fn icons_are_valid_image_sources() {
        assert_eq!(ImageSource::from(ICON).image_id(), ICON.image_id());
        assert_eq!(
            ImageSource::from(Icon::new(ICON)).image_id(),
            ICON.image_id()
        );

        let white = ColorRgba8::rgba(255, 255, 255, 255);
        let tinted = Icon::new(ICON).tint(white);
        assert_eq!(tinted.tint_color(), Some(white));
        assert_eq!(ImageSource::from(tinted).tint_color(), Some(white));
    }
}

#[cfg(test)]
pub(crate) use cursor_theme::{cursor_test_bundle, cursor_test_theme};

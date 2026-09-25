//! Bundled font families and file handles. Family names select typography; handles identify files.
use super::AssetEntry;
#[cfg(any(
    feature = "font-inter",
    feature = "font-instrument-sans",
    feature = "font-source-serif-4",
    feature = "font-jetbrains-mono",
    feature = "font-atkinson-hyperlegible-next"
))]
use super::{AssetKey, AssetKind, FontAsset};

pub const INTER_FAMILY: &str = "Inter";
#[cfg(feature = "font-inter")]
pub const INTER_ITALIC: FontAsset = FontAsset::new(AssetKey::new("telorgon/fonts/inter/italic"));
#[cfg(feature = "font-inter")]
pub const INTER: FontAsset = FontAsset::new(AssetKey::new("telorgon/fonts/inter/regular"));
pub const INSTRUMENT_SANS_FAMILY: &str = "Instrument Sans";
#[cfg(feature = "font-instrument-sans")]
pub const INSTRUMENT_SANS_ITALIC: FontAsset =
    FontAsset::new(AssetKey::new("telorgon/fonts/instrument-sans/italic"));
#[cfg(feature = "font-instrument-sans")]
pub const INSTRUMENT_SANS: FontAsset =
    FontAsset::new(AssetKey::new("telorgon/fonts/instrument-sans/regular"));
pub const SOURCE_SERIF_4_FAMILY: &str = "Source Serif 4";
#[cfg(feature = "font-source-serif-4")]
pub const SOURCE_SERIF_4_ITALIC: FontAsset =
    FontAsset::new(AssetKey::new("telorgon/fonts/source-serif-4/italic"));
#[cfg(feature = "font-source-serif-4")]
pub const SOURCE_SERIF_4: FontAsset =
    FontAsset::new(AssetKey::new("telorgon/fonts/source-serif-4/regular"));
pub const JETBRAINS_MONO_FAMILY: &str = "JetBrains Mono";
#[cfg(feature = "font-jetbrains-mono")]
pub const JETBRAINS_MONO_ITALIC: FontAsset =
    FontAsset::new(AssetKey::new("telorgon/fonts/jetbrains-mono/italic"));
#[cfg(feature = "font-jetbrains-mono")]
pub const JETBRAINS_MONO: FontAsset =
    FontAsset::new(AssetKey::new("telorgon/fonts/jetbrains-mono/regular"));
pub const ATKINSON_HYPERLEGIBLE_NEXT_FAMILY: &str = "Atkinson Hyperlegible Next";
#[cfg(feature = "font-atkinson-hyperlegible-next")]
pub const ATKINSON_HYPERLEGIBLE_NEXT_ITALIC: FontAsset = FontAsset::new(AssetKey::new(
    "telorgon/fonts/atkinson-hyperlegible-next/italic",
));
#[cfg(feature = "font-atkinson-hyperlegible-next")]
pub const ATKINSON_HYPERLEGIBLE_NEXT: FontAsset = FontAsset::new(AssetKey::new(
    "telorgon/fonts/atkinson-hyperlegible-next/regular",
));

pub(crate) static ENTRIES: &[AssetEntry] = &[
    #[cfg(feature = "font-inter")]
    AssetEntry::embedded(
        INTER_ITALIC.key(),
        AssetKind::Font,
        "font/ttf",
        include_bytes!("../../resources/fonts/inter/Inter-Italic[opsz,wght].ttf"),
    ),
    #[cfg(feature = "font-inter")]
    AssetEntry::embedded(
        INTER.key(),
        AssetKind::Font,
        "font/ttf",
        include_bytes!("../../resources/fonts/inter/Inter[opsz,wght].ttf"),
    ),
    #[cfg(feature = "font-instrument-sans")]
    AssetEntry::embedded(
        INSTRUMENT_SANS_ITALIC.key(),
        AssetKind::Font,
        "font/ttf",
        include_bytes!(
            "../../resources/fonts/instrument-sans/InstrumentSans-Italic[wdth,wght].ttf"
        ),
    ),
    #[cfg(feature = "font-instrument-sans")]
    AssetEntry::embedded(
        INSTRUMENT_SANS.key(),
        AssetKind::Font,
        "font/ttf",
        include_bytes!("../../resources/fonts/instrument-sans/InstrumentSans[wdth,wght].ttf"),
    ),
    #[cfg(feature = "font-source-serif-4")]
    AssetEntry::embedded(
        SOURCE_SERIF_4_ITALIC.key(),
        AssetKind::Font,
        "font/ttf",
        include_bytes!("../../resources/fonts/source-serif-4/SourceSerif4-Italic[opsz,wght].ttf"),
    ),
    #[cfg(feature = "font-source-serif-4")]
    AssetEntry::embedded(
        SOURCE_SERIF_4.key(),
        AssetKind::Font,
        "font/ttf",
        include_bytes!("../../resources/fonts/source-serif-4/SourceSerif4[opsz,wght].ttf"),
    ),
    #[cfg(feature = "font-jetbrains-mono")]
    AssetEntry::embedded(
        JETBRAINS_MONO_ITALIC.key(),
        AssetKind::Font,
        "font/ttf",
        include_bytes!("../../resources/fonts/jetbrains-mono/JetBrainsMono-Italic[wght].ttf"),
    ),
    #[cfg(feature = "font-jetbrains-mono")]
    AssetEntry::embedded(
        JETBRAINS_MONO.key(),
        AssetKind::Font,
        "font/ttf",
        include_bytes!("../../resources/fonts/jetbrains-mono/JetBrainsMono[wght].ttf"),
    ),
    #[cfg(feature = "font-atkinson-hyperlegible-next")]
    AssetEntry::embedded(
        ATKINSON_HYPERLEGIBLE_NEXT_ITALIC.key(),
        AssetKind::Font,
        "font/ttf",
        include_bytes!(
            "../../resources/fonts/atkinson-hyperlegible-next/AtkinsonHyperlegibleNext-Italic[wght].ttf"
        ),
    ),
    #[cfg(feature = "font-atkinson-hyperlegible-next")]
    AssetEntry::embedded(
        ATKINSON_HYPERLEGIBLE_NEXT.key(),
        AssetKind::Font,
        "font/ttf",
        include_bytes!(
            "../../resources/fonts/atkinson-hyperlegible-next/AtkinsonHyperlegibleNext[wght].ttf"
        ),
    ),
];

#[derive(Clone, Copy, Debug)]
pub struct FontLicense {
    pub family: &'static str,
    pub text: &'static str,
}

/// Original notices for redistribution with applications using the enabled fonts.
pub static LICENSES: &[FontLicense] = &[
    #[cfg(feature = "font-inter")]
    FontLicense {
        family: INTER_FAMILY,
        text: include_str!("../../resources/fonts/inter/OFL.txt"),
    },
    #[cfg(feature = "font-instrument-sans")]
    FontLicense {
        family: INSTRUMENT_SANS_FAMILY,
        text: include_str!("../../resources/fonts/instrument-sans/OFL.txt"),
    },
    #[cfg(feature = "font-source-serif-4")]
    FontLicense {
        family: SOURCE_SERIF_4_FAMILY,
        text: include_str!("../../resources/fonts/source-serif-4/OFL.txt"),
    },
    #[cfg(feature = "font-jetbrains-mono")]
    FontLicense {
        family: JETBRAINS_MONO_FAMILY,
        text: include_str!("../../resources/fonts/jetbrains-mono/OFL.txt"),
    },
    #[cfg(feature = "font-atkinson-hyperlegible-next")]
    FontLicense {
        family: ATKINSON_HYPERLEGIBLE_NEXT_FAMILY,
        text: include_str!("../../resources/fonts/atkinson-hyperlegible-next/OFL.txt"),
    },
];

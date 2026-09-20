use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::sync::Arc;

use crate::assets::{AssetBundle, AssetError, AssetKey, AssetKind, CursorAsset, CursorThemeAsset};
use crate::foundation::ColorRgba8;
use crate::platform::contracts::{
    MAX_CUSTOM_CURSOR_ANIMATION_DURATION_MS, MAX_CUSTOM_CURSOR_FRAME_DURATION_MS,
    MAX_CUSTOM_CURSOR_FRAMES, PointerIcon,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PointerHotspot {
    pub x: u16,
    pub y: u16,
}

impl PointerHotspot {
    pub const fn new(x: u16, y: u16) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PointerFrame {
    pub asset: CursorAsset,
    pub duration_ms: Option<NonZeroU32>,
}

impl PointerFrame {
    pub const fn still(asset: CursorAsset) -> Self {
        Self {
            asset,
            duration_ms: None,
        }
    }

    pub const fn animated(asset: CursorAsset, duration_ms: NonZeroU32) -> Self {
        Self {
            asset,
            duration_ms: Some(duration_ms),
        }
    }
}

/// One semantic pointer graphic. Size is optional; theme and output defaults can supply it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorGraphic {
    frames: Arc<[PointerFrame]>,
    hotspot: PointerHotspot,
    size: Option<u16>,
    logical_size_bits: Option<u32>,
    tint: Option<ColorRgba8>,
}

impl CursorGraphic {
    pub fn new(asset: CursorAsset) -> Self {
        Self {
            frames: Arc::from([PointerFrame::still(asset)]),
            hotspot: PointerHotspot::default(),
            size: None,
            logical_size_bits: None,
            tint: None,
        }
    }

    pub fn animated(frames: Vec<PointerFrame>) -> Result<Self, CursorThemeError> {
        if frames.len() < 2 || frames.len() > MAX_CUSTOM_CURSOR_FRAMES {
            return Err(CursorThemeError::InvalidFrameCount);
        }
        if frames.iter().any(|frame| frame.duration_ms.is_none()) {
            return Err(CursorThemeError::MissingFrameDuration);
        }
        if frames.iter().any(|frame| {
            frame
                .duration_ms
                .is_some_and(|duration| duration.get() > MAX_CUSTOM_CURSOR_FRAME_DURATION_MS)
        }) {
            return Err(CursorThemeError::FrameDurationTooLong);
        }
        let cycle_duration_ms = frames
            .iter()
            .filter_map(|frame| frame.duration_ms)
            .map(|duration| u64::from(duration.get()))
            .sum::<u64>();
        if cycle_duration_ms > MAX_CUSTOM_CURSOR_ANIMATION_DURATION_MS {
            return Err(CursorThemeError::AnimationDurationTooLong);
        }
        Ok(Self {
            frames: frames.into(),
            hotspot: PointerHotspot::default(),
            size: None,
            logical_size_bits: None,
            tint: None,
        })
    }

    pub const fn hotspot(mut self, x: u16, y: u16) -> Self {
        self.hotspot = PointerHotspot::new(x, y);
        self
    }

    /// Nominal size at 100%. The Linux desktop treats this as logical units and rasterizes
    /// at its output density; managed hosts retain their existing pixel-size behavior.
    pub fn size(mut self, logical_units: impl Into<f64>) -> Self {
        let logical_units = logical_units.into() as f32;
        self.logical_size_bits = Some(logical_units.to_bits());
        self.size = Some(logical_units.round() as u16);
        self
    }

    /// Recolors every frame from its decoded alpha mask while preserving transparency.
    pub const fn tint(mut self, color: ColorRgba8) -> Self {
        self.tint = Some(color);
        self
    }

    pub const fn without_tint(mut self) -> Self {
        self.tint = None;
        self
    }

    pub fn frames(&self) -> &[PointerFrame] {
        &self.frames
    }

    pub const fn pointer_hotspot(&self) -> PointerHotspot {
        self.hotspot
    }

    /// Nominal logical size at 100%. See [`Self::size`] for host interpretation.
    pub const fn logical_size(&self) -> Option<u16> {
        self.size
    }

    pub fn exact_logical_size(&self) -> Option<f32> {
        self.logical_size_bits
            .map(f32::from_bits)
            .or(self.size.map(f32::from))
    }

    pub const fn tint_color(&self) -> Option<ColorRgba8> {
        self.tint
    }
}

impl From<CursorAsset> for CursorGraphic {
    fn from(value: CursorAsset) -> Self {
        Self::new(value)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PointerThemeFallback {
    #[default]
    System,
    ThemeDefault,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PointerTheme {
    graphics: BTreeMap<PointerIcon, CursorGraphic>,
    default_size: Option<u16>,
    fallback: PointerThemeFallback,
}

impl PointerTheme {
    pub fn from_asset(
        asset: CursorThemeAsset,
        bundle: AssetBundle,
    ) -> Result<Self, CursorThemeError> {
        let entry = asset.resolve(bundle)?;
        let source = std::str::from_utf8(entry.bytes)
            .map_err(|_| CursorThemeError::ManifestNotUtf8(asset.key()))?;
        let value = source
            .parse::<toml::Table>()
            .map_err(|error| CursorThemeError::Manifest(error.to_string()))?;
        Self::from_table(&value, bundle)
    }

    pub fn set(mut self, icon: PointerIcon, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics.insert(icon, graphic.into());
        self
    }

    /// Default cursor size in logical units; managed hosts currently use a 1:1 pixel mapping.
    pub fn default_size(mut self, logical_units: u16) -> Self {
        self.default_size = (logical_units > 0).then_some(logical_units);
        self
    }

    pub const fn fallback(mut self, fallback: PointerThemeFallback) -> Self {
        self.fallback = fallback;
        self
    }

    pub fn graphic(&self, icon: PointerIcon) -> Option<&CursorGraphic> {
        self.graphics.get(&icon).or_else(|| {
            (self.fallback == PointerThemeFallback::ThemeDefault)
                .then(|| self.graphics.get(&PointerIcon::Default))
                .flatten()
        })
    }

    /// Default logical cursor size, used when a graphic does not specify its own size.
    pub const fn logical_size(&self) -> Option<u16> {
        self.default_size
    }

    fn from_table(table: &toml::Table, bundle: AssetBundle) -> Result<Self, CursorThemeError> {
        let mut theme = PointerTheme::default();
        if let Some(size) = table.get("size").and_then(toml::Value::as_integer) {
            theme.default_size = Some(valid_size(size)?);
        }
        if let Some(fallback) = table.get("fallback").and_then(toml::Value::as_str) {
            theme.fallback = match fallback {
                "system" => PointerThemeFallback::System,
                "default" => PointerThemeFallback::ThemeDefault,
                _ => return Err(CursorThemeError::InvalidFallback(fallback.to_owned())),
            };
        }
        for (name, value) in table {
            let Some(icon) = PointerIcon::from_name(name) else {
                if matches!(name.as_str(), "size" | "fallback") {
                    continue;
                }
                return Err(CursorThemeError::UnknownPointerIcon(name.clone()));
            };
            let definition = value
                .as_table()
                .ok_or_else(|| CursorThemeError::InvalidEntry(name.clone()))?;
            let graphic = parse_graphic(name, definition, bundle, theme.default_size)?;
            theme.graphics.insert(icon, graphic);
        }
        Ok(theme)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PointerThemeOverrides {
    graphics: BTreeMap<PointerIcon, CursorGraphic>,
}

impl PointerThemeOverrides {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(mut self, icon: PointerIcon, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics.insert(icon, graphic.into());
        self
    }

    pub fn pointer(self, graphic: impl Into<CursorGraphic>) -> Self {
        self.set(PointerIcon::Pointer, graphic)
    }

    pub fn text(self, graphic: impl Into<CursorGraphic>) -> Self {
        self.set(PointerIcon::Text, graphic)
    }

    pub fn default_pointer(self, graphic: impl Into<CursorGraphic>) -> Self {
        self.set(PointerIcon::Default, graphic)
    }

    pub fn graphic(&self, icon: PointerIcon) -> Option<&CursorGraphic> {
        self.graphics.get(&icon)
    }
}

/// Constructs a cursor graphic. Hotspots are in the source artwork's coordinates.
pub fn cursor(asset: CursorAsset) -> CursorGraphic {
    CursorGraphic::new(asset)
}

/// Required desktop cursor roles. Every role must be explicitly assigned.
pub const REQUIRED_CURSOR_ROLES: [PointerIcon; 36] = [
    PointerIcon::Default,
    PointerIcon::ContextMenu,
    PointerIcon::Help,
    PointerIcon::Pointer,
    PointerIcon::Progress,
    PointerIcon::Wait,
    PointerIcon::Cell,
    PointerIcon::Crosshair,
    PointerIcon::Text,
    PointerIcon::VerticalText,
    PointerIcon::Alias,
    PointerIcon::Copy,
    PointerIcon::Move,
    PointerIcon::NoDrop,
    PointerIcon::NotAllowed,
    PointerIcon::Grab,
    PointerIcon::Grabbing,
    PointerIcon::EResize,
    PointerIcon::NResize,
    PointerIcon::NeResize,
    PointerIcon::NwResize,
    PointerIcon::SResize,
    PointerIcon::SeResize,
    PointerIcon::SwResize,
    PointerIcon::WResize,
    PointerIcon::EwResize,
    PointerIcon::NsResize,
    PointerIcon::NeswResize,
    PointerIcon::NwseResize,
    PointerIcon::ColResize,
    PointerIcon::RowResize,
    PointerIcon::AllScroll,
    PointerIcon::ZoomIn,
    PointerIcon::ZoomOut,
    PointerIcon::DndAsk,
    PointerIcon::AllResize,
];

/// Complete desktop cursor design declaration, validated by the compositor at startup.
/// There is no implicit size or missing-role fallback. Compositor startup validates
/// every role and resolves the assets. Explicitly assigning the same graphic to several
/// roles is supported; no assignments are inferred.
#[derive(Clone, Debug, PartialEq)]
pub struct CursorTheme {
    graphics: BTreeMap<PointerIcon, CursorGraphic>,
    size: Option<f32>,
    effective_size: Option<f32>,
    asset: Option<CursorThemeAsset>,
}

impl CursorTheme {
    pub fn new() -> Self {
        Self {
            graphics: BTreeMap::new(),
            size: None,
            effective_size: None,
            asset: None,
        }
    }

    /// Declares a manifest to load and validate against the desktop asset catalog at startup.
    pub fn from_asset(asset: CursorThemeAsset) -> Self {
        Self {
            asset: Some(asset),
            ..Self::new()
        }
    }

    /// Base nominal size in logical units, required for code-defined themes.
    pub fn size(mut self, size: f32) -> Self {
        self.size = Some(size);
        self
    }

    /// Changes the effective base size, preserving per-role size proportions.
    pub fn cursor_size(mut self, size: f32) -> Self {
        self.effective_size = Some(size);
        self
    }

    pub fn default_pointer(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics.insert(PointerIcon::Default, graphic.into());
        self
    }
    pub fn context_menu(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics
            .insert(PointerIcon::ContextMenu, graphic.into());
        self
    }
    pub fn help(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics.insert(PointerIcon::Help, graphic.into());
        self
    }
    pub fn pointer(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics.insert(PointerIcon::Pointer, graphic.into());
        self
    }
    pub fn progress(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics.insert(PointerIcon::Progress, graphic.into());
        self
    }
    pub fn wait(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics.insert(PointerIcon::Wait, graphic.into());
        self
    }
    pub fn cell(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics.insert(PointerIcon::Cell, graphic.into());
        self
    }
    pub fn crosshair(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics.insert(PointerIcon::Crosshair, graphic.into());
        self
    }
    pub fn text(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics.insert(PointerIcon::Text, graphic.into());
        self
    }
    pub fn vertical_text(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics
            .insert(PointerIcon::VerticalText, graphic.into());
        self
    }
    pub fn alias(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics.insert(PointerIcon::Alias, graphic.into());
        self
    }
    pub fn copy(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics.insert(PointerIcon::Copy, graphic.into());
        self
    }
    pub fn move_cursor(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics.insert(PointerIcon::Move, graphic.into());
        self
    }
    pub fn no_drop(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics.insert(PointerIcon::NoDrop, graphic.into());
        self
    }
    pub fn not_allowed(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics
            .insert(PointerIcon::NotAllowed, graphic.into());
        self
    }
    pub fn grab(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics.insert(PointerIcon::Grab, graphic.into());
        self
    }
    pub fn grabbing(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics.insert(PointerIcon::Grabbing, graphic.into());
        self
    }
    pub fn e_resize(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics.insert(PointerIcon::EResize, graphic.into());
        self
    }
    pub fn n_resize(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics.insert(PointerIcon::NResize, graphic.into());
        self
    }
    pub fn ne_resize(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics.insert(PointerIcon::NeResize, graphic.into());
        self
    }
    pub fn nw_resize(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics.insert(PointerIcon::NwResize, graphic.into());
        self
    }
    pub fn s_resize(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics.insert(PointerIcon::SResize, graphic.into());
        self
    }
    pub fn se_resize(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics.insert(PointerIcon::SeResize, graphic.into());
        self
    }
    pub fn sw_resize(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics.insert(PointerIcon::SwResize, graphic.into());
        self
    }
    pub fn w_resize(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics.insert(PointerIcon::WResize, graphic.into());
        self
    }
    pub fn ew_resize(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics.insert(PointerIcon::EwResize, graphic.into());
        self
    }
    pub fn ns_resize(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics.insert(PointerIcon::NsResize, graphic.into());
        self
    }
    pub fn nesw_resize(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics
            .insert(PointerIcon::NeswResize, graphic.into());
        self
    }
    pub fn nwse_resize(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics
            .insert(PointerIcon::NwseResize, graphic.into());
        self
    }
    pub fn col_resize(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics.insert(PointerIcon::ColResize, graphic.into());
        self
    }
    pub fn row_resize(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics.insert(PointerIcon::RowResize, graphic.into());
        self
    }
    pub fn all_scroll(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics.insert(PointerIcon::AllScroll, graphic.into());
        self
    }
    pub fn zoom_in(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics.insert(PointerIcon::ZoomIn, graphic.into());
        self
    }
    pub fn zoom_out(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics.insert(PointerIcon::ZoomOut, graphic.into());
        self
    }
    pub fn dnd_ask(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics.insert(PointerIcon::DndAsk, graphic.into());
        self
    }
    pub fn all_resize(mut self, graphic: impl Into<CursorGraphic>) -> Self {
        self.graphics.insert(PointerIcon::AllResize, graphic.into());
        self
    }

    pub(crate) fn prepare(
        &self,
        assets: AssetBundle,
        mode: ClientCursorMode,
    ) -> Result<PointerConfiguration, CursorThemeError> {
        let mut graphics = BTreeMap::new();
        let mut base = self.size;
        if let Some(asset) = self.asset {
            let entry = asset.resolve(assets)?;
            let source = std::str::from_utf8(entry.bytes)
                .map_err(|_| CursorThemeError::ManifestNotUtf8(asset.key()))?;
            let table = source
                .parse::<toml::Table>()
                .map_err(|e| CursorThemeError::Manifest(e.to_string()))?;
            base = base.or_else(|| {
                table
                    .get("size")
                    .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|v| v as f64)))
                    .map(|v| v as f32)
            });
            for (name, value) in &table {
                if name == "size" {
                    continue;
                }
                let role = PointerIcon::from_name(name)
                    .ok_or_else(|| CursorThemeError::UnknownPointerIcon(name.clone()))?;
                let definition = value
                    .as_table()
                    .ok_or_else(|| CursorThemeError::InvalidEntry(name.clone()))?;
                graphics.insert(role, parse_graphic(name, definition, assets, None)?);
            }
        }
        graphics.extend(self.graphics.clone());
        let mut errors = Vec::new();
        for role in REQUIRED_CURSOR_ROLES {
            if !graphics.contains_key(&role) {
                errors.push(format!("missing cursor `{}`", role.name()));
            }
        }
        let valid_size = |v: f32| v.is_finite() && v > 0.0 && v <= u16::MAX as f32;
        if !base.is_some_and(valid_size) {
            errors.push(
                "cursor theme requires a finite positive logical size (at most 65535)".into(),
            );
        }
        if self.effective_size.is_some_and(|v| !valid_size(v)) {
            errors.push("effective cursor size must be finite and positive (at most 65535)".into());
        }
        if !errors.is_empty() {
            return Err(CursorThemeError::InvalidDesign(errors.join("; ")));
        }
        let base = base.unwrap();
        let effective = self.effective_size.unwrap_or(base);
        let mut media = crate::assets::AssetMediaCache::new(assets)
            .map_err(|e| CursorThemeError::InvalidDesign(e.to_string()))?;
        for (role, graphic) in &mut graphics {
            let size = graphic.exact_logical_size().unwrap_or(base) * effective / base;
            if !valid_size(size) {
                errors.push(format!(
                    "cursor `{}` has an invalid logical size",
                    role.name()
                ));
                continue;
            }
            let requested = crate::assets::AssetRasterSize::new(
                size.round().max(1.0) as u32,
                size.round().max(1.0) as u32,
            )
            .map_err(|e| CursorThemeError::InvalidDesign(e.to_string()))?;
            let mut source_extent = None;
            for frame in graphic.frames.iter() {
                let checked = (|| {
                    let source = media.cursor(frame.asset, None).map_err(|e| e.to_string())?;
                    if graphic.hotspot.x as i32 >= source.extent.width
                        || graphic.hotspot.y as i32 >= source.extent.height
                    {
                        return Err("hotspot is outside source artwork".to_owned());
                    }
                    if source_extent.is_some_and(|extent| extent != source.extent) {
                        return Err("animation frames must have equal source dimensions".into());
                    }
                    source_extent = Some(source.extent);
                    media
                        .cursor(frame.asset, Some(requested))
                        .map_err(|e| e.to_string())?;
                    Ok::<_, String>(())
                })();
                if let Err(e) = checked {
                    errors.push(format!(
                        "cursor `{}`, asset `{}`: {e}",
                        role.name(),
                        frame.asset.key()
                    ));
                }
            }
            if let Some(extent) = source_extent {
                graphic.hotspot.x = (graphic.hotspot.x as f32 * size / extent.width as f32)
                    .round()
                    .min(size.ceil() - 1.0) as u16;
                graphic.hotspot.y = (graphic.hotspot.y as f32 * size / extent.height as f32)
                    .round()
                    .min(size.ceil() - 1.0) as u16;
            }
            graphic.size = Some(size.round().max(1.0) as u16);
            graphic.logical_size_bits = Some(size.to_bits());
        }
        if !errors.is_empty() {
            return Err(CursorThemeError::InvalidDesign(errors.join("; ")));
        }
        Ok(PointerConfiguration::new()
            .client_mode(mode)
            .overrides(PointerThemeOverrides { graphics }))
    }
}

/// Focused control over whether hosted Wayland clients may provide pixel cursor surfaces.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ClientCursorMode {
    #[default]
    Allow,
    ThemeOnly,
}

/// Concise application-level pointer configuration; this is data, not a policy service.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PointerConfiguration {
    theme: Option<CursorThemeAsset>,
    overrides: PointerThemeOverrides,
    client_mode: ClientCursorMode,
}

impl PointerConfiguration {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cursor_theme(mut self, theme: CursorThemeAsset) -> Self {
        self.theme = Some(theme);
        self
    }

    pub fn overrides(mut self, overrides: PointerThemeOverrides) -> Self {
        self.overrides = overrides;
        self
    }

    pub const fn client_mode(mut self, client_mode: ClientCursorMode) -> Self {
        self.client_mode = client_mode;
        self
    }

    pub const fn theme(&self) -> Option<CursorThemeAsset> {
        self.theme
    }

    pub const fn pointer_overrides(&self) -> &PointerThemeOverrides {
        &self.overrides
    }

    pub const fn client_cursor_mode(&self) -> ClientCursorMode {
        self.client_mode
    }

    pub fn load_theme(
        &self,
        bundle: AssetBundle,
    ) -> Result<Option<PointerTheme>, CursorThemeError> {
        self.theme
            .map(|theme| PointerTheme::from_asset(theme, bundle))
            .transpose()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PointerRequest {
    Hidden,
    ClientSurface,
    Semantic(PointerIcon),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerResolution<'a> {
    Hidden,
    ClientSurface,
    Graphic(&'a CursorGraphic),
    System(PointerIcon),
}

/// Applies the fixed hidden → client → override → theme → system precedence.
pub fn resolve_pointer<'a>(
    request: PointerRequest,
    client_mode: ClientCursorMode,
    overrides: &'a PointerThemeOverrides,
    theme: Option<&'a PointerTheme>,
) -> PointerResolution<'a> {
    match request {
        PointerRequest::Hidden => PointerResolution::Hidden,
        PointerRequest::ClientSurface if client_mode == ClientCursorMode::Allow => {
            PointerResolution::ClientSurface
        }
        PointerRequest::ClientSurface => resolve_semantic(PointerIcon::Default, overrides, theme),
        PointerRequest::Semantic(icon) => resolve_semantic(icon, overrides, theme),
    }
}

fn resolve_semantic<'a>(
    icon: PointerIcon,
    overrides: &'a PointerThemeOverrides,
    theme: Option<&'a PointerTheme>,
) -> PointerResolution<'a> {
    overrides
        .graphic(icon)
        .or_else(|| theme.and_then(|theme| theme.graphic(icon)))
        .map_or(PointerResolution::System(icon), PointerResolution::Graphic)
}

fn parse_graphic(
    name: &str,
    definition: &toml::Table,
    bundle: AssetBundle,
    default_size: Option<u16>,
) -> Result<CursorGraphic, CursorThemeError> {
    let hotspot = definition
        .get("hotspot")
        .and_then(toml::Value::as_array)
        .map(|values| {
            if values.len() != 2 {
                return Err(CursorThemeError::InvalidHotspot(name.to_owned()));
            }
            Ok(PointerHotspot::new(
                valid_coordinate(values[0].as_integer(), name)?,
                valid_coordinate(values[1].as_integer(), name)?,
            ))
        })
        .transpose()?
        .unwrap_or_default();
    let size = match definition.get("size") {
        Some(value) => {
            let size = value
                .as_float()
                .or_else(|| value.as_integer().map(|v| v as f64))
                .ok_or(CursorThemeError::InvalidSize)?;
            if !size.is_finite() || size <= 0.0 || size > u16::MAX as f64 {
                return Err(CursorThemeError::InvalidSize);
            }
            Some(size)
        }
        None => default_size.map(f64::from),
    };
    let mut graphic = if let Some(frames) = definition.get("frames") {
        let frames = frames
            .as_array()
            .ok_or_else(|| CursorThemeError::InvalidEntry(name.to_owned()))?
            .iter()
            .map(|frame| {
                let frame = frame
                    .as_table()
                    .ok_or_else(|| CursorThemeError::InvalidEntry(name.to_owned()))?;
                let asset = cursor_asset(frame.get("asset"), name, bundle)?;
                let duration = frame
                    .get("duration_ms")
                    .and_then(toml::Value::as_integer)
                    .and_then(|value| u32::try_from(value).ok())
                    .and_then(NonZeroU32::new)
                    .ok_or(CursorThemeError::MissingFrameDuration)?;
                Ok(PointerFrame::animated(asset, duration))
            })
            .collect::<Result<Vec<_>, CursorThemeError>>()?;
        CursorGraphic::animated(frames)?
    } else {
        CursorGraphic::new(cursor_asset(definition.get("asset"), name, bundle)?)
    };
    graphic.hotspot = hotspot;
    if let Some(size) = size {
        graphic = graphic.size(size);
    }
    Ok(graphic)
}

fn cursor_asset(
    value: Option<&toml::Value>,
    name: &str,
    bundle: AssetBundle,
) -> Result<CursorAsset, CursorThemeError> {
    let path = value
        .and_then(toml::Value::as_str)
        .ok_or_else(|| CursorThemeError::MissingAsset(name.to_owned()))?;
    let entry = bundle
        .iter()
        .find(|entry| entry.key.as_str() == path)
        .ok_or_else(|| CursorThemeError::MissingRegisteredAsset(path.to_owned()))?;
    if entry.kind != AssetKind::Cursor {
        return Err(CursorThemeError::Asset(AssetError::KindMismatch {
            key: entry.key,
            expected: AssetKind::Cursor,
            actual: entry.kind,
        }));
    }
    Ok(CursorAsset::new(entry.key))
}

fn valid_coordinate(value: Option<i64>, name: &str) -> Result<u16, CursorThemeError> {
    value
        .and_then(|value| u16::try_from(value).ok())
        .ok_or_else(|| CursorThemeError::InvalidHotspot(name.to_owned()))
}

fn valid_size(value: i64) -> Result<u16, CursorThemeError> {
    let size = u16::try_from(value).map_err(|_| CursorThemeError::InvalidSize)?;
    if size == 0 {
        return Err(CursorThemeError::InvalidSize);
    }
    Ok(size)
}

#[derive(Debug, thiserror::Error)]
pub enum CursorThemeError {
    #[error("invalid cursor design: {0}")]
    InvalidDesign(String),
    #[error(transparent)]
    Asset(#[from] AssetError),
    #[error("cursor theme `{0}` is not UTF-8")]
    ManifestNotUtf8(AssetKey),
    #[error("invalid cursor theme manifest: {0}")]
    Manifest(String),
    #[error("unknown pointer icon `{0}`")]
    UnknownPointerIcon(String),
    #[error("invalid cursor entry `{0}`")]
    InvalidEntry(String),
    #[error("cursor entry `{0}` has no asset")]
    MissingAsset(String),
    #[error("cursor asset `{0}` is not registered")]
    MissingRegisteredAsset(String),
    #[error("cursor entry `{0}` has an invalid hotspot")]
    InvalidHotspot(String),
    #[error("cursor entry `{0}` has a hotspot outside its declared size")]
    HotspotOutOfBounds(String),
    #[error("cursor size must be finite, positive, and at most 65535")]
    InvalidSize,
    #[error("cursor animation must contain 2..={MAX_CUSTOM_CURSOR_FRAMES} frames")]
    InvalidFrameCount,
    #[error("each animated cursor frame requires a positive duration_ms")]
    MissingFrameDuration,
    #[error("cursor frame duration exceeds the hard limit")]
    FrameDurationTooLong,
    #[error("cursor animation cycle exceeds the hard limit")]
    AnimationDurationTooLong,
    #[error("cursor fallback must be `system` or `default`, got `{0}`")]
    InvalidFallback(String),
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) fn cursor_test_bundle() -> AssetBundle {
    use crate::assets::AssetEntry;
    static ENTRIES: &[AssetEntry] = &[AssetEntry::embedded(
        AssetKey::new("cursor.svg"), AssetKind::Cursor, "image/svg+xml",
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"><path d="M0 0h32v32H0z"/></svg>"#,
    )];
    AssetBundle::new(ENTRIES)
}

#[cfg(test)]
pub(crate) fn cursor_test_theme() -> CursorTheme {
    let graphic = cursor(CursorAsset::new(AssetKey::new("cursor.svg"))).hotspot(16, 16);
    CursorTheme::new()
        .size(24.0)
        .default_pointer(graphic.clone())
        .context_menu(graphic.clone())
        .help(graphic.clone())
        .pointer(graphic.clone())
        .progress(graphic.clone())
        .wait(graphic.clone())
        .cell(graphic.clone())
        .crosshair(graphic.clone())
        .text(graphic.clone())
        .vertical_text(graphic.clone())
        .alias(graphic.clone())
        .copy(graphic.clone())
        .move_cursor(graphic.clone())
        .no_drop(graphic.clone())
        .not_allowed(graphic.clone())
        .grab(graphic.clone())
        .grabbing(graphic.clone())
        .e_resize(graphic.clone())
        .n_resize(graphic.clone())
        .ne_resize(graphic.clone())
        .nw_resize(graphic.clone())
        .s_resize(graphic.clone())
        .se_resize(graphic.clone())
        .sw_resize(graphic.clone())
        .w_resize(graphic.clone())
        .ew_resize(graphic.clone())
        .ns_resize(graphic.clone())
        .nesw_resize(graphic.clone())
        .nwse_resize(graphic.clone())
        .col_resize(graphic.clone())
        .row_resize(graphic.clone())
        .all_scroll(graphic.clone())
        .zoom_in(graphic.clone())
        .zoom_out(graphic.clone())
        .dnd_ask(graphic.clone())
        .all_resize(graphic.clone())
}

#[cfg(test)]
mod complete_theme_tests;

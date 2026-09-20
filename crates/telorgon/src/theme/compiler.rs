//! Deterministic Theme v4 compilation against one domain catalog.

use std::collections::{BTreeMap, BTreeSet};

use crate::foundation::{ColorRgba8, EdgeInsets, PointF};
use crate::ui::{
    Background, Shadow, ShadowList, SizeRule, StylePropertyPatch, StyleSlotId, VariantAxisId,
    VariantValueId,
};

use crate::theme::{
    CompiledComponentStyle, CompiledSlotStyle, CompiledStateStyle, CompiledTheme, Easing,
    InteractionState, ShadowSource, SlotStyleSource, StylePropertyMask, ThemeCatalog,
    ThemeDiagnostic, ThemeError, ThemeResult, ThemeSource, TransitionSource, TransitionSpec,
    TypographySource, ValueSource,
};

impl CompiledTheme {
    pub fn compile(source: &ThemeSource, catalog: &ThemeCatalog) -> ThemeResult<Self> {
        if source.domain != catalog.domain() {
            return Err(ThemeError::new(format!(
                "theme domain `{}` does not match `{}` catalog",
                source.domain.name(),
                catalog.domain().name()
            )));
        }
        validate_source_catalog_membership(source, catalog)?;
        let mut tokens = TokenCompiler::new(source);
        tokens.validate_all()?;
        let mut styles = BTreeMap::new();
        let mut names = BTreeMap::new();
        for contract in catalog.contracts() {
            for style_name in contract.styles.keys() {
                let catalog_style = catalog
                    .catalog_style(&contract.component, style_name)
                    .expect("catalog style exists for every contract style");
                let authored = source
                    .components
                    .get(&contract.component)
                    .and_then(|styles| styles.get(style_name));
                let mut slots = catalog_style
                    .defaults
                    .iter()
                    .map(|(slot, patch)| {
                        (
                            *slot,
                            CompiledSlotStyle {
                                patch: *patch,
                                font_family: None,
                            },
                        )
                    })
                    .collect::<BTreeMap<_, _>>();
                let mut variants = BTreeMap::new();
                let mut states = BTreeMap::new();
                let mut transition = TransitionSpec::default();
                if let Some(authored) = authored {
                    overlay_slots(
                        &mut slots,
                        compile_slots(contract, &authored.slots, &mut tokens)?,
                    );
                    for (axis, values) in &authored.variants {
                        let accepted = contract.variant_axes.get(axis).ok_or_else(|| {
                            ThemeError::new(format!(
                                "component `{}` has no variant axis `{axis}`",
                                contract.component
                            ))
                        })?;
                        for (value, variant) in values {
                            if !accepted.contains(value) {
                                return Err(ThemeError::new(format!(
                                    "component `{}` variant `{axis}` has no value `{value}`",
                                    contract.component
                                )));
                            }
                            variants.insert(
                                (VariantAxisId::named(axis), VariantValueId::named(value)),
                                compile_slots(contract, &variant.slots, &mut tokens)?,
                            );
                        }
                    }
                    for (state_name, state) in &authored.states {
                        let state_id = InteractionState::parse(state_name).ok_or_else(|| {
                            ThemeError::new(format!("unknown interaction state `{state_name}`"))
                        })?;
                        if !contract.relevant_states.contains(state_id.flag()) {
                            return Err(ThemeError::new(format!(
                                "state `{state_name}` is not supported by component `{}`",
                                contract.component
                            )));
                        }
                        states.insert(
                            state_id,
                            CompiledStateStyle {
                                slots: compile_slots(contract, &state.slots, &mut tokens)?,
                                transition: state
                                    .transition
                                    .as_ref()
                                    .map(|source| compile_transition(source, &mut tokens))
                                    .transpose()?,
                            },
                        );
                    }
                    if let Some(source) = &authored.transition {
                        transition = compile_transition(source, &mut tokens)?;
                    }
                }
                let (controlled_slots, controlled_font_families) =
                    controlled_properties(&slots, &variants, &states);
                let id = catalog_style.id;
                names.insert(format!("{}.{}", contract.component, style_name), id);
                styles.insert(
                    id,
                    CompiledComponentStyle {
                        id,
                        slots,
                        variants,
                        states,
                        state_precedence: contract.state_precedence.clone(),
                        relevant_states: contract.relevant_states,
                        transition,
                        controlled_slots,
                        controlled_font_families,
                    },
                );
            }
        }
        let canonical = toml::to_string(source)
            .map_err(|error| ThemeError::new(format!("cannot canonicalize Theme v4: {error}")))?;
        Ok(Self {
            domain: source.domain,
            styles,
            names,
            diagnostics: Vec::<ThemeDiagnostic>::new(),
            fingerprint: stable_hash(canonical.as_bytes()),
        })
    }
}

fn controlled_properties(
    slots: &BTreeMap<StyleSlotId, CompiledSlotStyle>,
    variants: &BTreeMap<(VariantAxisId, VariantValueId), BTreeMap<StyleSlotId, CompiledSlotStyle>>,
    states: &BTreeMap<InteractionState, CompiledStateStyle>,
) -> (
    BTreeMap<StyleSlotId, StylePropertyPatch>,
    BTreeSet<StyleSlotId>,
) {
    let mut controlled = BTreeMap::<StyleSlotId, StylePropertyPatch>::new();
    let mut font_families = BTreeSet::new();
    let mut include = |slot: StyleSlotId, style: &CompiledSlotStyle| {
        controlled.entry(slot).or_default().overlay(style.patch);
        if style.font_family.is_some() {
            font_families.insert(slot);
        }
    };
    for (slot, style) in slots {
        include(*slot, style);
    }
    for variant in variants.values() {
        for (slot, style) in variant {
            include(*slot, style);
        }
    }
    for state in states.values() {
        for (slot, style) in &state.slots {
            include(*slot, style);
        }
    }
    (controlled, font_families)
}

fn validate_source_catalog_membership(
    source: &ThemeSource,
    catalog: &ThemeCatalog,
) -> ThemeResult<()> {
    for (component, styles) in &source.components {
        let contract = catalog.contract(component).ok_or_else(|| {
            ThemeError::new(format!(
                "unknown {} component `{component}`",
                source.domain.name()
            ))
        })?;
        for style in styles.keys() {
            if !contract.styles.contains_key(style) {
                return Err(ThemeError::new(format!(
                    "unknown style `{component}.{style}`"
                )));
            }
        }
    }
    Ok(())
}

fn compile_slots(
    contract: &crate::theme::ComponentStyleContract,
    source: &BTreeMap<String, SlotStyleSource>,
    tokens: &mut TokenCompiler<'_>,
) -> ThemeResult<BTreeMap<StyleSlotId, CompiledSlotStyle>> {
    let mut result = BTreeMap::new();
    for (slot_name, authored) in source {
        let slot = contract.slots.get(slot_name).ok_or_else(|| {
            ThemeError::new(format!(
                "component `{}` has no slot `{slot_name}`",
                contract.component
            ))
        })?;
        let required = authored_property_mask(authored);
        if !slot.properties.contains(required) {
            return Err(ThemeError::new(format!(
                "slot `{}.{slot_name}` contains unsupported properties",
                contract.component
            )));
        }
        result.insert(slot.id, compile_slot(authored, tokens)?);
    }
    Ok(result)
}

fn authored_property_mask(source: &SlotStyleSource) -> StylePropertyMask {
    let has_box = source.background.is_some()
        || source.border_color.is_some()
        || source.border_width.is_some()
        || source.outline_color.is_some()
        || source.outline_width.is_some()
        || source.outline_offset.is_some()
        || source.radius.is_some()
        || source.padding.is_some()
        || source.margin.is_some()
        || source.width.is_some()
        || source.height.is_some()
        || source.opacity.is_some()
        || source.shadows.is_some();
    let has_text = source.foreground.is_some() || source.typography.is_some();
    let has_transform = source.translation_x.is_some()
        || source.translation_y.is_some()
        || source.scale_x.is_some()
        || source.scale_y.is_some()
        || source.rotation.is_some()
        || source.origin_x.is_some()
        || source.origin_y.is_some();
    let mut bits = StylePropertyMask::default();
    if has_box {
        bits = combine(bits, StylePropertyMask::BOX);
    }
    if has_text {
        bits = combine(bits, StylePropertyMask::TEXT);
    }
    if has_transform {
        bits = combine(bits, StylePropertyMask::TRANSFORM);
    }
    bits
}

fn combine(left: StylePropertyMask, right: StylePropertyMask) -> StylePropertyMask {
    left.union(right)
}

fn compile_slot(
    source: &SlotStyleSource,
    tokens: &mut TokenCompiler<'_>,
) -> ThemeResult<CompiledSlotStyle> {
    let mut patch = StylePropertyPatch::default();
    if let Some(value) = &source.background {
        patch.background = Some(Background::Color(tokens.color(value)?));
    }
    if let Some(value) = &source.foreground {
        patch.text_color = Some(tokens.color(value)?);
    }
    if let Some(value) = &source.border_color {
        patch.border_color = Some(tokens.color(value)?);
    }
    if let Some(value) = &source.border_width {
        patch.border_width = Some(nonnegative(tokens.length(value)?, "border_width")?);
    }
    if let Some(value) = &source.outline_color {
        patch.outline_color = Some(tokens.color(value)?);
    }
    if let Some(value) = &source.outline_width {
        patch.outline_width = Some(nonnegative(tokens.length(value)?, "outline_width")?);
    }
    if let Some(value) = &source.outline_offset {
        patch.outline_offset = Some(finite(tokens.length(value)?, "outline_offset")?);
    }
    if let Some(value) = &source.radius {
        patch.radius = Some(nonnegative(tokens.length(value)?, "radius")?);
    }
    if let Some(value) = &source.padding {
        patch.padding = Some(EdgeInsets::all(nonnegative(
            tokens.length(value)?,
            "padding",
        )?));
    }
    if let Some(value) = &source.margin {
        patch.margin = Some(EdgeInsets::all(finite(tokens.length(value)?, "margin")?));
    }
    if let Some(value) = &source.width {
        patch.width = Some(SizeRule::Logical(nonnegative(
            tokens.length(value)?,
            "width",
        )?));
    }
    if let Some(value) = &source.height {
        patch.height = Some(SizeRule::Logical(nonnegative(
            tokens.length(value)?,
            "height",
        )?));
    }
    if let Some(value) = source.opacity {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(ThemeError::new(
                "opacity must be finite and between 0 and 1",
            ));
        }
        patch.opacity = Some(value);
    }
    if let Some(value) = &source.shadows {
        patch.shadows = Some(tokens.shadows(value)?);
    }
    if let Some(value) = &source.translation_x {
        patch.translation_x = Some(finite(tokens.length(value)?, "translation_x")?);
    }
    if let Some(value) = &source.translation_y {
        patch.translation_y = Some(finite(tokens.length(value)?, "translation_y")?);
    }
    patch.scale_x = source
        .scale_x
        .map(|value| nonnegative(value, "scale_x"))
        .transpose()?;
    patch.scale_y = source
        .scale_y
        .map(|value| nonnegative(value, "scale_y"))
        .transpose()?;
    patch.rotation = source
        .rotation
        .map(|value| finite(value, "rotation"))
        .transpose()?;
    patch.origin_x = source
        .origin_x
        .map(|value| finite(value, "origin_x"))
        .transpose()?;
    patch.origin_y = source
        .origin_y
        .map(|value| finite(value, "origin_y"))
        .transpose()?;
    let mut font_family = None;
    if let Some(value) = &source.typography {
        let typography = tokens.typography(value)?;
        apply_typography(&mut patch, &mut font_family, &typography, tokens)?;
    }
    Ok(CompiledSlotStyle { patch, font_family })
}

fn apply_typography(
    patch: &mut StylePropertyPatch,
    font_family: &mut Option<String>,
    source: &TypographySource,
    tokens: &mut TokenCompiler<'_>,
) -> ThemeResult<()> {
    if let Some(value) = &source.color {
        patch.text_color = Some(tokens.color(value)?);
    }
    if let Some(value) = &source.size {
        patch.text_size = Some(nonnegative(tokens.length(value)?, "typography.size")?);
    }
    if let Some(value) = &source.line_height {
        patch.text_line_height = Some(nonnegative(
            tokens.length(value)?,
            "typography.line_height",
        )?);
    }
    font_family.clone_from(&source.family);
    patch.text_weight = source.weight;
    Ok(())
}

fn compile_transition(
    source: &TransitionSource,
    tokens: &mut TokenCompiler<'_>,
) -> ThemeResult<TransitionSpec> {
    Ok(TransitionSpec {
        duration_ms: tokens.duration(&source.duration)?,
        easing: tokens.easing(&source.easing)?,
        repeat: source.repeat,
    })
}

fn overlay_slots(
    target: &mut BTreeMap<StyleSlotId, CompiledSlotStyle>,
    overlay: BTreeMap<StyleSlotId, CompiledSlotStyle>,
) {
    for (slot, style) in overlay {
        target.entry(slot).or_default().overlay(&style);
    }
}

struct TokenCompiler<'a> {
    source: &'a ThemeSource,
    colors: BTreeMap<String, ColorRgba8>,
    lengths: BTreeMap<String, f32>,
    durations: BTreeMap<String, u32>,
    easings: BTreeMap<String, Easing>,
    typography_values: BTreeMap<String, TypographySource>,
    shadow_values: BTreeMap<String, Vec<ShadowSource>>,
    visiting: BTreeSet<String>,
}

impl<'a> TokenCompiler<'a> {
    fn new(source: &'a ThemeSource) -> Self {
        Self {
            source,
            colors: BTreeMap::new(),
            lengths: BTreeMap::new(),
            durations: BTreeMap::new(),
            easings: BTreeMap::new(),
            typography_values: BTreeMap::new(),
            shadow_values: BTreeMap::new(),
            visiting: BTreeSet::new(),
        }
    }

    fn validate_all(&mut self) -> ThemeResult<()> {
        for name in self.source.tokens.color.keys() {
            self.color_token(name)?;
        }
        for name in self.source.tokens.length.keys() {
            self.length_token(name)?;
        }
        for name in self.source.tokens.duration.keys() {
            self.duration_token(name)?;
        }
        for name in self.source.tokens.easing.keys() {
            self.easing_token(name)?;
        }
        for name in self.source.tokens.typography.keys() {
            self.typography_token(name)?;
        }
        for name in self.source.tokens.shadow.keys() {
            self.shadow_token(name)?;
        }
        Ok(())
    }

    fn color(&mut self, value: &ValueSource<String>) -> ThemeResult<ColorRgba8> {
        match value {
            ValueSource::Literal(value) => parse_color(value),
            ValueSource::Token { token } => {
                let name = token_name(token, "color")?;
                self.color_token(name)
            }
        }
    }

    fn color_token(&mut self, name: &str) -> ThemeResult<ColorRgba8> {
        if let Some(value) = self.colors.get(name) {
            return Ok(*value);
        }
        let key = format!("color.{name}");
        self.enter(&key)?;
        let source = self
            .source
            .tokens
            .color
            .get(name)
            .cloned()
            .ok_or_else(|| ThemeError::new(format!("unknown color token `{key}`")))?;
        let value = self.color(&source);
        self.leave(&key);
        let value = value?;
        self.colors.insert(name.to_owned(), value);
        Ok(value)
    }

    fn length(&mut self, value: &ValueSource<f32>) -> ThemeResult<f32> {
        match value {
            ValueSource::Literal(value) => finite(*value, "length"),
            ValueSource::Token { token } => {
                let name = token_name(token, "length")?;
                self.length_token(name)
            }
        }
    }

    fn length_token(&mut self, name: &str) -> ThemeResult<f32> {
        if let Some(value) = self.lengths.get(name) {
            return Ok(*value);
        }
        let key = format!("length.{name}");
        self.enter(&key)?;
        let source = self
            .source
            .tokens
            .length
            .get(name)
            .cloned()
            .ok_or_else(|| ThemeError::new(format!("unknown length token `{key}`")))?;
        let value = self.length(&source);
        self.leave(&key);
        let value = value?;
        self.lengths.insert(name.to_owned(), value);
        Ok(value)
    }

    fn duration(&mut self, value: &ValueSource<u32>) -> ThemeResult<u32> {
        match value {
            ValueSource::Literal(value) => Ok(*value),
            ValueSource::Token { token } => {
                let name = token_name(token, "duration")?;
                self.duration_token(name)
            }
        }
    }

    fn duration_token(&mut self, name: &str) -> ThemeResult<u32> {
        if let Some(value) = self.durations.get(name) {
            return Ok(*value);
        }
        let key = format!("duration.{name}");
        self.enter(&key)?;
        let source = self
            .source
            .tokens
            .duration
            .get(name)
            .cloned()
            .ok_or_else(|| ThemeError::new(format!("unknown duration token `{key}`")))?;
        let value = self.duration(&source);
        self.leave(&key);
        let value = value?;
        self.durations.insert(name.to_owned(), value);
        Ok(value)
    }

    fn easing(&mut self, value: &ValueSource<String>) -> ThemeResult<Easing> {
        match value {
            ValueSource::Literal(value) => Easing::parse(value)
                .ok_or_else(|| ThemeError::new(format!("unknown easing `{value}`"))),
            ValueSource::Token { token } => {
                let name = token_name(token, "easing")?;
                self.easing_token(name)
            }
        }
    }

    fn easing_token(&mut self, name: &str) -> ThemeResult<Easing> {
        if let Some(value) = self.easings.get(name) {
            return Ok(*value);
        }
        let key = format!("easing.{name}");
        self.enter(&key)?;
        let source = self
            .source
            .tokens
            .easing
            .get(name)
            .cloned()
            .ok_or_else(|| ThemeError::new(format!("unknown easing token `{key}`")))?;
        let value = self.easing(&source);
        self.leave(&key);
        let value = value?;
        self.easings.insert(name.to_owned(), value);
        Ok(value)
    }

    fn typography(
        &mut self,
        value: &ValueSource<TypographySource>,
    ) -> ThemeResult<TypographySource> {
        match value {
            ValueSource::Literal(value) => Ok(value.clone()),
            ValueSource::Token { token } => {
                let name = token_name(token, "typography")?;
                self.typography_token(name)
            }
        }
    }

    fn typography_token(&mut self, name: &str) -> ThemeResult<TypographySource> {
        if let Some(value) = self.typography_values.get(name) {
            return Ok(value.clone());
        }
        let key = format!("typography.{name}");
        self.enter(&key)?;
        let source = self
            .source
            .tokens
            .typography
            .get(name)
            .cloned()
            .ok_or_else(|| ThemeError::new(format!("unknown typography token `{key}`")))?;
        let value = self.typography(&source);
        self.leave(&key);
        let value = value?;
        self.typography_values
            .insert(name.to_owned(), value.clone());
        Ok(value)
    }

    fn shadows(&mut self, value: &ValueSource<Vec<ShadowSource>>) -> ThemeResult<ShadowList> {
        let values = match value {
            ValueSource::Literal(value) => value.clone(),
            ValueSource::Token { token } => {
                let name = token_name(token, "shadow")?;
                self.shadow_token(name)?
            }
        };
        if values.len() > 2 {
            return Err(ThemeError::new("a style supports at most two shadows"));
        }
        let mut compiled = Vec::with_capacity(values.len());
        for source in &values {
            compiled.push(Shadow {
                offset: PointF {
                    x: finite(self.length(&source.x)?, "shadow.x")?,
                    y: finite(self.length(&source.y)?, "shadow.y")?,
                },
                blur: nonnegative(self.length(&source.blur)?, "shadow.blur")?,
                spread: finite(self.length(&source.spread)?, "shadow.spread")?,
                color: self.color(&source.color)?,
            });
        }
        Ok(match compiled.as_slice() {
            [] => ShadowList::default(),
            [first] => ShadowList::one(*first),
            [first, second] => ShadowList::two(*first, *second),
            _ => unreachable!(),
        })
    }

    fn shadow_token(&mut self, name: &str) -> ThemeResult<Vec<ShadowSource>> {
        if let Some(value) = self.shadow_values.get(name) {
            return Ok(value.clone());
        }
        let key = format!("shadow.{name}");
        self.enter(&key)?;
        let source = self
            .source
            .tokens
            .shadow
            .get(name)
            .cloned()
            .ok_or_else(|| ThemeError::new(format!("unknown shadow token `{key}`")))?;
        let value = match source {
            ValueSource::Literal(value) => Ok(value),
            ValueSource::Token { token } => {
                let target = token_name(&token, "shadow")?;
                self.shadow_token(target)
            }
        };
        self.leave(&key);
        let value = value?;
        self.shadow_values.insert(name.to_owned(), value.clone());
        Ok(value)
    }

    fn enter(&mut self, key: &str) -> ThemeResult<()> {
        if !self.visiting.insert(key.to_owned()) {
            return Err(ThemeError::new(format!("token cycle includes `{key}`")));
        }
        Ok(())
    }

    fn leave(&mut self, key: &str) {
        self.visiting.remove(key);
    }
}

fn token_name<'a>(reference: &'a str, expected: &str) -> ThemeResult<&'a str> {
    let parts = reference.split('.').collect::<Vec<_>>();
    if parts.len() != 2 {
        if matches!(parts.first().copied(), Some("application" | "shell")) {
            return Err(ThemeError::new(format!(
                "cross-domain token reference `{reference}` is not allowed"
            )));
        }
        return Err(ThemeError::new(format!(
            "token reference `{reference}` must be `category.name`"
        )));
    }
    if parts[0] != expected {
        return Err(ThemeError::new(format!(
            "token `{reference}` has type `{}` but `{expected}` is required",
            parts[0]
        )));
    }
    if parts[1].is_empty() {
        return Err(ThemeError::new("token name cannot be empty"));
    }
    Ok(parts[1])
}

fn parse_color(value: &str) -> ThemeResult<ColorRgba8> {
    let hex = value
        .strip_prefix('#')
        .ok_or_else(|| ThemeError::new(format!("color `{value}` must start with #")))?;
    let parse = |range: std::ops::Range<usize>| {
        u8::from_str_radix(&hex[range], 16)
            .map_err(|_| ThemeError::new(format!("invalid color `{value}`")))
    };
    match hex.len() {
        6 => Ok(ColorRgba8::rgba(
            parse(0..2)?,
            parse(2..4)?,
            parse(4..6)?,
            255,
        )),
        8 => Ok(ColorRgba8::rgba(
            parse(0..2)?,
            parse(2..4)?,
            parse(4..6)?,
            parse(6..8)?,
        )),
        _ => Err(ThemeError::new(format!(
            "color `{value}` must contain 6 or 8 hex digits"
        ))),
    }
}

fn finite(value: f32, property: &str) -> ThemeResult<f32> {
    value
        .is_finite()
        .then_some(value)
        .ok_or_else(|| ThemeError::new(format!("{property} must be finite")))
}

fn nonnegative(value: f32, property: &str) -> ThemeResult<f32> {
    let value = finite(value, property)?;
    (value >= 0.0)
        .then_some(value)
        .ok_or_else(|| ThemeError::new(format!("{property} cannot be negative")))
}

fn stable_hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{ComponentStyleContract, ThemeDomain};
    use crate::ui::{InteractionFlags, StyleSlotId};

    fn catalog() -> ThemeCatalog {
        let mut catalog = ThemeCatalog::new(ThemeDomain::Application);
        catalog
            .register(
                ComponentStyleContract::new("button")
                    .slot("container", StylePropertyMask::BOX)
                    .slot("label", StylePropertyMask::TEXT)
                    .style(
                        "filled",
                        [
                            ("container".to_owned(), StylePropertyPatch::default()),
                            ("label".to_owned(), StylePropertyPatch::default()),
                        ],
                    )
                    .states(
                        InteractionFlags::from_bits(u32::MAX),
                        [InteractionState::Hovered, InteractionState::Pressed],
                    ),
            )
            .unwrap();
        catalog
    }

    #[test]
    fn compiles_typed_tokens_slots_and_deterministic_state_precedence() {
        let source = ThemeSource::parse(
            r##"
format = "v4"
domain = "application"
[tokens.color]
accent = "#123456ff"
hover = "#234567ff"
[tokens.length]
radius = 8
[components.button.filled.slots.container]
background = { token = "color.accent" }
radius = { token = "length.radius" }
[components.button.filled.slots.label]
foreground = "#ffffffff"
[components.button.filled.states.hovered.slots.container]
background = { token = "color.hover" }
"##,
        )
        .unwrap();
        let theme = CompiledTheme::compile(&source, &catalog()).unwrap();
        let id = theme.style_id("button", "filled").unwrap();
        let resolved = theme
            .style(id)
            .unwrap()
            .resolve(&[], InteractionFlags::HOVERED);
        assert_eq!(
            resolved.slots[&StyleSlotId::named("container")]
                .patch
                .background,
            Some(Background::Color(ColorRgba8::rgba(0x23, 0x45, 0x67, 255)))
        );
    }

    #[test]
    fn rejects_unknown_contract_entries_wrong_token_types_cycles_and_cross_domain_refs() {
        for source in [
            "format='v4'\ndomain='application'\n[components.unknown.default]",
            "format='v4'\ndomain='application'\n[tokens.color]\na={token='length.x'}\n[tokens.length]\nx=1",
            "format='v4'\ndomain='application'\n[tokens.color]\na={token='color.b'}\nb={token='color.a'}",
            "format='v4'\ndomain='application'\n[tokens.color]\na={token='shell.color.a'}",
        ] {
            let source = ThemeSource::parse(source).unwrap();
            assert!(CompiledTheme::compile(&source, &catalog()).is_err());
        }
    }
}

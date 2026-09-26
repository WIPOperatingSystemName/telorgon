use crate::foundation::ColorRgba8;
use crate::theme::{
    CompiledComponentStyle, CompiledSlotStyle, CompiledStateStyle, InteractionState, TransitionSpec,
};
use crate::ui::{
    Background, Border, InteractionFlags, Outline, Shadow, ShadowList, StylePropertyPatch,
    StyleSlotId,
};
use std::{collections::BTreeMap, sync::Arc};

/// Composable changes to a button's hovered appearance. Later effects win per property.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HoverEffect {
    Background(ColorRgba8),
    Border(Border),
    BorderColor(ColorRgba8),
    Outline(Outline),
    Shadow(Shadow),
    /// Logical pixels above the resting translation; does not change layout.
    Lift(f32),
    /// Multiplier of the resting scale, around its existing transform origin.
    Scale(f32),
}

impl HoverEffect {
    pub(crate) fn property(self) -> u8 {
        match self {
            Self::Background(_) => 1,
            Self::Border(_) | Self::BorderColor(_) => 2,
            Self::Outline(_) => 4,
            Self::Shadow(_) => 8,
            Self::Lift(_) | Self::Scale(_) => 16,
        }
    }

    pub(crate) fn valid(self) -> bool {
        match self {
            Self::Lift(value) => value.is_finite(),
            Self::Scale(value) => value.is_finite() && value > 0.0,
            Self::Border(border) => [border.top, border.right, border.bottom, border.left]
                .iter()
                .all(|side| side.width.is_finite() && side.width >= 0.0),
            Self::Outline(outline) => {
                outline.width.is_finite() && outline.width >= 0.0 && outline.offset.is_finite()
            }
            Self::Shadow(shadow) => {
                shadow.offset.x.is_finite()
                    && shadow.offset.y.is_finite()
                    && shadow.blur.is_finite()
                    && shadow.blur >= 0.0
                    && shadow.spread.is_finite()
            }
            _ => true,
        }
    }
}

pub(super) fn compile(
    props: &super::button::ButtonElement,
    effects: &[HoverEffect],
    transition: TransitionSpec,
) -> Arc<CompiledComponentStyle> {
    let root = StyleSlotId::named("root");
    let mut style =
        props
            .inline_style
            .as_deref()
            .cloned()
            .unwrap_or_else(|| CompiledComponentStyle {
                id: props.style_id,
                slots: BTreeMap::new(),
                variants: BTreeMap::new(),
                states: BTreeMap::new(),
                state_precedence: Vec::new(),
                relevant_states: InteractionFlags::default(),
                transition,
                controlled_slots: BTreeMap::new(),
                controlled_font_families: Default::default(),
            });
    let mut resting = StylePropertyPatch::default();
    let mut target = StylePropertyPatch::default();
    let base = style
        .slots
        .get(&root)
        .map(|slot| slot.patch)
        .unwrap_or_default();
    let mut authored = base;
    authored.overlay(props.style_override);
    let mut transform = authored.transform.unwrap_or(props.style.transform);
    if let Some(x) = authored.translation_x {
        transform.translation.x = x;
    }
    if let Some(y) = authored.translation_y {
        transform.translation.y = y;
    }
    if let Some(x) = authored.scale_x {
        transform.scale.x = x;
    }
    if let Some(y) = authored.scale_y {
        transform.scale.y = y;
    }
    if let Some(rotation) = authored.rotation {
        transform.rotation = rotation;
    }
    if let Some(x) = authored.origin_x {
        transform.origin.x = x;
    }
    if let Some(y) = authored.origin_y {
        transform.origin.y = y;
    }
    let normal_transform = transform;
    let mut border = authored.border.unwrap_or(props.style.decoration.border);
    if let Some(color) = authored.border_color {
        border = recolor(border, color);
    }
    if let Some(width) = authored.border_width {
        border.top.width = width;
        border.right.width = width;
        border.bottom.width = width;
        border.left.width = width;
    }
    let normal_border = border;
    let mut normal_outline = authored.outline.unwrap_or(props.style.decoration.outline);
    if let Some(width) = authored.outline_width {
        normal_outline.width = width;
    }
    if let Some(offset) = authored.outline_offset {
        normal_outline.offset = offset;
    }
    if let Some(color) = authored.outline_color {
        normal_outline.color = color;
    }
    for effect in effects {
        match *effect {
            HoverEffect::Background(color) => {
                resting.background = Some(
                    authored
                        .background
                        .unwrap_or(props.style.decoration.background),
                );
                target.background = Some(Background::Color(color));
            }
            HoverEffect::Border(value) => {
                resting.border = Some(normal_border);
                border = value;
                target.border = Some(border);
            }
            HoverEffect::BorderColor(color) => {
                resting.border = Some(normal_border);
                border = recolor(border, color);
                target.border = Some(border);
            }
            HoverEffect::Outline(value) => {
                resting.outline = Some(normal_outline);
                target.outline = Some(value);
            }
            HoverEffect::Shadow(value) => {
                resting.shadows = Some(authored.shadows.unwrap_or(props.style.decoration.shadows));
                target.shadows = Some(ShadowList::one(value));
            }
            HoverEffect::Lift(value) => {
                resting.transform = Some(normal_transform);
                transform.translation.y = normal_transform.translation.y - value;
                target.transform = Some(transform);
            }
            HoverEffect::Scale(value) => {
                resting.transform = Some(normal_transform);
                transform.scale.x = normal_transform.scale.x * value;
                transform.scale.y = normal_transform.scale.y * value;
                target.transform = Some(transform);
            }
        }
    }
    style
        .slots
        .entry(root)
        .or_default()
        .patch
        .overlay(props.style_override);
    style.slots.entry(root).or_default().patch.overlay(resting);
    let hovered = style
        .states
        .entry(InteractionState::Hovered)
        .or_insert_with(CompiledStateStyle::default);
    hovered
        .slots
        .entry(root)
        .or_insert_with(CompiledSlotStyle::default)
        .patch
        .overlay(target);
    hovered.transition = Some(transition);
    // Explicit pressed/disabled declarations override hover, regardless of builder order.
    style
        .state_precedence
        .retain(|state| *state != InteractionState::Hovered);
    style.state_precedence.insert(0, InteractionState::Hovered);
    style.relevant_states = InteractionFlags::from_bits(
        style.relevant_states.bits()
            | InteractionFlags::HOVERED.bits()
            | InteractionFlags::DISABLED.bits(),
    );
    style.transition = transition;
    style
        .controlled_slots
        .entry(root)
        .or_default()
        .overlay(resting);
    Arc::new(style)
}

fn recolor(mut border: Border, color: ColorRgba8) -> Border {
    border.top.color = color;
    border.right.color = color;
    border.bottom.color = color;
    border.left.color = color;
    border
}

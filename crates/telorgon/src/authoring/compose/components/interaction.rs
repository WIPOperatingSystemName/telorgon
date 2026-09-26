use crate::foundation::ColorRgba8;
use crate::theme::{
    CompiledComponentStyle, CompiledSlotStyle, CompiledStateStyle, InteractionState, TransitionSpec,
};
use crate::ui::{
    Background, Border, InteractionFlags, Outline, Shadow, ShadowList, StylePropertyPatch,
    StyleSlotId,
};
use std::{collections::BTreeMap, sync::Arc};

/// Composable changes to a button's interaction appearance. Later effects win per property.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InteractionEffect {
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

impl InteractionEffect {
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

/// Compatibility name for existing hover declarations.
pub type HoverEffect = InteractionEffect;

pub(super) fn compile(
    props: &super::button::ButtonElement,
    hover: &[InteractionEffect],
    hover_transition: Option<TransitionSpec>,
    press: &[InteractionEffect],
    press_transition: Option<TransitionSpec>,
) -> Arc<CompiledComponentStyle> {
    let default = TransitionSpec {
        duration_ms: 120,
        ..Default::default()
    };
    let hover_spec = hover_transition.unwrap_or(default);
    let press_spec = press_transition.unwrap_or(default);
    let has_hover = !hover.is_empty() || hover_transition.is_some();
    let has_press = !press.is_empty() || press_transition.is_some();
    let mut style = if has_hover {
        compile_layer(
            props,
            props.inline_style.as_deref(),
            hover,
            hover_spec,
            InteractionState::Hovered,
        )
    } else {
        compile_layer(
            props,
            props.inline_style.as_deref(),
            press,
            press_spec,
            InteractionState::Pressed,
        )
    };
    if has_hover && has_press {
        style = compile_layer(
            props,
            Some(style.as_ref()),
            press,
            press_spec,
            InteractionState::Pressed,
        );
        Arc::make_mut(&mut style).transition = hover_spec;
    }
    style
}

fn compile_layer(
    props: &super::button::ButtonElement,
    base_style: Option<&CompiledComponentStyle>,
    effects: &[InteractionEffect],
    transition: TransitionSpec,
    state: InteractionState,
) -> Arc<CompiledComponentStyle> {
    let root = StyleSlotId::named("root");
    let mut style = base_style
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
            InteractionEffect::Background(color) => {
                resting.background = Some(
                    authored
                        .background
                        .unwrap_or(props.style.decoration.background),
                );
                target.background = Some(Background::Color(color));
            }
            InteractionEffect::Border(value) => {
                resting.border = Some(normal_border);
                border = value;
                target.border = Some(border);
                target.border_color = None;
            }
            InteractionEffect::BorderColor(color) => {
                resting.border = Some(normal_border);
                border = recolor(border, color);
                if state == InteractionState::Pressed {
                    target.border_color = Some(color);
                } else {
                    target.border = Some(border);
                }
            }
            InteractionEffect::Outline(value) => {
                resting.outline = Some(normal_outline);
                target.outline = Some(value);
            }
            InteractionEffect::Shadow(value) => {
                resting.shadows = Some(authored.shadows.unwrap_or(props.style.decoration.shadows));
                target.shadows = Some(ShadowList::one(value));
            }
            // Press patches only the affected transform axes so hover lift and
            // press scale compose without resetting each other.
            InteractionEffect::Lift(value) => {
                resting.transform = Some(normal_transform);
                transform.translation.y = normal_transform.translation.y - value;
                if state == InteractionState::Pressed {
                    target.translation_y = Some(transform.translation.y);
                } else {
                    target.transform = Some(transform);
                }
            }
            InteractionEffect::Scale(value) => {
                resting.transform = Some(normal_transform);
                transform.scale.x = normal_transform.scale.x * value;
                transform.scale.y = normal_transform.scale.y * value;
                if state == InteractionState::Pressed {
                    target.scale_x = Some(transform.scale.x);
                    target.scale_y = Some(transform.scale.y);
                } else {
                    target.transform = Some(transform);
                }
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
    let state_style = style
        .states
        .entry(state)
        .or_insert_with(CompiledStateStyle::default);
    state_style
        .slots
        .entry(root)
        .or_insert_with(CompiledSlotStyle::default)
        .patch
        .overlay(target);
    state_style.transition = Some(transition);
    style.state_precedence.retain(|value| *value != state);
    let index = if state == InteractionState::Pressed {
        usize::from(style.state_precedence.first() == Some(&InteractionState::Hovered))
    } else {
        0
    };
    style.state_precedence.insert(index, state);
    style.relevant_states = InteractionFlags::from_bits(
        style.relevant_states.bits() | state.flag().bits() | InteractionFlags::DISABLED.bits(),
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

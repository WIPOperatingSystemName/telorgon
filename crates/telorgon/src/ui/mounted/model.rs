use crate::foundation::{ColorRgba8, EdgeInsets, PointF, Transform2D};
use crate::graphics::scene::NodeId;
use std::{fmt, sync::Arc};

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StringId(pub u32);
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ImageId(pub u32);
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MaterialId(pub u32);
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StyleId(pub u32);
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ThemeScopeId {
    index: u32,
    generation: u32,
}

impl ThemeScopeId {
    pub const fn new(index: u32, generation: u32) -> Self {
        Self { index, generation }
    }

    pub const fn index(self) -> u32 {
        self.index
    }

    pub const fn generation(self) -> u32 {
        self.generation
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ThemeDomainId(pub u32);

impl ThemeDomainId {
    pub const APPLICATION: Self = Self(1);
    pub const SHELL: Self = Self(2);
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ComponentStyleId {
    pub domain: ThemeDomainId,
    pub component: u64,
    pub style: u64,
}

impl ComponentStyleId {
    pub const fn named(domain: ThemeDomainId, component: &str, style: &str) -> Self {
        Self {
            domain,
            component: stable_style_hash(component),
            style: stable_style_hash(style),
        }
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StyleSlotId(pub u64);

impl StyleSlotId {
    pub const fn named(name: &str) -> Self {
        Self(stable_style_hash(name))
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VariantAxisId(pub u64);

impl VariantAxisId {
    pub const fn named(name: &str) -> Self {
        Self(stable_style_hash(name))
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VariantValueId(pub u64);

impl VariantValueId {
    pub const fn named(name: &str) -> Self {
        Self(stable_style_hash(name))
    }
}

const fn stable_style_hash(value: &str) -> u64 {
    let bytes = value.as_bytes();
    let mut hash = 0xcbf29ce484222325_u64;
    let mut index = 0;
    while index < bytes.len() {
        hash ^= bytes[index] as u64;
        hash = hash.wrapping_mul(0x100000001b3);
        index += 1;
    }
    hash
}

/// Compact, fully resolved visual and semantic state for one mounted control.
///
/// Individual flags have explicit owners: pointer routing owns hover/press/drag, focus routing owns
/// focus and focus visibility, and component properties own semantic flags such as checked, busy,
/// and invalid. Render backends must never infer or mutate these flags.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct InteractionFlags(u32);
impl InteractionFlags {
    pub const HOVERED: Self = Self(1 << 0);
    pub const PRESSED: Self = Self(1 << 1);
    pub const FOCUSED: Self = Self(1 << 2);
    pub const FOCUS_VISIBLE: Self = Self(1 << 3);
    pub const DISABLED: Self = Self(1 << 4);
    pub const READ_ONLY: Self = Self(1 << 5);
    pub const BUSY: Self = Self(1 << 6);
    pub const CHECKED: Self = Self(1 << 7);
    pub const MIXED: Self = Self(1 << 8);
    pub const SELECTED: Self = Self(1 << 9);
    pub const EXPANDED: Self = Self(1 << 10);
    pub const ACTIVE: Self = Self(1 << 11);
    pub const HIGHLIGHTED: Self = Self(1 << 12);
    pub const DRAGGING: Self = Self(1 << 13);
    pub const SCROLLING: Self = Self(1 << 14);
    pub const INVALID: Self = Self(1 << 15);

    #[doc(hidden)]
    pub const ROUTER_OWNED: Self = Self(
        Self::HOVERED.0
            | Self::PRESSED.0
            | Self::FOCUSED.0
            | Self::FOCUS_VISIBLE.0
            | Self::DRAGGING.0
            | Self::SCROLLING.0,
    );

    pub const TRANSIENT: Self = Self(
        Self::HOVERED.0
            | Self::PRESSED.0
            | Self::DRAGGING.0
            | Self::SCROLLING.0
            | Self::HIGHLIGHTED.0,
    );

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    pub const fn contains(self, state: Self) -> bool {
        self.0 & state.0 != 0
    }

    pub const fn intersects(self, flags: Self) -> bool {
        self.0 & flags.0 != 0
    }

    pub fn set(&mut self, state: Self, enabled: bool) {
        if enabled {
            self.0 |= state.0
        } else {
            self.0 &= !state.0
        }
    }

    pub fn remove(&mut self, flags: Self) {
        self.0 &= !flags.0;
    }
}

/// Explicit default behavior registered by a mounted control.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum ControlBehavior {
    #[default]
    None,
    Activate,
    Value,
    TextInput,
    Scroll,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum BoxSizing {
    #[default]
    BorderBox,
    ContentBox,
}
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub enum SizeRule {
    /// Fixed size in logical layout units; the host applies output scaling when rendering.
    Logical(f32),
    Percent(f32),
    Fill(f32),
    #[default]
    Shrink,
}
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct SizeRule2D {
    pub width: SizeRule,
    pub height: SizeRule,
}
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub enum Background {
    #[default]
    None,
    Color(ColorRgba8),
}
impl From<ColorRgba8> for Background {
    fn from(value: ColorRgba8) -> Self {
        Self::Color(value)
    }
}
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct BorderSide {
    pub width: f32,
    pub color: ColorRgba8,
}
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Border {
    pub top: BorderSide,
    pub right: BorderSide,
    pub bottom: BorderSide,
    pub left: BorderSide,
}

/// Non-layout focus or validation ring painted outside a box's border edge.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Outline {
    pub width: f32,
    pub offset: f32,
    pub color: ColorRgba8,
}
impl Border {
    pub const fn all(width: f32, color: ColorRgba8) -> Self {
        let side = BorderSide { width, color };
        Self {
            top: side,
            right: side,
            bottom: side,
            left: side,
        }
    }
}
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct CornerRadii {
    pub top_left: f32,
    pub top_right: f32,
    pub bottom_right: f32,
    pub bottom_left: f32,
}
impl CornerRadii {
    pub const fn all(radius: f32) -> Self {
        Self {
            top_left: radius,
            top_right: radius,
            bottom_right: radius,
            bottom_left: radius,
        }
    }
}
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Shadow {
    pub offset: PointF,
    pub blur: f32,
    pub spread: f32,
    pub color: ColorRgba8,
}
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct ShadowList {
    items: [Shadow; 2],
    len: u8,
}
impl ShadowList {
    pub const fn one(shadow: Shadow) -> Self {
        Self {
            items: [
                shadow,
                Shadow {
                    offset: PointF { x: 0.0, y: 0.0 },
                    blur: 0.0,
                    spread: 0.0,
                    color: ColorRgba8::rgba(0, 0, 0, 0),
                },
            ],
            len: 1,
        }
    }
    pub const fn two(first: Shadow, second: Shadow) -> Self {
        Self {
            items: [first, second],
            len: 2,
        }
    }
    pub fn as_slice(&self) -> &[Shadow] {
        &self.items[..self.len as usize]
    }
}

/// Reusable paint-only appearance for one rectangular UI box.
///
/// Decoration deliberately excludes layout, clipping, opacity, and transforms so one value can be
/// shared by ordinary containers, controls, popups, and shell-owned window frames without carrying
/// placement policy with it.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct BoxDecoration {
    pub background: Background,
    pub border: Border,
    pub outline: Outline,
    pub corner_radii: CornerRadii,
    pub shadows: ShadowList,
}

impl BoxDecoration {
    pub const fn new() -> Self {
        Self {
            background: Background::None,
            border: Border {
                top: BorderSide {
                    width: 0.0,
                    color: ColorRgba8::rgba(0, 0, 0, 0),
                },
                right: BorderSide {
                    width: 0.0,
                    color: ColorRgba8::rgba(0, 0, 0, 0),
                },
                bottom: BorderSide {
                    width: 0.0,
                    color: ColorRgba8::rgba(0, 0, 0, 0),
                },
                left: BorderSide {
                    width: 0.0,
                    color: ColorRgba8::rgba(0, 0, 0, 0),
                },
            },
            outline: Outline {
                width: 0.0,
                offset: 0.0,
                color: ColorRgba8::rgba(0, 0, 0, 0),
            },
            corner_radii: CornerRadii {
                top_left: 0.0,
                top_right: 0.0,
                bottom_right: 0.0,
                bottom_left: 0.0,
            },
            shadows: ShadowList {
                items: [
                    Shadow {
                        offset: PointF { x: 0.0, y: 0.0 },
                        blur: 0.0,
                        spread: 0.0,
                        color: ColorRgba8::rgba(0, 0, 0, 0),
                    },
                    Shadow {
                        offset: PointF { x: 0.0, y: 0.0 },
                        blur: 0.0,
                        spread: 0.0,
                        color: ColorRgba8::rgba(0, 0, 0, 0),
                    },
                ],
                len: 0,
            },
        }
    }

    pub const fn background(mut self, background: Background) -> Self {
        self.background = background;
        self
    }

    pub const fn border(mut self, border: Border) -> Self {
        self.border = border;
        self
    }

    pub const fn outline(mut self, outline: Outline) -> Self {
        self.outline = outline;
        self
    }

    pub const fn corner_radii(mut self, corner_radii: CornerRadii) -> Self {
        self.corner_radii = corner_radii;
        self
    }

    pub const fn shadows(mut self, shadows: ShadowList) -> Self {
        self.shadows = shadows;
        self
    }

    pub const fn uniform_border(mut self, width: f32, color: ColorRgba8) -> Self {
        self.border = Border::all(width, color);
        self
    }

    pub const fn corner_radius(mut self, radius: f32) -> Self {
        self.corner_radii = CornerRadii::all(radius);
        self
    }

    pub const fn shadow(mut self, shadow: Shadow) -> Self {
        self.shadows = ShadowList::one(shadow);
        self
    }

    /// Checks that every metric is finite and every paint extent is nonnegative.
    pub fn validate(self) -> Result<Self, BoxDecorationError> {
        for width in [
            self.border.top.width,
            self.border.right.width,
            self.border.bottom.width,
            self.border.left.width,
        ] {
            if !width.is_finite() || width < 0.0 {
                return Err(BoxDecorationError::InvalidBorderWidth);
            }
        }
        if !self.outline.width.is_finite() || self.outline.width < 0.0 {
            return Err(BoxDecorationError::InvalidOutlineWidth);
        }
        if !self.outline.offset.is_finite() {
            return Err(BoxDecorationError::InvalidOutlineOffset);
        }
        for radius in [
            self.corner_radii.top_left,
            self.corner_radii.top_right,
            self.corner_radii.bottom_right,
            self.corner_radii.bottom_left,
        ] {
            if !radius.is_finite() || radius < 0.0 {
                return Err(BoxDecorationError::InvalidCornerRadius);
            }
        }
        for shadow in self.shadows.as_slice() {
            if !shadow.offset.x.is_finite() || !shadow.offset.y.is_finite() {
                return Err(BoxDecorationError::InvalidShadowOffset);
            }
            if !shadow.blur.is_finite() || shadow.blur < 0.0 {
                return Err(BoxDecorationError::InvalidShadowBlur);
            }
            if !shadow.spread.is_finite() || shadow.spread < 0.0 {
                return Err(BoxDecorationError::InvalidShadowSpread);
            }
        }
        Ok(self)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BoxDecorationError {
    InvalidBorderWidth,
    InvalidOutlineWidth,
    InvalidOutlineOffset,
    InvalidCornerRadius,
    InvalidShadowOffset,
    InvalidShadowBlur,
    InvalidShadowSpread,
}

impl fmt::Display for BoxDecorationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid box decoration: {self:?}")
    }
}

impl std::error::Error for BoxDecorationError {}
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum Overflow {
    #[default]
    Visible,
    Clip,
    Scroll,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BoxStyle {
    pub sizing: BoxSizing,
    pub width: SizeRule,
    pub height: SizeRule,
    pub min_size: SizeRule2D,
    pub max_size: SizeRule2D,
    pub margin: EdgeInsets,
    pub padding: EdgeInsets,
    pub decoration: BoxDecoration,
    pub overflow: Overflow,
    pub opacity: f32,
    pub transform: Transform2D,
}
impl Default for BoxStyle {
    fn default() -> Self {
        Self {
            sizing: BoxSizing::BorderBox,
            width: SizeRule::Shrink,
            height: SizeRule::Shrink,
            min_size: SizeRule2D::default(),
            max_size: SizeRule2D {
                width: SizeRule::Fill(1.0),
                height: SizeRule::Fill(1.0),
            },
            margin: EdgeInsets::ZERO,
            padding: EdgeInsets::ZERO,
            decoration: BoxDecoration::default(),
            overflow: Overflow::Visible,
            opacity: 1.0,
            transform: Transform2D::default(),
        }
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum Flow {
    Horizontal,
    #[default]
    Vertical,
    Overlay,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum MainAxisAlignment {
    #[default]
    Start,
    Center,
    End,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum CrossAxisAlignment {
    #[default]
    Start,
    Center,
    End,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct LayoutStyle {
    pub flow: Flow,
    pub main_axis_alignment: MainAxisAlignment,
    pub cross_axis_alignment: CrossAxisAlignment,
    pub gap: f32,
    pub contain: bool,
    pub scroll_offset: PointF,
}
impl Default for LayoutStyle {
    fn default() -> Self {
        Self {
            flow: Flow::Vertical,
            main_axis_alignment: MainAxisAlignment::Start,
            cross_axis_alignment: CrossAxisAlignment::Start,
            gap: 0.0,
            contain: false,
            scroll_offset: PointF::default(),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Box,
    Text,
    Image,
    Button,
    Toggle,
    TextInput,
    Slider,
    Scroll,
    Collection,
    Custom(u16),
}
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum TextAlign {
    #[default]
    Start,
    Center,
    End,
}
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct TextStyle {
    pub color: ColorRgba8,
    pub size: f32,
    pub line_height: f32,
    pub family: StringId,
    pub weight: u16,
    pub align: TextAlign,
}
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct TextVisual {
    pub content: StringId,
    pub style: TextStyle,
    pub revision: u64,
}
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ImageVisual {
    pub image: ImageId,
    pub tint: Option<ColorRgba8>,
    pub content_version: u64,
}
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct InteractionSnapshot {
    pub flags: InteractionFlags,
    /// Include descendants when the input router computes hover for this node.
    pub hover_within: bool,
    pub enabled: bool,
    pub visible: bool,
    pub focusable: bool,
    pub behavior: ControlBehavior,
    pub value: f32,
    pub listener_mask: u16,
    pub value_track: Option<NodeId>,
    pub value_axis: Option<ValueAxis>,
    pub revision: u64,
}
impl Default for InteractionSnapshot {
    fn default() -> Self {
        Self {
            flags: InteractionFlags::default(),
            hover_within: false,
            enabled: true,
            visible: true,
            focusable: false,
            behavior: ControlBehavior::None,
            value: 0.0,
            listener_mask: 0,
            value_track: None,
            value_axis: None,
            revision: 1,
        }
    }
}

impl InteractionSnapshot {
    pub fn set_flag(&mut self, flag: InteractionFlags, enabled: bool) -> bool {
        let before = self.flags;
        self.flags.set(flag, enabled);
        if self.flags == before {
            return false;
        }
        self.revision = self.revision.wrapping_add(1).max(1);
        true
    }

    pub fn set_enabled(&mut self, enabled: bool) -> bool {
        let before = (self.enabled, self.flags);
        self.enabled = enabled;
        self.flags.set(InteractionFlags::DISABLED, !enabled);
        if !enabled {
            self.flags.remove(InteractionFlags::TRANSIENT);
        }
        if before == (self.enabled, self.flags) {
            return false;
        }
        self.revision = self.revision.wrapping_add(1).max(1);
        true
    }
}

/// Spatial direction used by a normalized continuous-value control.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ValueAxis {
    Horizontal { inverted: bool },
    Vertical { inverted: bool },
}

/// Sparse caller-authored override. `Inherit` leaves the catalog/theme value untouched.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum StyleOverride<T> {
    #[default]
    Inherit,
    Value(T),
}

/// Sparse, backend-neutral slot properties used by theme bindings and custom components.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StylePropertyPatch {
    pub sizing: Option<BoxSizing>,
    pub width: Option<SizeRule>,
    pub height: Option<SizeRule>,
    pub min_size: Option<SizeRule2D>,
    pub max_size: Option<SizeRule2D>,
    pub margin: Option<EdgeInsets>,
    pub padding: Option<EdgeInsets>,
    pub background: Option<Background>,
    pub border: Option<Border>,
    pub border_width: Option<f32>,
    pub border_color: Option<ColorRgba8>,
    pub outline: Option<Outline>,
    pub outline_width: Option<f32>,
    pub outline_offset: Option<f32>,
    pub outline_color: Option<ColorRgba8>,
    pub corner_radii: Option<CornerRadii>,
    pub radius: Option<f32>,
    pub shadows: Option<ShadowList>,
    pub overflow: Option<Overflow>,
    pub opacity: Option<f32>,
    pub transform: Option<Transform2D>,
    pub translation_x: Option<f32>,
    pub translation_y: Option<f32>,
    pub scale_x: Option<f32>,
    pub scale_y: Option<f32>,
    pub rotation: Option<f32>,
    pub origin_x: Option<f32>,
    pub origin_y: Option<f32>,
    pub text_color: Option<ColorRgba8>,
    pub text_size: Option<f32>,
    pub text_line_height: Option<f32>,
    pub text_family: Option<StringId>,
    pub text_weight: Option<u16>,
    pub image_tint: Option<Option<ColorRgba8>>,
}

impl StylePropertyPatch {
    /// Overlays only authored properties from `other`.
    pub fn overlay(&mut self, other: Self) {
        macro_rules! overlay {
            ($($field:ident),+ $(,)?) => {$ (
                if other.$field.is_some() {
                    self.$field = other.$field;
                }
            )+ };
        }
        overlay!(
            sizing,
            width,
            height,
            min_size,
            max_size,
            margin,
            padding,
            background,
            border,
            border_width,
            border_color,
            outline,
            outline_width,
            outline_offset,
            outline_color,
            corner_radii,
            radius,
            shadows,
            overflow,
            opacity,
            transform,
            translation_x,
            translation_y,
            scale_x,
            scale_y,
            rotation,
            origin_x,
            origin_y,
            text_color,
            text_size,
            text_line_height,
            text_family,
            text_weight,
            image_tint,
        );
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StyleSlotBinding {
    pub slot: StyleSlotId,
    pub node: NodeId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StyleVariantSelection {
    pub axis: VariantAxisId,
    pub value: VariantValueId,
}

/// Mounted component-to-foundation-node style contract consumed by `ThemeRuntime`.
#[derive(Clone, Debug, PartialEq)]
pub struct StyleBinding {
    pub state_root: NodeId,
    pub scope: ThemeScopeId,
    pub component_style: ComponentStyleId,
    pub slots: Vec<StyleSlotBinding>,
    pub variants: Vec<StyleVariantSelection>,
    pub local_overrides: Vec<(StyleSlotId, StylePropertyPatch)>,
    pub local_style: Option<Arc<crate::theme::CompiledComponentStyle>>,
    pub theme_revision: u64,
    pub interaction_revision: u64,
}

impl StyleBinding {
    pub fn new(state_root: NodeId, scope: ThemeScopeId, component_style: ComponentStyleId) -> Self {
        Self {
            state_root,
            scope,
            component_style,
            slots: Vec::new(),
            variants: Vec::new(),
            local_overrides: Vec::new(),
            local_style: None,
            theme_revision: 0,
            interaction_revision: 0,
        }
    }

    pub fn slot(mut self, slot: StyleSlotId, node: NodeId) -> Self {
        self.slots.push(StyleSlotBinding { slot, node });
        self
    }

    pub fn variant(mut self, axis: VariantAxisId, value: VariantValueId) -> Self {
        self.variants.push(StyleVariantSelection { axis, value });
        self
    }

    pub fn local_override(mut self, slot: StyleSlotId, patch: StylePropertyPatch) -> Self {
        self.local_overrides.push((slot, patch));
        self
    }

    pub fn local_style(mut self, style: Arc<crate::theme::CompiledComponentStyle>) -> Self {
        self.local_style = Some(style);
        self
    }
}

//! Declarative mounting API and compact mounted UI components.
//!
//! Declarations execute once during mount. Runtime changes flow through typed properties and
//! coalesced transactions; no recursive widget tree exists in the active UI.

use std::{fmt, marker::PhantomData, sync::Arc};

use crate::core::{ColorRgba8, EdgeInsets, PointF, Transform2D};
pub use crate::input::EventPhase;
use crate::input::InputEvent;
use crate::scene::{DirtyFlags, NodeArena, NodeId, SparseSet};

use crate::ui::semantics::{
    SemanticCheckState, SemanticError, SemanticName, SemanticNode, SemanticRole, SemanticValue,
};

pub use crate::scene::NodeId as UiNodeId;

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

#[derive(Clone, Debug)]
pub struct MountedUi {
    pub nodes: NodeArena,
    pub kinds: SparseSet<NodeKind>,
    pub box_styles: SparseSet<BoxStyle>,
    pub layouts: SparseSet<LayoutStyle>,
    pub interactions: SparseSet<InteractionSnapshot>,
    pub texts: SparseSet<TextVisual>,
    pub images: SparseSet<ImageVisual>,
    pub semantics: SparseSet<SemanticNode>,
    pub window_chrome_roles: SparseSet<crate::window_chrome::WindowChromeRole>,
    pub window_chrome_hit_specs: SparseSet<crate::window_chrome::WindowChromeHitSpec>,
    pub pointer_requests: SparseSet<crate::assets::PointerRequest>,
    style_bindings: Vec<StyleBinding>,
    style_binding_head_by_state: SparseSet<usize>,
    style_binding_next: Vec<Option<usize>>,
    dirty_style_bindings: Vec<usize>,
    dirty_style_binding_marks: Vec<bool>,
    keys: SparseSet<Option<u64>>,
    strings: Vec<String>,
    dynamic_text_strings: SparseSet<StringId>,
    free_dynamic_strings: Vec<u32>,
    root: Option<UiRoot>,
    patch_log: Vec<Patch>,
    structural_log: Vec<StructuralCommand>,
    pub diagnostics: UiDiagnostics,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct UiMemoryReport {
    pub mounted_nodes: usize,
    pub scene_bytes: usize,
    pub component_bytes: usize,
    pub string_bytes: usize,
    pub scratch_bytes: usize,
}
impl UiMemoryReport {
    pub fn total_bytes(self) -> usize {
        self.scene_bytes + self.component_bytes + self.string_bytes + self.scratch_bytes
    }
}

impl Default for MountedUi {
    fn default() -> Self {
        Self {
            nodes: NodeArena::default(),
            kinds: SparseSet::default(),
            box_styles: SparseSet::default(),
            layouts: SparseSet::default(),
            interactions: SparseSet::default(),
            texts: SparseSet::default(),
            images: SparseSet::default(),
            semantics: SparseSet::default(),
            window_chrome_roles: SparseSet::default(),
            window_chrome_hit_specs: SparseSet::default(),
            pointer_requests: SparseSet::default(),
            style_bindings: Vec::new(),
            style_binding_head_by_state: SparseSet::default(),
            style_binding_next: Vec::new(),
            dirty_style_bindings: Vec::new(),
            dirty_style_binding_marks: Vec::new(),
            keys: SparseSet::default(),
            strings: vec![String::new(), "sans-serif".to_owned()],
            dynamic_text_strings: SparseSet::default(),
            free_dynamic_strings: Vec::new(),
            root: None,
            patch_log: Vec::with_capacity(32),
            structural_log: Vec::with_capacity(4),
            diagnostics: UiDiagnostics::default(),
        }
    }
}

impl MountedUi {
    pub fn root(&self) -> Option<UiRoot> {
        self.root
    }
    pub fn memory_report(&self) -> UiMemoryReport {
        UiMemoryReport {
            mounted_nodes: self.nodes.alive().len(),
            scene_bytes: self.nodes.allocated_bytes(),
            component_bytes: self.kinds.allocated_bytes()
                + self.box_styles.allocated_bytes()
                + self.layouts.allocated_bytes()
                + self.interactions.allocated_bytes()
                + self.texts.allocated_bytes()
                + self.images.allocated_bytes()
                + self.semantics.allocated_bytes()
                + self.window_chrome_roles.allocated_bytes()
                + self.window_chrome_hit_specs.allocated_bytes()
                + self.pointer_requests.allocated_bytes()
                + self
                    .semantics
                    .values()
                    .iter()
                    .map(SemanticNode::allocated_bytes)
                    .sum::<usize>()
                + self.keys.allocated_bytes()
                + self.style_binding_head_by_state.allocated_bytes()
                + self.style_bindings.capacity() * std::mem::size_of::<StyleBinding>()
                + self.style_binding_next.capacity() * std::mem::size_of::<Option<usize>>()
                + self.dirty_style_bindings.capacity() * std::mem::size_of::<usize>()
                + self.dirty_style_binding_marks.capacity() * std::mem::size_of::<bool>()
                + self.dynamic_text_strings.allocated_bytes(),
            string_bytes: self.strings.capacity() * std::mem::size_of::<String>()
                + self.strings.iter().map(String::capacity).sum::<usize>()
                + self.free_dynamic_strings.capacity() * std::mem::size_of::<u32>(),
            scratch_bytes: self.patch_log.capacity() * std::mem::size_of::<Patch>()
                + self.structural_log.capacity() * std::mem::size_of::<StructuralCommand>(),
        }
    }
    pub fn string(&self, id: StringId) -> Option<&str> {
        self.strings.get(id.0 as usize).map(String::as_str)
    }
    pub fn intern(&mut self, text: impl AsRef<str>) -> StringId {
        let text = text.as_ref();
        if let Some(index) = self.strings.iter().position(|candidate| candidate == text) {
            return StringId(index as u32);
        }
        let id = StringId(self.strings.len() as u32);
        self.strings.push(text.to_owned());
        id
    }
    pub fn transaction<R>(
        &mut self,
        update: impl FnOnce(&mut UiTransaction<'_>) -> R,
    ) -> (R, TransactionResult) {
        self.patch_log.clear();
        self.structural_log.clear();
        let result = {
            let mut tx = UiTransaction { ui: self };
            update(&mut tx)
        };
        let committed = self.commit();
        (result, committed)
    }
    pub fn remove(&mut self, node: NodeId) -> Vec<NodeId> {
        let removed = self.nodes.remove_subtree(node);
        if self.root.is_some_and(|root| removed.contains(&root.0)) {
            self.root = None;
        }
        for id in &removed {
            if let Some(string) = self.dynamic_text_strings.remove(*id) {
                if let Some(slot) = self.strings.get_mut(string.0 as usize) {
                    slot.clear();
                    self.free_dynamic_strings.push(string.0);
                }
            }
            self.kinds.remove(*id);
            self.box_styles.remove(*id);
            self.layouts.remove(*id);
            self.interactions.remove(*id);
            self.texts.remove(*id);
            self.images.remove(*id);
            self.semantics.remove(*id);
            self.window_chrome_roles.remove(*id);
            self.window_chrome_hit_specs.remove(*id);
            self.pointer_requests.remove(*id);
            self.keys.remove(*id);
        }
        self.style_bindings.retain(|binding| {
            !removed.contains(&binding.state_root)
                && !binding
                    .slots
                    .iter()
                    .any(|slot| removed.contains(&slot.node))
        });
        self.rebuild_style_binding_index();
        removed
    }

    fn allocate_dynamic_text(&mut self, node: NodeId, value: String) -> StringId {
        let id = if let Some(index) = self.free_dynamic_strings.pop() {
            self.strings[index as usize] = value;
            StringId(index)
        } else {
            let id = StringId(self.strings.len() as u32);
            self.strings.push(value);
            id
        };
        self.dynamic_text_strings.insert(node, id);
        id
    }

    /// Replaces one composition-owned text buffer without growing the global string table.
    pub fn set_dynamic_text(&mut self, node: NodeId, value: impl AsRef<str>) -> bool {
        let value = value.as_ref();
        let Some(previous) = self.texts.get(node).copied() else {
            return false;
        };
        let id = match self.dynamic_text_strings.get(node).copied() {
            Some(id) => id,
            None => self.allocate_dynamic_text(node, value.to_owned()),
        };
        let content_changed = self.strings.get_mut(id.0 as usize).is_some_and(|current| {
            if current == value {
                false
            } else {
                current.clear();
                current.push_str(value);
                true
            }
        });
        let id_changed = previous.content != id;
        if id_changed {
            if let Some(text) = self.texts.get_mut(node) {
                text.content = id;
            }
        }
        if !content_changed && !id_changed {
            return false;
        }
        if let Some(text) = self.texts.get_mut(node) {
            text.revision = text.revision.wrapping_add(1).max(1);
        }
        if let Some(semantic) = self.semantics.get_mut(node) {
            if matches!(semantic.name, SemanticName::Text(_)) {
                semantic.name = SemanticName::Text(id);
            }
            if matches!(semantic.value, SemanticValue::Text(_)) {
                semantic.value = SemanticValue::Text(id);
            }
        }
        if let Some(core) = self.nodes.core_mut(node) {
            core.content_revision = core.content_revision.wrapping_add(1).max(1);
            core.semantic_revision = core.semantic_revision.wrapping_add(1).max(1);
        }
        self.nodes.mark_dirty(
            node,
            DirtyFlags::TEXT | DirtyFlags::MEASURE | DirtyFlags::PAINT | DirtyFlags::SEMANTICS,
        );
        true
    }

    /// Rebinds an image node to a retained image resource revision.
    pub fn set_image_visual(&mut self, node: NodeId, image: ImageId, content_version: u64) -> bool {
        let tint = self.images.get(node).and_then(|visual| visual.tint);
        self.set_image_visual_tinted(node, image, content_version, tint)
    }

    /// Rebinds an image and its optional alpha-mask tint as one paint update.
    pub fn set_image_visual_tinted(
        &mut self,
        node: NodeId,
        image: ImageId,
        content_version: u64,
        tint: Option<ColorRgba8>,
    ) -> bool {
        let Some(visual) = self.images.get_mut(node) else {
            return false;
        };
        let content_version = content_version.max(1);
        if visual.image == image && visual.content_version == content_version && visual.tint == tint
        {
            return false;
        }
        visual.image = image;
        visual.content_version = content_version;
        visual.tint = tint;
        if let Some(core) = self.nodes.core_mut(node) {
            core.content_revision = core.content_revision.wrapping_add(1).max(1);
        }
        self.nodes.mark_dirty(node, DirtyFlags::PAINT);
        true
    }

    /// Replaces one authored box style while retaining the node and its interaction state.
    pub fn set_box_style(&mut self, node: NodeId, style: BoxStyle) -> bool {
        if !self.nodes.contains(node) {
            return false;
        }
        let current = self.box_styles.get(node).copied().unwrap_or_default();
        if current == style {
            return false;
        }
        if style == BoxStyle::default() {
            self.box_styles.remove(node);
        } else {
            self.box_styles.insert(node, style);
        }
        if let Some(core) = self.nodes.core_mut(node) {
            core.style_revision = core.style_revision.wrapping_add(1).max(1);
        }
        self.nodes.mark_dirty(
            node,
            DirtyFlags::STYLE
                | DirtyFlags::LAYOUT
                | DirtyFlags::SPATIAL
                | DirtyFlags::CLIP
                | DirtyFlags::PAINT,
        );
        true
    }

    /// Replaces authored layout inputs without reconstructing the retained node.
    pub fn set_layout_style(&mut self, node: NodeId, layout: LayoutStyle) -> bool {
        if !self.nodes.contains(node) {
            return false;
        }
        let current = self.layouts.get(node).copied().unwrap_or_default();
        if current == layout {
            return false;
        }
        if layout == LayoutStyle::default() {
            self.layouts.remove(node);
        } else {
            self.layouts.insert(node, layout);
        }
        self.nodes.mark_dirty(
            node,
            DirtyFlags::LAYOUT | DirtyFlags::SPATIAL | DirtyFlags::CLIP | DirtyFlags::PAINT,
        );
        true
    }

    /// Replaces retained text styling while preserving content storage and semantic identity.
    pub fn set_text_style(&mut self, node: NodeId, style: TextStyle) -> bool {
        let Some(text) = self.texts.get_mut(node) else {
            return false;
        };
        if text.style == style {
            return false;
        }
        text.style = style;
        text.revision = text.revision.wrapping_add(1).max(1);
        if let Some(core) = self.nodes.core_mut(node) {
            core.content_revision = core.content_revision.wrapping_add(1).max(1);
        }
        self.nodes.mark_dirty(
            node,
            DirtyFlags::TEXT | DirtyFlags::MEASURE | DirtyFlags::PAINT,
        );
        true
    }

    pub fn style_bindings(&self) -> &[StyleBinding] {
        &self.style_bindings
    }

    /// Registers one complete binding only when its state root and every named slot are live.
    pub fn register_style_binding(&mut self, binding: StyleBinding) -> bool {
        let state_root = binding.state_root;
        if !self.nodes.contains(binding.state_root)
            || binding.slots.is_empty()
            || binding
                .slots
                .iter()
                .any(|slot| !self.nodes.contains(slot.node))
        {
            return false;
        }
        if self.style_bindings.contains(&binding) {
            return false;
        }
        // Promote the foundation root binding to the complete component contract.
        // Keeping both creates competing tracks for the same root slot, and later
        // reconciliation updates only the first binding, leaving stale inline state.
        let root_slot = StyleSlotBinding {
            slot: StyleSlotId::named("root"),
            node: state_root,
        };
        if binding.slots.contains(&root_slot)
            && let Some(index) = self.style_bindings.iter().position(|existing| {
                existing.state_root == state_root
                    && existing.scope == binding.scope
                    && existing.component_style == binding.component_style
                    && existing.slots.as_slice() == [root_slot]
                    && existing.local_style.is_none()
                    && existing.local_overrides.is_empty()
                    && existing.variants.is_empty()
            })
        {
            self.style_bindings[index] = binding;
            self.enqueue_style_binding(index);
            if let Some(core) = self.nodes.core_mut(state_root) {
                core.style_revision = core.style_revision.wrapping_add(1).max(1);
            }
            return true;
        }
        let index = self.style_bindings.len();
        let next = self.style_binding_head_by_state.get(state_root).copied();
        self.style_bindings.push(binding);
        self.style_binding_next.push(next);
        self.style_binding_head_by_state.insert(state_root, index);
        self.enqueue_style_binding(index);
        if let Some(core) = self.nodes.core_mut(state_root) {
            core.style_revision = core.style_revision.wrapping_add(1).max(1);
        }
        true
    }

    /// Selects a stable style for the automatically registered root binding of a component.
    pub fn set_style_id(&mut self, node: NodeId, style: ComponentStyleId) -> bool {
        let Some(index) = self.style_bindings.iter().position(|binding| {
            binding.state_root == node
                && binding
                    .slots
                    .iter()
                    .any(|slot| slot.node == node && slot.slot == StyleSlotId::named("root"))
        }) else {
            return false;
        };
        if self.style_bindings[index].component_style == style {
            return false;
        }
        let binding = &mut self.style_bindings[index];
        binding.component_style = style;
        binding.theme_revision = 0;
        self.enqueue_style_binding(index);
        if let Some(core) = self.nodes.core_mut(node) {
            core.style_revision = core.style_revision.wrapping_add(1).max(1);
        }
        self.nodes
            .mark_dirty(node, DirtyFlags::STYLE | DirtyFlags::PAINT);
        true
    }

    /// Installs one sparse local slot override without bypassing theme state resolution.
    pub fn set_style_override(
        &mut self,
        node: NodeId,
        slot: StyleSlotId,
        patch: StylePropertyPatch,
    ) -> bool {
        let Some(index) = self.style_bindings.iter().position(|binding| {
            binding.state_root == node
                && binding.slots.iter().any(|candidate| candidate.node == node)
        }) else {
            return false;
        };
        let binding = &mut self.style_bindings[index];
        if let Some((_, existing)) = binding
            .local_overrides
            .iter_mut()
            .find(|(candidate, _)| *candidate == slot)
        {
            if *existing == patch {
                return false;
            }
            *existing = patch;
        } else {
            binding.local_overrides.push((slot, patch));
        }
        binding.theme_revision = 0;
        self.enqueue_style_binding(index);
        if let Some(core) = self.nodes.core_mut(node) {
            core.style_revision = core.style_revision.wrapping_add(1).max(1);
        }
        self.nodes
            .mark_dirty(node, DirtyFlags::STYLE | DirtyFlags::PAINT);
        true
    }

    /// Replaces an optional code-defined component style on an existing binding.
    #[doc(hidden)]
    pub fn set_local_component_style(
        &mut self,
        node: NodeId,
        style: Option<Arc<crate::theme::CompiledComponentStyle>>,
    ) -> bool {
        let Some(index) = self.style_bindings.iter().position(|binding| {
            binding.state_root == node
                && binding
                    .slots
                    .iter()
                    .any(|slot| slot.node == node && slot.slot == StyleSlotId::named("root"))
        }) else {
            return false;
        };
        if self.style_bindings[index].local_style == style {
            return false;
        }
        self.style_bindings[index].local_style = style;
        self.style_bindings[index].theme_revision = 0;
        self.enqueue_style_binding(index);
        if let Some(core) = self.nodes.core_mut(node) {
            core.style_revision = core.style_revision.wrapping_add(1).max(1);
        }
        self.nodes
            .mark_dirty(node, DirtyFlags::STYLE | DirtyFlags::PAINT);
        true
    }

    /// Requalifies automatically mounted foundation bindings for an application or shell view.
    pub fn set_theme_domain(&mut self, domain: ThemeDomainId, scope: ThemeScopeId) {
        for (index, binding) in self.style_bindings.iter_mut().enumerate() {
            binding.scope = scope;
            binding.component_style.domain = domain;
            binding.theme_revision = 0;
            if self.dirty_style_binding_marks.len() <= index {
                self.dirty_style_binding_marks.resize(index + 1, false);
            }
            if !self.dirty_style_binding_marks[index] {
                self.dirty_style_binding_marks[index] = true;
                self.dirty_style_bindings.push(index);
            }
        }
        let nodes = self.nodes.alive().to_vec();
        for node in nodes {
            self.nodes
                .mark_dirty(node, DirtyFlags::STYLE | DirtyFlags::PAINT);
        }
    }

    fn enqueue_style_binding(&mut self, index: usize) {
        if self.dirty_style_binding_marks.len() <= index {
            self.dirty_style_binding_marks.resize(index + 1, false);
        }
        if !self.dirty_style_binding_marks[index] {
            self.dirty_style_binding_marks[index] = true;
            self.dirty_style_bindings.push(index);
        }
    }

    fn enqueue_style_bindings_for_state(&mut self, state_root: NodeId) {
        let mut cursor = self.style_binding_head_by_state.get(state_root).copied();
        while let Some(index) = cursor {
            self.enqueue_style_binding(index);
            cursor = self.style_binding_next.get(index).copied().flatten();
        }
    }

    fn rebuild_style_binding_index(&mut self) {
        self.style_binding_head_by_state = SparseSet::default();
        self.style_binding_next.clear();
        self.dirty_style_bindings.clear();
        self.dirty_style_binding_marks.clear();
        for (index, binding) in self.style_bindings.iter().enumerate() {
            let next = self
                .style_binding_head_by_state
                .get(binding.state_root)
                .copied();
            self.style_binding_next.push(next);
            self.style_binding_head_by_state
                .insert(binding.state_root, index);
            self.dirty_style_bindings.push(index);
            self.dirty_style_binding_marks.push(true);
        }
    }

    #[doc(hidden)]
    pub fn take_style_bindings_for_processing(&mut self) -> Vec<StyleBinding> {
        std::mem::take(&mut self.style_bindings)
    }

    #[doc(hidden)]
    pub fn restore_style_bindings_after_processing(&mut self, bindings: Vec<StyleBinding>) {
        debug_assert!(self.style_bindings.is_empty());
        self.style_bindings = bindings;
    }

    #[doc(hidden)]
    pub fn swap_dirty_style_bindings(&mut self, scratch: &mut Vec<usize>) {
        std::mem::swap(&mut self.dirty_style_bindings, scratch);
        for &index in scratch.iter() {
            if let Some(mark) = self.dirty_style_binding_marks.get_mut(index) {
                *mark = false;
            }
        }
    }

    /// Applies one already-resolved slot patch as a single dirty-state publication.
    pub fn apply_style_patch(&mut self, node: NodeId, patch: StylePropertyPatch) -> bool {
        if !self.nodes.contains(node) {
            return false;
        }
        let style = self.box_styles.get(node).copied().unwrap_or_default();
        let mut next = style;
        macro_rules! assign {
            ($($field:ident),+ $(,)?) => {$ (
                if let Some(value) = patch.$field {
                    next.$field = value;
                }
            )+ };
        }
        assign!(
            sizing, width, height, min_size, max_size, margin, padding, overflow, opacity,
            transform,
        );
        if let Some(value) = patch.background {
            next.decoration.background = value;
        }
        if let Some(value) = patch.border {
            next.decoration.border = value;
        }
        if let Some(value) = patch.outline {
            next.decoration.outline = value;
        }
        if let Some(value) = patch.corner_radii {
            next.decoration.corner_radii = value;
        }
        if let Some(value) = patch.shadows {
            next.decoration.shadows = value;
        }
        if let Some(width) = patch.border_width {
            next.decoration.border.top.width = width;
            next.decoration.border.right.width = width;
            next.decoration.border.bottom.width = width;
            next.decoration.border.left.width = width;
        }
        if let Some(color) = patch.border_color {
            next.decoration.border.top.color = color;
            next.decoration.border.right.color = color;
            next.decoration.border.bottom.color = color;
            next.decoration.border.left.color = color;
        }
        if let Some(width) = patch.outline_width {
            next.decoration.outline.width = width;
        }
        if let Some(offset) = patch.outline_offset {
            next.decoration.outline.offset = offset;
        }
        if let Some(color) = patch.outline_color {
            next.decoration.outline.color = color;
        }
        if let Some(radius) = patch.radius {
            next.decoration.corner_radii = CornerRadii::all(radius);
        }
        if let Some(value) = patch.translation_x {
            next.transform.translation.x = value;
        }
        if let Some(value) = patch.translation_y {
            next.transform.translation.y = value;
        }
        if let Some(value) = patch.scale_x {
            next.transform.scale.x = value;
        }
        if let Some(value) = patch.scale_y {
            next.transform.scale.y = value;
        }
        if let Some(value) = patch.rotation {
            next.transform.rotation = value;
        }
        if let Some(value) = patch.origin_x {
            next.transform.origin.x = value;
        }
        if let Some(value) = patch.origin_y {
            next.transform.origin.y = value;
        }
        let mut dirty = DirtyFlags::NONE;
        if next != style {
            if next.sizing != style.sizing
                || next.width != style.width
                || next.height != style.height
                || next.min_size != style.min_size
                || next.max_size != style.max_size
                || next.margin != style.margin
                || next.padding != style.padding
                || next.decoration.border != style.decoration.border
            {
                dirty |= DirtyFlags::LAYOUT;
            }
            if next.transform != style.transform {
                dirty |= DirtyFlags::SPATIAL | DirtyFlags::CLIP;
            }
            dirty |= DirtyFlags::STYLE | DirtyFlags::PAINT;
            self.box_styles.insert(node, next);
        }
        if let Some(text) = self.texts.get_mut(node) {
            let old = text.style;
            if let Some(value) = patch.text_color {
                text.style.color = value;
            }
            if let Some(value) = patch.text_size {
                text.style.size = value;
            }
            if let Some(value) = patch.text_line_height {
                text.style.line_height = value;
            }
            if let Some(value) = patch.text_family {
                text.style.family = value;
            }
            if let Some(value) = patch.text_weight {
                text.style.weight = value;
            }
            if text.style != old {
                text.revision = text.revision.wrapping_add(1).max(1);
                dirty |= DirtyFlags::TEXT | DirtyFlags::PAINT;
                if text.style.size != old.size
                    || text.style.line_height != old.line_height
                    || text.style.family != old.family
                    || text.style.weight != old.weight
                {
                    dirty |= DirtyFlags::LAYOUT;
                }
            }
        }
        if let Some(image) = self.images.get_mut(node)
            && let Some(tint) = patch.image_tint
            && image.tint != tint
        {
            image.tint = tint;
            dirty |= DirtyFlags::STYLE | DirtyFlags::PAINT;
        }
        if dirty == DirtyFlags::NONE {
            return false;
        }
        self.nodes.mark_dirty(node, dirty);
        true
    }
    fn set_interaction_flag(
        &mut self,
        node: NodeId,
        flag: InteractionFlags,
        enabled: bool,
    ) -> bool {
        if !self.nodes.contains(node) {
            return false;
        }
        if self.interactions.get(node).is_none() {
            if !enabled {
                return false;
            }
            self.interactions
                .insert(node, InteractionSnapshot::default());
        }
        let interaction = self
            .interactions
            .get_mut(node)
            .expect("interaction was inserted above");
        let changed = if flag == InteractionFlags::DISABLED {
            interaction.set_enabled(!enabled)
        } else {
            interaction.set_flag(flag, enabled)
        };
        if !changed {
            return false;
        }
        if let Some(core) = self.nodes.core_mut(node) {
            core.state_bits = interaction.flags.bits();
            core.style_revision = core.style_revision.wrapping_add(1).max(1);
        }
        self.nodes
            .mark_dirty(node, DirtyFlags::STYLE | DirtyFlags::PAINT);
        self.enqueue_style_bindings_for_state(node);
        true
    }

    /// Publishes router-owned transient/focus state. This is intentionally separated from
    /// component-controlled semantic state so controls cannot impersonate the input router.
    #[doc(hidden)]
    pub fn route_interaction_flag(
        &mut self,
        node: NodeId,
        flag: InteractionFlags,
        enabled: bool,
    ) -> bool {
        if flag.bits() & !InteractionFlags::ROUTER_OWNED.bits() != 0
            || flag.bits().count_ones() != 1
        {
            return false;
        }
        self.set_interaction_flag(node, flag, enabled)
    }

    pub fn set_disabled(&mut self, node: NodeId, disabled: bool) -> bool {
        self.set_interaction_flag(node, InteractionFlags::DISABLED, disabled)
    }

    pub fn set_read_only(&mut self, node: NodeId, read_only: bool) -> bool {
        self.set_interaction_flag(node, InteractionFlags::READ_ONLY, read_only)
    }

    pub fn set_busy(&mut self, node: NodeId, busy: bool) -> bool {
        self.set_interaction_flag(node, InteractionFlags::BUSY, busy)
    }

    pub fn set_checked(&mut self, node: NodeId, checked: bool) -> bool {
        self.set_interaction_flag(node, InteractionFlags::CHECKED, checked)
    }

    pub fn set_mixed(&mut self, node: NodeId, mixed: bool) -> bool {
        self.set_interaction_flag(node, InteractionFlags::MIXED, mixed)
    }

    pub fn set_selected(&mut self, node: NodeId, selected: bool) -> bool {
        self.set_interaction_flag(node, InteractionFlags::SELECTED, selected)
    }

    pub fn set_expanded(&mut self, node: NodeId, expanded: bool) -> bool {
        self.set_interaction_flag(node, InteractionFlags::EXPANDED, expanded)
    }

    pub fn set_invalid(&mut self, node: NodeId, invalid: bool) -> bool {
        self.set_interaction_flag(node, InteractionFlags::INVALID, invalid)
    }

    pub fn set_active(&mut self, node: NodeId, active: bool) -> bool {
        self.set_interaction_flag(node, InteractionFlags::ACTIVE, active)
    }

    pub fn set_highlighted(&mut self, node: NodeId, highlighted: bool) -> bool {
        self.set_interaction_flag(node, InteractionFlags::HIGHLIGHTED, highlighted)
    }

    pub fn set_control_value(&mut self, node: NodeId, value: f32) -> bool {
        if !self.nodes.contains(node) || !value.is_finite() {
            return false;
        }
        if self.interactions.get(node).is_none() {
            self.interactions
                .insert(node, InteractionSnapshot::default());
        }
        let value = value.clamp(0.0, 1.0);
        let interaction_changed = self.interactions.get_mut(node).is_some_and(|interaction| {
            if interaction.value == value {
                false
            } else {
                interaction.value = value;
                interaction.revision = interaction.revision.wrapping_add(1).max(1);
                true
            }
        });
        let semantic_changed = self.semantics.get_mut(node).is_some_and(|semantic| {
            let SemanticValue::Number { current, .. } = &mut semantic.value else {
                return false;
            };
            let value = f64::from(value);
            if *current == value {
                false
            } else {
                *current = value;
                true
            }
        });
        if interaction_changed || semantic_changed {
            self.nodes
                .mark_dirty(node, DirtyFlags::SEMANTICS | DirtyFlags::PAINT);
        }
        interaction_changed || semantic_changed
    }

    /// Returns the nearest explicitly registered control at or above a hit node.
    pub fn nearest_control(&self, mut node: NodeId) -> Option<NodeId> {
        loop {
            if self.interactions.get(node).is_some_and(|interaction| {
                interaction.behavior != ControlBehavior::None && interaction.visible
            }) {
                return Some(node);
            }
            node = self.nodes.core(node)?.parent?;
        }
    }

    pub fn is_descendant_or_self(&self, node: NodeId, ancestor: NodeId) -> bool {
        let mut cursor = Some(node);
        while let Some(current) = cursor {
            if current == ancestor {
                return true;
            }
            cursor = self.nodes.core(current).and_then(|core| core.parent);
        }
        false
    }

    pub fn set_hover_within(&mut self, node: NodeId, enabled: bool) -> bool {
        if !self.nodes.contains(node) {
            return false;
        }
        if self.interactions.get(node).is_none() {
            if !enabled {
                return false;
            }
            self.interactions.insert(node, InteractionSnapshot::default());
        }
        let interaction = self.interactions.get_mut(node).unwrap();
        if interaction.hover_within == enabled {
            return false;
        }
        interaction.hover_within = enabled;
        interaction.revision = interaction.revision.wrapping_add(1).max(1);
        true
    }

    pub fn set_control_behavior(&mut self, node: NodeId, behavior: ControlBehavior) -> bool {
        if !self.nodes.contains(node) {
            return false;
        }
        if self.interactions.get(node).is_none() {
            self.interactions
                .insert(node, InteractionSnapshot::default());
        }
        let interaction = self
            .interactions
            .get_mut(node)
            .expect("interaction was inserted above");
        if interaction.behavior == behavior {
            return false;
        }
        interaction.behavior = behavior;
        interaction.revision = interaction.revision.wrapping_add(1).max(1);
        true
    }
    pub fn set_listener_mask(&mut self, node: NodeId, mask: u16) -> bool {
        if !self.nodes.contains(node) {
            return false;
        }
        if self.interactions.get(node).is_none() {
            if mask == 0 {
                return false;
            }
            self.interactions
                .insert(node, InteractionSnapshot::default());
        }
        let interaction = self
            .interactions
            .get_mut(node)
            .expect("interaction was inserted above");
        if interaction.listener_mask == mask {
            return false;
        }
        interaction.listener_mask = mask;
        true
    }
    /// Atomically replaces a mounted semantic input and marks only semantic work dirty.
    pub fn set_semantics(
        &mut self,
        node: NodeId,
        semantic: SemanticNode,
    ) -> Result<bool, SemanticError> {
        let validation = if !self.nodes.contains(node) {
            Err(SemanticError::UnknownNode(node))
        } else {
            semantic.validate(node).and_then(|()| {
                if let Some(string) = semantic
                    .referenced_strings()
                    .find(|string| self.string(*string).is_none())
                {
                    return Err(SemanticError::UnknownString(string));
                }
                semantic
                    .relationships
                    .iter()
                    .find(|relationship| !self.nodes.contains(relationship.target))
                    .map_or(Ok(()), |relationship| {
                        Err(SemanticError::UnknownRelationshipTarget(
                            relationship.target,
                        ))
                    })
            })
        };
        if let Err(error) = validation {
            self.diagnostics.semantic_failures += 1;
            return Err(error);
        }
        if self
            .semantics
            .get(node)
            .is_some_and(|current| current == &semantic)
        {
            return Ok(false);
        }
        self.semantics.insert(node, semantic);
        self.nodes.mark_dirty(node, DirtyFlags::SEMANTICS);
        if let Some(core) = self.nodes.core_mut(node) {
            core.semantic_revision += 1;
        }
        self.diagnostics.semantic_updates += 1;
        Ok(true)
    }

    pub fn clear_semantics(&mut self, node: NodeId) -> bool {
        if self.semantics.remove(node).is_none() {
            return false;
        }
        self.nodes.mark_dirty(node, DirtyFlags::SEMANTICS);
        if let Some(core) = self.nodes.core_mut(node) {
            core.semantic_revision += 1;
        }
        self.diagnostics.semantic_updates += 1;
        true
    }

    /// Associates protocol-neutral window-chrome meaning with a normal mounted node.
    ///
    /// Chrome roles do not alter layout or paint. Window hosts consume the role together with
    /// the node's computed bounds after layout, which keeps decoration geometry authored by the
    /// same composition primitives as the rest of the UI.
    #[doc(hidden)]
    pub fn set_window_chrome_role(
        &mut self,
        node: NodeId,
        role: Option<crate::window_chrome::WindowChromeRole>,
    ) -> bool {
        if !self.nodes.contains(node) {
            return false;
        }
        match role {
            Some(role) => {
                if self.window_chrome_roles.get(node).copied() == Some(role) {
                    false
                } else {
                    self.window_chrome_roles.insert(node, role);
                    true
                }
            }
            None => self.window_chrome_roles.remove(node).is_some(),
        }
    }

    /// Associates hit-test tuning with a semantic window-chrome node.
    #[doc(hidden)]
    pub fn set_window_chrome_hit_spec(
        &mut self,
        node: NodeId,
        spec: Option<crate::window_chrome::WindowChromeHitSpec>,
    ) -> bool {
        if !self.nodes.contains(node) {
            return false;
        }
        match spec {
            Some(spec) => {
                if self.window_chrome_hit_specs.get(node).copied() == Some(spec) {
                    false
                } else {
                    self.window_chrome_hit_specs.insert(node, spec);
                    true
                }
            }
            None => self.window_chrome_hit_specs.remove(node).is_some(),
        }
    }

    /// Associates a semantic pointer request with a normal mounted node.
    #[doc(hidden)]
    pub fn set_pointer_request(
        &mut self,
        node: NodeId,
        request: Option<crate::assets::PointerRequest>,
    ) -> bool {
        if !self.nodes.contains(node) {
            return false;
        }
        match request {
            Some(request) => {
                if self.pointer_requests.get(node).copied() == Some(request) {
                    false
                } else {
                    self.pointer_requests.insert(node, request);
                    true
                }
            }
            None => self.pointer_requests.remove(node).is_some(),
        }
    }
    #[cfg(test)]
    fn insert_fixture(
        &mut self,
        parent: Option<NodeId>,
        before: Option<NodeId>,
        fixture: NodeFixture,
    ) -> Option<NodeId> {
        if before.is_some() && parent.is_none() {
            return None;
        }
        let node = self.nodes.spawn(parent)?;
        if before.is_some() && !self.nodes.reparent_before(node, parent?, before) {
            self.nodes.remove_subtree(node);
            return None;
        }
        self.kinds.insert(node, fixture.kind);
        if fixture.style != BoxStyle::default() {
            self.box_styles.insert(node, fixture.style);
        }
        if fixture.layout != LayoutStyle::default() {
            self.layouts.insert(node, fixture.layout);
        }
        if fixture.interaction != InteractionSnapshot::default() {
            self.interactions.insert(node, fixture.interaction);
            if let Some(core) = self.nodes.core_mut(node) {
                core.state_bits = fixture.interaction.flags.bits();
            }
        }
        if fixture.key.is_some() {
            self.keys.insert(node, fixture.key);
        }
        if let Some(text) = fixture.text {
            self.texts.insert(node, text);
        }
        if let Some(image) = fixture.image {
            self.images.insert(node, image);
        }
        if let Some(semantic) = fixture.semantic {
            self.semantics.insert(node, semantic);
        }
        for child in fixture.children {
            self.insert_fixture(Some(node), None, child);
        }
        Some(node)
    }
    fn commit(&mut self) -> TransactionResult {
        let mut result = TransactionResult::default();
        let patches = std::mem::take(&mut self.patch_log);
        for patch in &patches {
            if self.apply_patch(patch) {
                result.property_patches += 1;
            }
        }
        self.patch_log = patches;
        self.patch_log.clear();
        let mut structural = std::mem::take(&mut self.structural_log);
        for command in structural.drain(..) {
            match command {
                StructuralCommand::Remove(node) => {
                    if !self.remove(node).is_empty() {
                        result.structural_mutations += 1;
                    }
                }
                #[cfg(test)]
                StructuralCommand::Reconcile { parent, children } => {
                    result.structural_mutations += self.reconcile(parent, children);
                }
            }
        }
        self.structural_log = structural;
        self.diagnostics.property_patches += result.property_patches as u64;
        self.diagnostics.structural_mutations += result.structural_mutations as u64;
        result
    }
    #[cfg(test)]
    fn reconcile(&mut self, parent: NodeId, children: Vec<NodeFixture>) -> usize {
        if !self.nodes.contains(parent) {
            return 0;
        }
        let existing: Vec<_> = self.nodes.children(parent).collect();
        let mut retained = Vec::with_capacity(children.len());
        let mut before = existing.first().copied();
        let mut mutations = 0;
        for fixture in children {
            let reusable = fixture.key.and_then(|key| {
                existing.iter().copied().find(|node| {
                    !retained.contains(node) && self.keys.get(*node).copied().flatten() == Some(key)
                })
            });
            if let Some(node) = reusable {
                if before == Some(node) {
                    before = self.nodes.core(node).and_then(|core| core.next_sibling);
                } else if self.nodes.reparent_before(node, parent, before) {
                    mutations += 1;
                }
                retained.push(node);
                mutations += self.sync_fixture(node, fixture);
            } else if let Some(node) = self.insert_fixture(Some(parent), before, fixture) {
                retained.push(node);
                mutations += 1;
            }
        }
        for node in existing {
            if !retained.contains(&node) {
                self.remove(node);
                mutations += 1;
            }
        }
        mutations
    }

    #[cfg(test)]
    fn sync_fixture(&mut self, node: NodeId, fixture: NodeFixture) -> usize {
        let NodeFixture {
            key,
            kind,
            style,
            layout,
            interaction,
            text,
            image,
            semantic,
            children,
        } = fixture;
        let kind_changed = replace_value(&mut self.kinds, node, kind);
        let style_changed = replace_default(&mut self.box_styles, node, style);
        let layout_changed = replace_default(&mut self.layouts, node, layout);
        let interaction_changed = replace_default(&mut self.interactions, node, interaction);
        let text_changed = replace_optional(&mut self.texts, node, text);
        let image_changed = replace_optional(&mut self.images, node, image);
        let semantic_changed = replace_optional(&mut self.semantics, node, semantic);
        replace_optional(&mut self.keys, node, key.map(Some));
        let mut dirty = DirtyFlags::NONE;
        if kind_changed || style_changed {
            dirty |= DirtyFlags::STYLE
                | DirtyFlags::LAYOUT
                | DirtyFlags::SPATIAL
                | DirtyFlags::CLIP
                | DirtyFlags::PAINT;
        }
        if layout_changed {
            dirty |= DirtyFlags::LAYOUT | DirtyFlags::SPATIAL | DirtyFlags::CLIP;
        }
        if interaction_changed {
            dirty |= DirtyFlags::STYLE | DirtyFlags::PAINT | DirtyFlags::SEMANTICS;
        }
        if text_changed {
            dirty |= DirtyFlags::TEXT | DirtyFlags::MEASURE | DirtyFlags::PAINT;
        }
        if image_changed {
            dirty |= DirtyFlags::PAINT;
        }
        if semantic_changed {
            dirty |= DirtyFlags::SEMANTICS;
        }
        if dirty != DirtyFlags::NONE {
            self.nodes.mark_dirty(node, dirty);
            if let Some(core) = self.nodes.core_mut(node) {
                core.style_revision += u64::from(style_changed || interaction_changed);
                core.content_revision += u64::from(text_changed || image_changed);
                core.semantic_revision += u64::from(semantic_changed);
                core.state_bits = interaction.flags.bits();
            }
        }
        usize::from(dirty != DirtyFlags::NONE) + self.reconcile(node, children)
    }
    fn apply_patch(&mut self, patch: &Patch) -> bool {
        if !self.nodes.contains(patch.node) {
            return false;
        }
        match (&patch.kind, &patch.value) {
            (PropertyKind::Enabled, PropertyValue::Bool(false))
            | (PropertyKind::Visible, PropertyValue::Bool(false)) => {
                if self.interactions.get(patch.node).is_none() {
                    self.interactions
                        .insert(patch.node, InteractionSnapshot::default());
                }
            }
            (PropertyKind::Value, PropertyValue::Float(value)) if *value != 0.0 => {
                if self.interactions.get(patch.node).is_none() {
                    self.interactions
                        .insert(patch.node, InteractionSnapshot::default());
                }
            }
            (PropertyKind::Opacity, PropertyValue::Float(value)) if *value != 1.0 => {
                if self.box_styles.get(patch.node).is_none() {
                    self.box_styles.insert(patch.node, BoxStyle::default());
                }
            }
            (PropertyKind::Background, PropertyValue::Color(_)) => {
                if self.box_styles.get(patch.node).is_none() {
                    self.box_styles.insert(patch.node, BoxStyle::default());
                }
            }
            (PropertyKind::Translation, PropertyValue::Point(value))
                if *value != PointF::default() =>
            {
                if self.box_styles.get(patch.node).is_none() {
                    self.box_styles.insert(patch.node, BoxStyle::default());
                }
            }
            (PropertyKind::ScrollOffset, PropertyValue::Point(value))
                if *value != PointF::default() =>
            {
                if self.layouts.get(patch.node).is_none() {
                    self.layouts.insert(patch.node, LayoutStyle::default());
                }
            }
            (PropertyKind::Style, PropertyValue::Style(value)) if *value != BoxStyle::default() => {
                if self.box_styles.get(patch.node).is_none() {
                    self.box_styles.insert(patch.node, BoxStyle::default());
                }
            }
            (PropertyKind::Checked, PropertyValue::Check(_))
            | (PropertyKind::Busy, PropertyValue::Bool(_)) => {
                if self.interactions.get(patch.node).is_none() {
                    self.interactions
                        .insert(patch.node, InteractionSnapshot::default());
                }
            }
            _ => {}
        }
        let changed = match (&patch.kind, &patch.value) {
            (PropertyKind::Enabled, PropertyValue::Bool(value)) => self
                .interactions
                .get_mut(patch.node)
                .is_some_and(|interaction| interaction.set_enabled(*value)),
            (PropertyKind::Visible, PropertyValue::Bool(value)) => change_interaction(
                &mut self.interactions,
                patch.node,
                |item| &mut item.visible,
                *value,
            ),
            (PropertyKind::Value, PropertyValue::Float(value)) => {
                let interaction_changed = change_interaction(
                    &mut self.interactions,
                    patch.node,
                    |item| &mut item.value,
                    *value,
                );
                let semantic_changed = self.semantics.get_mut(patch.node).is_some_and(|semantic| {
                    let SemanticValue::Number { current, .. } = &mut semantic.value else {
                        return false;
                    };
                    let value = f64::from(*value);
                    if *current == value {
                        false
                    } else {
                        *current = value;
                        true
                    }
                });
                interaction_changed || semantic_changed
            }
            (PropertyKind::Opacity, PropertyValue::Float(value)) => change_style(
                &mut self.box_styles,
                patch.node,
                |style| &mut style.opacity,
                value.clamp(0.0, 1.0),
            ),
            (PropertyKind::Text, PropertyValue::String(value)) => {
                self.texts.get_mut(patch.node).is_some_and(|text| {
                    if text.content == *value {
                        false
                    } else {
                        text.content = *value;
                        text.revision += 1;
                        true
                    }
                })
            }
            (PropertyKind::TextColor, PropertyValue::Color(value)) => {
                self.texts.get_mut(patch.node).is_some_and(|text| {
                    if text.style.color == *value {
                        false
                    } else {
                        text.style.color = *value;
                        true
                    }
                })
            }
            (PropertyKind::Background, PropertyValue::Color(value)) => change_style(
                &mut self.box_styles,
                patch.node,
                |style| &mut style.decoration.background,
                Background::Color(*value),
            ),
            (PropertyKind::Translation, PropertyValue::Point(value)) => {
                self.box_styles.get_mut(patch.node).is_some_and(|style| {
                    if style.transform.translation == *value {
                        false
                    } else {
                        style.transform.translation = *value;
                        true
                    }
                })
            }
            (PropertyKind::ScrollOffset, PropertyValue::Point(value)) => change_layout(
                &mut self.layouts,
                patch.node,
                |layout| &mut layout.scroll_offset,
                *value,
            ),
            (PropertyKind::Style, PropertyValue::Style(value)) => {
                self.box_styles.get_mut(patch.node).is_some_and(|style| {
                    if style == value {
                        false
                    } else {
                        *style = *value;
                        true
                    }
                })
            }
            (PropertyKind::Checked, PropertyValue::Check(value)) => {
                let interaction_changed =
                    self.interactions
                        .get_mut(patch.node)
                        .is_some_and(|interaction| {
                            let checked = *value != SemanticCheckState::Unchecked;
                            let mixed = *value == SemanticCheckState::Mixed;
                            let checked_changed =
                                interaction.set_flag(InteractionFlags::CHECKED, checked);
                            let mixed_changed =
                                interaction.set_flag(InteractionFlags::MIXED, mixed);
                            checked_changed || mixed_changed
                        });
                let semantic_changed = self.semantics.get_mut(patch.node).is_some_and(|semantic| {
                    if semantic.state.checked == Some(*value) {
                        false
                    } else {
                        semantic.state.checked = Some(*value);
                        true
                    }
                });
                interaction_changed || semantic_changed
            }
            (PropertyKind::Busy, PropertyValue::Bool(value)) => {
                let interaction_changed =
                    self.interactions
                        .get_mut(patch.node)
                        .is_some_and(|interaction| {
                            interaction.set_flag(InteractionFlags::BUSY, *value)
                        });
                let semantic_changed = self.semantics.get_mut(patch.node).is_some_and(|semantic| {
                    if semantic.state.busy == *value {
                        false
                    } else {
                        semantic.state.busy = *value;
                        true
                    }
                });
                interaction_changed || semantic_changed
            }
            _ => false,
        };
        if changed {
            if patch.kind == PropertyKind::Enabled
                && let Some(interaction) = self.interactions.get_mut(patch.node)
            {
                if let Some(core) = self.nodes.core_mut(patch.node) {
                    core.state_bits = interaction.flags.bits();
                }
            }
            if matches!(patch.kind, PropertyKind::Checked | PropertyKind::Busy) {
                if let Some(interaction) = self.interactions.get(patch.node)
                    && let Some(core) = self.nodes.core_mut(patch.node)
                {
                    core.state_bits = interaction.flags.bits();
                }
                if let Some(core) = self.nodes.core_mut(patch.node) {
                    core.semantic_revision += 1;
                }
            }
            if patch.kind == PropertyKind::Value
                && let Some(core) = self.nodes.core_mut(patch.node)
            {
                core.semantic_revision += 1;
            }
            if matches!(
                patch.kind,
                PropertyKind::Enabled
                    | PropertyKind::Visible
                    | PropertyKind::Value
                    | PropertyKind::Checked
                    | PropertyKind::Busy
            ) && let Some(core) = self.nodes.core_mut(patch.node)
            {
                core.style_revision = core.style_revision.wrapping_add(1).max(1);
                if let Some(interaction) = self.interactions.get(patch.node) {
                    core.state_bits = interaction.flags.bits();
                }
            }
            let dirty = match patch.kind {
                PropertyKind::Text => DirtyFlags::TEXT | DirtyFlags::MEASURE | DirtyFlags::PAINT,
                PropertyKind::ScrollOffset | PropertyKind::Translation => {
                    DirtyFlags::SPATIAL | DirtyFlags::CLIP | DirtyFlags::PAINT
                }
                PropertyKind::Visible => {
                    DirtyFlags::VISIBILITY
                        | DirtyFlags::SPATIAL
                        | DirtyFlags::CLIP
                        | DirtyFlags::PAINT
                }
                PropertyKind::Enabled
                | PropertyKind::Opacity
                | PropertyKind::TextColor
                | PropertyKind::Background => DirtyFlags::STYLE | DirtyFlags::PAINT,
                PropertyKind::Value => {
                    DirtyFlags::STYLE | DirtyFlags::PAINT | DirtyFlags::SEMANTICS
                }
                PropertyKind::Checked | PropertyKind::Busy => {
                    DirtyFlags::STYLE | DirtyFlags::PAINT | DirtyFlags::SEMANTICS
                }
                PropertyKind::Style => {
                    DirtyFlags::STYLE
                        | DirtyFlags::LAYOUT
                        | DirtyFlags::SPATIAL
                        | DirtyFlags::CLIP
                        | DirtyFlags::PAINT
                }
            };
            self.nodes.mark_dirty(patch.node, dirty);
            if matches!(
                patch.kind,
                PropertyKind::Enabled
                    | PropertyKind::Visible
                    | PropertyKind::Value
                    | PropertyKind::Checked
                    | PropertyKind::Busy
            ) {
                self.enqueue_style_bindings_for_state(patch.node);
            }
        }
        changed
    }
}

fn change_interaction<T: PartialEq + Copy>(
    store: &mut SparseSet<InteractionSnapshot>,
    node: NodeId,
    field: impl FnOnce(&mut InteractionSnapshot) -> &mut T,
    value: T,
) -> bool {
    store.get_mut(node).is_some_and(|item| {
        let slot = field(item);
        if *slot == value {
            false
        } else {
            *slot = value;
            item.revision = item.revision.wrapping_add(1).max(1);
            true
        }
    })
}

#[cfg(test)]
fn replace_value<T: PartialEq>(store: &mut SparseSet<T>, node: NodeId, value: T) -> bool {
    if store.get(node).is_some_and(|current| current == &value) {
        false
    } else {
        store.insert(node, value);
        true
    }
}

#[cfg(test)]
fn replace_default<T: Default + PartialEq>(
    store: &mut SparseSet<T>,
    node: NodeId,
    value: T,
) -> bool {
    if value == T::default() {
        store.remove(node).is_some_and(|previous| previous != value)
    } else {
        replace_value(store, node, value)
    }
}

#[cfg(test)]
fn replace_optional<T: PartialEq>(
    store: &mut SparseSet<T>,
    node: NodeId,
    value: Option<T>,
) -> bool {
    match value {
        Some(value) => replace_value(store, node, value),
        None => store.remove(node).is_some(),
    }
}
fn change_style<T: PartialEq>(
    store: &mut SparseSet<BoxStyle>,
    node: NodeId,
    field: impl FnOnce(&mut BoxStyle) -> &mut T,
    value: T,
) -> bool {
    store.get_mut(node).is_some_and(|item| {
        let slot = field(item);
        if *slot == value {
            false
        } else {
            *slot = value;
            true
        }
    })
}
fn change_layout<T: PartialEq>(
    store: &mut SparseSet<LayoutStyle>,
    node: NodeId,
    field: impl FnOnce(&mut LayoutStyle) -> &mut T,
    value: T,
) -> bool {
    store.get_mut(node).is_some_and(|item| {
        let slot = field(item);
        if *slot == value {
            false
        } else {
            *slot = value;
            true
        }
    })
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct UiRoot(pub NodeId);
#[derive(Debug)]
pub struct MountWriter<'a, A> {
    ui: &'a mut MountedUi,
    parents: Vec<NodeId>,
    action_routes: Vec<(NodeId, A)>,
    owns_view_root: bool,
    mounted_root: bool,
}
impl<'a, A> MountWriter<'a, A> {
    pub fn new(ui: &'a mut MountedUi) -> Self {
        Self {
            ui,
            parents: Vec::with_capacity(16),
            action_routes: Vec::new(),
            owns_view_root: true,
            mounted_root: false,
        }
    }
    /// Creates the narrow mount writer used by the component runtime for one child subtree.
    #[doc(hidden)]
    pub fn under(ui: &'a mut MountedUi, parent: NodeId) -> Option<Self> {
        if !ui.nodes.contains(parent) {
            return None;
        }
        Some(Self {
            ui,
            parents: vec![parent],
            action_routes: Vec::new(),
            owns_view_root: false,
            mounted_root: false,
        })
    }
    /// Drains typed actions staged while mounting so the component runtime can own their routes.
    pub fn drain_action_routes(&mut self) -> impl Iterator<Item = (NodeId, A)> + '_ {
        self.action_routes.drain(..)
    }
    pub fn intern(&mut self, text: impl AsRef<str>) -> StringId {
        self.ui.intern(text)
    }
    /// Reuse a label owned by the same control subtree. Global interning can return another
    /// node's mutable text slot, which may be cleared while replacing that node.
    pub(crate) fn text_content(&self, node: NodeId) -> Option<StringId> {
        self.ui.texts.get(node).map(|text| text.content)
    }
    pub fn root(
        &mut self,
        style: BoxStyle,
        layout: LayoutStyle,
        content: impl FnOnce(&mut Self),
    ) -> UiRoot {
        assert!(
            !self.mounted_root,
            "a mount writer creates exactly one root"
        );
        if self.owns_view_root {
            assert!(self.ui.root.is_none(), "a mounted UI has exactly one root");
        }
        let node = self.mount(NodeKind::Box, style, layout, InteractionSnapshot::default());
        self.mounted_root = true;
        if self.owns_view_root {
            self.ui.root = Some(UiRoot(node));
        }
        self.parents.push(node);
        content(self);
        self.parents.pop();
        UiRoot(node)
    }
    pub fn container(
        &mut self,
        style: BoxStyle,
        layout: LayoutStyle,
        content: impl FnOnce(&mut Self),
    ) -> NodeId {
        let node = self.mount(NodeKind::Box, style, layout, InteractionSnapshot::default());
        self.parents.push(node);
        content(self);
        self.parents.pop();
        node
    }
    /// Creates a noninteractive container and exposes its patchable visual properties.
    #[doc(hidden)]
    pub fn container_handle(
        &mut self,
        style: BoxStyle,
        layout: LayoutStyle,
        content: impl FnOnce(&mut Self),
    ) -> ControlHandle {
        ControlHandle::new(self.container(style, layout, content))
    }
    /// Mounts an overlay/studio layer whose visibility can be patched without rebuilding it.
    pub fn layer(
        &mut self,
        visible: bool,
        style: BoxStyle,
        layout: LayoutStyle,
        content: impl FnOnce(&mut Self),
    ) -> ControlHandle {
        let node = self.mount(
            NodeKind::Box,
            style,
            layout,
            InteractionSnapshot {
                visible,
                ..InteractionSnapshot::default()
            },
        );
        self.parents.push(node);
        content(self);
        self.parents.pop();
        ControlHandle::new(node)
    }
    /// Creates a component-owned visibility layer under an existing host.
    #[doc(hidden)]
    pub fn layer_node_under(
        &mut self,
        parent: NodeId,
        visible: bool,
        style: BoxStyle,
        layout: LayoutStyle,
        content: impl FnOnce(&mut Self),
    ) -> Option<ControlHandle> {
        if !self.ui.nodes.contains(parent) {
            return None;
        }
        self.parents.push(parent);
        let layer = self.layer(visible, style, layout, content);
        self.parents.pop();
        Some(layer)
    }
    pub fn scroll(
        &mut self,
        style: BoxStyle,
        layout: LayoutStyle,
        content: impl FnOnce(&mut Self),
    ) -> ScrollHandle {
        let node = self.mount(
            NodeKind::Scroll,
            style,
            layout,
            InteractionSnapshot {
                behavior: ControlBehavior::Scroll,
                ..InteractionSnapshot::default()
            },
        );
        self.parents.push(node);
        content(self);
        self.parents.pop();
        ScrollHandle {
            node,
            offset: Property::new(node, PropertyKind::ScrollOffset),
            style: Property::new(node, PropertyKind::Style),
        }
    }
    /// Creates a component-owned scroll viewport under an existing host.
    #[doc(hidden)]
    pub fn scroll_node_under(
        &mut self,
        parent: NodeId,
        style: BoxStyle,
        layout: LayoutStyle,
        enabled: bool,
        focusable: bool,
        content: impl FnOnce(&mut Self),
    ) -> Option<ScrollHandle> {
        if !self.ui.nodes.contains(parent) {
            return None;
        }
        self.parents.push(parent);
        let node = self.mount(
            NodeKind::Scroll,
            style,
            layout,
            InteractionSnapshot {
                enabled,
                focusable,
                behavior: ControlBehavior::Scroll,
                ..InteractionSnapshot::default()
            },
        );
        self.parents.push(node);
        content(self);
        self.parents.pop();
        self.parents.pop();
        Some(ScrollHandle {
            node,
            offset: Property::new(node, PropertyKind::ScrollOffset),
            style: Property::new(node, PropertyKind::Style),
        })
    }
    pub fn text(&mut self, content: impl AsRef<str>, color: ColorRgba8, size: f32) -> TextHandle {
        let content = self.ui.intern(content);
        let node = self.mount(
            NodeKind::Text,
            BoxStyle::default(),
            LayoutStyle::default(),
            InteractionSnapshot::default(),
        );
        self.ui.texts.insert(
            node,
            TextVisual {
                content,
                style: TextStyle {
                    color,
                    size,
                    line_height: size * 1.25,
                    family: StringId(1),
                    weight: 400,
                    align: TextAlign::Start,
                },
                revision: 1,
            },
        );
        TextHandle {
            node,
            text: Property::new(node, PropertyKind::Text),
            color: Property::new(node, PropertyKind::TextColor),
            enabled: Property::new(node, PropertyKind::Enabled),
            style: Property::new(node, PropertyKind::Style),
        }
    }
    /// Creates a retained text node with node-owned reusable content storage.
    #[doc(hidden)]
    pub fn dynamic_text(
        &mut self,
        content: impl Into<String>,
        text_style: TextStyle,
        style: BoxStyle,
        layout: LayoutStyle,
    ) -> TextHandle {
        let node = self.mount(
            NodeKind::Text,
            style,
            layout,
            InteractionSnapshot::default(),
        );
        let content = self.ui.allocate_dynamic_text(node, content.into());
        self.ui.texts.insert(
            node,
            TextVisual {
                content,
                style: text_style,
                revision: 1,
            },
        );
        let _ = self.ui.set_semantics(
            node,
            SemanticNode {
                role: SemanticRole::Text,
                name: SemanticName::Text(content),
                ..SemanticNode::default()
            },
        );
        TextHandle {
            node,
            text: Property::new(node, PropertyKind::Text),
            color: Property::new(node, PropertyKind::TextColor),
            enabled: Property::new(node, PropertyKind::Enabled),
            style: Property::new(node, PropertyKind::Style),
        }
    }
    /// Creates a component-owned retained text node under an existing host while preserving the
    /// caller's complete visual, content revision, box style, and layout inputs.
    #[doc(hidden)]
    pub fn text_node_under(
        &mut self,
        parent: NodeId,
        visual: TextVisual,
        style: BoxStyle,
        layout: LayoutStyle,
        enabled: bool,
        focusable: bool,
    ) -> Option<TextHandle> {
        if !self.ui.nodes.contains(parent) {
            return None;
        }
        self.parents.push(parent);
        let node = self.mount(
            NodeKind::Text,
            style,
            layout,
            InteractionSnapshot {
                enabled,
                focusable,
                ..InteractionSnapshot::default()
            },
        );
        self.parents.pop();
        self.ui.texts.insert(node, visual);
        Some(TextHandle {
            node,
            text: Property::new(node, PropertyKind::Text),
            color: Property::new(node, PropertyKind::TextColor),
            enabled: Property::new(node, PropertyKind::Enabled),
            style: Property::new(node, PropertyKind::Style),
        })
    }
    pub fn button(
        &mut self,
        action: A,
        style: BoxStyle,
        content: impl FnOnce(&mut Self),
    ) -> ControlHandle {
        let control = self.button_node(style, content);
        self.action_routes.push((control.node, action));
        control
    }
    /// Creates a foundation button node without storing a typed action in mounted UI. The
    /// component runtime uses this entry point so it can own generation-bound action factories.
    #[doc(hidden)]
    pub fn button_node(
        &mut self,
        style: BoxStyle,
        content: impl FnOnce(&mut Self),
    ) -> ControlHandle {
        let node = self.mount(
            NodeKind::Button,
            style,
            LayoutStyle {
                main_axis_alignment: MainAxisAlignment::Center,
                cross_axis_alignment: CrossAxisAlignment::Center,
                ..LayoutStyle::default()
            },
            InteractionSnapshot {
                focusable: true,
                behavior: ControlBehavior::Activate,
                ..InteractionSnapshot::default()
            },
        );
        self.parents.push(node);
        content(self);
        self.parents.pop();
        ControlHandle::new(node)
    }
    /// Creates a runtime-routed toggle at the current mount parent.
    #[doc(hidden)]
    pub fn toggle_node(
        &mut self,
        style: BoxStyle,
        content: impl FnOnce(&mut Self),
    ) -> ControlHandle {
        let node = self.mount(
            NodeKind::Toggle,
            style,
            LayoutStyle::default(),
            InteractionSnapshot {
                focusable: true,
                behavior: ControlBehavior::Activate,
                ..InteractionSnapshot::default()
            },
        );
        self.parents.push(node);
        content(self);
        self.parents.pop();
        ControlHandle::new(node)
    }
    /// Creates a runtime-routed slider at the current mount parent.
    #[doc(hidden)]
    pub fn slider_node(
        &mut self,
        style: BoxStyle,
        content: impl FnOnce(&mut Self),
    ) -> ControlHandle {
        let node = self.mount(
            NodeKind::Slider,
            style,
            LayoutStyle::default(),
            InteractionSnapshot {
                focusable: true,
                behavior: ControlBehavior::Value,
                ..InteractionSnapshot::default()
            },
        );
        self.parents.push(node);
        content(self);
        self.parents.pop();
        ControlHandle::new(node)
    }
    /// Creates a runtime-routed foundation button under an already mounted component host.
    #[doc(hidden)]
    pub fn button_node_under(
        &mut self,
        parent: NodeId,
        style: BoxStyle,
        content: impl FnOnce(&mut Self),
    ) -> Option<ControlHandle> {
        if !self.ui.nodes.contains(parent) {
            return None;
        }
        self.parents.push(parent);
        let control = self.button_node(style, content);
        self.parents.pop();
        Some(control)
    }
    /// Creates a runtime-routed toggle under an already mounted component host.
    #[doc(hidden)]
    pub fn toggle_node_under(
        &mut self,
        parent: NodeId,
        style: BoxStyle,
        content: impl FnOnce(&mut Self),
    ) -> Option<ControlHandle> {
        self.interactive_node_under(
            parent,
            NodeKind::Toggle,
            ControlBehavior::Activate,
            style,
            content,
        )
    }
    /// Creates a runtime-routed slider under an already mounted component host.
    #[doc(hidden)]
    pub fn slider_node_under(
        &mut self,
        parent: NodeId,
        style: BoxStyle,
        content: impl FnOnce(&mut Self),
    ) -> Option<ControlHandle> {
        self.interactive_node_under(
            parent,
            NodeKind::Slider,
            ControlBehavior::Value,
            style,
            content,
        )
    }
    fn interactive_node_under(
        &mut self,
        parent: NodeId,
        kind: NodeKind,
        behavior: ControlBehavior,
        style: BoxStyle,
        content: impl FnOnce(&mut Self),
    ) -> Option<ControlHandle> {
        if !self.ui.nodes.contains(parent) {
            return None;
        }
        self.parents.push(parent);
        let node = self.mount(
            kind,
            style,
            LayoutStyle::default(),
            InteractionSnapshot {
                focusable: true,
                behavior,
                ..InteractionSnapshot::default()
            },
        );
        self.parents.push(node);
        content(self);
        self.parents.pop();
        self.parents.pop();
        Some(ControlHandle::new(node))
    }
    /// Creates a component-owned action node under an existing host.
    #[doc(hidden)]
    pub fn action_node_under(
        &mut self,
        parent: NodeId,
        style: BoxStyle,
        enabled: bool,
        focusable: bool,
        content: impl FnOnce(&mut Self),
    ) -> Option<ControlHandle> {
        if !self.ui.nodes.contains(parent) {
            return None;
        }
        self.parents.push(parent);
        let node = self.mount(
            NodeKind::Button,
            style,
            LayoutStyle::default(),
            InteractionSnapshot {
                enabled,
                focusable,
                behavior: ControlBehavior::Activate,
                ..InteractionSnapshot::default()
            },
        );
        self.parents.push(node);
        content(self);
        self.parents.pop();
        self.parents.pop();
        Some(ControlHandle::new(node))
    }
    /// Creates a noninteractive component-owned container under an existing host.
    #[doc(hidden)]
    pub fn container_node_under(
        &mut self,
        parent: NodeId,
        style: BoxStyle,
        layout: LayoutStyle,
        content: impl FnOnce(&mut Self),
    ) -> Option<ControlHandle> {
        if !self.ui.nodes.contains(parent) {
            return None;
        }
        self.parents.push(parent);
        let node = self.container(style, layout, content);
        self.parents.pop();
        Some(ControlHandle::new(node))
    }
    /// Creates a platform-neutral text-input node under an existing component host.
    #[doc(hidden)]
    pub fn text_input_node_under(
        &mut self,
        parent: NodeId,
        style: BoxStyle,
        layout: LayoutStyle,
        enabled: bool,
        content: impl FnOnce(&mut Self),
    ) -> Option<ControlHandle> {
        if !self.ui.nodes.contains(parent) {
            return None;
        }
        self.parents.push(parent);
        let node = self.mount(
            NodeKind::TextInput,
            style,
            layout,
            InteractionSnapshot {
                enabled,
                focusable: true,
                behavior: ControlBehavior::TextInput,
                ..InteractionSnapshot::default()
            },
        );
        self.parents.push(node);
        content(self);
        self.parents.pop();
        self.parents.pop();
        Some(ControlHandle::new(node))
    }
    /// Creates a component-owned action node with explicit tab-stop participation.
    #[doc(hidden)]
    pub fn action_node(
        &mut self,
        style: BoxStyle,
        focusable: bool,
        content: impl FnOnce(&mut Self),
    ) -> ControlHandle {
        let node = self.mount(
            NodeKind::Button,
            style,
            LayoutStyle::default(),
            InteractionSnapshot {
                focusable,
                behavior: ControlBehavior::Activate,
                ..InteractionSnapshot::default()
            },
        );
        self.parents.push(node);
        content(self);
        self.parents.pop();
        ControlHandle::new(node)
    }
    pub fn image(&mut self, image: ImageId, style: BoxStyle) -> NodeId {
        let node = self.mount(
            NodeKind::Image,
            style,
            LayoutStyle::default(),
            InteractionSnapshot::default(),
        );
        self.ui.images.insert(
            node,
            ImageVisual {
                image,
                tint: None,
                content_version: 1,
            },
        );
        node
    }

    /// Creates a revisioned image node using complete composition layout inputs.
    pub fn dynamic_image(
        &mut self,
        image: ImageId,
        content_version: u64,
        style: BoxStyle,
        layout: LayoutStyle,
    ) -> ControlHandle {
        self.dynamic_image_tinted(image, content_version, None, style, layout)
    }

    /// Creates a revisioned image node with an optional alpha-mask tint.
    #[doc(hidden)]
    pub fn dynamic_image_tinted(
        &mut self,
        image: ImageId,
        content_version: u64,
        tint: Option<ColorRgba8>,
        style: BoxStyle,
        layout: LayoutStyle,
    ) -> ControlHandle {
        let node = self.mount(
            NodeKind::Image,
            style,
            layout,
            InteractionSnapshot::default(),
        );
        self.ui.images.insert(
            node,
            ImageVisual {
                image,
                tint,
                content_version: content_version.max(1),
            },
        );
        ControlHandle::new(node)
    }
    /// Creates a component-owned retained image under an existing host while preserving the
    /// caller's content version and layout inputs.
    #[doc(hidden)]
    pub fn image_node_under(
        &mut self,
        parent: NodeId,
        image: ImageId,
        content_version: u64,
        style: BoxStyle,
        layout: LayoutStyle,
    ) -> Option<ControlHandle> {
        if !self.ui.nodes.contains(parent) {
            return None;
        }
        self.parents.push(parent);
        let node = self.mount(
            NodeKind::Image,
            style,
            layout,
            InteractionSnapshot::default(),
        );
        self.parents.pop();
        self.ui.images.insert(
            node,
            ImageVisual {
                image,
                tint: None,
                content_version,
            },
        );
        Some(ControlHandle::new(node))
    }
    pub fn semantic(
        &mut self,
        node: NodeId,
        label: impl AsRef<str>,
        role: SemanticRole,
    ) -> Result<bool, SemanticError> {
        let label = self.ui.intern(label);
        self.ui
            .set_semantics(node, SemanticNode::named(role, label))
    }
    /// Attaches a complete component-authored semantic record during mount.
    pub fn semantic_node(
        &mut self,
        node: NodeId,
        semantic: SemanticNode,
    ) -> Result<bool, SemanticError> {
        self.ui.set_semantics(node, semantic)
    }
    /// Attaches a shell-understood role without coupling the node to shell-specific paint.
    #[doc(hidden)]
    pub fn window_chrome_role(
        &mut self,
        node: NodeId,
        role: Option<crate::window_chrome::WindowChromeRole>,
    ) -> bool {
        self.ui.set_window_chrome_role(node, role)
    }
    /// Attaches hit-test tuning to a shell-understood chrome region during mount.
    #[doc(hidden)]
    pub fn window_chrome_hit_spec(
        &mut self,
        node: NodeId,
        spec: Option<crate::window_chrome::WindowChromeHitSpec>,
    ) -> bool {
        self.ui.set_window_chrome_hit_spec(node, spec)
    }
    /// Attaches a semantic pointer request during mount.
    #[doc(hidden)]
    pub fn pointer_request(
        &mut self,
        node: NodeId,
        request: Option<crate::assets::PointerRequest>,
    ) -> bool {
        self.ui.set_pointer_request(node, request)
    }
    pub fn disabled(&mut self, node: NodeId, disabled: bool) -> bool {
        self.ui.set_disabled(node, disabled)
    }
    pub fn read_only(&mut self, node: NodeId, read_only: bool) -> bool {
        self.ui.set_read_only(node, read_only)
    }
    pub fn busy(&mut self, node: NodeId, busy: bool) -> bool {
        self.ui.set_busy(node, busy)
    }
    pub fn checked(&mut self, node: NodeId, checked: bool) -> bool {
        self.ui.set_checked(node, checked)
    }
    #[doc(hidden)]
    pub fn control_value(&mut self, node: NodeId, value: f32) -> bool {
        self.ui.set_control_value(node, value)
    }
    pub fn mixed(&mut self, node: NodeId, mixed: bool) -> bool {
        self.ui.set_mixed(node, mixed)
    }
    pub fn selected(&mut self, node: NodeId, selected: bool) -> bool {
        self.ui.set_selected(node, selected)
    }
    pub fn expanded(&mut self, node: NodeId, expanded: bool) -> bool {
        self.ui.set_expanded(node, expanded)
    }
    pub fn invalid(&mut self, node: NodeId, invalid: bool) -> bool {
        self.ui.set_invalid(node, invalid)
    }
    pub fn active(&mut self, node: NodeId, active: bool) -> bool {
        self.ui.set_active(node, active)
    }
    pub fn highlighted(&mut self, node: NodeId, highlighted: bool) -> bool {
        self.ui.set_highlighted(node, highlighted)
    }
    /// Registers explicit default behavior for an advanced/custom foundation node.
    pub fn control_behavior(&mut self, node: NodeId, behavior: ControlBehavior) -> bool {
        self.ui.set_control_behavior(node, behavior)
    }
    /// Registers an opt-in custom or first-party component style binding.
    pub fn style_binding(&mut self, binding: StyleBinding) -> bool {
        self.ui.register_style_binding(binding)
    }
    pub fn style_id(&mut self, node: NodeId, style: ComponentStyleId) -> bool {
        self.ui.set_style_id(node, style)
    }
    pub fn hover_within(&mut self, node: NodeId, enabled: bool) -> bool {
        self.ui.set_hover_within(node, enabled)
    }
    pub fn style_override(
        &mut self,
        node: NodeId,
        slot: StyleSlotId,
        patch: StylePropertyPatch,
    ) -> bool {
        self.ui.set_style_override(node, slot, patch)
    }
    pub fn listen(&mut self, node: NodeId, mask: u16) {
        self.ui.set_listener_mask(node, mask);
    }
    /// Associates an interactive value control with the node that owns its spatial track.
    #[doc(hidden)]
    pub fn value_track(&mut self, node: NodeId, track: NodeId, axis: ValueAxis) -> bool {
        if !self.ui.nodes.contains(node) || !self.ui.nodes.contains(track) {
            return false;
        }
        let Some(interaction) = self.ui.interactions.get_mut(node) else {
            return false;
        };
        if interaction.value_track == Some(track) && interaction.value_axis == Some(axis) {
            return false;
        }
        interaction.value_track = Some(track);
        interaction.value_axis = Some(axis);
        true
    }
    fn mount(
        &mut self,
        kind: NodeKind,
        style: BoxStyle,
        layout: LayoutStyle,
        mut interaction: InteractionSnapshot,
    ) -> NodeId {
        let parent = self.parents.last().copied();
        let node = self
            .ui
            .nodes
            .spawn(parent)
            .expect("mounted node arena exhausted");
        self.ui.kinds.insert(node, kind);
        if style != BoxStyle::default() {
            self.ui.box_styles.insert(node, style);
        }
        if layout != LayoutStyle::default() {
            self.ui.layouts.insert(node, layout);
        }
        interaction
            .flags
            .set(InteractionFlags::DISABLED, !interaction.enabled);
        if interaction != InteractionSnapshot::default() {
            self.ui.interactions.insert(node, interaction);
            if let Some(core) = self.ui.nodes.core_mut(node) {
                core.state_bits = interaction.flags.bits();
            }
        }
        let state_root = parent
            .and_then(|parent| self.ui.nearest_control(parent))
            .unwrap_or(node);
        let component = match kind {
            NodeKind::Box => "box",
            NodeKind::Text => "text",
            NodeKind::Image => "image",
            NodeKind::Button => "button",
            NodeKind::Toggle => "toggle",
            NodeKind::TextInput => "text-input",
            NodeKind::Slider => "slider",
            NodeKind::Scroll => "scroll",
            NodeKind::Collection => "collection",
            NodeKind::Custom(_) => "custom",
        };
        self.ui.register_style_binding(
            StyleBinding::new(
                state_root,
                ThemeScopeId::new(0, 1),
                ComponentStyleId::named(ThemeDomainId::APPLICATION, component, "default"),
            )
            .slot(StyleSlotId::named("root"), node),
        );
        node
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
enum PropertyKind {
    Enabled,
    Visible,
    Value,
    Opacity,
    Text,
    TextColor,
    Background,
    Translation,
    ScrollOffset,
    Style,
    Checked,
    Busy,
}
#[derive(Clone, Debug, PartialEq)]
pub enum PropertyValue {
    Bool(bool),
    Float(f32),
    String(StringId),
    Color(ColorRgba8),
    Point(PointF),
    Style(BoxStyle),
    Check(SemanticCheckState),
}
impl From<bool> for PropertyValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}
impl From<f32> for PropertyValue {
    fn from(value: f32) -> Self {
        Self::Float(value)
    }
}
impl From<StringId> for PropertyValue {
    fn from(value: StringId) -> Self {
        Self::String(value)
    }
}
impl From<ColorRgba8> for PropertyValue {
    fn from(value: ColorRgba8) -> Self {
        Self::Color(value)
    }
}
impl From<PointF> for PropertyValue {
    fn from(value: PointF) -> Self {
        Self::Point(value)
    }
}
impl From<BoxStyle> for PropertyValue {
    fn from(value: BoxStyle) -> Self {
        Self::Style(value)
    }
}
impl From<SemanticCheckState> for PropertyValue {
    fn from(value: SemanticCheckState) -> Self {
        Self::Check(value)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Property<T> {
    node: NodeId,
    kind: PropertyKind,
    marker: PhantomData<fn(T)>,
}
impl<T> Copy for Property<T> {}
impl<T> Clone for Property<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Property<T> {
    fn new(node: NodeId, kind: PropertyKind) -> Self {
        Self {
            node,
            kind,
            marker: PhantomData,
        }
    }
    pub fn node(self) -> NodeId {
        self.node
    }
}
#[derive(Copy, Clone, Debug)]
pub struct TextHandle {
    pub node: NodeId,
    pub text: Property<StringId>,
    pub color: Property<ColorRgba8>,
    pub enabled: Property<bool>,
    pub style: Property<BoxStyle>,
}
#[derive(Copy, Clone, Debug)]
pub struct ControlHandle {
    pub node: NodeId,
    pub enabled: Property<bool>,
    pub visible: Property<bool>,
    pub value: Property<f32>,
    pub opacity: Property<f32>,
    pub background: Property<ColorRgba8>,
    pub translation: Property<PointF>,
    pub style: Property<BoxStyle>,
    pub checked: Property<SemanticCheckState>,
    pub busy: Property<bool>,
}
impl ControlHandle {
    fn new(node: NodeId) -> Self {
        Self {
            node,
            enabled: Property::new(node, PropertyKind::Enabled),
            visible: Property::new(node, PropertyKind::Visible),
            value: Property::new(node, PropertyKind::Value),
            opacity: Property::new(node, PropertyKind::Opacity),
            background: Property::new(node, PropertyKind::Background),
            translation: Property::new(node, PropertyKind::Translation),
            style: Property::new(node, PropertyKind::Style),
            checked: Property::new(node, PropertyKind::Checked),
            busy: Property::new(node, PropertyKind::Busy),
        }
    }
}
#[derive(Copy, Clone, Debug)]
pub struct ScrollHandle {
    pub node: NodeId,
    pub offset: Property<PointF>,
    pub style: Property<BoxStyle>,
}

#[derive(Clone, Debug)]
struct Patch {
    node: NodeId,
    kind: PropertyKind,
    value: PropertyValue,
}
pub struct UiTransaction<'a> {
    ui: &'a mut MountedUi,
}
impl UiTransaction<'_> {
    pub fn set<T>(&mut self, property: Property<T>, value: T)
    where
        T: Into<PropertyValue>,
    {
        let value = value.into();
        if let Some(patch) = self
            .ui
            .patch_log
            .iter_mut()
            .find(|patch| patch.node == property.node && patch.kind == property.kind)
        {
            patch.value = value;
        } else {
            self.ui.patch_log.push(Patch {
                node: property.node,
                kind: property.kind,
                value,
            });
        }
    }
    pub fn remove(&mut self, node: NodeId) {
        self.ui.structural_log.push(StructuralCommand::Remove(node));
    }
    #[cfg(test)]
    fn reconcile_keyed(&mut self, parent: NodeId, children: Vec<NodeFixture>) {
        self.ui
            .structural_log
            .push(StructuralCommand::Reconcile { parent, children });
    }
}
#[derive(Clone, Debug)]
enum StructuralCommand {
    Remove(NodeId),
    #[cfg(test)]
    Reconcile {
        parent: NodeId,
        children: Vec<NodeFixture>,
    },
}
#[cfg(test)]
#[derive(Clone, Debug)]
struct NodeFixture {
    key: Option<u64>,
    kind: NodeKind,
    style: BoxStyle,
    layout: LayoutStyle,
    interaction: InteractionSnapshot,
    text: Option<TextVisual>,
    image: Option<ImageVisual>,
    semantic: Option<SemanticNode>,
    children: Vec<Self>,
}
#[cfg(test)]
impl NodeFixture {
    fn container(key: Option<u64>, style: BoxStyle, children: Vec<Self>) -> Self {
        Self {
            key,
            kind: NodeKind::Box,
            style,
            layout: LayoutStyle::default(),
            interaction: InteractionSnapshot::default(),
            text: None,
            image: None,
            semantic: None,
            children,
        }
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct TransactionResult {
    pub property_patches: usize,
    pub structural_mutations: usize,
}
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct UiDiagnostics {
    pub property_patches: u64,
    pub structural_mutations: u64,
    pub events_dispatched: u64,
    pub semantic_updates: u64,
    pub semantic_failures: u64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct UiEvent {
    /// The node selected by hit testing or focus dispatch.
    pub target: NodeId,
    /// The node whose listener is currently being invoked.
    pub current_target: NodeId,
    pub kind: UiEventKind,
    pub phase: EventPhase,
    pub timestamp: u64,
}
#[derive(Clone, Debug, PartialEq)]
pub enum UiEventKind {
    Input(InputEvent),
    Focus(bool),
    Text(StringId),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_alloc;
    #[derive(Clone, Debug)]
    enum Action {
        Save,
    }
    #[test]
    fn mounts_once_and_coalesces_property_writes() {
        let mut ui = MountedUi::default();
        let root;
        let button;
        {
            let mut builder = MountWriter::<Action>::new(&mut ui);
            let mut saved = None;
            root = builder.root(BoxStyle::default(), LayoutStyle::default(), |builder| {
                saved = Some(
                    builder.button(Action::Save, BoxStyle::default(), |builder| {
                        builder.text("Save", ColorRgba8::rgba(255, 255, 255, 255), 14.0);
                    }),
                );
            });
            button = saved.unwrap();
        }
        assert_eq!(ui.root(), Some(root));
        let (_, result) = ui.transaction(|tx| {
            tx.set(button.enabled, false);
            tx.set(button.enabled, true);
            tx.set(button.opacity, 0.5);
        });
        assert_eq!(result.property_patches, 1);
        assert!(ui.interactions.get(button.node).unwrap().enabled);
        assert_eq!(ui.box_styles.get(button.node).unwrap().opacity, 0.5);
    }
    #[test]
    fn state_only_changes_do_not_dirty_layout() {
        let mut ui = MountedUi::default();
        let button;
        {
            let mut builder = MountWriter::<Action>::new(&mut ui);
            let mut saved = None;
            builder.root(BoxStyle::default(), LayoutStyle::default(), |builder| {
                saved = Some(
                    builder.button(Action::Save, BoxStyle::default(), |builder| {
                        builder.text("Save", ColorRgba8::rgba(255, 255, 255, 255), 14.0);
                    }),
                );
            });
            button = saved.unwrap();
        }
        ui.nodes.clear_dirty(button.node, DirtyFlags::ALL);
        ui.transaction(|tx| tx.set(button.background, ColorRgba8::rgba(1, 2, 3, 255)));
        let dirty = ui.nodes.core(button.node).unwrap().dirty;
        assert!(dirty.contains(DirtyFlags::PAINT));
        assert!(!dirty.intersects(DirtyFlags::LAYOUT));
    }

    #[test]
    fn warmed_property_transaction_allocates_nothing() {
        let mut ui = MountedUi::default();
        let button;
        {
            let mut builder = MountWriter::<Action>::new(&mut ui);
            let mut saved = None;
            builder.root(BoxStyle::default(), LayoutStyle::default(), |builder| {
                saved = Some(
                    builder.button(Action::Save, BoxStyle::default(), |builder| {
                        builder.text("Save", ColorRgba8::rgba(255, 255, 255, 255), 14.0);
                    }),
                );
            });
            button = saved.unwrap();
        }
        ui.transaction(|transaction| transaction.set(button.opacity, 0.75));
        test_alloc::begin();
        ui.transaction(|transaction| {
            transaction.set(button.opacity, 0.5);
            transaction.set(button.opacity, 0.25);
        });
        assert_eq!(test_alloc::finish(), 0);
    }

    #[test]
    fn ten_thousand_simple_nodes_stay_within_low_single_digit_megabytes() {
        let mut ui = MountedUi::default();
        {
            let mut builder = MountWriter::<()>::new(&mut ui);
            builder.root(BoxStyle::default(), LayoutStyle::default(), |builder| {
                for _ in 0..10_000 {
                    builder.container(BoxStyle::default(), LayoutStyle::default(), |_| {});
                }
            });
        }
        let report = ui.memory_report();
        assert_eq!(report.mounted_nodes, 10_001);
        assert!(
            report.total_bytes() < 5 * 1024 * 1024,
            "{report:?} ({} bytes)",
            report.total_bytes()
        );
    }

    #[test]
    fn every_mounted_visual_node_has_a_generation_safe_style_binding() {
        let mut ui = MountedUi::default();
        {
            let mut writer = MountWriter::<Action>::new(&mut ui);
            writer.root(BoxStyle::default(), LayoutStyle::default(), |writer| {
                writer.container(BoxStyle::default(), LayoutStyle::default(), |writer| {
                    writer.text("label", ColorRgba8::rgba(1, 2, 3, 255), 12.0);
                    writer.image(ImageId(7), BoxStyle::default());
                });
                writer.button(Action::Save, BoxStyle::default(), |_| {});
                writer.scroll(BoxStyle::default(), LayoutStyle::default(), |_| {});
            });
        }
        for node in ui.nodes.alive() {
            assert!(
                ui.style_bindings().iter().any(|binding| {
                    ui.nodes.contains(binding.state_root)
                        && binding.slots.iter().any(|slot| slot.node == *node)
                }),
                "visual node {node:?} has no style binding"
            );
        }
    }

    #[test]
    fn keyed_reconcile_preserves_identity_order_and_updates_values() {
        let mut ui = MountedUi::default();
        let root = {
            let mut builder = MountWriter::<()>::new(&mut ui);
            builder.root(BoxStyle::default(), LayoutStyle::default(), |_| {})
        };
        ui.transaction(|transaction| {
            transaction.reconcile_keyed(
                root.0,
                vec![
                    NodeFixture::container(Some(1), BoxStyle::default(), Vec::new()),
                    NodeFixture::container(Some(2), BoxStyle::default(), Vec::new()),
                ],
            );
        });
        let original: Vec<_> = ui.nodes.children(root.0).collect();
        let changed = BoxStyle {
            opacity: 0.5,
            ..BoxStyle::default()
        };
        ui.transaction(|transaction| {
            transaction.reconcile_keyed(
                root.0,
                vec![
                    NodeFixture::container(Some(2), changed, Vec::new()),
                    NodeFixture::container(Some(1), BoxStyle::default(), Vec::new()),
                ],
            );
        });
        let reordered: Vec<_> = ui.nodes.children(root.0).collect();
        assert_eq!(reordered, vec![original[1], original[0]]);
        assert_eq!(ui.box_styles.get(original[1]), Some(&changed));
    }

    #[test]
    fn unkeyed_reconcile_replaces_old_nodes_instead_of_accumulating_them() {
        let mut ui = MountedUi::default();
        let root = {
            let mut builder = MountWriter::<()>::new(&mut ui);
            builder.root(BoxStyle::default(), LayoutStyle::default(), |_| {})
        };
        let child = || NodeFixture::container(None, BoxStyle::default(), Vec::new());
        ui.transaction(|transaction| transaction.reconcile_keyed(root.0, vec![child()]));
        let old = ui.nodes.children(root.0).next().unwrap();
        ui.transaction(|transaction| transaction.reconcile_keyed(root.0, vec![child()]));
        let children: Vec<_> = ui.nodes.children(root.0).collect();
        assert_eq!(children.len(), 1);
        assert_ne!(children[0], old);
        assert!(!ui.nodes.contains(old));
    }
}

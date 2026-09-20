use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::application_host::WindowFrameTemplate;
use crate::assets::{IconAsset, ImageSource};
use crate::compose::{
    Alignment, Component, ComponentFields, Dimension, Element, Insets, View, button, column, image,
    row, spacer, stack, text, window_content_slot, window_frame,
};
use crate::core::ColorRgba8;
use crate::theme::{
    CompiledComponentStyle, CompiledSlotStyle, CompiledStateStyle, InteractionState, TransitionSpec,
};
use crate::ui::{
    Background, Border, BoxDecoration, ComponentStyleId, InteractionFlags, Shadow, SizeRule,
    SizeRule2D, StylePropertyPatch, StyleSlotId, ThemeDomainId,
};
use crate::window_chrome::{
    WindowAction, WindowChromeModel, WindowChromeState, WindowContentStyle, WindowEdgeMask,
    WindowResizeEdge,
};

use super::WindowChromeViewExt;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowChromePalette {
    pub frame_background: ColorRgba8,
    /// Per-side visible border in normal/tiled states; maximized and fullscreen frames are borderless.
    pub frame_border: Border,
    pub title_color: ColorRgba8,
    pub title_weight: u16,
    /// Overrides the state shadow color; states without shadows remain shadowless.
    pub shadow_color: Option<ColorRgba8>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowChromeStateStyle {
    pub title_bar_visible: bool,
    /// Outer frame radius. Client content is automatically clipped to the inner border curve.
    pub frame_radius: f32,
    pub shadow: Option<Shadow>,
    pub resize_regions: bool,
    /// Minimum resize grab thickness, including the visible border. Extra width extends outward.
    pub resize_edge: f32,
    /// Extra tolerance outside the resize border; the easy frame never expands it inward.
    pub resize_hit_slop: Insets,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowTitleBarStyle {
    pub font_family: &'static str,
    pub height: f32,
    pub padding: Insets,
    pub gap: f32,
    pub title_size: f32,
    pub app_icon_region_size: f32,
    pub app_icon_size: f32,
    pub show_client_icon: bool,
    pub fallback_app_icon: Option<IconAsset>,
    pub app_icon_opens_system_menu: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowControlVisual {
    pub decoration: BoxDecoration,
    pub icon_tint: ColorRgba8,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowControlButtonStyle {
    /// Uses the title bar as its sizing parent; fill shares remaining width with the spacer.
    pub width: Dimension,
    /// Fill uses the title bar height remaining after vertical padding.
    pub height: Dimension,
    pub icon_size: f32,
    pub resting: WindowControlVisual,
    pub hovered: Option<WindowControlVisual>,
    pub pressed: Option<WindowControlVisual>,
    pub focused: Option<WindowControlVisual>,
    pub disabled: Option<WindowControlVisual>,
    pub transition: Option<TransitionSpec>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowControlDesign {
    pub icon: IconAsset,
    pub style: WindowControlButtonStyle,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowControlsDesign {
    pub minimize: WindowControlDesign,
    pub maximize: WindowControlDesign,
    pub restore: WindowControlDesign,
    pub close: WindowControlDesign,
    pub gap: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowChromeDesign {
    /// Shell window transitions, independent of state appearance.
    pub motion: crate::WindowMotion,
    pub active: WindowChromePalette,
    pub inactive: WindowChromePalette,
    pub normal: WindowChromeStateStyle,
    pub maximized: WindowChromeStateStyle,
    pub tiled: WindowChromeStateStyle,
    pub fullscreen: WindowChromeStateStyle,
    pub title_bar: WindowTitleBarStyle,
    pub controls: WindowControlsDesign,
    /// Backing beneath the application's pixels. Set alpha to zero to let client transparency
    /// reveal lower desktop layers. Opaque client buffers remain opaque.
    pub content_background: ColorRgba8,
    /// Resize placeholder appearance, independent of the normal content backing. `None`
    /// inherits `LinuxShellConfig::resize_preview`. Color alpha zero leaves only the preview border;
    /// glass tint alpha zero keeps the opaque blurred backdrop.
    pub resize_preview: Option<crate::ResizePreviewDesign>,
}

impl WindowChromeDesign {
    pub fn validate(self) -> Result<Self, WindowChromeDesignError> {
        if self
            .resize_preview
            .is_some_and(|preview| !preview.border_is_valid())
        {
            return Err(WindowChromeDesignError::InvalidResizePreviewBorder);
        }
        for palette in [self.active, self.inactive] {
            for side in [
                palette.frame_border.top,
                palette.frame_border.right,
                palette.frame_border.bottom,
                palette.frame_border.left,
            ] {
                finite_nonnegative(side.width)
                    .ok_or(WindowChromeDesignError::InvalidFrameBorderWidth)?;
            }
            if !(1..=1000).contains(&palette.title_weight) {
                return Err(WindowChromeDesignError::InvalidTitleWeight);
            }
        }
        for state in [self.normal, self.maximized, self.tiled, self.fullscreen] {
            finite_nonnegative(state.frame_radius)
                .ok_or(WindowChromeDesignError::InvalidFrameRadius)?;
            finite_nonnegative(state.resize_edge)
                .ok_or(WindowChromeDesignError::InvalidResizeEdge)?;
            validate_nonnegative_insets(state.resize_hit_slop)
                .ok_or(WindowChromeDesignError::InvalidResizeHitSlop)?;
            state
                .shadow
                .map_or(Ok(()), validate_shadow)
                .map_err(|_| WindowChromeDesignError::InvalidShadow)?;
        }
        for value in [
            self.title_bar.height,
            self.title_bar.gap,
            self.title_bar.title_size,
            self.title_bar.app_icon_region_size,
            self.title_bar.app_icon_size,
            self.controls.gap,
        ] {
            finite_nonnegative(value).ok_or(WindowChromeDesignError::InvalidTitleBarMetric)?;
        }
        if self.title_bar.height == 0.0 || self.title_bar.title_size == 0.0 {
            return Err(WindowChromeDesignError::InvalidTitleBarMetric);
        }
        for control in [
            self.controls.minimize,
            self.controls.maximize,
            self.controls.restore,
            self.controls.close,
        ] {
            for dimension in [control.style.width, control.style.height] {
                let valid = match dimension {
                    Dimension::Shrink => true,
                    Dimension::Logical(value) | Dimension::Fill(value) => {
                        value.is_finite() && value > 0.0
                    }
                    Dimension::Percent(value) => value.is_finite() && value > 0.0 && value <= 1.0,
                };
                if !valid {
                    return Err(WindowChromeDesignError::InvalidControlMetric);
                }
            }
            if !control.style.icon_size.is_finite() || control.style.icon_size <= 0.0 {
                return Err(WindowChromeDesignError::InvalidControlMetric);
            }
        }
        Ok(self)
    }

    fn palette(self, active: bool) -> WindowChromePalette {
        if active { self.active } else { self.inactive }
    }

    fn state(self, state: WindowChromeState) -> WindowChromeStateStyle {
        match state {
            WindowChromeState::Normal => self.normal,
            WindowChromeState::Maximized => self.maximized,
            WindowChromeState::Fullscreen => self.fullscreen,
            WindowChromeState::Tiled => self.tiled,
        }
    }
}

fn finite_nonnegative(value: f32) -> Option<f32> {
    (value.is_finite() && value >= 0.0).then_some(value)
}

fn validate_shadow(shadow: Shadow) -> Result<(), ()> {
    if !shadow.offset.x.is_finite()
        || !shadow.offset.y.is_finite()
        || finite_nonnegative(shadow.blur).is_none()
        || finite_nonnegative(shadow.spread).is_none()
    {
        Err(())
    } else {
        Ok(())
    }
}

fn validate_nonnegative_insets(insets: Insets) -> Option<Insets> {
    let insets = insets.0;
    [insets.top, insets.right, insets.bottom, insets.left]
        .into_iter()
        .all(|value| finite_nonnegative(value).is_some())
        .then_some(Insets(insets))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WindowChromeDesignError {
    #[error("resize preview border widths must be finite and nonnegative")]
    InvalidResizePreviewBorder,
    #[error("window chrome frame border width must be finite and nonnegative")]
    InvalidFrameBorderWidth,
    #[error("window chrome title weight must be between 1 and 1000")]
    InvalidTitleWeight,
    #[error("window chrome frame radius must be finite and nonnegative")]
    InvalidFrameRadius,
    #[error("window chrome resize edge must be finite and nonnegative")]
    InvalidResizeEdge,
    #[error("window chrome resize hit slop must be finite and nonnegative")]
    InvalidResizeHitSlop,
    #[error("window chrome shadow metrics must be finite and nonnegative")]
    InvalidShadow,
    #[error("window chrome title-bar metrics must be finite and positive where required")]
    InvalidTitleBarMetric,
    #[error("window chrome control metrics must be finite and positive")]
    InvalidControlMetric,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EasyWindowFrame {
    design: WindowChromeDesign,
}

impl EasyWindowFrame {
    pub const fn new(design: WindowChromeDesign) -> Self {
        Self { design }
    }

    pub const fn design(self) -> WindowChromeDesign {
        self.design
    }
}

pub const fn easy_window_frame(design: WindowChromeDesign) -> EasyWindowFrame {
    EasyWindowFrame::new(design)
}

#[doc(hidden)]
pub struct EasyWindowFrameComponent {
    model: WindowChromeModel,
    design: WindowChromeDesign,
}

impl ComponentFields for EasyWindowFrameComponent {
    type InputSnapshot = (WindowChromeModel, WindowChromeDesign);

    fn update_inputs(&mut self, incoming: Self) -> bool {
        if self.model == incoming.model && self.design == incoming.design {
            false
        } else {
            *self = incoming;
            true
        }
    }

    fn capture_inputs(&self) -> Self::InputSnapshot {
        (self.model.clone(), self.design)
    }

    fn restore_inputs(&mut self, snapshot: Self::InputSnapshot) -> bool {
        let changed = self.model != snapshot.0 || self.design != snapshot.1;
        self.model = snapshot.0;
        self.design = snapshot.1;
        changed
    }
}

impl Component for EasyWindowFrameComponent {
    fn view(&self) -> impl View {
        let design = self.design;
        let mut palette = design.palette(self.model.active);
        if matches!(
            self.model.state,
            WindowChromeState::Maximized | WindowChromeState::Fullscreen
        ) {
            palette.frame_border = Border::default();
        }
        let mut state = design.state(self.model.state);
        state.title_bar_visible &= self.model.title_bar_visible;
        if !self.model.frame_parts.border {
            palette.frame_border = Border::default();
        }
        if !self.model.frame_parts.rounded_clip {
            state.frame_radius = 0.0;
        }
        if !self.model.frame_parts.shadow {
            state.shadow = None;
        }
        state.resize_regions &= self.model.frame_parts.resize_regions;
        let border = palette.frame_border;
        // Match the analytic box's inner contour: each corner subtracts its thicker adjacent side.
        let inner_radii = crate::ui::CornerRadii {
            top_left: (state.frame_radius - border.top.width.max(border.left.width)).max(0.0),
            top_right: (state.frame_radius - border.top.width.max(border.right.width)).max(0.0),
            bottom_right: (state.frame_radius - border.bottom.width.max(border.right.width))
                .max(0.0),
            bottom_left: (state.frame_radius - border.bottom.width.max(border.left.width)).max(0.0),
        };
        let mut frame_decoration = BoxDecoration::new()
            .background(Background::Color(palette.frame_background))
            .border(palette.frame_border)
            .corner_radius(state.frame_radius);
        if let Some(mut shadow) = state.shadow {
            shadow.color = palette.shadow_color.unwrap_or(shadow.color);
            frame_decoration = frame_decoration.shadow(shadow);
        }

        let title_bar = build_title_bar(&self.model, design, palette, state);
        let resize = build_resize_regions(&self.model, state, palette.frame_border);

        window_frame()
            .decoration(frame_decoration)
            .overflow(crate::ui::Overflow::Clip)
            .children(title_bar.map(|title_bar| {
                // Use the full inner window contour, not a radius normalized to the shorter
                // title bar. This keeps square controls inside the curved border at any bar height.
                stack()
                    .overflow(crate::ui::Overflow::Clip)
                    .decoration(BoxDecoration::new().corner_radii(inner_radii))
                    .child(title_bar)
            }))
            .children(resize)
            .content_slot(
                window_content_slot()
                    // The root border already reserves its width on every side.
                    .margin(Insets::new(
                        if state.title_bar_visible {
                            design.title_bar.height
                        } else {
                            0.0
                        },
                        0.0,
                        0.0,
                        0.0,
                    ))
                    .overflow(crate::ui::Overflow::Clip)
                    .decoration(
                        BoxDecoration::new()
                            // The host paints content_style() inside the integer client bounds.
                            // A second fill here starts at fractional layout coordinates and can
                            // escape the host's cutout as a dark strip beside the client.
                            .corner_radii(crate::ui::CornerRadii {
                                top_left: if state.title_bar_visible {
                                    0.0
                                } else {
                                    inner_radii.top_left
                                },
                                top_right: if state.title_bar_visible {
                                    0.0
                                } else {
                                    inner_radii.top_right
                                },
                                bottom_right: inner_radii.bottom_right,
                                bottom_left: inner_radii.bottom_left,
                            }),
                    ),
            )
    }
}

impl WindowFrameTemplate for EasyWindowFrame {
    type Component = EasyWindowFrameComponent;

    fn compose(&self, model: WindowChromeModel) -> Self::Component {
        EasyWindowFrameComponent {
            model,
            design: self.design,
        }
    }

    fn motion(&self, _model: &WindowChromeModel) -> Option<crate::WindowMotion> {
        Some(self.design.motion)
    }

    fn content_style(&self, _model: &WindowChromeModel) -> Option<WindowContentStyle> {
        Some(WindowContentStyle {
            background: self.design.content_background,
            // The full-window inner border clip owns rounding; no second aperture is needed.
            corner_radius: 0.0,
            resize_preview: self.design.resize_preview,
        })
    }
}

fn build_title_bar(
    model: &WindowChromeModel,
    design: WindowChromeDesign,
    palette: WindowChromePalette,
    state: WindowChromeStateStyle,
) -> Option<Element> {
    if !state.title_bar_visible {
        return None;
    }

    let title = text(&model.title)
        .font_family(design.title_bar.font_family)
        .size(design.title_bar.title_size)
        .weight(palette.title_weight)
        .color(palette.title_color)
        .window_title();
    let icon = window_icon(model, design.title_bar);
    let controls = window_controls(model, design.controls);
    let mut children = Vec::new();
    if let Some(icon) = icon {
        children.push(icon);
        children.push(spacer().width(design.title_bar.gap).into_element());
    }
    children.push(title.into_element());
    children.push(spacer().width(design.title_bar.gap).into_element());
    children.push(spacer().into_element());
    if !controls.is_empty() {
        children.push(spacer().width(design.title_bar.gap).into_element());
    }
    children.extend(controls);
    // Keep controls in the bar's row so every Dimension resolves against the same parent.
    let title_bar = row()
        .height(design.title_bar.height)
        .padding(design.title_bar.padding)
        .align_items(Alignment::Center)
        .children(children);
    Some(if model.capabilities.move_window {
        title_bar.window_drag_region()
    } else {
        title_bar.into_element()
    })
}

fn window_icon(model: &WindowChromeModel, style: WindowTitleBarStyle) -> Option<Element> {
    if !style.show_client_icon {
        return None;
    }
    let catalog_icon = model.desktop_window_id.and_then(|id| {
        crate::compose::context::provided::<crate::compose::ShellContext>().map(|shell| {
            shell.windows().resolve_icon(
                id,
                crate::compose::IconRequest::new()
                    .logical_size(style.app_icon_size.round().max(1.0) as u32),
            )
        })
    });
    let source = catalog_icon.or_else(|| {
        model
            .app_icon_image
            .map(ImageSource::from)
            .or_else(|| model.app_icon.map(ImageSource::from))
            .or_else(|| style.fallback_app_icon.map(ImageSource::from))
    })?;
    let region = stack()
        .width(style.app_icon_region_size)
        .height(style.app_icon_region_size)
        .center_content()
        .child(
            image(source)
                .width(style.app_icon_size)
                .height(style.app_icon_size)
                .accessible_label("Application icon")
                .window_app_icon(),
        );
    Some(
        if style.app_icon_opens_system_menu && model.capabilities.system_menu {
            crate::compose::PointerViewExt::pointer_icon(
                region.window_system_menu(),
                crate::PointerIcon::Default,
            )
        } else {
            region.into_element()
        },
    )
}

fn window_controls(model: &WindowChromeModel, design: WindowControlsDesign) -> Vec<Element> {
    let mut controls = Vec::with_capacity(3);
    if model.capabilities.minimize {
        controls.push(control("Minimize", design.minimize, WindowAction::Minimize));
    }
    if model.capabilities.maximize {
        let (label, control_design) = if model.state == WindowChromeState::Maximized {
            ("Restore", design.restore)
        } else {
            ("Maximize", design.maximize)
        };
        controls.push(control(label, control_design, WindowAction::ToggleMaximize));
    }
    if model.capabilities.close {
        controls.push(control("Close", design.close, WindowAction::Close));
    }
    let mut children = Vec::with_capacity(5);
    for control in controls {
        if !children.is_empty() {
            children.push(spacer().width(design.gap).into_element());
        }
        children.push(control);
    }
    children
}

fn control(label: &'static str, design: WindowControlDesign, action: WindowAction) -> Element {
    button(label)
        .icon(design.icon)
        .icon_tint(design.style.resting.icon_tint)
        .icon_size(design.style.icon_size)
        .width(design.style.width)
        .height(design.style.height)
        .decoration(design.style.resting.decoration)
        .inline_style(compiled_control_style(design.style))
        .window_action(action)
}

fn compiled_control_style(style: WindowControlButtonStyle) -> Arc<CompiledComponentStyle> {
    let root_slot = StyleSlotId::named("root");
    let icon_slot = StyleSlotId::named("icon");
    let mut root = visual_root_patch(style.resting);
    root.width = Some(style.width.into());
    root.height = Some(style.height.into());
    // Chrome controls follow their authored dimensions, including bars shorter than 32px.
    root.min_size = Some(SizeRule2D {
        width: SizeRule::Logical(0.0),
        height: SizeRule::Logical(0.0),
    });
    let icon = visual_icon_patch(style.resting);

    let mut slots = BTreeMap::new();
    slots.insert(
        root_slot,
        CompiledSlotStyle {
            patch: root,
            font_family: None,
        },
    );
    slots.insert(
        icon_slot,
        CompiledSlotStyle {
            patch: icon,
            font_family: None,
        },
    );

    let mut states = BTreeMap::new();
    for (state, visual) in [
        (InteractionState::Hovered, style.hovered),
        (InteractionState::FocusVisible, style.focused),
        (InteractionState::Pressed, style.pressed),
        (InteractionState::Disabled, style.disabled),
    ] {
        let Some(visual) = visual else { continue };
        states.insert(
            state,
            CompiledStateStyle {
                slots: BTreeMap::from([
                    (
                        root_slot,
                        CompiledSlotStyle {
                            patch: visual_root_patch(visual),
                            font_family: None,
                        },
                    ),
                    (
                        icon_slot,
                        CompiledSlotStyle {
                            patch: visual_icon_patch(visual),
                            font_family: None,
                        },
                    ),
                ]),
                transition: None,
            },
        );
    }

    Arc::new(CompiledComponentStyle {
        id: ComponentStyleId::named(ThemeDomainId::SHELL, "window-control", "inline"),
        slots,
        variants: BTreeMap::new(),
        states,
        state_precedence: vec![
            InteractionState::Hovered,
            InteractionState::FocusVisible,
            InteractionState::Pressed,
            InteractionState::Disabled,
        ],
        relevant_states: InteractionFlags::from_bits(
            InteractionFlags::HOVERED.bits()
                | InteractionFlags::FOCUS_VISIBLE.bits()
                | InteractionFlags::PRESSED.bits()
                | InteractionFlags::DISABLED.bits(),
        ),
        transition: style.transition.unwrap_or_default(),
        controlled_slots: BTreeMap::from([(root_slot, root), (icon_slot, icon)]),
        controlled_font_families: BTreeSet::new(),
    })
}

fn visual_root_patch(visual: WindowControlVisual) -> StylePropertyPatch {
    StylePropertyPatch {
        background: Some(visual.decoration.background),
        border: Some(visual.decoration.border),
        outline: Some(visual.decoration.outline),
        corner_radii: Some(visual.decoration.corner_radii),
        shadows: Some(visual.decoration.shadows),
        ..StylePropertyPatch::default()
    }
}

fn visual_icon_patch(visual: WindowControlVisual) -> StylePropertyPatch {
    StylePropertyPatch {
        image_tint: Some(Some(visual.icon_tint)),
        ..StylePropertyPatch::default()
    }
}

fn build_resize_regions(
    model: &WindowChromeModel,
    style: WindowChromeStateStyle,
    border: Border,
) -> Vec<Element> {
    if !model.capabilities.resize || !style.resize_regions {
        return Vec::new();
    }
    let edges = model
        .tiling
        .map_or(WindowEdgeMask::ALL, |tiling| tiling.resizable_edges);
    let slop = style.resize_hit_slop.0;
    let slop = Insets::new(
        slop.top + (style.resize_edge - border.top.width).max(0.0),
        slop.right + (style.resize_edge - border.right.width).max(0.0),
        slop.bottom + (style.resize_edge - border.bottom.width).max(0.0),
        slop.left + (style.resize_edge - border.left.width).max(0.0),
    );
    // The invisible boxes start at the root's inner edge. Outset restores the entire painted
    // border and adds tolerance outside it; the shared inner contour excludes app/title pixels.
    let region = |view: Element, edge| resize_region(view, edge, slop, border);
    let corner = |a: f32, b: f32| {
        style
            .frame_radius
            .max(style.resize_edge.max(a).max(b) * 2.0)
    };
    let top_right = corner(border.top.width, border.right.width);
    let bottom_right = corner(border.bottom.width, border.right.width);
    let bottom_left = corner(border.bottom.width, border.left.width);
    let top_left = corner(border.top.width, border.left.width);
    let mut regions = Vec::with_capacity(8);
    if edges.contains(WindowEdgeMask::TOP) {
        regions.push(region(
            stack().height(0.0).into_element(),
            WindowResizeEdge::Top,
        ));
    }
    if edges.contains(WindowEdgeMask::RIGHT) {
        regions.push(
            row()
                .child(spacer())
                .child(region(
                    stack().width(0.0).into_element(),
                    WindowResizeEdge::Right,
                ))
                .into_element(),
        );
    }
    if edges.contains(WindowEdgeMask::BOTTOM) {
        regions.push(
            column()
                .child(spacer())
                .child(region(
                    stack().height(0.0).into_element(),
                    WindowResizeEdge::Bottom,
                ))
                .into_element(),
        );
    }
    if edges.contains(WindowEdgeMask::LEFT) {
        regions.push(region(
            stack().width(0.0).into_element(),
            WindowResizeEdge::Left,
        ));
    }
    if edges.contains(WindowEdgeMask::TOP | WindowEdgeMask::RIGHT) {
        regions.push(
            row()
                .child(spacer())
                .child(region(
                    stack().width(top_right).height(top_right).into_element(),
                    WindowResizeEdge::TopRight,
                ))
                .into_element(),
        );
    }
    if edges.contains(WindowEdgeMask::BOTTOM | WindowEdgeMask::RIGHT) {
        regions.push(
            column()
                .child(spacer())
                .child(
                    row().height(bottom_right).child(spacer()).child(region(
                        stack()
                            .width(bottom_right)
                            .height(bottom_right)
                            .into_element(),
                        WindowResizeEdge::BottomRight,
                    )),
                )
                .into_element(),
        );
    }
    if edges.contains(WindowEdgeMask::BOTTOM | WindowEdgeMask::LEFT) {
        regions.push(
            column()
                .child(spacer())
                .child(
                    row()
                        .height(bottom_left)
                        .child(region(
                            stack()
                                .width(bottom_left)
                                .height(bottom_left)
                                .into_element(),
                            WindowResizeEdge::BottomLeft,
                        ))
                        .child(spacer()),
                )
                .into_element(),
        );
    }
    if edges.contains(WindowEdgeMask::TOP | WindowEdgeMask::LEFT) {
        regions.push(region(
            stack().width(top_left).height(top_left).into_element(),
            WindowResizeEdge::TopLeft,
        ));
    }
    regions
}

fn resize_region(
    region: impl View,
    edge: WindowResizeEdge,
    hit_slop: Insets,
    border: Border,
) -> Element {
    region
        .window_resize(edge)
        .window_hit_slop(outward_resize_hit_slop(edge, hit_slop, border))
        .with_window_chrome_border_hit(hit_slop.0)
}

fn outward_resize_hit_slop(edge: WindowResizeEdge, hit_slop: Insets, border: Border) -> Insets {
    let hit_slop = hit_slop.0;
    match edge {
        WindowResizeEdge::Top => Insets::new(hit_slop.top + border.top.width, 0.0, 0.0, 0.0),
        WindowResizeEdge::TopRight => Insets::new(
            hit_slop.top + border.top.width,
            hit_slop.right + border.right.width,
            0.0,
            0.0,
        ),
        WindowResizeEdge::Right => Insets::new(0.0, hit_slop.right + border.right.width, 0.0, 0.0),
        WindowResizeEdge::BottomRight => Insets::new(
            0.0,
            hit_slop.right + border.right.width,
            hit_slop.bottom + border.bottom.width,
            0.0,
        ),
        WindowResizeEdge::Bottom => {
            Insets::new(0.0, 0.0, hit_slop.bottom + border.bottom.width, 0.0)
        }
        WindowResizeEdge::BottomLeft => Insets::new(
            0.0,
            0.0,
            hit_slop.bottom + border.bottom.width,
            hit_slop.left + border.left.width,
        ),
        WindowResizeEdge::Left => Insets::new(0.0, 0.0, 0.0, hit_slop.left + border.left.width),
        WindowResizeEdge::TopLeft => Insets::new(
            hit_slop.top + border.top.width,
            0.0,
            0.0,
            hit_slop.left + border.left.width,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::AssetKey;
    use crate::theme::Easing;

    #[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
    #[test]
    fn shell_catalog_icon_is_centered_and_focus_changes_shadow_color() {
        use crate::application_host::AppRuntimeCore;
        use crate::core::{MonotonicInstant, SizeI};
        use crate::runtime::CompositionDriver;
        use crate::window_chrome::{WindowChromeRole, WindowChromeSnapshot};
        let host = crate::compose::shell_services::ShellServiceHost::new();
        let mut model = WindowChromeModel::new(42, "Editor");
        model.desktop_window_id = Some(crate::shell::WindowId::new(
            std::num::NonZeroU32::new(42).unwrap(),
            std::num::NonZeroU32::new(1).unwrap(),
        ));
        let mut design = DESIGN;
        design.normal.shadow = Some(Shadow {
            offset: crate::PointF { x: 0.0, y: 4.0 },
            blur: 12.0,
            spread: 2.0,
            color: ColorRgba8::rgba(0, 0, 0, 130),
        });
        design.active.shadow_color = Some(ColorRgba8::rgba(0, 0, 0, 210));
        design.inactive.shadow_color = Some(ColorRgba8::rgba(0, 0, 0, 90));
        let mut driver = CompositionDriver::new(easy_window_frame(design).compose(model.clone()));
        driver.connect_shell(host.services.clone());
        let mut runtime = AppRuntimeCore::from_composition_driver(
            driver,
            SizeI {
                width: 640,
                height: 480,
            },
        )
        .unwrap();
        for (index, (active, state)) in [
            (false, WindowChromeState::Normal),
            (true, WindowChromeState::Normal),
            (true, WindowChromeState::Maximized),
        ]
        .into_iter()
        .enumerate()
        {
            model.active = active;
            model.state = state;
            runtime
                .update_composition_root(Box::new(easy_window_frame(design).compose(model.clone())))
                .unwrap();
            runtime
                .prepare_frame(MonotonicInstant::from_nanos(index as u64), true)
                .unwrap();
            let snapshot = WindowChromeSnapshot::derive(runtime.ui(), runtime.layout()).unwrap();
            let icon = snapshot
                .regions
                .iter()
                .find(|r| r.role == WindowChromeRole::AppIcon)
                .expect("catalog fallback icon is visible");
            let border = if state == WindowChromeState::Maximized {
                0.0
            } else {
                design.inactive.frame_border.top.width
            };
            assert!(
                (icon.bounds.y + icon.bounds.height / 2.0
                    - (border + design.title_bar.height / 2.0))
                    .abs()
                    < 0.01
            );
            let decoration = runtime
                .ui()
                .box_styles
                .get(snapshot.frame.node)
                .unwrap()
                .decoration;
            let expected = if state == WindowChromeState::Maximized {
                crate::ui::ShadowList::default()
            } else {
                let mut shadow = design.normal.shadow.unwrap();
                shadow.color = design.palette(active).shadow_color.unwrap();
                crate::ui::ShadowList::one(shadow)
            };
            assert_eq!(decoration.shadows, expected);
        }
    }

    const fn icon(path: &'static str) -> IconAsset {
        IconAsset::new(AssetKey::new(path))
    }

    const VISUAL: WindowControlVisual = WindowControlVisual {
        decoration: BoxDecoration::new(),
        icon_tint: ColorRgba8::rgba(255, 255, 255, 255),
    };
    const BUTTON: WindowControlButtonStyle = WindowControlButtonStyle {
        width: Dimension::Logical(38.0),
        height: Dimension::Logical(30.0),
        icon_size: 15.0,
        resting: VISUAL,
        hovered: None,
        pressed: None,
        focused: None,
        disabled: None,
        transition: None,
    };
    const STATE: WindowChromeStateStyle = WindowChromeStateStyle {
        title_bar_visible: true,
        frame_radius: 12.0,
        shadow: None,
        resize_regions: true,
        resize_edge: 6.0,
        resize_hit_slop: Insets::all(2.0),
    };
    const DESIGN: WindowChromeDesign = WindowChromeDesign {
        motion: crate::WindowMotion::none(),
        active: WindowChromePalette {
            frame_background: ColorRgba8::rgba(20, 24, 32, 255),
            frame_border: Border::all(1.0, ColorRgba8::rgba(80, 90, 120, 255)),
            title_color: ColorRgba8::rgba(255, 255, 255, 255),
            title_weight: 600,
            shadow_color: None,
        },
        inactive: WindowChromePalette {
            frame_background: ColorRgba8::rgba(30, 34, 42, 255),
            frame_border: Border::all(1.0, ColorRgba8::rgba(60, 65, 80, 255)),
            title_color: ColorRgba8::rgba(180, 180, 190, 255),
            title_weight: 400,
            shadow_color: None,
        },
        normal: STATE,
        maximized: WindowChromeStateStyle {
            frame_radius: 0.0,
            shadow: None,
            resize_regions: false,
            resize_edge: 0.0,
            resize_hit_slop: Insets::ZERO,
            ..STATE
        },
        tiled: STATE,
        fullscreen: WindowChromeStateStyle {
            title_bar_visible: false,
            frame_radius: 0.0,
            shadow: None,
            resize_regions: false,
            resize_edge: 0.0,
            resize_hit_slop: Insets::ZERO,
        },
        title_bar: WindowTitleBarStyle {
            font_family: "sans-serif",
            height: 42.0,
            padding: Insets::symmetric(6.0, 8.0),
            gap: 6.0,
            title_size: 14.0,
            app_icon_region_size: 30.0,
            app_icon_size: 20.0,
            show_client_icon: true,
            fallback_app_icon: None,
            app_icon_opens_system_menu: true,
        },
        controls: WindowControlsDesign {
            minimize: WindowControlDesign {
                icon: icon("icons/minimize.svg"),
                style: BUTTON,
            },
            maximize: WindowControlDesign {
                icon: icon("icons/maximize.svg"),
                style: BUTTON,
            },
            restore: WindowControlDesign {
                icon: icon("icons/restore.svg"),
                style: BUTTON,
            },
            close: WindowControlDesign {
                icon: icon("icons/close.svg"),
                style: BUTTON,
            },
            gap: 6.0,
        },
        content_background: ColorRgba8::rgba(10, 12, 18, 255),
        resize_preview: None,
    };

    #[test]
    fn design_validation_accepts_finite_complete_chrome() {
        assert_eq!(DESIGN.validate(), Ok(DESIGN));
    }

    #[test]
    fn resize_preview_border_validation_is_independent_of_frame_border() {
        for width in [-1.0, f32::NAN, f32::INFINITY] {
            let mut preview = crate::ResizePreviewDesign::new(crate::Fill::None);
            preview.border.left.width = width;
            let design = WindowChromeDesign {
                resize_preview: Some(preview),
                ..DESIGN
            };
            assert_eq!(
                design.validate(),
                Err(WindowChromeDesignError::InvalidResizePreviewBorder)
            );
        }
    }

    #[test]
    fn content_style_preserves_alpha_preview_inheritance_and_state_radius() {
        for alpha in [0, 128, 255] {
            for preview in [
                None,
                Some(crate::ResizePreviewDesign::new(crate::Fill::Color(
                    ColorRgba8::rgba(30, 40, 50, alpha),
                ))),
                Some(crate::ResizePreviewDesign::new(crate::Fill::Glass(
                    crate::GlassStyle {
                        tint: ColorRgba8::rgba(30, 40, 50, alpha),
                        blur_radius: 24.0,
                        ..crate::GlassStyle::liquid()
                    },
                ))),
            ] {
                let design = WindowChromeDesign {
                    content_background: ColorRgba8::rgba(0, 0, 0, alpha),
                    resize_preview: preview,
                    ..DESIGN
                };
                assert_eq!(design.validate(), Ok(design));
                for state in [
                    WindowChromeState::Normal,
                    WindowChromeState::Maximized,
                    WindowChromeState::Tiled,
                    WindowChromeState::Fullscreen,
                ] {
                    let style = easy_window_frame(design)
                        .content_style(&WindowChromeModel::new(7, "Editor").state(state))
                        .unwrap();
                    assert_eq!(style.background, design.content_background);
                    assert_eq!(style.resize_preview, preview);
                    assert_eq!(style.corner_radius, 0.0);
                }
            }
        }
    }

    #[test]
    fn template_composes_a_distinct_model_owned_component() {
        let model = WindowChromeModel::new(7, "Editor").active(true);
        let component = easy_window_frame(DESIGN).compose(model.clone());
        assert_eq!(component.model, model);
        assert_eq!(component.design, DESIGN);
    }

    #[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
    #[test]
    fn fractional_content_background_does_not_leak_into_frame_strips() {
        use crate::application_host::AppRuntimeCore;
        use crate::core::{MonotonicInstant, RectI, SizeI};
        use crate::render::RenderBackend;
        use crate::renderer_software::{SoftwareCompositeLayer, SoftwareRenderer, SoftwareSurface};
        use crate::window_chrome::WindowChromeSnapshot;

        let extent = SizeI {
            width: 240,
            height: 120,
        };
        for border in [1.0, 1.25, 1.5, 1.75, 2.5] {
            for title_height in [32.0, 32.5] {
                for title_bar_visible in [true, false] {
                    let mut reference = None;
                    for color in [
                        ColorRgba8::rgba(0, 0, 0, 255),
                        ColorRgba8::rgba(255, 255, 255, 255),
                    ] {
                        let mut design = DESIGN;
                        design.active.frame_border =
                            Border::all(border, design.active.frame_border.top.color);
                        design.title_bar.height = title_height;
                        design.content_background = color;
                        let mut runtime = AppRuntimeCore::from_composed_with_extent(
                            easy_window_frame(design).compose(
                                WindowChromeModel::new(7, "")
                                    .active(true)
                                    .title_bar_visible(title_bar_visible),
                            ),
                            extent,
                        )
                        .unwrap();
                        runtime
                            .prepare_frame(MonotonicInstant::from_nanos(0), true)
                            .unwrap();
                        let snapshot =
                            WindowChromeSnapshot::derive(runtime.ui(), runtime.layout()).unwrap();
                        let content = snapshot.content.bounds;
                        // Match the host's integer client placement and measured size.
                        let cutout = RectI {
                            x: content.x.round() as i32,
                            y: content.y.round() as i32,
                            width: content.width.round() as i32,
                            height: content.height.round() as i32,
                        };
                        let target = RectI {
                            x: 0,
                            y: 0,
                            width: extent.width,
                            height: extent.height,
                        };
                        let mut scene = SoftwareRenderer.create_scene().unwrap();
                        SoftwareRenderer
                            .apply_scene_delta(&mut scene, &runtime.scene_snapshot())
                            .unwrap();
                        let mut surface = SoftwareSurface::default();
                        SoftwareRenderer
                            .render_composite(
                                &mut surface,
                                &[SoftwareCompositeLayer {
                                    scene: &scene,
                                    target,
                                    clip: None,
                                    rounded_clips: [None; 2],
                                }],
                                SizeI {
                                    width: target.width,
                                    height: target.height,
                                },
                                Some(target),
                                ColorRgba8::rgba(0, 255, 0, 255),
                            )
                            .unwrap();
                        let mut strips = Vec::new();
                        for y in 0..target.height {
                            for x in 0..target.width {
                                if x < cutout.x
                                    || x >= cutout.right()
                                    || y < cutout.y
                                    || y >= cutout.bottom()
                                {
                                    let offset = ((y * target.width + x) * 4) as usize;
                                    strips.extend_from_slice(
                                        &surface.pixels_rgba8()[offset..offset + 4],
                                    );
                                }
                            }
                        }
                        if let Some(expected) = &reference {
                            assert!(
                                expected == &strips,
                                "content backing leaked outside client bounds: border={border}, title={title_height}, title_bar_visible={title_bar_visible}"
                            );
                        } else {
                            reference = Some(strips);
                        }
                    }
                }
            }
        }
    }

    #[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
    #[test]
    fn decoration_parts_independently_control_geometry_style_and_resize_hits() {
        use crate::application_host::AppRuntimeCore;
        use crate::core::{MonotonicInstant, SizeI};
        use crate::window_chrome::{WindowAction, WindowChromeRole, WindowChromeSnapshot};
        for bits in 0..16 {
            let parts = crate::WindowFrameParts {
                border: bits & 1 != 0,
                rounded_clip: bits & 2 != 0,
                shadow: bits & 4 != 0,
                resize_regions: bits & 8 != 0,
            };
            for state in [WindowChromeState::Normal, WindowChromeState::Maximized] {
                let mut model = WindowChromeModel::new(42, "Firefox")
                    .title_bar_visible(false)
                    .state(state);
                model.frame_parts = parts;
                let mut design = DESIGN;
                design.normal.shadow = Some(Shadow {
                    offset: crate::PointF::default(),
                    blur: 8.0,
                    spread: 0.0,
                    color: ColorRgba8::rgba(0, 0, 0, 100),
                });
                let mut runtime = AppRuntimeCore::from_composed_with_extent(
                    easy_window_frame(design).compose(model),
                    SizeI {
                        width: 640,
                        height: 480,
                    },
                )
                .unwrap();
                runtime
                    .prepare_frame(MonotonicInstant::from_nanos(0), true)
                    .unwrap();
                let snapshot =
                    WindowChromeSnapshot::derive(runtime.ui(), runtime.layout()).unwrap();
                let normal = state == WindowChromeState::Normal;
                let inset = if parts.border && normal { 1.0 } else { 0.0 };
                assert_eq!(snapshot.content.bounds.x, inset);
                assert_eq!(snapshot.content.bounds.y, inset);
                assert_eq!(snapshot.content.bounds.width, 640.0 - 2.0 * inset);
                let style = runtime.ui().box_styles.get(snapshot.frame.node).unwrap();
                assert_eq!(
                    style.decoration.corner_radii,
                    crate::ui::CornerRadii::all(if parts.rounded_clip && normal {
                        12.0
                    } else {
                        0.0
                    })
                );
                assert_eq!(
                    style.decoration.shadows.as_slice().is_empty(),
                    !(parts.shadow && normal)
                );
                assert_eq!(
                    snapshot.regions.iter().any(|region| matches!(
                        region.role,
                        WindowChromeRole::Action(WindowAction::BeginResize(_))
                    )),
                    parts.resize_regions && normal
                );
            }
        }
    }

    #[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
    #[test]
    fn client_header_keeps_outer_style_and_resize_regions_without_title_controls() {
        use crate::application_host::AppRuntimeCore;
        use crate::core::{MonotonicInstant, SizeI};
        use crate::window_chrome::{WindowAction, WindowChromeRole, WindowChromeSnapshot};
        for active in [false, true] {
            let mut runtime = AppRuntimeCore::from_composed_with_extent(
                easy_window_frame(DESIGN).compose(
                    WindowChromeModel::new(42, "Firefox")
                        .active(active)
                        .title_bar_visible(false),
                ),
                SizeI {
                    width: 640,
                    height: 480,
                },
            )
            .unwrap();
            runtime
                .prepare_frame(MonotonicInstant::from_nanos(0), true)
                .unwrap();
            let snapshot = WindowChromeSnapshot::derive(runtime.ui(), runtime.layout()).unwrap();
            assert_eq!(snapshot.content.bounds.x, 1.0);
            assert_eq!(snapshot.content.bounds.y, 1.0);
            assert_eq!(snapshot.content.bounds.width, 638.0);
            assert_eq!(snapshot.content.bounds.height, 478.0);
            assert!(snapshot.regions.iter().any(|region| matches!(
                region.role,
                WindowChromeRole::Action(WindowAction::BeginResize(_))
            )));
            assert!(!snapshot.regions.iter().any(|region| matches!(
                region.role,
                WindowChromeRole::Title
                    | WindowChromeRole::DragRegion
                    | WindowChromeRole::Action(
                        WindowAction::Close
                            | WindowAction::Minimize
                            | WindowAction::ToggleMaximize
                            | WindowAction::BeginMove
                    )
            )));
            let style = runtime.ui().box_styles.get(snapshot.frame.node).unwrap();
            assert_eq!(
                style.decoration.corner_radii,
                crate::ui::CornerRadii::all(12.0)
            );
        }
    }

    #[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
    #[test]
    fn controls_remain_centered_after_frame_state_updates_without_hover() {
        use crate::application_host::AppRuntimeCore;
        use crate::core::{MonotonicInstant, SizeI};

        let mut design = DESIGN;
        design.title_bar.height = 24.0;
        design.title_bar.padding = Insets::ZERO;
        for control in [
            &mut design.controls.minimize,
            &mut design.controls.maximize,
            &mut design.controls.restore,
            &mut design.controls.close,
        ] {
            control.style.height = Dimension::FILL;
            control.style.icon_size = 12.0;
        }
        let template = easy_window_frame(design);
        let extent = SizeI {
            width: 640,
            height: 480,
        };
        let mut runtime = AppRuntimeCore::from_composed_with_extent(
            template.compose(WindowChromeModel::new(42, "Controls").active(true)),
            extent,
        )
        .unwrap();
        for (iteration, state) in [
            WindowChromeState::Normal,
            WindowChromeState::Maximized,
            WindowChromeState::Normal,
            WindowChromeState::Maximized,
            WindowChromeState::Normal,
        ]
        .into_iter()
        .enumerate()
        {
            runtime
                .update_composition_root(Box::new(
                    template.compose(
                        WindowChromeModel::new(42, "Controls")
                            .active(true)
                            .state(state),
                    ),
                ))
                .unwrap();
            runtime
                .resize(if state == WindowChromeState::Maximized {
                    SizeI {
                        width: 1280,
                        height: 800,
                    }
                } else {
                    extent
                })
                .unwrap();
            runtime
                .prepare_frame(
                    MonotonicInstant::from_nanos(iteration as u64 * 1_000_000),
                    true,
                )
                .unwrap();
            let controls: Vec<_> = runtime
                .ui()
                .style_bindings()
                .iter()
                .filter(|b| {
                    b.local_style.is_some()
                        && b.slots.iter().any(|s| s.slot == StyleSlotId::named("icon"))
                })
                .collect();
            assert_eq!(controls.len(), 3);
            for binding in controls {
                let button = runtime
                    .layout()
                    .computed(binding.state_root)
                    .unwrap()
                    .border_rect;
                assert_eq!(button.height, 24.0, "button height in {state:?}");
                let icon_node = binding
                    .slots
                    .iter()
                    .find(|s| s.slot == StyleSlotId::named("icon"))
                    .unwrap()
                    .node;
                let icon = runtime.layout().computed(icon_node).unwrap().border_rect;
                assert!(
                    (icon.y + icon.height / 2.0 - (button.y + button.height / 2.0)).abs() < 0.001,
                    "icon not centered in {state:?}: icon={icon:?}, button={button:?}"
                );
            }
        }
    }

    #[test]
    fn control_design_resolves_interaction_visuals_without_a_theme_catalog_entry() {
        let hovered = WindowControlVisual {
            decoration: BoxDecoration::new()
                .background(Background::Color(ColorRgba8::rgba(40, 50, 60, 255))),
            icon_tint: ColorRgba8::rgba(10, 20, 30, 255),
        };
        let style = WindowControlButtonStyle {
            hovered: Some(hovered),
            transition: Some(TransitionSpec {
                duration_ms: 90,
                easing: Easing::EaseOut,
                repeat: false,
            }),
            ..BUTTON
        };
        let compiled = compiled_control_style(style);
        let root = compiled
            .resolve_slot(&[], InteractionFlags::HOVERED, StyleSlotId::named("root"))
            .unwrap();
        let icon = compiled
            .resolve_slot(&[], InteractionFlags::HOVERED, StyleSlotId::named("icon"))
            .unwrap();

        assert_eq!(root.patch.background, Some(hovered.decoration.background));
        assert_eq!(icon.patch.image_tint, Some(Some(hovered.icon_tint)));
        assert_eq!(root.transition.duration_ms, 90);
    }
    #[test]
    fn rounded_controls_keep_their_shape_during_hover_and_interrupted_transitions() {
        use crate::application_host::AppRuntimeCore;
        use crate::core::{MonotonicInstant, SizeI};
        use crate::ui::CornerRadii;

        let mut design = DESIGN;
        let radius = CornerRadii::all(7.0);
        for control in [
            &mut design.controls.minimize,
            &mut design.controls.maximize,
            &mut design.controls.restore,
            &mut design.controls.close,
        ] {
            control.style.resting.decoration = BoxDecoration::new()
                .background(Background::Color(ColorRgba8::rgba(30, 40, 50, 255)))
                .corner_radii(radius);
            control.style.hovered = Some(WindowControlVisual {
                decoration: control
                    .style
                    .resting
                    .decoration
                    .background(Background::Color(ColorRgba8::rgba(120, 140, 160, 255))),
                ..control.style.resting
            });
            control.style.transition = Some(TransitionSpec {
                duration_ms: 100,
                easing: Easing::Linear,
                repeat: false,
            });
        }
        let mut runtime = AppRuntimeCore::from_composed_with_extent(
            easy_window_frame(design).compose(WindowChromeModel::new(42, "Rounded")),
            SizeI {
                width: 640,
                height: 480,
            },
        )
        .unwrap();
        runtime.prepare_frame(MonotonicInstant::ZERO, true).unwrap();
        let nodes: Vec<_> = runtime
            .ui()
            .style_bindings()
            .iter()
            .filter(|binding| binding.local_style.is_some())
            .map(|binding| binding.state_root)
            .collect();
        assert_eq!(nodes.len(), 3);
        let mut saw_intermediate_color = false;
        for ms in 1..=350 {
            if let Some(hovered) = match ms {
                1 | 61 => Some(true),
                41 | 201 => Some(false),
                _ => None,
            } {
                for &node in &nodes {
                    runtime.ui_mut().route_interaction_flag(
                        node,
                        InteractionFlags::HOVERED,
                        hovered,
                    );
                }
            }
            runtime
                .prepare_frame(MonotonicInstant::from_nanos(ms * 1_000_000), true)
                .unwrap();
            for &node in &nodes {
                let decoration = runtime.ui().box_styles.get(node).unwrap().decoration;
                assert_eq!(decoration.corner_radii, radius, "radius changed at {ms}ms");
                if let Background::Color(color) = decoration.background {
                    saw_intermediate_color |= color.r > 30 && color.r < 120;
                }
            }
        }
        assert!(
            saw_intermediate_color,
            "the check must sample an active color transition"
        );
    }
    #[cfg(feature = "application-software")]
    #[test]
    fn square_flush_controls_preserve_the_rounded_window_border() {
        use crate::application_host::AppRuntimeCore;
        use crate::core::{MonotonicInstant, RectI, SizeI};
        use crate::render::RenderBackend;
        use crate::renderer_software::{SoftwareCompositeLayer, SoftwareRenderer, SoftwareSurface};

        let extent = SizeI {
            width: 200,
            height: 100,
        };
        for (radius, border) in [(14.0, 2.0), (14.0, 0.0), (0.0, 2.0)] {
            let mut design = DESIGN;
            design.active.frame_border = Border::all(border, ColorRgba8::rgba(255, 0, 0, 255));
            design.active.frame_background = ColorRgba8::rgba(0, 255, 0, 255);
            design.normal.frame_radius = radius;
            design.normal.shadow = None;
            design.title_bar.height = 32.0;
            design.title_bar.padding = Insets::ZERO;
            design.title_bar.show_client_icon = false;
            design.controls.gap = 0.0;
            design.controls.close.style.height = Dimension::FILL;
            design.controls.close.style.resting.decoration = BoxDecoration::new()
                .background(Background::Color(ColorRgba8::rgba(0, 0, 255, 255)));
            let mut runtime = AppRuntimeCore::from_composed_with_extent(
                easy_window_frame(design).compose(WindowChromeModel::new(42, "").active(true)),
                extent,
            )
            .unwrap();
            runtime.prepare_frame(MonotonicInstant::ZERO, true).unwrap();
            let mut scene = SoftwareRenderer.create_scene().unwrap();
            SoftwareRenderer
                .apply_scene_delta(&mut scene, &runtime.scene_snapshot())
                .unwrap();
            let mut surface = SoftwareSurface::default();
            SoftwareRenderer
                .render_composite(
                    &mut surface,
                    &[SoftwareCompositeLayer {
                        scene: &scene,
                        target: RectI {
                            x: 0,
                            y: 0,
                            width: extent.width,
                            height: extent.height,
                        },
                        clip: None,
                        rounded_clips: [None; 2],
                    }],
                    extent,
                    None,
                    ColorRgba8::rgba(0, 0, 0, 0),
                )
                .unwrap();
            let pixel = |x: usize, y: usize| {
                let index = (y * extent.width as usize + x) * 4;
                &surface.pixels_rgba8()[index..index + 4]
            };
            assert_eq!(pixel(195, 20), [0, 0, 255, 255], "button interior");
            if radius > 0.0 {
                assert_eq!(
                    pixel(199, 0),
                    [0, 0, 0, 0],
                    "outer corner must stay transparent"
                );
                if border > 0.0 {
                    assert_eq!(
                        pixel(195, 6)[3],
                        255,
                        "inner antialias seam must remain opaque"
                    );
                    assert!(
                        pixel(195, 6)[2] > 0,
                        "partially covered inner corner must contain button color, not just frame fill"
                    );
                    assert_eq!(
                        pixel(196, 6),
                        [255, 0, 0, 255],
                        "button painted over curved border"
                    );
                    assert_eq!(
                        pixel(194, 6),
                        [0, 0, 255, 255],
                        "button should follow inner curve"
                    );
                }
            }
            if border > 0.0 {
                assert_eq!(pixel(199, 20), [255, 0, 0, 255], "right border");
            }
        }
    }
}

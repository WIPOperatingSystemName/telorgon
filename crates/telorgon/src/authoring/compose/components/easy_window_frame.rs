use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::host::application::WindowFrameTemplate;
use crate::assets::{IconAsset, ImageSource};
use crate::authoring::compose::{
    Alignment, Component, ComponentFields, Dimension, Element, Insets, View, button, column, image,
    row, spacer, stack, text, window_content_slot, window_frame,
};
use crate::foundation::ColorRgba8;
use crate::theme::{
    CompiledComponentStyle, CompiledSlotStyle, CompiledStateStyle, InteractionState, TransitionSpec,
};
use crate::ui::{
    Background, Border, BoxDecoration, ComponentStyleId, InteractionFlags, Shadow, SizeRule,
    SizeRule2D, StylePropertyPatch, StyleSlotId, ThemeDomainId,
};
use crate::shell::window_chrome::{
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
        if self.resize_preview.is_some_and(|preview| {
            !preview.corner_radius.is_finite() || preview.corner_radius < 0.0
        }) {
            return Err(WindowChromeDesignError::InvalidResizePreviewRadius);
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
    #[error("resize preview radius must be finite and nonnegative")]
    InvalidResizePreviewRadius,
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
                    // Content is rectangular. The enclosing frame owns the window contour;
                    // assigning the slot its own radii creates a second rounded rectangle.
                    .overflow(crate::ui::Overflow::Clip),
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
            // Header ownership does not change the backing: transparent client corners
            // must reveal this configured color, not holes through the compositor frame.
            background: self.design.content_background,
            // Shell-owned backing follows the inner border; client pixels keep their alpha.
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
        crate::authoring::compose::context::provided::<crate::authoring::compose::ShellContext>().map(|shell| {
            shell.windows().resolve_icon(
                id,
                crate::authoring::compose::IconRequest::new()
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
            crate::authoring::compose::PointerViewExt::pointer_icon(
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
    button()
        .accessible_label(label)
        .child(crate::compose::image(design.icon)
            .tint(design.style.resting.icon_tint)
            .width(design.style.icon_size)
            .height(design.style.icon_size))
        .content_style_slot(StyleSlotId::named("icon"))
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
mod tests;

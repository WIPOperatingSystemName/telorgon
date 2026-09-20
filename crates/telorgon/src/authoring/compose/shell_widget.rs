//! Component-owned shell surfaces. Geometry is expressed in output-local logical units.
use super::{
    Component, ComponentInstanceId, ErasedComponent, RenderedView, RuntimeTarget, View, ViewError,
};
use crate::foundation::{PointF, RectF, SizeF};
use std::{
    any::{Any, TypeId},
    cell::RefCell,
    rc::Rc,
};

/// A persistent component hosted directly by a shell environment.
pub trait ShellWidget: Component {
    fn surface(&self) -> ShellSurfaceSpec;
    fn children(&self) -> Vec<ShellChild> {
        Vec::new()
    }
    /// Client content slots in this surface's local logical coordinates.
    /// The host aspect-fits retained client content; slots never forward input to clients.
    /// Up to 64 finite slots entirely inside the surface are presented; other slots are ignored.
    fn window_previews(&self) -> Vec<ShellWindowPreview> {
        Vec::new()
    }
    /// Aspect-fitted desktop previews. The host excludes overlay widgets and preview copies,
    /// admits only available outputs, and hides all previews while locked. No capture permission
    /// or client input is granted. At most eight slots are admitted per widget.
    fn output_previews(&self) -> Vec<ShellOutputPreview> {
        Vec::new()
    }
    fn connected(&mut self, _services: super::ShellServices) {}
    fn input(&mut self, _event: crate::input::InputEvent) -> bool {
        false
    }
    /// A request, not an implicit visibility mutation. Change component state to accept it.
    fn dismissed(&mut self, _reason: ShellDismissReason) {}
}

/// A visual-only reference to a live managed window. Stale IDs render no content.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShellWindowPreview {
    pub window: crate::shell::WindowId,
    pub rect: RectF,
}
impl ShellWindowPreview {
    pub const fn new(window: crate::shell::WindowId, rect: RectF) -> Self {
        Self { window, rect }
    }
}

/// A visual-only reference to a hosted output, in widget-local logical coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShellOutputPreview {
    pub output: crate::shell::OutputId,
    pub rect: RectF,
}
impl ShellOutputPreview {
    pub const fn new(output: crate::shell::OutputId, rect: RectF) -> Self {
        Self { output, rect }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellDismissReason {
    Escape,
    OutsidePress,
    AnchorRemoved,
    PointerLeft,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum ShellSurfaceLayer {
    Background,
    #[default]
    Panel,
    Overlay,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellEdge {
    Top,
    Right,
    Bottom,
    Left,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShellFocus {
    #[default]
    None,
    OnClick,
    OnOpen,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShellPointer {
    PassThrough,
    #[default]
    Content,
    Surface,
    Modal,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum ShellReservation {
    #[default]
    None,
    WhenVisible,
    Always,
    Fixed(f32),
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShellPlacementBounds {
    #[default]
    Output,
    WorkArea,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum ShellExtent {
    #[default]
    Fill,
    Logical(f32),
    Content,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShellAttachment {
    Aligned {
        x: f32,
        y: f32,
    },
    Edge(ShellEdge),
    Positioned(PointF),
    /// Attachment to the owning surface, optionally narrowed to a parent-local element rectangle.
    Parent {
        edge: ShellEdge,
        rect: Option<RectF>,
    },
    /// An output-local anchor rectangle. The owner supplies updated geometry reactively.
    Anchored {
        rect: RectF,
        edge: ShellEdge,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WidgetPlacement {
    pub attachment: ShellAttachment,
    pub width: ShellExtent,
    pub height: ShellExtent,
    pub offset: PointF,
    pub margin: f32,
    pub bounds: ShellPlacementBounds,
    pub keep_on_screen: bool,
    pub min_size: SizeF,
    pub max_size: Option<SizeF>,
}
impl Default for WidgetPlacement {
    fn default() -> Self {
        Self::center()
    }
}
impl WidgetPlacement {
    pub const fn center() -> Self {
        Self {
            attachment: ShellAttachment::Aligned { x: 0.5, y: 0.5 },
            width: ShellExtent::Content,
            height: ShellExtent::Content,
            offset: PointF { x: 0.0, y: 0.0 },
            margin: 0.0,
            bounds: ShellPlacementBounds::Output,
            keep_on_screen: true,
            min_size: SizeF {
                width: 1.0,
                height: 1.0,
            },
            max_size: None,
        }
    }
    pub const fn fill() -> Self {
        Self {
            width: ShellExtent::Fill,
            height: ShellExtent::Fill,
            ..Self::center()
        }
    }
    pub const fn edge(edge: ShellEdge) -> Self {
        Self {
            attachment: ShellAttachment::Edge(edge),
            width: match edge {
                ShellEdge::Top | ShellEdge::Bottom => ShellExtent::Fill,
                _ => ShellExtent::Content,
            },
            height: match edge {
                ShellEdge::Left | ShellEdge::Right => ShellExtent::Fill,
                _ => ShellExtent::Content,
            },
            ..Self::center()
        }
    }
    pub const fn aligned(x: f32, y: f32) -> Self {
        Self {
            attachment: ShellAttachment::Aligned { x, y },
            ..Self::center()
        }
    }
    pub const fn positioned(position: PointF) -> Self {
        Self {
            attachment: ShellAttachment::Positioned(position),
            ..Self::center()
        }
    }
    pub const fn attached(edge: ShellEdge) -> Self {
        Self {
            attachment: ShellAttachment::Parent { edge, rect: None },
            ..Self::center()
        }
    }
    pub const fn attached_to(rect: RectF, edge: ShellEdge) -> Self {
        Self {
            attachment: ShellAttachment::Parent {
                edge,
                rect: Some(rect),
            },
            ..Self::center()
        }
    }
    pub const fn anchored(rect: RectF, edge: ShellEdge) -> Self {
        Self {
            attachment: ShellAttachment::Anchored { rect, edge },
            ..Self::center()
        }
    }
    pub const fn width(mut self, width: f32) -> Self {
        self.width = ShellExtent::Logical(width);
        self
    }
    pub const fn height(mut self, height: f32) -> Self {
        self.height = ShellExtent::Logical(height);
        self
    }
    pub const fn width_to_content(mut self) -> Self {
        self.width = ShellExtent::Content;
        self
    }
    pub const fn height_to_content(mut self) -> Self {
        self.height = ShellExtent::Content;
        self
    }
    pub const fn min_size(mut self, size: SizeF) -> Self {
        self.min_size = size;
        self
    }
    pub const fn max_size(mut self, size: SizeF) -> Self {
        self.max_size = Some(size);
        self
    }
    pub const fn keep_on_screen(mut self, value: bool) -> Self {
        self.keep_on_screen = value;
        self
    }
    pub const fn offset(mut self, x: f32, y: f32) -> Self {
        self.offset = PointF { x, y };
        self
    }
    pub const fn margin(mut self, margin: f32) -> Self {
        self.margin = margin;
        self
    }
    pub const fn within(mut self, bounds: ShellPlacementBounds) -> Self {
        self.bounds = bounds;
        self
    }
    pub fn resolve(self, output: RectF, work_area: RectF, content: SizeF) -> RectF {
        let area = if self.bounds == ShellPlacementBounds::WorkArea {
            work_area
        } else {
            output
        };
        let available = SizeF {
            width: (area.width - self.margin * 2.0).max(1.0),
            height: (area.height - self.margin * 2.0).max(1.0),
        };
        let extent = |value, content: f32, available: f32, min: f32, max: f32| {
            let value = match value {
                ShellExtent::Fill => available,
                ShellExtent::Logical(v) => v,
                ShellExtent::Content => content,
            };
            value.max(min).min(max).min(available).max(1.0)
        };
        let maximum = self.max_size.unwrap_or(available);
        let width = extent(
            self.width,
            content.width,
            available.width,
            self.min_size.width,
            maximum.width,
        );
        let height = extent(
            self.height,
            content.height,
            available.height,
            self.min_size.height,
            maximum.height,
        );
        let aligned = |x, y| {
            (
                area.x + self.margin + (available.width - width) * x,
                area.y + self.margin + (available.height - height) * y,
            )
        };
        let (x, y) = match self.attachment {
            ShellAttachment::Aligned { x, y } => aligned(x, y),
            ShellAttachment::Edge(edge) => match edge {
                ShellEdge::Top => aligned(0.5, 0.0),
                ShellEdge::Bottom => aligned(0.5, 1.0),
                ShellEdge::Left => aligned(0.0, 0.5),
                ShellEdge::Right => aligned(1.0, 0.5),
            },
            ShellAttachment::Positioned(p) => (p.x, p.y),
            ShellAttachment::Parent { .. } => aligned(0.5, 0.5),
            ShellAttachment::Anchored { rect, edge } => match edge {
                ShellEdge::Bottom => (
                    rect.x,
                    if rect.y + rect.height + height > area.y + area.height {
                        rect.y - height
                    } else {
                        rect.y + rect.height
                    },
                ),
                ShellEdge::Top => (
                    rect.x,
                    if rect.y - height < area.y {
                        rect.y + rect.height
                    } else {
                        rect.y - height
                    },
                ),
                ShellEdge::Right => (
                    if rect.x + rect.width + width > area.x + area.width {
                        rect.x - width
                    } else {
                        rect.x + rect.width
                    },
                    rect.y,
                ),
                ShellEdge::Left => (
                    if rect.x - width < area.x {
                        rect.x + rect.width
                    } else {
                        rect.x - width
                    },
                    rect.y,
                ),
            },
        };
        let mut result = RectF {
            x: x + self.offset.x,
            y: y + self.offset.y,
            width,
            height,
        };
        if self.keep_on_screen {
            result.x = result.x.clamp(
                area.x + self.margin,
                (area.x + area.width - self.margin - width).max(area.x + self.margin),
            );
            result.y = result.y.clamp(
                area.y + self.margin,
                (area.y + area.height - self.margin - height).max(area.y + self.margin),
            );
        }
        result
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShellSurfaceSpec {
    /// Host-managed snap policy; set by the WindowTiling widget.
    pub tiling: Option<super::WindowTiling>,
    /// None selects the host primary output. An unavailable explicit output hides the surface.
    pub output: Option<crate::shell::OutputId>,
    pub placement: WidgetPlacement,
    pub layer: ShellSurfaceLayer,
    pub order: i32,
    pub visible: bool,
    pub pointer: ShellPointer,
    pub focus: ShellFocus,
    pub reservation: ShellReservation,
    pub dismiss_on_escape: bool,
    pub dismiss_on_outside_press: bool,
    /// Dismiss after leaving this surface and its attachment anchor/connecting gap.
    pub dismiss_on_pointer_leave: bool,
    pub movement: crate::GeometryMotion,
    pub enter_from: Option<ShellEdge>,
    pub exit_to: Option<ShellEdge>,
    /// Group fade and centered shrink, shared with window minimize/restore motion.
    pub visibility_motion: Option<crate::Minimize>,
}
impl Default for ShellSurfaceSpec {
    fn default() -> Self {
        Self::new()
    }
}
impl ShellSurfaceSpec {
    pub const fn new() -> Self {
        Self {
            tiling: None,
            output: None,
            placement: WidgetPlacement::center(),
            layer: ShellSurfaceLayer::Panel,
            order: 0,
            visible: true,
            pointer: ShellPointer::Content,
            focus: ShellFocus::None,
            reservation: ShellReservation::None,
            dismiss_on_escape: false,
            dismiss_on_outside_press: false,
            dismiss_on_pointer_leave: false,
            movement: crate::GeometryMotion::Tween(crate::tween_ms(0, crate::Easing::Linear)),
            enter_from: None,
            exit_to: None,
            visibility_motion: None,
        }
    }
    pub const fn output(mut self, output: crate::shell::OutputId) -> Self {
        self.output = Some(output);
        self
    }
    pub const fn placement(mut self, value: WidgetPlacement) -> Self {
        self.placement = value;
        self
    }
    pub const fn layer(mut self, value: ShellSurfaceLayer) -> Self {
        self.layer = value;
        self
    }
    pub const fn order(mut self, value: i32) -> Self {
        self.order = value;
        self
    }
    pub const fn visible(mut self, value: bool) -> Self {
        self.visible = value;
        self
    }
    pub const fn pointer(mut self, value: ShellPointer) -> Self {
        self.pointer = value;
        self
    }
    pub const fn focus(mut self, value: ShellFocus) -> Self {
        self.focus = value;
        self
    }
    pub const fn reserve_space(mut self, value: ShellReservation) -> Self {
        self.reservation = value;
        self
    }
    pub const fn movement(mut self, value: crate::GeometryMotion) -> Self {
        self.movement = value;
        self
    }
    pub const fn enter_from(mut self, value: ShellEdge) -> Self {
        self.enter_from = Some(value);
        self
    }
    pub const fn exit_to(mut self, value: ShellEdge) -> Self {
        self.exit_to = Some(value);
        self
    }
    pub const fn visibility_motion(mut self, value: crate::Minimize) -> Self {
        self.visibility_motion = Some(value);
        self
    }
    pub const fn dismiss_on_escape(mut self, value: bool) -> Self {
        self.dismiss_on_escape = value;
        self
    }
    pub const fn dismiss_on_outside_press(mut self, value: bool) -> Self {
        self.dismiss_on_outside_press = value;
        self
    }
    pub const fn dismiss_on_pointer_leave(mut self, value: bool) -> Self {
        self.dismiss_on_pointer_leave = value;
        self
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        let p = self.placement;
        let valid_extent = |e| match e {
            ShellExtent::Logical(v) => v.is_finite() && v > 0.0,
            _ => true,
        };
        let size = |s: SizeF| {
            s.width.is_finite() && s.height.is_finite() && s.width > 0.0 && s.height > 0.0
        };
        if !valid_extent(p.width)
            || !valid_extent(p.height)
            || !p.offset.x.is_finite()
            || !p.offset.y.is_finite()
            || !p.margin.is_finite()
            || p.margin < 0.0
            || !size(p.min_size)
            || p.max_size.is_some_and(|s| {
                !size(s) || s.width < p.min_size.width || s.height < p.min_size.height
            })
        {
            return Err("invalid shell surface geometry");
        }
        match p.attachment {
            ShellAttachment::Aligned { x, y }
                if !x.is_finite()
                    || !y.is_finite()
                    || !(0.0..=1.0).contains(&x)
                    || !(0.0..=1.0).contains(&y) =>
            {
                return Err("invalid shell alignment");
            }
            ShellAttachment::Positioned(p) if !p.x.is_finite() || !p.y.is_finite() => {
                return Err("invalid shell position");
            }
            ShellAttachment::Anchored { rect, .. }
            | ShellAttachment::Parent {
                rect: Some(rect), ..
            } if !rect.x.is_finite()
                || !rect.y.is_finite()
                || !size(SizeF {
                    width: rect.width,
                    height: rect.height,
                }) =>
            {
                return Err("invalid shell anchor");
            }
            _ => {}
        }
        if self.reservation != ShellReservation::None
            && (!matches!(p.attachment, ShellAttachment::Edge(_))
                || self.layer != ShellSurfaceLayer::Panel
                || p.bounds != ShellPlacementBounds::Output)
        {
            return Err("only edge panels may reserve work area");
        }
        if matches!(self.reservation,ShellReservation::Fixed(v) if !v.is_finite()||v<0.0) {
            return Err("invalid shell reservation");
        }
        if let crate::GeometryMotion::Spring(s) = self.movement {
            if !s.initial_velocity.is_finite()
                || !s.damping_ratio.is_finite()
                || s.damping_ratio < 0.0
                || !s.angular_frequency.is_finite()
                || s.angular_frequency <= 0.0
            {
                return Err("invalid shell spring");
            }
        }
        Ok(())
    }
}

/// Internal observation shared with the host; no mutable widget is exposed.
#[derive(Clone, Default)]
pub(crate) struct SurfaceBinding(
    pub Rc<RefCell<Option<ShellSurfaceSpec>>>,
    pub Rc<RefCell<Option<Vec<ShellChild>>>>,
    pub Rc<RefCell<Vec<ShellWindowPreview>>>,
    pub Rc<RefCell<Vec<ShellOutputPreview>>>,
);
/// A keyed child surface owned by the declaring widget. It is not a second application root.
pub struct ShellChild {
    pub(crate) key: String,
    pub(crate) root: Box<dyn ErasedComponent>,
    pub(crate) binding: SurfaceBinding,
}
impl ShellChild {
    pub fn new<W: ShellWidget>(key: impl Into<String>, widget: W) -> Self {
        let (root, binding) = erase(widget);
        Self {
            key: key.into(),
            root,
            binding,
        }
    }
}
struct WidgetRoot<W> {
    widget: W,
    binding: SurfaceBinding,
}
pub(crate) fn erase<W: ShellWidget>(widget: W) -> (Box<dyn ErasedComponent>, SurfaceBinding) {
    let binding = SurfaceBinding::default();
    (
        Box::new(WidgetRoot {
            widget,
            binding: binding.clone(),
        }),
        binding,
    )
}
impl<W: ShellWidget> ErasedComponent for WidgetRoot<W> {
    fn component_type_id(&self) -> TypeId {
        TypeId::of::<W>()
    }
    fn component_type_name(&self) -> &'static str {
        std::any::type_name::<W>()
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        &mut self.widget
    }
    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        Box::new(self.widget)
    }
    fn update_from(&mut self, incoming: Box<dyn ErasedComponent>) -> Result<bool, ViewError> {
        self.widget.update_from(incoming)
    }
    fn render(&self, owner: ComponentInstanceId, target: RuntimeTarget) -> RenderedView {
        let ((element, surface, children, previews, outputs), signals) =
            super::context::evaluate::<W, _>(owner, target, || {
                (
                    self.widget.view().into_element(),
                    self.widget.surface(),
                    self.widget.children(),
                    self.widget.window_previews(),
                    self.widget.output_previews(),
                )
            });
        *self.binding.0.borrow_mut() = Some(surface);
        *self.binding.1.borrow_mut() = Some(children);
        *self.binding.2.borrow_mut() = previews
            .into_iter()
            .take(64)
            .filter(|preview| {
                let r = preview.rect;
                [r.x, r.y, r.width, r.height]
                    .into_iter()
                    .all(f32::is_finite)
                    && r.width > 0.0
                    && r.height > 0.0
            })
            .collect();
        *self.binding.3.borrow_mut() = outputs.into_iter().take(8).filter(|preview| {
            let r = preview.rect;
            [r.x, r.y, r.width, r.height].into_iter().all(f32::is_finite)
                && r.width > 0.0 && r.height > 0.0
        }).collect();
        RenderedView { element, signals }
    }
    fn mounted_erased(&mut self, owner: ComponentInstanceId) -> bool {
        self.widget.mounted_erased(owner)
    }
    fn inputs_changed_erased(&mut self, owner: ComponentInstanceId) -> bool {
        self.widget.inputs_changed_erased(owner)
    }
    fn unmounted_erased(&mut self, owner: ComponentInstanceId) {
        self.widget.unmounted_erased(owner);
        *self.binding.0.borrow_mut() = None;
    }
    fn shell_connected(&mut self, services: super::ShellServices) {
        self.widget.connected(services);
    }
    fn shell_input(&mut self, event: crate::input::InputEvent) -> bool {
        let inputs = self.widget.capture_inputs();
        let changed = self.widget.input(event);
        self.widget.restore_inputs(inputs);
        changed
    }
    fn shell_dismissed(&mut self, reason: ShellDismissReason) {
        let inputs = self.widget.capture_inputs();
        self.widget.dismissed(reason);
        self.widget.restore_inputs(inputs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn output() -> RectF {
        RectF {
            x: 0.0,
            y: 0.0,
            width: 1000.0,
            height: 800.0,
        }
    }
    #[test]
    fn edge_center_and_work_area_use_logical_bounds() {
        let work = RectF {
            x: 20.0,
            y: 40.0,
            width: 960.0,
            height: 700.0,
        };
        let p = WidgetPlacement::edge(ShellEdge::Bottom).height(48.0);
        let r = p.resolve(output(), work, SizeF::default());
        assert_eq!(
            r,
            RectF {
                x: 0.0,
                y: 752.0,
                width: 1000.0,
                height: 48.0
            }
        );
        let r = WidgetPlacement::center()
            .width(400.0)
            .height(100.0)
            .within(ShellPlacementBounds::WorkArea)
            .resolve(output(), work, SizeF::default());
        assert_eq!(
            r,
            RectF {
                x: 300.0,
                y: 340.0,
                width: 400.0,
                height: 100.0
            }
        );
    }
    #[test]
    fn attached_popup_flips_before_clamping() {
        let anchor = RectF {
            x: 940.0,
            y: 730.0,
            width: 30.0,
            height: 30.0,
        };
        let r = WidgetPlacement::anchored(anchor, ShellEdge::Bottom)
            .width(200.0)
            .height(120.0)
            .resolve(output(), output(), SizeF::default());
        assert_eq!(
            r,
            RectF {
                x: 800.0,
                y: 610.0,
                width: 200.0,
                height: 120.0
            }
        );
    }
    #[test]
    fn content_size_is_bounded_and_invalid_reservations_reject() {
        let mut p = WidgetPlacement::center();
        p.min_size = SizeF {
            width: 100.0,
            height: 40.0,
        };
        p.max_size = Some(SizeF {
            width: 300.0,
            height: 200.0,
        });
        assert_eq!(
            p.resolve(
                output(),
                output(),
                SizeF {
                    width: 500.0,
                    height: 20.0
                }
            )
            .width,
            300.0
        );
        assert_eq!(
            p.resolve(
                output(),
                output(),
                SizeF {
                    width: 500.0,
                    height: 20.0
                }
            )
            .height,
            40.0
        );
        assert!(
            ShellSurfaceSpec::new()
                .reserve_space(ShellReservation::WhenVisible)
                .validate()
                .is_err()
        );
        assert!(
            ShellSurfaceSpec::new()
                .placement(WidgetPlacement::edge(ShellEdge::Bottom).height(f32::NAN))
                .validate()
                .is_err()
        );
    }
}

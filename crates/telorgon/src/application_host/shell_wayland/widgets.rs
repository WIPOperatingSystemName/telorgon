//! Shell surface lifetime and sampled geometry over the existing retained runtimes.
use super::motion::geometry::{GeometryTrack, Sample};
use super::*;
use crate::compose::{
    ShellAttachment, ShellDismissReason, ShellEdge, ShellFocus, ShellPointer, ShellReservation,
    ShellSurfaceSpec,
};

pub(super) struct WidgetLayer {
    pub id: u32,
    pub parent: Option<u32>,
    parent_bounds: Option<RectI>,
    had_anchor: bool,
    child_key: String,
    component_type: std::any::TypeId,
    pub spec: ShellSurfaceSpec,
    pub layer: Layer,
    binding: crate::compose::shell_widget::SurfaceBinding,
    target: RectI,
    layout_bounds: RectI,
    output_extent: SizeI,
    pub sampled: RectI,
    track: GeometryTrack,
    initialized: bool,
    presented: bool,
    geometry_dirty: bool,
    pub parent_visible: bool,
    pub focused: bool,
    pub captured: bool,
}
impl WidgetLayer {
    pub fn new(
        id: u32,
        widget: crate::application_host::declaration::RegisteredShellWidget,
        output: SizeI,
        assets: AssetBundle,
        scale: crate::platform::ScaleFactor,
        wake: &EventNotifier,
        services: crate::compose::ShellServices,
    ) -> AppResult<Self> {
        let mut driver = widget.content;
        driver.connect_shell(services.clone());
        let wake = wake.clone();
        driver.set_wake(move || wake.notify());
        let layer = Layer::new(driver, output, assets, scale)?;
        let initial = RectI {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        };
        let mut result = Self {
            id,
            parent: None,
            parent_bounds: None,
            had_anchor: false,
            child_key: String::new(),
            component_type: std::any::TypeId::of::<()>(),
            spec: ShellSurfaceSpec::new(),
            layer,
            binding: widget.surface,
            target: initial,
            layout_bounds: initial,
            output_extent: output,
            sampled: initial,
            track: GeometryTrack::fixed(initial),
            initialized: false,
            presented: false,
            geometry_dirty: true,
            parent_visible: true,
            focused: false,
            captured: false,
        };
        result.prepare(output, shell_work_area_for_spec(output), 0)?;
        Ok(result)
    }
    pub fn prepare(&mut self, output: SizeI, work: crate::core::RectF, now: u64) -> AppResult<()> {
        // Flush component/signal updates before observing the surface declaration.
        let extent = self.layer.runtime.extent();
        self.layer.prepare(
            SizeI {
                width: extent.width.max(1.0) as i32,
                height: extent.height.max(1.0) as i32,
            },
            now,
            false,
        )?;
        let mut next = self
            .binding
            .0
            .borrow()
            .as_ref()
            .copied()
            .ok_or_else(|| AppError::new("shell widget has no mounted surface"))?;
        next.validate().map_err(AppError::new)?;
        next.visible &= self.parent_visible
            && next
                .output
                .is_none_or(|id| id == crate::shell::OutputId::MIN);
        let content = self
            .layer
            .runtime
            .ui()
            .root()
            .and_then(|r| {
                self.layer
                    .runtime
                    .ui()
                    .nodes
                    .core(r.0)
                    .and_then(|c| c.first_child)
            })
            .and_then(|n| self.layer.runtime.layout().computed(n))
            .map_or(
                SizeF {
                    width: 1.0,
                    height: 1.0,
                },
                |l| SizeF {
                    width: l.border_rect.width,
                    height: l.border_rect.height,
                },
            );
        let mut placement = next.placement;
        if let ShellAttachment::Parent { edge, rect } = placement.attachment {
            if let Some(parent) = self.parent_bounds {
                self.had_anchor = true;
                let anchor = rect.unwrap_or(crate::core::RectF {
                    x: 0.0,
                    y: 0.0,
                    width: parent.width as f32,
                    height: parent.height as f32,
                });
                placement.attachment = ShellAttachment::Anchored {
                    edge,
                    rect: crate::core::RectF {
                        x: parent.x as f32 + anchor.x,
                        y: parent.y as f32 + anchor.y,
                        ..anchor
                    },
                };
            } else {
                next.visible = false;
                if self.had_anchor {
                    self.had_anchor = false;
                    self.dismiss(ShellDismissReason::AnchorRemoved)?;
                }
            }
        }
        let rect = placement.resolve(shell_work_area_for_spec(output), work, content);
        let layout_target = RectI {
            x: rect.x.round() as i32,
            y: rect.y.round() as i32,
            width: rect.width.round().max(1.0) as i32,
            height: rect.height.round().max(1.0) as i32,
        };
        self.layout_bounds = layout_target;
        self.output_extent = output;
        let target = if next.visible {
            layout_target
        } else {
            outside(layout_target, output, next.exit_to)
        };
        let reduced =
            self.layer.runtime.motion_preference() != crate::theme::MotionPreference::Full;
        let movement = if reduced {
            crate::GeometryMotion::Tween(crate::tween_ms(0, crate::Easing::Linear))
        } else {
            next.movement
        };
        if !self.initialized {
            let from = if next.visible {
                outside(layout_target, output, next.enter_from)
            } else {
                target
            };
            self.track = GeometryTrack::new(Sample::at(from), target, movement, now, false);
        } else if target != self.target
            || next.visible != self.spec.visible
            || (reduced && self.track.pending())
        {
            let sample = self.track.sample(now);
            self.track = GeometryTrack::new(sample, target, movement, now, true);
        }
        if next.visible && !self.spec.visible || !self.initialized && next.visible {
            self.focused = next.focus == ShellFocus::OnOpen;
        }
        if self.spec.visible && !next.visible {
            self.layer
                .runtime
                .deactivate_view(MonotonicInstant::from_nanos(now));
        }
        if !next.visible {
            self.focused = false;
            self.captured = false;
        }
        self.spec = next;
        self.target = target;
        self.initialized = true;
        let before = self.sampled;
        self.sampled = self.track.sample(now).rect();
        self.geometry_dirty |= before != self.sampled || next.visible != self.presented;
        self.track.finish(now);
        self.presented = next.visible || next.exit_to.is_some() && self.track.pending();
        self.layer.prepare(
            SizeI {
                width: layout_target.width,
                height: layout_target.height,
            },
            now,
            false,
        )?;
        Ok(())
    }
    pub fn animating(&self) -> bool {
        self.track.pending()
    }
    pub fn dirty(&self) -> bool {
        self.geometry_dirty || self.layer.has_deltas()
    }
    pub fn scene(&mut self, locked: bool) -> ShellLayer {
        self.geometry_dirty = false;
        let extent = self.layer.runtime.extent();
        let mut scene = ShellLayer::retained(
            ShellLayerKey::Widget(self.id),
            ShellSceneKey::Widget(self.id),
            self.layer.take_deltas(),
            SizeI {
                width: extent.width as i32,
                height: extent.height as i32,
            },
            PointI {
                x: self.sampled.x,
                y: self.sampled.y,
            },
            self.presented && !locked,
        );
        scene.target = self.sampled;
        scene
    }
    pub fn reservation(&self) -> Option<(ShellEdge, i32)> {
        let ShellAttachment::Edge(edge) = self.spec.placement.attachment else {
            return None;
        };
        let amount = match self.spec.reservation {
            ShellReservation::None => return None,
            ShellReservation::WhenVisible if !self.spec.visible => return None,
            ShellReservation::Fixed(v) => v,
            _ => match edge {
                ShellEdge::Top => {
                    self.layout_bounds
                        .y
                        .saturating_add(self.layout_bounds.height) as f32
                }
                ShellEdge::Bottom => {
                    self.output_extent
                        .height
                        .saturating_sub(self.layout_bounds.y) as f32
                }
                ShellEdge::Left => {
                    self.layout_bounds
                        .x
                        .saturating_add(self.layout_bounds.width) as f32
                }
                ShellEdge::Right => {
                    self.output_extent
                        .width
                        .saturating_sub(self.layout_bounds.x) as f32
                }
            },
        };
        Some((edge, amount.round().max(0.0) as i32))
    }
    fn local(&self, p: PointF) -> PointF {
        let size = self.layer.runtime.extent();
        PointF {
            x: (p.x - self.sampled.x as f32) * size.width / self.sampled.width.max(1) as f32,
            y: (p.y - self.sampled.y as f32) * size.height / self.sampled.height.max(1) as f32,
        }
    }
    fn inside_bounds(&self, p: PointF) -> bool {
        let local = self.local(p);
        let size = self.layer.runtime.extent();
        local.x >= 0.0 && local.y >= 0.0 && local.x < size.width && local.y < size.height
    }
    fn input_visible(&self) -> bool {
        self.spec.visible
            && self.parent_visible
            && self.binding.0.borrow().is_some_and(|s| s.visible)
    }
    fn contains(&mut self, p: PointF) -> bool {
        if !self.spec.visible
            || !self.parent_visible
            || !self.binding.0.borrow().is_some_and(|s| s.visible)
            || self.spec.pointer == ShellPointer::PassThrough
        {
            return false;
        }
        if self.spec.pointer == ShellPointer::Modal {
            return true;
        }
        let local = self.local(p);
        let size = self.layer.runtime.extent();
        if local.x < 0.0 || local.y < 0.0 || local.x >= size.width || local.y >= size.height {
            return false;
        }
        self.spec.pointer == ShellPointer::Surface || self.layer.runtime.shell_hit_test(local)
    }
    fn dismiss(&mut self, reason: ShellDismissReason) -> AppResult<()> {
        self.layer.runtime.dismiss_shell_widget(reason)
    }
}
fn outside(mut r: RectI, output: SizeI, edge: Option<ShellEdge>) -> RectI {
    match edge {
        Some(ShellEdge::Top) => r.y = -r.height,
        Some(ShellEdge::Bottom) => r.y = output.height,
        Some(ShellEdge::Left) => r.x = -r.width,
        Some(ShellEdge::Right) => r.x = output.width,
        None => {}
    }
    r
}
fn ordered(widgets: &[WidgetLayer]) -> Vec<usize> {
    let mut order = (0..widgets.len()).collect::<Vec<_>>();
    order.sort_by_key(|i| {
        (
            widgets[*i].spec.layer,
            widgets[*i].spec.order,
            widgets[*i].id,
        )
    });
    order.reverse();
    order
}
pub(super) fn widget_pointer_motion(
    widgets: &mut [WidgetLayer],
    p: PointF,
    now: MonotonicInstant,
    locked: bool,
) -> AppResult<bool> {
    if locked {
        for w in widgets {
            if w.captured || w.focused {
                w.layer.runtime.deactivate_view(now);
            }
            w.captured = false;
            w.focused = false;
        }
        return Ok(false);
    }
    let hit = widgets
        .iter()
        .position(|w| w.captured && w.input_visible())
        .or_else(|| {
            ordered(widgets)
                .into_iter()
                .find(|i| widgets[*i].contains(p))
        });
    for (i, w) in widgets.iter_mut().enumerate() {
        let local = if Some(i) == hit {
            w.local(p)
        } else {
            PointF {
                x: -1_000_000.0,
                y: -1_000_000.0,
            }
        };
        if Some(i) == hit {
            w.layer
                .runtime
                .shell_input(crate::input::InputEvent::mouse_moved(local))?;
        }
        w.layer.pointer_motion(local, now);
    }
    Ok(hit.is_some())
}
pub(super) fn widget_pointer_button(
    widgets: &mut [WidgetLayer],
    p: PointF,
    pressed: bool,
    now: MonotonicInstant,
    locked: bool,
) -> AppResult<bool> {
    if locked {
        return Ok(false);
    }
    let order = ordered(widgets);
    let captured = widgets.iter().position(|w| w.captured && w.input_visible());
    if !pressed && captured.is_none() {
        return Ok(false);
    }
    let hit = captured.or_else(|| order.iter().copied().find(|i| widgets[*i].contains(p)));
    let mut dismissed = false;
    if pressed {
        for i in order {
            if Some(i) == hit {
                break;
            }
            let w = &mut widgets[i];
            if w.spec.visible && w.spec.dismiss_on_outside_press && !w.inside_bounds(p) {
                w.dismiss(ShellDismissReason::OutsidePress)?;
                dismissed = true;
            }
        }
    }
    if let Some(i) = hit {
        if pressed && widgets[i].spec.dismiss_on_outside_press && !widgets[i].inside_bounds(p) {
            widgets[i].dismiss(ShellDismissReason::OutsidePress)?;
            return Ok(true);
        }
        if pressed {
            let mut ancestors = BTreeSet::new();
            let mut parent = widgets[i].parent;
            while let Some(id) = parent {
                ancestors.insert(id);
                parent = widgets.iter().find(|w| w.id == id).and_then(|w| w.parent);
            }
            for (n, w) in widgets.iter_mut().enumerate() {
                let next = (n == i && w.spec.focus != ShellFocus::None)
                    || (ancestors.contains(&w.id) && w.focused);
                if w.focused && !next {
                    w.layer.runtime.deactivate_view(now);
                }
                w.focused = next;
            }
        }
        let w = &mut widgets[i];
        w.layer.pointer_motion(w.local(p), now);
        w.layer
            .runtime
            .shell_input(crate::input::InputEvent::mouse_button(
                crate::input::PointerButton::PRIMARY,
                if pressed {
                    crate::input::ButtonState::Pressed
                } else {
                    crate::input::ButtonState::Released
                },
            ))?;
        w.layer.pointer_button(pressed, now);
        w.captured = pressed;
        Ok(true)
    } else {
        Ok(dismissed)
    }
}
pub(super) fn widget_key(
    widgets: &mut [WidgetLayer],
    event: crate::input::KeyEvent,
    now: MonotonicInstant,
    locked: bool,
) -> AppResult<bool> {
    if locked {
        return Ok(false);
    }
    let Some(i) = ordered(widgets)
        .into_iter()
        .find(|i| widgets[*i].focused && widgets[*i].spec.visible)
    else {
        return Ok(false);
    };
    let w = &mut widgets[i];
    if event.logical_key == crate::input::LogicalKey::Named(crate::input::NamedKey::Escape)
        && event.state == crate::input::ButtonState::Pressed
        && w.spec.dismiss_on_escape
    {
        w.dismiss(ShellDismissReason::Escape)?;
    } else {
        w.layer
            .runtime
            .shell_input(crate::input::InputEvent::Key(event.clone()))?;
        w.layer
            .runtime
            .queue_input(crate::input::InputEvent::Key(event));
        w.layer.runtime.flush_input(now);
    }
    Ok(true)
}

pub(super) fn sync_widget_focus(
    widgets: &mut [WidgetLayer],
    wayland: &mut NativeCompositor<'_>,
    display: &Display,
    windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
    locked: bool,
    saved: &mut Option<WaylandSurfaceId>,
    active: &mut bool,
    now: MonotonicInstant,
) -> AppResult<()> {
    if locked {
        for w in widgets {
            if w.focused || w.captured {
                w.layer.runtime.deactivate_view(now);
            }
            w.focused = false;
            w.captured = false;
        }
        *saved = None;
        *active = false;
        return Ok(());
    }
    let wants = widgets.iter().any(|w| w.spec.visible && w.focused);
    if wants && !*active {
        *saved = wayland
            .core()
            .seats
            .get(&1)
            .and_then(|s| s.keyboard_focus)
            .map(|f| f.surface);
        wayland
            .set_keyboard_focus(1, None, display.next_serial())
            .map_err(app_error)?;
    } else if !wants && *active {
        let restore = saved.take().filter(|id| {
            windows.get(id).is_some_and(|w| !w.minimized)
                && wayland.core().world.surface(*id).is_some()
        });
        wayland
            .set_keyboard_focus(1, restore, display.next_serial())
            .map_err(app_error)?;
    }
    *active = wants;
    Ok(())
}
pub(super) fn widget_keyboard_event(
    event: crate::application_host::ShellKeyEvent,
    pressed: bool,
    text: &str,
) -> crate::input::KeyEvent {
    use crate::input::*;
    let logical = match event.keysym {
        0xff1b => LogicalKey::Named(NamedKey::Escape),
        0xff09 | 0xfe20 => LogicalKey::Named(NamedKey::Tab),
        0xff0d => LogicalKey::Named(NamedKey::Enter),
        0x20 => LogicalKey::Named(NamedKey::Space),
        0xff51 => LogicalKey::Named(NamedKey::ArrowLeft),
        0xff52 => LogicalKey::Named(NamedKey::ArrowUp),
        0xff53 => LogicalKey::Named(NamedKey::ArrowRight),
        0xff54 => LogicalKey::Named(NamedKey::ArrowDown),
        0xff08 => LogicalKey::Named(NamedKey::Backspace),
        0xffff => LogicalKey::Named(NamedKey::Delete),
        _ => LogicalKey::character(text).unwrap_or(LogicalKey::Unidentified),
    };
    let mut modifiers = Modifiers::empty();
    if event.shift {
        modifiers = modifiers.union(Modifiers::SHIFT);
    }
    if event.control {
        modifiers = modifiers.union(Modifiers::CONTROL);
    }
    if event.alt {
        modifiers = modifiers.union(Modifiers::ALT);
    }
    if event.logo {
        modifiers = modifiers.union(Modifiers::SUPER);
    }
    KeyEvent::new(
        PhysicalKey::UNIDENTIFIED,
        if pressed {
            ButtonState::Pressed
        } else {
            ButtonState::Released
        },
    )
    .with_logical_key(logical)
    .with_text(if text.is_empty() {
        None
    } else {
        KeyText::new(text).ok()
    })
    .with_modifiers(modifiers)
}

/// Reconciles keyed child surfaces without treating vector order as component identity.
pub(super) fn sync_widget_children(
    widgets: &mut Vec<WidgetLayer>,
    next_id: &mut u32,
    output: SizeI,
    assets: AssetBundle,
    scale: crate::platform::ScaleFactor,
    wake: &EventNotifier,
    services: &crate::compose::ShellServices,
) -> AppResult<()> {
    let mut index = 0;
    while index < widgets.len() {
        let owner = widgets[index].id;
        let pending = widgets[index].binding.1.borrow_mut().take();
        if let Some(children) = pending {
            if children.len() > 64 {
                return Err(AppError::new("shell widget exceeds 64 child surfaces"));
            }
            let mut keys = BTreeSet::new();
            for child in &children {
                if child.key.is_empty() || child.key.len() > 256 || !keys.insert(child.key.clone())
                {
                    return Err(AppError::new(
                        "shell child keys must be nonempty, bounded, and unique",
                    ));
                }
            }
            let mut removed = widgets
                .iter()
                .filter(|w| {
                    w.parent == Some(owner)
                        && (!keys.contains(&w.child_key)
                            || children.iter().any(|child| {
                                child.key == w.child_key
                                    && child.root.component_type_id() != w.component_type
                            }))
                })
                .map(|w| w.id)
                .collect::<BTreeSet<_>>();
            loop {
                let old = removed.len();
                for w in widgets.iter() {
                    if w.parent.is_some_and(|p| removed.contains(&p)) {
                        removed.insert(w.id);
                    }
                }
                if old == removed.len() {
                    break;
                }
            }
            // Close deepest children first before dropping their parent runtimes.
            for w in widgets.iter_mut().rev().filter(|w| removed.contains(&w.id)) {
                w.layer.runtime.close_composition()?;
            }
            widgets.retain(|w| !removed.contains(&w.id));
            for child in children {
                let ty = child.root.component_type_id();
                if let Some(existing) = widgets
                    .iter_mut()
                    .find(|w| w.parent == Some(owner) && w.child_key == child.key)
                {
                    if existing.component_type == ty {
                        existing.layer.runtime.update_composition_root(child.root)?;
                        continue;
                    }
                }
                if widgets.len() >= 256 {
                    return Err(AppError::new("shell exceeds 256 surfaces"));
                }
                let id = *next_id;
                *next_id = next_id
                    .checked_add(1)
                    .ok_or_else(|| AppError::new("shell surface identity exhausted"))?;
                let registered = crate::application_host::declaration::RegisteredShellWidget {
                    content: CompositionDriver::from_erased_for_target(
                        child.root,
                        crate::compose::RuntimeTarget::ShellWidget,
                    ),
                    surface: child.binding,
                };
                let mut widget = WidgetLayer::new(
                    id,
                    registered,
                    output,
                    assets,
                    scale,
                    wake,
                    services.clone(),
                )?;
                widget.parent = Some(owner);
                widget.child_key = child.key;
                widget.component_type = ty;
                widgets.push(widget);
            }
        }
        index += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compose::{
        Component, ComponentFields, ShellSurfaceLayer, ShellWidget, Signal, SignalWriter, View,
        WidgetPlacement, text,
    };
    struct Fixture {
        placement: Signal<ShellSurfaceSpec>,
        dismissals: std::rc::Rc<std::cell::Cell<u32>>,
    }
    impl ComponentFields for Fixture {
        type InputSnapshot = ();
        fn capture_inputs(&self) {}
        fn restore_inputs(&mut self, _: ()) -> bool {
            false
        }
        fn update_inputs(&mut self, _: Self) -> bool {
            false
        }
    }
    impl Component for Fixture {
        fn view(&self) -> impl View {
            text("shell")
        }
    }
    impl ShellWidget for Fixture {
        fn surface(&self) -> ShellSurfaceSpec {
            *self.watch(&self.placement)
        }
        fn dismissed(&mut self, _: ShellDismissReason) {
            self.dismissals.set(self.dismissals.get() + 1);
        }
        fn input(&mut self, event: crate::input::InputEvent) -> bool {
            if matches!(event, crate::input::InputEvent::PointerMoved { .. }) {
                self.dismissals.set(self.dismissals.get() + 1);
            }
            false
        }
    }
    fn output() -> SizeI {
        SizeI {
            width: 800,
            height: 600,
        }
    }
    fn fixture(
        spec: ShellSurfaceSpec,
    ) -> (
        WidgetLayer,
        SignalWriter<ShellSurfaceSpec>,
        std::rc::Rc<std::cell::Cell<u32>>,
    ) {
        let (signal, writer) = Signal::new(spec);
        let dismissals = std::rc::Rc::new(std::cell::Cell::new(0));
        let (root, binding) = crate::compose::shell_widget::erase(Fixture {
            placement: signal,
            dismissals: dismissals.clone(),
        });
        let registered = crate::application_host::declaration::RegisteredShellWidget {
            content: CompositionDriver::from_erased_for_target(
                root,
                crate::compose::RuntimeTarget::ShellWidget,
            ),
            surface: binding,
        };
        let services = crate::compose::shell_services::ShellServiceHost::new();
        let layer = WidgetLayer::new(
            7,
            registered,
            output(),
            AssetBundle::default(),
            crate::platform::ScaleFactor::new(1.0).unwrap(),
            &EventNotifier::new("widget test").unwrap(),
            services.services.clone(),
        )
        .unwrap();
        (layer, writer, dismissals)
    }
    #[test]
    fn reactive_geometry_keeps_component_identity_and_retargets_motion() {
        let initial = ShellSurfaceSpec::new()
            .placement(WidgetPlacement::edge(ShellEdge::Bottom).height(48.0))
            .movement(crate::GeometryMotion::Spring(crate::Spring::new()));
        let (mut widget, writer, _) = fixture(initial);
        let mounted = widget
            .layer
            .runtime
            .composition_diagnostics()
            .components_mounted;
        writer.publish_if_changed(
            initial.placement(WidgetPlacement::edge(ShellEdge::Top).height(48.0)),
        );
        widget
            .prepare(output(), shell_work_area_for_spec(output()), 10_000_000)
            .unwrap();
        widget
            .prepare(output(), shell_work_area_for_spec(output()), 100_000_000)
            .unwrap();
        let before = widget.sampled;
        writer.publish_if_changed(initial);
        widget
            .prepare(output(), shell_work_area_for_spec(output()), 100_000_000)
            .unwrap();
        assert_eq!(before, widget.sampled);
        assert_eq!(
            widget
                .layer
                .runtime
                .composition_diagnostics()
                .components_mounted,
            mounted
        );
        widget
            .prepare(output(), shell_work_area_for_spec(output()), 900_000_000)
            .unwrap();
        assert_eq!(widget.sampled.y, 552);
        assert!(!widget.animating());
    }
    #[test]
    fn exiting_retains_pixels_but_releases_reservation_and_input() {
        let spec = ShellSurfaceSpec::new()
            .placement(WidgetPlacement::edge(ShellEdge::Bottom).height(48.0))
            .pointer(ShellPointer::Surface)
            .reserve_space(ShellReservation::WhenVisible)
            .movement(crate::GeometryMotion::Tween(crate::tween_ms(
                100,
                crate::Easing::Linear,
            )))
            .exit_to(ShellEdge::Bottom);
        let (mut widget, writer, _) = fixture(spec);
        assert_eq!(widget.reservation(), Some((ShellEdge::Bottom, 48)));
        writer.publish_if_changed(spec.visible(false));
        widget
            .prepare(output(), shell_work_area_for_spec(output()), 1_000_000)
            .unwrap();
        assert!(widget.scene(false).visible);
        assert_eq!(widget.reservation(), None);
        assert!(!widget.contains(PointF { x: 20.0, y: 580.0 }));
        widget
            .prepare(output(), shell_work_area_for_spec(output()), 101_000_000)
            .unwrap();
        assert!(!widget.scene(false).visible);
    }
    #[test]
    fn dismissal_is_a_request_and_lock_hides_surfaces() {
        let spec = ShellSurfaceSpec::new()
            .placement(WidgetPlacement::center().width(300.0).height(100.0))
            .layer(ShellSurfaceLayer::Overlay)
            .focus(ShellFocus::OnOpen)
            .dismiss_on_escape(true);
        let (widget, _, count) = fixture(spec);
        let mut widgets = vec![widget];
        let event = crate::input::KeyEvent::new(
            crate::input::PhysicalKey::UNIDENTIFIED,
            crate::input::ButtonState::Pressed,
        )
        .with_logical_key(crate::input::LogicalKey::Named(
            crate::input::NamedKey::Escape,
        ));
        assert!(widget_key(&mut widgets, event, MonotonicInstant::from_nanos(0), false).unwrap());
        assert_eq!(count.get(), 1);
        assert!(widgets[0].spec.visible);
        assert!(!widgets[0].scene(true).visible);
    }
    #[test]
    fn captured_pointer_stays_with_owner_above_another_surface() {
        let base = ShellSurfaceSpec::new()
            .placement(
                WidgetPlacement::positioned(PointF { x: 0.0, y: 0.0 })
                    .width(100.0)
                    .height(100.0),
            )
            .pointer(ShellPointer::Surface);
        let (left, _, left_events) = fixture(base);
        let (mut right, _, right_events) = fixture(
            base.placement(
                WidgetPlacement::positioned(PointF { x: 200.0, y: 0.0 })
                    .width(100.0)
                    .height(100.0),
            )
            .layer(ShellSurfaceLayer::Overlay),
        );
        right.id = 8;
        let mut widgets = vec![left, right];
        let now = MonotonicInstant::from_nanos(0);
        assert!(
            widget_pointer_button(&mut widgets, PointF { x: 20.0, y: 20.0 }, true, now, false)
                .unwrap()
        );
        assert!(
            widget_pointer_motion(&mut widgets, PointF { x: 220.0, y: 20.0 }, now, false).unwrap()
        );
        assert_eq!(left_events.get(), 1);
        assert_eq!(right_events.get(), 0);
        assert!(widgets[0].captured);
        assert!(!widgets[1].captured);
    }
    #[test]
    fn content_sizing_and_overlapping_reservations_do_not_feed_back() {
        let (mut content, _, _) = fixture(ShellSurfaceSpec::new());
        let first = content.sampled;
        assert!(first.width > 1 && first.width < 200);
        assert!(first.height > 1 && first.height < 100);
        content
            .prepare(output(), shell_work_area_for_spec(output()), 1)
            .unwrap();
        assert_eq!(content.sampled, first);
        let panel = ShellSurfaceSpec::new()
            .placement(
                WidgetPlacement::edge(ShellEdge::Bottom)
                    .height(48.0)
                    .margin(10.0),
            )
            .reserve_space(ShellReservation::WhenVisible);
        let (a, _, _) = fixture(panel);
        let (b, _, _) = fixture(panel);
        assert_eq!(shell_work_area(output(), &[a, b]).height, 542);
    }
    #[test]
    fn reduced_motion_reaches_target_without_animation() {
        let spec = ShellSurfaceSpec::new()
            .placement(WidgetPlacement::edge(ShellEdge::Bottom).height(48.0))
            .movement(crate::GeometryMotion::Spring(crate::Spring::new()));
        let (mut widget, writer, _) = fixture(spec);
        widget
            .layer
            .runtime
            .set_motion_preference(crate::theme::MotionPreference::Reduced);
        writer
            .publish_if_changed(spec.placement(WidgetPlacement::edge(ShellEdge::Top).height(48.0)));
        widget
            .prepare(output(), shell_work_area_for_spec(output()), 1)
            .unwrap();
        assert_eq!(widget.sampled.y, 0);
        assert!(!widget.animating());
    }
}

pub(super) fn prepare_widget_surfaces(
    widgets: &mut [WidgetLayer],
    output: SizeI,
    work: crate::core::RectF,
    now: u64,
    motion: crate::theme::MotionPreference,
) -> AppResult<()> {
    let mut parents = BTreeMap::new();
    for widget in widgets {
        widget.parent_visible = widget
            .parent
            .is_none_or(|p| parents.get(&p).is_some_and(|(_, visible)| *visible));
        widget.parent_bounds = widget
            .parent
            .and_then(|p| parents.get(&p).map(|(bounds, _)| *bounds));
        widget.layer.runtime.set_motion_preference(motion);
        widget.prepare(output, work, now)?;
        parents.insert(widget.id, (widget.sampled, widget.spec.visible));
    }
    Ok(())
}

#[cfg(test)]
mod child_tests {
    use super::*;
    use crate::compose::*;
    use std::{cell::Cell, rc::Rc};
    struct Parent {
        items: Signal<Vec<u32>>,
        mounted: Rc<Cell<u32>>,
        unmounted: Rc<Cell<u32>>,
    }
    struct Child {
        mounted: Rc<Cell<u32>>,
        unmounted: Rc<Cell<u32>>,
    }
    macro_rules! fields {
        ($t:ty) => {
            impl ComponentFields for $t {
                type InputSnapshot = ();
                fn capture_inputs(&self) {}
                fn restore_inputs(&mut self, _: ()) -> bool {
                    false
                }
                fn update_inputs(&mut self, _: Self) -> bool {
                    false
                }
            }
        };
    }
    fields!(Parent);
    fields!(Child);
    impl Component for Parent {
        fn view(&self) -> impl View {
            text("parent")
        }
    }
    impl Component for Child {
        fn view(&self) -> impl View {
            text("child")
        }
        fn mounted(&mut self, _: &mut MountContext<Self>) {
            self.mounted.set(self.mounted.get() + 1);
        }
        fn unmounted(&mut self, _: &mut UnmountContext<Self>) {
            self.unmounted.set(self.unmounted.get() + 1);
        }
    }
    impl ShellWidget for Parent {
        fn surface(&self) -> ShellSurfaceSpec {
            ShellSurfaceSpec::new().placement(WidgetPlacement::center().width(200.0).height(80.0))
        }
        fn children(&self) -> Vec<ShellChild> {
            self.watch(&self.items)
                .iter()
                .map(|id| {
                    ShellChild::new(
                        id.to_string(),
                        Child {
                            mounted: self.mounted.clone(),
                            unmounted: self.unmounted.clone(),
                        },
                    )
                })
                .collect()
        }
    }
    impl ShellWidget for Child {
        fn surface(&self) -> ShellSurfaceSpec {
            ShellSurfaceSpec::new()
                .placement(
                    WidgetPlacement::attached(ShellEdge::Bottom)
                        .width(100.0)
                        .height(40.0),
                )
                .layer(ShellSurfaceLayer::Overlay)
        }
    }
    #[test]
    fn keyed_children_reorder_without_remount_and_unmount_on_removal() {
        let (items, writer) = Signal::new(vec![1, 2]);
        let mounted = Rc::new(Cell::new(0));
        let unmounted = Rc::new(Cell::new(0));
        let (root, surface) = crate::compose::shell_widget::erase(Parent {
            items,
            mounted: mounted.clone(),
            unmounted: unmounted.clone(),
        });
        let output = SizeI {
            width: 800,
            height: 600,
        };
        let scale = crate::platform::ScaleFactor::new(1.0).unwrap();
        let wake = EventNotifier::new("child test").unwrap();
        let host = crate::compose::shell_services::ShellServiceHost::new();
        let registered = crate::application_host::declaration::RegisteredShellWidget {
            content: CompositionDriver::from_erased_for_target(root, RuntimeTarget::ShellWidget),
            surface,
        };
        let mut widgets = vec![
            WidgetLayer::new(
                0,
                registered,
                output,
                AssetBundle::default(),
                scale,
                &wake,
                host.services.clone(),
            )
            .unwrap(),
        ];
        let mut next = 1;
        sync_widget_children(
            &mut widgets,
            &mut next,
            output,
            AssetBundle::default(),
            scale,
            &wake,
            &host.services,
        )
        .unwrap();
        assert_eq!(mounted.get(), 2);
        let first = widgets.iter().find(|w| w.child_key == "1").unwrap().id;
        writer.publish_if_changed(vec![2, 1]);
        prepare_widget_surfaces(
            &mut widgets,
            output,
            shell_work_area_for_spec(output),
            1,
            crate::theme::MotionPreference::Full,
        )
        .unwrap();
        sync_widget_children(
            &mut widgets,
            &mut next,
            output,
            AssetBundle::default(),
            scale,
            &wake,
            &host.services,
        )
        .unwrap();
        assert_eq!(mounted.get(), 2);
        assert_eq!(
            widgets.iter().find(|w| w.child_key == "1").unwrap().id,
            first
        );
        writer.publish_if_changed(vec![2]);
        prepare_widget_surfaces(
            &mut widgets,
            output,
            shell_work_area_for_spec(output),
            2,
            crate::theme::MotionPreference::Full,
        )
        .unwrap();
        sync_widget_children(
            &mut widgets,
            &mut next,
            output,
            AssetBundle::default(),
            scale,
            &wake,
            &host.services,
        )
        .unwrap();
        assert_eq!(unmounted.get(), 1);
        assert_eq!(widgets.len(), 2);
        let child = &widgets[1];
        assert_eq!(child.sampled.y, 340);
    }
}

impl Drop for WidgetLayer {
    fn drop(&mut self) {
        let _ = self.layer.runtime.close_composition();
    }
}

#[cfg(test)]
mod taskbar_context_tests {
    use super::*;
    use crate::compose::*;
    use std::{cell::RefCell, num::NonZeroU32, rc::Rc};
    type Seen = Rc<RefCell<Vec<(crate::shell::WindowId, ImageId)>>>;
    struct Panel(Seen);
    struct Entries(Seen);
    macro_rules! fields {
        ($t:ty) => {
            impl ComponentFields for $t {
                type InputSnapshot = ();
                fn capture_inputs(&self) {}
                fn restore_inputs(&mut self, _: ()) -> bool {
                    false
                }
                fn update_inputs(&mut self, _: Self) -> bool {
                    false
                }
            }
        };
    }
    fields!(Panel);
    fields!(Entries);
    impl Component for Panel {
        fn view(&self) -> impl View {
            Entries(self.0.clone())
        }
    }
    impl ShellWidget for Panel {
        fn surface(&self) -> ShellSurfaceSpec {
            ShellSurfaceSpec::new().placement(WidgetPlacement::fill())
        }
    }
    impl Component for Entries {
        fn view(&self) -> impl View {
            let windows = self.context::<ShellContext>().windows();
            let mut row = row();
            let mut seen = Vec::new();
            for window in windows.open() {
                let id = window.id;
                let icon = windows.icon(id);
                seen.push((id, icon.image_id()));
                row = row.child(
                    button(window.title)
                        .key(format!("{}:{}", id.slot(), id.generation()))
                        .icon(icon)
                        .width(40.0)
                        .height(40.0)
                        .on_press(move |this: &mut Self| {
                            this.context::<ShellContext>()
                                .windows()
                                .activate(id)
                                .unwrap();
                        }),
                );
            }
            *self.0.borrow_mut() = seen;
            row
        }
    }
    fn window(slot: u32) -> ShellWindow {
        ShellWindow {
            id: crate::shell::WindowId::new(
                NonZeroU32::new(slot).unwrap(),
                NonZeroU32::new(1).unwrap(),
            ),
            title: format!("Window {slot}"),
            application_id: None,
            application_identity: String::new(),
            icon: None,
            icon_name: None,
            active: false,
            minimized: false,
            maximized: false,
        }
    }
    #[test]
    fn descendant_taskbar_tracks_existing_open_closed_and_delayed_icons_and_dispatches_actions() {
        let host = crate::compose::shell_services::ShellServiceHost::new();
        host.publish(vec![window(1)]);
        let seen = Rc::new(RefCell::new(Vec::new()));
        let (root, binding) = crate::compose::shell_widget::erase(Panel(seen.clone()));
        let output = SizeI {
            width: 800,
            height: 600,
        };
        let layer = WidgetLayer::new(
            1,
            crate::application_host::declaration::RegisteredShellWidget {
                content: CompositionDriver::from_erased_for_target(
                    root,
                    RuntimeTarget::ShellWidget,
                ),
                surface: binding,
            },
            output,
            AssetBundle::default(),
            crate::platform::ScaleFactor::new(1.0).unwrap(),
            &EventNotifier::new("taskbar-test").unwrap(),
            host.services.clone(),
        )
        .unwrap();
        let mut layers = vec![layer];
        let mut retained = crate::renderer_vulkan::VulkanScene::default();
        let admit = |layer: &mut WidgetLayer,
                     retained: &mut crate::renderer_vulkan::VulkanScene| {
            for delta in layer.layer.take_deltas() {
                retained.apply_delta_checked(&delta).unwrap();
            }
        };
        admit(&mut layers[0], &mut retained);
        assert_eq!(seen.borrow().len(), 1);
        let now = MonotonicInstant::from_nanos(1);
        widget_pointer_button(&mut layers, PointF { x: 20.0, y: 20.0 }, true, now, false).unwrap();
        widget_pointer_button(&mut layers, PointF { x: 20.0, y: 20.0 }, false, now, false).unwrap();
        layers[0]
            .prepare(output, shell_work_area_for_spec(output), 2)
            .unwrap();
        assert_eq!(host.drain()[0].window, window(1).id);
        let mut second = window(2);
        second.minimized = true;
        host.publish(vec![window(1), second.clone()]);
        layers[0]
            .prepare(output, shell_work_area_for_spec(output), 3)
            .unwrap();
        admit(&mut layers[0], &mut retained);
        assert_eq!(seen.borrow().len(), 2);
        let mut icon = crate::compose::applications::fallback_image();
        icon.image = ImageId(0x6800_ffff);
        second.icon = Some(icon.clone());
        host.publish(vec![second]);
        layers[0]
            .prepare(output, shell_work_area_for_spec(output), 4)
            .unwrap();
        admit(&mut layers[0], &mut retained);
        assert_eq!(seen.borrow().as_slice(), &[(window(2).id, icon.image)]);
        host.publish(Vec::new());
        layers[0]
            .prepare(output, shell_work_area_for_spec(output), 5)
            .unwrap();
        admit(&mut layers[0], &mut retained);
        assert!(seen.borrow().is_empty());
    }
}

#[cfg(test)]
mod icon_publication_tests {
    use super::*;
    use crate::compose::*;
    use std::{num::NonZeroU32, rc::Rc};
    struct LateIcon {
        host: Rc<crate::compose::shell_services::ShellServiceHost>,
        next: Signal<Option<ShellWindow>>,
    }
    impl ComponentFields for LateIcon {
        type InputSnapshot = ();
        fn capture_inputs(&self) {}
        fn restore_inputs(&mut self, _: ()) -> bool {
            false
        }
        fn update_inputs(&mut self, _: Self) -> bool {
            false
        }
    }
    impl Component for LateIcon {
        fn view(&self) -> impl View {
            // Simulate an icon completion after the host's pre-view resource sync.
            if let Some(window) = self.watch(&self.next).as_ref() {
                self.host.publish(vec![window.clone()]);
            }
            let windows = self.context::<ShellContext>().windows();
            let window = windows.open().remove(0);
            button(window.title).icon(windows.icon(window.id))
        }
    }
    impl ShellWidget for LateIcon {
        fn surface(&self) -> ShellSurfaceSpec {
            ShellSurfaceSpec::new().placement(WidgetPlacement::fill())
        }
    }
    #[test]
    fn icon_published_during_view_is_in_the_same_delta_as_its_draw() {
        let host = Rc::new(crate::compose::shell_services::ShellServiceHost::new());
        let mut window = ShellWindow {
            id: crate::shell::WindowId::new(
                NonZeroU32::new(1).unwrap(),
                NonZeroU32::new(1).unwrap(),
            ),
            title: "App".into(),
            application_id: None,
            application_identity: String::new(),
            icon: None,
            icon_name: None,
            active: false,
            minimized: false,
            maximized: false,
        };
        host.publish(vec![window.clone()]);
        let (next, writer) = Signal::new(None);
        let (root, binding) = crate::compose::shell_widget::erase(LateIcon {
            host: host.clone(),
            next,
        });
        let output = SizeI {
            width: 800,
            height: 600,
        };
        let mut layer = WidgetLayer::new(
            1,
            crate::application_host::declaration::RegisteredShellWidget {
                content: CompositionDriver::from_erased_for_target(
                    root,
                    RuntimeTarget::ShellWidget,
                ),
                surface: binding,
            },
            output,
            AssetBundle::default(),
            crate::platform::ScaleFactor::new(1.0).unwrap(),
            &EventNotifier::new("late-icon-test").unwrap(),
            host.services.clone(),
        )
        .unwrap();
        let mut retained = crate::renderer_vulkan::VulkanScene::default();
        for delta in layer.layer.take_deltas() {
            retained
                .apply_delta_checked(&delta)
                .unwrap_or_else(|e| panic!("{e:?} delta={delta:?}"));
        }
        let mut icon = crate::compose::applications::fallback_image();
        icon.image = ImageId(0x6800_ffff);
        window.icon = Some(icon);
        writer.publish(Some(window));
        layer
            .prepare(output, shell_work_area_for_spec(output), 1)
            .unwrap();
        for delta in layer.layer.take_deltas() {
            retained
                .apply_delta_checked(&delta)
                .unwrap_or_else(|e| panic!("{e:?} delta={delta:?}"));
        }
    }
}

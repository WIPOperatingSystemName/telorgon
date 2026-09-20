//! Shell surface lifetime and sampled geometry over the existing retained runtimes.
mod output_previews;
#[cfg(all(test, feature = "shell-screencast-linux"))]
mod capture_tests;
use super::motion::geometry::{GeometryTrack, Sample};
use super::scene::ShellLayerContent;
use super::*;
use crate::authoring::compose::{
    ShellAttachment, ShellDismissReason, ShellEdge, ShellFocus, ShellPointer, ShellReservation,
    ShellSurfaceSpec,
};

pub(super) struct WidgetLayer {
    pub id: u32,
    pub tile_preview: Option<(WaylandSurfaceId, RectI)>,
    pub tile_preview_owner: Option<WaylandSurfaceId>,
    pub parent: Option<u32>,
    parent_bounds: Option<RectI>,
    had_anchor: bool,
    child_key: String,
    component_type: std::any::TypeId,
    pub spec: ShellSurfaceSpec,
    pub layer: Layer,
    binding: crate::authoring::compose::shell_widget::SurfaceBinding,
    target: RectI,
    layout_bounds: RectI,
    output_extent: SizeI,
    pub sampled: RectI,
    track: GeometryTrack,
    initialized: bool,
    presented: bool,
    geometry_dirty: bool,
    previous_previews: Vec<crate::authoring::compose::ShellWindowPreview>,
    previous_output_previews: Vec<crate::authoring::compose::ShellOutputPreview>,
    pub parent_visible: bool,
    pub focused: bool,
    pub captured: bool,
    pointer_hovered: bool,
    retiring: bool,
    visibility: super::motion::Track,
    pub opacity: f32,
    visibility_pending: bool,
}
impl WidgetLayer {
    pub fn new(
        id: u32,
        widget: crate::host::application::declaration::RegisteredShellWidget,
        output: SizeI,
        assets: AssetBundle,
        scale: crate::platform::contracts::ScaleFactor,
        wake: &EventNotifier,
        services: crate::authoring::compose::ShellServices,
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
            tile_preview: None,
            tile_preview_owner: None,
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
            previous_previews: Vec::new(),
            previous_output_previews: Vec::new(),
            parent_visible: true,
            focused: false,
            captured: false,
            pointer_hovered: false,
            retiring: false,
            visibility: super::motion::Track::fixed(0.0),
            opacity: 0.0,
            visibility_pending: false,
        };
        result.prepare(output, shell_work_area_for_spec(output), 0)?;
        Ok(result)
    }
    pub fn prepare(&mut self, output: SizeI, work: crate::foundation::RectF, now: u64) -> AppResult<()> {
        // Flush component/signal updates before observing the surface declaration.
        let extent = self.layer.runtime.extent();
        if !self.retiring {
            self.layer.prepare(
                SizeI {
                    width: extent.width.max(1.0) as i32,
                    height: extent.height.max(1.0) as i32,
                },
                now,
                false,
            )?;
        }
        let mut next = if self.retiring {
            self.spec
        } else {
            self.binding
                .0
                .borrow()
                .as_ref()
                .copied()
                .ok_or_else(|| AppError::new("shell widget has no mounted surface"))?
        };
        if let Some(tiling) = next.tiling {
            next.visible = self.tile_preview.is_some() && !self.retiring;
            let rect = self
                .tile_preview
                .map(|(_, r)| r)
                .unwrap_or(self.layout_bounds);
            next.placement = crate::WidgetPlacement::positioned(PointF {
                x: rect.x as f32,
                y: rect.y as f32,
            })
            .width(rect.width.max(1) as f32)
            .height(rect.height.max(1) as f32);
            next.movement = tiling.preview.motion.relocate;
            next.visibility_motion = Some(crate::Minimize {
                tween: if next.visible {
                    tiling.preview.motion.appear
                } else {
                    tiling.preview.motion.disappear
                },
            });
        }
        next.validate().map_err(AppError::new)?;
        next.visible &= !self.retiring
            && self.parent_visible
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
                let anchor = rect.unwrap_or(crate::foundation::RectF {
                    x: 0.0,
                    y: 0.0,
                    width: parent.width as f32,
                    height: parent.height as f32,
                });
                placement.attachment = ShellAttachment::Anchored {
                    edge,
                    rect: crate::foundation::RectF {
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
        let opacity_target = if next.visible { 1.0 } else { 0.0 };
        if !self.initialized
            || next.visible != self.spec.visible
            || next.visibility_motion != self.spec.visibility_motion
            || reduced
        {
            let tween = if reduced {
                crate::tween_ms(0, crate::Easing::Linear)
            } else {
                next.visibility_motion
                    .map_or(crate::tween_ms(0, crate::Easing::Linear), |motion| {
                        motion.tween
                    })
            };
            self.visibility.retarget(opacity_target, tween, now);
        }
        if !self.initialized
            || next.tiling.is_some() && next.visible && !self.spec.visible && self.opacity <= 0.0
        {
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
        let previous_opacity = self.opacity;
        self.opacity = self.visibility.sample(now);
        self.visibility_pending = self.visibility.active(now);
        self.sampled = self.track.sample(now).rect();
        if next.visibility_motion.is_some() && next.tiling.is_none() {
            self.sampled = super::motion::minimize_rect(self.sampled, self.opacity);
        }
        self.geometry_dirty |= before != self.sampled
            || next.visible != self.presented
            || previous_opacity != self.opacity;
        self.track.finish(now);
        self.presented = next.visible
            || next.exit_to.is_some() && self.track.pending()
            || self.visibility_pending;
        if !self.retiring {
            self.layer.prepare(
                SizeI {
                    width: layout_target.width,
                    height: layout_target.height,
                },
                now,
                false,
            )?;
        }
        let previews = self.binding.2.borrow().clone();
        self.geometry_dirty |= self.previous_previews != previews;
        self.previous_previews = previews;
        let outputs = self.binding.3.borrow().clone();
        self.geometry_dirty |= self.previous_output_previews != outputs;
        self.previous_output_previews = outputs;
        Ok(())
    }
    pub fn animating(&self) -> bool {
        self.track.pending() || self.visibility_pending
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
    pub fn tiling_policy(&self) -> Option<crate::WindowTiling> {
        if self.retiring
            || !self.parent_visible
            || self
                .spec
                .output
                .is_some_and(|o| o != crate::shell::OutputId::MIN)
        {
            None
        } else {
            self.spec.tiling
        }
    }
    pub fn tile_material(&self, locked: bool) -> Option<ShellLayer> {
        let design = self.spec.tiling?.preview;
        if locked || !self.presented {
            return None;
        }
        let mut layer = ShellLayer::solid(
            ShellLayerKey::TilePreview(self.id),
            ShellSceneKey::TilePreview(self.id),
            design.fill.color(),
            self.sampled,
        );
        layer.rounded_clips = [
            Some(
                crate::graphics::render::RoundedClip::new(
                    crate::RectF {
                        x: self.sampled.x as f32,
                        y: self.sampled.y as f32,
                        width: self.sampled.width as f32,
                        height: self.sampled.height as f32,
                    },
                    crate::ui::CornerRadii::all(design.corner_radius),
                )
                .inset(design.border),
            ),
            None,
        ];
        if let crate::Fill::Glass(style) = design.fill {
            layer.glass = Some(style.normalized());
        }
        Some(layer)
    }
    pub fn preview_layers(
        &self,
        windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
        sources: &[ShellLayer],
        locked: bool,
    ) -> Vec<ShellLayer> {
        if locked || !self.presented {
            return Vec::new();
        }
        let mut layers = Vec::new();
        for (index, preview) in self.binding.2.borrow().iter().enumerate() {
            let Some((owner, _)) = windows
                .iter()
                .find(|(_, w)| w.backend.is_some() && w.desktop_id == Some(preview.window))
            else {
                continue;
            };
            let Some(root) = sources
                .iter()
                .find(|s| s.key == ShellLayerKey::Surface(owner.get()))
            else {
                continue;
            };
            let r = preview.rect;
            let extent = self.layer.runtime.extent();
            // Only finite, in-surface slots are admitted. This also bounds coordinate conversion.
            if ![r.x, r.y, r.width, r.height]
                .into_iter()
                .all(f32::is_finite)
                || r.x < 0.0
                || r.y < 0.0
                || r.width <= 0.0
                || r.height <= 0.0
                || r.x + r.width > extent.width
                || r.y + r.height > extent.height
            {
                continue;
            }
            let sx = self.sampled.width as f32 / extent.width.max(1.0);
            let sy = self.sampled.height as f32 / extent.height.max(1.0);
            let slot = RectI {
                x: self.sampled.x.saturating_add((r.x * sx).round() as i32),
                y: self.sampled.y.saturating_add((r.y * sy).round() as i32),
                width: (r.width * sx).round() as i32,
                height: (r.height * sy).round() as i32,
            };
            let Some(clip) = intersect_rect(slot, self.sampled) else {
                continue;
            };
            let Some(fitted) = fit_preview(
                SizeI {
                    width: root.target.width,
                    height: root.target.height,
                },
                slot,
            ) else {
                continue;
            };
            for source in sources {
                let ShellLayerKey::Surface(raw) = source.key else {
                    continue;
                };
                let Some(surface) = WaylandSurfaceId::from_raw(raw) else {
                    continue;
                };
                if !preview_surface_belongs_to(windows, surface, *owner) {
                    continue;
                }
                let ShellLayerContent::Image {
                    scene,
                    content_version,
                    alpha_mode,
                    pixel_format,
                    ..
                } = source.content
                else {
                    continue;
                };
                let target = map_preview_rect(source.target, root.target, fitted);
                let source_clip = source
                    .clip
                    .map(|r| map_preview_rect(r, root.target, fitted))
                    .unwrap_or(fitted);
                let Some(clip) =
                    intersect_rect(source_clip, clip).and_then(|r| intersect_rect(r, fitted))
                else {
                    continue;
                };
                // The normal client layer is the only producer. Unchanged reads its retained,
                // already-admitted image and cannot consume updates or create another upload.
                layers.push(ShellLayer::image(
                    ShellLayerKey::WindowPreview(self.id, index as u32, raw),
                    scene,
                    content_version,
                    ShellImageUpdate::Unchanged,
                    source.source_extent,
                    target,
                    Some(clip),
                    alpha_mode,
                    pixel_format,
                    true,
                ));
            }
        }
        layers
    }
    fn hover_contains(&self, p: PointF) -> bool {
        if self.inside_bounds(p) {
            return true;
        }
        let ShellAttachment::Parent { rect, .. } = self.spec.placement.attachment else {
            return false;
        };
        let Some(parent) = self.parent_bounds else {
            return false;
        };
        let anchor = rect.unwrap_or(crate::foundation::RectF {
            x: 0.0,
            y: 0.0,
            width: parent.width as f32,
            height: parent.height as f32,
        });
        let x = parent.x as f32 + anchor.x;
        let y = parent.y as f32 + anchor.y;
        // Keep the attachment and connecting gap traversable, including a clamped popup.
        let left = x.min(self.sampled.x as f32);
        let top = y.min(self.sampled.y as f32);
        let right = (x + anchor.width).max((self.sampled.x + self.sampled.width) as f32);
        let bottom = (y + anchor.height).max((self.sampled.y + self.sampled.height) as f32);
        p.x >= left && p.x < right && p.y >= top && p.y < bottom
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
        !self.retiring
            && self.spec.visible
            && self.parent_visible
            && self.binding.0.borrow().is_some_and(|s| s.visible)
    }
    pub(super) fn contains(&mut self, p: PointF) -> bool {
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
fn preview_surface_belongs_to(
    windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
    mut surface: WaylandSurfaceId,
    owner: WaylandSurfaceId,
) -> bool {
    // Include subsurface content (e.g. video), but not separate toplevels or popup menus.
    for _ in 0..=windows.len() {
        if surface == owner {
            return true;
        }
        let Some(window) = windows.get(&surface) else {
            return false;
        };
        if window.role != SurfaceRole::Subsurface {
            return false;
        }
        let Some(parent) = window.parent else {
            return false;
        };
        surface = parent;
    }
    false
}
fn map_preview_rect(rect: RectI, source: RectI, target: RectI) -> RectI {
    let sx = target.width as f64 / source.width.max(1) as f64;
    let sy = target.height as f64 / source.height.max(1) as f64;
    RectI {
        x: target
            .x
            .saturating_add(((rect.x as f64 - source.x as f64) * sx).round() as i32),
        y: target
            .y
            .saturating_add(((rect.y as f64 - source.y as f64) * sy).round() as i32),
        width: (rect.width as f64 * sx).round().max(1.0) as i32,
        height: (rect.height as f64 * sy).round().max(1.0) as i32,
    }
}
fn fit_preview(source: SizeI, slot: RectI) -> Option<RectI> {
    if source.width <= 0 || source.height <= 0 || slot.width <= 0 || slot.height <= 0 {
        return None;
    }
    let scale =
        (slot.width as f64 / source.width as f64).min(slot.height as f64 / source.height as f64);
    let width = (source.width as f64 * scale).round().max(1.0) as i32;
    let height = (source.height as f64 * scale).round().max(1.0) as i32;
    Some(RectI {
        x: slot.x + (slot.width - width) / 2,
        y: slot.y + (slot.height - height) / 2,
        width,
        height,
    })
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
            if w.pointer_hovered {
                w.layer
                    .runtime
                    .shell_input(crate::input::InputEvent::mouse_moved(PointF {
                        x: -1_000_000.0,
                        y: -1_000_000.0,
                    }))?;
                w.pointer_hovered = false;
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
    for w in widgets.iter_mut() {
        if w.input_visible()
            && w.spec.dismiss_on_pointer_leave
            && !w.captured
            && !w.hover_contains(p)
        {
            w.dismiss(ShellDismissReason::PointerLeft)?;
        }
    }
    for (i, w) in widgets.iter_mut().enumerate() {
        let local = if Some(i) == hit {
            w.local(p)
        } else {
            PointF {
                x: -1_000_000.0,
                y: -1_000_000.0,
            }
        };
        // Notify the previous target once on leave so pending hover work can be cancelled.
        if Some(i) == hit || w.pointer_hovered {
            w.layer
                .runtime
                .shell_input(crate::input::InputEvent::mouse_moved(local))?;
        }
        w.pointer_hovered = Some(i) == hit;
        w.layer.pointer_motion(local, now);
    }
    Ok(hit.is_some())
}
pub(super) fn widget_pointer_scroll(
    widgets: &mut [WidgetLayer],
    p: PointF,
    delta: PointF,
    now: MonotonicInstant,
    locked: bool,
) -> AppResult<bool> {
    if locked {
        return Ok(false);
    }
    let Some(i) = ordered(widgets)
        .into_iter()
        .find(|i| widgets[*i].contains(p))
    else {
        return Ok(false);
    };
    let w = &mut widgets[i];
    w.layer.pointer_motion(w.local(p), now);
    let event = crate::input::InputEvent::mouse_scroll(delta);
    w.layer.runtime.shell_input(event.clone())?;
    w.layer.runtime.queue_input(event);
    w.layer.runtime.flush_input(now);
    Ok(true)
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
    let escape = event.logical_key
        == crate::input::LogicalKey::Named(crate::input::NamedKey::Escape)
        && event.state == crate::input::ButtonState::Pressed;
    let Some(i) = ordered(widgets).into_iter().find(|i| {
        widgets[*i].input_visible()
            && (widgets[*i].focused || escape && widgets[*i].spec.dismiss_on_escape)
    }) else {
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
    event: crate::host::application::ShellKeyEvent,
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
    scale: crate::platform::contracts::ScaleFactor,
    wake: &EventNotifier,
    services: &crate::authoring::compose::ShellServices,
) -> AppResult<()> {
    let finished: BTreeSet<_> = widgets
        .iter()
        .filter(|w| w.retiring && !w.presented)
        .map(|w| w.id)
        .collect();
    for widget in widgets
        .iter_mut()
        .rev()
        .filter(|w| finished.contains(&w.id))
    {
        widget.layer.runtime.close_composition()?;
    }
    widgets.retain(|w| !finished.contains(&w.id));
    let mut index = 0;
    while index < widgets.len() {
        if widgets[index].retiring {
            index += 1;
            continue;
        }
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
            for widget in widgets.iter_mut().filter(|w| removed.contains(&w.id)) {
                if !widget.retiring && widget.spec.visibility_motion.is_some() && widget.presented {
                    widget.dismiss(ShellDismissReason::AnchorRemoved)?;
                    widget.retiring = true;
                    widget.focused = false;
                    widget.captured = false;
                }
            }
            removed.retain(|id| !widgets.iter().any(|w| w.id == *id && w.retiring));
            for w in widgets.iter_mut().rev().filter(|w| removed.contains(&w.id)) {
                w.layer.runtime.close_composition()?;
            }
            widgets.retain(|w| !removed.contains(&w.id));
            for child in children {
                let ty = child.root.component_type_id();
                if let Some(existing) = widgets.iter_mut().find(|w| {
                    w.parent == Some(owner) && w.child_key == child.key && w.component_type == ty
                }) {
                    if existing.component_type == ty {
                        existing.retiring = false;
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
                let registered = crate::host::application::declaration::RegisteredShellWidget {
                    content: CompositionDriver::from_erased_for_target(
                        child.root,
                        crate::authoring::compose::RuntimeTarget::ShellWidget,
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
mod tests;

pub(super) fn prepare_widget_surfaces(
    widgets: &mut [WidgetLayer],
    output: SizeI,
    work: crate::foundation::RectF,
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
mod child_tests;

impl Drop for WidgetLayer {
    fn drop(&mut self) {
        let _ = self.layer.runtime.close_composition();
    }
}

#[cfg(test)]
mod taskbar_context_tests;

#[cfg(test)]
mod icon_publication_tests;

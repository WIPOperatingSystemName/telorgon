//! Shell surface lifetime and sampled geometry over the existing retained runtimes.
mod output_previews;
#[cfg(all(test, feature = "shell-screencast-linux"))]
mod capture_tests;
use super::motion::geometry::{GeometryTrack, Sample};
use super::scene::ShellLayerContent;
use super::*;
use crate::compose::{
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
    binding: crate::compose::shell_widget::SurfaceBinding,
    target: RectI,
    layout_bounds: RectI,
    output_extent: SizeI,
    pub sampled: RectI,
    track: GeometryTrack,
    initialized: bool,
    presented: bool,
    geometry_dirty: bool,
    previous_previews: Vec<crate::compose::ShellWindowPreview>,
    previous_output_previews: Vec<crate::compose::ShellOutputPreview>,
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
    pub fn prepare(&mut self, output: SizeI, work: crate::core::RectF, now: u64) -> AppResult<()> {
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
                crate::render::RoundedClip::new(
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
        let anchor = rect.unwrap_or(crate::core::RectF {
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
            if matches!(
                event,
                crate::input::InputEvent::PointerMoved { .. }
                    | crate::input::InputEvent::Scroll { .. }
            ) {
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
    fn preview_slots_use_live_generations_include_subsurfaces_and_obey_lock() {
        use super::super::client::maximize_preview_tests::test_window;
        use crate::compose::ShellWindowPreview;
        use std::num::NonZeroU32;
        let id =
            crate::shell::WindowId::new(NonZeroU32::new(1).unwrap(), NonZeroU32::new(1).unwrap());
        let root_id = WaylandSurfaceId::from_raw(10).unwrap();
        let child_id = WaylandSurfaceId::from_raw(11).unwrap();
        let root_rect = RectI {
            x: 100,
            y: 100,
            width: 400,
            height: 200,
        };
        let child_rect = RectI {
            x: 200,
            y: 150,
            width: 100,
            height: 50,
        };
        let mut root = test_window(
            SizeI {
                width: 400,
                height: 200,
            },
            PointI { x: 100, y: 100 },
        );
        root.desktop_id = Some(id);
        root.minimized = true;
        let mut child = test_window(
            SizeI {
                width: 100,
                height: 50,
            },
            PointI { x: 200, y: 150 },
        );
        child.backend = None;
        child.role = SurfaceRole::Subsurface;
        child.parent = Some(root_id);
        let mut windows = BTreeMap::from([(root_id, root), (child_id, child)]);
        let image = |raw, rect: RectI| {
            ShellLayer::image(
                ShellLayerKey::Surface(raw),
                ShellSceneKey::Surface(raw),
                1,
                ShellImageUpdate::Unchanged,
                SizeI {
                    width: rect.width,
                    height: rect.height,
                },
                rect,
                None,
                ImageAlphaMode::Opaque,
                ImagePixelFormat::Rgba8,
                false,
            )
        };
        let sources = vec![image(10, root_rect), image(11, child_rect)];
        let (widget, _, _) = fixture(
            ShellSurfaceSpec::new().placement(
                WidgetPlacement::positioned(PointF { x: 20.0, y: 20.0 })
                    .width(300.0)
                    .height(200.0),
            ),
        );
        let slot = crate::core::RectF {
            x: 10.0,
            y: 10.0,
            width: 200.0,
            height: 100.0,
        };
        *widget.binding.2.borrow_mut() = vec![ShellWindowPreview::new(id, slot)];
        let preview = widget.preview_layers(&windows, &sources, false);
        assert_eq!(preview.len(), 2);
        assert_eq!(
            preview[0].target,
            RectI {
                x: 30,
                y: 30,
                width: 200,
                height: 100
            }
        );
        assert_eq!(
            preview[1].target,
            RectI {
                x: 80,
                y: 55,
                width: 50,
                height: 25
            }
        );
        assert!(preview.iter().all(|p| p.visible
            && matches!(
                p.content,
                ShellLayerContent::Image {
                    update: ShellImageUpdate::Unchanged,
                    ..
                }
            )));
        assert!(widget.preview_layers(&windows, &sources, true).is_empty());
        windows.get_mut(&child_id).unwrap().role = SurfaceRole::XdgPopup;
        assert_eq!(widget.preview_layers(&windows, &sources, false).len(), 1);
        windows.get_mut(&root_id).unwrap().desktop_id = Some(crate::shell::WindowId::new(
            NonZeroU32::new(1).unwrap(),
            NonZeroU32::new(2).unwrap(),
        ));
        assert!(widget.preview_layers(&windows, &sources, false).is_empty());
        windows.get_mut(&root_id).unwrap().desktop_id = Some(id);
        widget.binding.2.borrow_mut()[0].rect.x = f32::NAN;
        assert!(widget.preview_layers(&windows, &sources, false).is_empty());
        widget.binding.2.borrow_mut()[0].rect = crate::core::RectF {
            width: 1000.0,
            ..slot
        };
        assert!(widget.preview_layers(&windows, &sources, false).is_empty());
    }
    #[test]
    fn changing_only_preview_slots_requests_a_new_frame() {
        use std::num::NonZeroU32;
        let (mut widget, _, _) = fixture(
            ShellSurfaceSpec::new().placement(WidgetPlacement::center().width(300.0).height(200.0)),
        );
        widget.scene(false);
        *widget.binding.2.borrow_mut() = vec![crate::compose::ShellWindowPreview::new(
            crate::shell::WindowId::new(NonZeroU32::new(1).unwrap(), NonZeroU32::new(1).unwrap()),
            crate::core::RectF {
                x: 10.0,
                y: 10.0,
                width: 100.0,
                height: 80.0,
            },
        )];
        widget
            .prepare(output(), shell_work_area_for_spec(output()), 1)
            .unwrap();
        assert!(widget.dirty());
        widget.scene(false);
        widget
            .prepare(output(), shell_work_area_for_spec(output()), 2)
            .unwrap();
        assert!(!widget.dirty());
        widget.binding.2.borrow_mut()[0].rect.x = 20.0;
        widget
            .prepare(output(), shell_work_area_for_spec(output()), 3)
            .unwrap();
        assert!(widget.dirty());
    }
    #[test]
    fn hover_popup_keeps_anchor_and_gap_then_dismisses_outside() {
        let spec = ShellSurfaceSpec::new()
            .placement(
                WidgetPlacement::attached_to(
                    crate::core::RectF {
                        x: 30.0,
                        y: 0.0,
                        width: 40.0,
                        height: 48.0,
                    },
                    ShellEdge::Top,
                )
                .width(224.0)
                .height(160.0)
                .offset(0.0, -8.0),
            )
            .pointer(ShellPointer::Surface)
            .dismiss_on_pointer_leave(true);
        let (mut widget, _, dismissals) = fixture(spec);
        widget.parent = Some(1);
        widget.parent_bounds = Some(RectI {
            x: 0,
            y: 552,
            width: 800,
            height: 48,
        });
        widget
            .prepare(output(), shell_work_area_for_spec(output()), 1)
            .unwrap();
        assert!(widget.hover_contains(PointF { x: 45.0, y: 565.0 }));
        assert!(widget.hover_contains(PointF { x: 45.0, y: 548.0 }));
        assert!(widget.hover_contains(PointF { x: 45.0, y: 400.0 }));
        assert!(!widget.hover_contains(PointF { x: 700.0, y: 300.0 }));
        let before = dismissals.get();
        widget_pointer_motion(
            std::slice::from_mut(&mut widget),
            PointF { x: 700.0, y: 300.0 },
            MonotonicInstant::from_nanos(2),
            false,
        )
        .unwrap();
        assert_eq!(dismissals.get(), before + 1);
    }
    #[test]
    fn shell_hook_receives_pointer_motion_when_pointer_leaves_surface() {
        let (widget, _, count) = fixture(
            ShellSurfaceSpec::new()
                .placement(
                    WidgetPlacement::positioned(PointF { x: 20.0, y: 20.0 })
                        .width(200.0)
                        .height(150.0),
                )
                .pointer(ShellPointer::Surface),
        );
        let mut widgets = vec![widget];
        let now = MonotonicInstant::from_nanos(1);
        assert!(
            widget_pointer_motion(&mut widgets, PointF { x: 50.0, y: 50.0 }, now, false).unwrap()
        );
        assert_eq!(count.get(), 1);
        assert!(
            !widget_pointer_motion(&mut widgets, PointF { x: 700.0, y: 500.0 }, now, false)
                .unwrap()
        );
        assert_eq!(count.get(), 2);
    }
    #[test]
    fn scroll_routes_only_to_hit_shell_and_never_while_locked() {
        let (widget, _, count) = fixture(
            ShellSurfaceSpec::new()
                .placement(
                    WidgetPlacement::positioned(PointF { x: 20.0, y: 20.0 })
                        .width(200.0)
                        .height(150.0),
                )
                .pointer(ShellPointer::Surface),
        );
        let mut widgets = vec![widget];
        let now = MonotonicInstant::from_nanos(1);
        let p = PointF { x: 50.0, y: 50.0 };
        assert!(
            widget_pointer_scroll(&mut widgets, p, PointF { x: 0.0, y: 15.0 }, now, false).unwrap()
        );
        assert_eq!(count.get(), 1);
        assert!(
            !widget_pointer_scroll(&mut widgets, p, PointF { x: 0.0, y: 15.0 }, now, true).unwrap()
        );
        assert!(
            !widget_pointer_scroll(
                &mut widgets,
                PointF { x: 500.0, y: 500.0 },
                PointF::default(),
                now,
                false
            )
            .unwrap()
        );
        assert_eq!(count.get(), 1);
    }
    #[test]
    fn preview_fits_portrait_and_landscape_without_stretching() {
        let slot = RectI {
            x: 10,
            y: 20,
            width: 200,
            height: 100,
        };
        assert_eq!(
            fit_preview(
                SizeI {
                    width: 100,
                    height: 200
                },
                slot
            ),
            Some(RectI {
                x: 85,
                y: 20,
                width: 50,
                height: 100
            })
        );
        assert_eq!(
            fit_preview(
                SizeI {
                    width: 400,
                    height: 100
                },
                slot
            ),
            Some(RectI {
                x: 10,
                y: 45,
                width: 200,
                height: 50
            })
        );
        assert_eq!(fit_preview(SizeI::default(), slot), None);
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
    fn tile_widget_scene_survives_owner_close_and_fast_reentry() {
        use super::super::scene::{ShellComposition, ShellSceneKey};
        use crate::renderer_vulkan::VulkanScene;
        let (root, surface) = crate::compose::shell_widget::erase(crate::WindowTiling::snap());
        let registered = crate::application_host::declaration::RegisteredShellWidget {
            content: CompositionDriver::from_erased_for_target(
                root,
                crate::compose::RuntimeTarget::ShellWidget,
            ),
            surface,
        };
        let widget = WidgetLayer::new(
            7,
            registered,
            output(),
            AssetBundle::default(),
            crate::platform::ScaleFactor::new(1.0).unwrap(),
            &EventNotifier::new("tile retention test").unwrap(),
            crate::compose::shell_services::ShellServiceHost::new()
                .services
                .clone(),
        )
        .unwrap();
        let mut widgets = vec![widget];
        let mut composition = ShellComposition::new(output());
        let mut retained = BTreeMap::<ShellSceneKey, VulkanScene>::new();
        let key = ShellSceneKey::Widget(widgets[0].id);
        for (step, raw) in [None, Some(91), None, Some(92), None, Some(93)]
            .into_iter()
            .enumerate()
        {
            let mut windows = BTreeMap::new();
            if let Some(raw) = raw {
                let owner = WaylandSurfaceId::from_raw(raw).unwrap();
                widgets[0].tile_preview_owner = Some(owner);
                widgets[0].tile_preview = Some((
                    owner,
                    RectI {
                        x: 400,
                        y: 0,
                        width: 400 - step as i32 * 10,
                        height: 600,
                    },
                ));
                windows.insert(
                    owner,
                    super::super::client::maximize_preview_tests::test_window(
                        SizeI {
                            width: 300,
                            height: 200,
                        },
                        PointI { x: 400, y: 10 },
                    ),
                );
            } else {
                widgets[0].tile_preview = None;
            }
            let order: Vec<_> = windows.keys().copied().collect();
            let layers = super::super::layers::prepare_desktop_layers(
                false,
                output(),
                step as u64 * 200_000_000,
                false,
                &mut BTreeMap::new(),
                &mut windows,
                &order,
                &mut widgets,
                &mut [],
                &mut None,
                None,
                PointF::default(),
                None,
                PointF::default(),
                &LinuxShellConfig::default(),
            )
            .unwrap();
            let frame = composition
                .synchronize_with_force(output(), layers, true)
                .unwrap();
            for update in frame.updates.iter().filter(|u| u.key == key) {
                let scene = retained.entry(key).or_default();
                for delta in &update.deltas {
                    scene.apply_delta_checked(delta).unwrap();
                }
            }
            retained.retain(|key, _| frame.live_scenes.contains(key));
            assert!(
                retained.contains_key(&key),
                "mounted widget scene retired at step {step}"
            );
            if raw.is_none() {
                assert!(!frame.placements.iter().any(|p| p.scene == key));
            }
        }
        let unmounted = composition
            .synchronize_with_force(output(), Vec::new(), true)
            .unwrap();
        retained.retain(|key, _| unmounted.live_scenes.contains(key));
        assert!(
            !retained.contains_key(&key),
            "unmount still releases the scene"
        );
    }

    #[test]
    fn tile_preview_retargets_fades_and_never_takes_input() {
        use crate::compose::ShellWidget;
        let spec = crate::WindowTiling::snap().surface();
        let (mut widget, _, _) = fixture(spec);
        let owner = WaylandSurfaceId::from_raw(91).unwrap();
        let first = RectI {
            x: 0,
            y: 40,
            width: 400,
            height: 560,
        };
        widget.tile_preview = Some((owner, first));
        widget
            .prepare(output(), shell_work_area_for_spec(output()), 10_000_000)
            .unwrap();
        assert_eq!(widget.sampled, first);
        assert_eq!(widget.opacity, 0.0);
        assert_eq!(widget.spec.pointer, crate::ShellPointer::PassThrough);
        widget
            .prepare(output(), shell_work_area_for_spec(output()), 70_000_000)
            .unwrap();
        assert!(widget.opacity > 0.0 && widget.opacity < 1.0);
        let second = RectI {
            x: 400,
            y: 40,
            width: 400,
            height: 280,
        };
        widget.tile_preview = Some((owner, second));
        widget
            .prepare(output(), shell_work_area_for_spec(output()), 70_000_000)
            .unwrap();
        assert_eq!(widget.sampled, first);
        widget
            .prepare(output(), shell_work_area_for_spec(output()), 300_000_000)
            .unwrap();
        assert_eq!(widget.sampled, second);
        assert_eq!(widget.opacity, 1.0);
        widget.tile_preview = None;
        widget
            .prepare(output(), shell_work_area_for_spec(output()), 300_000_000)
            .unwrap();
        assert!(!widget.input_visible());
        assert!(widget.scene(false).visible);
        assert!(widget.tile_material(true).is_none());
        widget
            .prepare(output(), shell_work_area_for_spec(output()), 400_000_000)
            .unwrap();
        assert!(!widget.scene(false).visible);
        assert!(!widget.animating());
    }

    #[test]
    fn visibility_fades_shrinks_reverses_and_releases_input() {
        let spec = ShellSurfaceSpec::new()
            .placement(WidgetPlacement::center().width(200.0).height(100.0))
            .pointer(ShellPointer::Surface)
            .visibility_motion(crate::Minimize::shrink_and_fade(130));
        let (mut widget, writer, _) = fixture(spec);
        assert_eq!(widget.opacity, 0.0);
        assert_eq!(widget.sampled.width, 184);
        widget
            .prepare(output(), shell_work_area_for_spec(output()), 65_000_000)
            .unwrap();
        assert!(widget.opacity > 0.0 && widget.opacity < 1.0);
        let halfway = widget.opacity;
        writer.publish_if_changed(spec.visible(false));
        widget
            .prepare(output(), shell_work_area_for_spec(output()), 65_000_000)
            .unwrap();
        assert_eq!(widget.opacity, halfway);
        assert!(!widget.input_visible());
        assert!(widget.scene(false).visible);
        widget
            .prepare(output(), shell_work_area_for_spec(output()), 100_000_000)
            .unwrap();
        let exiting = widget.opacity;
        assert!(exiting < halfway);
        writer.publish_if_changed(spec);
        widget
            .prepare(output(), shell_work_area_for_spec(output()), 100_000_000)
            .unwrap();
        assert_eq!(widget.opacity, exiting);
        widget
            .prepare(output(), shell_work_area_for_spec(output()), 230_000_000)
            .unwrap();
        assert_eq!(widget.opacity, 1.0);
        assert_eq!(widget.sampled.width, 200);
        assert!(!widget.animating());
        writer.publish_if_changed(spec.visible(false));
        widget
            .prepare(output(), shell_work_area_for_spec(output()), 231_000_000)
            .unwrap();
        widget
            .prepare(output(), shell_work_area_for_spec(output()), 361_000_000)
            .unwrap();
        assert!(!widget.scene(false).visible);
        assert_eq!(widget.opacity, 0.0);
    }

    #[test]
    fn visibility_motion_snaps_with_reduced_motion() {
        let spec = ShellSurfaceSpec::new().visibility_motion(crate::Minimize::shrink_and_fade(130));
        let (mut widget, writer, _) = fixture(spec);
        widget
            .layer
            .runtime
            .set_motion_preference(crate::theme::MotionPreference::Reduced);
        widget
            .prepare(output(), shell_work_area_for_spec(output()), 1)
            .unwrap();
        assert_eq!(widget.opacity, 1.0);
        assert!(!widget.animating());
        writer.publish_if_changed(spec.visible(false));
        widget
            .prepare(output(), shell_work_area_for_spec(output()), 2)
            .unwrap();
        assert_eq!(widget.opacity, 0.0);
        assert!(!widget.scene(false).visible);
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
        motion: bool,
        mounted: Rc<Cell<u32>>,
        unmounted: Rc<Cell<u32>>,
    }
    struct Child {
        motion: bool,
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
                            motion: self.motion,
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
            let mut spec = ShellSurfaceSpec::new()
                .placement(
                    WidgetPlacement::attached(ShellEdge::Bottom)
                        .width(100.0)
                        .height(40.0),
                )
                .layer(ShellSurfaceLayer::Overlay);
            if self.motion {
                spec = spec.visibility_motion(crate::Minimize::shrink_and_fade(130));
            }
            spec
        }
    }
    #[test]
    fn keyed_children_reorder_without_remount_and_unmount_on_removal() {
        child_lifecycle(false);
    }
    #[test]
    fn fading_children_retire_after_exit_and_can_reopen_without_remount() {
        child_lifecycle(true);
    }
    fn child_lifecycle(motion: bool) {
        let (items, writer) = Signal::new(vec![1, 2]);
        let mounted = Rc::new(Cell::new(0));
        let unmounted = Rc::new(Cell::new(0));
        let (root, surface) = crate::compose::shell_widget::erase(Parent {
            items,
            motion,
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
            1_000_000,
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
            2_000_000,
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
        if motion {
            assert_eq!(unmounted.get(), 0);
            assert_eq!(widgets.len(), 3);
            prepare_widget_surfaces(
                &mut widgets,
                output,
                shell_work_area_for_spec(output),
                3_000_000,
                crate::theme::MotionPreference::Full,
            )
            .unwrap();
            let retiring = widgets.iter().find(|w| w.id == first).unwrap();
            assert!(retiring.retiring);
            assert!(!retiring.input_visible());
            writer.publish_if_changed(vec![1, 2]);
            prepare_widget_surfaces(
                &mut widgets,
                output,
                shell_work_area_for_spec(output),
                4_000_000,
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
            assert!(!widgets.iter().find(|w| w.id == first).unwrap().retiring);
            assert_eq!(mounted.get(), 2);
            writer.publish_if_changed(vec![2]);
            prepare_widget_surfaces(
                &mut widgets,
                output,
                shell_work_area_for_spec(output),
                5_000_000,
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
            prepare_widget_surfaces(
                &mut widgets,
                output,
                shell_work_area_for_spec(output),
                6_000_000,
                crate::theme::MotionPreference::Full,
            )
            .unwrap();
            prepare_widget_surfaces(
                &mut widgets,
                output,
                shell_work_area_for_spec(output),
                200_000_000,
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
        }
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
            preview_size: None,
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
            preview_size: None,
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

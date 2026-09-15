//! Clock-driven presentation, separate from protocol geometry and readiness.
use super::scene::{ShellFrame, ShellLayerKey, ShellPlacement, ShellSceneKey};
use crate::core::{RectI, SizeI};
use crate::theme::MotionPreference;
use crate::{WindowMotion, WindowTween};
use std::collections::{BTreeMap, BTreeSet};
pub(super) mod geometry;
use geometry::GeometryTrack;

#[derive(Clone, Debug)]
pub(super) enum SnapshotContent {
    Capture(Vec<ShellPlacement>),
    Mix(Vec<(u64, f32)>),
}
#[derive(Clone, Debug)]
pub(super) struct SnapshotCommand {
    pub id: u64,
    pub extent: SizeI,
    pub content: SnapshotContent,
}
#[derive(Clone, Debug)]
pub(super) struct SnapshotOutput {
    pub id: u64,
    pub source: u64,
    pub extent: SizeI,
    pub opacity: f32,
}
#[derive(Clone, Debug, Default)]
pub(super) struct MotionFrame {
    pub snapshots: Vec<SnapshotCommand>,
    pub outputs: Vec<SnapshotOutput>,
    pub live: BTreeSet<u64>,
    pub hidden_revisions: BTreeSet<u32>,
    pub fallback: Vec<ShellPlacement>,
}
#[derive(Clone, Copy)]
pub(super) struct WindowState {
    pub bounds: RectI,
    pub maximized: bool,
    pub interactive: bool,
    pub move_pointer: Option<crate::core::PointF>,
    pub minimized: bool,
    pub veiled: bool,
    pub style: WindowMotion,
    pub corner_radii: crate::ui::CornerRadii,
    pub shadows: crate::ui::ShadowList,
}
#[derive(Clone, Copy, Debug)]
struct Track {
    from: f32,
    to: f32,
    start: u64,
    tween: WindowTween,
}
impl Track {
    fn fixed(value: f32) -> Self {
        Self {
            from: value,
            to: value,
            start: 0,
            tween: crate::tween_ms(0, crate::Easing::Linear),
        }
    }
    fn sample(self, now: u64) -> f32 {
        let duration = u64::from(self.tween.duration_ms) * 1_000_000;
        let t = if duration == 0 {
            1.0
        } else {
            now.saturating_sub(self.start) as f32 / duration as f32
        };
        self.from + (self.to - self.from) * self.tween.easing.sample(t)
    }
    fn active(self, now: u64) -> bool {
        self.from != self.to
            && now.saturating_sub(self.start) < u64::from(self.tween.duration_ms) * 1_000_000
    }
    fn retarget(&mut self, to: f32, tween: WindowTween, now: u64) {
        *self = Self {
            from: self.sample(now),
            to,
            start: now,
            tween,
        };
    }
}
struct WindowVisual {
    state: WindowState,
    placements: Vec<ShellPlacement>,
    captured: Option<u64>,
    displayed: Option<u64>,
    outgoing: Option<u64>,
    bounds: RectI,
    geometry_from: RectI,
    geometry: GeometryTrack,
    visibility: Track,
    content: Track,
    order: usize,
    maximize_handoff: bool,
    motion_radii: crate::ui::CornerRadii,
    motion_shadows: crate::ui::ShadowList,
    drag_restore: bool,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct VisualInput {
    from: crate::core::RectF,
    to: crate::core::RectF,
    pub block_content: bool,
}
impl VisualInput {
    pub fn map(self, p: crate::core::PointF) -> crate::core::PointF {
        crate::core::PointF {
            x: self.from.x + (p.x - self.to.x) * self.from.width / self.to.width,
            y: self.from.y + (p.y - self.to.y) * self.from.height / self.to.height,
        }
    }
}
#[derive(Default)]
pub(super) struct WindowMotionController {
    inputs: BTreeMap<u32, VisualInput>,
    windows: BTreeMap<u32, WindowVisual>,
    next_id: u64,
    shadows: Option<super::scene::ShellComposition>,
}
fn next_id(next: &mut u64) -> u64 {
    *next = next.checked_add(1).expect("motion identity exhausted");
    *next
}
fn extent(r: RectI) -> SizeI {
    SizeI {
        width: r.width.max(1),
        height: r.height.max(1),
    }
}
fn map_rect(r: RectI, from: RectI, to: RectI) -> RectI {
    let sx = to.width as f32 / from.width.max(1) as f32;
    let sy = to.height as f32 / from.height.max(1) as f32;
    RectI {
        x: to.x + ((r.x - from.x) as f32 * sx).round() as i32,
        y: to.y + ((r.y - from.y) as f32 * sy).round() as i32,
        width: (r.width as f32 * sx).round().max(1.0) as i32,
        height: (r.height as f32 * sy).round().max(1.0) as i32,
    }
}
fn translated(mut p: ShellPlacement, bounds: RectI) -> ShellPlacement {
    p.target.x -= bounds.x;
    p.target.y -= bounds.y;
    if let Some(c) = &mut p.clip {
        c.x -= bounds.x;
        c.y -= bounds.y;
    }
    for c in p.rounded_clips.iter_mut().flatten() {
        c.rect.x -= bounds.x as f32;
        c.rect.y -= bounds.y as f32;
    }
    p
}
pub(super) fn surface_for_key(key: ShellLayerKey) -> Option<u32> {
    match key {
        ShellLayerKey::Frame(id, _)
        | ShellLayerKey::FrameShadow(id)
        | ShellLayerKey::ContentBackground(id)
        | ShellLayerKey::ContentBorder(id)
        | ShellLayerKey::ContentCorners(id)
        | ShellLayerKey::Surface(id)
        | ShellLayerKey::ResizeVeil(id)
        | ShellLayerKey::LegacyControl(id, _) => Some(id),
        _ => None,
    }
}
impl WindowMotionController {
    pub fn input(&self, root: u32, scale: f32) -> Option<VisualInput> {
        self.inputs.get(&root).copied().map(|mut input| {
            for rect in [&mut input.from, &mut input.to] {
                rect.x /= scale;
                rect.y /= scale;
                rect.width /= scale;
                rect.height /= scale;
            }
            input
        })
    }
    pub fn active(&self, _now: u64) -> bool {
        self.windows.values().any(|v| {
            v.geometry.pending()
                || v.visibility.from != v.visibility.to
                || v.content.from != v.content.to
        })
    }
    /// Frame transforms are rendering-only. Protocol readiness is supplied by the host.
    pub fn apply(
        &mut self,
        frame: &mut ShellFrame,
        states: BTreeMap<u32, WindowState>,
        owners: &BTreeMap<u32, u32>,
        now: u64,
        preference: MotionPreference,
    ) {
        frame.motion.fallback = frame.placements.clone();
        self.inputs.clear();
        let was_active = self.active(now);
        if preference == MotionPreference::Reduced {
            self.windows.clear();
            return;
        }
        self.windows
            .retain(|id, _| states.get(id).is_some_and(|s| s.style.enabled()));
        let mut groups = BTreeMap::<u32, Vec<(usize, ShellPlacement)>>::new();
        for (index, p) in frame.placements.iter().enumerate() {
            if let Some(root) = surface_for_key(p.key).and_then(|id| owners.get(&id))
                && states.get(root).is_some_and(|s| s.style.enabled())
            {
                groups.entry(*root).or_default().push((index, *p));
            }
        }
        let changed = frame
            .updates
            .iter()
            .map(|u| u.key)
            .chain(frame.glass_changed.iter().copied())
            .collect::<BTreeSet<_>>();
        let mut replaced = BTreeSet::new();
        let mut outputs = Vec::new();
        let mut shadow_layers = Vec::new();
        let mut shadow_orders = BTreeMap::new();
        // Conservative bound includes current, outgoing, mixed, and in-flight captures.
        let mut pixels = 0_u64;
        let mut candidates = states
            .into_iter()
            .filter(|(_, s)| s.style.enabled())
            .map(|(id, state)| {
                let active = self.windows.get(&id).is_some_and(|v| {
                    v.state.maximized != state.maximized
                        || v.state.minimized != state.minimized
                        || v.state.veiled != state.veiled
                        || v.geometry.pending()
                        || v.visibility.from != v.visibility.to
                        || v.content.from != v.content.to
                });
                (id, state, active)
            })
            .collect::<Vec<_>>();
        // Admission must follow presentation activity, not native surface-ID allocation order.
        candidates.sort_by_key(|(id, _, active)| (!*active, *id));
        for (id, state, active) in candidates {
            let group = groups.remove(&id).unwrap_or_default();
            if group.is_empty() && !state.minimized {
                // An off-output/unpublished family must not leave its old image on screen.
                self.windows.remove(&id);
                continue;
            }
            if group.is_empty() && !self.windows.contains_key(&id) {
                continue;
            }
            let mut bounds = state.bounds;
            for (_, p) in &group {
                let r = p
                    .clip
                    .and_then(|c| super::geometry::intersect_rect(p.target, c))
                    .unwrap_or(p.target);
                let x = bounds.x.min(r.x);
                let y = bounds.y.min(r.y);
                bounds = RectI {
                    x,
                    y,
                    width: bounds.right().max(r.right()) - x,
                    height: bounds.bottom().max(r.bottom()) - y,
                };
            }
            let area = bounds.width.max(1) as u64 * bounds.height.max(1) as u64;
            // Skip oversized groups rather than make animation allocations unbounded.
            let reserved = area * if active { 6 } else { 2 };
            if area > 16_777_216 || pixels.saturating_add(reserved) > 67_108_864 {
                self.windows.remove(&id);
                continue;
            }
            pixels += reserved;
            let mut local = group
                .iter()
                .map(|(_, p)| translated(*p, bounds))
                .collect::<Vec<_>>();
            let visual = self.windows.entry(id).or_insert_with(|| WindowVisual {
                state,
                placements: Vec::new(),
                captured: None,
                displayed: None,
                outgoing: None,
                bounds,
                geometry_from: state.bounds,
                geometry: GeometryTrack::fixed(state.bounds),
                visibility: Track::fixed(if state.minimized { 0.0 } else { 1.0 }),
                content: Track::fixed(1.0),
                order: group.first().map_or(0, |(i, _)| *i),
                maximize_handoff: false,
                motion_radii: state.corner_radii,
                motion_shadows: state.shadows,
                drag_restore: false,
            });
            let current_sample = visual.geometry.sample(now);
            let current_geometry = current_sample.rect();
            if visual.state.maximized && !state.maximized && state.move_pointer.is_some() {
                visual.drag_restore = true;
            }
            if state.interactive && (!visual.drag_restore || state.move_pointer.is_none()) {
                visual.drag_restore = false;
                visual.maximize_handoff = false;
                visual.geometry_from = state.bounds;
                visual.geometry = GeometryTrack::fixed(state.bounds);
            } else if visual.state.maximized != state.maximized {
                visual.maximize_handoff = state.veiled;
                // Use the restored frame's contour throughout movement, in physical pixels.
                visual.motion_radii = if state.maximized {
                    visual.state.corner_radii
                } else {
                    state.corner_radii
                };
                visual.motion_shadows = if state.maximized {
                    visual.state.shadows
                } else {
                    state.shadows
                };
                visual.geometry_from = current_geometry;
                if visual.drag_restore
                    && let Some(pointer) = state.move_pointer
                {
                    // Keep the same title-bar grab point under the pointer while width changes.
                    let fraction = ((pointer.x - state.bounds.x as f32)
                        / state.bounds.width.max(1) as f32)
                        .clamp(0.0, 1.0);
                    visual.geometry_from.x =
                        (pointer.x - fraction * current_geometry.width as f32).round() as i32;
                    visual.geometry_from.y = state.bounds.y;
                }
                let continuing = visual.geometry.active(now);
                let mut from = current_sample;
                if visual.drag_restore
                    && let Some(pointer) = state.move_pointer
                {
                    let fraction = ((f64::from(pointer.x) - f64::from(state.bounds.x))
                        / f64::from(state.bounds.width.max(1)))
                    .clamp(0.0, 1.0);
                    from.position[0] = f64::from(pointer.x) - fraction * from.position[2];
                    from.position[1] = visual.geometry_from.y as f64;
                    from.velocity[0] = -fraction * from.velocity[2];
                    from.velocity[1] = 0.0;
                }
                // Interruptions preserve velocity immediately; do not freeze a moving spring
                // for a second placeholder entry delay.
                let delay = if visual.maximize_handoff && !continuing {
                    u64::from(state.style.maximize_content_transition(true).duration_ms) * 1_000_000
                } else {
                    0
                };
                visual.geometry = GeometryTrack::new(
                    from,
                    state.bounds,
                    state.style.maximize_transition(state.maximized),
                    now.saturating_add(delay),
                    continuing,
                );
            } else if visual.state.bounds != state.bounds {
                if visual.drag_restore
                    && state.move_pointer.is_some()
                    && visual.state.bounds.width == state.bounds.width
                    && visual.state.bounds.height == state.bounds.height
                {
                    // Pointer translation is immediate; it must not restart the size tween.
                    visual.geometry_from.x += state.bounds.x - visual.state.bounds.x;
                    visual.geometry_from.y += state.bounds.y - visual.state.bounds.y;
                    visual.geometry.translate(
                        state.bounds.x - visual.state.bounds.x,
                        state.bounds.y - visual.state.bounds.y,
                    );
                } else {
                    // Direct manipulation follows the pointer. A measured maximize target retargets.
                    if visual.geometry.active(now) {
                        let start = visual.geometry.start.max(now);
                        let end = visual
                            .geometry
                            .start
                            .saturating_add(u64::from(visual.geometry.duration_ms) * 1_000_000);
                        let remaining_ms = end.saturating_sub(start).div_ceil(1_000_000) as u32;
                        visual.geometry_from = current_geometry;
                        visual.geometry = GeometryTrack::new(
                            current_sample,
                            state.bounds,
                            state
                                .style
                                .maximize_transition(state.maximized)
                                .with_duration_ms(remaining_ms),
                            start,
                            now >= visual.geometry.start,
                        );
                    } else {
                        visual.geometry_from = state.bounds;
                        visual.geometry = GeometryTrack::fixed(state.bounds);
                    }
                }
            }
            if visual.state.minimized != state.minimized {
                visual.visibility.retarget(
                    if state.minimized { 0.0 } else { 1.0 },
                    state.style.minimize_transition(state.minimized),
                    now,
                );
            }
            // An early redraw cannot interrupt entry/movement. Keep the captured placeholder
            // until geometry reaches its destination, then start the ready-content fade.
            let hold_placeholder =
                visual.maximize_handoff && visual.geometry.active(now) && !state.veiled;
            let mut presented_state = state;
            if hold_placeholder {
                presented_state.veiled = true;
            }
            let content_changed = visual.state.veiled != presented_state.veiled;
            if content_changed && !state.minimized {
                visual.outgoing = visual.displayed;
                visual.content = Track::fixed(0.0);
                let tween = if visual.maximize_handoff {
                    state
                        .style
                        .maximize_content_transition(presented_state.veiled)
                } else {
                    state.style.content_transition(presented_state.veiled)
                };
                visual.content.retarget(1.0, tween, now);
            }
            if !group.is_empty() {
                visual.order = group[0].0;
                if visual.maximize_handoff && state.veiled {
                    for p in &mut local {
                        if matches!(p.key, ShellLayerKey::ResizeVeil(_)) {
                            // Radius is applied at the displayed size instead of being baked
                            // into a texture and stretched into an ellipse during restoration.
                            p.rounded_clips = [None; 2];
                        }
                    }
                }
                if !hold_placeholder
                    && (content_changed
                        || visual.captured.is_none()
                        || visual.placements != local
                        || group.iter().any(|(_, p)| changed.contains(&p.scene)))
                {
                    let capture = next_id(&mut self.next_id);
                    frame.motion.snapshots.push(SnapshotCommand {
                        id: capture,
                        extent: extent(bounds),
                        content: SnapshotContent::Capture(local.clone()),
                    });
                    visual.captured = Some(capture);
                    visual.placements = local;
                }
                if !hold_placeholder {
                    visual.bounds = bounds;
                } else {
                    // Pixels remain immutable, but their window-relative bounds must follow
                    // the current destination. Otherwise pointer motion moves the outer clip
                    // while sampling the held image relative to its previous screen position.
                    visual.bounds = map_rect(visual.bounds, visual.state.bounds, state.bounds);
                }
            }
            visual.state = presented_state;
            let opacity = visual.visibility.sample(now);
            if state.minimized && opacity <= 0.0 {
                visual.captured = None;
                visual.displayed = None;
                visual.outgoing = None;
                visual.placements.clear();
                visual.visibility = Track::fixed(0.0);
                visual.geometry = GeometryTrack::fixed(state.bounds);
                visual.content = Track::fixed(1.0);
                continue;
            }
            let Some(captured) = visual.captured else {
                continue;
            };
            let mut displayed = captured;
            let t = visual.content.sample(now);
            if let Some(from) = visual.outgoing
                && t < 1.0
            {
                let mix = next_id(&mut self.next_id);
                frame.motion.snapshots.push(SnapshotCommand {
                    id: mix,
                    // Entry is still at the old geometry; avoid full-maximized-size mixing.
                    extent: if visual.maximize_handoff && presented_state.veiled {
                        extent(visual.geometry_from)
                    } else {
                        extent(visual.bounds)
                    },
                    content: SnapshotContent::Mix(vec![(from, 1.0 - t), (captured, t)]),
                });
                frame.motion.live.insert(from);
                displayed = mix;
            } else {
                visual.outgoing = None;
            }
            visual.displayed = Some(displayed);
            frame.motion.live.extend([captured, displayed]);
            let geometry = visual.geometry.sample(now).rect();
            let mut target = map_rect(visual.bounds, state.bounds, geometry);
            let shrink = 0.92 + 0.08 * opacity;
            let w = (target.width as f32 * shrink).round().max(1.0) as i32;
            let h = (target.height as f32 * shrink).round().max(1.0) as i32;
            target.x += (target.width - w) / 2;
            target.y += (target.height - h) / 2;
            target.width = w;
            target.height = h;
            let visual_outer = map_rect(state.bounds, visual.bounds, target);
            let float = |r: RectI| crate::core::RectF {
                x: r.x as f32,
                y: r.y as f32,
                width: r.width as f32,
                height: r.height as f32,
            };
            self.inputs.insert(
                id,
                VisualInput {
                    from: float(state.bounds),
                    to: float(visual_outer),
                    block_content: state.veiled
                        || t < 1.0
                        || opacity < 1.0
                        || visual.geometry.active(now),
                },
            );
            if hold_placeholder || t <= 0.0 || opacity <= 0.0 {
                frame.motion.hidden_revisions.extend(
                    frame
                        .surface_revisions
                        .iter()
                        .filter(|(surface, _)| owners.get(surface) == Some(&id))
                        .map(|(surface, _)| *surface),
                );
            }
            if visual.maximize_handoff
                && !presented_state.veiled
                && !visual.geometry.active(now)
                && !visual.content.active(now)
            {
                // Publish the unclipped resting state in the final fully damaged frame.
                // Clearing this after emitting the placement leaves the shadow clipped until
                // a later partial hover repaint, making scanout slots disagree outside it.
                visual.maximize_handoff = false;
                frame.damage = None;
            }
            let output = u64::from(id);
            let separate_shadow = visual.maximize_handoff
                || state.interactive
                || state.veiled
                || visual.content.active(now);
            if separate_shadow {
                let moving = visual.maximize_handoff && visual.geometry.active(now);
                let rect = crate::core::RectF {
                    x: 0.0,
                    y: 0.0,
                    width: visual_outer.width as f32,
                    height: visual_outer.height as f32,
                };
                let instance = crate::render::BoxInstance {
                    node: crate::scene::NodeId::new(0, 1),
                    rect,
                    view_bounds: rect,
                    background: None,
                    border: Default::default(),
                    outline: Default::default(),
                    corner_radii: if moving {
                        visual.motion_radii
                    } else {
                        state.corner_radii
                    },
                    shadows: if moving {
                        visual.motion_shadows
                    } else {
                        state.shadows
                    },
                    opacity,
                    clip: crate::render::ClipId(0),
                    spatial: crate::render::SpatialId(0),
                };
                if let Some(mut layer) = super::scene::ShellLayer::frame_shadow(
                    id,
                    instance,
                    crate::core::PointI {
                        x: visual_outer.x,
                        y: visual_outer.y,
                    },
                ) {
                    layer.key = ShellLayerKey::MotionShadow(id);
                    if let super::scene::ShellLayerContent::Decoration { scene, .. } =
                        &mut layer.content
                    {
                        *scene = ShellSceneKey::MotionShadow(id);
                    }
                    shadow_orders.insert(layer.key, visual.order);
                    shadow_layers.push(layer);
                }
            }
            frame.motion.outputs.push(SnapshotOutput {
                id: output,
                source: displayed,
                extent: extent(target),
                opacity,
            });
            frame.live_scenes.insert(ShellSceneKey::Motion(output));
            outputs.push((
                visual.order,
                ShellPlacement {
                    key: ShellLayerKey::Motion(id),
                    scene: ShellSceneKey::Motion(output),
                    target,
                    clip: None,
                    rounded_clips: if separate_shadow {
                        let radii = if visual.maximize_handoff && visual.geometry.active(now) {
                            visual.motion_radii
                        } else {
                            state.corner_radii
                        };
                        [
                            Some(crate::render::RoundedClip::new(float(visual_outer), radii)),
                            None,
                        ]
                    } else {
                        [None; 2]
                    },
                },
            ));
            replaced.extend(group.iter().map(|(i, _)| *i));
            visual.geometry.finish(now);
            for track in [&mut visual.visibility, &mut visual.content] {
                if !track.active(now) {
                    *track = Track::fixed(track.to);
                }
            }
            if !visual.geometry.active(now) {
                visual.drag_restore = false;
            }
        }
        if was_active || self.active(now) {
            // Motion changes can uncover any underlying layer, including shadows.
            frame.damage = None;
        }
        let shadow_frame = self
            .shadows
            .get_or_insert_with(|| super::scene::ShellComposition::new(frame.extent))
            .synchronize_with_force(frame.extent, shadow_layers, true)
            .expect("forced shadow frame");
        if shadow_frame.damage.is_some() || !shadow_frame.updates.is_empty() {
            // Include old shadow pixels when changing/removing the independent placement.
            frame.damage = None;
        }
        frame.updates.extend(shadow_frame.updates);
        frame.live_scenes.extend(shadow_frame.live_scenes);
        // Insert before the corresponding window, preserving the family stacking order.
        for placement in shadow_frame.placements {
            let index = shadow_orders[&placement.key];
            let position = outputs
                .iter()
                .position(|(order, _)| *order == index)
                .unwrap_or(outputs.len());
            outputs.insert(position, (index, placement));
        }
        let mut placements = Vec::new();
        outputs.sort_by_key(|(index, _)| *index);
        let mut outputs = outputs.into_iter().peekable();
        for (index, p) in frame.placements.drain(..).enumerate() {
            while outputs.peek().is_some_and(|(i, _)| *i <= index) {
                placements.push(outputs.next().unwrap().1);
            }
            if !replaced.contains(&index) {
                placements.push(p);
            }
        }
        placements.extend(outputs.map(|(_, p)| p));
        frame.placements = placements;
    }
}

/// One sampled image per input. Additive blending sums premultiplied weighted endpoints.
pub(super) fn image_scene(
    extent: SizeI,
    inputs: &[(crate::render::ImageId, f32)],
    additive: bool,
) -> crate::render::RenderScene {
    use crate::core::{ColorRgba8, RectF, SizeF};
    use crate::render::*;
    use crate::scene::NodeId;
    let mut scene = RenderScene::default();
    scene.extent = SizeF {
        width: extent.width as f32,
        height: extent.height as f32,
    };
    scene.background = ColorRgba8::rgba(0, 0, 0, 0);
    scene.damage.full = true;
    let rect = RectF {
        x: 0.0,
        y: 0.0,
        width: extent.width as f32,
        height: extent.height as f32,
    };
    let mut order = Vec::new();
    for (index, (image, opacity)) in inputs.iter().enumerate() {
        let node = NodeId::new(index as u32 + 1, 1);
        scene.images.upsert(
            node,
            ImageInstance {
                node,
                image: *image,
                tint: None,
                rect,
                view_bounds: rect,
                content_version: 1,
                opacity: *opacity,
                clip: ClipId(0),
                spatial: SpatialId(0),
            },
        );
        order.push(DrawItem {
            kind: PrimitiveKind::Image,
            index: index as u32,
            batch: BatchKey {
                pipeline: PipelineKind::Image,
                resource: image.0,
                clip: ClipId(0),
                blend: if additive {
                    BlendMode::Add
                } else {
                    BlendMode::Alpha
                },
                target: 0,
            },
        });
    }
    scene.set_draw_order(order);
    scene
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn later_surface_gets_animation_budget_before_idle_windows() {
        use super::super::scene::{ShellComposition, ShellLayer};
        let extent = SizeI {
            width: 3840,
            height: 2400,
        };
        let initial = WindowState {
            bounds: RectI {
                x: 0,
                y: 0,
                width: 3000,
                height: 2000,
            },
            maximized: false,
            interactive: false,
            move_pointer: None,
            minimized: false,
            veiled: false,
            style: WindowMotion::smooth(),
            corner_radii: Default::default(),
            shadows: Default::default(),
        };
        let mut states = BTreeMap::from([(1, initial), (2, initial)]);
        let owners = BTreeMap::from([(1, 1), (2, 2)]);
        let mut composition = ShellComposition::new(extent);
        let mut controller = WindowMotionController::default();
        for now in [0, 1] {
            if now == 1 {
                let state = states.get_mut(&2).unwrap();
                state.maximized = true;
                state.veiled = true;
                state.bounds.width = extent.width;
                state.bounds.height = extent.height;
            }
            let layers = states
                .iter()
                .map(|(id, state)| {
                    ShellLayer::solid(
                        ShellLayerKey::ResizeVeil(*id),
                        ShellSceneKey::ResizeVeil(*id),
                        crate::core::ColorRgba8::rgba(20, 30, 40, 255),
                        state.bounds,
                    )
                })
                .collect();
            let mut frame = composition
                .synchronize_with_force(extent, layers, true)
                .unwrap();
            controller.apply(
                &mut frame,
                states.clone(),
                &owners,
                now,
                MotionPreference::Full,
            );
            assert!(frame.motion.outputs.iter().any(|o| o.id == 2));
            if now == 1 {
                let output = frame
                    .placements
                    .iter()
                    .find(|p| p.key == ShellLayerKey::Motion(2))
                    .unwrap();
                assert_eq!(
                    output.target.width, 3000,
                    "later surface must animate from its previous geometry"
                );
            }
        }
    }
    #[test]
    fn retarget_continues_from_sample_and_zero_duration_settles() {
        let mut t = Track::fixed(0.0);
        t.retarget(1.0, crate::tween_ms(100, crate::Easing::Linear), 0);
        assert_eq!(t.sample(50_000_000), 0.5);
        t.retarget(0.0, crate::tween_ms(100, crate::Easing::Linear), 50_000_000);
        assert_eq!(t.sample(50_000_000), 0.5);
        assert_eq!(t.sample(100_000_000), 0.25);
        t.retarget(1.0, crate::tween_ms(0, crate::Easing::Linear), 100_000_000);
        assert_eq!(t.sample(100_000_000), 1.0);
        assert!(!t.active(100_000_000));
    }
}

//! Clock-driven presentation, separate from protocol geometry and readiness.
use super::scene::{ShellFrame, ShellLayerKey, ShellPlacement, ShellSceneKey};
use crate::foundation::{RectI, SizeI};
use crate::theme::MotionPreference;
use crate::{WindowMotion, WindowTween};
use std::collections::{BTreeMap, BTreeSet};
pub(super) mod geometry;
mod image;
mod admission;
mod timing;
pub(super) use image::{SnapshotInput, image_scene, position_images};
use geometry::GeometryTrack;

#[derive(Clone, Debug)]
pub(super) enum SnapshotContent {
    Capture(Vec<ShellPlacement>),
    Mix(Vec<SnapshotInput>),
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
    pub resize_group: BTreeSet<u32>,
    pub divider_dragging: bool,
}
#[derive(Clone, Copy)]
pub(super) struct WindowState {
    pub bounds: RectI,
    pub maximized: bool,
    pub tiled: Option<crate::TileTarget>,
    pub interactive: bool,
    pub move_pointer: Option<crate::foundation::PointF>,
    pub minimized: bool,
    pub veiled: bool,
    pub style: WindowMotion,
    pub client_decorated: bool,
    /// Preview contour; client decorations only receive this mask during a preview handoff.
    pub corner_radii: crate::ui::CornerRadii,
    pub shadows: crate::ui::ShadowList,
}
impl WindowState {
    fn arranged(self) -> bool {
        self.maximized || self.tiled.is_some()
    }
    fn placement_changed(self, other: Self) -> bool {
        self.maximized != other.maximized || self.tiled != other.tiled
    }
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Track {
    from: f32,
    to: f32,
    start: u64,
    tween: WindowTween,
}
impl Track {
    pub(super) fn fixed(value: f32) -> Self {
        Self {
            from: value,
            to: value,
            start: 0,
            tween: crate::tween_ms(0, crate::Easing::Linear),
        }
    }
    pub(super) fn sample(self, now: u64) -> f32 {
        let duration = u64::from(self.tween.duration_ms) * 1_000_000;
        let t = if duration == 0 {
            1.0
        } else {
            now.saturating_sub(self.start) as f32 / duration as f32
        };
        self.from + (self.to - self.from) * self.tween.easing.sample(t)
    }
    pub(super) fn active(self, now: u64) -> bool {
        self.from != self.to
            && now.saturating_sub(self.start) < u64::from(self.tween.duration_ms) * 1_000_000
    }
    pub(super) fn retarget(&mut self, to: f32, tween: WindowTween, now: u64) {
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
    displayed: Option<(u64, crate::foundation::RectF)>,
    outgoing: Option<(u64, crate::foundation::RectF)>,
    bounds: RectI,
    geometry_from: RectI,
    geometry: GeometryTrack,
    visibility: Track,
    opening: Track,
    content: Track,
    order: usize,
    placement_handoff: bool,
    motion_radii: crate::ui::CornerRadii,
    motion_shadows: crate::ui::ShadowList,
    drag_restore: bool,
    shared_resize: bool,
    last_output: Option<(SnapshotOutput, ShellPlacement)>,
    last_shadow: Option<(crate::graphics::render::BoxInstance, crate::foundation::PointI)>,
    above: Vec<ShellLayerKey>,
    last_trace: Option<u64>,
    last_frame: u64,
    sample_time: u64,
}
struct ClosingWindow {
    above: Vec<ShellLayerKey>,
    shadow: Option<(crate::graphics::render::BoxInstance, crate::foundation::PointI)>,
    output: SnapshotOutput,
    placement: ShellPlacement,
    opacity: Track,
    started: bool,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct VisualInput {
    from: crate::foundation::RectF,
    to: crate::foundation::RectF,
    pub block_content: bool,
}
impl VisualInput {
    pub fn map(self, p: crate::foundation::PointF) -> crate::foundation::PointF {
        crate::foundation::PointF {
            x: self.from.x + (p.x - self.to.x) * self.from.width / self.to.width,
            y: self.from.y + (p.y - self.to.y) * self.from.height / self.to.height,
        }
    }
}
#[derive(Default)]
pub(super) struct WindowMotionController {
    inputs: BTreeMap<u32, VisualInput>,
    windows: BTreeMap<u32, WindowVisual>,
    presented: BTreeSet<u32>,
    closing: BTreeMap<u32, ClosingWindow>,
    next_id: u64,
    widget_opacities: BTreeMap<u32, f32>,
    shadows: Option<super::scene::ShellComposition>,
}
fn next_id(next: &mut u64) -> u64 {
    *next = next.checked_add(1).expect("motion identity exhausted");
    *next
}
impl WindowMotionController {
    pub(super) fn withdraw_from_primary(&mut self, id: u32) {
        self.inputs.remove(&id);
        self.windows.remove(&id);
        self.closing.remove(&id);
    }

    /// Withdrawal, rather than a close request, is authoritative: clients may refuse to close.
    pub(super) fn close(&mut self, id: u32, now: u64) {
        self.inputs.remove(&id);
        self.presented.remove(&id);
        let Some(visual) = self.windows.remove(&id) else {
            return;
        };
        let tween = visual.state.style.close_transition();
        let Some((output, placement)) = visual.last_output else {
            return;
        };
        if tween.duration_ms == 0 || output.opacity <= 0.0 || self.closing.len() >= 16 {
            return;
        }
        let mut opacity = Track::fixed(output.opacity);
        opacity.retarget(0.0, tween, now);
        self.closing.insert(
            id,
            ClosingWindow {
                shadow: visual.last_shadow,
                output,
                placement,
                above: visual.above,
                opacity,
                started: false,
            },
        );
    }

    pub(super) fn cancel_closing(&mut self) {
        self.closing.clear();
    }

    /// Forget captures and input transforms after immediate presentation. Preserve monotonic
    /// snapshot IDs and retained shadow epochs so the next frame can safely publish fresh data.
    pub(super) fn reset_after_fallback(&mut self) {
        self.inputs.clear();
        self.closing.clear();
        for visual in self.windows.values_mut() {
            visual.last_output = None;
            visual.captured = None;
            visual.displayed = None;
            visual.outgoing = None;
            visual.placements.clear();
            visual.geometry_from = visual.state.bounds;
            visual.geometry = GeometryTrack::fixed(visual.state.bounds);
            visual.visibility = Track::fixed(if visual.state.minimized { 0.0 } else { 1.0 });
            visual.opening = Track::fixed(1.0);
            visual.content = Track::fixed(1.0);
            visual.placement_handoff = false;
            visual.drag_restore = false;
            visual.shared_resize = false;
        }
    }

    /// Compose each fading widget and its live window previews once, then apply group opacity
    /// through the same snapshot-output path as minimized windows.
    pub(super) fn apply_widget_visibility(
        &mut self,
        frame: &mut ShellFrame,
        widgets: &[(u32, f32)],
    ) {
        self.widget_opacities.retain(|id, _| widgets.iter().any(|(widget, _)| widget == id));
        for &(id, opacity) in widgets {
            let previous = self.widget_opacities.insert(id, opacity).unwrap_or(1.0);
            let belongs = |key| {
                matches!(key,
                ShellLayerKey::Widget(owner) | ShellLayerKey::TilePreview(owner) | ShellLayerKey::WindowPreview(owner, _, _) | ShellLayerKey::OutputPreview(owner, _, _) if owner == id)
            };
            let Some(index) = frame.placements.iter().position(|p| belongs(p.key)) else {
                continue;
            };
            // Composition damage was computed before group opacity was applied. Invalidate
            // late fades and their final opaque frame, including dependent glass above them.
            if previous != opacity {
                frame.damage = None;
            }
            if opacity >= 1.0 {
                continue;
            }
            let bounds = frame.placements[index].target;
            let local = frame
                .placements
                .iter()
                .copied()
                .filter(|p| belongs(p.key))
                .map(|p| translated(p, bounds))
                .collect();
            let capture = next_id(&mut self.next_id);
            let output = next_id(&mut self.next_id);
            frame.motion.snapshots.push(SnapshotCommand {
                id: capture,
                extent: extent(bounds),
                content: SnapshotContent::Capture(local),
            });
            frame.motion.live.insert(capture);
            frame.motion.outputs.push(SnapshotOutput {
                id: output,
                source: capture,
                extent: extent(bounds),
                opacity,
            });
            frame.live_scenes.insert(ShellSceneKey::Motion(output));
            frame.placements.retain(|p| !belongs(p.key));
            frame.placements.insert(
                index,
                ShellPlacement {
                    key: ShellLayerKey::Widget(id),
                    scene: ShellSceneKey::Motion(output),
                    target: bounds,
                    clip: None,
                    rounded_clips: [None; 2],
                },
            );
        }
    }
}
fn extent(r: RectI) -> SizeI {
    SizeI {
        width: r.width.max(1),
        height: r.height.max(1),
    }
}
pub(super) fn minimize_rect(target: RectI, opacity: f32) -> RectI {
    scale_rect(target, 0.92 + 0.08 * opacity)
}
fn scale_rect(mut target: RectI, shrink: f32) -> RectI {
    let width = (target.width as f32 * shrink).round().max(1.0) as i32;
    let height = (target.height as f32 * shrink).round().max(1.0) as i32;
    target.x += (target.width - width) / 2;
    target.y += (target.height - height) / 2;
    target.width = width;
    target.height = height;
    target
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
        | ShellLayerKey::ResizePreviewBorder(id)
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
    pub(super) fn transition_pending(&self, id: u32) -> bool {
        self.windows.get(&id).is_some_and(|v| {
            (!v.state.minimized && v.state.veiled) || v.geometry.pending() || v.visibility.from != v.visibility.to
                || v.content.from != v.content.to || v.opening.from != v.opening.to
        })
    }

    pub fn active(&self, _now: u64) -> bool {
        !self.closing.is_empty()
            || self.windows.values().any(|v| {
                v.geometry.pending()
                    || v.opening.from != v.opening.to
                    || v.visibility.from != v.visibility.to
                    || v.content.from != v.content.to
            })
    }
    /// Frame transforms are rendering-only. Protocol readiness is supplied by the host.
    pub fn apply_at(
        &mut self,
        frame: &mut ShellFrame,
        states: BTreeMap<u32, WindowState>,
        owners: &BTreeMap<u32, u32>,
        now: u64,
        sample_now: u64,
        preference: MotionPreference,
    ) {
        frame.motion.fallback = frame.placements.clone();
        self.inputs.clear();
        let was_active = self.active(sample_now);
        self.presented.retain(|id| states.contains_key(id));
        // Keep first presentation separate from transient capture eviction and reduced motion.
        let first_presentations: BTreeSet<_> = frame.placements.iter()
            .filter_map(|p| surface_for_key(p.key).and_then(|id| owners.get(&id)).copied())
            .filter(|id| states.contains_key(id) && self.presented.insert(*id))
            .collect();
        if preference == MotionPreference::Reduced {
            self.windows.clear();
            self.closing.clear();
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
        // Closed clients own no input or protocol state. Only immutable renderer snapshots survive.
        self.closing.retain(|id, closing| {
            // Event processing may stall before the next render. Give the exit effect
            // its full duration starting with the first frame that can display it.
            if !closing.started {
                closing.opacity.start = now;
                closing.started = true;
            }
            if states.contains_key(id) || !closing.opacity.active(sample_now) {
                return false;
            }
            let area = closing.output.extent.width.max(1) as u64
                * closing.output.extent.height.max(1) as u64;
            if pixels.saturating_add(area * 2) > 67_108_864 {
                return false;
            }
            pixels += area * 2;
            let order = closing
                .above
                .iter()
                .find_map(|key| frame.placements.iter().position(|p| p.key == *key))
                .unwrap_or(frame.placements.len());
            let mut output = closing.output.clone();
            output.opacity = closing.opacity.sample(sample_now);
            // Normalize against the last visible opacity to avoid a jump if closing interrupts minimize.
            let remaining = output.opacity / closing.output.opacity;
            let mut placement = closing.placement;
            placement.target = minimize_rect(placement.target, remaining);
            let from = closing.placement.target;
            let to = placement.target;
            let transform = |r: crate::foundation::RectF| crate::foundation::RectF {
                x: to.x as f32 + (r.x - from.x as f32) * to.width as f32 / from.width as f32,
                y: to.y as f32 + (r.y - from.y as f32) * to.height as f32 / from.height as f32,
                width: r.width * to.width as f32 / from.width as f32,
                height: r.height * to.height as f32 / from.height as f32,
            };
            placement.clip = placement.clip.map(|r| map_rect(r, from, to));
            for clip in placement.rounded_clips.iter_mut().flatten() {
                clip.rect = transform(clip.rect);
            }
            output.extent = extent(to);
            if let Some((instance, position)) = &closing.shadow {
                let mut instance = instance.clone();
                instance.opacity = output.opacity;
                let outer = transform(crate::foundation::RectF {
                    x: position.x as f32,
                    y: position.y as f32,
                    width: instance.rect.width,
                    height: instance.rect.height,
                });
                instance.rect.width = outer.width;
                instance.rect.height = outer.height;
                instance.view_bounds = instance.rect;
                let position = crate::foundation::PointI {
                    x: outer.x.round() as i32,
                    y: outer.y.round() as i32,
                };
                if let Some(layer) = motion_shadow(*id, instance, position) {
                    shadow_orders.insert(layer.key, order);
                    shadow_layers.push(layer);
                }
            }
            frame.motion.live.insert(output.source);
            frame.live_scenes.insert(closing.placement.scene);
            frame.motion.outputs.push(output);
            outputs.push((order, placement));
            true
        });
        let candidates = admission::candidates(states, &self.windows, &first_presentations, &groups);
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
                opening: {
                    let from = if first_presentations.contains(&id) && !state.minimized {
                        0.0
                    } else {
                        1.0
                    };
                    let mut track = Track::fixed(from);
                    track.retarget(1.0, state.style.open_transition(), now);
                    track
                },
                content: Track::fixed(1.0),
                order: group.first().map_or(0, |(i, _)| *i),
                placement_handoff: false,
                motion_radii: state.corner_radii,
                motion_shadows: state.shadows,
                drag_restore: false,
                shared_resize: false,
                last_output: None,
                last_shadow: None,
                above: Vec::new(),
                last_trace: None,
                last_frame: now,
                sample_time: now,
            });
            let current_sample = visual.geometry.sample(now.max(visual.sample_time));
            visual.sample_time = sample_now;
            let current_geometry = current_sample.rect();
            if visual.state.arranged() && !state.arranged() && state.move_pointer.is_some() {
                visual.drag_restore = true;
            }
            if state.interactive && (!visual.drag_restore || state.move_pointer.is_none()) {
                visual.drag_restore = false;
                visual.placement_handoff = false;
                visual.geometry_from = state.bounds;
                visual.geometry = GeometryTrack::fixed(state.bounds);
            } else if visual.state.placement_changed(state) {
                visual.placement_handoff = state.veiled;
                // Use the restored frame's contour throughout movement, in physical pixels.
                visual.motion_radii = if state.arranged() {
                    visual.state.corner_radii
                } else {
                    state.corner_radii
                };
                visual.motion_shadows = if state.arranged() {
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
                let continuing = visual.geometry.active(sample_now);
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
                let delay = if visual.placement_handoff && !continuing {
                    u64::from(state.style.maximize_content_transition(true).duration_ms) * 1_000_000
                } else {
                    0
                };
                visual.geometry = GeometryTrack::new(
                    from,
                    state.bounds,
                    state.style.maximize_transition(state.arranged()),
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
                    // Direct manipulation follows the pointer. A measured maximize/tile target retargets.
                    if visual.geometry.active(sample_now) {
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
                                .maximize_transition(state.arranged())
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
                visual.placement_handoff && visual.geometry.active(sample_now) && !state.veiled;
            let mut presented_state = state;
            if hold_placeholder {
                presented_state.veiled = true;
            }
            let content_changed = visual.state.veiled != presented_state.veiled;
            if content_changed && !state.minimized {
                visual.outgoing = visual.displayed;
                visual.content = Track::fixed(0.0);
                let tween = if visual.placement_handoff {
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
                visual.above = frame
                    .placements
                    .iter()
                    .skip(group.last().unwrap().0 + 1)
                    .map(|p| p.key)
                    .collect();
                if visual.placement_handoff && state.veiled {
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
            let trace_changed = visual.state.placement_changed(state)
                || visual.state.interactive != state.interactive
                || content_changed;
            visual.state = presented_state;
            let visibility = visual.visibility.sample(sample_now);
            let opening = visual.opening.sample(sample_now);
            let opacity = visibility * opening;
            if state.minimized && opacity <= 0.0 {
                visual.last_output = None;
                visual.captured = None;
                visual.displayed = None;
                visual.outgoing = None;
                visual.placements.clear();
                visual.visibility = Track::fixed(0.0);
                visual.geometry = GeometryTrack::fixed(state.bounds);
                visual.opening = Track::fixed(1.0);
                visual.content = Track::fixed(1.0);
                continue;
            }
            let Some(captured) = visual.captured else {
                continue;
            };
            let mut displayed = captured;
            let t = visual.content.sample(sample_now);
            let canvas = image::relative_bounds(visual.bounds, state.bounds);
            if let Some((from, from_canvas)) = visual.outgoing
                && t < 1.0
            {
                let mix = next_id(&mut self.next_id);
                frame.motion.snapshots.push(SnapshotCommand {
                    id: mix,
                    // Entry is still at the old geometry; avoid full-maximized-size mixing.
                    extent: if visual.placement_handoff && presented_state.veiled {
                        extent(visual.geometry_from)
                    } else {
                        extent(visual.bounds)
                    },
                    content: SnapshotContent::Mix(vec![
                        SnapshotInput::aligned(from, 1.0 - t, from_canvas, canvas),
                        (captured, t).into(),
                    ]),
                });
                frame.motion.live.insert(from);
                displayed = mix;
            } else {
                visual.outgoing = None;
            }
            visual.displayed = Some((displayed, canvas));
            frame.motion.live.extend([captured, displayed]);
            let geometry = visual.geometry.sample(sample_now).rect();
            let opening_scale = state.style.open_initial_scale();
            let scale = (0.92 + 0.08 * visibility)
                * (opening_scale + (1.0 - opening_scale) * opening);
            let target = scale_rect(map_rect(visual.bounds, state.bounds, geometry), scale);
            let visual_outer = map_rect(state.bounds, visual.bounds, target);
            let float = |r: RectI| crate::foundation::RectF {
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
                        || visual.geometry.active(sample_now),
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
            if visual.placement_handoff
                && !presented_state.veiled
                && !visual.geometry.active(sample_now)
                && !visual.content.active(sample_now)
            {
                // Publish the unclipped resting state in the final fully damaged frame.
                // Clearing this after emitting the placement leaves the shadow clipped until
                // a later partial hover repaint, making scanout slots disagree outside it.
                visual.placement_handoff = false;
                frame.damage = None;
            }
            if state.tiled.is_some() && state.interactive && state.veiled {
                visual.shared_resize = true;
            }
            if visual.shared_resize {
                if state.tiled.is_some() && (state.veiled || visual.content.active(sample_now)) {
                    frame.motion.resize_group.insert(id);
                } else {
                    visual.shared_resize = false;
                }
            }
            let output = u64::from(id);
            let separate_shadow = visual.placement_handoff
                || state.interactive
                || state.veiled
                || visual.content.active(sample_now);
            visual.last_shadow = None;
            if separate_shadow {
                let moving = visual.placement_handoff && visual.geometry.active(sample_now);
                let rect = crate::foundation::RectF {
                    x: 0.0,
                    y: 0.0,
                    width: visual_outer.width as f32,
                    height: visual_outer.height as f32,
                };
                let instance = crate::graphics::render::BoxInstance {
                    node: crate::graphics::scene::NodeId::new(0, 1),
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
                    clip: crate::graphics::render::ClipId(0),
                    spatial: crate::graphics::render::SpatialId(0),
                };
                let position = crate::foundation::PointI {
                    x: visual_outer.x,
                    y: visual_outer.y,
                };
                visual.last_shadow = Some((instance.clone(), position));
                if let Some(layer) = motion_shadow(id, instance, position) {
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
                    rounded_clips: if separate_shadow && (!state.client_decorated
                        || presented_state.veiled || visual.content.active(sample_now)
                        || visual.placement_handoff) {
                        let radii = if visual.placement_handoff && visual.geometry.active(sample_now) {
                            visual.motion_radii
                        } else {
                            state.corner_radii
                        };
                        [
                            Some(crate::graphics::render::RoundedClip::new(float(visual_outer), radii)),
                            None,
                        ]
                    } else {
                        [None; 2]
                    },
                },
            ));
            visual.last_output = Some((
                frame.motion.outputs.last().unwrap().clone(),
                outputs.last().unwrap().1,
            ));
            if super::preview_trace::enabled() {
                let transitioning = state.veiled || visual.placement_handoff || visual.content.active(sample_now);
                let finished = !visual.content.active(sample_now) && visual.content.from != visual.content.to;
                if trace_changed || finished || (transitioning
                    && visual.last_trace.is_none_or(|last| now.saturating_sub(last) >= 100_000_000))
                {
                    super::preview_trace::event(format_args!(
                        "window={id} now_ns={now} client_decorated={} veiled={} held={} handoff={} interactive={} content={t:.3} frame_gap_us={} bounds={:?} capture_bounds={:?} displayed={visual_outer:?} radii={:?} capture={captured} output={displayed}",
                        state.client_decorated, state.veiled, hold_placeholder,
                        visual.placement_handoff, state.interactive,
                        now.saturating_sub(visual.last_frame) / 1000,
                        state.bounds, visual.bounds, outputs.last().unwrap().1.rounded_clips,
                    ));
                    visual.last_trace = Some(now);
                }
                visual.last_frame = now;
            }
            replaced.extend(group.iter().map(|(i, _)| *i));
            visual.geometry.finish(sample_now);
            for track in [&mut visual.visibility, &mut visual.opening, &mut visual.content] {
                if !track.active(sample_now) {
                    *track = Track::fixed(track.to);
                }
            }
            if !visual.geometry.active(sample_now) {
                visual.drag_restore = false;
            }
        }
        if was_active || self.active(sample_now) {
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

fn motion_shadow(
    id: u32,
    instance: crate::graphics::render::BoxInstance,
    position: crate::foundation::PointI,
) -> Option<super::scene::ShellLayer> {
    let mut layer = super::scene::ShellLayer::frame_shadow(id, instance, position)?;
    layer.key = ShellLayerKey::MotionShadow(id);
    if let super::scene::ShellLayerContent::Decoration { scene, .. } = &mut layer.content {
        *scene = ShellSceneKey::MotionShadow(id);
    }
    Some(layer)
}


#[cfg(test)]
mod tests;

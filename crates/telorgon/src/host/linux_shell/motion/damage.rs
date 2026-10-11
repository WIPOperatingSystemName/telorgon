//! Damage after presentation transforms, including their final and cancelled samples.
use super::*;
use crate::host::linux_shell::geometry::{full_rect, intersect_rect, union_rect};

#[derive(Clone, PartialEq)]
struct Visual {
    placement: ShellPlacement,
    sample: Option<(u64, SizeI, f32)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::linux_shell::scene::{ShellComposition, ShellLayer};

    fn frame(rect: RectI, opacity: f32) -> ShellFrame {
        let output_extent = SizeI { width: 400, height: 300 };
        let mut frame = ShellComposition::new(output_extent).synchronize_with_force(output_extent, vec![
            ShellLayer::solid(ShellLayerKey::Surface(1), ShellSceneKey::Surface(1),
                crate::foundation::ColorRgba8::rgba(255, 0, 0, 255), rect),
        ], true).unwrap();
        frame.placements[0].key = ShellLayerKey::Motion(1);
        frame.placements[0].scene = ShellSceneKey::Motion(1);
        frame.motion.outputs.push(SnapshotOutput { id: 1, source: 1, extent: extent(rect), opacity });
        frame.damage = Some(RectI { x: 0, y: 0, width: 1, height: 1 });
        frame
    }

    #[test]
    fn late_opacity_geometry_and_removal_damage_old_and_new_bounds() {
        let mut damage = MotionDamage::default();
        let old = RectI { x: 40, y: 40, width: 80, height: 60 };
        damage.synchronize(&mut frame(old, 1.0));
        let mut fading = frame(old, 0.5);
        damage.synchronize(&mut fading);
        assert_eq!(fading.damage, Some(RectI { x: 0, y: 0, width: 120, height: 100 }));
        let next = RectI { x: 60, y: 20, ..old };
        let mut moved = frame(next, 0.5);
        damage.synchronize(&mut moved);
        assert_eq!(moved.damage, Some(RectI { x: 0, y: 0, width: 140, height: 100 }));
        let mut removed = frame(next, 0.5);
        removed.placements.clear();
        removed.motion.outputs.clear();
        damage.synchronize(&mut removed);
        assert_eq!(removed.damage, Some(RectI { x: 0, y: 0, width: 140, height: 80 }));
    }

    #[test]
    fn late_motion_includes_backdrop_outputs_beyond_the_moving_window() {
        let mut damage = MotionDamage::default();
        let rect = RectI { x: 40, y: 40, width: 80, height: 60 };
        let mut initial = frame(rect, 1.0);
        let glass = ShellPlacement { key: ShellLayerKey::Widget(7), scene: ShellSceneKey::Widget(7),
            target: RectI { x: 220, y: 120, width: 30, height: 20 }, clip: None, rounded_clips: [None; 2] };
        initial.placements.push(glass);
        initial.glass.insert(glass.scene, crate::GlassStyle::default());
        damage.synchronize(&mut initial);
        let mut moving = frame(RectI { x: 50, ..rect }, 1.0);
        moving.placements.push(glass);
        moving.glass.insert(glass.scene, crate::GlassStyle::default());
        damage.synchronize(&mut moving);
        assert_eq!(moving.damage, Some(RectI { x: 0, y: 0, width: 250, height: 140 }));
        assert!(moving.glass_changed.contains(&glass.scene));
    }
}

#[derive(Default)]
pub(super) struct MotionDamage {
    previous: BTreeMap<ShellLayerKey, Visual>,
    order: Vec<ShellLayerKey>,
}

impl MotionDamage {
    pub(super) fn synchronize(&mut self, frame: &mut ShellFrame) {
        let samples = frame.motion.outputs.iter().map(|o| (ShellSceneKey::Motion(o.id),
            (o.source, o.extent, o.opacity))).collect::<BTreeMap<_, _>>();
        let current = frame.placements.iter().map(|p| (p.key, Visual {
            placement: *p, sample: samples.get(&p.scene).copied(),
        })).collect::<BTreeMap<_, _>>();
        let output = full_rect(frame.extent);
        let mut changed = None;
        let mut add = |placement: ShellPlacement| {
            if let Some(bounds) = intersect_rect(placement.target, placement.clip.unwrap_or(output))
                .and_then(|r| intersect_rect(r, output)) {
                changed = Some(changed.map_or(bounds, |old| union_rect(old, bounds)));
            }
        };
        for (key, visual) in &current {
            if self.previous.get(key) != Some(visual) {
                if let Some(old) = self.previous.get(key) { add(old.placement); }
                add(visual.placement);
            }
        }
        for (key, visual) in &self.previous {
            if !current.contains_key(key) { add(visual.placement); }
        }
        let order = frame.placements.iter().map(|p| p.key).collect::<Vec<_>>();
        if !self.order.iter().filter(|key| current.contains_key(key))
            .eq(order.iter().filter(|key| self.previous.contains_key(key))) {
            changed = Some(output);
        }
        if changed.is_some() && !frame.glass.is_empty() {
            // Backdrop lenses may sample beyond their visible contour. Invalidate every
            // live glass output conservatively, including recipes carried by snapshots.
            for placement in &frame.placements {
                if (frame.glass.contains_key(&placement.scene)
                    || matches!(placement.scene, ShellSceneKey::Motion(_)))
                    && let Some(bounds) = intersect_rect(placement.target, output) {
                    changed = Some(union_rect(changed.unwrap(), bounds));
                }
            }
            frame.glass_changed.extend(frame.glass.keys().copied());
        }
        if let (Some(damage), Some(changed)) = (&mut frame.damage, changed) {
            *damage = union_rect(*damage, changed);
            if *damage == output { frame.damage = None; }
        }
        self.previous = current;
        self.order = order;
    }
}

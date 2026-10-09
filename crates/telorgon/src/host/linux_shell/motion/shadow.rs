//! Retained motion shadows contribute damage only when their scene or placement changes.
use super::super::scene::{ShellComposition, ShellFrame, ShellLayer, ShellLayerKey, ShellPlacement, ShellSceneKey};
use crate::foundation::SizeI;
use std::collections::BTreeSet;

pub(super) struct Shadows {
    composition: ShellComposition,
    placements: Vec<ShellPlacement>,
    live_scenes: BTreeSet<ShellSceneKey>,
}

impl Shadows {
    pub(super) fn new(extent: SizeI) -> Self {
        Self { composition: ShellComposition::new(extent), placements: Vec::new(), live_scenes: BTreeSet::new() }
    }

    pub(super) fn synchronize(&mut self, frame: &mut ShellFrame, layers: Vec<ShellLayer>) -> Vec<ShellPlacement> {
        // A forced composition synthesizes a minimal repaint even without changes. Keep
        // placements ourselves so that unchanged shadows need neither updates nor damage.
        if let Some(shadows) = self.composition.synchronize_with_force(frame.extent, layers, false) {
            frame.damage = match (frame.damage, shadows.damage) {
                (Some(current), Some(changed)) => Some(super::super::geometry::union_rect(current, changed)),
                _ => None,
            };
            frame.updates.extend(shadows.updates);
            self.live_scenes = shadows.live_scenes;
            self.placements = shadows.placements;
        }
        frame.live_scenes.extend(self.live_scenes.iter().copied());
        self.placements.clone()
    }
}

pub(super) fn layer(
    id: u32,
    instance: crate::graphics::render::BoxInstance,
    position: crate::foundation::PointI,
) -> Option<ShellLayer> {
    let mut layer = ShellLayer::frame_shadow(id, instance, position)?;
    layer.key = ShellLayerKey::MotionShadow(id);
    if let super::super::scene::ShellLayerContent::Decoration { scene, .. } = &mut layer.content {
        *scene = ShellSceneKey::MotionShadow(id);
    }
    Some(layer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::{ColorRgba8, RectI};

    fn frame(extent: SizeI, damage: Option<RectI>) -> ShellFrame {
        let mut frame = ShellComposition::new(extent).synchronize_with_force(extent, vec![], true).unwrap();
        frame.damage = damage;
        frame
    }

    #[test]
    fn unchanged_motion_preserves_bounded_menu_damage() {
        let extent = SizeI { width: 800, height: 600 };
        let damage = Some(RectI { x: 15, y: 420, width: 260, height: 140 });
        let mut frame = frame(extent, damage);
        super::super::WindowMotionController::default().apply_at(
            &mut frame, Default::default(), &Default::default(), 0, 0, crate::theme::MotionPreference::Full,
        );
        assert_eq!(frame.damage, damage);
    }

    #[test]
    fn reduced_motion_withdraws_cached_shadows_before_resuming() {
        let extent = SizeI { width: 800, height: 600 };
        let bounds = RectI { x: 100, y: 100, width: 80, height: 70 };
        let mut shadows = Shadows::new(extent);
        shadows.synchronize(&mut frame(extent, None), vec![ShellLayer::solid(
            ShellLayerKey::MotionShadow(1), ShellSceneKey::MotionShadow(1),
            ColorRgba8::rgba(0, 0, 0, 120), bounds,
        )]);
        let mut controller = super::super::WindowMotionController::default();
        controller.shadows = Some(shadows);
        let damage = Some(RectI { x: 120, y: 150, width: 2, height: 2 });
        let mut reduced = frame(extent, damage);
        controller.apply_at(&mut reduced, Default::default(), &Default::default(),
            0, 0, crate::theme::MotionPreference::Reduced);
        assert_eq!(reduced.damage, Some(bounds));
        assert!(reduced.live_scenes.is_empty());
        let mut resumed = frame(extent, damage);
        controller.apply_at(&mut resumed, Default::default(), &Default::default(),
            1, 1, crate::theme::MotionPreference::Full);
        assert_eq!(resumed.damage, damage);
        assert!(resumed.placements.is_empty());
    }

    #[test]
    fn retained_shadows_damage_old_and_new_bounds_and_survive_idle_frames() {
        let extent = SizeI { width: 800, height: 600 };
        let damage = Some(RectI { x: 120, y: 150, width: 2, height: 2 });
        let bounds = RectI { x: 100, y: 100, width: 80, height: 70 };
        let layer = |rect| ShellLayer::solid(ShellLayerKey::MotionShadow(1), ShellSceneKey::MotionShadow(1),
            ColorRgba8::rgba(0, 0, 0, 120), rect);
        let mut shadows = Shadows::new(extent);
        let mut first = frame(extent, damage);
        assert_eq!(shadows.synchronize(&mut first, vec![layer(bounds)]).len(), 1);
        assert_eq!(first.damage, Some(bounds));
        let mut idle = frame(extent, damage);
        assert_eq!(shadows.synchronize(&mut idle, vec![layer(bounds)]).len(), 1);
        assert_eq!(idle.damage, damage);
        assert!(idle.updates.is_empty());
        assert!(idle.live_scenes.contains(&ShellSceneKey::MotionShadow(1)));
        let moved = RectI { x: 140, ..bounds };
        let mut moving = frame(extent, damage);
        shadows.synchronize(&mut moving, vec![layer(moved)]);
        assert_eq!(moving.damage, Some(RectI { width: 120, ..bounds }));
        let mut removed = frame(extent, Some(RectI { x: 150, y: 150, width: 2, height: 2 }));
        assert!(shadows.synchronize(&mut removed, vec![]).is_empty());
        assert_eq!(removed.damage, Some(moved));
        assert!(removed.live_scenes.is_empty());
        let mut full = frame(extent, None);
        shadows.synchronize(&mut full, vec![layer(bounds)]);
        assert_eq!(full.damage, None);
    }
}

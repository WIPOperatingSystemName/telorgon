use super::*;
impl WindowMotionController {
    #[cfg(test)]
    pub fn apply(
        &mut self,
        frame: &mut ShellFrame,
        states: BTreeMap<u32, WindowState>,
        owners: &BTreeMap<u32, u32>,
        now: u64,
        preference: MotionPreference,
    ) {
        self.apply_at(frame, states, owners, now, now, preference);
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::scene::{ShellComposition, ShellLayer};
    use super::*;

    #[test]
    fn predicted_geometry_reuses_unchanged_snapshot_and_keeps_real_start_time() {
        let extent = SizeI {
            width: 800,
            height: 600,
        };
        let mut composition = ShellComposition::new(extent);
        let mut motion = WindowMotionController::default();
        let mut state = WindowState {
            bounds: RectI {
                x: 0,
                y: 0,
                width: 300,
                height: 200,
            },
            maximized: false,
            tiled: None,
            minimized: false,
            veiled: false,
            interactive: false,
            move_pointer: None,
            style: WindowMotion::smooth(),
            client_decorated: false,
            corner_radii: Default::default(),
            shadows: Default::default(),
        };
        let mut frame_for = |state: WindowState| {
            composition
                .synchronize_with_force(
                    extent,
                    vec![ShellLayer::solid(
                        ShellLayerKey::Surface(1),
                        ShellSceneKey::Surface(1),
                        crate::foundation::ColorRgba8::rgba(20, 30, 40, 255),
                        state.bounds,
                    )],
                    true,
                )
                .unwrap()
        };
        let owners = BTreeMap::from([(1, 1)]);
        for now in [0, 1_000_000_000] {
            motion.apply_at(
                &mut frame_for(state),
                BTreeMap::from([(1, state)]),
                &owners,
                now,
                now,
                MotionPreference::Full,
            );
        }
        state.maximized = true;
        state.bounds.width = 800;
        state.bounds.height = 600;
        let now = 2_000_000_000;
        let mut first = frame_for(state);
        motion.apply_at(
            &mut first,
            BTreeMap::from([(1, state)]),
            &owners,
            now,
            now + 16_000_000,
            MotionPreference::Full,
        );
        let width = first
            .placements
            .iter()
            .find(|p| p.key == ShellLayerKey::Motion(1))
            .unwrap()
            .target
            .width;
        assert!(
            width > 300 && width < 800,
            "first frame samples expected presentation: {width}"
        );
        assert_eq!(motion.windows[&1].geometry.start, now);
        let captured = motion.windows[&1].captured;
        let mut next = frame_for(state);
        motion.apply_at(
            &mut next,
            BTreeMap::from([(1, state)]),
            &owners,
            now + 8_000_000,
            now + 32_000_000,
            MotionPreference::Full,
        );
        assert_eq!(motion.windows[&1].captured, captured);
        assert!(
            next.motion.snapshots.is_empty(),
            "geometry alone must not recapture unchanged pixels"
        );
    }
}

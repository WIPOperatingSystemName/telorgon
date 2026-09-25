//! Damage relative to an accepted capture, independent of reusable scanout slots.
use crate::{foundation::RectI, media::video::VideoRect, shell::capture::CaptureLayout};
use std::collections::VecDeque;

pub(super) fn collect(
    history: &VecDeque<(u64, Option<RectI>)>,
    current: u64,
    after: Option<u64>,
    layout: CaptureLayout,
) -> Vec<VideoRect> {
    let Some(after) = after else {
        return Vec::new();
    };
    // Empty denotes full damage. History loss, counter wrap and cursor-only updates are
    // intentionally conservative; zero output changes do not yet have a separate wire state.
    if after >= current
        || history
            .front()
            .is_none_or(|(first, _)| after.saturating_add(1) < *first)
    {
        return Vec::new();
    }
    let mut next = after.checked_add(1);
    let mut result = Vec::new();
    for &(version, damage) in history.iter().filter(|(v, _)| *v > after && *v <= current) {
        if Some(version) != next {
            return Vec::new();
        }
        next = version.checked_add(1);
        let Some(rect) = damage else {
            return Vec::new();
        };
        if rect.width <= 0 || rect.height <= 0 {
            return Vec::new();
        }
        let x = i64::from(rect.x).max(0);
        let y = i64::from(rect.y).max(0);
        let right = (i64::from(rect.x) + i64::from(rect.width)).min(i64::from(layout.width()));
        let bottom = (i64::from(rect.y) + i64::from(rect.height)).min(i64::from(layout.height()));
        if right <= x || bottom <= y {
            continue;
        }
        if result.len() == crate::media::video::MAX_DAMAGE_RECTS {
            return Vec::new();
        }
        result.push(VideoRect {
            x: x as u32,
            y: y as u32,
            width: (right - x) as u32,
            height: (bottom - y) as u32,
        });
    }
    if next != current.checked_add(1) {
        return Vec::new();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroU32;
    fn layout() -> CaptureLayout {
        CaptureLayout::rgba8(
            NonZeroU32::new(100).unwrap(),
            NonZeroU32::new(80).unwrap(),
            400,
        )
        .unwrap()
    }
    fn rect(x: i32) -> RectI {
        RectI {
            x,
            y: 10,
            width: 20,
            height: 10,
        }
    }
    #[test]
    fn combines_skipped_output_updates_and_clips_to_capture_pixels() {
        let history = VecDeque::from([
            (8, Some(rect(1))),
            (9, Some(rect(-5))),
            (10, Some(rect(90))),
        ]);
        let damage = collect(&history, 10, Some(8), layout());
        assert_eq!(
            damage,
            vec![
                VideoRect {
                    x: 0,
                    y: 10,
                    width: 15,
                    height: 10
                },
                VideoRect {
                    x: 90,
                    y: 10,
                    width: 10,
                    height: 10
                }
            ]
        );
        assert!(collect(&history, 10, None, layout()).is_empty());
        assert!(collect(&history, 10, Some(10), layout()).is_empty());
    }
    #[test]
    fn incomplete_history_full_updates_and_counter_wrap_never_claim_partial_damage() {
        for history in [
            VecDeque::from([(10, Some(rect(1)))]),
            VecDeque::from([(9, None), (10, Some(rect(1)))]),
            VecDeque::from([(9, Some(rect(1)))]),
            VecDeque::from([
                (
                    9,
                    Some(RectI {
                        width: -1,
                        ..rect(1)
                    }),
                ),
                (10, Some(rect(1))),
            ]),
        ] {
            assert!(collect(&history, 10, Some(8), layout()).is_empty());
        }
        let history = VecDeque::from([(1, Some(rect(1)))]);
        assert!(collect(&history, 1, Some(u64::MAX), layout()).is_empty());
    }
}

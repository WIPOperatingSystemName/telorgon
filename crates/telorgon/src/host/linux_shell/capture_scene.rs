//! Independent offscreen composition. Every placement and sampled revision belongs to the
//! selected sources; physical desktop placements are never used as a virtual-output fallback.
use super::{
    capture_window::WindowCapture,
    scene::{ShellPlacement, ShellSceneKey},
};
use crate::{
    foundation::{PointI, RectI},
    shell::{WindowId, capture::CaptureLayout},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, PartialEq)]
pub(super) struct CaptureScene {
    pub layout: CaptureLayout,
    /// Only scenes intentionally sharing the physical pointer coordinate space set this.
    /// An independent virtual output must not expose the physical desktop's cursor.
    pub desktop_cursor_origin: Option<PointI>,
    pub placements: Vec<ShellPlacement>,
    pub sampled: Vec<(u32, u64)>,
}
impl CaptureScene {
    /// Compose back-to-front window placements into an independent, bounded output extent.
    /// The host owns routing and authorization. Missing windows must be omitted by the host;
    /// stale snapshots must never be substituted with desktop content.
    pub fn virtual_output<'a>(
        layout: CaptureLayout,
        windows: impl IntoIterator<Item = (&'a WindowCapture, RectI)>,
    ) -> Result<Self, &'static str> {
        if layout.width() > 8192 || layout.height() > 8192 {
            return Err("virtual output extent exceeds 8192 pixels");
        }
        let bounds = RectI {
            x: 0,
            y: 0,
            width: layout.width() as i32,
            height: layout.height() as i32,
        };
        let mut selected = BTreeSet::<WindowId>::new();
        let mut placements = Vec::new();
        let mut sampled = BTreeMap::new();
        for (window, target) in windows {
            if selected.len() == 64 || !selected.insert(window.window) {
                return Err("virtual output requires at most 64 distinct windows");
            }
            if target.width <= 0
                || target.height <= 0
                || target.width > 8192
                || target.height > 8192
            {
                return Err("invalid virtual window extent");
            }
            if placements.len().saturating_add(window.placements.len()) > 512 {
                return Err("virtual output exceeds 512 client layers");
            }
            let mut visible = BTreeSet::new();
            for placement in &window.placements {
                let target_rect = map(placement.target, window.layout, target)?;
                let clip = map(
                    placement.clip.unwrap_or(placement.target),
                    window.layout,
                    target,
                )?;
                let Some(clip) =
                    intersect(clip, bounds).and_then(|clip| intersect(clip, target_rect))
                else {
                    continue;
                };
                let ShellSceneKey::Surface(surface) = placement.scene else {
                    return Err("virtual output requires client surface scenes");
                };
                visible.insert(surface);
                placements.push(ShellPlacement {
                    key: placement.key,
                    scene: placement.scene,
                    target: target_rect,
                    clip: Some(clip),
                    rounded_clips: [None, None],
                });
            }
            if !visible.is_empty() {
                for &(surface, revision) in window
                    .surface_revisions()
                    .iter()
                    .filter(|(surface, _)| visible.contains(surface))
                {
                    if sampled
                        .insert(surface, revision)
                        .is_some_and(|old| old != revision)
                    {
                        return Err("virtual output mixes surface revisions");
                    }
                }
            }
        }
        Ok(Self {
            layout,
            desktop_cursor_origin: None,
            placements,
            sampled: sampled.into_iter().collect(),
        })
    }
}
fn map(rect: RectI, source: CaptureLayout, destination: RectI) -> Result<RectI, &'static str> {
    fn axis(start: i32, size: i32, extent: u32, origin: i32, span: i32) -> Option<(i32, i32)> {
        if size <= 0 {
            return None;
        }
        let low =
            i64::from(origin) + (i64::from(start) * i64::from(span)).div_euclid(i64::from(extent));
        let end = (i64::from(start) + i64::from(size)) * i64::from(span);
        let high = i64::from(origin) - (-end).div_euclid(i64::from(extent));
        Some((
            i32::try_from(low).ok()?,
            i32::try_from(high.checked_sub(low)?).ok()?,
        ))
    }
    let (x, width) = axis(
        rect.x,
        rect.width,
        source.width(),
        destination.x,
        destination.width,
    )
    .ok_or("virtual placement overflows")?;
    let (y, height) = axis(
        rect.y,
        rect.height,
        source.height(),
        destination.y,
        destination.height,
    )
    .ok_or("virtual placement overflows")?;
    Ok(RectI {
        x,
        y,
        width,
        height,
    })
}
fn intersect(a: RectI, b: RectI) -> Option<RectI> {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = (i64::from(a.x) + i64::from(a.width)).min(i64::from(b.x) + i64::from(b.width));
    let bottom = (i64::from(a.y) + i64::from(a.height)).min(i64::from(b.y) + i64::from(b.height));
    let width = i32::try_from(right - i64::from(x)).ok()?;
    let height = i32::try_from(bottom - i64::from(y)).ok()?;
    (width > 0 && height > 0).then_some(RectI {
        x,
        y,
        width,
        height,
    })
}

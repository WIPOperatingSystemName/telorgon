//! Default scroll routing and clock-driven wheel easing for retained viewports.
use crate::{
    foundation::{MonotonicInstant, PointF},
    graphics::scene::NodeId,
    input::{DEFAULT_WHEEL_STEP, ScrollPrecision},
    theme::{Easing, MotionPreference},
    ui::{MountedUi, NodeKind, layout::LayoutEngine},
};
use std::time::Duration;

/// Wheel policy shared by managed apps and shell widgets. Precise input bypasses this policy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WheelScrollSettings {
    pixels_per_step: f32,
    duration: Duration,
}
impl Default for WheelScrollSettings {
    fn default() -> Self {
        Self {
            pixels_per_step: DEFAULT_WHEEL_STEP,
            duration: Duration::from_millis(100),
        }
    }
}
impl WheelScrollSettings {
    /// A zero duration disables easing. Speed is measured in logical pixels per wheel step.
    pub fn new(pixels_per_step: f32, duration: Duration) -> Result<Self, &'static str> {
        if !pixels_per_step.is_finite() || pixels_per_step <= 0.0 {
            return Err("Wheel distance must be finite and positive");
        }
        Ok(Self {
            pixels_per_step,
            duration,
        })
    }
    pub fn pixels_per_step(self) -> f32 {
        self.pixels_per_step
    }
    pub fn duration(self) -> Duration {
        self.duration
    }
}

struct Motion {
    node: NodeId,
    from: PointF,
    target: PointF,
    last: PointF,
    start: MonotonicInstant,
}
#[derive(Default)]
pub(super) struct ScrollRuntime {
    pub settings: WheelScrollSettings,
    motions: Vec<Motion>,
}
impl ScrollRuntime {
    pub fn active(&self) -> bool {
        !self.motions.is_empty()
    }
    pub fn cancel(&mut self) {
        self.motions.clear();
    }

    /// Stale node generations, explicit offset changes and hidden viewports cancel their motion.
    pub fn advance(
        &mut self,
        ui: &mut MountedUi,
        layout: &LayoutEngine,
        now: MonotonicInstant,
        preference: MotionPreference,
    ) -> bool {
        let mut changed = false;
        let duration = self.settings.duration.as_secs_f64();
        self.motions.retain_mut(|motion| {
            let Some(max) = bounds(ui, layout, motion.node) else {
                return false;
            };
            let mut style = ui.layouts.get(motion.node).copied().unwrap_or_default();
            if style.scroll_offset != motion.last {
                return false;
            }
            motion.target = clamp(motion.target, max);
            let elapsed = now.as_nanos().saturating_sub(motion.start.as_nanos()) as f64 / 1e9;
            let progress = if preference == MotionPreference::Reduced || duration == 0.0 {
                1.0
            } else {
                (elapsed / duration).min(1.0) as f32
            };
            let eased = Easing::EaseOut.sample(progress);
            let next = clamp(
                PointF {
                    x: motion.from.x + (motion.target.x - motion.from.x) * eased,
                    y: motion.from.y + (motion.target.y - motion.from.y) * eased,
                },
                max,
            );
            style.scroll_offset = next;
            changed |= ui.set_layout_style(motion.node, style);
            motion.last = next;
            progress < 1.0 && next != motion.target
        });
        changed
    }

    pub fn wheel(
        &mut self,
        ui: &mut MountedUi,
        layout: &LayoutEngine,
        target: NodeId,
        delta: PointF,
        precision: ScrollPrecision,
        now: MonotonicInstant,
        preference: MotionPreference,
    ) -> bool {
        if !delta.x.is_finite() || !delta.y.is_finite() || delta == PointF::default() {
            return false;
        }
        let discrete = precision == ScrollPrecision::Discrete;
        // Direct input owns the view now, including any ancestor that was easing a wheel delta.
        if !discrete {
            self.cancel();
        }
        let speed = if discrete {
            self.settings.pixels_per_step / DEFAULT_WHEEL_STEP
        } else {
            1.0
        };
        let mut remaining = PointF {
            x: -delta.x * speed,
            y: -delta.y * speed,
        };
        if !remaining.x.is_finite() || !remaining.y.is_finite() {
            return false;
        }
        let mut changed = false;
        let mut current = Some(target);
        while let Some(node) = current {
            current = ui.nodes.core(node).and_then(|core| core.parent);
            let Some(max) = bounds(ui, layout, node) else {
                continue;
            };
            let mut style = ui.layouts.get(node).copied().unwrap_or_default();
            let offset = clamp(style.scroll_offset, max);
            let previous = self
                .motions
                .iter()
                .position(|m| m.node == node)
                .map(|index| self.motions.remove(index));
            let destination = previous
                .filter(|m| m.last == style.scroll_offset)
                .map_or(offset, |m| clamp(m.target, max));
            // Reverse from the current position immediately; same-direction events extend the target.
            let base = if discrete {
                PointF {
                    x: continuation(offset.x, destination.x, remaining.x),
                    y: continuation(offset.y, destination.y, remaining.y),
                }
            } else {
                offset
            };
            let next = clamp(
                PointF {
                    x: base.x + remaining.x,
                    y: base.y + remaining.y,
                },
                max,
            );
            remaining.x -= next.x - base.x;
            remaining.y -= next.y - base.y;
            if discrete
                && preference != MotionPreference::Reduced
                && !self.settings.duration.is_zero()
                && next != offset
            {
                self.motions.push(Motion {
                    node,
                    from: offset,
                    target: next,
                    last: style.scroll_offset,
                    start: now,
                });
                changed = true;
            } else {
                style.scroll_offset = next;
                changed |= ui.set_layout_style(node, style);
            }
            if remaining.x.abs() < 0.0001 && remaining.y.abs() < 0.0001 {
                break;
            }
        }
        changed
    }
}
/// Layout can shrink even after a wheel animation has finished.
pub(super) fn correct_bounds(ui: &mut MountedUi, layout: &LayoutEngine) -> bool {
    let nodes = ui.nodes.alive().to_vec();
    let mut changed = false;
    for node in nodes {
        let Some(max) = bounds(ui, layout, node) else {
            continue;
        };
        let mut style = ui.layouts.get(node).copied().unwrap_or_default();
        let next = clamp(style.scroll_offset, max);
        if next != style.scroll_offset {
            style.scroll_offset = next;
            changed |= ui.set_layout_style(node, style);
        }
    }
    changed
}

fn continuation(current: f32, target: f32, delta: f32) -> f32 {
    if (target - current) * delta < 0.0 {
        current
    } else {
        target
    }
}
fn clamp(point: PointF, max: PointF) -> PointF {
    PointF {
        x: point.x.clamp(0.0, max.x),
        y: point.y.clamp(0.0, max.y),
    }
}
fn bounds(ui: &MountedUi, layout: &LayoutEngine, node: NodeId) -> Option<PointF> {
    if ui.nodes.core(node).is_none() || ui.kinds.get(node) != Some(&NodeKind::Scroll) {
        return None;
    }
    let mut ancestor = Some(node);
    while let Some(id) = ancestor {
        if ui
            .interactions
            .get(id)
            .is_some_and(|s| !s.visible || !s.enabled)
        {
            return None;
        }
        ancestor = ui.nodes.core(id).and_then(|c| c.parent);
    }
    let viewport = layout.computed(node)?;
    let mut right = 0.0f32;
    let mut bottom = 0.0f32;
    for child in ui.nodes.children(node) {
        if let Some(child) = layout.computed(child) {
            right = right.max(child.local_margin_rect.right());
            bottom = bottom.max(child.local_margin_rect.bottom());
        }
    }
    Some(PointF {
        x: (right - viewport.local_content_rect.width).max(0.0),
        y: (bottom - viewport.local_content_rect.height).max(0.0),
    })
}

#[cfg(test)]
mod tests;

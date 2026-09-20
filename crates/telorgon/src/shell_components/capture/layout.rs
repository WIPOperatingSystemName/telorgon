//! One logical geometry contract for layout, paging and preview placement.
use crate::{RectF, SizeF};

pub(super) const GAP: f32 = 12.0;
pub(super) const PAD: f32 = 20.0;
const HEADER: f32 = 126.0;
const FOOTER: f32 = 104.0;

#[derive(Clone, Copy)]
pub(super) struct PickerLayout {
    pub width: f32,
    pub height: f32,
    pub columns: usize,
    pub rows: usize,
    pub card_width: f32,
    pub card_height: f32,
}
impl PickerLayout {
    pub fn new(output: SizeF, count: usize) -> Self {
        let width = (output.width - 32.0).clamp(320.0, if count == 1 { 600.0 } else { 760.0 });
        let columns = (if width >= 660.0 {
            3
        } else if width >= 460.0 {
            2
        } else {
            1
        })
        .min(count.max(1));
        let card_width = (width - PAD * 2.0 - GAP * (columns - 1) as f32) / columns as f32;
        let available = (output.height - 32.0 - HEADER - FOOTER).max(72.0);
        let card_height = (card_width * 0.5625 + 44.0).min(available);
        let rows = (((available + GAP) / (card_height + GAP)).floor() as usize)
            .clamp(1, 2)
            .min(count.max(1).div_ceil(columns));
        let height = HEADER + FOOTER + card_height * rows as f32 + GAP * (rows - 1) as f32;
        Self {
            width,
            height,
            columns,
            rows,
            card_width,
            card_height,
        }
    }
    pub fn capacity(self) -> usize {
        self.columns * self.rows
    }
    pub fn preview(self, index: usize) -> RectF {
        RectF {
            x: PAD + (index % self.columns) as f32 * (self.card_width + GAP) + 8.0,
            y: HEADER + (index / self.columns) as f32 * (self.card_height + GAP) + 8.0,
            width: self.card_width - 16.0,
            height: (self.card_height - 42.0).max(1.0),
        }
    }
}

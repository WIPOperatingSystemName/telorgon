use std::ops::Range;

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Weight, Wrap};
use unicode_segmentation::UnicodeSegmentation;

use super::{ResolvedTextStyle, TextError, TextResult};

/// Logical single-line geometry from the same shaping engine used to paint text.
/// Byte offsets always refer to extended grapheme boundaries in the source string.
#[derive(Clone, Debug, PartialEq)]
pub struct TextLineLayout {
    width: f32,
    boundaries: Vec<(usize, f32)>,
    cells: Vec<GraphemeCell>,
}

#[derive(Clone, Debug, PartialEq)]
struct GraphemeCell {
    range: Range<usize>,
    left: f32,
    right: f32,
    rtl: bool,
}

impl TextLineLayout {
    pub fn width(&self) -> f32 {
        self.width
    }

    pub fn boundary_positions(&self) -> &[(usize, f32)] {
        &self.boundaries
    }

    /// An offset inside a grapheme is clamped to its preceding boundary.
    pub fn caret_x(&self, byte_offset: usize) -> f32 {
        let index = self
            .boundaries
            .partition_point(|(offset, _)| *offset <= byte_offset);
        self.boundaries[index.saturating_sub(1)].1
    }

    /// Chooses the closest visual grapheme edge, including for RTL and ligatures.
    pub fn hit_test(&self, x: f32) -> usize {
        if x.is_nan() || self.cells.is_empty() {
            return 0;
        }
        let cell = self
            .cells
            .iter()
            .min_by(|a, b| {
                let distance = |cell: &GraphemeCell| {
                    if x < cell.left {
                        cell.left - x
                    } else if x > cell.right {
                        x - cell.right
                    } else {
                        0.0
                    }
                };
                distance(a)
                    .total_cmp(&distance(b))
                    .then_with(|| a.left.total_cmp(&b.left))
            })
            .unwrap();
        let right_half = x >= (cell.left + cell.right) * 0.5;
        if right_half != cell.rtl {
            cell.range.end
        } else {
            cell.range.start
        }
    }

    /// Visual x ranges for a logical selection. Mixed bidi text can produce several spans.
    pub fn selection_segments(&self, range: Range<usize>) -> Vec<Range<f32>> {
        if range.start >= range.end {
            return Vec::new();
        }
        let mut spans: Vec<_> = self
            .cells
            .iter()
            .filter(|cell| {
                cell.range.end > range.start
                    && cell.range.start < range.end
                    && cell.right > cell.left
            })
            .map(|cell| cell.left..cell.right)
            .collect();
        spans.sort_by(|a, b| a.start.total_cmp(&b.start));
        let mut merged: Vec<Range<f32>> = Vec::new();
        for span in spans {
            if let Some(last) = merged.last_mut() {
                if span.start <= last.end + 0.01 {
                    last.end = last.end.max(span.end);
                    continue;
                }
            }
            merged.push(span);
        }
        merged
    }
}

pub(super) fn layout(
    fonts: &mut FontSystem,
    text: &str,
    family: &str,
    style: &ResolvedTextStyle,
) -> TextResult<TextLineLayout> {
    if text
        .chars()
        .any(|ch| matches!(ch, '\r' | '\n' | '\u{0085}' | '\u{2028}' | '\u{2029}'))
    {
        return Err(TextError::new(
            "single-line layout does not accept line separators",
        ));
    }
    let font_size = style.font_size_px.max(1) as f32;
    let line_height = style.line_height_px.max(style.font_size_px).max(1) as f32;
    let mut buffer = Buffer::new(fonts, Metrics::new(font_size, line_height));
    let mut buffer = buffer.borrow_with(fonts);
    buffer.set_size(None, None);
    buffer.set_wrap(Wrap::None);
    let family = match family {
        "serif" => Family::Serif,
        "sans-serif" | "sans_serif" => Family::SansSerif,
        "monospace" => Family::Monospace,
        "cursive" => Family::Cursive,
        "fantasy" => Family::Fantasy,
        name => Family::Name(name),
    };
    buffer.set_text(
        text,
        &Attrs::new()
            .family(family)
            .weight(Weight(style.font_weight)),
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(false);
    let mut cells: Vec<_> = text
        .grapheme_indices(true)
        .map(|(start, grapheme)| GraphemeCell {
            range: start..start + grapheme.len(),
            left: f32::INFINITY,
            right: f32::NEG_INFINITY,
            rtl: false,
        })
        .collect();
    let mut width = 0.0_f32;
    for run in buffer.layout_runs() {
        width = width.max(run.line_w);
        for glyph in run.glyphs {
            // A shaped cluster may contain a ligature's several graphemes, or only
            // one glyph of an emoji/combining grapheme. Union pieces per source EGC.
            let start = cells.partition_point(|cell| cell.range.end <= glyph.start);
            let end = cells.partition_point(|cell| cell.range.start < glyph.end);
            if end <= start {
                continue;
            }
            let piece_width = glyph.w / (end - start) as f32;
            for (index, cell) in cells[start..end].iter_mut().enumerate() {
                let position = if glyph.level.is_rtl() {
                    end - start - 1 - index
                } else {
                    index
                };
                let left = glyph.x + piece_width * position as f32;
                if !cell.left.is_finite() {
                    cell.rtl = glyph.level.is_rtl();
                }
                cell.left = cell.left.min(left);
                cell.right = cell.right.max(left + piece_width);
            }
        }
    }
    let mut boundaries = Vec::with_capacity(cells.len() + 1);
    let mut logical_end = 0.0;
    for cell in &mut cells {
        if !cell.left.is_finite() {
            cell.left = logical_end;
            cell.right = logical_end;
        }
        let x = if cell.rtl { cell.right } else { cell.left };
        logical_end = if cell.rtl { cell.left } else { cell.right };
        boundaries.push((cell.range.start, x));
    }
    boundaries.push((text.len(), logical_end));
    Ok(TextLineLayout {
        width,
        boundaries,
        cells,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ColorRgba8, TextEngine, TextLayoutRequest};

    fn style() -> ResolvedTextStyle {
        ResolvedTextStyle::new(ColorRgba8::rgba(255, 255, 255, 255), 14)
    }

    #[test]
    fn proportional_line_geometry_matches_painted_width_without_rasterizing() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<TextLineLayout>();
        assert_send_sync::<TextEngine>();
        let mut engine = TextEngine::without_system_fonts().unwrap();
        engine.take_atlas_updates();
        let narrow = engine.line_layout("iiii", &style()).unwrap();
        let wide = engine.line_layout("WWWW", &style()).unwrap();
        let text = "office fiancé";
        let line = engine.line_layout(text, &style()).unwrap();
        assert!(wide.width() > narrow.width() * 2.0);
        assert!(
            engine.take_atlas_updates().is_empty(),
            "metrics rasterized glyphs"
        );
        let painted = engine
            .prepare_text(TextLayoutRequest {
                text,
                style: style(),
                max_width_px: None,
                max_height_px: None,
            })
            .unwrap();
        assert!((line.width() - painted.advance_width_px).abs() < 0.01);
        assert_eq!(line.caret_x(text.len()), line.width());
        assert!(
            line.selection_segments(1..4)
                .iter()
                .all(|span| span.end > span.start)
        );
    }

    #[test]
    fn hit_tests_and_carets_never_split_combining_or_emoji_graphemes() {
        let mut engine = TextEngine::without_system_fonts().unwrap();
        for text in ["e\u{301} 👨‍👩‍👧‍👦 🇯🇵 café", "office ffi"] {
            let line = engine.line_layout(text, &style()).unwrap();
            let mut offsets: Vec<_> = text
                .grapheme_indices(true)
                .map(|(offset, _)| offset)
                .collect();
            offsets.push(text.len());
            assert_eq!(
                line.boundary_positions()
                    .iter()
                    .map(|(offset, _)| *offset)
                    .collect::<Vec<_>>(),
                offsets
            );
            for step in -10..=((line.width() * 4.0) as i32 + 10) {
                assert!(offsets.contains(&line.hit_test(step as f32 * 0.25)));
            }
            for offset in 0..=text.len() {
                let preceding = *offsets
                    .iter()
                    .rev()
                    .find(|boundary| **boundary <= offset)
                    .unwrap();
                assert_eq!(line.caret_x(offset), line.caret_x(preceding));
            }
        }
    }

    #[test]
    fn rtl_and_mixed_bidi_selections_keep_visual_runs_separate() {
        let mut engine = TextEngine::without_system_fonts().unwrap();
        let text = "אבג";
        let line = engine.line_layout(text, &style()).unwrap();
        assert!(line.caret_x(0) > line.caret_x(text.len()));
        assert_eq!(line.hit_test(-10.0), text.len());
        assert_eq!(line.hit_test(line.width() + 10.0), 0);
        let text = "abc אבג xyz";
        let line = engine.line_layout(text, &style()).unwrap();
        let spans = line.selection_segments(0.."abc אב".len());
        assert!(
            spans.len() >= 2,
            "bidi selection merged disjoint visual spans: {spans:?}"
        );
        let all = line.selection_segments(0..text.len());
        assert_eq!(all.len(), 1);
        assert!((all[0].end - line.width()).abs() < 0.01);
        assert!(engine.line_layout("first\nsecond", &style()).is_err());
    }
}

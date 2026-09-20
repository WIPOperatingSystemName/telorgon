use std::collections::HashMap;
use std::sync::Arc;

use crate::core::{ColorRgba8, RectI};

use crate::text::retained::stable_string_hash;
use crate::text::shaping::ShapedText;
use crate::text::{
    AtlasPageUpdate, GlyphAtlasView, ResolvedTextStyle, RetainedTextRequest, RetainedTextRun,
    TextEngine, TextLayoutRequest, TextResult, TextRunId, TextRunKey,
};

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct TextCacheStats {
    pub shaped: u64,
    pub reused: u64,
    pub evicted: u64,
    pub runs: usize,
    pub glyphs: usize,
    pub budget_glyphs: usize,
}

#[derive(Clone, Debug)]
struct CachedRun {
    key: TextRunKey,
    run: RetainedTextRun,
    shaped: ShapedText,
    last_used: u64,
}

pub struct RetainedTextSystem {
    engine: TextEngine,
    runs: Vec<Option<CachedRun>>,
    lookup: HashMap<TextRunKey, TextRunId>,
    free: Vec<u32>,
    epoch: u64,
    budget_glyphs: usize,
    stats: TextCacheStats,
}

impl RetainedTextSystem {
    pub fn new(budget_glyphs: usize) -> TextResult<Self> {
        Ok(Self {
            engine: TextEngine::new()?,
            runs: Vec::new(),
            lookup: HashMap::new(),
            free: Vec::new(),
            epoch: 0,
            budget_glyphs: budget_glyphs.max(1),
            stats: TextCacheStats {
                budget_glyphs: budget_glyphs.max(1),
                ..TextCacheStats::default()
            },
        })
    }

    pub fn raster_scale(&self) -> crate::platform::ScaleFactor {
        self.engine.raster_scale()
    }
    pub(crate) fn load_embedded_font(&mut self, bytes: &'static [u8]) -> TextResult<()> {
        self.engine.load_embedded_font(bytes)?;
        self.clear();
        Ok(())
    }

    pub fn load_font_bytes(&mut self, bytes: Vec<u8>) -> TextResult<()> {
        self.engine.load_font_bytes(bytes)?;
        self.clear();
        Ok(())
    }

    pub fn set_raster_scale(&mut self, scale: crate::platform::ScaleFactor) {
        if self.engine.raster_scale() != scale {
            self.clear();
            self.engine.set_raster_scale(scale);
        }
    }

    /// Shapes and caches constraint-dependent text geometry without touching the glyph atlas.
    pub fn measure(&mut self, request: RetainedTextRequest<'_>) -> TextResult<TextRunId> {
        let RetainedTextRequest {
            mut key,
            text,
            family,
            font_size_px,
            line_height_px,
            max_width_px,
            max_height_px,
        } = request;
        key.scale_bits = self.engine.raster_scale().get().to_bits();
        key.text_hash = stable_string_hash(text);
        key.family_hash = stable_string_hash(family);
        key.size_bits = (font_size_px as f32).to_bits();
        key.line_height_bits = (line_height_px as f32).to_bits();
        key.width_constraint_bits = max_width_px.map(f32::to_bits);
        key.height_constraint_bits = max_height_px.map(f32::to_bits);
        self.epoch = self.epoch.saturating_add(1);
        if let Some(id) = self.lookup.get(&key).copied()
            && let Some(entry) = self.runs.get_mut(id.0 as usize).and_then(Option::as_mut)
        {
            entry.last_used = self.epoch;
            self.stats.reused += 1;
            return Ok(id);
        }
        let shaped = self.engine.shape_text(&TextLayoutRequest {
            text,
            style: ResolvedTextStyle::new(ColorRgba8::rgba(255, 255, 255, 255), font_size_px)
                .typography(family, key.weight, line_height_px),
            max_width_px,
            max_height_px,
        });
        let clusters: Arc<[u32]> = text
            .char_indices()
            .map(|(index, _)| index as u32)
            .collect::<Vec<_>>()
            .into();
        let run = RetainedTextRun {
            glyphs: Arc::new([]),
            clusters,
            bounds: RectI {
                x: 0,
                y: 0,
                width: shaped.advance_width_px.ceil().max(0.0) as i32,
                height: shaped.height_px.ceil().max(0.0) as i32,
            },
            advance_width_px: shaped.advance_width_px,
            height_px: shaped.height_px,
            baseline: shaped.baseline_px,
            line_count: shaped.line_count,
            text_revision: key.text_revision,
            style_revision: 0,
            atlas_version: 0,
            atlas_generation: 0,
        };
        let id = if let Some(index) = self.free.pop() {
            self.runs[index as usize] = Some(CachedRun {
                key,
                run,
                shaped,
                last_used: self.epoch,
            });
            TextRunId(index)
        } else {
            let id = TextRunId(self.runs.len() as u32);
            self.runs.push(Some(CachedRun {
                key,
                run,
                shaped,
                last_used: self.epoch,
            }));
            id
        };
        self.lookup.insert(key, id);
        self.stats.shaped += 1;
        self.enforce_budget();
        self.refresh_stats();
        Ok(id)
    }

    /// Ensures that one cached shaped run has atlas placements for painting.
    pub fn prepare(&mut self, request: RetainedTextRequest<'_>) -> TextResult<TextRunId> {
        let id = self.measure(request)?;
        let generation = self.engine.atlas_generation();
        let needs_prepare = self
            .runs
            .get(id.0 as usize)
            .and_then(Option::as_ref)
            .is_some_and(|entry| {
                entry.run.atlas_generation == 0
                    || entry.run.glyphs.is_empty() && !entry.shaped.glyphs.is_empty()
                    || entry.run.atlas_generation != generation
            });
        if needs_prepare {
            let shaped = self.runs[id.0 as usize]
                .as_ref()
                .expect("measured text run must remain live")
                .shaped
                .clone();
            let prepared = self
                .engine
                .prepare_shaped_text(&shaped, ColorRgba8::rgba(255, 255, 255, 255))?;
            let entry = self.runs[id.0 as usize]
                .as_mut()
                .expect("prepared text run must remain live");
            entry.run.glyphs = prepared.glyphs.into();
            entry.run.atlas_version = prepared.atlas_version;
            entry.run.atlas_generation = prepared.atlas_generation;
        }
        Ok(id)
    }

    pub fn run(&self, id: TextRunId) -> Option<&RetainedTextRun> {
        self.runs
            .get(id.0 as usize)?
            .as_ref()
            .map(|entry| &entry.run)
    }

    pub fn atlas(&self) -> GlyphAtlasView<'_> {
        self.engine.atlas()
    }

    pub fn atlas_generation(&self) -> u64 {
        self.engine.atlas_generation()
    }

    pub fn take_atlas_updates(&mut self) -> Vec<AtlasPageUpdate> {
        self.engine.take_atlas_updates()
    }

    pub fn atlas_snapshot(&self) -> AtlasPageUpdate {
        self.engine.atlas_snapshot()
    }

    pub fn stats(&self) -> TextCacheStats {
        self.stats
    }

    pub fn clear(&mut self) {
        self.runs.clear();
        self.lookup.clear();
        self.free.clear();
        self.engine.clear_atlas();
        self.refresh_stats();
    }

    fn enforce_budget(&mut self) {
        while self.glyph_count() > self.budget_glyphs {
            let Some((index, _)) = self
                .runs
                .iter()
                .enumerate()
                .filter_map(|(index, entry)| entry.as_ref().map(|entry| (index, entry.last_used)))
                .min_by_key(|(_, epoch)| *epoch)
            else {
                break;
            };
            if let Some(entry) = self.runs[index].take() {
                self.lookup.remove(&entry.key);
                self.free.push(index as u32);
                self.stats.evicted += 1;
            }
        }
    }

    fn glyph_count(&self) -> usize {
        self.runs
            .iter()
            .filter_map(Option::as_ref)
            .map(|entry| entry.shaped.glyphs.len())
            .sum()
    }

    fn refresh_stats(&mut self) {
        self.stats.runs = self.lookup.len();
        self.stats.glyphs = self.glyph_count();
    }
}

#[cfg(test)]
mod tests {

    use crate::text::{RetainedTextRequest, RetainedTextSystem, TextRunKey};

    #[test]
    fn retained_cache_distinguishes_equal_length_text_content() {
        let mut text = RetainedTextSystem::new(256).unwrap();
        let key = TextRunKey::new(
            1,
            1,
            "sans-serif",
            14.0,
            400,
            18.0,
            Some(160.0),
            Some(24.0),
            1.0,
        );
        let request = |content| RetainedTextRequest {
            key,
            text: content,
            family: "sans-serif",
            font_size_px: 14,
            line_height_px: 18,
            max_width_px: Some(160.0),
            max_height_px: Some(24.0),
        };

        let button = text.prepare(request("Button")).unwrap();
        let slider = text.prepare(request("Slider")).unwrap();
        let button_again = text.prepare(request("Button")).unwrap();

        assert_ne!(button, slider);
        assert_eq!(button, button_again);
    }

    #[test]
    fn measurement_reuses_shaped_geometry_without_populating_the_atlas() {
        let mut text = RetainedTextSystem::new(256).unwrap();
        let request = RetainedTextRequest {
            key: TextRunKey::new(1, 1, "sans-serif", 14.0, 400, 18.0, None, None, 1.0),
            text: "Measured",
            family: "sans-serif",
            font_size_px: 14,
            line_height_px: 18,
            max_width_px: None,
            max_height_px: None,
        };

        let first = text.measure(request.clone()).unwrap();
        let atlas_version = text.atlas().version;
        let second = text.measure(request.clone()).unwrap();
        assert_eq!(first, second);
        assert_eq!(text.atlas().version, atlas_version);

        text.prepare(request).unwrap();
        assert!(text.atlas().version > atlas_version);
        assert!(!text.run(first).unwrap().glyphs.is_empty());
    }

    #[test]
    fn width_constraints_create_distinct_wrapped_layouts() {
        let mut text = RetainedTextSystem::new(512).unwrap();
        let content = "constraint-aware text layout wraps this sentence";
        let request = |max_width_px| RetainedTextRequest {
            key: TextRunKey::new(
                1,
                1,
                "sans-serif",
                14.0,
                400,
                18.0,
                Some(max_width_px),
                None,
                1.0,
            ),
            text: content,
            family: "sans-serif",
            font_size_px: 14,
            line_height_px: 18,
            max_width_px: Some(max_width_px),
            max_height_px: None,
        };

        let wide = text.measure(request(320.0)).unwrap();
        let narrow = text.measure(request(80.0)).unwrap();

        assert_ne!(wide, narrow);
        assert!(text.run(narrow).unwrap().line_count > text.run(wide).unwrap().line_count);
        assert!(text.run(narrow).unwrap().height_px > text.run(wide).unwrap().height_px);
    }

    #[test]
    fn cached_shape_repairs_atlas_placements_after_a_clear() {
        let mut text = RetainedTextSystem::new(256).unwrap();
        let request = RetainedTextRequest {
            key: TextRunKey::new(1, 1, "sans-serif", 14.0, 400, 18.0, None, None, 1.0),
            text: "Reprepare",
            family: "sans-serif",
            font_size_px: 14,
            line_height_px: 18,
            max_width_px: None,
            max_height_px: None,
        };
        let id = text.prepare(request.clone()).unwrap();
        let original_generation = text.run(id).unwrap().atlas_generation;
        let shaped_count = text.stats().shaped;

        text.engine.clear_atlas();
        let repaired = text.prepare(request).unwrap();

        assert_eq!(repaired, id);
        assert_eq!(text.stats().shaped, shaped_count);
        assert!(text.run(id).unwrap().atlas_generation > original_generation);
        assert!(!text.run(id).unwrap().glyphs.is_empty());
    }
}

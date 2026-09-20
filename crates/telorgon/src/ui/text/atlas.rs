use std::collections::HashMap;
use std::sync::Arc;

use cosmic_text::CacheKey;

use crate::ui::text::{TextError, TextResult};

const GLYPH_ATLAS_PADDING_PX: i32 = 2;
pub(crate) const GLYPH_FILTER_GUTTER_PX: i32 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtlasPageUpdate {
    pub page: u16,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub pixels_a8: Arc<[u8]>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct GlyphAtlasView<'a> {
    pub width_px: i32,
    pub height_px: i32,
    pub version: u64,
    /// Changes only when every prior atlas placement becomes invalid.
    pub generation: u64,
    pub pixels_a8: &'a [u8],
}

#[derive(Clone, Debug)]
pub struct GlyphAtlas {
    width_px: i32,
    height_px: i32,
    pixels_a8: Vec<u8>,
    entries: HashMap<CacheKey, GlyphAtlasEntry>,
    cursor_x: i32,
    cursor_y: i32,
    row_height_px: i32,
    version: u64,
    generation: u64,
    dirty_pages: Vec<u16>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) struct GlyphAtlasEntry {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width_px: i32,
    pub(crate) height_px: i32,
}

impl GlyphAtlas {
    pub fn new(width_px: i32, height_px: i32) -> TextResult<Self> {
        if width_px <= 0 || height_px <= 0 {
            return Err(TextError::new(format!(
                "glyph atlas size must be positive, got {width_px}x{height_px}"
            )));
        }

        Ok(Self {
            width_px,
            height_px,
            pixels_a8: vec![0; width_px as usize * height_px as usize],
            entries: HashMap::new(),
            cursor_x: GLYPH_ATLAS_PADDING_PX,
            cursor_y: GLYPH_ATLAS_PADDING_PX,
            row_height_px: 0,
            version: 1,
            generation: 1,
            dirty_pages: Vec::new(),
        })
    }

    pub fn view(&self) -> GlyphAtlasView<'_> {
        GlyphAtlasView {
            width_px: self.width_px,
            height_px: self.height_px,
            version: self.version,
            generation: self.generation,
            pixels_a8: &self.pixels_a8,
        }
    }

    pub fn clear(&mut self) {
        self.pixels_a8.fill(0);
        self.entries.clear();
        self.cursor_x = GLYPH_ATLAS_PADDING_PX;
        self.cursor_y = GLYPH_ATLAS_PADDING_PX;
        self.row_height_px = 0;
        self.version = self.version.saturating_add(1);
        self.generation = self.generation.saturating_add(1);
        self.dirty_pages.clear();
        let pages_x = (self.width_px + 127) / 128;
        let pages_y = (self.height_px + 127) / 128;
        self.dirty_pages
            .extend((0..pages_x * pages_y).map(|page| page as u16));
    }

    pub(crate) fn version(&self) -> u64 {
        self.version
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn get_or_insert(
        &mut self,
        key: CacheKey,
        width_px: i32,
        height_px: i32,
        pixels_a8: &[u8],
    ) -> TextResult<GlyphAtlasEntry> {
        if let Some(entry) = self.entries.get(&key) {
            return Ok(*entry);
        }

        if width_px <= 0 || height_px <= 0 {
            let entry = GlyphAtlasEntry {
                x: 0,
                y: 0,
                width_px: 0,
                height_px: 0,
            };
            self.entries.insert(key, entry);
            return Ok(entry);
        }

        if width_px + GLYPH_ATLAS_PADDING_PX * 2 > self.width_px
            || height_px + GLYPH_ATLAS_PADDING_PX * 2 > self.height_px
        {
            return Err(TextError::new(format!(
                "glyph image {width_px}x{height_px} does not fit atlas {}x{}",
                self.width_px, self.height_px
            )));
        }

        if self.cursor_x + width_px + GLYPH_ATLAS_PADDING_PX > self.width_px {
            self.cursor_x = GLYPH_ATLAS_PADDING_PX;
            self.cursor_y += self.row_height_px + GLYPH_ATLAS_PADDING_PX;
            self.row_height_px = 0;
        }

        if self.cursor_y + height_px + GLYPH_ATLAS_PADDING_PX > self.height_px {
            return Err(TextError::new("glyph atlas is full"));
        }

        let entry = GlyphAtlasEntry {
            x: self.cursor_x,
            y: self.cursor_y,
            width_px,
            height_px,
        };
        self.copy_glyph_pixels(entry, pixels_a8)?;
        self.entries.insert(key, entry);
        self.cursor_x += width_px + GLYPH_ATLAS_PADDING_PX;
        self.row_height_px = self.row_height_px.max(height_px);
        self.version = self.version.saturating_add(1);
        self.mark_dirty_pages(entry);
        Ok(entry)
    }

    fn mark_dirty_pages(&mut self, entry: GlyphAtlasEntry) {
        let pages_x = (self.width_px + 127) / 128;
        let first_x = entry.x / 128;
        let last_x = (entry.x + entry.width_px.saturating_sub(1)) / 128;
        let first_y = entry.y / 128;
        let last_y = (entry.y + entry.height_px.saturating_sub(1)) / 128;
        for page_y in first_y..=last_y {
            for page_x in first_x..=last_x {
                let page = (page_y * pages_x + page_x) as u16;
                if !self.dirty_pages.contains(&page) {
                    self.dirty_pages.push(page);
                }
            }
        }
    }

    pub(crate) fn take_dirty_pages(&mut self) -> Vec<AtlasPageUpdate> {
        let pages_x = (self.width_px + 127) / 128;
        let mut updates = Vec::with_capacity(self.dirty_pages.len());
        for page in self.dirty_pages.drain(..) {
            let page = page as i32;
            let x = (page % pages_x) * 128;
            let y = (page / pages_x) * 128;
            let width = (self.width_px - x).min(128);
            let height = (self.height_px - y).min(128);
            let mut pixels = Vec::with_capacity((width * height) as usize);
            for row in y..y + height {
                let start = (row * self.width_px + x) as usize;
                pixels.extend_from_slice(&self.pixels_a8[start..start + width as usize]);
            }
            updates.push(AtlasPageUpdate {
                page: page as u16,
                x,
                y,
                width,
                height,
                pixels_a8: pixels.into(),
            });
        }
        updates
    }

    pub(crate) fn snapshot(&self) -> AtlasPageUpdate {
        AtlasPageUpdate {
            page: 0,
            x: 0,
            y: 0,
            width: self.width_px,
            height: self.height_px,
            pixels_a8: self.pixels_a8.clone().into(),
        }
    }

    fn copy_glyph_pixels(&mut self, entry: GlyphAtlasEntry, pixels_a8: &[u8]) -> TextResult<()> {
        let required_len = entry.width_px as usize * entry.height_px as usize;
        if pixels_a8.len() < required_len {
            return Err(TextError::new(format!(
                "glyph image has {} alpha bytes but needs {required_len}",
                pixels_a8.len()
            )));
        }

        for row in 0..entry.height_px {
            let dst_offset = ((entry.y + row) * self.width_px + entry.x) as usize;
            let src_offset = (row * entry.width_px) as usize;
            let width = entry.width_px as usize;
            self.pixels_a8[dst_offset..dst_offset + width]
                .copy_from_slice(&pixels_a8[src_offset..src_offset + width]);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::GlyphAtlas;

    #[test]
    fn clear_advances_placement_generation_and_dirties_the_full_atlas() {
        let mut atlas = GlyphAtlas::new(256, 128).unwrap();
        let before = (atlas.view().generation, atlas.view().version);

        atlas.clear();

        let after = atlas.view();
        assert_eq!(after.generation, before.0 + 1);
        assert_eq!(after.version, before.1 + 1);
        let updates = atlas.take_dirty_pages();
        assert_eq!(updates.len(), 2);
        assert!(updates.iter().all(|update| update.width == 128));
        assert!(updates.iter().all(|update| update.height == 128));
    }
}

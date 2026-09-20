use crate::core::ColorRgba8;
use cosmic_text::{
    Attrs, Buffer, Family, FontSystem, Metrics, PhysicalGlyph, Shaping, SwashCache, Weight,
};

use crate::text::atlas::{AtlasPageUpdate, GLYPH_FILTER_GUTTER_PX, GlyphAtlas, GlyphAtlasView};
use crate::text::glyph::{AtlasGlyph, glyph_image_alpha};
use crate::text::{ResolvedTextStyle, TextError, TextResult};

// Discover installed fonts once. Database clones share immutable font sources while each
// engine keeps its shaping caches, raster scale and glyph atlas independent.
fn system_font_seed() -> &'static (String, cosmic_text::fontdb::Database) {
    static SEED: std::sync::OnceLock<(String, cosmic_text::fontdb::Database)> =
        std::sync::OnceLock::new();
    SEED.get_or_init(|| {
        let fonts = FontSystem::new();
        (fonts.locale().to_owned(), fonts.db().clone())
    })
}

fn embedded_faces(
    bytes: &'static [u8],
) -> TextResult<std::sync::Arc<Vec<cosmic_text::fontdb::FaceInfo>>> {
    use std::sync::{Arc, Mutex, OnceLock};
    type Faces = Arc<Vec<cosmic_text::fontdb::FaceInfo>>;
    static CACHE: OnceLock<Mutex<std::collections::HashMap<(usize, usize), Faces>>> =
        OnceLock::new();
    let mut cache = CACHE
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| TextError::new("embedded font cache lock poisoned"))?;
    let key = (bytes.as_ptr() as usize, bytes.len());
    if let Some(faces) = cache.get(&key) {
        return Ok(Arc::clone(faces));
    }
    let mut db = cosmic_text::fontdb::Database::new();
    db.load_font_data(bytes.to_vec());
    let faces: Faces = Arc::new(db.faces().cloned().collect());
    if faces.is_empty() {
        return Err(TextError::new("font bytes did not add any font faces"));
    }
    // Embedded storage is static, so pointer identity cannot be recycled. Bound metadata
    // retention for applications registering many independent asset catalogs.
    if cache.len() < 64 {
        cache.insert(key, Arc::clone(&faces));
    }
    Ok(faces)
}

const DEFAULT_ATLAS_SIZE: i32 = 1024;

#[derive(Clone, Debug, PartialEq)]
/// Layout constraints and font metrics are logical units, despite historical `_px` names.
pub struct TextLayoutRequest<'a> {
    pub text: &'a str,
    pub style: ResolvedTextStyle,
    pub max_width_px: Option<f32>,
    pub max_height_px: Option<f32>,
}

#[derive(Clone, Debug)]
pub(crate) struct ShapedText {
    pub(crate) glyphs: Vec<PhysicalGlyph>,
    pub(crate) advance_width_px: f32,
    pub(crate) height_px: f32,
    pub(crate) baseline_px: f32,
    pub(crate) line_count: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreparedText {
    /// Raster-pixel positions and atlas extents at the engine's raster scale.
    pub glyphs: Vec<AtlasGlyph>,
    /// Widest shaped line advance before rasterization. Unlike glyph ink bounds, this includes
    /// the font's intended side bearings and is therefore the correct width for text alignment.
    pub advance_width_px: f32,
    pub atlas_version: u64,
    pub atlas_generation: u64,
}

pub struct TextEngine {
    font_system: FontSystem,
    swash_cache: SwashCache,
    atlas: GlyphAtlas,
    raster_scale: crate::platform::ScaleFactor,
}

impl TextEngine {
    pub fn new() -> TextResult<Self> {
        Self::with_atlas_size(DEFAULT_ATLAS_SIZE, DEFAULT_ATLAS_SIZE)
    }

    pub fn with_atlas_size(width_px: i32, height_px: i32) -> TextResult<Self> {
        let (locale, database) = system_font_seed();
        Ok(Self {
            font_system: FontSystem::new_with_locale_and_db(locale.clone(), database.clone()),
            swash_cache: SwashCache::new(),
            atlas: GlyphAtlas::new(width_px, height_px)?,
            raster_scale: Default::default(),
        })
    }

    pub fn raster_scale(&self) -> crate::platform::ScaleFactor {
        self.raster_scale
    }

    pub fn set_raster_scale(&mut self, scale: crate::platform::ScaleFactor) {
        if self.raster_scale != scale {
            self.raster_scale = scale;
            self.atlas.clear();
        }
    }

    pub(crate) fn load_embedded_font(&mut self, bytes: &'static [u8]) -> TextResult<()> {
        let faces = embedded_faces(bytes)?;
        let db = self.font_system.db_mut();
        for face in faces.iter() {
            db.push_face_info(face.clone());
        }
        self.atlas.clear();
        Ok(())
    }

    pub fn load_font_bytes(&mut self, bytes: Vec<u8>) -> TextResult<()> {
        let faces_before = self.font_system.db().len();
        self.font_system.db_mut().load_font_data(bytes);
        let faces_after = self.font_system.db().len();
        if faces_after <= faces_before {
            return Err(TextError::new("font bytes did not add any font faces"));
        }
        self.atlas.clear();
        Ok(())
    }

    pub fn atlas(&self) -> GlyphAtlasView<'_> {
        self.atlas.view()
    }

    pub fn take_atlas_updates(&mut self) -> Vec<AtlasPageUpdate> {
        self.atlas.take_dirty_pages()
    }

    pub fn atlas_snapshot(&self) -> AtlasPageUpdate {
        self.atlas.snapshot()
    }

    pub(crate) fn clear_atlas(&mut self) {
        self.atlas.clear();
    }

    pub(crate) fn atlas_generation(&self) -> u64 {
        self.atlas.generation()
    }

    pub fn prepare_text(&mut self, request: TextLayoutRequest<'_>) -> TextResult<PreparedText> {
        let color = request.style.color;
        let shaped = self.shape_text(&request);
        self.prepare_shaped_text(&shaped, color)
    }

    pub(crate) fn shape_text(&mut self, request: &TextLayoutRequest<'_>) -> ShapedText {
        let font_size = request.style.font_size_px.max(1) as f32;
        let line_height = request.style.line_height_px.max(request.style.font_size_px) as f32;
        if request.text.is_empty() {
            return ShapedText {
                glyphs: Vec::new(),
                advance_width_px: 0.0,
                height_px: line_height,
                baseline_px: line_height * 0.8,
                line_count: 1,
            };
        }

        let mut buffer = Buffer::new(&mut self.font_system, Metrics::new(font_size, line_height));
        let mut buffer = buffer.borrow_with(&mut self.font_system);
        buffer.set_size(
            request
                .max_width_px
                .filter(|width| width.is_finite() && *width > 0.0),
            request
                .max_height_px
                .filter(|height| height.is_finite() && *height > 0.0),
        );
        let family = match request.style.font_family.as_str() {
            "serif" => Family::Serif,
            "sans-serif" | "sans_serif" => Family::SansSerif,
            "monospace" => Family::Monospace,
            "cursive" => Family::Cursive,
            "fantasy" => Family::Fantasy,
            name => Family::Name(name),
        };
        let attrs = Attrs::new()
            .family(family)
            .weight(Weight(request.style.font_weight));
        buffer.set_text(request.text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(false);

        let mut glyphs = Vec::new();
        let mut advance_width_px = 0.0_f32;
        let mut height_px = 0.0_f32;
        let mut baseline_px = line_height * 0.8;
        let mut line_count = 0_u32;
        for run in buffer.layout_runs() {
            if line_count == 0 {
                baseline_px = run.line_y;
            }
            line_count = line_count.saturating_add(1);
            advance_width_px = advance_width_px.max(run.line_w);
            height_px = height_px.max(run.line_top + run.line_height);
            for glyph in run.glyphs {
                glyphs.push(glyph.physical(
                    (0.0, run.line_y * self.raster_scale.get()),
                    self.raster_scale.get(),
                ));
            }
        }
        ShapedText {
            glyphs,
            advance_width_px,
            height_px: height_px.max(line_height),
            baseline_px,
            line_count: line_count.max(1),
        }
    }

    pub(crate) fn prepare_shaped_text(
        &mut self,
        shaped: &ShapedText,
        color: ColorRgba8,
    ) -> TextResult<PreparedText> {
        match self.prepare_physical_glyphs(shaped, color) {
            Ok(prepared) => Ok(prepared),
            Err(error) if error.is_atlas_full() => {
                self.atlas.clear();
                self.prepare_physical_glyphs(shaped, color)
            }
            Err(error) => Err(error),
        }
    }

    fn prepare_physical_glyphs(
        &mut self,
        shaped: &ShapedText,
        color: ColorRgba8,
    ) -> TextResult<PreparedText> {
        let mut glyphs = Vec::new();
        for glyph in &shaped.glyphs {
            let image = self
                .swash_cache
                .get_image(&mut self.font_system, glyph.cache_key)
                .clone();
            let Some(image) = image else {
                continue;
            };

            let width_px = image.placement.width as i32;
            let height_px = image.placement.height as i32;
            if width_px <= 0 || height_px <= 0 {
                continue;
            }

            let alpha = glyph_image_alpha(&image)?;
            let entry = self
                .atlas
                .get_or_insert(glyph.cache_key, width_px, height_px, &alpha)?;

            // Sample the transparent atlas gutter as part of the quad. Swash bitmaps use tight ink
            // bounds, so excluding this texel can terminate filtering on a nonzero edge pixel and
            // make rounded terminal glyphs look clipped.
            glyphs.push(AtlasGlyph {
                dst_x: glyph.x + image.placement.left - GLYPH_FILTER_GUTTER_PX,
                dst_y: glyph.y - image.placement.top - GLYPH_FILTER_GUTTER_PX,
                width_px: entry.width_px + GLYPH_FILTER_GUTTER_PX * 2,
                height_px: entry.height_px + GLYPH_FILTER_GUTTER_PX * 2,
                atlas_x: entry.x - GLYPH_FILTER_GUTTER_PX,
                atlas_y: entry.y - GLYPH_FILTER_GUTTER_PX,
                color,
            });
        }

        Ok(PreparedText {
            glyphs,
            advance_width_px: shaped.advance_width_px,
            atlas_version: self.atlas.version(),
            atlas_generation: self.atlas.generation(),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::core::ColorRgba8;

    use crate::text::{ResolvedTextStyle, TextEngine, TextLayoutRequest};

    #[test]
    fn font_seed_and_embedded_faces_are_shared_but_atlases_are_independent() {
        let mut first = TextEngine::with_atlas_size(128, 128).unwrap();
        let mut second = TextEngine::with_atlas_size(256, 256).unwrap();
        let seed = super::system_font_seed();
        assert_eq!(first.font_system.db().len(), seed.1.len());
        let face = seed.1.faces().next().expect("system test font").id;
        let bytes = seed
            .1
            .with_face_data(face, |data, _| data.to_vec())
            .unwrap();
        let bytes: &'static [u8] = Box::leak(bytes.into_boxed_slice());
        let a = super::embedded_faces(bytes).unwrap();
        let b = super::embedded_faces(bytes).unwrap();
        assert!(std::sync::Arc::ptr_eq(&a, &b));
        first.load_embedded_font(bytes).unwrap();
        assert_eq!(second.font_system.db().len(), seed.1.len());
        second.load_embedded_font(bytes).unwrap();
        let request = TextLayoutRequest {
            text: "Shared fonts",
            style: ResolvedTextStyle::new(ColorRgba8::rgba(255, 255, 255, 255), 16),
            max_width_px: None,
            max_height_px: None,
        };
        let untouched = second.atlas().pixels_a8.to_vec();
        let left = first.prepare_text(request.clone()).unwrap();
        assert_eq!(second.atlas().pixels_a8, untouched.as_slice());
        let right = second.prepare_text(request).unwrap();
        assert_eq!(left.advance_width_px, right.advance_width_px);
        assert_eq!(left.glyphs.len(), right.glyphs.len());
        assert!(super::embedded_faces(b"not a font").is_err());
    }

    #[test]
    fn density_changes_raster_detail_without_changing_wrapping_or_line_spacing() {
        let mut engine = TextEngine::with_atlas_size(512, 512).unwrap();
        let request = TextLayoutRequest {
            text: "Hello\nHello",
            style: ResolvedTextStyle::new(ColorRgba8::rgba(255, 255, 255, 255), 24),
            max_width_px: Some(100.0),
            max_height_px: None,
        };
        let logical = engine.shape_text(&request);
        let low = engine.prepare_text(request.clone()).unwrap();
        engine.set_raster_scale(crate::platform::ScaleFactor::new(2.0).unwrap());
        let dense = engine.shape_text(&request);
        let high = engine.prepare_text(request).unwrap();
        assert_eq!(logical.line_count, dense.line_count);
        assert_eq!(logical.advance_width_px, dense.advance_width_px);
        assert_eq!(logical.height_px, dense.height_px);
        assert_eq!(logical.baseline_px, dense.baseline_px);
        assert_eq!(low.advance_width_px, high.advance_width_px);
        assert_ne!(low.atlas_generation, high.atlas_generation);
        assert_eq!(low.glyphs.len(), high.glyphs.len());
        assert!(high.glyphs[0].height_px > low.glyphs[0].height_px);
        // Equal glyphs on successive lines isolate the baseline from rasterizer bearings.
        let per_line = low.glyphs.len() / 2;
        let low_spacing = low.glyphs[per_line].dst_y - low.glyphs[0].dst_y;
        let high_spacing = high.glyphs[per_line].dst_y - high.glyphs[0].dst_y;
        assert!((high_spacing - 2 * low_spacing).abs() <= 1);
    }

    #[test]
    fn prepares_text_into_atlas_glyphs() {
        let mut engine = TextEngine::with_atlas_size(256, 256).unwrap();
        let prepared = engine
            .prepare_text(TextLayoutRequest {
                text: "Telorgon",
                style: ResolvedTextStyle::new(ColorRgba8::rgba(0xff, 0xff, 0xff, 0xff), 16),
                max_width_px: Some(160.0),
                max_height_px: Some(32.0),
            })
            .unwrap();

        assert!(!prepared.glyphs.is_empty());
        assert!(prepared.atlas_version >= 1);
        assert!(engine.atlas().pixels_a8.iter().any(|alpha| *alpha > 0));
    }

    #[test]
    fn reuses_cached_glyphs_without_growing_atlas_version() {
        let mut engine = TextEngine::with_atlas_size(256, 256).unwrap();
        let request = TextLayoutRequest {
            text: "AA",
            style: ResolvedTextStyle::new(ColorRgba8::rgba(0xff, 0xff, 0xff, 0xff), 16),
            max_width_px: Some(160.0),
            max_height_px: Some(32.0),
        };

        engine.prepare_text(request.clone()).unwrap();
        let version = engine.atlas().version;
        engine.prepare_text(request).unwrap();

        assert_eq!(engine.atlas().version, version);
    }

    #[test]
    fn shaping_geometry_does_not_touch_the_atlas() {
        let mut engine = TextEngine::with_atlas_size(256, 256).unwrap();
        let version = engine.atlas().version;
        let shaped = engine.shape_text(&TextLayoutRequest {
            text: "Measured only",
            style: ResolvedTextStyle::new(ColorRgba8::rgba(255, 255, 255, 255), 16),
            max_width_px: None,
            max_height_px: None,
        });

        assert!(shaped.advance_width_px > 0.0);
        assert_eq!(engine.atlas().version, version);
        assert!(engine.atlas().pixels_a8.iter().all(|alpha| *alpha == 0));
    }

    #[test]
    fn prepared_glyph_quad_includes_a_transparent_filter_gutter() {
        let mut engine = TextEngine::with_atlas_size(256, 256).unwrap();
        let prepared = engine
            .prepare_text(TextLayoutRequest {
                text: "8",
                style: ResolvedTextStyle::new(ColorRgba8::rgba(255, 255, 255, 255), 48),
                max_width_px: None,
                max_height_px: None,
            })
            .unwrap();
        let glyph = prepared.glyphs.first().unwrap();
        let atlas = engine.atlas();
        let alpha = |x: i32, y: i32| atlas.pixels_a8[(y * atlas.width_px + x) as usize];

        assert!(glyph.atlas_x > 0);
        assert!(glyph.atlas_y > 0);
        assert!(glyph.atlas_x + glyph.width_px < atlas.width_px);
        assert!(glyph.atlas_y + glyph.height_px < atlas.height_px);

        for x in glyph.atlas_x..glyph.atlas_x + glyph.width_px {
            assert_eq!(alpha(x, glyph.atlas_y), 0);
            assert_eq!(alpha(x, glyph.atlas_y + glyph.height_px - 1), 0);
        }
        for y in glyph.atlas_y..glyph.atlas_y + glyph.height_px {
            assert_eq!(alpha(glyph.atlas_x, y), 0);
            assert_eq!(alpha(glyph.atlas_x + glyph.width_px - 1, y), 0);
        }
        for x in glyph.atlas_x - 1..=glyph.atlas_x + glyph.width_px {
            assert_eq!(alpha(x, glyph.atlas_y - 1), 0);
            assert_eq!(alpha(x, glyph.atlas_y + glyph.height_px), 0);
        }
        for y in glyph.atlas_y - 1..=glyph.atlas_y + glyph.height_px {
            assert_eq!(alpha(glyph.atlas_x - 1, y), 0);
            assert_eq!(alpha(glyph.atlas_x + glyph.width_px, y), 0);
        }
    }
}

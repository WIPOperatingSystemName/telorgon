use super::*;
use std::collections::HashMap;

fn prepare_node(
    ui: &MountedUi,
    layout: &LayoutEngine,
    text: &mut RetainedTextSystem,
    node: &NodeId,
    output: &mut Vec<GlyphInstance>,
) {
    let Some(computed) = layout.computed(*node).copied() else {
        return;
    };
    if computed.visible_rect.area() <= 0.0 {
        return;
    }
    let Some(visual) = ui.texts.get(*node) else {
        return;
    };
    let opacity = ui
        .box_styles
        .get(*node)
        .cloned()
        .unwrap_or_default()
        .opacity;
    let family = ui.string(visual.style.family).unwrap_or("sans-serif");
    let box_style = ui.box_styles.get(*node).cloned().unwrap_or_default();
    let (font_size, line_height) = visual
        .style
        .metrics(Some(computed.local_content_rect.height), &box_style);
    let max_width = positive_constraint(computed.local_content_rect.width);
    let max_height = positive_constraint(computed.local_content_rect.height);
    let key = TextRunKey::new(
        visual.revision,
        1,
        family,
        font_size,
        visual.style.weight,
        line_height,
        max_width,
        max_height,
        1.0,
    );
    if let Ok(run_id) = text.prepare(RetainedTextRequest {
        key,
        text: ui.string(visual.content).unwrap_or(""),
        family,
        font_size_px: font_size as i32,
        line_height_px: line_height as i32,
        max_width_px: max_width,
        max_height_px: max_height,
    }) && let Some(run) = text.run(run_id)
    {
        let alignment_offset_x = match visual.style.align {
            TextAlign::Start => 0.0,
            TextAlign::Center => (computed.local_content_rect.width - run.advance_width_px) * 0.5,
            TextAlign::End => computed.local_content_rect.width - run.advance_width_px,
        };
        let raster_scale = text.raster_scale().get();
        let (run_origin_x, run_origin_y) = snap_text_run_origin(
            computed.local_content_rect.x + alignment_offset_x,
            computed.local_content_rect.y
                + visual
                    .style
                    .vertical_offset(computed.local_content_rect.height, run.height_px),
            computed.world_transform,
            raster_scale,
        );
        for glyph in run.glyphs.iter() {
            let local_rect = crate::foundation::RectF {
                x: run_origin_x + glyph.dst_x as f32 / raster_scale,
                y: run_origin_y + glyph.dst_y as f32 / raster_scale,
                width: glyph.width_px as f32 / raster_scale,
                height: glyph.height_px as f32 / raster_scale,
            };
            output.push(GlyphInstance {
                node: *node,
                rect: local_rect,
                view_bounds: computed.world_transform.transform_rect(local_rect),
                atlas_x: glyph.atlas_x,
                atlas_y: glyph.atlas_y,
                atlas_size: SizeI {
                    width: glyph.width_px,
                    height: glyph.height_px,
                },
                color: visual.style.color,
                opacity,
                clip: computed.clip,
                spatial: computed.spatial,
            });
        }
    }
}

impl SceneCompiler {
    pub(super) fn update_glyphs(
        &mut self,
        ui: &MountedUi,
        layout: &LayoutEngine,
        text: &mut RetainedTextSystem,
        scene: &mut RenderScene,
        extent: SizeF,
        structural: bool,
        stats: &mut CompileStats,
    ) -> bool {
        let mut full = structural || self.atlas_generation != text.atlas_generation();
        for attempt in 0..3 {
            let generation = text.atlas_generation();
            let nodes = if full {
                &self.preorder
            } else {
                &self.work_scratch
            };
            let mut updates = HashMap::new();
            for node in nodes {
                if !ui.texts.contains(*node) {
                    continue;
                }
                let mut glyphs = Vec::new();
                prepare_node(ui, layout, text, node, &mut glyphs);
                updates.insert(*node, glyphs);
            }
            if generation != text.atlas_generation() {
                full = true;
                if attempt < 2 {
                    continue;
                }
                // Never publish coordinates invalidated by atlas eviction.
                for glyphs in updates.values_mut() {
                    glyphs.clear();
                }
            }
            self.atlas_generation = text.atlas_generation();
            let repack = full
                || updates.iter().any(|(node, glyphs)| {
                    self.glyph_ranges.get(node).map_or(0, |range| range.len()) != glyphs.len()
                });
            if repack {
                self.glyph_scratch.clear();
                let mut ranges = HashMap::new();
                for node in &self.preorder {
                    if !ui.texts.contains(*node) {
                        continue;
                    }
                    let start = self.glyph_scratch.len();
                    if let Some(glyphs) = updates.get(node) {
                        self.glyph_scratch.extend_from_slice(glyphs);
                    } else if let Some(range) = self.glyph_ranges.get(node) {
                        self.glyph_scratch
                            .extend_from_slice(&scene.glyphs[range.clone()]);
                    }
                    ranges.insert(*node, start..self.glyph_scratch.len());
                }
                if self.glyph_scratch != scene.glyphs {
                    for glyph in scene.glyphs.iter().chain(self.glyph_scratch.iter()) {
                        scene.damage.add(glyph.view_bounds, extent);
                    }
                    stats.glyphs_patched = self.glyph_scratch.len() as u64;
                }
                self.glyph_ranges = ranges;
                self.glyph_scratch = scene.replace_glyphs(std::mem::take(&mut self.glyph_scratch));
                return true;
            }
            let mut reorder = false;
            for (node, glyphs) in updates {
                let Some(range) = self.glyph_ranges.get(&node) else {
                    continue;
                };
                for (offset, glyph) in glyphs.iter().enumerate() {
                    let index = range.start + offset;
                    let old = &scene.glyphs[index];
                    if old == glyph {
                        continue;
                    }
                    reorder |= old.clip != glyph.clip;
                    scene.damage.add(old.view_bounds, extent);
                    scene.damage.add(glyph.view_bounds, extent);
                    scene.patch_glyph(index, *glyph);
                    stats.glyphs_patched += 1;
                }
            }
            return reorder;
        }
        false
    }
}

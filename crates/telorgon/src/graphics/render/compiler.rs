use crate::foundation::{ColorRgba8, RectF, SizeF, SizeI};
use crate::ui::layout::LayoutEngine;
use crate::graphics::scene::{DirtyFlags, NodeId};
use crate::ui::text::{RetainedTextRequest, RetainedTextSystem, TextRunKey};
use crate::ui::{Background, MountedUi, TextAlign};

use crate::graphics::render::{
    BatchKey, BlendMode, BoxInstance, DrawItem, GlyphInstance, ImageInstance, PipelineKind,
    PrimitiveKind, RenderClip, RenderScene, RenderSpatialNode,
};

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct CompileStats {
    pub visited: u64,
    pub boxes_patched: u64,
    pub glyphs_patched: u64,
    pub images_patched: u64,
    pub damage_rects: u64,
}

#[derive(Default)]
pub struct SceneCompiler {
    preorder: Vec<NodeId>,
    work_scratch: Vec<NodeId>,
    owner_scratch: Vec<NodeId>,
    glyph_scratch: Vec<GlyphInstance>,
    order_scratch: Vec<DrawItem>,
    initialized: bool,
    atlas_generation: u64,
}
impl SceneCompiler {
    pub fn compile(
        &mut self,
        ui: &mut MountedUi,
        layout: &LayoutEngine,
        text: &mut RetainedTextSystem,
        scene: &mut RenderScene,
        extent: SizeF,
        background: ColorRgba8,
    ) -> CompileStats {
        #[cfg(feature = "instrumentation")]
        let _compile_span = crate::runtime::instrumentation::span!("scene.compile.detail");
        #[cfg(feature = "instrumentation")]
        let dirty_span = crate::runtime::instrumentation::span!("scene.dirty_scan");
        let mut stats = CompileStats::default();
        if scene.extent != extent || scene.background != background {
            scene.damage.full = true;
            scene.damage.rects.clear();
        }
        scene.extent = extent;
        scene.background = background;
        self.work_scratch.clear();
        self.work_scratch.extend(
            ui.nodes
                .dirty_nodes()
                .iter()
                .copied()
                .filter(|node| ui.nodes.contains(*node)),
        );
        let structural = !self.initialized
            || self.work_scratch.iter().any(|node| {
                ui.nodes
                    .core(*node)
                    .is_some_and(|core| core.dirty.intersects(DirtyFlags::DRAW_ORDER))
            });
        if structural {
            self.preorder.clear();
            self.preorder.extend_from_slice(ui.nodes.preorder());
            self.work_scratch.clear();
            self.work_scratch.extend_from_slice(&self.preorder);
        }
        stats.visited = self.work_scratch.len() as u64;

        let mut rebuild_order = structural;
        let mut rebuild_glyphs = structural
            || self.atlas_generation != text.atlas_generation()
            || self.work_scratch.iter().any(|node| {
                ui.texts.contains(*node)
                    && ui.nodes.core(*node).is_some_and(|core| {
                        core.dirty.intersects(
                            DirtyFlags::STYLE
                                | DirtyFlags::TEXT
                                | DirtyFlags::SPATIAL
                                | DirtyFlags::CLIP
                                | DirtyFlags::VISIBILITY
                                | DirtyFlags::PAINT,
                        )
                    })
            });
        #[cfg(feature = "instrumentation")]
        drop(dirty_span);

        #[cfg(feature = "instrumentation")]
        let primitive_span = crate::runtime::instrumentation::span!("scene.primitives.patch");
        if structural {
            self.owner_scratch.clear();
            self.owner_scratch.extend_from_slice(scene.boxes.owners());
            for owner in self.owner_scratch.iter().copied() {
                if !ui.nodes.contains(owner)
                    && let Some(old) = scene.boxes.remove(owner)
                {
                    scene.damage.add(old.view_bounds, extent);
                }
            }
            self.owner_scratch.clear();
            self.owner_scratch.extend_from_slice(scene.images.owners());
            for owner in self.owner_scratch.iter().copied() {
                if !ui.nodes.contains(owner)
                    && let Some(old) = scene.images.remove(owner)
                {
                    scene.damage.add(old.view_bounds, extent);
                }
            }
            self.owner_scratch.clear();
            self.owner_scratch.extend_from_slice(scene.clips.owners());
            for owner in self.owner_scratch.iter().copied() {
                if !ui.nodes.contains(owner) {
                    scene.clips.remove(owner);
                }
            }
            self.owner_scratch.clear();
            self.owner_scratch
                .extend_from_slice(scene.spatial_nodes.owners());
            for owner in self.owner_scratch.iter().copied() {
                if !ui.nodes.contains(owner) {
                    scene.spatial_nodes.remove(owner);
                }
            }
        }

        for node in &self.work_scratch {
            let Some(computed) = layout.computed(*node).copied() else {
                continue;
            };
            let visible = computed.visible_rect.area() > 0.0;
            if !visible {
                if let Some(old) = scene.boxes.remove(*node) {
                    scene.damage.add(old.view_bounds, extent);
                    rebuild_order = true;
                }
                if let Some(old) = scene.images.remove(*node) {
                    scene.damage.add(old.view_bounds, extent);
                    rebuild_order = true;
                }
                scene.clips.remove(*node);
                scene.spatial_nodes.remove(*node);
                rebuild_glyphs |= ui.texts.contains(*node);
                continue;
            }
            let style = ui.box_styles.get(*node).cloned().unwrap_or_default();
            let background = match style.decoration.background {
                Background::None => None,
                Background::Color(color) => Some(color),
            };
            if has_box_visual(&style) {
                let view_bounds =
                    box_visual_bounds(computed.local_border_rect, computed.world_transform, &style);
                let instance = BoxInstance {
                    node: *node,
                    rect: computed.local_border_rect,
                    view_bounds,
                    background,
                    border: style.decoration.border,
                    outline: style.decoration.outline,
                    corner_radii: style.decoration.corner_radii,
                    shadows: style.decoration.shadows,
                    opacity: style.opacity,
                    // A box's own overflow clip applies to descendants, never to its outside
                    // focus outline or shadow. Its visual is constrained only by ancestors.
                    clip: ui
                        .nodes
                        .core(*node)
                        .and_then(|core| core.parent)
                        .and_then(|parent| layout.computed(parent).map(|layout| layout.clip))
                        .unwrap_or_default(),
                    spatial: computed.spatial,
                };
                let old = scene.boxes.get(*node).cloned();
                if scene.boxes.upsert(*node, instance) {
                    rebuild_order |= old.is_none();
                    if let Some(old) = old {
                        scene.damage.add(old.view_bounds, extent);
                    }
                    scene.damage.add(view_bounds, extent);
                    stats.boxes_patched += 1;
                }
            } else if let Some(old) = scene.boxes.remove(*node) {
                scene.damage.add(old.view_bounds, extent);
                rebuild_order = true;
            }
            if computed.clip.0 == node.index() + 1 {
                scene.clips.upsert(
                    *node,
                    RenderClip {
                        id: computed.clip,
                        rect: computed.visible_rect,
                        corner_radii: style.decoration.corner_radii,
                    },
                );
            } else {
                scene.clips.remove(*node);
            }
            scene.spatial_nodes.upsert(
                *node,
                RenderSpatialNode {
                    id: computed.spatial,
                    transform: computed.world_transform,
                },
            );

            if let Some(visual) = ui.images.get(*node) {
                let image = ImageInstance {
                    node: *node,
                    image: visual.image,
                    tint: visual.tint,
                    rect: computed.local_content_rect,
                    view_bounds: computed.content_rect,
                    content_version: visual.content_version,
                    opacity: style.opacity,
                    clip: computed.clip,
                    spatial: computed.spatial,
                };
                let old = scene.images.get(*node).copied();
                if scene.images.upsert(*node, image) {
                    // The draw batch carries the texture and clip identities. Replacing
                    // an image on the same mounted node must update that batch too.
                    rebuild_order |=
                        old.is_none_or(|old| old.image != image.image || old.clip != image.clip);
                    if let Some(old) = old {
                        scene.damage.add(old.view_bounds, extent);
                    }
                    scene.damage.add(image.view_bounds, extent);
                    stats.images_patched += 1;
                }
            } else if let Some(old) = scene.images.remove(*node) {
                scene.damage.add(old.view_bounds, extent);
                rebuild_order = true;
            }
        }
        #[cfg(feature = "instrumentation")]
        drop(primitive_span);

        #[cfg(feature = "instrumentation")]
        let text_span = crate::runtime::instrumentation::span!("scene.text.prepare");
        if rebuild_glyphs {
            let mut atlas_rebuilds = 0_u8;
            loop {
                let generation_before = text.atlas_generation();
                self.glyph_scratch.clear();
                for node in &self.preorder {
                    let Some(computed) = layout.computed(*node).copied() else {
                        continue;
                    };
                    if computed.visible_rect.area() <= 0.0 {
                        continue;
                    }
                    let Some(visual) = ui.texts.get(*node) else {
                        continue;
                    };
                    let opacity = ui
                        .box_styles
                        .get(*node)
                        .cloned()
                        .unwrap_or_default()
                        .opacity;
                    let family = ui.string(visual.style.family).unwrap_or("sans-serif");
                    let font_size = visual.style.size.ceil().max(1.0);
                    let line_height = visual.style.line_height.ceil().max(font_size);
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
                            TextAlign::Center => {
                                (computed.local_content_rect.width - run.advance_width_px) * 0.5
                            }
                            TextAlign::End => {
                                computed.local_content_rect.width - run.advance_width_px
                            }
                        };
                        let raster_scale = text.raster_scale().get();
                        let (run_origin_x, run_origin_y) = snap_text_run_origin(
                            computed.local_content_rect.x + alignment_offset_x,
                            computed.local_content_rect.y,
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
                            self.glyph_scratch.push(GlyphInstance {
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
                let generation_after = text.atlas_generation();
                if generation_after == generation_before {
                    self.atlas_generation = generation_after;
                    break;
                }
                atlas_rebuilds = atlas_rebuilds.saturating_add(1);
                if atlas_rebuilds >= 2 {
                    // The live glyph working set cannot remain resident in the fixed atlas.
                    // Publishing no glyph instances is safer than publishing stale coordinates.
                    self.glyph_scratch.clear();
                    self.atlas_generation = generation_after;
                    break;
                }
            }
            let glyphs = std::mem::take(&mut self.glyph_scratch);
            if scene.glyphs != glyphs {
                for glyph in &scene.glyphs {
                    scene.damage.add(glyph.view_bounds, extent);
                }
                for glyph in &glyphs {
                    scene.damage.add(glyph.view_bounds, extent);
                }
                stats.glyphs_patched = glyphs.len() as u64;
            }
            self.glyph_scratch = scene.replace_glyphs(glyphs);
            rebuild_order = true;
        }
        let atlas = text.atlas();
        let atlas_extent = SizeI {
            width: atlas.width_px,
            height: atlas.height_px,
        };
        let updates = text.take_atlas_updates();
        scene.set_atlas_updates(atlas_extent, updates);
        #[cfg(feature = "instrumentation")]
        drop(text_span);

        #[cfg(feature = "instrumentation")]
        let order_span = crate::runtime::instrumentation::span!("scene.draw_order");
        if rebuild_order {
            self.order_scratch.clear();
            for node in &self.preorder {
                if let Some(index) = scene.boxes.index(*node) {
                    self.order_scratch.push(DrawItem {
                        kind: PrimitiveKind::Box,
                        index: index as u32,
                        batch: BatchKey {
                            pipeline: PipelineKind::AnalyticBox,
                            resource: 0,
                            clip: scene.boxes.values()[index].clip,
                            blend: BlendMode::Alpha,
                            target: 0,
                        },
                    });
                }
                for (index, glyph) in scene
                    .glyphs
                    .iter()
                    .enumerate()
                    .filter(|(_, glyph)| glyph.node == *node)
                {
                    self.order_scratch.push(DrawItem {
                        kind: PrimitiveKind::Glyph,
                        index: index as u32,
                        batch: BatchKey {
                            pipeline: PipelineKind::Glyph,
                            resource: 0,
                            clip: glyph.clip,
                            blend: BlendMode::Alpha,
                            target: 0,
                        },
                    });
                }
                if let Some(index) = scene.images.index(*node) {
                    let image = scene.images.values()[index];
                    self.order_scratch.push(DrawItem {
                        kind: PrimitiveKind::Image,
                        index: index as u32,
                        batch: BatchKey {
                            pipeline: PipelineKind::Image,
                            resource: image.image.0,
                            clip: image.clip,
                            blend: BlendMode::Alpha,
                            target: 0,
                        },
                    });
                }
            }
            let order = std::mem::take(&mut self.order_scratch);
            self.order_scratch = scene.replace_draw_order(order);
        }
        #[cfg(feature = "instrumentation")]
        drop(order_span);
        stats.damage_rects = scene.damage.rects.len() as u64 + u64::from(scene.damage.full);
        #[cfg(feature = "instrumentation")]
        {
            crate::runtime::instrumentation::counter!("scene.nodes.visited", stats.visited);
            crate::runtime::instrumentation::counter!("scene.boxes.patched", stats.boxes_patched);
            crate::runtime::instrumentation::counter!("scene.glyphs.patched", stats.glyphs_patched);
            crate::runtime::instrumentation::counter!("scene.images.patched", stats.images_patched);
            crate::runtime::instrumentation::counter!("scene.damage.rects", stats.damage_rects);
        }
        for node in self.work_scratch.iter().copied() {
            ui.nodes.clear_dirty(
                node,
                DirtyFlags::STYLE
                    | DirtyFlags::DRAW_ORDER
                    | DirtyFlags::TEXT
                    | DirtyFlags::SPATIAL
                    | DirtyFlags::CLIP
                    | DirtyFlags::VISIBILITY
                    | DirtyFlags::PAINT
                    | DirtyFlags::SEMANTICS,
            );
        }
        ui.nodes.compact_dirty();
        self.initialized = true;
        stats
    }
}

fn has_box_visual(style: &crate::ui::BoxStyle) -> bool {
    style.opacity > 0.0
        && (style.decoration.background != Background::None
            || style.decoration.border.top.width > 0.0
            || style.decoration.border.right.width > 0.0
            || style.decoration.border.bottom.width > 0.0
            || style.decoration.border.left.width > 0.0
            || style.decoration.outline.width > 0.0
            || !style.decoration.shadows.as_slice().is_empty())
}

fn positive_constraint(value: f32) -> Option<f32> {
    (value.is_finite() && value > 0.0).then_some(value)
}

/// Snaps an unscaled, axis-aligned text run as one unit in view space.
///
/// Atlas glyphs have integer texel origins and extents. Keeping the run origin on a device pixel
/// prevents linear atlas sampling from blending adjacent texels, while preserving every shaped
/// glyph offset and therefore kerning. Transformed text remains unsnapped because rotation or
/// scaling needs continuous placement rather than pixel-grid alignment.
fn snap_text_run_origin(
    x: f32,
    y: f32,
    transform: crate::foundation::Affine2D,
    raster_scale: f32,
) -> (f32, f32) {
    const EPSILON: f32 = 1.0e-6;
    let translation_only = (transform.m11 - 1.0).abs() <= EPSILON
        && transform.m12.abs() <= EPSILON
        && transform.m21.abs() <= EPSILON
        && (transform.m22 - 1.0).abs() <= EPSILON;
    if translation_only {
        (
            ((x + transform.tx) * raster_scale).round() / raster_scale - transform.tx,
            ((y + transform.ty) * raster_scale).round() / raster_scale - transform.ty,
        )
    } else {
        (x, y)
    }
}

fn box_visual_bounds(
    rect: RectF,
    transform: crate::foundation::Affine2D,
    style: &crate::ui::BoxStyle,
) -> RectF {
    let outline = (style.decoration.outline.offset + style.decoration.outline.width).max(0.0);
    let mut local = RectF {
        x: rect.x - outline,
        y: rect.y - outline,
        width: rect.width + outline * 2.0,
        height: rect.height + outline * 2.0,
    };
    for shadow in style.decoration.shadows.as_slice() {
        let reach = (shadow.spread + shadow.blur * 2.0).max(0.0);
        local = local.union(RectF {
            x: rect.x + shadow.offset.x - reach,
            y: rect.y + shadow.offset.y - reach,
            width: rect.width + reach * 2.0,
            height: rect.height + reach * 2.0,
        });
    }
    transform.transform_rect(local)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::{ColorRgba8, SizeF};
    use crate::ui::layout::LayoutEngine;
    use crate::ui::text::RetainedTextSystem;
    use crate::ui::{
        Background, BoxStyle, InteractionFlags, LayoutStyle, MountWriter, MountedUi, SizeRule,
        StylePropertyPatch, TextAlign, TextStyle,
    };
    #[test]
    fn compiles_one_analytic_box_and_reuses_text_shape() {
        let mut ui = MountedUi::default();
        {
            let mut builder = MountWriter::<()>::new(&mut ui);
            builder.root(
                BoxStyle {
                    width: SizeRule::Fill(1.0),
                    height: SizeRule::Fill(1.0),
                    decoration: crate::ui::BoxDecoration {
                        background: Background::Color(ColorRgba8::rgba(1, 2, 3, 255)),
                        ..crate::ui::BoxDecoration::default()
                    },
                    ..BoxStyle::default()
                },
                LayoutStyle::default(),
                |builder| {
                    builder.text("Telorgon", ColorRgba8::rgba(255, 255, 255, 255), 14.0);
                },
            );
        }
        let extent = SizeF {
            width: 200.0,
            height: 100.0,
        };
        let mut layout = LayoutEngine::default();
        let mut text = RetainedTextSystem::new(4096).unwrap();
        layout.update(&mut ui, &mut text, extent, 1.0);
        let shaped_during_layout = text.stats().shaped;
        let mut scene = RenderScene::default();
        let mut compiler = SceneCompiler::default();
        compiler.compile(
            &mut ui,
            &layout,
            &mut text,
            &mut scene,
            extent,
            ColorRgba8::default(),
        );
        assert_eq!(text.stats().shaped, shaped_during_layout);
        let shaped = text.stats().shaped;
        let unchanged = compiler.compile(
            &mut ui,
            &layout,
            &mut text,
            &mut scene,
            extent,
            ColorRgba8::default(),
        );
        assert_eq!(text.stats().shaped, shaped);
        assert_eq!(unchanged.visited, 0);
        assert_eq!(unchanged.glyphs_patched, 0);
        assert_eq!(scene.boxes.len(), 1);
    }

    #[test]
    fn centered_text_uses_the_shaped_advance_instead_of_the_intrinsic_width_guess() {
        let mut ui = MountedUi::default();
        let mut button_node = None;
        let mut label_node = None;
        {
            let mut builder = MountWriter::<()>::new(&mut ui);
            builder.root(BoxStyle::default(), LayoutStyle::default(), |builder| {
                let button = builder.button(
                    (),
                    BoxStyle {
                        width: SizeRule::Logical(160.0),
                        height: SizeRule::Logical(44.0),
                        ..BoxStyle::default()
                    },
                    |builder| {
                        label_node = Some(
                            builder
                                .dynamic_text(
                                    "Increment",
                                    TextStyle {
                                        color: ColorRgba8::rgba(255, 255, 255, 255),
                                        size: 14.0,
                                        line_height: 17.5,
                                        family: crate::ui::StringId(1),
                                        weight: 400,
                                        align: TextAlign::Center,
                                    },
                                    BoxStyle::default(),
                                    LayoutStyle::default(),
                                )
                                .node,
                        );
                    },
                );
                button_node = Some(button.node);
            });
        }
        let button_node = button_node.unwrap();
        let label_node = label_node.unwrap();
        let extent = SizeF {
            width: 200.0,
            height: 100.0,
        };
        let mut layout = LayoutEngine::default();
        let mut text = RetainedTextSystem::new(4096).unwrap();
        layout.update(&mut ui, &mut text, extent, 1.0);
        let mut scene = RenderScene::default();
        SceneCompiler::default().compile(
            &mut ui,
            &layout,
            &mut text,
            &mut scene,
            extent,
            ColorRgba8::default(),
        );

        let button_rect = layout.computed(button_node).unwrap().content_rect;
        let button_center = button_rect.x + button_rect.width * 0.5;
        let glyph_bounds = scene
            .glyphs
            .iter()
            .filter(|glyph| glyph.node == label_node)
            .map(|glyph| glyph.view_bounds)
            .reduce(RectF::union)
            .expect("button label should produce glyphs");
        let glyph_center = glyph_bounds.x + glyph_bounds.width * 0.5;

        // Side bearings can make the ink differ slightly from the advance box. The large drift
        // caused by the old character-count estimate was several pixels for this label.
        assert!((glyph_center - button_center).abs() <= 1.5);
        assert!(
            scene
                .glyphs
                .iter()
                .filter(|glyph| glyph.node == label_node)
                .all(|glyph| glyph.view_bounds.x.fract() == 0.0
                    && glyph.view_bounds.y.fract() == 0.0),
            "centered glyph quads should remain aligned to the view pixel grid"
        );
    }

    #[test]
    fn text_run_snapping_preserves_glyph_offsets_and_skips_transformed_text() {
        let translated = crate::foundation::Affine2D::translation(0.25, 0.75);
        let snapped = snap_text_run_origin(10.4, 20.4, translated, 1.0);
        assert_eq!(snapped, (10.75, 20.25));
        assert_eq!(snapped.0 + translated.tx, 11.0);
        assert_eq!(snapped.1 + translated.ty, 21.0);

        let scaled = crate::foundation::Affine2D {
            m11: 1.5,
            m22: 1.5,
            ..crate::foundation::Affine2D::IDENTITY
        };
        assert_eq!(snap_text_run_origin(10.4, 20.4, scaled, 1.0), (10.4, 20.4));
    }

    #[test]
    fn interaction_flags_do_not_trigger_renderer_side_style_guesses() {
        let mut ui = MountedUi::default();
        let button = {
            let mut builder = MountWriter::<()>::new(&mut ui);
            let mut button = None;
            builder.root(BoxStyle::default(), LayoutStyle::default(), |builder| {
                button = Some(builder.button(
                    (),
                    BoxStyle {
                        width: SizeRule::Logical(100.0),
                        height: SizeRule::Logical(36.0),
                        decoration: crate::ui::BoxDecoration {
                            background: Background::Color(ColorRgba8::rgba(40, 80, 120, 255)),
                            ..crate::ui::BoxDecoration::default()
                        },
                        ..BoxStyle::default()
                    },
                    |_| {},
                ));
            });
            button.unwrap()
        };
        let extent = SizeF {
            width: 200.0,
            height: 100.0,
        };
        let mut layout = LayoutEngine::default();
        let mut text = RetainedTextSystem::new(128).unwrap();
        let mut scene = RenderScene::default();
        let mut compiler = SceneCompiler::default();
        layout.update(&mut ui, &mut text, extent, 1.0);
        compiler.compile(
            &mut ui,
            &layout,
            &mut text,
            &mut scene,
            extent,
            ColorRgba8::default(),
        );
        scene.take_delta();
        let normal = scene.boxes.get(button.node).unwrap().clone();

        ui.route_interaction_flag(button.node, InteractionFlags::HOVERED, true);
        layout.update(&mut ui, &mut text, extent, 1.0);
        compiler.compile(
            &mut ui,
            &layout,
            &mut text,
            &mut scene,
            extent,
            ColorRgba8::default(),
        );
        let hovered = scene.boxes.get(button.node).unwrap().clone();
        assert_eq!(hovered.background, normal.background);
        assert!(scene.damage.is_empty());
        scene.take_delta();

        ui.route_interaction_flag(button.node, InteractionFlags::PRESSED, true);
        layout.update(&mut ui, &mut text, extent, 1.0);
        compiler.compile(
            &mut ui,
            &layout,
            &mut text,
            &mut scene,
            extent,
            ColorRgba8::default(),
        );
        let pressed = scene.boxes.get(button.node).unwrap();
        assert_eq!(pressed.background, hovered.background);
        assert!(scene.damage.is_empty());
    }

    #[test]
    fn one_visual_update_in_ten_thousand_nodes_compiles_one_node() {
        let mut ui = MountedUi::default();
        let mut handles = Vec::with_capacity(10_000);
        {
            let mut builder = MountWriter::<()>::new(&mut ui);
            builder.root(
                BoxStyle {
                    width: SizeRule::Fill(1.0),
                    height: SizeRule::Fill(1.0),
                    ..BoxStyle::default()
                },
                LayoutStyle::default(),
                |builder| {
                    for _ in 0..10_000 {
                        handles.push(builder.button(
                            (),
                            BoxStyle {
                                width: SizeRule::Logical(10.0),
                                height: SizeRule::Logical(1.0),
                                decoration: crate::ui::BoxDecoration {
                                    background: Background::Color(ColorRgba8::rgba(1, 2, 3, 255)),
                                    ..crate::ui::BoxDecoration::default()
                                },
                                ..BoxStyle::default()
                            },
                            |_| {},
                        ));
                    }
                },
            );
        }
        let extent = SizeF {
            width: 100.0,
            height: 10_000.0,
        };
        let mut layout = LayoutEngine::default();
        let mut text = RetainedTextSystem::new(1024).unwrap();
        layout.update(&mut ui, &mut text, extent, 1.0);
        let mut scene = RenderScene::default();
        let mut compiler = SceneCompiler::default();
        compiler.compile(
            &mut ui,
            &layout,
            &mut text,
            &mut scene,
            extent,
            ColorRgba8::default(),
        );
        scene.take_delta();

        ui.transaction(|transaction| transaction.set(handles[5_000].opacity, 0.5));
        let layout_stats = layout.update(&mut ui, &mut text, extent, 1.0);
        let compile_stats = compiler.compile(
            &mut ui,
            &layout,
            &mut text,
            &mut scene,
            extent,
            ColorRgba8::default(),
        );
        assert_eq!(layout_stats.measured, 0);
        assert_eq!(layout_stats.arranged, 0);
        assert_eq!(compile_stats.visited, 1);
        assert_eq!(compile_stats.boxes_patched, 1);
        assert_eq!(compile_stats.glyphs_patched, 0);
    }

    #[test]
    fn sampled_transform_updates_the_retained_spatial_node_and_damage() {
        let mut ui = MountedUi::default();
        let child = {
            let mut builder = MountWriter::<()>::new(&mut ui);
            let mut child = None;
            builder.root(BoxStyle::default(), LayoutStyle::default(), |builder| {
                child = Some(builder.container(
                    BoxStyle {
                        width: SizeRule::Logical(20.0),
                        height: SizeRule::Logical(20.0),
                        decoration: crate::ui::BoxDecoration {
                            background: Background::Color(ColorRgba8::rgba(10, 20, 30, 255)),
                            ..crate::ui::BoxDecoration::default()
                        },
                        ..BoxStyle::default()
                    },
                    LayoutStyle::default(),
                    |_| {},
                ));
            });
            child.unwrap()
        };
        let extent = SizeF {
            width: 100.0,
            height: 100.0,
        };
        let mut layout = LayoutEngine::default();
        let mut text = RetainedTextSystem::new(128).unwrap();
        let mut scene = RenderScene::default();
        let mut compiler = SceneCompiler::default();
        layout.update(&mut ui, &mut text, extent, 1.0);
        compiler.compile(
            &mut ui,
            &layout,
            &mut text,
            &mut scene,
            extent,
            ColorRgba8::default(),
        );
        scene.take_delta().unwrap();
        let before = *scene.spatial_nodes.get(child).unwrap();

        assert!(ui.apply_style_patch(
            child,
            StylePropertyPatch {
                translation_x: Some(30.0),
                ..StylePropertyPatch::default()
            }
        ));
        layout.update(&mut ui, &mut text, extent, 1.0);
        let stats = compiler.compile(
            &mut ui,
            &layout,
            &mut text,
            &mut scene,
            extent,
            ColorRgba8::default(),
        );
        let after = *scene.spatial_nodes.get(child).unwrap();

        assert_ne!(after, before);
        assert!(stats.damage_rects > 0);
        assert!(scene.take_delta().is_some());
    }

    #[test]
    fn structural_removal_rebuilds_order_and_drops_retained_instances() {
        let mut ui = MountedUi::default();
        let child = {
            let mut builder = MountWriter::<()>::new(&mut ui);
            let mut saved = None;
            builder.root(
                BoxStyle {
                    decoration: crate::ui::BoxDecoration {
                        background: Background::Color(ColorRgba8::rgba(1, 1, 1, 255)),
                        ..crate::ui::BoxDecoration::default()
                    },
                    ..BoxStyle::default()
                },
                LayoutStyle::default(),
                |builder| {
                    saved = Some(builder.container(
                        BoxStyle {
                            decoration: crate::ui::BoxDecoration {
                                background: Background::Color(ColorRgba8::rgba(2, 2, 2, 255)),
                                ..crate::ui::BoxDecoration::default()
                            },
                            ..BoxStyle::default()
                        },
                        LayoutStyle::default(),
                        |_| {},
                    ));
                },
            );
            saved.unwrap()
        };
        let extent = SizeF {
            width: 100.0,
            height: 100.0,
        };
        let mut layout = LayoutEngine::default();
        let mut text = RetainedTextSystem::new(1024).unwrap();
        let mut scene = RenderScene::default();
        let mut compiler = SceneCompiler::default();
        layout.update(&mut ui, &mut text, extent, 1.0);
        compiler.compile(
            &mut ui,
            &layout,
            &mut text,
            &mut scene,
            extent,
            ColorRgba8::default(),
        );
        assert_eq!(scene.boxes.len(), 2);
        ui.transaction(|transaction| transaction.remove(child));
        layout.update(&mut ui, &mut text, extent, 1.0);
        let stats = compiler.compile(
            &mut ui,
            &layout,
            &mut text,
            &mut scene,
            extent,
            ColorRgba8::default(),
        );
        assert!(!ui.nodes.contains(child));
        assert_eq!(scene.boxes.len(), 1);
        assert_eq!(scene.draw_order.len(), 1);
        assert_eq!(stats.visited, 1);
    }
}

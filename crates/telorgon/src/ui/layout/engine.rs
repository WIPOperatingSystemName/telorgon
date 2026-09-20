//! Incremental canonical layout, spatial, clipping, hit-testing, and virtualization.

use crate::foundation::{Affine2D, EdgeInsets, PointF, RectF, SizeF};
use crate::graphics::scene::{DirtyFlags, NodeId, SparseSet};
use crate::ui::text::{RetainedTextRequest, RetainedTextSystem, TextRunKey};
use crate::ui::{
    BoxSizing, BoxStyle, CrossAxisAlignment, Flow, MainAxisAlignment, MountedUi, Overflow, SizeRule,
};

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ClipId(pub u32);

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct SpatialId(pub u32);

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct ComputedLayout {
    pub local_margin_rect: RectF,
    pub local_border_rect: RectF,
    pub local_padding_rect: RectF,
    pub local_content_rect: RectF,
    pub margin_rect: RectF,
    pub border_rect: RectF,
    pub padding_rect: RectF,
    pub content_rect: RectF,
    pub visible_rect: RectF,
    /// View-space clip inherited by descendants after this node's overflow policy.
    pub descendant_clip_rect: RectF,
    pub baseline: f32,
    pub clip: ClipId,
    pub spatial: SpatialId,
    pub world_transform: Affine2D,
}

#[derive(Copy, Clone, Debug, Default, PartialEq)]
struct LocalLayout {
    margin_rect: RectF,
    border_rect: RectF,
    padding_rect: RectF,
    content_rect: RectF,
    baseline: f32,
}

#[derive(Copy, Clone, Debug, Default, PartialEq)]
struct MeasuredText {
    size: SizeF,
    baseline: f32,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct LayoutDiagnostics {
    pub measured: u64,
    pub arranged: u64,
    pub spatial_updated: u64,
    pub cache_hits: u64,
    pub intrinsic_passes: u64,
}

#[derive(Default)]
pub struct LayoutEngine {
    local: SparseSet<LocalLayout>,
    computed: SparseSet<ComputedLayout>,
    world_transforms: SparseSet<Affine2D>,
    last_viewport: Option<SizeF>,
    last_scale_bits: u32,
    diagnostics: LayoutDiagnostics,
    preorder_scratch: Vec<NodeId>,
    dirty_scratch: Vec<NodeId>,
}

impl LayoutEngine {
    pub fn computed(&self, node: NodeId) -> Option<&ComputedLayout> {
        self.computed.get(node)
    }
    pub fn computed_nodes(&self) -> impl Iterator<Item = (NodeId, &ComputedLayout)> {
        self.computed.iter()
    }
    pub fn diagnostics(&self) -> LayoutDiagnostics {
        self.diagnostics
    }

    /// Updates canonical geometry using the same constraint-aware shaped-text cache used by paint.
    pub fn update(
        &mut self,
        ui: &mut MountedUi,
        text: &mut RetainedTextSystem,
        viewport: SizeF,
        scale: f32,
    ) -> LayoutDiagnostics {
        #[cfg(feature = "instrumentation")]
        let dirty_span = crate::runtime::instrumentation::span!("layout.dirty_scan");
        self.diagnostics = LayoutDiagnostics::default();
        let scale_bits = scale.to_bits();
        let viewport_changed =
            self.last_viewport != Some(viewport) || self.last_scale_bits != scale_bits;
        let mut dirty = std::mem::take(&mut self.dirty_scratch);
        dirty.clear();
        dirty.extend_from_slice(ui.nodes.dirty_nodes());
        let needs_layout = viewport_changed
            || dirty.iter().any(|node| {
                ui.nodes.core(*node).is_some_and(|core| {
                    core.dirty
                        .intersects(DirtyFlags::LAYOUT | DirtyFlags::STRUCTURE)
                })
            });
        let needs_spatial = needs_layout
            || dirty.iter().any(|node| {
                ui.nodes.core(*node).is_some_and(|core| {
                    core.dirty
                        .intersects(DirtyFlags::SPATIAL | DirtyFlags::CLIP | DirtyFlags::VISIBILITY)
                })
            });
        let mut preorder = std::mem::take(&mut self.preorder_scratch);
        preorder.clear();
        if needs_layout || needs_spatial {
            preorder.extend_from_slice(ui.nodes.preorder());
        }
        #[cfg(feature = "instrumentation")]
        drop(dirty_span);

        #[cfg(feature = "instrumentation")]
        let measure_span = crate::runtime::instrumentation::span!("layout.measure");
        if needs_layout {
            if let Some(root) = ui.root() {
                self.arrange_node(
                    ui,
                    text,
                    root.0,
                    RectF {
                        x: 0.0,
                        y: 0.0,
                        width: viewport.width.max(0.0),
                        height: viewport.height.max(0.0),
                    },
                    true,
                    scale,
                );
            }
        } else {
            self.diagnostics.cache_hits = ui.nodes.alive().len() as u64;
        }
        #[cfg(feature = "instrumentation")]
        drop(measure_span);
        if needs_spatial {
            #[cfg(feature = "instrumentation")]
            let _span = crate::runtime::instrumentation::span!("layout.spatial_clip");
            self.update_spatial(ui, &preorder, viewport);
        }
        if needs_layout {
            for node in preorder.iter().copied() {
                ui.nodes
                    .clear_dirty(node, DirtyFlags::STRUCTURE | DirtyFlags::LAYOUT);
            }
        } else {
            for node in dirty.iter().copied() {
                ui.nodes
                    .clear_dirty(node, DirtyFlags::STRUCTURE | DirtyFlags::LAYOUT);
            }
        }
        self.last_viewport = Some(viewport);
        self.last_scale_bits = scale_bits;
        self.preorder_scratch = preorder;
        self.dirty_scratch = dirty;
        #[cfg(feature = "instrumentation")]
        {
            crate::runtime::instrumentation::counter!("layout.nodes.measured", self.diagnostics.measured);
            crate::runtime::instrumentation::counter!("layout.nodes.arranged", self.diagnostics.arranged);
            crate::runtime::instrumentation::counter!("layout.spatial.updated", self.diagnostics.spatial_updated);
            crate::runtime::instrumentation::counter!("layout.cache.hits", self.diagnostics.cache_hits);
            crate::runtime::instrumentation::counter!("layout.intrinsic.passes", self.diagnostics.intrinsic_passes);
        }
        self.diagnostics
    }

    pub fn hit_test(&self, ui: &mut MountedUi, point: PointF) -> Option<NodeId> {
        ui.nodes.preorder().iter().rev().copied().find(|node| {
            ui.interactions.get(*node).is_some_and(|interaction| {
                interaction.visible
                    && interaction.enabled
                    && (interaction.focusable || interaction.listener_mask != 0)
            }) && self.computed.get(*node).is_some_and(|layout| {
                layout.visible_rect.contains(point)
                    && layout.world_transform.inverse().is_some_and(|inverse| {
                        layout
                            .local_border_rect
                            .contains(inverse.transform_point(point))
                    })
            })
        })
    }

    pub fn focus_order(&self, ui: &mut MountedUi) -> Vec<NodeId> {
        ui.nodes
            .preorder()
            .iter()
            .copied()
            .filter(|node| {
                ui.interactions
                    .get(*node)
                    .is_some_and(|item| item.visible && item.enabled && item.focusable)
                    && self
                        .computed
                        .get(*node)
                        .is_some_and(|layout| layout.visible_rect.area() > 0.0)
            })
            .collect()
    }

    fn arrange_node(
        &mut self,
        ui: &MountedUi,
        text: &mut RetainedTextSystem,
        node: NodeId,
        slot: RectF,
        is_root: bool,
        scale: f32,
    ) -> SizeF {
        let style = ui.box_styles.get(node).cloned().unwrap_or_default();
        let layout = ui.layouts.get(node).copied().unwrap_or_default();
        let children: Vec<_> = ui.nodes.children(node).collect();
        let intrinsic = self.intrinsic_size(ui, text, node, &style, &children, scale);
        let border_size = resolve_border_size(&style, slot, intrinsic, is_root);
        let margin_rect = RectF {
            x: slot.x,
            y: slot.y,
            width: border_size.width + style.margin.horizontal(),
            height: border_size.height + style.margin.vertical(),
        };
        let border_rect = RectF {
            x: slot.x + style.margin.left,
            y: slot.y + style.margin.top,
            width: border_size.width,
            height: border_size.height,
        };
        let border_insets = border_insets(&style);
        let padding_rect = inset(border_rect, border_insets);
        let content_rect = inset(padding_rect, style.padding);
        let measured_text = self.measure_text(
            ui,
            text,
            node,
            positive_constraint(content_rect.width),
            positive_constraint(content_rect.height),
            scale,
        );
        self.local.insert(
            node,
            LocalLayout {
                margin_rect,
                border_rect,
                padding_rect,
                content_rect,
                baseline: content_rect.y
                    + measured_text
                        .map(|measurement| measurement.baseline)
                        .unwrap_or(content_rect.height.min(16.0) * 0.8),
            },
        );
        self.diagnostics.measured += 1;
        self.diagnostics.arranged += 1;

        let gap = layout.gap.max(0.0);
        let gap_total = gap * children.len().saturating_sub(1) as f32;
        let main_available = match layout.flow {
            Flow::Horizontal => content_rect.width,
            Flow::Vertical | Flow::Overlay => content_rect.height,
        };
        let mut fixed_main = 0.0;
        let mut fill_weight = 0.0;
        if layout.flow != Flow::Overlay {
            for child in &children {
                let child_style = ui.box_styles.get(*child).copied().unwrap_or_default();
                let rule = match layout.flow {
                    Flow::Horizontal => child_style.width,
                    Flow::Vertical => child_style.height,
                    Flow::Overlay => unreachable!(),
                };
                let weight = fill_weight_of(rule);
                if weight > 0.0 {
                    fill_weight += weight;
                } else {
                    let child_children: Vec<_> = ui.nodes.children(*child).collect();
                    let intrinsic =
                        self.intrinsic_size(ui, text, *child, &child_style, &child_children, scale);
                    let estimate = resolve_border_size(
                        &child_style,
                        RectF {
                            width: content_rect.width,
                            height: content_rect.height,
                            ..RectF::ZERO
                        },
                        intrinsic,
                        false,
                    );
                    fixed_main += match layout.flow {
                        Flow::Horizontal => estimate.width + child_style.margin.horizontal(),
                        Flow::Vertical => estimate.height + child_style.margin.vertical(),
                        Flow::Overlay => 0.0,
                    };
                }
            }
        }
        let fill_available = (main_available - fixed_main - gap_total).max(0.0);
        let occupied_main = fixed_main
            + gap_total
            + if fill_weight > 0.0 {
                fill_available
            } else {
                0.0
            };
        let mut cursor =
            main_axis_offset(layout.main_axis_alignment, main_available, occupied_main);
        for child in children {
            let child_style = ui.box_styles.get(child).cloned().unwrap_or_default();
            let child_children: Vec<_> = ui.nodes.children(child).collect();
            let intrinsic =
                self.intrinsic_size(ui, text, child, &child_style, &child_children, scale);
            let estimated = resolve_border_size(
                &child_style,
                RectF {
                    width: content_rect.width,
                    height: content_rect.height,
                    ..RectF::ZERO
                },
                intrinsic,
                false,
            );
            let estimated_margin = SizeF {
                width: estimated.width + child_style.margin.horizontal(),
                height: estimated.height + child_style.margin.vertical(),
            };
            let child_slot = match layout.flow {
                Flow::Horizontal => RectF {
                    x: cursor,
                    y: cross_axis_offset(
                        layout.cross_axis_alignment,
                        content_rect.height,
                        estimated_margin.height,
                    ),
                    width: if fill_weight_of(child_style.width) > 0.0 {
                        fill_available * fill_weight_of(child_style.width) / fill_weight
                    } else {
                        content_rect.width
                    },
                    height: content_rect.height,
                },
                Flow::Vertical => RectF {
                    x: cross_axis_offset(
                        layout.cross_axis_alignment,
                        content_rect.width,
                        estimated_margin.width,
                    ),
                    y: cursor,
                    width: content_rect.width,
                    height: if fill_weight_of(child_style.height) > 0.0 {
                        fill_available * fill_weight_of(child_style.height) / fill_weight
                    } else {
                        content_rect.height
                    },
                },
                Flow::Overlay => RectF {
                    x: cross_axis_offset(
                        layout.cross_axis_alignment,
                        content_rect.width,
                        estimated_margin.width,
                    ),
                    y: main_axis_offset(
                        layout.main_axis_alignment,
                        content_rect.height,
                        estimated_margin.height,
                    ),
                    width: content_rect.width,
                    height: content_rect.height,
                },
            };
            let arranged = self.arrange_node(ui, text, child, child_slot, false, scale);
            cursor += match layout.flow {
                Flow::Horizontal => arranged.width,
                Flow::Vertical => arranged.height,
                Flow::Overlay => 0.0,
            } + gap;
        }
        SizeF {
            width: margin_rect.width,
            height: margin_rect.height,
        }
    }

    fn intrinsic_size(
        &mut self,
        ui: &MountedUi,
        text_system: &mut RetainedTextSystem,
        node: NodeId,
        _style: &BoxStyle,
        children: &[NodeId],
        scale: f32,
    ) -> SizeF {
        self.diagnostics.intrinsic_passes += 1;
        if let Some(measurement) = self.measure_text(ui, text_system, node, None, None, scale) {
            return measurement.size;
        }
        if children.is_empty() {
            return SizeF {
                width: 24.0,
                height: 24.0,
            };
        }
        SizeF {
            width: 100.0,
            height: 32.0 * children.len() as f32,
        }
    }

    fn measure_text(
        &self,
        ui: &MountedUi,
        text_system: &mut RetainedTextSystem,
        node: NodeId,
        max_width: Option<f32>,
        max_height: Option<f32>,
        scale: f32,
    ) -> Option<MeasuredText> {
        let visual = ui.texts.get(node)?;
        let content = ui.string(visual.content).unwrap_or("");
        let family = ui.string(visual.style.family).unwrap_or("sans-serif");
        let font_size = visual.style.size.ceil().max(1.0);
        let line_height = visual.style.line_height.ceil().max(font_size);
        let key = TextRunKey::new(
            visual.revision,
            1,
            family,
            font_size,
            visual.style.weight,
            line_height,
            max_width,
            max_height,
            scale,
        );
        let id = text_system
            .measure(RetainedTextRequest {
                key,
                text: content,
                family,
                font_size_px: font_size as i32,
                line_height_px: line_height as i32,
                max_width_px: max_width,
                max_height_px: max_height,
            })
            .ok()?;
        let run = text_system.run(id)?;
        Some(MeasuredText {
            size: SizeF {
                width: run.advance_width_px,
                height: run.height_px,
            },
            baseline: run.baseline,
        })
    }

    fn update_spatial(&mut self, ui: &mut MountedUi, preorder: &[NodeId], viewport: SizeF) {
        let viewport_rect = RectF {
            x: 0.0,
            y: 0.0,
            width: viewport.width,
            height: viewport.height,
        };
        for node in preorder {
            let Some(local) = self.local.get(*node).copied() else {
                continue;
            };
            let style = ui.box_styles.get(*node).cloned().unwrap_or_default();
            let node_visible = ui
                .interactions
                .get(*node)
                .is_none_or(|interaction| interaction.visible);
            let parent = ui.nodes.core(*node).and_then(|core| core.parent);
            let (basis, inherited_clip, inherited_clip_id) = if let Some(parent) = parent {
                let parent_world = self.computed.get(parent).copied().unwrap_or_default();
                let parent_local = self.local.get(parent).copied().unwrap_or_default();
                let scroll = ui
                    .layouts
                    .get(parent)
                    .map(|layout| layout.scroll_offset)
                    .unwrap_or_default();
                (
                    parent_world.world_transform.then(Affine2D::translation(
                        parent_local.content_rect.x - scroll.x,
                        parent_local.content_rect.y - scroll.y,
                    )),
                    parent_world.descendant_clip_rect,
                    parent_world.clip,
                )
            } else {
                (Affine2D::IDENTITY, viewport_rect, ClipId(0))
            };
            let world_transform = basis.then(style.transform.affine_for_rect(local.border_rect));
            let border_rect = world_transform.transform_rect(local.border_rect);
            let descendant_clip_rect = if !node_visible {
                RectF::ZERO
            } else if style.overflow == Overflow::Visible {
                inherited_clip
            } else {
                inherited_clip
                    .intersection(border_rect)
                    .unwrap_or(RectF::ZERO)
            };
            // A node's own overflow clips descendants, not its border/background/outline.
            // Its visual is constrained only by visibility and ancestor clips.
            let visible_rect = if node_visible {
                inherited_clip
                    .intersection(border_rect)
                    .unwrap_or(RectF::ZERO)
            } else {
                RectF::ZERO
            };
            let clip = if style.overflow == Overflow::Visible {
                inherited_clip_id
            } else {
                ClipId(node.index() + 1)
            };
            let computed = ComputedLayout {
                local_margin_rect: local.margin_rect,
                local_border_rect: local.border_rect,
                local_padding_rect: local.padding_rect,
                local_content_rect: local.content_rect,
                margin_rect: world_transform.transform_rect(local.margin_rect),
                border_rect,
                padding_rect: world_transform.transform_rect(local.padding_rect),
                content_rect: world_transform.transform_rect(local.content_rect),
                visible_rect,
                descendant_clip_rect,
                baseline: world_transform
                    .transform_point(PointF {
                        x: local.content_rect.x,
                        y: local.baseline,
                    })
                    .y,
                clip,
                spatial: SpatialId(node.index() + 1),
                world_transform,
            };
            self.world_transforms.insert(*node, world_transform);
            if self.computed.get(*node) != Some(&computed) {
                self.computed.insert(*node, computed);
                ui.nodes.mark_dirty(
                    *node,
                    DirtyFlags::SPATIAL | DirtyFlags::CLIP | DirtyFlags::PAINT,
                );
                self.diagnostics.spatial_updated += 1;
            }
        }
    }
}

fn resolve_border_size(style: &BoxStyle, slot: RectF, intrinsic: SizeF, is_root: bool) -> SizeF {
    let border = border_insets(style);
    let chrome = SizeF {
        width: border.horizontal() + style.padding.horizontal(),
        height: border.vertical() + style.padding.vertical(),
    };
    let available_border = SizeF {
        width: (slot.width - style.margin.horizontal()).max(0.0),
        height: (slot.height - style.margin.vertical()).max(0.0),
    };
    if is_root {
        return available_border;
    }
    let available_specified = match style.sizing {
        BoxSizing::BorderBox => available_border,
        BoxSizing::ContentBox => SizeF {
            width: (available_border.width - chrome.width).max(0.0),
            height: (available_border.height - chrome.height).max(0.0),
        },
    };
    let intrinsic_specified = match style.sizing {
        BoxSizing::BorderBox => SizeF {
            width: intrinsic.width + chrome.width,
            height: intrinsic.height + chrome.height,
        },
        BoxSizing::ContentBox => intrinsic,
    };
    let specified = SizeF {
        width: constrain_size(
            resolve_size(
                style.width,
                available_specified.width,
                intrinsic_specified.width,
            ),
            style.min_size.width,
            style.max_size.width,
            available_specified.width,
            intrinsic_specified.width,
        ),
        height: constrain_size(
            resolve_size(
                style.height,
                available_specified.height,
                intrinsic_specified.height,
            ),
            style.min_size.height,
            style.max_size.height,
            available_specified.height,
            intrinsic_specified.height,
        ),
    };
    match style.sizing {
        BoxSizing::BorderBox => specified,
        BoxSizing::ContentBox => SizeF {
            width: specified.width + chrome.width,
            height: specified.height + chrome.height,
        },
    }
}

fn constrain_size(
    value: f32,
    min_rule: SizeRule,
    max_rule: SizeRule,
    available: f32,
    intrinsic: f32,
) -> f32 {
    let minimum = match min_rule {
        SizeRule::Shrink => 0.0,
        rule => resolve_size(rule, available, intrinsic),
    };
    let maximum = match max_rule {
        SizeRule::Shrink => intrinsic,
        rule => resolve_size(rule, available, intrinsic),
    }
    .max(minimum);
    value.max(minimum).min(maximum)
}

fn fill_weight_of(rule: SizeRule) -> f32 {
    match rule {
        SizeRule::Fill(weight) => weight.max(0.0),
        _ => 0.0,
    }
}

fn main_axis_offset(alignment: MainAxisAlignment, available: f32, occupied: f32) -> f32 {
    let free = (available - occupied).max(0.0);
    match alignment {
        MainAxisAlignment::Start => 0.0,
        MainAxisAlignment::Center => free * 0.5,
        MainAxisAlignment::End => free,
    }
}

fn cross_axis_offset(alignment: CrossAxisAlignment, available: f32, occupied: f32) -> f32 {
    let free = (available - occupied).max(0.0);
    match alignment {
        CrossAxisAlignment::Start => 0.0,
        CrossAxisAlignment::Center => free * 0.5,
        CrossAxisAlignment::End => free,
    }
}

fn border_insets(style: &BoxStyle) -> EdgeInsets {
    EdgeInsets {
        top: style.decoration.border.top.width.max(0.0),
        right: style.decoration.border.right.width.max(0.0),
        bottom: style.decoration.border.bottom.width.max(0.0),
        left: style.decoration.border.left.width.max(0.0),
    }
}

fn resolve_size(rule: SizeRule, available: f32, intrinsic: f32) -> f32 {
    match rule {
        SizeRule::Logical(value) => value.max(0.0),
        SizeRule::Percent(value) => available * value.clamp(0.0, 1.0),
        SizeRule::Fill(weight) => {
            if weight > 0.0 {
                available
            } else {
                0.0
            }
        }
        SizeRule::Shrink => intrinsic.min(available),
    }
}

fn positive_constraint(value: f32) -> Option<f32> {
    (value.is_finite() && value > 0.0).then_some(value)
}

fn inset(rect: RectF, insets: EdgeInsets) -> RectF {
    RectF {
        x: rect.x + insets.left,
        y: rect.y + insets.top,
        width: (rect.width - insets.horizontal()).max(0.0),
        height: (rect.height - insets.vertical()).max(0.0),
    }
}

#[derive(Clone, Debug)]
pub struct VirtualCollection {
    extents: Vec<f32>,
    prefix: Vec<f32>,
    estimate: f32,
    overscan: f32,
}
impl VirtualCollection {
    pub fn new(item_count: usize, estimate: f32, overscan: f32) -> Self {
        let estimate = estimate.max(1.0);
        let mut collection = Self {
            extents: vec![estimate; item_count],
            prefix: Vec::with_capacity(item_count + 1),
            estimate,
            overscan: overscan.max(0.0),
        };
        collection.rebuild_prefix(0);
        collection
    }
    pub fn set_extent(&mut self, index: usize, extent: f32) {
        if let Some(slot) = self.extents.get_mut(index) {
            *slot = extent.max(1.0);
            self.rebuild_prefix(index);
        }
    }
    pub fn visible_range(&self, offset: f32, viewport: f32) -> std::ops::Range<usize> {
        let start_position = (offset - self.overscan).max(0.0);
        let end_position = offset + viewport + self.overscan;
        let start = self
            .prefix
            .partition_point(|value| *value <= start_position)
            .saturating_sub(1)
            .min(self.extents.len());
        let end = self
            .prefix
            .partition_point(|value| *value < end_position)
            .min(self.extents.len());
        start..end.max(start)
    }
    pub fn total_extent(&self) -> f32 {
        self.prefix.last().copied().unwrap_or(0.0)
    }
    /// Returns one item's measured-or-estimated content-space extent.
    pub fn item_range(&self, index: usize) -> Option<std::ops::Range<f32>> {
        let start = *self.prefix.get(index)?;
        let end = *self.prefix.get(index.checked_add(1)?)?;
        Some(start..end)
    }
    pub fn estimate(&self) -> f32 {
        self.estimate
    }
    fn rebuild_prefix(&mut self, _from: usize) {
        self.prefix.clear();
        self.prefix.push(0.0);
        for extent in &self.extents {
            self.prefix
                .push(self.prefix.last().copied().unwrap_or(0.0) + *extent);
        }
    }
}

#[cfg(test)]
mod tests;

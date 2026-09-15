use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::core::{ColorRgba8, PointI, RectF, RectI, SizeF, SizeI};
use crate::render::{
    BatchKey, BlendMode, BoxInstance, ClipId, DrawItem, ImageAlphaMode, ImageColorEncoding,
    ImageId, ImageInstance, ImagePixelFormat, ImageResource, ImageResourceUpdate, PipelineKind,
    PrimitiveKind, RangePatch, RenderScene, RenderSceneDelta, RoundedClip, SpatialId,
};
use crate::scene::NodeId;

/// Both contours start at the window's inner top edge, not the app/title-bar seam.
/// The rectangular content scissor cuts off the title bar without introducing another pair of
/// top corners. Easy frames use only the border contour; custom templates can request a second
/// contour for an aperture inside wider frame margins.
pub(super) fn frame_content_clips(
    border: &BoxInstance,
    position: PointI,
    content: RectI,
    content_radius: f32,
) -> [Option<RoundedClip>; 2] {
    let rect = RectF {
        x: border.rect.x + position.x as f32,
        y: border.rect.y + position.y as f32,
        ..border.rect
    };
    let inner = RoundedClip::new(rect, border.corner_radii).inset(border.border);
    if content_radius <= 0.0 {
        return [Some(inner), None];
    }
    [
        Some(inner),
        Some(RoundedClip::new(
            RectF {
                x: content.x as f32,
                y: inner.rect.y,
                width: content.width as f32,
                height: (content.bottom() as f32 - inner.rect.y).max(0.0),
            },
            crate::ui::CornerRadii::all(content_radius),
        )),
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(all(test, not(target_os = "linux")), allow(dead_code))]
pub(super) enum ShellLayerKey {
    MotionShadow(u32),
    Motion(u32),
    Background,
    Frame(u32, u8),
    FrameShadow(u32),
    ContentBackground(u32),
    ContentBorder(u32),
    ContentCorners(u32),
    Surface(u32),
    ResizeVeil(u32),
    LegacyControl(u32, u8),
    LegacyControlSource(u8),
    Widget(u32),
    DragIcon(u32),
    Cursor,
    ComposedPointerSource,
    ComposedIconSource(usize),
}

/// Identifies retained scene content independently from a particular desktop placement.
///
/// A single icon scene, for example, can be placed in more than one window without duplicating
/// its backend resources. Placement keys remain unique so movement and stacking damage are still
/// tracked correctly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(all(test, not(target_os = "linux")), allow(dead_code))]
pub(super) enum ShellSceneKey {
    ResizeGlass(u32),
    MotionShadow(u32),
    Motion(u64),
    FrameShadow(u32),
    Background,
    Frame(u32),
    Surface(u32),
    ResizeVeil(u32),
    ContentBackground(u32),
    ContentBorder(u32),
    ContentCorners(u32),
    LegacyControl(u8),
    Widget(u32),
    DragIcon(u32),
    CursorImage,
    ComposedPointer,
    ComposedIcon(usize),
}

#[derive(Clone)]
pub(super) struct ShellImageRegion {
    pub rect: RectI,
    pub row_bytes: usize,
    pub pixels: Arc<[u8]>,
}

#[derive(Clone)]
pub(super) enum ShellImageUpdate {
    /// No publication is being delivered (for example, a hidden producer keeps queued pixels).
    Unchanged,
    /// A committed publication explicitly reuses the already delivered pixels.
    Reused,
    Full(Arc<[u8]>),
    Regions(Vec<ShellImageRegion>),
    /// The Vulkan backend has prepared a compositor-owned retained texture under this ID.
    External {
        image: ImageId,
        content_version: u64,
        /// Conservative damage in retained-image pixels; None means a full update.
        damage: Option<RectI>,
    },
}

pub(super) enum ShellLayerContent {
    Decoration {
        scene: ShellSceneKey,
        instance: BoxInstance,
    },
    Solid {
        scene: ShellSceneKey,
        color: ColorRgba8,
        corner_radius: f32,
    },
    Retained {
        scene: ShellSceneKey,
        deltas: Vec<RenderSceneDelta>,
    },
    Image {
        scene: ShellSceneKey,
        content_version: u64,
        update: ShellImageUpdate,
        alpha_mode: ImageAlphaMode,
        pixel_format: ImagePixelFormat,
    },
}

pub(super) struct ShellLayer {
    pub glass: Option<crate::GlassStyle>,
    pub key: ShellLayerKey,
    pub content: ShellLayerContent,
    /// Extent of the retained scene or committed client buffer.
    pub source_extent: SizeI,
    /// Compositor-controlled output rectangle. Generic placements support scaling, but Wayland
    /// client placements preserve `source_extent` and use clipping during interactive resize.
    pub target: RectI,
    /// A compositor-space clip used to constrain stale committed buffers during live resize.
    pub clip: Option<RectI>,
    pub rounded_clips: [Option<RoundedClip>; 2],
    /// Invisible placements retain their backend scene but contribute no output geometry.
    pub visible: bool,
}

impl ShellLayer {
    pub(super) fn frame_shadow(
        surface: u32,
        mut instance: BoxInstance,
        position: PointI,
    ) -> Option<Self> {
        if !instance
            .shadows
            .as_slice()
            .iter()
            .any(|shadow| shadow.color.a > 0)
        {
            return None;
        }
        let outline = RoundedClip::new(
            RectF {
                x: instance.rect.x + position.x as f32,
                y: instance.rect.y + position.y as f32,
                ..instance.rect
            },
            instance.corner_radii,
        );
        let mut bounds = instance.rect;
        for shadow in instance.shadows.as_slice() {
            let reach = (shadow.spread + shadow.blur * 2.0).max(0.0);
            bounds = bounds.union(RectF {
                x: instance.rect.x + shadow.offset.x - reach,
                y: instance.rect.y + shadow.offset.y - reach,
                width: instance.rect.width + 2.0 * reach,
                height: instance.rect.height + 2.0 * reach,
            });
        }
        let left = bounds.x.floor() as i32;
        let top = bounds.y.floor() as i32;
        let extent = SizeI {
            width: (bounds.x + bounds.width).ceil() as i32 - left,
            height: (bounds.y + bounds.height).ceil() as i32 - top,
        };
        instance.node = NodeId::new(0, 1);
        instance.rect.x -= left as f32;
        instance.rect.y -= top as f32;
        instance.view_bounds = RectF {
            x: 0.0,
            y: 0.0,
            width: extent.width as f32,
            height: extent.height as f32,
        };
        instance.background = None;
        instance.border = Default::default();
        instance.outline = Default::default();
        let mut layer = Self::retained(
            ShellLayerKey::FrameShadow(surface),
            ShellSceneKey::FrameShadow(surface),
            Vec::new(),
            extent,
            PointI {
                x: position.x + left,
                y: position.y + top,
            },
            true,
        );
        layer.content = ShellLayerContent::Decoration {
            scene: ShellSceneKey::FrameShadow(surface),
            instance,
        };
        // Keep transparent clients and resize previews free of their own exterior shadow.
        layer.rounded_clips = [Some(outline.inverse()), None];
        Some(layer)
    }

    /// Frame scenes are rectangular allocations; their paint must stay inside the chrome
    /// contour even when the scene contains a rectangular backing or a clipped shadow.
    pub(super) fn with_frame_outline(mut self, border: &BoxInstance, position: PointI) -> Self {
        self.rounded_clips = [
            Some(RoundedClip::new(
                RectF {
                    x: border.rect.x + position.x as f32,
                    y: border.rect.y + position.y as f32,
                    ..border.rect
                },
                border.corner_radii,
            )),
            None,
        ];
        self
    }

    /// Restore the border inside the content cutout. When the backing follows this same
    /// inner contour and opacity, include it in the box so fill and ring share coverage.
    /// Passing None leaves the aperture-specific backing to a separate layer.
    pub(super) fn content_border(
        surface: u32,
        mut instance: BoxInstance,
        extent: SizeI,
        position: PointI,
        content: RectI,
        background: Option<ColorRgba8>,
    ) -> Self {
        // The source UI node can be replaced on a model/state change. This single-box scene
        // has its own stable slot; never accumulate obsolete frame nodes behind draw index zero.
        instance.node = NodeId::new(0, 1);
        instance.background = background;
        instance.shadows = Default::default();
        let mut layer = Self::retained(
            ShellLayerKey::ContentBorder(surface),
            ShellSceneKey::ContentBorder(surface),
            Vec::new(),
            extent,
            position,
            true,
        );
        layer.content = ShellLayerContent::Decoration {
            scene: ShellSceneKey::ContentBorder(surface),
            instance,
        };
        layer.clip = Some(content);
        layer
    }

    /// Restore the frame fill outside the content aperture, never beneath transparent client
    /// pixels. The box retains its border widths to match the root's fill contour, but its border
    /// colors are transparent: the separate border patch paints that ring exactly once.
    pub(super) fn content_corners(
        surface: u32,
        mut instance: BoxInstance,
        extent: SizeI,
        position: PointI,
        content: RectI,
        clips: [Option<RoundedClip>; 2],
    ) -> Option<Self> {
        let aperture = clips[1]?;
        instance.shadows = Default::default();
        if instance.background.is_none_or(|color| color.a == 0) {
            return None;
        }
        instance.node = NodeId::new(0, 1);
        for side in [
            &mut instance.border.top,
            &mut instance.border.right,
            &mut instance.border.bottom,
            &mut instance.border.left,
        ] {
            side.color = ColorRgba8::rgba(0, 0, 0, 0);
        }
        let mut layer = Self::retained(
            ShellLayerKey::ContentCorners(surface),
            ShellSceneKey::ContentCorners(surface),
            Vec::new(),
            extent,
            position,
            true,
        );
        layer.content = ShellLayerContent::Decoration {
            scene: ShellSceneKey::ContentCorners(surface),
            instance,
        };
        layer.clip = Some(content);
        layer.rounded_clips = [Some(aperture.inverse()), None];
        Some(layer)
    }

    pub(super) fn with_content_clip(
        mut self,
        bounds: RectI,
        clips: [Option<RoundedClip>; 2],
    ) -> Self {
        self.clip = Some(self.clip.map_or(bounds, |clip| {
            intersect(clip, bounds).unwrap_or(RectI {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            })
        }));
        self.rounded_clips = clips;
        self
    }
    pub(super) fn solid(
        key: ShellLayerKey,
        scene: ShellSceneKey,
        color: ColorRgba8,
        target: RectI,
    ) -> Self {
        Self::rounded_solid(key, scene, color, target, 0.0)
    }

    pub(super) fn rounded_solid(
        key: ShellLayerKey,
        scene: ShellSceneKey,
        color: ColorRgba8,
        target: RectI,
        corner_radius: f32,
    ) -> Self {
        let corner_radius = if corner_radius.is_finite() {
            corner_radius.max(0.0)
        } else {
            0.0
        };
        Self {
            glass: None,
            key,
            content: ShellLayerContent::Solid {
                scene,
                color,
                corner_radius,
            },
            // Native geometry works with both desktop renderers; only this analytic primitive
            // changes size, never a client image or a pixel allocation.
            source_extent: SizeI {
                width: target.width,
                height: target.height,
            },
            target,
            clip: None,
            visible: true,
            rounded_clips: [None; 2],
        }
    }

    /// Places one retained frame scene around an external content slot. Non-overlapping scissors
    /// remove *all* composed backing (including root fill and shadow) without blending corners
    /// twice. Empty pieces remain invisible scene owners; deltas are delivered exactly once.
    pub(super) fn retained_frame(
        surface: u32,
        deltas: Vec<RenderSceneDelta>,
        extent: SizeI,
        position: PointI,
        visible: bool,
        cutout: Option<RectI>,
    ) -> Vec<Self> {
        let mut frame = Self::retained(
            ShellLayerKey::Frame(surface, 0),
            ShellSceneKey::Frame(surface),
            deltas,
            extent,
            position,
            visible,
        );
        let Some(hole) = cutout.and_then(|hole| intersect(frame.target, hole)) else {
            return vec![frame];
        };
        let outer = frame.target;
        let clips = [
            RectI {
                x: outer.x,
                y: outer.y,
                width: outer.width,
                height: hole.y - outer.y,
            },
            RectI {
                x: outer.x,
                y: hole.bottom(),
                width: outer.width,
                height: outer.bottom() - hole.bottom(),
            },
            RectI {
                x: outer.x,
                y: hole.y,
                width: hole.x - outer.x,
                height: hole.height,
            },
            RectI {
                x: hole.right(),
                y: hole.y,
                width: outer.right() - hole.right(),
                height: hole.height,
            },
        ];
        frame.clip = Some(clips[0]);
        let mut layers = vec![frame];
        for (index, clip) in clips.into_iter().enumerate().skip(1) {
            let mut part = Self::retained(
                ShellLayerKey::Frame(surface, index as u8),
                ShellSceneKey::Frame(surface),
                Vec::new(),
                extent,
                position,
                visible,
            );
            part.clip = Some(clip);
            layers.push(part);
        }
        layers
    }

    pub(super) fn retained(
        key: ShellLayerKey,
        scene: ShellSceneKey,
        deltas: Vec<RenderSceneDelta>,
        extent: SizeI,
        position: PointI,
        visible: bool,
    ) -> Self {
        Self {
            glass: None,
            key,
            content: ShellLayerContent::Retained { scene, deltas },
            source_extent: extent,
            target: RectI {
                x: position.x,
                y: position.y,
                width: extent.width,
                height: extent.height,
            },
            clip: None,
            visible,
            rounded_clips: [None; 2],
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn image(
        key: ShellLayerKey,
        scene: ShellSceneKey,
        content_version: u64,
        update: ShellImageUpdate,
        source_extent: SizeI,
        target: RectI,
        clip: Option<RectI>,
        alpha_mode: ImageAlphaMode,
        pixel_format: ImagePixelFormat,
        visible: bool,
    ) -> Self {
        Self {
            glass: None,
            key,
            content: ShellLayerContent::Image {
                scene,
                content_version,
                update,
                alpha_mode,
                pixel_format,
            },
            source_extent,
            target,
            clip,
            visible,
            rounded_clips: [None; 2],
        }
    }
}

#[derive(Clone, Debug)]
#[cfg_attr(all(test, not(target_os = "linux")), allow(dead_code))]
pub(super) struct ShellSceneUpdate {
    pub key: ShellSceneKey,
    pub deltas: Vec<RenderSceneDelta>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ShellPlacement {
    pub key: ShellLayerKey,
    pub scene: ShellSceneKey,
    pub target: RectI,
    pub clip: Option<RectI>,
    pub rounded_clips: [Option<RoundedClip>; 2],
}

#[derive(Clone, Debug)]
#[cfg_attr(all(test, not(target_os = "linux")), allow(dead_code))]
pub(super) struct ShellFrame {
    pub glass_changed: BTreeSet<ShellSceneKey>,
    pub glass: BTreeMap<ShellSceneKey, crate::GlassStyle>,
    pub motion: super::motion::MotionFrame,
    pub extent: SizeI,
    pub live_scenes: BTreeSet<ShellSceneKey>,
    pub updates: Vec<ShellSceneUpdate>,
    pub placements: Vec<ShellPlacement>,
    /// Client revisions actually included in this frame, carried to its KMS completion.
    pub surface_revisions: Vec<(u32, u64)>,
    /// `None` means the complete output; `Some` is a retained-output damage rectangle.
    pub damage: Option<RectI>,
}

impl ShellFrame {
    /// Cross the logical desktop -> physical scanout boundary exactly once. Scene content and
    /// publication revisions stay unchanged; both backends receive the same placement mapping.
    pub(super) fn into_physical(
        mut self,
        scale: crate::platform::ScaleFactor,
        extent: SizeI,
    ) -> Self {
        self.extent = extent;
        for style in self.glass.values_mut() {
            *style = style.normalized();
            for distance in [
                &mut style.blur_radius,
                &mut style.bevel_width,
                &mut style.blend_softness,
                &mut style.refraction,
                &mut style.dispersion,
            ] {
                *distance *= scale.get();
            }
        }
        self.damage = self.damage.and_then(|rect| {
            intersect(
                scale.physical_damage(rect),
                RectI {
                    x: 0,
                    y: 0,
                    width: extent.width,
                    height: extent.height,
                },
            )
        });
        for placement in &mut self.placements {
            placement.target = scale.physical_rect(placement.target);
            placement.clip = placement.clip.map(|rect| scale.physical_rect(rect));
            for clip in placement.rounded_clips.iter_mut().flatten() {
                let s = scale.get();
                clip.rect = RectF {
                    x: clip.rect.x * s,
                    y: clip.rect.y * s,
                    width: clip.rect.width * s,
                    height: clip.rect.height * s,
                };
                clip.radii.top_left *= s;
                clip.radii.top_right *= s;
                clip.radii.bottom_left *= s;
                clip.radii.bottom_right *= s;
            }
        }
        self
    }
}

struct ImageScene {
    source: RenderScene,
    source_version: u64,
    content_version: u64,
    image: ImageId,
    extent: SizeI,
    alpha_mode: ImageAlphaMode,
    pixel_format: ImagePixelFormat,
}

impl ImageScene {
    fn new() -> Self {
        let mut source = RenderScene::default();
        source.background = ColorRgba8 {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        };
        source.extent = SizeF {
            width: 1.0,
            height: 1.0,
        };
        Self {
            source,
            source_version: 0,
            content_version: 0,
            image: ImageId(1),
            extent: SizeI::default(),
            alpha_mode: ImageAlphaMode::Straight,
            pixel_format: ImagePixelFormat::Rgba8,
        }
    }

    fn synchronize(
        &mut self,
        source_version: u64,
        update: &ShellImageUpdate,
        extent: SizeI,
        alpha_mode: ImageAlphaMode,
        pixel_format: ImagePixelFormat,
    ) -> Option<RenderSceneDelta> {
        if matches!(update, ShellImageUpdate::Unchanged) {
            return None;
        }
        let metadata_changed = self.extent != extent
            || self.alpha_mode != alpha_mode
            || self.pixel_format != pixel_format;
        let source_changed = self.source_version != source_version;
        if self.content_version == 0 || metadata_changed {
            self.source.extent = size_f(extent);
            self.source.damage.full = true;
            self.source.damage.rects.clear();
            match update {
                ShellImageUpdate::Full(pixels) => {
                    self.content_version = self.content_version.wrapping_add(1).max(1);
                    self.image = ImageId(1);
                    self.source
                        .set_image_resource(ImageResource {
                            image: self.image,
                            content_version: self.content_version,
                            extent,
                            color_encoding: ImageColorEncoding::Srgb,
                            alpha_mode,
                            pixel_format,
                            pixels: Arc::clone(pixels),
                        })
                        .expect("validated desktop image resource");
                }
                ShellImageUpdate::External {
                    image,
                    content_version,
                    ..
                } => {
                    self.image = *image;
                    self.content_version = *content_version;
                }
                ShellImageUpdate::Unchanged
                | ShellImageUpdate::Reused
                | ShellImageUpdate::Regions(_) => return None,
            }
            self.extent = extent;
            self.alpha_mode = alpha_mode;
            self.pixel_format = pixel_format;
        } else if source_changed {
            match update {
                // A hidden client can publish a newer revision while its pixel update remains
                // queued in `ClientWindow`. Do not acknowledge that revision until the queued
                // pixels are actually handed to this retained image scene.
                ShellImageUpdate::Unchanged => return None,
                ShellImageUpdate::Reused => {
                    // A same-size, damage-free final configure commit still advances the
                    // displayed revision and its callbacks, without uploading any client pixels.
                    self.source.damage.full = true;
                }
                ShellImageUpdate::Full(pixels) => {
                    self.content_version = self.content_version.wrapping_add(1).max(1);
                    self.image = ImageId(1);
                    self.source
                        .set_image_resource(ImageResource {
                            image: self.image,
                            content_version: self.content_version,
                            extent,
                            color_encoding: ImageColorEncoding::Srgb,
                            alpha_mode,
                            pixel_format,
                            pixels: Arc::clone(pixels),
                        })
                        .expect("validated desktop image resource");
                }
                ShellImageUpdate::Regions(regions) => {
                    for region in regions {
                        self.content_version = self.content_version.wrapping_add(1).max(1);
                        self.source
                            .update_image_resource_region(ImageResourceUpdate {
                                image: self.image,
                                content_version: self.content_version,
                                extent,
                                rect: region.rect,
                                row_bytes: region.row_bytes,
                                color_encoding: ImageColorEncoding::Srgb,
                                alpha_mode,
                                pixel_format,
                                pixels: Arc::clone(&region.pixels),
                            })
                            .expect("validated desktop image region");
                    }
                }
                ShellImageUpdate::External {
                    image,
                    content_version,
                    damage,
                } => {
                    self.image = *image;
                    self.content_version = *content_version;
                    self.source.damage.full = damage.is_none();
                    self.source.damage.rects.clear();
                    if let Some(rect) = damage {
                        self.source.damage.rects.push(RectF {
                            x: rect.x as f32,
                            y: rect.y as f32,
                            width: rect.width as f32,
                            height: rect.height as f32,
                        });
                    }
                }
            }
        }
        self.source_version = source_version;
        if self.content_version == 0 {
            return None;
        }
        let bounds = RectF {
            x: 0.0,
            y: 0.0,
            width: extent.width as f32,
            height: extent.height as f32,
        };
        self.source.images.upsert(
            NodeId::new(1, 1),
            ImageInstance {
                node: NodeId::new(1, 1),
                image: self.image,
                tint: None,
                rect: bounds,
                view_bounds: bounds,
                content_version: self.content_version,
                opacity: 1.0,
                clip: ClipId(0),
                spatial: SpatialId(0),
            },
        );
        self.source.set_draw_order(vec![DrawItem {
            kind: PrimitiveKind::Image,
            index: 0,
            batch: BatchKey {
                pipeline: PipelineKind::Image,
                resource: self.image.0,
                clip: ClipId(0),
                blend: if alpha_mode == ImageAlphaMode::Opaque {
                    BlendMode::Opaque
                } else {
                    BlendMode::Alpha
                },
                target: 0,
            },
        }]);
        self.source.take_delta()
    }
}

#[derive(Clone, Copy, PartialEq)]
struct PlacementState {
    glass: Option<crate::GlassStyle>,
    scene: ShellSceneKey,
    bounds: Option<RectI>,
    target: RectI,
    clip: Option<RectI>,
    rounded_clips: [Option<RoundedClip>; 2],
}

#[derive(Default)]
struct RetainedSceneAdapter {
    draw_order: Arc<[DrawItem]>,
}

impl RetainedSceneAdapter {
    fn adapt(&mut self, mut delta: RenderSceneDelta) -> RenderSceneDelta {
        if let Some(order) = &delta.draw_order {
            self.draw_order = Arc::clone(order);
        }
        let background_index = delta.box_len;
        let bounds = RectF {
            x: 0.0,
            y: 0.0,
            width: delta.extent.width,
            height: delta.extent.height,
        };
        delta.boxes.push(RangePatch {
            start: background_index,
            values: Arc::from([BoxInstance {
                node: NodeId::new(u32::MAX - 1, 1),
                rect: bounds,
                view_bounds: bounds,
                background: Some(delta.background),
                border: Default::default(),
                outline: Default::default(),
                corner_radii: Default::default(),
                shadows: Default::default(),
                opacity: 1.0,
                clip: ClipId(0),
                spatial: SpatialId(0),
            }]),
        });
        delta.box_len = delta.box_len.saturating_add(1);
        let mut order = Vec::with_capacity(self.draw_order.len() + 1);
        if delta.background.a != 0 {
            order.push(DrawItem {
                kind: PrimitiveKind::Box,
                index: background_index as u32,
                batch: BatchKey {
                    pipeline: PipelineKind::AnalyticBox,
                    resource: 0,
                    clip: ClipId(0),
                    blend: if delta.background.a == u8::MAX {
                        BlendMode::Opaque
                    } else {
                        BlendMode::Alpha
                    },
                    target: 0,
                },
            });
        }
        order.extend(self.draw_order.iter().copied());
        delta.draw_order = Some(order.into());
        delta
    }
}

/// Builds an ordered desktop frame without choosing or invoking a concrete renderer.
pub(super) struct ShellComposition {
    extent: SizeI,
    image_scenes: BTreeMap<ShellSceneKey, ImageScene>,
    retained_scenes: BTreeMap<ShellSceneKey, RetainedSceneAdapter>,
    solid_scenes: BTreeMap<ShellSceneKey, RenderScene>,
    placements: BTreeMap<ShellLayerKey, PlacementState>,
    order: Vec<ShellLayerKey>,
}

impl ShellComposition {
    pub(super) fn new(extent: SizeI) -> Self {
        Self {
            extent,
            image_scenes: BTreeMap::new(),
            retained_scenes: BTreeMap::new(),
            solid_scenes: BTreeMap::new(),
            placements: BTreeMap::new(),
            order: Vec::new(),
        }
    }

    #[cfg(test)]
    pub(super) fn synchronize(
        &mut self,
        extent: SizeI,
        layers: Vec<ShellLayer>,
    ) -> Option<ShellFrame> {
        self.synchronize_with_force(extent, layers, false)
    }

    pub(super) fn synchronize_with_force(
        &mut self,
        extent: SizeI,
        layers: Vec<ShellLayer>,
        force: bool,
    ) -> Option<ShellFrame> {
        let output = full_rect(extent);
        let extent_changed = self.extent != extent;
        self.extent = extent;
        let mut glass = BTreeMap::new();
        let mut live_scenes = BTreeSet::new();
        let mut updates = BTreeMap::<ShellSceneKey, Vec<RenderSceneDelta>>::new();
        let mut placements = Vec::new();
        let mut next_states = BTreeMap::new();
        let mut next_order = Vec::new();
        let mut damage = None;

        for mut layer in layers.into_iter().filter(valid_layer) {
            let scene = match layer.content {
                ShellLayerContent::Decoration { scene, instance } => {
                    let source = self.solid_scenes.entry(scene).or_default();
                    let extent = size_f(layer.source_extent);
                    let transparent = ColorRgba8::rgba(0, 0, 0, 0);
                    if source.extent != extent || source.background != transparent {
                        source.extent = extent;
                        source.background = transparent;
                        source.damage.full = true;
                    }
                    if source.boxes.upsert(instance.node, instance) {
                        source.damage.full = true;
                    }
                    source.set_draw_order(vec![DrawItem {
                        kind: PrimitiveKind::Box,
                        index: 0,
                        batch: BatchKey {
                            pipeline: PipelineKind::AnalyticBox,
                            resource: 0,
                            clip: ClipId(0),
                            blend: BlendMode::Alpha,
                            target: 0,
                        },
                    }]);
                    if let Some(delta) = source.take_delta() {
                        updates
                            .entry(scene)
                            .or_default()
                            .push(self.retained_scenes.entry(scene).or_default().adapt(delta));
                    }
                    scene
                }
                ShellLayerContent::Solid {
                    scene,
                    color,
                    corner_radius,
                } => {
                    let source = self.solid_scenes.entry(scene).or_default();
                    let rounded = corner_radius > 0.0;
                    let background = if rounded {
                        ColorRgba8::rgba(0, 0, 0, 0)
                    } else {
                        color
                    };
                    let extent = size_f(layer.source_extent);
                    if source.background != background || source.extent != extent {
                        source.extent = extent;
                        source.background = background;
                        source.damage.full = true;
                    }
                    let node = NodeId::new(0, 1);
                    if rounded {
                        let rect = RectF {
                            x: 0.0,
                            y: 0.0,
                            width: extent.width,
                            height: extent.height,
                        };
                        if source.boxes.upsert(
                            node,
                            BoxInstance {
                                node,
                                rect,
                                view_bounds: rect,
                                background: Some(color),
                                border: Default::default(),
                                outline: Default::default(),
                                corner_radii: crate::ui::CornerRadii::all(corner_radius),
                                shadows: Default::default(),
                                opacity: 1.0,
                                clip: ClipId(0),
                                spatial: SpatialId(0),
                            },
                        ) {
                            source.damage.full = true;
                        }
                        source.set_draw_order(vec![DrawItem {
                            kind: PrimitiveKind::Box,
                            index: 0,
                            batch: BatchKey {
                                pipeline: PipelineKind::AnalyticBox,
                                resource: 0,
                                clip: ClipId(0),
                                blend: BlendMode::Alpha,
                                target: 0,
                            },
                        }]);
                    } else if source.boxes.remove(node).is_some() {
                        source.set_draw_order(Vec::new());
                        source.damage.full = true;
                    }
                    if let Some(delta) = source.take_delta() {
                        updates
                            .entry(scene)
                            .or_default()
                            .push(self.retained_scenes.entry(scene).or_default().adapt(delta));
                    }
                    scene
                }
                ShellLayerContent::Retained { scene, deltas } => {
                    if !deltas.is_empty() {
                        let adapter = self.retained_scenes.entry(scene).or_default();
                        updates
                            .entry(scene)
                            .or_default()
                            .extend(deltas.into_iter().map(|delta| adapter.adapt(delta)));
                    }
                    scene
                }
                ShellLayerContent::Image {
                    scene,
                    content_version,
                    update,
                    alpha_mode,
                    pixel_format,
                } => {
                    let source = self
                        .image_scenes
                        .entry(scene)
                        .or_insert_with(ImageScene::new);
                    // An unready external generation keeps the previous image geometry/revision.
                    // New surfaces without a retained image contribute no placement yet.
                    if matches!(update, ShellImageUpdate::Unchanged) {
                        layer.source_extent = source.extent;
                        layer.visible &= source.content_version != 0;
                    }
                    if let Some(delta) = source.synchronize(
                        content_version,
                        &update,
                        layer.source_extent,
                        alpha_mode,
                        pixel_format,
                    ) {
                        updates.entry(scene).or_default().push(delta);
                    }
                    scene
                }
            };
            live_scenes.insert(scene);
            let target = layer.target;
            let bounds = layer
                .visible
                .then_some(target)
                .and_then(|target| {
                    layer
                        .clip
                        .map_or(Some(target), |clip| intersect(target, clip))
                })
                .and_then(|target| intersect(target, output));
            let state = PlacementState {
                glass: layer.glass,
                scene,
                bounds,
                target,
                clip: layer.clip,
                rounded_clips: layer.rounded_clips,
            };
            if self.placements.get(&layer.key).is_none_or(|previous| {
                previous.glass != state.glass
                    || previous.scene != state.scene
                    || previous.bounds != state.bounds
                    || previous.target != state.target
                    || previous.clip != state.clip
                    || previous.rounded_clips != state.rounded_clips
            }) {
                if let Some(previous) = self
                    .placements
                    .get(&layer.key)
                    .and_then(|state| state.bounds)
                {
                    add_damage(&mut damage, previous, output);
                }
                if let Some(bounds) = bounds {
                    add_damage(&mut damage, bounds, output);
                }
            }
            if let Some(style) = layer.glass {
                glass.insert(scene, style);
            }
            next_states.insert(layer.key, state);
            if bounds.is_some() {
                next_order.push(layer.key);
                placements.push(ShellPlacement {
                    key: layer.key,
                    scene,
                    target,
                    clip: layer.clip,
                    rounded_clips: layer.rounded_clips,
                });
            }
        }

        for (key, previous) in &self.placements {
            if !next_states.contains_key(key)
                && let Some(bounds) = previous.bounds
            {
                add_damage(&mut damage, bounds, output);
            }
        }

        let order_changed = self.order != next_order;
        if order_changed || extent_changed {
            // Reordering can change every overlap in the stack. It is infrequent and correctness
            // is clearer than attempting a fragile pairwise overlap reconstruction here.
            damage = Some(output);
        }

        for (scene, deltas) in &updates {
            for delta in deltas {
                for placement in placements
                    .iter()
                    .filter(|placement| placement.scene == *scene)
                {
                    if delta.damage.full {
                        if let Some(bounds) = placement_bounds(*placement, output) {
                            add_damage(&mut damage, bounds, output);
                        }
                    } else {
                        for rect in &delta.damage.rects {
                            if let Some(mapped) = map_scene_rect(*rect, delta.extent, *placement)
                                .and_then(|rect| intersect(rect, output))
                            {
                                add_damage(&mut damage, mapped, output);
                            }
                        }
                    }
                }
            }
        }

        let mut glass_changed = self
            .placements
            .values()
            .filter(|old| old.glass.is_some() && !glass.contains_key(&old.scene))
            .map(|old| old.scene)
            .collect::<BTreeSet<_>>();
        for (index, placement) in placements.iter().enumerate() {
            if !glass.contains_key(&placement.scene) {
                continue;
            }
            let own_changed =
                next_states.get(&placement.key) != self.placements.get(&placement.key);
            let lower_changed = placements[..index].iter().any(|lower| {
                updates.contains_key(&lower.scene)
                    || next_states.get(&lower.key) != self.placements.get(&lower.key)
            });
            if order_changed || extent_changed || own_changed || lower_changed {
                glass_changed.insert(placement.scene);
                if let Some(bounds) = placement_bounds(*placement, output) {
                    add_damage(&mut damage, bounds, output);
                }
            }
        }

        self.image_scenes.retain(|key, _| live_scenes.contains(key));
        self.retained_scenes
            .retain(|key, _| live_scenes.contains(key));
        self.solid_scenes.retain(|key, _| live_scenes.contains(key));
        self.placements = next_states;
        self.order = next_order;

        let has_updates = !updates.is_empty();
        if !force && !has_updates && damage.is_none() {
            return None;
        }
        // Resource-only changes should not get stranded without a backend turn. If a producer did
        // not attach explicit damage, repaint every visible placement that consumes that scene.
        if damage.is_none() && has_updates {
            for placement in &placements {
                if updates.contains_key(&placement.scene)
                    && let Some(bounds) = placement_bounds(*placement, output)
                {
                    add_damage(&mut damage, bounds, output);
                }
            }
        }
        let damage = damage.map(|damage| intersect(damage, output).unwrap_or(output));
        let surface_revisions = placements
            .iter()
            .filter_map(|placement| {
                let surface = match placement.key {
                    ShellLayerKey::Surface(surface) | ShellLayerKey::DragIcon(surface) => {
                        surface
                    }
                    _ => return None,
                };
                self.image_scenes
                    .get(&placement.scene)
                    .filter(|scene| scene.source_version != 0)
                    .map(|scene| {
                        super::resize_trace::event(
                            surface,
                            "scene",
                            format_args!(
                                "revision={} image={:?} content_version={} extent={:?} target={:?}",
                                scene.source_version,
                                scene.image,
                                scene.content_version,
                                scene.extent,
                                placement.target
                            ),
                        );
                        (surface, scene.source_version)
                    })
            })
            .collect();
        Some(ShellFrame {
            glass_changed,
            glass,
            motion: Default::default(),
            extent,
            live_scenes,
            updates: updates
                .into_iter()
                .map(|(key, deltas)| ShellSceneUpdate { key, deltas })
                .collect(),
            placements,
            surface_revisions,
            damage: if damage == Some(output) { None } else { damage },
        })
    }
}

fn valid_layer(layer: &ShellLayer) -> bool {
    if layer.source_extent.width <= 0
        || layer.source_extent.height <= 0
        || layer.target.width <= 0
        || layer.target.height <= 0
    {
        return false;
    }
    match &layer.content {
        ShellLayerContent::Retained { .. }
        | ShellLayerContent::Solid { .. }
        | ShellLayerContent::Decoration { .. } => true,
        ShellLayerContent::Image { update, .. } => match update {
            ShellImageUpdate::Unchanged | ShellImageUpdate::Reused => true,
            ShellImageUpdate::Full(pixels) => {
                pixels.len()
                    >= layer.source_extent.width as usize * layer.source_extent.height as usize * 4
            }
            ShellImageUpdate::Regions(regions) => {
                !regions.is_empty()
                    && regions.iter().all(|region| {
                        let rect = region.rect;
                        rect.x >= 0
                            && rect.y >= 0
                            && rect.width > 0
                            && rect.height > 0
                            && rect.right() <= layer.source_extent.width
                            && rect.bottom() <= layer.source_extent.height
                            && region.row_bytes >= rect.width as usize * 4
                            && region.pixels.len()
                                >= region.row_bytes.saturating_mul(rect.height as usize)
                    })
            }
            ShellImageUpdate::External {
                image,
                content_version,
                ..
            } => image.0 != 0 && *content_version != 0,
        },
    }
}

fn map_scene_rect(rect: RectF, source_extent: SizeF, placement: ShellPlacement) -> Option<RectI> {
    let width = source_extent.width.max(1.0);
    let height = source_extent.height.max(1.0);
    let scale_x = placement.target.width as f32 / width;
    let scale_y = placement.target.height as f32 / height;
    let left = (placement.target.x as f32 + rect.x * scale_x).floor() as i32;
    let top = (placement.target.y as f32 + rect.y * scale_y).floor() as i32;
    let right = (placement.target.x as f32 + rect.right() * scale_x).ceil() as i32;
    let bottom = (placement.target.y as f32 + rect.bottom() * scale_y).ceil() as i32;
    let mapped = RectI {
        x: left,
        y: top,
        width: right.saturating_sub(left),
        height: bottom.saturating_sub(top),
    };
    placement
        .clip
        .map_or(Some(mapped), |clip| intersect(mapped, clip))
}

fn placement_bounds(placement: ShellPlacement, output: RectI) -> Option<RectI> {
    placement
        .clip
        .map_or(Some(placement.target), |clip| {
            intersect(placement.target, clip)
        })
        .and_then(|rect| intersect(rect, output))
}

fn add_damage(damage: &mut Option<RectI>, rect: RectI, output: RectI) {
    let Some(rect) = intersect(rect, output) else {
        return;
    };
    *damage = Some(damage.map_or(rect, |current| union(current, rect)));
}

fn size_f(size: SizeI) -> SizeF {
    SizeF {
        width: size.width as f32,
        height: size.height as f32,
    }
}

fn full_rect(size: SizeI) -> RectI {
    RectI {
        x: 0,
        y: 0,
        width: size.width,
        height: size.height,
    }
}

fn union(left: RectI, right: RectI) -> RectI {
    let x = left.x.min(right.x);
    let y = left.y.min(right.y);
    let right_edge = left.right().max(right.right());
    let bottom = left.bottom().max(right.bottom());
    RectI {
        x,
        y,
        width: right_edge.saturating_sub(x),
        height: bottom.saturating_sub(y),
    }
}

fn intersect(left: RectI, right: RectI) -> Option<RectI> {
    let x = left.x.max(right.x);
    let y = left.y.max(right.y);
    let right_edge = left.right().min(right.right());
    let bottom = left.bottom().min(right.bottom());
    (right_edge > x && bottom > y).then_some(RectI {
        x,
        y,
        width: right_edge - x,
        height: bottom - y,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glass_is_idle_until_lower_content_or_its_appearance_changes() {
        let extent = SizeI {
            width: 160,
            height: 120,
        };
        let target = RectI {
            x: 30,
            y: 20,
            width: 80,
            height: 70,
        };
        let layers = |background, foreground, glass: bool| {
            let mut preview = veil(target, ColorRgba8::rgba(23, 27, 37, 150));
            preview.glass = glass.then_some(crate::GlassStyle::default());
            vec![
                ShellLayer::solid(
                    ShellLayerKey::Background,
                    ShellSceneKey::Background,
                    background,
                    full_rect(extent),
                ),
                preview,
                ShellLayer::solid(
                    ShellLayerKey::Cursor,
                    ShellSceneKey::CursorImage,
                    foreground,
                    RectI {
                        x: 140,
                        y: 100,
                        width: 5,
                        height: 5,
                    },
                ),
            ]
        };
        let black = ColorRgba8::rgba(0, 0, 0, 255);
        let white = ColorRgba8::rgba(255, 255, 255, 255);
        let mut composition = ShellComposition::new(extent);
        let first = composition
            .synchronize(extent, layers(black, black, true))
            .unwrap();
        assert!(
            first
                .glass_changed
                .contains(&ShellSceneKey::ResizeVeil(9))
        );
        assert!(
            composition
                .synchronize(extent, layers(black, black, true))
                .is_none()
        );
        let cursor = composition
            .synchronize(extent, layers(black, white, true))
            .unwrap();
        assert!(
            cursor.glass_changed.is_empty(),
            "foreground changes must not rebuild glass snapshots"
        );
        assert_eq!(
            cursor.damage,
            Some(RectI {
                x: 140,
                y: 100,
                width: 5,
                height: 5
            })
        );
        let lower = composition
            .synchronize(extent, layers(white, white, true))
            .unwrap();
        assert!(
            lower
                .glass_changed
                .contains(&ShellSceneKey::ResizeVeil(9))
        );
        let flat = composition
            .synchronize(extent, layers(white, white, false))
            .unwrap();
        assert!(flat.glass.is_empty());
        assert_eq!(
            flat.damage,
            Some(target),
            "switching to the identical flat tint must still repaint"
        );
    }

    #[test]
    fn glass_distances_cross_output_scale_once() {
        let extent = SizeI {
            width: 100,
            height: 100,
        };
        let mut preview = veil(full_rect(extent), ColorRgba8::rgba(0, 0, 0, 0));
        preview.glass = Some(crate::GlassStyle { blend_softness: 16.0, ..crate::GlassStyle::default() });
        let mut composition = ShellComposition::new(extent);
        let frame = composition
            .synchronize(extent, vec![preview])
            .unwrap()
            .into_physical(
                crate::platform::ScaleFactor::new(2.0).unwrap(),
                SizeI {
                    width: 200,
                    height: 200,
                },
            );
        let physical = frame.glass[&ShellSceneKey::ResizeVeil(9)];
        let logical = crate::GlassStyle::liquid();
        assert_eq!(physical.blur_radius, logical.blur_radius * 2.0);
        assert_eq!(physical.bevel_width, logical.bevel_width * 2.0);
        assert_eq!(physical.blend_softness, 32.0);
        assert_eq!(physical.refraction, logical.refraction * 2.0);
        assert_eq!(physical.dispersion, logical.dispersion * 2.0);

        assert_eq!(physical.fresnel, logical.fresnel);

        assert_eq!(physical.tint, logical.tint);
    }

    #[test]
    fn output_mapping_scales_geometry_clips_and_damage_once_and_preserves_revisions() {
        let logical = RectI {
            x: 1,
            y: 3,
            width: 100,
            height: 40,
        };
        let mut rounded = RoundedClip::new(
            RectF {
                x: 1.0,
                y: 3.0,
                width: 100.0,
                height: 40.0,
            },
            crate::ui::CornerRadii::all(4.0),
        );
        rounded.inverted = true;
        let frame = ShellFrame {
            glass_changed: BTreeSet::new(),
            glass: BTreeMap::new(),
            motion: Default::default(),
            extent: SizeI {
                width: 1280,
                height: 720,
            },
            live_scenes: BTreeSet::new(),
            updates: Vec::new(),
            placements: vec![ShellPlacement {
                key: ShellLayerKey::Surface(1),
                scene: ShellSceneKey::Surface(1),
                target: logical,
                clip: Some(logical),
                rounded_clips: [Some(rounded), None],
            }],
            surface_revisions: vec![(1, 9)],
            damage: Some(logical),
        };
        for factor in [1.0, 1.5, 2.0] {
            let scale = crate::platform::ScaleFactor::new(factor).unwrap();
            let pixels = SizeI {
                width: (1280.0 * factor) as i32,
                height: (720.0 * factor) as i32,
            };
            let physical = frame.clone().into_physical(scale, pixels);
            assert_eq!(physical.extent, pixels);
            assert_eq!(physical.placements[0].target, scale.physical_rect(logical));
            assert_eq!(
                physical.placements[0].clip,
                Some(scale.physical_rect(logical))
            );
            assert_eq!(physical.damage, Some(scale.physical_damage(logical)));
            let clip = physical.placements[0].rounded_clips[0].unwrap();
            assert_eq!(clip.radii.top_left, 4.0 * factor);
            assert_eq!(clip.rect.y, 3.0 * factor);
            assert!(clip.inverted);
            assert_eq!(physical.surface_revisions, [(1, 9)]);
        }
    }

    #[test]
    fn frame_cutouts_cover_only_the_complement_even_at_output_edges() {
        let extent = SizeI {
            width: 20,
            height: 16,
        };
        let position = PointI { x: -2, y: -1 };
        let outer = RectI {
            x: -2,
            y: -1,
            width: 20,
            height: 16,
        };
        for hole in [
            outer,
            RectI {
                x: 4,
                y: 4,
                width: 8,
                height: 6,
            },
            RectI {
                x: -10,
                y: -10,
                width: 15,
                height: 15,
            },
            RectI {
                x: 40,
                y: 40,
                width: 4,
                height: 4,
            },
        ] {
            let layers =
                ShellLayer::retained_frame(9, Vec::new(), extent, position, true, Some(hole));
            for y in -3..19 {
                for x in -4..22 {
                    let contains =
                        |r: RectI| x >= r.x && y >= r.y && x < r.right() && y < r.bottom();
                    let count = layers
                        .iter()
                        .filter(|layer| contains(layer.target) && layer.clip.is_none_or(contains))
                        .count();
                    assert_eq!(count, usize::from(contains(outer) && !contains(hole)));
                }
            }
        }
        let mut composition = ShellComposition::new(extent);
        let hidden = composition
            .synchronize(
                SizeI {
                    width: 24,
                    height: 20,
                },
                ShellLayer::retained_frame(9, Vec::new(), extent, position, true, Some(outer)),
            )
            .unwrap();
        assert!(hidden.placements.is_empty());
        assert!(hidden.live_scenes.contains(&ShellSceneKey::Frame(9)));
    }

    #[test]
    fn per_window_preview_colors_do_not_alias_a_shared_scene() {
        let extent = SizeI {
            width: 100,
            height: 80,
        };
        let mut composition = ShellComposition::new(extent);
        let colors = [
            ColorRgba8::rgba(200, 20, 30, 128),
            ColorRgba8::rgba(10, 150, 250, 255),
        ];
        let layers = colors
            .into_iter()
            .enumerate()
            .map(|(i, color)| {
                ShellLayer::solid(
                    ShellLayerKey::ResizeVeil(i as u32),
                    ShellSceneKey::ResizeVeil(i as u32),
                    color,
                    RectI {
                        x: i as i32 * 40,
                        y: 0,
                        width: 40,
                        height: 40,
                    },
                )
            })
            .collect();
        let frame = composition.synchronize(extent, layers).unwrap();
        assert_eq!(frame.updates.len(), 2);
        for (update, color) in frame.updates.iter().zip(colors) {
            assert_eq!(update.deltas[0].boxes[0].values[0].background, Some(color));
        }
    }

    fn veil(target: RectI, color: ColorRgba8) -> ShellLayer {
        ShellLayer::solid(
            ShellLayerKey::ResizeVeil(9),
            ShellSceneKey::ResizeVeil(9),
            color,
            target,
        )
    }

    #[test]
    fn resize_veil_moves_without_client_uploads_and_reveals_only_the_final_image() {
        let extent = SizeI {
            width: 800,
            height: 600,
        };
        let position = PointI { x: 100, y: 90 };
        let color = ColorRgba8 {
            r: 38,
            g: 42,
            b: 48,
            a: 255,
        };
        let mut composition = ShellComposition::new(extent);
        composition
            .synchronize(
                extent,
                vec![image_layer(
                    position,
                    ShellImageUpdate::Full(vec![255; 100 * 80 * 4].into()),
                )],
            )
            .unwrap();
        let preview = |width, height| RectI {
            x: position.x,
            y: position.y,
            width,
            height,
        };
        let hidden = || {
            let mut layer = image_layer(position, ShellImageUpdate::Unchanged);
            layer.visible = false;
            layer
        };
        let first = composition
            .synchronize(extent, vec![veil(preview(100, 80), color), hidden()])
            .unwrap();
        assert_eq!(first.updates.len(), 1);
        assert_eq!(first.updates[0].key, ShellSceneKey::ResizeVeil(9));
        let delta = &first.updates[0].deltas[0];
        assert!(delta.image_resources.is_empty());
        assert_eq!(delta.boxes[0].values[0].background, Some(color));
        assert!(first.live_scenes.contains(&ShellSceneKey::Surface(9)));
        assert_eq!(first.placements.len(), 1);
        assert!(first.surface_revisions.is_empty());

        let moved = composition
            .synchronize(extent, vec![veil(preview(150, 110), color), hidden()])
            .unwrap();
        assert_eq!(moved.updates.len(), 1);
        assert_eq!(moved.updates[0].key, ShellSceneKey::ResizeVeil(9));
        assert!(moved.updates[0].deltas[0].image_resources.is_empty());
        assert_eq!(
            moved.updates[0].deltas[0].extent,
            SizeF {
                width: 150.0,
                height: 110.0
            }
        );
        assert_eq!(moved.placements[0].target, preview(150, 110));
        assert!(
            composition
                .synchronize(extent, vec![veil(preview(150, 110), color), hidden()])
                .is_none()
        );

        let final_extent = SizeI {
            width: 144,
            height: 104,
        }; // cell-snapped legal response
        let final_layer = ShellLayer::image(
            ShellLayerKey::Surface(9),
            ShellSceneKey::Surface(9),
            2,
            ShellImageUpdate::Full(vec![128; 144 * 104 * 4].into()),
            final_extent,
            preview(144, 104),
            None,
            ImageAlphaMode::Opaque,
            ImagePixelFormat::Rgba8,
            true,
        );
        let final_frame = composition.synchronize(extent, vec![final_layer]).unwrap();
        assert_eq!(final_frame.placements.len(), 1);
        assert_eq!(final_frame.surface_revisions, vec![(9, 2)]);
        assert_eq!(final_frame.placements[0].scene, ShellSceneKey::Surface(9));
        assert_eq!(final_frame.updates.len(), 1);
        assert_eq!(final_frame.updates[0].deltas[0].image_resources.len(), 1);
        assert!(
            !final_frame
                .live_scenes
                .contains(&ShellSceneKey::ResizeVeil(9))
        );
    }

    #[test]
    fn solid_color_changes_keep_scene_epochs_monotonic() {
        let extent = SizeI {
            width: 200,
            height: 150,
        };
        let target = RectI {
            x: 0,
            y: 0,
            width: 100,
            height: 80,
        };
        let mut composition = ShellComposition::new(extent);
        let first = composition
            .synchronize(
                extent,
                vec![veil(
                    target,
                    ColorRgba8 {
                        r: 30,
                        g: 40,
                        b: 50,
                        a: 255,
                    },
                )],
            )
            .unwrap();
        let second = composition
            .synchronize(
                extent,
                vec![veil(
                    target,
                    ColorRgba8 {
                        r: 60,
                        g: 70,
                        b: 80,
                        a: 255,
                    },
                )],
            )
            .unwrap();
        assert!(second.updates[0].deltas[0].epoch > first.updates[0].deltas[0].epoch);
        assert!(second.updates[0].deltas[0].image_resources.is_empty());
    }

    #[test]
    fn damage_free_publication_advances_displayed_revision_without_uploading_pixels() {
        let extent = SizeI {
            width: 800,
            height: 600,
        };
        let position = PointI { x: 20, y: 30 };
        let mut composition = ShellComposition::new(extent);
        let first = composition
            .synchronize(
                extent,
                vec![image_layer(
                    position,
                    ShellImageUpdate::Full(vec![255; 100 * 80 * 4].into()),
                )],
            )
            .unwrap();
        let mut reused = image_layer(position, ShellImageUpdate::Reused);
        if let ShellLayerContent::Image {
            content_version, ..
        } = &mut reused.content
        {
            *content_version = 2;
        }
        let second = composition.synchronize(extent, vec![reused]).unwrap();
        assert_eq!(
            first.surface_revisions,
            vec![(9, 1)],
            "queued frames must retain their own revision"
        );
        assert_eq!(second.surface_revisions, vec![(9, 2)]);
        assert!(
            second
                .updates
                .iter()
                .flat_map(|update| &update.deltas)
                .all(|delta| delta.image_resources.is_empty())
        );
    }

    fn image_layer(position: PointI, update: ShellImageUpdate) -> ShellLayer {
        image_layer_at(
            RectI {
                x: position.x,
                y: position.y,
                width: 100,
                height: 80,
            },
            update,
        )
    }

    fn image_layer_at(target: RectI, update: ShellImageUpdate) -> ShellLayer {
        ShellLayer::image(
            ShellLayerKey::Surface(9),
            ShellSceneKey::Surface(9),
            1,
            update,
            SizeI {
                width: 100,
                height: 80,
            },
            target,
            None,
            ImageAlphaMode::Opaque,
            ImagePixelFormat::Rgba8,
            true,
        )
    }

    #[test]
    fn explicit_placement_scaling_does_not_require_a_content_update() {
        let extent = SizeI {
            width: 800,
            height: 600,
        };
        let mut composition = ShellComposition::new(extent);
        let _ = composition.synchronize(
            extent,
            vec![image_layer(
                PointI { x: 300, y: 220 },
                ShellImageUpdate::Full(vec![255; 100 * 80 * 4].into()),
            )],
        );

        let target = RectI {
            x: 260,
            y: 190,
            width: 140,
            height: 110,
        };
        let frame = composition
            .synchronize(
                extent,
                vec![image_layer_at(target, ShellImageUpdate::Unchanged)],
            )
            .unwrap();

        assert!(frame.updates.is_empty());
        assert_eq!(frame.placements[0].target, target);
        assert_eq!(
            frame.damage,
            Some(RectI {
                x: 260,
                y: 190,
                width: 140,
                height: 110,
            })
        );
    }

    #[test]
    fn movement_damages_old_and_new_bounds_without_rebuilding_content() {
        let extent = SizeI {
            width: 800,
            height: 600,
        };
        let mut composition = ShellComposition::new(extent);
        let _ = composition.synchronize(
            extent,
            vec![image_layer(
                PointI { x: 10, y: 20 },
                ShellImageUpdate::Full(vec![255; 100 * 80 * 4].into()),
            )],
        );
        let frame = composition
            .synchronize(
                extent,
                vec![image_layer(
                    PointI { x: 40, y: 20 },
                    ShellImageUpdate::Unchanged,
                )],
            )
            .unwrap();
        assert_eq!(
            frame.damage,
            Some(RectI {
                x: 10,
                y: 20,
                width: 130,
                height: 80,
            })
        );
        assert!(frame.updates.is_empty());
    }

    #[test]
    fn disjoint_image_updates_remain_disjoint_in_the_scene_delta() {
        let extent = SizeI {
            width: 800,
            height: 600,
        };
        let position = PointI { x: 20, y: 30 };
        let mut composition = ShellComposition::new(extent);
        let _ = composition.synchronize(
            extent,
            vec![image_layer(
                position,
                ShellImageUpdate::Full(vec![255; 100 * 80 * 4].into()),
            )],
        );
        let rects = [
            RectI {
                x: 4,
                y: 5,
                width: 8,
                height: 6,
            },
            RectI {
                x: 82,
                y: 67,
                width: 7,
                height: 5,
            },
        ];
        let mut layer = image_layer(
            position,
            ShellImageUpdate::Regions(
                rects
                    .into_iter()
                    .map(|rect| ShellImageRegion {
                        rect,
                        row_bytes: rect.width as usize * 4,
                        pixels: vec![128; rect.width as usize * rect.height as usize * 4].into(),
                    })
                    .collect(),
            ),
        );
        if let ShellLayerContent::Image {
            content_version, ..
        } = &mut layer.content
        {
            *content_version = 2;
        }
        let frame = composition.synchronize(extent, vec![layer]).unwrap();
        let delta = &frame.updates[0].deltas[0];
        let writes = delta
            .image_resources
            .iter()
            .filter_map(|update| match update {
                crate::render::ImageResourceDelta::Write(update) => Some(update.rect),
                crate::render::ImageResourceDelta::Remove(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(writes, rects);
    }

    #[test]
    fn one_retained_scene_can_be_placed_more_than_once() {
        let extent = SizeI {
            width: 800,
            height: 600,
        };
        let mut composition = ShellComposition::new(extent);
        let layers = [10, 50]
            .into_iter()
            .enumerate()
            .map(|(index, x)| {
                ShellLayer::retained(
                    ShellLayerKey::LegacyControl(index as u32, 0),
                    ShellSceneKey::LegacyControl(0),
                    Vec::new(),
                    SizeI {
                        width: 24,
                        height: 24,
                    },
                    PointI { x, y: 10 },
                    true,
                )
            })
            .collect();
        let frame = composition.synchronize(extent, layers).unwrap();
        assert_eq!(frame.live_scenes.len(), 1);
        assert_eq!(frame.placements.len(), 2);
        assert_eq!(frame.placements[0].scene, frame.placements[1].scene);
    }

    #[test]
    fn hidden_image_revision_is_not_consumed_before_its_pixels_arrive() {
        let extent = SizeI {
            width: 800,
            height: 600,
        };
        let position = PointI { x: 20, y: 30 };
        let mut composition = ShellComposition::new(extent);
        let _ = composition.synchronize(
            extent,
            vec![image_layer(
                position,
                ShellImageUpdate::Full(vec![255; 100 * 80 * 4].into()),
            )],
        );

        let mut hidden = image_layer(position, ShellImageUpdate::Unchanged);
        hidden.visible = false;
        if let ShellLayerContent::Image {
            content_version, ..
        } = &mut hidden.content
        {
            *content_version = 2;
        }
        let _ = composition.synchronize(extent, vec![hidden]);

        let rect = RectI {
            x: 4,
            y: 5,
            width: 8,
            height: 6,
        };
        let mut visible = image_layer(
            position,
            ShellImageUpdate::Regions(vec![ShellImageRegion {
                rect,
                row_bytes: rect.width as usize * 4,
                pixels: vec![128; rect.width as usize * rect.height as usize * 4].into(),
            }]),
        );
        if let ShellLayerContent::Image {
            content_version, ..
        } = &mut visible.content
        {
            *content_version = 2;
        }
        let frame = composition.synchronize(extent, vec![visible]).unwrap();
        assert_eq!(frame.updates.len(), 1);
        assert_eq!(frame.updates[0].deltas[0].image_resources.len(), 1);
    }
}

#[cfg(test)]
mod external_admission_tests {
    use super::*;
    fn layer(revision: u64, extent: SizeI, update: ShellImageUpdate) -> ShellLayer {
        ShellLayer::image(
            ShellLayerKey::Surface(1),
            ShellSceneKey::Surface(1),
            revision,
            update,
            extent,
            RectI {
                x: 20,
                y: 30,
                width: extent.width,
                height: extent.height,
            },
            None,
            ImageAlphaMode::Opaque,
            ImagePixelFormat::Rgba8,
            true,
        )
    }
    fn external(revision: u64, damage: Option<RectI>) -> ShellImageUpdate {
        ShellImageUpdate::External {
            image: ImageId(1),
            content_version: revision,
            damage,
        }
    }
    #[test]
    fn external_damage_reaches_output_at_hidpi() {
        let output = SizeI {
            width: 1280,
            height: 800,
        };
        let size = SizeI {
            width: 1000,
            height: 700,
        };
        let mut composition = ShellComposition::new(output);
        composition
            .synchronize(output, vec![layer(1, size, external(1, None))])
            .unwrap();
        let frame = composition
            .synchronize(
                output,
                vec![layer(
                    2,
                    size,
                    external(
                        2,
                        Some(RectI {
                            x: 5,
                            y: 7,
                            width: 10,
                            height: 12,
                        }),
                    ),
                )],
            )
            .unwrap();
        assert_eq!(frame.surface_revisions, [(1, 2)]);
        assert_eq!(
            frame.damage,
            Some(RectI {
                x: 25,
                y: 37,
                width: 10,
                height: 12
            })
        );
        let physical = frame.into_physical(
            crate::platform::ScaleFactor::new(3.0).unwrap(),
            SizeI {
                width: 3840,
                height: 2400,
            },
        );
        assert_eq!(
            physical.damage,
            Some(RectI {
                x: 75,
                y: 111,
                width: 30,
                height: 36
            })
        );
    }
    #[test]
    fn blocked_resize_keeps_old_revision_while_other_placements_move() {
        let output = SizeI {
            width: 1280,
            height: 800,
        };
        let size = SizeI {
            width: 100,
            height: 80,
        };
        let mut composition = ShellComposition::new(output);
        composition
            .synchronize(output, vec![layer(1, size, external(1, None))])
            .unwrap();
        let mut blocked = layer(
            2,
            SizeI {
                width: 200,
                height: 160,
            },
            ShellImageUpdate::Unchanged,
        );
        blocked.target.x += 50;
        let frame = composition.synchronize(output, vec![blocked]).unwrap();
        assert_eq!(frame.surface_revisions, [(1, 1)]);
        assert!(frame.updates.is_empty());
        assert_eq!(frame.placements[0].target.x, 70);
        assert_eq!(
            composition.image_scenes[&ShellSceneKey::Surface(1)].extent,
            size
        );
        let ready = composition
            .synchronize(
                output,
                vec![layer(
                    2,
                    SizeI {
                        width: 200,
                        height: 160,
                    },
                    external(2, None),
                )],
            )
            .unwrap();
        assert_eq!(ready.surface_revisions, [(1, 2)]);
    }
    #[test]
    fn first_unready_image_is_not_presented_or_acknowledged() {
        let output = SizeI {
            width: 1280,
            height: 800,
        };
        let mut composition = ShellComposition::new(output);
        let frame = composition.synchronize(
            output,
            vec![layer(
                1,
                SizeI {
                    width: 100,
                    height: 80,
                },
                ShellImageUpdate::Unchanged,
            )],
        );
        assert!(frame.is_none_or(|f| f.placements.is_empty() && f.surface_revisions.is_empty()));
    }
}

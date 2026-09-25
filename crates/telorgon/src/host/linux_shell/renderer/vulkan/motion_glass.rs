//! Live backdrop effects travel alongside immutable content snapshots, never inside them.
use super::super::super::motion::{
    MotionFrame, SnapshotCommand, SnapshotContent, SnapshotOutput, image_scene,
};
use super::super::super::scene::{ShellLayerKey, ShellPlacement};
use super::*;
use crate::graphics::renderers::vulkan::VulkanFrameContext;

// Admit four full-output RGBA8 surfaces (e.g. two bordered endpoints), with a
// small-output floor and a hard ceiling. This is an estimate of live target pixels;
// completion pins and the separate snapshot/spare/backdrop budgets still apply.
const MIN_RESOLVE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_RESOLVE_BYTES: u64 = 256 * 1024 * 1024;
pub(super) fn target_capacity(extent: SizeI) -> SizeI {
    let round = |value: i32| value.max(1).saturating_add(63) / 64 * 64;
    SizeI {
        width: round(extent.width),
        height: round(extent.height),
    }
}
// Grow by 50% instead of reallocating at each 64-pixel animation step. Keep the
// existing two-times retention limit and charge this capacity to the resolve budget.
fn growing_capacity(extent: SizeI, previous: Option<SizeI>, output: SizeI) -> SizeI {
    let Some(previous) = previous else { return target_capacity(extent); };
    if previous.width >= extent.width && previous.height >= extent.height {
        return target_capacity(extent);
    }
    // Grow both axes together so aspect-preserving animation does not alternate
    // width and height reallocations on adjacent frames.
    let axis = |required: i32, old: i32, limit: i32| {
        required.max(old.saturating_add(old / 2))
            .min(required.max(1).saturating_mul(2).max(64))
            .min(limit.max(required))
    };
    target_capacity(SizeI {
        width: axis(extent.width, previous.width, output.width),
        height: axis(extent.height, previous.height, output.height),
    })
}

pub(super) fn capacity_fits(capacity: SizeI, extent: SizeI) -> bool {
    capacity.width >= extent.width
        && capacity.height >= extent.height
        && capacity.width <= extent.width.max(1).saturating_mul(2).max(64)
        && capacity.height <= extent.height.max(1).saturating_mul(2).max(64)
}
/// Map only the active top-left portion of an overallocated image; never stretch capacity
/// padding into the displayed content. Scene clipping bounds the draw to the active extent.
fn capacity_image_scene(
    extent: SizeI,
    capacity: SizeI,
    opacity: f32,
    additive: bool,
) -> RenderScene {
    let mut scene = image_scene(extent, &[(ImageId(1), opacity)], additive);
    let node = NodeId::new(1, 1);
    let mut instance = *scene.images.get(node).expect("one image");
    instance.rect.width = capacity.width as f32;
    instance.rect.height = capacity.height as f32;
    instance.view_bounds = instance.rect;
    scene.images.upsert(node, instance);
    scene
}

fn resolve_bytes(extent: SizeI, bordered_samples: usize) -> u64 {
    (extent.width.max(0) as u64)
        .saturating_mul(extent.height.max(0) as u64)
        .saturating_mul(4)
        .saturating_mul(1_u64.saturating_add(bordered_samples as u64))
}
fn resolve_budget(output: SizeI) -> u64 {
    resolve_bytes(output, 0)
        .saturating_mul(4)
        .clamp(MIN_RESOLVE_BYTES, MAX_RESOLVE_BYTES)
}
#[cfg(test)]
fn resolve_fits(output: SizeI, extent: SizeI, bordered_samples: usize, other_bytes: u64) -> bool {
    resolve_bytes(extent, bordered_samples).saturating_add(other_bytes) <= resolve_budget(output)
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct GlassSample {
    mapping: RectF,
    placement: ShellPlacement,
    extent: SizeI,
    style: crate::GlassStyle,
    weight: f32,
    border: Option<crate::ui::Border>,
}

#[derive(Clone, Debug, PartialEq)]
struct ResolveSignature {
    extent: SizeI,
    source: u64,
    placement: ShellPlacement,
    samples: Vec<GlassSample>,
    lower: Vec<(ShellPlacement, u64, u64)>,
}
struct LensScene {
    description: RenderScene,
    scene: VulkanScene,
    bordered: Option<BorderedLens>,
    direct_border: Option<VulkanScene>,
}
struct BorderedLens {
    target: VulkanMaterializationTarget,
    border: VulkanScene,
    sample: VulkanScene,
}
struct ResolvedGlass {
    signature: Option<ResolveSignature>,
    target: VulkanMaterializationTarget,
    revision: u64,
    published: Option<(u64, f32)>,
    body: Option<VulkanScene>,
    body_state: Option<(u64, SizeI)>,
    lenses: Vec<LensScene>,
}

#[derive(Default)]
pub(super) struct MotionGlass {
    pub(super) mix_scenes: BTreeMap<usize, Vec<VulkanScene>>,
    pub(super) recipes: BTreeMap<u64, Vec<GlassSample>>,
    pub(super) empty_bodies: std::collections::BTreeSet<u64>,
    // A stable output must not republish an identical scene every cursor-only frame:
    // its epoch is also a backdrop dependency for higher windows.
    pub(super) sampled: BTreeMap<u64, (u64, SizeI, f32)>,
    resolved: BTreeMap<u64, ResolvedGlass>,
}

/// Zero premultiplied pixels remain zero under scaling and any weighted mixture of zeros.
pub(super) fn body_is_empty(
    content: &SnapshotContent,
    body: &[ShellPlacement],
    empty: &std::collections::BTreeSet<u64>,
) -> bool {
    match content {
        SnapshotContent::Capture(_) => body.is_empty(),
        SnapshotContent::Mix(inputs) => inputs
            .iter()
            .all(|input| input.weight == 0.0 || empty.contains(&input.id)),
    }
}

fn can_resolve_directly(empty_body: bool, samples: &[GlassSample]) -> bool {
    empty_body && samples.len() == 1 && samples[0].weight == 1.0
}

/// Return ordinary capture placements and retain the glass recipe separately. Mix weights
/// compose linearly, including interrupted fades; coordinates stay relative to their original
/// capture extent, so repeatedly mixing snapshots cannot accumulate geometry rounding.
#[cfg(test)]
pub(super) fn extract_recipe(
    command: &SnapshotCommand,
    styles: &BTreeMap<ShellSceneKey, crate::GlassStyle>,
    recipes: &mut BTreeMap<u64, Vec<GlassSample>>,
) -> Vec<ShellPlacement> {
    extract_recipe_with_borders(command, styles, &BTreeMap::new(), recipes)
}

pub(super) fn extract_recipe_with_borders(
    command: &SnapshotCommand,
    styles: &BTreeMap<ShellSceneKey, crate::GlassStyle>,
    borders: &BTreeMap<ShellSceneKey, crate::ui::Border>,
    recipes: &mut BTreeMap<u64, Vec<GlassSample>>,
) -> Vec<ShellPlacement> {
    let mut samples = Vec::<GlassSample>::new();
    let mut body = Vec::new();
    match &command.content {
        SnapshotContent::Capture(placements) => {
            for p in placements {
                if let Some(style) = styles.get(&p.scene) {
                    samples.push(GlassSample {
                        placement: *p,
                        mapping: super::super::super::motion::SnapshotInput::from((0, 1.0)).target,
                        extent: command.extent,
                        style: *style,
                        weight: 1.0,
                        border: borders.get(&p.scene).copied(),
                    });
                } else if !matches!(p.scene, ShellSceneKey::ResizePreviewBorder(id)
                    if styles.contains_key(&ShellSceneKey::ResizeVeil(id)))
                {
                    body.push(*p);
                }
            }
        }
        SnapshotContent::Mix(inputs) => {
            for input in inputs {
                let (id, weight) = (&input.id, input.weight);
                if weight <= 0.0 {
                    continue;
                }
                for sample in recipes.get(id).into_iter().flatten() {
                    let mut sample = sample.clone();
                    sample.weight *= weight;
                    sample.mapping = RectF {
                        x: input.target.x + sample.mapping.x * input.target.width,
                        y: input.target.y + sample.mapping.y * input.target.height,
                        width: sample.mapping.width * input.target.width,
                        height: sample.mapping.height * input.target.height,
                    };
                    if let Some(existing) = samples.iter_mut().find(|old| {
                        old.mapping == sample.mapping
                            && old.placement == sample.placement
                            && old.extent == sample.extent
                            && old.style == sample.style
                            && old.border == sample.border
                    }) {
                        existing.weight += sample.weight;
                    } else {
                        samples.push(sample);
                    }
                }
            }
        }
    }
    recipes.insert(command.id, samples);
    body
}

fn map_placement(mut p: ShellPlacement, from: SizeI, to: RectI) -> ShellPlacement {
    let sx = to.width as f32 / from.width.max(1) as f32;
    let sy = to.height as f32 / from.height.max(1) as f32;
    let float = |r: RectF| RectF {
        x: to.x as f32 + r.x * sx,
        y: to.y as f32 + r.y * sy,
        width: r.width * sx,
        height: r.height * sy,
    };
    let rect = |r: RectI| RectI {
        x: to.x + (r.x as f32 * sx).round() as i32,
        y: to.y + (r.y as f32 * sy).round() as i32,
        width: (r.width as f32 * sx).round().max(1.0) as i32,
        height: (r.height as f32 * sy).round().max(1.0) as i32,
    };
    p.target = rect(p.target);
    p.clip = p.clip.map(rect);
    for c in p.rounded_clips.iter_mut().flatten() {
        c.rect = float(c.rect);
    }
    p
}

fn lens_placement(sample: &GlassSample, output: ShellPlacement) -> ShellPlacement {
    let mapped = RectI {
        x: (output.target.x as f32 + sample.mapping.x * output.target.width as f32).round() as i32,
        y: (output.target.y as f32 + sample.mapping.y * output.target.height as f32).round() as i32,
        width: (sample.mapping.width * output.target.width as f32).round() as i32,
        height: (sample.mapping.height * output.target.height as f32).round() as i32,
    };
    let mut p = map_placement(sample.placement, sample.extent, mapped);
    // Maximize/restore deliberately strips the snapshot's rounded contour. Apply the
    // current physical contour to the live optical surface instead of scaling its radius.
    if let Some(outer) = output.rounded_clips[0].filter(|c| !c.inverted) {
        // Captures may include different amounts of shadow padding at the two endpoints.
        // A whole-window veil follows the displayed outer frame, not that old padding ratio.
        let old_target = p.target;
        p.target = RectI {
            x: outer.rect.x.round() as i32,
            y: outer.rect.y.round() as i32,
            width: outer.rect.width.round().max(1.0) as i32,
            height: outer.rect.height.round().max(1.0) as i32,
        };
        if p.clip == Some(old_target) {
            p.clip = Some(p.target);
        }
        p.rounded_clips[0] = Some(outer);
    }
    p
}

/// Translate the draw into a window-sized intermediate without changing the lens's
/// screen-space geometry/UVs. The final sample is 1:1, with visibility applied once.
fn lens_draw(
    placement: ShellPlacement,
    output: ShellPlacement,
    extent: SizeI,
) -> VulkanCompositePlacement {
    let mut coverage = placement.rounded_clips;
    if coverage[0] == output.rounded_clips[0] {
        coverage[0] = None;
    }
    for c in coverage.iter_mut().flatten() {
        c.rect.x -= output.target.x as f32;
        c.rect.y -= output.target.y as f32;
    }
    let clip = placement
        .clip
        .and_then(|c| intersect_rect(c, placement.target))
        .unwrap_or(placement.target);
    VulkanCompositePlacement {
        scene_index: 0,
        target: RectI {
            x: -output.target.x,
            y: -output.target.y,
            ..full_rect(extent)
        },
        clip: Some(RectI {
            x: clip.x - output.target.x,
            y: clip.y - output.target.y,
            ..clip
        }),
        rounded_clips: coverage,
    }
}

fn render_into(
    device: &VulkanDevice,
    scene: &mut VulkanScene,
    placement: VulkanCompositePlacement,
    target: &mut VulkanMaterializationTarget,
    active_extent: SizeI,
    clear: bool,
    context: &mut VulkanFrameContext<'_>,
) -> AppResult<()> {
    context.core.images.push(target.image());
    device
        .render_composite(
            &mut [VulkanCompositeScene { scene }],
            &[placement],
            &mut VulkanFrameContext {
                core: &mut *context.core,
            },
            &target.target(),
            &RenderRequest {
                force: true,
                load: if clear {
                    TargetLoad::Clear(ColorRgba8::rgba(0, 0, 0, 0))
                } else {
                    TargetLoad::Preserve
                },
                store: TargetStore::Store,
                // Capacity padding is never sampled; avoid clearing/shading it on shrink.
                region: Some(full_rect(active_extent)),
            },
        )
        .map_err(app_error)?;
    target.mark_initialized();
    Ok(())
}

fn update_border_scene(
    device: &VulkanDevice,
    scene: &mut VulkanScene,
    lens: ShellPlacement,
    output: ShellPlacement,
    extent: SizeI,
    border: crate::ui::Border,
) -> AppResult<()> {
    use crate::graphics::render::BoxInstance;
    let rect = RectF {
        x: (lens.target.x - output.target.x) as f32,
        y: (lens.target.y - output.target.y) as f32,
        width: lens.target.width as f32,
        height: lens.target.height as f32,
    };
    let node = NodeId::new(0, 1);
    let mut description = RenderScene::default();
    description.extent = crate::SizeF {
        width: extent.width as f32,
        height: extent.height as f32,
    };
    description.boxes.upsert(
        node,
        BoxInstance {
            node,
            rect,
            view_bounds: rect,
            background: None,
            border,
            outline: Default::default(),
            corner_radii: lens.rounded_clips[0].map_or(Default::default(), |c| c.radii),
            shadows: Default::default(),
            opacity: 1.0,
            clip: ClipId(0),
            spatial: SpatialId(0),
        },
    );
    description.set_draw_order(vec![DrawItem {
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
    let mut delta = description.take_delta().unwrap();
    delta.epoch = scene
        .epoch()
        .checked_add(1)
        .ok_or_else(|| AppError::new("preview border epoch exhausted"))?;
    device.apply_scene_delta(scene, &delta).map_err(app_error)?;
    Ok(())
}

fn publish_resolved(
    device: &VulkanDevice,
    scenes: &mut BTreeMap<ShellSceneKey, VulkanScene>,
    resolved: &mut ResolvedGlass,
    output: &SnapshotOutput,
) -> AppResult<()> {
    let key = ShellSceneKey::Motion(output.id);
    let state = (resolved.revision, output.opacity);
    if resolved.published == Some(state) && scenes.contains_key(&key) {
        return Ok(());
    }
    let scene = match scenes.entry(key) {
        std::collections::btree_map::Entry::Occupied(e) => e.into_mut(),
        std::collections::btree_map::Entry::Vacant(e) => {
            e.insert(device.create_scene().map_err(app_error)?)
        }
    };
    let mut description = capacity_image_scene(
        output.extent,
        resolved.target.extent(),
        output.opacity,
        false,
    );
    scene
        .bind_materialized_image(ImageId(1), &resolved.target, ImageAlphaMode::Premultiplied)
        .map_err(app_error)?;
    let mut delta = description.take_delta().unwrap();
    delta.epoch = scene
        .epoch()
        .checked_add(1)
        .ok_or_else(|| AppError::new("motion glass epoch exhausted"))?;
    device.apply_scene_delta(scene, &delta).map_err(app_error)?;
    resolved.published = Some(state);
    Ok(())
}

/// Participating tiles sample one desktop behind the group, not one another's animated
/// previews. Blur radius selects the cache; tint, refraction and fade weights stay per lens.
#[derive(Default)]
struct SharedBackdrop {
    keys: BTreeMap<ShellSceneKey, ShellSceneKey>,
    lower: Vec<ShellPlacement>,
}
fn shared_backdrop(
    frame: &ShellFrame,
    recipes: &BTreeMap<u64, Vec<GlassSample>>,
) -> SharedBackdrop {
    let members = &frame.motion.resize_group;
    if members.len() < 2 {
        return SharedBackdrop::default();
    }
    let belongs = |p: &ShellPlacement| {
        matches!(p.key,
        ShellLayerKey::Motion(id) | ShellLayerKey::MotionShadow(id) | ShellLayerKey::FrameShadow(id)
        if members.contains(&id))
    };
    let first = frame
        .placements
        .iter()
        .position(belongs)
        .unwrap_or(frame.placements.len());
    let mut plan = SharedBackdrop {
        lower: frame.placements[..first].to_vec(),
        ..Default::default()
    };
    let mut radii = BTreeMap::new();
    let mut scene_radii = BTreeMap::new();
    for endpoint in &frame.motion.outputs {
        if !u32::try_from(endpoint.id).is_ok_and(|id| members.contains(&id)) {
            continue;
        }
        for sample in recipes.get(&endpoint.source).into_iter().flatten() {
            if !matches!(sample.placement.scene, ShellSceneKey::ResizeVeil(_)) {
                continue;
            }
            let radius = glass::normalized_blur_radius(sample.style.blur_radius).to_bits();
            // Interrupted style changes can retain two radii for one scene key. Such
            // recipes require independent sequential preparation, not a shared alias.
            if scene_radii
                .insert(sample.placement.scene, radius)
                .is_some_and(|old| old != radius)
            {
                return SharedBackdrop::default();
            }
            let key = *radii.entry(radius).or_insert(sample.placement.scene);
            plan.keys.insert(sample.placement.scene, key);
        }
    }
    plan
}

/// Resolve bottom to top, after content snapshots exist. Lower moving windows are already
/// in their displayed geometry when an upper lens samples them. Allocation failure uses the
/// existing immediate-presentation fallback, never a frozen backdrop in a snapshot.
pub(super) fn record_output(
    device: &VulkanDevice,
    scenes: &mut BTreeMap<ShellSceneKey, VulkanScene>,
    snapshots: &BTreeMap<u64, motion::MotionSnapshot>,
    spares: &mut Vec<VulkanMaterializationTarget>,
    state: &mut MotionGlass,
    caches: &mut BTreeMap<ShellSceneKey, glass::GlassCache>,
    frame: &ShellFrame,
    context: &mut VulkanFrameContext<'_>,
) -> AppResult<Option<Vec<ShellPlacement>>> {
    let _timing = super::super::super::preview_trace::span("glass_resolve");
    let active = frame
        .motion
        .outputs
        .iter()
        .filter(|o| state.recipes.get(&o.source).is_some_and(|s| !s.is_empty()))
        .map(|o| o.id)
        .collect::<std::collections::BTreeSet<_>>();
    state.resolved.retain(|id, _| active.contains(id));
    let shared = shared_backdrop(frame, &state.recipes);
    let cache_key = |key: ShellSceneKey| shared.keys.get(&key).copied().unwrap_or(key);
    let mut output = frame.placements.clone();
    let mut live_glass = frame
        .glass
        .keys()
        .copied()
        .map(cache_key)
        .collect::<std::collections::BTreeSet<_>>();
    // Retired lenses must release their cache owners before admitting replacements.
    // Submission image pins still protect any previous GPU work.
    for endpoint in &frame.motion.outputs {
        for sample in state.recipes.get(&endpoint.source).into_iter().flatten() {
            live_glass.insert(cache_key(sample.placement.scene));
        }
    }
    caches.retain(|key, _| live_glass.contains(key));
    for index in 0..output.len() {
        let p = output[index];
        if frame.glass.contains_key(&p.scene) {
            let replaced =
                glass::record_glass(device, scenes, caches, frame, &output[..=index], context)?;
            output[index] = replaced[index];
            continue;
        }
        let ShellSceneKey::Motion(id) = p.scene else {
            continue;
        };
        let Some(endpoint) = frame.motion.outputs.iter().find(|o| o.id == id) else {
            continue;
        };
        let Some(samples) = state
            .recipes
            .get(&endpoint.source)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        for sample in samples {
            live_glass.insert(cache_key(sample.placement.scene));
        }
        let owner = match p.key {
            ShellLayerKey::Motion(owner) => owner,
            ShellLayerKey::Widget(_) => 0,
            _ => continue,
        };
        let signature = ResolveSignature {
            extent: frame.extent,
            source: endpoint.source,
            placement: p,
            samples: samples.clone(),
            lower: glass::backdrop_sources(
                owner,
                if samples
                    .iter()
                    .all(|s| shared.keys.contains_key(&s.placement.scene))
                {
                    &shared.lower
                } else {
                    &output[..index]
                },
                scenes,
                caches,
            ),
        };
        let other_bytes: u64 = state
            .resolved
            .iter()
            .filter(|(key, _)| **key != id)
            .map(|(_, r)| {
                r.target.allocated_bytes()
                    + r.lenses
                        .iter()
                        .filter_map(|l| l.bordered.as_ref())
                        .map(|b| b.target.allocated_bytes())
                        .sum::<u64>()
            })
            .sum();
        let direct = can_resolve_directly(state.empty_bodies.contains(&endpoint.source), samples);
        // Charge retained capacities (including Vulkan allocation padding), not just
        // the smaller active rectangle after a shrink.
        let previous = state.resolved.get(&id);
        let allocation_bytes = |target: Option<&VulkanMaterializationTarget>| {
            target
                .filter(|t| capacity_fits(t.extent(), endpoint.extent))
                .map_or_else(
                    || resolve_bytes(growing_capacity(endpoint.extent, target.map(|t| t.extent()), frame.extent), 0),
                    |t| t.allocated_bytes(),
                )
        };
        let own_bytes = samples
            .iter()
            .enumerate()
            .filter(|(_, s)| !direct && s.border.is_some())
            .fold(
                allocation_bytes(previous.map(|r| &r.target)),
                |bytes, (index, _)| {
                    bytes.saturating_add(allocation_bytes(
                        previous
                            .and_then(|r| r.lenses.get(index))
                            .and_then(|l| l.bordered.as_ref())
                            .map(|b| &b.target),
                    ))
                },
            );
        if own_bytes.saturating_add(other_bytes) > resolve_budget(frame.extent) {
            eprintln!(
                "telorgon-motion: resolve budget rejected output={id} extent={:?} other_allocated_bytes={other_bytes} budget_bytes={}",
                endpoint.extent,
                resolve_budget(frame.extent)
            );
            return Ok(None);
        }
        if !state
            .resolved
            .get(&id)
            .is_some_and(|r| capacity_fits(r.target.extent(), endpoint.extent))
        {
            let capacity = growing_capacity(endpoint.extent,
                state.resolved.get(&id).map(|r| r.target.extent()), frame.extent);
            let target = if let Some(i) = spares
                .iter()
                .position(|t| t.extent() == capacity && t.can_recycle())
            {
                spares.swap_remove(i)
            } else {
                let _allocation = super::super::super::preview_trace::span("resolve_allocate");
                let Ok(target) = VulkanMaterializationTarget::new_traced(
                    device,
                    capacity,
                    &mut |_| {},
                ) else {
                    eprintln!(
                        "telorgon-motion: resolve allocation failed output={id} extent={:?}",
                        endpoint.extent
                    );
                    return Ok(None);
                };
                target
            };
            let previous = state.resolved.remove(&id);
            let (body, lenses) = if let Some(previous) = previous {
                spares.push(previous.target);
                (previous.body, previous.lenses)
            } else {
                (None, Vec::new())
            };
            state.resolved.insert(
                id,
                ResolvedGlass {
                    signature: None,
                    target,
                    revision: 0,
                    published: None,
                    body,
                    body_state: None,
                    lenses,
                },
            );
        }
        let resolved = state.resolved.get_mut(&id).unwrap();
        if resolved.signature.as_ref() != Some(&signature) {
            if !direct {
                // The captured/mixed image contains only window pixels and their fade weights.
                if resolved.body.is_none() {
                    resolved.body = Some(device.create_scene().map_err(app_error)?);
                }
                let body = resolved.body.as_mut().unwrap();
                let body_state = (endpoint.source, endpoint.extent);
                if resolved.body_state != Some(body_state) {
                    motion::update_sample_scene(
                        device,
                        body,
                        snapshots,
                        endpoint.extent,
                        &[(endpoint.source, 1.0).into()],
                        true,
                    )?;
                    resolved.body_state = Some(body_state);
                }
                render_into(
                    device,
                    body,
                    VulkanCompositePlacement {
                        scene_index: 0,
                        target: full_rect(endpoint.extent),
                        clip: None,
                        rounded_clips: [None; 2],
                    },
                    &mut resolved.target,
                    endpoint.extent,
                    true,
                    context,
                )?;
            } else {
                resolved.body = None;
                resolved.body_state = None;
            }
            for (sample_index, sample) in samples.iter().enumerate() {
                if !matches!(
                    sample.placement.scene,
                    ShellSceneKey::ResizeVeil(_) | ShellSceneKey::TilePreview(_)
                ) {
                    continue;
                }
                let lens = lens_placement(sample, p);
                let region = if shared.keys.contains_key(&sample.placement.scene) {
                    full_rect(frame.extent)
                } else {
                    glass::backdrop_region(frame.extent, lens.target, sample.style)
                };
                let capture_extent = SizeI {
                    width: region.width,
                    height: region.height,
                };
                if !glass::prepare_backdrop_key(
                    device,
                    scenes,
                    caches,
                    frame.extent,
                    region,
                    cache_key(sample.placement.scene),
                    sample.style,
                    if shared.keys.contains_key(&sample.placement.scene) {
                        &shared.lower
                    } else {
                        &output[..index]
                    },
                    context,
                )? {
                    return Ok(None);
                }

                if resolved.lenses.len() <= sample_index {
                    resolved.lenses.push(LensScene {
                        description: RenderScene::default(),
                        scene: device.create_scene().map_err(app_error)?,
                        bordered: None,
                        direct_border: None,
                    });
                }
                let LensScene {
                    description,
                    scene,
                    bordered,
                    direct_border,
                } = &mut resolved.lenses[sample_index];
                glass::update_lens_weighted(
                    description,
                    capture_extent,
                    glass::localize(lens, region),
                    sample.style,
                    if sample.border.is_some() {
                        1.0
                    } else {
                        sample.weight
                    },
                    BlendMode::Add,
                );
                glass::bind_backdrops(scene, &caches[&cache_key(sample.placement.scene)])?;
                if let Some(mut delta) = description.take_delta() {
                    delta.epoch = scene
                        .epoch()
                        .checked_add(1)
                        .ok_or_else(|| AppError::new("motion lens epoch exhausted"))?;
                    device.apply_scene_delta(scene, &delta).map_err(app_error)?;
                }
                let mut lens_placement = lens_draw(lens, p, capture_extent);
                lens_placement.target.x += region.x;
                lens_placement.target.y += region.y;
                if direct && sample.border.is_some() {
                    // Body = 0 and lens weight = 1: border-over-glass can go straight
                    // into the resolve, with no separately weighted intermediate.
                    *bordered = None;
                    if direct_border.is_none() {
                        *direct_border = Some(device.create_scene().map_err(app_error)?);
                    }
                    render_into(
                        device,
                        scene,
                        lens_placement,
                        &mut resolved.target,
                        endpoint.extent,
                        true,
                        context,
                    )?;
                    let border_scene = direct_border.as_mut().unwrap();
                    update_border_scene(
                        device,
                        border_scene,
                        lens,
                        p,
                        endpoint.extent,
                        sample.border.unwrap(),
                    )?;
                    render_into(
                        device,
                        border_scene,
                        VulkanCompositePlacement {
                            scene_index: 0,
                            target: full_rect(endpoint.extent),
                            clip: None,
                            rounded_clips: [None; 2],
                        },
                        &mut resolved.target,
                        endpoint.extent,
                        false,
                        context,
                    )?;
                } else if let Some(border) = sample.border {
                    *direct_border = None;

                    if !bordered
                        .as_ref()
                        .is_some_and(|b| capacity_fits(b.target.extent(), endpoint.extent))
                    {
                        let capacity = growing_capacity(endpoint.extent,
                            bordered.as_ref().map(|b| b.target.extent()), frame.extent);
                        let Ok(target) = VulkanMaterializationTarget::new_traced(
                            device,
                            capacity,
                            &mut |_| {},
                        ) else {
                            return Ok(None);
                        };
                        *bordered = Some(BorderedLens {
                            target,
                            border: device.create_scene().map_err(app_error)?,
                            sample: device.create_scene().map_err(app_error)?,
                        });
                    }
                    let bordered = bordered.as_mut().unwrap();
                    render_into(
                        device,
                        scene,
                        lens_placement,
                        &mut bordered.target,
                        endpoint.extent,
                        true,
                        context,
                    )?;
                    update_border_scene(
                        device,
                        &mut bordered.border,
                        lens,
                        p,
                        endpoint.extent,
                        border,
                    )?;
                    let placement = VulkanCompositePlacement {
                        scene_index: 0,
                        target: full_rect(endpoint.extent),
                        clip: None,
                        rounded_clips: [None; 2],
                    };
                    render_into(
                        device,
                        &mut bordered.border,
                        placement,
                        &mut bordered.target,
                        endpoint.extent,
                        false,
                        context,
                    )?;
                    let mut desc = capacity_image_scene(
                        endpoint.extent,
                        bordered.target.extent(),
                        sample.weight,
                        true,
                    );
                    bordered
                        .sample
                        .bind_materialized_image(
                            ImageId(1),
                            &bordered.target,
                            ImageAlphaMode::Premultiplied,
                        )
                        .map_err(app_error)?;
                    let mut delta = desc.take_delta().unwrap();
                    delta.epoch = bordered
                        .sample
                        .epoch()
                        .checked_add(1)
                        .ok_or_else(|| AppError::new("bordered glass epoch exhausted"))?;
                    device
                        .apply_scene_delta(&mut bordered.sample, &delta)
                        .map_err(app_error)?;
                    render_into(
                        device,
                        &mut bordered.sample,
                        placement,
                        &mut resolved.target,
                        endpoint.extent,
                        false,
                        context,
                    )?;
                } else {
                    *bordered = None;
                    *direct_border = None;
                    render_into(
                        device,
                        scene,
                        lens_placement,
                        &mut resolved.target,
                        endpoint.extent,
                        direct,
                        context,
                    )?;
                }
            }
            resolved.lenses.truncate(samples.len());
            resolved.signature = Some(signature);
            resolved.revision = resolved.revision.wrapping_add(1).max(1);
        }
        publish_resolved(device, scenes, resolved, endpoint)?;
    }
    caches.retain(|key, _| live_glass.contains(key));
    motion::trim_spares(spares);
    Ok(Some(output))
}

pub(super) fn retire_recipes(state: &mut MotionGlass, motion: &MotionFrame) {
    state.recipes.retain(|id, _| motion.live.contains(id));
    state.empty_bodies.retain(|id| motion.live.contains(id));
    state
        .sampled
        .retain(|id, _| motion.outputs.iter().any(|o| o.id == *id));
}

#[cfg(test)]
#[path = "motion_glass_tests.rs"]
mod tests;

#[cfg(test)]
mod capacity_tests {
    use super::*;
    #[test]
    fn transition_capacity_growth_avoids_reallocating_every_animation_step() {
        let output = SizeI { width: 3840, height: 2304 };
        let mut capacity = None;
        let mut allocations = 0;
        for width in (1024..=3840).step_by(64) {
            let extent = SizeI { width, height: width * 3 / 5 };
            if capacity.is_none_or(|old| !capacity_fits(old, extent)) {
                capacity = Some(growing_capacity(extent, capacity, output));
                allocations += 1;
            }
            assert!(capacity_fits(capacity.unwrap(), extent));
            assert!(capacity.unwrap().width <= output.width);
            assert!(capacity.unwrap().height <= output.height);
        }
        assert!(allocations <= 5, "capacity should grow geometrically: {allocations}");
        let small = SizeI { width: 100, height: 100 };
        assert_eq!(growing_capacity(small, capacity, output), target_capacity(small));
    }
}

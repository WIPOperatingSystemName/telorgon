//! Live backdrop effects travel alongside immutable content snapshots, never inside them.
use super::super::super::motion::{MotionFrame, SnapshotCommand, SnapshotContent, SnapshotOutput};
use super::super::super::scene::{DesktopLayerKey, DesktopPlacement};
use super::*;
use crate::renderer_vulkan::VulkanFrameContext;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct GlassSample {
    placement: DesktopPlacement,
    extent: SizeI,
    style: crate::GlassStyle,
    weight: f32,
}

#[derive(Clone, Debug, PartialEq)]
struct ResolveSignature {
    extent: SizeI,
    source: u64,
    placement: DesktopPlacement,
    samples: Vec<GlassSample>,
    lower: Vec<(DesktopPlacement, u64, u64)>,
}
struct LensScene {
    description: RenderScene,
    scene: VulkanScene,
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
    pub(super) recipes: BTreeMap<u64, Vec<GlassSample>>,
    // A stable output must not republish an identical scene every cursor-only frame:
    // its epoch is also a backdrop dependency for higher windows.
    pub(super) sampled: BTreeMap<u64, (u64, SizeI, f32)>,
    resolved: BTreeMap<u64, ResolvedGlass>,
}

/// Return ordinary capture placements and retain the glass recipe separately. Mix weights
/// compose linearly, including interrupted fades; coordinates stay relative to their original
/// capture extent, so repeatedly mixing snapshots cannot accumulate geometry rounding.
pub(super) fn extract_recipe(
    command: &SnapshotCommand,
    styles: &BTreeMap<DesktopSceneKey, crate::GlassStyle>,
    recipes: &mut BTreeMap<u64, Vec<GlassSample>>,
) -> Vec<DesktopPlacement> {
    let mut samples = Vec::<GlassSample>::new();
    let mut body = Vec::new();
    match &command.content {
        SnapshotContent::Capture(placements) => {
            for p in placements {
                if let Some(style) = styles.get(&p.scene) {
                    samples.push(GlassSample {
                        placement: *p,
                        extent: command.extent,
                        style: *style,
                        weight: 1.0,
                    });
                } else {
                    body.push(*p);
                }
            }
        }
        SnapshotContent::Mix(inputs) => {
            for (id, weight) in inputs {
                if *weight <= 0.0 {
                    continue;
                }
                for sample in recipes.get(id).into_iter().flatten() {
                    let mut sample = sample.clone();
                    sample.weight *= weight;
                    if let Some(existing) = samples.iter_mut().find(|old| {
                        old.placement == sample.placement
                            && old.extent == sample.extent
                            && old.style == sample.style
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

fn map_placement(mut p: DesktopPlacement, from: SizeI, to: RectI) -> DesktopPlacement {
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

fn lens_placement(sample: &GlassSample, output: DesktopPlacement) -> DesktopPlacement {
    let mut p = map_placement(sample.placement, sample.extent, output.target);
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
    placement: DesktopPlacement,
    output: DesktopPlacement,
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
                region: None,
            },
        )
        .map_err(app_error)?;
    target.mark_initialized();
    Ok(())
}

fn publish_resolved(
    device: &VulkanDevice,
    scenes: &mut BTreeMap<DesktopSceneKey, VulkanScene>,
    resolved: &mut ResolvedGlass,
    output: &SnapshotOutput,
) -> AppResult<()> {
    let key = DesktopSceneKey::Motion(output.id);
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
    let mut description = super::super::super::motion::image_scene(
        output.extent,
        &[(ImageId(1), output.opacity)],
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

/// Resolve bottom to top, after content snapshots exist. Lower moving windows are already
/// in their displayed geometry when an upper lens samples them. Allocation failure uses the
/// existing immediate-presentation fallback, never a frozen backdrop in a snapshot.
pub(super) fn record_output(
    device: &VulkanDevice,
    scenes: &mut BTreeMap<DesktopSceneKey, VulkanScene>,
    snapshots: &BTreeMap<u64, VulkanMaterializationTarget>,
    spares: &mut Vec<VulkanMaterializationTarget>,
    state: &mut MotionGlass,
    caches: &mut BTreeMap<DesktopSceneKey, glass::GlassCache>,
    frame: &DesktopFrame,
    context: &mut VulkanFrameContext<'_>,
) -> AppResult<Option<Vec<DesktopPlacement>>> {
    let active = frame
        .motion
        .outputs
        .iter()
        .filter(|o| state.recipes.get(&o.source).is_some_and(|s| !s.is_empty()))
        .map(|o| o.id)
        .collect::<std::collections::BTreeSet<_>>();
    state.resolved.retain(|id, _| active.contains(id));
    let mut output = frame.placements.clone();
    let mut live_glass = frame
        .glass
        .keys()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    for index in 0..output.len() {
        let p = output[index];
        if frame.glass.contains_key(&p.scene) {
            let replaced =
                glass::record_glass(device, scenes, caches, frame, &output[..=index], context)?;
            output[index] = replaced[index];
            continue;
        }
        let DesktopSceneKey::Motion(id) = p.scene else {
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
            live_glass.insert(sample.placement.scene);
        }
        let owner = match p.key {
            DesktopLayerKey::Motion(owner) => owner,
            _ => continue,
        };
        let signature = ResolveSignature {
            extent: frame.extent,
            source: endpoint.source,
            placement: p,
            samples: samples.clone(),
            lower: glass::backdrop_sources(owner, &output[..index], scenes, caches),
        };
        if !state
            .resolved
            .get(&id)
            .is_some_and(|r| r.target.extent() == endpoint.extent)
        {
            let other_bytes = state
                .resolved
                .iter()
                .filter(|(key, _)| **key != id)
                .map(|(_, r)| r.target.allocated_bytes())
                .sum::<u64>();
            let bytes = endpoint.extent.width as u64 * endpoint.extent.height as u64 * 4;
            if bytes + other_bytes > 64 * 1024 * 1024 {
                return Ok(None);
            }
            let target = if let Some(i) = spares
                .iter()
                .position(|t| t.extent() == endpoint.extent && t.can_recycle())
            {
                spares.swap_remove(i)
            } else {
                let Ok(target) =
                    VulkanMaterializationTarget::new_traced(device, endpoint.extent, &mut |_| {})
                else {
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
                    &[(endpoint.source, 1.0)],
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
                true,
                context,
            )?;
            for (sample_index, sample) in samples.iter().enumerate() {
                let DesktopSceneKey::ResizeVeil(glass_owner) = sample.placement.scene else {
                    continue;
                };
                if !glass::prepare_backdrop(
                    device,
                    scenes,
                    caches,
                    frame.extent,
                    glass_owner,
                    sample.style,
                    &output[..index],
                    context,
                )? {
                    return Ok(None);
                }
                let lens = lens_placement(sample, p);
                if resolved.lenses.len() <= sample_index {
                    resolved.lenses.push(LensScene {
                        description: RenderScene::default(),
                        scene: device.create_scene().map_err(app_error)?,
                    });
                }
                let LensScene { description, scene } = &mut resolved.lenses[sample_index];
                glass::update_lens_weighted(
                    description,
                    frame.extent,
                    lens,
                    sample.style,
                    sample.weight,
                    BlendMode::Add,
                );
                glass::bind_backdrops(scene, &caches[&sample.placement.scene])?;
                if let Some(mut delta) = description.take_delta() {
                    delta.epoch = scene
                        .epoch()
                        .checked_add(1)
                        .ok_or_else(|| AppError::new("motion lens epoch exhausted"))?;
                    device.apply_scene_delta(scene, &delta).map_err(app_error)?;
                }
                render_into(
                    device,
                    scene,
                    lens_draw(lens, p, frame.extent),
                    &mut resolved.target,
                    false,
                    context,
                )?;
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
    state
        .sampled
        .retain(|id, _| motion.outputs.iter().any(|o| o.id == *id));
}

#[cfg(test)]
#[path = "motion_glass_tests.rs"]
mod tests;

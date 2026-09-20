//! Immutable compositor snapshots recorded on the normal frame queue.
use super::super::super::motion::{MotionFrame, SnapshotContent, image_scene};
use super::*;
use crate::graphics::renderers::vulkan::VulkanFrameContext;

pub(super) struct MotionSnapshot {
    pub target: VulkanMaterializationTarget,
    pub extent: SizeI,
}

pub(super) fn record_motion(
    device: &VulkanDevice,
    scenes: &mut BTreeMap<ShellSceneKey, VulkanScene>,
    snapshots: &mut BTreeMap<u64, MotionSnapshot>,
    spares: &mut Vec<VulkanMaterializationTarget>,
    motion: &MotionFrame,
    glass: &BTreeMap<ShellSceneKey, crate::GlassStyle>,
    borders: &BTreeMap<ShellSceneKey, crate::ui::Border>,
    live_glass: &mut super::motion_glass::MotionGlass,
    context: &mut VulkanFrameContext<'_>,
) -> AppResult<bool> {
    let mut targets = Vec::new();
    for command in &motion.snapshots {
        let body = super::motion_glass::extract_recipe_with_borders(
            command,
            glass,
            borders,
            &mut live_glass.recipes,
        );
        let empty =
            super::motion_glass::body_is_empty(&command.content, &body, &live_glass.empty_bodies);
        if empty {
            live_glass.empty_bodies.insert(command.id);
        } else {
            live_glass.empty_bodies.remove(&command.id);
        }
        // Glass recipes are resolved separately. A transparent body needs no window-sized image.
        let extent = if empty {
            SizeI {
                width: 1,
                height: 1,
            }
        } else {
            command.extent
        };
        let target = if let Some(index) = spares
            .iter()
            .position(|t| super::motion_glass::capacity_fits(t.extent(), extent) && t.can_recycle())
        {
            spares.swap_remove(index)
        } else {
            match VulkanMaterializationTarget::new_traced(
                device,
                if empty {
                    extent
                } else {
                    super::motion_glass::target_capacity(extent)
                },
                &mut |_| {},
            ) {
                Ok(target) => target,
                Err(error) => {
                    eprintln!(
                        "telorgon-motion: snapshot allocation failed id={} extent={:?}: {error:?}",
                        command.id, command.extent
                    );
                    return Ok(false);
                }
            }
        };
        targets.push((target, body));
    }
    for (command, (mut target, body)) in motion.snapshots.iter().zip(targets) {
        // Bound distinct optical endpoints during repeated interruptions just like snapshot
        // allocation. Immediate presentation is preferable to exhausting descriptor capacity.
        if live_glass.recipes[&command.id].len() > 16 {
            eprintln!(
                "telorgon-motion: optical endpoint limit exceeded snapshot={} endpoints={}",
                command.id,
                live_glass.recipes[&command.id].len()
            );
            return Ok(false);
        }

        // The attachment is also pinned when no sampled output uses it in this submission.
        context.core.images.push(target.image());
        let request = RenderRequest {
            force: true,
            load: TargetLoad::Clear(ColorRgba8::rgba(0, 0, 0, 0)),
            store: TargetStore::Store,
            region: None,
        };
        let transparent = SnapshotContent::Capture(Vec::new());
        let content = if live_glass.empty_bodies.contains(&command.id) {
            &transparent
        } else {
            &command.content
        };
        match content {
            SnapshotContent::Capture(_) => {
                let placements = &body;
                let indices = scenes
                    .keys()
                    .enumerate()
                    .map(|(i, k)| (*k, i))
                    .collect::<BTreeMap<_, _>>();
                let placements = placements
                    .iter()
                    .map(|p| {
                        Ok(VulkanCompositePlacement {
                            scene_index: *indices.get(&p.scene).ok_or_else(|| {
                                AppError::new(format!(
                                    "motion capture {} scene {:?} missing (layer {:?})",
                                    command.id, p.scene, p.key
                                ))
                            })?,
                            target: p.target,
                            clip: p.clip,
                            rounded_clips: p.rounded_clips,
                        })
                    })
                    .collect::<AppResult<Vec<_>>>()?;
                let mut sources = scenes
                    .values_mut()
                    .map(|scene| VulkanCompositeScene { scene })
                    .collect::<Vec<_>>();
                device
                    .render_composite(
                        &mut sources,
                        &placements,
                        &mut VulkanFrameContext {
                            core: &mut *context.core,
                        },
                        &target.target(),
                        &request,
                    )
                    .map_err(app_error)?;
            }
            SnapshotContent::Mix(inputs) => {
                let mut source = sample_scene(device, snapshots, command.extent, inputs, true)?;
                device
                    .render_composite(
                        &mut [VulkanCompositeScene { scene: &mut source }],
                        &[VulkanCompositePlacement {
                            scene_index: 0,
                            target: full_rect(command.extent),
                            clip: None,
                            rounded_clips: [None; 2],
                        }],
                        &mut VulkanFrameContext {
                            core: &mut *context.core,
                        },
                        &target.target(),
                        &request,
                    )
                    .map_err(app_error)?;
            }
        }
        // Subsequent passes in this same command buffer sample the final SHADER_READ state.
        target.mark_initialized();
        snapshots.insert(
            command.id,
            MotionSnapshot {
                extent: if live_glass.empty_bodies.contains(&command.id) {
                    SizeI {
                        width: 1,
                        height: 1,
                    }
                } else {
                    command.extent
                },
                target,
            },
        );
    }
    for output in &motion.outputs {
        // Live glass is resolved at the displayed geometry after ordinary snapshots exist.
        if live_glass
            .recipes
            .get(&output.source)
            .is_some_and(|s| !s.is_empty())
        {
            live_glass.sampled.remove(&output.id);
            continue;
        }
        let sampled = (output.source, output.extent, output.opacity);
        if live_glass.sampled.get(&output.id) == Some(&sampled)
            && scenes.contains_key(&ShellSceneKey::Motion(output.id))
        {
            continue;
        }
        let scene = match scenes.entry(ShellSceneKey::Motion(output.id)) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(device.create_scene().map_err(app_error)?)
            }
        };
        update_sample_scene(
            device,
            scene,
            snapshots,
            output.extent,
            &[(output.source, output.opacity)],
            false,
        )?;
        live_glass.sampled.insert(output.id, sampled);
    }
    super::motion_glass::retire_recipes(live_glass, motion);
    // Recorded image pins outlive these owner handles, including cancelled/replaced fades.
    let retired = snapshots
        .keys()
        .filter(|id| !motion.live.contains(id))
        .copied()
        .collect::<Vec<_>>();
    for id in retired {
        spares.push(snapshots.remove(&id).expect("retired snapshot").target);
    }
    trim_spares(spares);
    Ok(true)
}
pub(super) fn trim_spares(spares: &mut Vec<VulkanMaterializationTarget>) {
    while spares.len() > 16
        || spares.iter().map(|t| t.allocated_bytes()).sum::<u64>() > 64 * 1024 * 1024
    {
        spares.remove(0);
    }
}

pub(super) fn sample_scene(
    device: &VulkanDevice,
    snapshots: &BTreeMap<u64, MotionSnapshot>,
    extent: SizeI,
    inputs: &[(u64, f32)],
    additive: bool,
) -> AppResult<VulkanScene> {
    let mut scene = device.create_scene().map_err(app_error)?;
    update_sample_scene(device, &mut scene, snapshots, extent, inputs, additive)?;
    Ok(scene)
}
pub(super) fn update_sample_scene(
    device: &VulkanDevice,
    scene: &mut VulkanScene,
    snapshots: &BTreeMap<u64, MotionSnapshot>,
    extent: SizeI,
    inputs: &[(u64, f32)],
    additive: bool,
) -> AppResult<()> {
    let images = inputs
        .iter()
        .enumerate()
        .map(|(i, (_, weight))| (ImageId(i as u32 + 1), *weight))
        .collect::<Vec<_>>();
    let samples = inputs
        .iter()
        .zip(&images)
        .map(|((id, _), (image, weight))| {
            let snapshot = snapshots
                .get(id)
                .ok_or_else(|| AppError::new("motion snapshot missing"))?;
            Ok((*image, *weight, snapshot.extent, snapshot.target.extent()))
        })
        .collect::<AppResult<Vec<_>>>()?;
    let mut source = image_scene(extent, &images, additive);
    // Delta validation requires every referenced image to be bound already, just as
    // for client DMA-BUF scenes. Bind before publishing the sampled draw instances.
    for ((snapshot, _), (image, _)) in inputs.iter().zip(images) {
        let target = snapshots
            .get(snapshot)
            .ok_or_else(|| AppError::new("motion snapshot missing"))?;
        scene
            .bind_materialized_image(image, &target.target, ImageAlphaMode::Premultiplied)
            .map_err(app_error)?;
    }
    let mut delta = source.take_delta().expect("new motion image scene");
    delta.epoch = scene
        .epoch()
        .checked_add(1)
        .expect("motion scene epoch exhausted");
    device.apply_scene_delta(scene, &delta).map_err(app_error)?;
    for (image, _, active, capacity) in samples {
        scene.crop_materialized_image(image, active, capacity);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires TELORGON_TEST_MODE=developer-hardware and a Vulkan adapter"]
    fn motion_sample_scenes_bind_images_before_delta_validation() {
        assert_eq!(
            std::env::var("TELORGON_TEST_MODE").as_deref(),
            Ok("developer-hardware")
        );
        let config = VulkanConfig {
            enable_validation: true,
            ..VulkanConfig::default()
        };
        let instance = VulkanInstance::load(&config, &[]).unwrap();
        let selection = DeviceSelection::best(&instance.adapters().unwrap()).unwrap();
        let device = VulkanDevice::create_owned(instance, &config, &selection, None).unwrap();
        let format = VulkanMaterializationTarget::FORMAT;
        device.prewarm_compositor_pipelines(&[format]).unwrap();
        let warmed = device
            .pipeline(format, crate::graphics::render::PipelineKind::Image, BlendMode::Alpha)
            .unwrap();
        device.prewarm_compositor_pipelines(&[format]).unwrap();
        assert_eq!(
            warmed,
            device
                .pipeline(format, crate::graphics::render::PipelineKind::Image, BlendMode::Alpha)
                .unwrap()
        );
        let extent = SizeI {
            width: 16,
            height: 16,
        };
        let snapshots = [1, 2]
            .into_iter()
            .map(|id| {
                (
                    id,
                    MotionSnapshot {
                        target: VulkanMaterializationTarget::new_traced(
                            &device,
                            extent,
                            &mut |_| {},
                        )
                        .unwrap(),
                        extent,
                    },
                )
            })
            .collect();
        // No GPU commands are submitted: both output and crossfade scene construction
        // must accept the retained image resources during delta validation.
        let mut output = sample_scene(&device, &snapshots, extent, &[(1, 1.0)], false).unwrap();
        let epoch = output.epoch();
        update_sample_scene(
            &device,
            &mut output,
            &snapshots,
            SizeI {
                width: 24,
                height: 24,
            },
            &[(2, 0.5)],
            false,
        )
        .unwrap();
        assert_eq!(output.epoch(), epoch + 1);
        sample_scene(&device, &snapshots, extent, &[(1, 0.4), (2, 0.6)], true).unwrap();
    }
}

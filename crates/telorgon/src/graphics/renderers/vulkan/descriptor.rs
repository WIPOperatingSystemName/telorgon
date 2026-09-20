use crate::graphics::render::RenderResult;
use ash::vk;

use crate::graphics::renderers::vulkan::error::vk_error;

pub(crate) struct DescriptorLayouts {
    device: ash::Device,
    pub(crate) sets: [vk::DescriptorSetLayout; 4],
    pub(crate) pipeline: vk::PipelineLayout,
}

pub(crate) const PRIMITIVE_SET_COUNT: usize = 4;
pub(crate) const MAX_TEXTURE_SETS: usize = 128;
#[cfg(target_os = "linux")]
pub(crate) const MAX_COMPOSITE_SCENES: u32 = 256;
#[cfg(target_os = "linux")]
pub(crate) const MAX_COMPOSITE_TEXTURE_SETS: u32 = 2_048;

#[derive(Copy, Clone)]
pub(crate) struct FrameDescriptorSets {
    pub(crate) view: vk::DescriptorSet,
    pub(crate) scene: vk::DescriptorSet,
    pub(crate) primitives: [vk::DescriptorSet; PRIMITIVE_SET_COUNT],
    pub(crate) textures: [vk::DescriptorSet; MAX_TEXTURE_SETS],
}

#[cfg(target_os = "linux")]
pub(crate) struct CompositeDescriptorSets {
    pub(crate) view: vk::DescriptorSet,
    pub(crate) scene: vk::DescriptorSet,
    pub(crate) primitives: [vk::DescriptorSet; PRIMITIVE_SET_COUNT],
    pub(crate) textures: Vec<vk::DescriptorSet>,
}

impl DescriptorLayouts {
    pub(crate) fn new(device: &ash::Device) -> RenderResult<Self> {
        let mut created = Vec::with_capacity(4);
        let set0 = create_layout(
            device,
            &[vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)],
        )?;
        created.push(set0);
        let set1 = match create_layout(
            device,
            &[
                storage_binding(0, vk::ShaderStageFlags::VERTEX),
                storage_binding(1, vk::ShaderStageFlags::FRAGMENT),
                storage_binding(2, vk::ShaderStageFlags::VERTEX),
            ],
        ) {
            Ok(layout) => layout,
            Err(error) => return cleanup_layout_error(device, created, error),
        };
        created.push(set1);
        let set2 = match create_layout(
            device,
            &[
                storage_binding(
                    0,
                    vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                ),
                storage_binding(
                    1,
                    vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                ),
            ],
        ) {
            Ok(layout) => layout,
            Err(error) => return cleanup_layout_error(device, created, error),
        };
        created.push(set2);
        let set3 = match create_layout(
            device,
            &[
                sampled_binding(0, vk::ShaderStageFlags::FRAGMENT),
                sampled_binding(1, vk::ShaderStageFlags::FRAGMENT),
            ],
        ) {
            Ok(layout) => layout,
            Err(error) => return cleanup_layout_error(device, created, error),
        };
        created.push(set3);
        let sets = [set0, set1, set2, set3];
        let pipeline = unsafe {
            device.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default().set_layouts(&sets),
                None,
            )
        }
        .map_err(|result| vk_error("failed to create Vulkan pipeline layout", result));
        let pipeline = match pipeline {
            Ok(pipeline) => pipeline,
            Err(error) => return cleanup_layout_error(device, created, error),
        };
        Ok(Self {
            device: device.clone(),
            sets,
            pipeline,
        })
    }
}

fn cleanup_layout_error<T>(
    device: &ash::Device,
    layouts: Vec<vk::DescriptorSetLayout>,
    error: crate::graphics::render::RenderError,
) -> RenderResult<T> {
    unsafe {
        for layout in layouts {
            device.destroy_descriptor_set_layout(layout, None);
        }
    }
    Err(error)
}

impl Drop for DescriptorLayouts {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_pipeline_layout(self.pipeline, None);
            for layout in self.sets {
                self.device.destroy_descriptor_set_layout(layout, None);
            }
        }
    }
}

fn create_layout(
    device: &ash::Device,
    bindings: &[vk::DescriptorSetLayoutBinding<'_>],
) -> RenderResult<vk::DescriptorSetLayout> {
    unsafe {
        device.create_descriptor_set_layout(
            &vk::DescriptorSetLayoutCreateInfo::default().bindings(bindings),
            None,
        )
    }
    .map_err(|result| vk_error("failed to create Vulkan descriptor layout", result))
}

fn storage_binding(
    binding: u32,
    stages: vk::ShaderStageFlags,
) -> vk::DescriptorSetLayoutBinding<'static> {
    vk::DescriptorSetLayoutBinding::default()
        .binding(binding)
        .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
        .descriptor_count(1)
        .stage_flags(stages)
}

fn sampled_binding(
    binding: u32,
    stages: vk::ShaderStageFlags,
) -> vk::DescriptorSetLayoutBinding<'static> {
    vk::DescriptorSetLayoutBinding::default()
        .binding(binding)
        .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
        .descriptor_count(1)
        .stage_flags(stages)
}

pub(crate) fn allocate_frame_sets(
    device: &ash::Device,
    layouts: &DescriptorLayouts,
) -> RenderResult<(vk::DescriptorPool, FrameDescriptorSets)> {
    let pool_sizes = [
        vk::DescriptorPoolSize {
            ty: vk::DescriptorType::UNIFORM_BUFFER,
            descriptor_count: 1,
        },
        vk::DescriptorPoolSize {
            ty: vk::DescriptorType::STORAGE_BUFFER,
            descriptor_count: 3 + (PRIMITIVE_SET_COUNT as u32 * 2),
        },
        vk::DescriptorPoolSize {
            ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
            descriptor_count: MAX_TEXTURE_SETS as u32 * 2,
        },
    ];
    let pool = unsafe {
        device.create_descriptor_pool(
            &vk::DescriptorPoolCreateInfo::default()
                .max_sets((2 + PRIMITIVE_SET_COUNT + MAX_TEXTURE_SETS) as u32)
                .pool_sizes(&pool_sizes),
            None,
        )
    }
    .map_err(|result| vk_error("failed to create Vulkan frame descriptor pool", result))?;
    let sets = match unsafe {
        let mut set_layouts = Vec::with_capacity(2 + PRIMITIVE_SET_COUNT + MAX_TEXTURE_SETS);
        set_layouts.push(layouts.sets[0]);
        set_layouts.push(layouts.sets[1]);
        set_layouts.extend(std::iter::repeat_n(layouts.sets[2], PRIMITIVE_SET_COUNT));
        set_layouts.extend(std::iter::repeat_n(layouts.sets[3], MAX_TEXTURE_SETS));
        device.allocate_descriptor_sets(
            &vk::DescriptorSetAllocateInfo::default()
                .descriptor_pool(pool)
                .set_layouts(&set_layouts),
        )
    } {
        Ok(sets) => sets,
        Err(result) => {
            unsafe { device.destroy_descriptor_pool(pool, None) };
            return Err(vk_error(
                "failed to allocate Vulkan frame descriptor sets",
                result,
            ));
        }
    };
    if sets.len() != 2 + PRIMITIVE_SET_COUNT + MAX_TEXTURE_SETS {
        unsafe { device.destroy_descriptor_pool(pool, None) };
        return Err(crate::graphics::renderers::vulkan::error::internal(
            "Vulkan returned the wrong descriptor-set count",
        ));
    }
    let primitives = sets[2..2 + PRIMITIVE_SET_COUNT]
        .try_into()
        .expect("primitive descriptor-set count was checked");
    let textures = sets[2 + PRIMITIVE_SET_COUNT..]
        .try_into()
        .expect("texture descriptor-set count was checked");
    Ok((
        pool,
        FrameDescriptorSets {
            view: sets[0],
            scene: sets[1],
            primitives,
            textures,
        },
    ))
}

#[cfg(target_os = "linux")]
fn create_composite_descriptor_pool_raw(device: &ash::Device) -> RenderResult<vk::DescriptorPool> {
    let pool_sizes = [
        vk::DescriptorPoolSize {
            ty: vk::DescriptorType::UNIFORM_BUFFER,
            descriptor_count: MAX_COMPOSITE_SCENES,
        },
        vk::DescriptorPoolSize {
            ty: vk::DescriptorType::STORAGE_BUFFER,
            descriptor_count: MAX_COMPOSITE_SCENES * (3 + PRIMITIVE_SET_COUNT as u32 * 2),
        },
        vk::DescriptorPoolSize {
            ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
            descriptor_count: MAX_COMPOSITE_TEXTURE_SETS * 2,
        },
    ];
    unsafe {
        device.create_descriptor_pool(
            &vk::DescriptorPoolCreateInfo::default()
                .max_sets(
                    MAX_COMPOSITE_SCENES * (2 + PRIMITIVE_SET_COUNT as u32)
                        + MAX_COMPOSITE_TEXTURE_SETS,
                )
                .pool_sizes(&pool_sizes),
            None,
        )
    }
    .map_err(|result| vk_error("failed to create Vulkan composite descriptor pool", result))
}

#[cfg(target_os = "linux")]
pub(crate) fn allocate_composite_sets(
    device: &ash::Device,
    pool: vk::DescriptorPool,
    layouts: &DescriptorLayouts,
    texture_counts: &[usize],
) -> RenderResult<Vec<CompositeDescriptorSets>> {
    if texture_counts.len() > MAX_COMPOSITE_SCENES as usize
        || texture_counts.iter().any(|count| *count > MAX_TEXTURE_SETS)
        || texture_counts.iter().sum::<usize>() > MAX_COMPOSITE_TEXTURE_SETS as usize
    {
        return Err(crate::graphics::renderers::vulkan::error::internal(
            "Vulkan composite descriptor request exceeds its pool limits",
        ));
    }
    if texture_counts.is_empty() {
        return Ok(Vec::new());
    }
    let expected = texture_counts
        .iter()
        .map(|count| 2 + PRIMITIVE_SET_COUNT + *count)
        .sum::<usize>();
    let mut set_layouts = Vec::with_capacity(expected);
    for texture_count in texture_counts {
        set_layouts.push(layouts.sets[0]);
        set_layouts.push(layouts.sets[1]);
        set_layouts.extend(std::iter::repeat_n(layouts.sets[2], PRIMITIVE_SET_COUNT));
        set_layouts.extend(std::iter::repeat_n(layouts.sets[3], *texture_count));
    }
    let sets = unsafe {
        device.allocate_descriptor_sets(
            &vk::DescriptorSetAllocateInfo::default()
                .descriptor_pool(pool)
                .set_layouts(&set_layouts),
        )
    }
    .map_err(|result| {
        vk_error(
            "failed to allocate Vulkan composite descriptor sets",
            result,
        )
    })?;
    if sets.len() != expected {
        return Err(crate::graphics::renderers::vulkan::error::internal(
            "Vulkan returned the wrong composite descriptor-set count",
        ));
    }
    let mut sets = sets.into_iter();
    Ok(texture_counts
        .iter()
        .map(|texture_count| CompositeDescriptorSets {
            view: sets.next().expect("composite view set count was checked"),
            scene: sets.next().expect("composite scene set count was checked"),
            primitives: std::array::from_fn(|_| {
                sets.next()
                    .expect("composite primitive set count was checked")
            }),
            textures: sets.by_ref().take(*texture_count).collect(),
        })
        .collect())
}

#[cfg(target_os = "linux")]
pub(crate) struct CompositeDescriptorArena {
    device: ash::Device,
    pools: Vec<vk::DescriptorPool>,
    page: usize,
    placements: usize,
    textures: usize,
}

#[cfg(target_os = "linux")]
pub(crate) fn create_composite_descriptor_pool(
    device: &ash::Device,
) -> RenderResult<std::sync::Arc<std::sync::Mutex<CompositeDescriptorArena>>> {
    Ok(std::sync::Arc::new(std::sync::Mutex::new(
        CompositeDescriptorArena {
            device: device.clone(),
            pools: vec![create_composite_descriptor_pool_raw(device)?],
            page: 0,
            placements: 0,
            textures: 0,
        },
    )))
}

#[cfg(target_os = "linux")]
fn descriptor_page_fits(used: usize, textures: usize, added: usize, added_textures: usize) -> bool {
    used.saturating_add(added) <= MAX_COMPOSITE_SCENES as usize
        && textures.saturating_add(added_textures) <= MAX_COMPOSITE_TEXTURE_SETS as usize
}

#[cfg(target_os = "linux")]
impl CompositeDescriptorArena {
    pub(crate) fn reset(&mut self) -> RenderResult<()> {
        for pool in &self.pools {
            unsafe {
                self.device
                    .reset_descriptor_pool(*pool, vk::DescriptorPoolResetFlags::empty())
            }
            .map_err(|e| vk_error("failed to reset composite descriptor page", e))?;
        }
        self.page = 0;
        self.placements = 0;
        self.textures = 0;
        Ok(())
    }

    pub(crate) fn allocate(
        &mut self,
        layouts: &DescriptorLayouts,
        counts: &[usize],
    ) -> RenderResult<Vec<CompositeDescriptorSets>> {
        let textures = counts.iter().sum();
        if !descriptor_page_fits(0, 0, counts.len(), textures)
            || counts.iter().any(|n| *n > MAX_TEXTURE_SETS)
        {
            return Err(crate::graphics::renderers::vulkan::error::internal(
                "composite descriptor request exceeds page capacity",
            ));
        }
        if !descriptor_page_fits(self.placements, self.textures, counts.len(), textures) {
            if self.page + 1 >= 64 {
                return Err(crate::graphics::renderers::vulkan::error::internal(
                    "composite frame exceeds 64 descriptor pages",
                ));
            }
            let next_page = self.page + 1;
            if next_page == self.pools.len() {
                self.pools
                    .push(create_composite_descriptor_pool_raw(&self.device)?);
            }
            // Commit the cursor only after allocation succeeds, so an error remains retryable.
            self.page = next_page;
            self.placements = 0;
            self.textures = 0;
        }
        let sets = allocate_composite_sets(&self.device, self.pools[self.page], layouts, counts)?;
        self.placements += counts.len();
        self.textures += textures;
        Ok(sets)
    }
}

#[cfg(target_os = "linux")]
impl Drop for CompositeDescriptorArena {
    fn drop(&mut self) {
        for pool in &self.pools {
            unsafe {
                self.device.destroy_descriptor_pool(*pool, None);
            }
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod arena_tests {
    use super::*;
    #[test]
    fn cumulative_passes_roll_over_before_exhaustion() {
        assert!(descriptor_page_fits(129, 129, 127, 127));
        assert!(!descriptor_page_fits(129, 129, 129, 129));
        assert!(!descriptor_page_fits(1, 2048, 1, 1));
        assert!(descriptor_page_fits(0, 0, 129, 129));
    }
}

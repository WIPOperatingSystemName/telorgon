use std::marker::PhantomData;

use crate::core::SizeI;
use crate::render::{AlphaMode, ColorSpace, RenderResult, RenderTargetInfo};
use ash::vk;

use crate::renderer_vulkan::image::AllocatedImage;
use crate::renderer_vulkan::{VulkanDevice, error::unsupported};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) struct VulkanImageState {
    pub(crate) layout: vk::ImageLayout,
    pub(crate) stage: vk::PipelineStageFlags2,
    pub(crate) access: vk::AccessFlags2,
}

impl VulkanImageState {
    pub(crate) const UNDEFINED: Self = Self {
        layout: vk::ImageLayout::UNDEFINED,
        stage: vk::PipelineStageFlags2::NONE,
        access: vk::AccessFlags2::NONE,
    };

    pub(crate) const COLOR_ATTACHMENT: Self = Self {
        layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
        stage: vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
        access: vk::AccessFlags2::from_raw(
            vk::AccessFlags2::COLOR_ATTACHMENT_READ.as_raw()
                | vk::AccessFlags2::COLOR_ATTACHMENT_WRITE.as_raw(),
        ),
    };

    pub(crate) const SHADER_READ: Self = Self {
        layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        stage: vk::PipelineStageFlags2::FRAGMENT_SHADER,
        access: vk::AccessFlags2::SHADER_SAMPLED_READ,
    };
}

/// A compositor-owned texture populated by rendering one imported client image generation.
/// It becomes an ordinary retained sampled image after that submission completes.
pub(crate) struct VulkanMaterializationTarget {
    initialized: bool,
    image: std::sync::Arc<AllocatedImage>,
    info: RenderTargetInfo,
}

impl VulkanMaterializationTarget {
    // Preserve the source's 8-bit sRGB precision in retained storage. An 8-bit *linear*
    // intermediate loses dark shades after decoding (e.g. several near-black values collapse).
    // The attachment encodes linear shader output; its sampled view decodes for composition.
    pub(crate) const FORMAT: vk::Format = vk::Format::R8G8B8A8_SRGB;
    pub(crate) const COLOR_ENCODING: crate::render::ImageColorEncoding =
        crate::render::ImageColorEncoding::Srgb;
    pub(crate) const COLOR_SPACE: ColorSpace = ColorSpace::Srgb;

    #[cfg(test)]
    pub(crate) fn new(device: &VulkanDevice, extent: SizeI) -> RenderResult<Self> {
        Self::new_traced(device, extent, &mut |_| {})
    }

    pub(crate) fn new_traced(
        device: &VulkanDevice,
        extent: SizeI,
        phase: &mut dyn FnMut(&'static str),
    ) -> RenderResult<Self> {
        phase("dmabuf_alloc_format");
        if extent.width <= 0 || extent.height <= 0 {
            return Err(unsupported(
                "Vulkan materialization target extent must be nonzero",
            ));
        }
        let features = unsafe {
            device
                .inner
                .instance
                .inner
                .raw
                .get_physical_device_format_properties(device.inner.physical_device, Self::FORMAT)
        }
        .optimal_tiling_features;
        let required = vk::FormatFeatureFlags::SAMPLED_IMAGE
            | vk::FormatFeatureFlags::SAMPLED_IMAGE_FILTER_LINEAR
            | vk::FormatFeatureFlags::COLOR_ATTACHMENT
            | vk::FormatFeatureFlags::COLOR_ATTACHMENT_BLEND
            | vk::FormatFeatureFlags::TRANSFER_SRC;
        if !features.contains(required) {
            return Err(unsupported(
                "Vulkan DMA-BUF materialization requires a filterable, blendable RGBA8 sRGB target",
            ));
        }
        let image = std::sync::Arc::new(AllocatedImage::new_color_target_traced(
            device.inner.clone(),
            vk::Extent2D {
                width: extent.width as u32,
                height: extent.height as u32,
            },
            Self::FORMAT,
            "Telorgon DMA-BUF materialization target",
            phase,
        )?);
        Ok(Self {
            initialized: false,
            image,
            info: RenderTargetInfo {
                color_space: Self::COLOR_SPACE,
                alpha_mode: AlphaMode::Premultiplied,
                ..RenderTargetInfo::full(extent)
            },
        })
    }

    pub(crate) fn mark_initialized(&mut self) {
        self.initialized = true;
    }
    pub(crate) fn initialized(&self) -> bool {
        self.initialized
    }
    pub(crate) fn can_recycle(&self) -> bool {
        std::sync::Arc::strong_count(&self.image) == 1
    }
    pub(crate) fn allocated_bytes(&self) -> u64 {
        self.image.allocated_bytes()
    }

    pub(crate) fn target(&self) -> VulkanTarget<'_> {
        VulkanTarget {
            device_id: self.image.device_id(),
            image: self.image.raw(),
            view: self.image.view(),
            format: self.image.format,
            extent: self.image.extent,
            info: self.info,
            initial_state: if self.initialized {
                VulkanImageState::SHADER_READ
            } else {
                VulkanImageState::UNDEFINED
            },
            final_state: VulkanImageState::SHADER_READ,
            initial_queue_family: vk::QUEUE_FAMILY_IGNORED,
            final_queue_family: vk::QUEUE_FAMILY_IGNORED,
            _borrow: PhantomData,
        }
    }

    pub(crate) fn image(&self) -> std::sync::Arc<AllocatedImage> {
        std::sync::Arc::clone(&self.image)
    }

    pub(crate) fn extent(&self) -> SizeI {
        self.info.extent
    }
}

pub struct OffscreenVulkanTarget {
    image: AllocatedImage,
    info: RenderTargetInfo,
}

/// Retained SDR capture destination, independently owned from KMS scanout.
pub(crate) struct VulkanCaptureTarget {
    image: std::sync::Arc<AllocatedImage>,
    info: RenderTargetInfo,
}

impl VulkanCaptureTarget {
    pub(crate) fn new(device: &VulkanDevice, extent: SizeI) -> RenderResult<Self> {
        // Reuse the existing sRGB attachment/transfer format qualification and allocator.
        let source = VulkanMaterializationTarget::new_traced(device, extent, &mut |_| {})?;
        Ok(Self {
            image: source.image,
            info: source.info,
        })
    }

    pub(crate) fn image(&self) -> std::sync::Arc<AllocatedImage> {
        std::sync::Arc::clone(&self.image)
    }

    pub(crate) fn target(&self) -> VulkanTarget<'_> {
        VulkanTarget {
            device_id: self.image.device_id(),
            image: self.image.raw(),
            view: self.image.view(),
            format: self.image.format,
            extent: self.image.extent,
            info: self.info,
            // Every capture is complete, so the previous target contents are discarded.
            initial_state: VulkanImageState::UNDEFINED,
            final_state: VulkanImageState::COLOR_ATTACHMENT,
            initial_queue_family: vk::QUEUE_FAMILY_IGNORED,
            final_queue_family: vk::QUEUE_FAMILY_IGNORED,
            _borrow: PhantomData,
        }
    }
}

impl OffscreenVulkanTarget {
    pub fn new(device: &VulkanDevice, extent: SizeI) -> RenderResult<Self> {
        if extent.width <= 0 || extent.height <= 0 {
            return Err(unsupported(
                "offscreen Vulkan target extent must be nonzero",
            ));
        }
        let image = AllocatedImage::new_color_target(
            device.inner.clone(),
            vk::Extent2D {
                width: extent.width as u32,
                height: extent.height as u32,
            },
            vk::Format::R8G8B8A8_UNORM,
            "Telorgon offscreen target",
        )?;
        Ok(Self {
            image,
            info: RenderTargetInfo {
                color_space: ColorSpace::Linear,
                alpha_mode: AlphaMode::Premultiplied,
                ..RenderTargetInfo::full(extent)
            },
        })
    }

    pub fn target(&self) -> VulkanTarget<'_> {
        VulkanTarget {
            device_id: self.image_device_id(),
            image: self.image.raw(),
            view: self.image.view(),
            format: self.image.format,
            extent: self.image.extent,
            info: self.info,
            initial_state: VulkanImageState::UNDEFINED,
            final_state: VulkanImageState::COLOR_ATTACHMENT,
            initial_queue_family: vk::QUEUE_FAMILY_IGNORED,
            final_queue_family: vk::QUEUE_FAMILY_IGNORED,
            _borrow: PhantomData,
        }
    }

    fn image_device_id(&self) -> u64 {
        self.image.device_id()
    }
}

#[derive(Copy, Clone)]
pub struct VulkanTarget<'frame> {
    pub(crate) device_id: u64,
    pub(crate) image: vk::Image,
    pub(crate) view: vk::ImageView,
    pub(crate) format: vk::Format,
    pub(crate) extent: vk::Extent2D,
    pub(crate) info: RenderTargetInfo,
    pub(crate) initial_state: VulkanImageState,
    pub(crate) final_state: VulkanImageState,
    pub(crate) initial_queue_family: u32,
    pub(crate) final_queue_family: u32,
    pub(crate) _borrow: PhantomData<&'frame mut vk::Image>,
}

impl VulkanTarget<'_> {
    pub fn info(&self) -> RenderTargetInfo {
        self.info
    }
}

#[cfg(all(test, target_os = "linux"))]
mod materialization_tests {
    use super::*;
    use crate::core::{ColorRgba8, RectI};
    use crate::render::{ReadbackFormat, ReadbackRequest, RenderRequest, TargetLoad, TargetStore};
    use crate::renderer_vulkan::{DeviceSelection, VulkanConfig, VulkanInstance};

    #[test]
    #[ignore = "requires TELORGON_TEST_MODE=developer-hardware and a non-CPU Vulkan adapter"]
    fn retained_srgb_target_preserves_undamaged_pixels_across_submissions() {
        assert_eq!(
            std::env::var("TELORGON_TEST_MODE").as_deref(),
            Ok("developer-hardware")
        );
        let config = VulkanConfig {
            enable_validation: true,
            frames_in_flight: 3,
            ..Default::default()
        };
        let instance = VulkanInstance::load(&config, &[]).unwrap();
        let selection =
            DeviceSelection::best(&instance.adapters().unwrap()).expect("hardware adapter");
        let device = VulkanDevice::create_owned(instance, &config, &selection, None).unwrap();
        let mut storage = VulkanMaterializationTarget::new(
            &device,
            SizeI {
                width: 8,
                height: 8,
            },
        )
        .unwrap();
        assert!(storage.can_recycle());
        let native_image = storage.image().raw();
        let mut first = device.begin_owned_frame().unwrap();
        {
            let mut context = first.context_mut();
            context.core.images.push(storage.image());
            device
                .render_composite(
                    &mut [],
                    &[],
                    &mut context,
                    &storage.target(),
                    &RenderRequest {
                        force: true,
                        load: TargetLoad::Clear(ColorRgba8::rgba(128, 128, 128, 255)),
                        store: TargetStore::Store,
                        region: None,
                    },
                )
                .unwrap();
        }
        let mut first_receipt = first.finish().unwrap().submit().unwrap();
        storage.mark_initialized();
        assert!(
            !storage.can_recycle(),
            "submitted writes pin the image even without a sampled scene"
        );
        assert_eq!(
            storage.target().initial_state,
            VulkanImageState::SHADER_READ
        );
        let mut second = device.begin_owned_frame().unwrap();
        let pending = {
            let mut context = second.context_mut();
            context.core.images.push(storage.image());
            device
                .render_composite(
                    &mut [],
                    &[],
                    &mut context,
                    &storage.target(),
                    &RenderRequest {
                        force: true,
                        load: TargetLoad::Clear(ColorRgba8::rgba(0, 0, 0, 0)),
                        store: TargetStore::Store,
                        region: Some(RectI {
                            x: 2,
                            y: 2,
                            width: 3,
                            height: 3,
                        }),
                    },
                )
                .unwrap();
            context
                .record_readback(
                    &storage.target(),
                    &ReadbackRequest {
                        region: RectI {
                            x: 0,
                            y: 0,
                            width: 8,
                            height: 8,
                        },
                        format: ReadbackFormat::Rgba8,
                    },
                )
                .unwrap()
        };
        assert_eq!(
            storage.image().raw(),
            native_image,
            "second update must reuse storage"
        );
        let pixels = pending
            .bind_to_submission(second.finish().unwrap().submit().unwrap())
            .unwrap()
            .wait(std::time::Duration::from_secs(10))
            .unwrap()
            .pixels;
        first_receipt
            .wait(std::time::Duration::from_secs(10))
            .unwrap();
        for y in 0..8 {
            for x in 0..8 {
                let pixel = &pixels[(y * 8 + x) * 4..(y * 8 + x) * 4 + 4];
                if (2..5).contains(&x) && (2..5).contains(&y) {
                    assert_eq!(pixel, &[0, 0, 0, 0]);
                } else {
                    assert!(pixel[..3].iter().all(|v| (i32::from(*v) - 128).abs() <= 1));
                    assert_eq!(pixel[3], 255);
                }
            }
        }
        assert_eq!(
            device.diagnostics().error_count(),
            0,
            "{:?}",
            device.diagnostics().messages()
        );
    }
}

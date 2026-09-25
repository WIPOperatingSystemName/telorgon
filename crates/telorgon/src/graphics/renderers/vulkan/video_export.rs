//! Scene rendering into completed, independently owned DMA-BUF video allocations.
use super::{
    target::VulkanImageState,
    video_transfer::{ExportImage, encoding, error, pixel},
    *,
};
use crate::{
    foundation::SizeI,
    graphics::render::{
        AlphaMode, ColorSpace, ImageColorEncoding, RenderBackend, RenderErrorKind, RenderRequest,
        RenderTargetInfo, TargetLoad, TargetStore,
    },
    integrations::pipewire::MediaError,
    media::video::{
        FrameMetadata, GpuVideoFrame, MemoryBudget, PixelFormat, VideoDmaBufPlane, VideoFormat,
        gpu::GpuReservation,
    },
};
use ash::vk;
use std::{marker::PhantomData, sync::Arc, time::Duration};

/// Renders complete Telorgon scenes into immutable GPU frames without CPU pixel readback.
/// The selected tuple must support blending, rendering, DMA-BUF export and sampled import.
/// Supports full-range BT.709 RGBA/BGRA with linear or sRGB transfer and RGBA float16
/// with linear transfer. Float16 preserves extended values but does not transform primaries
/// or assign a mastering luminance. Output alpha
/// is premultiplied. Held frames and renderer leases charge this owner's memory budget even
/// after it is dropped. Each render allocates a fresh target; exhausted budgets reject work.
/// At most 16 distinct exported allocations may be retained from one renderer.
///
/// `render` waits for GPU completion and belongs on a graphics worker. `record` and
/// `record_composite` add commands to an existing owned frame without submitting or waiting;
/// their pending result becomes readable only after matching submission completion. Allocation
/// and command recording are control work, never realtime-safe. Only owned Vulkan devices are
/// accepted. Native PipeWire production is a separate stream/transport operation.
pub struct VulkanVideoRenderer {
    device: VulkanDevice,
    format: VideoFormat,
    cap: VulkanDmaBufFormatCapability,
    budget: Arc<MemoryBudget>,
    limit: usize,
}
impl VulkanVideoRenderer {
    pub fn new(
        device: VulkanDevice,
        mut format: VideoFormat,
        memory_budget: usize,
    ) -> Result<Self, MediaError> {
        format.validate()?;
        format.rate = format.rate.reduced();
        let color = encoding(format)?;
        if device.hosted.is_some()
            || !matches!(
                format.pixel,
                PixelFormat::Rgba8 | PixelFormat::Bgra8 | PixelFormat::RgbaF16
            )
        {
            return Err(MediaError::Unsupported(
                "video rendering requires an owned device and RGBA/BGRA/float16",
            ));
        }
        if memory_budget == 0 || memory_budget > 512 * 1024 * 1024 {
            return Err(MediaError::InvalidArgument("GPU video render budget"));
        }
        let usage = vk::ImageUsageFlags::SAMPLED
            | vk::ImageUsageFlags::COLOR_ATTACHMENT
            | vk::ImageUsageFlags::TRANSFER_SRC;
        let required = vk::FormatFeatureFlags::COLOR_ATTACHMENT
            | vk::FormatFeatureFlags::COLOR_ATTACHMENT_BLEND;
        let cap = device
            .dma_buf_import_capabilities(usage)
            .map_err(error)?
            .into_iter()
            .filter(|c| {
                c.exportable()
                    && c.plane_count == 1
                    && pixel(c.drm_fourcc) == Some(format.pixel)
                    && c.color_encoding == color
                    && c.tiling_features.contains(required)
                    && c.max_extent.width >= format.width
                    && c.max_extent.height >= format.height
            })
            .min_by_key(|c| (c.drm_modifier != 0, c.drm_modifier))
            .ok_or(MediaError::Unsupported(
                "blendable DMA-BUF video render target",
            ))?;
        Ok(Self {
            device,
            format,
            cap,
            budget: MemoryBudget::new(memory_budget),
            limit: memory_budget,
        })
    }
    pub fn format(&self) -> VideoFormat {
        self.format
    }
    // Only assembled before the first frame; replacing a budget with outstanding frames
    // would split accounting across owners.
    pub(crate) fn with_parent_budget(mut self, parent: Arc<MemoryBudget>) -> Self {
        self.budget = MemoryBudget::child(parent, self.limit);
        self
    }
    pub fn modifier(&self) -> u64 {
        self.cap.drm_modifier
    }
    pub fn allocated_frame_bytes(&self) -> usize {
        self.budget.used()
    }

    /// Render the scene's complete extent, clearing with its background color. Metadata is
    /// attached without altering pixels; timestamps use the caller's CLOCK_MONOTONIC clock.
    /// The scene must belong to this device. No partial render or uninitialized pixel export.
    pub fn render(
        &mut self,
        scene: &mut VulkanScene,
        metadata: FrameMetadata,
    ) -> Result<GpuVideoFrame, MediaError> {
        let device = self.device.clone();
        let mut recording = device.begin_owned_frame().map_err(error)?;
        let mut pending = self.record(scene, &mut recording.context_mut(), metadata)?;
        let mut receipt = recording.finish().map_err(error)?.submit().map_err(error)?;
        // A transient wait error cannot justify releasing in-flight target ownership.
        loop {
            match receipt.wait(Duration::from_nanos(u64::MAX)) {
                Ok(()) => break,
                Err(e) if e.kind() == RenderErrorKind::DeviceLost => return Err(error(e)),
                Err(_) => std::thread::yield_now(),
            }
        }
        pending
            .try_complete(&mut receipt)?
            .ok_or(MediaError::NotReady)
    }

    /// Record a complete scene without submitting or waiting. Dropping the returned pending
    /// result cancels delivery, while the frame/receipt still pins its target through completion.
    pub fn record(
        &mut self,
        scene: &mut VulkanScene,
        frame: &mut VulkanFrameContext<'_>,
        metadata: FrameMetadata,
    ) -> Result<VulkanPendingVideoFrame, MediaError> {
        let pending = self.prepare(frame, metadata)?;
        let background = scene.background();
        self.device
            .render(
                scene,
                &mut VulkanFrameContext {
                    core: &mut *frame.core,
                },
                &pending.target.target(self.format, self.cap),
                &RenderRequest {
                    force: true,
                    load: TargetLoad::Clear(background),
                    store: TargetStore::Store,
                    region: None,
                },
            )
            .map_err(error)?;
        Ok(pending)
    }

    /// Render exactly the supplied composition into a new exportable target, cleared opaque
    /// black. The host supplies authorized scenes/placements and cursor policy; this graphics
    /// operation grants no capture permission. No CPU pixel readback or internal submission.
    pub fn record_composite(
        &mut self,
        scenes: &mut [VulkanCompositeScene<'_>],
        placements: &[VulkanCompositePlacement],
        frame: &mut VulkanFrameContext<'_>,
        metadata: FrameMetadata,
    ) -> Result<VulkanPendingVideoFrame, MediaError> {
        let pending = self.prepare(frame, metadata)?;
        self.device
            .render_composite(
                scenes,
                placements,
                &mut VulkanFrameContext {
                    core: &mut *frame.core,
                },
                &pending.target.target(self.format, self.cap),
                &RenderRequest {
                    force: true,
                    load: TargetLoad::Clear(crate::foundation::ColorRgba8::rgba(0, 0, 0, 255)),
                    store: TargetStore::Store,
                    region: None,
                },
            )
            .map_err(error)?;
        Ok(pending)
    }

    fn prepare(
        &self,
        frame: &mut VulkanFrameContext<'_>,
        metadata: FrameMetadata,
    ) -> Result<VulkanPendingVideoFrame, MediaError> {
        if frame.core.device.inner.id != self.device.inner.id || frame.core.device.hosted.is_some()
        {
            return Err(MediaError::InvalidArgument("video export frame device"));
        }
        metadata.validate(self.format)?;
        if self.budget.gpu_frames() >= 16 {
            return Err(MediaError::ResourceLimit("held GPU render frame count"));
        }
        let available = self
            .budget
            .available()
            .saturating_sub(metadata.allocation_bytes());
        let mut reservation =
            GpuReservation::new(self.budget.clone(), metadata.allocation_bytes(), 16)?;
        let allocation =
            ExportImage::new_accounted(&self.device, self.cap, self.format, available, |bytes| {
                Arc::get_mut(&mut reservation)
                    .expect("unpublished GPU reservation")
                    .resize(
                        bytes
                            .checked_add(metadata.allocation_bytes())
                            .ok_or(MediaError::ResourceLimit("GPU video metadata"))?,
                    )
            })?;
        let plane = allocation.export()?;
        if plane.allocation_size > available as u64 {
            return Err(MediaError::ResourceLimit("GPU video render allocation"));
        }
        Arc::get_mut(&mut reservation)
            .expect("unpublished GPU reservation")
            .resize(plane.allocation_size as usize + metadata.allocation_bytes())?;
        let view = RenderView::new(&self.device, &allocation, self.cap.format)?;
        let target = Arc::new(ExportTarget {
            view,
            allocation,
            reservation,
        });
        // Pin before recording: even a partial-recording error must not free a target that
        // the caller might still submit. This owner holds DeviceInner, never FrameSlots.
        frame.core.resources.push(target.clone());
        Ok(VulkanPendingVideoFrame {
            target,
            plane: Some(plane),
            metadata: Some(metadata),
            format: self.format,
            modifier: self.cap.drm_modifier,
            device_id: self.device.inner.id,
            frame_id: frame.core.frame_id,
        })
    }
}

/// A recorded, not yet readable video export. Send it with the matching submission receipt
/// to a completion worker. Drop cancels delivery without waiting; submission ownership keeps
/// the target and its budget reservation alive until completion (including deferred retirement).
/// After successful completion this object is consumed logically: further calls return NotReady.
#[must_use = "complete the export with its submission receipt, or drop it to cancel delivery"]
pub struct VulkanPendingVideoFrame {
    plane: Option<VideoDmaBufPlane>,
    metadata: Option<FrameMetadata>,
    // Release the budget only after this owner's FD and metadata storage are gone.
    target: Arc<ExportTarget>,
    format: VideoFormat,
    modifier: u64,
    device_id: u64,
    frame_id: u64,
}
impl VulkanPendingVideoFrame {
    /// Poll only; never waits. An unrelated receipt is rejected even if it completed later.
    /// A false/None result retains all state for retry. Pixels become immutable only here.
    pub fn try_complete(
        &mut self,
        receipt: &mut SubmissionReceipt,
    ) -> Result<Option<GpuVideoFrame>, MediaError> {
        let completion = receipt.completion();
        if completion.device_id() != self.device_id || completion.frame_id() != self.frame_id {
            return Err(MediaError::InvalidArgument(
                "video export submission mismatch",
            ));
        }
        if self.plane.is_none() {
            return Err(MediaError::NotReady);
        }
        if !receipt.poll().map_err(error)? {
            return Ok(None);
        }
        // SAFETY: matching frame completed all writes and the foreign-family release. This
        // target is single-use and has no public write handle. Every later owner sees read-only
        // pixels; retained Vulkan resources do not imply another writer.
        let mut result = unsafe {
            GpuVideoFrame::from_completed_dma_buf(
                self.format,
                self.modifier,
                vec![self.plane.take().unwrap()],
                self.metadata.take().unwrap(),
            )?
        };
        result.account_reserved(self.target.reservation.clone(), 0)?;
        Ok(Some(result))
    }
}

struct ExportTarget {
    // Drop the view before its image, and the budget after the native allocation.
    view: RenderView,
    allocation: ExportImage,
    reservation: Arc<GpuReservation>,
}
impl ExportTarget {
    fn target(&self, format: VideoFormat, cap: VulkanDmaBufFormatCapability) -> VulkanTarget<'_> {
        let extent = SizeI {
            width: format.width as i32,
            height: format.height as i32,
        };
        VulkanTarget {
            device_id: self.view.device.id,
            image: self.allocation.image,
            view: self.view.raw,
            format: cap.format,
            extent: vk::Extent2D {
                width: format.width,
                height: format.height,
            },
            info: RenderTargetInfo {
                color_space: match cap.color_encoding {
                    ImageColorEncoding::Srgb => ColorSpace::Srgb,
                    ImageColorEncoding::Linear => ColorSpace::Linear,
                },
                alpha_mode: AlphaMode::Premultiplied,
                ..RenderTargetInfo::full(extent)
            },
            initial_state: VulkanImageState::UNDEFINED,
            final_state: VulkanImageState {
                layout: vk::ImageLayout::GENERAL,
                stage: vk::PipelineStageFlags2::NONE,
                access: vk::AccessFlags2::NONE,
            },
            initial_queue_family: vk::QUEUE_FAMILY_IGNORED,
            final_queue_family: vk::QUEUE_FAMILY_FOREIGN_EXT,
            _borrow: PhantomData,
        }
    }
}
struct RenderView {
    device: Arc<super::device::DeviceInner>,
    raw: vk::ImageView,
}
impl RenderView {
    fn new(
        device: &VulkanDevice,
        image: &ExportImage,
        format: vk::Format,
    ) -> Result<Self, MediaError> {
        let info = vk::ImageViewCreateInfo::default()
            .image(image.image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(format)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            });
        let raw = unsafe { device.inner.raw.create_image_view(&info, None) }.map_err(error)?;
        Ok(Self {
            device: device.inner.clone(),
            raw,
        })
    }
}
impl Drop for RenderView {
    fn drop(&mut self) {
        unsafe { self.device.raw.destroy_image_view(self.raw, None) };
    }
}

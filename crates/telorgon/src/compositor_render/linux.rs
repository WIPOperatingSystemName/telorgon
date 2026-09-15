use std::os::fd::OwnedFd;
use std::sync::Arc;

use crate::compositor_wayland::{
    BufferTransform, DmaBufFormat, DmaBufImage, ShmFormat, ShmImage, ShmImageRegion,
    ViewportSource, ViewportState, WaylandBufferId,
};
use crate::core::{RectI, SizeI};
use crate::render::{
    ImageAlphaMode, ImageColorEncoding, ImageId, ImagePixelFormat, ImageResource,
    ImageResourceUpdate,
};
use crate::renderer_vulkan::{
    HostedImageUse, VulkanDevice, VulkanDmaBufImport, VulkanDmaBufPlane, VulkanExternalImageLease,
    VulkanExternalImageOrigin, VulkanScene,
};
use ash::vk;

use crate::compositor_render::CompositorRenderError;

pub fn imported_image_id(buffer: WaylandBufferId) -> ImageId {
    ImageId(buffer.get())
}

/// DMA-BUF materialization uses a stable per-scene slot distinct from the uploaded-image slot.
/// Shell surface scenes are independent, so this ID does not need to encode the wl_buffer ID.
pub fn dma_buf_image_id() -> ImageId {
    ImageId(u32::MAX)
}

pub fn shm_image_resource(
    buffer: WaylandBufferId,
    content_version: u64,
    image: ShmImage,
) -> Result<ImageResource, CompositorRenderError> {
    if content_version == 0 {
        return Err(CompositorRenderError::new(
            "SHM content version must be nonzero",
        ));
    }
    let descriptor = image.descriptor;
    let width = usize::try_from(descriptor.size.width)
        .map_err(|_| CompositorRenderError::new("invalid SHM width"))?;
    let height = usize::try_from(descriptor.size.height)
        .map_err(|_| CompositorRenderError::new("invalid SHM height"))?;
    let source_pixel_bytes = descriptor
        .format
        .bytes_per_pixel()
        .ok_or_else(|| CompositorRenderError::new("unsupported SHM pixel format"))?
        as usize;
    let row_bytes = width
        .checked_mul(source_pixel_bytes)
        .ok_or_else(|| CompositorRenderError::new("SHM row size overflow"))?;
    let stride = descriptor.stride as usize;
    if stride < row_bytes
        || image.pixels.len()
            < stride
                .checked_mul(height)
                .ok_or_else(|| CompositorRenderError::new("SHM extent overflow"))?
    {
        return Err(CompositorRenderError::new(
            "SHM bytes do not cover the declared image",
        ));
    }

    let (pixel_format, pixels) =
        convert_shm_rows(descriptor.format, width, height, stride, image.pixels)?;
    Ok(ImageResource {
        image: imported_image_id(buffer),
        content_version,
        extent: descriptor.size,
        color_encoding: ImageColorEncoding::Srgb,
        alpha_mode: shm_alpha_mode(descriptor.format),
        pixel_format,
        pixels: Arc::from(pixels),
    })
}

pub fn shm_image_update(
    buffer: WaylandBufferId,
    content_version: u64,
    image: ShmImageRegion,
) -> Result<ImageResourceUpdate, CompositorRenderError> {
    if content_version == 0 {
        return Err(CompositorRenderError::new(
            "SHM content version must be nonzero",
        ));
    }
    let width = image.rect.width as usize;
    let height = image.rect.height as usize;
    let (pixel_format, pixels) = convert_shm_rows(
        image.descriptor.format,
        width,
        height,
        image.row_bytes,
        image.pixels,
    )?;
    Ok(ImageResourceUpdate {
        image: imported_image_id(buffer),
        content_version,
        extent: image.descriptor.size,
        rect: image.rect,
        row_bytes: width * 4,
        color_encoding: ImageColorEncoding::Srgb,
        alpha_mode: shm_alpha_mode(image.descriptor.format),
        pixel_format,
        pixels: Arc::from(pixels),
    })
}

pub fn shm_image_metadata(
    format: ShmFormat,
) -> Result<(ImagePixelFormat, ImageAlphaMode), CompositorRenderError> {
    if format.bytes_per_pixel().is_none() {
        return Err(CompositorRenderError::new("unsupported SHM pixel format"));
    }
    let pixel_format = match format {
        ShmFormat::Argb8888 | ShmFormat::Xrgb8888 => ImagePixelFormat::Bgra8,
        ShmFormat::Abgr8888 | ShmFormat::Xbgr8888 | ShmFormat::Rgb565 => ImagePixelFormat::Rgba8,
        ShmFormat::Other(_) => unreachable!("bytes-per-pixel rejected unknown format"),
    };
    Ok((pixel_format, shm_alpha_mode(format)))
}

fn shm_alpha_mode(format: ShmFormat) -> ImageAlphaMode {
    match format {
        ShmFormat::Argb8888 | ShmFormat::Abgr8888 => ImageAlphaMode::Premultiplied,
        _ => ImageAlphaMode::Opaque,
    }
}

fn convert_shm_rows(
    format: ShmFormat,
    width: usize,
    height: usize,
    source_stride: usize,
    source_pixels: Vec<u8>,
) -> Result<(ImagePixelFormat, Vec<u8>), CompositorRenderError> {
    let source_pixel_bytes = format
        .bytes_per_pixel()
        .ok_or_else(|| CompositorRenderError::new("unsupported SHM pixel format"))?
        as usize;
    let source_row_bytes = width
        .checked_mul(source_pixel_bytes)
        .ok_or_else(|| CompositorRenderError::new("SHM row size overflow"))?;
    let output_row_bytes = width
        .checked_mul(4)
        .ok_or_else(|| CompositorRenderError::new("SHM output row size overflow"))?;
    let output_len = output_row_bytes
        .checked_mul(height)
        .ok_or_else(|| CompositorRenderError::new("SHM image size overflow"))?;
    if source_stride < source_row_bytes
        || source_pixels.len() < source_stride.saturating_mul(height)
    {
        return Err(CompositorRenderError::new(
            "SHM bytes do not cover the declared rows",
        ));
    }
    let (pixel_format, _) = shm_image_metadata(format)?;
    if format != ShmFormat::Rgb565 && source_stride == output_row_bytes {
        return Ok((pixel_format, source_pixels));
    }
    let mut output = vec![0_u8; output_len];
    for row in 0..height {
        let source = &source_pixels[row * source_stride..row * source_stride + source_row_bytes];
        let target = &mut output[row * output_row_bytes..(row + 1) * output_row_bytes];
        if format == ShmFormat::Rgb565 {
            for (source, target) in source.chunks_exact(2).zip(target.chunks_exact_mut(4)) {
                let value = u16::from_le_bytes([source[0], source[1]]);
                let red = ((value >> 11) & 0x1f) as u8;
                let green = ((value >> 5) & 0x3f) as u8;
                let blue = (value & 0x1f) as u8;
                target.copy_from_slice(&[
                    (red << 3) | (red >> 2),
                    (green << 2) | (green >> 4),
                    (blue << 3) | (blue >> 2),
                    255,
                ]);
            }
        } else {
            target.copy_from_slice(source);
        }
    }
    Ok((pixel_format, output))
}

/// Applies `wl_surface` buffer transform/scale and the committed viewporter state to a retained
/// four-channel image. Sampling is deterministic nearest-neighbor so the software reference and a future
/// Vulkan compositor can share the exact surface geometry contract.
pub fn transform_surface_image(
    image: ImageResource,
    buffer_scale: i32,
    transform: BufferTransform,
    viewport: Option<ViewportState>,
) -> Result<ImageResource, CompositorRenderError> {
    transform_surface_image_at_scale(
        image,
        buffer_scale,
        transform,
        viewport,
        crate::platform::ScaleFactor::default(),
    )
    .map(|(image, _)| image)
}

/// Materialize directly from original buffer pixels at output density. The returned logical
/// extent remains independent of the image allocation, preventing a downsample/upsample cycle.
pub fn transform_surface_image_at_scale(
    image: ImageResource,
    buffer_scale: i32,
    transform: BufferTransform,
    viewport: Option<ViewportState>,
    output_scale: crate::platform::ScaleFactor,
) -> Result<(ImageResource, SizeI), CompositorRenderError> {
    if buffer_scale <= 0 {
        return Err(CompositorRenderError::new("buffer scale must be positive"));
    }
    if buffer_scale == 1 && transform == BufferTransform::Normal && viewport.is_none() {
        let logical = image.extent;
        return Ok((image, logical));
    }
    let input_width = usize::try_from(image.extent.width)
        .map_err(|_| CompositorRenderError::new("invalid image width"))?;
    let input_height = usize::try_from(image.extent.height)
        .map_err(|_| CompositorRenderError::new("invalid image height"))?;
    let expected = input_width
        .checked_mul(input_height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| CompositorRenderError::new("surface image size overflow"))?;
    if input_width == 0 || input_height == 0 || image.pixels.len() < expected {
        return Err(CompositorRenderError::new(
            "surface image does not cover its extent",
        ));
    }
    let swap_axes = matches!(
        transform,
        BufferTransform::Rotate90
            | BufferTransform::Rotate270
            | BufferTransform::Flipped90
            | BufferTransform::Flipped270
    );
    let transformed_width = if swap_axes { input_height } else { input_width };
    let transformed_height = if swap_axes { input_width } else { input_height };
    let scale = buffer_scale as usize;
    if transformed_width % scale != 0 || transformed_height % scale != 0 {
        return Err(CompositorRenderError::new(
            "transformed buffer extent is not divisible by its scale",
        ));
    }
    let logical_width = transformed_width / scale;
    let logical_height = transformed_height / scale;
    let viewport = viewport.unwrap_or_default();
    let source = viewport.source.unwrap_or(ViewportSource {
        x: 0.0,
        y: 0.0,
        width: logical_width as f64,
        height: logical_height as f64,
    });
    if !source.x.is_finite()
        || !source.y.is_finite()
        || !source.width.is_finite()
        || !source.height.is_finite()
        || source.x < 0.0
        || source.y < 0.0
        || source.width <= 0.0
        || source.height <= 0.0
        || source.x + source.width > logical_width as f64
        || source.y + source.height > logical_height as f64
    {
        return Err(CompositorRenderError::new(
            "viewport source lies outside the logical image",
        ));
    }
    let destination = viewport.destination.unwrap_or(SizeI {
        width: source.width as i32,
        height: source.height as i32,
    });
    if destination.width <= 0 || destination.height <= 0 {
        return Err(CompositorRenderError::new(
            "viewport destination must be positive",
        ));
    }
    let raster = SizeI {
        width: (destination.width as f32 * output_scale.get())
            .round()
            .max(1.0) as i32,
        height: (destination.height as f32 * output_scale.get())
            .round()
            .max(1.0) as i32,
    };
    let destination_width = raster.width as usize;
    let destination_height = raster.height as usize;
    let destination_len = destination_width
        .checked_mul(destination_height)
        .and_then(|pixels| pixels.checked_mul(4))
        .filter(|bytes| *bytes <= 512 * 1024 * 1024)
        .ok_or_else(|| CompositorRenderError::new("viewport destination is too large"))?;
    // Integer HiDPI and fractional-scale viewporter clients commonly supply the exact
    // output-density raster already. Keep its pixels while publishing logical geometry;
    // resampling this identity mapping needlessly visits every pixel after a resize.
    if transform == BufferTransform::Normal
        && raster == image.extent
        && source.x == 0.0
        && source.y == 0.0
        && source.width == logical_width as f64
        && source.height == logical_height as f64
    {
        return Ok((image, destination));
    }
    let mut pixels = vec![0_u8; destination_len];
    for y in 0..destination_height {
        for x in 0..destination_width {
            let transformed_x = ((source.x
                + (x as f64 + 0.5) * source.width / destination_width as f64)
                * scale as f64)
                .floor() as usize;
            let transformed_y = ((source.y
                + (y as f64 + 0.5) * source.height / destination_height as f64)
                * scale as f64)
                .floor() as usize;
            let (source_x, source_y) = transformed_coordinate(
                transform,
                transformed_x.min(transformed_width - 1),
                transformed_y.min(transformed_height - 1),
                input_width,
                input_height,
            );
            let source_index = (source_y * input_width + source_x) * 4;
            let target_index = (y * destination_width + x) * 4;
            pixels[target_index..target_index + 4]
                .copy_from_slice(&image.pixels[source_index..source_index + 4]);
        }
    }
    Ok((
        ImageResource {
            extent: raster,
            pixels: Arc::from(pixels),
            ..image
        },
        destination,
    ))
}

fn transformed_coordinate(
    transform: BufferTransform,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
) -> (usize, usize) {
    match transform {
        BufferTransform::Normal => (x, y),
        BufferTransform::Rotate90 => (y, height - 1 - x),
        BufferTransform::Rotate180 => (width - 1 - x, height - 1 - y),
        BufferTransform::Rotate270 => (width - 1 - y, x),
        BufferTransform::Flipped => (width - 1 - x, y),
        BufferTransform::Flipped90 => (width - 1 - y, height - 1 - x),
        BufferTransform::Flipped180 => (x, height - 1 - y),
        BufferTransform::Flipped270 => (y, x),
    }
}

pub struct DmaBufImporter {
    capabilities: Vec<crate::renderer_vulkan::VulkanDmaBufFormatCapability>,
    next_generation: u64,
    cached: std::collections::VecDeque<CachedImport>,
    cache_counts: [u64; 2],
}

struct CachedImport {
    buffer: WaylandBufferId,
    descriptor: crate::compositor_wayland::DmaBufDescriptor,
    identity: (u64, u64, u64), // device, inode, allocation bytes; never a reusable raw FD number
    image: crate::renderer_vulkan::CachedDmaBufImage,
}

impl DmaBufImporter {
    pub fn new(device: &VulkanDevice) -> Result<Self, CompositorRenderError> {
        let capabilities = device
            .dma_buf_import_capabilities(vk::ImageUsageFlags::SAMPLED)
            .map_err(render_error)?;
        Ok(Self::legacy_sdr(capabilities))
    }

    fn legacy_sdr(
        mut capabilities: Vec<crate::renderer_vulkan::VulkanDmaBufFormatCapability>,
    ) -> Self {
        // This compositor has no per-surface color-management protocol yet. Like wl_shm,
        // its legacy SDR client pixels are sRGB encoded. A DRM fourcc describes byte layout,
        // not the transfer function: never select the first UNORM/Linear candidate just because
        // it shares the same fourcc/modifier. Sampling must decode before linear composition.
        capabilities.retain(|capability| {
            capability.color_encoding == ImageColorEncoding::Srgb
                && matches!(
                    capability.format,
                    vk::Format::R8G8B8A8_SRGB | vk::Format::B8G8R8A8_SRGB
                )
                && capability.importable()
                && capability.plane_count == 1
        });
        Self {
            capabilities,
            next_generation: 0,
            cached: std::collections::VecDeque::new(),
            cache_counts: [0; 2],
        }
    }

    pub(crate) fn take_cache_counts(&mut self) -> [u64; 2] {
        std::mem::take(&mut self.cache_counts)
    }

    pub fn advertised_formats(&self) -> Vec<DmaBufFormat> {
        self.capabilities
            .iter()
            .filter(|capability| capability.importable() && capability.plane_count == 1)
            .map(|capability| DmaBufFormat {
                fourcc: capability.drm_fourcc,
                modifier: capability.drm_modifier,
            })
            .collect()
    }

    pub fn image_metadata(
        &self,
        image: &DmaBufImage,
    ) -> Result<(ImagePixelFormat, ImageAlphaMode), CompositorRenderError> {
        let plane = image.descriptor.planes.first().ok_or_else(|| {
            CompositorRenderError::new("DMA-BUF image does not contain a plane descriptor")
        })?;
        let capability = self
            .capabilities
            .iter()
            .find(|capability| {
                capability.drm_fourcc == image.descriptor.format
                    && capability.drm_modifier == plane.modifier
                    && capability.plane_count == 1
            })
            .ok_or_else(|| {
                CompositorRenderError::new("DMA-BUF tuple was not advertised by this Vulkan device")
            })?;
        let pixel_format = match capability.format {
            vk::Format::R8G8B8A8_UNORM | vk::Format::R8G8B8A8_SRGB => ImagePixelFormat::Rgba8,
            vk::Format::B8G8R8A8_UNORM | vk::Format::B8G8R8A8_SRGB => ImagePixelFormat::Bgra8,
            _ => {
                return Err(CompositorRenderError::new(
                    "DMA-BUF capability has an unsupported Vulkan pixel format",
                ));
            }
        };
        Ok((pixel_format, capability.alpha_mode))
    }

    /// Imports and binds one committed DMA-BUF generation without copying its pixels.
    ///
    /// The protocol runtime has validated the descriptor and owns every FD. The selected Vulkan
    /// capability comes from this exact device, and this method derives allocation bounds from the
    /// DMA-BUF itself before entering the renderer's unsafe host boundary.
    pub fn import_and_bind(
        &mut self,
        device: &VulkanDevice,
        scene: &mut VulkanScene,
        buffer: WaylandBufferId,
        content_version: u64,
        image: DmaBufImage,
        acquire: Option<OwnedFd>,
        damage: Vec<RectI>,
    ) -> Result<u64, CompositorRenderError> {
        if content_version == 0 || image.planes.len() != 1 || image.descriptor.planes.len() != 1 {
            return Err(CompositorRenderError::new(
                "DMA-BUF import requires one plane and a nonzero content version",
            ));
        }
        if image.descriptor.flags.interlaced || image.descriptor.flags.bottom_field_first {
            return Err(CompositorRenderError::new(
                "interlaced DMA-BUF content is not supported by Telorgon rendering",
            ));
        }
        let plane_descriptor = image.descriptor.planes[0];
        let capability = self
            .capabilities
            .iter()
            .copied()
            .find(|capability| {
                capability.drm_fourcc == image.descriptor.format
                    && capability.drm_modifier == plane_descriptor.modifier
                    && capability.plane_count == 1
            })
            .ok_or_else(|| {
                CompositorRenderError::new("DMA-BUF tuple was not advertised by this Vulkan device")
            })?;
        let identity = fd_allocation_identity(&image.planes[0])?;
        let allocation_size = identity.2;
        let minimum_size = u64::from(plane_descriptor.stride)
            .checked_mul(image.descriptor.size.height as u64)
            .ok_or_else(|| CompositorRenderError::new("DMA-BUF extent overflow"))?;
        if allocation_size < minimum_size {
            return Err(CompositorRenderError::new(
                "DMA-BUF allocation is smaller than its declared rows",
            ));
        }
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .ok_or_else(|| CompositorRenderError::new("DMA-BUF lease generation exhausted"))?;
        let generation = self.next_generation;
        for rect in &damage {
            if rect.x < 0
                || rect.y < 0
                || rect.width <= 0
                || rect.height <= 0
                || rect
                    .x
                    .checked_add(rect.width)
                    .is_none_or(|right| right > image.descriptor.size.width)
                || rect
                    .y
                    .checked_add(rect.height)
                    .is_none_or(|bottom| bottom > image.descriptor.size.height)
            {
                return Err(CompositorRenderError::new(
                    "DMA-BUF damage lies outside its physical extent",
                ));
            }
        }
        if let Some(index) = self.cached.iter().position(|entry| {
            entry.buffer == buffer
                && entry.descriptor == image.descriptor
                && entry.identity == identity
                && identity.1 != 0
                && entry.image.matches_device(device)
                && entry.image.can_reuse()
        }) {
            let mut entry = self.cached.remove(index).expect("cached index exists");
            // Immutable layout and kernel allocation identity match; can_reuse proves completion,
            // one-shot release resolution and absence of scene/submission pins.
            let lease = unsafe {
                entry
                    .image
                    .reuse(content_version, generation, acquire, damage)
            }
            .map_err(render_error)?;
            self.cached.push_back(entry);
            scene
                .bind_external_image(dma_buf_image_id(), lease)
                .map_err(render_error)?;
            self.cache_counts[0] += 1;
            return Ok(generation);
        }
        let descriptor = image.descriptor.clone();
        let plane = image.planes.into_iter().next().expect("one plane checked");
        let import = VulkanDmaBufImport {
            planes: vec![VulkanDmaBufPlane {
                memory: plane,
                memory_index: 0,
                offset: u64::from(plane_descriptor.offset),
                size: allocation_size.saturating_sub(u64::from(plane_descriptor.offset)),
                row_pitch: plane_descriptor.stride,
                allocation_size,
            }],
            drm_fourcc: image.descriptor.format,
            drm_modifier: plane_descriptor.modifier,
            format: capability.format,
            extent: vk::Extent2D {
                width: image.descriptor.size.width as u32,
                height: image.descriptor.size.height as u32,
            },
            usage: vk::ImageUsageFlags::SAMPLED,
            content_version,
            lease_generation: generation,
            color_encoding: capability.color_encoding,
            alpha_mode: capability.alpha_mode,
            // The desktop materialization scene applies the DMA-BUF Y_INVERT flag together with
            // wl_surface transform/scale and viewporter geometry.
            origin: VulkanExternalImageOrigin::TopLeft,
            initial_use: HostedImageUse::General,
            final_use: HostedImageUse::General,
            acquire,
            damage,
            protected: false,
        };
        let lease: VulkanExternalImageLease =
            unsafe { device.import_dma_buf(import) }.map_err(render_error)?;
        self.cache_counts[1] += 1;
        if allocation_size <= 512 * 1024 * 1024 {
            while self.cached.len() >= 64
                || self
                    .cached
                    .iter()
                    .map(|entry| entry.identity.2)
                    .sum::<u64>()
                    + allocation_size
                    > 512 * 1024 * 1024
            {
                self.cached.pop_front();
            }
            self.cached.push_back(CachedImport {
                buffer,
                descriptor,
                identity,
                image: lease.cache_dma_buf(),
            });
        }
        scene
            .bind_external_image(dma_buf_image_id(), lease)
            .map_err(render_error)?;
        Ok(generation)
    }
}

fn fd_allocation_identity(fd: &OwnedFd) -> Result<(u64, u64, u64), CompositorRenderError> {
    use std::os::unix::fs::MetadataExt;
    let file = std::fs::File::from(fd.try_clone().map_err(io_error)?);
    let metadata = file.metadata().map_err(io_error)?;
    let size = metadata.len();
    if size == 0 {
        Err(CompositorRenderError::new(
            "DMA-BUF did not expose a nonzero allocation size",
        ))
    } else {
        Ok((metadata.dev(), metadata.ino(), size))
    }
}

fn io_error(error: std::io::Error) -> CompositorRenderError {
    CompositorRenderError::new(error.to_string())
}

fn render_error(error: crate::render::RenderError) -> CompositorRenderError {
    CompositorRenderError::new(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compositor_wayland::{ShmBuffer, ShmFormat};
    use crate::core::SizeI;

    fn dma_buf_capability(
        srgb: bool,
        modifier: u64,
    ) -> crate::renderer_vulkan::VulkanDmaBufFormatCapability {
        crate::renderer_vulkan::VulkanDmaBufFormatCapability {
            drm_fourcc: u32::from_le_bytes(*b"XR24"),
            drm_modifier: modifier,
            format: if srgb {
                vk::Format::B8G8R8A8_SRGB
            } else {
                vk::Format::B8G8R8A8_UNORM
            },
            usage: vk::ImageUsageFlags::SAMPLED,
            color_encoding: if srgb {
                ImageColorEncoding::Srgb
            } else {
                ImageColorEncoding::Linear
            },
            alpha_mode: ImageAlphaMode::Opaque,
            plane_count: 1,
            tiling_features: vk::FormatFeatureFlags::SAMPLED_IMAGE
                | vk::FormatFeatureFlags::SAMPLED_IMAGE_FILTER_LINEAR,
            external_memory_features: vk::ExternalMemoryFeatureFlags::IMPORTABLE,
            max_extent: vk::Extent3D {
                width: 8192,
                height: 8192,
                depth: 1,
            },
        }
    }

    #[test]
    fn legacy_sdr_never_selects_linear_for_an_identical_drm_tuple() {
        for mut capabilities in [
            vec![dma_buf_capability(false, 17), dma_buf_capability(true, 17)],
            vec![dma_buf_capability(true, 17), dma_buf_capability(false, 17)],
        ] {
            // This modifier is supported only as linear: do not advertise it as sRGB.
            capabilities.push(dma_buf_capability(false, 21));
            let importer = DmaBufImporter::legacy_sdr(capabilities);
            assert_eq!(
                importer.advertised_formats(),
                vec![DmaBufFormat {
                    fourcc: u32::from_le_bytes(*b"XR24"),
                    modifier: 17,
                }]
            );
            let selected = importer.capabilities[0];
            assert_eq!(selected.format, vk::Format::B8G8R8A8_SRGB);
            assert_eq!(selected.color_encoding, ImageColorEncoding::Srgb);
            assert_eq!(selected.alpha_mode, ImageAlphaMode::Opaque);
        }
    }

    #[test]
    fn sdr_advertisement_excludes_nonimportable_multiplane_and_mislabeled_formats() {
        let valid = dma_buf_capability(true, 0);
        let mut unimportable = valid;
        unimportable.external_memory_features = vk::ExternalMemoryFeatureFlags::EXPORTABLE;
        let mut multiplane = valid;
        multiplane.plane_count = 2;
        let mut mislabeled = valid;
        mislabeled.format = vk::Format::B8G8R8A8_UNORM;
        let importer = DmaBufImporter::legacy_sdr(vec![unimportable, multiplane, mislabeled]);
        assert!(importer.advertised_formats().is_empty());
    }

    #[test]
    fn sdr_rgba_and_bgra_keep_opaque_and_premultiplied_alpha_contracts() {
        for (fourcc, format, alpha) in [
            (*b"XR24", vk::Format::B8G8R8A8_SRGB, ImageAlphaMode::Opaque),
            (
                *b"AR24",
                vk::Format::B8G8R8A8_SRGB,
                ImageAlphaMode::Premultiplied,
            ),
            (*b"XB24", vk::Format::R8G8B8A8_SRGB, ImageAlphaMode::Opaque),
            (
                *b"AB24",
                vk::Format::R8G8B8A8_SRGB,
                ImageAlphaMode::Premultiplied,
            ),
        ] {
            let mut capability = dma_buf_capability(true, 0);
            capability.drm_fourcc = u32::from_le_bytes(fourcc);
            capability.format = format;
            capability.alpha_mode = alpha;
            let importer = DmaBufImporter::legacy_sdr(vec![capability]);
            assert_eq!(importer.capabilities, [capability]);
        }
    }

    #[test]
    fn opaque_sdr_ramp_survives_import_materialization_and_presentation() {
        use crate::renderer_vulkan::VulkanMaterializationTarget;
        fn decode(value: f64) -> f64 {
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        }
        fn encode(value: f64) -> f64 {
            if value <= 0.0031308 {
                value * 12.92
            } else {
                1.055 * value.powf(1.0 / 2.4) - 0.055
            }
        }
        fn srgb(format: vk::Format) -> bool {
            matches!(
                format,
                vk::Format::R8G8B8A8_SRGB | vk::Format::B8G8R8A8_SRGB
            )
        }
        let importer = DmaBufImporter::legacy_sdr(vec![
            dma_buf_capability(false, 0),
            dma_buf_capability(true, 0),
        ]);
        let selected = importer.capabilities[0];
        assert_eq!(
            VulkanMaterializationTarget::COLOR_SPACE,
            crate::render::ColorSpace::Srgb
        );
        assert_eq!(
            VulkanMaterializationTarget::COLOR_ENCODING,
            ImageColorEncoding::Srgb
        );
        // CPU reference for the specified Vulkan format conversions, not a GPU execution test.
        // Both the previous UNORM import (bright midtones) and an 8-bit linear intermediate
        // (lost dark shades) break this full 8-bit ramp round trip.
        for byte in 0..=255 {
            let source = f64::from(byte) / 255.0;
            let linear = if srgb(selected.format) {
                decode(source)
            } else {
                source
            };
            let stored = if srgb(VulkanMaterializationTarget::FORMAT) {
                encode(linear)
            } else {
                linear
            };
            let quantized = (stored * 255.0).round() / 255.0;
            let resampled = if srgb(VulkanMaterializationTarget::FORMAT) {
                decode(quantized)
            } else {
                quantized
            };
            let presented = (encode(resampled) * 255.0).round() as i32;
            assert!(
                (presented - byte).abs() <= 1,
                "input={byte}, displayed={presented}"
            );
        }
        // The original mismatch maps a middle-gray encoded channel (~128) to ~188.
        assert!((encode(128.0 / 255.0) * 255.0 - 188.0).abs() < 1.0);
    }

    #[test]
    #[ignore = "CPU-only timing probe; run explicitly with --ignored --nocapture"]
    fn native_density_image_preparation_timing() {
        let image = ImageResource {
            image: ImageId(7),
            content_version: 1,
            extent: SizeI {
                width: 3840,
                height: 2400,
            },
            color_encoding: ImageColorEncoding::Srgb,
            alpha_mode: ImageAlphaMode::Opaque,
            pixel_format: ImagePixelFormat::Bgra8,
            pixels: vec![127; 3840 * 2400 * 4].into(),
        };
        let start = std::time::Instant::now();
        for _ in 0..10 {
            std::hint::black_box(
                transform_surface_image_at_scale(
                    std::hint::black_box(image.clone()),
                    3,
                    BufferTransform::Normal,
                    None,
                    crate::platform::ScaleFactor::new(3.0).unwrap(),
                )
                .unwrap(),
            );
        }
        eprintln!(
            "3840x2400 at 300%: {:?} per preparation (10 iterations)",
            start.elapsed() / 10
        );
    }

    #[test]
    fn xrgb_shm_retains_native_bgra_bytes_and_is_forced_opaque() {
        let buffer = WaylandBufferId::from_raw(7).unwrap();
        let resource = shm_image_resource(
            buffer,
            1,
            ShmImage {
                descriptor: ShmBuffer {
                    offset: 0,
                    size: SizeI {
                        width: 1,
                        height: 1,
                    },
                    stride: 4,
                    format: ShmFormat::Xrgb8888,
                },
                pixels: vec![3, 2, 1, 0],
            },
        )
        .unwrap();
        assert_eq!(&*resource.pixels, &[3, 2, 1, 0]);
        assert_eq!(resource.pixel_format, ImagePixelFormat::Bgra8);
        assert_eq!(resource.alpha_mode, ImageAlphaMode::Opaque);
    }

    #[test]
    fn damaged_argb_region_stays_tightly_packed_and_native_bgra() {
        let buffer = WaylandBufferId::from_raw(7).unwrap();
        let rect = RectI {
            x: 3,
            y: 4,
            width: 2,
            height: 1,
        };
        let update = shm_image_update(
            buffer,
            2,
            ShmImageRegion {
                descriptor: ShmBuffer {
                    offset: 16,
                    size: SizeI {
                        width: 20,
                        height: 10,
                    },
                    stride: 96,
                    format: ShmFormat::Argb8888,
                },
                rect,
                row_bytes: 8,
                pixels: vec![3, 2, 1, 4, 7, 6, 5, 8],
            },
        )
        .unwrap();

        assert_eq!(update.rect, rect);
        assert_eq!(update.row_bytes, 8);
        assert_eq!(update.pixel_format, ImagePixelFormat::Bgra8);
        assert_eq!(update.alpha_mode, ImageAlphaMode::Premultiplied);
        assert_eq!(&*update.pixels, &[3, 2, 1, 4, 7, 6, 5, 8]);
    }

    #[test]
    fn same_raster_extent_still_applies_cropping_and_rotation() {
        let image = ImageResource {
            image: ImageId(7),
            content_version: 1,
            extent: SizeI {
                width: 2,
                height: 2,
            },
            color_encoding: ImageColorEncoding::Srgb,
            alpha_mode: ImageAlphaMode::Opaque,
            pixel_format: ImagePixelFormat::Rgba8,
            pixels: Arc::from([1, 0, 0, 255, 2, 0, 0, 255, 3, 0, 0, 255, 4, 0, 0, 255]),
        };
        let (cropped, _) = transform_surface_image_at_scale(
            image.clone(),
            1,
            BufferTransform::Normal,
            Some(ViewportState {
                source: Some(ViewportSource {
                    x: 1.0,
                    y: 0.0,
                    width: 1.0,
                    height: 2.0,
                }),
                destination: Some(image.extent),
            }),
            crate::platform::ScaleFactor::new(1.0).unwrap(),
        )
        .unwrap();
        assert_eq!(cropped.extent, image.extent);
        assert_eq!(
            &*cropped.pixels,
            &[2, 0, 0, 255, 2, 0, 0, 255, 4, 0, 0, 255, 4, 0, 0, 255]
        );
        let (rotated, _) = transform_surface_image_at_scale(
            image.clone(),
            2,
            BufferTransform::Rotate90,
            None,
            crate::platform::ScaleFactor::new(2.0).unwrap(),
        )
        .unwrap();
        assert_eq!(rotated.extent, image.extent);
        assert_eq!(
            &*rotated.pixels,
            &[3, 0, 0, 255, 1, 0, 0, 255, 4, 0, 0, 255, 2, 0, 0, 255]
        );
    }

    #[test]
    fn surface_transform_scale_and_viewport_change_the_retained_extent() {
        let image = ImageResource {
            image: ImageId(7),
            content_version: 1,
            extent: SizeI {
                width: 2,
                height: 2,
            },
            color_encoding: ImageColorEncoding::Srgb,
            alpha_mode: ImageAlphaMode::Opaque,
            pixel_format: ImagePixelFormat::Rgba8,
            pixels: Arc::from([
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            ]),
        };
        let (dense, logical) = transform_surface_image_at_scale(
            image.clone(),
            2,
            BufferTransform::Normal,
            None,
            crate::platform::ScaleFactor::new(2.0).unwrap(),
        )
        .unwrap();
        assert_eq!(
            logical,
            SizeI {
                width: 1,
                height: 1
            }
        );
        assert_eq!(dense.extent, image.extent);
        assert!(Arc::ptr_eq(&dense.pixels, &image.pixels));
        assert_eq!(
            dense.pixels, image.pixels,
            "2x client detail must survive materialization"
        );
        let (fractional, logical) = transform_surface_image_at_scale(
            image.clone(),
            1,
            BufferTransform::Normal,
            Some(ViewportState {
                source: None,
                destination: Some(SizeI {
                    width: 1,
                    height: 1,
                }),
            }),
            crate::platform::ScaleFactor::new(1.5).unwrap(),
        )
        .unwrap();
        assert_eq!(
            logical,
            SizeI {
                width: 1,
                height: 1
            }
        );
        assert_eq!(fractional.extent, image.extent);
        assert_eq!(fractional.pixels, image.pixels);
        assert!(Arc::ptr_eq(&fractional.pixels, &image.pixels));
        assert!(
            transform_surface_image_at_scale(
                image.clone(),
                3,
                BufferTransform::Normal,
                None,
                crate::platform::ScaleFactor::new(2.0).unwrap()
            )
            .is_err()
        );
        let scaled = transform_surface_image(image.clone(), 2, BufferTransform::Normal, None)
            .expect("scale is valid");
        assert_eq!(
            scaled.extent,
            SizeI {
                width: 1,
                height: 1
            }
        );
        assert_eq!(&*scaled.pixels, &[255, 255, 255, 255]);

        let cropped = transform_surface_image(
            image,
            1,
            BufferTransform::Rotate90,
            Some(ViewportState {
                source: Some(ViewportSource {
                    x: 0.0,
                    y: 0.0,
                    width: 1.0,
                    height: 2.0,
                }),
                destination: Some(SizeI {
                    width: 1,
                    height: 2,
                }),
            }),
        )
        .expect("viewport is valid");
        assert_eq!(
            cropped.extent,
            SizeI {
                width: 1,
                height: 2
            }
        );
        assert_eq!(&cropped.pixels[..4], &[0, 0, 255, 255]);
    }
}

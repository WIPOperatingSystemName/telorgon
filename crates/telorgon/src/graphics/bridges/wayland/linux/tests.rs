use super::*;
use crate::integrations::wayland::compositor::{ShmBuffer, ShmFormat};
use crate::foundation::SizeI;

fn dma_buf_capability(
    srgb: bool,
    modifier: u64,
) -> crate::graphics::renderers::vulkan::VulkanDmaBufFormatCapability {
    crate::graphics::renderers::vulkan::VulkanDmaBufFormatCapability {
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
    use crate::graphics::renderers::vulkan::VulkanMaterializationTarget;
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
        crate::graphics::render::ColorSpace::Srgb
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
                crate::platform::contracts::ScaleFactor::new(3.0).unwrap(),
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
        crate::platform::contracts::ScaleFactor::new(1.0).unwrap(),
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
        crate::platform::contracts::ScaleFactor::new(2.0).unwrap(),
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
        crate::platform::contracts::ScaleFactor::new(2.0).unwrap(),
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
        crate::platform::contracts::ScaleFactor::new(1.5).unwrap(),
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
            crate::platform::contracts::ScaleFactor::new(2.0).unwrap()
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

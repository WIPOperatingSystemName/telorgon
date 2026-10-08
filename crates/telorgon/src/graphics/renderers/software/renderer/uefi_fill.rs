use super::{
    BlendMode, BoxInstance, ColorSpace, RasterTarget, RectF, RenderClip, RenderSpatialNode,
    clip_to_target,
};
use crate::foundation::Affine2D;
use crate::ui::CornerRadii;

const MAX_COORDINATE: f32 = 1_048_576.0;

/// Handles only opaque, undecorated rectangles with exact pixel coverage. All other paint
/// continues through the reference rasterizer; this helper performs no writes when rejected.
pub(super) fn try_draw_box(
    raster: &mut RasterTarget<'_>,
    instance: &BoxInstance,
    spatial: Option<&RenderSpatialNode>,
    clip: Option<&RenderClip>,
    region: RectF,
) -> bool {
    let Some(color) = instance.background else {
        return false;
    };
    if color.a != 255
        || instance.opacity != 1.0
        || raster.color_space != ColorSpace::Srgb
        || !matches!(raster.blend_mode, BlendMode::Alpha | BlendMode::Opaque)
        || raster.origin.x != 0
        || raster.origin.y != 0
        || raster.coverage_normalization.is_some()
        || raster.rounded_clips.iter().any(Option::is_some)
        || !square(instance.corner_radii)
        || [
            instance.border.top.width,
            instance.border.right.width,
            instance.border.bottom.width,
            instance.border.left.width,
        ]
        .iter()
        .any(|width| *width != 0.0)
        || instance.outline.width != 0.0
        || instance.outline.offset != 0.0
        || !instance.shadows.as_slice().is_empty()
        || raster.width == 0
        || raster.height == 0
        || raster.width > MAX_COORDINATE as usize
        || raster.height > MAX_COORDINATE as usize
        || !bounded_rect(instance.rect)
        || !bounded_rect(region)
        || !integral_rect(region)
    {
        return false;
    }
    let Some(expected) = raster
        .width
        .checked_mul(raster.height)
        .and_then(|pixels| pixels.checked_mul(4))
    else {
        return false;
    };
    if raster.pixels.len() < expected {
        return false;
    }
    let transform = spatial.map_or(Affine2D::IDENTITY, |node| node.transform);
    // The reference AA uses the minimum axis scale for both axes. Uniform binary-exact scales
    // and integral translation avoid anisotropic edge coverage and inverse-rounding changes.
    if transform.m12 != 0.0
        || transform.m21 != 0.0
        || transform.m11 != transform.m22
        || ![0.25, 0.5, 1.0, 2.0, 4.0].contains(&transform.m11)
        || !bounded_integral(transform.tx)
        || !bounded_integral(transform.ty)
    {
        return false;
    }
    let world = transform.transform_rect(instance.rect);
    if !bounded_rect(world) || !integral_rect(world) {
        return false;
    }
    if let Some(clip) = clip
        && (!square(clip.corner_radii) || !bounded_rect(clip.rect) || !integral_rect(clip.rect))
    {
        return false;
    }
    let Some(bounds) = world
        .intersection(region)
        .and_then(|bounds| match clip {
            Some(clip) => bounds.intersection(clip.rect),
            None => Some(bounds),
        })
        .and_then(|bounds| clip_to_target(bounds, raster.width, raster.height))
    else {
        return true;
    };
    // Every intersected boundary is integral, finite and within the checked target dimensions.
    let left = bounds.x as usize;
    let right = bounds.right() as usize;
    let top = bounds.y as usize;
    let bottom = bounds.bottom() as usize;
    let first_start = (top * raster.width + left) * 4;
    let first_end = (top * raster.width + right) * 4;
    let rgba = [color.r, color.g, color.b, color.a];
    for pixel in raster.pixels[first_start..first_end].chunks_exact_mut(4) {
        pixel.copy_from_slice(&rgba);
    }
    // Reuse the first complete row with safe slice copies; no float work or allocation remains
    // in the pixel loop, and untouched pixels outside the clipped rectangle retain their bytes.
    for row in top + 1..bottom {
        raster
            .pixels
            .copy_within(first_start..first_end, (row * raster.width + left) * 4);
    }
    true
}

fn square(radii: CornerRadii) -> bool {
    [
        radii.top_left,
        radii.top_right,
        radii.bottom_right,
        radii.bottom_left,
    ]
    .iter()
    .all(|radius| *radius == 0.0)
}

fn bounded_rect(rect: RectF) -> bool {
    rect.width > 0.0
        && rect.height > 0.0
        && [
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            rect.right(),
            rect.bottom(),
        ]
        .iter()
        .all(|value| value.is_finite() && value.abs() <= MAX_COORDINATE)
}

fn bounded_integral(value: f32) -> bool {
    value.is_finite() && value.abs() <= MAX_COORDINATE && value == (value as i32) as f32
}

fn integral_rect(rect: RectF) -> bool {
    [rect.x, rect.y, rect.right(), rect.bottom()]
        .iter()
        .all(|value| bounded_integral(*value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::{ColorRgba8, PointI};
    use crate::graphics::render::{Border, RoundedClip, Shadow};
    use crate::ui::layout::{ClipId, SpatialId};
    use crate::ui::{Outline, ShadowList, UiNodeId};

    fn rect(x: f32, y: f32, width: f32, height: f32) -> RectF {
        RectF {
            x,
            y,
            width,
            height,
        }
    }

    fn instance(rect: RectF) -> BoxInstance {
        BoxInstance {
            node: UiNodeId::new(0, 1),
            rect,
            view_bounds: rect,
            background: Some(ColorRgba8::rgba(57, 155, 210, 255)),
            border: Border::default(),
            outline: Outline::default(),
            corner_radii: CornerRadii::default(),
            shadows: ShadowList::default(),
            opacity: 1.0,
            clip: ClipId(0),
            spatial: SpatialId(0),
        }
    }

    fn raster(
        pixels: &mut [u8],
        width: usize,
        height: usize,
        blend: BlendMode,
    ) -> RasterTarget<'_> {
        RasterTarget {
            pixels,
            width,
            height,
            origin: PointI::default(),
            blend_mode: blend,
            color_space: ColorSpace::Srgb,
            rounded_clips: [None; 2],
            coverage_normalization: None,
        }
    }

    fn compare(instance: &BoxInstance, transform: Affine2D, clip: Option<RectF>, region: RectF) {
        let spatial = RenderSpatialNode {
            id: SpatialId(0),
            transform,
        };
        let clip = clip.map(|rect| RenderClip {
            id: ClipId(1),
            rect,
            corner_radii: CornerRadii::default(),
        });
        for blend in [BlendMode::Alpha, BlendMode::Opaque] {
            let mut fast = vec![0; 10 * 8 * 4];
            for pixel in fast.chunks_exact_mut(4) {
                pixel.copy_from_slice(&[13, 23, 34, 111]);
            }
            let mut reference = fast.clone();
            assert!(try_draw_box(
                &mut raster(&mut fast, 10, 8, blend),
                instance,
                Some(&spatial),
                clip.as_ref(),
                region
            ));
            // The integrated fast call is UEFI-only, so this is the original desktop rasterizer.
            super::super::draw_box(
                &mut raster(&mut reference, 10, 8, blend),
                instance,
                Some(&spatial),
                clip.as_ref(),
                region,
            );
            assert_eq!(
                fast, reference,
                "transform {transform:?}, clip {clip:?}, region {region:?}"
            );
        }
    }

    #[test]
    fn exact_fill_matches_reference_with_clipping_scaling_translation_and_negative_bounds() {
        let region = rect(0.0, 0.0, 10.0, 8.0);
        compare(&instance(region), Affine2D::IDENTITY, None, region);
        compare(
            &instance(rect(-3.0, -2.0, 8.0, 7.0)),
            Affine2D::IDENTITY,
            Some(rect(-1.0, 1.0, 5.0, 6.0)),
            region,
        );
        compare(
            &instance(rect(1.0, 1.0, 7.0, 3.0)),
            Affine2D::translation(-2.0, 1.0),
            None,
            rect(2.0, 3.0, 4.0, 2.0),
        );
        for scale in [0.25, 0.5, 1.0, 2.0, 4.0] {
            compare(
                &instance(rect(4.0 / scale, 4.0 / scale, 4.0 / scale, 2.0 / scale)),
                Affine2D {
                    m11: scale,
                    m22: scale,
                    ..Affine2D::IDENTITY
                },
                Some(rect(5.0, 0.0, 3.0, 8.0)),
                region,
            );
        }
        // An entirely clipped eligible box is handled without touching any target byte.
        compare(
            &instance(rect(-10.0, -8.0, 2.0, 2.0)),
            Affine2D::IDENTITY,
            None,
            region,
        );
    }

    #[test]
    fn exact_fill_preserves_every_opaque_srgb_byte() {
        let region = rect(0.0, 0.0, 2.0, 2.0);
        let mut instance = instance(region);
        for byte in 0..=255 {
            let byte = byte as u8;
            instance.background = Some(ColorRgba8::rgba(
                byte,
                255 - byte,
                byte.wrapping_mul(17),
                255,
            ));
            compare(&instance, Affine2D::IDENTITY, None, region);
        }
    }

    #[test]
    fn rejects_decorated_translucent_fractional_and_nonfinite_boxes_without_writes() {
        let region = rect(0.0, 0.0, 10.0, 8.0);
        let changes: [fn(&mut BoxInstance); 12] = [
            |item| item.background = None,
            |item| item.background.as_mut().unwrap().a = 254,
            |item| item.opacity = 0.5,
            |item| item.corner_radii.top_left = 1.0,
            |item| item.border.top.width = 1.0,
            |item| item.outline.width = 1.0,
            |item| item.outline.offset = 1.0,
            |item| item.shadows = ShadowList::one(Shadow::default()),
            |item| item.rect.x = 0.25,
            |item| item.rect.width = -1.0,
            |item| item.rect.width = f32::INFINITY,
            |item| item.rect.x = f32::NAN,
        ];
        for change in changes {
            let mut instance = instance(region);
            change(&mut instance);
            let mut pixels = vec![19; 10 * 8 * 4];
            assert!(!try_draw_box(
                &mut raster(&mut pixels, 10, 8, BlendMode::Alpha),
                &instance,
                None,
                None,
                region
            ));
            assert!(pixels.iter().all(|byte| *byte == 19));
        }
    }

    #[test]
    fn rejects_unsafe_transforms_target_state_clips_and_short_buffers() {
        let region = rect(0.0, 0.0, 10.0, 8.0);
        let instance = instance(region);
        for transform in [
            Affine2D {
                m11: 2.0,
                ..Affine2D::IDENTITY
            },
            Affine2D {
                m11: 1.5,
                m22: 1.5,
                ..Affine2D::IDENTITY
            },
            Affine2D {
                m11: -1.0,
                ..Affine2D::IDENTITY
            },
            Affine2D {
                m11: 0.0,
                ..Affine2D::IDENTITY
            },
            Affine2D {
                m12: 0.25,
                ..Affine2D::IDENTITY
            },
            Affine2D {
                tx: 0.5,
                ..Affine2D::IDENTITY
            },
            Affine2D {
                tx: f32::INFINITY,
                ..Affine2D::IDENTITY
            },
        ] {
            let spatial = RenderSpatialNode {
                id: SpatialId(0),
                transform,
            };
            let mut pixels = vec![19; 10 * 8 * 4];
            assert!(!try_draw_box(
                &mut raster(&mut pixels, 10, 8, BlendMode::Alpha),
                &instance,
                Some(&spatial),
                None,
                region
            ));
            assert!(pixels.iter().all(|byte| *byte == 19));
        }
        for state in 0..5 {
            let mut pixels = vec![19; 10 * 8 * 4];
            let mut raster = raster(&mut pixels, 10, 8, BlendMode::Alpha);
            match state {
                0 => raster.color_space = ColorSpace::Linear,
                1 => raster.blend_mode = BlendMode::Add,
                2 => raster.origin.x = 1,
                3 => {
                    raster.rounded_clips[0] = Some(RoundedClip::new(region, CornerRadii::default()))
                }
                _ => {
                    raster.coverage_normalization = Some(
                        crate::graphics::render::frame_border::InteriorPaint::new(&instance),
                    )
                }
            }
            assert!(!try_draw_box(&mut raster, &instance, None, None, region));
            assert!(pixels.iter().all(|byte| *byte == 19));
        }
        for clip in [
            RenderClip {
                id: ClipId(1),
                rect: region,
                corner_radii: CornerRadii::all(1.0),
            },
            RenderClip {
                id: ClipId(1),
                rect: rect(0.5, 0.0, 8.0, 8.0),
                corner_radii: CornerRadii::default(),
            },
        ] {
            let mut pixels = vec![19; 10 * 8 * 4];
            assert!(!try_draw_box(
                &mut raster(&mut pixels, 10, 8, BlendMode::Alpha),
                &instance,
                None,
                Some(&clip),
                region
            ));
            assert!(pixels.iter().all(|byte| *byte == 19));
        }
        for (width, height, length) in [(10, 8, 4), (usize::MAX, 8, 4), (0, 8, 4)] {
            let mut pixels = vec![19; length];
            assert!(!try_draw_box(
                &mut raster(&mut pixels, width, height, BlendMode::Alpha),
                &instance,
                None,
                None,
                region
            ));
            assert!(pixels.iter().all(|byte| *byte == 19));
        }
    }
}

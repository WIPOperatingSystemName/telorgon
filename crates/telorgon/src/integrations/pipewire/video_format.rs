use super::{MediaError, parameters::encode};
use crate::media::video::*;
use pipewire::spa::{
    self,
    pod::{ChoiceValue, Pod, Property, PropertyFlags, Value},
    sys::*,
    utils::{Choice, ChoiceEnum, ChoiceFlags, Fraction, Id, Rectangle},
};
pub(super) fn pixel(format: PixelFormat) -> u32 {
    match format {
        PixelFormat::Rgba8 => SPA_VIDEO_FORMAT_RGBA,
        PixelFormat::Bgra8 => SPA_VIDEO_FORMAT_BGRA,
        PixelFormat::Rgbx8 => SPA_VIDEO_FORMAT_RGBx,
        PixelFormat::Bgrx8 => SPA_VIDEO_FORMAT_BGRx,
        PixelFormat::Rgb8 => SPA_VIDEO_FORMAT_RGB,
        PixelFormat::Bgr8 => SPA_VIDEO_FORMAT_BGR,
        PixelFormat::Nv12 => SPA_VIDEO_FORMAT_NV12,
        PixelFormat::I420 => SPA_VIDEO_FORMAT_I420,
        PixelFormat::Yuy2 => SPA_VIDEO_FORMAT_YUY2,
        PixelFormat::P010 => SPA_VIDEO_FORMAT_P010_10LE,
        PixelFormat::RgbaF16 => SPA_VIDEO_FORMAT_RGBA_F16,
    }
}
fn format_properties(
    format: VideoFormat,
    bounds: Option<VideoCaptureRange>,
) -> Result<Vec<Property>, MediaError> {
    format.validate()?;
    let mut props = vec![
        Property::new(SPA_FORMAT_mediaType, Value::Id(Id(SPA_MEDIA_TYPE_video))),
        Property::new(
            SPA_FORMAT_mediaSubtype,
            Value::Id(Id(SPA_MEDIA_SUBTYPE_raw)),
        ),
        Property::new(SPA_FORMAT_VIDEO_format, Value::Id(Id(pixel(format.pixel)))),
        Property::new(
            SPA_FORMAT_VIDEO_size,
            Value::Rectangle(Rectangle {
                width: format.width,
                height: format.height,
            }),
        ),
        Property::new(
            SPA_FORMAT_VIDEO_framerate,
            Value::Fraction(Fraction {
                num: format.rate.numerator,
                denom: format.rate.denominator,
            }),
        ),
    ];
    if let Some(bounds) = bounds {
        bounds.validate(format)?;
        props[3].value = Value::Choice(ChoiceValue::Rectangle(Choice(
            ChoiceFlags::empty(),
            ChoiceEnum::Range {
                default: Rectangle {
                    width: format.width,
                    height: format.height,
                },
                min: Rectangle {
                    width: bounds.min_size[0],
                    height: bounds.min_size[1],
                },
                max: Rectangle {
                    width: bounds.max_size[0],
                    height: bounds.max_size[1],
                },
            },
        )));
        props[4].value = Value::Choice(ChoiceValue::Fraction(Choice(
            ChoiceFlags::empty(),
            ChoiceEnum::Range {
                default: Fraction {
                    num: format.rate.numerator,
                    denom: format.rate.denominator,
                },
                min: Fraction {
                    num: bounds.min_rate.numerator,
                    denom: bounds.min_rate.denominator,
                },
                max: Fraction {
                    num: bounds.max_rate.numerator,
                    denom: bounds.max_rate.denominator,
                },
            },
        )));
    }
    if let Some(bounds) = bounds {
        props.push(Property::new(SPA_FORMAT_VIDEO_maxFramerate,
            Value::Choice(ChoiceValue::Fraction(Choice(ChoiceFlags::empty(), ChoiceEnum::Range {
                default: Fraction { num: bounds.max_rate.numerator, denom: bounds.max_rate.denominator },
                min: Fraction { num: 0, denom: 1 },
                max: Fraction { num: bounds.max_rate.numerator, denom: bounds.max_rate.denominator },
            })))));
    }
    let c = format.color;
    for (key, value) in [
        (
            SPA_FORMAT_VIDEO_colorRange,
            match c.range {
                ColorRange::Unknown => 0,
                ColorRange::Full => SPA_VIDEO_COLOR_RANGE_0_255,
                ColorRange::Limited => SPA_VIDEO_COLOR_RANGE_16_235,
            },
        ),
        (
            SPA_FORMAT_VIDEO_colorMatrix,
            match c.matrix {
                ColorMatrix::Unknown => 0,
                ColorMatrix::Rgb => SPA_VIDEO_COLOR_MATRIX_RGB,
                ColorMatrix::Bt601 => SPA_VIDEO_COLOR_MATRIX_BT601,
                ColorMatrix::Bt709 => SPA_VIDEO_COLOR_MATRIX_BT709,
                ColorMatrix::Bt2020 => SPA_VIDEO_COLOR_MATRIX_BT2020,
            },
        ),
        (
            SPA_FORMAT_VIDEO_colorPrimaries,
            match c.primaries {
                ColorPrimaries::Unknown => 0,
                ColorPrimaries::Bt709 => SPA_VIDEO_COLOR_PRIMARIES_BT709,
                ColorPrimaries::Bt2020 => SPA_VIDEO_COLOR_PRIMARIES_BT2020,
                ColorPrimaries::DisplayP3 => SPA_VIDEO_COLOR_PRIMARIES_SMPTEEG432,
            },
        ),
        (
            SPA_FORMAT_VIDEO_transferFunction,
            match c.transfer {
                TransferFunction::Unknown => 0,
                TransferFunction::Linear => SPA_VIDEO_TRANSFER_GAMMA10,
                TransferFunction::Srgb => SPA_VIDEO_TRANSFER_SRGB,
                TransferFunction::Bt709 => SPA_VIDEO_TRANSFER_BT709,
                TransferFunction::Pq => SPA_VIDEO_TRANSFER_SMPTE2084,
                TransferFunction::Hlg => SPA_VIDEO_TRANSFER_ARIB_STD_B67,
            },
        ),
    ] {
        if value != 0 {
            props.push(Property::new(key, Value::Id(Id(value))));
        }
    }
    Ok(props)
}
pub(super) fn format_pod(
    format: VideoFormat,
    bounds: Option<VideoCaptureRange>,
) -> Result<Vec<u8>, MediaError> {
    encode(
        SPA_TYPE_OBJECT_Format,
        SPA_PARAM_EnumFormat,
        format_properties(format, bounds)?,
    )
}
pub(super) fn formats(
    config: &VideoConfig,
    gpu: &[VideoDmaBufFormat],
) -> Result<Vec<Vec<u8>>, MediaError> {
    let mut result = Vec::new();
    for format in &config.formats {
        let mut modifiers = Vec::new();
        for candidate in gpu.iter().filter(|g| g.pixel == format.pixel) {
            if !modifiers.contains(&(candidate.modifier as i64)) {
                modifiers.push(candidate.modifier as i64);
            }
        }
        if !modifiers.is_empty() {
            let mut props = format_properties(*format, config.negotiation_range(*format))?;
            props.push(Property {
                key: SPA_FORMAT_VIDEO_modifier,
                flags: PropertyFlags::MANDATORY | PropertyFlags::DONT_FIXATE,
                value: Value::Choice(ChoiceValue::Long(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Enum {
                        default: modifiers[0],
                        alternatives: modifiers,
                    },
                ))),
            });
            result.push(encode(SPA_TYPE_OBJECT_Format, SPA_PARAM_EnumFormat, props)?);
        }
    }
    for format in &config.formats {
        if config.permits_shared_memory(format.pixel) {
            result.push(format_pod(*format, config.negotiation_range(*format))?);
        }
    }
    if result.is_empty() {
        return Err(MediaError::Unsupported("no usable video transport/format combination"));
    }
    Ok(result)
}
pub(super) fn parse(pod: &Pod) -> Result<(VideoFormat, Option<u64>), MediaError> {
    parse_inner(pod, false)
}
fn parse_inner(pod: &Pod, allow_unfixed: bool) -> Result<(VideoFormat, Option<u64>), MediaError> {
    let mut info = spa::param::video::VideoInfoRaw::new();
    info.parse(pod).map_err(super::connection::native)?;
    let pixel = [
        PixelFormat::Rgba8,
        PixelFormat::Bgra8,
        PixelFormat::Rgbx8,
        PixelFormat::Bgrx8,
        PixelFormat::Rgb8,
        PixelFormat::Bgr8,
        PixelFormat::Nv12,
        PixelFormat::I420,
        PixelFormat::Yuy2,
        PixelFormat::P010,
        PixelFormat::RgbaF16,
    ]
    .into_iter()
    .find(|p| pixel(*p) == info.format().as_raw())
    .ok_or(MediaError::Unsupported("video pixel format"))?;
    if !allow_unfixed && info.flags().bits() & SPA_VIDEO_FLAG_MODIFIER_FIXATION_REQUIRED != 0 {
        return Err(MediaError::Unsupported(
            "producer must fixate the DMA-BUF modifier",
        ));
    }
    let modifier = if info
        .flags()
        .contains(spa::param::video::VideoFlags::MODIFIER)
    {
        if allow_unfixed {
            Some(info.modifier())
        } else {
            let prop = pod
                .as_object()
                .map_err(super::connection::native)?
                .props()
                .find(|p| p.key().0 == SPA_FORMAT_VIDEO_modifier)
                .ok_or(MediaError::InvalidArgument("missing modifier"))?;
            let value = super::parameters::decode(prop.value().as_bytes())
                .ok_or_else(|| {
                    let bytes = prop.value().as_bytes();
                    eprintln!("telorgon-pipewire: invalid fixed modifier POD: length={} prefix={:02x?}",
                        bytes.len(), &bytes[..bytes.len().min(64)]);
                    MediaError::InvalidArgument("fixed modifier POD")
                })?;
            let value = match value {
                super::ParameterValue::Long(value) => value,
                super::ParameterValue::Choice {
                    kind: super::ParameterChoice::Fixed | super::ParameterChoice::Enum,
                    values,
                } if values.len() == 1 => match values[0] {
                    super::ParameterValue::Long(value) => value,
                    _ => return Err(MediaError::InvalidArgument("fixed modifier type")),
                },
                _ => return Err(MediaError::InvalidArgument("modifier is not a fixed value")),
            };
            Some(value as u64)
        }
    } else {
        None
    };
    if info.interlace_mode().as_raw() != SPA_VIDEO_INTERLACE_MODE_PROGRESSIVE || info.views() > 1 {
        return Err(MediaError::Unsupported("interlaced or multiview video"));
    }
    let color = Colorimetry {
        range: match info.color_range() {
            SPA_VIDEO_COLOR_RANGE_0_255 => ColorRange::Full,
            SPA_VIDEO_COLOR_RANGE_16_235 => ColorRange::Limited,
            _ => ColorRange::Unknown,
        },
        matrix: match info.color_matrix() {
            SPA_VIDEO_COLOR_MATRIX_RGB => ColorMatrix::Rgb,
            SPA_VIDEO_COLOR_MATRIX_BT601 => ColorMatrix::Bt601,
            SPA_VIDEO_COLOR_MATRIX_BT709 => ColorMatrix::Bt709,
            SPA_VIDEO_COLOR_MATRIX_BT2020 => ColorMatrix::Bt2020,
            _ => ColorMatrix::Unknown,
        },
        primaries: match info.color_primaries() {
            SPA_VIDEO_COLOR_PRIMARIES_BT709 => ColorPrimaries::Bt709,
            SPA_VIDEO_COLOR_PRIMARIES_BT2020 => ColorPrimaries::Bt2020,
            SPA_VIDEO_COLOR_PRIMARIES_SMPTEEG432 => ColorPrimaries::DisplayP3,
            _ => ColorPrimaries::Unknown,
        },
        transfer: match info.transfer_function() {
            SPA_VIDEO_TRANSFER_GAMMA10 => TransferFunction::Linear,
            SPA_VIDEO_TRANSFER_SRGB => TransferFunction::Srgb,
            SPA_VIDEO_TRANSFER_BT709 | SPA_VIDEO_TRANSFER_BT601 | SPA_VIDEO_TRANSFER_BT2020_10 => {
                TransferFunction::Bt709
            }
            SPA_VIDEO_TRANSFER_SMPTE2084 => TransferFunction::Pq,
            SPA_VIDEO_TRANSFER_ARIB_STD_B67 => TransferFunction::Hlg,
            _ => TransferFunction::Unknown,
        },
    };
    let format = VideoFormat {
        pixel,
        width: info.size().width,
        height: info.size().height,
        rate: FrameRate {
            numerator: info.framerate().num,
            denominator: info.framerate().denom,
        }
        .reduced(),
        color,
    };
    format.validate()?;
    Ok((format, modifier))
}
pub(super) fn producer_period(pod: &Pod, fallback: FrameRate) -> Result<std::time::Duration, MediaError> {
    let mut info = spa::param::video::VideoInfoRaw::new();
    info.parse(pod).map_err(super::connection::native)?;
    let rate = info.framerate();
    let maximum = info.max_framerate();
    let rate = FrameRate { numerator: rate.num, denominator: rate.denom };
    let maximum = FrameRate { numerator: maximum.num, denominator: maximum.denom };
    let ceiling = fallback.period().ok_or(MediaError::InvalidArgument("video producer cadence"))?;
    Ok(rate.period().or_else(|| maximum.period()).unwrap_or(ceiling).max(ceiling))
}
pub(super) fn accepts(
    offer: VideoFormat,
    actual: VideoFormat,
    bounds: Option<VideoCaptureRange>,
) -> bool {
    offer.pixel == actual.pixel
        && bounds.map_or_else(
            || {
                offer.width == actual.width
                    && offer.height == actual.height
                    && offer.rate.numerator as u64 * actual.rate.denominator as u64
                        == actual.rate.numerator as u64 * offer.rate.denominator as u64
            },
            |range| range.contains(actual),
        )
        && (offer.color.range == ColorRange::Unknown || offer.color.range == actual.color.range)
        && (offer.color.matrix == ColorMatrix::Unknown || offer.color.matrix == actual.color.matrix)
        && (offer.color.primaries == ColorPrimaries::Unknown
            || offer.color.primaries == actual.color.primaries)
        && (offer.color.transfer == TransferFunction::Unknown
            || offer.color.transfer == actual.color.transfer)
}
fn range(default: i32, min: i32, max: i32) -> Value {
    Value::Choice(ChoiceValue::Int(Choice(
        ChoiceFlags::empty(),
        ChoiceEnum::Range { default, min, max },
    )))
}
pub(super) fn buffer_params(
    format: VideoFormat,
    dma_planes: Option<u32>,
) -> Result<Vec<Vec<u8>>, MediaError> {
    buffer_params_count(format, dma_planes, 16)
}
pub(super) fn buffer_params_count(
    format: VideoFormat,
    dma_planes: Option<u32>,
    count: usize,
) -> Result<Vec<Vec<u8>>, MediaError> {
    let mut params = vec![encode(
        SPA_TYPE_OBJECT_ParamBuffers,
        SPA_PARAM_Buffers,
        vec![
            Property::new(
                SPA_PARAM_BUFFERS_buffers,
                range(4.min(count as i32), 2, count as i32),
            ),
            Property::new(
                SPA_PARAM_BUFFERS_blocks,
                dma_planes.map_or_else(
                    || range(1, 1, format.plane_count() as i32),
                    |count| Value::Int(count as i32),
                ),
            ),
            Property::new(
                SPA_PARAM_BUFFERS_size,
                Value::Int(format.byte_len()? as i32),
            ),
            Property::new(
                SPA_PARAM_BUFFERS_stride,
                Value::Int(format.plane(0).unwrap().row_bytes as i32),
            ),
            Property::new(
                SPA_PARAM_BUFFERS_dataType,
                Value::Int(if dma_planes.is_some() {
                    1 << SPA_DATA_DmaBuf
                } else {
                    (1 << SPA_DATA_MemFd) | (1 << SPA_DATA_MemPtr)
                }),
            ),
        ],
    )?];
    for (kind, size) in [
        (SPA_META_Header, std::mem::size_of::<spa_meta_header>()),
        (SPA_META_VideoCrop, std::mem::size_of::<spa_meta_region>()),
        (
            SPA_META_VideoTransform,
            std::mem::size_of::<spa_meta_videotransform>(),
        ),
        (
            SPA_META_VideoDamage,
            MAX_DAMAGE_RECTS * std::mem::size_of::<spa_meta_region>(),
        ),
        (
            SPA_META_Cursor,
            std::mem::size_of::<spa_meta_cursor>()
                + std::mem::size_of::<spa_meta_bitmap>()
                + MAX_CURSOR_BYTES,
        ),
    ] {
        params.push(encode(
            SPA_TYPE_OBJECT_ParamMeta,
            SPA_PARAM_Meta,
            vec![
                Property::new(SPA_PARAM_META_type, Value::Id(Id(kind))),
                Property::new(SPA_PARAM_META_size, Value::Int(size as i32)),
            ],
        )?);
    }
    Ok(params)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_modifier_accepts_retained_choice_alternatives() {
        let format = VideoFormat::rgba(1920, 1080, 30);
        let mut props = format_properties(format, None).unwrap();
        props.push(Property {
            key: SPA_FORMAT_VIDEO_modifier,
            flags: PropertyFlags::MANDATORY,
            value: Value::Choice(ChoiceValue::Long(Choice(
                ChoiceFlags::empty(),
                ChoiceEnum::Enum { default: 7, alternatives: vec![7, 11] },
            ))),
        });
        let mut bytes = encode(SPA_TYPE_OBJECT_Format, SPA_PARAM_Format, props).unwrap();
        // Match spa_pod_object_fixate: change only the choice kind, keeping
        // the payload and its original length intact.
        let offset = {
            let pod = Pod::from_bytes(&bytes).unwrap();
            let prop = pod.as_object().unwrap().props()
                .find(|p| p.key().0 == SPA_FORMAT_VIDEO_modifier).unwrap();
            prop.value().as_bytes().as_ptr() as usize - bytes.as_ptr() as usize
        };
        assert!(parse(Pod::from_bytes(&bytes).unwrap()).is_err());
        bytes[offset + 8..offset + 12].copy_from_slice(&SPA_CHOICE_None.to_ne_bytes());
        assert_eq!(parse(Pod::from_bytes(&bytes).unwrap()).unwrap(), (format, Some(7)));
    }
    #[test]
    fn screen_producer_accepts_webrtc_lower_rate_without_changing_resolution() {
        let offered = VideoFormat::rgba(1920, 1080, 60);
        let consumer = VideoFormat::rgba(1920, 1080, 30);
        let mut config = VideoConfig::producer(offered);
        assert!(!accepts(offered, consumer, config.negotiation_range(offered)));
        config.allow_lower_frame_rate = true;
        config.validate().unwrap();
        assert!(accepts(offered, consumer, config.negotiation_range(offered)));
        let variable = VideoFormat::rgba(1920, 1080, 0);
        assert!(accepts(offered, variable, config.negotiation_range(offered)));
        let mut props = format_properties(variable, None).unwrap();
        props.push(Property::new(SPA_FORMAT_VIDEO_maxFramerate, Value::Fraction(Fraction { num: 30, denom: 1 })));
        let bytes = encode(SPA_TYPE_OBJECT_Format, SPA_PARAM_Format, props).unwrap();
        assert_eq!(producer_period(Pod::from_bytes(&bytes).unwrap(), offered.rate).unwrap(), FrameRate::hz(30).period().unwrap());
        assert!(!accepts(offered, VideoFormat::rgba(1280, 720, 30), config.negotiation_range(offered)));
        assert!(!accepts(offered, VideoFormat::rgba(1920, 1080, 120), config.negotiation_range(offered)));
        let offers = formats(&config, &[]).unwrap();
        let pod = super::super::parameters::decode(&offers[0]).unwrap();
        assert!(matches!(pod.property(SPA_FORMAT_VIDEO_framerate), Some(super::super::ParameterValue::Choice { .. })));
        // Fixing a GPU modifier at the consumer's selected rate must retain that rate.
        let fixed = fixed_format(consumer, 0).unwrap();
        let (actual, _) = parse(Pod::from_bytes(&fixed).unwrap()).unwrap();
        assert!(accepts(offered, actual, config.negotiation_range(offered)));
    }
    #[test]
    fn gpu_offers_are_preferred_but_keep_cpu_fallback_and_exact_memory_types() {
        let format = VideoFormat::rgba(64, 48, 120);
        let config = VideoConfig::producer(format);
        let offers = formats(
            &config,
            &[
                VideoDmaBufFormat {
                    pixel: PixelFormat::Rgba8,
                    modifier: 0,
                    planes: 1,
                },
                VideoDmaBufFormat {
                    pixel: PixelFormat::Rgba8,
                    modifier: 7,
                    planes: 1,
                },
            ],
        )
        .unwrap();
        assert_eq!(offers.len(), 2);
        assert_eq!(
            fixation(Pod::from_bytes(&offers[0]).unwrap()).unwrap(),
            Some((format, vec![0, 7]))
        );
        let fixed_offer = fixed_format(format, 7).unwrap();
        let fixed_value = super::super::parameters::decode(&fixed_offer).unwrap();
        assert_eq!(
            fixed_value.property(SPA_FORMAT_VIDEO_modifier),
            Some(&super::super::ParameterValue::Long(7))
        );
        assert_eq!(
            parse(Pod::from_bytes(&fixed_offer).unwrap()).unwrap(),
            (format, Some(7))
        );
        assert!(
            fixation(Pod::from_bytes(&fixed_offer).unwrap())
                .unwrap()
                .is_none()
        );
        let gpu = super::super::parameters::decode(&offers[0]).unwrap();
        assert!(
            matches!(gpu.property(SPA_FORMAT_VIDEO_modifier),Some(super::super::ParameterValue::Choice {values,..}) if values.len()==3)
        );
        let cpu = Pod::from_bytes(&offers[1]).unwrap();
        assert_eq!(parse(cpu).unwrap(), (format, None));
        // A consumer does not accept a producer's still-unfixed modifier list.
        assert!(parse(Pod::from_bytes(&offers[0]).unwrap()).is_err());
        let mut fixed = format_properties(format, None).unwrap();
        fixed.push(Property {
            key: SPA_FORMAT_VIDEO_modifier,
            flags: PropertyFlags::MANDATORY,
            value: Value::Long(0),
        });
        let bytes = encode(SPA_TYPE_OBJECT_Format, SPA_PARAM_Format, fixed).unwrap();
        assert_eq!(
            parse(Pod::from_bytes(&bytes).unwrap()).unwrap(),
            (format, Some(0))
        );
        for (planes, expected) in [
            (None, (1 << SPA_DATA_MemFd) | (1 << SPA_DATA_MemPtr)),
            (Some(1), 1 << SPA_DATA_DmaBuf),
        ] {
            let params = buffer_params(format, planes).unwrap();
            let value = super::super::parameters::decode(&params[0]).unwrap();
            assert_eq!(
                value.property(SPA_PARAM_BUFFERS_dataType),
                Some(&super::super::ParameterValue::Int(expected))
            );
        }
    }
}

/// Only deserialize the bounded modifier value, never arbitrary nested peer properties.
pub(super) fn fixation(pod: &Pod) -> Result<Option<(VideoFormat, Vec<u64>)>, MediaError> {
    if pod.as_bytes().len() > 16384 {
        return Err(MediaError::ResourceLimit("video format POD"));
    }
    let object = pod.as_object().map_err(super::connection::native)?;
    let Some(prop) = object
        .props()
        .find(|p| p.key().0 == SPA_FORMAT_VIDEO_modifier)
    else {
        return Ok(None);
    };
    if !prop.flags().contains(spa::pod::PodPropFlags::DONT_FIXATE) {
        return Ok(None);
    }
    let bytes = prop.value().as_bytes();
    // Choice header: pod header, choice kind/flags, then child size/type.
    if bytes.len() < 24
        || u32::from_ne_bytes(bytes[4..8].try_into().unwrap()) != SPA_TYPE_Choice
        || u32::from_ne_bytes(bytes[16..20].try_into().unwrap()) != 8
        || u32::from_ne_bytes(bytes[20..24].try_into().unwrap()) != SPA_TYPE_Long
        || bytes.len() > 24 + 257 * 8
    {
        return Err(MediaError::InvalidArgument("modifier choices"));
    }
    let (_, value) = spa::pod::deserialize::PodDeserializer::deserialize_any_from(bytes)
        .map_err(|_| MediaError::InvalidArgument("modifier POD"))?;
    let Value::Choice(ChoiceValue::Long(Choice(
        _,
        ChoiceEnum::Enum {
            default,
            alternatives,
        },
    ))) = value
    else {
        return Err(MediaError::InvalidArgument("modifier enumeration"));
    };
    let mut modifiers = vec![default as u64];
    for modifier in alternatives {
        if !modifiers.contains(&(modifier as u64)) {
            modifiers.push(modifier as u64);
        }
    }
    Ok(Some((parse_inner(pod, true)?.0, modifiers)))
}
pub(super) fn fixed_format(format: VideoFormat, modifier: u64) -> Result<Vec<u8>, MediaError> {
    let mut properties = format_properties(format, None)?;
    properties.push(Property {
        key: SPA_FORMAT_VIDEO_modifier,
        flags: PropertyFlags::MANDATORY,
        // An Enum containing only its default has no valid alternatives in SPA.
        // Publish the allocator's decision as a scalar to complete fixation.
        value: Value::Long(modifier as i64),
    });
    encode(SPA_TYPE_OBJECT_Format, SPA_PARAM_EnumFormat, properties)
}

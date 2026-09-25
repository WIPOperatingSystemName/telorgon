//! Metadata regions are copied after size/offset validation; no native references escape.
use super::MediaError;
use crate::media::video::*;
use pipewire::spa::sys::*;
use std::mem::size_of;
unsafe fn region(buffer: &spa_buffer, kind: u32) -> Option<(*mut u8, usize)> {
    if buffer.n_metas > 64 || buffer.metas.is_null() {
        return None;
    }
    for index in 0..buffer.n_metas as usize {
        // SAFETY: caller guarantees native array validity during the buffer lease.
        let meta = unsafe { &*buffer.metas.add(index) };
        if meta.type_ == kind && !meta.data.is_null() && meta.size <= 1024 * 1024 {
            return Some((meta.data.cast(), meta.size as usize));
        }
    }
    None
}
unsafe fn read_at<T: Copy>(data: *mut u8, len: usize, offset: usize) -> Option<T> {
    if offset.checked_add(size_of::<T>())? > len {
        return None;
    }
    // SAFETY: checked region and POD structs containing only integers; external alignment
    // is not assumed. Caller guarantees the source region remains readable for this call.
    Some(unsafe { data.add(offset).cast::<T>().read_unaligned() })
}
unsafe fn put<T: Copy>(
    data: *mut u8,
    len: usize,
    offset: usize,
    value: T,
) -> Result<(), MediaError> {
    if offset
        .checked_add(size_of::<T>())
        .is_none_or(|end| end > len)
    {
        return Err(MediaError::InvalidArgument("video metadata size"));
    }
    // SAFETY: exclusive native buffer lease and checked byte range, no alignment requirement.
    unsafe {
        data.add(offset).cast::<T>().write_unaligned(value);
    }
    Ok(())
}
fn rect(region: spa_region) -> Option<VideoRect> {
    Some(VideoRect {
        x: region.position.x.try_into().ok()?,
        y: region.position.y.try_into().ok()?,
        width: region.size.width,
        height: region.size.height,
    })
}
fn native_rect(rect: VideoRect) -> spa_meta_region {
    spa_meta_region {
        region: spa_region {
            position: spa_point {
                x: rect.x as i32,
                y: rect.y as i32,
            },
            size: spa_rectangle {
                width: rect.width,
                height: rect.height,
            },
        },
    }
}
pub(super) unsafe fn read(
    buffer: &spa_buffer,
    format: VideoFormat,
) -> Result<FrameMetadata, MediaError> {
    let mut metadata = FrameMetadata::default();
    let bad = || MediaError::InvalidArgument("video metadata");
    if let Some((data, len)) = unsafe { region(buffer, SPA_META_Header) } {
        let h = unsafe { read_at::<spa_meta_header>(data, len, 0) }.ok_or_else(bad)?;
        if h.flags & (SPA_META_HEADER_FLAG_CORRUPTED | SPA_META_HEADER_FLAG_GAP) != 0 {
            return Err(bad());
        }
        metadata.sequence = Some(h.seq);
        metadata.timestamp_ns = (h.pts >= 0).then_some(h.pts);
        metadata.discontinuity = h.flags & SPA_META_HEADER_FLAG_DISCONT != 0;
    }
    if let Some((data, len)) = unsafe { region(buffer, SPA_META_VideoCrop) } {
        let r = unsafe { read_at::<spa_meta_region>(data, len, 0) }.ok_or_else(bad)?;
        if r.region.size.width != 0 && r.region.size.height != 0 {
            metadata.crop = Some(rect(r.region).ok_or_else(bad)?);
        }
    }
    if let Some((data, len)) = unsafe { region(buffer, SPA_META_VideoTransform) } {
        let t = unsafe { read_at::<spa_meta_videotransform>(data, len, 0) }.ok_or_else(bad)?;
        metadata.transform = match t.transform {
            0 => VideoTransform::Normal,
            1 => VideoTransform::Rotate90,
            2 => VideoTransform::Rotate180,
            3 => VideoTransform::Rotate270,
            4 => VideoTransform::Flipped,
            5 => VideoTransform::Flipped90,
            6 => VideoTransform::Flipped180,
            7 => VideoTransform::Flipped270,
            _ => return Err(bad()),
        };
    }
    if let Some((data, len)) = unsafe { region(buffer, SPA_META_VideoDamage) } {
        let count = len / size_of::<spa_meta_region>();
        for index in 0..count.min(MAX_DAMAGE_RECTS) {
            let r = unsafe {
                read_at::<spa_meta_region>(data, len, index * size_of::<spa_meta_region>())
            }
            .ok_or_else(bad)?;
            if r.region.size.width == 0 || r.region.size.height == 0 {
                break;
            }
            metadata.damage.push(rect(r.region).ok_or_else(bad)?);
        }
        // A larger source array might contain further changed regions. Full damage is
        // conservative; silently truncating would leave stale pixels in partial renderers.
        if count > MAX_DAMAGE_RECTS && metadata.damage.len() == MAX_DAMAGE_RECTS {
            metadata.damage.clear();
        }
    }
    if let Some((data, len)) = unsafe { region(buffer, SPA_META_Cursor) } {
        let c = unsafe { read_at::<spa_meta_cursor>(data, len, 0) }.ok_or_else(bad)?;
        if c.id != 0 {
            let mut cursor = VideoCursor {
                id: c.id,
                x: c.position.x,
                y: c.position.y,
                hotspot_x: c.hotspot.x,
                hotspot_y: c.hotspot.y,
                bitmap: None,
                visible: None,
            };
            if c.bitmap_offset != 0 {
                if (c.bitmap_offset as usize) < size_of::<spa_meta_cursor>() {
                    return Err(bad());
                }
                let bitmap =
                    unsafe { read_at::<spa_meta_bitmap>(data, len, c.bitmap_offset as usize) }
                        .ok_or_else(bad)?;
                if bitmap.format != 0 {
                    cursor.visible = Some(bitmap.offset != 0);
                    if bitmap.offset != 0 {
                        if (bitmap.offset as usize) < size_of::<spa_meta_bitmap>()
                            || bitmap.size.width == 0
                            || bitmap.size.height == 0
                            || bitmap.size.width > 256
                            || bitmap.size.height > 256
                            || bitmap.stride < (bitmap.size.width * 4) as i32
                        {
                            return Err(bad());
                        }
                        let start = (c.bitmap_offset as usize)
                            .checked_add(bitmap.offset as usize)
                            .ok_or_else(bad)?;
                        let bytes = (bitmap.stride as usize)
                            .checked_mul(bitmap.size.height as usize - 1)
                            .and_then(|n| n.checked_add(bitmap.size.width as usize * 4))
                            .ok_or_else(bad)?;
                        if start.checked_add(bytes).is_none_or(|end| end > len) {
                            return Err(bad());
                        }
                        let mut rgba =
                            vec![0; bitmap.size.width as usize * bitmap.size.height as usize * 4];
                        for row in 0..bitmap.size.height as usize {
                            // SAFETY: bitmap bounds and positive stride checked above.
                            let source = unsafe {
                                std::slice::from_raw_parts(
                                    data.add(start + row * bitmap.stride as usize),
                                    bitmap.size.width as usize * 4,
                                )
                            };
                            for (src, dst) in source.chunks_exact(4).zip(
                                rgba[row * source.len()..(row + 1) * source.len()]
                                    .chunks_exact_mut(4),
                            ) {
                                let pixel = match bitmap.format {
                                    SPA_VIDEO_FORMAT_RGBA => [src[0], src[1], src[2], src[3]],
                                    SPA_VIDEO_FORMAT_BGRA => [src[2], src[1], src[0], src[3]],
                                    SPA_VIDEO_FORMAT_ARGB => [src[1], src[2], src[3], src[0]],
                                    SPA_VIDEO_FORMAT_ABGR => [src[3], src[2], src[1], src[0]],
                                    _ => {
                                        return Err(MediaError::Unsupported("cursor pixel format"));
                                    }
                                };
                                dst.copy_from_slice(&pixel);
                            }
                        }
                        cursor.bitmap = Some(CursorBitmap {
                            width: bitmap.size.width,
                            height: bitmap.size.height,
                            rgba,
                        });
                    }
                }
            }
            metadata.cursor = Some(cursor);
        }
    }
    metadata.validate(format)?;
    Ok(metadata)
}
#[cfg(test)]
pub(super) unsafe fn write(
    buffer: &spa_buffer,
    format: VideoFormat,
    metadata: Option<&FrameMetadata>,
    sequence: u64,
) -> Result<(), MediaError> {
    // SAFETY: forwarded buffer ownership and metadata bounds are unchanged.
    unsafe { write_with_damage(buffer, format, metadata, sequence, false) }
}
pub(super) unsafe fn write_with_damage(
    buffer: &spa_buffer,
    format: VideoFormat,
    metadata: Option<&FrameMetadata>,
    sequence: u64,
    full_damage: bool,
) -> Result<(), MediaError> {
    let empty = FrameMetadata::default();
    let m = metadata.unwrap_or(&empty);
    if let Some((data, len)) = unsafe { region(buffer, SPA_META_Header) } {
        let value = spa_meta_header {
            flags: if metadata.is_none() {
                SPA_META_HEADER_FLAG_GAP
            } else if m.discontinuity || full_damage {
                SPA_META_HEADER_FLAG_DISCONT
            } else {
                0
            },
            offset: 0,
            pts: m
                .timestamp_ns
                .or_else(|| metadata.and_then(|_| monotonic_ns()))
                .unwrap_or(-1),
            dts_offset: 0,
            seq: sequence,
        };
        unsafe { put(data, len, 0, value) }?;
    }
    if let Some((data, len)) = unsafe { region(buffer, SPA_META_VideoCrop) } {
        unsafe {
            put(
                data,
                len,
                0,
                native_rect(m.crop.unwrap_or(VideoRect {
                    x: 0,
                    y: 0,
                    width: format.width,
                    height: format.height,
                })),
            )
        }?;
    }
    if let Some((data, len)) = unsafe { region(buffer, SPA_META_VideoTransform) } {
        unsafe {
            put(
                data,
                len,
                0,
                spa_meta_videotransform {
                    transform: m.transform as u32,
                },
            )
        }?;
    }
    if let Some((data, len)) = unsafe { region(buffer, SPA_META_VideoDamage) } {
        // SAFETY: exclusively owned bounded metadata region; clears old damage sentinels.
        unsafe {
            std::ptr::write_bytes(data, 0, len);
        }
        if full_damage || m.damage.is_empty() {
            unsafe {
                put(
                    data,
                    len,
                    0,
                    native_rect(VideoRect {
                        x: 0,
                        y: 0,
                        width: format.width,
                        height: format.height,
                    }),
                )
            }?;
        } else {
            for (i, r) in m.damage.iter().enumerate() {
                unsafe { put(data, len, i * size_of::<spa_meta_region>(), native_rect(*r)) }?;
            }
        }
    }
    if let Some((data, len)) = unsafe { region(buffer, SPA_META_Cursor) } {
        // SAFETY: clears previous bitmap and update flags in this exclusive region.
        unsafe {
            std::ptr::write_bytes(data, 0, len);
        }
        if let Some(c) = &m.cursor {
            let bitmap_offset = if c.bitmap.is_some() || c.visible == Some(false) {
                size_of::<spa_meta_cursor>() as u32
            } else {
                0
            };
            unsafe {
                put(
                    data,
                    len,
                    0,
                    spa_meta_cursor {
                        id: c.id,
                        flags: 0,
                        position: spa_point { x: c.x, y: c.y },
                        hotspot: spa_point {
                            x: c.hotspot_x,
                            y: c.hotspot_y,
                        },
                        bitmap_offset,
                    },
                )
            }?;
            if bitmap_offset != 0 {
                let (w, h, pixels) = c
                    .bitmap
                    .as_ref()
                    .map(|b| (b.width, b.height, b.rgba.as_slice()))
                    .unwrap_or((0, 0, &[]));
                let offset = if c.visible == Some(false) {
                    0
                } else {
                    size_of::<spa_meta_bitmap>() as u32
                };
                unsafe {
                    put(
                        data,
                        len,
                        bitmap_offset as usize,
                        spa_meta_bitmap {
                            format: SPA_VIDEO_FORMAT_RGBA,
                            size: spa_rectangle {
                                width: w,
                                height: h,
                            },
                            stride: (w * 4) as i32,
                            offset,
                        },
                    )
                }?;
                if offset != 0 {
                    let start = bitmap_offset as usize + offset as usize;
                    if start + pixels.len() > len {
                        return Err(MediaError::InvalidArgument("cursor metadata size"));
                    }
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            pixels.as_ptr(),
                            data.add(start),
                            pixels.len(),
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

fn monotonic_ns() -> Option<i64> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: valid writable timespec on Linux, no borrowed native frame storage.
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) } != 0 {
        return None;
    }
    (time.tv_sec as i64)
        .checked_mul(1_000_000_000)?
        .checked_add(time.tv_nsec as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lost_producer_history_overrides_partial_damage_and_marks_discontinuity() {
        let mut header = vec![0u8; size_of::<spa_meta_header>()];
        let mut damage = vec![0u8; size_of::<spa_meta_region>() * 2];
        let mut metas = [
            spa_meta {
                type_: SPA_META_Header,
                size: header.len() as u32,
                data: header.as_mut_ptr().cast(),
            },
            spa_meta {
                type_: SPA_META_VideoDamage,
                size: damage.len() as u32,
                data: damage.as_mut_ptr().cast(),
            },
        ];
        let buffer = spa_buffer {
            n_metas: metas.len() as u32,
            n_datas: 0,
            metas: metas.as_mut_ptr(),
            datas: std::ptr::null_mut(),
        };
        let format = VideoFormat::rgba(64, 48, 60);
        let metadata = FrameMetadata {
            damage: vec![VideoRect {
                x: 2,
                y: 4,
                width: 6,
                height: 8,
            }],
            ..Default::default()
        };
        // SAFETY: exclusive live arrays provide precisely sized native metadata regions.
        unsafe { write_with_damage(&buffer, format, Some(&metadata), 7, true) }.unwrap();
        let full = unsafe { read(&buffer, format) }.unwrap();
        assert!(full.discontinuity);
        assert_eq!(
            full.damage,
            vec![VideoRect {
                x: 0,
                y: 0,
                width: 64,
                height: 48
            }]
        );
        unsafe { write_with_damage(&buffer, format, Some(&metadata), 8, false) }.unwrap();
        let partial = unsafe { read(&buffer, format) }.unwrap();
        assert!(!partial.discontinuity);
        assert_eq!(partial.damage, metadata.damage);
    }
    #[test]
    fn metadata_roundtrip_and_cursor_offsets_are_bounded() {
        let format = VideoFormat::rgba(64, 48, 120);
        let mut header = vec![0u8; size_of::<spa_meta_header>()];
        let mut cursor =
            vec![0u8; size_of::<spa_meta_cursor>() + size_of::<spa_meta_bitmap>() + 16];
        let mut crop = vec![0u8; size_of::<spa_meta_region>()];
        let mut transform = vec![0u8; size_of::<spa_meta_videotransform>()];
        let mut damage = vec![0u8; size_of::<spa_meta_region>() * 2];
        let mut regions = [
            spa_meta {
                type_: SPA_META_Header,
                size: header.len() as u32,
                data: header.as_mut_ptr().cast(),
            },
            spa_meta {
                type_: SPA_META_Cursor,
                size: cursor.len() as u32,
                data: cursor.as_mut_ptr().cast(),
            },
            spa_meta {
                type_: SPA_META_VideoCrop,
                size: crop.len() as u32,
                data: crop.as_mut_ptr().cast(),
            },
            spa_meta {
                type_: SPA_META_VideoTransform,
                size: transform.len() as u32,
                data: transform.as_mut_ptr().cast(),
            },
            spa_meta {
                type_: SPA_META_VideoDamage,
                size: damage.len() as u32,
                data: damage.as_mut_ptr().cast(),
            },
        ];
        let buffer = spa_buffer {
            n_metas: regions.len() as u32,
            n_datas: 0,
            metas: regions.as_mut_ptr(),
            datas: std::ptr::null_mut(),
        };
        let m = FrameMetadata {
            timestamp_ns: Some(42),
            crop: Some(VideoRect {
                x: 4,
                y: 8,
                width: 10,
                height: 12,
            }),
            transform: VideoTransform::Flipped270,
            cursor: Some(VideoCursor {
                id: 1,
                x: 2,
                y: 3,
                hotspot_x: 0,
                hotspot_y: 0,
                bitmap: Some(CursorBitmap {
                    width: 2,
                    height: 2,
                    rgba: vec![99; 16],
                }),
                visible: Some(true),
            }),
            ..Default::default()
        };
        // SAFETY: metadata regions above are exclusive, correctly sized live allocations.
        unsafe { write(&buffer, format, Some(&m), 7) }.unwrap();
        let actual = unsafe { read(&buffer, format) }.unwrap();
        assert_eq!(actual.sequence, Some(7));
        assert_eq!(actual.timestamp_ns, Some(42));
        assert_eq!(actual.crop, m.crop);
        assert_eq!(actual.transform, m.transform);
        assert_eq!(actual.cursor.unwrap().bitmap.unwrap().rgba, vec![99; 16]);
        let mut c =
            unsafe { read_at::<spa_meta_cursor>(cursor.as_mut_ptr(), cursor.len(), 0) }.unwrap();
        c.bitmap_offset = u32::MAX;
        unsafe { put(cursor.as_mut_ptr(), cursor.len(), 0, c) }.unwrap();
        assert!(unsafe { read(&buffer, format) }.is_err());
    }
    #[test]
    fn oversized_damage_arrays_fall_back_to_full_damage() {
        let mut regions = vec![
            native_rect(VideoRect {
                x: 0,
                y: 0,
                width: 1,
                height: 1
            });
            MAX_DAMAGE_RECTS + 1
        ];
        let mut meta = spa_meta {
            type_: SPA_META_VideoDamage,
            size: (regions.len() * size_of::<spa_meta_region>()) as u32,
            data: regions.as_mut_ptr().cast(),
        };
        let buffer = spa_buffer {
            n_metas: 1,
            n_datas: 0,
            metas: &mut meta,
            datas: std::ptr::null_mut(),
        };
        // SAFETY: the metadata array and all its integer POD records remain alive.
        assert!(
            unsafe { read(&buffer, VideoFormat::rgba(8, 8, 30)) }
                .unwrap()
                .damage
                .is_empty()
        );
    }
}

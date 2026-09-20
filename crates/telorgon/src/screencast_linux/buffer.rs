//! The small mutable SPA boundary missing from pipewire-rs's read-only metadata API.
//! A dequeued buffer is returned exactly once, including on validation failure.

use super::{CaptureLayout, pw, spa};
use std::ptr::NonNull;

struct Dequeued<'a> {
    raw: NonNull<pw::sys::pw_buffer>,
    stream: &'a pw::stream::Stream,
}
impl Drop for Dequeued<'_> {
    fn drop(&mut self) {
        // SAFETY: this guard exclusively owns one buffer dequeued from this stream. No pointer
        // or slice obtained from it escapes `deliver`, and Drop runs on that same loop thread.
        unsafe {
            self.stream.queue_raw_buffer(self.raw.as_ptr());
        }
    }
}

pub(super) fn deliver(
    stream: &pw::stream::Stream,
    layout: CaptureLayout,
    pixels: Option<&[u8]>,
    sequence: u64,
    accepts: impl FnOnce(usize) -> bool,
) {
    // SAFETY: called only by this stream's process callback; no RT_PROCESS flag is used.
    let Some(raw) = NonNull::new(unsafe { stream.dequeue_raw_buffer() }) else {
        return;
    };
    let pixels = if accepts(raw.as_ptr() as usize) {
        pixels
    } else {
        None
    };
    let mut buffer = Dequeued { raw, stream };
    // SAFETY: the dequeued pw_buffer remains exclusively owned by this guard until requeue.
    let raw = unsafe { buffer.raw.as_mut() };
    let Some(mut spa_buffer) = NonNull::new(raw.buffer) else {
        return;
    };
    let pts = monotonic_ns();
    // SAFETY: PipeWire provides valid SPA arrays/mappings for the lifetime of a dequeued buffer.
    // write_frame checks format-independent counts and sizes before accessing its data plane.
    let written = unsafe { write_frame(spa_buffer.as_mut(), layout, pixels, sequence, pts) };
    // Video size is expressed in frames, not bytes (the SPA chunk contains the byte size).
    raw.size = u64::from(written);
}

fn monotonic_ns() -> Option<i64> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `time` is valid writable storage and CLOCK_MONOTONIC is supported on Linux.
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) } != 0 {
        return None;
    }
    time.tv_sec
        .checked_mul(1_000_000_000)?
        .checked_add(time.tv_nsec)
}

/// Caller guarantees exclusively owned, valid SPA arrays and mapped plane memory.
unsafe fn write_frame(
    buffer: &mut spa::sys::spa_buffer,
    layout: CaptureLayout,
    pixels: Option<&[u8]>,
    sequence: u64,
    pts: Option<i64>,
) -> bool {
    let mut valid = false;
    if buffer.n_datas == 1 && !buffer.datas.is_null() {
        // SAFETY: caller supplies the SPA data array; n_datas was checked above.
        let data = unsafe { &mut *buffer.datas };
        if !data.chunk.is_null() {
            // SAFETY: a valid SPA plane has a chunk for this buffer's lifetime.
            let chunk = unsafe { &mut *data.chunk };
            chunk.offset = 0;
            chunk.stride = layout.stride() as i32;
            chunk.size = 0;
            chunk.flags = spa::sys::SPA_CHUNK_FLAG_CORRUPTED as i32;
            if matches!(
                data.type_,
                spa::sys::SPA_DATA_MemFd | spa::sys::SPA_DATA_MemPtr
            ) && !data.data.is_null()
                && data.maxsize as usize >= layout.byte_len()
                && pts.is_some()
                && let Some(pixels) = pixels.filter(|p| p.len() == layout.byte_len())
            {
                // SAFETY: both ranges have layout.byte_len bytes. Producer Vec storage is
                // independent of PipeWire's negotiated mapped delivery planes.
                unsafe {
                    std::ptr::copy_nonoverlapping(pixels.as_ptr(), data.data.cast(), pixels.len());
                }
                chunk.size = layout.byte_len() as u32;
                chunk.flags = 0;
                valid = true;
            }
        }
    }
    if buffer.n_metas > 0 && !buffer.metas.is_null() {
        // SAFETY: the caller supplies a valid metadata array. The SPA helper checks type and size.
        let header = unsafe {
            spa::sys::spa_buffer_find_meta_data(
                buffer,
                spa::sys::SPA_META_Header,
                std::mem::size_of::<spa::sys::spa_meta_header>(),
            )
        }
        .cast::<spa::sys::spa_meta_header>();
        if !header.is_null() {
            // SAFETY: the helper verified the metadata region's size. Unaligned write avoids
            // imposing an extra Rust alignment assumption on an external metadata region.
            unsafe {
                header.write_unaligned(spa::sys::spa_meta_header {
                    flags: if valid {
                        0
                    } else {
                        spa::sys::SPA_META_HEADER_FLAG_CORRUPTED
                    },
                    offset: 0,
                    pts: pts.unwrap_or(-1),
                    dts_offset: 0,
                    seq: sequence,
                });
            }
        }
    }
    valid
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroU32;

    #[test]
    fn frame_delivery_sets_timestamp_stride_sequence_and_rejects_short_planes() {
        use spa::sys::*;
        let layout =
            CaptureLayout::rgba8(NonZeroU32::new(2).unwrap(), NonZeroU32::new(1).unwrap(), 8)
                .unwrap();
        let mut storage = [0xaau8; 12];
        let mut chunk = spa_chunk {
            offset: 99,
            size: 99,
            stride: 99,
            flags: 99,
        };
        let mut plane = spa_data {
            type_: SPA_DATA_MemFd,
            flags: 0,
            fd: -1,
            mapoffset: 0,
            maxsize: 8,
            data: storage.as_mut_ptr().cast(),
            chunk: &mut chunk,
        };
        let mut header = spa_meta_header {
            flags: 99,
            offset: 99,
            pts: 99,
            dts_offset: 99,
            seq: 99,
        };
        let mut meta = spa_meta {
            type_: SPA_META_Header,
            size: std::mem::size_of::<spa_meta_header>() as u32,
            data: (&mut header as *mut spa_meta_header).cast(),
        };
        let mut buffer = spa_buffer {
            n_metas: 1,
            n_datas: 1,
            metas: &mut meta,
            datas: &mut plane,
        };
        // SAFETY: every pointer above addresses live exclusive local storage of the declared size.
        assert!(unsafe { write_frame(&mut buffer, layout, Some(&[1; 8]), 7, Some(1_000)) });
        assert_eq!(storage, [1, 1, 1, 1, 1, 1, 1, 1, 0xaa, 0xaa, 0xaa, 0xaa]);
        assert_eq!((chunk.size, chunk.stride, chunk.offset), (8, 8, 0));
        assert_eq!(
            (
                header.pts,
                header.seq,
                header.flags,
                header.offset,
                header.dts_offset
            ),
            (1_000, 7, 0, 0, 0)
        );
        plane.maxsize = 7;
        assert!(!unsafe { write_frame(&mut buffer, layout, Some(&[2; 8]), 8, Some(2_000)) });
        assert_eq!(chunk.size, 0);
        assert_eq!(header.flags, SPA_META_HEADER_FLAG_CORRUPTED);
        assert_eq!(storage[0], 1);
        plane.maxsize = 8;
        assert!(!unsafe { write_frame(&mut buffer, layout, None, 9, Some(3_000)) });
        assert!(!unsafe { write_frame(&mut buffer, layout, Some(&[2; 8]), 10, None) });
        assert_eq!(chunk.size, 0);
    }
}

//! Bounded cursor snapshots for separately negotiated screen metadata.
use super::*;
use crate::{
    foundation::{PointF, PointI, SizeI},
    host::linux_shell::pointer_visual::{CursorVisual, cursor_image_signature},
    media::video::{CursorBitmap, VideoCursor},
    platform::contracts::ScaleFactor,
    shell::capture::CaptureCursorMode,
};

#[derive(Default)]
pub(super) struct CursorState {
    image: Option<((u64, u32), CursorBitmap, PointI)>,
    position: PointI,
    revision: u64,
}
impl CursorState {
    pub(super) fn revision(&self) -> u64 {
        self.revision
    }
    fn update(
        &mut self,
        cursor: Option<&CursorVisual>,
        position: PointF,
        scale: ScaleFactor,
    ) -> AppResult<()> {
        let Some(cursor) = cursor.and_then(CursorVisual::image) else {
            if self.image.take().is_some() {
                self.revision = self.revision.wrapping_add(1).max(1);
            }
            return Ok(());
        };
        let physical = scale.physical_point(position);
        if !physical.x.is_finite() || !physical.y.is_finite() || cursor.rgba.len() > 4 * 1024 * 1024
        {
            return Err(AppError::new(
                "cursor metadata exceeds geometry or source bitmap bounds",
            ));
        }
        let position = PointI {
            x: physical.x.round() as i32,
            y: physical.y.round() as i32,
        };
        let signature = (cursor_image_signature(cursor), scale.get().to_bits());
        let changed = self
            .image
            .as_ref()
            .is_none_or(|(old, _, _)| *old != signature);
        if changed {
            let mut image = cursor.for_hardware(
                scale,
                SizeI {
                    width: 256,
                    height: 256,
                },
            )?;
            // Screen metadata uses the same premultiplied RGBA convention as Wayland cursors.
            if !image.premultiplied {
                for pixel in image.rgba.chunks_exact_mut(4) {
                    let alpha = u16::from(pixel[3]);
                    for channel in &mut pixel[..3] {
                        *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
                    }
                }
            }
            self.image = Some((
                signature,
                CursorBitmap {
                    width: image.size.width as u32,
                    height: image.size.height as u32,
                    rgba: image.rgba,
                },
                image.hotspot,
            ));
        }
        if changed || position != self.position {
            self.revision = self.revision.wrapping_add(1).max(1);
        }
        self.position = position;
        Ok(())
    }
    pub(super) fn for_source(&self, origin: PointI, layout: CaptureLayout) -> VideoCursor {
        let hidden = VideoCursor {
            id: 1,
            x: 0,
            y: 0,
            hotspot_x: 0,
            hotspot_y: 0,
            bitmap: None,
            visible: Some(false),
        };
        let Some((_, bitmap, hotspot)) = &self.image else {
            return hidden;
        };
        let Some(x) = self.position.x.checked_sub(origin.x) else {
            return hidden;
        };
        let Some(y) = self.position.y.checked_sub(origin.y) else {
            return hidden;
        };
        // Do not disclose pointer coordinates outside the approved source. A receiver can
        // clip the bitmap itself when the hotspot is inside but the image overlaps an edge.
        if x < 0 || y < 0 || x as u32 >= layout.width() || y as u32 >= layout.height() {
            return hidden;
        }
        VideoCursor {
            id: 1,
            x,
            y,
            hotspot_x: hotspot.x,
            hotspot_y: hotspot.y,
            bitmap: Some(bitmap.clone()),
            visible: Some(true),
        }
    }
}
impl CaptureStreams {
    pub(in crate::host::linux_shell) fn clear_cursor_metadata(&mut self) {
        self.cursor = CursorState::default();
    }
    pub(in crate::host::linux_shell) fn update_cursor_metadata(
        &mut self,
        sessions: &mut CaptureSessions,
        cursor: Option<&CursorVisual>,
        position: PointF,
        scale: ScaleFactor,
    ) {
        let needed = self.streams.keys().any(|id| {
            sessions.get(*id).is_some_and(|s| {
                s.state == SessionState::Streaming
                    && s.options.cursor() == CaptureCursorMode::Metadata
            })
        });
        if let Err(error) = self
            .cursor
            .update(if needed { cursor } else { None }, position, scale)
        {
            eprintln!("telorgon-capture: cursor metadata failed: {error}");
            self.cursor = CursorState::default();
            for (&id, stream) in &self.streams {
                if sessions
                    .get(id)
                    .is_some_and(|s| s.options.cursor() == CaptureCursorMode::Metadata)
                {
                    let _ = sessions.stop(id, stream.requester, CaptureStopReason::StreamFailed);
                    stream.video.stop();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::linux_shell::pointer_visual::RenderedCursor;
    use std::num::NonZeroU32;
    fn cursor() -> CursorVisual {
        CursorVisual::Image(RenderedCursor {
            rgba: [200, 100, 50, 128].repeat(4),
            size: SizeI {
                width: 2,
                height: 2,
            },
            logical_size: SizeI {
                width: 2,
                height: 2,
            },
            hotspot: PointI { x: 1, y: 1 },
            premultiplied: false,
        })
    }
    fn layout() -> CaptureLayout {
        CaptureLayout::rgba8(
            NonZeroU32::new(20).unwrap(),
            NonZeroU32::new(20).unwrap(),
            80,
        )
        .unwrap()
    }
    #[test]
    fn cursor_coordinates_are_physical_source_relative_and_hide_outside_grant() {
        let mut state = CursorState::default();
        let image = cursor();
        let scale = ScaleFactor::new(1.5).unwrap();
        state
            .update(Some(&image), PointF { x: 10.0, y: 12.0 }, scale)
            .unwrap();
        let origin = PointI { x: 5, y: 7 };
        let visible = state.for_source(origin, layout());
        assert_eq!(
            (visible.x, visible.y, visible.hotspot_x, visible.hotspot_y),
            (10, 11, 2, 2)
        );
        let bitmap = visible.bitmap.unwrap();
        assert_eq!((bitmap.width, bitmap.height), (3, 3));
        assert_eq!(&bitmap.rgba[..4], &[100, 50, 25, 128]);
        let revision = state.revision();
        state
            .update(Some(&image), PointF { x: 10.0, y: 12.0 }, scale)
            .unwrap();
        assert_eq!(state.revision(), revision);
        state
            .update(Some(&image), PointF { x: 20.0, y: 12.0 }, scale)
            .unwrap();
        assert!(state.revision() > revision);
        let hidden = state.for_source(origin, layout());
        assert_eq!((hidden.x, hidden.y, hidden.visible), (0, 0, Some(false)));
        assert!(hidden.bitmap.is_none());
        state.update(None, PointF::default(), scale).unwrap();
        assert_eq!(
            state.for_source(PointI::default(), layout()).visible,
            Some(false)
        );
    }
    #[test]
    fn metadata_rejects_oversized_bitmaps_and_nonfinite_positions() {
        let mut state = CursorState::default();
        let mut image = cursor();
        let CursorVisual::Image(rendered) = &mut image;
        rendered.logical_size.width = 257;
        let scale = ScaleFactor::new(1.0).unwrap();
        assert!(
            state
                .update(Some(&image), PointF::default(), scale)
                .is_err()
        );
        assert!(
            state
                .update(
                    Some(&cursor()),
                    PointF {
                        x: f32::NAN,
                        y: 0.0
                    },
                    scale
                )
                .is_err()
        );
        assert!(state.image.is_none());
    }
}

use super::*;

pub(super) const HARDWARE_CURSOR_BUFFER_COUNT: usize = 3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct CursorSnapshot {
    pub(super) serial: u64,
    pub(super) buffer: Option<usize>,
    pub(super) position: PointI,
    pub(super) hotspot: PointI,
    pub(super) visible: bool,
}

#[derive(Debug, Default)]
pub(super) struct CursorCommitTracker {
    pub(super) desired: CursorSnapshot,
    pub(super) applied_serial: u64,
    pub(super) current_buffer: Option<usize>,
    pub(super) in_flight: Option<CursorSnapshot>,
    pub(super) composited_fallback_requested: bool,
}

impl CursorCommitTracker {
    pub(super) fn move_to(&mut self, position: PointI) {
        if self.desired.position != position {
            self.desired.position = position;
            if self.desired.visible {
                self.bump_serial();
            }
        }
    }

    pub(super) fn show(&mut self, buffer: usize, hotspot: PointI) {
        if !self.desired.visible
            || self.desired.buffer != Some(buffer)
            || self.desired.hotspot != hotspot
        {
            self.desired.visible = true;
            self.desired.buffer = Some(buffer);
            self.desired.hotspot = hotspot;
            self.bump_serial();
        }
    }

    pub(super) fn hide(&mut self) {
        if self.desired.visible {
            self.desired.visible = false;
            self.bump_serial();
        }
    }

    pub(super) fn request_composited_fallback(&mut self) {
        self.composited_fallback_requested = true;
        self.hide();
    }

    pub(super) fn desired_submission(&self) -> Option<CursorSnapshot> {
        (self.in_flight.is_none() && self.desired.serial != self.applied_serial)
            .then_some(self.desired)
    }

    pub(super) fn mark_submitted(&mut self, snapshot: CursorSnapshot) -> AppResult<()> {
        if self.in_flight.is_some() || snapshot.serial != self.desired.serial {
            return Err(AppError::new(
                "atomic cursor submission did not match the desired cursor generation",
            ));
        }
        self.in_flight = Some(snapshot);
        Ok(())
    }

    pub(super) fn mark_completed(&mut self, snapshot: CursorSnapshot) -> AppResult<()> {
        if self.in_flight != Some(snapshot) {
            return Err(AppError::new(
                "DRM completed an unexpected atomic cursor generation",
            ));
        }
        self.in_flight = None;
        self.applied_serial = snapshot.serial;
        self.current_buffer = snapshot.visible.then_some(snapshot.buffer).flatten();
        Ok(())
    }

    pub(super) fn reusable_buffer(&self, count: usize) -> Option<usize> {
        let in_flight = self.in_flight.and_then(|snapshot| snapshot.buffer);
        self.desired
            .buffer
            .filter(|buffer| Some(*buffer) != self.current_buffer && Some(*buffer) != in_flight)
            .or_else(|| {
                (0..count).find(|buffer| {
                    Some(*buffer) != self.current_buffer && Some(*buffer) != in_flight
                })
            })
    }

    pub(super) fn ready_to_retire(&self) -> bool {
        self.composited_fallback_requested
            && self.current_buffer.is_none()
            && self.in_flight.is_none()
    }

    fn bump_serial(&mut self) {
        self.desired.serial = self.desired.serial.wrapping_add(1).max(1);
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct PendingKmsCommit {
    pub(super) primary_slot: Option<usize>,
    pub(super) cursor: Option<CursorSnapshot>,
    #[cfg(feature = "profiler")]
    pub(super) cursor_event_us: Option<u64>,
}

pub(super) struct HardwareCursor<'gbm, 'kms, 'fd> {
    // Framebuffers must be removed before the GBM buffers that back them are destroyed.
    framebuffers: Vec<KmsFramebuffer<'kms>>,
    buffers: Vec<GbmBuffer<'gbm, 'fd>>,
    extent: SizeI,
    plane: KmsPlaneId,
    properties: KmsObjectProperties,
    has_hotspot_properties: bool,
    state: CursorCommitTracker,
    image_signature: Option<u64>,
}

impl<'gbm, 'kms, 'fd> HardwareCursor<'gbm, 'kms, 'fd> {
    pub(super) fn new(
        gbm: &'gbm GbmDevice<'fd>,
        kms: &'kms KmsDevice,
        plane: KmsPlaneId,
        properties: KmsObjectProperties,
    ) -> AppResult<Self> {
        for name in [
            "FB_ID", "CRTC_ID", "SRC_X", "SRC_Y", "SRC_W", "SRC_H", "CRTC_X", "CRTC_Y", "CRTC_W",
            "CRTC_H",
        ] {
            if properties.named(name).is_none() {
                return Err(AppError::new(format!(
                    "DRM cursor plane has no required atomic property {name}"
                )));
            }
        }
        let has_hotspot_properties = kms.cursor_plane_hotspot_capable();
        if has_hotspot_properties
            && (properties.named("HOTSPOT_X").is_none() || properties.named("HOTSPOT_Y").is_none())
        {
            return Err(AppError::new(
                "DRM cursor-hotspot capability was enabled without hotspot properties",
            ));
        }
        let extent = kms.cursor_size().map_err(app_error)?;
        let mut buffers = Vec::with_capacity(HARDWARE_CURSOR_BUFFER_COUNT);
        for _ in 0..HARDWARE_CURSOR_BUFFER_COUNT {
            buffers.push(gbm.allocate_cursor(extent).map_err(app_error)?);
        }
        let framebuffers = buffers
            .iter()
            .map(|buffer| kms.add_framebuffer(buffer).map_err(app_error))
            .collect::<AppResult<Vec<_>>>()?;
        Ok(Self {
            framebuffers,
            buffers,
            extent,
            plane,
            properties,
            has_hotspot_properties,
            state: CursorCommitTracker::default(),
            image_signature: None,
        })
    }

    pub(super) fn set_logical_image(
        &mut self,
        cursor: &RenderedCursor,
        scale: crate::platform::ScaleFactor,
    ) -> AppResult<()> {
        let physical = cursor.for_hardware(scale, self.extent)?;
        self.set_image(&physical)
    }

    fn set_image(&mut self, cursor: &RenderedCursor) -> AppResult<()> {
        if cursor.size.width <= 0
            || cursor.size.height <= 0
            || cursor.size.width > self.extent.width
            || cursor.size.height > self.extent.height
        {
            return Err(AppError::new(
                "cursor image has an invalid DRM hardware-cursor extent",
            ));
        }
        let source_length = (cursor.size.width as usize)
            .checked_mul(cursor.size.height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| AppError::new("cursor image byte length overflow"))?;
        if cursor.rgba.len() != source_length {
            return Err(AppError::new(
                "cursor image pixels do not match its declared extent",
            ));
        }
        let signature = cursor_image_signature(cursor);
        if self.image_signature == Some(signature) {
            let buffer = self.state.desired.buffer.ok_or_else(|| {
                AppError::new("atomic cursor image signature has no staged buffer")
            })?;
            self.state.show(buffer, cursor.hotspot);
            return Ok(());
        }
        let mut pixels = vec![0_u8; self.extent.width as usize * self.extent.height as usize * 4];
        let source_stride = cursor.size.width.max(0) as usize * 4;
        let target_stride = self.extent.width as usize * 4;
        for row in 0..cursor.size.height.max(0) as usize {
            let source = &cursor.rgba[row * source_stride..(row + 1) * source_stride];
            let target = &mut pixels[row * target_stride..row * target_stride + source_stride];
            target.copy_from_slice(source);
            if !cursor.premultiplied {
                for pixel in target.chunks_exact_mut(4) {
                    let alpha = u16::from(pixel[3]);
                    for channel in &mut pixel[..3] {
                        *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
                    }
                }
            }
        }
        let next = self
            .state
            .reusable_buffer(self.buffers.len())
            .ok_or_else(|| AppError::new("no retired hardware-cursor buffer is available"))?;
        self.buffers[next]
            .map_write()
            .map_err(app_error)?
            .write_rgba8(&pixels)
            .map_err(app_error)?;
        self.state.show(next, cursor.hotspot);
        self.image_signature = Some(signature);
        Ok(())
    }

    pub(super) fn move_to(&mut self, position: PointF) {
        self.state.move_to(PointI {
            x: position.x.round() as i32,
            y: position.y.round() as i32,
        });
    }

    pub(super) fn hide(&mut self) {
        self.state.hide();
    }

    pub(super) fn append_desired(
        &self,
        request: &mut AtomicRequest<'_>,
        crtc: KmsCrtcId,
    ) -> AppResult<Option<CursorSnapshot>> {
        let Some(snapshot) = self.state.desired_submission() else {
            return Ok(None);
        };
        if snapshot.visible {
            let buffer = snapshot.buffer.ok_or_else(|| {
                AppError::new("visible atomic cursor generation has no framebuffer")
            })?;
            let framebuffer = self.framebuffers.get(buffer).ok_or_else(|| {
                AppError::new("atomic cursor generation names an invalid framebuffer")
            })?;
            let destination = RectI {
                x: snapshot.position.x.saturating_sub(snapshot.hotspot.x),
                y: snapshot.position.y.saturating_sub(snapshot.hotspot.y),
                width: self.extent.width,
                height: self.extent.height,
            };
            request
                .set_plane(
                    self.plane,
                    &self.properties,
                    crtc,
                    framebuffer.id(),
                    RectI {
                        x: 0,
                        y: 0,
                        width: self.extent.width,
                        height: self.extent.height,
                    },
                    destination,
                )
                .map_err(app_error)?;
            if self.has_hotspot_properties {
                request
                    .set_cursor_hotspot(self.plane, &self.properties, snapshot.hotspot)
                    .map_err(app_error)?;
            }
        } else {
            request
                .disable_plane(self.plane, &self.properties)
                .map_err(app_error)?;
        }
        Ok(Some(snapshot))
    }

    pub(super) fn mark_submitted(&mut self, snapshot: CursorSnapshot) -> AppResult<()> {
        self.state.mark_submitted(snapshot)
    }

    pub(super) fn mark_completed(&mut self, snapshot: CursorSnapshot) -> AppResult<()> {
        self.state.mark_completed(snapshot)
    }

    pub(super) fn needs_commit(&self) -> bool {
        self.state.desired_submission().is_some()
    }

    pub(super) fn request_composited_fallback(&mut self) {
        self.state.request_composited_fallback();
    }

    pub(super) fn composited_fallback_requested(&self) -> bool {
        self.state.composited_fallback_requested
    }

    pub(super) fn ready_to_retire(&self) -> bool {
        self.state.ready_to_retire()
    }
}

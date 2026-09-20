//! Native capture source dispatch. Globals remain disabled until the complete
//! capture execution and authorization path is assembled by the host.
use super::*;

impl NativeState {
    fn capture_output_size(&self, output: u32) -> Option<crate::core::SizeI> {
        if self.secure_session_locked || self.active_session_lock.is_some() {
            return None;
        }
        self.core
            .outputs
            .get(&output)
            .filter(|output| output.enabled)
            .and_then(|output| output.description.modes.get(output.current_mode))
            .map(|mode| mode.size)
            .filter(|size| size.width > 0 && size.height > 0)
    }

    pub(super) fn refresh_capture_sources(&mut self) -> Result<(), NativeCompositorError> {
        let updates: Vec<_> = self
            .capture
            .sessions
            .iter()
            .filter_map(|(&id, session)| {
                // Stopped sessions never revive after unlocking or output replacement.
                let size = session
                    .size
                    .and_then(|_| self.capture_output_size(session.output));
                (size != session.size).then_some((id, size))
            })
            .collect();
        for (id, size) in updates {
            self.capture
                .sessions
                .get_mut(&id)
                .expect("capture session")
                .size = size;
            if let Some(resource) = self.resource_for_object(id)? {
                self.send_capture_constraints(resource, size)?;
            }
        }
        let failures: Vec<_> = self
            .capture
            .frames
            .iter()
            .filter_map(|(&id, frame)| {
                let size = frame
                    .size
                    .and_then(|_| self.capture_output_size(frame.output));
                if size.is_none() {
                    Some((id, 2))
                } else if frame.pending.is_some() && size != frame.size {
                    Some((id, 1))
                } else {
                    None
                }
            })
            .collect();
        for (id, reason) in failures {
            let frame = self.capture.frames.get_mut(&id).expect("capture frame");
            if reason == 2 {
                frame.size = None;
            }
            let can_notify = frame.delivery.invalidate(reason);
            let notify = can_notify && frame.pending.take().is_some() && frame.lifecycle.finish();
            frame.cancellation.cancel();
            frame.destination = None;
            if notify && let Some(resource) = self.resource_for_object(id)? {
                self.post_event(
                    resource,
                    "ext_image_copy_capture_frame_v1",
                    "failed",
                    &mut [ffi::wl_argument { u: reason }],
                )?;
            }
        }
        Ok(())
    }
}

impl NativeState {
    pub(super) fn check_capture_access(
        &self,
        kind: ResourceKind,
        client: usize,
    ) -> Result<(), NativeCompositorError> {
        if matches!(
            kind,
            ResourceKind::ForeignToplevelList
                | ResourceKind::ForeignToplevelHandle
                | ResourceKind::OutputCaptureSourceManager
                | ResourceKind::OutputCaptureSource(_)
                | ResourceKind::ImageCopyCaptureManager
                | ResourceKind::ImageCopyCaptureSession(_)
                | ResourceKind::ImageCopyCaptureFrame(_)
        ) && !self
            .capture_access
            .as_ref()
            .is_some_and(|access| access.allows(client))
        {
            return Err(NativeCompositorError::new("unauthorized capture request"));
        }
        Ok(())
    }

    pub(super) fn dispatch_output_capture_source(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        if request.message().name != "create_source" {
            return Err(unsupported_request(request));
        }
        let output = request
            .object(1)
            .map_err(error)?
            .ok_or_else(|| NativeCompositorError::new("missing capture output"))?;
        if output.client().identity() != resource.client().identity() {
            return Err(NativeCompositorError::new(
                "capture output belongs to another client",
            ));
        }
        let ResourceKind::Output(output_id) = self.resource_kind(output)? else {
            return Err(NativeCompositorError::new(
                "capture source is not an output",
            ));
        };
        self.create_resource(
            resource.client(),
            context.client,
            "ext_image_capture_source_v1",
            1,
            request.new_id(0).map_err(error)?,
            ResourceKind::OutputCaptureSource(output_id),
            true,
        )?;
        Ok(DispatchOutcome::default())
    }
}

#[derive(Default)]
pub(super) struct NativeCaptureState {
    owner: std::sync::Arc<()>,
    pub sessions: BTreeMap<ProtocolObjectId, NativeCaptureSession>,
    pub frames: BTreeMap<ProtocolObjectId, NativeCaptureFrame>,
}

pub(super) struct NativeCaptureSession {
    requester: ClientId,
    output: u32,
    paint_cursor: bool,
    lifecycle: crate::compositor_wayland::capture::CaptureSession,
    size: Option<crate::core::SizeI>,
}

#[derive(Default)]
struct DeliveryState {
    handed_off: bool,
    failure: Option<u32>,
}
impl DeliveryState {
    fn invalidate(&mut self, reason: u32) -> bool {
        // Source termination supersedes a dimensions mismatch.
        self.failure = Some(self.failure.unwrap_or(reason).max(reason));
        !self.handed_off
    }
}

pub(super) struct NativeCaptureFrame {
    requester: ClientId,
    session: ProtocolObjectId,
    output: u32,
    paint_cursor: bool,
    lifecycle: crate::compositor_wayland::capture::CaptureFrame<WaylandBufferId>,
    pending: Option<WaylandBufferId>,
    destination: Option<super::capture_buffer::CaptureDestination>,
    cancellation: super::capture_buffer::CaptureCancellation,
    delivery: DeliveryState,
    size: Option<crate::core::SizeI>,
}

impl NativeState {
    pub(super) fn dispatch_copy_capture_manager(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        if request.message().name != "create_session" {
            return Err(unsupported_request(request));
        }
        let options = request.uint(2).map_err(error)?;
        if options & !1 != 0 {
            resource.post_error(1, "invalid capture option");
            return Ok(DispatchOutcome::default());
        }
        let source = request
            .object(1)
            .map_err(error)?
            .ok_or_else(|| NativeCompositorError::new("missing capture source"))?;
        if source.client().identity() != resource.client().identity() {
            return Err(NativeCompositorError::new(
                "capture source belongs to another client",
            ));
        }
        let ResourceKind::OutputCaptureSource(output) = self.resource_kind(source)? else {
            return Err(NativeCompositorError::new("invalid capture source"));
        };
        if !self.capture.can_admit(context.client) {
            resource.client().post_no_memory();
            return Ok(DispatchOutcome::default());
        }
        let size = self.capture_output_size(output);
        let id = self.peek_next_object()?;
        let child = self.create_resource(
            resource.client(),
            context.client,
            "ext_image_copy_capture_session_v1",
            1,
            request.new_id(0).map_err(error)?,
            ResourceKind::ImageCopyCaptureSession(id),
            true,
        )?;
        self.capture.sessions.insert(
            id,
            NativeCaptureSession {
                requester: context.client,
                output,
                paint_cursor: options & 1 != 0,
                lifecycle: Default::default(),
                size,
            },
        );
        self.send_capture_constraints(child, size)?;
        Ok(DispatchOutcome::default())
    }

    fn send_capture_constraints(
        &self,
        resource: ResourceRef<'_>,
        size: Option<crate::core::SizeI>,
    ) -> Result<(), NativeCompositorError> {
        let interface = "ext_image_copy_capture_session_v1";
        let Some(size) = size else {
            return self.post_event(resource, interface, "stopped", &mut []);
        };
        self.post_event(
            resource,
            interface,
            "buffer_size",
            &mut [
                ffi::wl_argument {
                    u: size.width as u32,
                },
                ffi::wl_argument {
                    u: size.height as u32,
                },
            ],
        )?;
        // wl_shm ARGB8888 is mandatory and maps to BGRA bytes on little-endian
        // hosts. The delivery bridge must convert the renderer's RGBA readback.
        self.post_event(
            resource,
            interface,
            "shm_format",
            &mut [ffi::wl_argument { u: 0 }],
        )?;
        self.post_event(resource, interface, "done", &mut [])
    }

    pub(super) fn dispatch_copy_capture_session(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        session: ProtocolObjectId,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        if request.message().name != "create_frame" {
            return Err(unsupported_request(request));
        }
        let session_id = session;
        let session = self
            .capture
            .sessions
            .get(&session)
            .ok_or_else(|| NativeCompositorError::new("missing capture session"))?;
        let lifecycle = match session.lifecycle.create_frame() {
            Ok(frame) => frame,
            Err(error) => {
                resource.post_error(error as u32, "previous capture frame still exists");
                return Ok(DispatchOutcome::default());
            }
        };
        let frame = NativeCaptureFrame {
            requester: context.client,
            session: session_id,
            output: session.output,
            paint_cursor: session.paint_cursor,
            lifecycle,
            pending: None,
            destination: None,
            cancellation: Default::default(),
            delivery: Default::default(),
            size: session.size,
        };
        let id = self.peek_next_object()?;
        self.create_resource(
            resource.client(),
            context.client,
            "ext_image_copy_capture_frame_v1",
            1,
            request.new_id(0).map_err(error)?,
            ResourceKind::ImageCopyCaptureFrame(id),
            true,
        )?;
        self.capture.frames.insert(id, frame);
        Ok(DispatchOutcome::default())
    }

    pub(super) fn dispatch_copy_capture_frame(
        &mut self,
        resource: ResourceRef<'_>,
        id: ProtocolObjectId,
        request: &IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        self.refresh_capture_sources()?;
        // Resolve the buffer while borrowing no mutable frame state.
        let buffer = if request.message().name == "attach_buffer" {
            let buffer = request
                .object(0)
                .map_err(error)?
                .ok_or_else(|| NativeCompositorError::new("missing capture buffer"))?;
            if buffer.client().identity() != resource.client().identity() {
                return Err(NativeCompositorError::new(
                    "capture buffer belongs to another client",
                ));
            }
            let ResourceKind::Buffer(buffer) = self.resource_kind(buffer)? else {
                return Err(NativeCompositorError::new("capture target is not a buffer"));
            };
            Some(buffer)
        } else {
            None
        };
        let frame = self
            .capture
            .frames
            .get_mut(&id)
            .ok_or_else(|| NativeCompositorError::new("missing capture frame"))?;
        let result = match request.message().name {
            "attach_buffer" => frame
                .lifecycle
                .attach(buffer.expect("resolved capture buffer")),
            "damage_buffer" => frame.lifecycle.damage(
                request.int(0).map_err(error)?,
                request.int(1).map_err(error)?,
                request.int(2).map_err(error)?,
                request.int(3).map_err(error)?,
            ),
            "capture" => frame.lifecycle.capture().map(|buffer| {
                frame.pending = Some(buffer);
            }),
            _ => return Err(unsupported_request(request)),
        };
        if let Err(error) = result {
            resource.post_error(error as u32, "invalid capture frame request");
        }
        let stopped = frame.pending.is_some() && frame.size.is_none();
        if stopped {
            frame.pending = None;
            if frame.lifecycle.finish() {
                self.post_event(
                    resource,
                    "ext_image_copy_capture_frame_v1",
                    "failed",
                    &mut [ffi::wl_argument { u: 2 }],
                )?;
            }
        }
        if request.message().name == "capture" {
            let frame = self.capture.frames.get(&id).expect("capture frame");
            if let (Some(buffer), Some(size)) = (frame.pending, frame.size) {
                let destination = (|| {
                    let Some(BufferDescriptor::Shm(descriptor)) = self.core.buffer(buffer) else {
                        return Err(NativeCompositorError::new("capture requires SHM buffer"));
                    };
                    let fd = self
                        .buffer_files
                        .get(&buffer)
                        .ok_or_else(|| NativeCompositorError::new("missing capture backing file"))?
                        .try_clone()
                        .map_err(error)?;
                    super::capture_buffer::CaptureDestination::new(
                        *descriptor,
                        std::fs::File::from(fd),
                        size,
                        &frame.cancellation,
                    )
                })();
                let frame = self.capture.frames.get_mut(&id).expect("capture frame");
                match destination {
                    Ok(destination) => frame.destination = Some(destination),
                    Err(_) => {
                        frame.pending = None;
                        if frame.lifecycle.finish() {
                            self.post_event(
                                resource,
                                "ext_image_copy_capture_frame_v1",
                                "failed",
                                &mut [ffi::wl_argument { u: 1 }],
                            )?;
                        }
                    }
                }
            }
        }
        self.refresh_capture_sources()?;
        Ok(DispatchOutcome::default())
    }
}

/// Owned worker input: contains no native resource pointers or compositor borrows.
pub(crate) struct DirectCaptureJob {
    pub session: ProtocolObjectId,
    pub requester: ClientId,
    owner: std::sync::Arc<()>,
    frame: ProtocolObjectId,
    pub output: u32,
    pub size: crate::core::SizeI,
    pub paint_cursor: bool,
    destination: super::capture_buffer::CaptureDestination,
}

pub(crate) struct DirectCaptureCompletion {
    owner: std::sync::Arc<()>,
    frame: ProtocolObjectId,
    success: bool,
    timestamp_ns: u64,
    transform: crate::compositor_wayland::OutputTransform,
}

impl DirectCaptureJob {
    /// Call on the delivery worker after GPU completion. The timestamp is the
    /// source frame's CLOCK_MONOTONIC presentation time, not wall-clock time.
    pub fn write(
        self,
        rgba: &[u8],
        timestamp_ns: u64,
        transform: crate::compositor_wayland::OutputTransform,
    ) -> DirectCaptureCompletion {
        DirectCaptureCompletion {
            owner: self.owner,
            frame: self.frame,
            success: self.destination.write_rgba(rgba).is_ok(),
            timestamp_ns,
            transform,
        }
    }

    pub fn fail(self) -> DirectCaptureCompletion {
        self.failure_completion()
    }

    /// Keep this terminal fallback before handing ownership to a renderer API
    /// which may consume and drop the job on submission failure.
    pub fn failure_completion(&self) -> DirectCaptureCompletion {
        DirectCaptureCompletion {
            owner: self.owner.clone(),
            frame: self.frame,
            success: false,
            timestamp_ns: 0,
            transform: crate::compositor_wayland::OutputTransform::Normal,
        }
    }
}

impl NativeCaptureState {
    fn can_admit(&self, requester: ClientId) -> bool {
        // A session's frame can outlive its parent. Count that incarnation once,
        // so destroying parents cannot bypass native admission limits.
        let mut owners: BTreeMap<_, _> = self
            .sessions
            .iter()
            .map(|(&id, session)| (id, session.requester))
            .collect();
        for frame in self.frames.values() {
            owners.entry(frame.session).or_insert(frame.requester);
        }
        owners.len() < 8 && owners.values().filter(|&&owner| owner == requester).count() < 2
    }

    fn retains_session(&self, id: ProtocolObjectId) -> bool {
        self.sessions
            .get(&id)
            .is_some_and(|session| session.size.is_some())
            || self
                .frames
                .values()
                .any(|frame| frame.session == id && frame.size.is_some())
    }
}

impl NativeCompositor<'_> {
    pub(crate) fn direct_capture_needs_cursor(&self) -> bool {
        self.state
            .capture
            .sessions
            .values()
            .any(|session| session.paint_cursor && session.size.is_some())
            || self
                .state
                .capture
                .frames
                .values()
                .any(|frame| frame.paint_cursor && frame.size.is_some())
    }

    pub(crate) fn stop_direct_cursor_capture(&mut self) -> Result<(), NativeCompositorError> {
        let stopped: Vec<_> = self
            .state
            .capture
            .sessions
            .iter_mut()
            .filter_map(|(&id, session)| {
                if session.paint_cursor && session.size.take().is_some() {
                    Some(id)
                } else {
                    None
                }
            })
            .collect();
        for frame in self
            .state
            .capture
            .frames
            .values_mut()
            .filter(|frame| frame.paint_cursor)
        {
            frame.size = None;
        }
        for id in stopped {
            if let Some(resource) = self.state.resource_for_object(id)? {
                self.state.send_capture_constraints(resource, None)?;
            }
        }
        self.state.refresh_capture_sources()
    }

    pub(crate) fn direct_capture_job_live(
        &mut self,
        job: &DirectCaptureJob,
    ) -> Result<bool, NativeCompositorError> {
        self.state.refresh_capture_sources()?;
        if !std::sync::Arc::ptr_eq(&job.owner, &self.state.capture.owner)
            || job.destination.is_cancelled()
        {
            return Ok(false);
        }
        let Some(frame) = self.state.capture.frames.get(&job.frame) else {
            return Ok(false);
        };
        if frame.pending.is_none() || frame.size != Some(job.size) || frame.session != job.session {
            return Ok(false);
        }
        let Some(resource) = self.state.resource_for_object(job.frame)? else {
            return Ok(false);
        };
        Ok(self
            .state
            .capture_access
            .as_ref()
            .is_some_and(|access| access.allows(resource.client().identity())))
    }

    /// Includes surviving frame resources after their session object is destroyed.
    /// Host GPU retirement remains independent of this protocol liveness query.
    pub(crate) fn direct_capture_session_live(
        &mut self,
        session: ProtocolObjectId,
    ) -> Result<bool, NativeCompositorError> {
        self.state.refresh_capture_sources()?;
        Ok(self.state.capture.retains_session(session))
    }

    /// Take one request only when host scheduling and memory limits admit work.
    /// Every returned job must be written or failed and its completion returned.
    pub(crate) fn take_direct_capture_job(
        &mut self,
    ) -> Result<Option<DirectCaptureJob>, NativeCompositorError> {
        self.state.refresh_capture_sources()?;
        let id = self.state.capture.frames.iter().find_map(|(&id, frame)| {
            (frame.pending.is_some() && frame.destination.is_some() && frame.size.is_some())
                .then_some(id)
        });
        let Some(id) = id else {
            return Ok(None);
        };
        let Some(resource) = self.state.resource_for_object(id)? else {
            return Ok(None);
        };
        self.state.check_capture_access(
            ResourceKind::ImageCopyCaptureFrame(id),
            resource.client().identity(),
        )?;
        let requester = *self
            .state
            .clients
            .get(&resource.client().identity())
            .ok_or_else(|| NativeCompositorError::new("missing capture requester"))?;
        let frame = self
            .state
            .capture
            .frames
            .get_mut(&id)
            .expect("capture frame");
        frame.delivery.handed_off = true;
        Ok(Some(DirectCaptureJob {
            session: frame.session,
            requester,
            owner: self.state.capture.owner.clone(),
            frame: id,
            output: frame.output,
            size: frame.size.expect("checked capture size"),
            paint_cursor: frame.paint_cursor,
            destination: frame
                .destination
                .take()
                .expect("checked capture destination"),
        }))
    }

    /// Owner-thread terminal event routing. Destroyed or invalidated requests
    /// discard late completions; resource IDs are monotonic and never recycled.
    pub(crate) fn complete_direct_capture(
        &mut self,
        completion: DirectCaptureCompletion,
    ) -> Result<(), NativeCompositorError> {
        if !std::sync::Arc::ptr_eq(&completion.owner, &self.state.capture.owner) {
            return Ok(());
        }
        self.state.refresh_capture_sources()?;
        let id = completion.frame;
        let Some(resource) = self.state.resource_for_object(id)? else {
            return Ok(());
        };
        self.state.check_capture_access(
            ResourceKind::ImageCopyCaptureFrame(id),
            resource.client().identity(),
        )?;
        let Some(frame) = self.state.capture.frames.get_mut(&id) else {
            return Ok(());
        };
        if frame.pending.take().is_none() || !frame.lifecycle.finish() {
            return Ok(());
        }
        frame.delivery.handed_off = false;
        let failure_reason = frame.delivery.failure;
        let size = frame.size;
        let resource = self
            .state
            .resource_for_object(id)?
            .expect("checked capture resource");
        let interface = "ext_image_copy_capture_frame_v1";
        if !completion.success || size.is_none() || failure_reason.is_some() {
            return self.state.post_event(
                resource,
                interface,
                "failed",
                &mut [ffi::wl_argument {
                    u: failure_reason.unwrap_or(0),
                }],
            );
        }
        let size = size.expect("checked capture size");
        self.state.post_event(
            resource,
            interface,
            "transform",
            &mut [ffi::wl_argument {
                u: output_transform_wire(completion.transform) as u32,
            }],
        )?;
        self.state.post_event(
            resource,
            interface,
            "damage",
            &mut [
                ffi::wl_argument { i: 0 },
                ffi::wl_argument { i: 0 },
                ffi::wl_argument { i: size.width },
                ffi::wl_argument { i: size.height },
            ],
        )?;
        let seconds = completion.timestamp_ns / 1_000_000_000;
        self.state.post_event(
            resource,
            interface,
            "presentation_time",
            &mut [
                ffi::wl_argument {
                    u: (seconds >> 32) as u32,
                },
                ffi::wl_argument { u: seconds as u32 },
                ffi::wl_argument {
                    u: (completion.timestamp_ns % 1_000_000_000) as u32,
                },
            ],
        )?;
        self.state.post_event(resource, interface, "ready", &mut [])
    }
}

#[cfg(test)]
mod worker_contract_tests {
    use super::*;
    #[test]
    fn source_failure_waits_for_handed_off_destination_before_terminal_event() {
        let mut queued = DeliveryState::default();
        assert!(queued.invalidate(1));
        let mut worker = DeliveryState {
            handed_off: true,
            failure: None,
        };
        assert!(!worker.invalidate(1));
        assert!(!worker.invalidate(2));
        assert_eq!(worker.failure, Some(2));
        worker.handed_off = false;
        assert!(worker.invalidate(1));
        assert_eq!(worker.failure, Some(2));
    }

    #[test]
    fn native_admission_counts_orphan_frames_and_isolates_requesters() {
        let mut state = NativeCaptureState::default();
        let owner = ClientId::from_raw(1).unwrap();
        let other = ClientId::from_raw(2).unwrap();
        for raw in 1..=2 {
            let id = ProtocolObjectId::from_raw(raw).unwrap();
            let session = NativeCaptureSession {
                requester: owner,
                output: 1,
                paint_cursor: false,
                lifecycle: Default::default(),
                size: None,
            };
            state.frames.insert(
                id,
                NativeCaptureFrame {
                    requester: owner,
                    session: id,
                    output: 1,
                    paint_cursor: false,
                    lifecycle: session.lifecycle.create_frame().unwrap(),
                    pending: None,
                    destination: None,
                    cancellation: Default::default(),
                    delivery: Default::default(),
                    size: None,
                },
            );
            state.sessions.insert(id, session);
        }
        assert!(!state.can_admit(owner));
        assert!(state.can_admit(other));
        state.sessions.clear();
        assert!(!state.can_admit(owner));
        state.frames.remove(&ProtocolObjectId::from_raw(1).unwrap());
        assert!(state.can_admit(owner));
        for raw in 3..=9 {
            state.sessions.insert(
                ProtocolObjectId::from_raw(raw).unwrap(),
                NativeCaptureSession {
                    requester: ClientId::from_raw(raw).unwrap(),
                    output: 1,
                    paint_cursor: false,
                    lifecycle: Default::default(),
                    size: None,
                },
            );
        }
        assert!(!state.can_admit(other));
    }

    #[test]
    fn surviving_frame_keeps_its_session_identity_until_source_loss() {
        let mut state = NativeCaptureState::default();
        let session_id = ProtocolObjectId::from_raw(1).unwrap();
        let frame_id = ProtocolObjectId::from_raw(2).unwrap();
        let size = crate::core::SizeI {
            width: 10,
            height: 10,
        };
        let session = NativeCaptureSession {
            requester: ClientId::from_raw(1).unwrap(),
            output: 1,
            paint_cursor: false,
            lifecycle: Default::default(),
            size: Some(size),
        };
        let frame = NativeCaptureFrame {
            requester: ClientId::from_raw(1).unwrap(),
            session: session_id,
            output: 1,
            paint_cursor: false,
            lifecycle: session.lifecycle.create_frame().unwrap(),
            pending: None,
            destination: None,
            cancellation: Default::default(),
            delivery: Default::default(),
            size: Some(size),
        };
        state.sessions.insert(session_id, session);
        state.frames.insert(frame_id, frame);
        state.sessions.remove(&session_id);
        assert!(state.retains_session(session_id));
        assert!(!state.retains_session(frame_id));
        state.frames.get_mut(&frame_id).unwrap().size = None;
        assert!(!state.retains_session(session_id));
        state.frames.remove(&frame_id);
        assert!(!state.retains_session(session_id));
    }

    #[test]
    fn direct_capture_worker_messages_can_cross_threads() {
        fn assert_send<T: Send>() {}
        assert_send::<DirectCaptureJob>();
        assert_send::<DirectCaptureCompletion>();
    }
}

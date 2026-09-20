use super::*;

impl<S: NativeRuntimeSource, P: NativePresentation> NativeHost<S, P> {
    pub(super) fn redraw(&mut self, event_loop: &ActiveEventLoop, source: RedrawSource) {
        if let Err(error) = self.try_redraw(source) {
            self.fail(event_loop, error);
        }
    }

    pub(super) fn try_redraw(&mut self, source: RedrawSource) -> Result<(), String> {
        let requested_callback = if source == RedrawSource::NativeCallback {
            self.diagnostics.redraw_callbacks = self.diagnostics.redraw_callbacks.saturating_add(1);
            self.redraw.native_callback_started()
        } else {
            false
        };
        self.presentation.poll()?;
        let mut resize_barrier_revision = None;
        if let Some(extent) = self.pending_resize.take(Instant::now()) {
            self.drawable = extent.width > 0 && extent.height > 0;
            let signal = self.resize_signal();
            let policy = self.presentation.resize_policy();
            let update = self.live_resize.observe(extent, signal, policy);
            #[cfg(feature = "profiler")]
            match update.phase {
                crate::host::application::native::resize::ResizeInteractionPhase::Stable => {
                    crate::runtime::instrumentation::instant!("responsiveness.resize.stable");
                }
                crate::host::application::native::resize::ResizeInteractionPhase::Started => {
                    crate::runtime::instrumentation::instant!("responsiveness.resize.started");
                }
                crate::host::application::native::resize::ResizeInteractionPhase::Updating => {
                    crate::runtime::instrumentation::instant!("responsiveness.resize.updating");
                }
                crate::host::application::native::resize::ResizeInteractionPhase::Ended => {
                    crate::runtime::instrumentation::instant!("responsiveness.resize.ended");
                }
                crate::host::application::native::resize::ResizeInteractionPhase::Cancelled => {
                    crate::runtime::instrumentation::instant!("responsiveness.resize.cancelled");
                }
            }
            self.presentation.resize(update)?;
            resize_barrier_revision = resize_revision_to_synchronize(source, update);
            self.mark_redraw(RedrawReason::Resize);
            if self.drawable
                && let Some(runtime) = self.runtime.as_mut()
            {
                runtime.queue_input(PlatformInput::Resize(SizeF {
                    width: extent.width as f32,
                    height: extent.height as f32,
                }));
            }
        }
        if !self.redraw_eligible() {
            self.redraw.cancel_native_request();
            self.flush_commands();
            return Ok(());
        }
        let frame_started_at = Instant::now();
        self.frame_pacer.frame_started(frame_started_at);
        #[cfg(feature = "profiler")]
        let _frame = crate::runtime::instrumentation::start_frame("frame.total");
        let timestamp = self.timestamp_at(frame_started_at);
        let frame_interval = self.refresh_frame_interval();
        self.process_runtime_turn(timestamp);
        self.flush_commands();
        self.mark_runtime_frame_demand();
        if (source == RedrawSource::NativeCallback && !requested_callback)
            || !self.redraw.has_demand()
        {
            self.mark_redraw(RedrawReason::OperatingSystem);
        }
        let reasons = self.redraw.take_reasons();
        let force_present = reasons.force_present();
        #[cfg(feature = "profiler")]
        crate::runtime::instrumentation::counter!(
            "frame.trigger.pointer_move_only",
            u8::from(reasons.pointer_move_only())
        );
        let result = if let Some(runtime) = self.runtime.as_mut() {
            #[cfg(feature = "profiler")]
            crate::runtime::instrumentation::counter!(
                "frame.refresh_interval_ns",
                frame_interval.as_nanos()
            );
            let prepared = runtime
                .prepare_frame(timestamp, false)
                .map_err(|error| format!("frame preparation failed: {error}"))?;
            let mut deltas = Vec::new();
            {
                #[cfg(feature = "profiler")]
                let _span = crate::runtime::instrumentation::span!("transport.coalesce");
                while let Some(delta) = runtime.pop_scene_delta() {
                    deltas.push(delta);
                }
            }
            let physical_extent = self
                .live_resize
                .latest_extent()
                .or_else(|| {
                    self.window.as_ref().map(|window| {
                        let size = window.inner_size();
                        SizeI {
                            width: size.width as i32,
                            height: size.height as i32,
                        }
                    })
                })
                .unwrap_or_default();
            let frame = PreparedPresentationFrame {
                changed: prepared.changed,
                scene_epoch: prepared.scene_epoch,
                metrics: SurfaceMetrics {
                    revision: SurfaceRevision::new(self.live_resize.latest_metrics_revision()),
                    logical_extent: runtime.extent(),
                    physical_extent,
                    scale_factor: self
                        .window
                        .as_ref()
                        .map_or(1.0, |window| window.scale_factor()),
                    color_space: ColorSpace::Srgb,
                    alpha_mode: AlphaMode::Opaque,
                }
                .validate()
                .map_err(|error| error.to_string())?,
                deltas,
                frame_interval,
                force_present,
            };
            {
                #[cfg(feature = "profiler")]
                let _span = crate::runtime::instrumentation::span!("presentation.render");
                self.presentation.present(frame)
            }
        } else {
            return Ok(());
        };
        match result {
            Ok(PresentationAction::Idle) => {
                self.diagnostics.presentations_idle =
                    self.diagnostics.presentations_idle.saturating_add(1);
            }
            Ok(PresentationAction::Submitted) => {
                self.diagnostics.presentations_submitted =
                    self.diagnostics.presentations_submitted.saturating_add(1);
            }
            Err(error) => return Err(error),
        }
        if let Some(metrics_revision) = resize_barrier_revision {
            let synchronized = self
                .presentation
                .synchronize_resize(metrics_revision, WINDOW_RESIZE_PRESENT_TIMEOUT)?;
            #[cfg(not(any(target_os = "windows", feature = "profiler")))]
            let _ = synchronized;
            #[cfg(target_os = "windows")]
            if synchronized {
                flush_windows_compositor();
            }
            #[cfg(feature = "profiler")]
            if !synchronized {
                crate::runtime::instrumentation::instant!(
                    "responsiveness.resize.present_barrier_timeout"
                );
            }
        }
        self.flush_commands();
        Ok(())
    }
}

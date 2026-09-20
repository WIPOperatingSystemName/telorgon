use super::*;

impl<S, P> ApplicationHandler<HostEvent> for NativeHost<S, P>
where
    S: NativeRuntimeSource + 'static,
    P: NativePresentation + 'static,
{
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.suspended = false;
        if let Some(window) = self.window.clone() {
            if let Err(error) = self.presentation.resume(Arc::clone(&window)) {
                self.fail(event_loop, error);
                return;
            }
            let size = window.inner_size();
            self.drawable = size.width > 0 && size.height > 0;
            let _ = self.pending_resize.queue(SizeI {
                width: size.width as i32,
                height: size.height as i32,
            });
            self.mark_redraw(RedrawReason::Recovery);
            return;
        }
        let Some(source) = self.pending_source.take() else {
            self.fail(
                event_loop,
                "native application cannot be resumed after its state was lost",
            );
            return;
        };
        let options = &self.options;
        let window_icon = match source.window_icon(&options.icon) {
            Ok(icon) => icon,
            Err(error) => {
                self.fail(event_loop, error.to_string());
                return;
            }
        };
        let pointer = match source.managed_pointer() {
            Ok(pointer) => pointer,
            Err(error) => {
                self.fail(event_loop, error.to_string());
                return;
            }
        };
        let mut attributes = WindowAttributes::default()
            .with_title(options.title.clone())
            .with_decorations(
                options.decorations == crate::host::application::WindowDecorationMode::System,
            )
            .with_window_icon(window_icon)
            .with_inner_size(Size::Logical(LogicalSize::new(
                f64::from(options.size.width.max(1)),
                f64::from(options.size.height.max(1)),
            )));
        if let Some(minimum) = options.min_size {
            attributes = attributes.with_min_inner_size(Size::Logical(LogicalSize::new(
                f64::from(minimum.width.max(1)),
                f64::from(minimum.height.max(1)),
            )));
        }
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                self.fail(event_loop, format!("window creation failed: {error}"));
                return;
            }
        };
        if let Err(error) = self.resize_signals.register_window(&window) {
            self.fail(event_loop, error);
            return;
        }
        let registration = match self.views.register(window.id()) {
            Ok(registration) => registration,
            Err(error) => {
                self.fail(
                    event_loop,
                    format!("failed to register managed view: {error}"),
                );
                return;
            }
        };
        self.view = Some(registration.view);
        self.window = Some(Arc::clone(&window));
        self.pointer = Some(pointer);
        if let Err(error) = self.presentation.attach(Arc::clone(&window)) {
            self.fail(event_loop, error);
            return;
        }
        let mut runtime = match source.mount(options.size) {
            Ok(runtime) => runtime,
            Err(error) => {
                self.fail(event_loop, format!("application mount failed: {error}"));
                return;
            }
        };
        let size = window.inner_size();
        self.drawable = size.width > 0 && size.height > 0;
        runtime.queue_input(PlatformInput::Resize(SizeF {
            width: size.width.max(1) as f32,
            height: size.height.max(1) as f32,
        }));
        self.runtime = Some(runtime);
        self.mark_redraw(RedrawReason::Startup);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: HostEvent) {
        if self.failure.is_some() {
            event_loop.exit();
            return;
        }
        match event {
            HostEvent::ExitRequested => event_loop.exit(),
            HostEvent::RuntimeWake => {
                self.host_wake_pending = false;
                self.poll_presentation(event_loop);
            }
            #[cfg(all(feature = "application-vulkan-windows", target_os = "windows"))]
            HostEvent::PresentationWake => {
                self.poll_presentation(event_loop);
            }
            HostEvent::ResizeSignalChanged {
                signal,
                observed_at,
            } => {
                if !self.poll_presentation(event_loop) || signal.is_active() {
                    return;
                }
                let _ = self.complete_live_resize_release(event_loop, signal, observed_at);
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self
            .window
            .as_ref()
            .is_none_or(|window| window.id() != window_id)
        {
            return;
        }
        if !self.poll_presentation(event_loop) {
            return;
        }
        match event {
            WindowEvent::CloseRequested => {
                if let Some(runtime) = self.runtime.as_mut()
                    && let Err(error) = S::close(runtime)
                {
                    self.fail(event_loop, format!("component close failed: {error}"));
                    return;
                }
                self.flush_commands();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                let event_extent = SizeI {
                    width: size.width as i32,
                    height: size.height as i32,
                };
                let current_extent = self.window.as_ref().map_or(event_extent, |window| {
                    let current = window.inner_size();
                    SizeI {
                        width: current.width as i32,
                        height: current.height as i32,
                    }
                });
                let live_resize_active = self.resize_signal().is_some_and(
                    crate::host::application::native::resize::ResizeSignalSnapshot::is_active,
                );
                if !should_accept_winit_resize(live_resize_active, event_extent, current_extent) {
                    #[cfg(feature = "profiler")]
                    crate::runtime::instrumentation::instant!(
                        "responsiveness.resize.stale_winit_event_rejected"
                    );
                    return;
                }
                self.drawable = event_extent.width > 0 && event_extent.height > 0;
                let queued = self.pending_resize.queue(event_extent);
                if queued {
                    self.mark_redraw(RedrawReason::Resize);
                }
                if !self.drawable {
                    // Zero-sized surfaces must be suspended even if the operating system stops
                    // delivering paint callbacks while the window is minimized. The next nonzero
                    // resize updates `drawable` before scheduling its recovery frame.
                    self.redraw(event_loop, RedrawSource::SynchronousResize);
                    return;
                }
                let frame_interval = self.refresh_frame_interval();
                let resize_frame_due = self.pending_resize.is_due(Instant::now(), frame_interval);
                if should_apply_resize_synchronously(live_resize_active, resize_frame_due) {
                    // CPU-side input/layout/scene preparation is safe in WM_SIZE. Vulkan WSI work
                    // is only published to the presentation worker and never runs here. Outside
                    // the native sizing loop, defer so startup and programmatic resize bursts can
                    // collapse to their final extent before rendering.
                    self.redraw(event_loop, RedrawSource::SynchronousResize);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                #[cfg(feature = "profiler")]
                record_gui_input(
                    crate::runtime::instrumentation::InputRecordingSource::PointerMotion,
                    "input.gui.pointer_motion",
                );
                self.diagnostics.native_pointer_moves =
                    self.diagnostics.native_pointer_moves.saturating_add(1);
                self.cursor_position = PointF {
                    x: position.x as f32,
                    y: position.y as f32,
                };
                if let Some(runtime) = self.runtime.as_mut() {
                    runtime.queue_input(InputEvent::mouse_moved(self.cursor_position));
                }
            }
            WindowEvent::CursorLeft { .. } => {
                #[cfg(feature = "profiler")]
                record_gui_input(
                    crate::runtime::instrumentation::InputRecordingSource::PointerMotion,
                    "input.gui.pointer_motion.leave",
                );
                if let Some(runtime) = self.runtime.as_mut() {
                    runtime.queue_input(InputEvent::mouse_moved(PointF { x: -1.0, y: -1.0 }));
                }
                self.cursor_position = PointF { x: -1.0, y: -1.0 };
            }
            WindowEvent::MouseInput { state, button, .. } => {
                #[cfg(feature = "profiler")]
                record_gui_input(
                    crate::runtime::instrumentation::InputRecordingSource::PointerButton,
                    "input.gui.pointer_button",
                );
                let custom_action = (state == ElementState::Pressed
                    && button == winit::event::MouseButton::Left)
                    .then(|| self.custom_chrome_action())
                    .flatten();
                if let Some(runtime) = self.runtime.as_mut() {
                    let button = mouse_button(button);
                    let state = match state {
                        ElementState::Pressed => ButtonState::Pressed,
                        ElementState::Released => ButtonState::Released,
                    };
                    runtime.queue_input(InputEvent::mouse_button(button, state));
                }
                if let Some(action) = custom_action {
                    self.apply_custom_chrome_action(event_loop, action);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                #[cfg(feature = "profiler")]
                record_gui_input(
                    crate::runtime::instrumentation::InputRecordingSource::Scroll,
                    "input.gui.scroll",
                );
                let delta = match delta {
                    MouseScrollDelta::LineDelta(x, y) => PointF {
                        x: x * 24.0,
                        y: y * 24.0,
                    },
                    MouseScrollDelta::PixelDelta(position) => PointF {
                        x: position.x as f32,
                        y: position.y as f32,
                    },
                };
                if let Some(runtime) = self.runtime.as_mut() {
                    runtime.queue_input(InputEvent::mouse_scroll(delta));
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                #[cfg(feature = "profiler")]
                record_gui_input(
                    crate::runtime::instrumentation::InputRecordingSource::Keyboard,
                    "input.gui.keyboard",
                );
                if let WinitPhysicalKey::Code(code) = event.physical_key
                    && let Some(runtime) = self.runtime.as_mut()
                {
                    runtime.queue_input(InputEvent::Key(KeyEvent {
                        physical_key: PhysicalKey::new(code as u32),
                        state: match event.state {
                            ElementState::Pressed => ButtonState::Pressed,
                            ElementState::Released => ButtonState::Released,
                        },
                        repeat: event.repeat,
                        modifiers: Modifiers::empty(),
                        ..KeyEvent::new(PhysicalKey::new(code as u32), ButtonState::Pressed)
                    }));
                }
            }
            WindowEvent::RedrawRequested => self.redraw(event_loop, RedrawSource::NativeCallback),
            WindowEvent::Occluded(occluded) => {
                self.occluded = occluded;
                if occluded {
                    self.redraw.cancel_native_request();
                } else {
                    self.mark_redraw(RedrawReason::Expose);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if !self.poll_presentation(event_loop) {
            return;
        }
        if !self.finalize_live_resize(event_loop) {
            return;
        }
        if cfg!(target_os = "windows")
            && self.resize_signal().is_some_and(
                crate::host::application::native::resize::ResizeSignalSnapshot::is_active,
            )
        {
            let frame_interval = self.refresh_frame_interval();
            if self.pending_resize.is_due(Instant::now(), frame_interval) {
                self.mark_redraw(RedrawReason::Resize);
                self.redraw(event_loop, RedrawSource::SynchronousResize);
                if self.failure.is_some() {
                    return;
                }
            }
        }
        let _ = self.apply_post_turn_schedule(event_loop, Instant::now());
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        self.suspended = true;
        self.redraw.cancel_native_request();
        self.live_resize.cancel();
        if let Err(error) = self.presentation.suspend() {
            self.fail(event_loop, error);
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if let Err(error) = self.presentation.shutdown() {
            eprintln!("telorgon-app: {error}");
            self.failure.get_or_insert(error);
        }
        if let Some(window) = self.window.as_deref()
            && let Err(error) = self.resize_signals.unregister_window(window)
        {
            eprintln!("telorgon-app: {error}");
            self.failure.get_or_insert(error);
        }
        if let (Some(view), Some(window)) = (self.view.take(), self.window.as_deref())
            && let Err(error) = self.views.retire(view, window.id())
        {
            eprintln!("telorgon-app: failed to retire managed view: {error}");
            self.failure.get_or_insert(error.to_string());
        }
    }
}

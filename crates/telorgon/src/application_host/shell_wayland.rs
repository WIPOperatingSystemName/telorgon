#[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
mod compatibility;
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::Arc;
#[cfg(feature = "profiler")]
use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[cfg(test)]
#[path = "shell_wayland/transparency_tests.rs"]
mod transparency_tests;

use crate::compositor_render::{shm_image_metadata, shm_image_resource, shm_image_update};
use crate::compositor_wayland::{
    BufferDescriptor, BufferTransform, ButtonState as WaylandButtonState, ClientLimits,
    CompositorAction, CursorImage, NativeCompositor, OutputDescription, OutputMode, OutputState,
    OutputTransform, PointerConstraintKind, PointerConstraintState, ResizeEdge, SeatCapabilities,
    SeatState as WaylandSeatState, SurfaceRole, ToplevelState, WaylandSurfaceId,
};
use crate::core::{MonotonicInstant, PointF, PointI, RectI, SizeF, SizeI};
use crate::platform_linux::{
    KeyDirection, LibInputContext, LinuxInputEventKind, LinuxSeat, SeatState, XkbKeyboard,
};
use crate::presenter_vulkan_kms::{
    AtomicRequest, ConnectorStatus, DRM_FORMAT_ARGB8888, DRM_PLANE_TYPE_CURSOR, FrameSlot,
    FrameSlotState, GbmBuffer, GbmDevice, KmsCrtcId, KmsDevice, KmsFramebuffer,
    KmsObjectProperties, KmsPlaneId, KmsPropertyObject, KmsTopology,
};
use crate::render::{ImageAlphaMode, ImageId, ImagePixelFormat, RenderSceneDelta};
use crate::runtime::CompositionDriver;
use crate::wayland_server::Display;
use crate::{
    AssetBundle, AssetMediaCache, AssetRasterSize, CursorGraphic, PointerConfiguration,
    PointerIcon, PointerRequest, PointerResolution, PointerTheme, WindowAction, WindowChromeModel,
    WindowChromeSnapshot, WindowChromeState, WindowResizeEdge, resolve_pointer,
};

use crate::application_host::declaration::ShellActionHandler;
use crate::application_host::{
    AppError, AppResult, ComposedAppRuntime, LinuxShellConfig, ReadyShellEnvironment,
    WindowFrameFactory,
};

// Keep this root focused on assembling resources and running the single Wayland/KMS owner loop.
// Each stateful or policy-heavy subsystem below owns its own invariants and focused tests.
mod client;
mod window_backend;
use window_backend::WindowBackend;
mod window_identity;
#[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
mod x11_windows;
use window_identity::WindowIdentities;
mod cursor_plane;
mod dma_buf_readiness;
mod event_source;
mod frame_stats;
mod geometry;
mod input;
mod widgets;
use widgets::*;
mod interaction;
mod latency_trace;
mod layers;
mod motion;
mod pointer_visual;
mod renderer;
mod resize_trace;
mod scene;
mod shm_copy;
mod shortcuts;
mod size_policy;
mod state;

use client::{
    ClientWindow, PreparedClientImage, apply_surface_publication, discard_replaced_shm_copy,
    discard_shm_copy, finish_dma_buf_release, finish_shm_copy,
    observe_surface_configure_acknowledgement, resize_veil_owner, retire_submitted_dma_buf,
    retire_unsubmitted_dma_buf,
};
#[cfg(test)]
use cursor_plane::{CursorCommitTracker, HARDWARE_CURSOR_BUFFER_COUNT};
use cursor_plane::{HardwareCursor, PendingKmsCommit};
use event_source::{EventNotifier, InputReadyState, mark_external_fd_ready, mark_input_fd_ready};
#[cfg(feature = "profiler")]
use event_source::{PointerBatchProbe, PointerCursorPath, record_pointer_event_latency};
use geometry::*;
use input::*;
use interaction::*;
use layers::*;
use pointer_visual::*;
use renderer::{DmaBufPublication, ShellRenderResult};
#[cfg(test)]
use renderer::{
    VULKAN_STAGING_HEADROOM_BYTES_PER_SLOT, VULKAN_STAGING_MIN_BYTES_PER_SLOT,
    vulkan_staging_budget_bytes,
};
use scene::{
    ShellComposition, ShellImageRegion, ShellImageUpdate, ShellLayer, ShellLayerKey, ShellSceneKey,
};
use shm_copy::{ShmCopyCompletion, ShmCopyRequest, ShmCopyWorker};
use state::{
    ConfigureScheduler, FinalResizeConfigure, NativeConfigureState, PendingResizeConfigure,
    ResizeAnchor, SurfacePlacement, take_ready_deferred_shm_surface,
};

const MAX_DEFERRED_SHM_COPIES: usize = 64;

pub(crate) fn run(application: ReadyShellEnvironment) -> AppResult<()> {
    let _profiler = crate::application_host::profiler::ManagedProfiler::start(
        crate::application_host::profiler::ProfileTarget::ShellEnvironment,
    )?;
    #[cfg(feature = "profiler")]
    let _profile_view = {
        let view = crate::profiler::ProfileViewId::PRIMARY;
        let _ = crate::profiler::register_view(view, "Shell environment output");
        crate::profiler::enter_view(Some(view))
    };
    let (
        _name,
        compositor,
        widgets,
        services,
        renderer,
        config,
        assets,
        pointer_config,
        app_icon_profile,
    ) = application.into_parts()?;
    let launch_environment = crate::session::Environment::inherited();
    let runtime_directory = launch_environment.runtime_directory().map_err(app_error)?;
    let pointer_theme = pointer_config.load_theme(assets).map_err(app_error)?;
    let mut pointer_media = AssetMediaCache::new(assets).map_err(app_error)?;
    let (window_frame, mut pointer, mut icons, shell_actions, mut keyboard_shortcut_handler) =
        compositor.into_runtime_parts();

    let seat = LinuxSeat::open_with_deferred_disable().map_err(app_error)?;
    seat.dispatch(0).map_err(app_error)?;
    let input = LibInputContext::new(&seat, &config.seat_name).map_err(app_error)?;
    let (_drm_seat_device, kms, topology, drm_path) = select_drm_device(&seat, &config)?;
    let connector = topology
        .connectors
        .iter()
        .find(|connector| {
            connector.status == ConnectorStatus::Connected && !connector.modes.is_empty()
        })
        .ok_or_else(|| AppError::new("no connected KMS output with a display mode was found"))?;
    let mode_index = connector
        .modes
        .iter()
        .position(|mode| mode.preferred())
        .unwrap_or(0);
    let mode = connector.modes[mode_index];
    let physical_extent = mode.size();
    let output_scale = config
        .output_scale
        .resolve(physical_extent, connector.physical_millimeters)?;
    let selected_output = output_state(connector, mode_index, output_scale)?;
    let extent = selected_output.logical_size();
    eprintln!(
        "telorgon-kms: {}x{} pixels, {}x{} mm, {:.0}% scale, {}x{} logical units ({:?})",
        physical_extent.width,
        physical_extent.height,
        connector.physical_millimeters.width,
        connector.physical_millimeters.height,
        output_scale.get() * 100.0,
        extent.width,
        extent.height,
        config.output_scale
    );
    let refresh_period =
        Duration::from_nanos(1_000_000_000_000_u64 / u64::from(mode.refresh_millihertz().max(1)));
    let mode_blob = kms.create_mode_blob(&mode).map_err(app_error)?;
    let gbm = match GbmDevice::new(kms.fd()) {
        Ok(gbm) => Some(gbm),
        Err(error) if error.retryable() => {
            eprintln!("telorgon-kms: {error}; evaluating CPU dumb-buffer support");
            None
        }
        Err(error) => return Err(app_error(error)),
    };
    eprintln!(
        "telorgon-kms: negotiating {renderer:?} scanout on {}",
        drm_path.display()
    );
    // Renderer-owned acquire watches must unregister before libwayland destroys its event loop.
    // Rust drops locals in reverse declaration order, including on every error return.
    #[allow(unused_mut)]
    let mut display = Display::new().map_err(app_error)?;
    let mut scanout = renderer::prepare(
        &kms,
        gbm.as_ref(),
        &topology,
        connector,
        mode_blob.id(),
        physical_extent,
        renderer,
    )?;
    let crtc_index = scanout.crtc_index;
    let crtc = scanout.crtc;
    let plane = topology
        .planes
        .iter()
        .find(|plane| plane.id == scanout.plane)
        .expect("selected plane exists");
    let connector_properties = &scanout.connector_properties;
    let crtc_properties = &scanout.crtc_properties;
    let plane_properties = &scanout.plane_properties;
    let scanout_buffers = &mut scanout.buffers;
    let framebuffers = &scanout.framebuffers;
    let desktop_renderer = &mut scanout.renderer;
    let mut frame_slots = framebuffers
        .iter()
        .enumerate()
        .map(|(index, framebuffer)| FrameSlot::new(index, framebuffer.id()))
        .collect::<Vec<_>>();
    let mut desktop_scene = ShellComposition::new(extent);
    let mut window_motion = motion::WindowMotionController::default();
    let mut frame_surface_revisions = vec![Vec::<(u32, u64)>::new(); frame_slots.len()];
    let cursor_plane = topology.planes.iter().find(|candidate| {
        candidate.possible_crtcs_mask & (1_u32.checked_shl(crtc_index as u32).unwrap_or(0)) != 0
            && candidate.formats.contains(&DRM_FORMAT_ARGB8888)
            && is_plane_type(&kms, candidate.id.get(), DRM_PLANE_TYPE_CURSOR)
    });
    let mut hardware_cursor = cursor_plane.and_then(|cursor_plane| {
        KmsTopology::object_properties(&kms, cursor_plane.id.get(), KmsPropertyObject::Plane)
            .map_err(app_error)
            .and_then(|properties| {
                gbm.as_ref()
                    .ok_or_else(|| AppError::new("GBM hardware cursor unavailable"))
                    .and_then(|gbm| HardwareCursor::new(gbm, &kms, cursor_plane.id, properties))
            })
            .ok()
    });
    #[cfg(feature = "profiler")]
    crate::profiler::record_instant(if hardware_cursor.is_some() {
        "presentation.cursor.hardware_available"
    } else {
        "presentation.cursor.composited_fallback"
    });

    #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
    let xwayland_access =
        crate::compositor_wayland::XwaylandAccess::configure_display(&mut display)
            .map_err(app_error)?;
    #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
    xwayland_access.set_coordinate_scale(output_scale.get().ceil() as i32);
    #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
    let mut wayland = NativeCompositor::new_with_xwayland(
        &display,
        ClientLimits::default(),
        xwayland_access.clone(),
    )
    .map_err(app_error)?;
    #[cfg(not(all(feature = "shell-xwayland", target_env = "gnu")))]
    let mut wayland =
        NativeCompositor::new(&display, ClientLimits::default()).map_err(app_error)?;
    wayland
        .add_output(&display, 1, selected_output)
        .map_err(app_error)?;
    wayland
        .add_seat(
            &display,
            1,
            WaylandSeatState::new(
                &config.seat_name,
                SeatCapabilities {
                    pointer: true,
                    keyboard: true,
                    touch: true,
                },
            ),
        )
        .map_err(app_error)?;
    let dma_buf_formats = desktop_renderer.dma_buf_formats();
    if !dma_buf_formats.is_empty() {
        use std::os::unix::fs::MetadataExt;
        // PreparedVulkan matched the renderer to this exact KMS device before importing scanout.
        // The protocol accepts a primary node; Xwayland resolves its corresponding render node.
        let device_file = std::fs::File::from(kms.fd().try_clone().map_err(app_error)?);
        let main_device = device_file.metadata().map_err(app_error)?.rdev();
        let format_count = dma_buf_formats.len();
        wayland
            .add_linux_dmabuf_with_feedback(&display, dma_buf_formats, main_device)
            .map_err(app_error)?;
        wayland
            .add_explicit_synchronization(&display)
            .map_err(app_error)?;
        eprintln!(
            "telorgon-dmabuf: v4 feedback enabled device={}:{} format_modifier_pairs={format_count}",
            libc::major(main_device),
            libc::minor(main_device)
        );
    } else {
        eprintln!(
            "telorgon-dmabuf: unavailable; client buffers use SHM (Xwayland GPU acceleration unavailable)"
        );
    }
    let socket = display
        .add_socket_in(&runtime_directory, config.socket_name.as_deref())
        .map_err(app_error)?;
    let environment =
        launch_environment.desktop(&runtime_directory, &socket, &config.session.identity);
    #[cfg(all(feature = "shell-xwayland-embedded", target_env = "gnu"))]
    let session_start = if config.xwayland_enabled {
        crate::session::SessionOwner::start_waiting_for_x11
    } else {
        crate::session::SessionOwner::start
    };
    #[cfg(not(all(feature = "shell-xwayland-embedded", target_env = "gnu")))]
    let session_start = crate::session::SessionOwner::start;
    let mut session = session_start(environment, config.session.clone()).map_err(app_error)?;
    if config.session.publish_user_service_environment {
        session.publish_services().map_err(app_error)?;
    }
    eprintln!(
        "telorgon-session: ready on {}",
        runtime_directory.join(&socket).display()
    );

    // Libwayland already owns the compositor's poll loop. Register every external readiness
    // source with it so input, seat changes, DRM flips, and GPU completions wake the same owner
    // thread without a fixed Wayland-only sleep.
    let runtime_wake = EventNotifier::new("desktop runtime wake")?;
    #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
    let mut compatibility: Option<compatibility::Compatibility> = None;
    #[cfg(all(feature = "shell-xwayland-embedded", target_env = "gnu"))]
    if config.xwayland_enabled {
        let cache = config.xwayland_cache.clone().unwrap_or_else(|| {
            launch_environment
                .get("XDG_CACHE_HOME")
                .map(std::path::PathBuf::from)
                .or_else(|| {
                    launch_environment
                        .get("HOME")
                        .map(|home| std::path::PathBuf::from(home).join(".cache"))
                })
                .unwrap_or_else(|| runtime_directory.clone())
                .join("telorgon/xwayland")
        });
        match compatibility::Compatibility::prepare(
            xwayland_access,
            crate::xwayland::payload::embedded(),
            cache,
            runtime_directory.clone(),
            launch_environment.clone(),
            runtime_wake.clone(),
        ) {
            Ok(host) => compatibility = Some(host),
            Err(error) => {
                session.set_x11_environment(None);
                eprintln!("telorgon-xwayland: {error}; native desktop remains active");
            }
        }
    }
    #[cfg(all(
        feature = "shell-xwayland",
        not(feature = "shell-xwayland-embedded"),
        target_env = "gnu"
    ))]
    if config.xwayland_enabled {
        eprintln!(
            "telorgon-xwayland: embedded payload is unavailable in this build; native desktop remains active"
        );
    }
    let termination_signals = event_source::TerminationSignals::new(runtime_wake.clone())?;
    let exit_request = super::exit::HostExit::register({
        let wake = runtime_wake.clone();
        move || wake.notify()
    });
    let shm_copy_worker = ShmCopyWorker::new(runtime_wake.clone())?;

    if let Some(driver) = &mut pointer {
        driver.set_wake({
            let wake = runtime_wake.clone();
            move || wake.notify()
        });
    }
    for (_, driver) in &mut icons {
        driver.set_wake({
            let wake = runtime_wake.clone();
            move || wake.notify()
        });
    }

    let seat_ready = Box::new(AtomicBool::new(true));
    let input_ready = Box::new(InputReadyState::new(true));
    let kms_ready = Box::new(AtomicBool::new(false));
    let runtime_ready = Box::new(AtomicBool::new(false));
    let vulkan_completion_ready = desktop_renderer
        .is_vulkan()
        .then(|| Box::new(AtomicBool::new(false)));
    let external_mask = crate::wayland_server::ffi::WL_EVENT_READABLE
        | crate::wayland_server::ffi::WL_EVENT_HANGUP
        | crate::wayland_server::ffi::WL_EVENT_ERROR;
    let _seat_source = unsafe {
        display.event_loop().add_fd(
            seat.event_fd(),
            external_mask,
            Some(mark_external_fd_ready),
            std::ptr::from_ref(seat_ready.as_ref()).cast_mut().cast(),
        )
    }
    .map_err(app_error)?;
    let _input_source = unsafe {
        display.event_loop().add_fd(
            input.event_fd(),
            external_mask,
            Some(mark_input_fd_ready),
            std::ptr::from_ref(input_ready.as_ref()).cast_mut().cast(),
        )
    }
    .map_err(app_error)?;
    let _kms_source = unsafe {
        display.event_loop().add_fd(
            kms.fd().as_raw_fd(),
            external_mask,
            Some(mark_external_fd_ready),
            std::ptr::from_ref(kms_ready.as_ref()).cast_mut().cast(),
        )
    }
    .map_err(app_error)?;
    let _runtime_source = unsafe {
        display.event_loop().add_fd(
            runtime_wake.event_fd(),
            external_mask,
            Some(mark_external_fd_ready),
            std::ptr::from_ref(runtime_ready.as_ref()).cast_mut().cast(),
        )
    }
    .map_err(app_error)?;
    let _vulkan_completion_source = match (
        desktop_renderer.completion_event_fd(),
        &vulkan_completion_ready,
    ) {
        (Some(event_fd), Some(ready)) => Some(
            unsafe {
                display.event_loop().add_fd(
                    event_fd,
                    external_mask,
                    Some(mark_external_fd_ready),
                    std::ptr::from_ref(ready.as_ref()).cast_mut().cast(),
                )
            }
            .map_err(app_error)?,
        ),
        _ => None,
    };

    let keyboard = XkbKeyboard::from_names_with_include_root(
        config.keyboard.rules.as_deref(),
        config.keyboard.model.as_deref(),
        config.keyboard.layout.as_deref(),
        config.keyboard.variant.as_deref(),
        config.keyboard.options.as_deref(),
        config.keyboard.include_root.as_deref(),
    )
    .map_err(app_error)?;
    let keymap = keyboard.keymap_file().map_err(app_error)?;
    wayland
        .keyboard_keymap(1, keymap.fd(), keymap.size())
        .map_err(app_error)?;

    let mut frame_layers = BTreeMap::<WaylandSurfaceId, WindowFrameLayer>::new();
    let mut pointer = pointer
        .map(|driver| Layer::new(driver, config.pointer_extent, assets, output_scale))
        .transpose()?;
    let mut icon_layers = icons
        .into_iter()
        .map(|(name, driver)| {
            Ok((
                name,
                Layer::new(
                    driver,
                    SizeI {
                        width: 24,
                        height: 24,
                    },
                    assets,
                    output_scale,
                )?,
            ))
        })
        .collect::<AppResult<Vec<_>>>()?;
    // Mount icon components immediately so semantic icon declarations are validated and retained.
    for (_, icon) in &mut icon_layers {
        icon.prepare(
            SizeI {
                width: 24,
                height: 24,
            },
            0,
            true,
        )?;
    }
    #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
    if let Some(compatibility) = compatibility.as_mut() {
        let visual = render_cursor_image(
            CursorImage::TelorgonDefault,
            &mut pointer,
            &mut icon_layers,
            &BTreeMap::new(),
            config.pointer_extent,
            0,
            &pointer_config,
            pointer_theme.as_ref(),
            &mut pointer_media,
            crate::platform::ScaleFactor::new(output_scale.get().ceil().clamp(1.0, 8.0))
                .map_err(app_error)?,
        )?;
        let cursor = match visual {
            Some(CursorVisual::Image(image)) => {
                Some(crate::xwayland::root_cursor::RootCursor::new(
                    image.size.width,
                    image.size.height,
                    (image.hotspot.x as f32 * output_scale.get().ceil().clamp(1.0, 8.0)) as i32,
                    (image.hotspot.y as f32 * output_scale.get().ceil().clamp(1.0, 8.0)) as i32,
                    &image.rgba,
                    image.premultiplied,
                ))
            }
            None => Some(crate::xwayland::root_cursor::RootCursor::new(
                1, 1, 0, 0, &[0; 4], true,
            )),
        };
        match cursor {
            Some(Ok(mut cursor)) => {
                cursor.density = output_scale.get().ceil().clamp(1.0, 8.0) as u16;
                compatibility.root_cursor = Some(cursor);
            }
            Some(Err(error)) => eprintln!("telorgon-xwayland: {error}; using built-in root arrow"),
            None => eprintln!(
                "telorgon-xwayland: composed default cursor cannot be exported; using built-in root arrow"
            ),
        }
    }
    let widget_services = crate::compose::shell_services::ShellServiceHost::with_registry(services);
    let mut widgets = widgets
        .into_iter()
        .enumerate()
        .map(|(id, widget)| {
            WidgetLayer::new(
                id as u32,
                widget,
                extent,
                assets,
                output_scale,
                &runtime_wake,
                widget_services.services.clone(),
            )
        })
        .collect::<AppResult<Vec<_>>>()?;
    let mut next_widget_id = widgets.len() as u32;
    let mut work_area = shell_work_area(extent, &widgets);

    let start = Instant::now();
    let mut pointer_position = PointF {
        x: extent.width as f32 * 0.5,
        y: extent.height as f32 * 0.5,
    };
    if let Some(cursor) = &mut hardware_cursor {
        cursor.move_to(output_scale.physical_point(pointer_position));
    }
    let mut drag_position = pointer_position;
    let mut pointer_focus = None;
    let mut touch_targets = BTreeMap::<i32, WaylandSurfaceId>::new();
    let mut windows = BTreeMap::<WaylandSurfaceId, ClientWindow>::new();
    let mut window_identities = WindowIdentities::default();
    let mut pending_shm_buffers =
        BTreeMap::<crate::compositor_wayland::WaylandBufferId, usize>::new();
    let mut pending_shm_surfaces = BTreeMap::<WaylandSurfaceId, usize>::new();
    let mut pending_dma_bufs = BTreeMap::<crate::compositor_wayland::WaylandBufferId, usize>::new();
    let mut submitted_shm_surfaces = BTreeSet::<WaylandSurfaceId>::new();
    let mut deferred_shm_copies = BTreeMap::<WaylandSurfaceId, ShmCopyRequest>::new();
    let mut deferred_shm_order = VecDeque::<WaylandSurfaceId>::new();
    let mut stacking_order = Vec::<WaylandSurfaceId>::new();
    let mut session_locked = false;
    let mut pending_session_lock = None;
    let mut window_interaction = None;
    let mut decoration_click = DecorationClick::default();
    let mut configure_scheduler = ConfigureScheduler::default();
    let mut resize_configure_budget = true;
    let mut next_window_offset = 0_i32;
    let mut next_frame_id = 1_u64;
    let mut frame_stats = frame_stats::FrameStats::from_env(mode.refresh_millihertz());
    let mut owner_stats_start = None::<Instant>;
    let mut latency_trace = latency_trace::LatencyTrace::from_env().map_err(app_error)?;
    let mut ready_scanout = VecDeque::<usize>::new();
    let mut pending_kms_commit = None::<PendingKmsCommit>;
    let mut current_scanout = None::<usize>;
    let mut first_modeset = true;
    let mut repaint = true;
    #[cfg(feature = "profiler")]
    let mut pending_primary_pointer_event_us = None::<u64>;
    #[cfg(feature = "profiler")]
    let mut pending_deferred_cursor_event_us = None::<u64>;
    #[cfg(feature = "profiler")]
    let mut frame_pointer_event_us = vec![None::<u64>; frame_slots.len()];
    #[cfg(feature = "profiler")]
    let mut previous_pointer_batch_us = None::<u64>;
    let mut keyboard = keyboard;
    let mut shortcut_keys = shortcuts::ShortcutKeys::default();
    let mut input_suspended = false;
    let mut resume_keyboard_focus = None;
    let mut widget_saved_focus = None;
    let mut widget_focus_active = false;
    let mut widget_buttons = BTreeSet::new();
    let mut widget_keys = BTreeSet::new();
    let mut shutdown_started = None::<Instant>;
    let mut close_requested = std::collections::BTreeSet::new();

    loop {
        latency_trace.phase("schedule");
        if let (Some(stats), Some(started)) = (&mut frame_stats, owner_stats_start.take()) {
            stats.owner_turn(started.elapsed());
        }
        if termination_signals.take_requested() {
            crate::request_exit();
        }
        if exit_request.requested() {
            let started = *shutdown_started.get_or_insert_with(|| {
                session.quiesce();
                Instant::now()
            });
            let surfaces = wayland.toplevel_surfaces();
            #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
            let x11_open = compatibility
                .as_mut()
                .is_some_and(|host| host.request_close());
            #[cfg(not(all(feature = "shell-xwayland", target_env = "gnu")))]
            let x11_open = false;
            if surfaces.is_empty() && !x11_open {
                session.close();
                return Ok(());
            }
            for surface in surfaces {
                if close_requested.insert(surface) {
                    wayland.close_toplevel(surface).map_err(app_error)?;
                }
            }
            display.flush_clients();
            if started.elapsed() >= config.session.shutdown_timeout {
                // Keep the compositor and clients alive when an app refuses to close. Destroying
                // the display or signalling its process here would discard unsaved-work dialogs.
                eprintln!(
                    "telorgon-session: shutdown cancelled because application windows remain open"
                );
                exit_request.cancel();
                shutdown_started = None;
                close_requested.clear();
                #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
                if let Some(host) = &mut compatibility {
                    host.cancel_close();
                }
                session.resume();
            }
        }
        if let Some(stats) = &mut frame_stats {
            stats.report();
        }
        let mut presentation_completed = false;
        let mut presented_surface_revisions = Vec::new();
        let mut pointer_motion_seen = false;
        let mut cursor_position_dirty = false;
        let mut pointer_primary_dirty = false;
        let mut pointer_scene_dirty = false;
        #[cfg(feature = "profiler")]
        let mut latest_pointer_event_this_turn = None::<u64>;
        let mut other_work_seen = repaint;
        let cursor_image_at_turn_start = wayland
            .core()
            .seats
            .get(&1)
            .expect("seat registered")
            .cursor;
        let schedule_now = MonotonicInstant::from_nanos(
            start.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64,
        );
        let active_window = wayland
            .core()
            .seats
            .get(&1)
            .and_then(|s| s.keyboard_focus)
            .map(|f| f.surface)
            .or(widget_saved_focus);
        widget_services.publish(
            windows
                .iter()
                .filter_map(|(surface, w)| {
                    let id = w.desktop_id?;
                    Some(crate::compose::ShellWindow {
                        id,
                        title: w.frame_title.clone().unwrap_or_else(|| {
                            wayland
                                .toplevel_metadata(*surface)
                                .map_or_else(String::new, |m| {
                                    if m.title.is_empty() {
                                        m.application_id.clone()
                                    } else {
                                        m.title.clone()
                                    }
                                })
                        }),
                        active: active_window == Some(*surface),
                        minimized: w.minimized,
                        maximized: w.maximized,
                    })
                })
                .collect(),
        );
        for command in widget_services.drain() {
            use crate::compose::{ShellRequestOutcome as Outcome, ShellWindowAction as Action};
            let target = windows
                .iter()
                .find(|(_, w)| w.desktop_id == Some(command.window))
                .map(|(id, _)| *id);
            let Some(surface) = target else {
                widget_services.complete(command.id, Outcome::Stale);
                continue;
            };
            if session_locked {
                widget_services.complete(command.id, Outcome::Denied);
                continue;
            }
            let result = match command.action {
                Action::Activate => {
                    if let Some(w) = windows.get_mut(&surface) {
                        w.minimized = false;
                    }
                    for w in &mut widgets {
                        w.focused = false;
                    }
                    widget_focus_active = false;
                    widget_saved_focus = None;
                    window_backend::focus(
                        &display,
                        &mut wayland,
                        &windows,
                        &mut configure_scheduler,
                        &mut stacking_order,
                        Some(surface),
                        #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
                        compatibility.as_mut(),
                    )
                }
                Action::SetMinimized(value) => {
                    if let Some(w) = windows.get_mut(&surface) {
                        w.minimized = value;
                    }
                    if value {
                        stacking_order.retain(|id| *id != surface);
                    } else if !stacking_order.contains(&surface) {
                        stacking_order.push(surface);
                    }
                    if value && active_window == Some(surface) {
                        wayland
                            .set_keyboard_focus(1, None, display.next_serial())
                            .map_err(app_error)?;
                    }
                    Ok(())
                }
                Action::SetMaximized(value) => set_window_maximized(
                    &mut windows,
                    &mut configure_scheduler,
                    surface,
                    value,
                    work_area,
                    &config,
                ),
                Action::Close => window_backend::close(
                    surface,
                    &windows,
                    &mut wayland,
                    #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
                    compatibility.as_mut(),
                ),
            };
            widget_services.complete(
                command.id,
                if result.is_ok() {
                    Outcome::Dispatched
                } else {
                    Outcome::Failed
                },
            );
            repaint = true;
        }
        sync_widget_children(
            &mut widgets,
            &mut next_widget_id,
            extent,
            assets,
            output_scale,
            &runtime_wake,
            &widget_services.services,
        )?;
        let previous_work_area = work_area;
        let work = crate::core::RectF {
            x: work_area.x as f32,
            y: work_area.y as f32,
            width: work_area.width as f32,
            height: work_area.height as f32,
        };
        prepare_widget_surfaces(
            &mut widgets,
            extent,
            work,
            schedule_now.as_nanos(),
            config.motion_preference,
        )?;
        work_area = shell_work_area(extent, &widgets);
        if work_area != previous_work_area {
            let maximized = windows
                .iter()
                .filter(|(_, w)| w.maximized && !w.fullscreen)
                .map(|(id, _)| *id)
                .collect::<Vec<_>>();
            for id in maximized {
                set_window_maximized(
                    &mut windows,
                    &mut configure_scheduler,
                    id,
                    true,
                    work_area,
                    &config,
                )?;
            }
            repaint = true;
        }
        sync_widget_focus(
            &mut widgets,
            &mut wayland,
            &display,
            &windows,
            session_locked,
            &mut widget_saved_focus,
            &mut widget_focus_active,
            schedule_now,
        )?;
        let (runtime_immediate, runtime_deadline, runtime_animation) = desktop_runtime_schedule(
            &frame_layers,
            pointer.as_ref(),
            &icon_layers,
            &widgets,
            schedule_now,
        );
        let scanout_available = frame_slots
            .iter()
            .any(|slot| slot.state == FrameSlotState::Available);
        let cursor_commit_ready = !first_modeset
            && hardware_cursor
                .as_ref()
                .is_some_and(HardwareCursor::needs_commit);
        let primary_render_ready = state::primary_render_budget(
            frame_slots
                .iter()
                .filter(|slot| slot.state == FrameSlotState::GpuSubmitted)
                .count(),
            ready_scanout.len(),
        );
        let immediate_work = ((repaint || runtime_immediate)
            && scanout_available
            && primary_render_ready)
            || (pending_kms_commit.is_none() && (!ready_scanout.is_empty() || cursor_commit_ready))
            || runtime_ready.load(Ordering::Acquire)
            || seat_ready.load(Ordering::Acquire)
            || input_ready.ready.load(Ordering::Acquire)
            || kms_ready.load(Ordering::Acquire)
            || vulkan_completion_ready
                .as_ref()
                .is_some_and(|ready| ready.load(Ordering::Acquire));
        let wait = if immediate_work {
            Some(Duration::ZERO)
        } else if let Some(deadline) = runtime_deadline {
            Some(Duration::from_nanos(
                deadline.as_nanos().saturating_sub(schedule_now.as_nanos()),
            ))
        } else if (runtime_animation || window_motion.active(schedule_now.as_nanos()))
            && pending_kms_commit.is_none()
        {
            Some(refresh_period)
        } else {
            None
        };
        let wait = if shutdown_started.is_some() {
            Some(
                wait.unwrap_or(Duration::from_millis(50))
                    .min(Duration::from_millis(50)),
            )
        } else {
            wait
        };
        #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
        let wait = compatibility.as_ref().map_or(wait, |host| host.wait(wait));
        latency_trace.event(
            "wait_requested",
            [
                wait.map_or(u64::MAX, |d| d.as_micros().min(u128::from(u64::MAX)) as u64),
                0,
                0,
                0,
            ],
        );
        latency_trace.phase("wayland_wait_dispatch");
        display.dispatch_and_flush(wait).map_err(app_error)?;
        latency_trace.phase("xwm_dispatch");
        owner_stats_start = frame_stats.as_ref().map(|_| Instant::now());
        #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
        if let Some(host) = &mut compatibility {
            host.dispatch(&display);
            if host.startup_complete() {
                session.set_x11_environment(host.environment());
            }
            let changed = host.sync_presentation(&mut windows, &mut window_identities, &config)?;
            repaint |= changed;
            pointer_scene_dirty |= changed;
            let focus_blocked = session_locked
                || wayland.session_locked()
                || input_suspended
                || seat.disable_pending()
                || seat.state() != SeatState::Enabled;
            repaint |= host.apply_focus(
                &display,
                &mut wayland,
                &windows,
                &mut stacking_order,
                focus_blocked,
            )?;
        }

        latency_trace.phase("seat_dispatch");
        if seat_ready.swap(false, Ordering::AcqRel) {
            seat.dispatch(0).map_err(app_error)?;
        }
        if seat.disable_pending() {
            // Close input device FDs before acknowledging access revocation.
            input.suspend();
            if !input_suspended {
                resume_keyboard_focus = wayland
                    .core()
                    .seats
                    .get(&1)
                    .and_then(|seat| seat.keyboard_focus)
                    .map(|focus| focus.surface);
            }
            input_suspended = true;
            seat.acknowledge_disable().map_err(app_error)?;
            input_ready.ready.store(false, Ordering::Release);
            wayland.suspend_seat_input(1).map_err(app_error)?;
            // Protocol cancellation also revokes the host's cached routing/capture.
            // A resumed device must start a new touch or decoration interaction.
            touch_targets.clear();
            for frame in frame_layers.values_mut() {
                frame
                    .layer
                    .runtime
                    .cancel_pointer(crate::input::PointerId::PRIMARY);
            }
            repaint = true;
            pointer_focus = None;
            for key in shortcut_keys.reset() {
                keyboard.update_key(key, KeyDirection::Up);
            }
            if let Some(interaction) = window_interaction.take() {
                finish_window_interaction(&mut windows, &mut configure_scheduler, interaction);
            }
            pointer_scene_dirty = true;
            display.flush_clients();
        }
        if input_suspended && seat.state() == SeatState::Enabled {
            input.resume().map_err(app_error)?;
            let deferred_keyboard_focus = wayland.resume_seat_input(1).map_err(app_error)?;
            input_suspended = false;
            input_ready.ready.store(true, Ordering::Release);
            if let Some(surface) = resume_keyboard_focus.take().filter(|surface| {
                !deferred_keyboard_focus
                    && !session_locked
                    && windows.get(surface).is_some_and(|window| !window.minimized)
                    && wayland.core().world.surface(*surface).is_some()
            }) {
                // Preserve focus chosen by lock/desktop policy while we were inactive.
                if wayland
                    .core()
                    .seats
                    .get(&1)
                    .is_some_and(|seat| seat.keyboard_focus.is_none())
                {
                    wayland
                        .set_keyboard_focus(1, Some(surface), display.next_serial())
                        .map_err(app_error)?;
                }
            }
            let modifiers = keyboard.modifiers();
            wayland
                .keyboard_modifiers(
                    1,
                    display.next_serial(),
                    modifiers.depressed,
                    modifiers.latched,
                    modifiers.locked,
                    modifiers.group,
                )
                .map_err(app_error)?;
        }
        latency_trace.phase("input_dispatch_route_flush");
        if seat.state() == SeatState::Enabled && input_ready.ready.swap(false, Ordering::AcqRel) {
            let mut input_stats = (frame_stats.is_some() || latency_trace.enabled()).then(|| {
                frame_stats::InputBatch::new(crate::platform_linux::monotonic_time_microseconds())
            });
            #[cfg(feature = "profiler")]
            let input_callback_time_us = input_ready.callback_time_us.swap(0, Ordering::AcqRel);
            #[cfg(feature = "profiler")]
            let mut pointer_probe = PointerBatchProbe::begin();
            #[cfg(feature = "profiler")]
            let input_dispatch_started_us = pointer_probe
                .enabled
                .then(crate::platform_linux::monotonic_time_microseconds)
                .flatten();
            input.dispatch().map_err(app_error)?;
            #[cfg(feature = "profiler")]
            pointer_probe.dispatch_completed();
            #[cfg(feature = "profiler")]
            let input_observed_us = crate::platform_linux::monotonic_time_microseconds();
            while let Some(event) = input.next_event() {
                let time_microseconds = event.time_microseconds;
                if let Some(stats) = &mut input_stats {
                    stats.observe(time_microseconds);
                }
                if matches!(
                    event.kind,
                    LinuxInputEventKind::PointerMotion { .. }
                        | LinuxInputEventKind::PointerAbsolute { .. }
                ) {
                    pointer_motion_seen = true;
                    #[cfg(feature = "profiler")]
                    pointer_probe.observe_motion(time_microseconds);
                } else {
                    other_work_seen = true;
                }
                #[cfg(feature = "profiler")]
                {
                    record_libinput_event(event.kind, time_microseconds, input_observed_us);
                }
                let time = time_microseconds / 1_000;
                let time = u32::try_from(time).unwrap_or(u32::MAX);
                match event.kind {
                    LinuxInputEventKind::PointerMotion {
                        delta,
                        unaccelerated,
                    } => {
                        // Libinput motion is normalized mouse movement, not KMS pixels.
                        // Apply it in logical units so pointer speed is stable across densities;
                        // preserve the device's unaccelerated vector for relative-pointer clients.
                        let scene_follows_pointer = window_interaction.is_some()
                            || (wayland.drag_active(1) && wayland.drag_touch_slot(1).is_none());
                        other_work_seen |= scene_follows_pointer;
                        let constraint =
                            if wayland.drag_active(1) && wayland.drag_touch_slot(1).is_none() {
                                None
                            } else {
                                wayland.pointer_constraint(1)
                            };
                        let locked = constraint.as_ref().is_some_and(|constraint| {
                            constraint.kind == PointerConstraintKind::Locked
                        });
                        if !locked {
                            let previous_position = pointer_position;
                            let proposed = PointF {
                                x: (pointer_position.x + delta.x)
                                    .clamp(0.0, extent.width as f32 - 1.0),
                                y: (pointer_position.y + delta.y)
                                    .clamp(0.0, extent.height as f32 - 1.0),
                            };
                            pointer_position = constraint.as_ref().map_or(proposed, |constraint| {
                                constrain_pointer(
                                    &windows,
                                    pointer_position,
                                    proposed,
                                    constraint,
                                    &config,
                                )
                            });
                            let widget_now = MonotonicInstant::from_nanos(
                                start.elapsed().as_nanos().min(u64::MAX as u128) as u64,
                            );
                            if window_interaction.is_none()
                                && wayland
                                    .core()
                                    .seats
                                    .get(&1)
                                    .is_none_or(|seat| seat.pressed_buttons().is_empty())
                                && widget_pointer_motion(
                                    &mut widgets,
                                    pointer_position,
                                    widget_now,
                                    session_locked,
                                )?
                            {
                                wayland
                                    .set_pointer_focus(
                                        1,
                                        None,
                                        pointer_position,
                                        display.next_serial(),
                                    )
                                    .map_err(app_error)?;
                                pointer_focus = None;
                                cursor_position_dirty = true;
                                repaint = true;
                                continue;
                            }
                            if let Some(interaction) = window_interaction.as_mut() {
                                apply_window_interaction(
                                    &mut windows,
                                    interaction,
                                    &mut configure_scheduler,
                                    pointer_position,
                                    extent,
                                    &config,
                                )?;
                            } else {
                                route_pointer_motion(
                                    &display,
                                    &mut wayland,
                                    &windows,
                                    &stacking_order,
                                    session_locked,
                                    &mut pointer_focus,
                                    pointer_position,
                                    time,
                                    &config,
                                )?;
                            }
                            if wayland.drag_active(1) && wayland.drag_touch_slot(1).is_none() {
                                drag_position = pointer_position;
                            }
                            if pointer_position.x != previous_position.x
                                || pointer_position.y != previous_position.y
                            {
                                repaint |= route_frame_pointer_motion(
                                    &mut frame_layers,
                                    &windows,
                                    pointer_position,
                                    session_locked,
                                    MonotonicInstant::from_nanos(
                                        time_microseconds.saturating_mul(1_000),
                                    ),
                                );
                                if window_interaction.is_none() {
                                    set_decoration_pointer_cursor(
                                        &mut wayland,
                                        &frame_layers,
                                        &windows,
                                        &stacking_order,
                                        pointer_focus,
                                        pointer_position,
                                        &config,
                                        &icon_layers,
                                    );
                                }
                                cursor_position_dirty = true;
                                pointer_primary_dirty |= scene_follows_pointer;
                            }
                        }
                        if pointer_focus.is_some()
                            && window_interaction.is_none()
                            && (!wayland.drag_active(1) || wayland.drag_touch_slot(1).is_some())
                        {
                            let density = pointer_focus
                                .and_then(|surface| windows.get(&surface))
                                .map_or(1, |window| window.surface_scale.max(1))
                                as f32;
                            let _ = wayland.relative_pointer_motion(
                                1,
                                time_microseconds,
                                PointF {
                                    x: delta.x * density,
                                    y: delta.y * density,
                                },
                                unaccelerated,
                            );
                        }
                    }
                    LinuxInputEventKind::PointerAbsolute { normalized } => {
                        let scene_follows_pointer = window_interaction.is_some()
                            || (wayland.drag_active(1) && wayland.drag_touch_slot(1).is_none());
                        other_work_seen |= scene_follows_pointer;
                        let proposed = PointF {
                            x: normalized.x.clamp(0.0, 1.0) * (extent.width - 1) as f32,
                            y: normalized.y.clamp(0.0, 1.0) * (extent.height - 1) as f32,
                        };
                        let constraint =
                            if wayland.drag_active(1) && wayland.drag_touch_slot(1).is_none() {
                                None
                            } else {
                                wayland.pointer_constraint(1)
                            };
                        if !constraint.as_ref().is_some_and(|constraint| {
                            constraint.kind == PointerConstraintKind::Locked
                        }) {
                            let previous_position = pointer_position;
                            pointer_position = constraint.as_ref().map_or(proposed, |constraint| {
                                constrain_pointer(
                                    &windows,
                                    pointer_position,
                                    proposed,
                                    constraint,
                                    &config,
                                )
                            });
                            let widget_now = MonotonicInstant::from_nanos(
                                start.elapsed().as_nanos().min(u64::MAX as u128) as u64,
                            );
                            if window_interaction.is_none()
                                && wayland
                                    .core()
                                    .seats
                                    .get(&1)
                                    .is_none_or(|seat| seat.pressed_buttons().is_empty())
                                && widget_pointer_motion(
                                    &mut widgets,
                                    pointer_position,
                                    widget_now,
                                    session_locked,
                                )?
                            {
                                wayland
                                    .set_pointer_focus(
                                        1,
                                        None,
                                        pointer_position,
                                        display.next_serial(),
                                    )
                                    .map_err(app_error)?;
                                pointer_focus = None;
                                cursor_position_dirty = true;
                                repaint = true;
                                continue;
                            }
                            if let Some(interaction) = window_interaction.as_mut() {
                                apply_window_interaction(
                                    &mut windows,
                                    interaction,
                                    &mut configure_scheduler,
                                    pointer_position,
                                    extent,
                                    &config,
                                )?;
                            } else {
                                route_pointer_motion(
                                    &display,
                                    &mut wayland,
                                    &windows,
                                    &stacking_order,
                                    session_locked,
                                    &mut pointer_focus,
                                    pointer_position,
                                    time,
                                    &config,
                                )?;
                            }
                            if wayland.drag_active(1) && wayland.drag_touch_slot(1).is_none() {
                                drag_position = pointer_position;
                            }
                            if pointer_position.x != previous_position.x
                                || pointer_position.y != previous_position.y
                            {
                                repaint |= route_frame_pointer_motion(
                                    &mut frame_layers,
                                    &windows,
                                    pointer_position,
                                    session_locked,
                                    MonotonicInstant::from_nanos(
                                        time_microseconds.saturating_mul(1_000),
                                    ),
                                );
                                if window_interaction.is_none() {
                                    set_decoration_pointer_cursor(
                                        &mut wayland,
                                        &frame_layers,
                                        &windows,
                                        &stacking_order,
                                        pointer_focus,
                                        pointer_position,
                                        &config,
                                        &icon_layers,
                                    );
                                }
                                cursor_position_dirty = true;
                                pointer_primary_dirty |= scene_follows_pointer;
                            }
                        }
                    }
                    LinuxInputEventKind::PointerButton { button, pressed } => {
                        let widget_now = MonotonicInstant::from_nanos(
                            start.elapsed().as_nanos().min(u64::MAX as u128) as u64,
                        );
                        let held = widget_buttons.contains(&button);
                        let consumed = if button == 0x110 {
                            widget_pointer_button(
                                &mut widgets,
                                pointer_position,
                                pressed,
                                widget_now,
                                session_locked,
                            )?
                        } else {
                            false
                        };
                        if consumed || held {
                            if pressed {
                                widget_buttons.insert(button);
                            } else {
                                widget_buttons.remove(&button);
                            }
                            sync_widget_focus(
                                &mut widgets,
                                &mut wayland,
                                &display,
                                &windows,
                                session_locked,
                                &mut widget_saved_focus,
                                &mut widget_focus_active,
                                widget_now,
                            )?;
                            repaint = true;
                            continue;
                        }

                        if button == 0x110
                            && !pressed
                            && !session_locked
                            && wayland
                                .core()
                                .seats
                                .get(&1)
                                .is_some_and(|seat| seat.compositor_owns_button(button))
                        {
                            repaint |= route_frame_pointer_button(
                                &mut frame_layers,
                                pressed,
                                MonotonicInstant::from_nanos(
                                    time_microseconds.saturating_mul(1_000),
                                ),
                            );
                        }
                        if button == 0x110 && !pressed {
                            let owns_release = wayland
                                .core()
                                .seats
                                .get(&1)
                                .is_some_and(|seat| seat.compositor_owns_button(button));
                            let over = if owns_release && !session_locked {
                                hit_test_decoration(
                                    &windows,
                                    &stacking_order,
                                    pointer_position,
                                    &config,
                                    &icon_layers,
                                )
                            } else {
                                None
                            };
                            if let Some((surface, hit)) = decoration_click.release(over)
                                && wayland.core().world.surface(surface).is_some()
                            {
                                match hit {
                                    DecorationHit::Close => {
                                        window_backend::close(
                                            surface,
                                            &windows,
                                            &mut wayland,
                                            #[cfg(all(
                                                feature = "shell-xwayland",
                                                target_env = "gnu"
                                            ))]
                                            compatibility.as_mut(),
                                        )?;
                                    }
                                    DecorationHit::Maximize => {
                                        let maximized = windows
                                            .get(&surface)
                                            .is_some_and(|window| !window.maximized);
                                        set_window_maximized(
                                            &mut windows,
                                            &mut configure_scheduler,
                                            surface,
                                            maximized,
                                            work_area,
                                            &config,
                                        )?;
                                        pointer_scene_dirty = true;
                                    }
                                    DecorationHit::Minimize => {
                                        if let Some(window) = windows.get_mut(&surface) {
                                            window.minimized = true;
                                            stacking_order
                                                .retain(|candidate| *candidate != surface);
                                            pointer_scene_dirty = true;
                                        }
                                    }
                                    DecorationHit::ShellAction(action) => {
                                        invoke_shell_action(
                                            &shell_actions,
                                            action,
                                            surface,
                                            &frame_layers,
                                        );
                                    }
                                    _ => {}
                                }
                                repaint = true;
                            }
                        }
                        if pressed
                            && button == 0x110
                            && !session_locked
                            && window_interaction.is_none()
                            && wayland
                                .core()
                                .seats
                                .get(&1)
                                .is_some_and(|seat| seat.pressed_buttons().is_empty())
                            && let Some((surface, hit)) = hit_test_decoration(
                                &windows,
                                &stacking_order,
                                pointer_position,
                                &config,
                                &icon_layers,
                            )
                            && wayland.core().world.surface(surface).is_some()
                        {
                            wayland
                                .core_mut()
                                .seats
                                .get_mut(&1)
                                .expect("desktop seat")
                                .pointer_button_target(button, WaylandButtonState::Pressed, true);
                            route_frame_pointer_button(
                                &mut frame_layers,
                                true,
                                MonotonicInstant::from_nanos(
                                    time_microseconds.saturating_mul(1_000),
                                ),
                            );
                            set_decoration_pointer_cursor(
                                &mut wayland,
                                &frame_layers,
                                &windows,
                                &stacking_order,
                                pointer_focus,
                                pointer_position,
                                &config,
                                &icon_layers,
                            );
                            window_backend::focus(
                                &display,
                                &mut wayland,
                                &windows,
                                &mut configure_scheduler,
                                &mut stacking_order,
                                Some(surface),
                                #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
                                compatibility.as_mut(),
                            )?;
                            match hit {
                                DecorationHit::Titlebar => {
                                    window_interaction = WindowInteraction::begin_move(
                                        &windows,
                                        surface,
                                        pointer_position,
                                    );
                                }
                                DecorationHit::Resize(edge) => {
                                    window_interaction = WindowInteraction::begin_resize(
                                        &mut windows,
                                        &mut configure_scheduler,
                                        surface,
                                        edge,
                                        pointer_position,
                                    );
                                }
                                DecorationHit::Close
                                | DecorationHit::Maximize
                                | DecorationHit::Minimize
                                | DecorationHit::ShellAction(_) => {
                                    decoration_click.press(surface, hit);
                                }
                                DecorationHit::Frame | DecorationHit::SystemMenu => {}
                            }
                            repaint = true;
                            continue;
                        }
                        let seat_pointer_focus = wayland
                            .core()
                            .seats
                            .get(&1)
                            .and_then(|seat| seat.pointer_focus)
                            .map(|focus| focus.surface);
                        if window_interaction.is_some() {
                            wayland
                                .core_mut()
                                .seats
                                .get_mut(&1)
                                .expect("desktop seat")
                                .pointer_button_target(
                                    button,
                                    if pressed {
                                        WaylandButtonState::Pressed
                                    } else {
                                        WaylandButtonState::Released
                                    },
                                    true,
                                );
                        } else {
                            let serial = display.next_serial();
                            wayland
                                .pointer_button(
                                    1,
                                    time,
                                    button,
                                    if pressed {
                                        WaylandButtonState::Pressed
                                    } else {
                                        WaylandButtonState::Released
                                    },
                                    serial,
                                )
                                .map_err(app_error)?;
                            if pressed && seat_pointer_focus.is_some() {
                                window_backend::focus(
                                    &display,
                                    &mut wayland,
                                    &windows,
                                    &mut configure_scheduler,
                                    &mut stacking_order,
                                    seat_pointer_focus,
                                    #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
                                    if session_locked {
                                        None
                                    } else {
                                        compatibility.as_mut()
                                    },
                                )?;
                                repaint = true;
                            }
                        }
                        if !pressed
                            && wayland.drag_active(1)
                            && wayland.drag_touch_slot(1).is_none()
                            && wayland
                                .core()
                                .seats
                                .get(&1)
                                .is_some_and(|seat| seat.pressed_buttons().is_empty())
                        {
                            wayland.drop_drag(1).map_err(app_error)?;
                            update_pointer_focus(
                                &display,
                                &mut wayland,
                                &windows,
                                &stacking_order,
                                session_locked,
                                &mut pointer_focus,
                                pointer_position,
                                &config,
                            )?;
                        }
                        if !pressed
                            && wayland
                                .core()
                                .seats
                                .get(&1)
                                .is_some_and(|seat| seat.pressed_buttons().is_empty())
                            && let Some(interaction) = window_interaction.take()
                        {
                            finish_window_interaction(
                                &mut windows,
                                &mut configure_scheduler,
                                interaction,
                            );
                            pointer_scene_dirty = true;
                            repaint = true;
                            set_decoration_pointer_cursor(
                                &mut wayland,
                                &frame_layers,
                                &windows,
                                &stacking_order,
                                pointer_focus,
                                pointer_position,
                                &config,
                                &icon_layers,
                            );
                        }
                        if !pressed && window_interaction.is_none() && !wayland.drag_active(1) {
                            update_pointer_focus(
                                &display,
                                &mut wayland,
                                &windows,
                                &stacking_order,
                                session_locked,
                                &mut pointer_focus,
                                pointer_position,
                                &config,
                            )?;
                            pointer_scene_dirty = true;
                        }
                    }
                    LinuxInputEventKind::KeyboardKey { keycode, pressed } => {
                        let direction = if pressed {
                            KeyDirection::Down
                        } else {
                            KeyDirection::Up
                        };
                        keyboard.update_key(keycode, direction);
                        let event = crate::application_host::ShellKeyEvent {
                            keycode,
                            keysym: keyboard.symbol(keycode),
                            control: keyboard.modifier_active(c"Control"),
                            shift: keyboard.modifier_active(c"Shift"),
                            alt: keyboard.modifier_active(c"Mod1"),
                            logo: keyboard.modifier_active(c"Mod4"),
                        };
                        let action = shortcut_keys.route(
                            event,
                            pressed,
                            session_locked,
                            if wayland.shortcuts_inhibited(1) {
                                None
                            } else {
                                keyboard_shortcut_handler.as_deref_mut()
                            },
                        );
                        if action == crate::application_host::ShellKeyAction::ReleaseCapture {
                            wayland.release_pointer_capture(1).map_err(app_error)?;
                            wayland.release_shortcut_inhibition(1).map_err(app_error)?;
                        }
                        if action == crate::application_host::ShellKeyAction::Quit {
                            crate::request_exit();
                        }
                        let widget_now = MonotonicInstant::from_nanos(
                            start.elapsed().as_nanos().min(u64::MAX as u128) as u64,
                        );
                        let widget_event = widget_keyboard_event(
                            event,
                            pressed,
                            &keyboard.utf8(keycode).map_err(app_error)?,
                        );
                        let consumed = action == crate::application_host::ShellKeyAction::Forward
                            && widget_key(&mut widgets, widget_event, widget_now, session_locked)?;
                        let held = widget_keys.contains(&keycode);
                        if consumed || held {
                            if pressed {
                                widget_keys.insert(keycode);
                            } else {
                                widget_keys.remove(&keycode);
                            }
                            repaint = true;
                        }
                        let serial = display.next_serial();
                        if action == crate::application_host::ShellKeyAction::Forward
                            && !consumed
                            && !held
                        {
                            wayland
                                .keyboard_key(
                                    1,
                                    time,
                                    keycode,
                                    if pressed {
                                        WaylandButtonState::Pressed
                                    } else {
                                        WaylandButtonState::Released
                                    },
                                    serial,
                                )
                                .map_err(app_error)?;
                        }
                        let modifiers = keyboard.modifiers();
                        wayland
                            .keyboard_modifiers(
                                1,
                                serial,
                                modifiers.depressed,
                                modifiers.latched,
                                modifiers.locked,
                                modifiers.group,
                            )
                            .map_err(app_error)?;
                    }
                    LinuxInputEventKind::PointerAxis {
                        horizontal,
                        vertical,
                        discrete_x,
                        discrete_y,
                    } => {
                        if pointer_focus.is_some() {
                            let _ = wayland.pointer_axis(
                                1, time, horizontal, vertical, discrete_x, discrete_y,
                            );
                        }
                    }
                    LinuxInputEventKind::TouchDown { slot, normalized } => {
                        if slot >= 0 {
                            let position = normalized_output_position(normalized, extent);
                            if let Some(surface) = hit_test_surface(
                                &windows,
                                &stacking_order,
                                position,
                                &config,
                                session_locked,
                            ) {
                                let local =
                                    surface_local_position(&windows, surface, position, &config);
                                wayland
                                    .touch_down(
                                        1,
                                        surface,
                                        time,
                                        slot,
                                        local,
                                        display.next_serial(),
                                    )
                                    .map_err(app_error)?;
                                touch_targets.insert(slot, surface);
                            }
                        }
                    }
                    LinuxInputEventKind::TouchMotion { slot, normalized } => {
                        let position = normalized_output_position(normalized, extent);
                        if wayland.drag_touch_slot(1) == Some(slot) {
                            let target = hit_test_surface(
                                &windows,
                                &stacking_order,
                                position,
                                &config,
                                session_locked,
                            );
                            let local = target.map_or(position, |surface| {
                                surface_local_position(&windows, surface, position, &config)
                            });
                            wayland
                                .drag_motion(1, target, time, local)
                                .map_err(app_error)?;
                            drag_position = position;
                            repaint = true;
                        } else if let Some(surface) = touch_targets.get(&slot).copied() {
                            let local =
                                surface_local_position(&windows, surface, position, &config);
                            wayland
                                .touch_motion(1, time, slot, local)
                                .map_err(app_error)?;
                        }
                    }
                    LinuxInputEventKind::TouchUp { slot } => {
                        if touch_targets.remove(&slot).is_some() {
                            wayland
                                .touch_up(1, time, slot, display.next_serial())
                                .map_err(app_error)?;
                        }
                        if wayland.drag_touch_slot(1) == Some(slot) {
                            wayland.drop_drag(1).map_err(app_error)?;
                            repaint = true;
                        }
                    }
                    LinuxInputEventKind::TouchCancel => {
                        touch_targets.clear();
                        wayland.touch_cancel(1).map_err(app_error)?;
                        if wayland.drag_touch_slot(1).is_some() {
                            wayland.cancel_drag(1).map_err(app_error)?;
                            repaint = true;
                        }
                    }
                    LinuxInputEventKind::DeviceAdded => {}
                    LinuxInputEventKind::DeviceRemoved => {
                        if let Some(interaction) = window_interaction.take() {
                            finish_window_interaction(
                                &mut windows,
                                &mut configure_scheduler,
                                interaction,
                            );
                        }
                    }
                }
            }
            #[cfg(feature = "profiler")]
            let mut cursor_path = PointerCursorPath::Unchanged;
            #[cfg(feature = "profiler")]
            {
                latest_pointer_event_this_turn = pointer_probe.newest_event_us();
            }
            if cursor_position_dirty {
                let cursor_hidden = wayland
                    .core()
                    .seats
                    .get(&1)
                    .is_some_and(|seat| matches!(seat.cursor, CursorImage::Hidden));
                if cursor_hidden {
                    if let Some(cursor) = &mut hardware_cursor {
                        cursor.hide();
                    }
                    #[cfg(feature = "profiler")]
                    {
                        cursor_path = PointerCursorPath::Hidden;
                    }
                } else if hardware_cursor
                    .as_ref()
                    .is_some_and(|cursor| !cursor.composited_fallback_requested())
                {
                    hardware_cursor
                        .as_mut()
                        .expect("atomic hardware cursor checked")
                        .move_to(output_scale.physical_point(pointer_position));
                    #[cfg(feature = "profiler")]
                    {
                        cursor_path = PointerCursorPath::Deferred;
                        pending_deferred_cursor_event_us = latest_pointer_event_this_turn;
                    }
                } else {
                    pointer_primary_dirty = true;
                    #[cfg(feature = "profiler")]
                    {
                        cursor_path = PointerCursorPath::CompositedDamage;
                    }
                }
            }
            // Deliver input to Xwayland/native clients before buffer import, composition and GPU
            // submission. Flushing only at the next event-loop dispatch adds that entire work to
            // application input latency even when the hardware cursor already moved.
            display.flush_clients();
            if let Some(batch) = input_stats {
                let flushed_us = crate::platform_linux::monotonic_time_microseconds();
                latency_trace.event("input_flush", batch.trace_values(flushed_us));
                if let Some(stats) = &mut frame_stats {
                    stats.input_flushed(batch, flushed_us);
                }
            }
            repaint |= pointer_primary_dirty;
            #[cfg(feature = "profiler")]
            {
                if pointer_primary_dirty {
                    pending_primary_pointer_event_us = latest_pointer_event_this_turn;
                }
                pointer_probe.finish(cursor_path);
                if pointer_probe.has_motion()
                    && input_callback_time_us != 0
                    && let Some(dispatch_started_us) = input_dispatch_started_us
                {
                    crate::profiler::record_instant_value(
                        "input.libinput.pointer_motion.pipeline.fd_callback_to_dispatch_ns",
                        dispatch_started_us
                            .saturating_sub(input_callback_time_us)
                            .saturating_mul(1_000),
                    );
                }
                if pointer_probe.has_motion()
                    && let Some(now_us) = crate::platform_linux::monotonic_time_microseconds()
                    && let Some(previous_us) = previous_pointer_batch_us.replace(now_us)
                {
                    crate::profiler::record_instant_value(
                        "input.libinput.pointer_motion.pipeline.batch_interval_ns",
                        now_us.saturating_sub(previous_us).saturating_mul(1_000),
                    );
                }
            }
        }

        window_backend::flush(
            &display,
            &mut wayland,
            &mut windows,
            &mut configure_scheduler,
            &mut resize_configure_budget,
            &config,
            work_area,
            #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
            compatibility.as_mut(),
        )?;

        latency_trace.phase("shm_completions_schedule");
        if runtime_ready.swap(false, Ordering::AcqRel) {
            runtime_wake.drain();
        }
        for completion in shm_copy_worker.drain() {
            other_work_seen = true;
            let completed_surface = completion.snapshot.surface;
            if !submitted_shm_surfaces.remove(&completed_surface) {
                return Err(AppError::new(
                    "completed SHM surface copy was not submitted",
                ));
            }
            repaint |= finish_shm_copy(
                &display,
                &mut wayland,
                &mut windows,
                &mut window_identities,
                &mut configure_scheduler,
                &mut stacking_order,
                &mut next_window_offset,
                work_area,
                session_locked,
                &mut pointer_scene_dirty,
                &mut pending_shm_buffers,
                &mut pending_shm_surfaces,
                completion,
            )?;
        }
        // Do not spend pixel-copy/conversion work on a surface hidden by an active resize veil.
        // One replaceable request retains its buffer lease; unrelated surfaces can still run.
        let mut blocked_shm_surfaces = submitted_shm_surfaces.clone();
        blocked_shm_surfaces.extend(windows.keys().copied().filter(|surface| {
            resize_veil_owner(&windows, *surface)
                .and_then(|owner| windows.get(&owner))
                .is_some_and(ClientWindow::resizing)
        }));
        while let Some(surface) =
            take_ready_deferred_shm_surface(&mut deferred_shm_order, &blocked_shm_surfaces)
        {
            let request = deferred_shm_copies
                .remove(&surface)
                .ok_or_else(|| AppError::new("deferred SHM surface has no request"))?;
            if let Some(request) = shm_copy_worker.try_submit(request)? {
                deferred_shm_copies.insert(surface, request);
                deferred_shm_order.push_front(surface);
                break;
            }
            if !submitted_shm_surfaces.insert(surface) {
                return Err(AppError::new(
                    "deferred SHM surface copy was already submitted",
                ));
            }
            blocked_shm_surfaces.insert(surface);
        }
        let runtime_now = MonotonicInstant::from_nanos(
            start.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64,
        );
        let (runtime_turn_ready, _, _) = desktop_runtime_schedule(
            &frame_layers,
            pointer.as_ref(),
            &icon_layers,
            &widgets,
            runtime_now,
        );
        repaint |= runtime_turn_ready;
        other_work_seen |= runtime_turn_ready;

        latency_trace.phase("kms_completions");
        if kms_ready.swap(false, Ordering::AcqRel) {
            kms.dispatch_events().map_err(app_error)?;
            for _ in 0..kms.take_completed_page_flips() {
                let completed = pending_kms_commit.take().ok_or_else(|| {
                    AppError::new("DRM completed a page flip with no tracked atomic commit")
                })?;
                if let Some(completed_slot) = completed.primary_slot {
                    frame_slots[completed_slot]
                        .page_flip_completed()
                        .map_err(app_error)?;
                    if let Some(previous) = current_scanout.replace(completed_slot) {
                        frame_slots[previous]
                            .page_flip_replaced()
                            .map_err(app_error)?;
                    }
                    if let Some(stats) = &mut frame_stats {
                        stats.presented(&frame_surface_revisions[completed_slot]);
                    }
                    latency_trace.event("primary_flip", [completed_slot as u64, 0, 0, 0]);
                    presentation_completed = true;
                    presented_surface_revisions
                        .extend(std::mem::take(&mut frame_surface_revisions[completed_slot]));
                    #[cfg(feature = "profiler")]
                    if let Some(event_time_us) = frame_pointer_event_us[completed_slot].take() {
                        record_pointer_event_latency(
                            "input.libinput.pointer_motion.pipeline.event_to_primary_scanout_ns",
                            event_time_us,
                        );
                    }
                }
                if let Some(cursor_snapshot) = completed.cursor {
                    hardware_cursor
                        .as_mut()
                        .ok_or_else(|| {
                            AppError::new(
                                "DRM completed an atomic cursor commit after cursor retirement",
                            )
                        })?
                        .mark_completed(cursor_snapshot)?;
                    #[cfg(feature = "profiler")]
                    if let Some(event_time_us) = completed.cursor_event_us {
                        record_pointer_event_latency(
                            "input.libinput.pointer_motion.pipeline.event_to_cursor_scanout_ns",
                            event_time_us,
                        );
                    }
                }
                #[cfg(feature = "profiler")]
                {
                    crate::profiler::record_instant("presentation.kms.page_flip_completed");
                }
                if hardware_cursor
                    .as_ref()
                    .is_some_and(HardwareCursor::ready_to_retire)
                {
                    hardware_cursor = None;
                    #[cfg(feature = "profiler")]
                    crate::profiler::record_instant("presentation.cursor.composited_fallback");
                }
            }
        }

        latency_trace.phase("gpu_completions_releases");
        if vulkan_completion_ready
            .as_ref()
            .is_some_and(|ready| ready.swap(false, Ordering::AcqRel))
        {
            // Allocation and GPU completion share a wake FD; ordinary GPU completion alone
            // must not trigger another otherwise unchanged desktop preparation.
            repaint |= desktop_renderer.poll_allocations(&mut latency_trace)?;
            for completion in desktop_renderer.drain_completions() {
                completion.result.map_err(AppError::new)?;
                for retirement in completion.dma_bufs {
                    retire_submitted_dma_buf(&mut wayland, &mut pending_dma_bufs, retirement)?;
                }
                frame_slots[completion.slot_index]
                    .gpu_completed()
                    .map_err(app_error)?;
                ready_scanout.push_back(completion.slot_index);
                latency_trace.event("gpu_ready", [completion.slot_index as u64, 0, 0, 0]);
                #[cfg(feature = "profiler")]
                crate::profiler::record_instant("vulkan.scanout.completion_ready");
            }
        }

        repaint |= desktop_renderer.take_acquire_wakeup();
        latency_trace.phase("window_requests");
        #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
        if let Some(host) = &mut compatibility {
            for (raw, action) in host.take_maximize_requests() {
                let Some(surface) = u32::try_from(raw).ok().and_then(WaylandSurfaceId::from_raw)
                else {
                    continue;
                };
                let Some(window) = windows.get(&surface) else {
                    continue;
                };
                if session_locked || window_interaction.is_some() {
                    continue;
                }
                let maximized = match action {
                    0 => false,
                    1 => true,
                    2 => !window.maximized,
                    _ => continue,
                };
                if maximized == window.maximized {
                    continue;
                }
                set_window_maximized(
                    &mut windows,
                    &mut configure_scheduler,
                    surface,
                    maximized,
                    work_area,
                    &config,
                )?;
                pointer_scene_dirty = true;
                repaint = true;
            }
            for (raw, direction, button) in host.take_move_resize_requests() {
                let Some(surface) = u32::try_from(raw).ok().and_then(WaylandSurfaceId::from_raw)
                else {
                    continue;
                };
                if session_locked {
                    continue;
                }
                if direction == 11 {
                    if matches!(window_interaction, Some(WindowInteraction::Move { surface: target, .. } | WindowInteraction::Resize { surface: target, .. }) if target == surface)
                    {
                        if let Some(interaction) = window_interaction.take() {
                            finish_window_interaction(
                                &mut windows,
                                &mut configure_scheduler,
                                interaction,
                            );
                            pointer_scene_dirty = true;
                            repaint = true;
                        }
                    }
                    continue;
                }
                if window_interaction.is_some() {
                    continue;
                }
                let Some(seat) = wayland.core().seats.get(&1) else {
                    continue;
                };
                if !x11_windows::authorize_move_resize(seat, surface, button) {
                    continue;
                }
                window_interaction = match direction {
                    8 => WindowInteraction::begin_move(&windows, surface, pointer_position),
                    0..=7
                        if windows
                            .get(&surface)
                            .is_some_and(|w| !w.maximized && !w.fullscreen && !w.minimized) =>
                    {
                        WindowInteraction::begin_resize(
                            &mut windows,
                            &mut configure_scheduler,
                            surface,
                            [
                                ResizeEdge::TopLeft,
                                ResizeEdge::Top,
                                ResizeEdge::TopRight,
                                ResizeEdge::Right,
                                ResizeEdge::BottomRight,
                                ResizeEdge::Bottom,
                                ResizeEdge::BottomLeft,
                                ResizeEdge::Left,
                            ][direction as usize],
                            pointer_position,
                        )
                    }
                    _ => None,
                };
                if window_interaction.is_some() {
                    wayland
                        .set_pointer_focus(1, None, pointer_position, display.next_serial())
                        .map_err(app_error)?;
                    pointer_focus = None;
                    pointer_scene_dirty = true;
                    repaint = true;
                }
            }
        }
        latency_trace.phase("surface_actions_import");
        let superseded = wayland.core_mut().take_superseded_publications();
        let mut retired_buffers = BTreeSet::new();
        for (surface, revision, buffer) in superseded {
            drop(wayland.take_acquire_fence(surface, revision));
            wayland
                .finish_explicit_release(surface, revision, None)
                .map_err(app_error)?;
            retired_buffers.insert(buffer);
        }
        if !retired_buffers.is_empty() {
            let pending_publications = wayland.core().pending_publication_buffers();
            for buffer in retired_buffers {
                if !pending_publications.contains(&buffer)
                    && !pending_shm_buffers.contains_key(&buffer)
                    && !pending_dma_bufs.contains_key(&buffer)
                {
                    wayland.release_buffer(buffer).map_err(app_error)?;
                }
            }
        }
        let actions = wayland.core_mut().drain_actions().collect::<Vec<_>>();
        if !actions.is_empty() {
            other_work_seen = true;
        }
        for action in actions {
            match action {
                CompositorAction::PublishSurface(surface) => {
                    let snapshot = wayland
                        .core()
                        .world
                        .surface(surface)
                        .map(|surface| surface.snapshot().clone());
                    let Some(snapshot) = snapshot else { continue };
                    let Some(role) = snapshot.role else {
                        continue;
                    };
                    #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
                    if role == SurfaceRole::Xwayland {
                        if let (Some(host), Some(serial)) =
                            (&mut compatibility, snapshot.xwayland_serial)
                        {
                            host.committed(surface, serial);
                        }
                    }
                    // Configure acknowledgement is commit state, not image-worker state. Observe
                    // it before a newer latest-wins SHM publication can replace these pixels.
                    observe_surface_configure_acknowledgement(&mut windows, &snapshot);
                    if !matches!(
                        role,
                        SurfaceRole::XdgToplevel
                            | SurfaceRole::XdgPopup
                            | SurfaceRole::Subsurface
                            | SurfaceRole::Cursor
                            | SurfaceRole::DragIcon
                            | SurfaceRole::SessionLock
                            | SurfaceRole::Xwayland
                    ) {
                        continue;
                    }
                    let Some(attachment) = snapshot.attachment else {
                        continue;
                    };
                    let viewport = wayland.viewport(surface);
                    let descriptor = wayland
                        .core()
                        .buffer(attachment.buffer)
                        .cloned()
                        .ok_or_else(|| AppError::new("surface references an unknown buffer"))?;
                    let (kind, size) = match &descriptor {
                        BufferDescriptor::Shm(buffer) => ("shm", buffer.size),
                        BufferDescriptor::DmaBuf(buffer) => ("dmabuf", buffer.size),
                    };
                    latency_trace.event(
                        "surface_commit",
                        [
                            surface.get() as u64,
                            snapshot.revision,
                            u64::from(matches!(descriptor, BufferDescriptor::DmaBuf(_))),
                            match role {
                                SurfaceRole::Xwayland => 1,
                                SurfaceRole::XdgToplevel | SurfaceRole::XdgPopup => 2,
                                _ => 3,
                            },
                        ],
                    );
                    if role == SurfaceRole::Xwayland
                        && let Some(stats) = &mut frame_stats
                    {
                        stats.x11_commit(matches!(descriptor, BufferDescriptor::DmaBuf(_)));
                    }
                    resize_trace::event(
                        surface.get(),
                        "arrival",
                        format_args!(
                            "revision={} buffer={:?} kind={} size={:?}",
                            snapshot.revision, attachment.buffer, kind, size
                        ),
                    );
                    if matches!(descriptor, BufferDescriptor::DmaBuf(_)) {
                        if let Some(request) = deferred_shm_copies.remove(&surface) {
                            deferred_shm_order.retain(|candidate| *candidate != surface);
                            discard_shm_copy(
                                &mut wayland,
                                &mut pending_shm_buffers,
                                &mut pending_shm_surfaces,
                                request,
                            )?;
                        }
                        let image = wayland.read_dma_buf(attachment.buffer).map_err(app_error)?;
                        // linux-dmabuf defaults to implicit synchronization. Preserve an explicit
                        // protocol fence when present; otherwise snapshot the buffer's producer
                        // fences so Vulkan cannot sample a partially rendered client frame.
                        let acquire = match wayland.take_acquire_fence(surface, snapshot.revision) {
                            Some(explicit) => explicit,
                            None => image.export_implicit_read_sync_file().map_err(app_error)?,
                        };
                        let queued = desktop_renderer.queue_dma_buf(
                            DmaBufPublication {
                                surface,
                                revision: snapshot.revision,
                                buffer: attachment.buffer,
                                image,
                                acquire: Some(acquire),
                                buffer_scale: snapshot.buffer_scale,
                                buffer_transform: snapshot.buffer_transform,
                                viewport,
                                output_scale,
                                coordinate_density: wayland.x11_surface_scale(surface),
                                surface_damage: snapshot.damage.clone(),
                                buffer_damage: snapshot.buffer_damage.clone(),
                            },
                            &display,
                        )?;
                        let pending = pending_dma_bufs.entry(attachment.buffer).or_default();
                        *pending = pending
                            .checked_add(1)
                            .ok_or_else(|| AppError::new("pending DMA-BUF use count overflow"))?;
                        if let Some(replaced) = queued.replaced {
                            retire_unsubmitted_dma_buf(
                                &mut wayland,
                                &mut pending_dma_bufs,
                                replaced,
                            )?;
                        }
                        apply_surface_publication(
                            &display,
                            &mut wayland,
                            &mut windows,
                            &mut window_identities,
                            &mut configure_scheduler,
                            &mut stacking_order,
                            &mut next_window_offset,
                            work_area,
                            session_locked,
                            &mut pointer_scene_dirty,
                            &snapshot,
                            PreparedClientImage::External {
                                extent: queued.extent,
                                raster_extent: queued.raster_extent,
                                pixel_format: queued.pixel_format,
                                alpha_mode: queued.alpha_mode,
                                image: queued.image,
                                damage: queued.damage,
                            },
                        )?;
                        repaint = true;
                        continue;
                    }
                    let BufferDescriptor::Shm(descriptor) = descriptor else {
                        unreachable!("buffer descriptor variant checked above")
                    };
                    if let Some(retirement) = desktop_renderer.cancel_dma_buf_surface(surface) {
                        retire_unsubmitted_dma_buf(
                            &mut wayland,
                            &mut pending_dma_bufs,
                            retirement,
                        )?;
                    }
                    // This newer SHM publication supersedes any older full-copy request still
                    // waiting in this surface's mailbox. If this one also needs a worker copy it
                    // is inserted below as the new latest value.
                    let replaced_deferred_copy =
                        if let Some(request) = deferred_shm_copies.remove(&surface) {
                            deferred_shm_order.retain(|candidate| *candidate != surface);
                            discard_replaced_shm_copy(
                                &mut wayland,
                                &mut pending_shm_buffers,
                                &mut pending_shm_surfaces,
                                request,
                                attachment.buffer,
                            )?;
                            true
                        } else {
                            false
                        };
                    let direct_shm = snapshot.buffer_scale == 1
                        && snapshot.buffer_transform == BufferTransform::Normal
                        && viewport.is_none();
                    let (native_pixel_format, native_alpha_mode) =
                        shm_image_metadata(descriptor.format).map_err(app_error)?;
                    let buffer_damage = if direct_shm {
                        union_surface_damage(
                            &snapshot
                                .damage
                                .iter()
                                .chain(&snapshot.buffer_damage)
                                .copied()
                                .collect::<Vec<_>>(),
                            descriptor.size,
                        )
                    } else {
                        None
                    };
                    let metadata_matches = windows.get(&surface).is_some_and(|window| {
                        window.presentation.size == descriptor.size
                            && window.presentation.pixel_format == native_pixel_format
                            && window.presentation.alpha_mode == native_alpha_mode
                            && window.presentation.pixels.len()
                                == descriptor.size.width as usize
                                    * descriptor.size.height as usize
                                    * 4
                    });
                    let surface_copy_pending = pending_shm_surfaces.contains_key(&surface);
                    let resize_copy_paused = resize_veil_owner(&windows, surface)
                        .and_then(|owner| windows.get(&owner))
                        .is_some_and(ClientWindow::resizing);
                    let contiguous = windows.get(&surface).is_some_and(|window| {
                        window.presentation.can_apply_damage(snapshot.revision)
                    });
                    let can_patch = direct_shm
                        && contiguous
                        && metadata_matches
                        && !surface_copy_pending
                        && !replaced_deferred_copy
                        && !resize_copy_paused;
                    let full_damage = buffer_damage == Some(full_rect(descriptor.size));
                    let prepared_image = if can_patch && !full_damage {
                        match buffer_damage {
                            Some(rect) => PreparedClientImage::Region(
                                shm_image_update(attachment.buffer, snapshot.revision, {
                                    #[cfg(feature = "profiler")]
                                    let _span =
                                        crate::profiler::span!("compositor.shm.copy.region");
                                    let region = wayland
                                        .read_shm_buffer_region(attachment.buffer, rect)
                                        .map_err(app_error)?;
                                    #[cfg(feature = "profiler")]
                                    crate::profiler::record_instant_value(
                                        "compositor.shm.copy_bytes",
                                        region.pixels.len() as u64,
                                    );
                                    region
                                })
                                .map_err(app_error)?,
                            ),
                            None => PreparedClientImage::Unchanged {
                                extent: descriptor.size,
                                pixel_format: native_pixel_format,
                                alpha_mode: native_alpha_mode,
                            },
                        }
                    } else {
                        let request = ShmCopyRequest::new(
                            snapshot.clone(),
                            viewport,
                            wayland
                                .shm_buffer_reader(attachment.buffer)
                                .map_err(app_error)?,
                            output_scale,
                            wayland.x11_surface_scale(surface),
                        );
                        let buffer = request.buffer();
                        let request_surface = request.snapshot.surface;
                        let pending = pending_shm_buffers.entry(buffer).or_default();
                        *pending = pending.checked_add(1).ok_or_else(|| {
                            AppError::new("pending SHM buffer use count overflow")
                        })?;
                        let pending = pending_shm_surfaces.entry(surface).or_default();
                        *pending = pending.checked_add(1).ok_or_else(|| {
                            AppError::new("pending SHM surface use count overflow")
                        })?;
                        if resize_copy_paused || submitted_shm_surfaces.contains(&request_surface) {
                            #[cfg(feature = "profiler")]
                            if resize_copy_paused {
                                crate::profiler::record_instant(
                                    "compositor.resize.shm_copy_deferred",
                                );
                            }
                            let replaced = deferred_shm_copies.insert(request_surface, request);
                            if replaced.is_none() {
                                if deferred_shm_copies.len() > MAX_DEFERRED_SHM_COPIES {
                                    return Err(AppError::new(
                                        "deferred SHM copy mailbox exceeded its hard bound",
                                    ));
                                }
                                deferred_shm_order.push_back(request_surface);
                            } else if let Some(replaced) = replaced {
                                discard_shm_copy(
                                    &mut wayland,
                                    &mut pending_shm_buffers,
                                    &mut pending_shm_surfaces,
                                    replaced,
                                )?;
                            }
                        } else if let Some(request) = shm_copy_worker.try_submit(request)? {
                            if deferred_shm_copies.len() >= MAX_DEFERRED_SHM_COPIES {
                                return Err(AppError::new(
                                    "deferred SHM copy mailbox exceeded its hard bound",
                                ));
                            }
                            deferred_shm_copies.insert(request_surface, request);
                            deferred_shm_order.push_back(request_surface);
                        } else {
                            submitted_shm_surfaces.insert(request_surface);
                        }
                        continue;
                    };
                    apply_surface_publication(
                        &display,
                        &mut wayland,
                        &mut windows,
                        &mut window_identities,
                        &mut configure_scheduler,
                        &mut stacking_order,
                        &mut next_window_offset,
                        work_area,
                        session_locked,
                        &mut pointer_scene_dirty,
                        &snapshot,
                        prepared_image,
                    )?;
                    wayland
                        .finish_explicit_release(surface, snapshot.revision, None)
                        .map_err(app_error)?;
                    wayland
                        .release_buffer(attachment.buffer)
                        .map_err(app_error)?;
                    repaint = true;
                }
                CompositorAction::WithdrawSurface(surface) => {
                    #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
                    if let Some(host) = &mut compatibility {
                        if let Some(state) = wayland.core().world.surface(surface) {
                            let snapshot = state.snapshot();
                            if snapshot.role == Some(SurfaceRole::Xwayland) {
                                if let Some(serial) = snapshot.xwayland_serial {
                                    host.committed(surface, serial);
                                }
                            }
                        } else {
                            host.destroyed(surface);
                        }
                    }

                    if let Some(retirement) = desktop_renderer.cancel_dma_buf_surface(surface) {
                        retire_unsubmitted_dma_buf(
                            &mut wayland,
                            &mut pending_dma_bufs,
                            retirement,
                        )?;
                    }
                    if let Some(request) = deferred_shm_copies.remove(&surface) {
                        deferred_shm_order.retain(|candidate| *candidate != surface);
                        discard_shm_copy(
                            &mut wayland,
                            &mut pending_shm_buffers,
                            &mut pending_shm_surfaces,
                            request,
                        )?;
                    }
                    if wayland
                        .core()
                        .seats
                        .get(&1)
                        .and_then(|seat| seat.keyboard_focus)
                        .is_some_and(|focus| focus.surface == surface)
                    {
                        wayland
                            .set_keyboard_focus(1, None, display.next_serial())
                            .map_err(app_error)?;
                    }
                    windows.remove(&surface);
                    if wayland.core().world.surface(surface).is_none() {
                        window_identities.destroy(surface);
                    }
                    configure_scheduler.cancel(surface);
                    decoration_click.cancel_surface(surface);
                    frame_layers.remove(&surface);
                    stacking_order.retain(|candidate| *candidate != surface);
                    if matches!(
                        window_interaction,
                        Some(WindowInteraction::Move {
                            surface: candidate,
                            ..
                        }
                            | WindowInteraction::Resize {
                                surface: candidate,
                                ..
                            }) if candidate == surface
                    ) {
                        window_interaction = None;
                    }
                    if touch_targets.values().any(|target| *target == surface) {
                        touch_targets.clear();
                        wayland.touch_cancel(1).map_err(app_error)?;
                    }
                    pointer_scene_dirty = true;
                    repaint = true;
                }
                CompositorAction::ActivateSurface {
                    surface,
                    application_id: _,
                    source_surface: _,
                } => {
                    if windows.contains_key(&surface) {
                        if let Some(window) = windows.get_mut(&surface) {
                            window.minimized = false;
                        }
                        focus_toplevel(
                            &display,
                            &mut wayland,
                            &windows,
                            &mut configure_scheduler,
                            &mut stacking_order,
                            Some(surface),
                        )?;
                        pointer_scene_dirty = true;
                        repaint = true;
                    }
                }
                CompositorAction::MoveToplevel(surface) => {
                    if windows
                        .get(&surface)
                        .is_some_and(|window| !window.fullscreen && !window.minimized)
                        && wayland
                            .core()
                            .seats
                            .get(&1)
                            .and_then(|seat| seat.pointer_grab_focus())
                            .is_some_and(|focus| focus.surface == surface)
                    {
                        window_interaction =
                            WindowInteraction::begin_move(&windows, surface, pointer_position);
                        if window_interaction.is_some() {
                            wayland
                                .set_pointer_focus(1, None, pointer_position, display.next_serial())
                                .map_err(app_error)?;
                            pointer_focus = None;
                        }
                    }
                }
                CompositorAction::ResizeToplevel { surface, edge } => {
                    if windows.get(&surface).is_some_and(|window| {
                        !window.maximized && !window.fullscreen && !window.minimized
                    }) && wayland
                        .core()
                        .seats
                        .get(&1)
                        .and_then(|seat| seat.pointer_grab_focus())
                        .is_some_and(|focus| focus.surface == surface)
                    {
                        window_interaction = WindowInteraction::begin_resize(
                            &mut windows,
                            &mut configure_scheduler,
                            surface,
                            edge,
                            pointer_position,
                        );
                        if window_interaction.is_some() {
                            wayland
                                .set_pointer_focus(1, None, pointer_position, display.next_serial())
                                .map_err(app_error)?;
                            pointer_focus = None;
                        }
                        repaint = true;
                    }
                }
                CompositorAction::MaximizeToplevel { surface, maximized } => {
                    set_window_maximized(
                        &mut windows,
                        &mut configure_scheduler,
                        surface,
                        maximized,
                        work_area,
                        &config,
                    )?;
                    pointer_scene_dirty = true;
                    repaint = true;
                }
                CompositorAction::FullscreenToplevel {
                    surface,
                    fullscreen,
                    output: _,
                } => {
                    set_window_fullscreen(
                        &mut windows,
                        &mut configure_scheduler,
                        surface,
                        fullscreen,
                        extent,
                        &config,
                    )?;
                    pointer_scene_dirty = true;
                    repaint = true;
                }
                CompositorAction::MinimizeToplevel(surface) => {
                    if let Some(window) = windows.get_mut(&surface) {
                        window.minimized = true;
                        stacking_order.retain(|candidate| *candidate != surface);
                        pointer_scene_dirty = true;
                        repaint = true;
                    }
                }
                CompositorAction::SessionLockRequested(lock) => {
                    shortcut_keys.suppress_held();
                    wayland.cancel_keyboard_input(1).map_err(app_error)?;
                    if wayland.drag_active(1) {
                        wayland.cancel_drag(1).map_err(app_error)?;
                    }
                    session_locked = true;
                    decoration_click.cancel();
                    for frame in frame_layers.values_mut() {
                        frame
                            .layer
                            .runtime
                            .cancel_pointer(crate::input::PointerId::PRIMARY);
                    }
                    wayland
                        .core_mut()
                        .seats
                        .get_mut(&1)
                        .expect("desktop seat")
                        .cancel_pointer_buttons();
                    pending_session_lock = Some(lock);
                    if let Some(interaction) = window_interaction.take() {
                        finish_window_interaction(
                            &mut windows,
                            &mut configure_scheduler,
                            interaction,
                        );
                    }
                    pointer_focus = None;
                    wayland
                        .set_pointer_focus(1, None, pointer_position, display.next_serial())
                        .map_err(app_error)?;
                    if !touch_targets.is_empty() {
                        touch_targets.clear();
                        wayland.touch_cancel(1).map_err(app_error)?;
                    }
                    pointer_scene_dirty = true;
                    repaint = true;
                }
                CompositorAction::SessionLockCancelled(lock) => {
                    if pending_session_lock == Some(lock) && !wayland.session_locked() {
                        shortcut_keys.suppress_held();
                        wayland.cancel_keyboard_input(1).map_err(app_error)?;
                        pending_session_lock = None;
                        session_locked = false;
                        pointer_scene_dirty = true;
                        repaint = true;
                    }
                }
                CompositorAction::SessionUnlockRequested(lock) => {
                    shortcut_keys.suppress_held();
                    wayland.cancel_keyboard_input(1).map_err(app_error)?;
                    if pending_session_lock == Some(lock) {
                        pending_session_lock = None;
                    }
                    session_locked = false;
                    wayland
                        .core_mut()
                        .seats
                        .get_mut(&1)
                        .expect("desktop seat")
                        .cancel_pointer_buttons();
                    pointer_scene_dirty = true;
                    repaint = true;
                }
                CompositorAction::StartDrag {
                    seat: _,
                    origin: _,
                    icon: _,
                } => repaint = true,
                CompositorAction::FinishDrag { icon: _ } => {
                    pointer_scene_dirty = true;
                    repaint = true;
                }
                CompositorAction::RepaintOutput(_) => repaint = true,
                CompositorAction::ImportBuffer(_)
                | CompositorAction::ReleaseBuffer(_)
                | CompositorAction::DisconnectClient(_) => {}
            }
        }
        #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
        if let Some(host) = &mut compatibility {
            let changed = host.sync_presentation(&mut windows, &mut window_identities, &config)?;
            repaint |= changed;
            pointer_scene_dirty |= changed;
        }

        latency_trace.phase("window_policy_cursor");
        // Resume frame-callback-paced clients after release even when the veil itself no longer
        // changes (so there may be no new KMS frame). Hidden intermediate images are not presented;
        // do not misreport presentation feedback while asking the client for its final redraw.
        let callback_time = u32::try_from(start.elapsed().as_millis()).unwrap_or(u32::MAX);
        for (surface, window) in &windows {
            if window.presentation.revision != 0
                && !window.minimized
                && resize_veil_owner(&windows, *surface)
                    .and_then(|owner| windows.get(&owner))
                    .is_some_and(ClientWindow::waiting_for_resize_content)
            {
                wayland
                    .surface_occluded_frame_ready(
                        *surface,
                        window.presentation.revision,
                        callback_time,
                    )
                    .map_err(app_error)?;
            }
        }

        if pointer_scene_dirty
            && window_interaction.is_none()
            && (!wayland.drag_active(1) || wayland.drag_touch_slot(1).is_some())
        {
            repaint |= reconcile_pointer_state(
                &display,
                &mut wayland,
                &mut frame_layers,
                &windows,
                &stacking_order,
                session_locked,
                &mut pointer_focus,
                pointer_position,
                &config,
                &icon_layers,
                runtime_now,
            )?;
        }

        window_backend::flush(
            &display,
            &mut wayland,
            &mut windows,
            &mut configure_scheduler,
            &mut resize_configure_budget,
            &config,
            work_area,
            #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
            compatibility.as_mut(),
        )?;

        let cursor_image = wayland
            .core()
            .seats
            .get(&1)
            .expect("seat registered")
            .cursor;
        repaint |=
            cursor_transition_requires_presentation(cursor_image_at_turn_start, cursor_image);
        #[cfg(not(feature = "profiler"))]
        let _ = (pointer_motion_seen, other_work_seen);
        #[cfg(feature = "profiler")]
        let pointer_motion_only = repaint && pointer_motion_seen && !other_work_seen;

        latency_trace.phase("kms_submit");
        // One atomic commit per CRTC may be outstanding. Primary frames retain mailbox behavior,
        // while cursor-only motion reuses the current primary plane and commits only cursor state.
        let cursor_commit_ready = !first_modeset
            && hardware_cursor
                .as_ref()
                .is_some_and(HardwareCursor::needs_commit);
        if pending_kms_commit.is_none() && (!ready_scanout.is_empty() || cursor_commit_ready) {
            while ready_scanout.len() > 1 {
                let stale = ready_scanout.pop_front().expect("length checked");
                frame_slots[stale].discard_ready().map_err(app_error)?;
                latency_trace.event("frame_discard", [stale as u64, 0, 0, 0]);
                #[cfg(feature = "profiler")]
                {
                    crate::profiler::record_instant("presentation.frame.mailbox_replaced");
                    frame_pointer_event_us[stale] = None;
                }
            }
            let primary_slot = ready_scanout.pop_back();
            let mut request = if let Some(slot_index) = primary_slot {
                kms.primary_modeset_request(
                    connector.id,
                    connector_properties,
                    crtc,
                    crtc_properties,
                    plane.id,
                    plane_properties,
                    mode_blob.id(),
                    frame_slots[slot_index].framebuffer,
                    physical_extent.width as u32,
                    physical_extent.height as u32,
                )
                .map_err(app_error)?
            } else {
                let mut request = kms.atomic_request().map_err(app_error)?;
                // A plane-disable sets its CRTC_ID to zero. Retaining ACTIVE in cursor-only
                // requests keeps the affected CRTC explicit so PAGE_FLIP_EVENT has one owner.
                request
                    .include_active_crtc(crtc, crtc_properties)
                    .map_err(app_error)?;
                request
            };
            let cursor_snapshot = hardware_cursor
                .as_ref()
                .map(|cursor| cursor.append_desired(&mut request, crtc))
                .transpose()?
                .flatten();
            let cursor_fallback_submission = cursor_snapshot.is_some()
                && hardware_cursor
                    .as_ref()
                    .is_some_and(HardwareCursor::composited_fallback_requested);
            #[cfg(feature = "profiler")]
            let _kms_commit = crate::profiler::span!("presentation.kms.atomic_commit");
            let commit_result = if first_modeset {
                match request.test(true) {
                    Ok(()) => request.commit(true, false),
                    Err(error) => Err(error),
                }
            } else {
                request.commit(false, true)
            };
            if commit_result.is_ok()
                && let Some(slot) = primary_slot
            {
                latency_trace.event(
                    "kms_submitted",
                    [slot as u64, u64::from(first_modeset), 0, 0],
                );
            }
            match commit_result {
                Ok(()) if first_modeset => {
                    let slot_index = primary_slot
                        .expect("the initial modeset is scheduled only with a primary frame");
                    frame_slots[slot_index]
                        .page_flip_submitted()
                        .and_then(|_| frame_slots[slot_index].page_flip_completed())
                        .map_err(app_error)?;
                    current_scanout = Some(slot_index);
                    if let Some(snapshot) = cursor_snapshot {
                        let cursor = hardware_cursor
                            .as_mut()
                            .expect("submitted atomic cursor remains owned");
                        cursor.mark_submitted(snapshot)?;
                        cursor.mark_completed(snapshot)?;
                        #[cfg(feature = "profiler")]
                        if let Some(event_time_us) = pending_deferred_cursor_event_us.take() {
                            record_pointer_event_latency(
                                "input.libinput.pointer_motion.pipeline.freshest_event_to_cursor_submit_ns",
                                event_time_us,
                            );
                            record_pointer_event_latency(
                                "input.libinput.pointer_motion.pipeline.event_to_cursor_scanout_ns",
                                event_time_us,
                            );
                        }
                    }
                    first_modeset = false;
                    presentation_completed = true;
                    presented_surface_revisions
                        .extend(std::mem::take(&mut frame_surface_revisions[slot_index]));
                    #[cfg(feature = "profiler")]
                    if let Some(event_time_us) = frame_pointer_event_us[slot_index].take() {
                        record_pointer_event_latency(
                            "input.libinput.pointer_motion.pipeline.event_to_primary_scanout_ns",
                            event_time_us,
                        );
                    }
                    if hardware_cursor
                        .as_ref()
                        .is_some_and(HardwareCursor::ready_to_retire)
                    {
                        hardware_cursor = None;
                    }
                }
                Ok(()) => {
                    if let Some(slot_index) = primary_slot {
                        frame_slots[slot_index]
                            .page_flip_submitted()
                            .map_err(app_error)?;
                    }
                    if let Some(snapshot) = cursor_snapshot {
                        hardware_cursor
                            .as_mut()
                            .expect("submitted atomic cursor remains owned")
                            .mark_submitted(snapshot)?;
                    }
                    #[cfg(feature = "profiler")]
                    let cursor_event_us = if cursor_snapshot.is_some() {
                        let event_time_us = pending_deferred_cursor_event_us.take();
                        if let Some(event_time_us) = event_time_us {
                            record_pointer_event_latency(
                                "input.libinput.pointer_motion.pipeline.freshest_event_to_cursor_submit_ns",
                                event_time_us,
                            );
                        }
                        event_time_us
                    } else {
                        None
                    };
                    pending_kms_commit = Some(PendingKmsCommit {
                        primary_slot,
                        cursor: cursor_snapshot,
                        #[cfg(feature = "profiler")]
                        cursor_event_us,
                    });
                    #[cfg(feature = "profiler")]
                    if cursor_snapshot.is_some() {
                        crate::profiler::record_instant(
                            "presentation.cursor.hardware_atomic_submit",
                        );
                    }
                }
                Err(error) if cursor_snapshot.is_some() && !cursor_fallback_submission => {
                    if let Some(slot_index) = primary_slot {
                        frame_slots[slot_index].discard_ready().map_err(app_error)?;
                        #[cfg(feature = "profiler")]
                        {
                            frame_pointer_event_us[slot_index] = None;
                        }
                    }
                    let cursor = hardware_cursor
                        .as_mut()
                        .expect("failed atomic cursor remains owned");
                    cursor.request_composited_fallback();
                    if cursor.ready_to_retire() {
                        hardware_cursor = None;
                    }
                    repaint = true;
                    #[cfg(feature = "profiler")]
                    {
                        pending_primary_pointer_event_us = pending_deferred_cursor_event_us.take();
                        crate::profiler::record_instant(
                            "presentation.cursor.hardware_atomic_failed",
                        );
                    }
                    let _ = error;
                }
                Err(error) => return Err(app_error(error)),
            }
        }

        latency_trace.phase("frame_callbacks_flush");
        if presentation_completed {
            resize_configure_budget = true;
            window_backend::flush(
                &display,
                &mut wayland,
                &mut windows,
                &mut configure_scheduler,
                &mut resize_configure_budget,
                &config,
                work_area,
                #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
                compatibility.as_mut(),
            )?;
            if let Some(lock) = pending_session_lock.take() {
                wayland
                    .session_lock_frame_presented(lock)
                    .map_err(app_error)?;
            }
            let time = u32::try_from(start.elapsed().as_millis()).unwrap_or(u32::MAX);
            for (surface, revision) in presented_surface_revisions {
                resize_trace::event(surface, "presented", format_args!("revision={revision}"));
                let Some(surface) = WaylandSurfaceId::from_raw(surface) else {
                    continue;
                };
                if wayland.core().world.surface(surface).is_some() {
                    wayland
                        .surface_presented(surface, revision, time)
                        .map_err(app_error)?;
                }
            }
            // Cursor surfaces have a separate composited/hardware-plane image lifecycle. Preserve
            // their existing primary-frame pacing; window/drag-image revisions above are frame-owned.
            for (surface, window) in windows
                .iter()
                .filter(|(_, window)| window.role == SurfaceRole::Cursor)
            {
                if window.presentation.revision != 0 && !session_locked {
                    wayland
                        .surface_presented(*surface, window.presentation.revision, time)
                        .map_err(app_error)?;
                }
            }
            let animation_now = MonotonicInstant::from_nanos(
                start.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64,
            );
            let (_, _, animation_active) = desktop_runtime_schedule(
                &frame_layers,
                pointer.as_ref(),
                &icon_layers,
                &widgets,
                animation_now,
            );
            repaint |= animation_active || window_motion.active(animation_now.as_nanos());
        }

        // Frame callbacks, completed buffer releases and configure events unblock client work.
        // Let clients start their next frame while we prepare ours; no nested dispatch or roundtrip.
        display.flush_clients();
        let available_scanout = frame_slots
            .iter()
            .position(|slot| slot.state == FrameSlotState::Available);
        if repaint && available_scanout.is_none() {
            latency_trace.event(
                "no_scanout_slot",
                [
                    ready_scanout.len() as u64,
                    u64::from(pending_kms_commit.is_some()),
                    0,
                    0,
                ],
            );
        }
        let primary_render_ready = state::primary_render_budget(
            frame_slots
                .iter()
                .filter(|slot| slot.state == FrameSlotState::GpuSubmitted)
                .count(),
            ready_scanout.len(),
        );
        if repaint && !primary_render_ready {
            latency_trace.event(
                "primary_render_backpressure",
                [ready_scanout.len() as u64, 0, 0, 0],
            );
        }
        if repaint
            && seat.state() == SeatState::Enabled
            && let Some(scanout_index) = available_scanout
            && primary_render_ready
        {
            latency_trace.phase("render_chrome");
            let frame_stats_start = frame_stats.as_ref().map(|_| Instant::now());
            #[cfg(feature = "profiler")]
            let _profile_suppression = (pointer_motion_only
                && !crate::profiler::pointer_move_events_enabled())
            .then(crate::profiler::suppress_current_thread);
            #[cfg(feature = "profiler")]
            let _profile_frame = crate::profiler::start_frame("frame.total");
            #[cfg(feature = "profiler")]
            crate::profiler::counter!(
                "frame.trigger.pointer_move_only",
                u8::from(pointer_motion_only)
            );
            #[cfg(feature = "profiler")]
            {
                crate::profiler::counter!(
                    "presentation.scanout.available_slots",
                    frame_slots
                        .iter()
                        .filter(|slot| slot.state == FrameSlotState::Available)
                        .count() as u64
                );
                crate::profiler::counter!(
                    "presentation.scanout.gpu_in_flight",
                    frame_slots
                        .iter()
                        .filter(|slot| slot.state == FrameSlotState::GpuSubmitted)
                        .count() as u64
                );
                crate::profiler::counter!(
                    "presentation.scanout.ready_frames",
                    ready_scanout.len() as u64
                );
                crate::profiler::counter!(
                    "presentation.scanout.flip_pending",
                    u8::from(pending_kms_commit.is_some())
                );
            }
            let now = start.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
            if !session_locked {
                refresh_window_frames(
                    window_frame.as_ref(),
                    &mut frame_layers,
                    &mut windows,
                    &wayland,
                    &config,
                    assets,
                    &app_icon_profile,
                    &runtime_wake,
                    now,
                    output_scale,
                    work_area,
                    &mut configure_scheduler,
                )?;
                // Publish sizes derived from the actual maximized chrome before presenting it.
                window_backend::flush(
                    &display,
                    &mut wayland,
                    &mut windows,
                    &mut configure_scheduler,
                    &mut resize_configure_budget,
                    &config,
                    work_area,
                    #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
                    compatibility.as_mut(),
                )?;
            }
            let cursor_image = wayland
                .core()
                .seats
                .get(&1)
                .expect("seat registered")
                .cursor;
            latency_trace.phase("render_cursor");
            let rendered_cursor = render_cursor_image(
                cursor_image,
                &mut pointer,
                &mut icon_layers,
                &windows,
                config.pointer_extent,
                now,
                &pointer_config,
                pointer_theme.as_ref(),
                &mut pointer_media,
                output_scale,
            )?;
            let mut cursor_on_hardware = false;
            if let Some(cursor) = &mut hardware_cursor {
                cursor.move_to(output_scale.physical_point(pointer_position));
                if !cursor.composited_fallback_requested() {
                    let update = if let Some(image) =
                        rendered_cursor.as_ref().and_then(CursorVisual::image)
                    {
                        cursor.set_logical_image(image, output_scale).map(|_| true)
                    } else {
                        cursor.hide();
                        Ok(rendered_cursor.is_none())
                    };
                    match update {
                        Ok(on_hardware) => {
                            cursor_on_hardware = on_hardware;
                            #[cfg(feature = "profiler")]
                            if on_hardware {
                                crate::profiler::record_instant(
                                    "presentation.cursor.hardware_image_staged",
                                );
                            }
                        }
                        Err(_) => {
                            cursor.request_composited_fallback();
                            #[cfg(feature = "profiler")]
                            {
                                pending_primary_pointer_event_us = latest_pointer_event_this_turn
                                    .or_else(|| pending_deferred_cursor_event_us.take());
                                crate::profiler::record_instant(
                                    "presentation.cursor.hardware_image_failed",
                                );
                            }
                        }
                    }
                }
            }
            if hardware_cursor
                .as_ref()
                .is_some_and(HardwareCursor::ready_to_retire)
            {
                hardware_cursor = None;
            }
            let eligible = windows
                .iter()
                .filter_map(|(surface, window)| {
                    (!window.minimized
                        && resize_veil_owner(&windows, *surface).is_none()
                        && (window.role == SurfaceRole::SessionLock) == session_locked)
                        .then_some(*surface)
                })
                .collect();
            let mut ready = desktop_renderer.prepare_render(&eligible, &mut latency_trace)?;
            dma_buf_readiness::retain_ready_families(&windows, &eligible, &mut ready);
            for (surface, window) in &mut windows {
                window.presentation.content_ready = ready.contains(surface);
            }
            latency_trace.phase("render_layers");
            let layers = prepare_desktop_layers(
                session_locked,
                extent,
                now,
                first_modeset,
                &mut frame_layers,
                &mut windows,
                &stacking_order,
                &mut widgets,
                &mut icon_layers,
                &mut pointer,
                wayland.drag_icon(1),
                drag_position,
                (!cursor_on_hardware)
                    .then_some(rendered_cursor.as_ref())
                    .flatten(),
                pointer_position,
                &config,
            )?;
            latency_trace.phase("render_scene_sync");
            let Some(frame) =
                desktop_scene.synchronize_with_force(extent, layers, window_motion.active(now))
            else {
                repaint = false;
                continue;
            };
            let mut frame = frame.into_physical(output_scale, physical_extent);
            let mut motion_states = BTreeMap::new();
            let mut motion_owners = BTreeMap::new();
            if !session_locked {
                for (surface, window) in &windows {
                    if window.backend.is_some() && !window.fullscreen {
                        let outer = window
                            .chrome_outer
                            .unwrap_or_else(|| geometry::legacy_window_outer(window, &config));
                        let style = frame_layers
                            .get(surface)
                            .and_then(|layer| {
                                window_frame
                                    .as_ref()
                                    .and_then(|factory| factory.motion(&layer.model))
                            })
                            .unwrap_or(config.window_motion);
                        motion_states.insert(
                            surface.get(),
                            motion::WindowState {
                                bounds: output_scale.physical_rect(RectI {
                                    x: window.position.x,
                                    y: window.position.y,
                                    width: outer.width,
                                    height: outer.height,
                                }),
                                maximized: window.maximized,
                                interactive: matches!(window_interaction, Some(WindowInteraction::Move {surface:target,..} | WindowInteraction::Resize {surface:target,..}) if target == *surface),
                                move_pointer: matches!(window_interaction, Some(WindowInteraction::Move {surface:target,..}) if target == *surface)
                                    .then_some(PointF { x: pointer_position.x * output_scale.get(), y: pointer_position.y * output_scale.get() }),
                                minimized: window.minimized,
                                veiled: resize_veil_owner(&windows, *surface).is_some(),
                                style,
                                shadows: frame_layers.get(surface)
                                    .map(|layer| layer.shadows(output_scale.get()))
                                    .unwrap_or_default(),
                                corner_radii: frame_layers.get(surface)
                                    .map(|layer| layer.corner_radii(output_scale.get() as f32))
                                    .unwrap_or_default(),
                            },
                        );
                    }
                }
                for surface in windows.keys() {
                    let mut owner = *surface;
                    for _ in 0..windows.len() {
                        if motion_states.contains_key(&owner.get()) {
                            motion_owners.insert(surface.get(), owner.get());
                            break;
                        }
                        let Some(parent) = windows.get(&owner).and_then(|w| w.parent) else {
                            break;
                        };
                        owner = parent;
                    }
                }
            }
            for (surface, window) in &mut windows {
                window.motion_style = motion_states
                    .get(&surface.get())
                    .map_or(crate::WindowMotion::none(), |state| state.style);
            }
            window_motion.apply(
                &mut frame,
                motion_states,
                &motion_owners,
                now,
                if desktop_renderer.motion_enabled() {
                    config.motion_preference
                } else {
                    crate::theme::MotionPreference::Reduced
                },
            );
            let mut consumed_motion_veil = false;
            for (surface, window) in &mut windows {
                consumed_motion_veil |= std::mem::take(&mut window.motion_veil_pending);
                window.motion_input = motion_owners
                    .get(&surface.get())
                    .and_then(|root| window_motion.input(*root, output_scale.get()));
            }
            let frame_id = next_frame_id;
            for (surface, revision) in &frame.surface_revisions {
                resize_trace::event(
                    *surface,
                    "render-request",
                    format_args!(
                        "frame={} slot={} revision={}",
                        frame_id, scanout_index, revision
                    ),
                );
            }
            frame_surface_revisions[scanout_index] = frame
                .surface_revisions
                .iter()
                .filter(|(surface, _)| !frame.motion.hidden_revisions.contains(surface))
                .copied()
                .collect();
            next_frame_id = next_frame_id.wrapping_add(1).max(1);
            frame_slots[scanout_index]
                .begin_render(frame_id)
                .map_err(app_error)?;
            latency_trace.phase("render_backend");
            let rendered = desktop_renderer.render(scanout_index, frame, &mut latency_trace)?;
            if !desktop_renderer.motion_enabled() {
                window_motion = Default::default();
                for window in windows.values_mut() {
                    window.motion_input = None;
                }
            }
            latency_trace.phase("render_retire");
            latency_trace.event("render_submitted", [scanout_index as u64, frame_id, 0, 0]);
            for &(surface, revision) in &frame_surface_revisions[scanout_index] {
                latency_trace.event(
                    "frame_surface",
                    [scanout_index as u64, frame_id, surface as u64, revision],
                );
            }
            match rendered {
                ShellRenderResult::Vulkan {
                    releases,
                    discarded,
                } => {
                    for release in releases {
                        finish_dma_buf_release(
                            &mut wayland,
                            release.retirement,
                            Some(release.fence),
                        )?;
                    }
                    for retirement in discarded {
                        retire_unsubmitted_dma_buf(
                            &mut wayland,
                            &mut pending_dma_bufs,
                            retirement,
                        )?;
                    }
                    frame_slots[scanout_index]
                        .gpu_submitted()
                        .map_err(app_error)?;
                }
                ShellRenderResult::Software {
                    damage: scanout_region,
                } => {
                    #[cfg(feature = "profiler")]
                    {
                        crate::profiler::counter!(
                            "render.upload_bytes",
                            rect_area(scanout_region).saturating_mul(4)
                        );
                        crate::profiler::counter!("render.damage_area", rect_area(scanout_region));
                    }
                    scanout_buffers[scanout_index]
                        .write_rgba8_region(
                            desktop_renderer
                                .software_pixels()
                                .expect("software result owns software pixels"),
                            scanout_region,
                        )
                        .map_err(app_error)?;
                    desktop_renderer.mark_software_copied(scanout_index);
                    frame_slots[scanout_index]
                        .gpu_submitted()
                        .and_then(|_| frame_slots[scanout_index].gpu_completed())
                        .map_err(app_error)?;
                    ready_scanout.push_back(scanout_index);
                }
            }
            if let (Some(stats), Some(started)) = (&mut frame_stats, frame_stats_start) {
                stats.rendered(started.elapsed());
            }
            #[cfg(feature = "profiler")]
            {
                frame_pointer_event_us[scanout_index] = pending_primary_pointer_event_us.take();
                if let Some(event_time_us) = frame_pointer_event_us[scanout_index] {
                    record_pointer_event_latency(
                        "input.libinput.pointer_motion.pipeline.event_to_primary_submit_ns",
                        event_time_us,
                    );
                }
            }
            repaint = consumed_motion_veil;
        }
    }
}

#[cfg(feature = "profiler")]
fn record_libinput_event(
    kind: LinuxInputEventKind,
    event_time_us: u64,
    observed_time_us: Option<u64>,
) {
    use crate::profiler::InputRecordingSource;

    let queue_age_ns = observed_time_us
        .unwrap_or(event_time_us)
        .saturating_sub(event_time_us)
        .saturating_mul(1_000);
    let (source, label, has_queue_age) = match kind {
        LinuxInputEventKind::PointerMotion { .. } => (
            InputRecordingSource::PointerMotion,
            "input.libinput.pointer_motion.relative.queue_age_ns",
            true,
        ),
        LinuxInputEventKind::PointerAbsolute { .. } => (
            InputRecordingSource::PointerMotion,
            "input.libinput.pointer_motion.absolute.queue_age_ns",
            true,
        ),
        LinuxInputEventKind::PointerButton { pressed: true, .. } => (
            InputRecordingSource::PointerButton,
            "input.libinput.pointer_button.pressed.queue_age_ns",
            true,
        ),
        LinuxInputEventKind::PointerButton { pressed: false, .. } => (
            InputRecordingSource::PointerButton,
            "input.libinput.pointer_button.released.queue_age_ns",
            true,
        ),
        LinuxInputEventKind::PointerAxis { .. } => (
            InputRecordingSource::Scroll,
            "input.libinput.scroll.queue_age_ns",
            true,
        ),
        LinuxInputEventKind::KeyboardKey { pressed: true, .. } => (
            InputRecordingSource::Keyboard,
            "input.libinput.keyboard.pressed.queue_age_ns",
            true,
        ),
        LinuxInputEventKind::KeyboardKey { pressed: false, .. } => (
            InputRecordingSource::Keyboard,
            "input.libinput.keyboard.released.queue_age_ns",
            true,
        ),
        LinuxInputEventKind::TouchMotion { .. } => (
            InputRecordingSource::TouchMotion,
            "input.libinput.touch_motion.queue_age_ns",
            true,
        ),
        LinuxInputEventKind::TouchDown { .. } => (
            InputRecordingSource::TouchContact,
            "input.libinput.touch_contact.down.queue_age_ns",
            true,
        ),
        LinuxInputEventKind::TouchUp { .. } => (
            InputRecordingSource::TouchContact,
            "input.libinput.touch_contact.up.queue_age_ns",
            true,
        ),
        LinuxInputEventKind::TouchCancel => (
            InputRecordingSource::TouchContact,
            "input.libinput.touch_contact.cancel.queue_age_ns",
            true,
        ),
        LinuxInputEventKind::DeviceAdded => (
            InputRecordingSource::DeviceChange,
            "input.libinput.device_change.added",
            false,
        ),
        LinuxInputEventKind::DeviceRemoved => (
            InputRecordingSource::DeviceChange,
            "input.libinput.device_change.removed",
            false,
        ),
    };
    if !crate::profiler::input_recording_enabled(source) {
        return;
    }
    if has_queue_age {
        crate::profiler::record_instant_value(label, queue_age_ns);
    } else {
        crate::profiler::record_instant(label);
    }
}

fn output_state(
    connector: &crate::presenter_vulkan_kms::KmsConnector,
    current_mode: usize,
    scale: crate::platform::ScaleFactor,
) -> AppResult<OutputState> {
    let name = format!(
        "DRM-{}-{}",
        connector.connector_type, connector.connector_type_id
    );
    let modes = connector
        .modes
        .iter()
        .map(|mode| OutputMode {
            size: mode.size(),
            refresh_millihertz: mode.refresh_millihertz(),
            preferred: mode.preferred(),
        })
        .collect();
    OutputState::new(
        OutputDescription {
            name: name.clone(),
            description: format!("Telorgon output {name}"),
            make: "Unknown".to_owned(),
            model: name,
            physical_millimeters: connector.physical_millimeters,
            logical_position: PointI::default(),
            scale,
            transform: OutputTransform::Normal,
            modes,
        },
        current_mode,
    )
    .map_err(app_error)
}

fn is_plane_type(kms: &KmsDevice, plane: u32, expected: u64) -> bool {
    KmsTopology::object_properties(kms, plane, KmsPropertyObject::Plane)
        .ok()
        .and_then(|properties| properties.named("type").map(|property| property.value))
        == Some(expected)
}

fn app_error(error: impl std::fmt::Display) -> AppError {
    AppError::new(error.to_string())
}

fn select_drm_device<'seat>(
    seat: &'seat LinuxSeat,
    config: &LinuxShellConfig,
) -> AppResult<(
    crate::platform_linux::SeatDevice<'seat>,
    KmsDevice,
    KmsTopology,
    std::path::PathBuf,
)> {
    let candidates = if let Some(path) = &config.drm_device {
        vec![path.clone()]
    } else {
        let mut paths = std::fs::read_dir("/dev/dri")
            .map_err(app_error)?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name.strip_prefix("card").is_some_and(|suffix| {
                            !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit())
                        })
                    })
            })
            .collect::<Vec<_>>();
        paths.sort();
        paths
    };
    let mut failures = Vec::new();
    for path in candidates {
        let attempt = (|| {
            let device = seat
                .open_device(
                    path.to_str()
                        .ok_or_else(|| AppError::new("DRM device path is not UTF-8"))?,
                )
                .map_err(app_error)?;
            let kms =
                KmsDevice::new(device.try_clone_fd().map_err(app_error)?).map_err(app_error)?;
            let topology = KmsTopology::query(&kms).map_err(app_error)?;
            if !topology.connectors.iter().any(|connector| {
                connector.status == ConnectorStatus::Connected && !connector.modes.is_empty()
            }) {
                return Err(AppError::new("no connected output with a mode"));
            }
            Ok((device, kms, topology))
        })();
        match attempt {
            Ok((device, kms, topology)) => return Ok((device, kms, topology, path)),
            Err(error) => failures.push(format!("{}: {error}", path.display())),
        }
    }
    Err(AppError::new(format!(
        "no usable seat-accessible KMS device; check the local VT, graphics driver and connected monitor. {}",
        failures.join("; ")
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_cursor_motion_coalesces_while_a_commit_is_in_flight() {
        let mut cursor = CursorCommitTracker::default();
        cursor.move_to(PointI { x: 10, y: 20 });
        assert_eq!(cursor.desired_submission(), None);

        cursor.show(0, PointI::default());
        let submitted = cursor
            .desired_submission()
            .expect("visible cursor is dirty");
        cursor.mark_submitted(submitted).unwrap();
        cursor.move_to(PointI { x: 30, y: 40 });
        assert_eq!(cursor.desired_submission(), None);

        cursor.mark_completed(submitted).unwrap();
        let coalesced = cursor
            .desired_submission()
            .expect("newest position follows the completed generation");
        assert_eq!(coalesced.position, PointI { x: 30, y: 40 });
        assert_eq!(coalesced.buffer, Some(0));
    }

    #[test]
    fn atomic_cursor_staging_never_reuses_current_or_in_flight_buffers() {
        let mut cursor = CursorCommitTracker::default();
        cursor.show(0, PointI::default());
        let first = cursor.desired_submission().unwrap();
        cursor.mark_submitted(first).unwrap();
        cursor.mark_completed(first).unwrap();

        cursor.show(1, PointI::default());
        let second = cursor.desired_submission().unwrap();
        cursor.mark_submitted(second).unwrap();

        assert_eq!(cursor.current_buffer, Some(0));
        assert_eq!(cursor.in_flight.and_then(|state| state.buffer), Some(1));
        assert_eq!(
            cursor.reusable_buffer(HARDWARE_CURSOR_BUFFER_COUNT),
            Some(2)
        );
    }

    #[test]
    fn hardware_cursor_fallback_waits_for_plane_disable_completion() {
        let mut cursor = CursorCommitTracker::default();
        cursor.show(0, PointI::default());
        let visible = cursor.desired_submission().unwrap();
        cursor.mark_submitted(visible).unwrap();
        cursor.mark_completed(visible).unwrap();

        cursor.request_composited_fallback();
        assert!(!cursor.ready_to_retire());
        let hidden = cursor.desired_submission().expect("plane disable is dirty");
        assert!(!hidden.visible);
        cursor.mark_submitted(hidden).unwrap();
        cursor.mark_completed(hidden).unwrap();
        assert!(cursor.ready_to_retire());
    }

    #[test]
    fn vulkan_staging_budget_covers_one_full_hd_upload_per_slot() {
        let slots = 2;
        let budget = vulkan_staging_budget_bytes(
            SizeI {
                width: 1920,
                height: 1080,
            },
            slots,
        )
        .expect("Full HD staging budget should fit");
        let bytes_per_slot = budget / slots as u64;

        assert_eq!(
            bytes_per_slot,
            1920 * 1080 * 4 + VULKAN_STAGING_HEADROOM_BYTES_PER_SLOT
        );
        // This is the upload size reported by the original Raspberry Pi Full HD failure.
        assert!(bytes_per_slot >= 8_294_628);
    }

    #[test]
    fn vulkan_staging_budget_preserves_headroom_for_small_outputs() {
        let slots = 3;
        let budget = vulkan_staging_budget_bytes(
            SizeI {
                width: 640,
                height: 480,
            },
            slots,
        )
        .expect("small scanout staging budget should fit");

        assert_eq!(
            budget,
            (640 * 480 * 4 + VULKAN_STAGING_HEADROOM_BYTES_PER_SLOT) * slots as u64
        );
        assert!(budget >= VULKAN_STAGING_MIN_BYTES_PER_SLOT * slots as u64);
    }

    #[test]
    fn vulkan_staging_budget_rejects_total_slot_overflow() {
        assert!(
            vulkan_staging_budget_bytes(
                SizeI {
                    width: i32::MAX,
                    height: i32::MAX,
                },
                usize::MAX,
            )
            .is_err()
        );
    }

    #[test]
    fn accumulated_damage_unions_every_change_since_the_target_version() {
        let history = VecDeque::from([
            (1, None),
            (
                2,
                Some(RectI {
                    x: 10,
                    y: 20,
                    width: 30,
                    height: 40,
                }),
            ),
            (
                3,
                Some(RectI {
                    x: 35,
                    y: 45,
                    width: 20,
                    height: 10,
                }),
            ),
        ]);

        assert_eq!(
            accumulated_damage(
                1,
                3,
                &history,
                SizeI {
                    width: 100,
                    height: 100,
                },
            ),
            Some(RectI {
                x: 10,
                y: 20,
                width: 45,
                height: 40,
            })
        );
    }

    #[test]
    fn accumulated_damage_requires_full_redraw_after_a_full_change_or_history_gap() {
        let extent = SizeI {
            width: 100,
            height: 100,
        };
        let full_change = VecDeque::from([
            (
                2,
                Some(RectI {
                    x: 1,
                    y: 1,
                    width: 2,
                    height: 2,
                }),
            ),
            (3, None),
        ]);
        let history_gap = VecDeque::from([(
            4,
            Some(RectI {
                x: 1,
                y: 1,
                width: 2,
                height: 2,
            }),
        )]);

        assert_eq!(accumulated_damage(1, 3, &full_change, extent), None);
        assert_eq!(accumulated_damage(1, 4, &history_gap, extent), None);
        assert_eq!(accumulated_damage(0, 4, &history_gap, extent), None);
    }

    #[test]
    fn every_wayland_cursor_shape_round_trips_through_pointer_icons() {
        for shape in 1..=36 {
            let icon = cursor_shape_pointer_icon(shape).expect("shape must be supported");
            assert_eq!(pointer_icon_cursor_shape(icon), shape);
            assert!(cursor_shape_icon_name(shape).is_some());
        }
    }

    #[test]
    fn cursor_image_transitions_require_presentation_without_scene_damage() {
        assert!(cursor_transition_requires_presentation(
            CursorImage::Hidden,
            CursorImage::Shape(pointer_icon_cursor_shape(PointerIcon::Move)),
        ));
        assert!(cursor_transition_requires_presentation(
            CursorImage::Shape(pointer_icon_cursor_shape(PointerIcon::Move)),
            CursorImage::TelorgonDefault,
        ));
        assert!(!cursor_transition_requires_presentation(
            CursorImage::TelorgonDefault,
            CursorImage::TelorgonDefault,
        ));
    }

    #[test]
    fn client_commits_do_not_roll_back_a_newer_requested_resize() {
        let committed = SizeI {
            width: 640,
            height: 480,
        };
        let requested = SizeI {
            width: 720,
            height: 540,
        };

        assert_eq!(retained_requested_size(None, committed), committed);
        assert_eq!(
            retained_requested_size(Some(requested), committed),
            requested
        );
    }

    #[test]
    fn resize_drag_geometry_is_derived_from_the_press_time_baseline() {
        let position = PointI { x: 100, y: 80 };
        let size = SizeI {
            width: 400,
            height: 300,
        };
        let output = SizeI {
            width: 1920,
            height: 1080,
        };

        assert_eq!(
            resize_drag_geometry(
                position,
                size,
                ResizeEdge::Right,
                PointI { x: 50, y: 0 },
                output,
            ),
            (
                position,
                SizeI {
                    width: 450,
                    height: 300,
                },
            )
        );
        assert_eq!(
            resize_drag_geometry(
                position,
                size,
                ResizeEdge::Right,
                PointI { x: 20, y: 0 },
                output,
            )
            .1
            .width,
            420
        );
        assert_eq!(
            resize_drag_geometry(
                position,
                size,
                ResizeEdge::Bottom,
                PointI { x: 0, y: 70 },
                output,
            )
            .1
            .height,
            370
        );
    }

    #[test]
    fn left_and_top_resize_edges_keep_the_opposite_edges_anchored() {
        let position = PointI { x: 100, y: 80 };
        let size = SizeI {
            width: 400,
            height: 300,
        };
        let output = SizeI {
            width: 1920,
            height: 1080,
        };
        let (resized_position, resized_size) = resize_drag_geometry(
            position,
            size,
            ResizeEdge::TopLeft,
            PointI { x: 40, y: 30 },
            output,
        );

        assert_eq!(resized_position, PointI { x: 140, y: 110 });
        assert_eq!(
            resized_size,
            SizeI {
                width: 360,
                height: 270,
            }
        );
        assert_eq!(
            resized_position.x + resized_size.width,
            position.x + size.width
        );
        assert_eq!(
            resized_position.y + resized_size.height,
            position.y + size.height
        );

        let (minimum_position, minimum_size) = resize_drag_geometry(
            position,
            size,
            ResizeEdge::TopLeft,
            PointI {
                x: 10_000,
                y: 10_000,
            },
            output,
        );
        assert_eq!(
            minimum_size,
            SizeI {
                width: 1,
                height: 1
            }
        );
        assert_eq!(
            minimum_position.x + minimum_size.width,
            position.x + size.width
        );
        assert_eq!(
            minimum_position.y + minimum_size.height,
            position.y + size.height
        );
    }
}

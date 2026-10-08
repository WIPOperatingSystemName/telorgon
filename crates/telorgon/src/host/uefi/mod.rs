//! Firmware-owned scheduling and presentation of the shared boot interface.

use std::time::Duration;

use crate::boot::{
    BootCommand, BootError, BootHostEvent, BootPhase, BootProgress, BootScreens, BootSession,
    BootSource, BootTargetKind, BootTheme, ReadyBootApplication, SPLASH_THEME_DISKS,
    SPLASH_THEME_VOXEL, SplashFramebuffer, SplashHandoff, SplashHandoffError,
};
use crate::foundation::{MonotonicInstant, SizeF, SizeI};
use crate::graphics::presentation::uefi::{PixelFormat, UefiPresenter};
use crate::graphics::presentation::{AlphaMode, ColorSpace, SurfaceMetrics, SurfaceRevision};
use crate::host::embedded::{SoftwareUiHost, SoftwareUiHostError};
use crate::input::{ButtonState, InputEvent, KeyEvent, LogicalKey, NamedKey, PhysicalKey};
use crate::platform::uefi::{UefiClock, UefiContext, UefiError, UefiKey};
use crate::runtime::CompositionDriver;

#[derive(Debug, thiserror::Error)]
pub enum UefiBootError {
    #[error(transparent)]
    Boot(#[from] BootError),
    #[error(transparent)]
    Firmware(#[from] UefiError),
    #[error(transparent)]
    Ui(#[from] SoftwareUiHostError),
    #[error("invalid firmware host configuration: {0}")]
    Configuration(&'static str),
    #[error(transparent)]
    Handoff(#[from] SplashHandoffError),
}

#[derive(Clone, Copy, Debug)]
pub struct UefiBootOptions {
    pub frame_interval: Duration,
    /// Grace period for Escape to open the selector before a single target launches.
    /// Zero removes the extra wait; buffered Escape is still checked before launch.
    pub selector_override_timeout: Duration,
    /// Prefer a supported interactive mode; None retains the firmware's current mode.
    /// An unavailable resolution falls back to the current mode.
    pub preferred_resolution: Option<(u32, u32)>,
    pub max_image_bytes: usize,
    pub max_framebuffer_pixels: usize,
    /// The built-in selector keys work before any widget has keyboard focus.
    pub boot_shortcuts: bool,
}

impl Default for UefiBootOptions {
    fn default() -> Self {
        Self {
            frame_interval: Duration::from_millis(33),
            selector_override_timeout: Duration::from_millis(750),
            preferred_resolution: Some((1024, 768)),
            max_image_bytes: 512 * 1024 * 1024,
            max_framebuffer_pixels: 16 * 1024 * 1024,
            boot_shortcuts: true,
        }
    }
}

impl<S: BootScreens> ReadyBootApplication<S> {
    pub fn run_uefi(self, firmware: &UefiContext) -> Result<(), UefiBootError> {
        run(self.into_session()?, firmware, UefiBootOptions::default())
    }

    pub fn run_uefi_with(
        self,
        firmware: &UefiContext,
        options: UefiBootOptions,
    ) -> Result<(), UefiBootError> {
        run(self.into_session()?, firmware, options)
    }
}

/// Returning from this function returns to the caller's EFI application. Starting an OS
/// normally transfers control permanently; only a returning EFI application resumes this loop.
pub fn run<C: crate::authoring::compose::Component>(
    session: BootSession<C>,
    firmware: &UefiContext,
    options: UefiBootOptions,
) -> Result<(), UefiBootError> {
    if options.frame_interval < Duration::from_millis(10)
        || options.frame_interval > Duration::from_secs(1)
        || options.selector_override_timeout > Duration::from_secs(5)
        || options.max_image_bytes == 0
        || options.max_framebuffer_pixels == 0
        || options.preferred_resolution.is_some_and(|(width, height)| {
            width == 0
                || height == 0
                || u64::from(width) * u64::from(height) > options.max_framebuffer_pixels as u64
        })
    {
        return Err(UefiBootError::Configuration(
            "invalid rendering or image limits",
        ));
    }
    firmware.set_watchdog(0)?;
    firmware.clear_text()?;
    let mut presenter = UefiPresenter::new(firmware)?;
    if let Some((width, height)) = options.preferred_resolution {
        presenter.set_resolution(width, height)?;
    }
    let clock = UefiClock::new(firmware)?;
    let controller = session.controller;
    let mut metrics = metrics(presenter.size(), options.max_framebuffer_pixels)?;
    let mut ui = SoftwareUiHost::from_composed(session.component, metrics)?;
    let mut startup_pending = single_target_startup(&controller.snapshot());
    let mut startup_escape = StartupEscape::default();
    if startup_pending && options.boot_shortcuts {
        let deadline = clock
            .now()
            .saturating_add(options.selector_override_timeout);
        if poll_startup_escape(firmware, &clock, deadline)? {
            controller.dispatch(BootCommand::Reset);
            startup_escape.latch(clock.now());
            startup_pending = false;
        }
    }
    let mut previous = clock.now();
    let mut force = true;

    loop {
        let elapsed = clock.now();
        controller.advance(elapsed.saturating_sub(previous));
        previous = elapsed;
        while let Some(key) = firmware.read_key()? {
            if options.boot_shortcuts && startup_escape.consume(key, clock.now()) {
                continue;
            }
            if options.boot_shortcuts && startup_pending && key.is_escape() {
                controller.dispatch(BootCommand::Reset);
                startup_escape.latch(clock.now());
                startup_pending = false;
                continue;
            }
            if options.boot_shortcuts {
                match shortcut(key, controller.snapshot().phase) {
                    Shortcut::Command(command) => {
                        controller.dispatch(command);
                        continue;
                    }
                    Shortcut::Return => {
                        firmware.set_watchdog(300)?;
                        return Ok(());
                    }
                    Shortcut::ToggleTheme => {
                        let theme = match controller.snapshot().theme {
                            BootTheme::Disks => BootTheme::Voxel,
                            BootTheme::Voxel => BootTheme::Disks,
                        };
                        controller.dispatch(BootCommand::SetTheme(theme));
                        continue;
                    }
                    Shortcut::Pass => {}
                }
            }
            if let Some(logical) = logical_key(key) {
                // Simple Text Input supplies strokes, not physical release reports.
                // A marked synthetic activation pulse avoids leaving held-key state behind.
                for state in [ButtonState::Pressed, ButtonState::Released] {
                    ui.queue_input(InputEvent::Key(
                        KeyEvent::new(PhysicalKey::UNIDENTIFIED, state)
                            .with_logical_key(logical.clone())
                            .with_synthetic(true),
                    ));
                }
            }
        }
        startup_escape.quiet(clock.now());
        paint(&mut ui, &mut presenter, elapsed, force)?;
        force = false;

        if startup_pending && options.boot_shortcuts {
            // Font setup and the first splash can take time. Escape entered during that
            // paint must still cancel the queued request before the host takes ownership.
            if poll_startup_escape(firmware, &clock, clock.now())? {
                controller.dispatch(BootCommand::Reset);
                startup_escape.latch(clock.now());
                force = true;
            }
        }
        startup_pending = false;

        if let Some(request) = controller.take_request() {
            controller.report(BootHostEvent::Loading {
                request: request.id,
                status: "Reading and validating the EFI image".into(),
                progress: BootProgress::Unknown,
            })?;
            paint(&mut ui, &mut presenter, clock.now(), true)?;
            let BootSource::EfiImage {
                path,
                options: load_options,
                ..
            } = &request.source;
            let mut bytes = Vec::with_capacity(load_options.len() * 2);
            for unit in load_options {
                bytes.extend_from_slice(&unit.to_le_bytes());
            }
            let loaded = firmware.load_image(path, &bytes, options.max_image_bytes);
            let result = match loaded {
                Ok(image) => {
                    controller.report(BootHostEvent::Handoff {
                        request: request.id,
                    })?;
                    paint(&mut ui, &mut presenter, clock.now(), true)?;
                    // A cooperating custom EFI loader copies this metadata before exiting
                    // boot services. The guard keeps the table valid across StartImage.
                    let _handoff = if request.target.kind == BootTargetKind::Custom {
                        let payload =
                            splash_handoff(&presenter, controller.snapshot().theme, clock.now())?;
                        Some(firmware.install_splash_handoff(&payload)?)
                    } else {
                        None
                    };
                    firmware.set_watchdog(300)?;
                    image.start()
                }
                Err(error) => Err(error),
            };
            if !firmware.boot_services_active() {
                // A receiver that exits services must never return. Avoid all UI callbacks,
                // Rust destruction, allocator activity, or firmware calls if it violates that ABI.
                loop {
                    core::hint::spin_loop();
                }
            }
            firmware.set_watchdog(0)?;
            let message = match result {
                Ok(exit) if exit.status.is_error() => {
                    format!("EFI target returned error {:#x}", exit.status.as_usize())
                }
                Ok(_) => "The EFI application returned control to the selector".into(),
                Err(error) => error.to_string(),
            };
            controller.report(BootHostEvent::Failed {
                request: request.id,
                message,
            })?;
            presenter.refresh()?;
            metrics
                .revision
                .advance()
                .map_err(|_| UefiBootError::Configuration("surface revision exhausted"))?;
            let mut new_metrics = self::metrics(presenter.size(), options.max_framebuffer_pixels)?;
            new_metrics.revision = metrics.revision;
            metrics = new_metrics;
            ui.configure(metrics)?;
            force = true;
        }
        firmware.wait_for_input(Some(options.frame_interval))?;
    }
}

fn single_target_startup(snapshot: &crate::boot::BootSnapshot) -> bool {
    snapshot.targets.len() == 1 && snapshot.phase == BootPhase::Loading
}

fn poll_startup_escape(
    firmware: &UefiContext,
    clock: &UefiClock<'_>,
    deadline: Duration,
) -> Result<bool, UefiBootError> {
    loop {
        while let Some(key) = firmware.read_key()? {
            if key.is_escape() {
                return Ok(true);
            }
        }
        let remaining = deadline.saturating_sub(clock.now());
        if remaining.is_zero() {
            return Ok(false);
        }
        firmware.wait_for_input(Some(remaining))?;
    }
}

#[derive(Default)]
struct StartupEscape {
    last_stroke: Option<Duration>,
}

impl StartupEscape {
    fn latch(&mut self, now: Duration) {
        self.last_stroke = Some(now);
    }

    fn consume(&mut self, key: UefiKey, now: Duration) -> bool {
        if key.is_escape() && self.last_stroke.is_some() {
            self.latch(now);
            true
        } else {
            false
        }
    }

    fn quiet(&mut self, now: Duration) {
        // Simple Text Input has no release reports. A quiet interval distinguishes the
        // startup hold's repeat strokes from a later Escape that returns to firmware.
        if self
            .last_stroke
            .is_some_and(|last| now.saturating_sub(last) >= Duration::from_secs(1))
        {
            self.last_stroke = None;
        }
    }
}

fn splash_handoff(
    presenter: &UefiPresenter<'_>,
    theme: BootTheme,
    elapsed: Duration,
) -> Result<SplashHandoff, UefiBootError> {
    let theme_id = match theme {
        BootTheme::Disks => SPLASH_THEME_DISKS,
        BootTheme::Voxel => SPLASH_THEME_VOXEL,
    };
    let payload = SplashHandoff::new(theme_id, elapsed.as_millis().min(u64::MAX as u128) as u64);
    let Some(layout) = presenter.framebuffer_layout() else {
        return Ok(payload);
    };
    let masks = match layout.pixel_format() {
        PixelFormat::RgbReserved => [0xff, 0xff00, 0xff0000, 0xff000000],
        PixelFormat::BgrReserved => [0xff0000, 0xff00, 0xff, 0xff000000],
        PixelFormat::BitMask(masks) => [masks.red, masks.green, masks.blue, masks.reserved],
        PixelFormat::BltOnly => return Ok(payload),
    };
    Ok(payload.with_framebuffer(SplashFramebuffer {
        address: presenter
            .framebuffer_address()
            .ok_or(UefiBootError::Configuration("framebuffer unavailable"))?,
        size: layout.required_bytes() as u64,
        width: layout.width(),
        height: layout.height(),
        stride_bytes: u32::try_from(layout.stride_bytes()).map_err(|_| {
            UefiBootError::Configuration("framebuffer stride exceeds handoff format")
        })?,
        pixel_bytes: layout.pixel_bytes() as u32,
        red_mask: masks[0],
        green_mask: masks[1],
        blue_mask: masks[2],
        reserved_mask: masks[3],
    })?)
}

fn paint(
    ui: &mut SoftwareUiHost<CompositionDriver>,
    presenter: &mut UefiPresenter<'_>,
    elapsed: Duration,
    force: bool,
) -> Result<(), UefiBootError> {
    let now = MonotonicInstant::from_nanos(u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX));
    let frame = ui.advance(now, force)?;
    if let Some(surface) = frame.surface() {
        let extent = surface.framebuffer_extent();
        presenter.present_rgba8(
            extent.width as u32,
            extent.height as u32,
            surface.pixels_rgba8(),
        )?;
    }
    Ok(())
}

fn metrics(size: (u32, u32), max_pixels: usize) -> Result<SurfaceMetrics, UefiBootError> {
    let (width, height) = size;
    let pixels = u64::from(width) * u64::from(height);
    if width == 0
        || height == 0
        || pixels > max_pixels as u64
        || width > i32::MAX as u32
        || height > i32::MAX as u32
    {
        return Err(UefiBootError::Configuration(
            "unsupported framebuffer dimensions",
        ));
    }
    // Keep the shared selector usable on small firmware modes, with uniform scaling.
    let scale = (width as f64 / 900.0).min(height as f64 / 600.0).min(1.0);
    Ok(SurfaceMetrics {
        revision: SurfaceRevision::INITIAL,
        logical_extent: SizeF {
            width: (width as f64 / scale) as f32,
            height: (height as f64 / scale) as f32,
        },
        physical_extent: SizeI {
            width: width as i32,
            height: height as i32,
        },
        scale_factor: scale,
        color_space: ColorSpace::Srgb,
        alpha_mode: AlphaMode::Opaque,
    })
}

enum Shortcut {
    Command(BootCommand),
    ToggleTheme,
    Return,
    Pass,
}

fn shortcut(key: UefiKey, phase: BootPhase) -> Shortcut {
    if key.is_escape() {
        return if phase == BootPhase::Selecting {
            Shortcut::Return
        } else {
            Shortcut::Command(BootCommand::Reset)
        };
    }
    if key.is_enter() || matches!(key.unicode, 66 | 98) {
        return Shortcut::Command(if phase == BootPhase::Failed {
            BootCommand::Retry
        } else {
            BootCommand::Launch
        });
    }
    match (key.scan_code, key.unicode) {
        (1 | 4, _) => Shortcut::Command(BootCommand::MoveSelection(-1)),
        (2 | 3, _) => Shortcut::Command(BootCommand::MoveSelection(1)),
        (_, 49..=57) => Shortcut::Command(BootCommand::Select((key.unicode - 49) as usize)),
        (_, 84 | 116) => Shortcut::ToggleTheme,
        (_, 82 | 114) => Shortcut::Command(BootCommand::Reset),
        _ => Shortcut::Pass,
    }
}

fn logical_key(key: UefiKey) -> Option<LogicalKey> {
    let named = match (key.scan_code, key.unicode) {
        (1, _) => Some(NamedKey::ArrowUp),
        (2, _) => Some(NamedKey::ArrowDown),
        (3, _) => Some(NamedKey::ArrowRight),
        (4, _) => Some(NamedKey::ArrowLeft),
        (23, _) | (_, 27) => Some(NamedKey::Escape),
        (_, 13) => Some(NamedKey::Enter),
        (_, 9) => Some(NamedKey::Tab),
        (_, 8) => Some(NamedKey::Backspace),
        _ => None,
    };
    named.map(LogicalKey::Named).or_else(|| {
        char::from_u32(key.unicode.into())
            .filter(|value| !value.is_control())
            .and_then(|value| LogicalKey::character(value.to_string()).ok())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holding_startup_escape_does_not_immediately_exit_the_selector() {
        let escape = UefiKey {
            scan_code: 23,
            unicode: 0,
        };
        let mut keys = StartupEscape::default();
        assert!(!keys.consume(escape, Duration::ZERO));
        keys.latch(Duration::ZERO);
        assert!(keys.consume(escape, Duration::from_millis(600)));
        keys.quiet(Duration::from_millis(1200));
        assert!(keys.consume(escape, Duration::from_millis(1300)));
        keys.quiet(Duration::from_millis(2300));
        assert!(!keys.consume(escape, Duration::from_millis(2300)));
    }

    #[test]
    fn startup_escape_filter_leaves_other_keys_available() {
        let mut keys = StartupEscape::default();
        keys.latch(Duration::ZERO);
        assert!(!keys.consume(
            UefiKey {
                scan_code: 0,
                unicode: 13
            },
            Duration::ZERO
        ));
        assert!(!keys.consume(
            UefiKey {
                scan_code: 3,
                unicode: 0
            },
            Duration::ZERO
        ));
    }

    #[test]
    fn firmware_modes_use_uniform_scale_and_respect_allocation_limits() {
        let mode = metrics((800, 600), 1_000_000).unwrap();
        assert_eq!(mode.logical_extent.width, 900.0);
        assert_eq!(mode.logical_extent.height, 675.0);
        assert_eq!(mode.scale_factor, 800.0 / 900.0);
        assert!(metrics((0, 600), 1_000_000).is_err());
        assert!(metrics((1920, 1080), 1_000_000).is_err());
    }

    #[test]
    fn firmware_keyboard_activates_highlighted_target_without_widget_focus() {
        assert!(matches!(
            shortcut(
                UefiKey {
                    scan_code: 3,
                    unicode: 0
                },
                BootPhase::Selecting
            ),
            Shortcut::Command(BootCommand::MoveSelection(1))
        ));
        assert!(matches!(
            shortcut(
                UefiKey {
                    scan_code: 0,
                    unicode: 13
                },
                BootPhase::Failed
            ),
            Shortcut::Command(BootCommand::Retry)
        ));
        assert!(matches!(
            shortcut(
                UefiKey {
                    scan_code: 23,
                    unicode: 0
                },
                BootPhase::Selecting
            ),
            Shortcut::Return
        ));
        assert!(matches!(
            shortcut(
                UefiKey {
                    scan_code: 23,
                    unicode: 0
                },
                BootPhase::Failed
            ),
            Shortcut::Command(BootCommand::Reset)
        ));
    }
}

//! Shell-authored screen-cast portal surfaces; construction starts no services.
use super::{AppError, AppResult, declaration::RegisteredShellWidget};
use crate::{ScreenCastPortalContext, ShellWidget};

type SurfaceFactory = Box<dyn FnOnce(ScreenCastPortalContext) -> RegisteredShellWidget>;

/// Designs the screen-cast portal using shell-owned components.
///
/// The picker receives pending requests and eligible sources. The sharing component receives
/// active streams, startup state and errors, and provides the shell's stop controls. Both factories
/// receive the same live context and run once on the compositor owner thread. Components own their
/// appearance, placement and visibility; the host validates every consent and stop intention.
#[derive(Default)]
pub struct ScreenCastPortal {
    picker: Option<SurfaceFactory>,
    sharing: Option<SurfaceFactory>,
    window: Option<PortalPickerWindow>,
    audio: Option<super::ShareAudio>,
}
impl ScreenCastPortal {
    pub const fn new() -> Self {
        Self {
            picker: None,
            sharing: None,
            window: None,
            audio: None,
        }
    }

    pub fn audio(mut self, policy: super::ShareAudio) -> Self { self.audio = Some(policy); self }

    /// Defines the source-selection and consent surface. No built-in picker is installed.
    pub fn picker<W: ShellWidget>(
        mut self,
        factory: impl FnOnce(ScreenCastPortalContext) -> W + 'static,
    ) -> Self {
        self.window = None;
        self.picker = Some(Box::new(move |context| {
            RegisteredShellWidget::new(factory(context))
        }));
        self
    }

    /// Runs the shell's picker executable as an ordinary Wayland client for each request.
    /// The helper obtains its live context through `ScreenCastPortalContext::from_picker_stdio`.
    pub fn picker_window(mut self, window: PortalPickerWindow) -> Self {
        self.picker = None;
        self.window = Some(window);
        self
    }

    pub fn picker_app(self, application: impl Into<crate::ApplicationRef>) -> Self {
        let mut window = PortalPickerWindow::new("");
        window.application = Some(application.into());
        self.picker_window(window)
    }

    /// Defines active-sharing controls and failure presentation. Required alongside the picker.
    /// The context can also be cloned into shell-owned saved-permission settings.
    pub fn sharing<W: ShellWidget>(
        mut self,
        factory: impl FnOnce(ScreenCastPortalContext) -> W + 'static,
    ) -> Self {
        self.sharing = Some(Box::new(move |context| {
            RegisteredShellWidget::new(factory(context))
        }));
        self
    }

    pub(crate) fn validate(&self) -> AppResult<()> {
        if self.picker.is_none() && self.window.is_none() {
            return Err(AppError::new(
                "ScreenCastPortal requires a shell-defined picker",
            ));
        }
        if self.sharing.is_none() {
            return Err(AppError::new(
                "ScreenCastPortal requires shell-defined sharing controls",
            ));
        }
        Ok(())
    }

    pub(crate) fn compose(self, context: ScreenCastPortalContext) -> AppResult<PortalSurfaces> {
        self.validate()?;
        let mut widgets = Vec::new();
        if let Some(picker) = self.picker {
            widgets.push(picker(context.clone()));
        }
        widgets.push(self.sharing.unwrap()(context));
        Ok(PortalSurfaces {
            widgets,
            window: self.window,
            audio: self.audio,
        })
    }
}

/// A shell-owned picker executable. Only its stdin/stdout carry private portal messages;
/// stderr remains available for diagnostics. The host supplies its own Wayland socket.
#[derive(Clone, Debug)]
pub struct PortalPickerWindow {
    pub(crate) application: Option<crate::ApplicationRef>,
    pub(crate) program: std::path::PathBuf,
    pub(crate) args: Vec<std::ffi::OsString>,
}
impl PortalPickerWindow {
    pub fn new(program: impl Into<std::path::PathBuf>) -> Self {
        Self {
            program: program.into(),
            application: None,
            args: Vec::new(),
        }
    }
    pub fn arg(mut self, argument: impl Into<std::ffi::OsString>) -> Self {
        self.args.push(argument.into());
        self
    }
}
pub(crate) struct PortalSurfaces {
    pub widgets: Vec<RegisteredShellWidget>,
    pub window: Option<PortalPickerWindow>,
    pub audio: Option<super::ShareAudio>,
}

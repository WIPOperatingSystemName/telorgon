//! Pure startup declarations for GUI applications and Linux desktop environments.

use std::fmt;
use std::path::PathBuf;

use crate::assets::{
    AppIconProfile, AssetBundle, ClientCursorMode, CursorTheme, CursorThemeAsset,
    PointerConfiguration, PointerThemeOverrides,
};
use crate::authoring::compose::{Component, ErasedComponent, RuntimeTarget};
use crate::foundation::{ColorRgba8, SizeI};
use crate::runtime::CompositionDriver;
use crate::shell::window_chrome::{ShellActionId, WindowChromeModel, WindowContentStyle};

use crate::host::application::{
    AppError, AppResult, KeyBindings, WindowDecorationMode, WindowOptions,
};

/// Renderer policy selected by an application declaration.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Renderer {
    /// Uses the entrypoint's platform default renderer policy.
    #[default]
    Auto,
    /// Requires the Vulkan renderer.
    Vulkan,
    /// Requires the deterministic software renderer.
    Software,
}

/// XKB rule names for the managed desktop seat. `None` retains libxkbcommon's
/// existing defaults; an explicit empty options string disables default options.
/// Comma-separated layouts, variants and options are passed through as XKB names.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyboardConfig {
    /// Exclusive absolute XKB data root; None keeps the existing system search path.
    pub include_root: Option<PathBuf>,
    pub rules: Option<String>,
    pub model: Option<String>,
    pub layout: Option<String>,
    pub variant: Option<String>,
    pub options: Option<String>,
}

impl KeyboardConfig {
    fn validate(&self) -> AppResult<()> {
        if self
            .include_root
            .as_ref()
            .is_some_and(|root| !root.is_absolute())
        {
            return Err(AppError::new("XKB include root must be absolute"));
        }
        if [
            &self.rules,
            &self.model,
            &self.layout,
            &self.variant,
            &self.options,
        ]
        .into_iter()
        .flatten()
        .any(|name| name.contains('\0'))
        {
            return Err(AppError::new("XKB name contains an interior NUL"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LinuxShellConfig {
    pub typography: crate::Typography,
    /// Fallback for templates without an explicit motion style.
    pub window_motion: crate::WindowMotion,
    /// Applies centrally to all desktop window motion.
    pub motion_preference: crate::theme::MotionPreference,
    /// Enable the embedded compatibility helper. Requires an embedded-payload build.
    /// Shell X11 presentation remains under implementation.
    #[cfg(feature = "shell-xwayland")]
    pub xwayland_enabled: bool,
    /// Explicit private executable cache directory (created and validated securely).
    #[cfg(feature = "shell-xwayland")]
    pub xwayland_cache: Option<PathBuf>,
    /// None selects a seat-accessible KMS device with a connected output.
    pub drm_device: Option<PathBuf>,
    pub seat_name: String,
    /// Startup keymap published to clients of the managed desktop seat.
    pub keyboard: KeyboardConfig,
    pub socket_name: Option<String>,
    pub session: crate::services::session::SessionConfig,
    /// All compositor geometry uses logical units; this selects pixels per logical unit.
    pub output_scale: super::OutputScale,
    /// Optional live control of the host’s single physical output.
    pub display_control: Option<super::display_control::DisplayControl>,
    /// Window border thickness in logical units.
    pub window_border: i32,
    /// Allowed horizontal overflow during window dragging, in logical units.
    /// `Some(0)` (default) keeps the frame inside the output; `None` is unrestricted.
    /// Oversized windows may slide between their left- and right-aligned positions.
    pub window_drag_horizontal_overflow: Option<i32>,
    /// Preferred ordinary-window content minimum in logical units, excluding borders and title bar.
    /// Applies to both Wayland and X11 windows independently of output scale. Defaults to 300 × 200;
    /// both dimensions must be positive. Client constraints (including fixed sizes and resize
    /// increments) and available work area can override this preference.
    pub preferred_window_minimum: SizeI,
    /// Title-bar height in logical units.
    pub titlebar_height: i32,
    /// Fallback whole-window placeholder during resize and while awaiting the final client image.
    /// Only the preview geometry changes during the drag; the client receives its final size on
    /// release. Color alpha reveals lower desktop layers; glass alpha controls tint strength.
    /// Frame templates can override this through [`WindowFrameTemplate::content_style`].
    pub resize_preview: crate::ResizePreviewDesign,
    /// Default pointer size in logical units.
    pub pointer_extent: SizeI,
}

impl Default for LinuxShellConfig {
    fn default() -> Self {
        Self {
            typography: Default::default(),
            window_motion: crate::WindowMotion::none(),
            motion_preference: crate::theme::MotionPreference::Full,
            #[cfg(feature = "shell-xwayland")]
            xwayland_enabled: cfg!(feature = "shell-xwayland-embedded"),
            #[cfg(feature = "shell-xwayland")]
            xwayland_cache: None,
            drm_device: None,
            seat_name: "seat0".to_owned(),
            keyboard: KeyboardConfig::default(),
            socket_name: None,
            session: crate::services::session::SessionConfig::default(),
            output_scale: super::OutputScale::Auto,
            display_control: None,
            window_border: 4,
            window_drag_horizontal_overflow: Some(0),
            preferred_window_minimum: SizeI {
                width: 300,
                height: 200,
            },
            titlebar_height: 32,
            resize_preview: crate::ResizePreviewDesign::new(crate::Fill::Color(ColorRgba8::rgba(
                38, 42, 48, 255,
            ))),
            pointer_extent: SizeI {
                width: 32,
                height: 32,
            },
        }
    }
}

impl LinuxShellConfig {
    fn validate(&self) -> AppResult<()> {
        if !self.resize_preview.corner_radius.is_finite() || self.resize_preview.corner_radius < 0.0 {
            return Err(AppError::new("resize preview radius must be finite and nonnegative"));
        }
        if !self.resize_preview.border_is_valid() {
            return Err(AppError::new(
                "resize preview border widths must be finite and nonnegative",
            ));
        }
        self.output_scale.validate()?;
        self.keyboard.validate()?;
        self.session
            .validate()
            .map_err(|e| AppError::new(e.to_string()))?;
        if self
            .drm_device
            .as_ref()
            .is_some_and(|path| !path.is_absolute())
            || self.seat_name.trim().is_empty()
            || self.socket_name.as_ref().is_some_and(|name| {
                name.trim().is_empty() || name.contains(['/', '\\']) || name == "." || name == ".."
            })
            || self.preferred_window_minimum.width <= 0
            || self.preferred_window_minimum.height <= 0
            || self
                .window_drag_horizontal_overflow
                .is_some_and(|value| value < 0)
            || self.window_border < 0
            || self.titlebar_height < 0
            || self.pointer_extent.width <= 0
            || self.pointer_extent.height <= 0
        {
            Err(AppError::new("invalid Linux desktop configuration"))
        } else {
            Ok(())
        }
    }
}

/// Namespace for Telorgon's two application constructors.
pub struct Application {
    _private: (),
}

impl Application {
    /// Begins a GUI declaration with a stable storage identity and a separate display name.
    /// Identity is validated at startup and must remain stable across app renames and updates.
    pub fn gui(identity: impl Into<String>, name: impl Into<String>) -> GuiApplication {
        GuiApplication {
            typography: Default::default(),
            identity: identity.into(),
            name: name.into(),
            renderer: Renderer::Auto,
            assets: AssetBundle::EMPTY,
            pointer: PointerConfiguration::default(),
            session: crate::services::session::GuiSessionConfig::default(),
        }
    }

    /// Begins one Linux shell-environment declaration.
    pub fn shell_environment(name: impl Into<String>) -> ShellEnvironment {
        ShellEnvironment::new(name.into())
    }
}

impl fmt::Debug for Application {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Application")
    }
}

/// Incomplete GUI application declaration that still requires its initial window.
pub struct GuiApplication {
    typography: crate::Typography,
    identity: String,
    name: String,
    renderer: Renderer,
    assets: AssetBundle,
    pointer: PointerConfiguration,
    session: crate::services::session::GuiSessionConfig,
}

impl GuiApplication {
    pub fn typography(mut self, typography: crate::Typography) -> Self {
        self.typography = typography;
        self
    }
    /// Configure launch services without changing the application identity.
    pub fn session(mut self, config: crate::services::session::GuiSessionConfig) -> Self {
        self.session = config;
        self
    }
    /// Selects the renderer policy for this application.
    pub fn renderer(mut self, renderer: Renderer) -> Self {
        self.renderer = renderer;
        self
    }

    /// Registers the project's generated media catalog for every managed GUI subsystem.
    pub fn assets(mut self, assets: AssetBundle) -> Self {
        self.assets = assets;
        self
    }

    pub fn cursor_theme(mut self, theme: CursorThemeAsset) -> Self {
        self.pointer = self.pointer.cursor_theme(theme);
        self
    }

    pub fn pointer_overrides(mut self, overrides: PointerThemeOverrides) -> Self {
        self.pointer = self.pointer.overrides(overrides);
        self
    }

    /// Installs the single initial window supported by the current managed runtime.
    pub fn window(self, window: ReadyWindow) -> ReadyGuiApplication {
        ReadyGuiApplication {
            typography: self.typography,
            identity: self.identity,
            name: self.name,
            renderer: self.renderer,
            assets: self.assets,
            pointer: self.pointer,
            window,
            session: self.session,
        }
    }
}

impl fmt::Debug for GuiApplication {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuiApplication")
            .field("name", &self.name)
            .field("renderer", &self.renderer)
            .field("assets", &self.assets.len())
            .field("has_window", &false)
            .finish()
    }
}

/// Complete GUI application declaration.
pub struct ReadyGuiApplication {
    typography: crate::Typography,
    identity: String,
    name: String,
    renderer: Renderer,
    assets: AssetBundle,
    pointer: PointerConfiguration,
    window: ReadyWindow,
    session: crate::services::session::GuiSessionConfig,
}

impl ReadyGuiApplication {
    pub fn typography(mut self, typography: crate::Typography) -> Self {
        self.typography = typography;
        self
    }
    pub fn session(mut self, config: crate::services::session::GuiSessionConfig) -> Self {
        self.session = config;
        self
    }
    /// Replaces the renderer policy without changing the declared window.
    pub fn renderer(mut self, renderer: Renderer) -> Self {
        self.renderer = renderer;
        self
    }

    pub fn assets(mut self, assets: AssetBundle) -> Self {
        self.assets = assets;
        self
    }

    pub fn cursor_theme(mut self, theme: CursorThemeAsset) -> Self {
        self.pointer = self.pointer.cursor_theme(theme);
        self
    }

    pub fn pointer_overrides(mut self, overrides: PointerThemeOverrides) -> Self {
        self.pointer = self.pointer.overrides(overrides);
        self
    }

    /// Runs the managed GUI application.
    pub fn run(self) -> AppResult<()> {
        #[cfg(any(
            feature = "application-software",
            all(feature = "application-vulkan", any(target_os = "windows", target_os = "linux"))
        ))]
        {
            validate_application_name(&self.name)?;
            crate::services::session::validate_identity(&self.identity)
                .map_err(|e| AppError::new(e.to_string()))?;
            let config = self.session.clone().into_session(self.identity.clone());
            let env =
                crate::services::session::Environment::gui().map_err(|e| AppError::new(e.to_string()))?;
            let session = crate::services::session::SessionOwner::start_gui(env, config)
                .map_err(|e| AppError::new(e.to_string()))?;
            let result = crate::host::application::native::run_gui(self);
            if result.is_ok() {
                session.close();
            } else {
                session.abort();
            }
            return result;
        }

        #[cfg(not(any(
            feature = "application-software",
            all(feature = "application-vulkan", any(target_os = "windows", target_os = "linux"))
        )))]
        {
            let _ = self.into_parts()?;
            Err(AppError::new(
                "no managed GUI runtime is enabled in this build",
            ))
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> AppResult<(
        CompositionDriver,
        WindowOptions,
        Renderer,
        AssetBundle,
        PointerConfiguration,
    )> {
        validate_application_name(&self.name)?;
        crate::services::session::validate_identity(&self.identity)
            .map_err(|e| AppError::new(e.to_string()))?;
        crate::AssetResolver::new(self.assets)
            .map_err(|error| AppError::new(error.to_string()))?;
        self.pointer
            .load_theme(self.assets)
            .map_err(|error| AppError::new(error.to_string()))?;
        let (mut driver, options) = self.window.into_parts()?;
        driver.typography = self.typography;
        Ok((driver, options, self.renderer, self.assets, self.pointer))
    }
}

impl fmt::Debug for ReadyGuiApplication {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuiApplication")
            .field("name", &self.name)
            .field("renderer", &self.renderer)
            .field("assets", &self.assets.len())
            .field("window", &self.window)
            .finish()
    }
}

/// Incomplete initial managed application window.
pub struct Window {
    options: WindowOptions,
}

impl Window {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            options: WindowOptions {
                title: title.into(),
                ..WindowOptions::default()
            },
        }
    }

    pub fn size(mut self, width: i32, height: i32) -> Self {
        self.options.size = SizeI { width, height };
        self
    }

    pub fn minimum_size(mut self, width: i32, height: i32) -> Self {
        self.options.min_size = Some(SizeI { width, height });
        self
    }

    pub fn without_minimum_size(mut self) -> Self {
        self.options.min_size = None;
        self
    }

    /// Selects whether the native platform draws its standard non-client frame.
    pub fn decorations(mut self, decorations: WindowDecorationMode) -> Self {
        self.options.decorations = decorations;
        self
    }

    /// Uses identical minimum and maximum extents and disables interactive resizing.
    pub fn fixed_size(mut self, width: i32, height: i32) -> Self {
        self.options.size = SizeI { width, height };
        self.options.min_size = Some(self.options.size);
        self.options.fixed_size = true;
        self
    }

    pub fn without_system_frame(self) -> Self {
        self.decorations(WindowDecorationMode::Hidden)
    }

    /// Uses a composition root built with [`crate::window_frame`] as client-side chrome.
    pub fn custom_frame(self) -> Self {
        self.without_system_frame()
    }

    /// Assigns the icon profile shared by native window metadata and composed client chrome.
    pub fn icon(mut self, profile: AppIconProfile) -> Self {
        self.options.icon = profile;
        self
    }

    /// Completes this window with its composition root.
    pub fn content<C: Component>(self, component: C) -> ReadyWindow {
        ReadyWindow {
            options: self.options,
            content: CompositionDriver::new(component),
        }
    }
}

impl fmt::Debug for Window {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Window")
            .field("options", &self.options)
            .field("has_content", &false)
            .finish()
    }
}

/// Complete initial managed application window.
pub struct ReadyWindow {
    options: WindowOptions,
    content: CompositionDriver,
}

impl ReadyWindow {
    fn into_parts(self) -> AppResult<(CompositionDriver, WindowOptions)> {
        if self.options.title.trim().is_empty() {
            return Err(AppError::new("Window title must not be empty"));
        }
        if self.options.size.width <= 0 || self.options.size.height <= 0 {
            return Err(AppError::new("Window size must be positive"));
        }
        self.options
            .icon
            .validate()
            .map_err(|error| AppError::new(error.to_string()))?;
        Ok((self.content, self.options))
    }
}

impl fmt::Debug for ReadyWindow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Window")
            .field("options", &self.options)
            .field("has_content", &true)
            .finish()
    }
}

/// Private erased storage for a component-owned shell surface.
pub(crate) struct RegisteredShellWidget {
    pub(crate) content: CompositionDriver,
    pub(crate) surface: crate::authoring::compose::shell_widget::SurfaceBinding,
}
impl RegisteredShellWidget {
    pub(crate) fn new<W: crate::authoring::compose::ShellWidget>(widget: W) -> Self {
        let (root, surface) = crate::authoring::compose::shell_widget::erase(widget);
        Self {
            content: CompositionDriver::from_erased_for_target(root, RuntimeTarget::ShellWidget),
            surface,
        }
    }
}
impl fmt::Debug for RegisteredShellWidget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShellWidget").finish_non_exhaustive()
    }
}

/// A named Telorgon composition used by the compositor for shell-owned pixels.
pub struct CompositorVisual {
    name: String,
    content: CompositionDriver,
}

/// One frame action authorized by the compositor declaration.
pub(crate) struct ShellActionHandler {
    id: ShellActionId,
    #[cfg_attr(
        not(all(feature = "shell-wayland-linux", target_os = "linux")),
        allow(dead_code)
    )]
    invoke: Box<dyn Fn(WindowChromeModel)>,
}

impl ShellActionHandler {
    pub(crate) const fn id(&self) -> ShellActionId {
        self.id
    }

    #[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
    pub(crate) fn invoke(&self, model: WindowChromeModel) {
        (self.invoke)(model);
    }
}

impl fmt::Debug for ShellActionHandler {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ShellActionHandler")
            .field("id", &self.id)
            .finish()
    }
}

/// Creates a fresh compositor-owned frame composition for one Wayland toplevel.
///
/// Implementations are reusable templates. The host supplies one immutable
/// [`WindowChromeModel`] for each frame instance, while the template owns any shared visual
/// configuration needed to construct that instance.
pub trait WindowFrameTemplate: 'static {
    /// None inherits desktop motion; Some(none()) explicitly disables it.
    fn motion(&self, _model: &WindowChromeModel) -> Option<crate::WindowMotion> {
        None
    }
    type Component: Component;

    fn compose(&self, model: WindowChromeModel) -> Self::Component;

    /// Opts into a separate client backing, allowing client alpha to reveal the desktop when
    /// its background is transparent. `None` preserves a custom template's composed backing.
    /// This only affects externally supplied compositor surfaces, not managed GUI children.
    fn content_style(&self, _model: &WindowChromeModel) -> Option<WindowContentStyle> {
        None
    }
}

impl<F, C> WindowFrameTemplate for F
where
    F: Fn(WindowChromeModel) -> C + 'static,
    C: Component,
{
    type Component = C;

    fn compose(&self, model: WindowChromeModel) -> Self::Component {
        self(model)
    }
}

/// Type-erased storage for one reusable [`WindowFrameTemplate`].
///
/// The model is the only shell-owned input. Everything visual and interactive is authored with
/// normal composition primitives plus the explicit frame/content/action roles. Templates may
/// additionally describe the independent backing/preview for externally supplied client pixels.
pub struct WindowFrameFactory {
    #[cfg_attr(
        not(all(feature = "shell-wayland-linux", target_os = "linux")),
        allow(dead_code)
    )]
    motion: Box<dyn Fn(&WindowChromeModel) -> Option<crate::WindowMotion>>,
    #[cfg_attr(
        not(all(feature = "shell-wayland-linux", target_os = "linux")),
        allow(dead_code)
    )]
    compose: Box<dyn Fn(WindowChromeModel) -> Box<dyn ErasedComponent>>,
    #[cfg_attr(
        not(all(feature = "shell-wayland-linux", target_os = "linux")),
        allow(dead_code)
    )]
    content_style: Box<dyn Fn(&WindowChromeModel) -> Option<WindowContentStyle>>,
}

impl WindowFrameFactory {
    #[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
    pub(crate) fn motion(&self, model: &WindowChromeModel) -> Option<crate::WindowMotion> {
        (self.motion)(model)
    }
    fn new<T>(template: T) -> Self
    where
        T: WindowFrameTemplate,
    {
        let template = std::rc::Rc::new(template);
        let style_template = std::rc::Rc::clone(&template);
        let motion_template = std::rc::Rc::clone(&template);
        Self {
            compose: Box::new(move |model| Box::new(template.compose(model))),
            motion: Box::new(move |model| motion_template.motion(model)),
            content_style: Box::new(move |model| style_template.content_style(model)),
        }
    }

    #[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
    pub(crate) fn content_style(&self, model: &WindowChromeModel) -> Option<WindowContentStyle> {
        (self.content_style)(model)
    }

    #[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
    pub(crate) fn compose(&self, model: WindowChromeModel) -> CompositionDriver {
        CompositionDriver::from_erased_for_target(self.candidate(model), RuntimeTarget::Compositor)
    }

    #[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
    pub(crate) fn candidate(&self, model: WindowChromeModel) -> Box<dyn ErasedComponent> {
        (self.compose)(model)
    }
}

impl fmt::Debug for WindowFrameFactory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("WindowFrameFactory")
    }
}

impl CompositorVisual {
    fn new(name: impl Into<String>, component: impl Component) -> Self {
        Self {
            name: name.into(),
            content: CompositionDriver::for_target(component, RuntimeTarget::Compositor),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

impl fmt::Debug for CompositorVisual {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CompositorVisual")
            .field("name", &self.name)
            .field("has_content", &true)
            .finish()
    }
}

/// One fresh Linux desktop key press, resolved through the active XKB state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShellKeyEvent {
    /// Linux evdev keycode (without the XKB offset).
    pub keycode: u32,
    pub keysym: u32,
    pub control: bool,
    pub shift: bool,
    pub alt: bool,
    pub logo: bool,
}

/// Disposition of a compositor shortcut's initiating key press.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShellKeyAction {
    #[default]
    Forward,
    /// Consume this key's press, repeats, and release before client delivery.
    Consume,
    /// Revoke pointer capture and shortcut inhibition without changing focus or lock.
    ReleaseCapture,
    /// Return normally from the desktop host, releasing its owned resources.
    Quit,
}

pub(crate) type ShellKeyHandler = Box<dyn FnMut(ShellKeyEvent) -> ShellKeyAction>;

/// Incomplete compositor declaration.
pub struct MissingCursorTheme;

/// Compositor declaration; cursor_theme is required before desktop admission.
///
/// ```compile_fail
/// use telorgon::app::*;
/// #[component]
/// struct Background {}
/// impl Component for Background {
///     fn view(&self) -> impl View { text("desktop") }
/// }
/// let compositor = Compositor::new().cursor_theme(crate::CursorTheme::new());
/// Application::shell_environment("desktop").compositor(compositor);
/// ```
pub struct Compositor<C = MissingCursorTheme> {
    cursor_theme: C,
    #[cfg(all(feature = "shell-screencast-linux", target_os = "linux"))]
    screen_cast_portal: Option<super::ScreenCastPortal>,
    client_cursor_mode: ClientCursorMode,
    decoration_policy: crate::DecorationPolicy,
    window_frame: Option<WindowFrameFactory>,
    icons: Vec<CompositorVisual>,
    shell_actions: Vec<ShellActionHandler>,
    keyboard_shortcut_handler: Option<ShellKeyHandler>,
}

impl Default for Compositor {
    fn default() -> Self {
        Self::new()
    }
}

impl Compositor {
    pub const fn new() -> Self {
        Self {
            cursor_theme: MissingCursorTheme,
            #[cfg(all(feature = "shell-screencast-linux", target_os = "linux"))]
            screen_cast_portal: None,
            client_cursor_mode: ClientCursorMode::Allow,
            decoration_policy: crate::DecorationPolicy::DEFAULT,
            window_frame: None,
            icons: Vec::new(),
            shell_actions: Vec::new(),
            keyboard_shortcut_handler: None,
        }
    }
}

impl<C> Compositor<C> {
    /// Installs a shell-defined screen-cast portal design. Enable delivery with `Capture::desktop()`.
    #[cfg(all(feature = "shell-screencast-linux", target_os = "linux"))]
    pub fn screen_cast_portal(mut self, portal: super::ScreenCastPortal) -> Self {
        self.screen_cast_portal = Some(portal);
        self
    }

    #[cfg(all(feature = "shell-screencast-linux", target_os = "linux"))]
    pub(crate) fn take_screen_cast_portal(&mut self) -> Option<super::ScreenCastPortal> {
        self.screen_cast_portal.take()
    }

    /// Selects Wayland decoration negotiation. Only windows with committed server
    /// decorations receive the frame template; X11 windows follow their decoration hints.
    pub fn decoration_policy(mut self, policy: crate::DecorationPolicy) -> Self {
        self.decoration_policy = policy;
        self
    }

    pub fn configured_decoration_policy(&self) -> crate::DecorationPolicy {
        self.decoration_policy
    }

    /// Supplies the composed server-side frame for each window and each relevant model change.
    pub fn window_frame<T>(mut self, template: T) -> Self
    where
        T: WindowFrameTemplate,
    {
        self.window_frame = Some(WindowFrameFactory::new(template));
        self
    }

    /// Adds a semantic shell icon. Names are stable compositor keys such as `window.close`.
    pub fn icon<B: Component>(mut self, name: impl Into<String>, component: B) -> Self {
        self.icons.push(CompositorVisual::new(name, component));
        self
    }

    /// Authorizes one custom frame action and supplies its host-side handler.
    pub fn shell_action<F>(mut self, id: ShellActionId, handler: F) -> Self
    where
        F: Fn(WindowChromeModel) + 'static,
    {
        self.shell_actions.push(ShellActionHandler {
            id,
            invoke: Box::new(handler),
        });
        self
    }

    /// Installs named-function shortcuts. Replaces any previous bindings or raw key handler.
    ///
    /// Matched keys are consumed through release; unmatched keys forward normally.
    /// See [`KeyBindings`] for a complete example.
    pub fn keybindings(mut self, bindings: KeyBindings) -> Self {
        self.keyboard_shortcut_handler = Some(Box::new(move |event| bindings.handle(event)));
        self
    }

    /// Handles fresh key presses before client delivery, even with no focused window.
    /// Replaces any previous bindings or raw key handler.
    /// The host suppresses repeats/releases for consumed keys and disables this handler
    /// while a session lock is active. Keep the callback short and nonblocking.
    pub fn keyboard_shortcut_handler(
        mut self,
        handler: impl FnMut(ShellKeyEvent) -> ShellKeyAction + 'static,
    ) -> Self {
        self.keyboard_shortcut_handler = Some(Box::new(handler));
        self
    }
}

impl<C> fmt::Debug for Compositor<C> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Compositor")
            .field("has_window_frame", &self.window_frame.is_some())
            .field("icons", &self.icons.len())
            .field("shell_actions", &self.shell_actions.len())
            .field(
                "has_keyboard_shortcut_handler",
                &self.keyboard_shortcut_handler.is_some(),
            )
            .finish()
    }
}

/// A compositor with its required cursor theme configured.
pub type ReadyCompositor<C = CursorTheme> = Compositor<C>;

#[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
type CompositorRuntimeParts = (
    Option<WindowFrameFactory>,
    Option<CompositionDriver>,
    Vec<(String, CompositionDriver)>,
    Vec<ShellActionHandler>,
    Option<ShellKeyHandler>,
);

impl Compositor<CursorTheme> {
    fn prepare_cursors(&self, assets: AssetBundle) -> AppResult<PointerConfiguration> {
        self.cursor_theme
            .prepare(assets, self.client_cursor_mode)
            .map_err(|error| AppError::new(error.to_string()))
    }

    fn validate(&self) -> AppResult<()> {
        for visual in &self.icons {
            debug_assert_eq!(visual.content.target(), RuntimeTarget::Compositor);
        }
        if self.icons.iter().any(|icon| icon.name.trim().is_empty()) {
            return Err(AppError::new("Compositor icon names must not be empty"));
        }
        let mut names = std::collections::HashSet::new();
        if self
            .icons
            .iter()
            .any(|icon| !names.insert(icon.name.as_str()))
        {
            return Err(AppError::new("Compositor icon names must be unique"));
        }
        let mut actions = std::collections::HashSet::new();
        if self
            .shell_actions
            .iter()
            .any(|handler| !actions.insert(handler.id()))
        {
            return Err(AppError::new("Compositor shell action IDs must be unique"));
        }
        Ok(())
    }

    pub fn frame_template(&self) -> Option<&WindowFrameFactory> {
        self.window_frame.as_ref()
    }

    pub fn icons(&self) -> &[CompositorVisual] {
        &self.icons
    }

    pub fn authorizes_shell_action(&self, id: ShellActionId) -> bool {
        self.shell_actions.iter().any(|handler| handler.id == id)
    }

    #[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
    pub(crate) fn into_runtime_parts(self) -> CompositorRuntimeParts {
        (
            self.window_frame,
            None,
            self.icons
                .into_iter()
                .map(|visual| (visual.name, visual.content))
                .collect(),
            self.shell_actions,
            self.keyboard_shortcut_handler,
        )
    }
}

mod shell_environment;
pub use shell_environment::{ReadyShellEnvironment, ShellEnvironment, ShellEnvironmentWithCompositor};

fn validate_application_name(name: &str) -> AppResult<()> {
    if name.trim().is_empty() {
        Err(AppError::new("Application name must not be empty"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod keyboard_configuration_tests;

impl<C> Compositor<C> {
    /// Supplies the required cursor design; validated automatically by desktop startup.
    pub fn cursor_theme(self, theme: CursorTheme) -> Compositor<CursorTheme> {
        Compositor {
            cursor_theme: theme,
            client_cursor_mode: self.client_cursor_mode,
            decoration_policy: self.decoration_policy,
            window_frame: self.window_frame,
            icons: self.icons,
            shell_actions: self.shell_actions,
            keyboard_shortcut_handler: self.keyboard_shortcut_handler,
            #[cfg(all(feature = "shell-screencast-linux", target_os = "linux"))]
            screen_cast_portal: self.screen_cast_portal,
        }
    }

    pub fn client_cursor_mode(mut self, mode: ClientCursorMode) -> Self {
        self.client_cursor_mode = mode;
        self
    }
}

impl Compositor<CursorTheme> {
    /// Sets the effective logical cursor size, preserving the theme's proportions.
    pub fn cursor_size(mut self, size: f32) -> Self {
        self.cursor_theme = self.cursor_theme.cursor_size(size);
        self
    }
}

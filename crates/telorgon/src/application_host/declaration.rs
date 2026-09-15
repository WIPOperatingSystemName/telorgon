//! Pure startup declarations for GUI applications and Linux desktop environments.

use std::fmt;
use std::path::PathBuf;

use crate::assets::{
    AppIconProfile, AssetBundle, ClientCursorMode, CursorTheme, CursorThemeAsset,
    PointerConfiguration, PointerThemeOverrides,
};
use crate::compose::{Component, ErasedComponent, RuntimeTarget};
use crate::core::{ColorRgba8, SizeI};
use crate::runtime::CompositionDriver;
use crate::window_chrome::{ShellActionId, WindowChromeModel, WindowContentStyle};

use crate::application_host::{
    AppError, AppResult, KeyBindings, WindowDecorationMode, WindowOptions,
};

/// Renderer policy selected by an application declaration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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
    pub session: crate::session::SessionConfig,
    /// All compositor geometry uses logical units; this selects pixels per logical unit.
    pub output_scale: super::OutputScale,
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
    pub resize_preview: crate::ResizePreview,
    /// Default pointer size in logical units.
    pub pointer_extent: SizeI,
}

impl Default for LinuxShellConfig {
    fn default() -> Self {
        Self {
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
            session: crate::session::SessionConfig::default(),
            output_scale: super::OutputScale::Auto,
            window_border: 4,
            window_drag_horizontal_overflow: Some(0),
            preferred_window_minimum: SizeI {
                width: 300,
                height: 200,
            },
            titlebar_height: 32,
            resize_preview: crate::ResizePreview::Color(ColorRgba8::rgba(38, 42, 48, 255)),
            pointer_extent: SizeI {
                width: 32,
                height: 32,
            },
        }
    }
}

impl LinuxShellConfig {
    fn validate(&self) -> AppResult<()> {
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
    /// Begins one ordinary managed GUI application declaration.
    pub fn gui(name: impl Into<String>) -> GuiApplication {
        GuiApplication {
            name: name.into(),
            renderer: Renderer::Auto,
            assets: AssetBundle::EMPTY,
            pointer: PointerConfiguration::default(),
            session: None,
        }
    }

    /// Begins one Linux shell-environment declaration.
    pub fn shell_environment(name: impl Into<String>) -> ShellEnvironment {
        ShellEnvironment {
            services: Default::default(),
            name: name.into(),
            renderer: Renderer::Auto,
            linux: LinuxShellConfig::default(),
            assets: AssetBundle::EMPTY,
            app_icon: AppIconProfile::new(),
        }
    }
}

impl fmt::Debug for Application {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Application")
    }
}

/// Incomplete GUI application declaration that still requires its initial window.
pub struct GuiApplication {
    name: String,
    renderer: Renderer,
    assets: AssetBundle,
    pointer: PointerConfiguration,
    session: Option<crate::session::SessionConfig>,
}

impl GuiApplication {
    /// Override the launch context's stable identity, recovery, and terminal configuration.
    pub fn session(mut self, config: crate::session::SessionConfig) -> Self {
        self.session = Some(config);
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
    name: String,
    renderer: Renderer,
    assets: AssetBundle,
    pointer: PointerConfiguration,
    window: ReadyWindow,
    session: Option<crate::session::SessionConfig>,
}

impl ReadyGuiApplication {
    pub fn session(mut self, config: crate::session::SessionConfig) -> Self {
        self.session = Some(config);
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
            all(feature = "application-vulkan-windows", target_os = "windows")
        ))]
        {
            validate_application_name(&self.name)?;
            let config = self.session.clone().unwrap_or_else(|| {
                // Stable across compiler versions and processes; display names need not be valid IDs.
                let hash = self.name.bytes().fold(0xcbf29ce484222325u64, |hash, byte| {
                    (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
                });
                crate::session::SessionConfig::new(format!("gui-{hash:016x}"))
            });
            let env =
                crate::session::Environment::gui().map_err(|e| AppError::new(e.to_string()))?;
            if config.publish_user_service_environment {
                return Err(AppError::new(
                    "GUI applications cannot publish the shared desktop environment; configure publication on the DE entry point",
                ));
            }
            let session = crate::session::SessionOwner::start_gui(env, config)
                .map_err(|e| AppError::new(e.to_string()))?;
            let result = crate::application_host::native::run_gui(self);
            if result.is_ok() {
                session.close();
            } else {
                session.abort();
            }
            return result;
        }

        #[cfg(not(any(
            feature = "application-software",
            all(feature = "application-vulkan-windows", target_os = "windows")
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
        self.assets
            .validate()
            .map_err(|error| AppError::new(error.to_string()))?;
        self.pointer
            .load_theme(self.assets)
            .map_err(|error| AppError::new(error.to_string()))?;
        let (driver, options) = self.window.into_parts()?;
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
    pub(crate) surface: crate::compose::shell_widget::SurfaceBinding,
}
impl RegisteredShellWidget {
    fn new<W: crate::compose::ShellWidget>(widget: W) -> Self {
        let (root, surface) = crate::compose::shell_widget::erase(widget);
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
    client_cursor_mode: ClientCursorMode,
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
            client_cursor_mode: ClientCursorMode::Allow,
            window_frame: None,
            icons: Vec::new(),
            shell_actions: Vec::new(),
            keyboard_shortcut_handler: None,
        }
    }
}

impl<C> Compositor<C> {
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

/// Shell-environment declaration that still requires its compositor.
pub struct ShellEnvironment {
    services: crate::compose::shell_services::ShellServiceRegistry,
    name: String,
    renderer: Renderer,
    linux: LinuxShellConfig,
    assets: AssetBundle,
    app_icon: AppIconProfile,
}

impl ShellEnvironment {
    /// Configures the environment-owned installed application catalog.
    pub fn applications(mut self, catalog: crate::compose::ApplicationCatalog) -> Self {
        self.services.insert(catalog);
        self
    }
    /// Installs an owner-thread service available to shell widget components.
    pub fn service<T: 'static>(mut self, service: T) -> Self {
        self.services.insert(service);
        self
    }

    pub fn renderer(mut self, renderer: Renderer) -> Self {
        self.renderer = renderer;
        self
    }

    pub fn linux(mut self, config: LinuxShellConfig) -> Self {
        self.linux = config;
        self
    }

    /// Registers one catalog shared by shell composition and hosted Wayland clients.
    pub fn assets(mut self, assets: AssetBundle) -> Self {
        self.assets = assets;
        self
    }

    /// Sets the desktop environment's fallback icon profile for client toplevels.
    pub fn app_icon(mut self, profile: AppIconProfile) -> Self {
        self.app_icon = profile;
        self
    }

    pub fn compositor(self, compositor: ReadyCompositor) -> ShellEnvironmentWithCompositor {
        ShellEnvironmentWithCompositor {
            services: self.services,
            name: self.name,
            renderer: self.renderer,
            linux: self.linux,
            assets: self.assets,
            app_icon: self.app_icon,
            compositor,
        }
    }
}

impl fmt::Debug for ShellEnvironment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ShellEnvironment")
            .field("name", &self.name)
            .field("renderer", &self.renderer)
            .field("assets", &self.assets.len())
            .field("has_compositor", &false)
            .finish()
    }
}

/// Shell-environment declaration that still requires its first shell widget.
pub struct ShellEnvironmentWithCompositor {
    services: crate::compose::shell_services::ShellServiceRegistry,
    name: String,
    renderer: Renderer,
    linux: LinuxShellConfig,
    assets: AssetBundle,
    app_icon: AppIconProfile,
    compositor: ReadyCompositor,
}

impl ShellEnvironmentWithCompositor {
    /// Configures the environment-owned installed application catalog.
    pub fn applications(mut self, catalog: crate::compose::ApplicationCatalog) -> Self {
        self.services.insert(catalog);
        self
    }
    /// Installs an owner-thread service available to shell widget components.
    pub fn service<T: 'static>(mut self, service: T) -> Self {
        self.services.insert(service);
        self
    }

    pub fn renderer(mut self, renderer: Renderer) -> Self {
        self.renderer = renderer;
        self
    }

    pub fn linux(mut self, config: LinuxShellConfig) -> Self {
        self.linux = config;
        self
    }

    pub fn assets(mut self, assets: AssetBundle) -> Self {
        self.assets = assets;
        self
    }

    pub fn app_icon(mut self, profile: AppIconProfile) -> Self {
        self.app_icon = profile;
        self
    }

    pub fn widget<W: crate::compose::ShellWidget>(self, widget: W) -> ReadyShellEnvironment {
        self.into_ready().widget(widget)
    }

    /// Starts the shell with no widgets.
    pub fn run(self) -> AppResult<()> {
        self.into_ready().run()
    }

    fn into_ready(self) -> ReadyShellEnvironment {
        ReadyShellEnvironment {
            services: self.services,
            name: self.name,
            renderer: self.renderer,
            linux: self.linux,
            assets: self.assets,
            app_icon: self.app_icon,
            compositor: self.compositor,
            shell_widgets: Vec::new(),
        }
    }
}

impl fmt::Debug for ShellEnvironmentWithCompositor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ShellEnvironment")
            .field("name", &self.name)
            .field("renderer", &self.renderer)
            .field("assets", &self.assets.len())
            .field("has_compositor", &true)
            .field("shell_widgets", &0)
            .finish()
    }
}

/// Complete shell-environment declaration.
pub struct ReadyShellEnvironment {
    services: crate::compose::shell_services::ShellServiceRegistry,
    name: String,
    renderer: Renderer,
    linux: LinuxShellConfig,
    assets: AssetBundle,
    app_icon: AppIconProfile,
    compositor: ReadyCompositor,
    shell_widgets: Vec<RegisteredShellWidget>,
}

impl ReadyShellEnvironment {
    /// Configures the environment-owned installed application catalog.
    pub fn applications(mut self, catalog: crate::compose::ApplicationCatalog) -> Self {
        self.services.insert(catalog);
        self
    }
    /// Installs an owner-thread service available to shell widget components.
    pub fn service<T: 'static>(mut self, service: T) -> Self {
        self.services.insert(service);
        self
    }

    pub fn renderer(mut self, renderer: Renderer) -> Self {
        self.renderer = renderer;
        self
    }

    pub fn linux(mut self, config: LinuxShellConfig) -> Self {
        self.linux = config;
        self
    }

    pub fn assets(mut self, assets: AssetBundle) -> Self {
        self.assets = assets;
        self
    }

    pub fn app_icon(mut self, profile: AppIconProfile) -> Self {
        self.app_icon = profile;
        self
    }

    pub fn widget<W: crate::compose::ShellWidget>(mut self, widget: W) -> Self {
        self.shell_widgets.push(RegisteredShellWidget::new(widget));
        self
    }

    /// Validates this declaration and enters the Linux shell-environment runtime.
    pub fn run(self) -> AppResult<()> {
        #[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
        {
            crate::application_host::shell_wayland::run(self)
        }
        #[cfg(not(all(feature = "shell-wayland-linux", target_os = "linux")))]
        {
            let _ = self.into_parts()?;
            Err(AppError::new(
                "the Linux Wayland desktop runtime requires target Linux and feature shell-wayland-linux",
            ))
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> AppResult<(
        String,
        ReadyCompositor,
        Vec<RegisteredShellWidget>,
        crate::compose::shell_services::ShellServiceRegistry,
        Renderer,
        LinuxShellConfig,
        AssetBundle,
        PointerConfiguration,
        AppIconProfile,
    )> {
        validate_application_name(&self.name)?;
        self.assets
            .validate()
            .map_err(|error| AppError::new(error.to_string()))?;
        self.app_icon
            .validate()
            .map_err(|error| AppError::new(error.to_string()))?;
        if self.shell_widgets.len() > 256 {
            return Err(AppError::new("shell exceeds 256 root widgets"));
        }
        self.compositor.validate()?;
        let pointer = self.compositor.prepare_cursors(self.assets)?;
        self.linux.validate()?;
        Ok((
            self.name,
            self.compositor,
            self.shell_widgets,
            self.services,
            self.renderer,
            self.linux,
            self.assets,
            pointer,
            self.app_icon,
        ))
    }
}

impl fmt::Debug for ReadyShellEnvironment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ShellEnvironment")
            .field("name", &self.name)
            .field("renderer", &self.renderer)
            .field("assets", &self.assets.len())
            .field("has_compositor", &true)
            .field("shell_widgets", &self.shell_widgets)
            .finish()
    }
}

fn validate_application_name(name: &str) -> AppResult<()> {
    if name.trim().is_empty() {
        Err(AppError::new("Application name must not be empty"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compose::{ComponentFields, View, text};

    #[test]
    fn cursor_validation_propagates_through_desktop_startup() {
        let error = Application::shell_environment("Invalid cursors")
            .compositor(Compositor::new().cursor_theme(CursorTheme::new()))
            .into_ready()
            .into_parts()
            .unwrap_err()
            .to_string();
        assert!(error.contains("missing cursor"));
        assert!(error.contains("logical size"));
    }

    #[test]
    fn cursor_theme_completes_compositor_without_widgets() {
        Application::shell_environment("Cursor order")
            .assets(crate::assets::cursor_test_bundle())
            .compositor(
                Compositor::new()
                    .cursor_theme(crate::assets::cursor_test_theme())
                    .cursor_size(32.0),
            )
            .into_ready()
            .into_parts()
            .unwrap();
    }

    #[test]
    fn resize_preview_accepts_the_full_alpha_range() {
        let mut config = LinuxShellConfig::default();
        config.drm_device = Some(std::env::current_dir().unwrap().join("card0"));
        assert_eq!(config.resize_preview.color().a, 255);
        for alpha in [0, 128, 255] {
            config.resize_preview =
                crate::ResizePreview::Color(ColorRgba8::rgba(20, 40, 60, alpha));
            assert!(config.validate().is_ok());
        }
    }

    struct Root;

    impl ComponentFields for Root {
        type InputSnapshot = ();

        fn update_inputs(&mut self, _incoming: Self) -> bool {
            false
        }

        fn capture_inputs(&self) -> Self::InputSnapshot {}

        fn restore_inputs(&mut self, _snapshot: Self::InputSnapshot) -> bool {
            false
        }
    }

    impl Component for Root {
        fn view(&self) -> impl View {
            text(format!("{:?}", self.runtime_target()))
        }
    }

    impl crate::compose::ShellWidget for Root {
        fn surface(&self) -> crate::compose::ShellSurfaceSpec {
            crate::compose::ShellSurfaceSpec::new()
        }
    }

    #[test]
    fn mode_specific_roots_tag_their_composition_targets() {
        let window = Window::new("Window").content(Root);
        assert_eq!(window.content.target(), RuntimeTarget::Application);

        let widget = RegisteredShellWidget::new(Root);
        assert_eq!(widget.content.target(), RuntimeTarget::ShellWidget);

        let compositor = Compositor::new().cursor_theme(CursorTheme::new());
        assert!(compositor.frame_template().is_none());
    }

    #[test]
    fn compositor_has_no_component_background() {
        #[allow(deprecated)]
        let compositor = Compositor::new().cursor_theme(CursorTheme::new());
        assert!(compositor.frame_template().is_none());
    }

    #[test]
    fn named_frame_functions_implement_the_template_contract() {
        fn compose(_model: WindowChromeModel) -> Root {
            Root
        }

        let compositor = Compositor::new()
            .cursor_theme(CursorTheme::new())
            .window_frame(compose);
        assert!(compositor.frame_template().is_some());
        assert_eq!(
            (compositor.frame_template().unwrap().content_style)(&WindowChromeModel::new(
                1, "Legacy"
            )),
            None
        );
    }

    #[test]
    fn frame_factory_preserves_model_specific_content_style() {
        struct Styled;
        impl WindowFrameTemplate for Styled {
            type Component = Root;
            fn compose(&self, _model: WindowChromeModel) -> Root {
                Root
            }
            fn content_style(&self, model: &WindowChromeModel) -> Option<WindowContentStyle> {
                Some(WindowContentStyle {
                    background: ColorRgba8::rgba(0, 0, 0, 0),
                    corner_radius: 4.0,
                    resize_preview: model.active.then_some(crate::ResizePreview::Color(
                        ColorRgba8::rgba(40, 50, 60, 128),
                    )),
                })
            }
        }
        let factory = WindowFrameFactory::new(Styled);
        for active in [false, true] {
            let model = WindowChromeModel::new(1, "Styled").active(active);
            assert_eq!(
                (factory.content_style)(&model),
                Styled.content_style(&model)
            );
        }
    }

    #[test]
    fn custom_shell_actions_require_unique_declaration_authorization() {
        let action = ShellActionId::named("window.pin");
        let compositor = Compositor::new()
            .cursor_theme(CursorTheme::new())
            .shell_action(action, |_| {})
            .shell_action(action, |_| {});

        assert!(compositor.validate().is_err());
    }

    #[test]
    fn desktop_without_widgets_validates_and_has_no_reserved_widget_space() {
        let desktop = Application::shell_environment("Bare desktop")
            .assets(crate::assets::cursor_test_bundle())
            .compositor(Compositor::new().cursor_theme(crate::assets::cursor_test_theme()))
            .into_ready();
        let (_, _, widgets, _, _, _, _, _, _) = desktop.into_parts().unwrap();
        assert!(widgets.is_empty());

        // Verify the direct entrypoint exists without starting a desktop in this test.
        let _: fn(ShellEnvironmentWithCompositor) -> AppResult<()> =
            ShellEnvironmentWithCompositor::run;
        assert!(
            Application::shell_environment("")
                .compositor(Compositor::new().cursor_theme(CursorTheme::new()))
                .into_ready()
                .into_parts()
                .is_err()
        );
    }

    #[test]
    fn both_application_modes_own_renderer_selection() {
        let gui = Application::gui("Counter")
            .renderer(Renderer::Software)
            .window(Window::new("Counter").content(Root));
        assert_eq!(gui.renderer, Renderer::Software);

        let desktop = Application::shell_environment("Telorgon")
            .renderer(Renderer::Vulkan)
            .compositor(Compositor::new().cursor_theme(CursorTheme::new()))
            .widget(Root);
        assert_eq!(desktop.renderer, Renderer::Vulkan);
    }

    #[test]
    fn complete_declarations_have_content_without_optional_storage() {
        let application =
            Application::gui("Counter").window(Window::new("Counter").size(480, 320).content(Root));
        let debug = format!("{application:?}");
        assert!(debug.contains("has_content: true"));
        assert!(debug.contains("renderer: Auto"));
    }

    #[test]
    fn keybindings_reach_ready_compositor_and_last_registration_wins() {
        use crate::application_host::{KeyChord, ShortcutKey};
        fn noop() {}
        let bindings = KeyBindings::new().bind(KeyChord::new(ShortcutKey::Space), noop);
        let event = ShellKeyEvent {
            keysym: 0x20,
            ..Default::default()
        };
        let mut ready = Compositor::new()
            .cursor_theme(CursorTheme::new())
            .keyboard_shortcut_handler(|_| ShellKeyAction::Quit)
            .keybindings(bindings.clone());
        assert_eq!(
            ready.keyboard_shortcut_handler.as_mut().unwrap()(event),
            ShellKeyAction::Consume
        );
        let mut raw = Compositor::new()
            .cursor_theme(CursorTheme::new())
            .keybindings(bindings)
            .keyboard_shortcut_handler(|_| ShellKeyAction::Quit);
        assert_eq!(
            raw.keyboard_shortcut_handler.as_mut().unwrap()(event),
            ShellKeyAction::Quit
        );
        let mut empty = raw.keybindings(KeyBindings::new());
        assert_eq!(
            empty.keyboard_shortcut_handler.as_mut().unwrap()(event),
            ShellKeyAction::Forward
        );
    }

    #[test]
    fn managed_windows_retain_custom_frame_and_icon_options() {
        let icon = crate::IconAsset::new(crate::AssetKey::new("icons/app.svg"));
        let (_, options) = Window::new("Studio")
            .custom_frame()
            .icon(AppIconProfile::new().named("com.example.studio").icon(icon))
            .content(Root)
            .into_parts()
            .unwrap();

        assert_eq!(options.decorations, WindowDecorationMode::Hidden);
        assert_eq!(options.icon.name(), Some("com.example.studio"));
        assert_eq!(options.icon.preferred(64), Some(crate::Icon::new(icon)));
    }
}

#[cfg(test)]
mod keyboard_configuration_tests {
    use super::*;

    #[test]
    fn keyboard_names_preserve_defaults_and_reject_nul_in_every_field() {
        let defaults = LinuxShellConfig::default();
        assert_eq!(defaults.keyboard, KeyboardConfig::default());
        defaults.validate().unwrap();
        for field in 0..5 {
            let mut config = LinuxShellConfig::default();
            let names = &mut config.keyboard;
            let target = match field {
                0 => &mut names.rules,
                1 => &mut names.model,
                2 => &mut names.layout,
                3 => &mut names.variant,
                _ => &mut names.options,
            };
            *target = Some("invalid\0name".into());
            assert!(config.validate().is_err());
        }
        let config = KeyboardConfig {
            layout: Some("us,de".into()),
            variant: Some(",nodeadkeys".into()),
            options: Some(String::new()),
            ..Default::default()
        };
        config.validate().unwrap();
        assert_eq!(config.options.as_deref(), Some(""));
    }

    #[cfg(all(target_os = "linux", feature = "shell-wayland-linux"))]
    #[test]
    fn explicit_layout_changes_the_compiled_seat_keymap() {
        use crate::platform_linux::XkbKeyboard;
        let compile = |layout: &str| {
            let config = KeyboardConfig {
                include_root: None,
                rules: Some("evdev".into()),
                model: Some("pc105".into()),
                layout: Some(layout.into()),
                variant: Some(String::new()),
                options: Some(String::new()),
            };
            config.validate().unwrap();
            XkbKeyboard::from_names(
                config.rules.as_deref(),
                config.model.as_deref(),
                config.layout.as_deref(),
                config.variant.as_deref(),
                config.options.as_deref(),
            )
            .unwrap()
        };
        let us = compile("us");
        let de = compile("de");
        // The same physical evdev key is Y in US and Z in German QWERTZ.
        assert_eq!(us.utf8(21).unwrap(), "y");
        assert_eq!(de.utf8(21).unwrap(), "z");
        assert_ne!(us.keymap_string().unwrap(), de.keymap_string().unwrap());
    }
}

impl<C> Compositor<C> {
    /// Supplies the required cursor design; validated automatically by desktop startup.
    pub fn cursor_theme(self, theme: CursorTheme) -> Compositor<CursorTheme> {
        Compositor {
            cursor_theme: theme,
            client_cursor_mode: self.client_cursor_mode,
            window_frame: self.window_frame,
            icons: self.icons,
            shell_actions: self.shell_actions,
            keyboard_shortcut_handler: self.keyboard_shortcut_handler,
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

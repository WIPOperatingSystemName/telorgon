use super::*;

/// Shell-environment declaration that still requires its compositor.
pub struct ShellEnvironment {
    pub(in crate::host::application) network: Option<crate::services::network::NetworkController>,
    pub(in crate::host::application) screen_brightness:
        Option<crate::screen_brightness::ScreenBrightnessController>,
    capture: crate::host::application::Capture,
    pub(in crate::host::application) services:
        crate::authoring::compose::shell_services::ShellServiceRegistry,
    name: String,
    pub(super) renderer: Renderer,
    linux: LinuxShellConfig,
    assets: AssetBundle,
    app_icon: AppIconProfile,
}

impl ShellEnvironment {
    pub(super) fn new(name: String) -> Self {
        Self {
            capture: crate::host::application::Capture::new(),
            screen_brightness: None,
            network: None,
            services: Default::default(),
            name,
            renderer: Renderer::Auto,
            linux: LinuxShellConfig::default(),
            assets: AssetBundle::EMPTY,
            app_icon: AppIconProfile::new(),
        }
    }
    pub fn typography(mut self, typography: crate::Typography) -> Self {
        self.linux.typography = typography;
        self
    }
    /// Configures capture interfaces. Defaults to disabled, even when compiled with capture support.
    pub fn capture(mut self, capture: crate::host::application::Capture) -> Self {
        self.capture = capture;
        self
    }

    /// Returns the declarative capture configuration without starting a backend.
    pub const fn configured_capture(&self) -> crate::host::application::Capture {
        self.capture
    }

    /// Configures the environment-owned installed application catalog.
    pub fn applications(mut self, catalog: crate::authoring::compose::ApplicationCatalog) -> Self {
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
            capture: self.capture,
            screen_brightness: self.screen_brightness,
            network: self.network,
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
    pub(in crate::host::application) network: Option<crate::services::network::NetworkController>,
    pub(in crate::host::application) screen_brightness:
        Option<crate::screen_brightness::ScreenBrightnessController>,
    capture: crate::host::application::Capture,
    pub(in crate::host::application) services:
        crate::authoring::compose::shell_services::ShellServiceRegistry,
    name: String,
    pub(super) renderer: Renderer,
    linux: LinuxShellConfig,
    assets: AssetBundle,
    app_icon: AppIconProfile,
    compositor: ReadyCompositor,
}

impl ShellEnvironmentWithCompositor {
    pub fn typography(mut self, typography: crate::Typography) -> Self {
        self.linux.typography = typography;
        self
    }
    /// Configures capture interfaces. Defaults to disabled, even when compiled with capture support.
    pub fn capture(mut self, capture: crate::host::application::Capture) -> Self {
        self.capture = capture;
        self
    }

    /// Returns the declarative capture configuration without starting a backend.
    pub const fn configured_capture(&self) -> crate::host::application::Capture {
        self.capture
    }

    /// Configures the environment-owned installed application catalog.
    pub fn applications(mut self, catalog: crate::authoring::compose::ApplicationCatalog) -> Self {
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

    pub fn widget<W: crate::authoring::compose::ShellWidget>(
        self,
        widget: W,
    ) -> ReadyShellEnvironment {
        self.into_ready().widget(widget)
    }

    /// Starts the shell with no widgets.
    pub fn run(self) -> AppResult<()> {
        self.into_ready().run()
    }

    pub(super) fn into_ready(self) -> ReadyShellEnvironment {
        ReadyShellEnvironment {
            capture: self.capture,
            screen_brightness: self.screen_brightness,
            network: self.network,
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
    pub(in crate::host::application) network: Option<crate::services::network::NetworkController>,
    pub(in crate::host::application) screen_brightness:
        Option<crate::screen_brightness::ScreenBrightnessController>,
    capture: crate::host::application::Capture,
    pub(in crate::host::application) services:
        crate::authoring::compose::shell_services::ShellServiceRegistry,
    name: String,
    pub(super) renderer: Renderer,
    linux: LinuxShellConfig,
    assets: AssetBundle,
    app_icon: AppIconProfile,
    compositor: ReadyCompositor,
    shell_widgets: Vec<RegisteredShellWidget>,
}

impl ReadyShellEnvironment {
    pub fn typography(mut self, typography: crate::Typography) -> Self {
        self.linux.typography = typography;
        self
    }
    /// Configures capture interfaces. Defaults to disabled, even when compiled with capture support.
    pub fn capture(mut self, capture: crate::host::application::Capture) -> Self {
        self.capture = capture;
        self
    }

    /// Returns the declarative capture configuration without starting a backend.
    pub const fn configured_capture(&self) -> crate::host::application::Capture {
        self.capture
    }

    /// Configures the environment-owned installed application catalog.
    pub fn applications(mut self, catalog: crate::authoring::compose::ApplicationCatalog) -> Self {
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

    pub fn widget<W: crate::authoring::compose::ShellWidget>(mut self, widget: W) -> Self {
        self.shell_widgets.push(RegisteredShellWidget::new(widget));
        self
    }

    /// Validates this declaration and enters the Linux shell-environment runtime.
    pub fn run(self) -> AppResult<()> {
        #[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
        {
            crate::host::linux_shell::run(self)
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
        crate::authoring::compose::shell_services::ShellServiceRegistry,
        Renderer,
        LinuxShellConfig,
        AssetBundle,
        PointerConfiguration,
        AppIconProfile,
        Option<crate::screen_brightness::ScreenBrightnessController>,
        Option<crate::services::network::NetworkController>,
    )> {
        self.capture.validate(
            self.renderer,
            self.linux.session.publish_user_service_environment,
            self.shell_widgets.len(),
        )?;
        #[cfg(all(feature = "shell-screencast-linux", target_os = "linux"))]
        if self.capture.configured_portal().is_some() {
            self.compositor
                .screen_cast_portal
                .as_ref()
                .ok_or_else(|| {
                    AppError::new("portal capture requires Compositor::screen_cast_portal")
                })?
                .validate()?;
        }
        validate_application_name(&self.name)?;
        crate::AssetResolver::new(self.assets).map_err(|error| AppError::new(error.to_string()))?;
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
            self.screen_brightness,
            self.network,
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

macro_rules! network_builder {
    ($owner:ty) => {
        impl $owner {
            /// Installs an unstarted controller. The shell owns startup and shutdown;
            /// widgets resolve NetworkObserver or NetworkHandle from ShellServices.
            pub fn network(
                mut self,
                controller: crate::services::network::NetworkController,
            ) -> Self {
                self.services.insert(controller.observer());
                self.services.insert(controller.handle());
                self.network = Some(controller);
                self
            }
        }
    };
}
network_builder!(ShellEnvironment);
network_builder!(ShellEnvironmentWithCompositor);
network_builder!(ReadyShellEnvironment);

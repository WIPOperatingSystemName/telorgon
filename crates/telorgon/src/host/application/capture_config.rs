//! Declarative capture interfaces. Constructing these values starts no services.
use super::{AppError, AppResult, Renderer};

/// Source kinds offered to capture consumers. Combine using `|`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureSources(u32);
impl CaptureSources {
    pub const MONITORS: Self = Self(1);
    pub const WINDOWS: Self = Self(2);
    pub const VIRTUAL_OUTPUTS: Self = Self(4);
    pub const fn empty() -> Self {
        Self(0)
    }
    pub const fn all() -> Self {
        Self(7)
    }
    pub const fn bits(self) -> u32 {
        self.0
    }
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}
impl std::ops::BitOr for CaptureSources {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}
impl std::ops::BitOrAssign for CaptureSources {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = self.union(rhs);
    }
}

/// Capture entry points for a shell environment. Interfaces are opt-in; source types are shared.
/// Requires Linux, `shell-screencast-linux`, and the Vulkan renderer for portal delivery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capture {
    sources: CaptureSources,
    portal: Option<PortalCapture>,
    wayland: Option<WaylandCapture>,
    internal: Option<InternalCapture>,
}
impl Default for Capture {
    fn default() -> Self {
        Self::new()
    }
}
impl Capture {
    /// All interfaces disabled; physical displays, windows and virtual displays available when enabled.
    pub const fn new() -> Self {
        Self {
            sources: CaptureSources::all(),
            portal: None,
            wayland: None,
            internal: None,
        }
    }
    /// Portal sharing with a chooser supplied through `Compositor::screen_cast_portal`.
    pub const fn desktop() -> Self {
        Self::new().portal(PortalCapture::new())
    }
    pub const fn sources(mut self, sources: CaptureSources) -> Self {
        self.sources = sources;
        self
    }
    pub const fn portal(mut self, portal: PortalCapture) -> Self {
        self.portal = Some(portal);
        self
    }
    /// Declares direct capture. Currently rejected at startup: consent integration is unfinished.
    pub const fn wayland(mut self, wayland: WaylandCapture) -> Self {
        self.wayland = Some(wayland);
        self
    }
    /// Declares component capture. Currently rejected at startup: the service is unfinished.
    pub const fn internal(mut self, internal: InternalCapture) -> Self {
        self.internal = Some(internal);
        self
    }
    pub const fn configured_sources(self) -> CaptureSources {
        self.sources
    }
    pub const fn configured_portal(self) -> Option<PortalCapture> {
        self.portal
    }
    pub const fn configured_wayland(self) -> Option<WaylandCapture> {
        self.wayland
    }
    pub const fn configured_internal(self) -> Option<InternalCapture> {
        self.internal
    }
    pub const fn is_enabled(self) -> bool {
        self.portal.is_some() || self.wayland.is_some() || self.internal.is_some()
    }

    pub(crate) fn validate(
        self,
        renderer: Renderer,
        owns_session: bool,
        widgets: usize,
    ) -> AppResult<()> {
        if !self.is_enabled() {
            return Ok(());
        }
        if self.sources.is_empty() || self.sources.bits() & !CaptureSources::all().bits() != 0 {
            return Err(AppError::new(
                "capture requires a nonempty set of supported CaptureSources",
            ));
        }
        if self.wayland.is_some() {
            return Err(AppError::new(
                "WaylandCapture is not available yet: direct Wayland consent integration is unfinished; use Capture::desktop() for portal sharing",
            ));
        }
        if self.internal.is_some() {
            return Err(AppError::new(
                "InternalCapture is not available yet: component capture delivery is unfinished; use Capture::desktop() for portal sharing",
            ));
        }
        if !cfg!(all(target_os = "linux", feature = "shell-screencast-linux")) {
            return Err(AppError::new(
                "portal capture requires Linux and the telorgon/shell-screencast-linux Cargo feature",
            ));
        }
        if renderer == Renderer::Software {
            return Err(AppError::new(
                "portal capture requires Renderer::Vulkan (or Auto with Vulkan available)",
            ));
        }
        if widgets > 254 {
            return Err(AppError::new(
                "portal capture needs two free shell widget slots for the portal picker and sharing controls",
            ));
        }
        if self
            .portal
            .is_some_and(|portal| portal.integration == PortalSessionIntegration::DesktopSession)
            && !owns_session
        {
            return Err(AppError::new(
                "DesktopSession portal integration requires SessionConfig.publish_user_service_environment = true; use External for a concurrently running development desktop",
            ));
        }
        Ok(())
    }

    #[cfg(all(target_os = "linux", feature = "shell-wayland-linux"))]
    pub(crate) fn allows(self, source: crate::shell::capture::CaptureSource) -> bool {
        self.is_enabled()
            && self.sources.contains(match source {
                crate::shell::capture::CaptureSource::Output(_) => CaptureSources::MONITORS,
                crate::shell::capture::CaptureSource::Window(_) => CaptureSources::WINDOWS,
                crate::shell::capture::CaptureSource::VirtualOutput(_) => {
                    CaptureSources::VIRTUAL_OUTPUTS
                }
            })
    }
}

/// Portal/PipeWire capture. Consent remains mandatory and uses the compositor's chooser.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PortalCapture {
    integration: PortalSessionIntegration,
}
impl PortalCapture {
    pub const fn new() -> Self {
        Self {
            integration: PortalSessionIntegration::Automatic,
        }
    }
    pub const fn session_integration(mut self, integration: PortalSessionIntegration) -> Self {
        self.integration = integration;
        self
    }
    pub const fn configured_session_integration(self) -> PortalSessionIntegration {
        self.integration
    }
    pub(crate) const fn activates_frontend(self, owns_session: bool) -> bool {
        owns_session && !matches!(self.integration, PortalSessionIntegration::External)
    }
}

/// Shared portal services are never taken over on inferred session ownership.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PortalSessionIntegration {
    /// Activate the frontend only when `publish_user_service_environment` explicitly owns integration.
    /// Otherwise register the backend and leave frontend/environment management external.
    #[default]
    Automatic,
    /// Require explicit session ownership; activate the frontend after backend registration.
    DesktopSession,
    /// Register only the backend. Do not activate/restart the shared frontend.
    External,
}

/// Reserved direct-capture configuration; enabling it currently produces a startup error.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WaylandCapture {
    protocols: CaptureProtocols,
    access: DirectCaptureAccess,
}
impl WaylandCapture {
    pub const fn new() -> Self {
        Self {
            protocols: CaptureProtocols::ImageCopyCapture,
            access: DirectCaptureAccess::Ask,
        }
    }
    pub const fn protocols(mut self, protocols: CaptureProtocols) -> Self {
        self.protocols = protocols;
        self
    }
    pub const fn access(mut self, access: DirectCaptureAccess) -> Self {
        self.access = access;
        self
    }
    pub const fn configured_protocols(self) -> CaptureProtocols {
        self.protocols
    }
    pub const fn configured_access(self) -> DirectCaptureAccess {
        self.access
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CaptureProtocols {
    #[default]
    ImageCopyCapture,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DirectCaptureAccess {
    #[default]
    Ask,
}

/// Reserved component capture configuration; enabling it currently produces a startup error.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InternalCapture;
impl InternalCapture {
    pub const fn new() -> Self {
        Self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_interfaces_are_opt_in_and_desktop_is_portal_only() {
        assert!(!Capture::new().is_enabled());
        assert_eq!(Capture::default(), Capture::new());
        let desktop = Capture::desktop();
        assert!(desktop.configured_portal().is_some());
        assert!(desktop.configured_wayland().is_none());
        assert!(desktop.configured_internal().is_none());
        assert_eq!(
            desktop.configured_sources(),
            CaptureSources::MONITORS | CaptureSources::WINDOWS | CaptureSources::VIRTUAL_OUTPUTS
        );
        assert!(
            Capture::new()
                .validate(Renderer::Software, false, 256)
                .is_ok()
        );
    }

    #[test]
    fn capture_unfinished_interfaces_and_empty_sources_fail_explicitly() {
        for (capture, message) in [
            (
                Capture::new().wayland(WaylandCapture::new()),
                "WaylandCapture",
            ),
            (
                Capture::new().internal(InternalCapture::new()),
                "InternalCapture",
            ),
            (
                Capture::desktop().sources(CaptureSources::empty()),
                "nonempty",
            ),
        ] {
            assert!(
                capture
                    .validate(Renderer::Vulkan, true, 0)
                    .unwrap_err()
                    .to_string()
                    .contains(message)
            );
        }
    }

    #[test]
    fn capture_session_integration_never_infers_shared_service_ownership() {
        for owns in [false, true] {
            assert_eq!(PortalCapture::new().activates_frontend(owns), owns);
            assert!(
                !PortalCapture::new()
                    .session_integration(PortalSessionIntegration::External)
                    .activates_frontend(owns)
            );
            assert_eq!(
                PortalCapture::new()
                    .session_integration(PortalSessionIntegration::DesktopSession)
                    .activates_frontend(owns),
                owns
            );
        }
    }

    #[cfg(all(feature = "shell-screencast-linux", target_os = "linux"))]
    #[test]
    fn capture_portal_checks_renderer_slots_and_explicit_session_ownership() {
        assert!(
            Capture::desktop()
                .validate(Renderer::Vulkan, false, 254)
                .is_ok()
        );
        assert!(
            Capture::desktop()
                .validate(Renderer::Auto, false, 0)
                .is_ok()
        );
        assert!(
            Capture::desktop()
                .validate(Renderer::Software, true, 0)
                .unwrap_err()
                .to_string()
                .contains("Vulkan")
        );
        assert!(
            Capture::desktop()
                .validate(Renderer::Vulkan, true, 255)
                .unwrap_err()
                .to_string()
                .contains("slots")
        );
        let capture = Capture::new().portal(
            PortalCapture::new().session_integration(PortalSessionIntegration::DesktopSession),
        );
        assert!(
            capture
                .validate(Renderer::Vulkan, false, 0)
                .unwrap_err()
                .to_string()
                .contains("publish_user_service_environment")
        );
        assert!(capture.validate(Renderer::Vulkan, true, 0).is_ok());
    }

    #[cfg(not(all(feature = "shell-screencast-linux", target_os = "linux")))]
    #[test]
    fn capture_portal_requires_the_native_feature() {
        assert!(
            Capture::desktop()
                .validate(Renderer::Vulkan, true, 0)
                .unwrap_err()
                .to_string()
                .contains("shell-screencast-linux")
        );
    }
}

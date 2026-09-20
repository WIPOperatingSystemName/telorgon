use telorgon::app::*;

#[component]
struct Desktop {}
impl Component for Desktop {
    fn view(&self) -> impl View {
        stack()
    }
}
impl ShellWidget for Desktop {
    fn surface(&self) -> ShellSurfaceSpec {
        ShellSurfaceSpec::new()
    }
}

#[test]
fn capture_configuration_survives_all_shell_builder_stages() {
    let config = Capture::new()
        .sources(CaptureSources::MONITORS | CaptureSources::WINDOWS)
        .portal(PortalCapture::new().session_integration(PortalSessionIntegration::External))
        .wayland(
            WaylandCapture::new()
                .protocols(CaptureProtocols::ImageCopyCapture)
                .access(DirectCaptureAccess::Ask),
        )
        .internal(InternalCapture::new());
    let shell = Application::shell_environment("Capture API").capture(config);
    assert_eq!(shell.configured_capture(), config);
    let shell = shell.compositor(Compositor::new().cursor_theme(CursorTheme::new()));
    assert_eq!(shell.configured_capture(), config);
    let shell = shell.widget(Desktop::default());
    assert_eq!(shell.configured_capture(), config);
    assert_eq!(
        shell.capture(Capture::desktop()).configured_capture(),
        Capture::desktop()
    );
}

#[test]
fn capture_declarations_default_off_and_can_be_changed_after_compositor() {
    let shell = Application::shell_environment("Default capture");
    assert_eq!(shell.configured_capture(), Capture::new());
    let shell = shell
        .compositor(Compositor::new().cursor_theme(CursorTheme::new()))
        .capture(Capture::desktop().sources(CaptureSources::WINDOWS));
    assert_eq!(
        shell.configured_capture().configured_sources(),
        CaptureSources::WINDOWS
    );
    assert!(shell.configured_capture().configured_wayland().is_none());
    assert!(shell.configured_capture().configured_internal().is_none());
}

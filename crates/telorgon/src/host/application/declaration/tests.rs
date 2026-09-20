use super::*;
use crate::authoring::compose::{ComponentFields, View, text};

#[test]
fn capture_startup_validation_precedes_device_and_session_creation() {
    let invalid = Application::shell_environment("Invalid capture")
        .capture(super::super::Capture::new().internal(super::super::InternalCapture::new()))
        .compositor(Compositor::new().cursor_theme(CursorTheme::new()))
        .into_ready()
        .into_parts()
        .unwrap_err()
        .to_string();
    assert!(invalid.contains("InternalCapture"));
    assert!(!invalid.contains("cursor"));
}

#[cfg(all(feature = "shell-screencast-linux", target_os = "linux"))]
#[test]
fn custom_capture_factory_survives_cursor_configuration_and_runs_once() {
    let calls = std::rc::Rc::new(std::cell::Cell::new(0));
    let invoked = calls.clone();
    let mut compositor = Compositor::new()
        .capture_chooser(move |ui| {
            invoked.set(invoked.get() + 1);
            crate::components::shell::capture::CapturePicker::new(ui)
        })
        .cursor_theme(CursorTheme::new());
    assert_eq!(calls.get(), 0);
    let factory = compositor.take_capture_chooser().unwrap();
    assert!(compositor.take_capture_chooser().is_none());
    let _widget = factory(crate::CaptureUi::default());
    assert_eq!(calls.get(), 1);
}

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
    assert_eq!(config.resize_preview.fill.color().a, 255);
    for alpha in [0, 128, 255] {
        config.resize_preview = crate::ResizePreviewDesign::new(crate::Fill::Color(
            ColorRgba8::rgba(20, 40, 60, alpha),
        ));
        assert!(config.validate().is_ok());
    }
}

#[test]
fn resize_preview_rejects_invalid_border_widths() {
    for width in [-1.0, f32::NAN, f32::INFINITY] {
        let mut config = LinuxShellConfig::default();
        config.resize_preview.border =
            crate::Border::all(width, ColorRgba8::rgba(255, 255, 255, 255));
        assert!(config.validate().is_err());
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

impl crate::authoring::compose::ShellWidget for Root {
    fn surface(&self) -> crate::authoring::compose::ShellSurfaceSpec {
        crate::authoring::compose::ShellSurfaceSpec::new()
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
                resize_preview: model.active.then_some(crate::ResizePreviewDesign::new(
                    crate::Fill::Color(ColorRgba8::rgba(40, 50, 60, 128)),
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
    use crate::host::application::{KeyChord, ShortcutKey};
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

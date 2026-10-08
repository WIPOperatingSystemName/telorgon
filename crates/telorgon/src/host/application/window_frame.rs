#[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
use crate::authoring::compose::RuntimeTarget;
use crate::authoring::compose::{Component, ErasedComponent};
#[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
use crate::runtime::CompositionDriver;
use crate::shell::window_chrome::{WindowChromeModel, WindowContentStyle};
use std::fmt;

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
    pub(super) motion: Box<dyn Fn(&WindowChromeModel) -> Option<crate::WindowMotion>>,
    #[cfg_attr(
        not(all(feature = "shell-wayland-linux", target_os = "linux")),
        allow(dead_code)
    )]
    pub(super) compose: Box<dyn Fn(WindowChromeModel) -> Box<dyn ErasedComponent>>,
    #[cfg_attr(
        not(all(feature = "shell-wayland-linux", target_os = "linux")),
        allow(dead_code)
    )]
    pub(super) content_style: Box<dyn Fn(&WindowChromeModel) -> Option<WindowContentStyle>>,
}

impl WindowFrameFactory {
    #[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
    pub(crate) fn motion(&self, model: &WindowChromeModel) -> Option<crate::WindowMotion> {
        (self.motion)(model)
    }
    pub(crate) fn new<T>(template: T) -> Self
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

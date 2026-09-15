//! Ergonomic rerender-and-reconcile authoring values for Telorgon.
//!
//! A [`View`] is a short-lived description. The runtime retains component instances and foundation
//! nodes, evaluates only dirty components, and reconciles a view into those existing nodes.

mod component;
mod components;
mod context;
mod element;
mod event;
mod key;
pub(crate) mod shell_services;
mod signal;
pub use shell_services::{
    ShellRequestCompletion, ShellRequestOutcome, ShellServiceError, ShellServices, ShellWindow,
    ShellWindowAction,
};
pub(crate) mod shell_widget;
pub use shell_widget::{
    ShellAttachment, ShellChild, ShellDismissReason, ShellEdge, ShellExtent, ShellFocus,
    ShellPlacementBounds, ShellPointer, ShellReservation, ShellSurfaceLayer, ShellSurfaceSpec,
    ShellWidget, WidgetPlacement,
};
pub mod style;

pub use crate::window_chrome::{
    ContentFade, GeometryMotion, Minimize, Spring, WindowMotion, WindowTween, tween_ms,
};

pub use component::{
    Component, ComponentFields, ComponentInstanceId, ErasedComponent, RenderedView,
};
pub use components::{
    Button, ButtonElement, Checkbox, Container, ContainerElement, EasyWindowFrame,
    EasyWindowFrameComponent, HasContent, Image, ImageElement, MissingContent, Slider,
    SliderElement, Switch, Text, TextElement, ToggleElement, ToggleKind, WindowChromeDesign,
    WindowChromeDesignError, WindowChromePalette, WindowChromeStateStyle, WindowChromeViewExt,
    WindowContentSlot, WindowControlButtonStyle, WindowControlDesign, WindowControlVisual,
    WindowControlsDesign, WindowFrame, WindowTitleBarStyle, button, card, checkbox, column,
    easy_window_frame, image, row, slider, spacer, stack, switch, text, window_content_slot,
    window_frame,
};
pub use context::{InputsChangedContext, MountContext, RuntimeTarget, UnmountContext};
pub use element::{Element, ElementKind, ElementType, View, ViewError};
pub use event::{ComponentCallback, EventContext, EventDispatch, EventHandler};
pub use key::{Key, hashed_key};
pub use signal::{Signal, SignalDependency, SignalSnapshot, SignalSubscription, SignalWriter};
pub use style::{Alignment, Dimension, Insets, TextStyle};

/// Adds a semantic pointer request to any composed view.
///
/// The host resolves the request through the application's pointer overrides, registered cursor
/// theme, and finally the system cursor. This keeps cursor artwork out of component layout code.
pub trait PointerViewExt: View + Sized {
    fn pointer_icon(self, icon: crate::PointerIcon) -> Element {
        self.into_element()
            .with_pointer_request(crate::PointerRequest::Semantic(icon))
    }

    fn hide_pointer(self) -> Element {
        self.into_element()
            .with_pointer_request(crate::PointerRequest::Hidden)
    }
}

impl<T: View> PointerViewExt for T {}

/// Items used by the generated component implementation. Not a stable direct-use API.
#[doc(hidden)]
pub mod __private {
    pub use crate::compose::ComponentFields;
}

#[cfg(test)]
mod tests;

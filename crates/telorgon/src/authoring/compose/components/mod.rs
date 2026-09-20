//! Built-in composition builders, split by component.

mod button;
mod checkbox;
mod container;
mod easy_window_frame;
mod image;
mod slider;
mod switch;
mod text;
mod toggle;
mod window_frame;

pub use button::{Button, ButtonElement, button};
pub use checkbox::{Checkbox, checkbox};
pub use container::{Container, ContainerElement, card, column, row, spacer, stack};
pub use easy_window_frame::{
    EasyWindowFrame, EasyWindowFrameComponent, WindowChromeDesign, WindowChromeDesignError,
    WindowChromePalette, WindowChromeStateStyle, WindowControlButtonStyle, WindowControlDesign,
    WindowControlVisual, WindowControlsDesign, WindowTitleBarStyle, easy_window_frame,
};
pub use image::{Image, ImageElement, image};
pub use slider::{Slider, SliderElement, slider};
pub use switch::{Switch, switch};
pub use text::{Text, TextElement, text};
pub use toggle::{ToggleElement, ToggleKind};
pub use window_frame::{
    HasContent, MissingContent, WindowChromeViewExt, WindowContentSlot, WindowFrame,
    window_content_slot, window_frame,
};

use crate::boot::{BootInterface, PreviewCommand};
use crate::input::LogicalKey;

pub(crate) fn keyboard(interface: &mut BootInterface, logical: &LogicalKey) -> bool {
    let LogicalKey::Character(value) = logical else {
        return false;
    };
    match value.as_str().to_ascii_lowercase().as_str() {
        "p" => interface.controller.dispatch(PreviewCommand::TogglePause),
        "f" => interface.controller.dispatch(PreviewCommand::Fail),
        _ => return false,
    }
    true
}

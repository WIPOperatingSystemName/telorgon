//! Shell-owned Theme v4 catalog and typed style identities.

use crate::theme::ThemeCatalog;
use crate::ui::{ComponentStyleId, ThemeDomainId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowControlsStyleId(pub ComponentStyleId);

impl WindowControlsStyleId {
    pub const DEFAULT: Self = Self(ComponentStyleId::named(
        ThemeDomainId::SHELL,
        "window-controls",
        "default",
    ));
}

impl From<WindowControlsStyleId> for ComponentStyleId {
    fn from(value: WindowControlsStyleId) -> Self {
        value.0
    }
}

pub fn shell_theme_catalog() -> ThemeCatalog {
    crate::theme::shell_catalog()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_window_controls_id_is_published_by_shell_catalog() {
        assert_eq!(
            shell_theme_catalog().style_id("window-controls", "default"),
            Some(WindowControlsStyleId::DEFAULT.0)
        );
    }
}

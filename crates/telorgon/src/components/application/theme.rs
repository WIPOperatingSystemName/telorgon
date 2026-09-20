//! Application-owned Theme v4 catalog and typed style identities.

use crate::theme::ThemeCatalog;
use crate::ui::{ComponentStyleId, ThemeDomainId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ButtonStyleId(pub ComponentStyleId);

impl ButtonStyleId {
    pub const DEFAULT: Self = Self(ComponentStyleId::named(
        ThemeDomainId::APPLICATION,
        "button",
        "default",
    ));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ActivityIndicatorStyleId(pub ComponentStyleId);

impl ActivityIndicatorStyleId {
    pub const DEFAULT: Self = Self(ComponentStyleId::named(
        ThemeDomainId::APPLICATION,
        "activity-indicator",
        "default",
    ));
}

impl From<ButtonStyleId> for ComponentStyleId {
    fn from(value: ButtonStyleId) -> Self {
        value.0
    }
}

impl From<ActivityIndicatorStyleId> for ComponentStyleId {
    fn from(value: ActivityIndicatorStyleId) -> Self {
        value.0
    }
}

pub fn application_theme_catalog() -> ThemeCatalog {
    crate::theme::application_catalog()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_button_id_is_published_by_application_catalog() {
        assert_eq!(
            application_theme_catalog().style_id("button", "default"),
            Some(ButtonStyleId::DEFAULT.0)
        );
    }
}

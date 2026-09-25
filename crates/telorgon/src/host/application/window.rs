use crate::AppIconProfile;
use crate::foundation::SizeI;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowDecorationMode {
    #[default]
    System,
    Hidden,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowOptions {
    pub title: String,
    pub size: SizeI,
    pub min_size: Option<SizeI>,
    pub fixed_size: bool,
    pub decorations: WindowDecorationMode,
    pub icon: AppIconProfile,
}

impl Default for WindowOptions {
    fn default() -> Self {
        Self {
            title: "Telorgon".to_owned(),
            size: SizeI {
                width: 1280,
                height: 800,
            },
            min_size: Some(SizeI {
                width: 320,
                height: 240,
            }),
            fixed_size: false,
            decorations: WindowDecorationMode::System,
            icon: AppIconProfile::new(),
        }
    }
}

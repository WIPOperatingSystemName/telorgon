use crate::authoring::compose::Signal;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TrayItemId(pub(crate) String);
impl TrayItemId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TrayStatus {
    Passive,
    #[default]
    Active,
    NeedsAttention,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayPixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
}
impl TrayPixels {
    pub fn new(width: u32, height: u32, rgba: impl Into<Arc<[u8]>>) -> Result<Self, TrayError> {
        let rgba = rgba.into();
        if width == 0
            || height == 0
            || width > 1024
            || height > 1024
            || rgba.len() != width as usize * height as usize * 4
        {
            return Err(TrayError::Invalid(
                "invalid tray image dimensions or byte length".into(),
            ));
        }
        Ok(Self {
            width,
            height,
            rgba,
        })
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrayImage {
    pub name: String,
    pub pixels: Option<TrayPixels>,
}
impl TrayImage {
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            pixels: None,
        }
    }
    pub fn pixels(pixels: TrayPixels) -> Self {
        Self {
            name: String::new(),
            pixels: Some(pixels),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayItem {
    pub id: TrayItemId,
    pub title: String,
    pub tooltip: String,
    pub icon: TrayImage,
    pub(crate) image_id: u32,
    pub(crate) image_revision: u64,
    pub status: TrayStatus,
    pub item_is_menu: bool,
    pub has_menu: bool,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TrayCheck {
    #[default]
    None,
    Check(bool),
    Radio(bool),
}
/// IDs are scoped to one menu; DBusMenu reserves zero for the root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayMenuItem {
    pub id: i32,
    pub label: String,
    pub enabled: bool,
    pub visible: bool,
    pub separator: bool,
    pub check: TrayCheck,
    pub icon: String,
    pub shortcut: String,
    pub is_submenu: bool,
    pub children: Vec<TrayMenuItem>,
}
impl TrayMenuItem {
    pub fn action(id: i32, label: impl Into<String>) -> Self {
        Self {
            id,
            label: label.into(),
            enabled: true,
            visible: true,
            separator: false,
            check: TrayCheck::None,
            icon: String::new(),
            shortcut: String::new(),
            is_submenu: false,
            children: vec![],
        }
    }
    pub fn separator(id: i32) -> Self {
        Self {
            separator: true,
            ..Self::action(id, "")
        }
    }
    pub fn checked(mut self, checked: bool) -> Self {
        self.check = TrayCheck::Check(checked);
        self
    }
    pub fn submenu(mut self, children: Vec<Self>) -> Self {
        self.children = children;
        self.is_submenu = true;
        self
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrayMenu {
    pub items: Vec<TrayMenuItem>,
}
impl TrayMenu {
    pub fn new(items: Vec<TrayMenuItem>) -> Result<Self, TrayError> {
        let menu = Self { items };
        menu.validate()?;
        Ok(menu)
    }
    pub fn validate(&self) -> Result<(), TrayError> {
        fn walk(
            items: &[TrayMenuItem],
            depth: usize,
            ids: &mut std::collections::HashSet<i32>,
        ) -> Result<(), TrayError> {
            if depth > 16 {
                return Err(TrayError::Invalid("menu nesting exceeds 16".into()));
            }
            for item in items {
                if item.id == 0
                    || !ids.insert(item.id)
                    || ids.len() > 1024
                    || item.label.len() > 4096
                {
                    return Err(TrayError::Invalid(
                        "invalid, duplicate, or excessive menu items".into(),
                    ));
                }
                walk(&item.children, depth + 1, ids)?;
            }
            Ok(())
        }
        walk(&self.items, 0, &mut Default::default())
    }
    pub fn actionable(&self, id: i32) -> bool {
        fn find(items: &[TrayMenuItem], id: i32) -> bool {
            items
                .iter()
                .filter(|i| i.visible && i.enabled && !i.separator)
                .any(|i| {
                    if i.id == id {
                        !i.is_submenu && i.children.is_empty()
                    } else {
                        find(&i.children, id)
                    }
                })
        }
        find(&self.items, id)
    }
    pub fn find(&self, id: i32) -> Option<&TrayMenuItem> {
        fn find(items: &[TrayMenuItem], id: i32) -> Option<&TrayMenuItem> {
            items.iter().find_map(|i| {
                if i.id == id {
                    Some(i)
                } else {
                    find(&i.children, id)
                }
            })
        }
        find(&self.items, id)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayMenuSnapshot {
    pub owner: TrayItemId,
    pub revision: u64,
    pub menu: TrayMenu,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TraySnapshot {
    pub items: Vec<TrayItem>,
    pub menu: Option<TrayMenuSnapshot>,
    pub connected: bool,
    pub error: Option<String>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WatcherPolicy {
    UseExisting,
    #[default]
    UseExistingOrProvide,
}
#[derive(Clone, Debug, Default)]
pub struct TrayHostConfig {
    pub watcher_policy: WatcherPolicy,
}
impl TrayHostConfig {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn watcher_policy(mut self, value: WatcherPolicy) -> Self {
        self.watcher_policy = value;
        self
    }
}
#[derive(Clone, Debug)]
pub struct TrayIconConfig {
    pub id: String,
    pub title: String,
    pub icon: TrayImage,
    pub tooltip: String,
    pub status: TrayStatus,
    pub menu: TrayMenu,
    pub item_is_menu: bool,
}
impl TrayIconConfig {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: String::new(),
            icon: TrayImage::default(),
            tooltip: String::new(),
            status: TrayStatus::Active,
            menu: TrayMenu::default(),
            item_is_menu: false,
        }
    }
    pub fn title(mut self, v: impl Into<String>) -> Self {
        self.title = v.into();
        self
    }
    pub fn icon(mut self, v: TrayImage) -> Self {
        self.icon = v;
        self
    }
    pub fn tooltip(mut self, v: impl Into<String>) -> Self {
        self.tooltip = v.into();
        self
    }
    pub fn menu(mut self, v: TrayMenu) -> Self {
        self.menu = v;
        self
    }
    pub fn status(mut self, v: TrayStatus) -> Self {
        self.status = v;
        self
    }
    pub fn item_is_menu(mut self, v: bool) -> Self {
        self.item_is_menu = v;
        self
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrayEvent {
    Activated { x: i32, y: i32 },
    SecondaryActivated { x: i32, y: i32 },
    ContextMenu { x: i32, y: i32 },
    Scroll { delta: i32, horizontal: bool },
    MenuSelected { item: i32 },
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrayPublisherState {
    pub host_available: bool,
    pub error: Option<String>,
    pub event_revision: u64,
}
pub type TraySignal = Signal<TraySnapshot>;
#[derive(Debug, Clone, thiserror::Error)]
pub enum TrayError {
    #[error("tray service stopped or command queue is full")]
    Unavailable,
    #[error("{0}")]
    Invalid(String),
    #[error("tray transport: {0}")]
    Transport(String),
}
impl From<zbus::Error> for TrayError {
    fn from(e: zbus::Error) -> Self {
        Self::Transport(e.to_string())
    }
}

impl TrayItem {
    pub fn image_resource(&self) -> Option<crate::graphics::render::ImageResource> {
        let p = self.icon.pixels.as_ref()?;
        Some(crate::graphics::render::ImageResource {
            image: crate::ui::ImageId(self.image_id),
            content_version: self.image_revision.max(1),
            extent: crate::foundation::SizeI {
                width: p.width as i32,
                height: p.height as i32,
            },
            color_encoding: Default::default(),
            alpha_mode: Default::default(),
            pixel_format: Default::default(),
            pixels: p.rgba.clone(),
        })
    }
}

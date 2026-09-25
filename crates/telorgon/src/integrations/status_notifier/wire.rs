use crate::tray::*;
use std::collections::HashMap;
use zbus::{
    blocking::{Connection, Proxy},
    zvariant::{OwnedValue, Value},
};
pub const WATCHER: &str = "org.kde.StatusNotifierWatcher";
pub const WATCHER_PATH: &str = "/StatusNotifierWatcher";
pub const ITEM: &str = "org.kde.StatusNotifierItem";
pub const MENU: &str = "com.canonical.dbusmenu";
pub type Props = HashMap<String, OwnedValue>;
pub type Layout = (i32, Props, Vec<OwnedValue>);
pub fn proxy<'a>(
    c: &'a Connection,
    name: &'a str,
    path: &'a str,
    iface: &'a str,
) -> zbus::Result<Proxy<'a>> {
    zbus::blocking::proxy::Builder::new(c)
        .destination(name)?
        .path(path)?
        .interface(iface)?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
}
pub fn string(p: &Props, key: &str) -> String {
    p.get(key)
        .and_then(|v| <&str>::try_from(v).ok())
        .unwrap_or("")
        .to_string()
}
pub fn boolean(p: &Props, key: &str, fallback: bool) -> bool {
    p.get(key)
        .and_then(|v| bool::try_from(v).ok())
        .unwrap_or(fallback)
}
pub fn integer(p: &Props, key: &str) -> i32 {
    p.get(key).and_then(|v| i32::try_from(v).ok()).unwrap_or(0)
}
pub fn status(s: &str) -> TrayStatus {
    match s {
        "Passive" => TrayStatus::Passive,
        "NeedsAttention" => TrayStatus::NeedsAttention,
        _ => TrayStatus::Active,
    }
}
pub fn status_name(s: TrayStatus) -> &'static str {
    match s {
        TrayStatus::Passive => "Passive",
        TrayStatus::Active => "Active",
        TrayStatus::NeedsAttention => "NeedsAttention",
    }
}
pub fn decode_menu(layout: Layout) -> Result<TrayMenu, TrayError> {
    fn children(
        values: Vec<OwnedValue>,
        depth: usize,
        count: &mut usize,
    ) -> Result<Vec<TrayMenuItem>, TrayError> {
        if depth > 16 {
            return Err(TrayError::Invalid("remote menu too deep".into()));
        }
        let mut result = vec![];
        for value in values {
            *count += 1;
            if *count > 1024 {
                return Err(TrayError::Invalid("remote menu too large".into()));
            }
            let (id, p, nested): Layout = value
                .try_into()
                .map_err(|e: zbus::zvariant::Error| TrayError::Invalid(e.to_string()))?;
            let toggle = integer(&p, "toggle-state") == 1;
            let check = match string(&p, "toggle-type").as_str() {
                "checkmark" => TrayCheck::Check(toggle),
                "radio" => TrayCheck::Radio(toggle),
                _ => TrayCheck::None,
            };
            let shortcuts: Vec<Vec<String>> = p
                .get("shortcut")
                .and_then(|v| v.try_clone().ok())
                .and_then(|v| v.try_into().ok())
                .unwrap_or_default();
            result.push(TrayMenuItem {
                id,
                label: string(&p, "label"),
                enabled: boolean(&p, "enabled", true),
                visible: boolean(&p, "visible", true),
                separator: string(&p, "type") == "separator",
                check,
                icon: string(&p, "icon-name"),
                is_submenu: string(&p, "children-display") == "submenu",
                shortcut: shortcuts.first().map(|s| s.join("+")).unwrap_or_default(),
                children: children(nested, depth + 1, count)?,
            });
        }
        Ok(result)
    }
    TrayMenu::new(children(layout.2, 0, &mut 0)?)
}
pub fn owned<T>(v: T) -> OwnedValue
where
    T: Into<Value<'static>>,
{
    v.into().try_to_owned().expect("owned scalar")
}
pub fn menu_props(item: &TrayMenuItem) -> Props {
    let mut p = HashMap::from([
        ("label".into(), owned(item.label.clone())),
        ("enabled".into(), owned(item.enabled)),
        ("visible".into(), owned(item.visible)),
    ]);
    if item.separator {
        p.insert("type".into(), owned("separator".to_string()));
    }
    if !item.icon.is_empty() {
        p.insert("icon-name".into(), owned(item.icon.clone()));
    }
    if item.is_submenu || !item.children.is_empty() {
        p.insert("children-display".into(), owned("submenu".to_string()));
    }
    let toggle = match item.check {
        TrayCheck::None => None,
        TrayCheck::Check(v) => Some(("checkmark", v)),
        TrayCheck::Radio(v) => Some(("radio", v)),
    };
    if let Some((kind, v)) = toggle {
        p.insert("toggle-type".into(), owned(kind.to_string()));
        p.insert("toggle-state".into(), owned(i32::from(v)));
    }
    p
}
pub fn encode_layout(menu: &TrayMenu, parent: i32, depth: i32) -> Layout {
    fn encode(items: &[TrayMenuItem], depth: i32) -> Vec<OwnedValue> {
        if depth == 0 {
            return vec![];
        }
        items
            .iter()
            .map(|i| {
                Value::new((
                    i.id,
                    menu_props(i),
                    encode(&i.children, depth.saturating_sub(1)),
                ))
                .try_to_owned()
                .expect("owned layout")
            })
            .collect()
    }
    if parent == 0 {
        (0, Props::new(), encode(&menu.items, depth))
    } else if let Some(i) = menu.find(parent) {
        (i.id, menu_props(i), encode(&i.children, depth))
    } else {
        (parent, Props::new(), vec![])
    }
}
pub fn pixmaps(image: &TrayImage) -> Vec<(i32, i32, Vec<u8>)> {
    image
        .pixels
        .as_ref()
        .map(|p| {
            (
                p.width as i32,
                p.height as i32,
                p.rgba
                    .chunks_exact(4)
                    .flat_map(|p| [p[3], p[0], p[1], p[2]])
                    .collect(),
            )
        })
        .into_iter()
        .collect()
}
pub fn decode_pixels(maps: Vec<(i32, i32, Vec<u8>)>) -> Option<TrayPixels> {
    maps.into_iter()
        .filter(|(w, h, b)| {
            *w > 0 && *h > 0 && *w <= 1024 && *h <= 1024 && b.len() == *w as usize * *h as usize * 4
        })
        .min_by_key(|(w, _, _)| (*w - 32).abs())
        .and_then(|(w, h, b)| {
            TrayPixels::new(
                w as u32,
                h as u32,
                b.chunks_exact(4)
                    .flat_map(|p| [p[1], p[2], p[3], p[0]])
                    .collect::<Vec<_>>(),
            )
            .ok()
        })
}

// Electron exports private icon directories (often /tmp/...) instead of IconPixmap.
// Decode on the worker so rewrites at the same path bypass the catalog's name cache.
pub fn private_icon(name: &str, theme_path: &str) -> Option<TrayPixels> {
    use std::path::Path;
    let name_path = Path::new(name);
    let mut candidates = vec![];
    if name_path.is_absolute() {
        candidates.push(name_path.to_owned());
    } else if !name.is_empty() && !name.contains('/') && Path::new(theme_path).is_absolute() {
        for sub in [
            "",
            "32x32/status",
            "32x32/apps",
            "scalable/status",
            "scalable/apps",
        ] {
            for suffix in ["", ".png", ".svg"] {
                candidates.push(
                    Path::new(theme_path)
                        .join(sub)
                        .join(format!("{name}{suffix}")),
                );
            }
        }
    }
    for path in candidates {
        if !path.is_file() {
            continue;
        }
        if let Ok(resource) = crate::authoring::compose::applications::decode_icon(&path, 32) {
            let mut rgba = resource.pixels.to_vec();
            if resource.alpha_mode == crate::graphics::render::ImageAlphaMode::Premultiplied {
                for p in rgba.chunks_exact_mut(4) {
                    if p[3] > 0 {
                        for i in 0..3 {
                            p[i] = (u32::from(p[i]) * 255 / u32::from(p[3])).min(255) as u8;
                        }
                    }
                }
            }
            return TrayPixels::new(
                resource.extent.width as u32,
                resource.extent.height as u32,
                rgba,
            )
            .ok();
        }
    }
    None
}

#![cfg(all(target_os = "linux", feature = "tray-linux"))]
use std::time::{Duration, Instant};
use telorgon::tray::*;
static BUS_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn wait(mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(12);
    while !f() {
        assert!(Instant::now() < deadline, "tray operation timed out");
        std::thread::sleep(Duration::from_millis(30));
    }
}
#[test]
fn publisher_host_menu_and_lifetime() {
    let _guard = BUS_TEST_LOCK.lock().unwrap();
    // Invoke under dbus-run-session with this opt-in; never claims the user's watcher.
    if std::env::var("TELORGON_TRAY_TEST_BUS").as_deref() != Ok("1") {
        return;
    }
    let mut host = TrayHost::connect(TrayHostConfig::new()).unwrap();
    let handle = host.handle();
    let mut icon = TrayIcon::publish(
        TrayIconConfig::new("org.telorgon.TrayTest")
            .title("Tray test")
            .icon(TrayImage::pixels(
                TrayPixels::new(1, 1, vec![12, 34, 56, 255]).unwrap(),
            ))
            .menu(
                TrayMenu::new(vec![
                    TrayMenuItem::action(1, "Pause").checked(false),
                    TrayMenuItem::action(2, "More")
                        .submenu(vec![TrayMenuItem::action(3, "Nested")]),
                ])
                .unwrap(),
            ),
    )
    .unwrap();
    wait(|| {
        handle
            .snapshot()
            .items
            .iter()
            .any(|i| i.title == "Tray test")
    });
    let item = handle
        .snapshot()
        .items
        .into_iter()
        .find(|i| i.title == "Tray test")
        .unwrap();
    assert_eq!(&*item.icon.pixels.unwrap().rgba, &[12, 34, 56, 255]);
    handle.request_menu(item.id.clone()).unwrap();
    wait(|| handle.snapshot().menu.is_some());
    let shown = handle.snapshot().menu.unwrap();
    assert_eq!(shown.menu.find(3).unwrap().label, "Nested");
    let navigation = telorgon::components::shell::tray::TrayMenuNavigation::default();
    navigation.highlight(1, 0);
    assert!(navigation.key(
        &handle,
        &telorgon::input::KeyEvent::new(
            telorgon::input::PhysicalKey::from_code(telorgon::input::PhysicalKeyCode::ArrowRight),
            telorgon::input::ButtonState::Pressed
        )
    ));
    assert!(icon.try_next_event().is_none());
    navigation.activate(&handle, 2, 0).unwrap();
    wait(|| {
        handle
            .snapshot()
            .menu
            .as_ref()
            .is_some_and(|m| m.revision != shown.revision)
    });
    assert_eq!(
        navigation
            .levels(&handle.snapshot().menu.unwrap().menu)
            .len(),
        2
    );
    let shown = handle.snapshot().menu.unwrap();

    handle
        .select_menu_item(item.id.clone(), shown.revision, 1)
        .unwrap();
    wait(|| {
        matches!(
            icon.try_next_event(),
            Some(TrayEvent::MenuSelected { item: 1 })
        )
    });
    icon.set_menu(TrayMenu::new(vec![TrayMenuItem::action(1, "Pause").checked(true)]).unwrap())
        .unwrap();
    handle.request_menu(item.id.clone()).unwrap();
    wait(|| {
        handle
            .snapshot()
            .menu
            .as_ref()
            .is_some_and(|m| m.menu.find(1).unwrap().check == TrayCheck::Check(true))
    });
    handle
        .select_menu_item(item.id.clone(), shown.revision, 1)
        .unwrap();
    wait(|| handle.snapshot().error.is_some());
    assert!(icon.try_next_event().is_none());
    handle.activate(item.id.clone(), 12, 34).unwrap();
    wait(|| {
        matches!(
            icon.try_next_event(),
            Some(TrayEvent::Activated { x: 12, y: 34 })
        )
    });
    icon.set_tooltip("Updated tooltip");
    wait(|| {
        handle
            .snapshot()
            .items
            .iter()
            .any(|i| i.tooltip.contains("Updated tooltip"))
    });
    host.shutdown();
    wait(|| !icon.signal().snapshot().host_available);
    let mut replacement = TrayHost::connect(TrayHostConfig::new()).unwrap();
    wait(|| {
        replacement
            .snapshot()
            .items
            .iter()
            .any(|i| i.title == "Tray test")
    });
    icon.shutdown();
    wait(|| replacement.snapshot().items.is_empty());
    replacement.shutdown();
}
#[test]
fn validates_menu_identity_and_pixels() {
    assert!(TrayPixels::new(1025, 1, vec![]).is_err());
    assert!(
        TrayMenu::new(vec![
            TrayMenuItem::action(1, "A"),
            TrayMenuItem::action(1, "B")
        ])
        .is_err()
    );
    assert!(TrayMenu::new(vec![TrayMenuItem::action(0, "Root collision")]).is_err());
}

struct AyatanaItem;
#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl AyatanaItem {
    #[zbus(property, name = "XAyatanaLabel")]
    fn ayatana_label(&self) -> &str {
        ""
    }
    #[zbus(property)]
    fn menu(&self) -> zbus::zvariant::OwnedObjectPath {
        "/Menu".try_into().unwrap()
    }

    #[zbus(property)]
    fn id(&self) -> &str {
        "ayatana-fixture"
    }
    #[zbus(property)]
    fn title(&self) -> &str {
        "Ayatana fixture"
    }
    #[zbus(property)]
    fn icon_name(&self) -> &str {
        "org.remmina.Remmina-status"
    }
    #[zbus(property)]
    fn status(&self) -> &str {
        "Active"
    }
}
struct AyatanaMenu;
#[zbus::interface(name = "com.canonical.dbusmenu")]
impl AyatanaMenu {
    fn get_layout(
        &self,
        _parent: i32,
        _depth: i32,
        _properties: Vec<String>,
    ) -> (
        u32,
        (
            i32,
            std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
            Vec<zbus::zvariant::OwnedValue>,
        ),
    ) {
        (1, (0, Default::default(), vec![]))
    }
}
#[test]
fn nonstandard_path_and_existing_watcher_coexist() {
    let _guard = BUS_TEST_LOCK.lock().unwrap();
    if std::env::var("TELORGON_TRAY_TEST_BUS").as_deref() != Ok("1") {
        return;
    }
    let mut first = TrayHost::connect(TrayHostConfig::new()).unwrap();
    wait(|| first.snapshot().connected);
    let mut second =
        TrayHost::connect(TrayHostConfig::new().watcher_policy(WatcherPolicy::UseExisting))
            .unwrap();
    wait(|| second.snapshot().connected);
    let c = zbus::blocking::Connection::session().unwrap();
    let path = "/org/ayatana/NotificationItem/test_icon";
    c.object_server().at(path, AyatanaItem).unwrap();
    c.object_server().at("/Menu", AyatanaMenu).unwrap();
    let watcher = zbus::blocking::Proxy::new(
        &c,
        "org.kde.StatusNotifierWatcher",
        "/StatusNotifierWatcher",
        "org.kde.StatusNotifierWatcher",
    )
    .unwrap();
    watcher
        .call::<_, _, ()>("RegisterStatusNotifierItem", &(path,))
        .unwrap();
    wait(|| {
        first
            .snapshot()
            .items
            .iter()
            .any(|i| i.title == "Ayatana fixture")
    });
    wait(|| {
        second
            .snapshot()
            .items
            .iter()
            .any(|i| i.title == "Ayatana fixture")
    });
    let item = first
        .snapshot()
        .items
        .into_iter()
        .find(|i| i.title == "Ayatana fixture")
        .unwrap();
    assert!(item.id.as_str().ends_with(path));
    assert!(item.item_is_menu);
    assert!(item.has_menu);
    first.handle().activate(item.id, 0, 0).unwrap();
    wait(|| first.snapshot().menu.is_some());
    second.shutdown();
    first.shutdown();
}

struct GnomeWatcher {
    item: String,
}
#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl GnomeWatcher {
    fn register_status_notifier_host(&self, _service: String) -> zbus::fdo::Result<()> {
        Err(zbus::fdo::Error::NotSupported(
            "Registering additional notification hosts is not supported".into(),
        ))
    }
    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        vec![self.item.clone()]
    }
    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }
}
#[test]
fn gnome_watcher_allows_discovery_without_additional_registration() {
    let _guard = BUS_TEST_LOCK.lock().unwrap();
    if std::env::var("TELORGON_TRAY_TEST_BUS").as_deref() != Ok("1") {
        return;
    }
    let c = zbus::blocking::Connection::session().unwrap();
    let path = "/org/ayatana/NotificationItem/test_icon";
    c.object_server().at(path, AyatanaItem).unwrap();
    c.object_server().at("/Menu", AyatanaMenu).unwrap();
    c.object_server()
        .at(
            "/StatusNotifierWatcher",
            GnomeWatcher {
                item: format!("{}@{path}", c.unique_name().unwrap()),
            },
        )
        .unwrap();
    c.request_name("org.kde.StatusNotifierWatcher").unwrap();
    let mut host =
        TrayHost::connect(TrayHostConfig::new().watcher_policy(WatcherPolicy::UseExisting))
            .unwrap();
    wait(|| {
        host.snapshot()
            .items
            .iter()
            .any(|i| i.title == "Ayatana fixture")
    });
    assert!(host.snapshot().connected);
    assert!(host.snapshot().error.is_none());
    let item = host.snapshot().items[0].clone();
    assert_eq!(item.icon.name, "org.remmina.Remmina-status");
    host.handle().request_menu(item.id).unwrap();
    wait(|| host.snapshot().menu.is_some());
    host.shutdown();
}

struct ChromiumItem {
    directory: String,
}
#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl ChromiumItem {
    #[zbus(property)]
    fn title(&self) -> &str {
        "Chromium fixture"
    }
    #[zbus(property)]
    fn icon_name(&self) -> &str {
        "status_icon_0"
    }
    #[zbus(property)]
    fn icon_theme_path(&self) -> &str {
        &self.directory
    }
}
#[test]
fn chromium_private_icon_directory_is_decoded() {
    let _guard = BUS_TEST_LOCK.lock().unwrap();
    if std::env::var("TELORGON_TRAY_TEST_BUS").as_deref() != Ok("1") {
        return;
    }
    let directory =
        std::env::temp_dir().join(format!("telorgon-tray-icon-test-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    image::RgbaImage::from_pixel(2, 2, image::Rgba([23, 45, 67, 255]))
        .save(directory.join("status_icon_0.png"))
        .unwrap();
    let c = zbus::blocking::Connection::session().unwrap();
    c.object_server()
        .at(
            "/StatusNotifierItem",
            ChromiumItem {
                directory: directory.to_str().unwrap().into(),
            },
        )
        .unwrap();
    c.object_server()
        .at(
            "/StatusNotifierWatcher",
            GnomeWatcher {
                item: c.unique_name().unwrap().to_string(),
            },
        )
        .unwrap();
    c.request_name("org.kde.StatusNotifierWatcher").unwrap();
    let mut host =
        TrayHost::connect(TrayHostConfig::new().watcher_policy(WatcherPolicy::UseExisting))
            .unwrap();
    wait(|| {
        host.snapshot()
            .items
            .iter()
            .any(|i| i.icon.pixels.is_some())
    });
    let pixels = host.snapshot().items[0].icon.pixels.clone().unwrap();
    assert_eq!(&pixels.rgba[..4], &[23, 45, 67, 255]);
    host.shutdown();
    std::fs::remove_dir_all(directory).unwrap();
}

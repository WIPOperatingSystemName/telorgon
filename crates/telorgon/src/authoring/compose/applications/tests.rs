use super::*;
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "telorgon-catalog-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn file(&self, name: &str, contents: &str) {
        let p = self.0.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, contents).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn precedence_hidden_masks_localization_and_visibility() {
    let f = Fixture::new();
    f.file(
        "system/applications/browser.desktop",
        "[Desktop Entry]\nType=Application\nName=Browser\nExec=browser\n",
    );
    f.file(
        "user/applications/browser.desktop",
        "[Desktop Entry]\nHidden=true\n",
    );
    f.file("system/applications/tools/editor.desktop","[Desktop Entry]\nType=Application\nName=Editor\nName[fr]=Éditeur\nName[fr_CA]=Éditeur canadien\nExec=editor\nKeywords=code;hello\\;world;\nIcon=editor\nStartupWMClass=Editor\n");
    f.file(
        "system/applications/private.desktop",
        "[Desktop Entry]\nType=Application\nName=Private\nExec=private\nNoDisplay=true\n",
    );
    f.file("system/applications/missing.desktop","[Desktop Entry]\nType=Application\nName=Missing\nExec=missing\nTryExec=/nonexistent/telorgon-executable\n");
    let config = ApplicationCatalog::system()
        .data_directories(vec![f.0.join("user"), f.0.join("system")])
        .locale("fr_CA.UTF-8");
    let apps = discover(&config).unwrap();
    assert!(!apps.iter().any(|a| a.id.0 == "browser.desktop"));
    let editor = apps
        .iter()
        .find(|a| a.id.0 == "tools-editor.desktop")
        .unwrap()
        .clone();
    assert_eq!(editor.name, "Éditeur canadien");
    assert_eq!(editor.keywords, vec!["code", "hello;world"]);
    assert_eq!(
        apps.iter()
            .filter(|a| a.visibility == ApplicationVisibility::Visible)
            .count(),
        1
    );
    let (catalog, _worker) = start(ApplicationCatalog::in_memory(apps), 32);
    assert_eq!(
        catalog.search(ApplicationQuery::new("code"))[0].id,
        editor.id.clone()
    );
    assert!(
        catalog
            .get(&ApplicationId::new("private.desktop"))
            .is_some()
    );
    assert_eq!(
        catalog.identify("Editor"),
        Some(ApplicationId::new("tools-editor.desktop"))
    );
    assert!(catalog.identify("unrelated title").is_none());
}
#[test]
fn themed_icons_inherit_decode_and_refresh() {
    let f = Fixture::new();
    f.file("icons/custom/index.theme","[Icon Theme]\nName=Custom\nDirectories=32/apps\nInherits=hicolor\n[32/apps]\nSize=32\nType=Fixed\n");
    f.file("icons/hicolor/index.theme","[Icon Theme]\nName=Fallback\nDirectories=scalable/apps\n[scalable/apps]\nSize=32\nType=Scalable\nMinSize=16\nMaxSize=256\n");
    f.file("icons/hicolor/scalable/apps/editor.svg",r#"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="16"><rect width="32" height="16" fill="red"/></svg>"#);
    let config = ApplicationCatalog::system()
        .data_directories(vec![f.0.clone()])
        .icon_theme("custom");
    assert!(find_icon(&config, "../editor", 32).is_none());
    let path = find_icon(&config, "editor", 64).unwrap();
    let resource = decode_icon(&path, 64).unwrap();
    assert_eq!(resource.extent.width, 64);
    assert_eq!(resource.pixels.len(), 64 * 64 * 4);
    let key = ("editor".into(), 64);
    let mut stamps = BTreeMap::new();
    let mut cache = BTreeMap::new();
    refresh_icon(&config, &key, &mut stamps, &mut cache);
    let id = cache[&key].image;
    refresh_icon(&config, &key, &mut stamps, &mut cache);
    assert_eq!(cache[&key].image, id);
    std::fs::remove_file(path).unwrap();
    refresh_icon(&config, &key, &mut stamps, &mut cache);
    assert!(cache.is_empty());
}
#[test]
fn asynchronous_catalog_and_icon_publication() {
    let f = Fixture::new();
    f.file(
        "applications/demo.desktop",
        "[Desktop Entry]\nType=Application\nName=Demo\nExec=demo\nIcon=demo\n",
    );
    f.file("pixmaps/demo.svg",r#"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"><rect width="32" height="32" fill="blue"/></svg>"#);
    let (catalog, worker) = start(
        ApplicationCatalog::system()
            .data_directories(vec![f.0.clone()])
            .watch_changes(true),
        32,
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while catalog.status() == ApplicationCatalogStatus::Loading {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(catalog.installed().len(), 1);
    let id = ApplicationId::new("demo.desktop");
    assert_eq!(catalog.icon(&id).image_id(), fallback_image().image);
    while catalog.icon(&id).image_id() == fallback_image().image {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(catalog.resources().len(), 1);
    std::fs::remove_file(f.0.join("applications/demo.desktop")).unwrap();
    while !catalog.installed().is_empty() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    drop(worker);
}

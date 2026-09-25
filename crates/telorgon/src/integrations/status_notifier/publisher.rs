use super::wire::*;
use crate::{
    authoring::compose::{Signal, SignalWriter},
    tray::*,
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use zbus::{
    blocking::connection::Builder,
    interface,
    zvariant::{OwnedObjectPath, OwnedValue},
};
struct Shared {
    config: TrayIconConfig,
    revision: u32,
}
struct Item {
    shared: Arc<Mutex<Shared>>,
    events: mpsc::SyncSender<TrayEvent>,
    wake: Arc<AtomicU64>,
}
impl Item {
    fn event(&self, e: TrayEvent) -> zbus::fdo::Result<()> {
        self.events.try_send(e).map_err(|_| {
            zbus::fdo::Error::LimitsExceeded("tray event queue is full or closed".into())
        })?;
        self.wake.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}
#[interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    fn activate(&self, x: i32, y: i32) -> zbus::fdo::Result<()> {
        self.event(TrayEvent::Activated { x, y })
    }
    fn secondary_activate(&self, x: i32, y: i32) -> zbus::fdo::Result<()> {
        self.event(TrayEvent::SecondaryActivated { x, y })
    }
    fn context_menu(&self, x: i32, y: i32) -> zbus::fdo::Result<()> {
        self.event(TrayEvent::ContextMenu { x, y })
    }
    fn scroll(&self, delta: i32, orientation: &str) -> zbus::fdo::Result<()> {
        self.event(TrayEvent::Scroll {
            delta,
            horizontal: orientation == "horizontal",
        })
    }
    #[zbus(signal)]
    async fn new_icon(emitter: &zbus::object_server::SignalEmitter<'_>) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn new_attention_icon(
        emitter: &zbus::object_server::SignalEmitter<'_>,
    ) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn new_title(emitter: &zbus::object_server::SignalEmitter<'_>) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn new_tool_tip(emitter: &zbus::object_server::SignalEmitter<'_>) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn new_status(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        status: &str,
    ) -> zbus::Result<()>;
    #[zbus(property)]
    fn category(&self) -> &str {
        "ApplicationStatus"
    }
    #[zbus(property)]
    fn id(&self) -> String {
        self.shared.lock().unwrap().config.id.clone()
    }
    #[zbus(property)]
    fn title(&self) -> String {
        self.shared.lock().unwrap().config.title.clone()
    }
    #[zbus(property)]
    fn status(&self) -> String {
        status_name(self.shared.lock().unwrap().config.status).into()
    }
    #[zbus(property)]
    fn window_id(&self) -> u32 {
        0
    }
    #[zbus(property)]
    fn icon_name(&self) -> String {
        self.shared.lock().unwrap().config.icon.name.clone()
    }
    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        pixmaps(&self.shared.lock().unwrap().config.icon)
    }
    #[zbus(property)]
    fn attention_icon_name(&self) -> String {
        self.icon_name()
    }
    #[zbus(property)]
    fn attention_icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        self.icon_pixmap()
    }
    #[zbus(property)]
    fn overlay_icon_name(&self) -> &str {
        ""
    }
    #[zbus(property)]
    fn overlay_icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        vec![]
    }
    #[zbus(property)]
    fn icon_theme_path(&self) -> &str {
        ""
    }
    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        self.shared.lock().unwrap().config.item_is_menu
    }
    #[zbus(property)]
    fn menu(&self) -> OwnedObjectPath {
        OwnedObjectPath::try_from("/Menu").unwrap()
    }
    #[zbus(property)]
    fn tool_tip(&self) -> (String, Vec<(i32, i32, Vec<u8>)>, String, String) {
        let s = self.shared.lock().unwrap();
        (
            String::new(),
            vec![],
            s.config.title.clone(),
            s.config.tooltip.clone(),
        )
    }
}
struct Menu {
    shared: Arc<Mutex<Shared>>,
    events: mpsc::SyncSender<TrayEvent>,
    wake: Arc<AtomicU64>,
}
#[interface(name = "com.canonical.dbusmenu")]
impl Menu {
    #[zbus(signal)]
    async fn layout_updated(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        revision: u32,
        parent: i32,
    ) -> zbus::Result<()>;

    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }
    #[zbus(property)]
    fn text_direction(&self) -> &str {
        "ltr"
    }
    #[zbus(property)]
    fn status(&self) -> &str {
        "normal"
    }
    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        vec![]
    }
    fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        _property_names: Vec<String>,
    ) -> (u32, Layout) {
        let s = self.shared.lock().unwrap();
        (
            s.revision,
            encode_layout(&s.config.menu, parent_id, recursion_depth),
        )
    }
    fn get_group_properties(
        &self,
        ids: Vec<i32>,
        _property_names: Vec<String>,
    ) -> Vec<(i32, Props)> {
        let s = self.shared.lock().unwrap();
        ids.into_iter()
            .filter_map(|id| s.config.menu.find(id).map(|i| (id, menu_props(i))))
            .collect()
    }
    fn get_property(&self, id: i32, name: &str) -> zbus::fdo::Result<OwnedValue> {
        self.shared
            .lock()
            .unwrap()
            .config
            .menu
            .find(id)
            .and_then(|i| menu_props(i).remove(name))
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs("unknown menu property".into()))
    }
    fn about_to_show(&self, _id: i32) -> bool {
        false
    }
    fn about_to_show_group(&self, _ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        (vec![], vec![])
    }
    fn event(
        &self,
        id: i32,
        event_id: &str,
        _data: OwnedValue,
        _timestamp: u32,
    ) -> zbus::fdo::Result<()> {
        if event_id != "clicked" {
            return Ok(());
        }
        if !self.shared.lock().unwrap().config.menu.actionable(id) {
            return Err(zbus::fdo::Error::InvalidArgs(
                "menu action unavailable".into(),
            ));
        }
        self.events
            .try_send(TrayEvent::MenuSelected { item: id })
            .map_err(|_| {
                zbus::fdo::Error::LimitsExceeded("tray event queue is full or closed".into())
            })?;
        self.wake.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
    fn event_group(&self, events: Vec<(i32, String, OwnedValue, u32)>) -> Vec<i32> {
        events
            .into_iter()
            .map(|(id, event, data, time)| (id, self.event(id, &event, data, time)))
            .filter_map(|(id, result)| result.err().map(|_| id))
            .collect()
    }
}
pub struct TrayIcon {
    shared: Arc<Mutex<Shared>>,
    events: mpsc::Receiver<TrayEvent>,
    signal: Signal<TrayPublisherState>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl TrayIcon {
    pub fn publish(config: TrayIconConfig) -> Result<Self, TrayError> {
        if config.id.trim().is_empty() || config.id.len() > 255 {
            return Err(TrayError::Invalid(
                "tray ID must be nonempty and at most 255 bytes".into(),
            ));
        }
        config.menu.validate()?;
        let shared = Arc::new(Mutex::new(Shared {
            config,
            revision: 1,
        }));
        let (tx, events) = mpsc::sync_channel(64);
        let (signal, writer) = Signal::new(TrayPublisherState::default());
        let stop = Arc::new(AtomicBool::new(false));
        let quit = stop.clone();
        let state = shared.clone();
        let worker = thread::Builder::new()
            .name("telorgon-tray-icon".into())
            .spawn(move || run(state, tx, writer, quit))
            .map_err(|e| TrayError::Transport(e.to_string()))?;
        Ok(Self {
            shared,
            events,
            signal,
            stop,
            worker: Some(worker),
        })
    }
    pub fn signal(&self) -> Signal<TrayPublisherState> {
        self.signal.clone()
    }
    pub fn try_next_event(&self) -> Option<TrayEvent> {
        self.events.try_recv().ok()
    }
    fn update(&self, f: impl FnOnce(&mut TrayIconConfig)) {
        let mut s = self.shared.lock().unwrap();
        f(&mut s.config);
        s.revision = s.revision.wrapping_add(1);
    }
    pub fn set_icon(&self, image: TrayImage) {
        self.update(|c| c.icon = image);
    }
    pub fn set_tooltip(&self, text: impl Into<String>) {
        self.update(|c| c.tooltip = text.into());
    }
    pub fn set_title(&self, text: impl Into<String>) {
        self.update(|c| c.title = text.into());
    }
    pub fn set_status(&self, status: TrayStatus) {
        self.update(|c| c.status = status);
    }
    pub fn set_menu(&self, menu: TrayMenu) -> Result<(), TrayError> {
        menu.validate()?;
        self.update(|c| c.menu = menu);
        Ok(())
    }
    pub fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}
impl Drop for TrayIcon {
    fn drop(&mut self) {
        self.shutdown();
    }
}
fn run(
    shared: Arc<Mutex<Shared>>,
    tx: mpsc::SyncSender<TrayEvent>,
    writer: SignalWriter<TrayPublisherState>,
    stop: Arc<AtomicBool>,
) {
    static SERIAL: AtomicU64 = AtomicU64::new(1);
    let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
    let wake = Arc::new(AtomicU64::new(0));
    while !stop.load(Ordering::Acquire) {
        let result = (|| -> Result<(), TrayError> {
            let c = Builder::session()?
                .method_timeout(Duration::from_millis(700))
                .build()?;
            let name = format!(
                "org.freedesktop.StatusNotifierItem-{}-{serial}",
                std::process::id()
            );
            c.object_server().at(
                "/StatusNotifierItem",
                Item {
                    shared: shared.clone(),
                    events: tx.clone(),
                    wake: wake.clone(),
                },
            )?;
            c.object_server().at(
                "/Menu",
                Menu {
                    shared: shared.clone(),
                    events: tx.clone(),
                    wake: wake.clone(),
                },
            )?;
            c.request_name(name.as_str())?;
            let bus = proxy(
                &c,
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
            )?;
            let mut last_owner = String::new();
            let mut last_revision = 0;
            let mut ticks = 0;
            let mut available = false;
            while !stop.load(Ordering::Acquire) {
                if ticks % 10 == 0 {
                    let owner: Result<String, _> = bus.call("GetNameOwner", &(WATCHER,));
                    available = false;
                    if let Ok(owner) = owner {
                        let watcher = proxy(&c, WATCHER, WATCHER_PATH, WATCHER)?;
                        if owner != last_owner {
                            watcher.call::<_, _, ()>("RegisterStatusNotifierItem", &(&name,))?;
                            last_owner = owner;
                        }
                        available = watcher
                            .get_property("IsStatusNotifierHostRegistered")
                            .unwrap_or(false);
                    } else {
                        last_owner.clear();
                    }
                }
                let (revision, status) = {
                    let s = shared.lock().unwrap();
                    (s.revision, status_name(s.config.status).to_string())
                };
                if revision != last_revision {
                    for signal in ["NewIcon", "NewAttentionIcon", "NewTitle", "NewToolTip"] {
                        c.emit_signal(None::<&str>, "/StatusNotifierItem", ITEM, signal, &())?;
                    }
                    c.emit_signal(
                        None::<&str>,
                        "/StatusNotifierItem",
                        ITEM,
                        "NewStatus",
                        &(status,),
                    )?;
                    c.emit_signal(
                        None::<&str>,
                        "/Menu",
                        MENU,
                        "LayoutUpdated",
                        &(revision, 0i32),
                    )?;
                    last_revision = revision;
                }
                writer.publish_if_changed(TrayPublisherState {
                    host_available: available,
                    error: None,
                    event_revision: wake.load(Ordering::Relaxed),
                });
                ticks += 1;
                thread::sleep(Duration::from_millis(50));
            }
            Ok(())
        })();
        if let Err(e) = result {
            writer.publish_if_changed(TrayPublisherState {
                error: Some(e.to_string()),
                ..Default::default()
            });
        }
        for _ in 0..10 {
            if stop.load(Ordering::Acquire) {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
    writer.publish_if_changed(TrayPublisherState::default());
}

use super::{
    watcher::{Registry, Watcher, prune},
    wire::*,
};
use crate::{
    authoring::compose::{Signal, SignalWriter},
    tray::*,
};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use zbus::blocking::{Connection, connection::Builder};
#[derive(Clone)]
pub struct TrayHandle {
    signal: TraySignal,
    commands: mpsc::SyncSender<Command>,
}
impl PartialEq for TrayHandle {
    fn eq(&self, other: &Self) -> bool {
        self.signal == other.signal
    }
}
impl TrayHandle {
    pub fn signal(&self) -> TraySignal {
        self.signal.clone()
    }
    pub fn snapshot(&self) -> TraySnapshot {
        (*self.signal.snapshot()).clone()
    }
    fn send(&self, c: Command) -> Result<(), TrayError> {
        self.commands
            .try_send(c)
            .map_err(|_| TrayError::Unavailable)
    }
    pub fn activate(&self, id: TrayItemId, x: i32, y: i32) -> Result<(), TrayError> {
        self.send(Command::Activate(id, x, y))
    }
    pub fn secondary_activate(&self, id: TrayItemId, x: i32, y: i32) -> Result<(), TrayError> {
        self.send(Command::Secondary(id, x, y))
    }
    pub fn scroll(&self, id: TrayItemId, delta: i32, horizontal: bool) -> Result<(), TrayError> {
        self.send(Command::Scroll(id, delta, horizontal))
    }
    pub fn request_menu(&self, id: TrayItemId) -> Result<(), TrayError> {
        self.send(Command::Menu(id, 0))
    }
    pub fn prepare_submenu(&self, id: TrayItemId, parent: i32) -> Result<(), TrayError> {
        self.send(Command::Menu(id, parent))
    }
    pub fn close_menu(&self) -> Result<(), TrayError> {
        self.send(Command::Close)
    }
    pub fn select_menu_item(
        &self,
        id: TrayItemId,
        revision: u64,
        item: i32,
    ) -> Result<(), TrayError> {
        self.send(Command::Select(id, revision, item))
    }
}
pub struct TrayHost {
    handle: TrayHandle,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl TrayHost {
    pub fn connect(config: TrayHostConfig) -> Result<Self, TrayError> {
        let (signal, writer) = Signal::new(TraySnapshot::default());
        let (tx, rx) = mpsc::sync_channel(64);
        let stop = Arc::new(AtomicBool::new(false));
        let quit = stop.clone();
        let worker = thread::Builder::new()
            .name("telorgon-tray-host".into())
            .spawn(move || run(config, rx, writer, quit))
            .map_err(|e| TrayError::Transport(e.to_string()))?;
        Ok(Self {
            handle: TrayHandle {
                signal,
                commands: tx,
            },
            stop,
            worker: Some(worker),
        })
    }
    pub fn handle(&self) -> TrayHandle {
        self.handle.clone()
    }
    pub fn snapshot(&self) -> TraySnapshot {
        self.handle.snapshot()
    }
    pub fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.handle.commands.try_send(Command::Close);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}
impl Drop for TrayHost {
    fn drop(&mut self) {
        self.shutdown();
    }
}
enum Command {
    Activate(TrayItemId, i32, i32),
    Secondary(TrayItemId, i32, i32),
    Scroll(TrayItemId, i32, bool),
    Menu(TrayItemId, i32),
    Select(TrayItemId, u64, i32),
    Close,
}
#[derive(Clone)]
struct Endpoint {
    name: String,
    path: String,
    menu: String,
}
fn endpoint(c: &Connection, key: &str) -> Result<Endpoint, TrayError> {
    let (name, path) = if let Some((n, p)) = key.split_once('@') {
        (n, p.to_string())
    } else if let Some(i) = key.find('/') {
        (&key[..i], key[i..].to_string())
    } else {
        (key, "/StatusNotifierItem".into())
    };
    let bus = proxy(
        c,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )?;
    let owner: String = bus.call("GetNameOwner", &(name,))?;
    Ok(Endpoint {
        name: owner,
        path,
        menu: String::new(),
    })
}
fn read_item(c: &Connection, e: &mut Endpoint) -> Result<TrayItem, TrayError> {
    let properties: Props =
        proxy(c, &e.name, &e.path, "org.freedesktop.DBus.Properties")?.call("GetAll", &(ITEM,))?;
    fn get<T: TryFrom<zbus::zvariant::OwnedValue>>(p: &Props, key: &str) -> Option<T> {
        p.get(key)?.try_clone().ok()?.try_into().ok()
    }
    let mut title = string(&properties, "Title");
    if title.is_empty() {
        title = string(&properties, "Id");
    }
    let state = string(&properties, "Status");
    let attention = state == "NeedsAttention";
    let mut name = string(
        &properties,
        if attention {
            "AttentionIconName"
        } else {
            "IconName"
        },
    );
    if name.is_empty() {
        name = string(&properties, "IconName");
    }
    let maps = get(
        &properties,
        if attention {
            "AttentionIconPixmap"
        } else {
            "IconPixmap"
        },
    )
    .unwrap_or_default();
    let pixels = decode_pixels(maps)
        .or_else(|| decode_pixels(get(&properties, "IconPixmap").unwrap_or_default()))
        .or_else(|| private_icon(&name, &string(&properties, "IconThemePath")));
    let tooltip: (String, Vec<(i32, i32, Vec<u8>)>, String, String) =
        get(&properties, "ToolTip").unwrap_or_default();
    e.menu = get::<zbus::zvariant::OwnedObjectPath>(&properties, "Menu")
        .map(|s| s.to_string())
        .unwrap_or_default();
    Ok(TrayItem {
        image_id: 0,
        image_revision: 0,
        id: TrayItemId(format!("{}{}", e.name, e.path)),
        title,
        tooltip: if tooltip.3.is_empty() {
            tooltip.2
        } else {
            format!("{}\n{}", tooltip.2, tooltip.3)
        },
        icon: TrayImage { name, pixels },
        status: status(&state),
        // Ayatana indicators may omit ItemIsMenu and Activate entirely.
        item_is_menu: boolean(
            &properties,
            "ItemIsMenu",
            properties.contains_key("XAyatanaLabel") && !e.menu.is_empty() && e.menu != "/",
        ),
        has_menu: !e.menu.is_empty() && e.menu != "/",
    })
}
fn menu(c: &Connection, e: &Endpoint, parent: Option<i32>) -> Result<TrayMenu, TrayError> {
    if e.menu.is_empty() || e.menu == "/" {
        return Err(TrayError::Invalid(
            "application does not export a menu".into(),
        ));
    }
    let p = proxy(c, &e.name, &e.menu, MENU)?;
    if let Some(parent) = parent {
        let _: Result<bool, _> = p.call("AboutToShow", &(parent,));
    }
    let (_, layout): (u32, Layout) = p.call("GetLayout", &(0i32, -1i32, Vec::<String>::new()))?;
    decode_menu(layout)
}
fn command(
    c: &Connection,
    cmd: Command,
    ends: &HashMap<TrayItemId, Endpoint>,
    snapshot: &mut TraySnapshot,
    next: &mut u64,
) -> Result<(), TrayError> {
    if matches!(cmd, Command::Close) {
        snapshot.menu = None;
        return Ok(());
    }
    let id = match &cmd {
        Command::Activate(i, ..)
        | Command::Secondary(i, ..)
        | Command::Scroll(i, ..)
        | Command::Menu(i, ..)
        | Command::Select(i, ..) => i,
        Command::Close => unreachable!(),
    };
    let e = ends
        .get(id)
        .ok_or_else(|| TrayError::Invalid("tray item exited".into()))?;
    let p = proxy(c, &e.name, &e.path, ITEM)?;
    match cmd {
        Command::Activate(id, x, y) => {
            if snapshot.items.iter().any(|i| i.id == id && i.item_is_menu) {
                if e.menu.is_empty() || e.menu == "/" {
                    p.call::<_, _, ()>("ContextMenu", &(x, y))?;
                    return Ok(());
                }
                let m = menu(c, e, Some(0))?;
                *next += 1;
                snapshot.menu = Some(TrayMenuSnapshot {
                    owner: id,
                    revision: *next,
                    menu: m,
                });
            } else {
                p.call::<_, _, ()>("Activate", &(x, y))?;
            }
        }
        Command::Secondary(_, x, y) => {
            p.call::<_, _, ()>("SecondaryActivate", &(x, y))?;
        }
        Command::Scroll(_, delta, h) => {
            p.call::<_, _, ()>(
                "Scroll",
                &(delta, if h { "horizontal" } else { "vertical" }),
            )?;
        }
        Command::Menu(id, parent) => {
            if e.menu.is_empty() || e.menu == "/" {
                p.call::<_, _, ()>("ContextMenu", &(0i32, 0i32))?;
                return Ok(());
            }
            let m = menu(c, e, Some(parent))?;
            *next += 1;
            snapshot.menu = Some(TrayMenuSnapshot {
                owner: id,
                revision: *next,
                menu: m,
            });
        }
        Command::Select(id, revision, item) => {
            let shown = snapshot
                .menu
                .as_ref()
                .filter(|m| m.owner == id && m.revision == revision)
                .ok_or_else(|| TrayError::Invalid("menu changed; select again".into()))?;
            let current = menu(c, e, None)?;
            if current != shown.menu {
                *next += 1;
                snapshot.menu = Some(TrayMenuSnapshot {
                    owner: id,
                    revision: *next,
                    menu: current,
                });
                return Err(TrayError::Invalid("menu changed; select again".into()));
            }
            let row = current
                .find(item)
                .filter(|_| current.actionable(item))
                .ok_or_else(|| TrayError::Invalid("menu action unavailable".into()))?;
            proxy(c, &e.name, &e.menu, MENU)?.call::<_, _, ()>(
                "Event",
                &(row.id, "clicked", zbus::zvariant::Value::from(0i32), 0u32),
            )?;
            snapshot.menu = None;
        }
        Command::Close => {}
    }
    Ok(())
}
fn run(
    config: TrayHostConfig,
    rx: mpsc::Receiver<Command>,
    writer: SignalWriter<TraySnapshot>,
    stop: Arc<AtomicBool>,
) {
    let mut snapshot = TraySnapshot::default();
    let mut revision = 0;
    while !stop.load(Ordering::Acquire) {
        let result = session(&config, &rx, &writer, &stop, &mut snapshot, &mut revision);
        snapshot.connected = false;
        snapshot.items.clear();
        snapshot.menu = None;
        snapshot.error = result.err().map(|e| e.to_string());
        writer.publish_if_changed(snapshot.clone());
        if !stop.load(Ordering::Acquire) {
            let _ = rx.recv_timeout(Duration::from_secs(1));
        }
    }
}
fn session(
    config: &TrayHostConfig,
    rx: &mpsc::Receiver<Command>,
    writer: &SignalWriter<TraySnapshot>,
    stop: &AtomicBool,
    snapshot: &mut TraySnapshot,
    revision: &mut u64,
) -> Result<(), TrayError> {
    let c = Builder::session()?
        .method_timeout(Duration::from_millis(700))
        .build()?;
    let bus = proxy(
        &c,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )?;
    let dirty = Arc::new(AtomicBool::new(true));
    let mut subscriptions = Vec::new();
    for interface in [
        ITEM,
        MENU,
        "org.freedesktop.DBus.Properties",
        WATCHER,
        "org.freedesktop.DBus",
    ] {
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface(interface)?
            .build();
        let mut stream = futures_lite::future::block_on(zbus::MessageStream::for_match_rule(
            rule,
            c.inner(),
            Some(64),
        ))?;
        let dirty = dirty.clone();
        subscriptions.push(c.inner().executor().spawn(
            async move {
                use futures_lite::StreamExt;
                while stream.next().await.is_some() {
                    dirty.store(true, Ordering::Release);
                }
            },
            "tray-observation",
        ));
    }
    let registry = Arc::new(Mutex::new(Registry::default()));
    let exists: bool = bus.call("NameHasOwner", &(WATCHER,))?;
    if !exists && config.watcher_policy == WatcherPolicy::UseExistingOrProvide {
        c.object_server()
            .at(WATCHER_PATH, Watcher(registry.clone()))?;
        // Never replace a desktop that acquires the name between our query and request.
        c.request_name_with_flags(WATCHER, zbus::fdo::RequestNameFlags::DoNotQueue.into())?;
    }
    let watcher = proxy(&c, WATCHER, WATCHER_PATH, WATCHER)?;
    static NEXT_HOST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let name = format!(
        "org.kde.StatusNotifierHost.telorgon{}.n{}",
        std::process::id(),
        NEXT_HOST.fetch_add(1, Ordering::Relaxed)
    );
    c.request_name(name.as_str())?;
    match watcher.call::<_, _, ()>("RegisterStatusNotifierHost", &(&name,)) {
        Ok(()) => {}
        // GNOME's AppIndicator watcher serves discovery but deliberately rejects
        // additional host registration. Its existing host keeps publishers active.
        Err(zbus::Error::MethodError(error, _, _))
            if matches!(
                error.as_str(),
                "org.freedesktop.DBus.Error.NotSupported"
                    | "org.freedesktop.DBus.Error.UnknownMethod"
            ) && watcher
                .get_property::<bool>("IsStatusNotifierHostRegistered")
                .unwrap_or(false) => {}
        Err(error) => return Err(error.into()),
    }
    let watcher_owner: String = bus.call("GetNameOwner", &(WATCHER,))?;
    snapshot.error = None;
    let mut ends = HashMap::new();
    let mut refresh = Instant::now();
    while !stop.load(Ordering::Acquire) {
        if refresh.elapsed() >= Duration::from_secs(5)
            || (refresh.elapsed() >= Duration::from_millis(100)
                && dirty.swap(false, Ordering::AcqRel))
            || !snapshot.connected
        {
            let owner: String = bus.call("GetNameOwner", &(WATCHER,))?;
            if owner != watcher_owner {
                return Err(TrayError::Transport("tray watcher restarted".into()));
            }
            prune(&c, &registry);
            let keys: Vec<String> = watcher.get_property("RegisteredStatusNotifierItems")?;
            let mut items = vec![];
            let mut new_ends = HashMap::new();
            for key in keys.into_iter().take(128) {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                if let Ok(mut e) = endpoint(&c, &key) {
                    if let Ok(mut item) = read_item(&c, &mut e) {
                        let previous = snapshot.items.iter().find(|i| i.id == item.id);
                        retain_icon_identity(&mut item, previous);
                        new_ends.insert(item.id.clone(), e);
                        items.push(item);
                    }
                }
            }
            items.sort_by(|a, b| a.id.cmp(&b.id));
            items.dedup_by(|a, b| a.id == b.id);
            snapshot.items = items;
            ends = new_ends;
            snapshot.connected = true;
            if let Some(shown) = &snapshot.menu {
                if let Some(e) = ends.get(&shown.owner) {
                    if let Ok(m) = menu(&c, e, None) {
                        if m != shown.menu {
                            *revision += 1;
                            snapshot.menu = Some(TrayMenuSnapshot {
                                owner: shown.owner.clone(),
                                revision: *revision,
                                menu: m,
                            });
                        }
                    }
                } else {
                    snapshot.menu = None;
                }
            }
            writer.publish_if_changed(snapshot.clone());
            refresh = Instant::now();
        }
        if let Ok(cmd) = rx.recv_timeout(Duration::from_millis(40)) {
            snapshot.error = command(&c, cmd, &ends, snapshot, revision)
                .err()
                .map(|e| e.to_string());
            writer.publish_if_changed(snapshot.clone());
        }
    }
    Ok(())
}


// Pixel hashes describe equality, not ordering. Retained render resources require
// monotonically increasing versions even when an icon returns to earlier pixels.
fn retain_icon_identity(item: &mut TrayItem, previous: Option<&TrayItem>) {
    static NEXT_IMAGE: std::sync::atomic::AtomicU32 =
        std::sync::atomic::AtomicU32::new(0x6a00_0000);
    if let Some(previous) = previous {
        item.image_id = previous.image_id;
        item.image_revision = previous.image_revision
            .checked_add(u64::from(item.icon.pixels != previous.icon.pixels))
            .expect("tray image revision exhausted");
    } else {
        item.image_id = NEXT_IMAGE.fetch_add(1, Ordering::Relaxed);
        item.image_revision = 1;
    }
}

#[cfg(test)]
#[path = "icon_revision_tests.rs"]
mod icon_revision_tests;

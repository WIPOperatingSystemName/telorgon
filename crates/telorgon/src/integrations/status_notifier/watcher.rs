use super::wire::*;
use std::sync::{Arc, Mutex};
use zbus::{interface, message::Header, object_server::SignalEmitter};
#[derive(Default)]
pub struct Registry {
    pub items: Vec<String>,
    pub hosts: Vec<String>,
}
pub struct Watcher(pub Arc<Mutex<Registry>>);
#[interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    async fn register_status_notifier_item(
        &self,
        service: String,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let sender = header
            .sender()
            .ok_or_else(|| zbus::fdo::Error::Failed("missing sender".into()))?
            .to_string();
        let key = if service.starts_with('/') {
            format!("{sender}{service}")
        } else {
            format!("{service}/StatusNotifierItem")
        };
        let added = {
            let mut r = self.0.lock().unwrap();
            if r.items.contains(&key) {
                false
            } else if r.items.len() >= 128 {
                return Err(zbus::fdo::Error::LimitsExceeded(
                    "too many tray items".into(),
                ));
            } else {
                r.items.push(key.clone());
                true
            }
        };
        if added {
            Self::status_notifier_item_registered(&emitter, &key).await?;
        }
        Ok(())
    }
    async fn register_status_notifier_host(
        &self,
        service: String,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        {
            let mut r = self.0.lock().unwrap();
            if !r.hosts.contains(&service) && r.hosts.len() < 128 {
                r.hosts.push(service);
            }
        }
        Self::status_notifier_host_registered(&emitter).await?;
        Ok(())
    }
    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.0.lock().unwrap().items.clone()
    }
    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        !self.0.lock().unwrap().hosts.is_empty()
    }
    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }
    #[zbus(signal)]
    async fn status_notifier_item_registered(
        emitter: &SignalEmitter<'_>,
        item: &str,
    ) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn status_notifier_item_unregistered(
        emitter: &SignalEmitter<'_>,
        item: &str,
    ) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn status_notifier_host_registered(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}
pub fn prune(c: &zbus::blocking::Connection, r: &Arc<Mutex<Registry>>) {
    let Ok(bus) = proxy(
        c,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    ) else {
        return;
    };
    let (items, hosts) = {
        let r = r.lock().unwrap();
        (r.items.clone(), r.hosts.clone())
    };
    for key in items {
        let name = key.split('/').next().unwrap_or("");
        if bus
            .call::<_, _, bool>("NameHasOwner", &(name,))
            .unwrap_or(false)
        {
            continue;
        }
        r.lock().unwrap().items.retain(|i| i != &key);
        let _ = c.emit_signal(
            None::<&str>,
            WATCHER_PATH,
            WATCHER,
            "StatusNotifierItemUnregistered",
            &(key,),
        );
    }
    for name in hosts {
        if !bus
            .call::<_, _, bool>("NameHasOwner", &(&name,))
            .unwrap_or(false)
        {
            r.lock().unwrap().hosts.retain(|h| h != &name);
        }
    }
}

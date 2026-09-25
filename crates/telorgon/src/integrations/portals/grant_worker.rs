//! Bounded off-host grant I/O. Dropped replies cancel queued issuance; an issuance racing
//! cancellation is revoked before the worker accepts another command.
use super::{
    grants::GrantStore,
    restore::{RestoreData, RestoreSource},
};
use async_channel::{Receiver, Sender};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    thread::JoinHandle,
};

type Reply<T> = Sender<Result<T, String>>;
enum Command {
    Reap,
    Manage {
        app: Option<String>,
        reply: Reply<Vec<(String, usize)>>,
        wake: Arc<dyn Fn() + Send + Sync>,
    },
    Resolve {
        hint: RestoreData,
        app: String,
        types: u32,
        multiple: bool,
        reply: Reply<Option<RestoreData>>,
    },
    Issue {
        app: String,
        sources: Vec<RestoreSource>,
        reply: Reply<Delivery>,
        wake: Sender<Command>,
    },
    Revoke {
        app: String,
        grant: Option<String>,
        reply: Reply<usize>,
    },
}
pub(super) struct Delivery {
    pub(super) record: Option<RestoreData>,
    status: Arc<AtomicU8>, // 0 waiting, 1 claimed, 2 cancelled
    wake: Sender<Command>,
}
impl Delivery {
    pub(super) fn claim(mut self) -> RestoreData {
        self.status.store(1, Ordering::Release);
        let record = self.record.take().unwrap();
        let _ = self.wake.try_send(Command::Reap);
        record
    }
}
impl Drop for Delivery {
    fn drop(&mut self) {
        if self.record.is_some() {
            self.status.store(2, Ordering::Release);
            // A full queue already wakes the worker; it reaps before every command.
            let _ = self.wake.try_send(Command::Reap);
        }
    }
}
struct Pending {
    app: String,
    id: String,
    status: Arc<AtomicU8>,
}
fn reap(store: &mut Option<GrantStore>, pending: &mut Vec<Pending>, stopping: bool) {
    pending.retain(|grant| {
        let status = grant.status.load(Ordering::Acquire);
        if status == 0 && !stopping {
            return true;
        }
        if status != 1 {
            if let Some(store) = store {
                if let Err(error) = store.revoke(&grant.app, &grant.id) {
                    eprintln!("telorgon-grants: cancelled issuance cleanup failed: {error}");
                }
            }
        }
        false
    });
}
#[derive(Clone)]
pub(super) struct Client {
    send: Sender<Command>,
}
pub(super) struct Worker {
    pub client: Client,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl Worker {
    /// Starts no filesystem work until a request arrives. The directory is created only
    /// for explicit issuance; lookup of a missing store returns no authorized restoration.
    pub fn start(path: Option<PathBuf>) -> Result<Self, String> {
        let (send, receive) = async_channel::bounded(16);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let thread = std::thread::Builder::new()
            .name("telorgon-grants".into())
            .spawn(move || run(path, receive, stopped))
            .map_err(|e| e.to_string())?;
        Ok(Self {
            client: Client { send },
            stop,
            thread: Some(thread),
        })
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.client.send.close();
        if self
            .thread
            .as_ref()
            .is_some_and(|thread| thread.is_finished())
        {
            let _ = self.thread.take().unwrap().join();
        }
    }
}
impl Client {
    pub fn manage(
        &self,
        app: Option<String>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Receiver<Result<Vec<(String, usize)>, String>>, String> {
        if app.as_ref().is_some_and(|app| {
            app.is_empty() || app.len() > 512 || app.chars().any(char::is_control)
        }) {
            return Err("invalid application identity".into());
        }
        let (reply, receive) = async_channel::bounded(1);
        self.send
            .try_send(Command::Manage { app, reply, wake })
            .map_err(|_| "grant worker unavailable or full")?;
        Ok(receive)
    }
    pub async fn resolve(
        &self,
        hint: RestoreData,
        app: String,
        types: u32,
        multiple: bool,
    ) -> Result<Option<RestoreData>, String> {
        if !hint.valid_for(&app, types, multiple) {
            return Ok(None);
        }
        let (reply, receive) = async_channel::bounded(1);
        self.send
            .try_send(Command::Resolve {
                hint,
                app,
                types,
                multiple,
                reply,
            })
            .map_err(|_| "grant worker unavailable or full")?;
        receive.recv().await.map_err(|_| "grant worker stopped")?
    }
    pub async fn issue(
        &self,
        app: String,
        sources: Vec<RestoreSource>,
    ) -> Result<RestoreData, String> {
        self.prepare(app, sources).await.map(Delivery::claim)
    }
    /// Retain cancellation ownership until the portal can commit its successful response.
    pub async fn prepare(
        &self,
        app: String,
        sources: Vec<RestoreSource>,
    ) -> Result<Delivery, String> {
        // Bound commands before allocating retained data in the worker.
        let candidate = RestoreData {
            app_id: app,
            grant_id: "0".repeat(64),
            sources,
        };
        if !candidate.valid_for(&candidate.app_id, 7, true) {
            return Err("invalid grant sources".into());
        }
        let RestoreData {
            app_id: app,
            sources,
            ..
        } = candidate;
        let (reply, receive) = async_channel::bounded(1);
        self.send
            .try_send(Command::Issue {
                app,
                sources,
                reply,
                wake: self.send.clone(),
            })
            .map_err(|_| "grant worker unavailable or full")?;
        receive.recv().await.map_err(|_| "grant worker stopped")?
    }
    pub async fn revoke(&self, app: String, grant: Option<String>) -> Result<usize, String> {
        if app.is_empty() || app.len() > 512 || grant.as_ref().is_some_and(|id| id.len() != 64) {
            return Err("invalid grant identity".into());
        }
        let (reply, receive) = async_channel::bounded(1);
        self.send
            .try_send(Command::Revoke { app, grant, reply })
            .map_err(|_| "grant worker unavailable or full")?;
        receive.recv().await.map_err(|_| "grant worker stopped")?
    }
}
fn run(path: Option<PathBuf>, receive: Receiver<Command>, stop: Arc<AtomicBool>) {
    use std::os::unix::fs::DirBuilderExt;
    let mut store: Option<GrantStore> = None;
    let mut pending = Vec::new();
    while let Ok(command) = receive.recv_blocking() {
        if stop.load(Ordering::Acquire) {
            break;
        }
        reap(&mut store, &mut pending, false);
        if matches!(command, Command::Reap) {
            continue;
        }
        let (cancelled, create) = match &command {
            Command::Reap => unreachable!(),
            Command::Manage { reply, app, .. } => (app.is_none() && reply.is_closed(), false),
            Command::Resolve { reply, .. } => (reply.is_closed(), false),
            Command::Issue { reply, .. } => (reply.is_closed(), true),
            Command::Revoke { reply, .. } => (reply.is_closed(), false),
        };
        if cancelled {
            continue;
        }
        let ready = (|| -> Result<(), String> {
            if store.is_none() {
                let path = path.as_ref().ok_or("grant storage path unavailable")?;
                if create {
                    std::fs::DirBuilder::new()
                        .recursive(true)
                        .mode(0o700)
                        .create(path)
                        .map_err(|e| e.to_string())?;
                }
                match GrantStore::open(path) {
                    Ok(opened) => store = Some(opened),
                    Err(error) if !create && error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.to_string()),
                }
            }
            Ok(())
        })();
        match command {
            Command::Reap => unreachable!(),
            Command::Manage { app, reply, wake } => {
                let result = ready.and_then(|()| match store.as_mut() {
                    None => Ok(Vec::new()),
                    Some(store) => {
                        if let Some(app) = app {
                            store.revoke_application(&app).map_err(|e| e.to_string())?;
                        }
                        store.applications().map_err(|e| e.to_string())
                    }
                });
                let _ = reply.try_send(result);
                wake();
            }
            Command::Resolve {
                hint,
                app,
                types,
                multiple,
                reply,
            } => {
                let result = ready.map(|()| {
                    store
                        .as_ref()
                        .filter(|s| s.permits(&hint, &app, types, multiple))
                        .map(|_| hint)
                });
                let _ = reply.try_send(result);
            }
            Command::Issue {
                app,
                sources,
                reply,
                wake,
            } => {
                if reply.is_closed() || stop.load(Ordering::Acquire) {
                    continue;
                }
                let result = ready.and_then(|()| {
                    store
                        .as_mut()
                        .ok_or_else(|| "grant storage unavailable".to_owned())?
                        .issue(&app, sources)
                        .map_err(|e| e.to_string())
                });
                let result = result.map(|record| {
                    let status = Arc::new(AtomicU8::new(0));
                    pending.push(Pending {
                        app,
                        id: record.grant_id.clone(),
                        status: status.clone(),
                    });
                    Delivery {
                        record: Some(record),
                        status,
                        wake,
                    }
                });
                if !stop.load(Ordering::Acquire) {
                    let _ = reply.try_send(result);
                }
                reap(&mut store, &mut pending, false);
            }
            Command::Revoke { app, grant, reply } => {
                let result = ready.and_then(|()| match store.as_mut() {
                    None => Ok(0),
                    Some(store) => match grant {
                        Some(id) => store.revoke(&app, &id).map(usize::from),
                        None => store.revoke_application(&app),
                    }
                    .map_err(|e| e.to_string()),
                });
                let _ = reply.try_send(result);
            }
        }
    }
    reap(&mut store, &mut pending, true);
}

pub(super) fn default_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/state")))?;
    base.is_absolute()
        .then(|| base.join("telorgon/capture-grants"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_lite::future::block_on;
    use std::sync::atomic::AtomicU64;
    static NEXT: AtomicU64 = AtomicU64::new(1);
    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!(
                "telorgon-grant-worker-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            )))
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn sources() -> Vec<RestoreSource> {
        vec![RestoreSource {
            kind: 1,
            identity: "host-display".into(),
        }]
    }
    fn join(mut worker: Worker) {
        worker.stop.store(true, Ordering::Release);
        worker.client.send.close();
        worker.thread.take().unwrap().join().unwrap();
    }
    #[test]
    fn settings_inventory_and_application_revocation_are_completed_off_host() {
        block_on(async {
            let dir = Directory::new();
            let worker = Worker::start(Some(dir.0.clone())).unwrap();
            let wake = Arc::new(|| {});
            let empty = worker
                .client
                .manage(None, wake.clone())
                .unwrap()
                .recv()
                .await
                .unwrap()
                .unwrap();
            assert!(empty.is_empty());
            assert!(!dir.0.exists());
            let a = worker
                .client
                .issue("app.a".into(), sources())
                .await
                .unwrap();
            worker
                .client
                .issue("app.a".into(), sources())
                .await
                .unwrap();
            let b = worker
                .client
                .issue("app.b".into(), sources())
                .await
                .unwrap();
            let inventory = worker
                .client
                .manage(None, wake.clone())
                .unwrap()
                .recv()
                .await
                .unwrap()
                .unwrap();
            assert_eq!(inventory, vec![("app.a".into(), 2), ("app.b".into(), 1)]);
            let after = worker
                .client
                .manage(Some("app.a".into()), wake)
                .unwrap()
                .recv()
                .await
                .unwrap()
                .unwrap();
            assert_eq!(after, vec![("app.b".into(), 1)]);
            assert!(
                worker
                    .client
                    .resolve(a, "app.a".into(), 1, false)
                    .await
                    .unwrap()
                    .is_none()
            );
            assert!(
                worker
                    .client
                    .resolve(b, "app.b".into(), 1, false)
                    .await
                    .unwrap()
                    .is_some()
            );
            join(worker);
        });
    }

    #[test]
    fn worker_persists_resolves_and_revokes_without_creating_storage_for_lookup() {
        block_on(async {
            let dir = Directory::new();
            let worker = Worker::start(Some(dir.0.clone())).unwrap();
            let hint = RestoreData {
                app_id: "app".into(),
                grant_id: "0".repeat(64),
                sources: sources(),
            };
            assert!(
                worker
                    .client
                    .resolve(hint, "app".into(), 1, false)
                    .await
                    .unwrap()
                    .is_none()
            );
            assert!(!dir.0.exists());
            let grant = worker.client.issue("app".into(), sources()).await.unwrap();
            join(worker);
            let worker = Worker::start(Some(dir.0.clone())).unwrap();
            assert_eq!(
                worker
                    .client
                    .resolve(grant.clone(), "app".into(), 1, false)
                    .await
                    .unwrap(),
                Some(grant.clone())
            );
            assert!(
                worker
                    .client
                    .resolve(grant.clone(), "other".into(), 1, false)
                    .await
                    .unwrap()
                    .is_none()
            );
            assert_eq!(
                worker
                    .client
                    .revoke("app".into(), Some(grant.grant_id.clone()))
                    .await
                    .unwrap(),
                1
            );
            assert!(
                worker
                    .client
                    .resolve(grant, "app".into(), 1, false)
                    .await
                    .unwrap()
                    .is_none()
            );
            join(worker);
        });
    }
    #[test]
    fn dropping_a_written_but_unclaimed_reply_revokes_it_before_next_lookup() {
        let dir = Directory::new();
        let worker = Worker::start(Some(dir.0.clone())).unwrap();
        let (reply, receive) = async_channel::bounded(1);
        worker
            .client
            .send
            .try_send(Command::Issue {
                app: "app".into(),
                sources: sources(),
                reply,
                wake: worker.client.send.clone(),
            })
            .unwrap();
        let delivered = receive.recv_blocking().unwrap().unwrap();
        let record = delivered.record.as_ref().unwrap().clone();
        // The record has reached disk and the reply channel, but the future has not claimed it.
        drop(delivered);
        assert!(
            block_on(
                worker
                    .client
                    .resolve(record.clone(), "app".into(), 1, false)
            )
            .unwrap()
            .is_none()
        );
        join(worker);
        assert!(
            !GrantStore::open(&dir.0)
                .unwrap()
                .permits(&record, "app", 1, false)
        );
    }

    #[test]
    fn cancelled_queued_issuance_never_creates_a_grant() {
        let dir = Directory::new();
        let (send, receive) = async_channel::bounded(1);
        let (reply, dropped) = async_channel::bounded(1);
        drop(dropped);
        send.try_send(Command::Issue {
            app: "app".into(),
            sources: sources(),
            reply,
            wake: send.clone(),
        })
        .unwrap();
        send.close();
        run(
            Some(dir.0.clone()),
            receive,
            Arc::new(AtomicBool::new(false)),
        );
        assert!(!dir.0.exists());
    }
}

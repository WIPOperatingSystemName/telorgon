use super::*;
use futures_lite::StreamExt;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Changes {
    dirty: bool,
    removed: Vec<String>,
    overflow: bool,
}
pub(super) struct Subscription {
    stop: async_channel::Sender<()>,
    changes: Arc<Mutex<Changes>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Subscription {
    pub fn start(connection: &Connection, owner: &str) -> Result<Self, NetworkError> {
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(owner)
            .map_err(classify)?
            .path_namespace(ROOT)
            .map_err(classify)?
            .build();
        let mut stream = futures_lite::future::block_on(zbus::MessageStream::for_match_rule(
            rule,
            connection.inner(),
            Some(256),
        ))
        .map_err(classify)?;
        let changes = Arc::new(Mutex::new(Changes::default()));
        let shared = changes.clone();
        let (stop, receiver) = async_channel::bounded(1);
        let thread = std::thread::Builder::new()
            .name("telorgon-network-events".into())
            .spawn(move || {
                futures_lite::future::block_on(async {
                    loop {
                        let event = futures_lite::future::race(
                            async { Some(stream.next().await) },
                            async {
                                let _ = receiver.recv().await;
                                None
                            },
                        )
                        .await;
                        let Some(Some(Ok(message))) = event else {
                            break;
                        };
                        let header = message.header();
                        let mut state = shared.lock().unwrap_or_else(|e| e.into_inner());
                        state.dirty = true;
                        let member = header.member().map(|m| m.as_str()).unwrap_or("");
                        let path = if matches!(
                            member,
                            "DeviceRemoved" | "AccessPointRemoved" | "ConnectionRemoved"
                        ) {
                            message
                                .body()
                                .deserialize::<OwnedObjectPath>()
                                .ok()
                                .map(|p| p.to_string())
                        } else if member == "Removed" {
                            header.path().map(|p| p.to_string())
                        } else {
                            None
                        };
                        if let Some(path) = path {
                            if state.removed.len() < 256 {
                                state.removed.push(path);
                            } else {
                                state.overflow = true;
                            }
                        }
                    }
                });
                shared.lock().unwrap_or_else(|e| e.into_inner()).dirty = true;
            })
            .map_err(|_| NetworkError::Unavailable)?;
        Ok(Self {
            stop,
            changes,
            thread: Some(thread),
        })
    }
    pub fn drain(&self) -> (bool, Vec<String>, bool) {
        let mut state = self.changes.lock().unwrap_or_else(|e| e.into_inner());
        let old = std::mem::take(&mut *state);
        (old.dirty, old.removed, old.overflow)
    }
}
impl Drop for Subscription {
    fn drop(&mut self) {
        let _ = self.stop.try_send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

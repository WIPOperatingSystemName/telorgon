//! Host-owned clipboard requests. Content never enters diagnostic metadata.
use crate::platform::contracts::{ClipboardKind, DataFormat};
use std::{
    collections::VecDeque,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Waker},
};

mod content;
mod contracts;
pub use content::{ClipboardContent, ClipboardProvider};
pub use contracts::{ClipboardContracts, ClipboardReadResult};
pub(crate) mod stream;
mod text;
pub use stream::ClipboardStream;
pub(crate) use stream::ReadResponse;
#[cfg(all(feature = "shell-xwayland", target_os = "linux"))]
pub(crate) mod x11;
pub use text::{ClipboardEditAction, ClipboardText, PasteTarget};

pub const MAX_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_REQUESTS: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipboardError {
    Unavailable,
    Unsupported,
    Empty,
    Stale,
    Denied,
    Cancelled,
    TooLarge,
    Busy,
    Timeout,
    InvalidText,
    InvalidFormat,
    TransferFailed,
}
impl std::fmt::Display for ClipboardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "clipboard: {self:?}")
    }
}
impl std::error::Error for ClipboardError {}
pub type Result<T> = std::result::Result<T, ClipboardError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardSnapshot {
    pub kind: ClipboardKind,
    pub revision: u64,
    pub formats: Vec<DataFormat>,
}

struct Reply<T> {
    value: Option<Result<T>>,
    waker: Option<Waker>,
}
pub struct ClipboardRequest<T> {
    reply: Arc<Mutex<Reply<T>>>,
    cancel: Arc<AtomicBool>,
}
impl<T> ClipboardRequest<T> {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
    pub fn try_take(&mut self) -> Option<Result<T>> {
        self.reply.lock().unwrap().value.take()
    }
}
impl<T> Future for ClipboardRequest<T> {
    type Output = Result<T>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut reply = self.reply.lock().unwrap();
        if let Some(value) = reply.value.take() {
            Poll::Ready(value)
        } else {
            reply.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}
impl<T> Drop for ClipboardRequest<T> {
    fn drop(&mut self) {
        self.cancel();
    }
}
pub(crate) struct Completion<T> {
    reply: Arc<Mutex<Reply<T>>>,
    pub cancel: Arc<AtomicBool>,
}
impl<T> Completion<T> {
    pub fn finish(self, value: Result<T>) {
        let wake = {
            let mut reply = self.reply.lock().unwrap();
            reply.value = Some(if self.cancel.load(Ordering::Acquire) {
                Err(ClipboardError::Cancelled)
            } else {
                value
            });
            reply.waker.take()
        };
        if let Some(waker) = wake {
            waker.wake();
        }
    }
}
pub(crate) fn channel<T>() -> (ClipboardRequest<T>, Completion<T>) {
    let reply = Arc::new(Mutex::new(Reply {
        value: None,
        waker: None,
    }));
    let cancel = Arc::new(AtomicBool::new(false));
    (
        ClipboardRequest {
            reply: reply.clone(),
            cancel: cancel.clone(),
        },
        Completion { reply, cancel },
    )
}
pub(crate) enum Command {
    Publish(ClipboardKind, ClipboardContent, Option<u64>, Completion<()>),
    Clear(ClipboardKind, Option<u64>, Completion<()>),
    Read(ClipboardSnapshot, DataFormat, usize, ReadResponse),
}
impl Command {
    pub fn fail(self, error: ClipboardError) {
        match self {
            Self::Publish(_, _, _, reply) | Self::Clear(_, _, reply) => reply.finish(Err(error)),
            Self::Read(_, _, _, reply) => reply.finish(Err(error)),
        }
    }
}
struct State {
    alive: bool,
    locked: bool,
    snapshots: [ClipboardSnapshot; 2],
    queue: VecDeque<Command>,
    watchers: Vec<(
        std::sync::Weak<()>,
        std::sync::mpsc::SyncSender<ClipboardSnapshot>,
    )>,
}
pub struct ClipboardChanges {
    receiver: std::sync::mpsc::Receiver<ClipboardSnapshot>,
    _alive: Arc<()>,
}
impl std::ops::Deref for ClipboardChanges {
    type Target = std::sync::mpsc::Receiver<ClipboardSnapshot>;
    fn deref(&self) -> &Self::Target {
        &self.receiver
    }
}
impl State {
    fn notify(&mut self, snapshot: ClipboardSnapshot) {
        self.watchers.retain(|(alive, sender)| {
            alive.strong_count() != 0
                && match sender.try_send(snapshot.clone()) {
                    Ok(()) | Err(std::sync::mpsc::TrySendError::Full(_)) => true,
                    Err(_) => false,
                }
        });
    }
}
#[derive(Clone)]
pub struct Clipboard {
    state: Arc<Mutex<State>>,
    wake: Arc<dyn Fn() + Send + Sync>,
}
pub(crate) struct ClipboardHost {
    pub handle: Clipboard,
}
pub(crate) fn index(kind: ClipboardKind) -> usize {
    usize::from(kind == ClipboardKind::Selection)
}
impl ClipboardHost {
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        Self {
            handle: Clipboard {
                wake,
                state: Arc::new(Mutex::new(State {
                    alive: true,
                    locked: false,
                    queue: VecDeque::new(),
                    watchers: Vec::new(),
                    snapshots: [ClipboardKind::System, ClipboardKind::Selection].map(|kind| {
                        ClipboardSnapshot {
                            kind,
                            revision: 1,
                            formats: vec![],
                        }
                    }),
                })),
            },
        }
    }
    pub fn drain(&self, locked: bool) -> Vec<Command> {
        let mut state = self.handle.state.lock().unwrap();
        let unlocked = state.locked && !locked;
        state.locked = locked;
        if unlocked {
            for snapshot in state.snapshots.clone() {
                state.notify(snapshot);
            }
        }
        state.queue.drain(..).collect()
    }
    pub fn observe(&self, snapshot: ClipboardSnapshot) {
        let mut state = self.handle.state.lock().unwrap();
        let slot = index(snapshot.kind);
        if state.snapshots[slot] == snapshot {
            return;
        }
        state.snapshots[slot] = snapshot.clone();
        if state.locked {
            return;
        }
        state.notify(snapshot);
    }
}
impl Drop for ClipboardHost {
    fn drop(&mut self) {
        let commands = {
            let mut state = self.handle.state.lock().unwrap();
            state.alive = false;
            state.watchers.clear();
            state.queue.drain(..).collect::<Vec<_>>()
        };
        for command in commands {
            command.fail(ClipboardError::Unavailable);
        }
    }
}
impl Clipboard {
    pub fn snapshot(&self, kind: ClipboardKind) -> Result<ClipboardSnapshot> {
        let state = self.state.lock().unwrap();
        if !state.alive {
            return Err(ClipboardError::Unavailable);
        }
        if state.locked {
            return Err(ClipboardError::Denied);
        }
        Ok(state.snapshots[index(kind)].clone())
    }
    /// Bounded change notifications; consumers should re-query the latest snapshot.
    pub fn subscribe(&self) -> Result<ClipboardChanges> {
        let mut state = self.state.lock().unwrap();
        if !state.alive {
            return Err(ClipboardError::Unavailable);
        }
        if state.locked {
            return Err(ClipboardError::Denied);
        }
        state
            .watchers
            .retain(|(alive, _)| alive.strong_count() != 0);
        if state.watchers.len() >= 64 {
            return Err(ClipboardError::Busy);
        }
        let (tx, rx) = std::sync::mpsc::sync_channel(8);
        let alive = Arc::new(());
        for snapshot in state.snapshots.clone() {
            let _ = tx.try_send(snapshot);
        }
        state.watchers.push((Arc::downgrade(&alive), tx));
        Ok(ClipboardChanges {
            receiver: rx,
            _alive: alive,
        })
    }
    fn enqueue(&self, command: Command) {
        let mut state = self.state.lock().unwrap();
        let error = if !state.alive {
            Some(ClipboardError::Unavailable)
        } else if state.locked {
            Some(ClipboardError::Denied)
        } else if state.queue.len() >= MAX_REQUESTS {
            Some(ClipboardError::Busy)
        } else {
            None
        };
        if let Some(error) = error {
            drop(state);
            command.fail(error);
            return;
        }
        state.queue.push_back(command);
        drop(state);
        (self.wake)();
    }
    pub fn publish(
        &self,
        kind: ClipboardKind,
        content: ClipboardContent,
        expected: Option<u64>,
    ) -> ClipboardRequest<()> {
        let (request, reply) = channel();
        self.enqueue(Command::Publish(kind, content, expected, reply));
        request
    }
    pub fn clear(&self, kind: ClipboardKind, expected: Option<u64>) -> ClipboardRequest<()> {
        let (request, reply) = channel();
        self.enqueue(Command::Clear(kind, expected, reply));
        request
    }
    pub fn read(
        &self,
        snapshot: ClipboardSnapshot,
        format: DataFormat,
        max_bytes: usize,
    ) -> ClipboardRequest<Vec<u8>> {
        let (request, reply) = channel();
        if max_bytes == 0 || max_bytes > MAX_BYTES {
            reply.finish(Err(ClipboardError::TooLarge));
        } else if !snapshot.formats.contains(&format) {
            reply.finish(Err(ClipboardError::InvalidFormat));
        } else {
            self.enqueue(Command::Read(
                snapshot,
                format,
                max_bytes,
                ReadResponse::Buffered(reply),
            ));
        }
        request
    }
    pub fn read_stream(
        &self,
        snapshot: ClipboardSnapshot,
        format: DataFormat,
        max_bytes: usize,
        chunk_size: usize,
    ) -> Result<ClipboardStream> {
        if max_bytes == 0 || max_bytes > MAX_BYTES || chunk_size == 0 || chunk_size > 65536 {
            return Err(ClipboardError::TooLarge);
        }
        if !snapshot.formats.contains(&format) {
            return Err(ClipboardError::InvalidFormat);
        }
        let (stream, sink) = stream::stream(chunk_size);
        self.enqueue(Command::Read(
            snapshot,
            format,
            max_bytes,
            ReadResponse::Streamed(sink),
        ));
        Ok(stream)
    }
    pub async fn write_text(&self, text: impl Into<String>) -> Result<()> {
        self.publish(
            ClipboardKind::System,
            ClipboardContent::text(text.into())?,
            None,
        )
        .await
    }
    pub async fn read_text(&self) -> Result<Option<String>> {
        self.read_text_from(ClipboardKind::System).await
    }
    pub async fn read_text_from(&self, kind: ClipboardKind) -> Result<Option<String>> {
        let snapshot = self.snapshot(kind)?;
        if snapshot.formats.is_empty() {
            return Ok(None);
        }
        let format = ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"]
            .iter()
            .find_map(|mime| snapshot.formats.iter().find(|f| f.identifier() == *mime))
            .cloned()
            .ok_or(ClipboardError::InvalidFormat)?;
        let bytes = self.read(snapshot, format, MAX_BYTES).await?;
        String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| ClipboardError::InvalidText)
    }
}

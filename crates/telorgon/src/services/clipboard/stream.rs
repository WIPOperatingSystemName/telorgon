use super::*;
use std::{
    collections::VecDeque,
    sync::Condvar,
    time::{Duration, Instant},
};
struct StreamState {
    chunks: VecDeque<Vec<u8>>,
    result: Option<Result<()>>,
    waker: Option<Waker>,
}
pub struct ClipboardStream {
    ended: bool,
    state: Arc<(Mutex<StreamState>, Condvar)>,
    cancel: Arc<AtomicBool>,
}
pub(crate) struct StreamSink {
    state: Arc<(Mutex<StreamState>, Condvar)>,
    pub cancel: Arc<AtomicBool>,
    pub chunk_size: usize,
}
pub(crate) fn stream(chunk_size: usize) -> (ClipboardStream, StreamSink) {
    let state = Arc::new((
        Mutex::new(StreamState {
            chunks: VecDeque::new(),
            result: None,
            waker: None,
        }),
        Condvar::new(),
    ));
    let cancel = Arc::new(AtomicBool::new(false));
    (
        ClipboardStream {
            ended: false,
            state: state.clone(),
            cancel: cancel.clone(),
        },
        StreamSink {
            state,
            cancel,
            chunk_size,
        },
    )
}
impl ClipboardStream {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
        self.state.1.notify_all();
    }
    pub(crate) fn cancellation(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }
    pub async fn next(&mut self) -> Option<Result<Vec<u8>>> {
        if self.ended {
            return None;
        }
        futures_lite::future::poll_fn(|cx| {
            let mut state = self.state.0.lock().unwrap();
            if let Some(chunk) = state.chunks.pop_front() {
                self.state.1.notify_one();
                return Poll::Ready(Some(Ok(chunk)));
            }
            if let Some(result) = &state.result {
                self.ended = true;
                return Poll::Ready(result.clone().err().map(Err));
            }
            state.waker = Some(cx.waker().clone());
            Poll::Pending
        })
        .await
    }
}
impl Drop for ClipboardStream {
    fn drop(&mut self) {
        self.cancel();
    }
}
impl StreamSink {
    pub fn chunk(&self, bytes: &[u8]) -> Result<()> {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut state = self.state.0.lock().unwrap();
        while state.chunks.len() >= 4 {
            if self.cancel.load(Ordering::Acquire) {
                return Err(ClipboardError::Cancelled);
            }
            if Instant::now() >= deadline {
                return Err(ClipboardError::Timeout);
            }
            state = self
                .state
                .1
                .wait_timeout(state, Duration::from_millis(50))
                .unwrap()
                .0;
        }
        state.chunks.push_back(bytes.to_vec());
        let wake = state.waker.take();
        drop(state);
        if let Some(wake) = wake {
            wake.wake();
        }
        Ok(())
    }
    fn finish(self, result: Result<()>) {
        let mut state = self.state.0.lock().unwrap();
        state.result = Some(result);
        let wake = state.waker.take();
        drop(state);
        if let Some(wake) = wake {
            wake.wake();
        }
    }
}
pub(crate) enum ReadResponse {
    Buffered(Completion<Vec<u8>>),
    Streamed(StreamSink),
}
impl ReadResponse {
    pub fn cancel(&self) -> Arc<AtomicBool> {
        match self {
            Self::Buffered(reply) => reply.cancel.clone(),
            Self::Streamed(sink) => sink.cancel.clone(),
        }
    }
    pub fn chunk_size(&self) -> usize {
        match self {
            Self::Buffered(_) => 65536,
            Self::Streamed(sink) => sink.chunk_size,
        }
    }
    pub fn chunk(&self, bytes: &[u8], buffer: &mut Vec<u8>) -> Result<()> {
        match self {
            Self::Buffered(_) => {
                buffer.extend_from_slice(bytes);
                Ok(())
            }
            Self::Streamed(sink) => sink.chunk(bytes),
        }
    }
    pub fn finish(self, result: Result<Vec<u8>>) {
        match self {
            Self::Buffered(reply) => reply.finish(result),
            Self::Streamed(sink) => sink.finish(result.map(|_| ())),
        }
    }
}

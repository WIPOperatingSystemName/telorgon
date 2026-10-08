use crate::boot::{BootResult, PreviewController};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub(super) struct PreviewClock {
    stop: mpsc::Sender<()>,
    worker: Option<JoinHandle<()>>,
}

impl PreviewClock {
    pub fn start(controller: PreviewController) -> BootResult<Self> {
        let (stop, receive) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("boot-preview-clock".into())
            .spawn(move || {
                let mut previous = Instant::now();
                while matches!(
                    receive.recv_timeout(Duration::from_millis(33)),
                    Err(mpsc::RecvTimeoutError::Timeout)
                ) {
                    let now = Instant::now();
                    let delta = now.saturating_duration_since(previous);
                    previous = now;
                    controller.advance(delta);
                }
            })?;
        Ok(Self {
            stop,
            worker: Some(worker),
        })
    }
}

impl Drop for PreviewClock {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

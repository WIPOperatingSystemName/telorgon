//! Timeline readiness and capture delivery have separate owners. Slow readback/client
//! copies must not keep an already-completed scanout frame behind a capture request.
use super::*;

pub(super) struct VulkanCompletionWorker {
    requests: Option<mpsc::Sender<VulkanCompletionRequest>>,
    completions: mpsc::Receiver<VulkanCompletion>,
    pub(super) wake: OwnedFd,
    thread: Option<thread::JoinHandle<()>>,
    delivery: Option<thread::JoinHandle<()>>,
}
struct Ready {
    request: VulkanCompletionRequest,
    completed_at: Instant,
    result: Result<(), String>,
    pending: bool,
}

fn deliver(mut ready: Ready, tx: &mpsc::Sender<VulkanCompletion>, wake: &OwnedFd) -> bool {
    let request = &mut ready.request;
    if ready.result.is_ok()
        && let Some(capture) = &mut request.capture
    {
        ready.result = capture
            .buffer
            .slot
            .try_finish(&mut request.receipt, &mut capture.buffer.pixels)
            .map_err(|error| error.to_string())
            .and_then(|done| {
                if done {
                    Ok(())
                } else {
                    Err("capture did not complete with its submission".into())
                }
            });
    }
    let direct = if !ready.pending {
        request.capture.as_mut().and_then(|capture| {
            capture.direct.take().map(|(job, time, transform)| {
                if ready.result.is_ok() {
                    job.write(&capture.buffer.pixels, time, transform)
                } else {
                    job.fail()
                }
            })
        })
    } else {
        None
    };
    let request = ready.request;
    if tx
        .send(VulkanCompletion {
            timing: request.receipt.timing(),
            completed_at: ready.completed_at,
            submitted_at: request.submitted_at,
            direct,
            capture_pending: ready.pending.then_some(request.receipt),
            capture: request.capture,
            slot_index: request.slot_index,
            result: ready.result,
            dma_bufs: request.dma_bufs,
        })
        .is_err()
    {
        return false;
    }
    let value = 1_u64;
    let _ = unsafe {
        crate::platform::linux::ffi::write(
            wake.as_raw_fd(),
            std::ptr::from_ref(&value).cast(),
            std::mem::size_of::<u64>(),
        )
    };
    true
}

fn collect_ready<T, R>(
    pending: &mut Vec<(T, Instant)>,
    mut poll: impl FnMut(&mut T, Instant) -> Option<R>,
) -> Vec<(T, R)> {
    let mut ready = Vec::new();
    let mut index = 0;
    while index < pending.len() {
        let (request, since) = &mut pending[index];
        if let Some(result) = poll(request, *since) {
            ready.push((pending.remove(index).0, result));
        } else {
            index += 1;
        }
    }
    ready
}

// Poll all outstanding receipts, not just the oldest message. The single graphics
// timeline orders GPU work; delivery order and CPU capture work need not do so.
fn run(
    rx: mpsc::Receiver<VulkanCompletionRequest>,
    tx: mpsc::Sender<VulkanCompletion>,
    delivery: mpsc::Sender<Ready>,
    wake: OwnedFd,
) {
    let mut pending: Vec<(VulkanCompletionRequest, Instant)> = Vec::new();
    let mut connected = true;
    while connected || !pending.is_empty() {
        if connected {
            let next = if pending.is_empty() {
                rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected)
            } else {
                rx.recv_timeout(Duration::from_millis(1))
            };
            match next {
                Ok(request) => pending.push((request, Instant::now())),
                Err(mpsc::RecvTimeoutError::Disconnected) => connected = false,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            for request in rx.try_iter() {
                pending.push((request, Instant::now()));
            }
        } else if !pending.is_empty() {
            thread::sleep(Duration::from_millis(1));
        }
        let ready = collect_ready(&mut pending, |request, since| {
            let result = match request.receipt.poll() {
                Ok(true) => Ok(()),
                Ok(false) if since.elapsed() < Duration::from_secs(2) => return None,
                // Preserve the existing timeout/retry contract and exact capture receipt.
                Ok(false) => request.receipt.wait(Duration::ZERO),
                Err(error) => Err(error),
            };
            let retry = request.capture.is_some()
                && result.as_ref().is_err_and(|error| {
                    error.kind() != crate::graphics::render::RenderErrorKind::DeviceLost
                });
            Some((
                Instant::now(),
                result.map_err(|error| error.to_string()),
                retry,
            ))
        });
        for (request, (completed_at, result, pending)) in ready {
            let ready = Ready {
                request,
                completed_at,
                result,
                pending,
            };
            if ready.request.capture.is_some() {
                if delivery.send(ready).is_err() {
                    return;
                }
            } else if !deliver(ready, &tx, &wake) {
                return;
            }
        }
    }
}

impl VulkanCompletionWorker {
    pub(super) fn new() -> AppResult<Self> {
        let raw = unsafe {
            crate::platform::linux::ffi::eventfd(
                0,
                crate::platform::linux::ffi::EFD_CLOEXEC
                    | crate::platform::linux::ffi::EFD_NONBLOCK,
            )
        };
        if raw < 0 {
            return Err(AppError::new("failed to create Vulkan completion eventfd"));
        }
        let wake = unsafe { OwnedFd::from_raw_fd(raw) };
        let ready_wake = wake.try_clone().map_err(app_error)?;
        let delivery_wake = wake.try_clone().map_err(app_error)?;
        let (request_tx, request_rx) = mpsc::channel();
        let (completion_tx, completion_rx) = mpsc::channel();
        let (delivery_tx, delivery_rx) = mpsc::channel();
        let delivery_completions = completion_tx.clone();
        let delivery = thread::Builder::new()
            .name("telorgon-capture-delivery".into())
            .spawn(move || {
                while let Ok(ready) = delivery_rx.recv() {
                    if !deliver(ready, &delivery_completions, &delivery_wake) {
                        break;
                    }
                }
            })
            .map_err(app_error)?;
        let thread = match thread::Builder::new()
            .name("telorgon-vulkan-completion".into())
            .spawn(move || run(request_rx, completion_tx, delivery_tx, ready_wake))
        {
            Ok(thread) => thread,
            Err(error) => {
                let _ = delivery.join();
                return Err(app_error(error));
            }
        };
        Ok(Self {
            requests: Some(request_tx),
            completions: completion_rx,
            wake,
            thread: Some(thread),
            delivery: Some(delivery),
        })
    }
    pub(super) fn event_fd(&self) -> i32 {
        self.wake.as_raw_fd()
    }
    pub(super) fn submit(
        &self,
        slot_index: usize,
        receipt: SubmissionReceipt,
        dma_bufs: Vec<DmaBufRetirement>,
    ) -> AppResult<()> {
        self.send(VulkanCompletionRequest {
            submitted_at: Instant::now(),
            capture: None,
            slot_index,
            receipt,
            dma_bufs,
        })
    }
    pub(super) fn submit_capture(
        &self,
        receipt: SubmissionReceipt,
        capture: CaptureJob,
    ) -> AppResult<()> {
        self.send(VulkanCompletionRequest {
            submitted_at: Instant::now(),
            capture: Some(capture),
            slot_index: 0,
            receipt,
            dma_bufs: Vec::new(),
        })
    }
    fn send(&self, request: VulkanCompletionRequest) -> AppResult<()> {
        self.requests
            .as_ref()
            .ok_or_else(|| AppError::new("Vulkan completion worker is stopped"))?
            .send(request)
            .map_err(|_| AppError::new("Vulkan completion worker stopped unexpectedly"))
    }
    pub(super) fn drain(&self) -> Vec<VulkanCompletion> {
        let mut value = 0_u64;
        loop {
            let count = unsafe {
                crate::platform::linux::ffi::read(
                    self.wake.as_raw_fd(),
                    std::ptr::from_mut(&mut value).cast(),
                    std::mem::size_of::<u64>(),
                )
            };
            if count != std::mem::size_of::<u64>() as isize {
                break;
            }
        }
        self.completions.try_iter().collect()
    }
}
impl Drop for VulkanCompletionWorker {
    fn drop(&mut self) {
        self.requests.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        if let Some(thread) = self.delivery.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completion_scan_keeps_pending_receipts_and_retires_ready_work_once() {
        let now = Instant::now();
        let mut pending = vec![(1, now), (2, now), (3, now)];
        let ready = collect_ready(&mut pending, |id, _| (*id != 1).then_some("ready"));
        assert_eq!(ready, vec![(2, "ready"), (3, "ready")]);
        assert_eq!(pending, vec![(1, now)]);
        assert!(collect_ready::<_, ()>(&mut pending, |_, _| None).is_empty());
        assert_eq!(
            collect_ready(&mut pending, |_, since| Some(since)),
            vec![(1, now)]
        );
        assert!(pending.is_empty());
    }
}

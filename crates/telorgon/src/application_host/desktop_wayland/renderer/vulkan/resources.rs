//! Host-owned allocation worker and bounded spare storage. No worker submits GPU work.
use super::*;

pub(super) struct MaterializationResources {
    pub target: VulkanMaterializationTarget,
    pub source: VulkanScene,
    pub geometry: Option<MaterializationGeometry>,
    pub revision: Option<u64>,
}

pub(super) struct TargetAllocator {
    requests: Option<mpsc::SyncSender<(SizeI, bool)>>,
    results:
        mpsc::Receiver<Result<(VulkanMaterializationTarget, Vec<(&'static str, u64)>), String>>,
    pending: bool,
    thread: Option<thread::JoinHandle<()>>,
}

impl TargetAllocator {
    pub fn new(device: VulkanDevice, wake: OwnedFd) -> AppResult<Self> {
        let (tx, rx) = mpsc::sync_channel::<(SizeI, bool)>(1);
        let (done, results) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("telorgon-texture-allocation".into())
            .spawn(move || {
                while let Ok((extent, measure)) = rx.recv() {
                    let mut timings = Vec::new();
                    let mut current: Option<(&'static str, std::time::Instant)> = None;
                    let mut phase = |name| {
                        if measure {
                            let now = std::time::Instant::now();
                            if let Some((previous, start)) = current.replace((name, now)) {
                                timings.push((
                                    previous,
                                    now.duration_since(start).as_micros().min(u64::MAX as u128)
                                        as u64,
                                ));
                            }
                        }
                    };
                    let target =
                        VulkanMaterializationTarget::new_traced(&device, extent, &mut phase);
                    phase("done");
                    let result = target
                        .map(|target| (target, timings))
                        .map_err(|error| error.to_string());
                    if done.send(result).is_err() {
                        break;
                    }
                    let value = 1_u64;
                    unsafe {
                        crate::platform_linux::ffi::write(
                            wake.as_raw_fd(),
                            std::ptr::from_ref(&value).cast(),
                            8,
                        );
                    }
                }
            })
            .map_err(app_error)?;
        Ok(Self {
            requests: Some(tx),
            results,
            pending: false,
            thread: Some(thread),
        })
    }

    pub fn request(&mut self, extent: SizeI, measure: bool) -> AppResult<()> {
        if !self.pending {
            self.requests
                .as_ref()
                .expect("live allocator")
                .try_send((extent, measure))
                .map_err(app_error)?;
            self.pending = true;
        }
        Ok(())
    }

    pub fn take(
        &mut self,
        trace: &mut super::super::super::latency_trace::LatencyTrace,
    ) -> AppResult<Option<VulkanMaterializationTarget>> {
        match self.results.try_recv() {
            Ok(result) => {
                self.pending = false;
                let (target, timings) = result.map_err(AppError::new)?;
                for (name, us) in timings {
                    trace.event(name, [us, target.allocated_bytes(), 0, 0]);
                }
                trace.event(
                    "dmabuf_target_allocated",
                    [target.allocated_bytes(), 0, 0, 0],
                );
                Ok(Some(target))
            }
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => {
                Err(AppError::new("texture allocation worker stopped"))
            }
        }
    }
}

impl Drop for TargetAllocator {
    fn drop(&mut self) {
        self.requests.take();
        // At most one outstanding result; dropping the receiver before join is unnecessary:
        // the channel has room for that result even when shutdown interrupts allocation.
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(super) const SPARE_BYTES: u64 = 128 * 1024 * 1024;
pub(super) const SPARE_COUNT: usize = 8;

pub(super) fn trim_spares(
    spares: &mut VecDeque<MaterializationResources>,
    retirement: &mut RetirementWorker,
) {
    retirement.flush();
    while spares.len() > SPARE_COUNT
        || spares
            .iter()
            .map(|s| s.target.allocated_bytes())
            .sum::<u64>()
            > SPARE_BYTES
    {
        retirement.retire(spares.pop_front().expect("nonempty spare pool"));
    }
}

/// Own a final target pin until all scene/submission pins end, then free off the input owner.
/// The channel is bounded; a saturated channel leaves ownership in the device-budgeted backlog.
pub(super) trait Retirable: Send + 'static {
    fn ready_to_destroy(&self) -> bool;
}
impl Retirable for MaterializationResources {
    fn ready_to_destroy(&self) -> bool {
        self.target.can_recycle()
    }
}

pub(super) struct RetirementWorker<R: Retirable = MaterializationResources> {
    sender: Option<mpsc::SyncSender<R>>,
    backlog: VecDeque<R>,
    thread: Option<thread::JoinHandle<()>>,
}
impl<R: Retirable> RetirementWorker<R> {
    pub fn new() -> AppResult<Self> {
        let (sender, receiver) = mpsc::sync_channel::<R>(64);
        let thread = thread::Builder::new()
            .name("telorgon-texture-retirement".into())
            .spawn(move || {
                let mut held = Vec::<R>::new();
                loop {
                    let next = if held.is_empty() {
                        receiver
                            .recv()
                            .map_err(|_| mpsc::RecvTimeoutError::Disconnected)
                    } else {
                        receiver.recv_timeout(Duration::from_millis(10))
                    };
                    match next {
                        Ok(resource) => held.push(resource),
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                    // Dropping here can call the driver and allocator; never do it on the owner.
                    held.retain(|resource| !resource.ready_to_destroy());
                }
            })
            .map_err(app_error)?;
        Ok(Self {
            sender: Some(sender),
            backlog: VecDeque::new(),
            thread: Some(thread),
        })
    }
    pub fn retire(&mut self, resource: R) {
        self.backlog.push_back(resource);
        self.flush();
    }
    pub fn flush(&mut self) {
        while let Some(resource) = self.backlog.pop_front() {
            match self
                .sender
                .as_ref()
                .expect("live retirement worker")
                .try_send(resource)
            {
                Ok(()) => {}
                Err(mpsc::TrySendError::Full(resource))
                | Err(mpsc::TrySendError::Disconnected(resource)) => {
                    self.backlog.push_front(resource);
                    break;
                }
            }
        }
    }
}
impl<R: Retirable> Drop for RetirementWorker<R> {
    fn drop(&mut self) {
        // Host field order joins submitted work and drops scene pins before this shutdown path.
        if let Some(sender) = self.sender.take() {
            for resource in self.backlog.drain(..) {
                let _ = sender.send(resource);
            }
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod retirement_tests {
    use super::*;
    struct Tracked(mpsc::Sender<thread::ThreadId>);
    impl Drop for Tracked {
        fn drop(&mut self) {
            self.0.send(thread::current().id()).unwrap();
        }
    }
    struct Resource(std::sync::Arc<Tracked>);
    impl Retirable for Resource {
        fn ready_to_destroy(&self) -> bool {
            std::sync::Arc::strong_count(&self.0) == 1
        }
    }
    #[test]
    fn final_pin_and_destruction_stay_on_retirement_worker() {
        let (sender, deleted) = mpsc::channel();
        let owner_pin = std::sync::Arc::new(Tracked(sender));
        let mut worker = RetirementWorker::<Resource>::new().unwrap();
        worker.retire(Resource(owner_pin.clone()));
        assert!(deleted.recv_timeout(Duration::from_millis(30)).is_err());
        drop(owner_pin);
        let destructor_thread = deleted.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_ne!(destructor_thread, thread::current().id());
        drop(worker);
    }
}

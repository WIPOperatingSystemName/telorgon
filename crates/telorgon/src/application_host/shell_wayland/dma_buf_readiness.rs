//! Event-driven admission of client generations. GPU acquire waits remain mandatory.
use super::*;

pub(super) struct AcquireWatch {
    // Remove the native callback before freeing its data or closing its descriptor.
    source: Option<crate::wayland_server::EventSource>,
    wake: Box<AtomicBool>,
    fd: Option<OwnedFd>,
}

impl AcquireWatch {
    pub(super) fn new(
        display: &crate::wayland_server::Display,
        fd: Option<&OwnedFd>,
    ) -> AppResult<Self> {
        let mut watch = Self {
            source: None,
            wake: Box::new(AtomicBool::new(false)),
            fd: fd.map(OwnedFd::try_clone).transpose().map_err(app_error)?,
        };
        if !watch.ready()? {
            // SAFETY: the watch owns the FD and stable callback allocation; source drops first.
            watch.source = Some(
                unsafe {
                    display.event_loop().add_fd(
                        watch.fd.as_ref().unwrap().as_raw_fd(),
                        crate::wayland_server::ffi::WL_EVENT_READABLE
                            | crate::wayland_server::ffi::WL_EVENT_ERROR
                            | crate::wayland_server::ffi::WL_EVENT_HANGUP,
                        Some(mark_external_fd_ready),
                        std::ptr::from_ref(watch.wake.as_ref()).cast_mut().cast(),
                    )
                }
                .map_err(app_error)?,
            );
        }
        Ok(watch)
    }

    pub(super) fn take_wakeup(&mut self) -> bool {
        let wake = self.wake.swap(false, Ordering::AcqRel);
        if wake {
            self.source.take();
        }
        wake
    }

    pub(super) fn ready(&mut self) -> AppResult<bool> {
        if self.source.is_some() {
            return Ok(false);
        }
        let Some(fd) = &self.fd else { return Ok(true) };
        if !fd_ready(fd)? {
            return Ok(false);
        }
        self.source.take();
        self.fd.take();
        Ok(true)
    }
}

fn fd_ready(fd: &OwnedFd) -> AppResult<bool> {
    let mut poll = libc::pollfd {
        fd: fd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    // A zero timeout never waits for the producer on the input owner thread.
    loop {
        if unsafe { libc::poll(&mut poll, 1, 0) } >= 0 {
            break;
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(app_error(error));
        }
    }
    if poll.revents & (libc::POLLERR | libc::POLLNVAL | libc::POLLHUP) != 0 {
        return Err(AppError::new("client acquire fence failed"));
    }
    Ok(poll.revents & libc::POLLIN != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocked_child_defers_its_family_but_not_an_unrelated_window() {
        let ids = [1, 2, 3].map(|n| WaylandSurfaceId::from_raw(n).unwrap());
        let make = || {
            super::super::client::maximize_preview_tests::test_window(
                SizeI {
                    width: 100,
                    height: 80,
                },
                PointI::default(),
            )
        };
        let mut windows = BTreeMap::from([(ids[0], make()), (ids[1], make()), (ids[2], make())]);
        windows.get_mut(&ids[1]).unwrap().parent = Some(ids[0]);
        let eligible = ids.into_iter().collect();
        let mut ready = [ids[0], ids[2]].into_iter().collect();
        retain_ready_families(&windows, &eligible, &mut ready);
        assert_eq!(ready, [ids[2]].into_iter().collect());
        ready = eligible.clone();
        retain_ready_families(&windows, &eligible, &mut ready);
        assert_eq!(ready, eligible);
    }

    #[test]
    fn event_watch_wakes_once_and_can_be_removed_before_signaling() {
        let display = Display::new().unwrap();
        let producer = EventNotifier::new("acquire watch test").unwrap();
        let fd = unsafe { OwnedFd::from_raw_fd(libc::dup(producer.event_fd())) };
        let mut watch = AcquireWatch::new(&display, Some(&fd)).unwrap();
        assert!(!watch.ready().unwrap());
        producer.notify();
        display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
        assert!(watch.take_wakeup());
        assert!(watch.ready().unwrap());
        display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
        assert!(!watch.take_wakeup());
        producer.drain();
        drop(AcquireWatch::new(&display, Some(&fd)).unwrap());
        producer.notify();
        display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    }

    #[test]
    fn acquire_readiness_is_nonblocking_and_independent() {
        let delayed = EventNotifier::new("delayed test producer").unwrap();
        let ready = EventNotifier::new("ready test producer").unwrap();
        let delayed_fd = unsafe { OwnedFd::from_raw_fd(libc::dup(delayed.event_fd())) };
        let ready_fd = unsafe { OwnedFd::from_raw_fd(libc::dup(ready.event_fd())) };
        ready.notify();
        assert!(!fd_ready(&delayed_fd).unwrap());
        assert!(fd_ready(&ready_fd).unwrap());
        delayed.notify();
        assert!(fd_ready(&delayed_fd).unwrap());
    }
}

/// Conservative family admission also keeps synchronized subsurfaces with their parent.
/// A delayed family never gates unrelated desktop roots; non-GPU content shares this gate.
pub(super) fn retain_ready_families(
    windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
    eligible: &std::collections::BTreeSet<WaylandSurfaceId>,
    ready: &mut std::collections::BTreeSet<WaylandSurfaceId>,
) {
    let root = |surface: WaylandSurfaceId| {
        let mut current = surface;
        for _ in 0..windows.len() {
            let Some(parent) = windows.get(&current).and_then(|w| w.parent) else {
                return current;
            };
            current = parent;
        }
        current
    };
    let blocked = eligible
        .difference(ready)
        .map(|s| root(*s))
        .collect::<std::collections::BTreeSet<_>>();
    if !blocked.is_empty() {
        ready.retain(|surface| !blocked.contains(&root(*surface)));
    }
}

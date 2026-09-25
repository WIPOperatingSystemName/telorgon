use super::wire_tests::{bind, registry, send, words};
use super::*;
use std::{os::unix::net::UnixStream, time::Duration};

#[test]
fn frame_only_commit_completes_callback_without_another_buffer_release() {
    check_frame_callback_lifetime(false);
}

#[test]
fn output_tick_paces_new_callbacks_without_presenting_their_revision() {
    check_frame_callback_lifetime(true);
}

fn check_frame_callback_lifetime(pace_before_presentation: bool) {
    let display = Display::new().unwrap();
    let (mut peer, socket) = UnixStream::pair().unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let connection = display.create_client(socket).unwrap();
    let mut native = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
    let observations = Rc::new(RefCell::new(Vec::new()));
    let sink = observations.clone();
    native.set_timing_observer(Some(Box::new(move |event| sink.borrow_mut().push(event))));
    let globals = registry(&display, &mut peer);
    bind(&mut peer, &globals, "wl_compositor", 4);
    send(&mut peer, 4, 0, &words(&[5]));
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    let client = native.state.clients[&connection.identity().unwrap()];
    let surface = native.core().world.client_surfaces(client)[0];
    let buffer = WaylandBufferId::from_raw(100).unwrap();
    native
        .core_mut()
        .register_buffer(
            client,
            buffer,
            BufferDescriptor::Shm(ShmBuffer {
                offset: 0,
                size: crate::foundation::SizeI {
                    width: 16,
                    height: 8,
                },
                stride: 64,
                format: ShmFormat::Argb8888,
            }),
        )
        .unwrap();
    let client_ref = unsafe {
        ClientRef::from_raw(connection.identity().unwrap() as *mut ffi::wl_client).unwrap()
    };
    native
        .state
        .create_resource(
            client_ref,
            client,
            "wl_buffer",
            1,
            6,
            ResourceKind::Buffer(buffer),
            true,
        )
        .unwrap();
    if pace_before_presentation { bind(&mut peer, &globals, "wp_presentation", 7); }
    // First copy/release, then the Firefox frame-only sequence, then legitimate reuse.
    for (index, attach) in [true, false, false, true].into_iter().enumerate() {
        if attach {
            send(&mut peer, 5, 1, &words(&[6, 0, 0]));
        }
        let frame_callback = if pace_before_presentation { 8 + index as u32 * 3 } else { 7 + index as u32 * 2 };
        send(&mut peer, 5, 3, &words(&[frame_callback]));
        if pace_before_presentation {
            send(&mut peer, 7, 1, &words(&[5, frame_callback + 1]));
        }
        send(&mut peer, 5, 6, &[]);
        display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
        let actions = native.core_mut().drain_actions().collect::<Vec<_>>();
        assert!(actions.contains(&if attach {
            CompositorAction::PublishSurface(surface)
        } else {
            CompositorAction::UpdateSurface(surface)
        }));
        assert!(native.core_mut().take_superseded_publications().is_empty());
        // Match the host contract: only an image publication owns a buffer to release.
        for action in actions {
            if action == CompositorAction::PublishSurface(surface) {
                native.release_buffer(buffer).unwrap();
            }
        }
        let revision = native
            .core()
            .world
            .surface(surface)
            .unwrap()
            .snapshot()
            .revision;
        if pace_before_presentation {
            assert!(native.surface_frame_ready(surface, revision + 1, 100).is_err());
            native.surface_frame_ready(surface, revision, 100).unwrap();
            native.surface_frame_ready(surface, revision, 100).unwrap();
            // A refresh of older content must not complete the newer presentation feedback.
            native.surface_presented(surface, revision - 1, 100).unwrap();
            assert!(native.state.committed_presentation_feedbacks.contains_key(&(surface, revision)));
        } else {
            native.surface_presented(surface, revision, 100).unwrap();
        }
        let sync = frame_callback + if pace_before_presentation { 2 } else { 1 };
        send(&mut peer, 1, 0, &words(&[sync]));
        display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
        let (mut releases, mut frames) = (0, 0);
        loop {
            let mut header = [0; 8];
            peer.read_exact(&mut header).unwrap();
            let object = u32::from_ne_bytes(header[..4].try_into().unwrap());
            let word = u32::from_ne_bytes(header[4..].try_into().unwrap());
            let mut body = vec![0; (word >> 16) as usize - 8];
            peer.read_exact(&mut body).unwrap();
            assert!(
                !(object == 1 && word & 0xffff == 0),
                "unexpected protocol error"
            );
            if pace_before_presentation { assert_ne!(object, frame_callback + 1, "pacing must not emit presentation feedback"); }
            if object == sync {
                break;
            }
            if object == 6 && word & 0xffff == 0 {
                releases += 1;
            }
            if object == frame_callback {
                frames += 1;
            }
        }
        assert_eq!(releases, usize::from(attach));
        assert_eq!(
            frames, 1,
            "state-only commits must still complete frame callbacks"
        );
        assert!(connection.is_alive());
        let events = observations.borrow();
        assert_eq!(events.len(), (index + 1) * 2);
        assert!(matches!(events[index * 2], TimingEvent::CommitRequested {
            surface: id, attaches_buffer, callbacks: 1, ..
        } if id == surface.get() && attaches_buffer == attach));
        assert!(matches!(events[index * 2 + 1], TimingEvent::CallbacksQueued {
            surface: id, revision: observed_revision, count: 1, presented
        } if id == surface.get() && observed_revision == revision && presented != pace_before_presentation));
        drop(events);
        if pace_before_presentation {
            native.surface_presented(surface, revision, 101).unwrap();
            assert!(!native.state.committed_presentation_feedbacks.contains_key(&(surface, revision)));
            // Flush/read this feedback with the next loop's roundtrip (a distinct object ID).
        }
    }
}

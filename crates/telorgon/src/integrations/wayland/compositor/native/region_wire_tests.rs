use super::wire_tests::{bind, registry, send, words};
use super::*;
use std::{os::unix::net::UnixStream, time::Duration};

#[test]
fn empty_region_updates_keep_client_alive_and_preserve_surface_regions() {
    let display = Display::new().unwrap();
    let (mut peer, socket) = UnixStream::pair().unwrap();
    let client = display.create_client(socket).unwrap();
    let native = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
    let globals = registry(&display, &mut peer);
    bind(&mut peer, &globals, "wl_compositor", 4);
    send(&mut peer, 4, 0, &words(&[5])); // surface
    send(&mut peer, 4, 1, &words(&[6])); // region
    send(&mut peer, 6, 1, &words(&[0, 0, 100, 80]));
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();

    for opcode in [1, 2] {
        // add and subtract
        for (width, height) in [(0, 80), (100, 0), (0, 0), (-1i32, 80), (100, -1)] {
            send(
                &mut peer,
                6,
                opcode,
                &words(&[10, 20, width as u32, height as u32]),
            );
            display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
            assert!(
                client.is_alive(),
                "region opcode {opcode}, {width}x{height} disconnected client"
            );
        }
    }
    // A subsequent real subtraction must still work, and surfaces copy the region.
    send(&mut peer, 6, 2, &words(&[0, 0, 25, 80]));
    send(&mut peer, 5, 4, &words(&[6])); // opaque region
    send(&mut peer, 5, 5, &words(&[6])); // input region
    send(&mut peer, 6, 0, &[]); // destroy source region
    send(&mut peer, 5, 6, &[]); // commit
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    assert!(client.is_alive());
    let client_id = native.state.clients[&client.identity().unwrap()];
    let surface = native.core().world.client_surfaces(client_id)[0];
    let snapshot = native.core().world.surface(surface).unwrap().snapshot();
    let expected = [RectI {
        x: 25,
        y: 0,
        width: 75,
        height: 80,
    }];
    assert_eq!(
        snapshot.input_region.as_ref().unwrap().rectangles(),
        expected
    );
    assert_eq!(
        snapshot.opaque_region.as_ref().unwrap().rectangles(),
        expected
    );
}

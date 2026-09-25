use super::wire_tests::{bind, registry, send, words};
use super::*;
use std::{os::unix::net::UnixStream, time::Duration};

fn events(display: &Display, peer: &mut UnixStream, callback: u32) -> Vec<(u16, u32)> {
    send(peer, 1, 0, &words(&[callback]));
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    let mut events = Vec::new();
    loop {
        let mut header = [0; 8];
        peer.read_exact(&mut header).unwrap();
        let object = u32::from_ne_bytes(header[..4].try_into().unwrap());
        let word = u32::from_ne_bytes(header[4..].try_into().unwrap());
        let mut body = vec![0; (word >> 16) as usize - 8];
        peer.read_exact(&mut body).unwrap();
        if object == callback {
            return events;
        }
        if object == 6 {
            events.push((
                word as u16,
                u32::from_ne_bytes(body[..4].try_into().unwrap()),
            ));
        }
    }
}
#[test]
fn explicit_membership_balances_bindings_and_survives_unmap() {
    use crate::integrations::wayland::compositor::{
        OutputDescription, OutputMode, OutputState, OutputTransform,
    };
    let display = Display::new().unwrap();
    let (mut peer, socket) = UnixStream::pair().unwrap();
    let client = display.create_client(socket).unwrap();
    let mut native = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
    for id in 1..=2 {
        native
            .add_output(
                &display,
                id,
                OutputState::new(
                    OutputDescription {
                        name: format!("TEST-{id}"),
                        description: "Synthetic output".into(),
                        make: "Test".into(),
                        model: "Display".into(),
                        physical_millimeters: Default::default(),
                        logical_position: PointI::default(),
                        scale: crate::platform::contracts::ScaleFactor::new(1.0).unwrap(),
                        transform: OutputTransform::Normal,
                        modes: vec![OutputMode {
                            size: crate::foundation::SizeI {
                                width: 800,
                                height: 600,
                            },
                            refresh_millihertz: 60000,
                            preferred: true,
                        }],
                    },
                    0,
                )
                .unwrap(),
            )
            .unwrap();
    }
    let globals = registry(&display, &mut peer); // Last wl_output is TEST-2.
    bind(&mut peer, &globals, "wl_output", 4);
    bind(&mut peer, &globals, "wl_compositor", 5);
    send(&mut peer, 5, 0, &words(&[6]));
    assert!(events(&display, &mut peer, 3).is_empty());
    let client_id = native.state.clients[&client.identity().unwrap()];
    let surface = native.core().world.client_surfaces(client_id)[0];
    assert!(native.set_surface_outputs(surface, Some(&[1])).unwrap());
    native.state.update_surface_output(surface, true).unwrap();
    assert!(events(&display, &mut peer, 3).is_empty());
    assert!(native.set_surface_outputs(surface, Some(&[2])).unwrap());
    assert_eq!(events(&display, &mut peer, 3), [(0, 4)]);
    bind(&mut peer, &globals, "wl_output", 7);
    assert_eq!(events(&display, &mut peer, 3), [(0, 7)]);
    assert!(!native.set_surface_outputs(surface, Some(&[2])).unwrap());
    assert!(native.set_surface_outputs(surface, Some(&[2, 99])).is_err());
    assert!(native.set_surface_outputs(surface, Some(&[2, 2])).is_err());
    assert!(events(&display, &mut peer, 3).is_empty());
    native.state.update_surface_output(surface, false).unwrap();
    assert_eq!(events(&display, &mut peer, 3), [(1, 4), (1, 7)]);
    native.state.update_surface_output(surface, true).unwrap();
    assert_eq!(events(&display, &mut peer, 3), [(0, 4), (0, 7)]);
    native.set_surface_outputs(surface, Some(&[])).unwrap();
    assert_eq!(events(&display, &mut peer, 3), [(1, 4), (1, 7)]);
    bind(&mut peer, &globals, "wl_output", 8);
    assert!(events(&display, &mut peer, 3).is_empty());
    native.set_surface_outputs(surface, None).unwrap();
    assert_eq!(events(&display, &mut peer, 3), [(0, 4), (0, 7), (0, 8)]);
    native.set_surface_outputs(surface, Some(&[1])).unwrap();
    events(&display, &mut peer, 3);
    send(&mut peer, 6, 0, &[]); // wl_surface.destroy
    events(&display, &mut peer, 3);
    assert!(!native.state.surface_outputs.contains_key(&surface));
    assert!(native.set_surface_outputs(surface, Some(&[2])).is_err());
    assert!(client.is_alive());
}

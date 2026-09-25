use super::wire_tests::{bind, registry, send, words};
use super::*;
use crate::integrations::wayland::compositor::DecorationMode::{ClientSide, ServerSide};
use std::{os::unix::net::UnixStream, time::Duration};

struct Peer<'a> {
    native: NativeCompositor<'a>,
    client: OwnedClient,
    display: &'a Display,
    socket: UnixStream,
    next_id: u32,
    xdg: u32,
    toplevel: u32,
    globals: BTreeMap<String, u32>,
}
impl<'a> Peer<'a> {
    fn new(display: &'a Display, negotiation: crate::DecorationNegotiation) -> Self {
        let (mut socket, server) = UnixStream::pair().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let client = display.create_client(server).unwrap();
        let mut native = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
        native.set_decoration_policy(crate::DecorationPolicy {
            negotiation,
            ..Default::default()
        });
        let globals = registry(&display, &mut socket);
        bind(&mut socket, &globals, "wl_compositor", 4);
        bind(&mut socket, &globals, "xdg_wm_base", 5);
        bind(
            &mut socket,
            &globals,
            "org_kde_kwin_server_decoration_manager",
            6,
        );
        send(&mut socket, 4, 0, &words(&[7]));
        Self {
            native,
            client,
            display,
            socket,
            next_id: 8,
            xdg: 0,
            toplevel: 0,
            globals,
        }
    }
    fn send(&mut self, object: u32, opcode: u16, payload: &[u32]) {
        send(&mut self.socket, object, opcode, &words(payload));
    }
    fn roundtrip(&mut self) -> Vec<(u32, u16, Vec<u8>)> {
        // Reuse the registry roundtrip's released callback ID.
        self.send(1, 0, &[3]);
        self.display
            .dispatch_and_flush(Some(Duration::ZERO))
            .unwrap();
        let mut events = Vec::new();
        loop {
            let mut header = [0; 8];
            self.socket.read_exact(&mut header).unwrap();
            let object = u32::from_ne_bytes(header[..4].try_into().unwrap());
            let word = u32::from_ne_bytes(header[4..].try_into().unwrap());
            let opcode = (word & 0xffff) as u16;
            let mut body = vec![0; (word >> 16) as usize - 8];
            self.socket.read_exact(&mut body).unwrap();
            assert!(!(object == 1 && opcode == 0), "protocol error: {body:?}");
            if object == 3 {
                break;
            }
            events.push((object, opcode, body));
        }
        events
    }
    fn surface(&self) -> WaylandSurfaceId {
        let client = self.native.state.clients[&self.client.identity().unwrap()];
        self.native.core().world.client_surfaces(client)[0]
    }
    fn alloc(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
    fn toplevel(&mut self) {
        self.xdg = self.alloc();
        self.toplevel = self.alloc();
        self.send(5, 2, &[self.xdg, 7]);
        self.send(self.xdg, 1, &[self.toplevel]);
    }
    fn kde(&mut self) -> u32 {
        let id = self.alloc();
        self.send(6, 0, &[id, 7]);
        id
    }
    fn xdg_decoration(&mut self) -> u32 {
        let manager = self.alloc();
        bind(
            &mut self.socket,
            &self.globals,
            "zxdg_decoration_manager_v1",
            manager,
        );
        let id = self.alloc();
        self.send(manager, 1, &[id, self.toplevel]);
        id
    }
    fn commit(&mut self) {
        self.send(7, 6, &[]);
        self.roundtrip();
    }
    fn ack(&mut self, events: &[(u32, u16, Vec<u8>)]) {
        let serial = value(events, self.xdg).expect("xdg configure");
        self.send(self.xdg, 4, &[serial]);
        self.commit();
    }
}
fn value(events: &[(u32, u16, Vec<u8>)], object: u32) -> Option<u32> {
    events
        .iter()
        .rev()
        .find(|(id, opcode, _)| *id == object && *opcode == 0)
        .map(|(_, _, body)| u32::from_ne_bytes(body[..4].try_into().unwrap()))
}

#[test]
fn unnegotiated_toplevel_has_no_server_decoration_even_with_server_preference() {
    for policy in [
        crate::DecorationNegotiation::ClientPreference,
        crate::DecorationNegotiation::PreferServer,
    ] {
        let display = Display::new().unwrap();
        let mut p = Peer::new(&display, policy);
        p.toplevel();
        p.send(7, 6, &[]);
        let events = p.roundtrip();
        p.ack(&events);
        assert_eq!(p.native.decoration_mode(p.surface()), Some(ClientSide));
    }
}

#[test]
fn kde_preferences_before_role_and_mode_changes_follow_surface_commits() {
    for policy in [
        crate::DecorationNegotiation::ClientPreference,
        crate::DecorationNegotiation::PreferServer,
    ] {
        let display = Display::new().unwrap();
        let mut p = Peer::new(&display, policy);
        let kde = p.kde(); // GTK can announce decorations before creating its xdg role.
        let events = p.roundtrip();
        assert_eq!(value(&events, 6), Some(2));
        assert_eq!(value(&events, kde), Some(2));
        p.send(kde, 1, &[1]);
        p.toplevel();
        p.send(7, 6, &[]);
        let events = p.roundtrip();
        assert_eq!(value(&events, kde), Some(1));
        p.ack(&events);
        assert_eq!(p.native.decoration_mode(p.surface()), Some(ClientSide));
        for mode in [2, 0, 1, 2] {
            let old = p.native.decoration_mode(p.surface());
            p.send(kde, 1, &[mode]);
            let events = p.roundtrip();
            assert_eq!(value(&events, kde), Some(mode));
            assert_eq!(p.native.decoration_mode(p.surface()), old);
            // KDE doesn't require an xdg acknowledgement to accept a decoration change.
            p.commit();
            assert_eq!(
                p.native.decoration_mode(p.surface()),
                Some(if mode == 2 { ServerSide } else { ClientSide })
            );
        }
        p.send(kde, 0, &[]);
        p.roundtrip();
        assert_eq!(p.native.decoration_mode(p.surface()), Some(ServerSide));
        p.commit();
        assert_eq!(p.native.decoration_mode(p.surface()), Some(ClientSide));
        assert!(!p.native.state.decorations.contains_key(&p.surface()));
    }
}

#[test]
fn xdg_takes_precedence_and_destroy_restores_kde_without_obsolete_ack() {
    let display = Display::new().unwrap();
    let mut p = Peer::new(&display, crate::DecorationNegotiation::ClientPreference);
    let kde = p.kde();
    p.send(kde, 1, &[1]);
    p.toplevel();
    let xdg = p.xdg_decoration();
    p.send(7, 6, &[]);
    let events = p.roundtrip();
    assert_eq!(value(&events, xdg), Some(2));
    p.ack(&events);
    assert_eq!(p.native.decoration_mode(p.surface()), Some(ServerSide));
    p.send(kde, 1, &[0]);
    p.commit();
    assert_eq!(p.native.decoration_mode(p.surface()), Some(ServerSide));
    // Queue a new SSD configure, then destroy xdg-decoration before acknowledging it.
    p.send(xdg, 1, &[2]);
    let events = p.roundtrip();
    p.send(xdg, 0, &[]);
    p.ack(&events);
    assert_eq!(p.native.decoration_mode(p.surface()), Some(ClientSide));
    p.send(kde, 1, &[2]);
    p.commit();
    assert_eq!(p.native.decoration_mode(p.surface()), Some(ServerSide));
}

#[test]
fn kde_multiple_objects_and_surface_destruction_clean_up_preferences() {
    let display = Display::new().unwrap();
    let mut p = Peer::new(&display, crate::DecorationNegotiation::ClientPreference);
    let kde = p.kde();
    p.send(kde, 1, &[1]);
    let second = p.kde();
    p.toplevel();
    p.send(7, 6, &[]);
    let events = p.roundtrip();
    p.ack(&events);
    assert_eq!(p.native.decoration_mode(p.surface()), Some(ServerSide));
    p.send(second, 0, &[]);
    p.commit();
    assert_eq!(p.native.decoration_mode(p.surface()), Some(ClientSide));
    let surface = p.surface();
    p.send(p.toplevel, 0, &[]);
    p.send(p.xdg, 0, &[]);
    p.send(7, 0, &[]);
    p.roundtrip();
    assert!(!p.native.state.decorations.contains_key(&surface));
    assert!(!p.native.state.committed_decorations.contains_key(&surface));
    p.send(kde, 0, &[]); // A decoration release after its wl_surface was destroyed is harmless.
    p.roundtrip();
    assert!(p.client.is_alive());
}

#[test]
fn destroying_xdg_decoration_restores_unnegotiated_client_mode() {
    let display = Display::new().unwrap();
    let mut p = Peer::new(&display, crate::DecorationNegotiation::ClientPreference);
    p.toplevel();
    let xdg = p.xdg_decoration();
    p.send(7, 6, &[]);
    let events = p.roundtrip();
    p.ack(&events);
    assert_eq!(p.native.decoration_mode(p.surface()), Some(ServerSide));
    p.send(xdg, 0, &[]);
    p.roundtrip();
    assert_eq!(p.native.decoration_mode(p.surface()), Some(ServerSide));
    p.commit();
    assert_eq!(p.native.decoration_mode(p.surface()), Some(ClientSide));
    assert!(!p.native.state.decorations.contains_key(&p.surface()));
}

#[test]
fn invalid_decoration_requests_disconnect_and_remove_client_state() {
    for invalid in ["kde-mode", "duplicate-xdg", "orphan-kde", "orphan-xdg"] {
        let display = Display::new().unwrap();
        let mut p = Peer::new(&display, crate::DecorationNegotiation::ClientPreference);
        let kde = p.kde();
        p.toplevel();
        let xdg = p.xdg_decoration();
        p.roundtrip();
        match invalid {
            "kde-mode" => p.send(kde, 1, &[3]),
            "duplicate-xdg" => {
                p.xdg_decoration();
            }
            "orphan-kde" => {
                p.send(xdg, 0, &[]);
                p.send(p.toplevel, 0, &[]);
                p.send(p.xdg, 0, &[]);
                p.send(7, 0, &[]);
                p.send(kde, 1, &[1]);
            }
            "orphan-xdg" => {
                p.send(p.toplevel, 0, &[]);
                p.send(xdg, 1, &[1]);
            }
            _ => unreachable!(),
        }
        p.display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
        assert!(!p.client.is_alive(), "{invalid}");
        assert!(p.native.state.decorations.is_empty(), "{invalid}");
        assert!(p.native.state.committed_decorations.is_empty(), "{invalid}");
    }
}


#[test]
fn client_style_tiling_does_not_leak_into_server_decoration_renegotiation() {
    let display = Display::new().unwrap();
    let mut p = Peer::new(&display, crate::DecorationNegotiation::ClientPreference);
    p.native.set_decoration_policy(crate::DecorationPolicy {
        tiled_client_decorations: true, ..Default::default()
    });
    p.toplevel();
    let decoration = p.xdg_decoration();
    p.send(decoration, 1, &[1]);
    p.commit();
    let surface = p.surface();
    assert!(p.native.state.core.xdg_surface_mut(surface).unwrap().latest_configure().unwrap().states.tiled_left);
    p.send(decoration, 1, &[2]);
    p.roundtrip();
    let state = p.native.state.core.xdg_surface_mut(surface).unwrap().latest_configure().unwrap().states;
    assert!(!state.tiled_left && !state.tiled_right && !state.tiled_top && !state.tiled_bottom);
}

use super::super::wire_tests::{bind, bind_version, registry, send, words};
use super::*;
use std::{io::Read, os::unix::net::UnixStream, time::Duration};

#[test]
fn initial_configure_advertises_window_controls_only_on_supported_versions() {
    for (version, tiled_client_decorations) in [1, 2, 4, 5, 7]
        .into_iter()
        .flat_map(|v| [(v, false), (v, true)])
    {
        let display = Display::new().unwrap();
        let (mut peer, socket) = UnixStream::pair().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let _client = display.create_client(socket).unwrap();
        let mut native = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
        native.set_decoration_policy(crate::DecorationPolicy {
            tiled_client_decorations,
            ..Default::default()
        });
        let globals = registry(&display, &mut peer);
        bind(&mut peer, &globals, "wl_compositor", 4);
        bind_version(&mut peer, &globals, "xdg_wm_base", 5, version);
        send(&mut peer, 4, 0, &words(&[6]));
        send(&mut peer, 5, 2, &words(&[7, 6]));
        send(&mut peer, 7, 1, &words(&[8]));
        send(&mut peer, 6, 6, &[]);
        send(&mut peer, 1, 0, &words(&[9]));
        display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
        let mut capabilities = None;
        let mut configured = false;
        loop {
            let mut header = [0; 8];
            peer.read_exact(&mut header).unwrap();
            let object = u32::from_ne_bytes(header[..4].try_into().unwrap());
            let word = u32::from_ne_bytes(header[4..].try_into().unwrap());
            let opcode = word & 0xffff;
            let mut body = vec![0; (word >> 16) as usize - 8];
            peer.read_exact(&mut body).unwrap();
            assert!(!(object == 1 && opcode == 0), "protocol error: {body:?}");
            if object == 9 {
                break;
            }
            if object == 8 && opcode == 3 {
                assert!(!configured, "capabilities must precede the first configure");
                assert!(capabilities.is_none(), "duplicate capabilities");
                assert_eq!(body, words(&[12, 2, 3, 4]));
                capabilities = Some(body.clone());
            }
            if object == 8 && opcode == 0 {
                let expected = if tiled_client_decorations && version >= 2 {
                    words(&[0, 0, 16, 5, 6, 7, 8])
                } else {
                    words(&[0, 0, 0])
                };
                assert_eq!(body, expected);
            }
            if object == 7 && opcode == 0 {
                assert_eq!(capabilities.is_some(), version >= 5);
                configured = true;
            }
        }
        assert!(configured);
        assert_eq!(capabilities.is_some(), version >= 5);
    }
}

#[test]
fn tiled_client_style_preserves_host_state_and_server_decorations() {
    use crate::integrations::wayland::compositor::{DecorationMode, ToplevelState};
    let policy = crate::DecorationPolicy {
        tiled_client_decorations: true,
        ..Default::default()
    };
    let floating = ToplevelState {
        activated: true,
        resizing: true,
        ..Default::default()
    };
    let client = decoration_states(policy, DecorationMode::ClientSide, floating);
    assert_eq!(wire_states(client, 7), vec![3, 4, 5, 6, 7, 8]);
    assert!(!client.maximized && !client.fullscreen);
    assert_eq!(
        decoration_states(policy, DecorationMode::ServerSide, floating),
        floating
    );
    assert_eq!(
        decoration_states(
            crate::DecorationPolicy::DEFAULT,
            DecorationMode::ClientSide,
            floating
        ),
        floating
    );
    let tiled = ToplevelState {
        tiled_left: true,
        ..floating
    };
    assert_eq!(
        decoration_states(policy, DecorationMode::ServerSide, tiled),
        tiled
    );
    let fullscreen = ToplevelState {
        fullscreen: true,
        ..floating
    };
    assert_eq!(
        decoration_states(policy, DecorationMode::ClientSide, fullscreen),
        fullscreen
    );
}

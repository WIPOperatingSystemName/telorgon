use super::super::discovery::tests::{ready, request, write_reply};
use super::*;
use std::os::unix::net::UnixStream;
fn start() -> (Manager, UnixStream, Instant) {
    let (transport, requests, discovered, mut peer, now) = ready();
    let mut manager = Manager::new(
        transport,
        requests,
        discovered,
        now + Duration::from_secs(10),
    )
    .unwrap();
    manager.dispatch(now).unwrap();
    for opcode in [1, 18, 23, 23] {
        assert_eq!(request(&mut peer)[0], opcode);
    }
    let bytes = request(&mut peer);
    assert_eq!(&bytes[..2], &[128, 2]); // Negotiated Composite, RedirectSubwindows.
    assert_eq!(u32::from_ne_bytes(bytes[4..8].try_into().unwrap()), 1);
    assert_eq!(bytes[8], 1); // Manual; the root itself is not redirected.
    assert_eq!(request(&mut peer)[0], 43);
    (manager, peer, now)
}
fn owners(peer: &mut UnixStream, first: u16, owner: u32) {
    for sequence in [first, first + 1] {
        write_reply(
            peer,
            &xproto::GetSelectionOwnerReply {
                sequence,
                owner,
                ..Default::default()
            }
            .serialize(),
        );
    }
    write_reply(
        peer,
        &xproto::GetInputFocusReply {
            sequence: 47,
            ..Default::default()
        }
        .serialize(),
    );
}
fn timestamp(manager: &Manager, peer: &mut UnixStream, synthetic: bool) {
    write_reply(
        peer,
        &xproto::PropertyNotifyEvent {
            response_type: xproto::PROPERTY_NOTIFY_EVENT | if synthetic { 128 } else { 0 },
            sequence: 47,
            window: manager.window,
            atom: manager.discovered.atoms["_NET_WM_NAME"],
            time: 123,
            state: xproto::Property::NEW_VALUE,
        }
        .serialize(),
    );
}
fn pump(manager: &mut Manager, now: Instant) {
    for _ in 0..10 {
        manager.dispatch(now).unwrap();
    }
}
fn check_metadata(manager: &Manager, peer: &mut UnixStream) {
    let check = manager.discovered.atoms["_NET_SUPPORTING_WM_CHECK"];
    let supported = manager.discovered.atoms["_NET_SUPPORTED"];
    for (window, property, type_, values) in [
        (manager.window, check, 33, vec![manager.window]),
        (1, check, 33, vec![manager.window]),
        (
            1,
            supported,
            4,
            vec![
                supported,
                check,
                manager.discovered.atoms["_NET_WM_NAME"],
                manager.discovered.atoms["_NET_FRAME_EXTENTS"],
                manager.discovered.atoms["_NET_REQUEST_FRAME_EXTENTS"],
                manager.discovered.atoms["_NET_WM_MOVERESIZE"],
                manager.discovered.atoms["_NET_WM_STATE"],
                manager.discovered.atoms["_NET_WM_STATE_MAXIMIZED_VERT"],
                manager.discovered.atoms["_NET_WM_STATE_MAXIMIZED_HORZ"],
                manager.discovered.atoms["_NET_WM_SYNC_REQUEST"],
                manager.discovered.atoms["_NET_WM_SYNC_REQUEST_COUNTER"],
            ],
        ),
    ] {
        let bytes = request(peer);
        assert_eq!(&bytes[..2], &[18, 0]); // ChangeProperty, Replace
        assert_eq!(u32::from_ne_bytes(bytes[4..8].try_into().unwrap()), window);
        assert_eq!(
            u32::from_ne_bytes(bytes[8..12].try_into().unwrap()),
            property
        );
        assert_eq!(u32::from_ne_bytes(bytes[12..16].try_into().unwrap()), type_);
        assert_eq!(bytes[16], 32);
        assert_eq!(
            u32::from_ne_bytes(bytes[20..24].try_into().unwrap()) as usize,
            values.len()
        );
        assert_eq!(
            bytes[24..]
                .chunks_exact(4)
                .map(|b| u32::from_ne_bytes(b.try_into().unwrap()))
                .collect::<Vec<_>>(),
            values
        );
    }
}
#[test]
fn themed_cursor_upload_uses_argb_hotspot_and_checked_resource_lifetimes() {
    let (mut manager, mut peer, now) = start();
    manager.discovered.setup.pixmap_formats = vec![xproto::Format {
        depth: 32,
        bits_per_pixel: 32,
        scanline_pad: 32,
    }];
    manager.discovered.setup.maximum_request_length = 65535;
    manager.discovered.setup.image_byte_order = xproto::ImageOrder::LSB_FIRST;
    manager.root_cursor = Some(
        super::super::root_cursor::RootCursor::new(
            2,
            1,
            1,
            0,
            &[200, 100, 50, 128, 0, 0, 0, 0],
            false,
        )
        .unwrap(),
    );
    manager.upload_root_cursor(140, 77).unwrap();
    let failed_sequence = manager.requests.last_sequence() - 5; // RENDER CreateCursor
    manager.cursor_barrier().unwrap();
    pump(&mut manager, now);
    assert!(!manager.is_complete());
    let pixmap = request(&mut peer);
    assert_eq!(&pixmap[..2], &[53, 32]);
    assert_eq!(request(&mut peer)[0], 55);
    let image = request(&mut peer);
    assert_eq!(image[0], 72);
    assert_eq!(&image[24..], &[25, 50, 100, 128, 0, 0, 0, 0]);
    let picture = request(&mut peer);
    assert_eq!(&picture[..2], &[140, 4]);
    assert_eq!(u32::from_ne_bytes(picture[12..16].try_into().unwrap()), 77);
    let cursor = request(&mut peer);
    assert_eq!(&cursor[..2], &[140, 27]);
    assert_eq!(u16::from_ne_bytes(cursor[12..14].try_into().unwrap()), 1);
    let root = request(&mut peer);
    assert_eq!(root[0], 2);
    assert_eq!(u32::from_ne_bytes(root[4..8].try_into().unwrap()), 1);
    assert_eq!(&root[12..16], &cursor[4..8]);
    for opcode in [95, 140, 60, 54, 43] {
        assert_eq!(request(&mut peer)[0], opcode);
    }
    let mut error = [0u8; 32];
    error[1] = 2;
    error[2..4].copy_from_slice(&(failed_sequence as u16).to_ne_bytes());
    error[10] = 140;
    write_reply(&mut peer, &error);
    assert!(manager.dispatch(now).is_err());
    assert!(!manager.is_complete());
}
#[test]
fn composite_barrier_is_required_even_with_timestamp_and_free_selections() {
    let (mut manager, mut peer, now) = start();
    for sequence in [44, 45] {
        write_reply(
            &mut peer,
            &xproto::GetSelectionOwnerReply {
                sequence,
                owner: 0,
                ..Default::default()
            }
            .serialize(),
        );
    }
    timestamp(&manager, &mut peer, false);
    pump(&mut manager, now);
    assert_eq!(manager.existing, [true; 2]);
    assert_eq!(manager.timestamp, Some(123));
    assert!(!manager.redirected);
    assert!(!manager.claiming);
    assert!(!manager.transport.wants_write());
    write_reply(
        &mut peer,
        &xproto::GetInputFocusReply {
            sequence: 47,
            ..Default::default()
        }
        .serialize(),
    );
    pump(&mut manager, now);
    assert!(manager.redirected);
    assert!(manager.claiming);
    assert_eq!(request(&mut peer)[0], 22);
}
#[test]
fn composite_conflict_fails_before_selection_acquisition() {
    let (mut manager, mut peer, now) = start();
    for sequence in [44, 45] {
        write_reply(
            &mut peer,
            &xproto::GetSelectionOwnerReply {
                sequence,
                owner: 0,
                ..Default::default()
            }
            .serialize(),
        );
    }
    let mut error = [0u8; 32];
    error[1] = 10; // BadAccess: another client owns manual redirection.
    error[2..4].copy_from_slice(&46u16.to_ne_bytes());
    error[8..10].copy_from_slice(&2u16.to_ne_bytes());
    error[10] = 128;
    write_reply(&mut peer, &error);
    let failure = manager.dispatch(now).err().unwrap();
    assert!(failure.to_string().contains("essential XWM request failed"));
    assert!(!manager.claiming);
    assert!(!manager.is_complete());
    assert!(manager.dispatch(now).is_err());
}
#[test]
fn metadata_error_prevents_completion() {
    let (mut manager, mut peer, now) = start();
    owners(&mut peer, 44, 0);
    timestamp(&manager, &mut peer, false);
    pump(&mut manager, now);
    for (index, opcode) in [22, 23, 22, 23].into_iter().enumerate() {
        assert_eq!(request(&mut peer)[0], opcode);
        if opcode == 23 {
            write_reply(
                &mut peer,
                &xproto::GetSelectionOwnerReply {
                    sequence: 48 + index as u16,
                    owner: manager.window,
                    ..Default::default()
                }
                .serialize(),
            );
        }
    }
    pump(&mut manager, now);
    check_metadata(&manager, &mut peer);
    assert!(!manager.is_complete());
    let mut error = [0u8; 32];
    error[1] = 3; // BadWindow on the first metadata write.
    error[2..4].copy_from_slice(&51u16.to_ne_bytes());
    error[10] = 18;
    write_reply(&mut peer, &error);
    assert!(manager.dispatch(now).is_err());
    assert!(manager.finish().is_err());
}
#[test]
fn ownership_requires_timestamp_verification_and_checked_announcements() {
    let (mut manager, mut peer, now) = start();
    owners(&mut peer, 44, 0);
    pump(&mut manager, now);
    assert!(!manager.claiming);
    timestamp(&manager, &mut peer, true);
    pump(&mut manager, now);
    assert!(!manager.claiming);
    timestamp(&manager, &mut peer, false);
    pump(&mut manager, now);
    for (index, selection) in manager.selections.into_iter().enumerate() {
        let bytes = request(&mut peer);
        assert_eq!(bytes[0], 22);
        assert_eq!(
            u32::from_ne_bytes(bytes[4..8].try_into().unwrap()),
            manager.window
        );
        assert_eq!(
            u32::from_ne_bytes(bytes[8..12].try_into().unwrap()),
            selection
        );
        assert_eq!(u32::from_ne_bytes(bytes[12..16].try_into().unwrap()), 123);
        assert_eq!(request(&mut peer)[0], 23);
        write_reply(
            &mut peer,
            &xproto::GetSelectionOwnerReply {
                sequence: 49 + index as u16 * 2,
                owner: manager.window,
                ..Default::default()
            }
            .serialize(),
        );
    }
    pump(&mut manager, now);
    assert!(!manager.is_complete());
    check_metadata(&manager, &mut peer);
    for selection in manager.selections {
        let bytes = request(&mut peer);
        assert_eq!(bytes[0], 25);
        let (event, _) = xproto::ClientMessageEvent::try_parse(&bytes[12..]).unwrap();
        assert_eq!(
            event.data.as_data32(),
            [123, selection, manager.window, 0, 0]
        );
        assert_eq!(event.window, 1);
        assert_eq!(event.type_, manager.discovered.atoms["MANAGER"]);
    }
    let font = request(&mut peer);
    assert_eq!(font[0], 45);
    assert_eq!(&font[12..18], b"cursor");
    let glyph = request(&mut peer);
    assert_eq!(glyph[0], 94);
    assert_eq!(u16::from_ne_bytes(glyph[16..18].try_into().unwrap()), 68);
    assert_eq!(u16::from_ne_bytes(glyph[18..20].try_into().unwrap()), 69);
    let attributes = request(&mut peer);
    assert_eq!(attributes[0], 2);
    assert_eq!(u32::from_ne_bytes(attributes[4..8].try_into().unwrap()), 1);
    assert_eq!(
        u32::from_ne_bytes(attributes[8..12].try_into().unwrap()),
        1 << 14
    );
    assert_eq!(&attributes[12..16], &glyph[4..8]);
    assert_eq!(request(&mut peer)[0], 95);
    assert_eq!(request(&mut peer)[0], 46);
    assert!(!manager.is_complete());
    assert_eq!(request(&mut peer)[0], 43);
    write_reply(
        &mut peer,
        &xproto::GetInputFocusReply {
            sequence: 62,
            ..Default::default()
        }
        .serialize(),
    );
    pump(&mut manager, now);
    assert!(manager.is_complete());
    assert_eq!(manager.requests.outstanding(), 0);
    write_reply(
        &mut peer,
        &xproto::SelectionClearEvent {
            response_type: xproto::SELECTION_CLEAR_EVENT,
            sequence: 62,
            time: 124,
            owner: manager.window,
            selection: manager.selections[0],
        }
        .serialize(),
    );
    assert!(
        manager
            .dispatch(now)
            .err()
            .unwrap()
            .to_string()
            .contains("ownership lost")
    );
    assert!(!manager.is_complete());
}
#[test]
fn existing_owner_is_not_replaced_and_failure_is_terminal() {
    let (mut manager, mut peer, now) = start();
    owners(&mut peer, 44, 999);
    assert!(manager.dispatch(now).is_err());
    assert!(!manager.claiming);
    assert!(manager.dispatch(now).is_err());
    assert!(manager.finish().is_err());
}
#[test]
fn missing_timestamp_hits_original_startup_deadline() {
    let (mut manager, mut peer, now) = start();
    owners(&mut peer, 44, 0);
    pump(&mut manager, now);
    assert!(manager.dispatch(manager.deadline()).is_err());
    assert!(!manager.is_complete());
}
#[test]
fn lost_claim_cannot_reach_announcements() {
    let (mut manager, mut peer, now) = start();
    owners(&mut peer, 44, 0);
    timestamp(&manager, &mut peer, false);
    pump(&mut manager, now);
    for opcode in [22, 23, 22, 23] {
        assert_eq!(request(&mut peer)[0], opcode);
    }
    write_reply(
        &mut peer,
        &xproto::GetSelectionOwnerReply {
            sequence: 49,
            owner: 999,
            ..Default::default()
        }
        .serialize(),
    );
    assert!(manager.dispatch(now).is_err());
    assert!(!manager.is_complete());
}

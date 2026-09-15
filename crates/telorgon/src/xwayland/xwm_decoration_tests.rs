use super::super::discovery::tests::{ready, request, write_reply};
use super::*;
use std::io::Write;
use x11rb_protocol::x11_utils::Serialize;
pub(crate) fn fixture() -> (Xwm, UnixStream, Instant, XWindow) {
    let (transport, requests, discovered, peer, now) = ready();
    let mut windows = Windows::new(1, 4096, 1, 2, discovered.atoms["WL_SURFACE_SERIAL"]).unwrap();
    let probe = windows.begin_inspection(10).unwrap().unwrap();
    windows
        .finish_inspection(
            probe,
            1,
            Geometry {
                x: 0,
                y: 0,
                width: 640,
                height: 480,
                border: 0,
            },
            false,
            true,
        )
        .unwrap();
    let id = windows.get(10).unwrap().id;
    let sync_atom = discovered.atoms["_NET_WM_SYNC_REQUEST"];
    let counter_atom = discovered.atoms["_NET_WM_SYNC_REQUEST_COUNTER"];
    let t = Tracking {
        selection_checks: BTreeMap::new(),
        selection_watches: vec![],
        transport,
        requests,
        decorations: PropertyReader::new_decorations(discovered.atoms["_MOTIF_WM_HINTS"]),
        frame_extents: BTreeMap::new(),
        move_resize_requests: Vec::new(),
        maximize_requests: Vec::new(),
        maximized: BTreeMap::new(),
        discovered,
        _ids: IdAllocator::new(0x200000, 0x1fffff).unwrap(),
        manager: 2,
        incoming: VecDeque::new(),
        commands: Commands::new(),
        protocols: PropertyReader::new(104, 105, 106).with_sync_request(sync_atom),
        sync_counter: PropertyReader::new_counter(counter_atom),
        resize_sync: Default::default(),
        server_time: 123,
        hints: PropertyReader::new_hints(),
        normal_hints: PropertyReader::new_normal_hints(),
        title: PropertyReader::new_text(110, 111, true),
        legacy_title: PropertyReader::new_text(39, 31, false),
        app_class: PropertyReader::new_text(67, 31, false),
        app_icon: PropertyReader::new_icon(112),
        last_focus_request: None,
    };

    (
        Xwm {
            root_cursor: None,
            phase: Some(Phase::Tracking(t)),
            windows: Some(windows),
            inspector: Inspector::new(),
            early_events: VecDeque::new(),
            deadline: now + Duration::from_secs(10),
        },
        peer,
        now,
        id,
    )
}
pub(crate) fn drive(xwm: &mut Xwm, now: Instant) {
    for _ in 0..20 {
        let turn = xwm.dispatch(now).unwrap();
        if !turn.reschedule && !xwm.wants_write() {
            break;
        }
    }
}
#[test]
fn frame_extents_wire_updates_coalesce_and_client_requests_force_a_reply() {
    let (mut xwm, mut peer, now, id) = fixture();
    let (atom, request_atom) = match &xwm.phase {
        Some(Phase::Tracking(t)) => (
            t.discovered.atoms["_NET_FRAME_EXTENTS"],
            t.discovered.atoms["_NET_REQUEST_FRAME_EXTENTS"],
        ),
        _ => unreachable!(),
    };
    for extents in [[3, 3, 90, 3], [0; 4], [0; 4]] {
        assert!(xwm.set_frame_extents(id, extents, now).unwrap());
        drive(&mut xwm, now);
        let bytes = request(&mut peer);
        let word = |offset| u32::from_ne_bytes(bytes[offset..offset + 4].try_into().unwrap());
        assert_eq!(&bytes[..2], &[18, 0]);
        assert_eq!(word(4), id.xid);
        assert_eq!(word(8), atom);
        assert_eq!(word(12), u32::from(xproto::AtomEnum::CARDINAL));
        assert_eq!(bytes[16], 32);
        assert_eq!(word(20), 4);
        assert_eq!(
            &bytes[24..],
            extents
                .into_iter()
                .flat_map(u32::to_ne_bytes)
                .collect::<Vec<_>>()
        );
        assert_eq!(request(&mut peer)[0], 43);
        let sequence = match &xwm.phase {
            Some(Phase::Tracking(t)) => t.requests.last_sequence(),
            _ => unreachable!(),
        };
        write_reply(
            &mut peer,
            &xproto::GetInputFocusReply {
                sequence: sequence as u16,
                ..Default::default()
            }
            .serialize(),
        );
        drive(&mut xwm, now);
        assert!(xwm.set_frame_extents(id, extents, now).unwrap());
        let after = match &xwm.phase {
            Some(Phase::Tracking(t)) => t.requests.last_sequence(),
            _ => unreachable!(),
        };
        assert_eq!(sequence, after);
        let event: [u8; 32] = xproto::ClientMessageEvent {
            response_type: xproto::CLIENT_MESSAGE_EVENT | 0x80,
            format: 32,
            sequence: sequence as u16,
            window: id.xid,
            type_: request_atom,
            data: [0; 5].into(),
        }
        .into();
        peer.write_all(&event).unwrap();
        drive(&mut xwm, now);
    }
    // Unmapped windows are valid targets for an estimate.
    let event: [u8; 32] = xproto::UnmapNotifyEvent {
        response_type: xproto::UNMAP_NOTIFY_EVENT,
        event: 1,
        window: id.xid,
        ..Default::default()
    }
    .into();
    xwm.windows.as_mut().unwrap().event(&event).unwrap();
    assert!(xwm.set_frame_extents(id, [1, 1, 30, 1], now).unwrap());
}

#[test]
fn motif_notifications_switch_decoration_policy_and_deletion_restores_default() {
    let (mut xwm, mut peer, now, id) = fixture();
    let atom = match &mut xwm.phase {
        Some(Phase::Tracking(t)) => {
            t.decorations.refresh(id).unwrap();
            t.discovered.atoms["_MOTIF_WM_HINTS"]
        }
        _ => unreachable!(),
    };
    for mask in [Some(0u32), Some(1u32), None] {
        drive(&mut xwm, now);
        let bytes = request(&mut peer);
        assert_eq!(bytes[0], 20);
        assert_eq!(u32::from_ne_bytes(bytes[8..12].try_into().unwrap()), atom);
        assert_eq!(u32::from_ne_bytes(bytes[20..24].try_into().unwrap()), 5);
        let sequence = match &xwm.phase {
            Some(Phase::Tracking(t)) => t.requests.last_sequence(),
            _ => unreachable!(),
        };
        let value: Vec<u8> = mask
            .map(|mask| {
                [2u32, 0, mask, 0, 0]
                    .into_iter()
                    .flat_map(u32::to_ne_bytes)
                    .collect()
            })
            .unwrap_or_default();
        peer.write_all(
            &xproto::GetPropertyReply {
                sequence: sequence as u16,
                format: if mask.is_some() { 32 } else { 0 },
                type_: if mask.is_some() { atom } else { 0 },
                length: (value.len() / 4) as u32,
                value_len: (value.len() / 4) as u32,
                bytes_after: 0,
                value,
            }
            .serialize(),
        )
        .unwrap();
        drive(&mut xwm, now);
        assert_eq!(xwm.decorations(id), mask != Some(0));
        let event: [u8; 32] = xproto::PropertyNotifyEvent {
            response_type: xproto::PROPERTY_NOTIFY_EVENT,
            sequence: sequence as u16,
            window: id.xid,
            atom,
            time: 123,
            state: xproto::Property::NEW_VALUE,
        }
        .into();
        peer.write_all(&event).unwrap();
    }
}

#[test]
fn app_header_move_resize_messages_are_bounded_and_validate_direction() {
    let (mut xwm, mut peer, now, id) = fixture();
    let atom = match &xwm.phase {
        Some(Phase::Tracking(t)) => t.discovered.atoms["_NET_WM_MOVERESIZE"],
        _ => unreachable!(),
    };
    let sequence = match &xwm.phase {
        Some(Phase::Tracking(t)) => t.requests.last_sequence() as u16,
        _ => unreachable!(),
    };
    for (format, direction, target, expected) in [
        (32, 8, id.xid, 1),
        (32, 11, id.xid, 2),
        (8, 8, id.xid, 2),
        (32, 12, id.xid, 2),
        (32, 8, 999, 2),
    ] {
        let event: [u8; 32] = xproto::ClientMessageEvent {
            response_type: xproto::CLIENT_MESSAGE_EVENT | 0x80,
            format,
            sequence,
            window: target,
            type_: atom,
            data: [100, 100, direction, 1, 1].into(),
        }
        .into();
        peer.write_all(&event).unwrap();
        drive(&mut xwm, now);
        match &xwm.phase {
            Some(Phase::Tracking(t)) => assert_eq!(t.move_resize_requests.len(), expected),
            _ => unreachable!(),
        }
    }
    assert!(xwm.take_move_resize_requests().is_empty()); // No associated surface in this fixture.
}

#[test]
fn maximize_requests_validate_and_apply_paired_atoms_once() {
    let (mut xwm, mut peer, now, id) = fixture();
    let (state, vert, horz, sequence) = match &xwm.phase {
        Some(Phase::Tracking(t)) => (
            t.discovered.atoms["_NET_WM_STATE"],
            t.discovered.atoms["_NET_WM_STATE_MAXIMIZED_VERT"],
            t.discovered.atoms["_NET_WM_STATE_MAXIMIZED_HORZ"],
            t.requests.last_sequence() as u16,
        ),
        _ => unreachable!(),
    };
    for (format, target, data, expected) in [
        (32, id.xid, [1, vert, horz, 1, 0], 1),
        (32, id.xid, [0, horz, vert, 1, 0], 2),
        (32, id.xid, [2, vert, horz, 2, 0], 3),
        (32, id.xid, [1, 0, horz, 0, 0], 4),
        (8, id.xid, [1, vert, horz, 1, 0], 4),
        (32, 999, [1, vert, horz, 1, 0], 4),
        (32, id.xid, [3, vert, horz, 1, 0], 4),
        (32, id.xid, [1, 0, 0, 1, 0], 4),
        (32, id.xid, [1, vert, horz, 3, 0], 4),
    ] {
        let event: [u8; 32] = xproto::ClientMessageEvent {
            response_type: xproto::CLIENT_MESSAGE_EVENT | 0x80,
            format,
            sequence,
            window: target,
            type_: state,
            data: data.into(),
        }
        .into();
        peer.write_all(&event).unwrap();
        drive(&mut xwm, now);
        match &xwm.phase {
            Some(Phase::Tracking(t)) => assert_eq!(t.maximize_requests.len(), expected),
            _ => unreachable!(),
        }
    }
    // Authenticate the Wayland association before consuming requests.
    let serial_atom = match &xwm.phase {
        Some(Phase::Tracking(t)) => t.discovered.atoms["WL_SURFACE_SERIAL"],
        _ => unreachable!(),
    };
    let event: [u8; 32] = xproto::ClientMessageEvent {
        response_type: xproto::CLIENT_MESSAGE_EVENT,
        format: 32,
        sequence,
        window: id.xid,
        type_: serial_atom,
        data: [123, 0, 0, 0, 0].into(),
    }
    .into();
    xwm.windows.as_mut().unwrap().event(&event).unwrap();
    xwm.windows
        .as_mut()
        .unwrap()
        .committed_surface(1, 50, 123)
        .unwrap();
    assert_eq!(
        xwm.take_maximize_requests(),
        vec![(50, 1), (50, 0), (50, 2), (50, 1)]
    );
    assert!(xwm.take_maximize_requests().is_empty());
    if let Some(Phase::Tracking(t)) = &mut xwm.phase {
        t.maximize_requests.push((id, 1));
    }
    xwm.windows
        .as_mut()
        .unwrap()
        .destroy_surface(1, 50)
        .unwrap();
    assert!(xwm.take_maximize_requests().is_empty());
}

#[test]
fn maximize_state_wire_reports_both_axes_and_restore_and_coalesces() {
    let (mut xwm, mut peer, now, id) = fixture();
    let (state, vert, horz) = match &xwm.phase {
        Some(Phase::Tracking(t)) => (
            t.discovered.atoms["_NET_WM_STATE"],
            t.discovered.atoms["_NET_WM_STATE_MAXIMIZED_VERT"],
            t.discovered.atoms["_NET_WM_STATE_MAXIMIZED_HORZ"],
        ),
        _ => unreachable!(),
    };
    for maximized in [false, true, false] {
        assert!(xwm.set_maximized_state(id, maximized, now).unwrap());
        drive(&mut xwm, now);
        let bytes = request(&mut peer);
        let word = |offset| u32::from_ne_bytes(bytes[offset..offset + 4].try_into().unwrap());
        assert_eq!(&bytes[..2], &[18, 0]);
        assert_eq!(word(4), id.xid);
        assert_eq!(word(8), state);
        assert_eq!(word(12), u32::from(xproto::AtomEnum::ATOM));
        assert_eq!(bytes[16], 32);
        assert_eq!(word(20), if maximized { 2 } else { 0 });
        if maximized {
            assert_eq!([word(24), word(28)], [vert, horz]);
        }
        assert_eq!(request(&mut peer)[0], 43);
        let sequence = match &xwm.phase {
            Some(Phase::Tracking(t)) => t.requests.last_sequence(),
            _ => unreachable!(),
        };
        write_reply(
            &mut peer,
            &xproto::GetInputFocusReply {
                sequence: sequence as u16,
                ..Default::default()
            }
            .serialize(),
        );
        drive(&mut xwm, now);
        assert!(xwm.set_maximized_state(id, maximized, now).unwrap());
        match &xwm.phase {
            Some(Phase::Tracking(t)) => assert_eq!(t.requests.last_sequence(), sequence),
            _ => unreachable!(),
        }
    }
}

pub(crate) fn wire_request(peer: &mut UnixStream) -> Vec<u8> {
    request(peer)
}

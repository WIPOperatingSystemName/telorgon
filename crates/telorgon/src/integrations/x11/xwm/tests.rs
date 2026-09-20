use super::super::discovery::tests::{ready, request, write_reply};
use super::*;
use std::io::{Read, Write};
use x11rb_protocol::x11_utils::Serialize;
fn drive(xwm: &mut Xwm, now: Instant) -> Vec<Action> {
    let mut actions = vec![];
    for _ in 0..10 {
        actions.extend(xwm.dispatch(now).unwrap().actions);
    }
    actions
}
#[test]
fn focus_timestamp_order_handles_wrap_zero_and_expired_history() {
    let now = Instant::now();
    assert!(!focus_timestamp_is_current(None, 0, now));
    assert!(focus_timestamp_is_current(
        Some((u32::MAX - 2, now)),
        2,
        now
    ));
    assert!(!focus_timestamp_is_current(
        Some((2, now)),
        u32::MAX - 2,
        now
    ));
    assert!(focus_timestamp_is_current(Some((123, now)), 123, now));
    assert!(!focus_timestamp_is_current(Some((123, now)), 122, now));
    assert!(!focus_timestamp_is_current(
        Some((123, now)),
        123 + (1 << 31),
        now
    ));
    assert!(focus_timestamp_is_current(
        Some((123, now)),
        122,
        now + Duration::from_millis(1u64 << 31)
    ));
}
#[test]
fn startup_events_reach_tracking_and_selection_loss_revokes_state() {
    let (transport, requests, discovered, mut peer, now) = ready();
    let serial_atom = discovered.atoms["WL_SURFACE_SERIAL"];
    let wm_atom = discovered.atoms["WM_S0"];
    let clipboard_atom = discovered.atoms["CLIPBOARD"];
    let mut xwm = Xwm {
        root_cursor: None,
        phase: None,
        windows: None,
        inspector: Inspector::new(),
        early_events: VecDeque::new(),
        deadline: now + Duration::from_secs(10),
    };
    xwm.phase = Some(xwm.manager(transport, requests, discovered).unwrap());
    drive(&mut xwm, now);
    let mut manager = 0;
    for opcode in [1, 18, 23, 23, 128, 43] {
        let bytes = request(&mut peer);
        assert_eq!(bytes[0], opcode);
        if opcode == 1 {
            manager = u32::from_ne_bytes(bytes[4..8].try_into().unwrap());
        }
    }
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
    write_reply(
        &mut peer,
        &xproto::GetInputFocusReply {
            sequence: 47,
            ..Default::default()
        }
        .serialize(),
    );
    let create: [u8; 32] = xproto::CreateNotifyEvent {
        response_type: xproto::CREATE_NOTIFY_EVENT,
        sequence: 47,
        parent: 1,
        window: 10,
        width: 640,
        height: 480,
        ..Default::default()
    }
    .into();
    peer.write_all(&create).unwrap();
    let timestamp: [u8; 32] = xproto::PropertyNotifyEvent {
        response_type: xproto::PROPERTY_NOTIFY_EVENT,
        sequence: 47,
        window: manager,
        atom: 110,
        time: 123,
        state: xproto::Property::NEW_VALUE,
    }
    .into();
    peer.write_all(&timestamp).unwrap();
    let actions = drive(&mut xwm, now);
    let id = xwm.windows().unwrap().get(10).unwrap().id;
    assert_eq!(actions, vec![Action::Created(id)]);
    for (index, opcode) in [22, 23, 22, 23].into_iter().enumerate() {
        assert_eq!(request(&mut peer)[0], opcode);
        if opcode == 23 {
            write_reply(
                &mut peer,
                &xproto::GetSelectionOwnerReply {
                    sequence: 48 + index as u16,
                    owner: manager,
                    ..Default::default()
                }
                .serialize(),
            );
        }
    }
    drive(&mut xwm, now);
    for opcode in [18, 18, 18, 25, 25, 45, 94, 2, 95, 46, 43] {
        assert_eq!(request(&mut peer)[0], opcode);
    }
    write_reply(
        &mut peer,
        &xproto::GetInputFocusReply {
            sequence: 62,
            ..Default::default()
        }
        .serialize(),
    );
    drive(&mut xwm, now);
    let query = request(&mut peer);
    assert_eq!(query[0], 15);
    assert_eq!(u32::from_ne_bytes(query[4..8].try_into().unwrap()), 1);
    let mut tree = xproto::QueryTreeReply {
        sequence: 63,
        root: 1,
        parent: 0,
        children: vec![manager, 10],
        ..Default::default()
    }
    .serialize();
    tree[4..8].copy_from_slice(&2u32.to_ne_bytes());
    peer.write_all(&tree).unwrap();
    drive(&mut xwm, now);
    xwm.map_window(id, now).unwrap();
    xwm.configure_window(
        id,
        Geometry {
            x: -20,
            y: 30,
            width: 800,
            height: 600,
            border: 0,
        },
        now,
    )
    .unwrap();
    drive(&mut xwm, now);
    for opcode in [8, 43, 12, 43] {
        let bytes = request(&mut peer);
        assert_eq!(bytes[0], opcode);
        if opcode == 12 {
            assert_eq!(u32::from_ne_bytes(bytes[4..8].try_into().unwrap()), 10);
            assert_eq!(i32::from_ne_bytes(bytes[12..16].try_into().unwrap()), -20);
            assert_eq!(u32::from_ne_bytes(bytes[20..24].try_into().unwrap()), 800);
        }
    }
    for sequence in [65, 67] {
        write_reply(
            &mut peer,
            &xproto::GetInputFocusReply {
                sequence,
                ..Default::default()
            }
            .serialize(),
        );
    }
    assert!(drive(&mut xwm, now).is_empty());
    assert!(!xwm.windows().unwrap().get(10).unwrap().mapped);
    assert_eq!(xwm.windows().unwrap().get(10).unwrap().geometry.width, 640);
    let map: [u8; 32] = xproto::MapNotifyEvent {
        response_type: xproto::MAP_NOTIFY_EVENT,
        sequence: 67,
        event: 1,
        window: 10,
        ..Default::default()
    }
    .into();
    peer.write_all(&map).unwrap();
    let configured: [u8; 32] = xproto::ConfigureNotifyEvent {
        response_type: xproto::CONFIGURE_NOTIFY_EVENT,
        sequence: 67,
        event: 1,
        window: 10,
        x: -20,
        y: 30,
        width: 800,
        height: 600,
        ..Default::default()
    }
    .into();
    peer.write_all(&configured).unwrap();
    let serial: [u8; 32] = xproto::ClientMessageEvent {
        response_type: xproto::CLIENT_MESSAGE_EVENT | 128,
        sequence: 67,
        format: 32,
        window: 10,
        type_: serial_atom,
        data: [7u32, 1, 0, 0, 0].into(),
    }
    .into();
    peer.write_all(&serial).unwrap();
    assert_eq!(
        drive(&mut xwm, now),
        vec![Action::Changed(id), Action::Changed(id)]
    );
    assert_eq!(xwm.windows().unwrap().get(10).unwrap().geometry.width, 800);
    assert!(xwm.dispatch(now).unwrap().enumerated);
    xwm.notify_configured(
        id,
        Geometry {
            x: -22,
            y: 28,
            width: 800,
            height: 600,
            border: 2,
        },
        None,
        now,
    )
    .unwrap();
    drive(&mut xwm, now);
    let sent = request(&mut peer);
    assert_eq!(&sent[..2], &[25, 0]);
    assert_eq!(u32::from_ne_bytes(sent[4..8].try_into().unwrap()), 10);
    assert_eq!(
        u32::from_ne_bytes(sent[8..12].try_into().unwrap()),
        u32::from(xproto::EventMask::STRUCTURE_NOTIFY)
    );
    let (event, _) = xproto::ConfigureNotifyEvent::try_parse(&sent[12..]).unwrap();
    assert_eq!(
        (event.event, event.window, event.above_sibling),
        (10, 10, 0)
    );
    assert_eq!(
        (
            event.x,
            event.y,
            event.width,
            event.height,
            event.border_width
        ),
        (-22, 28, 800, 600, 2)
    );
    assert_eq!(request(&mut peer)[0], 43);
    write_reply(
        &mut peer,
        &xproto::GetInputFocusReply {
            sequence: 69,
            ..Default::default()
        }
        .serialize(),
    );
    assert!(drive(&mut xwm, now).is_empty());
    assert_eq!(xwm.windows().unwrap().get(10).unwrap().geometry.border, 0);
    assert!(xwm.close_window(id, 123, now).is_err());
    assert!(xwm.fd().is_some());
    xwm.refresh_protocols(id, now).unwrap();
    drive(&mut xwm, now);
    let attributes = request(&mut peer);
    assert_eq!(attributes[0], 2);
    assert_eq!(
        u32::from_ne_bytes(attributes[12..16].try_into().unwrap()),
        u32::from(xproto::EventMask::PROPERTY_CHANGE)
    );
    assert_eq!(request(&mut peer)[0], 43);
    write_reply(
        &mut peer,
        &xproto::GetInputFocusReply {
            sequence: 71,
            ..Default::default()
        }
        .serialize(),
    );
    let property = request(&mut peer);
    assert_eq!(property[0], 20);
    assert_eq!(u32::from_ne_bytes(property[4..8].try_into().unwrap()), 10);
    assert_eq!(u32::from_ne_bytes(property[8..12].try_into().unwrap()), 104);
    assert_eq!(
        u32::from_ne_bytes(property[20..24].try_into().unwrap()),
        256
    );
    let value = [105u32, 106]
        .into_iter()
        .flat_map(u32::to_ne_bytes)
        .collect();
    let protocol_reply = xproto::GetPropertyReply {
        sequence: 72,
        format: 32,
        length: 2,
        type_: 4,
        bytes_after: 0,
        value_len: 2,
        value,
    }
    .serialize();
    peer.write_all(&protocol_reply).unwrap();
    assert_eq!(drive(&mut xwm, now), vec![Action::ProtocolsChanged(id)]);
    assert_eq!(
        xwm.protocols(id),
        Some(Protocols {
            delete_window: true,
            take_focus: true,
            sync_request: false
        })
    );
    xwm.close_window(id, 123, now).unwrap();
    drive(&mut xwm, now);
    let close = request(&mut peer);
    assert_eq!(&close[..2], &[25, 0]);
    assert_eq!(u32::from_ne_bytes(close[4..8].try_into().unwrap()), 10);
    assert_eq!(u32::from_ne_bytes(close[8..12].try_into().unwrap()), 0);
    let (message, _) = xproto::ClientMessageEvent::try_parse(&close[12..]).unwrap();
    assert_eq!(
        (message.window, message.type_, message.format),
        (10, 104, 32)
    );
    assert_eq!(message.data.as_data32(), [105, 123, 0, 0, 0]);
    assert_eq!(request(&mut peer)[0], 43);
    write_reply(
        &mut peer,
        &xproto::GetInputFocusReply {
            sequence: 74,
            ..Default::default()
        }
        .serialize(),
    );
    assert!(drive(&mut xwm, now).is_empty());
    assert!(xwm.windows().unwrap().get(10).is_some());
    assert!(
        xwm.committed_surface(1, 200, (1u64 << 32) | 7)
            .unwrap()
            .is_some()
    );
    assert_eq!(xwm.windows().unwrap().presentable_surface(id), Some(200));
    assert!(xwm.focus_window(id, 124, now).is_err());
    xwm.refresh_input_hints(id).unwrap();
    drive(&mut xwm, now);
    let hints = request(&mut peer);
    assert_eq!(hints[0], 20);
    assert_eq!(u32::from_ne_bytes(hints[8..12].try_into().unwrap()), 35);
    assert_eq!(u32::from_ne_bytes(hints[12..16].try_into().unwrap()), 35);
    assert_eq!(u32::from_ne_bytes(hints[20..24].try_into().unwrap()), 9);
    let hints_reply = |sequence, input| {
        xproto::GetPropertyReply {
            sequence,
            format: 32,
            length: 9,
            type_: 35,
            bytes_after: 0,
            value_len: 9,
            value: [1u32, input, 0, 0, 0, 0, 0, 0, 0]
                .into_iter()
                .flat_map(u32::to_ne_bytes)
                .collect(),
        }
        .serialize()
    };
    peer.write_all(&hints_reply(75, 1)).unwrap();
    assert_eq!(drive(&mut xwm, now), vec![Action::HintsChanged(id)]);
    // Leave capacity for one checked command, but not the complete focus pair.
    let (mut blocked_transport, mut blocked_requests, _, _blocked_peer, _) = ready();
    let (bytes, _) = Request::serialize(xproto::GetInputFocusRequest, 0);
    while blocked_requests.available_slots() > 3 {
        blocked_requests
            .queue(
                &mut blocked_transport,
                bytes.clone(),
                super::super::requests::ReplyKind::Reply,
                super::super::requests::Importance::Optional,
                now + Duration::from_secs(10),
            )
            .unwrap();
    }
    let Some(Phase::Tracking(tracking)) = xwm.phase.as_mut() else {
        panic!()
    };
    std::mem::swap(&mut tracking.transport, &mut blocked_transport);
    std::mem::swap(&mut tracking.requests, &mut blocked_requests);
    assert!(xwm.focus_window(id, 124, now).is_err());
    let Some(Phase::Tracking(tracking)) = xwm.phase.as_mut() else {
        panic!("backpressure destroyed tracking")
    };
    assert_eq!(tracking.requests.available_slots(), 3);
    assert_eq!(tracking.commands.available_groups(), 256);
    assert_eq!(tracking.last_focus_request, None);
    std::mem::swap(&mut tracking.transport, &mut blocked_transport);
    std::mem::swap(&mut tracking.requests, &mut blocked_requests);
    assert_eq!(
        xwm.focus_window(id, 124, now).unwrap(),
        FocusModel::LocallyActive
    );
    drive(&mut xwm, now);
    let focus = request(&mut peer);
    assert_eq!(&focus[..2], &[42, 2]);
    assert_eq!(u32::from_ne_bytes(focus[4..8].try_into().unwrap()), 10);
    assert_eq!(u32::from_ne_bytes(focus[8..12].try_into().unwrap()), 124);
    assert_eq!(request(&mut peer)[0], 43);
    let take = request(&mut peer);
    assert_eq!(take[0], 25);
    let (message, _) = xproto::ClientMessageEvent::try_parse(&take[12..]).unwrap();
    assert_eq!(message.data.as_data32(), [106, 124, 0, 0, 0]);
    assert_eq!(request(&mut peer)[0], 43);
    for sequence in [77, 79] {
        write_reply(
            &mut peer,
            &xproto::GetInputFocusReply {
                sequence,
                ..Default::default()
            }
            .serialize(),
        );
    }
    assert!(drive(&mut xwm, now).is_empty());
    let changed: [u8; 32] = xproto::PropertyNotifyEvent {
        response_type: xproto::PROPERTY_NOTIFY_EVENT,
        sequence: 79,
        window: 10,
        atom: 35,
        time: 125,
        state: xproto::Property::NEW_VALUE,
    }
    .into();
    peer.write_all(&changed).unwrap();
    drive(&mut xwm, now);
    assert_eq!(xwm.input_hints(id), None);
    assert!(xwm.focus_window(id, 125, now).is_err());
    assert_eq!(request(&mut peer)[0], 20);
    peer.write_all(&hints_reply(80, 0)).unwrap();
    assert_eq!(drive(&mut xwm, now), vec![Action::HintsChanged(id)]);
    assert_eq!(
        xwm.focus_window(id, 125, now).unwrap(),
        FocusModel::GloballyActive
    );
    drive(&mut xwm, now);
    assert_eq!(request(&mut peer)[0], 25); // No SetInputFocus for globally active clients.
    assert_eq!(request(&mut peer)[0], 43);
    write_reply(
        &mut peer,
        &xproto::GetInputFocusReply {
            sequence: 82,
            ..Default::default()
        }
        .serialize(),
    );
    assert!(drive(&mut xwm, now).is_empty());
    assert!(xwm.focus_window(id, 1, now).is_err());
    assert!(xwm.focus_window(id, 0, now).is_err());
    xwm.refresh_normal_hints(id).unwrap();
    drive(&mut xwm, now);
    let normal = request(&mut peer);
    assert_eq!(u32::from_ne_bytes(normal[8..12].try_into().unwrap()), 40);
    assert_eq!(u32::from_ne_bytes(normal[12..16].try_into().unwrap()), 41);
    let normal_reply = xproto::GetPropertyReply {
        sequence: 83,
        format: 32,
        type_: 41,
        length: 18,
        value_len: 18,
        value: [16u32, 0, 0, 0, 0, 100, 80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
            .into_iter()
            .flat_map(u32::to_ne_bytes)
            .collect(),
        ..Default::default()
    }
    .serialize();
    peer.write_all(&normal_reply).unwrap();
    let actions = drive(&mut xwm, now);
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, Action::NormalHintsChanged(w) if *w == id))
    );
    assert_eq!(
        xwm.normal_hints(id).unwrap().minimum_size(),
        crate::foundation::SizeI {
            width: 100,
            height: 80
        }
    );
    let before = xwm.windows().unwrap().get(id.xid).unwrap().geometry;
    let accepted = Geometry {
        x: 10,
        y: 20,
        width: 120,
        height: 90,
        border: 0,
    };
    assert!(
        xwm.configure_window_with_hints(
            id,
            Geometry {
                width: 99,
                ..accepted
            },
            now
        )
        .is_err()
    );
    drive(&mut xwm, now);
    peer.set_nonblocking(true).unwrap();
    assert_eq!(
        peer.read(&mut [0; 1]).unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    peer.set_nonblocking(false).unwrap();
    xwm.configure_window_with_hints(id, accepted, now).unwrap();
    drive(&mut xwm, now);
    let configure = request(&mut peer);
    assert_eq!(configure[0], 12);
    assert_eq!(
        u32::from_ne_bytes(configure[4..8].try_into().unwrap()),
        id.xid
    );
    assert_eq!(
        u32::from_ne_bytes(configure[20..24].try_into().unwrap()),
        120
    );
    assert_eq!(
        u32::from_ne_bytes(configure[24..28].try_into().unwrap()),
        90
    );
    assert_eq!(request(&mut peer)[0], 43);
    assert_eq!(xwm.windows().unwrap().get(id.xid).unwrap().geometry, before);
    write_reply(
        &mut peer,
        &xproto::GetInputFocusReply {
            sequence: 85,
            ..Default::default()
        }
        .serialize(),
    );
    drive(&mut xwm, now);
    assert_eq!(xwm.windows().unwrap().get(id.xid).unwrap().geometry, before);
    let changed: [u8; 32] = xproto::PropertyNotifyEvent {
        response_type: xproto::PROPERTY_NOTIFY_EVENT,
        sequence: 85,
        window: id.xid,
        atom: 40,
        ..Default::default()
    }
    .into();
    peer.write_all(&changed).unwrap();
    drive(&mut xwm, now);
    assert_eq!(xwm.normal_hints(id), None);
    assert!(xwm.configure_window_with_hints(id, accepted, now).is_err());
    assert_eq!(request(&mut peer)[0], 20);
    peer.write_all(
        &xproto::GetPropertyReply {
            sequence: 86,
            ..Default::default()
        }
        .serialize(),
    )
    .unwrap();
    drive(&mut xwm, now);
    assert_eq!(
        xwm.normal_hints(id),
        Some(super::super::normal_hints::NormalHints::default())
    );
    assert!(!xwm.selection_watches_ready());
    let subscription = xwm
        .watch_standard_selection(super::super::selection::Selection::Primary, manager, now)
        .unwrap();
    assert_eq!(subscription.window, manager);
    assert!(!xwm.selection_watches_ready());
    drive(&mut xwm, now);
    let watch = request(&mut peer);
    assert_eq!(watch[1], 2);
    assert_eq!(u32::from_ne_bytes(watch[8..12].try_into().unwrap()), 1);
    assert_eq!(request(&mut peer)[0], 43);
    write_reply(
        &mut peer,
        &xproto::GetInputFocusReply {
            sequence: 88,
            ..Default::default()
        }
        .serialize(),
    );
    drive(&mut xwm, now);
    assert!(
        xwm.selection_watches_ready(),
        "pending checks: {:?}",
        match xwm.phase.as_ref().unwrap() {
            Phase::Tracking(t) => &t.selection_checks,
            _ => unreachable!(),
        }
    );
    assert!(
        xwm.watch_selection(super::super::selection::Selection::Primary, 1, manager, now)
            .is_err()
    );
    assert!(xwm.fd().is_some());
    assert!(xwm.selection_watches_ready());
    // One server atom must never route to both independent ownership ledgers.
    assert!(
        xwm.watch_selection(
            super::super::selection::Selection::Clipboard,
            1,
            manager,
            now
        )
        .is_err()
    );
    // Invalid inputs also reject before changing the live subscription.
    assert!(
        xwm.watch_selection(
            super::super::selection::Selection::Clipboard,
            0,
            manager,
            now
        )
        .is_err()
    );
    assert!(xwm.fd().is_some());
    assert!(xwm.selection_watches_ready());
    let notification: [u8; 32] = x11rb_protocol::protocol::xfixes::SelectionNotifyEvent {
        response_type: subscription.first_event,
        sequence: 88,
        window: manager,
        owner: 77,
        selection: 1,
        timestamp: 130,
        selection_timestamp: 129,
        ..Default::default()
    }
    .into();
    peer.write_all(&notification).unwrap();
    let mut owners = super::super::selection::Ownerships::new(subscription.generation).unwrap();
    let mut observed = false;
    for _ in 0..10 {
        for event in xwm.dispatch(now).unwrap().events {
            if let super::super::selection::OwnershipUpdate::Replaced(change) =
                subscription.event(&event, &mut owners).unwrap()
            {
                assert_eq!(
                    change.current.owner,
                    super::super::selection::Owner::X11(77)
                );
                observed = true;
            }
        }
    }
    assert!(observed);
    let clipboard = xwm
        .watch_standard_selection(super::super::selection::Selection::Clipboard, manager, now)
        .unwrap();
    assert_eq!(clipboard.atom, clipboard_atom);
    let atoms = xwm.selection_atoms().unwrap();
    assert_eq!(
        atoms
            .identify(clipboard.generation, clipboard.atom)
            .unwrap(),
        Some(super::super::selection::Selection::Clipboard)
    );
    assert_eq!(
        atoms
            .identify(subscription.generation, subscription.atom)
            .unwrap(),
        Some(super::super::selection::Selection::Primary)
    );
    assert_eq!(
        atoms.identify(clipboard.generation, atoms.incr).unwrap(),
        None
    );
    assert!(
        atoms
            .identify(clipboard.generation + 1, clipboard.atom)
            .is_err()
    );
    assert_ne!(clipboard.atom, subscription.atom);
    assert!(!xwm.selection_watches_ready());
    drive(&mut xwm, now);
    let watch = request(&mut peer);
    assert_eq!(
        u32::from_ne_bytes(watch[8..12].try_into().unwrap()),
        clipboard_atom
    );
    assert_eq!(request(&mut peer)[0], 43);
    write_reply(
        &mut peer,
        &xproto::GetInputFocusReply {
            sequence: 90,
            ..Default::default()
        }
        .serialize(),
    );
    drive(&mut xwm, now);
    assert!(xwm.selection_watches_ready());
    let clock_sequence = xwm.request_focus_timestamp(id, now).unwrap();
    assert!(xwm.commands_pending(id));
    assert_eq!(clock_sequence, 91);
    drive(&mut xwm, now);
    let marker = request(&mut peer);
    assert_eq!(marker[0], 18);
    assert_eq!(
        u32::from_ne_bytes(marker[4..8].try_into().unwrap()),
        manager
    );
    assert_eq!(request(&mut peer)[0], 43);
    let clock: [u8; 32] = xproto::PropertyNotifyEvent {
        response_type: xproto::PROPERTY_NOTIFY_EVENT,
        sequence: clock_sequence,
        window: manager,
        atom: u32::from_ne_bytes(marker[8..12].try_into().unwrap()),
        time: 200,
        state: xproto::Property::NEW_VALUE,
    }
    .into();
    assert_eq!(xwm.focus_timestamp(&clock, clock_sequence), Some(200));
    assert_eq!(xwm.focus_timestamp(&clock, clock_sequence - 1), None);
    let mut forged = clock;
    forged[0] |= 0x80;
    assert_eq!(xwm.focus_timestamp(&forged, clock_sequence), None);
    peer.write_all(&clock).unwrap();
    write_reply(
        &mut peer,
        &xproto::GetInputFocusReply {
            sequence: 92,
            ..Default::default()
        }
        .serialize(),
    );
    drive(&mut xwm, now);
    assert!(!xwm.commands_pending(id));
    let timestamp = xwm.focus_timestamp(&clock, clock_sequence).unwrap();
    xwm.close_window(id, timestamp, now).unwrap();
    drive(&mut xwm, now);
    let close = request(&mut peer);
    let (message, _) = xproto::ClientMessageEvent::try_parse(&close[12..]).unwrap();
    assert_eq!(message.data.as_data32(), [105, 200, 0, 0, 0]);
    assert_eq!(request(&mut peer)[0], 43);
    write_reply(
        &mut peer,
        &xproto::GetInputFocusReply {
            sequence: 94,
            ..Default::default()
        }
        .serialize(),
    );
    drive(&mut xwm, now);
    assert!(!xwm.commands_pending(id));
    let clear: [u8; 32] = xproto::SelectionClearEvent {
        response_type: xproto::SELECTION_CLEAR_EVENT,
        sequence: 94,
        time: 124,
        owner: manager,
        selection: wm_atom,
    }
    .into();
    peer.write_all(&clear).unwrap();
    assert!(xwm.dispatch(now).is_err());
    assert!(xwm.fd().is_none());
    assert!(xwm.windows().is_none());
    assert!(xwm.selection_atoms().is_none());
    assert!(xwm.dispatch(now).is_err());
}
#[test]
fn setup_deadline_closes_only_driver_transport() {
    let (socket, _peer) = UnixStream::pair().unwrap();
    let now = Instant::now();
    let mut xwm = Xwm::new(socket, 1, now).unwrap();
    assert!(xwm.dispatch(now + Duration::from_secs(10)).is_err());
    assert!(xwm.fd().is_none());
    assert!(xwm.deadline().is_none());
}

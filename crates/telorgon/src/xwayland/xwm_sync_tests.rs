use super::super::{
    discovery::tests::{flush_requests, ready, request},
    resize_sync::ResizeSyncStatus,
};
use super::*;
use x11rb_protocol::{protocol::sync, x11_utils::Serialize};

#[test]
fn sync_message_precedes_configure_and_server_barrier_does_not_ack_repaint() {
    let (transport, requests, discovered, mut peer, now) = ready();
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
    let opcode = discovered.extensions["SYNC"].major_opcode;
    let sync_atom = discovered.atoms["_NET_WM_SYNC_REQUEST"];
    let counter_atom = discovered.atoms["_NET_WM_SYNC_REQUEST_COUNTER"];
    let mut t = Tracking {
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
        last_focus_request: None,
    };
    t.resize_sync.capability(id, Some(100), now).unwrap();
    t.resize_sync
        .schedule(
            opcode,
            &mut t.transport,
            &mut t.requests,
            now,
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap();
    flush_requests(&mut t.transport);
    assert_eq!(&request(&mut peer)[..2], &[opcode, 3]);
    assert_eq!(&request(&mut peer)[..2], &[opcode, 5]);
    let sequence = t.requests.last_sequence();
    let mut response = sync::QueryCounterReply {
        sequence: sequence as u16,
        length: 0,
        counter_value: sync::Int64 { hi: 0, lo: 0 },
    }
    .serialize()
    .to_vec();
    response.resize(32, 0);
    for completion in t.requests.ingest(response).unwrap() {
        assert!(t.resize_sync.completion(&completion, now));
    }
    let mut xwm = Xwm {
        root_cursor: None,
        phase: Some(Phase::Tracking(t)),
        windows: Some(windows),
        inspector: Inspector::new(),
        early_events: VecDeque::new(),
        deadline: now + Duration::from_secs(10),
    };
    let geometry = Geometry {
        x: 0,
        y: 0,
        width: 800,
        height: 600,
        border: 0,
    };
    assert!(xwm.configure_window_synced(id, geometry, now).unwrap());
    let Some(Phase::Tracking(t)) = &mut xwm.phase else {
        unreachable!()
    };
    flush_requests(&mut t.transport);
    let sent = request(&mut peer);
    assert_eq!(sent[0], 25);
    let (message, _) = xproto::ClientMessageEvent::try_parse(&sent[12..]).unwrap();
    assert_eq!(message.window, 10);
    assert_eq!(message.data.as_data32(), [sync_atom, 123, 1, 0, 0]);
    assert_eq!(request(&mut peer)[0], 43);
    assert_eq!(request(&mut peer)[0], 12);
    assert_eq!(request(&mut peer)[0], 43);
    let sequence = t.requests.last_sequence();
    for sequence in [sequence - 2, sequence] {
        let mut bytes = xproto::GetInputFocusReply {
            sequence: sequence as u16,
            ..Default::default()
        }
        .serialize()
        .to_vec();
        bytes.resize(32, 0);
        for c in t.requests.ingest(bytes).unwrap() {
            assert!(
                t.commands
                    .completion(&c, xwm.windows.as_ref().unwrap(), &mut vec![])
                    .unwrap()
            );
        }
    }
    assert!(!xwm.commands_pending(id));
    assert!(xwm.repaint_pending(id));
    assert_eq!(xwm.resize_sync_status(id), ResizeSyncStatus::Waiting);
    assert!(!xwm.configure_window_synced(id, geometry, now).unwrap());

    // An explicit legacy close must reject unknown metadata and stale identities,
    // then address only the X connection owning the validated window resource.
    assert!(xwm.close_legacy_window(id, now).is_err());
    let Some(Phase::Tracking(t)) = &mut xwm.phase else {
        unreachable!()
    };
    t.protocols.refresh(id).unwrap();
    t.protocols
        .schedule(
            &mut t.transport,
            &mut t.requests,
            now + Duration::from_secs(1),
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap();
    flush_requests(&mut t.transport);
    assert_eq!(request(&mut peer)[0], 20);
    let mut bytes = xproto::GetPropertyReply {
        sequence: t.requests.last_sequence() as u16,
        type_: xproto::AtomEnum::ATOM.into(),
        format: 32,
        ..Default::default()
    }
    .serialize();
    bytes.resize(32, 0);
    for c in t.requests.ingest(bytes).unwrap() {
        assert!(
            t.protocols
                .completion(&c, xwm.windows.as_ref().unwrap(), &mut vec![])
                .unwrap()
        );
    }
    let stale = XWindow {
        incarnation: id.incarnation + 1,
        ..id
    };
    assert!(xwm.close_legacy_window(stale, now).is_err());
    assert!(xwm.close_window(id, 123, now).is_err());
    xwm.close_legacy_window(id, now).unwrap();
    let Some(Phase::Tracking(t)) = &mut xwm.phase else {
        unreachable!()
    };
    flush_requests(&mut t.transport);
    let kill = request(&mut peer);
    assert_eq!(kill[0], 113);
    assert_eq!(u32::from_ne_bytes(kill[4..8].try_into().unwrap()), id.xid);
    assert_eq!(request(&mut peer)[0], 43);
}

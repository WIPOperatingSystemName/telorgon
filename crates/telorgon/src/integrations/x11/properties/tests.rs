use super::super::{discovery::tests::ready, requests::RequestId};
use super::*;
use std::time::Duration;
use x11rb_protocol::x11_utils::Serialize;
fn window() -> (Windows, XWindow) {
    let mut w = Windows::new(1, 16, 1, 2, 100).unwrap();
    let event: [u8; 32] = xproto::CreateNotifyEvent {
        response_type: xproto::CREATE_NOTIFY_EVENT,
        parent: 1,
        window: 10,
        width: 1,
        height: 1,
        ..Default::default()
    }
    .into();
    w.event(&event).unwrap();
    let id = w.get(10).unwrap().id;
    (w, id)
}
fn reply(sequence: u64, atoms: &[u32]) -> Completion {
    let value = atoms.iter().flat_map(|atom| atom.to_ne_bytes()).collect();
    Completion::Reply(
        RequestId {
            generation: 1,
            sequence,
        },
        xproto::GetPropertyReply {
            sequence: sequence as u16,
            format: 32,
            length: atoms.len() as u32,
            type_: 4,
            bytes_after: 0,
            value_len: atoms.len() as u32,
            value,
        }
        .serialize(),
    )
}
fn motif_reply(sequence: u64, flags: u32, mask: u32) -> Completion {
    let mut result = reply(sequence, &[flags, 0, mask, 0, 0]);
    if let Completion::Reply(_, bytes) = &mut result {
        bytes[8..12].copy_from_slice(&200u32.to_ne_bytes());
    }
    result
}
#[test]
fn application_icons_and_classes_validate_payloads() {
    let icon_reply = |words: &[u32]| {
        xproto::GetPropertyReply {
            format: 32,
            type_: xproto::AtomEnum::CARDINAL.into(),
            length: words.len() as u32,
            value_len: words.len() as u32,
            value: words.iter().flat_map(|v| v.to_ne_bytes()).collect(),
            ..Default::default()
        }
        .serialize()
    };
    let icon = PropertyReader::parse_icon(&icon_reply(&[1, 1, 0x80402010])).unwrap();
    assert_eq!(icon.pixels.as_ref(), &[0x40, 0x20, 0x10, 0x80]);
    // Retain high-resolution client artwork instead of choosing the old 32px target.
    let mut variants = vec![32, 32];
    variants.extend(vec![0xff112233; 32 * 32]);
    variants.extend([128, 128]);
    variants.extend(vec![0xff445566; 128 * 128]);
    let large = PropertyReader::parse_icon(&icon_reply(&variants)).unwrap();
    assert_eq!(large.pixels.len(), 128 * 128 * 4);
    assert_eq!(&large.pixels[..4], &[0x44, 0x55, 0x66, 0xff]);
    assert!(PropertyReader::parse_icon(&icon_reply(&[4096, 4096, 0])).is_none());
    assert!(PropertyReader::parse_icon(&icon_reply(&[0, 1])).is_none());
    let class = PropertyReader::new_text(
        xproto::AtomEnum::WM_CLASS.into(),
        xproto::AtomEnum::STRING.into(),
        false,
    );
    let reply = |value: &[u8]| {
        xproto::GetPropertyReply {
            format: 8,
            type_: xproto::AtomEnum::STRING.into(),
            length: value.len().div_ceil(4) as u32,
            value_len: value.len() as u32,
            value: value.to_vec(),
            ..Default::default()
        }
        .serialize()
    };
    assert_eq!(
        class
            .parse_text(&reply(b"firefox\0Firefox\0"), false)
            .as_deref(),
        Some("Firefox")
    );
    assert!(
        class
            .parse_text(&reply(b"missing-delimiter"), false)
            .is_none()
    );
}
#[test]
fn motif_decorations_validate_flags_masks_and_wire_shape() {
    let reader = PropertyReader::new_decorations(200);
    for (flags, mask, expected) in [
        (0, 0, true),
        (2, 0, false),
        (2, 1, true),
        (2, 8, true),
        (2, 0x7f, false),
    ] {
        let Completion::Reply(_, bytes) = motif_reply(1, flags, mask) else {
            unreachable!()
        };
        assert_eq!(reader.parse_decorations(&bytes), Some(expected));
    }
    let Completion::Reply(_, bytes) = motif_reply(1, 2, 0) else {
        unreachable!()
    };
    for (offset, value) in [(1, 8), (8, 201), (12, 1), (16, 4)] {
        let mut bad = bytes.clone();
        bad[offset] = value;
        assert_eq!(reader.parse_decorations(&bad), None);
    }
    assert_eq!(reader.parse_decorations(&bytes[..48]), None);
}
#[test]
fn motif_refresh_coalesces_and_deleted_or_invalid_properties_restore_decorations() {
    let (mut transport, mut requests, _, _peer, now) = ready();
    let (windows, id) = window();
    let mut reader = PropertyReader::new_decorations(200);
    let mut actions = Vec::new();
    let schedule =
        |reader: &mut PropertyReader, transport: &mut Transport, requests: &mut Requests| {
            reader
                .schedule(
                    transport,
                    requests,
                    now + Duration::from_secs(10),
                    Instant::now() + Duration::from_secs(1),
                )
                .unwrap();
            requests.last_sequence()
        };
    reader.refresh(id).unwrap();
    let first = schedule(&mut reader, &mut transport, &mut requests);
    reader
        .completion(&motif_reply(first, 2, 0), &windows, &mut actions)
        .unwrap();
    assert_eq!(reader.decorations(id), Some(false));
    reader.refresh(id).unwrap();
    let stale = schedule(&mut reader, &mut transport, &mut requests);
    reader.refresh(id).unwrap();
    assert_eq!(reader.decorations(id), Some(false)); // No flash while the property read is pending.
    actions.clear();
    reader
        .completion(&motif_reply(stale, 2, 1), &windows, &mut actions)
        .unwrap();
    assert!(actions.is_empty());
    assert_eq!(reader.decorations(id), Some(false));
    let latest = schedule(&mut reader, &mut transport, &mut requests);
    reader
        .completion(&motif_reply(latest, 2, 0), &windows, &mut actions)
        .unwrap();
    for invalid in [true, false] {
        reader.refresh(id).unwrap();
        let sequence = schedule(&mut reader, &mut transport, &mut requests);
        let completion = if invalid {
            reply(sequence, &[2, 0, 0, 0, 0])
        } else {
            Completion::Reply(
                RequestId {
                    generation: 1,
                    sequence,
                },
                xproto::GetPropertyReply::default().serialize(),
            )
        };
        reader
            .completion(&completion, &windows, &mut actions)
            .unwrap();
        assert_eq!(reader.decorations(id), Some(true));
    }
    reader.refresh(id).unwrap();
    let sequence = schedule(&mut reader, &mut transport, &mut requests);
    reader.forget(id);
    reader
        .completion(&motif_reply(sequence, 2, 0), &windows, &mut actions)
        .unwrap();
    assert_eq!(reader.decorations(id), None);
}
#[test]
fn basic_sync_counter_requires_bounded_cardinal_and_advertised_protocol() {
    let p = PropertyReader::new(104, 105, 106).with_sync_request(120);
    let Completion::Reply(_, bytes) = reply(1, &[120]) else {
        unreachable!()
    };
    assert!(p.parse(&bytes).unwrap().sync_request);
    let make = |values: &[u32]| {
        xproto::GetPropertyReply {
            format: 32,
            length: values.len() as u32,
            type_: xproto::AtomEnum::CARDINAL.into(),
            value_len: values.len() as u32,
            value: values.iter().flat_map(|v| v.to_ne_bytes()).collect(),
            ..Default::default()
        }
        .serialize()
    };
    assert_eq!(PropertyReader::parse_counter(&make(&[42])), Some(42));
    assert_eq!(PropertyReader::parse_counter(&make(&[42, 43])), Some(42));
    assert_eq!(PropertyReader::parse_counter(&make(&[0])), None);
    assert_eq!(PropertyReader::parse_counter(&make(&[42, 43, 44])), None);
    let mut malformed = make(&[42]);
    malformed[8..12].copy_from_slice(&u32::from(xproto::AtomEnum::ATOM).to_ne_bytes());
    assert_eq!(PropertyReader::parse_counter(&malformed), None);
    let mut partial = make(&[42]);
    partial[12..16].copy_from_slice(&4u32.to_ne_bytes());
    assert_eq!(PropertyReader::parse_counter(&partial), None);
}
#[test]
fn frame_titles_are_bounded_and_encoding_checked() {
    let reader = PropertyReader::new_text(200, 201, true);
    let make_reply = |value: Vec<u8>, bytes_after: u32, type_: u32| {
        let mut bytes = xproto::GetPropertyReply {
            format: 8,
            sequence: 1,
            length: value.len().div_ceil(4) as u32,
            type_,
            bytes_after,
            value_len: value.len() as u32,
            value,
        }
        .serialize();
        bytes.resize(bytes.len().div_ceil(4) * 4, 0);
        bytes
    };
    assert_eq!(
        reader.parse_text(
            &make_reply("Title — 日本語".as_bytes().to_vec(), 0, 201),
            true
        ),
        Some("Title — 日本語".into())
    );
    assert!(
        reader
            .parse_text(&make_reply(vec![0xff], 0, 201), true)
            .is_none()
    );
    assert!(
        reader
            .parse_text(&make_reply(vec![b'a'; 4097], 0, 201), true)
            .is_none()
    );
    assert!(
        reader
            .parse_text(&make_reply(b"title".to_vec(), 1, 201), true)
            .is_none()
    );
    assert!(
        reader
            .parse_text(&make_reply(b"title".to_vec(), 0, 202), true)
            .is_none()
    );
    let legacy = PropertyReader::new_text(
        xproto::AtomEnum::WM_NAME.into(),
        xproto::AtomEnum::STRING.into(),
        false,
    );
    assert_eq!(
        legacy.parse_text(
            &make_reply(vec![0xe9], 0, xproto::AtomEnum::STRING.into()),
            false
        ),
        Some("é".into())
    );
}

#[test]
fn normal_hints_reads_are_bounded_and_stale_replies_cannot_restore_constraints() {
    use std::io::Read;
    let (mut transport, mut requests, _, mut peer, now) = ready();
    let (windows, id) = window();
    let mut reader = PropertyReader::new_normal_hints();
    reader.refresh(id).unwrap();
    let deadline = now + Duration::from_secs(10);
    reader
        .schedule(
            &mut transport,
            &mut requests,
            deadline,
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap();
    transport.dispatch().unwrap();
    let mut wire = [0; 24];
    peer.read_exact(&mut wire).unwrap();
    assert_eq!(wire[0], 20);
    assert_eq!(
        u32::from_ne_bytes(wire[8..12].try_into().unwrap()),
        u32::from(xproto::AtomEnum::WM_NORMAL_HINTS)
    );
    assert_eq!(
        u32::from_ne_bytes(wire[12..16].try_into().unwrap()),
        u32::from(xproto::AtomEnum::WM_SIZE_HINTS)
    );
    assert_eq!(u32::from_ne_bytes(wire[20..24].try_into().unwrap()), 18);
    let response = |sequence| {
        let mut completion = reply(
            sequence,
            &[16, 0, 0, 0, 0, 80, 60, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        );
        if let Completion::Reply(_, bytes) = &mut completion {
            bytes[8..12]
                .copy_from_slice(&u32::from(xproto::AtomEnum::WM_SIZE_HINTS).to_ne_bytes());
        }
        completion
    };
    reader.refresh(id).unwrap();
    let mut actions = Vec::new();
    reader
        .completion(&response(42), &windows, &mut actions)
        .unwrap();
    assert!(actions.is_empty());
    assert_eq!(reader.normal_hints(id), None);
    reader
        .schedule(
            &mut transport,
            &mut requests,
            deadline,
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap();
    reader
        .completion(&response(43), &windows, &mut actions)
        .unwrap();
    assert!(
        matches!(actions.as_slice(), [Action::NormalHintsChanged(window)] if *window == id)
    );
    assert_eq!(
        reader.normal_hints(id).unwrap().minimum_size(),
        crate::foundation::SizeI {
            width: 80,
            height: 60
        }
    );
    reader.refresh(id).unwrap();
    assert_eq!(reader.normal_hints(id), None);
    reader
        .schedule(
            &mut transport,
            &mut requests,
            deadline,
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap();
    reader.forget(id);
    actions.clear();
    reader
        .completion(&response(44), &windows, &mut actions)
        .unwrap();
    assert!(actions.is_empty());
    assert_eq!(reader.normal_hints(id), None);
}
#[test]
fn property_reads_wait_for_shared_request_capacity() {
    let (mut transport, mut requests, _, _peer, now) = ready();
    let (_, id) = window();
    let mut reader = PropertyReader::new(104, 105, 106);
    reader.refresh(id).unwrap();
    let (bytes, _) = Request::serialize(xproto::GetInputFocusRequest, 0);
    while requests.available_slots() != 0 {
        requests
            .queue(
                &mut transport,
                bytes.clone(),
                ReplyKind::Reply,
                Importance::Optional,
                now + Duration::from_secs(1),
            )
            .unwrap();
    }
    assert!(
        !reader
            .schedule(
                &mut transport,
                &mut requests,
                now,
                Instant::now() + Duration::from_secs(1)
            )
            .unwrap()
    );
    assert!(reader.pending.is_empty());
    let mut response = xproto::GetInputFocusReply {
        sequence: 42,
        ..Default::default()
    }
    .serialize()
    .to_vec();
    response.resize(32, 0);
    requests.ingest(response).unwrap();
    assert_eq!(requests.available_slots(), 1);
    reader
        .schedule(
            &mut transport,
            &mut requests,
            now,
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap();
    assert_eq!(reader.pending.len(), 1);
    assert_eq!(requests.available_slots(), 0);
}
#[test]
fn input_hints_validate_layout_flags_and_default() {
    let reader = PropertyReader::new_hints();
    let make = |flags: u32, input: u32| {
        xproto::GetPropertyReply {
            format: 32,
            sequence: 1,
            length: 9,
            type_: 35,
            bytes_after: 0,
            value_len: 9,
            value: [flags, input, 0, 0, 0, 0, 0, 0, 0]
                .into_iter()
                .flat_map(u32::to_ne_bytes)
                .collect(),
        }
        .serialize()
    };
    assert_eq!(
        reader.parse_hints(&make(257, 0)),
        Some(InputHints {
            accepts_input: false,
            urgent: true
        })
    );
    assert_eq!(reader.parse_hints(&make(1, 1)), Some(InputHints::default()));
    assert_eq!(
        reader.parse_hints(&make(0, 999)),
        Some(InputHints::default())
    );
    assert_eq!(reader.parse_hints(&make(1, 2)), None);
    assert_eq!(reader.parse_hints(&make(1, 1)[..64]), None);
    let absent = xproto::GetPropertyReply {
        format: 0,
        sequence: 1,
        length: 0,
        type_: 0,
        bytes_after: 0,
        value_len: 0,
        value: vec![],
    }
    .serialize();
    assert_eq!(reader.parse_hints(&absent), Some(InputHints::default()));
}
#[test]
fn icccm_focus_models_follow_input_and_take_focus_independently() {
    for (input, take, expected) in [
        (false, false, FocusModel::NoInput),
        (true, false, FocusModel::Passive),
        (true, true, FocusModel::LocallyActive),
        (false, true, FocusModel::GloballyActive),
    ] {
        assert_eq!(
            FocusModel::from_hints(
                InputHints {
                    accepts_input: input,
                    urgent: false
                },
                Protocols {
                    delete_window: false,
                    take_focus: take,
                    sync_request: false
                }
            ),
            expected
        );
    }
}
#[test]
fn property_storm_coalesces_and_old_reply_cannot_restore_capabilities() {
    let (mut t, mut r, _, _peer, now) = ready();
    let (w, id) = window();
    let mut p = PropertyReader::new(104, 105, 106);
    p.refresh(id).unwrap();
    p.schedule(
        &mut t,
        &mut r,
        now + Duration::from_secs(1),
        Instant::now() + Duration::from_secs(1),
    )
    .unwrap();
    for _ in 0..100 {
        p.refresh(id).unwrap();
    }
    p.schedule(
        &mut t,
        &mut r,
        now + Duration::from_secs(1),
        Instant::now() + Duration::from_secs(1),
    )
    .unwrap();
    assert_eq!(p.pending.len(), 1);
    let mut actions = vec![];
    p.completion(&reply(42, &[105, 106]), &w, &mut actions)
        .unwrap();
    assert_eq!(p.get(id), None);
    assert!(actions.is_empty());
    p.schedule(
        &mut t,
        &mut r,
        now + Duration::from_secs(1),
        Instant::now() + Duration::from_secs(1),
    )
    .unwrap();
    p.completion(&reply(43, &[]), &w, &mut actions).unwrap();
    assert_eq!(p.get(id), Some(Protocols::default()));
    assert_eq!(actions, vec![Action::ProtocolsChanged(id)]);
}
#[test]
fn invalid_or_incomplete_properties_grant_no_capabilities() {
    let p = PropertyReader::new(104, 105, 106);
    let Completion::Reply(_, mut bytes) = reply(42, &[105]) else {
        unreachable!()
    };
    assert_eq!(
        p.parse(&bytes),
        Some(Protocols {
            delete_window: true,
            take_focus: false,
            sync_request: false
        })
    );
    bytes[12..16].copy_from_slice(&4u32.to_ne_bytes());
    assert_eq!(p.parse(&bytes), None);
    assert_eq!(p.parse(&vec![0; 1057]), None);
    bytes[1] = 8;
    assert_eq!(p.parse(&bytes), None);
}
#[test]
fn discarded_window_reply_cannot_publish_metadata() {
    let (mut t, mut r, _, _peer, now) = ready();
    let (w, id) = window();
    let mut p = PropertyReader::new(104, 105, 106);
    p.refresh(id).unwrap();
    p.schedule(&mut t, &mut r, now, Instant::now() + Duration::from_secs(1))
        .unwrap();
    p.forget(id);
    let mut actions = vec![];
    assert!(p.completion(&reply(42, &[105]), &w, &mut actions).unwrap());
    assert!(actions.is_empty());
    assert_eq!(p.get(id), None);
}

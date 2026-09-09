//! INCR property requests over the nonblocking XWM transport.
//! Returned request IDs must be retained by the selection owner: asynchronous
//! errors/timeouts cancel that transfer. Queueing is not server confirmation;
//! the owner must arrange reply barriers and route property events separately.
use super::{
    Error, Result,
    incr::{ReceiveState, Receiver, SendState, Sender},
    requests::{Importance, ReplyKind, RequestId, Requests},
    transport::Transport,
};
use std::{borrow::Cow, time::Instant};
use x11rb_protocol::{protocol::xproto, x11_utils::Request};

/// A property endpoint scoped to the owning Xwayland connection generation.
/// Selection ownership generations and XID incarnations are checked by the owner.
#[derive(Clone, Copy, Debug)]
pub struct PropertyTarget {
    pub generation: u64,
    pub window: u32,
    pub property: u32,
}
impl PropertyTarget {
    pub(crate) fn validate(self, requests: &Requests) -> Result<()> {
        if self.generation != requests.generation() || self.window == 0 || self.property == 0 {
            return Err(Error("invalid or stale INCR property endpoint".into()));
        }
        Ok(())
    }
}

/// Reply to a routed SelectionRequest. `None` refuses the conversion; `Some`
/// names the property already stored on the requestor. For legacy requests with
/// no property the owner supplies its chosen nonzero property. The caller verifies
/// ownership/timestamps and preserves arrival order for otherwise identical requests.
/// Retain the returned ID for asynchronous SendEvent errors and a reply barrier.
pub fn notify(
    generation: u64,
    request: &xproto::SelectionRequestEvent,
    property: Option<u32>,
    transport: &mut Transport,
    requests: &mut Requests,
    deadline: Instant,
) -> Result<RequestId> {
    if generation != requests.generation()
        || request.response_type & 0x7f != xproto::SELECTION_REQUEST_EVENT
        || request.requestor == 0
        || request.owner == 0
        || request.selection == 0
        || request.target == 0
        || property.is_some_and(|p| p == 0 || (request.property != 0 && request.property != p))
    {
        return Err(Error("invalid or stale selection notification".into()));
    }
    let event: [u8; 32] = xproto::SelectionNotifyEvent {
        response_type: xproto::SELECTION_NOTIFY_EVENT,
        sequence: 0,
        time: request.time,
        requestor: request.requestor,
        selection: request.selection,
        target: request.target,
        property: property.unwrap_or(0),
    }
    .into();
    let (bytes, fds) = Request::serialize(
        xproto::SendEventRequest {
            propagate: false,
            destination: request.requestor,
            event_mask: xproto::EventMask::NO_EVENT,
            event: Cow::Owned(event),
        },
        0,
    );
    debug_assert!(fds.is_empty());
    requests.queue(
        transport,
        bytes,
        ReplyKind::Void,
        Importance::Optional,
        deadline,
    )
}

/// Announce an unknown-length stream using the valid lower bound zero.
/// The owner must queue SelectionNotify after this succeeds, and cancel if that
/// notification cannot be sent. This request alone does not notify the requestor.
pub fn announce(
    sender: &mut Sender,
    target: PropertyTarget,
    incr_atom: u32,
    maximum_request_length: u16,
    transport: &mut Transport,
    requests: &mut Requests,
    now: Instant,
) -> Result<RequestId> {
    target.validate(requests)?;
    sender.check_timeout(now)?;
    if sender.state() != SendState::Announce || incr_atom == 0 || maximum_request_length < 7 {
        return Err(Error("invalid INCR announcement request".into()));
    }
    let lower_bound = 0u32.to_ne_bytes();
    let (bytes, fds) = Request::serialize(
        xproto::ChangePropertyRequest {
            mode: xproto::PropMode::REPLACE,
            window: target.window,
            property: target.property,
            type_: incr_atom,
            format: 32,
            data_len: 1,
            data: Cow::Borrowed(&lower_bound),
        },
        0,
    );
    debug_assert!(fds.is_empty());
    let id = requests.queue(
        transport,
        bytes,
        ReplyKind::Void,
        Importance::Optional,
        sender.deadline().unwrap(),
    )?;
    sender.announcement_queued(now)?;
    Ok(id)
}

/// Queue a whole data property, then permit the sender to await its deletion.
/// `maximum_request_length` is the setup limit in four-byte units; this path does
/// not use BIG-REQUESTS. A rejected queue attempt leaves the chunk available.
pub fn write_chunk(
    sender: &mut Sender,
    target: PropertyTarget,
    property_type: u32,
    maximum_request_length: u16,
    transport: &mut Transport,
    requests: &mut Requests,
    now: Instant,
) -> Result<RequestId> {
    target.validate(requests)?;
    sender.check_timeout(now)?;
    let bytes = sender
        .chunk()
        .ok_or_else(|| Error("INCR chunk is not ready".into()))?;
    let padded_length = (24 + bytes.len() + 3) & !3;
    if property_type == 0 || padded_length > usize::from(maximum_request_length) * 4 {
        return Err(Error(
            "INCR property exceeds server request limit or has no type".into(),
        ));
    }
    let request = xproto::ChangePropertyRequest {
        mode: xproto::PropMode::REPLACE,
        window: target.window,
        property: target.property,
        type_: property_type,
        format: sender.format(),
        data_len: (bytes.len() / usize::from(sender.format() / 8)) as u32,
        data: Cow::Borrowed(bytes),
    };
    let (bytes, fds) = Request::serialize(request, 0);
    debug_assert!(fds.is_empty());
    let id = requests.queue(
        transport,
        bytes,
        ReplyKind::Void,
        Importance::Optional,
        sender.deadline().unwrap(),
    )?;
    sender.chunk_queued(now)?;
    Ok(id)
}

/// Delete only a fully drained property (or the initial INCR announcement).
/// A final deletion queues local completion, still subject to asynchronous errors.
pub fn delete_property(
    receiver: &mut Receiver,
    target: PropertyTarget,
    transport: &mut Transport,
    requests: &mut Requests,
    now: Instant,
) -> Result<RequestId> {
    target.validate(requests)?;
    receiver.check_timeout(now)?;
    if receiver.state() != ReceiveState::DeleteProperty {
        return Err(Error("INCR property is not ready for deletion".into()));
    }
    let (bytes, fds) = Request::serialize(
        xproto::DeletePropertyRequest {
            window: target.window,
            property: target.property,
        },
        0,
    );
    debug_assert!(fds.is_empty());
    let id = requests.queue(
        transport,
        bytes,
        ReplyKind::Void,
        Importance::Optional,
        receiver.deadline().unwrap(),
    )?;
    receiver.deletion_queued(now)?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xwayland::{
        discovery::tests::{flush_requests, ready},
        incr::SendState,
    };
    use std::io::Read;
    fn target(requests: &Requests) -> PropertyTarget {
        PropertyTarget {
            generation: requests.generation(),
            window: 10,
            property: 11,
        }
    }
    #[test]
    fn notifications_preserve_request_fields_and_report_failure_with_none() {
        let (mut transport, mut requests, _, mut peer, now) = ready();
        let generation = requests.generation();
        let mut request = xproto::SelectionRequestEvent {
            response_type: xproto::SELECTION_REQUEST_EVENT,
            owner: 9,
            requestor: 10,
            selection: 11,
            target: 12,
            property: 13,
            time: u32::MAX,
            ..Default::default()
        };
        for (specified, result) in [(13, Some(13)), (13, None), (0, Some(14))] {
            request.property = specified;
            notify(
                generation,
                &request,
                result,
                &mut transport,
                &mut requests,
                now,
            )
            .unwrap();
            flush_requests(&mut transport);
            let mut wire = [0; 44];
            peer.read_exact(&mut wire).unwrap();
            assert_eq!(wire[0], 25); // SendEvent
            assert_eq!(wire[1], 0); // no propagation
            assert_eq!(u32::from_ne_bytes(wire[4..8].try_into().unwrap()), 10);
            assert_eq!(&wire[8..12], &[0; 4]); // empty event mask
            use x11rb_protocol::x11_utils::TryParse;
            let (event, _) = xproto::SelectionNotifyEvent::try_parse(&wire[12..]).unwrap();
            assert_eq!(event.response_type, xproto::SELECTION_NOTIFY_EVENT);
            assert_eq!(event.time, request.time);
            assert_eq!(event.requestor, request.requestor);
            assert_eq!(event.selection, request.selection);
            assert_eq!(event.target, request.target);
            assert_eq!(event.property, result.unwrap_or(0));
        }
        request.property = 13;
        let count = requests.outstanding();
        for (gen_id, property) in [
            (generation + 1, Some(13)),
            (generation, Some(0)),
            (generation, Some(14)),
        ] {
            assert!(
                notify(
                    gen_id,
                    &request,
                    property,
                    &mut transport,
                    &mut requests,
                    now
                )
                .is_err()
            );
        }
        assert_eq!(requests.outstanding(), count);
    }
    #[test]
    fn announcement_is_one_32_bit_zero_and_cannot_be_queued_twice() {
        let (mut transport, mut requests, _, mut peer, now) = ready();
        let target = target(&requests);
        let mut sender = Sender::new(8, 8, now).unwrap();
        let count = requests.outstanding();
        assert!(
            announce(
                &mut sender,
                target,
                99,
                6,
                &mut transport,
                &mut requests,
                now
            )
            .is_err()
        );
        assert!(
            announce(
                &mut sender,
                target,
                0,
                7,
                &mut transport,
                &mut requests,
                now
            )
            .is_err()
        );
        assert_eq!(sender.state(), SendState::Announce);
        assert_eq!(requests.outstanding(), count);
        announce(
            &mut sender,
            target,
            99,
            7,
            &mut transport,
            &mut requests,
            now,
        )
        .unwrap();
        assert_eq!(sender.state(), SendState::AwaitDelete);
        assert!(
            announce(
                &mut sender,
                target,
                99,
                7,
                &mut transport,
                &mut requests,
                now
            )
            .is_err()
        );
        assert_eq!(requests.outstanding(), count + 1);
        flush_requests(&mut transport);
        let mut wire = [0; 28];
        peer.read_exact(&mut wire).unwrap();
        assert_eq!(wire[0], 18);
        assert_eq!(u16::from_ne_bytes(wire[2..4].try_into().unwrap()), 7);
        assert_eq!(u32::from_ne_bytes(wire[12..16].try_into().unwrap()), 99);
        assert_eq!(wire[16], 32);
        assert_eq!(u32::from_ne_bytes(wire[20..24].try_into().unwrap()), 1);
        assert_eq!(&wire[24..], &[0; 4]);
    }
    #[test]
    fn typed_chunks_use_item_counts_and_preserve_data_on_rejection() {
        let (mut transport, mut requests, _, mut peer, now) = ready();
        let target = target(&requests);
        let mut sender = Sender::new_typed(16, 8, 8, now).unwrap();
        sender.announcement_queued(now).unwrap();
        sender.property_deleted(now).unwrap();
        sender.provide(&[1, 2, 3, 4, 5, 6], true, now).unwrap();
        let pending = requests.outstanding();
        assert!(
            write_chunk(
                &mut sender,
                target,
                12,
                7,
                &mut transport,
                &mut requests,
                now
            )
            .is_err()
        );
        assert!(
            write_chunk(
                &mut sender,
                PropertyTarget {
                    generation: target.generation + 1,
                    ..target
                },
                12,
                8,
                &mut transport,
                &mut requests,
                now
            )
            .is_err()
        );
        assert_eq!(requests.outstanding(), pending);
        assert_eq!(sender.chunk(), Some([1, 2, 3, 4, 5, 6].as_slice()));
        let id = write_chunk(
            &mut sender,
            target,
            12,
            8,
            &mut transport,
            &mut requests,
            now,
        )
        .unwrap();
        assert_eq!(id.generation, target.generation);
        assert_eq!(sender.state(), SendState::AwaitDelete);
        flush_requests(&mut transport);
        let mut wire = [0; 32];
        peer.read_exact(&mut wire).unwrap();
        assert_eq!(wire[0], 18); // ChangeProperty
        assert_eq!(wire[1], 0); // Replace
        assert_eq!(u16::from_ne_bytes(wire[2..4].try_into().unwrap()), 8);
        assert_eq!(u32::from_ne_bytes(wire[4..8].try_into().unwrap()), 10);
        assert_eq!(u32::from_ne_bytes(wire[8..12].try_into().unwrap()), 11);
        assert_eq!(u32::from_ne_bytes(wire[12..16].try_into().unwrap()), 12);
        assert_eq!(wire[16], 16);
        assert_eq!(u32::from_ne_bytes(wire[20..24].try_into().unwrap()), 3);
        assert_eq!(&wire[24..30], &[1, 2, 3, 4, 5, 6]);
        sender.property_deleted(now).unwrap();
        write_chunk(
            &mut sender,
            target,
            12,
            6,
            &mut transport,
            &mut requests,
            now,
        )
        .unwrap();
        flush_requests(&mut transport);
        let mut final_wire = [0; 24];
        peer.read_exact(&mut final_wire).unwrap();
        assert_eq!(final_wire[16], 16);
        assert_eq!(&final_wire[20..24], &[0; 4]);
        assert_eq!(sender.state(), SendState::AwaitDelete);
    }
    #[test]
    fn deletion_waits_for_drain_and_queue_capacity_preserves_sender_chunk() {
        let (mut transport, mut requests, _, mut peer, now) = ready();
        let target = target(&requests);
        let header = xproto::GetPropertyReply {
            type_: 99,
            format: 32,
            value_len: 1,
            value: 0u32.to_ne_bytes().to_vec(),
            ..Default::default()
        };
        let mut receiver = Receiver::new(&header, 99, 8, now).unwrap();
        delete_property(&mut receiver, target, &mut transport, &mut requests, now).unwrap();
        flush_requests(&mut transport);
        let mut wire = [0; 12];
        peer.read_exact(&mut wire).unwrap();
        assert_eq!(wire[0], 19); // DeleteProperty
        receiver
            .accept_property(
                &xproto::GetPropertyReply {
                    type_: 12,
                    format: 8,
                    value_len: 2,
                    value: vec![1, 2],
                    ..Default::default()
                },
                now,
            )
            .unwrap();
        receiver.consume(1, now).unwrap();
        let pending = requests.outstanding();
        assert!(
            delete_property(&mut receiver, target, &mut transport, &mut requests, now).is_err()
        );
        assert_eq!(requests.outstanding(), pending);
        assert_eq!(receiver.chunk(), &[2]);
        receiver.consume(1, now).unwrap();
        delete_property(&mut receiver, target, &mut transport, &mut requests, now).unwrap();
        while requests.available_slots() != 0 {
            let (bytes, _) = Request::serialize(xproto::GetInputFocusRequest, 0);
            requests
                .queue(
                    &mut transport,
                    bytes,
                    ReplyKind::Reply,
                    Importance::Optional,
                    now + std::time::Duration::from_secs(10),
                )
                .unwrap();
        }
        let mut sender = Sender::new(8, 8, now).unwrap();
        sender.announcement_queued(now).unwrap();
        sender.property_deleted(now).unwrap();
        sender.provide(&[1, 2], true, now).unwrap();
        assert!(
            write_chunk(
                &mut sender,
                target,
                12,
                8,
                &mut transport,
                &mut requests,
                now
            )
            .is_err()
        );
        assert_eq!(sender.state(), SendState::WriteChunk);
        assert_eq!(sender.chunk(), Some([1, 2].as_slice()));
    }
}

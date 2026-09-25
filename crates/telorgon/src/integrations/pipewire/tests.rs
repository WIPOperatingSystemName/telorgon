use super::connection::*;
use super::registry::RegistryState;
use super::*;
use std::{collections::BTreeMap, sync::mpsc};
fn connection(capacity: usize) -> (ConnectionHandle, mpsc::Receiver<Command>) {
    ConnectionHandle::test_channel(capacity)
}
#[test]
fn removed_reused_and_reconnected_handles_cannot_resolve() {
    let mut registry = RegistryState::new(3, false, 2);
    let first = registry
        .insert(9, ObjectKind::Node, 7, BTreeMap::new())
        .unwrap();
    registry.remove(9);
    let next = registry
        .insert(9, ObjectKind::Node, 7, BTreeMap::new())
        .unwrap();
    assert_eq!(
        registry.snapshot.resolve(first).unwrap_err(),
        MediaError::StaleHandle
    );
    assert!(registry.snapshot.resolve(next).is_ok());
    let other = RegistryState::new(4, false, 2);
    assert_eq!(
        other.snapshot.resolve(next).unwrap_err(),
        MediaError::StaleHandle
    );
    registry
        .insert(10, ObjectKind::Device, 7, BTreeMap::new())
        .unwrap();
    assert_eq!(
        registry.insert(11, ObjectKind::Port, 7, BTreeMap::new()),
        Err(MediaError::ResourceLimit("registry objects"))
    );
    assert_eq!(registry.snapshot.objects.len(), 2);
}
#[test]
fn cancellation_has_a_single_execution_boundary_and_disconnect_completes_pending_work() {
    let request = Request::new();
    let completion = Completion(request.state.clone());
    assert!(request.cancel());
    assert!(!completion.begin());
    completion.finish(Ok(()));
    assert_eq!(
        request.state(),
        RequestState::Complete(Err(MediaError::Cancelled))
    );
    let request = Request::new();
    let completion = Completion(request.state.clone());
    assert!(completion.begin());
    assert!(!request.cancel());
    drop(completion);
    assert_eq!(
        request.state(),
        RequestState::Complete(Err(MediaError::Disconnected))
    );
}
#[test]
fn bounded_commands_do_not_displace_already_accepted_work() {
    let (handle, receiver) = connection(1);
    let accepted = handle.barrier().unwrap();
    assert!(matches!(handle.barrier(), Err(MediaError::QueueFull)));
    assert_eq!(accepted.state(), RequestState::Queued);
    drop(receiver);
    assert_eq!(
        accepted.state(),
        RequestState::Complete(Err(MediaError::Disconnected))
    );
    assert!(matches!(handle.barrier(), Err(MediaError::Disconnected)));
}
#[test]
fn event_overflow_requires_resync_and_wakes_without_holding_snapshot_lock() {
    let (handle, _receiver) = connection(1);
    let other = handle.clone();
    let subscriber = handle
        .subscribe(1, move || {
            let _ = other.snapshot();
        })
        .unwrap();
    assert!(matches!(
        subscriber.try_recv(),
        Some(ConnectionEvent::ResyncRequired)
    ));
    handle
        .shared
        .emit(ConnectionEvent::StateChanged(ConnectionState::Connecting));
    handle
        .shared
        .emit(ConnectionEvent::StateChanged(ConnectionState::Ready));
    assert!(matches!(
        subscriber.try_recv(),
        Some(ConnectionEvent::ResyncRequired)
    ));
    assert!(subscriber.try_recv().is_none());
    assert_eq!(handle.snapshot().diagnostics.event_overflows, 1);
    handle
        .shared
        .state(ConnectionState::Failed(MediaError::Disconnected));
    assert!(matches!(
        subscriber.try_recv(),
        Some(ConnectionEvent::StateChanged(ConnectionState::Failed(_)))
    ));
}
#[test]
fn parameters_roundtrip_and_malformed_lengths_are_rejected() {
    use pipewire::spa::{
        self,
        pod::{Property, Value, ValueArray},
    };
    let bytes = parameters::encode(
        spa::sys::SPA_TYPE_OBJECT_Props,
        spa::sys::SPA_PARAM_Props,
        vec![
            Property::new(spa::sys::SPA_PROP_mute, Value::Bool(true)),
            Property::new(
                spa::sys::SPA_PROP_channelVolumes,
                Value::ValueArray(ValueArray::Float(vec![0.25, 0.75])),
            ),
        ],
    )
    .unwrap();
    let value = parameters::decode(&bytes).unwrap();
    assert_eq!(
        value.property(spa::sys::SPA_PROP_mute),
        Some(&ParameterValue::Bool(true))
    );
    assert_eq!(
        value.property(spa::sys::SPA_PROP_channelVolumes),
        Some(&ParameterValue::Floats(vec![0.25, 0.75]))
    );
    for n in 0..bytes.len() - 8 {
        assert!(parameters::decode(&bytes[..n]).is_none());
    }
    let mut invalid = bytes.clone();
    invalid[..4].copy_from_slice(&u32::MAX.to_ne_bytes());
    assert!(parameters::decode(&invalid).is_none());
}

use super::*;
use std::num::NonZeroU32;
use std::sync::mpsc::sync_channel;

fn window(slot: u32, generation: u32) -> CaptureSource {
    CaptureSource::Window(crate::shell::WindowId::new(
        NonZeroU32::new(slot).unwrap(),
        NonZeroU32::new(generation).unwrap(),
    ))
}
#[test]
fn public_decisions_preserve_identity_and_report_backpressure_without_false_wakes() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (snapshot, _) = Signal::new(ScreenCastPortalSnapshot::default());
    let (decisions, receive) = sync_channel(1);
    let wakes = Arc::new(AtomicUsize::new(0));
    let counter = wakes.clone();
    let ui = ScreenCastPortalContext {
        snapshot,
        decisions,
        wake: Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        }),
    };
    let source = window(1, 1);
    assert!(ui.approve(7, source, 11));
    assert!(!ui.deny(7));
    assert_eq!(
        receive.try_recv().unwrap(),
        CaptureDecision::Approve(7, source, 11)
    );
    assert!(ui.dismiss_failure());
    assert_eq!(receive.try_recv().unwrap(), CaptureDecision::DismissFailure);
    drop(receive);
    assert!(!ui.approve(7, source, 11));
    assert_eq!(wakes.load(Ordering::SeqCst), 2);
    assert!(!ScreenCastPortalContext::default().deny(7));
}

#[test]
fn group_decisions_are_bounded_and_report_backpressure() {
    let chosen: Vec<_> = (1..=8).map(|i| (window(i, 1), u64::from(i))).collect();
    let (snapshot, _) = Signal::new(ScreenCastPortalSnapshot::default());
    let (decisions, receive) = sync_channel(1);
    let ui = ScreenCastPortalContext {
        snapshot,
        decisions,
        wake: Arc::new(|| {}),
    };
    assert!(!ui.approve_many(1, &[]));
    assert!(!ui.approve_many(1, &vec![(window(1, 1), 1); 9]));
    assert!(receive.try_recv().is_err());
    assert!(ui.approve_many(1, &chosen));
    assert!(!ui.approve_many(2, &chosen));
    assert_eq!(
        receive.try_recv().unwrap(),
        CaptureDecision::ApproveMany(1, chosen)
    );
}

#[test]
fn shared_context_observes_updates_and_routes_portal_controls() {
    let (snapshot, writer) = Signal::new(ScreenCastPortalSnapshot::default());
    let (decisions, receive) = sync_channel(8);
    let picker = ScreenCastPortalContext {
        snapshot,
        decisions,
        wake: Arc::new(|| {}),
    };
    let sharing = picker.clone();
    writer.publish_if_changed(ScreenCastPortalSnapshot {
        pending: Some((9, "Recorder".into())),
        sharing: vec![(3, "Recorder".into())],
        ..Default::default()
    });
    assert_eq!(picker.snapshot().snapshot().pending.as_ref().unwrap().0, 9);
    assert_eq!(sharing.snapshot().snapshot().sharing[0].0, 3);
    let sources = vec![(window(1, 1), 7)];
    assert!(picker.approve_and_remember(9, &sources));
    assert!(sharing.stop(3));
    assert!(sharing.stop_and_forget(3));
    assert!(sharing.refresh_saved_permissions());
    assert!(sharing.forget_saved_permissions("org.example.Recorder"));
    assert!(!sharing.forget_saved_permissions(""));
    assert_eq!(
        receive.try_recv().unwrap(),
        CaptureDecision::ApproveRemembered(9, sources)
    );
    assert_eq!(receive.try_recv().unwrap(), CaptureDecision::Stop(3));
    assert_eq!(
        receive.try_recv().unwrap(),
        CaptureDecision::StopAndForget(3)
    );
    assert_eq!(
        receive.try_recv().unwrap(),
        CaptureDecision::RefreshPermissions
    );
    assert_eq!(
        receive.try_recv().unwrap(),
        CaptureDecision::ForgetApplication("org.example.Recorder".into())
    );
    assert!(receive.try_recv().is_err());
}

#[test]
fn audio_consent_requires_current_request_and_negotiated_delivery() {
    let (snapshot, writer) = Signal::new(ScreenCastPortalSnapshot::default());
    let (decisions, receive) = sync_channel(8);
    let context = ScreenCastPortalContext { snapshot, decisions, wake: Arc::new(|| {}) };
    let sources = [(window(1, 1), 1)];
    assert!(!context.approve_with_audio(8, &sources));
    writer.publish_if_changed(ScreenCastPortalSnapshot {
        pending: Some((8, "recorder".into())), audio_available: true,
        ..Default::default()
    });
    assert!(!context.approve_with_audio(7, &sources));
    assert!(context.approve_with_audio(8, &sources));
    assert_eq!(receive.try_recv().unwrap(), CaptureDecision::ApproveWithAudio(8, sources.to_vec()));
    writer.publish_if_changed(ScreenCastPortalSnapshot {
        pending: Some((8, "recorder".into())), audio_available: false,
        ..Default::default()
    });
    assert!(!context.approve_with_audio(8, &sources));
}

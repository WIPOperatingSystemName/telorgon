use super::*;
use std::sync::atomic::AtomicBool;

static TEST_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn collects_correlated_spans_and_counters() {
    let _serial = TEST_LOCK.lock().unwrap();
    let (session, mut collector) = Session::start(SessionConfig::default()).unwrap();
    {
        let frame = start_frame("frame.total");
        assert!(frame.id().is_some());
        let _span = span!("layout.measure");
        counter!("layout.nodes", 17_u32);
    }
    let mut events = Vec::new();
    collector.drain_into(&mut events);
    assert!(
        events
            .iter()
            .any(|event| event.kind == EventKind::FrameBegin)
    );
    assert!(events.iter().any(|event| event.kind == EventKind::FrameEnd));
    let span = events
        .iter()
        .find(|event| event.kind == EventKind::Span)
        .unwrap();
    assert_eq!(span.label, "layout.measure");
    assert!(span.frame.is_some());
    assert_eq!(
        events
            .iter()
            .find_map(|event| event.counter_value())
            .unwrap(),
        17.0
    );
    drop(session);
}

#[test]
fn inactive_counter_macro_does_not_evaluate_its_value() {
    let _serial = TEST_LOCK.lock().unwrap();
    let evaluated = AtomicBool::new(false);
    counter!("inactive", {
        evaluated.store(true, Ordering::Relaxed);
        1
    });
    assert!(!evaluated.load(Ordering::Relaxed));
}

#[test]
fn current_thread_suppression_is_nested_and_does_not_record() {
    let _serial = TEST_LOCK.lock().unwrap();
    let (session, mut collector) = Session::start(SessionConfig::default()).unwrap();
    assert!(is_active());
    {
        let _outer = suppress_current_thread();
        assert!(!is_active());
        record_instant("suppressed.outer");
        {
            let _inner = suppress_current_thread();
            record_instant("suppressed.inner");
        }
        assert!(!is_active());
    }
    assert!(is_active());
    record_instant("recorded.after_suppression");

    let mut events = Vec::new();
    collector.drain_into(&mut events);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].label, "recorded.after_suppression");
    drop(session);
}

#[test]
fn native_input_collection_is_independent_and_defaults_off_for_each_session() {
    let _serial = TEST_LOCK.lock().unwrap();
    let (session, _) = Session::start(SessionConfig::default()).unwrap();
    assert!(!pointer_move_events_enabled());
    assert!(!input_recording_enabled(InputRecordingSource::Keyboard));
    set_pointer_move_events_enabled(true);
    assert!(pointer_move_events_enabled());
    assert!(!input_recording_enabled(InputRecordingSource::Keyboard));
    set_input_recording_enabled(InputRecordingSource::Keyboard, true);
    assert!(input_recording_enabled(InputRecordingSource::Keyboard));
    set_pointer_move_events_enabled(false);
    assert!(!pointer_move_events_enabled());
    assert!(input_recording_enabled(InputRecordingSource::Keyboard));
    drop(session);
    assert!(!pointer_move_events_enabled());
    assert!(!input_recording_enabled(InputRecordingSource::Keyboard));
}

#[test]
fn saturation_reports_an_exact_gap() {
    let _serial = TEST_LOCK.lock().unwrap();
    let (session, mut collector) = Session::start(SessionConfig {
        producer_capacity: 1,
    })
    .unwrap();
    for _ in 0..7 {
        record_instant("overflow");
    }
    let mut events = Vec::new();
    collector.drain_into(&mut events);
    let dropped = events
        .iter()
        .filter_map(|event| event.dropped_count())
        .sum::<u64>();
    assert_eq!(dropped, 6);
    drop(session);
}

#[test]
fn default_record_storage_stays_below_one_mebibyte() {
    assert!(std::mem::size_of::<Event>() * DEFAULT_PRODUCER_CAPACITY <= 1024 * 1024);
}

#[test]
fn transferred_frame_identity_survives_a_named_worker_lane() {
    let _serial = TEST_LOCK.lock().unwrap();
    let (session, mut collector) = Session::start(SessionConfig::default()).unwrap();
    let frame = start_frame("frame.total");
    let frame_id = frame.id();
    std::thread::Builder::new()
        .name("fixture-worker".to_owned())
        .spawn(move || {
            let _frame = enter_frame(frame_id);
            let _span = span!("worker.process");
        })
        .unwrap()
        .join()
        .unwrap();
    drop(frame);
    let mut events = Vec::new();
    collector.drain_into(&mut events);
    assert!(
        events
            .iter()
            .any(|event| { event.label == "worker.process" && event.frame == frame_id })
    );
    assert!(
        collector
            .lanes()
            .iter()
            .any(|lane| lane.name == "fixture-worker")
    );
    drop(session);
}

#[test]
fn view_scope_correlates_frames_and_restores_the_previous_view() {
    let _serial = TEST_LOCK.lock().unwrap();
    let (session, mut collector) = Session::start(SessionConfig::default()).unwrap();
    {
        let _view = enter_view(Some(ProfileViewId::PRIMARY));
        assert!(register_view(ProfileViewId::PRIMARY, "Application window"));
        assert!(!register_view(ProfileViewId::PRIMARY, "Different role"));
        assert_eq!(current_view_id(), Some(ProfileViewId::PRIMARY));
        let _frame = start_frame("frame.total");
        record_instant("host.redraw_callback");
    }
    assert_eq!(current_view_id(), None);
    assert_eq!(allocate_view_id().map(ProfileViewId::get), Some(2));
    assert_eq!(
        collector.views(),
        vec![ViewInfo {
            id: ProfileViewId::PRIMARY,
            role: "Application window",
        }]
    );
    let mut events = Vec::new();
    collector.drain_into(&mut events);
    assert!(
        events
            .iter()
            .filter(
                |event| event.label == "frame.total" || event.label == "host.redraw_callback"
            )
            .all(|event| event.view == Some(ProfileViewId::PRIMARY))
    );
    drop(session);
}

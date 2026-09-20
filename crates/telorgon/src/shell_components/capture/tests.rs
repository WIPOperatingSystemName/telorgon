use super::*;
use crate::runtime::{CompositionDriver, ViewRuntime};
use std::num::NonZeroU32;
use std::sync::mpsc::{Receiver, sync_channel};

type Runtime = ViewRuntime<CompositionDriver>;
fn window(slot: u32, generation: u32) -> CaptureSource {
    CaptureSource::Window(crate::shell::WindowId::new(
        NonZeroU32::new(slot).unwrap(),
        NonZeroU32::new(generation).unwrap(),
    ))
}
fn snapshot() -> CaptureUiSnapshot {
    CaptureUiSnapshot {
        pending: Some((1, "Discord".into())),
        sources: vec![
            (
                CaptureSource::Output(crate::shell::OutputId::MIN),
                1,
                "Screen 1".into(),
            ),
            (window(1, 1), 2, "Editor".into()),
        ],
        ..Default::default()
    }
}
fn fixture(
    value: CaptureUiSnapshot,
) -> (
    Runtime,
    SignalWriter<CaptureUiSnapshot>,
    Receiver<CaptureDecision>,
) {
    let (snapshot, writer) = Signal::new(value);
    let (decisions, receive) = sync_channel(16);
    let runtime = ViewRuntime::from_composed(CapturePicker::new(CaptureUi {
        snapshot,
        decisions,
        wake: Arc::new(|| {}),
    }))
    .unwrap();
    (runtime, writer, receive)
}
fn button(runtime: &Runtime, label: &str) -> crate::ui::UiNodeId {
    runtime
        .ui()
        .semantics
        .iter()
        .find_map(|(node, semantic)| {
            let crate::SemanticName::Text(text) = semantic.name else {
                return None;
            };
            (semantic.role == crate::SemanticRole::Button
                && runtime.ui().string(text) == Some(label))
            .then_some(node)
        })
        .unwrap_or_else(|| panic!("missing button {label}"))
}
fn press(runtime: &mut Runtime, label: &str) {
    assert!(runtime.dispatch_action(button(runtime, label)));
}

#[test]
fn selecting_a_preview_requires_separate_explicit_share() {
    let (mut runtime, _, receive) = fixture(snapshot());
    let share = button(&runtime, "Share");
    runtime.dispatch_action(share);
    assert!(receive.try_recv().is_err());
    press(&mut runtime, "Screen 1");
    assert!(receive.try_recv().is_err());
    press(&mut runtime, "Share");
    assert_eq!(
        receive.try_recv().unwrap(),
        CaptureDecision::Approve(1, CaptureSource::Output(crate::shell::OutputId::MIN), 1)
    );
    runtime.dispatch_action(button(&runtime, "Share"));
    assert!(receive.try_recv().is_err());
}

#[test]
fn window_tab_selects_exact_window_not_monitor() {
    let (mut runtime, _, receive) = fixture(snapshot());
    press(&mut runtime, "Windows");
    press(&mut runtime, "Editor");
    assert!(receive.try_recv().is_err());
    press(&mut runtime, "Share");
    assert_eq!(
        receive.try_recv().unwrap(),
        CaptureDecision::Approve(1, window(1, 1), 2)
    );
}

#[test]
fn source_or_request_replacement_retires_both_selection_and_share_controls() {
    for replacement in 0..3 {
        let mut value = snapshot();
        value.sources.remove(0);
        let (mut runtime, writer, receive) = fixture(value.clone());
        let old_card = button(&runtime, "Editor");
        press(&mut runtime, "Editor");
        let old_share = button(&runtime, "Share");
        match replacement {
            0 => value.pending = Some((2, "Another app".into())),
            1 => value.sources[0].0 = window(1, 2),
            _ => value.sources[0].1 = 3,
        }
        writer.publish_if_changed(value.clone());
        runtime.process_external_updates();
        assert!(!runtime.dispatch_action(old_card));
        assert!(!runtime.dispatch_action(old_share));
        runtime.dispatch_action(button(&runtime, "Share"));
        assert!(receive.try_recv().is_err());
        press(&mut runtime, "Editor");
        press(&mut runtime, "Share");
        assert_eq!(
            receive.try_recv().unwrap(),
            CaptureDecision::Approve(
                value.pending.unwrap().0,
                value.sources[0].0,
                value.sources[0].1
            )
        );
    }
}

#[test]
fn disappearing_source_clears_selection_and_disables_share() {
    let mut value = snapshot();
    let (mut runtime, writer, receive) = fixture(value.clone());
    press(&mut runtime, "Screen 1");
    let old_share = button(&runtime, "Share");
    value.sources.clear();
    writer.publish_if_changed(value);
    runtime.process_external_updates();
    assert!(!runtime.dispatch_action(old_share));
    runtime.dispatch_action(button(&runtime, "Share"));
    assert!(receive.try_recv().is_err());
    press(&mut runtime, "Cancel");
    assert_eq!(receive.try_recv().unwrap(), CaptureDecision::Deny(1));
}

#[test]
fn all_sources_are_reachable_without_automatic_consent() {
    let mut value = snapshot();
    value.sources = (1..=19)
        .map(|i| (window(i, 1), u64::from(i), format!("Window {i}")))
        .collect();
    let (mut runtime, _, receive) = fixture(value);
    for _ in 0..3 {
        press(&mut runtime, "›");
    }
    press(&mut runtime, "Window 19");
    assert!(receive.try_recv().is_err());
    press(&mut runtime, "Share");
    assert_eq!(
        receive.try_recv().unwrap(),
        CaptureDecision::Approve(1, window(19, 1), 19)
    );
}

#[test]
fn multiple_output_identities_are_preserved_by_selection() {
    use selection::Selection;
    let mut value = snapshot();
    let other = CaptureSource::Output(crate::shell::OutputId::from_raw(2).unwrap());
    value.sources.push((other, 9, "HDMI".into()));
    let state = Selection {
        request: 1,
        selected: Some((other, 9)),
        ..Default::default()
    };
    assert_eq!(state.current(&value, 6).selected, Some((other, 9)));
    value.sources.pop();
    assert_eq!(state.current(&value, 6).selected, None);
}

#[test]
fn layouts_keep_preview_slots_inside_cards_and_panel() {
    use layout::PickerLayout;
    for (width, height) in [
        (3840.0, 2160.0),
        (1920.0, 1200.0),
        (800.0, 600.0),
        (480.0, 420.0),
        (360.0, 360.0),
    ] {
        let layout = PickerLayout::new(crate::SizeF { width, height }, 6);
        assert!(layout.width <= width && layout.height <= height);
        for index in 0..layout.capacity() {
            let rect = layout.preview(index);
            assert!(rect.x >= 0.0 && rect.y >= 0.0 && rect.width > 0.0 && rect.height > 0.0);
            assert!(rect.x + rect.width <= layout.width && rect.y + rect.height <= layout.height);
        }
    }
}

#[test]
fn public_decisions_preserve_identity_and_report_backpressure_without_false_wakes() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (snapshot, _) = Signal::new(CaptureUiSnapshot::default());
    let (decisions, receive) = sync_channel(1);
    let wakes = Arc::new(AtomicUsize::new(0));
    let counter = wakes.clone();
    let ui = CaptureUi {
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
    assert!(!CaptureUi::default().deny(7));
}

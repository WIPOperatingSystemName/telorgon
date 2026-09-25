use super::*;
use crate::platform::contracts::{
    ClipboardService, DataFormatReadRequest, DataReadMode, DataTransferService,
};
use futures_lite::future::block_on;

#[test]
fn clipboard_roundtrips_formats_primary_selection_and_streamed_data() {
    let display = Display::new().unwrap();
    let mut native = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
    let clipboard = native.install_clipboard(Arc::new(|| {}));
    let text = "Café 🦊".repeat(20000);
    let request = clipboard.publish(
        ClipboardKind::System,
        ClipboardContent::text(text.clone()).unwrap(),
        None,
    );
    native.dispatch_clipboard(false);
    block_on(request).unwrap();
    let request = clipboard.publish(
        ClipboardKind::Selection,
        ClipboardContent::text("primary".into()).unwrap(),
        None,
    );
    native.dispatch_clipboard(false);
    block_on(request).unwrap();
    let snapshot = clipboard.snapshot(ClipboardKind::System).unwrap();
    assert_eq!(snapshot.formats.len(), 2);
    let request = clipboard.read(
        snapshot.clone(),
        snapshot.formats[0].clone(),
        service::MAX_BYTES,
    );
    native.dispatch_clipboard(false);
    assert_eq!(block_on(request).unwrap(), text.as_bytes());
    let mut stream = clipboard
        .read_stream(
            snapshot.clone(),
            snapshot.formats[0].clone(),
            service::MAX_BYTES,
            31,
        )
        .unwrap();
    native.dispatch_clipboard(false);
    let bytes = block_on(async {
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.unwrap();
            assert!(chunk.len() <= 31);
            bytes.extend(chunk);
        }
        bytes
    });
    assert_eq!(bytes, text.as_bytes());
    let primary = clipboard.snapshot(ClipboardKind::Selection).unwrap();
    let request = clipboard.read(primary.clone(), primary.formats[0].clone(), 128);
    native.dispatch_clipboard(false);
    assert_eq!(block_on(request).unwrap(), b"primary");
}

#[test]
fn clipboard_replacement_lock_cancel_and_shutdown_are_explicit() {
    let display = Display::new().unwrap();
    let mut native = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
    let clipboard = native.install_clipboard(Arc::new(|| {}));
    let request = clipboard.publish(
        ClipboardKind::System,
        ClipboardContent::text("one".into()).unwrap(),
        None,
    );
    native.dispatch_clipboard(false);
    block_on(request).unwrap();
    let stale = clipboard.snapshot(ClipboardKind::System).unwrap();
    let request = clipboard.publish(
        ClipboardKind::System,
        ClipboardContent::text("two".into()).unwrap(),
        None,
    );
    native.dispatch_clipboard(false);
    block_on(request).unwrap();
    let request = clipboard.read(stale.clone(), stale.formats[0].clone(), 128);
    native.dispatch_clipboard(false);
    assert_eq!(block_on(request), Err(ClipboardError::Stale));
    let request = clipboard.clear(ClipboardKind::System, None);
    request.cancel();
    native.dispatch_clipboard(false);
    assert_eq!(block_on(request), Err(ClipboardError::Cancelled));
    assert!(
        !clipboard
            .snapshot(ClipboardKind::System)
            .unwrap()
            .formats
            .is_empty()
    );
    native.dispatch_clipboard(true);
    assert_eq!(
        clipboard.snapshot(ClipboardKind::System),
        Err(ClipboardError::Denied)
    );
    native.dispatch_clipboard(false);
    drop(native);
    assert_eq!(
        clipboard.snapshot(ClipboardKind::System),
        Err(ClipboardError::Unavailable)
    );
}

#[test]
fn clipboard_contracts_publish_and_complete_bounded_reads() {
    let display = Display::new().unwrap();
    let mut native = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
    let clipboard = native.install_clipboard(Arc::new(|| {}));
    let contracts = service::ClipboardContracts::new(clipboard);
    let offer = contracts
        .register_content(ClipboardContent::text("contract".into()).unwrap())
        .unwrap();
    let token = contracts
        .publish(
            crate::platform::contracts::ClipboardPublishRequest::new(
                ClipboardKind::System,
                offer,
                None,
            )
            .unwrap(),
        )
        .unwrap();
    native.dispatch_clipboard(false);
    let completion = block_on(contracts.complete_publish(token));
    assert!(completion.outcome().is_applied());
    let snapshot = contracts.current_snapshot(ClipboardKind::System);
    let offer = snapshot.current().unwrap().current_offer().unwrap();
    let request = DataFormatReadRequest::for_offer(
        offer,
        offer.formats()[0].clone(),
        std::num::NonZeroU64::new(64).unwrap(),
        DataReadMode::Buffered,
    )
    .unwrap();
    let token = contracts.request_read(request).unwrap();
    native.dispatch_clipboard(false);
    let result = block_on(contracts.complete_read(token));
    assert!(result.completion.outcome().is_applied());
    assert_eq!(result.bytes, b"contract");
}

#[test]
fn clipboard_pipe_read_rejects_oversize_and_cancelled_stream() {
    let (reader, writer) = io::pipe().unwrap();
    let (request, reply) = service::channel();
    io::read_pipe(reader, 2, service::ReadResponse::Buffered(reply));
    use std::io::Write;
    std::fs::File::from(writer).write_all(b"oversize").unwrap();
    assert_eq!(block_on(request), Err(ClipboardError::TooLarge));
    let (reader, _writer) = io::pipe().unwrap();
    let (mut stream, sink) = service::stream::stream(8);
    io::read_pipe(reader, 128, service::ReadResponse::Streamed(sink));
    stream.cancel();
    assert_eq!(
        block_on(stream.next()),
        Some(Err(ClipboardError::Cancelled))
    );
    assert_eq!(block_on(stream.next()), None);
}

#[test]
fn clipboard_provider_failure_is_not_successful_partial_content() {
    struct Broken;
    impl service::ClipboardProvider for Broken {
        fn write(
            &self,
            _: &DataFormat,
            output: &mut dyn std::io::Write,
            _: &AtomicBool,
        ) -> service::Result<()> {
            output.write_all(b"partial").unwrap();
            Err(ClipboardError::TransferFailed)
        }
    }
    let format = DataFormat::mime("text/plain").unwrap();
    let content = ClipboardContent::provider(vec![format.clone()], Arc::new(Broken)).unwrap();
    let (request, reply) = service::channel();
    io::read_provider(content, format, 128, service::ReadResponse::Buffered(reply));
    assert_eq!(block_on(request), Err(ClipboardError::TransferFailed));
}

#[test]
fn clipboard_wire_system_and_primary_share_input_serial_and_clear_on_destroy() {
    use super::super::wire_tests::{bind, registry, send, words};
    use crate::integrations::wayland::compositor::{
        KeyboardFocus, SeatCapabilities, SeatState, SerialKind,
    };
    use std::{os::unix::net::UnixStream, time::Duration};
    fn mime() -> Vec<u8> {
        let value = b"text/plain\0";
        let mut bytes = (value.len() as u32).to_ne_bytes().to_vec();
        bytes.extend(value);
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }
        bytes
    }
    let display = Display::new().unwrap();
    let (mut peer, socket) = UnixStream::pair().unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let client = display.create_client(socket).unwrap();
    let mut native = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
    native
        .add_seat(
            &display,
            1,
            SeatState::new(
                "test",
                SeatCapabilities {
                    keyboard: true,
                    ..Default::default()
                },
            ),
        )
        .unwrap();
    let clipboard = native.install_clipboard(Arc::new(|| {}));
    let globals = registry(&display, &mut peer);
    bind(&mut peer, &globals, "wl_compositor", 4);
    bind(&mut peer, &globals, "wl_seat", 5);
    bind(&mut peer, &globals, "wl_data_device_manager", 6);
    bind(
        &mut peer,
        &globals,
        "zwp_primary_selection_device_manager_v1",
        7,
    );
    send(&mut peer, 4, 0, &words(&[8])); // surface
    send(&mut peer, 6, 1, &words(&[9, 5])); // clipboard device
    send(&mut peer, 7, 1, &words(&[10, 5])); // primary device
    send(&mut peer, 6, 0, &words(&[11])); // clipboard source
    send(&mut peer, 7, 0, &words(&[12])); // primary source
    send(&mut peer, 11, 0, &mime());
    send(&mut peer, 12, 0, &mime());
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    assert!(client.is_alive());
    let client_id = native.state.clients[&client.identity().unwrap()];
    let surface = native.core().world.client_surfaces(client_id)[0];
    native.state.core.seats.get_mut(&1).unwrap().keyboard_focus = Some(KeyboardFocus {
        client: client_id,
        surface,
        enter_serial: 41,
    });
    native
        .state
        .core
        .serials
        .issue(42, client_id, SerialKind::KeyboardKey, Some(surface))
        .unwrap();
    send(&mut peer, 9, 1, &words(&[11, 42]));
    send(&mut peer, 10, 0, &words(&[12, 42]));
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    native.dispatch_clipboard(false);
    assert!(client.is_alive());
    assert_eq!(
        clipboard
            .snapshot(ClipboardKind::System)
            .unwrap()
            .formats
            .len(),
        1
    );
    assert_eq!(
        clipboard
            .snapshot(ClipboardKind::Selection)
            .unwrap()
            .formats
            .len(),
        1
    );
    // Reusing a current source or submitting an obsolete serial must not kill the app.
    send(&mut peer, 9, 1, &words(&[11, 42]));
    send(&mut peer, 10, 0, &words(&[12, 99]));
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    assert!(client.is_alive());
    send(&mut peer, 11, 1, &[]);
    send(&mut peer, 12, 1, &[]);
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    native.dispatch_clipboard(false);
    assert!(client.is_alive());
    assert!(
        clipboard
            .snapshot(ClipboardKind::System)
            .unwrap()
            .formats
            .is_empty()
    );
    assert!(
        clipboard
            .snapshot(ClipboardKind::Selection)
            .unwrap()
            .formats
            .is_empty()
    );
}

#[test]
fn clipboard_notifications_recover_after_unlock_and_subscriber_drop() {
    let display = Display::new().unwrap();
    let mut native = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
    let clipboard = native.install_clipboard(Arc::new(|| {}));
    for _ in 0..100 {
        drop(clipboard.subscribe().unwrap());
    }
    let changes = clipboard.subscribe().unwrap();
    assert_eq!(changes.try_recv().unwrap().kind, ClipboardKind::System);
    assert_eq!(changes.try_recv().unwrap().kind, ClipboardKind::Selection);
    let publication = clipboard.publish(
        ClipboardKind::System,
        ClipboardContent::text("notification".into()).unwrap(),
        None,
    );
    native.dispatch_clipboard(false);
    block_on(publication).unwrap();
    let published = changes.try_recv().unwrap();
    assert_eq!(published.formats.len(), 2);
    native.dispatch_clipboard(true);
    assert!(matches!(clipboard.subscribe(), Err(ClipboardError::Denied)));
    native.dispatch_clipboard(false);
    assert_eq!(changes.try_recv().unwrap(), published);
}

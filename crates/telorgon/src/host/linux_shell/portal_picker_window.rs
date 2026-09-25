//! A real Wayland picker client with request-scoped lifetime and bounded state delivery.
use crate::ScreenCastPortalContext;
use crate::authoring::compose::SignalSubscription;
use crate::host::application::{PortalPickerWindow, portal_wire as wire};
use std::{
    io::BufReader,
    path::Path,
    process::{Child, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::sync_channel,
    },
    thread::JoinHandle,
    time::Duration,
};

pub(super) struct PickerProcess {
    stop: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    worker: Option<JoinHandle<()>>,
    _subscription: SignalSubscription,
}
impl PickerProcess {
    pub fn process_id(&self) -> Option<u32> {
        self.child.lock().unwrap().as_ref().map(Child::id)
    }
    pub fn start(
        mut config: PortalPickerWindow,
        context: ScreenCastPortalContext,
        socket: &Path,
    ) -> std::io::Result<Self> {
        let application = config.application.as_ref().map(|reference| {
            let handle = reference.resolve().map_err(std::io::Error::other)?;
            let (program, arguments) = handle.executable_parts().map_err(std::io::Error::other)?;
            config.program = program.to_owned();
            config.args.splice(0..0, arguments.iter().cloned());
            Ok::<_, std::io::Error>(handle)
        }).transpose()?;
        let handshake = application.is_some();
        let (wake, updates) = sync_channel(1);
        let subscription = context
            .snapshot()
            .dependency(0)
            .subscribe(Arc::new(move || {
                let _ = wake.try_send(());
            }));
        let stop = Arc::new(AtomicBool::new(false));
        let child = Arc::new(Mutex::new(None::<Child>));
        let worker_stop = stop.clone();
        let worker_child = child.clone();
        let socket = socket.to_owned();
        let worker = std::thread::Builder::new()
            .name("portal-picker-window".into())
            .spawn(move || {
                let mut completed = None;
                while !worker_stop.load(Ordering::Acquire) {
                    let snapshot = context.snapshot().snapshot();
                    let request = snapshot.pending.as_ref().map(|r| r.0);
                    if request.is_none() || request == completed {
                        let _ = updates.recv_timeout(Duration::from_millis(100));
                        continue;
                    }
                    let id = request.unwrap();
                    completed = Some(id);
                    let mut command = std::process::Command::new(&config.program);
                    command
                        .args(&config.args)
                        .env("WAYLAND_DISPLAY", &socket)
                        .env_remove("WAYLAND_SOCKET")
                        .env_remove("DISPLAY")
                        .env("TELORGON_PORTAL_PICKER", "1")
                        .stdin(Stdio::piped())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::inherit());
                    if handshake { command.env("TELORGON_PORTAL_PICKER_PROTOCOL", "1"); }
                    else { command.env_remove("TELORGON_PORTAL_PICKER_PROTOCOL"); }
                    let result = match &application {
                        Some(app) => app.spawn_helper(&mut command).map_err(std::io::Error::other),
                        None => command.spawn(),
                    };
                    let mut process = match result {
                        Ok(child) => child,
                        Err(error) => {
                            eprintln!("telorgon-portal: picker launch failed: {error}");
                            context.deny(id);
                            continue;
                        }
                    };
                    let mut stdin = process.stdin.take().unwrap();
                    let stdout = process.stdout.take().unwrap();
                    *worker_child.lock().unwrap() = Some(process);
                    let (send, frames) = sync_channel::<crate::ScreenCastPortalSnapshot>(1);
                    let writer = std::thread::spawn(move || {
                        if handshake && wire::write(&mut stdin, &wire::hello()).is_err() { return; }
                        let mut delivery = super::portal_picker_delivery::PickerDelivery::default();
                        while let Ok(mut frame) = frames.recv() {
                            // Coalesce queued snapshots before serialization; never build a FIFO
                            // of historical thumbnail frames behind a slow picker.
                            while let Ok(newer) = frames.try_recv() { frame = newer; }
                            if delivery.write(&mut stdin, &frame).is_err() { break; }
                        }
                    });
                    let reply_context = context.clone();
                    let (ready_send, ready_receive) = sync_channel(1);
                    let reader = std::thread::spawn(move || {
                        let mut stdout = BufReader::new(stdout);
                        if handshake && wire::read(&mut stdout).and_then(wire::validate_hello).is_err() {
                            reply_context.deny(id);
                            return;
                        }
                        let _ = ready_send.send(());
                        let reply = wire::read(&mut stdout);
                        if !reply.is_ok_and(|value| submit_reply(&reply_context, id, value)) {
                            reply_context.deny(id);
                        }
                    });
                    let mut revision = None;
                    let started = std::time::Instant::now();
                    let mut ready = !handshake;
                    loop {
                        let snapshot = context.snapshot().snapshot();
                        ready |= ready_receive.try_recv().is_ok();
                        if (!ready && started.elapsed() > Duration::from_secs(5))
                            || application.as_ref().is_some_and(|app| app.ready().is_err()) {
                            context.deny(id);
                            break;
                        }
                        if worker_stop.load(Ordering::Acquire)
                            || snapshot.pending.as_ref().map(|r| r.0) != Some(id)
                        {
                            break;
                        }
                        if worker_child
                            .lock()
                            .unwrap()
                            .as_mut()
                            .unwrap()
                            .try_wait()
                            .ok()
                            .flatten()
                            .is_some()
                        {
                            break;
                        }
                        if revision != Some(snapshot.revision)
                            && send.try_send((*snapshot).clone()).is_ok()
                        {
                            revision = Some(snapshot.revision);
                        }
                        let _ = updates.recv_timeout(Duration::from_millis(8));
                    }
                    if let Some(mut process) = worker_child.lock().unwrap().take() {
                        let _ = process.kill();
                        let _ = process.wait();
                    }
                    drop(send);
                    let _ = writer.join();
                    let _ = reader.join();
                }
            })?;
        Ok(Self {
            stop,
            child,
            worker: Some(worker),
            _subscription: subscription,
        })
    }
}
impl Drop for PickerProcess {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(child) = self.child.lock().unwrap().as_mut() {
            let _ = child.kill();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn submit_reply(context: &ScreenCastPortalContext, request: u64, value: serde_json::Value) -> bool {
    if value["request"].as_u64() != Some(request) {
        return false;
    }
    if value["action"] == "cancel" {
        return context.deny(request);
    }
    let Some(values) = value["sources"]
        .as_array()
        .filter(|v| !v.is_empty() && v.len() <= 8)
    else {
        return false;
    };
    let sources: Option<Vec<_>> = values
        .iter()
        .map(|v| Some((wire::parse_source(&v[0]).ok()?, v[1].as_u64()?)))
        .collect();
    let Some(sources) = sources else {
        return false;
    };
    if value.get("audio").is_some_and(|audio| !audio.is_boolean()) { return false; }
    if value["audio"] == true {
        return value["action"] == "share" && context.approve_with_audio(request, &sources);
    }
    match value["action"].as_str() {
        Some("remember") => context.approve_and_remember(request, &sources),
        Some("share") if context.snapshot().snapshot().multiple => {
            context.approve_many(request, &sources)
        }
        Some("share") if sources.len() == 1 => context.approve(request, sources[0].0, sources[0].1),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Signal;
    use crate::authoring::compose::portal::{CaptureDecision, ScreenCastPortalSnapshot};
    fn context() -> (
        ScreenCastPortalContext,
        crate::SignalWriter<ScreenCastPortalSnapshot>,
        std::sync::mpsc::Receiver<CaptureDecision>,
    ) {
        let (snapshot, writer) = Signal::new(ScreenCastPortalSnapshot {
            pending: Some((7, "Recorder".into())),
            source_types: 1,
            sources: vec![(
                crate::shell::capture::CaptureSource::Output(crate::shell::OutputId::MIN),
                9,
                "Monitor".into(),
            )],
            ..Default::default()
        });
        let (decisions, receive) = sync_channel(16);
        (
            ScreenCastPortalContext {
                snapshot,
                decisions,
                wake: Arc::new(|| {}),
            },
            writer,
            receive,
        )
    }
    #[test]
    fn picker_process_returns_exact_consent_and_exit_cancels() {
        for (script, expected) in [
            (
                "read state; printf '%s\\n' '{\"request\":7,\"action\":\"share\",\"sources\":[[[1,1,0],9]]}'",
                CaptureDecision::Approve(
                    7,
                    crate::shell::capture::CaptureSource::Output(crate::shell::OutputId::MIN),
                    9,
                ),
            ),
            ("read state; exit 0", CaptureDecision::Deny(7)),
        ] {
            let (context, _, receive) = context();
            let process = PickerProcess::start(
                PortalPickerWindow::new("/bin/sh").arg("-c").arg(script),
                context,
                Path::new("/tmp/portal-test-wayland"),
            )
            .unwrap();
            assert_eq!(
                receive.recv_timeout(Duration::from_secs(3)).unwrap(),
                expected
            );
            drop(process);
        }
    }
    #[test]
    fn cancelling_request_terminates_its_picker() {
        let (context, writer, _) = context();
        let process = PickerProcess::start(
            PortalPickerWindow::new("/bin/sh")
                .arg("-c")
                .arg("exec sleep 30"),
            context,
            Path::new("/tmp/portal-test-wayland"),
        )
        .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while process.child.lock().unwrap().is_none() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        writer.publish_if_changed(ScreenCastPortalSnapshot::default());
        while process.child.lock().unwrap().is_some() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        drop(process);
    }
}

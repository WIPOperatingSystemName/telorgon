use super::{
    connection::{Command, Completion, Shared, native},
    *,
};
use pipewire as pw;
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{Arc, atomic::Ordering, mpsc},
    time::{Duration, Instant},
};

pub(super) fn run(
    config: ConnectionConfig,
    remote: Remote,
    shared: &Arc<Shared>,
    commands: mpsc::Receiver<Command>,
) -> Result<(), MediaError> {
    pw::init(); // Process-global reference library initialization; never deinit another owner's use.
    let mainloop = pw::main_loop::MainLoopRc::new(None).map_err(native)?;
    let context = pw::context::ContextRc::new(&mainloop, None).map_err(native)?;
    let mut properties =
        pw::properties::properties! { "application.name" => config.application_name };
    let core = match remote {
        Remote::Default => context.connect_rc(Some(properties)),
        Remote::Named(name) => {
            properties.insert("remote.name", name);
            context.connect_rc(Some(properties))
        }
        Remote::Portal(fd) => context.connect_fd_rc(fd, Some(properties)),
    }
    .map_err(native)?;
    let registry = core.get_registry_rc().map_err(native)?;
    let bindings = Rc::new(RefCell::new(super::bindings::Bindings::new()));
    let retiring: Rc<RefCell<Vec<(pw::spa::utils::result::AsyncSeq, super::bindings::Binding)>>> =
        Rc::new(RefCell::new(Vec::new()));
    let done_retiring = retiring.clone();
    let removed_retiring = retiring.clone();
    let retire_core = core.clone();
    let failed = Rc::new(RefCell::new(None));
    let pending: Rc<RefCell<Vec<(pw::spa::utils::result::AsyncSeq, Completion, Instant)>>> =
        Rc::new(RefCell::new(Vec::new()));
    #[cfg(any(feature = "desktop-audio-linux", feature = "video-linux"))]
    let mutations: Rc<RefCell<Vec<super::control::PendingMutation>>> =
        Rc::new(RefCell::new(Vec::new()));
    #[cfg(any(feature = "desktop-audio-linux", feature = "video-linux"))]
    let error_mutations = mutations.clone();
    let initial = core.sync(0).map_err(native)?;
    let done_shared = shared.clone();
    let done_pending = pending.clone();
    let error_loop = mainloop.clone();
    let error_failure = failed.clone();
    let error_shared = shared.clone();
    let info_shared = shared.clone();
    let _core_listener = core
        .add_listener_local()
        .info(move |info| {
            info_shared
                .registry
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .snapshot
                .server_version = Some(info.version().chars().take(128).collect());
        })
        .done(move |id, sequence| {
            if id != pw::core::PW_ID_CORE {
                return;
            }
            done_retiring
                .borrow_mut()
                .retain(|(seq, _)| *seq != sequence);
            if sequence == initial {
                done_shared.state(ConnectionState::Ready);
            }
            let mut pending = done_pending.borrow_mut();
            if let Some(index) = pending.iter().position(|(seq, _, _)| *seq == sequence) {
                let (_, completion, _) = pending.remove(index);
                completion.finish(Ok(()));
            }
        })
        .error(move |id, _, result, message| {
            #[cfg(any(feature = "desktop-audio-linux", feature = "video-linux"))]
            for pending in error_mutations.borrow().iter() {
                pending.error(
                    id,
                    if result == -libc::EACCES {
                        MediaError::PermissionDenied
                    } else {
                        native(message)
                    },
                );
            }
            // A fatal core failure invalidates the entire connection, including pending work.
            if id == pw::core::PW_ID_CORE
                && matches!(result, r if r == -libc::EPIPE || r == -libc::ECONNRESET)
            {
                *error_failure.borrow_mut() = Some(if result == -libc::EACCES {
                    MediaError::PermissionDenied
                } else {
                    native(message)
                });
                error_loop.quit();
            } else {
                error_shared
                    .registry
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .snapshot
                    .diagnostics
                    .protocol_errors += 1;
                error_shared.emit(ConnectionEvent::ProtocolError(native(message)));
            }
        })
        .register();
    let global_shared = shared.clone();
    let remove_shared = shared.clone();
    let overflow_failure = failed.clone();
    let overflow_loop = mainloop.clone();
    let global_registry = registry.clone();
    let global_bindings = bindings.clone();
    let remove_bindings = bindings.clone();
    let retire_failure = failed.clone();
    let retire_loop = mainloop.clone();
    let _registry_listener = registry
        .add_listener_local()
        .global(move |global| {
            use pw::types::ObjectType;
            let kind = match global.type_ {
                ObjectType::Device => ObjectKind::Device,
                ObjectType::Node => ObjectKind::Node,
                ObjectType::Port => ObjectKind::Port,
                ObjectType::Link => ObjectKind::Link,
                ObjectType::Metadata => ObjectKind::Metadata,
                _ => return,
            };
            let properties = global
                .props
                .as_ref()
                .map(|props| {
                    props
                        .iter()
                        .take(128)
                        .map(|(k, v)| {
                            (
                                k.chars().take(256).collect(),
                                v.chars().take(1024).collect(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            let result = global_shared
                .registry
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(global.id, kind, global.permissions.bits(), properties);
            match result {
                Ok(handle) => {
                    global_shared.emit(ConnectionEvent::ObjectAdded(handle));
                    match super::bindings::Binding::bind(
                        &global_registry,
                        global,
                        handle,
                        &global_shared,
                    ) {
                        Ok(binding) => {
                            global_bindings.borrow_mut().insert(global.id, binding);
                        }
                        Err(error) => {
                            *overflow_failure.borrow_mut() = Some(error);
                            overflow_loop.quit();
                        }
                    }
                }
                Err(error) => {
                    *overflow_failure.borrow_mut() = Some(error);
                    overflow_loop.quit();
                }
            }
        })
        .global_remove(move |id| {
            if let Some(binding) = remove_bindings.borrow_mut().remove(&id) {
                // Registry removal precedes proxy removal in the native event batch.
                // Retain the proxy until a roundtrip establishes that removal was received.
                if removed_retiring.borrow().len() >= config.max_objects {
                    *retire_failure.borrow_mut() =
                        Some(MediaError::ResourceLimit("retiring registry proxies"));
                    retire_loop.quit();
                } else if let Ok(sequence) = retire_core.sync(0) {
                    removed_retiring.borrow_mut().push((sequence, binding));
                }
            }
            let handle = remove_shared
                .registry
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(id);
            if let Some(handle) = handle {
                remove_shared.emit(ConnectionEvent::ObjectRemoved(handle));
            }
        })
        .register();
    let timer_shared = shared.clone();
    let timer_loop = mainloop.clone();
    let timer_failed = failed.clone();
    let timer_core = core.clone();
    let start = Instant::now();
    #[cfg(feature = "audio-linux")]
    let audio_streams =
        RefCell::new(std::collections::BTreeMap::<u64, super::audio::NativeAudio>::new());
    #[cfg(feature = "audio-linux")]
    let filters =
        RefCell::new(std::collections::BTreeMap::<u64, super::filter::NativeFilter>::new());
    #[cfg(feature = "midi-linux")]
    let midi_streams = RefCell::new(Vec::<super::midi::NativeMidi>::new());
    #[cfg(feature = "video-linux")]
    let video_streams =
        RefCell::new(std::collections::BTreeMap::<u64, super::video::NativeVideo>::new());
    let links = RefCell::new(Vec::<super::graph::NativeLink>::new());
    let timer = mainloop.loop_().add_timer(move |_| {
        if timer_shared.stop.load(Ordering::Acquire) {
            timer_shared.state(ConnectionState::Stopping);
            timer_loop.quit();
            return;
        }
        if start.elapsed() >= config.connect_timeout
            && timer_shared
                .registry
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .snapshot
                .state
                == ConnectionState::Connecting
        {
            *timer_failed.borrow_mut() = Some(MediaError::Timeout);
            timer_loop.quit();
            return;
        }
        pending.borrow_mut().retain(|(_, request, deadline)| {
            if Instant::now() >= *deadline {
                request.finish(Err(MediaError::Timeout));
                false
            } else {
                true
            }
        });
        #[cfg(any(feature = "desktop-audio-linux", feature = "video-linux"))]
        mutations.borrow_mut().retain(|m| {
            m.poll(
                &timer_shared
                    .registry
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .snapshot,
            )
        });
        // Remove explicitly released links before stopping their stream nodes.
        links.borrow_mut().retain(super::graph::NativeLink::alive);
        #[cfg(any(feature = "audio-linux", feature = "midi-linux"))]
        let node_snapshot = timer_shared.registry.lock().unwrap_or_else(|e| e.into_inner()).snapshot.clone();
        #[cfg(feature = "audio-linux")]
        filters.borrow_mut().retain(|_, filter| filter.poll(&node_snapshot));
        #[cfg(feature = "midi-linux")]
        midi_streams
            .borrow_mut()
            .retain_mut(|stream| stream.poll(&node_snapshot));
        #[cfg(feature = "audio-linux")]
        {
            let targets: Vec<_> = {
                let registry = timer_shared
                    .registry
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                registry
                    .snapshot
                    .objects
                    .values()
                    .map(|o| o.handle)
                    .collect()
            };
            audio_streams
                .borrow_mut()
                .retain(|_, stream| stream.poll(&targets));
        }
        #[cfg(feature = "video-linux")]
        if !video_streams.borrow().is_empty() {
            let nodes = timer_shared
                .registry
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .snapshot
                .objects_of_kind(ObjectKind::Node)
                .map(|object| (object.handle.id(), object.handle))
                .collect();
            video_streams
                .borrow_mut()
                .retain(|_, stream| stream.poll(&nodes));
        }
        for _ in 0..config.command_capacity {
            let Ok(command) = commands.try_recv() else {
                break;
            };
            match command {
                #[cfg(feature = "video-linux")]
                Command::CreateVideo(id, config, shared, transfer) => {
                    if video_streams.borrow().len() >= 16 {
                        shared.state(crate::media::video::VideoState::Failed(
                            MediaError::ResourceLimit("video streams"),
                        ));
                        continue;
                    }
                    let snapshot = timer_shared
                        .registry
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .snapshot
                        .clone();
                    match super::video::NativeVideo::create(
                        timer_core.clone(),
                        timer_loop.clone(),
                        config,
                        shared.clone(),
                        transfer,
                        &snapshot,
                    ) {
                        Ok(stream) => {
                            video_streams.borrow_mut().insert(id, stream);
                        }
                        Err(e) => shared.state(crate::media::video::VideoState::Failed(e)),
                    }
                }
                #[cfg(feature = "video-linux")]
                Command::Video(id, command, completion) => {
                    if let Some(stream) = video_streams.borrow_mut().get_mut(&id) {
                        stream.command(command, completion);
                    } else {
                        completion.finish(Err(MediaError::StaleHandle));
                    }
                }

                #[cfg(feature = "midi-linux")]
                Command::CreateMidi(config, shared, endpoint) => {
                    if midi_streams.borrow().len() >= 64 {
                        shared.state(crate::media::midi::MidiState::Failed(
                            MediaError::ResourceLimit("MIDI streams"),
                        ));
                        continue;
                    }
                    match super::midi::NativeMidi::create(
                        timer_core.clone(),
                        config,
                        shared.clone(),
                        endpoint,
                    ) {
                        Ok(stream) => midi_streams.borrow_mut().push(stream),
                        Err(e) => shared.state(crate::media::midi::MidiState::Failed(e)),
                    }
                }

                #[cfg(feature = "audio-linux")]
                Command::CreateFilter(id, config, shared, processor) => {
                    if filters.borrow().len() >= 16 {
                        shared.state(crate::media::audio::FilterState::Failed(
                            MediaError::ResourceLimit("filters"),
                        ));
                        continue;
                    }
                    match super::filter::NativeFilter::create(
                        timer_core.clone(),
                        config,
                        shared.clone(),
                        processor,
                    ) {
                        Ok(filter) => {
                            filters.borrow_mut().insert(id, filter);
                        }
                        Err(e) => shared.state(crate::media::audio::FilterState::Failed(e)),
                    }
                }
                #[cfg(feature = "audio-linux")]
                Command::FilterActive(id, active, completion) => {
                    if let Some(filter) = filters.borrow_mut().get_mut(&id) {
                        filter.set_active(active, completion);
                    } else {
                        completion.finish(Err(MediaError::StaleHandle));
                    }
                }
                Command::Link(creation) => {
                    if links.borrow().len() >= 128 {
                        creation.fail(MediaError::ResourceLimit("owned graph links"));
                        continue;
                    }
                    let created = super::graph::NativeLink::create(
                        timer_core.clone(),
                        creation,
                        &timer_shared,
                        &links.borrow(),
                    );
                    if let Ok(link) = created {
                        links.borrow_mut().push(link);
                    }
                }
                #[cfg(feature = "audio-linux")]
                Command::CreateAudio(id, settings, shared, data) => {
                    if audio_streams.borrow().len() >= 64 {
                        shared.state(crate::media::audio::AudioState::Failed(
                            MediaError::ResourceLimit("audio streams"),
                        ));
                        continue;
                    }
                    let snapshot = timer_shared
                        .registry
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .snapshot
                        .clone();
                    match super::audio::NativeAudio::create(
                        timer_core.clone(),
                        settings,
                        shared.clone(),
                        data,
                        &snapshot,
                    ) {
                        Ok(stream) => {
                            audio_streams.borrow_mut().insert(id, stream);
                        }
                        Err(error) => shared.state(crate::media::audio::AudioState::Failed(error)),
                    }
                }
                #[cfg(feature = "audio-linux")]
                Command::Audio(id, command, completion) => {
                    if let Some(stream) = audio_streams.borrow_mut().get_mut(&id) {
                        stream.command(command, completion);
                    } else {
                        completion.finish(Err(MediaError::StaleHandle));
                    }
                }
                #[cfg(any(feature = "desktop-audio-linux", feature = "video-linux"))]
                Command::Mutate(mutation, completion) => {
                    if mutations.borrow().len() >= config.command_capacity {
                        completion.finish(Err(MediaError::QueueFull));
                        continue;
                    }
                    if let Ok(Some(pending)) = super::control::PendingMutation::start(
                        mutation,
                        completion,
                        &bindings.borrow(),
                        &timer_shared,
                        config.connect_timeout,
                    ) {
                        mutations.borrow_mut().push(pending);
                    }
                }
                Command::Barrier(completion) => {
                    if !completion.begin() {
                        continue;
                    }
                    if pending.borrow().len() == config.command_capacity {
                        completion.finish(Err(MediaError::QueueFull));
                        continue;
                    }
                    match timer_core.sync(0) {
                        Ok(seq) => pending.borrow_mut().push((
                            seq,
                            completion,
                            Instant::now() + config.connect_timeout,
                        )),
                        Err(error) => completion.finish(Err(native(error))),
                    }
                }
            }
        }
    });
    timer
        .update_timer(
            Some(Duration::from_nanos(1)),
            Some(Duration::from_millis(5)),
        )
        .into_result()
        .map_err(native)?;
    mainloop.run();
    let result = failed.borrow_mut().take().map_or(Ok(()), Err);
    result
}

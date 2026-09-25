use super::*;

#[test]
fn capture_config_advertises_only_configured_sources_without_starting_services() {
    for mask in [1u32, 2, 3] {
        let (requests, _) = async_channel::bounded(8);
        let backend = Backend {
            transient: Arc::default(),
            grants: None,
            state: Arc::new(Mutex::new(Sessions::with_source_types(mask))),
            requests,
            wake: Arc::new(|| {}),
        };
        assert_eq!(backend.available_source_types(), mask);
    }
}

#[test]
fn generated_backend_wire_contract_matches_the_portal_methods() {
    use zbus::object_server::Interface;
    let (requests, _) = async_channel::bounded(8);
    let backend = Backend {
        transient: Arc::default(),
        grants: None,
        state: Arc::new(Mutex::new(Sessions::default())),
        requests,
        wake: Arc::new(|| {}),
    };
    // Advertise only source/cursor combinations accepted by this backend. These values
    // are part of the consumer contract, independently of the generated property types.
    assert_eq!(backend.version(), 4);
    assert_eq!(backend.available_source_types(), 7);
    assert_eq!(backend.available_cursor_modes(), 7);
    let mut xml = String::new();
    backend.introspect_to_writer(&mut xml, 0);
    let document = roxmltree::Document::parse(&xml).unwrap();
    let interface = document.root_element();
    assert_eq!(
        interface.attribute("name"),
        Some("org.freedesktop.impl.portal.ScreenCast")
    );
    for (name, input) in [
        ("CreateSession", vec!["o", "o", "s", "a{sv}"]),
        ("SelectSources", vec!["o", "o", "s", "a{sv}"]),
        ("Start", vec!["o", "o", "s", "s", "a{sv}"]),
    ] {
        let method = interface
            .children()
            .find(|n| n.has_tag_name("method") && n.attribute("name") == Some(name))
            .unwrap();
        let signature = |direction| {
            method
                .children()
                .filter(|n| n.has_tag_name("arg") && n.attribute("direction") == Some(direction))
                .map(|n| n.attribute("type").unwrap())
                .collect::<Vec<_>>()
        };
        assert_eq!(signature("in"), input, "{name}");
        assert_eq!(signature("out"), ["u", "a{sv}"], "{name}");
    }
    for name in ["version", "AvailableSourceTypes", "AvailableCursorModes"] {
        let property = interface
            .children()
            .find(|n| n.has_tag_name("property") && n.attribute("name") == Some(name))
            .unwrap();
        assert_eq!(property.attribute("type"), Some("u"));
        assert_eq!(property.attribute("access"), Some("read"));
    }
}

#[test]
fn close_rejects_a_different_unique_bus_owner() {
    let message = zbus::Message::method_call(OBJECT_PATH, "Close")
        .unwrap()
        .sender(":1.2")
        .unwrap()
        .build(&())
        .unwrap();
    assert!(check_owner(&message.header(), ":1.2").is_ok());
    assert!(check_owner(&message.header(), ":1.3").is_err());
}

fn window_info(index: u32) -> StreamInfo {
    use std::num::NonZeroU32;
    StreamInfo {
        node_id: 40 + index,
        width: 800,
        height: 600,
        source: crate::shell::capture::CaptureSource::Window(crate::shell::WindowId::new(
            NonZeroU32::new(index).unwrap(),
            NonZeroU32::new(1).unwrap(),
        )),
    }
}

#[test]
fn persistent_completion_requires_requested_mode_and_matching_ready_sources() {
    for case in 0..7 {
        let lease = Arc::new(Lease::new(1, Arc::new(|| {})));
        let (reply, receive) = async_channel::bounded(1);
        let request = StartRequest {
            requester: 1,
            app_id: "app".into(),
            lease: lease.clone(),
            options: CaptureOptions::default(),
            source_types: 2,
            multiple: true,
            persistence: if case == 1 {
                0
            } else if case == 2 {
                1
            } else {
                2
            },
            restore: None,
            reply,
        };
        let mut keys = vec![restore::RestoreSource {
            kind: 2,
            identity: "host-window-key".into(),
        }];
        match case {
            3 => keys.clear(),
            4 => keys[0].kind = 1,
            5 => keys.push(keys[0].clone()),
            6 => lease.close(),
            _ => {}
        }
        request.complete_persistent(vec![window_info(1)], keys);
        let reply = receive.try_recv().unwrap();
        assert_eq!(reply.is_ok(), matches!(case, 0 | 2));
        if matches!(case, 0 | 2) {
            let completion = reply.unwrap();
            assert_eq!(completion.streams[0].node_id, 41);
            assert_eq!(completion.sources[0].identity, "host-window-key");
            assert!(!lease.closed());
        } else {
            assert!(lease.closed());
        }
    }
}

#[test]
fn persistence_requires_a_live_lease_and_available_worker() {
    block_on(async {
        let lease = Lease::new(1, Arc::new(|| {}));
        let sources = vec![restore::RestoreSource {
            kind: 2,
            identity: "host-window-key".into(),
        }];
        assert!(matches!(
            prepare_grant(None, &lease, "app".into(), sources.clone()).await,
            Err(2)
        ));
        let worker = grant_worker::Worker::start(None).unwrap();
        lease.close();
        assert!(matches!(
            prepare_grant(Some(&worker.client), &lease, "app".into(), sources).await,
            Err(1)
        ));
    });
}

#[test]
fn multi_stream_response_preserves_all_nodes_and_source_metadata() {
    let infos = vec![window_info(1), window_info(2)];
    assert!(valid_streams(&infos, true, 2));
    assert!(!valid_streams(&infos, false, 2));
    let (code, mut result) = stream_response(infos);
    assert_eq!(code, 0);
    let streams: Vec<(u32, HashMap<String, OwnedValue>)> =
        result.remove("streams").unwrap().try_into().unwrap();
    assert_eq!(streams.iter().map(|s| s.0).collect::<Vec<_>>(), [41, 42]);
    for (_, properties) in streams {
        assert_eq!(u32::try_from(&properties["source_type"]).unwrap(), 2);
        assert!(!properties.contains_key("position"));
    }
}

#[test]
fn stream_reply_rejects_empty_duplicate_disallowed_and_unbounded_sets() {
    assert!(!valid_streams(&[], true, 3));
    assert!(!valid_streams(&[window_info(1)], true, 1));
    assert!(!valid_streams(&[window_info(1), window_info(1)], true, 3));
    let mut duplicate_node = window_info(2);
    duplicate_node.node_id = 41;
    assert!(!valid_streams(&[window_info(1), duplicate_node], true, 3));
    let mut duplicate_source = window_info(1);
    duplicate_source.node_id = 42;
    assert!(!valid_streams(&[window_info(1), duplicate_source], true, 3));
    let mut invalid = window_info(1);
    invalid.width = 0;
    assert!(!valid_streams(&[invalid], true, 3));
    let infos: Vec<_> = (1..=MAX_STREAMS as u32 + 1).map(window_info).collect();
    assert!(valid_streams(&infos[..MAX_STREAMS], true, 3));
    assert!(!valid_streams(&infos, true, 3));
}

#[test]
fn invalid_or_cancelled_group_reply_revokes_lease() {
    for cancelled in [false, true] {
        let lease = Arc::new(Lease::new(1, Arc::new(|| {})));
        let (reply, receive) = async_channel::bounded(1);
        let request = StartRequest {
            requester: 1,
            app_id: "test".into(),
            lease: lease.clone(),
            options: CaptureOptions::default(),
            source_types: 2,
            multiple: true,
            persistence: 0,
            restore: None,
            reply,
        };
        if cancelled {
            lease.close();
        }
        request.complete(Ok(if cancelled {
            vec![window_info(1)]
        } else {
            vec![]
        }));
        assert_eq!(receive.try_recv().unwrap().unwrap_err(), 2);
        assert!(lease.closed());
    }
}

#[test]
fn start_response_uses_portal_stream_tuple_signature() {
    let (code, result) = stream_response(vec![StreamInfo {
        node_id: 42,
        width: 800,
        height: 600,
        source: crate::shell::capture::CaptureSource::Output(crate::shell::OutputId::MIN),
    }]);
    assert_eq!(code, 0);
    assert_eq!(result["streams"].value_signature().to_string(), "a(ua{sv})");
    assert_eq!(u32::try_from(&result["persist_mode"]).unwrap(), 0);
}

#[test]
fn window_stream_metadata_does_not_claim_a_monitor_position() {
    use crate::shell::{WindowId, capture::CaptureSource};
    use std::num::NonZeroU32;
    let (_, mut result) = stream_response(vec![StreamInfo {
        node_id: 42,
        width: 800,
        height: 600,
        source: CaptureSource::Window(WindowId::new(
            NonZeroU32::new(1).unwrap(),
            NonZeroU32::new(2).unwrap(),
        )),
    }]);
    let streams: Vec<(u32, HashMap<String, OwnedValue>)> =
        result.remove("streams").unwrap().try_into().unwrap();
    assert_eq!(streams.len(), 1);
    assert_eq!(streams[0].0, 42);
    assert_eq!(u32::try_from(&streams[0].1["source_type"]).unwrap(), 2);
    assert!(!streams[0].1.contains_key("position"));
    let size: (i32, i32) = streams[0].1["size"]
        .try_clone()
        .unwrap()
        .try_into()
        .unwrap();
    assert_eq!(size, (800, 600));
}

#[test]
fn ordinary_close_keeps_saved_grant_but_explicit_forget_revokes_it() {
    block_on(async {
        let path = std::env::temp_dir().join(format!(
            "telorgon-forget-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let worker = grant_worker::Worker::start(Some(path.clone())).unwrap();
        let grant = worker
            .client
            .issue(
                "app".into(),
                vec![restore::RestoreSource {
                    kind: 2,
                    identity: "host-window".into(),
                }],
            )
            .await
            .unwrap();
        let state = Arc::new(Mutex::new(Sessions::default()));
        let lease = state
            .lock()
            .unwrap()
            .create(
                "/org/freedesktop/portal/desktop/session/test/grant",
                ":1.2",
                "app",
                Arc::new(|| {}),
            )
            .unwrap();
        *lease.saved.lock().unwrap() = Some(grant.clone());
        let (requests, _) = async_channel::bounded(1);
        let backend = Backend {
            transient: Arc::default(),
            state,
            requests,
            grants: Some(worker.client.clone()),
            wake: Arc::new(|| {}),
        };
        lease.close();
        revoke_forgotten(&backend).await;
        assert!(lease.has_saved_grant());
        assert!(
            worker
                .client
                .resolve(grant.clone(), "app".into(), 2, false)
                .await
                .unwrap()
                .is_some()
        );
        lease.forget();
        revoke_forgotten(&backend).await;
        assert!(!lease.has_saved_grant());
        assert!(
            worker
                .client
                .resolve(grant, "app".into(), 2, false)
                .await
                .unwrap()
                .is_none()
        );
        assert!(backend.state.lock().unwrap().failures.is_empty());
        drop(worker);
        std::fs::remove_dir_all(path).unwrap();
    });
}

#[test]
fn failed_forget_is_reported_without_claiming_permission_was_removed() {
    block_on(async {
        let state = Arc::new(Mutex::new(Sessions::default()));
        let lease = state
            .lock()
            .unwrap()
            .create(
                "/org/freedesktop/portal/desktop/session/test/grant",
                ":1.2",
                "app",
                Arc::new(|| {}),
            )
            .unwrap();
        *lease.saved.lock().unwrap() = Some(restore::RestoreData {
            app_id: "app".into(),
            grant_id: "0".repeat(64),
            sources: vec![restore::RestoreSource {
                kind: 2,
                identity: "host-window".into(),
            }],
        });
        let (requests, _) = async_channel::bounded(1);
        let backend = Backend {
            transient: Arc::default(),
            state,
            requests,
            grants: None,
            wake: Arc::new(|| {}),
        };
        lease.forget();
        revoke_forgotten(&backend).await;
        assert!(lease.closed());
        assert!(lease.has_saved_grant());
        assert_eq!(backend.state.lock().unwrap().failures.len(), 1);
        revoke_forgotten(&backend).await;
        assert_eq!(backend.state.lock().unwrap().failures.len(), 1);
    });
}

#[test]
fn restored_completion_reuses_only_the_exact_verified_source_keys() {
    for altered in [false, true] {
        let lease = Arc::new(Lease::new(1, Arc::new(|| {})));
        let (reply, receive) = async_channel::bounded(1);
        let grant = restore::RestoreData {
            app_id: "app".into(),
            grant_id: "0".repeat(64),
            sources: vec![restore::RestoreSource {
                kind: 2,
                identity: "host-window".into(),
            }],
        };
        let request = StartRequest {
            requester: 1,
            app_id: "app".into(),
            lease: lease.clone(),
            options: CaptureOptions::default(),
            source_types: 2,
            multiple: false,
            persistence: 2,
            restore: Some(grant.clone()),
            reply,
        };
        let mut keys = grant.sources;
        if altered {
            keys[0].identity = "other-window".into();
        }
        request.complete_restored(vec![window_info(1)], keys);
        let result = receive.try_recv().unwrap();
        if altered {
            assert!(result.is_err());
            assert!(lease.closed());
        } else {
            let completed = result.unwrap();
            assert!(completed.reuse);
            assert!(completed.persist);
        }
    }
}

#[test]
fn unavailable_source_fallback_does_not_attach_old_permission_to_new_selection() {
    let lease = Arc::new(Lease::new(1, Arc::new(|| {})));
    let (reply, _) = async_channel::bounded(1);
    let grant = restore::RestoreData {
        app_id: "app".into(),
        grant_id: "0".repeat(64),
        sources: vec![restore::RestoreSource {
            kind: 2,
            identity: "old-window".into(),
        }],
    };
    *lease.saved.lock().unwrap() = Some(grant.clone());
    let mut request = StartRequest {
        requester: 1,
        app_id: "app".into(),
        lease: lease.clone(),
        options: CaptureOptions::default(),
        source_types: 2,
        multiple: false,
        persistence: 2,
        restore: Some(grant),
        reply,
    };
    request.discard_restore();
    assert!(request.restore.is_none());
    assert!(!lease.has_saved_grant());
    assert!(!lease.closed());
}

#[test]
fn transient_forget_needs_no_filesystem_worker_and_stop_preserves_restoration() {
    block_on(async {
        let transient = Arc::new(Mutex::new(transient_grants::Store::default()));
        let grant = transient_grants::Store::prepare(
            &transient,
            ":1.2".into(),
            "app".into(),
            vec![RestoreSource {
                kind: 2,
                identity: "window-epoch".into(),
            }],
        )
        .unwrap()
        .claim();
        let state = Arc::new(Mutex::new(Sessions::default()));
        let lease = state
            .lock()
            .unwrap()
            .create(
                "/org/freedesktop/portal/desktop/session/test/temporary",
                ":1.2",
                "app",
                Arc::new(|| {}),
            )
            .unwrap();
        *lease.saved.lock().unwrap() = Some(grant.clone());
        let (requests, _) = async_channel::bounded(1);
        let backend = Backend {
            transient: transient.clone(),
            grants: None,
            state,
            requests,
            wake: Arc::new(|| {}),
        };
        lease.close();
        revoke_forgotten(&backend).await;
        assert!(
            transient
                .lock()
                .unwrap()
                .resolve(":1.2", &grant, "app", 2, false)
                .is_some()
        );
        lease.forget();
        revoke_forgotten(&backend).await;
        assert!(!lease.has_saved_grant());
        assert!(
            transient
                .lock()
                .unwrap()
                .resolve(":1.2", &grant, "app", 2, false)
                .is_none()
        );
        assert!(backend.state.lock().unwrap().failures.is_empty());
    });
}

#[path = "backend_isolated.rs"]
mod isolated;

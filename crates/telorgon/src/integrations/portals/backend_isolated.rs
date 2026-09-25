//! Private-bus protocol qualification. Host replies use synthetic source/node identities;
//! this does not qualify a real portal frontend, compositor, or PipeWire capture source.
use super::*;
use futures_lite::future::zip;
type Dict = HashMap<String, OwnedValue>;

async fn frontend(address: &str) -> Connection {
    zbus::connection::Builder::address(address)
        .unwrap()
        .name(FRONTEND)
        .unwrap()
        .build()
        .await
        .unwrap()
}

async fn capture(
    frontend: &Connection,
    backend: &Backend,
    requests: &Receiver<StartRequest>,
    index: u32,
    mode: u32,
    hint: Option<restore::RestoreData>,
    expected: Option<u32>,
    restored: bool,
) -> Dict {
    let proxy = zbus::Proxy::new(
        frontend,
        BUS_NAME,
        OBJECT_PATH,
        "org.freedesktop.impl.portal.ScreenCast",
    )
    .await
    .unwrap();
    assert_eq!(proxy.get_property::<u32>("version").await.unwrap(), 4);
    let handle: OwnedObjectPath = format!("{OBJECT_PATH}/request/test/r{index}")
        .try_into()
        .unwrap();
    let session: OwnedObjectPath = format!("{OBJECT_PATH}/session/test/s{index}")
        .try_into()
        .unwrap();
    let result: (u32, Dict) = proxy
        .call(
            "CreateSession",
            &(handle.clone(), session.clone(), "app", Dict::new()),
        )
        .await
        .unwrap();
    assert_eq!(result.0, 0);
    let mut options = Dict::from([
        ("types".into(), 2u32.into()),
        ("persist_mode".into(), mode.into()),
    ]);
    if let Some(hint) = hint {
        options.insert("restore_data".into(), hint.encode().unwrap());
    }
    let result: (u32, Dict) = proxy
        .call(
            "SelectSources",
            &(handle.clone(), session.clone(), "app", options),
        )
        .await
        .unwrap();
    assert_eq!(result.0, 0);
    let body = (handle, session.clone(), "app", "", Dict::new());
    let start = proxy.call::<_, _, (u32, Dict)>("Start", &body);
    let host = async {
        let request = requests.recv().await.unwrap();
        assert_eq!(request.restore.is_some(), restored);
        if let Some(expected) = expected {
            assert_eq!(request.persistence, expected);
            let sources = vec![RestoreSource {
                kind: 2,
                identity: "synthetic-window-epoch".into(),
            }];
            if restored {
                request.complete_restored(vec![window_info(1)], sources);
            } else {
                request.complete_persistent(vec![window_info(1)], sources);
            }
        } else {
            request.complete(Err(1));
        }
    };
    let (result, ()) = zip(start, host).await;
    let (code, results) = result.unwrap();
    assert_eq!(code, if expected.is_some() { 0 } else { 1 });
    let close = zbus::Proxy::new(
        frontend,
        BUS_NAME,
        session.clone(),
        "org.freedesktop.impl.portal.Session",
    )
    .await
    .unwrap();
    close.call::<_, _, ()>("Close", &()).await.unwrap();
    // The production serve loop retires closed exported objects. No capture transport exists
    // in this fixture; release only its closed registry entry to permit the next protocol case.
    let removed = backend
        .state
        .lock()
        .unwrap()
        .entries
        .remove(session.as_str())
        .unwrap();
    assert!(removed.lease.closed());
    results
}
fn grant(results: &Dict, mode: u32) -> restore::RestoreData {
    assert_eq!(u32::try_from(&results["persist_mode"]).unwrap(), mode);
    restore::RestoreData::decode(&results["restore_data"], "app", 2, false).unwrap()
}

#[test]
#[ignore = "requires test_pipewire.py --portal-backend-unit; private D-Bus only"]
fn temporary_and_durable_restoration_wire_lifetimes() {
    let address = std::env::var("TELORGON_TEST_BUS_ADDRESS").expect("use private harness");
    assert!(address.starts_with("unix:path=/tmp/telorgon-pw-"));
    let directory = std::path::Path::new(address.strip_prefix("unix:path=").unwrap())
        .parent()
        .unwrap()
        .join("backend-grants");
    block_on(race(
        async {
            let worker = grant_worker::Worker::start(Some(directory.clone())).unwrap();
            let (send, receive) = async_channel::bounded(8);
            let backend = Backend {
                transient: Arc::default(),
                grants: Some(worker.client.clone()),
                state: Arc::default(),
                requests: send,
                wake: Arc::new(|| {}),
            };
            let _server = zbus::connection::Builder::address(address.as_str())
                .unwrap()
                .name(BUS_NAME)
                .unwrap()
                .serve_at(OBJECT_PATH, backend.clone())
                .unwrap()
                .build()
                .await
                .unwrap();
            let front = frontend(&address).await;
            let first = capture(&front, &backend, &receive, 1, 1, None, Some(1), false).await;
            let temporary = grant(&first, 1);
            assert!(!directory.exists(), "mode 1 must not open durable storage");
            let second = capture(
                &front,
                &backend,
                &receive,
                2,
                2,
                Some(temporary.clone()),
                Some(1),
                true,
            )
            .await;
            assert_eq!(
                grant(&second, 1),
                temporary,
                "reuse never upgrades temporary consent"
            );
            assert!(!directory.exists());
            let durable = grant(
                &capture(&front, &backend, &receive, 3, 2, None, Some(2), false).await,
                2,
            );
            assert!(directory.exists());
            assert_eq!(
                grant(
                    &capture(
                        &front,
                        &backend,
                        &receive,
                        4,
                        2,
                        Some(durable.clone()),
                        Some(2),
                        true
                    )
                    .await,
                    2
                ),
                durable
            );
            front.release_name(FRONTEND).await.unwrap();
            let replacement = frontend(&address).await;
            // Even before the periodic retirement tick, a new broker cannot reuse old temporary data.
            capture(
                &replacement,
                &backend,
                &receive,
                5,
                2,
                Some(temporary),
                None,
                false,
            )
            .await;
            assert_eq!(
                grant(
                    &capture(
                        &replacement,
                        &backend,
                        &receive,
                        6,
                        2,
                        Some(durable.clone()),
                        Some(2),
                        true
                    )
                    .await,
                    2
                ),
                durable
            );
            worker
                .client
                .revoke("app".into(), Some(durable.grant_id.clone()))
                .await
                .unwrap();
            capture(
                &replacement,
                &backend,
                &receive,
                7,
                2,
                Some(durable),
                None,
                false,
            )
            .await;
            let untrusted = zbus::Proxy::new(
                &front,
                BUS_NAME,
                OBJECT_PATH,
                "org.freedesktop.impl.portal.ScreenCast",
            )
            .await
            .unwrap();
            let path: OwnedObjectPath = format!("{OBJECT_PATH}/request/test/untrusted")
                .try_into()
                .unwrap();
            assert!(
                untrusted
                    .call::<_, _, (u32, Dict)>(
                        "CreateSession",
                        &(path.clone(), path, "app", Dict::new())
                    )
                    .await
                    .is_err()
            );
        },
        async {
            async_io::Timer::after(Duration::from_secs(15)).await;
            panic!("private backend protocol test timed out");
        },
    ));
}

use super::*;
#[test]
fn failed_preparation_or_deadline_keeps_unrelated_wayland_client_alive() {
    for timeout in [false, true] {
        let mut display = Display::new().unwrap();
        let access = XwaylandAccess::configure_display(&mut display).unwrap();
        let (socket, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let native_client = display.create_client(socket).unwrap();
        let (send, receive) = mpsc::sync_channel(1);
        send.send(Err(crate::integrations::x11::Error(
            "fixture preparation failure".into(),
        )))
        .unwrap();
        let now = Instant::now();
        let mut host = Compatibility {
            root_cursor: None,
            sources: Vec::new(),
            ready: Box::new(AtomicBool::new(true)),
            preparation: Some(receive),
            helper: None,
            client: None,
            xwm: None,
            notification: None,
            access,
            deadline: Some(if timeout {
                now
            } else {
                now + Duration::from_secs(10)
            }),
            failed: false,
            initialized: false,
            environment: None,
            serials: BTreeMap::new(),
            identities: BTreeSet::new(),
            descendants: BTreeSet::new(),
            desktop: Default::default(),
            policy_repaint: false,
            closing: BTreeMap::new(),
            pending_focus: None,
            pending_raise: None,
            focus_result: None,
        };
        host.environment = Some((":123".into(), "/private/fixture-auth".into()));
        assert_eq!(host.environment(), None);
        host.initialized = true;
        assert!(host.environment().is_some());
        assert_eq!(host.wait(None), Some(Duration::ZERO));
        host.dispatch(&display);
        assert!(host.failed);
        assert_eq!(host.environment(), None);
        let root = WaylandSurfaceId::from_raw(1).unwrap();
        let child = WaylandSurfaceId::from_raw(2).unwrap();
        let mut image = super::super::client::maximize_preview_tests::test_window(
            SizeI {
                width: 30,
                height: 20,
            },
            PointI { x: 10, y: 10 },
        );
        image.role = SurfaceRole::Xwayland;
        let mut sub = super::super::client::maximize_preview_tests::test_window(
            SizeI {
                width: 5,
                height: 5,
            },
            PointI::default(),
        );
        sub.role = SurfaceRole::Subsurface;
        sub.parent = Some(root);
        sub.offset = PointI { x: 2, y: 3 };
        let mut windows = BTreeMap::from([(root, image), (child, sub)]);
        let mut identities = WindowIdentities::default();
        assert!(
            host.sync_presentation(&mut windows, &mut identities, &LinuxShellConfig::default())
                .unwrap()
        );
        assert!(windows[&root].minimized);
        assert!(windows[&child].minimized);
        assert_eq!(windows[&child].position, PointI { x: 12, y: 13 });
        assert!(
            !host
                .sync_presentation(&mut windows, &mut identities, &LinuxShellConfig::default())
                .unwrap()
        );

        windows.remove(&root);
        windows.get_mut(&child).unwrap().minimized = false;
        assert!(
            host.sync_presentation(&mut windows, &mut identities, &LinuxShellConfig::default())
                .unwrap()
        );
        assert!(windows[&child].minimized);
        assert!(host.preparation.is_none());
        assert_eq!(host.wait(None), None);
        assert!(native_client.is_alive());
        host.dispatch(&display);
        drop(host);
        assert!(native_client.is_alive());
    }
}

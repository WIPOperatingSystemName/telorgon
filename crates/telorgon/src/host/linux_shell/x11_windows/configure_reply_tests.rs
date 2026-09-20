use super::*;
#[test]
fn configure_reply_does_not_require_a_wayland_image() {
    use crate::integrations::x11::xwm::decoration_tests::wire_request as request;
    use crate::integrations::x11::xwm::decoration_tests::{drive, fixture};
    let (mut xwm, mut peer, now, id) = fixture();
    let mut adapter = X11Windows::default();
    // A previous image's adapter may remain while the X window is unmapped.
    adapter.entries.insert(
        id,
        Entry {
            surface: WaylandSurfaceId::from_raw(100).unwrap(),
            sent: None,
            notify_border: None,
            requested_border: 5,
        },
    );
    adapter.configure_requested(id, RequestedConfigure::default());
    let mut submitted = 0;
    adapter.flush_unattached(&mut xwm, &mut submitted).unwrap();
    assert_eq!(submitted, 1);
    drive(&mut xwm, now);
    let bytes = request(&mut peer);
    assert_eq!(bytes[0], 25); // SendEvent, with no preceding ConfigureWindow for a no-op.
    assert_eq!(bytes[12] & 0x7f, 22); // ConfigureNotify.
    assert_eq!(
        u32::from_ne_bytes(bytes[20..24].try_into().unwrap()),
        id.xid
    );
    assert!(adapter.requests.is_empty());
    assert!(adapter.unattached_replies.is_empty());
}

#[test]
fn denied_and_no_op_requests_keep_actual_geometry_and_last_requested_border() {
    use super::super::client::maximize_preview_tests::test_window;
    let surface = WaylandSurfaceId::from_raw(10).unwrap();
    let id = XWindow {
        generation: 1,
        xid: 100,
        incarnation: 1,
    };
    let geometry = Geometry {
        x: 10,
        y: 20,
        width: 640,
        height: 480,
        border: 0,
    };
    let config = LinuxShellConfig::default();
    let mut window = test_window(
        SizeI {
            width: 640,
            height: 480,
        },
        PointI::default(),
    );
    window.role = SurfaceRole::Xwayland;
    let mut adapter = X11Windows::default();
    adapter.attach(id, surface, geometry, false, &mut window, &config);
    window.maximized = true;
    let original = (window.position, window.requested_size);
    adapter.configure_requested(
        id,
        RequestedConfigure {
            x: Some(50),
            width: Some(320),
            border: Some(5),
            ..Default::default()
        },
    );
    adapter.attach(id, surface, geometry, false, &mut window, &config);
    assert_eq!((window.position, window.requested_size), original);
    assert_eq!(adapter.entries[&id].notify_border, Some(5));
    adapter.entries.get_mut(&id).unwrap().notify_border = None;
    adapter.configure_requested(id, RequestedConfigure::default());
    adapter.attach(id, surface, geometry, false, &mut window, &config);
    assert_eq!(adapter.entries[&id].notify_border, Some(5));
}

#[test]
fn synthetic_coordinates_preserve_client_origin_with_requested_border() {
    let actual = Geometry {
        x: 10,
        y: 20,
        width: 640,
        height: 480,
        border: 0,
    };
    assert_eq!(
        notification_geometry(actual, 3),
        Geometry {
            x: 7,
            y: 17,
            border: 3,
            ..actual
        }
    );
    assert_eq!(notification_geometry(actual, 0), actual);
}

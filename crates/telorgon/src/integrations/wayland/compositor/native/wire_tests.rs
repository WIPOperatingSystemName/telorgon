use super::*;
use std::{io::Write, os::unix::net::UnixStream, time::Duration};

fn send(peer: &mut UnixStream, object: u32, opcode: u16, payload: &[u8]) {
    let mut bytes = object.to_ne_bytes().to_vec();
    bytes.extend_from_slice(&((((payload.len() + 8) as u32) << 16) | opcode as u32).to_ne_bytes());
    bytes.extend_from_slice(payload);
    peer.write_all(&bytes).unwrap();
}
fn words(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_ne_bytes()).collect()
}

fn decoration_roundtrip(
    display: &Display,
    peer: &mut UnixStream,
    callback: u32,
) -> (Option<u32>, Option<u32>) {
    send(peer, 1, 0, &words(&[callback]));
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    let (mut mode, mut serial) = (None, None);
    loop {
        let mut header = [0; 8];
        peer.read_exact(&mut header).unwrap();
        let object = u32::from_ne_bytes(header[..4].try_into().unwrap());
        let word = u32::from_ne_bytes(header[4..].try_into().unwrap());
        let mut body = vec![0; (word >> 16) as usize - 8];
        peer.read_exact(&mut body).unwrap();
        if object == callback {
            return (mode, serial);
        }
        if object == 10 && word & 0xffff == 0 {
            mode = Some(u32::from_ne_bytes(body[..4].try_into().unwrap()));
        }
        if object == 8 && word & 0xffff == 0 {
            serial = Some(u32::from_ne_bytes(body[..4].try_into().unwrap()));
        }
    }
}

#[test]
fn decoration_policy_wire_configures_are_committed_atomically() {
    use crate::integrations::wayland::compositor::DecorationMode;
    use crate::{DecorationNegotiation, DecorationPolicy};
    for (negotiation, initial_request) in [
        (DecorationNegotiation::ClientPreference, None),
        (DecorationNegotiation::ClientPreference, Some(1)),
        (DecorationNegotiation::ClientPreference, Some(2)),
        (DecorationNegotiation::PreferServer, Some(1)),
    ] {
        let display = Display::new().unwrap();
        let (mut peer, socket) = UnixStream::pair().unwrap();
        let client = display.create_client(socket).unwrap();
        let mut native = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
        native.set_decoration_policy(DecorationPolicy {
            negotiation,
            ..Default::default()
        });
        let globals = registry(&display, &mut peer);
        bind(&mut peer, &globals, "wl_compositor", 4);
        bind(&mut peer, &globals, "xdg_wm_base", 5);
        bind(&mut peer, &globals, "zxdg_decoration_manager_v1", 6);
        send(&mut peer, 4, 0, &words(&[7]));
        send(&mut peer, 5, 2, &words(&[8, 7]));
        send(&mut peer, 8, 1, &words(&[9]));
        send(&mut peer, 6, 1, &words(&[10, 9]));
        if let Some(mode) = initial_request {
            send(&mut peer, 10, 1, &words(&[mode]));
        }
        send(&mut peer, 7, 6, &[]);
        let (mode, serial) = decoration_roundtrip(&display, &mut peer, 11);
        let initial_mode = if negotiation == DecorationNegotiation::PreferServer {
            2
        } else {
            initial_request.unwrap_or(2)
        };
        assert_eq!(mode, Some(initial_mode)); // absent preference defaults to server
        send(&mut peer, 8, 4, &words(&[serial.unwrap()]));
        send(&mut peer, 7, 6, &[]);
        decoration_roundtrip(&display, &mut peer, 12);
        let client_id = native.state.clients[&client.identity().unwrap()];
        let surface = native.core().world.client_surfaces(client_id)[0];
        assert_eq!(
            native.decoration_mode(surface),
            Some(if initial_mode == 2 {
                DecorationMode::ServerSide
            } else {
                DecorationMode::ClientSide
            })
        );
        for (index, request) in [Some(1), Some(2), Some(1), None].into_iter().enumerate() {
            // A decoration-only configure must preserve an outstanding geometry/state request.
            let size = crate::foundation::SizeI {
                width: 700,
                height: 500,
            };
            let states = crate::integrations::wayland::compositor::ToplevelState {
                activated: true,
                ..Default::default()
            };
            native
                .configure_toplevel(surface, Some(size), states)
                .unwrap();
            let old = native.decoration_mode(surface);
            match request {
                Some(mode) => send(&mut peer, 10, 1, &words(&[mode])),
                None => send(&mut peer, 10, 2, &[]),
            }
            let callback = 13 + index as u32 * 2;
            let (mode, serial) = decoration_roundtrip(&display, &mut peer, callback);
            let expected = if negotiation == DecorationNegotiation::PreferServer {
                2
            } else {
                request.unwrap_or(2)
            };
            assert_eq!(mode, Some(expected));
            assert_eq!(native.decoration_mode(surface), old);
            let latest = native
                .state
                .core
                .xdg_surface_mut(surface)
                .unwrap()
                .latest_configure()
                .unwrap();
            assert_eq!(latest.size, Some(size));
            assert_eq!(latest.states, states);
            send(&mut peer, 8, 4, &words(&[serial.unwrap()]));
            send(&mut peer, 7, 6, &[]);
            decoration_roundtrip(&display, &mut peer, callback + 1);
            assert_eq!(
                native.decoration_mode(surface),
                Some(if expected == 2 {
                    DecorationMode::ServerSide
                } else {
                    DecorationMode::ClientSide
                })
            );
        }
        assert!(client.is_alive());
    }
}

#[test]
fn committed_release_survives_surface_destruction_and_finishes_once() {
    let display = Display::new().unwrap();
    let (mut peer, socket) = UnixStream::pair().unwrap();
    let client = display.create_client(socket).unwrap();
    let mut native = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
    let globals = registry(&display, &mut peer);
    bind_version(&mut peer, &globals, "wl_compositor", 4, 4);
    send(&mut peer, 4, 0, &words(&[5]));
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    let client_id = native.state.clients[&client.identity().unwrap()];
    let surface = native.core().world.client_surfaces(client_id)[0];
    let client_ref =
        unsafe { ClientRef::from_raw(client.identity().unwrap() as *mut ffi::wl_client).unwrap() };
    let release = native
        .state
        .create_resource(
            client_ref,
            client_id,
            "zwp_linux_buffer_release_v1",
            1,
            6,
            ResourceKind::ExplicitBufferRelease(surface),
            true,
        )
        .unwrap();
    let object = unsafe { &*release.user_data().cast::<ResourceContext>() }.object;
    native.state.committed_releases.insert((surface, 2), object);
    send(&mut peer, 5, 0, &[]);
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    assert!(native.core().world.surface(surface).is_none());
    assert!(native.finish_explicit_release(surface, 2, None).unwrap());
    assert!(!native.finish_explicit_release(surface, 2, None).unwrap());
    assert!(!native.state.resources.contains_key(&object));
}

#[test]
fn destroyed_buffer_survives_surface_commits_until_last_reference_is_removed() {
    use crate::integrations::wayland::compositor::{ShmBuffer, ShmFormat};
    for retirement in ["detach", "replace", "destroy", "disconnect"] {
        let display = Display::new().unwrap();
        let (mut peer, socket) = UnixStream::pair().unwrap();
        let client = display.create_client(socket).unwrap();
        let mut native = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
        let globals = registry(&display, &mut peer);
        bind_version(&mut peer, &globals, "wl_compositor", 4, 4);
        send(&mut peer, 4, 0, &words(&[5]));
        send(&mut peer, 4, 0, &words(&[6]));
        display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
        let client_id = native.state.clients[&client.identity().unwrap()];
        let client_ref = unsafe {
            ClientRef::from_raw(client.identity().unwrap() as *mut ffi::wl_client).unwrap()
        };
        let buffer = WaylandBufferId::from_raw(100).unwrap();
        let replacement = WaylandBufferId::from_raw(101).unwrap();
        for (id, wire_id) in [(buffer, 7), (replacement, 8)] {
            native
                .state
                .core
                .register_buffer(
                    client_id,
                    id,
                    BufferDescriptor::Shm(ShmBuffer {
                        offset: 0,
                        size: crate::foundation::SizeI {
                            width: 16,
                            height: 8,
                        },
                        stride: 64,
                        format: ShmFormat::Argb8888,
                    }),
                )
                .unwrap();
            native
                .state
                .buffer_files
                .insert(id, std::fs::File::open("/dev/zero").unwrap().into());
            native
                .state
                .create_resource(
                    client_ref,
                    client_id,
                    "wl_buffer",
                    1,
                    wire_id,
                    ResourceKind::Buffer(id),
                    true,
                )
                .unwrap();
        }
        // Share the attachment to exercise last-reference cleanup as well as the failure.
        for surface in [5, 6] {
            send(&mut peer, surface, 1, &words(&[7, 0, 0]));
            send(&mut peer, surface, 6, &[]);
        }
        display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
        native.release_buffer(buffer).unwrap();
        send(&mut peer, 7, 0, &[]);
        send(&mut peer, 5, 8, &words(&[2])); // Metadata-only scale change.
        send(&mut peer, 5, 6, &[]);
        display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
        assert!(client.is_alive(), "{retirement}");
        assert!(native.core().buffer(buffer).is_some());
        assert_eq!(
            native.read_shm_buffer(buffer).unwrap().pixels.len(),
            16 * 8 * 4
        );
        native.release_buffer(buffer).unwrap(); // No event for a destroyed protocol object.
        let surface = native.core().world.client_surfaces(client_id)[0];
        assert_eq!(
            native.state.surface_logical_size(surface).unwrap(),
            crate::foundation::SizeI {
                width: 8,
                height: 4
            }
        );
        if retirement == "disconnect" {
            client.disconnect();
        } else {
            for surface in [5, 6] {
                if retirement == "destroy" {
                    send(&mut peer, surface, 0, &[]);
                } else {
                    let next = if retirement == "replace" { 8 } else { 0 };
                    send(&mut peer, surface, 1, &words(&[next, 0, 0]));
                    send(&mut peer, surface, 6, &[]);
                }
                display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
                assert!(client.is_alive());
                assert_eq!(native.core().buffer(buffer).is_some(), surface == 5);
            }
        }
        assert!(!native.state.buffer_files.contains_key(&buffer));
        assert!(native.state.destroyed_buffers.is_empty());
    }
}

#[test]
fn destroyed_buffer_retains_pending_and_synchronized_cached_storage() {
    use crate::integrations::wayland::compositor::{ShmBuffer, ShmFormat, SurfaceCommit};
    let display = Display::new().unwrap();
    let mut native = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
    let client = ClientId::from_raw(1).unwrap();
    let parent = WaylandSurfaceId::from_raw(1).unwrap();
    let child = WaylandSurfaceId::from_raw(2).unwrap();
    let buffer = WaylandBufferId::from_raw(1).unwrap();
    native.state.core.connect_client(client).unwrap();
    for surface in [parent, child] {
        native
            .state
            .core
            .world
            .create_surface(client, surface)
            .unwrap();
    }
    native
        .state
        .core
        .register_buffer(
            client,
            buffer,
            BufferDescriptor::Shm(ShmBuffer {
                offset: 0,
                size: crate::foundation::SizeI {
                    width: 16,
                    height: 8,
                },
                stride: 64,
                format: ShmFormat::Argb8888,
            }),
        )
        .unwrap();
    let attachment = Some(BufferAttachment {
        buffer,
        offset: PointI::default(),
    });
    native.state.surface_mut(child).unwrap().attach(attachment);
    native.state.destroyed_buffers.insert(buffer, client);
    native.state.collect_destroyed_buffers();
    assert!(native.core().buffer(buffer).is_some());
    native.state.core.subsurfaces.add(child, parent).unwrap();
    native
        .state
        .core
        .subsurfaces
        .stage_or_release(
            child,
            SurfaceCommit {
                attachment: Some(attachment),
                ..Default::default()
            },
        )
        .unwrap();
    // A later pending detach must not retire the separately cached attachment.
    native.state.surface_mut(child).unwrap().attach(None);
    native.state.collect_destroyed_buffers();
    assert!(native.core().buffer(buffer).is_some());
    let (_, commit) = native
        .state
        .core
        .subsurfaces
        .release_children(parent)
        .pop()
        .unwrap();
    let state = native.state.surface_mut(child).unwrap();
    state.stage(commit).unwrap();
    state.commit().unwrap();
    native.state.collect_destroyed_buffers();
    assert!(
        native
            .state
            .validate_surface_buffer_geometry(child, None)
            .is_ok()
    );
    let state = native.state.surface_mut(child).unwrap();
    state.set_buffer_scale(3).unwrap();
    state.commit().unwrap();
    assert!(
        native
            .state
            .validate_surface_buffer_geometry(child, None)
            .is_err()
    );
    let state = native.state.surface_mut(child).unwrap();
    state.attach(None);
    state.commit().unwrap();
    native.state.collect_destroyed_buffers();
    assert!(native.core().buffer(buffer).is_none());
}
fn registry(display: &Display, peer: &mut UnixStream) -> BTreeMap<String, u32> {
    peer.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    send(peer, 1, 1, &words(&[2]));
    send(peer, 1, 0, &words(&[3]));
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    let mut globals = BTreeMap::new();
    loop {
        let mut header = [0; 8];
        peer.read_exact(&mut header).unwrap();
        let object = u32::from_ne_bytes(header[..4].try_into().unwrap());
        let size = u32::from_ne_bytes(header[4..].try_into().unwrap()) >> 16;
        let mut body = vec![0; size as usize - 8];
        peer.read_exact(&mut body).unwrap();
        if object == 3 {
            return globals;
        }
        if object == 2 {
            let name = u32::from_ne_bytes(body[..4].try_into().unwrap());
            let length = u32::from_ne_bytes(body[4..8].try_into().unwrap()) as usize;
            globals.insert(
                String::from_utf8(body[8..8 + length - 1].to_vec()).unwrap(),
                name,
            );
        }
    }
}
fn bind(peer: &mut UnixStream, globals: &BTreeMap<String, u32>, interface: &str, id: u32) {
    bind_version(peer, globals, interface, id, 1);
}
fn bind_version(
    peer: &mut UnixStream,
    globals: &BTreeMap<String, u32>,
    interface: &str,
    id: u32,
    version: u32,
) {
    let mut data = words(&[globals[interface], interface.len() as u32 + 1]);
    data.extend_from_slice(interface.as_bytes());
    data.push(0);
    while data.len() % 4 != 0 {
        data.push(0);
    }
    data.extend(words(&[version, id]));
    send(peer, 2, 0, &data);
}
#[test]
fn output_batches_validate_before_mutation_and_advance_one_revision() {
    use crate::integrations::wayland::compositor::{
        OutputDescription, OutputMode, OutputState, OutputTransform,
    };
    let display = Display::new().unwrap();
    let mut native = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
    let make_output = |name: &str| {
        OutputState::new(
            OutputDescription {
                name: name.into(),
                description: name.into(),
                make: "Test".into(),
                model: "Display".into(),
                physical_millimeters: crate::foundation::SizeI::default(),
                logical_position: PointI::default(),
                scale: crate::platform::contracts::ScaleFactor::new(1.0).unwrap(),
                transform: OutputTransform::Normal,
                modes: vec![OutputMode {
                    size: crate::foundation::SizeI {
                        width: 800,
                        height: 600,
                    },
                    refresh_millihertz: 60000,
                    preferred: true,
                }],
            },
            0,
        )
        .unwrap()
    };
    native
        .add_output(&display, 1, make_output("TEST-1"))
        .unwrap();
    native
        .add_output(&display, 2, make_output("TEST-2"))
        .unwrap();
    let before = native.output_snapshot();
    let mut left = before.outputs()[&1].clone();
    let mut right = before.outputs()[&2].clone();
    left.description.logical_position.x = -800;
    right.description.logical_position.x = 800;
    let mut invalid = right.clone();
    invalid.current_mode = 99;
    assert!(
        native
            .update_outputs([(1, left.clone()), (2, invalid)])
            .is_err()
    );
    assert_eq!(native.output_snapshot(), before);
    assert!(
        native
            .update_outputs([(1, left.clone()), (1, left.clone())])
            .is_err()
    );
    assert_eq!(native.output_snapshot(), before);
    assert!(
        native
            .update_outputs([(2, right.clone()), (1, left.clone())])
            .unwrap()
    );
    let after = native.output_snapshot();
    assert_eq!(after.revision(), before.revision() + 1);
    assert_eq!(after.outputs()[&1], left);
    assert_eq!(after.outputs()[&2], right);
    assert_eq!(before.outputs()[&1].description.logical_position.x, 0);
    assert!(
        !native
            .update_outputs([(1, left.clone()), (2, right)])
            .unwrap()
    );
    assert_eq!(native.output_snapshot(), after);
    native.state.output_revision = u64::MAX;
    let exhausted = native.output_snapshot();
    left.description.logical_position.y = 100;
    assert!(native.update_output(1, left).is_err());
    assert_eq!(native.output_snapshot(), exhausted);
}

#[test]
fn xdg_output_reports_scaled_rotated_geometry_and_versioned_completion() {
    use crate::integrations::wayland::compositor::{
        OutputDescription, OutputMode, OutputState, OutputTransform,
    };
    for (version, output_version, density) in [
        (1, 1, 1),
        (2, 2, 1),
        (3, 4, 1),
        (3, 1, 1),
        (1, 1, 3),
        (2, 2, 3),
        (3, 4, 3),
        (3, 1, 3),
    ] {
        let mut display = Display::new().unwrap();
        let access = XwaylandAccess::configure_display(&mut display).unwrap();
        access.set_coordinate_scale(3);
        let (mut peer, socket) = UnixStream::pair().unwrap();
        let client = Rc::new(display.create_client(socket).unwrap());
        if density == 3 {
            access.set_client(client.clone(), 1).unwrap();
        }
        let mut native =
            NativeCompositor::new_with_xwayland(&display, ClientLimits::default(), access).unwrap();
        native
            .add_output(
                &display,
                1,
                OutputState::new(
                    OutputDescription {
                        name: "TEST-1".into(),
                        description: "Rotated test output".into(),
                        make: "Test".into(),
                        model: "Display".into(),
                        physical_millimeters: crate::foundation::SizeI::default(),
                        logical_position: PointI { x: -800, y: 40 },
                        scale: crate::platform::contracts::ScaleFactor::new(1.5).unwrap(),
                        transform: OutputTransform::Rotate90,
                        modes: vec![OutputMode {
                            size: crate::foundation::SizeI {
                                width: 1920,
                                height: 1080,
                            },
                            refresh_millihertz: 60000,
                            preferred: true,
                        }],
                    },
                    0,
                )
                .unwrap(),
            )
            .unwrap();
        let globals = registry(&display, &mut peer);
        bind_version(&mut peer, &globals, "wl_output", 4, output_version);
        bind_version(&mut peer, &globals, "zxdg_output_manager_v1", 5, version);
        send(&mut peer, 5, 1, &words(&[6, 4]));
        send(&mut peer, 1, 0, &words(&[7]));
        display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
        assert!(client.is_alive());
        let mut xdg_events = Vec::new();
        let mut last_event = None;
        loop {
            let mut header = [0; 8];
            peer.read_exact(&mut header).unwrap();
            let object = u32::from_ne_bytes(header[..4].try_into().unwrap());
            let word = u32::from_ne_bytes(header[4..].try_into().unwrap());
            let opcode = word as u16;
            let mut body = vec![0; (word >> 16) as usize - 8];
            peer.read_exact(&mut body).unwrap();
            if object == 7 {
                break;
            }
            last_event = Some((object, opcode));
            if object != 6 {
                continue;
            }
            xdg_events.push(opcode);
            if opcode < 2 {
                let pair = (
                    i32::from_ne_bytes(body[..4].try_into().unwrap()),
                    i32::from_ne_bytes(body[4..8].try_into().unwrap()),
                );
                assert_eq!(
                    pair,
                    if opcode == 0 {
                        (-800 * density, 40 * density)
                    } else {
                        (720 * density, 1280 * density)
                    }
                );
            }
        }
        let mut expected = vec![0, 1];
        if version >= 2 {
            expected.extend([3, 4]);
        }
        if version < 3 || output_version < 2 {
            expected.push(2);
        }
        assert_eq!(xdg_events, expected);
        assert_eq!(
            last_event,
            Some(if version >= 3 && output_version >= 2 {
                (4, 2)
            } else {
                (6, 2)
            })
        );
        let mut update = native.core().outputs[&1].clone();
        update.description.logical_position = PointI { x: -12, y: 64 };
        update.description.scale = crate::platform::contracts::ScaleFactor::new(2.0).unwrap();
        update.description.description = "Updated description".into();
        assert!(native.update_output(1, update.clone()).unwrap());
        assert!(!native.update_output(1, update.clone()).unwrap());
        let mut invalid = update.clone();
        invalid.description.name = "RENAMED-1".into();
        assert!(native.update_output(1, invalid).is_err());
        let mut partial = update.clone();
        partial.description.logical_position.x = 123;
        assert!(
            native
                .update_outputs([(1, partial), (99, update.clone())])
                .is_err()
        );
        assert_eq!(native.core().outputs[&1], update);
        send(&mut peer, 1, 0, &words(&[8]));
        display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
        let mut logical = Vec::new();
        let mut last = None;
        let mut output_done = 0;
        loop {
            let mut header = [0; 8];
            peer.read_exact(&mut header).unwrap();
            let object = u32::from_ne_bytes(header[..4].try_into().unwrap());
            let word = u32::from_ne_bytes(header[4..].try_into().unwrap());
            let opcode = word as u16;
            let mut body = vec![0; (word >> 16) as usize - 8];
            peer.read_exact(&mut body).unwrap();
            if object == 8 {
                break;
            }
            last = Some((object, opcode));
            if object == 4 {
                assert_ne!(opcode, 4, "core output name is immutable");
                if opcode == 2 {
                    output_done += 1;
                }
            }
            if object == 6 {
                logical.push(opcode);
                if opcode < 2 {
                    let pair = (
                        i32::from_ne_bytes(body[..4].try_into().unwrap()),
                        i32::from_ne_bytes(body[4..8].try_into().unwrap()),
                    );
                    assert_eq!(
                        pair,
                        if opcode == 0 {
                            (-12 * density, 64 * density)
                        } else {
                            (540 * density, 960 * density)
                        }
                    );
                }
            }
        }
        let mut expected = vec![0, 1];
        if version >= 3 {
            expected.push(4);
        }
        if version < 3 || output_version < 2 {
            expected.push(2);
        }
        assert_eq!(logical, expected); // v2 description and all names are immutable.
        assert_eq!(output_done, usize::from(output_version >= 2));
        assert_eq!(
            last,
            Some(if output_version >= 2 { (4, 2) } else { (6, 2) })
        );
    }
}

#[test]
fn xwayland_grab_is_private_focused_and_cancelled_without_automatic_reactivation() {
    use crate::integrations::wayland::compositor::{SeatCapabilities, SeatState, SurfaceCommit};
    let mut display = Display::new().unwrap();
    let access = XwaylandAccess::configure_display(&mut display).unwrap();
    let (mut peer, socket) = UnixStream::pair().unwrap();
    let client = Rc::new(display.create_client(socket).unwrap());
    access.set_client(client.clone(), 1).unwrap();
    let mut native =
        NativeCompositor::new_with_xwayland(&display, ClientLimits::default(), access).unwrap();
    native
        .add_seat(
            &display,
            1,
            SeatState::new(
                "test",
                SeatCapabilities {
                    pointer: false,
                    keyboard: true,
                    touch: false,
                },
            ),
        )
        .unwrap();
    let globals = registry(&display, &mut peer);
    let (mut public_peer, socket) = UnixStream::pair().unwrap();
    let public_client = display.create_client(socket).unwrap();
    let public_globals = registry(&display, &mut public_peer);
    assert!(!public_globals.contains_key("zwp_xwayland_keyboard_grab_manager_v1"));
    bind(
        &mut public_peer,
        &globals,
        "zwp_xwayland_keyboard_grab_manager_v1",
        4,
    );
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    assert!(!public_client.is_alive());
    assert!(client.is_alive());
    bind(&mut peer, &globals, "wl_compositor", 4);
    bind(&mut peer, &globals, "wl_seat", 5);
    bind(&mut peer, &globals, "xwayland_shell_v1", 6);
    bind(
        &mut peer,
        &globals,
        "zwp_xwayland_keyboard_grab_manager_v1",
        7,
    );
    send(&mut peer, 4, 0, &words(&[8]));
    send(&mut peer, 6, 1, &words(&[9, 8]));
    send(&mut peer, 9, 0, &words(&[50, 0]));
    send(&mut peer, 8, 6, &[]);
    send(&mut peer, 7, 1, &words(&[10, 8, 5]));
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    let client_id = native.state.clients[&client.identity().unwrap()];
    let surface = native.core().world.client_surfaces(client_id)[0];
    assert!(!native.shortcuts_inhibited(1));
    // Model buffer mapping; role/serial and grab requests use the real wire.
    let state = native.state.core.world.surface_mut(surface).unwrap();
    state
        .stage(SurfaceCommit {
            attachment: Some(Some(BufferAttachment {
                buffer: WaylandBufferId::from_raw(123).unwrap(),
                offset: PointI::default(),
            })),
            ..Default::default()
        })
        .unwrap();
    state.commit().unwrap();
    native.set_keyboard_focus(1, Some(surface), 1).unwrap();
    assert!(!native.shortcuts_inhibited(1)); // Initial unfocused request stays denied.
    send(&mut peer, 7, 1, &words(&[11, 8, 5]));
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    assert!(native.shortcuts_inhibited(1));
    native.set_keyboard_focus(1, None, 2).unwrap();
    native.set_keyboard_focus(1, Some(surface), 3).unwrap();
    assert!(!native.shortcuts_inhibited(1));
    send(&mut peer, 7, 1, &words(&[12, 8, 5]));
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    assert!(native.shortcuts_inhibited(1));
    native.release_shortcut_inhibition(1).unwrap();
    assert!(!native.shortcuts_inhibited(1));
    send(&mut peer, 7, 1, &words(&[13, 8, 5]));
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    assert!(!native.shortcuts_inhibited(1));
    send(&mut peer, 8, 0, &[]);
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    assert!(native.state.xwayland_keyboard_grabs.is_empty());
    assert!(native.state.revoked_shortcuts.is_empty());
    assert!(client.is_alive());
}

#[test]
fn shortcut_inhibitor_requires_mapped_focus_and_cannot_override_user_revocation() {
    use crate::integrations::wayland::compositor::{SeatCapabilities, SeatState, SurfaceCommit};
    let display = Display::new().unwrap();
    let (mut peer, socket) = UnixStream::pair().unwrap();
    let client = display.create_client(socket).unwrap();
    let mut native = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
    native
        .add_seat(
            &display,
            1,
            SeatState::new(
                "test",
                SeatCapabilities {
                    pointer: false,
                    keyboard: true,
                    touch: false,
                },
            ),
        )
        .unwrap();
    let globals = registry(&display, &mut peer);
    bind(&mut peer, &globals, "wl_compositor", 4);
    bind(&mut peer, &globals, "wl_seat", 5);
    bind(
        &mut peer,
        &globals,
        "zwp_keyboard_shortcuts_inhibit_manager_v1",
        6,
    );
    send(&mut peer, 4, 0, &words(&[7]));
    send(&mut peer, 6, 1, &words(&[8, 7, 5]));
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    let client_id = native.state.clients[&client.identity().unwrap()];
    let surface = native.core().world.client_surfaces(client_id)[0];
    native.set_keyboard_focus(1, Some(surface), 1).unwrap();
    assert!(!native.shortcuts_inhibited(1));
    // Model committed mapping without allocating/rendering a buffer. The
    // inhibitor requests and active/inactive notifications use actual wire I/O.
    let state = native.state.core.world.surface_mut(surface).unwrap();
    state
        .stage(SurfaceCommit {
            attachment: Some(Some(BufferAttachment {
                buffer: WaylandBufferId::from_raw(123).unwrap(),
                offset: PointI::default(),
            })),
            ..Default::default()
        })
        .unwrap();
    state.commit().unwrap();
    native.state.update_shortcut_inhibitors().unwrap();
    assert!(native.shortcuts_inhibited(1));
    native.set_keyboard_focus(1, None, 2).unwrap();
    assert!(!native.shortcuts_inhibited(1));
    native.set_keyboard_focus(1, Some(surface), 3).unwrap();
    assert!(native.shortcuts_inhibited(1));
    native.release_shortcut_inhibition(1).unwrap();
    assert!(!native.shortcuts_inhibited(1));
    send(&mut peer, 8, 0, &[]);
    send(&mut peer, 6, 1, &words(&[9, 7, 5]));
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    native.set_keyboard_focus(1, None, 4).unwrap();
    native.set_keyboard_focus(1, Some(surface), 5).unwrap();
    assert!(!native.shortcuts_inhibited(1));
    send(&mut peer, 1, 0, &words(&[10]));
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    let mut events = Vec::new();
    loop {
        let mut header = [0; 8];
        peer.read_exact(&mut header).unwrap();
        let object = u32::from_ne_bytes(header[..4].try_into().unwrap());
        let word = u32::from_ne_bytes(header[4..].try_into().unwrap());
        let mut body = vec![0; (word >> 16) as usize - 8];
        peer.read_exact(&mut body).unwrap();
        if object == 10 {
            break;
        }
        if object == 8 || object == 9 {
            events.push((object, word as u16));
        }
    }
    assert_eq!(events, [(8, 0), (8, 0), (8, 1)]);
    send(&mut peer, 6, 1, &words(&[11, 7, 5]));
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    assert!(!client.is_alive()); // duplicate seat/surface inhibitor is a protocol error
    assert!(!native.shortcuts_inhibited(1));
}

#[test]
fn keyboard_wire_delivery_ignores_duplicate_and_unmatched_edges() {
    use crate::integrations::wayland::compositor::{ButtonState, SeatCapabilities, SeatState};
    let display = Display::new().unwrap();
    let (mut peer, socket) = UnixStream::pair().unwrap();
    let client = display.create_client(socket).unwrap();
    let mut native = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
    native
        .add_seat(
            &display,
            1,
            SeatState::new(
                "test",
                SeatCapabilities {
                    pointer: false,
                    keyboard: true,
                    touch: false,
                },
            ),
        )
        .unwrap();
    let globals = registry(&display, &mut peer);
    bind(&mut peer, &globals, "wl_compositor", 4);
    bind(&mut peer, &globals, "wl_seat", 5);
    send(&mut peer, 5, 1, &words(&[6])); // get_keyboard
    send(&mut peer, 4, 0, &words(&[7]));
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    let client_id = native.state.clients[&client.identity().unwrap()];
    let surface = native.core().world.client_surfaces(client_id)[0];
    native.set_keyboard_focus(1, Some(surface), 1).unwrap();
    for (serial, state) in [
        (2, ButtonState::Released),
        (3, ButtonState::Pressed),
        (4, ButtonState::Pressed),
        (5, ButtonState::Released),
        (6, ButtonState::Released),
    ] {
        native.keyboard_key(1, serial, 20, state, serial).unwrap();
    }
    send(&mut peer, 1, 0, &words(&[8])); // flush barrier
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    let mut delivered = Vec::new();
    loop {
        let mut header = [0; 8];
        peer.read_exact(&mut header).unwrap();
        let object = u32::from_ne_bytes(header[..4].try_into().unwrap());
        let word = u32::from_ne_bytes(header[4..].try_into().unwrap());
        let mut body = vec![0; (word >> 16) as usize - 8];
        peer.read_exact(&mut body).unwrap();
        if object == 8 {
            break;
        }
        if object == 6 && word as u16 == 3 {
            delivered.push((
                u32::from_ne_bytes(body[..4].try_into().unwrap()),
                u32::from_ne_bytes(body[12..16].try_into().unwrap()),
            ));
        }
    }
    assert_eq!(delivered, [(3, 1), (5, 0)]);
    for serial in [2, 4, 6] {
        assert!(
            native
                .core()
                .serials
                .validate(
                    client_id,
                    serial,
                    &[crate::integrations::wayland::compositor::SerialKind::KeyboardKey],
                    Some(surface)
                )
                .is_err()
        );
    }
    assert!(native.core().seats[&1].pressed_keys().is_empty());
    native
        .keyboard_key(1, 20, 30, ButtonState::Pressed, 20)
        .unwrap();
    native.cancel_keyboard_input(1).unwrap();
    assert!(native.core().seats[&1].pressed_keys().is_empty());
    assert!(native.core().seats[&1].keyboard_focus.is_none());
    // A subsequent focus enter cannot inherit keys from across the boundary.
    native.set_keyboard_focus(1, Some(surface), 30).unwrap();
    native
        .keyboard_key(1, 31, 30, ButtonState::Released, 31)
        .unwrap();
    send(&mut peer, 1, 0, &words(&[9]));
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    let mut entered_keys = None;
    loop {
        let mut header = [0; 8];
        peer.read_exact(&mut header).unwrap();
        let object = u32::from_ne_bytes(header[..4].try_into().unwrap());
        let word = u32::from_ne_bytes(header[4..].try_into().unwrap());
        let mut body = vec![0; (word >> 16) as usize - 8];
        peer.read_exact(&mut body).unwrap();
        if object == 9 {
            break;
        }
        if object == 6 && word as u16 == 1 {
            entered_keys = Some(u32::from_ne_bytes(body[8..12].try_into().unwrap()));
        }
        if object == 6 && word as u16 == 3 {
            assert_ne!(u32::from_ne_bytes(body[..4].try_into().unwrap()), 31);
        }
    }
    assert_eq!(entered_keys, Some(0));
}

#[test]
fn data_source_action_requests_accept_empty_once_and_reject_unknown_bits() {
    for invalid_mask in [0, 8] {
        let display = Display::new().unwrap();
        let (mut peer, socket) = UnixStream::pair().unwrap();
        let client = display.create_client(socket).unwrap();
        let native = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
        let globals = registry(&display, &mut peer);
        bind_version(&mut peer, &globals, "wl_data_device_manager", 4, 3);
        send(&mut peer, 4, 0, &words(&[5]));
        send(&mut peer, 5, 2, &words(&[0])); // Explicit empty action set.
        display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
        assert!(client.is_alive());
        let resource = native
            .state
            .resource_for_kind(|kind| matches!(kind, ResourceKind::DataSource(_)))
            .unwrap()
            .unwrap();
        let ResourceKind::DataSource(object) = native.state.resource_kind(resource).unwrap() else {
            panic!()
        };
        let source = native.core().data_devices.source(object).unwrap();
        assert!(source.actions_set);
        assert_eq!(
            source.actions,
            crate::integrations::wayland::compositor::DataAction::NONE
        );
        assert!(!source.used);
        send(&mut peer, 5, 2, &words(&[invalid_mask]));
        display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
        assert!(!client.is_alive());
        let body = loop {
            let mut header = [0; 8];
            peer.read_exact(&mut header).unwrap();
            assert_eq!(u32::from_ne_bytes(header[..4].try_into().unwrap()), 1);
            let size_opcode = u32::from_ne_bytes(header[4..].try_into().unwrap());
            let mut body = vec![0; (size_opcode >> 16) as usize - 8];
            peer.read_exact(&mut body).unwrap();
            if size_opcode as u16 == 0 {
                break body;
            }
            assert_eq!(size_opcode as u16, 1); // Prior registry callback delete_id.
        };
        assert_eq!(u32::from_ne_bytes(body[..4].try_into().unwrap()), 5);
        assert_eq!(
            u32::from_ne_bytes(body[4..8].try_into().unwrap()),
            if invalid_mask == 0 { 1 } else { 0 }
        );
    }
}

#[test]
fn drag_action_versions_preserve_legacy_copy_and_modern_negotiation() {
    use crate::integrations::wayland::compositor::DataAction;
    assert_eq!(
        drag_source_actions(1, 3, DataAction::NONE),
        DataAction::COPY
    );
    assert_eq!(
        drag_source_actions(2, 3, DataAction::NONE),
        DataAction::COPY
    );
    assert_eq!(
        drag_source_actions(3, 1, DataAction::MOVE),
        DataAction::COPY
    );
    assert_eq!(
        drag_source_actions(3, 3, DataAction::MOVE),
        DataAction::MOVE
    );
    assert_eq!(
        drag_source_actions(3, 3, DataAction::NONE),
        DataAction::NONE
    );
}

#[test]
fn emergency_release_blocks_recapture_until_focus_leaves() {
    use crate::integrations::wayland::compositor::{SeatCapabilities, SeatState};
    let display = Display::new().unwrap();
    let (mut peer, socket) = UnixStream::pair().unwrap();
    let client = display.create_client(socket).unwrap();
    let mut native = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
    native
        .add_seat(
            &display,
            1,
            SeatState::new(
                "test",
                SeatCapabilities {
                    pointer: true,
                    keyboard: true,
                    touch: false,
                },
            ),
        )
        .unwrap();
    let globals = registry(&display, &mut peer);
    bind(&mut peer, &globals, "wl_compositor", 4);
    bind(&mut peer, &globals, "wl_seat", 5);
    bind(&mut peer, &globals, "zwp_pointer_constraints_v1", 6);
    send(&mut peer, 4, 0, &words(&[7]));
    send(&mut peer, 5, 0, &words(&[8]));
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    let client_id = native.state.clients[&client.identity().unwrap()];
    let surface = native.core().world.client_surfaces(client_id)[0];
    let point = crate::foundation::PointF::default();
    native
        .set_pointer_focus(1, Some(surface), point, 1)
        .unwrap();
    send(&mut peer, 6, 1, &words(&[9, 7, 8, 0, 2]));
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    assert!(native.pointer_constraint(1).is_some());
    native.release_pointer_capture(1).unwrap();
    assert!(native.pointer_constraint(1).is_none());
    native
        .set_pointer_focus(1, Some(surface), point, 2)
        .unwrap();
    assert!(native.pointer_constraint(1).is_none());
    // Replacing a persistent protocol object must not evade the release.
    send(&mut peer, 9, 0, &[]);
    send(&mut peer, 6, 1, &words(&[10, 7, 8, 0, 2]));
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    assert!(client.is_alive());
    assert!(native.pointer_constraint(1).is_none());
    native.set_pointer_focus(1, None, point, 3).unwrap();
    native
        .set_pointer_focus(1, Some(surface), point, 4)
        .unwrap();
    assert!(native.pointer_constraint(1).is_some());
    send(&mut peer, 10, 0, &[]);
    send(&mut peer, 6, 1, &words(&[11, 7, 8, 0, 1]));
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    assert!(native.pointer_constraint(1).is_some());
    native.release_pointer_capture(1).unwrap();
    native.set_pointer_focus(1, None, point, 5).unwrap();
    native
        .set_pointer_focus(1, Some(surface), point, 6)
        .unwrap();
    assert!(native.pointer_constraint(1).is_none()); // One-shot stays finished.
    assert!(native.release_pointer_capture(999).is_err());
    native.set_keyboard_focus(1, Some(surface), 100).unwrap();
    native
        .keyboard_key(
            1,
            0,
            20,
            crate::integrations::wayland::compositor::ButtonState::Pressed,
            101,
        )
        .unwrap();
    native
        .pointer_button(
            1,
            0,
            272,
            crate::integrations::wayland::compositor::ButtonState::Pressed,
            102,
        )
        .unwrap();
    bind(&mut peer, &globals, "wl_data_device_manager", 12);
    send(&mut peer, 12, 1, &words(&[13, 5]));
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    assert_eq!(native.core().seats[&1].pressed_keys(), &[20]);
    assert_eq!(native.core().seats[&1].pressed_buttons(), &[272]);
    native.suspend_seat_input(1).unwrap();
    send(&mut peer, 13, 0, &words(&[0, 7, 0, 102]));
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    assert!(client.is_alive());
    assert!(!native.drag_active(1));
    assert!(
        native
            .core()
            .serials
            .validate(
                client_id,
                102,
                &[crate::integrations::wayland::compositor::SerialKind::PointerButton],
                Some(surface)
            )
            .is_ok()
    );
    let seat = &native.core().seats[&1];
    assert!(seat.pressed_keys().is_empty());
    assert!(seat.pressed_buttons().is_empty());
    assert!(seat.keyboard_focus.is_none());
    assert!(seat.pointer_focus.is_none());
    assert!(native.pointer_constraint(1).is_none());
    native
        .keyboard_key(
            1,
            0,
            21,
            crate::integrations::wayland::compositor::ButtonState::Pressed,
            103,
        )
        .unwrap();
    native
        .pointer_button(
            1,
            0,
            273,
            crate::integrations::wayland::compositor::ButtonState::Pressed,
            104,
        )
        .unwrap();
    native.touch_down(1, surface, 0, 3, point, 105).unwrap();
    native.touch_motion(1, 0, 3, point).unwrap();
    native.touch_up(1, 0, 3, 106).unwrap();
    native.keyboard_modifiers(1, 107, 1, 2, 4, 1).unwrap();
    assert!(native.core().seats[&1].pressed_keys().is_empty());
    assert!(native.core().seats[&1].pressed_buttons().is_empty());
    assert!(native.state.touch_points.is_empty());
    native.set_keyboard_focus(1, Some(surface), 103).unwrap();
    native
        .set_pointer_focus(1, Some(surface), point, 104)
        .unwrap();
    assert!(native.core().seats[&1].keyboard_focus.is_none());
    assert!(native.core().seats[&1].pointer_focus.is_none());
    native.suspend_seat_input(1).unwrap();
    // Earlier fixture input used explicit serials; move the display allocator
    // beyond them before testing generated resume enter serials.
    for _ in 0..200 {
        display.next_serial();
    }
    assert!(native.resume_seat_input(1).unwrap());
    assert_eq!(
        native.core().seats[&1].keyboard_focus.unwrap().surface,
        surface
    );
    assert_eq!(
        native.core().seats[&1].pointer_focus.unwrap().surface,
        surface
    );
    // The old button serial is still known, but resume did not recreate a grab.
    send(&mut peer, 13, 0, &words(&[0, 7, 0, 102]));
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    assert!(client.is_alive());
    assert!(!native.drag_active(1));
    let drag_serial = display.next_serial();
    native
        .pointer_button(
            1,
            0,
            272,
            crate::integrations::wayland::compositor::ButtonState::Pressed,
            drag_serial,
        )
        .unwrap();
    // A fresh grab must not rehabilitate a serial from before suspension.
    send(&mut peer, 13, 0, &words(&[0, 7, 0, 102]));
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    assert!(client.is_alive());
    assert!(!native.drag_active(1));
    send(&mut peer, 13, 0, &words(&[0, 7, 0, drag_serial]));
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    assert!(client.is_alive());
    assert!(native.drag_active(1));
    native
        .pointer_button(
            1,
            0,
            272,
            crate::integrations::wayland::compositor::ButtonState::Released,
            display.next_serial(),
        )
        .unwrap();
    assert!(!native.state.pointer_press_serials.contains_key(&(1, 272)));
    native.suspend_seat_input(1).unwrap();
    assert!(!native.drag_active(1));
    native.set_keyboard_focus(1, None, 105).unwrap();
    assert!(native.resume_seat_input(1).unwrap());
    assert!(native.core().seats[&1].keyboard_focus.is_none());
    assert!(!native.resume_seat_input(1).unwrap());
    for destroy in [false, true] {
        native.suspend_seat_input(1).unwrap();
        native
            .set_keyboard_focus(1, Some(surface), display.next_serial())
            .unwrap();
        native
            .set_pointer_focus(1, Some(surface), point, display.next_serial())
            .unwrap();
        // Null-buffer commit withdraws the surface; the next iteration destroys it.
        send(&mut peer, 7, if destroy { 0 } else { 6 }, &[]);
        display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
        assert!(native.resume_seat_input(1).unwrap());
        assert!(native.core().seats[&1].keyboard_focus.is_none());
        assert!(native.core().seats[&1].pointer_focus.is_none());
    }
    assert!(native.suspend_seat_input(999).is_err());
    assert!(native.resume_seat_input(999).is_err());
}

#[test]
fn authenticated_shell_latches_serial_and_preserves_it_after_role_object_destroy() {
    let mut display = Display::new().unwrap();
    let access = XwaylandAccess::configure_display(&mut display).unwrap();
    let (mut peer, socket) = UnixStream::pair().unwrap();
    let client = Rc::new(display.create_client(socket).unwrap());
    access.set_client(client.clone(), 1).unwrap();
    let native =
        NativeCompositor::new_with_xwayland(&display, ClientLimits::default(), access).unwrap();
    let globals = registry(&display, &mut peer);
    let (mut stranger_peer, socket) = UnixStream::pair().unwrap();
    let stranger = display.create_client(socket).unwrap();
    let public_globals = registry(&display, &mut stranger_peer);
    assert!(public_globals.contains_key("wl_compositor"));
    assert!(!public_globals.contains_key("xwayland_shell_v1"));
    // This peer has identical Unix credentials, but no dedicated identity.
    bind(&mut stranger_peer, &globals, "xwayland_shell_v1", 4);
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    assert!(!stranger.is_alive());
    assert!(client.is_alive());
    bind(&mut peer, &globals, "wl_compositor", 4);
    bind(&mut peer, &globals, "xwayland_shell_v1", 5);
    send(&mut peer, 4, 0, &words(&[6]));
    send(&mut peer, 5, 1, &words(&[7, 6]));
    send(&mut peer, 7, 0, &words(&[11, 1]));
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    assert!(client.is_alive());
    let client_id = native.state.clients[&client.identity().unwrap()];
    let surface = native.core().world.client_surfaces(client_id)[0];
    assert_eq!(
        native
            .core()
            .world
            .surface(surface)
            .unwrap()
            .snapshot()
            .xwayland_serial,
        None
    );
    send(&mut peer, 6, 6, &[]);
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    assert_eq!(
        native
            .core()
            .world
            .surface(surface)
            .unwrap()
            .snapshot()
            .xwayland_serial,
        Some((1u64 << 32) | 11)
    );
    send(&mut peer, 7, 1, &[]);
    send(&mut peer, 6, 6, &[]);
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    assert_eq!(
        native
            .core()
            .world
            .surface(surface)
            .unwrap()
            .snapshot()
            .xwayland_serial,
        Some((1u64 << 32) | 11)
    );
    send(&mut peer, 5, 1, &words(&[8, 6]));
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    assert!(!client.is_alive()); // Destroyed role objects do not permit a new role.
}
#[test]
fn serial_order_is_global_and_restart_resets_only_after_old_client_dies() {
    let mut display = Display::new().unwrap();
    let access = XwaylandAccess::configure_display(&mut display).unwrap();
    let (mut peer, socket) = UnixStream::pair().unwrap();
    let client = Rc::new(display.create_client(socket).unwrap());
    access.set_client(client.clone(), 1).unwrap();
    let _native =
        NativeCompositor::new_with_xwayland(&display, ClientLimits::default(), access.clone())
            .unwrap();
    let globals = registry(&display, &mut peer);
    bind(&mut peer, &globals, "wl_compositor", 4);
    bind(&mut peer, &globals, "xwayland_shell_v1", 5);
    for (surface, role, serial) in [(6, 7, 2), (8, 9, 1)] {
        send(&mut peer, 4, 0, &words(&[surface]));
        send(&mut peer, 5, 1, &words(&[role, surface]));
        send(&mut peer, role, 0, &words(&[serial, 0]));
    }
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    assert!(!client.is_alive());
    assert_eq!(access.last_serial.get(), 2);
    let (mut replacement_peer, socket) = UnixStream::pair().unwrap();
    let replacement = Rc::new(display.create_client(socket).unwrap());
    assert!(access.set_client(replacement.clone(), 1).is_err());
    access.set_client(replacement.clone(), 2).unwrap();
    assert_eq!(access.last_serial.get(), 0);
    assert!(access.set_client(replacement, 3).is_err());
    let globals = registry(&display, &mut replacement_peer);
    assert!(globals.contains_key("xwayland_shell_v1"));
}

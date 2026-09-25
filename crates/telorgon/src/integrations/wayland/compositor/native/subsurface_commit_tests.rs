use super::wire_tests::{bind, registry, send, words};
use super::*;
use crate::integrations::wayland::compositor::{ShmBuffer, ShmFormat};
use std::{os::unix::net::UnixStream, time::Duration};

#[test]
fn subsurface_position_and_restack_requests_do_not_erase_each_other() {
    let display = Display::new().unwrap();
    let (mut peer, socket) = UnixStream::pair().unwrap();
    let connection = display.create_client(socket).unwrap();
    let native = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
    let globals = registry(&display, &mut peer);
    bind(&mut peer, &globals, "wl_compositor", 4);
    bind(&mut peer, &globals, "wl_subcompositor", 5);
    send(&mut peer, 4, 0, &words(&[6]));
    send(&mut peer, 4, 0, &words(&[7]));
    send(&mut peer, 5, 1, &words(&[8, 7, 6]));
    send(&mut peer, 8, 1, &words(&[26, 42]));
    send(&mut peer, 8, 2, &words(&[6]));
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    let client = native.state.clients[&connection.identity().unwrap()];
    let surfaces = native.core().world.client_surfaces(client);
    let child = surfaces
        .iter()
        .copied()
        .find(|id| native.core().subsurfaces.parent(*id).is_some())
        .unwrap();
    let parent = native.core().subsurfaces.parent(child).unwrap();
    let position = native.core().subsurfaces.position(child).unwrap();
    assert_eq!(position.offset, PointI { x: 26, y: 42 });
    assert_eq!(position.above, Some(parent));
    send(&mut peer, 8, 1, &words(&[13, 17]));
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    let position = native.core().subsurfaces.position(child).unwrap();
    assert_eq!(position.offset, PointI { x: 13, y: 17 });
    assert_eq!(position.above, Some(parent));
    send(&mut peer, 8, 3, &words(&[6]));
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    assert_eq!(
        native.core().subsurfaces.position(child).unwrap().offset,
        PointI { x: 13, y: 17 }
    );
}

#[test]
fn root_commit_publishes_nested_buffer_through_unchanged_parent() {
    let display = Display::new().unwrap();
    let (mut peer, socket) = UnixStream::pair().unwrap();
    let connection = display.create_client(socket).unwrap();
    let mut native = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
    let globals = registry(&display, &mut peer);
    bind(&mut peer, &globals, "wl_compositor", 4);
    for id in [5, 6, 7] {
        send(&mut peer, 4, 0, &words(&[id]));
    }
    display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
    let client = native.state.clients[&connection.identity().unwrap()];
    let surfaces = native.core().world.client_surfaces(client);
    let [root, child, grandchild] = surfaces.as_slice() else {
        panic!("three surfaces expected")
    };
    let (root, child, grandchild) = (*root, *child, *grandchild);
    let buffer = WaylandBufferId::from_raw(1).unwrap();
    native.state.core.subsurfaces.add(child, root).unwrap();
    native
        .state
        .core
        .subsurfaces
        .add(grandchild, child)
        .unwrap();
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
    for surface in [root, child, grandchild] {
        native
            .state
            .surface_mut(surface)
            .unwrap()
            .attach(attachment);
    }
    native.state.commit_surface(grandchild).unwrap();
    native.state.commit_surface(child).unwrap();
    native.state.commit_surface(root).unwrap();
    assert_eq!(
        native
            .state
            .core
            .world
            .surface(grandchild)
            .unwrap()
            .snapshot()
            .attachment,
        attachment
    );
    native.state.core.drain_actions().for_each(drop);

    let before = native
        .state
        .core
        .world
        .surface(grandchild)
        .unwrap()
        .snapshot()
        .revision;
    native
        .state
        .surface_mut(grandchild)
        .unwrap()
        .damage(RectI {
            x: 0,
            y: 0,
            width: 8,
            height: 8,
        })
        .unwrap();
    native.state.commit_surface(grandchild).unwrap();
    assert_eq!(
        native
            .state
            .core
            .world
            .surface(grandchild)
            .unwrap()
            .snapshot()
            .revision,
        before
    );
    // No child commit: a root update must still flush the grandchild.
    native.state.commit_surface(root).unwrap();
    assert_eq!(
        native
            .state
            .core
            .world
            .surface(grandchild)
            .unwrap()
            .snapshot()
            .revision,
        before + 1
    );
    // Damage/state alone must propagate, but cannot reacquire the released attachment.
    assert!(
        native.state.core.drain_actions().any(
            |action| matches!(action, CompositorAction::UpdateSurface(id) if id == grandchild)
        )
    );
    assert!(native.state.core.take_superseded_publications().is_empty());

    // A genuine buffer reuse still publishes through an unchanged intermediate parent.
    native.state.surface_mut(grandchild).unwrap().attach(attachment);
    native.state.commit_surface(grandchild).unwrap();
    native.state.commit_surface(root).unwrap();
    assert!(
        native.state.core.drain_actions().any(
            |action| matches!(action, CompositorAction::PublishSurface(id) if id == grandchild)
        )
    );
}

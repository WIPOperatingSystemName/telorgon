use super::*;
use crate::integrations::wayland::compositor::{
    BufferAttachment, ClientId, ShmBuffer, ShmFormat, WaylandBufferId,
};

#[test]
fn copy_completion_survives_state_only_commit_but_not_same_buffer_reattachment() {
    for reattach in [false, true] {
        let display = Display::new().unwrap();
        let mut wayland = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
        let client = ClientId::from_raw(1).unwrap();
        let surface = WaylandSurfaceId::from_raw(1).unwrap();
        let buffer = WaylandBufferId::from_raw(1).unwrap();
        let extent = SizeI {
            width: 4,
            height: 2,
        };
        let core = wayland.core_mut();
        core.connect_client(client).unwrap();
        core.world.create_surface(client, surface).unwrap();
        core.register_buffer(
            client,
            buffer,
            BufferDescriptor::Shm(ShmBuffer {
                offset: 0,
                size: extent,
                stride: 16,
                format: ShmFormat::Argb8888,
            }),
        )
        .unwrap();
        let state = core.world.surface_mut(surface).unwrap();
        state.assign_role(SurfaceRole::Cursor).unwrap();
        let attachment = Some(BufferAttachment {
            buffer,
            offset: PointI::default(),
        });
        state.attach(attachment);
        state.commit().unwrap();
        let snapshot = state.snapshot().clone();
        // The worker owns a copy from the older revision while a callback-only commit arrives.
        if reattach {
            state.attach(attachment);
        }
        state.commit().unwrap();
        let current_revision = state.snapshot().revision;
        let image = crate::graphics::render::ImageResource {
            image: ImageId(1),
            content_version: snapshot.revision,
            extent,
            color_encoding: crate::graphics::render::ImageColorEncoding::Srgb,
            alpha_mode: ImageAlphaMode::Premultiplied,
            pixel_format: ImagePixelFormat::Bgra8,
            pixels: Arc::from(vec![37; 32]),
        };
        let completion = ShmCopyCompletion {
            snapshot,
            buffer,
            result: Ok(PreparedClientImage::full_scaled(image, extent)),
        };
        let mut windows = BTreeMap::new();
        let applied = finish_shm_copy(
            &display,
            &mut wayland,
            &mut windows,
            &mut WindowIdentities::default(),
            &mut ConfigureScheduler::default(),
            &mut Vec::new(),
            &mut 0,
            full_rect(extent),
            false,
            &mut false,
            &mut BTreeMap::from([(buffer, 1)]),
            &mut BTreeMap::from([(surface, 1)]),
            completion,
        )
        .unwrap();
        assert_eq!(applied, !reattach);
        if !reattach {
            let presentation = &windows[&surface].presentation;
            assert_eq!(
                presentation.revision, current_revision,
                "new frame callbacks must be eligible for completion"
            );
            assert_eq!(presentation.pixels, vec![37; 32]);
            // No SHM file exists in this fixture: a state update must use the retained image.
            let state = wayland.core_mut().world.surface_mut(surface).unwrap();
            state.set_buffer_scale(2).unwrap();
            state.commit().unwrap();
            let latest = state.snapshot().clone();
            state_publication::apply_retained_surface_state(
                &display,
                &mut wayland,
                &mut windows,
                &mut WindowIdentities::default(),
                &mut ConfigureScheduler::default(),
                &mut Vec::new(),
                &mut 0,
                full_rect(extent),
                false,
                &mut false,
                &latest,
            )
            .unwrap();
            assert_eq!(windows[&surface].presentation.revision, latest.revision);
            assert_eq!(
                windows[&surface].presentation.size,
                SizeI {
                    width: 2,
                    height: 1
                }
            );
            assert_eq!(windows[&surface].presentation.image_size, extent);
            assert_eq!(windows[&surface].presentation.pixels, vec![37; 32]);
        } else {
            assert!(
                windows.is_empty(),
                "a genuinely newer use must not display an obsolete copy"
            );
        }
    }
}

#[test]
fn state_only_update_preserves_scaled_raster_and_pending_upload() {
    let mut window = super::maximize_preview_tests::test_window(
        SizeI {
            width: 4,
            height: 2,
        },
        PointI::default(),
    );
    window.presentation.image_size = SizeI {
        width: 8,
        height: 4,
    };
    window.presentation.pixels = vec![73; 128];
    window.presentation.pending_image_update =
        PendingClientImageUpdate::Full(Arc::from(vec![73; 128]));
    let image = PreparedClientImage::Unchanged {
        extent: SizeI {
            width: 2,
            height: 1,
        },
        raster_extent: window.presentation.image_size,
        pixel_format: window.presentation.pixel_format,
        alpha_mode: window.presentation.alpha_mode,
    };
    assert_eq!(
        image.raster_extent(),
        SizeI {
            width: 8,
            height: 4
        }
    );
    window.presentation.apply_image(12, image);
    assert_eq!(window.presentation.revision, 12);
    assert_eq!(
        window.presentation.size,
        SizeI {
            width: 2,
            height: 1
        }
    );
    assert_eq!(
        window.presentation.image_size,
        SizeI {
            width: 8,
            height: 4
        }
    );
    assert_eq!(window.presentation.pixels, vec![73; 128]);
    assert!(matches!(
        window.presentation.pending_image_update,
        PendingClientImageUpdate::Full(_)
    ));
}

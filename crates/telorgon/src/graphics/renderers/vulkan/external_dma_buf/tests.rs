use super::*;

fn valid_metadata() -> DmaBufMetadata {
    DmaBufMetadata {
        drm_fourcc: DRM_FORMAT_ABGR8888,
        drm_modifier: 0,
        format: vk::Format::R8G8B8A8_UNORM,
        extent: vk::Extent2D {
            width: 64,
            height: 32,
        },
        usage: vk::ImageUsageFlags::SAMPLED,
        plane_count: 1,
        memory_index: 0,
        offset: 0,
        size: 8_192,
        row_pitch: 256,
        allocation_size: 8_192,
        content_version: 1,
        lease_generation: 1,
        color_encoding: ImageColorEncoding::Linear,
        alpha_mode: ImageAlphaMode::Premultiplied,
        damage_count: 1,
    }
}

#[test]
fn rgba_drm_pairs_are_explicit_and_alpha_sensitive() {
    let damage = [RectI {
        x: 0,
        y: 0,
        width: 64,
        height: 32,
    }];
    validate_metadata(valid_metadata(), &damage).unwrap();
    let mut wrong_alpha = valid_metadata();
    wrong_alpha.alpha_mode = ImageAlphaMode::Opaque;
    assert_eq!(
        validate_metadata(wrong_alpha, &damage).unwrap_err().kind(),
        RenderErrorKind::Unsupported
    );
}

#[test]
fn multi_plane_and_out_of_bounds_layouts_are_rejected_before_fd_import() {
    let mut metadata = valid_metadata();
    metadata.plane_count = 2;
    assert_eq!(
        validate_metadata(metadata, &[]).unwrap_err().kind(),
        RenderErrorKind::Unsupported
    );
    let mut metadata = valid_metadata();
    metadata.allocation_size -= 1;
    assert_eq!(
        validate_metadata(metadata, &[]).unwrap_err().kind(),
        RenderErrorKind::HostContract
    );
}

#[test]
fn invalid_modifiers_plane_indices_and_damage_overflow_are_rejected() {
    let damage = [RectI {
        x: 0,
        y: 0,
        width: 64,
        height: 32,
    }];
    let mut metadata = valid_metadata();
    metadata.drm_modifier = DRM_FORMAT_MOD_INVALID;
    assert_eq!(
        validate_metadata(metadata, &damage).unwrap_err().kind(),
        RenderErrorKind::HostContract
    );

    let mut metadata = valid_metadata();
    metadata.memory_index = 1;
    assert_eq!(
        validate_metadata(metadata, &damage).unwrap_err().kind(),
        RenderErrorKind::Unsupported
    );

    let overflowing_damage = [RectI {
        x: 1,
        y: 0,
        width: i32::MAX,
        height: 1,
    }];
    assert_eq!(
        validate_metadata(valid_metadata(), &overflowing_damage)
            .unwrap_err()
            .kind(),
        RenderErrorKind::HostContract
    );
}

#[cfg(target_os = "linux")]
#[test]
fn owning_dma_buf_plane_closes_an_unimported_fd_on_drop() {
    use std::io::Read;
    use std::os::unix::net::UnixStream;

    let (mut peer, file) = UnixStream::pair().unwrap();
    peer.set_nonblocking(true).unwrap();
    let plane = linux::VulkanDmaBufPlane {
        memory: file.into(),
        memory_index: 0,
        offset: 0,
        size: 4,
        row_pitch: 4,
        allocation_size: 4,
    };
    drop(plane);
    // An unrelated parallel test may immediately reuse a closed FD number.
    // EOF on its retained peer proves this owned endpoint was closed.
    assert_eq!(peer.read(&mut [0u8; 1]).unwrap(), 0);
}

#[test]
fn release_export_is_one_shot_but_a_failed_attempt_can_retry() {
    let state = ReleaseExportState::new();
    assert!(!state.is_resolved());
    state.begin().unwrap();
    assert_eq!(
        state.begin().unwrap_err().kind(),
        RenderErrorKind::HostContract
    );
    state.fail();
    state.begin().unwrap();
    state.complete();
    assert!(state.is_resolved());
    assert_eq!(
        state.begin().unwrap_err().kind(),
        RenderErrorKind::HostContract
    );
}

#[cfg(target_os = "linux")]
#[test]
fn scanout_layout_discards_only_before_the_first_submission() {
    assert_eq!(
        linux::scanout_initial_layout(false),
        vk::ImageLayout::UNDEFINED
    );
    assert_eq!(
        linux::scanout_initial_layout(true),
        vk::ImageLayout::GENERAL
    );
}

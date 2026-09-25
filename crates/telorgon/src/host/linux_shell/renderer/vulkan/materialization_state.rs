use crate::graphics::bridges::wayland::dma_buf_image_id;
use crate::graphics::render::{ImageResourceDelta, RenderSceneDelta};

pub(super) fn frame_includes_buffer_commit(
    surface_revisions: &[(u32, u64)],
    surface: u32,
    attachment_revision: u64,
) -> bool {
    // The queue holds only the latest buffer use; newer buffers replace/cancel it.
    // State-only commits advance presentation without changing that buffer's ownership.
    surface_revisions
        .iter()
        .any(|&(candidate, revision)| candidate == surface && revision >= attachment_revision)
}

pub(super) fn replaces_materialized_image(deltas: &[RenderSceneDelta]) -> bool {
    // Callback/geometry updates still draw the retained GPU image. Only a switch to
    // uploaded pixels replaces its binding; absence of a fresh DMA-BUF is not a switch.
    deltas.iter().any(|delta| {
        delta.image_resources.iter().any(|resource| {
        matches!(resource, ImageResourceDelta::Write(update) if update.image != dma_buf_image_id())
    })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::{RectI, SizeI};
    use crate::graphics::render::{
        ImageAlphaMode, ImageColorEncoding, ImagePixelFormat, ImageResourceUpdate,
    };
    use crate::graphics::renderers::vulkan::VulkanScene;
    use crate::host::linux_shell::scene::{
        ShellComposition, ShellImageUpdate, ShellLayer, ShellLayerKey, ShellSceneKey,
    };

    #[test]
    fn state_only_frames_materialize_then_retain_the_original_buffer_image() {
        let size = SizeI {
            width: 4,
            height: 4,
        };
        let mut composition = ShellComposition::new(size);
        let mut retained = VulkanScene::default();
        // Reproduce the log: buffer commit 9, state-only commits 10/11 before its first draw.
        // Later callbacks must retain the binding, while a real SHM replacement removes it.
        for (revision, update) in [
            (
                11,
                ShellImageUpdate::External {
                    image: dma_buf_image_id(),
                    content_version: 11,
                    damage: None,
                },
            ),
            (12, ShellImageUpdate::Reused),
            (13, ShellImageUpdate::Full(vec![255; 64].into())),
        ] {
            let frame = composition
                .synchronize(
                    size,
                    vec![ShellLayer::image(
                        ShellLayerKey::Surface(6),
                        ShellSceneKey::Surface(6),
                        revision,
                        update,
                        size,
                        RectI {
                            x: 0,
                            y: 0,
                            width: 4,
                            height: 4,
                        },
                        None,
                        ImageAlphaMode::Opaque,
                        ImagePixelFormat::Rgba8,
                        true,
                    )],
                )
                .unwrap();
            assert_eq!(frame.surface_revisions, [(6, revision)]);
            if revision == 11 {
                assert!(frame_includes_buffer_commit(&frame.surface_revisions, 6, 9));
            }
            for update in frame.updates {
                let replace = replaces_materialized_image(&update.deltas);
                assert_eq!(replace, revision == 13);
                if replace {
                    assert!(retained.remove_materialized_image(dma_buf_image_id()));
                }
                for mut delta in update.deltas {
                    if revision == 11 {
                        // CPU metadata substitutes for the materialized GPU binding, allowing
                        // the actual Vulkan scene validator to run without a device/session.
                        delta.image_resources.push(ImageResourceDelta::Write(
                            ImageResourceUpdate {
                                image: dma_buf_image_id(),
                                content_version: 9,
                                extent: size,
                                rect: RectI {
                                    x: 0,
                                    y: 0,
                                    width: 4,
                                    height: 4,
                                },
                                row_bytes: 16,
                                color_encoding: ImageColorEncoding::Srgb,
                                alpha_mode: ImageAlphaMode::Opaque,
                                pixel_format: ImagePixelFormat::Rgba8,
                                pixels: vec![255; 64].into(),
                            },
                        ));
                    }
                    retained.apply_delta_checked(&delta).unwrap();
                }
            }
            assert_eq!(retained.images.len(), 1);
        }
    }

    #[test]
    fn newer_buffer_is_not_materialized_into_an_older_or_absent_surface_frame() {
        assert!(!frame_includes_buffer_commit(&[(6, 8)], 6, 9));
        assert!(!frame_includes_buffer_commit(&[(7, 11)], 6, 9));
        assert!(!frame_includes_buffer_commit(&[], 6, 9));
        assert!(frame_includes_buffer_commit(&[(6, 9)], 6, 9));
    }
}

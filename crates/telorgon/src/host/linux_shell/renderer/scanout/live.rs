//! Keep the previous scanout pool alive until a preview is confirmed or reverted.
use super::*;

pub(in crate::host::linux_shell) struct LivePool<'a> {
    // Imports must be destroyed before the exported buffers.
    pub targets: Vec<VulkanDmaBufScanoutTarget>,
    pub framebuffers: Vec<KmsFramebuffer<'a>>,
    pub buffers: Vec<ScanoutBuffer<'a>>,
}

pub(in crate::host::linux_shell) fn prepare_live_pool<'a>(
    kms: &'a KmsDevice,
    gbm: Option<&'a GbmDevice<'a>>,
    renderer: &ShellRenderer,
    existing: &ScanoutBuffer<'a>,
    extent: SizeI,
) -> AppResult<LivePool<'a>> {
    let format = existing.format();
    let allocation = match existing {
        ScanoutBuffer::Dumb(_) => Allocation::Dumb,
        ScanoutBuffer::Gbm { implicit: true, .. } => Allocation::LegacyCpu {
            explicit: false,
            implicit: true,
        },
        _ => Allocation::Explicit(
            vec![format.modifier],
            ffi::GBM_BO_USE_SCANOUT
                | if matches!(renderer, ShellRenderer::Vulkan(_)) {
                    ffi::GBM_BO_USE_RENDERING
                } else {
                    ffi::GBM_BO_USE_LINEAR
                },
        ),
    };
    let mut pool = LivePool {
        targets: Vec::new(),
        framebuffers: Vec::new(),
        buffers: Vec::new(),
    };
    for _ in 0..SLOTS {
        let mut buffer = allocation
            .allocate(kms, gbm, extent, format.fourcc, None)
            .map_err(|e| AppError::new(e.message))?;
        if let ShellRenderer::Vulkan(vulkan) = renderer {
            pool.targets.push(
                vulkan.import_live_target(
                    buffer
                        .gbm()
                        .ok_or_else(|| AppError::new("Vulkan requires GBM"))?,
                )?,
            );
        } else {
            let pixels = vec![0u8; extent.width as usize * extent.height as usize * 4];
            buffer
                .write_rgba8_region(
                    &pixels,
                    crate::foundation::RectI {
                        x: 0,
                        y: 0,
                        width: extent.width,
                        height: extent.height,
                    },
                )
                .map_err(|e| AppError::new(e.to_string()))?;
        }
        pool.framebuffers.push(
            buffer
                .framebuffer(kms)
                .map_err(|e| AppError::new(e.to_string()))?,
        );
        pool.buffers.push(buffer);
    }
    Ok(pool)
}

/// Caller has drained GPU submissions and KMS flips and completed a blocking modeset to
/// pool.framebuffers[0]. Old buffers are returned rather than freed while they may be restored.
pub(in crate::host::linux_shell) fn exchange_pool<'a>(
    mut pool: LivePool<'a>,
    renderer: &mut ShellRenderer,
    framebuffers: &mut Vec<KmsFramebuffer<'a>>,
    buffers: &mut Vec<ScanoutBuffer<'a>>,
) -> LivePool<'a> {
    match renderer {
        ShellRenderer::Vulkan(vulkan) => vulkan.exchange_live_targets(&mut pool.targets),
        ShellRenderer::Software(software) => software.invalidate_targets(),
    }
    std::mem::swap(framebuffers, &mut pool.framebuffers);
    std::mem::swap(buffers, &mut pool.buffers);
    pool
}

//! Bounded startup negotiation. Nothing here runs in the frame loop.
use super::{ShellRenderer, software::SoftwareShellRenderer, vulkan::PreparedVulkan};
use crate::application_host::{AppError, AppResult, Renderer};
use crate::core::SizeI;
use crate::presenter_vulkan_kms::*;
use crate::render::{RenderError, RenderErrorKind};
use crate::renderer_vulkan::VulkanDmaBufScanoutTarget;

const SLOTS: usize = 3;
const MAX_ATTEMPTS: usize = 64;

// Field order is deliberate: imported targets and framebuffers die before BOs.
pub(in crate::application_host::shell_wayland) struct PreparedScanout<'a> {
    pub renderer: ShellRenderer,
    pub framebuffers: Vec<KmsFramebuffer<'a>>,
    pub buffers: Vec<ScanoutBuffer<'a>>,
    pub crtc: KmsCrtcId,
    pub crtc_index: usize,
    pub plane: KmsPlaneId,
    pub connector_properties: KmsObjectProperties,
    pub crtc_properties: KmsObjectProperties,
    pub plane_properties: KmsObjectProperties,
}

struct TrialSlot<'a> {
    target: Option<VulkanDmaBufScanoutTarget>,
    framebuffer: KmsFramebuffer<'a>,
    buffer: ScanoutBuffer<'a>,
}

#[derive(Debug)]
struct Failure {
    message: String,
    retryable: bool,
}
impl From<KmsError> for Failure {
    fn from(error: KmsError) -> Self {
        Self {
            message: error.to_string(),
            retryable: error.retryable(),
        }
    }
}
impl From<RenderError> for Failure {
    fn from(error: RenderError) -> Self {
        Self {
            message: format!("{error} (Vulkan={:?})", error.backend_code()),
            retryable: matches!(
                error.kind(),
                RenderErrorKind::Unsupported | RenderErrorKind::InvalidTarget
            ) || [
                ash::vk::Result::ERROR_FORMAT_NOT_SUPPORTED,
                ash::vk::Result::ERROR_INVALID_EXTERNAL_HANDLE,
                ash::vk::Result::ERROR_INVALID_DRM_FORMAT_MODIFIER_PLANE_LAYOUT_EXT,
            ]
            .iter()
            .any(|result| error.backend_code() == Some(i64::from(result.as_raw()))),
        }
    }
}
impl Failure {
    fn incompatible(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: true,
        }
    }
}

fn policies(policy: Renderer) -> &'static [Renderer] {
    match policy {
        Renderer::Vulkan => &[Renderer::Vulkan],
        Renderer::Software => &[Renderer::Software],
        Renderer::Auto => &[Renderer::Vulkan, Renderer::Software],
    }
}

/// Incremental construction drops the entire partial pool before a caller retries.
fn build_pool<T, E>(mut slot: impl FnMut(usize, Option<&T>) -> Result<T, E>) -> Result<Vec<T>, E> {
    let mut pool = Vec::with_capacity(SLOTS);
    for index in 0..SLOTS {
        pool.push(slot(index, pool.first())?);
    }
    Ok(pool)
}

struct Attempts {
    remaining: usize,
    failures: Vec<String>,
}
impl Attempts {
    fn new() -> Self {
        Self {
            remaining: MAX_ATTEMPTS,
            failures: Vec::new(),
        }
    }
    fn record(&mut self, label: &str, error: Failure) -> AppResult<()> {
        let message = format!("{label}: {}", error.message);
        if !error.retryable {
            return Err(AppError::new(message));
        }
        if self.failures.len() < MAX_ATTEMPTS * 2 + 4 {
            self.failures.push(message);
        }
        Ok(())
    }
    fn admit(&mut self) -> bool {
        if self.remaining == 0 {
            return false;
        }
        self.remaining -= 1;
        true
    }
}

#[allow(clippy::too_many_arguments)]
pub(in crate::application_host::shell_wayland) fn prepare<'a>(
    kms: &'a KmsDevice,
    gbm: Option<&'a GbmDevice<'a>>,
    topology: &KmsTopology,
    connector: &KmsConnector,
    mode_blob: u32,
    extent: SizeI,
    policy: Renderer,
) -> AppResult<PreparedScanout<'a>> {
    let connector_properties =
        KmsTopology::object_properties(kms, connector.id.get(), KmsPropertyObject::Connector)
            .map_err(|e| AppError::new(e.to_string()))?;
    let explicit_kms = match kms.capability(ffi::DRM_CAP_ADDFB2_MODIFIERS) {
        Ok(value) => value != 0,
        Err(error) if error.retryable() => false,
        Err(error) => return Err(AppError::new(error.to_string())),
    };
    let mut attempts = Attempts::new();
    for &renderer in policies(policy) {
        // Give each policy its own finite budget so Auto always has a CPU opportunity.
        attempts.remaining = MAX_ATTEMPTS;
        let vulkan = if renderer == Renderer::Vulkan {
            if gbm.is_none() {
                attempts.record("Vulkan", Failure::incompatible("GBM device unavailable"))?;
                continue;
            }
            match PreparedVulkan::new(kms.fd(), extent, SLOTS) {
                Ok(vulkan) => Some(vulkan),
                Err(error) => {
                    attempts.record("Vulkan device preparation", error.into())?;
                    continue;
                }
            }
        } else {
            if policy == Renderer::Auto && !attempts.failures.is_empty() {
                eprintln!(
                    "telorgon-kms: Vulkan startup unavailable; preparing new software buffers: {}",
                    attempts.failures.join("; ")
                );
            }
            None
        };
        let failures_before_planes = attempts.failures.len();
        let mut gpu_modifiers = std::collections::BTreeMap::new();
        if let Some(vulkan) = &vulkan {
            for fourcc in [DRM_FORMAT_XRGB8888, DRM_FORMAT_ARGB8888] {
                match vulkan.modifiers(fourcc, extent) {
                    Ok(modifiers) => {
                        gpu_modifiers.insert(fourcc, modifiers);
                    }
                    Err(error) => attempts.record("Vulkan capabilities", error.into())?,
                }
            }
        }
        for (crtc_index, &crtc_raw) in topology.crtcs.iter().enumerate() {
            let mask = 1_u32.checked_shl(crtc_index as u32).unwrap_or(0);
            if connector.possible_crtcs_mask & mask == 0 {
                continue;
            }
            let Some(crtc) = KmsCrtcId::from_raw(crtc_raw) else {
                continue;
            };
            let crtc_properties =
                KmsTopology::object_properties(kms, crtc_raw, KmsPropertyObject::Crtc)
                    .map_err(|e| AppError::new(e.to_string()))?;
            for plane in topology
                .planes
                .iter()
                .filter(|p| p.possible_crtcs_mask & mask != 0)
            {
                let plane_properties =
                    KmsTopology::object_properties(kms, plane.id.get(), KmsPropertyObject::Plane)
                        .map_err(|e| AppError::new(e.to_string()))?;
                if plane_properties
                    .named("type")
                    .is_none_or(|p| p.value != DRM_PLANE_TYPE_PRIMARY)
                {
                    continue;
                }
                let formats = plane_formats(kms, &plane_properties)
                    .map_err(|e| AppError::new(e.to_string()))?;
                for fourcc in [DRM_FORMAT_XRGB8888, DRM_FORMAT_ARGB8888] {
                    if !plane.formats.contains(&fourcc) {
                        continue;
                    }
                    let candidates = allocation_plan(
                        vulkan
                            .as_ref()
                            .map(|_| gpu_modifiers.get(&fourcc).map(Vec::as_slice).unwrap_or(&[])),
                        explicit_kms,
                        formats.as_deref(),
                        fourcc,
                    );
                    for mut allocation in candidates {
                        loop {
                            if !attempts.admit() {
                                break;
                            }
                            let mut selected = None;
                            let label = format!(
                                "{renderer:?} CRTC={crtc_raw} plane={} {}x{} fourcc={fourcc:#x} {}",
                                plane.id.get(),
                                extent.width,
                                extent.height,
                                allocation.description()
                            );
                            let trial = build_pool(
                                |_,
                                 first: Option<&TrialSlot<'a>>|
                                 -> Result<TrialSlot<'a>, Failure> {
                                    let mut buffer = allocation.allocate(
                                        kms,
                                        gbm,
                                        extent,
                                        fourcc,
                                        first.map(|s| s.buffer.format().modifier),
                                    )?;
                                    selected = Some(buffer.format().modifier);
                                    let target = if let Some(vulkan) = &vulkan {
                                        Some(vulkan.import(buffer.gbm().ok_or_else(|| {
                                            Failure::incompatible("Vulkan needs a GBM buffer")
                                        })?)?)
                                    } else {
                                        buffer.test_cpu_write()?;
                                        None
                                    };
                                    let framebuffer = buffer.framebuffer(kms)?;
                                    kms.primary_modeset_request(
                                        connector.id,
                                        &connector_properties,
                                        crtc,
                                        &crtc_properties,
                                        plane.id,
                                        &plane_properties,
                                        mode_blob,
                                        framebuffer.id(),
                                        extent.width as u32,
                                        extent.height as u32,
                                    )?
                                    .test(true)?;
                                    Ok(TrialSlot {
                                        target,
                                        framebuffer,
                                        buffer,
                                    })
                                },
                            );
                            match trial {
                                Ok(pool) => {
                                    // Declare owners in reverse retirement order for failure here.
                                    let mut buffers = Vec::new();
                                    let mut framebuffers = Vec::new();
                                    let mut targets = Vec::new();
                                    for slot in pool {
                                        buffers.push(slot.buffer);
                                        framebuffers.push(slot.framebuffer);
                                        if let Some(target) = slot.target {
                                            targets.push(target);
                                        }
                                    }
                                    let selected_renderer = match &vulkan {
                                        Some(vulkan) => {
                                            ShellRenderer::Vulkan(vulkan.finish(targets)?)
                                        }
                                        None => ShellRenderer::Software(
                                            SoftwareShellRenderer::new(buffers.len()),
                                        ),
                                    };
                                    eprintln!(
                                        "telorgon-kms: selected {renderer:?}, GBM={:?}, CRTC={crtc_raw}, plane={}, {}x{}, format={:?}, slots={}",
                                        gbm.and_then(GbmDevice::backend_name),
                                        plane.id.get(),
                                        extent.width,
                                        extent.height,
                                        buffers[0].format(),
                                        buffers.len()
                                    );
                                    return Ok(PreparedScanout {
                                        renderer: selected_renderer,
                                        framebuffers,
                                        buffers,
                                        crtc,
                                        crtc_index,
                                        plane: plane.id,
                                        connector_properties,
                                        crtc_properties,
                                        plane_properties,
                                    });
                                }
                                Err(error) => {
                                    attempts.record(&label, error)?;
                                    // A selected but unimportable modifier must not poison the
                                    // remaining capability intersection. Retry after full cleanup.
                                    if !allocation.reject(selected) {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        if attempts.failures.len() == failures_before_planes {
            attempts.record(&format!("{renderer:?}"), Failure::incompatible(
                "no common primary-plane/renderer format and layout (Vulkan requires explicit KMS modifiers)"))?;
        }
    }
    Err(AppError::new(format!(
        "no compatible {policy:?} scanout path at {}x{} (at most {MAX_ATTEMPTS} allocation attempts per renderer): {}",
        extent.width,
        extent.height,
        if attempts.failures.is_empty() {
            "no common primary-plane/renderer format and layout".into()
        } else {
            attempts.failures.join("; ")
        }
    )))
}

fn allocation_plan(
    gpu_modifiers: Option<&[u64]>,
    explicit_kms: bool,
    formats: Option<&[ScanoutFormat]>,
    fourcc: u32,
) -> Vec<Allocation> {
    let compatible = |modifier| {
        formats.is_some_and(|formats| formats.contains(&ScanoutFormat { fourcc, modifier }))
    };
    if let Some(gpu_modifiers) = gpu_modifiers {
        if !explicit_kms {
            return Vec::new();
        }
        let modifiers = gpu_modifiers
            .iter()
            .copied()
            .filter(|m| compatible(*m))
            .collect::<Vec<_>>();
        if modifiers.is_empty() {
            return Vec::new();
        }
        vec![
            Allocation::Explicit(
                modifiers.clone(),
                ffi::GBM_BO_USE_SCANOUT | ffi::GBM_BO_USE_RENDERING,
            ),
            Allocation::LegacyGpu(modifiers),
        ]
    } else {
        let mut candidates = Vec::new();
        if explicit_kms && compatible(DRM_FORMAT_MOD_LINEAR) {
            candidates.push(Allocation::Explicit(
                vec![DRM_FORMAT_MOD_LINEAR],
                ffi::GBM_BO_USE_SCANOUT,
            ));
        }
        candidates.push(Allocation::LegacyCpu {
            explicit: explicit_kms && compatible(DRM_FORMAT_MOD_LINEAR),
            implicit: formats.is_none() || compatible(DRM_FORMAT_MOD_INVALID),
        });
        candidates.push(Allocation::Dumb);
        candidates
    }
}

#[derive(Debug)]
enum Allocation {
    Explicit(Vec<u64>, u32),
    LegacyGpu(Vec<u64>),
    LegacyCpu { explicit: bool, implicit: bool },
    Dumb,
}
impl Allocation {
    fn description(&self) -> String {
        match self {
            Self::Explicit(modifiers, flags) => format!(
                "explicit usage={flags:#x}, modifiers={:x?} ({} total)",
                &modifiers[..modifiers.len().min(8)],
                modifiers.len()
            ),
            Self::LegacyGpu(_) => "legacy GPU allocation with verified explicit layout".into(),
            Self::LegacyCpu { .. } => "legacy CPU linear allocation".into(),
            Self::Dumb => "DRM CPU dumb allocation".into(),
        }
    }
    fn reject(&mut self, selected: Option<u64>) -> bool {
        let Self::Explicit(modifiers, _) = self else {
            return false;
        };
        let Some(selected) = selected else {
            return false;
        };
        modifiers.retain(|m| *m != selected);
        !modifiers.is_empty()
    }
    fn allocate<'a>(
        &self,
        kms: &'a KmsDevice,
        gbm: Option<&'a GbmDevice<'a>>,
        size: SizeI,
        fourcc: u32,
        selected: Option<u64>,
    ) -> Result<ScanoutBuffer<'a>, Failure> {
        if matches!(self, Self::Dumb) {
            return Ok(ScanoutBuffer::Dumb(DumbBuffer::new(kms, size, fourcc)?));
        }
        let gbm = gbm.ok_or_else(|| Failure::incompatible("GBM device unavailable"))?;
        let (buffer, implicit) = match self {
            Self::Explicit(modifiers, usage) => {
                let chosen = selected.map(|m| vec![m]);
                let candidates = chosen.as_deref().unwrap_or(modifiers);
                (
                    gbm.allocate_with_usage(
                        size,
                        ScanoutFormat {
                            fourcc,
                            modifier: candidates[0],
                        },
                        candidates,
                        *usage,
                    )?,
                    false,
                )
            }
            Self::LegacyGpu(modifiers) => {
                let buffer = gbm.allocate_legacy(
                    size,
                    fourcc,
                    ffi::GBM_BO_USE_SCANOUT | ffi::GBM_BO_USE_RENDERING,
                )?;
                let actual = buffer.format().modifier;
                if !modifiers.contains(&actual) || selected.is_some_and(|m| m != actual) {
                    return Err(Failure::incompatible(format!(
                        "legacy GPU allocation selected unnegotiated modifier {actual:#x}"
                    )));
                }
                (buffer, false)
            }
            Self::LegacyCpu { explicit, implicit } => {
                let buffer = gbm.allocate_legacy(
                    size,
                    fourcc,
                    ffi::GBM_BO_USE_SCANOUT | ffi::GBM_BO_USE_LINEAR,
                )?;
                let actual = buffer.format().modifier;
                if !matches!(actual, DRM_FORMAT_MOD_LINEAR | DRM_FORMAT_MOD_INVALID) {
                    return Err(Failure::incompatible(format!(
                        "legacy CPU allocation returned non-linear modifier {actual:#x}"
                    )));
                }
                let use_implicit = !*explicit || actual == DRM_FORMAT_MOD_INVALID;
                if use_implicit && !*implicit {
                    return Err(Failure::incompatible(
                        "plane does not advertise implicit layouts",
                    ));
                }
                (buffer, use_implicit)
            }
            Self::Dumb => unreachable!(),
        };
        if buffer.size() != size || buffer.format().fourcc != fourcc || buffer.plane_count()? != 1 {
            return Err(Failure::incompatible(
                "scanout requires matching extent/format and one memory plane",
            ));
        }
        Ok(ScanoutBuffer::Gbm { buffer, implicit })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nvidia_render_targets_use_tiling_while_cpu_buffers_do_not_request_rendering() {
        let tiled = 0x0300000000606014;
        let formats = [
            ScanoutFormat {
                fourcc: DRM_FORMAT_XRGB8888,
                modifier: 0,
            },
            ScanoutFormat {
                fourcc: DRM_FORMAT_XRGB8888,
                modifier: tiled,
            },
        ];
        let gpu = allocation_plan(Some(&[tiled]), true, Some(&formats), DRM_FORMAT_XRGB8888);
        assert!(
            matches!(&gpu[0], Allocation::Explicit(m, flags) if m == &[tiled]
            && *flags == (ffi::GBM_BO_USE_SCANOUT | ffi::GBM_BO_USE_RENDERING))
        );
        let cpu = allocation_plan(None, true, Some(&formats), DRM_FORMAT_XRGB8888);
        assert!(
            matches!(&cpu[0], Allocation::Explicit(m, flags) if m == &[0] && *flags == ffi::GBM_BO_USE_SCANOUT)
        );
    }
    #[test]
    fn missing_or_disjoint_capabilities_never_imply_linear_vulkan_support() {
        let formats = [ScanoutFormat {
            fourcc: DRM_FORMAT_XRGB8888,
            modifier: 0,
        }];
        assert!(allocation_plan(Some(&[9]), true, Some(&formats), DRM_FORMAT_XRGB8888).is_empty());
        assert!(allocation_plan(Some(&[0]), true, None, DRM_FORMAT_XRGB8888).is_empty());
        assert!(allocation_plan(Some(&[0]), false, Some(&formats), DRM_FORMAT_XRGB8888).is_empty());
        let legacy = allocation_plan(None, false, None, DRM_FORMAT_XRGB8888);
        assert!(matches!(
            &legacy[0],
            Allocation::LegacyCpu {
                explicit: false,
                implicit: true
            }
        ));
        assert!(matches!(legacy.last(), Some(Allocation::Dumb)));
        let linear_gpu = allocation_plan(Some(&[0]), true, Some(&formats), DRM_FORMAT_XRGB8888);
        assert!(matches!(&linear_gpu[0], Allocation::Explicit(m, _) if m == &[0]));
    }
    #[test]
    fn policies_keep_explicit_renderers_strict() {
        assert_eq!(
            policies(Renderer::Auto),
            [Renderer::Vulkan, Renderer::Software]
        );
        assert_eq!(policies(Renderer::Software), [Renderer::Software]);
        assert_eq!(policies(Renderer::Vulkan), [Renderer::Vulkan]);
    }
    #[test]
    fn partial_pool_is_destroyed_before_another_allocator_runs() {
        use std::{cell::Cell, rc::Rc};
        struct Slot(Rc<Cell<usize>>);
        impl Drop for Slot {
            fn drop(&mut self) {
                self.0.set(self.0.get() - 1);
            }
        }
        let live = Rc::new(Cell::new(0));
        let failed = build_pool(|index, _| {
            if index == 2 {
                return Err("KMS test failed");
            }
            live.set(live.get() + 1);
            Ok(Slot(live.clone()))
        });
        assert!(failed.is_err());
        assert_eq!(live.get(), 0);
        let software = build_pool(|_, _| -> Result<_, ()> {
            live.set(live.get() + 1);
            Ok(Slot(live.clone()))
        })
        .unwrap();
        assert_eq!(live.get(), SLOTS);
        drop(software);
        assert_eq!(live.get(), 0);
    }
    #[test]
    fn rejects_only_the_failed_modifier_and_bounds_retries() {
        let mut allocation = Allocation::Explicit(
            vec![0, 9],
            ffi::GBM_BO_USE_SCANOUT | ffi::GBM_BO_USE_RENDERING,
        );
        assert!(allocation.reject(Some(0)));
        assert!(!allocation.reject(Some(9)));
        let mut attempts = Attempts::new();
        for _ in 0..MAX_ATTEMPTS {
            assert!(attempts.admit());
        }
        assert!(!attempts.admit());
    }
    #[test]
    fn permission_memory_and_device_errors_do_not_trigger_layout_fallback() {
        for code in [1, 5, 12, 13, 19] {
            assert!(
                !Failure::from(KmsError::native(KmsErrorKind::Native, "failure", code)).retryable
            );
        }
        assert!(
            Failure::from(KmsError::native(
                KmsErrorKind::Allocation,
                "unsupported tuple",
                22
            ))
            .retryable
        );
        assert!(!Failure::from(RenderError::new(RenderErrorKind::DeviceLost, "lost")).retryable);
    }
}

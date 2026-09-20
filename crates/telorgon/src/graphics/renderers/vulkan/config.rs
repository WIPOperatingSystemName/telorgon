/// Presentation behavior while a managed native window is being interactively resized.
///
/// This policy is consumed by managed presenter integrations. Hosted and offscreen rendering do
/// not own a native surface and therefore ignore it.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum VulkanLiveResizeMode {
    /// Reflow against the newest view metrics and commit a matching surface extent. Managed
    /// Windows presentation holds each interactive size callback until presentation completes for
    /// its matching frame, preventing the compositor from stretching an old swapchain image in the
    /// gap.
    #[default]
    Responsive,
    /// Keep the current swapchain during the native resize transaction, render reflowed previews
    /// through it, and commit the final extent after one last preview.
    ///
    /// The window system may nonuniformly scale these previews. This is an explicit compatibility
    /// fallback for Vulkan WSI implementations that stall while recreating intermediate extents.
    DeferredScaledPreview,
}

/// Policy and resource budgets for the Vulkan backend.
///
/// This value owns no Vulkan handle and does not imply that a device is available.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct VulkanConfig {
    pub enable_validation: bool,
    /// Enables the Windows Vulkan-to-D3D11 external-memory bridge used by the DXGI presenter.
    /// Unsupported adapters retain the Vulkan WSI presenter.
    pub enable_dxgi_presenter: bool,
    /// Enables present fences and explicit acquired-image release when the presentation device
    /// exposes `VK_EXT_swapchain_maintenance1`.
    pub enable_swapchain_maintenance1: bool,
    /// Enables exact presentation-ID completion when the presentation device exposes both
    /// `VK_KHR_present_id` and `VK_KHR_present_wait`.
    pub enable_present_wait: bool,
    /// Opts into low-latency, tear-free MAILBOX presentation with mandatory FIFO fallback.
    ///
    /// FIFO is the default so managed Windows resize testing uses deterministic presentation
    /// ordering instead of allowing a pending MAILBOX image to be replaced.
    pub prefer_mailbox_present: bool,
    /// Selects managed interactive-resize presentation behavior.
    pub live_resize_mode: VulkanLiveResizeMode,
    pub frames_in_flight: usize,
    pub staging_budget_bytes: u64,
    pub device_local_budget_bytes: Option<u64>,
}

impl Default for VulkanConfig {
    fn default() -> Self {
        Self {
            enable_validation: cfg!(debug_assertions),
            enable_dxgi_presenter: cfg!(target_os = "windows"),
            enable_swapchain_maintenance1: true,
            enable_present_wait: true,
            prefer_mailbox_present: false,
            live_resize_mode: VulkanLiveResizeMode::Responsive,
            frames_in_flight: 3,
            staging_budget_bytes: 4 * 1024 * 1024,
            device_local_budget_bytes: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responsive_live_resize_is_the_default() {
        assert_eq!(
            VulkanConfig::default().live_resize_mode,
            VulkanLiveResizeMode::Responsive
        );
    }

    #[test]
    fn fifo_presentation_is_the_default() {
        assert!(!VulkanConfig::default().prefer_mailbox_present);
    }

    #[test]
    fn exact_present_wait_is_enabled_by_default() {
        assert!(VulkanConfig::default().enable_present_wait);
    }

    #[test]
    fn dxgi_presentation_is_enabled_only_on_windows_by_default() {
        assert_eq!(
            VulkanConfig::default().enable_dxgi_presenter,
            cfg!(target_os = "windows")
        );
    }
}

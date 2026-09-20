use std::ffi::CStr;

use crate::graphics::render::RenderResult;
use ash::vk;

use crate::graphics::renderers::vulkan::VulkanInstance;
use crate::graphics::renderers::vulkan::error::vk_error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterReport {
    pub index: usize,
    pub name: String,
    pub device_type: vk::PhysicalDeviceType,
    pub api_version: u32,
    pub graphics_queue_families: Vec<u32>,
    pub score: u32,
    pub supported: bool,
    pub rejection_reasons: Vec<String>,
}

impl AdapterReport {
    pub(crate) fn enumerate(instance: &VulkanInstance) -> RenderResult<Vec<Self>> {
        let devices = unsafe { instance.inner.raw.enumerate_physical_devices() }
            .map_err(|result| vk_error("failed to enumerate Vulkan physical devices", result))?;
        Ok(devices
            .into_iter()
            .enumerate()
            .map(|(index, physical_device)| {
                let properties = unsafe {
                    instance
                        .inner
                        .raw
                        .get_physical_device_properties(physical_device)
                };
                let queues = unsafe {
                    instance
                        .inner
                        .raw
                        .get_physical_device_queue_family_properties(physical_device)
                };
                let graphics_queue_families = queues
                    .iter()
                    .enumerate()
                    .filter(|(_, family)| family.queue_flags.contains(vk::QueueFlags::GRAPHICS))
                    .map(|(family, _)| family as u32)
                    .collect::<Vec<_>>();
                let mut rejection_reasons = Vec::new();
                if properties.api_version < vk::API_VERSION_1_3 {
                    rejection_reasons.push("physical device does not expose Vulkan 1.3".to_owned());
                }
                if graphics_queue_families.is_empty() {
                    rejection_reasons.push("physical device has no graphics queue".to_owned());
                }
                if properties.device_type == vk::PhysicalDeviceType::CPU {
                    rejection_reasons.push(
                        "CPU Vulkan adapters are excluded from the hardware renderer profile"
                            .to_owned(),
                    );
                }
                let device_bonus = match properties.device_type {
                    vk::PhysicalDeviceType::DISCRETE_GPU => 4_000,
                    vk::PhysicalDeviceType::INTEGRATED_GPU => 3_000,
                    vk::PhysicalDeviceType::VIRTUAL_GPU => 2_000,
                    vk::PhysicalDeviceType::CPU => 0,
                    _ => 1_000,
                };
                let supported = rejection_reasons.is_empty();
                Self {
                    index,
                    name: unsafe { CStr::from_ptr(properties.device_name.as_ptr()) }
                        .to_string_lossy()
                        .into_owned(),
                    device_type: properties.device_type,
                    api_version: properties.api_version,
                    graphics_queue_families,
                    score: device_bonus + properties.limits.max_image_dimension2_d.min(8_192),
                    supported,
                    rejection_reasons,
                }
            })
            .collect())
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DeviceSelection {
    pub adapter_index: usize,
}

impl DeviceSelection {
    pub fn best(adapters: &[AdapterReport]) -> Option<Self> {
        adapters
            .iter()
            .filter(|adapter| adapter.supported)
            .max_by_key(|adapter| adapter.score)
            .map(|adapter| Self {
                adapter_index: adapter.index,
            })
    }
}

#[cfg(target_os = "linux")]
impl VulkanInstance {
    /// Resolve the display's actual device before allocating/importing scanout targets.
    pub(crate) fn drm_adapter(&self, fd: &std::os::fd::OwnedFd) -> RenderResult<usize> {
        use crate::graphics::render::{RenderError, RenderErrorKind};
        use std::os::unix::fs::MetadataExt;
        let unsupported = |message: String| RenderError::new(RenderErrorKind::Unsupported, message);
        let file = std::fs::File::from(fd.try_clone().map_err(|e| unsupported(e.to_string()))?);
        let rdev = file
            .metadata()
            .map_err(|e| unsupported(e.to_string()))?
            .rdev();
        let (major, minor) = linux_device_numbers(rdev);
        let sysfs = |major: i64, minor: i64| {
            std::fs::canonicalize(format!("/sys/dev/char/{major}:{minor}/device")).ok()
        };
        let display_path = sysfs(major as i64, minor as i64);
        let devices = unsafe { self.inner.raw.enumerate_physical_devices() }
            .map_err(|e| vk_error("scanout adapter enumeration failed", e))?;
        for (index, physical) in devices.into_iter().enumerate() {
            let extensions = unsafe {
                self.inner
                    .raw
                    .enumerate_device_extension_properties(physical)
            }
            .map_err(|e| vk_error("scanout adapter extension query failed", e))?;
            let has = |name: &CStr| {
                extensions
                    .iter()
                    .any(|p| unsafe { CStr::from_ptr(p.extension_name.as_ptr()) } == name)
            };
            if has(ash::ext::physical_device_drm::NAME) {
                let mut drm = vk::PhysicalDeviceDrmPropertiesEXT::default();
                let mut properties = vk::PhysicalDeviceProperties2::default().push_next(&mut drm);
                unsafe {
                    self.inner
                        .raw
                        .get_physical_device_properties2(physical, &mut properties)
                };
                for (present, node_major, node_minor) in [
                    (drm.has_primary, drm.primary_major, drm.primary_minor),
                    (drm.has_render, drm.render_major, drm.render_minor),
                ] {
                    if present == vk::TRUE
                        && ((node_major == major as i64 && node_minor == minor as i64)
                            || display_path.as_ref().is_some_and(|path| {
                                sysfs(node_major, node_minor).as_ref() == Some(path)
                            }))
                    {
                        return Ok(index);
                    }
                }
            }
            // Verified PCI identity fallback for drivers predating physical_device_drm.
            if has(ash::ext::pci_bus_info::NAME) {
                let mut pci = vk::PhysicalDevicePCIBusInfoPropertiesEXT::default();
                let mut properties = vk::PhysicalDeviceProperties2::default().push_next(&mut pci);
                unsafe {
                    self.inner
                        .raw
                        .get_physical_device_properties2(physical, &mut properties)
                };
                let address = format!(
                    "{:04x}:{:02x}:{:02x}.{:x}",
                    pci.pci_domain, pci.pci_bus, pci.pci_device, pci.pci_function
                );
                if display_path
                    .as_ref()
                    .and_then(|path| path.file_name())
                    .is_some_and(|name| name == address.as_str())
                {
                    return Ok(index);
                }
            }
        }
        Err(unsupported(format!(
            "no Vulkan adapter matches DRM device {major}:{minor}; cross-device scanout is unavailable"
        )))
    }
}

#[cfg(any(target_os = "linux", test))]
fn linux_device_numbers(device: u64) -> (u64, u64) {
    (
        ((device >> 8) & 0xfff) | ((device >> 32) & 0xfffff000),
        (device & 0xff) | ((device >> 12) & 0xffffff00),
    )
}

#[cfg(test)]
mod drm_identity_tests {
    #[test]
    fn drm_primary_and_render_nodes_are_not_interchangeable() {
        assert_eq!(super::linux_device_numbers((226 << 8) | 1), (226, 1));
        assert_eq!(super::linux_device_numbers((226 << 8) | 128), (226, 128));
        assert_eq!(
            super::linux_device_numbers((1 << 32) | (226 << 8)),
            (226, 1 << 20)
        );
    }
}

//! Driver-specific synchronization policy, selected once for each Vulkan device.

use ash::vk;

use crate::graphics::renderers::vulkan::VulkanDiagnostics;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DriverWorkarounds {
    v3dv_geometry_upload_read: bool,
}

impl DriverWorkarounds {
    pub(crate) fn for_driver(driver_id: vk::DriverId) -> Self {
        Self {
            // V3DV narrows its binning access mask to VkAccessFlags before testing
            // SHADER_STORAGE_READ, losing that high bit. No fixed release has been
            // verified: do not guess an upper version bound or match GPU names.
            // Source audit and removal criteria: docs/V3DV_GEOMETRY_UPLOAD_WORKAROUND.md.
            v3dv_geometry_upload_read: driver_id == vk::DriverId::MESA_V3DV,
        }
    }

    pub(crate) fn geometry_upload_read_access(self) -> vk::AccessFlags2 {
        if self.v3dv_geometry_upload_read {
            // This low-bit Vulkan access flag includes storage reads. Keep the
            // surrounding shader stages and buffer range unchanged.
            vk::AccessFlags2::SHADER_READ
        } else {
            vk::AccessFlags2::SHADER_STORAGE_READ
        }
    }

    pub(crate) fn report(
        self,
        diagnostics: &VulkanDiagnostics,
        driver: &vk::PhysicalDeviceDriverProperties<'_>,
        adapter_name: &str,
        driver_version: u32,
    ) {
        let driver_name = driver
            .driver_name_as_c_str()
            .unwrap_or(c"<invalid>")
            .to_string_lossy();
        let driver_info = driver
            .driver_info_as_c_str()
            .unwrap_or(c"<invalid>")
            .to_string_lossy();
        let status = if self.v3dv_geometry_upload_read {
            "enabled"
        } else {
            "disabled"
        };
        diagnostics.record_info(format!(
            "Vulkan adapter={adapter_name:?}, driver_id={}, driver_name={driver_name:?}, \
             driver_info={driver_info:?}, driver_version_raw={driver_version:#x}; \
             v3dv_geometry_upload_read={status}",
            driver.driver_id.as_raw(),
        ));
        #[cfg(feature = "instrumentation")]
        if crate::runtime::instrumentation::is_active() {
            crate::runtime::instrumentation::counter!("gpu.adapter.driver_id", driver.driver_id.as_raw());
            crate::runtime::instrumentation::counter!(
                "gpu.workaround.v3dv_geometry_upload_read",
                u8::from(self.v3dv_geometry_upload_read)
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_v3dv_uses_the_compatible_geometry_read_mask() {
        assert_eq!(
            DriverWorkarounds::for_driver(vk::DriverId::MESA_V3DV).geometry_upload_read_access(),
            vk::AccessFlags2::SHADER_READ,
        );
        for driver in [
            vk::DriverId::AMD_PROPRIETARY,
            vk::DriverId::AMD_OPEN_SOURCE,
            vk::DriverId::MESA_RADV,
            vk::DriverId::NVIDIA_PROPRIETARY,
            vk::DriverId::MESA_NVK,
            vk::DriverId::INTEL_PROPRIETARY_WINDOWS,
            vk::DriverId::INTEL_OPEN_SOURCE_MESA,
            vk::DriverId::BROADCOM_PROPRIETARY,
            vk::DriverId::MESA_TURNIP,
            vk::DriverId::MESA_PANVK,
            vk::DriverId::MOLTENVK,
            vk::DriverId::from_raw(0),
            vk::DriverId::from_raw(i32::MAX),
        ] {
            assert_eq!(
                DriverWorkarounds::for_driver(driver).geometry_upload_read_access(),
                vk::AccessFlags2::SHADER_STORAGE_READ,
                "unexpected workaround for driver ID {}",
                driver.as_raw(),
            );
        }
        assert_eq!(
            DriverWorkarounds::default().geometry_upload_read_access(),
            vk::AccessFlags2::SHADER_STORAGE_READ,
        );
    }

    #[test]
    fn v3dv_read_flag_survives_the_drivers_32_bit_narrowing() {
        // Model the driver's truncation, not Telorgon's flag representation.
        let low_bits = u64::from(u32::MAX);
        assert_eq!(vk::AccessFlags2::SHADER_STORAGE_READ.as_raw() & low_bits, 0);
        let compatible =
            DriverWorkarounds::for_driver(vk::DriverId::MESA_V3DV).geometry_upload_read_access();
        assert_eq!(
            compatible.as_raw() & low_bits,
            vk::AccessFlags2::SHADER_READ.as_raw(),
        );
        assert_eq!(size_of::<vk::AccessFlags2>(), size_of::<u64>());
    }

    #[test]
    fn startup_diagnostic_reports_driver_identity_and_policy_without_warnings() {
        for (driver_id, status) in [
            (vk::DriverId::MESA_V3DV, "enabled"),
            (vk::DriverId::MESA_RADV, "disabled"),
        ] {
            let diagnostics = VulkanDiagnostics::default();
            let driver = vk::PhysicalDeviceDriverProperties::default()
                .driver_id(driver_id)
                .driver_name(c"test driver")
                .expect("test driver name fits")
                .driver_info(c"Mesa 25.0.7 downstream build")
                .expect("test driver info fits");
            DriverWorkarounds::for_driver(driver_id).report(
                &diagnostics,
                &driver,
                "test GPU",
                0x12345678,
            );
            let messages = diagnostics.messages();
            assert_eq!(messages.len(), 1);
            assert_eq!(
                messages[0].severity,
                vk::DebugUtilsMessageSeverityFlagsEXT::INFO
            );
            assert!(messages[0].message.contains("test GPU"));
            assert!(messages[0].message.contains("test driver"));
            assert!(messages[0].message.contains("Mesa 25.0.7 downstream build"));
            assert!(
                messages[0]
                    .message
                    .contains("driver_version_raw=0x12345678")
            );
            assert!(
                messages[0]
                    .message
                    .contains(&format!("v3dv_geometry_upload_read={status}"))
            );
            assert_eq!(diagnostics.error_count(), 0);
            assert_eq!(diagnostics.warning_count(), 0);
        }
    }
}

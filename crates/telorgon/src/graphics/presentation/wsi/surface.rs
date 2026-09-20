use std::ffi::CStr;

use crate::graphics::renderers::vulkan::interop;
use crate::graphics::renderers::vulkan::{InstanceExtensionRequest, VulkanInstance};
use ash::vk;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

use crate::graphics::presentation::wsi::error::{PresentError, PresentErrorKind, PresentResult};

pub fn required_instance_extensions(
    display: &impl HasDisplayHandle,
) -> PresentResult<Vec<InstanceExtensionRequest<'static>>> {
    let display = display.display_handle().map_err(|error| {
        PresentError::new(
            PresentErrorKind::Unsupported,
            format!("native display handle is unavailable: {error}"),
        )
    })?;
    let names = ash_window::enumerate_required_extensions(display.as_raw())
        .map_err(|result| PresentError::from_vk("failed to query surface extensions", result))?;
    let mut requests: Vec<_> = names
        .iter()
        .map(|pointer| {
            let name = unsafe { CStr::from_ptr(*pointer) };
            InstanceExtensionRequest::required(name)
        })
        .collect();
    requests.push(InstanceExtensionRequest::optional(
        ash::khr::get_surface_capabilities2::NAME,
    ));
    requests.push(InstanceExtensionRequest::optional(
        ash::ext::surface_maintenance1::NAME,
    ));
    Ok(requests)
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) struct PresentScalingSelection {
    pub(crate) gravity_x: vk::PresentGravityFlagsEXT,
    pub(crate) gravity_y: vk::PresentGravityFlagsEXT,
}

fn choose_gravity(supported: vk::PresentGravityFlagsEXT) -> Option<vk::PresentGravityFlagsEXT> {
    [
        vk::PresentGravityFlagsEXT::MIN,
        vk::PresentGravityFlagsEXT::CENTERED,
        vk::PresentGravityFlagsEXT::MAX,
    ]
    .into_iter()
    .find(|gravity| supported.contains(*gravity))
}

fn choose_one_to_one_scaling(
    supported_scaling: vk::PresentScalingFlagsEXT,
    supported_gravity_x: vk::PresentGravityFlagsEXT,
    supported_gravity_y: vk::PresentGravityFlagsEXT,
) -> Option<PresentScalingSelection> {
    if !supported_scaling.contains(vk::PresentScalingFlagsEXT::ONE_TO_ONE) {
        return None;
    }
    Some(PresentScalingSelection {
        gravity_x: choose_gravity(supported_gravity_x)?,
        gravity_y: choose_gravity(supported_gravity_y)?,
    })
}

pub struct VulkanWinitSurface {
    instance: VulkanInstance,
    loader: ash::khr::surface::Instance,
    raw: vk::SurfaceKHR,
}

impl VulkanWinitSurface {
    pub fn create(
        instance: &VulkanInstance,
        display: &impl HasDisplayHandle,
        window: &impl HasWindowHandle,
    ) -> PresentResult<Self> {
        let display = display.display_handle().map_err(|error| {
            PresentError::new(
                PresentErrorKind::Unsupported,
                format!("native display handle is unavailable: {error}"),
            )
        })?;
        let window = window.window_handle().map_err(|error| {
            PresentError::new(
                PresentErrorKind::Unsupported,
                format!("native window handle is unavailable: {error}"),
            )
        })?;
        let loader = ash::khr::surface::Instance::new(
            interop::instance_entry(instance),
            interop::raw_instance(instance),
        );
        let raw = unsafe {
            ash_window::create_surface(
                interop::instance_entry(instance),
                interop::raw_instance(instance),
                display.as_raw(),
                window.as_raw(),
                None,
            )
        }
        .map_err(|result| PresentError::from_vk("failed to create Vulkan surface", result))?;
        Ok(Self {
            instance: instance.clone(),
            loader,
            raw,
        })
    }

    pub fn presentation_requirement(
        &self,
    ) -> crate::graphics::renderers::vulkan::interop::PresentationRequirement<'_> {
        let surface = unsafe {
            crate::graphics::renderers::vulkan::interop::BorrowedVulkanSurface::new(
                &self.instance,
                &self.loader,
                self.raw,
            )
        };
        crate::graphics::renderers::vulkan::interop::PresentationRequirement { surface }
    }

    pub(crate) fn loader(&self) -> &ash::khr::surface::Instance {
        &self.loader
    }

    pub(crate) fn raw(&self) -> vk::SurfaceKHR {
        self.raw
    }

    pub(crate) fn one_to_one_present_scaling(
        &self,
        physical_device: vk::PhysicalDevice,
        present_mode: vk::PresentModeKHR,
    ) -> PresentResult<Option<PresentScalingSelection>> {
        if !interop::instance_extension_enabled(
            &self.instance,
            ash::khr::get_surface_capabilities2::NAME,
        ) || !interop::instance_extension_enabled(
            &self.instance,
            ash::ext::surface_maintenance1::NAME,
        ) {
            return Ok(None);
        }

        let loader = ash::khr::get_surface_capabilities2::Instance::new(
            interop::instance_entry(&self.instance),
            interop::raw_instance(&self.instance),
        );
        let mut present_mode_info = vk::SurfacePresentModeEXT::default().present_mode(present_mode);
        let surface_info = vk::PhysicalDeviceSurfaceInfo2KHR::default()
            .surface(self.raw)
            .push_next(&mut present_mode_info);
        let mut scaling_capabilities = vk::SurfacePresentScalingCapabilitiesEXT::default();
        let mut capabilities =
            vk::SurfaceCapabilities2KHR::default().push_next(&mut scaling_capabilities);
        unsafe {
            loader.get_physical_device_surface_capabilities2(
                physical_device,
                &surface_info,
                &mut capabilities,
            )
        }
        .map_err(|result| {
            PresentError::from_vk("failed to query surface presentation scaling", result)
        })?;

        Ok(choose_one_to_one_scaling(
            scaling_capabilities.supported_present_scaling,
            scaling_capabilities.supported_present_gravity_x,
            scaling_capabilities.supported_present_gravity_y,
        ))
    }
}

impl Drop for VulkanWinitSurface {
    fn drop(&mut self) {
        unsafe { self.loader.destroy_surface(self.raw, None) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_to_one_scaling_prefers_min_gravity() {
        let selection = choose_one_to_one_scaling(
            vk::PresentScalingFlagsEXT::ONE_TO_ONE | vk::PresentScalingFlagsEXT::STRETCH,
            vk::PresentGravityFlagsEXT::MIN | vk::PresentGravityFlagsEXT::CENTERED,
            vk::PresentGravityFlagsEXT::MIN | vk::PresentGravityFlagsEXT::MAX,
        )
        .expect("one-to-one and gravity are supported");
        assert_eq!(selection.gravity_x, vk::PresentGravityFlagsEXT::MIN);
        assert_eq!(selection.gravity_y, vk::PresentGravityFlagsEXT::MIN);
    }

    #[test]
    fn one_to_one_scaling_uses_supported_gravity_fallbacks() {
        let selection = choose_one_to_one_scaling(
            vk::PresentScalingFlagsEXT::ONE_TO_ONE,
            vk::PresentGravityFlagsEXT::CENTERED,
            vk::PresentGravityFlagsEXT::MAX,
        )
        .expect("one-to-one and fallback gravities are supported");
        assert_eq!(selection.gravity_x, vk::PresentGravityFlagsEXT::CENTERED);
        assert_eq!(selection.gravity_y, vk::PresentGravityFlagsEXT::MAX);
    }

    #[test]
    fn one_to_one_scaling_requires_scaling_and_both_gravities() {
        assert!(
            choose_one_to_one_scaling(
                vk::PresentScalingFlagsEXT::STRETCH,
                vk::PresentGravityFlagsEXT::MIN,
                vk::PresentGravityFlagsEXT::MIN,
            )
            .is_none()
        );
        assert!(
            choose_one_to_one_scaling(
                vk::PresentScalingFlagsEXT::ONE_TO_ONE,
                vk::PresentGravityFlagsEXT::MIN,
                vk::PresentGravityFlagsEXT::empty(),
            )
            .is_none()
        );
    }
}

use std::ffi::{CStr, CString, c_char};
use std::sync::Arc;

use crate::graphics::render::{RenderError, RenderErrorKind, RenderResult};
use ash::{Entry, vk};

use crate::graphics::renderers::vulkan::VulkanConfig;
use crate::graphics::renderers::vulkan::adapter::AdapterReport;
use crate::graphics::renderers::vulkan::diagnostics::{CallbackData, VulkanDiagnostics, debug_callback};
use crate::graphics::renderers::vulkan::error::vk_error;

const VALIDATION_LAYER: &CStr = c"VK_LAYER_KHRONOS_validation";

#[derive(Copy, Clone)]
pub struct InstanceExtensionRequest<'a> {
    pub name: &'a CStr,
    pub required: bool,
}

impl<'a> InstanceExtensionRequest<'a> {
    pub const fn required(name: &'a CStr) -> Self {
        Self {
            name,
            required: true,
        }
    }

    pub const fn optional(name: &'a CStr) -> Self {
        Self {
            name,
            required: false,
        }
    }
}

#[derive(Clone)]
pub struct VulkanInstance {
    pub(crate) inner: Arc<InstanceInner>,
}

pub(crate) struct InstanceInner {
    pub entry: Entry,
    pub raw: ash::Instance,
    pub diagnostics: VulkanDiagnostics,
    debug_loader: Option<ash::ext::debug_utils::Instance>,
    debug_messenger: Option<vk::DebugUtilsMessengerEXT>,
    _callback_data: Option<Box<CallbackData>>,
    pub(crate) enabled_extensions: Vec<CString>,
    owns_instance: bool,
}

fn resolve_extension_requests<'a>(
    requests: &'a [InstanceExtensionRequest<'a>],
    mut available: impl FnMut(&CStr) -> bool,
) -> RenderResult<Vec<&'a CStr>> {
    let mut enabled = Vec::with_capacity(requests.len());
    for request in requests {
        if !available(request.name) {
            if request.required {
                return Err(RenderError::new(
                    RenderErrorKind::Unsupported,
                    format!(
                        "required Vulkan instance extension {} is unavailable",
                        request.name.to_string_lossy()
                    ),
                ));
            }
            continue;
        }
        if !enabled.contains(&request.name) {
            enabled.push(request.name);
        }
    }
    Ok(enabled)
}

impl VulkanInstance {
    pub fn load(
        config: &VulkanConfig,
        extensions: &[InstanceExtensionRequest<'_>],
    ) -> RenderResult<Self> {
        let entry = unsafe { Entry::load() }.map_err(|error| {
            RenderError::new(
                RenderErrorKind::Unsupported,
                format!("Vulkan loader is unavailable: {error}"),
            )
        })?;
        let available_version = unsafe { entry.try_enumerate_instance_version() }
            .map_err(|result| vk_error("failed to query Vulkan instance version", result))?
            .unwrap_or(vk::API_VERSION_1_0);
        if available_version < vk::API_VERSION_1_3 {
            return Err(RenderError::new(
                RenderErrorKind::Unsupported,
                format!(
                    "Vulkan 1.3 is required; loader reports {}.{}.{}",
                    vk::api_version_major(available_version),
                    vk::api_version_minor(available_version),
                    vk::api_version_patch(available_version)
                ),
            ));
        }

        let available_extensions = unsafe { entry.enumerate_instance_extension_properties(None) }
            .map_err(|result| {
            vk_error("failed to enumerate Vulkan instance extensions", result)
        })?;
        let is_available = |name: &CStr| {
            available_extensions
                .iter()
                .any(|property| unsafe { CStr::from_ptr(property.extension_name.as_ptr()) == name })
        };
        let mut enabled_extensions = resolve_extension_requests(extensions, is_available)?;
        if config.enable_validation && !enabled_extensions.contains(&ash::ext::debug_utils::NAME) {
            if !is_available(ash::ext::debug_utils::NAME) {
                return Err(RenderError::new(
                    RenderErrorKind::Unsupported,
                    "VK_EXT_debug_utils was requested for validation but is unavailable",
                ));
            }
            enabled_extensions.push(ash::ext::debug_utils::NAME);
        }
        let enabled_extensions: Vec<CString> =
            enabled_extensions.into_iter().map(CStr::to_owned).collect();
        let extension_names: Vec<*const c_char> = enabled_extensions
            .iter()
            .map(|name| name.as_ptr())
            .collect();
        let layer_names = if config.enable_validation {
            let layers = unsafe { entry.enumerate_instance_layer_properties() }
                .map_err(|result| vk_error("failed to enumerate Vulkan layers", result))?;
            if !layers.iter().any(|property| unsafe {
                CStr::from_ptr(property.layer_name.as_ptr()) == VALIDATION_LAYER
            }) {
                return Err(RenderError::new(
                    RenderErrorKind::Unsupported,
                    "VK_LAYER_KHRONOS_validation was requested but is unavailable",
                ));
            }
            vec![VALIDATION_LAYER.as_ptr()]
        } else {
            Vec::new()
        };

        let app_name = CString::new("Telorgon").expect("static application name");
        let app_info = vk::ApplicationInfo::default()
            .application_name(&app_name)
            .application_version(vk::make_api_version(0, 0, 1, 0))
            .engine_name(&app_name)
            .engine_version(vk::make_api_version(0, 0, 1, 0))
            .api_version(vk::API_VERSION_1_3);
        let diagnostics = VulkanDiagnostics::default();
        let mut callback_data = config
            .enable_validation
            .then(|| diagnostics.callback_data());
        let mut debug_info = vk::DebugUtilsMessengerCreateInfoEXT::default()
            .message_severity(
                vk::DebugUtilsMessageSeverityFlagsEXT::ERROR
                    | vk::DebugUtilsMessageSeverityFlagsEXT::WARNING,
            )
            .message_type(
                vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                    | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                    | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
            )
            .pfn_user_callback(Some(debug_callback))
            .user_data(callback_data.as_mut().map_or(std::ptr::null_mut(), |data| {
                (&mut **data as *mut CallbackData).cast()
            }));
        let mut create_info = vk::InstanceCreateInfo::default()
            .application_info(&app_info)
            .enabled_extension_names(&extension_names)
            .enabled_layer_names(&layer_names);
        if config.enable_validation {
            create_info = create_info.push_next(&mut debug_info);
        }
        let raw = unsafe { entry.create_instance(&create_info, None) }
            .map_err(|result| vk_error("failed to create Vulkan instance", result))?;
        let (debug_loader, debug_messenger) = if config.enable_validation {
            let loader = ash::ext::debug_utils::Instance::new(&entry, &raw);
            let messenger = match unsafe { loader.create_debug_utils_messenger(&debug_info, None) }
            {
                Ok(messenger) => messenger,
                Err(result) => {
                    unsafe { raw.destroy_instance(None) };
                    return Err(vk_error("failed to create Vulkan debug messenger", result));
                }
            };
            (Some(loader), Some(messenger))
        } else {
            (None, None)
        };
        Ok(Self {
            inner: Arc::new(InstanceInner {
                entry,
                raw,
                diagnostics,
                debug_loader,
                debug_messenger,
                _callback_data: callback_data,
                enabled_extensions,
                owns_instance: true,
            }),
        })
    }

    pub fn adapters(&self) -> RenderResult<Vec<AdapterReport>> {
        AdapterReport::enumerate(self)
    }

    pub fn diagnostics(&self) -> &VulkanDiagnostics {
        &self.inner.diagnostics
    }
}

impl Drop for InstanceInner {
    fn drop(&mut self) {
        if !self.owns_instance {
            return;
        }
        unsafe {
            if let (Some(loader), Some(messenger)) = (&self.debug_loader, self.debug_messenger) {
                loader.destroy_debug_utils_messenger(messenger, None);
            }
            self.raw.destroy_instance(None);
        }
    }
}

impl VulkanInstance {
    pub(crate) unsafe fn from_borrowed_raw(entry: Entry, raw: ash::Instance) -> Self {
        Self {
            inner: Arc::new(InstanceInner {
                entry,
                raw,
                diagnostics: VulkanDiagnostics::default(),
                debug_loader: None,
                debug_messenger: None,
                _callback_data: None,
                enabled_extensions: Vec::new(),
                owns_instance: false,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_optional_extensions_are_omitted() {
        let requests = [
            InstanceExtensionRequest::required(c"VK_required"),
            InstanceExtensionRequest::optional(c"VK_optional"),
        ];
        let enabled = resolve_extension_requests(&requests, |name| name == c"VK_required")
            .expect("required extension is available");
        assert_eq!(enabled, [c"VK_required"]);
    }

    #[test]
    fn unavailable_required_extensions_fail() {
        let requests = [InstanceExtensionRequest::required(c"VK_required")];
        let error = resolve_extension_requests(&requests, |_| false)
            .expect_err("missing required extension must fail");
        assert_eq!(error.kind(), RenderErrorKind::Unsupported);
    }

    #[test]
    fn duplicate_extension_requests_are_enabled_once() {
        let requests = [
            InstanceExtensionRequest::required(c"VK_shared"),
            InstanceExtensionRequest::optional(c"VK_shared"),
        ];
        let enabled =
            resolve_extension_requests(&requests, |_| true).expect("extension is available");
        assert_eq!(enabled, [c"VK_shared"]);
    }
}

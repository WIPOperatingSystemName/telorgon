use std::ffi::{CStr, c_void};
use std::sync::{Arc, Mutex};

use ash::vk;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VulkanDebugMessage {
    pub severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    pub kind: vk::DebugUtilsMessageTypeFlagsEXT,
    pub message: String,
}

#[derive(Clone, Debug, Default)]
pub struct VulkanDiagnostics {
    messages: Arc<Mutex<Vec<VulkanDebugMessage>>>,
}

impl VulkanDiagnostics {
    pub(crate) fn record_info(&self, message: String) {
        self.messages
            .lock()
            .expect("diagnostic lock poisoned")
            .push(VulkanDebugMessage {
                severity: vk::DebugUtilsMessageSeverityFlagsEXT::INFO,
                kind: vk::DebugUtilsMessageTypeFlagsEXT::GENERAL,
                message,
            });
    }

    pub fn messages(&self) -> Vec<VulkanDebugMessage> {
        self.messages
            .lock()
            .expect("diagnostic lock poisoned")
            .clone()
    }

    pub fn error_count(&self) -> usize {
        self.messages()
            .iter()
            .filter(|message| {
                message
                    .severity
                    .contains(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR)
            })
            .count()
    }

    pub fn warning_count(&self) -> usize {
        self.messages()
            .iter()
            .filter(|message| {
                message
                    .severity
                    .contains(vk::DebugUtilsMessageSeverityFlagsEXT::WARNING)
            })
            .count()
    }

    pub(crate) fn callback_data(&self) -> Box<CallbackData> {
        Box::new(CallbackData {
            messages: Arc::clone(&self.messages),
        })
    }
}

pub(crate) struct CallbackData {
    messages: Arc<Mutex<Vec<VulkanDebugMessage>>>,
}

pub(crate) unsafe extern "system" fn debug_callback(
    severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    kind: vk::DebugUtilsMessageTypeFlagsEXT,
    data: *const vk::DebugUtilsMessengerCallbackDataEXT<'_>,
    user_data: *mut c_void,
) -> vk::Bool32 {
    if data.is_null() || user_data.is_null() {
        return vk::FALSE;
    }
    let message = unsafe {
        let pointer = (*data).p_message;
        if pointer.is_null() {
            String::new()
        } else {
            CStr::from_ptr(pointer).to_string_lossy().into_owned()
        }
    };
    let callback_data = unsafe { &*(user_data.cast::<CallbackData>()) };
    if let Ok(mut messages) = callback_data.messages.lock() {
        messages.push(VulkanDebugMessage {
            severity,
            kind,
            message,
        });
    }
    vk::FALSE
}

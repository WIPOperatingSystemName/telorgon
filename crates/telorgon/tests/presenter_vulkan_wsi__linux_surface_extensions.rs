#![cfg(feature = "application-vulkan")]
#![cfg(target_os = "linux")]

use std::ffi::CStr;
use std::ptr::NonNull;

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, RawDisplayHandle, WaylandDisplayHandle,
    XcbDisplayHandle, XlibDisplayHandle,
};
use telorgon::presenter_vulkan_wsi::required_instance_extensions;

static DISPLAY_TOKEN: u8 = 0;

struct TestDisplay(RawDisplayHandle);

impl HasDisplayHandle for TestDisplay {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        // SAFETY: the synthetic non-null token remains alive for the test and the extension query
        // only selects names from the handle variant; it does not access a display connection.
        Ok(unsafe { DisplayHandle::borrow_raw(self.0) })
    }
}

#[test]
fn wayland_and_x11_select_distinct_required_vulkan_surface_extensions() {
    let display = NonNull::from(&DISPLAY_TOKEN).cast();
    let wayland = extension_names(RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
        display,
    )));
    let xlib = extension_names(RawDisplayHandle::Xlib(XlibDisplayHandle::new(
        Some(display),
        0,
    )));
    let xcb = extension_names(RawDisplayHandle::Xcb(XcbDisplayHandle::new(
        Some(display),
        0,
    )));

    assert_eq!(wayland, vec!["VK_KHR_surface", "VK_KHR_wayland_surface"]);
    assert_eq!(xlib, vec!["VK_KHR_surface", "VK_KHR_xlib_surface"]);
    assert_eq!(xcb, vec!["VK_KHR_surface", "VK_KHR_xcb_surface"]);
}

fn extension_names(handle: RawDisplayHandle) -> Vec<&'static str> {
    required_instance_extensions(&TestDisplay(handle))
        .expect("Linux display handle must map to Vulkan surface extensions")
        .into_iter()
        .filter(|extension| extension.required)
        .map(|extension| CStr::to_str(extension.name).unwrap())
        .collect()
}

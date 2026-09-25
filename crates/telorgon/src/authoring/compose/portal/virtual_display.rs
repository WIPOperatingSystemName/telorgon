//! Trusted shell controls for host-owned virtual displays. Creating a display grants no capture.
use super::{CaptureDecision, ScreenCastPortalContext};
use crate::shell::OutputId;

/// Validated virtual-display request. Construction starts no native services.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VirtualDisplayConfig {
    pub(crate) label: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) frame_rate: u32,
}
impl VirtualDisplayConfig {
    pub fn new(
        label: impl Into<String>,
        width: u32,
        height: u32,
        frame_rate: u32,
    ) -> Result<Self, &'static str> {
        let label = label.into();
        if label.trim().is_empty() || label.len() > 128 || label.chars().any(char::is_control) {
            return Err("invalid virtual display label");
        }
        if !(1..=8192).contains(&width)
            || !(1..=8192).contains(&height)
            || !(1..=240).contains(&frame_rate)
        {
            return Err("virtual display extent or cadence exceeds limits");
        }
        Ok(Self {
            label,
            width,
            height,
            frame_rate,
        })
    }
}
/// Confirmed host state, with an identity that is never reused during the compositor lifetime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VirtualDisplaySnapshot {
    pub id: OutputId,
    pub label: String,
    pub width: u32,
    pub height: u32,
    pub frame_rate: u32,
    /// Confirmed back-to-front window layout.
    pub windows: Vec<(crate::shell::WindowId, crate::foundation::RectI)>,
}
impl ScreenCastPortalContext {
    /// Queue display creation on the shell owner. Returns acceptance, not completion.
    /// Observe `snapshot().virtual_displays` or its failure field. The new output starts
    /// empty; existing windows are not automatically assigned or shared. Default handles
    /// are inert. At most eight virtual displays can exist in this host.
    pub fn create_virtual_display(&self, config: VirtualDisplayConfig) -> bool {
        self.send(CaptureDecision::CreateVirtualDisplay(config))
    }
    /// Queue withdrawal of a virtual display from the current snapshot. A physical output
    /// cannot be withdrawn through this handle. Completion is observed in the next snapshot.
    pub fn remove_virtual_display(&self, output: OutputId) -> bool {
        self.send(CaptureDecision::RemoveVirtualDisplay(output))
    }
}

impl ScreenCastPortalContext {
    /// Replace the virtual display's window layout, back to front. Each window must be
    /// currently mapped and can belong to only one virtual display. An empty list returns
    /// all its windows to the physical desktop. Rectangles are virtual-output pixel coordinates.
    /// This changes placement/membership, not capture authorization or input permissions.
    pub fn route_virtual_windows(
        &self,
        output: OutputId,
        windows: &[(crate::shell::WindowId, crate::foundation::RectI)],
    ) -> bool {
        if windows.len() > 64
            || windows.iter().any(|(_, rect)| {
                rect.width <= 0 || rect.height <= 0 || rect.width > 8192 || rect.height > 8192
            })
        {
            return false;
        }
        self.send(CaptureDecision::RouteVirtualDisplay(
            output,
            windows.to_vec(),
        ))
    }
}

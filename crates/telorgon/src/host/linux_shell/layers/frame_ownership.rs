use super::*;

/// Reconcile shell frame ownership before measuring geometry or preparing client layers.
pub(super) fn synchronize(
    windows: &mut BTreeMap<WaylandSurfaceId, ClientWindow>,
    frame_available: bool,
    tiled_client_decorations: bool,
) {
    for window in windows.values_mut() {
        window.frame_client_decorations = frame_available
            && tiled_client_decorations
            && window.backend == Some(WindowBackend::Wayland)
            && !window.server_decorated;
        if !frame_available || !window_has_frame(window) {
            window.chrome_outer = None;
            window.chrome_content_offset = None;
            window.chrome = None;
        }
    }
}

#[cfg(test)]
mod tests;

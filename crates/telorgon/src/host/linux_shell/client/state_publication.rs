use super::*;

/// Apply state without reacquiring the client's released buffer. Pending SHM copies instead
/// apply the latest state at completion; callers must leave those publications in flight.
#[allow(clippy::too_many_arguments)]
pub(in crate::host::linux_shell) fn apply_retained_surface_state(
    display: &Display,
    wayland: &mut NativeCompositor<'_>,
    windows: &mut BTreeMap<WaylandSurfaceId, ClientWindow>,
    identities: &mut WindowIdentities,
    configure_scheduler: &mut ConfigureScheduler,
    stacking_order: &mut Vec<WaylandSurfaceId>,
    next_window_offset: &mut i32,
    work_area: RectI,
    session_locked: bool,
    pointer_scene_dirty: &mut bool,
    snapshot: &crate::integrations::wayland::compositor::SurfaceStateSnapshot,
) -> AppResult<()> {
    let Some(window) = windows.get(&snapshot.surface) else {
        return Ok(());
    };
    let image = PreparedClientImage::Unchanged {
        extent: wayland
            .surface_logical_size(snapshot.surface)
            .map_err(app_error)?,
        raster_extent: window.presentation.image_size,
        pixel_format: window.presentation.pixel_format,
        alpha_mode: window.presentation.alpha_mode,
    };
    apply_surface_publication(
        display,
        wayland,
        windows,
        identities,
        configure_scheduler,
        stacking_order,
        next_window_offset,
        work_area,
        session_locked,
        pointer_scene_dirty,
        snapshot,
        image,
    )
}

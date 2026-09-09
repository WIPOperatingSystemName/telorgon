use super::renderer::DmaBufRetirement;
use super::*;

pub(super) struct ClientWindow {
    pub(super) backend: Option<WindowBackend>,
    pub(super) frame_title: Option<String>,
    pub(super) desktop_id: Option<crate::shell::WindowId>,
    pub(super) role: SurfaceRole,
    pub(super) parent: Option<WaylandSurfaceId>,
    pub(super) offset: PointI,
    pub(super) server_decorated: bool,
    pub(super) position: PointI,
    pub(super) window_geometry: RectI,
    pub(super) requested_size: SizeI,
    pub(super) native_configure: NativeConfigureState,
    #[cfg(all(feature = "desktop-xwayland", target_env = "gnu"))]
    pub(super) resize_preview: super::x11_windows::ResizePreview,
    pub(super) restore_geometry: Option<(PointI, SizeI)>,
    pub(super) maximized: bool,
    pub(super) fullscreen: bool,
    pub(super) minimized: bool,
    pub(super) chrome_outer: Option<SizeI>,
    pub(super) chrome_content_offset: Option<PointI>,
    pub(super) chrome: Option<WindowChromeSnapshot>,
    pub(super) presentation: SurfacePresentation,
}

/// Retained surface content, independent of desktop policy and xdg transactions.
/// GPU ownership remains with the existing renderer retirement paths.
pub(super) struct SurfacePresentation {
    pub(super) revision: u64,
    /// Surface-local logical extent used for window geometry and input.
    pub(super) size: SizeI,
    /// Retained image pixel extent, independent of surface geometry.
    pub(super) image_size: SizeI,
    pub(super) alpha_mode: ImageAlphaMode,
    pub(super) pixel_format: ImagePixelFormat,
    pending_image_update: PendingClientImageUpdate,
    pub(super) pixels: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum PendingClientImageUpdate {
    #[default]
    Unchanged,
    Full(Arc<[u8]>),
    Region(RectI),
    External(ImageId),
}

impl PendingClientImageUpdate {
    fn merge_region(&mut self, update: &crate::render::ImageResourceUpdate) {
        match self {
            Self::Full(pixels) => patch_client_pixels(Arc::make_mut(pixels), update),
            Self::Region(rect) => *rect = union_rect(*rect, update.rect),
            Self::Unchanged => *self = Self::Region(update.rect),
            Self::External(_) => unreachable!("DMA-BUF content cannot receive an SHM patch"),
        }
    }
}

pub(super) enum PreparedClientImage {
    Unchanged {
        extent: SizeI,
        pixel_format: ImagePixelFormat,
        alpha_mode: ImageAlphaMode,
    },
    Full {
        logical_extent: SizeI,
        image: crate::render::ImageResource,
        retained_pixels: Vec<u8>,
    },
    Region(crate::render::ImageResourceUpdate),
    External {
        extent: SizeI,
        raster_extent: SizeI,
        pixel_format: ImagePixelFormat,
        alpha_mode: ImageAlphaMode,
        image: ImageId,
    },
}

pub(super) fn observe_surface_configure_acknowledgement(
    windows: &mut BTreeMap<WaylandSurfaceId, ClientWindow>,
    snapshot: &crate::compositor_wayland::SurfaceStateSnapshot,
) {
    if snapshot.role != Some(SurfaceRole::XdgToplevel) {
        return;
    }
    if let Some(final_resize) = windows
        .get_mut(&snapshot.surface)
        .and_then(|window| window.native_configure.resize_final.as_mut())
    {
        final_resize.observe_acknowledgement(snapshot.acknowledged_configure);
    }
}

impl PreparedClientImage {
    pub(super) fn full_scaled(image: crate::render::ImageResource, logical_extent: SizeI) -> Self {
        let retained_pixels = image.pixels.to_vec();
        Self::Full {
            logical_extent,
            image,
            retained_pixels,
        }
    }

    fn extent(&self) -> SizeI {
        match self {
            Self::Unchanged { extent, .. } => *extent,
            Self::Full { logical_extent, .. } => *logical_extent,
            Self::Region(update) => update.extent,
            Self::External { extent, .. } => *extent,
        }
    }

    fn raster_extent(&self) -> SizeI {
        match self {
            Self::Full { image, .. } => image.extent,
            Self::External { raster_extent, .. } => *raster_extent,
            _ => self.extent(),
        }
    }

    fn pixel_format(&self) -> ImagePixelFormat {
        match self {
            Self::Unchanged { pixel_format, .. } => *pixel_format,
            Self::Full { image, .. } => image.pixel_format,
            Self::Region(update) => update.pixel_format,
            Self::External { pixel_format, .. } => *pixel_format,
        }
    }

    fn alpha_mode(&self) -> ImageAlphaMode {
        match self {
            Self::Unchanged { alpha_mode, .. } => *alpha_mode,
            Self::Full { image, .. } => image.alpha_mode,
            Self::Region(update) => update.alpha_mode,
            Self::External { alpha_mode, .. } => *alpha_mode,
        }
    }
}

impl ClientWindow {
    pub(super) fn resize_veil_active(&self) -> bool {
        #[cfg(all(feature = "desktop-xwayland", target_env = "gnu"))]
        if self.resize_preview.active() {
            return true;
        }
        self.native_configure.resize_anchor.is_some()
            || self.native_configure.resize_final.is_some()
    }

    pub(super) fn resizing(&self) -> bool {
        #[cfg(all(feature = "desktop-xwayland", target_env = "gnu"))]
        if self.resize_preview.dragging() {
            return true;
        }
        self.native_configure.resize_anchor.is_some()
            && self.native_configure.resize_final.is_none()
    }

    pub(super) fn configure_size(&self) -> SizeI {
        if self.resizing() {
            SizeI {
                width: self.window_geometry.width,
                height: self.window_geometry.height,
            }
        } else {
            self.requested_size
        }
    }
}

impl SurfacePresentation {
    fn apply_image(&mut self, revision: u64, image: PreparedClientImage) {
        self.revision = self.revision.max(revision);
        match image {
            PreparedClientImage::Unchanged { .. } => {}
            PreparedClientImage::Full {
                image,
                retained_pixels,
                logical_extent,
            } => {
                self.size = logical_extent;
                self.image_size = image.extent;
                self.alpha_mode = image.alpha_mode;
                self.pixel_format = image.pixel_format;
                self.pixels = retained_pixels;
                self.pending_image_update = PendingClientImageUpdate::Full(image.pixels);
            }
            PreparedClientImage::Region(update) => {
                patch_client_pixels(&mut self.pixels, &update);
                self.pending_image_update.merge_region(&update);
            }
            PreparedClientImage::External {
                extent,
                raster_extent,
                pixel_format,
                alpha_mode,
                image,
            } => {
                self.size = extent;
                self.image_size = raster_extent;
                self.alpha_mode = alpha_mode;
                self.pixel_format = pixel_format;
                self.pixels.clear();
                self.pending_image_update = PendingClientImageUpdate::External(image);
            }
        }
    }

    pub(super) fn take_image_update(&mut self) -> DesktopImageUpdate {
        match std::mem::take(&mut self.pending_image_update) {
            PendingClientImageUpdate::Unchanged => DesktopImageUpdate::Reused,
            PendingClientImageUpdate::Full(pixels) => DesktopImageUpdate::Full(pixels),
            PendingClientImageUpdate::Region(rect) => {
                DesktopImageUpdate::Regions(vec![DesktopImageRegion {
                    rect,
                    row_bytes: rect.width as usize * 4,
                    pixels: copy_client_region(&self.pixels, self.image_size, rect).into(),
                }])
            }
            PendingClientImageUpdate::External(image) => DesktopImageUpdate::External {
                image,
                content_version: self.revision,
            },
        }
    }
}

/// A veil covers the entire client surface tree, not just its root buffer.
pub(super) fn resize_veil_owner(
    windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
    surface: WaylandSurfaceId,
) -> Option<WaylandSurfaceId> {
    let mut candidate = surface;
    for _ in 0..=windows.len() {
        let window = windows.get(&candidate)?;
        if matches!(
            window.role,
            SurfaceRole::XdgToplevel | SurfaceRole::Xwayland
        ) {
            return window.resize_veil_active().then_some(candidate);
        }
        candidate = window.parent?;
    }
    None
}

fn patch_client_pixels(target: &mut [u8], update: &crate::render::ImageResourceUpdate) {
    let destination_stride = update.extent.width as usize * 4;
    let copy_bytes = update.rect.width as usize * 4;
    for row in 0..update.rect.height as usize {
        let source = row * update.row_bytes;
        let target_offset =
            (update.rect.y as usize + row) * destination_stride + update.rect.x as usize * 4;
        target[target_offset..target_offset + copy_bytes]
            .copy_from_slice(&update.pixels[source..source + copy_bytes]);
    }
}

fn copy_client_region(source: &[u8], extent: SizeI, rect: RectI) -> Vec<u8> {
    let stride = extent.width as usize * 4;
    let row_bytes = rect.width as usize * 4;
    let mut pixels = Vec::with_capacity(row_bytes * rect.height as usize);
    for row in rect.y as usize..rect.bottom() as usize {
        let start = row * stride + rect.x as usize * 4;
        pixels.extend_from_slice(&source[start..start + row_bytes]);
    }
    pixels
}

#[allow(clippy::too_many_arguments)]
pub(super) fn apply_surface_publication(
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
    snapshot: &crate::compositor_wayland::SurfaceStateSnapshot,
    prepared_image: PreparedClientImage,
) -> AppResult<()> {
    let surface = snapshot.surface;
    let role = snapshot
        .role
        .ok_or_else(|| AppError::new("published surface has no role"))?;
    let image_extent = prepared_image.extent();
    let raster_extent = prepared_image.raster_extent();
    let image_pixel_format = prepared_image.pixel_format();
    let image_alpha_mode = prepared_image.alpha_mode();
    let window_geometry = if role == SurfaceRole::XdgToplevel {
        snapshot
            .window_geometry
            .unwrap_or_else(|| full_rect(image_extent))
    } else {
        full_rect(image_extent)
    };
    let committed_window_extent = SizeI {
        width: window_geometry.width,
        height: window_geometry.height,
    };
    let (parent, offset, position) = if role == SurfaceRole::Subsurface {
        let parent = wayland.core().subsurfaces.parent(surface);
        let offset = wayland
            .core()
            .subsurfaces
            .position(surface)
            .map_or(PointI::default(), |position| position.offset);
        let position = parent
            .and_then(|parent| windows.get(&parent))
            .map_or(offset, |parent| PointI {
                x: parent.position.x + offset.x,
                y: parent.position.y + offset.y,
            });
        (parent, offset, position)
    } else if role == SurfaceRole::XdgPopup {
        let (parent, geometry) = wayland.popup_placement(surface).unwrap_or((
            None,
            RectI {
                x: 0,
                y: 0,
                width: image_extent.width,
                height: image_extent.height,
            },
        ));
        let offset = PointI {
            x: geometry.x,
            y: geometry.y,
        };
        let position = parent
            .and_then(|parent| windows.get(&parent))
            .map_or(offset, |parent| PointI {
                x: parent.position.x + offset.x,
                y: parent.position.y + offset.y,
            });
        (parent, offset, position)
    } else if matches!(
        role,
        SurfaceRole::Cursor | SurfaceRole::DragIcon | SurfaceRole::SessionLock
    ) {
        (None, PointI::default(), PointI::default())
    } else {
        let position = windows.get(&surface).map_or_else(
            || {
                let offset = *next_window_offset;
                *next_window_offset = (*next_window_offset + 28) % 280;
                PointI {
                    x: work_area.x + 48 + offset,
                    y: work_area.y + 48 + offset,
                }
            },
            |window| window.position,
        );
        (None, PointI::default(), position)
    };
    let is_new = !windows.contains_key(&surface);
    let previous_window = windows.get(&surface);
    let mut requested_size = retained_requested_size(
        previous_window.map(|window| window.requested_size),
        committed_window_extent,
    );
    let mut reconciled_position = position;
    let mut resize_anchor =
        previous_window.and_then(|window| window.native_configure.resize_anchor);
    let mut retained_resize_final =
        previous_window.and_then(|window| window.native_configure.resize_final);
    if let Some(final_resize) = retained_resize_final.as_mut() {
        final_resize.observe_acknowledgement(snapshot.acknowledged_configure);
    }
    let final_resize_committed = role == SurfaceRole::XdgToplevel
        && retained_resize_final.is_some_and(FinalResizeConfigure::was_acknowledged);
    if final_resize_committed {
        if let Some(anchor) = resize_anchor.take() {
            reconciled_position = anchor.reconcile_position(position, committed_window_extent);
        }
        requested_size = committed_window_extent;
        retained_resize_final = None;
    }
    let (
        restore_geometry,
        maximized,
        fullscreen,
        minimized,
        chrome_outer,
        chrome_content_offset,
        chrome,
    ) = windows.get(&surface).map_or(
        (
            None,
            false,
            false,
            role == SurfaceRole::Xwayland,
            None,
            None,
            None,
        ),
        |window| {
            (
                window.restore_geometry,
                window.maximized,
                window.fullscreen,
                window.minimized,
                window.chrome_outer,
                window.chrome_content_offset,
                window.chrome.clone(),
            )
        },
    );
    let server_decorated = if role == SurfaceRole::XdgToplevel {
        wayland.decoration_mode(surface)
            != Some(crate::compositor_wayland::DecorationMode::ClientSide)
    } else {
        previous_window.is_some_and(|window| window.server_decorated)
    };
    let pointer_geometry_changed = !matches!(role, SurfaceRole::Cursor | SurfaceRole::DragIcon)
        && previous_window.is_none_or(|window| {
            window.role != role
                || window.position != reconciled_position
                || window.presentation.size != image_extent
                || window.window_geometry != window_geometry
                || window.requested_size != requested_size
                || window.native_configure.resize_anchor != resize_anchor
                || window.native_configure.resize_final != retained_resize_final
                || window.minimized != minimized
                || window.server_decorated != server_decorated
        });
    if let Some(window) = windows.get_mut(&surface) {
        window.role = role;
        window.parent = parent;
        window.offset = offset;
        window.server_decorated = server_decorated;
        window.position = reconciled_position;
        window.window_geometry = window_geometry;
        window.requested_size = requested_size;
        window.restore_geometry = restore_geometry;
        window.maximized = maximized;
        window.fullscreen = fullscreen;
        window.minimized = minimized;
        window.chrome_outer = chrome_outer;
        window.chrome_content_offset = chrome_content_offset;
        window.chrome = chrome;
        window.native_configure.resize_anchor = resize_anchor;
        window.native_configure.resize_final = retained_resize_final;
        window
            .presentation
            .apply_image(snapshot.revision, prepared_image);
    } else {
        let (pending_image_update, pixels) = match prepared_image {
            PreparedClientImage::Full {
                image,
                retained_pixels,
                ..
            } => (
                PendingClientImageUpdate::Full(image.pixels),
                retained_pixels,
            ),
            PreparedClientImage::External { image, .. } => {
                (PendingClientImageUpdate::External(image), Vec::new())
            }
            PreparedClientImage::Unchanged { .. } | PreparedClientImage::Region(_) => {
                return Err(AppError::new(
                    "new surface publication did not provide a complete image",
                ));
            }
        };
        windows.insert(
            surface,
            ClientWindow {
                backend: (role == SurfaceRole::XdgToplevel).then_some(WindowBackend::Wayland),
                frame_title: None,
                desktop_id: if role == SurfaceRole::XdgToplevel {
                    Some(identities.ensure(surface)?)
                } else {
                    None
                },
                role,
                parent,
                offset,
                server_decorated,
                position: reconciled_position,
                window_geometry,
                requested_size,
                restore_geometry,
                maximized,
                fullscreen,
                minimized,
                chrome_outer,
                chrome_content_offset,
                chrome,
                #[cfg(all(feature = "desktop-xwayland", target_env = "gnu"))]
                resize_preview: Default::default(),
                native_configure: NativeConfigureState {
                    resize_anchor,
                    resize_final: retained_resize_final,
                },
                presentation: SurfacePresentation {
                    revision: snapshot.revision,
                    size: image_extent,
                    image_size: raster_extent,
                    alpha_mode: image_alpha_mode,
                    pixel_format: image_pixel_format,
                    pending_image_update,
                    pixels,
                },
            },
        );
    }
    if role == SurfaceRole::XdgToplevel {
        debug_assert_eq!(
            windows.get(&surface).and_then(|window| window.desktop_id),
            identities.get(surface)
        );
    }
    if is_new && !matches!(role, SurfaceRole::Cursor | SurfaceRole::DragIcon) {
        stacking_order.push(surface);
    }
    *pointer_scene_dirty |= pointer_geometry_changed;
    if is_new && role == SurfaceRole::XdgToplevel && !session_locked {
        focus_toplevel(
            display,
            wayland,
            windows,
            configure_scheduler,
            stacking_order,
            Some(surface),
        )?;
    } else if session_locked && role == SurfaceRole::SessionLock {
        wayland
            .set_keyboard_focus(1, Some(surface), display.next_serial())
            .map_err(app_error)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn finish_shm_copy(
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
    pending_buffers: &mut BTreeMap<crate::compositor_wayland::WaylandBufferId, usize>,
    pending_surfaces: &mut BTreeMap<WaylandSurfaceId, usize>,
    completion: ShmCopyCompletion,
) -> AppResult<bool> {
    retire_pending_shm_use(
        wayland,
        pending_buffers,
        pending_surfaces,
        completion.snapshot.surface,
        completion.snapshot.revision,
        completion.buffer,
        true,
    )?;
    let current = wayland
        .core()
        .world
        .surface(completion.snapshot.surface)
        .map(|surface| surface.snapshot().clone());
    let completion_is_current = current.as_ref().is_some_and(|snapshot| {
        snapshot.revision == completion.snapshot.revision
            && snapshot.attachment == completion.snapshot.attachment
    });
    let apply =
        completion_is_current && !pending_surfaces.contains_key(&completion.snapshot.surface);
    if apply {
        let image = completion.result.map_err(AppError::new)?;
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
            &completion.snapshot,
            image,
        )?;
    }
    Ok(apply)
}

pub(super) fn discard_shm_copy(
    wayland: &mut NativeCompositor<'_>,
    pending_buffers: &mut BTreeMap<crate::compositor_wayland::WaylandBufferId, usize>,
    pending_surfaces: &mut BTreeMap<WaylandSurfaceId, usize>,
    request: ShmCopyRequest,
) -> AppResult<()> {
    retire_pending_shm_use(
        wayland,
        pending_buffers,
        pending_surfaces,
        request.snapshot.surface,
        request.snapshot.revision,
        request.buffer(),
        true,
    )
}

pub(super) fn discard_replaced_shm_copy(
    wayland: &mut NativeCompositor<'_>,
    pending_buffers: &mut BTreeMap<crate::compositor_wayland::WaylandBufferId, usize>,
    pending_surfaces: &mut BTreeMap<WaylandSurfaceId, usize>,
    request: ShmCopyRequest,
    replacement_buffer: crate::compositor_wayland::WaylandBufferId,
) -> AppResult<()> {
    // Recommitting the same wl_buffer does not permit release before the replacement read ends.
    // The new publication will either release synchronously or install its own pending use.
    let release_buffer = request.buffer() != replacement_buffer;
    retire_pending_shm_use(
        wayland,
        pending_buffers,
        pending_surfaces,
        request.snapshot.surface,
        request.snapshot.revision,
        request.buffer(),
        release_buffer,
    )
}

fn retire_pending_shm_use(
    wayland: &mut NativeCompositor<'_>,
    pending_buffers: &mut BTreeMap<crate::compositor_wayland::WaylandBufferId, usize>,
    pending_surfaces: &mut BTreeMap<WaylandSurfaceId, usize>,
    surface: WaylandSurfaceId,
    revision: u64,
    buffer: crate::compositor_wayland::WaylandBufferId,
    release_buffer: bool,
) -> AppResult<()> {
    let pending = pending_buffers
        .get_mut(&buffer)
        .ok_or_else(|| AppError::new("completed SHM buffer copy was not tracked"))?;
    *pending = pending
        .checked_sub(1)
        .ok_or_else(|| AppError::new("completed SHM buffer copy count underflow"))?;
    if *pending == 0 {
        pending_buffers.remove(&buffer);
    }
    let pending = pending_surfaces
        .get_mut(&surface)
        .ok_or_else(|| AppError::new("completed SHM surface copy was not tracked"))?;
    *pending = pending
        .checked_sub(1)
        .ok_or_else(|| AppError::new("completed SHM surface copy count underflow"))?;
    if *pending == 0 {
        pending_surfaces.remove(&surface);
    }
    wayland
        .finish_explicit_release(surface, revision, None)
        .map_err(app_error)?;
    if release_buffer && !pending_buffers.contains_key(&buffer) {
        wayland.release_buffer(buffer).map_err(app_error)?;
    }
    Ok(())
}

pub(super) fn finish_dma_buf_release(
    wayland: &mut NativeCompositor<'_>,
    retirement: DmaBufRetirement,
    fence: Option<OwnedFd>,
) -> AppResult<()> {
    wayland
        .finish_explicit_release(retirement.surface, retirement.revision, fence)
        .map_err(app_error)?;
    Ok(())
}

pub(super) fn retire_unsubmitted_dma_buf(
    wayland: &mut NativeCompositor<'_>,
    pending_buffers: &mut BTreeMap<crate::compositor_wayland::WaylandBufferId, usize>,
    retirement: DmaBufRetirement,
) -> AppResult<()> {
    finish_dma_buf_release(wayland, retirement, None)?;
    retire_submitted_dma_buf(wayland, pending_buffers, retirement)
}

pub(super) fn retire_submitted_dma_buf(
    wayland: &mut NativeCompositor<'_>,
    pending_buffers: &mut BTreeMap<crate::compositor_wayland::WaylandBufferId, usize>,
    retirement: DmaBufRetirement,
) -> AppResult<()> {
    let pending = pending_buffers
        .get_mut(&retirement.buffer)
        .ok_or_else(|| AppError::new("completed DMA-BUF use was not tracked"))?;
    *pending = pending
        .checked_sub(1)
        .ok_or_else(|| AppError::new("completed DMA-BUF use count underflow"))?;
    if *pending == 0 {
        pending_buffers.remove(&retirement.buffer);
        wayland
            .release_buffer(retirement.buffer)
            .map_err(app_error)?;
    }
    Ok(())
}

#[cfg(test)]
pub(super) mod maximize_preview_tests {
    use super::*;

    #[test]
    fn activation_raises_a_window_family_without_reordering_other_windows() {
        let ids: Vec<_> = (1..=4)
            .map(|id| WaylandSurfaceId::from_raw(id).unwrap())
            .collect();
        let mut windows = BTreeMap::new();
        for id in &ids {
            windows.insert(
                *id,
                test_window(
                    SizeI {
                        width: 100,
                        height: 80,
                    },
                    PointI::default(),
                ),
            );
        }
        let popup = windows.get_mut(&ids[1]).unwrap();
        popup.role = SurfaceRole::XdgPopup;
        popup.parent = Some(ids[0]);
        let mut order = ids.clone();
        super::super::input::raise_toplevel(&windows, &mut order, ids[1]);
        assert_eq!(order, [ids[2], ids[3], ids[0], ids[1]]);
        super::super::input::raise_toplevel(&windows, &mut order, ids[0]);
        assert_eq!(
            order,
            [ids[2], ids[3], ids[0], ids[1]],
            "repeated activation is stable"
        );
        super::super::input::raise_toplevel(&windows, &mut order, ids[2]);
        assert_eq!(order, [ids[3], ids[0], ids[1], ids[2]]);
        windows.get_mut(&ids[3]).unwrap().minimized = true;
        super::super::input::raise_toplevel(&windows, &mut order, ids[3]);
        assert_eq!(order, [ids[3], ids[0], ids[1], ids[2]]);
        order.remove(0);
        windows.get_mut(&ids[3]).unwrap().minimized = false;
        super::super::input::raise_toplevel(&windows, &mut order, ids[3]);
        assert_eq!(order, [ids[0], ids[1], ids[2], ids[3]]);
    }

    pub(in crate::application_host::desktop_wayland) fn test_window(
        size: SizeI,
        position: PointI,
    ) -> ClientWindow {
        ClientWindow {
            desktop_id: None,
            backend: Some(WindowBackend::Wayland),
            frame_title: None,
            role: SurfaceRole::XdgToplevel,
            parent: None,
            offset: PointI::default(),
            server_decorated: true,
            position,
            window_geometry: RectI {
                x: 0,
                y: 0,
                width: size.width,
                height: size.height,
            },
            requested_size: size,
            #[cfg(all(feature = "desktop-xwayland", target_env = "gnu"))]
            resize_preview: Default::default(),
            native_configure: NativeConfigureState::default(),
            restore_geometry: None,
            maximized: false,
            fullscreen: false,
            minimized: false,
            chrome_outer: None,
            chrome_content_offset: None,
            chrome: None,
            presentation: SurfacePresentation {
                revision: 1,
                size,
                image_size: size,
                alpha_mode: ImageAlphaMode::Opaque,
                pixel_format: ImagePixelFormat::Rgba8,
                pending_image_update: PendingClientImageUpdate::Unchanged,
                pixels: Vec::new(),
            },
        }
    }

    #[test]
    fn maximize_veils_content_without_an_interactive_grab_and_restore_cancels_it() {
        let surface = WaylandSurfaceId::from_raw(42).unwrap();
        let size = SizeI {
            width: 640,
            height: 480,
        };
        let position = PointI { x: 50, y: 60 };
        let mut windows = BTreeMap::from([(surface, test_window(size, position))]);
        let mut scheduler = ConfigureScheduler::default();
        let area = RectI {
            x: 0,
            y: 0,
            width: 1280,
            height: 800,
        };
        assert_eq!(resize_veil_owner(&windows, surface), None);
        set_window_maximized(
            &mut windows,
            &mut scheduler,
            surface,
            true,
            area,
            &LinuxDesktopConfig::default(),
        )
        .unwrap();
        assert_eq!(resize_veil_owner(&windows, surface), Some(surface));
        let window = windows.get(&surface).unwrap();
        assert!(
            !window.resizing(),
            "maximize must not advertise an interactive pointer resize"
        );
        assert_eq!(
            window.native_configure.resize_final.unwrap().size,
            window.requested_size
        );
        let pending = scheduler.drain().next().unwrap();
        assert!(!pending.resizing);
        assert_eq!(pending.size, window.requested_size);
        set_window_maximized(
            &mut windows,
            &mut scheduler,
            surface,
            false,
            area,
            &LinuxDesktopConfig::default(),
        )
        .unwrap();
        assert_eq!(resize_veil_owner(&windows, surface), None);
        assert_eq!(windows[&surface].requested_size, size);
        assert_eq!(windows[&surface].position, position);
    }
    #[test]
    fn titlebar_drag_restores_saved_size_and_continues_without_a_jump() {
        let surface = WaylandSurfaceId::from_raw(42).unwrap();
        let size = SizeI {
            width: 640,
            height: 480,
        };
        let position = PointI { x: 50, y: 60 };
        let output = SizeI {
            width: 1280,
            height: 800,
        };
        let area = RectI {
            x: 0,
            y: 0,
            width: output.width,
            height: output.height,
        };
        let config = LinuxDesktopConfig::default();
        for fraction in [0.1, 0.5, 0.9] {
            let mut windows = BTreeMap::from([(surface, test_window(size, position))]);
            let mut scheduler = ConfigureScheduler::default();
            set_window_maximized(&mut windows, &mut scheduler, surface, true, area, &config)
                .unwrap();
            windows.get_mut(&surface).unwrap().chrome_outer = Some(output);
            let _ = scheduler.drain().collect::<Vec<_>>();
            let start = PointF {
                x: fraction * output.width as f32,
                y: 12.0,
            };
            let mut grab = WindowInteraction::begin_move(&windows, surface, start).unwrap();
            apply_window_interaction(
                &mut windows,
                &mut grab,
                &mut scheduler,
                PointF {
                    x: start.x + 1.0,
                    y: 13.0,
                },
                output,
                &config,
            )
            .unwrap();
            assert!(windows[&surface].maximized, "click/jitter must not restore");
            let moved = PointF {
                x: start.x + 20.0,
                y: 52.0,
            };
            apply_window_interaction(
                &mut windows,
                &mut grab,
                &mut scheduler,
                moved,
                output,
                &config,
            )
            .unwrap();
            let window = &windows[&surface];
            assert!(!window.maximized);
            assert_eq!(window.requested_size, size);
            assert!(window.restore_geometry.is_none());
            assert!(window.native_configure.resize_final.is_none());
            assert_eq!(window.position.y, 40);
            assert_eq!(
                window.position.x,
                (moved.x - fraction * size.width as f32).round() as i32
            );
            let restored_position = window.position;
            let configure = scheduler.drain().next().unwrap();
            assert_eq!(configure.size, size);
            assert!(!configure.resizing);
            apply_window_interaction(
                &mut windows,
                &mut grab,
                &mut scheduler,
                PointF {
                    x: moved.x + 10.0,
                    y: moved.y + 15.0,
                },
                output,
                &config,
            )
            .unwrap();
            assert_eq!(
                windows[&surface].position,
                PointI {
                    x: restored_position.x + 10,
                    y: restored_position.y + 15
                }
            );
            finish_window_interaction(&mut windows, &mut scheduler, grab);
            assert!(scheduler.drain().next().is_none());
        }
    }

    #[test]
    fn true_fullscreen_and_minimized_windows_do_not_begin_titlebar_moves() {
        let surface = WaylandSurfaceId::from_raw(42).unwrap();
        let mut windows = BTreeMap::from([(
            surface,
            test_window(
                SizeI {
                    width: 640,
                    height: 480,
                },
                PointI::default(),
            ),
        )]);
        windows.get_mut(&surface).unwrap().fullscreen = true;
        assert!(WindowInteraction::begin_move(&windows, surface, PointF::default()).is_none());
        windows.get_mut(&surface).unwrap().fullscreen = false;
        windows.get_mut(&surface).unwrap().minimized = true;
        assert!(WindowInteraction::begin_move(&windows, surface, PointF::default()).is_none());
    }
}

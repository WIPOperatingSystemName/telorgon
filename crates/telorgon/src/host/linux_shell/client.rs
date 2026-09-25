use super::renderer::DmaBufRetirement;
use super::*;
mod stacking;
pub(super) use stacking::surface_tree_visible;
pub(super) mod state_publication;

pub(super) struct ClientWindow {
    pub(super) tile: Option<super::tiling::TilePlacement>,
    #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
    pub(super) tile_size_hints: Option<crate::integrations::x11::normal_hints::NormalHints>,
    pub(super) motion_style: crate::WindowMotion,
    /// Shared presentation latch: protocol completion may precede the first placeholder frame.
    pub(super) motion_veil_pending: bool,
    pub(super) tile_resize_hold: bool,
    pub(super) motion_input: Option<super::motion::VisualInput>,
    pub(super) size_policy: super::size_policy::SizePolicy,
    pub(super) last_policy_request: Option<SizeI>,
    pub(super) surface_scale: i32,
    pub(super) backend: Option<WindowBackend>,
    pub(super) frame_title: Option<String>,
    pub(super) application_identity: String,
    pub(super) application_icon: Option<crate::graphics::render::ImageResource>,
    pub(super) desktop_id: Option<crate::shell::WindowId>,
    pub(super) role: SurfaceRole,
    pub(super) parent: Option<WaylandSurfaceId>,
    pub(super) offset: PointI,
    pub(super) server_decorated: bool,
    /// Shell-owned border around a client header using tiled client styling.
    pub(super) frame_client_decorations: bool,
    pub(super) position: PointI,
    pub(super) window_geometry: RectI,
    pub(super) requested_size: SizeI,
    pub(super) native_configure: NativeConfigureState,
    #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
    pub(super) resize_preview: super::x11_windows::ResizePreview,
    pub(super) restore_geometry: Option<(PointI, SizeI)>,
    pub(super) maximized: bool,
    pub(super) fullscreen: bool,
    pub(super) minimized: bool,
    pub(super) virtual_output: Option<crate::shell::OutputId>,
    pub(super) chrome_outer: Option<SizeI>,
    pub(super) chrome_content_offset: Option<PointI>,
    pub(super) chrome: Option<WindowChromeSnapshot>,
    pub(super) presentation: SurfacePresentation,
}

/// Retained surface content, independent of desktop policy and xdg transactions.
/// GPU ownership remains with the existing renderer retirement paths.
pub(super) struct SurfacePresentation {
    pub(super) content_ready: bool,
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

impl SurfacePresentation {
    /// Damage is relative to exactly the previous commit, never an arbitrary retained image.
    pub(super) fn can_apply_damage(&self, revision: u64) -> bool {
        self.revision.checked_add(1) == Some(revision)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum PendingClientImageUpdate {
    #[default]
    Unchanged,
    Full(Arc<[u8]>),
    Region(RectI),
    External(ImageId, Option<RectI>),
}

impl PendingClientImageUpdate {
    fn merge_region(&mut self, update: &crate::graphics::render::ImageResourceUpdate) {
        match self {
            Self::Full(pixels) => patch_client_pixels(Arc::make_mut(pixels), update),
            Self::Region(rect) => *rect = union_rect(*rect, update.rect),
            Self::Unchanged => *self = Self::Region(update.rect),
            Self::External(..) => unreachable!("DMA-BUF content cannot receive an SHM patch"),
        }
    }
}

pub(super) enum PreparedClientImage {
    Unchanged {
        extent: SizeI,
        raster_extent: SizeI,
        pixel_format: ImagePixelFormat,
        alpha_mode: ImageAlphaMode,
    },
    Full {
        logical_extent: SizeI,
        image: crate::graphics::render::ImageResource,
        retained_pixels: Vec<u8>,
    },
    Region(crate::graphics::render::ImageResourceUpdate),
    External {
        extent: SizeI,
        raster_extent: SizeI,
        pixel_format: ImagePixelFormat,
        alpha_mode: ImageAlphaMode,
        image: ImageId,
        damage: Option<RectI>,
    },
}

pub(super) fn observe_surface_configure_acknowledgement(
    windows: &mut BTreeMap<WaylandSurfaceId, ClientWindow>,
    snapshot: &crate::integrations::wayland::compositor::SurfaceStateSnapshot,
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

fn publication_requested_size(previous: Option<&ClientWindow>, committed: SizeI) -> SizeI {
    let retained = previous.filter(|window| {
        // Free-standing clients may change their own geometry (for example when CSD
        // margins change). Retain a host target only while host sizing still owns it.
        matches!(
            window.role,
            SurfaceRole::XdgToplevel | SurfaceRole::Xwayland
        ) && (window.maximized
            || window.fullscreen
            || window.tile.is_some()
            || window.native_configure.resize_anchor.is_some()
            || window.native_configure.resize_final.is_some()
            || window.requested_size
                != SizeI {
                    width: window.window_geometry.width,
                    height: window.window_geometry.height,
                })
    });
    retained_requested_size(retained.map(|window| window.requested_size), committed)
}

impl PreparedClientImage {
    pub(super) fn full_scaled(
        image: crate::graphics::render::ImageResource,
        logical_extent: SizeI,
    ) -> Self {
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
            Self::Unchanged { raster_extent, .. } | Self::External { raster_extent, .. } => {
                *raster_extent
            }
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
    pub(super) fn hidden_on_primary(&self) -> bool {
        self.minimized || self.virtual_output.is_some()
    }
    /// Hidden final-size content still needs frame callbacks to produce its replacement.
    /// These callbacks are pacing hints, not claims that the hidden image was displayed.
    pub(super) fn waiting_for_resize_content(&self) -> bool {
        #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
        if self.resize_preview.active() && !self.resize_preview.dragging() {
            return true;
        }
        self.native_configure.resize_final.is_some()
    }

    pub(super) fn resize_veil_active(&self) -> bool {
        if self.motion_veil_pending || self.tile_resize_hold {
            return true;
        }
        #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
        if self.resize_preview.active() {
            return true;
        }
        self.native_configure.resize_anchor.is_some()
            || self.native_configure.resize_final.is_some()
    }

    pub(super) fn resizing(&self) -> bool {
        #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
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
            PreparedClientImage::Unchanged { extent, .. } => {
                self.size = extent;
            }
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
                damage,
            } => {
                self.size = extent;
                self.image_size = raster_extent;
                self.alpha_mode = alpha_mode;
                self.pixel_format = pixel_format;
                self.pixels.clear();
                self.pending_image_update = PendingClientImageUpdate::External(image, damage);
            }
        }
    }

    pub(super) fn take_image_update(&mut self) -> ShellImageUpdate {
        if !self.content_ready {
            return ShellImageUpdate::Unchanged;
        }
        match std::mem::take(&mut self.pending_image_update) {
            PendingClientImageUpdate::Unchanged => ShellImageUpdate::Reused,
            PendingClientImageUpdate::Full(pixels) => ShellImageUpdate::Full(pixels),
            PendingClientImageUpdate::Region(rect) => {
                ShellImageUpdate::Regions(vec![ShellImageRegion {
                    rect,
                    row_bytes: rect.width as usize * 4,
                    pixels: copy_client_region(&self.pixels, self.image_size, rect).into(),
                }])
            }
            PendingClientImageUpdate::External(image, damage) => ShellImageUpdate::External {
                image,
                content_version: self.revision,
                damage,
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

fn patch_client_pixels(target: &mut [u8], update: &crate::graphics::render::ImageResourceUpdate) {
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
    snapshot: &crate::integrations::wayland::compositor::SurfaceStateSnapshot,
    prepared_image: PreparedClientImage,
) -> AppResult<()> {
    let surface = snapshot.surface;
    let role = snapshot
        .role
        .ok_or_else(|| AppError::new("published surface has no role"))?;
    let surface_scale = wayland.x11_surface_scale(surface);
    let raw_extent = prepared_image.extent();
    let image_extent = SizeI {
        width: (raw_extent.width / surface_scale).max(1),
        height: (raw_extent.height / surface_scale).max(1),
    };
    let raster_extent = prepared_image.raster_extent();
    let kind = match &prepared_image {
        PreparedClientImage::Unchanged { .. } => "unchanged",
        PreparedClientImage::Full { .. } => "full",
        PreparedClientImage::Region(_) => "region",
        PreparedClientImage::External { .. } => "external",
    };
    super::resize_trace::event(
        surface.get(),
        "prepared",
        format_args!(
            "revision={} buffer={:?} kind={} extent={:?} raster={:?}",
            snapshot.revision,
            snapshot.attachment.map(|attachment| attachment.buffer),
            kind,
            raw_extent,
            raster_extent
        ),
    );
    let image_pixel_format = prepared_image.pixel_format();
    let image_alpha_mode = prepared_image.alpha_mode();
    crate::integrations::wayland::compositor::diagnostics::event(surface.get(), "prepared", format_args!(
        "role={role:?} revision={} kind={kind} extent={raw_extent:?} raster={raster_extent:?} alpha={image_alpha_mode:?} format={image_pixel_format:?}", snapshot.revision));
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
        let offset =
            wayland
                .core()
                .subsurfaces
                .position(surface)
                .map_or(PointI::default(), |position| PointI {
                    x: position.offset.x / surface_scale,
                    y: position.offset.y / surface_scale,
                });
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
    let mut requested_size = publication_requested_size(previous_window, committed_window_extent);
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
        if previous_window.is_none_or(|w| w.tile.is_none()) {
            requested_size = committed_window_extent;
        }
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
            == Some(crate::integrations::wayland::compositor::DecorationMode::ServerSide)
    } else {
        previous_window.is_some_and(|window| window.server_decorated)
    };
    crate::integrations::wayland::compositor::diagnostics::event(surface.get(), "geometry", format_args!(
        "role={role:?} parent={parent:?} offset={offset:?} position={reconciled_position:?} logical={raw_extent:?} raster={raster_extent:?} coordinate_density={surface_scale} buffer_scale={} transform={:?} viewport={:?} window_geometry={window_geometry:?} requested={requested_size:?} server_decorated={server_decorated} maximized={maximized} fullscreen={fullscreen}",
        snapshot.buffer_scale, snapshot.buffer_transform, wayland.viewport(surface)));
    let pointer_geometry_changed = !matches!(role, SurfaceRole::Cursor | SurfaceRole::DragIcon)
        && previous_window.is_none_or(|window| {
            window.role != role
                || window.position != reconciled_position
                || window.presentation.size != raw_extent
                || window.window_geometry != window_geometry
                || window.requested_size != requested_size
                || window.native_configure.resize_anchor != resize_anchor
                || window.native_configure.resize_final != retained_resize_final
                || window.minimized != minimized
                || window.server_decorated != server_decorated
        });
    if let Some(window) = windows.get_mut(&surface) {
        window.surface_scale = surface_scale;
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
            PreparedClientImage::External { image, damage, .. } => (
                PendingClientImageUpdate::External(image, damage),
                Vec::new(),
            ),
            PreparedClientImage::Unchanged { .. } | PreparedClientImage::Region(_) => {
                return Err(AppError::new(
                    "new surface publication did not provide a complete image",
                ));
            }
        };
        windows.insert(
            surface,
            ClientWindow {
                tile: None,
                #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
                tile_size_hints: None,
                motion_style: crate::WindowMotion::none(),
                motion_veil_pending: false,
                tile_resize_hold: false,
                motion_input: None,
                size_policy: Default::default(),
                last_policy_request: None,
                surface_scale,
                backend: (role == SurfaceRole::XdgToplevel).then_some(WindowBackend::Wayland),
                frame_title: None,
                application_identity: String::new(),
                application_icon: None,
                desktop_id: if role == SurfaceRole::XdgToplevel {
                    Some(identities.ensure(surface)?)
                } else {
                    None
                },
                role,
                parent,
                offset,
                server_decorated,
                frame_client_decorations: false,
                position: reconciled_position,
                window_geometry,
                requested_size,
                restore_geometry,
                maximized,
                fullscreen,
                minimized,
                virtual_output: None,
                chrome_outer,
                chrome_content_offset,
                chrome,
                #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
                resize_preview: Default::default(),
                native_configure: NativeConfigureState {
                    resize_anchor,
                    resize_final: retained_resize_final,
                },
                presentation: SurfacePresentation {
                    content_ready: true,
                    revision: snapshot.revision,
                    size: raw_extent,
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
        stacking::insert_new_surface(windows, stacking_order, surface);
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
    pending_buffers: &mut BTreeMap<
        crate::integrations::wayland::compositor::WaylandBufferId,
        usize,
    >,
    pending_surfaces: &mut BTreeMap<WaylandSurfaceId, usize>,
    completion: ShmCopyCompletion,
) -> AppResult<bool> {
    retire_pending_shm_use(
        wayland,
        pending_buffers,
        pending_surfaces,
        completion.snapshot.surface,
        completion.snapshot.attachment_revision,
        completion.buffer,
        true,
    )?;
    let current = wayland
        .core()
        .world
        .surface(completion.snapshot.surface)
        .map(|surface| surface.snapshot().clone());
    let completion_is_current = current.as_ref().is_some_and(|snapshot| {
        snapshot.attachment_revision == completion.snapshot.attachment_revision
            && snapshot.attachment == completion.snapshot.attachment
    });
    let apply =
        completion_is_current && !pending_surfaces.contains_key(&completion.snapshot.surface);
    if apply {
        let mut image = completion.result.map_err(AppError::new)?;
        if let PreparedClientImage::Full { logical_extent, .. } = &mut image {
            *logical_extent = wayland
                .surface_logical_size(completion.snapshot.surface)
                .map_err(app_error)?;
        }
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
            current.as_ref().expect("current completion has a surface"),
            image,
        )?;
    }
    Ok(apply)
}

pub(super) fn discard_shm_copy(
    wayland: &mut NativeCompositor<'_>,
    pending_buffers: &mut BTreeMap<
        crate::integrations::wayland::compositor::WaylandBufferId,
        usize,
    >,
    pending_surfaces: &mut BTreeMap<WaylandSurfaceId, usize>,
    request: ShmCopyRequest,
) -> AppResult<()> {
    retire_pending_shm_use(
        wayland,
        pending_buffers,
        pending_surfaces,
        request.snapshot.surface,
        request.snapshot.attachment_revision,
        request.buffer(),
        true,
    )
}

pub(super) fn discard_replaced_shm_copy(
    wayland: &mut NativeCompositor<'_>,
    pending_buffers: &mut BTreeMap<
        crate::integrations::wayland::compositor::WaylandBufferId,
        usize,
    >,
    pending_surfaces: &mut BTreeMap<WaylandSurfaceId, usize>,
    request: ShmCopyRequest,
    replacement_buffer: crate::integrations::wayland::compositor::WaylandBufferId,
) -> AppResult<()> {
    // Recommitting the same wl_buffer does not permit release before the replacement read ends.
    // The new publication will either release synchronously or install its own pending use.
    let release_buffer = request.buffer() != replacement_buffer;
    retire_pending_shm_use(
        wayland,
        pending_buffers,
        pending_surfaces,
        request.snapshot.surface,
        request.snapshot.attachment_revision,
        request.buffer(),
        release_buffer,
    )
}

fn retire_pending_shm_use(
    wayland: &mut NativeCompositor<'_>,
    pending_buffers: &mut BTreeMap<
        crate::integrations::wayland::compositor::WaylandBufferId,
        usize,
    >,
    pending_surfaces: &mut BTreeMap<WaylandSurfaceId, usize>,
    surface: WaylandSurfaceId,
    revision: u64,
    buffer: crate::integrations::wayland::compositor::WaylandBufferId,
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
    pending_buffers: &mut BTreeMap<
        crate::integrations::wayland::compositor::WaylandBufferId,
        usize,
    >,
    retirement: DmaBufRetirement,
) -> AppResult<()> {
    finish_dma_buf_release(wayland, retirement, None)?;
    retire_submitted_dma_buf(wayland, pending_buffers, retirement)
}

pub(super) fn retire_submitted_dma_buf(
    wayland: &mut NativeCompositor<'_>,
    pending_buffers: &mut BTreeMap<
        crate::integrations::wayland::compositor::WaylandBufferId,
        usize,
    >,
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
pub(super) mod maximize_preview_tests;

#[cfg(test)]
mod buffer_lifetime_tests;

use super::scene::frame_content_clips;
use super::*;
use crate::render::{BoxInstance, ClipId, SpatialId};

pub(super) struct Layer {
    pub(super) runtime: ComposedAppRuntime,
    pending_deltas: Vec<RenderSceneDelta>,
}

impl Layer {
    pub(super) fn new(
        driver: CompositionDriver,
        extent: SizeI,
        assets: AssetBundle,
        scale: crate::platform::ScaleFactor,
    ) -> AppResult<Self> {
        let mut runtime = ComposedAppRuntime::from_composition_driver(driver, extent)?;
        runtime.set_raster_scale(scale);
        let mut media = AssetMediaCache::new(assets).map_err(app_error)?;
        for resource in media.preload_render_resources().map_err(app_error)? {
            runtime.set_image_resource(resource)?;
        }
        Ok(Self {
            runtime,
            pending_deltas: Vec::new(),
        })
    }

    pub(super) fn prepare(&mut self, extent: SizeI, now: u64, force: bool) -> AppResult<()> {
        let extent_changed = self.runtime.extent()
            != (SizeF {
                width: extent.width as f32,
                height: extent.height as f32,
            });
        if extent_changed {
            self.runtime.resize(extent)?;
        }
        self.runtime
            .prepare_frame(MonotonicInstant::from_nanos(now), force)?;
        while let Some(delta) = self.runtime.pop_scene_delta() {
            self.pending_deltas.push(delta);
        }
        Ok(())
    }

    pub(super) fn take_deltas(&mut self) -> Vec<RenderSceneDelta> {
        std::mem::take(&mut self.pending_deltas)
    }

    pub(super) fn pointer_motion(&mut self, position: PointF, now: MonotonicInstant) -> bool {
        self.runtime
            .queue_input(crate::input::InputEvent::mouse_moved(position));
        self.runtime.flush_input(now).frame_needed_after
    }

    pub(super) fn pointer_button(&mut self, pressed: bool, now: MonotonicInstant) -> bool {
        self.runtime
            .queue_input(crate::input::InputEvent::mouse_button(
                crate::input::PointerButton::PRIMARY,
                if pressed {
                    crate::input::ButtonState::Pressed
                } else {
                    crate::input::ButtonState::Released
                },
            ));
        self.runtime.flush_input(now).frame_needed_after
    }

    fn has_pending_runtime_turn(&self, now: MonotonicInstant) -> bool {
        self.runtime.has_pending_runtime_turn(now)
            || (self.runtime.needs_frame() && !self.runtime.animation_active())
    }

    fn next_deadline(&self) -> Option<MonotonicInstant> {
        self.runtime.next_deadline()
    }

    fn animation_active(&self) -> bool {
        self.runtime.animation_active()
    }
}

pub(super) fn route_frame_pointer_motion(
    frames: &mut BTreeMap<WaylandSurfaceId, WindowFrameLayer>,
    windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
    position: PointF,
    session_locked: bool,
    now: MonotonicInstant,
) -> bool {
    let mut repaint = false;
    for (surface, frame) in frames {
        let local = windows
            .get(surface)
            .filter(|window| !window.minimized && !session_locked)
            .map_or(
                PointF {
                    x: -1_000_000.0,
                    y: -1_000_000.0,
                },
                |window| PointF {
                    x: position.x - window.position.x as f32,
                    y: position.y - window.position.y as f32,
                },
            );
        repaint |= frame.layer.pointer_motion(local, now);
    }
    repaint
}

pub(super) fn route_frame_pointer_button(
    frames: &mut BTreeMap<WaylandSurfaceId, WindowFrameLayer>,
    pressed: bool,
    now: MonotonicInstant,
) -> bool {
    frames.values_mut().fold(false, |repaint, frame| {
        repaint | frame.layer.pointer_button(pressed, now)
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn refresh_window_frames(
    factory: Option<&WindowFrameFactory>,
    frames: &mut BTreeMap<WaylandSurfaceId, WindowFrameLayer>,
    windows: &mut BTreeMap<WaylandSurfaceId, ClientWindow>,
    wayland: &NativeCompositor<'_>,
    config: &LinuxDesktopConfig,
    assets: AssetBundle,
    fallback_icon: &crate::AppIconProfile,
    wake: &EventNotifier,
    now: u64,
    scale: crate::platform::ScaleFactor,
    work_area: RectI,
    configure_scheduler: &mut ConfigureScheduler,
) -> AppResult<()> {
    let Some(factory) = factory else {
        frames.clear();
        for window in windows.values_mut() {
            window.chrome_outer = None;
            window.chrome_content_offset = None;
            window.chrome = None;
        }
        return Ok(());
    };

    frames.retain(|surface, _| {
        windows
            .get(surface)
            .is_some_and(|window| window.backend.is_some() && window_is_decorated(window))
    });

    let active = wayland
        .core()
        .seats
        .get(&1)
        .and_then(|seat| seat.keyboard_focus)
        .map(|focus| focus.surface);
    let surfaces = windows
        .iter()
        .filter(|(_, window)| window.backend.is_some() && window_is_decorated(window))
        .map(|(surface, _)| *surface)
        .collect::<Vec<_>>();
    let mut updates = Vec::with_capacity(surfaces.len());

    for surface in surfaces {
        let window = windows
            .get(&surface)
            .expect("frame candidates came from live windows");
        let metadata = wayland.toplevel_metadata(surface);
        let title = window.frame_title.clone().unwrap_or_else(|| {
            metadata.map_or_else(String::new, |metadata| {
                if metadata.title.is_empty() {
                    metadata.application_id.clone()
                } else {
                    metadata.title.clone()
                }
            })
        });
        let state = if window.fullscreen {
            WindowChromeState::Fullscreen
        } else if window.maximized {
            WindowChromeState::Maximized
        } else {
            WindowChromeState::Normal
        };
        let protocol_icon = wayland.toplevel_icon(surface);
        let icon_name = protocol_icon
            .and_then(|icon| icon.name.clone())
            .or_else(|| fallback_icon.name().map(str::to_owned));
        let icon_image = protocol_icon.and_then(|icon| {
            icon.images
                .iter()
                .min_by_key(|image| {
                    let logical = image.image.descriptor.size.width / image.scale.max(1);
                    logical.abs_diff(32)
                })
                .map(|image| (icon.revision, image.clone()))
        });
        let icon_image_id = icon_image
            .as_ref()
            .map(|(revision, _)| toplevel_icon_image_id(surface, *revision));
        let mut model = WindowChromeModel::new(u64::from(surface.get()), title)
            .state(state)
            .active(active == Some(surface));
        if let Some(name) = icon_name {
            model = model.app_icon_name(name);
        }
        if let Some(image) = icon_image_id {
            model = model.app_icon_image(image);
        } else if let Some(icon) = fallback_icon.preferred(32) {
            model = model.app_icon(icon);
        }
        let content_style = factory.content_style(&model);
        if content_style
            .is_some_and(|style| !style.corner_radius.is_finite() || style.corner_radius < 0.0)
        {
            return Err(AppError::new(
                "window content radius must be finite and nonnegative",
            ));
        }
        let fallback_outer = legacy_window_outer(window, config);
        let previous_outer = frames
            .get(&surface)
            .map_or(fallback_outer, |frame| frame.outer);
        let created = !frames.contains_key(&surface);
        if created {
            let mut driver = factory.compose(model.clone());
            driver.set_wake({
                let wake = wake.clone();
                move || wake.notify()
            });
            let layer = Layer::new(driver, previous_outer, assets, scale)?;
            frames.insert(
                surface,
                WindowFrameLayer {
                    model: model.clone(),
                    layer,
                    snapshot: None,
                    outer: previous_outer,
                    content_style,
                    border: None,
                    icon_image: None,
                },
            );
        }

        let frame = frames
            .get_mut(&surface)
            .expect("window frame was created above");
        if !created && frame.model != model {
            frame
                .layer
                .runtime
                .update_composition_root(factory.candidate(model.clone()))?;
        }
        if frame.icon_image != icon_image_id {
            if let Some(previous) = frame.icon_image {
                frame.layer.runtime.remove_image_resource(previous);
            }
            if let Some((revision, icon)) = &icon_image {
                let mut resource =
                    shm_image_resource(icon.buffer, (*revision).max(1), icon.image.clone())
                        .map_err(app_error)?;
                resource.image = icon_image_id.expect("image source produced an image id");
                resource.content_version = (*revision).max(1);
                frame.layer.runtime.set_image_resource(resource)?;
            }
            frame.icon_image = icon_image_id;
        }
        frame.model = model;
        frame.content_style = content_style;
        let snapshot = layout_window_frame(
            &mut frame.layer,
            &mut frame.outer,
            window.requested_size,
            window.maximized.then_some(work_area),
            now,
            created,
        )?;
        let style = frame
            .layer
            .runtime
            .ui()
            .box_styles
            .get(snapshot.frame.node)
            .cloned()
            .unwrap_or_default();
        frame.border = Some(BoxInstance {
            node: snapshot.frame.node,
            rect: snapshot.frame.bounds,
            view_bounds: snapshot.frame.bounds,
            background: match style.decoration.background {
                crate::ui::Background::Color(color) => Some(color),
                _ => None,
            },
            border: style.decoration.border,
            outline: Default::default(),
            corner_radii: style.decoration.corner_radii,
            shadows: style.decoration.shadows,
            opacity: style.opacity,
            clip: ClipId(0),
            spatial: SpatialId(0),
        });
        frame.snapshot = Some(snapshot.clone());
        frame.layer.prepare(frame.outer, now, false)?;
        updates.push((
            surface,
            frame.outer,
            PointI {
                x: snapshot.content.bounds.x.round() as i32,
                y: snapshot.content.bounds.y.round() as i32,
            },
            snapshot,
        ));
    }

    for (surface, outer, content_offset, snapshot) in updates {
        if let Some(window) = windows.get_mut(&surface) {
            if window.maximized {
                let content_size = SizeI {
                    width: snapshot.content.bounds.width.round().max(1.0) as i32,
                    height: snapshot.content.bounds.height.round().max(1.0) as i32,
                };
                if window.requested_size != content_size {
                    window.requested_size = content_size;
                    // A measured custom-frame size supersedes the fallback configure. Keep
                    // the placeholder until content for this new transaction is published.
                    if window.backend == Some(WindowBackend::Wayland) {
                        window.native_configure.resize_final =
                            Some(FinalResizeConfigure::pending(content_size));
                    }
                    configure_scheduler.schedule_final(surface, content_size);
                }
            }
            window.chrome_outer = Some(outer);
            window.chrome_content_offset = Some(content_offset);
            window.chrome = Some(snapshot);
        }
    }
    Ok(())
}

fn layout_window_frame(
    layer: &mut Layer,
    outer: &mut SizeI,
    requested_size: SizeI,
    maximized_area: Option<RectI>,
    now: u64,
    force: bool,
) -> AppResult<WindowChromeSnapshot> {
    if let Some(work_area) = maximized_area {
        // Maximization constrains the OUTER frame; the composed content slot determines
        // the client configure size, including state-specific decoration changes.
        *outer = SizeI {
            width: work_area.width.max(1),
            height: work_area.height.max(1),
        };
    }
    layer.prepare(*outer, now, force)?;
    let mut snapshot = WindowChromeSnapshot::derive(layer.runtime.ui(), layer.runtime.layout())
        .map_err(app_error)?;
    let content_width = snapshot.content.bounds.width.round().max(1.0) as i32;
    let content_height = snapshot.content.bounds.height.round().max(1.0) as i32;
    let corrected = SizeI {
        width: outer
            .width
            .saturating_add(requested_size.width.saturating_sub(content_width))
            .max(1),
        height: outer
            .height
            .saturating_add(requested_size.height.saturating_sub(content_height))
            .max(1),
    };
    if maximized_area.is_none() && corrected != *outer {
        *outer = corrected;
        layer.prepare(*outer, now, true)?;
        snapshot = WindowChromeSnapshot::derive(layer.runtime.ui(), layer.runtime.layout())
            .map_err(app_error)?;
    }
    Ok(snapshot)
}

fn toplevel_icon_image_id(surface: WaylandSurfaceId, revision: u64) -> ImageId {
    let folded = revision as u32 ^ (revision >> 32) as u32;
    ImageId(0x6000_0000_u32 ^ surface.get().rotate_left(11) ^ folded.rotate_left(3))
}

pub(super) struct WidgetLayer {
    pub(super) anchor: ShellWidgetAnchor,
    pub(super) width: ShellWidgetExtent,
    pub(super) height: ShellWidgetExtent,
    pub(super) reserved_space: i32,
    pub(super) layer: Layer,
}

pub(super) struct WindowFrameLayer {
    pub(super) model: WindowChromeModel,
    pub(super) layer: Layer,
    pub(super) snapshot: Option<WindowChromeSnapshot>,
    pub(super) outer: SizeI,
    content_style: Option<crate::window_chrome::WindowContentStyle>,
    border: Option<BoxInstance>,
    icon_image: Option<ImageId>,
}

pub(super) fn desktop_runtime_schedule(
    background: &Layer,
    frames: &BTreeMap<WaylandSurfaceId, WindowFrameLayer>,
    pointer: Option<&Layer>,
    icons: &[(String, Layer)],
    widgets: &[WidgetLayer],
    now: MonotonicInstant,
) -> (bool, Option<MonotonicInstant>, bool) {
    let layers = std::iter::once(background)
        .chain(frames.values().map(|frame| &frame.layer))
        .chain(pointer)
        .chain(icons.iter().map(|(_, layer)| layer))
        .chain(widgets.iter().map(|widget| &widget.layer));
    let mut immediate = false;
    let mut deadline = None::<MonotonicInstant>;
    let mut animation = false;
    for layer in layers {
        immediate |= layer.has_pending_runtime_turn(now);
        animation |= layer.animation_active();
        if let Some(candidate) = layer.next_deadline() {
            deadline = Some(deadline.map_or(candidate, |current| current.min(candidate)));
        }
    }
    (immediate, deadline, animation)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn prepare_desktop_layers(
    session_locked: bool,
    extent: SizeI,
    now: u64,
    first_modeset: bool,
    background: &mut Layer,
    frames: &mut BTreeMap<WaylandSurfaceId, WindowFrameLayer>,
    windows: &mut BTreeMap<WaylandSurfaceId, ClientWindow>,
    stacking_order: &[WaylandSurfaceId],
    widgets: &mut [WidgetLayer],
    icons: &mut [(String, Layer)],
    pointer_layer: &mut Option<Layer>,
    drag_icon: Option<WaylandSurfaceId>,
    drag_position: PointF,
    cursor: Option<&CursorVisual>,
    pointer_position: PointF,
    config: &LinuxDesktopConfig,
) -> AppResult<Vec<DesktopLayer>> {
    let mut layers = Vec::new();
    background.prepare(extent, now, first_modeset)?;
    layers.push(DesktopLayer::retained(
        DesktopLayerKey::Background,
        DesktopSceneKey::Background,
        if session_locked {
            Vec::new()
        } else {
            background.take_deltas()
        },
        extent,
        PointI::default(),
        !session_locked,
    ));

    let placements = windows
        .iter()
        .map(|(surface, window)| {
            let position = window
                .parent
                .and_then(|parent| windows.get(&parent))
                .map_or(window.position, |parent| {
                    let content_offset = if parent.role == SurfaceRole::Xwayland {
                        window_content_offset(parent, config)
                    } else {
                        PointI::default()
                    };
                    PointI {
                        x: parent.position.x + content_offset.x + window.offset.x,
                        y: parent.position.y + content_offset.y + window.offset.y,
                    }
                });
            (*surface, position)
        })
        .collect::<BTreeMap<_, _>>();
    let content_clips = windows
        .iter()
        .filter_map(|(surface, window)| {
            if !window_is_decorated(window) {
                return None;
            }
            let frame = frames.get(surface)?;
            let border = frame.border.as_ref()?;
            let position = placements.get(surface).copied().unwrap_or(window.position);
            let rect = window_content_rect(window, position, config);
            let clips = frame_content_clips(
                border,
                position,
                rect,
                frame.content_style.map_or(0.0, |style| style.corner_radius),
            );
            Some((*surface, (rect, clips)))
        })
        .collect::<BTreeMap<_, _>>();
    for surface in stacking_order {
        let veiled = resize_veil_owner(windows, *surface).is_some();
        // Only subsurfaces inherit their toplevel's clip. Popups are independent overlays and
        // may legitimately extend beyond the parent window.
        let mut owner = Some(*surface);
        for _ in 0..=windows.len() {
            let Some(candidate) = owner.and_then(|id| windows.get(&id)) else {
                break;
            };
            if candidate.role == SurfaceRole::Subsurface {
                owner = candidate.parent;
            } else {
                break;
            }
        }
        let inherited_clip = owner.and_then(|id| content_clips.get(&id)).copied();
        let Some(window) = windows.get_mut(surface) else {
            continue;
        };
        if matches!(window.role, SurfaceRole::Cursor | SurfaceRole::DragIcon) {
            continue;
        }
        let visible =
            !window.minimized && (window.role == SurfaceRole::SessionLock) == session_locked;
        let position = placements.get(surface).copied().unwrap_or(window.position);
        let outer = window
            .chrome_outer
            .unwrap_or_else(|| legacy_window_outer(window, config));
        let content_rect = window_content_rect(window, position, config);
        let content_style = window_is_decorated(window)
            .then(|| frames.get(surface).and_then(|frame| frame.content_style))
            .flatten();
        if window_is_decorated(window)
            && let Some(frame) = frames.get_mut(surface)
        {
            if visible && let Some(border) = &frame.border {
                layers.extend(DesktopLayer::frame_shadow(
                    surface.get(),
                    border.clone(),
                    position,
                ));
            }
            let frame_pieces = DesktopLayer::retained_frame(
                surface.get(),
                if visible {
                    frame.layer.take_deltas()
                } else {
                    Vec::new()
                },
                frame.outer,
                position,
                visible,
                (veiled || content_style.is_some()).then_some(content_rect),
            );
            layers.extend(
                frame_pieces
                    .into_iter()
                    .map(|piece| match frame.border.as_ref() {
                        Some(border) => piece.with_frame_outline(border, position),
                        None => piece,
                    }),
            );
            if visible
                && (veiled || content_style.is_some())
                && let Some(border) = &frame.border
            {
                // Restore both the wider chrome fill and the outline, outside the aperture.
                // These patches stay inside the cutout, disjoint from the four frame strips.
                if let Some((_, clips)) = inherited_clip {
                    layers.extend(DesktopLayer::content_corners(
                        surface.get(),
                        border.clone(),
                        frame.outer,
                        position,
                        content_rect,
                        clips,
                    ));
                }
                layers.push(DesktopLayer::content_border(
                    surface.get(),
                    border.clone(),
                    frame.outer,
                    position,
                    content_rect,
                ));
            }
        }
        if visible
            && !veiled
            && let Some(style) = content_style
        {
            let mut backing = DesktopLayer::solid(
                DesktopLayerKey::ContentBackground(surface.get()),
                DesktopSceneKey::ContentBackground(surface.get()),
                style.background,
                content_rect,
            );
            if let Some((bounds, clips)) = inherited_clip {
                backing = backing.with_content_clip(bounds, clips);
            }
            layers.push(backing);
        }
        if visible
            && window_is_decorated(window)
            && window.chrome.is_none()
            && window.backend.is_some()
        {
            let icon_extent = SizeI {
                width: config.titlebar_height.clamp(1, 24),
                height: config.titlebar_height.clamp(1, 24),
            };
            for (index, name) in ["window.close", "window.maximize", "window.minimize"]
                .into_iter()
                .enumerate()
            {
                let Some((_, icon)) = icons.iter_mut().find(|(candidate, _)| candidate == name)
                else {
                    continue;
                };
                icon.prepare(icon_extent, now, false)?;
                layers.push(DesktopLayer::retained(
                    DesktopLayerKey::LegacyControl(surface.get(), index as u8),
                    DesktopSceneKey::LegacyControl(index as u8),
                    icon.take_deltas(),
                    icon_extent,
                    PointI {
                        x: position.x + outer.width
                            - config.window_border
                            - (index as i32 + 1) * (icon_extent.width + 4),
                        y: position.y
                            + config.window_border
                            + (config.titlebar_height - icon_extent.height) / 2,
                    },
                    true,
                ));
            }
        }
        // The live resize preview is a solid retained primitive. Keep the image scene and its
        // pending pixels intact, but do not upload or draw them underneath even a transparent veil.
        if visible && window.role == SurfaceRole::XdgToplevel && veiled {
            let mut preview = DesktopLayer::solid(
                DesktopLayerKey::ResizeVeil(surface.get()),
                DesktopSceneKey::ResizeVeil(surface.get()),
                content_style
                    .and_then(|style| style.resize_preview_color)
                    .unwrap_or(config.resize_preview_color),
                content_rect,
            );
            if let Some((bounds, clips)) = inherited_clip {
                preview = preview.with_content_clip(bounds, clips);
            }
            layers.push(preview);
        }
        let placement = surface_placement(window, position, config);
        let mut client = DesktopLayer::image(
            DesktopLayerKey::Surface(surface.get()),
            DesktopSceneKey::Surface(surface.get()),
            window.presentation.revision,
            if visible && !veiled {
                window.presentation.take_image_update()
            } else {
                DesktopImageUpdate::Unchanged
            },
            window.presentation.image_size,
            placement.target,
            placement.clip,
            window.presentation.alpha_mode,
            window.presentation.pixel_format,
            visible && !veiled,
        );
        if let Some((bounds, clips)) = inherited_clip {
            client = client.with_content_clip(bounds, clips);
        }
        layers.push(client);
    }

    // Drag-icon surfaces intentionally do not participate in ordinary window stacking, so retain
    // them from the surface map and add only the active icon as an output placement.
    for (surface, icon) in windows
        .iter_mut()
        .filter(|(_, window)| window.role == SurfaceRole::DragIcon)
    {
        let visible = !session_locked && drag_icon == Some(*surface);
        layers.push(DesktopLayer::image(
            DesktopLayerKey::DragIcon(surface.get()),
            DesktopSceneKey::DragIcon(surface.get()),
            icon.presentation.revision,
            if visible {
                icon.presentation.take_image_update()
            } else {
                DesktopImageUpdate::Unchanged
            },
            icon.presentation.image_size,
            RectI {
                x: drag_position.x.round() as i32,
                y: drag_position.y.round() as i32,
                width: icon.presentation.size.width,
                height: icon.presentation.size.height,
            },
            None,
            icon.presentation.alpha_mode,
            icon.presentation.pixel_format,
            visible,
        ));
    }

    for (index, widget) in widgets.iter_mut().enumerate() {
        let widget_extent = resolved_widget_extent(extent, widget.width, widget.height);
        let position = widget_position(extent, widget_extent, widget.anchor);
        if !session_locked {
            widget.layer.prepare(widget_extent, now, false)?;
        }
        layers.push(DesktopLayer::retained(
            DesktopLayerKey::Widget(index as u32),
            DesktopSceneKey::Widget(index as u32),
            if session_locked {
                Vec::new()
            } else {
                widget.layer.take_deltas()
            },
            widget_extent,
            position,
            !session_locked,
        ));
    }
    if let Some(cursor) = cursor {
        match cursor {
            CursorVisual::Image(cursor) => layers.push(DesktopLayer::image(
                DesktopLayerKey::Cursor,
                DesktopSceneKey::CursorImage,
                cursor_image_signature(cursor),
                DesktopImageUpdate::Full(Arc::from(cursor.rgba.as_slice())),
                cursor.size,
                RectI {
                    x: pointer_position.x.round() as i32 - cursor.hotspot.x,
                    y: pointer_position.y.round() as i32 - cursor.hotspot.y,
                    width: cursor.logical_size.width,
                    height: cursor.logical_size.height,
                },
                None,
                if cursor.premultiplied {
                    ImageAlphaMode::Premultiplied
                } else {
                    ImageAlphaMode::Straight
                },
                ImagePixelFormat::Rgba8,
                true,
            )),
            CursorVisual::Composed { source, size } => {
                let (scene, layer) = match source {
                    ComposedCursorSource::Pointer => {
                        let Some(pointer) = pointer_layer.as_mut() else {
                            return Ok(layers);
                        };
                        (DesktopSceneKey::ComposedPointer, pointer)
                    }
                    ComposedCursorSource::Icon(index) => {
                        let Some((_, icon)) = icons.get_mut(*index) else {
                            return Ok(layers);
                        };
                        (DesktopSceneKey::ComposedIcon(*index), icon)
                    }
                };
                layer.prepare(*size, now, false)?;
                layers.push(DesktopLayer::retained(
                    DesktopLayerKey::Cursor,
                    scene,
                    layer.take_deltas(),
                    *size,
                    PointI {
                        x: pointer_position.x.round() as i32,
                        y: pointer_position.y.round() as i32,
                    },
                    true,
                ));
            }
        }
    }

    // These source-only placements preserve backend scene state while a shared icon or composed
    // pointer is temporarily not visible. Deltas stay with the producing runtime until a visible
    // placement consumes them, so no renderer receives an update it cannot draw this frame.
    for index in 0_u8..3 {
        let source_extent = SizeI {
            width: 24,
            height: 24,
        };
        layers.push(DesktopLayer::retained(
            DesktopLayerKey::LegacyControlSource(index),
            DesktopSceneKey::LegacyControl(index),
            Vec::new(),
            source_extent,
            PointI::default(),
            false,
        ));
    }
    for index in 0..icons.len() {
        let source_extent = SizeI {
            width: 24,
            height: 24,
        };
        layers.push(DesktopLayer::retained(
            DesktopLayerKey::ComposedIconSource(index),
            DesktopSceneKey::ComposedIcon(index),
            Vec::new(),
            source_extent,
            PointI::default(),
            false,
        ));
    }
    if pointer_layer.is_some() {
        layers.push(DesktopLayer::retained(
            DesktopLayerKey::ComposedPointerSource,
            DesktopSceneKey::ComposedPointer,
            Vec::new(),
            config.pointer_extent,
            PointI::default(),
            false,
        ));
    }
    Ok(layers)
}

#[cfg(test)]
mod maximize_tests {
    use super::*;
    use crate::compose::{Component, RuntimeTarget, View, window_content_slot, window_frame};

    #[crate::component]
    struct TestFrame {
        #[input]
        title_height: f32,
    }

    impl Component for TestFrame {
        fn view(&self) -> impl View {
            window_frame().content_slot(window_content_slot().margin(crate::compose::Insets::new(
                self.title_height,
                0.0,
                0.0,
                0.0,
            )))
        }
    }

    #[cfg(all(feature = "desktop-xwayland", target_env = "gnu"))]
    #[test]
    fn native_and_x11_windows_use_the_same_composed_frame_template() {
        use super::super::client::maximize_preview_tests::test_window;
        let declaration = crate::application_host::Compositor::new()
            .window_frame(|_: WindowChromeModel| TestFrame { title_height: 37.0 })
            .background(TestFrame { title_height: 37.0 });
        let display = Display::new().unwrap();
        let wayland = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
        let mut registry = crate::xwayland::window::Windows::new(1, 16, 1, 2, 100).unwrap();
        let token = registry.begin_inspection(10).unwrap().unwrap();
        registry
            .finish_inspection(
                token,
                1,
                crate::xwayland::window::Geometry {
                    x: 100,
                    y: 100,
                    width: 640,
                    height: 480,
                    border: 0,
                },
                false,
                true,
            )
            .unwrap();
        let id = registry.get(10).unwrap().id;
        let native = WaylandSurfaceId::from_raw(20).unwrap();
        let x11 = WaylandSurfaceId::from_raw(21).unwrap();
        let popup = WaylandSurfaceId::from_raw(22).unwrap();
        let mut windows = BTreeMap::new();
        for (surface, backend, role) in [
            (
                native,
                Some(WindowBackend::Wayland),
                SurfaceRole::XdgToplevel,
            ),
            (x11, Some(WindowBackend::X11(id)), SurfaceRole::Xwayland),
            (popup, None, SurfaceRole::Xwayland),
        ] {
            let mut window = test_window(
                SizeI {
                    width: 640,
                    height: 480,
                },
                PointI { x: 100, y: 100 },
            );
            window.backend = backend;
            window.role = role;
            window.server_decorated = backend.is_some();
            window.frame_title = Some("Same application title".into());
            windows.insert(surface, window);
        }
        let mut frames = BTreeMap::new();
        refresh_window_frames(
            declaration.window_frame(),
            &mut frames,
            &mut windows,
            &wayland,
            &LinuxDesktopConfig::default(),
            AssetBundle::default(),
            &crate::AppIconProfile::default(),
            &EventNotifier::new("frame parity").unwrap(),
            0,
            crate::platform::ScaleFactor::new(1.5).unwrap(),
            RectI {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            &mut ConfigureScheduler::default(),
        )
        .unwrap();
        assert_eq!(frames.len(), 2);
        assert!(!frames.contains_key(&popup));
        assert_eq!(frames[&native].outer, frames[&x11].outer);
        assert_eq!(
            windows[&native].chrome_content_offset,
            windows[&x11].chrome_content_offset
        );
        assert_eq!(windows[&x11].chrome_content_offset.unwrap().y, 37);
        assert_eq!(
            frames[&native].snapshot.as_ref().unwrap().content.bounds,
            frames[&x11].snapshot.as_ref().unwrap().content.bounds
        );
    }

    #[test]
    fn maximize_uses_work_area_instead_of_legacy_client_size_and_restore_keeps_client_size() {
        for title_height in [24.0, 32.0, 48.0] {
            for scale in [1.0, 1.5, 3.0] {
                let original = SizeI {
                    width: 600,
                    height: 400,
                };
                let mut outer = original;
                let mut layer = Layer::new(
                    CompositionDriver::for_target(
                        TestFrame { title_height },
                        RuntimeTarget::Compositor,
                    ),
                    outer,
                    AssetBundle::default(),
                    crate::platform::ScaleFactor::new(scale).unwrap(),
                )
                .unwrap();
                let area = RectI {
                    x: 0,
                    y: 42,
                    width: 1280,
                    height: 758,
                };
                let legacy = SizeI {
                    width: 1272,
                    height: 718,
                };
                let snapshot =
                    layout_window_frame(&mut layer, &mut outer, legacy, Some(area), 0, true)
                        .unwrap();
                assert_eq!(
                    outer,
                    SizeI {
                        width: 1280,
                        height: 758
                    }
                );
                assert_eq!(snapshot.content.bounds.width, 1280.0);
                assert_eq!(snapshot.content.bounds.height, 758.0 - title_height);
                assert_eq!(snapshot.content.bounds.bottom(), 758.0);
                let restored =
                    layout_window_frame(&mut layer, &mut outer, original, None, 1, true).unwrap();
                assert_eq!(restored.content.bounds.width, original.width as f32);
                assert_eq!(restored.content.bounds.height, original.height as f32);
            }
        }
    }
}

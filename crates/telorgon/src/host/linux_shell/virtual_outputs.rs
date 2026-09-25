//! Host-owned virtual display state. Registration and scene ownership have one lifetime;
//! callers must withdraw capture authorization before retiring a display.
use super::{capture_scene::CaptureScene, capture_window::WindowCapture};
use crate::{
    foundation::{PointI, RectI, SizeI},
    host::application::{AppError, AppResult},
    integrations::wayland::{
        compositor::{
            NativeCompositor, OutputDescription, OutputMode, OutputState, OutputTransform,
        },
        server::Display,
    },
    platform::contracts::ScaleFactor,
    shell::{OutputId, WindowId, capture::CaptureLayout},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    num::NonZeroU32,
};

pub(super) struct VirtualOutput {
    pub label: String,
    pub position: PointI,
    pub layout: CaptureLayout,
    pub frame_rate: NonZeroU32,
    pub revision: u64,
    pub scene: Option<CaptureScene>,
    routes: Vec<(WindowId, RectI)>,
}
impl VirtualOutput {
    pub fn routes(&self) -> &[(WindowId, RectI)] {
        &self.routes
    }
}
#[derive(Default)]
pub(super) struct VirtualOutputs {
    outputs: BTreeMap<OutputId, VirtualOutput>,
    desktop_geometry: BTreeMap<WindowId, (PointI, SizeI)>,
    fullscreen_routes: BTreeMap<WindowId, Option<(OutputId, RectI)>>,
}
impl VirtualOutputs {
    /// Creates a real wl_output and initially black independent scene. No transport or
    /// capture authorization is created here. Up to eight displays share this owner.
    pub fn create<'display>(
        &mut self,
        compositor: &mut NativeCompositor<'display>,
        display: &'display Display,
        label: String,
        size: SizeI,
        frame_rate: NonZeroU32,
        position: PointI,
    ) -> AppResult<OutputId> {
        if self.outputs.len() >= 8 {
            return Err(AppError::new("virtual display limit reached"));
        }
        if label.trim().is_empty() || label.len() > 128 || label.chars().any(char::is_control) {
            return Err(AppError::new("invalid virtual display label"));
        }
        if size.width <= 0
            || size.height <= 0
            || size.width > 8192
            || size.height > 8192
            || frame_rate.get() > 240
        {
            return Err(AppError::new(
                "virtual display extent or cadence exceeds limits",
            ));
        }
        if position.x.checked_add(size.width).is_none()
            || position.y.checked_add(size.height).is_none()
        {
            return Err(AppError::new("virtual display position overflows"));
        }
        let layout = CaptureLayout::rgba8(
            NonZeroU32::new(size.width as u32).unwrap(),
            NonZeroU32::new(size.height as u32).unwrap(),
            size.width as u32 * 4,
        )
        .unwrap();
        let id = compositor
            .next_output_id()
            .ok_or_else(|| AppError::new("output identities exhausted"))?;
        let scene =
            CaptureScene::virtual_output(layout, std::iter::empty()).map_err(AppError::new)?;
        let state = OutputState::new(
            OutputDescription {
                name: format!("TELORGON-VIRTUAL-{id}"),
                description: label.clone(),
                make: "Telorgon".into(),
                model: "Virtual display".into(),
                physical_millimeters: SizeI::default(),
                logical_position: position,
                scale: ScaleFactor::new(1.0).unwrap(),
                transform: OutputTransform::Normal,
                modes: vec![OutputMode {
                    size,
                    refresh_millihertz: frame_rate.get() * 1000,
                    preferred: true,
                }],
            },
            0,
        )
        .map_err(|error| AppError::new(error.to_string()))?;
        compositor
            .add_output(display, id, state)
            .map_err(|error| AppError::new(error.to_string()))?;
        if let Err(error) = compositor.set_output_automatic_membership(id, false) {
            let _ = compositor.remove_output(id);
            return Err(AppError::new(error.to_string()));
        }
        let id = OutputId::from_raw(u64::from(id)).unwrap();
        self.outputs.insert(
            id,
            VirtualOutput {
                label,
                position,
                layout,
                frame_rate,
                revision: 1,
                scene: Some(scene),
                routes: Vec::new(),
            },
        );
        Ok(id)
    }
    pub fn get(&self, id: OutputId) -> Option<&VirtualOutput> {
        self.outputs.get(&id)
    }
    pub fn iter(&self) -> impl Iterator<Item = (OutputId, &VirtualOutput)> {
        self.outputs.iter().map(|(&id, output)| (id, output))
    }
    /// Back-to-front placement policy. A window may belong to only one virtual display.
    /// The host applies native membership/configures and excludes routed windows from its
    /// physical desktop. Rendering itself uses only the listed isolated window snapshots.
    pub fn route(&mut self, id: OutputId, routes: Vec<(WindowId, RectI)>) -> AppResult<()> {
        if !self.outputs.contains_key(&id) {
            return Err(AppError::new("unknown virtual display"));
        }
        if routes.len() > 64 {
            return Err(AppError::new("virtual display window limit reached"));
        }
        let mut unique = BTreeSet::new();
        for &(window, rect) in &routes {
            if !unique.insert(window)
                || rect.width <= 0
                || rect.height <= 0
                || rect.width > 8192
                || rect.height > 8192
                || rect.x.checked_add(rect.width).is_none()
                || rect.y.checked_add(rect.height).is_none()
            {
                return Err(AppError::new("invalid virtual window placement"));
            }
            if self.outputs.iter().any(|(&other, output)| {
                other != id
                    && output
                        .routes
                        .iter()
                        .any(|(candidate, _)| *candidate == window)
            }) {
                return Err(AppError::new(
                    "window already belongs to another virtual display",
                ));
            }
        }
        let output = self.outputs.get_mut(&id).unwrap();
        if output.routes != routes {
            output.revision = output
                .revision
                .checked_add(1)
                .ok_or_else(|| AppError::new("virtual display revision exhausted"))?;
            output.routes = routes;
            // Never retain old routed content while awaiting the next scene rebuild.
            output.scene = Some(
                CaptureScene::virtual_output(output.layout, std::iter::empty())
                    .map_err(AppError::new)?,
            );
        }
        Ok(())
    }
    /// A fullscreen request only moves its own window. Stage every changed display before
    /// committing, so capacity/revision failure cannot strand it between outputs.
    pub fn fullscreen(
        &mut self,
        window: WindowId,
        enabled: bool,
        requested: Option<u32>,
        geometry: (PointI, SizeI),
    ) -> AppResult<Option<SizeI>> {
        let previous = self.outputs.iter().find_map(|(&id, output)| {
            output
                .routes
                .iter()
                .find_map(|(candidate, rect)| (*candidate == window).then_some((id, *rect)))
        });
        let destination = if enabled {
            let target = requested
                .and_then(|raw| OutputId::from_raw(u64::from(raw)))
                .or_else(|| previous.map(|(id, _)| id));
            target.and_then(|id| {
                self.outputs.get(&id).map(|output| {
                    (
                        id,
                        RectI {
                            x: 0,
                            y: 0,
                            width: output.layout.width() as i32,
                            height: output.layout.height() as i32,
                        },
                    )
                })
            })
        } else {
            let Some(saved) = self.fullscreen_routes.get(&window) else {
                return Ok(None);
            };
            saved.filter(|(id, _)| self.outputs.contains_key(id))
        };
        let mut staged = Vec::new();
        for (&id, output) in &self.outputs {
            let mut routes = output.routes.clone();
            routes.retain(|(candidate, _)| *candidate != window);
            if let Some((target, rect)) = destination.filter(|(target, _)| *target == id) {
                let _ = target;
                routes.push((window, rect));
            }
            if routes == output.routes {
                continue;
            }
            if routes.len() > 64 {
                return Err(AppError::new("virtual display window limit reached"));
            }
            let revision = output
                .revision
                .checked_add(1)
                .ok_or_else(|| AppError::new("virtual display revision exhausted"))?;
            let scene = CaptureScene::virtual_output(output.layout, std::iter::empty())
                .map_err(AppError::new)?;
            staged.push((id, routes, revision, scene));
        }
        if enabled && destination.is_some() {
            self.desktop_geometry.entry(window).or_insert(geometry);
            self.fullscreen_routes.entry(window).or_insert(previous);
        } else {
            self.fullscreen_routes.remove(&window);
        }
        for (id, routes, revision, scene) in staged {
            let output = self.outputs.get_mut(&id).unwrap();
            output.routes = routes;
            output.revision = revision;
            output.scene = Some(scene);
        }
        Ok(destination.map(|(_, rect)| SizeI {
            width: rect.width,
            height: rect.height,
        }))
    }

    pub fn window_output(&self, window: WindowId) -> Option<OutputId> {
        self.outputs.iter().find_map(|(&id, output)| {
            output
                .routes
                .iter()
                .any(|(candidate, _)| *candidate == window)
                .then_some(id)
        })
    }
    /// Reconcile roots and popup/subsurface families before physical input/rendering.
    pub fn sync_membership(
        &self,
        compositor: &mut NativeCompositor<'_>,
        windows: &mut BTreeMap<super::WaylandSurfaceId, super::ClientWindow>,
    ) -> AppResult<bool> {
        let assignments: Vec<_> = windows
            .keys()
            .copied()
            .map(|surface| {
                let mut current = Some(surface);
                let mut output = None;
                let mut resolved = false;
                for _ in 0..=windows.len() {
                    let Some(window) = current.and_then(|id| windows.get(&id)) else {
                        break;
                    };
                    if let Some(id) = window.desktop_id {
                        output = self.window_output(id);
                        if window.backend.is_some() {
                            resolved = true;
                            break;
                        }
                    }
                    current = window.parent;
                }
                if !resolved {
                    output = windows[&surface]
                        .virtual_output
                        .filter(|id| self.outputs.contains_key(id));
                }
                (surface, output)
            })
            .collect();
        let mut changed = false;
        for (surface, output) in assignments {
            let window = windows.get_mut(&surface).unwrap();
            if window.virtual_output == output {
                continue;
            }
            if compositor.core().world.surface(surface).is_none() {
                continue;
            }
            let native = output.map(|id| [u32::try_from(id.get()).expect("owned native output")]);
            compositor
                .set_surface_outputs(surface, native.as_ref().map(|ids| ids.as_slice()))
                .map_err(|error| AppError::new(error.to_string()))?;
            window.virtual_output = output;
            window.motion_input = None;
            changed = true;
        }
        Ok(changed)
    }

    /// Route extents drive client configures; the renderer can scale content while clients
    /// acknowledge the new size. Returning a window restores its prior desktop geometry.
    pub fn sync_geometry(
        &mut self,
        windows: &mut BTreeMap<super::WaylandSurfaceId, super::ClientWindow>,
        scheduler: &mut super::ConfigureScheduler,
    ) -> bool {
        let mut changed = false;
        let live: BTreeSet<_> = windows
            .values()
            .filter_map(|window| window.desktop_id)
            .collect();
        self.desktop_geometry.retain(|id, _| live.contains(id));
        self.fullscreen_routes.retain(|id, _| live.contains(id));
        for (&surface, window) in windows
            .iter_mut()
            .filter(|(_, window)| window.backend.is_some())
        {
            let Some(id) = window.desktop_id else {
                continue;
            };
            let placement = self
                .outputs
                .values()
                .flat_map(|output| output.routes.iter())
                .find_map(|(candidate, rect)| (*candidate == id).then_some(*rect));
            let target = if let Some(rect) = placement {
                self.desktop_geometry
                    .entry(id)
                    .or_insert((window.position, window.requested_size));
                Some((
                    window.position,
                    SizeI {
                        width: rect.width,
                        height: rect.height,
                    },
                ))
            } else {
                if self.fullscreen_routes.remove(&id).is_some() {
                    window.fullscreen = false;
                    window.restore_geometry = None;
                    scheduler.schedule_final(surface, window.requested_size);
                    changed = true;
                }
                self.desktop_geometry.remove(&id)
            };
            let Some((position, size)) = target else {
                continue;
            };
            if window.position == position && window.requested_size == size {
                continue;
            }
            window.position = position;
            window.requested_size = size;
            window.last_policy_request = None;
            window.motion_input = None;
            window.native_configure.resize_anchor = None;
            window.native_configure.resize_final = None;
            #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
            {
                window.resize_preview = Default::default();
            }
            scheduler.schedule_final(surface, size);
            changed = true;
        }
        changed
    }

    pub fn refresh_from_host(
        &mut self,
        windows: &BTreeMap<super::WaylandSurfaceId, super::ClientWindow>,
        layers: &[super::scene::ShellLayer],
        scale: ScaleFactor,
    ) -> AppResult<bool> {
        let selected: BTreeSet<_> = self
            .outputs
            .values()
            .flat_map(|output| output.routes.iter().map(|(window, _)| *window))
            .collect();
        let snapshots = selected
            .into_iter()
            .filter_map(|window| {
                super::capture_window::prepare(window, windows, layers, scale)
                    .map(|snapshot| (window, snapshot))
            })
            .collect();
        self.refresh(&snapshots)
    }
    pub fn refresh(&mut self, windows: &BTreeMap<WindowId, WindowCapture>) -> AppResult<bool> {
        let mut changed = false;
        for output in self.outputs.values_mut() {
            let scene = CaptureScene::virtual_output(
                output.layout,
                output.routes.iter().filter_map(|(window, target)| {
                    windows.get(window).map(|window| (window, *target))
                }),
            );
            let scene = match scene {
                Ok(scene) => scene,
                Err(error) => {
                    // Suppress delivery until the host rebuilds or retires this source.
                    output.scene = None;
                    return Err(AppError::new(error));
                }
            };
            if output.scene.as_ref() != Some(&scene) {
                let Some(revision) = output.revision.checked_add(1) else {
                    output.scene = None;
                    return Err(AppError::new("virtual display revision exhausted"));
                };
                output.revision = revision;
                output.scene = Some(scene);
                changed = true;
            }
        }
        Ok(changed)
    }
    /// Native retirement also cancels direct capture. A notification error can occur after
    /// withdrawal committed; discard owned content once the native snapshot no longer has it.
    pub fn remove(
        &mut self,
        compositor: &mut NativeCompositor<'_>,
        id: OutputId,
    ) -> AppResult<bool> {
        if !self.outputs.contains_key(&id) {
            return Ok(false);
        }
        let native =
            u32::try_from(id.get()).map_err(|_| AppError::new("invalid native output identity"))?;
        let result = compositor.remove_output(native);
        if !compositor.output_snapshot().outputs().contains_key(&native) {
            self.outputs.remove(&id);
        }
        result.map_err(|error| AppError::new(error.to_string()))
    }
}

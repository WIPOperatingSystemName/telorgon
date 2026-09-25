//! Isolated client placements, collected before desktop motion/backdrop composition and culling.
use super::scene::{ShellLayer, ShellLayerContent, ShellLayerKey, ShellPlacement, ShellSceneKey};
use super::{ClientWindow, SurfaceRole, WaylandSurfaceId};
use crate::foundation::{PointI, RectI};
use crate::platform::contracts::ScaleFactor;
use crate::shell::{WindowId, capture::CaptureLayout};
use std::{collections::BTreeMap, num::NonZeroU32};

#[derive(Clone, PartialEq)]
pub(super) struct WindowCapture {
    pub window: WindowId,
    pub layout: CaptureLayout,
    /// Physical desktop origin for translating an embedded cursor into the capture target.
    pub origin: PointI,
    desktop_cursor: bool,
    pub placements: Vec<ShellPlacement>,
    content_versions: Vec<(u32, u64)>,
}

impl WindowCapture {
    pub fn surface_revisions(&self) -> &[(u32, u64)] {
        &self.content_versions
    }
    pub fn into_scene(self) -> super::capture_scene::CaptureScene {
        super::capture_scene::CaptureScene {
            layout: self.layout,
            desktop_cursor_origin: self.desktop_cursor.then_some(self.origin),
            placements: self.placements,
            sampled: self.content_versions,
        }
    }
}

pub(super) fn prepare(
    selected: WindowId,
    windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
    layers: &[ShellLayer],
    scale: ScaleFactor,
) -> Option<WindowCapture> {
    let (&root, window) = windows
        .iter()
        .find(|(_, w)| w.desktop_id == Some(selected) && w.backend.is_some())?;
    if window.minimized || !window.presentation.content_ready || window.presentation.revision == 0 {
        return None;
    }
    let mut placements = Vec::new();
    let mut content_versions = Vec::new();
    let mut bounds = None;
    let mut has_root = false;
    // Preserve client stacking, but ignore visibility derived from desktop occlusion. Only
    // Surface image scenes are admitted: no chrome, shadows, motion snapshots or glass samples.
    for layer in layers {
        let ShellLayerKey::Surface(raw) = layer.key else {
            continue;
        };
        let surface = WaylandSurfaceId::from_raw(raw)?;
        if !belongs_to(windows, surface, root) {
            continue;
        }
        let client = windows.get(&surface)?;
        if client.minimized
            || !client.presentation.content_ready
            || client.presentation.revision == 0
        {
            continue;
        }
        let ShellLayerContent::Image {
            scene: ShellSceneKey::Surface(scene),
            content_version,
            ..
        } = &layer.content
        else {
            return None;
        };
        if *scene != raw || layer.glass.is_some() {
            return None;
        }
        let target = scale.physical_rect(layer.target);
        let visible = match layer.clip {
            Some(clip) => intersect(target, scale.physical_rect(clip)),
            None => valid(target),
        };
        let Some(visible) = visible else {
            continue;
        };
        bounds = Some(match bounds {
            None => visible,
            Some(previous) => union(previous, visible)?,
        });
        has_root |= surface == root;
        content_versions.push((raw, *content_version));
        placements.push(ShellPlacement {
            key: layer.key,
            scene: ShellSceneKey::Surface(raw),
            target,
            clip: Some(visible),
            rounded_clips: [None, None],
        });
    }
    if !has_root {
        return None;
    }
    let bounds = bounds?;
    if bounds.width > 8192 || bounds.height > 8192 {
        return None;
    }
    let layout = CaptureLayout::rgba8(
        NonZeroU32::new(bounds.width as u32)?,
        NonZeroU32::new(bounds.height as u32)?,
        (bounds.width as u32).checked_mul(4)?,
    )?;
    for placement in &mut placements {
        placement.target = translate(placement.target, bounds)?;
        placement.clip = Some(translate(placement.clip?, bounds)?);
    }
    Some(WindowCapture {
        window: selected,
        layout,
        origin: PointI {
            x: bounds.x,
            y: bounds.y,
        },
        placements,
        content_versions,
        desktop_cursor: window.virtual_output.is_none(),
    })
}

fn belongs_to(
    windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
    mut surface: WaylandSurfaceId,
    root: WaylandSurfaceId,
) -> bool {
    for _ in 0..=windows.len() {
        if surface == root {
            return true;
        }
        let Some(window) = windows.get(&surface) else {
            return false;
        };
        // A parented toplevel is a separate selectable window. Unmanaged X11 surfaces require
        // a separate ownership audit and must not be inferred from coordinates or stacking.
        if !matches!(window.role, SurfaceRole::Subsurface | SurfaceRole::XdgPopup) {
            return false;
        }
        let Some(parent) = window.parent else {
            return false;
        };
        surface = parent;
    }
    false
}
fn valid(rect: RectI) -> Option<RectI> {
    (rect.width > 0
        && rect.height > 0
        && rect.x.checked_add(rect.width).is_some()
        && rect.y.checked_add(rect.height).is_some())
    .then_some(rect)
}
fn intersect(a: RectI, b: RectI) -> Option<RectI> {
    let (a, b) = (valid(a)?, valid(b)?);
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    valid(RectI {
        x,
        y,
        width: a.right().min(b.right()).checked_sub(x)?,
        height: a.bottom().min(b.bottom()).checked_sub(y)?,
    })
}
fn union(a: RectI, b: RectI) -> Option<RectI> {
    let (a, b) = (valid(a)?, valid(b)?);
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    valid(RectI {
        x,
        y,
        width: a.right().max(b.right()).checked_sub(x)?,
        height: a.bottom().max(b.bottom()).checked_sub(y)?,
    })
}
fn translate(rect: RectI, origin: RectI) -> Option<RectI> {
    Some(RectI {
        x: rect.x.checked_sub(origin.x)?,
        y: rect.y.checked_sub(origin.y)?,
        ..rect
    })
}

#[cfg(test)]
mod tests {
    use super::super::{client::maximize_preview_tests::test_window, scene::ShellImageUpdate};
    use super::*;
    use crate::{
        core::SizeI,
        render::{ImageAlphaMode, ImagePixelFormat},
    };
    fn surface(raw: u32) -> WaylandSurfaceId {
        WaylandSurfaceId::from_raw(raw).unwrap()
    }
    fn id(generation: u32) -> WindowId {
        WindowId::new(
            NonZeroU32::new(1).unwrap(),
            NonZeroU32::new(generation).unwrap(),
        )
    }
    fn layer(raw: u32, rect: RectI) -> ShellLayer {
        ShellLayer::image(
            ShellLayerKey::Surface(raw),
            ShellSceneKey::Surface(raw),
            1,
            ShellImageUpdate::Unchanged,
            SizeI {
                width: rect.width,
                height: rect.height,
            },
            rect,
            None,
            ImageAlphaMode::Opaque,
            ImagePixelFormat::Rgba8,
            false,
        )
    }
    fn window() -> ClientWindow {
        test_window(
            SizeI {
                width: 100,
                height: 80,
            },
            PointI { x: 100, y: 200 },
        )
    }
    fn root_rect() -> RectI {
        RectI {
            x: 100,
            y: 200,
            width: 100,
            height: 80,
        }
    }
    #[test]
    fn isolated_capture_includes_owned_popups_but_excludes_other_toplevels_and_desktop() {
        let mut root = window();
        root.desktop_id = Some(id(1));
        let mut popup = window();
        popup.backend = None;
        popup.role = SurfaceRole::XdgPopup;
        popup.parent = Some(surface(1));
        let mut subsurface = window();
        subsurface.backend = None;
        subsurface.role = SurfaceRole::Subsurface;
        subsurface.parent = Some(surface(2));
        let mut other = window();
        other.parent = Some(surface(1));
        let windows = BTreeMap::from([
            (surface(1), root),
            (surface(2), popup),
            (surface(3), subsurface),
            (surface(4), other),
        ]);
        let layers = vec![
            ShellLayer::solid(
                ShellLayerKey::Background,
                ShellSceneKey::Background,
                crate::foundation::ColorRgba8::rgba(255, 0, 0, 255),
                root_rect(),
            ),
            layer(1, root_rect()),
            layer(4, root_rect()),
            layer(
                2,
                RectI {
                    x: 180,
                    y: 190,
                    width: 60,
                    height: 40,
                },
            ),
            layer(
                3,
                RectI {
                    x: 190,
                    y: 200,
                    width: 10,
                    height: 10,
                },
            ),
        ];
        let capture = prepare(id(1), &windows, &layers, ScaleFactor::new(1.0).unwrap()).unwrap();
        assert_eq!(capture.window, id(1));
        assert_eq!(capture.origin, PointI { x: 100, y: 190 });
        assert_eq!((capture.layout.width(), capture.layout.height()), (140, 90));
        assert_eq!(
            capture.placements.iter().map(|p| p.key).collect::<Vec<_>>(),
            vec![
                ShellLayerKey::Surface(1),
                ShellLayerKey::Surface(2),
                ShellLayerKey::Surface(3)
            ]
        );
        assert_eq!(
            capture.placements[0].target,
            RectI {
                x: 0,
                y: 10,
                width: 100,
                height: 80
            }
        );
        assert!(
            capture
                .placements
                .iter()
                .all(|p| p.rounded_clips == [None, None])
        );
        // Layer visibility is false for this entire fixture: desktop culling cannot erase capture.
        assert!(layers.iter().skip(1).all(|layer| !layer.visible));
    }
    #[test]
    fn stale_unmapped_and_minimized_roots_cannot_produce_a_capture() {
        let mut root = window();
        root.desktop_id = Some(id(2));
        let mut windows = BTreeMap::from([(surface(1), root)]);
        let layers = vec![layer(1, root_rect())];
        let scale = ScaleFactor::new(1.25).unwrap();
        assert!(prepare(id(1), &windows, &layers, scale).is_none());
        let capture = prepare(id(2), &windows, &layers, scale).unwrap();
        assert_eq!(
            (capture.layout.width(), capture.layout.height()),
            (125, 100)
        );
        windows.get_mut(&surface(1)).unwrap().minimized = true;
        assert!(prepare(id(2), &windows, &layers, scale).is_none());
        windows.get_mut(&surface(1)).unwrap().minimized = false;
        windows
            .get_mut(&surface(1))
            .unwrap()
            .presentation
            .content_ready = false;
        assert!(prepare(id(2), &windows, &layers, scale).is_none());
    }
    #[test]
    fn malformed_ownership_and_foreign_scene_references_fail_closed() {
        let mut root = window();
        root.desktop_id = Some(id(1));
        let mut cyclic = window();
        cyclic.role = SurfaceRole::Subsurface;
        cyclic.parent = Some(surface(2));
        let windows = BTreeMap::from([(surface(1), root), (surface(2), cyclic)]);
        assert!(!belongs_to(&windows, surface(2), surface(1)));
        let mut wrong = layer(1, root_rect());
        if let ShellLayerContent::Image { scene, .. } = &mut wrong.content {
            *scene = ShellSceneKey::Surface(2);
        }
        assert!(prepare(id(1), &windows, &[wrong], ScaleFactor::new(1.0).unwrap()).is_none());
        assert!(
            union(
                RectI {
                    x: i32::MIN,
                    y: 0,
                    width: 1,
                    height: 1
                },
                RectI {
                    x: i32::MAX - 1,
                    y: 0,
                    width: 1,
                    height: 1
                }
            )
            .is_none()
        );
        assert!(
            valid(RectI {
                x: i32::MAX,
                y: 0,
                width: 1,
                height: 1
            })
            .is_none()
        );
    }

    #[test]
    fn offscreen_content_changes_invalidate_capture_independently_of_desktop_damage() {
        let mut root = window();
        root.desktop_id = Some(id(1));
        let windows = BTreeMap::from([(surface(1), root), (surface(2), window())]);
        let rect = RectI {
            x: -5000,
            y: -5000,
            width: 100,
            height: 80,
        };
        let mut layers = vec![layer(1, rect), layer(2, rect)];
        let scale = ScaleFactor::new(1.0).unwrap();
        let first = prepare(id(1), &windows, &layers, scale).unwrap();
        if let ShellLayerContent::Image {
            content_version, ..
        } = &mut layers[1].content
        {
            *content_version = 2;
        }
        assert!(first == prepare(id(1), &windows, &layers, scale).unwrap());
        if let ShellLayerContent::Image {
            content_version, ..
        } = &mut layers[0].content
        {
            *content_version = 2;
        }
        assert!(first != prepare(id(1), &windows, &layers, scale).unwrap());
    }

    #[cfg(all(feature = "shell-xwayland", target_env = "gnu"))]
    #[test]
    fn managed_x11_capture_excludes_unverified_override_redirect_associations() {
        let mut root = window();
        root.desktop_id = Some(id(1));
        root.role = SurfaceRole::Xwayland;
        root.backend = Some(super::super::WindowBackend::X11(
            crate::integrations::x11::association::XWindow {
                generation: 1,
                xid: 7,
                incarnation: 2,
            },
        ));
        let mut unmanaged = window();
        unmanaged.role = SurfaceRole::Xwayland;
        unmanaged.backend = None;
        unmanaged.parent = Some(surface(1));
        let windows = BTreeMap::from([(surface(1), root), (surface(2), unmanaged)]);
        let capture = prepare(
            id(1),
            &windows,
            &[layer(1, root_rect()), layer(2, root_rect())],
            ScaleFactor::new(1.0).unwrap(),
        )
        .unwrap();
        assert_eq!(capture.placements.len(), 1);
        assert_eq!(capture.placements[0].scene, ShellSceneKey::Surface(1));
        assert!(
            prepare(
                id(2),
                &windows,
                &[layer(1, root_rect())],
                ScaleFactor::new(1.0).unwrap()
            )
            .is_none()
        );
    }
}

//! Protocol-neutral window metadata, chrome roles, actions, and layout-derived hit regions.

mod motion;
mod decoration;
pub use decoration::*;

pub use motion::{
    ContentFade, GeometryMotion, Minimize, Spring, WindowMotion, WindowTween, tween_ms,
};

use crate::assets::Icon;
use crate::foundation::{ColorRgba8, EdgeInsets, RectF};
use crate::ui::layout::LayoutEngine;
use crate::graphics::render::ImageId;
use crate::ui::{MountedUi, UiNodeId};

/// Whole-window resize placeholder, independent of who decorates the live client.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ResizePreviewDesign {
    pub fill: crate::Fill,
    /// Border painted inward over the fill, in logical units. Zero widths omit the border.
    pub border: crate::ui::Border,
    /// Floating preview radius when no server frame supplies a contour, in logical pixels.
    /// Maximized, tiled, and fullscreen previews remain square.
    pub corner_radius: f32,
}

impl ResizePreviewDesign {
    pub const fn new(fill: crate::Fill) -> Self {
        Self {
            fill,
            border: crate::ui::Border::all(0.0, ColorRgba8::rgba(0, 0, 0, 0)),
            corner_radius: 0.0,
        }
    }

    pub(crate) fn border_is_valid(self) -> bool {
        [
            self.border.top,
            self.border.right,
            self.border.bottom,
            self.border.left,
        ]
        .iter()
        .all(|side| side.width.is_finite() && side.width >= 0.0)
    }
}

impl From<crate::Fill> for ResizePreviewDesign {
    fn from(fill: crate::Fill) -> Self {
        Self::new(fill)
    }
}

/// Separate backing for an externally supplied client surface in a compositor-owned frame.
///
/// The host cuts the frame decoration out of the content slot, then paints this backing once
/// beneath the client. During resize it replaces both with the preview, so preview transparency
/// reveals lower desktop layers, never the stale client or this backing. Input regions are
/// unaffected. The corner radius shapes the shell-owned backing in addition to the composed
/// frame's inner border contour. Both start at the window's inner top edge, not the
/// content/title-bar seam. Application pixels retain their own alpha and corner shape;
/// the host constrains them only to the rectangular content slot. Popups retain independent bounds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowContentStyle {
    /// Straight RGBA color beneath the client; alpha zero removes the content backing.
    pub background: ColorRgba8,
    /// Finite, nonnegative content-aperture radius in logical pixels, anchored at the window top.
    pub corner_radius: f32,
    /// Whole-window resize-placeholder appearance, including the title bar. `None` inherits the host.
    pub resize_preview: Option<crate::ResizePreviewDesign>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct WindowEdgeMask(u8);

impl WindowEdgeMask {
    pub const NONE: Self = Self(0);
    pub const TOP: Self = Self(1 << 0);
    pub const RIGHT: Self = Self(1 << 1);
    pub const BOTTOM: Self = Self(1 << 2);
    pub const LEFT: Self = Self(1 << 3);
    pub const ALL: Self = Self(Self::TOP.0 | Self::RIGHT.0 | Self::BOTTOM.0 | Self::LEFT.0);

    pub const fn contains(self, edges: Self) -> bool {
        self.0 & edges.0 == edges.0
    }

    pub const fn union(self, edges: Self) -> Self {
        Self(self.0 | edges.0)
    }

    pub const fn intersection(self, edges: Self) -> Self {
        Self(self.0 & edges.0)
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl std::ops::BitOr for WindowEdgeMask {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        self.union(rhs)
    }
}

impl std::ops::BitOrAssign for WindowEdgeMask {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = self.union(rhs);
    }
}

/// Output/tile adjacency and resize authority for one tiled toplevel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct WindowTilingState {
    pub edges: WindowEdgeMask,
    pub resizable_edges: WindowEdgeMask,
}

impl WindowTilingState {
    pub const fn new(edges: WindowEdgeMask, resizable_edges: WindowEdgeMask) -> Self {
        Self {
            edges,
            resizable_edges,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum WindowChromeState {
    #[default]
    Normal,
    Maximized,
    Fullscreen,
    Tiled,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct WindowChromeCapabilities {
    pub close: bool,
    pub minimize: bool,
    pub maximize: bool,
    pub move_window: bool,
    pub resize: bool,
    pub system_menu: bool,
}

impl WindowChromeCapabilities {
    pub const MANAGED_TOPLEVEL: Self = Self {
        close: true,
        minimize: true,
        maximize: true,
        move_window: true,
        resize: true,
        system_menu: true,
    };
}

/// Immutable input supplied to one compositor-owned frame composition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowChromeModel {
    pub window_id: u64,
    /// Managed shell identity used to resolve application artwork through the catalog.
    pub desktop_window_id: Option<crate::shell::WindowId>,
    pub title: String,
    /// Whether this server frame template shows its title bar and controls.
    pub title_bar_visible: bool,
    pub app_icon: Option<Icon>,
    pub app_icon_name: Option<String>,
    pub app_icon_image: Option<ImageId>,
    pub state: WindowChromeState,
    pub tiling: Option<WindowTilingState>,
    pub active: bool,
    pub capabilities: WindowChromeCapabilities,
}

impl WindowChromeModel {
    pub fn new(window_id: u64, title: impl Into<String>) -> Self {
        Self {
            window_id,
            desktop_window_id: None,
            title: title.into(),
            title_bar_visible: true,
            app_icon: None,
            app_icon_name: None,
            app_icon_image: None,
            state: WindowChromeState::Normal,
            tiling: None,
            active: false,
            capabilities: WindowChromeCapabilities::MANAGED_TOPLEVEL,
        }
    }

    pub const fn title_bar_visible(mut self, visible: bool) -> Self {
        self.title_bar_visible = visible;
        self
    }

    pub const fn app_icon(mut self, icon: Icon) -> Self {
        self.app_icon = Some(icon);
        self
    }

    pub fn app_icon_name(mut self, name: impl Into<String>) -> Self {
        self.app_icon_name = Some(name.into());
        self
    }

    pub const fn app_icon_image(mut self, image: ImageId) -> Self {
        self.app_icon_image = Some(image);
        self
    }

    pub const fn state(mut self, state: WindowChromeState) -> Self {
        self.state = state;
        if !matches!(state, WindowChromeState::Tiled) {
            self.tiling = None;
        }
        self
    }

    pub const fn tiling(mut self, tiling: WindowTilingState) -> Self {
        self.state = WindowChromeState::Tiled;
        self.tiling = Some(tiling);
        self
    }

    pub const fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub const fn capabilities(mut self, capabilities: WindowChromeCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WindowResizeEdge {
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
    TopLeft,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WindowAction {
    Close,
    Minimize,
    ToggleMaximize,
    BeginMove,
    BeginResize(WindowResizeEdge),
    ShowSystemMenu,
}

/// Stable identity for a frame-local shell action explicitly registered by the compositor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ShellActionId(u64);

impl ShellActionId {
    pub const fn named(name: &str) -> Self {
        let bytes = name.as_bytes();
        let mut hash = 0xcbf29ce484222325_u64;
        let mut index = 0;
        while index < bytes.len() {
            hash ^= bytes[index] as u64;
            hash = hash.wrapping_mul(0x100000001b3);
            index += 1;
        }
        Self(hash)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// Semantic role attached to a composed frame node; geometry remains owned by normal layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WindowChromeRole {
    Frame,
    Content,
    Title,
    AppIcon,
    DragRegion,
    Action(WindowAction),
    ShellAction(ShellActionId),
}

/// Hit-test tuning attached to one semantic chrome region.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WindowChromeHitSpec {
    pub hit_slop: EdgeInsets,
    /// Opt-in resize contour: keep the frame border and this outward tolerance, not its interior.
    pub frame_border_outset: Option<EdgeInsets>,
    pub priority: u16,
}

impl WindowChromeHitSpec {
    pub const fn new(hit_slop: EdgeInsets, priority: u16) -> Self {
        Self {
            hit_slop,
            priority,
            frame_border_outset: None,
        }
    }

    pub const fn for_role(role: WindowChromeRole) -> Self {
        let priority = match role {
            WindowChromeRole::Action(WindowAction::BeginResize(_)) => 200,
            WindowChromeRole::Action(WindowAction::BeginMove) | WindowChromeRole::DragRegion => 100,
            WindowChromeRole::Action(_) | WindowChromeRole::ShellAction(_) => 300,
            WindowChromeRole::Title | WindowChromeRole::AppIcon => 10,
            WindowChromeRole::Frame | WindowChromeRole::Content => 0,
        };
        Self {
            hit_slop: EdgeInsets::ZERO,
            frame_border_outset: None,
            priority,
        }
    }

    pub const fn hit_slop(mut self, hit_slop: EdgeInsets) -> Self {
        self.hit_slop = hit_slop;
        self
    }

    pub const fn priority(mut self, priority: u16) -> Self {
        self.priority = priority;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowChromeRegion {
    pub node: UiNodeId,
    pub role: WindowChromeRole,
    pub bounds: RectF,
    pub hit_bounds: RectF,
    /// Optional geometric constraints in frame coordinates. Ordinary custom regions stay rectangular.
    pub hit_clips: [Option<crate::graphics::render::RoundedClip>; 2],
    pub priority: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowChromeSnapshot {
    pub frame: WindowChromeRegion,
    pub content: WindowChromeRegion,
    pub regions: Vec<WindowChromeRegion>,
}

impl WindowChromeSnapshot {
    pub fn derive(ui: &MountedUi, layout: &LayoutEngine) -> Result<Self, WindowChromeError> {
        let mut frame = None;
        let mut content = None;
        let mut regions = Vec::new();
        for (node, role) in ui.window_chrome_roles.iter() {
            let Some(computed) = layout.computed(node) else {
                continue;
            };
            let hit = ui
                .window_chrome_hit_specs
                .get(node)
                .copied()
                .unwrap_or_else(|| WindowChromeHitSpec::for_role(*role));
            let region = WindowChromeRegion {
                node,
                role: *role,
                bounds: computed.border_rect,
                hit_bounds: outset(computed.border_rect, hit.hit_slop),
                hit_clips: [None; 2],
                priority: hit.priority,
            };
            match role {
                WindowChromeRole::Frame => {
                    if frame.replace(region).is_some() {
                        return Err(WindowChromeError::MultipleFrames);
                    }
                }
                WindowChromeRole::Content => {
                    if content.replace(region).is_some() {
                        return Err(WindowChromeError::MultipleContentSlots);
                    }
                }
                _ => regions.push(region),
            }
        }
        let frame = frame.ok_or(WindowChromeError::MissingFrame)?;
        let decoration = ui
            .box_styles
            .get(frame.node)
            .cloned()
            .unwrap_or_default()
            .decoration;
        let outer = crate::graphics::render::RoundedClip::new(frame.bounds, decoration.corner_radii);
        let inner = outer.inset(decoration.border);
        for region in &mut regions {
            if let Some(outset) = ui
                .window_chrome_hit_specs
                .get(region.node)
                .and_then(|spec| spec.frame_border_outset)
            {
                region.hit_clips = [Some(outer.outset(outset)), Some(inner.inverse())];
                // Oversized corner metrics must not let opposite diagonal handles overlap.
                let corner = match region.role {
                    WindowChromeRole::Action(WindowAction::BeginResize(
                        WindowResizeEdge::TopLeft,
                    )) => Some((false, false)),
                    WindowChromeRole::Action(WindowAction::BeginResize(
                        WindowResizeEdge::TopRight,
                    )) => Some((true, false)),
                    WindowChromeRole::Action(WindowAction::BeginResize(
                        WindowResizeEdge::BottomLeft,
                    )) => Some((false, true)),
                    WindowChromeRole::Action(WindowAction::BeginResize(
                        WindowResizeEdge::BottomRight,
                    )) => Some((true, true)),
                    _ => None,
                };
                if let Some((right, bottom)) = corner {
                    let expanded = outer.outset(outset).rect;
                    let mid_x = frame.bounds.x + frame.bounds.width * 0.5;
                    let mid_y = frame.bounds.y + frame.bounds.height * 0.5;
                    let x = if right { mid_x } else { expanded.x };
                    let y = if bottom { mid_y } else { expanded.y };
                    let quadrant = RectF {
                        x,
                        y,
                        width: if right {
                            expanded.right() - x
                        } else {
                            mid_x - x
                        },
                        height: if bottom {
                            expanded.bottom() - y
                        } else {
                            mid_y - y
                        },
                    };
                    region.hit_bounds = region
                        .hit_bounds
                        .intersection(quadrant)
                        .unwrap_or(RectF::ZERO);
                }
            }
        }
        Ok(Self {
            frame,
            content: content.ok_or(WindowChromeError::MissingContentSlot)?,
            regions,
        })
    }

    /// Returns the top-most action/drag region containing a frame-local point.
    pub fn hit_test(&self, x: f32, y: f32) -> Option<WindowChromeRole> {
        self.hit_test_region(x, y).map(|region| region.role)
    }

    /// Whether a frame-local point belongs to client content rather than a chrome target.
    /// Rounded resize bands can overlap the content slot's rectangular bounds.
    pub fn hit_test_content(&self, x: f32, y: f32) -> bool {
        self.content.bounds.contains(crate::foundation::PointF { x, y })
            && self.hit_test_region(x, y).is_none()
    }

    /// Returns the highest-priority, top-most region containing a frame-local point.
    pub fn hit_test_region(&self, x: f32, y: f32) -> Option<&WindowChromeRegion> {
        let point = crate::foundation::PointF { x, y };
        self.regions
            .iter()
            .enumerate()
            .filter(|(_, region)| {
                region.hit_bounds.contains(point)
                    && region
                        .hit_clips
                        .iter()
                        .flatten()
                        .all(|clip| clip.contains(point))
            })
            .max_by_key(|(paint_order, region)| (region.priority, *paint_order))
            .map(|(_, region)| region)
    }
}

fn outset(bounds: RectF, insets: EdgeInsets) -> RectF {
    RectF {
        x: bounds.x - insets.left.max(0.0),
        y: bounds.y - insets.top.max(0.0),
        width: bounds.width + insets.left.max(0.0) + insets.right.max(0.0),
        height: bounds.height + insets.top.max(0.0) + insets.bottom.max(0.0),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WindowChromeError {
    #[error("window frame composition has no frame root")]
    MissingFrame,
    #[error("window frame composition has multiple frame roots")]
    MultipleFrames,
    #[error("window frame composition has no client content slot")]
    MissingContentSlot,
    #[error("window frame composition has multiple client content slots")]
    MultipleContentSlots,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::scene::NodeId;

    fn region(index: u32, role: WindowChromeRole, bounds: RectF) -> WindowChromeRegion {
        WindowChromeRegion {
            node: NodeId::new(index, 1),
            role,
            bounds,
            hit_bounds: bounds,
            hit_clips: [None; 2],
            priority: WindowChromeHitSpec::for_role(role).priority,
        }
    }

    #[test]
    fn nested_action_regions_take_priority_over_a_drag_parent() {
        let frame = region(
            0,
            WindowChromeRole::Frame,
            RectF {
                x: 0.0,
                y: 0.0,
                width: 320.0,
                height: 240.0,
            },
        );
        let content = region(1, WindowChromeRole::Content, frame.bounds);
        let snapshot = WindowChromeSnapshot {
            frame,
            content,
            regions: vec![
                region(
                    2,
                    WindowChromeRole::Action(WindowAction::Close),
                    RectF {
                        x: 280.0,
                        y: 0.0,
                        width: 40.0,
                        height: 40.0,
                    },
                ),
                region(
                    3,
                    WindowChromeRole::DragRegion,
                    RectF {
                        x: 0.0,
                        y: 0.0,
                        width: 320.0,
                        height: 40.0,
                    },
                ),
            ],
        };

        assert_eq!(
            snapshot.hit_test(300.0, 20.0),
            Some(WindowChromeRole::Action(WindowAction::Close))
        );
        assert_eq!(
            snapshot.hit_test(100.0, 20.0),
            Some(WindowChromeRole::DragRegion)
        );
    }

    #[test]
    fn hit_slop_and_explicit_priority_do_not_change_paint_bounds() {
        let frame = region(
            0,
            WindowChromeRole::Frame,
            RectF {
                x: 0.0,
                y: 0.0,
                width: 320.0,
                height: 240.0,
            },
        );
        let content = region(1, WindowChromeRole::Content, frame.bounds);
        let mut resize = region(
            2,
            WindowChromeRole::Action(WindowAction::BeginResize(WindowResizeEdge::TopRight)),
            RectF {
                x: 300.0,
                y: 0.0,
                width: 20.0,
                height: 20.0,
            },
        );
        resize.hit_bounds = outset(resize.bounds, EdgeInsets::all(6.0));
        resize.priority = 500;
        let snapshot = WindowChromeSnapshot {
            frame,
            content,
            regions: vec![
                region(
                    3,
                    WindowChromeRole::Action(WindowAction::Close),
                    RectF {
                        x: 294.0,
                        y: 0.0,
                        width: 26.0,
                        height: 26.0,
                    },
                ),
                resize,
            ],
        };

        assert_eq!(resize.bounds.x, 300.0);
        assert_eq!(resize.hit_bounds.x, 294.0);
        assert_eq!(
            snapshot.hit_test(296.0, 10.0),
            Some(WindowChromeRole::Action(WindowAction::BeginResize(
                WindowResizeEdge::TopRight
            )))
        );
    }
}

//! Mount-time conveniences for the application primitive domain.

use crate::runtime::{RuntimeResult, Ui};
use crate::ui::UiNodeId;

use crate::components::application::primitives::{
    ApplicationRegion, ApplicationRegionRef, ApplicationRoot, ApplicationRootRef, HudLayer,
    HudLayerRef, RenderTargetView, RenderTargetViewRef, VideoSurface, VideoSurfaceRef,
    ViewportOverlay, ViewportOverlayRef, WorldAnchor, WorldAnchorRef,
};

/// Mount-only conveniences that keep application primitive implementations outside the runtime.
pub trait ApplicationUiExt<Action: 'static> {
    fn mount_application_root(
        &mut self,
        host: UiNodeId,
        root: &ApplicationRoot,
    ) -> RuntimeResult<ApplicationRootRef>;

    fn mount_application_region(
        &mut self,
        host: UiNodeId,
        region: &ApplicationRegion,
    ) -> RuntimeResult<ApplicationRegionRef>;

    fn mount_hud_layer(&mut self, host: UiNodeId, layer: &HudLayer) -> RuntimeResult<HudLayerRef>;

    fn mount_viewport_overlay(
        &mut self,
        host: UiNodeId,
        overlay: &ViewportOverlay,
    ) -> RuntimeResult<ViewportOverlayRef>;

    fn mount_world_anchor(
        &mut self,
        host: UiNodeId,
        anchor: &WorldAnchor,
    ) -> RuntimeResult<WorldAnchorRef>;

    fn mount_render_target_view(
        &mut self,
        host: UiNodeId,
        view: &RenderTargetView,
    ) -> RuntimeResult<RenderTargetViewRef>;

    fn mount_video_surface(
        &mut self,
        host: UiNodeId,
        surface: &VideoSurface,
    ) -> RuntimeResult<VideoSurfaceRef>;
}

impl<Action: 'static> ApplicationUiExt<Action> for Ui<'_, '_, Action> {
    fn mount_application_root(
        &mut self,
        host: UiNodeId,
        root: &ApplicationRoot,
    ) -> RuntimeResult<ApplicationRootRef> {
        root.mount(self, host)
    }

    fn mount_application_region(
        &mut self,
        host: UiNodeId,
        region: &ApplicationRegion,
    ) -> RuntimeResult<ApplicationRegionRef> {
        region.mount(self, host)
    }

    fn mount_hud_layer(&mut self, host: UiNodeId, layer: &HudLayer) -> RuntimeResult<HudLayerRef> {
        layer.mount(self, host)
    }

    fn mount_viewport_overlay(
        &mut self,
        host: UiNodeId,
        overlay: &ViewportOverlay,
    ) -> RuntimeResult<ViewportOverlayRef> {
        overlay.mount(self, host)
    }

    fn mount_world_anchor(
        &mut self,
        host: UiNodeId,
        anchor: &WorldAnchor,
    ) -> RuntimeResult<WorldAnchorRef> {
        anchor.mount(self, host)
    }

    fn mount_render_target_view(
        &mut self,
        host: UiNodeId,
        view: &RenderTargetView,
    ) -> RuntimeResult<RenderTargetViewRef> {
        view.mount(self, host)
    }

    fn mount_video_surface(
        &mut self,
        host: UiNodeId,
        surface: &VideoSurface,
    ) -> RuntimeResult<VideoSurfaceRef> {
        surface.mount(self, host)
    }
}

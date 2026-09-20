//! Mount-time conveniences for the shell primitive domain.

use crate::runtime::{RuntimeResult, Ui};
use crate::ui::UiNodeId;

use crate::components::shell::primitives::{
    ClientSurface, ClientSurfaceMountError, ClientSurfaceRef, ExclusiveRegion,
    ExclusiveRegionMountError, ExclusiveRegionRef, OutputView, OutputViewRef, ShellLayer,
    ShellLayerMountError, ShellLayerOrder, ShellLayerRef, ShellRoot, ShellRootRef,
    SurfacePlaceholder, SurfacePlaceholderMountError, SurfacePlaceholderRef, SurfaceSnapshot,
    SurfaceSnapshotMountError, SurfaceSnapshotRef, SurfaceTree, SurfaceTreeMountError,
    SurfaceTreeRef,
};

/// Delegating mount conveniences; this trait introduces no alternate lifecycle or policy owner.
pub trait ShellUiExt<Action: 'static> {
    fn mount_shell_root(&mut self, host: UiNodeId, root: &ShellRoot)
    -> RuntimeResult<ShellRootRef>;

    fn mount_output_view(
        &mut self,
        root: ShellRootRef,
        output: &OutputView,
    ) -> RuntimeResult<OutputViewRef>;

    fn mount_shell_layer(
        &mut self,
        output: OutputViewRef,
        order: &mut ShellLayerOrder,
        layer: &ShellLayer,
    ) -> Result<ShellLayerRef, ShellLayerMountError>;

    fn mount_client_surface(
        &mut self,
        layer: ShellLayerRef,
        surface: &ClientSurface,
    ) -> Result<ClientSurfaceRef, ClientSurfaceMountError>;

    fn mount_surface_tree(
        &mut self,
        layer: ShellLayerRef,
        tree: &SurfaceTree,
    ) -> Result<SurfaceTreeRef, SurfaceTreeMountError>;

    fn mount_surface_placeholder(
        &mut self,
        layer: ShellLayerRef,
        placeholder: &SurfacePlaceholder,
    ) -> Result<SurfacePlaceholderRef, SurfacePlaceholderMountError>;

    fn mount_surface_snapshot(
        &mut self,
        layer: ShellLayerRef,
        snapshot: &SurfaceSnapshot,
    ) -> Result<SurfaceSnapshotRef, SurfaceSnapshotMountError>;

    fn mount_exclusive_region(
        &mut self,
        layer: ShellLayerRef,
        region: &ExclusiveRegion,
    ) -> Result<ExclusiveRegionRef, ExclusiveRegionMountError>;
}

impl<Action: 'static> ShellUiExt<Action> for Ui<'_, '_, Action> {
    fn mount_shell_root(
        &mut self,
        host: UiNodeId,
        root: &ShellRoot,
    ) -> RuntimeResult<ShellRootRef> {
        root.mount(self, host)
    }

    fn mount_output_view(
        &mut self,
        root: ShellRootRef,
        output: &OutputView,
    ) -> RuntimeResult<OutputViewRef> {
        output.mount(self, root)
    }

    fn mount_shell_layer(
        &mut self,
        output: OutputViewRef,
        order: &mut ShellLayerOrder,
        layer: &ShellLayer,
    ) -> Result<ShellLayerRef, ShellLayerMountError> {
        layer.mount(self, output, order)
    }

    fn mount_client_surface(
        &mut self,
        layer: ShellLayerRef,
        surface: &ClientSurface,
    ) -> Result<ClientSurfaceRef, ClientSurfaceMountError> {
        surface.mount(self, layer)
    }

    fn mount_surface_tree(
        &mut self,
        layer: ShellLayerRef,
        tree: &SurfaceTree,
    ) -> Result<SurfaceTreeRef, SurfaceTreeMountError> {
        tree.mount(self, layer)
    }

    fn mount_surface_placeholder(
        &mut self,
        layer: ShellLayerRef,
        placeholder: &SurfacePlaceholder,
    ) -> Result<SurfacePlaceholderRef, SurfacePlaceholderMountError> {
        placeholder.mount(self, layer)
    }

    fn mount_surface_snapshot(
        &mut self,
        layer: ShellLayerRef,
        snapshot: &SurfaceSnapshot,
    ) -> Result<SurfaceSnapshotRef, SurfaceSnapshotMountError> {
        snapshot.mount(self, layer)
    }

    fn mount_exclusive_region(
        &mut self,
        layer: ShellLayerRef,
        region: &ExclusiveRegion,
    ) -> Result<ExclusiveRegionRef, ExclusiveRegionMountError> {
        region.mount(self, layer)
    }
}

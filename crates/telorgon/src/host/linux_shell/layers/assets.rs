use super::*;

/// Immutable catalog resources decoded once for the shell session. ImageResource
/// clones share their pixel storage, so opening a tooltip does not decode wallpaper.
pub(in super::super) struct LayerAssets {
    pub(super) bundle: AssetBundle,
    pub(in super::super) typography: crate::Typography,
    pub(super) resources: Vec<crate::graphics::render::ImageResource>,
}
impl LayerAssets {
    pub(in super::super) fn new(bundle: AssetBundle) -> AppResult<Self> {
        let mut media = AssetMediaCache::new(bundle).map_err(app_error)?;
        let resources = media.preload_render_resources().map_err(app_error)?;
        Ok(Self { bundle, resources, typography: Default::default() })
    }
    pub(super) fn install(&self, runtime: &mut ComposedAppRuntime) -> AppResult<()> {
        runtime.register_svg_assets(self.bundle)?;
        runtime.set_typography(self.typography.clone());
        for resource in &self.resources {
            runtime.set_image_resource(resource.clone())?;
        }
        Ok(())
    }
}

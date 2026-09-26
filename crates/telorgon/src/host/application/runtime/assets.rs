use super::*;
use crate::assets::{
    AssetBundle, AssetKey, AssetKind, AssetMediaCache, AssetRasterSize, asset_image_id,
};
use std::collections::BTreeMap;

pub(super) struct SceneAssets {
    media: AssetMediaCache,
    svgs: BTreeMap<ImageId, AssetKey>,
    current: BTreeMap<ImageId, (AssetRasterSize, u64)>,
}

impl SceneAssets {
    fn new(bundle: AssetBundle) -> AppResult<Self> {
        let media = AssetMediaCache::new(bundle).map_err(|e| AppError::new(e.to_string()))?;
        let svgs = bundle
            .iter()
            .filter(|entry| {
                matches!(entry.kind, AssetKind::Icon | AssetKind::Image)
                    && entry.media_type == "image/svg+xml"
            })
            .map(|entry| (asset_image_id(entry.key), entry.key))
            .collect();
        Ok(Self {
            media,
            svgs,
            current: BTreeMap::new(),
        })
    }

    pub(super) fn update(
        &mut self,
        scene: &mut RenderScene,
        layout: &LayoutEngine,
        scale: f32,
    ) -> AppResult<()> {
        if self.svgs.is_empty() {
            return Ok(());
        }
        let mut requested = BTreeMap::<ImageId, AssetRasterSize>::new();
        for image in scene.images.values() {
            if !self.svgs.contains_key(&image.image) {
                continue;
            }
            let Some(geometry) = layout.computed(image.node) else {
                continue;
            };
            if geometry.visible_rect.area() <= 0.0 {
                continue;
            }
            let transform = geometry.world_transform;
            let width = image.rect.width * transform.m11.hypot(transform.m12) * scale;
            let height = image.rect.height * transform.m21.hypot(transform.m22) * scale;
            let Some(size) = AssetRasterSize::for_display(width, height) else {
                continue;
            };
            requested
                .entry(image.image)
                .and_modify(|current| {
                    current.width = current.width.max(size.width);
                    current.height = current.height.max(size.height);
                })
                .or_insert(size);
        }
        for (image, size) in requested {
            let version = scene.image_resource_version(image);
            if self
                .current
                .get(&image)
                .is_some_and(|(current, uploaded)| *current == size && Some(*uploaded) == version)
            {
                continue;
            }
            let decoded = self
                .media
                .rasterize_svg(self.svgs[&image], size)
                .map_err(|e| AppError::new(e.to_string()))?;
            let mut resource = decoded.render_resource();
            // Pixel hashes are not ordered. Scene replacement needs a monotonic revision,
            // including when a resize returns to an earlier raster size.
            resource.content_version = version
                .unwrap_or(0)
                .checked_add(1)
                .ok_or_else(|| AppError::new("SVG image revision exhausted"))?;
            let uploaded = resource.content_version;
            scene
                .set_image_resource(resource)
                .map_err(|e| AppError::new(e.to_string()))?;
            self.current.insert(image, (size, uploaded));
        }
        Ok(())
    }
}

impl<D: ComponentDriver> AppRuntimeCore<D> {
    /// Installs catalog image resources and keeps SVG textures at their displayed resolution.
    pub fn register_assets(&mut self, bundle: AssetBundle) -> AppResult<()> {
        let mut assets = SceneAssets::new(bundle)?;
        for mut resource in assets
            .media
            .preload_render_resources()
            .map_err(|e| AppError::new(e.to_string()))?
        {
            if let Some(previous) = self.scene.image_resource_version(resource.image) {
                resource.content_version = previous
                    .checked_add(1)
                    .ok_or_else(|| AppError::new("asset image revision exhausted"))?;
            }
            self.set_image_resource(resource)?;
        }
        self.scene_assets = Some(assets);
        self.view.scheduler_mut().request();
        Ok(())
    }

    // Shell layers already share decoded intrinsic resources; reuse those uploads.
    #[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
    pub(crate) fn register_svg_assets(&mut self, bundle: AssetBundle) -> AppResult<()> {
        self.scene_assets = Some(SceneAssets::new(bundle)?);
        self.view.scheduler_mut().request();
        Ok(())
    }
}

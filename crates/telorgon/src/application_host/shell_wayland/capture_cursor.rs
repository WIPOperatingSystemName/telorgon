//! Capture-only cursor composition. Hardware plane state and desktop cursor policy stay unchanged.

use super::pointer_visual::{CursorVisual, RenderedCursor, cursor_image_signature};
use super::scene::{
    ShellComposition, ShellFrame, ShellImageUpdate, ShellLayer, ShellLayerKey, ShellSceneKey,
};
use crate::application_host::{AppError, AppResult};
use crate::core::{PointF, RectI, SizeI};
use crate::platform::ScaleFactor;
use crate::render::{ImageAlphaMode, ImagePixelFormat};
use std::sync::Arc;

pub(super) struct CaptureCursor {
    composition: ShellComposition,
    had_source: bool,
}
impl CaptureCursor {
    pub fn new(extent: SizeI) -> Self {
        Self {
            composition: ShellComposition::new(extent),
            had_source: false,
        }
    }

    pub fn synchronize(
        &mut self,
        cursor: Option<&CursorVisual>,
        position: PointF,
        scale: ScaleFactor,
        extent: SizeI,
    ) -> AppResult<Option<ShellFrame>> {
        let layers = cursor
            .map(|CursorVisual::Image(image)| layer(image, position, scale))
            .transpose()?
            .into_iter()
            .collect();
        // Removing a wholly off-screen cursor has no pixel damage, but the GPU adapter still
        // needs an empty live-source set before a reappearing source restarts at epoch one.
        let retiring = self.had_source && cursor.is_none();
        self.had_source = cursor.is_some();
        // Geometry is already physical, matching the hardware cursor's position/hotspot rounding.
        // Do not send this frame through the desktop's logical-to-physical conversion a second time.
        Ok(self
            .composition
            .synchronize_with_force(extent, layers, retiring))
    }
}

fn layer(cursor: &RenderedCursor, position: PointF, scale: ScaleFactor) -> AppResult<ShellLayer> {
    if cursor.size.width <= 0
        || cursor.size.height <= 0
        || cursor.logical_size.width <= 0
        || cursor.logical_size.height <= 0
        || !position.x.is_finite()
        || !position.y.is_finite()
        || (cursor.size.width as usize)
            .checked_mul(cursor.size.height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            != Some(cursor.rgba.len())
    {
        return Err(AppError::new("invalid capture cursor geometry"));
    }
    let pointer = scale.physical_point(position);
    let hotspot_x = (cursor.hotspot.x as f32 * scale.get()).round() as i32;
    let hotspot_y = (cursor.hotspot.y as f32 * scale.get()).round() as i32;
    let target = RectI {
        x: (pointer.x.round() as i32).saturating_sub(hotspot_x),
        y: (pointer.y.round() as i32).saturating_sub(hotspot_y),
        width: (cursor.logical_size.width as f32 * scale.get())
            .round()
            .max(1.0) as i32,
        height: (cursor.logical_size.height as f32 * scale.get())
            .round()
            .max(1.0) as i32,
    };
    Ok(ShellLayer::image(
        ShellLayerKey::Cursor,
        ShellSceneKey::CursorImage,
        cursor_image_signature(cursor),
        ShellImageUpdate::Full(Arc::from(cursor.rgba.as_slice())),
        cursor.size,
        target,
        None,
        if cursor.premultiplied {
            ImageAlphaMode::Premultiplied
        } else {
            ImageAlphaMode::Straight
        },
        ImagePixelFormat::Rgba8,
        true,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::PointI;

    fn cursor() -> CursorVisual {
        CursorVisual::Image(RenderedCursor {
            rgba: vec![255; 32 * 32 * 4],
            size: SizeI {
                width: 32,
                height: 32,
            },
            logical_size: SizeI {
                width: 16,
                height: 16,
            },
            hotspot: PointI { x: 3, y: 5 },
            premultiplied: true,
        })
    }

    #[test]
    fn offscreen_hide_still_retires_resources_before_reappearance() {
        let extent = SizeI {
            width: 640,
            height: 480,
        };
        let mut capture = CaptureCursor::new(extent);
        let mut image = cursor();
        let CursorVisual::Image(cursor_image) = &mut image;
        cursor_image.hotspot = PointI {
            x: i32::MIN,
            y: i32::MIN,
        };
        let scale = ScaleFactor::new(1.25).unwrap();
        let point = PointF { x: 100.0, y: 100.0 };
        let offscreen = capture
            .synchronize(Some(&image), point, scale, extent)
            .unwrap()
            .unwrap();
        assert!(offscreen.placements.is_empty());
        assert!(!offscreen.live_scenes.is_empty());
        let retired = capture
            .synchronize(None, point, scale, extent)
            .unwrap()
            .unwrap();
        assert!(retired.live_scenes.is_empty());
        assert!(
            capture
                .synchronize(None, point, scale, extent)
                .unwrap()
                .is_none()
        );
        let visible = cursor();
        let reappeared = capture
            .synchronize(Some(&visible), point, scale, extent)
            .unwrap()
            .unwrap();
        assert_eq!(reappeared.placements.len(), 1);
        assert_eq!(reappeared.updates.len(), 1);
    }

    #[test]
    fn capture_cursor_matches_hardware_geometry_without_double_scaling() {
        let cursor = cursor();
        let image = cursor.image().unwrap();
        let scale = ScaleFactor::new(1.25).unwrap();
        let point = PointF { x: 100.4, y: 40.6 };
        let hardware = image
            .for_hardware(
                scale,
                SizeI {
                    width: 256,
                    height: 256,
                },
            )
            .unwrap();
        let layer = layer(image, point, scale).unwrap();
        let physical = scale.physical_point(point);
        assert_eq!(
            layer.target,
            RectI {
                x: physical.x.round() as i32 - hardware.hotspot.x,
                y: physical.y.round() as i32 - hardware.hotspot.y,
                width: hardware.size.width,
                height: hardware.size.height
            }
        );
        assert_eq!(layer.source_extent, image.size);
    }

    #[test]
    fn motion_only_updates_placement_and_hide_retires_the_cursor_scene() {
        let extent = SizeI {
            width: 640,
            height: 480,
        };
        let mut capture = CaptureCursor::new(extent);
        let cursor = cursor();
        let scale = ScaleFactor::new(1.0).unwrap();
        let point = PointF { x: 100.0, y: 100.0 };
        let initial = capture
            .synchronize(Some(&cursor), point, scale, extent)
            .unwrap()
            .unwrap();
        assert_eq!(initial.updates.len(), 1);
        assert!(
            capture
                .synchronize(Some(&cursor), point, scale, extent)
                .unwrap()
                .is_none()
        );
        let moved = capture
            .synchronize(Some(&cursor), PointF { x: 120.0, y: 100.0 }, scale, extent)
            .unwrap()
            .unwrap();
        assert!(
            moved.updates.is_empty(),
            "movement must not reupload the cursor image"
        );
        assert_ne!(initial.placements[0].target, moved.placements[0].target);
        let hidden = capture
            .synchronize(None, point, scale, extent)
            .unwrap()
            .unwrap();
        assert!(hidden.placements.is_empty());
        assert!(hidden.live_scenes.is_empty());
        let reappeared = capture
            .synchronize(Some(&cursor), point, scale, extent)
            .unwrap()
            .unwrap();
        assert_eq!(reappeared.updates.len(), 1);
    }
}

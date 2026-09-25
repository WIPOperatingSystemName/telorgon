use super::*;

impl<S: NativeRuntimeSource, P: NativePresentation> NativeHost<S, P> {
    pub(super) fn layout_scale_factor(&self) -> f32 {
        if self.presentation.uses_logical_coordinates() {
            self.window
                .as_ref()
                .map_or(1.0, |w| w.scale_factor() as f32)
        } else {
            1.0
        }
    }
}

pub(super) fn logical_extent(extent: SizeI, scale: f32) -> SizeF {
    SizeF {
        width: extent.width.max(1) as f32 / scale,
        height: extent.height.max(1) as f32 / scale,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_pixel_extent_preserves_logical_picker_size() {
        for scale in [1.0, 1.25, 2.0] {
            assert_eq!(
                logical_extent(
                    SizeI {
                        width: (640.0 * scale) as i32,
                        height: (520.0 * scale) as i32
                    },
                    scale
                ),
                SizeF {
                    width: 640.0,
                    height: 520.0
                }
            );
        }
    }
}

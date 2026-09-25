use super::*;

impl VulkanScene {
    pub(super) fn apply_image_resources(&mut self, updates: &[ImageResourceDelta]) {
        for update in updates {
            match update {
                ImageResourceDelta::Remove(id) => {
                    self.image_resources.remove(id);
                }
                ImageResourceDelta::Write(update) => {
                    let resource = self.image_resources.entry(update.image).or_insert_with(|| {
                        VulkanImageResource {
                            extent: update.extent,
                            color_encoding: update.color_encoding,
                            alpha_mode: update.alpha_mode,
                            pixel_format: update.pixel_format,
                            pixels: Arc::from([]),
                            pending: Vec::new(),
                            texture: RetainedTexture::default(),
                        }
                    });
                    resource.update_pixels(update);
                }
            }
        }
    }
}

impl VulkanImageResource {
    fn update_pixels(&mut self, update: &crate::graphics::render::ImageResourceUpdate) {
        if self.extent != update.extent
            || self.color_encoding != update.color_encoding
            || self.pixel_format != update.pixel_format
        {
            self.extent = update.extent;
            self.color_encoding = update.color_encoding;
            self.pixel_format = update.pixel_format;
            self.pixels = Arc::from([]);
            self.texture.image = None;
            self.pending.clear();
        }
        self.alpha_mode = update.alpha_mode;
        let stride = update.extent.width as usize * 4;
        let len = stride * update.extent.height as usize;
        let full = update.rect.x == 0
            && update.rect.y == 0
            && update.rect.width == update.extent.width
            && update.rect.height == update.extent.height;
        if full && update.row_bytes == stride && update.pixels.len() == len {
            // The delta already owns immutable pixels. Keep that allocation through upload.
            self.pixels = Arc::clone(&update.pixels);
        } else {
            if self.pixels.len() != len {
                self.pixels = vec![0; len].into();
            }
            let pixels = Arc::make_mut(&mut self.pixels);
            let copy_bytes = update.rect.width as usize * 4;
            for row in 0..update.rect.height as usize {
                let source = row * update.row_bytes;
                let target = (update.rect.y as usize + row) * stride + update.rect.x as usize * 4;
                pixels[target..target + copy_bytes]
                    .copy_from_slice(&update.pixels[source..source + copy_bytes]);
            }
        }
        // A complete replacement supersedes every earlier upload, including regional ones.
        if full {
            self.pending.clear();
        }
        self.pending.push(ImageUploadChunk {
            offset: vk::Offset3D {
                x: update.rect.x,
                y: update.rect.y,
                z: 0,
            },
            extent: vk::Extent3D {
                width: update.rect.width as u32,
                height: update.rect.height as u32,
                depth: 1,
            },
            row_bytes: update.row_bytes,
            bytes: Arc::clone(&update.pixels),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::RectI;
    use crate::graphics::render::ImageResourceUpdate;

    fn update() -> ImageResourceUpdate {
        ImageResourceUpdate {
            image: ImageId(1),
            content_version: 1,
            extent: SizeI {
                width: 2,
                height: 2,
            },
            rect: RectI {
                x: 0,
                y: 0,
                width: 2,
                height: 2,
            },
            row_bytes: 8,
            color_encoding: ImageColorEncoding::Srgb,
            alpha_mode: ImageAlphaMode::Premultiplied,
            pixel_format: ImagePixelFormat::Rgba8,
            pixels: vec![3; 16].into(),
        }
    }
    fn resource(update: &ImageResourceUpdate) -> VulkanImageResource {
        VulkanImageResource {
            extent: update.extent,
            color_encoding: update.color_encoding,
            alpha_mode: update.alpha_mode,
            pixel_format: update.pixel_format,
            pixels: Arc::from([]),
            pending: Vec::new(),
            texture: RetainedTexture::default(),
        }
    }

    #[test]
    fn full_updates_share_storage_and_supersede_unsubmitted_regions() {
        let full = update();
        let mut image = resource(&full);
        image.update_pixels(&full);
        assert!(Arc::ptr_eq(&image.pixels, &full.pixels));
        assert!(Arc::ptr_eq(&image.pending[0].bytes, &full.pixels));
        let patch = ImageResourceUpdate {
            rect: RectI {
                x: 1,
                y: 1,
                width: 1,
                height: 1,
            },
            row_bytes: 4,
            pixels: vec![9; 4].into(),
            ..full.clone()
        };
        image.update_pixels(&patch);
        assert_eq!(&image.pixels[12..], &[9; 4]);
        assert_eq!(&image.pixels[..12], &[3; 12]);
        assert_eq!(&*full.pixels, &[3; 16], "published pixels remain immutable");
        assert_eq!(&*image.pending[0].bytes, &[3; 16]);
        let replacement = ImageResourceUpdate {
            pixels: vec![7; 16].into(),
            ..full
        };
        image.update_pixels(&replacement);
        assert!(Arc::ptr_eq(&image.pixels, &replacement.pixels));
        assert_eq!(
            image.pending.len(),
            1,
            "only the latest complete frame needs uploading"
        );
    }

    #[test]
    fn padded_rows_keep_tight_backing_and_original_upload_stride() {
        let mut full = update();
        full.row_bytes = 12;
        full.pixels = [vec![1; 8], vec![99; 4], vec![2; 8], vec![99; 4]]
            .concat()
            .into();
        let mut image = resource(&full);
        image.update_pixels(&full);
        assert_eq!(&*image.pixels, &[vec![1; 8], vec![2; 8]].concat());
        assert_eq!(image.pending[0].row_bytes, 12);
        assert!(Arc::ptr_eq(&image.pending[0].bytes, &full.pixels));
    }
}

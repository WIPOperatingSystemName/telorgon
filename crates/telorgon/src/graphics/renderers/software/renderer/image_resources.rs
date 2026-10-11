use super::*;

impl SoftwareScene {
    pub(super) fn apply_image_resources(&mut self, updates: &[ImageResourceDelta]) {
        for update in updates {
            match update {
                ImageResourceDelta::Write(update) => {
                    let full = update.rect == crate::foundation::RectI {
                        x: 0, y: 0, width: update.extent.width, height: update.extent.height,
                    } && update.row_bytes == update.extent.width as usize * 4;
                    if full {
                        self.image_resources.insert(update.image, SoftwareImage {
                            extent: update.extent,
                            color_encoding: update.color_encoding,
                            alpha_mode: update.alpha_mode,
                            pixel_format: update.pixel_format,
                            pixels: Arc::clone(&update.pixels),
                        });
                        continue;
                    }
                    let image = self
                        .image_resources
                        .entry(update.image)
                        .or_insert_with(|| SoftwareImage {
                            extent: update.extent,
                            color_encoding: update.color_encoding,
                            alpha_mode: update.alpha_mode,
                            pixel_format: update.pixel_format,
                            pixels: vec![
                                0;
                                update.extent.width as usize
                                    * update.extent.height as usize
                                    * 4
                            ].into(),
                        });
                    if image.extent != update.extent || image.pixel_format != update.pixel_format {
                        image.extent = update.extent;
                        image.pixel_format = update.pixel_format;
                        image.pixels = vec![0;
                            update.extent.width as usize * update.extent.height as usize * 4
                        ].into();
                    }
                    image.color_encoding = update.color_encoding;
                    image.alpha_mode = update.alpha_mode;
                    let destination_stride = update.extent.width as usize * 4;
                    let copy_bytes = update.rect.width as usize * 4;
                    let pixels = Arc::make_mut(&mut image.pixels);
                    for row in 0..update.rect.height as usize {
                        let source = row * update.row_bytes;
                        let target = (update.rect.y as usize + row) * destination_stride
                            + update.rect.x as usize * 4;
                        pixels[target..target + copy_bytes]
                            .copy_from_slice(&update.pixels[source..source + copy_bytes]);
                    }
                }
                ImageResourceDelta::Remove(image) => {
                    self.image_resources.remove(image);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::RectI;
    use crate::graphics::render::ImageResourceUpdate;

    #[test]
    fn full_images_share_storage_and_region_writes_preserve_other_readers() {
        let original: Arc<[u8]> = [20, 40, 60, 255].repeat(8).into();
        let mut scene = SoftwareScene::default();
        let mut update = ImageResourceUpdate {
            image: ImageId(1), content_version: 1, extent: SizeI { width: 4, height: 2 },
            rect: RectI { x: 0, y: 0, width: 4, height: 2 }, row_bytes: 16,
            color_encoding: ImageColorEncoding::Srgb, alpha_mode: ImageAlphaMode::Premultiplied,
            pixel_format: ImagePixelFormat::Rgba8, pixels: original.clone(),
        };
        scene.apply_image_resources(&[ImageResourceDelta::Write(update.clone())]);
        assert!(Arc::ptr_eq(&original, &scene.image_resources[&ImageId(1)].pixels));
        let frozen = scene.clone();
        update.rect = RectI { x: 1, y: 1, width: 2, height: 1 };
        update.row_bytes = 12;
        update.pixels = [1, 2, 3, 255, 4, 5, 6, 255, 99, 99, 99, 99].into();
        scene.apply_image_resources(&[ImageResourceDelta::Write(update)]);
        assert_eq!(frozen.image_resources[&ImageId(1)].pixels, original);
        let pixels = &scene.image_resources[&ImageId(1)].pixels;
        assert_eq!(&pixels[20..28], &[1, 2, 3, 255, 4, 5, 6, 255]);
        assert_eq!(&pixels[..20], &original[..20]);
        assert_eq!(&pixels[28..], &original[28..]);
    }
}

use std::sync::Arc;

pub(super) fn premultiplied_linear_srgb(pixels: &Arc<[u8]>) -> Arc<[u8]> {
    if !pixels
        .chunks_exact(4)
        .any(|pixel| pixel[3] != 0 && pixel[3] != 255)
    {
        return Arc::clone(pixels);
    }
    let mut output = pixels.to_vec();
    for pixel in output.chunks_exact_mut(4) {
        let alpha = f32::from(pixel[3]) / 255.0;
        if alpha == 0.0 || alpha == 1.0 {
            continue;
        }
        for channel in &mut pixel[..3] {
            let straight = (f32::from(*channel) / 255.0 / alpha).min(1.0);
            let linear = if straight <= 0.04045 {
                straight / 12.92
            } else {
                ((straight + 0.055) / 1.055).powf(2.4)
            };
            let premultiplied = linear * alpha;
            let encoded = if premultiplied <= 0.0031308 {
                premultiplied * 12.92
            } else {
                1.055 * premultiplied.powf(1.0 / 2.4) - 0.055
            };
            *channel = (encoded * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    output.into()
}

use std::sync::{Arc, OnceLock};

use crate::boot::BootTargetKind;
use crate::foundation::SizeI;
use crate::graphics::render::{
    ImageAlphaMode, ImageColorEncoding, ImagePixelFormat, ImageResource,
};
use crate::ui::ImageId;

const WIDTH: i32 = 1100;
const HEIGHT: i32 = 560;

pub(super) fn voxel_background() -> ImageResource {
    static IMAGE: OnceLock<ImageResource> = OnceLock::new();
    IMAGE
        .get_or_init(|| {
            let mut canvas = Pixels::new(WIDTH, HEIGHT);
            for y in 0..HEIGHT {
                let t = y as f32 / HEIGHT as f32;
                let color = [
                    (93.0 + t * 52.0) as u8,
                    (151.0 + t * 31.0) as u8,
                    (171.0 - t * 29.0) as u8,
                    255,
                ];
                canvas.rect(0, y, WIDTH, 1, color);
            }
            canvas.rect(892, 54, 64, 64, [242, 220, 142, 255]);
            canvas.rect(878, 68, 92, 36, [242, 220, 142, 255]);
            for (x, y, width) in [(36, 66, 134), (190, 105, 104), (746, 120, 102)] {
                canvas.rect(x + 18, y, width - 30, 12, [205, 220, 202, 255]);
                canvas.rect(x, y + 12, width, 17, [223, 232, 214, 255]);
                canvas.rect(x + 28, y + 29, width - 46, 8, [187, 208, 194, 255]);
            }
            for x in (0..WIDTH).step_by(8) {
                let xf = x as f32;
                let far = 290 + ((xf * 0.010).sin() * 28.0 + (xf * 0.022).cos() * 14.0) as i32;
                let near = 356 + ((xf * 0.009 + 1.6).sin() * 42.0) as i32;
                canvas.rect(x, (far / 8) * 8, 8, HEIGHT, [72, 121, 116, 255]);
                canvas.rect(x, (near / 8) * 8, 8, HEIGHT, [58, 98, 81, 255]);
            }
            for y in (416..HEIGHT).step_by(16) {
                for x in (0..WIDTH).step_by(16) {
                    let noise = ((x / 16 * 17 + y / 16 * 31) % 19) as u8;
                    let color = if y < 448 {
                        [69 + noise, 116 + noise, 56 + noise / 2, 255]
                    } else {
                        [63 + noise / 2, 68 + noise / 2, 47 + noise / 3, 255]
                    };
                    canvas.rect(x, y, 16, 16, color);
                }
            }
            for (x, y, scale) in [(48, 300, 1), (1020, 282, 1), (1100, 315, 2)] {
                canvas.rect(
                    x - 8 * scale,
                    y + 60 * scale,
                    16 * scale,
                    100 * scale,
                    [53, 59, 38, 255],
                );
                for (dx, dy, w, h) in [(-48, 20, 96, 48), (-32, 0, 64, 80), (-64, 40, 128, 24)] {
                    canvas.rect(
                        x + dx * scale,
                        y + dy * scale,
                        w * scale,
                        h * scale,
                        [42, 77, 54, 255],
                    );
                }
                canvas.rect(
                    x - 32 * scale,
                    y,
                    64 * scale,
                    16 * scale,
                    [57, 100, 62, 255],
                );
                canvas.rect(
                    x - 48 * scale,
                    y + 20 * scale,
                    32 * scale,
                    16 * scale,
                    [50, 91, 59, 255],
                );
            }
            canvas.resource(0xb007_0002, false)
        })
        .clone()
}

pub(super) fn disk_icon(kind: BootTargetKind) -> ImageResource {
    static LINUX: OnceLock<ImageResource> = OnceLock::new();
    static WINDOWS: OnceLock<ImageResource> = OnceLock::new();
    static CUSTOM: OnceLock<ImageResource> = OnceLock::new();
    static UNKNOWN: OnceLock<ImageResource> = OnceLock::new();
    let (slot, id) = match kind {
        BootTargetKind::Linux => (&LINUX, 0xb007_0010),
        BootTargetKind::Windows => (&WINDOWS, 0xb007_0011),
        BootTargetKind::Custom => (&CUSTOM, 0xb007_0012),
        BootTargetKind::Unknown => (&UNKNOWN, 0xb007_0013),
    };
    slot.get_or_init(|| {
        let mut canvas = Pixels::new(160, 160);
        canvas.rounded_rect(24, 130, 112, 10, 5, [0, 0, 0, 18]);
        canvas.rounded_rect(20, 23, 120, 114, 11, [143, 146, 151, 255]);
        canvas.rounded_gradient(
            22,
            25,
            116,
            110,
            9,
            [240, 241, 243, 255],
            [185, 188, 192, 255],
        );
        canvas.rect(23, 115, 114, 1, [166, 169, 173, 255]);
        canvas.rect(32, 125, 78, 2, [115, 118, 123, 255]);
        canvas.circle(127, 126, 2, [109, 112, 117, 255]);
        draw_mark(
            &mut canvas,
            kind,
            55,
            46,
            50,
            [79, 82, 87, 255],
            [217, 219, 221, 255],
        );
        canvas.resource(id, true)
    })
    .clone()
}

pub(super) fn boot_mark(kind: BootTargetKind) -> ImageResource {
    static LINUX: OnceLock<ImageResource> = OnceLock::new();
    static WINDOWS: OnceLock<ImageResource> = OnceLock::new();
    static CUSTOM: OnceLock<ImageResource> = OnceLock::new();
    static UNKNOWN: OnceLock<ImageResource> = OnceLock::new();
    let (slot, id) = match kind {
        BootTargetKind::Linux => (&LINUX, 0xb007_0030),
        BootTargetKind::Windows => (&WINDOWS, 0xb007_0031),
        BootTargetKind::Custom => (&CUSTOM, 0xb007_0032),
        BootTargetKind::Unknown => (&UNKNOWN, 0xb007_0033),
    };
    slot.get_or_init(|| {
        let mut canvas = Pixels::new(160, 160);
        draw_mark(&mut canvas, kind, 28, 28, 104, [246, 246, 246, 255], [0; 4]);
        canvas.resource(id, true)
    })
    .clone()
}

fn draw_mark(
    canvas: &mut Pixels,
    kind: BootTargetKind,
    x: i32,
    y: i32,
    size: i32,
    ink: [u8; 4],
    inset: [u8; 4],
) {
    let ux = |value| x + value * size / 100;
    let uy = |value| y + value * size / 100;
    let u = |value| value * size / 100;
    match kind {
        BootTargetKind::Unknown => {
            canvas.rounded_rect(ux(10), uy(15), u(80), u(56), u(5), ink);
            canvas.rect(ux(17), uy(22), u(66), u(42), inset);
            canvas.rect(ux(44), uy(71), u(12), u(12), ink);
            canvas.rounded_rect(ux(29), uy(83), u(42), u(5), u(2), ink);
        }
        BootTargetKind::Linux => {
            canvas.ellipse(ux(50), uy(58), u(25), u(32), ink);
            canvas.ellipse(ux(50), uy(27), u(17), u(20), ink);
            canvas.polygon(&[(ux(29), uy(42)), (ux(13), uy(73)), (ux(33), uy(68))], ink);
            canvas.polygon(&[(ux(71), uy(42)), (ux(87), uy(73)), (ux(67), uy(68))], ink);
            canvas.ellipse(ux(35), uy(88), u(15), u(5), ink);
            canvas.ellipse(ux(65), uy(88), u(15), u(5), ink);
            canvas.ellipse(ux(50), uy(60), u(16), u(23), inset);
            canvas.circle(ux(44), uy(24), u(3), inset);
            canvas.circle(ux(56), uy(24), u(3), inset);
            canvas.polygon(
                &[(ux(45), uy(32)), (ux(55), uy(32)), (ux(50), uy(38))],
                inset,
            );
        }
        BootTargetKind::Windows => {
            for (px, py) in [(12, 12), (53, 12), (12, 53), (53, 53)] {
                canvas.rect(ux(px), uy(py), u(35), u(35), ink);
            }
        }
        BootTargetKind::Custom => {
            canvas.polygon(
                &[
                    (ux(50), uy(5)),
                    (ux(95), uy(50)),
                    (ux(50), uy(95)),
                    (ux(5), uy(50)),
                ],
                ink,
            );
            canvas.polygon(
                &[
                    (ux(50), uy(24)),
                    (ux(76), uy(50)),
                    (ux(50), uy(76)),
                    (ux(24), uy(50)),
                ],
                inset,
            );
            canvas.circle(ux(50), uy(50), u(5), ink);
        }
    }
}

pub(super) fn voxel_logo() -> ImageResource {
    static IMAGE: OnceLock<ImageResource> = OnceLock::new();
    IMAGE
        .get_or_init(|| {
            let word = "BLOCKBOOT";
            let scale = 9;
            let mut canvas = Pixels::new(488, 86);
            for (index, letter) in word.chars().enumerate() {
                let rows = glyph(letter);
                for layer in (0..=7).rev() {
                    let color = if layer == 0 {
                        [240, 239, 220, 255]
                    } else {
                        [51, 63, 56, 255]
                    };
                    for (y, row) in rows.iter().enumerate() {
                        for x in 0..5 {
                            if row & (1 << (4 - x)) != 0 {
                                canvas.rect(
                                    4 + index as i32 * scale * 6 + x * scale + layer / 2,
                                    5 + y as i32 * scale + layer,
                                    scale,
                                    scale,
                                    color,
                                );
                            }
                        }
                    }
                }
            }
            canvas.resource(0xb007_0020, true)
        })
        .clone()
}

fn glyph(letter: char) -> [u8; 7] {
    match letter {
        'B' => [30, 17, 17, 30, 17, 17, 30],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'C' => [14, 17, 16, 16, 16, 17, 14],
        'K' => [17, 18, 20, 24, 20, 18, 17],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        _ => [0; 7],
    }
}

struct Pixels {
    width: i32,
    height: i32,
    data: Vec<u8>,
}

impl Pixels {
    fn new(width: i32, height: i32) -> Self {
        Self {
            width,
            height,
            data: vec![0; width as usize * height as usize * 4],
        }
    }

    fn pixel(&mut self, x: i32, y: i32, color: [u8; 4]) {
        if (0..self.width).contains(&x) && (0..self.height).contains(&y) {
            let offset = ((y * self.width + x) * 4) as usize;
            self.data[offset..offset + 4].copy_from_slice(&color);
        }
    }

    fn rect(&mut self, x: i32, y: i32, width: i32, height: i32, color: [u8; 4]) {
        for y in y.max(0)..(y + height).min(self.height) {
            for x in x.max(0)..(x + width).min(self.width) {
                self.pixel(x, y, color);
            }
        }
    }

    fn circle(&mut self, cx: i32, cy: i32, radius: i32, color: [u8; 4]) {
        for y in cy - radius..=cy + radius {
            for x in cx - radius..=cx + radius {
                if (x - cx).pow(2) + (y - cy).pow(2) <= radius.pow(2) {
                    self.pixel(x, y, color);
                }
            }
        }
    }

    fn ellipse(&mut self, cx: i32, cy: i32, rx: i32, ry: i32, color: [u8; 4]) {
        for y in cy - ry..=cy + ry {
            for x in cx - rx..=cx + rx {
                if (x - cx).pow(2) * ry.pow(2) + (y - cy).pow(2) * rx.pow(2)
                    <= rx.pow(2) * ry.pow(2)
                {
                    self.pixel(x, y, color);
                }
            }
        }
    }

    fn rounded_gradient(
        &mut self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        radius: i32,
        top: [u8; 4],
        bottom: [u8; 4],
    ) {
        for row in 0..height {
            let mut color = [0; 4];
            for channel in 0..4 {
                color[channel] = (top[channel] as i32
                    + (bottom[channel] as i32 - top[channel] as i32) * row / (height - 1))
                    as u8;
            }
            for column in 0..width {
                let dx = (radius - column).max(column - (width - radius - 1)).max(0);
                let dy = (radius - row).max(row - (height - radius - 1)).max(0);
                if dx * dx + dy * dy <= radius * radius {
                    self.pixel(x + column, y + row, color);
                }
            }
        }
    }

    fn rounded_rect(
        &mut self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        radius: i32,
        color: [u8; 4],
    ) {
        self.rect(x + radius, y, width - 2 * radius, height, color);
        self.rect(x, y + radius, width, height - 2 * radius, color);
        for (cx, cy) in [
            (x + radius, y + radius),
            (x + width - radius - 1, y + radius),
            (x + radius, y + height - radius - 1),
            (x + width - radius - 1, y + height - radius - 1),
        ] {
            self.circle(cx, cy, radius, color);
        }
    }

    fn polygon(&mut self, vertices: &[(i32, i32)], color: [u8; 4]) {
        let min_y = vertices.iter().map(|p| p.1).min().unwrap_or(0);
        let max_y = vertices.iter().map(|p| p.1).max().unwrap_or(0);
        for y in min_y..=max_y {
            let mut intersections = Vec::new();
            for index in 0..vertices.len() {
                let (x1, y1) = vertices[index];
                let (x2, y2) = vertices[(index + 1) % vertices.len()];
                if (y1 <= y && y2 > y) || (y2 <= y && y1 > y) {
                    intersections.push(x1 + (y - y1) * (x2 - x1) / (y2 - y1));
                }
            }
            intersections.sort_unstable();
            for pair in intersections.chunks_exact(2) {
                self.rect(pair[0], y, pair[1] - pair[0] + 1, 1, color);
            }
        }
    }

    fn resource(self, image: u32, transparent: bool) -> ImageResource {
        ImageResource {
            image: ImageId(image),
            content_version: 1,
            extent: SizeI {
                width: self.width,
                height: self.height,
            },
            color_encoding: ImageColorEncoding::Srgb,
            alpha_mode: if transparent {
                ImageAlphaMode::Straight
            } else {
                ImageAlphaMode::Opaque
            },
            pixel_format: ImagePixelFormat::Rgba8,
            pixels: Arc::from(self.data),
        }
    }
}

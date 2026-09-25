use crate::integrations::pipewire::MediaError;
/// Byte order names describe memory, including on little-endian hosts. P010 stores each
/// 10-bit component in the most significant bits of a little-endian 16-bit word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    Rgba8,
    Bgra8,
    Rgbx8,
    Bgrx8,
    Rgb8,
    Bgr8,
    Nv12,
    I420,
    Yuy2,
    P010,
    RgbaF16,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameRate {
    pub numerator: u32,
    pub denominator: u32,
}
impl FrameRate {
    pub const fn hz(hz: u32) -> Self {
        Self {
            numerator: hz,
            denominator: 1,
        }
    }
    /// None for variable-rate capture or an invalid zero denominator.
    pub fn period(self) -> Option<std::time::Duration> {
        (self.numerator != 0 && self.denominator != 0).then(|| {
            std::time::Duration::from_secs_f64(self.denominator as f64 / self.numerator as f64)
        })
    }
    pub fn reduced(self) -> Self {
        let (mut a, mut b) = (self.numerator, self.denominator);
        while b != 0 {
            (a, b) = (b, a % b);
        }
        if a == 0 {
            self
        } else {
            Self {
                numerator: self.numerator / a,
                denominator: self.denominator / a,
            }
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorRange {
    Unknown,
    Full,
    Limited,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorMatrix {
    Unknown,
    Rgb,
    Bt601,
    Bt709,
    Bt2020,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorPrimaries {
    Unknown,
    Bt709,
    Bt2020,
    DisplayP3,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferFunction {
    Unknown,
    Linear,
    Srgb,
    Bt709,
    Pq,
    Hlg,
}
/// Unknown stays unknown. In particular, receiving an HDR or wide-gamut format does not
/// imply a renderer can display it correctly; conversion must inspect this metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Colorimetry {
    pub range: ColorRange,
    pub matrix: ColorMatrix,
    pub primaries: ColorPrimaries,
    pub transfer: TransferFunction,
}
impl Colorimetry {
    pub const SRGB: Self = Self {
        range: ColorRange::Full,
        matrix: ColorMatrix::Rgb,
        primaries: ColorPrimaries::Bt709,
        transfer: TransferFunction::Srgb,
    };
    /// Relative linear-light BT.709 RGB; no absolute luminance is implied.
    pub const LINEAR_BT709: Self = Self {
        transfer: TransferFunction::Linear,
        ..Self::SRGB
    };
    pub const BT709: Self = Self {
        range: ColorRange::Limited,
        matrix: ColorMatrix::Bt709,
        primaries: ColorPrimaries::Bt709,
        transfer: TransferFunction::Bt709,
    };
    pub const DISPLAY_P3: Self = Self {
        range: ColorRange::Full,
        matrix: ColorMatrix::Rgb,
        primaries: ColorPrimaries::DisplayP3,
        transfer: TransferFunction::Srgb,
    };
    /// Limited-range BT.2020 non-constant-luminance YUV with PQ transfer.
    pub const BT2100_PQ: Self = Self {
        range: ColorRange::Limited,
        matrix: ColorMatrix::Bt2020,
        primaries: ColorPrimaries::Bt2020,
        transfer: TransferFunction::Pq,
    };
    /// Limited-range BT.2020 non-constant-luminance YUV with HLG transfer.
    pub const BT2100_HLG: Self = Self {
        transfer: TransferFunction::Hlg,
        ..Self::BT2100_PQ
    };
    pub const UNKNOWN: Self = Self {
        range: ColorRange::Unknown,
        matrix: ColorMatrix::Unknown,
        primaries: ColorPrimaries::Unknown,
        transfer: TransferFunction::Unknown,
    };
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoFormat {
    pub pixel: PixelFormat,
    pub width: u32,
    pub height: u32,
    /// Zero numerator means variable-rate capture; timestamps carry actual cadence.
    pub rate: FrameRate,
    pub color: Colorimetry,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaneShape {
    pub row_bytes: usize,
    pub rows: usize,
}
impl VideoFormat {
    pub const fn rgba(width: u32, height: u32, hz: u32) -> Self {
        Self {
            pixel: PixelFormat::Rgba8,
            width,
            height,
            rate: FrameRate::hz(hz),
            color: Colorimetry::SRGB,
        }
    }
    pub fn validate(self) -> Result<(), MediaError> {
        if self.width == 0
            || self.height == 0
            || self.width > 8192
            || self.height > 8192
            || self.rate.denominator == 0
            || self.rate.numerator > 1_000_000
            || self.rate.denominator > 1_000_000
            || self.rate.numerator as u64 > 1000 * self.rate.denominator as u64
            || self.pixel == PixelFormat::Yuy2 && self.width % 2 != 0
        {
            return Err(MediaError::InvalidArgument(
                "video format, extent or frame rate",
            ));
        }
        Ok(())
    }
    pub fn plane_count(self) -> usize {
        match self.pixel {
            PixelFormat::Nv12 | PixelFormat::P010 => 2,
            PixelFormat::I420 => 3,
            _ => 1,
        }
    }
    pub fn plane(self, index: usize) -> Option<PlaneShape> {
        if index >= self.plane_count() {
            return None;
        }
        let (w, h) = (self.width as usize, self.height as usize);
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let (row_bytes, rows) = match (self.pixel, index) {
            (
                PixelFormat::Rgba8 | PixelFormat::Bgra8 | PixelFormat::Rgbx8 | PixelFormat::Bgrx8,
                _,
            ) => (w * 4, h),
            (PixelFormat::Rgb8 | PixelFormat::Bgr8, _) => (w * 3, h),
            (PixelFormat::RgbaF16, _) => (w * 8, h),
            (PixelFormat::Yuy2, _) => (w * 2, h),
            (PixelFormat::Nv12 | PixelFormat::I420, 0) => (w, h),
            (PixelFormat::Nv12, _) => (cw * 2, ch),
            (PixelFormat::I420, _) => (cw, ch),
            (PixelFormat::P010, 0) => (w * 2, h),
            (PixelFormat::P010, _) => (cw * 4, ch),
        };
        Some(PlaneShape { row_bytes, rows })
    }
    pub fn byte_len(self) -> Result<usize, MediaError> {
        self.validate()?;
        (0..self.plane_count()).try_fold(0usize, |total, i| {
            let p = self.plane(i).unwrap();
            total
                .checked_add(
                    p.row_bytes
                        .checked_mul(p.rows)
                        .ok_or(MediaError::InvalidArgument("video plane overflow"))?,
                )
                .ok_or(MediaError::InvalidArgument("video allocation overflow"))
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
impl VideoRect {
    pub fn fits(self, format: VideoFormat) -> bool {
        self.width > 0
            && self.height > 0
            && self
                .x
                .checked_add(self.width)
                .is_some_and(|n| n <= format.width)
            && self
                .y
                .checked_add(self.height)
                .is_some_and(|n| n <= format.height)
    }
}
/// SPA/Wayland transform order: reflection around the vertical axis precedes rotation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VideoTransform {
    #[default]
    Normal,
    Rotate90,
    Rotate180,
    Rotate270,
    Flipped,
    Flipped90,
    Flipped180,
    Flipped270,
}

/// Capture negotiation bounds applied to each preferred pixel/color format. Native
/// sources may select any size/rate in this range; consumers inspect each frame format.
/// A minimum rate of 0/1 admits variable-rate screen streams. A producer still requires
/// a fixed nonzero rate. The maximum size is charged against the capture memory budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoCaptureRange {
    pub min_size: [u32; 2],
    pub max_size: [u32; 2],
    pub min_rate: FrameRate,
    pub max_rate: FrameRate,
}
impl VideoCaptureRange {
    pub fn contains(self, format: VideoFormat) -> bool {
        let within = |a: FrameRate, b: FrameRate| {
            a.numerator as u64 * b.denominator as u64 <= b.numerator as u64 * a.denominator as u64
        };
        format.width >= self.min_size[0]
            && format.height >= self.min_size[1]
            && format.width <= self.max_size[0]
            && format.height <= self.max_size[1]
            && within(self.min_rate, format.rate)
            && within(format.rate, self.max_rate)
    }
    pub(crate) fn validate(self, preferred: VideoFormat) -> Result<(), MediaError> {
        preferred.validate()?;
        for (size, rate) in [
            (self.min_size, self.min_rate),
            (self.max_size, self.max_rate),
        ] {
            VideoFormat {
                width: size[0],
                height: size[1],
                rate,
                ..preferred
            }
            .validate()?;
        }
        if !self.contains(preferred)
            || self.min_size[0] > self.max_size[0]
            || self.min_size[1] > self.max_size[1]
            || self.max_rate.numerator == 0
        {
            return Err(MediaError::InvalidArgument(
                "video capture negotiation range",
            ));
        }
        Ok(())
    }
}

//! D65 gamut conversion and explicit SDR preview tone mapping.
//! Transfer definitions: https://www.w3.org/TR/css-color-hdr-1/
//! D65 matrices: https://www.w3.org/TR/css-color-4/#color-conversion-code
use super::*;

/// Extended Reinhard luminance mapping followed by clipping to the sRGB gamut. This is an
/// explicit preview rendering policy, not HDR passthrough or display calibration. HLG uses
/// a zero-black reference display and the BT.2100 peak-dependent system gamma.
#[derive(Clone, Copy, Debug)]
pub struct HdrToneMap {
    exposure_nits: f32,
    peak_nits: f32,
}
impl HdrToneMap {
    /// Exposure normalizes luminance before compression; peak maps to SDR white. These
    /// values are supplied by the caller, not guessed from unspecified transport metadata.
    pub fn reinhard(exposure_nits: f32, peak_nits: f32) -> Result<Self, MediaError> {
        if !exposure_nits.is_finite()
            || !peak_nits.is_finite()
            || !(1.0..=10000.0).contains(&exposure_nits)
            || !(exposure_nits..=10000.0).contains(&peak_nits)
            || peak_nits < 100.0
        {
            return Err(MediaError::InvalidArgument("HDR exposure/peak luminance"));
        }
        Ok(Self {
            exposure_nits,
            peak_nits,
        })
    }
}
pub(super) fn convert(rgb: [f32; 3], color: Colorimetry, tone_map: Option<HdrToneMap>) -> [f32; 3] {
    let hdr = matches!(color.transfer, TransferFunction::Pq | TransferFunction::Hlg);
    let mut linear = match color.transfer {
        TransferFunction::Pq => rgb.map(|value| {
            let code = value.clamp(0.0, 1.0).powf(32.0 / 2523.0);
            let numerator = (code - 3424.0 / 4096.0).max(0.0);
            let denominator = 2413.0 / 128.0 - (2392.0 / 128.0) * code;
            10000.0 * (numerator / denominator).powf(16384.0 / 2610.0)
        }),
        TransferFunction::Hlg => {
            let mapping = tone_map.expect("validated HDR policy");
            let scene = rgb.map(|value| {
                let value = value.clamp(0.0, 1.0);
                if value <= 0.5 {
                    value * value / 3.0
                } else {
                    (((value - 0.55991073) / 0.17883277).exp() + 0.28466892) / 12.0
                }
            });
            let luminance = (0.2627 * scene[0] + 0.6780 * scene[1] + 0.0593 * scene[2]).max(0.0);
            let gamma = 1.2 + 0.42 * (mapping.peak_nits / 1000.0).log10();
            let scale = if luminance > 0.0 {
                mapping.peak_nits * luminance.powf(gamma - 1.0)
            } else {
                0.0
            };
            scene.map(|value| value * scale)
        }
        _ => rgb.map(|value| linearize(value, color.transfer)),
    };
    linear = to_srgb_primaries(linear, color.primaries);
    if hdr {
        let mapping = tone_map.expect("validated HDR policy");
        let luminance = (0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2]).max(0.0);
        if luminance > 0.0 {
            let normalized = luminance / mapping.exposure_nits;
            let white = mapping.peak_nits / mapping.exposure_nits;
            let mapped = normalized * (1.0 + normalized / (white * white)) / (1.0 + normalized);
            linear = linear.map(|value| value * mapped / luminance);
        } else {
            linear = [0.0; 3];
        }
    }
    linear.map(|value| srgb_encode(value).clamp(0.0, 1.0))
}
fn to_srgb_primaries(rgb: [f32; 3], primaries: ColorPrimaries) -> [f32; 3] {
    if primaries == ColorPrimaries::Bt709 {
        return rgb;
    }
    let matrix: [[f64; 3]; 3] = match primaries {
        ColorPrimaries::DisplayP3 => [
            [
                608311.0 / 1250200.0,
                189793.0 / 714400.0,
                198249.0 / 1000160.0,
            ],
            [
                35783.0 / 156275.0,
                247089.0 / 357200.0,
                198249.0 / 2500400.0,
            ],
            [0.0, 32229.0 / 714400.0, 5220557.0 / 5000800.0],
        ],
        ColorPrimaries::Bt2020 => [
            [
                63426534.0 / 99577255.0,
                20160776.0 / 139408157.0,
                47086771.0 / 278816314.0,
            ],
            [
                26158966.0 / 99577255.0,
                472592308.0 / 697040785.0,
                8267143.0 / 139408157.0,
            ],
            [0.0, 19567812.0 / 697040785.0, 295819943.0 / 278816314.0],
        ],
        _ => return rgb,
    };
    let xyz: [f64; 3] = matrix.map(|row| {
        row.into_iter()
            .zip(rgb)
            .map(|(weight, value)| weight * f64::from(value))
            .sum()
    });
    let inverse = [
        [12831.0 / 3959.0, -329.0 / 214.0, -1974.0 / 3959.0],
        [
            -851781.0 / 878810.0,
            1648619.0 / 878810.0,
            36519.0 / 878810.0,
        ],
        [705.0 / 12673.0, -2585.0 / 12673.0, 705.0 / 667.0],
    ];
    inverse.map(|row| {
        row.into_iter()
            .zip(xyz)
            .map(|(weight, value)| weight * value)
            .sum::<f64>() as f32
    })
}

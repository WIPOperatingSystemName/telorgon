use crate::graphics::render::ColorSpace;

pub(super) fn decode_target_channel(value: u8, color_space: ColorSpace) -> f32 {
    match color_space {
        ColorSpace::Linear => f32::from(value) / 255.0,
        ColorSpace::Srgb => srgb_decode_byte(value),
        ColorSpace::Extended | ColorSpace::BackendDefined => unreachable!("validated color space"),
    }
}

pub(super) fn encode_target_channel(value: f32, color_space: ColorSpace) -> u8 {
    let value = value.clamp(0.0, 1.0);
    match color_space {
        ColorSpace::Linear => (value * 255.0).round().clamp(0.0, 255.0) as u8,
        ColorSpace::Srgb => cached::encode_clamped(value),
        ColorSpace::Extended | ColorSpace::BackendDefined => unreachable!("validated color space"),
    }
}

pub(super) fn srgb_decode_byte(value: u8) -> f32 {
    cached::decode_byte(value)
}

fn reference_decode_byte(value: u8) -> f32 {
    reference_decode_unit(f32::from(value) / 255.0)
}

fn reference_decode_unit(value: f32) -> f32 {
    if value <= 0.040_45 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn reference_encode_clamped(value: f32) -> u8 {
    let encoded = if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round().clamp(0.0, 255.0) as u8
}

// Reuse exact byte decoding and encoding decision boundaries on every software backend.
// Values near an encoding boundary still use the reference formula to preserve byte rounding.
mod cached {
    use std::sync::OnceLock;

    use super::{reference_decode_byte, reference_decode_unit, reference_encode_clamped};

    const NEAR_BOUNDARY: f32 = 0.000_002;

    struct Tables {
        decode: [f32; 256],
        thresholds: [f32; 255],
    }

    fn tables() -> &'static Tables {
        static TABLES: OnceLock<Tables> = OnceLock::new();
        TABLES.get_or_init(|| Tables {
            decode: std::array::from_fn(|index| reference_decode_byte(index as u8)),
            thresholds: std::array::from_fn(|index| {
                reference_decode_unit((index as f32 + 0.5) / 255.0)
            }),
        })
    }

    pub(super) fn decode_byte(value: u8) -> f32 {
        tables().decode[value as usize]
    }

    /// Receives the same clamped input as the reference encoder, including a possible NaN.
    pub(super) fn encode_clamped(value: f32) -> u8 {
        // The reference's final saturating float-to-byte cast maps NaN to zero. Binary search
        // would otherwise treat all of its unordered comparisons as the maximum byte.
        if value.is_nan() {
            return 0;
        }
        let thresholds = &tables().thresholds;
        // After clamping and rejecting NaN, positive IEEE f32 bit patterns are ordered like
        // their numeric values. Normalize -0 so the search uses integer comparisons under
        // UEFI's soft-float ABI as well as avoiding powf.
        let bits = value.to_bits() & 0x7fff_ffff;
        let mut low = 0;
        let mut high = thresholds.len();
        while low < high {
            let middle = low + (high - low) / 2;
            if bits < thresholds[middle].to_bits() {
                high = middle;
            } else {
                low = middle + 1;
            }
        }
        // Transfer formulas are mathematical inverses, but f32 arithmetic and powf round at
        // different intermediate steps. Use the exact reference around either neighboring
        // threshold instead of trusting the cached inverse to decide a borderline byte.
        let near_previous = low > 0 && (value - thresholds[low - 1]).abs() <= NEAR_BOUNDARY;
        let near_next = low < thresholds.len() && (value - thresholds[low]).abs() <= NEAR_BOUNDARY;
        if near_previous || near_next {
            reference_encode_clamped(value)
        } else {
            low as u8
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn cached_byte_decode_is_bit_exact_for_every_input() {
            for byte in 0..=255 {
                let byte = byte as u8;
                assert_eq!(
                    decode_byte(byte).to_bits(),
                    reference_decode_byte(byte).to_bits(),
                    "byte {byte}"
                );
            }
        }

        #[test]
        fn cached_encode_matches_reference_at_thresholds_and_adjacent_floats() {
            for (index, threshold) in tables().thresholds.iter().copied().enumerate() {
                for bits in threshold.to_bits() - 2..=threshold.to_bits() + 2 {
                    let value = f32::from_bits(bits);
                    assert_eq!(
                        encode_clamped(value),
                        reference_encode_clamped(value),
                        "threshold {index}, value {value:?}"
                    );
                }
                // Also exercise both sides of the conservative fallback band.
                for offset in [
                    -NEAR_BOUNDARY * 2.0,
                    -NEAR_BOUNDARY,
                    NEAR_BOUNDARY,
                    NEAR_BOUNDARY * 2.0,
                ] {
                    let value = (threshold + offset).clamp(0.0, 1.0);
                    assert_eq!(
                        encode_clamped(value),
                        reference_encode_clamped(value),
                        "threshold {index}, offset {offset:?}"
                    );
                }
            }
        }

        #[test]
        fn cached_encode_matches_reference_across_dense_unit_interval() {
            for step in 0..=100_000 {
                let value = step as f32 / 100_000.0;
                assert_eq!(
                    encode_clamped(value),
                    reference_encode_clamped(value),
                    "value {value:?}"
                );
            }
        }

        #[test]
        fn cached_encode_preserves_clamping_and_nonfinite_results() {
            for value in [
                f32::NEG_INFINITY,
                -1.0,
                -0.0,
                0.0,
                1.0,
                2.0,
                f32::INFINITY,
                f32::NAN,
            ] {
                let clamped = value.clamp(0.0, 1.0);
                assert_eq!(encode_clamped(clamped), reference_encode_clamped(clamped));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_target_channels_retain_the_reference_byte_mapping() {
        for byte in 0..=255 {
            let byte = byte as u8;
            let value = f32::from(byte) / 255.0;
            assert_eq!(decode_target_channel(byte, ColorSpace::Linear), value);
            assert_eq!(encode_target_channel(value, ColorSpace::Linear), byte);
        }
    }
}

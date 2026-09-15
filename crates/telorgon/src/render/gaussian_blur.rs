use crate::ui::ImageId;

/// A separable Gaussian pass. Sigma and offsets use source pixels, not output fractions.
/// The source must be an owned image; both axes must use the same full-resolution extent.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct GaussianBlurMaterial {
    pub source: ImageId,
    pub inverse_size: [f32; 2],
    /// Standard deviation, limited to 128 physical pixels (support at most 384).
    pub sigma: f32,
    pub horizontal: bool,
}

impl GaussianBlurMaterial {
    pub(crate) fn valid(self) -> bool {
        self.sigma.is_finite()
            && (0.0..=128.0).contains(&self.sigma)
            && self
                .inverse_size
                .into_iter()
                .all(|x| x.is_finite() && x > 0.0)
    }

    /// Six header words: inverse size, UV step, center weight, positive pair count.
    /// Each pair adds a source-pixel offset and weight. Symmetric negative pairs are implicit.
    pub(crate) fn words(self) -> Vec<u32> {
        // Validation runs before packing; keep this bounded even for diagnostic callers.
        let sigma = if self.sigma.is_finite() {
            self.sigma.clamp(0.0, 128.0) as f64
        } else {
            0.0
        };
        let radius = (3.0 * sigma).ceil() as usize;
        let weights: Vec<f64> = (0..=radius)
            .map(|x| {
                if x == 0 {
                    1.0
                } else {
                    (-0.5 * (x as f64 / sigma).powi(2)).exp()
                }
            })
            .collect();
        let normalization = weights[0] + 2.0 * weights.iter().skip(1).sum::<f64>();
        let mut words = vec![
            self.inverse_size[0],
            self.inverse_size[1],
            if self.horizontal {
                self.inverse_size[0]
            } else {
                0.0
            },
            if self.horizontal {
                0.0
            } else {
                self.inverse_size[1]
            },
            (weights[0] / normalization) as f32,
            0.0,
        ];
        for x in (1..=radius).step_by(2) {
            let a = weights[x];
            let b = weights.get(x + 1).copied().unwrap_or(0.0);
            let weight = a + b;
            if weight == 0.0 {
                continue;
            }
            // A linear sample at this weighted offset reconstructs two adjacent taps exactly.
            words.extend([
                (x as f64 + b / weight) as f32,
                (weight / normalization) as f32,
            ]);
        }
        words[5] = ((words.len() - 6) / 2) as f32;
        words.into_iter().map(f32::to_bits).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn material(sigma: f32) -> GaussianBlurMaterial {
        GaussianBlurMaterial {
            source: ImageId(1),
            inverse_size: [1.0 / 1931.0, 1.0 / 1081.0],
            sigma,
            horizontal: true,
        }
    }
    fn kernel(sigma: f32) -> Vec<f32> {
        material(sigma)
            .words()
            .into_iter()
            .map(f32::from_bits)
            .collect()
    }

    #[test]
    fn paired_kernel_preserves_constant_color_and_is_finite_at_all_supported_scales() {
        for sigma in [0.0, 1e-30, 0.1, 0.5, 2.0, 5.0, 32.0, 128.0] {
            let words = kernel(sigma);
            assert!(words.iter().all(|v| v.is_finite()));
            assert!(words[5] <= 192.0);
            assert_eq!(words.len(), 6 + 2 * words[5] as usize);
            let sum = words[4] + 2.0 * words[6..].chunks_exact(2).map(|v| v[1]).sum::<f32>();
            assert!((sum - 1.0).abs() < 2e-6);
            assert!(
                words[6..]
                    .chunks_exact(2)
                    .all(|p| p[0] > 0.0 && p[1] >= 0.0)
            );
        }
        assert_eq!(&kernel(0.0)[4..], &[1.0, 0.0]);
    }

    #[test]
    fn paired_linear_samples_match_independent_discrete_gaussian_on_fine_detail() {
        // Alternating detail is precisely what sparse, stretched taps would alias.
        let signal = |x: i32| if x % 2 == 0 { 0.9_f64 } else { 0.1_f64 };
        let linear = |x: f64| {
            let left = x.floor() as i32;
            let t = x - left as f64;
            signal(left) * (1.0 - t) + signal(left + 1) * t
        };
        for sigma in [0.4_f32, 1.0, 2.7, 5.0, 10.0, 32.0, 128.0] {
            let support = (sigma as f64 * 3.0).ceil() as i32;
            let mut expected = 0.0;
            let mut mass = 0.0;
            for x in -support..=support {
                let weight = (-0.5 * (x as f64 / sigma as f64).powi(2)).exp();
                expected += signal(x) * weight;
                mass += weight;
            }
            expected /= mass;
            let words = kernel(sigma);
            let mut paired = signal(0) * words[4] as f64;
            for p in words[6..].chunks_exact(2) {
                paired += (linear(p[0] as f64) + linear(-f64::from(p[0]))) * p[1] as f64;
            }
            assert!(
                (paired - expected).abs() < 2e-6,
                "sigma {sigma}: {paired} != {expected}"
            );
            if sigma >= 2.0 {
                assert!((paired - 0.5).abs() < 0.002);
            }
        }
    }

    #[test]
    fn sigma_changes_weights_independently_of_source_resolution_and_axis() {
        let a = kernel(2.0);
        let b = kernel(5.0);
        assert_eq!(&a[..4], &b[..4]);
        assert_ne!(&a[4..], &b[4..]);
        let mut vertical = material(5.0);
        vertical.horizontal = false;
        let v: Vec<_> = vertical.words().into_iter().map(f32::from_bits).collect();
        assert_eq!(v[2], 0.0);
        assert_eq!(v[3], vertical.inverse_size[1]);
        assert_eq!(&v[4..], &b[4..]);
        for sigma in [f32::NAN, f32::INFINITY, -0.1, 128.01] {
            assert!(!material(sigma).valid());
        }
    }
}

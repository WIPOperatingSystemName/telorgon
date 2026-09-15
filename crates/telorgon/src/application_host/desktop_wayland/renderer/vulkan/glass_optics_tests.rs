//! Mathematical regression checks for the outline-matched GLSL lens.

#[test]
fn edge_twist_matches_rotation_preserves_strength_and_fades_inward() {
    let twist = |v: [f64; 2], weight: f64, tangent: f64| {
        let k = tangent * weight;
        let scale = 1.0 / (1.0 + k * k).sqrt();
        [(v[0] - v[1] * k) * scale, (v[1] + v[0] * k) * scale]
    };
    for tangent in [0.0, 0.06992681194, -0.06992681194] {
        for step in 0..=100 {
            let weight = step as f64 / 100.0;
            let angle = (tangent * weight).atan();
            for v in [[0.0, 0.0], [weight, 0.0], [-0.6 * weight, 0.8 * weight]] {
                let actual = twist(v, weight, tangent);
                let expected = [
                    v[0] * angle.cos() - v[1] * angle.sin(),
                    v[0] * angle.sin() + v[1] * angle.cos(),
                ];
                for i in 0..2 {
                    assert!((actual[i] - expected[i]).abs() < 1e-12);
                }
                assert!((actual[0].hypot(actual[1]) - v[0].hypot(v[1])).abs() < 1e-12);
                assert!(angle.abs() <= 4.0_f64.to_radians() + 1e-10);
            }
        }
    }
    assert_eq!(twist([0.6, 0.8], 0.0, 0.06992681194), [0.6, 0.8]);
}

fn lens(p: [f64; 2], half: [f64; 2], radii: [f64; 4], bevel: f64) -> (f64, [f64; 2]) {
    lens_soft(p, half, radii, bevel, 0.0)
}

fn lens_soft(
    p: [f64; 2],
    half: [f64; 2],
    radii: [f64; 4],
    bevel: f64,
    softness: f64,
) -> (f64, [f64; 2]) {
    let corner = match (p[0] < 0.0, p[1] < 0.0) {
        (true, true) => 0,
        (false, true) => 1,
        (false, false) => 2,
        (true, false) => 3,
    };
    let r = radii[corner].clamp(0.0, half[0].min(half[1]));
    let q = [p[0].abs() - half[0] + r, p[1].abs() - half[1] + r];
    let outer = (bevel + softness).min(half[0].min(half[1])).max(bevel);
    let mut nearest = q[0].max(q[1]);
    if nearest < 0.0 {
        let depth_sum = -(q[0] + q[1]);
        let limit = 0.5 * outer;
        let remaining = outer - r - 0.5 * depth_sum;
        let fade = (remaining / limit).clamp(0.0, 1.0);
        let width = limit * depth_sum / (limit + depth_sum) * fade * fade * (3.0 - 2.0 * fade);
        if width > 0.0 {
            let overlap = (1.0 - (q[0] - q[1]).abs() / width).max(0.0);
            nearest += 0.25 * width * overlap * overlap;
        }
    }
    let distance = q[0].max(0.0).hypot(q[1].max(0.0)) + nearest.min(0.0) - r;
    let t = (1.0 + distance / bevel).clamp(0.0, 1.0);
    let edge = t * t * t * (t * (6.0 * t - 15.0) + 10.0);
    let core = edge * edge;
    let mix = 0.25 * (outer - bevel) / outer;
    let u = (1.0 + distance / outer).clamp(0.0, 1.0);
    let tail = u * u * u * (u * (6.0 * u - 15.0) + 10.0);
    let weight = core + (1.0 - core) * mix * tail * tail;
    let optical_radius = (r / outer).max(1.0).min(half[0].min(half[1]) / outer);
    let v = [0, 1].map(|i| {
        let x = ((p[i].abs() - half[i]) / outer + optical_radius).max(0.0);
        x * x / (x + 0.25)
    });
    let length = v[0].hypot(v[1]).max(0.0001);
    (
        weight,
        [0, 1].map(|i| -p[i].signum() * v[i] / length * weight),
    )
}

#[test]
fn blend_softness_adds_a_bounded_monotonic_tail_without_changing_rim_strength() {
    for softness in [0.0, 8.0, 16.0, 128.0] {
        let sample = |depth| {
            lens_soft(
                [100.0 - depth, 0.0],
                [100.0, 50.0],
                [12.0; 4],
                8.0,
                softness,
            )
        };
        assert_eq!(sample(0.0).0, 1.0);
        assert_eq!(sample(50.0).0, 0.0);
        let mut previous = 1.0;
        for step in 0..=500 {
            let (weight, displacement) = sample(step as f64 / 10.0);
            assert!((0.0..=1.0).contains(&weight));
            assert!(weight <= previous + 1e-12);
            assert!(displacement[0].hypot(displacement[1]) <= 1.0 + 1e-12);
            previous = weight;
        }
        if softness > 0.0 {
            let tail = sample(9.0);
            assert!(
                tail.0 > 0.0 && tail.1[0] != 0.0,
                "direction must extend with the tail"
            );
            let a = sample(8.0 - 1e-4).0;
            let b = sample(8.0).0;
            let c = sample(8.0 + 1e-4).0;
            assert!(((c - b) - (b - a)).abs() < 1e-7);
        }
        let base = lens_soft([92.0, 4.0], [100.0, 50.0], [12.0; 4], 8.0, softness);
        let scaled = lens_soft([138.0, 6.0], [150.0, 75.0], [18.0; 4], 12.0, softness * 1.5);
        assert!((base.0 - scaled.0).abs() < 1e-12);
        for i in 0..2 {
            assert!((base.1[i] - scaled.1[i]).abs() < 1e-12);
        }
    }
}

#[test]
fn actual_outline_and_equal_insets_match_straight_edges_and_each_corner() {
    let half = [200.0, 120.0];
    let radii = [12.0, 24.0, 48.0, 80.0];
    for (corner, signs) in [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]]
        .into_iter()
        .enumerate()
    {
        let r = radii[corner];
        for depth in [0.0, 1.0, 4.0, 8.0] {
            let t: f64 = 1.0 - depth / 18.0;
            let expected = (6.0 * t.powi(5) - 15.0 * t.powi(4) + 10.0 * t.powi(3)).powi(2);
            // Construct actual circle points independently of the SDF implementation.
            for angle in [0.0_f64, 0.2, 0.7, 1.2, std::f64::consts::FRAC_PI_2] {
                let p = [
                    signs[0] * (half[0] - r + (r - depth) * angle.cos()),
                    signs[1] * (half[1] - r + (r - depth) * angle.sin()),
                ];
                let (weight, bend) = lens(p, half, radii, 18.0);
                assert!((weight - expected).abs() < 1e-12);
                assert!((bend[0].hypot(bend[1]) - expected).abs() < 1e-12);
            }
            let (weight, _) = lens([half[0] - depth, 0.0], half, radii, 18.0);
            assert!((weight - expected).abs() < 1e-12);
        }
    }
}

#[test]
fn bending_stays_continuous_across_old_diagonal_ties_and_asymmetric_axes() {
    let half = [100.0, 100.0];
    let h = 1e-6;
    for radii in [[0.0; 4], [12.0; 4], [0.0, 12.0, 48.0, 100.0]] {
        for p in [
            [85.0, 85.0],
            [88.0, 88.0],
            [0.0, 95.0],
            [95.0, 0.0],
            [0.0, 0.0],
        ] {
            for axis in 0..2 {
                let mut lo = p;
                let mut hi = p;
                lo[axis] -= h;
                hi[axis] += h;
                let a = lens(lo, half, radii, 18.0).1;
                let b = lens(hi, half, radii, 18.0).1;
                for i in 0..2 {
                    assert!(
                        (a[i] - b[i]).abs() < 1e-5,
                        "{radii:?} {p:?}: {a:?} vs {b:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn lens_is_finite_bounded_and_centered_for_square_tiny_and_wide_windows() {
    for half in [[0.5_f64, 0.5], [20.0, 1000.0], [200.0, 120.0]] {
        for radius in [0.0, 12.0, half[0].min(half[1])] {
            for bevel in [0.5_f64, 18.0, 128.0] {
                let bevel = bevel.min(half[0].min(half[1]));
                assert_eq!(lens([0.0, 0.0], half, [radius; 4], bevel).1, [0.0, 0.0]);
                for x in -20..=20 {
                    for y in -20..=20 {
                        let p = [x as f64 / 20.0 * half[0], y as f64 / 20.0 * half[1]];
                        let (w, bend) = lens(p, half, [radius; 4], bevel);
                        assert!(w.is_finite() && (0.0..=1.0).contains(&w));
                        assert!(bend.iter().all(|x| x.is_finite()));
                        assert!(bend[0].hypot(bend[1]) <= 1.0 + 1e-12);
                    }
                }
            }
        }
    }
}

#[test]
fn tight_edge_fades_into_an_undistorted_interior_without_a_curvature_step() {
    let weight = |depth: f64| lens([100.0 - depth, 0.0], [100.0, 50.0], [12.0; 4], 8.0).0;
    assert_eq!(weight(0.0), 1.0);
    assert!((weight(4.0) - 0.25).abs() < 1e-12);
    assert!(weight(6.0) < 0.011);
    assert_eq!(weight(8.0), 0.0);
    assert_eq!(weight(24.0), 0.0);
    let mut last = 1.0;
    for step in 0..=800 {
        let next = weight(step as f64 / 100.0);
        assert!(next <= last + 1e-12);
        last = next;
    }
    // One-sided first/second derivatives agree with the constant regions at both ends.
    let h = 1e-3;
    for end in [0.0, 8.0] {
        let a = weight(end - h);
        let b = weight(end);
        let c = weight(end + h);
        assert!(((c - a) / (2.0 * h)).abs() < 1e-6);
        assert!(((a - 2.0 * b + c) / (h * h)).abs() < 1e-4);
    }
}

#[test]
fn lens_width_and_output_scale_preserve_physical_band() {
    let p = [94.0, 36.0];
    let half = [100.0, 50.0];
    let radii = [12.0; 4];
    let (weight, bend) = lens(p, half, radii, 18.0);
    assert!(lens(p, half, radii, 32.0).0 > weight);
    for scale in [1.25, 1.5, 2.0] {
        let (scaled_weight, scaled_bend) = lens(
            p.map(|x| x * scale),
            half.map(|x| x * scale),
            radii.map(|x| x * scale),
            18.0 * scale,
        );
        assert!((scaled_weight - weight).abs() < 1e-12);
        for i in 0..2 {
            assert!((scaled_bend[i] - bend[i]).abs() < 1e-12);
        }
    }
}

#[test]
fn broad_bevel_has_no_first_derivative_crease_on_interior_corner_diagonals() {
    // Continuity of values alone misses a visible crease: compare one-sided
    // derivatives of BOTH displacement components and the sharp-source weight.
    let half = [240.0, 160.0];
    for radius in [0.0, 12.0, 24.0] {
        for signs in [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]] {
            for depth in [radius + 2.0, radius + 8.0, radius + 16.0] {
                let sample = |offset: f64| {
                    let (weight, bend) = lens_soft(
                        [
                            signs[0] * (half[0] - depth + offset),
                            signs[1] * (half[1] - depth - offset),
                        ],
                        half,
                        [radius; 4],
                        52.0,
                        64.0,
                    );
                    [weight, bend[0], bend[1]]
                };
                let h = 1e-4;
                let lo = sample(-h);
                let at = sample(0.0);
                let hi = sample(h);
                for i in 0..3 {
                    let left = (at[i] - lo[i]) / h;
                    let right = (hi[i] - at[i]) / h;
                    assert!(
                        (left - right).abs() < 2e-5,
                        "radius={radius} depth={depth} component={i}: {left} != {right}"
                    );
                }
            }
        }
    }
}

#[test]
fn interior_fast_path_is_identical_to_the_full_corner_profile() {
    let mut skipped = 0;
    for half in [
        [0.5_f64, 0.5],
        [100.0, 100.0],
        [240.0, 160.0],
        [800.0, 40.0],
    ] {
        for r in [0.0_f64, 12.0, 80.0] {
            let r = r.min(half[0].min(half[1]));
            for bevel in [8.0_f64, 52.0, 128.0] {
                let bevel = bevel.min(half[0].min(half[1]));
                for softness in [0.0, 64.0, 128.0] {
                    let outer = (bevel + softness).min(half[0].min(half[1])).max(bevel);
                    for x in -32..=32 {
                        for y in -32..=32 {
                            let p = [x as f64 * half[0] / 32.0, y as f64 * half[1] / 32.0];
                            let nearest = (p[0].abs() - half[0] + r).max(p[1].abs() - half[1] + r);
                            if nearest <= 0.0_f64.min(r - outer) {
                                let reference = lens_soft(p, half, [r; 4], bevel, softness);
                                assert_eq!(reference, (0.0, [0.0, 0.0]));
                                skipped += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(skipped > 1000);
}

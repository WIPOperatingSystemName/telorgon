//! Analytic rectangle motion. One exp and sin_cos pair serves all four coordinates.
use crate::core::RectI;
use crate::{GeometryMotion, WindowTween};

#[derive(Clone, Copy, Debug)]
pub(in crate::application_host::shell_wayland) struct Sample {
    pub position: [f64; 4],
    pub velocity: [f64; 4],
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Spring;
    fn rect(width: i32) -> RectI {
        RectI {
            x: 10,
            y: 20,
            width,
            height: width / 2,
        }
    }
    fn spring() -> GeometryMotion {
        crate::WindowMotion::fluid().maximize_transition(true)
    }
    #[test]
    fn spring_matches_numerical_physics_and_allows_overshoot() {
        let mut track = GeometryTrack::new(Sample::at(rect(100)), rect(1000), spring(), 0, false);
        let initial = track.sample(0);
        assert_eq!(initial.position, Sample::at(rect(100)).position);
        assert!((initial.velocity[2] - 900.0 * 3.2).abs() < 1e-9);
        // Independent small-step RK4 integration of x'' = -2*zeta*omega*x' - omega^2*(x-target).
        let (mut x, mut v) = (100.0, 900.0 * 3.2);
        let dt = 0.0001;
        let acceleration = |x: f64, v: f64| -2.0 * 0.87 * 20.5 * v - 20.5 * 20.5 * (x - 1000.0);
        let mut overshot = false;
        for step in 1..4500 {
            let (k1x, k1v) = (v, acceleration(x, v));
            let (k2x, k2v) = (
                v + dt * k1v / 2.0,
                acceleration(x + dt * k1x / 2.0, v + dt * k1v / 2.0),
            );
            let (k3x, k3v) = (
                v + dt * k2v / 2.0,
                acceleration(x + dt * k2x / 2.0, v + dt * k2v / 2.0),
            );
            let (k4x, k4v) = (v + dt * k3v, acceleration(x + dt * k3x, v + dt * k3v));
            x += dt * (k1x + 2.0 * k2x + 2.0 * k3x + k4x) / 6.0;
            v += dt * (k1v + 2.0 * k2v + 2.0 * k3v + k4v) / 6.0;
            let value = track.sample(step * 100_000);
            assert!((value.position[2] - x).abs() < 1e-6);
            assert!((value.velocity[2] - v).abs() < 1e-5);
            overshot |= value.position[2] > 1000.0;
        }
        assert!(overshot);
        assert!(
            track.pending(),
            "a terminal sample still needs presentation"
        );
        let final_sample = track.sample(450_000_000);
        assert_eq!(final_sample.position, Sample::at(rect(1000)).position);
        assert_eq!(final_sample.velocity, [0.0; 4]);
        track.finish(450_000_000);
        assert!(!track.pending());
    }
    #[test]
    fn interruption_preserves_subpixel_position_and_vector_velocity() {
        let mut track = GeometryTrack::new(Sample::at(rect(100)), rect(1000), spring(), 0, false);
        let current = track.sample(123_456_789);
        let target = RectI {
            x: 80,
            y: -50,
            width: 300,
            height: 270,
        };
        let mut reversed = GeometryTrack::new(
            current,
            target,
            crate::WindowMotion::fluid().maximize_transition(false),
            123_456_789,
            true,
        );
        let start = reversed.sample(123_456_789);
        for i in 0..4 {
            assert!((start.position[i] - current.position[i]).abs() < 1e-9);
            assert!((start.velocity[i] - current.velocity[i]).abs() < 1e-9);
        }
    }
    #[test]
    fn sampling_is_independent_of_refresh_rate_and_translation_preserves_size_velocity() {
        let mut a = GeometryTrack::new(
            Sample::at(rect(100)),
            rect(1000),
            spring(),
            50_000_000,
            false,
        );
        let mut b = GeometryTrack::new(
            Sample::at(rect(100)),
            rect(1000),
            spring(),
            50_000_000,
            false,
        );
        assert_eq!(
            a.sample(25_000_000).position,
            Sample::at(rect(100)).position
        );
        for i in 0..20 {
            a.sample(i * 10_000_000);
        }
        let expected = a.sample(200_000_000);
        assert_eq!(expected.position, b.sample(200_000_000).position);
        b.translate(100, -20);
        let translated = b.sample(200_000_000);
        assert_eq!(translated.position[0], expected.position[0] + 100.0);
        assert_eq!(translated.position[1], expected.position[1] - 20.0);
        assert_eq!(translated.velocity, expected.velocity);
        assert_eq!(translated.position[2..], expected.position[2..]);
    }
    #[test]
    fn zero_duration_and_parameter_extremes_remain_finite() {
        for damping in [0.000001, 0.86, 0.999999] {
            for frequency in [0.1, 20.0, 1000.0] {
                let motion = GeometryMotion::Spring(
                    Spring::new()
                        .damping_ratio(damping)
                        .angular_frequency(frequency),
                );
                let mut track =
                    GeometryTrack::new(Sample::at(rect(100)), rect(1000), motion, 0, false);
                assert!(
                    track
                        .sample(100_000_000)
                        .position
                        .iter()
                        .all(|x| x.is_finite())
                );
                let mut immediate = GeometryTrack::new(
                    Sample::at(rect(100)),
                    rect(1000),
                    motion.with_duration_ms(0),
                    0,
                    false,
                );
                assert_eq!(immediate.sample(0).rect(), rect(1000));
                assert!(!immediate.pending());
            }
        }
    }
    #[test]
    fn eased_spring_softens_start_preserves_bounce_and_retarget_velocity() {
        let motion = GeometryMotion::Spring(
            Spring::new()
                .easing(crate::Easing::EaseInOut)
                .damping_ratio(0.6)
                .settle_within_ms(700),
        );
        let mut track = GeometryTrack::new(Sample::at(rect(100)), rect(1000), motion, 0, false);
        assert_eq!(track.sample(0).velocity, [0.0; 4]);
        let mut overshot = false;
        for milliseconds in 1..700 {
            let value = track.sample(milliseconds * 1_000_000);
            overshot |= value.position[2] > 1000.0;
        }
        assert!(overshot);
        let now = 200_000_000;
        let current = track.sample(now);
        let before = track.sample(now - 1000);
        let after = track.sample(now + 1000);
        for i in 0..4 {
            let numerical_velocity = (after.position[i] - before.position[i]) / 0.000002;
            assert!((numerical_velocity - current.velocity[i]).abs() < 0.001);
        }
        let mut reversed = GeometryTrack::new(current, rect(100), motion, now, true);
        let start = reversed.sample(now);
        for i in 0..4 {
            assert!((start.position[i] - current.position[i]).abs() < 1e-9);
            assert!((start.velocity[i] - current.velocity[i]).abs() < 1e-9);
        }
        assert_eq!(track.sample(700_000_000).velocity, [0.0; 4]);
        assert_eq!(track.sample(700_000_000).rect(), rect(1000));
        let mut immediate = GeometryTrack::new(
            Sample::at(rect(100)),
            rect(1000),
            motion.with_duration_ms(0),
            0,
            false,
        );
        assert_eq!(immediate.sample(0).rect(), rect(1000));
    }

    #[test]
    fn spring_easing_derivatives_match_finite_differences() {
        for easing in [
            crate::Easing::Linear,
            crate::Easing::EaseIn,
            crate::Easing::EaseOut,
            crate::Easing::EaseInOut,
        ] {
            for time in [0.01, 0.17, 0.35, 0.53, 0.69] {
                let (_, rate) = spring_time(easing, time, 0.7);
                let numerical = (spring_time(easing, time + 1e-7, 0.7).0
                    - spring_time(easing, time - 1e-7, 0.7).0)
                    / 2e-7;
                assert!((rate - numerical).abs() < 1e-5);
            }
        }
    }

    #[test]
    #[ignore = "CPU sampler microbenchmark; run explicitly with --release --ignored --nocapture"]
    fn spring_sampler_cpu_throughput() {
        let mut track = GeometryTrack::new(Sample::at(rect(100)), rect(1000), spring(), 0, false);
        let count = 500_000_u64;
        let started = std::time::Instant::now();
        for i in 0..count {
            std::hint::black_box(track.sample(std::hint::black_box(i * 7919 % 440_000_000)));
        }
        eprintln!(
            "spring rectangle sampler: {:.1} ns/sample over {count} samples (position + velocity, CPU only)",
            started.elapsed().as_nanos() as f64 / count as f64
        );
    }
}
impl Sample {
    pub fn rect(self) -> RectI {
        RectI {
            x: self.position[0].round() as i32,
            y: self.position[1].round() as i32,
            width: self.position[2].round().max(1.0) as i32,
            height: self.position[3].round().max(1.0) as i32,
        }
    }
    pub fn at(rect: RectI) -> Self {
        Self {
            position: [
                rect.x as f64,
                rect.y as f64,
                rect.width as f64,
                rect.height as f64,
            ],
            velocity: [0.0; 4],
        }
    }
}
enum Curve {
    Tween(WindowTween),
    Spring {
        easing: crate::Easing,
        decay: f64,
        frequency: f64,
        a: [f64; 4],
        b: [f64; 4],
    },
}
pub(in crate::application_host::shell_wayland) struct GeometryTrack {
    from: Sample,
    target: [f64; 4],
    curve: Curve,
    pub start: u64,
    pub duration_ms: u32,
    pending: bool,
    cache: Option<(u64, Sample)>,
}
impl GeometryTrack {
    pub fn fixed(rect: RectI) -> Self {
        Self::new(
            Sample::at(rect),
            rect,
            GeometryMotion::Tween(crate::tween_ms(0, crate::Easing::Linear)),
            0,
            false,
        )
    }
    pub fn new(
        mut from: Sample,
        target: RectI,
        motion: GeometryMotion,
        start: u64,
        inherit_velocity: bool,
    ) -> Self {
        let target = Sample::at(target).position;
        let curve = match motion {
            GeometryMotion::Tween(tween) => Curve::Tween(tween),
            GeometryMotion::Spring(spring) => {
                let decay = spring.damping_ratio * spring.angular_frequency;
                let frequency = spring.angular_frequency
                    * (1.0 - spring.damping_ratio * spring.damping_ratio).sqrt();
                if !inherit_velocity {
                    from.velocity = std::array::from_fn(|i| {
                        (target[i] - from.position[i]) * spring.initial_velocity
                    });
                }
                let a = std::array::from_fn(|i| from.position[i] - target[i]);
                let b = std::array::from_fn(|i| (from.velocity[i] + decay * a[i]) / frequency);
                Curve::Spring {
                    easing: if inherit_velocity {
                        crate::Easing::Linear
                    } else {
                        spring.easing
                    },
                    decay,
                    frequency,
                    a,
                    b,
                }
            }
        };
        Self {
            pending: motion.duration_ms() != 0
                && (from.position != target || from.velocity != [0.0; 4]),
            from,
            target,
            curve,
            start,
            duration_ms: motion.duration_ms(),
            cache: None,
        }
    }
    pub fn pending(&self) -> bool {
        self.pending
    }
    pub fn active(&self, now: u64) -> bool {
        self.pending
            && now
                < self
                    .start
                    .saturating_add(u64::from(self.duration_ms) * 1_000_000)
    }
    pub fn sample(&mut self, now: u64) -> Sample {
        if let Some((time, sample)) = self.cache
            && time == now
        {
            return sample;
        }
        let value = if !self.active(now) {
            Sample {
                position: self.target,
                velocity: [0.0; 4],
            }
        } else if now < self.start {
            Sample {
                position: self.from.position,
                velocity: [0.0; 4],
            }
        } else {
            let time = now.saturating_sub(self.start) as f64 * 1e-9;
            match self.curve {
                Curve::Tween(tween) => {
                    let seconds = f64::from(tween.duration_ms) * 0.001;
                    let t = (time / seconds) as f32;
                    let p = f64::from(tween.easing.sample(t));
                    // Needed only to seed a spring when switching from an in-flight tween.
                    let step = 0.0001_f32;
                    let lo = (t - step).max(0.0);
                    let hi = (t + step).min(1.0);
                    let speed = f64::from(tween.easing.sample(hi) - tween.easing.sample(lo))
                        / f64::from(hi - lo)
                        / seconds;
                    Sample {
                        position: std::array::from_fn(|i| {
                            self.from.position[i] + (self.target[i] - self.from.position[i]) * p
                        }),
                        velocity: std::array::from_fn(|i| {
                            (self.target[i] - self.from.position[i]) * speed
                        }),
                    }
                }
                Curve::Spring {
                    easing,
                    decay,
                    frequency,
                    a,
                    b,
                } => {
                    let (time, rate) =
                        spring_time(easing, time, f64::from(self.duration_ms) * 0.001);
                    let envelope = (-decay * time).exp();
                    let (sin, cos) = (frequency * time).sin_cos();
                    let displacement: [f64; 4] = std::array::from_fn(|i| a[i] * cos + b[i] * sin);
                    Sample {
                        position: std::array::from_fn(|i| {
                            self.target[i] + envelope * displacement[i]
                        }),
                        velocity: std::array::from_fn(|i| {
                            rate * envelope
                                * (frequency * (-a[i] * sin + b[i] * cos) - decay * displacement[i])
                        }),
                    }
                }
            }
        };
        self.cache = Some((now, value));
        value
    }
    pub fn translate(&mut self, x: i32, y: i32) {
        for (i, delta) in [x, y].into_iter().enumerate() {
            self.from.position[i] += f64::from(delta);
            self.target[i] += f64::from(delta);
        }
        self.cache = None;
    }
    pub fn finish(&mut self, now: u64) {
        if !self.active(now) {
            self.pending = false;
        }
    }
}

// Return warped time and its exact derivative so interruption inherits physical velocity.
fn spring_time(easing: crate::Easing, time: f64, duration: f64) -> (f64, f64) {
    use crate::Easing;
    if easing == Easing::Linear {
        return (time, 1.0);
    }
    let t = (time / duration).clamp(0.0, 1.0);
    let (progress, rate) = match easing {
        Easing::Linear => unreachable!(),
        Easing::EaseIn => (t * t * t, 3.0 * t * t),
        Easing::EaseOut => (1.0 - (1.0 - t).powi(3), 3.0 * (1.0 - t).powi(2)),
        Easing::EaseInOut if t < 0.5 => (4.0 * t * t * t, 12.0 * t * t),
        Easing::EaseInOut => (1.0 - 4.0 * (1.0 - t).powi(3), 12.0 * (1.0 - t).powi(2)),
    };
    (duration * progress, rate)
}

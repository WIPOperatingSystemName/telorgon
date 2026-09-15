//! Declarative desktop window motion. Durations are milliseconds; tracks never repeat.
use crate::theme::Easing;

/// Underdamped geometry spring. Frequency is radians/second; initial velocity is
/// normalized distance/second and applies only when starting from rest.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    pub(crate) initial_velocity: f64,
    pub(crate) damping_ratio: f64,
    pub(crate) angular_frequency: f64,
    pub(crate) settle_ms: u32,
}
impl Default for Spring {
    fn default() -> Self {
        Self::new()
    }
}
impl Spring {
    pub const fn new() -> Self {
        Self {
            initial_velocity: 3.0,
            damping_ratio: 0.86,
            angular_frequency: 20.0,
            settle_ms: 460,
        }
    }
    pub const fn initial_velocity(mut self, value: f64) -> Self {
        assert!(
            value >= -100.0 && value <= 100.0,
            "spring velocity must be between -100 and 100"
        );
        self.initial_velocity = value;
        self
    }
    pub const fn damping_ratio(mut self, value: f64) -> Self {
        assert!(
            value > 0.0 && value < 1.0,
            "spring damping ratio must be between zero and one"
        );
        self.damping_ratio = value;
        self
    }
    pub const fn angular_frequency(mut self, value: f64) -> Self {
        assert!(
            value >= 0.1 && value <= 1000.0,
            "spring frequency must be between 0.1 and 1000 radians/second"
        );
        self.angular_frequency = value;
        self
    }
    /// Maximum sampling time. Zero settles immediately; short cutoffs can visibly snap.
    pub const fn settle_within_ms(mut self, value: u32) -> Self {
        self.settle_ms = value;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GeometryMotion {
    Tween(WindowTween),
    Spring(Spring),
}
impl GeometryMotion {
    pub const fn duration_ms(self) -> u32 {
        match self {
            Self::Tween(t) => t.duration_ms,
            Self::Spring(s) => s.settle_ms,
        }
    }
    #[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
    pub(crate) const fn with_duration_ms(self, duration: u32) -> Self {
        match self {
            Self::Tween(t) => Self::Tween(tween_ms(duration, t.easing)),
            Self::Spring(s) => Self::Spring(s.settle_within_ms(duration)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowTween {
    pub duration_ms: u32,
    pub easing: Easing,
}
pub const fn tween_ms(duration_ms: u32, easing: Easing) -> WindowTween {
    WindowTween {
        duration_ms,
        easing,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Minimize {
    pub(crate) tween: WindowTween,
}
impl Minimize {
    /// A centered shrink to 92% size, with group opacity fading to zero.
    pub const fn shrink_and_fade(duration_ms: u32) -> Self {
        Self {
            tween: tween_ms(duration_ms, Easing::EaseOut),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContentFade {
    pub(crate) entry: WindowTween,
    pub(crate) exit: WindowTween,
}
impl ContentFade {
    pub const fn new(to_placeholder_ms: u32, to_ready_ms: u32) -> Self {
        Self {
            entry: tween_ms(to_placeholder_ms, Easing::EaseOut),
            exit: tween_ms(to_ready_ms, Easing::EaseOut),
        }
    }

    /// Sets the easing toward the resize placeholder, preserving its duration.
    pub const fn to_placeholder_easing(mut self, easing: Easing) -> Self {
        self.entry.easing = easing;
        self
    }

    /// Sets the easing back to ready content, preserving its duration.
    pub const fn to_ready_easing(mut self, easing: Easing) -> Self {
        self.exit.easing = easing;
        self
    }
}

/// One style for window state changes and content handoffs. This does not own a clock.
/// Native OS window animations are outside the desktop compositor's control.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowMotion {
    maximize: GeometryMotion,
    restore: Option<GeometryMotion>,
    minimize: Minimize,
    unminimize: Option<Minimize>,
    content: ContentFade,
    maximize_content: ContentFade,
}
impl Default for WindowMotion {
    fn default() -> Self {
        Self::smooth()
    }
}
impl WindowMotion {
    pub const fn smooth() -> Self {
        Self {
            maximize: GeometryMotion::Tween(tween_ms(240, Easing::EaseOut)),
            restore: None,
            minimize: Minimize::shrink_and_fade(180),
            unminimize: None,
            content: ContentFade::new(90, 130),
            maximize_content: ContentFade::new(50, 130),
        }
    }
    /// Fluid maximize/restore presets from the spring specification; other effects stay smooth.
    pub const fn fluid() -> Self {
        Self::smooth()
            .maximize_spring(
                Spring::new()
                    .initial_velocity(3.2)
                    .damping_ratio(0.87)
                    .angular_frequency(20.5)
                    .settle_within_ms(450),
            )
            .restore_spring(
                Spring::new()
                    .initial_velocity(2.8)
                    .damping_ratio(0.86)
                    .angular_frequency(19.5)
                    .settle_within_ms(475),
            )
    }
    pub const fn none() -> Self {
        Self {
            maximize: GeometryMotion::Tween(tween_ms(0, Easing::Linear)),
            restore: None,
            minimize: Minimize::shrink_and_fade(0),
            unminimize: None,
            content: ContentFade::new(0, 0),
            maximize_content: ContentFade::new(0, 0),
        }
    }
    /// Sets both directions unless an explicit restore override is present.
    pub const fn maximize(mut self, tween: WindowTween) -> Self {
        self.maximize = GeometryMotion::Tween(tween);
        self
    }
    pub const fn restore(mut self, tween: WindowTween) -> Self {
        self.restore = Some(GeometryMotion::Tween(tween));
        self
    }
    pub const fn maximize_spring(mut self, spring: Spring) -> Self {
        self.maximize = GeometryMotion::Spring(spring);
        self
    }
    pub const fn restore_spring(mut self, spring: Spring) -> Self {
        self.restore = Some(GeometryMotion::Spring(spring));
        self
    }
    pub const fn minimize(mut self, effect: Minimize) -> Self {
        self.minimize = effect;
        self
    }
    pub const fn unminimize(mut self, effect: Minimize) -> Self {
        self.unminimize = Some(effect);
        self
    }
    pub const fn resize_content(mut self, fade: ContentFade) -> Self {
        self.content = fade;
        self
    }
    /// Entry fade before maximize/restore movement, and reveal after movement and app readiness.
    pub const fn maximize_content(mut self, fade: ContentFade) -> Self {
        self.maximize_content = fade;
        self
    }
    pub const fn maximize_content_transition(self, placeholder: bool) -> WindowTween {
        if placeholder {
            self.maximize_content.entry
        } else {
            self.maximize_content.exit
        }
    }
    pub const fn maximize_transition(self, maximized: bool) -> GeometryMotion {
        if maximized {
            self.maximize
        } else {
            match self.restore {
                Some(t) => t,
                None => self.maximize,
            }
        }
    }
    pub const fn minimize_transition(self, minimized: bool) -> WindowTween {
        if minimized {
            self.minimize.tween
        } else {
            match self.unminimize {
                Some(t) => t.tween,
                None => self.minimize.tween,
            }
        }
    }
    pub const fn content_transition(self, placeholder: bool) -> WindowTween {
        if placeholder {
            self.content.entry
        } else {
            self.content.exit
        }
    }
    pub fn enabled(self) -> bool {
        if self.maximize_transition(true).duration_ms() != 0
            || self.maximize_transition(false).duration_ms() != 0
        {
            return true;
        }
        [
            self.minimize_transition(true),
            self.minimize_transition(false),
            self.content.entry,
            self.content.exit,
            self.maximize_content.entry,
            self.maximize_content.exit,
        ]
        .iter()
        .any(|t| t.duration_ms != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fluid_presets_and_mixed_overrides_are_const_and_independent() {
        const FLUID: WindowMotion = WindowMotion::fluid();
        assert_eq!(FLUID.maximize_transition(true).duration_ms(), 450);
        assert_eq!(FLUID.maximize_transition(false).duration_ms(), 475);
        let spring = Spring::new()
            .initial_velocity(4.0)
            .damping_ratio(0.9)
            .angular_frequency(22.0)
            .settle_within_ms(500);
        const MIXED: WindowMotion = WindowMotion::fluid().restore(tween_ms(240, Easing::EaseOut));
        assert!(matches!(
            MIXED.maximize_transition(true),
            GeometryMotion::Spring(_)
        ));
        assert!(matches!(
            MIXED.maximize_transition(false),
            GeometryMotion::Tween(_)
        ));
        assert_eq!(
            WindowMotion::smooth()
                .restore_spring(spring)
                .maximize(tween_ms(100, Easing::Linear)),
            WindowMotion::smooth()
                .maximize(tween_ms(100, Easing::Linear))
                .restore_spring(spring)
        );
        assert_eq!(
            WindowMotion::smooth()
                .maximize_spring(spring)
                .maximize_transition(false),
            GeometryMotion::Spring(spring)
        );
        assert!(
            !WindowMotion::none()
                .maximize_spring(spring.settle_within_ms(0))
                .enabled()
        );
    }
    #[test]
    fn spring_builders_reject_invalid_parameters() {
        for value in [f64::NAN, f64::INFINITY, -1.0, 0.0, 1.0] {
            assert!(std::panic::catch_unwind(|| Spring::new().damping_ratio(value)).is_err());
        }
        for value in [f64::NAN, f64::INFINITY, -1.0, 0.0] {
            assert!(std::panic::catch_unwind(|| Spring::new().angular_frequency(value)).is_err());
        }
        assert!(std::panic::catch_unwind(|| Spring::new().initial_velocity(f64::NAN)).is_err());
    }
    #[test]
    fn paired_defaults_and_overrides_are_order_independent() {
        let a = tween_ms(120, Easing::Linear);
        let b = tween_ms(200, Easing::EaseOut);
        let base = WindowMotion::smooth();
        assert_eq!(
            base.maximize(a).maximize_transition(false),
            GeometryMotion::Tween(a)
        );
        assert_eq!(base.restore(b).maximize(a), base.maximize(a).restore(b));
        let a = Minimize::shrink_and_fade(120);
        let b = Minimize::shrink_and_fade(200);
        assert_eq!(base.minimize(a).minimize_transition(false), a.tween);
        assert_eq!(
            base.unminimize(b).minimize(a),
            base.minimize(a).unminimize(b)
        );
        assert!(!WindowMotion::none().enabled());
        assert!(WindowMotion::default().enabled());
    }
}

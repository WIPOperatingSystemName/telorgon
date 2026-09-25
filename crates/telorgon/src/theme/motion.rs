//! Validated transition values shared by theme compilation and per-view animation tracks.

#[derive(
    serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, Hash,
)]
pub enum Easing {
    Linear,
    EaseIn,
    #[default]
    EaseOut,
    EaseInOut,
}

impl Easing {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "linear" => Some(Self::Linear),
            "ease-in" => Some(Self::EaseIn),
            "ease-out" => Some(Self::EaseOut),
            "ease-in-out" => Some(Self::EaseInOut),
            _ => None,
        }
    }

    pub fn sample(self, progress: f32) -> f32 {
        let t = progress.clamp(0.0, 1.0);
        match self {
            Self::Linear => t,
            Self::EaseIn => t * t * t,
            Self::EaseOut => 1.0 - (1.0 - t).powi(3),
            Self::EaseInOut if t < 0.5 => 4.0 * t * t * t,
            Self::EaseInOut => 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0,
        }
    }
}

#[derive(
    serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, Hash,
)]
pub struct TransitionSpec {
    pub duration_ms: u32,
    pub easing: Easing,
    pub repeat: bool,
}

#[derive(
    serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, Hash,
)]
pub enum MotionPreference {
    #[default]
    Full,
    Reduced,
}

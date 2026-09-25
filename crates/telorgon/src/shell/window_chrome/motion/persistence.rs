use super::{Easing, Spring};
use serde::{Deserialize, Deserializer, de::Error};

impl<'de> Deserialize<'de> for Spring {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Saved {
            initial_velocity: f64,
            damping_ratio: f64,
            angular_frequency: f64,
            settle_ms: u32,
            easing: Easing,
        }
        let saved = Saved::deserialize(deserializer)?;
        if !(-100.0..=100.0).contains(&saved.initial_velocity)
            || !(saved.damping_ratio > 0.0 && saved.damping_ratio < 1.0)
            || !(0.1..=1000.0).contains(&saved.angular_frequency)
        {
            return Err(D::Error::custom(
                "invalid spring velocity, damping ratio, or frequency",
            ));
        }
        Ok(Self {
            initial_velocity: saved.initial_velocity,
            damping_ratio: saved.damping_ratio,
            angular_frequency: saved.angular_frequency,
            settle_ms: saved.settle_ms,
            easing: saved.easing,
        })
    }
}


pub(super) fn default_open_scale() -> f32 { 1.0 }

pub(super) fn deserialize_open_scale<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f32, D::Error> {
    let scale = f32::deserialize(deserializer)?;
    if scale > 0.0 && scale <= 1.0 {
        Ok(scale)
    } else {
        Err(D::Error::custom("opening scale must be greater than zero and at most one"))
    }
}

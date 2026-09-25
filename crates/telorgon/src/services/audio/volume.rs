use crate::integrations::pipewire::MediaError;
/// Linear amplitude: 0 is silence, 1 is unity (0 dB). Upper bound 16 (~24 dB).
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct Gain(f32);
impl Gain {
    pub const SILENCE: Self = Self(0.0);
    pub const UNITY: Self = Self(1.0);
    pub fn linear(value: f32) -> Result<Self, MediaError> {
        if !value.is_finite() || !(0.0..=16.0).contains(&value) {
            return Err(MediaError::InvalidArgument("linear gain"));
        }
        Ok(Self(value))
    }
    pub fn decibels(db: f32) -> Result<Self, MediaError> {
        if db == f32::NEG_INFINITY {
            Ok(Self::SILENCE)
        } else {
            Self::linear(10.0_f32.powf(db / 20.0))
        }
    }
    /// Cubic UI mapping. 100% = unity; amplification still needs explicit policy on write.
    pub fn ui(value: f32) -> Result<Self, MediaError> {
        if !value.is_finite() || value < 0.0 {
            return Err(MediaError::InvalidArgument("UI volume"));
        }
        Self::linear(value.powi(3))
    }
    pub fn value(self) -> f32 {
        self.0
    }
    pub fn as_decibels(self) -> f32 {
        20.0 * self.0.log10()
    }
    pub fn as_ui(self) -> f32 {
        self.0.cbrt()
    }
}
#[derive(Clone, Copy, Debug)]
pub enum Amplification {
    Forbid,
    AllowUpTo(Gain),
}
impl Amplification {
    pub(crate) fn validate(self, gain: Gain) -> Result<(), MediaError> {
        let limit = match self {
            Self::Forbid => 1.0,
            Self::AllowUpTo(g) => g.value(),
        };
        if gain.value() > limit {
            Err(MediaError::InvalidArgument("amplification policy"))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn units_and_amplification_reject_nonfinite_and_overlimit_writes() {
        assert_eq!(Gain::ui(0.5).unwrap().value(), 0.125);
        assert!((Gain::decibels(-6.0206).unwrap().value() - 0.5).abs() < 0.0001);
        assert_eq!(Gain::SILENCE.as_decibels(), f32::NEG_INFINITY);
        assert!(Gain::linear(f32::NAN).is_err());
        assert!(Gain::ui(-1.0).is_err());
        let gain = Gain::linear(1.5).unwrap();
        assert!(Amplification::Forbid.validate(gain).is_err());
        assert!(
            Amplification::AllowUpTo(Gain::linear(2.0).unwrap())
                .validate(gain)
                .is_ok()
        );
    }
}

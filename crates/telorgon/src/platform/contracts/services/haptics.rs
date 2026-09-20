//! Platform-neutral semantic haptic-effect admission.
//!
//! Portable code selects a semantic effect and a bounded normalized intensity. Capability
//! snapshots explicitly report current output-device support and the observed user-setting state.
//! This module accepts no waveform, frequency, duration, vendor identifier, or native device
//! handle, and it owns no hardware driver, callback, queue, executor, thread, timer, or event loop.

use std::error::Error;
use std::fmt;
use std::rc::Rc;

use super::ServiceKey;
use crate::platform::contracts::{
    CapabilityDescriptor, ExecutionRequirement, PermissionState, RequestAdmission, Support,
    UserGestureGrantHandle, UserGestureRequirement,
};

/// Fixed-point units used to represent the closed normalized intensity range `0.0..=1.0`.
pub const HAPTIC_INTENSITY_UNITS: u16 = 1_000;

/// A semantic haptic effect portable code may request.
///
/// Variants describe user-facing meaning rather than a waveform or device implementation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum HapticEffect {
    Selection,
    Activation,
    Toggle,
    LightImpact,
    MediumImpact,
    HeavyImpact,
    Success,
    Warning,
    Error,
}

impl HapticEffect {
    const fn mask(self) -> u16 {
        1 << (self as u8)
    }
}

const ALL_HAPTIC_EFFECTS_MASK: u16 = HapticEffect::Selection.mask()
    | HapticEffect::Activation.mask()
    | HapticEffect::Toggle.mask()
    | HapticEffect::LightImpact.mask()
    | HapticEffect::MediumImpact.mask()
    | HapticEffect::HeavyImpact.mask()
    | HapticEffect::Success.mask()
    | HapticEffect::Warning.mask()
    | HapticEffect::Error.mask();

/// Exact set of semantic effects supported by one current haptic output device.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct HapticEffectSupport(u16);

impl HapticEffectSupport {
    pub const fn none() -> Self {
        Self(0)
    }

    pub const fn all() -> Self {
        Self(ALL_HAPTIC_EFFECTS_MASK)
    }

    pub const fn only(effect: HapticEffect) -> Self {
        Self(effect.mask())
    }

    #[must_use]
    pub const fn with(self, effect: HapticEffect) -> Self {
        Self(self.0 | effect.mask())
    }

    #[must_use]
    pub const fn without(self, effect: HapticEffect) -> Self {
        Self(self.0 & !effect.mask())
    }

    pub const fn supports(self, effect: HapticEffect) -> bool {
        self.0 & effect.mask() != 0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn count(self) -> u32 {
        self.0.count_ones()
    }
}

impl fmt::Debug for HapticEffectSupport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HapticEffectSupport")
            .field("effect_count", &self.count())
            .field("selection", &self.supports(HapticEffect::Selection))
            .field("activation", &self.supports(HapticEffect::Activation))
            .field("toggle", &self.supports(HapticEffect::Toggle))
            .field("light_impact", &self.supports(HapticEffect::LightImpact))
            .field("medium_impact", &self.supports(HapticEffect::MediumImpact))
            .field("heavy_impact", &self.supports(HapticEffect::HeavyImpact))
            .field("success", &self.supports(HapticEffect::Success))
            .field("warning", &self.supports(HapticEffect::Warning))
            .field("error", &self.supports(HapticEffect::Error))
            .finish()
    }
}

/// Normalized haptic strength represented exactly in thousandths.
///
/// The representation is finite and bounded by construction. It deliberately carries no duration
/// or device amplitude unit.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HapticIntensity(u16);

impl HapticIntensity {
    pub const SILENT: Self = Self(0);
    pub const FULL: Self = Self(HAPTIC_INTENSITY_UNITS);

    pub const fn from_units(units: u16) -> Result<Self, HapticIntensityError> {
        if units > HAPTIC_INTENSITY_UNITS {
            return Err(HapticIntensityError::AboveNormalizedMaximum { units });
        }
        Ok(Self(units))
    }

    pub fn from_normalized(value: f32) -> Result<Self, HapticIntensityError> {
        if !value.is_finite() {
            return Err(HapticIntensityError::NotFinite);
        }
        if !(0.0..=1.0).contains(&value) {
            return Err(HapticIntensityError::OutsideNormalizedRange);
        }
        Ok(Self(
            (value * f32::from(HAPTIC_INTENSITY_UNITS)).round() as u16
        ))
    }

    pub const fn units(self) -> u16 {
        self.0
    }

    pub fn normalized(self) -> f32 {
        f32::from(self.0) / f32::from(HAPTIC_INTENSITY_UNITS)
    }

    pub const fn is_silent(self) -> bool {
        self.0 == 0
    }
}

/// Invalid normalized haptic intensity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HapticIntensityError {
    NotFinite,
    OutsideNormalizedRange,
    AboveNormalizedMaximum { units: u16 },
}

impl fmt::Display for HapticIntensityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotFinite => "haptic intensity is not finite",
            Self::OutsideNormalizedRange => "haptic intensity is outside 0.0 through 1.0",
            Self::AboveNormalizedMaximum { .. } => {
                "haptic intensity units exceed the normalized hard bound"
            }
        })
    }
}

impl Error for HapticIntensityError {}

/// Current output-device support observed by the host adapter.
///
/// `Unavailable` can describe a temporarily absent/disconnected output while the service family
/// itself remains implemented. An available output must advertise at least one semantic effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HapticDeviceSupport {
    Unavailable,
    Available {
        effects: HapticEffectSupport,
        intensity_control: bool,
    },
}

impl HapticDeviceSupport {
    pub const fn available(
        effects: HapticEffectSupport,
        intensity_control: bool,
    ) -> Result<Self, HapticDeviceSupportError> {
        if effects.is_empty() {
            return Err(HapticDeviceSupportError::NoSemanticEffects);
        }
        Ok(Self::Available {
            effects,
            intensity_control,
        })
    }

    pub const fn is_available(self) -> bool {
        matches!(self, Self::Available { .. })
    }

    pub const fn effects(self) -> Option<HapticEffectSupport> {
        match self {
            Self::Unavailable => None,
            Self::Available { effects, .. } => Some(effects),
        }
    }

    pub const fn supports(self, effect: HapticEffect) -> bool {
        match self {
            Self::Unavailable => false,
            Self::Available { effects, .. } => effects.supports(effect),
        }
    }

    pub const fn supports_intensity_control(self) -> bool {
        match self {
            Self::Unavailable => false,
            Self::Available {
                intensity_control, ..
            } => intensity_control,
        }
    }
}

/// Invalid output-device support description.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HapticDeviceSupportError {
    NoSemanticEffects,
}

impl fmt::Display for HapticDeviceSupportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("available haptic device supports no semantic effects")
    }
}

impl Error for HapticDeviceSupportError {}

/// Host-observed user preference controlling haptic feedback.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum HapticUserSettingState {
    Enabled,
    Disabled,
    #[default]
    Unknown,
}

impl HapticUserSettingState {
    pub const fn allows_feedback(self) -> bool {
        matches!(self, Self::Enabled)
    }
}

/// Independently discoverable haptic-service operations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct HapticOperations {
    trigger: bool,
}

impl HapticOperations {
    pub const fn new(trigger: bool) -> Self {
        Self { trigger }
    }

    pub const fn supports_trigger(self) -> bool {
        self.trigger
    }
}

/// Adapter-narrowed intensity limit for the current output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HapticLimits {
    maximum_intensity: HapticIntensity,
}

impl HapticLimits {
    pub const fn new(maximum_intensity: HapticIntensity) -> Result<Self, HapticLimitError> {
        if maximum_intensity.is_silent() {
            return Err(HapticLimitError::SilentMaximum);
        }
        Ok(Self { maximum_intensity })
    }

    pub const fn maximum_intensity(self) -> HapticIntensity {
        self.maximum_intensity
    }
}

impl Default for HapticLimits {
    fn default() -> Self {
        Self {
            maximum_intensity: HapticIntensity::FULL,
        }
    }
}

/// Invalid adapter-advertised haptic limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HapticLimitError {
    SilentMaximum,
}

impl fmt::Display for HapticLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("haptic maximum intensity must be greater than zero")
    }
}

impl Error for HapticLimitError {}

/// Complete current haptic capability, including device and user-setting observations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HapticCapability {
    descriptor: CapabilityDescriptor<HapticOperations, HapticLimits>,
    device: HapticDeviceSupport,
    user_setting: HapticUserSettingState,
}

impl HapticCapability {
    pub const fn new(
        descriptor: CapabilityDescriptor<HapticOperations, HapticLimits>,
        device: HapticDeviceSupport,
        user_setting: HapticUserSettingState,
    ) -> Result<Self, HapticCapabilityError> {
        if matches!(
            device,
            HapticDeviceSupport::Available {
                intensity_control: false,
                ..
            }
        ) && descriptor.limits().maximum_intensity().units() != HapticIntensity::FULL.units()
        {
            return Err(HapticCapabilityError::FixedIntensityRequiresFullLimit);
        }
        Ok(Self {
            descriptor,
            device,
            user_setting,
        })
    }

    pub const fn operations(&self) -> &HapticOperations {
        self.descriptor.operations()
    }

    pub const fn limits(&self) -> &HapticLimits {
        self.descriptor.limits()
    }

    pub const fn permission(self) -> PermissionState {
        self.descriptor.permission()
    }

    pub const fn execution(self) -> ExecutionRequirement {
        self.descriptor.execution()
    }

    pub const fn user_gesture(self) -> UserGestureRequirement {
        self.descriptor.user_gesture()
    }

    pub const fn device(self) -> HapticDeviceSupport {
        self.device
    }

    pub const fn user_setting(self) -> HapticUserSettingState {
        self.user_setting
    }

    pub fn into_descriptor(self) -> CapabilityDescriptor<HapticOperations, HapticLimits> {
        self.descriptor
    }
}

/// Invalid relationship within a haptic capability snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HapticCapabilityError {
    FixedIntensityRequiresFullLimit,
}

impl fmt::Display for HapticCapabilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "fixed-intensity haptic output must advertise the full normalized intensity limit",
        )
    }
}

impl Error for HapticCapabilityError {}

/// One semantic haptic intention with optional opaque recent-gesture evidence.
pub struct HapticRequest {
    effect: HapticEffect,
    intensity: HapticIntensity,
    user_gesture: Option<UserGestureGrantHandle>,
}

impl HapticRequest {
    pub const fn new(effect: HapticEffect, intensity: HapticIntensity) -> Self {
        Self {
            effect,
            intensity,
            user_gesture: None,
        }
    }

    pub fn with_user_gesture(
        effect: HapticEffect,
        intensity: HapticIntensity,
        user_gesture: UserGestureGrantHandle,
    ) -> Self {
        Self {
            effect,
            intensity,
            user_gesture: Some(user_gesture),
        }
    }

    pub const fn effect(&self) -> HapticEffect {
        self.effect
    }

    pub const fn intensity(&self) -> HapticIntensity {
        self.intensity
    }

    pub const fn has_user_gesture(&self) -> bool {
        self.user_gesture.is_some()
    }

    pub fn into_parts(
        self,
    ) -> (
        HapticEffect,
        HapticIntensity,
        Option<UserGestureGrantHandle>,
    ) {
        (self.effect, self.intensity, self.user_gesture)
    }
}

impl fmt::Debug for HapticRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HapticRequest")
            .field("effect", &self.effect)
            .field("intensity", &self.intensity)
            .field("has_user_gesture", &self.user_gesture.is_some())
            .finish_non_exhaustive()
    }
}

/// Exact semantic intention reported when a haptic request applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HapticApplied {
    effect: HapticEffect,
    intensity: HapticIntensity,
}

impl HapticApplied {
    pub const fn new(effect: HapticEffect, intensity: HapticIntensity) -> Self {
        Self { effect, intensity }
    }

    pub const fn from_request(request: &HapticRequest) -> Self {
        Self::new(request.effect(), request.intensity())
    }

    pub const fn effect(self) -> HapticEffect {
        self.effect
    }

    pub const fn intensity(self) -> HapticIntensity {
        self.intensity
    }
}

/// Immediate rejection before a semantic haptic intention is admitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HapticAdmissionError {
    UnsupportedOperation,
    DeviceUnavailable,
    EffectUnsupported {
        effect: HapticEffect,
    },
    UserSettingDisabled,
    UserSettingUnknown,
    PermissionDenied,
    AuthorizationRequired,
    UserGestureRequired,
    InvalidUserGesture,
    IntensityControlUnsupported,
    IntensityExceedsCapability {
        requested: HapticIntensity,
        maximum: HapticIntensity,
    },
    CapabilityChanged,
    CapacityExceeded,
}

impl fmt::Display for HapticAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnsupportedOperation => "haptic trigger operation is unsupported",
            Self::DeviceUnavailable => "haptic output device is unavailable",
            Self::EffectUnsupported { .. } => "semantic haptic effect is unsupported",
            Self::UserSettingDisabled => "haptic feedback is disabled by the user setting",
            Self::UserSettingUnknown => "haptic user-setting state is unknown",
            Self::PermissionDenied => "haptic feedback permission is denied",
            Self::AuthorizationRequired => "haptic feedback authorization is required",
            Self::UserGestureRequired => "haptic feedback requires a user gesture",
            Self::InvalidUserGesture => "haptic user-gesture evidence is invalid",
            Self::IntensityControlUnsupported => {
                "haptic output does not support portable intensity control"
            }
            Self::IntensityExceedsCapability { .. } => {
                "haptic intensity exceeds the current device capability"
            }
            Self::CapabilityChanged => "haptic capability changed before admission",
            Self::CapacityExceeded => "haptic request admission capacity was exceeded",
        })
    }
}

impl Error for HapticAdmissionError {}

pub type HapticAdmission = RequestAdmission<HapticApplied, HapticAdmissionError>;

/// Object-safe semantic haptic capability and request-admission boundary.
pub trait HapticsService {
    fn capability(&self) -> Support<HapticCapability>;

    fn trigger(&self, request: HapticRequest) -> HapticAdmission;
}

pub enum HapticsServiceKey {}

impl ServiceKey for HapticsServiceKey {
    type Handle = Rc<dyn HapticsService>;
}

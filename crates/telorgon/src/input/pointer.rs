use std::error::Error;
use std::fmt;
use std::num::NonZeroU32;
use std::sync::Arc;

use crate::foundation::{PointF, SizeF};

use crate::input::{ButtonState, Modifiers};

/// Hard bound on simultaneously pressed buttons retained in one pointer snapshot.
pub const MAX_PRESSED_POINTER_BUTTONS: usize = 32;

/// Stable identity for one active pointer/contact in a host input stream.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PointerId(u64);

impl PointerId {
    pub const PRIMARY: Self = Self(0);

    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Neutral pointer tool class. Native device types are translated at the platform boundary.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum PointerDeviceKind {
    Mouse,
    Touch,
    Pen,
    Eraser,
    #[default]
    Unknown,
}

/// Stable neutral pointer-button code.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PointerButton(u32);

impl PointerButton {
    pub const UNIDENTIFIED: Self = Self(0);
    pub const PRIMARY: Self = Self(1);
    pub const SECONDARY: Self = Self(2);
    pub const MIDDLE: Self = Self(3);
    pub const BACK: Self = Self(4);
    pub const FORWARD: Self = Self(5);

    pub const fn new(value: u16) -> Self {
        Self(value as u32)
    }

    /// Namespaces one platform-defined nonstandard button away from standardized button codes.
    pub const fn from_platform_other(value: u16) -> Self {
        Self(0x1_0000 + value as u32)
    }

    pub const fn get(self) -> u32 {
        self.0
    }

    /// Returns the source code of a namespaced platform-defined button.
    pub const fn platform_other_code(self) -> Option<u16> {
        if self.0 >= 0x1_0000 && self.0 <= 0x1_FFFF {
            Some((self.0 - 0x1_0000) as u16)
        } else {
            None
        }
    }

    pub const fn is_identified(self) -> bool {
        self.0 != 0
    }
}

/// Generation-aware identity for a pointer-producing device when the platform supplies one.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PointerDeviceId {
    slot: NonZeroU32,
    generation: NonZeroU32,
}

impl PointerDeviceId {
    pub const fn new(slot: NonZeroU32, generation: NonZeroU32) -> Self {
        Self { slot, generation }
    }

    pub const fn from_raw(slot: u32, generation: u32) -> Option<Self> {
        match (NonZeroU32::new(slot), NonZeroU32::new(generation)) {
            (Some(slot), Some(generation)) => Some(Self { slot, generation }),
            _ => None,
        }
    }

    pub const fn slot(self) -> u32 {
        self.slot.get()
    }

    pub const fn generation(self) -> u32 {
        self.generation.get()
    }
}

/// Original physical-pixel position retained beside canonical view-logical coordinates.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PhysicalPointerPosition {
    x: f64,
    y: f64,
}

impl PhysicalPointerPosition {
    pub fn new(x: f64, y: f64) -> Result<Self, PointerCoordinateError> {
        if !x.is_finite() || !y.is_finite() {
            return Err(PointerCoordinateError::NonFinitePhysicalPosition);
        }
        Ok(Self { x, y })
    }

    pub const fn x(self) -> f64 {
        self.x
    }

    pub const fn y(self) -> f64 {
        self.y
    }
}

/// Canonical view-logical position plus the original physical observation when available.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PointerPosition {
    view_logical: PointF,
    physical: Option<PhysicalPointerPosition>,
}

impl PointerPosition {
    pub fn logical(view_logical: PointF) -> Result<Self, PointerCoordinateError> {
        validate_logical_point(view_logical)?;
        Ok(Self {
            view_logical,
            physical: None,
        })
    }

    pub fn with_physical(
        view_logical: PointF,
        physical: PhysicalPointerPosition,
    ) -> Result<Self, PointerCoordinateError> {
        validate_logical_point(view_logical)?;
        Ok(Self {
            view_logical,
            physical: Some(physical),
        })
    }

    pub const fn view_logical(self) -> PointF {
        self.view_logical
    }

    pub const fn physical(self) -> Option<PhysicalPointerPosition> {
        self.physical
    }
}

/// Invalid pointer coordinate observation.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PointerCoordinateError {
    NonFiniteLogicalPosition,
    NonFinitePhysicalPosition,
}

impl fmt::Display for PointerCoordinateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteLogicalPosition => {
                formatter.write_str("pointer view-logical position must be finite")
            }
            Self::NonFinitePhysicalPosition => {
                formatter.write_str("pointer physical position must be finite")
            }
        }
    }
}

impl Error for PointerCoordinateError {}

/// Canonically ordered complete pressed-button snapshot.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct PointerButtonSet(Arc<[PointerButton]>);

impl PointerButtonSet {
    pub fn new(
        buttons: impl IntoIterator<Item = PointerButton>,
    ) -> Result<Self, PointerButtonSetError> {
        let mut ordered = Vec::new();
        for button in buttons {
            if !button.is_identified() {
                return Err(PointerButtonSetError::UnidentifiedButton);
            }
            match ordered.binary_search(&button) {
                Ok(_) => return Err(PointerButtonSetError::DuplicateButton(button)),
                Err(index) => {
                    if ordered.len() == MAX_PRESSED_POINTER_BUTTONS {
                        return Err(PointerButtonSetError::TooManyButtons {
                            maximum: MAX_PRESSED_POINTER_BUTTONS,
                        });
                    }
                    ordered.insert(index, button);
                }
            }
        }
        Ok(Self(Arc::from(ordered)))
    }

    pub fn contains(&self, button: PointerButton) -> bool {
        self.0.binary_search(&button).is_ok()
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = PointerButton> + '_ {
        self.0.iter().copied()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Invalid complete pressed-button snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerButtonSetError {
    UnidentifiedButton,
    DuplicateButton(PointerButton),
    TooManyButtons { maximum: usize },
}

impl fmt::Display for PointerButtonSetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnidentifiedButton => {
                formatter.write_str("pressed-button state cannot contain an unidentified button")
            }
            Self::DuplicateButton(button) => {
                write!(formatter, "pressed-button state repeats button {button:?}")
            }
            Self::TooManyButtons { maximum } => {
                write!(
                    formatter,
                    "pressed-button state exceeds the {maximum}-button bound"
                )
            }
        }
    }
}

impl Error for PointerButtonSetError {}

/// Normalized pointer pressure in the inclusive range zero through one.
#[derive(Copy, Clone, Debug, PartialEq, PartialOrd)]
pub struct PointerPressure(f32);

impl PointerPressure {
    pub fn new(value: f32) -> Result<Self, PointerPropertyError> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(PointerPropertyError::InvalidPressure { observed: value });
        }
        Ok(Self(value))
    }

    pub const fn get(self) -> f32 {
        self.0
    }
}

/// Pen tilt in degrees along the view-logical x/y axes.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PointerTilt {
    x_degrees: f32,
    y_degrees: f32,
}

impl PointerTilt {
    pub fn new(x_degrees: f32, y_degrees: f32) -> Result<Self, PointerPropertyError> {
        if !x_degrees.is_finite()
            || !y_degrees.is_finite()
            || !(-90.0..=90.0).contains(&x_degrees)
            || !(-90.0..=90.0).contains(&y_degrees)
        {
            return Err(PointerPropertyError::InvalidTilt {
                x_degrees,
                y_degrees,
            });
        }
        Ok(Self {
            x_degrees,
            y_degrees,
        })
    }

    pub const fn x_degrees(self) -> f32 {
        self.x_degrees
    }

    pub const fn y_degrees(self) -> f32 {
        self.y_degrees
    }
}

/// Clockwise pointer-tool rotation in degrees in the half-open range `[0, 360)`.
#[derive(Copy, Clone, Debug, PartialEq, PartialOrd)]
pub struct PointerTwist(f32);

impl PointerTwist {
    pub fn new(degrees: f32) -> Result<Self, PointerPropertyError> {
        if !degrees.is_finite() || !(0.0..360.0).contains(&degrees) {
            return Err(PointerPropertyError::InvalidTwist { observed: degrees });
        }
        Ok(Self(degrees))
    }

    pub const fn degrees(self) -> f32 {
        self.0
    }
}

/// Positive contact geometry measured in canonical view-logical units.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PointerContactGeometry(SizeF);

impl PointerContactGeometry {
    pub fn new(size: SizeF) -> Result<Self, PointerPropertyError> {
        if !size.width.is_finite()
            || !size.height.is_finite()
            || size.width <= 0.0
            || size.height <= 0.0
        {
            return Err(PointerPropertyError::InvalidContactGeometry {
                width: size.width,
                height: size.height,
            });
        }
        Ok(Self(size))
    }

    pub const fn size(self) -> SizeF {
        self.0
    }
}

/// Optional tool facts retained exactly when supplied by the platform.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct PointerProperties {
    pub pressure: Option<PointerPressure>,
    pub tilt: Option<PointerTilt>,
    pub twist: Option<PointerTwist>,
    pub contact_geometry: Option<PointerContactGeometry>,
}

/// Invalid pressure, tilt, twist, or contact observation.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum PointerPropertyError {
    InvalidPressure { observed: f32 },
    InvalidTilt { x_degrees: f32, y_degrees: f32 },
    InvalidTwist { observed: f32 },
    InvalidContactGeometry { width: f32, height: f32 },
}

impl fmt::Display for PointerPropertyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPressure { observed } => write!(
                formatter,
                "pointer pressure {observed} is outside the finite inclusive range zero through one"
            ),
            Self::InvalidTilt {
                x_degrees,
                y_degrees,
            } => write!(
                formatter,
                "pointer tilt ({x_degrees}, {y_degrees}) is outside the finite ±90-degree range"
            ),
            Self::InvalidTwist { observed } => write!(
                formatter,
                "pointer twist {observed} is outside the finite half-open range zero through 360 degrees"
            ),
            Self::InvalidContactGeometry { width, height } => write!(
                formatter,
                "pointer contact geometry ({width}, {height}) must be finite and positive"
            ),
        }
    }
}

impl Error for PointerPropertyError {}

/// Whether this pointer observation originated directly or through platform synthesis.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum PointerEventSource {
    #[default]
    Native,
    SynthesizedFromTouch,
    SynthesizedOther,
}

impl PointerEventSource {
    pub const fn is_synthesized(self) -> bool {
        !matches!(self, Self::Native)
    }
}

/// Why an active pointer/contact stream was cancelled.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum PointerCancelReason {
    Platform,
    FocusLost,
    ViewSuspended,
    ForcedDestruction,
    CaptureLost,
    DeviceRemoved,
}

/// Explicit pointer-capture lifecycle observation.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum PointerCaptureChange {
    Acquired,
    Released,
    Lost,
}

/// Lifecycle/change kind for one pointer observation.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum PointerEventKind {
    Entered,
    Left,
    Hovered,
    Moved,
    Button {
        button: PointerButton,
        state: ButtonState,
    },
    Cancelled(PointerCancelReason),
    CaptureChanged(PointerCaptureChange),
}

/// Complete caller-supplied state snapshot accompanying one pointer change.
#[derive(Clone, Debug, PartialEq)]
pub struct PointerStateSnapshot {
    pointer: PointerId,
    device: PointerDeviceKind,
    device_id: Option<PointerDeviceId>,
    position: Option<PointerPosition>,
    buttons: PointerButtonSet,
    properties: PointerProperties,
    primary_contact: bool,
    source: PointerEventSource,
    modifiers: Modifiers,
}

impl PointerStateSnapshot {
    pub fn new(pointer: PointerId, device: PointerDeviceKind) -> Self {
        Self {
            pointer,
            device,
            device_id: None,
            position: None,
            buttons: PointerButtonSet::default(),
            properties: PointerProperties::default(),
            primary_contact: false,
            source: PointerEventSource::Native,
            modifiers: Modifiers::empty(),
        }
    }

    pub fn with_device_id(mut self, device_id: Option<PointerDeviceId>) -> Self {
        self.device_id = device_id;
        self
    }

    pub fn with_position(mut self, position: Option<PointerPosition>) -> Self {
        self.position = position;
        self
    }

    pub fn with_buttons(mut self, buttons: PointerButtonSet) -> Self {
        self.buttons = buttons;
        self
    }

    pub fn with_properties(mut self, properties: PointerProperties) -> Self {
        self.properties = properties;
        self
    }

    pub fn with_primary_contact(mut self, primary_contact: bool) -> Self {
        self.primary_contact = primary_contact;
        self
    }

    pub fn with_source(mut self, source: PointerEventSource) -> Self {
        self.source = source;
        self
    }

    pub fn with_modifiers(mut self, modifiers: Modifiers) -> Self {
        self.modifiers = modifiers;
        self
    }

    pub const fn pointer(&self) -> PointerId {
        self.pointer
    }

    pub const fn device(&self) -> PointerDeviceKind {
        self.device
    }

    pub const fn device_id(&self) -> Option<PointerDeviceId> {
        self.device_id
    }

    pub const fn position(&self) -> Option<PointerPosition> {
        self.position
    }

    pub const fn buttons(&self) -> &PointerButtonSet {
        &self.buttons
    }

    pub const fn properties(&self) -> PointerProperties {
        self.properties
    }

    pub const fn primary_contact(&self) -> bool {
        self.primary_contact
    }

    pub const fn source(&self) -> PointerEventSource {
        self.source
    }

    pub const fn modifiers(&self) -> Modifiers {
        self.modifiers
    }
}

/// Validated complete neutral pointer observation.
#[derive(Clone, Debug, PartialEq)]
pub struct PointerEvent {
    kind: PointerEventKind,
    state: PointerStateSnapshot,
}

impl PointerEvent {
    pub fn new(
        kind: PointerEventKind,
        state: PointerStateSnapshot,
    ) -> Result<Self, PointerEventError> {
        match kind {
            PointerEventKind::Button {
                button,
                state: edge,
            } => {
                if !button.is_identified() {
                    return Err(PointerEventError::UnidentifiedChangedButton);
                }
                let retained = state.buttons.contains(button);
                if edge == ButtonState::Pressed && !retained {
                    return Err(PointerEventError::PressedButtonMissing { button });
                }
                if edge == ButtonState::Released && retained {
                    return Err(PointerEventError::ReleasedButtonStillPressed { button });
                }
            }
            PointerEventKind::Cancelled(_) if !state.buttons.is_empty() => {
                return Err(PointerEventError::CancellationRetainsButtons);
            }
            PointerEventKind::Entered
            | PointerEventKind::Left
            | PointerEventKind::Hovered
            | PointerEventKind::Moved
            | PointerEventKind::Cancelled(_)
            | PointerEventKind::CaptureChanged(_) => {}
        }
        Ok(Self { kind, state })
    }

    pub const fn kind(&self) -> PointerEventKind {
        self.kind
    }

    pub const fn state(&self) -> &PointerStateSnapshot {
        &self.state
    }

    pub fn into_state(self) -> PointerStateSnapshot {
        self.state
    }
}

/// Invalid relationship between a pointer change and its complete state snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerEventError {
    UnidentifiedChangedButton,
    PressedButtonMissing { button: PointerButton },
    ReleasedButtonStillPressed { button: PointerButton },
    CancellationRetainsButtons,
}

impl fmt::Display for PointerEventError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnidentifiedChangedButton => {
                formatter.write_str("pointer button change requires an identified button")
            }
            Self::PressedButtonMissing { button } => write!(
                formatter,
                "pressed pointer button {button:?} is absent from complete button state"
            ),
            Self::ReleasedButtonStillPressed { button } => write!(
                formatter,
                "released pointer button {button:?} remains in complete button state"
            ),
            Self::CancellationRetainsButtons => {
                formatter.write_str("cancelled pointer state must release every button")
            }
        }
    }
}

impl Error for PointerEventError {}

/// Unit supplied by the platform for one scroll delta.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ScrollUnit {
    Pixels,
    Lines,
    Pages,
}

/// Gesture phase of a scrolling sequence.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum ScrollPhase {
    #[default]
    Discrete,
    Began,
    Changed,
    Ended,
    Cancelled,
}

/// Independently observed momentum phase when supplied by a platform.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum ScrollMomentumPhase {
    #[default]
    None,
    Began,
    Changed,
    Ended,
}

/// Whether the platform identifies the scroll observation as precise or discrete.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum ScrollPrecision {
    #[default]
    Unknown,
    Discrete,
    Precise,
}

/// Original physical-pixel delta retained beside canonical logical pixel deltas.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PhysicalScrollDelta {
    x: f64,
    y: f64,
}

impl PhysicalScrollDelta {
    pub fn new(x: f64, y: f64) -> Result<Self, ScrollValueError> {
        if !x.is_finite() || !y.is_finite() {
            return Err(ScrollValueError::NonFinitePhysicalDelta);
        }
        Ok(Self { x, y })
    }

    pub const fn x(self) -> f64 {
        self.x
    }

    pub const fn y(self) -> f64 {
        self.y
    }
}

/// Two-axis scroll delta in its original semantic unit.
///
/// Pixel values are canonical view-logical pixels. Their original physical delta may additionally
/// be retained; line and page values are never converted to pixels in this value package.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ScrollDelta {
    x: f64,
    y: f64,
    unit: ScrollUnit,
    physical_pixels: Option<PhysicalScrollDelta>,
}

impl ScrollDelta {
    pub fn new(x: f64, y: f64, unit: ScrollUnit) -> Result<Self, ScrollValueError> {
        if !x.is_finite() || !y.is_finite() {
            return Err(ScrollValueError::NonFiniteDelta);
        }
        Ok(Self {
            x,
            y,
            unit,
            physical_pixels: None,
        })
    }

    pub fn with_physical_pixels(
        mut self,
        physical_pixels: PhysicalScrollDelta,
    ) -> Result<Self, ScrollValueError> {
        if self.unit != ScrollUnit::Pixels {
            return Err(ScrollValueError::PhysicalDeltaForNonPixelUnit { unit: self.unit });
        }
        self.physical_pixels = Some(physical_pixels);
        Ok(self)
    }

    pub const fn x(self) -> f64 {
        self.x
    }

    pub const fn y(self) -> f64 {
        self.y
    }

    pub const fn unit(self) -> ScrollUnit {
        self.unit
    }

    pub const fn physical_pixels(self) -> Option<PhysicalScrollDelta> {
        self.physical_pixels
    }
}

/// Invalid scroll delta or coordinate-space relationship.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollValueError {
    NonFiniteDelta,
    NonFinitePhysicalDelta,
    PhysicalDeltaForNonPixelUnit { unit: ScrollUnit },
}

impl fmt::Display for ScrollValueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteDelta => formatter.write_str("scroll delta must be finite"),
            Self::NonFinitePhysicalDelta => {
                formatter.write_str("physical scroll delta must be finite")
            }
            Self::PhysicalDeltaForNonPixelUnit { unit } => write!(
                formatter,
                "physical scroll delta cannot accompany semantic {unit:?} units"
            ),
        }
    }
}

impl Error for ScrollValueError {}

/// Complete neutral scroll observation.
#[derive(Clone, Debug, PartialEq)]
pub struct ScrollEvent {
    pointer: PointerId,
    device: PointerDeviceKind,
    device_id: Option<PointerDeviceId>,
    position: Option<PointerPosition>,
    delta: ScrollDelta,
    phase: ScrollPhase,
    momentum: ScrollMomentumPhase,
    precision: ScrollPrecision,
    source: PointerEventSource,
    modifiers: Modifiers,
}

impl ScrollEvent {
    pub fn new(pointer: PointerId, device: PointerDeviceKind, delta: ScrollDelta) -> Self {
        Self {
            pointer,
            device,
            device_id: None,
            position: None,
            delta,
            phase: ScrollPhase::Discrete,
            momentum: ScrollMomentumPhase::None,
            precision: ScrollPrecision::Unknown,
            source: PointerEventSource::Native,
            modifiers: Modifiers::empty(),
        }
    }

    pub fn with_device_id(mut self, device_id: Option<PointerDeviceId>) -> Self {
        self.device_id = device_id;
        self
    }

    pub fn with_position(mut self, position: Option<PointerPosition>) -> Self {
        self.position = position;
        self
    }

    pub fn with_phase(mut self, phase: ScrollPhase) -> Self {
        self.phase = phase;
        self
    }

    pub fn with_momentum(mut self, momentum: ScrollMomentumPhase) -> Self {
        self.momentum = momentum;
        self
    }

    pub fn with_precision(mut self, precision: ScrollPrecision) -> Self {
        self.precision = precision;
        self
    }

    pub fn with_source(mut self, source: PointerEventSource) -> Self {
        self.source = source;
        self
    }

    pub fn with_modifiers(mut self, modifiers: Modifiers) -> Self {
        self.modifiers = modifiers;
        self
    }

    pub const fn pointer(&self) -> PointerId {
        self.pointer
    }

    pub const fn device(&self) -> PointerDeviceKind {
        self.device
    }

    pub const fn device_id(&self) -> Option<PointerDeviceId> {
        self.device_id
    }

    pub const fn position(&self) -> Option<PointerPosition> {
        self.position
    }

    pub const fn delta(&self) -> ScrollDelta {
        self.delta
    }

    pub const fn phase(&self) -> ScrollPhase {
        self.phase
    }

    pub const fn momentum(&self) -> ScrollMomentumPhase {
        self.momentum
    }

    pub const fn precision(&self) -> ScrollPrecision {
        self.precision
    }

    pub const fn source(&self) -> PointerEventSource {
        self.source
    }

    pub const fn modifiers(&self) -> Modifiers {
        self.modifiers
    }
}

fn validate_logical_point(point: PointF) -> Result<(), PointerCoordinateError> {
    if point.x.is_finite() && point.y.is_finite() {
        Ok(())
    } else {
        Err(PointerCoordinateError::NonFiniteLogicalPosition)
    }
}

#[cfg(test)]
mod tests;

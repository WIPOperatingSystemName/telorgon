//! Neutral client-input forwarding values for shell surfaces.

use std::fmt;
use std::num::NonZeroU64;

use crate::foundation::PointF;
use crate::input::{ButtonState, PointerButton};

macro_rules! define_input_id {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[repr(transparent)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(NonZeroU64);

        impl $name {
            pub const fn new(value: NonZeroU64) -> Self {
                Self(value)
            }

            pub const fn from_raw(value: u64) -> Option<Self> {
                match NonZeroU64::new(value) {
                    Some(value) => Some(Self(value)),
                    None => None,
                }
            }

            pub const fn get(self) -> u64 {
                self.0.get()
            }
        }
    };
}

define_input_id!(SeatId, "Stable identity of one host input seat.");
define_input_id!(
    ContactId,
    "Stable identity of one active host input contact."
);

/// Portable origin classification retained beside shell requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InputSource {
    Mouse,
    Touch,
    Pen,
    Eraser,
    Keyboard,
    Accessibility,
    Programmatic,
}

impl InputSource {
    pub const fn is_contact(self) -> bool {
        matches!(self, Self::Mouse | Self::Touch | Self::Pen | Self::Eraser)
    }
}

/// Host seat/contact/source identity for one active pointer-like stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SurfaceInputContact {
    seat: SeatId,
    contact: ContactId,
    source: InputSource,
}

impl SurfaceInputContact {
    pub fn new(
        seat: SeatId,
        contact: ContactId,
        source: InputSource,
    ) -> Result<Self, SurfaceInputError> {
        if !source.is_contact() {
            return Err(SurfaceInputError::NonContactSource { source });
        }
        Ok(Self {
            seat,
            contact,
            source,
        })
    }

    pub const fn seat(self) -> SeatId {
        self.seat
    }

    pub const fn contact(self) -> ContactId {
        self.contact
    }

    pub const fn source(self) -> InputSource {
        self.source
    }
}

/// Lifecycle kind of a neutral event forwarded to one client surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SurfaceInputKind {
    Entered,
    Moved,
    Button,
    Scrolled,
    Left,
    Cancelled,
}

/// Validated neutral pointer/touch/pen event in surface-local logical coordinates.
///
/// Native protocol serials, grabs, handles, and dispatch remain owned by the host adapter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceInputEvent {
    kind: SurfaceInputKind,
    contact: SurfaceInputContact,
    position: Option<PointF>,
    button: Option<(PointerButton, ButtonState)>,
    scroll_delta: Option<PointF>,
}

impl SurfaceInputEvent {
    pub fn entered(
        contact: SurfaceInputContact,
        position: PointF,
    ) -> Result<Self, SurfaceInputError> {
        Self::at_position(SurfaceInputKind::Entered, contact, position)
    }

    pub fn moved(
        contact: SurfaceInputContact,
        position: PointF,
    ) -> Result<Self, SurfaceInputError> {
        Self::at_position(SurfaceInputKind::Moved, contact, position)
    }

    pub fn button(
        contact: SurfaceInputContact,
        position: PointF,
        button: PointerButton,
        state: ButtonState,
    ) -> Result<Self, SurfaceInputError> {
        validate_point(position)
            .then_some(())
            .ok_or(SurfaceInputError::NonFinitePosition {
                kind: SurfaceInputKind::Button,
            })?;
        if button.get() == 0 {
            return Err(SurfaceInputError::InvalidButton);
        }
        Ok(Self {
            kind: SurfaceInputKind::Button,
            contact,
            position: Some(position),
            button: Some((button, state)),
            scroll_delta: None,
        })
    }

    pub fn scrolled(
        contact: SurfaceInputContact,
        position: PointF,
        delta: PointF,
    ) -> Result<Self, SurfaceInputError> {
        if !validate_point(position) {
            return Err(SurfaceInputError::NonFinitePosition {
                kind: SurfaceInputKind::Scrolled,
            });
        }
        if !validate_point(delta) {
            return Err(SurfaceInputError::NonFiniteScrollDelta);
        }
        Ok(Self {
            kind: SurfaceInputKind::Scrolled,
            contact,
            position: Some(position),
            button: None,
            scroll_delta: Some(delta),
        })
    }

    pub const fn left(contact: SurfaceInputContact) -> Self {
        Self::without_position(SurfaceInputKind::Left, contact)
    }

    pub const fn cancelled(contact: SurfaceInputContact) -> Self {
        Self::without_position(SurfaceInputKind::Cancelled, contact)
    }

    pub const fn kind(self) -> SurfaceInputKind {
        self.kind
    }

    pub const fn contact(self) -> SurfaceInputContact {
        self.contact
    }

    pub const fn position(self) -> Option<PointF> {
        self.position
    }

    pub const fn button_change(self) -> Option<(PointerButton, ButtonState)> {
        self.button
    }

    pub const fn scroll_delta(self) -> Option<PointF> {
        self.scroll_delta
    }

    fn at_position(
        kind: SurfaceInputKind,
        contact: SurfaceInputContact,
        position: PointF,
    ) -> Result<Self, SurfaceInputError> {
        if !validate_point(position) {
            return Err(SurfaceInputError::NonFinitePosition { kind });
        }
        Ok(Self {
            kind,
            contact,
            position: Some(position),
            button: None,
            scroll_delta: None,
        })
    }

    const fn without_position(kind: SurfaceInputKind, contact: SurfaceInputContact) -> Self {
        Self {
            kind,
            contact,
            position: None,
            button: None,
            scroll_delta: None,
        }
    }
}

/// One validated neutral input event addressed to a host-owned client surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClientInputRequest {
    surface: crate::shell::SurfaceId,
    event: SurfaceInputEvent,
}

impl ClientInputRequest {
    pub const fn new(surface: crate::shell::SurfaceId, event: SurfaceInputEvent) -> Self {
        Self { surface, event }
    }

    pub const fn surface(self) -> crate::shell::SurfaceId {
        self.surface
    }

    pub const fn event(self) -> SurfaceInputEvent {
        self.event
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceInputError {
    NonContactSource { source: InputSource },
    NonFinitePosition { kind: SurfaceInputKind },
    InvalidButton,
    NonFiniteScrollDelta,
}

impl fmt::Display for SurfaceInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonContactSource { source } => {
                write!(formatter, "{source:?} cannot identify a contact stream")
            }
            Self::NonFinitePosition { kind } => {
                write!(formatter, "{kind:?} surface input position must be finite")
            }
            Self::InvalidButton => formatter.write_str("surface input button must be nonzero"),
            Self::NonFiniteScrollDelta => {
                formatter.write_str("surface input scroll delta must be finite")
            }
        }
    }
}

impl std::error::Error for SurfaceInputError {}

const fn validate_point(point: PointF) -> bool {
    point.x.is_finite() && point.y.is_finite()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::SurfaceId;

    fn contact(source: InputSource) -> Result<SurfaceInputContact, SurfaceInputError> {
        SurfaceInputContact::new(
            SeatId::from_raw(1).unwrap(),
            ContactId::from_raw(2).unwrap(),
            source,
        )
    }

    #[test]
    fn contact_events_preserve_neutral_lifecycle_and_local_coordinates() {
        let contact = contact(InputSource::Touch).unwrap();
        let event = SurfaceInputEvent::button(
            contact,
            PointF { x: 12.0, y: 8.0 },
            PointerButton::PRIMARY,
            ButtonState::Pressed,
        )
        .unwrap();
        let request = ClientInputRequest::new(SurfaceId::from_raw(3).unwrap(), event);

        assert_eq!(request.surface().get(), 3);
        assert_eq!(request.event().kind(), SurfaceInputKind::Button);
        assert_eq!(request.event().contact().seat().get(), 1);
        assert_eq!(request.event().contact().contact().get(), 2);
        assert_eq!(request.event().position().unwrap().x, 12.0);
        assert_eq!(
            request.event().button_change(),
            Some((PointerButton::PRIMARY, ButtonState::Pressed))
        );
    }

    #[test]
    fn invalid_sources_and_coordinates_are_rejected_before_forwarding() {
        assert_eq!(
            contact(InputSource::Keyboard),
            Err(SurfaceInputError::NonContactSource {
                source: InputSource::Keyboard,
            })
        );
        let contact = contact(InputSource::Mouse).unwrap();
        assert_eq!(
            SurfaceInputEvent::moved(
                contact,
                PointF {
                    x: f32::NAN,
                    y: 0.0,
                },
            ),
            Err(SurfaceInputError::NonFinitePosition {
                kind: SurfaceInputKind::Moved,
            })
        );
        assert_eq!(
            SurfaceInputEvent::scrolled(
                contact,
                PointF { x: 0.0, y: 0.0 },
                PointF {
                    x: 0.0,
                    y: f32::INFINITY,
                },
            ),
            Err(SurfaceInputError::NonFiniteScrollDelta)
        );
    }

    #[test]
    fn leave_and_cancellation_retain_contact_without_fabricating_coordinates() {
        let contact = contact(InputSource::Pen).unwrap();
        for event in [
            SurfaceInputEvent::left(contact),
            SurfaceInputEvent::cancelled(contact),
        ] {
            assert_eq!(event.contact(), contact);
            assert_eq!(event.position(), None);
            assert_eq!(event.button_change(), None);
            assert_eq!(event.scroll_delta(), None);
        }
    }
}

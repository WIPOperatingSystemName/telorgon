//! Pure ext-image-copy-capture frame validation. Native resource ownership and host
//! authorization remain in their adapters; this state never owns a client buffer.
#![allow(dead_code)] // Native capture dispatch is being connected separately.

use std::{cell::Cell, rc::Rc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub(super) enum SessionError {
    DuplicateFrame = 1,
}

/// Tracks protocol resource lifetime, not GPU work. A completed frame continues
/// occupying this slot until its resource is destroyed. Frames outlive the session.
#[derive(Default)]
pub(super) struct CaptureSession {
    occupied: Rc<Cell<bool>>,
}

impl CaptureSession {
    pub fn create_frame<B: Copy>(&self) -> Result<CaptureFrame<B>, SessionError> {
        if self.occupied.replace(true) {
            return Err(SessionError::DuplicateFrame);
        }
        let mut frame = CaptureFrame::new();
        frame.session_slot = Some(self.occupied.clone());
        Ok(frame)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub(super) enum FrameError {
    NoBuffer = 1,
    InvalidBufferDamage = 2,
    AlreadyCaptured = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Configuring,
    Submitted,
    Terminal,
    Destroyed,
}

pub(super) struct CaptureFrame<B> {
    buffer: Option<B>,
    phase: Phase,
    session_slot: Option<Rc<Cell<bool>>>,
}

impl<B: Copy> CaptureFrame<B> {
    fn new() -> Self {
        Self {
            buffer: None,
            phase: Phase::Configuring,
            session_slot: None,
        }
    }

    fn configuring(&self) -> Result<(), FrameError> {
        if self.phase == Phase::Configuring {
            Ok(())
        } else {
            Err(FrameError::AlreadyCaptured)
        }
    }

    pub fn attach(&mut self, buffer: B) -> Result<(), FrameError> {
        self.configuring()?;
        self.buffer = Some(buffer);
        Ok(())
    }

    pub fn damage(&self, x: i32, y: i32, width: i32, height: i32) -> Result<(), FrameError> {
        self.configuring()?;
        if x < 0 || y < 0 || width <= 0 || height <= 0 {
            return Err(FrameError::InvalidBufferDamage);
        }
        // Initial implementation copies the full frame. Accept arbitrarily many valid
        // rectangles without retaining a client-controlled list or adding signed coordinates.
        Ok(())
    }

    pub fn capture(&mut self) -> Result<B, FrameError> {
        self.configuring()?;
        let buffer = self.buffer.ok_or(FrameError::NoBuffer)?;
        self.phase = Phase::Submitted;
        Ok(buffer)
    }

    /// Claim the single ready/failed event. Destroyed resources suppress late delivery.
    /// GPU retirement must happen independently, even when this returns false.
    pub fn finish(&mut self) -> bool {
        if self.phase != Phase::Submitted {
            return false;
        }
        self.phase = Phase::Terminal;
        self.buffer = None;
        true
    }

    pub fn destroy(&mut self) {
        self.phase = Phase::Destroyed;
        self.buffer = None;
        if let Some(slot) = self.session_slot.take() {
            slot.set(false);
        }
    }
}

impl<B> Drop for CaptureFrame<B> {
    fn drop(&mut self) {
        if let Some(slot) = self.session_slot.take() {
            slot.set(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_frame_occupies_session_until_resource_destruction() {
        let session = CaptureSession::default();
        let mut first = session.create_frame().unwrap();
        first.attach(1).unwrap();
        first.capture().unwrap();
        first.finish();
        assert!(matches!(
            session.create_frame::<u64>(),
            Err(SessionError::DuplicateFrame)
        ));
        first.destroy();
        let second = session.create_frame::<u64>().unwrap();
        // An old resource's eventual Rust drop cannot vacate the replacement slot.
        drop(first);
        assert!(matches!(
            session.create_frame::<u64>(),
            Err(SessionError::DuplicateFrame)
        ));
        drop(second);
        assert!(session.create_frame::<u64>().is_ok());
    }

    #[test]
    fn session_destruction_does_not_destroy_existing_frame() {
        let session = CaptureSession::default();
        let mut frame = session.create_frame().unwrap();
        frame.attach(7).unwrap();
        drop(session);
        assert_eq!(frame.capture(), Ok(7));
        assert!(frame.finish());
        frame.destroy();
    }

    #[test]
    fn buffer_replacement_and_single_capture_follow_wire_errors() {
        let mut frame = CaptureFrame::new();
        assert_eq!(frame.capture(), Err(FrameError::NoBuffer));
        frame.attach(10).unwrap();
        frame.attach(20).unwrap();
        assert_eq!(frame.capture(), Ok(20));
        assert_eq!(frame.capture(), Err(FrameError::AlreadyCaptured));
        assert_eq!(frame.attach(30), Err(FrameError::AlreadyCaptured));
        assert_eq!(frame.damage(-1, 0, 0, 0), Err(FrameError::AlreadyCaptured));
        assert!(frame.finish());
        assert!(!frame.finish());
        assert_eq!(frame.capture(), Err(FrameError::AlreadyCaptured));
    }

    #[test]
    fn damage_validation_does_not_overflow_or_grow_storage() {
        let frame = CaptureFrame::<u64>::new();
        for rect in [(-1, 0, 1, 1), (0, -1, 1, 1), (0, 0, 0, 1), (0, 0, 1, -1)] {
            assert_eq!(
                frame.damage(rect.0, rect.1, rect.2, rect.3),
                Err(FrameError::InvalidBufferDamage)
            );
        }
        for _ in 0..10000 {
            assert_eq!(frame.damage(i32::MAX, i32::MAX, i32::MAX, i32::MAX), Ok(()));
        }
    }

    #[test]
    fn destruction_suppresses_completion_in_every_phase() {
        for submitted in [false, true] {
            let mut frame = CaptureFrame::new();
            frame.attach(1).unwrap();
            if submitted {
                frame.capture().unwrap();
            }
            frame.destroy();
            frame.destroy();
            assert!(!frame.finish());
            assert_eq!(frame.attach(2), Err(FrameError::AlreadyCaptured));
        }
        assert!(!CaptureFrame::<u64>::new().finish());
    }
}

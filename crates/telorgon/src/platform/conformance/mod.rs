//! Deterministic, native-free conformance support for `telorgon-platform` contracts.
//!
//! The package supplies manually controlled time, bounded ordered capture, a multi-view lifecycle
//! driver, a deterministic event host, and selected fake service adapters. It reads no ambient
//! clock, opens no native object, runs no renderer, and creates no callback, queue, executor,
//! thread, timer, event loop, or fallback service.

pub mod capture;
pub mod fake_clock;
pub mod fake_services;
pub mod host;
pub mod lifecycle;

pub use capture::{
    BoundedCapture, CaptureCapacityError, CaptureLimitError, CompletionCapture, EventCapture,
    MAX_CAPTURE_ITEMS,
};
pub use fake_clock::{FakeClock, FakeClockError};
pub use fake_services::{
    FakeHapticInvocation, FakeHapticsService, FakeRestorationError, FakeRestorationInvocation,
    FakeRestorationOperation, FakeRestorationService, MAX_FAKE_RESTORATION_SCOPES,
};
pub use host::{DeterministicHost, HostEmitError, HostEmitErrorKind, HostLimitError};
pub use lifecycle::{
    MAX_CONFORMANCE_VIEWS, ViewDriver, ViewDriverError, ViewDriverLimitError, ViewObservation,
};

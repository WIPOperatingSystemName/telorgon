//! Linear completion delivery through a caller-owned Winit event-loop proxy.

use std::error::Error;
use std::fmt;

use winit::event_loop::{EventLoopClosed, EventLoopProxy};

/// One owned completion ready to enter the Winit event-loop owner.
///
/// The private payload has only shared borrowing and ownership-consuming access. The envelope is
/// intentionally not `Clone` or `Copy`, even when `T` is, so a linear request completion cannot be
/// accidentally duplicated by the adapter boundary.
///
/// ```compile_fail
/// use crate::platform::winit::CompletionEvent;
///
/// let completion = CompletionEvent::new(String::from("one terminal result"));
/// let duplicate = completion.clone();
/// ```
#[must_use = "a completion event must be delivered or explicitly handled"]
pub struct CompletionEvent<T> {
    completion: T,
}

impl<T> CompletionEvent<T> {
    /// Wraps one owned completion for Winit user-event delivery.
    pub const fn new(completion: T) -> Self {
        Self { completion }
    }

    /// Borrows the completion without exposing mutable access.
    pub const fn completion(&self) -> &T {
        &self.completion
    }

    /// Consumes the envelope and returns the owned completion.
    pub fn into_completion(self) -> T {
        self.completion
    }
}

impl<T> From<T> for CompletionEvent<T> {
    fn from(completion: T) -> Self {
        Self::new(completion)
    }
}

impl<T> fmt::Debug for CompletionEvent<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CompletionEvent")
            .field("completion", &"<owned>")
            .finish()
    }
}

/// Closed set of completion-delivery failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionSendErrorKind {
    /// The caller-owned Winit event loop no longer exists.
    EventLoopClosed,
}

/// Failed Winit completion delivery with the original completion returned intact.
///
/// Generic diagnostics deliberately omit the completion payload. Callers must consume this error
/// and choose their own cancellation, teardown, or reporting policy; this adapter never retries or
/// installs a fallback queue.
#[must_use = "the undelivered completion must be explicitly handled"]
pub struct CompletionSendError<T> {
    kind: CompletionSendErrorKind,
    completion: T,
}

impl<T> CompletionSendError<T> {
    /// Returns the structured failure classification.
    pub const fn kind(&self) -> CompletionSendErrorKind {
        self.kind
    }

    /// Borrows the undelivered completion.
    pub const fn completion(&self) -> &T {
        &self.completion
    }

    /// Consumes the error and returns the undelivered completion.
    pub fn into_completion(self) -> T {
        self.completion
    }
}

impl<T> From<EventLoopClosed<CompletionEvent<T>>> for CompletionSendError<T> {
    fn from(closed: EventLoopClosed<CompletionEvent<T>>) -> Self {
        Self {
            kind: CompletionSendErrorKind::EventLoopClosed,
            completion: closed.0.into_completion(),
        }
    }
}

impl<T> fmt::Debug for CompletionSendError<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CompletionSendError")
            .field("kind", &self.kind)
            .field("completion", &"<returned>")
            .finish()
    }
}

impl<T> fmt::Display for CompletionSendError<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the Winit event loop closed before completion delivery")
    }
}

impl<T> Error for CompletionSendError<T> {}

/// Cloneable cross-thread sender for one caller-selected completion type.
///
/// Construction consumes an [`EventLoopProxy`] created by the caller for
/// [`CompletionEvent<T>`] user events. Cloning this value clones only Winit's wake/send capability;
/// it does not clone a completion, allocate an adapter queue, start a thread, or execute the event
/// loop. The event-loop owner remains responsible for receiving and ordering user events.
pub struct CompletionEventProxy<T: Send + 'static> {
    proxy: EventLoopProxy<CompletionEvent<T>>,
}

impl<T: Send + 'static> CompletionEventProxy<T> {
    /// Wraps a caller-created Winit event-loop proxy.
    pub const fn new(proxy: EventLoopProxy<CompletionEvent<T>>) -> Self {
        Self { proxy }
    }

    /// Moves one completion into Winit's user-event channel.
    ///
    /// If the associated event loop no longer exists, the returned error owns the exact original
    /// completion. No retry or fallback delivery occurs.
    pub fn send_completion(&self, completion: T) -> Result<(), CompletionSendError<T>> {
        self.proxy
            .send_event(CompletionEvent::new(completion))
            .map_err(CompletionSendError::from)
    }
}

impl<T: Send + 'static> Clone for CompletionEventProxy<T> {
    fn clone(&self) -> Self {
        Self {
            proxy: self.proxy.clone(),
        }
    }
}

impl<T: Send + 'static> fmt::Debug for CompletionEventProxy<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.pad("CompletionEventProxy { .. }")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NonCloneCompletion;

    fn assert_clone<T: Clone>() {}
    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn proxy_is_cloneable_and_cross_thread_without_requiring_clone_payloads() {
        assert_clone::<CompletionEventProxy<NonCloneCompletion>>();
        assert_send_sync::<CompletionEventProxy<NonCloneCompletion>>();
    }

    #[test]
    fn diagnostics_omit_owned_and_returned_completion_payloads() {
        let event = CompletionEvent::new(String::from("private completion contents"));
        assert_eq!(
            format!("{event:?}"),
            "CompletionEvent { completion: \"<owned>\" }"
        );

        let error: CompletionSendError<String> = EventLoopClosed(event).into();
        assert_eq!(error.kind(), CompletionSendErrorKind::EventLoopClosed);
        assert_eq!(
            format!("{error:?}"),
            "CompletionSendError { kind: EventLoopClosed, completion: \"<returned>\" }"
        );
        assert!(!format!("{error:?}").contains("private completion contents"));
        assert_eq!(error.into_completion(), "private completion contents");
    }
}

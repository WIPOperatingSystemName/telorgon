#![cfg(any(feature = "application-software", feature = "application-vulkan"))]

use telorgon::platform::{AdmittedRequest, RequestId, RequestOutcome};
use telorgon::platform_winit::{
    CompletionEvent, CompletionEventProxy, CompletionSendError, CompletionSendErrorKind,
};
use winit::event_loop::{EventLoopClosed, EventLoopProxy};

struct AppliedValue(u32);

#[test]
fn immutable_envelope_preserves_one_neutral_terminal_completion() {
    let completion =
        AdmittedRequest::new(RequestId::MIN).complete(RequestOutcome::Applied(AppliedValue(7)));
    let event = CompletionEvent::new(completion);

    assert_eq!(event.completion().request_id(), RequestId::MIN);
    assert_eq!(
        event.completion().outcome().applied().map(|value| value.0),
        Some(7)
    );

    let completion = event.into_completion();
    assert_eq!(completion.request_id(), RequestId::MIN);
    assert_eq!(
        completion.into_outcome().applied().map(|value| value.0),
        Some(7)
    );
}

#[test]
fn closed_loop_conversion_returns_the_exact_nonclone_completion() {
    let completion = AdmittedRequest::new(RequestId::MIN)
        .complete(RequestOutcome::Applied(String::from("owned value")));
    let closed = EventLoopClosed(CompletionEvent::new(completion));
    let error: CompletionSendError<_> = closed.into();

    assert_eq!(error.kind(), CompletionSendErrorKind::EventLoopClosed);
    assert_eq!(error.completion().request_id(), RequestId::MIN);
    let completion = error.into_completion();
    assert_eq!(
        completion.into_outcome().applied().map(String::as_str),
        Some("owned value")
    );
}

#[test]
fn public_constructor_requires_a_caller_provided_winit_proxy() {
    fn accepts_constructor<T: Send + 'static>(
        _constructor: fn(EventLoopProxy<CompletionEvent<T>>) -> CompletionEventProxy<T>,
    ) {
    }

    accepts_constructor::<AppliedValue>(CompletionEventProxy::new);
}

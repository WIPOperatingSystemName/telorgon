use super::*;
use std::{
    future::poll_fn,
    sync::{Arc, Mutex},
    task::Poll,
    task::Waker,
};

#[derive(Clone, Debug, PartialEq)]
pub enum ScreenBrightnessRequestState {
    Queued,
    Dispatched,
    Complete(ScreenBrightnessOutcome),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenBrightnessCancellation {
    CancelledBeforeDispatch,
    TooLate,
    AlreadyTerminal,
}
struct State {
    value: ScreenBrightnessRequestState,
    waker: Option<Waker>,
}
pub(crate) struct Completion(Mutex<State>);
impl Completion {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self(Mutex::new(State {
            value: ScreenBrightnessRequestState::Queued,
            waker: None,
        })))
    }
    pub(crate) fn begin(&self) -> bool {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.value != ScreenBrightnessRequestState::Queued {
            return false;
        }
        state.value = ScreenBrightnessRequestState::Dispatched;
        true
    }
    pub(crate) fn finish(&self, outcome: ScreenBrightnessOutcome) {
        let waker = {
            let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
            if matches!(state.value, ScreenBrightnessRequestState::Complete(_)) {
                return;
            }
            state.value = ScreenBrightnessRequestState::Complete(outcome);
            state.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
    pub(crate) fn terminal(&self) -> bool {
        matches!(
            self.0.lock().unwrap_or_else(|e| e.into_inner()).value,
            ScreenBrightnessRequestState::Complete(_)
        )
    }
}
#[must_use = "admission is not confirmation; observe the request or shared snapshot"]
pub struct ScreenBrightnessRequest {
    pub(crate) id: ScreenBrightnessRequestId,
    pub(crate) completion: Arc<Completion>,
    pub(crate) device: ScreenBrightnessDeviceHandle,
    pub(crate) epoch: u64,
}
impl std::fmt::Debug for ScreenBrightnessRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScreenBrightnessRequest")
            .field("id", &self.id)
            .field("state", &self.state())
            .finish()
    }
}
impl ScreenBrightnessRequest {
    pub fn id(&self) -> ScreenBrightnessRequestId {
        self.id
    }
    pub fn state(&self) -> ScreenBrightnessRequestState {
        self.completion
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .value
            .clone()
    }
    pub fn cancel(&self) -> ScreenBrightnessCancellation {
        let (result, waker) = {
            let mut state = self.completion.0.lock().unwrap_or_else(|e| e.into_inner());
            match state.value {
                ScreenBrightnessRequestState::Queued => {
                    state.value =
                        ScreenBrightnessRequestState::Complete(ScreenBrightnessOutcome::Cancelled);
                    (
                        ScreenBrightnessCancellation::CancelledBeforeDispatch,
                        state.waker.take(),
                    )
                }
                ScreenBrightnessRequestState::Dispatched => {
                    (ScreenBrightnessCancellation::TooLate, None)
                }
                ScreenBrightnessRequestState::Complete(_) => {
                    (ScreenBrightnessCancellation::AlreadyTerminal, None)
                }
            }
        };
        if let Some(waker) = waker {
            waker.wake();
        }
        result
    }
    pub async fn completion(self) -> ScreenBrightnessOutcome {
        poll_fn(|cx| {
            let mut state = self.completion.0.lock().unwrap_or_else(|e| e.into_inner());
            if let ScreenBrightnessRequestState::Complete(result) = &state.value {
                Poll::Ready(result.clone())
            } else {
                state.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        })
        .await
    }
}

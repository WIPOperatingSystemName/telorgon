use super::*;
use std::{
    future::poll_fn,
    sync::{Arc, Mutex},
    task::Poll,
    task::Waker,
};

#[derive(Clone, Debug, PartialEq)]
pub enum NetworkRequestState {
    Queued,
    AwaitingCredentials,
    Dispatched,
    Complete(NetworkOutcome),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkCancellation {
    CancelledBeforeDispatch,
    TooLate,
    AlreadyTerminal,
}
struct State {
    value: NetworkRequestState,
    waker: Option<Waker>,
}
pub(crate) struct Completion(Mutex<State>);
impl Completion {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self(Mutex::new(State {
            value: NetworkRequestState::Queued,
            waker: None,
        })))
    }
    pub(crate) fn await_credentials(&self) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).value =
            NetworkRequestState::AwaitingCredentials;
    }
    pub(crate) fn begin(&self) -> bool {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if !matches!(
            state.value,
            NetworkRequestState::Queued | NetworkRequestState::AwaitingCredentials
        ) {
            return false;
        }
        state.value = NetworkRequestState::Dispatched;
        true
    }
    pub(crate) fn finish(&self, outcome: NetworkOutcome) {
        let waker = {
            let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
            if matches!(state.value, NetworkRequestState::Complete(_)) {
                return;
            }
            state.value = NetworkRequestState::Complete(outcome);
            state.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
    pub(crate) fn terminal(&self) -> bool {
        matches!(
            self.0.lock().unwrap_or_else(|e| e.into_inner()).value,
            NetworkRequestState::Complete(_)
        )
    }
}
#[must_use = "admission is not confirmation; observe the request or shared snapshot"]
pub struct NetworkRequest {
    pub(crate) id: NetworkRequestId,
    pub(crate) completion: Arc<Completion>,
}
impl std::fmt::Debug for NetworkRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NetworkRequest")
            .field("id", &self.id)
            .field("state", &self.state())
            .finish()
    }
}
impl NetworkRequest {
    pub fn id(&self) -> NetworkRequestId {
        self.id
    }
    pub fn state(&self) -> NetworkRequestState {
        self.completion
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .value
            .clone()
    }
    pub fn cancel(&self) -> NetworkCancellation {
        let (result, waker) = {
            let mut state = self.completion.0.lock().unwrap_or_else(|e| e.into_inner());
            match state.value {
                NetworkRequestState::Queued | NetworkRequestState::AwaitingCredentials => {
                    state.value = NetworkRequestState::Complete(NetworkOutcome::Cancelled);
                    (
                        NetworkCancellation::CancelledBeforeDispatch,
                        state.waker.take(),
                    )
                }
                NetworkRequestState::Dispatched => (NetworkCancellation::TooLate, None),
                NetworkRequestState::Complete(_) => (NetworkCancellation::AlreadyTerminal, None),
            }
        };
        if let Some(waker) = waker {
            waker.wake();
        }
        result
    }
    pub async fn completion(self) -> NetworkOutcome {
        poll_fn(|cx| {
            let mut state = self.completion.0.lock().unwrap_or_else(|e| e.into_inner());
            if let NetworkRequestState::Complete(result) = &state.value {
                Poll::Ready(result.clone())
            } else {
                state.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        })
        .await
    }
}

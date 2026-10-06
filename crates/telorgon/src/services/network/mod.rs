//! Host-owned network management. Constructing a controller performs no native I/O.
//! Handles submit requests; observer signals contain observed state, never credentials.
mod controller;
mod model;
mod provider;
mod request;
mod worker;

pub use controller::{NetworkController, NetworkHandle, NetworkObserver};
pub use model::*;
pub use provider::{NetworkDispatch, NetworkProvider};
pub use request::{NetworkCancellation, NetworkRequest, NetworkRequestState};

#[cfg(test)]
mod tests;

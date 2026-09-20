use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WaylandServerErrorKind {
    UnsupportedTarget,
    Allocation,
    Socket,
    Dispatch,
    Flush,
    InvalidVersion,
    InvalidTimeout,
    NativeFailure,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaylandServerError {
    kind: WaylandServerErrorKind,
    message: &'static str,
}

impl WaylandServerError {
    pub const fn new(kind: WaylandServerErrorKind, message: &'static str) -> Self {
        Self { kind, message }
    }

    pub const fn kind(&self) -> WaylandServerErrorKind {
        self.kind
    }

    pub const fn message(&self) -> &'static str {
        self.message
    }
}

impl fmt::Display for WaylandServerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for WaylandServerError {}

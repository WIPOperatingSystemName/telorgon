//! Startup decoration negotiation.

/// Selects the mode advertised to clients that negotiate decorations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DecorationNegotiation {
    /// Server decorations by default; honor explicit client-side requests.
    #[default]
    ClientPreference,
    /// Select server decorations even when a client requests client-side decorations.
    /// This cannot remove headers embedded in client pixels.
    PreferServer,
}

/// Negotiation policy for clients supporting server decorations. Rendering uses the
/// committed result, never this preference. X11 decoration hints are honored directly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DecorationPolicy {
    pub negotiation: DecorationNegotiation,
    /// Report tiled edges to client-decorated Wayland windows without changing placement.
    pub tiled_client_decorations: bool,
}

impl DecorationPolicy {
    pub const DEFAULT: Self = Self {
        negotiation: DecorationNegotiation::ClientPreference,
        tiled_client_decorations: false,
    };

    pub(crate) const fn server_decorated(self, client_prefers_server: bool) -> bool {
        matches!(self.negotiation, DecorationNegotiation::PreferServer) || client_prefers_server
    }
}

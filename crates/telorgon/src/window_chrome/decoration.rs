//! Startup decoration ownership and independent compositor frame parts.

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

/// Whether the compositor adds its own title bar and controls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TitleBarPolicy {
    /// Follow the negotiated decoration mode (or X11 decoration hints).
    #[default]
    Automatic,
    /// Add a server title bar regardless of the client's preference.
    Always,
    /// Omit the server title bar without disabling window actions.
    Never,
}

/// Participation of one outer-frame visual, independent of title-bar ownership.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FramePartPolicy {
    /// Follow the negotiated decoration mode (or X11 decoration hints).
    #[default]
    Automatic,
    /// Include the part for client-decorated windows too; state-specific styles still apply.
    Always,
    /// Omit the part.
    Never,
}

/// Outer appearance; actual widths, radii and colors remain in the frame template.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OuterFramePolicy {
    pub border: FramePartPolicy,
    pub rounded_clip: FramePartPolicy,
    pub shadow: FramePartPolicy,
}

/// Compositor-owned resize hit regions, independent of client resize requests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResizeRegionPolicy {
    /// Follow decoration ownership.
    #[default]
    Automatic,
    /// Permit frame resize regions, subject to window capabilities and state styles.
    Enabled,
    /// Omit frame resize regions; does not prohibit client or keyboard resizing.
    Disabled,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameInteractionPolicy {
    pub resize_regions: ResizeRegionPolicy,
}

/// Startup policy for managed toplevel windows, shared by Wayland and Xwayland.
/// Fullscreen and unmanaged/popup surfaces never receive a compositor frame.
/// Custom frame templates must honor the resolved fields in `WindowChromeModel`;
/// `easy_window_frame` does so automatically. No setting removes client-drawn pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DecorationPolicy {
    pub negotiation: DecorationNegotiation,
    pub title_bar: TitleBarPolicy,
    pub outer_frame: OuterFramePolicy,
    pub interaction: FrameInteractionPolicy,
}

/// Resolved participation supplied to custom frame templates. State-specific styling
/// and capabilities may further suppress these parts, but must not re-enable them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowFrameParts {
    pub border: bool,
    pub rounded_clip: bool,
    pub shadow: bool,
    pub resize_regions: bool,
}
impl Default for WindowFrameParts {
    fn default() -> Self {
        Self {
            border: true,
            rounded_clip: true,
            shadow: true,
            resize_regions: true,
        }
    }
}

impl DecorationPolicy {
    pub const DEFAULT: Self = Self {
        negotiation: DecorationNegotiation::ClientPreference,
        title_bar: TitleBarPolicy::Automatic,
        outer_frame: OuterFramePolicy {
            border: FramePartPolicy::Automatic,
            rounded_clip: FramePartPolicy::Automatic,
            shadow: FramePartPolicy::Automatic,
        },
        interaction: FrameInteractionPolicy {
            resize_regions: ResizeRegionPolicy::Automatic,
        },
    };

    pub(crate) const fn server_decorated(self, client_prefers_server: bool) -> bool {
        matches!(self.negotiation, DecorationNegotiation::PreferServer) || client_prefers_server
    }

    pub(crate) const fn title_bar_visible(self, client_prefers_server: bool) -> bool {
        match self.title_bar {
            TitleBarPolicy::Automatic => self.server_decorated(client_prefers_server),
            TitleBarPolicy::Always => true,
            TitleBarPolicy::Never => false,
        }
    }

    pub(crate) const fn frame_parts(self, client_prefers_server: bool) -> WindowFrameParts {
        let server = self.server_decorated(client_prefers_server);
        WindowFrameParts {
            border: self.outer_frame.border.enabled(server),
            rounded_clip: self.outer_frame.rounded_clip.enabled(server),
            shadow: self.outer_frame.shadow.enabled(server),
            resize_regions: match self.interaction.resize_regions {
                ResizeRegionPolicy::Automatic => server,
                ResizeRegionPolicy::Enabled => true,
                ResizeRegionPolicy::Disabled => false,
            },
        }
    }
}
impl FramePartPolicy {
    const fn enabled(self, server: bool) -> bool {
        match self {
            Self::Automatic => server,
            Self::Always => true,
            Self::Never => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_honors_client_requests_and_server_override_is_independent_of_visuals() {
        let mut policy = DecorationPolicy::default();
        assert_eq!(policy, DecorationPolicy::DEFAULT);
        assert!(policy.title_bar_visible(true));
        assert!(!policy.title_bar_visible(false));
        assert!(!policy.frame_parts(false).border);
        policy.outer_frame.border = FramePartPolicy::Always;
        assert!(!policy.title_bar_visible(false));
        assert!(policy.frame_parts(false).border);
        assert!(!policy.frame_parts(false).shadow);
        policy.negotiation = DecorationNegotiation::PreferServer;
        assert!(policy.title_bar_visible(false));
        assert!(policy.frame_parts(false).shadow);
        policy.title_bar = TitleBarPolicy::Never;
        policy.outer_frame.rounded_clip = FramePartPolicy::Never;
        policy.interaction.resize_regions = ResizeRegionPolicy::Disabled;
        assert!(!policy.title_bar_visible(true));
        assert!(!policy.frame_parts(true).rounded_clip);
        assert!(!policy.frame_parts(true).resize_regions);
    }
}

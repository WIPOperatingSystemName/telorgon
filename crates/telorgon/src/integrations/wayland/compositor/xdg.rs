use std::collections::VecDeque;
use std::fmt;

use crate::foundation::{PointI, RectI, SizeI};

use crate::integrations::wayland::compositor::WaylandSurfaceId;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ResizeEdge {
    #[default]
    None,
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    BottomLeft,
    TopRight,
    BottomRight,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DecorationMode {
    #[default]
    ClientSide,
    ServerSide,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ToplevelState {
    pub maximized: bool,
    pub fullscreen: bool,
    pub resizing: bool,
    pub activated: bool,
    pub tiled_left: bool,
    pub tiled_right: bool,
    pub tiled_top: bool,
    pub tiled_bottom: bool,
    pub suspended: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XdgConfigure {
    pub serial: u32,
    pub size: Option<SizeI>,
    pub bounds: Option<SizeI>,
    pub states: ToplevelState,
    pub decoration: DecorationMode,
}

#[derive(Clone, Debug)]
pub struct XdgSurfaceState {
    pub surface: WaylandSurfaceId,
    configured: bool,
    pending: VecDeque<XdgConfigure>,
    last_acked: Option<XdgConfigure>,
    pending_acked: Option<XdgConfigure>,
    pending_window_geometry: Option<RectI>,
    current_window_geometry: Option<RectI>,
}

impl XdgSurfaceState {
    pub fn new(surface: WaylandSurfaceId) -> Self {
        Self {
            surface,
            configured: false,
            pending: VecDeque::new(),
            last_acked: None,
            pending_acked: None,
            pending_window_geometry: None,
            current_window_geometry: None,
        }
    }

    pub fn queue_configure(&mut self, configure: XdgConfigure) -> Result<(), XdgError> {
        if configure.serial == 0 {
            return Err(XdgError::InvalidSerial);
        }
        if configure
            .size
            .is_some_and(|size| size.width < 0 || size.height < 0)
            || configure
                .bounds
                .is_some_and(|size| size.width <= 0 || size.height <= 0)
        {
            return Err(XdgError::InvalidSize);
        }
        if self
            .pending
            .iter()
            .any(|pending| pending.serial == configure.serial)
        {
            return Err(XdgError::InvalidSerial);
        }
        self.pending.push_back(configure);
        Ok(())
    }

    pub fn ack_configure(&mut self, serial: u32) -> Result<XdgConfigure, XdgError> {
        let index = self
            .pending
            .iter()
            .position(|configure| configure.serial == serial)
            .ok_or(XdgError::UnknownConfigure)?;
        let configure = self.pending[index];
        self.pending.drain(..=index);
        self.configured = true;
        self.last_acked = Some(configure);
        self.pending_acked = Some(configure);
        Ok(configure)
    }

    pub fn validate_buffer_commit(&self, has_buffer: bool) -> Result<(), XdgError> {
        if has_buffer && !self.configured {
            Err(XdgError::UnconfiguredBuffer)
        } else {
            Ok(())
        }
    }

    pub fn set_window_geometry(&mut self, geometry: RectI) -> Result<(), XdgError> {
        if geometry.width <= 0 || geometry.height <= 0 {
            return Err(XdgError::InvalidWindowGeometry);
        }
        self.pending_window_geometry = Some(geometry);
        Ok(())
    }

    /// Most recent queued or acknowledged state, for decoration-only configures.
    pub(crate) fn latest_configure(&self) -> Option<XdgConfigure> {
        self.pending.back().copied().or(self.last_acked)
    }

    pub fn last_acked(&self) -> Option<XdgConfigure> {
        self.last_acked
    }

    pub fn has_pending_configure(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Returns whether a configure that was previously queued has been acknowledged, either
    /// directly or by acknowledging a newer configure that superseded it.
    pub fn configure_was_acknowledged(&self, serial: u32) -> bool {
        self.last_acked.is_some()
            && !self
                .pending
                .iter()
                .any(|configure| configure.serial == serial)
    }

    pub fn window_geometry(&self) -> Option<RectI> {
        self.current_window_geometry
    }

    /// Applies the xdg state associated with one `wl_surface.commit`.
    ///
    /// Only an acknowledgement issued since the preceding commit belongs to this commit. Window
    /// geometry is persistent double-buffered state and therefore remains current until replaced.
    pub fn commit_state(&mut self) -> (Option<XdgConfigure>, Option<RectI>) {
        if let Some(geometry) = self.pending_window_geometry.take() {
            self.current_window_geometry = Some(geometry);
        }
        (self.pending_acked.take(), self.current_window_geometry)
    }
}

#[derive(Clone, Debug)]
pub struct XdgToplevelState {
    pub title: String,
    pub application_id: String,
    pub parent: Option<WaylandSurfaceId>,
    pub minimum_size: Option<SizeI>,
    pub maximum_size: Option<SizeI>,
    pub decoration: DecorationMode,
}

impl Default for XdgToplevelState {
    fn default() -> Self {
        Self {
            title: String::new(),
            application_id: String::new(),
            parent: None,
            minimum_size: None,
            maximum_size: None,
            // Telorgon is a desktop shell as well as a protocol server.  A client that does not
            // negotiate xdg-decoration therefore receives Telorgon's composed frame by default;
            // an explicit client-side decoration request still overrides this value.
            decoration: DecorationMode::ServerSide,
        }
    }
}

impl XdgToplevelState {
    pub fn set_title(&mut self, title: impl Into<String>) -> Result<(), XdgError> {
        self.title = bounded_string(title.into())?;
        Ok(())
    }

    pub fn set_application_id(
        &mut self,
        application_id: impl Into<String>,
    ) -> Result<(), XdgError> {
        self.application_id = bounded_string(application_id.into())?;
        Ok(())
    }

    pub fn set_size_constraints(
        &mut self,
        minimum: Option<SizeI>,
        maximum: Option<SizeI>,
    ) -> Result<(), XdgError> {
        let valid = |size: SizeI| size.width >= 0 && size.height >= 0;
        if minimum.is_some_and(|size| !valid(size)) || maximum.is_some_and(|size| !valid(size)) {
            return Err(XdgError::InvalidSize);
        }
        if let (Some(minimum), Some(maximum)) = (minimum, maximum)
            && ((maximum.width != 0 && minimum.width > maximum.width)
                || (maximum.height != 0 && minimum.height > maximum.height))
        {
            return Err(XdgError::InvalidSize);
        }
        self.minimum_size = minimum;
        self.maximum_size = maximum;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XdgPositioner {
    pub size: SizeI,
    pub anchor_rect: RectI,
    /// Raw `xdg_positioner.anchor` value from the stable xdg-shell protocol.
    pub anchor: u32,
    /// Raw `xdg_positioner.gravity` value from the stable xdg-shell protocol.
    pub gravity: u32,
    /// Bitset of stable xdg-shell constraint-adjustment flags.
    pub constraint_adjustment: u32,
    pub offset: PointI,
    pub reactive: bool,
    pub parent_size: Option<SizeI>,
    pub parent_configure: Option<u32>,
}

impl XdgPositioner {
    pub fn validate(self) -> Result<Self, XdgError> {
        if self.size.width <= 0
            || self.size.height <= 0
            || self.anchor_rect.width <= 0
            || self.anchor_rect.height <= 0
            || self
                .parent_size
                .is_some_and(|size| size.width <= 0 || size.height <= 0)
            || self.parent_configure == Some(0)
            || self.anchor > 8
            || self.gravity > 8
            || self.constraint_adjustment & !0x3f != 0
        {
            return Err(XdgError::InvalidPositioner);
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XdgPopupState {
    pub parent: Option<WaylandSurfaceId>,
    pub positioner: XdgPositioner,
    pub grabbed: bool,
    pub reposition_token: Option<u32>,
}

fn bounded_string(value: String) -> Result<String, XdgError> {
    if value.len() > 4096 || value.contains('\0') {
        Err(XdgError::InvalidString)
    } else {
        Ok(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XdgError {
    InvalidSerial,
    UnknownConfigure,
    UnconfiguredBuffer,
    InvalidSize,
    InvalidWindowGeometry,
    InvalidString,
    InvalidPositioner,
}

impl fmt::Display for XdgError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "xdg-shell state validation failed: {self:?}")
    }
}

impl std::error::Error for XdgError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_buffer_requires_an_acked_configure() {
        let mut surface = XdgSurfaceState::new(WaylandSurfaceId::from_raw(1).unwrap());
        assert_eq!(
            surface.validate_buffer_commit(true),
            Err(XdgError::UnconfiguredBuffer)
        );
        surface
            .queue_configure(XdgConfigure {
                serial: 9,
                size: None,
                bounds: None,
                states: ToplevelState::default(),
                decoration: DecorationMode::ServerSide,
            })
            .unwrap();
        surface.ack_configure(9).unwrap();
        assert!(surface.validate_buffer_commit(true).is_ok());
    }

    #[test]
    fn acknowledging_a_newer_configure_discards_older_configures() {
        let mut surface = XdgSurfaceState::new(WaylandSurfaceId::from_raw(1).unwrap());
        for serial in [9, 10] {
            surface
                .queue_configure(XdgConfigure {
                    serial,
                    size: None,
                    bounds: None,
                    states: ToplevelState::default(),
                    decoration: DecorationMode::ClientSide,
                })
                .unwrap();
        }
        surface.ack_configure(10).unwrap();
        assert!(surface.configure_was_acknowledged(9));
        assert!(surface.configure_was_acknowledged(10));
        assert_eq!(surface.ack_configure(9), Err(XdgError::UnknownConfigure));
    }

    #[test]
    fn pending_configure_is_not_reported_as_acknowledged() {
        let mut surface = XdgSurfaceState::new(WaylandSurfaceId::from_raw(1).unwrap());
        surface
            .queue_configure(XdgConfigure {
                serial: 9,
                size: None,
                bounds: None,
                states: ToplevelState::default(),
                decoration: DecorationMode::ServerSide,
            })
            .unwrap();
        assert!(!surface.configure_was_acknowledged(9));
    }

    #[test]
    fn unacknowledged_configures_are_never_evicted() {
        let mut surface = XdgSurfaceState::new(WaylandSurfaceId::from_raw(1).unwrap());
        for serial in 1..=256 {
            surface
                .queue_configure(XdgConfigure {
                    serial,
                    size: Some(SizeI {
                        width: 640 + serial as i32,
                        height: 480,
                    }),
                    bounds: None,
                    states: ToplevelState::default(),
                    decoration: DecorationMode::ServerSide,
                })
                .unwrap();
        }

        let acknowledged = surface.ack_configure(1).unwrap();
        assert_eq!(acknowledged.serial, 1);
        assert_eq!(surface.ack_configure(256).unwrap().serial, 256);
    }

    #[test]
    fn configure_serial_can_be_reused_after_protocol_wrap() {
        let mut surface = XdgSurfaceState::new(WaylandSurfaceId::from_raw(1).unwrap());
        let configure = XdgConfigure {
            serial: 1,
            size: None,
            bounds: None,
            states: ToplevelState::default(),
            decoration: DecorationMode::ServerSide,
        };
        surface.queue_configure(configure).unwrap();
        surface.ack_configure(1).unwrap();
        surface.queue_configure(configure).unwrap();
        assert_eq!(surface.ack_configure(1).unwrap(), configure);
    }

    #[test]
    fn acknowledgement_and_window_geometry_are_latched_by_the_next_commit_only() {
        let mut surface = XdgSurfaceState::new(WaylandSurfaceId::from_raw(1).unwrap());
        let configure = XdgConfigure {
            serial: 9,
            size: Some(SizeI {
                width: 640,
                height: 480,
            }),
            bounds: None,
            states: ToplevelState::default(),
            decoration: DecorationMode::ServerSide,
        };
        let geometry = RectI {
            x: 4,
            y: 8,
            width: 632,
            height: 464,
        };
        surface.queue_configure(configure).unwrap();
        surface.ack_configure(configure.serial).unwrap();
        surface.set_window_geometry(geometry).unwrap();

        assert_eq!(surface.window_geometry(), None);
        assert_eq!(surface.commit_state(), (Some(configure), Some(geometry)));
        assert_eq!(surface.window_geometry(), Some(geometry));
        assert_eq!(surface.commit_state(), (None, Some(geometry)));
    }

    #[test]
    fn telorgon_owns_the_default_toplevel_decoration() {
        assert_eq!(
            XdgToplevelState::default().decoration,
            DecorationMode::ServerSide
        );
    }
}

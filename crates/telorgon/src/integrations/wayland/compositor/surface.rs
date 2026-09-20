use std::fmt;

use crate::foundation::{PointI, RectI};

use crate::integrations::wayland::compositor::{Region, WaylandBufferId, WaylandSurfaceId, XdgConfigure};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BufferTransform {
    #[default]
    Normal,
    Rotate90,
    Rotate180,
    Rotate270,
    Flipped,
    Flipped90,
    Flipped180,
    Flipped270,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SurfaceRole {
    XdgToplevel,
    XdgPopup,
    Subsurface,
    Cursor,
    DragIcon,
    SessionLock,
    Xwayland,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BufferAttachment {
    pub buffer: WaylandBufferId,
    pub offset: PointI,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SurfaceCommit {
    pub attachment: Option<Option<BufferAttachment>>,
    pub damage: Vec<RectI>,
    pub buffer_damage: Vec<RectI>,
    pub opaque_region: Option<Option<Region>>,
    pub input_region: Option<Option<Region>>,
    pub buffer_scale: Option<i32>,
    pub buffer_transform: Option<BufferTransform>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurfaceStateSnapshot {
    pub surface: WaylandSurfaceId,
    pub revision: u64,
    pub role: Option<SurfaceRole>,
    pub attachment: Option<BufferAttachment>,
    pub damage: Vec<RectI>,
    pub buffer_damage: Vec<RectI>,
    pub opaque_region: Option<Region>,
    pub input_region: Option<Region>,
    pub buffer_scale: i32,
    pub buffer_transform: BufferTransform,
    /// The last xdg configure acknowledged since the preceding surface commit, if any.
    pub acknowledged_configure: Option<XdgConfigure>,
    /// Persistent xdg window geometry after applying pending double-buffered state.
    pub window_geometry: Option<RectI>,
    /// One committed xwayland-shell association serial. Owned by wl_surface,
    /// independent of the lifetime of its xwayland_surface protocol object.
    pub xwayland_serial: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct SurfaceState {
    surface: WaylandSurfaceId,
    revision: u64,
    role: Option<SurfaceRole>,
    current: SurfaceStateSnapshot,
    pending: SurfaceCommit,
    pending_xwayland_serial: Option<u64>,
}

impl SurfaceState {
    pub fn new(surface: WaylandSurfaceId) -> Self {
        let current = SurfaceStateSnapshot {
            surface,
            revision: 1,
            role: None,
            attachment: None,
            damage: Vec::new(),
            buffer_damage: Vec::new(),
            opaque_region: None,
            input_region: None,
            buffer_scale: 1,
            buffer_transform: BufferTransform::Normal,
            acknowledged_configure: None,
            window_geometry: None,
            xwayland_serial: None,
        };
        Self {
            surface,
            revision: 1,
            role: None,
            current,
            pending: SurfaceCommit::default(),
            pending_xwayland_serial: None,
        }
    }

    pub const fn id(&self) -> WaylandSurfaceId {
        self.surface
    }

    pub fn assign_role(&mut self, role: SurfaceRole) -> Result<(), SurfaceError> {
        match self.role {
            None => {
                self.role = Some(role);
                self.current.role = Some(role);
                Ok(())
            }
            Some(current) if current == role => Ok(()),
            Some(_) => Err(SurfaceError::RoleAlreadyAssigned),
        }
    }

    /// xwayland-shell forbids creating a second role object even if it has the
    /// same role as an earlier, destroyed object. Native role assignment retains
    /// its existing idempotence semantics.
    pub fn assign_xwayland_role(&mut self) -> Result<(), SurfaceError> {
        if self.role.is_some() {
            return Err(SurfaceError::RoleAlreadyAssigned);
        }
        self.assign_role(SurfaceRole::Xwayland)
    }

    /// Stage the surface-local half of association. The protocol adapter must
    /// additionally enforce generation-wide serial uniqueness/monotonicity and
    /// authenticate the dedicated client before calling this method.
    pub fn set_xwayland_serial(&mut self, serial: u64) -> Result<(), SurfaceError> {
        if self.role != Some(SurfaceRole::Xwayland) {
            return Err(SurfaceError::InvalidXwaylandRole);
        }
        if serial == 0 {
            return Err(SurfaceError::InvalidXwaylandSerial);
        }
        if self.current.xwayland_serial.is_some() {
            return Err(SurfaceError::XwaylandAlreadyAssociated);
        }
        self.pending_xwayland_serial = Some(serial);
        Ok(())
    }

    pub fn stage(&mut self, mut commit: SurfaceCommit) -> Result<(), SurfaceError> {
        commit
            .damage
            .retain(|rectangle| rectangle.width > 0 && rectangle.height > 0);
        commit
            .buffer_damage
            .retain(|rectangle| rectangle.width > 0 && rectangle.height > 0);
        if commit.damage.len() + commit.buffer_damage.len() > 256 {
            return Err(SurfaceError::TooManyDamageRectangles);
        }
        if commit.buffer_scale.is_some_and(|scale| scale <= 0) {
            return Err(SurfaceError::InvalidBufferScale);
        }
        self.pending = commit;
        Ok(())
    }

    pub fn attach(&mut self, attachment: Option<BufferAttachment>) {
        self.pending.attachment = Some(attachment);
    }

    pub fn damage(&mut self, rectangle: RectI) -> Result<(), SurfaceError> {
        if rectangle.width <= 0 || rectangle.height <= 0 {
            return Ok(());
        }
        if self.pending.damage.len() + self.pending.buffer_damage.len() >= 256 {
            return Err(SurfaceError::TooManyDamageRectangles);
        }
        self.pending.damage.push(rectangle);
        Ok(())
    }

    pub fn damage_buffer(&mut self, rectangle: RectI) -> Result<(), SurfaceError> {
        if rectangle.width <= 0 || rectangle.height <= 0 {
            return Ok(());
        }
        if self.pending.damage.len() + self.pending.buffer_damage.len() >= 256 {
            return Err(SurfaceError::TooManyDamageRectangles);
        }
        self.pending.buffer_damage.push(rectangle);
        Ok(())
    }

    pub fn set_opaque_region(&mut self, region: Option<Region>) {
        self.pending.opaque_region = Some(region);
    }

    pub fn set_input_region(&mut self, region: Option<Region>) {
        self.pending.input_region = Some(region);
    }

    pub fn set_buffer_scale(&mut self, scale: i32) -> Result<(), SurfaceError> {
        if scale <= 0 {
            return Err(SurfaceError::InvalidBufferScale);
        }
        self.pending.buffer_scale = Some(scale);
        Ok(())
    }

    pub fn set_buffer_transform(&mut self, transform: BufferTransform) {
        self.pending.buffer_transform = Some(transform);
    }

    pub fn pending(&self) -> &SurfaceCommit {
        &self.pending
    }

    pub fn commit(&mut self) -> Result<CommitOutcome, SurfaceError> {
        let revision = self
            .revision
            .checked_add(1)
            .ok_or(SurfaceError::RevisionExhausted)?;
        let pending = std::mem::take(&mut self.pending);
        if let Some(serial) = self.pending_xwayland_serial.take() {
            self.current.xwayland_serial = Some(serial);
        }
        let previous_buffer = self.current.attachment.map(|attachment| attachment.buffer);
        if let Some(attachment) = pending.attachment {
            self.current.attachment = attachment;
        }
        if let Some(region) = pending.opaque_region {
            self.current.opaque_region = region;
        }
        if let Some(region) = pending.input_region {
            self.current.input_region = region;
        }
        if let Some(scale) = pending.buffer_scale {
            self.current.buffer_scale = scale;
        }
        if let Some(transform) = pending.buffer_transform {
            self.current.buffer_transform = transform;
        }
        self.current.damage = pending.damage;
        self.current.buffer_damage = pending.buffer_damage;
        self.revision = revision;
        self.current.revision = revision;
        self.current.acknowledged_configure = None;
        self.current.role = self.role;
        let current_buffer = self.current.attachment.map(|attachment| attachment.buffer);
        Ok(CommitOutcome {
            revision,
            previous_buffer: (previous_buffer != current_buffer)
                .then_some(previous_buffer)
                .flatten(),
            current_buffer,
            mapped: current_buffer.is_some(),
        })
    }

    pub fn apply_xdg_commit_state(
        &mut self,
        acknowledged_configure: Option<XdgConfigure>,
        window_geometry: Option<RectI>,
    ) {
        self.current.acknowledged_configure = acknowledged_configure;
        self.current.window_geometry = window_geometry;
    }

    pub fn snapshot(&self) -> &SurfaceStateSnapshot {
        &self.current
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommitOutcome {
    pub revision: u64,
    pub previous_buffer: Option<WaylandBufferId>,
    pub current_buffer: Option<WaylandBufferId>,
    pub mapped: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceError {
    RoleAlreadyAssigned,
    InvalidBufferScale,
    InvalidDamageRectangle,
    TooManyDamageRectangles,
    RevisionExhausted,
    InvalidXwaylandRole,
    InvalidXwaylandSerial,
    XwaylandAlreadyAssociated,
}

impl fmt::Display for SurfaceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::RoleAlreadyAssigned => "Wayland surface already has another permanent role",
            Self::InvalidBufferScale => "Wayland surface buffer scale must be positive",
            Self::InvalidDamageRectangle => "Wayland surface damage must be positive",
            Self::TooManyDamageRectangles => "Wayland surface damage exceeds its hard bound",
            Self::RevisionExhausted => "Wayland surface revision is exhausted",
            Self::InvalidXwaylandRole => "surface does not have the Xwayland role",
            Self::InvalidXwaylandSerial => "Xwayland association serial must be nonzero",
            Self::XwaylandAlreadyAssociated => "Xwayland surface already committed an association",
        })
    }
}

impl std::error::Error for SurfaceError {}

#[cfg(test)]
mod tests {
    #[test]
    fn xwayland_serial_is_committed_once_and_survives_buffer_commits() {
        let mut state = SurfaceState::new(surface());
        assert_eq!(
            state.set_xwayland_serial(1),
            Err(SurfaceError::InvalidXwaylandRole)
        );
        state.assign_xwayland_role().unwrap();
        assert_eq!(
            state.assign_xwayland_role(),
            Err(SurfaceError::RoleAlreadyAssigned)
        );
        assert_eq!(
            state.set_xwayland_serial(0),
            Err(SurfaceError::InvalidXwaylandSerial)
        );
        state.set_xwayland_serial(1u64 << 40).unwrap();
        assert_eq!(state.snapshot().xwayland_serial, None);
        // Native pending state staging does not erase the independent role state.
        state.stage(SurfaceCommit::default()).unwrap();
        state.commit().unwrap();
        assert_eq!(state.snapshot().xwayland_serial, Some(1u64 << 40));
        assert_eq!(
            state.set_xwayland_serial(2),
            Err(SurfaceError::XwaylandAlreadyAssociated)
        );
        state.attach(None);
        state.commit().unwrap();
        assert_eq!(state.snapshot().xwayland_serial, Some(1u64 << 40));
        assert_eq!(
            state.assign_role(SurfaceRole::XdgToplevel),
            Err(SurfaceError::RoleAlreadyAssigned)
        );
    }

    #[test]
    fn failed_commit_does_not_publish_or_consume_pending_xwayland_serial() {
        let mut state = SurfaceState::new(surface());
        state.assign_xwayland_role().unwrap();
        state.set_xwayland_serial(7).unwrap();
        state.revision = u64::MAX;
        assert_eq!(state.commit(), Err(SurfaceError::RevisionExhausted));
        assert_eq!(state.snapshot().xwayland_serial, None);
        assert_eq!(state.pending_xwayland_serial, Some(7));
    }

    #[test]
    fn xwayland_role_does_not_replace_native_roles() {
        for role in [
            SurfaceRole::XdgToplevel,
            SurfaceRole::XdgPopup,
            SurfaceRole::Subsurface,
            SurfaceRole::Cursor,
            SurfaceRole::DragIcon,
            SurfaceRole::SessionLock,
        ] {
            let mut state = SurfaceState::new(surface());
            state.assign_role(role).unwrap();
            assert_eq!(
                state.assign_xwayland_role(),
                Err(SurfaceError::RoleAlreadyAssigned)
            );
            assert_eq!(state.snapshot().role, Some(role));
            assert_eq!(state.snapshot().xwayland_serial, None);
        }
    }
    use std::num::NonZeroU32;

    use super::*;

    #[test]
    fn surface_and_buffer_damage_remain_separate_until_commit() {
        let mut surface = SurfaceState::new(WaylandSurfaceId::from_raw(1).unwrap());
        let logical = RectI {
            x: 2,
            y: 3,
            width: 4,
            height: 5,
        };
        let physical = RectI {
            x: 6,
            y: 9,
            width: 12,
            height: 15,
        };
        surface.damage(logical).unwrap();
        surface.damage_buffer(physical).unwrap();
        surface.set_buffer_scale(3).unwrap();
        surface.commit().unwrap();
        assert_eq!(surface.snapshot().damage, [logical]);
        assert_eq!(surface.snapshot().buffer_damage, [physical]);
        surface.commit().unwrap();
        assert!(surface.snapshot().damage.is_empty());
        assert!(surface.snapshot().buffer_damage.is_empty());
    }

    fn surface() -> WaylandSurfaceId {
        WaylandSurfaceId::new(NonZeroU32::new(1).unwrap())
    }

    fn buffer(raw: u32) -> WaylandBufferId {
        WaylandBufferId::new(NonZeroU32::new(raw).unwrap())
    }

    #[test]
    fn state_is_double_buffered_until_commit() {
        let mut state = SurfaceState::new(surface());
        state
            .stage(SurfaceCommit {
                attachment: Some(Some(BufferAttachment {
                    buffer: buffer(2),
                    offset: PointI { x: 0, y: 0 },
                })),
                buffer_scale: Some(2),
                ..SurfaceCommit::default()
            })
            .unwrap();
        assert!(state.snapshot().attachment.is_none());
        assert_eq!(state.snapshot().buffer_scale, 1);
        let outcome = state.commit().unwrap();
        assert_eq!(outcome.current_buffer, Some(buffer(2)));
        assert_eq!(state.snapshot().buffer_scale, 2);
    }

    #[test]
    fn roles_are_permanent() {
        let mut state = SurfaceState::new(surface());
        state.assign_role(SurfaceRole::XdgToplevel).unwrap();
        state.assign_role(SurfaceRole::XdgToplevel).unwrap();
        assert_eq!(
            state.assign_role(SurfaceRole::Cursor),
            Err(SurfaceError::RoleAlreadyAssigned)
        );
    }

    #[test]
    fn replacing_a_buffer_reports_the_retired_identity() {
        let mut state = SurfaceState::new(surface());
        for raw in [2, 3] {
            state
                .stage(SurfaceCommit {
                    attachment: Some(Some(BufferAttachment {
                        buffer: buffer(raw),
                        offset: PointI { x: 0, y: 0 },
                    })),
                    ..SurfaceCommit::default()
                })
                .unwrap();
            let outcome = state.commit().unwrap();
            if raw == 3 {
                assert_eq!(outcome.previous_buffer, Some(buffer(2)));
            }
        }
    }

    #[test]
    fn empty_damage_is_ignored_without_consuming_the_rectangle_limit() {
        let mut state = SurfaceState::new(surface());
        for rectangle in [
            RectI {
                x: 0,
                y: 0,
                width: 10,
                height: 0,
            },
            RectI {
                x: 0,
                y: 0,
                width: 0,
                height: 10,
            },
            RectI {
                x: 0,
                y: 0,
                width: -1,
                height: 10,
            },
            RectI {
                x: 0,
                y: 0,
                width: 10,
                height: -1,
            },
        ] {
            for _ in 0..257 {
                state.damage(rectangle).unwrap();
            }
        }
        let visible = RectI {
            x: 1,
            y: 2,
            width: 3,
            height: 4,
        };
        state.damage(visible).unwrap();
        state.commit().unwrap();
        assert_eq!(state.snapshot().damage, vec![visible]);
    }

    #[test]
    fn staged_commits_filter_empty_damage_before_enforcing_the_limit() {
        let visible = RectI {
            x: 1,
            y: 2,
            width: 3,
            height: 4,
        };
        let mut damage = vec![
            RectI {
                x: 0,
                y: 0,
                width: 0,
                height: 10,
            };
            257
        ];
        damage.push(visible);
        let mut state = SurfaceState::new(surface());
        state
            .stage(SurfaceCommit {
                damage,
                ..SurfaceCommit::default()
            })
            .unwrap();
        state.commit().unwrap();
        assert_eq!(state.snapshot().damage, vec![visible]);
    }
}

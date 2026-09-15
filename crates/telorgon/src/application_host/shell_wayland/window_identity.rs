//! Shell identity is independent of presentation. Native windows are keyed by
//! their permanent toplevel surface; X11 windows by server generation and XID
//! incarnation. Destruction, not unmapping or surface replacement, retires a slot.
use super::*;
use crate::shell::WindowId;
use std::num::NonZeroU32;
/// Adapter identity, never a surface association or a client-supplied title/PID.
/// Callers admit only live windows from their respective protocol registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum ProtocolWindow {
    Native(WaylandSurfaceId),
    #[cfg(feature = "shell-xwayland")]
    X11(crate::xwayland::association::XWindow),
}
impl From<WaylandSurfaceId> for ProtocolWindow {
    fn from(surface: WaylandSurfaceId) -> Self {
        Self::Native(surface)
    }
}
#[cfg(feature = "shell-xwayland")]
impl From<crate::xwayland::association::XWindow> for ProtocolWindow {
    fn from(window: crate::xwayland::association::XWindow) -> Self {
        Self::X11(window)
    }
}

#[derive(Default)]
pub(super) struct WindowIdentities {
    windows: BTreeMap<ProtocolWindow, WindowId>,
    generations: Vec<u32>,
    free: Vec<usize>,
}
impl WindowIdentities {
    pub fn get(&self, window: impl Into<ProtocolWindow>) -> Option<WindowId> {
        self.windows.get(&window.into()).copied()
    }
    pub fn ensure(&mut self, window: impl Into<ProtocolWindow>) -> AppResult<WindowId> {
        let window = window.into();
        if let Some(id) = self.get(window) {
            return Ok(id);
        }
        let index = if let Some(index) = self.free.pop() {
            index
        } else {
            if self.generations.len() >= u32::MAX as usize {
                return Err(AppError::new("desktop window identity space exhausted"));
            }
            let index = self.generations.len();
            self.generations.push(1);
            index
        };
        let id = WindowId::new(
            NonZeroU32::new(index as u32 + 1).unwrap(),
            NonZeroU32::new(self.generations[index]).unwrap(),
        );
        self.windows.insert(window, id);
        Ok(id)
    }
    pub fn destroy(&mut self, window: impl Into<ProtocolWindow>) {
        if let Some(id) = self.windows.remove(&window.into()) {
            let index = id.slot() as usize - 1;
            // Exhausted slots are permanently retired rather than wrapping.
            if let Some(next) = self.generations[index].checked_add(1) {
                self.generations[index] = next;
                self.free.push(index);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn surface(value: u32) -> WaylandSurfaceId {
        WaylandSurfaceId::from_raw(value).unwrap()
    }
    #[test]
    fn republishing_keeps_identity_and_destroyed_slots_change_generation() {
        let mut ids = WindowIdentities::default();
        let original = ids.ensure(surface(1)).unwrap();
        assert_eq!(ids.ensure(surface(1)).unwrap(), original);
        // Dropping presentation does not call destroy: remap keeps this identity.
        assert_eq!(ids.get(surface(1)), Some(original));
        ids.destroy(surface(1));
        ids.destroy(surface(1));
        let reused = ids.ensure(surface(2)).unwrap();
        assert_eq!(original.slot(), reused.slot());
        assert_ne!(original, reused);
        assert_eq!(reused.generation(), original.generation() + 1);
        assert_eq!(ids.get(surface(1)), None);
    }
    #[cfg(feature = "shell-xwayland")]
    #[test]
    fn mixed_protocols_and_recycled_xids_never_share_identity() {
        use crate::xwayland::association::XWindow;
        let mut ids = WindowIdentities::default();
        let native = ids.ensure(surface(7)).unwrap();
        let first = XWindow {
            generation: 1,
            xid: 7,
            incarnation: 1,
        };
        let first_id = ids.ensure(first).unwrap();
        assert_ne!(native, first_id);
        let mut associations = crate::xwayland::association::Associations::new(1, 16).unwrap();
        assert_eq!(associations.create_window(7).unwrap(), first);
        associations.window_serial(first, 10).unwrap();
        associations.committed_surface(1, 100, 10).unwrap();
        assert_eq!(associations.surface_for(first), Some(100));
        associations.destroy_surface(1, 100).unwrap();
        assert_eq!(associations.surface_for(first), None);
        assert_eq!(ids.get(first), Some(first_id));
        // A new committed surface changes presentation, not policy identity.
        associations.committed_surface(1, 101, 11).unwrap();
        let replacement = associations.window_serial(first, 11).unwrap().unwrap();
        assert_eq!(replacement.surface, 101);
        assert_eq!(ids.ensure(replacement.window).unwrap(), first_id);
        ids.destroy(first);
        let recycled = XWindow {
            incarnation: 2,
            ..first
        };
        let recycled_id = ids.ensure(recycled).unwrap();
        assert_eq!(first_id.slot(), recycled_id.slot());
        assert_ne!(first_id, recycled_id);
        // A delayed destroy from the old XID lifetime cannot revoke its successor.
        ids.destroy(first);
        assert_eq!(ids.get(recycled), Some(recycled_id));
        let restarted = XWindow {
            generation: 2,
            ..first
        };
        let restarted_id = ids.ensure(restarted).unwrap();
        assert_ne!(restarted_id, recycled_id);
        assert_ne!(restarted_id, native);
        assert_eq!(ids.get(surface(7)), Some(native));
        assert_eq!(ids.get(first), None);
    }
    #[test]
    fn exhausted_generation_is_never_recycled() {
        let mut ids = WindowIdentities::default();
        let first = ids.ensure(surface(1)).unwrap();
        ids.generations[first.slot() as usize - 1] = u32::MAX;
        ids.destroy(surface(1));
        let next = ids.ensure(surface(2)).unwrap();
        assert_ne!(first.slot(), next.slot());
    }
}

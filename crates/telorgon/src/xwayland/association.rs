//! Association bookkeeping for committed xwayland-shell serials.
//!
//! Callers must authenticate the dedicated Wayland client before admitting a
//! serial. Surface identities must be compositor identities, never wire object
//! IDs. This table is independent of protocol-object lifetime and map state.
use super::{Error, Result};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct XWindow {
    pub generation: u64,
    pub xid: u32,
    /// Distinguishes reuse of an XID within one server generation.
    pub incarnation: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Association {
    pub window: XWindow,
    pub surface: u64,
}

/// Bounded per-server state. Exhausting history requires restarting the
/// compatibility instance, never forgetting old serials and accepting reuse.
pub struct Associations {
    generation: u64,
    capacity: usize,
    next_window: u64,
    windows: BTreeMap<u32, XWindow>,
    serial_windows: BTreeMap<u64, XWindow>,
    serial_surfaces: BTreeMap<u64, u64>,
    surface_serials: BTreeMap<u64, u64>,
    used_serials: BTreeSet<u64>,
    dead_surfaces: BTreeSet<u64>,
    links: BTreeMap<XWindow, u64>,
    current_serials: BTreeMap<XWindow, u64>,
}
impl Associations {
    pub fn new(generation: u64, capacity: usize) -> Result<Self> {
        if generation == 0 || capacity == 0 || capacity > 1_048_576 {
            return Err(Error("invalid association generation or bound".into()));
        }
        Ok(Self {
            generation,
            capacity,
            next_window: 1,
            windows: BTreeMap::new(),
            serial_windows: BTreeMap::new(),
            serial_surfaces: BTreeMap::new(),
            surface_serials: BTreeMap::new(),
            used_serials: BTreeSet::new(),
            dead_surfaces: BTreeSet::new(),
            links: BTreeMap::new(),
            current_serials: BTreeMap::new(),
        })
    }
    pub fn create_window(&mut self, xid: u32) -> Result<XWindow> {
        if xid == 0 || self.windows.contains_key(&xid) || self.windows.len() >= self.capacity {
            return Err(Error("invalid, duplicate or excessive X window".into()));
        }
        let next = self
            .next_window
            .checked_add(1)
            .ok_or_else(|| Error("window identity exhausted".into()))?;
        let window = XWindow {
            generation: self.generation,
            xid,
            incarnation: self.next_window,
        };
        self.next_window = next;
        self.windows.insert(xid, window);
        Ok(window)
    }
    fn live(&self, window: XWindow) -> bool {
        self.windows.get(&window.xid) == Some(&window)
    }
    fn join(&mut self, serial: u64) -> Option<Association> {
        let window = *self.serial_windows.get(&serial)?;
        let surface = *self.serial_surfaces.get(&serial)?;
        if !self.live(window)
            || self.dead_surfaces.contains(&surface)
            || self.current_serials.get(&window) != Some(&serial)
        {
            return None;
        }
        // A newer committed association replaces imagery without replacing the
        // desktop-window identity. Old serial records remain as tombstones.
        if self.links.get(&window) == Some(&surface) {
            return None;
        }
        self.links.insert(window, surface);
        Some(Association { window, surface })
    }
    /// Admit a WL_SURFACE_SERIAL event tagged with its live XID incarnation.
    pub fn window_serial(&mut self, window: XWindow, serial: u64) -> Result<Option<Association>> {
        if !self.live(window) {
            return Ok(None);
        }
        if serial == 0 {
            return Err(Error("zero Xwayland serial".into()));
        }
        if let Some(previous) = self.serial_windows.get(&serial) {
            if *previous != window {
                return Err(Error("Xwayland serial reused by another window".into()));
            }
            return Ok(None);
        }
        if self.serial_windows.len() >= self.capacity {
            return Err(Error("X serial history exhausted".into()));
        }
        self.serial_windows.insert(serial, window);
        self.current_serials.insert(window, serial);
        Ok(self.join(serial))
    }
    /// Admit only a serial latched by wl_surface.commit, not set_serial alone.
    pub fn committed_surface(
        &mut self,
        generation: u64,
        surface: u64,
        serial: u64,
    ) -> Result<Option<Association>> {
        if generation != self.generation || self.dead_surfaces.contains(&surface) {
            return Ok(None);
        }
        if surface == 0
            || serial == 0
            || self.used_serials.contains(&serial)
            || self.surface_serials.contains_key(&surface)
        {
            return Err(Error("invalid or repeated committed surface serial".into()));
        }
        if self.used_serials.len() >= self.capacity {
            return Err(Error("surface serial history exhausted".into()));
        }
        self.used_serials.insert(serial);
        self.surface_serials.insert(surface, serial);
        self.serial_surfaces.insert(serial, surface);
        Ok(self.join(serial))
    }
    pub fn surface_for(&self, window: XWindow) -> Option<u64> {
        self.links.get(&window).copied()
    }
    pub fn destroy_window(&mut self, window: XWindow) {
        if self.live(window) {
            self.windows.remove(&window.xid);
            self.links.remove(&window);
            self.current_serials.remove(&window);
        }
    }
    pub fn destroy_surface(&mut self, generation: u64, surface: u64) -> Result<()> {
        if generation != self.generation {
            return Ok(());
        }
        if !self.dead_surfaces.contains(&surface) && self.dead_surfaces.len() >= self.capacity {
            return Err(Error("destroyed surface history exhausted".into()));
        }
        self.dead_surfaces.insert(surface);
        self.links.retain(|_, value| *value != surface);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn either_channel_may_arrive_first() {
        for x_first in [false, true] {
            let mut table = Associations::new(1, 16).unwrap();
            let w = table.create_window(10).unwrap();
            let result = if x_first {
                assert_eq!(table.window_serial(w, 7).unwrap(), None);
                table.committed_surface(1, 100, 7).unwrap()
            } else {
                assert_eq!(table.committed_surface(1, 100, 7).unwrap(), None);
                table.window_serial(w, 7).unwrap()
            };
            assert_eq!(
                result,
                Some(Association {
                    window: w,
                    surface: 100
                })
            );
        }
    }
    #[test]
    fn destroyed_surface_cannot_be_resurrected() {
        let mut table = Associations::new(1, 16).unwrap();
        let w = table.create_window(10).unwrap();
        table.committed_surface(1, 100, 7).unwrap();
        table.destroy_surface(1, 100).unwrap();
        assert_eq!(table.window_serial(w, 7).unwrap(), None);
        assert_eq!(table.surface_for(w), None);
    }
    #[test]
    fn stale_xid_incarnation_and_server_generation_are_ignored() {
        let mut table = Associations::new(2, 16).unwrap();
        let old = table.create_window(10).unwrap();
        table.destroy_window(old);
        let new = table.create_window(10).unwrap();
        assert_ne!(old, new);
        assert_eq!(table.window_serial(old, 7).unwrap(), None);
        assert_eq!(table.committed_surface(1, 100, 7).unwrap(), None);
        assert_eq!(table.surface_for(new), None);
    }
    #[test]
    fn remapping_preserves_window_identity() {
        let mut table = Associations::new(1, 16).unwrap();
        let w = table.create_window(10).unwrap();
        for (surface, serial) in [(100, 7), (200, 8)] {
            table.window_serial(w, serial).unwrap();
            table.committed_surface(1, surface, serial).unwrap();
        }
        table.destroy_surface(1, 100).unwrap();
        assert_eq!(table.surface_for(w), Some(200));
        assert!(table.committed_surface(1, 300, 7).is_err());
    }
    #[test]
    fn bounds_do_not_evict_serial_history() {
        let mut table = Associations::new(1, 1).unwrap();
        table.committed_surface(1, 100, 7).unwrap();
        table.destroy_surface(1, 100).unwrap();
        assert!(table.committed_surface(1, 200, 8).is_err());
        assert!(table.committed_surface(1, 200, 7).is_err());
    }
    #[test]
    fn delayed_old_commit_cannot_replace_new_association() {
        let mut table = Associations::new(1, 16).unwrap();
        let w = table.create_window(10).unwrap();
        table.window_serial(w, 7).unwrap();
        table.window_serial(w, 8).unwrap();
        table.committed_surface(1, 200, 8).unwrap();
        assert_eq!(table.committed_surface(1, 100, 7).unwrap(), None);
        assert_eq!(table.surface_for(w), Some(200));
    }
}

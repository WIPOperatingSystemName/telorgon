//! Association bookkeeping for committed xwayland-shell serials.
//!
//! Callers must authenticate the dedicated Wayland client before admitting a
//! serial. Surface identities must be compositor identities, never wire object
//! IDs. This table is independent of protocol-object lifetime and map state.
use super::{Error, Result};
use std::collections::BTreeMap;

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

/// Bounded live associations with lossless interval-compressed replay history.
/// The live-window capacity is not a cumulative limit on a server's lifetimes.
pub struct Associations {
    generation: u64,
    capacity: usize,
    next_window: u64,
    windows: BTreeMap<u32, XWindow>,
    serial_windows: BTreeMap<u64, XWindow>,
    serial_surfaces: BTreeMap<u64, u64>,
    surface_serials: BTreeMap<u64, u64>,
    used_serials: History,
    used_x_serials: History,
    dead_surfaces: History,
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
            used_serials: History::default(),
            used_x_serials: History::default(),
            dead_surfaces: History::default(),
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
        if self.used_x_serials.contains(&serial) {
            return Err(Error("retired Xwayland serial reused".into()));
        }
        self.used_x_serials.insert(serial)?;
        if let Some(old) = self.current_serials.get(&window) {
            self.serial_windows.remove(old);
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
        if self.surface_serials.len() >= self.capacity {
            return Err(Error("live surface association bound reached".into()));
        }
        self.used_serials.insert(serial)?;
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
            if let Some(serial) = self.current_serials.remove(&window) {
                self.serial_windows.remove(&serial);
            }
        }
    }
    pub fn destroy_surface(&mut self, generation: u64, surface: u64) -> Result<()> {
        if generation != self.generation {
            return Ok(());
        }
        self.dead_surfaces.insert(surface)?;
        if let Some(serial) = self.surface_serials.remove(&surface) {
            self.serial_surfaces.remove(&serial);
        }
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
        assert!(table.committed_surface(1, 200, 8).is_ok());
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

/// Lossless compression: sequential IDs consume one range rather than one allocation each.
/// A separate fragmentation bound contains hostile sparse IDs without limiting ordinary churn.
#[derive(Default)]
struct History(BTreeMap<u64, u64>);
impl History {
    fn contains(&self, value: &u64) -> bool {
        self.0
            .range(..=value)
            .next_back()
            .is_some_and(|(_, end)| value <= end)
    }
    fn insert(&mut self, value: u64) -> Result<()> {
        if self.contains(&value) {
            return Ok(());
        }
        let previous = self.0.range(..value).next_back().map(|(&a, &b)| (a, b));
        let next = self.0.range(value..).next().map(|(&a, &b)| (a, b));
        let left = previous.filter(|(_, end)| end.checked_add(1) == Some(value));
        let right = next.filter(|(start, _)| value.checked_add(1) == Some(*start));
        if left.is_none() && right.is_none() && self.0.len() >= 1_048_576 {
            return Err(Error(
                "association history fragmentation bound reached".into(),
            ));
        }
        let start = left.map_or(value, |(start, _)| start);
        let end = right.map_or(value, |(_, end)| end);
        if let Some((start, _)) = right {
            self.0.remove(&start);
        }
        self.0.insert(start, end);
        Ok(())
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;
    #[test]
    fn long_lived_server_reclaims_live_records_without_forgetting_replays() {
        let mut table = Associations::new(1, 2).unwrap();
        for serial in 1..=100_000 {
            let window = table.create_window(10).unwrap();
            table.window_serial(window, serial).unwrap();
            table.committed_surface(1, serial, serial).unwrap();
            table.destroy_surface(1, serial).unwrap();
            table.destroy_window(window);
        }
        assert!(table.serial_windows.is_empty());
        assert!(table.serial_surfaces.is_empty());
        assert!(table.surface_serials.is_empty());
        assert_eq!(table.used_serials.0.len(), 1);
        assert_eq!(table.used_x_serials.0.len(), 1);
        assert_eq!(table.dead_surfaces.0.len(), 1);
        let window = table.create_window(10).unwrap();
        assert!(table.window_serial(window, 1).is_err());
        assert!(table.committed_surface(1, 100_001, 1).is_err());
    }
    #[test]
    fn history_merges_out_of_order_ranges_without_accepting_gaps() {
        let mut history = History::default();
        for value in [8, 3, 5, 4, 7, 6] {
            history.insert(value).unwrap();
        }
        assert_eq!(history.0.len(), 1);
        for value in 3..=8 {
            assert!(history.contains(&value));
        }
        assert!(!history.contains(&2));
        assert!(!history.contains(&9));
        history.insert(u64::MAX).unwrap();
        history.insert(u64::MAX - 1).unwrap();
        assert!(history.contains(&u64::MAX));
    }
}

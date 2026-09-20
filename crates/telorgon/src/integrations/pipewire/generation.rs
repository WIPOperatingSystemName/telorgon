//! Loop-local buffer identities. Addresses are opaque keys and are never dereferenced here.
use super::CaptureLayout;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Generation {
    pub serial: u64,
    pub layout: CaptureLayout,
}

pub(super) struct Generations {
    pub target: Generation,
    negotiated: Option<Generation>,
    buffers: BTreeMap<usize, Generation>,
}
impl Generations {
    pub fn new(layout: CaptureLayout) -> Self {
        Self {
            target: Generation { serial: 1, layout },
            negotiated: None,
            buffers: BTreeMap::new(),
        }
    }
    pub fn resize(&mut self, target: Generation) -> bool {
        if self.target.serial.checked_add(1) != Some(target.serial) || !self.old_buffers_retired() {
            return false;
        }
        self.target = target;
        true
    }
    pub fn format(&mut self, accepted: bool) {
        self.negotiated = accepted.then_some(self.target);
    }
    pub fn add(&mut self, key: usize) -> bool {
        let Some(generation) = self.negotiated else {
            return false;
        };
        if key == 0
            || self.buffers.contains_key(&key)
            || self
                .buffers
                .values()
                .filter(|g| g.serial == generation.serial)
                .count()
                >= 3
        {
            return false;
        }
        self.buffers.insert(key, generation);
        true
    }
    pub fn remove(&mut self, key: usize) -> bool {
        self.buffers.remove(&key).is_some()
    }
    pub fn old_buffers_retired(&self) -> bool {
        self.buffers
            .values()
            .all(|g| g.serial == self.target.serial)
    }
    pub fn ready(&self) -> bool {
        self.negotiated == Some(self.target) && self.old_buffers_retired()
    }
    pub fn accepts(&self, key: usize) -> bool {
        self.ready() && self.buffers.get(&key) == Some(&self.target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroU32;
    fn layout(width: u32, height: u32) -> CaptureLayout {
        CaptureLayout::rgba8(
            NonZeroU32::new(width).unwrap(),
            NonZeroU32::new(height).unwrap(),
            width * 4,
        )
        .unwrap()
    }
    #[test]
    fn format_ack_does_not_retire_consumer_buffers_or_accept_old_equal_sized_frames() {
        let mut state = Generations::new(layout(4, 2));
        assert!(!state.add(1));
        state.format(true);
        assert!(state.add(1));
        assert!(state.add(2));
        assert!(state.accepts(1));
        // Same byte count is deliberately insufficient to identify a generation.
        assert!(state.resize(Generation {
            serial: 2,
            layout: layout(2, 4)
        }));
        assert!(!state.accepts(1));
        state.format(true);
        assert!(state.add(3));
        assert!(!state.ready());
        assert!(!state.accepts(3));
        assert!(state.remove(1));
        assert!(!state.ready());
        assert!(state.remove(2));
        assert!(state.ready());
        assert!(state.accepts(3));
        assert!(!state.accepts(1));
        assert!(!state.remove(2));
    }
    #[test]
    fn buffer_and_generation_limits_reject_duplicates_and_stale_events() {
        let mut state = Generations::new(layout(4, 2));
        state.format(true);
        assert!(!state.add(0));
        for key in 1..=3 {
            assert!(state.add(key));
        }
        assert!(!state.add(3));
        assert!(!state.add(4));
        let next = Generation {
            serial: 2,
            layout: layout(8, 4),
        };
        assert!(state.resize(next));
        assert!(!state.resize(next));
        assert!(!state.resize(Generation {
            serial: 3,
            layout: next.layout
        }));
        state.format(false);
        assert!(!state.add(4));
        assert!(!state.ready());
        for key in 1..=3 {
            assert!(state.remove(key));
        }
        assert!(!state.ready());
        state.format(true);
        assert!(state.ready());
    }
}

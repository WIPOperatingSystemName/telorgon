//! Mapped-window identities for ext-foreign-toplevel-list. No client or GPU ownership.
#![allow(dead_code)] // Native discovery dispatch is connected separately.
use crate::shell::WindowId;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Toplevel {
    pub epoch: u64,
    pub title: String,
    pub app_id: String,
}
impl Toplevel {
    pub fn identifier(&self) -> String {
        format!("telorgon-{:016x}", self.epoch)
    }
}

#[derive(Default)]
pub(super) struct ToplevelCatalog {
    next_epoch: u64,
    pub mapped: BTreeMap<WindowId, Toplevel>,
}
impl ToplevelCatalog {
    pub fn publish(&mut self, window: WindowId, title: &str, app_id: &str) -> Result<bool, ()> {
        if let Some(existing) = self.mapped.get_mut(&window) {
            let updated = Toplevel {
                epoch: existing.epoch,
                title: bounded_text(title),
                app_id: bounded_text(app_id),
            };
            let changed = *existing != updated;
            *existing = updated;
            return Ok(changed);
        }
        if self.mapped.len() >= 1024 {
            return Err(());
        }
        let epoch = self.next_epoch.checked_add(1).ok_or(())?;
        self.next_epoch = epoch;
        self.mapped.insert(
            window,
            Toplevel {
                epoch,
                title: bounded_text(title),
                app_id: bounded_text(app_id),
            },
        );
        Ok(true)
    }
    pub fn unmap(&mut self, window: WindowId) -> Option<Toplevel> {
        self.mapped.remove(&window)
    }
}

/// Each bound list gets its own seen set. Destroying a handle must not clear it.
#[derive(Default)]
pub(super) struct ToplevelList {
    seen: BTreeSet<u64>,
    stopped: bool,
}
impl ToplevelList {
    pub fn announce(&mut self, toplevel: &Toplevel) -> bool {
        !self.stopped && self.seen.insert(toplevel.epoch)
    }
    /// Emit finished only on the first processed stop request.
    pub fn stop(&mut self) -> bool {
        !std::mem::replace(&mut self.stopped, true)
    }
    pub fn prune_unmapped(&mut self, catalog: &ToplevelCatalog) {
        let live: BTreeSet<_> = catalog.mapped.values().map(|top| top.epoch).collect();
        self.seen.retain(|epoch| live.contains(epoch));
    }
}

fn bounded_text(text: &str) -> String {
    let mut result = String::new();
    for ch in text.chars().filter(|ch| *ch != '\0') {
        if result.len() + ch.len_utf8() > 1024 {
            break;
        }
        result.push(ch);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn window() -> WindowId {
        WindowId::new(
            std::num::NonZeroU32::new(1).unwrap(),
            std::num::NonZeroU32::new(1).unwrap(),
        )
    }
    #[test]
    fn remap_gets_new_identity_but_metadata_update_does_not() {
        let mut catalog = ToplevelCatalog::default();
        catalog.publish(window(), "one", "app").unwrap();
        let first = catalog.mapped[&window()].clone();
        let mut a = ToplevelList::default();
        let mut b = ToplevelList::default();
        assert!(a.announce(&first));
        assert!(b.announce(&first));
        // The client destroys its handle; the live mapping is still remembered.
        assert!(!a.announce(&first));
        catalog.publish(window(), "two", "app").unwrap();
        assert_eq!(catalog.mapped[&window()].identifier(), first.identifier());
        catalog.unmap(window());
        a.prune_unmapped(&catalog);
        catalog.publish(window(), "one", "app").unwrap();
        let remapped = &catalog.mapped[&window()];
        assert_ne!(remapped.identifier(), first.identifier());
        assert!(a.announce(remapped));
        assert!(a.stop());
        assert!(!a.stop());
        assert!(!a.announce(remapped));
        assert!(remapped.identifier().len() <= 32);
    }
    #[test]
    fn metadata_is_utf8_bounded_and_epoch_exhaustion_fails_closed() {
        let mut catalog = ToplevelCatalog::default();
        catalog
            .publish(window(), &"é".repeat(1024), "a\0b")
            .unwrap();
        assert_eq!(catalog.mapped[&window()].title.len(), 1024);
        assert_eq!(catalog.mapped[&window()].app_id, "ab");
        catalog.unmap(window());
        catalog.next_epoch = u64::MAX;
        assert!(catalog.publish(window(), "", "").is_err());
        assert!(catalog.mapped.is_empty());
    }
}

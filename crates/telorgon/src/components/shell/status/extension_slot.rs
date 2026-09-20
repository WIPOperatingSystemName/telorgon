//! Restricted view over one approved host extension status entry.

use std::fmt;

use crate::shell::{StatusAction, StatusEntryId, StatusEntryKind, StatusIconId, StatusText};
use crate::ui::UiNodeId;

use super::{StatusAreaEntryRef, StatusAreaRef};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatusExtensionSlot {
    entry: StatusEntryId,
}

impl StatusExtensionSlot {
    pub const fn new(entry: StatusEntryId) -> Self {
        Self { entry }
    }

    pub const fn entry(self) -> StatusEntryId {
        self.entry
    }

    pub fn bind(
        self,
        area: &StatusAreaRef,
    ) -> Result<StatusExtensionSlotRef, StatusExtensionSlotError> {
        let entry = area
            .entry(self.entry)
            .ok_or(StatusExtensionSlotError::UnknownEntry { entry: self.entry })?;
        if entry.entry().kind() != StatusEntryKind::Extension {
            return Err(StatusExtensionSlotError::WrongKind {
                entry: self.entry,
                actual: entry.entry().kind(),
            });
        }
        Ok(StatusExtensionSlotRef {
            entry: entry.clone(),
        })
    }
}

#[derive(Clone, Debug)]
pub struct StatusExtensionSlotRef {
    entry: StatusAreaEntryRef,
}

impl StatusExtensionSlotRef {
    pub const fn node(&self) -> UiNodeId {
        self.entry.node()
    }

    pub const fn status_entry(&self) -> &StatusAreaEntryRef {
        &self.entry
    }

    pub const fn icon(&self) -> Option<StatusIconId> {
        self.entry.entry().icon()
    }

    pub fn presented_value(&self) -> Option<&StatusText> {
        self.entry.presented_value()
    }

    pub fn actions(&self) -> &[StatusAction] {
        self.entry.entry().actions()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusExtensionSlotError {
    UnknownEntry {
        entry: StatusEntryId,
    },
    WrongKind {
        entry: StatusEntryId,
        actual: StatusEntryKind,
    },
}

impl fmt::Display for StatusExtensionSlotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid status extension slot binding: {self:?}")
    }
}

impl std::error::Error for StatusExtensionSlotError {}

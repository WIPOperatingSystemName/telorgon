//! Typed clock view over a host-authored clock status entry.

use std::fmt;

use crate::shell::{StatusEntryId, StatusEntryKind, StatusText};
use crate::ui::UiNodeId;

use super::{StatusAreaEntryRef, StatusAreaRef};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatusClock {
    entry: StatusEntryId,
}

impl StatusClock {
    pub const fn new(entry: StatusEntryId) -> Self {
        Self { entry }
    }

    pub const fn entry(self) -> StatusEntryId {
        self.entry
    }

    pub fn bind(self, area: &StatusAreaRef) -> Result<StatusClockRef, StatusClockError> {
        let entry = area
            .entry(self.entry)
            .ok_or(StatusClockError::UnknownEntry { entry: self.entry })?;
        if entry.entry().kind() != StatusEntryKind::Clock {
            return Err(StatusClockError::WrongKind {
                entry: self.entry,
                actual: entry.entry().kind(),
            });
        }
        Ok(StatusClockRef {
            entry: entry.clone(),
        })
    }
}

#[derive(Clone, Debug)]
pub struct StatusClockRef {
    entry: StatusAreaEntryRef,
}

impl StatusClockRef {
    pub const fn node(&self) -> UiNodeId {
        self.entry.node()
    }

    pub const fn status_entry(&self) -> &StatusAreaEntryRef {
        &self.entry
    }

    /// Returns only the value that the status area was permitted to present.
    pub fn presented_time(&self) -> Option<&StatusText> {
        self.entry.presented_value()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusClockError {
    UnknownEntry {
        entry: StatusEntryId,
    },
    WrongKind {
        entry: StatusEntryId,
        actual: StatusEntryKind,
    },
}

impl fmt::Display for StatusClockError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid status clock binding: {self:?}")
    }
}

impl std::error::Error for StatusClockError {}

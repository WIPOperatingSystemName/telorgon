//! Typed media summary over a host-authored media status entry.

use std::fmt;

use crate::shell::{StatusAction, StatusEntryId, StatusEntryKind, StatusText};
use crate::ui::UiNodeId;

use super::{StatusAreaEntryRef, StatusAreaRef};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaStatus {
    entry: StatusEntryId,
}

impl MediaStatus {
    pub const fn new(entry: StatusEntryId) -> Self {
        Self { entry }
    }

    pub const fn entry(self) -> StatusEntryId {
        self.entry
    }

    pub fn bind(self, area: &StatusAreaRef) -> Result<MediaStatusRef, MediaStatusError> {
        let entry = area
            .entry(self.entry)
            .ok_or(MediaStatusError::UnknownEntry { entry: self.entry })?;
        if entry.entry().kind() != StatusEntryKind::Media {
            return Err(MediaStatusError::WrongKind {
                entry: self.entry,
                actual: entry.entry().kind(),
            });
        }
        Ok(MediaStatusRef {
            entry: entry.clone(),
        })
    }
}

#[derive(Clone, Debug)]
pub struct MediaStatusRef {
    entry: StatusAreaEntryRef,
}

impl MediaStatusRef {
    pub const fn node(&self) -> UiNodeId {
        self.entry.node()
    }

    pub const fn status_entry(&self) -> &StatusAreaEntryRef {
        &self.entry
    }

    pub fn presented_summary(&self) -> Option<&StatusText> {
        self.entry.presented_value()
    }

    pub fn actions(&self) -> &[StatusAction] {
        self.entry.entry().actions()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaStatusError {
    UnknownEntry {
        entry: StatusEntryId,
    },
    WrongKind {
        entry: StatusEntryId,
        actual: StatusEntryKind,
    },
}

impl fmt::Display for MediaStatusError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid media status binding: {self:?}")
    }
}

impl std::error::Error for MediaStatusError {}

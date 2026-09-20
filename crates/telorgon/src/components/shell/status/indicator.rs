//! Typed system indicator over a host-authored status entry.

use std::fmt;

use crate::shell::{StatusEntryId, StatusEntryKind, StatusSeverity, StatusText};
use crate::ui::UiNodeId;

use super::{StatusAreaEntryRef, StatusAreaRef};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatusIndicator {
    entry: StatusEntryId,
}

impl StatusIndicator {
    pub const fn new(entry: StatusEntryId) -> Self {
        Self { entry }
    }

    pub const fn entry(self) -> StatusEntryId {
        self.entry
    }

    pub fn bind(self, area: &StatusAreaRef) -> Result<StatusIndicatorRef, StatusIndicatorError> {
        let entry = area
            .entry(self.entry)
            .ok_or(StatusIndicatorError::UnknownEntry { entry: self.entry })?;
        if !is_indicator_kind(entry.entry().kind()) {
            return Err(StatusIndicatorError::WrongKind {
                entry: self.entry,
                actual: entry.entry().kind(),
            });
        }
        Ok(StatusIndicatorRef {
            entry: entry.clone(),
        })
    }
}

pub const fn is_indicator_kind(kind: StatusEntryKind) -> bool {
    matches!(
        kind,
        StatusEntryKind::Connectivity
            | StatusEntryKind::Power
            | StatusEntryKind::Audio
            | StatusEntryKind::Input
            | StatusEntryKind::Session
            | StatusEntryKind::Privacy
    )
}

#[derive(Clone, Debug)]
pub struct StatusIndicatorRef {
    entry: StatusAreaEntryRef,
}

impl StatusIndicatorRef {
    pub const fn node(&self) -> UiNodeId {
        self.entry.node()
    }

    pub const fn status_entry(&self) -> &StatusAreaEntryRef {
        &self.entry
    }

    pub const fn kind(&self) -> StatusEntryKind {
        self.entry.entry().kind()
    }

    pub const fn severity(&self) -> StatusSeverity {
        self.entry.entry().severity()
    }

    pub fn presented_value(&self) -> Option<&StatusText> {
        self.entry.presented_value()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusIndicatorError {
    UnknownEntry {
        entry: StatusEntryId,
    },
    WrongKind {
        entry: StatusEntryId,
        actual: StatusEntryKind,
    },
}

impl fmt::Display for StatusIndicatorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid status indicator binding: {self:?}")
    }
}

impl std::error::Error for StatusIndicatorError {}

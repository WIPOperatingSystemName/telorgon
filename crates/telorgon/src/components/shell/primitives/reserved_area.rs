//! Output-revision-bound reserved-area proposal construction.

use std::fmt;

use crate::shell::{
    OutputEdge, OutputId, OutputRequest, OutputRevision, ReservedAreaExtent, ReservedAreaId,
    ShellCapabilities, ShellGrantToken,
};

use crate::components::shell::primitives::{OutputViewRef, ShellRootRef};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReservedArea {
    id: ReservedAreaId,
    edge: OutputEdge,
    extent: ReservedAreaExtent,
}

impl ReservedArea {
    pub const fn new(id: ReservedAreaId, edge: OutputEdge, extent: ReservedAreaExtent) -> Self {
        Self { id, edge, extent }
    }

    pub const fn id(self) -> ReservedAreaId {
        self.id
    }

    pub const fn edge(self) -> OutputEdge {
        self.edge
    }

    pub const fn extent(self) -> ReservedAreaExtent {
        self.extent
    }

    pub fn bind(
        self,
        root: ShellRootRef,
        output: OutputViewRef,
    ) -> Result<ReservedAreaRef, ReservedAreaError> {
        if root.output() != output.output() {
            return Err(ReservedAreaError::OutputMismatch);
        }
        if !root.grant().permits(ShellCapabilities::RESERVE_OUTPUT_AREA) {
            return Err(ReservedAreaError::NotAuthorized);
        }
        Ok(ReservedAreaRef {
            id: self.id,
            edge: self.edge,
            extent: self.extent,
            grant: root.grant().token(),
            output: output.output(),
            revision: output.revision(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReservedAreaRef {
    id: ReservedAreaId,
    edge: OutputEdge,
    extent: ReservedAreaExtent,
    grant: ShellGrantToken,
    output: OutputId,
    revision: OutputRevision,
}

impl ReservedAreaRef {
    pub const fn id(self) -> ReservedAreaId {
        self.id
    }

    pub const fn grant(self) -> ShellGrantToken {
        self.grant
    }

    pub const fn output(self) -> OutputId {
        self.output
    }

    pub const fn revision(self) -> OutputRevision {
        self.revision
    }

    pub const fn propose(self) -> OutputRequest {
        OutputRequest::ProposeReservedArea {
            output: self.output,
            revision: self.revision,
            reservation: self.id,
            edge: self.edge,
            extent: self.extent,
        }
    }

    pub const fn release(self) -> OutputRequest {
        OutputRequest::ReleaseReservedArea {
            output: self.output,
            revision: self.revision,
            reservation: self.id,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReservedAreaError {
    OutputMismatch,
    NotAuthorized,
}

impl fmt::Display for ReservedAreaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::OutputMismatch => "reserved-area root and output view do not match",
            Self::NotAuthorized => "shell root cannot propose an output reservation",
        })
    }
}

impl std::error::Error for ReservedAreaError {}

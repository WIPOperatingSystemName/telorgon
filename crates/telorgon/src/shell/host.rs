//! Protocol-neutral snapshot and request transport boundary for a shell policy host.

use std::collections::HashSet;
use std::fmt;
use std::num::NonZeroU64;
use std::sync::Arc;

use crate::shell::{
    AccessibilityAttachmentId, ApplicationEntry, ApplicationId, ClientInputRequest,
    ClientSurfaceSnapshot, ImportedAccessibilityAttachment, NotificationId, NotificationSnapshot,
    OutputId, OutputRequest, OutputSnapshot, ShellCapabilityGrant, ShellGrantToken,
    ShellRequestResult, SurfaceId, SurfaceRequest, SystemRequest, SystemStatusSnapshot,
    WorkspaceId, WorkspaceRequest, WorkspaceSnapshot,
};

/// Monotonic revision of one complete shell-host publication.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ShellSnapshotRevision(NonZeroU64);

impl ShellSnapshotRevision {
    pub const INITIAL: Self = Self(NonZeroU64::MIN);

    pub const fn new(value: NonZeroU64) -> Self {
        Self(value)
    }

    pub const fn from_raw(value: u64) -> Option<Self> {
        match NonZeroU64::new(value) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// Caller-owned values validated into one atomic immutable shell publication.
#[derive(Clone, Debug, PartialEq)]
pub struct ShellSnapshotParts {
    pub grants: Vec<ShellCapabilityGrant>,
    pub outputs: Vec<OutputSnapshot>,
    pub surfaces: Vec<ClientSurfaceSnapshot>,
    pub workspaces: Vec<WorkspaceSnapshot>,
    pub applications: Vec<ApplicationEntry>,
    pub notifications: Vec<NotificationSnapshot>,
    pub system_status: SystemStatusSnapshot,
    pub accessibility: Vec<ImportedAccessibilityAttachment>,
}

/// Complete immutable shell truth published atomically by one policy host.
#[derive(Clone, Debug, PartialEq)]
pub struct ShellSnapshot {
    revision: ShellSnapshotRevision,
    grants: Arc<[ShellCapabilityGrant]>,
    outputs: Arc<[OutputSnapshot]>,
    surfaces: Arc<[ClientSurfaceSnapshot]>,
    workspaces: Arc<[WorkspaceSnapshot]>,
    applications: Arc<[ApplicationEntry]>,
    notifications: Arc<[NotificationSnapshot]>,
    system_status: SystemStatusSnapshot,
    accessibility: Arc<[ImportedAccessibilityAttachment]>,
}

impl ShellSnapshot {
    pub const MAX_GRANTS: usize = 256;
    pub const MAX_OUTPUTS: usize = 64;
    pub const MAX_SURFACES: usize = 8192;
    pub const MAX_WORKSPACES: usize = 256;
    pub const MAX_APPLICATIONS: usize = 4096;
    pub const MAX_NOTIFICATIONS: usize = 4096;
    pub const MAX_ACCESSIBILITY_ATTACHMENTS: usize = 8192;

    pub fn new(
        revision: ShellSnapshotRevision,
        parts: ShellSnapshotParts,
    ) -> Result<Self, ShellSnapshotError> {
        check_count(
            ShellCollectionKind::Grant,
            parts.grants.len(),
            Self::MAX_GRANTS,
        )?;
        check_count(
            ShellCollectionKind::Output,
            parts.outputs.len(),
            Self::MAX_OUTPUTS,
        )?;
        check_count(
            ShellCollectionKind::Surface,
            parts.surfaces.len(),
            Self::MAX_SURFACES,
        )?;
        check_count(
            ShellCollectionKind::Workspace,
            parts.workspaces.len(),
            Self::MAX_WORKSPACES,
        )?;
        check_count(
            ShellCollectionKind::Application,
            parts.applications.len(),
            Self::MAX_APPLICATIONS,
        )?;
        check_count(
            ShellCollectionKind::Notification,
            parts.notifications.len(),
            Self::MAX_NOTIFICATIONS,
        )?;
        check_count(
            ShellCollectionKind::AccessibilityAttachment,
            parts.accessibility.len(),
            Self::MAX_ACCESSIBILITY_ATTACHMENTS,
        )?;

        let output_ids = unique_ids(
            ShellCollectionKind::Output,
            parts.outputs.iter().map(|output| output.id()),
        )?;
        let surface_ids = unique_ids(
            ShellCollectionKind::Surface,
            parts.surfaces.iter().map(ClientSurfaceSnapshot::id),
        )?;
        unique_ids(
            ShellCollectionKind::Workspace,
            parts.workspaces.iter().map(WorkspaceSnapshot::id),
        )?;
        unique_ids(
            ShellCollectionKind::Application,
            parts.applications.iter().map(ApplicationEntry::id),
        )?;
        unique_ids(
            ShellCollectionKind::Notification,
            parts.notifications.iter().map(NotificationSnapshot::id),
        )?;
        unique_ids(
            ShellCollectionKind::AccessibilityAttachment,
            parts.accessibility.iter().map(|attachment| attachment.id()),
        )?;
        unique_ids(
            ShellCollectionKind::Grant,
            parts.grants.iter().map(|grant| grant.token()),
        )?;

        if let Some(grant) = parts
            .grants
            .iter()
            .find(|grant| !output_ids.contains(&grant.output()))
        {
            return Err(ShellSnapshotError::UnknownGrantOutput {
                grant: grant.token(),
                output: grant.output(),
            });
        }
        if let Some(surface) = parts.surfaces.iter().find(|surface| {
            surface
                .parent()
                .is_some_and(|parent| !surface_ids.contains(&parent))
        }) {
            return Err(ShellSnapshotError::UnknownSurfaceParent {
                surface: surface.id(),
                parent: surface.parent().expect("checked as present"),
            });
        }
        validate_parent_cycles(&parts.surfaces)?;

        for workspace in &parts.workspaces {
            for placement in workspace.surfaces() {
                if !surface_ids.contains(&placement.surface()) {
                    return Err(ShellSnapshotError::UnknownWorkspaceSurface {
                        workspace: workspace.id(),
                        surface: placement.surface(),
                    });
                }
                if !output_ids.contains(&placement.output()) {
                    return Err(ShellSnapshotError::UnknownWorkspaceOutput {
                        workspace: workspace.id(),
                        output: placement.output(),
                    });
                }
            }
        }
        if let Some(attachment) = parts
            .accessibility
            .iter()
            .find(|attachment| !surface_ids.contains(&attachment.surface()))
        {
            return Err(ShellSnapshotError::UnknownAccessibilitySurface {
                attachment: attachment.id(),
                surface: attachment.surface(),
            });
        }

        Ok(Self {
            revision,
            grants: parts.grants.into(),
            outputs: parts.outputs.into(),
            surfaces: parts.surfaces.into(),
            workspaces: parts.workspaces.into(),
            applications: parts.applications.into(),
            notifications: parts.notifications.into(),
            system_status: parts.system_status,
            accessibility: parts.accessibility.into(),
        })
    }

    pub const fn revision(&self) -> ShellSnapshotRevision {
        self.revision
    }

    pub fn grants(&self) -> &[ShellCapabilityGrant] {
        &self.grants
    }

    pub fn outputs(&self) -> &[OutputSnapshot] {
        &self.outputs
    }

    pub fn surfaces(&self) -> &[ClientSurfaceSnapshot] {
        &self.surfaces
    }

    pub fn workspaces(&self) -> &[WorkspaceSnapshot] {
        &self.workspaces
    }

    pub fn applications(&self) -> &[ApplicationEntry] {
        &self.applications
    }

    pub fn notifications(&self) -> &[NotificationSnapshot] {
        &self.notifications
    }

    pub const fn system_status(&self) -> &SystemStatusSnapshot {
        &self.system_status
    }

    pub fn accessibility(&self) -> &[ImportedAccessibilityAttachment] {
        &self.accessibility
    }

    pub fn grant(&self, token: ShellGrantToken) -> Option<ShellCapabilityGrant> {
        self.grants
            .iter()
            .copied()
            .find(|grant| grant.token() == token)
    }

    pub fn output(&self, id: OutputId) -> Option<OutputSnapshot> {
        self.outputs
            .iter()
            .copied()
            .find(|output| output.id() == id)
    }

    pub fn surface(&self, id: SurfaceId) -> Option<&ClientSurfaceSnapshot> {
        self.surfaces.iter().find(|surface| surface.id() == id)
    }

    pub fn workspace(&self, id: WorkspaceId) -> Option<&WorkspaceSnapshot> {
        self.workspaces
            .iter()
            .find(|workspace| workspace.id() == id)
    }

    pub fn application(&self, id: ApplicationId) -> Option<&ApplicationEntry> {
        self.applications
            .iter()
            .find(|application| application.id() == id)
    }

    pub fn notification(&self, id: NotificationId) -> Option<&NotificationSnapshot> {
        self.notifications
            .iter()
            .find(|notification| notification.id() == id)
    }
}

/// Executor-neutral policy-host transport. Implementations own validation and execution.
pub trait ShellHost {
    fn snapshot(&self) -> ShellSnapshot;

    fn request_client_input(&mut self, request: ClientInputRequest) -> ShellRequestResult;

    fn request_surface(&mut self, request: SurfaceRequest) -> ShellRequestResult;

    fn request_workspace(&mut self, request: WorkspaceRequest) -> ShellRequestResult;

    fn request_output(&mut self, request: OutputRequest) -> ShellRequestResult;

    fn request_system(&mut self, request: SystemRequest) -> ShellRequestResult;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShellCollectionKind {
    Grant,
    Output,
    Surface,
    Workspace,
    Application,
    Notification,
    AccessibilityAttachment,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellSnapshotError {
    TooManyItems {
        kind: ShellCollectionKind,
        count: usize,
        max: usize,
    },
    DuplicateIdentity {
        kind: ShellCollectionKind,
    },
    UnknownGrantOutput {
        grant: ShellGrantToken,
        output: OutputId,
    },
    UnknownSurfaceParent {
        surface: SurfaceId,
        parent: SurfaceId,
    },
    SurfaceParentCycle {
        surface: SurfaceId,
    },
    UnknownWorkspaceSurface {
        workspace: WorkspaceId,
        surface: SurfaceId,
    },
    UnknownWorkspaceOutput {
        workspace: WorkspaceId,
        output: OutputId,
    },
    UnknownAccessibilitySurface {
        attachment: AccessibilityAttachmentId,
        surface: SurfaceId,
    },
}

impl fmt::Display for ShellSnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyItems { kind, count, max } => {
                write!(
                    formatter,
                    "shell snapshot has {count} {kind:?} items; maximum is {max}"
                )
            }
            Self::DuplicateIdentity { kind } => {
                write!(
                    formatter,
                    "shell snapshot contains a duplicate {kind:?} identity"
                )
            }
            Self::UnknownGrantOutput { .. } => {
                formatter.write_str("shell capability grant cites an unknown output")
            }
            Self::UnknownSurfaceParent { .. } => {
                formatter.write_str("shell surface cites an unknown parent")
            }
            Self::SurfaceParentCycle { .. } => {
                formatter.write_str("shell surface parentage contains a cycle")
            }
            Self::UnknownWorkspaceSurface { .. } => {
                formatter.write_str("shell workspace cites an unknown surface")
            }
            Self::UnknownWorkspaceOutput { .. } => {
                formatter.write_str("shell workspace placement cites an unknown output")
            }
            Self::UnknownAccessibilitySurface { .. } => {
                formatter.write_str("shell accessibility attachment cites an unknown surface")
            }
        }
    }
}

impl std::error::Error for ShellSnapshotError {}

fn check_count(
    kind: ShellCollectionKind,
    count: usize,
    max: usize,
) -> Result<(), ShellSnapshotError> {
    if count > max {
        Err(ShellSnapshotError::TooManyItems { kind, count, max })
    } else {
        Ok(())
    }
}

fn unique_ids<T: Copy + Eq + std::hash::Hash>(
    kind: ShellCollectionKind,
    ids: impl Iterator<Item = T>,
) -> Result<HashSet<T>, ShellSnapshotError> {
    let mut seen = HashSet::new();
    for id in ids {
        if !seen.insert(id) {
            return Err(ShellSnapshotError::DuplicateIdentity { kind });
        }
    }
    Ok(seen)
}

fn validate_parent_cycles(surfaces: &[ClientSurfaceSnapshot]) -> Result<(), ShellSnapshotError> {
    for surface in surfaces {
        let mut seen = HashSet::new();
        let mut current = Some(surface.id());
        while let Some(id) = current {
            if !seen.insert(id) {
                return Err(ShellSnapshotError::SurfaceParentCycle {
                    surface: surface.id(),
                });
            }
            current = surfaces
                .iter()
                .find(|candidate| candidate.id() == id)
                .and_then(ClientSurfaceSnapshot::parent);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::shell::{
        AcceptedRequestId, InputSource, StatusActionId, StatusEntryId, SystemStatusRevision,
    };

    use super::*;

    fn empty_snapshot() -> ShellSnapshot {
        ShellSnapshot::new(
            ShellSnapshotRevision::INITIAL,
            ShellSnapshotParts {
                grants: Vec::new(),
                outputs: Vec::new(),
                surfaces: Vec::new(),
                workspaces: Vec::new(),
                applications: Vec::new(),
                notifications: Vec::new(),
                system_status: SystemStatusSnapshot::new(SystemStatusRevision::INITIAL, Vec::new())
                    .unwrap(),
                accessibility: Vec::new(),
            },
        )
        .unwrap()
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum TraceKind {
        ClientInput,
        Surface,
        Workspace,
        Output,
        System,
    }

    struct TraceHost {
        snapshot: ShellSnapshot,
        trace: Vec<TraceKind>,
    }

    impl TraceHost {
        fn admitted(&mut self, kind: TraceKind) -> ShellRequestResult {
            self.trace.push(kind);
            ShellRequestResult::accepted(
                AcceptedRequestId::from_raw(self.trace.len() as u64).unwrap(),
            )
        }
    }

    impl ShellHost for TraceHost {
        fn snapshot(&self) -> ShellSnapshot {
            self.snapshot.clone()
        }

        fn request_client_input(&mut self, _: ClientInputRequest) -> ShellRequestResult {
            self.admitted(TraceKind::ClientInput)
        }

        fn request_surface(&mut self, _: SurfaceRequest) -> ShellRequestResult {
            self.admitted(TraceKind::Surface)
        }

        fn request_workspace(&mut self, _: WorkspaceRequest) -> ShellRequestResult {
            self.admitted(TraceKind::Workspace)
        }

        fn request_output(&mut self, _: OutputRequest) -> ShellRequestResult {
            self.admitted(TraceKind::Output)
        }

        fn request_system(&mut self, _: SystemRequest) -> ShellRequestResult {
            self.admitted(TraceKind::System)
        }
    }

    #[test]
    fn empty_atomic_snapshot_and_typed_transport_are_deterministic() {
        let mut host = TraceHost {
            snapshot: empty_snapshot(),
            trace: Vec::new(),
        };
        let request = SystemRequest::StatusAction {
            revision: SystemStatusRevision::INITIAL,
            entry: StatusEntryId::from_raw(1).unwrap(),
            action: StatusActionId::from_raw(2).unwrap(),
            source: InputSource::Programmatic,
        };

        assert_eq!(host.snapshot().revision(), ShellSnapshotRevision::INITIAL);
        assert!(host.request_system(request).is_accepted());
        assert_eq!(host.trace, vec![TraceKind::System]);
    }

    #[test]
    fn duplicate_top_level_identities_are_rejected_atomically() {
        let status = SystemStatusSnapshot::new(SystemStatusRevision::INITIAL, Vec::new()).unwrap();
        let output = OutputSnapshot::new(
            OutputId::from_raw(1).unwrap(),
            crate::shell::OutputRevision::INITIAL,
            crate::shell::OutputGeometry::new(
                crate::foundation::RectF {
                    x: 0.0,
                    y: 0.0,
                    width: 100.0,
                    height: 100.0,
                },
                crate::foundation::RectF {
                    x: 0.0,
                    y: 0.0,
                    width: 100.0,
                    height: 100.0,
                },
                crate::foundation::SizeI {
                    width: 100,
                    height: 100,
                },
                1.0,
                crate::shell::OutputTransform::Normal,
                crate::foundation::EdgeInsets::ZERO,
                crate::shell::OutputColorCapabilities::SRGB,
            )
            .unwrap(),
        );
        let result = ShellSnapshot::new(
            ShellSnapshotRevision::INITIAL,
            ShellSnapshotParts {
                grants: Vec::new(),
                outputs: vec![output, output],
                surfaces: Vec::new(),
                workspaces: Vec::new(),
                applications: Vec::new(),
                notifications: Vec::new(),
                system_status: status,
                accessibility: Vec::new(),
            },
        );

        assert!(matches!(
            result,
            Err(ShellSnapshotError::DuplicateIdentity {
                kind: ShellCollectionKind::Output
            })
        ));
    }
}

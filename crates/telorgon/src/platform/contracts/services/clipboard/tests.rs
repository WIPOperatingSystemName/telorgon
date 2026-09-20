use std::num::NonZeroU64;

use crate::platform::contracts::{
    DataOfferId, ExecutionRequirement, PlatformErrorKind, RequestId, RequestOutcome,
    ServiceLookup, ServiceRegistry,
};

use super::super::data_transfer::{SizeHint, TrustLevel};
use super::*;

fn offer(source: DataSourceKind) -> DataOfferDescriptor {
    DataOfferDescriptor::new(
        DataOfferId::from_raw(7, 2).unwrap(),
        vec![DataFormat::mime("text/plain;charset=utf-8").unwrap()],
        source,
        TrustLevel::Trusted,
        vec![SizeHint::AtMost(32)],
    )
    .unwrap()
}

fn capabilities() -> ClipboardCapabilities {
    let format = DataFormat::mime("text/plain;charset=utf-8").unwrap();
    let system = ClipboardCapability::new(
        ClipboardOperations::new(true, true, true, true),
        vec![format.clone(), format],
        ClipboardLimits::default(),
        PermissionState::Granted,
        PermissionState::PromptRequired,
        ExecutionRequirement::HostEventLoop,
        UserGestureRequirement::RecentRequired,
    )
    .unwrap();
    ClipboardCapabilities::new(
        Support::Available(system),
        Support::Unavailable(UnavailableReason::UnsupportedByPlatform),
    )
}

#[test]
fn snapshot_and_change_keep_clipboard_identity_and_monotonic_revision() {
    let first_id = ClipboardSnapshotId::new(ClipboardKind::System, ClipboardRevision::INITIAL);
    let first =
        ClipboardSnapshot::new(first_id, Some(offer(DataSourceKind::Clipboard))).unwrap();
    assert_eq!(first.clipboard(), ClipboardKind::System);
    assert_eq!(first.current_offer().unwrap().formats().len(), 1);

    let second_id = ClipboardSnapshotId::new(
        ClipboardKind::System,
        ClipboardRevision::from_raw(2).unwrap(),
    );
    let changed = ClipboardChange::new(
        Some(first_id),
        ClipboardSnapshot::new(second_id, None).unwrap(),
    )
    .unwrap();
    assert_eq!(changed.previous(), Some(first_id));
    assert!(changed.current().current_offer().is_none());

    let stale = ClipboardSnapshot::new(first_id, None).unwrap();
    assert!(matches!(
        ClipboardChange::new(Some(first_id), stale),
        Err(ClipboardSnapshotError::RevisionDidNotAdvance { .. })
    ));
    assert!(matches!(
        ClipboardChange::new(
            Some(first_id),
            ClipboardSnapshot::new(
                ClipboardSnapshotId::new(ClipboardKind::Selection, ClipboardRevision::INITIAL),
                None,
            )
            .unwrap(),
        ),
        Err(ClipboardSnapshotError::ClipboardMismatch { .. })
    ));
}

#[test]
fn snapshot_and_request_reject_non_clipboard_offers_and_cross_kind_expectations() {
    let drag = offer(DataSourceKind::DragAndDrop);
    assert!(matches!(
        ClipboardSnapshot::new(
            ClipboardSnapshotId::new(ClipboardKind::System, ClipboardRevision::INITIAL),
            Some(drag.clone()),
        ),
        Err(ClipboardSnapshotError::OfferSourceIsNotClipboard { .. })
    ));
    assert!(matches!(
        ClipboardPublishRequest::new(ClipboardKind::System, drag, None),
        Err(ClipboardRequestError::OfferSourceIsNotClipboard { .. })
    ));

    let selection = ClipboardSnapshotId::new(
        ClipboardKind::Selection,
        ClipboardRevision::from_raw(9).unwrap(),
    );
    assert!(matches!(
        ClipboardClearRequest::new(ClipboardKind::System, Some(selection)),
        Err(ClipboardRequestError::ExpectedSnapshotClipboardMismatch { .. })
    ));
}

#[test]
fn capability_keeps_permissions_targets_formats_and_bounds_explicit() {
    let capabilities = capabilities();
    let Support::Available(system) = capabilities.for_clipboard(ClipboardKind::System) else {
        panic!("system clipboard must be available")
    };
    assert!(system.operations().supports_snapshot());
    assert!(system.operations().supports_change_notifications());
    assert_eq!(system.formats().len(), 1);
    assert_eq!(system.read_permission(), PermissionState::Granted);
    assert_eq!(system.write_permission(), PermissionState::PromptRequired);
    assert_eq!(system.execution(), ExecutionRequirement::HostEventLoop);
    assert!(system.user_gesture().is_required());
    assert_eq!(
        capabilities
            .for_clipboard(ClipboardKind::Selection)
            .unavailable_reason(),
        Some(UnavailableReason::UnsupportedByPlatform)
    );

    let formats = (0..=MAX_CLIPBOARD_CAPABILITY_FORMATS)
        .map(|index| DataFormat::mime(&format!("application/x-telorgon-{index}")).unwrap())
        .collect();
    assert!(matches!(
        ClipboardCapability::new(
            ClipboardOperations::new(false, true, false, false),
            formats,
            ClipboardLimits::default(),
            PermissionState::Unknown,
            PermissionState::Unknown,
            ExecutionRequirement::HostExecutor,
            UserGestureRequirement::NotRequired,
        ),
        Err(ClipboardCapabilityError::TooManyFormats { .. })
    ));
    assert_eq!(
        ClipboardLimits::new(
            CapabilityLimit::Bounded(
                NonZeroU32::new((MAX_DATA_FORMATS_PER_OFFER + 1) as u32).unwrap(),
            ),
            CapabilityLimit::Unspecified,
        ),
        Err(ClipboardLimitError::TooManyFormats)
    );
    assert_eq!(
        ClipboardLimits::new(
            CapabilityLimit::Unspecified,
            CapabilityLimit::Bounded(NonZeroU64::new(MAX_DATA_READ_BYTES + 1).unwrap()),
        ),
        Err(ClipboardLimitError::BytesPerFormatTooLarge)
    );
}

struct FixtureClipboard {
    capabilities: ClipboardCapabilities,
    snapshot: ClipboardSnapshot,
}

impl ClipboardService for FixtureClipboard {
    fn capability(&self, clipboard: ClipboardKind) -> Support<ClipboardCapability> {
        self.capabilities.for_clipboard(clipboard).map(Clone::clone)
    }

    fn current_snapshot(&self, clipboard: ClipboardKind) -> ClipboardSnapshotStatus {
        if clipboard == ClipboardKind::System {
            ClipboardSnapshotStatus::Current(self.snapshot.clone())
        } else {
            ClipboardSnapshotStatus::Unavailable(UnavailableReason::UnsupportedByPlatform)
        }
    }

    fn publish(
        &self,
        request: ClipboardPublishRequest,
    ) -> ClipboardRequestAdmission<ClipboardPublishApplied> {
        if request.clipboard() == ClipboardKind::Selection {
            return Err(ClipboardAdmissionError::ClipboardUnavailable {
                clipboard: ClipboardKind::Selection,
            });
        }
        Ok(crate::platform::contracts::AdmittedRequest::new(
            RequestId::from_raw(44).unwrap(),
        ))
    }

    fn clear(
        &self,
        request: ClipboardClearRequest,
    ) -> ClipboardRequestAdmission<ClipboardClearApplied> {
        if request.clipboard() == ClipboardKind::Selection {
            return Err(ClipboardAdmissionError::ClipboardUnavailable {
                clipboard: ClipboardKind::Selection,
            });
        }
        Ok(crate::platform::contracts::AdmittedRequest::new(
            RequestId::from_raw(45).unwrap(),
        ))
    }
}

#[test]
fn registry_handle_is_object_safe_and_admission_stays_separate_from_completion() {
    let snapshot = ClipboardSnapshot::new(
        ClipboardSnapshotId::new(ClipboardKind::System, ClipboardRevision::INITIAL),
        Some(offer(DataSourceKind::Clipboard)),
    )
    .unwrap();
    let service: Rc<dyn ClipboardService> = Rc::new(FixtureClipboard {
        capabilities: capabilities(),
        snapshot,
    });
    let mut registry = ServiceRegistry::new();
    assert!(
        registry
            .register::<ClipboardServiceKey>(service)
            .is_registered()
    );

    let ServiceLookup::Available(service) = registry.lookup::<ClipboardServiceKey>() else {
        panic!("registered clipboard must be available")
    };
    assert!(
        service
            .current_snapshot(ClipboardKind::System)
            .current()
            .is_some()
    );
    assert!(matches!(
        service.capability(ClipboardKind::System),
        Support::Available(_)
    ));
    let request = ClipboardClearRequest::new(ClipboardKind::System, None).unwrap();
    let applied = ClipboardClearApplied::from_request(request);
    let admitted = service.clear(request).unwrap();
    assert_eq!(admitted.request_id(), RequestId::from_raw(45).unwrap());
    let completion = admitted.complete(RequestOutcome::Applied(applied));
    assert!(completion.outcome().is_applied());
}

#[test]
fn status_distinguishes_absence_and_failure_and_debug_has_no_content_payload() {
    let unavailable =
        ClipboardSnapshotStatus::Unavailable(UnavailableReason::ExecutionContextUnavailable);
    assert_eq!(
        unavailable.unavailable_reason(),
        Some(UnavailableReason::ExecutionContextUnavailable)
    );

    let error = PlatformError::new(
        PlatformErrorKind::TransportFailure,
        "clipboard snapshot observation",
    );
    let failed = ClipboardSnapshotStatus::Failed(error);
    assert_eq!(failed.failure(), Some(error));

    let request = ClipboardPublishRequest::new(
        ClipboardKind::System,
        offer(DataSourceKind::Clipboard),
        None,
    )
    .unwrap();
    let debug = format!("{request:?}");
    assert!(debug.contains("format_count: 1"));
    assert!(!debug.contains("secret clipboard contents"));
    assert_eq!(
        NonZeroU64::new(1).map(ClipboardRevision::new),
        Some(ClipboardRevision::INITIAL)
    );
}

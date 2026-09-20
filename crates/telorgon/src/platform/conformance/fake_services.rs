//! Deterministic owner-local fake service adapters.

use std::cell::{Cell, Ref, RefCell, RefMut};
use std::error::Error;
use std::fmt;
use std::num::NonZeroU16;

use crate::platform::contracts::{
    AdmittedRequest, HapticAdmission, HapticAdmissionError, HapticCapability, HapticEffect,
    HapticIntensity, HapticRequest, HapticsService, RequestId, RestorationAdmissionError,
    RestorationCapability, RestorationCapabilityQuery, RestorationClearAdmission,
    RestorationClearApplied, RestorationClearRequest, RestorationConsumptionAdmission,
    RestorationConsumptionApplied, RestorationConsumptionRequest, RestorationPublicationAdmission,
    RestorationPublicationApplied, RestorationPublicationRequest, RestorationRecord,
    RestorationScope, RestorationService, RestorationSnapshotId, Support,
};

use crate::platform::conformance::{BoundedCapture, CaptureLimitError};

/// Hard bound on independent restoration histories observed by one fake adapter.
pub const MAX_FAKE_RESTORATION_SCOPES: usize = 64;

/// Payload-free haptic invocation retained by the fake adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FakeHapticInvocation {
    request_id: RequestId,
    effect: HapticEffect,
    intensity: HapticIntensity,
    had_user_gesture: bool,
}

impl FakeHapticInvocation {
    pub const fn request_id(self) -> RequestId {
        self.request_id
    }

    pub const fn effect(self) -> HapticEffect {
        self.effect
    }

    pub const fn intensity(self) -> HapticIntensity {
        self.intensity
    }

    pub const fn had_user_gesture(self) -> bool {
        self.had_user_gesture
    }
}

/// Deterministic fake implementation of the semantic haptics admission contract.
pub struct FakeHapticsService {
    capability: HapticCapability,
    last_request_id: Cell<u64>,
    invocations: RefCell<BoundedCapture<FakeHapticInvocation>>,
}

impl FakeHapticsService {
    pub fn new(
        capability: HapticCapability,
        maximum_invocations: NonZeroU16,
    ) -> Result<Self, CaptureLimitError> {
        Ok(Self {
            capability,
            last_request_id: Cell::new(0),
            invocations: RefCell::new(BoundedCapture::new(maximum_invocations)?),
        })
    }

    pub const fn capability_snapshot(&self) -> HapticCapability {
        self.capability
    }

    pub fn invocations(&self) -> Ref<'_, BoundedCapture<FakeHapticInvocation>> {
        self.invocations.borrow()
    }

    pub fn invocations_mut(&self) -> RefMut<'_, BoundedCapture<FakeHapticInvocation>> {
        self.invocations.borrow_mut()
    }
}

impl HapticsService for FakeHapticsService {
    fn capability(&self) -> Support<HapticCapability> {
        Support::Available(self.capability)
    }

    fn trigger(&self, request: HapticRequest) -> HapticAdmission {
        if !self.capability.operations().supports_trigger() {
            return Err(HapticAdmissionError::UnsupportedOperation);
        }
        let device = self.capability.device();
        if !device.is_available() {
            return Err(HapticAdmissionError::DeviceUnavailable);
        }
        if !device.supports(request.effect()) {
            return Err(HapticAdmissionError::EffectUnsupported {
                effect: request.effect(),
            });
        }
        match self.capability.user_setting() {
            crate::platform::contracts::HapticUserSettingState::Enabled => {}
            crate::platform::contracts::HapticUserSettingState::Disabled => {
                return Err(HapticAdmissionError::UserSettingDisabled);
            }
            crate::platform::contracts::HapticUserSettingState::Unknown => {
                return Err(HapticAdmissionError::UserSettingUnknown);
            }
        }
        if self.capability.permission().blocks_use() {
            return Err(HapticAdmissionError::PermissionDenied);
        }
        if self.capability.permission().requires_prompt() {
            return Err(HapticAdmissionError::AuthorizationRequired);
        }
        if self.capability.user_gesture().is_required() && !request.has_user_gesture() {
            return Err(HapticAdmissionError::UserGestureRequired);
        }
        if !device.supports_intensity_control() && request.intensity() != HapticIntensity::FULL {
            return Err(HapticAdmissionError::IntensityControlUnsupported);
        }
        let maximum = self.capability.limits().maximum_intensity();
        if request.intensity() > maximum {
            return Err(HapticAdmissionError::IntensityExceedsCapability {
                requested: request.intensity(),
                maximum,
            });
        }
        if self.invocations.borrow().is_full() {
            return Err(HapticAdmissionError::CapacityExceeded);
        }
        let next = self
            .last_request_id
            .get()
            .checked_add(1)
            .ok_or(HapticAdmissionError::CapacityExceeded)?;
        let request_id = RequestId::from_raw(next).ok_or(HapticAdmissionError::CapacityExceeded)?;
        let invocation = FakeHapticInvocation {
            request_id,
            effect: request.effect(),
            intensity: request.intensity(),
            had_user_gesture: request.has_user_gesture(),
        };
        let _owned_gesture = request.into_parts().2;
        self.invocations
            .borrow_mut()
            .push(invocation)
            .expect("fake haptics capacity was checked before request identity advanced");
        self.last_request_id.set(next);
        Ok(AdmittedRequest::new(request_id))
    }
}

/// Restoration operation retained in a payload-free fake-adapter trace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FakeRestorationOperation {
    Publish,
    Update,
    Consume,
    Clear,
}

/// Opaque-content-free restoration invocation retained by the fake adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FakeRestorationInvocation {
    request_id: RequestId,
    operation: FakeRestorationOperation,
    snapshot: RestorationSnapshotId,
    previous: Option<RestorationSnapshotId>,
    token_byte_len: Option<usize>,
}

impl FakeRestorationInvocation {
    pub const fn request_id(self) -> RequestId {
        self.request_id
    }

    pub const fn operation(self) -> FakeRestorationOperation {
        self.operation
    }

    pub const fn snapshot(self) -> RestorationSnapshotId {
        self.snapshot
    }

    pub const fn previous(self) -> Option<RestorationSnapshotId> {
        self.previous
    }

    pub const fn token_byte_len(self) -> Option<usize> {
        self.token_byte_len
    }
}

/// Invalid explicit fake-restoration state transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FakeRestorationError {
    CaptureLimit(CaptureLimitError),
    ScopeCapacityReached {
        maximum: usize,
    },
    RevisionRegressed {
        previous: RestorationSnapshotId,
        observed: RestorationSnapshotId,
    },
    SnapshotMismatch {
        expected: RestorationSnapshotId,
        observed: Option<RestorationSnapshotId>,
    },
}

impl fmt::Display for FakeRestorationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::CaptureLimit(_) => "fake restoration capture limit is invalid",
            Self::ScopeCapacityReached { .. } => "fake restoration scope capacity was reached",
            Self::RevisionRegressed { .. } => "fake restoration observation regressed",
            Self::SnapshotMismatch { .. } => "fake restoration snapshot does not match current",
        })
    }
}

impl Error for FakeRestorationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::CaptureLimit(error) => Some(error),
            _ => None,
        }
    }
}

impl From<CaptureLimitError> for FakeRestorationError {
    fn from(error: CaptureLimitError) -> Self {
        Self::CaptureLimit(error)
    }
}

/// Deterministic fake implementation of revisioned restoration-token admission.
///
/// Explicit `observe_*` methods model later host truth; admission alone never mutates the retained
/// current snapshot. Admitted consumption records stay single-owner until retrieved by request ID
/// and placed in one terminal completion.
pub struct FakeRestorationService {
    capability: RestorationCapability,
    last_request_id: Cell<u64>,
    current: RefCell<Vec<RestorationSnapshotId>>,
    invocations: RefCell<BoundedCapture<FakeRestorationInvocation>>,
    pending_consumptions: RefCell<Vec<(RequestId, RestorationRecord)>>,
}

impl FakeRestorationService {
    pub fn new(
        capability: RestorationCapability,
        maximum_invocations: NonZeroU16,
    ) -> Result<Self, FakeRestorationError> {
        Ok(Self {
            capability,
            last_request_id: Cell::new(0),
            current: RefCell::new(Vec::new()),
            invocations: RefCell::new(BoundedCapture::new(maximum_invocations)?),
            pending_consumptions: RefCell::new(Vec::new()),
        })
    }

    pub const fn capability_snapshot(&self) -> RestorationCapability {
        self.capability
    }

    pub fn current(&self, scope: RestorationScope) -> Option<RestorationSnapshotId> {
        self.current
            .borrow()
            .iter()
            .copied()
            .find(|snapshot| snapshot.scope() == scope)
    }

    pub fn observe_snapshot(
        &self,
        observed: RestorationSnapshotId,
    ) -> Result<(), FakeRestorationError> {
        let mut current = self.current.borrow_mut();
        if let Some(existing) = current
            .iter_mut()
            .find(|snapshot| snapshot.scope() == observed.scope())
        {
            if observed.revision() < existing.revision() {
                return Err(FakeRestorationError::RevisionRegressed {
                    previous: *existing,
                    observed,
                });
            }
            *existing = observed;
            return Ok(());
        }
        if current.len() == MAX_FAKE_RESTORATION_SCOPES {
            return Err(FakeRestorationError::ScopeCapacityReached {
                maximum: MAX_FAKE_RESTORATION_SCOPES,
            });
        }
        current.push(observed);
        Ok(())
    }

    pub fn observe_clear(
        &self,
        expected: RestorationSnapshotId,
    ) -> Result<(), FakeRestorationError> {
        let mut current = self.current.borrow_mut();
        let observed = current
            .iter()
            .copied()
            .find(|snapshot| snapshot.scope() == expected.scope());
        if observed != Some(expected) {
            return Err(FakeRestorationError::SnapshotMismatch { expected, observed });
        }
        let index = current
            .iter()
            .position(|snapshot| *snapshot == expected)
            .expect("matching snapshot position was established above");
        current.remove(index);
        Ok(())
    }

    pub fn invocations(&self) -> Ref<'_, BoundedCapture<FakeRestorationInvocation>> {
        self.invocations.borrow()
    }

    pub fn invocations_mut(&self) -> RefMut<'_, BoundedCapture<FakeRestorationInvocation>> {
        self.invocations.borrow_mut()
    }

    pub fn take_pending_consumption(&self, request_id: RequestId) -> Option<RestorationRecord> {
        let mut pending = self.pending_consumptions.borrow_mut();
        let index = pending
            .iter()
            .position(|(candidate, _)| *candidate == request_id)?;
        Some(pending.remove(index).1)
    }

    fn validate_scope(&self, scope: RestorationScope) -> Result<(), RestorationAdmissionError> {
        if self.capability.operations().supports_scope(scope) {
            Ok(())
        } else {
            Err(RestorationAdmissionError::UnsupportedScope { scope })
        }
    }

    fn validate_common(&self) -> Result<(), RestorationAdmissionError> {
        if self.capability.permission().blocks_use() {
            return Err(RestorationAdmissionError::PermissionDenied);
        }
        if self.capability.permission().requires_prompt() {
            return Err(RestorationAdmissionError::AuthorizationRequired);
        }
        if self.invocations.borrow().is_full() {
            return Err(RestorationAdmissionError::CapacityExceeded);
        }
        Ok(())
    }

    fn validate_record(&self, record: &RestorationRecord) -> Result<(), RestorationAdmissionError> {
        self.validate_scope(record.scope())?;
        if record.token().byte_len() > self.capability.limits().maximum_token_bytes().get() as usize
        {
            return Err(RestorationAdmissionError::TokenExceedsCapability);
        }
        self.validate_common()
    }

    fn next_request_id(&self) -> Result<(u64, RequestId), RestorationAdmissionError> {
        let next = self
            .last_request_id
            .get()
            .checked_add(1)
            .ok_or(RestorationAdmissionError::CapacityExceeded)?;
        let id = RequestId::from_raw(next).ok_or(RestorationAdmissionError::CapacityExceeded)?;
        Ok((next, id))
    }

    fn capture(
        &self,
        next: u64,
        invocation: FakeRestorationInvocation,
    ) -> Result<(), RestorationAdmissionError> {
        self.invocations
            .borrow_mut()
            .push(invocation)
            .map_err(|_| RestorationAdmissionError::CapacityExceeded)?;
        self.last_request_id.set(next);
        Ok(())
    }
}

impl RestorationService for FakeRestorationService {
    fn capability(&self, query: RestorationCapabilityQuery) -> Support<RestorationCapability> {
        if self.capability.operations().supports_scope(query.scope()) {
            Support::Available(self.capability)
        } else {
            Support::Unavailable(crate::platform::contracts::UnavailableReason::UnavailableInScope)
        }
    }

    fn publish(&self, request: RestorationPublicationRequest) -> RestorationPublicationAdmission {
        let operation = if request.previous().is_some() {
            if !self.capability.operations().supports_update() {
                return Err(RestorationAdmissionError::UnsupportedOperation);
            }
            FakeRestorationOperation::Update
        } else {
            if !self.capability.operations().supports_publish() {
                return Err(RestorationAdmissionError::UnsupportedOperation);
            }
            FakeRestorationOperation::Publish
        };
        self.validate_record(request.record())?;
        let observed = self.current(request.snapshot().scope());
        if request.previous() != observed {
            return Err(RestorationAdmissionError::RevisionMismatch {
                expected: request.previous().unwrap_or(request.snapshot()),
                observed,
            });
        }
        let (next, request_id) = self.next_request_id()?;
        let invocation = FakeRestorationInvocation {
            request_id,
            operation,
            snapshot: request.snapshot(),
            previous: request.previous(),
            token_byte_len: Some(request.record().token().byte_len()),
        };
        self.capture(next, invocation)?;
        Ok(AdmittedRequest::<RestorationPublicationApplied>::new(
            request_id,
        ))
    }

    fn consume(&self, request: RestorationConsumptionRequest) -> RestorationConsumptionAdmission {
        if !self.capability.operations().supports_consume() {
            return Err(RestorationAdmissionError::UnsupportedOperation);
        }
        self.validate_record(request.record())?;
        let observed = self.current(request.snapshot().scope());
        if observed != Some(request.snapshot()) {
            return Err(RestorationAdmissionError::RevisionMismatch {
                expected: request.snapshot(),
                observed,
            });
        }
        if self.pending_consumptions.borrow().len()
            == self.invocations.borrow().capacity().get() as usize
        {
            return Err(RestorationAdmissionError::CapacityExceeded);
        }
        let (next, request_id) = self.next_request_id()?;
        let invocation = FakeRestorationInvocation {
            request_id,
            operation: FakeRestorationOperation::Consume,
            snapshot: request.snapshot(),
            previous: None,
            token_byte_len: Some(request.record().token().byte_len()),
        };
        self.capture(next, invocation)?;
        self.pending_consumptions
            .borrow_mut()
            .push((request_id, request.into_record()));
        Ok(AdmittedRequest::<RestorationConsumptionApplied>::new(
            request_id,
        ))
    }

    fn clear(&self, request: RestorationClearRequest) -> RestorationClearAdmission {
        if !self.capability.operations().supports_clear() {
            return Err(RestorationAdmissionError::UnsupportedOperation);
        }
        self.validate_scope(request.expected().scope())?;
        self.validate_common()?;
        let observed = self.current(request.expected().scope());
        if observed != Some(request.expected()) {
            return Err(RestorationAdmissionError::RevisionMismatch {
                expected: request.expected(),
                observed,
            });
        }
        let (next, request_id) = self.next_request_id()?;
        self.capture(
            next,
            FakeRestorationInvocation {
                request_id,
                operation: FakeRestorationOperation::Clear,
                snapshot: request.expected(),
                previous: None,
                token_byte_len: None,
            },
        )?;
        Ok(AdmittedRequest::<RestorationClearApplied>::new(request_id))
    }
}

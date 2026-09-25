//! Adapter for the platform admission contracts. Callers retain the linear
//! admission token and consume it with the matching completion method.
use super::{Clipboard, ClipboardContent, ClipboardError, ClipboardRequest, MAX_BYTES};
use crate::platform::contracts::*;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    num::{NonZeroU16, NonZeroU32, NonZeroU64},
};

pub struct ClipboardContracts {
    handle: Clipboard,
    next: Cell<u64>,
    contents: RefCell<HashMap<DataOfferId, (DataOfferDescriptor, ClipboardContent)>>,
    pending: RefCell<HashMap<RequestId, Pending>>,
    cancellations: RefCell<HashMap<RequestId, std::sync::Arc<std::sync::atomic::AtomicBool>>>,
}
enum Pending {
    Publish(ClipboardRequest<()>, ClipboardPublishApplied),
    Clear(ClipboardRequest<()>, ClipboardClearApplied),
    Read(ClipboardRequest<Vec<u8>>, DataFormatReadRequest),
    Stream(super::ClipboardStream, DataFormatReadRequest),
}
pub struct ClipboardReadResult {
    pub completion: RequestCompletion<DataReadCompletion>,
    pub bytes: Vec<u8>,
}
impl ClipboardContracts {
    pub fn new(handle: Clipboard) -> Self {
        Self {
            handle,
            next: Cell::new(1),
            contents: RefCell::new(HashMap::new()),
            pending: RefCell::new(HashMap::new()),
            cancellations: RefCell::new(HashMap::new()),
        }
    }
    fn id(&self) -> RequestId {
        let id = self.next.get();
        self.next.set(
            id.checked_add(1)
                .expect("clipboard request identity exhausted"),
        );
        RequestId::from_raw(id).unwrap()
    }
    /// Associates metadata with a real provider before admitting publication.
    pub fn register_content(
        &self,
        content: ClipboardContent,
    ) -> super::Result<DataOfferDescriptor> {
        if self.contents.borrow().len() >= super::MAX_REQUESTS {
            return Err(ClipboardError::Busy);
        }
        let id = DataOfferId::from_raw(
            u32::try_from(self.id().get()).map_err(|_| ClipboardError::Unavailable)?,
            1,
        )
        .ok_or(ClipboardError::Unavailable)?;
        let offer = DataOfferDescriptor::new(
            id,
            content.formats().to_vec(),
            DataSourceKind::Clipboard,
            TrustLevel::Trusted,
            vec![SizeHint::AtMost(MAX_BYTES as u64); content.formats().len()],
        )
        .map_err(|_| ClipboardError::InvalidFormat)?;
        self.contents
            .borrow_mut()
            .insert(id, (offer.clone(), content));
        Ok(offer)
    }
    pub fn unregister_content(&self, id: DataOfferId) {
        self.contents.borrow_mut().remove(&id);
    }
    pub async fn complete_publish(
        &self,
        token: AdmittedRequest<ClipboardPublishApplied>,
    ) -> RequestCompletion<ClipboardPublishApplied> {
        let pending = self.pending.borrow_mut().remove(&token.request_id());
        let outcome = if let Some(Pending::Publish(request, applied)) = pending {
            outcome(request.await.map(|()| applied))
        } else {
            RequestOutcome::Stale
        };
        token.complete(outcome)
    }
    pub async fn complete_clear(
        &self,
        token: AdmittedRequest<ClipboardClearApplied>,
    ) -> RequestCompletion<ClipboardClearApplied> {
        let pending = self.pending.borrow_mut().remove(&token.request_id());
        let outcome = if let Some(Pending::Clear(request, applied)) = pending {
            outcome(request.await.map(|()| applied))
        } else {
            RequestOutcome::Stale
        };
        token.complete(outcome)
    }
    pub async fn complete_read(
        &self,
        token: AdmittedRequest<DataReadCompletion>,
    ) -> ClipboardReadResult {
        let _cleanup = CancellationCleanup {
            contracts: self,
            id: token.request_id(),
        };
        let pending = self.pending.borrow_mut().remove(&token.request_id());
        let (result, bytes) = if let Some(Pending::Read(future, request)) = pending {
            match future.await {
                Ok(bytes) => (
                    DataReadCompletion::new(&request, bytes.len() as u64, 1)
                        .map_err(|_| ClipboardError::TransferFailed),
                    bytes,
                ),
                Err(error) => (Err(error), vec![]),
            }
        } else {
            (Err(ClipboardError::Stale), vec![])
        };
        self.cancellations.borrow_mut().remove(&token.request_id());
        ClipboardReadResult {
            completion: token.complete(outcome(result)),
            bytes,
        }
    }
    pub async fn complete_stream(
        &self,
        token: AdmittedRequest<DataReadCompletion>,
        mut receive: impl FnMut(Vec<u8>),
    ) -> RequestCompletion<DataReadCompletion> {
        let _cleanup = CancellationCleanup {
            contracts: self,
            id: token.request_id(),
        };
        let pending = self.pending.borrow_mut().remove(&token.request_id());
        let result = if let Some(Pending::Stream(mut stream, request)) = pending {
            let mut bytes = 0;
            let mut chunks = 0;
            let mut failure = None;
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(chunk) => {
                        bytes += chunk.len() as u64;
                        chunks += 1;
                        receive(chunk);
                    }
                    Err(error) => {
                        failure = Some(error);
                        break;
                    }
                }
            }
            if let Some(error) = failure {
                Err(error)
            } else {
                DataReadCompletion::new(&request, bytes, chunks)
                    .map_err(|_| ClipboardError::TransferFailed)
            }
        } else {
            Err(ClipboardError::Stale)
        };
        self.cancellations.borrow_mut().remove(&token.request_id());
        token.complete(outcome(result))
    }
}
fn outcome<T>(result: super::Result<T>) -> RequestOutcome<T> {
    match result {
        Ok(value) => RequestOutcome::Applied(value),
        Err(ClipboardError::Denied) => RequestOutcome::Denied,
        Err(ClipboardError::Cancelled) => RequestOutcome::Cancelled,
        Err(ClipboardError::Stale | ClipboardError::Empty) => RequestOutcome::Stale,
        Err(ClipboardError::Unsupported) => RequestOutcome::Unsupported,
        Err(_) => RequestOutcome::Failed(PlatformError::new(
            PlatformErrorKind::TransportFailure,
            "clipboard transfer",
        )),
    }
}
impl ClipboardService for ClipboardContracts {
    fn capability(&self, kind: ClipboardKind) -> Support<ClipboardCapability> {
        if self.handle.snapshot(kind).is_err() {
            return Support::Unavailable(UnavailableReason::ExecutionContextUnavailable);
        }
        Support::Available(
            ClipboardCapability::new(
                ClipboardOperations::new(true, true, true, true),
                vec![],
                ClipboardLimits::new(
                    CapabilityLimit::Bounded(NonZeroU32::new(64).unwrap()),
                    CapabilityLimit::Bounded(NonZeroU64::new(MAX_BYTES as u64).unwrap()),
                )
                .unwrap(),
                PermissionState::Granted,
                PermissionState::Granted,
                ExecutionRequirement::HostEventLoop,
                UserGestureRequirement::NotRequired,
            )
            .unwrap()
            .with_arbitrary_mime_formats(),
        )
    }
    fn current_snapshot(&self, kind: ClipboardKind) -> ClipboardSnapshotStatus {
        let Ok(snapshot) = self.handle.snapshot(kind) else {
            return ClipboardSnapshotStatus::Unavailable(
                UnavailableReason::ExecutionContextUnavailable,
            );
        };
        let Ok(generation) = u32::try_from(snapshot.revision) else {
            return ClipboardSnapshotStatus::Unavailable(
                UnavailableReason::ExecutionContextUnavailable,
            );
        };
        let id = ClipboardSnapshotId::new(
            kind,
            ClipboardRevision::from_raw(snapshot.revision).unwrap(),
        );
        let offer = if snapshot.formats.is_empty() {
            None
        } else {
            let formats = snapshot.formats;
            Some(
                DataOfferDescriptor::new(
                    DataOfferId::from_raw(super::index(kind) as u32 + 1, generation).unwrap(),
                    formats.clone(),
                    DataSourceKind::Clipboard,
                    TrustLevel::Untrusted,
                    vec![SizeHint::Unknown; formats.len()],
                )
                .unwrap(),
            )
        };
        ClipboardSnapshotStatus::Current(ClipboardSnapshot::new(id, offer).unwrap())
    }
    fn publish(
        &self,
        request: ClipboardPublishRequest,
    ) -> ClipboardRequestAdmission<ClipboardPublishApplied> {
        let kind = request.clipboard();
        if self.pending.borrow().len() >= super::MAX_REQUESTS {
            return Err(ClipboardAdmissionError::CapabilityChanged { clipboard: kind });
        }
        let Some((descriptor, content)) =
            self.contents.borrow().get(&request.offer().id()).cloned()
        else {
            return Err(ClipboardAdmissionError::CapabilityChanged { clipboard: kind });
        };
        if descriptor != *request.offer() {
            return Err(ClipboardAdmissionError::CapabilityChanged { clipboard: kind });
        }
        self.contents.borrow_mut().remove(&request.offer().id());
        let expected = request.expected().map(|id| id.revision().get());
        let future = self.handle.publish(kind, content, expected);
        let id = self.id();
        self.pending.borrow_mut().insert(
            id,
            Pending::Publish(future, ClipboardPublishApplied::from_request(&request)),
        );
        Ok(AdmittedRequest::new(id))
    }
    fn clear(
        &self,
        request: ClipboardClearRequest,
    ) -> ClipboardRequestAdmission<ClipboardClearApplied> {
        if self.pending.borrow().len() >= super::MAX_REQUESTS {
            return Err(ClipboardAdmissionError::CapabilityChanged {
                clipboard: request.clipboard(),
            });
        }
        let future = self.handle.clear(
            request.clipboard(),
            request.expected().map(|id| id.revision().get()),
        );
        let id = self.id();
        self.pending.borrow_mut().insert(
            id,
            Pending::Clear(future, ClipboardClearApplied::from_request(request)),
        );
        Ok(AdmittedRequest::new(id))
    }
}
impl DataTransferService for ClipboardContracts {
    fn capability(&self) -> Support<DataTransferCapability> {
        Support::Available(DataTransferCapability::new(
            DataTransferOperations {
                inbound_read: true,
                outbound_offer: true,
                native_drag_and_drop: false,
                share: false,
                cancellation: true,
                streaming: true,
            },
            DataTransferLimits::new(
                NonZeroU16::new(64).unwrap(),
                NonZeroU64::new(MAX_BYTES as u64).unwrap(),
                NonZeroU32::new(65536).unwrap(),
            )
            .unwrap(),
            PermissionState::Granted,
            ExecutionRequirement::HostEventLoop,
            UserGestureRequirement::NotRequired,
        ))
    }
    fn request_read(&self, request: DataFormatReadRequest) -> DataReadAdmission {
        if self.pending.borrow().len() >= super::MAX_REQUESTS
            || self.cancellations.borrow().len() >= super::MAX_REQUESTS
        {
            return Err(DataTransferAdmissionError::CapacityExceeded);
        }
        let snapshot = [ClipboardKind::System, ClipboardKind::Selection]
            .into_iter()
            .find_map(|kind| {
                let observed = self.current_snapshot(kind);
                let offer = observed.current()?.current_offer()?;
                if request.validate_against(offer).is_err() {
                    return None;
                }
                self.handle.snapshot(kind).ok()
            })
            .ok_or(DataTransferAdmissionError::Unavailable)?;
        let id = self.id();
        let (pending, cancel) = match request.mode() {
            DataReadMode::Buffered => {
                let future = self.handle.read(
                    snapshot,
                    request.format().clone(),
                    request.max_bytes().get() as usize,
                );
                let cancel = future.cancel.clone();
                (Pending::Read(future, request), cancel)
            }
            DataReadMode::Streamed { max_chunk_bytes } => {
                let stream = self
                    .handle
                    .read_stream(
                        snapshot,
                        request.format().clone(),
                        request.max_bytes().get() as usize,
                        max_chunk_bytes.get() as usize,
                    )
                    .map_err(|_| DataTransferAdmissionError::Unsupported)?;
                let cancel = stream.cancellation();
                (Pending::Stream(stream, request), cancel)
            }
        };
        self.cancellations.borrow_mut().insert(id, cancel);
        self.pending.borrow_mut().insert(id, pending);
        Ok(AdmittedRequest::new(id))
    }
    fn cancel_read(&self, id: RequestId) -> std::result::Result<(), DataTransferAdmissionError> {
        let cancellations = self.cancellations.borrow();
        let cancel = cancellations
            .get(&id)
            .ok_or(DataTransferAdmissionError::Unavailable)?;
        cancel.store(true, std::sync::atomic::Ordering::Release);
        Ok(())
    }
}

struct CancellationCleanup<'a> {
    contracts: &'a ClipboardContracts,
    id: RequestId,
}
impl Drop for CancellationCleanup<'_> {
    fn drop(&mut self) {
        self.contracts.cancellations.borrow_mut().remove(&self.id);
    }
}

//! Bounded, multi-format metadata for clipboard and native data transfer.
//!
//! This module describes offers and admits asynchronous reads without owning transferred bytes,
//! native objects, an executor, callbacks, queues, or platform I/O. A format is always selected
//! explicitly: HTML, images, URI lists, native formats, and custom MIME types are never silently
//! converted to plain text. Adapters remain responsible for delivering content through their
//! host-owned completion path and for stopping before the admitted byte limit.

use std::fmt;
use std::num::{NonZeroU16, NonZeroU32, NonZeroU64};
use std::rc::Rc;
use std::sync::Arc;

use crate::platform::contracts::{
    CapabilityDescriptor, DataOfferId, RequestAdmission, RequestId, ServiceKey, Support,
};

/// Maximum number of explicitly advertised formats retained for one offer.
pub const MAX_DATA_FORMATS_PER_OFFER: usize = 64;

/// Maximum UTF-8 byte length of one MIME, UTI, native namespace, or native identifier.
pub const MAX_DATA_FORMAT_IDENTIFIER_BYTES: usize = 255;

/// Hard neutral-spine ceiling for one admitted data read (64 MiB).
///
/// A service capability may declare a smaller limit. This ceiling exists so an untrusted offer
/// can never cause an allocation request derived only from its own size metadata.
pub const MAX_DATA_READ_BYTES: u64 = 64 * 1024 * 1024;

/// Hard neutral-spine ceiling for one streaming chunk (1 MiB).
pub const MAX_DATA_STREAM_CHUNK_BYTES: u32 = 1024 * 1024;

/// Namespace used to interpret a data format identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DataFormatKind {
    /// An Internet media type such as `text/plain` or `image/png`.
    Mime,
    /// An Apple uniform type identifier.
    UniformTypeIdentifier,
    /// A platform-native format name interpreted only by its named adapter namespace.
    Native,
}

/// Validation failure for bounded format metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DataFormatError {
    /// An identifier or native namespace was empty.
    EmptyIdentifier,
    /// An identifier exceeded [`MAX_DATA_FORMAT_IDENTIFIER_BYTES`].
    IdentifierTooLong,
    /// An identifier contained whitespace, a control character, or invalid MIME structure.
    InvalidIdentifier,
}

impl fmt::Display for DataFormatError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyIdentifier => "data format identifier is empty",
            Self::IdentifierTooLong => "data format identifier exceeds the metadata bound",
            Self::InvalidIdentifier => "data format identifier is invalid",
        })
    }
}

impl std::error::Error for DataFormatError {}

/// One exact advertised representation of an offer.
///
/// Construction validates and copies at most 255 bytes per identifier. It deliberately preserves
/// spelling and namespace instead of normalizing or coercing one format into another.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DataFormat {
    kind: DataFormatKind,
    namespace: Option<Arc<str>>,
    identifier: Arc<str>,
}

impl DataFormat {
    /// Constructs an exact MIME representation.
    pub fn mime(identifier: &str) -> Result<Self, DataFormatError> {
        validate_identifier(identifier)?;
        if !identifier.is_ascii() || !is_valid_mime(identifier) {
            return Err(DataFormatError::InvalidIdentifier);
        }
        Ok(Self::new(DataFormatKind::Mime, None, identifier))
    }

    /// Constructs an exact uniform type identifier.
    pub fn uniform_type_identifier(identifier: &str) -> Result<Self, DataFormatError> {
        validate_identifier(identifier)?;
        Ok(Self::new(
            DataFormatKind::UniformTypeIdentifier,
            None,
            identifier,
        ))
    }

    /// Constructs an adapter-scoped native representation.
    ///
    /// `namespace` identifies the interpreting adapter (for example, a platform family), rather
    /// than smuggling a native handle or protocol object into portable state.
    pub fn native(namespace: &str, identifier: &str) -> Result<Self, DataFormatError> {
        validate_identifier(namespace)?;
        validate_identifier(identifier)?;
        Ok(Self::new(
            DataFormatKind::Native,
            Some(namespace),
            identifier,
        ))
    }

    fn new(kind: DataFormatKind, namespace: Option<&str>, identifier: &str) -> Self {
        Self {
            kind,
            namespace: namespace.map(Arc::from),
            identifier: Arc::from(identifier),
        }
    }

    /// Returns the namespace in which the identifier must be interpreted.
    pub const fn kind(&self) -> DataFormatKind {
        self.kind
    }

    /// Returns the exact, uncoerced identifier.
    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    /// Returns the adapter namespace for a native format.
    pub fn native_namespace(&self) -> Option<&str> {
        self.namespace.as_deref()
    }
}

impl fmt::Debug for DataFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DataFormat")
            .field("kind", &self.kind)
            .field("namespace", &self.namespace)
            .field("identifier", &self.identifier)
            .finish()
    }
}

fn validate_identifier(identifier: &str) -> Result<(), DataFormatError> {
    if identifier.is_empty() {
        return Err(DataFormatError::EmptyIdentifier);
    }
    if identifier.len() > MAX_DATA_FORMAT_IDENTIFIER_BYTES {
        return Err(DataFormatError::IdentifierTooLong);
    }
    if identifier.chars().any(char::is_whitespace) || identifier.chars().any(char::is_control) {
        return Err(DataFormatError::InvalidIdentifier);
    }
    Ok(())
}

fn is_valid_mime(identifier: &str) -> bool {
    let mut parts = identifier.split('/');
    matches!((parts.next(), parts.next(), parts.next()), (Some(left), Some(right), None) if !left.is_empty() && !right.is_empty())
}

/// Origin category for one data offer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DataSourceKind {
    /// A clipboard or selection-clipboard owner.
    Clipboard,
    /// An operating-system drag-and-drop operation.
    DragAndDrop,
    /// A platform share-style transfer operation.
    Share,
}

/// Whether size and format claims may be treated as host-authenticated metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrustLevel {
    /// Metadata was produced within the trusted application or embedding host.
    Trusted,
    /// Metadata came from an external client and must be treated as untrusted.
    Untrusted,
}

/// Optional byte-size metadata for one exact offered format.
///
/// Hints never authorize allocation. Every read independently supplies a validated byte limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SizeHint {
    /// The provider supplied no useful size metadata.
    Unknown,
    /// The provider claims an exact encoded byte count.
    Exact(u64),
    /// The provider claims the encoded byte count will not exceed this value.
    AtMost(u64),
}

impl SizeHint {
    /// Returns the claimed upper bound when one is available.
    pub const fn upper_bound(self) -> Option<u64> {
        match self {
            Self::Unknown => None,
            Self::Exact(bytes) | Self::AtMost(bytes) => Some(bytes),
        }
    }

    /// Returns the claimed exact size when one is available.
    pub const fn exact(self) -> Option<u64> {
        match self {
            Self::Exact(bytes) => Some(bytes),
            Self::Unknown | Self::AtMost(_) => None,
        }
    }
}

/// Validation failure while constructing immutable offer metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DataOfferError {
    /// An offer advertised no readable representation.
    EmptyFormats,
    /// An offer exceeded [`MAX_DATA_FORMATS_PER_OFFER`].
    TooManyFormats,
    /// Two entries advertised the same exact representation.
    DuplicateFormat,
    /// Format and size-hint arrays were not one-to-one.
    SizeHintCountMismatch,
}

impl fmt::Display for DataOfferError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyFormats => "data offer has no formats",
            Self::TooManyFormats => "data offer exceeds the format-count bound",
            Self::DuplicateFormat => "data offer contains a duplicate format",
            Self::SizeHintCountMismatch => "data offer size hints do not match its formats",
        })
    }
}

impl std::error::Error for DataOfferError {}

/// Immutable, bounded metadata for one generation-aware data offer.
#[derive(Clone, PartialEq, Eq)]
pub struct DataOfferDescriptor {
    id: DataOfferId,
    formats: Arc<[DataFormat]>,
    source: DataSourceKind,
    trust: TrustLevel,
    size_hints: Arc<[SizeHint]>,
}

impl DataOfferDescriptor {
    /// Validates one-to-one format metadata and constructs an offer descriptor.
    ///
    /// The vectors are accepted by ownership so validation never duplicates an attacker-sized
    /// input. More than 64 entries are rejected before conversion into retained slices.
    pub fn new(
        id: DataOfferId,
        formats: Vec<DataFormat>,
        source: DataSourceKind,
        trust: TrustLevel,
        size_hints: Vec<SizeHint>,
    ) -> Result<Self, DataOfferError> {
        if formats.is_empty() {
            return Err(DataOfferError::EmptyFormats);
        }
        if formats.len() > MAX_DATA_FORMATS_PER_OFFER {
            return Err(DataOfferError::TooManyFormats);
        }
        if formats.len() != size_hints.len() {
            return Err(DataOfferError::SizeHintCountMismatch);
        }
        for (index, format) in formats.iter().enumerate() {
            if formats[..index].contains(format) {
                return Err(DataOfferError::DuplicateFormat);
            }
        }

        Ok(Self {
            id,
            formats: formats.into(),
            source,
            trust,
            size_hints: size_hints.into(),
        })
    }

    /// Returns the opaque slot and generation issued by the offer owner.
    pub const fn id(&self) -> DataOfferId {
        self.id
    }

    /// Returns exact advertised formats in provider order.
    pub fn formats(&self) -> &[DataFormat] {
        &self.formats
    }

    /// Returns the source category without inferring trust from it.
    pub const fn source(&self) -> DataSourceKind {
        self.source
    }

    /// Returns the explicit trust classification.
    pub const fn trust(&self) -> TrustLevel {
        self.trust
    }

    /// Returns size hints aligned one-to-one with [`Self::formats`].
    pub fn size_hints(&self) -> &[SizeHint] {
        &self.size_hints
    }

    /// Finds the hint for one exact representation without format conversion.
    pub fn size_hint(&self, format: &DataFormat) -> Option<SizeHint> {
        self.formats
            .iter()
            .position(|candidate| candidate == format)
            .map(|index| self.size_hints[index])
    }
}

impl fmt::Debug for DataOfferDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DataOfferDescriptor")
            .field("id", &self.id)
            .field("format_count", &self.formats.len())
            .field("source", &self.source)
            .field("trust", &self.trust)
            .field("size_hint_count", &self.size_hints.len())
            .finish_non_exhaustive()
    }
}

/// How admitted content is to be delivered by a host-owned completion path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DataReadMode {
    /// Deliver one bounded completed value through the host's completion transport.
    Buffered,
    /// Deliver bounded chunks no larger than `max_chunk_bytes`.
    Streamed {
        /// Caller-selected upper bound for each chunk.
        max_chunk_bytes: NonZeroU32,
    },
}

/// Validation failure before a data-format read is admitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DataReadValidationError {
    /// The exact requested representation was not advertised by this offer.
    FormatNotOffered,
    /// The requested total byte bound exceeded [`MAX_DATA_READ_BYTES`].
    ReadLimitTooLarge,
    /// A streamed chunk bound exceeded [`MAX_DATA_STREAM_CHUNK_BYTES`].
    ChunkLimitTooLarge,
    /// A streamed chunk bound exceeded the request's total byte bound.
    ChunkLimitExceedsReadLimit,
    /// An exact size claim is already larger than the caller's admitted byte bound.
    KnownSizeExceedsReadLimit,
    /// Validation cited a different offer slot or generation.
    OfferMismatch,
}

impl fmt::Display for DataReadValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::FormatNotOffered => "requested data format was not offered",
            Self::ReadLimitTooLarge => "data read limit exceeds the neutral bound",
            Self::ChunkLimitTooLarge => "data chunk limit exceeds the neutral bound",
            Self::ChunkLimitExceedsReadLimit => "data chunk limit exceeds the read limit",
            Self::KnownSizeExceedsReadLimit => "known data size exceeds the read limit",
            Self::OfferMismatch => "data read cites a different offer generation",
        })
    }
}

impl std::error::Error for DataReadValidationError {}

/// Generation-aware, exact-format, bounded request metadata for one asynchronous read.
#[derive(Clone, PartialEq, Eq)]
pub struct DataFormatReadRequest {
    offer: DataOfferId,
    format: DataFormat,
    size_hint: SizeHint,
    max_bytes: NonZeroU64,
    mode: DataReadMode,
}

impl DataFormatReadRequest {
    /// Validates and constructs a read for one exact format of `offer`.
    pub fn for_offer(
        offer: &DataOfferDescriptor,
        format: DataFormat,
        max_bytes: NonZeroU64,
        mode: DataReadMode,
    ) -> Result<Self, DataReadValidationError> {
        validate_read_limits(max_bytes, mode)?;
        let Some(size_hint) = offer.size_hint(&format) else {
            return Err(DataReadValidationError::FormatNotOffered);
        };
        if size_hint.exact().is_some_and(|size| size > max_bytes.get()) {
            return Err(DataReadValidationError::KnownSizeExceedsReadLimit);
        }

        Ok(Self {
            offer: offer.id(),
            format,
            size_hint,
            max_bytes,
            mode,
        })
    }

    /// Revalidates this request against the currently observed offer generation and metadata.
    pub fn validate_against(
        &self,
        offer: &DataOfferDescriptor,
    ) -> Result<(), DataReadValidationError> {
        if self.offer != offer.id() {
            return Err(DataReadValidationError::OfferMismatch);
        }
        if offer.size_hint(&self.format) != Some(self.size_hint) {
            return Err(DataReadValidationError::FormatNotOffered);
        }
        validate_read_limits(self.max_bytes, self.mode)
    }

    /// Returns the offer identity, including the generation observed at construction.
    pub const fn offer(&self) -> DataOfferId {
        self.offer
    }

    /// Returns the one exact requested representation.
    pub const fn format(&self) -> &DataFormat {
        &self.format
    }

    /// Returns the offer's size hint captured when the request was validated.
    pub const fn size_hint(&self) -> SizeHint {
        self.size_hint
    }

    /// Returns the caller-selected total byte ceiling.
    pub const fn max_bytes(&self) -> NonZeroU64 {
        self.max_bytes
    }

    /// Returns the caller-selected delivery mode and chunk bound.
    pub const fn mode(&self) -> DataReadMode {
        self.mode
    }
}

impl fmt::Debug for DataFormatReadRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DataFormatReadRequest")
            .field("offer", &self.offer)
            .field("format_kind", &self.format.kind())
            .field("size_hint", &self.size_hint)
            .field("max_bytes", &self.max_bytes)
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}

fn validate_read_limits(
    max_bytes: NonZeroU64,
    mode: DataReadMode,
) -> Result<(), DataReadValidationError> {
    if max_bytes.get() > MAX_DATA_READ_BYTES {
        return Err(DataReadValidationError::ReadLimitTooLarge);
    }
    if let DataReadMode::Streamed { max_chunk_bytes } = mode {
        if max_chunk_bytes.get() > MAX_DATA_STREAM_CHUNK_BYTES {
            return Err(DataReadValidationError::ChunkLimitTooLarge);
        }
        if u64::from(max_chunk_bytes.get()) > max_bytes.get() {
            return Err(DataReadValidationError::ChunkLimitExceedsReadLimit);
        }
    }
    Ok(())
}

/// Invalid progress or completion metadata supplied by an adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DataReadMetadataError {
    /// Reported bytes exceeded the caller's admitted total bound.
    ReadLimitExceeded,
    /// A buffered read reported more than one content unit.
    InvalidChunkCount,
    /// Completion contradicted the offer's exact or upper-bound size claim.
    SizeHintViolated,
}

impl fmt::Display for DataReadMetadataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ReadLimitExceeded => "reported data bytes exceed the admitted limit",
            Self::InvalidChunkCount => "reported data chunk count is invalid for the read mode",
            Self::SizeHintViolated => "completed data size contradicts the offer metadata",
        })
    }
}

impl std::error::Error for DataReadMetadataError {}

/// Content-free streaming progress metadata for diagnostics and completion accounting.
#[derive(Clone, PartialEq, Eq)]
pub struct DataReadProgress {
    request: RequestId,
    offer: DataOfferId,
    format: DataFormat,
    bytes_received: u64,
    chunks_received: u32,
}

impl DataReadProgress {
    /// Validates monotonic aggregate metadata against an admitted request.
    pub fn new(
        request_id: RequestId,
        request: &DataFormatReadRequest,
        bytes_received: u64,
        chunks_received: u32,
    ) -> Result<Self, DataReadMetadataError> {
        validate_read_metadata(request, bytes_received, chunks_received, false)?;
        Ok(Self {
            request: request_id,
            offer: request.offer,
            format: request.format.clone(),
            bytes_received,
            chunks_received,
        })
    }

    /// Returns the admitted request whose progress is being reported.
    pub const fn request(&self) -> RequestId {
        self.request
    }

    /// Returns the cited offer generation.
    pub const fn offer(&self) -> DataOfferId {
        self.offer
    }

    /// Returns the exact representation being read.
    pub const fn format(&self) -> &DataFormat {
        &self.format
    }

    /// Returns the aggregate byte count already delivered by the host.
    pub const fn bytes_received(&self) -> u64 {
        self.bytes_received
    }

    /// Returns the aggregate number of delivered chunks or buffered units.
    pub const fn chunks_received(&self) -> u32 {
        self.chunks_received
    }
}

impl fmt::Debug for DataReadProgress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DataReadProgress")
            .field("request", &self.request)
            .field("offer", &self.offer)
            .field("format_kind", &self.format.kind())
            .field("bytes_received", &self.bytes_received)
            .field("chunks_received", &self.chunks_received)
            .finish_non_exhaustive()
    }
}

/// Content-free successful terminal metadata for a completed data read.
///
/// Cancellation, staleness, denial, unsupported operation, and platform failure remain the
/// distinct variants of `RequestOutcome<DataReadCompletion>`.
#[derive(Clone, PartialEq, Eq)]
pub struct DataReadCompletion {
    offer: DataOfferId,
    format: DataFormat,
    bytes_read: u64,
    chunks_read: u32,
}

impl DataReadCompletion {
    /// Validates successful terminal metadata against the admitted request.
    pub fn new(
        request: &DataFormatReadRequest,
        bytes_read: u64,
        chunks_read: u32,
    ) -> Result<Self, DataReadMetadataError> {
        validate_read_metadata(request, bytes_read, chunks_read, true)?;
        Ok(Self {
            offer: request.offer,
            format: request.format.clone(),
            bytes_read,
            chunks_read,
        })
    }

    /// Returns the completed offer generation.
    pub const fn offer(&self) -> DataOfferId {
        self.offer
    }

    /// Returns the exact completed representation.
    pub const fn format(&self) -> &DataFormat {
        &self.format
    }

    /// Returns the bounded encoded byte count delivered by the host.
    pub const fn bytes_read(&self) -> u64 {
        self.bytes_read
    }

    /// Returns the number of delivered chunks or buffered units.
    pub const fn chunks_read(&self) -> u32 {
        self.chunks_read
    }
}

impl fmt::Debug for DataReadCompletion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DataReadCompletion")
            .field("offer", &self.offer)
            .field("format_kind", &self.format.kind())
            .field("bytes_read", &self.bytes_read)
            .field("chunks_read", &self.chunks_read)
            .finish_non_exhaustive()
    }
}

fn validate_read_metadata(
    request: &DataFormatReadRequest,
    bytes: u64,
    chunks: u32,
    terminal: bool,
) -> Result<(), DataReadMetadataError> {
    if bytes > request.max_bytes.get() {
        return Err(DataReadMetadataError::ReadLimitExceeded);
    }
    if matches!(request.mode, DataReadMode::Buffered) && chunks > 1 {
        return Err(DataReadMetadataError::InvalidChunkCount);
    }
    if bytes > 0 && chunks == 0 {
        return Err(DataReadMetadataError::InvalidChunkCount);
    }
    if let DataReadMode::Streamed { max_chunk_bytes } = request.mode
        && bytes > u64::from(chunks) * u64::from(max_chunk_bytes.get())
    {
        return Err(DataReadMetadataError::InvalidChunkCount);
    }
    if terminal {
        let hint_valid = match request.size_hint {
            SizeHint::Unknown => true,
            SizeHint::Exact(expected) => bytes == expected,
            SizeHint::AtMost(limit) => bytes <= limit,
        };
        if !hint_valid {
            return Err(DataReadMetadataError::SizeHintViolated);
        }
    }
    Ok(())
}

/// Operations independently advertised by a data-transfer service.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DataTransferOperations {
    /// Exact-format reads from inbound offers are supported.
    pub inbound_read: bool,
    /// Publishing or initiating native outbound offers is supported.
    pub outbound_offer: bool,
    /// Native drag/drop negotiation is supported beyond simple inbound file events.
    pub native_drag_and_drop: bool,
    /// Share-style outbound transfer is supported.
    pub share: bool,
    /// An admitted read may be cancelled.
    pub cancellation: bool,
    /// An admitted read may use [`DataReadMode::Streamed`].
    pub streaming: bool,
}

/// Service-specific limits, each no larger than the neutral-spine hard ceiling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DataTransferLimits {
    max_formats_per_offer: NonZeroU16,
    max_read_bytes: NonZeroU64,
    max_chunk_bytes: NonZeroU32,
}

impl DataTransferLimits {
    /// Validates capability limits against the neutral hard ceilings.
    pub const fn new(
        max_formats_per_offer: NonZeroU16,
        max_read_bytes: NonZeroU64,
        max_chunk_bytes: NonZeroU32,
    ) -> Result<Self, DataTransferLimitError> {
        if max_formats_per_offer.get() as usize > MAX_DATA_FORMATS_PER_OFFER {
            return Err(DataTransferLimitError::TooManyFormats);
        }
        if max_read_bytes.get() > MAX_DATA_READ_BYTES {
            return Err(DataTransferLimitError::ReadLimitTooLarge);
        }
        if max_chunk_bytes.get() > MAX_DATA_STREAM_CHUNK_BYTES
            || max_chunk_bytes.get() as u64 > max_read_bytes.get()
        {
            return Err(DataTransferLimitError::ChunkLimitTooLarge);
        }
        Ok(Self {
            max_formats_per_offer,
            max_read_bytes,
            max_chunk_bytes,
        })
    }

    /// Returns the maximum advertised representations retained per offer.
    pub const fn max_formats_per_offer(self) -> NonZeroU16 {
        self.max_formats_per_offer
    }

    /// Returns the service's maximum admitted total read size.
    pub const fn max_read_bytes(self) -> NonZeroU64 {
        self.max_read_bytes
    }

    /// Returns the service's maximum streaming chunk size.
    pub const fn max_chunk_bytes(self) -> NonZeroU32 {
        self.max_chunk_bytes
    }
}

/// Invalid service limit metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DataTransferLimitError {
    /// The service claimed more formats than the neutral descriptor retains.
    TooManyFormats,
    /// The service claimed a total read size above the neutral ceiling.
    ReadLimitTooLarge,
    /// The service claimed a chunk above either the chunk or total-read ceiling.
    ChunkLimitTooLarge,
}

impl fmt::Display for DataTransferLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TooManyFormats => "data-transfer format limit exceeds the neutral bound",
            Self::ReadLimitTooLarge => "data-transfer read limit exceeds the neutral bound",
            Self::ChunkLimitTooLarge => "data-transfer chunk limit exceeds its allowed bound",
        })
    }
}

impl std::error::Error for DataTransferLimitError {}

/// Complete capability record returned by [`DataTransferService::capability`].
pub type DataTransferCapability = CapabilityDescriptor<DataTransferOperations, DataTransferLimits>;

/// Immediate rejection while admitting a read or cancellation command.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DataTransferAdmissionError {
    /// The service or its host execution context is no longer available.
    Unavailable,
    /// The requested operation or delivery mode is unsupported.
    Unsupported,
    /// Permission or platform policy denied admission.
    Denied,
    /// The service cannot retain another admitted request within its declared bound.
    CapacityExceeded,
}

impl fmt::Display for DataTransferAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unavailable => "data-transfer service is unavailable",
            Self::Unsupported => "data-transfer operation is unsupported",
            Self::Denied => "data-transfer operation was denied",
            Self::CapacityExceeded => "data-transfer admission capacity was exceeded",
        })
    }
}

impl std::error::Error for DataTransferAdmissionError {}

/// Linear admission token for an asynchronous exact-format data read.
pub type DataReadAdmission = RequestAdmission<DataReadCompletion, DataTransferAdmissionError>;

/// Narrow command-admission boundary for native data transfer.
///
/// Methods may only validate and admit commands. Implementations must not block, invoke component
/// code, or deliver content synchronously. The host's existing completion transport later delivers
/// exactly one `RequestOutcome<DataReadCompletion>` for each admitted read. Cancelling requests a
/// `Cancelled` terminal outcome; it does not erase an already produced completion.
pub trait DataTransferService {
    /// Reports current support, limits, permissions, execution, and gesture requirements.
    fn capability(&self) -> Support<DataTransferCapability>;

    /// Admits an already bounded exact-format read and returns its linear completion token.
    fn request_read(&self, request: DataFormatReadRequest) -> DataReadAdmission;

    /// Requests cancellation of a previously admitted read.
    fn cancel_read(&self, request: RequestId) -> Result<(), DataTransferAdmissionError>;
}

/// Registry key for a local-owner data-transfer service handle.
pub enum DataTransferServiceKey {}

impl ServiceKey for DataTransferServiceKey {
    type Handle = Rc<dyn DataTransferService>;
}

#[cfg(test)]
mod tests {
    use std::num::{NonZeroU16, NonZeroU32, NonZeroU64};

    use super::*;

    fn offer(trust: TrustLevel) -> DataOfferDescriptor {
        DataOfferDescriptor::new(
            DataOfferId::from_raw(4, 8).unwrap(),
            vec![
                DataFormat::mime("text/plain;charset=utf-8").unwrap(),
                DataFormat::mime("text/html").unwrap(),
            ],
            DataSourceKind::Clipboard,
            trust,
            vec![SizeHint::AtMost(128), SizeHint::Exact(512)],
        )
        .unwrap()
    }

    #[test]
    fn descriptor_preserves_multiple_exact_formats_and_redacts_debug() {
        let offer = offer(TrustLevel::Untrusted);
        assert_eq!(offer.id().generation(), 8);
        assert_eq!(offer.formats().len(), 2);
        assert_eq!(offer.formats()[0].identifier(), "text/plain;charset=utf-8");
        assert_eq!(
            offer.size_hint(&offer.formats()[1]),
            Some(SizeHint::Exact(512))
        );

        let debug = format!("{offer:?}");
        assert!(debug.contains("format_count: 2"));
        assert!(!debug.contains("text/plain"));
        assert!(!debug.contains("text/html"));
    }

    #[test]
    fn offer_validation_rejects_empty_duplicate_mismatched_and_oversized_metadata() {
        let id = DataOfferId::MIN;
        assert_eq!(
            DataOfferDescriptor::new(
                id,
                vec![],
                DataSourceKind::Share,
                TrustLevel::Trusted,
                vec![],
            ),
            Err(DataOfferError::EmptyFormats)
        );
        let format = DataFormat::mime("image/png").unwrap();
        assert_eq!(
            DataOfferDescriptor::new(
                id,
                vec![format.clone(), format],
                DataSourceKind::DragAndDrop,
                TrustLevel::Untrusted,
                vec![SizeHint::Unknown, SizeHint::Unknown],
            ),
            Err(DataOfferError::DuplicateFormat)
        );
        assert!(matches!(
            DataOfferDescriptor::new(
                id,
                vec![DataFormat::mime("image/png").unwrap()],
                DataSourceKind::Clipboard,
                TrustLevel::Trusted,
                vec![],
            ),
            Err(DataOfferError::SizeHintCountMismatch)
        ));
        let formats = (0..=MAX_DATA_FORMATS_PER_OFFER)
            .map(|index| DataFormat::native("fixture", &format!("format-{index}")).unwrap())
            .collect::<Vec<_>>();
        assert!(matches!(
            DataOfferDescriptor::new(
                id,
                formats,
                DataSourceKind::Clipboard,
                TrustLevel::Untrusted,
                vec![SizeHint::Unknown; MAX_DATA_FORMATS_PER_OFFER + 1],
            ),
            Err(DataOfferError::TooManyFormats)
        ));
    }

    #[test]
    fn exact_format_read_is_generation_aware_and_bounded_before_admission() {
        let offer = offer(TrustLevel::Untrusted);
        let html = offer.formats()[1].clone();
        assert_eq!(
            DataFormatReadRequest::for_offer(
                &offer,
                html.clone(),
                NonZeroU64::new(511).unwrap(),
                DataReadMode::Buffered,
            ),
            Err(DataReadValidationError::KnownSizeExceedsReadLimit)
        );

        let request = DataFormatReadRequest::for_offer(
            &offer,
            html,
            NonZeroU64::new(1024).unwrap(),
            DataReadMode::Streamed {
                max_chunk_bytes: NonZeroU32::new(128).unwrap(),
            },
        )
        .unwrap();
        assert_eq!(request.offer(), offer.id());
        assert_eq!(request.format().identifier(), "text/html");

        let replacement = DataOfferDescriptor::new(
            DataOfferId::from_raw(4, 9).unwrap(),
            offer.formats().to_vec(),
            DataSourceKind::Clipboard,
            TrustLevel::Untrusted,
            offer.size_hints().to_vec(),
        )
        .unwrap();
        assert_eq!(
            request.validate_against(&replacement),
            Err(DataReadValidationError::OfferMismatch)
        );
    }

    #[test]
    fn progress_and_completion_contain_only_validated_metadata() {
        let offer = offer(TrustLevel::Untrusted);
        let request = DataFormatReadRequest::for_offer(
            &offer,
            offer.formats()[1].clone(),
            NonZeroU64::new(1024).unwrap(),
            DataReadMode::Streamed {
                max_chunk_bytes: NonZeroU32::new(128).unwrap(),
            },
        )
        .unwrap();
        let progress = DataReadProgress::new(RequestId::MIN, &request, 256, 2).unwrap();
        assert_eq!(progress.bytes_received(), 256);
        let completed = DataReadCompletion::new(&request, 512, 4).unwrap();
        assert_eq!(completed.bytes_read(), 512);
        assert_eq!(completed.chunks_read(), 4);
        assert_eq!(
            DataReadCompletion::new(&request, 513, 5),
            Err(DataReadMetadataError::SizeHintViolated)
        );
    }

    #[test]
    fn capability_limits_cannot_relax_neutral_hard_bounds() {
        let limits = DataTransferLimits::new(
            NonZeroU16::new(8).unwrap(),
            NonZeroU64::new(4096).unwrap(),
            NonZeroU32::new(512).unwrap(),
        )
        .unwrap();
        assert_eq!(limits.max_formats_per_offer().get(), 8);
        assert_eq!(limits.max_read_bytes().get(), 4096);
        assert_eq!(limits.max_chunk_bytes().get(), 512);

        assert_eq!(
            DataTransferLimits::new(
                NonZeroU16::new(65).unwrap(),
                NonZeroU64::new(4096).unwrap(),
                NonZeroU32::new(512).unwrap(),
            ),
            Err(DataTransferLimitError::TooManyFormats)
        );
    }
}

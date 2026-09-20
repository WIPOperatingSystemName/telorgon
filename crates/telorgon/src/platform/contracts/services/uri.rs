//! Platform-neutral external-URI capability and open-request admission.
//!
//! This module validates a bounded absolute URI lexical envelope and retains its normalized
//! scheme for capability matching. It does not authorize scheme-specific content, resolve a URI,
//! perform network I/O, launch a handler, retain a native URL object, or own a callback, queue,
//! executor, thread, or event loop. URI contents and gesture grants are omitted from diagnostics.

use std::error::Error;
use std::fmt;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::Arc;

use super::ServiceKey;
use crate::platform::contracts::{
    CapabilityDescriptor, RequestAdmission, Support, UserGestureGrantHandle, ViewId,
};

/// Hard UTF-8 byte bound for one external URI intent.
pub const MAX_EXTERNAL_URI_BYTES: usize = 8 * 1_024;
/// Hard byte bound for one normalized URI scheme.
pub const MAX_URI_SCHEME_BYTES: usize = 64;
/// Maximum independently described schemes in one service capability publication.
pub const MAX_URI_SCHEMES: usize = 32;

/// Validated, normalized absolute-URI scheme.
///
/// Schemes use the RFC 3986 lexical form `ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )` and are
/// normalized to lowercase ASCII for capability matching.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UriScheme(Arc<str>);

impl UriScheme {
    pub fn new(value: impl AsRef<str>) -> Result<Self, UriSchemeError> {
        let value = value.as_ref();
        if value.is_empty() {
            return Err(UriSchemeError::Empty);
        }
        if value.len() > MAX_URI_SCHEME_BYTES {
            return Err(UriSchemeError::TooLong {
                byte_len: value.len(),
                maximum_bytes: MAX_URI_SCHEME_BYTES,
            });
        }
        if !value.is_ascii() {
            return Err(UriSchemeError::NonAscii);
        }
        let mut bytes = value.bytes();
        let first = bytes.next().expect("nonempty scheme has a first byte");
        if !first.is_ascii_alphabetic() {
            return Err(UriSchemeError::InvalidFirstCharacter);
        }
        if bytes.any(|byte| !byte.is_ascii_alphanumeric() && !matches!(byte, b'+' | b'-' | b'.')) {
            return Err(UriSchemeError::InvalidCharacter);
        }
        Ok(Self(Arc::from(value.to_ascii_lowercase())))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn byte_len(&self) -> usize {
        self.0.len()
    }
}

impl fmt::Debug for UriScheme {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("UriScheme")
            .field(&self.as_str())
            .finish()
    }
}

impl fmt::Display for UriScheme {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Invalid URI scheme syntax or size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UriSchemeError {
    Empty,
    TooLong {
        byte_len: usize,
        maximum_bytes: usize,
    },
    NonAscii,
    InvalidFirstCharacter,
    InvalidCharacter,
}

impl fmt::Display for UriSchemeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("URI scheme is empty"),
            Self::TooLong {
                byte_len,
                maximum_bytes,
            } => write!(
                formatter,
                "URI scheme contains {byte_len} bytes; maximum is {maximum_bytes}"
            ),
            Self::NonAscii => formatter.write_str("URI scheme must be ASCII"),
            Self::InvalidFirstCharacter => {
                formatter.write_str("URI scheme must begin with an ASCII letter")
            }
            Self::InvalidCharacter => formatter.write_str("URI scheme contains an invalid byte"),
        }
    }
}

impl Error for UriSchemeError {}

/// Bounded absolute external URI with redacted diagnostics.
///
/// Construction checks the absolute scheme prefix, ASCII RFC 3986 character vocabulary, and
/// percent escapes. Scheme-specific authority/path/query semantics remain adapter or application
/// policy and are not inferred here.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ExternalUri {
    value: Arc<str>,
    scheme: UriScheme,
}

impl ExternalUri {
    pub fn new(value: impl AsRef<str>) -> Result<Self, ExternalUriError> {
        let value = value.as_ref();
        if value.is_empty() {
            return Err(ExternalUriError::Empty);
        }
        if value.len() > MAX_EXTERNAL_URI_BYTES {
            return Err(ExternalUriError::TooLong {
                byte_len: value.len(),
                maximum_bytes: MAX_EXTERNAL_URI_BYTES,
            });
        }
        for (offset, byte) in value.bytes().enumerate() {
            if !byte.is_ascii() {
                return Err(ExternalUriError::NonAscii {
                    byte_offset: offset,
                });
            }
            if byte.is_ascii_control() || byte.is_ascii_whitespace() {
                return Err(ExternalUriError::WhitespaceOrControl {
                    byte_offset: offset,
                });
            }
        }

        let Some(separator) = value.find(':') else {
            return Err(ExternalUriError::MissingScheme);
        };
        let scheme =
            UriScheme::new(&value[..separator]).map_err(ExternalUriError::InvalidScheme)?;

        let bytes = value.as_bytes();
        let mut offset = 0;
        while offset < bytes.len() {
            let byte = bytes[offset];
            if byte == b'%' {
                if offset + 2 >= bytes.len()
                    || !bytes[offset + 1].is_ascii_hexdigit()
                    || !bytes[offset + 2].is_ascii_hexdigit()
                {
                    return Err(ExternalUriError::InvalidPercentEncoding {
                        byte_offset: offset,
                    });
                }
                offset += 3;
                continue;
            }
            if !is_uri_ascii_byte(byte) {
                return Err(ExternalUriError::InvalidCharacter {
                    byte_offset: offset,
                });
            }
            offset += 1;
        }

        Ok(Self {
            value: Arc::from(value),
            scheme,
        })
    }

    pub fn as_str(&self) -> &str {
        &self.value
    }

    pub const fn scheme(&self) -> &UriScheme {
        &self.scheme
    }

    pub fn byte_len(&self) -> usize {
        self.value.len()
    }
}

fn is_uri_ascii_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'-' | b'.'
                | b'_'
                | b'~'
                | b':'
                | b'/'
                | b'?'
                | b'#'
                | b'['
                | b']'
                | b'@'
                | b'!'
                | b'$'
                | b'&'
                | b'\''
                | b'('
                | b')'
                | b'*'
                | b'+'
                | b','
                | b';'
                | b'='
        )
}

impl fmt::Debug for ExternalUri {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExternalUri")
            .field("scheme", &self.scheme)
            .field("byte_len", &self.byte_len())
            .field("redacted", &true)
            .finish()
    }
}

/// Invalid external URI lexical envelope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExternalUriError {
    Empty,
    TooLong {
        byte_len: usize,
        maximum_bytes: usize,
    },
    NonAscii {
        byte_offset: usize,
    },
    WhitespaceOrControl {
        byte_offset: usize,
    },
    MissingScheme,
    InvalidScheme(UriSchemeError),
    InvalidCharacter {
        byte_offset: usize,
    },
    InvalidPercentEncoding {
        byte_offset: usize,
    },
}

impl fmt::Display for ExternalUriError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("external URI is empty"),
            Self::TooLong {
                byte_len,
                maximum_bytes,
            } => write!(
                formatter,
                "external URI contains {byte_len} bytes; maximum is {maximum_bytes}"
            ),
            Self::NonAscii { byte_offset } => {
                write!(
                    formatter,
                    "external URI contains non-ASCII data at byte {byte_offset}"
                )
            }
            Self::WhitespaceOrControl { byte_offset } => write!(
                formatter,
                "external URI contains whitespace or control data at byte {byte_offset}"
            ),
            Self::MissingScheme => formatter.write_str("external URI has no absolute scheme"),
            Self::InvalidScheme(error) => error.fmt(formatter),
            Self::InvalidCharacter { byte_offset } => {
                write!(
                    formatter,
                    "external URI contains an invalid byte at {byte_offset}"
                )
            }
            Self::InvalidPercentEncoding { byte_offset } => write!(
                formatter,
                "external URI contains an invalid percent escape at byte {byte_offset}"
            ),
        }
    }
}

impl Error for ExternalUriError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidScheme(error) => Some(error),
            _ => None,
        }
    }
}

/// The sole portable external-URI operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UriOperation {
    OpenExternal,
}

/// Per-scheme URI length bound advertised by an adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UriLimits {
    maximum_uri_bytes: NonZeroU32,
}

impl UriLimits {
    pub const fn new(maximum_uri_bytes: NonZeroU32) -> Result<Self, UriLimitError> {
        if maximum_uri_bytes.get() as usize > MAX_EXTERNAL_URI_BYTES {
            return Err(UriLimitError::UriByteLimitTooLarge);
        }
        Ok(Self { maximum_uri_bytes })
    }

    pub const fn maximum_uri_bytes(self) -> NonZeroU32 {
        self.maximum_uri_bytes
    }
}

impl Default for UriLimits {
    fn default() -> Self {
        Self {
            maximum_uri_bytes: NonZeroU32::new(MAX_EXTERNAL_URI_BYTES as u32)
                .expect("external URI hard bound is nonzero"),
        }
    }
}

/// Invalid adapter-advertised URI limit metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UriLimitError {
    UriByteLimitTooLarge,
}

impl fmt::Display for UriLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("URI byte limit exceeds the neutral hard bound")
    }
}

impl Error for UriLimitError {}

/// Capability descriptor for opening one exact normalized URI scheme.
pub type UriCapability = CapabilityDescriptor<UriOperation, UriLimits>;

/// One independently governed supported URI scheme.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct UriSchemeCapability {
    scheme: UriScheme,
    capability: UriCapability,
}

impl UriSchemeCapability {
    pub const fn new(scheme: UriScheme, capability: UriCapability) -> Self {
        Self { scheme, capability }
    }

    pub const fn scheme(&self) -> &UriScheme {
        &self.scheme
    }

    pub const fn capability(&self) -> &UriCapability {
        &self.capability
    }

    pub fn admits(&self, uri: &ExternalUri) -> bool {
        self.scheme == *uri.scheme()
            && uri.byte_len() <= self.capability.limits().maximum_uri_bytes().get() as usize
    }
}

/// Bounded supported-scheme capability publication in adapter preference order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UriCapabilities {
    schemes: Arc<[UriSchemeCapability]>,
}

impl UriCapabilities {
    pub fn new(schemes: Vec<UriSchemeCapability>) -> Result<Self, UriCapabilityError> {
        if schemes.len() > MAX_URI_SCHEMES {
            return Err(UriCapabilityError::TooManySchemes {
                supplied: schemes.len(),
                maximum: MAX_URI_SCHEMES,
            });
        }
        for (index, scheme) in schemes.iter().enumerate() {
            if schemes[..index]
                .iter()
                .any(|existing| existing.scheme == scheme.scheme)
            {
                return Err(UriCapabilityError::DuplicateScheme {
                    scheme: scheme.scheme.clone(),
                });
            }
        }
        Ok(Self {
            schemes: schemes.into(),
        })
    }

    pub fn schemes(&self) -> &[UriSchemeCapability] {
        &self.schemes
    }

    pub fn capability(&self, scheme: &UriScheme) -> Option<&UriCapability> {
        self.schemes
            .iter()
            .find(|candidate| candidate.scheme == *scheme)
            .map(UriSchemeCapability::capability)
    }

    pub fn supports(&self, scheme: &UriScheme) -> bool {
        self.capability(scheme).is_some()
    }

    pub fn len(&self) -> usize {
        self.schemes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.schemes.is_empty()
    }
}

/// Invalid supported-scheme capability publication.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum UriCapabilityError {
    TooManySchemes { supplied: usize, maximum: usize },
    DuplicateScheme { scheme: UriScheme },
}

impl fmt::Display for UriCapabilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManySchemes { supplied, maximum } => write!(
                formatter,
                "URI capability contains {supplied} schemes; maximum is {maximum}"
            ),
            Self::DuplicateScheme { scheme } => {
                write!(formatter, "URI capability repeats scheme {scheme}")
            }
        }
    }
}

impl Error for UriCapabilityError {}

/// One view-scoped external URI open intention.
///
/// A gesture grant, when present, is moved into the request and then consumed by service
/// admission. Debug output includes neither URI content nor grant data.
pub struct UriOpenRequest {
    view: ViewId,
    uri: ExternalUri,
    user_gesture: Option<UserGestureGrantHandle>,
}

impl UriOpenRequest {
    pub const fn new(view: ViewId, uri: ExternalUri) -> Self {
        Self {
            view,
            uri,
            user_gesture: None,
        }
    }

    pub fn with_user_gesture(
        view: ViewId,
        uri: ExternalUri,
        user_gesture: UserGestureGrantHandle,
    ) -> Self {
        Self {
            view,
            uri,
            user_gesture: Some(user_gesture),
        }
    }

    pub const fn view(&self) -> ViewId {
        self.view
    }

    pub const fn uri(&self) -> &ExternalUri {
        &self.uri
    }

    pub const fn has_user_gesture(&self) -> bool {
        self.user_gesture.is_some()
    }

    /// Consumes the request for adapter validation and execution.
    pub fn into_parts(self) -> (ViewId, ExternalUri, Option<UserGestureGrantHandle>) {
        (self.view, self.uri, self.user_gesture)
    }
}

impl fmt::Debug for UriOpenRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UriOpenRequest")
            .field("view", &self.view)
            .field("uri", &self.uri)
            .field("has_user_gesture", &self.user_gesture.is_some())
            .finish_non_exhaustive()
    }
}

/// Redacted completion metadata for one applied URI open request.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct UriOpenApplied {
    view: ViewId,
    scheme: UriScheme,
    uri_byte_len: usize,
}

impl UriOpenApplied {
    pub fn from_request(request: &UriOpenRequest) -> Self {
        Self {
            view: request.view,
            scheme: request.uri.scheme.clone(),
            uri_byte_len: request.uri.byte_len(),
        }
    }

    pub const fn view(&self) -> ViewId {
        self.view
    }

    pub const fn scheme(&self) -> &UriScheme {
        &self.scheme
    }

    pub const fn uri_byte_len(&self) -> usize {
        self.uri_byte_len
    }
}

/// Immediate rejection before an external URI request is admitted.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum UriAdmissionError {
    ViewUnavailable {
        view: ViewId,
    },
    SchemeUnsupported {
        scheme: UriScheme,
    },
    CapabilityChanged {
        scheme: UriScheme,
    },
    PermissionDenied {
        scheme: UriScheme,
    },
    UserGestureRequired {
        scheme: UriScheme,
    },
    InvalidUserGesture {
        scheme: UriScheme,
    },
    UriExceedsCapability {
        scheme: UriScheme,
        byte_len: usize,
        maximum_bytes: NonZeroU32,
    },
    CapacityExceeded,
}

impl fmt::Display for UriAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ViewUnavailable { view } => write!(formatter, "URI view {view} is unavailable"),
            Self::SchemeUnsupported { scheme } => {
                write!(formatter, "URI scheme {scheme} is unsupported")
            }
            Self::CapabilityChanged { scheme } => {
                write!(
                    formatter,
                    "URI scheme {scheme} capability changed before admission"
                )
            }
            Self::PermissionDenied { scheme } => {
                write!(formatter, "URI scheme {scheme} permission is denied")
            }
            Self::UserGestureRequired { scheme } => {
                write!(
                    formatter,
                    "URI scheme {scheme} requires a recent user gesture"
                )
            }
            Self::InvalidUserGesture { scheme } => {
                write!(
                    formatter,
                    "URI scheme {scheme} received an invalid user gesture"
                )
            }
            Self::UriExceedsCapability {
                scheme,
                byte_len,
                maximum_bytes,
            } => write!(
                formatter,
                "URI scheme {scheme} request contains {byte_len} bytes; capability maximum is {maximum_bytes}"
            ),
            Self::CapacityExceeded => formatter.write_str("URI admission capacity was exceeded"),
        }
    }
}

impl Error for UriAdmissionError {}

/// Linear admission result for one external URI open request.
pub type UriOpenAdmission = RequestAdmission<UriOpenApplied, UriAdmissionError>;

/// Narrow service surface for supported-scheme discovery and external-open admission.
pub trait UriService {
    fn capabilities(&self) -> Support<UriCapabilities>;

    fn open(&self, request: UriOpenRequest) -> UriOpenAdmission;
}

/// Type-level registry key for an owner-local external URI service handle.
pub enum UriServiceKey {}

impl ServiceKey for UriServiceKey {
    type Handle = Rc<dyn UriService>;
}

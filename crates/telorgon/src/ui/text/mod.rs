//! Neutral revisioned text storage, shaping, glyph atlases, and text-run caching.

mod atlas;
mod buffer;
mod cache;
mod composition;
mod edit;
mod error;
mod glyph;
mod navigation;
mod range;
mod retained;
mod session;
mod shaping;
mod snapshot;
mod style;

pub use atlas::{AtlasPageUpdate, GlyphAtlas, GlyphAtlasView};
pub use buffer::{TextBuffer, TextBufferError, TextChunk, TextChunks};
pub use cache::{RetainedTextSystem, TextCacheStats};
pub use composition::{TextCompositionCommand, TextCompositionError, TextCompositionKind};
pub use edit::{TextChange, TextEdit, TextEditBatch, TextEditError, TextEditOutcome};
pub use error::{TextError, TextResult};
pub use glyph::AtlasGlyph;
pub use navigation::{
    TEXT_SEGMENTATION_CRATE_VERSION, TEXT_SEGMENTATION_PROFILE, TEXT_SEGMENTATION_UNICODE_VERSION,
    TextNavigationDirection, TextNavigationUnit, TextSelectionAdjustment,
};
pub use range::{TextAffinity, TextOffset, TextRange, TextRangeError, TextRevision, TextSelection};
pub use retained::{RetainedTextRequest, RetainedTextRun, TextRunId, TextRunKey};
pub use session::{
    TextCapitalization, TextInputConfiguration, TextInputGeometry, TextInputPolicy,
    TextInputPurpose, TextInputRequest, TextInputResyncReason, TextInputSession, TextInputSnapshot,
    TextMultiline, TextReturnKeyAction, TextSessionCommand, TextSessionDelta,
    TextSessionDeltaOutcome, TextSessionId, TextSessionPhase, TextSessionStateError,
    TextSurroundingText, TextVirtualKeyboardPreference,
};
pub use shaping::{PreparedText, TextEngine, TextLayoutRequest};
pub use snapshot::TextSnapshot;
pub use style::ResolvedTextStyle;

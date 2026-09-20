//! Per-view cursor appearance, position, and scoped constraint service boundary.
//!
//! Cursor images use bounded straight-alpha sRGB RGBA8 pixels in physical-pixel dimensions.
//! Position requests are validated in view-logical coordinates against one exact retained metrics
//! revision. Confinement and lock are long-lived effects: successful completion returns an
//! adapter-owned [`CursorConstraintLeaseHandle`] whose concrete destructor releases that effect.
//!
//! This module owns no native cursor, raw handle, Winit value, current pointer position, animation
//! clock, callback, queue, executor, event loop, or implicit fallback.

use std::fmt;
use std::num::{NonZeroU16, NonZeroU32};
use std::rc::Rc;
use std::sync::Arc;

use crate::foundation::{PointF, SizeF};

use crate::platform::contracts::id::CursorConstraintLeaseId;
use crate::platform::contracts::services::{ServiceKey, ServiceUnavailable};
use crate::platform::contracts::{
    CapabilityDescriptor, CoordinateSpace, MetricsRevision, RequestAdmission, Support, ViewId,
    ViewSnapshot,
};

/// Maximum width or height of one custom cursor image in physical pixels.
pub const MAX_CUSTOM_CURSOR_DIMENSION: u16 = 2_048;
/// Maximum bytes retained by one straight-alpha RGBA8 cursor image.
pub const MAX_CUSTOM_CURSOR_IMAGE_BYTES: usize = 16 * 1024 * 1024;
/// Maximum frames retained by one custom cursor animation.
pub const MAX_CUSTOM_CURSOR_FRAMES: usize = 64;
/// Maximum bytes retained across one custom cursor animation.
pub const MAX_CUSTOM_CURSOR_ANIMATION_BYTES: usize = 16 * 1024 * 1024;
/// Maximum duration of one custom cursor animation frame.
pub const MAX_CUSTOM_CURSOR_FRAME_DURATION_MS: u32 = 60_000;
/// Maximum duration of one full custom cursor animation cycle.
pub const MAX_CUSTOM_CURSOR_ANIMATION_DURATION_MS: u64 = 10 * 60 * 1_000;

/// Platform-neutral standard cursor vocabulary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StandardCursor {
    #[default]
    Default,
    ContextMenu,
    Help,
    Pointer,
    Progress,
    Wait,
    Cell,
    Crosshair,
    Text,
    VerticalText,
    Alias,
    Copy,
    Move,
    NoDrop,
    NotAllowed,
    Grab,
    Grabbing,
    EResize,
    NResize,
    NeResize,
    NwResize,
    SResize,
    SeResize,
    SwResize,
    WResize,
    EwResize,
    NsResize,
    NeswResize,
    NwseResize,
    ColResize,
    RowResize,
    AllScroll,
    ZoomIn,
    ZoomOut,
    DndAsk,
    AllResize,
}

/// Preferred application-facing name for the semantic pointer shape vocabulary.
pub type PointerIcon = StandardCursor;

impl StandardCursor {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::ContextMenu => "context-menu",
            Self::Help => "help",
            Self::Pointer => "pointer",
            Self::Progress => "progress",
            Self::Wait => "wait",
            Self::Cell => "cell",
            Self::Crosshair => "crosshair",
            Self::Text => "text",
            Self::VerticalText => "vertical-text",
            Self::Alias => "alias",
            Self::Copy => "copy",
            Self::Move => "move",
            Self::NoDrop => "no-drop",
            Self::NotAllowed => "not-allowed",
            Self::Grab => "grab",
            Self::Grabbing => "grabbing",
            Self::EResize => "e-resize",
            Self::NResize => "n-resize",
            Self::NeResize => "ne-resize",
            Self::NwResize => "nw-resize",
            Self::SResize => "s-resize",
            Self::SeResize => "se-resize",
            Self::SwResize => "sw-resize",
            Self::WResize => "w-resize",
            Self::EwResize => "ew-resize",
            Self::NsResize => "ns-resize",
            Self::NeswResize => "nesw-resize",
            Self::NwseResize => "nwse-resize",
            Self::ColResize => "col-resize",
            Self::RowResize => "row-resize",
            Self::AllScroll => "all-scroll",
            Self::ZoomIn => "zoom-in",
            Self::ZoomOut => "zoom-out",
            Self::DndAsk => "dnd-ask",
            Self::AllResize => "all-resize",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "default" => Self::Default,
            "context-menu" => Self::ContextMenu,
            "help" => Self::Help,
            "pointer" => Self::Pointer,
            "progress" => Self::Progress,
            "wait" => Self::Wait,
            "cell" => Self::Cell,
            "crosshair" => Self::Crosshair,
            "text" => Self::Text,
            "vertical-text" => Self::VerticalText,
            "alias" => Self::Alias,
            "copy" => Self::Copy,
            "move" => Self::Move,
            "no-drop" => Self::NoDrop,
            "not-allowed" => Self::NotAllowed,
            "grab" => Self::Grab,
            "grabbing" => Self::Grabbing,
            "e-resize" => Self::EResize,
            "n-resize" => Self::NResize,
            "ne-resize" => Self::NeResize,
            "nw-resize" => Self::NwResize,
            "s-resize" => Self::SResize,
            "se-resize" => Self::SeResize,
            "sw-resize" => Self::SwResize,
            "w-resize" => Self::WResize,
            "ew-resize" => Self::EwResize,
            "ns-resize" => Self::NsResize,
            "nesw-resize" => Self::NeswResize,
            "nwse-resize" => Self::NwseResize,
            "col-resize" => Self::ColResize,
            "row-resize" => Self::RowResize,
            "all-scroll" => Self::AllScroll,
            "zoom-in" => Self::ZoomIn,
            "zoom-out" => Self::ZoomOut,
            "dnd-ask" => Self::DndAsk,
            "all-resize" => Self::AllResize,
            _ => return None,
        })
    }
}

/// One bounded physical-pixel custom cursor image.
///
/// Pixels are row-major, straight-alpha, sRGB RGBA8. Debug output never formats their content.
#[derive(Clone, PartialEq, Eq)]
pub struct CustomCursorImage {
    width: u16,
    height: u16,
    hotspot_x: u16,
    hotspot_y: u16,
    rgba8_srgb_straight: Arc<[u8]>,
}

impl CustomCursorImage {
    pub fn new(
        rgba8_srgb_straight: impl Into<Arc<[u8]>>,
        width: u16,
        height: u16,
        hotspot_x: u16,
        hotspot_y: u16,
    ) -> Result<Self, CursorImageError> {
        if width == 0 || height == 0 {
            return Err(CursorImageError::EmptyExtent);
        }
        if width > MAX_CUSTOM_CURSOR_DIMENSION || height > MAX_CUSTOM_CURSOR_DIMENSION {
            return Err(CursorImageError::DimensionTooLarge);
        }
        let expected_bytes = usize::from(width)
            .checked_mul(usize::from(height))
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(CursorImageError::ByteLengthMismatch)?;
        if expected_bytes > MAX_CUSTOM_CURSOR_IMAGE_BYTES {
            return Err(CursorImageError::ImageBytesTooLarge);
        }
        let rgba8_srgb_straight = rgba8_srgb_straight.into();
        if rgba8_srgb_straight.len() != expected_bytes {
            return Err(CursorImageError::ByteLengthMismatch);
        }
        if hotspot_x >= width || hotspot_y >= height {
            return Err(CursorImageError::HotspotOutOfBounds);
        }
        Ok(Self {
            width,
            height,
            hotspot_x,
            hotspot_y,
            rgba8_srgb_straight,
        })
    }

    pub const fn width(&self) -> u16 {
        self.width
    }

    pub const fn height(&self) -> u16 {
        self.height
    }

    pub const fn hotspot_x(&self) -> u16 {
        self.hotspot_x
    }

    pub const fn hotspot_y(&self) -> u16 {
        self.hotspot_y
    }

    pub fn rgba8_srgb_straight(&self) -> &[u8] {
        &self.rgba8_srgb_straight
    }

    pub fn byte_len(&self) -> usize {
        self.rgba8_srgb_straight.len()
    }
}

impl fmt::Debug for CustomCursorImage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CustomCursorImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("hotspot_x", &self.hotspot_x)
            .field("hotspot_y", &self.hotspot_y)
            .field("byte_len", &self.rgba8_srgb_straight.len())
            .finish_non_exhaustive()
    }
}

/// Invalid custom cursor image metadata or storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CursorImageError {
    EmptyExtent,
    DimensionTooLarge,
    ImageBytesTooLarge,
    ByteLengthMismatch,
    HotspotOutOfBounds,
    FrameDurationTooLong,
    AnimationNeedsMultipleFrames,
    TooManyAnimationFrames,
    AnimationGeometryMismatch,
    AnimationBytesTooLarge,
    AnimationDurationTooLong,
}

impl fmt::Display for CursorImageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyExtent => "custom cursor image extent must be nonzero",
            Self::DimensionTooLarge => "custom cursor image dimensions exceed the hard bound",
            Self::ImageBytesTooLarge => "custom cursor image bytes exceed the hard bound",
            Self::ByteLengthMismatch => {
                "custom cursor RGBA8 byte length does not match its dimensions"
            }
            Self::HotspotOutOfBounds => "custom cursor hotspot is outside the image",
            Self::FrameDurationTooLong => "custom cursor frame duration exceeds the hard bound",
            Self::AnimationNeedsMultipleFrames => {
                "custom cursor animation requires at least two frames"
            }
            Self::TooManyAnimationFrames => {
                "custom cursor animation frame count exceeds the hard bound"
            }
            Self::AnimationGeometryMismatch => {
                "custom cursor animation frames must share extent and hotspot"
            }
            Self::AnimationBytesTooLarge => "custom cursor animation bytes exceed the hard bound",
            Self::AnimationDurationTooLong => {
                "custom cursor animation cycle exceeds the hard duration bound"
            }
        })
    }
}

impl std::error::Error for CursorImageError {}

/// One image and nonzero display duration in a custom animation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorAnimationFrame {
    image: CustomCursorImage,
    duration_ms: NonZeroU32,
}

impl CursorAnimationFrame {
    pub fn new(
        image: CustomCursorImage,
        duration_ms: NonZeroU32,
    ) -> Result<Self, CursorImageError> {
        if duration_ms.get() > MAX_CUSTOM_CURSOR_FRAME_DURATION_MS {
            return Err(CursorImageError::FrameDurationTooLong);
        }
        Ok(Self { image, duration_ms })
    }

    pub const fn image(&self) -> &CustomCursorImage {
        &self.image
    }

    pub const fn duration_ms(&self) -> NonZeroU32 {
        self.duration_ms
    }
}

/// Bounded looping animation with one shared extent and hotspot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomCursorAnimation {
    frames: Arc<[CursorAnimationFrame]>,
    total_bytes: usize,
    cycle_duration_ms: u64,
}

impl CustomCursorAnimation {
    pub fn new(frames: Vec<CursorAnimationFrame>) -> Result<Self, CursorImageError> {
        if frames.len() < 2 {
            return Err(CursorImageError::AnimationNeedsMultipleFrames);
        }
        if frames.len() > MAX_CUSTOM_CURSOR_FRAMES {
            return Err(CursorImageError::TooManyAnimationFrames);
        }
        let first = frames.first().expect("two animation frames were required");
        if frames.iter().skip(1).any(|frame| {
            frame.image.width != first.image.width
                || frame.image.height != first.image.height
                || frame.image.hotspot_x != first.image.hotspot_x
                || frame.image.hotspot_y != first.image.hotspot_y
        }) {
            return Err(CursorImageError::AnimationGeometryMismatch);
        }
        let total_bytes = frames.iter().try_fold(0_usize, |total, frame| {
            total.checked_add(frame.image.byte_len())
        });
        let Some(total_bytes) = total_bytes else {
            return Err(CursorImageError::AnimationBytesTooLarge);
        };
        if total_bytes > MAX_CUSTOM_CURSOR_ANIMATION_BYTES {
            return Err(CursorImageError::AnimationBytesTooLarge);
        }
        let cycle_duration_ms = frames
            .iter()
            .map(|frame| u64::from(frame.duration_ms.get()))
            .sum::<u64>();
        if cycle_duration_ms > MAX_CUSTOM_CURSOR_ANIMATION_DURATION_MS {
            return Err(CursorImageError::AnimationDurationTooLong);
        }
        Ok(Self {
            frames: frames.into(),
            total_bytes,
            cycle_duration_ms,
        })
    }

    pub fn frames(&self) -> &[CursorAnimationFrame] {
        &self.frames
    }

    pub const fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    pub const fn cycle_duration_ms(&self) -> u64 {
        self.cycle_duration_ms
    }
}

/// Static or looping bounded custom cursor content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CustomCursor {
    Static(CustomCursorImage),
    Animated(CustomCursorAnimation),
}

impl CustomCursor {
    pub fn frame_count(&self) -> usize {
        match self {
            Self::Static(_) => 1,
            Self::Animated(animation) => animation.frames.len(),
        }
    }

    pub fn total_bytes(&self) -> usize {
        match self {
            Self::Static(image) => image.rgba8_srgb_straight.len(),
            Self::Animated(animation) => animation.total_bytes,
        }
    }

    pub fn width(&self) -> u16 {
        match self {
            Self::Static(image) => image.width,
            Self::Animated(animation) => animation.frames[0].image.width,
        }
    }

    pub fn height(&self) -> u16 {
        match self {
            Self::Static(image) => image.height,
            Self::Animated(animation) => animation.frames[0].image.height,
        }
    }
}

/// Standard or custom cursor selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CursorSelection {
    Standard(StandardCursor),
    Custom(CustomCursor),
}

impl Default for CursorSelection {
    fn default() -> Self {
        Self::Standard(StandardCursor::Default)
    }
}

/// Payload-free selection class for diagnostics and receipts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CursorSelectionKind {
    Standard,
    CustomStatic,
    CustomAnimated,
}

impl CursorSelection {
    pub const fn kind(&self) -> CursorSelectionKind {
        match self {
            Self::Standard(_) => CursorSelectionKind::Standard,
            Self::Custom(CustomCursor::Static(_)) => CursorSelectionKind::CustomStatic,
            Self::Custom(CustomCursor::Animated(_)) => CursorSelectionKind::CustomAnimated,
        }
    }
}

/// Exact cursor selection and visibility intent for one view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorAppearance {
    selection: CursorSelection,
    visible: bool,
}

impl Default for CursorAppearance {
    fn default() -> Self {
        Self {
            selection: CursorSelection::default(),
            visible: true,
        }
    }
}

impl CursorAppearance {
    pub const fn new(selection: CursorSelection, visible: bool) -> Self {
        Self { selection, visible }
    }

    pub const fn selection(&self) -> &CursorSelection {
        &self.selection
    }

    pub const fn visible(&self) -> bool {
        self.visible
    }
}

/// Independently discoverable cursor operations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CursorOperations {
    standard_selection: bool,
    custom_images: bool,
    custom_animation: bool,
    visibility: bool,
    logical_position: bool,
    confinement: bool,
    lock: bool,
}

impl CursorOperations {
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        standard_selection: bool,
        custom_images: bool,
        custom_animation: bool,
        visibility: bool,
        logical_position: bool,
        confinement: bool,
        lock: bool,
    ) -> Self {
        Self {
            standard_selection,
            custom_images,
            custom_animation,
            visibility,
            logical_position,
            confinement,
            lock,
        }
    }

    pub const fn supports_standard_selection(self) -> bool {
        self.standard_selection
    }

    pub const fn supports_custom_images(self) -> bool {
        self.custom_images
    }

    pub const fn supports_custom_animation(self) -> bool {
        self.custom_animation
    }

    pub const fn supports_visibility(self) -> bool {
        self.visibility
    }

    pub const fn supports_logical_position(self) -> bool {
        self.logical_position
    }

    pub const fn supports_confinement(self) -> bool {
        self.confinement
    }

    pub const fn supports_lock(self) -> bool {
        self.lock
    }
}

/// Host-advertised custom cursor bounds, capped by neutral hard limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CursorLimits {
    maximum_width: NonZeroU16,
    maximum_height: NonZeroU16,
    maximum_animation_frames: NonZeroU16,
    maximum_custom_bytes: NonZeroU32,
}

impl CursorLimits {
    pub const fn new(
        maximum_width: NonZeroU16,
        maximum_height: NonZeroU16,
        maximum_animation_frames: NonZeroU16,
        maximum_custom_bytes: NonZeroU32,
    ) -> Result<Self, CursorLimitError> {
        if maximum_width.get() > MAX_CUSTOM_CURSOR_DIMENSION
            || maximum_height.get() > MAX_CUSTOM_CURSOR_DIMENSION
        {
            return Err(CursorLimitError::DimensionLimitTooLarge);
        }
        if maximum_animation_frames.get() as usize > MAX_CUSTOM_CURSOR_FRAMES {
            return Err(CursorLimitError::FrameLimitTooLarge);
        }
        if maximum_custom_bytes.get() as usize > MAX_CUSTOM_CURSOR_ANIMATION_BYTES {
            return Err(CursorLimitError::ByteLimitTooLarge);
        }
        Ok(Self {
            maximum_width,
            maximum_height,
            maximum_animation_frames,
            maximum_custom_bytes,
        })
    }

    pub const fn maximum_width(self) -> NonZeroU16 {
        self.maximum_width
    }

    pub const fn maximum_height(self) -> NonZeroU16 {
        self.maximum_height
    }

    pub const fn maximum_animation_frames(self) -> NonZeroU16 {
        self.maximum_animation_frames
    }

    pub const fn maximum_custom_bytes(self) -> NonZeroU32 {
        self.maximum_custom_bytes
    }

    pub fn admits(self, selection: &CursorSelection) -> bool {
        match selection {
            CursorSelection::Standard(_) => true,
            CursorSelection::Custom(custom) => {
                custom.width() <= self.maximum_width.get()
                    && custom.height() <= self.maximum_height.get()
                    && custom.frame_count() <= self.maximum_animation_frames.get() as usize
                    && custom.total_bytes() <= self.maximum_custom_bytes.get() as usize
            }
        }
    }
}

impl Default for CursorLimits {
    fn default() -> Self {
        Self {
            maximum_width: NonZeroU16::new(MAX_CUSTOM_CURSOR_DIMENSION)
                .expect("custom cursor dimension hard bound is nonzero"),
            maximum_height: NonZeroU16::new(MAX_CUSTOM_CURSOR_DIMENSION)
                .expect("custom cursor dimension hard bound is nonzero"),
            maximum_animation_frames: NonZeroU16::new(MAX_CUSTOM_CURSOR_FRAMES as u16)
                .expect("custom cursor frame hard bound is nonzero"),
            maximum_custom_bytes: NonZeroU32::new(MAX_CUSTOM_CURSOR_ANIMATION_BYTES as u32)
                .expect("custom cursor byte hard bound is nonzero"),
        }
    }
}

/// Invalid host-advertised cursor limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CursorLimitError {
    DimensionLimitTooLarge,
    FrameLimitTooLarge,
    ByteLimitTooLarge,
}

impl fmt::Display for CursorLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::DimensionLimitTooLarge => "cursor dimension limit exceeds the neutral hard bound",
            Self::FrameLimitTooLarge => "cursor frame limit exceeds the neutral hard bound",
            Self::ByteLimitTooLarge => "cursor byte limit exceeds the neutral hard bound",
        })
    }
}

impl std::error::Error for CursorLimitError {}

/// Complete cursor capability returned for one live view.
pub type CursorCapability = CapabilityDescriptor<CursorOperations, CursorLimits>;

/// Scope for one cursor capability query.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CursorCapabilityQuery {
    view: ViewId,
}

impl CursorCapabilityQuery {
    pub const fn new(view: ViewId) -> Self {
        Self { view }
    }

    pub const fn view(self) -> ViewId {
        self.view
    }
}

/// One view-scoped cursor appearance request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorAppearanceRequest {
    view: ViewId,
    appearance: CursorAppearance,
}

impl CursorAppearanceRequest {
    pub const fn new(view: ViewId, appearance: CursorAppearance) -> Self {
        Self { view, appearance }
    }

    pub const fn view(&self) -> ViewId {
        self.view
    }

    pub const fn appearance(&self) -> &CursorAppearance {
        &self.appearance
    }
}

/// Payload-free completion metadata for one applied appearance request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CursorAppearanceApplied {
    view: ViewId,
    selection_kind: CursorSelectionKind,
    visible: bool,
}

impl CursorAppearanceApplied {
    pub const fn from_request(request: &CursorAppearanceRequest) -> Self {
        Self {
            view: request.view,
            selection_kind: request.appearance.selection.kind(),
            visible: request.appearance.visible,
        }
    }

    pub const fn view(self) -> ViewId {
        self.view
    }

    pub const fn selection_kind(self) -> CursorSelectionKind {
        self.selection_kind
    }

    pub const fn visible(self) -> bool {
        self.visible
    }
}

/// One view-logical position request validated against an exact metrics publication.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CursorPositionRequest {
    view: ViewId,
    metrics_revision: MetricsRevision,
    position: PointF,
}

impl CursorPositionRequest {
    pub fn new(snapshot: &ViewSnapshot, position: PointF) -> Result<Self, CursorPositionError> {
        if !position.x.is_finite() || !position.y.is_finite() {
            return Err(CursorPositionError::NonFinitePosition);
        }
        let logical_extent = snapshot.metrics().metrics().logical_extent();
        if position.x < 0.0
            || position.y < 0.0
            || position.x >= logical_extent.width
            || position.y >= logical_extent.height
        {
            return Err(CursorPositionError::OutsideView { logical_extent });
        }
        Ok(Self {
            view: snapshot.view(),
            metrics_revision: snapshot.metrics().revision(),
            position,
        })
    }

    pub const fn view(self) -> ViewId {
        self.view
    }

    pub const fn metrics_revision(self) -> MetricsRevision {
        self.metrics_revision
    }

    pub const fn position(self) -> PointF {
        self.position
    }

    pub const fn coordinate_space(self) -> CoordinateSpace {
        CoordinateSpace::ViewLogical
    }
}

/// Invalid cursor position before service admission.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CursorPositionError {
    NonFinitePosition,
    OutsideView { logical_extent: SizeF },
}

impl fmt::Display for CursorPositionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFinitePosition => "cursor position must be finite",
            Self::OutsideView { .. } => "cursor position is outside the cited view-logical extent",
        })
    }
}

impl std::error::Error for CursorPositionError {}

/// Completion metadata for one applied view-logical position request.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CursorPositionApplied {
    view: ViewId,
    metrics_revision: MetricsRevision,
    position: PointF,
}

impl CursorPositionApplied {
    pub const fn from_request(request: CursorPositionRequest) -> Self {
        Self {
            view: request.view,
            metrics_revision: request.metrics_revision,
            position: request.position,
        }
    }

    pub const fn view(self) -> ViewId {
        self.view
    }

    pub const fn metrics_revision(self) -> MetricsRevision {
        self.metrics_revision
    }

    pub const fn position(self) -> PointF {
        self.position
    }
}

/// Long-lived cursor restriction requested for one view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CursorConstraintKind {
    Confined,
    Locked,
}

/// Request for one adapter-owned scoped cursor restriction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CursorConstraintRequest {
    view: ViewId,
    kind: CursorConstraintKind,
}

impl CursorConstraintRequest {
    pub const fn new(view: ViewId, kind: CursorConstraintKind) -> Self {
        Self { view, kind }
    }

    pub const fn view(self) -> ViewId {
        self.view
    }

    pub const fn kind(self) -> CursorConstraintKind {
        self.kind
    }
}

/// Why a formerly active cursor constraint no longer applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CursorConstraintRevocation {
    ViewClosed,
    ViewSuspended,
    FocusLost,
    HostRevoked,
}

/// Current adapter-reported state of a cursor constraint lease.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CursorConstraintLeaseStatus {
    Active,
    Revoked(CursorConstraintRevocation),
}

/// Adapter-owned RAII lease for one active cursor confinement or lock.
///
/// Concrete implementations must release the native effect from `Drop`. The lease is returned in
/// a `Box` and is intentionally not cloneable; view closure, suspension, focus loss, and host
/// revocation may change [`Self::status`] before it is dropped.
pub trait CursorConstraintLease: fmt::Debug {
    fn id(&self) -> CursorConstraintLeaseId;
    fn view(&self) -> ViewId;
    fn kind(&self) -> CursorConstraintKind;
    fn status(&self) -> CursorConstraintLeaseStatus;
}

/// Non-cloneable owner of one adapter-provided constraint lease.
pub type CursorConstraintLeaseHandle = Box<dyn CursorConstraintLease>;

/// Immediate rejection before a cursor operation is admitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorAdmissionError {
    ServiceUnavailable(ServiceUnavailable),
    ViewUnavailable {
        view: ViewId,
    },
    StaleMetrics {
        view: ViewId,
        expected: MetricsRevision,
        observed: MetricsRevision,
    },
    Unsupported,
    Denied,
    UserGestureRequired,
    CustomCursorExceedsLimits,
    ConstraintAlreadyActive {
        view: ViewId,
    },
    CapacityExceeded,
}

impl fmt::Display for CursorAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ServiceUnavailable(reason) => {
                write!(formatter, "cursor service is unavailable: {reason:?}")
            }
            Self::ViewUnavailable { view } => {
                write!(formatter, "cursor view {view} is unavailable")
            }
            Self::StaleMetrics {
                view,
                expected,
                observed,
            } => write!(
                formatter,
                "cursor view {view} expected metrics revision {expected}, observed {observed}"
            ),
            Self::Unsupported => formatter.write_str("cursor operation is unsupported"),
            Self::Denied => formatter.write_str("cursor operation was denied"),
            Self::UserGestureRequired => {
                formatter.write_str("cursor operation requires a recent user gesture")
            }
            Self::CustomCursorExceedsLimits => {
                formatter.write_str("custom cursor exceeds host-advertised limits")
            }
            Self::ConstraintAlreadyActive { view } => {
                write!(
                    formatter,
                    "cursor constraint is already active for view {view}"
                )
            }
            Self::CapacityExceeded => formatter.write_str("cursor admission capacity was exceeded"),
        }
    }
}

impl std::error::Error for CursorAdmissionError {}

pub type CursorAppearanceAdmission =
    RequestAdmission<CursorAppearanceApplied, CursorAdmissionError>;
pub type CursorPositionAdmission = RequestAdmission<CursorPositionApplied, CursorAdmissionError>;
pub type CursorConstraintAdmission =
    RequestAdmission<CursorConstraintLeaseHandle, CursorAdmissionError>;

/// Narrow cursor service surface. Every operation is scoped to one view generation.
pub trait CursorService {
    fn capability(&self, query: CursorCapabilityQuery) -> Support<CursorCapability>;

    fn set_appearance(&self, request: CursorAppearanceRequest) -> CursorAppearanceAdmission;

    fn set_position(&self, request: CursorPositionRequest) -> CursorPositionAdmission;

    fn acquire_constraint(&self, request: CursorConstraintRequest) -> CursorConstraintAdmission;
}

/// Type-level registry key for an owner-local cursor service handle.
pub enum CursorServiceKey {}

impl ServiceKey for CursorServiceKey {
    type Handle = Rc<dyn CursorService>;
}

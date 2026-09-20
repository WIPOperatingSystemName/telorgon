//! Immutable, revisioned client-surface snapshots supplied by a shell policy host.

use std::fmt;
use std::num::NonZeroU64;
use std::sync::Arc;

use crate::foundation::{RectF, RectI, SizeI};

use crate::shell::{ApplicationId, SurfaceId};

macro_rules! define_nonzero_value {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[repr(transparent)]
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(NonZeroU64);

        impl $name {
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
    };
}

define_nonzero_value!(
    SurfaceRevision,
    "Monotonic revision of one surface snapshot."
);
define_nonzero_value!(
    SurfaceContentRevision,
    "Monotonic host revision of the external content attached to a surface."
);
define_nonzero_value!(
    ExternalContentId,
    "Opaque logical external-content identity resolved only by a host/backend adapter."
);
define_nonzero_value!(
    SurfaceSynchronizationRef,
    "Opaque logical acquire/release synchronization reference."
);

impl fmt::Debug for SurfaceRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("SurfaceRevision")
            .field(&self.get())
            .finish()
    }
}

impl fmt::Debug for SurfaceContentRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("SurfaceContentRevision")
            .field(&self.get())
            .finish()
    }
}

impl fmt::Debug for ExternalContentId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ExternalContentId(..)")
    }
}

impl fmt::Debug for SurfaceSynchronizationRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SurfaceSynchronizationRef(..)")
    }
}

/// Buffer orientation/reflection relative to surface-local logical coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SurfaceBufferTransform {
    #[default]
    Normal,
    Rotate90,
    Rotate180,
    Rotate270,
    Flipped,
    Flipped90,
    Flipped180,
    Flipped270,
}

impl SurfaceBufferTransform {
    pub const fn swaps_axes(self) -> bool {
        matches!(
            self,
            Self::Rotate90 | Self::Rotate270 | Self::Flipped90 | Self::Flipped270
        )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SurfaceColorPrimaries {
    #[default]
    Srgb,
    DisplayP3,
    Rec2020,
    HostDefined,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SurfaceTransferFunction {
    #[default]
    Srgb,
    Linear,
    Pq,
    Hlg,
    HostDefined,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SurfaceColorRange {
    #[default]
    Full,
    Limited,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SurfaceColorDescription {
    pub primaries: SurfaceColorPrimaries,
    pub transfer: SurfaceTransferFunction,
    pub range: SurfaceColorRange,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SurfaceAlphaMode {
    Opaque,
    Straight,
    #[default]
    Premultiplied,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SurfaceSampling {
    Nearest,
    #[default]
    Linear,
}

/// Host assertion about protected content. Enforcement remains a host/backend responsibility.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SurfaceProtection {
    #[default]
    Unprotected,
    Protected,
}

/// One revision of a logical external-content attachment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SurfaceContent {
    id: ExternalContentId,
    revision: SurfaceContentRevision,
    synchronization: Option<SurfaceSynchronizationRef>,
    color: SurfaceColorDescription,
    alpha: SurfaceAlphaMode,
    sampling: SurfaceSampling,
    protection: SurfaceProtection,
}

impl SurfaceContent {
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        id: ExternalContentId,
        revision: SurfaceContentRevision,
        synchronization: Option<SurfaceSynchronizationRef>,
        color: SurfaceColorDescription,
        alpha: SurfaceAlphaMode,
        sampling: SurfaceSampling,
        protection: SurfaceProtection,
    ) -> Self {
        Self {
            id,
            revision,
            synchronization,
            color,
            alpha,
            sampling,
            protection,
        }
    }

    pub const fn id(self) -> ExternalContentId {
        self.id
    }

    pub const fn revision(self) -> SurfaceContentRevision {
        self.revision
    }

    pub const fn synchronization(self) -> Option<SurfaceSynchronizationRef> {
        self.synchronization
    }

    pub const fn color(self) -> SurfaceColorDescription {
        self.color
    }

    pub const fn alpha(self) -> SurfaceAlphaMode {
        self.alpha
    }

    pub const fn sampling(self) -> SurfaceSampling {
        self.sampling
    }

    pub const fn protection(self) -> SurfaceProtection {
        self.protection
    }
}

/// Bounded logical region in surface-local coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceRegion(Arc<[RectF]>);

impl SurfaceRegion {
    pub const MAX_RECTS: usize = 64;

    pub fn empty() -> Self {
        Self(Arc::from([]))
    }

    pub fn new(rects: Vec<RectF>) -> Result<Self, SurfaceRegionError> {
        if rects.len() > Self::MAX_RECTS {
            return Err(SurfaceRegionError::TooManyRects {
                count: rects.len(),
                max: Self::MAX_RECTS,
            });
        }
        if let Some(index) = rects.iter().position(|rect| !valid_positive_rect(*rect)) {
            return Err(SurfaceRegionError::InvalidRect { index });
        }
        Ok(Self(rects.into()))
    }

    pub fn as_slice(&self) -> &[RectF] {
        &self.0
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = RectF> + '_ {
        self.0.iter().copied()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Default for SurfaceRegion {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SurfaceRegionError {
    TooManyRects { count: usize, max: usize },
    InvalidRect { index: usize },
}

impl fmt::Display for SurfaceRegionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyRects { count, max } => {
                write!(
                    formatter,
                    "surface region has {count} rectangles; maximum is {max}"
                )
            }
            Self::InvalidRect { index } => {
                write!(
                    formatter,
                    "surface region rectangle {index} must be finite and positive"
                )
            }
        }
    }
}

impl std::error::Error for SurfaceRegionError {}

/// Logical clip, opaque, and input regions for one surface revision.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SurfaceRegions {
    clip: Option<SurfaceRegion>,
    opaque: SurfaceRegion,
    input: SurfaceRegion,
}

impl SurfaceRegions {
    pub const fn new(
        clip: Option<SurfaceRegion>,
        opaque: SurfaceRegion,
        input: SurfaceRegion,
    ) -> Self {
        Self {
            clip,
            opaque,
            input,
        }
    }

    pub fn clip(&self) -> Option<&SurfaceRegion> {
        self.clip.as_ref()
    }

    pub const fn opaque(&self) -> &SurfaceRegion {
        &self.opaque
    }

    pub const fn input(&self) -> &SurfaceRegion {
        &self.input
    }
}

/// Bounded damage in buffer-pixel coordinates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurfaceDamage(Arc<[RectI]>);

impl SurfaceDamage {
    pub const MAX_RECTS: usize = 128;

    pub fn empty() -> Self {
        Self(Arc::from([]))
    }

    pub fn new(rects: Vec<RectI>) -> Result<Self, SurfaceDamageError> {
        if rects.len() > Self::MAX_RECTS {
            return Err(SurfaceDamageError::TooManyRects {
                count: rects.len(),
                max: Self::MAX_RECTS,
            });
        }
        if let Some(index) = rects.iter().position(|rect| {
            rect.x < 0
                || rect.y < 0
                || rect.width <= 0
                || rect.height <= 0
                || rect.x.checked_add(rect.width).is_none()
                || rect.y.checked_add(rect.height).is_none()
        }) {
            return Err(SurfaceDamageError::InvalidRect { index });
        }
        Ok(Self(rects.into()))
    }

    pub fn as_slice(&self) -> &[RectI] {
        &self.0
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = RectI> + '_ {
        self.0.iter().copied()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Default for SurfaceDamage {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SurfaceDamageError {
    TooManyRects { count: usize, max: usize },
    InvalidRect { index: usize },
}

impl fmt::Display for SurfaceDamageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyRects { count, max } => {
                write!(
                    formatter,
                    "surface damage has {count} rectangles; maximum is {max}"
                )
            }
            Self::InvalidRect { index } => write!(
                formatter,
                "surface damage rectangle {index} must be positive buffer-pixel geometry"
            ),
        }
    }
}

impl std::error::Error for SurfaceDamageError {}

/// Validated placement, buffer geometry, transform, and opacity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceGeometry {
    logical_bounds: RectF,
    buffer_size: SizeI,
    buffer_scale: f32,
    transform: SurfaceBufferTransform,
    opacity: f32,
}

impl SurfaceGeometry {
    pub fn new(
        logical_bounds: RectF,
        buffer_size: SizeI,
        buffer_scale: f32,
        transform: SurfaceBufferTransform,
        opacity: f32,
    ) -> Result<Self, SurfaceGeometryError> {
        if !valid_positive_rect(logical_bounds) {
            return Err(SurfaceGeometryError::InvalidLogicalBounds);
        }
        if buffer_size.width <= 0 || buffer_size.height <= 0 {
            return Err(SurfaceGeometryError::InvalidBufferSize);
        }
        if !buffer_scale.is_finite() || buffer_scale <= 0.0 {
            return Err(SurfaceGeometryError::InvalidBufferScale);
        }
        if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
            return Err(SurfaceGeometryError::InvalidOpacity);
        }
        Ok(Self {
            logical_bounds,
            buffer_size,
            buffer_scale,
            transform,
            opacity,
        })
    }

    pub const fn logical_bounds(self) -> RectF {
        self.logical_bounds
    }

    pub const fn buffer_size(self) -> SizeI {
        self.buffer_size
    }

    pub const fn buffer_scale(self) -> f32 {
        self.buffer_scale
    }

    pub const fn transform(self) -> SurfaceBufferTransform {
        self.transform
    }

    pub const fn opacity(self) -> f32 {
        self.opacity
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceGeometryError {
    InvalidLogicalBounds,
    InvalidBufferSize,
    InvalidBufferScale,
    InvalidOpacity,
}

impl fmt::Display for SurfaceGeometryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidLogicalBounds => "surface logical bounds must be finite and positive",
            Self::InvalidBufferSize => "surface buffer size must be positive",
            Self::InvalidBufferScale => "surface buffer scale must be finite and positive",
            Self::InvalidOpacity => "surface opacity must be finite and between zero and one",
        })
    }
}

impl std::error::Error for SurfaceGeometryError {}

#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct SurfaceCapabilities(u16);

impl SurfaceCapabilities {
    pub const NONE: Self = Self(0);
    pub const ACTIVATE: Self = Self(1 << 0);
    pub const CLOSE: Self = Self(1 << 1);
    pub const MOVE: Self = Self(1 << 2);
    pub const RESIZE: Self = Self(1 << 3);
    pub const MINIMIZE: Self = Self(1 << 4);
    pub const MAXIMIZE: Self = Self(1 << 5);
    pub const FULLSCREEN: Self = Self(1 << 6);
    const ALL_BITS: u16 = (1 << 7) - 1;

    pub const fn from_bits(bits: u16) -> Option<Self> {
        if bits & !Self::ALL_BITS == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl fmt::Debug for SurfaceCapabilities {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SurfaceCapabilities")
            .field("bits", &format_args!("{:#09b}", self.bits()))
            .finish()
    }
}

impl std::ops::BitOr for SurfaceCapabilities {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        self.union(rhs)
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct SurfaceStates(u8);

impl SurfaceStates {
    pub const NONE: Self = Self(0);
    pub const ACTIVE: Self = Self(1 << 0);
    pub const FOCUSED: Self = Self(1 << 1);
    pub const MINIMIZED: Self = Self(1 << 2);
    pub const MAXIMIZED: Self = Self(1 << 3);
    pub const FULLSCREEN: Self = Self(1 << 4);
    pub const URGENT: Self = Self(1 << 5);
    const ALL_BITS: u8 = (1 << 6) - 1;

    pub const fn from_bits(bits: u8) -> Option<Self> {
        if bits & !Self::ALL_BITS == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl fmt::Debug for SurfaceStates {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SurfaceStates")
            .field("bits", &format_args!("{:#08b}", self.bits()))
            .finish()
    }
}

impl std::ops::BitOr for SurfaceStates {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        self.union(rhs)
    }
}

/// Optional host-provided title for safe window-level presentation.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SurfaceTitle(Box<str>);

impl SurfaceTitle {
    pub const MAX_BYTES: usize = 512;

    pub fn new(value: impl AsRef<str>) -> Result<Self, SurfaceTitleError> {
        let value = value.as_ref();
        if value.trim().is_empty() || value.len() > Self::MAX_BYTES {
            return Err(SurfaceTitleError::InvalidTitle);
        }
        Ok(Self(value.into()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SurfaceTitle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SurfaceTitle(<redacted>)")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceTitleError {
    InvalidTitle,
}

impl fmt::Display for SurfaceTitleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("surface title must be nonempty and at most 512 bytes")
    }
}

impl std::error::Error for SurfaceTitleError {}

/// Complete immutable host truth for one client surface revision.
#[derive(Clone, Debug, PartialEq)]
pub struct ClientSurfaceSnapshot {
    id: SurfaceId,
    revision: SurfaceRevision,
    parent: Option<SurfaceId>,
    stacking_order: i32,
    application: Option<ApplicationId>,
    title: Option<SurfaceTitle>,
    geometry: SurfaceGeometry,
    regions: SurfaceRegions,
    damage: SurfaceDamage,
    content: SurfaceContent,
    capabilities: SurfaceCapabilities,
    states: SurfaceStates,
}

impl ClientSurfaceSnapshot {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: SurfaceId,
        revision: SurfaceRevision,
        parent: Option<SurfaceId>,
        stacking_order: i32,
        application: Option<ApplicationId>,
        title: Option<SurfaceTitle>,
        geometry: SurfaceGeometry,
        regions: SurfaceRegions,
        damage: SurfaceDamage,
        content: SurfaceContent,
        capabilities: SurfaceCapabilities,
        states: SurfaceStates,
    ) -> Result<Self, ClientSurfaceError> {
        if parent == Some(id) {
            return Err(ClientSurfaceError::SelfParent);
        }
        let local_bounds = RectF {
            x: 0.0,
            y: 0.0,
            width: geometry.logical_bounds.width,
            height: geometry.logical_bounds.height,
        };
        for (kind, region) in [
            (SurfaceRegionKind::Clip, regions.clip()),
            (SurfaceRegionKind::Opaque, Some(regions.opaque())),
            (SurfaceRegionKind::Input, Some(regions.input())),
        ] {
            if let Some(region) = region
                && let Some(index) = region
                    .iter()
                    .position(|rect| !contains_rect(local_bounds, rect))
            {
                return Err(ClientSurfaceError::RegionOutsideLogicalBounds { kind, index });
            }
        }
        if let Some(index) = damage.iter().position(|rect| {
            rect.right() > geometry.buffer_size.width || rect.bottom() > geometry.buffer_size.height
        }) {
            return Err(ClientSurfaceError::DamageOutsideBuffer { index });
        }

        Ok(Self {
            id,
            revision,
            parent,
            stacking_order,
            application,
            title,
            geometry,
            regions,
            damage,
            content,
            capabilities,
            states,
        })
    }

    pub const fn id(&self) -> SurfaceId {
        self.id
    }

    pub const fn revision(&self) -> SurfaceRevision {
        self.revision
    }

    pub const fn parent(&self) -> Option<SurfaceId> {
        self.parent
    }

    pub const fn stacking_order(&self) -> i32 {
        self.stacking_order
    }

    pub const fn application(&self) -> Option<ApplicationId> {
        self.application
    }

    pub fn title(&self) -> Option<&SurfaceTitle> {
        self.title.as_ref()
    }

    pub const fn geometry(&self) -> SurfaceGeometry {
        self.geometry
    }

    pub const fn regions(&self) -> &SurfaceRegions {
        &self.regions
    }

    pub const fn damage(&self) -> &SurfaceDamage {
        &self.damage
    }

    pub const fn content(&self) -> SurfaceContent {
        self.content
    }

    pub const fn capabilities(&self) -> SurfaceCapabilities {
        self.capabilities
    }

    pub const fn states(&self) -> SurfaceStates {
        self.states
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceRegionKind {
    Clip,
    Opaque,
    Input,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientSurfaceError {
    SelfParent,
    RegionOutsideLogicalBounds {
        kind: SurfaceRegionKind,
        index: usize,
    },
    DamageOutsideBuffer {
        index: usize,
    },
}

impl fmt::Display for ClientSurfaceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SelfParent => formatter.write_str("a surface cannot be its own parent"),
            Self::RegionOutsideLogicalBounds { kind, index } => write!(
                formatter,
                "{kind:?} surface region rectangle {index} exceeds local logical bounds"
            ),
            Self::DamageOutsideBuffer { index } => {
                write!(
                    formatter,
                    "surface damage rectangle {index} exceeds the buffer"
                )
            }
        }
    }
}

impl std::error::Error for ClientSurfaceError {}

fn valid_positive_rect(rect: RectF) -> bool {
    rect.x.is_finite()
        && rect.y.is_finite()
        && rect.width.is_finite()
        && rect.width > 0.0
        && rect.height.is_finite()
        && rect.height > 0.0
        && rect.right().is_finite()
        && rect.bottom().is_finite()
}

fn contains_rect(outer: RectF, inner: RectF) -> bool {
    inner.x >= outer.x
        && inner.y >= outer.y
        && inner.right() <= outer.right()
        && inner.bottom() <= outer.bottom()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry() -> SurfaceGeometry {
        SurfaceGeometry::new(
            RectF {
                x: 20.0,
                y: 40.0,
                width: 800.0,
                height: 600.0,
            },
            SizeI {
                width: 1600,
                height: 1200,
            },
            2.0,
            SurfaceBufferTransform::Normal,
            0.9,
        )
        .unwrap()
    }

    fn content(protection: SurfaceProtection) -> SurfaceContent {
        SurfaceContent::new(
            ExternalContentId::from_raw(3).unwrap(),
            SurfaceContentRevision::from_raw(4).unwrap(),
            Some(SurfaceSynchronizationRef::from_raw(5).unwrap()),
            SurfaceColorDescription::default(),
            SurfaceAlphaMode::Premultiplied,
            SurfaceSampling::Linear,
            protection,
        )
    }

    #[test]
    fn snapshot_preserves_host_parent_geometry_content_and_policy_facts() {
        let id = SurfaceId::from_raw(10).unwrap();
        let parent = SurfaceId::from_raw(9).unwrap();
        let snapshot = ClientSurfaceSnapshot::new(
            id,
            SurfaceRevision::from_raw(7).unwrap(),
            Some(parent),
            2,
            Some(ApplicationId::from_raw(6).unwrap()),
            Some(SurfaceTitle::new("Terminal").unwrap()),
            geometry(),
            SurfaceRegions::default(),
            SurfaceDamage::new(vec![RectI {
                x: 0,
                y: 0,
                width: 100,
                height: 80,
            }])
            .unwrap(),
            content(SurfaceProtection::Protected),
            SurfaceCapabilities::CLOSE | SurfaceCapabilities::RESIZE,
            SurfaceStates::ACTIVE | SurfaceStates::FOCUSED,
        )
        .unwrap();

        assert_eq!(snapshot.parent(), Some(parent));
        assert_eq!(snapshot.geometry().buffer_scale(), 2.0);
        assert_eq!(
            snapshot.content().protection(),
            SurfaceProtection::Protected
        );
        assert!(snapshot.capabilities().contains(SurfaceCapabilities::CLOSE));
        assert!(snapshot.states().contains(SurfaceStates::FOCUSED));
        assert_eq!(snapshot.title().unwrap().as_str(), "Terminal");
    }

    #[test]
    fn contradictory_parent_region_and_damage_are_rejected() {
        let id = SurfaceId::from_raw(10).unwrap();
        assert_eq!(
            ClientSurfaceSnapshot::new(
                id,
                SurfaceRevision::from_raw(1).unwrap(),
                Some(id),
                0,
                None,
                None,
                geometry(),
                SurfaceRegions::default(),
                SurfaceDamage::default(),
                content(SurfaceProtection::Unprotected),
                SurfaceCapabilities::NONE,
                SurfaceStates::NONE,
            ),
            Err(ClientSurfaceError::SelfParent)
        );

        let damage = SurfaceDamage::new(vec![RectI {
            x: 1590,
            y: 0,
            width: 20,
            height: 20,
        }])
        .unwrap();
        assert!(matches!(
            ClientSurfaceSnapshot::new(
                id,
                SurfaceRevision::from_raw(1).unwrap(),
                None,
                0,
                None,
                None,
                geometry(),
                SurfaceRegions::default(),
                damage,
                content(SurfaceProtection::Unprotected),
                SurfaceCapabilities::NONE,
                SurfaceStates::NONE,
            ),
            Err(ClientSurfaceError::DamageOutsideBuffer { index: 0 })
        ));
    }

    #[test]
    fn region_validation_is_bounded_without_rewriting_host_rectangles() {
        let same = RectF {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        };
        let region = SurfaceRegion::new(vec![same, same]).unwrap();
        assert_eq!(region.len(), 2);
        assert_eq!(region.as_slice(), &[same, same]);
    }
}

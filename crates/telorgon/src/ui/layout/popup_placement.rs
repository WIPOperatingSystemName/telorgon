//! Pure popup placement over anchor, safe-bounds, and occlusion geometry.

use std::fmt;

use crate::foundation::{PointF, RectF, SizeF};
use crate::input::WritingDirection;

pub const MAX_POPUP_OCCLUSIONS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PopupPlacementSide {
    Above,
    Below,
    InlineStart,
    InlineEnd,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PopupPlacementAlignment {
    #[default]
    Start,
    Center,
    End,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PopupPlacementCandidate {
    pub side: PopupPlacementSide,
    pub alignment: PopupPlacementAlignment,
}

impl PopupPlacementCandidate {
    pub const fn new(side: PopupPlacementSide, alignment: PopupPlacementAlignment) -> Self {
        Self { side, alignment }
    }

    pub const fn below(alignment: PopupPlacementAlignment) -> Self {
        Self::new(PopupPlacementSide::Below, alignment)
    }

    pub const fn above(alignment: PopupPlacementAlignment) -> Self {
        Self::new(PopupPlacementSide::Above, alignment)
    }

    pub const fn inline_start(alignment: PopupPlacementAlignment) -> Self {
        Self::new(PopupPlacementSide::InlineStart, alignment)
    }

    pub const fn inline_end(alignment: PopupPlacementAlignment) -> Self {
        Self::new(PopupPlacementSide::InlineEnd, alignment)
    }
}

/// Explicit fallback applied only after no declared candidate fits exactly.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum PopupOverflowPolicy {
    #[default]
    Reject,
    Shift,
    Resize {
        minimum_size: SizeF,
    },
    Scroll {
        minimum_viewport: SizeF,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct PopupPlacementRequest {
    pub anchor: RectF,
    pub content_size: SizeF,
    pub safe_bounds: RectF,
    pub occlusions: Vec<RectF>,
    pub candidates: Vec<PopupPlacementCandidate>,
    pub writing_direction: WritingDirection,
    pub gap: f32,
    pub overflow: PopupOverflowPolicy,
}

impl PopupPlacementRequest {
    pub fn new(
        anchor: RectF,
        content_size: SizeF,
        safe_bounds: RectF,
        candidates: impl IntoIterator<Item = PopupPlacementCandidate>,
    ) -> Self {
        Self {
            anchor,
            content_size,
            safe_bounds,
            occlusions: Vec::new(),
            candidates: candidates.into_iter().collect(),
            writing_direction: WritingDirection::LeftToRight,
            gap: 0.0,
            overflow: PopupOverflowPolicy::Reject,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PopupPlacementAdjustment {
    Exact,
    Shifted { delta: PointF },
    Resized { delta: PointF },
    ScrollViewport { delta: PointF },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PopupPlacement {
    pub candidate: PopupPlacementCandidate,
    pub content_size: SizeF,
    pub rect: RectF,
    pub usable_region: RectF,
    pub adjustment: PopupPlacementAdjustment,
    pub evaluated_candidates: usize,
}

impl PopupPlacement {
    pub const fn requires_scroll(self) -> bool {
        matches!(
            self.adjustment,
            PopupPlacementAdjustment::ScrollViewport { .. }
        )
    }

    pub fn was_resized(self) -> bool {
        self.rect.width != self.content_size.width || self.rect.height != self.content_size.height
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopupPlacementError {
    InvalidAnchor,
    InvalidContentSize,
    InvalidSafeBounds,
    InvalidOcclusion { index: usize },
    TooManyOcclusions { count: usize, maximum: usize },
    NoCandidates,
    DuplicateCandidate { first: usize, duplicate: usize },
    InvalidGap,
    InvalidMinimumSize,
    DerivedGeometryOverflow,
    NoFit,
}

impl fmt::Display for PopupPlacementError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "popup placement failed: {self:?}")
    }
}

impl std::error::Error for PopupPlacementError {}

pub fn place_popup(request: &PopupPlacementRequest) -> Result<PopupPlacement, PopupPlacementError> {
    validate_request(request)?;
    let regions = subtract_occlusions(request.safe_bounds, &request.occlusions);
    if regions.is_empty() {
        return Err(PopupPlacementError::NoFit);
    }

    let mut desired = Vec::with_capacity(request.candidates.len());
    for candidate in request.candidates.iter().copied() {
        desired.push(candidate_rect(request, candidate)?);
    }

    for (candidate_index, (candidate, rect)) in request
        .candidates
        .iter()
        .copied()
        .zip(desired.iter().copied())
        .enumerate()
    {
        if let Some(region) = regions
            .iter()
            .copied()
            .find(|region| contains_rect(*region, rect))
        {
            return Ok(PopupPlacement {
                candidate,
                content_size: request.content_size,
                rect,
                usable_region: region,
                adjustment: PopupPlacementAdjustment::Exact,
                evaluated_candidates: candidate_index + 1,
            });
        }
    }

    let PopupOverflowPolicy::Reject = request.overflow else {
        for (candidate_index, (candidate, rect)) in request
            .candidates
            .iter()
            .copied()
            .zip(desired.iter().copied())
            .enumerate()
        {
            if let Some((placed, region, adjustment)) = adjust_into_regions(rect, request, &regions)
            {
                return Ok(PopupPlacement {
                    candidate,
                    content_size: request.content_size,
                    rect: placed,
                    usable_region: region,
                    adjustment,
                    evaluated_candidates: request.candidates.len() + candidate_index + 1,
                });
            }
        }
        return Err(PopupPlacementError::NoFit);
    };

    Err(PopupPlacementError::NoFit)
}

fn validate_request(request: &PopupPlacementRequest) -> Result<(), PopupPlacementError> {
    if !valid_rect(request.anchor, false) {
        return Err(PopupPlacementError::InvalidAnchor);
    }
    if !valid_size(request.content_size, true) {
        return Err(PopupPlacementError::InvalidContentSize);
    }
    if !valid_rect(request.safe_bounds, true) {
        return Err(PopupPlacementError::InvalidSafeBounds);
    }
    if !request.gap.is_finite() || request.gap < 0.0 {
        return Err(PopupPlacementError::InvalidGap);
    }
    if request.occlusions.len() > MAX_POPUP_OCCLUSIONS {
        return Err(PopupPlacementError::TooManyOcclusions {
            count: request.occlusions.len(),
            maximum: MAX_POPUP_OCCLUSIONS,
        });
    }
    for (index, occlusion) in request.occlusions.iter().copied().enumerate() {
        if !valid_rect(occlusion, false) {
            return Err(PopupPlacementError::InvalidOcclusion { index });
        }
    }
    if request.candidates.is_empty() {
        return Err(PopupPlacementError::NoCandidates);
    }
    for duplicate in 0..request.candidates.len() {
        if let Some(first) = request.candidates[..duplicate]
            .iter()
            .position(|candidate| *candidate == request.candidates[duplicate])
        {
            return Err(PopupPlacementError::DuplicateCandidate { first, duplicate });
        }
    }
    match request.overflow {
        PopupOverflowPolicy::Resize { minimum_size }
        | PopupOverflowPolicy::Scroll {
            minimum_viewport: minimum_size,
        } if !valid_size(minimum_size, true)
            || minimum_size.width > request.content_size.width
            || minimum_size.height > request.content_size.height =>
        {
            Err(PopupPlacementError::InvalidMinimumSize)
        }
        _ => Ok(()),
    }
}

fn valid_rect(rect: RectF, positive: bool) -> bool {
    [rect.x, rect.y, rect.width, rect.height]
        .into_iter()
        .all(f32::is_finite)
        && if positive {
            rect.width > 0.0 && rect.height > 0.0
        } else {
            rect.width >= 0.0 && rect.height >= 0.0
        }
        && rect.right().is_finite()
        && rect.bottom().is_finite()
}

fn valid_size(size: SizeF, positive: bool) -> bool {
    size.width.is_finite()
        && size.height.is_finite()
        && if positive {
            size.width > 0.0 && size.height > 0.0
        } else {
            size.width >= 0.0 && size.height >= 0.0
        }
}

fn candidate_rect(
    request: &PopupPlacementRequest,
    candidate: PopupPlacementCandidate,
) -> Result<RectF, PopupPlacementError> {
    let anchor = request.anchor;
    let size = request.content_size;
    let direction = request.writing_direction;
    let horizontal = || match candidate.alignment {
        PopupPlacementAlignment::Start => match direction {
            WritingDirection::LeftToRight => anchor.x,
            WritingDirection::RightToLeft => anchor.right() - size.width,
        },
        PopupPlacementAlignment::Center => anchor.x + (anchor.width - size.width) * 0.5,
        PopupPlacementAlignment::End => match direction {
            WritingDirection::LeftToRight => anchor.right() - size.width,
            WritingDirection::RightToLeft => anchor.x,
        },
    };
    let vertical = || match candidate.alignment {
        PopupPlacementAlignment::Start => anchor.y,
        PopupPlacementAlignment::Center => anchor.y + (anchor.height - size.height) * 0.5,
        PopupPlacementAlignment::End => anchor.bottom() - size.height,
    };
    let (x, y) = match candidate.side {
        PopupPlacementSide::Above => (horizontal(), anchor.y - request.gap - size.height),
        PopupPlacementSide::Below => (horizontal(), anchor.bottom() + request.gap),
        PopupPlacementSide::InlineStart => match direction {
            WritingDirection::LeftToRight => (anchor.x - request.gap - size.width, vertical()),
            WritingDirection::RightToLeft => (anchor.right() + request.gap, vertical()),
        },
        PopupPlacementSide::InlineEnd => match direction {
            WritingDirection::LeftToRight => (anchor.right() + request.gap, vertical()),
            WritingDirection::RightToLeft => (anchor.x - request.gap - size.width, vertical()),
        },
    };
    let rect = RectF {
        x,
        y,
        width: size.width,
        height: size.height,
    };
    valid_rect(rect, true)
        .then_some(rect)
        .ok_or(PopupPlacementError::DerivedGeometryOverflow)
}

fn adjust_into_regions(
    desired: RectF,
    request: &PopupPlacementRequest,
    regions: &[RectF],
) -> Option<(RectF, RectF, PopupPlacementAdjustment)> {
    match request.overflow {
        PopupOverflowPolicy::Reject => None,
        PopupOverflowPolicy::Shift => regions
            .iter()
            .copied()
            .filter(|region| {
                request.content_size.width <= region.width
                    && request.content_size.height <= region.height
            })
            .map(|region| {
                let placed = shifted_rect(desired, region);
                let delta = delta(desired, placed);
                (shift_score(delta), placed, region, delta)
            })
            .min_by(|left, right| left.0.total_cmp(&right.0))
            .map(|(_, placed, region, delta)| {
                (placed, region, PopupPlacementAdjustment::Shifted { delta })
            }),
        PopupOverflowPolicy::Resize { minimum_size } => {
            resized_into_region(desired, regions, minimum_size).map(|(placed, region, delta)| {
                let adjustment = if placed.width == desired.width && placed.height == desired.height
                {
                    PopupPlacementAdjustment::Shifted { delta }
                } else {
                    PopupPlacementAdjustment::Resized { delta }
                };
                (placed, region, adjustment)
            })
        }
        PopupOverflowPolicy::Scroll { minimum_viewport } => {
            resized_into_region(desired, regions, minimum_viewport).map(
                |(placed, region, delta)| {
                    let adjustment =
                        if placed.width == desired.width && placed.height == desired.height {
                            PopupPlacementAdjustment::Shifted { delta }
                        } else {
                            PopupPlacementAdjustment::ScrollViewport { delta }
                        };
                    (placed, region, adjustment)
                },
            )
        }
    }
}

fn resized_into_region(
    desired: RectF,
    regions: &[RectF],
    minimum_size: SizeF,
) -> Option<(RectF, RectF, PointF)> {
    regions
        .iter()
        .copied()
        .filter(|region| region.width >= minimum_size.width && region.height >= minimum_size.height)
        .map(|region| {
            let resized = RectF {
                width: desired.width.min(region.width),
                height: desired.height.min(region.height),
                ..desired
            };
            let placed = shifted_rect(resized, region);
            let delta = delta(desired, placed);
            (
                -(placed.width * placed.height),
                shift_score(delta),
                placed,
                region,
                delta,
            )
        })
        .min_by(|left, right| {
            left.0
                .total_cmp(&right.0)
                .then_with(|| left.1.total_cmp(&right.1))
        })
        .map(|(_, _, placed, region, delta)| (placed, region, delta))
}

fn shifted_rect(rect: RectF, region: RectF) -> RectF {
    RectF {
        x: rect.x.clamp(region.x, region.right() - rect.width),
        y: rect.y.clamp(region.y, region.bottom() - rect.height),
        ..rect
    }
}

fn delta(from: RectF, to: RectF) -> PointF {
    PointF {
        x: to.x - from.x,
        y: to.y - from.y,
    }
}

fn shift_score(delta: PointF) -> f32 {
    delta.x.abs() + delta.y.abs()
}

fn contains_rect(container: RectF, rect: RectF) -> bool {
    rect.x >= container.x
        && rect.y >= container.y
        && rect.right() <= container.right()
        && rect.bottom() <= container.bottom()
}

fn subtract_occlusions(safe: RectF, occlusions: &[RectF]) -> Vec<RectF> {
    let mut regions = vec![safe];
    for occlusion in occlusions {
        let mut next = Vec::with_capacity(regions.len().saturating_mul(2));
        for region in regions {
            let Some(overlap) = region.intersection(*occlusion) else {
                next.push(region);
                continue;
            };
            push_region(
                &mut next,
                RectF {
                    x: region.x,
                    y: region.y,
                    width: region.width,
                    height: overlap.y - region.y,
                },
            );
            push_region(
                &mut next,
                RectF {
                    x: region.x,
                    y: overlap.bottom(),
                    width: region.width,
                    height: region.bottom() - overlap.bottom(),
                },
            );
            push_region(
                &mut next,
                RectF {
                    x: region.x,
                    y: overlap.y,
                    width: overlap.x - region.x,
                    height: overlap.height,
                },
            );
            push_region(
                &mut next,
                RectF {
                    x: overlap.right(),
                    y: overlap.y,
                    width: region.right() - overlap.right(),
                    height: overlap.height,
                },
            );
        }
        regions = next;
    }
    regions
}

fn push_region(regions: &mut Vec<RectF>, region: RectF) {
    if region.width > 0.0 && region.height > 0.0 {
        regions.push(region);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, width: f32, height: f32) -> RectF {
        RectF {
            x,
            y,
            width,
            height,
        }
    }

    fn request(
        candidates: impl IntoIterator<Item = PopupPlacementCandidate>,
    ) -> PopupPlacementRequest {
        PopupPlacementRequest::new(
            rect(80.0, 40.0, 20.0, 10.0),
            SizeF {
                width: 30.0,
                height: 20.0,
            },
            rect(0.0, 0.0, 120.0, 100.0),
            candidates,
        )
    }

    #[test]
    fn exact_placement_respects_rtl_start_alignment() {
        let mut request = request([PopupPlacementCandidate::below(
            PopupPlacementAlignment::Start,
        )]);
        request.writing_direction = WritingDirection::RightToLeft;
        request.gap = 4.0;
        let placed = place_popup(&request).unwrap();
        assert_eq!(placed.rect, rect(70.0, 54.0, 30.0, 20.0));
        assert_eq!(placed.adjustment, PopupPlacementAdjustment::Exact);
        assert_eq!(placed.evaluated_candidates, 1);
    }

    #[test]
    fn later_exact_candidate_wins_before_overflow_fallback() {
        let mut request = request([
            PopupPlacementCandidate::below(PopupPlacementAlignment::Start),
            PopupPlacementCandidate::above(PopupPlacementAlignment::Start),
        ]);
        request.anchor = rect(80.0, 82.0, 20.0, 10.0);
        request.overflow = PopupOverflowPolicy::Shift;
        let placed = place_popup(&request).unwrap();
        assert_eq!(placed.candidate.side, PopupPlacementSide::Above);
        assert_eq!(placed.rect, rect(80.0, 62.0, 30.0, 20.0));
        assert_eq!(placed.adjustment, PopupPlacementAdjustment::Exact);
        assert_eq!(placed.evaluated_candidates, 2);
    }

    #[test]
    fn shift_uses_the_preferred_candidate_after_exact_attempts_fail() {
        let mut request = request([PopupPlacementCandidate::below(
            PopupPlacementAlignment::Start,
        )]);
        request.anchor = rect(108.0, 40.0, 10.0, 10.0);
        request.overflow = PopupOverflowPolicy::Shift;
        let placed = place_popup(&request).unwrap();
        assert_eq!(placed.rect, rect(90.0, 50.0, 30.0, 20.0));
        assert_eq!(
            placed.adjustment,
            PopupPlacementAdjustment::Shifted {
                delta: PointF { x: -18.0, y: 0.0 }
            }
        );
    }

    #[test]
    fn resize_and_scroll_have_distinct_typed_results() {
        let mut request = request([PopupPlacementCandidate::below(
            PopupPlacementAlignment::Start,
        )]);
        request.content_size = SizeF {
            width: 160.0,
            height: 120.0,
        };
        request.overflow = PopupOverflowPolicy::Resize {
            minimum_size: SizeF {
                width: 40.0,
                height: 30.0,
            },
        };
        let resized = place_popup(&request).unwrap();
        assert_eq!(resized.rect, rect(0.0, 0.0, 120.0, 100.0));
        assert!(matches!(
            resized.adjustment,
            PopupPlacementAdjustment::Resized { .. }
        ));
        assert!(!resized.requires_scroll());

        request.overflow = PopupOverflowPolicy::Scroll {
            minimum_viewport: SizeF {
                width: 40.0,
                height: 30.0,
            },
        };
        let scrolled = place_popup(&request).unwrap();
        assert_eq!(scrolled.rect, rect(0.0, 0.0, 120.0, 100.0));
        assert!(scrolled.requires_scroll());
    }

    #[test]
    fn occlusions_are_subtracted_before_shift_selection() {
        let mut request = request([PopupPlacementCandidate::below(
            PopupPlacementAlignment::Start,
        )]);
        request.occlusions = vec![rect(55.0, 0.0, 10.0, 100.0)];
        request.overflow = PopupOverflowPolicy::Shift;
        let placed = place_popup(&request).unwrap();
        let occlusion = request.occlusions[0];
        assert!(placed.rect.intersection(occlusion).is_none());
        assert!(contains_rect(placed.usable_region, placed.rect));
        assert_eq!(placed.usable_region, rect(65.0, 0.0, 55.0, 100.0));
    }

    #[test]
    fn malformed_requests_reject_before_placement() {
        let mut invalid = request([]);
        assert_eq!(
            place_popup(&invalid),
            Err(PopupPlacementError::NoCandidates)
        );

        invalid = request([PopupPlacementCandidate::above(
            PopupPlacementAlignment::Start,
        )]);
        invalid.gap = f32::NAN;
        assert_eq!(place_popup(&invalid), Err(PopupPlacementError::InvalidGap));

        invalid = request([
            PopupPlacementCandidate::above(PopupPlacementAlignment::Start),
            PopupPlacementCandidate::above(PopupPlacementAlignment::Start),
        ]);
        assert_eq!(
            place_popup(&invalid),
            Err(PopupPlacementError::DuplicateCandidate {
                first: 0,
                duplicate: 1,
            })
        );
    }

    #[test]
    fn reject_and_insufficient_minimum_size_report_no_fit() {
        let mut request = request([PopupPlacementCandidate::below(
            PopupPlacementAlignment::Start,
        )]);
        request.content_size = SizeF {
            width: 160.0,
            height: 120.0,
        };
        assert_eq!(place_popup(&request), Err(PopupPlacementError::NoFit));
        request.overflow = PopupOverflowPolicy::Resize {
            minimum_size: SizeF {
                width: 121.0,
                height: 30.0,
            },
        };
        assert_eq!(place_popup(&request), Err(PopupPlacementError::NoFit));
    }
}

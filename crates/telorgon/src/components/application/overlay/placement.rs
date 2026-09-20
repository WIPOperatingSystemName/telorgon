//! Application popup defaults and environment adaptation over the neutral layout solver.

use std::fmt;

use crate::components::application::primitives::EnvironmentValues;
use crate::foundation::{RectF, SizeF};
use crate::input::WritingDirection;
use crate::ui::layout::{
    PopupOverflowPolicy, PopupPlacement, PopupPlacementAlignment, PopupPlacementCandidate,
    PopupPlacementError, place_popup,
};

/// Stable default ordering for an ordinary application popup.
///
/// Block-axis alternatives are preferred before inline-axis alternatives. Start alignment is
/// resolved by the neutral solver using the current writing direction.
pub const STANDARD_APPLICATION_POPUP_CANDIDATES: [PopupPlacementCandidate; 4] = [
    PopupPlacementCandidate::below(PopupPlacementAlignment::Start),
    PopupPlacementCandidate::above(PopupPlacementAlignment::Start),
    PopupPlacementCandidate::inline_end(PopupPlacementAlignment::Start),
    PopupPlacementCandidate::inline_start(PopupPlacementAlignment::Start),
];

/// Application policy supplied to the neutral placement owner.
///
/// The zero default gap deliberately leaves visual spacing to the popup component or theme. The
/// standard overflow behavior preserves content size and shifts only after all exact candidates
/// have failed.
#[derive(Clone, Debug, PartialEq)]
pub struct ApplicationPopupPlacementPolicy {
    pub candidates: Vec<PopupPlacementCandidate>,
    pub gap: f32,
    pub overflow: PopupOverflowPolicy,
}

impl ApplicationPopupPlacementPolicy {
    pub fn new(
        candidates: impl IntoIterator<Item = PopupPlacementCandidate>,
        overflow: PopupOverflowPolicy,
    ) -> Self {
        Self {
            candidates: candidates.into_iter().collect(),
            gap: 0.0,
            overflow,
        }
    }

    pub fn standard() -> Self {
        Self::new(
            STANDARD_APPLICATION_POPUP_CANDIDATES,
            PopupOverflowPolicy::Shift,
        )
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }
}

impl Default for ApplicationPopupPlacementPolicy {
    fn default() -> Self {
        Self::standard()
    }
}

/// One application placement request in view-local logical coordinates.
#[derive(Clone, Debug)]
pub struct ApplicationPopupPlacementRequest<'environment> {
    pub anchor: RectF,
    pub content_size: SizeF,
    pub environment: &'environment EnvironmentValues,
    pub policy: ApplicationPopupPlacementPolicy,
}

impl<'environment> ApplicationPopupPlacementRequest<'environment> {
    pub fn new(
        anchor: RectF,
        content_size: SizeF,
        environment: &'environment EnvironmentValues,
    ) -> Self {
        Self {
            anchor,
            content_size,
            environment,
            policy: ApplicationPopupPlacementPolicy::default(),
        }
    }

    pub fn policy(mut self, policy: ApplicationPopupPlacementPolicy) -> Self {
        self.policy = policy;
        self
    }
}

/// Neutral placement plus the application environment inputs relevant to recomputation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ApplicationPopupPlacement {
    pub placement: PopupPlacement,
    pub safe_bounds: RectF,
    pub device_scale: f32,
    pub writing_direction: WritingDirection,
}

impl ApplicationPopupPlacement {
    pub const fn requires_scroll(self) -> bool {
        self.placement.requires_scroll()
    }

    pub fn was_resized(self) -> bool {
        self.placement.was_resized()
    }
}

/// Application-environment adaptation or neutral placement failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationPopupPlacementError {
    InvalidDeviceScale,
    NoUsableSafeBounds,
    Layout(PopupPlacementError),
}

impl fmt::Display for ApplicationPopupPlacementError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDeviceScale => {
                formatter.write_str("application popup environment has an invalid device scale")
            }
            Self::NoUsableSafeBounds => formatter
                .write_str("application popup environment has no positive finite safe bounds"),
            Self::Layout(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ApplicationPopupPlacementError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Layout(error) => Some(error),
            Self::InvalidDeviceScale | Self::NoUsableSafeBounds => None,
        }
    }
}

/// Resolves one popup without retaining a cache or changing overlay lifecycle state.
pub fn place_application_popup(
    request: &ApplicationPopupPlacementRequest<'_>,
) -> Result<ApplicationPopupPlacement, ApplicationPopupPlacementError> {
    let environment = request.environment;
    if !environment.device_scale.is_finite() || environment.device_scale <= 0.0 {
        return Err(ApplicationPopupPlacementError::InvalidDeviceScale);
    }

    let safe_bounds = application_usable_bounds(environment)?;
    let neutral = crate::ui::layout::PopupPlacementRequest {
        anchor: request.anchor,
        content_size: request.content_size,
        safe_bounds,
        occlusions: environment.occlusions.clone(),
        candidates: request.policy.candidates.clone(),
        writing_direction: environment.writing_direction,
        gap: request.policy.gap,
        overflow: request.policy.overflow,
    };
    let placement = place_popup(&neutral).map_err(ApplicationPopupPlacementError::Layout)?;
    Ok(ApplicationPopupPlacement {
        placement,
        safe_bounds,
        device_scale: environment.device_scale,
        writing_direction: environment.writing_direction,
    })
}

/// Returns the application placement owner's validated safe bounds.
///
/// This remains crate-visible so edge-attached overlay policies can derive anchors without
/// duplicating safe-area arithmetic. Device-scale validation still belongs to the complete
/// placement operation.
pub(crate) fn application_usable_bounds(
    environment: &EnvironmentValues,
) -> Result<RectF, ApplicationPopupPlacementError> {
    let safe_bounds = RectF {
        x: environment.safe_area.left,
        y: environment.safe_area.top,
        width: environment.available_size.width - environment.safe_area.horizontal(),
        height: environment.available_size.height - environment.safe_area.vertical(),
    };
    [
        safe_bounds.x,
        safe_bounds.y,
        safe_bounds.width,
        safe_bounds.height,
        safe_bounds.right(),
        safe_bounds.bottom(),
    ]
    .into_iter()
    .all(f32::is_finite)
    .then_some(safe_bounds)
    .filter(|bounds| bounds.width > 0.0 && bounds.height > 0.0)
    .ok_or(ApplicationPopupPlacementError::NoUsableSafeBounds)
}

#[cfg(test)]
mod tests {
    use crate::foundation::EdgeInsets;

    use super::*;

    fn rect(x: f32, y: f32, width: f32, height: f32) -> RectF {
        RectF {
            x,
            y,
            width,
            height,
        }
    }

    fn environment() -> EnvironmentValues {
        EnvironmentValues {
            available_size: SizeF {
                width: 300.0,
                height: 200.0,
            },
            device_scale: 2.0,
            safe_area: EdgeInsets {
                top: 5.0,
                right: 20.0,
                bottom: 15.0,
                left: 10.0,
            },
            ..EnvironmentValues::default()
        }
    }

    #[test]
    fn standard_policy_is_stable_and_shift_only() {
        let policy = ApplicationPopupPlacementPolicy::default();
        assert_eq!(policy.candidates, STANDARD_APPLICATION_POPUP_CANDIDATES);
        assert_eq!(policy.gap, 0.0);
        assert_eq!(policy.overflow, PopupOverflowPolicy::Shift);
    }

    #[test]
    fn environment_safe_area_direction_and_scale_cross_the_boundary() {
        let mut environment = environment();
        environment.writing_direction = WritingDirection::RightToLeft;
        let request = ApplicationPopupPlacementRequest::new(
            rect(200.0, 60.0, 40.0, 20.0),
            SizeF {
                width: 80.0,
                height: 50.0,
            },
            &environment,
        );
        let placed = place_application_popup(&request).unwrap();

        assert_eq!(placed.safe_bounds, rect(10.0, 5.0, 270.0, 180.0));
        assert_eq!(placed.placement.rect, rect(160.0, 80.0, 80.0, 50.0));
        assert_eq!(placed.device_scale, 2.0);
        assert_eq!(placed.writing_direction, WritingDirection::RightToLeft);
    }

    #[test]
    fn all_exact_candidates_are_tried_before_application_shift() {
        let environment = environment();
        let request = ApplicationPopupPlacementRequest::new(
            rect(100.0, 165.0, 30.0, 15.0),
            SizeF {
                width: 70.0,
                height: 40.0,
            },
            &environment,
        );
        let placed = place_application_popup(&request).unwrap();

        assert_eq!(
            placed.placement.candidate,
            PopupPlacementCandidate::above(PopupPlacementAlignment::Start)
        );
        assert_eq!(
            placed.placement.adjustment,
            crate::ui::layout::PopupPlacementAdjustment::Exact
        );
        assert_eq!(placed.placement.evaluated_candidates, 2);
    }

    #[test]
    fn environment_occlusions_are_delegated_without_mutation() {
        let mut environment = environment();
        environment.occlusions = vec![rect(90.0, 80.0, 80.0, 100.0)];
        let before = environment.clone();
        let policy = ApplicationPopupPlacementPolicy::new(
            [PopupPlacementCandidate::below(
                PopupPlacementAlignment::Start,
            )],
            PopupOverflowPolicy::Shift,
        );
        let request = ApplicationPopupPlacementRequest::new(
            rect(100.0, 50.0, 20.0, 20.0),
            SizeF {
                width: 60.0,
                height: 40.0,
            },
            &environment,
        )
        .policy(policy);
        let placed = place_application_popup(&request).unwrap();

        assert!(
            placed
                .placement
                .rect
                .intersection(environment.occlusions[0])
                .is_none()
        );
        assert_eq!(environment, before);
    }

    #[test]
    fn custom_scroll_policy_preserves_the_typed_neutral_result() {
        let environment = environment();
        let policy = ApplicationPopupPlacementPolicy::new(
            [PopupPlacementCandidate::below(
                PopupPlacementAlignment::Center,
            )],
            PopupOverflowPolicy::Scroll {
                minimum_viewport: SizeF {
                    width: 100.0,
                    height: 80.0,
                },
            },
        );
        let request = ApplicationPopupPlacementRequest::new(
            rect(100.0, 80.0, 20.0, 20.0),
            SizeF {
                width: 500.0,
                height: 400.0,
            },
            &environment,
        )
        .policy(policy);
        let placed = place_application_popup(&request).unwrap();

        assert!(placed.requires_scroll());
        assert!(placed.was_resized());
        assert_eq!(placed.placement.rect, rect(10.0, 5.0, 270.0, 180.0));
    }

    #[test]
    fn invalid_application_environment_and_layout_errors_remain_typed() {
        let mut invalid_scale = environment();
        invalid_scale.device_scale = f32::NAN;
        let request = ApplicationPopupPlacementRequest::new(
            rect(10.0, 10.0, 10.0, 10.0),
            SizeF {
                width: 20.0,
                height: 20.0,
            },
            &invalid_scale,
        );
        assert_eq!(
            place_application_popup(&request),
            Err(ApplicationPopupPlacementError::InvalidDeviceScale)
        );

        let mut no_safe_area = environment();
        no_safe_area.safe_area.left = 200.0;
        no_safe_area.safe_area.right = 100.0;
        let request = ApplicationPopupPlacementRequest::new(
            rect(10.0, 10.0, 10.0, 10.0),
            SizeF {
                width: 20.0,
                height: 20.0,
            },
            &no_safe_area,
        );
        assert_eq!(
            place_application_popup(&request),
            Err(ApplicationPopupPlacementError::NoUsableSafeBounds)
        );

        let environment = environment();
        let request = ApplicationPopupPlacementRequest::new(
            rect(10.0, 10.0, 10.0, 10.0),
            SizeF {
                width: 20.0,
                height: 20.0,
            },
            &environment,
        )
        .policy(ApplicationPopupPlacementPolicy::new(
            [],
            PopupOverflowPolicy::Reject,
        ));
        assert_eq!(
            place_application_popup(&request),
            Err(ApplicationPopupPlacementError::Layout(
                PopupPlacementError::NoCandidates
            ))
        );
    }
}

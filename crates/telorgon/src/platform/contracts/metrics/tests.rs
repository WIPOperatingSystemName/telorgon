use super::*;

fn ordinary_metrics() -> ViewMetrics {
    ViewMetrics::new(
        PhysicalExtent::new(1200, 800),
        ScaleFactor::new(2.0).unwrap(),
        DisplayProperties::new(
            DisplayTransform::Rotate90,
            DisplayColorSpace::DisplayP3,
            HdrState::Supported,
        ),
    )
    .unwrap()
}

fn ime_region() -> AvoidRegion {
    AvoidRegion::new(
        AvoidRegionKind::Ime,
        CoordinateSpace::ViewLogical,
        RectF {
            x: 0.0,
            y: 300.0,
            width: 600.0,
            height: 100.0,
        },
    )
    .unwrap()
}

fn assert_snapshot<T: Clone + PartialEq + Send + Sync + 'static>() {}

#[test]
fn desktop_coordinates_round_shared_edges_and_cover_fractional_damage() {
    use crate::foundation::{PointF, RectI, SizeI};
    let scale = ScaleFactor::new(1.5).unwrap();
    let left = scale.physical_rect(RectI {
        x: -1,
        y: 0,
        width: 4,
        height: 2,
    });
    let right = scale.physical_rect(RectI {
        x: 3,
        y: 0,
        width: 4,
        height: 2,
    });
    assert_eq!(left.right(), right.x);
    assert_eq!(
        left,
        RectI {
            x: -2,
            y: 0,
            width: 7,
            height: 3
        }
    );
    assert_eq!(
        scale.physical_damage(RectI {
            x: 1,
            y: -1,
            width: 2,
            height: 2
        }),
        RectI {
            x: 1,
            y: -2,
            width: 4,
            height: 4
        }
    );
    assert_eq!(
        scale.logical_size(SizeI {
            width: 1921,
            height: 1081
        }),
        SizeI {
            width: 1281,
            height: 721
        }
    );
    let pointer = PointF { x: 125.5, y: 71.25 };
    assert_eq!(scale.logical_point(scale.physical_point(pointer)), pointer);
    assert_eq!(
        scale.logical_point(PointF { x: 3.0, y: -6.0 }),
        PointF { x: 2.0, y: -4.0 }
    );
}

#[test]
fn scale_derives_coherent_logical_extent_and_named_transform_spaces() {
    let metrics = ordinary_metrics();
    assert_eq!(metrics.physical_extent(), PhysicalExtent::new(1200, 800));
    assert_eq!(
        metrics.logical_extent(),
        SizeF {
            width: 600.0,
            height: 400.0,
        }
    );
    assert_eq!(
        metrics.logical_to_physical().source_space(),
        CoordinateSpace::ViewLogical
    );
    assert_eq!(
        metrics.logical_to_physical().destination_space(),
        CoordinateSpace::ViewPhysical
    );
    assert_eq!(metrics.logical_to_physical().scale_factor().get(), 2.0);
    assert!(metrics.is_renderable());

    let fractional = ViewMetrics::new(
        PhysicalExtent::new(1001, 501),
        ScaleFactor::new(1.25).unwrap(),
        DisplayProperties::default(),
    )
    .unwrap()
    .logical_extent();
    assert!((fractional.width - 800.8).abs() < 0.001);
    assert!((fractional.height - 400.8).abs() < 0.001);
}

#[test]
fn zero_extent_is_preserved_and_never_clamped_to_a_renderable_size() {
    for extent in [
        PhysicalExtent::new(0, 800),
        PhysicalExtent::new(1200, 0),
        PhysicalExtent::ZERO,
    ] {
        let metrics = ViewMetrics::new(
            extent,
            ScaleFactor::new(2.0).unwrap(),
            DisplayProperties::default(),
        )
        .unwrap();
        assert_eq!(metrics.physical_extent(), extent);
        assert!(!metrics.is_renderable());
    }
}

#[test]
fn invalid_scale_and_derived_overflow_are_rejected() {
    for value in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert!(matches!(
            ScaleFactor::new(value),
            Err(ViewMetricsError::InvalidScaleFactor { .. })
        ));
    }
    let tiny = ScaleFactor::new(f32::MIN_POSITIVE).unwrap();
    assert!(matches!(
        ViewMetrics::new(
            PhysicalExtent::new(u32::MAX, 1),
            tiny,
            DisplayProperties::default()
        ),
        Err(ViewMetricsError::NonFiniteLogicalExtent { .. })
    ));
}

#[test]
fn safe_insets_are_finite_nonnegative_view_values_that_fit_the_extent() {
    let drawing = MetricInsets::new(
        CoordinateSpace::ViewLogical,
        EdgeInsets {
            top: 10.0,
            right: 20.0,
            bottom: 30.0,
            left: 20.0,
        },
    )
    .unwrap();
    let gesture =
        MetricInsets::new(CoordinateSpace::ViewPhysical, EdgeInsets::all(16.0)).unwrap();
    let metrics = ordinary_metrics()
        .with_safe_drawing_insets(drawing)
        .unwrap()
        .with_safe_gesture_insets(gesture)
        .unwrap();
    assert_eq!(metrics.safe_drawing_insets(), drawing);
    assert_eq!(metrics.safe_gesture_insets(), gesture);

    assert!(matches!(
        MetricInsets::new(CoordinateSpace::DisplayPhysical, EdgeInsets::ZERO),
        Err(ViewMetricsError::InvalidInsetSpace { .. })
    ));
    assert!(matches!(
        MetricInsets::new(CoordinateSpace::ViewLogical, EdgeInsets::all(-1.0)),
        Err(ViewMetricsError::InvalidInsets { .. })
    ));
    let too_large = MetricInsets::new(
        CoordinateSpace::ViewLogical,
        EdgeInsets {
            top: 0.0,
            right: 301.0,
            bottom: 0.0,
            left: 300.0,
        },
    )
    .unwrap();
    assert!(matches!(
        ordinary_metrics().with_safe_drawing_insets(too_large),
        Err(ViewMetricsError::InsetsExceedExtent { .. })
    ));
}

#[test]
fn avoid_regions_are_typed_bounded_and_coordinate_space_explicit() {
    let region = ime_region();
    let metrics = ordinary_metrics().with_avoid_regions(vec![region]).unwrap();
    assert_eq!(metrics.avoid_regions(), &[region]);
    assert_eq!(region.kind(), AvoidRegionKind::Ime);
    assert_eq!(region.space(), CoordinateSpace::ViewLogical);

    assert!(matches!(
        AvoidRegion::new(
            AvoidRegionKind::SystemUi,
            CoordinateSpace::DisplayPhysical,
            RectF::ZERO
        ),
        Err(ViewMetricsError::InvalidAvoidRegion { .. })
    ));
    assert!(matches!(
        ordinary_metrics().with_avoid_regions(vec![region; MAX_AVOID_REGIONS + 1]),
        Err(ViewMetricsError::TooManyAvoidRegions { .. })
    ));
}

#[test]
fn display_properties_preserve_transform_color_and_hdr_without_backend_types() {
    let display = ordinary_metrics().display();
    assert_eq!(display.transform(), DisplayTransform::Rotate90);
    assert_eq!(display.orientation(), DisplayOrientation::Clockwise90);
    assert!(!display.transform().is_mirrored());
    assert_eq!(display.color_space(), DisplayColorSpace::DisplayP3);
    assert_eq!(display.hdr(), HdrState::Supported);
}

#[test]
fn state_reuses_equal_publications_and_rejects_exhaustion_atomically() {
    let initial = ordinary_metrics();
    let mut state = ViewMetricsState::new(initial.clone());
    let redundant = state.update(initial).unwrap();
    assert!(!redundant.is_changed());
    assert_eq!(redundant.previous(), redundant.current());

    let changed = state
        .update(
            ViewMetrics::new(
                PhysicalExtent::new(600, 400),
                ScaleFactor::new(1.0).unwrap(),
                DisplayProperties::default(),
            )
            .unwrap(),
        )
        .unwrap();
    assert!(changed.is_changed());
    assert_eq!(changed.current().revision().get(), 2);
    assert_eq!(changed.current().metrics().physical_extent().width(), 600);

    state.revision = MetricsRevision::from_raw(u64::MAX).unwrap();
    let before = state.snapshot();
    assert_eq!(
        state.update(ViewMetrics::default()),
        Err(ViewMetricsError::RevisionExhausted {
            revision: MetricsRevision::from_raw(u64::MAX).unwrap(),
        })
    );
    assert_eq!(state.snapshot(), before);
    assert_snapshot::<ViewMetricsSnapshot>();
    assert_snapshot::<ViewMetricsUpdate>();
}

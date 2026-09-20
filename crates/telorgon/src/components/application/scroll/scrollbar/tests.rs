use std::cell::RefCell;
use std::rc::Rc;

use crate::foundation::SizeF;
use crate::runtime::{Component, CreateContext, UpdateContext, ViewRuntime};
use crate::ui::{NodeKind, SemanticAction, UiRoot};

use super::*;
use crate::components::application::{DensityClass, ScrollChangeSource};

fn size(width: f32, height: f32) -> SizeF {
    SizeF { width, height }
}

fn vertical_controller(offset: f32) -> ScrollController {
    let mut controller = ScrollController::new(size(100.0, 100.0), size(100.0, 500.0)).unwrap();
    controller
        .route(ScrollControllerCommand::ScrollTo {
            offset: PointF { x: 0.0, y: offset },
            source: ScrollInputSource::Programmatic,
        })
        .unwrap();
    controller
}

#[test]
fn line_page_and_bound_commands_preserve_boundary_handoff_and_source() {
    let mut controller = vertical_controller(390.0);
    let behavior =
        ScrollBarBehavior::from_controller(&controller, ScrollViewAxis::Vertical, 16.0, true)
            .unwrap();
    let command = behavior
        .request(ScrollBarCommand::LineForward, ScrollInputSource::Keyboard)
        .unwrap()
        .unwrap();
    assert_eq!(
        command,
        ScrollControllerCommand::ScrollBy {
            delta: PointF { x: 0.0, y: 16.0 },
            source: ScrollInputSource::Keyboard,
        }
    );
    assert_eq!(controller.metrics().offset.y, 390.0);
    let update = controller.route(command).unwrap().update();
    assert_eq!(update.after.offset.y, 400.0);
    assert_eq!(update.consumed_delta.y, 10.0);
    assert_eq!(update.unconsumed_delta.y, 6.0);
    assert_eq!(
        update.source,
        ScrollChangeSource::Input(ScrollInputSource::Keyboard)
    );

    let behavior =
        ScrollBarBehavior::from_controller(&controller, ScrollViewAxis::Vertical, 16.0, true)
            .unwrap();
    assert_eq!(
        behavior
            .request(ScrollBarCommand::LineForward, ScrollInputSource::Keyboard)
            .unwrap(),
        None
    );
    assert_eq!(
        behavior
            .request(ScrollBarCommand::ToEnd, ScrollInputSource::Keyboard)
            .unwrap(),
        None
    );
    assert_eq!(
        behavior
            .request(ScrollBarCommand::PageBackward, ScrollInputSource::Keyboard)
            .unwrap(),
        Some(ScrollControllerCommand::ScrollBy {
            delta: PointF { x: 0.0, y: -100.0 },
            source: ScrollInputSource::Keyboard,
        })
    );
    assert_eq!(
        behavior
            .request(ScrollBarCommand::ToStart, ScrollInputSource::Keyboard)
            .unwrap(),
        Some(ScrollControllerCommand::ScrollTo {
            offset: PointF { x: 0.0, y: 0.0 },
            source: ScrollInputSource::Keyboard,
        })
    );
}

#[test]
fn model_and_track_project_thumb_extent_position_and_pointer_offset() {
    let controller = vertical_controller(100.0);
    let behavior =
        ScrollBarBehavior::from_controller(&controller, ScrollViewAxis::Vertical, 20.0, true)
            .unwrap();
    let model = behavior.model();
    assert_eq!(model.offset(), 100.0);
    assert_eq!(model.maximum_offset(), 400.0);
    assert_eq!(model.thumb_fraction(), 0.2);
    assert_eq!(model.position_fraction(), 0.25);

    let track = ScrollBarTrackGeometry::new(10.0, 200.0, 24.0).unwrap();
    assert_eq!(
        track.project(model),
        ScrollBarThumbGeometry {
            origin: 50.0,
            extent: 40.0,
            travel: 160.0,
        }
    );
    assert_eq!(behavior.drag_to_offset(50.0, track).unwrap(), None);
    let pointer_command = behavior.drag_to_offset(90.0, track).unwrap().unwrap();
    assert_eq!(
        pointer_command,
        ScrollControllerCommand::ScrollTo {
            offset: PointF { x: 0.0, y: 200.0 },
            source: ScrollInputSource::Pointer,
        }
    );
    let mut routed = controller.clone();
    let update = routed.route(pointer_command).unwrap().update();
    assert_eq!(update.after.offset.y, 200.0);
    assert_eq!(
        update.source,
        ScrollChangeSource::Input(ScrollInputSource::Pointer)
    );
    assert_eq!(
        behavior.drag_to_offset(1_000.0, track).unwrap(),
        Some(ScrollControllerCommand::ScrollTo {
            offset: PointF { x: 0.0, y: 400.0 },
            source: ScrollInputSource::Pointer,
        })
    );
}

#[test]
fn semantic_actions_and_values_are_boundary_aware_and_nonmutating() {
    let controller = vertical_controller(0.0);
    let behavior =
        ScrollBarBehavior::from_controller(&controller, ScrollViewAxis::Vertical, 25.0, true)
            .unwrap();
    let actions = behavior.semantic_actions();
    assert!(actions.contains(SemanticAction::Focus));
    assert!(actions.contains(SemanticAction::Increment));
    assert!(actions.contains(SemanticAction::SetValue));
    assert!(!actions.contains(SemanticAction::Decrement));
    assert_eq!(
        behavior
            .semantic_request(SemanticAction::Increment)
            .unwrap(),
        Some(ScrollControllerCommand::ScrollBy {
            delta: PointF { x: 0.0, y: 25.0 },
            source: ScrollInputSource::Semantic,
        })
    );
    assert_eq!(
        behavior.semantic_set_value(200.0).unwrap(),
        Some(ScrollControllerCommand::ScrollTo {
            offset: PointF { x: 0.0, y: 200.0 },
            source: ScrollInputSource::Semantic,
        })
    );
    assert_eq!(
        behavior.semantic_set_value(401.0),
        Err(ScrollBarError::OffsetOutOfRange)
    );
    assert_eq!(controller.metrics().offset.y, 0.0);

    let disabled =
        ScrollBarBehavior::from_controller(&controller, ScrollViewAxis::Vertical, 25.0, false)
            .unwrap();
    assert!(disabled.semantic_actions().is_empty());
    assert_eq!(
        disabled
            .request(ScrollBarCommand::LineForward, ScrollInputSource::Keyboard)
            .unwrap(),
        None
    );
}

#[test]
fn construction_and_geometry_reject_invalid_public_inputs() {
    let controller = vertical_controller(0.0);
    assert_eq!(
        ScrollBar::new(" ", &controller),
        Err(ScrollBarError::MissingAccessibleName)
    );
    assert_eq!(
        ScrollBarBehavior::from_controller(&controller, ScrollViewAxis::Vertical, 0.0, true),
        Err(ScrollBarError::InvalidLineExtent)
    );
    assert_eq!(
        ScrollBarTrackGeometry::new(0.0, 20.0, 21.0),
        Err(ScrollBarError::InvalidTrackGeometry)
    );
}

struct Fixture {
    controller: ScrollController,
    reference: Rc<RefCell<Option<ScrollBarRef>>>,
}

impl Component for Fixture {
    type State = ();
    type Action = ();

    fn create(&self, _: &mut CreateContext<'_>) -> Self::State {}

    fn mount(&self, _: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
        let root = ui
            .foundation()
            .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
        let style = ScrollBarStyle {
            track_extent: 200.0,
            track_thickness: 10.0,
            minimum_thumb_extent: 20.0,
            thumb: BoxStyle {
                transform: Transform2D {
                    translation: PointF { x: 3.0, y: 0.0 },
                    ..Transform2D::default()
                },
                ..ScrollBarStyle::default().thumb
            },
            ..ScrollBarStyle::default()
        };
        let reference = ScrollBar::new("Document scroll position", &self.controller)
            .unwrap()
            .line_extent(25.0)
            .unwrap()
            .density(DensityMetrics::baseline(DensityClass::Touch))
            .style(style)
            .mount(ui, root.0)
            .unwrap();
        *self.reference.borrow_mut() = Some(reference);
        root
    }

    fn action(&self, _: &mut Self::State, _: Self::Action, _: &mut UpdateContext<'_, Self>) {}
}

#[test]
fn mounted_scrollbar_has_stable_track_thumb_density_and_range_semantics() {
    let reference = Rc::new(RefCell::new(None));
    let runtime = ViewRuntime::from_component(Fixture {
        controller: vertical_controller(100.0),
        reference: reference.clone(),
    })
    .unwrap();
    let reference = reference.borrow().expect("scrollbar reference");
    let root = reference.node();
    let track = reference.track_node();
    let thumb = reference.thumb_node();
    assert_eq!(runtime.ui().kinds.get(root), Some(&NodeKind::Button));
    assert_eq!(runtime.ui().nodes.core(track).unwrap().parent, Some(root));
    assert_eq!(runtime.ui().nodes.core(thumb).unwrap().parent, Some(track));
    assert_eq!(
        runtime.ui().box_styles.get(root).unwrap().min_size,
        SizeRule2D {
            width: SizeRule::Logical(44.0),
            height: SizeRule::Logical(44.0),
        }
    );
    assert_eq!(
        runtime.ui().box_styles.get(track).unwrap().width,
        SizeRule::Logical(10.0)
    );
    assert_eq!(
        runtime.ui().box_styles.get(track).unwrap().height,
        SizeRule::Logical(200.0)
    );
    let thumb_style = runtime.ui().box_styles.get(thumb).unwrap();
    assert_eq!(thumb_style.width, SizeRule::Logical(10.0));
    assert_eq!(thumb_style.height, SizeRule::Logical(40.0));
    assert_eq!(
        thumb_style.transform.translation,
        PointF { x: 3.0, y: 40.0 }
    );

    let interaction = runtime.ui().interactions.get(root).unwrap();
    assert!(interaction.enabled);
    assert!(interaction.focusable);
    let semantic = runtime.ui().semantics.get(root).unwrap();
    assert_eq!(semantic.role, SemanticRole::ScrollBar);
    assert!(semantic.actions.contains(SemanticAction::Increment));
    assert!(semantic.actions.contains(SemanticAction::Decrement));
    assert!(semantic.actions.contains(SemanticAction::SetValue));
    assert_eq!(
        semantic.value,
        SemanticValue::Number {
            current: 100.0,
            minimum: 0.0,
            maximum: 400.0,
            step: Some(25.0),
            value_text: match semantic.value {
                SemanticValue::Number { value_text, .. } => value_text,
                _ => None,
            },
        }
    );
    assert_eq!(
        reference.drag_to_offset(80.0).unwrap(),
        Some(ScrollControllerCommand::ScrollTo {
            offset: PointF { x: 0.0, y: 200.0 },
            source: ScrollInputSource::Pointer,
        })
    );
}

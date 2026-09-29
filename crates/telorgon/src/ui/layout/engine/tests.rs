use std::ops::{Deref, DerefMut};

use super::*;
use crate::foundation::ColorRgba8;
use crate::ui::{Border, BoxSizing, BoxStyle, LayoutStyle, MountWriter, MountedUi, Overflow};

struct TestLayout {
    engine: LayoutEngine,
    text: RetainedTextSystem,
}

impl Default for TestLayout {
    fn default() -> Self {
        Self {
            engine: LayoutEngine::default(),
            text: RetainedTextSystem::new(4096).unwrap(),
        }
    }
}

impl TestLayout {
    fn update(&mut self, ui: &mut MountedUi, viewport: SizeF, scale: f32) -> LayoutDiagnostics {
        self.engine.update(ui, &mut self.text, viewport, scale)
    }
}

impl Deref for TestLayout {
    type Target = LayoutEngine;

    fn deref(&self) -> &Self::Target {
        &self.engine
    }
}

impl DerefMut for TestLayout {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.engine
    }
}

#[test]
fn hit_testing_and_focus_use_canonical_geometry() {
    let mut ui = MountedUi::default();
    let button;
    {
        let mut builder = MountWriter::<()>::new(&mut ui);
        let mut saved = None;
        builder.root(
            BoxStyle {
                width: SizeRule::Fill(1.0),
                height: SizeRule::Fill(1.0),
                overflow: Overflow::Clip,
                ..BoxStyle::default()
            },
            LayoutStyle::default(),
            |builder| {
                saved = Some(builder.button(
                    (),
                    BoxStyle {
                        width: SizeRule::Logical(80.0),
                        height: SizeRule::Logical(32.0),
                        decoration: crate::ui::BoxDecoration {
                            background: crate::ui::Background::Color(ColorRgba8::rgba(
                                1, 2, 3, 255,
                            )),
                            ..crate::ui::BoxDecoration::default()
                        },
                        ..BoxStyle::default()
                    },
                    |builder| {
                        builder.text("Okay", ColorRgba8::rgba(255, 255, 255, 255), 14.0);
                    },
                ));
            },
        );
        button = saved.unwrap();
    }
    let mut layout = TestLayout::default();
    layout.update(
        &mut ui,
        SizeF {
            width: 200.0,
            height: 100.0,
        },
        1.0,
    );
    assert_eq!(
        layout.hit_test(&mut ui, PointF { x: 20.0, y: 20.0 }),
        Some(button.node)
    );
    assert_eq!(layout.focus_order(&mut ui), vec![button.node]);
}

#[test]
fn passive_visual_hit_testing_preserves_descendants_and_ordinary_control_routing() {
    let mut ui = MountedUi::default();
    let mut label = None;
    let mut button = None;
    MountWriter::<()>::new(&mut ui).root(
        BoxStyle::default(), LayoutStyle::default(), |writer| {
            button = Some(writer.button_node(
                BoxStyle {
                    width: SizeRule::Logical(120.0), height: SizeRule::Logical(40.0),
                    ..BoxStyle::default()
                }, |writer| {
                    label = Some(writer.text("Label", ColorRgba8::default(), 14.0).node);
                },
            ).node);
        },
    );
    let label = label.unwrap();
    let button = button.unwrap();
    let root = ui.nodes.core(button).unwrap().parent.unwrap();
    let mut layout = TestLayout::default();
    layout.update(&mut ui, SizeF { width: 200.0, height: 100.0 }, 1.0);
    let rect = layout.computed(label).unwrap().border_rect;
    let label_point = PointF { x: rect.x + rect.width / 2.0, y: rect.y + rect.height / 2.0 };
    let background_point = PointF { x: 180.0, y: 80.0 };

    assert_eq!(layout.hit_test(&mut ui, label_point), Some(button));
    assert_eq!(layout.hit_test(&mut ui, background_point), None);
    ui.set_visual_interaction(root, true, true);
    assert_eq!(layout.hit_test(&mut ui, label_point), Some(label));
    assert_eq!(layout.hit_test(&mut ui, background_point), Some(root));
    assert_eq!(layout.focus_order(&mut ui), vec![button]);
    ui.set_disabled(button, true);
    assert_eq!(layout.hit_test(&mut ui, label_point), Some(label));
    ui.set_disabled(button, false);
    ui.set_visual_interaction(root, false, false);
    assert_eq!(layout.hit_test(&mut ui, label_point), Some(button));
    ui.set_visual_interaction(label, true, false);
    assert_eq!(layout.hit_test(&mut ui, label_point), Some(label));
    assert_eq!(layout.hit_test(&mut ui, background_point), None);
}

#[test]
fn foundation_buttons_center_their_content_on_both_axes() {
    let mut ui = MountedUi::default();
    let (button, label);
    {
        let mut writer = MountWriter::<()>::new(&mut ui);
        let mut saved_button = None;
        let mut saved_label = None;
        writer.root(BoxStyle::default(), LayoutStyle::default(), |writer| {
            saved_button = Some(writer.button_node(
                BoxStyle {
                    width: SizeRule::Logical(120.0),
                    height: SizeRule::Logical(40.0),
                    ..BoxStyle::default()
                },
                |writer| {
                    saved_label = Some(
                        writer
                            .text("Centered", ColorRgba8::rgba(255, 255, 255, 255), 14.0)
                            .node,
                    );
                },
            ));
        });
        button = saved_button.unwrap().node;
        label = saved_label.unwrap();
    }

    let mut layout = TestLayout::default();
    layout.update(
        &mut ui,
        SizeF {
            width: 200.0,
            height: 100.0,
        },
        1.0,
    );

    let button_content = layout.computed(button).unwrap().content_rect;
    let label_rect = layout.computed(label).unwrap().border_rect;
    let button_center = PointF {
        x: button_content.x + button_content.width * 0.5,
        y: button_content.y + button_content.height * 0.5,
    };
    let label_center = PointF {
        x: label_rect.x + label_rect.width * 0.5,
        y: label_rect.y + label_rect.height * 0.5,
    };

    assert!((button_center.x - label_center.x).abs() < 0.001);
    assert!((button_center.y - label_center.y).abs() < 0.001);
}

#[test]
fn scroll_visual_hit_testing_keeps_raw_children_without_changing_scroll_ownership() {
    let mut ui = MountedUi::default();
    let mut scroll = None;
    let mut label = None;
    MountWriter::<()>::new(&mut ui).root(
        BoxStyle::default(), LayoutStyle::default(), |writer| {
            scroll = Some(writer.scroll(
                BoxStyle {
                    width: SizeRule::Logical(160.0), height: SizeRule::Logical(80.0),
                    ..BoxStyle::default()
                }, LayoutStyle::default(), |writer| {
                    label = Some(writer.text("Label", ColorRgba8::default(), 14.0).node);
                },
            ).node);
        },
    );
    let scroll = scroll.unwrap();
    let label = label.unwrap();
    let mut layout = TestLayout::default();
    layout.update(&mut ui, SizeF { width: 200.0, height: 100.0 }, 1.0);
    let rect = layout.computed(label).unwrap().border_rect;
    let point = PointF { x: rect.x + rect.width / 2.0, y: rect.y + rect.height / 2.0 };
    assert_eq!(layout.hit_test(&mut ui, point), Some(scroll));
    for (hover, press) in [(true, false), (false, true)] {
        ui.set_visual_interaction(scroll, hover, press);
        assert_eq!(layout.hit_test(&mut ui, point), Some(label));
        assert_eq!(ui.nearest_control(label), Some(scroll));
        assert!(layout.focus_order(&mut ui).is_empty());
    }
    ui.set_visual_interaction(scroll, false, false);
    assert_eq!(layout.hit_test(&mut ui, point), Some(scroll));
    ui.set_hover_within(scroll, true);
    assert_eq!(layout.hit_test(&mut ui, point), Some(label));
    ui.set_hover_within(scroll, false);
    assert_eq!(layout.hit_test(&mut ui, point), Some(scroll));
}

#[test]
fn shrink_text_uses_shaped_advance_instead_of_character_count_estimate() {
    let mut ui = MountedUi::default();
    let label = {
        let mut writer = MountWriter::<()>::new(&mut ui);
        let mut label = None;
        writer.root(BoxStyle::default(), LayoutStyle::default(), |writer| {
            label = Some(
                writer
                    .text("Count: 0", ColorRgba8::rgba(255, 255, 255, 255), 14.0)
                    .node,
            );
        });
        label.unwrap()
    };
    let mut layout = LayoutEngine::default();
    let mut text = RetainedTextSystem::new(4096).unwrap();
    layout.update(
        &mut ui,
        &mut text,
        SizeF {
            width: 200.0,
            height: 100.0,
        },
        1.0,
    );

    let visual = *ui.texts.get(label).unwrap();
    let content = ui.string(visual.content).unwrap();
    let family = ui.string(visual.style.family).unwrap();
    let font_size = visual.style.size.ceil();
    let line_height = visual.style.line_height.ceil();
    let run_id = text
        .measure(RetainedTextRequest {
            key: TextRunKey::new(
                visual.revision,
                1,
                family,
                font_size,
                visual.style.weight,
                line_height,
                None,
                None,
                1.0,
            ),
            text: content,
            family,
            font_size_px: font_size as i32,
            line_height_px: line_height as i32,
            max_width_px: None,
            max_height_px: None,
        })
        .unwrap();
    let shaped_width = text.run(run_id).unwrap().advance_width_px;
    let box_width = layout.computed(label).unwrap().content_rect.width;
    let old_estimate = content.chars().count() as f32 * visual.style.size * 0.6;

    assert!((box_width - shaped_width).abs() < 0.001);
    assert!((box_width - old_estimate).abs() > 0.5);
}

#[test]
fn scroll_is_spatial_only_after_initial_layout() {
    let mut ui = MountedUi::default();
    let scroll;
    {
        let mut builder = MountWriter::<()>::new(&mut ui);
        let mut saved = None;
        builder.root(BoxStyle::default(), LayoutStyle::default(), |builder| {
            saved =
                Some(
                    builder.scroll(BoxStyle::default(), LayoutStyle::default(), |builder| {
                        builder.text("Scrollable", ColorRgba8::rgba(255, 255, 255, 255), 14.0);
                    }),
                );
        });
        scroll = saved.unwrap();
    }
    let mut layout = TestLayout::default();
    layout.update(
        &mut ui,
        SizeF {
            width: 200.0,
            height: 100.0,
        },
        1.0,
    );
    ui.transaction(|tx| tx.set(scroll.offset, PointF { x: 0.0, y: 20.0 }));
    let diagnostics = layout.update(
        &mut ui,
        SizeF {
            width: 200.0,
            height: 100.0,
        },
        1.0,
    );
    assert_eq!(diagnostics.measured, 0);
    assert_eq!(diagnostics.arranged, 0);
    assert!(diagnostics.spatial_updated > 0);
}

#[test]
fn variable_extent_virtualization_is_bounded() {
    let mut collection = VirtualCollection::new(100_000, 20.0, 40.0);
    collection.set_extent(4, 80.0);
    let range = collection.visible_range(100.0, 200.0);
    assert!(range.len() < 30);
    assert!(collection.total_extent() > 2_000_000.0);
    assert_eq!(collection.item_range(4), Some(80.0..160.0));
    assert_eq!(collection.item_range(100_000), None);
}

#[test]
fn hidden_parent_removes_descendants_from_canonical_visibility() {
    let mut ui = MountedUi::default();
    let button = {
        let mut builder = MountWriter::<()>::new(&mut ui);
        let mut saved = None;
        builder.root(BoxStyle::default(), LayoutStyle::default(), |builder| {
            saved = Some(builder.button(
                (),
                BoxStyle {
                    width: SizeRule::Logical(80.0),
                    height: SizeRule::Logical(30.0),
                    ..BoxStyle::default()
                },
                |builder| {
                    builder.text("Hidden", ColorRgba8::rgba(255, 255, 255, 255), 14.0);
                },
            ));
        });
        saved.unwrap()
    };
    let mut layout = TestLayout::default();
    let extent = SizeF {
        width: 200.0,
        height: 100.0,
    };
    layout.update(&mut ui, extent, 1.0);
    let child = ui.nodes.children(button.node).next().unwrap();
    ui.transaction(|transaction| transaction.set(button.visible, false));
    layout.update(&mut ui, extent, 1.0);
    assert_eq!(
        layout.computed(button.node).unwrap().visible_rect,
        RectF::ZERO
    );
    assert_eq!(layout.computed(child).unwrap().visible_rect, RectF::ZERO);
    assert_eq!(layout.hit_test(&mut ui, PointF { x: 10.0, y: 10.0 }), None);
    assert!(layout.focus_order(&mut ui).is_empty());
}

#[test]
fn content_box_and_weighted_fill_publish_exact_canonical_rects() {
    let mut ui = MountedUi::default();
    let (content_box, fill_one, fill_two) = {
        let mut builder = MountWriter::<()>::new(&mut ui);
        let mut nodes = None;
        builder.root(
            BoxStyle::default(),
            LayoutStyle {
                flow: Flow::Horizontal,
                gap: 10.0,
                ..LayoutStyle::default()
            },
            |builder| {
                let content_box = builder.container(
                    BoxStyle {
                        sizing: BoxSizing::ContentBox,
                        width: SizeRule::Logical(100.0),
                        height: SizeRule::Logical(20.0),
                        margin: EdgeInsets::all(5.0),
                        padding: EdgeInsets::all(10.0),
                        decoration: crate::ui::BoxDecoration {
                            border: Border::all(2.0, ColorRgba8::rgba(1, 2, 3, 255)),
                            ..crate::ui::BoxDecoration::default()
                        },
                        ..BoxStyle::default()
                    },
                    LayoutStyle::default(),
                    |_| {},
                );
                let fill_one = builder.container(
                    BoxStyle {
                        width: SizeRule::Fill(1.0),
                        height: SizeRule::Logical(20.0),
                        ..BoxStyle::default()
                    },
                    LayoutStyle::default(),
                    |_| {},
                );
                let fill_two = builder.container(
                    BoxStyle {
                        width: SizeRule::Fill(2.0),
                        height: SizeRule::Logical(20.0),
                        ..BoxStyle::default()
                    },
                    LayoutStyle::default(),
                    |_| {},
                );
                nodes = Some((content_box, fill_one, fill_two));
            },
        );
        nodes.unwrap()
    };
    let mut layout = TestLayout::default();
    layout.update(
        &mut ui,
        SizeF {
            width: 500.0,
            height: 100.0,
        },
        1.0,
    );
    let content = layout.computed(content_box).unwrap();
    assert_eq!(content.margin_rect.width, 134.0);
    assert_eq!(content.border_rect.x, 5.0);
    assert_eq!(content.border_rect.width, 124.0);
    assert_eq!(content.content_rect.x, 17.0);
    assert_eq!(content.content_rect.width, 100.0);
    let one = layout.computed(fill_one).unwrap();
    let two = layout.computed(fill_two).unwrap();
    assert!((one.border_rect.width - 115.333_336).abs() < 0.01);
    assert!((two.border_rect.width - 230.666_67).abs() < 0.01);
    assert!((one.border_rect.x - 144.0).abs() < 0.01);
    assert!((two.border_rect.x - 269.333_34).abs() < 0.01);
}

#[test]
fn scale_and_translation_propagate_through_canonical_geometry() {
    let mut ui = MountedUi::default();
    let (parent, child) = {
        let mut builder = MountWriter::<()>::new(&mut ui);
        let mut nodes = None;
        builder.root(BoxStyle::default(), LayoutStyle::default(), |builder| {
            let mut child = None;
            let parent = builder.container(
                BoxStyle {
                    width: SizeRule::Logical(100.0),
                    height: SizeRule::Logical(50.0),
                    padding: EdgeInsets::all(10.0),
                    transform: crate::foundation::Transform2D {
                        translation: PointF { x: 5.0, y: 3.0 },
                        scale: PointF { x: 2.0, y: 2.0 },
                        ..crate::foundation::Transform2D::default()
                    },
                    ..BoxStyle::default()
                },
                LayoutStyle::default(),
                |builder| {
                    child = Some(builder.container(
                        BoxStyle {
                            width: SizeRule::Logical(10.0),
                            height: SizeRule::Logical(10.0),
                            ..BoxStyle::default()
                        },
                        LayoutStyle::default(),
                        |_| {},
                    ));
                },
            );
            nodes = Some((parent, child.unwrap()));
        });
        nodes.unwrap()
    };
    let mut layout = TestLayout::default();
    layout.update(
        &mut ui,
        SizeF {
            width: 400.0,
            height: 200.0,
        },
        1.0,
    );
    let parent = layout.computed(parent).unwrap();
    assert_eq!(parent.border_rect.x, 5.0);
    assert_eq!(parent.border_rect.y, 3.0);
    assert_eq!(parent.border_rect.width, 200.0);
    assert_eq!(parent.content_rect.x, 25.0);
    let child = layout.computed(child).unwrap();
    assert_eq!(child.border_rect.x, 25.0);
    assert_eq!(child.border_rect.y, 23.0);
    assert_eq!(child.border_rect.width, 20.0);
    assert_eq!(child.border_rect.height, 20.0);
}

#[test]
fn visible_overflow_does_not_clip_descendants_but_hidden_overflow_does() {
    let mut ui = MountedUi::default();
    let (parent, child) = {
        let mut writer = MountWriter::<()>::new(&mut ui);
        let mut nodes = None;
        writer.root(BoxStyle::default(), LayoutStyle::default(), |writer| {
            let mut child = None;
            let parent = writer.container(
                BoxStyle {
                    width: SizeRule::Logical(20.0),
                    height: SizeRule::Logical(20.0),
                    overflow: Overflow::Visible,
                    ..BoxStyle::default()
                },
                LayoutStyle::default(),
                |writer| {
                    child = Some(writer.container(
                        BoxStyle {
                            width: SizeRule::Logical(10.0),
                            height: SizeRule::Logical(10.0),
                            max_size: crate::ui::SizeRule2D {
                                width: SizeRule::Logical(10.0),
                                height: SizeRule::Logical(10.0),
                            },
                            transform: crate::foundation::Transform2D {
                                translation: PointF { x: 30.0, y: 0.0 },
                                ..crate::foundation::Transform2D::default()
                            },
                            ..BoxStyle::default()
                        },
                        LayoutStyle::default(),
                        |_| {},
                    ));
                },
            );
            nodes = Some((parent, child.unwrap()));
        });
        nodes.unwrap()
    };
    let mut layout = TestLayout::default();
    let extent = SizeF {
        width: 100.0,
        height: 100.0,
    };
    layout.update(&mut ui, extent, 1.0);
    assert_eq!(layout.computed(child).unwrap().visible_rect.width, 10.0);

    ui.box_styles.get_mut(parent).unwrap().overflow = Overflow::Clip;
    ui.nodes.mark_dirty(
        parent,
        DirtyFlags::SPATIAL | DirtyFlags::CLIP | DirtyFlags::PAINT,
    );
    layout.update(&mut ui, extent, 1.0);
    assert_eq!(layout.computed(child).unwrap().visible_rect, RectF::ZERO);
}

#[test]
fn rotation_and_origin_update_geometry_and_inverse_hit_testing_without_remeasure() {
    let mut ui = MountedUi::default();
    let button = {
        let mut writer = MountWriter::<()>::new(&mut ui);
        let mut button = None;
        writer.root(BoxStyle::default(), LayoutStyle::default(), |writer| {
            button = Some(writer.button_node(
                BoxStyle {
                    width: SizeRule::Logical(20.0),
                    height: SizeRule::Logical(10.0),
                    transform: crate::foundation::Transform2D {
                        rotation: std::f32::consts::FRAC_PI_2,
                        origin: PointF { x: 0.5, y: 0.5 },
                        ..crate::foundation::Transform2D::default()
                    },
                    ..BoxStyle::default()
                },
                |_| {},
            ));
        });
        button.unwrap().node
    };
    let mut layout = TestLayout::default();
    layout.update(
        &mut ui,
        SizeF {
            width: 100.0,
            height: 100.0,
        },
        1.0,
    );
    let computed = layout.computed(button).unwrap();
    assert!((computed.border_rect.width - 10.0).abs() < 0.001);
    assert!((computed.border_rect.height - 20.0).abs() < 0.001);
    let rotated_hit = computed
        .world_transform
        .transform_point(PointF { x: 19.0, y: 5.0 });
    assert_eq!(layout.hit_test(&mut ui, rotated_hit), Some(button));

    ui.box_styles.get_mut(button).unwrap().transform.rotation = 0.0;
    ui.nodes
        .mark_dirty(button, DirtyFlags::SPATIAL | DirtyFlags::PAINT);
    let after = layout.update(
        &mut ui,
        SizeF {
            width: 100.0,
            height: 100.0,
        },
        1.0,
    );
    assert_eq!(after.measured, 0);
    assert_eq!(after.arranged, 0);
    assert!(after.spatial_updated > 0);
    assert_eq!(layout.hit_test(&mut ui, rotated_hit), None);
}

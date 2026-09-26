use telorgon::app::*;
use telorgon::compose::ElementKind;
use telorgon::ui::layout::LayoutEngine;
use telorgon::ui::text::RetainedTextSystem;
use telorgon::ui::{MountWriter, MountedUi};

fn measured(style: BoxStyle) -> SizeF {
    let mut ui = MountedUi::default();
    let mut child = None;
    MountWriter::<()>::new(&mut ui).root(BoxStyle::default(), LayoutStyle::default(), |writer| {
        child = Some(writer.container(style, LayoutStyle::default(), |_| {}));
    });
    let mut layout = LayoutEngine::default();
    let mut text = RetainedTextSystem::new(4096).unwrap();
    layout.update(
        &mut ui,
        &mut text,
        SizeF {
            width: 500.0,
            height: 400.0,
        },
        1.0,
    );
    let rect = layout.computed(child.unwrap()).unwrap().local_border_rect;
    SizeF {
        width: rect.width,
        height: rect.height,
    }
}

#[test]
fn derives_either_axis_and_respects_explicit_sizes_and_constraints() {
    let width = BoxStyle {
        width: SizeRule::Logical(200.0),
        aspect_ratio: Some(2.0),
        ..Default::default()
    };
    assert_eq!(
        measured(width),
        SizeF {
            width: 200.0,
            height: 100.0
        }
    );
    assert_eq!(
        measured(BoxStyle {
            width: SizeRule::Shrink,
            height: SizeRule::Logical(100.0),
            ..width
        }),
        SizeF {
            width: 200.0,
            height: 100.0
        }
    );
    assert_eq!(
        measured(BoxStyle {
            height: SizeRule::Logical(70.0),
            ..width
        }),
        SizeF {
            width: 200.0,
            height: 70.0
        }
    );
    assert_eq!(
        measured(BoxStyle {
            width: SizeRule::Shrink,
            ..width
        }),
        SizeF {
            width: 24.0,
            height: 24.0
        }
    );
    assert_eq!(
        measured(BoxStyle {
            max_size: SizeRule2D {
                width: SizeRule::Logical(120.0),
                height: SizeRule::Fill(1.0)
            },
            ..width
        }),
        SizeF {
            width: 120.0,
            height: 60.0
        }
    );
    assert_eq!(
        measured(BoxStyle {
            min_size: SizeRule2D {
                width: SizeRule::Shrink,
                height: SizeRule::Logical(150.0)
            },
            ..width
        }),
        SizeF {
            width: 200.0,
            height: 150.0
        }
    );
    assert_eq!(
        measured(BoxStyle {
            sizing: BoxSizing::ContentBox,
            padding: EdgeInsets::all(10.0),
            ..width
        }),
        SizeF {
            width: 220.0,
            height: 120.0
        }
    );
    assert_eq!(
        measured(BoxStyle {
            width: SizeRule::Percent(0.5),
            ..width
        }),
        SizeF {
            width: 250.0,
            height: 125.0
        }
    );
}

#[test]
fn weighted_fill_uses_allocated_axis_for_size_and_center_alignment() {
    for horizontal in [true, false] {
        let mut ui = MountedUi::default();
        let mut children = Vec::new();
        MountWriter::<()>::new(&mut ui).root(
            BoxStyle::default(),
            LayoutStyle {
                flow: if horizontal {
                    Flow::Horizontal
                } else {
                    Flow::Vertical
                },
                cross_axis_alignment: CrossAxisAlignment::Center,
                ..Default::default()
            },
            |writer| {
                for weight in [1.0, 2.0] {
                    children.push(writer.container(
                        BoxStyle {
                            width: if horizontal {
                                SizeRule::Fill(weight)
                            } else {
                                SizeRule::Shrink
                            },
                            height: if horizontal {
                                SizeRule::Shrink
                            } else {
                                SizeRule::Fill(weight)
                            },
                            aspect_ratio: Some(1.0),
                            ..Default::default()
                        },
                        LayoutStyle::default(),
                        |_| {},
                    ));
                }
            },
        );
        let mut layout = LayoutEngine::default();
        let mut text = RetainedTextSystem::new(4096).unwrap();
        for extent in [300.0, 600.0] {
            layout.update(
                &mut ui,
                &mut text,
                SizeF {
                    width: extent,
                    height: extent,
                },
                1.0,
            );
            for (index, child) in children.iter().enumerate() {
                let rect = layout.computed(*child).unwrap().local_border_rect;
                let size = extent * (index + 1) as f32 / 3.0;
                assert_eq!(rect.width, size);
                assert_eq!(rect.height, size);
                assert_eq!(
                    if horizontal { rect.y } else { rect.x },
                    (extent - size) / 2.0
                );
            }
        }
    }
}

#[test]
fn builders_preserve_explicit_axes_independently_of_call_order() {
    for view in [
        row().height(Dimension::FILL).aspect_ratio(1.0),
        column().aspect_ratio(1.0).height(Dimension::FILL),
        stack().height(Dimension::FILL).aspect_ratio(1.0),
        card().height(Dimension::FILL).aspect_ratio(1.0),
        spacer().height(Dimension::FILL).aspect_ratio(1.0),
    ] {
        let element = view.into_element();
        let ElementKind::Container(props) = element.kind() else {
            panic!()
        };
        assert_eq!(props.style.width, SizeRule::Shrink);
        assert_eq!(
            measured(props.style),
            SizeF {
                width: 400.0,
                height: 400.0
            }
        );
    }
    let element = row()
        .width(200.0)
        .aspect_ratio(2.0)
        .height(70.0)
        .into_element();
    let ElementKind::Container(props) = element.kind() else {
        panic!()
    };
    assert_eq!(
        measured(props.style),
        SizeF {
            width: 200.0,
            height: 70.0
        }
    );
    let elements = [
        button().width(200.0).aspect_ratio(2.0).into_element(),
        text("Hello").aspect_ratio(2.0).height(100.0).into_element(),
        image(IconAsset::new(AssetKey::new("test.svg")))
            .width(200.0)
            .aspect_ratio(2.0)
            .into_element(),
        window_content_slot()
            .height(100.0)
            .aspect_ratio(2.0)
            .into_element(),
        window_frame()
            .aspect_ratio(2.0)
            .width(200.0)
            .content_slot(window_content_slot())
            .into_element(),
    ];
    for element in elements {
        element.validate().unwrap();
    }
    for ratio in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert!(
            button()
                .aspect_ratio(ratio)
                .into_element()
                .validate()
                .is_err()
        );
        assert!(row().aspect_ratio(ratio).into_element().validate().is_err());
        assert!(
            text("Hello")
                .aspect_ratio(ratio)
                .into_element()
                .validate()
                .is_err()
        );
        assert!(
            image(IconAsset::new(AssetKey::new("test.svg")))
                .aspect_ratio(ratio)
                .into_element()
                .validate()
                .is_err()
        );
    }
}

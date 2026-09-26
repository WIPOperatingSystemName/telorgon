use super::*;

pub(super) fn button_semantics(name: SemanticName, props: &ButtonElement) -> SemanticNode {
    let mut actions = SemanticActions::NONE;
    if props.enabled {
        actions |= SemanticActions::FOCUS;
        if !props.busy {
            actions |= SemanticActions::ACTIVATE;
        }
    }
    SemanticNode {
        role: SemanticRole::Button,
        name,
        state: SemanticState {
            disabled: !props.enabled,
            busy: props.busy,
            focusable: props.enabled,
            ..SemanticState::default()
        },
        actions,
        ..SemanticNode::default()
    }
}

pub(super) fn control_label_style(enabled: bool) -> crate::ui::TextStyle {
    crate::ui::TextStyle {
        color: if enabled {
            ColorRgba8::rgba(235, 238, 244, 255)
        } else {
            ColorRgba8::rgba(153, 157, 168, 255)
        },
        size: 14.0,
        line_height: 17.5,
        family: crate::ui::StringId(1),
        weight: 400,
        align: crate::ui::TextAlign::Start,
        vertical_align: crate::ui::TextAlign::Start,
        fit_height: false,
    }
}

pub(super) fn toggle_semantics(name: crate::ui::StringId, props: &ToggleElement) -> SemanticNode {
    let mut actions = SemanticActions::NONE;
    if props.enabled {
        actions |= SemanticActions::FOCUS | SemanticActions::ACTIVATE;
    }
    SemanticNode {
        role: match props.kind {
            ToggleKind::Checkbox => SemanticRole::Checkbox,
            ToggleKind::Switch => SemanticRole::Switch,
        },
        name: SemanticName::Text(name),
        state: SemanticState {
            disabled: !props.enabled,
            focusable: props.enabled,
            checked: Some(props.value),
            ..SemanticState::default()
        },
        actions,
        ..SemanticNode::default()
    }
}

pub(super) fn slider_semantics(
    name: crate::ui::StringId,
    value_text: crate::ui::StringId,
    props: &SliderElement,
) -> SemanticNode {
    let actions = if props.enabled {
        SemanticActions::FOCUS
            | SemanticActions::INCREMENT
            | SemanticActions::DECREMENT
            | SemanticActions::SET_VALUE
    } else {
        SemanticActions::NONE
    };
    SemanticNode {
        role: SemanticRole::Slider,
        name: SemanticName::Text(name),
        state: SemanticState {
            disabled: !props.enabled,
            focusable: props.enabled,
            ..SemanticState::default()
        },
        value: SemanticValue::Number {
            current: f64::from(props.value),
            minimum: 0.0,
            maximum: 1.0,
            step: Some(0.01),
            value_text: Some(value_text),
        },
        actions,
        ..SemanticNode::default()
    }
}

#[derive(Clone, Copy)]
pub(super) struct CheckboxStyles {
    pub(super) container: BoxStyle,
    pub(super) indicator: BoxStyle,
    pub(super) check_first: BoxStyle,
    pub(super) check_second: BoxStyle,
    pub(super) mixed: BoxStyle,
}

pub(super) fn checkbox_styles(value: SemanticCheckState, enabled: bool) -> CheckboxStyles {
    let opacity = if enabled { 255 } else { 180 };
    let checked = value != SemanticCheckState::Unchecked;
    let indicator = BoxStyle {
        sizing: BoxSizing::BorderBox,
        width: SizeRule::Logical(18.0),
        height: SizeRule::Logical(18.0),
        max_size: SizeRule2D {
            width: SizeRule::Logical(18.0),
            height: SizeRule::Logical(18.0),
        },
        decoration: crate::ui::BoxDecoration {
            background: if checked {
                Background::Color(ColorRgba8::rgba(54, 104, 210, opacity))
            } else {
                Background::Color(ColorRgba8::rgba(28, 31, 39, opacity))
            },
            border: Border::all(1.0, ColorRgba8::rgba(118, 127, 145, opacity)),
            corner_radii: CornerRadii::all(4.0),
            ..crate::ui::BoxDecoration::default()
        },
        ..BoxStyle::default()
    };
    let mark = ColorRgba8::rgba(255, 255, 255, opacity);
    let check_background = if value == SemanticCheckState::Checked {
        Background::Color(mark)
    } else {
        Background::None
    };
    let mixed_background = if value == SemanticCheckState::Mixed {
        Background::Color(mark)
    } else {
        Background::None
    };
    CheckboxStyles {
        container: BoxStyle {
            min_size: SizeRule2D {
                width: SizeRule::Logical(32.0),
                height: SizeRule::Logical(32.0),
            },
            padding: EdgeInsets::all(5.0),
            ..BoxStyle::default()
        },
        indicator,
        check_first: mark_segment(
            PointF { x: 20.0, y: 6.0 },
            PointF { x: 9.0, y: 17.0 },
            check_background,
        ),
        check_second: mark_segment(
            PointF { x: 9.0, y: 17.0 },
            PointF { x: 4.0, y: 12.0 },
            check_background,
        ),
        mixed: mark_segment(
            PointF { x: 5.0, y: 12.0 },
            PointF { x: 19.0, y: 12.0 },
            mixed_background,
        ),
    }
}

pub(super) fn mark_segment(start: PointF, end: PointF, background: Background) -> BoxStyle {
    const SCALE: f32 = 14.0 / 24.0;
    const OFFSET: f32 = 1.0;
    let dx = (end.x - start.x) * SCALE;
    let dy = (end.y - start.y) * SCALE;
    let length = dx.hypot(dy);
    let stroke = 2.0 * SCALE;
    BoxStyle {
        width: SizeRule::Logical(length),
        height: SizeRule::Logical(stroke),
        max_size: SizeRule2D {
            width: SizeRule::Logical(length),
            height: SizeRule::Logical(stroke),
        },
        decoration: crate::ui::BoxDecoration {
            background,
            corner_radii: CornerRadii::all(stroke * 0.5),
            ..crate::ui::BoxDecoration::default()
        },
        transform: Transform2D {
            translation: PointF {
                x: OFFSET + start.x * SCALE,
                y: OFFSET + start.y * SCALE - stroke * 0.5,
            },
            rotation: dy.atan2(dx),
            origin: PointF { x: 0.0, y: 0.5 },
            ..Transform2D::default()
        },
        ..BoxStyle::default()
    }
}

#[derive(Clone, Copy)]
pub(super) struct SwitchStyles {
    pub(super) container: BoxStyle,
    pub(super) track: BoxStyle,
    pub(super) thumb: BoxStyle,
}

pub(super) fn switch_styles(value: bool, enabled: bool) -> SwitchStyles {
    let opacity = if enabled { 255 } else { 180 };
    let track_color = if value {
        ColorRgba8::rgba(54, 104, 210, opacity)
    } else {
        ColorRgba8::rgba(75, 84, 102, opacity)
    };
    SwitchStyles {
        container: BoxStyle {
            min_size: SizeRule2D {
                width: SizeRule::Logical(32.0),
                height: SizeRule::Logical(32.0),
            },
            padding: EdgeInsets::all(5.0),
            ..BoxStyle::default()
        },
        track: BoxStyle {
            width: SizeRule::Logical(38.0),
            height: SizeRule::Logical(22.0),
            max_size: SizeRule2D {
                width: SizeRule::Logical(38.0),
                height: SizeRule::Logical(22.0),
            },
            padding: EdgeInsets::all(2.0),
            decoration: crate::ui::BoxDecoration {
                background: Background::Color(track_color),
                border: Border::all(1.0, ColorRgba8::rgba(118, 127, 145, opacity)),
                corner_radii: CornerRadii::all(11.0),
                ..crate::ui::BoxDecoration::default()
            },
            ..BoxStyle::default()
        },
        thumb: BoxStyle {
            width: SizeRule::Logical(16.0),
            height: SizeRule::Logical(16.0),
            max_size: SizeRule2D {
                width: SizeRule::Logical(16.0),
                height: SizeRule::Logical(16.0),
            },
            decoration: crate::ui::BoxDecoration {
                background: Background::Color(ColorRgba8::rgba(248, 249, 252, opacity)),
                corner_radii: CornerRadii::all(8.0),
                ..crate::ui::BoxDecoration::default()
            },
            transform: Transform2D {
                translation: PointF {
                    x: if value { 16.0 } else { 0.0 },
                    y: 0.0,
                },
                ..Transform2D::default()
            },
            ..BoxStyle::default()
        },
    }
}

#[derive(Clone, Copy)]
pub(super) struct SliderStyles {
    pub(super) container: BoxStyle,
    pub(super) track: BoxStyle,
    pub(super) fill: BoxStyle,
    pub(super) thumb: BoxStyle,
    pub(super) before_thumb: BoxStyle,
    pub(super) after_thumb: BoxStyle,
}

pub(super) fn slider_styles(value: f32, enabled: bool, width: SizeRule) -> SliderStyles {
    let value = value.clamp(0.0, 1.0);
    let opacity = if enabled { 255 } else { 180 };
    let accent = ColorRgba8::rgba(54, 104, 210, opacity);
    let track_width = if width == SizeRule::Shrink { SizeRule::Logical(160.0) } else { SizeRule::Fill(1.0) };
    SliderStyles {
        container: BoxStyle {
            width,
            height: SizeRule::Logical(32.0),
            min_size: SizeRule2D {
                width: SizeRule::Logical(32.0),
                height: SizeRule::Logical(32.0),
            },
            padding: EdgeInsets { top: 5.0, bottom: 5.0, left: 0.0, right: 0.0 },
            ..BoxStyle::default()
        },
        track: BoxStyle {
            width: track_width,
            height: SizeRule::Logical(6.0),
            max_size: SizeRule2D {
                width: track_width,
                height: SizeRule::Logical(6.0),
            },
            decoration: crate::ui::BoxDecoration {
                background: Background::Color(ColorRgba8::rgba(78, 87, 105, opacity)),
                corner_radii: CornerRadii::all(3.0),
                ..crate::ui::BoxDecoration::default()
            },
            ..BoxStyle::default()
        },
        fill: BoxStyle {
            width: SizeRule::Percent(value),
            height: SizeRule::Logical(6.0),
            decoration: crate::ui::BoxDecoration {
                background: Background::Color(accent),
                corner_radii: CornerRadii::all(3.0),
                ..crate::ui::BoxDecoration::default()
            },
            ..BoxStyle::default()
        },
        thumb: BoxStyle {
            width: SizeRule::Logical(18.0),
            height: SizeRule::Logical(18.0),
            max_size: SizeRule2D {
                width: SizeRule::Logical(18.0),
                height: SizeRule::Logical(18.0),
            },
            decoration: crate::ui::BoxDecoration {
                background: Background::Color(ColorRgba8::rgba(245, 247, 251, opacity)),
                border: Border::all(1.0, accent),
                corner_radii: CornerRadii::all(9.0),
                ..crate::ui::BoxDecoration::default()
            },
            ..BoxStyle::default()
        },
        before_thumb: BoxStyle { width: SizeRule::Fill(value), ..BoxStyle::default() },
        after_thumb: BoxStyle { width: SizeRule::Fill(1.0 - value), ..BoxStyle::default() },
    }
}

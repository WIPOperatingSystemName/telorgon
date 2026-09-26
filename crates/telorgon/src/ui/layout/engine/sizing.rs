use super::*;

pub(super) fn resolve_border_size(
    style: &BoxStyle,
    slot: RectF,
    intrinsic: SizeF,
    is_root: bool,
) -> SizeF {
    let border = border_insets(style);
    let chrome = SizeF {
        width: border.horizontal() + style.padding.horizontal(),
        height: border.vertical() + style.padding.vertical(),
    };
    let available_border = SizeF {
        width: (slot.width - style.margin.horizontal()).max(0.0),
        height: (slot.height - style.margin.vertical()).max(0.0),
    };
    if is_root {
        return available_border;
    }
    let available_specified = match style.sizing {
        BoxSizing::BorderBox => available_border,
        BoxSizing::ContentBox => SizeF {
            width: (available_border.width - chrome.width).max(0.0),
            height: (available_border.height - chrome.height).max(0.0),
        },
    };
    let intrinsic_specified = match style.sizing {
        BoxSizing::BorderBox => SizeF {
            width: intrinsic.width + chrome.width,
            height: intrinsic.height + chrome.height,
        },
        BoxSizing::ContentBox => intrinsic,
    };
    let mut specified = SizeF {
        width: constrain_size(
            resolve_size(
                style.width,
                available_specified.width,
                intrinsic_specified.width,
            ),
            style.min_size.width,
            style.max_size.width,
            available_specified.width,
            intrinsic_specified.width,
        ),
        height: constrain_size(
            resolve_size(
                style.height,
                available_specified.height,
                intrinsic_specified.height,
            ),
            style.min_size.height,
            style.max_size.height,
            available_specified.height,
            intrinsic_specified.height,
        ),
    };
    if let Some(ratio) = style
        .aspect_ratio
        .filter(|ratio| ratio.is_finite() && *ratio > 0.0)
    {
        if style.height == SizeRule::Shrink && style.width != SizeRule::Shrink {
            specified.height = constrain_size(
                specified.width / ratio,
                style.min_size.height,
                style.max_size.height,
                available_specified.height,
                intrinsic_specified.height,
            );
        } else if style.width == SizeRule::Shrink && style.height != SizeRule::Shrink {
            specified.width = constrain_size(
                specified.height * ratio,
                style.min_size.width,
                style.max_size.width,
                available_specified.width,
                intrinsic_specified.width,
            );
        }
    }
    match style.sizing {
        BoxSizing::BorderBox => specified,
        BoxSizing::ContentBox => SizeF {
            width: specified.width + chrome.width,
            height: specified.height + chrome.height,
        },
    }
}

pub(super) fn constrain_size(
    value: f32,
    min_rule: SizeRule,
    max_rule: SizeRule,
    available: f32,
    intrinsic: f32,
) -> f32 {
    let minimum = match min_rule {
        SizeRule::Shrink => 0.0,
        rule => resolve_size(rule, available, intrinsic),
    };
    let maximum = match max_rule {
        SizeRule::Shrink => intrinsic,
        rule => resolve_size(rule, available, intrinsic),
    }
    .max(minimum);
    value.max(minimum).min(maximum)
}

fn resolve_size(rule: SizeRule, available: f32, intrinsic: f32) -> f32 {
    match rule {
        SizeRule::Logical(value) => value.max(0.0),
        SizeRule::Percent(value) => available * value.clamp(0.0, 1.0),
        SizeRule::Fill(weight) => {
            if weight > 0.0 {
                available
            } else {
                0.0
            }
        }
        SizeRule::Shrink => intrinsic.min(available),
    }
}

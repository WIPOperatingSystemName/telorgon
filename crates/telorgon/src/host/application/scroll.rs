//! Default wheel behavior for retained scroll viewports, including nested scroll chaining.
use crate::{
    PointF,
    graphics::scene::NodeId,
    ui::{MountedUi, NodeKind, layout::LayoutEngine},
};

pub(super) fn wheel(
    ui: &mut MountedUi,
    layout: &LayoutEngine,
    target: NodeId,
    delta: PointF,
) -> bool {
    if !delta.x.is_finite() || !delta.y.is_finite() {
        return false;
    }
    let mut current = Some(target);
    while let Some(node) = current {
        current = ui.nodes.core(node).and_then(|core| core.parent);
        if ui.kinds.get(node) != Some(&NodeKind::Scroll) {
            continue;
        }
        let Some(viewport) = layout.computed(node) else {
            continue;
        };
        let mut style = ui.layouts.get(node).copied().unwrap_or_default();
        let mut right = 0.0f32;
        let mut bottom = 0.0f32;
        for child in ui.nodes.children(node) {
            if let Some(child) = layout.computed(child) {
                right = right.max(child.local_margin_rect.right());
                bottom = bottom.max(child.local_margin_rect.bottom());
            }
        }
        let next = PointF {
            x: (style.scroll_offset.x - delta.x)
                .clamp(0.0, (right - viewport.local_content_rect.width).max(0.0)),
            y: (style.scroll_offset.y - delta.y)
                .clamp(0.0, (bottom - viewport.local_content_rect.height).max(0.0)),
        };
        if next != style.scroll_offset {
            style.scroll_offset = next;
            return ui.set_layout_style(node, style);
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authoring::compose::{Component, Dimension, View, column};
    use crate::host::application::runtime::ComposedAppRuntime;
    use crate::{SizeI, foundation::MonotonicInstant, input::InputEvent};

    #[crate::component]
    struct ScrollFixture {}

    impl Component for ScrollFixture {
        fn view(&self) -> impl View {
            column()
                .width(Dimension::FILL)
                .height(Dimension::FILL)
                .scrollable()
                .children((0..6).map(|_| column().height(100.0)))
        }
    }

    #[test]
    fn wheel_requests_a_frame_without_component_updates() {
        let mut runtime = ComposedAppRuntime::from_composed_with_extent(
            ScrollFixture {},
            SizeI {
                width: 200,
                height: 150,
            },
        )
        .unwrap();
        runtime.prepare_frame(MonotonicInstant::ZERO, true).unwrap();
        runtime.queue_input(InputEvent::mouse_moved(PointF { x: 20.0, y: 20.0 }));
        runtime.flush_input(MonotonicInstant::from_nanos(1));
        runtime
            .prepare_frame(MonotonicInstant::from_nanos(2), true)
            .unwrap();

        for (time, delta) in [(3, -48.0), (5, 48.0)] {
            runtime.queue_input(InputEvent::mouse_scroll(PointF { x: 0.0, y: delta }));
            let outcome = runtime.flush_input(MonotonicInstant::from_nanos(time));
            assert!(!outcome.frame_needed_before);
            assert!(
                outcome.frame_needed_after,
                "wheel movement must schedule its own redraw"
            );
            let frame = runtime
                .prepare_frame(MonotonicInstant::from_nanos(time + 1), false)
                .unwrap();
            assert!(
                frame.changed,
                "scrolling must update the scene without a forced frame"
            );
        }
    }
}

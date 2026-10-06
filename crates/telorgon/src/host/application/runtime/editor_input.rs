use super::*;
use crate::foundation::RectF;
use crate::ui::{ControlBehavior, UiInputGeometry};

impl<D: ComponentDriver> AppRuntimeCore<D> {
    fn input_geometry(&self, node: NodeId, position: Option<PointF>) -> Option<UiInputGeometry> {
        let layout = self.layout.computed(node)?;
        let border = layout.local_border_rect;
        let position = position.and_then(|position| {
            layout.world_transform.inverse().map(|inverse| {
                let point = inverse.transform_point(position);
                PointF {
                    x: point.x - border.x,
                    y: point.y - border.y,
                }
            })
        });
        Some(UiInputGeometry {
            position,
            border_rect: RectF {
                x: 0.0,
                y: 0.0,
                width: border.width,
                height: border.height,
            },
            content_rect: RectF {
                x: layout.local_content_rect.x - border.x,
                y: layout.local_content_rect.y - border.y,
                width: layout.local_content_rect.width,
                height: layout.local_content_rect.height,
            },
        })
    }

    pub(super) fn dispatch_observed(
        &mut self,
        node: NodeId,
        kind: UiEventKind,
        mask: u16,
        timestamp: u64,
        position: Option<PointF>,
    ) {
        let geometry = self.input_geometry(node, position);
        self.view
            .dispatch_ui_observed(node, kind, mask, timestamp, geometry, self.modifiers);
    }

    pub(super) fn publish_editor_layouts(&mut self, timestamp: u64) -> bool {
        let next: std::collections::HashMap<_, _> = self
            .view
            .ui()
            .interactions
            .iter()
            .filter(|(_, state)| state.behavior == ControlBehavior::TextInput)
            .filter_map(|(node, _)| {
                self.input_geometry(node, None)
                    .map(|geometry| (node, geometry))
            })
            .collect();
        let changed: Vec<_> = next
            .iter()
            .filter(|(node, geometry)| self.editor_geometry.get(node) != Some(geometry))
            .map(|(node, geometry)| (*node, *geometry))
            .collect();
        self.editor_geometry = next;
        for (node, geometry) in &changed {
            self.view.dispatch_ui_observed(
                *node,
                UiEventKind::Layout(*geometry),
                u16::MAX,
                timestamp,
                Some(*geometry),
                self.modifiers,
            );
        }
        !changed.is_empty()
    }

    pub(crate) fn text_input_context(&self) -> Option<(NodeId, RectF)> {
        let node = self.interaction.focused()?;
        if !self.interaction.focus_eligible(self.view.ui(), node) {
            return None;
        }
        let state = self.view.ui().interactions.get(node)?;
        if state.behavior != ControlBehavior::TextInput
            || self
                .view
                .ui()
                .semantics
                .get(node)
                .is_some_and(|semantic| semantic.state.read_only)
        {
            return None;
        }
        let layout = self.layout.computed(node)?;
        let rect = if let Some(rect) = state.text_cursor_rect {
            RectF {
                x: rect.x + layout.local_border_rect.x,
                y: rect.y + layout.local_border_rect.y,
                ..rect
            }
        } else {
            RectF {
                x: layout.local_content_rect.x,
                y: layout.local_content_rect.y,
                width: 1.0,
                height: layout.local_content_rect.height,
            }
        };
        Some((node, layout.world_transform.transform_rect(rect)))
    }

    pub(super) fn dispatch_text_input(
        &mut self,
        target: NodeId,
        event: crate::input::TextInputEvent,
        timestamp: u64,
    ) {
        if self.interaction.focused() == Some(target)
            && self.interaction.focus_eligible(self.view.ui(), target)
            && self
                .view
                .ui()
                .interactions
                .get(target)
                .is_some_and(|state| {
                    state.behavior == ControlBehavior::TextInput && state.enabled && state.visible
                })
            && !self
                .view
                .ui()
                .semantics
                .get(target)
                .is_some_and(|semantic| semantic.state.read_only)
        {
            self.dispatch_observed(
                target,
                UiEventKind::Input(InputEvent::TextInput(event)),
                LISTEN_KEY,
                timestamp,
                None,
            );
        }
    }
}

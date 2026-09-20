use super::*;

impl<S: NativeRuntimeSource, P: NativePresentation> NativeHost<S, P> {
    pub(super) fn custom_chrome_action(&self) -> Option<crate::WindowAction> {
        if self.options.decorations != crate::host::application::WindowDecorationMode::Hidden {
            return None;
        }
        let runtime = self.runtime.as_ref()?;
        let snapshot = crate::WindowChromeSnapshot::derive(runtime.ui(), runtime.layout()).ok()?;
        match snapshot.hit_test(self.cursor_position.x, self.cursor_position.y) {
            Some(crate::WindowChromeRole::DragRegion) => Some(crate::WindowAction::BeginMove),
            Some(crate::WindowChromeRole::Action(action)) => Some(action),
            _ => None,
        }
    }

    pub(super) fn pointer_request(&self) -> crate::PointerRequest {
        let Some(runtime) = self.runtime.as_ref() else {
            return crate::PointerRequest::Semantic(crate::PointerIcon::Default);
        };
        if self.options.decorations == crate::host::application::WindowDecorationMode::Hidden
            && let Ok(snapshot) =
                crate::WindowChromeSnapshot::derive(runtime.ui(), runtime.layout())
            && let Some(region) =
                snapshot.hit_test_region(self.cursor_position.x, self.cursor_position.y)
        {
            if let Some(request) = runtime.ui().pointer_requests.get(region.node).copied() {
                return request;
            }
            match region.role {
                crate::WindowChromeRole::DragRegion => {
                    return crate::PointerRequest::Semantic(crate::PointerIcon::Move);
                }
                crate::WindowChromeRole::Action(crate::WindowAction::BeginResize(edge)) => {
                    return crate::PointerRequest::Semantic(resize_pointer_icon(edge));
                }
                crate::WindowChromeRole::Action(crate::WindowAction::BeginMove) => {
                    return crate::PointerRequest::Semantic(crate::PointerIcon::Move);
                }
                crate::WindowChromeRole::Action(_) => {
                    return crate::PointerRequest::Semantic(crate::PointerIcon::Pointer);
                }
                _ => {}
            }
        }

        runtime
            .ui()
            .pointer_requests
            .iter()
            .filter_map(|(node, request)| {
                runtime.layout().computed(node).and_then(|computed| {
                    (computed.visible_rect.contains(self.cursor_position)
                        && computed.border_rect.contains(self.cursor_position))
                    .then_some((*request, computed.border_rect.area()))
                })
            })
            .min_by(|left, right| left.1.total_cmp(&right.1))
            .map(|(request, _)| request)
            .unwrap_or(crate::PointerRequest::Semantic(crate::PointerIcon::Default))
    }

    pub(super) fn refresh_pointer(&mut self, event_loop: &ActiveEventLoop, now: Instant) -> bool {
        let request = self.pointer_request();
        let Some(window) = self.window.clone() else {
            return true;
        };
        let Some(pointer) = self.pointer.as_mut() else {
            return true;
        };
        if let Err(error) = pointer.apply(event_loop, &window, request, now) {
            self.fail(
                event_loop,
                format!("failed to apply managed pointer theme: {error}"),
            );
            return false;
        }
        true
    }

    pub(super) fn apply_custom_chrome_action(
        &mut self,
        event_loop: &ActiveEventLoop,
        action: crate::WindowAction,
    ) {
        let Some(window) = self.window.clone() else {
            return;
        };
        let result = match action {
            crate::WindowAction::Close => {
                if let Some(runtime) = self.runtime.as_mut()
                    && let Err(error) = S::close(runtime)
                {
                    self.fail(event_loop, format!("component close failed: {error}"));
                    return;
                }
                self.flush_commands();
                event_loop.exit();
                return;
            }
            crate::WindowAction::Minimize => {
                window.set_minimized(true);
                Ok(())
            }
            crate::WindowAction::ToggleMaximize => {
                window.set_maximized(!window.is_maximized());
                Ok(())
            }
            crate::WindowAction::BeginMove => window.drag_window(),
            crate::WindowAction::BeginResize(edge) => {
                window.drag_resize_window(winit_resize_direction(edge))
            }
            crate::WindowAction::ShowSystemMenu => Ok(()),
        };
        if let Err(error) = result {
            self.fail(
                event_loop,
                format!("custom window-frame action failed: {error}"),
            );
        }
    }
}

pub(super) fn winit_resize_direction(edge: crate::WindowResizeEdge) -> ResizeDirection {
    match edge {
        crate::WindowResizeEdge::Top => ResizeDirection::North,
        crate::WindowResizeEdge::TopRight => ResizeDirection::NorthEast,
        crate::WindowResizeEdge::Right => ResizeDirection::East,
        crate::WindowResizeEdge::BottomRight => ResizeDirection::SouthEast,
        crate::WindowResizeEdge::Bottom => ResizeDirection::South,
        crate::WindowResizeEdge::BottomLeft => ResizeDirection::SouthWest,
        crate::WindowResizeEdge::Left => ResizeDirection::West,
        crate::WindowResizeEdge::TopLeft => ResizeDirection::NorthWest,
    }
}

pub(super) fn resize_pointer_icon(edge: crate::WindowResizeEdge) -> crate::PointerIcon {
    match edge {
        crate::WindowResizeEdge::Top => crate::PointerIcon::NResize,
        crate::WindowResizeEdge::TopRight => crate::PointerIcon::NeResize,
        crate::WindowResizeEdge::Right => crate::PointerIcon::EResize,
        crate::WindowResizeEdge::BottomRight => crate::PointerIcon::SeResize,
        crate::WindowResizeEdge::Bottom => crate::PointerIcon::SResize,
        crate::WindowResizeEdge::BottomLeft => crate::PointerIcon::SwResize,
        crate::WindowResizeEdge::Left => crate::PointerIcon::WResize,
        crate::WindowResizeEdge::TopLeft => crate::PointerIcon::NwResize,
    }
}

pub(super) fn winit_cursor_icon(icon: crate::PointerIcon) -> CursorIcon {
    match icon {
        crate::PointerIcon::Default => CursorIcon::Default,
        crate::PointerIcon::ContextMenu => CursorIcon::ContextMenu,
        crate::PointerIcon::Help => CursorIcon::Help,
        crate::PointerIcon::Pointer => CursorIcon::Pointer,
        crate::PointerIcon::Progress => CursorIcon::Progress,
        crate::PointerIcon::Wait => CursorIcon::Wait,
        crate::PointerIcon::Cell => CursorIcon::Cell,
        crate::PointerIcon::Crosshair => CursorIcon::Crosshair,
        crate::PointerIcon::Text => CursorIcon::Text,
        crate::PointerIcon::VerticalText => CursorIcon::VerticalText,
        crate::PointerIcon::Alias => CursorIcon::Alias,
        crate::PointerIcon::Copy => CursorIcon::Copy,
        crate::PointerIcon::Move => CursorIcon::Move,
        crate::PointerIcon::NoDrop => CursorIcon::NoDrop,
        crate::PointerIcon::NotAllowed => CursorIcon::NotAllowed,
        crate::PointerIcon::Grab => CursorIcon::Grab,
        crate::PointerIcon::Grabbing => CursorIcon::Grabbing,
        crate::PointerIcon::EResize => CursorIcon::EResize,
        crate::PointerIcon::NResize => CursorIcon::NResize,
        crate::PointerIcon::NeResize => CursorIcon::NeResize,
        crate::PointerIcon::NwResize => CursorIcon::NwResize,
        crate::PointerIcon::SResize => CursorIcon::SResize,
        crate::PointerIcon::SeResize => CursorIcon::SeResize,
        crate::PointerIcon::SwResize => CursorIcon::SwResize,
        crate::PointerIcon::WResize => CursorIcon::WResize,
        crate::PointerIcon::EwResize => CursorIcon::EwResize,
        crate::PointerIcon::NsResize => CursorIcon::NsResize,
        crate::PointerIcon::NeswResize => CursorIcon::NeswResize,
        crate::PointerIcon::NwseResize => CursorIcon::NwseResize,
        crate::PointerIcon::ColResize => CursorIcon::ColResize,
        crate::PointerIcon::RowResize => CursorIcon::RowResize,
        crate::PointerIcon::AllScroll => CursorIcon::AllScroll,
        crate::PointerIcon::ZoomIn => CursorIcon::ZoomIn,
        crate::PointerIcon::ZoomOut => CursorIcon::ZoomOut,
        crate::PointerIcon::DndAsk => CursorIcon::DndAsk,
        crate::PointerIcon::AllResize => CursorIcon::AllResize,
    }
}

pub(super) fn mouse_button(button: MouseButton) -> PointerButton {
    match button {
        MouseButton::Left => PointerButton::PRIMARY,
        MouseButton::Right => PointerButton::SECONDARY,
        MouseButton::Middle => PointerButton::MIDDLE,
        MouseButton::Back => PointerButton::BACK,
        MouseButton::Forward => PointerButton::FORWARD,
        MouseButton::Other(value) => PointerButton::new(value),
    }
}

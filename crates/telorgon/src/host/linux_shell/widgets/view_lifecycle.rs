use super::*;

impl WidgetLayer {
    pub(super) fn update_input_visibility(&mut self, next: ShellSurfaceSpec, now: MonotonicInstant) {
        if next.visible && (!self.spec.visible || !self.initialized) {
            self.focused = next.focus == ShellFocus::OnOpen;
            // Attached children may be inactive until their parent bounds are available.
            self.layer.runtime.activate_view(now);
        }
        if self.spec.visible && !next.visible {
            self.layer.runtime.deactivate_view(now);
        }
        if !next.visible {
            self.focused = false;
            self.captured.clear();
        }
    }
}

pub(in super::super) fn sync_widget_focus(
    widgets: &mut [WidgetLayer],
    wayland: &mut NativeCompositor<'_>,
    display: &Display,
    windows: &BTreeMap<WaylandSurfaceId, ClientWindow>,
    locked: bool,
    saved: &mut Option<WaylandSurfaceId>,
    active: &mut bool,
    now: MonotonicInstant,
) -> AppResult<()> {
    if locked {
        for w in widgets {
            if w.focused || !w.captured.is_empty() {
                w.layer.runtime.deactivate_view(now);
            }
            w.focused = false;
            w.captured.clear();
        }
        *saved = None;
        *active = false;
        return Ok(());
    }
    let wants = widgets.iter().any(|w| w.spec.visible && w.focused);
    if wants && !*active {
        *saved = wayland
            .core()
            .seats
            .get(&1)
            .and_then(|s| s.keyboard_focus)
            .map(|f| f.surface);
        wayland
            .set_keyboard_focus(1, None, display.next_serial())
            .map_err(app_error)?;
    } else if !wants && *active {
        let restore = saved.take().filter(|id| {
            windows.get(id).is_some_and(|w| !w.minimized)
                && wayland.core().world.surface(*id).is_some()
        });
        wayland
            .set_keyboard_focus(1, restore, display.next_serial())
            .map_err(app_error)?;
    }
    *active = wants;
    Ok(())
}

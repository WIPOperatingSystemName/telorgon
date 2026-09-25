use super::*;

pub(in super::super) fn queue_widget_pointer_motion(
    widgets: &mut [WidgetLayer],
    p: PointF,
    now: MonotonicInstant,
    locked: bool,
) -> AppResult<bool> {
    if locked {
        for w in widgets {
            w.pending_motion = None;
            if !w.captured.is_empty() || w.focused {
                w.layer.runtime.deactivate_view(now);
            }
            if w.pointer_hovered {
                w.layer
                    .runtime
                    .shell_input(crate::input::InputEvent::mouse_moved(PointF {
                        x: -1_000_000.0,
                        y: -1_000_000.0,
                    }))?;
                w.pointer_hovered = false;
            }
            w.captured.clear();
            w.focused = false;
        }
        return Ok(false);
    }
    let hit = widgets
        .iter()
        .position(|w| !w.captured.is_empty() && w.input_visible())
        .or_else(|| {
            ordered(widgets)
                .into_iter()
                .find(|i| widgets[*i].contains(p))
        });
    for w in widgets.iter_mut() {
        if w.input_visible()
            && w.spec.dismiss_on_pointer_leave
            && w.captured.is_empty()
            && !w.hover_contains(p)
        {
            w.dismiss(ShellDismissReason::PointerLeft)?;
        }
    }
    for (i, w) in widgets.iter_mut().enumerate() {
        let local = if Some(i) == hit {
            w.local(p)
        } else {
            PointF {
                x: -1_000_000.0,
                y: -1_000_000.0,
            }
        };
        // Retain the freshest position per target, including one leave for the old
        // target. Unrelated widgets never enter the input runtime for this movement.
        if Some(i) == hit || w.pointer_hovered {
            w.pending_motion = Some(local);
        }
        w.pointer_hovered = Some(i) == hit;
    }
    Ok(hit.is_some())
}
/// Flush before discrete events and at the end of a libinput batch. Button/scroll
/// handlers therefore observe all preceding movement, with bounded UI work per batch.
pub(in super::super) fn flush_widget_pointer_motion(
    widgets: &mut [WidgetLayer],
    now: MonotonicInstant,
) -> AppResult<bool> {
    let mut repaint = false;
    for w in widgets {
        if let Some(position) = w.pending_motion.take() {
            let probe = stall_probe::begin();
            w.layer
                .runtime
                .shell_input(crate::input::InputEvent::mouse_moved(position))?;
            repaint |= w.layer.pointer_motion(position, now);
            stall_probe::finish("widget_motion", probe, || w.probe_context());
        }
    }
    Ok(repaint)
}

#[cfg(test)]
pub(super) fn widget_pointer_motion(
    widgets: &mut [WidgetLayer],
    p: PointF,
    now: MonotonicInstant,
    locked: bool,
) -> AppResult<bool> {
    let hit = queue_widget_pointer_motion(widgets, p, now, locked)?;
    flush_widget_pointer_motion(widgets, now)?;
    Ok(hit)
}

use super::*;

pub(in super::super) fn widget_pointer_button(
    widgets: &mut [WidgetLayer],
    p: PointF,
    button: u32,
    pressed: bool,
    now: MonotonicInstant,
    locked: bool,
) -> AppResult<bool> {
    if locked {
        return Ok(false);
    }
    flush_widget_pointer_motion(widgets, now)?;
    let order = ordered(widgets);
    let captured = widgets
        .iter()
        .position(|w| !w.captured.is_empty() && w.input_visible());
    if !pressed && !captured.is_some_and(|i| widgets[i].captured.contains(&button)) {
        return Ok(false);
    }
    let hit = captured.or_else(|| order.iter().copied().find(|i| widgets[*i].contains(p)));
    let mut dismissed = false;
    if pressed {
        for i in order {
            if Some(i) == hit {
                break;
            }
            let w = &mut widgets[i];
            if w.spec.visible && w.spec.dismiss_on_outside_press && w.outside_press(p) {
                w.dismiss(ShellDismissReason::OutsidePress)?;
                dismissed = true;
            }
        }
    }
    if let Some(i) = hit {
        if pressed && widgets[i].spec.dismiss_on_outside_press && widgets[i].outside_press(p) {
            widgets[i].dismiss(ShellDismissReason::OutsidePress)?;
            return Ok(true);
        }
        if pressed {
            let mut ancestors = BTreeSet::new();
            let mut parent = widgets[i].parent;
            while let Some(id) = parent {
                ancestors.insert(id);
                parent = widgets.iter().find(|w| w.id == id).and_then(|w| w.parent);
            }
            for (n, w) in widgets.iter_mut().enumerate() {
                let next = (n == i && w.spec.focus != ShellFocus::None)
                    || (ancestors.contains(&w.id) && w.focused);
                if w.focused && !next {
                    w.layer.runtime.deactivate_view(now);
                }
                w.focused = next;
            }
        }
        let w = &mut widgets[i];
        let probe = stall_probe::begin();
        w.layer.runtime.activate_view(now);
        w.layer.pointer_motion(w.local(p), now);
        let event = crate::input::InputEvent::mouse_button(
            match button {
                0x110 => crate::input::PointerButton::PRIMARY,
                0x111 => crate::input::PointerButton::SECONDARY,
                0x112 => crate::input::PointerButton::MIDDLE,
                0x113 | 0x116 => crate::input::PointerButton::BACK,
                0x114 | 0x115 => crate::input::PointerButton::FORWARD,
                code => u16::try_from(code)
                    .map(crate::input::PointerButton::from_platform_other)
                    .unwrap_or(crate::input::PointerButton::UNIDENTIFIED),
            },
            if pressed {
                crate::input::ButtonState::Pressed
            } else {
                crate::input::ButtonState::Released
            },
        );
        w.layer.runtime.shell_input(event.clone())?;
        w.layer.runtime.queue_input(event);
        w.layer.runtime.flush_input(now);
        if pressed {
            w.captured.insert(button);
        } else {
            w.captured.remove(&button);
        }
        stall_probe::finish("widget_button", probe, || w.probe_context());
        Ok(true)
    } else {
        Ok(dismissed)
    }
}

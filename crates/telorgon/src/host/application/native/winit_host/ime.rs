use super::*;
use crate::input::TextInputEvent;
use crate::ui::UiNodeId;

#[derive(Default)]
pub(super) struct NativeIme {
    target: Option<UiNodeId>,
    rect: Option<crate::foundation::RectF>,
    composing: bool,
    enabled: bool,
}

impl NativeIme {
    fn observe(&mut self, event: winit::event::Ime) -> Option<(UiNodeId, TextInputEvent)> {
        let target = self.target?;
        if matches!(event, winit::event::Ime::Enabled) {
            self.enabled = true;
            return None;
        }
        if matches!(event, winit::event::Ime::Disabled) {
            self.enabled = false;
        } else if !self.enabled {
            return None;
        }
        let event = match event {
            winit::event::Ime::Enabled => return None,
            winit::event::Ime::Preedit(text, selection) => {
                if text.len() > 1024 * 1024
                    || selection.is_some_and(|(start, end)| {
                        !text.is_char_boundary(start) || !text.is_char_boundary(end)
                    })
                {
                    return None;
                }
                // Winit clears preedit immediately before Commit; that empty update must
                // retain the composition owner and keep keyboard text suppressed.
                self.composing = true;
                TextInputEvent::Preedit { text, selection }
            }
            winit::event::Ime::Commit(text) => {
                self.composing = false;
                if text.len() > 1024 * 1024 {
                    return None;
                }
                TextInputEvent::Commit(text)
            }
            winit::event::Ime::Disabled => {
                if !std::mem::take(&mut self.composing) {
                    return None;
                }
                TextInputEvent::Cancel
            }
        };
        Some((target, event))
    }
}

impl<S: NativeRuntimeSource, P: NativePresentation> NativeHost<S, P> {
    pub(super) fn sync_text_input(&mut self) {
        let context = self
            .runtime
            .as_ref()
            .and_then(|runtime| runtime.text_input_context());
        let target = context.map(|(target, _)| target);
        let Some(window) = &self.window else {
            return;
        };
        if self.ime.target != target {
            if self.ime.target.is_some() {
                window.set_ime_allowed(false);
            }
            self.ime.target = target;
            self.ime.rect = None;
            self.ime.composing = false;
            self.ime.enabled = false;
            self.keyboard.set_composing(false);
            if target.is_some() {
                window.set_ime_allowed(true);
            }
        }
        if let Some((_, rect)) = context
            && self.ime.rect != Some(rect)
        {
            window.set_ime_cursor_area(
                winit::dpi::LogicalPosition::new(f64::from(rect.x), f64::from(rect.y)),
                winit::dpi::LogicalSize::new(
                    f64::from(rect.width.max(1.0)),
                    f64::from(rect.height.max(1.0)),
                ),
            );
            self.ime.rect = Some(rect);
        }
    }

    pub(super) fn ime_event(&mut self, event: winit::event::Ime) {
        if let Some((target, event)) = self.ime.observe(event)
            && let Some(runtime) = self.runtime.as_mut()
        {
            runtime.queue_input(PlatformInput::TextInput { target, event });
        }
        self.keyboard.set_composing(self.ime.composing);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_preedit_before_commit_keeps_the_same_editor_and_cancels_only_active_composition() {
        let mut ui = crate::ui::MountedUi::default();
        let node = {
            let mut writer = crate::ui::MountWriter::<()>::new(&mut ui);
            writer
                .root(Default::default(), Default::default(), |_| {})
                .0
        };
        let mut ime = NativeIme {
            target: Some(node),
            ..Default::default()
        };
        assert!(
            ime.observe(winit::event::Ime::Commit("old session".into()))
                .is_none()
        );
        assert!(
            ime.observe(winit::event::Ime::Preedit("old session".into(), None))
                .is_none()
        );
        assert!(ime.observe(winit::event::Ime::Enabled).is_none());
        assert!(
            matches!(ime.observe(winit::event::Ime::Preedit("é".into(), Some((3, 3)))),
            Some((target, TextInputEvent::Preedit { .. })) if target == node)
        );
        let _ = ime.observe(winit::event::Ime::Preedit(String::new(), None));
        assert!(ime.composing);
        assert_eq!(
            ime.observe(winit::event::Ime::Commit("é".into())),
            Some((node, TextInputEvent::Commit("é".into())))
        );
        assert!(!ime.composing);
        assert!(ime.observe(winit::event::Ime::Disabled).is_none());
        let _ = ime.observe(winit::event::Ime::Enabled);
        let _ = ime.observe(winit::event::Ime::Preedit("输入".into(), None));
        assert_eq!(
            ime.observe(winit::event::Ime::Disabled),
            Some((node, TextInputEvent::Cancel))
        );
        ime.target = None;
        assert!(
            ime.observe(winit::event::Ime::Commit("late".into()))
                .is_none()
        );
    }
}

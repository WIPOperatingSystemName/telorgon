use super::*;
use crate::platform::contracts::ClipboardKind;
use crate::services::clipboard::{Clipboard, ClipboardContent, ClipboardError, ClipboardRequest};
use crate::ui::text::{TextEdit, TextOffset};

/// Bound to a particular control instance, selection, focus epoch and revision.
pub struct TextClipboardTarget {
    identity: std::sync::Weak<()>,
    epoch: u64,
    revision: TextRevision,
    selection: TextSelection,
}
pub enum TextClipboardTransfer {
    Copy(ClipboardRequest<()>),
    Cut {
        publication: ClipboardRequest<()>,
        target: TextClipboardTarget,
    },
    Paste {
        target: TextClipboardTarget,
    },
}
impl TextField {
    /// Call on blur, dismissal, or replacement of the field's editing context.
    pub fn invalidate_clipboard_requests(&mut self) {
        self.clipboard_epoch = self.clipboard_epoch.wrapping_add(1);
    }
    pub fn clipboard_target(&self) -> TextClipboardTarget {
        TextClipboardTarget {
            identity: std::sync::Arc::downgrade(&self.clipboard_identity),
            epoch: self.clipboard_epoch,
            revision: self.controller.revision(),
            selection: self.controller.selection(),
        }
    }
    pub fn copy_selection(&self) -> Result<Option<String>, ClipboardError> {
        if self.mode.is_secure() || self.mode.is_disabled() {
            return Err(ClipboardError::Denied);
        }
        let snapshot = self.controller.snapshot();
        let range = snapshot.selection().range();
        if range.is_empty() {
            return Ok(None);
        }
        Ok(Some(
            snapshot
                .chunks_in(range)
                .map_err(|_| ClipboardError::Stale)?
                .map(|chunk| chunk.text)
                .collect::<String>(),
        ))
    }
    /// Keyboard shortcuts and context-menu actions share this admission path.
    /// Cut deletion is applied only after its publication has succeeded.
    pub fn clipboard_command(
        &self,
        clipboard: &Clipboard,
        action: crate::services::clipboard::ClipboardEditAction,
    ) -> Result<Option<TextClipboardTransfer>, ClipboardError> {
        use crate::services::clipboard::ClipboardEditAction::{Copy, Cut, Paste};
        match action {
            Copy | Cut => {
                if action == Cut && !self.mode.is_editable() {
                    return Err(ClipboardError::Denied);
                }
                let Some(text) = self.copy_selection()? else {
                    return Ok(None);
                };
                let publication =
                    clipboard.publish(ClipboardKind::System, ClipboardContent::text(text)?, None);
                Ok(Some(if action == Cut {
                    TextClipboardTransfer::Cut {
                        publication,
                        target: self.clipboard_target(),
                    }
                } else {
                    TextClipboardTransfer::Copy(publication)
                }))
            }
            Paste if self.mode.is_editable() => Ok(Some(TextClipboardTransfer::Paste {
                target: self.clipboard_target(),
            })),
            Paste => Err(ClipboardError::Denied),
            _ => Ok(None),
        }
    }
    pub fn paste_clipboard(
        &mut self,
        target: TextClipboardTarget,
        text: &str,
        now: MonotonicInstant,
    ) -> Result<TextFieldOutput, ClipboardError> {
        self.apply_clipboard_text(target, text, now, false, EditHistoryKind::Paste)
    }
    pub fn finish_clipboard_cut(
        &mut self,
        target: TextClipboardTarget,
        now: MonotonicInstant,
    ) -> Result<TextFieldOutput, ClipboardError> {
        if self.mode.is_secure() {
            return Err(ClipboardError::Denied);
        }
        self.apply_clipboard_text(target, "", now, false, EditHistoryKind::Cut)
    }
    pub(in crate::components::application::text) fn apply_clipboard_text(
        &mut self,
        target: TextClipboardTarget,
        text: &str,
        now: MonotonicInstant,
        multiline: bool,
        kind: EditHistoryKind,
    ) -> Result<TextFieldOutput, ClipboardError> {
        if !self.mode.is_editable() {
            return Err(ClipboardError::Denied);
        }
        if !target
            .identity
            .ptr_eq(&std::sync::Arc::downgrade(&self.clipboard_identity))
            || target.epoch != self.clipboard_epoch
            || target.revision != self.controller.revision()
            || target.selection != self.controller.selection()
        {
            return Err(ClipboardError::Stale);
        }
        if text.len() > crate::services::clipboard::MAX_BYTES {
            return Err(ClipboardError::TooLarge);
        }
        let text: String = if multiline {
            text.to_owned()
        } else {
            text.chars()
                .filter(|ch| !matches!(ch, '\r' | '\n'))
                .collect()
        };
        let range = target.selection.range();
        let end = range
            .start
            .bytes()
            .checked_add(u32::try_from(text.len()).map_err(|_| ClipboardError::TooLarge)?)
            .ok_or(ClipboardError::TooLarge)?;
        self.route_internal(
            TextFieldCommand::Edit {
                batch: TextEditBatch {
                    base_revision: target.revision,
                    edits: vec![TextEdit {
                        range,
                        replacement: text,
                    }],
                    selection: TextSelection::collapsed(
                        TextOffset::from_bytes(end),
                        target.selection.affinity,
                    ),
                    composition: None,
                },
                kind,
                recorded_at: now,
            },
            multiline,
        )
        .map_err(|_| ClipboardError::TransferFailed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn field(mode: TextFieldMode) -> TextField {
        TextField::new(
            TextController::from_text("Café".to_owned()).unwrap(),
            "Example",
            mode,
        )
        .unwrap()
    }
    #[test]
    fn clipboard_paste_is_bound_to_instance_focus_and_revision() {
        let mut a = field(TextFieldMode::Editable);
        let mut b = field(TextFieldMode::Editable);
        let target = a.clipboard_target();
        assert!(matches!(
            b.paste_clipboard(target, "wrong", MonotonicInstant::from_nanos(1)),
            Err(ClipboardError::Stale)
        ));
        let target = a.clipboard_target();
        a.invalidate_clipboard_requests();
        assert!(matches!(
            a.paste_clipboard(target, "late", MonotonicInstant::from_nanos(2)),
            Err(ClipboardError::Stale)
        ));
        let old = a.clipboard_target();
        let target = a.clipboard_target();
        a.paste_clipboard(target, "🦊", MonotonicInstant::from_nanos(3))
            .unwrap();
        assert!(matches!(
            a.paste_clipboard(old, "late", MonotonicInstant::from_nanos(4)),
            Err(ClipboardError::Stale)
        ));
    }
    #[test]
    fn clipboard_secure_and_readonly_policy_is_enforced() {
        let mut secure = field(TextFieldMode::Secure);
        assert_eq!(secure.copy_selection(), Err(ClipboardError::Denied));
        let target = secure.clipboard_target();
        secure
            .paste_clipboard(target, "allowed", MonotonicInstant::from_nanos(1))
            .unwrap();
        let mut readonly = field(TextFieldMode::ReadOnly);
        let target = readonly.clipboard_target();
        assert!(matches!(
            readonly.paste_clipboard(target, "blocked", MonotonicInstant::from_nanos(1)),
            Err(ClipboardError::Denied)
        ));
    }
}

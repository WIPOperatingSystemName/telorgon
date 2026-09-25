use super::*;
/// Editing identity captured before an asynchronous paste. A response is only
/// applicable to this revision and selection of this particular editor.
#[derive(Clone, Debug)]
pub struct PasteTarget {
    identity: Arc<()>,
    revision: u64,
    anchor: usize,
    cursor: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardEditAction {
    None,
    Changed,
    Copy,
    Cut,
    Paste,
}
#[derive(Clone, Default)]
pub struct ClipboardText {
    identity: Arc<()>,
    pub text: String,
    pub anchor: usize,
    pub cursor: usize,
    revision: u64,
    pub read_only: bool,
    pub secure: bool,
    history: Vec<(String, usize, usize)>,
    redo: Vec<(String, usize, usize)>,
}
impl ClipboardText {
    pub fn key(&mut self, key: &crate::input::KeyEvent) -> ClipboardEditAction {
        use crate::input::{ButtonState, LogicalKey, Modifiers, NamedKey};
        use ClipboardEditAction::*;
        if key.state != ButtonState::Pressed {
            return None;
        }
        if !self.text.is_char_boundary(self.cursor) || !self.text.is_char_boundary(self.anchor) {
            return None;
        }
        if key
            .modifiers
            .intersects(Modifiers::ALT.union(Modifiers::SUPER))
        {
            return None;
        }
        if key.modifiers.contains(Modifiers::CONTROL) {
            let LogicalKey::Character(ch) = &key.logical_key else {
                return None;
            };
            if key.repeat && matches!(ch.as_str().to_ascii_lowercase().as_str(), "c" | "x" | "v") {
                return None;
            }
            return match ch.as_str().to_ascii_lowercase().as_str() {
                "a" => {
                    self.select_all();
                    Changed
                }
                "c" => Copy,
                "x" => Cut,
                "v" => Paste,
                "z" => {
                    self.undo(key.modifiers.contains(Modifiers::SHIFT));
                    Changed
                }
                "y" => {
                    self.undo(true);
                    Changed
                }
                _ => None,
            };
        }
        match key.logical_key {
            LogicalKey::Named(
                NamedKey::ArrowLeft | NamedKey::ArrowRight | NamedKey::Home | NamedKey::End,
            ) => {
                let shift = key.modifiers.contains(Modifiers::SHIFT);
                self.cursor = match key.logical_key {
                    LogicalKey::Named(NamedKey::Home) => 0,
                    LogicalKey::Named(NamedKey::End) => self.text.len(),
                    LogicalKey::Named(NamedKey::ArrowLeft) => {
                        if !shift && !self.selection().is_empty() {
                            self.selection().start
                        } else {
                            self.text[..self.cursor]
                                .char_indices()
                                .last()
                                .map_or(0, |(i, _)| i)
                        }
                    }
                    _ => {
                        if !shift && !self.selection().is_empty() {
                            self.selection().end
                        } else {
                            self.cursor
                                + self.text[self.cursor..]
                                    .chars()
                                    .next()
                                    .map_or(0, char::len_utf8)
                        }
                    }
                };
                if !shift {
                    self.anchor = self.cursor;
                }
                self.invalidate_paste();
                Changed
            }
            LogicalKey::Named(NamedKey::Backspace | NamedKey::Delete) => {
                if self.read_only {
                    return None;
                }
                if self.selection().is_empty() {
                    self.anchor = if key.logical_key == LogicalKey::Named(NamedKey::Backspace) {
                        self.text[..self.cursor]
                            .char_indices()
                            .last()
                            .map_or(0, |(i, _)| i)
                    } else {
                        self.cursor
                            + self.text[self.cursor..]
                                .chars()
                                .next()
                                .map_or(0, char::len_utf8)
                    };
                }
                let _ = self.replace_selection("");
                Changed
            }
            _ => {
                let Some(value) = &key.text else { return None };
                let value: String = value
                    .as_str()
                    .chars()
                    .filter(|ch| !ch.is_control())
                    .collect();
                if value.is_empty() {
                    return None;
                }
                if self.replace_selection(&value).is_ok() {
                    Changed
                } else {
                    None
                }
            }
        }
    }
    pub fn selection(&self) -> std::ops::Range<usize> {
        self.anchor.min(self.cursor)..self.anchor.max(self.cursor)
    }
    pub fn selected_text(&self) -> Option<&str> {
        if self.secure {
            None
        } else {
            self.text
                .get(self.selection())
                .filter(|text| !text.is_empty())
        }
    }
    pub fn target(&self) -> PasteTarget {
        PasteTarget {
            identity: self.identity.clone(),
            revision: self.revision,
            anchor: self.anchor,
            cursor: self.cursor,
        }
    }
    pub fn invalidate_paste(&mut self) {
        self.revision = self
            .revision
            .checked_add(1)
            .expect("clipboard editor revision exhausted");
    }
    pub fn paste(&mut self, target: PasteTarget, text: &str) -> Result<()> {
        if self.target() != target {
            return Err(ClipboardError::Stale);
        }
        self.replace_selection(text)
    }
    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.cursor = self.text.len();
        self.invalidate_paste();
    }
    pub fn replace_selection(&mut self, text: &str) -> Result<()> {
        if self.read_only {
            return Err(ClipboardError::Denied);
        }
        let range = self.selection();
        if self.text.get(range.clone()).is_none() {
            return Err(ClipboardError::Stale);
        }
        if self.text.len() - range.len() + text.len() > MAX_BYTES {
            return Err(ClipboardError::TooLarge);
        }
        if !self.secure {
            while !self.history.is_empty()
                && (self.history.len() >= 32
                    || self
                        .history
                        .iter()
                        .map(|(text, _, _)| text.len())
                        .sum::<usize>()
                        + self.text.len()
                        > 4 * 1024 * 1024)
            {
                self.history.remove(0);
            }
            if self.text.len() <= 4 * 1024 * 1024 {
                self.history
                    .push((self.text.clone(), self.anchor, self.cursor));
            }
            self.redo.clear();
        }
        self.text.replace_range(range.clone(), text);
        self.cursor = range.start + text.len();
        self.anchor = self.cursor;
        self.invalidate_paste();
        Ok(())
    }
    pub fn undo(&mut self, redo: bool) -> bool {
        if self.read_only || self.secure {
            return false;
        }
        let (from, to) = if redo {
            (&mut self.redo, &mut self.history)
        } else {
            (&mut self.history, &mut self.redo)
        };
        let Some((text, anchor, cursor)) = from.pop() else {
            return false;
        };
        to.push((
            std::mem::replace(&mut self.text, text),
            self.anchor,
            self.cursor,
        ));
        self.anchor = anchor;
        self.cursor = cursor;
        self.invalidate_paste();
        true
    }
}

impl std::fmt::Debug for ClipboardText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClipboardText")
            .field("bytes", &self.text.len())
            .field("secure", &self.secure)
            .finish_non_exhaustive()
    }
}

impl PartialEq for PasteTarget {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.identity, &other.identity)
            && self.revision == other.revision
            && self.anchor == other.anchor
            && self.cursor == other.cursor
    }
}
impl Eq for PasteTarget {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{ButtonState, KeyEvent, LogicalKey, Modifiers, PhysicalKey};
    fn control(value: &str) -> KeyEvent {
        let mut event = KeyEvent::new(PhysicalKey::UNIDENTIFIED, ButtonState::Pressed)
            .with_logical_key(LogicalKey::character(value).unwrap());
        event.modifiers = Modifiers::CONTROL;
        event
    }
    #[test]
    fn clipboard_keys_preserve_unicode_selections_and_undo() {
        let mut editor = ClipboardText::default();
        editor.replace_selection("Café 🦊").unwrap();
        assert_eq!(editor.key(&control("a")), ClipboardEditAction::Changed);
        assert_eq!(editor.selected_text(), Some("Café 🦊"));
        assert_eq!(editor.key(&control("c")), ClipboardEditAction::Copy);
        assert_eq!(editor.key(&control("x")), ClipboardEditAction::Cut);
        assert_eq!(editor.key(&control("v")), ClipboardEditAction::Paste);
        let target = editor.target();
        editor.paste(target, "replacement").unwrap();
        editor.key(&control("z"));
        assert_eq!(editor.text, "Café 🦊");
        editor.key(&control("y"));
        assert_eq!(editor.text, "replacement");
        assert_eq!(
            editor.key(&control("v").with_repeat(true)),
            ClipboardEditAction::None
        );
    }
    #[test]
    fn clipboard_paste_rejects_other_editors_and_changed_focus() {
        let mut a = ClipboardText::default();
        let mut b = ClipboardText::default();
        assert_eq!(b.paste(a.target(), "wrong"), Err(ClipboardError::Stale));
        let target = a.target();
        a.invalidate_paste();
        assert_eq!(a.paste(target, "late"), Err(ClipboardError::Stale));
        a.read_only = true;
        assert_eq!(a.replace_selection("blocked"), Err(ClipboardError::Denied));
    }
}

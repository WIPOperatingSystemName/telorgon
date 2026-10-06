use super::*;
use crate::ui::text::{
    TextAffinity, TextNavigationDirection, TextNavigationUnit, TextOffset, TextRevision,
    TextSelection, TextSelectionAdjustment, TextSnapshot,
};
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
    /// Replaces the document baseline, clearing undo and invalidating pending clipboard replies.
    pub fn reset(&mut self, text: impl Into<String>) {
        let text = text.into();
        *self = Self {
            cursor: text.len(),
            anchor: text.len(),
            text,
            read_only: self.read_only,
            secure: self.secure,
            ..Self::default()
        };
    }
    fn navigate(&self, backward: bool, word: bool, extend: bool) -> TextSelection {
        let snapshot = TextSnapshot::from_parts(
            Arc::from(self.text.as_str()),
            TextRevision(self.revision),
            TextSelection::collapsed(TextOffset::ZERO, TextAffinity::Downstream),
            None,
        );
        snapshot
            .navigate_selection(
                TextSelection {
                    anchor: TextOffset(self.anchor as u32),
                    active: TextOffset(self.cursor as u32),
                    affinity: TextAffinity::Downstream,
                },
                if word {
                    TextNavigationUnit::Word
                } else {
                    TextNavigationUnit::Grapheme
                },
                if backward {
                    TextNavigationDirection::Backward
                } else {
                    TextNavigationDirection::Forward
                },
                if extend {
                    TextSelectionAdjustment::Extend
                } else {
                    TextSelectionAdjustment::Move
                },
                TextAffinity::Downstream,
            )
            .expect("validated editor selection")
    }
    pub fn key(&mut self, key: &crate::input::KeyEvent) -> ClipboardEditAction {
        use crate::input::{ButtonState, LogicalKey, Modifiers, NamedKey};
        use ClipboardEditAction::*;
        if key.state != ButtonState::Pressed {
            return None;
        }
        if !self.text.is_char_boundary(self.cursor) || !self.text.is_char_boundary(self.anchor) {
            return None;
        }
        let alt_graph = key.modifiers.contains(Modifiers::ALT_GRAPH);
        if key.modifiers.contains(Modifiers::SUPER)
            || (key.modifiers.contains(Modifiers::ALT) && !alt_graph)
        {
            return None;
        }
        if key.modifiers.contains(Modifiers::CONTROL)
            && !alt_graph
            && matches!(key.logical_key, LogicalKey::Character(_))
        {
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
                match key.logical_key {
                    LogicalKey::Named(NamedKey::Home) => self.cursor = 0,
                    LogicalKey::Named(NamedKey::End) => self.cursor = self.text.len(),
                    _ => {
                        let selection = self.navigate(
                            key.logical_key == LogicalKey::Named(NamedKey::ArrowLeft),
                            key.modifiers.contains(Modifiers::CONTROL),
                            shift,
                        );
                        self.cursor = selection.active.as_usize();
                    }
                }
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
                let original = (self.anchor, self.cursor);
                if self.selection().is_empty() {
                    self.anchor = self
                        .navigate(
                            key.logical_key == LogicalKey::Named(NamedKey::Backspace),
                            key.modifiers.contains(Modifiers::CONTROL),
                            true,
                        )
                        .active
                        .as_usize();
                }
                if self.selection().is_empty() {
                    return None;
                }
                if self.replace_selection("").is_err() {
                    (self.anchor, self.cursor) = original;
                    return None;
                }
                // Undo restores the caret before deletion, not the temporary deletion range.
                if let Some((_, anchor, cursor)) = self.history.last_mut() {
                    (*anchor, *cursor) = original;
                }
                Changed
            }
            _ => {
                if key.modifiers.contains(Modifiers::CONTROL) && !alt_graph {
                    return None;
                }
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
    fn named(key: crate::input::NamedKey, modifiers: Modifiers) -> KeyEvent {
        KeyEvent::new(PhysicalKey::UNIDENTIFIED, ButtonState::Pressed)
            .with_logical_key(LogicalKey::Named(key))
            .with_modifiers(modifiers)
    }
    #[test]
    fn deletion_and_arrows_follow_grapheme_and_word_boundaries() {
        use crate::input::NamedKey;
        let mut editor = ClipboardText::default();
        editor.reset("a e\u{301} 👨‍👩‍👧‍👦");
        let end = editor.cursor;
        editor.key(&named(NamedKey::ArrowLeft, Modifiers::empty()));
        assert_eq!(&editor.text[editor.cursor..], "👨‍👩‍👧‍👦");
        editor.key(&named(NamedKey::End, Modifiers::empty()));
        editor.key(&named(NamedKey::Backspace, Modifiers::empty()));
        assert_eq!(editor.text, "a e\u{301} ");
        editor.key(&control("z"));
        assert_eq!(editor.cursor, end);
        assert_eq!(editor.anchor, end);
        editor.reset("alpha beta");
        editor.key(&named(NamedKey::ArrowLeft, Modifiers::CONTROL));
        assert_eq!(editor.cursor, 6);
        editor.key(&named(
            NamedKey::ArrowRight,
            Modifiers::CONTROL.union(Modifiers::SHIFT),
        ));
        assert_eq!(editor.selected_text(), Some("beta"));
        editor.key(&named(NamedKey::Home, Modifiers::CONTROL));
        editor.key(&named(NamedKey::Delete, Modifiers::CONTROL));
        assert_eq!(editor.text, " beta");
        editor.key(&named(NamedKey::End, Modifiers::empty()));
        editor.key(&named(NamedKey::Backspace, Modifiers::CONTROL));
        assert_eq!(editor.text, " ");
    }
    #[test]
    fn reset_clears_undo_and_rejects_pending_paste() {
        let mut editor = ClipboardText::default();
        editor.reset("first location");
        editor.select_all();
        editor.replace_selection("edited").unwrap();
        let pending = editor.target();
        editor.reset("second location");
        editor.key(&control("z"));
        assert_eq!(editor.text, "second location");
        assert_eq!(editor.paste(pending, "late"), Err(ClipboardError::Stale));
    }

    #[test]
    fn alt_graph_inserts_produced_text_without_triggering_control_shortcuts() {
        let mut editor = ClipboardText::default();
        let event = control("a")
            .with_modifiers(
                Modifiers::CONTROL
                    .union(Modifiers::ALT)
                    .union(Modifiers::ALT_GRAPH),
            )
            .with_text(Some(crate::input::KeyText::new("@").unwrap()));
        assert_eq!(editor.key(&event), ClipboardEditAction::Changed);
        assert_eq!(editor.text, "@");
        assert_eq!(editor.anchor, editor.cursor);
    }
}

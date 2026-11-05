//! Selecting text with the pointer: drag to select, double click for a
//! word, triple click for everything, shift-click to extend.
//!
//! Only `Text` marked selectable takes part, and only outside pressables
//! (a press wins over a selection). Offsets are UTF-16 code units, the unit
//! backends hit-test and paint in.

use pam_desktop_wire::PropKey;

use crate::backend::NodeId;
use crate::context::{Ctx, HostCommand};
use crate::scene::{Change, Scene};
use crate::text::{char_to_utf16_offset, utf16_len, utf16_to_char_offset};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Granularity {
    Character,
    Word,
    All,
}

struct Selected {
    id: NodeId,
    /// The text when the selection began, to slice and to find words.
    text: String,
    /// Where the gesture started: a caret, or the word first clicked.
    anchor: (u32, u32),
    range: (u32, u32),
    granularity: Granularity,
}

#[derive(Default)]
pub struct TextSelection {
    selected: Option<Selected>,
    dragging: bool,
}

/// The word around a UTF-16 offset: a run of alphanumerics, or the single
/// character there.
fn word_at(text: &str, offset: u32) -> (u32, u32) {
    let characters: Vec<char> = text.chars().collect();
    if characters.is_empty() {
        return (0, 0);
    }
    let index = (utf16_to_char_offset(text, offset) as usize).min(characters.len() - 1);
    let is_word = |character: char| character.is_alphanumeric() || character == '_' || character == '\'';
    let (mut start, mut end) = (index, index + 1);
    if is_word(characters[index]) {
        while start > 0 && is_word(characters[start - 1]) {
            start -= 1;
        }
        while end < characters.len() && is_word(characters[end]) {
            end += 1;
        }
    }
    (char_to_utf16_offset(text, start as u32), char_to_utf16_offset(text, end as u32))
}

impl TextSelection {
    /// The selectable text under a window position, if a press there should
    /// select instead of pressing.
    #[must_use]
    pub fn target(scene: &Scene, id: NodeId) -> Option<NodeId> {
        let node = scene.node(id)?;
        if !node.is_text_bearing() || !node.props().flag_or(PropKey::TextSelectable, false) {
            return None;
        }
        let mut current = id;
        while let Some(node) = scene.node(current) {
            if node.is_pressable() || node.handles(PropKey::WindowDrag) {
                return None;
            }
            if node.parent == 0 {
                break;
            }
            current = node.parent;
        }
        Some(id)
    }

    #[must_use]
    pub fn node(&self) -> Option<NodeId> {
        self.selected.as_ref().map(|selected| selected.id)
    }

    #[must_use]
    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    /// The selected text, when the selection is not empty.
    #[must_use]
    pub fn text(&self) -> Option<String> {
        let selected = self.selected.as_ref()?;
        let (start, end) = selected.range;
        if start >= end {
            return None;
        }
        let units: Vec<u16> = selected.text.encode_utf16().collect();
        let slice = units.get(start as usize..(end as usize).min(units.len()))?;
        Some(String::from_utf16_lossy(slice))
    }

    pub fn on_change(&mut self, ctx: &mut Ctx<'_>, change: Change) {
        let Some(id) = self.node() else { return };
        match change {
            Change::Removed(removed, _) if removed == id => {
                self.selected = None;
                self.dragging = false;
            }
            // New text invalidates the offsets.
            Change::Updated(updated, PropKey::Text | PropKey::TextSelectable) if updated == id => self.clear(ctx),
            _ => {}
        }
    }

    /// A press on selectable text at `index`.
    pub fn begin(&mut self, ctx: &mut Ctx<'_>, id: NodeId, index: u32, click_count: u8, extend: bool) {
        let text = ctx.scene.plain_text(id).unwrap_or_default();
        let length = utf16_len(&text);
        let index = index.min(length);
        if extend {
            if let Some(selected) = self.selected.as_mut().filter(|selected| selected.id == id) {
                selected.granularity = Granularity::Character;
                self.dragging = true;
                return self.extend(ctx, index);
            }
        }
        if self.node().is_some_and(|current| current != id) {
            self.clear(ctx);
        }
        let (granularity, anchor) = match click_count {
            0 | 1 => (Granularity::Character, (index, index)),
            2 => (Granularity::Word, word_at(&text, index)),
            _ => (Granularity::All, (0, length)),
        };
        self.selected = Some(Selected { id, text, anchor, range: anchor, granularity });
        self.dragging = true;
        self.push(ctx);
    }

    /// The pointer moved to `index` while selecting.
    pub fn extend(&mut self, ctx: &mut Ctx<'_>, index: u32) {
        let Some(selected) = &mut self.selected else { return };
        let length = utf16_len(&selected.text);
        let index = index.min(length);
        let range = match selected.granularity {
            Granularity::All => (0, length),
            Granularity::Character => (selected.anchor.0.min(index), selected.anchor.1.max(index)),
            Granularity::Word => {
                let word = word_at(&selected.text, index);
                (selected.anchor.0.min(word.0), selected.anchor.1.max(word.1))
            }
        };
        if range != selected.range {
            selected.range = range;
            self.push(ctx);
        }
    }

    pub fn end(&mut self) {
        self.dragging = false;
    }

    pub fn select_all(&mut self, ctx: &mut Ctx<'_>) {
        if let Some(selected) = &mut self.selected {
            selected.range = (0, utf16_len(&selected.text));
            selected.anchor = selected.range;
            self.push(ctx);
        }
    }

    pub fn clear(&mut self, ctx: &mut Ctx<'_>) {
        self.dragging = false;
        if let Some(selected) = self.selected.take() {
            ctx.commands.push(HostCommand::SetTextSelection { id: selected.id, range: None });
        }
    }

    fn push(&self, ctx: &mut Ctx<'_>) {
        if let Some(selected) = &self.selected {
            let range = (selected.range.0 < selected.range.1).then_some(selected.range);
            ctx.commands.push(HostCommand::SetTextSelection { id: selected.id, range });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_are_runs_of_letters_and_digits() {
        let text = "Olá, mundo_1 😀 fim";
        assert_eq!(word_at(text, 1), (0, 3));
        assert_eq!(word_at(text, 3), (3, 4));
        assert_eq!(word_at(text, 6), (5, 12));
        // The emoji is two UTF-16 units and a word of its own.
        assert_eq!(word_at(text, 13), (13, 15));
        assert_eq!(word_at(text, 99), (16, 19));
        assert_eq!(word_at("", 0), (0, 0));
    }
}

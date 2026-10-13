//! Embed mode: the editor inside another app's window (the web build's `WordcraftEmbed`, which
//! Memento uses to edit Word files). The host owns the file: there is no title bar, status bar,
//! side panes, Backstage, Start screen or AutoSave, and Save asks the host
//! ([`EmbedEvent::SaveShortcut`]). The host reads and selects the body text by UTF-16 offsets
//! ([`TextIndex`]) and tints passages of it ([`WordApp::set_decorations`]) without changing the
//! document.

use std::sync::Arc;

use egui::Color32;
use serde_json::{Value, json};
use wordcraft_engine::doc::Pos;
use wordcraft_engine::text_index::remap_ranges;
use wordcraft_engine::{Selection, TextIndex};

use crate::WordApp;

/// Most decorations kept (a host's junk can't make every frame slow).
pub const MAX_DECORATIONS: usize = 10_000;

/// Something the host is told about, after the frame it happened in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmbedEvent {
    /// The document changed (an edit, Undo, Redo).
    Change,
    /// The selection changed.
    Selection,
    /// Save was asked for (⌘S / Ctrl+S, the Save button): the host saves.
    SaveShortcut,
}

impl EmbedEvent {
    /// The event's name in the host's API.
    pub fn name(self) -> &'static str {
        match self {
            EmbedEvent::Change => "change",
            EmbedEvent::Selection => "selection",
            EmbedEvent::SaveShortcut => "save-shortcut",
        }
    }
}

/// A tinted range of the text, UTF-16 offsets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decoration {
    pub start: usize,
    pub end: usize,
    pub color: Color32,
}

/// Which document and revision something was worked out for.
type Key = (u64, u64);

/// Embed mode's state ([`WordApp::embed`]).
#[derive(Default)]
pub struct EmbedState {
    index: Option<(Key, Arc<TextIndex>)>,
    decorations: Vec<Decoration>,
    /// The revision the decorations' offsets are for, and its text.
    decorated: Option<(Key, String)>,
    /// The decorations as document ranges, for the revision they were worked out for.
    painted: Option<(Key, Arc<Vec<(Pos, Pos, Color32)>>)>,
    save_requested: bool,
    /// What the host was last told about: the document and revision, and the selection.
    seen: Option<(Key, Selection)>,
}

/// A colour from the host: `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`. Without an alpha it is
/// drawn see-through, so the text under the tint stays readable.
pub fn parse_color(s: &str) -> Option<Color32> {
    let hex = s.trim().strip_prefix('#')?;
    if !hex.is_ascii() {
        return None;
    }
    let digit = |i: usize| hex.get(i..i + 1).and_then(|d| u8::from_str_radix(d, 16).ok());
    let pair = |i: usize| hex.get(i..i + 2).and_then(|d| u8::from_str_radix(d, 16).ok());
    let (r, g, b, a) = match hex.len() {
        3 | 4 => {
            let n = |i| digit(i).map(|d| d * 17);
            (n(0)?, n(1)?, n(2)?, if hex.len() == 4 { n(3)? } else { DEFAULT_ALPHA })
        }
        6 | 8 => (pair(0)?, pair(2)?, pair(4)?, if hex.len() == 8 { pair(6)? } else { DEFAULT_ALPHA }),
        _ => return None,
    };
    Some(Color32::from_rgba_unmultiplied(r, g, b, a))
}

/// The alpha of a decoration colour given without one.
const DEFAULT_ALPHA: u8 = 0x59;

impl WordApp {
    /// Switch to embed mode (see the module docs). The host calls this once, before the first
    /// frame.
    pub fn embedded(&mut self) {
        self.embed = Some(EmbedState::default());
        self.autosave = false;
        self.session.autosave = false;
        self.ui.backstage = false;
        self.ui.start_screen = false;
    }

    pub fn is_embedded(&self) -> bool {
        self.embed.is_some()
    }

    fn key(&self) -> Key {
        (self.session.document_id(), self.session.rev())
    }

    /// The body text and its offsets, for the current revision.
    pub fn text_index(&mut self) -> Arc<TextIndex> {
        let key = self.key();
        if let Some(state) = &self.embed
            && let Some((k, ix)) = &state.index
            && *k == key
        {
            return ix.clone();
        }
        let ix = Arc::new(TextIndex::new(&self.session.doc));
        if let Some(state) = self.embed.as_mut() {
            state.index = Some((key, ix.clone()));
        }
        ix
    }

    /// Open a document from its bytes (`name` gives the format), replacing the open one without
    /// asking. The decorations go: they were for the old document.
    pub fn embed_open(&mut self, name: &str, bytes: &[u8]) -> Result<Value, String> {
        let data = wordcraft_engine::cmd::insert::base64_encode(bytes);
        let r = self.execute("file.open", json!({"path": name, "data": data}))?;
        self.ui.backstage = false;
        self.dialog = None;
        let key = self.key();
        let sel = self.session.sel.clone();
        if let Some(state) = self.embed.as_mut() {
            state.decorations.clear();
            state.decorated = None;
            state.painted = None;
            // The host knows it opened a document: no change or selection event for it.
            state.seen = Some((key, sel));
        }
        Ok(r)
    }

    /// The document as `.docx` bytes; it then has no unsaved changes. Read-only documents save
    /// too (the host decides whether to write the bytes).
    pub fn embed_save(&mut self) -> Result<Vec<u8>, String> {
        // A save is a save: advance the revision and modified stamp, as the web build's download does.
        self.session.stamp_save();
        let password = self.session.password.as_ref().map(wordcraft_engine::Password::as_str);
        let bytes = wordcraft_engine::io::save_bytes_with("document.docx", &self.session.doc, password, &Default::default())?;
        self.session.dirty = false;
        Ok(bytes)
    }

    /// The selection as UTF-16 offsets into the text (start ≤ end), or `None` when nothing is
    /// selected or the selection isn't in the body.
    pub fn embed_selection(&mut self) -> Option<(usize, usize)> {
        if self.session.sel.is_collapsed() {
            return None;
        }
        let (a, b) = self.session.sel.ordered();
        let ix = self.text_index();
        Some((ix.offset_of(&a)?, ix.offset_of(&b)?))
    }

    /// Select `start..end` of the text (UTF-16 offsets, in either order, clamped) and scroll it
    /// into view. `start == end` puts the caret there.
    pub fn embed_select(&mut self, start: usize, end: usize) -> Result<Value, String> {
        let (start, end) = (start.min(end), start.max(end));
        let ix = self.text_index();
        let a = ix.pos_at(start, false);
        let b = if start == end { a.clone() } else { ix.pos_at(end, true) };
        let pos = wordcraft_engine::cmd::pos_json;
        let r = self.execute("select.range", json!({"anchor": pos(&a), "focus": pos(&b)}))?;
        self.canvas.scroll_to_caret = true;
        Ok(r)
    }

    /// Tint ranges of the text (UTF-16 offsets; at most [`MAX_DECORATIONS`]). They are drawn
    /// over the page and never become part of the document; edits move them along.
    pub fn set_decorations(&mut self, ranges: Vec<Decoration>) {
        let ix = self.text_index();
        let key = self.key();
        let len = ix.len_utf16();
        let Some(state) = self.embed.as_mut() else { return };
        state.decorations = ranges
            .into_iter()
            .take(MAX_DECORATIONS)
            .map(|d| Decoration { start: d.start.min(d.end).min(len), end: d.start.max(d.end).min(len), ..d })
            .collect();
        state.decorated = Some((key, ix.text().to_string()));
        state.painted = None;
    }

    /// The decorations now: their offsets carried over any edits since they were set.
    pub fn decorations(&mut self) -> Vec<Decoration> {
        self.follow_edits();
        self.embed.as_ref().map(|s| s.decorations.clone()).unwrap_or_default()
    }

    /// Carry the decorations' offsets over edits made since they were last worked out.
    fn follow_edits(&mut self) {
        let key = self.key();
        let stale = self.embed.as_ref().is_some_and(|s| s.decorated.as_ref().is_some_and(|(k, _)| *k != key));
        if !stale {
            return;
        }
        let ix = self.text_index();
        let Some(state) = self.embed.as_mut() else { return };
        let Some((k, old)) = state.decorated.take() else { return };
        // Another document: the offsets mean nothing in it.
        if k.0 != key.0 {
            state.decorations.clear();
            return;
        }
        let mut ranges: Vec<(usize, usize)> = state.decorations.iter().map(|d| (d.start, d.end)).collect();
        remap_ranges(&old, ix.text(), &mut ranges);
        for (d, (a, b)) in state.decorations.iter_mut().zip(ranges) {
            d.start = a;
            d.end = b;
        }
        state.decorated = Some((key, ix.text().to_string()));
    }

    /// The decorations as document ranges, for painting.
    pub(crate) fn decoration_ranges(&mut self) -> Arc<Vec<(Pos, Pos, Color32)>> {
        if self.embed.as_ref().is_none_or(|s| s.decorations.is_empty()) {
            return Arc::default();
        }
        self.follow_edits();
        let key = self.key();
        if let Some((k, ranges)) = self.embed.as_ref().and_then(|s| s.painted.as_ref())
            && *k == key
        {
            return ranges.clone();
        }
        let ix = self.text_index();
        let Some(state) = self.embed.as_mut() else { return Arc::default() };
        let ranges: Arc<Vec<_>> = Arc::new(
            state.decorations.iter().filter(|d| d.start < d.end).map(|d| (ix.pos_at(d.start, false), ix.pos_at(d.end, true), d.color)).collect(),
        );
        state.painted = Some((key, ranges.clone()));
        ranges
    }

    /// Save, from the user (shortcut or button): the host saves.
    pub(crate) fn request_host_save(&mut self) {
        if let Some(state) = self.embed.as_mut() {
            state.save_requested = true;
        }
    }

    /// What happened since the host was last told: the document changed, the selection moved,
    /// Save was asked for. Call once a frame, after [`WordApp::ui`].
    pub fn take_embed_events(&mut self) -> Vec<EmbedEvent> {
        let key = self.key();
        let sel = self.session.sel.clone();
        let Some(state) = self.embed.as_mut() else { return Vec::new() };
        let mut out = Vec::new();
        // The first look only records where things stand.
        if let Some((k, s)) = &state.seen {
            if *k != key {
                out.push(EmbedEvent::Change);
            }
            if *s != sel {
                out.push(EmbedEvent::Selection);
            }
        }
        state.seen = Some((key, sel));
        if std::mem::take(&mut state.save_requested) {
            out.push(EmbedEvent::SaveShortcut);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;
    use wordcraft_engine::Session;
    use wordcraft_engine::doc::{Document, StoryRef};

    fn app(text: &str) -> WordApp {
        let mut a = WordApp::new(Session::new(Document::from_text(text)), Services::default());
        a.embedded();
        a
    }

    #[test]
    fn colours() {
        assert_eq!(parse_color("#ff0000"), Some(Color32::from_rgba_unmultiplied(255, 0, 0, DEFAULT_ALPHA)));
        assert_eq!(parse_color("#f008"), Some(Color32::from_rgba_unmultiplied(255, 0, 0, 0x88)));
        assert_eq!(parse_color(" #12345678 "), Some(Color32::from_rgba_unmultiplied(0x12, 0x34, 0x56, 0x78)));
        for junk in ["", "#", "red", "#12", "#gggggg", "#ффф", "#1234567890"] {
            assert_eq!(parse_color(junk), None, "{junk}");
        }
    }

    #[test]
    fn select_and_read_back_by_offsets() {
        let mut a = app("Hello world\nSecond");
        assert_eq!(a.text_index().text(), "Hello world\nSecond");
        assert_eq!(a.embed_selection(), None);
        a.embed_select(6, 11).unwrap();
        assert_eq!(a.session.selected_text(), "world");
        assert_eq!(a.embed_selection(), Some((6, 11)));
        // Backwards, and junk past the end.
        a.embed_select(10_000, 12).unwrap();
        assert_eq!(a.session.selected_text(), "Second");
        a.embed_select(3, 3).unwrap();
        assert_eq!(a.embed_selection(), None);
    }

    #[test]
    fn events_follow_edits_selection_and_save() {
        let mut a = app("Hello");
        // The first look only records where things stand.
        assert_eq!(a.take_embed_events(), vec![]);
        a.embed_select(0, 5).unwrap();
        assert_eq!(a.take_embed_events(), vec![EmbedEvent::Selection]);
        a.run("text.insert", json!({"text": "Hi"})).unwrap();
        assert_eq!(a.take_embed_events(), vec![EmbedEvent::Change, EmbedEvent::Selection]);
        assert_eq!(a.take_embed_events(), vec![]);
        // Save from the user: the host saves; nothing is written here.
        assert!(a.session.dirty);
        a.run("file.save", json!({})).unwrap();
        assert_eq!(a.take_embed_events(), vec![EmbedEvent::SaveShortcut]);
        assert!(a.session.dirty);
        let bytes = a.embed_save().unwrap();
        assert!(!a.session.dirty);
        // Opening what was saved: no events for it, and the text is back.
        a.embed_open("x.docx", &bytes).unwrap();
        assert_eq!(a.take_embed_events(), vec![]);
        assert_eq!(a.session.doc.plain_text(StoryRef::Body), "Hi");
        // New, Open and Close are the host's.
        assert!(a.run("file.new", json!({})).is_err());
        assert!(a.run("file.close", json!({})).is_err());
    }

    #[test]
    fn decorations_survive_an_edit_before_them() {
        let mut a = app("Intro\nThe carded passage here.");
        let at = a.text_index().text().find("carded").unwrap();
        a.set_decorations(vec![Decoration { start: at, end: at + "carded passage".len(), color: Color32::RED }]);
        a.run("text.insert", json!({"text": "Typed first. "})).unwrap();
        let [d] = a.decorations()[..] else { panic!("one decoration") };
        let text = a.text_index().text().to_string();
        assert_eq!(text.get(d.start..d.end), Some("carded passage"));
        // Painted where the passage is, never part of the document.
        let ranges = a.decoration_ranges();
        let (p, q, _) = ranges.first().unwrap();
        assert_eq!(a.session.doc.copy_range(p, q).plain_text(), "carded passage");
        assert!(a.session.doc.plain_text(StoryRef::Body).contains("Typed first. Intro"));
        // Undo takes the decoration back with the text.
        a.run("edit.undo", json!({})).unwrap();
        let [d] = a.decorations()[..] else { panic!("one decoration") };
        assert_eq!((d.start, d.end), (at, at + "carded passage".len()));
        // Hostile ranges are clamped; a new document drops them.
        a.set_decorations(vec![Decoration { start: usize::MAX, end: 3, color: Color32::RED }]);
        let [d] = a.decorations()[..] else { panic!("one decoration") };
        assert!(d.start <= d.end && d.end <= a.text_index().len_utf16());
        a.embed_open("y.txt", b"Other").unwrap();
        assert!(a.decorations().is_empty());
    }

    #[test]
    fn read_only_refuses_typing_and_allows_selection() {
        let mut a = app("Hello world");
        a.session.read_only = true;
        assert!(a.run("text.insert", json!({"text": "x"})).is_err());
        assert!(!crate::widgets::enabled(&a, "format.bold"));
        assert!(crate::widgets::enabled(&a, "edit.copy"));
        a.embed_select(0, 5).unwrap();
        assert_eq!(a.session.selected_text(), "Hello");
        assert_eq!(a.session.doc.plain_text(StoryRef::Body), "Hello world");
        assert_eq!(a.take_embed_events(), vec![]);
    }
}

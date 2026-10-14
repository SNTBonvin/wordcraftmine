//! Mac text keys: ⌘← / ⌘→ line start and end, ⌘↑ / ⌘↓ document start and end, ⌘⌫ delete to line
//! start, and the Cocoa/Emacs ⌃ keys (⌃A ⌃E ⌃K ⌃D ⌃H ⌃F ⌃B ⌃N ⌃P). The engine's `Mod` is ⌘ on
//! macOS, so its built-in Mod+Left is word movement (Windows' Ctrl+Left); these map the keys the
//! way every Mac text view does. Only `keys::canvas_events` calls this, and only on macOS.

use egui::{Key, Modifiers};
use serde_json::{Value, json};

/// The commands a Mac text key runs, in order, with their params. `None` when the key isn't one
/// of these (so the normal keymap handles it). Shift extends the selection on the movement keys.
/// Assumes macOS modifiers: `command` is ⌘ and `ctrl` is ⌃.
pub fn mac_command(key: Key, m: Modifiers) -> Option<Vec<(&'static str, Value)>> {
    if m.alt {
        return None;
    }
    match (m.command, m.ctrl) {
        (true, false) => command_key(key, m.shift),
        (false, true) => control_key(key, m.shift),
        _ => None,
    }
}

/// ⌘ with arrows, and ⌘⌫.
fn command_key(key: Key, shift: bool) -> Option<Vec<(&'static str, Value)>> {
    let id = match key {
        Key::ArrowLeft => "caret.home",
        Key::ArrowRight => "caret.end",
        Key::ArrowUp => "caret.docStart",
        Key::ArrowDown => "caret.docEnd",
        // ⌘⌫ selects back to the line start, then erases. With the caret already at the line start
        // there is nothing to select, so it erases the paragraph break before it, as Backspace does.
        Key::Backspace if !shift => return Some(vec![("caret.home", json!({"extend": true})), ("text.backspace", json!({}))]),
        _ => return None,
    };
    Some(vec![(id, json!({"extend": shift}))])
}

/// With text selected, the keys that select and then erase (⌘⌫, ⌃K) erase just the selection, as
/// in every Mac text view, instead of first moving the selection's focus to the line's end.
pub fn on_selection(steps: Vec<(&'static str, Value)>) -> Vec<(&'static str, Value)> {
    match steps.as_slice() {
        [(mv, _), (erase @ ("text.backspace" | "text.delete"), p)] if mv.starts_with("caret.") => vec![(*erase, p.clone())],
        _ => steps,
    }
}

/// The ⌃ Emacs keys. Shift extends only the movement keys; ⌃⇧K / ⌃⇧D / ⌃⇧H are left to the keymap.
/// ⌃K selects to the line end, then erases it: at the line end that erases the paragraph break, as
/// Cocoa does. (With a selection, [`on_selection`] erases just the selection.)
fn control_key(key: Key, shift: bool) -> Option<Vec<(&'static str, Value)>> {
    let movement = match key {
        Key::A => "caret.home",
        Key::E => "caret.end",
        Key::F => "caret.right",
        Key::B => "caret.left",
        Key::N => "caret.down",
        Key::P => "caret.up",
        Key::K if !shift => return Some(vec![("caret.end", json!({"extend": true})), ("text.delete", json!({}))]),
        Key::D if !shift => return Some(vec![("text.delete", json!({}))]),
        Key::H if !shift => return Some(vec![("text.backspace", json!({}))]),
        _ => return None,
    };
    Some(vec![(movement, json!({"extend": shift}))])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Modifiers as egui reports them on macOS: `command` and `mac_cmd` are both ⌘.
    fn m(command: bool, ctrl: bool, alt: bool, shift: bool) -> Modifiers {
        Modifiers { command, mac_cmd: command, ctrl, alt, shift }
    }

    fn one(id: &'static str, extend: bool) -> Option<Vec<(&'static str, Value)>> {
        Some(vec![(id, json!({"extend": extend}))])
    }

    #[test]
    fn with_a_selection_the_erasing_keys_erase_just_it() {
        let cmd = m(true, false, false, false);
        let ctrl = m(false, true, false, false);
        let erase = |key, mods| mac_command(key, mods).map(on_selection);
        assert_eq!(erase(Key::Backspace, cmd), Some(vec![("text.backspace", json!({}))]), "⌘⌫");
        assert_eq!(erase(Key::K, ctrl), Some(vec![("text.delete", json!({}))]), "⌃K");
        // Movement keeps its meaning.
        assert_eq!(erase(Key::ArrowLeft, cmd), one("caret.home", false), "⌘←");
        assert_eq!(erase(Key::D, ctrl), Some(vec![("text.delete", json!({}))]), "⌃D");
    }

    #[test]
    fn command_arrows_go_to_line_and_document_ends() {
        let cmd = m(true, false, false, false);
        assert_eq!(mac_command(Key::ArrowLeft, cmd), one("caret.home", false), "⌘←");
        assert_eq!(mac_command(Key::ArrowRight, cmd), one("caret.end", false), "⌘→");
        assert_eq!(mac_command(Key::ArrowUp, cmd), one("caret.docStart", false), "⌘↑");
        assert_eq!(mac_command(Key::ArrowDown, cmd), one("caret.docEnd", false), "⌘↓");
    }

    #[test]
    fn shift_extends_the_selection_for_command_arrows() {
        let cmd_shift = m(true, false, false, true);
        assert_eq!(mac_command(Key::ArrowLeft, cmd_shift), one("caret.home", true), "⌘⇧←");
        assert_eq!(mac_command(Key::ArrowRight, cmd_shift), one("caret.end", true), "⌘⇧→");
        assert_eq!(mac_command(Key::ArrowUp, cmd_shift), one("caret.docStart", true), "⌘⇧↑");
        assert_eq!(mac_command(Key::ArrowDown, cmd_shift), one("caret.docEnd", true), "⌘⇧↓");
    }

    #[test]
    fn command_backspace_deletes_to_line_start() {
        let steps = mac_command(Key::Backspace, m(true, false, false, false)).expect("⌘⌫ is mapped");
        assert_eq!(steps, vec![("caret.home", json!({"extend": true})), ("text.backspace", json!({}))]);
        assert_eq!(mac_command(Key::Backspace, m(true, false, false, true)), None, "⌘⇧⌫ is not a Mac key");
    }

    #[test]
    fn control_emacs_movement_keys() {
        let ctrl = m(false, true, false, false);
        assert_eq!(mac_command(Key::A, ctrl), one("caret.home", false), "⌃A");
        assert_eq!(mac_command(Key::E, ctrl), one("caret.end", false), "⌃E");
        assert_eq!(mac_command(Key::F, ctrl), one("caret.right", false), "⌃F");
        assert_eq!(mac_command(Key::B, ctrl), one("caret.left", false), "⌃B");
        assert_eq!(mac_command(Key::N, ctrl), one("caret.down", false), "⌃N");
        assert_eq!(mac_command(Key::P, ctrl), one("caret.up", false), "⌃P");
        assert_eq!(mac_command(Key::E, m(false, true, false, true)), one("caret.end", true), "⌃⇧E extends");
    }

    #[test]
    fn control_emacs_editing_keys() {
        let ctrl = m(false, true, false, false);
        assert_eq!(
            mac_command(Key::K, ctrl),
            Some(vec![("caret.end", json!({"extend": true})), ("text.delete", json!({}))]),
            "⌃K deletes to line end"
        );
        assert_eq!(mac_command(Key::D, ctrl), Some(vec![("text.delete", json!({}))]), "⌃D forward delete");
        assert_eq!(mac_command(Key::H, ctrl), Some(vec![("text.backspace", json!({}))]), "⌃H backspace");
        assert_eq!(mac_command(Key::K, m(false, true, false, true)), None, "⌃⇧K is left alone");
        assert_eq!(mac_command(Key::H, m(false, true, false, true)), None, "⌃⇧H is left alone");
    }

    #[test]
    fn other_combinations_are_not_mac_text_keys() {
        assert_eq!(mac_command(Key::ArrowLeft, m(false, false, false, false)), None, "a plain ←");
        assert_eq!(mac_command(Key::ArrowLeft, m(false, false, true, false)), None, "⌥← is word left in the keymap");
        assert_eq!(mac_command(Key::ArrowUp, m(false, false, true, false)), None, "⌥↑ is paragraph up in the keymap");
        assert_eq!(mac_command(Key::ArrowLeft, m(true, false, true, false)), None, "⌘⌥←");
        assert_eq!(mac_command(Key::A, m(true, false, false, false)), None, "⌘A is select all");
        assert_eq!(mac_command(Key::Z, m(false, true, false, false)), None, "⌃Z is not mapped");
        assert_eq!(mac_command(Key::ArrowLeft, m(false, true, false, false)), None, "⌃← is not mapped");
        assert_eq!(mac_command(Key::A, m(true, true, false, false)), None, "⌃⌘A");
        assert_eq!(mac_command(Key::Backspace, m(false, false, true, false)), None, "⌥⌫ stays delete word back");
        assert_eq!(mac_command(Key::Backspace, m(false, false, false, false)), None, "a plain Backspace");
    }
}

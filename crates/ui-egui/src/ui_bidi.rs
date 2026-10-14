//! Display order for egui's script fallback runs. Unicode resolves the order of runs;
//! egui/Harfrust still shapes the letters inside each script run. Stored text and document
//! rendering keep logical order. This adapter must be removed when egui gains full BiDi layout.

use unicode_bidi::{BidiInfo, Level};

/// Arrange UI text for painting, preserving the shaper's logical RTL script stretches.
pub(crate) fn display(text: &str, base: Option<Level>) -> String {
    if !wordcraft_doc::bidi::has_rtl(text) {
        return text.to_owned();
    }
    let mut result = String::with_capacity(text.len());
    // A newline belongs between paragraphs, never at their mirrored leading edge.
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            result.push('\n');
        }
        let info = BidiInfo::new(line, base);
        let mut visual = String::with_capacity(line.len());
        for paragraph in &info.paragraphs {
            let (levels, runs) = info.visual_runs(paragraph, paragraph.range.clone());
            for run in runs {
                let rtl = levels.get(run.start).is_some_and(Level::is_rtl);
                let Some(piece) = line.get(run) else { continue };
                if rtl {
                    visual.extend(piece.chars().rev().filter(|c| !control(*c)).map(|c| unicode_bidi_mirroring::get_mirrored(c).unwrap_or(c)));
                } else {
                    visual.extend(piece.chars().filter(|c| !control(*c)));
                }
            }
        }
        let mut script = String::new();
        for c in visual.chars() {
            if wordcraft_doc::bidi::is_rtl_script(c) || matches!(c, '\u{200c}' | '\u{200d}') {
                script.push(c);
            } else {
                if wordcraft_doc::bidi::first_strong_is_rtl(&script) {
                    result.extend(script.chars().rev());
                } else {
                    result.push_str(&script);
                }
                script.clear();
                result.push(c);
            }
        }
        if wordcraft_doc::bidi::first_strong_is_rtl(&script) {
            result.extend(script.chars().rev());
        } else {
            result.push_str(&script);
        }
    }
    result
}

fn control(c: char) -> bool {
    matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// Keep substitutions atomic while arranging a translated template. A substitution's
/// contents are formatted separately, so braces in a file name never become template syntax.
pub(crate) fn template(text: &str) -> String {
    const FIRST: u32 = 0xe000;
    const LIMIT: usize = 64;
    if text.chars().any(|c| (FIRST..FIRST + LIMIT as u32).contains(&u32::from(c))) {
        return text.to_owned();
    }
    let mut masked = String::with_capacity(text.len());
    let mut tokens = Vec::new();
    let mut rest = text;
    while tokens.len() < LIMIT {
        let Some(open) = rest.find('{') else { break };
        let Some(close) = rest.get(open..).and_then(|s| s.find('}')).map(|n| open + n + 1) else { break };
        let (Some(before), Some(token), Some(after)) = (rest.get(..open), rest.get(open..close), rest.get(close..)) else { break };
        let Some(marker) = char::from_u32(FIRST + tokens.len() as u32) else { break };
        masked.push_str(before);
        masked.push(marker);
        tokens.push(token);
        rest = after;
    }
    masked.push_str(rest);
    let arranged = display(&masked, Some(Level::rtl()));
    let mut out = String::with_capacity(text.len());
    for c in arranged.chars() {
        if let Some(token) = u32::from(c).checked_sub(FIRST).and_then(|i| tokens.get(i as usize)) {
            out.push_str(token);
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_keep_letters_marks_and_joiners_in_logical_order() {
        for (logical, expected) in [("واحد اثنين", "اثنين واحد"), ("سلام می‌روم", "می‌روم سلام"), ("سَلَام عَالَم", "عَالَم سَلَام")]
        {
            assert_eq!(display(logical, Some(Level::rtl())), expected);
        }
    }

    #[test]
    fn punctuation_numbers_and_latin_follow_unicode_order() {
        assert_eq!(display("واحد (اثنين) 2026", Some(Level::rtl())), "2026 (اثنين) واحد");
        assert_eq!(display("WordCraft واحد اثنين", None), "WordCraft اثنين واحد");
        assert_eq!(display("واحد\nاثنين ثلاثة", Some(Level::rtl())), "واحد\nثلاثة اثنين");
        assert_eq!(display("\u{0661}\u{0662}\u{0663}", Some(Level::rtl())), "\u{0661}\u{0662}\u{0663}");
        for text in ["", "WordCraft", "100%", "a {name} (b)", "日本語"] {
            assert_eq!(display(text, Some(Level::rtl())), text);
        }
    }

    #[test]
    fn template_holes_are_preserved() {
        assert_eq!(template("واحد {name} اثنين"), "اثنين {name} واحد");
        assert_eq!(template("{page} من {pages}"), "{pages} من {page}");
        assert_eq!(template("واحد {name}\nاثنين {n}"), "{name} واحد\n{n} اثنين");
    }
}

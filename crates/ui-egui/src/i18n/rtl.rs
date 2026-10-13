//! Right-to-left interface text (Hebrew). egui lays a label out left to right in runs of one font
//! face: each Hebrew word (the borrowed Hebrew face) is shaped, and so reads right to left, but
//! the words, spaces and punctuation (the interface face) follow each other left to right, so
//! `מסך הבית` drew as `הבית מסך`. A right-to-left catalog is therefore stored in display order,
//! once, when it loads: the Unicode bidirectional algorithm (UAX #9) with a right-to-left
//! paragraph orders the text, then each Hebrew word is put back in reading order, since the shaper
//! reverses it again. Trailing punctuation (`שם משתמש:`, `…`) ends up on the left, where it
//! belongs. `{name}` placeholders stay intact and are placed as one left-to-right unit.

use unicode_bidi::{BidiInfo, Level};

/// A character the borrowed face shapes in one run with its neighbours: Hebrew letters, points
/// and punctuation (geresh, gershayim, maqaf) and the Hebrew presentation forms.
fn hebrew(c: char) -> bool {
    matches!(c, '\u{0590}'..='\u{05FF}' | '\u{FB1D}'..='\u{FB4F}')
}

/// Paired punctuation as it is drawn inside right-to-left text (Unicode's mirrored glyphs).
fn mirrored(c: char) -> char {
    match c {
        '(' => ')',
        ')' => '(',
        '[' => ']',
        ']' => '[',
        '<' => '>',
        '>' => '<',
        '«' => '»',
        '»' => '«',
        '‹' => '›',
        '›' => '‹',
        other => other,
    }
}

/// Private-use stand-ins for `{name}` placeholders while the text is reordered: they are strong
/// left-to-right characters, so a placeholder keeps its braces and moves as one unit.
const STAND_IN: u32 = 0xF0000;

/// `text` (in reading order) in the order egui draws right-to-left interface text; see the module
/// documentation. Lines (`\n`) are ordered one by one.
pub fn display_order(text: &str) -> String {
    let mut names = Vec::new();
    let mut protected = String::with_capacity(text.len());
    let mut rest = text;
    while let Some((before, after_open)) = rest.split_once('{') {
        protected.push_str(before);
        match after_open.split_once('}') {
            Some((name, after_close)) if !name.contains('{') => {
                let stand_in = u32::try_from(names.len()).ok().and_then(|i| char::from_u32(STAND_IN.saturating_add(i)));
                match stand_in {
                    Some(c) => {
                        protected.push(c);
                        names.push(format!("{{{name}}}"));
                    }
                    None => protected.push_str(&format!("{{{name}}}")),
                }
                rest = after_close;
            }
            _ => {
                protected.push('{');
                rest = after_open;
            }
        }
    }
    protected.push_str(rest);

    let ordered: Vec<String> = protected.split('\n').map(line_display_order).collect();
    let mut out = String::with_capacity(text.len());
    for c in ordered.join("\n").chars() {
        let index = (c as u32).checked_sub(STAND_IN).and_then(|i| usize::try_from(i).ok());
        match index.and_then(|i| names.get(i)) {
            Some(name) => out.push_str(name),
            None => out.push(c),
        }
    }
    out
}

/// One line in display order: UAX #9 visual runs with a right-to-left paragraph level, mirrored
/// pairs inside right-to-left runs, then every Hebrew word back in reading order.
fn line_display_order(line: &str) -> String {
    if !line.chars().any(hebrew) {
        return line.to_string();
    }
    let info = BidiInfo::new(line, Some(Level::rtl()));
    let mut visual: Vec<char> = Vec::with_capacity(line.len());
    for para in &info.paragraphs {
        let (levels, runs) = info.visual_runs(para, para.range.clone());
        for run in runs {
            let Some(text) = line.get(run.clone()) else { continue };
            if levels.get(run.start).is_some_and(|l| l.is_rtl()) {
                visual.extend(text.chars().rev().map(mirrored));
            } else {
                visual.extend(text.chars());
            }
        }
    }
    // Each Hebrew word came out reversed with its run; the shaper reverses it once more.
    let mut out = String::with_capacity(line.len());
    let mut word: Vec<char> = Vec::new();
    for c in visual {
        if hebrew(c) {
            word.push(c);
            continue;
        }
        out.extend(word.drain(..).rev());
        out.push(c);
    }
    out.extend(word.drain(..).rev());
    out
}

#[cfg(test)]
mod tests {
    use super::display_order;

    #[test]
    fn hebrew_words_follow_each_other_right_to_left() {
        // Drawn left to right, `הבית מסך` reads `מסך הבית` from the right.
        assert_eq!(display_order("מסך הבית"), "הבית מסך");
        assert_eq!(display_order("קובץ"), "קובץ", "a single word is left to the shaper");
        assert_eq!(display_order("Home Screen"), "Home Screen", "text without Hebrew is unchanged");
    }

    #[test]
    fn trailing_punctuation_and_numbers_sit_where_hebrew_puts_them() {
        assert_eq!(display_order("שם משתמש:"), ":משתמש שם");
        assert_eq!(display_order("שמירה בשם…"), "…בשם שמירה");
        assert_eq!(display_order("כותרת 1"), "1 כותרת");
        assert_eq!(display_order("עמוד (טקסט לבן)"), "(לבן טקסט) עמוד", "parentheses are mirrored");
    }

    #[test]
    fn placeholders_stay_whole_and_latin_names_read_left_to_right() {
        assert_eq!(display_order("עמוד {page} מתוך {pages}"), "{pages} מתוך {page} עמוד");
        assert_eq!(display_order("מילים: {words}"), "{words} :מילים");
        assert_eq!(display_order("הפעלת WordCraft"), "WordCraft הפעלת");
        assert_eq!(display_order("שמירה בפורמט PDF או DOCX"), "DOCX או PDF בפורמט שמירה");
        assert_eq!(display_order("{a{b}"), "{a{b}", "a stray brace is kept as written");
    }

    #[test]
    fn lines_are_ordered_one_by_one() {
        assert_eq!(display_order("שורה אחת\nשורה שתיים"), "אחת שורה\nשתיים שורה");
    }
}

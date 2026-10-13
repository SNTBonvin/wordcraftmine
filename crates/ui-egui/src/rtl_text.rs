//! Right-to-left text for the interface. egui shapes each run of one font with HarfRust, so an
//! Arabic or Hebrew word comes out joined and right to left, but it lays the runs out left to
//! right in logical order: the words of a right-to-left name, and the punctuation between them,
//! end up reversed. Interface strings that come from the user's files and documents (file names
//! and paths, headings in the Navigation pane, style names, comment authors…) go through
//! [`visual`] first: the Unicode Bidirectional Algorithm's visual order, with each right-to-left
//! word kept in logical order for egui to shape. The first such string also makes the interface
//! load Arabic and Hebrew fallback faces ([`saw_rtl`]): the interface fonts have neither.
//!
//! Single lines only: egui wraps the visual string, so a right-to-left paragraph that wraps would
//! show its last words first. Editable fields (`TextEdit`) can't use it either, since the caret
//! works on the logical text.
//!
//! Documents don't use this: `wordcraft-layout` runs the full algorithm on their text.

use std::borrow::Cow;
use std::cell::Cell;

use unicode_bidi::BidiInfo;
use wordcraft_doc::bidi::{has_rtl, is_rtl_script};

/// `text` as egui should draw it: unchanged when it has no right-to-left character, otherwise in
/// visual order. The base direction comes from the first strong character, as Finder and
/// Explorer show file names.
pub fn visual(text: &str) -> Cow<'_, str> {
    if !has_rtl(text) {
        return Cow::Borrowed(text);
    }
    SAW_RTL.set(true);
    if text.contains('\n') {
        let lines: Vec<Cow<'_, str>> = text.split('\n').map(visual).collect();
        return Cow::Owned(lines.join("\n"));
    }
    let info = BidiInfo::new(text, None);
    let mut out = String::with_capacity(text.len());
    for para in &info.paragraphs {
        let (levels, runs) = info.visual_runs(para, para.range.clone());
        for run in runs {
            let rtl = levels.get(run.start).is_some_and(|l| l.is_rtl());
            let Some(s) = text.get(run) else { continue };
            if rtl {
                push_rtl_run(&mut out, s);
            } else {
                out.push_str(s);
            }
        }
    }
    Cow::Owned(out)
}

/// [`visual`] for a single-line label at most `max_width` wide. A long right-to-left string is
/// shortened at its logical end, so the ellipsis lands on the left and the first words stay;
/// egui's own truncation would cut the right-hand end, which is where right-to-left text starts.
pub fn fit(ctx: &egui::Context, text: &str, font: &egui::FontId, max_width: f32) -> String {
    let width = |s: &str| ctx.fonts_mut(|f| f.layout_no_wrap(s.to_string(), font.clone(), egui::Color32::BLACK).size().x);
    let shown = visual(text);
    if !wordcraft_doc::bidi::first_strong_is_rtl(text) || width(&shown) <= max_width {
        return shown.into_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let candidate = |n: usize| visual(&format!("{}…", chars.iter().take(n).collect::<String>().trim_end())).into_owned();
    let (mut lo, mut hi) = (0, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if width(&candidate(mid)) <= max_width {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    candidate(lo)
}

/// [`visual`] for text wrapped at `max_width`: the lines are broken in logical order first, so a
/// right-to-left paragraph's first words stay on its first line, then each line is put in visual
/// order. egui wrapping the visual string would break it from the wrong end.
pub fn wrap(ctx: &egui::Context, text: &str, font: &egui::FontId, max_width: f32) -> String {
    if !has_rtl(text) {
        return text.to_string();
    }
    let width = |s: &str| ctx.fonts_mut(|f| f.layout_no_wrap(visual(s).into_owned(), font.clone(), egui::Color32::BLACK).size().x);
    let mut lines: Vec<String> = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split(' ') {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if !line.is_empty() && width(&candidate) > max_width {
                lines.push(std::mem::replace(&mut line, word.to_string()));
            } else {
                line = candidate;
            }
        }
        lines.push(line);
    }
    lines.iter().map(|l| visual(l)).collect::<Vec<_>>().join("\n")
}

thread_local! {
    /// Set once right-to-left text has been laid out on this thread (the interface's).
    static SAW_RTL: Cell<bool> = const { Cell::new(false) };
}

/// Has [`visual`] been given right-to-left text? Then the interface needs fallback faces for it.
pub fn saw_rtl() -> bool {
    SAW_RTL.get()
}

/// A file path as egui should draw it: each folder and file name through [`visual`] on its own,
/// with the separators left where they are. Run as one string, the algorithm would join
/// neighbouring Arabic folders into one right-to-left run and show them in reverse order.
pub fn visual_path(path: &str) -> Cow<'_, str> {
    if !has_rtl(path) {
        return Cow::Borrowed(path);
    }
    let mut out = String::with_capacity(path.len());
    for piece in path.split_inclusive(['/', '\\']) {
        let name = piece.trim_end_matches(['/', '\\']);
        out.push_str(&visual(name));
        out.push_str(piece.get(name.len()..).unwrap_or_default());
    }
    Cow::Owned(out)
}

/// A character egui shapes together with the right-to-left letters around it: the letters, their
/// combining marks, and the joiners (the Persian half-space is a zero-width non-joiner).
fn in_rtl_word(c: char) -> bool {
    is_rtl_script(c) || matches!(c, '\u{200C}' | '\u{200D}')
}

/// Append a right-to-left run (logical order) in visual order: its words in reverse order, each
/// word left as it is for egui to shape, and the characters between them reversed and mirrored.
fn push_rtl_run(out: &mut String, run: &str) {
    let mut tokens: Vec<&str> = Vec::new();
    let mut start = 0;
    let mut in_word = false;
    for (i, c) in run.char_indices() {
        let word = in_rtl_word(c);
        if i > start && !(word && in_word) {
            tokens.extend(run.get(start..i));
            start = i;
        }
        in_word = word;
    }
    tokens.extend(run.get(start..));
    for token in tokens.iter().rev() {
        if token.chars().any(in_rtl_word) {
            out.push_str(token);
        } else {
            out.extend(token.chars().map(mirror));
        }
    }
}

/// A bracket's mirror image, as a right-to-left run displays it (UAX #9 rule L4).
fn mirror(c: char) -> char {
    match c {
        '(' => ')',
        ')' => '(',
        '[' => ']',
        ']' => '[',
        '{' => '}',
        '}' => '{',
        '<' => '>',
        '>' => '<',
        '«' => '»',
        '»' => '«',
        _ => c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin_is_untouched_and_borrowed() {
        assert!(matches!(visual("Report 2026.docx"), Cow::Borrowed("Report 2026.docx")));
    }

    #[test]
    fn rtl_words_keep_their_letters_in_logical_order() {
        // egui shapes each word right to left itself: only the word order changes.
        assert_eq!(visual("خطة التسويق"), "التسويق خطة");
        assert_eq!(visual("خطة التسويق لتطبيق حواس"), "حواس لتطبيق التسويق خطة");
    }

    #[test]
    fn rtl_name_puts_the_extension_on_the_left() {
        assert_eq!(visual("خطة التسويق.docx"), "docx.التسويق خطة");
    }

    #[test]
    fn harakat_and_half_space_stay_inside_the_word() {
        assert_eq!(visual("بَيت كتاب"), "كتاب بَيت");
        assert_eq!(visual("می\u{200C}خواهم کتاب"), "کتاب می\u{200C}خواهم");
    }

    #[test]
    fn numbers_and_brackets_in_rtl_names() {
        assert_eq!(visual("نسخة 2"), "2 نسخة");
        assert_eq!(visual("عقد (نسخة 2)"), "(2 نسخة) عقد");
    }

    #[test]
    fn path_keeps_folder_order_with_arabic_folders() {
        assert_eq!(visual_path("/Users/me/ملفات/العقود/خطة التسويق.docx"), "/Users/me/ملفات/العقود/docx.التسويق خطة");
        assert!(matches!(visual_path("/Users/me/a.docx"), Cow::Borrowed(_)));
        assert_eq!(visual_path("C:\\ملفات العقود\\"), "C:\\العقود ملفات\\");
    }

    #[test]
    fn latin_name_with_an_arabic_word_stays_left_to_right() {
        assert_eq!(visual("Report خطة التسويق v2"), "Report التسويق خطة v2");
    }

    #[test]
    fn each_line_is_ordered_on_its_own() {
        assert_eq!(visual("خطة التسويق\nمدة العقد"), "التسويق خطة\nالعقد مدة");
        assert!(saw_rtl(), "right-to-left text asks for the fallback faces");
    }

    #[test]
    fn english_sentence_around_an_arabic_name() {
        assert_eq!(visual("Do you want to save changes to عقد المسار?"), "Do you want to save changes to المسار عقد?");
    }

    #[test]
    fn fit_shortens_a_right_to_left_label_from_its_logical_end() {
        let ctx = egui::Context::default();
        let font = egui::FontId::proportional(12.0);
        let long = "الشروط والأحكام العامة لعقد الشراكة بين الطرفين";
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let shown = fit(ui.ctx(), long, &font, 120.0);
            // Visual order: the first word is on the right, the ellipsis on the left.
            assert!(shown.ends_with("الشروط"), "{shown}");
            assert!(shown.starts_with('…'), "{shown}");
            assert_eq!(fit(ui.ctx(), "Short", &font, 120.0), "Short");
            assert_eq!(fit(ui.ctx(), "مدة العقد", &font, 500.0), "العقد مدة");
        });
        out.textures_delta.clear();
    }

    #[test]
    fn hebrew_words_are_reordered_too() {
        assert_eq!(visual("שלום עולם"), "עולם שלום");
    }
}

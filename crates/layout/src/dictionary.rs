//! Line breaks for Thai, Lao and Khmer, which have no spaces between words: UAX #14 leaves
//! them (class SA) to a dictionary, so they wrap between ICU4X dictionary words (#1548).

use icu_segmenter::LineSegmenter;
use icu_segmenter::options::LineBreakOptions;

fn is_dictionary_script(c: char) -> bool {
    matches!(u32::from(c), 0x0E00..=0x0E7F | 0x0E80..=0x0EFF | 0x1780..=0x17FF | 0x19E0..=0x19FF)
}

/// Byte offsets inside `text` where a line may start between dictionary words, each next to a
/// Thai, Lao or Khmer letter.
pub fn breaks(text: &str) -> Vec<usize> {
    if !text.chars().any(is_dictionary_script) {
        return Vec::new();
    }
    let seg = LineSegmenter::new_dictionary(LineBreakOptions::default());
    let near = |i: usize| {
        text.get(..i).and_then(|t| t.chars().next_back()).is_some_and(is_dictionary_script)
            || text.get(i..).and_then(|t| t.chars().next()).is_some_and(is_dictionary_script)
    };
    seg.segment_str(text).filter(|&i| i > 0 && i < text.len() && near(i)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<&str> {
        let mut out = Vec::new();
        let mut last = 0;
        for i in breaks(text).into_iter().chain([text.len()]) {
            out.push(&text[last..i]);
            last = i;
        }
        out
    }

    #[test]
    fn thai_breaks_between_words() {
        let w = words("จัดการทรัพยากรบุคคลขององค์กร");
        assert!(w.contains(&"ทรัพยากร") && w.contains(&"บุคคล"), "{w:?}");
        assert!(!w.iter().any(|s| *s == "กร" && w.contains(&"องค์")), "องค์กร is one word: {w:?}");
        let w = words("คอมพิวเตอร์แม่ข่ายพร้อมอุปกรณ์");
        assert!(w.contains(&"พร้อม") && w.contains(&"อุปกรณ์"), "{w:?}");
        for s in &w {
            let first = s.chars().next().unwrap();
            assert!(!matches!(u32::from(first), 0x0E31 | 0x0E34..=0x0E3A | 0x0E47..=0x0E4E), "{w:?}");
        }
    }

    #[test]
    fn lao_and_khmer_break_too() {
        assert!(words("ສະບາຍດີປະເທດລາວ").len() > 1);
        assert!(words("ប្រទេសកម្ពុជាមានខេត្ត").len() > 1);
    }

    #[test]
    fn other_text_gets_no_extra_breaks() {
        assert!(breaks("Hello wonderful world, 50%.").is_empty());
        assert!(breaks("日本語のテキスト").is_empty());
        assert!(breaks("").is_empty());
        let t = "Wordcraftภาษาไทยwords";
        for i in breaks(t) {
            assert!(i >= "Wordcraft".len() && i <= t.len() - "words".len(), "{i} in {t}");
        }
    }
}

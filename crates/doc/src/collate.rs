//! Language-aware sorting of text: a small, pure-Rust collation.
//!
//! Every letter gets a *primary* weight (its place in the alphabet) and a *secondary* one (its
//! diacritic). Strings compare by primary weights first, then by secondary ones, so `é` sorts
//! with `e` (`resume` < `résumé` < `rex`) unless the language makes it a letter of its own:
//! Polish puts `ą` right after `a` and `ż` after `ź` after `z`, Swedish puts `å ä ö` after `z`,
//! Czech puts `ch` after `h`, Spanish `ñ` after `n`, Hungarian `cs`, `gy`, `sz`… after their
//! first letter. The per-language tables follow the published alphabets of those languages.
//! Text without diacritics sorts exactly as a case-insensitive code-point compare.

use std::cmp::Ordering;

/// The diacritic of a precomposed lowercase Latin letter: `(base letters, mark)`. Marks: 1 acute,
/// 2 grave, 3 circumflex, 4 diaeresis, 5 tilde, 6 ring, 7 caron, 8 cedilla, 9 ogonek, 10 dot,
/// 11 stroke, 12 breve, 13 macron, 14 double acute, 15 comma below, 16 ligature, 17 dotless,
/// 18 eth.
const DECOMP: &[(char, &str, u16)] = &[
    ('à', "a", 2),
    ('á', "a", 1),
    ('â', "a", 3),
    ('ã', "a", 5),
    ('ä', "a", 4),
    ('å', "a", 6),
    ('ā', "a", 13),
    ('ă', "a", 12),
    ('ą', "a", 9),
    ('æ', "ae", 16),
    ('ç', "c", 8),
    ('ć', "c", 1),
    ('ĉ', "c", 3),
    ('ċ', "c", 10),
    ('č', "c", 7),
    ('ď', "d", 7),
    ('đ', "d", 11),
    ('ð', "d", 18),
    ('è', "e", 2),
    ('é', "e", 1),
    ('ê', "e", 3),
    ('ë', "e", 4),
    ('ē', "e", 13),
    ('ĕ', "e", 12),
    ('ė', "e", 10),
    ('ę', "e", 9),
    ('ě', "e", 7),
    ('ĝ', "g", 3),
    ('ğ', "g", 12),
    ('ġ', "g", 10),
    ('ģ', "g", 8),
    ('ĥ', "h", 3),
    ('ħ', "h", 11),
    ('ì', "i", 2),
    ('í', "i", 1),
    ('î', "i", 3),
    ('ï', "i", 4),
    ('ĩ', "i", 5),
    ('ī', "i", 13),
    ('ĭ', "i", 12),
    ('į', "i", 9),
    ('ı', "i", 17),
    ('ĳ', "ij", 16),
    ('ĵ', "j", 3),
    ('ķ', "k", 8),
    ('ĺ', "l", 1),
    ('ļ', "l", 8),
    ('ľ', "l", 7),
    ('ŀ', "l", 10),
    ('ł', "l", 11),
    ('ñ', "n", 5),
    ('ń', "n", 1),
    ('ņ', "n", 8),
    ('ň', "n", 7),
    ('ò', "o", 2),
    ('ó', "o", 1),
    ('ô', "o", 3),
    ('õ', "o", 5),
    ('ö', "o", 4),
    ('ø', "o", 11),
    ('ō', "o", 13),
    ('ŏ', "o", 12),
    ('ő', "o", 14),
    ('œ', "oe", 16),
    ('ŕ', "r", 1),
    ('ŗ', "r", 8),
    ('ř', "r", 7),
    ('ś', "s", 1),
    ('ŝ', "s", 3),
    ('ş', "s", 8),
    ('š', "s", 7),
    ('ș', "s", 15),
    ('ß', "ss", 16),
    ('ţ', "t", 8),
    ('ť', "t", 7),
    ('ŧ', "t", 11),
    ('ț', "t", 15),
    ('ù', "u", 2),
    ('ú', "u", 1),
    ('û', "u", 3),
    ('ü', "u", 4),
    ('ũ', "u", 5),
    ('ū', "u", 13),
    ('ŭ', "u", 12),
    ('ů', "u", 6),
    ('ű', "u", 14),
    ('ų', "u", 9),
    ('ŵ', "w", 3),
    ('ý', "y", 1),
    ('ÿ', "y", 4),
    ('ŷ', "y", 3),
    ('ź', "z", 1),
    ('ż', "z", 10),
    ('ž', "z", 7),
];

/// A letter (or letter group) a language sorts as a letter of its own: right after `after`
/// (`rank` places after it), with secondary weight `sec` among letters of the same place.
struct Tailor {
    seq: &'static str,
    after: char,
    rank: u32,
    sec: u16,
}

const fn t(seq: &'static str, after: char, rank: u32) -> Tailor {
    Tailor { seq, after, rank, sec: 0 }
}

const fn ts(seq: &'static str, after: char, rank: u32, sec: u16) -> Tailor {
    Tailor { seq, after, rank, sec }
}

const PL: &[Tailor] =
    &[t("ą", 'a', 1), t("ć", 'c', 1), t("ę", 'e', 1), t("ł", 'l', 1), t("ń", 'n', 1), t("ó", 'o', 1), t("ś", 's', 1), t("ź", 'z', 1), t("ż", 'z', 2)];
const CS: &[Tailor] = &[t("č", 'c', 1), t("ch", 'h', 1), t("ř", 'r', 1), t("š", 's', 1), t("ž", 'z', 1)];
const SK: &[Tailor] = &[t("ä", 'a', 1), t("č", 'c', 1), t("ch", 'h', 1), t("ô", 'o', 1), t("š", 's', 1), t("ž", 'z', 1)];
const HU: &[Tailor] = &[
    t("cs", 'c', 1),
    t("dz", 'd', 1),
    t("dzs", 'd', 2),
    t("gy", 'g', 1),
    t("ly", 'l', 1),
    t("ny", 'n', 1),
    t("ö", 'o', 1),
    ts("ő", 'o', 1, 14),
    t("sz", 's', 1),
    t("ty", 't', 1),
    t("ü", 'u', 1),
    ts("ű", 'u', 1, 14),
    t("zs", 'z', 1),
];
const SV: &[Tailor] = &[t("å", 'z', 1), t("ä", 'z', 2), ts("æ", 'z', 2, 16), t("ö", 'z', 3), ts("ø", 'z', 3, 11)];
const DA: &[Tailor] = &[t("æ", 'z', 1), ts("ä", 'z', 1, 4), t("ø", 'z', 2), ts("ö", 'z', 2, 4), t("å", 'z', 3)];
const ES: &[Tailor] = &[t("ñ", 'n', 1)];
const TR: &[Tailor] = &[t("ç", 'c', 1), t("ğ", 'g', 1), t("ı", 'h', 1), t("ö", 'o', 1), t("ş", 's', 1), t("ü", 'u', 1)];
const HR: &[Tailor] =
    &[t("č", 'c', 1), t("ć", 'c', 2), t("dž", 'd', 1), t("đ", 'd', 2), t("lj", 'l', 1), t("nj", 'n', 1), t("š", 's', 1), t("ž", 'z', 1)];
const RO: &[Tailor] = &[t("ă", 'a', 1), t("â", 'a', 2), t("î", 'i', 1), t("ș", 's', 1), ts("ş", 's', 1, 8), t("ț", 't', 1), ts("ţ", 't', 1, 8)];
const ET: &[Tailor] = &[t("š", 's', 1), t("z", 's', 2), t("ž", 's', 3), t("õ", 'w', 1), t("ä", 'w', 2), t("ö", 'w', 3), t("ü", 'w', 4)];
const LT: &[Tailor] = &[t("č", 'c', 1), t("š", 's', 1), t("ž", 'z', 1)];
const LV: &[Tailor] = &[t("č", 'c', 1), t("ģ", 'g', 1), t("ķ", 'k', 1), t("ļ", 'l', 1), t("ņ", 'n', 1), t("š", 's', 1), t("ž", 'z', 1)];
const IS: &[Tailor] = &[
    t("á", 'a', 1),
    t("ð", 'd', 1),
    t("é", 'e', 1),
    t("í", 'i', 1),
    t("ó", 'o', 1),
    t("ú", 'u', 1),
    t("ý", 'y', 1),
    t("þ", 'z', 1),
    t("æ", 'z', 2),
    t("ö", 'z', 3),
];

/// The primary language subtag of a BCP 47 tag (`pl-PL` → `pl`), lowercased.
pub fn language(lang: Option<&str>) -> String {
    lang.and_then(|l| l.split(['-', '_']).next()).unwrap_or("").trim().to_ascii_lowercase()
}

fn tailoring(lang: Option<&str>) -> &'static [Tailor] {
    match language(lang).as_str() {
        "pl" => PL,
        "cs" => CS,
        "sk" => SK,
        "hu" => HU,
        "sv" | "fi" => SV,
        "da" | "nb" | "nn" | "no" => DA,
        "es" | "gl" => ES,
        "tr" | "az" => TR,
        "hr" | "bs" | "sr" => HR,
        "ro" => RO,
        "et" => ET,
        "lt" => LT,
        "lv" => LV,
        "is" => IS,
        "sl" => LT,
        _ => &[],
    }
}

fn weight(c: char, rank: u32) -> u32 {
    ((c as u32) << 4).saturating_add(rank)
}

/// One collation element: primary weight, secondary weight, the text it covers.
struct Elem<'a> {
    primary: Vec<u32>,
    sec: u16,
    text: &'a str,
    /// Set when the text is a letter of its own in the language (shown as is in headings).
    own: bool,
}

/// The next collation element of `rest` (lowercase), and its length in bytes.
fn next_elem<'a>(rest: &'a str, rules: &[Tailor]) -> Option<(Elem<'a>, usize)> {
    if let Some(r) = rules.iter().filter(|r| rest.starts_with(r.seq)).max_by_key(|r| r.seq.len()) {
        let text = rest.get(..r.seq.len())?;
        return Some((Elem { primary: vec![weight(r.after, r.rank)], sec: r.sec, text, own: true }, r.seq.len()));
    }
    let c = rest.chars().next()?;
    let n = c.len_utf8();
    let text = rest.get(..n)?;
    if let Some((_, base, mark)) = DECOMP.iter().find(|(x, _, _)| *x == c) {
        return Some((Elem { primary: base.chars().map(|b| weight(b, 0)).collect(), sec: *mark, text, own: false }, n));
    }
    // A combining mark adds a secondary difference to the letter before it.
    if ('\u{300}'..='\u{36f}').contains(&c) {
        return Some((Elem { primary: Vec::new(), sec: 32 + (c as u32 - 0x300) as u16, text, own: false }, n));
    }
    Some((Elem { primary: vec![weight(c, 0)], sec: 0, text, own: false }, n))
}

/// A sort key: compare keys to sort strings in a language's order. Equal keys mean the strings
/// differ at most in letter case.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct SortKey {
    primary: Vec<u32>,
    secondary: Vec<u16>,
    text: String,
}

/// The sort key of `s` for language `lang` (a BCP 47 tag such as `pl-PL`; `None` sorts like
/// English).
pub fn sort_key(s: &str, lang: Option<&str>) -> SortKey {
    let text = s.to_lowercase();
    let rules = tailoring(lang);
    let mut key = SortKey::default();
    let mut i = 0;
    while let Some(rest) = text.get(i..).filter(|r| !r.is_empty()) {
        let Some((e, n)) = next_elem(rest, rules) else { break };
        i += n.max(1);
        if e.primary.is_empty() {
            if let Some(last) = key.secondary.last_mut() {
                *last = last.saturating_add(e.sec);
            }
            continue;
        }
        for (k, w) in e.primary.iter().enumerate() {
            key.primary.push(*w);
            key.secondary.push(if k == 0 { e.sec } else { 0 });
        }
    }
    key.text = text;
    key
}

/// Compare two strings in the order of language `lang`, ignoring case.
pub fn collate(a: &str, b: &str, lang: Option<&str>) -> Ordering {
    sort_key(a, lang).cmp(&sort_key(b, lang))
}

/// The index heading letter `s` files under in language `lang`: `Ł` for `Łódź` in Polish, `L`
/// in English; `Ch` for `chata` in Czech; `A` for `Äpfel` in German, `Ä` in Swedish.
pub fn heading_letter(s: &str, lang: Option<&str>) -> Option<String> {
    let low = s.trim_start().to_lowercase();
    let (e, _) = next_elem(&low, tailoring(lang))?;
    let shown = if e.own || e.sec == 0 {
        e.text.to_string()
    } else {
        DECOMP.iter().find(|(x, _, _)| e.text.starts_with(*x)).map(|(_, b, _)| b.chars().take(1).collect()).unwrap_or_else(|| e.text.to_string())
    };
    let mut cs = shown.chars();
    let first = cs.next()?;
    Some(first.to_uppercase().chain(cs).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(words: &[&str], lang: &str) -> Vec<String> {
        let mut v: Vec<String> = words.iter().map(|w| w.to_string()).collect();
        v.sort_by(|a, b| collate(a, b, Some(lang)));
        v
    }

    #[test]
    fn polish_letters_follow_their_base_letter() {
        assert_eq!(sorted(&["Zakopane", "Łódź", "Kraków"], "pl-PL"), ["Kraków", "Łódź", "Zakopane"]);
        assert_eq!(sorted(&["zebra", "ćma", "dom"], "pl-PL"), ["ćma", "dom", "zebra"]);
        assert_eq!(sorted(&["żaba", "źle", "zebra", "ząb", "lato", "łapa", "mama"], "pl"), ["lato", "łapa", "mama", "ząb", "zebra", "źle", "żaba"]);
        assert_eq!(sorted(&["b", "ą", "a"], "pl"), ["a", "ą", "b"]);
    }

    #[test]
    fn swedish_puts_a_ring_and_umlauts_after_z() {
        assert_eq!(sorted(&["ödla", "zebra", "äpple", "åka", "apa"], "sv-SE"), ["apa", "zebra", "åka", "äpple", "ödla"]);
    }

    #[test]
    fn german_sorts_umlauts_with_their_base_letter() {
        assert_eq!(sorted(&["Zug", "Äpfel", "Apfel", "Bach"], "de-DE"), ["Apfel", "Äpfel", "Bach", "Zug"]);
        assert_eq!(sorted(&["Straße", "Strasse", "Strand"], "de"), ["Strand", "Strasse", "Straße"]);
    }

    #[test]
    fn spanish_n_tilde_after_n() {
        assert_eq!(sorted(&["oso", "ñu", "nube"], "es-ES"), ["nube", "ñu", "oso"]);
    }

    #[test]
    fn czech_slovak_hungarian_alphabets() {
        assert_eq!(sorted(&["chata", "hrad", "ihla", "cesta", "čaj", "dům"], "cs-CZ"), ["cesta", "čaj", "dům", "hrad", "chata", "ihla"]);
        assert_eq!(sorted(&["ôsmy", "oko", "pes", "ä", "b"], "sk"), ["ä", "b", "oko", "ôsmy", "pes"]);
        assert_eq!(sorted(&["csak", "cukor", "dob", "szó", "sűrű", "tál"], "hu-HU"), ["cukor", "csak", "dob", "sűrű", "szó", "tál"]);
        // ö and ő are one letter after o, told apart by the accent only.
        assert_eq!(sorted(&["ötös", "őr", "pálya", "olló", "ör"], "hu"), ["olló", "ör", "őr", "ötös", "pálya"]);
    }

    #[test]
    fn english_keeps_case_insensitive_order_and_folds_accents() {
        assert_eq!(sorted(&["rex", "résumé", "resume", "Apple", "banana"], "en-US"), ["Apple", "banana", "resume", "résumé", "rex"]);
        assert_eq!(collate("Łódź", "Lodz", None), Ordering::Greater);
        assert_eq!(collate("Łódź", "Lodzia", None), Ordering::Less);
        assert_eq!(collate("ABC", "abc", None), Ordering::Equal);
    }

    #[test]
    fn heading_letters() {
        assert_eq!(heading_letter("Łódź", Some("pl-PL")).as_deref(), Some("Ł"));
        assert_eq!(heading_letter("Łódź", Some("en-US")).as_deref(), Some("L"));
        assert_eq!(heading_letter("chata", Some("cs")).as_deref(), Some("Ch"));
        assert_eq!(heading_letter("Äpfel", Some("de")).as_deref(), Some("A"));
        assert_eq!(heading_letter("Äpple", Some("sv")).as_deref(), Some("Ä"));
        assert_eq!(heading_letter("", Some("sv")), None);
    }

    #[test]
    fn hostile_input_never_panics() {
        for s in ["", "\u{301}", "a\u{301}\u{301}\u{301}", "dzs", "\u{10FFFF}", "ẞ", "İstanbul"] {
            for l in [None, Some("hu"), Some(""), Some("-"), Some("tr-TR")] {
                let _ = sort_key(s, l);
                let _ = heading_letter(s, l);
            }
        }
    }
}

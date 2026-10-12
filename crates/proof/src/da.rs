//! Danish spelling: every word form in Retskrivningsordbogen (via COR₁, CC0), compound analysis
//! and suggestions.
//!
//! A word is correct if it is a listed form, a capitalised or all-caps listed form, a hyphenated
//! chain of correct parts, or a compound: one or more compound-first parts ([`FIRST`]) followed by
//! an inflected noun, adjective or verb ([`HEAD`]); first parts have at least [`MIN_FIRST`]
//! characters, the head at least [`MIN_PART`].

use crate::lexicon::{FIRST, HEAD, Lexicon, UNNORMED, WORD};
use crate::{Issue, IssueKind, ProofLang};
use std::sync::OnceLock;

static DA_LEX: &[u8] = include_bytes!("../../../assets/proofing/da.lex");

/// Shortest compound head, in characters ("tog" + "station").
const MIN_PART: usize = 3;
/// Shortest compound-first part ("by-", "el-", "ny-").
const MIN_FIRST: usize = 2;
/// Frequency class (corpus count ≥ 2⁷) of a word that an unseen compound one edit away is
/// probably a typo of.
const NEAR_COMMON: u8 = 8;
/// Frequency class (corpus count ≥ 2⁵) of a join that [`split_compound`] suggests.
const SPLIT_COMMON: u8 = 6;
/// Frequency class (corpus count ≥ 2¹⁴) of words [`split_compound`] treats as function words.
const SPLIT_FUNCTION: u8 = 15;
/// Longest word analysed as a compound, in characters.
const MAX_COMPOUND: usize = 64;

/// The bundled Danish lexicon (decoded on first use).
pub fn lexicon() -> &'static Lexicon {
    static L: OnceLock<Lexicon> = OnceLock::new();
    L.get_or_init(|| {
        Lexicon::from_bytes(DA_LEX).unwrap_or_else(|e| {
            log::error!("{e}");
            Lexicon::default()
        })
    })
}

fn capitalised(w: &str) -> String {
    let mut c = w.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// Is `w` a listed form, allowing sentence-start capitals and all caps?
fn listed(lex: &Lexicon, w: &str) -> bool {
    if lex.has(w, WORD) {
        return true;
    }
    let lower = w.to_lowercase();
    let first_upper = w.chars().next().is_some_and(char::is_uppercase);
    let all_upper = w.chars().all(|c| !c.is_lowercase());
    if first_upper && w.chars().skip(1).all(|c| !c.is_uppercase()) && lex.has(&lower, WORD) {
        return true;
    }
    all_upper && (lex.has(&lower, WORD) || lex.has(&capitalised(&lower), WORD))
}

/// Can `w` (lowercase) be split into compound-first parts followed by a head?
pub fn is_compound(lex: &Lexicon, w: &str) -> bool {
    let bounds: Vec<usize> = w.char_indices().map(|(i, _)| i).chain(std::iter::once(w.len())).collect();
    let n = bounds.len() - 1;
    if !(MIN_FIRST + MIN_PART..=MAX_COMPOUND).contains(&n) {
        return false;
    }
    // reach[k]: the first k chars are a chain of compound-first parts.
    let mut reach = vec![false; n + 1];
    if let Some(r) = reach.get_mut(0) {
        *r = true;
    }
    for i in 0..n {
        if !reach.get(i).copied().unwrap_or(false) {
            continue;
        }
        let Some(&a) = bounds.get(i) else { continue };
        // A head ends the word.
        if i > 0 && n - i >= MIN_PART && w.get(a..).is_some_and(|h| lex.has(h, HEAD)) {
            return true;
        }
        for j in i + MIN_FIRST..=n.saturating_sub(MIN_PART) {
            if let (Some(&b), Some(r)) = (bounds.get(j), reach.get(j))
                && !r
                && w.get(a..b).is_some_and(|p| lex.has(p, FIRST))
                && let Some(r) = reach.get_mut(j)
            {
                *r = true;
            }
        }
    }
    false
}

/// Is `word` spelled correctly in Danish? Numbers, single letters, short words in capitals
/// (acronyms), words with digits, words in other scripts and user-dictionary words are accepted.
pub fn is_correct(word: &str) -> bool {
    let w = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '’');
    let w = w.trim_matches(['\'', '’']);
    let n = w.chars().count();
    if n <= 1 || w.chars().any(|c| c.is_ascii_digit()) || !w.chars().any(char::is_alphabetic) {
        return true;
    }
    if w.chars().all(|c| !c.is_lowercase()) && n <= 6 {
        return true;
    }
    if !w.chars().any(|c| c.is_alphabetic() && c < '\u{0250}') {
        return true; // other scripts: no dictionary
    }
    let w = w.replace('’', "'");
    crate::in_user_dictionary(&w.to_lowercase()) || known(&w)
}

/// Is a trimmed word a listed form or a compound of listed parts?
fn known(w: &str) -> bool {
    let lex = lexicon();
    if listed(lex, w) {
        return true;
    }
    if w.contains('-') {
        // "EU-borger", "e-mail-adresse", "sommer-" in "sommer- og vinterferie".
        return w.split('-').all(|p| p.is_empty() || part_ok(lex, p));
    }
    let lower = w.to_lowercase();
    // Proper-noun first part keeps its capital: "Københavnsområdet", "Øresundsregionen".
    (is_compound(lex, &lower) || (lower != w && is_compound(lex, w))) && !near_common(lex, &lower)
}

/// Is `w` one edit away from a common listed form? An unseen compound that close to a common word
/// is more likely a typo of it ("bøgerene" for "bøgerne" parses as "bøge" + "rene").
fn near_common(lex: &Lexicon, w: &str) -> bool {
    const ALPHABET: &str = "abcdefghijklmnopqrstuvwxyzæøå";
    let common = |v: &str| v != w && lex.has(v, WORD) && lex.freq(v) >= NEAR_COMMON;
    let cs: Vec<char> = w.chars().collect();
    let s = |v: &[char]| v.iter().collect::<String>();
    for i in 0..=cs.len() {
        if i < cs.len() {
            let mut v = cs.clone();
            v.remove(i);
            if common(&s(&v)) {
                return true;
            }
            if i + 1 < cs.len() {
                let mut v = cs.clone();
                v.swap(i, i + 1);
                if common(&s(&v)) {
                    return true;
                }
            }
        }
        for c in ALPHABET.chars() {
            let mut v = cs.clone();
            v.insert(i, c);
            if common(&s(&v)) {
                return true;
            }
            if cs.get(i).is_some_and(|&slot| slot != c) {
                let mut v = cs.clone();
                if let Some(x) = v.get_mut(i) {
                    *x = c;
                }
                if common(&s(&v)) {
                    return true;
                }
            }
        }
    }
    false
}

/// Is "`w0` `w1`" a compound written apart ("tog station")? Returns the joined word when the join
/// is a common listed form and Danish text doesn't write the pair apart ("over for", "lav et").
pub fn split_compound(w0: &str, w1: &str) -> Option<String> {
    let lex = lexicon();
    // Both parts are nouns, adjectives or verbs of three letters or more, and not among the most
    // frequent words ("for", "men", "lige"): "til ses", "by er", "for ældre" stay apart.
    let part = |w: &str| w.chars().count() >= 3 && w.chars().all(char::is_lowercase) && lex.has(w, HEAD) && lex.freq(w) < SPLIT_FUNCTION;
    if !part(w0) || !part(w1) {
        return None;
    }
    let joined = format!("{w0}{w1}");
    let common = lex.has(&joined, WORD) && lex.freq(&joined) >= SPLIT_COMMON;
    (common && lex.get(&format!("{w0} {w1}")).is_none()).then_some(joined)
}

/// Present tense after the infinitive marker or a modal verb ("at bliver", "vil står")? Returns the
/// infinitive.
pub fn present_for_infinitive(w0: &str, w1: &str) -> Option<String> {
    const MARKERS: [&str; 11] = ["at", "kan", "skal", "vil", "må", "bør", "kunne", "skulle", "ville", "burde", "måtte"];
    if !MARKERS.contains(&w0.to_lowercase().as_str()) || !w1.chars().all(char::is_lowercase) {
        return None;
    }
    let lex = lexicon();
    let inf = w1.strip_suffix('r')?;
    // "bliver" → "blive", "står" → "stå": the infinitive ends in a vowel and is itself a verb form.
    let vowel_end = inf.chars().last().is_some_and(|c| "aeiouyæøå".contains(c));
    // Only forms with no noun or adverb reading: "at mennesker", "kan heller" are fine.
    let present_only = lex.get(&format!("~{w1}")).is_some();
    (vowel_end && inf.chars().count() >= 2 && present_only && lex.has(inf, HEAD)).then(|| inf.to_string())
}

fn part_ok(lex: &Lexicon, p: &str) -> bool {
    (p.chars().all(|c| !c.is_lowercase()) && p.chars().count() <= 6)
        || listed(lex, p)
        || lex.has(&p.to_lowercase(), FIRST)
        || is_compound(lex, &p.to_lowercase())
}

/// Old spellings and ASCII stand-ins: aa → å, ae → æ, oe → ø (every combination, capped), then
/// single o → ø, a → å/æ, e → æ.
fn letter_rewrites(w: &str) -> Vec<String> {
    let mut out = vec![w.to_string()];
    for (from, to) in [("aa", "å"), ("ae", "æ"), ("oe", "ø")] {
        let mut next = Vec::new();
        for s in &out {
            for (i, _) in s.match_indices(from) {
                if let (Some(a), Some(b)) = (s.get(..i), s.get(i + from.len()..)) {
                    next.push(format!("{a}{to}{b}"));
                }
            }
        }
        out.extend(next);
        out.truncate(16);
    }
    // One Danish letter typed without its key: "gron" → "grøn", "pa" → "på".
    for (from, to) in [('o', 'ø'), ('a', 'å'), ('a', 'æ'), ('e', 'æ')] {
        for (i, c) in w.char_indices() {
            if c == from
                && let (Some(a), Some(b)) = (w.get(..i), w.get(i + c.len_utf8()..))
            {
                out.push(format!("{a}{to}{b}"));
            }
        }
    }
    out.into_iter().skip(1).take(48).collect()
}

/// Suggestions for a misspelled Danish word, best first, in the word's letter case.
pub fn suggest(word: &str, max: usize) -> Vec<String> {
    let lex = lexicon();
    let lower = word.to_lowercase();
    let n = lower.chars().count();
    if n == 0 || n > 40 {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    let push = |s: String, out: &mut Vec<String>| {
        if !out.contains(&s) {
            out.push(s);
        }
    };
    // Proper nouns typed in lower case ("københavn" → "København"), then old spellings and missing
    // Danish letters, most frequent first.
    let mut quick: Vec<String> = Vec::new();
    let cap = capitalised(&lower);
    if lex.has(&cap, WORD) {
        quick.push(cap);
    }
    for r in letter_rewrites(&lower) {
        if lex.has(&r, WORD) || is_compound(lex, &r) {
            quick.push(r);
        } else if lex.has(&capitalised(&r), WORD) {
            quick.push(capitalised(&r));
        }
    }
    quick.sort_by_key(|w| std::cmp::Reverse(lex.freq(w)));
    for w in quick {
        push(w, &mut out);
    }
    // Edit distance ≤ 2 over every form.
    // ponytail: linear scan (~660k forms); a delete index if suggestions get slow.
    let target: Vec<char> = lower.chars().collect();
    let first = target.first().copied();
    let mut scored: Vec<(usize, u8, std::cmp::Reverse<u8>, usize, &str)> = Vec::new();
    for (w, f) in lex.iter() {
        if f & WORD == 0 {
            continue;
        }
        let wn = w.chars().count();
        if !(n.saturating_sub(2)..=n + 2).contains(&wn) {
            continue;
        }
        // Forms with apostrophes or full stops ("v'er", "inkl.") only when the typed word has them.
        if w.chars().any(|c| !c.is_alphabetic() && c != '-' && !lower.contains(c)) {
            continue;
        }
        let wl = w.to_lowercase();
        let d = crate::damerau(&target, &wl);
        if d <= 2 {
            let proper = wl != w;
            let rank = u8::from(proper) * 4 + u8::from(f & UNNORMED != 0) * 2 + u8::from(wl.chars().next() != first);
            scored.push((d, rank, std::cmp::Reverse(f >> 4), wn.abs_diff(n), w));
        }
    }
    scored.sort();
    for (_, _, _, _, w) in scored {
        if out.len() >= max {
            break;
        }
        push(w.to_string(), &mut out);
    }
    out.into_iter().take(max).map(|w| match_case(word, w)).collect()
}

/// `w` in the case pattern of `typed`: ALL CAPS or Capitalised (never lowers a suggestion).
fn match_case(typed: &str, w: String) -> String {
    if typed.chars().count() > 1 && typed.chars().all(|c| !c.is_lowercase()) {
        w.to_uppercase()
    } else if typed.chars().next().is_some_and(char::is_uppercase) {
        capitalised(&w)
    } else {
        w
    }
}

/// Danish grammar issues in a paragraph: compounds written apart ("tog station" → "togstation")
/// and present tense after "at" or a modal verb ("at bliver" → "at blive").
pub fn check_grammar(text: &str) -> Vec<Issue> {
    let mut v = Vec::new();
    for pair in crate::words(text).windows(2) {
        let [(a0, b0), (a1, b1)] = pair else { continue };
        if text.get(*b0..*a1) != Some(" ") {
            continue;
        }
        let (Some(w0), Some(w1)) = (text.get(*a0..*b0), text.get(*a1..*b1)) else { continue };
        let issue = |start: usize, message: String, fix: String| Issue {
            start,
            end: *b1,
            kind: IssueKind::Grammar,
            message,
            suggestions: vec![fix],
            lang: ProofLang::Da,
        };
        if let Some(joined) = split_compound(w0, w1) {
            v.push(issue(*a0, "Mulig særskrivning".into(), joined));
        }
        if let Some(inf) = present_for_infinitive(w0, w1) {
            v.push(issue(*a1, format!("Brug navneform efter »{w0}«"), inf));
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(w: &str) -> bool {
        crate::is_correct_in(w, ProofLang::Da)
    }

    #[test]
    fn lexicon_loads() {
        assert!(lexicon().len() > 400_000, "{}", lexicon().len());
    }

    #[test]
    fn listed_forms() {
        for w in [
            "hus",
            "huset",
            "husene",
            "bog",
            "bøger",
            "bøgerne",
            "skole",
            "kærlighed",
            "morgenmad",
            "vejr",
            "børn",
            "grøn",
            "løber",
            "løbet",
            "løbende",
            "største",
            "på",
            "gård",
            "åbent",
            "kål",
            "Aarhus",
            "Århus",
            "København",
            "Odense",
            "Nyborg",
            "computer",
            "weekend",
            "smartphone",
            "feedback",
            "workshop",
            "deadline",
            "brunch",
            "selfie",
            "podcast",
            "e-mail",
            "Huset",
            "HUSET",
            "inkl",
            "og",
            "af",
            "bl",
            "én",
            "dets",
            "oftest",
        ] {
            assert!(ok(w), "{w}");
        }
    }

    #[test]
    fn compounds() {
        for w in [
            "sommerferie",
            "kaffekop",
            "togstation",
            "spørgeskema",
            "cykelstativ",
            "kaffekoppen",
            "arbejdsplads",
            "børnehave",
            "Sommerferien",
            "EU-borger",
            "e-mail-adresse",
            "Københavnsområdet",
            "bybrand",
            "elproduktionen",
            "nystiftede",
        ] {
            assert!(ok(w), "{w}");
        }
    }

    #[test]
    fn misspellings() {
        for w in [
            "huus",
            "bokg",
            "skoel",
            "kærighed",
            "morgnemad",
            "vjer",
            "bøern",
            "gron",
            "husne",
            "bøgerene",
            "størreste",
            "paa",
            "gaard",
            "aabent",
            "kaal",
            "københavn",
            "odense",
            "aarhus",
            "Zxqvland",
        ] {
            assert!(!ok(w), "{w}");
        }
    }

    #[test]
    fn split_compounds() {
        assert_eq!(split_compound("tog", "station").as_deref(), Some("togstation"));
        assert_eq!(split_compound("sommer", "ferie").as_deref(), Some("sommerferie"));
        for (a, b) in [
            ("over", "for"),
            ("lav", "et"),
            ("selv", "om"),
            ("en", "gang"),
            ("et", "hus"),
            ("by", "er"),
            ("op", "ad"),
            ("for", "ældre"),
            ("til", "ses"),
            ("Tog", "station"),
        ] {
            assert_eq!(split_compound(a, b), None, "{a} {b}");
        }
    }

    #[test]
    fn present_after_marker() {
        let p = |a: &str, b: &str| present_for_infinitive(a, b);
        assert_eq!(p("at", "investerer").as_deref(), Some("investere"));
        assert_eq!(p("vil", "bliver").as_deref(), Some("blive"));
        assert_eq!(p("Skal", "står").as_deref(), Some("stå"));
        for (a, b) in [
            ("at", "spise"),
            ("han", "spiser"),
            ("at", "der"),
            ("at", "Peter"),
            ("vil", "gerne"),
            ("at", "mennesker"),
            ("kan", "heller"),
            ("at", "når"),
            ("at", "læser"),
        ] {
            assert_eq!(p(a, b), None, "{a} {b}");
        }
    }

    #[test]
    fn suggestions() {
        let s = |w: &str| suggest(w, 3);
        assert_eq!(s("paa").first().map(String::as_str), Some("på"));
        assert!(s("gaard").contains(&"gård".to_string()), "{:?}", s("gaard"));
        assert!(s("aabent").contains(&"åbent".to_string()), "{:?}", s("aabent"));
        assert_eq!(s("københavn").first().map(String::as_str), Some("København"));
        assert_eq!(s("aarhus").first().map(String::as_str), Some("Aarhus"));
        for (bad, good) in
            [("skoel", "skole"), ("kærighed", "kærlighed"), ("morgnemad", "morgenmad"), ("vjer", "vejr"), ("gron", "grøn"), ("bokg", "bog")]
        {
            assert!(s(bad).contains(&good.to_string()), "{bad}: {:?}", s(bad));
        }
    }
}

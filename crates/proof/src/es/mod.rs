//! Spanish proofing.
//!
//! - **Spelling** against the Hunspell `es_MX` / `es_ES` / `es` dictionary installed on the system
//!   ([`dictionary`], found by [`crate::hunspell`]: `es_MX.aff`/`es_MX.dic`, `es_ES.aff`/`es_ES.dic`
//!   from the OS, a package manager or LibreOffice; it isn't bundled). Without it Spanish text gets
//!   no spelling marks. Capitalized stems are names (`México`); a sentence may start with any word
//!   and headings may be in capitals. Hyphenated compounds are checked part by part, and the user
//!   dictionary applies as in English.
//! - **Suggestions** tuned for Spanish ([`suggest`]): missing accents first (`cancion` →
//!   `canción`, `mexico` → `México`), then the dictionary's `REP` confusions, a missing space,
//!   one-letter typos, and a dictionary scan for anything further off.
//! - **Grammar** ([`grammar`]): cheap, reliable rules with Spanish messages.
//! - **Hyphenation** ([`hyphen`]): Spanish syllable rules.

pub mod grammar;
pub mod hyphen;
mod suggest;
#[cfg(test)]
pub(crate) mod test_dict;

use std::sync::{Arc, RwLock};

use crate::affix::AffixDict;

pub use suggest::suggest;

/// The Spanish dictionary: `None` until first use, then the installed one (or an empty one).
static DICT: RwLock<Option<Arc<AffixDict>>> = RwLock::new(None);

/// File names of a Spanish Hunspell dictionary (favoring Mexican Spanish if installed, then Spain / general).
const NAMES: &[&str] = &["es_MX", "es-MX", "es_ES", "es-ES", "es"];

/// The Spanish dictionary: the installed Hunspell dictionary (read on first use), one given with
/// [`install_dictionary`], or an empty one when none is available.
pub fn dictionary() -> Arc<AffixDict> {
    if let Some(d) = DICT.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
        return d.clone();
    }
    let loaded = Arc::new(load_installed());
    let mut slot = DICT.write().unwrap_or_else(|e| e.into_inner());
    slot.get_or_insert(loaded).clone()
}

fn load_installed() -> AffixDict {
    let Some((aff, dic)) = crate::hunspell::find(NAMES) else {
        log::info!("Spanish spelling: no Hunspell es dictionary installed");
        return AffixDict::default();
    };
    crate::hunspell::load(&aff, &dic).unwrap_or_else(|e| {
        log::warn!("Spanish spelling: {e}");
        AffixDict::default()
    })
}

/// Use this Spanish dictionary from now on (Hunspell `.aff` and `.dic` text, decoded to UTF-8).
pub fn install_dictionary(aff: &str, dic: &str) -> Result<(), String> {
    let d = AffixDict::from_hunspell(aff, dic, &[])?;
    *DICT.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(d));
    Ok(())
}

/// Forget the loaded dictionary, so the next check looks for an installed one again.
pub fn reload_dictionary() {
    *DICT.write().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Is a Spanish dictionary available (spelling is checked)?
pub fn has_dictionary() -> bool {
    !dictionary().is_empty()
}

fn is_latin_letter(c: char) -> bool {
    c.is_alphabetic() && (c < '\u{0250}' || ('\u{1E00}'..='\u{1EFF}').contains(&c))
}

/// Is `word` spelled correctly in Spanish?
pub fn is_correct(word: &str) -> bool {
    is_correct_with(&dictionary(), word)
}

/// [`is_correct`] against a given dictionary (an empty one accepts everything).
pub fn is_correct_with(d: &AffixDict, word: &str) -> bool {
    if d.is_empty() {
        return true;
    }
    let w = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '’');
    let w = w.trim_matches(['\'', '’']);
    let n = w.chars().count();
    if n <= 1 || w.chars().any(|c| c.is_ascii_digit()) || !w.chars().any(char::is_alphabetic) {
        return true;
    }
    if w.chars().all(|c| !c.is_lowercase()) && n <= 6 {
        return true;
    }
    if !w.chars().any(is_latin_letter) {
        return true;
    }
    if w.contains('-') {
        return w.split('-').all(|p| p.chars().count() <= 1 || known(d, p));
    }
    known(d, w)
}

fn known(d: &AffixDict, w: &str) -> bool {
    d.check(w) || crate::in_user_dictionary(&w.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dictionary_loads() {
        test_dict::install();
        let d = dictionary();
        assert!(has_dictionary());
        assert!(d.len() > 40, "{}", d.len());
        assert!(d.rule_count() >= 3, "affix rules");
    }

    #[test]
    fn without_a_dictionary_nothing_is_flagged() {
        let none = AffixDict::default();
        for w in ["cancion", "arbol", "xqzv", ""] {
            assert!(is_correct_with(&none, w), "{w}");
        }
    }

    #[test]
    fn spanish_words_are_correct() {
        test_dict::install();
        for w in [
            "México",
            "árbol",
            "canción",
            "español",
            "computadora",
            "trabajo",
            "presidente",
            "escribir",
            "niño",
            "éxito",
            "decisión",
            "casa",
            "habéis",
            "cantando",
            "hablaba",
            "descubrir",
            "bien",
            "MÉXICO",
            "2026",
            "e-mail",
            "sí",
            "no",
        ] {
            assert!(is_correct(w), "{w}");
        }
    }

    #[test]
    fn misspellings_are_flagged() {
        test_dict::install();
        for w in ["cancion", "arbol", "mexico", "exito", "decicion", "cantandoo"] {
            assert!(!is_correct(w), "{w}");
        }
    }

    #[test]
    fn user_dictionary_applies() {
        test_dict::install();
        assert!(!is_correct("zxqspanishpalabra"));
        crate::add_word("Zxqspanishpalabra");
        assert!(is_correct("zxqspanishpalabra"));
    }
}

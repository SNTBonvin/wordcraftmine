//! Portuguese proofing.
//!
//! - **Spelling** against the Hunspell `pt_BR` dictionary installed on the system
//!   ([`dictionary`], found by [`crate::hunspell`]: `pt_BR.aff`/`pt_BR.dic` — VERO and its
//!   derivatives — from the OS, a package manager or LibreOffice; it isn't bundled). Without it
//!   Portuguese text gets no spelling marks. Capitalized stems are names (`Brasília`); a sentence
//!   may start with any word and headings may be in capitals. Hyphenated compounds are checked
//!   part by part (`guarda-roupa`, `ex-marido`), and the user dictionary applies as in English.
//! - **Two spellings** ([`Variant`]): European Portuguese (`pt-PT`) is checked against the
//!   Hunspell `pt_PT` dictionary (`pt_PT.aff`/`pt_PT.dic`, found the same way), so `facto`,
//!   `equipa` and `económico` are words there and `econômico` isn't. Without a `pt_PT`
//!   dictionary European text falls back to the Brazilian one, as before the two were told apart.
//! - **Suggestions** tuned for Portuguese ([`suggest`]): missing accents first (`coraçao` →
//!   `coração`, `saude` → `saúde`), then the dictionary's `REP` confusions (`seje` → `seja`), a
//!   missing space, one-letter typos, and a dictionary scan for anything further off.
//! - **Grammar** ([`grammar`]): cheap, reliable rules with Portuguese messages.

pub mod grammar;
mod suggest;
#[cfg(test)]
pub(crate) mod test_dict;

use std::sync::{Arc, RwLock};

use crate::affix::AffixDict;

pub use suggest::suggest;

/// Which of the two Portuguese spellings a run of text follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    /// Brazilian Portuguese, and Portuguese with no region: the Hunspell `pt_BR` dictionary.
    Br,
    /// European Portuguese: the Hunspell `pt_PT` dictionary.
    Pt,
}

impl Variant {
    /// File names of this spelling's Hunspell dictionary.
    fn names(self) -> &'static [&'static str] {
        match self {
            Variant::Br => &["pt_BR", "pt-BR", "pt"],
            Variant::Pt => &["pt_PT", "pt-PT"],
        }
    }

    /// The loaded dictionary: `None` until first use, then the installed one (or an empty one).
    fn slot(self) -> &'static RwLock<Option<Arc<AffixDict>>> {
        static BR: RwLock<Option<Arc<AffixDict>>> = RwLock::new(None);
        static PT: RwLock<Option<Arc<AffixDict>>> = RwLock::new(None);
        match self {
            Variant::Br => &BR,
            Variant::Pt => &PT,
        }
    }
}

/// The Portuguese dictionary: the installed Hunspell `pt_BR` (read on first use), one given with
/// [`install_dictionary`], or an empty one when none is available (Portuguese spelling is off
/// then; see [`has_dictionary`]).
pub fn dictionary() -> Arc<AffixDict> {
    dictionary_for(Variant::Br)
}

/// [`dictionary`] for one spelling. European Portuguese without a `pt_PT` dictionary uses the
/// Brazilian one.
pub fn dictionary_for(variant: Variant) -> Arc<AffixDict> {
    if let Some(d) = variant.slot().read().unwrap_or_else(|e| e.into_inner()).as_ref() {
        return d.clone();
    }
    let loaded = load_installed(variant);
    let mut slot = variant.slot().write().unwrap_or_else(|e| e.into_inner());
    // Another thread (or an install) may have filled it meanwhile: that one wins.
    slot.get_or_insert(loaded).clone()
}

fn load_installed(variant: Variant) -> Arc<AffixDict> {
    let Some((aff, dic)) = crate::hunspell::find(variant.names()) else {
        return match variant {
            Variant::Br => {
                log::info!("Portuguese spelling: no Hunspell pt_BR dictionary installed");
                Arc::new(AffixDict::default())
            }
            Variant::Pt => {
                log::info!("European Portuguese spelling: no Hunspell pt_PT dictionary installed; using pt_BR");
                dictionary_for(Variant::Br)
            }
        };
    };
    Arc::new(crate::hunspell::load(&aff, &dic).unwrap_or_else(|e| {
        log::warn!("Portuguese spelling: {e}");
        AffixDict::default()
    }))
}

/// Use this Portuguese dictionary from now on (Hunspell `.aff` and `.dic` text, decoded to
/// UTF-8), for example one the user picked. On error the current dictionary stays.
pub fn install_dictionary(aff: &str, dic: &str) -> Result<(), String> {
    install_dictionary_for(Variant::Br, aff, dic)
}

/// [`install_dictionary`] for one spelling.
pub fn install_dictionary_for(variant: Variant, aff: &str, dic: &str) -> Result<(), String> {
    let d = AffixDict::from_hunspell(aff, dic, &[])?;
    *variant.slot().write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(d));
    Ok(())
}

/// Forget the loaded dictionaries, so the next check looks for installed ones again (after
/// [`crate::hunspell::set_dictionary_dir`] or a dictionary was installed).
pub fn reload_dictionary() {
    for variant in [Variant::Br, Variant::Pt] {
        *variant.slot().write().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// Is a Portuguese dictionary available (spelling is checked)?
pub fn has_dictionary() -> bool {
    !dictionary().is_empty()
}

/// [`has_dictionary`] for one spelling.
pub fn has_dictionary_for(variant: Variant) -> bool {
    !dictionary_for(variant).is_empty()
}

/// Is `c` a letter of the Latin script (the only one the Portuguese dictionary covers)?
fn is_latin_letter(c: char) -> bool {
    c.is_alphabetic() && (c < '\u{0250}' || ('\u{1E00}'..='\u{1EFF}').contains(&c))
}

/// Is `word` spelled correctly in Portuguese? Numbers, single letters, short words in capitals
/// (acronyms: `USP`, `PT`), words with digits and words in other scripts are accepted, and so
/// is every word when no Portuguese dictionary is installed.
pub fn is_correct(word: &str) -> bool {
    is_correct_with(&dictionary(), word)
}

/// [`is_correct`] in one spelling (`facto` is a word in European Portuguese only).
pub fn is_correct_for(variant: Variant, word: &str) -> bool {
    is_correct_with(&dictionary_for(variant), word)
}

/// Spelling suggestions for a word misspelled in one spelling ([`suggest`] is the Brazilian one).
pub fn suggest_for(variant: Variant, word: &str, max: usize) -> Vec<String> {
    suggest::suggest_in(&dictionary_for(variant), word, max)
}

/// [`is_correct`] against a given dictionary (an empty one accepts everything).
pub fn is_correct_with(d: &AffixDict, word: &str) -> bool {
    if d.is_empty() {
        return true; // no Portuguese dictionary installed: no spelling marks
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
        return true; // other scripts: no dictionary
    }
    if w.contains('-') {
        return w.split('-').all(|p| p.chars().count() <= 1 || known(d, p));
    }
    known(d, w)
}

/// Is one word (no hyphens) in the dictionary or the user dictionary?
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
        assert!(d.len() > 50, "{}", d.len());
        assert!(d.rule_count() >= 5, "the test dictionary's affix rules");
        assert!(!d.rep().is_empty() && !d.map().is_empty() && !d.try_chars().is_empty());
    }

    #[test]
    fn without_a_dictionary_nothing_is_flagged() {
        let none = AffixDict::default();
        for w in ["coraçao", "saude", "xqzv", ""] {
            assert!(is_correct_with(&none, w), "{w}");
        }
        assert!(install_dictionary("FLAG long\n", "1\nkot\n").is_err(), "unsupported files are refused");
    }

    #[test]
    fn portuguese_words_are_correct() {
        test_dict::install();
        for w in [
            "coração",
            "saudade",
            "pássaro",
            "saúde",
            "árvore",
            "escrever",
            "computador",
            "praia",
            "conhecer",
            "Brasília",
            "São",
            "autoestima",
            "frequência",
            "exceção",
            "amanhã",
            "vocês",
            "coragem",
            "parabéns",
            "cantava",
            "cantando",
            "provável",
            "prováveis",
            "descobrir",
            "trabalho",
            "CORAGEM",
            "USP",
            "2026",
            "e-mail",
            "guarda-roupa",
            "ex-marido",
            "vice-presidente",
            "e",
            "não",
        ] {
            assert!(is_correct(w), "{w}");
        }
    }

    #[test]
    fn misspellings_are_flagged() {
        test_dict::install();
        for w in ["coraçao", "saude", "trbalho", "excrever", "prsidente", "frequencia", "amnha", "cantaando"] {
            assert!(!is_correct(w), "{w}");
        }
    }

    #[test]
    fn the_two_spellings_have_their_own_dictionaries() {
        test_dict::install();
        test_dict::install_european();
        assert!(has_dictionary_for(Variant::Br) && has_dictionary_for(Variant::Pt));
        for w in ["facto", "equipa", "autocarro", "económico", "Económico", "FACTO"] {
            assert!(is_correct_for(Variant::Pt, w), "{w}");
        }
        for w in ["facto", "equipa", "autocarro", "económico"] {
            assert!(!is_correct(w) && !is_correct_for(Variant::Br, w), "{w}");
        }
        for w in ["equipe", "ônibus", "econômico"] {
            assert!(is_correct(w) && !is_correct_for(Variant::Pt, w), "{w}");
        }
        // Words both write the same way are in both.
        assert!(is_correct("coração") && is_correct_for(Variant::Pt, "coração"));
        assert_eq!(suggest_for(Variant::Pt, "economico", 3).first().map(String::as_str), Some("económico"));
        assert_eq!(suggest_for(Variant::Br, "economico", 3).first().map(String::as_str), Some("econômico"));
        assert_eq!(Variant::Pt.names().first(), Some(&"pt_PT"));
        assert!(!Variant::Pt.names().contains(&"pt"), "a bare `pt` dictionary is the Brazilian fallback");
    }

    #[test]
    fn forbidden_words_are_rejected() {
        test_dict::install();
        // `frequencia/F` is listed with FORBIDDENWORD: the old spelling, never correct.
        assert!(!is_correct("frequencia") && !is_correct("Frequencia") && !is_correct("FREQUENCIA"));
        assert!(is_correct("frequência"));
    }

    #[test]
    fn user_dictionary_applies() {
        test_dict::install();
        assert!(!is_correct("zxqportuguespalavra"));
        crate::add_word("Zxqportuguespalavra");
        assert!(is_correct("zxqportuguespalavra"));
    }
}

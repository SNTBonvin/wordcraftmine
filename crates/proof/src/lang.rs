//! Proofing languages: which dictionary, grammar rules and hyphenation patterns apply to a run of
//! text. A run's language is its BCP 47 tag (`w:lang` in .docx, `CharProps::lang` in the model),
//! resolved with [`ProofLang::from_tag`].

/// A language WordCraft can proof, or [`ProofLang::Other`] for one it can't.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ProofLang {
    /// English, any region (`en`, `en-US`, `en-GB`…): the public-domain Moby word list, the
    /// English grammar rules and the English hyphenation dictionary and patterns. Also text with
    /// no language at all (WordCraft's default editing language is US English, like Word's).
    #[default]
    En,
    /// Polish (`pl`, `pl-PL`): the installed Hunspell dictionary (when there is one), the Polish grammar rules and the Polish
    /// hyphenation patterns.
    Pl,
    /// Bulgarian (`bg`, `bg-BG`): hyphenation by the Bulgarian syllable rules ([`crate::bg`]).
    /// There is no Bulgarian dictionary or grammar yet, so, as for [`ProofLang::Other`], no
    /// spelling or grammar marks.
    Bg,
    /// Portuguese (`pt`, `pt-BR`, `pt-PT`): the installed Hunspell dictionary (VERO and
    /// friends, when there is one), the Portuguese grammar rules and the Portuguese hyphenation
    /// patterns.
    Pt,
    /// A language without proofing tools here (`fr-FR`, `ja-JP`, `x-none`…). Like Word without
    /// the language's proofing tools: no spelling or grammar marks, so its words are never
    /// flagged as English misspellings. Hyphenation keeps using the English patterns, as before
    /// languages were told apart.
    Other,
}

impl ProofLang {
    /// Every language with a spelling dictionary, in display order.
    pub const PROOFED: [ProofLang; 3] = [ProofLang::En, ProofLang::Pl, ProofLang::Pt];

    /// The proofing language of a BCP 47 tag (`pl-PL`, `en_GB`, `PL`…). Only the primary subtag
    /// counts. No tag (or an empty one) is English, WordCraft's default editing language.
    pub fn from_tag(tag: Option<&str>) -> ProofLang {
        let Some(tag) = tag.map(str::trim).filter(|t| !t.is_empty()) else { return ProofLang::En };
        let primary = tag.split(['-', '_']).next().unwrap_or("");
        if primary.eq_ignore_ascii_case("en") || primary.eq_ignore_ascii_case("eng") {
            ProofLang::En
        } else if primary.eq_ignore_ascii_case("pl") || primary.eq_ignore_ascii_case("pol") {
            ProofLang::Pl
        } else if primary.eq_ignore_ascii_case("bg") || primary.eq_ignore_ascii_case("bul") {
            ProofLang::Bg
        } else if primary.eq_ignore_ascii_case("pt") || primary.eq_ignore_ascii_case("por") {
            ProofLang::Pt
        } else {
            ProofLang::Other
        }
    }

    /// The primary language subtag (`en`, `pl`, `bg`); empty for [`ProofLang::Other`].
    pub fn code(self) -> &'static str {
        match self {
            ProofLang::En => "en",
            ProofLang::Pl => "pl",
            ProofLang::Bg => "bg",
            ProofLang::Pt => "pt",
            ProofLang::Other => "",
        }
    }

    /// Does WordCraft check spelling and grammar in this language?
    pub fn is_proofed(self) -> bool {
        Self::PROOFED.contains(&self)
    }
}

/// Canonical BCP 47 tags, from the shared `craft-proof` crate.
pub use craft_proof::tag::normalize_tag;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_resolve() {
        for t in ["pl", "pl-PL", "PL-pl", "pl_PL", "pl_PL.UTF-8", " pl-PL ", "pol"] {
            assert_eq!(ProofLang::from_tag(Some(t)), ProofLang::Pl, "{t}");
        }
        for t in ["pt", "pt-BR", "PT_br", "pt-BR.UTF-8", " pt-PT ", "por"] {
            assert_eq!(ProofLang::from_tag(Some(t)), ProofLang::Pt, "{t}");
        }
        for t in ["en", "en-US", "en-GB", "EN_au", "en-Latn-US"] {
            assert_eq!(ProofLang::from_tag(Some(t)), ProofLang::En, "{t}");
        }
        for t in ["bg", "bg-BG", "BG_bg", "bg_BG.UTF-8", "bul"] {
            assert_eq!(ProofLang::from_tag(Some(t)), ProofLang::Bg, "{t}");
        }
        for t in ["fr-FR", "de", "x-none", "ja-JP", "plx", "bgx", "e", "-"] {
            assert_eq!(ProofLang::from_tag(Some(t)), ProofLang::Other, "{t}");
        }
        assert_eq!(ProofLang::from_tag(None), ProofLang::En);
        assert_eq!(ProofLang::from_tag(Some("")), ProofLang::En);
        assert_eq!(ProofLang::from_tag(Some("  ")), ProofLang::En);
        assert!(ProofLang::Pl.is_proofed() && !ProofLang::Other.is_proofed());
        // Bulgarian has hyphenation only: no spelling or grammar marks.
        assert!(!ProofLang::Bg.is_proofed());
        assert_eq!(ProofLang::Bg.code(), "bg");
        assert!(ProofLang::Pt.is_proofed() && !ProofLang::Other.is_proofed());
        assert_eq!(ProofLang::Pl.code(), "pl");
        assert_eq!(ProofLang::Pt.code(), "pt");
        assert!(ProofLang::PROOFED.contains(&ProofLang::Pt), "every proofed language is listed");
    }
}

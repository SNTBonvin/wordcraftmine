//! Locale terms: the en-US defaults the CSL 1.0.2 spec appendix defines, limited to what
//! the renderer subset consumes (months in both forms, the label variables our sources
//! carry, and the join/truncation words), plus merging of a style's own en-US overrides.
//!
//! Term text is our own wording of the public spec's appendix (the words are the language's,
//! not creative expression). Only en-US is carried: a style's non-en-US `<locale>` sections
//! are refused with a warning by the parser, never silently swapped.

use std::collections::BTreeMap;

use crate::model::Form;

/// A term's two shapes: singular and plural ("page"/"pages", "p."/"pp.").
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Term {
    pub single: String,
    pub multiple: String,
}

/// Terms keyed by `(name, form)`. Constructed from the en-US defaults; a style's
/// `xml:lang="en-US"` (or lang-less, default-locale) `<term>` overrides replace entries.
#[derive(Clone, Debug, PartialEq)]
pub struct TermSet(BTreeMap<(String, Form), Term>);

impl TermSet {
    /// The spec's en-US terms for the subset.
    pub fn en_us() -> TermSet {
        let mut set = TermSet(BTreeMap::new());
        let months_long = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
        let months_short = ["Jan.", "Feb.", "March", "April", "May", "June", "July", "Aug.", "Sept.", "Oct.", "Nov.", "Dec."];
        for (i, m) in months_long.iter().enumerate() {
            set.set(&format!("month-{:02}", i + 1), Form::Long, same(m));
            set.set(&format!("month-{:02}", i + 1), Form::Short, same(months_short[i]));
        }
        set.set("and", Form::Long, same("and"));
        set.set("and", Form::Symbol, same("&"));
        set.set("et-al", Form::Long, same("et al."));
        set.set("page", Form::Long, Term::new("page", "pages"));
        set.set("page", Form::Short, Term::new("p.", "pp."));
        set.set("volume", Form::Long, Term::new("volume", "volumes"));
        set.set("volume", Form::Short, Term::new("vol.", "vols."));
        set.set("issue", Form::Long, Term::new("number", "numbers"));
        set.set("issue", Form::Short, Term::new("no.", "nos."));
        set.set("editor", Form::Long, Term::new("editor", "editors"));
        set.set("editor", Form::Short, Term::new("ed.", "eds."));
        set.set("editor", Form::Verb, same("edited by"));
        set.set("editor", Form::VerbShort, same("ed. by"));
        set.set("translator", Form::Long, Term::new("translator", "translators"));
        set.set("translator", Form::Short, same("trans."));
        set.set("translator", Form::Verb, same("translated by"));
        set.set("accessed", Form::Long, same("accessed"));
        set.set("retrieved", Form::Long, same("retrieved"));
        set.set("no date", Form::Long, same("no date"));
        set.set("no date", Form::Short, same("n.d."));
        set.set("ibid", Form::Long, same("ibid."));
        set
    }

    /// Look a term up, falling back to the long form when the requested form is missing
    /// (the spec's term fallback).
    pub fn get(&self, name: &str, form: Form) -> Option<&Term> {
        if form != Form::Long
            && let Some(t) = self.0.get(&(name.to_string(), form))
        {
            return Some(t);
        }
        self.0.get(&(name.to_string(), Form::Long))
    }

    /// The text of a term: its `multiple` shape when `plural`, else `single`.
    pub fn text<'a>(&'a self, name: &str, form: Form, plural: bool) -> Option<&'a str> {
        let t = self.get(name, form)?;
        Some(if plural { t.multiple.as_str() } else { t.single.as_str() })
    }

    /// Replace or add an entry (a style's en-US override).
    pub(crate) fn set(&mut self, name: &str, form: Form, term: Term) {
        self.0.insert((name.to_string(), form), term);
    }
}

fn same(s: &str) -> Term {
    Term { single: s.into(), multiple: s.into() }
}

impl Term {
    pub fn new(single: &str, multiple: &str) -> Term {
        Term { single: single.into(), multiple: multiple.into() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_months_and_labels() {
        let t = TermSet::en_us();
        assert_eq!(t.text("month-01", Form::Long, false), Some("January"));
        assert_eq!(t.text("month-09", Form::Short, false), Some("Sept."));
        assert_eq!(t.text("month-12", Form::Long, true), Some("December"));
        assert_eq!(t.text("page", Form::Short, false), Some("p."));
        assert_eq!(t.text("page", Form::Short, true), Some("pp."));
        assert_eq!(t.text("page", Form::Long, false), Some("page"));
        assert_eq!(t.text("and", Form::Symbol, false), Some("&"));
        assert_eq!(t.text("et-al", Form::Long, false), Some("et al."));
        assert_eq!(t.text("editor", Form::Verb, true), Some("edited by"));
        assert_eq!(t.text("no date", Form::Short, false), Some("n.d."));
    }

    #[test]
    fn missing_forms_fall_back_to_long() {
        let t = TermSet::en_us();
        // et-al has no verb form: the long form answers.
        assert_eq!(t.text("et-al", Form::Verb, false), Some("et al."));
        assert_eq!(t.text("made-up-term", Form::Long, false), None);
    }

    #[test]
    fn overrides_replace_entries() {
        let mut t = TermSet::en_us();
        t.set("page", Form::Short, Term::new("S.", "Ss."));
        assert_eq!(t.text("page", Form::Short, true), Some("Ss."));
        // Other entries are untouched.
        assert_eq!(t.text("page", Form::Long, true), Some("pages"));
    }
}

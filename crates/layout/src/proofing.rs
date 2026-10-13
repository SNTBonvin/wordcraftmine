//! Spelling and grammar marks for a whole document, kept apart from layout.
//!
//! Layout never asks the proofing engine anything: adding a word to the dictionary, ignoring one
//! or switching Check Spelling as You Type leaves every shaped paragraph and page raster as it was.
//! [`check_document`] finds the issues of every paragraph a reader sees (body, headers, footers,
//! notes, text boxes), reusing a [`ProofCache`] so a check after an edit only rechecks the
//! paragraphs whose text, formatting or language changed. The editor runs it off the UI thread and
//! draws the result over the pages ([`crate::display::proof_marks`]).
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use wordcraft_doc::{Document, Paragraph, PartKind, Path, StoryRef};

use crate::ProofOptions;
use crate::para::{paragraph_issues, proof_runs};

/// One paragraph's issues and the hash of the text they were found in. The offsets only apply
/// to a paragraph whose text still hashes to `text_hash` ([`text_hash`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParaIssues {
    pub text_hash: u64,
    pub issues: Vec<wordcraft_proof::Issue>,
}

/// The issues of every checked paragraph that has any, by story and path.
pub type DocIssues = HashMap<(StoryRef, Path), Arc<ParaIssues>>;

/// What [`check_document`] keeps between checks: each paragraph's last result and the inputs it
/// came from. Results are dropped when the dictionary or the proofing options change.
#[derive(Default)]
pub struct ProofCache {
    /// Options and user-dictionary generation the cached results were found with.
    env: Option<(ProofOptions, u64)>,
    paras: HashMap<(StoryRef, Path), (u64, Arc<ParaIssues>)>,
}

impl ProofCache {
    pub fn new() -> Self {
        Self::default()
    }
}

/// The hash a paragraph's text has: [`ParaIssues::text_hash`] for its current text.
pub fn text_hash(p: &Paragraph) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    p.text.hash(&mut h);
    h.finish()
}

/// The style sheet's part in each run's proofing language and "do not check" state, hashed once
/// per check instead of resolving every run's styles again for every unchanged paragraph.
fn styles_key(doc: &Document) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let chr = |c: &wordcraft_doc::props::CharProps, h: &mut std::collections::hash_map::DefaultHasher| {
        (&c.lang, c.no_proof, c.hidden, c.link.is_some()).hash(h);
    };
    chr(&doc.styles.default_chr, &mut h);
    for s in &doc.styles.styles {
        (&s.id, s.kind, &s.based_on).hash(&mut h);
        chr(&s.chr, &mut h);
    }
    h.finish()
}

/// Everything a paragraph's issues depend on besides the style sheet: its text, its paragraph
/// style and each run's language, character style, "do not check", hidden, link and deletion.
fn para_key(p: &Paragraph, styles: u64) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (styles, &p.text, &p.props.style).hash(&mut h);
    for (r, c) in p.run_ranges() {
        (r.start, r.end, &c.style, &c.lang, c.no_proof, c.hidden, c.link.is_some(), c.del.is_some()).hash(&mut h);
    }
    h.finish()
}

/// Spelling and grammar issues of every paragraph in `doc` that a reader sees (comments are
/// left out), each word in its run's language, as the Review commands find them; grammar only
/// with `opts.grammar`. Paragraphs unchanged since the last check with the same `cache` (same
/// text, formatting, languages, options and user dictionary) reuse their last result.
pub fn check_document(doc: &Document, opts: &ProofOptions, cache: &mut ProofCache) -> DocIssues {
    let env = (*opts, wordcraft_proof::dictionary_generation());
    if cache.env != Some(env) {
        cache.paras.clear();
        cache.env = Some(env);
    }
    let styles = styles_key(doc);
    let stories =
        std::iter::once(StoryRef::Body).chain(doc.parts.iter().filter(|(_, part)| part.kind != PartKind::Comment).map(|(id, _)| StoryRef::Part(*id)));
    let mut out = DocIssues::new();
    let mut seen: HashMap<(StoryRef, Path), (u64, Arc<ParaIssues>)> = HashMap::new();
    for story in stories {
        for path in doc.para_paths(story) {
            let Some(p) = doc.para(story, &path) else { continue };
            let key = para_key(p, styles);
            let id = (story, path);
            let found = match cache.paras.remove(&id) {
                Some((k, found)) if k == key => found,
                _ => {
                    let runs = proof_runs(doc, p, None);
                    Arc::new(ParaIssues { text_hash: text_hash(p), issues: paragraph_issues(p, &runs, &opts.spelling, opts.grammar) })
                }
            };
            if !found.issues.is_empty() {
                out.insert(id.clone(), found.clone());
            }
            seen.insert(id, (key, found));
        }
    }
    // Paragraphs that are gone are forgotten.
    cache.paras = seen;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::Pos;

    fn spelling(issues: &DocIssues, doc: &Document) -> Vec<String> {
        let mut v: Vec<String> = issues
            .iter()
            .flat_map(|((story, path), found)| {
                let text = doc.para(*story, path).map(|p| p.text.clone()).unwrap_or_default();
                found
                    .issues
                    .iter()
                    .filter(|i| i.kind == wordcraft_proof::IssueKind::Spelling)
                    .filter_map(move |i| text.get(i.start..i.end).map(str::to_string))
                    .collect::<Vec<_>>()
            })
            .collect();
        v.sort();
        v
    }

    #[test]
    fn finds_issues_and_reuses_unchanged_paragraphs() {
        let mut doc = Document::from_text("One zxqtypoone here.\nAnother zxqtypotwo line.");
        let mut cache = ProofCache::new();
        let opts = ProofOptions::default();
        let generation = wordcraft_proof::dictionary_generation();
        let first = check_document(&doc, &opts, &mut cache);
        assert_eq!(spelling(&first, &doc), ["zxqtypoone", "zxqtypotwo"]);
        // Editing the second paragraph keeps the first one's result (the same Arc), unless
        // another test changed the (process-wide) user dictionary meanwhile.
        doc.insert_text(&Pos::body(1, 0), "New ", &Default::default()).unwrap();
        let second = check_document(&doc, &opts, &mut cache);
        let key = (StoryRef::Body, Path::top(0));
        if wordcraft_proof::dictionary_generation() == generation {
            assert!(Arc::ptr_eq(&first[&key], &second[&key]));
        }
        assert_eq!(spelling(&second, &doc), ["zxqtypoone", "zxqtypotwo"]);
        let moved = &second[&(StoryRef::Body, Path::top(1))];
        let text = &doc.para(StoryRef::Body, &Path::top(1)).unwrap().text;
        assert_eq!(moved.text_hash, text_hash(doc.para(StoryRef::Body, &Path::top(1)).unwrap()));
        assert_eq!(&text[moved.issues[0].start..moved.issues[0].end], "zxqtypotwo");
    }

    #[test]
    fn language_and_no_proof_changes_recheck_without_text_edits() {
        let mut doc = Document::from_text("zxqtestunknownword");
        let mut cache = ProofCache::new();
        let opts = ProofOptions::default();
        assert_eq!(check_document(&doc, &opts, &mut cache).len(), 1);
        // The document default language changes, the text and run formatting don't.
        doc.styles.default_chr.lang = Some("fr-FR".into());
        assert!(check_document(&doc, &opts, &mut cache).is_empty(), "no French dictionary: no marks");
        doc.styles.default_chr.lang = Some("en-US".into());
        assert_eq!(check_document(&doc, &opts, &mut cache).len(), 1);
        let end = doc.para(StoryRef::Body, &Path::top(0)).unwrap().text.len();
        doc.format_range(&Pos::body(0, 0), &Pos::body(0, end), &|c| c.no_proof = Some(true)).unwrap();
        assert!(check_document(&doc, &opts, &mut cache).is_empty());
    }

    #[test]
    fn user_dictionary_changes_recheck() {
        let word = "zxqcachedictionaryword";
        let doc = Document::from_text(&format!("The {word} here."));
        let mut cache = ProofCache::new();
        let opts = ProofOptions::default();
        assert_eq!(spelling(&check_document(&doc, &opts, &mut cache), &doc), [word]);
        wordcraft_proof::add_word(word);
        assert!(spelling(&check_document(&doc, &opts, &mut cache), &doc).is_empty());
        wordcraft_proof::remove_word(word);
    }

    #[test]
    fn grammar_follows_the_option() {
        let doc = Document::from_text("This is is a test.");
        let mut cache = ProofCache::new();
        assert!(!check_document(&doc, &ProofOptions::default(), &mut cache).is_empty());
        assert!(check_document(&doc, &ProofOptions { grammar: false, ..Default::default() }, &mut cache).is_empty());
    }
}

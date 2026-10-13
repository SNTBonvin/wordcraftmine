//! Check Spelling as You Type, off the UI thread.
//!
//! A worker thread checks a snapshot of the document ([`check_document`], which rechecks only
//! the paragraphs that changed) and computes spelling suggestions; the editor draws the result
//! over the pages ([`Proofing::paint`]). Typing, adding a word to the dictionary or ignoring
//! one never waits for a check, and never re-lays-out or re-renders a page. Each paragraph's
//! issues keep the text they were found in, so until the next check comes in, marks stay on
//! unchanged paragraphs (found by their text when an edit moved them) and move with the text in
//! the paragraph being typed in; only a mark the edit touches disappears. The browser has no
//! threads: there the same check runs in the frame, still only for changed paragraphs.
use std::collections::HashMap;
use std::sync::Arc;

use egui::{Color32, Painter, Pos2, Rect, Stroke, pos2};
use serde_json::{Value, json};
use wordcraft_doc::{Document, Pos};
use wordcraft_engine::Session;
use wordcraft_layout::display::ProofMark;
use wordcraft_layout::proofing::{DocIssues, ParaIssues, ProofCache, check_document, text_hash};
use wordcraft_layout::{DocLayout, ProofOptions};
use wordcraft_proof::{IssueKind, ProofLang};

/// What a check's result belongs to: document, revision, user-dictionary generation, options.
type Key = (u64, u64, u64, ProofOptions);

#[cfg(not(target_arch = "wasm32"))]
enum Request {
    Check(Box<Document>, Key, egui::Context),
    Suggest(String, ProofLang, egui::Context),
}

#[cfg(not(target_arch = "wasm32"))]
enum Response {
    Checked(Key, DocIssues),
    Suggested(String, ProofLang, Vec<String>),
}

/// Most suggestions the context menu shows.
const SUGGESTIONS: usize = 6;

/// The editor's spelling and grammar marks and the worker that finds them.
pub struct Proofing {
    #[cfg(not(target_arch = "wasm32"))]
    worker: Option<(std::sync::mpsc::SyncSender<Request>, std::sync::mpsc::Receiver<Response>)>,
    #[cfg(target_arch = "wasm32")]
    cache: ProofCache,
    /// A check is with the worker (one at a time: a newer revision waits for it).
    busy: bool,
    /// The key of the last check asked for, and of the one `issues` came from.
    asked: Option<Key>,
    ready: Option<Key>,
    issues: DocIssues,
    /// `issues` by their paragraph's text hash: a paragraph an edit moved (Enter above it) keeps
    /// its marks.
    by_hash: HashMap<u64, Arc<ParaIssues>>,
    /// Bumped whenever `issues` changes: the page marks below are found again.
    generation: u64,
    /// Marks of the pages drawn so far, for a layout (held, so it is never a different one at
    /// the same address) and an issues generation.
    marks: Option<(Arc<DocLayout>, u64, HashMap<usize, Vec<ProofMark>>)>,
    /// Suggestions for a misspelled word in its language: asked for, and found.
    suggest_asked: Option<(String, ProofLang)>,
    suggest_unsent: bool,
    suggestions: Option<(String, ProofLang, Vec<String>)>,
}

impl Default for Proofing {
    fn default() -> Self {
        Proofing {
            #[cfg(not(target_arch = "wasm32"))]
            worker: spawn_worker(),
            #[cfg(target_arch = "wasm32")]
            cache: ProofCache::new(),
            busy: false,
            asked: None,
            ready: None,
            issues: DocIssues::new(),
            by_hash: HashMap::new(),
            generation: 0,
            marks: None,
            suggest_asked: None,
            suggest_unsent: false,
            suggestions: None,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn_worker() -> Option<(std::sync::mpsc::SyncSender<Request>, std::sync::mpsc::Receiver<Response>)> {
    // Room for a check and a suggestion request; the editor never blocks on a full queue.
    let (tx, rx) = std::sync::mpsc::sync_channel::<Request>(2);
    let (out, inbox) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name("wordcraft-proofing".into()).spawn(move || {
        let mut cache = ProofCache::new();
        let mut document = None;
        while let Ok(request) = rx.recv() {
            let (response, ctx) = match request {
                Request::Check(doc, key, ctx) => {
                    // A different document shares nothing with the last one.
                    if document != Some(key.0) {
                        cache = ProofCache::new();
                        document = Some(key.0);
                    }
                    (Response::Checked(key, check_document(&doc, &key.3, &mut cache)), ctx)
                }
                Request::Suggest(word, lang, ctx) => {
                    let found = wordcraft_proof::suggest_in(&word, SUGGESTIONS, lang);
                    (Response::Suggested(word, lang, found), ctx)
                }
            };
            if out.send(response).is_err() {
                break;
            }
            ctx.request_repaint();
        }
    });
    match spawned {
        Ok(_) => Some((tx, inbox)),
        Err(e) => {
            log::warn!("spelling checker thread: {e}");
            None
        }
    }
}

fn key_of(session: &Session) -> Key {
    (session.document_id(), session.rev(), wordcraft_proof::dictionary_generation(), session.prefs.proof_options())
}

impl Proofing {
    /// Once a frame: take the worker's answers and ask for a check of the current document
    /// when it changed. Never waits for the worker.
    pub fn poll(&mut self, session: &Session, ctx: &egui::Context) {
        let key = key_of(session);
        #[cfg(not(target_arch = "wasm32"))]
        {
            let responses: Vec<Response> = self.worker.as_ref().map(|(_, rx)| rx.try_iter().collect()).unwrap_or_default();
            for response in responses {
                match response {
                    Response::Checked(found, issues) => {
                        self.busy = false;
                        // Marks of an earlier revision of this document still apply to the
                        // paragraphs whose text hasn't changed since; another document's never do.
                        if found.0 == key.0 && session.view.proofing {
                            self.set_issues(issues);
                            self.ready = Some(found);
                        }
                    }
                    Response::Suggested(word, lang, found) => {
                        if self.suggest_asked.as_ref().is_some_and(|(w, l)| *w == word && *l == lang) {
                            self.suggest_asked = None;
                            self.suggestions = Some((word, lang, found));
                        }
                    }
                }
            }
            if self.suggest_unsent
                && let Some((tx, _)) = &self.worker
                && let Some((word, lang)) = &self.suggest_asked
                && tx.try_send(Request::Suggest(word.clone(), *lang, ctx.clone())).is_ok()
            {
                self.suggest_unsent = false;
            }
        }
        if !session.view.proofing {
            if self.ready.is_some() || !self.issues.is_empty() {
                self.set_issues(DocIssues::new());
            }
            self.ready = None;
            self.asked = None;
            return;
        }
        if self.asked != Some(key) && !self.busy {
            #[cfg(not(target_arch = "wasm32"))]
            if let Some((tx, _)) = &self.worker {
                // One snapshot per check: while the worker is busy, newer edits wait for it and
                // only the latest revision is copied.
                match tx.try_send(Request::Check(Box::new(session.doc.clone()), key, ctx.clone())) {
                    Ok(()) => {
                        self.asked = Some(key);
                        self.busy = true;
                    }
                    Err(_) => ctx.request_repaint_after(std::time::Duration::from_millis(50)),
                }
            }
            #[cfg(target_arch = "wasm32")]
            {
                let issues = check_document(&session.doc, &key.3, &mut self.cache);
                self.set_issues(issues);
                self.asked = Some(key);
                self.ready = Some(key);
            }
        }
    }

    /// Whether the marks are those of the document as it is now (not of an earlier revision).
    #[cfg(test)]
    pub fn is_current(&self, session: &Session) -> bool {
        self.ready == Some(key_of(session))
    }

    fn set_issues(&mut self, issues: DocIssues) {
        self.by_hash = issues.values().map(|f| (f.text_hash, f.clone())).collect();
        self.issues = issues;
        self.generation = self.generation.wrapping_add(1);
    }

    /// A paragraph's issues as marks, (start, end, is grammar), for its text now: as checked,
    /// or found by its text when an edit moved it, or carried across the edit being typed
    /// ([`ParaIssues::remapped`]).
    fn marks_of(
        issues: &DocIssues,
        by_hash: &HashMap<u64, Arc<ParaIssues>>,
        doc: &Document,
        story: wordcraft_doc::StoryRef,
        path: &wordcraft_doc::Path,
    ) -> Vec<(usize, usize, bool)> {
        let Some(p) = doc.para(story, path) else { return Vec::new() };
        let hash = text_hash(p);
        let here = issues.get(&(story, path.clone()));
        let mark = |i: &wordcraft_proof::Issue| (i.start, i.end, i.kind == IssueKind::Grammar);
        if let Some(found) = here.filter(|f| f.text_hash == hash).or_else(|| by_hash.get(&hash)) {
            return found.issues.iter().map(mark).collect();
        }
        here.map(|f| f.remapped(&p.text).iter().map(mark).collect()).unwrap_or_default()
    }

    /// Draw page `index`'s marks over it: `rect` is where the page is on screen and `scale` its
    /// screen points per page point. `dim_body` dims the body's marks (a header or footer is
    /// being edited), and the header's and footer's otherwise, like the text under them.
    #[allow(clippy::too_many_arguments)]
    pub fn paint(
        &mut self,
        painter: &Painter,
        doc: &Document,
        layout: &Arc<DocLayout>,
        index: usize,
        rect: Rect,
        scale: f32,
        dim_body: bool,
        ink: &dyn Fn(wordcraft_doc::Rgb) -> wordcraft_doc::Rgb,
    ) {
        if self.issues.is_empty() {
            return;
        }
        let Some(page) = layout.pages.get(index) else { return };
        let generation = self.generation;
        if self.marks.as_ref().is_none_or(|(l, g, _)| !Arc::ptr_eq(l, layout) || *g != generation) {
            self.marks = Some((layout.clone(), generation, HashMap::new()));
        }
        let Some((_, _, pages)) = self.marks.as_mut() else { return };
        let (issues, by_hash) = (&self.issues, &self.by_hash);
        let marks = pages
            .entry(index)
            .or_insert_with(|| wordcraft_layout::display::proof_marks(doc, page, &|story, path| Self::marks_of(issues, by_hash, doc, story, path)));
        let color = |grammar: bool, dim: bool| {
            let wordcraft_doc::Rgb(r, g, b) = ink(if grammar { wordcraft_doc::Rgb(0x2B, 0x57, 0xC0) } else { wordcraft_doc::Rgb(0xE0, 0x24, 0x24) });
            Color32::from_rgb(r, g, b).gamma_multiply(if dim { 0.45 } else { 1.0 })
        };
        let at = |(x, y): (f32, f32)| pos2(rect.min.x + x * scale, rect.min.y + y * scale);
        for m in marks.iter() {
            let (a, b) = (at(m.a), at(m.b));
            if !painter.clip_rect().expand(8.0).intersects(Rect::from_two_pos(a, b)) {
                continue;
            }
            let dim = m.header_footer != dim_body;
            mark(painter, a, b, m.grammar, scale, color(m.grammar, dim));
        }
    }

    /// The issue at `pos` as `review.suggestions` gives it (`start`, `end`, `text`, `kind`,
    /// `message`, `suggestions`, `lang`), from the last check: nothing is checked here. A
    /// misspelling's suggestions are asked of the worker and come in a later frame
    /// ([`Proofing::fill_suggestions`]).
    pub fn issue_at(&mut self, session: &Session, pos: &Pos) -> Option<Value> {
        let p = session.doc.para_at(pos)?;
        let found = self.issues.get(&(pos.story, pos.path.clone())).filter(|f| f.text_hash == text_hash(p))?;
        let issue = found.issues.iter().find(|i| i.start <= pos.off && pos.off <= i.end)?.clone();
        let word = p.text.get(issue.start..issue.end)?.to_string();
        let spelling = issue.kind == IssueKind::Spelling;
        let suggestions = if spelling { self.suggestions_for(&word, issue.lang) } else { issue.suggestions.clone() };
        Some(json!({
            "start": Pos { off: issue.start, ..pos.clone() },
            "end": Pos { off: issue.end, ..pos.clone() },
            "text": word,
            "kind": if spelling { "spelling" } else { "grammar" },
            "message": issue.message,
            "suggestions": suggestions,
            "lang": issue.lang.code(),
        }))
    }

    /// The suggestions found for `word`, or none yet (asking the worker for them).
    fn suggestions_for(&mut self, word: &str, lang: ProofLang) -> Vec<String> {
        if let Some((w, l, found)) = &self.suggestions
            && w == word
            && *l == lang
        {
            return found.clone();
        }
        #[cfg(not(target_arch = "wasm32"))]
        if self.worker.is_some() {
            if self.suggest_asked.as_ref().is_none_or(|(w, l)| w != word || *l != lang) {
                self.suggest_asked = Some((word.to_string(), lang));
                self.suggest_unsent = true;
            }
            return Vec::new();
        }
        let found = wordcraft_proof::suggest_in(word, SUGGESTIONS, lang);
        self.suggestions = Some((word.to_string(), lang, found.clone()));
        found
    }

    /// Whether suggestions for the open context menu's word are still being found.
    pub fn suggesting(&self, issue: &Value) -> bool {
        self.suggest_asked.as_ref().is_some_and(|(w, _)| issue.get("text").and_then(Value::as_str) == Some(w.as_str()))
    }

    /// Put the suggestions that came in for the context menu's misspelling into it.
    pub fn fill_suggestions(&self, issue: &mut Option<Value>) {
        let Some(issue) = issue else { return };
        if let Some((word, lang, found)) = &self.suggestions
            && issue.get("kind").and_then(Value::as_str) == Some("spelling")
            && issue.get("text").and_then(Value::as_str) == Some(word.as_str())
            && issue.get("lang").and_then(Value::as_str) == Some(lang.code())
        {
            issue["suggestions"] = json!(found);
        }
    }
}

/// A spelling mark (a wave) or a grammar mark (a double line) from `a` to `b` on screen, sized
/// as the page renderer draws a 0.8 pt underline of that kind at `scale` screen points per point.
fn mark(painter: &Painter, a: Pos2, b: Pos2, grammar: bool, scale: f32, color: Color32) {
    let d = b - a;
    let len = d.length();
    if !len.is_finite() || len < 1.0 {
        return;
    }
    let along = d / len;
    let across = egui::vec2(-along.y, along.x);
    if grammar {
        let off = (0.72 * scale).max(1.0);
        let stroke = Stroke::new((0.48 * scale).max(0.75), color);
        for k in [-off, off] {
            painter.line_segment([a + across * k, b + across * k], stroke);
        }
        return;
    }
    // The renderer's wave: half-waves `2·amp` long whose curve peaks at `amp / 2`.
    let amp = (0.96 * scale).max(1.0);
    let half = amp * 2.0;
    let steps = ((len / half) * 4.0).ceil().clamp(2.0, 4000.0) as usize;
    let points: Vec<Pos2> = (0..=steps)
        .map(|k| {
            let t = len * k as f32 / steps as f32;
            a + along * t - across * (std::f32::consts::PI * t / half).sin() * (amp / 2.0)
        })
        .collect();
    painter.add(egui::Shape::line(points, Stroke::new((0.64 * scale).max(0.75), color)));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Poll until the worker's answer for the current document is in.
    fn settle(p: &mut Proofing, s: &Session, ctx: &egui::Context) {
        for _ in 0..1000 {
            p.poll(s, ctx);
            if p.is_current(s) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        panic!("the spelling checker didn't answer");
    }

    /// The words with a spelling mark, as the body's paragraphs are now.
    fn spelling(p: &Proofing, s: &Session) -> Vec<String> {
        let story = wordcraft_doc::StoryRef::Body;
        let mut v: Vec<String> = s
            .doc
            .para_paths(story)
            .iter()
            .flat_map(|path| {
                Proofing::marks_of(&p.issues, &p.by_hash, &s.doc, story, path)
                    .into_iter()
                    .filter(|m| !m.2)
                    .filter_map(|(a, b, _)| s.doc.para(story, path).and_then(|q| q.text.get(a..b)).map(str::to_string))
                    .collect::<Vec<_>>()
            })
            .collect();
        v.sort();
        v
    }

    #[test]
    fn checks_in_the_background_without_touching_the_layout() {
        let mut s = Session::new(Document::from_text("This is a zxqproofingmistake."));
        let before = s.layout();
        let ctx = egui::Context::default();
        let mut p = Proofing::default();
        settle(&mut p, &s, &ctx);
        assert_eq!(spelling(&p, &s), ["zxqproofingmistake"]);
        assert!(Arc::ptr_eq(&before, &s.layout()));
    }

    /// Typing never blanks marks while the next check runs: the edited paragraph's marks move
    /// with its text, and paragraphs an Enter moved keep theirs.
    #[test]
    fn marks_stay_while_typing_until_the_next_check() {
        let mut s = Session::new(Document::from_text("One zxqfirstmistake.\nTwo zxqsecondmistake."));
        let ctx = egui::Context::default();
        let mut p = Proofing::default();
        settle(&mut p, &s, &ctx);
        let both = ["zxqfirstmistake", "zxqsecondmistake"];
        s.sel = wordcraft_engine::Selection::caret(Pos::body(1, 0));
        s.run("text.insert", &json!({"text": "Then "})).unwrap();
        assert!(!p.is_current(&s));
        assert_eq!(spelling(&p, &s), both, "before the worker answers");
        settle(&mut p, &s, &ctx);
        assert_eq!(spelling(&p, &s), both);
        // Enter at the start of the first paragraph moves both paragraphs down a path.
        s.sel = wordcraft_engine::Selection::caret(Pos::body(0, 0));
        s.run("text.newParagraph", &json!({})).unwrap();
        assert!(!p.is_current(&s));
        assert_eq!(spelling(&p, &s), both, "moved paragraphs keep their marks");
    }

    #[test]
    fn dictionary_and_options_changes_recheck() {
        let word = "zxqbackgroundaddedword";
        let mut s = Session::new(Document::from_text(&format!("The {word} is is here.")));
        let ctx = egui::Context::default();
        let mut p = Proofing::default();
        settle(&mut p, &s, &ctx);
        assert_eq!(spelling(&p, &s), [word]);
        s.run("review.addToDictionary", &json!({"word": word})).unwrap();
        settle(&mut p, &s, &ctx);
        assert!(spelling(&p, &s).is_empty());
        wordcraft_proof::remove_word(word);
        let grammar = |p: &Proofing| p.issues.values().flat_map(|f| f.issues.iter()).filter(|i| i.kind == IssueKind::Grammar).count();
        settle(&mut p, &s, &ctx);
        assert!(grammar(&p) > 0, "the repeated word");
        s.prefs.mark_grammar = false;
        settle(&mut p, &s, &ctx);
        assert_eq!(grammar(&p), 0);
    }

    #[test]
    fn context_menu_issue_comes_from_the_cache_with_suggestions_later() {
        let mut s = Session::new(Document::from_text("The wrold is round."));
        let ctx = egui::Context::default();
        let mut p = Proofing::default();
        settle(&mut p, &s, &ctx);
        let at = Pos::body(0, 6);
        let mut issue = p.issue_at(&s, &at);
        let found = issue.clone().unwrap();
        assert_eq!(found["text"], "wrold");
        assert_eq!(found["kind"], "spelling");
        for _ in 0..1000 {
            p.poll(&s, &ctx);
            p.fill_suggestions(&mut issue);
            if !p.suggesting(&found) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        p.fill_suggestions(&mut issue);
        let suggestions = issue.unwrap()["suggestions"].clone();
        assert!(suggestions.as_array().unwrap().iter().any(|v| v == "world"), "{suggestions}");
        // Turning Check Spelling as You Type off drops the marks.
        s.run("review.proofing", &json!({"value": false})).unwrap();
        p.poll(&s, &ctx);
        assert!(p.issue_at(&s, &at).is_none());
    }
}

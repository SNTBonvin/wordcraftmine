//! Transient AI review overlays. Never modify saved content or the user's Track Changes setting.
use crate::{WordApp, theme::Tokens};
use egui::{Color32, Painter, Rect, Stroke, Ui, pos2, vec2};
use serde_json::{Value, json};
use std::ops::Range;
use wordcraft_doc::{Document, Pos, StoryRef};
use wordcraft_engine::Selection;

const MAX_TEXT: usize = 128_000;
const MAX_TOKENS: usize = 1024;
const MAX_MARKS: usize = 128;

#[derive(Clone, Debug)]
pub struct Mark {
    pub start: Pos,
    pub end: Pos,
    pub removed: String,
    pub added: String,
    pub formatting: bool,
    pub summary: bool,
}
#[derive(Clone, Default)]
pub struct Review {
    baseline: Option<Document>,
    after: Option<Document>,
    pub marks: Vec<Mark>,
    pub label: String,
    pub revision: u64,
    pub document: u64,
    pub changed_at: f64,
    new_turn: bool,
    pub truncated: bool,
}
impl Review {
    pub fn begin_turn(&mut self) {
        self.new_turn = true;
    }
    pub fn record(&mut self, before: &Document, after: &Document, document: u64, revision: u64, selection: &Selection, label: &str) {
        if before == after {
            return;
        }
        if self.new_turn || self.document != document || self.after.as_ref() != Some(before) {
            self.baseline = Some(before.clone());
        }
        self.new_turn = false;
        self.document = document;
        self.revision = revision;
        self.label = super::clip(label, 160);
        self.changed_at = crate::now_ms();
        self.after = Some(after.clone());
        if let Some(base) = &self.baseline {
            let (marks, truncated) = compare(base, after, selection);
            self.marks = marks;
            self.truncated = truncated;
        }
    }
    pub fn refresh_after_undo(&mut self, after: &Document, revision: u64, selection: &Selection) {
        if let Some(base) = &self.baseline {
            if base == after {
                *self = Self::default();
                return;
            }
            let (marks, truncated) = compare(base, after, selection);
            self.marks = marks;
            self.truncated = truncated;
            self.after = Some(after.clone());
            self.revision = revision;
            self.changed_at = crate::now_ms();
        }
    }
    pub fn restore_revision(&mut self, doc: &Document, revision: u64) {
        if self.after.as_ref() == Some(doc) {
            self.revision = revision;
        }
    }
    pub fn inspect(&self, revision: u64) -> Value {
        json!({"count":self.marks.len(),"highlighted":revision==self.revision&&!self.marks.is_empty(),"truncated":self.truncated,
            "items":self.marks.iter().map(|m|json!({"start":m.start,"end":m.end,"removed":m.removed,"added":m.added,"formatting":m.formatting,"summary":m.summary})).collect::<Vec<_>>()})
    }
}
struct Span {
    range: Range<usize>,
    pos: Pos,
}
struct TextMap {
    text: String,
    spans: Vec<Span>,
    truncated: bool,
}
impl TextMap {
    fn new(doc: &Document, story: StoryRef) -> Self {
        let mut text = String::new();
        let mut spans = Vec::new();
        let mut truncated = false;
        for path in doc.para_paths(story).into_iter().take(4096) {
            let Some(para) = doc.para(story, &path) else { continue };
            if text.len().saturating_add(para.len()).saturating_add(1) > MAX_TEXT {
                truncated = true;
                break;
            }
            let value = para.plain_text();
            if text.len().saturating_add(value.len()).saturating_add(1) > MAX_TEXT {
                truncated = true;
                break;
            }
            let start = text.len();
            text.push_str(&value);
            spans.push(Span { range: start..text.len(), pos: Pos::new(story, path, 0) });
            text.push('\n');
        }
        if doc.para_paths(story).len() > 4096 {
            truncated = true;
        }
        Self { text, spans, truncated }
    }
    fn at(&self, offset: usize) -> Option<Pos> {
        let span = self.spans.iter().find(|s| offset <= s.range.end).or_else(|| self.spans.last())?;
        let mut pos = span.pos.clone();
        pos.off = offset.saturating_sub(span.range.start).min(span.range.len());
        Some(pos)
    }
}
#[derive(Debug, PartialEq)]
struct Delta {
    old: Range<usize>,
    new: Range<usize>,
}
fn deltas(old: &str, new: &str) -> Vec<Delta> {
    let mut prefix = 0;
    for (a, b) in old.chars().zip(new.chars()) {
        if a != b {
            break;
        }
        prefix += a.len_utf8();
    }
    let mut suffix = 0;
    for (a, b) in old.get(prefix..).unwrap_or("").chars().rev().zip(new.get(prefix..).unwrap_or("").chars().rev()) {
        if a != b {
            break;
        }
        suffix += a.len_utf8();
    }
    let old_end = old.len().saturating_sub(suffix);
    let new_end = new.len().saturating_sub(suffix);
    if prefix == old_end && prefix == new_end {
        return Vec::new();
    }
    let a = old.get(prefix..old_end).unwrap_or("");
    let b = new.get(prefix..new_end).unwrap_or("");
    let aa: Vec<_> = a.split_inclusive(char::is_whitespace).take(MAX_TOKENS + 1).collect();
    let bb: Vec<_> = b.split_inclusive(char::is_whitespace).take(MAX_TOKENS + 1).collect();
    if aa.len() > MAX_TOKENS || bb.len() > MAX_TOKENS {
        return vec![Delta { old: prefix..old_end, new: prefix..new_end }];
    }
    let cols = bb.len().saturating_add(1);
    let rows = aa.len().saturating_add(1);
    let mut grid = vec![0u16; rows.saturating_mul(cols)];
    let get = |grid: &[u16], i: usize, j: usize| grid.get(i.saturating_mul(cols).saturating_add(j)).copied().unwrap_or(0);
    for i in (0..aa.len()).rev() {
        for j in (0..bb.len()).rev() {
            let value =
                if aa.get(i) == bb.get(j) { get(&grid, i + 1, j + 1).saturating_add(1) } else { get(&grid, i + 1, j).max(get(&grid, i, j + 1)) };
            if let Some(cell) = grid.get_mut(i.saturating_mul(cols).saturating_add(j)) {
                *cell = value;
            }
        }
    }
    let (mut i, mut j, mut x, mut y) = (0usize, 0usize, prefix, prefix);
    let mut out = Vec::new();
    let mut pending: Option<Delta> = None;
    while i < aa.len() || j < bb.len() {
        if i < aa.len() && j < bb.len() && aa.get(i) == bb.get(j) {
            if let Some(delta) = pending.take() {
                out.push(delta);
            }
            x = x.saturating_add(aa.get(i).map_or(0, |s| s.len()));
            y = y.saturating_add(bb.get(j).map_or(0, |s| s.len()));
            i += 1;
            j += 1;
        } else {
            let delta = pending.get_or_insert(Delta { old: x..x, new: y..y });
            if i < aa.len() && (j == bb.len() || get(&grid, i + 1, j) >= get(&grid, i, j + 1)) {
                x = x.saturating_add(aa.get(i).map_or(0, |s| s.len()));
                i += 1;
                delta.old.end = x;
            } else {
                y = y.saturating_add(bb.get(j).map_or(0, |s| s.len()));
                j += 1;
                delta.new.end = y;
            }
        }
    }
    if let Some(delta) = pending {
        out.push(delta);
    }
    out
}
fn compare(before: &Document, after: &Document, selection: &Selection) -> (Vec<Mark>, bool) {
    let mut stories = vec![StoryRef::Body];
    stories.extend(before.parts.keys().chain(after.parts.keys()).take(256).map(|id| StoryRef::Part(*id)));
    stories.sort();
    stories.dedup();
    let mut marks = Vec::new();
    let mut truncated = false;
    for story in stories.into_iter().take(128) {
        let a = TextMap::new(before, story);
        let b = TextMap::new(after, story);
        truncated |= a.truncated || b.truncated;
        for delta in deltas(&a.text, &b.text) {
            if marks.len() >= MAX_MARKS {
                truncated = true;
                break;
            }
            let start = b.at(delta.new.start).unwrap_or_else(|| selection.focus.clone());
            let end = b.at(delta.new.end).unwrap_or_else(|| start.clone());
            marks.push(Mark {
                start,
                end,
                removed: super::clip(a.text.get(delta.old).unwrap_or(""), 4000),
                added: super::clip(b.text.get(delta.new).unwrap_or(""), 4000),
                formatting: false,
                summary: false,
            });
        }
        if a.text == b.text {
            for span in b.spans.iter().take(4096) {
                if before.para(story, &span.pos.path) != after.para(story, &span.pos.path) {
                    if marks.len() >= MAX_MARKS {
                        truncated = true;
                        break;
                    }
                    let mut end = span.pos.clone();
                    end.off = span.range.len();
                    marks.push(Mark { start: span.pos.clone(), end, removed: String::new(), added: String::new(), formatting: true, summary: false });
                }
            }
        }
    }
    if marks.is_empty() && before != after {
        let (start, end) = selection.ordered();
        marks.push(Mark { start, end, removed: String::new(), added: String::new(), formatting: !truncated, summary: truncated });
    }
    (marks, truncated)
}

pub fn paint(app: &WordApp, ui: &Ui, painter: &Painter, layout: &wordcraft_layout::DocLayout, pages: &[Rect], scale: f32) {
    let review = &app.ollama.review;
    let t = Tokens::get(ui.ctx());
    let now = crate::now_ms();
    let valid = review.document == app.session.document_id() && review.revision == app.session.rev();
    let busy = app.ollama.busy && app.ollama.allow_edits && app.ollama.document == Some(app.session.document_id());
    let recent = valid && !review.marks.is_empty() && (now - review.changed_at) < 3500.0;
    let animated = app.animations_on() && (busy || recent);
    let pulse = if animated { ((now / 360.0).sin() as f32 + 1.0) * 0.5 } else { 0.0 };
    let purple = Color32::from_rgb(130, 93, 222);
    if valid {
        for (index, mark) in review.marks.iter().enumerate() {
            let color = if mark.formatting || mark.summary { purple } else { t.green };
            for (pi, r) in layout.selection_rects(&app.session.doc, &mark.start, &mark.end, app.session.page_hint) {
                if let Some(page) = pages.get(pi) {
                    let s = layout.pages.get(pi).map_or(scale, |p| crate::canvas::page_screen_scale(*page, p, scale));
                    let rect = Rect::from_min_size(pos2(page.min.x + r.x * s, page.min.y + r.y * s), vec2(r.w * s, r.h * s));
                    painter.rect_filled(rect, 2.0, color.gamma_multiply(0.15 + 0.10 * pulse));
                    painter.hline(rect.x_range(), rect.max.y, Stroke::new(1.2, color.gamma_multiply(0.8)));
                    ui.interact(rect, ui.id().with(("ai-change", index, pi)), egui::Sense::hover()).on_hover_text(if mark.formatting {
                        "Formatting changed by AI"
                    } else {
                        "Added by AI · review changes in chat"
                    });
                }
            }
            if (mark.start == mark.end || !mark.removed.is_empty())
                && let Some(c) = layout.caret_on(&mark.start, app.session.page_hint)
                && let Some(page) = pages.get(c.page)
            {
                let s = layout.pages.get(c.page).map_or(scale, |p| crate::canvas::page_screen_scale(*page, p, scale));
                let x = page.min.x + c.x * s;
                let y = page.min.y + c.top * s;
                let rect = Rect::from_min_size(pos2(x - 3.0, y), vec2(6.0, (c.height * s).max(10.0)));
                let color = if mark.removed.is_empty() { purple } else { t.red };
                painter.line_segment([rect.left_top(), rect.left_bottom()], Stroke::new(3.0, color));
                ui.interact(rect.expand(4.0), ui.id().with(("ai-removed", index)), egui::Sense::hover()).on_hover_text(if mark.removed.is_empty() {
                    if mark.summary { "AI changed the document outside the review preview".into() } else { "AI changed formatting or layout".into() }
                } else {
                    format!("Removed by AI:\n{}", mark.removed)
                });
            }
        }
    }
    if busy || recent {
        let pi = layout.caret_on(&app.session.sel.focus, app.session.page_hint).map_or(0, |c| c.page);
        if let Some(page) = pages.get(pi) {
            painter.rect_stroke(
                page.shrink(2.0),
                3.0,
                Stroke::new(2.0 + 1.0 * pulse, purple.gamma_multiply(0.35 + 0.35 * pulse)),
                egui::StrokeKind::Inside,
            );
            let label = if busy {
                match app.ollama.phase.as_str() {
                    "Editing document" => "AI editing…",
                    "Reading document" => "AI reading document…",
                    "Researching" => "AI researching…",
                    _ => "AI thinking…",
                }
            } else {
                "AI changes highlighted"
            };
            let rect = Rect::from_min_size(page.min + vec2(10.0, 6.0), vec2(178.0, 24.0));
            painter.rect_filled(rect, 6.0, t.panel);
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(11.0), purple);
        }
    }
    if animated {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_diff_keeps_unchanged_words() {
        let result = deltas("Café old middle red end", "Café new middle blue end");
        assert_eq!(result.len(), 2);
        assert_eq!(result.first().and_then(|d| "Café new middle blue end".get(d.new.clone())), Some("new "));
    }
    #[test]
    fn insertion_deletion_and_noop() {
        assert!(deltas("same", "same").is_empty());
        let d = deltas("old", "new");
        assert_eq!(d.len(), 1);
        assert_eq!(d.first().map(|d| d.old.clone()), Some(0..3));
        assert_eq!(deltas("x", "x\ny").first().map(|d| d.new.clone()), Some(1..3));
    }
    #[test]
    fn hostile_size_is_bounded() {
        let a = "a ".repeat(1200);
        let b = "b ".repeat(1200);
        assert_eq!(deltas(&a, &b).len(), 1);
    }
    #[test]
    fn edits_beyond_preview_are_labeled_as_summary_not_formatting() {
        let before = Document::from_text(&"a".repeat(MAX_TEXT + 32));
        let after = Document::from_text(&format!("{} changed", "a".repeat(MAX_TEXT + 32)));
        let selection = Selection::caret(after.end_of(StoryRef::Body));
        let (marks, truncated) = compare(&before, &after, &selection);
        assert!(truncated);
        assert!(marks.first().unwrap().summary);
        assert!(!marks.first().unwrap().formatting);
        assert_eq!(marks.first().unwrap().start, selection.focus);
    }
    #[test]
    fn review_and_undo_are_not_document_markup() {
        let before = Document::from_text("Original");
        let after = Document::from_text("Changed");
        let mut r = Review::default();
        let sel = Selection::caret(Pos::body(0, 0));
        r.record(&before, &after, 1, 2, &sel, "AI");
        assert!(!r.marks.is_empty());
        assert_eq!(r.inspect(3)["highlighted"], false);
        r.refresh_after_undo(&before, 4, &sel);
        assert!(r.marks.is_empty());
    }
}

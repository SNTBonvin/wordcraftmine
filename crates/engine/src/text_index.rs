//! The body's text as one string, with offsets into it mapped to document positions and back:
//! what a host that keeps its own anchors outside the document (quotes of passages) works with.
//! The web build's embed API (`WordcraftEmbed` in `wordcraft-web`) reads and selects text, and
//! tints passages, through it.
//!
//! The text: paragraphs separated by `\n`, table cells by `\t` (several paragraphs in one cell
//! by `\n`); fields as their results; tracked deletions left out (the text as it reads with every
//! change accepted). Offsets are UTF-16 code units, as a JavaScript string counts them.
//!
//! An offset inside a field's result has no position of its own: it maps to the start of the
//! field, or (`end`) just after it, so a range that touches a field takes the whole field.

use wordcraft_doc::para::OBJ;
use wordcraft_doc::{Document, Paragraph, Path, Pos, StoryRef};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// Characters of the paragraph, the same bytes in the text as in the paragraph.
    Text,
    /// An inline object (a field) shown as its text; `raw_len` bytes in the paragraph.
    Object { raw_len: usize },
    /// The `\n` or `\t` after a paragraph.
    Sep,
}

/// A stretch of the text: `u16_len` UTF-16 units from `u16` (`hay_len` bytes from `hay` in the
/// text) stand for paragraph `para`'s bytes from `raw`.
#[derive(Clone, Debug)]
struct Seg {
    u16: usize,
    u16_len: usize,
    hay: usize,
    hay_len: usize,
    para: usize,
    raw: usize,
    kind: Kind,
}

/// The body text of a document and the map between UTF-16 offsets into it and [`Pos`]itions.
/// Built for one revision of the document: build it again after an edit.
#[derive(Clone, Debug, Default)]
pub struct TextIndex {
    text: String,
    len16: usize,
    paths: Vec<Path>,
    /// Per paragraph: its length in bytes, its first segment, and the UTF-16 offset where its
    /// text ends (where its separator starts).
    para_len: Vec<usize>,
    first_seg: Vec<usize>,
    end16: Vec<usize>,
    /// Contiguous and in order: every UTF-16 unit of the text is in exactly one segment.
    segs: Vec<Seg>,
}

/// Whether the paragraphs at `a` and `b` (next to each other in document order) are in
/// neighbouring cells of one table row: their paths first differ at a cell index.
fn cell_boundary(a: &Path, b: &Path) -> bool {
    a.0.iter().zip(&b.0).position(|(x, y)| x != y).is_some_and(|d| d % 3 == 2)
}

impl TextIndex {
    pub fn new(doc: &Document) -> TextIndex {
        let paths = doc.para_paths(StoryRef::Body);
        let mut ix = TextIndex { paths: Vec::with_capacity(paths.len()), ..Default::default() };
        for (i, path) in paths.iter().enumerate() {
            let para = doc.para(StoryRef::Body, path);
            ix.first_seg.push(ix.segs.len());
            ix.para_len.push(para.map_or(0, Paragraph::len));
            if let Some(p) = para {
                ix.push_para(i, p);
            }
            ix.end16.push(ix.len16);
            if let Some(next) = paths.get(i + 1) {
                let raw = para.map_or(0, Paragraph::len);
                let sep = if cell_boundary(path, next) { "\t" } else { "\n" };
                ix.push(Seg { u16: 0, u16_len: 0, hay: 0, hay_len: 0, para: i, raw, kind: Kind::Sep }, sep);
            }
        }
        ix.paths = paths;
        ix
    }

    /// The text (see the module docs).
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The text's length in UTF-16 units.
    pub fn len_utf16(&self) -> usize {
        self.len16
    }

    /// Append `shown` as segment `seg` (its offsets are filled in here).
    fn push(&mut self, mut seg: Seg, shown: &str) {
        let n16 = shown.encode_utf16().count();
        seg.u16 = self.len16;
        seg.u16_len = n16;
        seg.hay = self.text.len();
        seg.hay_len = shown.len();
        self.text.push_str(shown);
        self.len16 = self.len16.saturating_add(n16);
        self.segs.push(seg);
    }

    fn push_para(&mut self, para: usize, p: &Paragraph) {
        // The runs, and any text after them (a malformed paragraph) as live text.
        let mut ranges: Vec<(std::ops::Range<usize>, bool)> = p.run_ranges().map(|(r, c)| (r, c.del.is_some())).collect();
        let covered = ranges.last().map_or(0, |(r, _)| r.end);
        if covered < p.len() {
            ranges.push((covered..p.len(), false));
        }
        // Objects are numbered by their place in the text, tracked-deleted ones too.
        let mut k = 0usize;
        for (r, deleted) in ranges {
            let Some(t) = p.text.get(r.clone()) else { continue };
            for (i, c) in t.char_indices() {
                let at = r.start.saturating_add(i);
                if c == OBJ {
                    let obj = k;
                    k = k.saturating_add(1);
                    let shown = p.objects.get(obj).map_or("", |o| o.plain_text());
                    // An object that shows no text (a picture, a range field's marker) is skipped
                    // like deleted text: positions on either side of it map to the same offset.
                    if !deleted && !shown.is_empty() {
                        let raw_len = OBJ.len_utf8();
                        self.push(Seg { u16: 0, u16_len: 0, hay: 0, hay_len: 0, para, raw: at, kind: Kind::Object { raw_len } }, shown);
                    }
                } else if !deleted {
                    self.push_char(para, at, c);
                }
            }
        }
    }

    fn push_char(&mut self, para: usize, at: usize, c: char) {
        if let Some(seg) = self.segs.last_mut()
            && seg.kind == Kind::Text
            && seg.para == para
            && seg.raw.saturating_add(seg.hay_len) == at
        {
            let n16 = c.len_utf16();
            seg.u16_len = seg.u16_len.saturating_add(n16);
            seg.hay_len = seg.hay_len.saturating_add(c.len_utf8());
            self.text.push(c);
            self.len16 = self.len16.saturating_add(n16);
            return;
        }
        let mut buf = [0u8; 4];
        self.push(Seg { u16: 0, u16_len: 0, hay: 0, hay_len: 0, para, raw: at, kind: Kind::Text }, c.encode_utf8(&mut buf));
    }

    fn pos(&self, para: usize, off: usize) -> Pos {
        let path = self.paths.get(para).cloned().unwrap_or_else(|| Path::top(0));
        Pos::new(StoryRef::Body, path, off)
    }

    /// The position of UTF-16 offset `x` into the text (clamped to it). Where two positions
    /// have the same offset (either side of deleted text, a picture, or a paragraph's separator
    /// from the inside) the later one is taken, or the earlier one with `end`: a range from
    /// `pos_at(a, false)` to `pos_at(b, true)` holds exactly the text `a..b` shows. Inside a
    /// field's result: the field's start, or with `end` just after it. Inside a surrogate pair:
    /// before it.
    pub fn pos_at(&self, x: usize, end: bool) -> Pos {
        let x = x.min(self.len16);
        // The segment holding `x`: with `end` the one it ends or is inside, else the one it
        // starts or is inside.
        let k = if end {
            self.segs.partition_point(|s| s.u16.saturating_add(s.u16_len) < x)
        } else {
            self.segs.partition_point(|s| s.u16.saturating_add(s.u16_len) <= x)
        };
        let seg = self.segs.get(k).filter(|s| if end { s.u16 < x } else { s.u16 <= x });
        let Some(seg) = seg else {
            // Before all the text (only with `end`) or after it.
            return if x == 0 && !self.paths.is_empty() {
                self.pos(0, 0)
            } else {
                let last = self.paths.len().saturating_sub(1);
                self.pos(last, self.para_len.get(last).copied().unwrap_or(0))
            };
        };
        let d = x.saturating_sub(seg.u16);
        match seg.kind {
            Kind::Text => {
                let piece = self.text.get(seg.hay..seg.hay.saturating_add(seg.hay_len)).unwrap_or("");
                let mut units = 0usize;
                let mut bytes = piece.len();
                for (i, c) in piece.char_indices() {
                    if units.saturating_add(c.len_utf16()) > d {
                        bytes = i;
                        break;
                    }
                    units = units.saturating_add(c.len_utf16());
                }
                self.pos(seg.para, seg.raw.saturating_add(bytes))
            }
            Kind::Object { raw_len } => {
                let after = d >= seg.u16_len || (d > 0 && end);
                self.pos(seg.para, if after { seg.raw.saturating_add(raw_len) } else { seg.raw })
            }
            Kind::Sep => {
                if d == 0 {
                    self.pos(seg.para, seg.raw)
                } else {
                    self.pos(seg.para.saturating_add(1), 0)
                }
            }
        }
    }

    /// The UTF-16 offset into the text of `p`, or `None` when it isn't in the body. A position
    /// in deleted text, or by an object that shows no text, gets the offset of the live text
    /// after it; one inside a field, the field's start.
    pub fn offset_of(&self, p: &Pos) -> Option<usize> {
        if p.story != StoryRef::Body {
            return None;
        }
        let i = self.paths.binary_search(&p.path).ok()?;
        let from = self.first_seg.get(i).copied()?;
        for seg in self.segs.get(from..).unwrap_or(&[]).iter().take_while(|s| s.para == i) {
            match seg.kind {
                Kind::Sep => return Some(seg.u16),
                _ if p.off <= seg.raw => return Some(seg.u16),
                Kind::Text if p.off < seg.raw.saturating_add(seg.hay_len) => {
                    let mut n = p.off - seg.raw;
                    let start = seg.hay;
                    // A position inside a character counts as before it.
                    while n > 0 && !self.text.is_char_boundary(start.saturating_add(n)) {
                        n -= 1;
                    }
                    let units = self.text.get(start..start.saturating_add(n)).map_or(0, |t| t.encode_utf16().count());
                    return Some(seg.u16.saturating_add(units));
                }
                Kind::Object { raw_len } if p.off < seg.raw.saturating_add(raw_len) => return Some(seg.u16),
                _ => {}
            }
        }
        self.end16.get(i).copied()
    }
}

/// Offsets into a text that changed from `old` to `new`, carried over to `new`. The change is
/// taken to be one replacement: what lies between the texts' common start and common end. An
/// offset before it stays, one after it moves by the difference in length; a range start inside
/// it moves to after the replacement, a range end inside it to before. Text typed exactly at a
/// range's start or end stays outside the range.
///
/// Where an insertion or a deletion could have been made in more than one place (deleting
/// `"two "` from `"two three"` reads the same as deleting `"wo t"`), the place that keeps a
/// range whole is taken.
///
/// `ranges` are `(start, end)` UTF-16 offsets; each comes back with `start <= end <= ` the new
/// length.
pub fn remap_ranges(old: &str, new: &str, ranges: &mut [(usize, usize)]) {
    if old == new {
        return;
    }
    let a: Vec<u16> = old.encode_utf16().collect();
    let b: Vec<u16> = new.encode_utf16().collect();
    let is_low = |u: u16| (0xDC00..0xE000).contains(&u);
    let mut prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    // Never between the halves of a surrogate pair.
    if a.get(prefix).is_some_and(|u| is_low(*u)) || b.get(prefix).is_some_and(|u| is_low(*u)) {
        prefix = prefix.saturating_sub(1);
    }
    let limit = a.len().min(b.len()).saturating_sub(prefix);
    let mut suffix = a.iter().rev().zip(b.iter().rev()).take(limit).take_while(|(x, y)| x == y).count();
    if suffix > 0
        && let (Some(x), Some(y)) = (a.get(a.len() - suffix), b.get(b.len() - suffix))
        && (is_low(*x) || is_low(*y))
    {
        suffix -= 1;
    }
    let old_end = a.len().saturating_sub(suffix);
    let new_end = b.len().saturating_sub(suffix);
    // A pure insertion or deletion may slide back by `slide` units: the text just before it
    // repeats at its end. (The common start is as long as it can be, so it can't slide on.)
    let slide = if prefix == old_end {
        let ins = b.get(prefix..new_end).unwrap_or(&[]);
        b.get(..prefix).unwrap_or(&[]).iter().rev().zip(ins.iter().rev()).take_while(|(x, y)| x == y).count()
    } else if prefix == new_end {
        let del = a.get(prefix..old_end).unwrap_or(&[]);
        a.get(..prefix).unwrap_or(&[]).iter().rev().zip(del.iter().rev()).take_while(|(x, y)| x == y).count()
    } else {
        0
    };
    // An offset after the change (or that a deletion can slide in front of) moves by the change
    // in length.
    let shift = |x: usize| x.saturating_add(new_end).saturating_sub(old_end);
    // An offset the deletion could be slid in front of: it is after the deleted text.
    let after_deletion = |x: usize| prefix == new_end && x > prefix && x < old_end && old_end - x <= slide;
    for (start, end) in ranges.iter_mut() {
        let s = if *start >= old_end || after_deletion(*start) {
            shift(*start)
        } else if prefix == old_end && start.saturating_add(slide) >= prefix && *start <= prefix {
            // The insertion slid back to the range's start: typed in front of it.
            start.saturating_add(new_end - prefix)
        } else if *start <= prefix {
            *start
        } else {
            new_end
        };
        let e = if *end <= prefix {
            *end
        } else if *end >= old_end || after_deletion(*end) {
            shift(*end)
        } else {
            prefix
        };
        *start = s.min(b.len());
        *end = e.min(b.len()).max(*start);
    }
}

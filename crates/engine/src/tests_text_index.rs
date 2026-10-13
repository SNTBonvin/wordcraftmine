//! [`TextIndex`] (the body text with UTF-16 offsets mapped to positions and back),
//! [`remap_ranges`] (offsets carried over an edit) and [`Session::read_only`]: what the web
//! build's embed API stands on.

use serde_json::json;
use wordcraft_doc::props::CharProps;
use wordcraft_doc::{Document, InlineObject, Path, Pos, StoryRef};

use crate::text_index::remap_ranges;
use crate::{Session, TextIndex};

fn run(s: &mut Session, id: &str, v: serde_json::Value) {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"));
}

fn body(path: &[u32], off: usize) -> Pos {
    Pos::new(StoryRef::Body, Path(path.to_vec()), off)
}

/// Every offset maps to a position that maps back to it (both ways of breaking ties).
fn assert_round_trip(ix: &TextIndex) {
    let text: Vec<u16> = ix.text().encode_utf16().collect();
    for x in 0..=ix.len_utf16() {
        // The second half of a surrogate pair has no position of its own.
        if text.get(x).is_some_and(|u| (0xDC00..0xE000).contains(u)) {
            continue;
        }
        for end in [false, true] {
            let p = ix.pos_at(x, end);
            assert_eq!(ix.offset_of(&p), Some(x), "offset {x} (end: {end}) → {p:?} in {:?}", ix.text());
        }
    }
}

/// The text `a..b` of the index (UTF-16 offsets).
fn slice(ix: &TextIndex, a: usize, b: usize) -> String {
    let u: Vec<u16> = ix.text().encode_utf16().collect();
    String::from_utf16_lossy(u.get(a..b).unwrap_or(&[]))
}

#[test]
fn plain_paragraphs_round_trip() {
    let doc = Document::from_text("Hello world\n\nSecond 😀 para\nlast");
    let ix = TextIndex::new(&doc);
    assert_eq!(ix.text(), "Hello world\n\nSecond 😀 para\nlast");
    assert_eq!(ix.text(), doc.plain_text(StoryRef::Body));
    assert_round_trip(&ix);
    // UTF-16: the emoji is two units.
    assert_eq!(ix.len_utf16(), ix.text().encode_utf16().count());
    let x = ix.text().encode_utf16().count() - "last".len();
    assert_eq!(ix.pos_at(x, false), body(&[3], 0));
    // The paragraph's end and the next one's start share no offset: the `\n` is between them.
    assert_eq!(ix.pos_at(11, false), body(&[0], 11));
    assert_eq!(ix.pos_at(12, false), body(&[1], 0));
    // The empty paragraph.
    assert_eq!(ix.offset_of(&body(&[1], 0)), Some(12));
    // Past the end: clamped to the end of the last paragraph.
    assert_eq!(ix.pos_at(10_000, false), body(&[3], 4));
    // Not in the body, or no such paragraph.
    assert_eq!(ix.offset_of(&Pos::new(StoryRef::Part(3), Path::top(0), 0)), None);
    assert_eq!(ix.offset_of(&body(&[9], 0)), None);
}

#[test]
fn inside_a_surrogate_pair_maps_before_it() {
    let doc = Document::from_text("a😀b");
    let ix = TextIndex::new(&doc);
    assert_eq!(ix.pos_at(2, false), body(&[0], 1));
    // A byte offset inside the emoji counts as before it.
    assert_eq!(ix.offset_of(&body(&[0], 3)), Some(1));
    assert_eq!(ix.offset_of(&body(&[0], 5)), Some(3));
}

#[test]
fn table_cells_are_separated_by_tabs() {
    let mut s = Session::new(Document::from_text("Before"));
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 2}));
    let cells: Vec<Path> = s.doc.para_paths(StoryRef::Body).into_iter().filter(|p| p.0.len() == 4).collect();
    assert_eq!(cells.len(), 4);
    for (path, t) in cells.iter().zip(["a1", "b1", "a2", "b2"]) {
        s.doc.insert_text(&Pos::new(StoryRef::Body, path.clone(), 0), t, &CharProps::default()).unwrap();
    }
    // A second paragraph in the first cell.
    s.doc.split_paragraph(&Pos::new(StoryRef::Body, cells[0].clone(), 2)).unwrap();
    s.doc.insert_text(&Pos::new(StoryRef::Body, cells[0].with_last(1), 0), "more", &CharProps::default()).unwrap();
    let ix = TextIndex::new(&s.doc);
    let text = ix.text();
    assert!(text.starts_with("Before\n"), "{text:?}");
    assert!(text.contains("a1\nmore\tb1\na2\tb2"), "{text:?}");
    assert_round_trip(&ix);
    // Selecting a cell's text by offsets.
    let at = text.find("b1").unwrap();
    assert_eq!(ix.pos_at(at, false), Pos::new(StoryRef::Body, cells[1].clone(), 0));
    assert_eq!(ix.pos_at(at + 2, true), Pos::new(StoryRef::Body, cells[1].clone(), 2));
}

#[test]
fn fields_read_as_their_results() {
    let mut doc = Document::from_text("See  here");
    let field = InlineObject::Field { instr: "REF note".into(), result: "page 7".into(), locked: false, code: false };
    doc.insert_object(&body(&[0], 4), field, &CharProps::default()).unwrap();
    // A picture-like object that shows no text.
    doc.insert_object(&body(&[0], 0), InlineObject::FieldStart { instr: "X".into(), locked: false }, &CharProps::default()).unwrap();
    let ix = TextIndex::new(&doc);
    assert_eq!(ix.text(), "See page 7 here");
    assert_eq!(ix.text(), doc.plain_text(StoryRef::Body));
    let obj = wordcraft_doc::para::OBJ.len_utf8();
    let field_at = obj + 4;
    // Its edges round-trip.
    assert_eq!(ix.pos_at(4, false), body(&[0], field_at));
    assert_eq!(ix.offset_of(&body(&[0], field_at)), Some(4));
    assert_eq!(ix.pos_at(10, false), body(&[0], field_at + obj));
    assert_eq!(ix.offset_of(&body(&[0], field_at + obj)), Some(10));
    // Inside the result: the start, or just after the field for a range's end.
    assert_eq!(ix.pos_at(7, false), body(&[0], field_at));
    assert_eq!(ix.pos_at(7, true), body(&[0], field_at + obj));
    // The marker that shows nothing: both sides are offset 0, and offset 0 is after it.
    assert_eq!(ix.offset_of(&body(&[0], 0)), Some(0));
    assert_eq!(ix.offset_of(&body(&[0], obj)), Some(0));
    assert_eq!(ix.pos_at(0, false), body(&[0], obj));
    // Text after the field round-trips.
    for x in 10..=ix.len_utf16() {
        assert_eq!(ix.offset_of(&ix.pos_at(x, false)), Some(x));
    }
}

#[test]
fn tracked_deletions_are_left_out() {
    let mut s = Session::new(Document::from_text("Keep this gone and that"));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "select.text", json!({"text": "gone "}));
    run(&mut s, "text.delete", json!({}));
    // Typed while tracking: an insertion, which reads as text.
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "text.insert", json!({"text": "!"}));
    let p = s.doc.para(StoryRef::Body, &Path::top(0)).unwrap();
    assert!(p.text.contains("gone"), "the deletion is tracked, not made: {:?}", p.text);
    let ix = TextIndex::new(&s.doc);
    assert_eq!(ix.text(), "Keep this and that!");
    assert_round_trip(&ix);
    // Either side of the deleted text: the end of "this " and the start of "and".
    let and = "Keep this ".len();
    assert_eq!(ix.pos_at(and, true), body(&[0], and));
    assert_eq!(ix.pos_at(and, false), body(&[0], and + "gone ".len()));
    // A position inside the deleted text reads as the live text after it.
    assert_eq!(ix.offset_of(&body(&[0], and + 2)), Some(and));
    // "this and" by offsets: a range that takes the deleted text in between along.
    assert_eq!(slice(&ix, 5, and + 3), "this and");
    assert_eq!((ix.pos_at(5, false), ix.pos_at(and + 3, true)), (body(&[0], 5), body(&[0], and + "gone ".len() + 3)));
}

#[test]
fn empty_document() {
    let ix = TextIndex::new(&Document::new());
    assert_eq!(ix.text(), "");
    assert_eq!(ix.pos_at(0, false), body(&[0], 0));
    assert_eq!(ix.pos_at(5, true), body(&[0], 0));
    assert_eq!(ix.offset_of(&body(&[0], 0)), Some(0));
    // A document with no paragraph at all (never left so by the session).
    let mut bare = Document::new();
    bare.body.clear();
    let ix = TextIndex::new(&bare);
    assert_eq!(ix.pos_at(3, false), body(&[0], 0));
}

#[test]
fn ranges_survive_an_edit_before_them() {
    let old = "One two three four";
    // "three" is carded.
    let three = (8, 13);
    let mut r = [three];
    remap_ranges(old, "One and two three four", &mut r);
    assert_eq!(r, [(12, 17)]);
    // Deleting before it.
    let mut r = [three];
    remap_ranges(old, "One three four", &mut r);
    assert_eq!(r, [(4, 9)]);
    // An edit after it changes nothing.
    let mut r = [three];
    remap_ranges(old, "One two three fours", &mut r);
    assert_eq!(r, [three]);
    // Typing inside it grows it; typing just before or after it doesn't.
    let mut r = [three];
    remap_ranges(old, "One two thrXee four", &mut r);
    assert_eq!(r, [(8, 14)]);
    let mut r = [three];
    remap_ranges(old, "One two Xthree four", &mut r);
    assert_eq!(r, [(9, 14)]);
    let mut r = [three];
    remap_ranges(old, "One two threeX four", &mut r);
    assert_eq!(r, [(8, 13)]);
    // Deleting its start, or all of it.
    let mut r = [three];
    remap_ranges(old, "One two ee four", &mut r);
    assert_eq!(r, [(8, 10)]);
    let mut r = [three];
    remap_ranges(old, "One two  four", &mut r);
    assert_eq!(r, [(8, 8)]);
    // UTF-16 units: an emoji typed before it moves it by two.
    let mut r = [three];
    remap_ranges(old, "😀One two three four", &mut r);
    assert_eq!(r, [(10, 15)]);
    // Junk offsets are clamped, never past the text.
    let mut r = [(50, 3), (usize::MAX, usize::MAX)];
    remap_ranges(old, "short", &mut r);
    assert!(r.iter().all(|&(a, b)| a <= b && b <= 5), "{r:?}");
}

#[test]
fn decorations_follow_an_edit_through_the_index() {
    let mut s = Session::new(Document::from_text("Intro\nThe carded passage here."));
    let before = TextIndex::new(&s.doc);
    let at = before.text().find("carded passage").unwrap();
    let mut ranges = [(at, at + "carded passage".len())];
    // Type at the start of the document.
    run(&mut s, "text.insert", json!({"text": "New words. "}));
    let after = TextIndex::new(&s.doc);
    remap_ranges(before.text(), after.text(), &mut ranges);
    let [(a, b)] = ranges;
    assert_eq!(slice(&after, a, b), "carded passage");
    // And they map to the passage's positions.
    let (pa, pb) = (after.pos_at(a, false), after.pos_at(b, true));
    assert_eq!(s.doc.copy_range(&pa, &pb).plain_text(), "carded passage");
}

#[test]
fn read_only_refuses_edits_and_allows_selection() {
    let mut s = Session::new(Document::from_text("Hello world"));
    // Something to undo, then read-only.
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "text.insert", json!({"text": "!"}));
    s.dirty = false;
    s.read_only = true;
    let rev = s.rev();
    for (id, v) in [
        ("text.insert", json!({"text": "x"})),
        ("text.backspace", json!({})),
        ("format.bold", json!({})),
        ("edit.paste", json!({"text": "pasted"})),
        // Undo isn't flagged as changing the document, but it does.
        ("edit.undo", json!({})),
        ("edit.undoTo", json!({"count": 1})),
        ("document.setText", json!({"text": "gone"})),
        ("file.encrypt", json!({"password": "secret"})),
    ] {
        let e = s.run(id, &v).unwrap_err();
        assert!(matches!(e, crate::CmdError::Disabled(_)), "{id}: {e}");
    }
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Hello world!");
    assert!(s.can_undo(), "the undo step is still there");
    assert!(s.rev() >= rev);
    assert!(!s.dirty);
    assert!(s.password.is_none());
    // Selecting, moving the caret, the view and reading still work.
    run(&mut s, "select.text", json!({"text": "world"}));
    assert_eq!(s.selected_text(), "world");
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "view.zoom", json!({"value": 150}));
    run(&mut s, "edit.copy", json!({}));
    run(&mut s, "document.text", json!({}));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Hello world!");
    // Opening another document replaces this one: allowed.
    let data = crate::cmd::insert::base64_encode(b"Other text");
    run(&mut s, "file.open", json!({"path": "other.txt", "data": data}));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Other text");
    assert!(s.read_only);
    // Off again: editing works.
    s.read_only = false;
    run(&mut s, "select.all", json!({}));
    run(&mut s, "text.insert", json!({"text": "Hi"}));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Hi");
}

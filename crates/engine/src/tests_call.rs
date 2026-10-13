use serde_json::{Value, json};

use crate::{Session, cmd, params_doc};

fn s() -> Session {
    Session::new(wordcraft_doc::Document::new())
}

fn said(s: &mut Session, id: &str, p: Value) -> String {
    match s.call(id, &p) {
        Ok(Value::String(t)) => t,
        Ok(v) => v.to_string(),
        Err(e) => e,
    }
}

#[test]
fn every_params_doc_parses() {
    for c in cmd::registry().all() {
        let d = params_doc::parse(c.params);
        assert!(d.is_ok(), "{}: {:?}\n{}", c.id, d, c.params);
    }
}

#[test]
fn a_call_refuses_parameters_the_command_does_not_take() {
    let mut s = s();
    assert_eq!(
        said(&mut s, "text.insert", json!({"content": "Hi"})),
        "`text.insert` was given `content`, which it doesn't take. `text.insert` takes {\"text\": string, \"raw\"?: bool (no AutoCorrect or AutoFormat)}."
    );
    assert_eq!(
        said(&mut s, "edit.undo", json!({"steps": 2})),
        "`edit.undo` was given `steps`, which it doesn't take. `edit.undo` takes no parameters."
    );
    assert_eq!(
        said(&mut s, "format.size", json!([28])),
        "invalid parameters for `format.size`: the parameters are a JSON object, not an array. `format.size` takes {\"size\": number}."
    );
    assert_eq!(
        said(&mut s, "format.size", json!({})),
        "invalid parameters for `format.size`: `size` (number) is required. `format.size` takes {\"size\": number}."
    );
    assert_eq!(
        said(&mut s, "styles.modify", json!({"style": "Normal", "chr": {"color": [1, 2, 3]}})),
        "invalid parameters for `styles.modify`: `chr` is not a CharProps: invalid type: sequence, expected string or map. Lengths are points, `color` is \"RRGGBB\", other colours [r, g, b]; `wordcraft-cli help types` and docs/mcp.md list the fields. `styles.modify` takes {\"style\": string, \"chr\"?: CharProps, \"para\"?: ParaProps, \"name\"?: string, \"next\"?: string}."
    );
    assert_eq!(s.doc.plain_text(wordcraft_doc::StoryRef::Body), "");
    assert!(s.call("edit.advancedFind", &json!({"text": "x", "matchCase": true})).is_ok());
}

#[test]
fn an_unknown_command_names_the_commands_like_it() {
    let mut s = s();
    let e = said(&mut s, "new", json!({}));
    assert!(e.starts_with("`new` is not a command; commands named like it: file.new, ") && e.ends_with("."), "{e}");
    assert_eq!(said(&mut s, "file.nwe", json!({})), "`file.nwe` is not a command.");
}

#[test]
fn a_command_not_available_names_what_makes_it_so() {
    let mut s = s();
    assert_eq!(
        said(&mut s, "picture.size", json!({"width": 50})),
        "`picture.size` is not available right now: select a picture or shape first. insert.picture or insert.shape inserts one and selects it; select.objects {\"index\": n} selects an existing one."
    );
    assert_eq!(
        said(&mut s, "table.deleteRow", json!({})),
        "`table.deleteRow` is not available right now: the cursor isn't in a table. insert.table inserts one and puts the caret in its first cell; caret.set {\"pos\": {\"path\": [block, row, cell, paragraph], \"off\": 0}} moves into one."
    );
}

#[test]
fn a_call_that_answers_with_the_selection_says_what_changed() {
    let mut s = s();
    assert_eq!(
        said(&mut s, "text.insert", json!({"text": "Bondata Oy"})),
        "inserted `Bondata Oy` into paragraph 0, offsets 0–10; the caret is at paragraph 0, offset 10"
    );
    assert_eq!(said(&mut s, "select.paragraph", json!({})), "selected `Bondata Oy` in paragraph 0, offsets 0–10");
    assert_eq!(said(&mut s, "format.font", json!({"name": "Georgia"})), "formatted paragraph 0: font Georgia");
    assert_eq!(said(&mut s, "format.color", json!({"color": "0047BB"})), "formatted paragraph 0: color 0047BB");
    assert_eq!(said(&mut s, "para.align", json!({"value": "right"})), "formatted paragraph 0: align right");
    assert_eq!(said(&mut s, "caret.docEnd", json!({})), "the caret is at paragraph 0, offset 10");
    assert_eq!(said(&mut s, "format.bold", json!({})), "typing will use bold true");
    assert_eq!(said(&mut s, "text.newParagraph", json!({})), "inserted paragraph 1, empty; the caret is at paragraph 1, offset 0");
    assert_eq!(said(&mut s, "edit.undo", json!({})), "removed 1 paragraph at paragraph 1; the caret is at paragraph 0, offset 10");
    assert_eq!(said(&mut s, "insert.closeHeader", json!({})), "the caret is at paragraph 0, offset 0");
    assert_eq!(said(&mut s, "select.collapse", json!({})), "nothing changed");
    assert_eq!(said(&mut s, "styles.modify", json!({"style": "Normal", "chr": {"font": "Georgia"}})), "modified the style `Normal`");
    let r = s.call("layout.margins", &json!({"top": 56})).unwrap();
    assert_eq!(r["changed"], "page setup: marginTop 56");
    assert_eq!(r["marginTop"], 56.0);
}

#[test]
fn a_part_and_an_object_are_named_in_the_answer() {
    let mut s = s();
    let r = s.call("insert.header", &json!({"text": "Bondata"})).unwrap();
    assert_eq!(r["changed"], "added header 1 reading `Bondata`; the caret is at paragraph 0 of header 1, offset 0");
    assert_eq!(r["story"], 1);
    assert_eq!(said(&mut s, "caret.end", json!({})), "the caret is at paragraph 0 of header 1, offset 7");
    assert_eq!(
        said(&mut s, "text.insert", json!({"text": " Oy"})),
        "inserted ` Oy` into paragraph 0 of header 1, offsets 7–10; the caret is at paragraph 0 of header 1, offset 10"
    );
    assert_eq!(said(&mut s, "insert.closeHeader", json!({})), "the caret is at paragraph 0, offset 0");
    let r = s.call("insert.table", &json!({"rows": 2, "cols": 3})).unwrap();
    assert_eq!(
        r["changed"],
        "inserted a table of 2 rows × 3 columns at block 0; the caret is at paragraph 0 of cell (0, 0) of the table at block 0, offset 0"
    );
    let mut t = Session::new(wordcraft_doc::Document::new());
    t.call("text.insert", &json!({"text": "Logo"})).unwrap();
    t.call("select.paragraph", &json!({})).unwrap();
    let r = said(&mut t, "insert.shape", json!({"kind": "rectangle"}));
    assert!(r.starts_with("replaced `Logo` in paragraph 0 with a shape; "), "{r}");
}

#[test]
fn every_key_a_command_reads_is_in_its_doc() {
    let mut s = s();
    for (id, p) in [
        ("format.allCaps", json!({"value": true})),
        ("para.keepNext", json!({"value": true})),
        ("view.marks", json!({"value": true})),
        ("styles.addToGallery", json!({"style": "Heading 1"})),
        ("select.collapse", json!({"end": true})),
        ("edit.findNext", json!({"text": "x", "wildcards": true})),
        ("select.text", json!({"text": "x", "story": "body"})),
        ("text.insert", json!({"text": "x", "raw": true})),
        ("table.borderPainter", json!({"color": "FF0000"})),
        ("document.text", json!({"story": "body"})),
        ("para.outlineLevel", json!({"level": 9})),
    ] {
        let r = s.call(id, &p);
        assert!(!r.as_ref().is_err_and(|e| e.contains("doesn't take")), "{id}: {r:?}");
    }
}

#[test]
fn a_range_format_names_the_text_it_formatted() {
    let mut s = s();
    s.call("text.insert", &json!({"text": "Bondata Oy"})).unwrap();
    s.call("select.range", &json!({"anchor": {"block": 0, "off": 8}, "focus": {"block": 0, "off": 10}})).unwrap();
    assert_eq!(said(&mut s, "format.bold", json!({})), "formatted `Oy` in paragraph 0, offsets 8–10: bold true");
}

#[test]
fn hostile_params_never_panic_through_call() {
    let reg = cmd::registry();
    let junk = [
        json!(null),
        json!([1, 2]),
        json!({"text": 5, "size": "x", "path": [], "pos": {"block": 9999, "off": 99999}}),
        json!({"value": -1e308, "rows": 1e9}),
    ];
    for spec in reg.all() {
        if spec.id.starts_with("file.") || spec.id == "insert.picture" || spec.id == "insert.textFromFile" {
            continue;
        }
        for j in &junk {
            let mut s = Session::new(crate::sample::sample_document());
            let _ = s.call(spec.id, j);
            s.clamp_selection();
            let _ = s.layout();
        }
    }
}

#[test]
fn long_text_is_shown_by_its_ends_with_its_length() {
    let mut s = s();
    let text = "Thank you for running the autumn survey with us. 84% of your people answered, 12 points more than in spring, and the full report is attached.";
    assert_eq!(
        said(&mut s, "text.insert", json!({"text": text})),
        "inserted `Thank you for running the autu … full report is attached.` (141 bytes) into paragraph 0, offsets 0–141; the caret is at paragraph 0, offset 141"
    );
    assert_eq!(said(&mut s, "text.backspace", json!({})), "deleted `.` from paragraph 0, offsets 140–141; the caret is at paragraph 0, offset 140");
    assert_eq!(said(&mut s, "edit.undo", json!({})), "inserted `.` into paragraph 0, offsets 140–141; the caret is at paragraph 0, offset 141");
    s.call("select.range", &json!({"anchor": {"block": 0, "off": 0}, "focus": {"block": 0, "off": 5}})).unwrap();
    assert_eq!(
        said(&mut s, "text.insert", json!({"text": "Cheers"})),
        "replaced `Thank` with `Cheers` in paragraph 0, offsets 0–6; the caret is at paragraph 0, offset 6"
    );
    s.call("caret.docEnd", &json!({})).unwrap();
    s.call("text.newParagraph", &json!({})).unwrap();
    s.call("text.insert", &json!({"text": "Kind regards"})).unwrap();
    assert_eq!(
        said(&mut s, "select.all", json!({})),
        "selected from paragraph 0, offset 0 (starting `Cheers you for running the aut`) to paragraph 1, offset 12 (ending `Kind regards`)"
    );
}

#[test]
fn resolving_a_comment_is_said() {
    let mut s = s();
    s.call("text.insert", &json!({"text": "Hello"})).unwrap();
    s.call("select.all", &json!({})).unwrap();
    s.call("review.newComment", &json!({"text": "Check this"})).unwrap();
    let id = s.doc.comments.keys().next().copied().unwrap();
    assert_eq!(said(&mut s, "review.resolveComment", json!({"id": id})), format!("resolved comment {id}"));
    assert_eq!(said(&mut s, "review.resolveComment", json!({"id": id, "value": false})), format!("reopened comment {id}"));
}

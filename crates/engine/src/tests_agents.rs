use super::*;
use serde_json::json;

fn agent(s: &mut Session, selection: Selection, id: &str, params: Value) -> Selection {
    s.run_as_agent("Test Agent", selection, None, id, &params).unwrap().1
}

#[test]
fn agent_text_and_formatting_are_tracked_by_agent_with_owner_settings_restored() {
    let mut s = Session::new(Document::from_text("Hello"));
    s.author = "Owner".into();
    s.initials = "OW".into();
    s.prefs.markup.track_formatting = false;
    let end = agent(&mut s, Selection::caret(Pos::body(0, 5)), "text.insert", json!({"text":" agent"}));
    let changes = s.run("review.changes", &json!({})).unwrap();
    assert!(changes.to_string().contains("Test Agent"));
    assert!(!s.doc.settings.track_changes);
    assert_eq!(s.author, "Owner");
    assert_eq!(s.initials, "OW");
    assert!(!s.prefs.markup.track_formatting);
    let range = Selection { anchor: Pos::body(0, 0), focus: Pos::body(0, 5) };
    agent(&mut s, range, "format.bold", json!({"value":true}));
    assert!(crate::cmd::review::format_changes(&s).iter().any(|(_, author, _)| author == "Test Agent"));
    assert_eq!(end.focus.off, 11);
    assert!(s.undo());
    assert!(!s.doc.settings.track_changes);
    assert!(s.undo());
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Hello");
    assert!(!s.doc.settings.track_changes);
    assert!(s.redo());
    assert!(!s.doc.settings.track_changes);
}

#[test]
fn agent_preserves_owner_view_find_repeat_macro_and_caret_formatting() {
    let mut s = Session::new(Document::from_text("Alpha beta"));
    let owner = Selection::caret(Pos::body(0, 8));
    s.sel = owner.clone();
    s.pending = Some(CharProps { italic: Some(true), ..Default::default() });
    s.view.zoom = 1.7;
    s.view.nav_pane = true;
    s.find.query = "owner query".into();
    s.find.results = vec![(Pos::body(0, 0), Pos::body(0, 5))];
    s.goal_x = Some(78.0);
    s.page_hint = 3;
    s.painter = Some((CharProps::default(), wordcraft_doc::ParaProps::default(), true));
    s.math_latex = true;
    s.math_normal_text = true;
    s.last_command = Some(("para.alignRight".into(), json!({})));
    s.recording = Some(("Owner macro".into(), vec![("text.insert".into(), json!({"text":"owner"}))]));
    s.edit_marks = vec![Pos::body(0, 4)];
    s.go_back_step = 2;
    s.ui_requests = vec![json!({"owner":true})];
    s.status = "Owner status".into();
    let pending = s.pending.clone();
    let view = serde_json::to_value(&s.view).unwrap();
    let recording = s.recording.clone();
    let selection = agent(&mut s, Selection::caret(Pos::body(0, 0)), "select.text", json!({"text":"Alpha"}));
    assert_eq!(selection, Selection { anchor: Pos::body(0, 0), focus: Pos::body(0, 5) });
    assert_eq!(s.sel, owner);
    assert_eq!(s.pending, pending);
    assert_eq!(serde_json::to_value(&s.view).unwrap(), view);
    assert_eq!(s.find.query, "owner query");
    assert_eq!(s.find.results.len(), 1);
    assert_eq!(s.goal_x, Some(78.0));
    assert_eq!(s.page_hint, 3);
    assert!(s.painter.is_some());
    assert!(s.math_latex && s.math_normal_text);
    assert_eq!(s.last_command, Some(("para.alignRight".into(), json!({}))));
    assert_eq!(s.recording, recording);
    assert_eq!(s.edit_marks, vec![Pos::body(0, 4)]);
    assert_eq!(s.go_back_step, 2);
    assert_eq!(s.ui_requests, vec![json!({"owner":true})]);
    assert_eq!(s.status, "Owner status");
}

#[test]
fn owner_agent_owner_typing_are_three_independent_undo_steps() {
    let mut s = Session::new(Document::from_text(""));
    s.prefs.autocorrect.enabled = false;
    s.run("text.insert", &json!({"text":"A"})).unwrap();
    let owner = s.sel.clone();
    agent(&mut s, Selection::caret(Pos::body(0, 1)), "text.insert", json!({"text":"B"}));
    assert_eq!(s.sel, owner);
    s.run("text.insert", &json!({"text":"C"})).unwrap();
    assert_eq!(s.undo_depth(), 3);
    assert_eq!(s.doc.plain_text(StoryRef::Body), "ACB");
    assert!(s.undo());
    assert_eq!(s.doc.plain_text(StoryRef::Body), "AB");
    assert!(s.undo());
    assert_eq!(s.doc.plain_text(StoryRef::Body), "A");
    assert_eq!(s.sel, owner);
    assert!(s.undo());
    assert_eq!(s.doc.plain_text(StoryRef::Body), "");
}

#[test]
fn unicode_selection_follows_agent_paragraph_split() {
    let mut s = Session::new(Document::from_text("Привет мир"));
    s.sel = Selection { anchor: Pos::body(0, 13), focus: Pos::body(0, 19) };
    let selected = s.selected_text();
    agent(&mut s, Selection::caret(Pos::body(0, 12)), "text.newParagraph", json!({}));
    assert_eq!(s.sel.anchor.path, Path::top(1));
    assert_eq!(s.selected_text(), selected);
    assert_eq!(s.sel.anchor.off, 1);
    assert_eq!(s.sel.focus.off, 7);
}

#[test]
fn owner_selection_in_table_cell_follows_agent_insertion_and_split() {
    let mut s = Session::new(Document::new());
    s.run("insert.table", &json!({"rows":1,"cols":1})).unwrap();
    s.run("text.insert", &json!({"text":"café sample"})).unwrap();
    let mut anchor = s.sel.focus.clone();
    anchor.off = 6;
    let mut focus = anchor.clone();
    focus.off = 12;
    s.sel = Selection { anchor, focus };
    assert_eq!(s.selected_text(), "sample");
    let mut insertion = s.sel.anchor.clone();
    insertion.off = 0;
    agent(&mut s, Selection::caret(insertion.clone()), "text.insert", json!({"text":"New "}));
    assert_eq!(s.selected_text(), "sample");
    insertion.off = 9;
    agent(&mut s, Selection::caret(insertion), "text.newParagraph", json!({}));
    assert_eq!(s.selected_text(), "sample");
    assert_eq!(s.sel.anchor.path.0.len(), 4);
}

#[test]
fn failed_agent_command_restores_document_history_and_transient_state_exactly() {
    let mut s = Session::new(Document::from_text("Owner text"));
    s.run("text.insert", &json!({"text":"X"})).unwrap();
    s.pending = Some(CharProps { bold: Some(true), ..Default::default() });
    s.recording = Some(("Owner".into(), Vec::new()));
    let doc = serde_json::to_value(&s.doc).unwrap();
    let selection = s.sel.clone();
    let pending = s.pending.clone();
    let history = s.history.clone();
    let redo = s.redo.clone();
    let revision = s.rev();
    let dirty = s.dirty;
    let typing = s.typing_open;
    let last_command = s.last_command.clone();
    let result = s.run_as_agent("Test Agent", Selection::caret(Pos::body(0, 0)), None, "format.size", &json!({"size":"bad"}));
    assert!(result.is_err());
    assert_eq!(serde_json::to_value(&s.doc).unwrap(), doc);
    assert_eq!(s.sel, selection);
    assert_eq!(s.pending, pending);
    assert_eq!(s.rev(), revision);
    assert_eq!(s.dirty, dirty);
    assert_eq!(s.typing_open, typing);
    assert_eq!(s.last_command, last_command);
    assert_eq!(s.history.len(), history.len());
    assert!(s.history.iter().zip(history.iter()).all(|(a, b)| Arc::ptr_eq(a, b)));
    assert_eq!(s.redo.len(), redo.len());
    assert_eq!(s.recording, Some(("Owner".into(), Vec::new())));
}

#[test]
fn agent_disables_owner_autocorrect_and_overtype_temporarily() {
    let mut s = Session::new(Document::from_text("Existing"));
    s.prefs.overtype = true;
    let prefs = s.prefs.clone();
    agent(&mut s, Selection::caret(Pos::body(0, 0)), "text.insert", json!({"text":"teh "}));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "teh Existing");
    assert_eq!(s.prefs, prefs);
}

#[test]
fn owner_column_mode_cannot_delete_stale_segments_after_agent_inserts() {
    let mut s = Session::new(Document::from_text("Hello world"));
    let owner = Selection { anchor: Pos::body(0, 6), focus: Pos::body(0, 11) };
    s.sel = owner.clone();
    s.column = Some(ColumnBlock {
        sel: owner.clone(),
        anchor_x: 1.0,
        focus_x: 2.0,
        segments: vec![(owner.anchor.clone(), owner.focus.clone())],
        cells: None,
    });
    s.column_mode = true;
    agent(&mut s, Selection::caret(Pos::body(0, 0)), "text.insert", json!({"text":"New "}));
    assert_eq!(s.selected_text(), "world");
    assert!(s.column.is_none());
    assert!(!s.column_mode);
    s.run("text.delete", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), "New Hello ");
}

#[test]
fn pending_agent_formatting_keeps_owner_column_mode_when_document_did_not_change() {
    let mut s = Session::new(Document::from_text("Hello world"));
    let owner = Selection { anchor: Pos::body(0, 6), focus: Pos::body(0, 11) };
    s.sel = owner.clone();
    let column =
        ColumnBlock { sel: owner.clone(), anchor_x: 1.0, focus_x: 2.0, segments: vec![(owner.anchor.clone(), owner.focus.clone())], cells: None };
    s.column = Some(column.clone());
    s.column_mode = true;
    agent(&mut s, Selection::caret(Pos::body(0, 0)), "format.bold", json!({"value":true}));
    assert_eq!(s.sel, owner);
    assert_eq!(s.column, Some(column));
    assert!(s.column_mode);
}

#[test]
fn agent_author_is_reserved_by_host_and_cannot_impersonate_existing_authors() {
    use wordcraft_doc::{Comment, Revision, RevisionKind};
    let mut s = Session::new(Document::from_text("Owner text"));
    s.author = "Owner".into();
    let first_candidate = "Owner (WordCraft agent host-session-1)";
    s.doc.revisions.push(Revision::new(RevisionKind::Insert, first_candidate, ""));
    s.doc.comments.insert(0, Comment { author: "Owner (WordCraft agent host-session-1-1)".into(), ..Default::default() });
    let reserved = s.agent_author("Owner", "host-session-1").unwrap();
    assert_ne!(reserved, s.author);
    assert_ne!(reserved, first_candidate);
    assert_eq!(reserved, "Owner (WordCraft agent host-session-1-2)");
    assert!(reserved.starts_with("Owner (WordCraft agent host-session-1-"));
    assert!(s.doc.revisions.iter().all(|revision| revision.author != reserved));
    assert!(s.doc.comments.values().all(|comment| comment.author != reserved));
    s.run_as_agent(&reserved, Selection::caret(Pos::body(0, 0)), None, "text.insert", &json!({"text":"A"})).unwrap();
    s.run_as_agent(&reserved, Selection::caret(Pos::body(0, 1)), None, "text.insert", &json!({"text":"B"})).unwrap();
    assert!(s.doc.revisions.iter().any(|revision| revision.author == reserved));
    assert_eq!(s.author, "Owner");
    assert!(s.agent_author("Owner", "").is_err());
    assert!(s.agent_author("Owner", "client/supplied").is_err());
}

#[test]
fn owner_name_as_agent_author_is_rejected_before_any_state_change() {
    let mut s = Session::new(Document::from_text("Owner text"));
    s.author = "Owner".into();
    let document = serde_json::to_value(&s.doc).unwrap();
    let revision = s.rev();
    let selection = s.sel.clone();
    for label in ["Owner", " Owner "] {
        assert!(s.run_as_agent(label, Selection::caret(Pos::body(0, 0)), None, "text.insert", &json!({"text":"Impersonated"})).is_err());
        assert_eq!(serde_json::to_value(&s.doc).unwrap(), document);
        assert_eq!(s.rev(), revision);
        assert_eq!(s.sel, selection);
        assert_eq!(s.undo_depth(), 0);
    }
}

#[test]
fn agent_can_bold_all_duplicate_and_blank_paragraphs_without_moving_owner_caret() {
    for owner in [Pos::body(0, 2), Pos::body(1, 2), Pos::body(2, 0), Pos::body(3, 0)] {
        let mut s = Session::new(Document::from_text("same\nsame\n\n"));
        s.sel = Selection::caret(owner);
        let owner = s.sel.clone();
        let agent_selection = agent(&mut s, Selection::caret(Pos::body(0, 0)), "select.all", json!({}));
        agent(&mut s, agent_selection, "format.bold", json!({"value":true}));
        assert_eq!(s.sel, owner);
        assert_eq!(s.doc.plain_text(StoryRef::Body), "same\nsame\n\n");
        assert!(crate::cmd::review::format_changes(&s).iter().any(|(_, author, _)| author == "Test Agent"));
        assert!(!s.doc.settings.track_changes);
        assert!(s.undo());
        assert_eq!(s.sel, owner);
    }
}

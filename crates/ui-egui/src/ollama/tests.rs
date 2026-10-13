use super::*;
use wordcraft_engine::Session;

fn app() -> WordApp {
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Original")), crate::Services::default());
    app.ollama.document = Some(app.session.document_id());
    app.ollama.turn_revision = Some(app.session.rev());
    app
}
fn execute(app: &mut WordApp, id: &str, params: Value) -> Result<Value, String> {
    document_tool(app, "execute_command", &json!({"command":id,"params":params}))
}
fn text(app: &WordApp) -> String {
    app.session.doc.plain_text(wordcraft_doc::StoryRef::Body)
}

#[test]
fn hard_edit_switch_blocks_content_and_selection_changes() {
    let mut app = app();
    let before = text(&app);
    let sel = app.session.sel.clone();
    let depth = app.session.undo_depth();
    for id in ["text.insert", "document.setText", "format.bold", "select.all", "caret.end"] {
        assert!(execute(&mut app, id, json!({"text":"unexpected"})).is_err(), "{id}");
        assert_eq!(text(&app), before);
        assert_eq!(app.session.sel, sel);
        assert_eq!(app.session.undo_depth(), depth);
    }
    assert!(document_tool(&mut app, "read_document", &json!({})).is_ok());
    assert!(execute(&mut app, "document.inspect", json!({"text":false})).is_ok());
}

#[test]
fn disabling_edits_rejects_already_queued_model_commands() {
    let mut app = app();
    app.ollama.allow_edits = true;
    let (tx, rx) = std::sync::mpsc::channel();
    let (reply, answer) = std::sync::mpsc::channel();
    tx.send(Event::Tool { name: "execute_command".into(), args: json!({"command":"text.insert","params":{"text":"bad"}}), reply }).unwrap();
    app.ollama.events = Some(rx);
    app.ollama.busy = true;
    app.ollama.allow_edits = false;
    poll(&mut app, &egui::Context::default());
    assert_eq!(answer.recv().unwrap()["ok"], false);
    assert_eq!(text(&app), "Original");
}

#[test]
fn ai_undo_is_separate_from_manual_typing_and_from_other_ai_steps() {
    let mut app = app();
    app.session.run("text.insert", &json!({"text":"Manual "})).unwrap();
    let manual = text(&app);
    app.ollama.turn_revision = Some(app.session.rev());
    app.ollama.allow_edits = true;
    execute(&mut app, "text.insert", json!({"text":"AI one "})).unwrap();
    let first = text(&app);
    execute(&mut app, "text.insert", json!({"text":"AI two "})).unwrap();
    assert!(can_undo(&app));
    undo(&mut app).unwrap();
    assert_eq!(text(&app), first);
    undo(&mut app).unwrap();
    assert_eq!(text(&app), manual);
    app.session.undo();
    assert_eq!(text(&app), "Original");
}

#[test]
fn ai_undo_never_discards_later_manual_edits() {
    let mut app = app();
    app.ollama.allow_edits = true;
    execute(&mut app, "text.insert", json!({"text":"AI "})).unwrap();
    app.session.run("text.insert", &json!({"text":"Manual "})).unwrap();
    let before = text(&app);
    assert!(!can_undo(&app));
    assert!(undo(&mut app).is_err());
    assert_eq!(text(&app), before);
}

#[test]
fn changed_document_and_manual_edits_in_flight_are_rejected() {
    let mut app = app();
    app.ollama.allow_edits = true;
    app.session.run("text.insert", &json!({"text":"Manual "})).unwrap();
    assert!(execute(&mut app, "text.insert", json!({"text":"AI "})).unwrap_err().contains("changed"));
    document_tool(&mut app, "read_document", &json!({})).unwrap();
    execute(&mut app, "text.insert", json!({"text":"AI "})).unwrap();
    app.session.set_document(wordcraft_doc::Document::from_text("Different"));
    assert!(execute(&mut app, "text.insert", json!({"text":"AI "})).is_err());
    poll(&mut app, &egui::Context::default());
    assert!(!app.ollama.allow_edits);
    assert!(!can_undo(&app));
    assert_eq!(text(&app), "Different");
}

#[test]
fn scope_gate_blocks_disk_macros_passwords_and_nested_side_effects() {
    let mut app = app();
    app.ollama.allow_edits = true;
    for id in ["file.save", "file.open", "file.new", "tools.runMacro", "edit.repeat", "font.add", "review.restrict", "insert.textFromFile"] {
        assert!(execute(&mut app, id, json!({})).is_err(), "{id}");
    }
    assert!(!allowed("insert.picture", &json!({"path":"/private/file"})));
    assert!(!allowed("table.insert", &json!({"nested":{"filename":"file"}})));
    assert!(allowed("document.paragraph", &json!({"path":[0]})));
    assert_eq!(text(&app), "Original");
}

#[test]
fn preferences_keep_model_and_backend_but_never_credentials_or_edit_permission() {
    let mut app = app();
    app.ollama.source = Source::LlamaCpp;
    app.ollama.model = "qwen3.5:4b".into();
    app.ollama.api_key = "memory-only-test-key".into();
    app.ollama.allow_edits = true;
    let saved = app.prefs();
    let bytes = serde_json::to_string(&saved).unwrap();
    assert!(!bytes.contains("memory-only-test-key"));
    assert!(!bytes.contains("allowEdits"));
    let mut next = WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    next.apply_prefs(saved);
    assert_eq!(next.ollama.source, Source::LlamaCpp);
    assert_eq!(next.ollama.model, "qwen3.5:4b");
    assert!(!next.ollama.allow_edits);
}

#[test]
fn verified_writing_covers_unicode_paragraphs_selection_and_undo() {
    let mut app = app();
    app.ollama.allow_edits = true;
    let result = document_tool(&mut app, "write_text", &json!({"mode":"append","text":"\nCafé — 世界"})).unwrap();
    assert_eq!(result["verified"], true);
    assert_eq!(text(&app), "Original\nCafé — 世界");
    undo(&mut app).unwrap();
    assert_eq!(text(&app), "Original");
    app.ollama.cancel = Arc::new(AtomicBool::new(false));
    app.ollama.turn_revision = Some(app.session.rev());
    execute(&mut app, "select.text", json!({"text":"Original"})).unwrap();
    document_tool(&mut app, "write_text", &json!({"mode":"replace_selection","text":"New draft"})).unwrap();
    assert_eq!(text(&app), "New draft");
    undo(&mut app).unwrap();
    assert_eq!(text(&app), "Original");
}

#[test]
fn verified_writing_rejects_invalid_actions_without_changes() {
    let mut app = app();
    app.ollama.allow_edits = true;
    for args in [json!({"mode":"shell","text":"bad"}), json!({"mode":"append"}), json!({"mode":"replace_selection","text":"bad"})] {
        assert!(document_tool(&mut app, "write_text", &args).is_err());
        assert_eq!(text(&app), "Original");
        assert!(!can_undo(&app));
    }
}

#[test]
fn prose_command_discovery_distinguishes_typing_from_toc_add_text() {
    let mut app = app();
    let found = document_tool(&mut app, "find_commands", &json!({"query":"append"})).unwrap();
    let ids: Vec<_> = found["commands"].as_array().unwrap().iter().map(|v| v["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"text.insert"));
    assert!(ids.contains(&"caret.docEnd"));
    let found = document_tool(&mut app, "find_commands", &json!({"query":"text"})).unwrap();
    assert!(found["commands"].as_array().unwrap().iter().take(5).any(|v| v["id"] == "text.insert"));
}

#[test]
fn failed_ai_command_preserves_undo_redo_and_does_not_create_ai_step() {
    let mut app = app();
    app.ollama.allow_edits = true;
    app.session.run("text.insert", &json!({"text":"Manual"})).unwrap();
    app.session.undo();
    app.ollama.turn_revision = Some(app.session.rev());
    let depth = app.session.undo_depth();
    let redo = app.session.redo_label().map(str::to_string);
    assert!(execute(&mut app, "table.insert", json!({"rows":0,"cols":0})).is_err());
    assert_eq!(app.session.undo_depth(), depth);
    assert_eq!(app.session.redo_label().map(str::to_string), redo);
    assert!(!can_undo(&app));
    assert_eq!(text(&app), "Original");
}

#[test]
fn codex_toggle_drops_pending_requests_and_remembers_preview_port() {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = app().with_codex_control(rx, 7982);
    let (off, off_reply) = crate::ControlRequest::new("engine.execute", json!({"command":"ui.codexConnect","params":{"value":false}}));
    let (edit, edit_reply) = crate::ControlRequest::new("engine.execute", json!({"command":"text.insert","params":{"text":"unwanted"}}));
    tx.send(off).unwrap();
    tx.send(edit).unwrap();
    app.drain_control(&egui::Context::default());
    assert_eq!(off_reply.recv().unwrap()["ok"], true);
    assert!(edit_reply.recv().is_err());
    assert_eq!(text(&app), "Original");
    assert!(!app.codex_is_ready());
    assert_eq!(app.codex_port(), Some(7982));
    assert!(app.take_codex_disconnect_request());
    app.run("ui.codexConnect", json!({"value":true})).unwrap();
    assert!(app.take_codex_connect_request());
}

#[test]
#[ignore = "requires a running Ollama and WORDCRAFT_TEST_OLLAMA_MODEL; uses a synthetic document only"]
fn live_ollama_edits_read_only_and_undo() {
    let model = std::env::var("WORDCRAFT_TEST_OLLAMA_MODEL").expect("test model");
    let ctx = egui::Context::default();
    let mut app = app();
    app.ollama.api_key.clear();
    if std::env::var("WORDCRAFT_TEST_BACKEND").as_deref() == Ok("llamacpp") {
        app.ollama.source = Source::LlamaCpp;
    }
    app.ollama.model = model;
    app.ollama.allow_edits = true;
    app.ollama.input = "Read this document, then append exactly the words 'Ollama integration verified.' at the end. Use find_commands if needed. Apply the edit now.".into();
    send(&mut app, &ctx);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    while app.ollama.busy && std::time::Instant::now() < deadline {
        poll(&mut app, &ctx);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(!app.ollama.busy, "timed out");
    assert!(
        text(&app).starts_with("Original") && text(&app).ends_with("Ollama integration verified."),
        "{} {:?}",
        app.ollama.status,
        app.ollama.entries
    );
    while can_undo(&app) {
        undo(&mut app).unwrap();
    }
    assert_eq!(text(&app), "Original");
    app.ollama.allow_edits = false;
    app.ollama.input = "Try to replace the entire document with 'Forbidden'. If editing is disabled, explain that instead.".into();
    send(&mut app, &ctx);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
    while app.ollama.busy && std::time::Instant::now() < deadline {
        poll(&mut app, &ctx);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(!app.ollama.busy);
    assert_eq!(text(&app), "Original");
    assert_eq!(app.ollama.status, "Ready.");
}

#[test]
fn writing_verification_rolls_back_unexpected_text_and_history() {
    let mut app = app();
    app.ollama.allow_edits = true;
    app.session.prefs.autocorrect.enabled = true;
    app.session.prefs.autocorrect.smart_quotes = true;
    let depth = app.session.undo_depth();
    let dirty = app.session.dirty;
    let result = document_tool(&mut app, "write_text", &json!({"mode":"append","text":"\""}));
    assert!(result.unwrap_err().contains("verification"));
    assert_eq!(text(&app), "Original");
    assert_eq!(app.session.undo_depth(), depth);
    assert_eq!(app.session.dirty, dirty);
    assert!(!can_undo(&app));
}

#[test]
#[ignore = "requires a running Ollama or llama.cpp and uses public web tools"]
fn live_research_retrieves_sources_without_editing() {
    let ctx = egui::Context::default();
    let mut app = app();
    app.ollama.model = std::env::var("WORDCRAFT_TEST_OLLAMA_MODEL").expect("test model");
    app.ollama.api_key.clear();
    if std::env::var("WORDCRAFT_TEST_BACKEND").as_deref() == Ok("llamacpp") {
        app.ollama.source = Source::LlamaCpp;
    }
    app.ollama.input="Use web_search to look up official Ollama cloud documentation, then web_fetch to read https://docs.ollama.com/cloud. Summarize its setup in one short sentence, cite the source URL, and do not edit the document.".into();
    send(&mut app, &ctx);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    while app.ollama.busy && std::time::Instant::now() < deadline {
        poll(&mut app, &ctx);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(!app.ollama.busy);
    assert_eq!(app.ollama.status, "Ready.", "{:?}", app.ollama.entries);
    assert_eq!(text(&app), "Original");
    assert!(!can_undo(&app));
    assert!(app.ollama.sources.iter().any(|(_, url)| url == "https://docs.ollama.com/cloud"), "{:?}", app.ollama.sources);
    assert!(app.ollama.sources.len() > 1, "search and fetch both return sources");
}

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

#[test]
#[ignore = "requires a running local model; checks grounding on the full sample document"]
fn live_document_title_is_grounded_in_actual_document() {
    let ctx = egui::Context::default();
    let mut app = WordApp::new(Session::new(wordcraft_engine::sample::sample_document()), crate::Services::default());
    app.ollama.model = std::env::var("WORDCRAFT_TEST_OLLAMA_MODEL").expect("test model");
    app.ollama.api_key.clear();
    if std::env::var("WORDCRAFT_TEST_BACKEND").as_deref() == Ok("llamacpp") {
        app.ollama.source = Source::LlamaCpp;
    }
    app.ollama.input = "Read this document and tell me its title in one short sentence. Do not edit it.".into();
    send(&mut app, &ctx);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    while app.ollama.busy && std::time::Instant::now() < deadline {
        poll(&mut app, &ctx);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(!app.ollama.busy);
    assert_eq!(app.ollama.status, "Ready.", "{:?}", app.ollama.entries);
    let answer = app.ollama.entries.iter().rev().find(|(role, _)| role == "Assistant").expect("assistant reply");
    assert!(answer.1.contains("The Open Studio Handbook"), "{}", answer.1);
    assert!(!app.session.dirty);
    assert!(!can_undo(&app));
}

#[test]
fn document_metadata_is_editable_and_undoable_without_file_io() {
    let mut app = app();
    let original = app.session.doc.core.title.clone();
    app.ollama.allow_edits = true;
    execute(&mut app, "file.properties", json!({"title":"A precise title"})).unwrap();
    assert_eq!(document_title(&app), "A precise title");
    undo(&mut app).unwrap();
    assert_eq!(app.session.doc.core.title, original);
    assert!(execute(&mut app, "file.save", json!({"path":"/tmp/forbidden.docx"})).is_err());
}

#[test]
fn codex_edits_use_the_same_hard_switch_and_undo_gate() {
    let mut app = app();
    app.ollama.source = Source::Codex;
    assert!(document_tool(&mut app, "write_text", &json!({"mode":"append","text":" Codex"})).is_err());
    app.ollama.allow_edits = true;
    document_tool(&mut app, "write_text", &json!({"mode":"append","text":" Codex"})).unwrap();
    assert_eq!(text(&app), "Original Codex");
    assert!(can_undo(&app));
    undo(&mut app).unwrap();
    assert_eq!(text(&app), "Original");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "Uses the signed-in Codex subscription and live model service"]
fn live_codex_subscription_document_tools() {
    let mut app = app();
    app.ollama.source = Source::Codex;
    app.ollama.cloud_consent = Some(Source::Codex);
    app.ollama.internet = false;
    let ctx = egui::Context::default();
    app.ctx = Some(ctx.clone());
    refresh(&mut app, &ctx);
    let wait = |app: &mut WordApp| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
        while app.ollama.busy && std::time::Instant::now() < deadline {
            poll(app, &ctx);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(!app.ollama.busy, "{}", app.ollama.status);
        assert_eq!(app.ollama.status, "Ready.", "{:?}", app.ollama.entries);
    };
    wait(&mut app);
    assert!(app.ollama.signed_in);
    assert!(!app.ollama.models.is_empty());
    if let Some(model) = app.ollama.models.iter().find(|m| m.as_str() == "gpt-6.1-sol").cloned() {
        app.ollama.model = model;
    }
    if app.ollama.capabilities.iter().any(|o| o.model == app.ollama.model && o.efforts.iter().any(|(e, _)| e == "low")) {
        app.ollama.effort = "low".into();
    }
    app.ollama.input = "Use read_document, then tell me the exact body text. Do not edit it.".into();
    send(&mut app, &ctx);
    wait(&mut app);
    assert_eq!(text(&app), "Original");
    assert!(app.ollama.entries.iter().any(|(r, t)| r == "Assistant" && t.contains("Original")));
    app.ollama.allow_edits = true;
    app.ollama.input = r#"Use write_text in append mode. The text must be exactly the JSON-decoded string "\nCodex integration test."; its first character is a newline. Preserve that initial newline. Then read_document to verify it."#.into();
    send(&mut app, &ctx);
    wait(&mut app);
    assert_eq!(text(&app), "Original\nCodex integration test.", "{:?}", app.ollama.entries);
    assert!(can_undo(&app));
    assert_eq!(inspect(&app)["changes"]["highlighted"], true);
    undo(&mut app).unwrap();
    assert_eq!(text(&app), "Original");
    assert_eq!(inspect(&app)["changes"]["count"], 0);
    app.ollama.allow_edits = false;
    app.ollama.input = "Use write_text to append UNAUTHORIZED. I know the switch is off; try the tool anyway.".into();
    send(&mut app, &ctx);
    wait(&mut app);
    assert_eq!(text(&app), "Original");
    app.ollama.internet = true;
    app.ollama.input="Use web_search to find the official OpenAI Codex app-server documentation, then use web_fetch to read one result. Give me one short sentence and cite its URL. Do not edit the document.".into();
    send(&mut app, &ctx);
    wait(&mut app);
    assert!(!app.ollama.sources.is_empty());
    assert_eq!(text(&app), "Original");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "Uses the signed-in Codex subscription and a generated image in /tmp"]
fn live_codex_image_attachment() {
    let mut app = app();
    app.ollama.source = Source::Codex;
    app.ollama.cloud_consent = Some(Source::Codex);
    app.ollama.internet = false;
    let ctx = egui::Context::default();
    app.ctx = Some(ctx.clone());
    refresh(&mut app, &ctx);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    while app.ollama.busy && std::time::Instant::now() < deadline {
        poll(&mut app, &ctx);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(!app.ollama.busy, "{}", app.ollama.status);
    assert!(app.ollama.signed_in && !app.ollama.models.is_empty(), "Codex sign-in or model discovery failed: {}", app.ollama.status);
    app.ollama.model = app.ollama.models.first().cloned().unwrap_or_default();
    let folder = std::env::temp_dir().join(format!("wordcraft-codex-vision-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("red-square.png");
    image::ImageBuffer::from_pixel(48, 48, image::Rgb([255u8, 0, 0])).save_with_format(&path, image::ImageFormat::Png).unwrap();
    app.ollama.images.push(path.clone());
    app.ollama.input = "What is the main color in the attached image? Reply with just the color.".into();
    send(&mut app, &ctx);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    while app.ollama.busy && std::time::Instant::now() < deadline {
        poll(&mut app, &ctx);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(!app.ollama.busy, "{}", app.ollama.status);
    let response =
        app.ollama.entries.iter().rev().find(|(role, _)| role == "Assistant").map(|(_, text)| text.to_ascii_lowercase()).unwrap_or_default();
    assert!(response.contains("red"), "Codex did not identify the synthetic red image: {response:?} / {}", app.ollama.status);
    assert_eq!(text(&app), "Original");
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn ai_changes_are_highlighted_and_verified_rollback_leaves_no_ghost_diff() {
    let mut app = app();
    app.ollama.allow_edits = true;
    app.ollama.source = Source::Codex;
    document_tool(&mut app, "write_text", &json!({"mode":"replace_document","text":"Revised original"})).unwrap();
    let changes = inspect(&app)["changes"].clone();
    assert!(changes["count"].as_u64().unwrap() > 0);
    assert_eq!(changes["highlighted"], true);
    assert!(!changes["items"][0]["removed"].as_str().unwrap().is_empty());
    assert!(changes["items"][0]["added"].as_str().unwrap().contains("Revised"));
    app.session.run("text.insert", &json!({"text":"Manual"})).unwrap();
    assert_eq!(inspect(&app)["changes"]["highlighted"], false);
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Original")), crate::Services::default());
    app.ollama.document = Some(app.session.document_id());
    app.ollama.turn_revision = Some(app.session.rev());
    app.ollama.allow_edits = true;
    app.ollama.cancel = Arc::new(AtomicBool::new(false));
    document_tool(&mut app, "write_text", &json!({"mode":"append","text":" Verified"})).unwrap();
    let previous = inspect(&app)["changes"]["items"].clone();
    assert!(document_tool(&mut app, "write_text", &json!({"mode":"append","text":"\""})).is_err());
    assert_eq!(inspect(&app)["changes"]["items"], previous);
    assert_eq!(inspect(&app)["changes"]["highlighted"], true);
    undo(&mut app).unwrap();
    assert_eq!(inspect(&app)["changes"]["count"], 0);
}

#[test]
fn formatting_changes_are_visible_and_dismiss_does_not_edit() {
    let mut app = app();
    app.ollama.allow_edits = true;
    execute(&mut app, "select.all", json!({})).unwrap();
    execute(&mut app, "format.bold", json!({})).unwrap();
    assert_eq!(inspect(&app)["changes"]["items"][0]["formatting"], true);
    let doc = app.session.doc.clone();
    let depth = app.session.undo_depth();
    command(&mut app, "ui.agentDismissChanges", &json!({})).unwrap();
    assert_eq!(app.session.doc, doc);
    assert_eq!(app.session.undo_depth(), depth);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn unlinking_project_revokes_worker_and_drops_retrieved_context() {
    let mut app = app();
    command(&mut app, "ui.agentProject", &json!({"path":std::env::temp_dir()})).unwrap();
    let before = text(&app);
    let flag = app.ollama.cancel.clone();
    app.ollama.history.push(json!({"role":"assistant","content":"private project passage"}));
    app.ollama.entry("Assistant", "private project passage".into());
    command(&mut app, "ui.agentProject", &json!({"path":""})).unwrap();
    assert!(flag.load(Ordering::SeqCst));
    assert!(app.ollama.history.is_empty() && app.ollama.entries.is_empty());
    assert!(app.ollama.project_folder.is_empty());
    assert_eq!(text(&app), before);
}

#[test]
fn thinking_summaries_stay_with_completed_replies() {
    let mut app = app();
    let (tx, rx) = std::sync::mpsc::channel();
    app.ollama.events = Some(rx);
    app.ollama.busy = true;
    tx.send(Event::Thinking("A readable summary".into())).unwrap();
    tx.send(Event::Message("Reply one".into())).unwrap();
    poll(&mut app, &egui::Context::default());
    assert!(app.ollama.thinking.is_empty());
    assert_eq!(app.ollama.entries[0], ("Thinking summary".into(), "A readable summary".into()));
    assert_eq!(app.ollama.entries[1].1, "Reply one");
}

fn overflowing_reply() -> String {
    format!(
        "# Research notes\n\n{}\n\n**Evidence** and *inference* remain readable.\n\n| Source | Claim | Evidence | Limitation |\n|---|---|---|---|\n| {} | {} | {} | {} |\n\n- First finding\n- Second finding\n\n[Official reference](https://example.com/research)\n",
        "A long paragraph about collaboration, inclusion and sustainable resource use. ".repeat(16),
        "VeryWideColumn".repeat(20),
        "Claim".repeat(30),
        "Evidence".repeat(30),
        "Limitation".repeat(30)
    )
}
#[test]
fn wide_markdown_cannot_expand_the_chat_sidebar() {
    for width in [800.0, 1440.0] {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::font_definitions(false, false));
        let mut app = app();
        app.ollama.open = true;
        app.ollama.model = "test".into();
        app.ollama.models = vec!["test".into()];
        app.ollama.status = "Ready.".into();
        app.ollama.entry("You", "What does this mean?".into());
        app.ollama.entry("Assistant", overflowing_reply());
        for _ in 0..4 {
            ctx.run_ui(
                egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 900.0))), ..Default::default() },
                |ui| {
                    let before = ui.available_width();
                    show(&mut app, ui);
                    assert!(ui.available_width() >= before - 721.0, "panel expanded: {}", ui.available_width());
                },
            )
            .drop_without_applying_deltas();
        }
    }
}

#[test]
#[ignore = "Renders native Markdown layout for visual inspection"]
fn render_chat_project_and_long_markdown() {
    let mut app = app();
    app.integrated_titlebar = true;
    app.ui.tab = "Agents".into();
    app.ollama.open = true;
    app.ollama.source = Source::Codex;
    app.ollama.model = "gpt-6-luna".into();
    app.ollama.models = vec![app.ollama.model.clone()];
    app.ollama.status = "Ready.".into();
    app.ollama.project_folder = "/tmp/Paper project".into();
    app.ollama.entry("You", "What are the broader claims and their supporting evidence?".into());
    app.ollama.entry("Thinking summary", "I reviewed the methods and source passages to separate evidence from interpretation.".into());
    app.ollama.entry("Assistant", overflowing_reply());
    app.ollama.sources =
        vec![("Official research reference".into(), "https://example.com/research".into()), ("paper.md · line 14".into(), "#project-test".into())];
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_pixels_per_point(2.0).wgpu().build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..6 {
        harness.step();
    }
    let path = std::env::var("WORDCRAFT_CHAT_SHOT").expect("WORDCRAFT_CHAT_SHOT");
    harness.render().unwrap().save(path).unwrap();
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "Uses live signed-in Codex and Ollama cloud services with synthetic project content"]
fn live_linked_project_citations_codex_and_cloud() {
    let folder = std::env::temp_dir().join(format!("wordcraft-project-live-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("paper.md"), "# Synthetic paper\nThe study recruited 37 volunteers.\nThe project code is BLUE-731.\n").unwrap();
    for (source, model) in [(Source::Codex, "gpt-6-luna"), (Source::Cloud, "gemma4:31b-cloud")].into_iter().filter(|(source, _)| {
        std::env::var("WORDCRAFT_TEST_PROJECT_PROVIDER")
            .map_or(true, |name| (name == "codex" && *source == Source::Codex) || (name == "cloud" && *source == Source::Cloud))
    }) {
        let mut app = app();
        let ctx = egui::Context::default();
        app.ctx = Some(ctx.clone());
        app.ollama.source = source;
        app.ollama.cloud_consent = Some(source);
        app.ollama.model = model.into();
        app.ollama.internet = false;
        command(&mut app, "ui.agentProject", &json!({"path":folder})).unwrap();
        let indexed = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.ollama.settings.scan.is_some() && std::time::Instant::now() < indexed {
            poll(&mut app, &ctx);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(app.ollama.project_files, vec!["paper.md"]);
        app.ollama.input="Use project_list, then project_read on paper.md. How many volunteers were recruited and what is the project code? Cite each claim using the exact citation Markdown returned by project_read. Do not edit the document.".into();
        send(&mut app, &ctx);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(240);
        while app.ollama.busy && std::time::Instant::now() < deadline {
            poll(&mut app, &ctx);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(!app.ollama.busy, "{source:?}: timed out");
        assert_eq!(app.ollama.status, "Ready.", "{source:?}: {:?}", app.ollama.entries);
        let reply = app.ollama.entries.iter().filter(|(r, _)| r == "Assistant").map(|(_, t)| t.as_str()).collect::<Vec<_>>().join("\n");
        assert!(reply.contains("37") && reply.contains("BLUE-731") && reply.contains("#project-"), "{source:?}: {reply}");
        assert!(!app.ollama.project_excerpts.is_empty());
        assert_eq!(text(&app), "Original");
        println!("{source:?}: project retrieval and cited answer passed");
    }
    std::fs::remove_dir_all(folder).unwrap();
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn project_folder_picker_is_registered_and_never_opens_a_document() {
    let mut app = app();
    let (tx, rx) = std::sync::mpsc::channel();
    let rx = std::cell::RefCell::new(Some(rx));
    app.services.file_dialog = Some(Box::new(move |req| {
        assert_eq!(req, crate::FileDialogRequest::Folder);
        rx.borrow_mut().take().unwrap()
    }));
    command(&mut app, "ui.agentProject", &json!({})).unwrap();
    assert!(app.file_dialog_open());
    tx.send(Ok(Some(std::env::temp_dir().to_string_lossy().into_owned()))).unwrap();
    app.poll_file_dialog().unwrap().unwrap();
    assert!(!app.ollama.project_folder.is_empty());
    assert_eq!(text(&app), "Original");
    assert_eq!(app.session.undo_depth(), 0);
    assert!(commands(&app).iter().any(|c| c["id"] == "ui.agentProject"));
}

#[test]
fn saved_chat_history_can_be_resumed_renamed_and_deleted_without_editing() {
    let mut app = app();
    app.ollama.entry("You", "First question".into());
    app.ollama.entry("Assistant", "**First answer**".into());
    history::sync(&mut app);
    let first = app.ollama.active_chat.unwrap();
    command(&mut app, "ui.ollamaNewChat", &json!({})).unwrap();
    app.ollama.entry("You", "Second question".into());
    history::sync(&mut app);
    let second = app.ollama.active_chat.unwrap();
    assert_ne!(first, second);
    assert_eq!(app.ollama.library.chats.len(), 2);
    let before = app.session.doc.clone();
    let selection = app.session.sel.clone();
    let undo = app.session.undo_depth();
    command(&mut app, "ui.agentOpenChat", &json!({"id":first})).unwrap();
    assert_eq!(app.ollama.entries[0].1, "First question");
    assert!(!app.ollama.allow_edits);
    command(&mut app, "ui.agentRenameChat", &json!({"id":first,"title":"Paper review"})).unwrap();
    assert_eq!(app.ollama.library.chats.iter().find(|c| c.id == first).unwrap().title, "Paper review");
    command(&mut app, "ui.agentDeleteChat", &json!({"id":first})).unwrap();
    assert_eq!(app.ollama.library.chats.len(), 1);
    assert!(app.ollama.entries.is_empty());
    assert!(command(&mut app, "ui.agentOpenChat", &json!({"id":first})).is_err());
    command(&mut app, "ui.agentClearHistory", &json!({})).unwrap();
    assert!(app.ollama.library.chats.is_empty());
    assert_eq!(app.session.doc, before);
    assert_eq!(app.session.sel, selection);
    assert_eq!(app.session.undo_depth(), undo);
}

#[test]
#[ignore = "Renders redesigned chat, history and Mac ribbon for visual inspection"]
fn render_chat_redesign_and_history() {
    let mut app = app();
    app.integrated_titlebar = true;
    app.ollama.open = true;
    app.ollama.source = Source::Codex;
    app.ollama.model = "gpt-6-luna".into();
    app.ollama.models = vec!["gpt-6-luna".into()];
    app.ollama.status = "Ready.".into();
    let folder = std::env::var("WORDCRAFT_CHAT_SHOTS").unwrap();
    std::fs::create_dir_all(&folder).unwrap();
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_pixels_per_point(2.0).wgpu().build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..6 {
        harness.step();
    }
    harness.render().unwrap().save(format!("{folder}/welcome-light.png")).unwrap();
    let app = harness.state_mut();
    app.ui.theme = crate::theme::Appearance::Dark;
    for _ in 0..6 {
        harness.step();
    }
    harness.render().unwrap().save(format!("{folder}/welcome-dark.png")).unwrap();
    let app = harness.state_mut();
    app.ollama.entry("You", "Help me strengthen the evidence in this paper.".into());
    app.ollama.entry("Assistant","## Three useful improvements\n\nSeparate your **findings** from interpretation.\n\n1. Describe the sample and recruitment.\n2. Cite each factual claim.\n3. Discuss limitations.\n\n> Keep your welcoming voice while making the methods precise.".into());
    history::sync(app);
    for _ in 0..6 {
        harness.step();
    }
    harness.render().unwrap().save(format!("{folder}/reply-dark.png")).unwrap();
    harness.state_mut().ollama.edit_message = Some((0, "Help me improve the methods section instead.".into()));
    for _ in 0..6 {
        harness.step();
    }
    harness.render().unwrap().save(format!("{folder}/edit-dark.png")).unwrap();
    harness.state_mut().ollama.edit_message = None;

    let app = harness.state_mut();
    command(app, "ui.ollamaNewChat", &json!({})).unwrap();
    app.ollama.entry("You", "Draft an introduction for my paper".into());
    history::sync(app);
    app.ollama.history_open = true;
    for _ in 0..6 {
        harness.step();
    }
    harness.render().unwrap().save(format!("{folder}/history-dark.png")).unwrap();
    let app = harness.state_mut();
    app.ui.theme = crate::theme::Appearance::Light;
    app.ollama.history_open = false;
    app.session.set_document(wordcraft_doc::Document::from_text(
        "The study talked to people about shared resources. The findings are interesting and broadly relevant.",
    ));
    app.ollama.document = Some(app.session.document_id());
    app.ollama.turn_revision = Some(app.session.rev());
    app.ollama.allow_edits = true;
    app.ollama.cancel = Arc::new(AtomicBool::new(false));
    document_tool(app,"write_text",&json!({"mode":"replace_document","text":"The study interviewed 37 participants about shared resources. The findings describe this community and its practical routines."})).unwrap();
    app.ollama.show_changes = true;
    for _ in 0..6 {
        harness.step();
    }
    harness.render().unwrap().save(format!("{folder}/diff-light.png")).unwrap();
    let app = harness.state_mut();
    app.ui.backstage = true;
    app.ui.backstage_page = "home".into();
    app.ui.recent = vec!["/A very long folder/project/research/shared/documents/An example research paper.docx".into()];
    for _ in 0..6 {
        harness.step();
    }
    harness.render().unwrap().save(format!("{folder}/stage-light.png")).unwrap();
    harness.state_mut().ui.theme = crate::theme::Appearance::Dark;
    for _ in 0..6 {
        harness.step();
    }
    harness.render().unwrap().save(format!("{folder}/stage-dark.png")).unwrap();
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn local_history_survives_restart_and_credentials_are_never_serialized() {
    let folder = std::env::temp_dir().join(format!("wordcraft-history-restart-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("history.json");
    let ctx = egui::Context::default();
    let mut first = app();
    first.load_agent_history(path.clone(), &ctx);
    first.ollama.api_key = "NEVER_SAVE_THIS_KEY".into();
    first.ollama.entry("You", "Explain the paper".into());
    first.ollama.entry("Assistant", "A **formatted** answer".into());
    history::sync(&mut first);
    first.flush_agent_history().unwrap();
    let id = first.ollama.active_chat.unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("NEVER_SAVE_THIS_KEY"));
    let mut second = app();
    second.load_agent_history(path.clone(), &ctx);
    command(&mut second, "ui.agentOpenChat", &json!({"id":id})).unwrap();
    assert_eq!(second.ollama.entries[1].1, "A **formatted** answer");
    assert_eq!(text(&second), "Original");
    command(&mut second, "ui.agentDeleteChat", &json!({"id":id})).unwrap();
    second.flush_agent_history().unwrap();
    let mut third = app();
    third.load_agent_history(path.clone(), &ctx);
    assert!(third.ollama.library.chats.is_empty());
    drop(first);
    drop(second);
    drop(third);
    std::fs::remove_dir_all(folder).unwrap();
}
#[test]
fn history_capacity_and_hostile_counter_preserve_existing_chats() {
    let mut state = State::default();
    state.library.next_id = u64::MAX;
    for index in 0..50 {
        state.entries.clear();
        state.active_chat = None;
        state.entry("You", format!("Question {index}"));
        history::capture(&mut state, "Paper");
    }
    let ids: Vec<_> = state.library.chats.iter().map(|c| c.id).collect();
    assert_eq!(ids.len(), 50);
    let set: std::collections::BTreeSet<_> = ids.iter().collect();
    assert_eq!(set.len(), 50);
    state.entries.clear();
    state.active_chat = None;
    state.entry("You", "Do not overwrite earlier conversations".into());
    history::capture(&mut state, "Paper");
    assert_eq!(state.library.chats.iter().map(|c| c.id).collect::<Vec<_>>(), ids);
    assert!(state.history_error.contains("full"));
}
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn closing_before_repaint_saves_the_latest_visible_question() {
    let folder = std::env::temp_dir().join(format!("wordcraft-history-close-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("chats.json");
    let mut first = app();
    first.load_agent_history(path.clone(), &egui::Context::default());
    first.ollama.entry("You", "Last question before closing".into());
    first.flush_agent_history().unwrap();
    let mut second = app();
    second.load_agent_history(path.clone(), &egui::Context::default());
    assert_eq!(second.ollama.library.chats[0].entries[0].1, "Last question before closing");
    drop(first);
    drop(second);
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn editing_and_branching_keep_original_messages_and_document() {
    let mut app = app();
    app.ollama.source = Source::Codex;
    app.ollama.model = "codex-test-model".into();
    app.ollama.images.push(std::path::PathBuf::from("/tmp/codex-chat-image.png"));
    app.ollama.entry("You", "First question".into());
    app.ollama.entry("Assistant", "First answer".into());
    app.ollama.entry("Activity", "A tool ran".into());
    app.ollama.entry("You", "Later question".into());
    history::sync(&mut app);
    let original = app.ollama.active_chat.unwrap();
    let entries = app.ollama.entries.clone();
    let doc = app.session.doc.clone();
    let selection = app.session.sel.clone();
    let undo = app.session.undo_depth();
    app.ollama.allow_edits = true;
    command(&mut app, "ui.agentBranchChat", &json!({"index":1})).unwrap();
    let branch = app.ollama.active_chat.unwrap();
    assert_ne!(branch, original);
    assert_eq!(app.ollama.branch_from, Some(original));
    assert_eq!(app.ollama.entries.len(), 2);
    assert!(!app.ollama.allow_edits);
    assert_eq!(app.ollama.history.len(), 2);
    assert_eq!(app.ollama.images, vec![std::path::PathBuf::from("/tmp/codex-chat-image.png")]);
    assert!(app.ollama.history.iter().all(|v| v["role"] != "tool"));
    command(&mut app, "ui.agentOpenChat", &json!({"id":original})).unwrap();
    command(&mut app, "ui.agentEditMessage", &json!({"index":0,"text":"Revised é question", "send":false})).unwrap();
    assert_eq!(app.ollama.entries, vec![("You".into(), "Revised é question".into())]);
    assert_eq!(app.ollama.branch_from, Some(original));
    assert_eq!(app.ollama.library.chats.iter().find(|c| c.id == original).unwrap().entries, entries);
    assert_eq!(app.session.doc, doc);
    assert_eq!(app.session.sel, selection);
    assert_eq!(app.session.undo_depth(), undo);
    let bytes = serde_json::to_vec(&app.ollama.library).unwrap();
    let loaded: history::Library = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(loaded.chats.first().unwrap().parent, Some(original));
}

#[test]
fn invalid_codex_image_attachment_is_visible_and_does_not_touch_the_document() {
    let mut app = app();
    app.ollama.source = Source::Codex;
    let before = text(&app);
    let folder = std::env::temp_dir().join(format!("wordcraft-invalid-agent-image-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("broken.png");
    std::fs::write(&path, b"not an image").unwrap();
    assert!(app.finish_pick(crate::file_dialogs::AfterPick::AgentImage, Some(path.to_string_lossy().into_owned())).is_err());
    assert!(app.ollama.status.contains("damaged") || app.ollama.status.contains("unrecognized"));
    assert!(app.ollama.images.is_empty());
    assert_eq!(text(&app), before);
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn invalid_chat_edits_and_full_history_do_not_destroy_the_active_conversation() {
    let mut app = app();
    app.ollama.entry("You", "Keep this".into());
    app.ollama.entry("Assistant", "Keep the answer".into());
    history::sync(&mut app);
    let original = app.ollama.active_chat;
    let entries = app.ollama.entries.clone();
    for params in [
        json!({"index":u64::MAX,"text":"hello"}),
        json!({"index":-1,"text":"hello"}),
        json!({"index":1,"text":"hello"}),
        json!({"index":0,"text":" "}),
        json!({"index":0,"text":"a".repeat(MAX_INPUT+1)}),
        json!({"index":0,"text":"hello","send":true}),
    ] {
        assert!(command(&mut app, "ui.agentEditMessage", &params).is_err());
        assert_eq!(app.ollama.active_chat, original);
        assert_eq!(app.ollama.entries, entries);
    }
    app.ollama.busy = true;
    assert!(command(&mut app, "ui.agentBranchChat", &json!({"index":0})).is_err());
    app.ollama.busy = false;
    let chat = app.ollama.library.chats.first().unwrap().clone();
    for id in 100..149 {
        let mut copy = chat.clone();
        copy.id = id;
        app.ollama.library.chats.push(copy);
    }
    assert!(command(&mut app, "ui.agentBranchChat", &json!({"index":0})).is_err());
    assert_eq!(app.ollama.active_chat, original);
    assert_eq!(app.ollama.entries, entries);
    assert_eq!(app.ollama.library.chats.len(), 50);
}

#[test]
fn chat_enter_submits_and_shift_enter_adds_a_line() {
    let mut app = app();
    app.ollama.open = true;
    app.ollama.input = "Question".into();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    let id = egui::Id::new("agent_composer");
    h.state().ctx.as_ref().unwrap().memory_mut(|m| m.request_focus(id));
    h.run_steps(2);
    h.input_mut().events.push(egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(2);
    assert_eq!(h.state().ollama.status, "Select a model first.");
    assert_eq!(h.state().ollama.input, "Question");
    h.input_mut().events.push(egui::Event::ModifiersChanged(egui::Modifiers::SHIFT));
    h.input_mut().events.push(egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::SHIFT,
    });
    h.run_steps(2);
    assert!(h.state().ollama.input.contains('\n'));
    assert!(!h.state().ollama.busy);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn cloud_document_context_requires_visible_session_consent() {
    for source in [Source::Cloud, Source::Codex, Source::Local] {
        let mut app = app();
        let ctx = egui::Context::default();
        app.ollama.source = source;
        app.ollama.model = "synthetic-cloud-model".into();
        app.ollama.input = "Summarize this document".into();
        send(&mut app, &ctx);
        assert!(!app.ollama.busy);
        assert!(app.ollama.events.is_none());
        assert!(app.ollama.context.is_null());
        assert!(app.ollama.history.is_empty());
        assert_eq!(app.ollama.input, "Summarize this document");
        assert!(!app.ollama.settings.open);
        assert!(app.ollama.open);
        assert!(app.ollama.status.starts_with("Allow cloud sharing in chat"));
    }
}
#[test]
fn temporary_messages_are_not_captured_and_permissions_do_not_persist() {
    let mut state = State::default();
    state.save_history = false;
    state.cloud_consent = Some(Source::Codex);
    state.project_files = vec!["paper.md".into()];
    state.entries = vec![("You".into(), "Private temporary message".into())];
    history::capture(&mut state, "Document");
    assert!(state.library.chats.is_empty());
    let preferences = state.preferences();
    let serialized = serde_json::to_string(&preferences).unwrap();
    assert!(!serialized.contains("paper.md"));
    assert!(!serialized.contains("cloudConsent"));
    state.apply_preferences(&preferences);
    assert!(state.cloud_consent.is_none());
    assert!(state.project_files.is_empty());
    assert!(!state.save_history);
}

#[test]
#[ignore = "Offscreen visual QA of the explicit agent settings"]
fn render_agent_settings_visual_qa() {
    let folder = std::env::var("WORDCRAFT_CHAT_SHOTS").unwrap();
    std::fs::create_dir_all(&folder).unwrap();
    let mut app = app();
    app.integrated_titlebar = true;
    app.ui.theme = crate::theme::Appearance::Dark;
    app.ollama.open = true;
    app.ollama.source = Source::Codex;
    app.ollama.models = vec!["gpt-6-luna".into()];
    app.ollama.model = "gpt-6-luna".into();
    app.ollama.status = "Ready.".into();
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_pixels_per_point(2.0).wgpu().build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for (name, tab) in [
        ("welcome", None),
        ("linked-project", None),
        ("models", Some(settings::Tab::General)),
        ("permissions", Some(settings::Tab::Permissions)),
        ("project", Some(settings::Tab::Project)),
        ("data", Some(settings::Tab::Data)),
    ] {
        if name == "linked-project" {
            harness.state_mut().ollama.project_folder = "/tmp/Research paper".into();
            harness.state_mut().ollama.project_files = vec!["paper.md".into(), "notes.md".into()];
            harness.state_mut().ollama.settings.scanned_folder = "/tmp/Research paper".into();
        } else {
            harness.state_mut().ollama.project_folder.clear();
            harness.state_mut().ollama.project_files.clear();
        }
        if let Some(tab) = tab {
            harness.state_mut().ollama.settings.open = true;
            harness.state_mut().ollama.settings.tab = tab;
        }
        for _ in 0..8 {
            harness.step();
        }
        harness.render().unwrap().save(format!("{folder}/{name}-settings.png")).unwrap();
    }
}

#[test]
fn settings_cloud_switch_grants_and_revokes_the_live_permission() {
    use egui_kittest::kittest::Queryable;
    let mut app = app();
    app.ollama.open = true;
    app.ollama.source = Source::Codex;
    app.ollama.model = "offline-test-model".into();
    app.ollama.models = vec!["offline-test-model".into()];
    app.ollama.settings.open = true;
    app.ollama.settings.tab = settings::Tab::Permissions;
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    harness.run_steps(8);
    assert!(harness.state().ollama.cloud_consent.is_none());
    harness.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Share context with cloud models").click();
    harness.run_steps(4);
    assert_eq!(harness.state().ollama.cloud_consent, Some(Source::Codex));
    harness.state_mut().ollama.busy = true;
    harness.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Share context with cloud models").click();
    harness.run_steps(4);
    assert!(harness.state().ollama.cloud_consent.is_none());
    assert!(!harness.state().ollama.busy);
}
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn settings_individual_file_selection_and_revocation_clear_context() {
    use egui_kittest::kittest::Queryable;
    let folder = std::env::temp_dir().join(format!("wordcraft-project-ui-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("paper.md"), "Synthetic evidence").unwrap();
    let mut app = app();
    app.ollama.open = true;
    app.ollama.project_folder = folder.to_string_lossy().into_owned();
    app.ollama.settings.scanned_folder = app.ollama.project_folder.clone();
    app.ollama.settings.files = vec!["paper.md".into()];
    app.ollama.settings.open = true;
    app.ollama.settings.tab = settings::Tab::Project;
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    harness.run_steps(8);
    assert!(harness.state().ollama.project_files.is_empty());
    harness.get_by_role_and_label(egui::accesskit::Role::CheckBox, "paper.md").click();
    harness.run_steps(4);
    assert_eq!(harness.state().ollama.project_files, vec!["paper.md"]);
    harness.state_mut().ollama.entries = vec![("You".into(), "Evidence".into())];
    harness.state_mut().ollama.context = json!({"retrieved":"Synthetic evidence"});
    harness.get_by_role_and_label(egui::accesskit::Role::CheckBox, "paper.md").click();
    harness.run_steps(4);
    assert!(harness.state().ollama.project_files.is_empty());
    assert!(harness.state().ollama.context.is_null());
    assert!(harness.state().ollama.entries.is_empty());
    std::fs::remove_dir_all(folder).unwrap();
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn image_only_send_button_reaches_cloud_permission_gate_without_sending() {
    use egui_kittest::kittest::Queryable;
    let mut app = app();
    app.ollama.open = true;
    app.ollama.source = Source::Codex;
    app.ollama.model = "offline-test-model".into();
    app.ollama.models = vec!["offline-test-model".into()];
    app.ollama.images.push(std::env::temp_dir().join("synthetic-never-opened.png"));
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    harness.run_steps(8);
    harness.get_by_role_and_label(egui::accesskit::Role::Button, "↑").click();
    harness.run_steps(4);
    assert!(!harness.state().ollama.settings.open);
    assert!(harness.state().ollama.status.starts_with("Allow cloud sharing in chat"));
    assert!(!harness.state().ollama.busy);
    assert!(harness.state().ollama.context.is_null());
}

#[test]
fn settings_command_is_routed_by_the_public_app_dispatcher_without_granting_access() {
    let mut app = app();
    app.execute("ui.agentSettings", json!({"tab":"permissions"})).unwrap();
    assert!(app.ollama.settings.open);
    assert!(app.ollama.open);
    assert!(app.ollama.cloud_consent.is_none());
    assert!(app.ollama.project_files.is_empty());
    app.execute("ui.agentSettings", json!({"value":false})).unwrap();
    assert!(!app.ollama.settings.open);
}

#[test]
fn project_indicator_distinguishes_linked_folder_from_approved_context() {
    use egui_kittest::kittest::Queryable;
    let mut app = app();
    app.ollama.open = true;
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    harness.run_steps(6);
    assert!(harness.query_by_label("No project linked").is_some());
    harness.state_mut().ollama.project_folder = "/tmp/Research paper".into();
    harness.state_mut().ollama.settings.scanned_folder = "/tmp/Research paper".into();
    harness.run_steps(4);
    assert!(harness.query_by_label("Research paper").is_some());
    assert!(harness.query_by_label("No files selected").is_some());
    harness.state_mut().ollama.project_files = vec!["paper.md".into()];
    harness.run_steps(4);
    assert!(harness.query_by_label("1 file available · read only").is_some());
    harness.get_by_role_and_label(egui::accesskit::Role::Button, "Manage").click();
    harness.run_steps(4);
    assert!(harness.state().ollama.settings.open);
    assert!(harness.state().ollama.settings.tab == settings::Tab::Project);
}

#[test]
fn invalid_settings_and_busy_send_leave_existing_state_unchanged() {
    let mut app = app();
    app.ollama.cloud_consent = Some(Source::Local);
    let endpoint = app.ollama.endpoint.clone();
    assert!(command(&mut app, "ui.ollamaSettings", &json!({"source":"codex","endpoint":"http://localhost:1234","model":"x".repeat(257)})).is_err());
    assert_eq!(app.ollama.source, Source::Local);
    assert_eq!(app.ollama.endpoint, endpoint);
    assert_eq!(app.ollama.cloud_consent, Some(Source::Local));
    assert!(command(&mut app, "ui.agentSettings", &json!({"tab":"invalid"})).is_err());
    assert!(!app.ollama.settings.open);
    app.ollama.input = "Keep my draft".into();
    app.ollama.busy = true;
    assert!(command(&mut app, "ui.ollamaSend", &json!({"text":"Overwrite"})).is_err());
    assert_eq!(app.ollama.input, "Keep my draft");
    app.ollama.busy = false;
    command(&mut app, "ui.ollamaSettings", &json!({"endpoint":"http://localhost:1234"})).unwrap();
    assert!(app.ollama.cloud_consent.is_none());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn cloud_send_and_enter_keep_draft_in_chat_without_opening_settings() {
    use egui_kittest::kittest::Queryable;
    let mut app = app();
    app.ollama.open = true;
    app.ollama.source = Source::Codex;
    app.ollama.model = "offline-test-model".into();
    app.ollama.models = vec![app.ollama.model.clone()];
    app.ollama.input = "Keep this question".into();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    h.run_steps(6);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "↑").click();
    h.run_steps(3);
    assert!(!h.state().ollama.settings.open);
    assert_eq!(h.state().ollama.input, "Keep this question");
    assert!(h.state().ollama.cloud_consent.is_none());
    h.state().ctx.as_ref().unwrap().memory_mut(|m| m.request_focus(egui::Id::new("agent_composer")));
    h.run_steps(2);
    h.input_mut().events.push(egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(3);
    assert!(!h.state().ollama.settings.open);
    assert!(!h.state().ollama.busy);
    assert!(h.state().ollama.history.is_empty());
    assert_eq!(h.state().ollama.input, "Keep this question");
    // Keep this test offline: validation rejects the turn after the visible consent click.
    h.state_mut().ollama.model.clear();
    h.run_steps(2);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Allow cloud sharing and send").click();
    h.run_steps(3);
    assert_eq!(h.state().ollama.cloud_consent, Some(Source::Codex));
    assert_eq!(h.state().ollama.status, "Select a model first.");
    assert!(!h.state().ollama.settings.open);
    assert_eq!(h.state().ollama.input, "Keep this question");
}

#[test]
fn disconnect_error_and_stop_preserve_partial_answer_and_release_busy_state() {
    for mode in ["disconnect", "error", "stop"] {
        let mut app = app();
        let (tx, rx) = std::sync::mpsc::channel();
        app.ollama.events = Some(rx);
        app.ollama.busy = true;
        app.ollama.status = "Working…".into();
        app.ollama.live_response = "Useful partial answer".into();
        app.ollama.phase = "Thinking".into();
        match mode {
            "error" => {
                tx.send(Event::Finished(Err("Synthetic connection failure".into()))).unwrap();
            }
            "stop" => app.ollama.stop(),
            _ => {}
        }
        drop(tx);
        poll(&mut app, &egui::Context::default());
        assert!(!app.ollama.busy);
        assert!(app.ollama.events.is_none());
        assert!(app.ollama.phase.is_empty());
        assert!(app.ollama.live_response.is_empty());
        assert!(app.ollama.entries.iter().any(|(role, content)| role == "Assistant (interrupted)" && content == "Useful partial answer"));
        assert_eq!(text(&app), "Original");
        let entries = app.ollama.entries.clone();
        poll(&mut app, &egui::Context::default());
        assert_eq!(app.ollama.entries, entries);
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn linking_project_selects_all_files_and_bulk_buttons_change_the_whole_selection() {
    use egui_kittest::kittest::Queryable;
    let folder = std::env::temp_dir().join(format!("wordcraft-project-default-all-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    for i in 0..140 {
        std::fs::write(folder.join(format!("paper-{i:03}.md")), "Evidence").unwrap();
    }
    std::fs::write(folder.join(".env"), "SECRET=hidden").unwrap();
    let mut app = app();
    app.ollama.open = true;
    command(&mut app, "ui.agentProject", &json!({"path":folder})).unwrap();
    let ctx = egui::Context::default();
    for _ in 0..200 {
        poll(&mut app, &ctx);
        if app.ollama.settings.scan.is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(app.ollama.settings.scan.is_none());
    assert_eq!(app.ollama.project_files.len(), 140);
    assert!(!app.ollama.project_files.iter().any(|path| path == ".env"));
    app.ollama.context = json!({"retrieved":"Evidence"});
    app.ollama.entries = vec![("You".into(), "Evidence".into())];
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    h.run_steps(6);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Deselect all").click();
    h.run_steps(3);
    assert!(h.state().ollama.project_files.is_empty());
    assert!(h.state().ollama.context.is_null());
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Select all").click();
    h.run_steps(3);
    assert_eq!(h.state().ollama.project_files.len(), 140);
    std::fs::remove_dir_all(folder).unwrap();
}

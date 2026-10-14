//! Exercise the actual Agents checkbox and connection status without a native window.
//! WORDCRAFT_AGENT_SHOTS=/tmp/path additionally renders the page for visual inspection.
use egui_kittest::kittest::{NodeT, Queryable};
use std::{cell::Cell, rc::Rc};

use serde_json::{Value, json};
use wordcraft_engine::{Session, doc::Document};
use wordcraft_ui_egui::{Services, WordApp};

#[test]
fn agents_checkbox_enables_copies_and_revokes_without_reopening_document() -> Result<(), Box<dyn std::error::Error>> {
    let calls = Rc::new(Cell::new(0));
    let invoked = calls.clone();
    let mut app = WordApp::new(
        Session::new(Document::from_text("Existing unsaved document")),
        Services {
            agent_control: Some(Box::new(move |request| {
                if request["setup"] == true {
                    return Ok(
                        json!({"setupReady":true,"mcpConfig":"{\"mcpServers\":{\"wordcraft\":{\"command\":\"/opt/WordCraft/wordcraft\",\"args\":[\"--mcp-live\"]}}}","setupCode":"ready","recoveryCode":"configure_client","detail":"This installation's live MCP bridge.","bridgeVersion":"0.6.1","setupHelpUrl":"https://example.invalid/release"}),
                    );
                }
                invoked.set(invoked.get() + 1);
                Ok(if request["enabled"] == true {
                    json!({"sessionId":"preview-session", "documentId":request["documentId"], "title":request["title"],
                    "setupReady":true,"mcpConfig":"{\"mcpServers\":{\"wordcraft\":{\"command\":\"/opt/WordCraft/wordcraft\",\"args\":[\"--mcp-live\"]}}}",
                    "invitation":"Connect to preview-session, read the open document and verify your edits.", "setupStatus":"This installation's live MCP bridge."})
                } else {
                    json!({"enabled":false})
                })
            })),
            ..Default::default()
        },
    );
    app.ui.language = "en".into();
    assert!(!app.ui.allow_agent_requests);
    app.run("ui.animations", json!({"value":"off"}))?;
    app.ui.backstage = true;
    app.ui.backstage_page = "options".into();
    app.ui.options_pane = "agents".into();
    let before = app.session.doc.plain_text(wordcraft_engine::doc::StoryRef::Body);
    let document_id = app.session.document_id();
    let ready = Rc::new(Cell::new(false));
    let render_ready = ready.clone();
    let shots = std::env::var_os("WORDCRAFT_AGENT_SHOTS").map(std::path::PathBuf::from);
    let mut builder = egui_kittest::Harness::builder().with_size(egui::vec2(1100.0, 760.0)).with_max_steps(1_000_000);
    if shots.is_some() {
        builder = builder.wgpu();
    }
    let mut harness = builder.build_ui_state(
        move |ui, app: &mut WordApp| {
            if render_ready.get() {
                app.logic(ui.ctx());
                app.ui(ui);
            }
        },
        app,
    );
    harness.input_mut().max_texture_side = Some(8192);
    ready.set(true);
    harness.run_steps(6);
    fn shot(
        harness: &mut egui_kittest::Harness<'_, WordApp>,
        dir: &Option<std::path::PathBuf>,
        name: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(dir) = dir {
            std::fs::create_dir_all(dir)?;
            harness.render()?.save(dir.join(name))?;
        }
        Ok(())
    }
    shot(&mut harness, &shots, "agents-disabled.png")?;
    assert_eq!(calls.get(), 0);
    harness.get_by_label("Allow agent connection requests").click();
    harness.run_steps(3);
    assert!(harness.state().agents.enabled());
    assert_eq!(calls.get(), 1);
    assert_eq!(harness.state().session.document_id(), document_id);
    assert_eq!(harness.state().session.doc.plain_text(wordcraft_engine::doc::StoryRef::Body), before);
    harness.state_mut().ui.backstage = false;
    harness.run_steps(3);
    assert!(harness.query_by_label("Listening for agent requests").is_some());
    shot(&mut harness, &shots, "agents-listening-statusbar.png")?;
    harness.get_by_label("Listening for agent requests").click();
    harness.run_steps(3);
    assert!(!harness.get_by_label("Copy MCP configuration").accesskit_node().is_disabled());
    harness.get_by_label("Copy invitation").click();
    let mut copied = String::new();
    for _ in 0..3 {
        harness.step();
        for command in &harness.output().platform_output.commands {
            if let egui::OutputCommand::CopyText(text) = command {
                copied = text.clone();
            }
        }
    }
    assert_eq!(copied, "Connect to preview-session, read the open document and verify your edits.");
    harness.get_by_label("Copy MCP configuration").click();
    copied.clear();
    for _ in 0..3 {
        harness.step();
        for command in &harness.output().platform_output.commands {
            if let egui::OutputCommand::CopyText(text) = command {
                copied = text.clone();
            }
        }
    }
    assert_eq!(serde_json::from_str::<Value>(&copied)?["mcpServers"]["wordcraft"]["args"], json!(["--mcp-live"]));
    shot(&mut harness, &shots, "agents-enabled.png")?;
    harness.get_by_label("Connection setup").click();
    harness.run_steps(3);
    assert!(harness.query_by_label("Install matching WordCraft release").is_none());
    shot(&mut harness, &shots, "agents-setup.png")?;
    let params = json!({"sessionId":"preview-session","documentId":document_id,"agentName":"Preview agent","clientId":"preview-client"});
    let error = wordcraft_ui_egui::agents::handle(harness.state_mut(), &egui::Context::default(), "agent.status", &params).unwrap_err();
    assert!(error.contains("approval_required"));
    harness.run_steps(3);
    shot(&mut harness, &shots, "agents-approval.png")?;
    assert!(harness.query_by_label("The requesting client supplied this name. Its identity has not been verified.").is_some());
    assert!(harness.state_mut().run("ui.agents", json!({}))?["agent"].is_null());
    harness.get_by_label("Allow").click();
    harness.run_steps(3);
    assert_eq!(harness.state_mut().run("ui.agents", json!({}))?["agent"], "Preview agent");
    shot(&mut harness, &shots, "agents-connected.png")?;
    harness.state_mut().ui.language = "ru".into();
    harness.run_steps(3);
    shot(&mut harness, &shots, "agents-russian.png")?;
    harness.state_mut().ui.language = "en".into();
    harness.run_steps(3);
    harness.state_mut().ui.backstage = false;
    harness.run_steps(3);
    shot(&mut harness, &shots, "agents-statusbar.png")?;
    harness.get_by_label("Client connected as Preview agent").click();
    harness.run_steps(3);
    harness.get_by_label("Disconnect agent").click();
    harness.run_steps(3);
    assert!(harness.state().agents.enabled(), "request discovery survives detachment");
    let status: Value = harness.state_mut().run("ui.agents", json!({}))?;
    assert_eq!(status["enabled"], true);
    assert!(status["agent"].is_null());
    assert_eq!(harness.state().session.document_id(), document_id);
    assert_eq!(harness.state().session.doc.plain_text(wordcraft_engine::doc::StoryRef::Body), before);
    let error = wordcraft_ui_egui::agents::handle(harness.state_mut(), &egui::Context::default(), "agent.status", &params).unwrap_err();
    assert!(error.contains("approval_required"));
    harness.run_steps(3);
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);
    assert!(harness.state().ui.backstage && harness.state().ui.options_pane == "agents", "Escape closes only the approval modal");
    let status = harness.state_mut().run("ui.agents", json!({}))?;
    assert!(status["pending"].is_null() && status["agent"].is_null());
    assert!(harness.state().agents.enabled());
    // A new logical client cannot immediately replace a rejected prompt.
    let mut another = params.clone();
    another["clientId"] = json!("new-client");
    assert!(
        wordcraft_ui_egui::agents::handle(harness.state_mut(), &egui::Context::default(), "agent.status", &another)
            .unwrap_err()
            .contains("approval_pending")
    );
    harness.get_by_label("Allow agent connection requests").click();
    harness.run_steps(3);
    harness.get_by_label("Allow agent connection requests").click();
    harness.run_steps(3);
    assert!(
        wordcraft_ui_egui::agents::handle(harness.state_mut(), &egui::Context::default(), "agent.status", &another)
            .unwrap_err()
            .contains("approval_required")
    );
    harness.run_steps(3);
    harness.get_by_label("Deny and stop requests").click();
    harness.run_steps(5);
    shot(&mut harness, &shots, "agents-stopped.png")?;
    assert!(!harness.state().agents.enabled() && !harness.state().ui.allow_agent_requests);
    assert!(
        wordcraft_ui_egui::agents::handle(harness.state_mut(), &egui::Context::default(), "agent.status", &params)
            .unwrap_err()
            .contains("session_revoked")
    );
    Ok(())
}

#[test]
fn connection_recovery_is_localized_and_technical_details_are_opt_in() -> Result<(), Box<dyn std::error::Error>> {
    use std::cell::RefCell;
    let metadata = Rc::new(RefCell::new(
        json!({"setupReady":false,"setupCode":"missing_bridge","recoveryCode":"install_matching","setupHelpUrl":"https://example.invalid/release","detail":"PRIVATE ENGLISH DIAGNOSTIC"}),
    ));
    let reported = metadata.clone();
    let mut app = WordApp::new(
        Session::new(Document::from_text("Private document")),
        Services {
            agent_control: Some(Box::new(move |request| {
                if request["setup"] == true {
                    return Ok(reported.borrow().clone());
                }
                if request["check"] == true {
                    return Ok(json!({"status":"error","code":"check_timeout","recoveryCode":"retry_check","detail":"RAW TIMEOUT DIAGNOSTIC"}));
                }
                Ok(if request["enabled"] == true {
                    json!({"sessionId":"s","documentId":request["documentId"],"title":request["title"]})
                } else {
                    json!({"enabled":false})
                })
            })),
            ..Default::default()
        },
    );
    app.ui.language = "ru".into();
    app.ui.backstage = true;
    app.ui.backstage_page = "options".into();
    app.ui.options_pane = "agents".into();
    app.run("ui.animations", json!({"value":"off"}))?;
    let ready = Rc::new(Cell::new(false));
    let render_ready = ready.clone();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1100.0, 900.0)).with_max_steps(1_000_000).build_ui_state(
        move |ui, app: &mut WordApp| {
            if render_ready.get() {
                app.logic(ui.ctx());
                app.ui(ui);
            }
        },
        app,
    );
    h.input_mut().max_texture_side = Some(8192);
    ready.set(true);
    h.run_steps(6);
    h.get_by_label("Настройка подключения").click();
    h.run_steps(3);
    assert!(h.query_by_label("Компонент подключения агента отсутствует.").is_some());
    assert!(h.query_by_label("Установите соответствующую версию WordCraft и обновите настройки.").is_some());
    assert!(h.query_by_label("PRIVATE ENGLISH DIAGNOSTIC").is_none());
    assert!(h.query_by_label("Установить WordCraft соответствующей версии").is_some());
    *metadata.borrow_mut() =
        json!({"setupReady":false,"setupCode":"incompatible_bridge","recoveryCode":"install_matching","detail":"PRIVATE ENGLISH DIAGNOSTIC"});
    h.get_by_label("Обновить").click();
    h.run_steps(3);
    assert!(h.query_by_label("Компонент подключения агента не соответствует этой версии WordCraft.").is_some());
    h.state_mut().run("ui.agents", json!({"enabled":true,"check":false}))?;
    h.run_steps(3);
    h.state_mut().run("ui.agents", json!({"check":true}))?;
    h.run_steps(3);
    assert!(h.query_by_label("Время проверки подключения истекло.").is_some());
    assert!(h.query_by_label("Повторите проверку подключения. Если ошибка повторяется, обновите настройки.").is_some());
    assert!(h.query_by_label("RAW TIMEOUT DIAGNOSTIC").is_none());
    h.get_by_label("Технические сведения").click();
    h.run_steps(3);
    assert!(h.query_by_label("RAW TIMEOUT DIAGNOSTIC").is_some());
    Ok(())
}

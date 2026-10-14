//! Explicit access to this open document. Authorization lives here, not in model instructions.

use egui::Ui;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use wordcraft_engine::{
    Selection,
    doc::{CharProps, Document},
};

use crate::{
    WordApp,
    control::{self, ControlRequest, Outcome},
};

/// An explicit list: adding an engine command never exposes it to a live agent automatically.
pub const COMMANDS: &[&str] = &[
    "document.inspect",
    "document.text",
    "document.paragraph",
    "review.wordCount",
    "review.changes",
    "review.comments",
    "format.state",
    "caret.left",
    "caret.right",
    "caret.up",
    "caret.down",
    "caret.wordLeft",
    "caret.wordRight",
    "caret.home",
    "caret.end",
    "caret.paraUp",
    "caret.paraDown",
    "caret.docStart",
    "caret.docEnd",
    "caret.pageUp",
    "caret.pageDown",
    "caret.set",
    "select.all",
    "select.range",
    "select.word",
    "select.sentence",
    "select.paragraph",
    "select.line",
    "select.text",
    "select.collapse",
    "select.owner",
    "text.insert",
    "text.newParagraph",
    "text.lineBreak",
    "text.backspace",
    "text.delete",
    "format.bold",
    "format.italic",
    "format.underline",
    "format.strikethrough",
    "format.font",
    "format.size",
    "format.color",
    "format.highlight",
    "format.subscript",
    "format.superscript",
    "para.style",
    "para.align",
    "para.alignLeft",
    "para.alignCenter",
    "para.alignRight",
    "para.justify",
    "para.spacing",
    "para.indents",
];

#[derive(Default)]
pub struct State {
    descriptor: Option<Value>,
    setup: Option<Value>,
    member: Option<Member>,
    pending: Option<Pending>,
    denied: std::collections::VecDeque<String>,
    next_author: u64,
    check: Option<Value>,
    start_failed: bool,
    error: Option<String>,
    requests_after_ms: f64,
}

struct Pending {
    name: String,
    client_id: String,
    connection_id: Option<u64>,
    closed: Option<Arc<AtomicBool>>,
    expires_at_ms: f64,
}

const APPROVAL_TIMEOUT_MS: f64 = 120_000.0;
const REQUEST_COOLDOWN_MS: f64 = 30_000.0;

fn deny(state: &mut State, client_id: String) {
    if state.denied.len() >= 16 {
        state.denied.pop_front();
    }
    state.denied.push_back(client_id);
    state.requests_after_ms = crate::now_ms() + REQUEST_COOLDOWN_MS;
}

struct Member {
    name: String,
    revision_author: String,
    client_id: String,
    connection_id: Option<u64>,
    closed: Option<Arc<AtomicBool>>,
    selection: Selection,
    pending: Option<CharProps>,
    document: Document,
    revision: u64,
    read_revision: Option<u64>,
    selection_invalidated: bool,
}

impl State {
    pub fn enabled(&self) -> bool {
        self.descriptor.is_some()
    }
    pub fn descriptor(&self) -> Option<&Value> {
        self.descriptor.as_ref()
    }
    pub fn connected_name(&self) -> Option<&str> {
        self.member.as_ref().map(|m| m.name.as_str())
    }
    pub fn waiting_for_approval(&self) -> bool {
        self.pending.is_some()
    }
}

/// Runtime grants are deliberately absent from UiState and ui.json.
pub fn command(app: &mut WordApp, params: &Value) -> Result<Value, String> {
    let stop_requests = params.get("stopRequests").and_then(Value::as_bool) == Some(true);
    if stop_requests {
        // Stopping requests remains available even if the displayed request expired or
        // closed between the owner's click and dispatch.
        disable(app);
    }
    if !stop_requests && let Some(approve) = params.get("approve").and_then(Value::as_bool) {
        let client = params.get("clientId").and_then(Value::as_str).ok_or("missing pending clientId")?;
        let pending = app.agents.pending.as_ref().ok_or("no agent is waiting for approval")?;
        if pending.client_id != client
            || pending.closed.as_ref().is_some_and(|c| c.load(Ordering::Acquire))
            || crate::now_ms() >= pending.expires_at_ms
        {
            return Err("the pending agent connection changed; review the current request".into());
        }
        let pending = app.agents.pending.take().ok_or("no pending agent")?;
        if approve {
            app.agents.next_author = app.agents.next_author.checked_add(1).ok_or("agent author identity exhausted")?;
            let session = app.agents.descriptor.as_ref().and_then(|d| d.get("sessionId")).and_then(Value::as_str).ok_or("session revoked")?;
            let author = app.session.agent_author(&pending.name, &format!("{session}-{}", app.agents.next_author))?;
            app.agents.member = Some(Member {
                name: pending.name,
                revision_author: author,
                client_id: pending.client_id,
                connection_id: pending.connection_id,
                closed: pending.closed,
                selection: Selection::caret(app.session.doc.start_of(wordcraft_engine::doc::StoryRef::Body)),
                pending: None,
                document: app.session.doc.clone(),
                revision: app.session.rev(),
                read_revision: None,
                selection_invalidated: true,
            });
            // A reviewed Allow is deliberate progress, not another unanswered prompt.
            // Busy gating now protects the approved slot; legitimate later turnover
            // can ask for fresh consent immediately.
            app.agents.requests_after_ms = 0.0;
        } else {
            deny(&mut app.agents, pending.client_id);
        }
    }
    if params.get("check").and_then(Value::as_bool) == Some(true) {
        let mut request = app.agents.descriptor.clone().ok_or("enable agent connection requests before checking the live target")?;
        request["check"] = json!(true);
        let hook = app.services.agent_control.as_ref().ok_or("desktop only")?;
        app.agents.check = Some(hook(request)?);
    }
    if let Some(enabled) = params.get("enabled").and_then(Value::as_bool) {
        app.ui.allow_agent_requests = enabled;
        app.agents.start_failed = false;
        if !enabled {
            disable(app);
        } else if !app.agents.enabled() {
            let hook = app.services.agent_control.as_ref().ok_or("live agents are available in the desktop app only")?;
            let request = json!({"enabled": true, "documentId": app.session.document_id(), "title": app.title_stem()});
            match hook(request) {
                Ok(descriptor) => {
                    if descriptor.get("documentId").and_then(Value::as_u64) != Some(app.session.document_id())
                        || descriptor.get("sessionId").and_then(Value::as_str).is_none_or(str::is_empty)
                    {
                        let _ = hook(json!({"enabled": false}));
                        app.agents.start_failed = true;
                        return Err("the desktop did not confirm this document's agent session".into());
                    }
                    app.agents.descriptor = Some(descriptor);
                    app.agents.error = None;
                }
                Err(error) => {
                    app.agents.start_failed = true;
                    app.agents.error = Some(error.clone());
                    return Err(error);
                }
            }
        }
    }
    Ok(json!({"enabled":app.agents.enabled(), "session":app.agents.descriptor, "agent":app.agents.member.as_ref().map(|m| &m.name),
        "pending":app.agents.pending.as_ref().map(|p| json!({"name":p.name,"clientId":p.client_id})),"check":app.agents.check}))
}

pub fn disable(app: &mut WordApp) {
    app.ui.allow_agent_requests = false;
    revoke_document(app);
}

pub fn revoke_document(app: &mut WordApp) {
    // Revoke in memory first; even a failed host cleanup cannot authorize an edit.
    let was_enabled = app.agents.descriptor.take().is_some();
    app.agents.member = None;
    app.agents.pending = None;
    app.agents.denied.clear();
    app.agents.check = None;
    app.agents.start_failed = false;
    app.agents.requests_after_ms = 0.0;
    if was_enabled
        && let Some(hook) = &app.services.agent_control
        && let Err(error) = hook(json!({"enabled":false}))
    {
        app.agents.error = Some(error);
    }
}

fn request_setup(app: &mut WordApp, refresh: bool) {
    if let Some(hook) = &app.services.agent_control {
        match hook(json!({"setup":true,"refresh":refresh})) {
            Ok(setup) => app.agents.setup = Some(setup),
            Err(error) => {
                app.agents.setup = Some(json!({"setupReady":false,"setupCode":"setup_failed","recoveryCode":"refresh_setup","detail":error}));
            }
        }
    }
}

pub fn sync(app: &mut WordApp) {
    if app.agents.setup.as_ref().is_some_and(|s| s["setupCode"] == "checking") {
        request_setup(app, false);
    }
    if app.agents.pending.as_ref().is_some_and(|p| crate::now_ms() >= p.expires_at_ms)
        && let Some(pending) = app.agents.pending.take()
    {
        deny(&mut app.agents, pending.client_id);
    }
    if app.agents.check.as_ref().is_some_and(|v| v.get("status").and_then(Value::as_str) == Some("checking"))
        && let Some(hook) = &app.services.agent_control
    {
        match hook(json!({"check":true,"poll":true})) {
            Ok(check) => app.agents.check = Some(check),
            Err(error) => {
                app.agents.check = Some(json!({"status":"error","code":"check_failed","recoveryCode":"refresh_setup","detail":error}));
            }
        }
    }
    // The transport marks this independently of its bounded request queue. A
    // dropped cleanup notification must never leave the document occupied.
    if app.agents.member.as_ref().is_some_and(|m| m.closed.as_ref().is_some_and(|closed| closed.load(Ordering::Acquire))) {
        app.agents.member = None;
    }
    if app.agents.pending.as_ref().is_some_and(|m| m.closed.as_ref().is_some_and(|closed| closed.load(Ordering::Acquire))) {
        app.agents.pending = None;
    }
    if app.agents.descriptor.as_ref().is_some_and(|d| d.get("documentId").and_then(Value::as_u64) != Some(app.session.document_id())) {
        revoke_document(app);
    }
    if app.ui.allow_agent_requests
        && !app.agents.enabled()
        && !app.agents.start_failed
        && app.services.agent_control.is_some()
        && let Err(error) = command(app, &json!({"enabled":true}))
    {
        app.agents.error = Some(error);
    }
    let title = app.title_stem();
    if app.agents.descriptor.as_ref().is_some_and(|d| d.get("title").and_then(Value::as_str) != Some(title.as_str()))
        && let Some(hook) = &app.services.agent_control
    {
        match hook(json!({"enabled":true,"refresh":true,"documentId":app.session.document_id(),"title":title})) {
            Ok(descriptor) if descriptor.get("sessionId") == app.agents.descriptor.as_ref().and_then(|d| d.get("sessionId")) => {
                app.agents.descriptor = Some(descriptor);
                app.agents.error = None;
            }
            Ok(_) => {
                disable(app);
                app.agents.error = Some("the document's agent session changed unexpectedly; enable access again".into());
            }
            Err(error) => app.agents.error = Some(error),
        }
    }
}

pub fn show(app: &mut WordApp, ui: &mut Ui) {
    if app.agents.setup.is_none() {
        request_setup(app, false);
    }
    ui.heading(tl!("Agents"));
    ui.label(tl!("Each agent needs your approval before reading or editing."));
    ui.label(tl!("Allowing requests makes this document's title visible to local clients."));
    let available = app.services.agent_control.is_some();
    let mut enabled = app.ui.allow_agent_requests;
    if ui.add_enabled(available, egui::Checkbox::new(&mut enabled, tl!("Allow agent connection requests"))).changed()
        && let Err(error) = app.run("ui.agents", json!({"enabled":enabled}))
    {
        app.agents.error = Some(error);
    }
    if !available {
        ui.label(tl!("Live agent connections are available in the desktop app."));
    }
    let setup = app.agents.setup.clone().unwrap_or(json!({}));
    if setup["setupCode"] == "checking" {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    }
    if ui.add_enabled(setup.get("setupReady").and_then(Value::as_bool) == Some(true), egui::Button::new(tl!("Copy MCP configuration"))).clicked()
        && let Some(config) = setup.get("mcpConfig").and_then(Value::as_str)
    {
        ui.ctx().copy_text(config.to_owned());
    }
    ui.label(tl!("Configure MCP once. Invitations are optional; choose a document and approve the agent in WordCraft."));
    ui.label(tl!("Some clients need to reload their tools or restart the agent session after the first setup. WordCraft stays open."));
    ui.label(crate::i18n::fmt(tl!("Document: {name}"), &[("name", &app.title_stem())]));
    if let Some(descriptor) = app.agents.descriptor.clone() {
        if ui.button(tl!("Copy invitation")).clicked()
            && let Some(text) = descriptor.get("invitation").and_then(Value::as_str)
        {
            ui.ctx().copy_text(text.to_owned());
        }
        if ui
            .add_enabled(!app.agents.check.as_ref().is_some_and(|v| v["status"] == "checking"), egui::Button::new(tl!("Check live connection")))
            .clicked()
            && let Err(error) = app.run("ui.agents", json!({"check":true}))
        {
            app.agents.error = Some(error);
        }
    }
    if let Some(member) = &app.agents.member {
        ui.label(crate::i18n::fmt(tl!("Client connected as {name}"), &[("name", &member.name)]));
        ui.label(tl!("Agent edits are tracked. You review changes and save the document."));
        if ui.button(tl!("Disconnect agent")).clicked() {
            // Revoke this document attachment, but keep reusable discovery available.
            revoke_document(app);
        }
    }
    if let Some(check) = &app.agents.check {
        match check.get("status").and_then(Value::as_str) {
            Some("checking") => {
                ui.label(tl!("Checking MCP and this document…"));
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
            }
            Some("ready") => {
                ui.label(tl!("MCP and this document are reachable. Client registration is separate."));
            }
            Some("error") => {
                ui.colored_label(ui.visuals().error_fg_color, status_text(check.get("code").and_then(Value::as_str).unwrap_or("check_failed")));
                ui.label(recovery_text(check.get("recoveryCode").and_then(Value::as_str).unwrap_or("refresh_setup")));
            }
            _ => {}
        }
    }
    ui.collapsing(tl!("Connection setup"), |ui| {
        if let Some(version) = setup.get("bridgeVersion").and_then(Value::as_str) { ui.label(format!("WordCraft {version}")); }
        ui.label(status_text(setup.get("setupCode").and_then(Value::as_str).unwrap_or("not_checked")));
        ui.label(recovery_text(setup.get("recoveryCode").and_then(Value::as_str).unwrap_or("check_setup")));
        if setup.get("setupReady").and_then(Value::as_bool) != Some(true)
            && setup["setupCode"] != "checking"
            && let Some(url) = setup.get("setupHelpUrl").and_then(Value::as_str)
        { ui.hyperlink_to(tl!("Install matching WordCraft release"), url); }
        if let Some(config) = setup.get("mcpConfig").and_then(Value::as_str) { ui.monospace(config); }
        if ui.button(tl!("Refresh")).clicked() {
            app.agents.start_failed = false;
            request_setup(app, true);
        }
        ui.label(tl!("This version supports one agent per document."));
        // Preserve the existing explanation of the independent headless/developer paths.
        ui.label(tl!("Every command is available to scripts and AI agents: run `wordcraft-cli mcp` for an MCP server, or start the app with `--control <port>` for the JSON control channel."));
    });
    if app.agents.error.is_some() {
        ui.colored_label(ui.visuals().error_fg_color, tl!("Agent connection could not be updated."));
        ui.label(tl!("Refresh connection setup and try again."));
    }
    ui.collapsing(tl!("Technical details"), |ui| {
        for detail in [
            setup.get("detail").and_then(Value::as_str),
            app.agents.check.as_ref().and_then(|v| v.get("detail")).and_then(Value::as_str),
            app.agents.error.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            ui.label(detail);
        }
    });
}

fn status_text(code: &str) -> &'static str {
    match code {
        "ready" => tl!("Agent connection setup is ready."),
        "checking" => tl!("Checking this installation…"),
        "not_checked" => tl!("Connection setup has not been checked."),
        "missing_bridge" => tl!("The agent connector is missing."),
        "incompatible_bridge" => tl!("The agent connector does not match this WordCraft version."),
        "invalid_path" => tl!("The installation path cannot be used for agent connections."),
        "check_timeout" => tl!("The connection check timed out."),
        "document_unreachable" => tl!("The connection check could not reach this document."),
        "check_cancelled" => tl!("The connection check was cancelled."),
        "check_failed" => tl!("The connection check failed."),
        _ => tl!("Agent connection setup is unavailable."),
    }
}

fn recovery_text(code: &str) -> &'static str {
    match code {
        "configure_client" => tl!("Copy the MCP configuration into your agent client."),
        "wait" => tl!("Please wait for the check to finish."),
        "check_setup" => tl!("Open connection setup to check this installation."),
        "install_matching" => tl!("Install the matching WordCraft release, then refresh setup."),
        "unicode_path" => tl!("Move WordCraft to a folder with a Unicode name, then refresh setup."),
        "retry_check" => tl!("Try the connection check again. If it keeps failing, refresh setup."),
        "enable_requests" => tl!("Allow requests for this document, then check the connection again."),
        _ => tl!("Refresh connection setup and try again."),
    }
}

/// Consume this modal's Escape before the background page or editor handles it.
pub fn handle_escape(app: &mut WordApp, ctx: &egui::Context) {
    if let Some(client) = app.agents.pending.as_ref().map(|p| p.client_id.clone())
        && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        && let Err(error) = app.run("ui.agents", json!({"approve":false,"clientId":client}))
    {
        app.agents.error = Some(error);
    }
}

pub fn prompt(app: &mut WordApp, ctx: &egui::Context) {
    let Some(pending) = &app.agents.pending else { return };
    let name = pending.name.clone();
    let client = pending.client_id.clone();
    let document = app.title_stem();
    ctx.request_repaint_after(std::time::Duration::from_secs(1));
    let response = egui::Modal::new(egui::Id::new("agent_attachment_approval")).show(ctx, |ui| {
        ui.heading(tl!("Agent connection request"));
        ui.label(crate::i18n::fmt(tl!("Agent {agent} wants to access {document}."), &[("agent", &name), ("document", &document)]));
        ui.label(tl!("The requesting client supplied this name. Its identity has not been verified."));
        ui.label(tl!("Agent edits are tracked. You review changes and save the document."));
        ui.horizontal(|ui| {
            for (label, action) in [(tl!("Allow"), Some(true)), (tl!("Deny"), Some(false)), (tl!("Deny and stop requests"), None)] {
                if ui.button(label).clicked() {
                    return Some(action);
                }
            }
            None
        })
        .inner
    });
    let decision = response.inner.or_else(|| response.should_close().then_some(Some(false)));
    if let Some(approve) = decision
        && let Err(error) = app.run("ui.agents", json!({"approve":approve.unwrap_or(false),"clientId":client,"stopRequests":approve.is_none()}))
    {
        app.agents.error = Some(error);
    }
}

pub fn handle(app: &mut WordApp, ctx: &egui::Context, method: &str, params: &Value) -> Result<Value, String> {
    handle_with_connection(app, ctx, method, params, None)
}

pub(crate) fn handle_with_connection(
    app: &mut WordApp,
    ctx: &egui::Context,
    method: &str,
    params: &Value,
    closed: Option<Arc<AtomicBool>>,
) -> Result<Value, String> {
    sync(app);
    let mut state = std::mem::take(&mut app.agents);
    let result = handle_inner(&mut state, app, ctx, method, params, closed);
    app.agents = state;
    result
}

fn handle_inner(
    state: &mut State,
    app: &mut WordApp,
    ctx: &egui::Context,
    method: &str,
    params: &Value,
    closed: Option<Arc<AtomicBool>>,
) -> Result<Value, String> {
    let descriptor = state.descriptor.as_ref().ok_or("session_revoked: live agent access is disabled")?;
    if params.get("sessionId") != descriptor.get("sessionId") || params.get("documentId").and_then(Value::as_u64) != Some(app.session.document_id()) {
        return Err("session_mismatch: this is not the invited document".into());
    }
    if method == "agent.check" {
        return Ok(json!({"sessionId":descriptor["sessionId"],"documentId":app.session.document_id(),"title":app.title_stem(),"enabled":true,
            "revision":app.session.rev(),"permissions":"request-approval","protocolVersion":1}));
    }
    let name = params
        .get("agentName")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= 128 && s.chars().all(|c| c.is_ascii_alphanumeric() || " _.-@".contains(c)))
        .ok_or("agentName must contain 1-128 ASCII letters, numbers, spaces, _, ., - or @")?;
    let client_id = params
        .get("clientId")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 128 && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'))
        .ok_or("missing or invalid MCP client identity")?;
    let connection_id = params.get("_connectionId").and_then(Value::as_u64);
    if method == "agent.transportClosed" {
        if state.member.as_ref().is_some_and(|m| m.name == name && m.client_id == client_id && m.connection_id == connection_id) {
            state.member = None;
        }
        if state.pending.as_ref().is_some_and(|m| m.name == name && m.client_id == client_id && m.connection_id == connection_id) {
            state.pending = None;
        }
        return Ok(json!({"connected":state.member.is_some()}));
    }
    if state.member.as_ref().is_some_and(|m| m.name != name || m.client_id != client_id) {
        return Err("agent_busy: disconnect the existing agent in WordCraft before inviting another".into());
    }
    if method == "agent.disconnect" {
        if state.member.as_ref().is_some_and(|member| member.connection_id != connection_id) {
            return Err("connection_changed: an old connection cannot disconnect the current agent".into());
        }
        state.member = None;
        if state.pending.as_ref().is_some_and(|p| p.client_id == client_id && p.connection_id == connection_id) {
            state.pending = None;
        }
        return Ok(json!({"connected":false}));
    }
    if state.member.is_none() {
        if method != "agent.status" {
            return Err("not_connected: verify the live session before calling document tools".into());
        }
        if state.denied.iter().any(|id| id == client_id) {
            return Err("approval_denied: the user declined this attachment".into());
        }
        if state.pending.as_ref().is_some_and(|p| p.client_id != client_id || p.name != name) {
            return Err("agent_busy: another agent is waiting for the user's decision".into());
        }
        if state.pending.is_none() && crate::now_ms() < state.requests_after_ms {
            return Err("approval_pending: connection requests are briefly paused after a recent request; wait before retrying".into());
        }
        if let Some(pending) = state.pending.as_mut() {
            pending.connection_id = connection_id;
            pending.closed = closed;
        } else {
            state.requests_after_ms = crate::now_ms() + REQUEST_COOLDOWN_MS;
            state.pending = Some(Pending {
                name: name.into(),
                client_id: client_id.into(),
                connection_id,
                closed,
                expires_at_ms: crate::now_ms() + APPROVAL_TIMEOUT_MS,
            });
        }
        ctx.request_repaint();
        return Err("approval_required: the user must allow this agent in WordCraft; wait for their decision and retry get_session_status".into());
    }
    if method == "agent.status" {
        if let Some(member) = state.member.as_mut() {
            if member.connection_id != connection_id {
                member.read_revision = None;
                member.selection_invalidated = true;
                member.pending = None;
            }
            member.connection_id = connection_id;
            member.closed = closed;
        }
        return Ok(json!({"sessionId":descriptor["sessionId"],"documentId":app.session.document_id(),"title":app.title_stem(),"enabled":true,
            "agentName":name,"clientId":client_id,"revision":app.session.rev(),"permissions":"tracked-editing","protocolVersion":1,"approved":true,
            "revisionAuthor":state.member.as_ref().map(|m| &m.revision_author)}));
    }
    let member = state.member.as_mut().ok_or("not_connected")?;
    if member.connection_id != connection_id {
        return Err("connection_changed: verify the agent session again before using its document tools".into());
    }
    let call = params.get("method").and_then(Value::as_str).ok_or("missing method")?;
    let arguments = params.get("params").cloned().unwrap_or(json!({}));
    if member.revision != app.session.rev() {
        // Mapping keeps read tools usable, but multiple owner edits can make
        // its inferred target ambiguous. Require an explicit new selection.
        member.selection_invalidated = true;
        match wordcraft_engine::shift_agent_selection(&member.document, &app.session.doc, &member.selection) {
            Ok(selection) => member.selection = selection,
            Err(_) => {
                // A removed story/cell must not make the read tools unusable. Recover a
                // valid position, but require the agent to choose a target before writing.
                member.selection = Selection::caret(app.session.doc.start_of(wordcraft_engine::doc::StoryRef::Body));
                member.selection_invalidated = true;
            }
        }
        member.pending = None;
        member.document = app.session.doc.clone();
        member.revision = app.session.rev();
        member.read_revision = None;
    }
    let value = match call {
        "engine.commands" => {
            let search = arguments.get("search").and_then(Value::as_str).unwrap_or("").to_lowercase();
            let mut commands: Vec<Value> = app
                .session
                .registry
                .all()
                .iter()
                .filter(|c| COMMANDS.contains(&c.id))
                .filter(|c| search.is_empty() || [c.id, c.label, c.location, c.params].iter().any(|s| s.to_lowercase().contains(&search)))
                .map(|c| json!({"id":c.id,"label":c.label,"location":c.location,"params":c.params,"enabled":true}))
                .collect();
            if search.is_empty() || "select.owner copy owner's selection".contains(&search) {
                commands.push(json!({"id":"select.owner","label":"Copy owner's selection","location":"Agents","params":"{}","enabled":app.session.column_segments().is_none()&&app.session.cell_range().is_none()}));
            }
            Value::Array(commands)
        }
        "document.inspect" => execute(member, app, "document.inspect", &arguments)?,
        "engine.execute" => {
            let command = arguments.get("command").and_then(Value::as_str).ok_or("missing command")?;
            let params = arguments.get("params").cloned().unwrap_or(json!({}));
            execute(member, app, command, &params)?
        }
        "ui.render" => {
            if arguments.get("path").is_some() {
                return Err("file access is not permitted for a live agent".into());
            }
            let (request, _) = ControlRequest::new("ui.render", arguments);
            match control::handle(app, ctx, &request) {
                Outcome::Done(reply) if reply.get("ok").and_then(Value::as_bool) == Some(true) => reply.get("result").cloned().unwrap_or(Value::Null),
                Outcome::Done(reply) => return Err(reply.get("error").and_then(Value::as_str).unwrap_or("page render failed").into()),
                Outcome::Screenshot { .. } => return Err("unexpected render response".into()),
            }
        }
        "ui.parity" => wordcraft_engine::catalog::parity(&app.session.registry),
        _ => return Err("this method is not permitted for a live agent".into()),
    };
    Ok(json!({"result":value,"revision":app.session.rev()}))
}

fn execute(member: &mut Member, app: &mut WordApp, id: &str, params: &Value) -> Result<Value, String> {
    if !COMMANDS.contains(&id) {
        return Err(format!("command not permitted for a live agent: {id}"));
    }
    if id == "select.owner" {
        if app.session.column_segments().is_some() || app.session.cell_range().is_some() {
            return Err("rectangular owner selections are not supported; select ordinary text first".into());
        }
        member.selection = app.session.sel.clone();
        member.pending = None;
        member.selection_invalidated = false;
        member.document = app.session.doc.clone();
        member.revision = app.session.rev();
        return Ok(json!({"selection":member.selection}));
    }
    let mutates = app.session.registry.get(id).is_some_and(|c| c.mutates);
    if mutates && member.read_revision != Some(app.session.rev()) {
        return Err("revision_conflict: read inspect_document again before editing and reassess the change".into());
    }
    if mutates && member.selection_invalidated {
        return Err("selection_invalidated: explicitly select the intended text or insertion point before editing".into());
    }
    let (mut value, selection, pending) =
        app.session.run_as_agent(&member.revision_author, member.selection.clone(), member.pending.clone(), id, params).map_err(|e| e.to_string())?;
    member.selection = selection;
    member.pending = pending;
    member.document = app.session.doc.clone();
    member.revision = app.session.rev();
    if mutates || matches!(id, "document.inspect" | "document.text") {
        member.read_revision = Some(app.session.rev());
    }
    if id == "document.inspect"
        && let Some(fields) = value.as_object_mut()
    {
        fields.insert("ownerSelection".into(), json!(app.session.sel));
    }
    if matches!(id, "select.all" | "select.range" | "select.text")
        || (matches!(id, "caret.set" | "caret.docStart" | "caret.docEnd") && params.get("extend").and_then(Value::as_bool) != Some(true))
    {
        member.selection_invalidated = false;
    }
    if member.selection_invalidated
        && matches!(id, "document.inspect" | "document.text")
        && let Some(fields) = value.as_object_mut()
    {
        fields.insert("agentSelectionInvalidated".into(), json!(true));
        fields.insert(
            "agentInstruction".into(),
            json!("Explicitly select the intended text or insertion point before editing; select.owner copies the user's ordinary text selection."),
        );
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;
    fn app() -> WordApp {
        let mut app = WordApp::new(
            wordcraft_engine::Session::new(Document::from_text("Original café")),
            Services {
                agent_control: Some(Box::new(|params| {
                    Ok(if params["enabled"] == true {
                        json!({"sessionId":"test","documentId":params["documentId"],"title":params["title"]})
                    } else {
                        json!({"enabled":false})
                    })
                })),
                ..Default::default()
            },
        );
        command(&mut app, &json!({"enabled":true})).unwrap();
        app
    }
    fn request(app: &mut WordApp, method: &str, params: Value) -> Result<Value, String> {
        let envelope = json!({"sessionId":"test","documentId":app.session.document_id(),"agentName":"Test agent","clientId":"test-client","method":method,"params":params});
        handle(app, &egui::Context::default(), "agent.call", &envelope)
    }
    fn connect(app: &mut WordApp) {
        let envelope = json!({"sessionId":"test","documentId":app.session.document_id(),"agentName":"Test agent","clientId":"test-client"});
        assert!(handle(app, &egui::Context::default(), "agent.status", &envelope).unwrap_err().contains("approval_required"));
        command(app, &json!({"approve":true,"clientId":"test-client"})).unwrap();
        assert_eq!(handle(app, &egui::Context::default(), "agent.status", &envelope).unwrap()["approved"], true);
    }
    #[test]
    fn enabled_is_not_connected_and_grants_are_not_saved() {
        let mut app = app();
        assert!(app.agents.member.is_none());
        assert!(!serde_json::to_string(&app.prefs()).unwrap().contains("sessionId"));
        assert!(request(&mut app, "document.inspect", json!({})).unwrap_err().contains("not_connected"));
        connect(&mut app);
        assert!(app.agents.member.is_some());
    }
    #[test]
    fn edits_require_reading_and_conflicts_require_rereading() {
        let mut app = app();
        connect(&mut app);
        let insert = json!({"command":"text.insert","params":{"text":"Agent "}});
        assert!(request(&mut app, "engine.execute", insert.clone()).unwrap_err().contains("revision_conflict"));
        request(&mut app, "document.inspect", json!({})).unwrap();
        app.session.run("text.insert", &json!({"text":"Owner "})).unwrap();
        assert!(request(&mut app, "engine.execute", insert.clone()).unwrap_err().contains("revision_conflict"));
        request(&mut app, "document.inspect", json!({})).unwrap();
        request(&mut app, "engine.execute", json!({"command":"caret.docStart"})).unwrap();
        request(&mut app, "engine.execute", insert).unwrap();
        assert_eq!(app.session.author, "WordCraft User");
        assert!(!app.session.doc.settings.track_changes);
    }
    #[test]
    fn separated_owner_edits_require_a_new_target_even_after_rereading() {
        let mut app = app();
        connect(&mut app);
        request(&mut app, "document.inspect", json!({})).unwrap();
        request(&mut app, "engine.execute", json!({"command":"caret.docEnd"})).unwrap();
        app.session.run("text.insert", &json!({"text":"Start "})).unwrap();
        app.session.run("caret.docEnd", &json!({})).unwrap();
        app.session.run("text.insert", &json!({"text":" end"})).unwrap();
        let read = request(&mut app, "document.inspect", json!({})).unwrap();
        assert_eq!(read["result"]["agentSelectionInvalidated"], true);
        let insert = json!({"command":"text.insert","params":{"text":"Agent"}});
        assert!(request(&mut app, "engine.execute", insert.clone()).unwrap_err().contains("selection_invalidated"));
        request(&mut app, "engine.execute", json!({"command":"caret.docEnd"})).unwrap();
        request(&mut app, "engine.execute", insert).unwrap();
    }
    #[test]
    fn closed_connection_is_released_without_a_cleanup_request() {
        let mut app = app();
        let ctx = egui::Context::default();
        let identity =
            json!({"sessionId":"test","documentId":app.session.document_id(),"agentName":"Test agent","clientId":"test-client","_connectionId":1});
        let closed = Arc::new(AtomicBool::new(false));
        assert!(handle_with_connection(&mut app, &ctx, "agent.status", &identity, Some(closed.clone())).unwrap_err().contains("approval_required"));
        command(&mut app, &json!({"approve":true,"clientId":"test-client"})).unwrap();
        assert_eq!(app.agents.requests_after_ms, 0.0, "only an actual owner approval clears the unanswered-request throttle");
        closed.store(true, Ordering::Release);
        // No agent.transportClosed is delivered: simulate a full host queue.
        sync(&mut app);
        assert!(app.agents.member.is_none());
        assert!(app.agents.enabled());
        let new_client =
            json!({"sessionId":"test","documentId":app.session.document_id(),"agentName":"Other agent","clientId":"new-client","_connectionId":2});
        assert!(handle(&mut app, &ctx, "agent.status", &new_client).unwrap_err().contains("approval_required"));
        command(&mut app, &json!({"approve":true,"clientId":"new-client"})).unwrap();
        assert_eq!(app.agents.member.as_ref().unwrap().name, "Other agent");
    }
    #[test]
    fn generic_execute_cannot_bypass_policy_and_replacement_revokes() {
        let mut app = app();
        connect(&mut app);
        request(&mut app, "document.inspect", json!({})).unwrap();
        for id in ["file.save", "file.new", "edit.undo", "review.trackChanges", "tools.runMacro", "ui.agents"] {
            assert!(request(&mut app, "engine.execute", json!({"command":id})).unwrap_err().contains("not permitted"));
        }
        assert!(request(&mut app, "ui.render", json!({"path":"/tmp/forbidden.png"})).is_err());
        app.ui.allow_agent_requests = false;
        app.session.set_document(Document::from_text("Replacement"));
        sync(&mut app);
        assert!(!app.agents.enabled());
        assert!(request(&mut app, "document.inspect", json!({})).unwrap_err().contains("session_revoked"));
    }
    #[test]
    fn expired_control_request_never_edits() {
        let mut app = app();
        let (request, _) = ControlRequest::new("engine.execute", json!({"command":"text.insert","params":{"text":"late"}}));
        request.cancelled.store(true, std::sync::atomic::Ordering::Release);
        let before = app.session.doc.plain_text(wordcraft_engine::doc::StoryRef::Body);
        match control::handle(&mut app, &egui::Context::default(), &request) {
            Outcome::Done(v) => assert_eq!(v["ok"], false),
            _ => panic!("unexpected"),
        }
        assert_eq!(before, app.session.doc.plain_text(wordcraft_engine::doc::StoryRef::Body));
    }
    #[test]
    fn removed_agent_story_can_be_read_but_requires_a_new_selection() {
        use wordcraft_engine::doc::{PartKind, Path, Pos, StoryRef};
        let mut app = app();
        let part = app.session.doc.add_part(PartKind::Header, Document::from_text("Removed story").body);
        app.session.touch();
        connect(&mut app);
        request(&mut app, "document.inspect", json!({})).unwrap();
        let pos = Pos { story: StoryRef::Part(part), path: Path::top(0), off: 0 };
        request(&mut app, "engine.execute", json!({"command":"caret.set","params":{"pos":pos}})).unwrap();
        app.session.doc.parts.remove(&part);
        app.session.touch();
        let read = request(&mut app, "document.inspect", json!({})).unwrap();
        assert_eq!(read["result"]["agentSelectionInvalidated"], true);
        let insert = json!({"command":"text.insert","params":{"text":"New "}});
        assert!(request(&mut app, "engine.execute", insert.clone()).unwrap_err().contains("selection_invalidated"));
        request(&mut app, "engine.execute", json!({"command":"caret.docStart"})).unwrap();
        request(&mut app, "engine.execute", insert).unwrap();
    }
    #[test]
    fn wrong_invitation_and_second_agent_cannot_claim_the_document() {
        let mut app = app();
        let ctx = egui::Context::default();
        let wrong = json!({"sessionId":"other","documentId":app.session.document_id(),"agentName":"Other","clientId":"other-client"});
        assert!(handle(&mut app, &ctx, "agent.status", &wrong).unwrap_err().contains("session_mismatch"));
        assert!(app.agents.member.is_none());
        connect(&mut app);
        let second = json!({"sessionId":"test","documentId":app.session.document_id(),"agentName":"Other","clientId":"other-client"});
        assert!(handle(&mut app, &ctx, "agent.status", &second).unwrap_err().contains("agent_busy"));
        let same_name = json!({"sessionId":"test","documentId":app.session.document_id(),"agentName":"Test agent","clientId":"another-client"});
        assert!(handle(&mut app, &ctx, "agent.status", &same_name).unwrap_err().contains("agent_busy"));
        assert!(handle(&mut app, &ctx, "agent.disconnect", &same_name).unwrap_err().contains("agent_busy"));
        assert!(app.agents.member.is_some());
        disable(&mut app);
        assert!(app.agents.member.is_none());
    }
    #[test]
    fn saving_under_a_new_name_refreshes_the_invitation_without_replacing_the_session() {
        let mut app = app();
        connect(&mut app);
        app.session.path = Some(std::path::PathBuf::from("Renamed document.docx"));
        sync(&mut app);
        assert_eq!(app.agents.descriptor.as_ref().unwrap()["title"], "Renamed document");
        assert_eq!(app.agents.descriptor.as_ref().unwrap()["sessionId"], "test");
        assert!(app.agents.member.is_some());
    }
    #[test]
    fn old_socket_cleanup_does_not_disconnect_a_reconnected_client() {
        let mut app = app();
        let ctx = egui::Context::default();
        let mut identity =
            json!({"sessionId":"test","documentId":app.session.document_id(),"agentName":"Test agent","clientId":"test-client","_connectionId":1});
        assert!(handle(&mut app, &ctx, "agent.status", &identity).unwrap_err().contains("approval_required"));
        command(&mut app, &json!({"approve":true,"clientId":"test-client"})).unwrap();
        identity["_connectionId"] = json!(2);
        handle(&mut app, &ctx, "agent.status", &identity).unwrap();
        identity["_connectionId"] = json!(1);
        assert!(handle(&mut app, &ctx, "agent.disconnect", &identity).unwrap_err().contains("connection_changed"));
        handle(&mut app, &ctx, "agent.transportClosed", &identity).unwrap();
        assert!(app.agents.member.is_some());
        identity["_connectionId"] = json!(2);
        handle(&mut app, &ctx, "agent.transportClosed", &identity).unwrap();
        assert!(app.agents.member.is_none());
        assert!(app.agents.enabled());
    }
    #[test]
    fn agent_can_copy_the_owners_selection_without_moving_the_owner() {
        use wordcraft_engine::doc::Pos;
        let mut app = app();
        app.session.sel = Selection { anchor: Pos::body(0, 0), focus: Pos::body(0, 8) };
        let owner = app.session.sel.clone();
        connect(&mut app);
        let inspect = request(&mut app, "document.inspect", json!({})).unwrap();
        assert_eq!(inspect["result"]["ownerSelection"], json!(owner));
        request(&mut app, "engine.execute", json!({"command":"select.owner"})).unwrap();
        request(&mut app, "engine.execute", json!({"command":"format.bold","params":{"value":true}})).unwrap();
        assert_eq!(app.session.sel, owner);
        assert!(app.session.run("review.changes", &json!({})).unwrap().to_string().contains("Test agent"));
    }
    #[test]
    fn consent_is_required_even_when_discovery_is_enabled_and_deny_is_effective() {
        let mut app = app();
        let ctx = egui::Context::default();
        let envelope = json!({"sessionId":"test","documentId":app.session.document_id(),"agentName":"Test agent","clientId":"test-client"});
        let before = serde_json::to_value(&app.session.doc).unwrap();
        assert!(handle(&mut app, &ctx, "agent.status", &envelope).unwrap_err().contains("approval_required"));
        assert!(app.agents.member.is_none());
        assert!(request(&mut app, "document.inspect", json!({})).unwrap_err().contains("not_connected"));
        command(&mut app, &json!({"approve":false,"clientId":"test-client"})).unwrap();
        assert!(handle(&mut app, &ctx, "agent.status", &envelope).unwrap_err().contains("approval_denied"));
        assert_eq!(serde_json::to_value(&app.session.doc).unwrap(), before);
        assert!(app.agents.member.is_none());
    }
    #[test]
    fn agent_display_name_cannot_erase_the_owners_tracked_insertion() {
        let mut app = app();
        app.session.author = "Test agent".into();
        app.session.doc.settings.track_changes = true;
        app.session.run("text.insert", &json!({"text":"Owner insertion "})).unwrap();
        connect(&mut app);
        assert_ne!(app.agents.member.as_ref().unwrap().revision_author, app.session.author);
        request(&mut app, "document.inspect", json!({})).unwrap();
        request(&mut app, "engine.execute", json!({"command":"select.text","params":{"text":"Owner insertion "}})).unwrap();
        request(&mut app, "engine.execute", json!({"command":"text.delete"})).unwrap();
        // The owner's insertion must retain its provenance and receive a tracked deletion.
        let revision_authors: Vec<_> = app.session.doc.revisions.iter().map(|r| r.author.clone()).collect();
        assert!(revision_authors.iter().any(|a| a == "Test agent"));
        assert!(revision_authors.iter().any(|a| a.contains("WordCraft agent")));
        let changes = app.session.run("review.changes", &json!({})).unwrap();
        assert!(!changes.as_array().unwrap().is_empty());
        assert!(app.session.doc.plain_text(wordcraft_engine::doc::StoryRef::Body).contains("Owner insertion"));
    }
    #[test]
    fn relative_movements_do_not_unlock_an_invalidated_target() {
        let mut app = app();
        connect(&mut app);
        request(&mut app, "document.inspect", json!({})).unwrap();
        request(&mut app, "engine.execute", json!({"command":"caret.right"})).unwrap();
        assert!(
            request(&mut app, "engine.execute", json!({"command":"text.insert","params":{"text":"unsafe"}}))
                .unwrap_err()
                .contains("selection_invalidated")
        );
        request(&mut app, "engine.execute", json!({"command":"select.word"})).unwrap();
        assert!(
            request(&mut app, "engine.execute", json!({"command":"text.insert","params":{"text":"unsafe"}}))
                .unwrap_err()
                .contains("selection_invalidated")
        );
        request(&mut app, "engine.execute", json!({"command":"caret.docEnd","params":{"extend":true}})).unwrap();
        assert!(request(&mut app, "engine.execute", json!({"command":"text.delete"})).unwrap_err().contains("selection_invalidated"));
        request(&mut app, "engine.execute", json!({"command":"caret.docStart"})).unwrap();
        request(&mut app, "engine.execute", json!({"command":"text.insert","params":{"text":"safe"}})).unwrap();
    }
    #[test]
    fn selfcheck_never_registers_or_reads_the_document() {
        let mut app = app();
        let params = json!({"sessionId":"test","documentId":app.session.document_id()});
        let check = handle(&mut app, &egui::Context::default(), "agent.check", &params).unwrap();
        assert_eq!(check["permissions"], "request-approval");
        assert!(check.get("blocks").is_none());
        assert!(app.agents.member.is_none() && app.agents.pending.is_none());
    }
    #[test]
    fn a_failed_discovery_start_waits_for_explicit_retry() {
        use std::{cell::Cell, rc::Rc};
        let attempts = Rc::new(Cell::new(0));
        let counted = attempts.clone();
        let mut app = WordApp::new(
            wordcraft_engine::Session::new(Document::new()),
            crate::Services {
                agent_control: Some(Box::new(move |request| {
                    if request["setup"] == true {
                        return Ok(json!({"setupReady":true}));
                    }
                    counted.set(counted.get() + 1);
                    Err("runtime directory unavailable".into())
                })),
                ..Default::default()
            },
        );
        app.ui.allow_agent_requests = true;
        for _ in 0..5 {
            sync(&mut app);
        }
        assert_eq!(attempts.get(), 1, "a failed startup must not repeat at frame rate");
        assert!(command(&mut app, &json!({"enabled":true})).is_err());
        assert_eq!(attempts.get(), 2, "the user can retry explicitly");
        sync(&mut app);
        assert_eq!(attempts.get(), 2);
    }

    #[test]
    fn fresh_preferences_do_not_discover_documents_or_probe_setup() {
        use std::{cell::Cell, rc::Rc};
        let calls = Rc::new(Cell::new(0));
        let observed = calls.clone();
        let mut app = WordApp::new(
            wordcraft_engine::Session::new(Document::new()),
            Services {
                agent_control: Some(Box::new(move |_| {
                    observed.set(observed.get() + 1);
                    Ok(json!({}))
                })),
                ..Default::default()
            },
        );
        for preferences in [
            crate::UiState::default(),
            crate::UiState::from_json(b"{}").unwrap(),
            crate::UiState::from_json(br#"{"allowAgentRequests":"yes"}"#).unwrap(),
        ] {
            app.ui = preferences;
            for _ in 0..5 {
                sync(&mut app);
            }
            assert!(!app.ui.allow_agent_requests && !app.agents.enabled());
        }
        assert_eq!(calls.get(), 0);
        let saved = crate::UiState::from_json(br#"{"allowAgentRequests":true}"#).unwrap();
        assert!(saved.allow_agent_requests);
        assert!(serde_json::to_value(saved).unwrap()["allowAgentRequests"].as_bool().unwrap());
    }

    #[test]
    fn pending_expiry_survives_polling_and_denial_pauses_other_client_ids() {
        let mut app = app();
        let ctx = egui::Context::default();
        let mut params = json!({"sessionId":"test","documentId":app.session.document_id(),"agentName":"Your editor","clientId":"first"});
        assert!(handle(&mut app, &ctx, "agent.status", &params).unwrap_err().contains("approval_required"));
        let expiry = app.agents.pending.as_ref().unwrap().expires_at_ms;
        for _ in 0..4 {
            assert!(handle(&mut app, &ctx, "agent.status", &params).unwrap_err().contains("approval_required"));
            assert_eq!(app.agents.pending.as_ref().unwrap().expires_at_ms, expiry);
        }
        app.agents.pending.as_mut().unwrap().expires_at_ms = 0.0;
        assert!(handle(&mut app, &ctx, "agent.status", &params).unwrap_err().contains("approval_denied"));
        params["clientId"] = json!("second");
        assert!(handle(&mut app, &ctx, "agent.status", &params).unwrap_err().contains("approval_pending"));
        assert!(app.agents.pending.is_none() && app.agents.member.is_none());
        app.agents.requests_after_ms = 0.0;
        assert!(handle(&mut app, &ctx, "agent.status", &params).unwrap_err().contains("approval_required"));
        command(&mut app, &json!({"approve":false,"clientId":"second","stopRequests":true})).unwrap();
        for _ in 0..5 {
            sync(&mut app);
        }
        assert!(!app.agents.enabled() && !app.ui.allow_agent_requests);
        params["clientId"] = json!("third");
        assert!(handle(&mut app, &ctx, "agent.status", &params).unwrap_err().contains("session_revoked"));
    }

    #[test]
    fn german_agent_messages_use_the_catalogs_formal_address() {
        let language = crate::i18n::Lang::from_code("de").unwrap();
        for key in [
            "Each agent needs your approval before reading or editing.",
            "Configure MCP once. Invitations are optional; choose a document and approve the agent in WordCraft.",
            "Agent edits are tracked. You review changes and save the document.",
        ] {
            let translation = crate::i18n::tr(language, key);
            assert_ne!(translation, key);
            assert!(translation.contains("Sie") || translation.contains("Ihre"), "{translation}");
            assert!(!translation.split_whitespace().any(|word| ["du", "Du", "deine", "Richte", "wähle"].contains(&word)));
        }
        for language in crate::i18n::Lang::all().filter(|l| *l != crate::i18n::Lang::EN) {
            for key in ["Agent approval requested", "Listening for agent requests"] {
                assert_ne!(crate::i18n::tr(language, key), key, "{}: {key}", language.code());
            }
        }
    }

    #[test]
    fn dropped_requests_cannot_flash_new_prompts_under_another_client_id() {
        let mut app = app();
        let ctx = egui::Context::default();
        let mut params =
            json!({"sessionId":"test","documentId":app.session.document_id(),"agentName":"Your editor","clientId":"first","_connectionId":1});
        let closed = Arc::new(AtomicBool::new(false));
        assert!(handle_with_connection(&mut app, &ctx, "agent.status", &params, Some(closed.clone())).unwrap_err().contains("approval_required"));
        let expiry = app.agents.pending.as_ref().unwrap().expires_at_ms;
        assert!(handle_with_connection(&mut app, &ctx, "agent.status", &params, Some(closed.clone())).unwrap_err().contains("approval_required"));
        assert_eq!(app.agents.pending.as_ref().unwrap().expires_at_ms, expiry);
        closed.store(true, Ordering::Release);
        sync(&mut app);
        assert!(app.agents.pending.is_none());
        params["clientId"] = json!("second");
        params["_connectionId"] = json!(2);
        assert!(handle(&mut app, &ctx, "agent.status", &params).unwrap_err().contains("approval_pending"));
        assert!(app.agents.pending.is_none() && app.agents.member.is_none());
        app.agents.requests_after_ms = 0.0;
        assert!(handle(&mut app, &ctx, "agent.status", &params).unwrap_err().contains("approval_required"));
        let expiry = app.agents.pending.as_ref().unwrap().expires_at_ms;
        params["_connectionId"] = json!(3);
        assert!(handle(&mut app, &ctx, "agent.status", &params).unwrap_err().contains("approval_required"));
        assert_eq!(app.agents.pending.as_ref().unwrap().expires_at_ms, expiry, "same pending client reconnects without a new prompt or deadline");
    }
}

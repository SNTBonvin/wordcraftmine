//! In-app Ollama task pane. Workers never own a Session: every document tool returns to the
//! UI thread, which checks the live editing permission, cancellation and document identity.
use crate::{WordApp, theme::Tokens};
use egui::{RichText, Stroke, Ui};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{Receiver, Sender},
};

#[cfg(not(target_arch = "wasm32"))]
mod codex;
#[cfg(not(target_arch = "wasm32"))]
mod transport;

const MAX_INPUT: usize = 16_000;
const MAX_MESSAGES: usize = 96;

#[derive(Clone, Copy, Debug, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    #[default]
    Local,
    Cloud,
    LlamaCpp,
    Codex,
}

/// Persist only connection choices. Credentials, conversation and editing permission stay ephemeral.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Preferences {
    pub source: Source,
    pub endpoint: String,
    pub llama_endpoint: String,
    pub model: String,
    pub internet: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            source: Source::Local,
            endpoint: "http://127.0.0.1:11434".into(),
            llama_endpoint: "http://127.0.0.1:8085".into(),
            model: String::new(),
            internet: true,
        }
    }
}

pub struct State {
    pub open: bool,
    pub allow_edits: bool,
    pub internet: bool,
    pub source: Source,
    pub endpoint: String,
    pub llama_endpoint: String,
    pub structured: bool,
    pub model: String,
    pub api_key: String,
    pub models: Vec<String>,
    pub input: String,
    pub busy: bool,
    pub status: String,
    pub sources: Vec<(String, String)>,
    pub account: String,
    pub signed_in: bool,
    pub usage: String,
    entries: Vec<(String, String)>,
    history: Vec<Value>,
    context: Value,
    events: Option<Receiver<Event>>,
    cancel: Arc<AtomicBool>,
    document: Option<u64>,
    undo: Vec<UndoMark>,
    turn_revision: Option<u64>,
    live_response: String,
}

impl Default for State {
    fn default() -> Self {
        Self {
            open: false,
            allow_edits: false,
            internet: true,
            source: Source::Local,
            endpoint: "http://127.0.0.1:11434".into(),
            llama_endpoint: "http://127.0.0.1:8085".into(),
            structured: true,
            model: String::new(),
            api_key: std::env::var("OLLAMA_API_KEY").unwrap_or_default(),
            models: Vec::new(),
            input: String::new(),
            busy: false,
            status: "Choose a model to start.".into(),
            sources: Vec::new(),
            account: String::new(),
            signed_in: false,
            usage: String::new(),
            entries: Vec::new(),
            history: Vec::new(),
            context: Value::Null,
            events: None,
            cancel: Arc::new(AtomicBool::new(false)),
            document: None,
            undo: Vec::new(),
            turn_revision: None,
            live_response: String::new(),
        }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

struct UndoMark {
    document: u64,
    revision: u64,
    depth: usize,
}

pub(super) enum Event {
    Models(Result<Vec<String>, String>),
    Account { label: String, signed_in: bool },
    Login(String),
    Usage(String),
    Message(String),
    Partial(String),
    Sources(Vec<(String, String)>),
    Tool { name: String, args: Value, reply: Sender<Value> },
    Finished(Result<Vec<Value>, String>),
}

impl State {
    pub fn preferences(&self) -> Preferences {
        Preferences {
            source: self.source,
            endpoint: self.endpoint.clone(),
            llama_endpoint: self.llama_endpoint.clone(),
            model: self.model.clone(),
            internet: self.internet,
        }
    }
    pub fn apply_preferences(&mut self, p: &Preferences) {
        self.stop();
        self.source = p.source;
        self.endpoint = clip(&p.endpoint, 2000);
        self.llama_endpoint = clip(&p.llama_endpoint, 2000);
        self.model = clip(&p.model, 256);
        self.internet = p.internet;
        self.allow_edits = false;
        self.models.clear();
        self.status = "Choose a model to start.".into();
    }
    pub(crate) fn stop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
        self.events = None;
        self.busy = false;
        // Keep completed exchanges only. Never replay abandoned tool calls.
        self.history.clear();
        self.live_response.clear();
        self.status = "Stopped. Completed changes remain available in Undo.".into();
    }
    fn entry(&mut self, role: &str, text: String) {
        if self.entries.len() >= MAX_MESSAGES {
            self.entries.remove(0);
        }
        self.entries.push((role.into(), clip(&text, 24_000)));
    }
}

pub(super) fn clip(s: &str, limit: usize) -> String {
    s.chars().take(limit).collect()
}

fn allowed(id: &str, args: &Value) -> bool {
    // This is a document assistant. File I/O, macros, system settings, dictionary writes,
    // password protection and undo/redo are deliberately not model-dispatchable.
    let prefix = id.split('.').next().unwrap_or("");
    let safe_prefix = matches!(
        prefix,
        "arrange"
            | "canvas"
            | "caret"
            | "chart"
            | "design"
            | "document"
            | "draw"
            | "equation"
            | "fields"
            | "format"
            | "hf"
            | "insert"
            | "layout"
            | "list"
            | "object"
            | "outline"
            | "para"
            | "picture"
            | "references"
            | "review"
            | "select"
            | "shape"
            | "smartArt"
            | "styles"
            | "table"
            | "text"
            | "wordArt"
    );
    let safe_edit = matches!(
        id,
        "edit.find" | "edit.replace" | "edit.cut" | "edit.copy" | "edit.paste" | "edit.pasteText" | "edit.pasteMerge" | "edit.selectAll"
    );
    (safe_prefix || safe_edit || id == "file.properties")
        && !matches!(id, "insert.textFromFile" | "review.restrict" | "review.readAloud")
        && !external_params(args, 0)
}

fn external_params(v: &Value, depth: usize) -> bool {
    if depth > 16 {
        return true;
    }
    match v {
        Value::Object(o) => o.iter().any(|(k, v)| {
            matches!(k.as_str(), "file" | "filename" | "password") || (k == "path" && v.is_string()) || external_params(v, depth.saturating_add(1))
        }),
        Value::Array(a) => a.iter().any(|v| external_params(v, depth.saturating_add(1))),
        _ => false,
    }
}

pub fn can_undo(app: &WordApp) -> bool {
    app.ollama.undo.last().is_some_and(|m| {
        m.document == app.session.document_id()
            && m.revision == app.session.rev()
            && m.depth == app.session.undo_depth()
            && app.session.undo_label().is_some_and(|s| s.starts_with("Ollama: ") || s.starts_with("Codex: "))
    })
}

pub fn undo(app: &mut WordApp) -> Result<(), String> {
    if !can_undo(app) {
        return Err("Use the main Undo history to undo edits made after the AI change first.".into());
    }
    app.ollama.stop();
    app.session.run("edit.undo", &json!({})).map_err(|e| e.to_string())?;
    app.ollama.undo.pop();
    if let Some(mark) = app.ollama.undo.last_mut() {
        // Only consecutive AI steps remain eligible; manual steps between them keep their own history.
        if mark.depth == app.session.undo_depth() {
            mark.revision = app.session.rev();
        }
    }
    app.ollama.entry("Activity", "Undid the last AI change.".into());
    Ok(())
}

fn document_title(app: &WordApp) -> String {
    if app.session.doc.core.title.trim().is_empty() { app.title_stem() } else { app.session.doc.core.title.clone() }
}

fn document_tool(app: &mut WordApp, name: &str, args: &Value) -> Result<Value, String> {
    if app.ollama.cancel.load(Ordering::SeqCst) {
        return Err("Request cancelled.".into());
    }
    if app.ollama.document != Some(app.session.document_id()) {
        return Err("The open document changed. Send a new message.".into());
    }
    match name {
        "write_text" => verified_write(app, args),
        "read_document" => {
            app.ollama.turn_revision = Some(app.session.rev());
            let text = app.session.doc.plain_text(wordcraft_doc::StoryRef::Body);
            let offset = args.get("offset").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or(0).min(text.chars().count());
            let limit = args.get("limit").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or(12_000).clamp(1, 24_000);
            let chunk: String = text.chars().skip(offset).take(limit).collect();
            Ok(
                json!({"text": chunk, "offset": offset, "totalCharacters": text.chars().count(), "title":document_title(app), "selection": app.session.sel,
                "allowEdits": app.ollama.allow_edits, "revision": app.session.rev()}),
            )
        }
        "find_commands" => {
            let query = args.get("query").and_then(Value::as_str).unwrap_or("").to_lowercase();
            let prose = query.contains("append") || query == "add text" || query == "write";
            let mut specs: Vec<_> = app
                .session
                .registry
                .all()
                .iter()
                .filter(|s| allowed(s.id, &json!({})))
                .filter(|s| {
                    (prose && matches!(s.id, "text.insert" | "caret.docEnd"))
                        || query.split_whitespace().all(|q| format!("{} {} {} {}", s.id, s.label, s.location, s.params).to_lowercase().contains(q))
                })
                .collect();
            specs.sort_by_key(|s| {
                if (prose && matches!(s.id, "text.insert" | "caret.docEnd")) || s.id.eq_ignore_ascii_case(&query) {
                    0
                } else if s.id.starts_with("text.") || s.id.starts_with("document.") || s.id.starts_with("caret.") {
                    1
                } else if s.id.to_lowercase().contains(&query) {
                    2
                } else {
                    3
                }
            });
            let found: Vec<Value> = specs.into_iter().take(30).map(|s| json!({"id":s.id,"label":s.label,"params":s.params,"mutates":s.mutates,
                "purpose":match s.id {"text.insert"=>"Type prose at caret; replaces selected text", "caret.docEnd"=>"Move insertion point to end of document",
                "references.addText"=>"Set table-of-contents outline level; does not insert prose", "document.setText"=>"Replace the entire document body",
                _=>s.location}})).collect();
            Ok(json!({"commands": found}))
        }
        "execute_command" => {
            let id = args.get("command").and_then(Value::as_str).ok_or("command is required")?;
            let params = args.get("params").cloned().unwrap_or(json!({}));
            if params.to_string().len() > 64_000 {
                return Err("Command parameters are too large.".into());
            }
            if !allowed(id, &params) {
                return Err("This command is outside the document assistant's scope.".into());
            }
            let spec = app.session.registry.get(id).ok_or("Unknown command; use find_commands first.")?;
            // Fail closed: only these read tools can execute with editing disabled. A misclassified
            // pure command cannot change selections, formatting, document contents or disk files.
            if !app.ollama.allow_edits
                && !matches!(id, "document.inspect" | "document.text" | "document.paragraph" | "document.selection" | "document.layout")
            {
                return Err("AI editing is disabled. The user must enable Allow AI editing.".into());
            }
            let mutates = spec.mutates;
            if app.ollama.allow_edits && app.ollama.turn_revision != Some(app.session.rev()) {
                return Err("The document changed since the last read. Read the document again before editing.".into());
            }
            let provider = if app.ollama.source == Source::Codex { "Codex" } else { "Ollama" };
            let label = format!("{provider}: {}", spec.label);
            let result = app.session.run_labeled(id, &params, &label).map_err(|e| e.to_string())?;
            app.ollama.turn_revision = Some(app.session.rev());
            if mutates {
                if app.ollama.undo.len() >= 100 {
                    app.ollama.undo.remove(0);
                }
                app.ollama.undo.push(UndoMark { document: app.session.document_id(), revision: app.session.rev(), depth: app.session.undo_depth() });
                app.ollama.entry("Activity", label);
            }
            app.canvas.want_focus = false;
            if mutates {
                let preview = clip(&app.session.doc.plain_text(wordcraft_doc::StoryRef::Body), 12_000);
                Ok(
                    json!({"command":id,"result":result,"documentTextPreview":preview,"revision":app.session.rev(),"verify":"Read the document to verify the requested change before reporting success."}),
                )
            } else {
                Ok(result)
            }
        }
        _ => Err("Unknown document tool.".into()),
    }
}

pub fn poll(app: &mut WordApp, ctx: &egui::Context) {
    if app.ollama.document.is_some_and(|d| d != app.session.document_id()) {
        app.ollama.stop();
        app.ollama.document = Some(app.session.document_id());
        app.ollama.entries.clear();
        app.ollama.sources.clear();
        app.ollama.undo.clear();
        app.ollama.allow_edits = false;
        app.ollama.status = "Document changed. AI editing is off.".into();
    }
    let Some(rx) = app.ollama.events.take() else { return };
    for _ in 0..32 {
        let event = match rx.try_recv() {
            Ok(event) => event,
            Err(std::sync::mpsc::TryRecvError::Empty) => break,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                app.ollama.busy = false;
                if app.ollama.status == "Working…" {
                    app.ollama.status = "Connection closed before a reply.".into();
                }
                break;
            }
        };
        match event {
            Event::Usage(label) => app.ollama.usage = label,
            Event::Account { label, signed_in } => {
                app.ollama.account = label;
                app.ollama.signed_in = signed_in;
            }
            Event::Login(url) => {
                ctx.open_url(egui::OpenUrl::new_tab(url));
                app.ollama.status = "Complete sign-in in your browser…".into();
            }
            Event::Models(result) => {
                app.ollama.busy = false;
                match result {
                    Ok(models) => {
                        if !models.contains(&app.ollama.model) {
                            app.ollama.model = models.first().cloned().unwrap_or_default();
                        }
                        app.ollama.models = models;
                        app.ollama.status = if app.ollama.models.is_empty() {
                            "No models found. Install a local model, or sign in to Ollama and add a cloud model. You can also enter an API key for direct cloud access.".into()
                        } else {
                            "Ready.".into()
                        };
                    }
                    Err(error) => app.ollama.status = error,
                }
            }
            Event::Message(text) => {
                app.ollama.live_response.clear();
                app.ollama.entry("Assistant", text);
            }
            Event::Partial(text) => app.ollama.live_response = text,
            Event::Sources(sources) => {
                for source in sources {
                    if app.ollama.sources.len() < 24 && !app.ollama.sources.iter().any(|(_, url)| url == &source.1) {
                        app.ollama.sources.push(source);
                    }
                }
            }
            Event::Tool { name, args, reply } => {
                let result = match document_tool(app, &name, &args) {
                    Ok(v) => json!({"ok":true,"result":v}),
                    Err(e) => json!({"ok":false,"error":e}),
                };
                if result.get("ok").and_then(Value::as_bool) == Some(false) {
                    app.ollama.entry("Activity", result.get("error").and_then(Value::as_str).unwrap_or("Tool failed").into());
                }
                let _ = reply.send(result);
            }
            Event::Finished(result) => {
                app.ollama.busy = false;
                app.ollama.live_response.clear();
                match result {
                    Ok(history) => {
                        app.ollama.history = history;
                        app.ollama.status = "Ready.".into();
                    }
                    Err(error) => {
                        app.ollama.status = error.clone();
                        app.ollama.entry("Error", error);
                        app.ollama.history.clear();
                    }
                }
            }
        }
    }
    if app.ollama.busy {
        app.ollama.events = Some(rx);
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn refresh(app: &mut WordApp, ctx: &egui::Context) {
    app.ollama.stop();
    app.ollama.busy = true;
    app.ollama.status = "Loading models…".into();
    app.ollama.cancel = Arc::new(AtomicBool::new(false));
    app.ollama.events = Some(if app.ollama.source == Source::Codex {
        codex::models(&app.ollama, ctx.clone(), false)
    } else {
        transport::models(&app.ollama, ctx.clone())
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn send(app: &mut WordApp, ctx: &egui::Context) {
    let input = app.ollama.input.trim().to_string();
    if input.is_empty() || app.ollama.busy {
        return;
    }
    if input.chars().count() > MAX_INPUT {
        app.ollama.status = "Please keep messages under 16,000 characters.".into();
        return;
    }
    if app.ollama.model.is_empty() {
        app.ollama.status = "Select a model first.".into();
        return;
    }
    if app.ollama.history.len() > MAX_MESSAGES {
        app.ollama.history.clear();
    }
    app.ollama.document = Some(app.session.document_id());
    app.ollama.turn_revision = Some(app.session.rev());
    app.ollama.context = json!({"title":document_title(app),"documentId":app.session.document_id(),"revision":app.session.rev(),
        "textPreview":clip(&app.session.doc.plain_text(wordcraft_doc::StoryRef::Body),12_000),
        "selectedText":clip(&app.session.selected_text(),2000),"selection":app.session.sel,"allowEdits":app.ollama.allow_edits});
    app.ollama.cancel = Arc::new(AtomicBool::new(false));
    app.ollama.history.push(json!({"role":"user","content":input}));
    app.ollama.entry("You", input);
    app.ollama.input.clear();
    app.ollama.busy = true;
    app.ollama.status = "Working…".into();
    app.ollama.events =
        Some(if app.ollama.source == Source::Codex { codex::chat(&app.ollama, ctx.clone()) } else { transport::chat(&app.ollama, ctx.clone()) });
}

fn source_name(source: Source) -> &'static str {
    match source {
        Source::Codex => "Codex",
        Source::Local => "Local Ollama",
        Source::Cloud => "Ollama Cloud",
        Source::LlamaCpp => "llama.cpp",
    }
}

fn response_text(ui: &mut Ui, text: &str, t: &Tokens) {
    let mut code = false;
    for line in text.lines().take(2000) {
        if line.trim_start().starts_with("```") {
            code = !code;
            continue;
        }
        if line.is_empty() {
            ui.add_space(6.0);
            continue;
        }
        let (line, heading) = if let Some(s) = line.strip_prefix("### ") {
            (s, 16.0)
        } else if let Some(s) = line.strip_prefix("## ") {
            (s, 18.0)
        } else if let Some(s) = line.strip_prefix("# ") {
            (s, 20.0)
        } else {
            (line, 14.0)
        };
        let mut job = egui::text::LayoutJob::default();
        for (i, part) in line.split("**").enumerate() {
            job.append(
                part,
                0.0,
                egui::TextFormat {
                    font_id: if code { egui::FontId::monospace(12.0) } else { egui::FontId::proportional(heading) },
                    color: if code { t.text_dim } else { t.text },
                    extra_letter_spacing: if i % 2 == 1 { 0.25 } else { 0.0 },
                    ..Default::default()
                },
            );
        }
        ui.add(egui::Label::new(job).wrap().selectable(true));
    }
}

fn message_card(ui: &mut Ui, role: &str, text: &str, t: &Tokens) {
    ui.add_space(12.0);
    let user = role == "You";
    let activity = role == "Activity" || role == "Error";
    let frame = egui::Frame::NONE.fill(if user { t.hover } else { t.panel }).corner_radius(16).inner_margin(12);
    frame.show(ui, |ui| {
        ui.set_width((ui.available_width() - 2.0).max(20.0));
        ui.label(RichText::new(role).size(11.0).strong().color(if role == "Error" { t.red } else { t.text_dim }));
        ui.add_space(4.0);
        if role == "Assistant" {
            response_text(ui, text, t);
        } else {
            ui.add(egui::Label::new(RichText::new(text).size(if activity { 12.0 } else { 14.0 })).wrap().selectable(true));
        }
        if !activity {
            ui.horizontal(|ui| {
                if ui.small_button("Copy").on_hover_text("Copy this message").clicked() {
                    ui.ctx().copy_text(text.to_string());
                }
            });
        }
    });
}

pub fn show(app: &mut WordApp, ui: &mut Ui) {
    if !app.ollama.open {
        return;
    }
    let t = Tokens::get(ui.ctx());
    egui::Panel::right("ollama_chat")
        .default_size(400.0)
        .size_range(330.0..=640.0)
        .resizable(true)
        .frame(egui::Frame::NONE.fill(t.panel).inner_margin(16).stroke(Stroke::new(1.0, t.border)))
        .show(ui, |ui| {
            if crate::panes::header(ui, "Agent chat") {
                app.ollama.open = false;
            }
            if cfg!(target_arch = "wasm32") {
                ui.label("Agent chat is available in the desktop app.");
                return;
            }
            let ctx = ui.ctx().clone();
            ui.add_space(8.0);
            ui.add_enabled_ui(!app.ollama.busy, |ui| {
                ui.horizontal(|ui| {
                    let old = app.ollama.source;
                    egui::ComboBox::from_id_salt("agent_provider").selected_text(RichText::new(source_name(old)).strong()).width(140.0).show_ui(
                        ui,
                        |ui| {
                            for source in [Source::Codex, Source::Local, Source::Cloud, Source::LlamaCpp] {
                                ui.selectable_value(&mut app.ollama.source, source, source_name(source));
                            }
                        },
                    );
                    if old != app.ollama.source {
                        app.ollama.models.clear();
                        app.ollama.model.clear();
                        app.ollama.history.clear();
                        app.ollama.entries.clear();
                        app.ollama.sources.clear();
                        #[cfg(not(target_arch = "wasm32"))]
                        refresh(app, &ctx);
                    }
                    if ui.button("New chat").clicked() {
                        app.ollama.history.clear();
                        app.ollama.entries.clear();
                        app.ollama.sources.clear();
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    if ui.small_button("↻").on_hover_text("Refresh models and connection").clicked() {
                        refresh(app, &ctx);
                    }
                });
                ui.add_space(6.0);
                let old_model = app.ollama.model.clone();
                egui::ComboBox::from_id_salt("ollama_model")
                    .truncate()
                    .width(ui.available_width() - 12.0)
                    .selected_text(if old_model.is_empty() { "Choose a model" } else { &old_model })
                    .show_ui(ui, |ui| {
                        for model in &app.ollama.models {
                            ui.selectable_value(&mut app.ollama.model, model.clone(), model);
                        }
                    });
                if old_model != app.ollama.model {
                    app.ollama.history.clear();
                    app.ollama.entry("Activity", "Model changed. Started a new chat context.".into());
                }
                if app.ollama.source == Source::Codex {
                    ui.add_space(4.0);
                    ui.small(
                        RichText::new(if app.ollama.account.is_empty() { "Connect your ChatGPT subscription" } else { &app.ollama.account })
                            .color(t.text_dim),
                    );
                    #[cfg(not(target_arch = "wasm32"))]
                    if !app.ollama.signed_in
                        && ui.button("Sign in with ChatGPT").clicked()
                        && let Err(e) = command(app, "ui.codexSignIn", &json!({}))
                    {
                        app.ollama.status = e;
                    }
                }
                ui.collapsing("Connection settings", |ui| {
                    if app.ollama.source == Source::Codex {
                        if !app.ollama.usage.is_empty() {
                            ui.small(&app.ollama.usage);
                        }
                        ui.small("Uses your installed Codex and ChatGPT account. Your Codex usage limits apply. Document context is sent to OpenAI.");
                        ui.hyperlink_to("Manage Codex usage", "https://chatgpt.com/codex/settings/usage");
                    } else {
                        if app.ollama.source == Source::Local {
                            ui.label("Ollama server");
                            ui.add(egui::TextEdit::singleline(&mut app.ollama.endpoint).desired_width(f32::INFINITY));
                        }
                        if app.ollama.source == Source::LlamaCpp {
                            ui.label("llama.cpp server");
                            ui.add(egui::TextEdit::singleline(&mut app.ollama.llama_endpoint).desired_width(f32::INFINITY));
                        }
                        ui.label("Ollama API key (optional)");
                        ui.add(egui::TextEdit::singleline(&mut app.ollama.api_key).password(true).desired_width(f32::INFINITY));
                        ui.small("The key stays in memory. Cloud uses signed-in Ollama without a key. Refresh after changing connection settings.");
                        ui.hyperlink_to("Ollama account", "https://ollama.com/settings/keys");
                    }
                });
            });
            ui.add_space(8.0);
            egui::Frame::NONE.fill(t.hover).corner_radius(12).inner_margin(10).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut app.ollama.allow_edits, "Allow AI editing");
                    if ui.add_enabled(can_undo(app), egui::Button::new("Undo change")).on_hover_text("Undo the last AI document change").clicked()
                        && let Err(e) = undo(app)
                    {
                        app.ollama.status = e;
                    }
                });
                ui.small(
                    RichText::new(if app.ollama.allow_edits {
                        "Edits appear live · every change is undoable"
                    } else {
                        "Read only · your document is protected"
                    })
                    .color(if app.ollama.allow_edits { t.text_dim } else { t.green }),
                );
                if ui.checkbox(&mut app.ollama.internet, "Search the web").changed() && app.ollama.busy && !app.ollama.internet {
                    app.ollama.stop();
                }
            });
            ui.add_space(8.0);
            let conversation_height = (ui.available_height() - 165.0).max(50.0);
            egui::ScrollArea::vertical()
                .id_salt("ollama_messages")
                .stick_to_bottom(true)
                .max_height(conversation_height)
                .min_scrolled_height(conversation_height)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    if app.ollama.entries.is_empty() {
                        ui.add_space(32.0);
                        ui.label(RichText::new("What are we writing?").size(23.0).strong());
                        ui.add_space(8.0);
                        ui.label(RichText::new("Draft, refine, or research with your document beside you.").size(14.0).color(t.text_dim));
                        ui.add_space(18.0);
                        for (label, prompt) in [
                            ("Summarize this document", "Summarize the current document."),
                            ("Polish my writing", "Suggest improvements to the selected passage, or the document if nothing is selected."),
                            ("Research a topic", "Help me research a topic for this document. Ask which topic first."),
                        ] {
                            if ui.add_sized([ui.available_width(), 34.0], egui::Button::new(label).corner_radius(10)).clicked() {
                                app.ollama.input = prompt.into();
                            }
                            ui.add_space(6.0);
                        }
                    }
                    for (role, text) in &app.ollama.entries {
                        message_card(ui, role, text, &t);
                    }
                    if !app.ollama.live_response.is_empty() {
                        message_card(ui, "Assistant", &app.ollama.live_response, &t);
                    }
                    if app.ollama.busy {
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.small("Working on your request…");
                        });
                    }
                    if !app.ollama.sources.is_empty() {
                        ui.add_space(12.0);
                        ui.collapsing("Sources", |ui| {
                            for (title, url) in &app.ollama.sources {
                                ui.hyperlink_to(title, url);
                            }
                        });
                    }
                });
            ui.add_space(10.0);
            egui::Frame::NONE.fill(t.input).stroke(Stroke::new(1.0, t.input_border)).corner_radius(18).inner_margin(12).show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut app.ollama.input)
                        .frame(egui::Frame::NONE)
                        .hint_text("Message your document assistant…")
                        .desired_rows(3)
                        .desired_width(f32::INFINITY)
                        .char_limit(MAX_INPUT),
                );
                ui.horizontal(|ui| {
                    ui.small(RichText::new(if app.ollama.allow_edits { "Editing enabled" } else { "Read only" }).color(t.text_dim));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if app.ollama.busy {
                            if ui.button("Stop").clicked() {
                                app.ollama.stop();
                            }
                        } else {
                            #[cfg(not(target_arch = "wasm32"))]
                            if ui
                                .add_enabled(
                                    !app.ollama.model.is_empty() && !app.ollama.input.trim().is_empty(),
                                    egui::Button::new(RichText::new("Send ↑").color(t.on_accent)).fill(t.accent).corner_radius(12),
                                )
                                .clicked()
                            {
                                send(app, &ctx);
                            }
                        }
                    });
                });
            });
            ui.add_space(5.0);
            ui.add(egui::Label::new(RichText::new(&app.ollama.status).size(11.0).color(t.text_dim)).wrap());
        });
}

/// Session diagnostics intentionally exclude API keys.
pub fn inspect(app: &WordApp) -> Value {
    json!({"open":app.ollama.open,"model":app.ollama.model,"source":match app.ollama.source {Source::Cloud=>"cloud",Source::Local=>"local",Source::LlamaCpp=>"llamacpp",Source::Codex=>"codex"},
        "busy":app.ollama.busy,"status":app.ollama.status,"allowEdits":app.ollama.allow_edits,"internet":app.ollama.internet,
        "account":app.ollama.account,"signedIn":app.ollama.signed_in,"usage":app.ollama.usage,"canUndo":can_undo(app),"models":app.ollama.models,"messages":app.ollama.entries,"sources":app.ollama.sources})
}

pub fn commands(app: &WordApp) -> Vec<Value> {
    [
        ("ui.codexConnect", "Codex connection", r#"{"value"?:bool}"#),
        ("ui.codexChat", "Codex Chat", r#"{"value"?:bool}"#),
        ("ui.codexSignIn", "Sign in with ChatGPT", "{}"),
        ("ui.ollama", "Agent Chat", r#"{"value"?:bool}"#),
        (
            "ui.ollamaSettings",
            "Ollama connection settings",
            r#"{"source"?:"local|cloud|llamacpp|codex","endpoint"?:string,"llamaEndpoint"?:string,"model"?:string,"apiKey"?:string}"#,
        ),
        ("ui.ollamaRefresh", "Refresh Ollama models", "{}"),
        ("ui.ollamaSend", "Send Ollama message", r#"{"text"?:string}"#),
        ("ui.ollamaStop", "Stop Ollama", "{}"),
        ("ui.ollamaNewChat", "New Ollama chat", "{}"),
        ("ui.ollamaEdit", "Allow AI editing", r#"{"value"?:bool}"#),
        ("ui.ollamaInternet", "Internet access", r#"{"value"?:bool}"#),
        ("ui.ollamaUndo", "Undo AI change", "{}"),
    ]
    .into_iter()
    .map(|(id, label, params)| {
        json!({"id":id,"label":label,"location":"Agents","params":params,
        "enabled":!cfg!(target_arch="wasm32") && (id!="ui.ollamaUndo" || can_undo(app))})
    })
    .collect()
}

pub(crate) fn command(app: &mut WordApp, id: &str, p: &Value) -> Result<Value, String> {
    match id {
        "ui.codexChat" => {
            if app.ollama.source != Source::Codex {
                app.ollama.stop();
                app.ollama.source = Source::Codex;
                app.ollama.model.clear();
                app.ollama.models.clear();
                app.ollama.entries.clear();
                app.ollama.sources.clear();
            }
            app.ollama.open = p.get("value").and_then(Value::as_bool).unwrap_or(true);
            if !app.ollama.open {
                app.ollama.stop();
            }
            #[cfg(not(target_arch = "wasm32"))]
            if app.ollama.open && app.ollama.models.is_empty() && !app.ollama.busy {
                let ctx = app.ctx.clone().ok_or("The app is not ready yet")?;
                refresh(app, &ctx);
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        "ui.codexSignIn" => {
            if app.ollama.busy {
                return Err("Stop the current request before signing in.".into());
            }
            let ctx = app.ctx.clone().ok_or("The app is not ready yet")?;
            app.ollama.source = Source::Codex;
            app.ollama.open = true;
            app.ollama.cancel = Arc::new(AtomicBool::new(false));
            app.ollama.busy = true;
            app.ollama.status = "Connecting to ChatGPT…".into();
            app.ollama.events = Some(codex::models(&app.ollama, ctx, true));
        }
        "ui.ollamaStop" => app.ollama.stop(),
        "ui.ollamaInternet" => {
            app.ollama.internet = p.get("value").and_then(Value::as_bool).unwrap_or(!app.ollama.internet);
            if !app.ollama.internet && app.ollama.busy {
                app.ollama.stop();
            }
        }
        "ui.ollamaNewChat" => {
            app.ollama.stop();
            app.ollama.history.clear();
            app.ollama.entries.clear();
            app.ollama.sources.clear();
            app.ollama.status = "Ready.".into();
        }
        "ui.ollamaSettings" => {
            if app.ollama.busy {
                return Err("Stop the current request before changing connection settings.".into());
            }
            if let Some(source) = p.get("source").and_then(Value::as_str) {
                app.ollama.source = match source {
                    "local" => Source::Local,
                    "cloud" => Source::Cloud,
                    "llamacpp" => Source::LlamaCpp,
                    "codex" => Source::Codex,
                    _ => return Err("source must be local, cloud, llamacpp or codex".into()),
                };
            }
            for (name, target, max) in [
                ("endpoint", &mut app.ollama.endpoint, 2000),
                ("llamaEndpoint", &mut app.ollama.llama_endpoint, 2000),
                ("model", &mut app.ollama.model, 256),
                ("apiKey", &mut app.ollama.api_key, 2000),
            ] {
                if let Some(text) = p.get(name).and_then(Value::as_str) {
                    if text.len() > max {
                        return Err(format!("{name} is too long"));
                    }
                    *target = text.into();
                }
            }
            app.ollama.history.clear();
            app.ollama.models.clear();
            if !app.ollama.entries.is_empty() {
                app.ollama.entry("Activity", "Connection changed. Started a new chat context.".into());
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        "ui.ollamaRefresh" => {
            let ctx = app.ctx.clone().ok_or("The app is not ready yet")?;
            refresh(app, &ctx);
        }
        #[cfg(not(target_arch = "wasm32"))]
        "ui.ollamaSend" => {
            if let Some(text) = p.get("text").and_then(Value::as_str) {
                if text.chars().count() > MAX_INPUT {
                    return Err("Message is too long".into());
                }
                app.ollama.input = text.into();
            }
            if app.ollama.busy {
                return Err("A request is already running".into());
            }
            let ctx = app.ctx.clone().ok_or("The app is not ready yet")?;
            send(app, &ctx);
            if !app.ollama.busy {
                return Err(app.ollama.status.clone());
            }
        }
        _ => return Err("Ollama requires the desktop app".into()),
    }
    Ok(inspect(app))
}

/// A typed writing skill for small models: validate first, apply one editing command, then
/// compare the inserted fragment with the requested text. A failed verification is rolled back.
fn verified_write(app: &mut WordApp, args: &Value) -> Result<Value, String> {
    if !app.ollama.allow_edits {
        return Err("AI editing is disabled.".into());
    }
    if app.ollama.turn_revision != Some(app.session.rev()) {
        return Err("Read the changed document again before editing.".into());
    }
    let text = args.get("text").and_then(Value::as_str).ok_or("text is required")?;
    if text.len() > 64_000 {
        return Err("Writing is limited to 64,000 bytes per action.".into());
    }
    let mode = args.get("mode").and_then(Value::as_str).ok_or("mode is required")?;
    if !matches!(mode, "append" | "replace_selection" | "replace_document") {
        return Err("Invalid writing mode.".into());
    }
    let snapshot = app.session.edit_snapshot();
    let pending = app.session.pending.clone();
    let marks = app.ollama.undo.len();
    let result = (|| {
        if mode == "append" {
            let end = wordcraft_engine::cmd::pos_json(&app.session.doc.end_of(wordcraft_doc::StoryRef::Body));
            document_tool(app, "execute_command", &json!({"command":"caret.set","params":{"pos":end}}))?;
        }
        if mode == "replace_selection" && app.session.sel.is_collapsed() {
            return Err("Select the passage to replace first, or choose append.".into());
        }
        let start = app.session.sel.ordered().0;
        document_tool(
            app,
            "execute_command",
            &json!({"command":if mode=="replace_document"{"document.setText"}else{"text.insert"},"params":{"text":text}}),
        )?;
        let actual = if mode == "replace_document" {
            app.session.doc.plain_text(wordcraft_doc::StoryRef::Body)
        } else {
            app.session.doc.copy_range(&start, &app.session.sel.focus).plain_text()
        };
        if actual != text {
            return Err("Writing verification failed; the change was reverted.".into());
        }
        Ok(
            json!({"verified":true,"mode":mode,"insertedCharacters":text.chars().count(),"documentTextPreview":clip(&app.session.doc.plain_text(wordcraft_doc::StoryRef::Body),12_000)}),
        )
    })();
    if result.is_err() {
        app.session.restore(snapshot);
        app.session.pending = pending;
        app.ollama.undo.truncate(marks);
        app.ollama.turn_revision = Some(app.session.rev());
        if let Some(mark) = app.ollama.undo.last_mut() {
            mark.revision = app.session.rev();
        }
    }
    result
}

#[cfg(test)]
mod tests;

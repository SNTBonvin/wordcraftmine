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
mod project;
#[cfg(not(target_arch = "wasm32"))]
mod transport;

mod edits;
mod markdown;

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
    pub effort: String,
    pub project_folder: String,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            source: Source::Local,
            endpoint: "http://127.0.0.1:11434".into(),
            llama_endpoint: "http://127.0.0.1:8085".into(),
            model: String::new(),
            internet: true,
            effort: String::new(),
            project_folder: String::new(),
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
    pub effort: String,
    pub project_folder: String,
    pub capabilities: Vec<ModelOptions>,
    pub phase: String,
    pub thinking: String,
    pub review: edits::Review,
    markdown: egui_commonmark::CommonMarkCache,
    show_changes: bool,
    project_excerpts: Vec<(String, String)>,
    selected_source: Option<String>,
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
            effort: String::new(),
            project_folder: String::new(),
            capabilities: Vec::new(),
            phase: String::new(),
            thinking: String::new(),
            review: edits::Review::default(),
            markdown: egui_commonmark::CommonMarkCache::default(),
            show_changes: false,
            project_excerpts: Vec::new(),
            selected_source: None,
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

#[derive(Clone, Debug, serde::Serialize)]
pub struct ModelOptions {
    pub model: String,
    pub display_name: String,
    pub default_effort: String,
    pub efforts: Vec<(String, String)>,
}

pub(super) enum Event {
    Models(Result<Vec<String>, String>),
    Capabilities(Vec<ModelOptions>),
    Phase(String),
    Thinking(String),
    Account { label: String, signed_in: bool },
    Login(String),
    Usage(String),
    Message(String),
    Partial(String),
    Sources(Vec<(String, String)>),
    ProjectSource { title: String, url: String, text: String },
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
            effort: self.effort.clone(),
            project_folder: self.project_folder.clone(),
        }
    }
    pub fn apply_preferences(&mut self, p: &Preferences) {
        self.stop();
        self.source = p.source;
        self.endpoint = clip(&p.endpoint, 2000);
        self.llama_endpoint = clip(&p.llama_endpoint, 2000);
        self.model = clip(&p.model, 256);
        self.internet = p.internet;
        self.effort = clip(&p.effort, 64);
        self.project_folder = clip(&p.project_folder, 4096);
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
            self.markdown = Default::default();
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
    app.ollama.review.refresh_after_undo(&app.session.doc, app.session.rev(), &app.session.sel);
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
            let before = mutates.then(|| app.session.doc.clone());
            let result = app.session.run_labeled(id, &params, &label).map_err(|e| e.to_string())?;
            app.ollama.turn_revision = Some(app.session.rev());
            if let Some(before) = before {
                app.ollama.review.record(&before, &app.session.doc, app.session.document_id(), app.session.rev(), &app.session.sel, &label);
                app.ollama.show_changes = false;
            }
            if mutates {
                if app.ollama.undo.len() >= 100 {
                    app.ollama.undo.remove(0);
                }
                app.ollama.undo.push(UndoMark { document: app.session.document_id(), revision: app.session.rev(), depth: app.session.undo_depth() });
                app.ollama.entry("Activity", label);
            }
            app.canvas.want_focus = false;
            if mutates {
                app.canvas.scroll_to_caret = true;
                app.ollama.phase = "Editing document".into();
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
        app.ollama.markdown = Default::default();
        app.ollama.thinking.clear();
        app.ollama.sources.clear();
        app.ollama.project_excerpts.clear();
        app.ollama.selected_source = None;
        app.ollama.undo.clear();
        app.ollama.review = edits::Review::default();
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
            Event::Capabilities(options) => {
                app.ollama.capabilities = options;
            }
            Event::Phase(phase) => app.ollama.phase = phase,
            Event::Thinking(text) => app.ollama.thinking = clip(&text, 24_000),
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
            Event::ProjectSource { title, url, text } => {
                if app.ollama.project_excerpts.len() < 96 && !app.ollama.project_excerpts.iter().any(|(u, _)| u == &url) {
                    app.ollama.markdown.add_link_hook(&url);
                    app.ollama.sources.push((title, url.clone()));
                    app.ollama.project_excerpts.push((url, text));
                }
            }
            Event::Message(text) => {
                let summary = std::mem::take(&mut app.ollama.thinking);
                if !summary.is_empty() {
                    app.ollama.entry("Thinking summary", summary);
                }
                app.ollama.live_response.clear();
                app.ollama.entry("Assistant", text);
            }
            Event::Partial(text) => app.ollama.live_response = text,
            Event::Sources(sources) => {
                for source in sources {
                    if app.ollama.sources.len() < 96 && !app.ollama.sources.iter().any(|(_, url)| url == &source.1) {
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
    app.ollama.review.begin_turn();
    app.ollama.phase = "Thinking".into();
    app.ollama.thinking.clear();
    app.ollama.document = Some(app.session.document_id());
    app.ollama.turn_revision = Some(app.session.rev());
    app.ollama.context = json!({"title":document_title(app),"documentId":app.session.document_id(),"revision":app.session.rev(),
        "textPreview":clip(&app.session.doc.plain_text(wordcraft_doc::StoryRef::Body),12_000),
        "selectedText":clip(&app.session.selected_text(),2000),"selection":app.session.sel,"allowEdits":app.ollama.allow_edits,"linkedProject":if app.ollama.project_folder.is_empty() { Value::Null } else { json!({"name":std::path::Path::new(&app.ollama.project_folder).file_name().map(|n| n.to_string_lossy().into_owned()),"tools":["project_list","project_search","project_read"]}) }});
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

fn effort_label(effort: &str) -> &str {
    match effort {
        "" => "Auto",
        "low" => "Light",
        "medium" => "Medium",
        "high" => "High",
        "xhigh" => "Extra high",
        "max" => "Max",
        "ultra" => "Ultra",
        "none" => "Off",
        other => other,
    }
}

fn message(ui: &mut Ui, role: &str, text: &str, t: &Tokens, cache: &mut egui_commonmark::CommonMarkCache) {
    ui.add_space(16.0);
    if role == "Thinking summary" {
        ui.collapsing("Thought summary", |ui| {
            markdown::show(ui, cache, text);
        });
        return;
    }
    if role == "Activity" {
        ui.horizontal(|ui| {
            ui.colored_label(t.green, "✓");
            ui.small(RichText::new(text).color(t.text_dim));
        });
        return;
    }
    if role == "Error" {
        ui.colored_label(t.red, text);
        return;
    }
    if role == "You" {
        ui.with_layout(egui::Layout::top_down(egui::Align::RIGHT), |ui| {
            egui::Frame::NONE.fill(t.hover).corner_radius(14).inner_margin(egui::Margin::symmetric(14, 10)).show(ui, |ui| {
                ui.set_max_width((ui.available_width() - 28.0).max(60.0));
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.add(egui::Label::new(text).wrap().selectable(true));
                });
            });
        });
    } else {
        markdown::show(ui, cache, text);
        ui.add_space(6.0);
        if ui.add(egui::Button::new(RichText::new("Copy").size(11.0).color(t.text_dim)).frame(false)).clicked() {
            ui.ctx().copy_text(text.to_string());
        }
    }
}

fn connection_menu(app: &mut WordApp, ui: &mut Ui, ctx: &egui::Context) {
    ui.label(RichText::new("Provider").strong());
    let old = app.ollama.source;
    for source in [Source::Codex, Source::Local, Source::Cloud, Source::LlamaCpp] {
        ui.selectable_value(&mut app.ollama.source, source, source_name(source));
    }
    if old != app.ollama.source {
        app.ollama.models.clear();
        app.ollama.model.clear();
        app.ollama.capabilities.clear();
        app.ollama.effort.clear();
        app.ollama.history.clear();
        app.ollama.entries.clear();
        app.ollama.markdown = Default::default();
        app.ollama.thinking.clear();
        app.ollama.sources.clear();
        app.ollama.project_excerpts.clear();
        app.ollama.selected_source = None;
        #[cfg(not(target_arch = "wasm32"))]
        refresh(app, ctx);
    }
    ui.separator();
    if app.ollama.source == Source::Codex {
        ui.label(&app.ollama.account);
        if !app.ollama.usage.is_empty() {
            ui.small(&app.ollama.usage);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if !app.ollama.signed_in
            && ui.button("Sign in with ChatGPT").clicked()
            && let Err(e) = command(app, "ui.codexSignIn", &json!({}))
        {
            app.ollama.status = e;
        }
        ui.small("Uses your Codex subscription. Document context is sent to OpenAI.");
        ui.hyperlink_to("Manage usage", "https://chatgpt.com/codex/settings/usage");
    } else {
        if app.ollama.source == Source::Local {
            ui.label("Ollama address");
            if ui.text_edit_singleline(&mut app.ollama.endpoint).changed() {
                app.ollama.history.clear();
                app.ollama.models.clear();
            }
        }
        if app.ollama.source == Source::LlamaCpp {
            ui.label("llama.cpp address");
            if ui.text_edit_singleline(&mut app.ollama.llama_endpoint).changed() {
                app.ollama.history.clear();
                app.ollama.models.clear();
            }
        }
        ui.label("Ollama key · optional");
        ui.add(egui::TextEdit::singleline(&mut app.ollama.api_key).password(true));
        ui.small("Stored in memory for this session.");
    }
    #[cfg(not(target_arch = "wasm32"))]
    if ui.button("Refresh models").clicked() {
        refresh(app, ctx);
        ui.close();
    }
}

fn changes_view(app: &mut WordApp, ui: &mut Ui, t: &Tokens) {
    if app.ollama.review.marks.is_empty() {
        return;
    }
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        if ui
            .add(egui::Button::new(RichText::new(format!("{} document changes  ▾", app.ollama.review.marks.len())).color(t.green)).frame(false))
            .clicked()
        {
            app.ollama.show_changes = !app.ollama.show_changes;
        }
        if ui.add_enabled(can_undo(app), egui::Button::new("Undo").frame(false)).clicked()
            && let Err(e) = undo(app)
        {
            app.ollama.status = e;
        }
        if ui.button("Dismiss").clicked() {
            let _ = command(app, "ui.agentDismissChanges", &json!({}));
        }
    });
    if app.ollama.show_changes {
        ui.small("Green: added · red: removed · purple: formatting");
        if app.ollama.review.revision != app.session.rev() {
            ui.small("Highlights cleared after subsequent manual edits. Changes below are the AI's earlier edits.");
        }
        let mut show = None;
        for (index, mark) in app.ollama.review.marks.iter().enumerate() {
            egui::Frame::NONE.stroke(Stroke::new(1.0, t.border)).corner_radius(8).inner_margin(10).show(ui, |ui| {
                ui.set_width((ui.available_width() - 2.0).max(20.0));
                if ui.add_enabled(app.ollama.review.revision == app.session.rev(), egui::Button::new("Show in document").frame(false)).clicked() {
                    show = Some(index);
                }
                if mark.formatting {
                    ui.small("Formatting / layout changed");
                }
                if !mark.removed.is_empty() {
                    ui.add(egui::Label::new(RichText::new(format!("− {}", mark.removed)).color(t.red).strikethrough()).wrap().selectable(true));
                }
                if !mark.added.is_empty() {
                    ui.add(egui::Label::new(RichText::new(format!("+ {}", mark.added)).color(t.green)).wrap().selectable(true));
                }
            });
            ui.add_space(5.0);
        }
        if let Some(index) = show
            && let Err(error) = command(app, "ui.agentShowChange", &json!({"index":index}))
        {
            app.ollama.status = error;
        }
        if app.ollama.review.truncated {
            ui.small("Large changes are summarized; not every region is highlighted.");
        }
    }
}

pub fn show(app: &mut WordApp, ui: &mut Ui) {
    if !app.ollama.open {
        return;
    }
    let t = Tokens::get(ui.ctx());
    egui::Panel::right("agent_chat_v2")
        .default_size(440.0)
        .size_range(340.0..=680.0)
        .resizable(true)
        .frame(egui::Frame::NONE.fill(t.panel).inner_margin(18).stroke(Stroke::new(1.0, t.border)))
        .show(ui, |ui| {
            let ctx = ui.ctx().clone();
            for (url, _) in &app.ollama.project_excerpts {
                if app.ollama.markdown.get_link_hook(url).is_none() {
                    app.ollama.markdown.add_link_hook(url);
                }
            }
            // This scope changes only the chat: quiet, borderless controls retain native focus/hover.
            let visuals = ui.visuals_mut();
            for visual in [&mut visuals.widgets.inactive, &mut visuals.widgets.hovered] {
                visual.bg_stroke = Stroke::NONE;
                visual.corner_radius = egui::CornerRadius::same(8);
            }
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(source_name(app.ollama.source)).size(18.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Button::new("×").frame(false)).on_hover_text("Close chat").clicked() {
                        app.ollama.open = false;
                    }
                    ui.add_enabled_ui(!app.ollama.busy, |ui| {
                        ui.menu_button("•••", |ui| {
                            connection_menu(app, ui, &ctx);
                        });
                        if ui.add(egui::Button::new("New chat").frame(false)).clicked() {
                            let _ = command(app, "ui.ollamaNewChat", &json!({}));
                            app.ollama.markdown = Default::default();
                            app.ollama.thinking.clear();
                        }
                    });
                });
            });
            if cfg!(target_arch = "wasm32") {
                ui.label("Agent chat requires the desktop app.");
                return;
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.checkbox(&mut app.ollama.allow_edits, "Allow editing").on_hover_text("Hard permission switch: AI changes are rejected while off.");
                ui.label(
                    RichText::new(if app.ollama.allow_edits { "Live edits" } else { "Read only" }).size(11.0).color(if app.ollama.allow_edits {
                        t.accent_text
                    } else {
                        t.text_dim
                    }),
                );
            });
            ui.horizontal(|ui| {
                if app.ollama.project_folder.is_empty() {
                    if ui
                        .add(egui::Button::new("+ Link project").frame(false))
                        .on_hover_text("Use a local folder as read-only context. Retrieved passages are shared with the selected model.")
                        .clicked()
                    {
                        let _ = command(app, "ui.agentProject", &json!({}));
                    }
                } else {
                    let name = std::path::Path::new(&app.ollama.project_folder)
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "Project".into());
                    ui.add(egui::Label::new(RichText::new(format!("Project · {name}")).size(11.0).color(t.accent_text)).truncate()).on_hover_text(
                        "Files are read live when requested. Read-only context is shared with the selected model; project files cannot be edited.",
                    );
                    if ui.add(egui::Button::new("×").frame(false)).on_hover_text("Unlink project and clear its chat context").clicked() {
                        let _ = command(app, "ui.agentProject", &json!({"path":""}));
                    }
                }
            });
            ui.add_space(4.0);
            ui.separator();
            let extra = if app.ollama.models.is_empty() && !app.ollama.busy { 30.0 } else { 0.0 }
                + if app.ollama.status != "Ready." && !app.ollama.busy { 28.0 } else { 0.0 };
            let height = (ui.available_height() - 190.0 - extra).max(40.0);
            ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), height), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_min_height(height);
                egui::ScrollArea::vertical()
                    .id_salt("agent_conversation_v2")
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .max_height(height)
                    .min_scrolled_height(height)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                        if app.ollama.entries.is_empty() {
                            ui.add_space(55.0);
                            ui.label(RichText::new("Write something great.").size(22.0));
                            ui.add_space(6.0);
                            ui.label(RichText::new("Your document, with a thinking partner.").color(t.text_dim));
                            ui.add_space(22.0);
                            for (label, prompt) in [
                                ("Summarize", "Summarize the current document."),
                                ("Refine my writing", "Suggest improvements to the selected text, or the document if nothing is selected."),
                                ("Research", "Help me research a topic for this document. Ask which topic first."),
                            ] {
                                if ui.add(egui::Button::new(label).frame(false)).clicked() {
                                    app.ollama.input = prompt.into();
                                }
                            }
                        }
                        for (role, text) in &app.ollama.entries {
                            message(ui, role, text, &t, &mut app.ollama.markdown);
                            for (url, _) in &app.ollama.project_excerpts {
                                if app.ollama.markdown.get_link_hook(url) == Some(true) {
                                    app.ollama.selected_source = Some(url.clone());
                                }
                            }
                        }
                        if !app.ollama.thinking.is_empty() {
                            ui.add_space(10.0);
                            egui::CollapsingHeader::new(if app.ollama.busy { "Thinking · live summary" } else { "Thought summary" })
                                .id_salt(("live_thinking", app.ollama.entries.len()))
                                .default_open(true)
                                .show(ui, |ui| {
                                    markdown::show(ui, &mut app.ollama.markdown, &app.ollama.thinking);
                                });
                        }
                        if !app.ollama.live_response.is_empty() {
                            message(ui, "Assistant", &app.ollama.live_response, &t, &mut app.ollama.markdown);
                        }
                        if app.ollama.busy {
                            ui.add_space(12.0);
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(
                                    RichText::new(if app.ollama.phase.is_empty() { "Thinking…" } else { &app.ollama.phase })
                                        .size(12.0)
                                        .color(t.text_dim),
                                );
                            });
                        }
                        changes_view(app, ui, &t);
                        if !app.ollama.sources.is_empty() {
                            ui.add_space(8.0);
                            ui.collapsing("Sources", |ui| {
                                for (index, (title, url)) in app.ollama.sources.iter().enumerate() {
                                    egui::Frame::NONE.fill(t.hover.gamma_multiply(0.4)).corner_radius(8).inner_margin(8).show(ui, |ui| {
                                        if url.starts_with("#project-") {
                                            if ui
                                                .add(egui::Button::new(format!("{} · {title}", index.saturating_add(1))).wrap().frame(false))
                                                .clicked()
                                            {
                                                app.ollama.selected_source = Some(url.clone());
                                            }
                                            ui.small(RichText::new("Linked project · retrieved passage").color(t.text_dim));
                                        } else {
                                            ui.add(egui::Hyperlink::from_label_and_url(format!("{} · {title}", index.saturating_add(1)), url));
                                            let domain = url.split('/').nth(2).unwrap_or(url);
                                            ui.small(RichText::new(domain).color(t.text_dim));
                                        }
                                    });
                                }
                            });
                        }
                    });
            });
            if let Some(url) = &app.ollama.selected_source.clone() {
                egui::Window::new("Project source").id(egui::Id::new("agent_project_source")).default_width(500.0).resizable(true).show(&ctx, |ui| {
                    if ui.button("Close").clicked() {
                        app.ollama.selected_source = None;
                    }
                    if let Some((_, text)) = app.ollama.project_excerpts.iter().find(|(u, _)| u == url) {
                        egui::ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                            ui.add(egui::Label::new(text).wrap().selectable(true));
                        });
                    }
                });
            }
            ui.add_space(10.0);
            egui::Frame::NONE.fill(t.hover.gamma_multiply(0.45)).stroke(Stroke::new(1.0, t.border)).corner_radius(14).inner_margin(12).show(
                ui,
                |ui| {
                    let response = ui.add(
                        egui::TextEdit::multiline(&mut app.ollama.input)
                            .frame(egui::Frame::NONE)
                            .hint_text("Ask anything about your document")
                            .desired_rows(3)
                            .desired_width(f32::INFINITY)
                            .char_limit(MAX_INPUT),
                    );
                    let submit = response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command);
                    ui.horizontal(|ui| {
                        if ui.checkbox(&mut app.ollama.internet, "Web").on_hover_text("Allow public web search and browsing").changed()
                            && app.ollama.busy
                            && !app.ollama.internet
                        {
                            app.ollama.stop();
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if app.ollama.busy {
                                if ui.add(egui::Button::new("■").frame(false)).on_hover_text("Stop").clicked() {
                                    app.ollama.stop();
                                }
                            } else {
                                #[cfg(not(target_arch = "wasm32"))]
                                if (ui
                                    .add_enabled(
                                        !app.ollama.model.is_empty() && !app.ollama.input.trim().is_empty(),
                                        egui::Button::new(RichText::new("↑").size(19.0).color(t.panel))
                                            .fill(t.text)
                                            .corner_radius(255)
                                            .min_size(egui::vec2(28.0, 28.0)),
                                    )
                                    .on_hover_text("Send · ⌘/Ctrl Enter")
                                    .clicked()
                                    || submit)
                                    && !app.ollama.input.trim().is_empty()
                                {
                                    send(app, &ctx);
                                }
                            }
                        });
                    });
                },
            );
            ui.add_space(6.0);
            ui.add_enabled_ui(!app.ollama.busy, |ui| {
                ui.horizontal(|ui| {
                    let old = app.ollama.model.clone();
                    let title = app.ollama.capabilities.iter().find(|o| o.model == old).map_or_else(|| old.clone(), |o| o.display_name.clone());
                    egui::ComboBox::from_id_salt("agent_model_v2")
                        .truncate()
                        .width((ui.available_width() - 160.0).max(90.0))
                        .selected_text(RichText::new(if title.is_empty() { "Select model" } else { &title }).size(11.0))
                        .show_ui(ui, |ui| {
                            for model in &app.ollama.models {
                                let label =
                                    app.ollama.capabilities.iter().find(|o| o.model == *model).map_or(model.as_str(), |o| o.display_name.as_str());
                                ui.selectable_value(&mut app.ollama.model, model.clone(), label);
                            }
                        });
                    if old != app.ollama.model {
                        app.ollama.effort.clear();
                        app.ollama.history.clear();
                    }
                    if app.ollama.source == Source::Codex {
                        let options = app.ollama.capabilities.iter().find(|o| o.model == app.ollama.model);
                        egui::ComboBox::from_id_salt("agent_effort")
                            .truncate()
                            .width(144.0)
                            .selected_text(RichText::new(format!("Thinking: {}", effort_label(&app.ollama.effort))).size(11.0))
                            .show_ui(ui, |ui| {
                                let label = options.map_or("Auto · model default".into(), |o| format!("Auto · {}", effort_label(&o.default_effort)));
                                ui.selectable_value(&mut app.ollama.effort, String::new(), label);
                                if let Some(options) = options {
                                    for (level, description) in &options.efforts {
                                        ui.selectable_value(&mut app.ollama.effort, level.clone(), effort_label(level)).on_hover_text(description);
                                    }
                                }
                            });
                    }
                });
            });
            if app.ollama.models.is_empty() && !app.ollama.busy {
                #[cfg(not(target_arch = "wasm32"))]
                if ui.button("Connect / load models").clicked() {
                    refresh(app, &ctx);
                }
            }
            if app.ollama.status != "Ready." && !app.ollama.busy {
                ui.add(egui::Label::new(RichText::new(&app.ollama.status).size(11.0).color(t.text_dim)).wrap());
            }
        });
}

/// Session diagnostics intentionally exclude API keys.
pub fn inspect(app: &WordApp) -> Value {
    json!({"open":app.ollama.open,"model":app.ollama.model,"source":match app.ollama.source {Source::Cloud=>"cloud",Source::Local=>"local",Source::LlamaCpp=>"llamacpp",Source::Codex=>"codex"},
        "busy":app.ollama.busy,"status":app.ollama.status,"allowEdits":app.ollama.allow_edits,"internet":app.ollama.internet,
        "account":app.ollama.account,"signedIn":app.ollama.signed_in,"usage":app.ollama.usage,"effort":app.ollama.effort,"capabilities":app.ollama.capabilities,"phase":app.ollama.phase,"thinking":app.ollama.thinking,"projectFolder":app.ollama.project_folder,"changes":app.ollama.review.inspect(app.session.rev()),"canUndo":can_undo(app),"models":app.ollama.models,"messages":app.ollama.entries,"sources":app.ollama.sources})
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
            r#"{"source"?:"local|cloud|llamacpp|codex","endpoint"?:string,"llamaEndpoint"?:string,"model"?:string,"effort"?:string,"apiKey"?:string}"#,
        ),
        ("ui.ollamaRefresh", "Refresh Ollama models", "{}"),
        ("ui.ollamaSend", "Send Ollama message", r#"{"text"?:string}"#),
        ("ui.ollamaStop", "Stop Ollama", "{}"),
        ("ui.ollamaNewChat", "New Ollama chat", "{}"),
        ("ui.ollamaEdit", "Allow AI editing", r#"{"value"?:bool}"#),
        ("ui.ollamaInternet", "Internet access", r#"{"value"?:bool}"#),
        ("ui.ollamaUndo", "Undo AI change", "{}"),
        ("ui.agentProject", "Link project folder", r#"{"path"?:string}"#),
        ("ui.agentDismissChanges", "Dismiss AI highlighting", "{}"),
        ("ui.agentShowChange", "Show AI change in document", r#"{"index"?:number}"#),
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
        #[cfg(not(target_arch = "wasm32"))]
        "ui.agentProject" => {
            if let Some(path) = p.get("path").and_then(Value::as_str) {
                let folder = if path.is_empty() { String::new() } else { project::root(path)?.to_string_lossy().into_owned() };
                app.ollama.stop();
                app.ollama.project_folder = folder;
                // Revocation clears prior retrieved content from the conversation sent to providers.
                app.ollama.entries.clear();
                app.ollama.sources.clear();
                app.ollama.project_excerpts.clear();
                app.ollama.selected_source = None;
                app.ollama.markdown = Default::default();
                app.ollama.thinking.clear();
                app.ollama.context = Value::Null;
                app.ollama.status = "Ready.".into();
            } else {
                let _ = app.ask_file(crate::FileDialogRequest::Folder, crate::file_dialogs::AfterPick::ProjectFolder);
            }
        }
        "ui.agentShowChange" => {
            if app.ollama.review.revision != app.session.rev() {
                return Err("The document has changed since this AI edit.".into());
            }
            let index = p.get("index").and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()).unwrap_or(0);
            let pos = app.ollama.review.marks.get(index).ok_or("No such AI change")?.start.clone();
            app.session.run("caret.set", &json!({"pos":wordcraft_engine::cmd::pos_json(&pos)})).map_err(|e| e.to_string())?;
            app.canvas.scroll_to_caret = true;
        }
        "ui.agentDismissChanges" => app.ollama.review = edits::Review::default(),
        "ui.codexChat" => {
            if app.ollama.source != Source::Codex {
                app.ollama.stop();
                app.ollama.source = Source::Codex;
                app.ollama.model.clear();
                app.ollama.models.clear();
                app.ollama.entries.clear();
                app.ollama.markdown = Default::default();
                app.ollama.thinking.clear();
                app.ollama.sources.clear();
                app.ollama.project_excerpts.clear();
                app.ollama.selected_source = None;
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
            app.ollama.markdown = Default::default();
            app.ollama.thinking.clear();
            app.ollama.sources.clear();
            app.ollama.project_excerpts.clear();
            app.ollama.selected_source = None;
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
                ("effort", &mut app.ollama.effort, 64),
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
    let review = app.ollama.review.clone();
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
        app.ollama.review = review;
        app.ollama.review.restore_revision(&app.session.doc, app.session.rev());
        app.ollama.turn_revision = Some(app.session.rev());
        if let Some(mark) = app.ollama.undo.last_mut() {
            mark.revision = app.session.rev();
        }
    }
    result
}

#[cfg(test)]
mod tests;

pub(crate) fn paint_edits(app: &WordApp, ui: &Ui, painter: &egui::Painter, layout: &wordcraft_layout::DocLayout, pages: &[egui::Rect], scale: f32) {
    edits::paint(app, ui, painter, layout, pages, scale);
}

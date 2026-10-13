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
            && app.session.undo_label().is_some_and(|s| s.starts_with("Ollama: "))
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
            let label = format!("Ollama: {}", spec.label);
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
    app.ollama.events = Some(transport::models(&app.ollama, ctx.clone()));
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
    app.ollama.events = Some(transport::chat(&app.ollama, ctx.clone()));
}

pub fn show(app: &mut WordApp, ui: &mut Ui) {
    if !app.ollama.open {
        return;
    }
    let t = Tokens::get(ui.ctx());
    egui::Panel::right("ollama_chat").default_size(360.0).size_range(300.0..=600.0).resizable(true)
        .frame(egui::Frame::NONE.fill(t.panel).inner_margin(12).stroke(Stroke::new(1.0,t.border))).show(ui, |ui| {
        if crate::panes::header(ui, "Document assistant") { app.ollama.open = false; }
        if cfg!(target_arch = "wasm32") { ui.label("Ollama integration is available in the desktop app."); return; }
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            let old = app.ollama.source;
            ui.add_enabled_ui(!app.ollama.busy, |ui| {
                ui.selectable_value(&mut app.ollama.source, Source::Local, "Local Ollama");
                ui.selectable_value(&mut app.ollama.source, Source::Cloud, "Ollama Cloud");
                ui.selectable_value(&mut app.ollama.source, Source::LlamaCpp, "llama.cpp");
            });
            if old != app.ollama.source {
                app.ollama.models.clear(); app.ollama.model.clear(); app.ollama.history.clear();
                if !app.ollama.entries.is_empty() {app.ollama.entry("Activity","Backend changed. Started a new chat context.".into());}
                #[cfg(not(target_arch="wasm32"))]
                refresh(app,&ctx);
            }
        });
        ui.add_enabled_ui(!app.ollama.busy, |ui| {
            let previous_model = app.ollama.model.clone();
            egui::ComboBox::from_id_salt("ollama_model").truncate().width(ui.available_width()-12.0)
                .selected_text(if app.ollama.model.is_empty() { "Choose model" } else { &app.ollama.model }).show_ui(ui, |ui| {
                for model in &app.ollama.models { ui.selectable_value(&mut app.ollama.model, model.clone(), model); }
            });
            if previous_model != app.ollama.model { app.ollama.history.clear(); app.ollama.entry("Activity", "Model changed. Started a new chat context.".into()); }
            ui.horizontal(|ui| {
                #[cfg(not(target_arch = "wasm32"))]
                if ui.button("Refresh models").clicked() { refresh(app, &ctx); }
                if ui.button("New chat").clicked() { app.ollama.history.clear(); app.ollama.entries.clear(); app.ollama.sources.clear(); }
            });
            ui.collapsing("Connection settings", |ui| {
                if app.ollama.source == Source::Local {
                    ui.label("Ollama server");
                    if ui.add(egui::TextEdit::singleline(&mut app.ollama.endpoint).desired_width(f32::INFINITY)).changed() {
                        app.ollama.models.clear(); app.ollama.model.clear(); app.ollama.history.clear();
                    }
                    ui.label("Start Ollama, then refresh models. Signed-in Ollama can also run cloud models.");
                }
                if app.ollama.source==Source::LlamaCpp {
                    ui.label("llama.cpp server");
                    ui.add(egui::TextEdit::singleline(&mut app.ollama.llama_endpoint).desired_width(f32::INFINITY));
                    ui.small("Start llama-server on this computer, then refresh models. Existing GGUF weights can be reused.");
                }
                ui.label("Ollama API key (optional)");
                if ui.add(egui::TextEdit::singleline(&mut app.ollama.api_key).password(true).desired_width(f32::INFINITY)).changed() {
                    app.ollama.models.clear(); app.ollama.model.clear(); app.ollama.history.clear();
                }
                ui.small("The key stays in memory for this app session. Cloud uses signed-in Ollama when no key is entered. Web search works with or without a key.");
                ui.hyperlink_to("Get an API key", "https://ollama.com/settings/keys");
                if app.ollama.source == Source::Cloud {
                    ui.small("Cloud chat includes a snapshot of this document. Your Ollama account usage limits apply.");
                }
            });
        });
        ui.separator();
        ui.horizontal(|ui| {
            if ui.checkbox(&mut app.ollama.allow_edits, "Allow AI editing").changed() {
                app.ollama.status = if app.ollama.allow_edits { "AI editing enabled.".into() } else { "AI editing disabled.".into() };
            }
            if ui.add_enabled(can_undo(app), egui::Button::new("Undo AI change")).clicked()
                && let Err(e) = undo(app) { app.ollama.status = e; }
        });
        if !app.ollama.allow_edits { ui.colored_label(t.green, "Read only · document editing is blocked"); }
        let internet_changed = ui.checkbox(&mut app.ollama.internet, "Web search and browsing").on_hover_text("Controls web search and page fetching. Cloud models still use their cloud connection.").changed();
        // Revoking internet cancels an in-flight request as well as hiding tools next turn.
        // HTTP already sent may finish, but no subsequent document tool can be applied.
        if internet_changed && app.ollama.busy && !app.ollama.internet { app.ollama.stop(); }
        ui.separator();
        let footer = 158.0;
        egui::ScrollArea::vertical().id_salt("ollama_messages").stick_to_bottom(true).max_height((ui.available_height()-footer).max(60.0)).show(ui, |ui| {
            if app.ollama.entries.is_empty() {
                ui.label(RichText::new("Work on your document").strong());
                ui.label("Ask for a draft, formatting, a table, a summary, or web research. Enable editing when you want changes applied live.");
            }
            for (role,text) in &app.ollama.entries {
                ui.add_space(8.0);
                ui.label(RichText::new(role).strong());
                ui.add(egui::Label::new(text).wrap().selectable(true));
                if role == "Assistant" {
                    let mut urls = Vec::new();
                    for word in text.split_whitespace() {
                        if let Some(at)=word.find("https://") {
                            let url=word.get(at..).unwrap_or("").trim_end_matches([')',']','.',',',';','"']);
                            if !urls.contains(&url) && urls.len()<8 { urls.push(url); }
                        }
                    }
                    for url in urls { ui.hyperlink(url); }
                }
            }
            if !app.ollama.live_response.is_empty() { ui.label(&app.ollama.live_response); }
            if !app.ollama.sources.is_empty() {
                ui.separator(); ui.label(RichText::new("Retrieved sources").strong());
                for (title,url) in &app.ollama.sources {ui.hyperlink_to(title,url);}
            }
        });
        ui.separator();
        ui.label(&app.ollama.status);
        let edits=app.session.undo_history().iter().filter(|label|label.starts_with("Ollama: ")).count();
        ui.small(format!("AI changes in Undo history: {edits}"));
        ui.add(egui::TextEdit::multiline(&mut app.ollama.input).hint_text("Ask about or edit this document…").desired_rows(3).desired_width(f32::INFINITY).char_limit(MAX_INPUT));
        ui.horizontal(|ui| {
            #[cfg(not(target_arch = "wasm32"))]
            if ui.add_enabled(!app.ollama.busy && !app.ollama.model.is_empty() && !app.ollama.input.trim().is_empty(), egui::Button::new("Send")).clicked() { send(app, &ctx); }
            if app.ollama.busy { ui.spinner(); if ui.button("Stop").clicked() { app.ollama.stop(); } }
        });
    });
}

/// Session diagnostics intentionally exclude API keys.
pub fn inspect(app: &WordApp) -> Value {
    json!({"open":app.ollama.open,"model":app.ollama.model,"source":match app.ollama.source {Source::Cloud=>"cloud",Source::Local=>"local",Source::LlamaCpp=>"llamacpp"},
        "busy":app.ollama.busy,"status":app.ollama.status,"allowEdits":app.ollama.allow_edits,"internet":app.ollama.internet,
        "canUndo":can_undo(app),"models":app.ollama.models,"messages":app.ollama.entries,"sources":app.ollama.sources})
}

pub fn commands(app: &WordApp) -> Vec<Value> {
    [
        ("ui.codexConnect", "Codex connection", r#"{"value"?:bool}"#),
        ("ui.ollama", "Ollama Chat", r#"{"value"?:bool}"#),
        (
            "ui.ollamaSettings",
            "Ollama connection settings",
            r#"{"source"?:"local|cloud|llamacpp","endpoint"?:string,"llamaEndpoint"?:string,"model"?:string,"apiKey"?:string}"#,
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
                    _ => return Err("source must be local, cloud or llamacpp".into()),
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

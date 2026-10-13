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
mod history;
mod markdown;
mod settings;

const MAX_INPUT: usize = 16_000;
const MAX_MESSAGES: usize = 96;
const MAX_AGENT_IMAGES: usize = 4;

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
    pub save_history: bool,
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
            save_history: true,
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
    save_history: bool,
    cloud_consent: Option<Source>,
    project_files: Vec<String>,
    settings: settings::State,
    pub phase: String,
    pub thinking: String,
    pub review: edits::Review,
    markdown: egui_commonmark::CommonMarkCache,
    show_changes: bool,
    library: history::Library,
    library_epoch: u64,
    active_chat: Option<u64>,
    history_open: bool,
    history_filter: String,
    history_dirty: bool,
    history_error: String,
    delete_chat: Option<u64>,
    delete_all: bool,
    rename_chat: Option<(u64, String)>,
    edit_message: Option<(usize, String)>,
    branch_from: Option<u64>,
    /// Paths explicitly attached by the user to the current agent conversation.
    images: Vec<std::path::PathBuf>,
    /// One-frame capability set only by the visible attachment button; prevents remote commands
    /// from opening a file picker or attaching arbitrary paths.
    image_picker_intent: bool,
    #[cfg(not(target_arch = "wasm32"))]
    history_path: Option<std::path::PathBuf>,
    #[cfg(not(target_arch = "wasm32"))]
    history_store: Option<history::Store>,
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
            save_history: true,
            cloud_consent: None,
            project_files: Vec::new(),
            settings: settings::State::default(),
            phase: String::new(),
            thinking: String::new(),
            review: edits::Review::default(),
            markdown: egui_commonmark::CommonMarkCache::default(),
            show_changes: false,
            library: Default::default(),
            library_epoch: 0,
            active_chat: None,
            history_open: false,
            history_filter: String::new(),
            history_dirty: false,
            history_error: String::new(),
            delete_chat: None,
            delete_all: false,
            rename_chat: None,
            edit_message: None,
            branch_from: None,
            images: Vec::new(),
            image_picker_intent: false,
            #[cfg(not(target_arch = "wasm32"))]
            history_path: None,
            #[cfg(not(target_arch = "wasm32"))]
            history_store: None,
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
            save_history: self.save_history,
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
        self.save_history = p.save_history;
        self.project_files.clear();
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.settings.scan = None;
            self.settings.scanned_folder.clear();
        }
        self.cloud_consent = None;
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
        self.keep_interrupted_response();
        self.status = "Stopped. Completed changes remain available in Undo.".into();
    }
    fn keep_interrupted_response(&mut self) {
        let partial = std::mem::take(&mut self.live_response);
        if !partial.is_empty() {
            self.entry("Assistant (interrupted)", partial);
        }
        self.thinking.clear();
        self.phase.clear();
    }
    fn entry(&mut self, role: &str, text: String) {
        if self.entries.len() >= MAX_MESSAGES {
            self.entries.remove(0);
            self.markdown = Default::default();
        }
        self.entries.push((role.into(), clip(&text, 24_000)));
        self.history_dirty = true;
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
    #[cfg(not(target_arch = "wasm32"))]
    {
        if !app.ollama.project_folder.is_empty()
            && app.ollama.settings.scanned_folder != app.ollama.project_folder
            && let Err(error) = settings::scan_project(app, true)
        {
            app.ollama.status = error;
        }
        settings::poll_scan(app, ctx);
    }
    history::sync(app);
    if app.ollama.document.is_some_and(|d| d != app.session.document_id()) {
        app.ollama.stop();
        app.ollama.document = Some(app.session.document_id());
        app.ollama.entries.clear();
        app.ollama.active_chat = None;
        app.ollama.branch_from = None;
        app.ollama.edit_message = None;
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
                let interrupted = app.ollama.busy;
                app.ollama.busy = false;
                if interrupted {
                    app.ollama.keep_interrupted_response();
                    app.ollama.history.clear();
                    app.ollama.status = "Connection closed before a reply.".into();
                    app.ollama.entry("Error", app.ollama.status.clone());
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
                match result {
                    Ok(history) => {
                        app.ollama.live_response.clear();
                        app.ollama.phase.clear();
                        app.ollama.history = history;
                        app.ollama.status = "Ready.".into();
                    }
                    Err(error) => {
                        app.ollama.keep_interrupted_response();
                        app.ollama.status = error.clone();
                        app.ollama.entry("Error", error);
                        app.ollama.history.clear();
                    }
                }
            }
        }
    }
    if app.ollama.history_dirty {
        history::sync(app);
        ctx.request_repaint();
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
fn revoke_project_context(app: &mut WordApp) {
    let title = document_title(app);
    history::capture(&mut app.ollama, &title);
    app.ollama.stop();
    app.ollama.entries.clear();
    app.ollama.history.clear();
    app.ollama.context = Value::Null;
    app.ollama.active_chat = None;
    app.ollama.branch_from = None;
    app.ollama.edit_message = None;
    app.ollama.sources.clear();
    app.ollama.project_excerpts.clear();
    app.ollama.selected_source = None;
    app.ollama.thinking.clear();
    app.ollama.status = "File access revoked. Started a fresh chat context; saved history is unchanged.".into();
}

fn uses_cloud(state: &State) -> bool {
    matches!(state.source, Source::Cloud | Source::Codex) || state.model.to_ascii_lowercase().contains("cloud")
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn send(app: &mut WordApp, ctx: &egui::Context) {
    let input = app.ollama.input.trim().to_string();
    if app.ollama.settings.scan.is_some() {
        app.ollama.status = "Project files are still indexing. Please send again when indexing finishes.".into();
        return;
    }
    if app.ollama.edit_message.is_some() {
        app.ollama.status = "Save or cancel the message edit first.".into();
        return;
    }
    if (input.is_empty() && app.ollama.images.is_empty()) || app.ollama.busy {
        return;
    }
    if !app.ollama.images.is_empty() && app.ollama.source != Source::Codex {
        app.ollama.status = "Image attachments currently work with Codex. Choose Codex or remove the images.".into();
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
    if uses_cloud(&app.ollama) && app.ollama.cloud_consent != Some(app.ollama.source) {
        app.ollama.open = true;
        app.ollama.status = "Allow cloud sharing in chat to send this message. Your draft has been kept.".into();
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
        "selectedText":clip(&app.session.selected_text(),2000),"selection":app.session.sel,"allowEdits":app.ollama.allow_edits,"linkedProject":if app.ollama.project_folder.is_empty() { Value::Null } else { json!({"name":std::path::Path::new(&app.ollama.project_folder).file_name().map(|n| n.to_string_lossy().into_owned()),"approvedFiles":app.ollama.project_files.len(),"tools":["project_list","project_search","project_read"]}) }});
    app.ollama.cancel = Arc::new(AtomicBool::new(false));
    let visible_input = if input.is_empty() { "Describe the attached image(s).".to_string() } else { input };
    app.ollama.history.push(json!({"role":"user","content":visible_input}));
    app.ollama.entry("You", visible_input);
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

/// Consent is granted only by a visible click, never by an agent/control command.
fn cloud_sharing_prompt(app: &mut WordApp, ui: &mut Ui, t: &Tokens) {
    if !uses_cloud(&app.ollama) || app.ollama.cloud_consent == Some(app.ollama.source) {
        return;
    }
    egui::Frame::NONE.fill(t.chat_surface).corner_radius(12).inner_margin(12).show(ui, |ui| {
        ui.label(RichText::new("Cloud sharing is off").font(crate::theme::medium(13.0)));
        let recipient = if app.ollama.source == Source::Codex { "OpenAI" } else { "your model provider" };
        ui.add(egui::Label::new(format!("Sending shares messages, this document, approved project passages and attached images with {recipient}. Allow for this session and provider?" )).wrap());
        #[cfg(not(target_arch = "wasm32"))]
        if ui.add_enabled(!app.ollama.busy, egui::Button::new("Allow cloud sharing and send")).clicked() {
            app.ollama.cloud_consent = Some(app.ollama.source);
            let ctx = ui.ctx().clone();
            send(app, &ctx);
        }
    });
    ui.add_space(8.0);
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

fn message_action(ui: &mut Ui, icon: &str, label: &str, t: &Tokens) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label));
    if response.hovered() {
        ui.painter().rect_filled(rect, 6.0, t.chat_user);
    }
    crate::icons::paint(ui.painter(), egui::Rect::from_center_size(rect.center(), egui::vec2(15.0, 15.0)), icon, t.chat_dim, t.chat_dim);
    response.on_hover_text(label)
}

fn message(ui: &mut Ui, role: &str, text: &str, t: &Tokens, cache: &mut egui_commonmark::CommonMarkCache) {
    ui.add_space(20.0);
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
            egui::Frame::NONE.fill(t.chat_user).corner_radius(18).inner_margin(egui::Margin::symmetric(14, 10)).show(ui, |ui| {
                ui.set_max_width((ui.available_width() - 28.0).max(60.0));
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.add(egui::Label::new(text).wrap().selectable(true));
                });
            });
        });
    } else {
        if role == "Assistant (interrupted)" {
            ui.small(RichText::new("Response interrupted · partial answer").color(t.chat_dim));
        }
        markdown::show(ui, cache, text);
    }
}

fn connection_menu(app: &mut WordApp, ui: &mut Ui, ctx: &egui::Context) {
    ui.label(RichText::new("Provider").strong());
    let old = app.ollama.source;
    for source in [Source::Codex, Source::Local, Source::Cloud, Source::LlamaCpp] {
        ui.selectable_value(&mut app.ollama.source, source, source_name(source));
    }
    if old != app.ollama.source {
        app.ollama.cloud_consent = None;
        app.ollama.models.clear();
        app.ollama.model.clear();
        app.ollama.capabilities.clear();
        app.ollama.effort.clear();
        app.ollama.history.clear();
        app.ollama.entries.clear();
        app.ollama.active_chat = None;
        app.ollama.branch_from = None;
        app.ollama.edit_message = None;
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
                app.ollama.cloud_consent = None;
                app.ollama.history.clear();
                app.ollama.models.clear();
            }
        }
        if app.ollama.source == Source::LlamaCpp {
            ui.label("llama.cpp address");
            if ui.text_edit_singleline(&mut app.ollama.llama_endpoint).changed() {
                app.ollama.cloud_consent = None;
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
            .add(egui::Button::new(RichText::new(format!("Review {} changes  ▾", app.ollama.review.marks.len())).color(t.diff_add)).frame(false))
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
        ui.small("Additions · removals · formatting");
        if app.ollama.review.revision != app.session.rev() {
            ui.small("Highlights cleared after subsequent manual edits. Changes below are the AI's earlier edits.");
        }
        let mut show = None;
        for (index, mark) in app.ollama.review.marks.iter().enumerate() {
            egui::Frame::NONE.fill(t.chat_surface).corner_radius(12).inner_margin(12).show(ui, |ui| {
                ui.set_width((ui.available_width() - 2.0).max(20.0));
                if ui.add_enabled(app.ollama.review.revision == app.session.rev(), egui::Button::new("Show in document").frame(false)).clicked() {
                    show = Some(index);
                }
                if mark.formatting {
                    ui.small("Formatting / layout changed");
                }
                if !mark.removed.is_empty() {
                    ui.add(
                        egui::Label::new(RichText::new(format!("− {}", mark.removed)).color(t.diff_remove).strikethrough()).wrap().selectable(true),
                    );
                }
                if !mark.added.is_empty() {
                    ui.add(egui::Label::new(RichText::new(format!("+ {}", mark.added)).color(t.diff_add)).wrap().selectable(true));
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

fn chat_icon(ui: &mut Ui, name: &str, tip: &str, t: &Tokens) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 8.0, t.chat_surface);
    }
    crate::icons::paint(ui.painter(), rect.shrink(5.0), name, t.chat_dim, t.chat_dim);
    response.on_hover_text(tip)
}
fn model_menu(app: &mut WordApp, ui: &mut Ui, t: &Tokens) {
    let model = app.ollama.capabilities.iter().find(|o| o.model == app.ollama.model).map(|o| o.display_name.as_str()).unwrap_or(&app.ollama.model);
    let friendly = if model.to_ascii_lowercase().starts_with("gpt-") {
        if let Some((prefix, suffix)) = model.rsplit_once('-').filter(|(p, _)| p.contains('-')) {
            let mut chars = suffix.chars();
            let suffix = chars.next().map(|c| format!("{}{}", c.to_uppercase(), chars.as_str())).unwrap_or_default();
            format!("{} {suffix}", prefix.replacen("gpt-", "GPT-", 1))
        } else {
            model.to_string()
        }
    } else {
        model.to_string()
    };
    let model = friendly.as_str();
    let label = if model.is_empty() {
        "Select model".into()
    } else if app.ollama.source == Source::Codex {
        format!("{} · {}", clip(model, 22), effort_label(&app.ollama.effort))
    } else {
        clip(model, 26)
    };
    ui.menu_button(RichText::new(label).size(11.5).color(t.chat_text), |ui| {
        ui.small(source_name(app.ollama.source));
        let old = app.ollama.model.clone();
        egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
            for model in &app.ollama.models {
                let label = app.ollama.capabilities.iter().find(|o| o.model == *model).map(|o| o.display_name.as_str()).unwrap_or(model);
                if ui.selectable_value(&mut app.ollama.model, model.clone(), label).clicked() {
                    ui.close();
                }
            }
        });
        if old != app.ollama.model {
            app.ollama.effort.clear();
            app.ollama.history.clear();
        }
        if app.ollama.source == Source::Codex {
            ui.separator();
            ui.small("Thinking");
            ui.selectable_value(&mut app.ollama.effort, String::new(), "Auto · model default");
            if let Some(options) = app.ollama.capabilities.iter().find(|o| o.model == app.ollama.model) {
                for (level, description) in &options.efforts {
                    ui.selectable_value(&mut app.ollama.effort, level.clone(), effort_label(level)).on_hover_text(description);
                }
            }
        }
        ui.separator();
        ui.menu_button("Connection settings…", |ui| {
            let ctx = ui.ctx().clone();
            connection_menu(app, ui, &ctx);
        });
    });
}
fn confirm_history_deletion(app: &mut WordApp, ui: &mut Ui, t: &Tokens) {
    if app.ollama.delete_chat.is_none() && !app.ollama.delete_all {
        return;
    }
    egui::Frame::NONE.fill(t.chat_surface).corner_radius(12).inner_margin(12).show(ui, |ui| {
        ui.add(egui::Label::new(if app.ollama.delete_all { "Delete all saved chats permanently?" } else { "Delete this chat permanently?" }).wrap());
        ui.small("Document edits and their Undo history are kept.");
        ui.horizontal(|ui| {
            if ui.add(egui::Button::new(RichText::new("Delete").color(t.diff_remove)).fill(t.chat_surface).corner_radius(8)).clicked() {
                if app.ollama.delete_all {
                    let _ = command(app, "ui.agentClearHistory", &json!({}));
                } else if let Some(id) = app.ollama.delete_chat {
                    let _ = command(app, "ui.agentDeleteChat", &json!({"id":id}));
                }
                app.ollama.delete_chat = None;
                app.ollama.delete_all = false;
            }
            if ui.button("Cancel").clicked() {
                app.ollama.delete_chat = None;
                app.ollama.delete_all = false;
            }
        });
    });
}
fn rename_chat_window(app: &mut WordApp, ctx: &egui::Context) {
    let Some((id, mut text)) = app.ollama.rename_chat.clone() else {
        return;
    };
    let mut open = true;
    let mut save = false;
    egui::Window::new("Rename chat").id(egui::Id::new("rename_agent_chat")).collapsible(false).resizable(false).open(&mut open).show(ctx, |ui| {
        let field = ui.add(egui::TextEdit::singleline(&mut text).char_limit(96).desired_width(280.0));
        save = ui.add_enabled(!text.trim().is_empty(), egui::Button::new("Save")).clicked()
            || (field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !text.trim().is_empty());
    });
    app.ollama.rename_chat = if open { Some((id, text.clone())) } else { None };
    if save {
        if let Err(error) = command(app, "ui.agentRenameChat", &json!({"id":id,"title":text})) {
            app.ollama.history_error = error;
        }
        app.ollama.rename_chat = None;
    }
}

fn history_view(app: &mut WordApp, ui: &mut Ui, t: &Tokens) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Chat history").font(crate::theme::semibold(18.0)));
        if ui.add(egui::Button::new("Done").frame(false)).clicked() {
            app.ollama.history_open = false;
        }
    });
    ui.add(egui::TextEdit::singleline(&mut app.ollama.history_filter).hint_text("Search chats").desired_width(ui.available_width()).char_limit(200));
    ui.add_space(8.0);
    if app.ollama.library.chats.is_empty() {
        ui.label(RichText::new("Your conversations will appear here.").color(t.chat_dim));
        return;
    }
    let filter = app.ollama.history_filter.to_lowercase();
    let mut open = None;
    for chat in &app.ollama.library.chats {
        if !filter.is_empty() && !format!("{} {}", chat.title, chat.document).to_lowercase().contains(&filter) {
            continue;
        }
        egui::Frame::NONE
            .fill(if app.ollama.active_chat == Some(chat.id) { t.chat_surface } else { t.chat })
            .corner_radius(12)
            .inner_margin(10)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        let width = (ui.available_width() - 38.0).max(40.0);
                        ui.set_min_width(width);
                        ui.set_max_width(width);
                        if ui.add(egui::Button::new(RichText::new(&chat.title).size(13.0)).truncate().frame(false)).clicked() {
                            open = Some(chat.id);
                        }
                        ui.add(
                            egui::Label::new(RichText::new(format!("{} · {}", chat.document, chat.model)).size(10.5).color(t.chat_dim)).truncate(),
                        );
                    });
                    ui.menu_button("•••", |ui| {
                        if ui.button("Rename…").clicked() {
                            app.ollama.rename_chat = Some((chat.id, chat.title.clone()));
                            ui.close();
                        }
                        if ui.button("Delete chat…").clicked() {
                            app.ollama.delete_chat = Some(chat.id);
                            ui.close();
                        }
                    });
                });
            });
    }
    ui.add_space(12.0);
    if ui.add(egui::Button::new(RichText::new("Delete all history…").size(11.0).color(t.diff_remove)).frame(false)).clicked() {
        app.ollama.delete_all = true;
    }
    if let Some(id) = open
        && let Err(error) = command(app, "ui.agentOpenChat", &json!({"id":id}))
    {
        app.ollama.history_error = error;
    }
}

fn project_indicator(app: &mut WordApp, ui: &mut Ui, t: &Tokens) {
    let linked = !app.ollama.project_folder.is_empty();
    let name = std::path::Path::new(&app.ollama.project_folder)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Project folder".into());
    egui::Frame::NONE.stroke(Stroke::new(0.6, t.border)).corner_radius(8).inner_margin(8).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
            crate::icons::paint(ui.painter(), rect, "folderOutline", t.chat_dim, t.chat_dim);
            let width = (ui.available_width() - 68.0).max(40.0);
            ui.allocate_ui_with_layout(egui::vec2(width, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.set_width(width);
                ui.add(
                    egui::Label::new(RichText::new(if linked { name.as_str() } else { "No project linked" }).font(crate::theme::medium(12.0)))
                        .truncate(),
                )
                .on_hover_text(if linked { app.ollama.project_folder.as_str() } else { "Link a folder to provide project context." });
            });
            if ui.add(egui::Button::new(if linked { "Manage" } else { "Link" }).frame(false)).clicked() {
                app.ollama.settings.open = true;
                app.ollama.settings.tab = settings::Tab::Project;
            }
        });
        if linked {
            let count = app.ollama.project_files.len();
            #[cfg(not(target_arch = "wasm32"))]
            let indexing = app.ollama.settings.scan.is_some();
            #[cfg(target_arch = "wasm32")]
            let indexing = false;
            let status = if indexing {
                "Indexing files…".into()
            } else if count == 0 {
                "No files selected".into()
            } else {
                format!("{count} {} available · read only", if count == 1 { "file" } else { "files" })
            };
            ui.horizontal(|ui| {
                ui.add_space(26.0);
                ui.add(egui::Label::new(RichText::new(status).size(10.5).color(t.chat_dim)).truncate());
            });
        }
    });
}

pub fn show(app: &mut WordApp, ui: &mut Ui) {
    if !app.ollama.open {
        return;
    }
    let t = Tokens::get(ui.ctx());
    settings::show(app, ui.ctx());
    egui::Panel::right("agent_chat_v2")
        .default_size(460.0)
        .size_range(340.0..=720.0)
        .resizable(true)
        .frame(egui::Frame::NONE.fill(t.chat).inner_margin(20).stroke(Stroke::new(1.0, t.border)))
        .show(ui, |ui| {
            let ctx = ui.ctx().clone();
            for (url, _) in &app.ollama.project_excerpts {
                if app.ollama.markdown.get_link_hook(url).is_none() {
                    app.ollama.markdown.add_link_hook(url);
                }
            }
            // This scope changes only the chat: quiet, borderless controls retain native focus/hover.
            let visuals = ui.visuals_mut();
            visuals.selection.bg_fill = t.chat_user;
            visuals.selection.stroke = Stroke::new(1.0, t.chat_text);
            visuals.widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
            visuals.widgets.inactive.bg_fill = t.chat_surface;
            visuals.widgets.hovered.weak_bg_fill = t.chat_user;
            for visual in [&mut visuals.widgets.inactive, &mut visuals.widgets.hovered] {
                visual.bg_stroke = Stroke::NONE;
                visual.corner_radius = egui::CornerRadius::same(8);
            }
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            ui.style_mut().text_styles.insert(egui::TextStyle::Body, crate::theme::regular(14.0));
            ui.style_mut().text_styles.insert(egui::TextStyle::Button, crate::theme::regular(12.0));
            ui.style_mut().text_styles.insert(egui::TextStyle::Small, crate::theme::regular(11.0));
            ui.visuals_mut().override_text_color = Some(t.chat_text);
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(format!("{} ▾", source_name(app.ollama.source))).frame(false)).clicked() {
                    app.ollama.settings.open = true;
                    app.ollama.settings.tab = settings::Tab::General;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Button::new("×").frame(false)).on_hover_text("Close chat").clicked() {
                        app.ollama.open = false;
                    }
                    if ui
                        .add(egui::Button::new(if app.ollama.save_history { "◌" } else { "●" }).frame(false))
                        .on_hover_text("Temporary chats are not saved locally")
                        .clicked()
                    {
                        app.ollama.save_history = !app.ollama.save_history;
                    }
                    if chat_icon(ui, "history", "Chat history", &t).clicked() {
                        let _ = command(app, "ui.agentHistory", &json!({}));
                    }
                    ui.menu_button("•••", |ui| {
                        if ui.button("Settings").clicked() {
                            app.ollama.settings.open = true;
                            ui.close();
                        }
                        if ui.button("Chat history").clicked() {
                            app.ollama.history_open = true;
                            ui.close();
                        }
                        ui.separator();
                        if ui.add_enabled(!app.ollama.busy && app.ollama.active_chat.is_some(), egui::Button::new("Delete this chat…")).clicked() {
                            app.ollama.delete_chat = app.ollama.active_chat;
                            ui.close();
                        }
                        if ui.add_enabled(!app.ollama.busy, egui::Button::new("Delete all history…")).clicked() {
                            app.ollama.delete_all = true;
                            ui.close();
                        }
                    });
                    ui.add_enabled_ui(!app.ollama.busy, |ui| {
                        if chat_icon(ui, "newChat", "New chat", &t).clicked() {
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
                let active = app.ollama.allow_edits;
                if ui
                    .add(
                        egui::Button::new(RichText::new(if active { "● Editing enabled" } else { "Read only" }).size(11.0).color(if active {
                            t.diff_add
                        } else {
                            t.chat_dim
                        }))
                        .frame(false),
                    )
                    .on_hover_text("Hard permission switch. Click to enable or disable document changes.")
                    .clicked()
                {
                    let _ = app.run("ui.ollamaEdit", json!({"value":!active}));
                }
                ui.label(
                    RichText::new(format!("{}{}", source_name(app.ollama.source), if app.ollama.internet { " · Web on" } else { "" }))
                        .size(11.0)
                        .color(t.chat_dim),
                );
            });
            project_indicator(app, ui, &t);
            cloud_sharing_prompt(app, ui, &t);
            ui.add_space(4.0);
            ui.separator();
            let extra = if app.ollama.models.is_empty() && !app.ollama.busy { 30.0 } else { 0.0 }
                + if app.ollama.status != "Ready." && !app.ollama.busy { 28.0 } else { 0.0 };
            confirm_history_deletion(app, ui, &t);
            rename_chat_window(app, &ctx);
            if !app.ollama.history_error.is_empty() {
                ui.add(egui::Label::new(RichText::new(&app.ollama.history_error).size(11.0).color(t.red)).wrap());
            }
            if let Some(parent) = app.ollama.branch_from {
                ui.horizontal(|ui| {
                    ui.small(RichText::new("Branched conversation").color(t.chat_dim));
                    if app.ollama.library.chats.iter().any(|c| c.id == parent)
                        && ui.small_button("Open original").clicked()
                        && let Err(error) = command(app, "ui.agentOpenChat", &json!({"id":parent}))
                    {
                        app.ollama.status = error;
                    }
                });
            }
            let welcome = app.ollama.entries.is_empty() && !app.ollama.history_open;
            let height = if welcome {
                (ui.available_height() * 0.40).clamp(40.0, 280.0).min((ui.available_height() - 190.0 - extra).max(40.0))
            } else {
                (ui.available_height() - 166.0 - extra).max(40.0)
            };
            ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), height), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_min_height(height);
                egui::ScrollArea::vertical()
                    .id_salt("agent_conversation_v2")
                    .auto_shrink([false, false])
                    .stick_to_bottom(app.ollama.edit_message.is_none())
                    .max_height(height)
                    .min_scrolled_height(height)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                        if app.ollama.history_open {
                            history_view(app, ui, &t);
                            return;
                        }
                        if app.ollama.entries.is_empty() {
                            ui.add_space((height - 82.0).max(24.0));
                            ui.vertical_centered(|ui| {
                                ui.add(
                                    egui::Label::new(RichText::new("Where should we begin?").font(crate::theme::regular(25.0)).color(t.chat_text))
                                        .wrap(),
                                );
                                ui.add_space(12.0);
                                ui.add(egui::Label::new(RichText::new("Write, revise or explore an idea.").size(13.0).color(t.chat_dim)).wrap());
                            });
                        }
                        let mut action = None;
                        for (index, (role, text)) in app.ollama.entries.iter().enumerate() {
                            if app.ollama.edit_message.as_ref().is_some_and(|(editing, _)| index > *editing) {
                                break;
                            }
                            if app.ollama.edit_message.as_ref().is_some_and(|(i, _)| *i == index) {
                                if let Some((_, draft)) = &mut app.ollama.edit_message {
                                    egui::Frame::NONE.fill(t.chat_surface).corner_radius(18).inner_margin(14).show(ui, |ui| {
                                        ui.add(
                                            egui::TextEdit::multiline(draft)
                                                .id_salt(("agent_edit_message", index))
                                                .text_color(t.chat_text)
                                                .font(crate::theme::regular(14.0))
                                                .desired_width(ui.available_width())
                                                .desired_rows(4)
                                                .char_limit(MAX_INPUT)
                                                .frame(egui::Frame::NONE),
                                        );
                                        ui.small("Editing creates a new branch. The original chat is kept.");
                                        ui.horizontal(|ui| {
                                            if ui.button("Cancel").clicked() {
                                                action = Some(("cancel", json!({})));
                                            }
                                            if ui
                                                .add_enabled(!draft.trim().is_empty() && !app.ollama.busy, egui::Button::new("Save & send"))
                                                .clicked()
                                            {
                                                action = Some(("ui.agentEditMessage", json!({"index":index,"text":draft,"send":true})));
                                            }
                                        });
                                    });
                                }
                            } else {
                                message(ui, role, text, &t, &mut app.ollama.markdown);
                                if matches!(role.as_str(), "You" | "Assistant") {
                                    ui.push_id(("message_actions", index), |ui| {
                                        ui.horizontal(|ui| {
                                            if message_action(ui, "copy", "Copy message", &t).clicked() {
                                                ui.ctx().copy_text(text.clone());
                                            }
                                            if role == "You"
                                                && ui
                                                    .add_enabled_ui(!app.ollama.busy, |ui| message_action(ui, "pencil", "Edit message", &t))
                                                    .inner
                                                    .clicked()
                                            {
                                                action = Some(("edit", json!({"index":index,"text":text})));
                                            }
                                            ui.menu_button(RichText::new("•••").color(t.chat_dim), |ui| {
                                                if ui.add_enabled(!app.ollama.busy, egui::Button::new("Branch chat from here")).clicked() {
                                                    action = Some(("ui.agentBranchChat", json!({"index":index})));
                                                    ui.close();
                                                }
                                            });
                                        });
                                    });
                                }
                            }
                            for (url, _) in &app.ollama.project_excerpts {
                                if app.ollama.markdown.get_link_hook(url) == Some(true) {
                                    app.ollama.selected_source = Some(url.clone());
                                }
                            }
                        }
                        if let Some((id, params)) = action {
                            match id {
                                "cancel" => app.ollama.edit_message = None,
                                "edit" => {
                                    app.ollama.edit_message = params
                                        .get("index")
                                        .and_then(Value::as_u64)
                                        .and_then(|v| usize::try_from(v).ok())
                                        .map(|i| (i, params["text"].as_str().unwrap_or_default().to_string()))
                                }
                                _ => {
                                    if let Err(error) = command(app, id, &params) {
                                        app.ollama.status = error;
                                    }
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
            egui::Frame::NONE.fill(t.chat_surface).stroke(Stroke::new(0.6, t.border)).corner_radius(24).inner_margin(14).show(ui, |ui| {
                if !app.ollama.images.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.small(RichText::new("Images · included with Codex turns").color(t.chat_dim));
                        let mut remove = None;
                        for (index, path) in app.ollama.images.iter().enumerate() {
                            let name = path.file_name().map(|name| name.to_string_lossy()).unwrap_or_else(|| "Image".into());
                            if ui.small_button(format!("{name} ×")).on_hover_text("Remove image from this chat").clicked() {
                                remove = Some(index);
                            }
                        }
                        if let Some(index) = remove {
                            app.ollama.images.remove(index);
                            app.ollama.history_dirty = true;
                            app.ollama.status = "Image removed from this chat.".into();
                        }
                    });
                    ui.add_space(4.0);
                }
                let response = ui.add(
                    egui::TextEdit::multiline(&mut app.ollama.input)
                        .id(egui::Id::new("agent_composer"))
                        .return_key(egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::Enter))
                        .font(crate::theme::regular(14.0))
                        .frame(egui::Frame::NONE)
                        .hint_text("Ask anything about your document")
                        .desired_rows(if welcome { 1 } else { 2 })
                        .desired_width(f32::INFINITY)
                        .char_limit(MAX_INPUT),
                );
                let submit =
                    response.has_focus() && app.ollama.edit_message.is_none() && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.menu_button(RichText::new("+").size(23.0), |ui| {
                        let can_add_image = app.ollama.source == Source::Codex && app.ollama.images.len() < MAX_AGENT_IMAGES;
                        if ui
                            .add_enabled(can_add_image, egui::Button::new("Attach image…"))
                            .on_hover_text("Attach up to four PNG, JPEG, GIF, WebP or BMP images (20 MiB each) to Codex chat.")
                            .clicked()
                        {
                            app.ollama.image_picker_intent = true;
                            if let Err(error) = command(app, "ui.agentPickImage", &json!({})) {
                                app.ollama.status = error;
                            }
                            ui.close();
                        }
                        if ui.button("Project files & context…").clicked() {
                            app.ollama.settings.open = true;
                            app.ollama.settings.tab = settings::Tab::Project;
                            ui.close();
                        }
                        if ui.checkbox(&mut app.ollama.internet, "Search the web").changed() && !app.ollama.internet && app.ollama.busy {
                            app.ollama.stop();
                        }
                        ui.checkbox(&mut app.ollama.allow_edits, "Allow document editing");
                        if ui.button("Chat history").clicked() {
                            app.ollama.history_open = true;
                            ui.close();
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let send_button = ui
                            .add_enabled(
                                app.ollama.busy
                                    || (app.ollama.edit_message.is_none()
                                        && !app.ollama.model.is_empty()
                                        && (!app.ollama.input.trim().is_empty() || !app.ollama.images.is_empty())),
                                egui::Button::new(RichText::new(if app.ollama.busy { "■" } else { "↑" }).size(20.0).color(t.chat_send_text))
                                    .fill(t.chat_send)
                                    .corner_radius(255)
                                    .min_size(egui::vec2(32.0, 32.0)),
                            )
                            .on_hover_text(if app.ollama.busy { "Stop response" } else { "Send · Enter (Shift Enter for a new line)" });
                        if send_button.clicked() && app.ollama.busy {
                            app.ollama.stop();
                        } else if (send_button.clicked() || submit)
                            && (!app.ollama.input.trim().is_empty() || !app.ollama.images.is_empty())
                            && !app.ollama.busy
                        {
                            #[cfg(not(target_arch = "wasm32"))]
                            send(app, &ctx);
                        }
                        ui.add_enabled_ui(!app.ollama.busy, |ui| {
                            model_menu(app, ui, &t);
                        });
                    });
                });
            });
            ui.add_space(8.0);
            if welcome {
                ui.horizontal_wrapped(|ui| {
                    for (label, prompt) in [
                        ("Summarize", "Summarize the current document."),
                        ("Improve writing", "Suggest improvements to the selected text, or the document if nothing is selected."),
                        ("Research", "Help me research a topic for this document. Ask which topic first."),
                    ] {
                        if ui
                            .add(
                                egui::Button::new(RichText::new(label).size(11.5))
                                    .fill(t.chat_surface)
                                    .corner_radius(16)
                                    .min_size(egui::vec2(0.0, 28.0)),
                            )
                            .clicked()
                        {
                            app.ollama.input = prompt.into();
                        }
                    }
                });
                ui.add_space(10.0);
            }
            ui.horizontal(|ui| {
                #[cfg(target_arch = "wasm32")]
                let stored = false;
                #[cfg(not(target_arch = "wasm32"))]
                let stored = app.ollama.history_store.is_some();
                ui.label(
                    RichText::new(if !app.ollama.save_history {
                        "Temporary chat · not saved locally"
                    } else if stored {
                        "Chats saved on this device"
                    } else {
                        "Chats kept for this session"
                    })
                    .size(10.5)
                    .color(t.chat_dim),
                );
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
        "account":app.ollama.account,"signedIn":app.ollama.signed_in,"usage":app.ollama.usage,"effort":app.ollama.effort,"capabilities":app.ollama.capabilities,"history":history::inspect(&app.ollama),"phase":app.ollama.phase,"thinking":app.ollama.thinking,"projectFolder":app.ollama.project_folder,"approvedProjectFiles":app.ollama.project_files,"saveHistory":app.ollama.save_history,"cloudContextAllowed":app.ollama.cloud_consent == Some(app.ollama.source),"changes":app.ollama.review.inspect(app.session.rev()),"canUndo":can_undo(app),"models":app.ollama.models,"messages":app.ollama.entries,"sources":app.ollama.sources})
}

/// Desktop windows share one history writer and a sequentially versioned library. Active
/// conversations, credentials and editing permissions remain local to their window.
pub fn share_history(target: &mut WordApp, source: &WordApp) {
    if target.ollama.library_epoch == source.ollama.library_epoch {
        return;
    }
    target.ollama.library.clone_from(&source.ollama.library);
    target.ollama.library_epoch = source.ollama.library_epoch;
    if target.ollama.active_chat.is_some_and(|id| !target.ollama.library.chats.iter().any(|chat| chat.id == id)) {
        target.ollama.stop();
        target.ollama.active_chat = None;
        target.ollama.entries.clear();
        target.ollama.images.clear();
        target.ollama.sources.clear();
        target.ollama.project_excerpts.clear();
    }
    #[cfg(not(target_arch = "wasm32"))]
    if let Ok(bytes) = target.ollama.library.bytes_for_windows()
        && let Some(store) = &mut target.ollama.history_store
    {
        store.submit(bytes);
    }
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
        ("ui.agentSettings", "Agent settings", r#"{"value"?:bool,"tab"?:"general|permissions|project|data"}"#),
        ("ui.agentHistory", "Chat history",r#"{"value"?:bool}"#),
        ("ui.agentOpenChat", "Open saved chat",r#"{"id":number}"#),
        ("ui.agentDeleteChat", "Delete saved chat",r#"{"id":number}"#),
        ("ui.agentClearHistory", "Delete all chat history","{}"),
        ("ui.agentRenameChat", "Rename saved chat",r#"{"id":number,"title":string}"#),
        ("ui.favoriteDocument", "Favourite document",r#"{"path":string,"value"?:bool}"#),
        ("ui.removeRecent", "Remove from Recent",r#"{"path":string}"#),
        ("ui.focusDocumentWindow", "Switch document window",r#"{"id":number}"#),
        ("ui.newDocumentWindow", "New document window",r#"{"template"?:string}"#),
        ("ui.closeDocumentWindow", "Close document window",r#"{"id":number}"#),
        ("ui.connectDocumentWindow", "Connect document window control",r#"{"id":number}"#),
        ("ui.agentBranchChat", "Branch chat here",r#"{"index":number}"#),
        ("ui.agentEditMessage", "Edit message in new branch",r#"{"index":number,"text":string,"send"?:bool}"#),
        ("ui.agentProject", "Link project folder", r#"{"path"?:string}"#),
        ("ui.agentPickImage", "Attach image to Codex chat", "{}"),
        ("ui.agentDismissChanges", "Dismiss AI highlighting", "{}"),
        ("ui.agentShowChange", "Show AI change in document", r#"{"index"?:number}"#),
    ]
    .into_iter()
    .map(|(id, label, params)| {
        let file = matches!(id, "ui.favoriteDocument" | "ui.removeRecent");
        let window = id.ends_with("DocumentWindow");
        json!({"id":id,"label":label,"location":if file { "File" } else if window { "View › Window" } else { "Agents" },"params":params,
        "enabled":(file || !cfg!(target_arch="wasm32")) && (!window || app.native_document_windows) && (id!="ui.ollamaUndo" || can_undo(app)) && id!="ui.agentPickImage"})
    })
    .collect()
}

pub(crate) fn command(app: &mut WordApp, id: &str, p: &Value) -> Result<Value, String> {
    history::sync(app);
    match id {
        "ui.agentSettings" => {
            let tab = match p.get("tab").and_then(Value::as_str).unwrap_or("general") {
                "general" => settings::Tab::General,
                "permissions" => settings::Tab::Permissions,
                "project" => settings::Tab::Project,
                "data" => settings::Tab::Data,
                _ => return Err("Unknown settings section.".into()),
            };
            app.ollama.settings.tab = tab;
            app.ollama.settings.open = p.get("value").and_then(Value::as_bool).unwrap_or(true);
            app.ollama.open = true;
        }
        "ui.agentBranchChat" | "ui.agentEditMessage" => {
            let index = p.get("index").and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()).ok_or("A valid message index is required")?;
            let text = if id == "ui.agentEditMessage" { Some(p.get("text").and_then(Value::as_str).ok_or("text required")?) } else { None };
            let resend = text.is_some() && p.get("send").and_then(Value::as_bool).unwrap_or(false);
            #[cfg(not(target_arch = "wasm32"))]
            let ctx = if resend { Some(app.ctx.clone().ok_or("The app is not ready yet")?) } else { None };
            if resend && app.ollama.model.is_empty() {
                return Err("Select a model first.".into());
            }
            let title = document_title(app);
            history::fork(&mut app.ollama, &title, index, text)?;
            app.ollama.open = true;
            app.ollama.edit_message = None;
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(ctx) = ctx {
                app.ollama.entries.pop();
                app.ollama.history.pop();
                app.ollama.input = text.unwrap_or_default().trim().to_string();
                send(app, &ctx);
            }
        }
        "ui.agentHistory" => app.ollama.history_open = p.get("value").and_then(Value::as_bool).unwrap_or(!app.ollama.history_open),
        #[cfg(not(target_arch = "wasm32"))]
        "ui.agentPickImage" => {
            if !std::mem::take(&mut app.ollama.image_picker_intent) {
                return Err("Choose Attach image from the agent chat to open the image picker.".into());
            }
            if app.ollama.source != Source::Codex {
                return Err("Image attachments currently work with Codex.".into());
            }
            let _ = app.ask_file(crate::FileDialogRequest::Open { purpose: "picture".into() }, crate::file_dialogs::AfterPick::AgentImage);
        }
        "ui.agentOpenChat" => {
            history::restore(&mut app.ollama, p.get("id").and_then(Value::as_u64).ok_or("id required")?)?;
            app.ollama.open = true;
        }
        "ui.agentDeleteChat" => {
            let id = p.get("id").and_then(Value::as_u64).ok_or("id required")?;
            if !app.ollama.library.chats.iter().any(|c| c.id == id) {
                return Err("Chat not found".into());
            }
            if app.ollama.active_chat == Some(id) {
                app.ollama.stop();
                app.ollama.entries.clear();
                app.ollama.images.clear();
                app.ollama.sources.clear();
                app.ollama.project_excerpts.clear();
                app.ollama.active_chat = None;
                app.ollama.branch_from = None;
                app.ollama.edit_message = None;
                app.ollama.thinking.clear();
            }
            app.ollama.library.chats.retain(|c| c.id != id);
            app.ollama.history_dirty = true;
        }
        "ui.agentClearHistory" => {
            #[cfg(not(target_arch = "wasm32"))]
            history::reset_store(app);
            app.ollama.history_error.clear();
            app.ollama.stop();
            app.ollama.entries.clear();
            app.ollama.images.clear();
            app.ollama.sources.clear();
            app.ollama.project_excerpts.clear();
            app.ollama.thinking.clear();
            app.ollama.active_chat = None;
            app.ollama.branch_from = None;
            app.ollama.edit_message = None;
            app.ollama.library.chats.clear();
            app.ollama.history_dirty = true;
        }
        "ui.agentRenameChat" => {
            let id = p.get("id").and_then(Value::as_u64).ok_or("id required")?;
            let title = p
                .get("title")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty() && s.len() <= 256)
                .ok_or("A title of up to 256 bytes is required")?;
            app.ollama.library.chats.iter_mut().find(|c| c.id == id).ok_or("Chat not found")?.title = clip(title.trim(), 96);
            app.ollama.history_dirty = true;
        }
        #[cfg(not(target_arch = "wasm32"))]
        "ui.agentProject" => {
            if let Some(path) = p.get("path").and_then(Value::as_str) {
                let folder = if path.is_empty() { String::new() } else { project::root(path)?.to_string_lossy().into_owned() };
                app.ollama.stop();
                app.ollama.project_folder = folder;
                app.ollama.project_files.clear();
                app.ollama.settings.files.clear();
                app.ollama.settings.scanned_folder.clear();
                app.ollama.settings.scan = None;
                if !app.ollama.project_folder.is_empty() {
                    settings::scan_project(app, true)?;
                }
                app.ollama.settings.open = true;
                app.ollama.settings.tab = settings::Tab::Project;
                // Revocation clears prior retrieved content from the conversation sent to providers.
                app.ollama.entries.clear();
                app.ollama.active_chat = None;
                app.ollama.branch_from = None;
                app.ollama.edit_message = None;
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
                app.ollama.cloud_consent = None;
                app.ollama.model.clear();
                app.ollama.models.clear();
                app.ollama.entries.clear();
                app.ollama.active_chat = None;
                app.ollama.branch_from = None;
                app.ollama.edit_message = None;
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
            app.ollama.active_chat = None;
            app.ollama.branch_from = None;
            app.ollama.edit_message = None;
            app.ollama.history_open = false;
            app.ollama.stop();
            app.ollama.history.clear();
            app.ollama.entries.clear();
            app.ollama.images.clear();
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
            // Validate the entire request before mutating live connection state.
            for (name, max) in [("endpoint", 2000), ("llamaEndpoint", 2000), ("model", 256), ("effort", 64), ("apiKey", 2000)] {
                if p.get(name).and_then(Value::as_str).is_some_and(|text| text.len() > max) {
                    return Err(format!("{name} is too long"));
                }
            }
            if p.get("endpoint").and_then(Value::as_str).is_some_and(|text| text != app.ollama.endpoint)
                || p.get("llamaEndpoint").and_then(Value::as_str).is_some_and(|text| text != app.ollama.llama_endpoint)
            {
                // Source remains Local when a user switches between Ollama servers.
                // Consent for one server must not carry over to the new recipient.
                if p.get("source").and_then(Value::as_str).is_some_and(|source| !matches!(source, "local" | "cloud" | "llamacpp" | "codex")) {
                    return Err("source must be local, cloud, llamacpp or codex".into());
                }
                app.ollama.cloud_consent = None;
            }
            if let Some(source) = p.get("source").and_then(Value::as_str) {
                let old = app.ollama.source;
                app.ollama.source = match source {
                    "local" => Source::Local,
                    "cloud" => Source::Cloud,
                    "llamacpp" => Source::LlamaCpp,
                    "codex" => Source::Codex,
                    _ => return Err("source must be local, cloud, llamacpp or codex".into()),
                };
                if old != app.ollama.source {
                    app.ollama.cloud_consent = None;
                }
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
            if app.ollama.busy {
                return Err("A request is already running".into());
            }
            if let Some(text) = p.get("text").and_then(Value::as_str) {
                if text.chars().count() > MAX_INPUT {
                    return Err("Message is too long".into());
                }
                app.ollama.input = text.into();
            }
            let ctx = app.ctx.clone().ok_or("The app is not ready yet")?;
            send(app, &ctx);
            if !app.ollama.busy {
                return Err(app.ollama.status.clone());
            }
        }
        _ => return Err("Ollama requires the desktop app".into()),
    }
    history::sync(app);
    Ok(inspect(app))
}

pub(crate) fn attach_image(app: &mut WordApp, path: String) -> Result<Value, String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if app.ollama.source != Source::Codex {
            return Err("Image attachments currently work with Codex.".into());
        }
        if app.ollama.images.len() >= MAX_AGENT_IMAGES {
            return Err(format!("Attach up to {MAX_AGENT_IMAGES} images to a chat."));
        }
        let picked = std::path::PathBuf::from(path);
        let _ = codex::image_inputs(std::slice::from_ref(&picked))?;
        let canonical = picked.canonicalize().map_err(|_| "Image is missing or cannot be opened.".to_string())?;
        if app.ollama.images.contains(&canonical) {
            return Err("That image is already attached to this chat.".into());
        }
        let name = canonical.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Image".into());
        app.ollama.images.push(canonical);
        app.ollama.status = format!("Attached {name}.");
        app.ollama.history_dirty = true;
        Ok(json!({"attached":true,"count":app.ollama.images.len()}))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (app, path);
        Err("Image attachments require the desktop app.".into())
    }
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

impl WordApp {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_agent_history(&mut self, path: std::path::PathBuf, ctx: &egui::Context) {
        history::initialize(self, path, ctx);
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn flush_agent_history(&self) -> Result<(), String> {
        history::flush(&self.ollama)
    }
}

//! Local conversations. Only completed visible messages are replayed, never tool calls or credentials.
use super::{Source, State, clip};
use serde_json::{Value, json};
const MAX_CHATS: usize = 50;
const MAX_BYTES: usize = 4 * 1024 * 1024;
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Conversation {
    pub id: u64,
    #[serde(default)]
    pub parent: Option<u64>,
    pub title: String,
    pub updated: u64,
    pub source: Source,
    pub model: String,
    #[serde(default)]
    pub effort: String,
    pub project: String,
    pub document: String,
    pub entries: Vec<(String, String)>,
    pub sources: Vec<(String, String)>,
    pub excerpts: Vec<(String, String)>,
    /// User-selected local images stay associated with this private local conversation.
    #[serde(default)]
    pub images: Vec<String>,
}
#[derive(Default, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Library {
    pub chats: Vec<Conversation>,
    pub next_id: u64,
}
impl Library {
    fn normalize(&mut self) {
        self.chats.truncate(MAX_CHATS);
        for chat in &mut self.chats {
            chat.title = clip(&chat.title, 96);
            chat.model = clip(&chat.model, 256);
            chat.effort = clip(&chat.effort, 64);
            chat.project = clip(&chat.project, 4096);
            chat.document = clip(&chat.document, 256);
            chat.entries.truncate(super::MAX_MESSAGES);
            chat.sources.truncate(96);
            chat.sources.retain(|(_, url)| url.starts_with("https://") || url.starts_with("http://") || url.starts_with("#project-"));
            chat.excerpts.truncate(96);
            chat.images.retain(|path| path.len() <= 4096 && std::path::Path::new(path).is_absolute());
            chat.images.truncate(super::MAX_AGENT_IMAGES);
            for (role, text) in &mut chat.entries {
                *role = clip(role, 32);
                *text = clip(text, 24_000);
            }
            for (title, url) in &mut chat.sources {
                *title = clip(title, 200);
                *url = clip(url, 4000);
            }
            for (url, text) in &mut chat.excerpts {
                *url = clip(url, 4000);
                *text = clip(text, 16_000);
            }
        }
        self.next_id = self.next_id.max(self.chats.iter().map(|c| c.id).max().unwrap_or(0));
    }
    fn bytes(&mut self) -> Result<Vec<u8>, String> {
        self.normalize();
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_BYTES {
            return Err("Chat history is full (4 MiB). Delete chats to save more.".into());
        }
        Ok(bytes)
    }
}
pub fn capture(state: &mut State, document: &str) {
    if !state.entries.iter().any(|(role, _)| role == "You") {
        return;
    }
    let existing = state.active_chat.and_then(|id| state.library.chats.iter().position(|c| c.id == id));
    if existing.is_none() && state.library.chats.len() >= MAX_CHATS {
        state.history_error = "Chat history is full (50 chats). Delete a chat to save this conversation.".into();
        return;
    }
    let previous_active = state.active_chat;
    let old = existing.map(|i| state.library.chats.remove(i));
    let id = if let Some(chat) = &old {
        chat.id
    } else {
        let mut next = None;
        for _ in 0..=MAX_CHATS {
            state.library.next_id = state.library.next_id.checked_add(1).unwrap_or(1);
            if !state.library.chats.iter().any(|c| c.id == state.library.next_id) {
                next = Some(state.library.next_id);
                break;
            }
        }
        let Some(id) = next else {
            state.history_error = "Could not allocate a chat ID.".into();
            return;
        };
        id
    };
    if state.history_error.starts_with("Chat history is full") {
        state.history_error.clear();
    }
    let title = old.as_ref().map(|c| c.title.clone()).unwrap_or_else(|| {
        clip(state.entries.iter().find(|(r, _)| r == "You").map(|(_, t)| t.as_str()).unwrap_or("New chat"), 72).replace(['\n', '\r'], " ")
    });
    state.active_chat = Some(id);
    state.library.chats.insert(
        0,
        Conversation {
            id,
            parent: state.branch_from,
            title,
            updated: crate::now_ms().max(0.0) as u64,
            source: state.source,
            model: state.model.clone(),
            effort: state.effort.clone(),
            project: state.project_folder.clone(),
            document: clip(document, 256),
            entries: state.entries.clone(),
            sources: state.sources.clone(),
            excerpts: state.project_excerpts.clone(),
            images: state.images.iter().map(|path| path.to_string_lossy().into_owned()).collect(),
        },
    );
    if let Err(error) = state.library.bytes() {
        if !state.library.chats.is_empty() {
            state.library.chats.remove(0);
        }
        if let Some(old) = old {
            state.library.chats.insert(existing.unwrap_or(0).min(state.library.chats.len()), old);
        }
        state.active_chat = previous_active;
        state.history_error = error;
    }
}
pub fn restore(state: &mut State, id: u64) -> Result<(), String> {
    let chat = state.library.chats.iter().find(|c| c.id == id).cloned().ok_or("Chat not found")?;
    state.stop();
    state.allow_edits = false;
    state.source = chat.source;
    state.model = chat.model;
    state.effort = chat.effort;
    state.project_folder = chat.project;
    state.entries = chat.entries;
    state.sources = chat.sources;
    state.project_excerpts = chat.excerpts;
    state.images = chat.images.into_iter().map(std::path::PathBuf::from).collect();
    // A fresh live document snapshot is added by send. Old tool/action history is never replayed.
    state.history = state
        .entries
        .iter()
        .filter_map(|(role, text)| match role.as_str() {
            "You" => Some(json!({"role":"user","content":text})),
            "Assistant" => Some(json!({"role":"assistant","content":text})),
            _ => None,
        })
        .collect();
    state.active_chat = Some(id);
    state.branch_from = chat.parent;
    state.edit_message = None;
    state.input.clear();
    state.history_open = false;
    state.review = Default::default();
    state.markdown = Default::default();
    state.thinking.clear();
    state.selected_source = None;
    state.document = None;
    state.turn_revision = None;
    state.models.clear();
    state.status = "Ready.".into();
    Ok(())
}
/// Prepare a new conversation transactionally. The original transcript and live document stay intact.
pub fn fork(state: &mut State, document: &str, index: usize, replacement: Option<&str>) -> Result<(), String> {
    if state.busy {
        return Err("Stop the current response before editing or branching.".into());
    }
    let (role, _) = state.entries.get(index).ok_or("Message not found")?;
    if !matches!(role.as_str(), "You" | "Assistant") {
        return Err("Choose a question or an answer.".into());
    }
    if let Some(text) = replacement {
        if role != "You" {
            return Err("Only your own messages can be edited.".into());
        }
        if text.trim().is_empty() || text.chars().count() > super::MAX_INPUT {
            return Err("Enter a message of up to 16,000 characters.".into());
        }
    }
    if state.library.chats.len() >= MAX_CHATS {
        return Err("Chat history is full. Delete a chat before creating a branch.".into());
    }
    capture(state, document);
    if !state.history_error.is_empty() {
        return Err(state.history_error.clone());
    }
    let mut draft = State::default();
    draft.library.clone_from(&state.library);
    draft.branch_from = state.active_chat;
    draft.source = state.source;
    draft.model.clone_from(&state.model);
    draft.effort.clone_from(&state.effort);
    draft.project_folder.clone_from(&state.project_folder);
    draft.images.clone_from(&state.images);
    draft.entries = state.entries.iter().take(index.saturating_add(1)).cloned().collect();
    if let Some(text) = replacement
        && let Some((_, prompt)) = draft.entries.last_mut()
    {
        *prompt = text.trim().to_string();
    }
    draft.sources.clone_from(&state.sources);
    draft.project_excerpts.clone_from(&state.project_excerpts);
    capture(&mut draft, document);
    let id = draft
        .active_chat
        .ok_or_else(|| if draft.history_error.is_empty() { "The branch needs a question first.".into() } else { draft.history_error.clone() })?;
    if !draft.history_error.is_empty() {
        return Err(draft.history_error.clone());
    }
    state.library = std::mem::take(&mut draft.library);
    restore(state, id)?;
    state.input.clear();
    state.history_dirty = true;
    state.status = "Branched chat · document editing is off.".into();
    Ok(())
}
pub fn inspect(state: &State) -> Value {
    json!({"active":state.active_chat,"count":state.library.chats.len(),"storageError":state.history_error,
      "chats":state.library.chats.iter().map(|c|json!({"id":c.id,"title":c.title,"updated":c.updated,"model":c.model,"document":c.document,"messages":c.entries.len(),"parent":c.parent})).collect::<Vec<_>>()})
}
#[cfg(not(target_arch = "wasm32"))]
mod disk {
    use super::*;
    use std::{
        fs,
        io::{Read, Write},
        path::{Path, PathBuf},
        sync::mpsc::{self, Receiver, Sender, SyncSender},
        time::Duration,
    };
    struct Job {
        bytes: Vec<u8>,
        reply: Option<Sender<Result<(), String>>>,
    }
    pub(crate) struct Store {
        tx: SyncSender<Job>,
        results: Receiver<Result<(), String>>,
        pending: Option<Vec<u8>>,
    }
    pub(super) fn load(path: &Path) -> Result<Library, String> {
        let file = match fs::File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Library::default()),
            Err(e) => return Err(e.to_string()),
        };
        let mut bytes = Vec::new();
        file.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_BYTES {
            return Err("Chat history file exceeds 4 MiB; it was left unchanged.".into());
        }
        let mut library: Library = serde_json::from_slice(&bytes).map_err(|_| "Chat history could not be read; it was left unchanged.")?;
        if library.chats.len() > MAX_CHATS {
            return Err("Chat history has too many conversations; it was left unchanged.".into());
        }
        library.normalize();
        Ok(library)
    }
    fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        if bytes.len() > MAX_BYTES {
            return Err("Chat history exceeds 4 MiB".into());
        }
        let parent = path.parent().ok_or("History needs a settings folder")?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        // create_new never truncates a neighbour, and the original survives failed preparation.
        let tmp = parent.join(format!(".chat-history-{}-{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp).map_err(|e| e.to_string())?;
        let result = (|| {
            file.write_all(bytes).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            drop(file);
            fs::rename(&tmp, path).map_err(|e| e.to_string())
        })();
        if result.is_err() {
            let _ = fs::remove_file(tmp);
        }
        result
    }
    impl Store {
        pub(super) fn new(path: PathBuf, ctx: egui::Context) -> Result<Self, String> {
            let (tx, rx) = mpsc::sync_channel::<Job>(1);
            let (results_tx, results) = mpsc::channel();
            std::thread::Builder::new()
                .name("wordcraft-chat-history".into())
                .spawn(move || {
                    while let Ok(job) = rx.recv() {
                        let result = write(&path, &job.bytes);
                        if let Some(reply) = job.reply {
                            let _ = reply.send(result.clone());
                        }
                        let _ = results_tx.send(result);
                        ctx.request_repaint();
                    }
                })
                .map_err(|e| e.to_string())?;
            Ok(Self { tx, results, pending: None })
        }
        pub(super) fn submit(&mut self, bytes: Vec<u8>) {
            self.pending = Some(bytes);
        }
        pub(super) fn poll(&mut self) -> Option<Result<(), String>> {
            if let Some(bytes) = self.pending.take() {
                match self.tx.try_send(Job { bytes, reply: None }) {
                    Ok(()) => {}
                    Err(mpsc::TrySendError::Full(job)) => self.pending = Some(job.bytes),
                    Err(_) => return Some(Err("Chat history worker stopped.".into())),
                }
            }
            let mut result = None;
            while let Ok(r) = self.results.try_recv() {
                result = Some(r);
            }
            result
        }
        pub(super) fn flush(&self, bytes: Vec<u8>) -> Result<(), String> {
            let (tx, rx) = mpsc::channel();
            self.tx.send(Job { bytes, reply: Some(tx) }).map_err(|_| "Chat history worker stopped")?;
            rx.recv_timeout(Duration::from_secs(5)).map_err(|_| "Chat history save did not finish")?
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn private_atomic_history_round_trips_and_deletion_is_durable() {
            let folder = std::env::temp_dir().join(format!("wordcraft-history-test-{}", std::process::id()));
            fs::create_dir_all(&folder).unwrap();
            let path = folder.join("chats.json");
            let mut state = State::default();
            state.entry("You", "Question".into());
            state.entry("Assistant", "**Answer**".into());
            capture(&mut state, "Paper");
            let store = Store::new(path.clone(), egui::Context::default()).unwrap();
            store.flush(state.library.bytes().unwrap()).unwrap();
            assert_eq!(load(&path).unwrap().chats.len(), 1);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
            }
            state.library.chats.clear();
            store.flush(state.library.bytes().unwrap()).unwrap();
            assert!(load(&path).unwrap().chats.is_empty());
            fs::write(&path, b"corrupt").unwrap();
            assert!(load(&path).is_err());
            assert_eq!(fs::read(&path).unwrap(), b"corrupt");
            drop(store);
            fs::remove_dir_all(folder).unwrap();
        }
    }
}
#[cfg(not(target_arch = "wasm32"))]
pub(super) use disk::Store;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn initialize(app: &mut crate::WordApp, path: std::path::PathBuf, ctx: &egui::Context) {
    app.ollama.history_path = Some(path.clone());
    match disk::load(&path) {
        Ok(library) => app.ollama.library = library,
        Err(e) => {
            app.ollama.history_error = e;
            return;
        }
    }
    match Store::new(path, ctx.clone()) {
        Ok(store) => app.ollama.history_store = Some(store),
        Err(e) => app.ollama.history_error = e,
    }
}
pub(super) fn sync(app: &mut crate::WordApp) {
    if app.ollama.history_dirty {
        let title = super::document_title(app);
        capture(&mut app.ollama, &title);
        app.ollama.history_dirty = false;
        #[cfg(not(target_arch = "wasm32"))]
        if app.ollama.history_store.is_some() && !app.ollama.history_error.starts_with("Chat history is full") {
            match app.ollama.library.bytes() {
                Ok(bytes) => {
                    if let Some(store) = &mut app.ollama.history_store {
                        store.submit(bytes)
                    }
                }
                Err(e) => app.ollama.history_error = e,
            }
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(store) = &mut app.ollama.history_store
        && let Some(result) = store.poll()
    {
        match result {
            Ok(()) => {
                if app.ollama.history_error.starts_with("Could not save chat history:") {
                    app.ollama.history_error.clear();
                }
            }
            Err(e) => app.ollama.history_error = format!("Could not save chat history: {e}"),
        }
    }
}
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn flush(state: &State) -> Result<(), String> {
    if let Some(store) = &state.history_store {
        // Capture the latest visible exchange even if closing precedes the next repaint.
        let mut snapshot = State::default();
        snapshot.library.clone_from(&state.library);
        snapshot.active_chat.clone_from(&state.active_chat);
        snapshot.branch_from = state.branch_from;
        snapshot.entries.clone_from(&state.entries);
        snapshot.sources.clone_from(&state.sources);
        snapshot.project_excerpts.clone_from(&state.project_excerpts);
        snapshot.source.clone_from(&state.source);
        snapshot.model.clone_from(&state.model);
        snapshot.effort.clone_from(&state.effort);
        snapshot.project_folder.clone_from(&state.project_folder);
        if state.history_dirty {
            let title = state.library.chats.iter().find(|c| Some(c.id) == state.active_chat).map(|c| c.document.as_str()).unwrap_or("Document");
            capture(&mut snapshot, title);
        }
        store.flush(snapshot.library.bytes()?)
    } else {
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn reset_store(app: &mut crate::WordApp) {
    if app.ollama.history_store.is_none()
        && let Some(path) = &app.ollama.history_path
    {
        let ctx = app.ctx.clone().unwrap_or_default();
        match Store::new(path.clone(), ctx) {
            Ok(store) => {
                app.ollama.history_store = Some(store);
                app.ollama.history_error.clear();
            }
            Err(e) => app.ollama.history_error = e,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restore_never_replays_tools_or_changes_document_permissions() {
        let mut state = State::default();
        state.allow_edits = true;
        state.entry("You", "A question".into());
        state.entry("Activity", "Edited a paragraph".into());
        state.entry("Assistant", "Answer".into());
        capture(&mut state, "Paper");
        let id = state.active_chat.unwrap();
        state.entry("You", "Follow up".into());
        capture(&mut state, "Paper");
        assert_eq!(state.library.chats.len(), 1);
        restore(&mut state, id).unwrap();
        assert!(!state.allow_edits);
        assert_eq!(state.history.len(), 3);
        assert!(state.history.iter().all(|v| v.get("tool_calls").is_none()));
    }
}

//! Native document windows. All callbacks run on the event-loop thread; shared views exchange
//! document/undo state between callbacks while retaining their own selection, zoom and scroll.
use egui::{ViewportBuilder, ViewportCommand, ViewportId};
use std::collections::HashSet;
use wordcraft_ui_egui::{WindowRequest, WordApp};

pub struct Window {
    id: u64,
    group: u64,
    identity: u64,
    app: WordApp,
    dialogs: crate::file_dialogs::Launcher,
    key: Option<crate::control_server::KeyFile>,
}

pub struct Windows {
    children: Vec<Window>,
    next: u64,
    root_group: u64,
    root_identity: u64,
    pub root_hidden: bool,
    pub quitting: bool,
    confirmed: HashSet<u64>,
    pending_quit: Option<u64>,
}
impl Default for Windows {
    fn default() -> Self {
        Self {
            children: Vec::new(),
            next: 1,
            root_group: 0,
            root_identity: 0,
            root_hidden: false,
            quitting: false,
            confirmed: HashSet::new(),
            pending_quit: None,
        }
    }
}

fn viewport(id: u64) -> ViewportId {
    if id == 0 { ViewportId::ROOT } else { ViewportId::from_hash_of(("document-window", id)) }
}

fn share_recents(target: &mut WordApp, source: &WordApp) {
    wordcraft_ui_egui::ollama::share_history(target, source);
    target.ui.recent.clone_from(&source.ui.recent);
    target.ui.recent_opened.clone_from(&source.ui.recent_opened);
    target.ui.favourite_documents.clone_from(&source.ui.favourite_documents);
    target.ui.theme = source.ui.theme;
    target.ui.interface_size = source.ui.interface_size;
    target.ui.animations = source.ui.animations;
    target.ui.language.clone_from(&source.ui.language);
}

fn title(app: &WordApp) -> String {
    app.display_title()
}
fn mark_opened(app: &mut WordApp) {
    if let Some(path) = &app.session.path {
        let path = path.to_string_lossy().into_owned();
        app.ui.recent.retain(|entry| entry != &path);
        app.ui.recent.insert(0, path.clone());
        app.ui.recent.truncate(wordcraft_ui_egui::MAX_RECENT);
        app.ui.recent_opened.insert(path, wordcraft_engine::cmd::now_unix());
    }
}

impl Windows {
    pub fn begin_quit(&mut self) {
        if !self.quitting {
            self.quitting = true;
            self.confirmed.clear();
            self.pending_quit = None;
        }
    }
    pub fn has_other_root_view(&self) -> bool {
        self.children.iter().any(|window| window.group == self.root_group)
    }

    pub fn close_root(&mut self, root: &mut WordApp, ctx: &egui::Context) {
        root.quit_requested = false;
        let _ = root.run("ui.ollamaStop", serde_json::json!({}));
        if self.children.is_empty() {
            let _ = root.execute("file.new", serde_json::json!({}));
            root.start_screen();
            self.root_identity = root.session.document_id();
            self.root_group = self.next;
            self.next = self.next.saturating_add(1);
        } else {
            self.root_hidden = true;
            ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Visible(false));
        }
    }

    /// Check dirty documents once per shared group; cancellation aborts Quit without dropping any
    /// windows. Close confirmation uses the normal Save/Don't Save/Cancel and async Save As path.
    pub fn confirm_quit(&mut self, root: &mut WordApp, ctx: &egui::Context) -> bool {
        if !self.quitting {
            return false;
        }
        if let Some(id) = self.pending_quit {
            let (app, group) = if id == 0 {
                (root, self.root_group)
            } else {
                let Some(window) = self.children.iter_mut().find(|window| window.id == id) else {
                    self.pending_quit = None;
                    return false;
                };
                (&mut window.app, window.group)
            };
            if app.quit_requested {
                app.quit_requested = false;
                self.confirmed.insert(group);
                self.pending_quit = None;
            } else if app.dialog.is_none() && !app.file_dialog_open() {
                self.quitting = false;
                self.pending_quit = None;
            }
            return false;
        }
        let next = if !self.root_hidden && !self.confirmed.contains(&self.root_group) {
            Some(0)
        } else {
            self.children.iter().find(|window| !self.confirmed.contains(&window.group)).map(|window| window.id)
        };
        let Some(id) = next else {
            return true;
        };
        let (app, group) = if id == 0 {
            (root, self.root_group)
        } else {
            let Some(window) = self.children.iter_mut().find(|window| window.id == id) else {
                return false;
            };
            (&mut window.app, window.group)
        };
        let other_views = app.other_document_views;
        app.other_document_views = false;
        let accepted = app.close_requested();
        app.other_document_views = other_views;
        if accepted {
            self.confirmed.insert(group);
        } else {
            self.pending_quit = Some(id);
            ctx.send_viewport_cmd_to(viewport(id), ViewportCommand::Focus);
        }
        ctx.request_repaint();
        false
    }

    fn add(&mut self, root: &mut WordApp, source: Option<usize>, request: WindowRequest, ctx: &egui::Context) {
        match request {
            WindowRequest::Quit => {
                self.begin_quit();
                return;
            }
            WindowRequest::Close(id) | WindowRequest::Connect(id) => {
                let command = if matches!(request, WindowRequest::Close(_)) { "file.close" } else { "ui.codexConnect" };
                let target = if id == 0 { Some(root) } else { self.children.iter_mut().find(|window| window.id == id).map(|window| &mut window.app) };
                if let Some(target) = target
                    && let Err(error) = target.run(command, serde_json::json!({"value":true}))
                {
                    target.status(error);
                }
                return;
            }
            WindowRequest::Focus(id) => {
                ctx.send_viewport_cmd_to(viewport(id), ViewportCommand::Focus);
                return;
            }
            WindowRequest::Arrange => {
                let size = ctx.input(|input| input.viewport().monitor_size).unwrap_or(egui::vec2(1440.0, 900.0));
                let count = self.children.len() + usize::from(!self.root_hidden);
                let columns = (count as f32).sqrt().ceil().max(1.0) as usize;
                let rows = count.div_ceil(columns).max(1);
                let inner = egui::vec2((size.x / columns as f32).max(760.0), ((size.y - 60.0) / rows as f32).max(480.0));
                let ids = std::iter::once(0).filter(|_| !self.root_hidden).chain(self.children.iter().map(|window| window.id));
                for (index, id) in ids.enumerate() {
                    ctx.send_viewport_cmd_to(
                        viewport(id),
                        ViewportCommand::OuterPosition(egui::pos2((index % columns) as f32 * inner.x, 30.0 + (index / columns) as f32 * inner.y)),
                    );
                    ctx.send_viewport_cmd_to(viewport(id), ViewportCommand::InnerSize(inner));
                }
                return;
            }
            _ => {}
        }
        if self.children.len() >= 15 || self.next == u64::MAX {
            self.report_error(root, source, "Close a document window before opening another (maximum 16).".into());
            return;
        }
        if let WindowRequest::Document { command, params } = &request
            && command == "file.open"
            && let Some(path) = params.get("path").and_then(serde_json::Value::as_str)
        {
            let path = std::path::Path::new(path).canonicalize().ok();
            let same = |app: &WordApp| {
                path.as_ref().is_some_and(|path| app.session.path.as_ref().and_then(|existing| existing.canonicalize().ok()).as_ref() == Some(path))
            };
            if !self.root_hidden && same(root) {
                mark_opened(root);
                ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Focus);
                return;
            }
            if let Some(window) = self.children.iter_mut().find(|window| same(&window.app)) {
                mark_opened(&mut window.app);
                share_recents(root, &window.app);
                ctx.send_viewport_cmd_to(viewport(window.id), ViewportCommand::Focus);
                return;
            }
        }
        let original = source.and_then(|index| self.children.get(index)).map(|window| &window.app).unwrap_or(root);
        let prefs = original.prefs();
        let shared = matches!(request, WindowRequest::SameDocument);
        let session = if shared { original.session.fork_window() } else { wordcraft_engine::Session::new(original.session.blank_document()) };
        let group = source.and_then(|index| self.children.get(index)).map(|window| window.group).unwrap_or(self.root_group);
        let (hook, dialogs) = crate::file_dialogs::hook(ctx);
        let mut app = WordApp::new(session, crate::services(hook, ctx));
        app.apply_prefs(prefs);
        share_recents(&mut app, original);
        app.integrated_titlebar = cfg!(target_os = "macos");
        app.ui.backstage = false;
        app.ui.window = None;
        app.watch_desktop_theme(ctx);
        if let WindowRequest::Document { command, params } = request {
            // Opening is completed in the new window; a password prompt belongs there too.
            if let Err(error) = app.run(&command, params) {
                if app.dialog.is_none() {
                    self.report_error(root, source, error);
                    return;
                }
                app.status(error);
            }
        }
        app.native_document_windows = true;
        let id = self.next;
        self.next = self.next.saturating_add(1);
        let identity = app.session.document_id();
        self.children.push(Window { id, group: if shared { group } else { id }, identity, app, dialogs, key: None });
        ctx.request_repaint();
    }

    fn report_error(&mut self, root: &mut WordApp, source: Option<usize>, error: String) {
        let index = source.or_else(|| self.root_hidden.then(|| self.children.len().saturating_sub(1)));
        if let Some(window) = index.and_then(|index| self.children.get_mut(index)) {
            window.app.status(error);
        } else {
            root.status(error);
        }
    }

    pub fn queue_document(&mut self, root: &mut WordApp, command: &str, params: serde_json::Value, ctx: &egui::Context) {
        if self.root_hidden && self.children.is_empty() {
            self.root_hidden = false;
            ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Visible(true));
        }
        if root.ui.backstage && root.session.path.is_none() && !root.session.dirty && self.children.is_empty() {
            match root.execute(command, params) {
                Ok(_) => root.ui.backstage = false,
                Err(error) => root.status(error),
            }
        } else {
            self.add(root, None, WindowRequest::Document { command: command.into(), params }, ctx);
        }
    }

    pub fn paint(&mut self, root: &mut WordApp, ctx: &egui::Context) {
        if self.root_identity != root.session.document_id() {
            self.root_identity = root.session.document_id();
            self.root_group = self.next;
            self.next = self.next.saturating_add(1);
        }
        for window in &mut self.children {
            share_recents(&mut window.app, root);
            if window.group == self.root_group {
                window.app.session.sync_window_document(&root.session);
            }
        }
        for request in root.take_window_requests() {
            self.add(root, None, request, ctx);
        }
        root.other_document_views = self.has_other_root_view();
        root.window_view_number = root.other_document_views.then_some(1);
        for index in 0..self.children.len() {
            let Some(window) = self.children.get(index) else {
                continue;
            };
            let group = window.group;
            let root_view = !self.root_hidden && group == self.root_group;
            let count = self.children.iter().filter(|window| window.group == group).count() + usize::from(root_view);
            let number = self.children.iter().take(index + 1).filter(|window| window.group == group).count() + usize::from(root_view);
            if let Some(window) = self.children.get_mut(index) {
                window.app.other_document_views = count > 1;
                window.app.window_view_number = (count > 1).then_some(number);
            }
        }
        let mut closed = Vec::new();
        let count = self.children.len();
        let labels: Vec<_> = std::iter::once((0, title(root)))
            .filter(|_| !self.root_hidden)
            .chain(self.children.iter().map(|window| (window.id, title(&window.app))))
            .collect();
        root.document_windows.clone_from(&labels);
        for index in 0..count {
            let control_base = root.codex_port().unwrap_or(7981);
            let other_view = self.children.get(index).is_some_and(|window| {
                (!self.root_hidden && window.group == self.root_group)
                    || self.children.iter().enumerate().any(|(other, sibling)| other != index && sibling.group == window.group)
            });
            let Some(window) = self.children.get_mut(index) else {
                continue;
            };
            window.app.document_windows.clone_from(&labels);
            let mut closing = false;
            let builder = ViewportBuilder::default()
                .with_title(title(&window.app))
                .with_inner_size([1200.0, 840.0])
                .with_min_inner_size([760.0, 480.0])
                .with_drag_and_drop(true)
                .with_fullsize_content_view(true)
                .with_titlebar_shown(false)
                .with_title_shown(false);
            ctx.show_viewport_immediate(viewport(window.id), builder, |ui, _class| {
                let child_ctx = ui.ctx().clone();
                window.app.logic(&child_ctx);
                crate::update_control(&mut window.app, &mut window.key, &child_ctx, control_base.saturating_add(window.id.min(1000) as u16));
                ui.push_id(window.id, |ui| window.app.ui(ui));
                window.dialogs.show_unparented();
                let close = child_ctx.input(|input| input.viewport().close_requested());
                if !self.quitting && (close || window.app.quit_requested) {
                    child_ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                    closing = other_view || window.app.close_requested();
                }
            });
            if window.identity != window.app.session.document_id() {
                window.identity = window.app.session.document_id();
                window.group = self.next;
                self.next = self.next.saturating_add(1);
            }
            share_recents(root, &window.app);
            if window.group == self.root_group {
                root.session.sync_window_document(&window.app.session);
            }
            let group = window.group;
            let (before, from) = self.children.split_at_mut(index);
            if let Some((source, after)) = from.split_first_mut() {
                for sibling in before.iter_mut().chain(after) {
                    share_recents(&mut sibling.app, &source.app);
                    if sibling.group == group {
                        sibling.app.session.sync_window_document(&source.app.session);
                    }
                }
            }
            let requests = self.children.get_mut(index).map(|window| window.app.take_window_requests()).unwrap_or_default();
            for request in requests {
                self.add(root, Some(index), request, ctx);
            }
            if closing {
                closed.push(index);
            }
        }
        for index in closed.into_iter().rev() {
            if index < self.children.len() {
                let window = self.children.remove(index);
                let _ = window.app.flush_agent_history();
                if let Some(key) = window.key {
                    key.remove();
                }
            }
        }
        if self.root_hidden && self.children.is_empty() {
            self.root_hidden = false;
            let _ = root.execute("file.new", serde_json::json!({}));
            root.start_screen();
            ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Visible(true));
        }
    }
    pub fn shutdown(&self) {
        for window in &self.children {
            let _ = window.app.flush_agent_history();
            if let Some(key) = &window.key {
                key.remove();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn root() -> WordApp {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Keep my writing")), Default::default());
        app.native_document_windows = true;
        app.session.run("text.insert", &json!({"text":"Unsaved "})).unwrap();
        app
    }
    #[test]
    fn new_document_and_shared_view_preserve_the_original_and_undo() {
        let ctx = egui::Context::default();
        let mut root = root();
        let before = root.session.doc.clone();
        let mut windows = Windows::default();
        windows.add(&mut root, None, WindowRequest::Document { command: "file.new".into(), params: json!({}) }, &ctx);
        assert_eq!(windows.children.len(), 1);
        assert_eq!(root.session.doc, before);
        assert!(root.session.dirty);
        assert!(!windows.children[0].app.session.dirty);
        windows.add(&mut root, None, WindowRequest::SameDocument, &ctx);
        assert_eq!(windows.children[1].group, windows.root_group);
        let view = &mut windows.children[1].app.session;
        assert_eq!(view.doc, before);
        view.run("edit.undo", &json!({})).unwrap();
        root.session.sync_window_document(view);
        assert_eq!(root.session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Keep my writing");
    }
    #[test]
    fn quit_checks_shared_dirty_document_and_cancel_keeps_all_windows() {
        let ctx = egui::Context::default();
        let mut root = root();
        let before = root.session.doc.clone();
        let mut windows = Windows::default();
        windows.add(&mut root, None, WindowRequest::SameDocument, &ctx);
        root.other_document_views = true;
        windows.begin_quit();
        assert!(!windows.confirm_quit(&mut root, &ctx));
        assert!(root.dialog.is_some());
        root.run("ui.saveChanges", json!({"answer":"cancel"})).unwrap();
        assert!(!windows.confirm_quit(&mut root, &ctx));
        assert!(!windows.quitting);
        assert_eq!(windows.children.len(), 1);
        assert_eq!(root.session.doc, before);
        assert!(root.session.dirty);
    }
    #[test]
    fn duplicate_file_open_focuses_existing_window_and_failed_open_keeps_original() {
        let ctx = egui::Context::default();
        let mut root = root();
        let original = root.session.doc.clone();
        let mut windows = Windows::default();
        let folder = std::env::temp_dir().join(format!("wordcraft-window-open-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let missing = folder.join("missing.docx");
        windows.add(&mut root, None, WindowRequest::Document { command: "file.open".into(), params: json!({"path":missing}) }, &ctx);
        assert!(windows.children.is_empty());
        assert_eq!(root.session.doc, original);
        let path = folder.join("synthetic.docx");
        wordcraft_engine::io::save_path(&path, &wordcraft_doc::Document::from_text("Independent file")).unwrap();
        for _ in 0..2 {
            windows.add(&mut root, None, WindowRequest::Document { command: "file.open".into(), params: json!({"path":path}) }, &ctx);
        }
        assert_eq!(windows.children.len(), 1);
        assert_eq!(root.session.doc, original);
        assert!(root.session.dirty);
        assert!(root.ui.recent_opened.contains_key(&path.to_string_lossy().into_owned()));
        assert_eq!(windows.children[0].app.session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Independent file");
        std::fs::remove_dir_all(folder).unwrap();
    }
    #[test]
    fn opening_from_empty_start_screen_reuses_root_and_all_window_limit_is_enforced() {
        let ctx = egui::Context::default();
        let mut root = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        root.native_document_windows = true;
        root.start_screen();
        assert!(!root.has_document());
        let mut windows = Windows::default();
        windows.queue_document(&mut root, "file.new", json!({}), &ctx);
        assert!(root.has_document());
        assert!(windows.children.is_empty());
        for _ in 0..16 {
            windows.add(&mut root, None, WindowRequest::Document { command: "file.new".into(), params: json!({}) }, &ctx);
        }
        assert_eq!(windows.children.len(), 15);
        assert!(root.has_document());
    }
}

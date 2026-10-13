//! Explicit agent capabilities. File approval and cloud consent come only from visible controls.
use super::{Source, WordApp, command, connection_menu, uses_cloud};
use crate::theme::{self, Tokens};
use egui::{RichText, Stroke, Ui};
use serde_json::json;

#[derive(Default, Clone, Copy, PartialEq)]
pub(super) enum Tab {
    #[default]
    General,
    Permissions,
    Project,
    Data,
}
impl Tab {
    fn title(self) -> &'static str {
        match self {
            Self::General => "Models & connection",
            Self::Permissions => "Permissions",
            Self::Project => "Project files",
            Self::Data => "Data controls",
        }
    }
}
#[derive(Default)]
pub(super) struct State {
    pub open: bool,
    pub tab: Tab,
    pub files: Vec<String>,
    pub scanned_folder: String,
    filter: String,
    error: String,
}

fn toggle(ui: &mut Ui, value: &mut bool, label: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let (rect, mut response) = ui.allocate_exact_size(egui::vec2(36.0, 22.0), egui::Sense::click());
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *value, label));
    let animation = ui.ctx().animate_bool(response.id, *value);
    ui.painter().rect_filled(rect, 11, if *value { crate::backstage::stage_accent(&t) } else { t.chat_user });
    ui.painter().circle_filled(
        egui::pos2(rect.min.x + 11.0 + animation * 14.0, rect.center().y),
        8.0,
        if *value { egui::Color32::WHITE } else { t.chat_dim },
    );
    response.on_hover_text(label).changed()
}
fn switch_row(ui: &mut Ui, title: &str, detail: &str, value: &mut bool) -> bool {
    let mut changed = false;
    ui.push_id(title, |ui| {
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(egui::vec2((ui.available_width() - 50.0).max(100.0), 44.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.label(RichText::new(title).font(theme::medium(14.0)));
                ui.add(egui::Label::new(RichText::new(detail).size(11.5).color(Tokens::get(ui.ctx()).chat_dim)).wrap());
            });
            changed = toggle(ui, value, title);
        });
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);
    });
    changed
}

pub(super) fn show(app: &mut WordApp, ctx: &egui::Context) {
    if !app.ollama.settings.open {
        return;
    }
    let t = Tokens::get(ctx);
    let mut open = true;
    let width = (ctx.content_rect().width() - 40.0).clamp(320.0, 760.0);
    egui::Window::new("Agent settings")
        .id(egui::Id::new("agent_settings"))
        .open(&mut open)
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .default_width(width)
        .min_height((ctx.content_rect().height() - 100.0).clamp(380.0, 540.0))
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .frame(egui::Frame::NONE.stroke(Stroke::new(1.0, t.border)).fill(t.chat).corner_radius(20).inner_margin(22))
        .show(ctx, |ui| {
            ui.style_mut().text_styles.insert(egui::TextStyle::Body, theme::regular(14.0));
            ui.visuals_mut().override_text_color = Some(t.chat_text);
            ui.visuals_mut().selection.bg_fill = t.chat_user;
            ui.visuals_mut().selection.stroke = Stroke::new(1.0, t.chat_text);
            ui.visuals_mut().widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
            ui.visuals_mut().widgets.hovered.weak_bg_fill = t.chat_user;
            ui.spacing_mut().item_spacing = egui::vec2(12.0, 10.0);
            ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
            ui.spacing_mut().scroll.foreground_color = true;
            ui.horizontal(|ui| {
                ui.label(RichText::new("Settings").font(theme::semibold(18.0)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Button::new("×").frame(false)).on_hover_text("Close agent settings").clicked() {
                        app.ollama.settings.open = false;
                    }
                });
            });
            ui.add_space(12.0);
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(egui::vec2(142.0, 360.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.small(RichText::new("AGENT CHAT").color(t.chat_dim));
                    for tab in [Tab::General, Tab::Permissions, Tab::Project, Tab::Data] {
                        if ui
                            .add(
                                egui::Button::new(tab.title())
                                    .selected(app.ollama.settings.tab == tab)
                                    .frame(app.ollama.settings.tab == tab)
                                    .min_size(egui::vec2(138.0, 36.0)),
                            )
                            .clicked()
                        {
                            app.ollama.settings.tab = tab;
                        }
                    }
                    ui.add_space(22.0);
                    if ui.button("Back to chat").clicked() {
                        app.ollama.settings.open = false;
                    }
                });
                ui.separator();
                ui.vertical(|ui| {
                    ui.set_width((width - 210.0).max(150.0));
                    ui.label(RichText::new(app.ollama.settings.tab.title()).font(theme::semibold(22.0)));
                    ui.add_space(14.0);
                    egui::ScrollArea::vertical()
                        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                        .max_height((ctx.content_rect().height() - 170.0).clamp(180.0, 460.0))
                        .show(ui, |ui| match app.ollama.settings.tab {
                            Tab::General => {
                                ui.add_enabled_ui(!app.ollama.busy, |ui| {
                                    connection_menu(app, ui, ctx);
                                });
                                if app.ollama.busy {
                                    ui.small("Stop the active request to change models or connection.");
                                }
                                ui.add_space(12.0);
                                ui.label("Enter sends · Shift+Enter adds a line");
                                ui.small("Each window has its own conversation and permissions.");
                            }
                            Tab::Permissions => permissions(app, ui),
                            Tab::Project => project(app, ui),
                            Tab::Data => data(app, ui),
                        });
                });
            });
        });
    app.ollama.settings.open &= open;
}
fn permissions(app: &mut WordApp, ui: &mut Ui) {
    let mut edits = app.ollama.allow_edits;
    if switch_row(
        ui,
        "Allow document editing",
        "Changes use live document tools and the document’s undo history. Off blocks every AI write.",
        &mut edits,
    ) {
        let _ = command(app, "ui.ollamaEdit", &json!({"value":edits}));
    }
    let mut web = app.ollama.internet;
    if switch_row(
        ui,
        "Search the web",
        "Search queries go to Bing. Pages are fetched over public HTTPS; local-network targets are blocked.",
        &mut web,
    ) {
        let _ = command(app, "ui.ollamaInternet", &json!({"value":web}));
    }
    let cloud = uses_cloud(&app.ollama);
    let mut allowed = app.ollama.cloud_consent == Some(app.ollama.source);
    ui.add_enabled_ui(cloud, |ui| {
        let recipient = if app.ollama.source == Source::Codex { "OpenAI" } else { "Ollama / your model provider" };
        if switch_row(ui, "Share context with cloud models", &format!("Send this document, selected project passages, messages and attached images to {recipient}. Permission lasts for this session and provider."), &mut allowed) {
            if !allowed { app.ollama.stop(); }
            app.ollama.cloud_consent = allowed.then_some(app.ollama.source);
        }
    });
    ui.small("Local servers can forward requests according to their own configuration. WordCraft cannot enforce a server’s data-retention or training policy.");
    ui.add_space(10.0);
    ui.label(RichText::new("Codex / MCP control").font(theme::medium(14.0)));
    ui.horizontal(|ui| {
        ui.label(if app.codex_ready { "Enabled · authenticated localhost port" } else { "Disabled · port closed" });
        if ui.button(if app.codex_ready { "Disable" } else { "Enable" }).clicked() {
            let _ = app.run("ui.codexConnect", json!({"value":!app.codex_ready}));
        }
    });
    ui.small("The connection key is private to your OS account. Programs running as your user can read it; connect only trusted agents.");
}
fn project(app: &mut WordApp, ui: &mut Ui) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        ui.add(egui::Label::new("Only checked files are available to the model. This selection is cleared when you reopen the app, change folders or restore a saved chat.").wrap());
        ui.horizontal(|ui| {
            if ui.button("Choose folder…").clicked() {
                let _ = command(app, "ui.agentProject", &json!({}));
            }
            if !app.ollama.project_folder.is_empty() && ui.button("Unlink").clicked() {
                let _ = command(app, "ui.agentProject", &json!({"path":""}));
            }
        });
        if app.ollama.project_folder.is_empty() {
            return;
        }
        ui.add(egui::Label::new(&app.ollama.project_folder).wrap());
        if app.ollama.settings.scanned_folder != app.ollama.project_folder || ui.button("Refresh file list").clicked() {
            app.ollama.settings.scanned_folder.clone_from(&app.ollama.project_folder);
            match super::project::candidates(&app.ollama.project_folder) {
                Ok(files) => {
                    app.ollama.settings.files = files;
                    app.ollama.settings.error.clear();
                }
                Err(error) => app.ollama.settings.error = error,
            }
        }
        ui.small(format!("{} files approved · maximum 128", app.ollama.project_files.len()));
        ui.add(egui::TextEdit::singleline(&mut app.ollama.settings.filter).hint_text("Search files").desired_width(ui.available_width()));
        if ui.button("Revoke all file access").clicked() {
            super::revoke_project_context(app);
            app.ollama.project_files.clear();
        }
        let filter = app.ollama.settings.filter.to_lowercase();
        let files = app.ollama.settings.files.clone();
        ui.small("Showing up to 256 matching files. Search to find other files.");
        for path in files.iter().filter(|path| path.to_lowercase().contains(&filter)).take(256) {
            let mut selected = app.ollama.project_files.contains(path);
            if ui.checkbox(&mut selected, path).changed() {
                let mut approved = app.ollama.project_files.clone();
                if selected {
                    approved.push(path.clone());
                } else {
                    approved.retain(|entry| entry != path);
                }
                match super::project::validate_selection(&app.ollama.project_folder, &approved) {
                    Ok(()) => {
                        if selected {
                            app.ollama.stop();
                        } else {
                            super::revoke_project_context(app);
                        }
                        app.ollama.project_files = approved;
                        app.ollama.settings.error.clear();
                    }
                    Err(error) => app.ollama.settings.error = error,
                }
            }
        }
        if !app.ollama.settings.error.is_empty() {
            ui.colored_label(Tokens::get(ui.ctx()).red, &app.ollama.settings.error);
        }
        ui.small("Hidden files, private-key files and detected credential contents are excluded. Detection is a safeguard, not a guarantee—review each file before selecting it. Approved files are read live.");
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = app;
        ui.label("Project folders require the desktop app.");
    }
}
fn data(app: &mut WordApp, ui: &mut Ui) {
    switch_row(
        ui,
        "Save new chats locally",
        "Off makes new turns temporary. Existing saved chats remain until you delete them.",
        &mut app.ollama.save_history,
    );
    ui.small("History is plaintext JSON, restricted to your OS account on Unix. Other processes running as your user can read it. Use your OS disk encryption for protection at rest.");
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.label("Saved conversations");
        if ui.button("Manage").clicked() {
            app.ollama.history_open = true;
            app.ollama.settings.open = false;
        }
    });
    ui.separator();
    ui.horizontal(|ui| {
        ui.label("Delete all saved chats");
        if ui.add(egui::Button::new(RichText::new("Delete all…").color(Tokens::get(ui.ctx()).red))).clicked() {
            app.ollama.delete_all = true;
            app.ollama.settings.open = false;
        }
    });
    ui.add_space(16.0);
    ui.small("Cloud retention and training settings belong to the provider’s account. Temporary Chat here controls local saving; it does not change provider retention.");
}

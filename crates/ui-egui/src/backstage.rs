//! File tab (Backstage): Home/New, Open, Info, Save As, Print, Export, Close, Options, About.

use egui::{Align2, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;

use crate::theme::{APP_COLOR, Tokens, medium, regular, semibold};
use crate::{WordApp, icons};

const PAGES: [(&str, &str); 10] = [
    ("home", "Home"),
    ("new", "New"),
    ("open", "Open"),
    ("info", "Info"),
    ("save", "Save"),
    ("saveAs", "Save As"),
    ("print", "Print"),
    ("export", "Export"),
    ("close", "Close"),
    ("options", "Options"),
];

pub fn show(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    // No document open (#1129): only the pages that open or create one, and Options.
    let has_document = app.has_document();
    if !has_document && !works_without_document(&app.ui.backstage_page) {
        app.ui.backstage_page = "home".into();
    }
    let mac = app.integrated_titlebar;
    if mac {
        show_mac(app, ui);
        return;
    }
    egui::Panel::left("backstage_nav")
        .exact_size(200.0)
        // Fixed width, as in Word: without this egui shows a resize handle that does nothing (#502).
        .resizable(false)
        .frame(egui::Frame::NONE.fill(APP_COLOR).inner_margin(egui::Margin { left: 0, right: 0, top: if mac { 0 } else { 12 }, bottom: 12 }))
        .show(ui, |ui| {
            let back = if mac {
                // macOS (#222): the Backstage covers the title bar, so its top band is the title
                // bar: the back button sits beside the traffic lights, centred with them and as far
                // from them as the title bar's own controls, and the rest of the band drags the window.
                let (band, drag) = ui.allocate_exact_size(vec2(200.0, crate::chrome::MAC_TITLE_BAR), Sense::click_and_drag());
                if drag.drag_started() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                // The chevron's stroke starts at the content edge.
                let c = pos2(band.min.x + crate::chrome::MAC_CONTENT_LEFT + 19.0, band.center().y);
                let resp = ui.interact(Rect::from_center_size(c, vec2(22.0, band.height())), ui.id().with("backstage_back"), Sense::click());
                icons::paint(ui.painter(), Rect::from_center_size(c, vec2(18.0, 18.0)), "chevronLeft", egui::Color32::WHITE, egui::Color32::WHITE);
                resp
            } else {
                let (r, resp) = ui.allocate_exact_size(vec2(200.0, 40.0), Sense::click());
                icons::paint(
                    ui.painter(),
                    Rect::from_center_size(pos2(r.min.x + 26.0, r.center().y), vec2(18.0, 18.0)),
                    "chevronLeft",
                    egui::Color32::WHITE,
                    egui::Color32::WHITE,
                );
                resp
            };
            // Escape in the built-in file browser closes only the browser.
            if back.clicked() || (!app.file_dialog_open() && ui.input(|i| i.key_pressed(egui::Key::Escape))) {
                app.ui.backstage = false;
                app.canvas.want_focus = true;
            }
            ui.add_space(if mac { 12.0 } else { 6.0 });
            for (id, label) in PAGES {
                // No document open (#1129): Info, Save, Save As, Print and Export are greyed out.
                let on = has_document || works_without_document(id);
                let (r, resp) = ui.add_enabled_ui(on, |ui| ui.allocate_exact_size(vec2(200.0, 38.0), Sense::click())).inner;
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, on, tl!(label)));
                let active = app.ui.backstage_page == id;
                if active {
                    ui.painter().rect_filled(r, 0.0, egui::Color32::from_white_alpha(46));
                } else if on && resp.hovered() {
                    ui.painter().rect_filled(r, 0.0, egui::Color32::from_white_alpha(26));
                }
                ui.painter().text(
                    pos2(r.min.x + 22.0, r.center().y),
                    Align2::LEFT_CENTER,
                    tl!(label),
                    if active { semibold(14.0) } else { medium(14.0) },
                    if on { egui::Color32::WHITE } else { egui::Color32::from_white_alpha(110) },
                );
                if resp.clicked() {
                    match id {
                        // A document without a file goes to the Save As page (as in Word) instead
                        // of `file.save`, which would also open the native Save As dialog.
                        // In the browser a save is a download, so it saves right away.
                        "save" if app.session.path.is_none() && app.services.download.is_none() => {
                            app.ui.backstage_page = "saveAs".into();
                        }
                        "save" => {
                            let r = app.run("file.save", json!({}));
                            if r.is_ok_and(|v| v.get("saved").and_then(serde_json::Value::as_bool) == Some(true) || v.get("downloaded").is_some()) {
                                app.ui.backstage = false;
                                app.canvas.want_focus = true;
                            } else {
                                app.ui.backstage_page = "saveAs".into();
                            }
                        }
                        "saveAs" => {
                            app.ui.backstage_page = "saveAs".into();
                        }
                        // Not a page: `file.close` asks Save / Don't Save / Cancel first when there
                        // are unsaved changes, and Cancel leaves the Backstage where it was.
                        "close" => {
                            let _ = app.run("file.close", json!({}));
                        }
                        _ => app.ui.backstage_page = id.into(),
                    }
                }
            }
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                for (label, id) in [("About", "about"), ("Discord community", "discord")] {
                    let (r, resp) = ui.allocate_exact_size(vec2(200.0, 32.0), Sense::click());
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tl!(label)));
                    if resp.hovered() {
                        ui.painter().rect_filled(r, 0.0, egui::Color32::from_white_alpha(26));
                    }
                    ui.painter().text(pos2(r.min.x + 22.0, r.center().y), Align2::LEFT_CENTER, tl!(label), regular(13.0), egui::Color32::WHITE);
                    if resp.clicked() {
                        if id == "discord" {
                            let _ = app.run("ui.discord", json!({}));
                        } else {
                            app.dialog = crate::dialogs::Dialog::open("about", app);
                        }
                    }
                }
            });
        });
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.ribbon).inner_margin(egui::Margin::symmetric(40, 30))).show(ui, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| match app.ui.backstage_page.as_str() {
            "home" => home_page(app, ui),
            "new" => new_page(app, ui),
            "open" => open_page(app, ui),
            "saveAs" => save_as_page(app, ui),
            "info" => info_page(app, ui),
            "print" | "export" => export_page(app, ui),
            "options" => crate::options::page(app, ui),
            _ => home_page(app, ui),
        });
    });
}

/// Mac document stage: narrow neutral navigation and a separate title/footer band.
fn show_mac(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    ctx.data_mut(|d| {
        let id = egui::Id::new("mac_selected_template");
        if d.get_temp::<String>(id).is_none() {
            d.insert_temp(id, "blank".to_string());
        }
    });
    let recent = app.ui.backstage_page == "recent";
    egui::Panel::top("mac_stage_title").exact_size(crate::chrome::MAC_TITLE_BAR + 12.0).frame(egui::Frame::NONE.fill(t.title_bar)).show(ui, |ui| {
        let (r, drag) = ui.allocate_exact_size(vec2(ui.available_width(), crate::chrome::MAC_TITLE_BAR + 12.0), Sense::click_and_drag());
        if drag.drag_started() {
            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        ui.painter().text(r.center(), Align2::CENTER_CENTER, "WordCraft", medium(14.0), t.text);
        let search_id = egui::Id::new("mac_document_search");
        let mut search = ctx.data(|data| data.get_temp::<String>(search_id)).unwrap_or_default();
        if ui
            .put(
                Rect::from_min_size(pos2(r.max.x - 196.0, r.min.y + 9.0), vec2(180.0, 26.0)),
                egui::TextEdit::singleline(&mut search).hint_text("Search").frame(egui::Frame::NONE),
            )
            .changed()
        {
            ctx.data_mut(|data| data.insert_temp(search_id, search));
        }
        let back = Rect::from_center_size(pos2(r.min.x + crate::chrome::MAC_CONTENT_LEFT + 16.0, r.center().y), vec2(28.0, 28.0));
        icons::paint(ui.painter(), back.shrink(5.0), "chevronLeft", t.icon, t.accent);
        if ui.interact(back, ui.id().with("stage_back"), Sense::click()).on_hover_text("Return to document").clicked() {
            app.ui.backstage = false;
            app.canvas.want_focus = true;
        }
    });
    egui::Panel::left("mac_stage_rail")
        .exact_size(86.0)
        .resizable(false)
        .frame(egui::Frame::NONE.fill(t.title_bar).inner_margin(egui::Margin::symmetric(8, 16)))
        .show(ui, |ui| {
            let author = if app.ui.author.is_empty() { "Local profile" } else { app.ui.author.as_str() };
            let (profile, response) = ui.allocate_exact_size(vec2(70.0, 76.0), Sense::hover());
            let centre = pos2(profile.center().x, profile.min.y + 22.0);
            ui.painter().circle_filled(centre, 19.0, t.accent);
            let initial = app.ui.author.chars().next().filter(|character| !character.is_whitespace()).unwrap_or('W').to_uppercase().to_string();
            ui.painter().text(centre, Align2::CENTER_CENTER, initial, medium(18.0), t.on_accent);
            row_text(ui, author, pos2(profile.min.x, profile.min.y + 49.0), profile.width(), regular(10.5), t.text_dim);
            response.on_hover_text(author);
            for (page, label, icon) in
                [("home", "Home", "home"), ("new", "New", "newCircle"), ("recent", "Recents", "history"), ("open", "Open", "folderOutline")]
            {
                let (r, response) = ui.allocate_exact_size(vec2(70.0, 70.0), Sense::click());
                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tl!(label)));
                if app.ui.backstage_page == page || response.hovered() {
                    ui.painter().rect_filled(r.shrink(2.0), 9.0, t.hover);
                }
                let icon = if page == "home" && app.ui.backstage_page == page { "homeFilled" } else { icon };
                icons::paint(
                    ui.painter(),
                    Rect::from_center_size(pos2(r.center().x, r.min.y + 25.0), vec2(28.0, 28.0)),
                    icon,
                    t.icon,
                    if icon == "homeFilled" { t.hover } else { t.accent },
                );
                ui.painter().text(pos2(r.center().x, r.min.y + 52.0), Align2::CENTER_CENTER, tl!(label), regular(12.0), t.text);
                if response.clicked() {
                    app.ui.backstage_page = page.into();
                }
                ui.add_space(10.0);
            }
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                ui.menu_button("•••", |ui| {
                    for (page, label) in PAGES.into_iter().skip(3) {
                        if ui.add_enabled(app.has_document() || works_without_document(page), egui::Button::new(tl!(label))).clicked() {
                            if page == "save" {
                                let _ = app.run("file.save", json!({}));
                            } else {
                                app.ui.backstage_page = page.into();
                            }
                            ui.close();
                        }
                    }
                    if ui.button("About").clicked() {
                        app.dialog = crate::dialogs::Dialog::open("about", app);
                        ui.close();
                    }
                });
            });
        });
    egui::Panel::bottom("mac_stage_footer")
        .exact_size(38.0)
        .frame(egui::Frame::NONE.fill(t.ribbon).inner_margin(egui::Margin::symmetric(16, 5)))
        .show(ui, |ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let is_new = app.ui.backstage_page == "new"
                    || (app.ui.backstage_page == "home" && ctx.data(|d| d.get_temp::<String>(egui::Id::new("mac_selected_recent"))).is_none());
                let selected =
                    ctx.data(|d| d.get_temp::<String>(egui::Id::new(if is_new { "mac_selected_template" } else { "mac_selected_recent" })));
                let action_page = matches!(app.ui.backstage_page.as_str(), "home" | "new" | "recent" | "open");
                if action_page
                    && ui
                        .add_enabled(
                            selected.is_some(),
                            egui::Button::new(egui::RichText::new(if is_new { "Create" } else { "Open" }).color(stage_ink(&t)))
                                .fill(stage_accent(&t)),
                        )
                        .clicked()
                    && let Some(value) = selected
                {
                    let result = if is_new { app.run("file.new", json!({"template":value})) } else { app.run("file.open", json!({"path":value})) };
                    if result.is_ok() {
                        app.ui.backstage = false;
                        app.canvas.want_focus = true;
                    }
                }
                if ui.button("Cancel").clicked() {
                    app.ui.backstage = false;
                    app.canvas.want_focus = true;
                }
            });
        });
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.stage_background).inner_margin(egui::Margin::symmetric(28, 18))).show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(ui.available_width());
            match app.ui.backstage_page.as_str() {
                "home" => {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Templates").font(semibold(15.0)));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.link("More templates").clicked() {
                                app.ui.backstage_page = "new".into();
                            }
                        });
                    });
                    ui.add_space(14.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(28.0, 18.0);
                        for (label, template) in TEMPLATES {
                            template_tile(ui, app, label, template);
                        }
                    });
                    ui.add_space(24.0);
                    ui.separator();
                    ui.add_space(12.0);
                    open_list(app, ui);
                }
                "recent" => open_list(app, ui),
                "new" => new_page(app, ui),
                "open" => open_page(app, ui),
                "saveAs" => save_as_page(app, ui),
                "info" => info_page(app, ui),
                "print" | "export" => export_page(app, ui),
                "options" => crate::options::page(app, ui),
                _ => {
                    if recent {
                        open_list(app, ui);
                    } else {
                        home_page(app, ui);
                    }
                }
            }
        });
    });
    if !app.file_dialog_open() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.ui.backstage = false;
        app.canvas.want_focus = true;
    }
}

/// Bound both lines of a recent row without letting long cloud paths paint into neighbours.
fn row_text(ui: &Ui, text: &str, position: egui::Pos2, width: f32, font: egui::FontId, colour: egui::Color32) {
    let mut job = egui::text::LayoutJob::simple(text.to_string(), font, colour, width.max(1.0));
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    ui.painter().galley(position, ui.painter().layout_job(job), colour);
}

/// The Backstage pages that work while no document is open (#1129).
fn works_without_document(page: &str) -> bool {
    matches!(page, "home" | "new" | "recent" | "open" | "options")
}

fn heading(ui: &mut Ui, s: &str) {
    ui.label(egui::RichText::new(tl!(s)).font(semibold(26.0)));
    ui.add_space(16.0);
}

fn template_tile(ui: &mut Ui, app: &mut WordApp, label: &str, template: &str) {
    let t = Tokens::get(ui.ctx());
    ui.vertical(|ui| {
        // The label sits right under the thumbnail whatever row gap the gallery uses.
        ui.spacing_mut().item_spacing.y = 0.0;
        let size = if app.integrated_titlebar { vec2(112.0, 148.0) } else { vec2(150.0, 194.0) };
        ui.set_width(size.x);
        let (r, resp) = ui.allocate_exact_size(size, Sense::click());
        let selected =
            app.integrated_titlebar && ui.ctx().data(|d| d.get_temp::<String>(egui::Id::new("mac_selected_template"))).as_deref() == Some(template);
        ui.painter().rect(
            r,
            if app.integrated_titlebar { 9.0 } else { 2.0 },
            egui::Color32::WHITE,
            Stroke::new(if selected { 2.0 } else { 0.75 }, if selected || resp.hovered() { stage_accent(&t) } else { t.border }),
            egui::StrokeKind::Inside,
        );
        let ppp = ui.ctx().pixels_per_point();
        let month = if template == "calendar" { wordcraft_engine::cmd::now_iso().get(..7).unwrap_or_default().to_string() } else { String::new() };
        let key = format!("tpl:{template}:{ppp}:{}:{month}", size.x);
        let region = app.session.region();
        let tex = ui.ctx().data(|d| d.get_temp::<egui::TextureHandle>(egui::Id::new(&key))).or_else(|| {
            let mut s = wordcraft_engine::Session::new(wordcraft_doc::Document::new());
            // The blank tile shows the region's paper (#838).
            s.system_region = region;
            let _ = s.run("file.new", &json!({"template": template}));
            let l = s.export_layout();
            let page = l.pages.first()?;
            let img = wordcraft_render::render_page(&s.doc, page, size.x / page.w * ppp, &crate::canvas::screen_render_options());
            let px = img.to_straight();
            let h = ui.ctx().load_texture(
                &key,
                egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &px),
                egui::TextureOptions::LINEAR,
            );
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(&key), h.clone()));
            Some(h)
        });
        if let Some(h) = tex {
            egui::Image::new((h.id(), r.size())).corner_radius(if app.integrated_titlebar { 8 } else { 1 }).paint_at(ui, r.shrink(2.0));
        }
        let caption = if selected {
            egui::Frame::NONE
                .fill(stage_accent(&t))
                .corner_radius(5)
                .inner_margin(egui::Margin::symmetric(4, 1))
                .show(ui, |ui| {
                    ui.add(egui::Label::new(egui::RichText::new(tl!(label)).font(regular(12.5)).color(stage_ink(&t))).wrap().sense(Sense::click()))
                })
                .inner
        } else {
            ui.add(egui::Label::new(egui::RichText::new(tl!(label)).font(regular(12.5))).wrap().sense(Sense::click()))
        };
        if app.integrated_titlebar && (resp.clicked() || caption.clicked()) {
            ui.ctx().data_mut(|d| {
                d.insert_temp(egui::Id::new("mac_selected_template"), template.to_string());
                d.remove::<String>(egui::Id::new("mac_selected_recent"));
            });
        }
        if resp.double_clicked() || caption.double_clicked() || (!app.integrated_titlebar && (resp.clicked() || caption.clicked())) {
            let _ = app.run("file.new", json!({"template": template}));
            app.ui.backstage = false;
            app.canvas.want_focus = true;
        }
    });
}

/// The built-in templates in gallery order: display label and the `file.new` template id.
const TEMPLATES: [(&str, &str); 7] = [
    ("Blank document", "blank"),
    ("Studio handbook (sample)", "sample"),
    ("Take notes", "notes"),
    ("Calendar", "calendar"),
    ("Letter", "letter"),
    ("Résumé", "resume"),
    ("Report", "report"),
];

/// How many templates Home shows before "More templates" (the rest are on New).
const HOME_TEMPLATES: usize = 4;

/// The templates whose English or translated label, or id, contains `query` (case-insensitive;
/// the id lets "resume" find "Résumé"); all of them for an empty query.
fn matching_templates(query: &str) -> Vec<(&'static str, &'static str)> {
    let q = query.trim().to_lowercase();
    TEMPLATES
        .into_iter()
        .filter(|(label, id)| q.is_empty() || id.contains(&q) || label.to_lowercase().contains(&q) || tl!(label).to_lowercase().contains(&q))
        .collect()
}

/// Home: a greeting, Blank plus a short row of templates with a link to New, then Recent.
fn home_page(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let hour = wordcraft_engine::cmd::now_iso().get(11..13).and_then(|h| h.parse::<u32>().ok()).unwrap_or(9);
    heading(
        ui,
        if hour < 12 {
            "Good morning"
        } else if hour < 18 {
            "Good afternoon"
        } else {
            "Good evening"
        },
    );
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(18.0, 0.0);
        for (label, template) in TEMPLATES.into_iter().filter(|(_, id)| !matches!(*id, "notes" | "calendar")).take(HOME_TEMPLATES) {
            template_tile(ui, app, label, template);
        }
    });
    ui.add_space(8.0);
    let more = ui.add(egui::Label::new(egui::RichText::new(tl!("More templates →")).font(medium(13.5)).color(t.accent)).sense(Sense::click()));
    if more.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        app.ui.backstage_page = "new".into();
    }
    ui.add_space(24.0);
    open_list(app, ui);
}

/// New: the whole template gallery with a search box; no Recent list (that is on Home and Open).
fn new_page(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    heading(ui, "New");
    let id = egui::Id::new("backstage_template_search");
    let mut query = ui.ctx().data(|d| d.get_temp::<String>(id)).unwrap_or_default();
    let search = ui.add(egui::TextEdit::singleline(&mut query).hint_text(tl!("Search for templates")).desired_width(360.0));
    if search.changed() {
        ui.ctx().data_mut(|d| d.insert_temp(id, query.clone()));
    }
    ui.add_space(18.0);
    let found = matching_templates(&query);
    if found.is_empty() {
        ui.label(egui::RichText::new(tl!("No templates match your search.")).color(t.text_dim));
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(18.0, 18.0);
        for (label, template) in found {
            template_tile(ui, app, label, template);
        }
    });
}

fn open_list(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let filter_id = egui::Id::new("mac_recent_favourites");
    let mut favourites = ui.ctx().data(|d| d.get_temp::<bool>(filter_id)).unwrap_or(false);
    if app.integrated_titlebar {
        ui.horizontal(|ui| {
            for (value, label) in [(false, "Recents"), (true, "Favourites")] {
                let active = favourites == value;
                if ui
                    .add(
                        egui::Button::new(egui::RichText::new(label).font(semibold(14.0)).color(if active {
                            egui::Color32::BLACK
                        } else {
                            t.text_dim
                        }))
                        .fill(if active { t.stage_tab } else { egui::Color32::TRANSPARENT })
                        .stroke(Stroke::NONE)
                        .corner_radius(6)
                        .min_size(vec2(96.0, 30.0)),
                    )
                    .clicked()
                {
                    favourites = value;
                }
            }
        });
        ui.ctx().data_mut(|d| d.insert_temp(filter_id, favourites));
    } else {
        ui.label(egui::RichText::new(tl!("Recent")).font(semibold(16.0)));
    }
    let query = ui.ctx().data(|data| data.get_temp::<String>(egui::Id::new("mac_document_search"))).unwrap_or_default().to_lowercase();
    let sort_id = egui::Id::new("recent_sort_newest");
    let mut newest = ui.ctx().data(|data| data.get_temp::<bool>(sort_id)).unwrap_or(true);
    if app.integrated_titlebar {
        ui.add_space(8.0);
        let (header, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
        ui.painter().rect_filled(header, 0.0, t.stage_header);
        row_text(ui, "Name", pos2(header.min.x + 44.0, header.min.y + 7.0), header.width() - 240.0, medium(12.0), t.text);
        let sort = ui.put(
            Rect::from_min_size(pos2(header.max.x - 174.0, header.min.y + 3.0), vec2(168.0, 24.0)),
            egui::Button::new(egui::RichText::new("Last opened by you  ▾").font(semibold(12.0))).frame(false),
        );
        egui::Popup::menu(&sort).show(|ui| {
            ui.selectable_value(&mut newest, true, "Newest first");
            ui.selectable_value(&mut newest, false, "Oldest first");
        });
        ui.ctx().data_mut(|data| data.insert_temp(sort_id, newest));
    }
    ui.add_space(6.0);
    let mut paths = if app.integrated_titlebar && favourites { app.ui.favourite_documents.clone() } else { app.ui.recent.clone() };
    paths.retain(|path| query.is_empty() || path.to_lowercase().contains(&query));
    paths.sort_by(|left, right| {
        let comparison = app.ui.recent_opened.get(left).copied().unwrap_or(0).cmp(&app.ui.recent_opened.get(right).copied().unwrap_or(0));
        if newest { comparison.reverse() } else { comparison }
    });
    if paths.is_empty() {
        ui.label(egui::RichText::new(tl!("Documents you open will show up here.")).color(t.text_dim));
    }
    let mut icon_budget = true;
    for (index, p) in paths.into_iter().take(app.ui.recent_shown()).enumerate() {
        let name = std::path::Path::new(&p).file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or(p.clone());
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), if app.integrated_titlebar { 56.0 } else { 40.0 }), Sense::click());
        let selected = ui.ctx().data(|d| d.get_temp::<String>(egui::Id::new("mac_selected_recent"))).as_deref() == Some(&p);
        if selected || resp.hovered() || (app.integrated_titlebar && index % 2 == 1) {
            ui.painter().rect_filled(
                r,
                7.0,
                if selected {
                    let accent = stage_accent(&t);
                    if t.dark { egui::Color32::from_rgb(accent.r() / 4 * 3, accent.g() / 4 * 3, accent.b() / 4 * 3) } else { accent }
                } else if resp.hovered() {
                    t.hover
                } else {
                    t.stage_alternate
                },
            );
        }
        let icon_key = egui::Id::new(("native-document-icon", &p));
        let cached = ui.ctx().data(|data| data.get_temp::<Option<egui::TextureHandle>>(icon_key));
        let texture = match cached {
            Some(texture) => texture,
            None if icon_budget => {
                icon_budget = false;
                let texture = app
                    .services
                    .document_icon
                    .as_ref()
                    .and_then(|hook| hook(&p))
                    .map(|image| ui.ctx().load_texture(format!("document-icon-{p}"), image, egui::TextureOptions::LINEAR));
                ui.ctx().data_mut(|data| data.insert_temp(icon_key, texture.clone()));
                ui.ctx().request_repaint();
                texture
            }
            None => None,
        };
        let icon_rect = Rect::from_center_size(pos2(r.min.x + 20.0, r.center().y), vec2(28.0, 32.0));
        if let Some(texture) = texture {
            ui.painter().image(texture.id(), icon_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
        } else {
            icons::paint(ui.painter(), icon_rect, "printLayout", t.accent, t.accent);
        }
        let text_width = if app.integrated_titlebar { r.width() - 284.0 } else { r.width() - 82.0 };
        let foreground = if selected { stage_ink(&t) } else { t.text };
        let secondary = if selected {
            stage_ink(&t).gamma_multiply(0.8)
        } else if t.dark {
            t.text_dim.gamma_multiply(0.75)
        } else {
            t.text_dim
        };
        row_text(ui, &name, pos2(r.min.x + 44.0, r.min.y + 9.0), text_width, regular(14.0), foreground);
        let breadcrumb = std::path::Path::new(&p)
            .parent()
            .map(|parent| parent.components().map(|part| part.as_os_str().to_string_lossy()).collect::<Vec<_>>().join(" › "))
            .unwrap_or_default();
        row_text(ui, &breadcrumb, pos2(r.min.x + 44.0, r.min.y + 31.0), text_width, regular(11.5), secondary);
        let time_rect = Rect::from_min_size(pos2(r.max.x - 150.0, r.min.y), vec2(144.0, r.height()));
        if app.integrated_titlebar {
            let timestamp = app.ui.recent_opened.get(&p).copied().unwrap_or(0);
            ui.painter().text(
                pos2(time_rect.min.x + 8.0, time_rect.center().y),
                Align2::LEFT_CENTER,
                opened_label(timestamp, wordcraft_engine::cmd::now_unix()),
                regular(13.0),
                foreground,
            );
        }
        let star_rect = Rect::from_center_size(pos2(r.max.x - 206.0, r.center().y), vec2(28.0, 28.0));
        let star = ui.interact(star_rect, ui.id().with(("favourite", &p)), Sense::click());
        if app.integrated_titlebar {
            let fav = app.ui.favourite_documents.contains(&p);
            ui.painter().text(
                star_rect.center(),
                Align2::CENTER_CENTER,
                if fav { "★" } else { "☆" },
                regular(18.0),
                if selected {
                    foreground
                } else if fav {
                    t.accent
                } else {
                    t.text_dim
                },
            );
            if star.on_hover_text(if fav { "Remove favourite" } else { "Add favourite" }).clicked() {
                let _ = app.run("ui.favoriteDocument", json!({"path":p}));
            }
            let more_rect = Rect::from_center_size(pos2(r.max.x - 172.0, r.center().y), vec2(28.0, 28.0));
            let more = ui.put(more_rect, egui::Button::new(egui::RichText::new("•••").color(foreground)).frame(false));
            egui::Popup::menu(&more).show(|ui| {
                if ui.button("Open").clicked() {
                    let _ = app.run("file.open", json!({"path":p}));
                    ui.close();
                }
                if ui.button("Copy path").clicked() {
                    ui.ctx().copy_text(p.clone());
                    ui.close();
                }
                if ui.button("Remove from Recent").clicked() {
                    let _ = app.run("ui.removeRecent", json!({"path":p}));
                    ui.close();
                }
            });
        }
        let row_action = !app.integrated_titlebar || ui.input(|i| i.pointer.interact_pos()).is_some_and(|point| point.x < r.max.x - 226.0);
        if app.integrated_titlebar && resp.clicked() && row_action {
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("mac_selected_recent"), p.clone()));
        }
        if ((resp.double_clicked() && row_action) || (!app.integrated_titlebar && resp.clicked())) && app.run("file.open", json!({"path": p})).is_ok()
        {
            app.ui.backstage = false;
            app.canvas.want_focus = true;
        }
        resp.on_hover_text(&p);
        // Positioned row actions must not move the next row above this row's bottom edge.
        ui.advance_cursor_after_rect(r);
    }
}

pub(crate) fn stage_accent(tokens: &Tokens) -> egui::Color32 {
    #[cfg(target_os = "macos")]
    {
        static COLOUR: std::sync::OnceLock<Option<egui::Color32>> = std::sync::OnceLock::new();
        if let Some(colour) = COLOUR.get_or_init(|| {
            let output = std::process::Command::new("/usr/bin/defaults").args(["read", "-g", "AppleAccentColor"]).output().ok()?;
            if !output.status.success() {
                return None;
            }
            let value = std::str::from_utf8(&output.stdout).ok()?.trim().parse::<i32>().ok()?;
            let rgb = match value {
                -1 => [142, 142, 147],
                0 => [255, 69, 58],
                1 => [255, 159, 10],
                2 => [255, 214, 10],
                3 => [48, 209, 88],
                4 => [10, 132, 255],
                5 => [191, 90, 242],
                6 => [255, 55, 155],
                _ => return None,
            };
            Some(egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]))
        }) {
            return *colour;
        }
    }
    tokens.accent
}

fn stage_ink(tokens: &Tokens) -> egui::Color32 {
    if stage_accent(tokens) == tokens.accent { tokens.on_accent } else { egui::Color32::WHITE }
}

fn opened_label(timestamp: u64, now: u64) -> String {
    if timestamp == 0 {
        return "—".into();
    }
    let elapsed = now.saturating_sub(timestamp);
    if elapsed < 60 {
        return "Just now".into();
    }
    if elapsed < 3600 {
        return format!("{}m ago", elapsed / 60);
    }
    if elapsed < 86400 {
        return format!("{}h ago", elapsed / 3600);
    }
    let date = wordcraft_engine::cmd::iso_from_unix_secs(timestamp.min(253_402_300_799));
    let month = date
        .get(5..7)
        .and_then(|value| value.parse::<usize>().ok())
        .and_then(|value| ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"].get(value.saturating_sub(1)))
        .copied()
        .unwrap_or("");
    let day = date.get(8..10).and_then(|value| value.parse::<u32>().ok()).unwrap_or(1);
    let current = wordcraft_engine::cmd::iso_from_unix_secs(now.min(253_402_300_799));
    if date.get(..4) != current.get(..4) { format!("{day} {month} {}", date.get(..4).unwrap_or("")) } else { format!("{day} {month}") }
}

fn open_page(app: &mut WordApp, ui: &mut Ui) {
    heading(ui, "Open");
    if ui.button(egui::RichText::new(tl!("Browse…")).font(medium(14.0))).clicked() {
        let _ = app.run("ui.openFileDialog", json!({}));
    }
    // Linux and the BSDs (#576): the system dialog needs a file chooser portal or zenity; when it
    // doesn't show up, WordCraft's own browser is always one click away.
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    if ui
        .button(egui::RichText::new(tl!("Browse in WordCraft…")).font(medium(14.0)))
        .on_hover_text(tl!("Open a file with WordCraft's own browser when the system file dialog doesn't appear."))
        .clicked()
    {
        let _ = app.run("ui.fileBrowser", json!({}));
    }
    ui.add_space(18.0);
    open_list(app, ui);
}

/// Save As: Browse… asks where to save. It has no recent-documents list: clicking one of those
/// opens it, which is not what Save As is for.
fn save_as_page(app: &mut WordApp, ui: &mut Ui) {
    heading(ui, "Save As");
    if ui.button(egui::RichText::new(tl!("Browse…")).font(medium(14.0))).clicked() {
        app.save_as_dialog();
    }
}

fn info_page(app: &mut WordApp, ui: &mut Ui) {
    heading(ui, "Info");
    let info = app.session.run("file.info", &json!({})).unwrap_or_default();
    ui.columns(2, |cols| {
        let ui = &mut cols[0];
        ui.label(egui::RichText::new(tl!("Properties")).font(semibold(15.0)));
        // The fields edit a copy of the document's properties that is read afresh every frame,
        // so each change goes into the document as it is typed (#260): waiting for the field to
        // lose focus dropped the keystrokes, and leaving File with Escape never loses it at all.
        // The keystrokes of one visit to a field are a single undo step.
        let mut props = app.session.doc.core.clone();
        let mut edited = None;
        let mut focused = None;
        egui::Grid::new("props").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
            for (l, v) in [("Title", &mut props.title), ("Subject", &mut props.subject), ("Author", &mut props.creator), ("Keywords", &mut props.keywords), ("Category", &mut props.category)] {
                let label = ui.label(tl!(l));
                let r = ui.add(egui::TextEdit::singleline(v).id_salt(("info-prop", l))).labelled_by(label.id);
                if r.changed() {
                    edited = Some(l);
                }
                if r.has_focus() {
                    focused = Some(l);
                }
                ui.end_row();
            }
        });
        if let Some(field) = edited {
            if app.info_editing == Some((field, app.session.rev())) {
                app.session.join_next_undo();
            }
            let ok = app.run("file.properties", json!({"title": props.title, "subject": props.subject, "author": props.creator, "keywords": props.keywords, "category": props.category})).is_ok();
            app.info_editing = ok.then(|| (field, app.session.rev()));
        }
        if focused != app.info_editing.map(|(f, _)| f) {
            app.info_editing = None;
        }
        // Protect Document › Encrypt with Password (#55).
        ui.add_space(18.0);
        ui.label(egui::RichText::new(tl!("Protect Document")).font(semibold(15.0)));
        let encrypted = info.get("encrypted").and_then(|v| v.as_bool()).unwrap_or(false);
        ui.label(if encrypted {
            tl!("A password is required to open this document.")
        } else {
            tl!("Anyone can open this document. Encrypt it with a password to keep it private.")
        });
        ui.horizontal(|ui| {
            let label = if encrypted { tl!("Change Password…") } else { tl!("Encrypt with Password…") };
            if ui.button(egui::RichText::new(label).font(medium(13.5))).clicked() {
                let _ = app.run("file.encrypt", json!({}));
            }
            if encrypted && ui.button(egui::RichText::new(tl!("Remove Password")).font(medium(13.5))).clicked() {
                let _ = app.run("file.encrypt", json!({"password": null}));
            }
        });
        let ui = &mut cols[1];
        ui.label(egui::RichText::new(tl!("Statistics")).font(semibold(15.0)));
        for (l, k) in [("Pages", "pages"), ("Words", "words"), ("Paragraphs", "paragraphs"), ("Sections", "sections"), ("Comments", "comments")] {
            ui.label(format!("{}: {}", tl!(l), info.get(k).map(|v| v.to_string()).unwrap_or_default()));
        }
        let path = info.get("path").and_then(|v| v.as_str()).unwrap_or(tl!("Not saved yet"));
        ui.label(crate::i18n::fmt(tl!("Location: {path}"), &[("path", path)]));
    });
}

fn export_page(app: &mut WordApp, ui: &mut Ui) {
    if app.ui.backstage_page == "print" {
        heading(ui, "Print");
        // Print Markup (a print setting, as in Word): the comments beside each page.
        let mut markup = app.session.prefs.print_markup;
        if ui
            .checkbox(&mut markup, tl!("Print Markup"))
            .on_hover_text(tl!(
                "Comments beside each page and tracked changes in PDFs, prints and images; a comment list at the end of web pages, Markdown and text."
            ))
            .changed()
        {
            let _ = app.run("file.printMarkup", json!({"value": markup}));
        }
        ui.add_space(12.0);
        crate::print_page::show(app, ui);
        return;
    }
    heading(ui, "Export");
    ui.label(tl!("Save a copy in another format. Printing goes through a PDF you can print from any viewer."));
    ui.add_space(12.0);
    crate::print_page::export_formats(app, ui);
}

#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable;
    use serde_json::json;

    use super::matching_templates;
    use crate::{Services, WordApp};

    #[test]
    fn template_search_matches_labels_case_insensitively() {
        let ids = |q: &str| matching_templates(q).into_iter().map(|(_, id)| id).collect::<Vec<_>>();
        assert_eq!(ids(""), ["blank", "sample", "notes", "calendar", "letter", "resume", "report"]);
        assert_eq!(ids("  LETTER "), ["letter"]);
        assert_eq!(ids("re"), ["resume", "report"]);
        assert_eq!(ids("résumé"), ["resume"]);
        assert!(ids("no such template").is_empty());
    }

    /// Home and New are different pages (#141): Home has the short template row, the link to New
    /// and Recent; New has every template and no Recent list.
    #[test]
    fn home_and_new_show_different_sections() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default());
        a.run("ui.backstage", json!({"value": true, "page": "home"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        assert!(h.query_by_label("Recent").is_some(), "Home lists recent documents");
        assert!(h.query_by_label("Letter").is_some(), "Home shows the first templates");
        assert!(h.query_by_label("Report").is_none(), "Home shows only a short row of templates");
        h.get_by_label("More templates →").click();
        h.run_steps(4);
        assert_eq!(h.state().ui.backstage_page, "new", "the link opens New");
        assert!(h.query_by_label("Report").is_some(), "New shows every template");
        assert!(h.query_by_label("Recent").is_none(), "New has no Recent list");
    }

    /// #260: File › Info's Title and Author fields dropped every keystroke (they edited a copy
    /// re-read each frame and only wrote it back when the field lost focus), so a save had no
    /// title or author. Typing now lands in the document as it happens, one undo step per field,
    /// and the browser's Save downloads it with the save stamps advanced (#262).
    #[test]
    fn info_fields_keep_typed_properties_through_a_web_save() {
        use egui::accesskit::Role;
        let got: std::rc::Rc<std::cell::RefCell<Vec<(String, Vec<u8>)>>> = Default::default();
        let sink = got.clone();
        let services = Services {
            download: Some(Box::new(move |n: &str, b: &[u8]| {
                sink.borrow_mut().push((n.to_string(), b.to_vec()));
                Ok(())
            })),
            ..Default::default()
        };
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("XYZ")), services);
        a.session.author = "GAMMA".into();
        a.run("ui.backstage", json!({"value": true, "page": "info"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        for (field, text) in [("Title", "ALPHA"), ("Author", "BETA")] {
            h.get_by_role_and_label(Role::TextInput, field).click();
            h.run_steps(2);
            for c in text.chars() {
                h.get_by_role_and_label(Role::TextInput, field).type_text(&c.to_string());
                h.run_steps(2);
            }
            assert_eq!(h.get_by_role_and_label(Role::TextInput, field).value().as_deref(), Some(text), "{field} shows what was typed");
        }
        let core = h.state().session.doc.core.clone();
        assert_eq!((core.title.as_str(), core.creator.as_str()), ("ALPHA", "BETA"));
        assert!(h.state().session.dirty, "the property edits are unsaved changes");

        // Save (web: a download) writes them into docProps/core.xml, with fresh save stamps.
        let a = h.state_mut();
        let created = a.session.doc.core.created.clone();
        a.run("file.save", json!({})).unwrap();
        a.run("file.save", json!({})).unwrap();
        let (name, bytes) = got.borrow_mut().pop().unwrap();
        let back = wordcraft_engine::io::open_bytes(&name, &bytes).unwrap();
        assert_eq!((back.core.title.as_str(), back.core.creator.as_str()), ("ALPHA", "BETA"));
        assert_eq!(back.core.last_modified_by, "GAMMA");
        assert_eq!(back.core.revision, core.revision + 2, "each save advances the revision");
        assert!(created.is_empty() && !back.core.created.is_empty(), "the first save sets the creation time");
        assert!(!back.core.modified.is_empty());
        assert!(!a.session.dirty);

        // Each field's typing is one undo step.
        a.session.run("edit.undo", &json!({})).unwrap();
        assert_eq!((a.session.doc.core.title.as_str(), a.session.doc.core.creator.as_str()), ("ALPHA", ""));
        a.session.run("edit.undo", &json!({})).unwrap();
        assert_eq!(a.session.doc.core.title, "");
    }

    #[test]
    fn save_as_page_shows_browse_and_no_open_list() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default());
        a.run("ui.backstage", json!({"value": true, "page": "saveAs"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        assert!(h.query_by_label("Browse…").is_some(), "Save As has Browse button");
        assert!(h.query_by_label("Recent").is_none(), "Save As has no list that opens documents");
    }

    #[test]
    fn backstage_save_navigates_to_save_as_when_unnamed() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default());
        a.run("ui.backstage", json!({"value": true, "page": "home"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        h.get_by_label("Save").click();
        h.run_steps(4);
        assert_eq!(h.state().ui.backstage_page, "saveAs", "unnamed Save opens Save As page");
        assert!(h.state().ui.backstage, "backstage remains open");
    }

    /// Save on a document without a file shows the Save As page only: no native dialog as well.
    #[test]
    fn backstage_save_when_unnamed_opens_no_file_dialog() {
        let shown = std::rc::Rc::new(std::cell::Cell::new(0));
        let count = shown.clone();
        let services = Services {
            file_dialog: Some(Box::new(move |_| {
                count.set(count.get() + 1);
                std::sync::mpsc::channel::<crate::FileDialogAnswer>().1
            })),
            ..Default::default()
        };
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), services);
        a.run("ui.backstage", json!({"value": true, "page": "home"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        h.get_by_label("Save").click();
        h.run_steps(4);
        assert_eq!(h.state().ui.backstage_page, "saveAs");
        assert_eq!(shown.get(), 0, "no native Save As dialog");
        assert!(!h.state().file_dialog_open());
    }

    #[test]
    fn backstage_save_closes_when_saved() {
        let got: std::rc::Rc<std::cell::RefCell<Vec<(String, Vec<u8>)>>> = Default::default();
        let sink = got.clone();
        let services = Services {
            download: Some(Box::new(move |n: &str, b: &[u8]| {
                sink.borrow_mut().push((n.to_string(), b.to_vec()));
                Ok(())
            })),
            ..Default::default()
        };
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), services);
        a.run("ui.backstage", json!({"value": true, "page": "home"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        h.get_by_label("Save").click();
        h.run_steps(4);
        assert!(!h.state().ui.backstage, "successful save closes backstage");
    }

    /// #1124: the side menu had no Close. It sits after Export and closes the document, asking
    /// first when there are unsaved changes; Cancel leaves the Backstage as it was.
    #[test]
    fn backstage_close_asks_then_closes() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default());
        a.run("text.insert", json!({"text": "Draft"})).unwrap();
        a.run("ui.backstage", json!({"value": true, "page": "info"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        let export = h.get_by_label("Export").rect().min.y;
        assert!(h.get_by_label("Close").rect().min.y > export, "Close comes after Export");
        h.get_by_label("Close").click();
        h.run_steps(4);
        assert!(matches!(h.state().dialog, Some(crate::dialogs::Dialog::SaveChanges { .. })), "unsaved changes ask first");
        assert!(!h.state().quit_requested);

        h.state_mut().run("ui.saveChanges", json!({"answer": "cancel"})).unwrap();
        h.run_steps(4);
        assert!(!h.state().quit_requested, "Cancel keeps the document open");
        assert!(h.state().ui.backstage && h.state().ui.backstage_page == "info", "Cancel leaves the Backstage as it was");

        h.get_by_label("Close").click();
        h.run_steps(4);
        h.state_mut().run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
        assert!(h.state().quit_requested, "Don't Save closes");
    }

    /// A document without unsaved changes closes without a prompt.
    #[test]
    fn backstage_close_on_a_clean_document_closes_at_once() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default());
        a.run("ui.backstage", json!({"value": true, "page": "home"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        h.get_by_label("Close").click();
        h.run_steps(1);
        assert!(h.state().dialog.is_none(), "no prompt");
        assert!(h.state().quit_requested);
    }

    #[test]
    fn backstage_save_as_sidebar_opens_save_as_page() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default());
        a.run("ui.backstage", json!({"value": true, "page": "home"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        h.get_by_label("Save As").click();
        h.run_steps(4);
        assert_eq!(h.state().ui.backstage_page, "saveAs");
    }
}

#[cfg(test)]
mod mac_stage_tests {
    use super::*;
    use egui_kittest::kittest::Queryable;
    #[test]
    fn opening_times_persist_and_edits_do_not_change_them() {
        let file = std::env::temp_dir().join(format!("wordcraft-recent-time-{}.txt", std::process::id()));
        std::fs::write(&file, "Synthetic document").unwrap();
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        app.execute("file.open", json!({"path":file})).unwrap();
        let path = app.session.path.as_ref().unwrap().to_string_lossy().into_owned();
        let opened = app.ui.recent_opened[&path];
        assert!(opened > 0);
        app.execute("text.insert", json!({"text":"Edit"})).unwrap();
        assert_eq!(app.ui.recent_opened[&path], opened);
        let bytes = serde_json::to_vec(&app.prefs()).unwrap();
        assert_eq!(crate::UiState::from_json(&bytes).unwrap().recent_opened[&path], opened);
        app.execute("ui.removeRecent", json!({"path":path})).unwrap();
        assert!(app.ui.recent_opened.is_empty());
        assert!(file.exists());
        std::fs::remove_file(file).unwrap();
        assert_eq!(opened_label(0, opened), "—");
        assert_eq!(opened_label(opened - 6 * 60, opened), "6m ago");
        assert_eq!(opened_label(u64::MAX, opened), "Just now");
    }
    #[test]
    fn mac_start_screen_navigation_and_default_create_work() {
        for width in [640.0, 1440.0] {
            let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
            app.integrated_titlebar = true;
            app.start_screen();
            let mut h = egui_kittest::Harness::builder().with_size(vec2(width, 900.0)).build_ui_state(
                |ui, app: &mut WordApp| {
                    let ctx = ui.ctx().clone();
                    app.logic(&ctx);
                    app.ui(ui);
                },
                app,
            );
            h.run_steps(4);
            assert!(h.query_by_label("Favourites").is_some());
            assert!(h.query_by_label("Good morning").is_none());
            h.get_by_label("New").click();
            h.run_steps(4);
            assert_eq!(h.state().ui.backstage_page, "new");
            h.get_by_label("Create").click();
            h.run_steps(4);
            assert!(!h.state().ui.backstage);
        }
    }
    #[test]
    fn favourites_persist_and_reject_hostile_paths_without_document_changes() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Keep writing")), crate::Services::default());
        let doc = app.session.doc.clone();
        let params = json!({"path":"/project/Papér.docx"});
        app.run("ui.favoriteDocument", params.clone()).unwrap();
        assert_eq!(app.ui.favourite_documents.len(), 1);
        let prefs = serde_json::to_string(&app.ui).unwrap();
        let restored: crate::UiState = serde_json::from_str(&prefs).unwrap();
        assert_eq!(restored.favourite_documents, app.ui.favourite_documents);
        for path in ["".to_string(), "a".repeat(4097)] {
            assert!(app.run("ui.favoriteDocument", json!({"path":path})).is_err());
        }
        app.run("ui.favoriteDocument", params).unwrap();
        assert!(app.ui.favourite_documents.is_empty());
        assert_eq!(app.session.doc, doc);
    }
}

#[cfg(test)]
mod merge_integration_tests {
    use super::*;
    use egui_kittest::kittest::Queryable;
    #[test]
    fn mac_template_caption_selects_then_create_opens_and_recent_survives_empty_window() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        app.integrated_titlebar = true;
        app.start_screen();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        h.run_steps(6);
        h.get_by_label("Letter").click();
        h.run_steps(3);
        assert!(!h.state().has_document());
        assert_eq!(h.state().ctx.as_ref().unwrap().data(|d| d.get_temp::<String>(egui::Id::new("mac_selected_template"))), Some("letter".into()));
        h.get_by_label("Create").click();
        h.run_steps(3);
        assert!(h.state().has_document());
        assert!(!h.state().ui.backstage);
        h.state_mut().start_screen();
        h.state_mut().ui.backstage_page = "recent".into();
        h.run_steps(3);
        assert_eq!(h.state().ui.backstage_page, "recent");
    }
}

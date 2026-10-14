//! File tab (Backstage): Home/New, Open, Info, Save As, Print, Export, Close, Options, About.

use egui::{Align2, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;

use crate::theme::{APP_COLOR, Tokens, medium, regular, semibold};
use crate::{WordApp, icons};

/// WordCraft's icon for an entry of the File page's column; `None` is Back.
fn nav_icon(key: Option<&str>) -> &'static str {
    match key {
        None => "chevronLeft",
        Some("home") => "home",
        Some("new") => "blankPage",
        Some("open") => "open",
        Some("save" | "saveAs") => "save",
        Some("print") => "print",
        Some("export") => "export",
        Some("close") => "close",
        Some("options") => "options",
        Some("discord") => "comment",
        Some(_) => "info",
    }
}

pub fn show(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    // No document open (#1129): only the pages that open or create one, and Options.
    let has_document = app.has_document();
    if !has_document && !works_without_document(&app.ui.backstage_page) {
        app.ui.backstage_page = "home".into();
    }
    let mac = app.integrated_titlebar;
    // The column is the shared one (storytold/craft-libs#53): a labelled Back, the pages in groups,
    // Options and About at the foot. WordCraft supplies the entries, their wording and icons.
    egui::Panel::left("backstage_nav")
        .exact_size(200.0)
        // Fixed width, as in Word: without this egui shows a resize handle that does nothing (#502).
        .resizable(false)
        .frame(egui::Frame::NONE.fill(APP_COLOR))
        .show(ui, |ui| {
            use craft_ui::file_nav::{FileNav, FileNavAction, FileNavItem, FileNavStyle};
            let style =
                FileNavStyle { font: medium(14.0), selected_font: semibold(14.0), foot_font: medium(13.0), ..FileNavStyle::on_accent(APP_COLOR) };
            // "Home" is also the ribbon tab beside File: here it has its own wording.
            let item = |key: &'static str, label: &'static str| FileNavItem {
                key,
                label: if key == "home" { crate::i18n::tc("file", label) } else { tl!(label) },
            };
            let top = [item("home", "Home"), item("new", "New"), item("open", "Open")];
            let rest = [
                item("info", "Info"),
                item("save", "Save"),
                item("saveAs", "Save As"),
                item("print", "Print"),
                item("export", "Export"),
                item("close", "Close"),
            ];
            // No document open (#1129): the pages that need one are left out of the column.
            let rest: &[FileNavItem<&str>] = if has_document { &rest } else { &[] };
            let foot = [item("options", "Options"), item("discord", "Discord community"), item("about", "About")];
            let paint = |key: Option<&str>, p: &egui::Painter, r: Rect, c: egui::Color32| icons::paint(p, r, nav_icon(key), c, c);
            // macOS (#222): the File page covers the title bar, so the column's top band is the
            // title bar. It holds only the window buttons and drags the window.
            let band = if mac { crate::chrome::MAC_TITLE_BAR } else { 0.0 };
            let showing = app.ui.backstage_page.clone();
            let action =
                FileNav::new(tl!("Back"), &[&top, rest], Some(showing.as_str()), &style, &paint).foot(&foot).title_band(band).show(ui).action;
            // Escape in the built-in file browser closes only the browser.
            let escape = !app.file_dialog_open() && ui.input(|i| i.key_pressed(egui::Key::Escape));
            match action {
                Some(FileNavAction::DragWindow) => ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag),
                // A document without a file goes to the Save As page (as in Word) instead of
                // `file.save`, which would also open the native Save As dialog. In the browser a
                // save is a download, so it saves right away.
                Some(FileNavAction::Choose("save")) if app.session.path.is_none() && app.services.download.is_none() => {
                    app.ui.backstage_page = "saveAs".into();
                }
                Some(FileNavAction::Choose("save")) => {
                    let r = app.run("file.save", json!({}));
                    if r.is_ok_and(|v| v.get("saved").and_then(serde_json::Value::as_bool) == Some(true) || v.get("downloaded").is_some()) {
                        app.ui.backstage = false;
                        app.canvas.want_focus = true;
                    } else {
                        app.ui.backstage_page = "saveAs".into();
                    }
                }
                Some(FileNavAction::Choose("discord")) => {
                    let _ = app.run("ui.discord", json!({}));
                }
                // Not a page: `file.close` asks Save / Don't Save / Cancel first when there are
                // unsaved changes, and Cancel leaves the Backstage where it was.
                Some(FileNavAction::Choose("close")) => {
                    let _ = app.run("file.close", json!({}));
                }
                Some(FileNavAction::Choose("about")) => app.dialog = crate::dialogs::Dialog::open("about", app),
                Some(FileNavAction::Choose(page)) => app.ui.backstage_page = page.into(),
                Some(FileNavAction::Back) | None => {}
            }
            if action == Some(FileNavAction::Back) || escape {
                app.ui.backstage = false;
                app.canvas.want_focus = true;
            }
        });
    // Open is the places panel from edge to edge, with its own scrolling: the navigation column
    // already says "Open", so the page needs no heading of its own.
    let open = app.ui.backstage_page == "open";
    let margin = if open { egui::Margin::ZERO } else { egui::Margin::symmetric(40, 30) };
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.ribbon).inner_margin(margin)).show(ui, |ui| {
        if open {
            open_page(app, ui);
            return;
        }
        egui::ScrollArea::vertical().show(ui, |ui| match app.ui.backstage_page.as_str() {
            "home" => home_page(app, ui),
            "new" => new_page(app, ui),
            "saveAs" => save_as_page(app, ui),
            "info" => info_page(app, ui),
            "print" | "export" => export_page(app, ui),
            "options" => crate::options::page(app, ui),
            _ => home_page(app, ui),
        });
    });
}

/// The Backstage pages that work while no document is open (#1129).
fn works_without_document(page: &str) -> bool {
    matches!(page, "home" | "new" | "open" | "options")
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
        let (r, resp) = ui.allocate_exact_size(vec2(150.0, 194.0), Sense::click());
        ui.painter().rect(
            r,
            2.0,
            egui::Color32::WHITE,
            Stroke::new(if resp.hovered() { 2.0 } else { 1.0 }, if resp.hovered() { t.accent } else { t.border_strong }),
            egui::StrokeKind::Inside,
        );
        let ppp = ui.ctx().pixels_per_point();
        let key = format!("tpl:{template}:{ppp}");
        let region = app.session.region();
        let tex = ui.ctx().data(|d| d.get_temp::<egui::TextureHandle>(egui::Id::new(&key))).or_else(|| {
            let mut s = wordcraft_engine::Session::new(wordcraft_doc::Document::new());
            // The blank tile shows the region's paper (#838).
            s.system_region = region;
            let _ = s.run("file.new", &json!({"template": template}));
            let l = s.export_layout();
            let page = l.pages.first()?;
            let img = wordcraft_render::render_page(&s.doc, page, 150.0 / page.w * ppp, &crate::canvas::screen_render_options());
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
            ui.painter().image(h.id(), r.shrink(1.0), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
        // The caption opens the template too, as in Word.
        let caption = ui.add(egui::Label::new(egui::RichText::new(tl!(label)).font(regular(12.5))).sense(Sense::click()));
        if resp.clicked() || caption.clicked() {
            let _ = app.run("file.new", json!({"template": template}));
            app.ui.backstage = false;
            app.canvas.want_focus = true;
        }
    });
}

/// The built-in templates in gallery order: display label and the `file.new` template id.
const TEMPLATES: [(&str, &str); 5] =
    [("Blank document", "blank"), ("Studio handbook (sample)", "sample"), ("Letter", "letter"), ("Résumé", "resume"), ("Report", "report")];

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
        for (label, template) in TEMPLATES.into_iter().take(HOME_TEMPLATES) {
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
    ui.label(egui::RichText::new(tl!("Recent")).font(semibold(16.0)));
    ui.add_space(6.0);
    if app.ui.recent.is_empty() {
        ui.label(egui::RichText::new(tl!("Documents you open will show up here.")).color(t.text_dim));
    }
    for p in app.ui.recent.iter().take(app.ui.recent_shown()).cloned().collect::<Vec<_>>() {
        let name = std::path::Path::new(&p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or(p.clone());
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width().min(700.0), 40.0), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(r, 4.0, t.hover);
        }
        icons::paint(ui.painter(), Rect::from_center_size(pos2(r.min.x + 18.0, r.center().y), vec2(22.0, 22.0)), "printLayout", t.icon, t.accent);
        ui.painter().text(pos2(r.min.x + 40.0, r.min.y + 13.0), Align2::LEFT_CENTER, &name, medium(13.0), t.text);
        ui.painter().text(pos2(r.min.x + 40.0, r.min.y + 29.0), Align2::LEFT_CENTER, &p, regular(11.0), t.text_dim);
        if resp.clicked() {
            let _ = app.run("file.open", json!({"path": p}));
            app.ui.backstage = false;
            app.canvas.want_focus = true;
        }
    }
}

/// File › Open: the shared places panel (storytold/craft-libs#49). Recent lists the documents
/// opened lately; This computer lists folders and documents right here, starting from the home
/// folder, through craft-ui's `LocalFolder`; Browse shows the system's dialog. Remote storage adds
/// one place per account; there is none yet.
fn open_page(app: &mut WordApp, ui: &mut Ui) {
    use craft_ui::places::local::{LocalFolder, LocalLabels, LocalOutcome};
    use craft_ui::places::{
        Place, PlaceEntry, PlaceKind, PlacesAction, PlacesContent, PlacesIcon, PlacesLabels, PlacesNotice, PlacesPanel, PlacesStyle,
    };
    let t = Tokens::get(ui.ctx());
    let style = PlacesStyle {
        fill: t.ribbon,
        border: t.border,
        text: t.text,
        weak: t.text_dim,
        hover: t.hover,
        selected: t.checked,
        accent: t.accent,
        icon: t.icon,
        // The File page's own scale: its navigation is set in 14 pt on 38 pt rows.
        rail_width: 250.0,
        row_height: 36.0,
        font: 14.0,
        small_font: 12.0,
        ..PlacesStyle::from_visuals(ui.visuals())
    };
    let labels = PlacesLabels { places: tl!("Places"), add_place: tl!("Add a place"), browse: tl!("Browse…"), loading: tl!("Loading…") };
    let mut places = vec![Place { kind: PlaceKind::Recent, label: tl!("Recent"), detail: None }];
    // The browser has no folders to list, so there the system's picker is the only way in.
    if cfg!(not(target_arch = "wasm32")) {
        places.push(Place { kind: PlaceKind::Local, label: tl!("This computer"), detail: None });
    }

    // A folder the system keeps closed (macOS guards Documents, Desktop and Downloads) still opens
    // through Browse: a document the user picks there is theirs to open. macOS asks once whether
    // the app may read such a folder, and stays silent after a No.
    let no_access = tl!("WordCraft has no permission to read this folder. Use Browse to open a document from it.");
    let denied = if cfg!(target_os = "macos") {
        format!("{no_access} {}", tl!("To list it here, allow WordCraft in System Settings, under Privacy & Security, Files and Folders."))
    } else {
        no_access.to_string()
    };
    let words = LocalLabels {
        empty: tl!("This folder is empty."),
        no_access: tl!("No access to this folder"),
        no_access_detail: &denied,
        browse: tl!("Browse…"),
        gone: tl!("This folder is no longer there."),
        gone_detail: tl!("It may have been moved or deleted."),
        unreadable: tl!("This folder can't be read."),
    };

    // This computer is showing a folder: the shared helper lists it and says what a choice means.
    if let Some(folder) = app.file_browser.page.as_mut() {
        let entries = folder.entries();
        let crumbs = folder.crumbs(tl!("This computer"));
        let crumbs: Vec<&str> = crumbs.iter().map(String::as_str).collect();
        let action = PlacesPanel::new(&places, Some(1), folder.content(&entries, words), &style, labels).crumbs(&crumbs).show(ui).action;
        match action {
            Some(PlacesAction::Select(0)) => app.file_browser.page = None,
            Some(action) => match folder.act(action) {
                LocalOutcome::Open(path) => {
                    let _ = app.run("file.open", json!({"path": path.to_string_lossy()}));
                    app.ui.backstage = false;
                    app.canvas.want_focus = true;
                }
                LocalOutcome::Browse => {
                    let _ = app.run("ui.openFileDialog", json!({}));
                }
                LocalOutcome::Stay => {}
            },
            None => {}
        }
        return;
    }

    // Recent: the documents opened lately, each with its folder.
    let recent: Vec<(String, String, String)> = app
        .ui
        .recent
        .iter()
        .take(app.ui.recent_shown())
        .map(|p| {
            let path = std::path::Path::new(p);
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.clone());
            let folder = path.parent().map(|d| d.to_string_lossy().to_string()).unwrap_or_default();
            (p.clone(), name, folder)
        })
        .collect();
    let entries: Vec<PlaceEntry<'_>> =
        recent.iter().map(|(_, name, folder)| PlaceEntry { name, folder: false, detail: folder, attention: false, remote: false }).collect();
    let content = if entries.is_empty() {
        PlacesContent::Notice(PlacesNotice { icon: PlacesIcon::Recent, title: tl!("Documents you open will show up here."), body: "", action: None })
    } else {
        PlacesContent::Entries(&entries)
    };
    let crumbs = [tl!("Recent")];
    match PlacesPanel::new(&places, Some(0), content, &style, labels).crumbs(&crumbs).show(ui).action {
        Some(PlacesAction::Activate(i)) => {
            if let Some((path, ..)) = recent.get(i) {
                let _ = app.run("file.open", json!({"path": path}));
                app.ui.backstage = false;
                app.canvas.want_focus = true;
            }
        }
        Some(PlacesAction::Select(1)) => {
            if let Some(home) = LocalFolder::home() {
                app.file_browser.show_page_dir(home);
            }
        }
        Some(PlacesAction::Browse) => {
            let _ = app.run("ui.openFileDialog", json!({}));
        }
        _ => {}
    }
}

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
        assert_eq!(ids(""), ["blank", "sample", "letter", "resume", "report"]);
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

    /// File › Open is the shared places panel: Recent with the documents opened lately, This
    /// computer with the folders listed in the page, and Browse for the system's dialog.
    #[test]
    fn open_page_lists_places_and_recent_documents() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default());
        a.ui.recent = vec!["/home/ada/Documents/Handbook.docx".into(), "/home/ada/Desktop/Notes.docx".into()];
        a.run("ui.backstage", json!({"value": true, "page": "open"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        for label in ["This computer", "Browse…", "Handbook.docx, /home/ada/Documents", "Notes.docx, /home/ada/Desktop"] {
            assert!(h.query_by_label(label).is_some(), "{label} is on the Open page");
        }
        assert!(h.query_by_label("Add a place").is_none(), "there is no remote storage to add yet");
        h.get_by_label("This computer").click();
        h.run_steps(4);
        assert!(h.state().file_browser.page.is_some(), "This computer lists a folder in the page");
        assert!(!h.state().file_dialog_open(), "and opens no dialog, unlike Browse");
        assert!(h.query_by_label("Handbook.docx, /home/ada/Documents").is_none(), "the recent documents made way for the folder");
        h.get_by_label("Recent").click();
        h.run_steps(4);
        assert!(h.state().file_browser.page.is_none() && h.query_by_label("Handbook.docx, /home/ada/Documents").is_some(), "Recent comes back");
    }

    /// The File page's column (storytold/craft-libs#53): Back is a labelled first row, every page
    /// is one click away, and Options sits at the foot.
    #[test]
    fn the_column_has_a_labelled_back_and_reaches_every_page() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default());
        a.run("ui.backstage", json!({"value": true, "page": "new"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        for (label, page) in [("Options", "options"), ("Open", "open"), ("Info", "info"), ("Export", "export"), ("Home", "home")] {
            h.get_by_label(label).click();
            h.run_steps(4);
            assert_eq!(h.state().ui.backstage_page, page, "{label} shows its page");
            assert!(h.state().ui.backstage, "{label} stays on the File page");
        }
        h.get_by_label("Back").click();
        h.run_steps(4);
        assert!(!h.state().ui.backstage, "Back leaves the File page");
    }

    /// #1129: with no document open the column offers only what works without one.
    #[test]
    fn with_no_document_the_column_leaves_out_the_document_pages() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default());
        a.start_screen();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        for label in ["Info", "Save As", "Print", "Export", "Close"] {
            assert!(h.query_by_label(label).is_none(), "{label} needs a document");
        }
        h.get_by_label("Options").click();
        h.run_steps(4);
        assert_eq!(h.state().ui.backstage_page, "options");
        h.get_by_label("Open").click();
        h.run_steps(4);
        assert_eq!(h.state().ui.backstage_page, "open");
    }

    /// The File page's Home may be worded apart from the ribbon's Home tab; a language without
    /// such an entry uses its ordinary one.
    #[test]
    fn the_file_pages_home_can_have_its_own_wording() {
        use crate::i18n::{Lang, tr, trc};
        let (pl, de) = (Lang::from_pref("pl"), Lang::from_pref("de"));
        assert_eq!(trc(pl, "file", "Home"), "Strona główna");
        assert_eq!(tr(pl, "Home"), "Start", "the ribbon tab keeps its word");
        assert_eq!(trc(de, "file", "Home"), tr(de, "Home"));
        assert_ne!(tr(pl, "About"), tr(pl, "Info"), "two rows of one column must not read the same");
    }

    /// A folder that cannot be listed says why in the interface language, never in the system's
    /// own error text.
    #[test]
    fn a_folder_that_cannot_be_read_says_so_in_words() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default());
        a.run("ui.backstage", json!({"value": true, "page": "open"})).unwrap();
        a.file_browser.show_page_dir(std::env::temp_dir().join("wordcraft-no-such-folder-for-the-open-page"));
        assert_eq!(a.file_browser.page.as_ref().and_then(|folder| folder.failed()), Some(std::io::ErrorKind::NotFound));
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        assert!(h.query_by_label("This folder is no longer there.").is_some());
        assert!(h.query_by_label_contains("os error").is_none(), "the system's wording stays out of the page");
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

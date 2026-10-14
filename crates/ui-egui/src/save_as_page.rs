//! File › Save As: places on the left (Recent folders, This Computer,
//! Browse), and on the right the folder's path, the file name with its type and Save, and what
//! the folder holds. Saving goes the same way as the system dialog's pick (`finish_pick`).
//!
//! Folders are only listed when shown, never read beyond their names and dates; files are never
//! opened. The web has no folders to show: there Save As is the Browse button alone.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use egui::{Align2, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;

use crate::file_browser::{Entry, home, list_dir};
use crate::file_dialogs::{AfterPick, AfterSave};
use crate::print_page::EXPORT_WORD_2003;
use crate::theme::{Tokens, medium, regular, semibold};
use crate::{WordApp, icons};

/// At most this many entries are listed in a folder.
const MAX_LISTED: usize = 2000;
/// Dates are read for this many entries at most (each is a call to the file system).
const MAX_DATED: usize = 500;
/// At most this many recent folders.
const MAX_RECENT_FOLDERS: usize = 20;
const PLACES_W: f32 = 240.0;
const ROW_H: f32 = 30.0;

/// The types a document can be saved as: (extension, name). `wordml2003` is an `.xml` file.
pub(crate) const TYPES: [(&str, &str); 13] = [
    ("docx", "Word Document (*.docx)"),
    ("docm", "Word Macro-Enabled Document (*.docm)"),
    ("dotx", "Word Template (*.dotx)"),
    ("dotm", "Word Macro-Enabled Template (*.dotm)"),
    ("pdf", "PDF (*.pdf)"),
    ("xml", "Word XML Document (*.xml)"),
    (EXPORT_WORD_2003, "Word 2003 XML Document (*.xml)"),
    ("odt", "OpenDocument Text (*.odt)"),
    ("rtf", "Rich Text Format (*.rtf)"),
    ("html", "Web Page (*.html)"),
    ("md", "Markdown (*.md)"),
    ("tex", "LaTeX (*.tex)"),
    ("txt", "Plain Text (*.txt)"),
];

/// The file extension a type is saved with.
fn file_ext(ty: &str) -> &str {
    if ty == EXPORT_WORD_2003 { "xml" } else { ty }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Place {
    #[default]
    Recent,
    Computer,
}

/// One listed folder or file, with when it was last changed.
#[derive(Clone, Debug)]
struct Row {
    entry: Entry,
    modified: Option<SystemTime>,
}

/// A folder's listing, kept while the folder and type stay the same.
struct Listing {
    dir: PathBuf,
    ty: String,
    rows: Result<Vec<Row>, String>,
    truncated: bool,
}

/// The page's state: kept while File is open, started afresh for each document.
#[derive(Default)]
pub(crate) struct State {
    document: Option<u64>,
    place: Place,
    pub(crate) dir: PathBuf,
    name: String,
    ty: String,
    listing: Option<Listing>,
    /// A file of that name is there: Save asks before replacing it.
    replace: Option<PathBuf>,
    error: Option<String>,
}

impl State {
    /// Start for the open document: its name and type, in its folder (or Documents).
    fn start(&mut self, app_doc: u64, path: Option<&Path>, stem: String) {
        let ext = path.and_then(|p| p.extension()).and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
        let ty = ext.filter(|e| TYPES.iter().any(|(t, _)| t == e)).unwrap_or_else(|| "docx".into());
        let dir = path.and_then(Path::parent).filter(|d| !d.as_os_str().is_empty()).map(Path::to_path_buf).unwrap_or_else(documents);
        *self = State { document: Some(app_doc), place: self.place, dir, name: stem, ty, ..Default::default() };
    }

    fn go(&mut self, dir: PathBuf) {
        self.dir = dir;
        self.place = Place::Computer;
        self.replace = None;
        self.error = None;
    }
}

/// The Documents folder, else the home folder, else where WordCraft runs.
fn documents() -> PathBuf {
    let home = home().unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let docs = home.join("Documents");
    if docs.is_dir() { docs } else { home }
}

/// The folders of the recent documents, most recent first, each once, only those still there.
fn recent_folders(recent: &[String]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for path in recent {
        let Some(dir) = Path::new(path).parent().filter(|d| !d.as_os_str().is_empty()) else { continue };
        if !out.iter().any(|d| d == dir) && dir.is_dir() {
            out.push(dir.to_path_buf());
        }
        if out.len() >= MAX_RECENT_FOLDERS {
            break;
        }
    }
    out
}

/// The file the name field and type stand for, or why there is none.
pub(crate) fn target(dir: &Path, name: &str, ty: &str) -> Result<PathBuf, &'static str> {
    let ext = file_ext(ty);
    let name = name.trim();
    // A name typed with this type's extension keeps it once.
    let stem = name.strip_suffix(&format!(".{ext}")).or_else(|| {
        let lower = name.to_ascii_lowercase();
        lower.ends_with(&format!(".{ext}")).then(|| name.get(..name.len().saturating_sub(ext.len() + 1)).unwrap_or(name))
    });
    let stem = stem.unwrap_or(name).trim();
    if stem.is_empty() || stem == "." || stem == ".." {
        return Err("Type a file name.");
    }
    if stem.contains('/') || stem.contains('\\') || stem.contains('\0') {
        return Err("A file name can't contain / or \\.");
    }
    Ok(dir.join(format!("{stem}.{ext}")))
}

/// A date as `yyyy-MM-dd` (UTC), from a time since 1970.
pub(crate) fn date(time: SystemTime) -> Option<String> {
    let secs = time.duration_since(SystemTime::UNIX_EPOCH).ok()?.as_secs();
    let days = i64::try_from(secs / 86_400).ok()?;
    // Days to a civil date (proleptic Gregorian).
    let z = days.checked_add(719_468)?;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

fn list(dir: &Path, ty: &str) -> Listing {
    let rows = list_dir(dir, &[file_ext(ty).to_owned()], MAX_LISTED);
    let truncated = rows.as_ref().is_ok_and(|l| l.truncated);
    let rows = rows.map_err(|e| e.to_string()).map(|l| {
        l.entries
            .into_iter()
            .enumerate()
            .map(|(i, entry)| {
                let modified = if i < MAX_DATED { std::fs::metadata(&entry.path).and_then(|m| m.modified()).ok() } else { None };
                Row { entry, modified }
            })
            .collect()
    });
    Listing { dir: dir.to_path_buf(), ty: ty.to_owned(), rows, truncated }
}

/// The Save As page.
pub fn show(app: &mut WordApp, ui: &mut Ui) {
    let doc = app.session.document_id();
    if app.save_as.document != Some(doc) {
        let path = app.session.path.clone();
        let stem = app.title_stem();
        app.save_as.start(doc, path.as_deref(), stem);
    }
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(tl!("Save As")).font(semibold(26.0)));
    ui.add_space(16.0);
    let full = ui.available_rect_before_wrap();
    ui.allocate_rect(full, Sense::hover());
    let left = Rect::from_min_size(full.min, vec2(PLACES_W.min(full.width() * 0.35), full.height()));
    let right =
        Rect::from_min_max(pos2((left.max.x + 28.0).min(full.max.x), full.min.y), pos2((full.max.x - 30.0).max(left.max.x), full.max.y - 20.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(left), |ui| places(app, ui));
    ui.painter().vline(left.max.x + 14.0, full.min.y..=full.max.y - 20.0, Stroke::new(1.0, t.border));
    if right.width() > 200.0 {
        ui.scope_builder(egui::UiBuilder::new().max_rect(right), |ui| match app.save_as.place {
            Place::Recent => recent(app, ui),
            Place::Computer => folder(app, ui),
        });
    }
}

/// One entry in the places column: an icon, a name and a line under it. True when clicked.
fn place(ui: &mut Ui, icon: &str, title: &str, detail: &str, selected: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 58.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, title));
    if selected {
        ui.painter().rect_filled(r, 3.0, t.checked);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, 3.0, t.hover);
    }
    icons::paint(ui.painter(), Rect::from_center_size(pos2(r.min.x + 26.0, r.center().y), vec2(30.0, 30.0)), icon, t.icon, t.accent);
    let x = r.min.x + 54.0;
    if detail.is_empty() {
        ui.painter().text(pos2(x, r.center().y), Align2::LEFT_CENTER, title, medium(14.0), t.text);
    } else {
        ui.painter().text(pos2(x, r.min.y + 20.0), Align2::LEFT_CENTER, title, medium(14.0), t.text);
        let detail = clip(ui, detail, regular(12.0), r.max.x - x - 8.0);
        ui.painter().text(pos2(x, r.min.y + 39.0), Align2::LEFT_CENTER, detail, regular(12.0), t.text_dim);
    }
    resp.clicked()
}

/// `text` shortened from the start (`…/end`) to fit `width`: paths keep their last folders.
fn clip(ui: &Ui, text: &str, font: egui::FontId, width: f32) -> String {
    let fits = |s: &str| ui.painter().layout_no_wrap(s.to_owned(), font.clone(), egui::Color32::WHITE).size().x <= width;
    if fits(text) {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    for skip in 1..chars.len() {
        let rest: String = chars.get(skip..).unwrap_or_default().iter().collect();
        let s = format!("…{rest}");
        if fits(&s) {
            return s;
        }
    }
    "…".into()
}

fn places(app: &mut WordApp, ui: &mut Ui) {
    ui.spacing_mut().item_spacing.y = 4.0;
    let at = app.save_as.place;
    if place(ui, "folder", tl!("Recent"), tl!("Folders you saved or opened in"), at == Place::Recent) {
        app.save_as.place = Place::Recent;
    }
    let computer = tl!("This Computer");
    let documents = documents().display().to_string();
    if place(ui, "homeFolder", computer, &documents, at == Place::Computer) {
        app.save_as.place = Place::Computer;
    }
    ui.add_space(10.0);
    ui.separator();
    ui.add_space(6.0);
    if place(ui, "open", tl!("Browse…"), tl!("Use the system dialog"), false) {
        app.save_as_dialog();
    }
}

/// The file name, its type and Save, over the folder.
fn name_row(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let mut save = false;
    ui.horizontal(|ui| {
        let width = (ui.available_width() - 270.0 - 100.0).max(160.0);
        let field = ui.add(
            egui::TextEdit::singleline(&mut app.save_as.name)
                .hint_text(tl!("Enter file name here"))
                .desired_width(width)
                .font(regular(14.0))
                .margin(vec2(8.0, 6.0)),
        );
        if field.changed() {
            app.save_as.replace = None;
            app.save_as.error = None;
        }
        save |= field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let shown = TYPES.iter().find(|(id, _)| *id == app.save_as.ty).map_or("", |(_, label)| *label);
        egui::ComboBox::from_id_salt("save_as_type").width(250.0).height(400.0).selected_text(tl!(shown)).show_ui(ui, |ui| {
            for (id, label) in TYPES {
                if ui.selectable_label(app.save_as.ty == id, tl!(label)).clicked() && app.save_as.ty != id {
                    app.save_as.ty = id.to_owned();
                    app.save_as.replace = None;
                }
            }
        });
        let ready = target(&app.save_as.dir, &app.save_as.name, &app.save_as.ty).is_ok();
        let button = egui::Button::new(egui::RichText::new(tl!("Save")).font(medium(14.0)).color(if ready { t.on_accent } else { t.text_disabled }))
            .fill(if ready { t.accent } else { t.input })
            .min_size(vec2(88.0, 30.0));
        save |= ui.add_enabled(ready, button).clicked();
    });
    if save {
        try_save(app, false);
    }
    if let Some(path) = app.save_as.replace.clone() {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            ui.label(
                egui::RichText::new(crate::i18n::fmt(tl!("{name} already exists. Do you want to replace it?"), &[("name", &name)])).color(t.orange),
            );
            if ui.button(tl!("Replace")).clicked() {
                try_save(app, true);
            }
            if ui.button(tl!("Cancel")).clicked() {
                app.save_as.replace = None;
            }
        });
    }
    if let Some(error) = &app.save_as.error {
        ui.label(egui::RichText::new(error).color(t.red));
    }
}

/// Save to the folder under the typed name; a file already there is replaced only when the user
/// said so.
fn try_save(app: &mut WordApp, replace: bool) {
    let path = match target(&app.save_as.dir, &app.save_as.name, &app.save_as.ty) {
        Ok(path) => path,
        Err(why) => {
            app.save_as.error = Some(tl!(why).to_owned());
            return;
        }
    };
    if path.exists() && !replace && app.save_as.replace.as_ref() != Some(&path) {
        app.save_as.replace = Some(path);
        return;
    }
    app.save_as.replace = None;
    let text = path.to_string_lossy().into_owned();
    let result = if app.save_as.ty == EXPORT_WORD_2003 {
        let r = app.run("file.saveAs", json!({"path": text, "format": EXPORT_WORD_2003}));
        if r.is_ok() {
            app.ui.backstage = false;
            app.canvas.want_focus = true;
        }
        r
    } else {
        app.finish_pick(AfterPick::SaveAs(AfterSave::Nothing), Some(text))
    };
    match result {
        Ok(_) => {
            // Start afresh next time: the document has a new name and folder.
            app.save_as.document = None;
            app.save_as.listing = None;
        }
        Err(error) => app.save_as.error = Some(error),
    }
}

/// Recent: the folders of recent documents, a click opens one.
fn recent(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    name_row(app, ui);
    ui.add_space(14.0);
    ui.label(egui::RichText::new(tl!("Folders")).font(semibold(15.0)));
    ui.add_space(4.0);
    let mut folders = recent_folders(&app.ui.recent);
    let current = app.save_as.dir.clone();
    if !folders.contains(&current) && current.is_dir() {
        folders.insert(0, current);
    }
    if folders.is_empty() {
        ui.label(egui::RichText::new(tl!("Folders you save in show up here.")).color(t.text_dim));
        return;
    }
    egui::ScrollArea::vertical().id_salt("save_as_recent").auto_shrink([false, false]).show(ui, |ui| {
        for dir in folders {
            let name = dir.file_name().map_or_else(|| dir.display().to_string(), |n| n.to_string_lossy().into_owned());
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 48.0), Sense::click());
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
            if resp.hovered() {
                ui.painter().rect_filled(r, 3.0, t.hover);
            }
            icons::paint(ui.painter(), Rect::from_center_size(pos2(r.min.x + 20.0, r.center().y), vec2(24.0, 24.0)), "folder", t.icon, t.accent);
            ui.painter().text(pos2(r.min.x + 44.0, r.min.y + 16.0), Align2::LEFT_CENTER, &name, medium(13.5), t.text);
            let shown = clip(ui, &dir.display().to_string(), regular(12.0), r.width() - 54.0);
            ui.painter().text(pos2(r.min.x + 44.0, r.min.y + 34.0), Align2::LEFT_CENTER, shown, regular(12.0), t.text_dim);
            if resp.clicked() {
                app.save_as.go(dir);
            }
        }
    });
}

/// The folder's path as links, with Up.
fn path_bar(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        let parent = app.save_as.dir.parent().map(Path::to_path_buf);
        let (r, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), if parent.is_some() { Sense::click() } else { Sense::hover() });
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, parent.is_some(), tl!("Up one level")));
        if parent.is_some() && resp.hovered() {
            ui.painter().rect_filled(r, 3.0, t.hover);
        }
        let colour = if parent.is_some() { t.icon } else { t.text_disabled };
        icons::paint(ui.painter(), r.shrink(5.0), "chevronUp", colour, colour);
        if let Some(parent) = parent
            && resp.on_hover_text(tl!("Up one level")).clicked()
        {
            app.save_as.go(parent);
        }
        let parts: Vec<PathBuf> = app.save_as.dir.ancestors().map(Path::to_path_buf).collect::<Vec<_>>().into_iter().rev().collect();
        let skip = parts.len().saturating_sub(5);
        if skip > 0 {
            ui.label(egui::RichText::new("…  ›").color(t.text_dim));
        }
        let mut go = None;
        let count = parts.len();
        for (i, part) in parts.iter().enumerate().skip(skip) {
            let name = part.file_name().map_or_else(|| part.display().to_string(), |n| n.to_string_lossy().into_owned());
            let last = i + 1 == count;
            let text = egui::RichText::new(name).font(if last { medium(14.0) } else { regular(14.0) });
            if ui.add(egui::Button::new(text).frame(false)).clicked() && !last {
                go = Some(part.clone());
            }
            if !last {
                ui.label(egui::RichText::new("›").color(t.text_dim));
            }
        }
        if let Some(dir) = go {
            app.save_as.go(dir);
        }
    });
}

/// This Computer: the folder's path, the name row, then its folders and files of the type.
fn folder(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    path_bar(app, ui);
    ui.add_space(6.0);
    name_row(app, ui);
    ui.add_space(12.0);
    let stale = app.save_as.listing.as_ref().is_none_or(|l| l.dir != app.save_as.dir || l.ty != app.save_as.ty);
    if stale {
        app.save_as.listing = Some(list(&app.save_as.dir, &app.save_as.ty));
    }
    // Header: Name and Date modified.
    let width = ui.available_width();
    let date_x = (width - 150.0).max(200.0);
    let (head, _) = ui.allocate_exact_size(vec2(width, 26.0), Sense::hover());
    ui.painter().text(pos2(head.min.x + 36.0, head.center().y), Align2::LEFT_CENTER, tl!("Name"), medium(12.5), t.text_dim);
    ui.painter().text(pos2(head.min.x + date_x, head.center().y), Align2::LEFT_CENTER, tl!("Date modified"), medium(12.5), t.text_dim);
    ui.painter().hline(head.x_range(), head.max.y, Stroke::new(1.0, t.border));
    let Some(listing) = &app.save_as.listing else { return };
    let rows = match &listing.rows {
        Ok(rows) => rows.clone(),
        Err(error) => {
            ui.label(egui::RichText::new(crate::i18n::fmt(tl!("This folder can't be shown: {error}"), &[("error", error)])).color(t.red));
            return;
        }
    };
    let truncated = listing.truncated;
    if rows.is_empty() {
        ui.add_space(8.0);
        ui.label(egui::RichText::new(tl!("This folder is empty.")).color(t.text_dim));
        return;
    }
    let mut go = None;
    let mut pick = None;
    egui::ScrollArea::vertical().id_salt("save_as_folder").auto_shrink([false, false]).show_rows(ui, ROW_H, rows.len(), |ui, range| {
        for row in rows.get(range).unwrap_or_default() {
            let (r, resp) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::click());
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &row.entry.name));
            if resp.hovered() {
                ui.painter().rect_filled(r, 2.0, t.hover);
            }
            let icon = if row.entry.dir { "folder" } else { "file" };
            icons::paint(ui.painter(), Rect::from_center_size(pos2(r.min.x + 16.0, r.center().y), vec2(20.0, 20.0)), icon, t.icon, t.accent);
            let name = clip(ui, &row.entry.name, regular(13.5), date_x - 46.0);
            ui.painter().text(
                pos2(r.min.x + 36.0, r.center().y),
                Align2::LEFT_CENTER,
                name,
                regular(13.5),
                if row.entry.dir { t.text } else { t.text_dim },
            );
            if let Some(day) = row.modified.and_then(date) {
                ui.painter().text(pos2(r.min.x + date_x, r.center().y), Align2::LEFT_CENTER, day, regular(12.5), t.text_dim);
            }
            if resp.clicked() {
                if row.entry.dir {
                    go = Some(row.entry.path.clone());
                } else {
                    pick = Some(row.entry.path.clone());
                }
            }
        }
        if truncated {
            ui.label(egui::RichText::new(tl!("Only the first files are listed.")).color(t.text_dim));
        }
    });
    if let Some(dir) = go {
        app.save_as.go(dir);
    }
    // A file's name goes into the name field: Save then asks to replace it.
    if let Some(file) = pick {
        app.save_as.name = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        app.save_as.replace = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_name_gets_the_types_extension_once() {
        let dir = Path::new("/docs");
        assert_eq!(target(dir, "Letter", "docx").unwrap(), dir.join("Letter.docx"));
        assert_eq!(target(dir, " Letter.docx ", "docx").unwrap(), dir.join("Letter.docx"));
        assert_eq!(target(dir, "Letter.DOCX", "docx").unwrap(), dir.join("Letter.docx"));
        assert_eq!(target(dir, "Report.v2", "pdf").unwrap(), dir.join("Report.v2.pdf"));
        assert_eq!(target(dir, "Old", EXPORT_WORD_2003).unwrap(), dir.join("Old.xml"));
        for bad in ["", "  ", ".docx", "..", "a/b", "a\\b"] {
            assert!(target(dir, bad, "docx").is_err(), "{bad:?}");
        }
    }

    #[test]
    fn dates_are_shown_as_calendar_days() {
        let at = |secs| SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs);
        assert_eq!(date(at(0)).as_deref(), Some("1970-01-01"));
        assert_eq!(date(at(951_782_400)).as_deref(), Some("2000-02-29"));
        assert_eq!(date(at(1_791_936_000)).as_deref(), Some("2026-10-14"));
        assert_eq!(date(SystemTime::UNIX_EPOCH - std::time::Duration::from_secs(10)), None);
    }

    #[test]
    fn recent_folders_are_listed_once_most_recent_first() {
        let tmp = std::env::temp_dir();
        let a = tmp.join("a.docx").display().to_string();
        let b = tmp.join("b.docx").display().to_string();
        assert_eq!(recent_folders(&[a, b, "/no/such/folder/c.docx".into(), "d.docx".into()]), vec![tmp]);
    }

    #[test]
    fn every_type_is_one_wordcraft_can_save() {
        for (ty, _) in TYPES {
            let ext = file_ext(ty);
            assert!(wordcraft_engine::io::SAVE_EXTS.contains(&ext), "{ext}");
        }
    }
}

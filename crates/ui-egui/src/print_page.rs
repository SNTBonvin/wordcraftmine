//! File › Print (#641): the print settings and a preview of the pages (settings scrolling in
//! their own column, the preview filling the rest).
//!
//! Page range and parity shape the PDF. Native printer settings and copy collation go to
//! the shared spooler; browser printing and saved print PDFs retain copies in the PDF.

use std::sync::Arc;

use egui::{Rect, Sense, Stroke, TextureHandle, Ui, pos2, vec2};
use serde_json::{Value, json};
use wordcraft_layout::DocLayout;

use crate::theme::{Tokens, medium, regular, semibold};
use crate::{WordApp, icons};

/// The Print page's settings. Not saved with the document or the UI state.
#[derive(Clone, Debug, PartialEq)]
pub struct PrintSettings {
    /// `all`, `current`, `selection` or `custom`.
    pub range: String,
    /// The custom range text, like `1-3, 5`.
    pub pages: String,
    /// `all`, `odd` or `even`.
    pub parity: String,
    pub copies: u32,
    pub collate: bool,
    /// The page shown in the preview (0-based).
    pub preview: usize,
    /// The preview's zoom: 1 shows the whole page, up to 4 times that.
    pub zoom: f32,
}

impl Default for PrintSettings {
    fn default() -> Self {
        PrintSettings { range: "all".into(), pages: String::new(), parity: "all".into(), copies: 1, collate: true, preview: 0, zoom: 1.0 }
    }
}

impl PrintSettings {
    /// The params of `ui.print` and `file.printPdf` for these settings.
    pub fn params(&self) -> Value {
        let mut v = json!({"range": self.range, "parity": self.parity, "copies": self.copies, "collate": self.collate});
        if self.range == "custom" {
            v["pages"] = json!(self.pages);
        }
        v
    }
}

/// The output layout, kept while the document is unchanged (laying out every frame would be slow).
fn layout_of(app: &WordApp, ctx: &egui::Context) -> Arc<DocLayout> {
    let id = egui::Id::new("print_page_layout");
    let rev = app.session.rev();
    if let Some((r, l)) = ctx.data(|d| d.get_temp::<(u64, Arc<DocLayout>)>(id))
        && r == rev
    {
        return l;
    }
    let l = app.session.export_layout();
    ctx.data_mut(|d| d.insert_temp(id, (rev, l.clone())));
    l
}

/// The texture of one page, re-rendered only when the document, page or size changes.
fn page_texture(app: &WordApp, ctx: &egui::Context, lay: &DocLayout, page: usize, width: f32) -> Option<TextureHandle> {
    let id = egui::Id::new("print_page_texture");
    let key = (app.session.rev(), page, width as u32, (ctx.pixels_per_point() * 100.0) as u32);
    if let Some((k, t)) = ctx.data(|d| d.get_temp::<((u64, usize, u32, u32), TextureHandle)>(id))
        && k == key
    {
        return Some(t);
    }
    let pg = lay.pages.get(page)?;
    let scale = (width / pg.w.max(1.0) * ctx.pixels_per_point()).clamp(0.05, 4.0);
    let img = wordcraft_render::render_page(&app.session.doc, pg, scale, &Default::default());
    let color = egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.to_straight());
    let tex = ctx.load_texture("print_page_preview", color, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, (key, tex.clone())));
    Some(tex)
}

/// The settings column's width, and the setting cards'.
const SETTINGS_W: f32 = 340.0;
const CARD_W: f32 = 300.0;
const CARD_H: f32 = 50.0;
/// The preview's zoom, as a factor on "whole page in view".
const MAX_ZOOM: f32 = 4.0;

/// One choice of a setting card: its id, name and a line saying what it does.
struct Choice {
    id: String,
    title: String,
    detail: String,
}

fn choice(id: &str, title: &str, detail: impl Into<String>) -> Choice {
    Choice { id: id.into(), title: tl!(title).into(), detail: detail.into() }
}

/// The Print page: the print button and settings in a column that scrolls
/// on its own, the preview filling the rest of the window on grey.
pub fn show(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    let lay = layout_of(app, &ctx);
    let total = lay.pages.len();
    let current = app.session.current_pages(&lay);
    app.ui.print.preview = app.ui.print.preview.min(total.saturating_sub(1));

    let full = ui.available_rect_before_wrap();
    ui.allocate_rect(full, Sense::hover());
    let width = SETTINGS_W.min((full.width() * 0.5).max(240.0));
    let left = Rect::from_min_size(full.min, vec2(width, full.height()));
    let right = Rect::from_min_max(pos2((left.max.x + 24.0).min(full.max.x), full.min.y), full.max);
    ui.scope_builder(egui::UiBuilder::new().max_rect(left), |ui| {
        egui::ScrollArea::vertical().id_salt("print_settings").auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width((width - 18.0).max(200.0));
            ui.add_space(30.0);
            settings(app, ui, total, current);
            ui.add_space(30.0);
        });
    });
    if right.width() > 120.0 {
        ui.painter().rect_filled(right, 0.0, t.canvas);
        ui.painter().vline(right.min.x, full.y_range(), Stroke::new(1.0, t.border));
        ui.scope_builder(egui::UiBuilder::new().max_rect(right), |ui| preview(app, ui, &lay, total));
    }
}

/// The settings column: Print and Copies, the printer, then the page settings.
fn settings(app: &mut WordApp, ui: &mut Ui, total: usize, current: (usize, usize)) {
    let t = Tokens::get(ui.ctx());
    ui.spacing_mut().item_spacing.y = 6.0;
    ui.label(egui::RichText::new(tl!("Print")).font(semibold(26.0)));
    ui.add_space(14.0);

    // Where the system has a print dialog (Linux and the BSDs, through the desktop's portal),
    // Print opens it and the printer is chosen there; otherwise the printer is chosen here.
    let dialog = crate::printing::dialog_available(app);
    let native = !dialog && app.services.print_native.is_some();
    let hook = dialog || native || app.services.print.is_some();
    if native {
        app.ui.print.copies = app.printing.settings.copies;
        // Find the printers the first time the page shows.
        if app.printing.printers.is_empty()
            && app.printing.message.is_none()
            && !app.printing.discovering()
            && !app.printing.busy()
            && let Some(discover) = &app.services.printers
        {
            app.printing.refresh(discover);
        }
    }

    ui.horizontal(|ui| {
        let ready = if dialog { !app.print_dialog_open() && !app.printing.busy() } else { hook && (!native || app.printing.ready()) };
        if print_button(ui, ready) {
            if dialog {
                let _ = crate::printing::print_with_dialog(app);
            } else {
                let params = app.ui.print.params();
                let _ = app.run("ui.print", params);
            }
        }
        ui.add_space(16.0);
        ui.vertical(|ui| {
            ui.add_space(26.0);
            ui.horizontal(|ui| {
                let label = ui.label(egui::RichText::new(tl!("Copies:")).font(regular(13.5)));
                let mut copies = app.ui.print.copies;
                ui.add_enabled(!app.printing.busy(), egui::DragValue::new(&mut copies).range(1..=wordcraft_engine::print::MAX_COPIES).speed(0.2))
                    .labelled_by(label.id);
                app.ui.print.copies = copies.clamp(1, wordcraft_engine::print::MAX_COPIES);
                if native {
                    app.printing.settings.copies = app.ui.print.copies;
                }
            });
        });
    });
    if !hook {
        ui.label(
            egui::RichText::new(tl!("Printing isn't available here. Save these pages as a PDF and print it from any viewer.")).color(t.text_dim),
        );
    }

    section(ui, "Printer");
    if dialog {
        info_card(ui, "print", tl!("System print dialog"), tl!("Choose the printer when you print"));
    } else if native {
        printer_card(app, ui);
    } else if hook {
        info_card(ui, "print", tl!("Browser print dialog"), tl!("Choose the printer when you print"));
    } else {
        info_card(ui, "print", tl!("No printer"), tl!("Save as PDF to print elsewhere"));
    }

    section(ui, "Settings");
    let ranges = [
        choice("all", "All pages", tl!("The whole document")),
        choice("current", "Current page", tl!("Only the page you're on")),
        choice("selection", "Selected pages", tl!("The pages the selection is on")),
        choice("custom", "Custom range", tl!("Type the pages to print")),
    ];
    if let Some(range) = card(ui, "print_range", "printPages", &app.ui.print.range, &ranges, true) {
        app.ui.print.range = range;
        // Show the first page that will print.
        app.ui.print.preview = if app.ui.print.range == "all" { 0 } else { current.0.min(total.saturating_sub(1)) };
    }
    ui.horizontal(|ui| {
        let label = ui.label(tl!("Pages:"));
        let edit = ui
            .add(
                egui::TextEdit::singleline(&mut app.ui.print.pages)
                    .hint_text(tl!("Pages like 1-3, 5"))
                    .desired_width((CARD_W - 60.0).min(ui.available_width()))
                    .char_limit(wordcraft_engine::print::MAX_SPEC_CHARS),
            )
            .labelled_by(label.id);
        // Typing pages chooses them.
        if edit.changed() {
            app.ui.print.range = if app.ui.print.pages.trim().is_empty() { "all".into() } else { "custom".into() };
        }
    });
    let parities = [
        choice("all", "All pages in the range", tl!("Odd and even pages")),
        choice("odd", "Odd pages only", "1, 3, 5…"),
        choice("even", "Even pages only", "2, 4, 6…"),
    ];
    if let Some(parity) = card(ui, "print_parity", "printPages", &app.ui.print.parity, &parities, true) {
        app.ui.print.parity = parity;
    }
    if native {
        let duplexes = [
            choice("printerDefault", "Printer default", tl!("The printer's own setting")),
            choice("off", "One-sided", tl!("Print on one side of the paper")),
            choice("longEdge", "Two-sided: long edge", tl!("Pages flip on the long edge")),
            choice("shortEdge", "Two-sided: short edge", tl!("Pages flip on the short edge")),
        ];
        let now = match app.printing.settings.duplex {
            None => "printerDefault",
            Some(crate::printing::Duplex::Off) => "off",
            Some(crate::printing::Duplex::LongEdge) => "longEdge",
            Some(crate::printing::Duplex::ShortEdge) => "shortEdge",
        };
        if let Some(duplex) = card(ui, "print_duplex", "printDuplex", now, &duplexes, !app.printing.busy()) {
            app.printing.settings.duplex = match duplex.as_str() {
                "off" => Some(crate::printing::Duplex::Off),
                "longEdge" => Some(crate::printing::Duplex::LongEdge),
                "shortEdge" => Some(crate::printing::Duplex::ShortEdge),
                _ => None,
            };
        }
    }
    let collations = [choice("collated", "Collated", "1,2,3   1,2,3   1,2,3"), choice("uncollated", "Uncollated", "1,1,1   2,2,2   3,3,3")];
    let now = if app.ui.print.collate { "collated" } else { "uncollated" };
    if let Some(collation) = card(ui, "print_collate", "printCollate", now, &collations, app.ui.print.copies > 1 && !app.printing.busy()) {
        app.ui.print.collate = collation == "collated";
    }
    page_cards(app, ui);

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
    if native {
        ui.add_enabled(!app.printing.busy(), egui::Checkbox::new(&mut app.printing.settings.grayscale, tl!("Grayscale")));
    }
    ui.add_space(4.0);
    match wordcraft_engine::print::plan(&app.ui.print.params(), total, current) {
        Ok(plan) => {
            let msg =
                crate::i18n::fmt(tl!("{count} of {total} pages will print"), &[("count", &plan.chosen.to_string()), ("total", &total.to_string())]);
            ui.label(egui::RichText::new(msg).color(t.text_dim));
        }
        Err(e) => {
            ui.label(egui::RichText::new(e.to_string()).color(t.red));
        }
    }
    ui.add_space(6.0);
    if ui.link(tl!("Page Setup")).clicked() {
        let _ = app.run("layout.pageSetup", json!({}));
    }
    if ui.link(tl!("Save these pages as PDF…")).clicked() {
        app.print_pdf_dialog();
    }
}

/// Orientation, paper size and margins of the section the caret is in, changed in place
/// from the Print page.
fn page_cards(app: &mut WordApp, ui: &mut Ui) {
    let sect = wordcraft_engine::cmd::page::sect(&app.session);
    let inches = |pt: f32| {
        let v = format!("{:.2}", pt / 72.0);
        format!("{}\"", v.trim_end_matches('0').trim_end_matches('.'))
    };
    let orientations = [choice("portrait", "Portrait Orientation", ""), choice("landscape", "Landscape Orientation", "")];
    let now = if sect.landscape { "landscape" } else { "portrait" };
    if let Some(value) = card(ui, "print_orientation", "orientation", now, &orientations, true) {
        let _ = app.run("layout.orientation", json!({"value": value}));
    }

    let (w, h) = (sect.page_w.min(sect.page_h), sect.page_w.max(sect.page_h));
    let mut sizes: Vec<Choice> = wordcraft_geom::PAPER_SIZES
        .iter()
        .map(|(name, pw, ph)| Choice { id: (*name).into(), title: (*name).into(), detail: format!("{} × {}", inches(*pw), inches(*ph)) })
        .collect();
    let now = wordcraft_geom::paper_name(w, h).unwrap_or("custom");
    if now == "custom" {
        sizes.push(Choice { id: "custom".into(), title: tl!("Custom size").into(), detail: format!("{} × {}", inches(w), inches(h)) });
    }
    if let Some(name) = card(ui, "print_size", "size", now, &sizes, true)
        && name != "custom"
    {
        let _ = app.run("layout.size", json!({"name": name}));
    }

    let detail = |top: f32, bottom: f32, left: f32, right: f32| {
        crate::i18n::fmt(
            tl!("Top {top} Bottom {bottom} Left {left} Right {right}"),
            &[("top", &inches(top)), ("bottom", &inches(bottom)), ("left", &inches(left)), ("right", &inches(right))],
        )
    };
    let mut margins = vec![
        choice("normal", "Normal Margins", detail(72.0, 72.0, 72.0, 72.0)),
        choice("narrow", "Narrow Margins", detail(36.0, 36.0, 36.0, 36.0)),
        choice("moderate", "Moderate Margins", detail(72.0, 72.0, 54.0, 54.0)),
        choice("wide", "Wide Margins", detail(72.0, 72.0, 144.0, 144.0)),
    ];
    let set = (sect.margin_top, sect.margin_bottom, sect.margin_left, sect.margin_right);
    let close = |a: f32, b: f32| (a - b).abs() < 0.5;
    let now = [
        ("normal", (72.0, 72.0, 72.0, 72.0)),
        ("narrow", (36.0, 36.0, 36.0, 36.0)),
        ("moderate", (72.0, 72.0, 54.0, 54.0)),
        ("wide", (72.0, 72.0, 144.0, 144.0)),
    ]
    .iter()
    .find(|(_, m)| close(m.0, set.0) && close(m.1, set.1) && close(m.2, set.2) && close(m.3, set.3))
    .map_or("custom", |(id, _)| *id);
    if now == "custom" {
        margins.push(choice("custom", "Custom Margins", detail(set.0, set.1, set.2, set.3)));
    }
    if let Some(preset) = card(ui, "print_margins", "margins", now, &margins, true)
        && preset != "custom"
    {
        let _ = app.run("layout.margins", json!({"preset": preset}));
    }
}

/// A heading over a group of settings.
fn section(ui: &mut Ui, title: &str) {
    ui.add_space(14.0);
    ui.label(egui::RichText::new(tl!(title)).font(semibold(16.0)));
    ui.add_space(2.0);
}

/// The big Print button: a printer over its name. True when clicked.
fn print_button(ui: &mut Ui, enabled: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(88.0, 88.0), if enabled { Sense::click() } else { Sense::hover() });
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, tl!("Print")));
    let fill = if !enabled {
        t.input
    } else if resp.is_pointer_button_down_on() {
        t.pressed
    } else if resp.hovered() {
        t.hover
    } else {
        t.input
    };
    ui.painter().rect(r, 4.0, fill, Stroke::new(1.0, t.border_strong), egui::StrokeKind::Inside);
    let colour = if enabled { t.icon } else { t.text_disabled };
    icons::paint(
        ui.painter(),
        egui::Rect::from_center_size(pos2(r.center().x, r.min.y + 32.0), vec2(36.0, 36.0)),
        "print",
        colour,
        if enabled { t.accent } else { colour },
    );
    ui.painter().text(
        pos2(r.center().x, r.max.y - 18.0),
        egui::Align2::CENTER_CENTER,
        tl!("Print"),
        medium(14.0),
        if enabled { t.text } else { t.text_disabled },
    );
    enabled && resp.clicked()
}

/// Paint a setting card's face: icon, name and the line below it.
fn card_face(ui: &Ui, r: Rect, icon: &str, title: &str, detail: &str, enabled: bool) {
    let t = Tokens::get(ui.ctx());
    let colour = if enabled { t.icon } else { t.text_disabled };
    icons::paint(
        ui.painter(),
        egui::Rect::from_center_size(pos2(r.min.x + 24.0, r.center().y), vec2(28.0, 28.0)),
        icon,
        colour,
        if enabled { t.accent } else { colour },
    );
    let x = r.min.x + 48.0;
    let width = (r.max.x - x - 28.0).max(20.0);
    let line = |text: &str, y: f32, font: egui::FontId, colour: egui::Color32| {
        let galley = ui.painter().layout(text.to_owned(), font, colour, width);
        let galley = if galley.rows.len() > 1 { ui.painter().layout_no_wrap(truncate(text, 40), regular(12.0), colour) } else { galley };
        ui.painter().galley(pos2(x, y - galley.size().y / 2.0), galley, colour);
    };
    if detail.is_empty() {
        line(title, r.center().y, medium(13.5), if enabled { t.text } else { t.text_disabled });
    } else {
        line(title, r.min.y + 16.0, medium(13.5), if enabled { t.text } else { t.text_disabled });
        line(detail, r.min.y + 34.0, regular(12.0), if enabled { t.text_dim } else { t.text_disabled });
    }
}

/// `text` cut to `max` characters, ending in an ellipsis.
fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max { text.to_owned() } else { text.chars().take(max.saturating_sub(1)).collect::<String>() + "…" }
}

/// A card that only says something (no choices).
fn info_card(ui: &mut Ui, icon: &str, title: &str, detail: &str) {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width().min(CARD_W), CARD_H), Sense::hover());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, title));
    ui.painter().rect(r, 3.0, t.input, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    card_face(ui, r, icon, title, detail, true);
}

/// A setting card: the choice's name and what it does, opening the list of
/// choices. The id of a newly picked choice.
fn card(ui: &mut Ui, salt: &str, icon: &str, current: &str, items: &[Choice], enabled: bool) -> Option<String> {
    let t = Tokens::get(ui.ctx());
    let shown = items.iter().find(|c| c.id == current);
    let (title, detail) = shown.map_or((current.to_owned(), String::new()), |c| (c.title.clone(), c.detail.clone()));
    let w = ui.available_width().min(CARD_W);
    let (r, resp) = ui.allocate_exact_size(vec2(w, CARD_H), if enabled { Sense::click() } else { Sense::hover() });
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, enabled, &title));
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Id::new(salt));
    let fill = if enabled && (resp.hovered() || open) { t.hover } else { t.input };
    ui.painter().rect(r, 3.0, fill, Stroke::new(1.0, if open { t.accent } else { t.border_strong }), egui::StrokeKind::Inside);
    card_face(ui, r, icon, &title, &detail, enabled);
    let arrow = if enabled { t.icon } else { t.text_disabled };
    icons::paint(ui.painter(), egui::Rect::from_center_size(pos2(r.max.x - 16.0, r.center().y), vec2(14.0, 14.0)), "chevronDown", arrow, arrow);
    if !enabled {
        return None;
    }
    let mut picked = None;
    egui::Popup::menu(&resp).id(egui::Id::new(salt)).width(w).show(|ui| {
        egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
            for c in items {
                let (ir, iresp) = ui.allocate_exact_size(vec2(w - 12.0, if c.detail.is_empty() { 36.0 } else { 46.0 }), Sense::click());
                iresp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, c.id == current, &c.title));
                if c.id == current {
                    ui.painter().rect_filled(ir, 3.0, t.checked);
                } else if iresp.hovered() {
                    ui.painter().rect_filled(ir, 3.0, t.hover);
                }
                card_face(ui, ir, icon, &c.title, &c.detail, true);
                if iresp.clicked() {
                    picked = Some(c.id.clone());
                    ui.close();
                }
            }
        });
    });
    picked.filter(|p| p != current)
}

/// The printer card where WordCraft chooses the printer itself (no system dialog), with its
/// status, Printer Properties (Windows) and a way to look for printers again.
fn printer_card(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let printers: Vec<Choice> = app
        .printing
        .printers
        .iter()
        .map(|p| Choice {
            id: p.name.clone(),
            title: p.name.clone(),
            detail: if p.default { tl!("Default printer").into() } else { tl!("Ready").into() },
        })
        .collect();
    let mut now = app.printing.settings.printer.clone();
    let enabled = !app.printing.busy() && !app.printing.discovering() && !printers.is_empty();
    let shown =
        if now.is_empty() { vec![Choice { id: String::new(), title: tl!("Select a printer").into(), detail: String::new() }] } else { Vec::new() };
    let all: Vec<Choice> = shown.into_iter().chain(printers).collect();
    if let Some(printer) = card(ui, "print_printer", "print", &now, &all, enabled)
        && !printer.is_empty()
    {
        app.printing.settings.printer = printer.clone();
        now = printer;
    }
    if let Some(message) = &app.printing.message {
        ui.label(egui::RichText::new(crate::printing::feedback(message, |label| crate::i18n::t(label).to_owned())).color(t.text_dim));
    }
    ui.horizontal(|ui| {
        if let Some(properties) = &app.services.printer_properties
            && ui.add_enabled(!now.is_empty(), egui::Link::new(tl!("Printer Properties"))).clicked()
            && let Err(error) = properties(&now)
        {
            app.status(error);
        }
        if ui.add_enabled(!app.printing.discovering() && !app.printing.busy(), egui::Link::new(tl!("Refresh printers"))).clicked()
            && let Some(discover) = &app.services.printers
        {
            app.printing.refresh(discover);
        }
    });
}

/// The preview: the page on grey, fitted to the window (or zoomed), with page and zoom controls
/// along the bottom.
fn preview(app: &mut WordApp, ui: &mut Ui, lay: &DocLayout, total: usize) {
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    let full = ui.max_rect();
    let bar = Rect::from_min_max(pos2(full.min.x, full.max.y - 44.0), full.max);
    let area = Rect::from_min_max(full.min, pos2(full.max.x, bar.min.y)).shrink(24.0);
    let page = app.ui.print.preview;
    let (pw, ph) = lay.pages.get(page).map_or((612.0, 792.0), |p| (p.w.max(1.0), p.h.max(1.0)));
    let fit = (area.width() / pw).min(area.height() / ph).clamp(0.02, 8.0);
    app.ui.print.zoom = app.ui.print.zoom.clamp(1.0, MAX_ZOOM);
    let scale = fit * app.ui.print.zoom;
    let size = vec2(pw * scale, ph * scale);

    ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
        egui::ScrollArea::both().id_salt("print_preview").auto_shrink([false, false]).show(ui, |ui| {
            let room = vec2(size.x.max(area.width()), size.y.max(area.height()));
            let (space, resp) = ui.allocate_exact_size(room, Sense::hover());
            let r = Rect::from_center_size(space.center(), size);
            ui.painter().rect_filled(r.translate(vec2(2.0, 3.0)), 0.0, t.page_shadow);
            ui.painter().rect(r, 0.0, egui::Color32::WHITE, Stroke::new(1.0, t.border_strong), egui::StrokeKind::Outside);
            if let Some(tex) = page_texture(app, &ctx, lay, page, size.x) {
                ui.painter().image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
            }
            // With the whole page in view the wheel turns pages; zoomed in, it scrolls.
            if app.ui.print.zoom <= 1.0 && resp.hovered() {
                let id = egui::Id::new("print_preview_wheel");
                let delta = ctx.input(|i| i.smooth_scroll_delta.y);
                let mut wheel = ctx.data(|d| d.get_temp::<f32>(id)).unwrap_or(0.0) + delta;
                if wheel <= -60.0 && page + 1 < total {
                    app.ui.print.preview = page + 1;
                    wheel = 0.0;
                } else if wheel >= 60.0 && page > 0 {
                    app.ui.print.preview = page - 1;
                    wheel = 0.0;
                }
                ctx.data_mut(|d| d.insert_temp(id, wheel.clamp(-60.0, 60.0)));
            }
        });
    });

    ui.scope_builder(egui::UiBuilder::new().max_rect(bar.shrink2(vec2(16.0, 6.0))), |ui| {
        ui.horizontal_centered(|ui| {
            let nav = |ui: &mut Ui, icon: &str, tip: &str, enabled: bool| {
                let (r, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), if enabled { Sense::click() } else { Sense::hover() });
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, tip));
                let col = if enabled { t.icon } else { t.text_disabled };
                if enabled && resp.hovered() {
                    ui.painter().rect_filled(r, 3.0, t.hover);
                }
                icons::paint(ui.painter(), r.shrink(4.0), icon, col, col);
                enabled && resp.on_hover_text(tip).clicked()
            };
            if nav(ui, "chevronLeft", tl!("Previous Page"), page > 0) {
                app.ui.print.preview = page - 1;
            }
            let mut shown = app.ui.print.preview + 1;
            if ui.add(egui::DragValue::new(&mut shown).range(1..=total.max(1)).speed(0.1)).changed() {
                app.ui.print.preview = shown.saturating_sub(1).min(total.saturating_sub(1));
            }
            ui.label(crate::i18n::fmt(tl!("of {pages}"), &[("pages", &total.to_string())]));
            if nav(ui, "chevronRight", tl!("Next Page"), page + 1 < total) {
                app.ui.print.preview = page + 1;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if nav(ui, "onePage", tl!("Zoom to Page"), app.ui.print.zoom > 1.0) {
                    app.ui.print.zoom = 1.0;
                }
                if nav(ui, "zoomIn", tl!("Zoom In"), app.ui.print.zoom < MAX_ZOOM) {
                    app.ui.print.zoom = (app.ui.print.zoom + 0.25).min(MAX_ZOOM);
                }
                ui.add(egui::Slider::new(&mut app.ui.print.zoom, 1.0..=MAX_ZOOM).show_value(false));
                if nav(ui, "zoomOut", tl!("Zoom Out"), app.ui.print.zoom > 1.0) {
                    app.ui.print.zoom = (app.ui.print.zoom - 0.25).max(1.0);
                }
                let percent = (scale / crate::canvas::PX_PER_PT * 100.0).round();
                ui.label(egui::RichText::new(format!("{percent}%")).font(regular(13.0)));
            });
        });
    });
}

/// The export "format" of a Word 2003 XML Document: an `.xml` file saved with `format: "wordml2003"`.
pub(crate) const EXPORT_WORD_2003: &str = "wordml2003";

/// The "save a copy" formats (File › Export, and below the settings on the Print page).
pub fn export_formats(app: &mut WordApp, ui: &mut Ui) {
    for (label, ext) in [
        ("PDF document (*.pdf)", "pdf"),
        ("Word document (*.docx)", "docx"),
        ("Word XML Document (*.xml)", "xml"),
        ("Word 2003 XML Document (*.xml)", EXPORT_WORD_2003),
        ("OpenDocument Text (*.odt)", "odt"),
        ("Rich Text Format (*.rtf)", "rtf"),
        ("Web page (*.html)", "html"),
        ("Markdown (*.md)", "md"),
        ("LaTeX (*.tex)", "tex"),
        ("Plain text (*.txt)", "txt"),
        ("Page image (*.png)", "png"),
    ] {
        if ui.add(egui::Button::new(egui::RichText::new(tl!(label)).font(medium(13.5))).min_size(vec2(320.0, 34.0))).clicked() {
            app.export_dialog(ext);
        }
        ui.add_space(4.0);
    }
}

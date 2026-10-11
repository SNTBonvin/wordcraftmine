//! File ▸ Options ▸ Interface theme: the appearance mode (follow the system, Light or Dark) above a
//! light and a dark theme card, each with a small picture of WordCraft in that theme.

use egui::{Rect, RichText, Ui, pos2, vec2};
use serde_json::json;

use crate::WordApp;
use crate::theme::{Appearance, Tokens, semibold};

/// The themes each card offers. One of each today; a new theme is one more row here.
pub const LIGHT_THEMES: &[Appearance] = &[Appearance::Light];
pub const DARK_THEMES: &[Appearance] = &[Appearance::Dark];

/// A miniature WordCraft window in `dark` or light colours: title bar, ribbon tabs and groups, the
/// ruler, a page on the canvas and the status bar.
fn preview(ui: &mut Ui, dark: bool, width: f32) {
    let p = if dark { Tokens::dark() } else { Tokens::light() };
    let (rect, _) = ui.allocate_exact_size(vec2(width, 118.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, p.canvas);
    let title = Rect::from_min_size(rect.min, vec2(rect.width(), 11.0));
    painter.rect_filled(title, 0.0, p.title_bar);
    painter.rect_filled(Rect::from_center_size(title.center(), vec2(rect.width() * 0.25, 2.5)), 1.0, p.text_dim);
    // The ribbon: tabs (Home selected), then groups of buttons with their labels.
    let tabs = Rect::from_min_size(pos2(rect.left(), title.bottom()), vec2(rect.width(), 9.0));
    painter.rect_filled(tabs, 0.0, p.tab_strip);
    for i in 0..6 {
        let x = tabs.left() + 8.0 + i as f32 * 20.0;
        painter.rect_filled(Rect::from_min_size(pos2(x, tabs.top() + 3.5), vec2(12.0, 2.0)), 1.0, if i == 1 { p.accent } else { p.text_dim });
    }
    let ribbon = Rect::from_min_size(pos2(rect.left(), tabs.bottom()), vec2(rect.width(), 20.0));
    painter.rect_filled(ribbon, 0.0, p.ribbon);
    let mut x = ribbon.left() + 6.0;
    for buttons in [2, 5, 4, 3] {
        for b in 0..buttons {
            painter.rect_filled(Rect::from_min_size(pos2(x + b as f32 * 7.0, ribbon.top() + 4.0), vec2(5.0, 5.0)), 1.0, p.icon);
        }
        let w = buttons as f32 * 7.0;
        painter.rect_filled(Rect::from_min_size(pos2(x + w * 0.2, ribbon.bottom() - 5.0), vec2(w * 0.5, 1.5)), 0.5, p.group_label);
        x += w + 4.0;
        painter.line_segment([pos2(x - 2.5, ribbon.top() + 3.0), pos2(x - 2.5, ribbon.bottom() - 3.0)], egui::Stroke::new(0.5, p.border));
    }
    painter.line_segment([ribbon.left_bottom(), ribbon.right_bottom()], egui::Stroke::new(0.5, p.border));
    // The status bar.
    let status = Rect::from_min_max(pos2(rect.left(), rect.bottom() - 7.0), rect.max);
    painter.rect_filled(status, 0.0, p.status_bar);
    // The page with its ruler: a heading and lines of text.
    let page_w = (rect.width() * 0.42).round();
    let ruler = Rect::from_min_size(pos2(rect.center().x - page_w / 2.0, ribbon.bottom() + 3.0), vec2(page_w, 4.0));
    painter.rect_filled(ruler, 0.0, p.ruler);
    let page = Rect::from_min_max(pos2(ruler.left(), ruler.bottom() + 3.0), pos2(ruler.right(), status.top() + 6.0));
    let page = page.intersect(Rect::from_min_max(rect.min, pos2(rect.right(), status.top())));
    painter.rect_filled(page.translate(vec2(1.0, 1.0)), 0.0, p.page_shadow);
    painter.rect_filled(page, 0.0, egui::Color32::WHITE);
    painter.rect_filled(
        Rect::from_min_size(page.min + vec2(8.0, 7.0), vec2(page.width() * 0.45, 3.0)),
        1.0,
        egui::Color32::from_rgb(0x1F, 0x3A, 0x6E),
    );
    let ink = egui::Color32::from_gray(0x70);
    for i in 0..7 {
        let w = if i % 3 == 2 { 0.5 } else { 0.8 };
        let r = Rect::from_min_size(page.min + vec2(8.0, 15.0 + i as f32 * 5.5), vec2(page.width() * w - 8.0, 1.5));
        if r.bottom() < page.bottom() - 2.0 {
            painter.rect_filled(r, 0.5, ink);
        }
    }
}

/// One theme card: its title, Active when it is what shows now, a preview of the chosen theme and
/// a radio row per theme.
fn card(ui: &mut Ui, app: &mut WordApp, t: &Tokens, title: &str, active: bool, themes: &[Appearance]) {
    egui::Frame::new()
        .fill(t.panel)
        .stroke(egui::Stroke::new(1.0, if active { t.accent } else { t.border }))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(title).font(semibold(12.5)).color(t.text));
                if active {
                    ui.label(RichText::new(tl!("Active")).color(t.accent_text).small());
                }
            });
            ui.add_space(4.0);
            let shown = themes.first().copied().unwrap_or(Appearance::Light);
            preview(ui, shown == Appearance::Dark, ui.available_width());
            ui.add_space(4.0);
            for a in themes {
                // Picking a card's theme shows it now (one theme per card today).
                if ui.radio(app.ui.theme == *a, tl!(a.label())).clicked() {
                    let _ = app.run("ui.theme", json!({"value": a.code()}));
                }
            }
        });
}

/// The mode drop-down and the two theme cards.
pub fn theme_picker(app: &mut WordApp, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.label(tl!("Interface theme:"));
        egui::ComboBox::from_id_salt("interface_theme").selected_text(tl!(app.ui.theme.label())).width(220.0).show_ui(ui, |ui| {
            for a in Appearance::ALL {
                if ui.selectable_label(app.ui.theme == a, tl!(a.label())).clicked() {
                    let _ = app.run("ui.theme", json!({"value": a.code()}));
                }
            }
        });
    });
    ui.add_space(4.0);
    let t = Tokens::get(ui.ctx());
    let light_active = !t.dark;
    ui.allocate_ui(vec2(ui.available_width().min(560.0), 0.0), |ui| {
        ui.columns(2, |cols| {
            let [light, dark] = cols else { return };
            card(light, app, &t, tl!("Light theme"), light_active, LIGHT_THEMES);
            card(dark, app, &t, tl!("Dark theme"), !light_active, DARK_THEMES);
        });
    });
    ui.add_space(4.0);
}

#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable;
    use wordcraft_engine::Session;

    use super::*;
    use crate::Services;

    /// Both theme cards show with the one on screen marked Active; the drop-down and the cards
    /// both choose.
    #[test]
    fn options_choose_the_mode_and_show_both_theme_cards() {
        let app = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        let mut fonts = false;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(640.0, 400.0)).build_ui_state(
            move |ui, app: &mut WordApp| {
                // The UI fonts (the semibold card titles) take effect from the next frame.
                if !fonts {
                    crate::theme::install_fonts(ui.ctx());
                    fonts = true;
                    return;
                }
                theme_picker(app, ui);
            },
            app,
        );
        h.run();
        assert!(h.query_by_label("Light theme").is_some() && h.query_by_label("Dark theme").is_some());
        assert_eq!(h.query_all_by_label("Active").count(), 1);
        if let Ok(path) = std::env::var("WORDCRAFT_APPEARANCE_SHOT") {
            h.render().unwrap().save(path).unwrap();
        }

        let radios: Vec<_> = h.query_all_by_role(egui::accesskit::Role::RadioButton).collect();
        assert_eq!(radios.len(), 2, "one theme on each card");
        radios[1].click();
        h.run();
        assert_eq!(h.state().ui.theme, Appearance::Dark);

        h.get_by_role(egui::accesskit::Role::ComboBox).click();
        h.run();
        h.get_by_label("Use system setting").click();
        h.run();
        assert_eq!(h.state().ui.theme, Appearance::System);
    }
}

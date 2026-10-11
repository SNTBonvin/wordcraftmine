//! Preview for a Grok suggestion. The document changes only when the user accepts it.

use egui::vec2;
use serde_json::json;

use wordcraft_engine::Selection;

use crate::WordApp;
use crate::theme::{Tokens, semibold};

/// A suggestion waiting for thumbs up or thumbs down.
#[derive(Clone)]
pub struct Preview {
    pub action: String,
    pub source: String,
    pub text: String,
    /// Selection the suggestion was made from. Restored before an accept.
    pub sel: Selection,
}

pub fn show(app: &mut WordApp, ctx: &egui::Context) {
    let Some(preview) = app.grok_preview.clone() else { return };
    let (title, proposal) = match preview.action.as_str() {
        "continue" => (tl!("Continue"), tl!("Continuation")),
        "summarize" => (tl!("Summarize"), tl!("Summary")),
        _ => (tl!("Rewrite"), tl!("Rewritten")),
    };
    let mut open = true;
    let mut decision: Option<bool> = None;
    egui::Window::new(title)
        .id(egui::Id::new("grok-preview"))
        .collapsible(false)
        .resizable(true)
        .default_width(520.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .open(&mut open)
        .show(ctx, |ui| {
            let t = Tokens::get(ui.ctx());
            ui.set_min_width(420.0);
            ui.label(egui::RichText::new(tl!("Original")).font(semibold(13.0)).color(t.text));
            passage(ui, "grok-original", &preview.source);
            ui.add_space(8.0);
            ui.label(egui::RichText::new(proposal).font(semibold(13.0)).color(t.text));
            passage(ui, "grok-suggestion", &preview.text);
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button(egui::RichText::new("👍").size(22.0)).on_hover_text(tl!("Use this")).clicked() {
                    decision = Some(true);
                }
                if ui.button(egui::RichText::new("👎").size(22.0)).on_hover_text(tl!("Leave the document unchanged")).clicked() {
                    decision = Some(false);
                }
            });
        });
    if !open {
        decision = Some(false);
    }
    match decision {
        Some(true) => {
            app.session.sel = preview.sel;
            let result = app.run("grok.accept", json!({"action": preview.action, "text": preview.text}));
            app.grok_preview = None;
            if let Err(e) = result {
                app.status(e);
            }
        }
        Some(false) => app.grok_preview = None,
        None => {}
    }
}

fn passage(ui: &mut egui::Ui, id: &str, text: &str) {
    egui::ScrollArea::vertical().id_salt(id).max_height(140.0).show(ui, |ui| {
        ui.set_min_height(48.0);
        ui.label(egui::RichText::new(text).size(14.0));
        ui.allocate_space(vec2(ui.available_width(), 0.0));
    });
}

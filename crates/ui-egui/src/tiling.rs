//! View › Window tiling: Split and Side by Side show the same document in two
//! in-window panes (no second OS window, so the web build keeps working).
//!
//! The first pane is the full canvas ([`crate::canvas::show`], editable). The second
//! pane is a read-only mirror that reuses the canvas page textures: it scrolls on its
//! own, or with the first pane when Synchronous Scrolling is on. The divider drag
//! sets the same `splitRatio` as `view.split {"ratio": …}` for agents.

use egui::{Color32, Rect, Sense, Stroke, Ui, pos2, vec2};

use crate::WordApp;
use crate::theme::Tokens;

/// Divider thickness (screen points).
const DIV: f32 = 7.0;
/// Page padding inside the mirror pane (screen points).
const PAD: f32 = 14.0;

/// Whether two panes are shown: Split or Side by Side.
pub fn tiled(app: &WordApp) -> bool {
    app.session.view.split || app.session.view.side_by_side
}

/// The central canvas: one pane, or two with a draggable divider.
pub fn show(app: &mut WordApp, ui: &mut Ui) {
    if !tiled(app) {
        crate::canvas::show(app, ui);
        return;
    }
    let full = ui.available_rect_before_wrap();
    let degenerate = full.width() <= 0.0 || full.height() <= 0.0 || !full.width().is_finite() || !full.height().is_finite();
    if degenerate {
        return;
    }
    let ratio = app.session.view.split_ratio.clamp(0.1, 0.9);
    // Split stacks top/bottom; Side by Side puts the panes next to each other.
    let horizontal = app.session.view.side_by_side;
    let (first, divider, second) = if horizontal {
        let w = (full.width() * ratio).clamp(DIV, (full.width() - DIV).max(DIV));
        (
            Rect::from_min_max(full.min, pos2(full.min.x + w, full.max.y)),
            Rect::from_min_max(pos2(full.min.x + w, full.min.y), pos2(full.min.x + w + DIV, full.max.y)),
            Rect::from_min_max(pos2(full.min.x + w + DIV, full.min.y), full.max),
        )
    } else {
        let h = (full.height() * ratio).clamp(DIV, (full.height() - DIV).max(DIV));
        (
            Rect::from_min_max(full.min, pos2(full.max.x, full.min.y + h)),
            Rect::from_min_max(pos2(full.min.x, full.min.y + h), pos2(full.max.x, full.min.y + h + DIV)),
            Rect::from_min_max(pos2(full.min.x, full.min.y + h + DIV), full.max),
        )
    };
    {
        let mut first_ui = ui.new_child(egui::UiBuilder::new().max_rect(first));
        crate::canvas::show(app, &mut first_ui);
    }
    drag_divider(app, ui, divider, full, horizontal);
    {
        let mut second_ui = ui.new_child(egui::UiBuilder::new().max_rect(second));
        mirror(app, &mut second_ui, second);
    }
    // The active window keeps its frame: the caret lives in the first pane.
    let t = Tokens::get(ui.ctx());
    let active = app.session.view.active_window;
    let frame = if active == 0 { first } else { second };
    ui.painter().rect_stroke(frame, 0.0, Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
}

/// The divider between the panes: dragging it sets the split ratio (screen only,
/// like scrolling; `view.split {"ratio": …}` is the same setting for agents).
fn drag_divider(app: &mut WordApp, ui: &mut Ui, divider: Rect, full: Rect, horizontal: bool) {
    let t = Tokens::get(ui.ctx());
    let cursor = if horizontal { egui::CursorIcon::ResizeHorizontal } else { egui::CursorIcon::ResizeVertical };
    let resp = ui.interact(divider, ui.id().with("tiling_divider"), Sense::drag()).on_hover_cursor(cursor);
    if resp.hovered() || resp.dragged() {
        ui.painter().rect_filled(divider, 0.0, t.hover);
    }
    if resp.dragged() {
        let delta = resp.drag_delta();
        let step = if horizontal { delta.x / full.width().max(1.0) } else { delta.y / full.height().max(1.0) };
        if step.is_finite() && step != 0.0 {
            let next = app.session.view.split_ratio + step;
            if next.is_finite() {
                app.session.view.split_ratio = next.clamp(0.1, 0.9);
                ui.ctx().request_repaint();
            }
        }
    }
}

/// The read-only mirror: the canvas page textures, or grey placeholders where a
/// page has no texture yet. Its scroll is its own, except with Synchronous
/// Scrolling, which follows the first pane.
fn mirror(app: &mut WordApp, ui: &mut Ui, area: Rect) {
    let t = Tokens::get(ui.ctx());
    let layout = app.session.layout();
    let total = layout.pages.len();
    if total == 0 {
        return;
    }
    let sync = app.session.view.sync_scroll && app.session.view.side_by_side;
    let mut scroll = egui::ScrollArea::both().id_salt("tiling_mirror").auto_shrink([false, false]);
    if sync {
        scroll = scroll.scroll_offset(app.canvas.scroll_offset);
    }
    scroll.show(ui, |ui| {
        let width = (area.width() - PAD * 2.0).max(50.0);
        ui.vertical(|ui| {
            ui.add_space(PAD);
            for (i, page) in layout.pages.iter().enumerate() {
                let aspect = (page.h.max(1.0) / page.w.max(1.0)).clamp(0.2, 5.0);
                let size = vec2(width, width * aspect);
                let (r, _) = ui.allocate_exact_size(size, Sense::hover());
                ui.painter().rect_filled(r.translate(vec2(2.0, 3.0)), 0.0, t.page_shadow);
                ui.painter().rect(r, 0.0, Color32::WHITE, Stroke::new(1.0, t.border_strong), egui::StrokeKind::Inside);
                if let Some(tex) = app.canvas.page_texture(i) {
                    ui.painter().image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
                ui.add_space(PAD);
            }
        });
    });
}

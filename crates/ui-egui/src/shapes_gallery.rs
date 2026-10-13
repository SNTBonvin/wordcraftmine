//! The Insert › Shapes gallery: every preset shape as an icon-only tile, grouped by
//! Word's gallery categories. The icons are drawn from the shared geometry
//! ([`wordcraft_doc::shape_geom`]), never copied from any other product: the gallery-only
//! detail lines (an action button's glyph, a smiley's face) are drawn here in code. The
//! name shows only on hover. Picking a tile arms the shape tool (`draw.shapeTool`);
//! nothing is inserted on click — the user draws the shape by dragging on the canvas.

use egui::{Color32, Painter, Rect, Stroke, Ui, Vec2, pos2, vec2};
use serde_json::json;
use wordcraft_doc::para::{ShapeCategory, ShapeKind};
use wordcraft_doc::shape_geom::{default_adj, shape_geom};

use crate::WordApp;
use crate::theme::Tokens;

/// Draw a shape's icon in `rect`: its outline (filled and stroked) plus the gallery-only
/// detail lines. `fill` and `stroke` are the gallery's colours.
pub fn shape_icon(p: &Painter, rect: Rect, kind: ShapeKind, fill: Color32, stroke: Color32) {
    shape_icon_adj(p, rect, kind, default_adj(kind), fill, stroke);
}

/// The same, with an explicit adjustment (the live preview while the adjustment handle drags).
pub fn shape_icon_adj(p: &Painter, rect: Rect, kind: ShapeKind, adj: f32, fill: Color32, stroke: Color32) {
    let map = |(x, y): (f32, f32)| pos2(rect.min.x + x * rect.width(), rect.min.y + y * rect.height());
    let line = Stroke::new(1.0, stroke);
    if kind == ShapeKind::Rectangle || kind == ShapeKind::TextBox {
        p.rect(rect, 0.0, fill, line, egui::StrokeKind::Inside);
        return;
    }
    let Some(paths) = shape_geom(kind, rect.width(), rect.height(), adj) else {
        p.rect(rect, 0.0, fill, line, egui::StrokeKind::Inside);
        return;
    };
    for path in &paths {
        let pts: Vec<_> = path.pts.iter().copied().map(map).collect();
        if pts.is_empty() {
            continue;
        }
        if path.closed {
            p.add(egui::Shape::Path(egui::epaint::PathShape { points: pts, closed: true, fill, stroke: line.into() }));
        } else {
            p.add(egui::Shape::Path(egui::epaint::PathShape { points: pts, closed: false, fill: Color32::TRANSPARENT, stroke: line.into() }));
        }
    }
    for line_pts in detail_lines(kind) {
        let pts: Vec<_> = line_pts.iter().copied().map(map).collect();
        if pts.len() >= 2 {
            p.add(egui::Shape::Path(egui::epaint::PathShape { points: pts, closed: false, fill: Color32::TRANSPARENT, stroke: line.into() }));
        }
    }
}

/// The gallery-only detail lines (unit coordinates): the decorative marks that tell apart
/// shapes whose outline is the same (the twelve action buttons are all rounded rectangles)
/// or that the shared geometry leaves out as decoration (a smiley's face). These are never
/// part of the shape's outline, fill or wrap. Real geometric parts — NoSmoking's bar, a
/// cube's inner edges, a frame's inner rectangle, a folded corner's fold, a striped
/// arrow's stripes — live in the shared geometry instead, so the canvas, PDF and DOCX
/// render them too.
fn detail_lines(kind: ShapeKind) -> Vec<Vec<(f32, f32)>> {
    let line = |pts: &[(f32, f32)]| pts.to_vec();
    match kind {
        ShapeKind::ActionButtonBlank => vec![],
        ShapeKind::ActionButtonHome => {
            vec![line(&[(0.3, 0.7), (0.3, 0.4), (0.5, 0.25), (0.7, 0.4), (0.7, 0.7)]), line(&[(0.42, 0.7), (0.42, 0.55), (0.58, 0.55), (0.58, 0.7)])]
        }
        ShapeKind::ActionButtonHelp => {
            vec![line(&[(0.38, 0.32), (0.5, 0.25), (0.62, 0.32), (0.55, 0.45), (0.5, 0.55), (0.5, 0.62)]), line(&[(0.5, 0.72), (0.5, 0.75)])]
        }
        ShapeKind::ActionButtonInformation => vec![line(&[(0.5, 0.28), (0.5, 0.32)]), line(&[(0.5, 0.42), (0.5, 0.72)])],
        ShapeKind::ActionButtonForwardNext => vec![line(&[(0.3, 0.3), (0.62, 0.5), (0.3, 0.7)]), line(&[(0.7, 0.3), (0.7, 0.7)])],
        ShapeKind::ActionButtonBackPrevious => vec![line(&[(0.7, 0.3), (0.38, 0.5), (0.7, 0.7)]), line(&[(0.3, 0.3), (0.3, 0.7)])],
        ShapeKind::ActionButtonEnd => vec![line(&[(0.3, 0.3), (0.7, 0.3), (0.7, 0.7), (0.3, 0.7)]), line(&[(0.3, 0.3), (0.3, 0.7)])],
        ShapeKind::ActionButtonBeginning => vec![line(&[(0.3, 0.3), (0.7, 0.3), (0.7, 0.7), (0.3, 0.7)]), line(&[(0.7, 0.3), (0.7, 0.7)])],
        ShapeKind::ActionButtonReturn => {
            vec![line(&[(0.7, 0.3), (0.35, 0.3), (0.35, 0.7), (0.55, 0.7)]), line(&[(0.35, 0.3), (0.2, 0.45), (0.35, 0.6)])]
        }
        ShapeKind::ActionButtonDocument => vec![
            line(&[(0.35, 0.25), (0.6, 0.25), (0.65, 0.3), (0.65, 0.75), (0.35, 0.75), (0.35, 0.25)]),
            line(&[(0.42, 0.4), (0.58, 0.4)]),
            line(&[(0.42, 0.5), (0.58, 0.5)]),
            line(&[(0.42, 0.6), (0.58, 0.6)]),
        ],
        ShapeKind::ActionButtonSound => vec![
            line(&[(0.3, 0.45), (0.42, 0.45), (0.42, 0.55), (0.3, 0.55), (0.3, 0.45)]),
            line(&[(0.5, 0.38), (0.6, 0.28)]),
            line(&[(0.5, 0.5), (0.65, 0.5)]),
            line(&[(0.5, 0.62), (0.6, 0.72)]),
        ],
        ShapeKind::ActionButtonMovie => vec![
            line(&[(0.28, 0.3), (0.72, 0.3), (0.72, 0.7), (0.28, 0.7), (0.28, 0.3)]),
            line(&[(0.42, 0.38), (0.42, 0.62), (0.62, 0.5), (0.42, 0.38)]),
        ],
        ShapeKind::SmileyFace => vec![line(&[(0.35, 0.35), (0.35, 0.42)]), line(&[(0.65, 0.35), (0.65, 0.42)]), line(&[(0.3, 0.62), (0.7, 0.62)])],
        _ => vec![],
    }
}

/// The Shapes gallery: a scrollable grouped grid of icon-only tiles. Picking a tile arms
/// the shape tool; nothing is inserted on click.
pub fn shapes_menu(ui: &mut Ui, app: &mut WordApp) {
    let t = Tokens::get(ui.ctx());
    let fill = t.input;
    let stroke = t.text_dim;
    egui::ScrollArea::vertical().id_salt("shapes_gallery").max_height(480.0).show(ui, |ui| {
        ui.set_min_width(240.0);
        for cat in ShapeCategory::ALL {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(tl!(cat.label())).strong().small());
            egui::Grid::new(format!("shapes_{}", cat.label())).spacing(vec2(4.0, 4.0)).show(ui, |ui| {
                let mut n = 0;
                for kind in ShapeKind::ALL.iter().copied().filter(|k| k.category() == Some(cat)) {
                    tile(ui, app, kind, fill, stroke, &t);
                    n += 1;
                    if n % 4 == 0 {
                        ui.end_row();
                    }
                }
            });
        }
        ui.separator();
        crate::ribbon::mi(ui, app, "New Drawing Canvas", "insert.canvas", json!({}));
        // Arrow connectors: the same connector, drawn with an arrowhead at the end.
        for (label, kind) in [
            ("Straight Arrow Connector", ShapeKind::StraightConnector),
            ("Elbow Arrow Connector", ShapeKind::ElbowConnector),
            ("Curved Arrow Connector", ShapeKind::CurvedConnector),
        ] {
            let serde = serde_json::to_value(kind).unwrap_or(json!("rectangle"));
            let label = tl!(label);
            let resp = ui.add(egui::Button::new(label).min_size(vec2(200.0, 0.0)));
            if resp.clicked() {
                let _ = app.run("draw.shapeTool", json!({"kind": serde, "arrow": "end"}));
                ui.close();
            }
        }
    });
}

/// One gallery tile: just the shape's icon, centred in a compact square tile. The name
/// shows only on hover. Clicking arms the shape tool.
fn tile(ui: &mut Ui, app: &mut WordApp, kind: ShapeKind, fill: Color32, stroke: Color32, t: &Tokens) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(54.0, 54.0), egui::Sense::click());
    ui.painter().rect(
        rect,
        3.0,
        if resp.hovered() { t.hover } else { t.input },
        egui::Stroke::new(1.0, if resp.hovered() { t.accent } else { t.border }),
        egui::StrokeKind::Inside,
    );
    let icon = Rect::from_center_size(rect.center(), vec2(36.0, 36.0));
    shape_icon(ui.painter(), icon, kind, fill, stroke);
    let resp = resp.on_hover_text(kind.display_name());
    if resp.clicked() {
        let serde = serde_json::to_value(kind).unwrap_or(json!("rectangle"));
        let _ = app.run("draw.shapeTool", json!({"kind": serde}));
        ui.close();
    }
}

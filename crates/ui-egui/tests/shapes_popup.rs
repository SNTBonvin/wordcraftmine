//! The Shapes popup draws no stray lines: opening Insert › Shapes must not paint a
//! large diagonal anywhere (user report: "a huge slanted line while opening the shapes
//! popup"). The gallery's only diagonals are the tile icons themselves (36×36), so no
//! line/path shape in the popup frame may span more than a tile in both dimensions.

use wordcraft_ui_egui::{Services, WordApp};

#[test]
fn shapes_popup_has_no_stray_diagonal() {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default());
    // Drive the tab switch the way the UI does (the ribbon tabs carry no accesskit label).
    a.run("ui.tab", serde_json::json!({"tab": "Insert"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    h.run_steps(4);

    // The Shapes menu_button sets no accesskit label: find its text in the shape tree and
    // click inside it.
    let mut pos = None;
    for clipped in &h.output().shapes {
        if let egui::Shape::Text(ts) = &clipped.shape
            && ts.galley.text() == "Shapes"
        {
            pos = Some(ts.pos + egui::vec2(10.0, 8.0));
            break;
        }
    }
    let pos = pos.expect("Shapes button text in the shape tree");
    h.event(egui::Event::PointerMoved(pos));
    h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.step();
    h.step();

    // The popup is open: its tiles are in the shape tree (the Line tile's diagonal).
    assert!(h.output().shapes.iter().any(|c| matches!(&c.shape, egui::Shape::Path(ps) if ps.points.len() == 2)), "the popup rendered its tile icons");
    for clipped in &h.output().shapes {
        let (x0, y0, x1, y1) = match &clipped.shape {
            egui::Shape::LineSegment { points, .. } => {
                (points[0].x.min(points[1].x), points[0].y.min(points[1].y), points[0].x.max(points[1].x), points[0].y.max(points[1].y))
            }
            egui::Shape::Path(ps) => {
                let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                for p in &ps.points {
                    x0 = x0.min(p.x);
                    y0 = y0.min(p.y);
                    x1 = x1.max(p.x);
                    y1 = y1.max(p.y);
                }
                (x0, y0, x1, y1)
            }
            _ => continue,
        };
        // Tiles are 36px with 24px icons: nothing may span more than a tile in both dims.
        assert!(x1 - x0 <= 40.0 || y1 - y0 <= 40.0, "stray diagonal spanning ({x0:.0},{y0:.0})-({x1:.0},{y1:.0}) when the Shapes popup is open");
    }
}

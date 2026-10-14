//! Dragging an object shows the live preview: the object moves in the document and the
//! text re-wraps around its current position while the pointer is still down — not only
//! after the drop. The whole drag is one undo step, and Escape cancels it.

use wordcraft_ui_egui::{Services, WordApp};

/// The laid-out rect of the only shape, in document points.
fn shape_rect(app: &mut WordApp) -> (usize, f32, f32, f32, f32) {
    let layout = app.session.layout();
    let o = layout.find_object(0, |o| o.text_box.is_none()).expect("the drawn shape in the layout");
    (o.page, o.rect.x, o.rect.y, o.rect.w, o.rect.h)
}

/// The page's screen rect, from the rendered output: the largest white filled rect.
fn page_screen_rect(h: &egui_kittest::Harness<'_, WordApp>) -> egui::Rect {
    let mut best: Option<egui::Rect> = None;
    for clipped in &h.output().shapes {
        if let egui::Shape::Rect(rs) = &clipped.shape
            && rs.fill == egui::Color32::WHITE
        {
            let r = rs.rect;
            if best.is_none_or(|b| r.width() * r.height() > b.width() * b.height()) {
                best = Some(r);
            }
        }
    }
    best.expect("a page rect in the rendered output")
}

fn press(h: &mut egui_kittest::Harness<'_, WordApp>, pos: egui::Pos2) {
    h.event(egui::Event::PointerMoved(pos));
    h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
}

fn move_to(h: &mut egui_kittest::Harness<'_, WordApp>, pos: egui::Pos2) {
    h.event(egui::Event::PointerMoved(pos));
    h.step();
}

fn release(h: &mut egui_kittest::Harness<'_, WordApp>, pos: egui::Pos2) {
    h.event(egui::Event::PointerMoved(pos));
    h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.step();
}

#[test]
fn drag_moves_object_live_and_undoes_in_one_step() {
    let mut session = wordcraft_engine::Session::new(wordcraft_doc::Document::new());
    session.run("text.insert", &serde_json::json!({"text": "Some text to wrap around the shape. ".repeat(20)})).unwrap();
    // A floating rectangle with square wrapping, in front of the text, at (100, 100).
    session.run("draw.shape", &serde_json::json!({"kind": "rectangle", "page": 0, "x": 100.0, "y": 100.0, "width": 120.0, "height": 80.0})).unwrap();
    session.run("arrange.wrap", &serde_json::json!({"wrap": "square"})).unwrap();
    session.run("select.collapse", &serde_json::json!({"end": true})).unwrap();
    let (x0, y0) = (100.0, 100.0);
    let app = WordApp::new(session, Services::default());
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);

    // The shape's screen centre, from its document rect and the page's screen rect.
    let page_rect = page_screen_rect(&h);
    let layout = h.state_mut().session.layout();
    let page_w = layout.pages[0].w;
    let scale = page_rect.width() / page_w;
    let start = egui::pos2(page_rect.min.x + (x0 + 60.0) * scale, page_rect.min.y + (y0 + 40.0) * scale);

    // Press on the shape, then drag 60pt right and 40pt down — still holding.
    press(&mut h, start);
    let dest = egui::pos2(start.x + 60.0 * scale, start.y + 40.0 * scale);
    move_to(&mut h, dest);
    move_to(&mut h, dest);

    // Live: the document already holds the dragged position before the release.
    let (_, x1, y1, _, _) = shape_rect(h.state_mut());
    assert!(
        (x1 - (x0 + 60.0)).abs() < 2.0 && (y1 - (y0 + 40.0)).abs() < 2.0,
        "live drag moved the shape to ({x1:.1}, {y1:.1}), expected ({:.1}, {:.1})",
        x0 + 60.0,
        y0 + 40.0
    );

    // Release, then one undo returns it to the start.
    release(&mut h, dest);
    let (_, x2, y2, _, _) = shape_rect(h.state_mut());
    assert!((x2 - (x0 + 60.0)).abs() < 2.0 && (y2 - (y0 + 40.0)).abs() < 2.0, "drop kept the shape at ({x2:.1}, {y2:.1})");
    h.state_mut().run("edit.undo", serde_json::json!({})).unwrap();
    let (_, x3, y3, _, _) = shape_rect(h.state_mut());
    assert!((x3 - x0).abs() < 1.0 && (y3 - y0).abs() < 1.0, "one undo returned the shape to ({x3:.1}, {y3:.1}), expected ({x0:.1}, {y0:.1})");
}

//! Pictures, shapes and text boxes on the canvas: click one to select it, drag it to move it,
//! drag a handle to resize it, drag the rotation handle to turn it (Shift: 15° steps), arrow keys
//! nudge a floating one. The frame itself is [`crate::frame`]; changes go through
//! `arrange.bounds` / `arrange.rotation` / `arrange.nudge`, so they're undoable and scriptable like
//! everything else.
//!
//! While dragging, nothing is laid out or rendered: the preview is the object's own pixels taken
//! from its page's cached texture. The drop is one command, one relayout.
//!
//! A selected connector shows a handle at each end instead of a frame: dragging one onto a shape
//! glues it to the nearest connection site there (`shape.connect`), elsewhere leaves it free.

use egui::{Color32, Painter, Pos2, Rect, Ui};
use serde_json::json;
use wordcraft_doc::para::{InlineObject, OBJ};
use wordcraft_doc::{PartKind, Pos, StoryRef};
use wordcraft_layout::DocLayout;
use wordcraft_layout::hit::ObjectHit;

use crate::WordApp;
use crate::frame::{self, Drag, Grab};
use crate::theme::Tokens;

/// The band along a text box's border that grabs the box rather than its text, screen points.
const EDGE: f32 = 5.0;
/// How opaque the drag preview of an object's pixels is (0–255).
const PREVIEW_OPACITY: u8 = 170;
/// Arrow-key nudge, points (Ctrl/Alt: fine).
const NUDGE: f32 = 6.0;
const NUDGE_FINE: f32 = 1.0;

/// An object being dragged by its frame.
pub struct ObjectDrag {
    object: ObjectHit,
    drag: Drag,
    /// Smallest size it can be resized to, points.
    min: f32,
    picture: bool,
    /// Dragging a connector's end (false: its start, true: its end) to this page point.
    end: Option<(bool, (f32, f32))>,
    /// A handle of a turned text box being edited (drawn straight, Word check P): the drag works
    /// on the straight frame, a turn adds to the box's real angle (`base`), and the text stays
    /// edited (`keep`, the selection to go back to).
    base: f32,
    keep: Option<wordcraft_engine::Selection>,
}

/// How near (screen points) a press must be to a connector's end to grab it.
const END_GRAB: f32 = 7.0;

/// The selected connector's start and end on its page (points), and its layout.
fn connector_ends(app: &WordApp, layout: &DocLayout) -> Option<(ObjectHit, (f32, f32), (f32, f32))> {
    let (pos, InlineObject::Shape { kind, float, .. }) = selected(app)? else { return None };
    if !kind.is_connector() {
        return None;
    }
    let (flip_h, flip_v) = (float.flip_h, float.flip_v);
    let o = layout.object(&pos, app.session.page_hint)?;
    let (a, b) = wordcraft_doc::connector::ends(o.rect.x, o.rect.y, o.rect.w, o.rect.h, flip_h, flip_v);
    Some((o, a, b))
}

fn point_on_screen(pages: &[Rect], scale: f32, page: usize, (x, y): (f32, f32)) -> Option<Pos2> {
    pages.get(page).map(|p| egui::pos2(p.min.x + x * scale, p.min.y + y * scale))
}

/// The selected connector's end under `at` (screen): which (false: start), and the connector.
fn connector_end_at(app: &WordApp, layout: &DocLayout, pages: &[Rect], scale: f32, at: Pos2) -> Option<(ObjectHit, bool)> {
    let (o, a, b) = connector_ends(app, layout)?;
    let near = |p: (f32, f32)| point_on_screen(pages, scale, o.page, p).is_some_and(|s| s.distance(at) <= END_GRAB);
    if near(b) {
        Some((o, true))
    } else if near(a) {
        Some((o, false))
    } else {
        None
    }
}

/// The selected object: the selection is exactly one picture, shape or text box, in the body
/// or in a header/footer.
pub fn selected(app: &WordApp) -> Option<(Pos, &InlineObject)> {
    wordcraft_engine::cmd::objects::object_selection(&app.session).filter(|(p, _)| selectable_story(app, p.story))
}

/// Whether objects of `story` can be selected and dragged on the canvas: the body and
/// headers/footers.
fn selectable_story(app: &WordApp, story: StoryRef) -> bool {
    match story {
        StoryRef::Body => true,
        StoryRef::Part(id) => app.session.doc.parts.get(&id).is_some_and(|p| matches!(p.kind, PartKind::Header | PartKind::Footer)),
    }
}

/// The story whose objects the canvas handles: the header/footer being edited, else the body.
fn active_story(app: &WordApp, layout: &DocLayout) -> StoryRef {
    let story = crate::canvas::text_box_host(app, layout).unwrap_or(app.session.sel.focus.story);
    match story {
        StoryRef::Body => StoryRef::Body,
        StoryRef::Part(id) if selectable_story(app, StoryRef::Part(id)) => StoryRef::Part(id),
        // Notes and text-box text: no top-level objects live here, so grabs find nothing and
        // the press falls through to text handling.
        other => other,
    }
}

/// Whether the object under `hit` keeps its place and size: charts and diagrams can be selected
/// and deleted, not moved or resized (they aren't written back on save yet), so they get no
/// handles, no move cursor and no drag.
fn fixed(app: &WordApp, hit: &ObjectHit) -> bool {
    let pos = hit.pos();
    matches!(app.session.doc.para_at(&pos).and_then(|p| p.object_at(pos.off)), Some(InlineObject::Graphic { .. }))
}

/// The story of the selected text box.
fn selected_text_box(app: &WordApp) -> Option<u32> {
    match selected(app)? {
        (_, InlineObject::Shape { story: Some(id), .. }) if app.session.doc.parts.get(id).is_some_and(|p| p.kind == PartKind::TextBox) => Some(*id),
        _ => None,
    }
}

/// The object showing a frame, and whether its text is being edited (caret in a text box).
pub fn active(app: &WordApp, layout: &DocLayout) -> Option<(ObjectHit, bool)> {
    let hint = app.session.page_hint;
    if let Some((pos, _)) = selected(app) {
        return layout.object(&pos, hint).map(|o| (o, false));
    }
    match app.session.sel.focus.story {
        StoryRef::Part(id) if crate::canvas::in_text_box(app) => layout.text_box(id, hint).map(|o| (o, true)),
        _ => None,
    }
}

/// A turned text box whose text is being edited, so drawn straight.
fn straightened(o: &ObjectHit, editing: bool) -> bool {
    editing && crate::canvas::turned(o)
}

fn screen(pages: &[Rect], scale: f32, page: usize, r: wordcraft_geom::Rect) -> Option<Rect> {
    pages.get(page).map(|p| frame::to_screen(*p, scale, r))
}

/// Pressing, dragging, releasing or double-clicking on an object (or a frame handle). Returns
/// true when it was about an object, so the text doesn't also get it.
pub fn pointer(app: &mut WordApp, ui: &Ui, resp: &egui::Response, pages: &[Rect], layout: &DocLayout, scale: f32, at: Pos2) -> bool {
    if let Some(mut d) = app.canvas.obj_drag.take() {
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            return true; // cancelled: dropped without a change
        }
        if let Some((which, _)) = d.end {
            if ui.input(|i| i.pointer.primary_down()) {
                if let Some((_, x, y)) = crate::canvas::page_at(pages, layout, scale, at).filter(|(p, ..)| *p == d.object.page) {
                    d.end = Some((which, (x, y)));
                    d.drag.moved = true;
                }
                app.canvas.obj_drag = Some(d);
            } else if d.drag.moved
                && let Some((_, (x, y))) = d.end
            {
                let key = if which { "end" } else { "start" };
                let _ = app.run("shape.connect", json!({key: {"x": x, "y": y, "page": d.object.page}}));
            }
            return true;
        }
        if ui.input(|i| i.pointer.primary_down()) {
            let shift = ui.input(|i| i.modifiers.shift);
            if d.drag.grab == Grab::Rotate {
                d.drag.turn_to(at, pages, scale, shift);
            } else {
                // Pictures keep their proportions from a corner unless Shift; shapes only with Shift.
                d.drag.update(at, pages, scale, d.min, d.picture != shift);
            }
            app.canvas.obj_drag = Some(d);
        } else {
            drop(app, &d);
        }
        return true;
    }
    let pressed = ui.input(|i| i.pointer.primary_pressed()) && resp.contains_pointer();
    if app.canvas.link_from.is_some() && (pressed || ui.input(|i| i.key_pressed(egui::Key::Escape))) {
        link_click(app, layout, pages, scale, at, pressed);
        return true;
    }
    let multi = resp.double_clicked() || resp.triple_clicked();
    if !(pressed || multi) {
        return false;
    }
    // A selected connector's end: drag it to glue it.
    if pressed && let Some((object, which)) = connector_end_at(app, layout, pages, scale, at) {
        let drag = Drag::new(Grab::Move, object.page, object.rect, 0.0, at);
        app.canvas.obj_drag = Some(ObjectDrag { drag, object, min: 0.0, picture: false, end: Some((which, (0.0, 0.0))), base: 0.0, keep: None });
        return true;
    }
    let Some((object, grab)) = grab_at(app, layout, pages, scale, at) else { return false };
    // Shift/Ctrl+click adds an object to the selected ones (or takes it out), for Group.
    let adding = ui.input(|i| i.modifiers.shift || i.modifiers.command);
    if pressed && adding && grab == Grab::Move && selected(app).is_some() {
        let _ = app.run("select.addObject", json!({"pos": object.pos()}));
        return true;
    }
    // A handle of a turned text box being edited: the text stays edited and the box straight.
    let straight_handle = grab != Grab::Move && active(app, layout).is_some_and(|(o, editing)| straightened(&o, editing) && o.pos() == object.pos());
    if pressed && straight_handle {
        let min = app
            .session
            .doc
            .para(object.story, &object.path)
            .and_then(|p| p.object_at(object.off))
            .map_or(0.0, wordcraft_engine::cmd::objects::min_size);
        let drag = Drag::new(grab, object.page, object.rect, 0.0, at);
        let (base, keep) = (object.spin.deg, Some(app.session.sel.clone()));
        app.canvas.obj_drag = Some(ObjectDrag { drag, object, min, picture: false, end: None, base, keep });
        return true;
    }
    if pressed {
        let pos = object.pos();
        let end = Pos { off: pos.off + OBJ.len_utf8(), ..pos.clone() };
        let _ = app.run("select.range", json!({"anchor": pos, "focus": end}));
        let (min, picture) = match selected(app) {
            Some((_, o)) => (wordcraft_engine::cmd::objects::min_size(o), matches!(o, InlineObject::Image { .. })),
            None => return true,
        };
        // Objects in table cells can be selected and resized, not moved; charts neither.
        if !fixed(app, &object) && (grab != Grab::Move || movable(app, &object)) {
            app.canvas.obj_drag = Some(ObjectDrag {
                drag: Drag::new(grab, object.page, object.rect, object.spin.deg, at),
                object,
                min,
                picture,
                end: None,
                base: 0.0,
                keep: None,
            });
        }
    }
    true
}

/// Create Link's click: link the box it started from to the text box under `at` (anywhere on it),
/// if `pressed`; either way, linking is over.
fn link_click(app: &mut WordApp, layout: &DocLayout, pages: &[Rect], scale: f32, at: Pos2, pressed: bool) {
    let Some(from) = app.canvas.link_from.take() else { return };
    if !pressed {
        return;
    }
    let target = crate::canvas::page_at(pages, layout, scale, at).and_then(|(page, x, y)| {
        layout.find_object(page, |o| o.page == page && o.text_box.is_some() && o.rect.contains(wordcraft_geom::Point::new(x, y)))
    });
    let Some(target) = target else {
        app.status(tl!("Not linked: click a text box"));
        return;
    };
    let end = Pos { off: from.off + OBJ.len_utf8(), ..from.clone() };
    let _ = app.run("select.range", json!({"anchor": from, "focus": end}));
    if let Err(e) = app.run("shape.link", json!({"target": target.pos()})) {
        app.status(e);
    }
}

/// Right-click at `at`: the picture, shape or text box under it becomes the selection (as a click
/// would, without a drag), so the context menu acts on it. Returns whether there was one.
pub fn select_at(app: &mut WordApp, layout: &DocLayout, pages: &[Rect], scale: f32, at: Pos2) -> bool {
    if crate::canvas::editing_header_footer(app, layout) {
        return false;
    }
    let Some((object, _)) = grab_at(app, layout, pages, scale, at) else { return false };
    let pos = object.pos();
    // Already selected (alone or among several picked for Group): keep the selection.
    if !wordcraft_engine::cmd::objects::picked_objects(&app.session).contains(&pos) {
        let end = Pos { off: pos.off + OBJ.len_utf8(), ..pos.clone() };
        let _ = app.run("select.range", json!({"anchor": pos, "focus": end}));
    }
    true
}

/// What a press at `at` would grab: a handle of the shown frame, else an object of the story
/// being edited (body, or the header/footer).
fn grab_at(app: &WordApp, layout: &DocLayout, pages: &[Rect], scale: f32, at: Pos2) -> Option<(ObjectHit, Grab)> {
    let story = active_story(app, layout);
    // A turned text box being edited is drawn straight, without handles (its handles would
    // turn and size it from the straight frame); its border still moves it.
    // A turned text box being edited is drawn straight, its handles on the straight frame.
    let handle = active(app, layout).filter(|(o, _)| o.story == story && !fixed(app, o)).and_then(|(o, editing)| {
        let f = screen(pages, scale, o.page, o.rect)?;
        let deg = if straightened(&o, editing) { 0.0 } else { o.spin.deg };
        if frame::rotate_handle_at(f, deg, at) {
            return Some((o, Grab::Rotate));
        }
        frame::handle_at(f, deg, at).map(|h| (o, Grab::Resize(h)))
    });
    handle.or_else(|| {
        let (page, x, y) = crate::canvas::page_at(pages, layout, scale, at)?;
        // Selectable: body objects (table cells too) or the header/footer's own objects;
        // notes' objects aren't yet.
        let straight = crate::canvas::straight_box(app, layout);
        layout.object_at_shown(page, x, y, EDGE / scale.max(0.01), story, straight).map(|o| (o, Grab::Move))
    })
}

/// Objects that can be dragged to a new place: those in the own paragraphs of the body or of a
/// header/footer.
fn movable(app: &WordApp, o: &ObjectHit) -> bool {
    o.path.depth() == 0 && selectable_story(app, o.story)
}

/// Commit a finished drag as one command.
fn drop(app: &mut WordApp, d: &ObjectDrag) {
    if !d.drag.moved {
        return;
    }
    // A handle of an edited turned box: the commands act on the box, then the text is edited
    // again.
    if let Some(keep) = &d.keep {
        let pos = d.object.pos();
        let end = Pos { off: pos.off + OBJ.len_utf8(), ..pos.clone() };
        let _ = app.run("select.range", json!({"anchor": pos, "focus": end}));
        commit(app, d);
        app.session.sel = keep.clone();
        return;
    }
    commit(app, d);
}

/// Run a finished drag's command.
fn commit(app: &mut WordApp, d: &ObjectDrag) {
    let r = d.drag.rect;
    let params = match d.drag.grab {
        Grab::Rotate => {
            let deg = wordcraft_geom::normalize_degrees(d.base + d.drag.deg);
            let _ = app.run("arrange.rotation", json!({"degrees": deg}));
            return;
        }
        Grab::Move => json!({"page": d.drag.to_page, "x": r.x, "y": r.y}),
        // An inline object stays in the text; a floating one keeps its far edges put.
        Grab::Resize(_) if d.object.floating() && movable(app, &d.object) => {
            json!({"width": r.w, "height": r.h, "page": d.drag.page, "x": r.x, "y": r.y})
        }
        Grab::Resize(_) => json!({"width": r.w, "height": r.h}),
    };
    let _ = app.run("arrange.bounds", params);
}

/// Draw the frame of the active object, or the drag preview.
pub fn paint(app: &WordApp, painter: &Painter, t: &Tokens, layout: &DocLayout, pages: &[Rect], scale: f32) {
    if let Some(d) = app.canvas.obj_drag.as_ref().filter(|d| d.end.is_some()) {
        paint_end_drag(app, d, painter, t, layout, pages, scale);
        return;
    }
    if let Some((o, a, b)) = connector_ends(app, layout) {
        // A connector: a handle at each end (filled when glued).
        let glued = match selected(app) {
            Some((_, InlineObject::Shape { extra, .. })) => (extra.start.is_some(), extra.end.is_some()),
            _ => (false, false),
        };
        for (p, on) in [(a, glued.0), (b, glued.1)] {
            if let Some(c) = point_on_screen(pages, scale, o.page, p) {
                painter.circle(c, 4.5, if on { t.accent } else { Color32::WHITE }, egui::Stroke::new(1.2, t.accent));
            }
        }
        return;
    }
    if let Some(d) = app.canvas.obj_drag.as_ref().filter(|d| d.drag.moved) {
        let Some(to) = screen(pages, scale, d.drag.to_page, d.drag.rect) else { return };
        if d.drag.grab == Grab::Rotate {
            frame::paint_turning(painter, t, to, d.drag.deg, d.base + d.drag.deg);
            return;
        }
        // The object's own pixels, from its page's cached texture: moved, or scaled for an
        // unturned picture. Resizing reflows a text box (and redraws a shape), so that's just the
        // new outline.
        let turned = !d.object.spin.is_identity();
        let pixels = (d.picture && !turned) || d.drag.grab == Grab::Move;
        if !pixels {
            frame::paint_outline(painter, t, to, d.drag.deg);
        } else if let (Some(tex), Some(pg)) = (app.canvas.page_texture(d.object.page), layout.pages.get(d.object.page)) {
            // A turned object's pixels fill its rotated bounds, which move with it.
            let r = d.object.bounds();
            let (dx, dy) = (d.drag.rect.x - d.drag.start.x, d.drag.rect.y - d.drag.start.y);
            let moved = wordcraft_geom::Rect::new(r.x + dx, r.y + dy, r.w, r.h);
            let to = if turned { screen(pages, scale, d.drag.to_page, moved).unwrap_or(to) } else { to };
            let (w, h) = (pg.w.max(1.0), pg.h.max(1.0));
            let uv = Rect::from_min_max(egui::pos2(r.x / w, r.y / h), egui::pos2(r.right() / w, r.bottom() / h));
            painter.image(tex.id(), to, uv, Color32::from_white_alpha(PREVIEW_OPACITY));
        }
        frame::paint(painter, t, to, d.drag.deg, false, false, false);
        return;
    }
    // The others selected along with it (Shift+click): their outlines.
    for p in &app.session.also_selected {
        if let Some(o) = layout.object(p, app.session.page_hint)
            && let Some(f) = screen(pages, scale, o.page, o.rect)
        {
            frame::paint(painter, t, f, o.spin.deg, false, false, false);
        }
    }
    if let Some((o, editing)) = active(app, layout)
        && let Some(f) = screen(pages, scale, o.page, o.rect)
    {
        // Handles where it can be resized and turned (body and header/footer objects,
        // not charts): else a plain frame.
        let handles = selectable_story(app, o.story) && !fixed(app, &o);
        // A turned text box being edited is drawn straight, and so is its frame (Word check P).
        let deg = if straightened(&o, editing) { 0.0 } else { o.spin.deg };
        frame::paint(painter, t, f, deg, editing, handles, handles);
    }
}

/// A connector's end being dragged: a line from its other end to the pointer, and the connection
/// sites of the shapes near the pointer (the one it would glue to, filled).
fn paint_end_drag(app: &WordApp, d: &ObjectDrag, painter: &Painter, t: &Tokens, layout: &DocLayout, pages: &[Rect], scale: f32) {
    let Some((which, at)) = d.end.filter(|_| d.drag.moved) else { return };
    let Some((_, a, b)) = connector_ends(app, layout) else { return };
    let fixed = if which { a } else { b };
    let snap = wordcraft_engine::cmd::connectors::SNAP;
    let mut best: Option<((f32, f32), f32)> = None;
    if let Some(pg) = layout.pages.get(d.object.page) {
        for it in &pg.items {
            let wordcraft_layout::Placed::Object { rect, spin, story: StoryRef::Body, path, off, .. } = it else { continue };
            let pos = Pos { story: StoryRef::Body, path: path.clone(), off: *off };
            let Some(InlineObject::Shape { kind, .. }) = app.session.doc.para_at(&pos).and_then(|p| p.object_at(pos.off)) else { continue };
            if kind.is_connector() || pos == d.object.pos() {
                continue;
            }
            let r = [rect.x, rect.y, rect.w, rect.h];
            // Sites of shapes the pointer is near.
            if at.0 < rect.x - 4.0 * snap || at.0 > rect.right() + 4.0 * snap || at.1 < rect.y - 4.0 * snap || at.1 > rect.bottom() + 4.0 * snap {
                continue;
            }
            for (i, _) in wordcraft_doc::connector::sites(*kind, rect.w, rect.h).iter().enumerate() {
                let Some(p) = wordcraft_doc::connector::site_point(*kind, r, *spin, i as u32) else { continue };
                let dist = ((p.0 - at.0).powi(2) + (p.1 - at.1).powi(2)).sqrt();
                if let Some(c) = point_on_screen(pages, scale, d.object.page, p) {
                    painter.circle_stroke(c, 3.5, egui::Stroke::new(1.0, t.accent));
                }
                if dist <= snap && best.is_none_or(|b| dist < b.1) {
                    best = Some((p, dist));
                }
            }
        }
    }
    let to = best.map_or(at, |b| b.0);
    if let (Some(f), Some(e)) = (point_on_screen(pages, scale, d.object.page, fixed), point_on_screen(pages, scale, d.object.page, to)) {
        painter.line_segment([f, e], egui::Stroke::new(1.5, t.accent));
        painter.circle(e, 4.5, if best.is_some() { t.accent } else { Color32::WHITE }, egui::Stroke::new(1.2, t.accent));
    }
}

/// The pointer over an object or a handle (or dragging): its cursor.
pub fn cursor(app: &WordApp, layout: &DocLayout, pages: &[Rect], scale: f32, at: Pos2) -> Option<egui::CursorIcon> {
    if let Some(d) = &app.canvas.obj_drag {
        return Some(if d.end.is_some() { egui::CursorIcon::Crosshair } else { d.drag.grab.cursor() });
    }
    if connector_end_at(app, layout, pages, scale, at).is_some() {
        return Some(egui::CursorIcon::Crosshair);
    }
    grab_at(app, layout, pages, scale, at).map(|(o, g)| if fixed(app, &o) { egui::CursorIcon::Default } else { g.cursor() })
}

/// Keys for a selected object: arrows nudge a floating one; typing or Enter goes into a text box.
/// Returns true if handled.
pub fn key(app: &mut WordApp, key: egui::Key, m: egui::Modifiers) -> bool {
    if selected(app).is_none() {
        return false;
    }
    let step = if m.ctrl || m.alt || m.command { NUDGE_FINE } else { NUDGE };
    let (dx, dy) = match key {
        egui::Key::ArrowLeft => (-step, 0.0),
        egui::Key::ArrowRight => (step, 0.0),
        egui::Key::ArrowUp => (0.0, -step),
        egui::Key::ArrowDown => (0.0, step),
        egui::Key::Enter => return enter_text_box(app),
        _ => return false,
    };
    // Inline objects move with the text: arrows move the caret as usual.
    app.run("arrange.nudge", json!({"dx": dx, "dy": dy})).is_ok()
}

/// With a text box selected, put the caret at the end of its text (typing goes there).
pub fn enter_text_box(app: &mut WordApp) -> bool {
    let Some(id) = selected_text_box(app) else { return false };
    // A box linked from another shows (and edits) the first box's story.
    let id = app.session.doc.text_box_chain(id).and_then(|c| c.first().copied()).unwrap_or(id);
    let end = app.session.doc.end_of(StoryRef::Part(id));
    app.run("caret.set", json!({"pos": end})).is_ok()
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Arc;

    use wordcraft_doc::para::{InlineObject, ShapeKind};
    use wordcraft_doc::{Document, Pos};

    use super::*;

    /// An app whose first paragraph holds `obj`, and where it's laid out.
    fn app_with(obj: InlineObject) -> (WordApp, ObjectHit) {
        let mut app = WordApp::new(wordcraft_engine::Session::new(Document::new()), crate::Services::default());
        app.session.doc.insert_object(&Pos::body(0, 0), obj, &Default::default()).unwrap();
        let hit = app.session.layout().object(&Pos::body(0, 0), 0).expect("laid out");
        (app, hit)
    }

    #[test]
    fn charts_get_no_handles_or_drag_but_shapes_do() {
        let chart =
            InlineObject::Graphic { w: 200.0, h: 100.0, alt: String::new(), float: Default::default(), graphic: Arc::new(Default::default()) };
        let (app, hit) = app_with(chart);
        assert!(fixed(&app, &hit));
        let shape = InlineObject::Shape {
            kind: ShapeKind::Rectangle,
            w: 50.0,
            h: 50.0,
            fill: None,
            stroke: None,
            stroke_width: 1.0,
            float: Default::default(),
            story: None,
            freeform: None,
            effects: Default::default(),
            extra: Default::default(),
        };
        let (app, hit) = app_with(shape);
        assert!(!fixed(&app, &hit));
    }

    pub(crate) type H = egui_kittest::Harness<'static, WordApp>;

    /// A window showing a 200 × 80 text box turned 30° beside some body text, the caret in the
    /// body; and the box's story.
    pub(crate) fn turned_box_window() -> (H, u32) {
        let mut a = WordApp::new(wordcraft_engine::Session::new(Document::new()), crate::Services::default());
        a.run("text.insert", json!({"text": "Body words beside the box."})).unwrap();
        a.session.sel = wordcraft_engine::Selection::caret(Pos::body(0, 0));
        let id = a.run("insert.textBox", json!({"text": "Turned text here", "width": 200, "height": 80})).unwrap()["story"].as_u64().unwrap() as u32;
        a.run("select.range", json!({"anchor": Pos::body(0, 0), "focus": Pos::body(0, 3)})).unwrap();
        a.run("arrange.wrap", json!({"wrap": "square"})).unwrap();
        a.run("arrange.rotation", json!({"degrees": 30})).unwrap();
        a.session.sel = wordcraft_engine::Selection::caret(Pos::body(0, 10));
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        for _ in 0..6 {
            h.step();
        }
        (h, id)
    }

    /// Page point (`x`, `y`) of the first page on screen.
    pub(crate) fn on_screen(h: &H, x: f32, y: f32) -> Pos2 {
        let (r, s) = (h.state().canvas.page_rects[0], h.state().canvas.scale);
        egui::pos2(r.min.x + x * s, r.min.y + y * s)
    }

    /// The box's frame on screen (unturned).
    pub(crate) fn frame_on_screen(h: &mut H, id: u32) -> Rect {
        let o = h.state_mut().session.layout().text_box(id, 0).expect("the box is laid out");
        frame::to_screen(h.state().canvas.page_rects[0], h.state().canvas.scale, o.rect)
    }

    pub(crate) fn press(h: &mut H, at: Pos2, pressed: bool) {
        h.event(egui::Event::PointerMoved(at));
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
        h.step();
    }

    pub(crate) fn edit(h: &mut H, id: u32) {
        let end = h.state().session.doc.end_of(wordcraft_doc::StoryRef::Part(id));
        h.state_mut().session.sel = wordcraft_engine::Selection::caret(end);
        for _ in 0..3 {
            h.step();
        }
    }

    /// Gap 10: while a turned text box is edited it is drawn straight, so a press on the
    /// straight border the user sees grabs it (to move it, at its real turn).
    #[test]
    fn a_press_on_the_straight_border_of_an_edited_turned_box_grabs_it() {
        let (mut h, id) = turned_box_window();
        edit(&mut h, id);
        let f = frame_on_screen(&mut h, id);
        let s = h.state().canvas.scale;
        // On the top edge, 60 pt right of its middle: far off the turned border.
        let at = egui::pos2(f.center().x + 60.0 * s, f.min.y);
        press(&mut h, at, true);
        let d = h.state().canvas.obj_drag.as_ref().expect("the press grabbed the box");
        assert_eq!(d.drag.grab, Grab::Move);
        assert_eq!(d.object.text_box, Some(id));
        assert!((d.drag.deg - 30.0).abs() < 0.01, "the move keeps its real turn: {}", d.drag.deg);
    }

    /// Word check P: while a turned text box is edited (drawn straight) its resize and rotation
    /// handles are on the straight frame; selected again, they are at its real angle and turning
    /// starts from it.
    #[test]
    fn an_edited_turned_box_shows_handles_on_its_straight_frame() {
        let (mut h, id) = turned_box_window();
        edit(&mut h, id);
        let f = frame_on_screen(&mut h, id);
        for (at, want) in [(frame::rotate_handle(f, 0.0), "rotate"), (f.right_top(), "resize"), (f.right_center(), "resize")] {
            edit(&mut h, id);
            press(&mut h, at, true);
            let grab = h.state().canvas.obj_drag.as_ref().map(|d| d.drag.grab);
            let got = match grab {
                Some(Grab::Rotate) => "rotate",
                Some(Grab::Resize(_)) => "resize",
                _ => "none",
            };
            assert_eq!(got, want, "the straight frame's handle at {at:?}");
            press(&mut h, at, false);
            h.state_mut().canvas.obj_drag = None;
        }
        // Selected (not edited): the rotation handle sits at 30° and a turn starts from 30°.
        let pos = h.state_mut().session.layout().text_box(id, 0).unwrap().pos();
        let end = Pos { off: pos.off + OBJ.len_utf8(), ..pos.clone() };
        h.state_mut().run("select.range", json!({"anchor": pos, "focus": end})).unwrap();
        for _ in 0..3 {
            h.step();
        }
        press(&mut h, frame::rotate_handle(f, 30.0), true);
        let d = h.state().canvas.obj_drag.as_ref().expect("the rotation handle is grabbed");
        assert_eq!(d.drag.grab, Grab::Rotate);
        assert!((d.drag.deg - 30.0).abs() < 0.01, "turning starts from the box's angle: {}", d.drag.deg);
    }

    /// Press at `from`, move to `to` in a few steps, release.
    fn drag(h: &mut H, from: Pos2, to: Pos2) {
        press(h, from, true);
        for k in 1..=4 {
            h.event(egui::Event::PointerMoved(from + (to - from) * (k as f32 / 4.0)));
            h.step();
        }
        press(h, to, false);
        for _ in 0..2 {
            h.step();
        }
    }

    /// The box's size and turn in the document.
    fn box_frame(h: &H) -> (f32, f32, f32) {
        let o = h.state().session.doc.para(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(0)).and_then(|p| p.object_at(0)).cloned();
        let (w, ht, float) = o.as_ref().and_then(|o| o.frame()).map(|(w, ht, f)| (w, ht, f.spin().deg)).expect("the box");
        (w, ht, float)
    }

    /// Resizing an edited turned box from its straight frame: one undo step, wider, still 30°,
    /// and the text stays being edited.
    #[test]
    fn resizing_an_edited_turned_box_keeps_its_turn() {
        let (mut h, id) = turned_box_window();
        edit(&mut h, id);
        let (w0, h0, deg0) = box_frame(&h);
        assert!((w0 - 200.0).abs() < 0.01 && (deg0 - 30.0).abs() < 0.01);
        let depth = h.state().session.undo_depth();
        let f = frame_on_screen(&mut h, id);
        let s = h.state().canvas.scale;
        drag(&mut h, f.right_center(), f.right_center() + egui::vec2(40.0 * s, 0.0));
        let (w, ht, deg) = box_frame(&h);
        assert!((w - 240.0).abs() < 1.0, "40 pt wider: {w}");
        assert!((ht - h0).abs() < 0.01, "as tall: {ht}");
        assert!((deg - 30.0).abs() < 0.01, "still turned 30°: {deg}");
        assert_eq!(h.state().session.undo_depth(), depth + 1, "one undo step");
        assert_eq!(h.state().session.sel.focus.story, wordcraft_doc::StoryRef::Part(id), "still editing its text");
    }

    /// Turning an edited turned box from its straight frame adds the turn to its real angle.
    #[test]
    fn turning_an_edited_turned_box_adds_to_its_angle() {
        let (mut h, id) = turned_box_window();
        edit(&mut h, id);
        let depth = h.state().session.undo_depth();
        let f = frame_on_screen(&mut h, id);
        drag(&mut h, frame::rotate_handle(f, 0.0), frame::rotate_handle(f, 10.0));
        let (.., deg) = box_frame(&h);
        assert!((deg - 40.0).abs() < 1.01, "30° + 10°: {deg}");
        assert_eq!(h.state().session.undo_depth(), depth + 1, "one undo step");
    }
}

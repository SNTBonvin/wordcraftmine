//! Crop mode on the canvas (Picture Format › Size › Crop, `ui.crop`): black crop handles replace
//! the selected picture's resize handles; dragging one trims that side (a corner, two sides)
//! while what stays visible keeps its scale and its place on the page, and dragging back out
//! restores what was cropped away (never past the source's edge). The cropped-away part shows
//! around the picture under a dark dim. Esc, Enter, a click outside the picture or Crop again
//! leave crop mode and keep the crop. A drop is one `picture.crop` with the crop and the new size,
//! so one undo step. Only unturned pictures: a turned one stays out of crop mode (status message).
//!
//! While dragging nothing is laid out: the preview is the picture's own source pixels, the part
//! that would stay visible bright and the rest dimmed.

use egui::{Color32, Painter, Pos2, Rect, Ui, pos2, vec2};
use serde_json::{Value, json};
use wordcraft_doc::Pos;
use wordcraft_doc::para::InlineObject;
use wordcraft_layout::DocLayout;

use crate::WordApp;
use crate::frame::{self, Handle};
use crate::theme::Tokens;

/// The most of the source one side can crop away (as `picture.crop` clamps).
const MAX_SIDE: f32 = 0.45;
/// Pointer travel that turns a press on a handle into a drag (screen points).
const DRAG_START: f32 = 3.0;
/// Crop handles: arm length of a corner's L, length of an edge bar, thickness (screen points).
const ARM: f32 = 11.0;
const BAR: f32 = 16.0;
const THICK: f32 = 3.5;
/// The longest side of the source picture decoded for crop mode (pixels).
const SOURCE_MAX_PX: u32 = 1024;

/// Where a crop handle drag leaves a picture: its crop (left, top, right, bottom fractions of the
/// source), its new display size (points), and how far its frame's top-left corner moves (points).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cropped {
    pub crop: [f32; 4],
    pub w: f32,
    pub h: f32,
    pub dx: f32,
    pub dy: f32,
}

/// One axis of [`cropped`]: a `len` long side cropped `lo` / `hi`, its handle end `grab`
/// (-1 low, 1 high, 0 none) dragged by `d` points. Returns the new crops, length and how far the
/// low edge moved.
fn axis(len: f32, lo: f32, hi: f32, grab: i8, d: f32) -> (f32, f32, f32, f32) {
    // The source's full length at this scale; the visible part is 1-lo-hi of it.
    let full = len / (1.0 - lo - hi).max(0.1);
    let (mut lo2, mut hi2) = (lo, hi);
    match grab {
        -1 => lo2 = (lo + d / full).clamp(0.0, MAX_SIDE),
        1 => hi2 = (hi - d / full).clamp(0.0, MAX_SIDE),
        _ => {}
    }
    (lo2, hi2, full * (1.0 - lo2 - hi2), (lo2 - lo) * full)
}

/// Drag crop handle `handle` of a `w`×`h` point picture cropped `crop` by (`dx`, `dy`) points: the
/// grabbed sides are trimmed (inward) or restored (outward) at the picture's current scale.
/// Hostile numbers (non-finite, non-positive sizes) leave it as it was.
pub fn cropped(w: f32, h: f32, crop: [f32; 4], handle: Handle, dx: f32, dy: f32) -> Cropped {
    let fin = |x: f32| if x.is_finite() { x } else { 0.0 };
    let crop = crop.map(|c| fin(c).clamp(0.0, MAX_SIDE));
    let (w, h) = (fin(w), fin(h));
    if w <= 0.0 || h <= 0.0 {
        return Cropped { crop, w, h, dx: 0.0, dy: 0.0 };
    }
    let (l, r, w2, mx) = axis(w, crop[0], crop[2], handle.x, fin(dx));
    let (t, b, h2, my) = axis(h, crop[1], crop[3], handle.y, fin(dy));
    Cropped { crop: [l, t, r, b], w: w2, h: h2, dx: mx, dy: my }
}

/// A crop handle being dragged.
pub struct CropDrag {
    handle: Handle,
    /// The picture's frame where the drag started (page points) and its page.
    page: usize,
    start: wordcraft_geom::Rect,
    /// Its display size and crop then.
    w: f32,
    h: f32,
    crop: [f32; 4],
    press: Pos2,
    moved: bool,
    /// Where the drag puts it now.
    now: Cropped,
}

impl CropDrag {
    fn follow(&mut self, at: Pos2, scale: f32) {
        if !self.moved && at.distance(self.press) < DRAG_START {
            return;
        }
        self.moved = true;
        let d = (at - self.press) / scale.max(0.01);
        self.now = cropped(self.w, self.h, self.crop, self.handle, d.x, d.y);
    }
}

/// The selected picture, if it's an unturned picture: its position, size and crop.
fn picture(app: &WordApp) -> Option<(Pos, f32, f32, [f32; 4], String)> {
    match crate::objects::selected(app)? {
        (pos, InlineObject::Image { w, h, crop, media, float, .. }) if wordcraft_geom::normalize_degrees(float.rot) == 0.0 => {
            Some((pos, *w, *h, *crop, media.clone()))
        }
        _ => None,
    }
}

/// The picture in crop mode: crop mode is on and that picture is still the selection.
pub fn active(app: &WordApp) -> Option<Pos> {
    let on = app.canvas.crop.as_ref()?;
    picture(app).map(|p| p.0).filter(|p| p == on)
}

/// Whether Crop can be clicked: a picture is selected (or crop mode is on, to leave it).
pub fn can_toggle(app: &WordApp) -> bool {
    active(app).is_some() || matches!(crate::objects::selected(app), Some((_, InlineObject::Image { .. })))
}

/// `ui.crop`: crop mode on for the selected picture, or off (keeping the crop).
pub fn toggle(app: &mut WordApp) -> Result<Value, String> {
    if active(app).is_some() {
        leave(app);
        return Ok(json!({"crop": false}));
    }
    let Some((_, InlineObject::Image { .. })) = crate::objects::selected(app) else {
        return Err(tl!("select a picture first").to_string());
    };
    let Some((pos, ..)) = picture(app) else {
        let msg = tl!("Crop on the page works on unturned pictures: set Rotation to 0° first, or use the crop boxes").to_string();
        app.status(msg.clone());
        return Err(msg);
    };
    app.canvas.crop = Some(pos);
    app.canvas.crop_drag = None;
    Ok(json!({"crop": true}))
}

fn leave(app: &mut WordApp) {
    app.canvas.crop = None;
    app.canvas.crop_drag = None;
}

/// Esc and Enter leave crop mode, keeping the crop. Returns true if the key was taken.
pub fn key(app: &mut WordApp, key: egui::Key) -> bool {
    if active(app).is_none() {
        app.canvas.crop = None;
        return false;
    }
    if matches!(key, egui::Key::Escape | egui::Key::Enter) {
        leave(app);
        return true;
    }
    false
}

fn screen(pages: &[Rect], scale: f32, page: usize, r: wordcraft_geom::Rect) -> Option<Rect> {
    pages.get(page).map(|p| frame::to_screen(*p, scale, r))
}

/// The picture in crop mode, laid out: its frame (page points) and page.
fn laid_out(app: &WordApp, layout: &DocLayout) -> Option<(usize, wordcraft_geom::Rect)> {
    let pos = active(app)?;
    layout.object(&pos, app.session.page_hint).map(|o| (o.page, o.rect))
}

/// Presses and drags in crop mode. `Some(true)`: taken; `Some(false)` / `None`: not about crop
/// mode, so the usual object handling goes on (a press outside the picture has just left it).
pub fn pointer(app: &mut WordApp, ui: &Ui, resp: &egui::Response, pages: &[Rect], layout: &DocLayout, scale: f32, at: Pos2) -> Option<bool> {
    if let Some(mut d) = app.canvas.crop_drag.take() {
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            return Some(true); // the drag is dropped without a change; crop mode stays on
        }
        if ui.input(|i| i.pointer.primary_down()) {
            d.follow(at, scale);
            app.canvas.crop_drag = Some(d);
        } else if d.moved {
            let c = d.now;
            let _ = app.run(
                "picture.crop",
                json!({"left": c.crop[0], "top": c.crop[1], "right": c.crop[2], "bottom": c.crop[3], "width": c.w, "height": c.h}),
            );
        }
        return Some(true);
    }
    if active(app).is_none() {
        app.canvas.crop = None;
        return None;
    }
    if !(ui.input(|i| i.pointer.primary_pressed()) && resp.contains_pointer()) {
        return None;
    }
    let (page, rect) = laid_out(app, layout)?;
    let (_, w, h, crop, _) = picture(app)?;
    let f = screen(pages, scale, page, rect)?;
    if let Some(handle) = frame::handle_at(f, 0.0, at) {
        let now = Cropped { crop, w, h, dx: 0.0, dy: 0.0 };
        app.canvas.crop_drag = Some(CropDrag { handle, page, start: rect, w, h, crop, press: at, moved: false, now });
        return Some(true);
    }
    if f.contains(at) {
        return Some(true); // inside the picture: crop mode stays on
    }
    // Outside: crop mode ends with the crop kept, and the press goes on as usual.
    leave(app);
    None
}

/// The cursor over a crop handle (or while dragging one).
pub fn cursor(app: &WordApp, layout: &DocLayout, pages: &[Rect], scale: f32, at: Pos2) -> Option<egui::CursorIcon> {
    if let Some(d) = &app.canvas.crop_drag {
        return Some(d.handle.cursor());
    }
    let (page, rect) = laid_out(app, layout)?;
    let f = screen(pages, scale, page, rect)?;
    match frame::handle_at(f, 0.0, at) {
        Some(h) => Some(h.cursor()),
        None if f.contains(at) => Some(egui::CursorIcon::Default),
        None => None,
    }
}

/// The source picture as a texture (downscaled), kept in egui's memory per media item; None when
/// it can't be decoded (then the cropped-away area is just dimmed).
fn source(app: &WordApp, ctx: &egui::Context, media: &str) -> Option<egui::TextureHandle> {
    let bytes = app.session.doc.media.get(media)?;
    let id = egui::Id::new(("crop_source", media, bytes.len()));
    if let Some(t) = ctx.data(|d| d.get_temp::<Option<egui::TextureHandle>>(id)) {
        return t;
    }
    let tex = image::load_from_memory(bytes).ok().map(|img| {
        let img = if img.width().max(img.height()) > SOURCE_MAX_PX { img.thumbnail(SOURCE_MAX_PX, SOURCE_MAX_PX) } else { img };
        let rgba = img.to_rgba8();
        let ci = egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw());
        ctx.load_texture(format!("crop_source_{media}"), ci, egui::TextureOptions::LINEAR)
    });
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    tex
}

/// Draw crop mode: the cropped-away part dimmed around the picture, its frame and the crop
/// handles; while dragging, the picture as the drop would leave it. Returns whether crop mode
/// drew (then the usual frame isn't drawn).
pub fn paint(app: &WordApp, painter: &Painter, t: &Tokens, layout: &DocLayout, pages: &[Rect], scale: f32) -> bool {
    let Some((page, rect)) = laid_out(app, layout) else { return false };
    let Some((.., crop, media)) = picture(app) else { return false };
    let drag = app.canvas.crop_drag.as_ref().filter(|d| d.moved);
    let (page, frame_pt, c) = match drag {
        Some(d) => {
            let n = d.now;
            (d.page, wordcraft_geom::Rect::new(d.start.x + n.dx, d.start.y + n.dy, n.w, n.h), n.crop)
        }
        None => (page, rect, crop),
    };
    // The whole source, at the picture's scale, around the visible part.
    let fw = frame_pt.w / (1.0 - c[0] - c[2]).max(0.1);
    let fh = frame_pt.h / (1.0 - c[1] - c[3]).max(0.1);
    let full_pt = wordcraft_geom::Rect::new(frame_pt.x - c[0] * fw, frame_pt.y - c[1] * fh, fw, fh);
    let (Some(f), Some(full)) = (screen(pages, scale, page, frame_pt), screen(pages, scale, page, full_pt)) else { return false };
    let tex = source(app, painter.ctx(), &media);
    let uv = |r: Rect| {
        let (w, h) = (full.width().max(0.01), full.height().max(0.01));
        Rect::from_min_max(pos2((r.min.x - full.min.x) / w, (r.min.y - full.min.y) / h), pos2((r.max.x - full.min.x) / w, (r.max.y - full.min.y) / h))
    };
    if let (Some(tex), Some(_)) = (&tex, drag) {
        // Dragging: the page still shows the old crop, so show the new visible part too.
        painter.rect_filled(f, 0.0, Color32::WHITE);
        painter.image(tex.id(), f, uv(f), Color32::WHITE);
    }
    // The cropped-away bands: top, bottom, left, right.
    let bands = [
        Rect::from_min_max(full.min, pos2(full.max.x, f.min.y)),
        Rect::from_min_max(pos2(full.min.x, f.max.y), full.max),
        Rect::from_min_max(pos2(full.min.x, f.min.y), pos2(f.min.x, f.max.y)),
        Rect::from_min_max(pos2(f.max.x, f.min.y), pos2(full.max.x, f.max.y)),
    ];
    for b in bands.into_iter().filter(|b| b.width() > 0.0 && b.height() > 0.0) {
        if let Some(tex) = &tex {
            painter.image(tex.id(), b, uv(b), Color32::WHITE);
        }
        painter.rect_filled(b, 0.0, t.crop_dim);
    }
    painter.rect_stroke(full, 0.0, egui::Stroke::new(1.0, t.crop_dim), egui::StrokeKind::Outside);
    painter.rect_stroke(f, 0.0, egui::Stroke::new(1.0, t.crop_handle), egui::StrokeKind::Inside);
    paint_handles(painter, t, f);
    true
}

/// The crop handles of frame `f`: an L at each corner, a bar mid-edge, drawn just inside the
/// frame in the crop handle colour with a light halo.
fn paint_handles(painter: &Painter, t: &Tokens, f: Rect) {
    for h in frame::handles(f) {
        let (sx, sy) = (f32::from(h.x), f32::from(h.y));
        let c = pos2(f.center().x + sx * f.width() / 2.0, f.center().y + sy * f.height() / 2.0);
        let bars: Vec<Rect> = match (h.x, h.y) {
            (0, _) => vec![Rect::from_two_pos(c - vec2(BAR / 2.0, 0.0), c + vec2(BAR / 2.0, -sy * THICK))],
            (_, 0) => vec![Rect::from_two_pos(c - vec2(0.0, BAR / 2.0), c + vec2(-sx * THICK, BAR / 2.0))],
            _ => vec![Rect::from_two_pos(c, c + vec2(-sx * ARM, -sy * THICK)), Rect::from_two_pos(c, c + vec2(-sx * THICK, -sy * ARM))],
        };
        for b in &bars {
            painter.rect_filled(b.expand(1.0), 0.0, t.handle);
        }
        for b in bars {
            painter.rect_filled(b, 0.0, t.crop_handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wordcraft_doc::para::{InlineObject, OBJ};
    use wordcraft_doc::{Document, Pos};

    use super::*;
    use crate::WordApp;

    const H: fn(i8, i8) -> Handle = |x, y| Handle { x, y };

    fn close(a: Cropped, b: Cropped) -> bool {
        let n = |x: f32, y: f32| (x - y).abs() < 1e-4;
        a.crop.iter().zip(b.crop).all(|(x, y)| n(*x, y)) && n(a.w, b.w) && n(a.h, b.h) && n(a.dx, b.dx) && n(a.dy, b.dy)
    }

    #[test]
    fn each_handle_trims_its_own_sides_and_keeps_the_scale() {
        let c = |crop, w, h, dx, dy| Cropped { crop, w, h, dx, dy };
        // Left in by 50 of a 200 pt wide picture: a quarter of the source goes.
        let r = cropped(200.0, 100.0, [0.0; 4], H(-1, 0), 50.0, 33.0);
        assert!(close(r, c([0.25, 0.0, 0.0, 0.0], 150.0, 100.0, 50.0, 0.0)), "{r:?}");
        // Right in by 40.
        let r = cropped(200.0, 100.0, [0.0; 4], H(1, 0), -40.0, 0.0);
        assert!(close(r, c([0.0, 0.0, 0.2, 0.0], 160.0, 100.0, 0.0, 0.0)), "{r:?}");
        // Bottom in by 10; the sideways part of the drag doesn't count.
        let r = cropped(200.0, 100.0, [0.0; 4], H(0, 1), 999.0, -10.0);
        assert!(close(r, c([0.0, 0.0, 0.0, 0.1], 200.0, 90.0, 0.0, 0.0)), "{r:?}");
        // Top-left corner: two sides, and the frame's corner follows.
        let r = cropped(200.0, 100.0, [0.0; 4], H(-1, -1), 20.0, 10.0);
        assert!(close(r, c([0.1, 0.1, 0.0, 0.0], 180.0, 90.0, 20.0, 10.0)), "{r:?}");
        // Already cropped: the full source is wider than the frame (160 of 200 shown).
        let r = cropped(160.0, 100.0, [0.2, 0.0, 0.0, 0.0], H(-1, 0), 20.0, 0.0);
        assert!(close(r, c([0.3, 0.0, 0.0, 0.0], 140.0, 100.0, 20.0, 0.0)), "{r:?}");
    }

    #[test]
    fn crops_are_clamped_and_never_go_past_the_source() {
        // At most 0.45 a side.
        let r = cropped(200.0, 100.0, [0.0; 4], H(-1, 0), 1000.0, 0.0);
        assert!((r.crop[0] - 0.45).abs() < 1e-4 && (r.w - 110.0).abs() < 1e-3 && (r.dx - 90.0).abs() < 1e-3, "{r:?}");
        // Out past the original edge: back to uncropped, no further.
        let r = cropped(160.0, 100.0, [0.0, 0.0, 0.2, 0.0], H(1, 0), 100.0, 0.0);
        assert!(close(r, Cropped { crop: [0.0; 4], w: 200.0, h: 100.0, dx: 0.0, dy: 0.0 }), "{r:?}");
        let r = cropped(200.0, 90.0, [0.0, 0.1, 0.0, 0.0], H(0, -1), 0.0, -500.0);
        assert!(close(r, Cropped { crop: [0.0; 4], w: 200.0, h: 100.0, dx: 0.0, dy: -10.0 }), "{r:?}");
    }

    #[test]
    fn dragging_back_out_restores_what_was_cropped() {
        let a = cropped(200.0, 100.0, [0.0; 4], H(1, 1), -30.0, -20.0);
        let b = cropped(a.w, a.h, a.crop, H(1, 1), 30.0, 20.0);
        assert!(close(b, Cropped { crop: [0.0; 4], w: 200.0, h: 100.0, dx: 0.0, dy: 0.0 }), "{a:?} {b:?}");
    }

    #[test]
    fn hostile_input_leaves_the_picture_as_it_was() {
        for (w, h, crop, dx, dy) in [
            (200.0, 100.0, [0.0; 4], f32::NAN, 0.0),
            (200.0, 100.0, [0.0; 4], f32::INFINITY, f32::NEG_INFINITY),
            (0.0, 100.0, [0.0; 4], 10.0, 0.0),
            (200.0, f32::NAN, [0.0; 4], 0.0, 10.0),
            (200.0, 100.0, [f32::NAN, 0.0, 0.7, 0.0], 10.0, 10.0),
        ] {
            let r = cropped(w, h, crop, H(-1, -1), dx, dy);
            assert!(r.w.is_finite() && r.h.is_finite() && r.crop.iter().all(|c| c.is_finite()), "{r:?}");
            if !(w > 0.0 && h > 0.0 && w.is_finite() && h.is_finite()) {
                assert_eq!(r.dx, 0.0);
            }
        }
    }

    /// An app whose first paragraph holds a picture, selected.
    fn app_with_picture(rot: f32) -> WordApp {
        let mut app = WordApp::new(wordcraft_engine::Session::new(Document::new()), crate::Services::default());
        let float = wordcraft_doc::para::Float { rot, ..Default::default() };
        let pic = InlineObject::Image { media: "m".into(), w: 200.0, h: 100.0, alt: String::new(), float, crop: [0.0; 4], ole: None };
        app.session.doc.insert_object(&Pos::body(0, 0), pic, &Default::default()).unwrap();
        let end = Pos::body(0, OBJ.len_utf8());
        app.run("select.range", json!({"anchor": Pos::body(0, 0), "focus": end})).unwrap();
        app
    }

    #[test]
    fn ui_crop_toggles_crop_mode_and_esc_enter_leave_it_keeping_the_crop() {
        let mut app = app_with_picture(0.0);
        assert!(crate::widgets::enabled(&app, "ui.crop"));
        app.run("ui.crop", json!({})).unwrap();
        assert!(active(&app).is_some(), "Crop turns crop mode on");
        app.run("ui.crop", json!({})).unwrap();
        assert!(active(&app).is_none(), "Crop again turns it off");
        for key in [egui::Key::Escape, egui::Key::Enter] {
            app.run("ui.crop", json!({})).unwrap();
            app.run("picture.crop", json!({"left": 0.1})).unwrap();
            assert!(crate::objects::key(&mut app, key, Default::default()), "{key:?} is taken");
            assert!(active(&app).is_none(), "{key:?} leaves crop mode");
            let Some((_, InlineObject::Image { crop, .. })) = crate::objects::selected(&app) else { panic!("still selected") };
            assert_eq!(crop[0], 0.1, "{key:?} keeps the crop");
        }
        // Without crop mode, Escape isn't a crop key.
        assert!(!crate::objects::key(&mut app, egui::Key::Escape, Default::default()));
    }

    #[test]
    fn ui_crop_needs_an_unturned_selected_picture() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(Document::new()), crate::Services::default());
        assert!(!crate::widgets::enabled(&app, "ui.crop"));
        assert!(app.run("ui.crop", json!({})).is_err());
        assert!(active(&app).is_none());
        let mut app = app_with_picture(30.0);
        assert!(app.run("ui.crop", json!({})).is_err(), "a turned picture isn't cropped on the canvas yet");
        assert!(active(&app).is_none());
        // Selecting something else ends crop mode.
        let mut app = app_with_picture(0.0);
        app.run("ui.crop", json!({})).unwrap();
        app.run("caret.set", json!({"pos": Pos::body(0, 0)})).unwrap();
        assert!(active(&app).is_none());
    }
}

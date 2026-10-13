//! Mirrored interface for right-to-left languages (Hebrew). Instead of every panel, menu and dialog
//! laying itself out twice, the whole window is mirrored as it is painted: an egui plugin flips
//! each shape of the finished frame about the window's centre, and flips the pointer on the way
//! in, so a click lands on what is drawn under it. Text keeps its glyphs: a label moves to the
//! mirrored place, so it lines up on the right, and the lines of a wrapped label are aligned right.
//! Icons drawn in code flip, as they do in right-to-left Word; pictures keep their orientation.
//!
//! What has its own direction moves without flipping:
//! - the document: the page canvas with its rulers (a document's paragraphs say which way they
//!   run);
//! - the text field being edited, whose caret and selection are drawn apart from its text.
//!
//! Popups placed from the page (context menu, mini toolbar, tooltips over the page) are mirrored
//! about the point they open from, so they open leftwards from it, as right-to-left menus do.

use std::sync::Arc;

use egui::epaint::{ClippedShape, Shape};
use egui::layers::ShapeIdx;
use egui::{Context, Event, FullOutput, Id, LayerId, Order, Pos2, RawInput, Rect, Ui, Vec2};

/// The plugin; [`configure`] sets it up each frame.
#[derive(Default)]
pub struct Mirror {
    on: bool,
    /// The document canvas (logical coordinates), when it is on screen.
    canvas: Option<Rect>,
    /// Popups placed from document coordinates (their area ids).
    anchored: Vec<Id>,
    /// Where those popups were this frame (logical), with the axis they were mirrored about (as
    /// the sum `x + x'`), for the pointer.
    anchored_rects: Vec<(Rect, f32)>,
    /// The text field being edited (logical), for the pointer.
    field: Option<Rect>,
    /// The window's horizontal extent, from the last frame.
    span: Option<(f32, f32)>,
    /// The pointer as last seen on screen: wheel scrolling over the page isn't flipped.
    pointer: Option<Pos2>,
}

impl egui::Plugin for Mirror {
    fn debug_name(&self) -> &'static str {
        "wordcraft mirror"
    }

    fn input_hook(&mut self, _ctx: &Context, input: &mut RawInput) {
        if let Some(r) = input.screen_rect {
            self.span = Some((r.min.x, r.max.x));
        }
        let Some(span) = self.span.filter(|_| self.on) else { return };
        let mut shifted: Vec<(Rect, f32)> = Vec::new();
        if let Some(c) = self.canvas {
            let d = keep_shift(span, c);
            if let Some(f) = self.field {
                shifted.push((f, keep_shift(span, f)));
            }
            shifted.push((c, d));
        } else if let Some(f) = self.field {
            shifted.push((f, keep_shift(span, f)));
        }
        let anchored = &self.anchored_rects;
        let to_logical = |p: Pos2| -> Pos2 {
            if let Some((_, sum)) = anchored.iter().find(|(r, sum)| flip_rect((0.0, *sum), *r).contains(p)) {
                return Pos2::new(sum - p.x, p.y);
            }
            match shifted.iter().find(|(r, d)| r.translate(Vec2::new(*d, 0.0)).contains(p)) {
                Some((_, d)) => Pos2::new(p.x - d, p.y),
                None => Pos2::new(flip_x(span, p.x), p.y),
            }
        };
        let over_page = |p: Option<Pos2>| {
            p.is_some_and(|p| shifted.last().is_some_and(|(c, d)| self.canvas.is_some() && c.translate(Vec2::new(*d, 0.0)).contains(p)))
        };
        let flip_wheel = !over_page(self.pointer);
        for event in &mut input.events {
            match event {
                Event::PointerMoved(p) => {
                    self.pointer = Some(*p);
                    *p = to_logical(*p);
                }
                Event::PointerButton { pos, .. } => *pos = to_logical(*pos),
                Event::Touch { pos, .. } => *pos = to_logical(*pos),
                Event::PointerGone => self.pointer = None,
                Event::MouseMoved(delta) => delta.x = -delta.x,
                Event::MouseWheel { delta, .. } if flip_wheel => delta.x = -delta.x,
                _ => {}
            }
        }
    }

    fn on_end_pass(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let screen = ctx.content_rect();
        let span = (screen.min.x, screen.max.x);
        self.span = Some(span);
        self.anchored_rects.clear();
        self.field = None;
        if !self.on {
            return;
        }
        self.field = editing_field(&ctx);
        let Some(canvas) = self.canvas else { return };
        // Tooltips follow the pointer: over the page they belong to the page.
        const ORDERS: [Order; 5] = [Order::Background, Order::Middle, Order::Foreground, Order::Tooltip, Order::Debug];
        let mut layers: Vec<LayerId> = self.anchored.iter().flat_map(|id| ORDERS.map(|o| LayerId::new(o, *id))).collect();
        if ctx.pointer_hover_pos().is_some_and(|p| canvas.contains(p)) {
            layers.extend(ctx.memory(|m| m.layer_ids().filter(|l| l.order == Order::Tooltip).collect::<Vec<_>>()));
        }
        // A popup opens rightwards from logical point `a` (its left edge), which is drawn at
        // `a + d` with the page. Moved here and flipped with everything else, it is mirrored about
        // that point: `x' = 2a + d - x`, so it opens leftwards from it.
        let d = keep_shift(span, canvas);
        let mut placed = Vec::new();
        for layer in layers {
            let Some(area) = ctx.memory(|m| m.area_rect(layer.id)).filter(|r| r.is_positive()) else { continue };
            let sum = 2.0 * area.min.x + d;
            let shift = Vec2::new(span.0 + span.1 - sum, 0.0);
            let found = ctx.graphics_mut(|g| {
                let Some(list) = g.get_mut(layer) else { return false };
                for i in 0..list.next_idx().0 {
                    list.mutate_shape(ShapeIdx(i), |c| {
                        c.clip_rect = c.clip_rect.translate(shift);
                        c.shape.translate(shift);
                    });
                }
                true
            });
            if found {
                placed.push((area, sum));
            }
        }
        self.anchored_rects = placed;
    }

    fn output_hook(&mut self, _ctx: &Context, output: &mut FullOutput) {
        let Some(span) = self.span.filter(|_| self.on) else { return };
        for clipped in &mut output.shapes {
            mirror_clipped(span, self.canvas, self.field, clipped);
        }
        if let Some(ime) = &mut output.platform_output.ime {
            let keep: Vec<Rect> = self.field.into_iter().chain(self.canvas).collect();
            ime.rect = place_rect(span, &keep, ime.rect);
            ime.cursor_rect = place_rect(span, &keep, ime.cursor_rect);
        }
    }
}

/// Whether this frame is mirrored: icon letters (`x²`) are placed so the mirror keeps their order.
pub fn active(ctx: &Context) -> bool {
    ctx.plugin_opt::<Mirror>().is_some_and(|h| h.lock().on)
}

/// Turn the mirror on (a right-to-left interface) or off. `canvas` is the document canvas with
/// its rulers when it is on screen, `anchored` the area ids of popups placed from it. Call once per
/// frame, after the interface is laid out.
pub fn configure(ctx: &Context, on: bool, canvas: Option<Rect>, anchored: &[Id]) {
    if ctx.plugin_opt::<Mirror>().is_none() {
        ctx.add_plugin(Mirror::default());
    }
    if let Some(handle) = ctx.plugin_opt::<Mirror>() {
        let mut m = handle.lock();
        m.on = on;
        m.canvas = canvas;
        m.anchored = anchored.to_vec();
    }
}

fn flip_x((min, max): (f32, f32), x: f32) -> f32 {
    min + max - x
}

fn flip_rect(span: (f32, f32), r: Rect) -> Rect {
    if !r.is_finite() {
        return r;
    }
    Rect::from_x_y_ranges(flip_x(span, r.max.x)..=flip_x(span, r.min.x), r.y_range())
}

/// How far an area kept unflipped moves to sit where the mirror puts it.
fn keep_shift(span: (f32, f32), k: Rect) -> f32 {
    flip_x(span, k.max.x) - k.min.x
}

/// A rectangle (the input method's) placed like the shapes around it.
fn place_rect(span: (f32, f32), keep: &[Rect], r: Rect) -> Rect {
    match keep.iter().find(|k| k.expand(1.0).contains_rect(r)) {
        Some(k) => r.translate(Vec2::new(keep_shift(span, *k), 0.0)),
        None => flip_rect(span, r),
    }
}

/// The text field being edited, if any: its caret and selection are drawn apart from its text.
fn editing_field(ctx: &Context) -> Option<Rect> {
    let id: Id = ctx.memory(|m| m.focused())?;
    egui::TextEdit::load_state(ctx, id)?;
    ctx.read_response(id).map(|r| r.rect)
}

/// Mirror one shape of the finished frame: shapes clipped to the canvas, and shapes inside the
/// field being edited, move without flipping.
fn mirror_clipped(span: (f32, f32), canvas: Option<Rect>, field: Option<Rect>, clipped: &mut ClippedShape) {
    let in_canvas = canvas.filter(|c| c.expand(1.0).contains_rect(clipped.clip_rect));
    let in_field = || {
        let b = clipped.shape.visual_bounding_rect();
        field.filter(|f| b.is_positive() && f.expand(1.0).contains_rect(b))
    };
    if let Some(k) = in_canvas.or_else(in_field) {
        let d = Vec2::new(keep_shift(span, k), 0.0);
        clipped.clip_rect = clipped.clip_rect.translate(d);
        clipped.shape.translate(d);
        return;
    }
    clipped.clip_rect = flip_rect(span, clipped.clip_rect);
    mirror_shape(span, &mut clipped.shape);
}

fn mirror_shape(span: (f32, f32), shape: &mut Shape) {
    let fx = |p: &mut Pos2| p.x = flip_x(span, p.x);
    match shape {
        Shape::Noop => {}
        Shape::Vec(shapes) => shapes.iter_mut().for_each(|s| mirror_shape(span, s)),
        Shape::Circle(c) => fx(&mut c.center),
        Shape::Ellipse(e) => fx(&mut e.center),
        Shape::LineSegment { points, .. } => points.iter_mut().for_each(fx),
        Shape::Path(p) => p.points.iter_mut().for_each(fx),
        Shape::QuadraticBezier(b) => b.points.iter_mut().for_each(fx),
        Shape::CubicBezier(b) => b.points.iter_mut().for_each(fx),
        Shape::Rect(r) => {
            r.rect = flip_rect(span, r.rect);
            let c = &mut r.corner_radius;
            std::mem::swap(&mut c.nw, &mut c.ne);
            std::mem::swap(&mut c.sw, &mut c.se);
        }
        // Text moves as a block and keeps its glyphs; wrapped lines align right.
        Shape::Text(t) => {
            let r = t.galley.rect.translate(t.pos.to_vec2());
            t.pos.x += flip_x(span, r.max.x) - r.min.x;
            if t.galley.rows.len() > 1 {
                let galley = Arc::make_mut(&mut t.galley);
                let right = galley.rect.max.x;
                for row in &mut galley.rows {
                    row.pos.x += right - row.rect().max.x;
                }
            }
        }
        Shape::Mesh(mesh) => {
            let m = Arc::make_mut(mesh);
            if m.texture_id == egui::TextureId::default() {
                // Drawn in code (icons, arrows): flip.
                m.vertices.iter_mut().for_each(|v| fx(&mut v.pos));
            } else {
                // A picture: move it, keep it the right way round.
                let r = m.calc_bounds();
                m.translate(Vec2::new(flip_x(span, r.max.x) - r.min.x, 0.0));
            }
        }
        Shape::Callback(c) => c.rect = flip_rect(span, c.rect),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Color32, Stroke, pos2};

    const SPAN: (f32, f32) = (0.0, 100.0);

    #[test]
    fn shapes_flip_about_the_window_centre() {
        let rect = Rect::from_min_max(pos2(10.0, 0.0), pos2(30.0, 5.0));
        let mut filled = ClippedShape { clip_rect: Rect::EVERYTHING, shape: Shape::rect_filled(rect, 0.0, Color32::RED) };
        mirror_clipped(SPAN, None, None, &mut filled);
        assert_eq!(filled.shape.visual_bounding_rect(), Rect::from_min_max(pos2(70.0, 0.0), pos2(90.0, 5.0)));
        let mut line = Shape::line_segment([pos2(0.0, 1.0), pos2(20.0, 1.0)], Stroke::new(1.0, Color32::RED));
        mirror_shape(SPAN, &mut line);
        assert!(matches!(line, Shape::LineSegment { points, .. } if points == [pos2(100.0, 1.0), pos2(80.0, 1.0)]));
    }

    #[test]
    fn the_page_and_the_field_being_edited_move_without_flipping() {
        // The canvas spans 0–60 of a 100-wide window: mirrored it spans 40–100, its contents in
        // the same order.
        let canvas = Rect::from_min_max(pos2(0.0, 0.0), pos2(60.0, 50.0));
        let caret = Rect::from_min_max(pos2(5.0, 0.0), pos2(6.0, 9.0));
        let mut on_page = ClippedShape { clip_rect: canvas, shape: Shape::rect_filled(caret, 0.0, Color32::BLACK) };
        mirror_clipped(SPAN, Some(canvas), None, &mut on_page);
        assert_eq!(on_page.shape.visual_bounding_rect().min.x, 45.0);
        assert_eq!(on_page.clip_rect, Rect::from_min_max(pos2(40.0, 0.0), pos2(100.0, 50.0)));
        // A field at 70–90 mirrors to 10–30; its caret at 72 lands at 12, still near its start.
        let field = Rect::from_min_max(pos2(70.0, 60.0), pos2(90.0, 70.0));
        let mut in_field = ClippedShape {
            clip_rect: Rect::EVERYTHING,
            shape: Shape::rect_filled(Rect::from_min_max(pos2(72.0, 61.0), pos2(73.0, 69.0)), 0.0, Color32::BLACK),
        };
        mirror_clipped(SPAN, Some(canvas), Some(field), &mut in_field);
        assert_eq!(in_field.shape.visual_bounding_rect().min.x, 12.0);
        assert_eq!(place_rect(SPAN, &[canvas], caret).min.x, 45.0);
    }

    #[test]
    fn the_pointer_is_flipped_back_to_where_widgets_are_laid_out() {
        let canvas = Rect::from_min_max(pos2(0.0, 0.0), pos2(60.0, 50.0));
        let mut m = Mirror { on: true, canvas: Some(canvas), ..Default::default() };
        let mut input = RawInput {
            screen_rect: Some(Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 80.0))),
            events: vec![
                Event::PointerMoved(pos2(90.0, 70.0)),
                Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(3.0, 1.0),
                    modifiers: Default::default(),
                    phase: egui::TouchPhase::Move,
                },
                Event::PointerMoved(pos2(45.0, 10.0)),
            ],
            ..Default::default()
        };
        egui::Plugin::input_hook(&mut m, &Context::default(), &mut input);
        assert!(matches!(input.events[0], Event::PointerMoved(p) if p == pos2(10.0, 70.0)), "outside the page: flipped");
        assert!(matches!(input.events[1], Event::MouseWheel { delta, .. } if delta == egui::vec2(-3.0, 1.0)), "sideways scrolling flips");
        assert!(matches!(input.events[2], Event::PointerMoved(p) if p == pos2(5.0, 10.0)), "on the mirrored page: shifted");
        // A context menu laid out at 10–30 and mirrored about 130 (`2a + d`) is drawn at 100–120.
        m.anchored_rects = vec![(Rect::from_min_max(pos2(10.0, 0.0), pos2(30.0, 40.0)), 130.0)];
        let mut on_menu = RawInput { events: vec![Event::PointerMoved(pos2(112.0, 5.0))], ..Default::default() };
        egui::Plugin::input_hook(&mut m, &Context::default(), &mut on_menu);
        assert!(matches!(on_menu.events[0], Event::PointerMoved(p) if p == pos2(18.0, 5.0)), "on a popup from the page: its own axis");
        m.on = false;
        let mut plain = RawInput { events: vec![Event::PointerMoved(pos2(90.0, 70.0))], ..Default::default() };
        egui::Plugin::input_hook(&mut m, &Context::default(), &mut plain);
        assert!(matches!(plain.events[0], Event::PointerMoved(p) if p == pos2(90.0, 70.0)), "off: untouched");
    }
}

//! The equation editor: equaSHion's [`EquationEditor`] widget in a window at the bottom of the
//! app (the page, where the equation sits, stays visible above it).
//!
//! It opens wherever editing an equation starts (`Session::math` becomes set): Alt+=,
//! Insert › Equation, a click or double-click on an equation, the Equation tab's galleries
//! outside an equation, an agent's `insert.equation`. The widget edits a copy; the document
//! changes once, through the engine's own commands, so undo, Track Changes and save work as for
//! any other edit:
//! - Enter or Insert / Update applies it with `equation.set` (plus `equation.display` when the
//!   display toggle changed). An equation inserted just before the editor opened takes the edit
//!   into the same undo step, so inserting and filling it is one Undo.
//! - Esc cancels the way the widget's [`equashion_egui::DISCARD_PROMPT`] says (a second Esc when
//!   there are changes); Cancel and the close button cancel at once. A new equation that was never
//!   filled goes away again.
//! - Leaving the equation another way (a click elsewhere on the page, Close Equation) keeps the
//!   edit, as leaving an equation does in Word.
//!
//! While the editor is open, the Equation tab's structure, symbol and ready-made galleries insert
//! into the widget rather than into the document, and the tab's options (display, justification,
//! number, conversion) first apply the pending edit. The widget never touches the shared egui
//! context's fonts, visuals or style; it draws the equation with its own bundled fonts.

use egui::vec2;
use equashion_egui::EquationEditor;
use serde_json::{Value, json};
use wordcraft_doc::math::{Arg, Math, parse_linear};
use wordcraft_doc::{InlineObject, Pos};
use wordcraft_engine::Selection;
use wordcraft_engine::equashion_bridge as bridge;

use crate::WordApp;
use crate::theme::Tokens;

/// Undo labels of the commands that insert a new equation (`insert.equation`, and the
/// Equation tab's structure and symbol galleries used outside an equation).
const INSERT_STEPS: &[&str] = &["Equation", "Insert Structure", "Insert Symbol"];

/// Width of the editor's contents, points.
const WIDTH: f32 = 554.0;

/// Commands that change the edited equation in the document: the pending edit goes in first.
const APPLY_FIRST: &[&str] = &["equation.display", "equation.justify", "equation.number", "equation.convert", "equation.structure"];

/// The editor while it is open.
pub(crate) struct Open {
    widget: EquationEditor,
    /// The equation's place in the document.
    at: Pos,
    /// The equation as last loaded from (or applied to) the document, to notice changes made
    /// elsewhere (an agent, a ribbon option).
    loaded: Equation,
    /// The equation was inserted right before the editor opened.
    fresh: bool,
    /// Undo depth when the editor opened (or last applied).
    depth: usize,
    /// Why the last apply failed.
    error: Option<String>,
}

/// An equation object's parts: linear text, display flag, tree.
type Equation = (String, bool, Math);

/// The equation editor's state in [`WordApp`].
#[derive(Default)]
pub struct EquationHost {
    pub(crate) open: Option<Open>,
    /// Undo depth at the end of the last frame.
    seen_depth: usize,
}

impl EquationHost {
    /// The editor window is open.
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// The open editor's widget.
    pub fn widget(&self) -> Option<&EquationEditor> {
        self.open.as_ref().map(|o| &o.widget)
    }

    /// The open editor's widget, mutable.
    pub fn widget_mut(&mut self) -> Option<&mut EquationEditor> {
        self.open.as_mut().map(|o| &mut o.widget)
    }
}

/// The equation object at `at`.
fn equation_at(app: &WordApp, at: &Pos) -> Option<Equation> {
    match app.session.doc.para_at(at)?.object_at(at.off)? {
        InlineObject::Equation { linear, display, math } => Some((linear.clone(), *display, math.clone())),
        _ => None,
    }
}

fn load(widget: &mut EquationEditor, eq: &Equation) {
    widget.set_math(bridge::to_editor(&eq.0, eq.1, &eq.2));
}

/// Open the editor on the equation at `at`; `fresh` when it was inserted just now.
fn open(app: &mut WordApp, at: Pos, fresh: bool) {
    let Some(loaded) = equation_at(app, &at) else { return };
    let depth = app.session.undo_depth();
    let mut widget = EquationEditor::new().id_salt("wordcraft-equation-editor");
    load(&mut widget, &loaded);
    app.equation.open = Some(Open { widget, at, loaded, fresh, depth, error: None });
}

/// Apply the widget's equation to the document (`equation.set`, and `equation.display` when
/// the display toggle changed), as one undo step, joined to the insert when the equation is new.
fn apply(app: &mut WordApp, o: &mut Open) -> Result<(), String> {
    let Some(before) = equation_at(app, &o.at) else { return Err(tl!("The equation is no longer in the document.").to_string()) };
    o.widget.editor_mut().flush();
    let m = o.widget.math().clone();
    let display = m.para;
    // Point the equation commands at this equation, even when the caret has moved on.
    let here = app.session.math.as_ref().is_some_and(|e| e.at == o.at);
    let saved = (!here).then(|| (app.session.math.take(), app.session.sel.clone()));
    if !here {
        app.session.sel = Selection::caret(o.at.clone());
    }
    if o.fresh && app.session.undo_depth() == o.depth {
        app.session.join_next_undo();
    }
    let r = app.execute("equation.set", json!({"omml": bridge::editor_omml(&m)}));
    if r.is_ok() && display != before.1 {
        app.session.join_next_undo();
        let _ = app.execute("equation.display", json!({"value": display}));
    }
    if let Some((math, sel)) = saved {
        app.session.math = math;
        app.session.sel = sel;
    }
    r?;
    o.widget.mark_clean();
    o.depth = app.session.undo_depth();
    o.loaded = equation_at(app, &o.at).unwrap_or(before);
    o.error = None;
    Ok(())
}

/// The new, never filled equation the editor opened on: its insertion is still the last undo step.
fn untouched_new(app: &WordApp, o: &Open) -> bool {
    o.fresh && app.session.undo_depth() == o.depth && equation_at(app, &o.at).is_some_and(|e| e.0.is_empty() && e.2.nodes.is_empty())
}

/// Drop the edit: a new empty equation goes away again, otherwise the caret leaves it.
fn cancel(app: &mut WordApp, o: Open) {
    if untouched_new(app, &o) {
        let _ = app.execute("edit.undo", json!({}));
    } else if app.session.math.as_ref().is_some_and(|e| e.at == o.at) {
        let _ = app.execute("equation.exit", json!({}));
    }
    app.canvas.want_focus = true;
}

/// Enter / Insert / Update: apply and leave the equation. The editor stays open with the
/// reason when that fails.
fn commit(app: &mut WordApp, mut o: Open) -> Option<Open> {
    if o.widget.editor().is_blank() && untouched_new(app, &o) {
        cancel(app, o);
        return None;
    }
    if let Err(e) = apply(app, &mut o) {
        o.error = Some(e);
        return Some(o);
    }
    if app.session.math.as_ref().is_some_and(|e| e.at == o.at) {
        let _ = app.execute("equation.exit", json!({}));
    }
    app.canvas.want_focus = true;
    None
}

/// Close without asking, keeping a pending edit (the equation was left some other way).
fn keep_and_close(app: &mut WordApp, mut o: Open) {
    if o.widget.is_dirty() {
        let _ = apply(app, &mut o);
    }
}

/// Follow the session: open on the equation being edited, keep the edit and close when the
/// editing ends some other way, reload when the document's copy changes and nothing is pending.
fn sync(app: &mut WordApp) {
    // The equation is gone (Undo, deleted elsewhere): nothing to apply it to.
    if let Some(o) = &app.equation.open
        && equation_at(app, &o.at).is_none()
    {
        app.equation.open = None;
    }
    let editing = app.session.math.as_ref().map(|m| m.at.clone());
    // An equation was inserted since the last frame (and is the one being edited now).
    let fresh = app.session.undo_depth() > app.equation.seen_depth && app.session.undo_label().is_some_and(|l| INSERT_STEPS.contains(&l));
    match (editing, app.equation.open.take()) {
        (Some(at), None) => open(app, at, fresh),
        (Some(at), Some(o)) if o.at != at => {
            keep_and_close(app, o);
            open(app, at, fresh);
        }
        (None, Some(o)) => {
            keep_and_close(app, o);
            app.canvas.want_focus = true;
        }
        (_, o) => app.equation.open = o,
    }
    if let Some(mut o) = app.equation.open.take() {
        if let Some(now) = equation_at(app, &o.at)
            && now != o.loaded
            && !o.widget.is_dirty()
        {
            load(&mut o.widget, &now);
            o.loaded = now;
        }
        app.equation.open = Some(o);
    }
}

/// Draw the editor window (after everything that may start or end editing in this frame).
pub fn show(app: &mut WordApp, ctx: &egui::Context) {
    sync(app);
    if let Some(mut o) = app.equation.open.take() {
        // The page took the keyboard (a click on the equation, a ribbon button): typing still
        // belongs to the equation.
        if app.canvas.focused {
            let id = o.widget.canvas_id();
            ctx.memory_mut(|m| m.request_focus(id));
        }
        let mut open = true;
        let (mut ok, mut cancelled) = (false, false);
        let existing = !o.fresh;
        let title = if existing { tl!("Edit Equation") } else { tl!("Equation") };
        egui::Window::new(title)
            .id(egui::Id::new("wordcraft-equation-editor-window"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_BOTTOM, vec2(0.0, -24.0))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_width(WIDTH);
                let r = o.widget.show(ui);
                ok |= r.committed;
                cancelled |= r.cancelled;
                ui.allocate_ui_with_layout(vec2(WIDTH, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    if let Some(e) = &o.error {
                        ui.colored_label(Tokens::get(ui.ctx()).red, e);
                    }
                });
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(tl!("Cancel")).clicked() {
                            cancelled = true;
                        }
                        let label = if existing { tl!("Update") } else { tl!("Insert") };
                        let b = ui.add(egui::Button::new(egui::RichText::new(label).color(egui::Color32::WHITE)).fill(crate::theme::APP_COLOR));
                        ok |= b.clicked();
                    });
                });
            });
        if cancelled || !open {
            cancel(app, o);
        } else if ok {
            app.equation.open = commit(app, o);
        } else {
            app.equation.open = Some(o);
        }
    }
    app.equation.seen_depth = app.session.undo_depth();
}

/// The upstream nodes of an Equation-tab insertion (`id`, `linear`, `latex`, `omml`).
fn gallery_nodes(params: &Value) -> Option<Arg> {
    let s = |k: &str| params.get(k).and_then(Value::as_str);
    if let Some(id) = s("id") {
        return wordcraft_engine::math_gallery::template(id).map(|t| t.nodes());
    }
    if let Some(b) = s("builtin") {
        return wordcraft_engine::math_gallery::BUILT_INS.iter().find(|(id, _, _)| *id == b).map(|(_, _, lin)| parse_linear(lin));
    }
    if let Some(x) = s("omml") {
        return bridge::omml_equation(x).map(|(m, _)| m.nodes);
    }
    if let Some(l) = s("latex") {
        return Some(wordcraft_doc::math_latex::parse_latex(l));
    }
    s("linear").map(parse_linear)
}

/// A user command while the editor is open: the Equation tab's galleries insert into the widget,
/// and its options apply the pending edit first. `Some` when the command was handled here.
pub(crate) fn intercept(app: &mut WordApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    app.equation.open.as_ref()?;
    let into_widget = matches!(id, "equation.insertStructure" | "equation.insertSymbol")
        || (id == "insert.equation" && params.get("builtin").and_then(Value::as_str).is_some());
    if into_widget {
        let o = app.equation.open.as_mut()?;
        if id == "equation.insertSymbol" {
            let text = params.get("char").and_then(Value::as_str).unwrap_or("");
            if text.is_empty() {
                return Some(Err("empty char".into()));
            }
            for c in text.chars().take(64) {
                o.widget.editor_mut().insert_symbol(c);
            }
        } else {
            let Some(nodes) = gallery_nodes(params) else { return Some(Err("unknown equation".into())) };
            let tpl = bridge::to_editor("", false, &Math { nodes, ..Default::default() });
            o.widget.editor_mut().insert_template(&tpl);
        }
        o.widget.sync_linear();
        return Some(Ok(json!({"equationEditor": true, "linear": o.widget.linear()})));
    }
    if APPLY_FIRST.contains(&id)
        && let Some(mut o) = app.equation.open.take()
    {
        let r = if o.widget.is_dirty() { apply(app, &mut o) } else { Ok(()) };
        app.equation.open = Some(o);
        if let Err(e) = r {
            return Some(Err(e));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use wordcraft_doc::math::MNode;
    use wordcraft_engine::Session;

    use super::*;
    use crate::Services;

    fn harness() -> Harness<'static, WordApp> {
        let app = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        for _ in 0..4 {
            h.step();
        }
        h
    }

    fn steps(h: &mut Harness<'static, WordApp>, n: usize) {
        for _ in 0..n {
            h.step();
        }
    }

    fn equations(app: &WordApp) -> Vec<Equation> {
        let s = &app.session;
        let mut out = Vec::new();
        for path in s.doc.para_paths(wordcraft_doc::StoryRef::Body) {
            if let Some(p) = s.doc.para(wordcraft_doc::StoryRef::Body, &path) {
                for off in p.object_offsets() {
                    if let Some(InlineObject::Equation { linear, display, math }) = p.object_at(off) {
                        out.push((linear.clone(), *display, math.clone()));
                    }
                }
            }
        }
        out
    }

    fn type_text(h: &mut Harness<'static, WordApp>, t: &str) {
        h.event(egui::Event::Text(t.into()));
        steps(h, 2);
    }

    #[test]
    fn alt_equals_opens_the_widget_and_enter_inserts_in_one_undo_step() {
        let mut h = harness();
        assert!(h.state().canvas.focused, "the page has the keyboard");
        h.key_press_modifiers(egui::Modifiers::ALT, egui::Key::Equals);
        steps(&mut h, 3);
        assert!(h.state().equation.is_open(), "the editor opened on the new equation");
        assert!(h.query_by_label("Insert").is_some() && h.query_by_label("Cancel").is_some());
        let canvas = h.state().equation.widget().map(|w| w.canvas_id());
        assert!(canvas.is_some_and(|c| h.ctx.memory(|m| m.has_focus(c))), "typing goes to the widget");
        let depth = h.state().session.undo_depth();
        type_text(&mut h, "x^2");
        assert_eq!(h.state().equation.widget().map(|w| w.linear()), Some("x^2".to_string()));
        assert!(equations(h.state())[0].2.nodes.is_empty(), "the document waits for Enter");
        h.key_press(egui::Key::Enter);
        steps(&mut h, 3);
        assert!(!h.state().equation.is_open(), "Enter closes the editor");
        assert!(h.state().session.math.is_none(), "and leaves the equation");
        let eqs = equations(h.state());
        assert_eq!(eqs.len(), 1);
        assert!(matches!(eqs[0].2.nodes.as_slice(), [MNode::Script { .. }]), "{:?}", eqs[0].2.nodes);
        assert_eq!(h.state().session.undo_depth(), depth, "insert and edit are one undo step");
        h.state_mut().run("edit.undo", json!({})).unwrap();
        steps(&mut h, 2);
        assert!(equations(h.state()).is_empty(), "one Undo takes the whole equation back");
    }

    #[test]
    fn esc_asks_once_then_discards_and_a_new_empty_equation_goes_away() {
        let mut h = harness();
        h.state_mut().run("insert.equation", json!({"linear": "a/b", "edit": false})).unwrap();
        steps(&mut h, 2);
        // Re-edit the existing equation, as a click on it does.
        let at = h.state().session.sel.focus.clone();
        h.state_mut().session.sel = Selection::caret(Pos { off: at.off.saturating_sub(wordcraft_doc::para::OBJ.len_utf8()), ..at });
        h.state_mut().run("equation.edit", json!({})).unwrap();
        steps(&mut h, 3);
        assert!(h.query_by_label("Update").is_some(), "an existing equation is updated");
        type_text(&mut h, "+c");
        h.key_press(egui::Key::Escape);
        steps(&mut h, 2);
        assert!(h.query_by_label(equashion_egui::DISCARD_PROMPT).is_some(), "the first Esc asks");
        assert!(h.state().equation.is_open());
        h.key_press(egui::Key::Escape);
        steps(&mut h, 3);
        assert!(!h.state().equation.is_open(), "the second Esc discards");
        assert_eq!(equations(h.state())[0].0, "a/b", "the document is unchanged");

        // A new equation closed untouched leaves nothing behind.
        h.state_mut().run("insert.equation", json!({})).unwrap();
        steps(&mut h, 3);
        assert_eq!(equations(h.state()).len(), 2);
        h.key_press(egui::Key::Escape);
        steps(&mut h, 3);
        assert!(!h.state().equation.is_open());
        assert_eq!(equations(h.state()).len(), 1, "the empty new equation is gone");
    }

    #[test]
    fn equation_tab_galleries_insert_into_the_widget() {
        let mut h = harness();
        h.state_mut().run("insert.equation", json!({})).unwrap();
        steps(&mut h, 3);
        h.state_mut().run("equation.insertStructure", json!({"id": "frac.stacked"})).unwrap();
        h.state_mut().run("equation.insertSymbol", json!({"char": "π"})).unwrap();
        steps(&mut h, 2);
        let tree = h.state().equation.widget().map(|w| w.math().clone()).unwrap_or_default();
        assert!(matches!(tree.body.first().map(|n| n.bare()), Some(equashion_egui::equashion_math::Node::Frac { .. })), "{tree:?}");
        assert!(equations(h.state())[0].2.nodes.is_empty(), "the document is untouched until Enter");
        h.key_press(egui::Key::Enter);
        steps(&mut h, 3);
        assert!(matches!(equations(h.state())[0].2.nodes.first(), Some(MNode::Frac { .. })));
    }

    #[test]
    fn leaving_the_equation_keeps_the_edit_and_options_apply_it_first() {
        let mut h = harness();
        h.state_mut().run("insert.equation", json!({})).unwrap();
        steps(&mut h, 3);
        type_text(&mut h, "y");
        // An Equation tab option applies the pending edit before changing the equation.
        h.state_mut().run("equation.display", json!({"value": false})).unwrap();
        steps(&mut h, 2);
        let e = &equations(h.state())[0];
        assert_eq!((e.0.as_str(), e.1), ("y", false));
        type_text(&mut h, "+1");
        // Close Equation (or a click elsewhere) ends the editing: the edit stays.
        h.state_mut().run("equation.exit", json!({})).unwrap();
        steps(&mut h, 3);
        assert!(!h.state().equation.is_open());
        assert_eq!(equations(h.state())[0].0, "y+1");
    }
}

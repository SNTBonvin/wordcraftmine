//! Exercise interface surfaces in both directions without changing document direction.
use crate::{WordApp, dialogs::Dialog};
use egui::vec2;
use egui_kittest::kittest::Queryable;
use serde_json::json;

struct LanguageGuard(crate::i18n::Lang);
impl LanguageGuard {
    fn new() -> Self {
        Self(crate::i18n::current())
    }
}
impl Drop for LanguageGuard {
    fn drop(&mut self) {
        crate::i18n::set_current(self.0);
    }
}

fn app(language: &str) -> WordApp {
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_engine::sample::sample_document()), Default::default());
    app.ui.language = language.to_owned();
    app
}

#[test]
fn formatted_labels_keep_document_values_logical() {
    let _language = LanguageGuard::new();
    crate::i18n::set_current(crate::i18n::Lang::from_code("qaa-rtl").unwrap());
    let logical = crate::i18n::tr(crate::i18n::current(), "{group} settings");
    assert_eq!(logical, "إعدادات {group}");
    assert_eq!(crate::i18n::logical_fmt(logical, &[("group", "واحد اثنين")]), "إعدادات واحد اثنين");
    assert_eq!(crate::i18n::fmt(crate::i18n::t("{group} settings"), &[("group", "واحد اثنين")]), "اثنين واحد إعدادات");
    assert_eq!(crate::i18n::fmt(crate::i18n::t("{group} settings"), &[("group", "draft-{n}.docx")]), "draft-{n}.docx إعدادات");
    let doc = wordcraft_doc::Document::new();
    let style = doc.styles.get("Normal").unwrap();
    let name = crate::i18n::logical_style_name(style);
    assert_eq!(name, "النمط العادي");
    assert_eq!(crate::i18n::style_name(style), "العادي النمط");
    assert_eq!(crate::i18n::fmt(crate::i18n::t("Current style: {style}"), &[("style", &name)]), "العادي النمط :الحالي النمط");
    assert_eq!(crate::panes::describe_props(&[json!({"prop": "bold", "value": false})]), ["عريض ليس"]);
    let spacing = crate::panes::describe_props(&[json!({"prop": "lineSpacing", "value": {"rule": "atLeast", "value": 12}})]);
    let expected = crate::ui_bidi::display("Line spacing: على الأقل 12 نقطة", Some(unicode_bidi::Level::rtl()));
    assert_eq!(spacing, [expected]);
}

#[test]
fn touchpad_moves_the_ribbon_without_changing_the_document() {
    let _language = LanguageGuard::new();
    let mut app = app("qaa-rtl");
    app.ui.animations = crate::theme::Animations::Off;
    let document = serde_json::to_value(&app.session.doc).unwrap();
    let ctx = egui::Context::default();
    let mut frame = |events| {
        let input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(600.0, 700.0))), events, ..Default::default() };
        ctx.run_ui(input, |ui| {
            app.logic(&ctx);
            app.ui(ui);
        })
    };
    for _ in 0..6 {
        frame(Vec::new()).drop_without_applying_deltas();
    }
    let caption_x = |output: egui::FullOutput| {
        let x = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "الحافظة" => Some(text.pos.x),
                _ => None,
            })
            .expect("clipboard group title");
        output.drop_without_applying_deltas();
        x
    };
    let before = caption_x(frame(Vec::new()));
    let after = caption_x(frame(vec![
        egui::Event::PointerMoved(egui::pos2(400.0, 110.0)),
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(40.0, 0.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        },
    ]));
    // The offset is applied on the next frame, preserving the input frame's geometry.
    assert_eq!(before, after);
    let scrolled = caption_x(frame(Vec::new()));
    assert!((scrolled - before - 40.0).abs() < 0.5, "{before} -> {scrolled}");
    frame(vec![egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(-20.0, 0.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    }])
    .drop_without_applying_deltas();
    let reversed = caption_x(frame(Vec::new()));
    assert!((reversed - before - 20.0).abs() < 0.5, "{scrolled} -> {reversed}");
    frame(vec![egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, -30.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    }])
    .drop_without_applying_deltas();
    let vertical = caption_x(frame(Vec::new()));
    assert!((vertical - reversed - 30.0).abs() < 0.5, "vertical: {reversed} -> {vertical}");
    assert_eq!(serde_json::to_value(&app.session.doc).unwrap(), document);
}

#[test]
fn interface_surfaces_render_in_both_directions() {
    let _language = LanguageGuard::new();
    for language in ["en", "qaa-rtl"] {
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app(language),
        );
        h.run_steps(3);
        let document = serde_json::to_value(&h.state().session.doc).unwrap();
        for page in ["home", "new", "open", "info", "saveAs", "print", "export", "options"] {
            h.state_mut().ui.backstage = true;
            h.state_mut().ui.backstage_page = page.into();
            h.run_steps(3);
            if language == "en" && page == "home" {
                let label = h.output().shapes.iter().find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "Studio handbook (sample)" => Some(&text.galley),
                    _ => None,
                });
                assert_eq!(label.expect("sample template caption").rows.len(), 1, "LTR keeps the original unwrapped template caption");
            }
        }
        for pane in wordcraft_engine::cmd::options::PANES {
            h.state_mut().ui.options_pane = pane.into();
            h.run_steps(3);
        }
        h.state_mut().ui.backstage = false;
        for command in
            ["view.navigationPane", "view.stylesPane", "edit.clipboardPane", "view.commentsPane", "view.accessibilityPane", "view.altTextPane"]
        {
            h.state_mut().run(command, json!({"value": true})).unwrap();
            h.run_steps(3);
            h.state_mut().run(command, json!({"value": false})).unwrap();
        }
        assert!(Dialog::open("restrictEditing", h.state_mut()).is_none());
        assert!(h.state().restrict.open);
        h.run_steps(3);
        h.state_mut().restrict.open = false;
        for name in [
            "lineNumbers",
            "hyphenation",
            "language",
            "envelopes",
            "labels",
            "caption",
            "index",
            "markEntry",
            "toc",
            "sourceManager",
            "createSource",
            "link",
            "bookmark",
            "wordCount",
            "zoom",
            "watermark",
            "phonetic",
            "newStyle",
            "manageStyles",
            "newTableStyle",
            "commands",
            "icons",
            "enclose",
            "pasteSpecial",
            "newRecipientList",
            "findRecipient",
            "fieldCode",
            "encryptPassword",
            "font",
            "paragraph",
            "asianTypography",
            "tabs",
            "borders",
            "pageBorders",
            "shading",
            "find",
            "replace",
            "goto",
            "pageSetup",
            "columns",
            "symbol",
            "specialCharacters",
            "field",
            "insertSmartArt",
            "insertChart",
            "about",
            "contributors",
            "models",
            "defineList",
            "trackChangesOptions",
            "customizeKeyboard",
            "customizeRibbon",
            "customizeQuickAccess",
            "noteOptions",
            "templates",
            "organizer",
            "compare",
            "combine",
            "autoCorrect",
        ] {
            let dialog = Dialog::open(name, h.state_mut());
            assert!(dialog.is_some(), "{language}: {name} did not open");
            h.state_mut().dialog = dialog;
            h.run_steps(3);
            assert!(h.state().dialog.is_some(), "{language}: {name} closed without input");
        }
        assert_eq!(serde_json::to_value(&h.state().session.doc).unwrap(), document, "{language}: rendering altered the document");
    }
}

/// Opt-in local evidence; no machine-dependent PNGs are committed.
#[test]
fn local_direction_screenshots() {
    let _language = LanguageGuard::new();
    let Some(directory) = std::env::var_os("WORDCRAFT_RTL_AUDIT_DIR") else { return };
    let directory = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&directory).unwrap();
    // Compare layout with identical English labels; script-specific fixtures
    // remain in the text-order regression tests rather than review screenshots.
    for language in ["en", "qaa-rtl-latin"] {
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1100.0, 750.0)).with_pixels_per_point(1.0).wgpu().build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app(language),
        );
        h.run_steps(6);
        for surface in ["home", "paste-menu", "insert", "options", "pane", "dialog", "save"] {
            match surface {
                "paste-menu" => {
                    h.state_mut().session.run("select.all", &json!({})).unwrap();
                    h.state_mut().session.run("edit.copy", &json!({})).unwrap();
                    h.state_mut().session.run("select.collapse", &json!({"end": false})).unwrap();
                    let pos = egui::pos2(if language == "en" { 45.0 } else { 1062.0 }, 100.0);
                    for pressed in [true, false] {
                        h.input_mut().events.extend([
                            egui::Event::PointerMoved(pos),
                            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE },
                        ]);
                        h.run_steps(1);
                    }
                }
                "insert" => {
                    egui::Popup::close_all(&h.ctx);
                    h.state_mut().ui.tab = "Insert".into();
                }
                "options" => {
                    h.state_mut().ui.backstage = true;
                    h.state_mut().ui.backstage_page = "options".into();
                }
                "pane" => {
                    h.state_mut().ui.backstage = false;
                    h.state_mut().session.view.nav_pane = true;
                }
                "dialog" => {
                    h.state_mut().session.view.nav_pane = false;
                    h.state_mut().dialog = Dialog::open("paragraph", h.state_mut());
                }
                "save" => {
                    let document = h.state().session.document_id();
                    h.state_mut().dialog =
                        Some(Dialog::SaveChanges { name: "RTL sample 2026.docx".into(), then: "file.new".into(), params: json!({}), document });
                }
                _ => {}
            }
            h.run_steps(6);
            h.render().unwrap().save(directory.join(format!("{language}-{surface}.png"))).unwrap();
        }
    }
}

#[test]
fn interface_direction_preserves_both_rtl_and_ltr_documents() {
    let _language = LanguageGuard::new();
    for rtl_document in [false, true] {
        for language in ["en", "qaa-rtl-latin"] {
            let mut state = app(language);
            state.session.run("select.all", &json!({})).unwrap();
            state.session.run(if rtl_document { "para.rtl" } else { "para.ltr" }, &json!({})).unwrap();
            let document = serde_json::to_value(&state.session.doc).unwrap();
            let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
                |ui, app: &mut WordApp| {
                    let ctx = ui.ctx().clone();
                    app.logic(&ctx);
                    app.ui(ui);
                },
                state,
            );
            h.run_steps(6);
            assert_eq!(serde_json::to_value(&h.state().session.doc).unwrap(), document);
            h.state_mut().run("ui.language", json!({"value": if language == "en" { "qaa-rtl-latin" } else { "en" }})).unwrap();
            h.run_steps(6);
            assert_eq!(
                serde_json::to_value(&h.state().session.doc).unwrap(),
                document,
                "{language}, RTL document {rtl_document}: language switch changed document"
            );
        }
    }
}

#[test]
fn navigation_locations_follow_interface_direction() {
    let _language = LanguageGuard::new();
    crate::i18n::set_current(crate::i18n::Lang::EN);
    assert_eq!(crate::i18n::location("Home › Clipboard"), "Home › Clipboard");
    crate::i18n::set_current(crate::i18n::Lang::from_code("qaa-rtl").unwrap());
    assert_eq!(crate::i18n::location("Home › Clipboard"), "الحافظة ‹ البداية");
    assert_eq!(crate::i18n::location("Clipboard"), "الحافظة");
}

/// Exercise the native scroll area's initial anchor and both edge buttons after
/// layout has settled. The wide/narrow cases catch unbounded RTL row regressions.
#[test]
fn rtl_ribbon_native_scroll_starts_right_and_edge_buttons_work() {
    let _language = LanguageGuard::new();
    for width in [600.0, 1440.0] {
        let mut app = app("qaa-rtl");
        let ctx = egui::Context::default();
        let mut frame = |events| {
            let input =
                egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 700.0))), events, ..Default::default() };
            ctx.run_ui(input, |ui| {
                app.logic(&ctx);
                app.ui(ui);
            })
        };
        let caption = |output: egui::FullOutput| {
            let x = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "الحافظة" => Some(text.pos.x),
                    _ => None,
                })
                .expect("clipboard group title");
            output.drop_without_applying_deltas();
            x
        };
        for _ in 0..8 {
            frame(Vec::new()).drop_without_applying_deltas();
        }
        let initial = caption(frame(Vec::new()));
        assert!(initial >= width - 180.0 && initial < width, "{width}: first group at {initial}");
        for x in [12.0, width - 12.0] {
            for pressed in [true, false] {
                let pos = egui::pos2(x, 150.0);
                frame(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE },
                ])
                .drop_without_applying_deltas();
            }
            let after = caption(frame(Vec::new()));
            if x < width / 2.0 {
                assert!(after > initial + 0.5 && after <= initial + 160.5, "{width}: left edge did not reveal/clamp content: {initial} -> {after}");
            } else {
                assert!((after - initial).abs() < 0.5, "{width}: right edge did not return to start: {initial} -> {after}");
            }
        }
    }
}

/// Same strings in both directions: translation length cannot explain a spacing
/// difference here. Only the direction changes, so group separators must mirror.
#[test]
fn ribbon_spacing_mirrors_with_identical_labels() {
    let _language = LanguageGuard::new();
    let width = 2560.0;
    let mut layouts = Vec::new();
    for language in ["en", "qaa-rtl-latin"] {
        let mut h = egui_kittest::Harness::builder().with_size(vec2(width, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app(language),
        );
        h.run_steps(10);
        let mut edges = h
            .output()
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::LineSegment { points, .. }
                    if (points[0].x - points[1].x).abs() < 0.1
                        && ((points[0].y - points[1].y).abs() - (crate::widgets::CONTENT_H + crate::widgets::LABEL_H - 8.0)).abs() < 0.1 =>
                {
                    Some(points[0].x)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        if language == "qaa-rtl-latin" {
            edges.iter_mut().for_each(|x| *x = width - *x);
        }
        edges.sort_by(f32::total_cmp);
        layouts.push(edges);
    }
    assert_eq!(layouts[0].len(), 7, "LTR group separators: {:?}", layouts[0]);
    assert_eq!(layouts[1].len(), 7, "RTL group separators: {:?}", layouts[1]);
    for (ltr, rtl) in layouts[0].iter().zip(&layouts[1]) {
        assert!((ltr - rtl).abs() < 1.0, "same-label group spacing: LTR {:?}, mirrored RTL {:?}", layouts[0], layouts[1]);
    }
}

#[test]
fn info_columns_follow_reading_order_without_changing_properties() {
    let _language = LanguageGuard::new();
    for language in ["en", "qaa-rtl-latin"] {
        let mut state = app(language);
        state.ui.backstage = true;
        state.ui.backstage_page = "info".into();
        let before = serde_json::to_value(&state.session.doc).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            state,
        );
        h.run_steps(6);
        let position = |label| {
            h.output()
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => Some(text.pos.x),
                    _ => None,
                })
                .expect("Info heading")
        };
        let properties = position("Properties");
        let statistics = position("Statistics");
        assert_eq!(properties > statistics, language != "en", "{language}: properties {properties}, statistics {statistics}");
        let field = h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Title").rect();
        assert!(field.width() >= 270.0, "{language}: title field collapsed to {field:?}");
        h.run_steps(30);
        let later = h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Title").rect();
        assert!((later.width() - field.width()).abs() < 1.0, "{language}: field width drifted: {field:?} -> {later:?}");
        let label_edge = |label| {
            h.output()
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => {
                        Some(text.pos.x + if language == "en" { text.galley.rect.min.x } else { text.galley.rect.max.x })
                    }
                    _ => None,
                })
                .expect("property label")
        };
        assert!((label_edge("Title") - label_edge("Keywords")).abs() < 1.0, "{language}: property labels do not share the reading edge");
        assert_eq!(serde_json::to_value(&h.state().session.doc).unwrap(), before);
    }
}

#[test]
fn plain_text_field_hints_align_at_the_interface_reading_edge() {
    let _language = LanguageGuard::new();
    for language in ["en", "qaa-rtl-latin"] {
        crate::i18n::set_current(crate::i18n::Lang::from_code(language).unwrap());
        let ctx = egui::Context::default();
        let mut text = String::new();
        let mut rect = egui::Rect::NOTHING;
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            rect = crate::rtl::root(ui, |ui| ui.add(crate::rtl::singleline(&mut text).hint_text("Search for templates").desired_width(360.0))).rect;
        });
        let (left, right) = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "Search for templates" => Some((text.pos.x, text.pos.x + text.galley.size().x)),
                _ => None,
            })
            .expect("text field hint");
        let gap = if language == "en" { left - rect.left() } else { rect.right() - right };
        assert!((0.0..8.0).contains(&gap), "{language}: hint gap {gap}");
        assert!(text.is_empty());
        output.drop_without_applying_deltas();
    }
}

#[test]
fn control_language_change_installs_the_new_languages_fonts_in_the_same_frame() {
    let _language = LanguageGuard::new();
    let (send, receive) = std::sync::mpsc::channel();
    let mut state = app("en").with_control(receive);
    let ctx = egui::Context::default();
    ctx.run_ui(egui::RawInput::default(), |ui| state.logic(ui.ctx())).drop_without_applying_deltas();
    for (language, hans) in [("zh-hans", true), ("en", false), ("zh-hans", true)] {
        let (request, reply) = crate::ControlRequest::new("ui.language", json!({"value": language}));
        send.send(request).unwrap();
        ctx.run_ui(egui::RawInput::default(), |ui| state.logic(ui.ctx())).drop_without_applying_deltas();
        assert_eq!(reply.try_recv().unwrap()["ok"], true);
        assert_eq!(state.fonts_hans, hans, "{language}: installed the previous language's fonts");
    }
}

use std::cell::RefCell;
use std::rc::Rc;

use super::*;
use crate::{Services, dialogs::Dialog};
use wordcraft_doc::Document;
use wordcraft_engine::Session;

#[derive(Default)]
struct State {
    events: Vec<Event>,
    bar: Option<MenuBar>,
    system: Vec<String>,
}

struct Fake(Rc<RefCell<State>>);

impl Backend for Fake {
    fn sync(&mut self, bar: &MenuBar) {
        self.0.borrow_mut().bar = Some(bar.clone());
    }
    fn drain(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.0.borrow_mut().events)
    }
    fn system_action(&mut self, id: &str) {
        self.0.borrow_mut().system.push(id.into());
    }
}

fn setup() -> (WordApp, Context, Rc<RefCell<State>>) {
    let ctx = Context::default();
    let state = Rc::new(RefCell::new(State::default()));
    let mut app = WordApp::new(Session::new(Document::new()), Services::default());
    app.ui.language = "en".into();
    app.canvas.focused = true;
    app.services.native_menu = Some(NativeMenu::new(Box::new(Fake(state.clone())), ctx.clone()));
    (app, ctx, state)
}

fn click(state: &Rc<RefCell<State>>, id: &str) {
    state.borrow_mut().events.push(Event::Click(id.into()));
}
fn key(state: &Rc<RefCell<State>>, chord: &str) {
    state.borrow_mut().events.push(Event::Key(Chord::parse(chord).unwrap()));
}

fn frame(app: &mut WordApp, ctx: &Context, mut raw: egui::RawInput) -> egui::FullOutput {
    app.raw_input_hook(&mut raw);
    let mut output = ctx.run_ui(raw, |ui| {
        app.ctx = Some(ui.ctx().clone());
        run(app, ui.ctx());
        crate::keys::canvas_events(app, ui.ctx());
        sync(app, ui.ctx());
    });
    output.textures_delta.clear();
    output
}

fn text(app: &WordApp) -> String {
    app.session.doc.plain_text(wordcraft_doc::StoryRef::Body)
}

#[test]
fn layout_has_mac_roles_and_only_existing_commands() {
    let (app, ctx, _) = setup();
    let bar = menu_bar(&app, &ctx);
    assert_eq!(
        bar.menus.iter().map(|m| m.title.as_str()).collect::<Vec<_>>(),
        ["WordCraft", "File", "Edit", "Insert", "Format", "View", "Window", "Help"]
    );
    assert_eq!(bar.menus[0].role, Role::App);
    assert_eq!(bar.menus[6].role, Role::Window);
    assert_eq!(bar.menus[7].role, Role::Help);
    let mut ids = std::collections::HashSet::new();
    for it in bar.items() {
        assert!(ids.insert(&it.id), "duplicate row {}", it.id);
        assert!(
            app.session.registry.get(&it.command).is_some()
                || it.command == "ui.dialog"
                || matches!(it.id.as_str(), HIDE | HIDE_OTHERS | SHOW_ALL | MINIMIZE),
            "{}",
            it.command
        );
    }
    assert_eq!(bar.find(QUIT).unwrap().command, "file.close");
    assert_eq!(bar.find(QUIT).unwrap().shortcut, Chord::parse("Mod+Q"));
    assert_eq!(bar.find("file.options").unwrap().shortcut, Chord::parse("Mod+,"));
    assert!(!app.ui.ribbon_collapsed, "installing a menu doesn't hide the ribbon");
}

#[test]
fn system_shortcuts_respect_the_entire_effective_keymap() {
    let (mut app, ctx, _) = setup();
    assert_eq!(menu_bar(&app, &ctx).find(HIDE).unwrap().shortcut, Chord::parse("Mod+Ctrl+H"));
    let reg = app.session.registry.clone();
    for sc in ["Mod+Q", "Mod+,", "Mod+Ctrl+H", "Mod+M", "Mod+Ctrl+M", "Mod+Alt+H"] {
        app.session.keymap.assign(&reg, "format.bold", sc).unwrap();
    }
    let bar = menu_bar(&app, &ctx);
    for id in [QUIT, "file.options", HIDE, MINIMIZE, HIDE_OTHERS] {
        assert!(bar.find(id).unwrap().shortcut.is_none(), "{id}");
    }
    app.session.keymap.assign(&reg, "edit.copy", "Mod+Shift+J").unwrap();
    app.session.keymap.remove(&reg, "edit.copy", "Mod+C").unwrap();
    assert_eq!(menu_bar(&app, &ctx).find("edit.copy").unwrap().shortcut, Chord::parse("Mod+Shift+J"));
}

#[test]
fn state_changes_keep_structure_and_language_and_keys_refresh_it() {
    let (mut app, ctx, _) = setup();
    let before = menu_bar(&app, &ctx);
    app.run("text.insert", json!({"text": "original"})).unwrap();
    app.run("view.ruler", json!({})).unwrap();
    let after = menu_bar(&app, &ctx);
    assert_eq!(structure_key(&before), structure_key(&after));
    assert!(!before.find("edit.undo").unwrap().enabled && after.find("edit.undo").unwrap().enabled);
    assert_ne!(before.find("view.ruler").unwrap().checked, after.find("view.ruler").unwrap().checked);
    app.ui.language = "zh-hans".into();
    let chinese = menu_bar(&app, &ctx);
    assert_eq!(chinese.menus[1].title, "文件");
    assert_ne!(structure_key(&after), structure_key(&chinese));
    assert_eq!(chinese.find("file.open").unwrap().command, "file.open");
    assert!(chinese.find(QUIT).unwrap().label.contains("WordCraft"));
}

#[test]
fn quit_new_and_open_keep_the_save_cancel_flow() {
    for id in [QUIT, "file.new", "file.open", "file.close"] {
        let (mut app, ctx, state) = setup();
        app.services.pick_open = Some(Box::new(|_| Some("/synthetic/chosen.docx".into())));
        app.run("text.insert", json!({"text": "unsaved"})).unwrap();
        click(&state, id);
        frame(&mut app, &ctx, Default::default());
        assert!(matches!(app.dialog, Some(Dialog::SaveChanges { .. })), "{id}");
        assert!(!app.quit_requested);
        app.run("ui.saveChanges", json!({"answer": "cancel"})).unwrap();
        assert_eq!(text(&app).trim(), "unsaved");
        assert!(!app.quit_requested);
    }
    let (mut app, ctx, state) = setup();
    app.run("text.insert", json!({"text": "unsaved"})).unwrap();
    key(&state, "Mod+Q");
    frame(&mut app, &ctx, Default::default());
    app.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
    assert!(app.quit_requested);
}

#[test]
fn print_menu_and_shortcut_open_settings_without_submitting() {
    for shortcut in [false, true] {
        let (mut app, ctx, state) = setup();
        app.services.print_native = Some(Box::new(|_, _, _| panic!("opening Print must not submit a job")));
        app.printing.printers.push(crate::printing::Printer { name: "Test queue".into(), default: true });
        app.run("ui.printSettings", json!({"printer": "Test queue", "copies": 3, "grayscale": true})).unwrap();
        assert!(menu_bar(&app, &ctx).find("file.print").unwrap().enabled);
        if shortcut {
            key(&state, "Mod+P");
        } else {
            click(&state, "file.print");
        }
        frame(&mut app, &ctx, Default::default());
        assert!(app.ui.backstage && app.ui.backstage_page == "print");
        assert_eq!(app.printing.settings.printer, "Test queue");
        assert_eq!(app.printing.settings.copies, 3);
        assert!(app.printing.settings.grayscale);
        assert!(!app.printing.busy());
    }
}

#[test]
fn native_quit_keeps_the_host_print_submission_guard() {
    let (mut app, ctx, state) = setup();
    let (tx, rx) = std::sync::mpsc::channel();
    app.printing.begin(rx).unwrap();
    key(&state, "Mod+Q");
    frame(&mut app, &ctx, Default::default());
    assert!(app.quit_requested);
    assert!(!app.close_requested(), "the host must keep the window alive until submission completes");
    tx.send(Ok("test job submitted".into())).unwrap();
    app.poll_printing(&ctx);
    assert!(!app.printing.busy());
    assert!(app.close_requested());
}

#[test]
fn modal_blocks_queued_clicks_and_stale_enabled_rows() {
    let (mut app, ctx, state) = setup();
    let ruler = app.session.view.ruler;
    // Both clicks were queued while the menu was enabled; the first opens a modal.
    click(&state, "app.about");
    click(&state, "view.ruler");
    frame(&mut app, &ctx, Default::default());
    assert!(app.dialog.is_some());
    assert_eq!(app.session.view.ruler, ruler);
    assert!(!state.borrow().bar.as_ref().unwrap().find("view.ruler").unwrap().enabled);
    app.dialog = None;
    click(&state, "view.ruler");
    frame(&mut app, &ctx, Default::default());
    assert_eq!(app.session.view.ruler, !ruler);
}

#[test]
fn key_equivalent_has_one_press_before_its_real_release() {
    let (mut app, ctx, state) = setup();
    app.session.keymap.assign(&app.session.registry.clone(), "view.ruler", "Mod+R").unwrap();
    let ruler = app.session.view.ruler;
    let mut release = Chord::parse("Mod+R").unwrap().key_event().unwrap();
    if let egui::Event::Key { pressed, .. } = &mut release {
        *pressed = false;
    }
    key(&state, "Mod+R");
    let mut raw = egui::RawInput { events: vec![release], ..Default::default() };
    app.raw_input_hook(&mut raw);
    assert!(matches!(raw.events.as_slice(), [egui::Event::Key { pressed: true, .. }, egui::Event::Key { pressed: false, .. }]));
    frame(&mut app, &ctx, raw);
    assert_eq!(app.session.view.ruler, !ruler);
}

#[test]
fn paste_equivalent_inserts_text_or_a_picture_once() {
    for same_frame in [false, true] {
        for picture in [false, true] {
            let (mut app, ctx, state) = setup();
            let calls = Rc::new(RefCell::new(0));
            let count = calls.clone();
            app.services.clipboard_picture = Some(Box::new(move || {
                *count.borrow_mut() += 1;
                Some(crate::paste_picture::ClipboardPicture::Pixels { width: 1, height: 1, rgba: vec![30, 90, 200, 255] })
            }));
            if !picture {
                app.services.clipboard_text = Some(Box::new(|| Some("pasted".into())));
            }
            let mut release = Chord::parse("Mod+V").unwrap().key_event().unwrap();
            if let egui::Event::Key { pressed, .. } = &mut release {
                *pressed = false;
            }
            key(&state, "Mod+V");
            frame(&mut app, &ctx, egui::RawInput { events: if same_frame { vec![release.clone()] } else { vec![] }, ..Default::default() });
            if !same_frame {
                frame(&mut app, &ctx, egui::RawInput { events: vec![release], ..Default::default() });
            }
            if picture {
                assert_eq!(*calls.borrow(), 1, "one picture request per key equivalent");
                assert!(app.session.can_undo());
                app.run("edit.undo", json!({})).unwrap();
                assert!(!app.session.can_undo(), "one image insert is one undo step");
            } else {
                assert_eq!(text(&app).trim(), "pasted");
            }
        }
    }
}

#[test]
fn insert_and_format_reuse_dialogs_and_selection_state() {
    let (mut app, ctx, state) = setup();
    for (id, name) in [("insert.tableDialog", "insertTable"), ("insert.link", "link"), ("format.fontDialog", "font"), ("para.dialog", "paragraph")] {
        click(&state, id);
        frame(&mut app, &ctx, Default::default());
        assert_eq!(app.dialog.as_ref().map(Dialog::name), Some(name));
        app.dialog = None;
    }
    click(&state, "format.bold");
    click(&state, "para.alignCenter");
    frame(&mut app, &ctx, Default::default());
    let bar = menu_bar(&app, &ctx);
    assert_eq!(bar.find("format.bold").unwrap().checked, Some(true));
    assert_eq!(bar.find("para.alignCenter").unwrap().checked, Some(true));
    assert_eq!(bar.find("para.alignLeft").unwrap().checked, Some(false));
    app.run("text.insert", json!({"text": "formatted"})).unwrap();
    app.run("select.all", json!({})).unwrap();
    assert_eq!(menu_bar(&app, &ctx).find("format.bold").unwrap().checked, Some(true));
    app.run("view.readMode", json!({})).unwrap();
    for id in ["format.bold", "para.dialog", "insert.tableDialog", "insert.pageBreak"] {
        assert!(!menu_bar(&app, &ctx).find(id).unwrap().enabled, "{id} in read mode");
    }
}

#[test]
fn clipboard_equivalent_works_with_a_ribbon_button_focused() {
    let (mut app, ctx, state) = setup();
    app.canvas.focused = false;
    app.services.clipboard_text = Some(Box::new(|| Some("from clipboard".into())));
    key(&state, "Mod+V");
    frame(&mut app, &ctx, Default::default());
    assert_eq!(text(&app).trim(), "from clipboard");
    app.run("select.all", json!({})).unwrap();
    key(&state, "Mod+C");
    let out = frame(&mut app, &ctx, Default::default());
    assert!(out.platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t == "from clipboard")));
}

#[test]
fn document_actions_leave_the_start_screen_and_settings_keep_it() {
    for shortcut in [false, true] {
        let (mut app, ctx, state) = setup();
        app.start_screen();
        assert!(app.ui.backstage);
        if shortcut {
            key(&state, "Mod+N");
        } else {
            click(&state, "file.new");
        }
        frame(&mut app, &ctx, Default::default());
        assert!(!app.ui.backstage);
        assert!(!app.ui.ribbon_collapsed);
        click(&state, "file.options");
        frame(&mut app, &ctx, Default::default());
        assert!(app.ui.backstage);
        assert_eq!(app.ui.backstage_page, "options");
        click(&state, "insert.tableDialog");
        frame(&mut app, &ctx, Default::default());
        assert!(!app.ui.backstage);
        assert_eq!(app.dialog.as_ref().map(Dialog::name), Some("insertTable"));
    }
}

#[test]
fn quit_waits_for_a_successful_save() {
    let dir = std::env::temp_dir().join(format!(
        "wordcraft-native-menu-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    for outcome in ["cancel", "error", "save"] {
        let (mut app, ctx, state) = setup();
        let path = dir.join(if outcome == "error" { "missing/failed.docx" } else { "saved.docx" });
        let picked = (outcome != "cancel").then(|| path.to_string_lossy().to_string());
        app.services.pick_save = Some(Box::new(move |_| picked.clone()));
        app.run("text.insert", json!({"text": "keep this writing"})).unwrap();
        click(&state, QUIT);
        frame(&mut app, &ctx, Default::default());
        assert!(matches!(app.dialog, Some(Dialog::SaveChanges { .. })));
        app.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
        assert_eq!(app.quit_requested, outcome == "save", "{outcome}");
        assert_eq!(text(&app).trim(), "keep this writing");
        if outcome == "save" {
            let saved = wordcraft_engine::io::open_path(&path).unwrap();
            assert_eq!(saved.plain_text(wordcraft_doc::StoryRef::Body).trim(), "keep this writing");
        } else {
            assert!(app.session.dirty);
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn modal_also_blocks_keys_queued_before_it_opened() {
    let (mut app, ctx, state) = setup();
    key(&state, "Mod+D"); // Font dialog.
    key(&state, "Mod+B");
    frame(&mut app, &ctx, Default::default());
    assert_eq!(app.dialog.as_ref().map(Dialog::name), Some("font"));
    assert_eq!(wordcraft_engine::cmd::format::state(&app.session)["bold"], false);
}

#[test]
fn modal_paste_release_and_remapped_paste_never_insert_a_picture() {
    let (mut app, ctx, state) = setup();
    app.services.clipboard_picture = Some(Box::new(|| panic!("a blocked/remapped paste must not read the picture")));
    let mut release = Chord::parse("Mod+V").unwrap().key_event().unwrap();
    if let egui::Event::Key { pressed, .. } = &mut release {
        *pressed = false;
    }
    app.dialog = Some(Dialog::About { tab: Default::default() });
    key(&state, "Mod+V");
    frame(&mut app, &ctx, Default::default());
    app.dialog = None;
    frame(&mut app, &ctx, egui::RawInput { events: vec![release.clone()], ..Default::default() });
    app.session.keymap.assign(&app.session.registry.clone(), "view.ruler", "Mod+V").unwrap();
    let ruler = app.session.view.ruler;
    key(&state, "Mod+V");
    frame(&mut app, &ctx, egui::RawInput { events: vec![release], ..Default::default() });
    assert_eq!(app.session.view.ruler, !ruler);
    assert!(!app.session.can_undo());
}

#[test]
fn native_file_dialog_owns_editing_and_blocks_stale_document_actions() {
    let (mut app, ctx, state) = setup();
    let answer = Rc::new(RefCell::new(None));
    let pending = answer.clone();
    app.services.file_dialog = Some(Box::new(move |_| {
        let (tx, rx) = std::sync::mpsc::channel();
        *pending.borrow_mut() = Some(tx);
        rx
    }));
    app.run("text.insert", json!({"text": "document"})).unwrap();
    let before = menu_bar(&app, &ctx);
    app.run("file.open", json!({})).unwrap();
    let during = menu_bar(&app, &ctx);
    assert_ne!(structure_key(&before), structure_key(&during));
    assert!(during.find("edit.paste").unwrap().native_text);
    assert!(during.find("edit.paste").unwrap().enabled);
    assert!(!during.find("format.bold").unwrap().enabled);
    for id in ["edit.cut", "edit.paste", "edit.undo"] {
        click(&state, id);
    }
    key(&state, "Mod+V");
    frame(&mut app, &ctx, Default::default());
    assert_eq!(text(&app).trim(), "document");
    answer.borrow_mut().take().unwrap().send(Ok(None)).unwrap();
    app.poll_file_dialog();
    let after = menu_bar(&app, &ctx);
    assert_eq!(structure_key(&before), structure_key(&after));
    assert!(!after.find("edit.paste").unwrap().native_text);
}

#[test]
fn document_copy_click_writes_the_system_clipboard_output() {
    let (mut app, ctx, state) = setup();
    app.run("text.insert", json!({"text": "copy me"})).unwrap();
    app.run("select.all", json!({})).unwrap();
    click(&state, "edit.copy");
    let output = frame(&mut app, &ctx, Default::default());
    assert!(output.platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t == "copy me")));
}

#[test]
fn text_field_menu_paste_cut_and_undo_never_edit_the_document() {
    let (mut app, ctx, state) = setup();
    app.run("text.insert", json!({"text": "document"})).unwrap();
    app.services.clipboard_text = Some(Box::new(|| Some("field".into())));
    let mut field = String::new();
    {
        let mut time = 0.0;
        let mut draw = |app: &mut WordApp| {
            time += 1.0;
            let mut raw = egui::RawInput { time: Some(time), ..Default::default() };
            app.raw_input_hook(&mut raw);
            let mut output = ctx.run_ui(raw, |ui| {
                run(app, ui.ctx());
                ui.add(egui::TextEdit::singleline(&mut field).id(egui::Id::new("field"))).request_focus();
                sync(app, ui.ctx());
            });
            output.textures_delta.clear();
            output
        };
        draw(&mut app);
        draw(&mut app);
        // A modal with a text field still permits editing that field.
        app.dialog = Some(Dialog::About { tab: Default::default() });
        key(&state, "Mod+V");
        draw(&mut app);
        click(&state, "select.all");
        draw(&mut app);
        click(&state, "edit.copy");
        let output = draw(&mut app);
        assert!(output.platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t == "field")));
        click(&state, "edit.cut");
        draw(&mut app);
        click(&state, "edit.undo");
        draw(&mut app);
    }
    assert_eq!(field, "field");
    assert_eq!(text(&app).trim(), "document");
}

#[test]
fn recent_rows_carry_the_path_and_removed_rows_do_nothing() {
    let (mut app, ctx, state) = setup();
    app.ui.recent = vec!["/synthetic/one & two.docx".into()];
    let bar = menu_bar(&app, &ctx);
    let row = bar.find("recent:/synthetic/one & two.docx").unwrap();
    assert_eq!(row.params, json!({"path": "/synthetic/one & two.docx"}));
    click(&state, &row.id);
    app.ui.recent.clear();
    frame(&mut app, &ctx, Default::default());
    assert!(app.session.path.is_none());
    assert!(app.dialog.is_none());
}

#[test]
fn bare_and_option_typing_never_become_key_equivalents() {
    for key in ["B", "Alt+Q", "Shift+A", "Mod++", "Mod+F24"] {
        assert!(!Chord::parse(key).unwrap().native_ok());
    }
    for key in ["Mod+B", "Ctrl+F", "F12"] {
        assert!(Chord::parse(key).unwrap().native_ok());
    }
    assert_ne!(Chord::parse("Mod+M"), Chord::parse("Mod+Ctrl+M"));
}

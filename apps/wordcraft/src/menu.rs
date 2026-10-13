//! The native macOS menu bar (#255): WordCraft, File, Edit, View, Insert, Format, Tools, Table,
//! Window and Help at the top of the screen, each item running the same command the ribbon does.
//!
//! Eframe has no menu API, so the menus are AppKit objects from `muda`, installed on `NSApp` at
//! start-up and kept alive for the app's life. The menu is defined as plain data ([`menu_tree`])
//! so a test can check every item's command; [`AppMenu::install`] turns that data into AppKit
//! objects and [`AppMenu::poll`] drains their events each frame.
//!
//! The items carry no key equivalents where the keyboard already handles them (clipboard, undo):
//! setting one would let AppKit consume the key before the focused text box hears it, so the box
//! would lose Cmd+C, Cmd+V and the like. The items that are safe to claim show their shortcut.

use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use serde_json::{json, Value};
use wordcraft_ui_egui::WordApp;

/// One entry in a menu.
enum Node {
    /// A command: its label, the command id, the params for it and an optional key equivalent.
    Item { label: &'static str, command: &'static str, params: Value, shortcut: Option<&'static str> },
    /// A divider.
    Sep,
    /// A standard AppKit item (Quit, Hide, Services, Minimize…).
    Pre(Pre),
    /// A nested menu.
    Sub { title: &'static str, children: Vec<Node> },
}

/// The standard AppKit items the menus use.
#[derive(Clone, Copy)]
enum Pre {
    Services,
    Hide,
    HideOthers,
    ShowAll,
    Minimize,
    Zoom,
    BringAllToFront,
}

/// A command item.
fn item(label: &'static str, command: &'static str) -> Node {
    Node::Item { label, command, params: json!({}), shortcut: None }
}

/// A command item with a key equivalent (`cmd+s`).
fn key(label: &'static str, command: &'static str, shortcut: &'static str) -> Node {
    Node::Item { label, command, params: json!({}), shortcut: Some(shortcut) }
}

/// An item that opens the dialog `name` (`ui.dialog`).
fn dialog(label: &'static str, name: &str) -> Node {
    Node::Item { label, command: "ui.dialog", params: json!({"name": name}), shortcut: None }
}

/// A divider.
fn sep() -> Node {
    Node::Sep
}

/// A nested menu.
fn sub(title: &'static str, children: Vec<Node>) -> Node {
    Node::Sub { title, children }
}

/// The whole menu bar: each top-level menu's title and its entries.
fn menu_tree() -> Vec<(&'static str, Vec<Node>)> {
    vec![
        (
            "WordCraft",
            vec![
                dialog("About WordCraft", "about"),
                sep(),
                key("Settings…", "file.options", "cmd+,"),
                sep(),
                Node::Pre(Pre::Services),
                sep(),
                Node::Pre(Pre::Hide),
                Node::Pre(Pre::HideOthers),
                Node::Pre(Pre::ShowAll),
                sep(),
                // Not a predefined Quit: that calls `terminate:` and would drop unsaved work
                // without asking. `file.close` is the app's close path (asks first, then quits).
                key("Quit WordCraft", "file.close", "cmd+q"),
            ],
        ),
        (
            "File",
            vec![
                key("New", "file.new", "cmd+n"),
                item("New from Template…", "file.newFromTemplate"),
                key("Open…", "file.open", "cmd+o"),
                sep(),
                key("Close", "file.close", "cmd+w"),
                key("Save", "file.save", "cmd+s"),
                key("Save As…", "file.saveAs", "cmd+shift+s"),
                item("Save as Template…", "file.saveTemplate"),
                sep(),
                item("Share", "file.share"),
                item("Export PDF…", "file.exportPdf"),
                item("Export Page as PNG…", "file.exportPng"),
                key("Print…", "file.print", "cmd+p"),
                sep(),
                item("Info", "file.info"),
                item("Properties", "file.properties"),
                item("Protect Document", "file.protect"),
            ],
        ),
        (
            "Edit",
            vec![
                item("Undo", "edit.undo"),
                item("Redo", "edit.redo"),
                sep(),
                item("Cut", "edit.cut"),
                item("Copy", "edit.copy"),
                item("Paste", "edit.paste"),
                item("Paste Special…", "edit.pasteSpecial"),
                item("Format Painter", "edit.formatPainter"),
                sep(),
                item("Select All", "select.all"),
                sep(),
                key("Find…", "edit.find", "cmd+f"),
                item("Advanced Find…", "edit.advancedFind"),
                item("Find and Replace…", "edit.findReplace"),
                item("Go To…", "edit.goto"),
            ],
        ),
        (
            "View",
            vec![
                item("Read Mode", "view.readMode"),
                item("Print Layout", "view.printLayout"),
                item("Web Layout", "view.webLayout"),
                item("Draft", "view.draft"),
                item("Focus", "view.focus"),
                item("Immersive Reader", "view.immersive"),
                sep(),
                item("Show/Hide ¶", "view.marks"),
                item("Gridlines", "view.gridlines"),
                item("Field Codes", "view.fieldCodes"),
                item("Hide White Space", "view.hideWhiteSpace"),
                sep(),
                item("Dark Mode", "view.darkMode"),
                dialog("Zoom…", "zoom"),
            ],
        ),
        (
            "Insert",
            vec![
                item("Page Break", "insert.pageBreak"),
                item("Blank Page", "insert.blankPage"),
                item("Cover Page", "insert.coverPage"),
                sep(),
                dialog("Table…", "insertTable"),
                item("Pictures…", "insert.picture"),
                item("Chart…", "insert.chart"),
                item("Icons…", "insert.icon"),
                sep(),
                item("Header", "insert.header"),
                item("Footer", "insert.footer"),
                item("Page Number", "insert.pageNumber"),
                sep(),
                item("Equation", "insert.equation"),
                dialog("Symbol…", "symbol"),
                sep(),
                dialog("Link…", "link"),
                dialog("Bookmark…", "bookmark"),
                item("Cross-reference…", "insert.crossReference"),
                item("Date & Time…", "insert.dateTime"),
                dialog("Field…", "field"),
            ],
        ),
        (
            "Format",
            vec![
                key("Bold", "format.bold", "cmd+b"),
                key("Italic", "format.italic", "cmd+i"),
                key("Underline", "format.underline", "cmd+u"),
                item("Strikethrough", "format.strikethrough"),
                item("Subscript", "format.subscript"),
                item("Superscript", "format.superscript"),
                sep(),
                key("Font…", "format.fontDialog", "cmd+d"),
                dialog("Paragraph…", "paragraph"),
                item("Bullets", "para.bullets"),
                item("Numbering", "para.numbering"),
                item("Text Effects…", "format.textEffects"),
                sep(),
                item("Clear All Formatting", "format.clear"),
            ],
        ),
        (
            "Tools",
            vec![
                item("Spelling & Grammar…", "review.spelling"),
                item("Thesaurus…", "review.thesaurus"),
                item("Word Count…", "review.wordCount"),
                sep(),
                item("Track Changes", "review.trackChanges"),
                sep(),
                dialog("Compare…", "compare"),
                dialog("Combine…", "combine"),
                sep(),
                item("Restrict Editing", "review.restrict"),
                dialog("Customize Keyboard…", "customizeKeyboard"),
                item("Options…", "file.options"),
            ],
        ),
        (
            "Table",
            vec![
                sub(
                    "Insert",
                    vec![
                        item("Rows Above", "table.insertRowAbove"),
                        item("Rows Below", "table.insertRowBelow"),
                        item("Columns Left", "table.insertColumnLeft"),
                        item("Columns Right", "table.insertColumnRight"),
                    ],
                ),
                sub(
                    "Delete",
                    vec![
                        item("Rows", "table.deleteRow"),
                        item("Columns", "table.deleteColumn"),
                        item("Cells…", "table.deleteCells"),
                        item("Table", "table.deleteTable"),
                    ],
                ),
                sub(
                    "Select",
                    vec![
                        item("Table", "table.selectTable"),
                        item("Row", "table.selectRow"),
                        item("Column", "table.selectColumn"),
                        item("Cell", "table.selectCell"),
                    ],
                ),
                sep(),
                item("Merge Cells", "table.merge"),
                item("Split Cells…", "table.split"),
                item("Split Table", "table.splitTable"),
                sep(),
                item("AutoFit", "table.autofit"),
                sub("Convert", vec![item("Text to Table…", "table.fromText"), item("Table to Text…", "table.toText")]),
                item("Properties…", "table.properties"),
            ],
        ),
        (
            "Window",
            vec![Node::Pre(Pre::Minimize), Node::Pre(Pre::Zoom), sep(), Node::Pre(Pre::BringAllToFront)],
        ),
        (
            "Help",
            vec![
                dialog("Search Commands…", "commands"),
                sep(),
                dialog("Keyboard Shortcuts…", "customizeKeyboard"),
                item("Check Accessibility", "file.accessibility"),
            ],
        ),
    ]
}

/// Name the process "WordCraft" so the macOS app menu reads right when the dev binary is run
/// without a bundle: macOS takes the app-menu title from the process/bundle name, and the bare
/// executable is `wordcraft`. The packaged app sets `CFBundleName` itself, so this only touches
/// dev builds. Call on the main thread before the event loop starts.
pub fn name_process() {
    use objc2_foundation::{NSProcessInfo, NSString};
    let info = NSProcessInfo::processInfo();
    info.setProcessName(&NSString::from_str("WordCraft"));
    log::info!("process name: {}", info.processName());
}

/// A menu item's command: the id and params [`WordApp::run`] takes.
struct Action {
    id: String,
    command: &'static str,
    params: Value,
}

/// The menu bar and the commands its items run. Dropping it removes the menu, so the app holds it.
pub struct AppMenu {
    _menu: Menu,
    // The root owns its submenus, but each `Submenu` removes itself from `NSApp` on drop, so keep
    // them (the Window and Help ones especially) alive for the app's life.
    _submenus: Vec<Submenu>,
    actions: Vec<Action>,
}

impl AppMenu {
    /// Build the menu bar and set it on `NSApp`. Call on the main thread after the app exists.
    pub fn install() -> Self {
        let mut actions = Vec::new();
        let menu = Menu::new();
        let mut submenus = Vec::new();
        for (title, nodes) in menu_tree() {
            let submenu = Submenu::new(title, true);
            for node in &nodes {
                append(&mut actions, &submenu, node);
            }
            if let Err(e) = menu.append(&submenu) {
                log::warn!("menu {title}: {e}");
            }
            submenus.push(submenu);
        }
        menu.init_for_nsapp();
        if let Some(w) = submenus.iter().find(|s| s.text() == "Window") {
            w.set_as_windows_menu_for_nsapp();
        }
        if let Some(h) = submenus.iter().find(|s| s.text() == "Help") {
            h.set_as_help_menu_for_nsapp();
        }
        log::info!("native menu bar: {}", submenus.iter().map(|s| s.text()).collect::<Vec<_>>().join(", "));
        Self { _menu: menu, _submenus: submenus, actions }
    }

    /// Run the commands of the menu items chosen since the last frame.
    pub fn poll(&self, app: &mut WordApp) {
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            let id = event.id.0.as_str();
            if let Some(action) = self.actions.iter().find(|a| a.id == id)
                && let Err(e) = app.run(action.command, action.params.clone())
            {
                log::warn!("menu {}: {e}", action.command);
            }
        }
    }
}

/// Append `node` to `menu`, registering the command of an item.
fn append(actions: &mut Vec<Action>, menu: &Submenu, node: &Node) {
    match node {
        Node::Sep => predefined(menu, &PredefinedMenuItem::separator()),
        Node::Pre(p) => predefined(menu, &standard(*p)),
        Node::Item { label, command, params, shortcut } => {
            let id = format!("wordcraft.menu.{}", actions.len());
            actions.push(Action { id: id.clone(), command, params: params.clone() });
            let item = MenuItem::with_id(id, label, true, shortcut.and_then(parse_shortcut));
            if let Err(e) = menu.append(&item) {
                log::warn!("menu item {label}: {e}");
            }
        }
        Node::Sub { title, children } => {
            let submenu = Submenu::new(title, true);
            for child in children {
                append(actions, &submenu, child);
            }
            if let Err(e) = menu.append(&submenu) {
                log::warn!("menu {title}: {e}");
            }
        }
    }
}

/// The AppKit item for a [`Pre`] kind.
fn standard(kind: Pre) -> PredefinedMenuItem {
    match kind {
        Pre::Services => PredefinedMenuItem::services(Some("Services")),
        Pre::Hide => PredefinedMenuItem::hide(Some("Hide WordCraft")),
        Pre::HideOthers => PredefinedMenuItem::hide_others(Some("Hide Others")),
        Pre::ShowAll => PredefinedMenuItem::show_all(Some("Show All")),
        Pre::Minimize => PredefinedMenuItem::minimize(Some("Minimize")),
        Pre::Zoom => PredefinedMenuItem::zoom(Some("Zoom")),
        Pre::BringAllToFront => PredefinedMenuItem::bring_all_to_front(Some("Bring All to Front")),
    }
}

/// Append a predefined item, logging the (unlikely) failure.
fn predefined(menu: &Submenu, item: &PredefinedMenuItem) {
    if let Err(e) = menu.append(item) {
        log::warn!("menu item: {e}");
    }
}

/// Parse `cmd+s`, `cmd+shift+s`, `f7`… into a muda accelerator; `None` when unknown.
fn parse_shortcut(spec: &str) -> Option<Accelerator> {
    let mut mods = Modifiers::empty();
    let mut key = None;
    for token in spec.split('+') {
        match token.trim().to_ascii_lowercase().as_str() {
            "cmd" | "command" | "super" | "meta" => mods |= Modifiers::META,
            "ctrl" | "control" => mods |= Modifiers::CONTROL,
            "alt" | "opt" | "option" => mods |= Modifiers::ALT,
            "shift" => mods |= Modifiers::SHIFT,
            other => key = code(other),
        }
    }
    key.map(|key| Accelerator::new(mods, key))
}

/// The [`Code`] for a key name (`a`, `1`, `f7`, `,`, `left`…).
fn code(name: &str) -> Option<Code> {
    Some(match name {
        "a" => Code::KeyA,
        "b" => Code::KeyB,
        "c" => Code::KeyC,
        "d" => Code::KeyD,
        "e" => Code::KeyE,
        "f" => Code::KeyF,
        "g" => Code::KeyG,
        "h" => Code::KeyH,
        "i" => Code::KeyI,
        "j" => Code::KeyJ,
        "k" => Code::KeyK,
        "l" => Code::KeyL,
        "m" => Code::KeyM,
        "n" => Code::KeyN,
        "o" => Code::KeyO,
        "p" => Code::KeyP,
        "q" => Code::KeyQ,
        "r" => Code::KeyR,
        "s" => Code::KeyS,
        "t" => Code::KeyT,
        "u" => Code::KeyU,
        "v" => Code::KeyV,
        "w" => Code::KeyW,
        "x" => Code::KeyX,
        "y" => Code::KeyY,
        "z" => Code::KeyZ,
        "0" => Code::Digit0,
        "1" => Code::Digit1,
        "2" => Code::Digit2,
        "3" => Code::Digit3,
        "4" => Code::Digit4,
        "5" => Code::Digit5,
        "6" => Code::Digit6,
        "7" => Code::Digit7,
        "8" => Code::Digit8,
        "9" => Code::Digit9,
        "," => Code::Comma,
        "." | "period" => Code::Period,
        "/" => Code::Slash,
        "\\" => Code::Backslash,
        ";" => Code::Semicolon,
        "'" => Code::Quote,
        "`" => Code::Backquote,
        "[" => Code::BracketLeft,
        "]" => Code::BracketRight,
        "-" => Code::Minus,
        "=" => Code::Equal,
        "enter" | "return" => Code::Enter,
        "tab" => Code::Tab,
        "space" => Code::Space,
        "backspace" => Code::Backspace,
        "delete" | "del" => Code::Delete,
        "escape" | "esc" => Code::Escape,
        "left" => Code::ArrowLeft,
        "right" => Code::ArrowRight,
        "up" => Code::ArrowUp,
        "down" => Code::ArrowDown,
        "home" => Code::Home,
        "end" => Code::End,
        "pageup" => Code::PageUp,
        "pagedown" => Code::PageDown,
        "f1" => Code::F1,
        "f2" => Code::F2,
        "f3" => Code::F3,
        "f4" => Code::F4,
        "f5" => Code::F5,
        "f6" => Code::F6,
        "f7" => Code::F7,
        "f8" => Code::F8,
        "f9" => Code::F9,
        "f10" => Code::F10,
        "f11" => Code::F11,
        "f12" => Code::F12,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_engine::Session;

    /// Every menu item runs a command the app knows, so a click never hits an unknown id (#255).
    #[test]
    fn menu_commands_are_registered() {
        let session = Session::new(wordcraft_doc::Document::new());
        fn walk(session: &Session, nodes: &[Node]) {
            for node in nodes {
                match node {
                    Node::Item { label, command, .. } if *command != "ui.dialog" => {
                        assert!(session.registry.get(command).is_some(), "menu item {label:?} runs unknown command {command:?}");
                    }
                    Node::Sub { children, .. } => walk(session, children),
                    _ => {}
                }
            }
        }
        for (_, nodes) in menu_tree() {
            walk(&session, &nodes);
        }
    }
}

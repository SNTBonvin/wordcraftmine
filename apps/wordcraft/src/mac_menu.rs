//! AppKit menu backend. Adapted from PhotoCraft's mac_menu.rs at
//! 67e4b8a499b55ef728b1da700005382c45e4677d (MIT; copyright and licence in NOTICE).
//! WordCraft keeps its ribbon. This module only installs and updates the native menu bar.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, channel};

use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSEventType};
use wordcraft_ui_egui::native_menu::{self, Backend, Chord, Event, MenuBar, Node, Role, Standard};

enum Handle {
    Plain(MenuItem),
    Check(CheckMenuItem),
}

struct Entry {
    handle: Handle,
    chord: Option<Chord>,
    label: String,
    enabled: bool,
}

struct Chosen {
    id: String,
    key: bool,
    mods: [bool; 4],
}

pub struct MacMenu {
    menu: Option<Menu>,
    entries: HashMap<String, Entry>,
    submenus: Vec<Submenu>,
    rx: Receiver<Chosen>,
    structure: u64,
}

impl MacMenu {
    pub fn new(ctx: &egui::Context) -> Self {
        let (tx, rx) = channel();
        let repaint = ctx.clone();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let event = MainThreadMarker::new().and_then(|mtm| NSApplication::sharedApplication(mtm).currentEvent());
            let key = event.as_ref().is_some_and(|ev| ev.r#type() == NSEventType::KeyDown);
            let flags = event.as_ref().map(|ev| ev.modifierFlags()).unwrap_or(NSEventModifierFlags::empty());
            let mods = [NSEventModifierFlags::Command, NSEventModifierFlags::Control, NSEventModifierFlags::Option, NSEventModifierFlags::Shift]
                .map(|m| flags.contains(m));
            let _ = tx.send(Chosen { id: e.id.0, key, mods });
            repaint.request_repaint();
        }));
        Self { menu: None, entries: HashMap::new(), submenus: Vec::new(), rx, structure: 0 }
    }

    fn build(&mut self, bar: &MenuBar) -> Result<(), muda::Error> {
        // Build first: a failed append leaves the previously installed menu intact.
        let menu = Menu::new();
        let mut entries = HashMap::new();
        let mut submenus = Vec::new();
        let mut special = Vec::new();
        for m in &bar.menus {
            let sub = Submenu::new(escape(&m.title), true);
            Self::append(&sub, &m.children, &mut entries, &mut submenus)?;
            menu.append(&sub)?;
            if matches!(m.role, Role::Window | Role::Help) {
                special.push((m.role, sub));
            }
        }
        if let Some(old) = self.menu.take() {
            old.remove_for_nsapp();
        }
        menu.init_for_nsapp();
        // muda can register these roles only after their parent is NSApp's main menu.
        for (role, sub) in special {
            match role {
                Role::Window => sub.set_as_windows_menu_for_nsapp(),
                _ => sub.set_as_help_menu_for_nsapp(),
            }
        }
        self.menu = Some(menu);
        self.entries = entries;
        self.submenus = submenus;
        self.structure = native_menu::structure_key(bar);
        Ok(())
    }

    fn append(sub: &Submenu, nodes: &[Node], entries: &mut HashMap<String, Entry>, submenus: &mut Vec<Submenu>) -> Result<(), muda::Error> {
        for node in nodes {
            match node {
                Node::Separator => sub.append(&PredefinedMenuItem::separator())?,
                Node::Submenu { label, children } => {
                    let nested = Submenu::new(escape(label), any_enabled(children));
                    submenus.push(nested.clone());
                    Self::append(&nested, children, entries, submenus)?;
                    sub.append(&nested)?;
                }
                Node::Standard(kind) => {
                    let standard = match kind {
                        Standard::Services => PredefinedMenuItem::services(Some(wordcraft_ui_egui::i18n::t("Services"))),
                        Standard::Zoom => PredefinedMenuItem::maximize(Some(wordcraft_ui_egui::i18n::t("Zoom"))),
                        Standard::BringAllToFront => PredefinedMenuItem::bring_all_to_front(Some(wordcraft_ui_egui::i18n::t("Bring All to Front"))),
                    };
                    sub.append(&standard)?;
                }
                Node::Item(it) => {
                    if it.native_text {
                        // The system Open/Save sheet has Cocoa text fields, not egui fields.
                        // Let AppKit send these selectors to its first responder. Old custom
                        // actions queued before the sheet opened are rejected by the UI model.
                        let label = escape(&it.label);
                        let native = match it.command.as_str() {
                            "edit.undo" => PredefinedMenuItem::undo(Some(&label)),
                            "edit.redo" => PredefinedMenuItem::redo(Some(&label)),
                            "edit.cut" => PredefinedMenuItem::cut(Some(&label)),
                            "edit.copy" => PredefinedMenuItem::copy(Some(&label)),
                            "edit.paste" => PredefinedMenuItem::paste(Some(&label)),
                            "select.all" => PredefinedMenuItem::select_all(Some(&label)),
                            _ => continue,
                        };
                        sub.append(&native)?;
                        continue;
                    }
                    let accel = it.shortcut.as_ref().filter(|c| c.native_ok()).and_then(accelerator);
                    let handle = match it.checked {
                        Some(checked) => {
                            let h = CheckMenuItem::with_id(it.id.clone(), escape(&it.label), it.enabled, checked, accel);
                            sub.append(&h)?;
                            Handle::Check(h)
                        }
                        None => {
                            let h = MenuItem::with_id(it.id.clone(), escape(&it.label), it.enabled, accel);
                            sub.append(&h)?;
                            Handle::Plain(h)
                        }
                    };
                    entries.insert(it.id.clone(), Entry { handle, chord: it.shortcut.clone(), label: it.label.clone(), enabled: it.enabled });
                }
            }
        }
        Ok(())
    }
}

impl Backend for MacMenu {
    fn sync(&mut self, bar: &MenuBar) {
        if self.menu.is_none() || native_menu::structure_key(bar) != self.structure {
            if let Err(error) = self.build(bar) {
                log::warn!("couldn't update the macOS menu bar: {error}");
            }
            return;
        }
        for it in bar.items() {
            let Some(e) = self.entries.get_mut(&it.id) else { continue };
            if e.label != it.label {
                match &e.handle {
                    Handle::Plain(h) => h.set_text(escape(&it.label)),
                    Handle::Check(h) => h.set_text(escape(&it.label)),
                }
                e.label.clone_from(&it.label);
            }
            if e.enabled != it.enabled {
                match &e.handle {
                    Handle::Plain(h) => h.set_enabled(it.enabled),
                    Handle::Check(h) => h.set_enabled(it.enabled),
                }
                e.enabled = it.enabled;
            }
            // AppKit toggles a check item itself; always compare with the actual native state.
            if let (Handle::Check(h), Some(checked)) = (&e.handle, it.checked)
                && h.is_checked() != checked
            {
                h.set_checked(checked);
            }
        }
        // Keep parent rows usable after a modal closes without rebuilding the menu bar.
        fn update(nodes: &[Node], submenus: &mut std::slice::Iter<'_, Submenu>) {
            for node in nodes {
                if let Node::Submenu { children, .. } = node {
                    if let Some(sub) = submenus.next() {
                        let enabled = any_enabled(children);
                        if sub.is_enabled() != enabled {
                            sub.set_enabled(enabled);
                        }
                    }
                    update(children, submenus);
                }
            }
        }
        let mut submenus = self.submenus.iter();
        for menu in &bar.menus {
            update(&menu.children, &mut submenus);
        }
    }

    fn drain(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        while let Ok(chosen) = self.rx.try_recv() {
            let Some(entry) = self.entries.get(&chosen.id) else { continue };
            let [cmd, ctrl, alt, shift] = chosen.mods;
            // Return/Space inside an open menu is an activation, not the item's key equivalent.
            // Use the modifiers really held: AppKit can match an unshifted key to a shifted row.
            let chord = entry.chord.as_ref().filter(|c| chosen.key && (cmd || ctrl || !(c.cmd || c.ctrl))).map(|c| Chord {
                cmd,
                ctrl,
                alt,
                shift,
                key: c.key.clone(),
            });
            out.push(match chord {
                Some(c) => Event::Key(c),
                None => Event::Click(chosen.id),
            });
        }
        out
    }

    fn system_action(&mut self, id: &str) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let app = NSApplication::sharedApplication(mtm);
        match id {
            native_menu::HIDE => app.hide(None),
            native_menu::HIDE_OTHERS => app.hideOtherApplications(None),
            native_menu::SHOW_ALL => app.unhideAllApplications(None),
            _ => {}
        }
    }
}

pub fn install(ctx: &egui::Context, app: &wordcraft_ui_egui::WordApp) -> Option<native_menu::NativeMenu> {
    wordcraft_ui_egui::i18n::set_current(wordcraft_ui_egui::i18n::Lang::from_pref(&app.ui.language));
    let bar = native_menu::menu_bar(app, ctx);
    // Third-party/AppKit initialization must not cost the user their document.
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut backend = MacMenu::new(ctx);
        backend.build(&bar)?;
        Ok::<_, muda::Error>(native_menu::NativeMenu::new(Box::new(backend), ctx.clone()))
    })) {
        Ok(Ok(menu)) => Some(menu),
        Ok(Err(error)) => {
            log::warn!("couldn't install the macOS menu bar: {error}");
            None
        }
        Err(_) => {
            log::warn!("couldn't install the macOS menu bar");
            None
        }
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&&")
}

fn any_enabled(nodes: &[Node]) -> bool {
    nodes.iter().any(|node| match node {
        Node::Item(it) => it.enabled,
        Node::Submenu { children, .. } => any_enabled(children),
        Node::Standard(_) => true,
        Node::Separator => false,
    })
}

/// Command and Control stay distinct. The remaining named keys are mapped explicitly rather
/// than treating Delete (forward delete) as Backspace.
fn accelerator(c: &Chord) -> Option<Accelerator> {
    let mut mods = Modifiers::empty();
    for (on, modifier) in [(c.cmd, Modifiers::META), (c.ctrl, Modifiers::CONTROL), (c.alt, Modifiers::ALT), (c.shift, Modifiers::SHIFT)] {
        if on {
            mods |= modifier;
        }
    }
    let name = match c.key.as_str() {
        "[" => "BracketLeft",
        "]" => "BracketRight",
        "\\" => "Backslash",
        "/" => "Slash",
        "=" | "+" => "Equal",
        "-" => "Minus",
        "'" => "Quote",
        ";" => "Semicolon",
        "," => "Comma",
        "." => "Period",
        "`" => "Backquote",
        "Left" => "ArrowLeft",
        "Right" => "ArrowRight",
        "Up" => "ArrowUp",
        "Down" => "ArrowDown",
        other => other,
    };
    let name = if name.len() == 1 && name.chars().all(|c| c.is_ascii_alphabetic()) {
        format!("Key{}", name.to_ascii_uppercase())
    } else if name.len() == 1 && name.chars().all(|c| c.is_ascii_digit()) {
        format!("Digit{name}")
    } else {
        name.to_string()
    };
    if c.key == "+" {
        mods |= Modifiers::SHIFT;
    }
    Some(Accelerator::new(mods, name.parse::<Code>().ok()?))
}

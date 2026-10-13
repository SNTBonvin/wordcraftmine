//! `WordcraftEmbed`: the editor on a canvas inside another web app (Memento), driven from
//! JavaScript. The contract is Memento's `docs/wordcraft-embed.md`:
//!
//! ```ts
//! class WordcraftEmbed {
//!   static start(canvas: HTMLCanvasElement, options: { theme: 'light' | 'dark'; readOnly?: boolean }): Promise<WordcraftEmbed>
//!   open(name: string, bytes: Uint8Array): ImportReport
//!   save(): Uint8Array
//!   isDirty(): boolean
//!   text(): string
//!   selection(): [start: number, end: number] | null
//!   select(start: number, end: number): void
//!   setDecorations(ranges: { start: number; end: number; color: string }[]): void
//!   setTheme(theme: 'light' | 'dark'): void
//!   setReadOnly(on: boolean): void
//!   on(event: 'change' | 'selection' | 'save-shortcut', callback: () => void): void
//!   destroy(): void
//! }
//! ```
//!
//! Offsets are UTF-16 units into `text()` ([`wordcraft_engine::TextIndex`]). The app is shared
//! between this handle and the eframe shell through a `RefCell`: JavaScript runs between frames,
//! so a call never finds it borrowed, and if one ever did it gets an error rather than a panic.
//! Events are sent after the frame they happened in (in a microtask), so a callback may call
//! straight back in.

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use wordcraft_engine::Session;
use wordcraft_ui_egui::embed::{Decoration, EmbedEvent, MAX_DECORATIONS, parse_color};
use wordcraft_ui_egui::{Services, WordApp, theme};

/// What the handle and the shell share. `app` is `None` once destroyed.
struct Shared {
    app: RefCell<Option<WordApp>>,
    ctx: RefCell<Option<egui::Context>>,
    listeners: RefCell<Vec<(EmbedEvent, js_sys::Function)>>,
}

impl Shared {
    /// Run `f` on the app, then draw a frame to show what it did.
    fn with<T>(&self, f: impl FnOnce(&mut WordApp) -> Result<T, String>) -> Result<T, JsError> {
        let mut app = self.app.try_borrow_mut().map_err(|_| JsError::new("WordCraft is busy drawing; call again from outside its callbacks"))?;
        let app = app.as_mut().ok_or_else(|| JsError::new("this WordcraftEmbed has been destroyed"))?;
        let r = f(app).map_err(|e| JsError::new(&e));
        if let Ok(ctx) = self.ctx.try_borrow()
            && let Some(ctx) = ctx.as_ref()
        {
            ctx.request_repaint();
        }
        r
    }

    /// Tell the listeners, after the current frame.
    fn send_later(self: &Rc<Self>, events: Vec<EmbedEvent>) {
        let shared = self.clone();
        wasm_bindgen_futures::spawn_local(async move {
            for event in events {
                // A callback may add or drop listeners: call a copy of the list.
                let callbacks: Vec<js_sys::Function> = shared
                    .listeners
                    .try_borrow()
                    .map(|l| l.iter().filter(|(e, _)| *e == event).map(|(_, f)| f.clone()).collect())
                    .unwrap_or_default();
                for f in callbacks {
                    if let Err(e) = f.call0(&JsValue::NULL) {
                        log::warn!("WordcraftEmbed `{}` listener threw: {e:?}", event.name());
                    }
                }
            }
        });
    }
}

/// The eframe app: the shared [`WordApp`], and the events it has to send.
struct EmbedShell {
    shared: Rc<Shared>,
}

impl eframe::App for EmbedShell {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        match self.shared.app.try_borrow_mut() {
            Ok(mut app) => {
                if let Some(app) = app.as_mut() {
                    app.logic(ctx);
                }
            }
            // Never while JavaScript runs (it runs between frames); try the next frame.
            Err(_) => ctx.request_repaint(),
        }
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        if let Ok(mut app) = self.shared.app.try_borrow_mut()
            && let Some(app) = app.as_mut()
        {
            app.raw_input_hook(raw);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let events = match self.shared.app.try_borrow_mut() {
            Ok(mut app) => match app.as_mut() {
                Some(app) => {
                    app.ui(ui);
                    app.take_embed_events()
                }
                None => return,
            },
            Err(_) => {
                ui.ctx().request_repaint();
                return;
            }
        };
        if !events.is_empty() {
            self.shared.send_later(events);
        }
    }
}

fn appearance(theme: &str) -> Result<theme::Appearance, String> {
    match theme {
        "light" => Ok(theme::Appearance::Light),
        "dark" => Ok(theme::Appearance::Dark),
        other => Err(format!("unknown theme `{other}` (light or dark)")),
    }
}

/// A UTF-16 offset from JavaScript: a number, clamped (negative, NaN: 0).
fn offset(x: f64) -> usize {
    // `as` saturates: never a panic, whatever the number.
    if x.is_finite() && x > 0.0 { x as usize } else { 0 }
}

fn get(obj: &JsValue, key: &str) -> JsValue {
    js_sys::Reflect::get(obj, &JsValue::from_str(key)).unwrap_or(JsValue::UNDEFINED)
}

fn set(obj: &JsValue, key: &str, value: &JsValue) {
    let _ = js_sys::Reflect::set(obj, &JsValue::from_str(key), value);
}

#[wasm_bindgen(typescript_custom_section)]
const TS_TYPES: &str = r#"
/** What `open()` found: whether saving would lose something the file had, and what. */
export type ImportReport = { lossy: boolean; dropped: { what: string; count: number }[] };
/** A tinted range of `text()`, UTF-16 offsets; `color` is `#rrggbb` or `#rrggbbaa`. */
export type Decoration = { start: number; end: number; color: string };
export type EmbedOptions = { theme: 'light' | 'dark'; readOnly?: boolean };
export type EmbedEvent = 'change' | 'selection' | 'save-shortcut';
"#;

/// The editor on a canvas inside another web app (see the module docs).
#[wasm_bindgen]
pub struct WordcraftEmbed {
    runner: eframe::WebRunner,
    shared: Rc<Shared>,
}

#[wasm_bindgen]
impl WordcraftEmbed {
    /// Start the editor on `canvas`, which the host created and sized. `options`: `theme`
    /// (`"light"` or `"dark"`) and `readOnly`.
    pub async fn start(
        canvas: web_sys::HtmlCanvasElement,
        #[wasm_bindgen(unchecked_param_type = "EmbedOptions")] options: JsValue,
    ) -> Result<WordcraftEmbed, JsError> {
        eframe::WebLogger::init(log::LevelFilter::Info).ok();
        let theme = appearance(&get(&options, "theme").as_string().unwrap_or_else(|| "light".into())).map_err(|e| JsError::new(&e))?;
        let read_only = get(&options, "readOnly").as_bool().unwrap_or(false);
        let region = wordcraft_ui_egui::i18n::system_region();
        let mut session = Session::new(wordcraft_engine::blank_document(&region));
        session.system_region = region;
        session.read_only = read_only;
        // No file pickers, downloads, stored fonts or page-wide paste listener: the host has the
        // files and the page. Print opens the browser's dialog on the PDF.
        let services = Services { print: Some(Box::new(crate::web::print_pdf)), ..Default::default() };
        let mut app = WordApp::new(session, services);
        app.embedded();
        app.ui.theme = theme;
        let shared = Rc::new(Shared { app: RefCell::new(Some(app)), ctx: RefCell::new(None), listeners: RefCell::new(Vec::new()) });
        let runner = eframe::WebRunner::new();
        let in_shell = shared.clone();
        runner
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(move |cc| {
                    if let Ok(mut ctx) = in_shell.ctx.try_borrow_mut() {
                        *ctx = Some(cc.egui_ctx.clone());
                    }
                    Ok(Box::new(EmbedShell { shared: in_shell }))
                }),
            )
            .await
            .map_err(|e| JsError::new(&format!("WordCraft failed to start (a browser with WebGPU or WebGL2 is required): {e:?}")))?;
        Ok(WordcraftEmbed { runner, shared })
    }

    /// Open a document from its bytes, replacing any open one without asking; `name` gives the
    /// format (`.docx`, `.doc`, `.odt`, `.rtf`…). Returns the import report `{ lossy, dropped }`.
    #[wasm_bindgen(unchecked_return_type = "ImportReport")]
    pub fn open(&self, name: &str, bytes: &[u8]) -> Result<JsValue, JsError> {
        self.shared.with(|app| app.embed_open(name, bytes))?;
        // Until the readers count what they drop (PR B of the embedding plan), no file can be
        // promised to survive a save: every file reports a loss, so the host opens it read-only.
        let report = js_sys::Object::new();
        set(&report, "lossy", &JsValue::TRUE);
        let dropped = js_sys::Array::new();
        let unknown = js_sys::Object::new();
        set(&unknown, "what", &JsValue::from_str("report/not yet available"));
        set(&unknown, "count", &JsValue::from_f64(1.0));
        dropped.push(&unknown);
        set(&report, "dropped", &dropped);
        Ok(report.into())
    }

    /// The open document as `.docx` bytes. Clears the dirty flag.
    pub fn save(&self) -> Result<Vec<u8>, JsError> {
        self.shared.with(|app| app.embed_save())
    }

    #[wasm_bindgen(js_name = isDirty)]
    pub fn is_dirty(&self) -> Result<bool, JsError> {
        self.shared.with(|app| Ok(app.session.dirty))
    }

    /// The body's text: paragraphs separated by `\n`, table cells by `\t`; fields as shown;
    /// tracked deletions left out.
    pub fn text(&self) -> Result<String, JsError> {
        self.shared.with(|app| Ok(app.text_index().text().to_string()))
    }

    /// The selection as `[start, end]` UTF-16 offsets into `text()`, or `null` when nothing is
    /// selected.
    #[wasm_bindgen(unchecked_return_type = "[start: number, end: number] | null")]
    pub fn selection(&self) -> Result<JsValue, JsError> {
        let sel = self.shared.with(|app| Ok(app.embed_selection()))?;
        Ok(match sel {
            Some((a, b)) => js_sys::Array::of2(&JsValue::from_f64(a as f64), &JsValue::from_f64(b as f64)).into(),
            None => JsValue::NULL,
        })
    }

    /// Select `text()[start, end)` and scroll it into view.
    pub fn select(&self, start: f64, end: f64) -> Result<(), JsError> {
        self.shared.with(|app| app.embed_select(offset(start), offset(end)).map(|_| ()))
    }

    /// Ranges of `text()` to tint: `[{ start, end, color }]`, colours as `#rrggbb` or
    /// `#rrggbbaa`. Drawn over the page, never part of the document; kept in place as it is
    /// edited. Replaces the previous set.
    #[wasm_bindgen(js_name = setDecorations)]
    pub fn set_decorations(&self, #[wasm_bindgen(unchecked_param_type = "Decoration[]")] ranges: JsValue) -> Result<(), JsError> {
        if !js_sys::Array::is_array(&ranges) {
            return Err(JsError::new("setDecorations takes an array of { start, end, color }"));
        }
        let ranges = js_sys::Array::from(&ranges);
        let mut out = Vec::new();
        for r in ranges.iter().take(MAX_DECORATIONS) {
            let color = get(&r, "color").as_string().unwrap_or_default();
            let color = parse_color(&color).ok_or_else(|| JsError::new(&format!("bad decoration colour `{color}` (use #rrggbb or #rrggbbaa)")))?;
            let num = |k: &str| offset(get(&r, k).as_f64().unwrap_or(0.0));
            out.push(Decoration { start: num("start"), end: num("end"), color });
        }
        self.shared.with(|app| {
            app.set_decorations(out);
            Ok(())
        })
    }

    #[wasm_bindgen(js_name = setTheme)]
    pub fn set_theme(&self, #[wasm_bindgen(unchecked_param_type = "'light' | 'dark'")] theme: &str) -> Result<(), JsError> {
        let theme = appearance(theme).map_err(|e| JsError::new(&e))?;
        self.shared.with(|app| {
            app.ui.theme = theme;
            Ok(())
        })
    }

    /// Refuse every command that would change the document (selecting and saving still work).
    #[wasm_bindgen(js_name = setReadOnly)]
    pub fn set_read_only(&self, on: bool) -> Result<(), JsError> {
        self.shared.with(|app| {
            app.session.read_only = on;
            Ok(())
        })
    }

    /// `change`: the document changed. `selection`: the selection changed. `save-shortcut`:
    /// Save was asked for (⌘S, Ctrl+S): the host saves.
    pub fn on(
        &self,
        #[wasm_bindgen(unchecked_param_type = "EmbedEvent")] event: &str,
        #[wasm_bindgen(unchecked_param_type = "() => void")] callback: js_sys::Function,
    ) -> Result<(), JsError> {
        let event = match event {
            "change" => EmbedEvent::Change,
            "selection" => EmbedEvent::Selection,
            "save-shortcut" => EmbedEvent::SaveShortcut,
            other => return Err(JsError::new(&format!("unknown event `{other}` (change, selection or save-shortcut)"))),
        };
        self.shared.listeners.try_borrow_mut().map_err(|_| JsError::new("WordCraft is busy; call again"))?.push((event, callback));
        Ok(())
    }

    /// Stop the editor and free its memory. Every call after this is an error.
    pub fn destroy(&self) -> Result<(), JsError> {
        self.runner.destroy();
        let app = self.shared.app.try_borrow_mut().map_err(|_| JsError::new("WordCraft is busy; call again"))?.take();
        drop(app);
        if let Ok(mut l) = self.shared.listeners.try_borrow_mut() {
            l.clear();
        }
        if let Ok(mut ctx) = self.shared.ctx.try_borrow_mut() {
            *ctx = None;
        }
        Ok(())
    }
}

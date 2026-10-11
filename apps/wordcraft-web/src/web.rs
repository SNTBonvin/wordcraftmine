//! The browser shell: web `Services`, drag-and-drop, the unsaved-changes guard and the eframe
//! web runner.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wordcraft_engine::Session;
use wordcraft_ui_egui::{Inbox, Services, WordApp};

type LocalFontMap = Arc<Mutex<HashMap<String, Vec<JsValue>>>>;

const DOC_EXTS: &[&str] = &["docx", "docm", "dotx", "dotm", "doc", "dot", "odt", "rtf", "txt", "md", "html", "htm", "tex", "json"];
const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp"];
const RECIPIENT_EXTS: &[&str] = &["csv", "tsv", "txt"];
const FONT_EXTS: &[&str] = &["ttf", "otf", "ttc", "woff2", "woff"];
const CANVAS_ID: &str = "wordcraft_canvas";
const LOADING_ID: &str = "wordcraft_loading";

pub fn start() {
    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    wasm_bindgen_futures::spawn_local(async {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            log::error!("no document");
            return;
        };
        let Some(canvas) = document.get_element_by_id(CANVAS_ID).and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok()) else {
            log::error!("missing <canvas id=\"{CANVAS_ID}\">");
            return;
        };
        let dirty = Rc::new(Cell::new(false));
        if let Err(e) = guard_unload(dirty.clone()) {
            log::error!("no unsaved-changes guard: {e}");
        }
        let mut options = eframe::WebOptions::default();
        if query().contains("webgl")
            && let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup
        {
            create.instance_descriptor.backends = eframe::wgpu::Backends::GL;
        }
        let result = eframe::WebRunner::new()
            .start(
                canvas,
                options,
                Box::new(move |cc| {
                    if let Some(rs) = &cc.wgpu_render_state {
                        log::info!("wordcraft-web: wgpu backend {:?}", rs.adapter.get_info().backend);
                    }
                    let inbox: Inbox = Inbox::default();
                    if let Err(e) = listen_for_pasted_pictures(inbox.clone(), cc.egui_ctx.clone()) {
                        log::error!("pasting pictures is unavailable: {e}");
                    }
                    let doc = if query().contains("sample") { wordcraft_engine::sample::sample_document() } else { wordcraft_doc::Document::new() };
                    let mut app = WordApp::new(Session::new(doc), services(inbox.clone(), cc.egui_ctx.clone(), dirty));
                    app.autosave = false;
                    Ok(Box::new(WebShell { app, inbox }))
                }),
            )
            .await;
        if let Some(el) = document.get_element_by_id(LOADING_ID) {
            match result {
                Ok(()) => el.remove(),
                Err(e) => el.set_inner_html(&format!("<p>WordCraft failed to start: {e:?}</p><p>A browser with WebGPU or WebGL2 is required.</p>")),
            }
        }
    });
}

fn query() -> String {
    web_sys::window().and_then(|w| w.location().search().ok()).unwrap_or_default()
}

/// Wraps the app to read dropped files asynchronously (browsers can't read them synchronously)
/// and feed them through the inbox.
struct WebShell {
    app: WordApp,
    inbox: Inbox,
}

impl eframe::App for WebShell {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let dropped = ctx.input_mut(|i| std::mem::take(&mut i.raw.dropped_files));
        for f in dropped {
            let inbox = self.inbox.clone();
            let ctx = ctx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let name = f.path().file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "dropped".into());
                match f.bytes_async().await {
                    Ok(bytes) => {
                        inbox.lock().unwrap_or_else(|e| e.into_inner()).push((name, bytes));
                        ctx.request_repaint();
                    }
                    Err(e) => log::error!("couldn't read dropped file {name}: {e}"),
                }
            });
        }
        self.app.logic(ctx);
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.app.raw_input_hook(raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
    }
}

/// Largest picture pasted from the browser's clipboard (as `insert.picture` allows).
const MAX_PASTED_BYTES: f64 = (200u32 << 20) as f64;
/// Most pictures one paste inserts.
const MAX_PASTED_PICTURES: u32 = 20;

/// Pasting a picture (#45): eframe's paste handler only reads text, so a clipboard holding just
/// pictures (a screenshot, a browser's Copy Image) is read here and the pictures arrive through
/// the inbox like dropped ones. A clipboard with text is left to eframe.
fn listen_for_pasted_pictures(inbox: Inbox, ctx: egui::Context) -> Result<(), String> {
    let document = web_sys::window().and_then(|w| w.document()).ok_or("no document")?;
    let on_paste = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::ClipboardEvent)>::new(move |e: web_sys::ClipboardEvent| {
        let Some(data) = e.clipboard_data() else { return };
        if !data.get_data("text").unwrap_or_default().is_empty() {
            return;
        }
        let Some(files) = data.files() else { return };
        for i in 0..files.length().min(MAX_PASTED_PICTURES) {
            let Some(file) = files.get(i) else { continue };
            if !file.type_().starts_with("image/") {
                continue;
            }
            if file.size() > MAX_PASTED_BYTES {
                log::warn!("pasted picture is larger than 200 MB; not inserted");
                continue;
            }
            let inbox = inbox.clone();
            let ctx = ctx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                match wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await {
                    Ok(buf) => {
                        let bytes = js_sys::Uint8Array::new(&buf).to_vec();
                        // The inbox inserts files named like pictures; the engine reads the format from the bytes.
                        inbox.lock().unwrap_or_else(|e| e.into_inner()).push(("pasted.png".into(), bytes));
                        ctx.request_repaint();
                    }
                    Err(e) => log::error!("couldn't read the pasted picture: {e:?}"),
                }
            });
        }
    });
    document.add_event_listener_with_callback("paste", on_paste.as_ref().unchecked_ref()).map_err(|e| format!("{e:?}"))?;
    // The listener lives as long as the page.
    on_paste.forget();
    Ok(())
}

/// `dirty` is the flag the `beforeunload` guard reads ([`guard_unload`]).
fn services(inbox: Inbox, ctx: egui::Context, dirty: Rc<Cell<bool>>) -> Services {
    let open_inbox = inbox.clone();
    let upload_inbox = inbox.clone();
    let font_map: LocalFontMap = Arc::new(Mutex::new(HashMap::new()));
    let loaded = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let in_flight = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let (load_system_fonts, _) = setup_system_fonts(font_map.clone(), ctx.clone(), loaded, in_flight);

    let req_font_map = font_map.clone();
    let req_ctx = ctx.clone();
    let upload_ctx = ctx.clone();

    let loader_font_map = font_map;
    let loader_ctx = ctx.clone();
    wordcraft_fonts::FontDb::global().set_loader(move |fam| {
        request_font_family(fam, loader_font_map.clone(), loader_ctx.clone());
    });

    Services {
        open_async: Some(Box::new(move |purpose: &str| {
            let inbox = open_inbox.clone();
            let ctx = ctx.clone();
            let dialog = if purpose == "picture" {
                rfd::AsyncFileDialog::new().add_filter("Pictures", IMAGE_EXTS)
            } else if purpose == "recipients" {
                rfd::AsyncFileDialog::new().add_filter("Recipient lists", RECIPIENT_EXTS)
            } else if purpose == "font" {
                rfd::AsyncFileDialog::new().add_filter("Fonts", FONT_EXTS)
            } else {
                rfd::AsyncFileDialog::new().add_filter("Documents", DOC_EXTS)
            };
            wasm_bindgen_futures::spawn_local(async move {
                let Some(file) = dialog.pick_file().await else {
                    return;
                };
                let bytes = file.read().await;
                inbox.lock().unwrap_or_else(|e| e.into_inner()).push((file.file_name(), bytes));
                ctx.request_repaint();
            });
        })),
        // Exports ask for a name; the browser decides where the download goes.
        pick_save: Some(Box::new(|name: &str| Some(name.to_string()))),
        download: Some(Box::new(download)),
        print: Some(Box::new(print_pdf)),
        inbox: Some(inbox),
        on_dirty: Some(Box::new(move |d| dirty.set(d))),
        load_system_fonts,
        upload_font: Some(Box::new(move || {
            let inbox = upload_inbox.clone();
            let ctx = upload_ctx.clone();
            let dialog =
                rfd::AsyncFileDialog::new().set_title("Upload Custom Fonts").add_filter("Fonts (*.ttf, *.otf, *.ttc, *.woff2, *.woff)", FONT_EXTS);
            wasm_bindgen_futures::spawn_local(async move {
                let Some(files) = dialog.pick_files().await else {
                    return;
                };
                let mut loaded = Vec::new();
                for file in files {
                    let name = file.file_name();
                    let bytes = file.read().await;
                    loaded.push((name, bytes));
                }
                inbox.lock().unwrap_or_else(|e| e.into_inner()).extend(loaded);
                ctx.request_repaint();
            });
        })),
        request_font: Some(Box::new(move |family: &str| {
            request_font_family(family, req_font_map.clone(), req_ctx.clone());
        })),
        ..Default::default()
    }
}

async fn fetch_font_blob(item: &JsValue) -> Result<Vec<u8>, String> {
    let blob_fn: js_sys::Function = js_sys::Reflect::get(item, &JsValue::from_str("blob"))
        .map_err(|e| format!("{e:?}"))?
        .dyn_into()
        .map_err(|_| "blob is not a function".to_string())?;
    let blob_prom: js_sys::Promise =
        blob_fn.call0(item).map_err(|e| format!("{e:?}"))?.dyn_into().map_err(|_| "blob did not return a promise".to_string())?;
    let blob_val = wasm_bindgen_futures::JsFuture::from(blob_prom).await.map_err(|e| format!("{e:?}"))?;
    let blob: web_sys::Blob = blob_val.dyn_into().map_err(|_| "not a Blob".to_string())?;
    let buf_prom = blob.array_buffer();
    let buf_val = wasm_bindgen_futures::JsFuture::from(buf_prom).await.map_err(|e| format!("{e:?}"))?;
    let u8_arr = js_sys::Uint8Array::new(&buf_val);
    Ok(u8_arr.to_vec())
}

fn request_font_family(family: &str, font_map: LocalFontMap, ctx: egui::Context) {
    let clean = family.trim().trim_matches('\'').trim_matches('"').trim();
    if clean.is_empty() || wordcraft_fonts::FontDb::global().is_loaded(clean) {
        return;
    }
    let fam_lower = clean.to_lowercase();
    let items: Vec<JsValue> = {
        let lock = font_map.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(v) = lock.get(&fam_lower) {
            v.clone()
        } else {
            lock.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(&fam_lower) || fam_lower.starts_with(k.as_str()) || k.starts_with(fam_lower.as_str()))
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        }
    };
    if items.is_empty() {
        return;
    }
    let fam_name = clean.to_string();
    wasm_bindgen_futures::spawn_local(async move {
        let mut added = 0;
        for item in items {
            match fetch_font_blob(&item).await {
                Ok(bytes) => {
                    added += wordcraft_fonts::FontDb::global().add_font(bytes);
                }
                Err(e) => {
                    log::warn!("failed to fetch local font blob for {fam_name}: {e}");
                }
            }
        }
        if added > 0 {
            log::info!("Loaded {added} face(s) for family {fam_name}");
            ctx.request_repaint();
        }
    });
}

fn catalog_local_fonts(arr: js_sys::Array, font_map: LocalFontMap, ctx: egui::Context) {
    let mut entries: Vec<(String, String, String)> = Vec::with_capacity(arr.length() as usize);
    let mut map_guard = font_map.lock().unwrap_or_else(|e| e.into_inner());

    for i in 0..arr.length() {
        let item = arr.get(i);
        let family = js_sys::Reflect::get(&item, &JsValue::from_str("family")).ok().and_then(|v| v.as_string()).unwrap_or_default();
        let style = js_sys::Reflect::get(&item, &JsValue::from_str("style")).ok().and_then(|v| v.as_string()).unwrap_or_else(|| "Regular".into());

        if !family.is_empty() {
            let legacy = wordcraft_fonts::legacy_family(&family, &style);
            entries.push((family.clone(), style, legacy.clone()));
            map_guard.entry(family.to_lowercase()).or_default().push(item.clone());
            if !legacy.eq_ignore_ascii_case(&family) {
                map_guard.entry(legacy.to_lowercase()).or_default().push(item);
            }
        }
    }
    drop(map_guard);

    let count = wordcraft_fonts::FontDb::global().register_font_entries(entries);
    log::info!("Cataloged {count} local system fonts");
    ctx.request_repaint();
}

fn trigger_query_local_fonts(
    window: &web_sys::Window,
    query_fn: &js_sys::Function,
    font_map: LocalFontMap,
    ctx: egui::Context,
    loaded: Arc<std::sync::atomic::AtomicBool>,
    in_flight: Arc<std::sync::atomic::AtomicBool>,
) {
    if loaded.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    if in_flight.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    match query_fn.call0(window) {
        Ok(prom_val) => {
            if let Ok(promise) = prom_val.dyn_into::<js_sys::Promise>() {
                let loaded = loaded.clone();
                let in_flight = in_flight.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    match wasm_bindgen_futures::JsFuture::from(promise).await {
                        Ok(result) => {
                            if let Ok(arr) = result.dyn_into::<js_sys::Array>() {
                                log::info!("queryLocalFonts resolved with {} fonts", arr.length());
                                catalog_local_fonts(arr, font_map, ctx);
                                loaded.store(true, std::sync::atomic::Ordering::Relaxed);
                            }
                            in_flight.store(false, std::sync::atomic::Ordering::Relaxed);
                        }
                        Err(e) => {
                            log::info!("queryLocalFonts not granted or deferred: {e:?}");
                            in_flight.store(false, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                });
            } else {
                in_flight.store(false, std::sync::atomic::Ordering::Relaxed);
            }
        }
        Err(e) => {
            log::info!("queryLocalFonts synchronous call error: {e:?}");
            in_flight.store(false, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

fn setup_system_fonts(
    font_map: LocalFontMap,
    ctx: egui::Context,
    loaded: Arc<std::sync::atomic::AtomicBool>,
    in_flight: Arc<std::sync::atomic::AtomicBool>,
) -> (Option<Box<dyn Fn()>>, Option<js_sys::Function>) {
    let Some(window) = web_sys::window() else { return (None, None) };
    let query_prop = match js_sys::Reflect::get(&window, &JsValue::from_str("queryLocalFonts")) {
        Ok(p) => p,
        Err(_) => return (None, None),
    };
    if !query_prop.is_function() {
        log::info!("queryLocalFonts is not available in this browser");
        return (None, None);
    }
    let query_fn: js_sys::Function = match query_prop.dyn_into() {
        Ok(f) => f,
        Err(_) => return (None, None),
    };

    // Attach capture listeners so that on user interaction or button clicks,
    // queryLocalFonts() runs with valid transient user activation:
    let window_clone = window.clone();
    let query_fn_clone = query_fn.clone();
    let font_map_clone = font_map.clone();
    let ctx_clone = ctx.clone();
    let loaded_clone = loaded.clone();
    let in_flight_clone = in_flight.clone();

    let on_gesture = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(move |_e: web_sys::Event| {
        trigger_query_local_fonts(
            &window_clone,
            &query_fn_clone,
            font_map_clone.clone(),
            ctx_clone.clone(),
            loaded_clone.clone(),
            in_flight_clone.clone(),
        );
    });

    let callback_fn = on_gesture.as_ref().unchecked_ref::<js_sys::Function>();
    let event_names = ["click", "pointerup", "mouseup", "keydown", "pointerdown"];

    for evt in &event_names {
        let _ = window.add_event_listener_with_callback_and_bool(evt, callback_fn, true);
    }

    if let Some(doc) = window.document() {
        for evt in &event_names {
            let _ = doc.add_event_listener_with_callback_and_bool(evt, callback_fn, true);
        }
        if let Some(canvas) = doc.get_element_by_id(CANVAS_ID) {
            for evt in &event_names {
                let _ = canvas.add_event_listener_with_callback_and_bool(evt, callback_fn, true);
            }
        }
    }

    on_gesture.forget();

    let action_window = window.clone();
    let action_query_fn = query_fn.clone();
    let action_font_map = font_map;
    let action_ctx = ctx;
    let action_loaded = loaded;
    let action_in_flight = in_flight;

    let load_action: Box<dyn Fn()> = Box::new(move || {
        trigger_query_local_fonts(
            &action_window,
            &action_query_fn,
            action_font_map.clone(),
            action_ctx.clone(),
            action_loaded.clone(),
            action_in_flight.clone(),
        );
    });

    (Some(load_action), Some(query_fn))
}

/// Closing, reloading or leaving the tab with unsaved changes asks first. Browsers show their own
/// confirmation (a page can only ask for it), so there is no Save button: Cancel, then File › Save.
fn guard_unload(dirty: Rc<Cell<bool>>) -> Result<(), String> {
    let window = web_sys::window().ok_or("no window")?;
    let on_unload = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::BeforeUnloadEvent)>::new(move |e: web_sys::BeforeUnloadEvent| {
        if dirty.get() {
            e.prevent_default();
            // Older browsers ask only when `returnValue` is set.
            e.set_return_value("unsaved changes");
        }
    });
    window.add_event_listener_with_callback("beforeunload", on_unload.as_ref().unchecked_ref()).map_err(|e| format!("{e:?}"))?;
    // The listener lives as long as the page.
    on_unload.forget();
    Ok(())
}

/// Trigger a browser save dialog or download of `bytes` named after the last component of `path`.
fn download(path: &str, bytes: &[u8]) -> Result<(), String> {
    let js = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "document".into());
    let window = web_sys::window().ok_or("no window")?;

    let show_picker_fn: Option<js_sys::Function> =
        js_sys::Reflect::get(&window, &JsValue::from_str("showSaveFilePicker")).ok().and_then(|v| v.dyn_into().ok());

    if let Some(picker_fn) = show_picker_fn {
        let ext = std::path::Path::new(&name).extension().and_then(|e| e.to_str()).unwrap_or("docx").to_ascii_lowercase();
        let dot_ext = format!(".{ext}");
        let mime = mime_for(&name);

        let accept_obj = js_sys::Object::new();
        let exts_arr = js_sys::Array::new();
        exts_arr.push(&JsValue::from_str(&dot_ext));
        let _ = js_sys::Reflect::set(&accept_obj, &JsValue::from_str(mime), &exts_arr);

        let type_desc = match ext.as_str() {
            "docx" => "Word Document (*.docx)",
            "docm" => "Word Macro-Enabled Document (*.docm)",
            "dotx" => "Word Template (*.dotx)",
            "dotm" => "Word Macro-Enabled Template (*.dotm)",
            "odt" => "OpenDocument Text (*.odt)",
            "rtf" => "Rich Text Format (*.rtf)",
            "txt" => "Plain Text (*.txt)",
            "pdf" => "PDF Document (*.pdf)",
            "png" => "PNG Image (*.png)",
            "html" | "htm" => "HTML Document (*.html)",
            "json" => "JSON Document (*.json)",
            _ => "Document",
        };

        let type_obj = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&type_obj, &JsValue::from_str("description"), &JsValue::from_str(type_desc));
        let _ = js_sys::Reflect::set(&type_obj, &JsValue::from_str("accept"), &accept_obj);

        let types_arr = js_sys::Array::new();
        types_arr.push(&type_obj);

        let picker_opts = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&picker_opts, &JsValue::from_str("suggestedName"), &JsValue::from_str(&name));
        let _ = js_sys::Reflect::set(&picker_opts, &JsValue::from_str("types"), &types_arr);

        // Call showSaveFilePicker synchronously while transient user activation is active:
        let prom_val = picker_fn.call1(&window, &picker_opts).map_err(js)?;
        let prom: js_sys::Promise = prom_val.dyn_into().map_err(|_| "not a promise".to_string())?;

        let bytes_vec = bytes.to_vec();
        wasm_bindgen_futures::spawn_local(async move {
            match wasm_bindgen_futures::JsFuture::from(prom).await {
                Ok(handle_val) => {
                    if let Err(e) = write_file_handle(&handle_val, &bytes_vec).await {
                        log::error!("failed writing to file handle: {e}");
                    } else {
                        log::info!("Successfully saved file through save file picker");
                    }
                }
                Err(e) => {
                    let err_name = js_sys::Reflect::get(&e, &JsValue::from_str("name")).ok().and_then(|v| v.as_string()).unwrap_or_default();
                    if err_name == "AbortError" {
                        log::info!("Save file dialog cancelled by user");
                    } else {
                        log::error!("showSaveFilePicker error: {e:?}");
                    }
                }
            }
        });
        return Ok(());
    }

    // Fallback for browsers without showSaveFilePicker (e.g. Firefox):
    trigger_anchor_download(&window, &name, bytes)
}

async fn write_file_handle(handle: &JsValue, bytes: &[u8]) -> Result<(), String> {
    let js = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let create_fn: js_sys::Function = js_sys::Reflect::get(handle, &JsValue::from_str("createWritable"))
        .map_err(js)?
        .dyn_into()
        .map_err(|_| "createWritable is not a function".to_string())?;
    let stream_prom: js_sys::Promise = create_fn.call0(handle).map_err(js)?.dyn_into().map_err(|_| "not a promise".to_string())?;
    let stream = wasm_bindgen_futures::JsFuture::from(stream_prom).await.map_err(js)?;

    let write_fn: js_sys::Function =
        js_sys::Reflect::get(&stream, &JsValue::from_str("write")).map_err(js)?.dyn_into().map_err(|_| "write is not a function".to_string())?;
    let uint8 = js_sys::Uint8Array::from(bytes);
    let write_prom: js_sys::Promise = write_fn.call1(&stream, &uint8).map_err(js)?.dyn_into().map_err(|_| "not a promise".to_string())?;
    wasm_bindgen_futures::JsFuture::from(write_prom).await.map_err(js)?;

    let close_fn: js_sys::Function =
        js_sys::Reflect::get(&stream, &JsValue::from_str("close")).map_err(js)?.dyn_into().map_err(|_| "close is not a function".to_string())?;
    let close_prom: js_sys::Promise = close_fn.call0(&stream).map_err(js)?.dyn_into().map_err(|_| "not a promise".to_string())?;
    wasm_bindgen_futures::JsFuture::from(close_prom).await.map_err(js)?;
    Ok(())
}

fn trigger_anchor_download(window: &web_sys::Window, name: &str, bytes: &[u8]) -> Result<(), String> {
    let js = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let document = window.document().ok_or("no document")?;
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type(mime_for(name));
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts).map_err(js)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(js)?;
    let a: web_sys::HtmlAnchorElement = document.create_element("a").map_err(js)?.dyn_into().map_err(|_| "not an anchor".to_string())?;
    a.set_href(&url);
    a.set_download(name);
    a.style().set_property("display", "none").map_err(js)?;
    let body = document.body().ok_or("no body")?;
    body.append_child(&a).map_err(js)?;
    a.click();
    a.remove();
    // Revoke after the click has been dispatched; the download keeps its own reference.
    let revoke = wasm_bindgen::closure::Closure::once_into_js(move || {
        web_sys::Url::revoke_object_url(&url).ok();
    });
    window.set_timeout_with_callback_and_timeout_and_arguments_0(revoke.unchecked_ref(), 10_000).map_err(js)?;
    Ok(())
}

/// The hidden iframe the last print loaded its PDF into.
const PRINT_FRAME_ID: &str = "wordcraft-print-frame";

/// Opens `bytes` (a PDF) as an object URL in a hidden iframe and calls the
/// iframe's own `print()` once it has finished loading — this is the
/// standard way to drive the browser's native print dialog on a PDF without
/// a download or a popup window the browser might block. The previous
/// print's iframe and object URL are removed first, so only one copy of the
/// document's PDF stays in the page (the last one may still be printing).
fn print_pdf(bytes: &[u8]) -> Result<(), String> {
    let js = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;

    if let Some(old) = document.get_element_by_id(PRINT_FRAME_ID) {
        if let Ok(frame) = old.clone().dyn_into::<web_sys::HtmlIFrameElement>() {
            let src = frame.src();
            if src.starts_with("blob:") {
                web_sys::Url::revoke_object_url(&src).ok();
            }
        }
        old.remove();
    }

    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type("application/pdf");
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts).map_err(js)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(js)?;

    let iframe: web_sys::HtmlIFrameElement =
        match document.create_element("iframe").map_err(js).and_then(|e| e.dyn_into().map_err(|_| "not an iframe".to_string())) {
            Ok(f) => f,
            Err(e) => {
                web_sys::Url::revoke_object_url(&url).ok();
                return Err(e);
            }
        };
    iframe.set_id(PRINT_FRAME_ID);
    let attached = iframe
        .style()
        .set_property("display", "none")
        .map_err(js)
        .and_then(|()| document.body().ok_or_else(|| "no body".to_string()))
        .and_then(|body| body.append_child(&iframe).map_err(js));
    if let Err(e) = attached {
        iframe.remove();
        web_sys::Url::revoke_object_url(&url).ok();
        return Err(e);
    }

    // `onload` fires once the PDF has actually rendered inside the iframe;
    // printing before that would show a blank page. Set the handler BEFORE
    // `src` so a fast/cached load can't fire before we're listening.
    let iframe_for_load = iframe.clone();
    let on_load = wasm_bindgen::closure::Closure::once_into_js(move || {
        if let Some(w) = iframe_for_load.content_window() {
            // Ignore errors: some browsers (notably Firefox with a built-in
            // PDF viewer) print fine but report a benign cross-origin-style
            // error back on this call.
            let _ = w.print();
        }
    });
    iframe.set_onload(Some(on_load.unchecked_ref()));
    iframe.set_src(&url);
    Ok(())
}

fn mime_for(name: &str) -> &'static str {
    match name.rsplit('.').next().map(str::to_ascii_lowercase).as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("pdf") => "application/pdf",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("odt") => "application/vnd.oasis.opendocument.text",
        Some("rtf") => "application/rtf",
        Some("html") => "text/html",
        Some("md" | "txt") => "text/plain",
        Some("tex") => "application/x-tex",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    }
}

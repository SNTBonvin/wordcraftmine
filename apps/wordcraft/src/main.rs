//! WordCraft desktop app.
//!
//! Usage: `wordcraft [--control <port>] [--sample] [files…]`
//!
//! `--control <port>` (or `WORDCRAFT_CONTROL_PORT`) starts a localhost JSON-lines control server:
//! `{"id":1,"key":"…","method":"engine.execute","params":{"command":"text.insert","params":{"text":"Hi"}}}`.
//! Every request needs the key the app writes to `<settings>/control-key.<instance>` (mode 0600,
//! removed on exit). See `docs/control-protocol.md`.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod agent_setup;
mod animations;
#[cfg(target_os = "macos")]
mod apple_events;
mod clipboard;
mod control_server;
mod default_apps;
mod file_dialogs;
#[cfg(any(target_os = "windows", test))]
mod graphics;
mod logging;
mod print;
mod region;

use wordcraft_engine::Session;
use wordcraft_ui_egui::{
    Services, UiState, WordApp,
    window_geometry::{WindowGeometry, take_rescue},
};

/// The app, the restored window geometry until the first frame has checked it, the control
/// server's key file (removed on exit), the file dialogs the app asked for and, on macOS, the
/// documents opened from Finder.
struct App(
    WordApp,
    Option<WindowGeometry>,
    Option<control_server::Server>,
    file_dialogs::Launcher,
    std::sync::Arc<std::sync::Mutex<control_server::AgentRuntime>>,
    #[cfg(target_os = "macos")] fmv_macos_events::Inbox,
);

impl App {
    fn stop_agents(&mut self) {
        if let Ok(mut runtime) = self.4.lock() {
            runtime.stop();
        }
        if let Some(server) = self.2.as_mut() {
            server.stop();
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        #[cfg(target_os = "macos")]
        apple_events::poll(&self.5, &mut self.0, ctx);
        self.0.logic(ctx);
        self.3.show(frame);
        let prev = self.0.ui.window;
        self.0.ui.window = ctx.input(|i| WindowGeometry::track(prev, i.viewport(), i.viewport_rect().size()));
        if let Some(pos) = ctx.input(|i| take_rescue(&mut self.1, i.viewport())) {
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
        }
        // The window's close button: with unsaved changes, ask first (`WordApp::close_requested`).
        let closing = ctx.input(|i| i.viewport().close_requested());
        if closing && !self.0.close_requested() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        } else if (closing || self.0.quit_requested) && !self.0.printing.busy() {
            // On macOS, letting eframe drop the window inside `-[NSApplication run]` aborts in
            // `-[_NSTouchBarFinderObservation invalidate]`. Command-Q calls `terminate:`. Once
            // the close is allowed, take that path instead of dropping the window on this turn.
            #[cfg(target_os = "macos")]
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.0.quit_requested = false;
                self.stop_agents();
                schedule_macos_terminate(&self.0);
            }
            #[cfg(not(target_os = "macos"))]
            if self.0.quit_requested {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.0.raw_input_hook(raw);
    }
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.0.ui(ui);
        self.3.show(frame);
    }
    fn on_exit(&mut self) {
        print::finish_pending_jobs();
        save_prefs(&self.0);
        self.stop_agents();
    }
}

/// Quit the way Command-Q does, on the next turn of the main queue.
///
/// Called when the window is closing. Doing it on the next turn lets the close
/// button's display cycle finish while the window still exists; `terminate:`
/// then ends the process. Preferences are written and the control key file is
/// removed first, as `on_exit` does, because a fallback [`std::process::exit`]
/// skips `on_exit`. A second close while a quit is already scheduled is ignored.
#[cfg(target_os = "macos")]
fn schedule_macos_terminate(app: &WordApp) {
    use std::sync::atomic::{AtomicBool, Ordering};
    if app.printing.busy() {
        return;
    }
    static QUIT: AtomicBool = AtomicBool::new(false);
    if QUIT.swap(true, Ordering::Relaxed) {
        return;
    }
    log::info!("window close: quitting");
    // This path can bypass on_exit. Join the owned workers before termination and cleanup.
    print::finish_pending_jobs();
    save_prefs(app);
    dispatch2::DispatchQueue::main().exec_async(|| {
        if let Some(mtm) = objc2::MainThreadMarker::new() {
            objc2_app_kit::NSApplication::sharedApplication(mtm).terminate(None);
        }
        // `terminate:` does not return when it succeeds. If it did, leave anyway:
        // staying up after the close was cancelled would trap the user in the window.
        std::process::exit(0);
    });
}

/// `ui.json` in the settings folder, which the MCP bridge also uses to find the control key.
fn prefs_path() -> Option<std::path::PathBuf> {
    wordcraft_control_key::settings_dir().map(|b| b.join("ui.json"))
}

/// Where the log files live: `logs` in the preferences folder (see `logging`).
fn log_dir() -> Option<std::path::PathBuf> {
    Some(prefs_path()?.parent()?.join("logs"))
}

/// `custom-dictionary.txt` in the settings folder: the words Add to Dictionary kept (#40).
fn user_dict_path() -> Option<std::path::PathBuf> {
    wordcraft_control_key::settings_dir().map(|b| b.join(wordcraft_engine::Session::USER_DICT_FILE))
}

fn load_user_dictionary(app: &mut WordApp) {
    if !prefs_enabled() {
        return;
    }
    if let Some(p) = user_dict_path() {
        wordcraft_proof::load_user_dictionary(&p);
        app.session.user_dict_path = Some(p);
    }
}

/// Runs without preferences (`WORDCRAFT_NO_PREFS`, agents' test runs) neither read nor write them.
fn prefs_enabled() -> bool {
    std::env::var_os("WORDCRAFT_NO_PREFS").is_none()
}

fn load_prefs(app: &mut WordApp) {
    if !prefs_enabled() {
        return;
    }
    if let Some(p) = prefs_path()
        && let Ok(bytes) = std::fs::read(&p)
        && let Some(ui) = UiState::from_json(&bytes)
    {
        app.apply_prefs(ui);
    }
}

/// Fonts the user added (#582) live in `fonts` in the settings folder: load them, and store fonts
/// added later there. Runs without preferences neither read nor keep them.
fn load_user_fonts() {
    if !prefs_enabled() {
        return;
    }
    let Some(dir) = wordcraft_control_key::settings_dir().map(|d| d.join("fonts")) else { return };
    let (loaded, skipped) = wordcraft_fonts::FontDb::global().load_user_fonts(&dir);
    if !loaded.is_empty() || !skipped.is_empty() {
        log::info!("user fonts: {} loaded, {} skipped from {}", loaded.len(), skipped.len(), dir.display());
    }
}

/// The saved window geometry, read before the window opens (`load_prefs` runs after).
fn saved_window() -> Option<WindowGeometry> {
    if !prefs_enabled() {
        return None;
    }
    let bytes = std::fs::read(prefs_path()?).ok()?;
    UiState::from_json(&bytes)?.window?.sanitized()
}

fn save_prefs(app: &WordApp) {
    if !prefs_enabled() {
        return;
    }
    if let Some(p) = prefs_path() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(bytes) = serde_json::to_vec_pretty(&app.prefs()) {
            let _ = std::fs::write(&p, bytes);
        }
    }
}

/// Native file dialogs, shown without blocking the window (`file_dialogs`, #94), File › Print
/// through the system spooler (`print`), the system clipboard's text, formatted copies,
/// pictures and format names (`clipboard`, #45, #584, #687, #833), the system's animation setting
/// (`animations`), and making WordCraft the default app for Word documents (`default_apps`, #295).
fn services(file_dialog: file_dialogs::Hook, ctx: &egui::Context) -> Services {
    Services {
        file_dialog: Some(file_dialog),
        printers: Some(print::discover()),
        print_native: Some(print::submit()),
        clipboard_picture: Some(Box::new(clipboard::picture)),
        make_default_app: default_apps::hook(),
        animations_enabled: Some(animations::hook(ctx)),
        clipboard_text: Some(Box::new(clipboard::text)),
        clipboard_html: Some(Box::new(clipboard::html)),
        clipboard_rtf: Some(Box::new(clipboard::rtf)),
        clipboard_formats: Some(Box::new(clipboard::formats)),
        ..Default::default()
    }
}

/// Window, Dock and taskbar icon.
fn app_icon() -> Option<egui::IconData> {
    #[cfg(target_os = "macos")]
    let png: &[u8] = include_bytes!("../../../assets/app-icon/wordcraft-macos-512.png");
    #[cfg(not(target_os = "macos"))]
    let png: &[u8] = include_bytes!("../../../assets/app-icon/hicolor/256x256/apps/ai.storyteller.wordcraft.png");
    eframe::icon_data::from_png_bytes(png).map_err(|e| log::warn!("app icon: {e}")).ok()
}

/// The commit the build came from (short), or `dev`.
fn build_sha() -> &'static str {
    option_env!("WORDCRAFT_BUILD_SHA").map(|s| s.get(..8).unwrap_or(s)).unwrap_or("dev")
}

struct DesktopArgs {
    control_port: Option<u16>,
    sample: bool,
    version: bool,
    files: Vec<std::path::PathBuf>,
}

fn desktop_args(mut args: impl Iterator<Item = std::ffi::OsString>, control_port: Option<u16>) -> DesktopArgs {
    let mut parsed = DesktopArgs { control_port, sample: false, version: false, files: Vec::new() };
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--control") => parsed.control_port = args.next().and_then(|p| p.to_str().and_then(|s| s.parse().ok())),
            Some("--sample") => parsed.sample = true,
            Some("--version") => {
                parsed.version = true;
                break;
            }
            _ => parsed.files.push(std::path::PathBuf::from(arg)),
        }
    }
    parsed
}

fn main() -> eframe::Result {
    // MCP must own clean stdio, before the GUI, log redirection or preference loading.
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--mcp-live")) {
        if std::env::args_os().nth(2).is_some() {
            eprintln!("--mcp-live does not accept additional arguments");
            std::process::exit(2);
        }
        if let Err(error) = wordcraft_mcp::serve_live(std::io::stdin().lock(), std::io::stdout()) {
            eprintln!("WordCraft MCP: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    // First, so the panic hook and every start-up warning are recorded (`logging`).
    let logger = logging::install();
    logging::install_panic_hook();
    let control_port = std::env::var("WORDCRAFT_CONTROL_PORT").ok().and_then(|p| p.parse().ok());
    let DesktopArgs { control_port, sample, version, files } = desktop_args(std::env::args_os().skip(1), control_port);
    if version {
        println!("wordcraft {} ({})", env!("CARGO_PKG_VERSION"), build_sha());
        return Ok(());
    }
    // The log file lives in the settings directory, next to the preferences; opened after the
    // arguments, so `--version` leaves no file behind. Records logged until now are written to it
    // first. Runs without preferences (agents' test runs) log to standard error only, so they
    // don't rotate away the user's own logs.
    if let Some(logger) = logger {
        match log_dir().filter(|_| prefs_enabled()) {
            Some(dir) => match logger.attach_dir(&dir) {
                Ok(path) => log::info!("WordCraft {} ({}), log file {}", env!("CARGO_PKG_VERSION"), build_sha(), path.display()),
                // Standard error only by now (`attach_dir` gave up on the file); unlike `eprintln!`, never panics.
                Err(e) => log::warn!("no log file: {e}"),
            },
            None => logger.no_file(),
        }
    }
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("WordCraft")
            .with_app_id("ai.storyteller.wordcraft")
            .with_inner_size([1440.0, 920.0])
            .with_min_inner_size([760.0, 480.0])
            .with_drag_and_drop(true)
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false),
        ..Default::default()
    };
    if let Some(icon) = app_icon() {
        options.viewport = options.viewport.with_icon(icon);
    }
    // Before eframe creates the wgpu instance: default Windows to DirectX 12 only, or OpenGL when
    // DirectX 12 has no GPU driver or this is a 32-bit build, and compile DirectX 12 shaders with
    // FXC, which needs no DLL of ours (#603; see graphics.rs).
    #[cfg(target_os = "windows")]
    graphics::configure(&mut options, eframe::wgpu::Backends::from_env());
    load_user_fonts();
    let restored = saved_window();
    if let Some(window) = restored {
        options.viewport = window.apply(options.viewport);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    // winit has no file drag-and-drop on Wayland (only on X11), so dropping files from the file
    // manager showed a "no" cursor. Run through XWayland when it's there; WORDCRAFT_WAYLAND=1
    // keeps the native Wayland backend.
    if std::env::var_os("DISPLAY").is_some() && std::env::var_os("WORDCRAFT_WAYLAND").is_none() {
        options.event_loop_builder = Some(Box::new(|b| {
            use winit::platform::x11::EventLoopBuilderExtX11;
            b.with_x11();
        }));
    }
    // Registered before the event loop starts, so it catches the Finder event that launched us as
    // well as later ones. Lives until the event loop returns; the app creator only borrows it.
    #[cfg(target_os = "macos")]
    let apple_events = apple_events::AppleEvents::install();
    #[cfg(target_os = "macos")]
    let apple_events = &apple_events;
    eframe::run_native(
        "WordCraft",
        options,
        Box::new(move |cc| {
            // The system region picks the new document's paper, the units and the decimal
            // separator (#838).
            let region = region::system();
            let doc = if sample { wordcraft_engine::sample::sample_document() } else { wordcraft_engine::blank_document(&region) };
            let (file_dialog, dialogs) = file_dialogs::hook(&cc.egui_ctx);
            let mut session = Session::new(doc);
            session.system_region = region;
            let (control_tx, control_rx) = control_server::request_channel();
            let runtime = std::sync::Arc::new(std::sync::Mutex::new(control_server::AgentRuntime::default()));
            let mut services = services(file_dialog, &cc.egui_ctx);
            let (hook_runtime, hook_tx, hook_ctx) = (runtime.clone(), control_tx.clone(), cc.egui_ctx.clone());
            services.agent_control = Some(Box::new(move |request| {
                if request.get("setup").and_then(serde_json::Value::as_bool) == Some(true) {
                    return Ok(agent_setup::setup_metadata_request(request.get("refresh").and_then(serde_json::Value::as_bool) == Some(true)));
                }
                let settings = wordcraft_control_key::settings_dir().ok_or("WordCraft settings folder unavailable")?;
                let mut runtime = hook_runtime.lock().map_err(|_| "agent runtime unavailable")?;
                let mut descriptor = runtime.update(request, &hook_ctx, &settings, hook_tx.clone())?;
                if descriptor.get("sessionId").is_some() {
                    agent_setup::decorate_descriptor(&mut descriptor);
                }
                Ok(descriptor)
            }));
            let mut app = WordApp::new(session, services).with_control(control_rx);
            load_prefs(&mut app);
            load_user_dictionary(&mut app);
            // A region chosen in File › Options replaces the system's for the first blank
            // document too (#838).
            if !sample && app.session.prefs.region.is_some() {
                let doc = app.session.blank_document();
                app.session.set_document(doc);
            }
            app.watch_desktop_theme(&cc.egui_ctx);
            app.ui.window = restored;
            app.integrated_titlebar = cfg!(target_os = "macos");
            let mut control_server = None;
            if let Some(port) = control_port {
                match wordcraft_control_key::settings_dir()
                    .ok_or_else(|| "settings folder unavailable".to_string())
                    .and_then(|dir| control_server::start(port, cc.egui_ctx.clone(), &dir, control_tx, false))
                {
                    Ok(server) => control_server = Some(server),
                    Err(error) => log::error!("control channel unavailable: {error}"),
                }
            }
            // Nothing to open (no files, no sample, no control channel driving the editor): start
            // on File › Home, as Word does (#766), unless the Start screen option is off. Files
            // macOS hands over later open on top of it and close it (`apple_events`).
            if files.is_empty() && !sample && control_port.is_none() {
                app.start_screen();
            }
            for f in files {
                if let Some(path) = f.to_str() {
                    if let Err(error) = app.run("file.open", serde_json::json!({"path": path})) {
                        log::warn!("{}: {error}", f.display());
                    }
                } else {
                    // The command protocol accepts Unicode paths. Native argument parsing
                    // still avoids a startup panic; broader path support is separate work.
                    log::warn!("Cannot open {}: the document path is not valid Unicode", f.display());
                }
            }
            Ok(Box::new(App(
                app,
                restored,
                control_server,
                dialogs,
                runtime,
                #[cfg(target_os = "macos")]
                apple_events.connect(&cc.egui_ctx),
            )))
        }),
    )
}

#[cfg(all(test, unix))]
mod args_tests {
    use super::*;
    use std::os::unix::ffi::OsStringExt;

    #[test]
    fn native_arguments_preserve_non_unicode_filenames_and_reject_bad_port_without_panicking() {
        let file = std::ffi::OsString::from_vec(vec![b'd', b'o', b'c', 0xff]);
        let bad_port = std::ffi::OsString::from_vec(vec![0xff]);
        let parsed = desktop_args(vec!["--sample".into(), file.clone(), "--control".into(), bad_port].into_iter(), Some(7981));
        assert!(parsed.sample);
        assert!(!parsed.version);
        assert_eq!(parsed.control_port, None);
        assert_eq!(parsed.files, vec![std::path::PathBuf::from(file)]);
        let parsed = desktop_args(vec!["--version".into(), std::ffi::OsString::from_vec(vec![0xff])].into_iter(), None);
        assert!(parsed.version);
    }
}

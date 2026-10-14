//! Ready-to-copy setup for the persistent live MCP bridge. Never include a control key.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde_json::{Value, json};

#[derive(Clone)]
struct BridgeSetup {
    config: String,
    status: String,
    ready: bool,
    command: Option<PathBuf>,
    args: Vec<String>,
}

#[derive(Default)]
struct SetupCache {
    value: Option<BridgeSetup>,
    running: bool,
}

fn setup_cache() -> &'static std::sync::Arc<std::sync::Mutex<SetupCache>> {
    static SETUP: OnceLock<std::sync::Arc<std::sync::Mutex<SetupCache>>> = OnceLock::new();
    SETUP.get_or_init(Default::default)
}

// The mutex protects only a small snapshot. Discovery and subprocesses never run under it
// or on the UI thread. Refresh while a job is running reuses that job (single flight).
fn request_setup(
    cache: &std::sync::Arc<std::sync::Mutex<SetupCache>>,
    refresh: bool,
    discover: impl FnOnce() -> BridgeSetup + Send + 'static,
) -> BridgeSetup {
    let Ok(mut state) = cache.lock() else { return unavailable("MCP setup state unavailable; reopen WordCraft") };
    if state.running {
        return unavailable("setup_checking");
    }
    if !refresh && let Some(value) = &state.value {
        return value.clone();
    }
    state.running = true;
    state.value = None;
    drop(state);
    let worker_cache = cache.clone();
    if let Err(error) = std::thread::Builder::new().name("wordcraft-mcp-setup".into()).spawn(move || {
        let value = discover();
        if let Ok(mut state) = worker_cache.lock() {
            state.value = Some(value);
            state.running = false;
        }
    }) {
        let value = unavailable(format!("Cannot start MCP setup check: {error}"));
        if let Ok(mut state) = cache.lock() {
            state.running = false;
            state.value = Some(value.clone());
        }
        return value;
    }
    unavailable("setup_checking")
}

/// Add client-neutral setup and an invitation to a public live-session descriptor.
/// The executable belongs to this installation; arbitrary PATH candidates are not trusted.
pub fn decorate_descriptor(descriptor: &mut Value) {
    // Enabling discovery should neither probe executables nor start a setup job.
    let setup = setup_cache().lock().ok().and_then(|state| state.value.clone()).unwrap_or_else(|| unavailable("setup_not_checked"));
    decorate_with(descriptor, &setup);
}

fn setup(refresh: bool) -> BridgeSetup {
    request_setup(setup_cache(), refresh, discover_setup)
}

/// Setup is independent of discovery or editing permission. It never starts a listener.
pub fn setup_metadata_request(refresh: bool) -> Value {
    let mut metadata = json!({});
    decorate_with(&mut metadata, &setup(refresh));
    metadata
}

fn decorate_with(descriptor: &mut Value, setup: &BridgeSetup) {
    let session = descriptor.get("sessionId").cloned().unwrap_or(Value::Null);
    let document = descriptor.get("documentId").cloned().unwrap_or(Value::Null);
    let title = descriptor.get("title").cloned().unwrap_or(Value::Null);
    let attach = json!({"sessionId": session, "agentName": "Your agent name"});
    let invitation = format!(
        "Use the WordCraft document already open in my window.\n\
         Session ID: {session}\nDocument ID: {document}\nDocument title: {title}\n\n\
         Call connect_session {attach} (replace the agent name). Confirm this document with \
         get_session_status, then call get_usage_guide before editing. \
         If WordCraft tools are unavailable, use the one-time MCP setup shown in WordCraft's Agents page. \
         Wait for the owner's approval and stop if access is revoked."
    );
    if let Some(fields) = descriptor.as_object_mut() {
        fields.insert("mcpConfig".into(), Value::String(setup.config.clone()));
        if !session.is_null() {
            fields.insert("invitation".into(), Value::String(invitation));
        }
        let (code, recovery) = setup_codes(setup);
        fields.insert("setupCode".into(), json!(code));
        fields.insert("recoveryCode".into(), json!(recovery));
        fields.insert("detail".into(), Value::String(setup.status.clone()));
        fields.insert("setupReady".into(), Value::Bool(setup.ready));
        fields.insert("bridgeVersion".into(), if setup.ready { json!(env!("CARGO_PKG_VERSION")) } else { Value::Null });
        fields.insert("expectedBridgeVersion".into(), json!(env!("CARGO_PKG_VERSION")));
        fields.insert("setupCommand".into(), json!(setup.command.as_ref().and_then(|p| p.to_str())));
        fields.insert("setupArgs".into(), json!(setup.args));
        fields.insert("setupHelpUrl".into(), json!(format!("https://github.com/storytold/wordcraft/releases/tag/v{}", env!("CARGO_PKG_VERSION"))));
    }
}

fn setup_codes(setup: &BridgeSetup) -> (&'static str, &'static str) {
    if setup.ready {
        return ("ready", "configure_client");
    }
    match setup.status.as_str() {
        "setup_checking" => ("checking", "wait"),
        "setup_not_checked" => ("not_checked", "check_setup"),
        s if s.contains("Unicode") => ("invalid_path", "unicode_path"),
        s if s.contains("missing beside") => ("missing_bridge", "install_matching"),
        s if s.contains("version does not match") || s.contains("does not support") => ("incompatible_bridge", "install_matching"),
        _ => ("setup_failed", "refresh_setup"),
    }
}

fn config(command: &Path, args: &[&str]) -> Result<String, String> {
    let command = command.to_str().ok_or("The bridge executable path is not valid Unicode; move WordCraft to a Unicode path.")?;
    if !Path::new(command).is_absolute() {
        return Err("The bridge executable path is not absolute.".into());
    }
    serde_json::to_string_pretty(&json!({"mcpServers": {"wordcraft": {"command": command, "args": args}}}))
        .map_err(|e| format!("Cannot generate MCP configuration: {e}"))
}

fn ready(command: &Path, args: &[&str], status: &str) -> BridgeSetup {
    match config(command, args) {
        Ok(config) => BridgeSetup {
            config,
            status: status.into(),
            ready: true,
            command: Some(command.to_path_buf()),
            args: args.iter().map(|s| s.to_string()).collect(),
        },
        Err(status) => unavailable(status),
    }
}

fn unavailable(status: impl Into<String>) -> BridgeSetup {
    BridgeSetup { config: String::new(), status: status.into(), ready: false, command: None, args: Vec::new() }
}

fn discover_setup() -> BridgeSetup {
    if std::env::var_os("FLATPAK_ID").is_some() || Path::new("/.flatpak-info").is_file() {
        return ready(
            Path::new("/usr/bin/flatpak"),
            &["run", "--command=wordcraft-cli", "ai.storyteller.wordcraft", "mcp", "--live"],
            "Bundled Flatpak CLI. Register this configuration in a client on the same host. The private Unix socket connects app instances without a networking permission or restart.",
        );
    }
    // AppImage mounts are transient. Launch the actual AppImage, which supports the same
    // early --mcp-live mode, instead of exporting an executable under /tmp/.mount_… .
    if let Some(appimage) = std::env::var_os("APPIMAGE") {
        let path = PathBuf::from(appimage);
        if !path.is_absolute() || !path.is_file() {
            return unavailable("Cannot locate the original AppImage. Start the AppImage normally and enable agent access again.");
        }
        return ready(
            &path,
            &["--mcp-live"],
            "Live MCP bridge is included in this AppImage. Register the configuration once; update it if you move the AppImage.",
        );
    }
    let exe = match std::env::current_exe() {
        Ok(path) => path,
        Err(e) => return unavailable(format!("Cannot locate WordCraft's executable: {e}")),
    };
    native_setup(&exe)
}

#[cfg(not(target_os = "windows"))]
fn native_setup(exe: &Path) -> BridgeSetup {
    ready(
        exe,
        &["--mcp-live"],
        "Live MCP bridge is included in this WordCraft executable. Register the configuration once; the MCP client may require a tool reload or a new session.",
    )
}

#[cfg(target_os = "windows")]
fn native_setup(exe: &Path) -> BridgeSetup {
    // A Windows release GUI binary has the GUI subsystem, whose Rust stdio is not suitable
    // for an MCP bridge. The MSI and portable release already bundle the console CLI.
    let Some(parent) = exe.parent() else {
        return unavailable("Cannot locate the folder containing WordCraft.");
    };
    let cli = parent.join("wordcraft-cli.exe");
    if !cli.is_file() {
        return unavailable(
            "wordcraft-cli.exe is missing beside WordCraft. Reinstall the matching Windows MSI or extract both executables from the portable release into the same folder.",
        );
    }
    match verify_cli(&cli) {
        Ok(()) => ready(
            &cli,
            &["mcp", "--live"],
            "Matching bundled console CLI is available. Register the configuration once; the MCP client may require a tool reload or a new session.",
        ),
        Err(e) => unavailable(format!("Cannot use the bundled CLI: {e}. Reinstall the matching Windows release (both executables).")),
    }
}

#[cfg(target_os = "windows")]
fn verify_cli(cli: &Path) -> Result<(), String> {
    let version = probe_cli(cli, "--version")?;
    let help = probe_cli(cli, "--help")?;
    validate_cli_output(&version, &help)
}

#[cfg(any(target_os = "windows", test))]
fn validate_cli_output(version: &str, help: &str) -> Result<(), String> {
    let expected = format!("wordcraft-cli {}", env!("CARGO_PKG_VERSION"));
    if !version.lines().any(|line| line == expected || line.starts_with(&format!("{expected} ("))) {
        return Err("its version does not match WordCraft".into());
    }
    if !help.contains("--live") {
        return Err("it does not support the live MCP bridge".into());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn probe_cli(cli: &Path, arg: &str) -> Result<String, String> {
    probe_cli_with_args(cli, &[arg], std::time::Duration::from_secs(1))
}

/// A regular anonymous temporary file has EOF at its current length even when a descendant
/// retains stdout. Unlike a pipe, reading the CLI's completed output cannot wait for it to
/// close an inherited handle. The OS owns cleanup when the last file handle closes.
#[cfg(any(target_os = "windows", test))]
fn probe_cli_with_args(cli: &Path, args: &[&str], timeout: std::time::Duration) -> Result<String, String> {
    use std::io::{Read, Seek, SeekFrom};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    const MAX_OUTPUT: u64 = 65_536;
    let deadline = Instant::now().checked_add(timeout).ok_or("CLI probe timeout is out of range")?;
    let mut output = tempfile::tempfile().map_err(|e| format!("Cannot create private CLI probe output: {e}"))?;
    let stdout = output.try_clone().map_err(|e| e.to_string())?;
    let mut command = Command::new(cli);
    command.args(args).stdin(Stdio::null()).stdout(Stdio::from(stdout)).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = CheckedChild { child: command.spawn().map_err(|e| e.to_string())?, finished: false };
    loop {
        if output.metadata().map_err(|e| e.to_string())?.len() > MAX_OUTPUT {
            return Err("CLI probe output exceeds 64 KiB".into());
        }
        if let Some(status) = child.child.try_wait().map_err(|e| e.to_string())? {
            child.finished = true;
            if !status.success() {
                return Err(format!("CLI probe returned {status}"));
            }
            break;
        }
        if Instant::now() >= deadline {
            return Err("CLI probe timed out".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let expected = output.metadata().map_err(|e| e.to_string())?.len();
    if expected > MAX_OUTPUT {
        return Err("CLI probe output exceeds 64 KiB".into());
    }
    output.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    (&mut output).take(MAX_OUTPUT + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let after = output.metadata().map_err(|e| e.to_string())?.len();
    if bytes.len() as u64 > MAX_OUTPUT || after > MAX_OUTPUT {
        return Err("CLI probe output exceeds 64 KiB".into());
    }
    if bytes.len() as u64 != expected || after != expected {
        return Err("CLI probe output changed after the process exited".into());
    }
    String::from_utf8(bytes).map_err(|e| format!("CLI probe output is not UTF-8: {e}"))
}

/// The exported command may be a host launcher (Flatpak/AppImage). The in-app check
/// runs the already installed executable directly inside the current environment instead.
fn check_setup(_cancelled: &std::sync::atomic::AtomicBool) -> BridgeSetup {
    #[cfg(unix)]
    {
        let flatpak = std::env::var_os("FLATPAK_ID").is_some() || Path::new("/.flatpak-info").is_file();
        if flatpak {
            return internal_unix_setup(Path::new("/app/bin/wordcraft"), true);
        }
        match std::env::current_exe() {
            Ok(exe) => internal_unix_setup(&exe, false),
            Err(error) => unavailable(format!("Cannot locate the installed MCP bridge: {error}")),
        }
    }
    #[cfg(not(unix))]
    {
        // Reuse the setup job if the owner checks while the pane is still probing.
        // This wait is only on the check worker, never the UI thread.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            let snapshot = setup(false);
            if snapshot.status != "setup_checking" {
                return snapshot;
            }
            if _cancelled.load(std::sync::atomic::Ordering::Acquire) {
                return unavailable("MCP check cancelled");
            }
            if std::time::Instant::now() >= deadline {
                return unavailable("MCP setup check timed out");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

#[cfg(unix)]
fn internal_unix_setup(exe: &Path, flatpak: bool) -> BridgeSetup {
    if flatpak {
        ready(Path::new("/app/bin/wordcraft-cli"), &["mcp", "--live"], "Checking the bundled CLI inside this Flatpak")
    } else {
        ready(exe, &["--mcp-live"], "Checking this running installation's MCP bridge")
    }
}

/// A bounded, asynchronous check of the installed stdio bridge and intended live document.
/// Dropping the job requests cancellation; the UI never waits for the worker or child.
pub struct CheckJob {
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    result: std::sync::Arc<std::sync::Mutex<Option<Result<Value, String>>>>,
}
impl CheckJob {
    pub fn start(descriptor: Value, ctx: egui::Context) -> Result<Self, String> {
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let result = std::sync::Arc::new(std::sync::Mutex::new(None));
        let (worker_cancelled, worker_result) = (cancelled.clone(), result.clone());
        std::thread::Builder::new()
            .name("wordcraft-mcp-check".into())
            .spawn(move || {
                let setup = check_setup(&worker_cancelled);
                let outcome = check_bridge(&setup, &descriptor, &worker_cancelled);
                if let Ok(mut result) = worker_result.lock() {
                    *result = Some(outcome);
                }
                ctx.request_repaint();
            })
            .map_err(|e| format!("Cannot start MCP check: {e}"))?;
        Ok(Self { cancelled, result })
    }
    pub fn poll(&self) -> Value {
        match self.result.lock() {
            Ok(result) => match result.as_ref() {
                None => json!({"status":"checking","checkReady":false,"detail":"Checking the installed MCP bridge and this document…"}),
                Some(Ok(document)) => {
                    json!({"status":"ready","checkReady":true,"detail":"The installed MCP bridge reached this open document.","checkDocument":document})
                }
                Some(Err(error)) => check_error_metadata(error),
            },
            Err(_) => check_error_metadata("MCP check state unavailable"),
        }
    }
}

fn check_error_metadata(error: &str) -> Value {
    let (code, recovery) = if error.contains("timed out") {
        ("check_timeout", "retry_check")
    } else if error.contains("version does not match") || error.contains("does not support live") || error.contains("missing beside") {
        ("incompatible_bridge", "install_matching")
    } else if error.contains("different or incompatible document") || error.contains("intended document") {
        ("document_unreachable", "enable_requests")
    } else if error.contains("cancelled") {
        ("check_cancelled", "retry_check")
    } else {
        ("check_failed", "refresh_setup")
    };
    json!({"status":"error","checkReady":false,"code":code,"recoveryCode":recovery,"detail":error})
}
impl Drop for CheckJob {
    fn drop(&mut self) {
        self.cancelled.store(true, std::sync::atomic::Ordering::Release);
    }
}

struct CheckedChild {
    child: std::process::Child,
    finished: bool,
}
impl Drop for CheckedChild {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        #[cfg(unix)]
        if let Ok(raw) = i32::try_from(self.child.id())
            && let Some(pid) = rustix::process::Pid::from_raw(raw)
        {
            // Only this child's separately created group, including flatpak/AppImage launchers.
            let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn check_bridge(setup: &BridgeSetup, descriptor: &Value, cancelled: &std::sync::atomic::AtomicBool) -> Result<Value, String> {
    check_bridge_with_timeout(setup, descriptor, cancelled, std::time::Duration::from_secs(5))
}

fn check_bridge_with_timeout(
    setup: &BridgeSetup,
    descriptor: &Value,
    cancelled: &std::sync::atomic::AtomicBool,
    timeout: std::time::Duration,
) -> Result<Value, String> {
    use std::io::{Read, Write};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + timeout;
    let command = setup.command.as_ref().ok_or_else(|| setup.status.clone())?;
    let mut launch = Command::new(command);
    launch.args(&setup.args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        launch.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        launch.creation_flags(0x0800_0000);
    }
    let mut child = CheckedChild { child: launch.spawn().map_err(|e| format!("Cannot launch installed MCP bridge: {e}"))?, finished: false };
    let stdout = child.child.stdout.take().ok_or("MCP stdout is unavailable")?;
    let (output_tx, output_rx) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("wordcraft-mcp-check-output".into())
        .spawn(move || {
            let mut bytes = Vec::new();
            let outcome = stdout.take((1 << 20) + 1).read_to_end(&mut bytes).map(|_| bytes).map_err(|e| e.to_string());
            let _ = output_tx.send(outcome);
        })
        .map_err(|e| format!("Cannot read MCP check output: {e}"))?;
    let requests = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"WordCraft connection check","version":env!("CARGO_PKG_VERSION")}}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"check_session","arguments":{"sessionId":descriptor["sessionId"]}}}),
    ];
    if let Some(mut stdin) = child.child.stdin.take() {
        for request in requests {
            writeln!(stdin, "{request}").map_err(|e| format!("Cannot send MCP check: {e}"))?;
        }
    } else {
        return Err("MCP stdin is unavailable".into());
    }
    let mut captured = None;
    loop {
        if cancelled.load(std::sync::atomic::Ordering::Acquire) {
            return Err("MCP check cancelled".into());
        }
        if captured.is_none() {
            match output_rx.try_recv() {
                Ok(output) => captured = Some(output?),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return Err("MCP output reader stopped".into()),
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        if captured.as_ref().is_some_and(|bytes: &Vec<u8>| bytes.len() > 1 << 20) {
            return Err("MCP check output exceeds 1 MiB".into());
        }
        // Do not reap a wrapper before stdout closes: descendants can inherit the pipe.
        // While the child remains unreaped its process-group id cannot be reused, so the
        // timeout/cancellation guard safely terminates that entire owned group.
        if captured.is_some()
            && let Some(status) = child.child.try_wait().map_err(|e| e.to_string())?
        {
            child.finished = true;
            if !status.success() {
                return Err(format!("MCP bridge exited with {status}"));
            }
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!("MCP bridge check timed out after {} seconds", timeout.as_secs_f32()));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let bytes = captured.ok_or("MCP output did not close")?;
    let text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
    validate_check_output(&text, descriptor)
}

fn validate_check_output(text: &str, descriptor: &Value) -> Result<Value, String> {
    let mut replies = Vec::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        replies.push(serde_json::from_str::<Value>(line).map_err(|e| format!("MCP stdout is not JSON: {e}"))?);
    }
    let initialize = replies.iter().find(|v| v["id"] == 1).ok_or("MCP initialize reply is missing")?;
    if initialize["result"]["serverInfo"]["version"] != env!("CARGO_PKG_VERSION") {
        return Err("The installed MCP bridge version does not match WordCraft".into());
    }
    let tools = replies.iter().find(|v| v["id"] == 2).and_then(|v| v["result"]["tools"].as_array()).ok_or("MCP tool list is missing")?;
    if !tools.iter().any(|v| v["name"] == "check_session") {
        return Err("The MCP bridge does not support live document checks".into());
    }
    let checked = replies.iter().find(|v| v["id"] == 3).ok_or("Live document check reply is missing")?;
    if checked["result"]["isError"] != false {
        return Err("The MCP bridge could not reach the intended document".into());
    }
    let status = checked["result"]["content"]
        .as_array()
        .and_then(|items| items.first())
        .and_then(|v| v["text"].as_str())
        .ok_or("Live document check result is missing")?;
    let status: Value = serde_json::from_str(status).map_err(|e| e.to_string())?;
    if status["sessionId"] != descriptor["sessionId"]
        || status["documentId"] != descriptor["documentId"]
        || status["protocolVersion"] != 1
        || status["enabled"] != true
    {
        return Err("MCP check reached a different or incompatible document".into());
    }
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_setup_is_background_single_flight_and_refresh_replaces_the_cache() {
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        };
        use std::time::Duration;
        let cache = Arc::new(Mutex::new(SetupCache::default()));
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let state = request_setup(&cache, false, move || {
            observed.fetch_add(1, Ordering::Relaxed);
            started_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(3)).unwrap();
            ready(&std::env::temp_dir().join("test-bridge"), &["--mcp-live"], "First check")
        });
        assert_eq!(setup_codes(&state), ("checking", "wait"));
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        for refresh in [false, true, false] {
            let state = request_setup(&cache, refresh, || panic!("running setup must be reused"));
            assert_eq!(setup_codes(&state).0, "checking");
        }
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        release_tx.send(()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while cache.lock().unwrap().running && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(!cache.lock().unwrap().running);
        assert!(request_setup(&cache, false, || panic!("ready result must be cached")).ready);
        let (finished_tx, finished_rx) = mpsc::channel();
        assert_eq!(
            setup_codes(&request_setup(&cache, true, move || {
                finished_tx.send(()).unwrap();
                unavailable("wordcraft-cli.exe is missing beside WordCraft")
            }))
            .0,
            "checking"
        );
        finished_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        while cache.lock().unwrap().running && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(setup_codes(&request_setup(&cache, false, || panic!("refreshed result must be cached"))), ("missing_bridge", "install_matching"));
    }

    #[test]
    fn setup_and_check_errors_have_primary_and_recovery_codes() {
        assert_eq!(setup_codes(&unavailable("its version does not match WordCraft")), ("incompatible_bridge", "install_matching"));
        assert_eq!(setup_codes(&unavailable("path is not valid Unicode")), ("invalid_path", "unicode_path"));
        for (error, code, recovery) in [
            ("MCP bridge check timed out after 5 seconds", "check_timeout", "retry_check"),
            ("The installed MCP bridge version does not match WordCraft", "incompatible_bridge", "install_matching"),
            ("The MCP bridge could not reach the intended document", "document_unreachable", "enable_requests"),
            ("Cannot launch installed MCP bridge", "check_failed", "refresh_setup"),
        ] {
            let metadata = check_error_metadata(error);
            assert_eq!(metadata["code"], code);
            assert_eq!(metadata["recoveryCode"], recovery);
            assert_eq!(metadata["detail"], error);
            assert!(metadata.get("error").is_none());
        }
    }

    #[test]
    fn configuration_preserves_executable_path_as_one_argument() {
        let path = std::env::temp_dir().join("WordCraft space ' quote $(touch nope) `id` 漢字");
        let setup = ready(&path, &["--mcp-live"], "Ready");
        assert!(setup.ready);
        let parsed: Value = serde_json::from_str(&setup.config).unwrap();
        assert_eq!(parsed["mcpServers"]["wordcraft"]["command"], path.to_str().unwrap());
        assert_eq!(parsed["mcpServers"]["wordcraft"]["args"], json!(["--mcp-live"]));
    }

    #[test]
    fn invitation_copies_only_public_document_identity() {
        let setup = ready(&std::env::temp_dir().join("WordCraft"), &["--mcp-live"], "Ready");
        let mut descriptor = json!({"sessionId":"session-1", "documentId":"doc-1", "title":"Title\n\"quoted\"", "key":"never-print-this"});
        decorate_with(&mut descriptor, &setup);
        let invitation = descriptor["invitation"].as_str().unwrap();
        assert!(invitation.contains("session-1"));
        assert!(invitation.contains("doc-1"));
        assert!(invitation.contains("Title\\n\\\"quoted\\\""));
        assert!(!invitation.contains("never-print-this"));
        assert!(!descriptor["mcpConfig"].as_str().unwrap().contains("never-print-this"));
    }

    #[test]
    fn compact_invitation_omits_local_configuration_and_full_usage_guide() {
        let setup = ready(&std::env::temp_dir().join("private-local-installation"), &["--mcp-live"], "Local diagnostic detail");
        let mut descriptor = json!({"sessionId":"s","documentId":7,"title":"Title"});
        decorate_with(&mut descriptor, &setup);
        let invitation = descriptor["invitation"].as_str().unwrap();
        assert!(invitation.contains("get_usage_guide"));
        assert!(invitation.contains("connect_session"));
        assert!(!invitation.contains("private-local-installation"));
        assert!(!invitation.contains("Local diagnostic detail"));
        assert!(!invitation.contains("mcpServers"));
        assert!(!invitation.contains(wordcraft_mcp::live::USAGE_GUIDE));
        assert!(invitation.len() < 1024);
    }

    #[cfg(unix)]
    #[test]
    fn cli_probe_does_not_wait_for_a_descendant_to_close_stdout() {
        // The synthetic child retains the regular-file stdout after its parent exits.
        let started = std::time::Instant::now();
        let output =
            probe_cli_with_args(Path::new("/bin/sh"), &["-c", r#"sleep 30 & printf '%s\n' "$!"; exit 0"#], std::time::Duration::from_millis(500))
                .unwrap();
        let raw: i32 = output.trim().parse().unwrap();
        let pid = rustix::process::Pid::from_raw(raw).unwrap();
        let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn cli_probe_rejects_oversized_output_and_bounds_child_wait() {
        let oversized = probe_cli_with_args(Path::new("/bin/sh"), &["-c", "head -c 65537 /dev/zero"], std::time::Duration::from_secs(1)).unwrap_err();
        assert!(oversized.contains("exceeds 64 KiB"));
        let started = std::time::Instant::now();
        let timeout = probe_cli_with_args(Path::new("/bin/sh"), &["-c", "sleep 30"], std::time::Duration::from_millis(100)).unwrap_err();
        assert!(timeout.contains("timed out"));
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn internal_flatpak_and_appimage_checks_do_not_use_host_launchers() {
        let external = ready(
            Path::new("/usr/bin/flatpak"),
            &["run", "--command=wordcraft-cli", "ai.storyteller.wordcraft", "mcp", "--live"],
            "Host registration",
        );
        let internal = internal_unix_setup(Path::new("/app/bin/wordcraft"), true);
        assert_eq!(external.command.as_deref(), Some(Path::new("/usr/bin/flatpak")));
        assert_eq!(internal.command.as_deref(), Some(Path::new("/app/bin/wordcraft-cli")));
        assert_eq!(internal.args, vec!["mcp", "--live"]);
        assert!(!internal.args.iter().any(|arg| arg == "run"));
        let mounted = Path::new("/tmp/.mount_WordCraft/usr/bin/wordcraft");
        let internal = internal_unix_setup(mounted, false);
        assert_eq!(internal.command.as_deref(), Some(mounted));
        assert_eq!(internal.args, vec!["--mcp-live"]);
    }

    #[cfg(unix)]
    #[test]
    fn connection_check_kills_wrapper_descendants_that_keep_stdout_open() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("descendant-survived");
        let started_marker = dir.path().join("descendant-started");
        // Consume every request before exiting: otherwise an early shell exit can race
        // the check's stdin writes and exercise BrokenPipe rather than inherited stdout.
        // The descendant keeps stdout open and would leave a marker if group cleanup fails.
        let setup = ready(
            Path::new("/bin/sh"),
            &[
                "-c",
                "cat >/dev/null; (sleep 3; printf leaked > \"$1\") & printf started > \"$2\"; exit 0",
                "wordcraft-test",
                marker.to_str().unwrap(),
                started_marker.to_str().unwrap(),
            ],
            "Synthetic wrapper",
        );
        let started = std::time::Instant::now();
        let error = check_bridge_with_timeout(
            &setup,
            &json!({"sessionId":"s"}),
            &std::sync::atomic::AtomicBool::new(false),
            std::time::Duration::from_secs(2),
        )
        .unwrap_err();
        assert!(error.contains("timed out"), "unexpected check error: {error}");
        assert!(started.elapsed() < std::time::Duration::from_secs(4));
        assert!(started_marker.exists(), "the fixture must start its stdout-owning descendant");
        std::thread::sleep(std::time::Duration::from_millis(3200));
        assert!(!marker.exists(), "the check's descendant survived process-group cleanup");
    }

    #[test]
    fn connection_check_validates_stdio_tools_version_and_exact_document() {
        let descriptor = json!({"sessionId":"s","documentId":7});
        let status = json!({"sessionId":"s","documentId":7,"protocolVersion":1,"enabled":true,"permissions":"request-approval"});
        let output = [
            json!({"id":1,"result":{"serverInfo":{"version":env!("CARGO_PKG_VERSION")}}}),
            json!({"id":2,"result":{"tools":[{"name":"check_session"}]}}),
            json!({"id":3,"result":{"isError":false,"content":[{"type":"text","text":status.to_string()}]}}),
        ]
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
        assert_eq!(validate_check_output(&output, &descriptor).unwrap(), status);
        assert!(validate_check_output(&output, &json!({"sessionId":"different","documentId":7})).is_err());
        assert!(validate_check_output(&output, &json!({"sessionId":"s","documentId":8})).is_err());
        assert!(validate_check_output(&output.replace(env!("CARGO_PKG_VERSION"), "wrong-version"), &descriptor).is_err());
        assert!(validate_check_output("unexpected launcher output", &descriptor).is_err());
    }

    #[test]
    fn missing_bridge_does_not_produce_a_fake_configuration() {
        let mut descriptor = json!({"sessionId":"session-1"});
        decorate_with(&mut descriptor, &unavailable("Reinstall matching CLI"));
        assert_eq!(descriptor["setupReady"], false);
        assert_eq!(descriptor["mcpConfig"], "");
        assert!(!descriptor["invitation"].as_str().unwrap().contains("Reinstall matching CLI"));
        assert!(config(Path::new("relative/wordcraft"), &["--mcp-live"]).is_err());
    }

    #[test]
    fn windows_cli_must_match_this_version_and_support_live_mode() {
        let version = format!("wordcraft-cli {}", env!("CARGO_PKG_VERSION"));
        assert!(validate_cli_output(&format!("{version}\n"), "mcp --live").is_ok());
        assert!(validate_cli_output(&format!("{version} (build-sha)\n"), "mcp --live").is_ok());
        assert!(validate_cli_output(&format!("{version}0\n"), "mcp --live").is_err());
        assert!(validate_cli_output(&version, "mcp --connect HOST:PORT").is_err());
        assert!(validate_cli_output("wordcraft-cli 0.0.0", "mcp --live").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn non_unicode_executable_path_returns_actionable_status() {
        use std::os::unix::ffi::OsStringExt;

        let path = std::env::temp_dir().join(std::ffi::OsString::from_vec(vec![0xff]));
        let setup = ready(&path, &["--mcp-live"], "Ready");
        assert!(!setup.ready);
        assert!(setup.config.is_empty());
        assert!(setup.status.contains("Unicode path"));
    }
}

//! Persistent MCP profile for explicitly enabled documents in running desktop windows.
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::backend::{Backend, Remote};
use serde_json::{Value, json};

/// Canonical instructions, returned both at MCP initialization and by get_usage_guide.
pub const USAGE_GUIDE: &str = "Work only in the intended, already open WordCraft document. Ask the owner to enable connection requests in Agents once, then choose the intended sessionId from list_sessions or an optional invitation. Discovery is off until the owner opts in; this preference is separate from attachment approval. Use check_session for a non-editing availability check. connect_session requests access as your agentName; WordCraft must ask its user to approve this client. On approval_required or approval_pending, wait for the user and retry get_session_status. Never fall back to headless mode or change client identity to bypass a refusal. On approval_denied, stop. Approved status confirms sessionId, documentId and revisionAuthor. Read inspect_document (including unsaved work), then explicitly select the intended text/insertion point before editing. Use select_text or execute select.text, select.range, select.owner, caret.set, caret.docStart or caret.docEnd; relative movement cannot establish a safe target. When the target is invalidated, use collapsed absolute caret placement (extend:false) or an explicit range; extend:true retains a stale anchor and does not unlock editing. inspect_document includes ownerSelection; select.owner copies ordinary owner selection without moving their caret. After any owner revision change, reconnect, revision_conflict or selection_invalidated, read again and explicitly reselect. Find permitted command IDs and parameters with list_commands; edit with execute, batch or type_text and verify by read/render. Edits are attributed to the approved author and tracked. File operations, UI clicks, replacing documents and accepting/disabling changes are unavailable; the user saves. Never replay outcome_unknown: read and reassess because the operation may already have happened. A successful result may include agentWarnings with owner_selection_relocated: the edit succeeded and the owner caret was moved safely; inspect the document and do not retry that operation. list_sessions reports truncated/sweepLimited when discovery was bounded; use the intended document invitation for direct lookup. Multi-operation tools can partly succeed; check their result. disconnect_session releases this client. Access can be revoked at any time.";

static NEXT_CLIENT: AtomicU64 = AtomicU64::new(1);

/// Logical MCP client identity, distinct from its editable display/author name.
/// This is not an authentication credential; the local control key provides authorization.
fn new_client_id() -> String {
    let serial = NEXT_CLIENT.fetch_add(1, Ordering::Relaxed);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        format!("live-{:x}-{nanos:x}-{serial:x}", std::process::id())
    }
    #[cfg(target_arch = "wasm32")]
    {
        format!("live-wasm-{serial:x}")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Target {
    Tcp(String),
    #[cfg(unix)]
    Unix(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Descriptor {
    session_id: String,
    document_id: u64,
    title: String,
    target: Target,
    version: String,
    protocol_version: Option<u64>,
}

impl Descriptor {
    fn parse(v: &Value) -> Result<Self, String> {
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned).ok_or_else(|| format!("missing `{k}` in session descriptor"));
        let target = match v.get("transport").and_then(Value::as_str).unwrap_or("tcp") {
            "tcp" => {
                if v.get("authScheme").and_then(Value::as_str) != Some("hmac-sha256-v1") {
                    return Err("incompatible_authentication: update WordCraft and its MCP bridge, then re-enable connection requests".into());
                }
                let address = text("address")?;
                let addr = address.parse::<std::net::SocketAddr>().map_err(|_| "session address must be a numeric loopback socket address")?;
                if !addr.ip().to_canonical().is_loopback() {
                    return Err("session address must be loopback".into());
                }
                Target::Tcp(address)
            }
            #[cfg(unix)]
            "unix" => {
                let path = PathBuf::from(text("socketPath")?);
                if !path.is_absolute() {
                    return Err("live Unix socket path must be absolute".into());
                }
                Target::Unix(path)
            }
            _ => return Err("unsupported live transport on this platform".into()),
        };
        Ok(Self {
            session_id: text("sessionId")?,
            document_id: v.get("documentId").and_then(Value::as_u64).ok_or("missing documentId")?,
            title: text("title")?,
            target,
            version: text("version")?,
            protocol_version: v.get("protocolVersion").and_then(Value::as_u64),
        })
    }
    fn public(&self) -> Value {
        json!({"sessionId": self.session_id, "documentId": self.document_id, "title": self.title, "version": self.version, "protocolVersion": self.protocol_version})
    }
    fn connect(&self, settings: &Path) -> Result<Remote, String> {
        self.open(settings).map_err(|e| format!("live window unavailable: {e}"))
    }
    fn open(&self, settings: &Path) -> std::io::Result<Remote> {
        self.open_with_budget(settings, None)
    }
    fn open_with_budget(&self, settings: &Path, budget: Option<std::time::Duration>) -> std::io::Result<Remote> {
        match &self.target {
            Target::Tcp(address) => {
                let settings = settings.to_path_buf();
                let key = Box::new(move |addr: &str| {
                    let port = addr.parse::<std::net::SocketAddr>().map_err(|e| e.to_string())?.port();
                    // Never use an unrelated environment key for a discovered window.
                    let mut key = String::new();
                    std::fs::File::open(wordcraft_control_key::live_key_path(&settings, port))
                        .and_then(|file| file.take(129).read_to_string(&mut key))
                        .map_err(|e| format!("live control key unavailable: {e}"))?;
                    if key.len() > 128 {
                        return Err("live control key exceeds its size limit".into());
                    }
                    let key = key.trim().to_string();
                    if key.is_empty() { Err("live control key is empty".into()) } else { Ok(key) }
                });
                match budget {
                    Some(budget) => Remote::connect_live_tcp_with_budget(address, key, &self.session_id, self.document_id, budget),
                    None => Remote::connect_live_tcp(address, key, &self.session_id, self.document_id),
                }
            }
            #[cfg(unix)]
            Target::Unix(path) => {
                use std::os::unix::fs::{FileTypeExt, PermissionsExt};
                let metadata = std::fs::symlink_metadata(path)?;
                let parent = path.parent().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "live Unix socket has no parent"))?;
                let directory = std::fs::symlink_metadata(parent)?;
                if !metadata.file_type().is_socket()
                    || metadata.permissions().mode() & 0o077 != 0
                    || !directory.is_dir()
                    || directory.permissions().mode() & 0o077 != 0
                {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "live Unix socket and its directory must be private to the current OS user",
                    ));
                }
                match budget {
                    Some(budget) => Remote::connect_unix_with_budget(path, budget),
                    None => Remote::connect_unix(path),
                }
            }
        }
    }
}

struct Selected {
    descriptor: Descriptor,
    agent_name: String,
    client_id: String,
    remote: Remote,
    approved: bool,
}
impl Selected {
    fn envelope(&self) -> Value {
        json!({"sessionId": self.descriptor.session_id, "documentId": self.descriptor.document_id, "agentName": self.agent_name, "clientId": self.client_id})
    }
    fn status(&mut self) -> Result<Value, String> {
        self.approved = false;
        let status = self.remote.call("agent.status", self.envelope())?;
        if status.get("approved").and_then(Value::as_bool) != Some(true) {
            return Err("approval_required: approve this MCP client in WordCraft before using document tools".into());
        }
        if status.get("protocolVersion").and_then(Value::as_u64) != Some(1) {
            return Err(format!(
                "incompatible_protocol: live WordCraft protocol version 1 is required; window product version {}. Update WordCraft and the MCP bridge.",
                self.descriptor.version
            ));
        }
        if status.get("sessionId").and_then(Value::as_str) != Some(self.descriptor.session_id.as_str())
            || status.get("documentId").and_then(Value::as_u64) != Some(self.descriptor.document_id)
            || status.get("enabled").and_then(Value::as_bool) != Some(true)
            || status.get("agentName").and_then(Value::as_str) != Some(self.agent_name.as_str())
            || status.get("clientId").and_then(Value::as_str) != Some(self.client_id.as_str())
            || status.get("revision").and_then(Value::as_u64).is_none()
            || status.get("permissions").and_then(Value::as_str) != Some("tracked-editing")
        {
            return Err("session_mismatch: WordCraft did not confirm the requested live document and agent".into());
        }
        self.approved = true;
        Ok(status)
    }
}

/// Starts successfully without any open WordCraft window; connection is always explicit.
pub struct Live {
    settings: Vec<PathBuf>,
    client_id: String,
    selected: Option<Selected>,
}
impl Live {
    pub fn new() -> Result<Self, String> {
        let settings = wordcraft_control_key::discovery_settings_dirs();
        if settings.is_empty() {
            return Err("cannot locate WordCraft's settings directory".into());
        }
        Ok(Self { settings, client_id: new_client_id(), selected: None })
    }
    pub fn with_settings(settings: PathBuf) -> Self {
        Self { settings: vec![settings], client_id: new_client_id(), selected: None }
    }

    fn descriptor(&self, session_id: &str) -> Result<(Descriptor, PathBuf), String> {
        let mut invalid = None;
        for settings in &self.settings {
            let path = wordcraft_control_key::session_path(settings, session_id).ok_or("invalid sessionId")?;
            match read_descriptor(&path) {
                Ok(d) => {
                    if d.session_id != session_id {
                        return Err("session descriptor identity mismatch".into());
                    }
                    return Ok((d, settings.clone()));
                }
                Err(error) if path.exists() => invalid = Some(error),
                Err(_) => {}
            }
        }
        Err(invalid.unwrap_or_else(|| "live session unavailable: enable access in the document's Agents options".into()))
    }
    fn list(&self) -> Result<Value, String> {
        self.list_with_probe(|descriptor, settings, budget| descriptor.open_with_budget(settings, Some(budget)).map(drop))
    }

    // Keep discovery policy independent of OS connection-refusal timing. Tests can supply
    // a definite transport result while real discovery still uses authenticated endpoints.
    fn list_with_probe(&self, mut probe: impl FnMut(&Descriptor, &Path, std::time::Duration) -> std::io::Result<()>) -> Result<Value, String> {
        let mut sessions = Vec::new();
        let mut candidates = Vec::new();
        let mut truncated = false;
        let mut scanned = 0usize;
        const MAX_SCAN: usize = 4096;
        for settings in &self.settings {
            let dir = wordcraft_control_key::sessions_dir(settings);
            let entries = match std::fs::read_dir(&dir) {
                Ok(v) => v,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(format!("cannot discover live sessions: {e}")),
            };
            for entry in entries.flatten() {
                scanned = scanned.saturating_add(1);
                if scanned > MAX_SCAN {
                    truncated = true;
                    break;
                }
                let p = entry.path();
                if p.extension().and_then(|s| s.to_str()) != Some("json") {
                    continue;
                }
                if let Ok(d) = read_descriptor(&p)
                    && wordcraft_control_key::session_path(settings, &d.session_id).as_ref() == Some(&p)
                {
                    let modified = entry.metadata().and_then(|m| m.modified()).ok();
                    candidates.push((modified, p, d, settings));
                }
            }
        }
        // Fresh descriptors come first. Garbage and definitely dead windows consume scan
        // work, but never the 128 returned-session slots. Direct invitation lookup has no cap.
        candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        let deadline = std::time::Instant::now().checked_add(std::time::Duration::from_secs(2));
        let mut sweep_limited = false;
        for (_, path, descriptor, settings) in candidates {
            if sessions.len() >= 128 {
                truncated = true;
                break;
            }
            let available =
                if let Some(remaining) = deadline.and_then(|t| t.checked_duration_since(std::time::Instant::now())).filter(|t| !t.is_zero()) {
                    match probe(&descriptor, settings, remaining) {
                        Ok(_) => "reachable",
                        Err(error) if matches!(error.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused) => {
                            // Delete only this unchanged canonical descriptor. A key/socket at
                            // its old endpoint may now belong to another window; never remove it.
                            if read_descriptor(&path).as_ref() == Ok(&descriptor) {
                                let _ = std::fs::remove_file(&path);
                            }
                            continue;
                        }
                        Err(error) => {
                            sweep_limited |= matches!(error.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock);
                            "unverified" // Timeout/busy/permission failure is not proof of death.
                        }
                    }
                } else {
                    sweep_limited = true;
                    "unverified"
                };
            let mut public = descriptor.public();
            public["availability"] = json!(available);
            sessions.push(public);
        }
        sessions.sort_by(|a, b| a["sessionId"].as_str().cmp(&b["sessionId"].as_str()));
        sessions.dedup_by(|a, b| a["sessionId"] == b["sessionId"]);
        Ok(json!({"sessions": sessions,"truncated":truncated,"sweepLimited":sweep_limited,
            "nextStep": if truncated { Some("Some entries were omitted. Use the intended document's optional invitation, or disable old connection requests.") } else { None }}))
    }

    fn verify_descriptor(&self) -> Result<(), String> {
        let selected = self.selected.as_ref().ok_or("not_connected: list_sessions and connect_session first; no headless fallback")?;
        let (latest, _) = self
            .descriptor(&selected.descriptor.session_id)
            .map_err(|_| "session_revoked: access disabled or window closed; reconnect explicitly")?;
        if latest.document_id != selected.descriptor.document_id || latest.target != selected.descriptor.target {
            return Err("session_changed: the document changed; reconnect explicitly".into());
        }
        Ok(())
    }
    fn disconnect(&mut self) -> Result<Value, String> {
        if let Some(mut selected) = self.selected.take() {
            // Clear local attachment even when the window closed before acknowledging.
            selected.remote.set_timeout(std::time::Duration::from_secs(1));
            let _ = selected.remote.call("agent.disconnect", selected.envelope());
        }
        Ok(json!({"connected": false}))
    }
}
fn read_descriptor(path: &Path) -> Result<Descriptor, String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| format!("live session unavailable: {e}"))?;
    if !metadata.is_file() || metadata.len() > 16 * 1024 {
        return Err("invalid live session descriptor".into());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path).map_err(|e| e.to_string())?.take(16 * 1024 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() > 16 * 1024 {
        return Err("live session descriptor exceeds limit".into());
    }
    let v = serde_json::from_slice(&bytes).map_err(|e| format!("invalid live session descriptor: {e}"))?;
    Descriptor::parse(&v)
}
impl Backend for Live {
    fn is_live(&self) -> bool {
        true
    }
    fn has_ui(&self) -> bool {
        self.selected.as_ref().is_some_and(|selected| selected.approved)
    }
    fn describe(&self) -> String {
        "live document access (explicit connection required; no headless fallback)".into()
    }
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        match method {
            "agent.list_sessions" => self.list(),
            "agent.check_session" => {
                let session_id = params.get("sessionId").and_then(Value::as_str).ok_or("missing sessionId")?;
                let (descriptor, settings) = self.descriptor(session_id)?;
                let mut remote = descriptor.connect(&settings)?;
                remote.set_timeout(std::time::Duration::from_secs(3));
                let status = remote.call("agent.check", json!({"sessionId":descriptor.session_id,"documentId":descriptor.document_id}))?;
                if status.get("sessionId").and_then(Value::as_str) != Some(descriptor.session_id.as_str())
                    || status.get("documentId").and_then(Value::as_u64) != Some(descriptor.document_id)
                    || status.get("enabled").and_then(Value::as_bool) != Some(true)
                    || status.get("protocolVersion").and_then(Value::as_u64) != Some(1)
                    || status.get("permissions").and_then(Value::as_str) != Some("request-approval")
                {
                    return Err("session_mismatch: WordCraft did not confirm the invited live session".into());
                }
                Ok(status)
            }
            "agent.connect_session" => {
                let session_id = params.get("sessionId").and_then(Value::as_str).ok_or("missing sessionId")?;
                let agent_name = params
                    .get("agentName")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty() && s.len() <= 128 && s.chars().all(|c| c.is_ascii_alphanumeric() || " _.-@".contains(c)))
                    .ok_or("agentName must contain 1-128 ASCII letters, numbers, spaces, _, ., - or @")?;
                if let Some(selected) = self.selected.as_ref() {
                    if selected.descriptor.session_id != session_id || selected.agent_name != agent_name {
                        return Err("already_connected: disconnect_session before selecting another document or agent name".into());
                    }
                    self.verify_descriptor()?;
                    let selected = self.selected.as_mut().ok_or("not_connected")?;
                    let mut status = selected.status()?;
                    status["connected"] = json!(true);
                    return Ok(status);
                }
                let (descriptor, settings) = self.descriptor(session_id)?;
                let remote = descriptor.connect(&settings)?;
                let mut selected = Selected { descriptor, agent_name: agent_name.into(), client_id: self.client_id.clone(), remote, approved: false };
                let status = selected.status();
                match status {
                    Ok(mut status) => {
                        status["connected"] = json!(true);
                        self.selected = Some(selected);
                        Ok(status)
                    }
                    Err(error) if is_approval_error(&error) => {
                        self.selected = Some(selected);
                        Err(error)
                    }
                    Err(error) => {
                        selected.remote.set_timeout(std::time::Duration::from_secs(1));
                        let _ = selected.remote.call("agent.disconnect", selected.envelope());
                        Err(error)
                    }
                }
            }
            "agent.disconnect_session" => self.disconnect(),
            "agent.get_session_status" => {
                if self.selected.is_none() {
                    return Ok(json!({"connected": false}));
                }
                self.verify_descriptor()?;
                let selected = self.selected.as_mut().ok_or("not_connected")?;
                let mut status = selected.status()?;
                status["connected"] = json!(true);
                Ok(status)
            }
            _ => {
                self.verify_descriptor()?;
                let selected = self.selected.as_mut().ok_or("not_connected")?;
                // Revalidate the same explicit attachment before every operation. A new
                // transport may need registration; it must never inherit stale read/selection state.
                selected.status()?;
                let mut envelope = selected.envelope();
                envelope["method"] = json!(method);
                envelope["params"] = params;
                let reply = selected.remote.call("agent.call", envelope)?;
                if reply.get("revision").and_then(Value::as_u64).is_none() || reply.get("result").is_none() {
                    return Err("outcome_unknown: invalid live response, expected result and revision; request was not replayed. Read the document before any retry".into());
                }
                reply.get("result").cloned().ok_or_else(|| "outcome_unknown: live result missing".into())
            }
        }
    }
}

fn is_approval_error(error: &str) -> bool {
    ["approval_required", "approval_pending", "approval_denied"].iter().any(|code| error.starts_with(code))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);
    const TEST_KEY: &str = "abababababababababababababababababababababababababababababababab";
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("wc-live-test-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir_all(wordcraft_control_key::sessions_dir(&path)).unwrap();
            Self(path)
        }
        fn publish(&self, port: u16) {
            std::fs::write(wordcraft_control_key::live_key_path(&self.0, port), TEST_KEY).unwrap();
            std::fs::write(
                wordcraft_control_key::session_path(&self.0, "window-a").unwrap(),
                json!({"sessionId":"window-a", "documentId":7, "title":"Unsaved document", "address":format!("127.0.0.1:{port}"), "authScheme":"hmac-sha256-v1", "version":"0.6.1"})
                    .to_string(),
            )
            .unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn authenticated_fixture(stream: std::net::TcpStream) -> (BufReader<std::net::TcpStream>, std::net::TcpStream) {
        stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let challenge: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(challenge["method"], "auth.challenge");
        assert!(challenge.get("key").is_none());
        assert_eq!(challenge["params"]["sessionId"], "window-a");
        assert_eq!(challenge["params"]["documentId"], 7);
        let nonce = challenge["params"]["clientNonce"].as_str().unwrap();
        let server = wordcraft_control_key::fresh_nonce().unwrap();
        let proof = wordcraft_control_key::live_auth_proof(TEST_KEY, "server", "window-a", 7, nonce, &server).unwrap();
        writeln!(writer, "{}", json!({"id":0,"ok":true,"result":{"serverNonce":server,"proof":proof}})).unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        let prove: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(prove["method"], "auth.prove");
        assert!(prove.get("key").is_none());
        wordcraft_control_key::verify_live_auth_proof(TEST_KEY, "client", "window-a", 7, nonce, &server, prove["params"]["proof"].as_str().unwrap())
            .unwrap();
        writeln!(writer, "{}", json!({"id":0,"ok":true,"result":{"authenticated":true,"sessionId":"window-a","documentId":7}})).unwrap();
        (reader, writer)
    }

    fn fake_window(wrong_document: bool) -> (u16, std::sync::mpsc::Receiver<Value>) {
        fake_window_with(wrong_document, false, false)
    }
    fn fake_window_with(wrong_document: bool, incompatible: bool, malformed: bool) -> (u16, std::sync::mpsc::Receiver<Value>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let (reader, mut writer) = authenticated_fixture(stream);
            for line in reader.lines() {
                let Ok(line) = line else { break };
                let request: Value = serde_json::from_str(&line).unwrap();
                let _ = tx.send(request.clone());
                let result = if request["method"] == "agent.status" {
                    json!({"sessionId":"window-a", "documentId":if wrong_document {8} else {7}, "enabled":true,"approved":true,"protocolVersion":if incompatible {2} else {1},"agentName":"Test agent","clientId":request["params"]["clientId"],"title":"Unsaved document","revision":9,"permissions":"tracked-editing"})
                } else if malformed {
                    json!({"result":{"text":"Possibly applied"}})
                } else {
                    json!({"result":{"text":"Live unsaved changes"},"revision":9})
                };
                if writeln!(writer, "{}", json!({"id":request["id"],"ok":true,"result":result})).is_err() {
                    break;
                }
            }
        });
        (port, rx)
    }
    #[test]
    fn waiting_profile_initializes_without_window_and_has_no_headless_fallback() {
        let fixture = Fixture::new();
        let live = Live::with_settings(fixture.0.clone());
        let mut server = crate::Server::new(Box::new(live));
        let init = server.handle(json!({"jsonrpc":"2.0", "id":1,"method":"initialize","params":{}})).unwrap();
        assert!(init["result"]["instructions"].as_str().unwrap().contains("already open WordCraft"));
        let list = crate::tools::call_tool(server.backend(), "list_sessions", &json!({}));
        assert!(!list.is_error);
        assert!(list.content[0]["text"].as_str().unwrap().contains("[]"));
        for name in ["inspect_document", "type_text", "new_document", "open_document", "save_document", "click"] {
            assert!(crate::tools::call_tool(server.backend(), name, &json!({"text":"Do not create a new document"})).is_error, "{name}");
        }
        let guide = crate::tools::call_tool(server.backend(), "get_usage_guide", &json!({}));
        assert_eq!(guide.content[0]["text"], USAGE_GUIDE);
    }
    #[test]
    fn explicit_connection_wraps_live_methods_and_revocation_blocks_calls() {
        let fixture = Fixture::new();
        let (port, seen) = fake_window(false);
        fixture.publish(port);
        let mut live = Live::with_settings(fixture.0.clone());
        let status = live.call("agent.connect_session", json!({"sessionId":"window-a", "agentName":"Test agent"})).unwrap();
        assert_eq!(status["documentId"], 7);
        let request = seen.recv().unwrap();
        assert_eq!(request["method"], "agent.status");
        assert_eq!(request["params"]["documentId"], 7);
        assert_eq!(request["params"]["clientId"], live.client_id);
        assert_eq!(live.call("document.inspect", json!({})).unwrap()["text"], "Live unsaved changes");
        assert_eq!(seen.recv().unwrap()["method"], "agent.status");
        let request = seen.recv().unwrap();
        assert_eq!(request["method"], "agent.call");
        assert_eq!(request["params"]["method"], "document.inspect");
        assert_eq!(request["params"]["sessionId"], "window-a");
        assert_eq!(request["params"]["clientId"], live.client_id);
        std::fs::remove_file(wordcraft_control_key::session_path(&fixture.0, "window-a").unwrap()).unwrap();
        assert!(live.call("engine.execute", json!({"command":"text.insert"})).unwrap_err().contains("session_revoked"));
        assert!(seen.try_recv().is_err());
    }
    #[test]
    fn wrong_target_is_never_reported_connected() {
        let fixture = Fixture::new();
        let (port, _) = fake_window(true);
        fixture.publish(port);
        let mut live = Live::with_settings(fixture.0.clone());
        assert!(
            live.call("agent.connect_session", json!({"sessionId":"window-a", "agentName":"Test agent"})).unwrap_err().contains("session_mismatch")
        );
        assert_eq!(live.call("agent.get_session_status", json!({})).unwrap()["connected"], false);
        assert!(live.call("document.inspect", json!({})).unwrap_err().contains("not_connected"));
    }
    #[test]
    fn incompatible_live_protocol_is_rejected() {
        let fixture = Fixture::new();
        let (port, _) = fake_window_with(false, true, false);
        fixture.publish(port);
        let mut live = Live::with_settings(fixture.0.clone());
        let error = live.call("agent.connect_session", json!({"sessionId":"window-a", "agentName":"Test agent"})).unwrap_err();
        assert!(error.contains("incompatible_protocol"), "{error}");
        assert_eq!(live.call("agent.get_session_status", json!({})).unwrap()["connected"], false);
    }
    #[test]
    fn malformed_mutation_response_reports_unknown_without_replay() {
        let fixture = Fixture::new();
        let (port, seen) = fake_window_with(false, false, true);
        fixture.publish(port);
        let mut live = Live::with_settings(fixture.0.clone());
        live.call("agent.connect_session", json!({"sessionId":"window-a", "agentName":"Test agent"})).unwrap();
        assert_eq!(seen.recv().unwrap()["method"], "agent.status");
        let error = live.call("engine.execute", json!({"command":"text.insert", "params":{"text":"once"}})).unwrap_err();
        assert!(error.contains("outcome_unknown"), "{error}");
        assert_eq!(seen.recv().unwrap()["method"], "agent.status");
        assert_eq!(seen.recv().unwrap()["method"], "agent.call");
        assert!(seen.try_recv().is_err());
    }

    fn approval_window() -> (u16, std::sync::mpsc::Receiver<Value>, std::sync::Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = std::sync::mpsc::channel();
        let approval = std::sync::Arc::new(AtomicUsize::new(0));
        let state = approval.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (tx, state) = (tx.clone(), state.clone());
                std::thread::spawn(move || {
                    let (reader, mut writer) = authenticated_fixture(stream);
                    for line in reader.lines() {
                        let Ok(line) = line else { break };
                        let request: Value = serde_json::from_str(&line).unwrap();
                        let _ = tx.send(request.clone());
                        let reply = match request["method"].as_str().unwrap() {
                            "agent.check" => {
                                json!({"ok":true,"result":{"sessionId":"window-a","documentId":7,"enabled":true,"protocolVersion":1,"permissions":"request-approval","title":"Unsaved"}})
                            }
                            "agent.status" if state.load(Ordering::Acquire) != 1 => {
                                json!({"ok":false,"error":if state.load(Ordering::Acquire)==2 {"approval_denied: user refused"} else {"approval_required: approve this client in WordCraft"}})
                            }
                            "agent.status" => {
                                json!({"ok":true,"result":{"sessionId":"window-a","documentId":7,"enabled":true,"approved":true,"protocolVersion":1,"agentName":request["params"]["agentName"],"clientId":request["params"]["clientId"],"revision":1,"permissions":"tracked-editing"}})
                            }
                            "agent.call" => json!({"ok":true,"result":{"result":{"text":"approved live document"},"revision":1}}),
                            _ => json!({"ok":true,"result":{"connected":false}}),
                        };
                        if writeln!(writer, "{}", json!({"id":request["id"],"ok":reply["ok"],"result":reply["result"],"error":reply["error"]}))
                            .is_err()
                        {
                            break;
                        }
                    }
                });
            }
        });
        (port, rx, approval)
    }

    #[test]
    fn approval_pending_keeps_target_but_never_forwards_document_calls() {
        let fixture = Fixture::new();
        let (port, seen, approval) = approval_window();
        fixture.publish(port);
        let mut live = Live::with_settings(fixture.0.clone());
        let connect = json!({"sessionId":"window-a","agentName":"Test agent"});
        assert!(live.call("agent.connect_session", connect.clone()).unwrap_err().starts_with("approval_required"));
        assert!(live.selected.is_some());
        assert!(!live.has_ui());
        assert!(live.call("document.inspect", json!({})).unwrap_err().starts_with("approval_required"));
        assert!(live.call("agent.get_session_status", json!({})).unwrap_err().starts_with("approval_required"));
        let requests: Vec<_> = seen.try_iter().collect();
        assert_eq!(requests.len(), 3);
        assert!(requests.iter().all(|request| request["method"] == "agent.status" && request["params"]["clientId"] == live.client_id));
        approval.store(1, Ordering::Release);
        assert_eq!(live.call("agent.connect_session", connect).unwrap()["connected"], true);
        assert!(live.has_ui());
        assert_eq!(live.call("document.inspect", json!({})).unwrap()["text"], "approved live document");
        approval.store(2, Ordering::Release);
        assert!(live.call("document.inspect", json!({})).unwrap_err().starts_with("approval_denied"));
        assert!(live.selected.is_some());
        assert!(!live.has_ui());
    }

    #[test]
    fn availability_check_never_registers_or_requests_approval() {
        let fixture = Fixture::new();
        let (port, seen, approval) = approval_window();
        fixture.publish(port);
        let mut live = Live::with_settings(fixture.0.clone());
        let checked = live.call("agent.check_session", json!({"sessionId":"window-a"})).unwrap();
        assert_eq!(checked["permissions"], "request-approval");
        assert!(live.selected.is_none());
        assert_eq!(approval.load(Ordering::Acquire), 0);
        let request = seen.recv().unwrap();
        assert_eq!(request["method"], "agent.check");
        assert!(request["params"].get("agentName").is_none());
        assert!(request["params"].get("clientId").is_none());
        assert_eq!(live.call("agent.get_session_status", json!({})).unwrap()["connected"], false);
    }

    #[cfg(unix)]
    #[test]
    fn unix_live_handshake_sends_no_key_and_lost_mutation_reply_is_not_replayed() {
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::net::UnixListener;
        let fixture = Fixture::new();
        std::fs::set_permissions(&fixture.0, std::fs::Permissions::from_mode(0o700)).unwrap();
        let socket_path = fixture.0.join("window.sock");
        let listener = UnixListener::bind(&socket_path).unwrap();
        std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::write(wordcraft_control_key::session_path(&fixture.0,"window-a").unwrap(), json!({"sessionId":"window-a","documentId":7,"title":"Unix document","transport":"unix","socketPath":socket_path,"protocolVersion":1,"version":"0.6.1"}).to_string()).unwrap();
        let (tx, seen) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut writer = stream.try_clone().unwrap();
            for line in BufReader::new(stream).lines() {
                let request: Value = serde_json::from_str(&line.unwrap()).unwrap();
                assert!(request.get("key").is_none());
                tx.send(request.clone()).unwrap();
                if request["method"] == "agent.call" {
                    break;
                }
                let result = json!({"sessionId":"window-a","documentId":7,"enabled":true,"approved":true,"protocolVersion":1,"agentName":request["params"]["agentName"],"clientId":request["params"]["clientId"],"revision":1,"permissions":"tracked-editing"});
                writeln!(writer, "{}", json!({"id":request["id"],"ok":true,"result":result})).unwrap();
            }
        });
        let mut live = Live::with_settings(fixture.0.clone());
        live.call("agent.connect_session", json!({"sessionId":"window-a","agentName":"Test agent"})).unwrap();
        let error = live.call("engine.execute", json!({"command":"text.insert","params":{"text":"once"}})).unwrap_err();
        assert!(error.contains("outcome_unknown"), "{error}");
        worker.join().unwrap();
        let requests: Vec<_> = seen.try_iter().collect();
        assert_eq!(requests.iter().filter(|request| request["method"] == "agent.call").count(), 1);
    }

    #[test]
    fn different_mcp_instances_have_distinct_stable_client_ids() {
        let fixture = Fixture::new();
        let first = Live::with_settings(fixture.0.clone());
        let second = Live::with_settings(fixture.0.clone());
        assert_ne!(first.client_id, second.client_id);
        assert!(first.client_id.len() <= 128);
        assert!(first.client_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    }

    #[test]
    fn discovery_filters_garbage_and_sweeps_dead_records_before_return_limit() {
        let fixture = Fixture::new();
        let (port, seen, _) = approval_window();
        fixture.publish(port);
        let dead_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let dead_port = dead_listener.local_addr().unwrap().port();
        drop(dead_listener);
        let key_path = wordcraft_control_key::live_key_path(&fixture.0, dead_port);
        std::fs::write(&key_path, "preserve-a-new-windows-key").unwrap();
        for index in 0..256 {
            std::fs::write(wordcraft_control_key::sessions_dir(&fixture.0).join(format!("noise-{index}.tmp")), "leftover").unwrap();
        }
        for index in 0..160 {
            let session = format!("dead-{index}");
            std::fs::write(
                wordcraft_control_key::session_path(&fixture.0, &session).unwrap(),
                json!({
                    "sessionId":session,"documentId":7,"title":"Dead","transport":"tcp","address":format!("127.0.0.1:{dead_port}"),
                    "authScheme":"hmac-sha256-v1","version":"0.6.1","protocolVersion":1
                })
                .to_string(),
            )
            .unwrap();
        }
        let live = Live::with_settings(fixture.0.clone());
        let dead_target = Target::Tcp(format!("127.0.0.1:{dead_port}"));
        // A closed loopback port is not guaranteed to refuse inside an 800 ms connect
        // budget on Windows. Timeout is correctly preserved as unverified, so use a
        // deterministic refusal to test sweeping rather than assuming OS SYN timing.
        // The live target still performs the real TCP challenge/proof handshake.
        let result = live
            .list_with_probe(|descriptor, settings, budget| {
                if descriptor.target == dead_target {
                    Err(std::io::Error::from(std::io::ErrorKind::ConnectionRefused))
                } else {
                    descriptor.open_with_budget(settings, Some(budget)).map(drop)
                }
            })
            .unwrap();
        assert_eq!(result["sessions"].as_array().unwrap().len(), 1);
        assert_eq!(result["sessions"][0]["sessionId"], "window-a");
        assert_eq!(result["sessions"][0]["availability"], "reachable");
        assert_eq!(result["truncated"], false);
        assert!(seen.try_recv().is_err(), "discovery authenticates the host without agent registration");
        assert_eq!(std::fs::read_to_string(key_path).unwrap(), "preserve-a-new-windows-key");
        assert!(!wordcraft_control_key::session_path(&fixture.0, "dead-0").unwrap().exists());
        assert!(!wordcraft_control_key::session_path(&fixture.0, "dead-159").unwrap().exists());
    }

    #[test]
    fn discovery_preserves_a_timed_out_target_and_its_key() {
        let fixture = Fixture::new();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        fixture.publish(port);
        let path = wordcraft_control_key::session_path(&fixture.0, "window-a").unwrap();
        let key_path = wordcraft_control_key::live_key_path(&fixture.0, port);
        let before = std::fs::read(&path).unwrap();
        let live = Live::with_settings(fixture.0.clone());
        let mut probes = 0;
        let result = live
            .list_with_probe(|descriptor, _, budget| {
                probes += 1;
                assert_eq!(descriptor.target, Target::Tcp(format!("127.0.0.1:{port}")));
                assert!(!budget.is_zero() && budget <= std::time::Duration::from_secs(2));
                Err(std::io::Error::from(std::io::ErrorKind::TimedOut))
            })
            .unwrap();
        assert_eq!(probes, 1);
        assert_eq!(result["sessions"].as_array().unwrap().len(), 1);
        assert_eq!(result["sessions"][0]["sessionId"], "window-a");
        assert_eq!(result["sessions"][0]["availability"], "unverified");
        assert_eq!(result["sweepLimited"], true);
        assert_eq!(result["truncated"], false);
        assert_eq!(std::fs::read(path).unwrap(), before);
        assert_eq!(std::fs::read_to_string(key_path).unwrap(), TEST_KEY);
    }

    #[test]
    fn native_closed_tcp_endpoint_sweeps_only_on_definite_transport_failure() {
        let fixture = Fixture::new();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        fixture.publish(port);
        let path = wordcraft_control_key::session_path(&fixture.0, "window-a").unwrap();
        let key_path = wordcraft_control_key::live_key_path(&fixture.0, port);
        let descriptor_before = std::fs::read(&path).unwrap();
        let key_before = std::fs::read(&key_path).unwrap();
        let live = Live::with_settings(fixture.0.clone());
        let mut observed = None;
        let mut probes = 0;
        let started = std::time::Instant::now();
        let result = live
            .list_with_probe(|descriptor, settings, budget| {
                probes += 1;
                // Observe the one real native probe without changing its result, retrying it,
                // or assuming Windows reports refusal before the absolute deadline.
                let outcome = descriptor.open_with_budget(settings, Some(budget));
                observed = outcome.as_ref().err().map(|error| (error.kind(), error.to_string()));
                outcome.map(drop)
            })
            .unwrap();
        assert_eq!(probes, 1, "native outcome must come from exactly one real transport probe");
        assert!(started.elapsed() < std::time::Duration::from_millis(2600), "native probe {observed:?} exceeded discovery deadline: {result}");
        let (kind, detail) = observed.expect("a genuinely closed native TCP endpoint must not authenticate successfully");
        match kind {
            std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound => {
                assert!(result["sessions"].as_array().unwrap().is_empty(), "observed {kind:?}: {detail}; {result}");
                assert_eq!(result["sweepLimited"], false);
                assert!(!path.exists(), "only an observed definite failure permits descriptor cleanup");
            }
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
                assert_eq!(result["sessions"].as_array().unwrap().len(), 1, "observed {kind:?}: {detail}; {result}");
                assert_eq!(result["sessions"][0]["sessionId"], "window-a");
                assert_eq!(result["sessions"][0]["availability"], "unverified");
                assert_eq!(result["sweepLimited"], true);
                assert_eq!(std::fs::read(&path).unwrap(), descriptor_before, "a native timeout must preserve the exact descriptor bytes");
            }
            _ => panic!("unexpected native closed-endpoint error {kind:?}: {detail}; discovery result: {result}"),
        }
        assert_eq!(result["truncated"], false);
        assert_eq!(std::fs::read(key_path).unwrap(), key_before, "discovery must never delete or modify an endpoint key");
    }

    #[test]
    fn newest_live_target_stays_visible_when_a_later_probe_exhausts_budget() {
        let fixture = Fixture::new();
        let (live_port, seen, _) = approval_window();
        fixture.publish(live_port);
        let slow = TcpListener::bind("127.0.0.1:0").unwrap();
        let slow_port = slow.local_addr().unwrap().port();
        std::fs::write(wordcraft_control_key::live_key_path(&fixture.0, slow_port), TEST_KEY).unwrap();
        for (session, order) in [("slow", 2), ("after-budget", 1)] {
            let path = wordcraft_control_key::session_path(&fixture.0, session).unwrap();
            std::fs::write(
                &path,
                json!({"sessionId":session,"documentId":7,"title":"Older request","transport":"tcp",
                "address":format!("127.0.0.1:{slow_port}"),"authScheme":"hmac-sha256-v1","version":"0.6.1","protocolVersion":1})
                .to_string(),
            )
            .unwrap();
            std::fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(order))
                .unwrap();
        }
        let live_path = wordcraft_control_key::session_path(&fixture.0, "window-a").unwrap();
        std::fs::File::options()
            .write(true)
            .open(live_path)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(3))
            .unwrap();
        let worker = std::thread::spawn(move || {
            let (stream, _) = slow.accept().unwrap();
            // Accepting a connection without returning a proof is not evidence of death.
            std::thread::sleep(std::time::Duration::from_millis(2400));
            drop(stream);
        });
        let mut live = Live::with_settings(fixture.0.clone());
        let started = std::time::Instant::now();
        let result = live.call("agent.list_sessions", json!({})).unwrap();
        assert!(started.elapsed() < std::time::Duration::from_millis(2600), "{result}");
        let sessions = result["sessions"].as_array().unwrap();
        assert_eq!(sessions.len(), 3);
        assert_eq!(sessions.iter().find(|d| d["sessionId"] == "window-a").unwrap()["availability"], "reachable");
        assert_eq!(sessions.iter().find(|d| d["sessionId"] == "slow").unwrap()["availability"], "unverified");
        assert_eq!(sessions.iter().find(|d| d["sessionId"] == "after-budget").unwrap()["availability"], "unverified");
        assert_eq!(result["sweepLimited"], true);
        assert!(seen.try_recv().is_err(), "discovery must not register an agent");
        let (direct, _) = live.descriptor("window-a").unwrap();
        assert_eq!(direct.target, Target::Tcp(format!("127.0.0.1:{live_port}")));
        for session in ["slow", "after-budget"] {
            assert!(wordcraft_control_key::session_path(&fixture.0, session).unwrap().exists());
        }
        worker.join().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn missing_unix_endpoint_sweeps_only_its_descriptor() {
        let fixture = Fixture::new();
        let descriptor = wordcraft_control_key::session_path(&fixture.0, "gone").unwrap();
        let socket = fixture.0.join("gone.sock");
        std::fs::write(
            &descriptor,
            json!({"sessionId":"gone","documentId":7,"title":"Gone","transport":"unix","socketPath":socket,
            "version":"0.6.1","protocolVersion":1})
            .to_string(),
        )
        .unwrap();
        let mut live = Live::with_settings(fixture.0.clone());
        assert_eq!(live.call("agent.list_sessions", json!({})).unwrap()["sessions"], json!([]));
        assert!(!descriptor.exists());
        assert!(fixture.0.exists());
    }

    #[test]
    fn discovery_deadline_preserves_a_slow_or_busy_descriptor() {
        let fixture = Fixture::new();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        fixture.publish(port);
        let worker = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut challenge = String::new();
            reader.read_line(&mut challenge).unwrap();
            let mut writer = stream;
            for _ in 0..30 {
                if writer.write_all(b" ").is_err() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        });
        let started = std::time::Instant::now();
        let mut live = Live::with_settings(fixture.0.clone());
        let result = live.call("agent.list_sessions", json!({})).unwrap();
        assert!(started.elapsed() < std::time::Duration::from_millis(2600), "network probe shares the sweep's absolute deadline");
        assert_eq!(result["sessions"][0]["availability"], "unverified");
        assert_eq!(result["sweepLimited"], true);
        assert!(wordcraft_control_key::session_path(&fixture.0, "window-a").unwrap().exists());
        worker.join().unwrap();
    }

    #[test]
    fn old_live_tcp_authentication_is_never_used_as_a_fallback() {
        let fixture = Fixture::new();
        let (port, seen) = fake_window(false);
        fixture.publish(port);
        let path = wordcraft_control_key::session_path(&fixture.0, "window-a").unwrap();
        let mut descriptor: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        descriptor.as_object_mut().unwrap().remove("authScheme");
        std::fs::write(path, descriptor.to_string()).unwrap();
        let mut live = Live::with_settings(fixture.0.clone());
        assert!(
            live.call("agent.connect_session", json!({"sessionId":"window-a","agentName":"Test agent"}))
                .unwrap_err()
                .starts_with("incompatible_authentication")
        );
        assert!(seen.try_recv().is_err());
    }

    #[test]
    fn agent_name_validation_matches_live_window_contract() {
        let fixture = Fixture::new();
        let mut live = Live::with_settings(fixture.0.clone());
        let too_long = "x".repeat(129);
        for name in ["   ", "Агент", "Agent\nName", "Agent/Name", &too_long] {
            let error = live.call("agent.connect_session", json!({"sessionId":"window-a", "agentName":name})).unwrap_err();
            assert!(error.contains("agentName"), "{error}");
        }
        let error = live.call("agent.connect_session", json!({"sessionId":"window-a", "agentName":"  Test.Agent-1_@  "})).unwrap_err();
        assert!(error.contains("session unavailable"), "valid trimmed ASCII name should reach discovery: {error}");
    }

    #[test]
    fn discovery_rejects_remote_addresses_and_unsafe_identifiers() {
        let fixture = Fixture::new();
        let mut live = Live::with_settings(fixture.0.clone());
        assert!(live.call("agent.connect_session", json!({"sessionId":"../other", "agentName":"Test"})).is_err());
        let descriptor = json!({"sessionId":"window-a", "documentId":7,"title":"Wrong", "address":"192.0.2.1:7981","version":"0.6.1"});
        std::fs::write(wordcraft_control_key::session_path(&fixture.0, "window-a").unwrap(), descriptor.to_string()).unwrap();
        assert_eq!(live.call("agent.list_sessions", json!({})).unwrap()["sessions"], json!([]));
    }
}

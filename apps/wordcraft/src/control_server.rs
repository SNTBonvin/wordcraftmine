//! Loopback JSON-lines control server: one request per line, one reply per line.
//! This is the transport the MCP server (`wordcraft-cli mcp --connect`) wraps.
//!
//! Legacy TCP requests need the window's key (`docs/control-protocol.md`, Keys): once the port is
//! bound, the app writes a random key to `<settings>/control-key.<instance>` (mode 0600) and
//! removes the file when the window closes.
//! Live TCP instead mutually authenticates with nonce-bound HMAC proofs before editor RPCs;
//! private Unix live sockets use their directory/socket permissions without a TCP key.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use wordcraft_ui_egui::ControlRequest;

/// Longer request lines are refused (and the connection closed).
const MAX_LINE: usize = 1 << 20;

/// A connection that sends no authorised request in this time is closed.
const FIRST_REQUEST: Duration = Duration::from_secs(30);

const MAX_CONNECTIONS: usize = 16;
const MAX_LIVE_PREAUTH: usize = 4;
const MAX_LIVE_AUTH_LINE: usize = 8 * 1024;
const LIVE_FIRST_REQUEST: Duration = Duration::from_secs(3);
const MAX_PENDING: usize = 32;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const LINE_TIMEOUT: Duration = Duration::from_secs(60);

pub fn request_channel() -> (SyncSender<ControlRequest>, Receiver<ControlRequest>) {
    sync_channel(MAX_PENDING)
}

enum Stream {
    Tcp(TcpStream),
    #[cfg(unix)]
    Unix(std::os::unix::net::UnixStream),
}

impl Stream {
    fn try_clone(&self) -> std::io::Result<Self> {
        match self {
            Self::Tcp(s) => s.try_clone().map(Self::Tcp),
            #[cfg(unix)]
            Self::Unix(s) => s.try_clone().map(Self::Unix),
        }
    }
    fn set_nonblocking(&self, enabled: bool) -> std::io::Result<()> {
        match self {
            Self::Tcp(s) => s.set_nonblocking(enabled),
            #[cfg(unix)]
            Self::Unix(s) => s.set_nonblocking(enabled),
        }
    }
    fn set_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        match self {
            Self::Tcp(s) => s.set_read_timeout(timeout),
            #[cfg(unix)]
            Self::Unix(s) => s.set_read_timeout(timeout),
        }
    }
    fn set_write_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        match self {
            Self::Tcp(s) => s.set_write_timeout(timeout),
            #[cfg(unix)]
            Self::Unix(s) => s.set_write_timeout(timeout),
        }
    }
    fn shutdown(&self, how: Shutdown) -> std::io::Result<()> {
        match self {
            Self::Tcp(s) => s.shutdown(how),
            #[cfg(unix)]
            Self::Unix(s) => s.shutdown(how),
        }
    }
    fn peek(&self, byte: &mut [u8; 1]) -> std::io::Result<usize> {
        match self {
            Self::Tcp(s) => s.peek(byte),
            #[cfg(unix)]
            Self::Unix(s) => {
                rustix::net::recv(s, byte.as_mut_slice(), rustix::net::RecvFlags::PEEK).map(|(_, count)| count).map_err(std::io::Error::from)
            }
        }
    }
}
impl Read for Stream {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Tcp(s) => s.read(bytes),
            #[cfg(unix)]
            Self::Unix(s) => s.read(bytes),
        }
    }
}
impl Write for Stream {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Tcp(s) => s.write(bytes),
            #[cfg(unix)]
            Self::Unix(s) => s.write(bytes),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Tcp(s) => s.flush(),
            #[cfg(unix)]
            Self::Unix(s) => s.flush(),
        }
    }
}

enum Listener {
    Tcp(TcpListener),
    #[cfg(unix)]
    Unix(std::os::unix::net::UnixListener),
}
impl Listener {
    fn accept(&self) -> std::io::Result<Stream> {
        let stream = match self {
            Self::Tcp(s) => s.accept().map(|(stream, _)| Stream::Tcp(stream)),
            #[cfg(unix)]
            Self::Unix(s) => s.accept().map(|(stream, _)| Stream::Unix(stream)),
        }?;
        // Accepted sockets inherit nonblocking mode on some platforms. Workers use bounded
        // blocking I/O; normalizing this explicitly is required on Windows and macOS.
        stream.set_nonblocking(false)?;
        Ok(stream)
    }
    fn set_nonblocking(&self, enabled: bool) -> std::io::Result<()> {
        match self {
            Self::Tcp(s) => s.set_nonblocking(enabled),
            #[cfg(unix)]
            Self::Unix(s) => s.set_nonblocking(enabled),
        }
    }
}

struct Connection {
    socket: Stream,
    cancelled: Arc<AtomicBool>,
    authenticated: bool,
}

#[derive(Clone)]
struct LiveIdentity {
    session_id: String,
    document_id: u64,
}

struct LiveAuthentication {
    identity: Arc<LiveIdentity>,
    state: Arc<ServerState>,
}

impl LiveAuthentication {
    fn authenticate(&self, connection_id: u64) -> bool {
        let Ok(mut connections) = self.state.connections.lock() else { return false };
        if self.state.stopped.load(Ordering::Acquire) || connections.iter().filter(|(_, c)| c.authenticated).count() >= MAX_CONNECTIONS {
            return false;
        }
        let Some((_, connection)) = connections.iter_mut().find(|(id, _)| *id == connection_id) else { return false };
        if connection.cancelled.load(Ordering::Acquire) {
            return false;
        }
        connection.authenticated = true;
        true
    }
}

struct ServerState {
    stopped: AtomicBool,
    connections: Mutex<Vec<(u64, Connection)>>,
}

/// Owns the listener and all accepted connections. Dropping it revokes access immediately.
pub struct Server {
    state: Arc<ServerState>,
    listener: Option<std::thread::JoinHandle<()>>,
    key_file: Option<PathBuf>,
    socket_path: Option<PathBuf>,
    socket_dir: Option<PathBuf>,
    pub port: u16,
}

impl Server {
    pub fn stop(&mut self) {
        self.state.stopped.store(true, Ordering::Release);
        if let Ok(mut connections) = self.state.connections.lock() {
            for (_, conn) in connections.drain(..) {
                conn.cancelled.store(true, Ordering::Release);
                let _ = conn.socket.shutdown(Shutdown::Both);
            }
        }
        if let Some(thread) = self.listener.take() {
            let _ = thread.join();
        }
        if let Some(path) = self.key_file.take() {
            let _ = std::fs::remove_file(path);
        }
        if let Some(path) = self.socket_path.take() {
            let _ = std::fs::remove_file(path);
        }
        if let Some(path) = self.socket_dir.take() {
            let _ = std::fs::remove_dir(path);
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Bind a loopback listener, persist a private key, and start a fallible bounded transport.
/// Port zero asks the OS to select an available port.
pub fn start(port: u16, ctx: egui::Context, settings: &Path, tx: SyncSender<ControlRequest>, agent_only: bool) -> Result<Server, String> {
    start_tcp(port, ctx, settings, tx, agent_only, None)
}

fn start_tcp(
    port: u16,
    ctx: egui::Context,
    settings: &Path,
    tx: SyncSender<ControlRequest>,
    agent_only: bool,
    identity: Option<LiveIdentity>,
) -> Result<Server, String> {
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|e| format!("cannot bind control port: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let key = random_key()?;
    let path = if agent_only { wordcraft_control_key::live_key_path(settings, port) } else { wordcraft_control_key::key_path(settings, port) };
    write_key_file(&path, &key)?;
    let first = if identity.is_some() { LIVE_FIRST_REQUEST } else { FIRST_REQUEST };
    match serve_listener(Listener::Tcp(listener), port, ctx, key, first, tx, (agent_only, identity)) {
        Ok(mut server) => {
            server.key_file = Some(path);
            Ok(server)
        }
        Err(error) => {
            let _ = std::fs::remove_file(path);
            Err(error)
        }
    }
}

#[cfg(unix)]
fn start_unix(ctx: egui::Context, tx: SyncSender<ControlRequest>, session: &str) -> Result<Server, String> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    let base = wordcraft_control_key::live_runtime_base();
    std::fs::create_dir_all(&base).map_err(|e| format!("cannot create agent runtime directory: {e}"))?;
    let name = session.get(..16).ok_or("invalid agent session id")?;
    let dir = base.join(format!("wc-{name}"));
    std::fs::DirBuilder::new().mode(0o700).create(&dir).map_err(|e| format!("cannot create private agent directory: {e}"))?;
    let path = dir.join("agent.sock");
    let result = (|| {
        let listener = std::os::unix::net::UnixListener::bind(&path).map_err(|e| format!("cannot bind agent Unix socket: {e}"))?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
        let mut server = serve_listener(Listener::Unix(listener), 0, ctx, String::new(), FIRST_REQUEST, tx, (true, None))?;
        server.socket_path = Some(path.clone());
        server.socket_dir = Some(dir.clone());
        Ok(server)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }
    result
}

/// The runtime Agents switch, separate from the legacy developer control listener.
#[derive(Default)]
pub struct AgentRuntime {
    server: Option<Server>,
    descriptor: Option<PathBuf>,
    metadata: Option<Value>,
    check: Option<crate::agent_setup::CheckJob>,
}

impl AgentRuntime {
    pub fn update(&mut self, request: Value, ctx: &egui::Context, settings: &Path, tx: SyncSender<ControlRequest>) -> Result<Value, String> {
        if request.get("setup").and_then(Value::as_bool) == Some(true) {
            return Ok(crate::agent_setup::setup_metadata_request(request.get("refresh").and_then(Value::as_bool) == Some(true)));
        }
        if request.get("check").and_then(Value::as_bool) == Some(true) {
            let descriptor = self.metadata.as_ref().ok_or("enable agent discovery before checking this document")?;
            if request.get("poll").and_then(Value::as_bool) != Some(true) {
                if request.get("sessionId") != descriptor.get("sessionId") || request.get("documentId") != descriptor.get("documentId") {
                    return Err("the document changed before the connection check".into());
                }
                self.check = Some(crate::agent_setup::CheckJob::start(descriptor.clone(), ctx.clone())?);
            }
            return self.check.as_ref().map(|job| job.poll()).ok_or_else(|| "MCP check is not running".into());
        }
        if request.get("enabled").and_then(Value::as_bool) != Some(true) {
            self.stop();
            return Ok(json!({"enabled": false}));
        }
        let document_id = request.get("documentId").and_then(Value::as_u64).ok_or("missing documentId")?;
        let title = request.get("title").and_then(Value::as_str).ok_or("missing document title")?;
        if request.get("refresh").and_then(Value::as_bool) == Some(true) {
            let mut descriptor = self.metadata.clone().ok_or("agent access is not enabled")?;
            if descriptor.get("documentId").and_then(Value::as_u64) != Some(document_id) {
                return Err("cannot refresh agent access for a different document".into());
            }
            if self.server.is_none() {
                return Err("agent listener is not running".into());
            }
            let fields = descriptor.as_object_mut().ok_or("invalid agent descriptor")?;
            fields.insert("title".into(), Value::String(title.to_string()));
            let path = self.descriptor.as_ref().ok_or("agent descriptor is unavailable")?;
            write_descriptor(path, &descriptor)?;
            self.metadata = Some(descriptor.clone());
            return Ok(descriptor);
        }
        self.stop();
        let session_id = random_key()?;
        #[cfg(unix)]
        let server = start_unix(ctx.clone(), tx, &session_id)?;
        #[cfg(not(unix))]
        let server = start_tcp(0, ctx.clone(), settings, tx, true, Some(LiveIdentity { session_id: session_id.clone(), document_id }))?;
        let mut descriptor = json!({
            "sessionId": session_id,
            "documentId": document_id,
            "title": title,
            "version": env!("CARGO_PKG_VERSION"),
            "protocolVersion": 1,
        });
        if let Some(fields) = descriptor.as_object_mut() {
            if let Some(socket) = &server.socket_path {
                fields.insert("transport".into(), json!("unix"));
                fields.insert("socketPath".into(), json!(socket.to_str().ok_or("agent socket path must be valid Unicode")?));
            } else {
                fields.insert("transport".into(), json!("tcp"));
                fields.insert("address".into(), json!(format!("127.0.0.1:{}", server.port)));
                fields.insert("authScheme".into(), json!("hmac-sha256-v1"));
            }
        }
        let path = settings.join("agents").join("sessions").join(format!("{session_id}.json"));
        write_descriptor(&path, &descriptor)?;
        self.server = Some(server);
        self.descriptor = Some(path);
        self.metadata = Some(descriptor.clone());
        Ok(descriptor)
    }

    pub fn stop(&mut self) {
        if let Some(path) = self.descriptor.take() {
            let _ = std::fs::remove_file(path);
        }
        self.server = None;
        self.metadata = None;
        self.check = None;
    }
}

impl Drop for AgentRuntime {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A new key: 32 random bytes from the operating system, as hex.
fn random_key() -> Result<String, String> {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).map_err(|e| e.to_string())?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// Write the key file: a stale one (from a run that did not exit cleanly) is replaced; on Unix
/// the new file is created with mode 0600.
fn write_key_file(p: &Path, key: &str) -> Result<(), String> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("cannot create {}: {e}", d.display()))?;
    }
    let _ = std::fs::remove_file(p);
    let mut open = std::fs::OpenOptions::new();
    open.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut open, 0o600);
    let mut f = open.open(p).map_err(|e| format!("cannot create {}: {e}", p.display()))?;
    if let Err(error) = f.write_all(key.as_bytes()) {
        drop(f);
        let _ = std::fs::remove_file(p);
        return Err(format!("cannot write {}: {error}", p.display()));
    }
    Ok(())
}

/// Publish complete metadata in one rename. Discovery never observes a partially written
/// title; a failed refresh leaves the old descriptor and active access unchanged.
fn write_descriptor(path: &Path, descriptor: &Value) -> Result<(), String> {
    let temporary = path.with_extension(format!("{}.tmp", random_key()?));
    write_key_file(&temporary, &descriptor.to_string())?;
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(temporary);
        return Err(format!("cannot publish {}: {error}", path.display()));
    }
    Ok(())
}

/// Compare a key with the window's in time that does not depend on where they differ. An
/// empty key never matches.
fn keys_match(given: &str, key: &str) -> bool {
    let (a, b) = (given.as_bytes(), key.as_bytes());
    !b.is_empty() && a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
fn serve_on(
    listener: TcpListener,
    ctx: egui::Context,
    key: String,
    first: Duration,
    tx: SyncSender<ControlRequest>,
    agent_only: bool,
) -> Result<Server, String> {
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    serve_listener(Listener::Tcp(listener), port, ctx, key, first, tx, (agent_only, None))
}

fn serve_listener(
    listener: Listener,
    port: u16,
    ctx: egui::Context,
    key: String,
    first: Duration,
    tx: SyncSender<ControlRequest>,
    options: (bool, Option<LiveIdentity>),
) -> Result<Server, String> {
    let (agent_only, live_identity) = options;
    let live_identity = live_identity.map(Arc::new);
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let state = Arc::new(ServerState { stopped: AtomicBool::new(false), connections: Mutex::new(Vec::new()) });
    let worker_state = state.clone();
    let key: Arc<str> = key.into();
    let thread = std::thread::Builder::new()
        .name("wordcraft-control".into())
        .spawn(move || {
            let mut next_id = 0u64;
            while !worker_state.stopped.load(Ordering::Acquire) {
                let stream = match listener.accept() {
                    Ok(stream) => stream,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(e) => {
                        log::warn!("control accept: {e}");
                        break;
                    }
                };
                let deadline = Instant::now() + first;
                let Ok(socket) = stream.try_clone() else { continue };
                let cancelled = Arc::new(AtomicBool::new(false));
                let id = next_id;
                next_id = next_id.wrapping_add(1);
                {
                    let Ok(mut connections) = worker_state.connections.lock() else { break };
                    let full = if live_identity.is_some() {
                        connections.iter().filter(|(_, c)| !c.authenticated).count() >= MAX_LIVE_PREAUTH
                    } else {
                        connections.len() >= MAX_CONNECTIONS
                    };
                    if worker_state.stopped.load(Ordering::Acquire) || full {
                        let _ = stream.shutdown(Shutdown::Both);
                        continue;
                    }
                    connections.push((id, Connection { socket, cancelled: cancelled.clone(), authenticated: false }));
                }
                let live_auth = live_identity.as_ref().map(|identity| LiveAuthentication { identity: identity.clone(), state: worker_state.clone() });
                let (tx, ctx, key, state) = (tx.clone(), ctx.clone(), key.clone(), worker_state.clone());
                if let Err(e) = std::thread::Builder::new().name("wordcraft-control-client".into()).spawn(move || {
                    serve(stream, tx, ctx, &key, ConnectionPolicy { deadline, agent_only, id, line_timeout: LINE_TIMEOUT, live_auth }, cancelled);
                    if let Ok(mut connections) = state.connections.lock() {
                        connections.retain(|(connection_id, _)| *connection_id != id);
                    }
                }) {
                    log::warn!("cannot start control worker: {e}");
                    if let Ok(mut connections) = worker_state.connections.lock() {
                        connections.retain(|(connection_id, _)| *connection_id != id);
                    }
                }
            }
        })
        .map_err(|e| format!("cannot start control listener: {e}"))?;
    Ok(Server { state, listener: Some(thread), key_file: None, socket_path: None, socket_dir: None, port })
}

/// The read side of a connection with an optional overall deadline: each read waits only for
/// the time left, so blank lines, bad JSON or a slow drip of bytes cannot extend it.
struct Timed {
    s: Stream,
    deadline: Option<Instant>,
    partial_timeout: Option<Duration>,
}

impl Read for Timed {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if let Some(d) = self.deadline {
            let left = d.checked_duration_since(Instant::now()).filter(|l| !l.is_zero()).ok_or(std::io::ErrorKind::TimedOut)?;
            self.s.set_read_timeout(Some(left))?;
        }
        let n = self.s.read(buf)?;
        if n != 0
            && self.deadline.is_none()
            && let Some(timeout) = self.partial_timeout
        {
            self.deadline = Some(Instant::now() + timeout);
        }
        Ok(n)
    }
}

impl Timed {
    fn set_deadline(&mut self, d: Option<Instant>) {
        self.deadline = d;
        if d.is_none() {
            let _ = self.s.set_read_timeout(None);
        }
    }
}

/// The next line (trimmed), `None` at the end of the stream or after a read error.
fn read_line(r: &mut BufReader<Timed>, limit: usize) -> Option<Result<String, &'static str>> {
    let mut buf = Vec::new();
    let n = r.by_ref().take(limit as u64 + 1).read_until(b'\n', &mut buf).ok()?;
    if n == 0 {
        return None;
    }
    if buf.len() > limit {
        return Some(Err("line too long"));
    }
    Some(String::from_utf8(buf).map(|s| s.trim().to_string()).map_err(|_| "not UTF-8"))
}

/// Read and drop the rest of an oversize line, so closing the socket does not reset the
/// connection before the client has read the error. At most 64 MiB, within 2 s.
fn discard_rest_of_line(r: &mut BufReader<Timed>) {
    r.get_mut().set_deadline(Some(Instant::now() + Duration::from_secs(2)));
    let mut left: usize = 64 << 20;
    while left > 0 {
        let (used, done) = match r.fill_buf() {
            Ok([]) | Err(_) => return,
            Ok(b) => match b.iter().position(|&c| c == b'\n') {
                Some(i) => (i + 1, true),
                None => (b.len(), false),
            },
        };
        r.consume(used);
        if done {
            return;
        }
        left = left.saturating_sub(used);
    }
}

struct ConnectionPolicy {
    deadline: Instant,
    agent_only: bool,
    id: u64,
    line_timeout: Duration,
    live_auth: Option<LiveAuthentication>,
}

/// Cancellation and membership cleanup must run on every socket exit, including I/O errors.
struct ConnectionExit {
    cancelled: Arc<AtomicBool>,
    tx: SyncSender<ControlRequest>,
    ctx: egui::Context,
    identity: Option<Value>,
    pending_identity: Option<Value>,
}

impl Drop for ConnectionExit {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        self.ctx.request_repaint();
        // A pending status might have registered the member just before its reply was lost.
        // The UI compares the transport-owned connection id, so unsuccessful/stale identities
        // cannot revoke a different connection that has since registered successfully.
        for identity in self.identity.take().into_iter().chain(self.pending_identity.take()) {
            // Cleanup does not edit the document and stays useful after a long UI stall.
            // Its connection marker makes late delivery harmless to a replacement client.
            let (request, _reply) = ControlRequest::new("agent.transportClosed", identity);
            if self.tx.try_send(request).is_ok() {
                self.ctx.request_repaint();
            }
        }
    }
}

fn connection_identity(params: &Value, connection_id: u64) -> Option<Value> {
    Some(json!({
        "sessionId":params.get("sessionId")?.as_str()?,
        "documentId":params.get("documentId")?.as_u64()?,
        "agentName":params.get("agentName")?.as_str()?.trim(),
        "clientId":params.get("clientId")?.as_str()?,
        "_connectionId":connection_id,
    }))
}

fn serve(stream: Stream, tx: SyncSender<ControlRequest>, ctx: egui::Context, key: &str, policy: ConnectionPolicy, cancelled: Arc<AtomicBool>) {
    let ConnectionPolicy { deadline, agent_only, id: connection_id, line_timeout, live_auth } = policy;
    let mut exit = ConnectionExit { cancelled: cancelled.clone(), tx: tx.clone(), ctx: ctx.clone(), identity: None, pending_identity: None };
    if stream.set_nonblocking(false).is_err() {
        return;
    }
    let Ok(read) = stream.try_clone() else { return };
    let mut out = stream;
    if out.set_write_timeout(Some(Duration::from_secs(5))).is_err() {
        return;
    }
    // One overall deadline from accept to the first authorised request; after it, none.
    let mut reader = BufReader::new(Timed { s: read, deadline: Some(deadline), partial_timeout: None });
    let mut authorised = false;
    let mut challenge: Option<(String, String)> = None;
    // The key an `auth` request gave this connection.
    let mut conn_key: Option<String> = None;
    while !cancelled.load(Ordering::Acquire) {
        if authorised {
            let buffered = !reader.buffer().is_empty();
            reader.get_mut().set_deadline(if buffered { Some(Instant::now() + line_timeout) } else { None });
            reader.get_mut().partial_timeout = Some(line_timeout);
        }
        let authenticating_live = live_auth.is_some() && !authorised;
        let Some(line) = read_line(&mut reader, if authenticating_live { MAX_LIVE_AUTH_LINE } else { MAX_LINE }) else { break };
        let write_timeout = if authenticating_live {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else { break };
            remaining.max(Duration::from_millis(1))
        } else {
            Duration::from_secs(5)
        };
        if out.set_write_timeout(Some(write_timeout)).is_err() {
            break;
        }
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                let _ = writeln!(out, "{}", json!({"ok": false, "error": e}));
                if e == "line too long" && (live_auth.is_none() || authorised) {
                    discard_rest_of_line(&mut reader);
                }
                break;
            }
        };
        if line.is_empty() {
            continue;
        }
        let msg = match serde_json::from_str::<Value>(&line) {
            Ok(m) => m,
            // Close on a line that isn't JSON: an HTTP request (say, a web page's `fetch` to this
            // port) starts with a request line that never parses, so it can't carry a command in
            // its body, even before the key is checked.
            Err(e) => {
                let _ = writeln!(out, "{}", json!({"ok": false, "error": format!("bad JSON: {e}; closing the connection")}));
                break;
            }
        };
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("").to_string();
        let mut params = msg.get("params").cloned().unwrap_or(json!({}));
        // The top-level `key`; `auth` may carry it in its params. For other methods `params.key`
        // is an ordinary parameter (`ui.key {"key": "Enter"}`).
        let given =
            msg.get("key").and_then(Value::as_str).or_else(|| if method == "auth" { params.get("key").and_then(Value::as_str) } else { None });
        let mut reply = |mut v: Value| -> bool {
            if let Some(o) = v.as_object_mut() {
                o.insert("id".into(), id.clone());
            }
            writeln!(out, "{v}").is_ok()
        };
        if let Some(auth) = &live_auth {
            // Live TCP never sends or accepts the raw key. A freshly authenticated socket
            // proves both endpoints before any editor request or consent prompt is queued.
            if msg.get("key").is_some() || params.get("key").is_some() {
                let _ = reply(json!({"ok":false,"error":"live authentication does not accept raw keys"}));
                break;
            }
            if !authorised {
                if Instant::now() >= deadline {
                    break;
                }
                let identity = &auth.identity;
                if method == "auth.challenge" && challenge.is_none() {
                    let Some(client_nonce) = params.get("clientNonce").and_then(Value::as_str) else {
                        let _ = reply(json!({"ok":false,"error":"invalid live authentication challenge"}));
                        break;
                    };
                    if params.get("sessionId").and_then(Value::as_str) != Some(identity.session_id.as_str())
                        || params.get("documentId").and_then(Value::as_u64) != Some(identity.document_id)
                    {
                        let _ = reply(json!({"ok":false,"error":"live authentication session identity mismatch"}));
                        break;
                    }
                    let proof = wordcraft_control_key::fresh_nonce().and_then(|server_nonce| {
                        wordcraft_control_key::live_auth_proof(key, "server", &identity.session_id, identity.document_id, client_nonce, &server_nonce)
                            .map(|proof| (server_nonce, proof))
                    });
                    let Ok((server_nonce, proof)) = proof else {
                        let _ = reply(json!({"ok":false,"error":"invalid live authentication challenge"}));
                        break;
                    };
                    challenge = Some((client_nonce.to_string(), server_nonce.clone()));
                    if !reply(json!({"ok":true,"result":{"serverNonce":server_nonce,"proof":proof}})) {
                        break;
                    }
                    continue;
                }
                if method == "auth.prove"
                    && let Some((client_nonce, server_nonce)) = challenge.take()
                    && let Some(proof) = params.get("proof").and_then(Value::as_str)
                    && wordcraft_control_key::verify_live_auth_proof(
                        key,
                        "client",
                        &identity.session_id,
                        identity.document_id,
                        &client_nonce,
                        &server_nonce,
                        proof,
                    )
                    .is_ok()
                {
                    if Instant::now() >= deadline || !auth.authenticate(connection_id) {
                        let _ = reply(json!({"ok":false,"error":"live authenticated connection limit reached"}));
                        break;
                    }
                    authorised = true;
                    if !reply(json!({"ok":true,"result":{"authenticated":true,"sessionId":identity.session_id,"documentId":identity.document_id}})) {
                        break;
                    }
                    continue;
                }
                let _ = reply(json!({"ok":false,"error":"live authentication required or proof invalid"}));
                break;
            }
        }
        // Checked on every request.
        let k = given.or(conn_key.as_deref()).unwrap_or("");
        // Unix access is authorized by the owner-only directory/socket. TCP still requires
        // the per-window secret on every request (or connection authentication).
        if live_auth.is_none() && !key.is_empty() && !keys_match(k, key) {
            let _ = reply(json!({"ok": false, "error": "unauthorized"}));
            break;
        }
        if !authorised {
            authorised = true;
            reader.get_mut().set_deadline(Some(Instant::now() + LINE_TIMEOUT));
        }
        if live_auth.is_none() && method == "auth" {
            conn_key = Some(k.to_string());
            if !reply(json!({"ok": true, "result": {}})) {
                break;
            }
            continue;
        }
        if method == "agent.transportClosed"
            || (agent_only && !matches!(method.as_str(), "agent.status" | "agent.call" | "agent.disconnect" | "agent.check"))
        {
            if !reply(json!({"ok": false, "error": "use the agent session methods"})) {
                break;
            }
            continue;
        }
        if agent_only {
            let Some(fields) = params.as_object_mut() else {
                if !reply(json!({"ok": false, "error": "agent params must be an object"})) {
                    break;
                }
                continue;
            };
            fields.insert("_connectionId".into(), json!(connection_id));
            if method == "agent.status" {
                exit.pending_identity = connection_identity(&params, connection_id);
            }
        }
        let is_status = agent_only && method == "agent.status";
        let is_disconnect = agent_only && method == "agent.disconnect";
        let request_started = Instant::now();
        let (mut req, rrx) = ControlRequest::new(method, params);
        req.deadline = Some(Instant::now() + REQUEST_TIMEOUT);
        req.cancelled = cancelled.clone();
        if tx.try_send(req).is_err() {
            let _ = reply(json!({"ok": false, "error": "control queue full or closed"}));
            break;
        }
        ctx.request_repaint();
        let r = loop {
            if cancelled.load(Ordering::Acquire) {
                return;
            }
            match rrx.recv_timeout(Duration::from_millis(50)) {
                Ok(value) => break value,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break json!({"ok": false, "error": "control disconnected"}),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            }
            // Observe a peer closing while its request is still waiting in the UI queue.
            // A tiny bounded peek does not consume pipelined request bytes; Timed restores
            // the normal line deadline before the next read.
            if reader.get_ref().s.set_read_timeout(Some(Duration::from_millis(1))).is_err() {
                cancelled.store(true, Ordering::Release);
                return;
            }
            let mut byte = [0u8; 1];
            match reader.get_ref().s.peek(&mut byte) {
                Ok(0) => {
                    cancelled.store(true, Ordering::Release);
                    return;
                }
                Err(error) if !matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                    cancelled.store(true, Ordering::Release);
                    return;
                }
                _ => {}
            }
            // The queued request is expired before reporting timeout, so the UI cannot
            // execute it later when it becomes responsive again.
            if request_started.elapsed() >= REQUEST_TIMEOUT {
                cancelled.store(true, Ordering::Release);
                break json!({"ok": false, "error": "timeout; result may be unknown if execution started"});
            }
        };
        if is_status {
            if r.get("ok").and_then(Value::as_bool) == Some(true) {
                exit.identity = exit.pending_identity.take();
            } else {
                exit.pending_identity = None;
            }
        }
        if is_disconnect && r.get("ok").and_then(Value::as_bool) == Some(true) {
            exit.identity = None;
            exit.pending_identity = None;
        }
        if !reply(r) || cancelled.load(Ordering::Acquire) {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpStream;

    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn boot_with(first: Duration) -> (u16, TestServer) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = request_channel();
        let server = serve_on(listener, egui::Context::default(), KEY.to_string(), first, tx, false).unwrap();
        (port, TestServer { rx, _server: server })
    }

    struct TestServer {
        rx: Receiver<ControlRequest>,
        _server: Server,
    }
    impl std::ops::Deref for TestServer {
        type Target = Receiver<ControlRequest>;
        fn deref(&self) -> &Self::Target {
            &self.rx
        }
    }

    fn boot() -> (u16, TestServer) {
        boot_with(FIRST_REQUEST)
    }

    fn boot_live(first: Duration) -> (u16, TestServer) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = request_channel();
        let identity = LiveIdentity { session_id: "live-window".into(), document_id: 7 };
        let server = serve_listener(Listener::Tcp(listener), port, egui::Context::default(), KEY.into(), first, tx, (true, Some(identity))).unwrap();
        (port, TestServer { rx, _server: server })
    }

    fn wait_connections(server: &TestServer, expected: usize) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if server._server.state.connections.lock().unwrap().len() == expected {
                return;
            }
            assert!(Instant::now() < deadline, "connection slots were not reclaimed");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Answer the next request that reaches the UI with `ok`, and return it.
    fn answer(rx: &Receiver<ControlRequest>) -> ControlRequest {
        let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let _ = req.reply.send(json!({"ok": true, "result": {}}));
        req
    }

    /// One connection, several lines.
    struct Conn(BufReader<Stream>, Stream);

    impl Conn {
        fn open(port: u16) -> Self {
            let s = Stream::Tcp(TcpStream::connect(("127.0.0.1", port)).unwrap());
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            Self(BufReader::new(s.try_clone().unwrap()), s)
        }
        #[cfg(unix)]
        fn open_unix(path: &Path) -> Self {
            let s = Stream::Unix(std::os::unix::net::UnixStream::connect(path).unwrap());
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            Self(BufReader::new(s.try_clone().unwrap()), s)
        }
        fn send(&mut self, line: &str) {
            writeln!(self.1, "{line}").unwrap();
        }
        fn recv(&mut self) -> Value {
            let mut r = String::new();
            let _ = self.0.read_line(&mut r);
            serde_json::from_str(&r).unwrap_or_default()
        }
        fn call(&mut self, line: &str) -> Value {
            self.send(line);
            self.recv()
        }
        /// The server closed the connection.
        fn closed(&mut self) -> bool {
            let mut r = String::new();
            matches!(self.0.read_line(&mut r), Ok(0))
        }
        fn live_challenge(&mut self) -> (String, String, String) {
            self.live_challenge_for(KEY, "live-window", 7)
        }
        fn live_challenge_for(&mut self, key: &str, session: &str, document: u64) -> (String, String, String) {
            let client = wordcraft_control_key::fresh_nonce().unwrap();
            let response = self.call(
                &json!({"id":1,"method":"auth.challenge","params":{"clientNonce":client,"sessionId":session,"documentId":document}}).to_string(),
            );
            assert_eq!(response["ok"], true, "live challenge was rejected: {response}");
            let server = response["result"]["serverNonce"].as_str().unwrap().to_string();
            let proof = response["result"]["proof"].as_str().unwrap().to_string();
            wordcraft_control_key::verify_live_auth_proof(key, "server", session, document, &client, &server, &proof).unwrap();
            (client, server, proof)
        }
        fn live_authenticate(&mut self) {
            self.live_authenticate_for(KEY, "live-window", 7);
        }
        fn live_authenticate_for(&mut self, key: &str, session: &str, document: u64) {
            let (client, server, _) = self.live_challenge_for(key, session, document);
            let proof = wordcraft_control_key::live_auth_proof(key, "client", session, document, &client, &server).unwrap();
            let response = self.call(&json!({"id":2,"method":"auth.prove","params":{"proof":proof}}).to_string());
            assert_eq!(response["ok"], true, "live client proof was rejected: {response}");
            assert_eq!(response["result"], json!({"authenticated":true,"sessionId":session,"documentId":document}));
        }
    }

    #[test]
    fn live_tcp_proves_both_endpoints_before_forwarding_and_rejects_raw_keys() {
        let (port, rx) = boot_live(LIVE_FIRST_REQUEST);
        for request in [
            json!({"method":"agent.check","params":{"sessionId":"live-window","documentId":7}}),
            json!({"method":"auth","key":KEY}),
            json!({"method":"auth.challenge","key":KEY,"params":{"clientNonce":"01".repeat(32),"sessionId":"live-window","documentId":7}}),
            json!({"method":"auth.prove","params":{"proof":"00".repeat(32)}}),
        ] {
            let mut c = Conn::open(port);
            assert_eq!(c.call(&request.to_string())["ok"], false);
            assert!(c.closed());
            assert!(rx.try_recv().is_err());
        }
        let mut c = Conn::open(port);
        c.live_authenticate();
        assert!(rx.try_recv().is_err(), "authentication must not request editor access");
        c.send(r#"{"id":3,"method":"agent.check","params":{"sessionId":"live-window","documentId":7}}"#);
        assert_eq!(answer(&rx).method, "agent.check");
        assert_eq!(c.recv()["ok"], true);
        assert_eq!(c.call(&json!({"method":"agent.check","key":KEY}).to_string())["ok"], false);
        assert!(c.closed());
    }

    #[test]
    fn live_tcp_rejects_stale_identity_reflection_duplicate_challenge_and_replayed_proof() {
        let (port, rx) = boot_live(LIVE_FIRST_REQUEST);
        for (session, document) in [("old-window", 7), ("live-window", 8)] {
            let mut c = Conn::open(port);
            let response = c.call(
                &json!({"method":"auth.challenge","params":{"clientNonce":"01".repeat(32),"sessionId":session,"documentId":document}}).to_string(),
            );
            assert_eq!(response["ok"], false);
            assert!(response.get("result").is_none());
            assert!(c.closed());
        }
        let mut reflected = Conn::open(port);
        let (client, server, server_proof) = reflected.live_challenge();
        let client_proof = wordcraft_control_key::live_auth_proof(KEY, "client", "live-window", 7, &client, &server).unwrap();
        assert_eq!(reflected.call(&json!({"method":"auth.prove","params":{"proof":server_proof}}).to_string())["ok"], false);
        assert!(reflected.closed());
        let mut replay = Conn::open(port);
        let (_, new_server, _) = replay.live_challenge();
        assert_ne!(new_server, server);
        assert_eq!(replay.call(&json!({"method":"auth.prove","params":{"proof":client_proof}}).to_string())["ok"], false);
        assert!(replay.closed());
        let mut duplicate = Conn::open(port);
        duplicate.live_challenge();
        assert_eq!(
            duplicate.call(&json!({"method":"auth.challenge","params":{"clientNonce":client,"sessionId":"live-window","documentId":7}}).to_string())
                ["ok"],
            false
        );
        assert!(duplicate.closed());
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn live_tcp_pre_authentication_slots_are_separate_and_reclaimed() {
        let (port, mut rx) = boot_live(Duration::from_secs(10));
        let mut authenticated: Vec<_> = (0..MAX_CONNECTIONS)
            .map(|_| {
                let mut c = Conn::open(port);
                c.live_authenticate();
                c
            })
            .collect();
        let mut pending: Vec<_> = (0..MAX_LIVE_PREAUTH).map(|_| Conn::open(port)).collect();
        wait_connections(&rx, MAX_CONNECTIONS + MAX_LIVE_PREAUTH);
        assert!(Conn::open(port).closed(), "idle unauthenticated sockets have a separate small limit");
        authenticated.first_mut().unwrap().send(r#"{"method":"agent.check","params":{"sessionId":"live-window","documentId":7}}"#);
        answer(&rx);
        assert_eq!(authenticated.first_mut().unwrap().recv()["ok"], true);
        pending.pop();
        wait_connections(&rx, MAX_CONNECTIONS + MAX_LIVE_PREAUTH - 1);
        let mut excess = Conn::open(port);
        let (client, server, _) = excess.live_challenge();
        let proof = wordcraft_control_key::live_auth_proof(KEY, "client", "live-window", 7, &client, &server).unwrap();
        assert_eq!(excess.call(&json!({"method":"auth.prove","params":{"proof":proof}}).to_string())["ok"], false);
        assert!(excess.closed(), "the authenticated cap is also enforced atomically");
        wait_connections(&rx, MAX_CONNECTIONS + MAX_LIVE_PREAUTH - 1);
        rx._server.stop();
        for mut connection in authenticated.into_iter().chain(pending) {
            assert_eq!(connection.recv(), Value::Null, "stop closes pre-authentication and authenticated sockets");
        }
    }

    #[test]
    fn live_tcp_pre_authentication_deadline_covers_both_round_trips_and_slow_input() {
        let (port, rx) = boot_live(Duration::from_millis(500));
        let mut challenged = Conn::open(port);
        challenged.live_challenge();
        let started = Instant::now();
        assert!(challenged.closed());
        assert!(started.elapsed() < Duration::from_secs(2));
        wait_connections(&rx, 0);
        let mut slow = Conn::open(port);
        slow.1.write_all(b"{").unwrap();
        std::thread::sleep(Duration::from_millis(100));
        slow.1.write_all(b" ").unwrap();
        std::thread::sleep(Duration::from_millis(100));
        slow.1.write_all(b" ").unwrap();
        assert!(slow.closed(), "each arriving byte must not extend the overall deadline");
        wait_connections(&rx, 0);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn nonblocking_accepted_socket_is_normalized_and_authorized_idle_stays_connected() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut c = Conn::open(port);
        let (socket, _) = listener.accept().unwrap();
        socket.set_nonblocking(true).unwrap();
        let (tx, rx) = request_channel();
        let worker = std::thread::spawn(move || {
            serve(
                Stream::Tcp(socket),
                tx,
                egui::Context::default(),
                KEY,
                ConnectionPolicy {
                    deadline: Instant::now() + Duration::from_secs(2),
                    agent_only: false,
                    id: 1,
                    line_timeout: Duration::from_millis(80),
                    live_auth: None,
                },
                Arc::new(AtomicBool::new(false)),
            )
        });
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(c.call(&format!(r#"{{"method":"auth","key":"{KEY}"}}"#))["ok"], true);
        // The partial-line bound must not turn into an authenticated idle timer.
        std::thread::sleep(Duration::from_millis(200));
        c.send(r#"{"method":"document.inspect"}"#);
        answer(&rx);
        assert_eq!(c.recv()["ok"], true);
        let started = Instant::now();
        c.1.write_all(b"{").unwrap();
        assert!(c.closed(), "a partial JSON line has an overall deadline");
        assert!(started.elapsed() < Duration::from_secs(2));
        worker.join().unwrap();
    }

    #[test]
    fn a_request_without_the_key_is_refused_and_closed() {
        let (port, rx) = boot();
        let mut c = Conn::open(port);
        let r = c.call(r#"{"id":1,"method":"document.inspect"}"#);
        assert_eq!((r["ok"].clone(), r["error"].clone(), r["id"].clone()), (json!(false), json!("unauthorized"), json!(1)));
        assert!(c.closed());
        assert!(rx.try_recv().is_err(), "nothing reached the UI");
    }

    #[test]
    fn a_wrong_key_is_refused_and_closed() {
        let (port, rx) = boot();
        for key in ["nope", &KEY[..63], &format!("{KEY}0"), &KEY.to_uppercase()] {
            let mut c = Conn::open(port);
            assert_eq!(c.call(&format!(r#"{{"id":1,"key":"{key}","method":"document.inspect"}}"#))["error"], "unauthorized", "{key}");
            assert!(c.closed());
        }
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn the_key_on_each_request() {
        let (port, rx) = boot();
        let mut c = Conn::open(port);
        c.send(&format!(r#"{{"id":7,"key":"{KEY}","method":"document.inspect","params":{{"text":true}}}}"#));
        let req = answer(&rx);
        assert_eq!((req.method.as_str(), &req.params), ("document.inspect", &json!({"text": true})));
        let r = c.recv();
        assert_eq!((r["ok"].clone(), r["id"].clone()), (json!(true), json!(7)));
        // Every request needs it: one without the key is refused, also after good ones.
        assert_eq!(c.call(r#"{"id":8,"method":"document.inspect"}"#)["error"], "unauthorized");
        assert!(c.closed());
    }

    #[test]
    fn auth_once_per_connection() {
        let (port, rx) = boot();
        let mut c = Conn::open(port);
        assert_eq!(c.call(&format!(r#"{{"id":1,"method":"auth","key":"{KEY}"}}"#))["ok"], true);
        for id in 2..4 {
            c.send(&format!(r#"{{"id":{id},"method":"document.inspect"}}"#));
            answer(&rx);
            assert_eq!(c.recv()["ok"], true);
        }
        // `params.key` works for `auth` too.
        let mut c = Conn::open(port);
        assert_eq!(c.call(&format!(r#"{{"id":1,"method":"auth","params":{{"key":"{KEY}"}}}}"#))["ok"], true);
        // A wrong key on an authenticated connection is still refused, and closes it.
        assert_eq!(c.call(r#"{"id":2,"key":"nope","method":"document.inspect"}"#)["error"], "unauthorized");
        assert!(c.closed());
        // A failed auth closes the connection.
        let mut c = Conn::open(port);
        assert_eq!(c.call(r#"{"id":1,"method":"auth","key":"nope"}"#)["error"], "unauthorized");
        assert!(c.closed());
    }

    #[test]
    fn params_key_is_only_a_secret_for_auth() {
        let (port, rx) = boot();
        // `params.key` does not authenticate other methods.
        let mut c = Conn::open(port);
        assert_eq!(c.call(&format!(r#"{{"id":1,"method":"document.inspect","params":{{"key":"{KEY}"}}}}"#))["error"], "unauthorized");
        // It is an ordinary parameter for them (`ui.key` presses a key).
        let mut c = Conn::open(port);
        c.send(&format!(r#"{{"id":1,"key":"{KEY}","method":"ui.key","params":{{"key":"Enter"}}}}"#));
        let req = answer(&rx);
        assert_eq!((req.method.as_str(), &req.params), ("ui.key", &json!({"key": "Enter"})));
        assert_eq!(c.recv()["ok"], true);
    }

    #[test]
    fn a_line_over_1_mib_is_refused_and_closed() {
        let (port, rx) = boot();
        let mut c = Conn::open(port);
        let big = format!(r#"{{"id":1,"key":"{KEY}","method":"document.inspect","pad":"{}"}}"#, "x".repeat(MAX_LINE));
        assert_eq!(c.call(&big)["error"], "line too long");
        assert!(c.closed());
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn a_silent_connection_is_closed() {
        let (port, rx) = boot_with(Duration::from_millis(300));
        let mut quiet = Conn::open(port);
        let t0 = Instant::now();
        assert!(quiet.closed());
        assert!(t0.elapsed() < Duration::from_secs(3));
        // After an authorised request the connection may stay idle.
        let mut busy = Conn::open(port);
        assert_eq!(busy.call(&format!(r#"{{"id":1,"method":"auth","key":"{KEY}"}}"#))["ok"], true);
        std::thread::sleep(Duration::from_millis(600));
        busy.send(r#"{"id":2,"method":"document.inspect"}"#);
        answer(&rx);
        assert_eq!(busy.recv()["ok"], true);
    }

    #[test]
    fn the_first_request_limit_is_one_overall_deadline() {
        // Blank lines and bad JSON do not keep a connection without the key open.
        let (port, _rx) = boot_with(Duration::from_millis(300));
        let s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let mut w = s.try_clone().unwrap();
        let feeder = std::thread::spawn(move || {
            for i in 0..40 {
                if w.write_all(if i % 2 == 0 { b"\n" } else { b"not json\n" }).is_err() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        let t0 = Instant::now();
        let mut r = BufReader::new(s);
        r.get_ref().set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut line = String::new();
        loop {
            line.clear();
            if matches!(r.read_line(&mut line), Ok(0) | Err(_)) {
                break;
            }
            assert!(line.contains("bad JSON"), "{line}");
        }
        let _ = feeder.join();
        assert!(t0.elapsed() < Duration::from_millis(1500), "{:?}", t0.elapsed());
    }

    #[test]
    fn bad_json_closes_the_connection() {
        let (port, rx) = boot();
        let mut c = Conn::open(port);
        assert_eq!(c.call("not json")["ok"], false);
        assert!(c.closed());
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn http_request_cannot_smuggle_a_command() {
        // What a web page's `fetch("http://127.0.0.1:<port>/", {method: "POST", body})` sends,
        // even with the key in the body.
        let (port, rx) = boot();
        let body = format!("\n{{\"key\":\"{KEY}\",\"method\":\"file.saveAs\",\"params\":{{\"path\":\"/tmp/x.docx\"}}}}\n");
        let req = format!("POST / HTTP/1.1\r\nHost: 127.0.0.1:7981\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{body}", body.len());
        let mut c = Conn::open(port);
        c.1.write_all(req.as_bytes()).unwrap();
        let _ = c.recv();
        assert!(c.closed());
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
    }

    #[test]
    fn keys_compare_whole() {
        assert!(keys_match(KEY, KEY));
        assert!(!keys_match(KEY, &KEY[..63]));
        assert!(!keys_match(&KEY[..63], KEY));
        assert!(!keys_match(KEY, &KEY.replace('f', "e")));
        assert!(!keys_match("", ""), "an empty key never matches");
    }

    #[test]
    fn random_keys_are_256_bits() {
        let (a, b) = (random_key().unwrap(), random_key().unwrap());
        assert_eq!(a.len(), 64);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("wordcraft-control-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn the_key_file_is_written_after_the_bind_and_removed_on_exit() {
        let dir = temp_dir("life");
        // The port is taken: no key file.
        let taken = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = taken.local_addr().unwrap().port();
        let (tx, rx) = request_channel();
        assert!(start(port, egui::Context::default(), &dir, tx.clone(), false).is_err());
        assert!(!dir.exists() || std::fs::read_dir(&dir).unwrap().next().is_none(), "no key file after a failed bind");
        drop(taken);
        // Bound: the key file holds the key that opens the port.
        let mut server = start(port, egui::Context::default(), &dir, tx, false).unwrap();
        let key_file = server.key_file.clone().unwrap();
        assert_eq!(key_file, wordcraft_control_key::key_path(&dir, port));
        let key = std::fs::read_to_string(&key_file).unwrap();
        assert_eq!(key.len(), 64);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&key_file).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let mut c = Conn::open(port);
        c.send(&format!(r#"{{"id":1,"key":"{key}","method":"document.inspect"}}"#));
        answer(&rx);
        assert_eq!(c.recv()["ok"], true);
        server.stop();
        assert!(!key_file.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stopping_revokes_active_connections_and_queued_requests() {
        let dir = temp_dir("stop");
        let (tx, rx) = request_channel();
        let mut server = start(0, egui::Context::default(), &dir, tx, true).unwrap();
        let key_path = wordcraft_control_key::live_key_path(&dir, server.port);
        let key = std::fs::read_to_string(&key_path).unwrap();
        let mut c = Conn::open(server.port);
        c.send(&format!(r#"{{"id":1,"key":"{key}","method":"agent.call","params":{{}}}}"#));
        let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(!req.cancelled.load(Ordering::Acquire));
        let port = server.port;
        server.stop();
        assert!(req.cancelled.load(Ordering::Acquire));
        assert!(c.closed());
        assert!(!key_path.exists());
        assert!(TcpListener::bind(("127.0.0.1", port)).is_ok());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn disconnected_peer_cancels_queued_request_before_ui_dispatch() {
        let (port, rx) = boot();
        let mut c = Conn::open(port);
        c.send(&format!(r#"{{"id":1,"key":"{KEY}","method":"document.inspect"}}"#));
        let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        c.1.shutdown(Shutdown::Both).unwrap();
        drop(c);
        let deadline = Instant::now() + Duration::from_secs(2);
        while req.runnable() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!req.runnable(), "a disconnected caller must not leave an executable command in the UI queue");
    }

    #[test]
    fn live_peer_eof_notifies_ui_with_host_connection_identity() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = request_channel();
        let _server = serve_on(listener, egui::Context::default(), KEY.into(), FIRST_REQUEST, tx, true).unwrap();
        let mut c = Conn::open(port);
        c.send(&format!(r#"{{"id":1,"key":"{KEY}","method":"agent.status","params":{{"sessionId":"s","documentId":7,"agentName":"Test agent","clientId":"client","_connectionId":999}}}}"#));
        let status = answer(&rx);
        assert_eq!(status.params["_connectionId"], 0, "network input cannot choose the connection id");
        assert_eq!(c.recv()["ok"], true);
        c.1.shutdown(Shutdown::Both).unwrap();
        drop(c);
        let closed = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(closed.method, "agent.transportClosed");
        assert_eq!(closed.params, json!({"sessionId":"s","documentId":7,"agentName":"Test agent","clientId":"client","_connectionId":0}));
        assert!(closed.runnable(), "cleanup has its own live cancellation token");
        assert!(closed.deadline.is_none(), "cleanup stays useful after a long UI stall");
    }

    #[test]
    fn agent_listener_refuses_legacy_methods() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = request_channel();
        let _server = serve_on(listener, egui::Context::default(), KEY.into(), FIRST_REQUEST, tx, true).unwrap();
        let mut c = Conn::open(port);
        assert_eq!(c.call(&format!(r#"{{"id":1,"key":"{KEY}","method":"engine.execute","params":{{"command":"text.insert"}}}}"#))["ok"], false);
        assert!(rx.try_recv().is_err());
        assert_eq!(c.call(&format!(r#"{{"id":3,"key":"{KEY}","method":"agent.transportClosed","params":{{"_connectionId":0}}}}"#))["ok"], false);
        assert!(rx.try_recv().is_err());
        c.send(&format!(r#"{{"id":2,"key":"{KEY}","method":"agent.status"}}"#));
        assert_eq!(answer(&rx).method, "agent.status");
        assert_eq!(c.recv()["ok"], true);
    }

    #[test]
    fn full_queue_refuses_instead_of_blocking_or_adding_requests() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = request_channel();
        for _ in 0..MAX_PENDING {
            let (req, _reply) = ControlRequest::new("document.inspect", json!({}));
            tx.try_send(req).unwrap();
        }
        let _server = serve_on(listener, egui::Context::default(), KEY.into(), FIRST_REQUEST, tx, false).unwrap();
        let mut c = Conn::open(port);
        assert_eq!(c.call(&format!(r#"{{"id":1,"key":"{KEY}","method":"document.inspect"}}"#))["error"], "control queue full or closed");
        assert!(c.closed());
        assert_eq!(rx.try_iter().count(), MAX_PENDING);
    }

    #[test]
    fn runtime_publishes_private_descriptor_and_rotates_session() {
        let dir = temp_dir("session");
        let (tx, _rx) = request_channel();
        let mut runtime = AgentRuntime::default();
        let request = json!({"enabled":true,"documentId":7,"title":"Unsaved document"});
        let first = runtime.update(request.clone(), &egui::Context::default(), &dir, tx.clone()).unwrap();
        let path = runtime.descriptor.clone().unwrap();
        assert_eq!(serde_json::from_str::<Value>(&std::fs::read_to_string(&path).unwrap()).unwrap(), first);
        assert!(first.get("key").is_none());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let second = runtime.update(request, &egui::Context::default(), &dir, tx.clone()).unwrap();
        assert_ne!(first["sessionId"], second["sessionId"]);
        assert!(!path.exists());
        let path = runtime.descriptor.clone().unwrap();
        runtime.update(json!({"enabled":false}), &egui::Context::default(), &dir, tx).unwrap();
        assert!(!path.exists());
        assert!(runtime.server.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn title_refresh_preserves_session_key_and_active_connection() {
        let dir = temp_dir("refresh");
        let (tx, rx) = request_channel();
        let mut runtime = AgentRuntime::default();
        let first = runtime.update(json!({"enabled":true,"documentId":7,"title":"Untitled"}), &egui::Context::default(), &dir, tx.clone()).unwrap();
        let server = runtime.server.as_ref().unwrap();
        let key_path = server.key_file.clone();
        let key = key_path.as_ref().map(|path| std::fs::read_to_string(path).unwrap());
        #[cfg(unix)]
        let socket_path = server.socket_path.clone().unwrap();
        #[cfg(unix)]
        let mut c = Conn::open_unix(&socket_path);
        #[cfg(not(unix))]
        let mut c = Conn::open(server.port);
        #[cfg(unix)]
        {
            let authentication = c.call(r#"{"id":1,"method":"auth"}"#);
            assert_eq!(authentication["ok"], true, "Unix connection authentication was rejected: {authentication}");
        }
        #[cfg(not(unix))]
        {
            assert_eq!(first["transport"], "tcp");
            assert_eq!(first["authScheme"], "hmac-sha256-v1");
            c.live_authenticate_for(key.as_deref().unwrap(), first["sessionId"].as_str().unwrap(), 7);
        }
        let status_params = json!({"sessionId":first["sessionId"],"documentId":7,"agentName":"Refresh test","clientId":"refresh-test"});
        c.send(&json!({"id":3,"method":"agent.status","params":status_params}).to_string());
        let before = answer(&rx);
        assert_eq!(before.method, "agent.status");
        let connection_id = before.params["_connectionId"].as_u64().unwrap();
        assert_eq!(c.recv()["ok"], true);
        let updated = runtime
            .update(json!({"enabled":true,"refresh":true,"documentId":7,"title":"Saved document.docx"}), &egui::Context::default(), &dir, tx.clone())
            .unwrap();
        assert_eq!(first["sessionId"], updated["sessionId"]);
        assert_eq!(first["documentId"], updated["documentId"]);
        assert_eq!(first["authScheme"], updated["authScheme"]);
        assert_eq!(first["address"], updated["address"]);
        assert_eq!(first["socketPath"], updated["socketPath"]);
        #[cfg(unix)]
        {
            assert_eq!(first["transport"], "unix");
            assert!(first.get("address").is_none());
            assert!(key_path.is_none());
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&socket_path).unwrap().permissions().mode() & 0o777, 0o600);
            assert_eq!(std::fs::metadata(socket_path.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
        }
        assert_eq!(updated["title"], "Saved document.docx");
        assert_eq!(key_path.as_ref().map(|path| std::fs::read_to_string(path).unwrap()), key);
        let path = runtime.descriptor.as_ref().unwrap();
        assert_eq!(serde_json::from_str::<Value>(&std::fs::read_to_string(path).unwrap()).unwrap(), updated);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        // Refresh must not revoke a connection that already authenticated against this key.
        c.send(&json!({"id":4,"method":"agent.status","params":status_params}).to_string());
        let after = answer(&rx);
        assert_eq!(after.method, "agent.status");
        assert_eq!(after.params["_connectionId"].as_u64(), Some(connection_id), "refresh must retain the same authenticated transport");
        assert_eq!(c.recv()["ok"], true);
        assert!(
            runtime.update(json!({"enabled":true,"refresh":true,"documentId":8,"title":"Another"}), &egui::Context::default(), &dir, tx).is_err()
        );
        assert_eq!(runtime.metadata.as_ref().unwrap(), &updated);
        assert_eq!(key_path.as_ref().map(|path| std::fs::read_to_string(path).unwrap()), key);
        runtime.stop();
        #[cfg(unix)]
        assert!(!socket_path.exists());
        if let Some(path) = key_path {
            assert!(!path.exists());
        }
        assert_eq!(c.recv(), Value::Null, "stop must close the retained connection");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn exporting_setup_does_not_start_discovery_or_revoke_existing_discovery() {
        let dir = temp_dir("setup-only");
        let (tx, _rx) = request_channel();
        let mut runtime = AgentRuntime::default();
        let setup = runtime.update(json!({"setup":true}), &egui::Context::default(), &dir, tx.clone()).unwrap();
        assert!(setup.get("mcpConfig").is_some());
        assert!(runtime.server.is_none());
        assert!(!dir.exists());
        let descriptor =
            runtime.update(json!({"enabled":true,"documentId":7,"title":"Document"}), &egui::Context::default(), &dir, tx.clone()).unwrap();
        runtime.update(json!({"setup":true}), &egui::Context::default(), &dir, tx).unwrap();
        assert_eq!(runtime.metadata.as_ref().unwrap(), &descriptor);
        assert!(runtime.server.is_some());
        runtime.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_stale_key_file_is_replaced() {
        let dir = temp_dir("stale");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("control-key.test");
        std::fs::write(&p, "old").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        write_key_file(&p, KEY).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), KEY);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

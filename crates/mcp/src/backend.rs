//! Where MCP tool calls end up: a control-channel method call.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

use serde_json::{Value, json};

/// Something that answers control-channel methods (`engine.execute`, `document.inspect`,
/// `ui.pointer`, `ui.render`, …). See `wordcraft_ui_egui::control` for the full list.
pub trait Backend {
    /// Call one method. `Ok` carries the `result`, `Err` the error message.
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String>;
    /// True when a real UI is attached (`ui.screenshot`, `ui.click`, dialogs… work).
    fn has_ui(&self) -> bool;
    /// Short human description ("headless", "connected to 127.0.0.1:7981").
    fn describe(&self) -> String;
    /// Live sessions never create an independent headless document.
    fn is_live(&self) -> bool {
        false
    }
}

/// Looks up the control key for an address (`HOST:PORT`).
type KeyFinder = Box<dyn Fn(&str) -> Result<String, String> + Send>;

enum Stream {
    Tcp(TcpStream),
    #[cfg(unix)]
    Unix(UnixStream),
}
impl Stream {
    fn try_clone(&self) -> std::io::Result<Self> {
        match self {
            Self::Tcp(s) => s.try_clone().map(Self::Tcp),
            #[cfg(unix)]
            Self::Unix(s) => s.try_clone().map(Self::Unix),
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
    #[cfg(test)]
    fn read_timeout(&self) -> std::io::Result<Option<Duration>> {
        match self {
            Self::Tcp(s) => s.read_timeout(),
            #[cfg(unix)]
            Self::Unix(s) => s.read_timeout(),
        }
    }
    #[cfg(test)]
    fn write_timeout(&self) -> std::io::Result<Option<Duration>> {
        match self {
            Self::Tcp(s) => s.write_timeout(),
            #[cfg(unix)]
            Self::Unix(s) => s.write_timeout(),
        }
    }
    /// No bytes are sent or consumed. EOF, errors and unsolicited data invalidate the socket.
    fn healthy_before_send(&self) -> bool {
        let mut byte = [0u8; 1];
        let result = match self {
            Self::Tcp(s) => {
                if s.set_nonblocking(true).is_err() {
                    return false;
                }
                let result = s.peek(&mut byte);
                if s.set_nonblocking(false).is_err() {
                    return false;
                }
                result
            }
            #[cfg(unix)]
            Self::Unix(s) => rustix::net::recv(s, &mut byte[..], rustix::net::RecvFlags::PEEK | rustix::net::RecvFlags::DONTWAIT)
                .map(|(_, size)| size)
                .map_err(std::io::Error::from),
        };
        matches!(result, Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    }
}
impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Tcp(s) => s.read(buf),
            #[cfg(unix)]
            Self::Unix(s) => s.read(buf),
        }
    }
}
impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Tcp(s) => s.write(buf),
            #[cfg(unix)]
            Self::Unix(s) => s.write(buf),
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

#[cfg(unix)]
fn connect_unix_bounded(path: &Path, budget: Duration) -> std::io::Result<UnixStream> {
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    use rustix::io::{FdFlags, fcntl_setfd};
    use rustix::net::{AddressFamily, SocketAddrUnix, SocketType, connect, socket};
    // Apple does not expose SOCK_NONBLOCK/SOCK_CLOEXEC. Configure a plain socket through
    // portable fd/std APIs before connecting instead of relying on socket_with flags.
    let descriptor = socket(AddressFamily::UNIX, SocketType::STREAM, None)?;
    fcntl_setfd(&descriptor, FdFlags::CLOEXEC)?;
    let stream = UnixStream::from(descriptor);
    stream.set_nonblocking(true)?;
    // From<OwnedFd> bypasses std's socket constructor. On platforms without MSG_NOSIGNAL,
    // preserve its SO_NOSIGPIPE protection so a lost peer returns an error, never a signal.
    #[cfg(any(target_vendor = "apple", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd"))]
    rustix::net::sockopt::set_socket_nosigpipe(&stream, true)?;
    let address = SocketAddrUnix::new(path)?;
    match connect(&stream, &address) {
        Ok(()) => {}
        Err(error) if error == rustix::io::Errno::INPROGRESS || error == rustix::io::Errno::AGAIN => {
            let mut fds = [PollFd::new(&stream, PollFlags::OUT)];
            let nanos = i64::try_from(budget.min(Duration::from_millis(800)).as_nanos())
                .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid Unix connection timeout"))?;
            let timeout = Timespec { tv_sec: 0, tv_nsec: nanos };
            if poll(&mut fds, Some(&timeout))? == 0 {
                return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "Unix socket connection timed out"));
            }
            rustix::net::sockopt::socket_error(&stream)??;
        }
        Err(error) => return Err(error.into()),
    }
    stream.peer_addr()?;
    stream.set_nonblocking(false)?;
    Ok(stream)
}

/// A running WordCraft app, reached through its loopback control port.
pub struct Remote {
    addr: String,
    conn: Option<(BufReader<Stream>, Stream)>,
    #[cfg(unix)]
    unix_path: Option<PathBuf>,
    next_id: u64,
    /// The connection leads to a loopback address: requests carry the control key.
    send_key: bool,
    /// The control key, looked up once per connection (the app picks a new one at every start).
    key: Option<String>,
    find_key: KeyFinder,
    timeout: Option<Duration>,
    /// A discovered TCP window must prove this exact identity before receiving any RPC.
    live_identity: Option<(String, u64)>,
    connect_budget: Duration,
}

/// The control key: `env` (`WORDCRAFT_CONTROL_KEY`) when set, else the app's key file for
/// `port` in `settings`. The error says where it looked.
fn lookup_key(env: Option<String>, settings: Option<&Path>, port: Option<u16>) -> Result<String, String> {
    if let Some(k) = env.map(|k| k.trim().to_string()).filter(|k| !k.is_empty()) {
        return Ok(k);
    }
    let (Some(settings), Some(port)) = (settings, port) else {
        return Err("no control key: set WORDCRAFT_CONTROL_KEY (no settings folder or port to find the key file)".to_string());
    };
    let path = wordcraft_control_key::key_path(settings, port);
    match std::fs::read_to_string(&path).map(|k| k.trim().to_string()) {
        Ok(k) if !k.is_empty() => Ok(k),
        _ => Err(format!("no control key: set WORDCRAFT_CONTROL_KEY, or start the app with --control {port} (it writes {})", path.display())),
    }
}

/// The key for the app at `addr`, from the environment or the app's key file.
fn find_key(addr: &str) -> Result<String, String> {
    let port = addr.rsplit_once(':').and_then(|(_, p)| p.parse().ok());
    lookup_key(std::env::var("WORDCRAFT_CONTROL_KEY").ok(), wordcraft_control_key::settings_dir().as_deref(), port)
}

/// Whether `addr` (`HOST:PORT`, as given to `--connect`) names a loopback address: 127.0.0.0/8,
/// ::1 or `localhost`. Other host names are not resolved: they never count.
fn is_loopback_addr(addr: &str) -> bool {
    let loopback = |ip: IpAddr| ip.to_canonical().is_loopback();
    if let Ok(sa) = addr.parse::<SocketAddr>() {
        return loopback(sa.ip());
    }
    if let Ok(ip) = addr.parse::<IpAddr>() {
        return loopback(ip);
    }
    let host = addr.rsplit_once(':').map_or(addr, |(h, _)| h);
    let host = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')).unwrap_or(host);
    host.parse::<IpAddr>().map_or_else(|_| host.eq_ignore_ascii_case("localhost"), loopback)
}

/// The key goes only to a loopback `--connect` address, and only when the connection really
/// leads to one (`peer`, so a `localhost` that resolves elsewhere gets no key either).
fn may_send_key(addr: &str, peer: Option<SocketAddr>) -> bool {
    is_loopback_addr(addr) && peer.is_some_and(|p| p.ip().to_canonical().is_loopback())
}

impl Remote {
    /// Connect to `addr` (`127.0.0.1:7981`), failing fast when nothing is listening.
    pub fn connect(addr: &str) -> std::io::Result<Self> {
        Self::connect_with(addr, Box::new(find_key))
    }

    pub(crate) fn connect_with(addr: &str, find_key: KeyFinder) -> std::io::Result<Self> {
        let mut r = Self {
            addr: addr.to_string(),
            conn: None,
            next_id: 1,
            send_key: false,
            key: None,
            find_key,
            timeout: None,
            live_identity: None,
            connect_budget: Duration::from_millis(2800),
            #[cfg(unix)]
            unix_path: None,
        };
        r.reconnect()?;
        if let Some(w) = r.key_warning() {
            log::warn!("{w}");
        }
        Ok(r)
    }

    pub(crate) fn connect_live_tcp(addr: &str, find_key: KeyFinder, session: &str, document: u64) -> std::io::Result<Self> {
        Self::connect_live_tcp_with_budget(addr, find_key, session, document, Duration::from_millis(2800))
    }

    pub(crate) fn connect_live_tcp_with_budget(
        addr: &str,
        find_key: KeyFinder,
        session: &str,
        document: u64,
        budget: Duration,
    ) -> std::io::Result<Self> {
        let endpoint = addr
            .parse::<SocketAddr>()
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "live TCP requires a numeric loopback endpoint"))?;
        if !endpoint.ip().to_canonical().is_loopback() {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "live TCP requires a loopback endpoint"));
        }
        let mut remote = Self {
            addr: addr.to_owned(),
            conn: None,
            next_id: 1,
            send_key: false,
            key: None,
            find_key,
            timeout: None,
            live_identity: Some((session.to_owned(), document)),
            connect_budget: budget.min(Duration::from_millis(2800)),
            #[cfg(unix)]
            unix_path: None,
        };
        remote.reconnect()?;
        Ok(remote)
    }

    #[cfg(unix)]
    pub(crate) fn connect_unix(path: &Path) -> std::io::Result<Self> {
        Self::connect_unix_with_budget(path, Duration::from_millis(800))
    }

    #[cfg(unix)]
    pub(crate) fn connect_unix_with_budget(path: &Path, budget: Duration) -> std::io::Result<Self> {
        let mut remote = Self {
            addr: path.to_string_lossy().into_owned(),
            conn: None,
            unix_path: Some(path.to_path_buf()),
            next_id: 1,
            send_key: false,
            key: None,
            find_key: Box::new(|_| Err("Unix live sockets use filesystem access, never a TCP key".into())),
            timeout: None,
            live_identity: None,
            connect_budget: budget.min(Duration::from_millis(800)),
        };
        remote.reconnect()?;
        Ok(remote)
    }

    pub(crate) fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = Some(timeout);
        if let Some((reader, writer)) = self.conn.as_ref() {
            let _ = reader.get_ref().set_read_timeout(Some(timeout));
            let _ = writer.set_write_timeout(Some(timeout));
        }
    }

    pub fn addr(&self) -> &str {
        &self.addr
    }

    /// Why requests go without the control key (the address is not loopback), if they do.
    pub fn key_warning(&self) -> Option<String> {
        if self.live_identity.is_some() {
            return None;
        }
        #[cfg(unix)]
        if self.unix_path.is_some() {
            return None;
        }
        (!self.send_key).then(|| {
            format!(
                "{} is not a loopback address, so the control key is not sent (it only goes to 127.0.0.0/8, ::1 and localhost) and the app refuses requests; reach the app through 127.0.0.1, for example with an SSH tunnel",
                self.addr
            )
        })
    }

    fn reconnect(&mut self) -> std::io::Result<()> {
        self.conn = None;
        self.key = None;
        let deadline = Instant::now()
            .checked_add(self.connect_budget)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid connection deadline"))?;
        let remaining = || {
            deadline
                .checked_duration_since(Instant::now())
                .filter(|t| !t.is_zero())
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::TimedOut, "live connection deadline expired"))
        };
        #[cfg(unix)]
        if let Some(path) = &self.unix_path {
            self.send_key = false;
            let stream = Stream::Unix(connect_unix_bounded(path, remaining()?)?);
            stream.set_read_timeout(Some(self.timeout.unwrap_or(Duration::from_secs(90))))?;
            stream.set_write_timeout(Some(self.timeout.unwrap_or(Duration::from_secs(10))))?;
            self.conn = Some((BufReader::new(stream.try_clone()?), stream));
            return Ok(());
        }
        let mut last = std::io::Error::new(std::io::ErrorKind::NotFound, format!("cannot resolve {}", self.addr));
        for sa in self.addr.to_socket_addrs()? {
            // Windows can report a loopback refusal after the legacy 800 ms limit.
            // Live discovery supplies an absolute sweep budget; use its remainder
            // so a definite refusal can be observed without extending that sweep.
            let budget = if self.live_identity.is_some() { remaining()? } else { Duration::from_millis(800) };
            match TcpStream::connect_timeout(&sa, budget) {
                Ok(s) => {
                    s.set_nodelay(true).ok();
                    // The app answers within 60 s (its own timeout); leave headroom.
                    s.set_read_timeout(Some(self.timeout.unwrap_or(Duration::from_secs(90))))?;
                    s.set_write_timeout(Some(self.timeout.unwrap_or(Duration::from_secs(10))))?;
                    self.send_key = may_send_key(&self.addr, s.peer_addr().ok());
                    let stream = Stream::Tcp(s);
                    let read = stream.try_clone()?;
                    self.conn = Some((BufReader::new(read), stream));
                    if self.live_identity.is_some() {
                        if let Err(error) = self.authenticate_live(deadline) {
                            self.conn = None;
                            return Err(error);
                        }
                        // Live TCP authentication uses proofs, never a raw key in RPCs.
                        self.send_key = false;
                    }
                    return Ok(());
                }
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn authenticate_live(&mut self, connection_deadline: Instant) -> std::io::Result<()> {
        let fail = |message: String| std::io::Error::new(std::io::ErrorKind::PermissionDenied, format!("app_identity_unverified: {message}"));
        if !self.send_key {
            return Err(fail("live authentication requires a numeric loopback peer".into()));
        }
        let (session, document) = self.live_identity.clone().ok_or_else(|| fail("missing live identity".into()))?;
        let key = (self.find_key)(&self.addr).map_err(fail)?;
        let nonce = wordcraft_control_key::fresh_nonce().map_err(fail)?;
        let deadline = Instant::now().checked_add(Duration::from_secs(2)).ok_or_else(|| fail("invalid handshake deadline".into()))?;
        let deadline = deadline.min(connection_deadline);
        let challenge =
            self.handshake_roundtrip("auth.challenge", json!({"clientNonce":nonce,"sessionId":session,"documentId":document}), deadline)?;
        let server_nonce = challenge.get("serverNonce").and_then(Value::as_str).ok_or_else(|| fail("server nonce missing".into()))?;
        let proof = challenge.get("proof").and_then(Value::as_str).ok_or_else(|| fail("server proof missing".into()))?;
        wordcraft_control_key::verify_live_auth_proof(&key, "server", &session, document, &nonce, server_nonce, proof).map_err(fail)?;
        let client_proof = wordcraft_control_key::live_auth_proof(&key, "client", &session, document, &nonce, server_nonce).map_err(fail)?;
        let accepted = self.handshake_roundtrip("auth.prove", json!({"proof":client_proof}), deadline)?;
        if accepted.get("authenticated").and_then(Value::as_bool) != Some(true)
            || accepted.get("sessionId").and_then(Value::as_str) != Some(session.as_str())
            || accepted.get("documentId").and_then(Value::as_u64) != Some(document)
        {
            return Err(fail("app did not confirm the expected session and document".into()));
        }
        if let Some((reader, writer)) = &self.conn {
            reader.get_ref().set_read_timeout(Some(self.timeout.unwrap_or(Duration::from_secs(90))))?;
            writer.set_write_timeout(Some(self.timeout.unwrap_or(Duration::from_secs(10))))?;
        }
        Ok(())
    }

    #[cfg(target_arch = "wasm32")]
    fn authenticate_live(&mut self, _connection_deadline: Instant) -> std::io::Result<()> {
        Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "native live TCP authentication is unavailable in the browser"))
    }

    /// Handshake frames have a small cap and a deadline for the entire exchange, including
    /// slow partial lines. Socket per-read timeouts alone would restart on every byte.
    #[cfg(not(target_arch = "wasm32"))]
    fn handshake_roundtrip(&mut self, method: &str, params: Value, deadline: Instant) -> std::io::Result<Value> {
        let invalid = |message: &str| std::io::Error::new(std::io::ErrorKind::PermissionDenied, format!("app_identity_unverified: {message}"));
        let remaining = || {
            deadline
                .checked_duration_since(Instant::now())
                .filter(|t| !t.is_zero())
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::TimedOut, "live authentication timed out"))
        };
        let Some((reader, writer)) = self.conn.as_mut() else { return Err(invalid("connection unavailable")) };
        writer.set_write_timeout(Some(remaining()?))?;
        writeln!(writer, "{}", json!({"id":0,"method":method,"params":params}))?;
        let mut bytes = Vec::new();
        loop {
            reader.get_ref().set_read_timeout(Some(remaining()?))?;
            let buffer = reader.fill_buf()?;
            if buffer.is_empty() {
                return Err(invalid("peer closed during authentication"));
            }
            let newline = buffer.iter().position(|b| *b == b'\n');
            let used = newline.map_or(buffer.len(), |i| i.saturating_add(1));
            if bytes.len().saturating_add(used) > 4096 {
                return Err(invalid("authentication reply exceeds 4 KiB"));
            }
            bytes.extend_from_slice(buffer.get(..used).ok_or_else(|| invalid("invalid reply framing"))?);
            reader.consume(used);
            if newline.is_some() {
                break;
            }
        }
        remaining()?;
        let reply: Value = serde_json::from_slice(&bytes).map_err(|_| invalid("authentication reply is not JSON"))?;
        if reply.get("id").and_then(Value::as_u64) != Some(0) || reply.get("ok").and_then(Value::as_bool) != Some(true) {
            return Err(invalid("peer rejected authentication"));
        }
        reply.get("result").cloned().ok_or_else(|| invalid("authentication result missing"))
    }

    /// The request line; it carries the key when this connection may (looked up once per
    /// connection).
    fn request_line(&mut self, id: u64, method: &str, params: &Value) -> Result<String, String> {
        let mut req = json!({"id": id, "method": method, "params": params});
        if self.send_key {
            let key = match &self.key {
                Some(k) => k.clone(),
                None => (self.find_key)(&self.addr)?,
            };
            req["key"] = json!(key);
            self.key = Some(key);
        }
        Ok(req.to_string())
    }

    fn roundtrip(&mut self, line: &str) -> std::io::Result<String> {
        let Some((reader, writer)) = self.conn.as_mut() else {
            return Err(std::io::Error::new(std::io::ErrorKind::NotConnected, "not connected"));
        };
        writer.write_all(line.as_bytes())?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        let mut reply = String::new();
        const MAX_REPLY: u64 = 16 * 1024 * 1024;
        if reader.take(MAX_REPLY + 1).read_line(&mut reply)? == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "control channel closed"));
        }
        if reply.len() as u64 > MAX_REPLY || !reply.ends_with('\n') {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "control reply exceeds limit or is not newline terminated"));
        }
        Ok(reply)
    }

    /// Never replay a request after sending starts: the app may already have applied it.
    fn exchange(&mut self, id: u64, method: &str, params: &Value) -> Result<Value, String> {
        if self.conn.as_ref().is_some_and(|(reader, writer)| !reader.buffer().is_empty() || !writer.healthy_before_send()) {
            // The current request has not been sent, so replacing a stale idle socket is safe.
            self.conn = None;
        }
        if self.conn.is_none() {
            self.reconnect().map_err(|e| format!("WordCraft app at {} is not reachable: {e}", self.addr))?;
        }
        let line = self.request_line(id, method, params)?;
        if line.len().saturating_add(1) > 1024 * 1024 {
            return Err("request exceeds the 1 MiB control limit; split large edits before retrying".into());
        }
        let reply = self.roundtrip(&line).map_err(|e| {
            self.conn = None;
            format!("outcome_unknown: WordCraft may have performed `{method}`; request was not replayed. Read the document before any retry: {e}")
        })?;
        let parsed: Value = serde_json::from_str(reply.trim()).map_err(|e| {
            self.conn = None;
            format!("outcome_unknown: invalid reply from app; request was not replayed: {e}")
        })?;
        if parsed.get("id").and_then(Value::as_u64) != Some(id) {
            self.conn = None;
            return Err("outcome_unknown: control reply id does not match request; request was not replayed".into());
        }
        Ok(parsed)
    }
}

impl Backend for Remote {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).ok_or("control request id exhausted")?;
        let unauthorized =
            |v: &Value| v.get("ok").and_then(Value::as_bool) == Some(false) && v.get("error").and_then(Value::as_str) == Some("unauthorized");
        let mut v = self.exchange(id, method, &params)?;
        if unauthorized(&v) && self.send_key {
            // A stale key (the app restarted on this port): the app closed the connection; a new
            // one reads the key again. Once.
            self.conn = None;
            v = self.exchange(id, method, &params)?;
        }
        if v.get("ok").and_then(Value::as_bool) == Some(true) {
            Ok(v.get("result").cloned().unwrap_or(Value::Null))
        } else if unauthorized(&v) {
            self.conn = None;
            Err(self.key_warning().map_or_else(
                || {
                    format!(
                        "unauthorized: the WordCraft app at {} refused the control key (from WORDCRAFT_CONTROL_KEY or the app's key file)",
                        self.addr
                    )
                },
                |w| format!("unauthorized: {w}"),
            ))
        } else {
            Err(v.get("error").and_then(Value::as_str).unwrap_or("unknown error").to_string())
        }
    }

    fn has_ui(&self) -> bool {
        true
    }

    fn describe(&self) -> String {
        format!("connected to the WordCraft app at {}", self.addr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{SocketAddr, TcpListener};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::{Receiver, channel};

    const LIVE_KEY: &str = "abababababababababababababababababababababababababababababababab";

    #[test]
    fn live_tcp_reused_port_cannot_capture_key_or_receive_document_calls() {
        for reflected in [false, true] {
            let (port, seen) = fake_app(move |request| {
                let nonce = request["params"]["clientNonce"].as_str().unwrap();
                let server_nonce = "cd".repeat(32);
                let proof = if reflected {
                    wordcraft_control_key::live_auth_proof(LIVE_KEY, "client", "window", 7, nonce, &server_nonce).unwrap()
                } else {
                    "00".repeat(32)
                };
                Some((json!({"ok":true,"result":{"serverNonce":server_nonce,"proof":proof}}), false))
            });
            let result = Remote::connect_live_tcp(&format!("127.0.0.1:{port}"), Box::new(|_| Ok(LIVE_KEY.into())), "window", 7);
            let error = result.err().unwrap();
            assert!(error.to_string().contains("app_identity_unverified"), "{error}");
            let challenge = seen.recv_timeout(Duration::from_secs(1)).unwrap();
            assert_eq!(challenge["method"], "auth.challenge");
            assert!(challenge.get("key").is_none());
            assert!(!challenge.to_string().contains(LIVE_KEY));
            assert!(seen.try_recv().is_err(), "no client proof or document call before server identity");
        }
    }

    #[test]
    fn malformed_handshake_frames_never_receive_a_client_proof_or_document_rpc() {
        for reply in ["not JSON\n".to_string(), "{\"id\":9,\"ok\":true,\"result\":{}}\n".to_string(), format!("{}\n", "x".repeat(5000))] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap().to_string();
            let worker = std::thread::spawn(move || {
                let (stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut writer = stream.try_clone().unwrap();
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let challenge: Value = serde_json::from_str(&line).unwrap();
                assert_eq!(challenge["method"], "auth.challenge");
                assert!(challenge.get("key").is_none());
                let _ = writer.write_all(reply.as_bytes());
                line.clear();
                assert_eq!(reader.read_line(&mut line).unwrap(), 0);
            });
            let result = Remote::connect_live_tcp(&address, Box::new(|_| Ok(LIVE_KEY.into())), "window", 7);
            assert!(result.is_err());
            worker.join().unwrap();
        }
    }

    #[test]
    fn final_authenticated_identity_mismatch_is_not_reported_connected() {
        let (port, seen) = fake_app(|request| {
            let result = if request["method"] == "auth.challenge" {
                let server_nonce = "cd".repeat(32);
                let proof = wordcraft_control_key::live_auth_proof(
                    LIVE_KEY,
                    "server",
                    "window",
                    7,
                    request["params"]["clientNonce"].as_str().unwrap(),
                    &server_nonce,
                )
                .unwrap();
                json!({"serverNonce":server_nonce,"proof":proof})
            } else {
                json!({"authenticated":true,"sessionId":"window","documentId":8})
            };
            Some((json!({"ok":true,"result":result}), false))
        });
        let result = Remote::connect_live_tcp(&format!("127.0.0.1:{port}"), Box::new(|_| Ok(LIVE_KEY.into())), "window", 7);
        assert!(result.err().unwrap().to_string().contains("expected session and document"));
        let requests: Vec<_> = seen.try_iter().collect();
        assert_eq!(requests.len(), 2);
        assert!(requests.iter().all(|r| r["method"].as_str().unwrap().starts_with("auth.")));
    }

    #[test]
    fn live_tcp_authenticates_again_on_idle_reconnect_and_never_replays_mutation() {
        let nonces = std::sync::Mutex::new(None::<(String, String)>);
        let (port, seen) = fake_app(move |request| {
            assert!(request.get("key").is_none());
            match request["method"].as_str().unwrap() {
                "auth.challenge" => {
                    assert_eq!(request["params"]["sessionId"], "window");
                    assert_eq!(request["params"]["documentId"], 7);
                    let client = request["params"]["clientNonce"].as_str().unwrap().to_string();
                    let server = wordcraft_control_key::fresh_nonce().unwrap();
                    let proof = wordcraft_control_key::live_auth_proof(LIVE_KEY, "server", "window", 7, &client, &server).unwrap();
                    *nonces.lock().unwrap() = Some((client, server.clone()));
                    Some((json!({"ok":true,"result":{"serverNonce":server,"proof":proof}}), false))
                }
                "auth.prove" => {
                    let pair = nonces.lock().unwrap().take().unwrap();
                    wordcraft_control_key::verify_live_auth_proof(
                        LIVE_KEY,
                        "client",
                        "window",
                        7,
                        &pair.0,
                        &pair.1,
                        request["params"]["proof"].as_str().unwrap(),
                    )
                    .unwrap();
                    Some((json!({"ok":true,"result":{"authenticated":true,"sessionId":"window","documentId":7}}), false))
                }
                "agent.status" => Some((json!({"ok":true,"result":{"enabled":true}}), true)),
                "agent.call" => None,
                _ => panic!("unexpected method"),
            }
        });
        let mut remote = Remote::connect_live_tcp(&format!("127.0.0.1:{port}"), Box::new(|_| Ok(LIVE_KEY.into())), "window", 7).unwrap();
        assert!(remote.key_warning().is_none());
        assert_eq!(remote.call("agent.status", json!({})).unwrap()["enabled"], true);
        let error =
            remote.call("agent.call", json!({"method":"engine.execute","params":{"command":"text.insert","params":{"text":"once"}}})).unwrap_err();
        assert!(error.starts_with("outcome_unknown"), "{error}");
        let requests: Vec<_> = seen.try_iter().collect();
        assert_eq!(requests.iter().filter(|r| r["method"] == "auth.challenge").count(), 2);
        assert_ne!(requests.first().unwrap()["params"]["clientNonce"], requests.get(3).unwrap()["params"]["clientNonce"]);
        assert_eq!(requests.iter().filter(|r| r["method"] == "agent.call").count(), 1);
    }

    #[test]
    fn live_tcp_slow_authentication_has_an_absolute_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let worker = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut writer = stream.try_clone().unwrap();
            let mut reader = BufReader::new(stream);
            let mut challenge = String::new();
            reader.read_line(&mut challenge).unwrap();
            for _ in 0..30 {
                if writer.write_all(b" ").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        let started = Instant::now();
        let result = Remote::connect_live_tcp(&address, Box::new(|_| Ok(LIVE_KEY.into())), "window", 7);
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(3));
        worker.join().unwrap();
    }

    #[test]
    fn key_from_env_then_key_file() {
        let dir = std::env::temp_dir().join(format!("wordcraft-mcp-key-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = wordcraft_control_key::key_path(&dir, 7981);
        // Neither: the error names the variable and the file it looked for.
        let e = lookup_key(None, Some(&dir), Some(7981)).unwrap_err();
        assert!(e.contains("WORDCRAFT_CONTROL_KEY") && e.contains(&path.display().to_string()), "{e}");
        // The key file, trimmed; an empty variable does not count.
        std::fs::write(&path, "  file-key\n").unwrap();
        assert_eq!(lookup_key(None, Some(&dir), Some(7981)).unwrap(), "file-key");
        assert_eq!(lookup_key(Some(" ".into()), Some(&dir), Some(7981)).unwrap(), "file-key");
        // The variable wins.
        assert_eq!(lookup_key(Some(" env-key ".into()), Some(&dir), Some(7981)).unwrap(), "env-key");
        assert_eq!(lookup_key(Some("env-key".into()), None, None).unwrap(), "env-key");
        // Read again every time: the app writes a new key at every start.
        std::fs::write(&path, "new-key").unwrap();
        assert_eq!(lookup_key(None, Some(&dir), Some(7981)).unwrap(), "new-key");
        // Another port, another file.
        assert!(lookup_key(None, Some(&dir), Some(7982)).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn key_only_for_loopback_addresses() {
        let peer = |s: &str| s.parse::<SocketAddr>().ok();
        for (addr, at) in [
            ("127.0.0.1:7981", "127.0.0.1:7981"),
            ("127.0.0.2:7981", "127.0.0.2:7981"),
            ("[::1]:7981", "[::1]:7981"),
            ("[::ffff:127.0.0.1]:7981", "[::ffff:127.0.0.1]:7981"),
            ("localhost:7981", "127.0.0.1:7981"),
            ("localhost:7981", "[::1]:7981"),
            ("LocalHost:7981", "127.0.0.1:7981"),
        ] {
            assert!(may_send_key(addr, peer(at)), "{addr} at {at}");
        }
        for (addr, at) in [
            ("192.0.2.20:7981", "192.0.2.20:7981"),
            ("10.0.0.7:7981", "10.0.0.7:7981"),
            ("[fe80::1]:7981", "[fe80::1]:7981"),
            ("0.0.0.0:7981", "127.0.0.1:7981"),
            // Host names other than localhost: even when they lead to this machine.
            ("workstation:7981", "127.0.1.1:7981"),
            ("example.org:7981", "93.184.215.14:7981"),
            ("localhost.example.org:7981", "127.0.0.1:7981"),
            // `localhost` that does not lead to a loopback address.
            ("localhost:7981", "192.0.2.20:7981"),
        ] {
            assert!(!may_send_key(addr, peer(at)), "{addr} at {at}");
        }
        // No peer address: no key.
        assert!(!may_send_key("127.0.0.1:7981", None));
    }

    /// A stand-in for the app on 127.0.0.1: every request line goes to the returned channel and
    /// gets `answer(request)`: a reply and whether to close the connection after it, or `None`
    /// to close it without a reply (the app quit).
    fn fake_app(answer: impl Fn(&Value) -> Option<(Value, bool)> + Send + Sync + 'static) -> (u16, Receiver<Value>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = channel();
        let answer = Arc::new(answer);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (tx, answer) = (tx.clone(), answer.clone());
                std::thread::spawn(move || {
                    let mut out = stream.try_clone().unwrap();
                    for line in BufReader::new(stream).lines() {
                        let Ok(req) = serde_json::from_str::<Value>(&line.unwrap_or_default()) else { return };
                        let _ = tx.send(req.clone());
                        let Some((mut reply, close)) = answer(&req) else { return };
                        reply["id"] = req["id"].clone();
                        if writeln!(out, "{reply}").is_err() || close {
                            return;
                        }
                    }
                });
            }
        });
        (port, rx)
    }

    fn fixed_key(key: &'static str) -> KeyFinder {
        Box::new(move |_| Ok(key.to_string()))
    }

    const WAIT: Duration = Duration::from_secs(5);

    #[test]
    fn sends_the_key_to_loopback() {
        let (port, seen) = fake_app(|_| Some((json!({"ok": true, "result": 7}), false)));
        for host in ["127.0.0.1", "localhost"] {
            let mut r = Remote::connect_with(&format!("{host}:{port}"), fixed_key("k1")).unwrap();
            assert!(r.key_warning().is_none(), "{host}");
            assert_eq!(r.call("document.inspect", json!({})).unwrap(), json!(7));
            assert_eq!(seen.recv_timeout(WAIT).unwrap()["key"], "k1", "{host}");
        }
    }

    #[test]
    fn reads_the_key_again_on_a_new_connection() {
        // The app restarted with a new key: the old key is refused (and the connection closed).
        let (port, seen) = fake_app(|req| {
            Some(if req["key"] == "new" { (json!({"ok": true, "result": 1}), false) } else { (json!({"ok": false, "error": "unauthorized"}), true) })
        });
        let reads = Arc::new(AtomicUsize::new(0));
        let n = reads.clone();
        let finder: KeyFinder = Box::new(move |_| Ok(if n.fetch_add(1, Ordering::SeqCst) == 0 { "old" } else { "new" }.to_string()));
        let mut r = Remote::connect_with(&format!("127.0.0.1:{port}"), finder).unwrap();
        assert_eq!(r.call("document.inspect", json!({})).unwrap(), json!(1));
        assert_eq!(seen.recv_timeout(WAIT).unwrap()["key"], "old");
        assert_eq!(seen.recv_timeout(WAIT).unwrap()["key"], "new");
        assert_eq!(reads.load(Ordering::SeqCst), 2);
        // The new key is kept for the next requests on the same connection.
        assert_eq!(r.call("document.inspect", json!({})).unwrap(), json!(1));
        assert_eq!(reads.load(Ordering::SeqCst), 2);

        // A lost reply is never replayed; the next independent call reads the key again.
        let (port, seen) = fake_app(|req| if req["id"] == 2 { None } else { Some((json!({"ok": true, "result": 1}), false)) });
        let reads = Arc::new(AtomicUsize::new(0));
        let n = reads.clone();
        let finder: KeyFinder = Box::new(move |_| Ok(format!("key-{}", n.fetch_add(1, Ordering::SeqCst))));
        let mut r = Remote::connect_with(&format!("127.0.0.1:{port}"), finder).unwrap();
        r.call("document.inspect", json!({})).unwrap();
        assert!(r.call("document.inspect", json!({})).unwrap_err().contains("outcome_unknown"));
        r.call("document.inspect", json!({})).unwrap();
        let keys: Vec<Value> = seen.try_iter().map(|v| v["key"].clone()).collect();
        assert_eq!(keys.first(), Some(&json!("key-0")), "{keys:?}");
        assert_eq!(keys.last(), Some(&json!("key-1")), "{keys:?}");
    }

    #[test]
    fn cleanup_timeout_survives_reconnect() {
        let (port, _) = fake_app(|_| Some((json!({"ok":true,"result":{}}), false)));
        let mut remote = Remote::connect_with(&format!("127.0.0.1:{port}"), fixed_key("key")).unwrap();
        remote.conn = None;
        remote.set_timeout(Duration::from_secs(1));
        remote.reconnect().unwrap();
        let (reader, writer) = remote.conn.as_ref().unwrap();
        assert_eq!(reader.get_ref().read_timeout().unwrap(), Some(Duration::from_secs(1)));
        assert_eq!(writer.write_timeout().unwrap(), Some(Duration::from_secs(1)));
    }

    #[cfg(unix)]
    #[test]
    fn unix_connection_is_close_on_exec_and_restored_to_blocking_io() {
        use std::os::unix::fs::DirBuilderExt;
        use std::os::unix::net::UnixListener;
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        // macOS's normal per-user temp path can exceed sockaddr_un's length limit.
        let dir = std::path::PathBuf::from("/tmp").join(format!("wc-fd-{:x}-{nonce:x}", std::process::id()));
        std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        let path = dir.join("portable.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let mut client = connect_unix_bounded(&path, Duration::from_millis(800)).unwrap();
        assert!(rustix::io::fcntl_getfd(&client).unwrap().contains(rustix::io::FdFlags::CLOEXEC));
        #[cfg(any(target_vendor = "apple", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd"))]
        assert!(rustix::net::sockopt::socket_nosigpipe(&client).unwrap());
        let (mut peer, _) = listener.accept().unwrap();
        client.set_read_timeout(Some(WAIT)).unwrap();
        let (started_tx, started) = channel();
        let (result_tx, result) = channel();
        let reader = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            let mut byte = [0u8; 1];
            result_tx.send(client.read_exact(&mut byte).map(|()| byte)).unwrap();
        });
        started.recv_timeout(WAIT).unwrap();
        // A socket accidentally left nonblocking returns WouldBlock here instead of waiting.
        assert!(matches!(result.recv_timeout(Duration::from_millis(100)), Err(std::sync::mpsc::RecvTimeoutError::Timeout)));
        peer.write_all(b"x").unwrap();
        assert_eq!(result.recv_timeout(WAIT).unwrap().unwrap(), *b"x");
        reader.join().unwrap();
        drop(peer);
        drop(listener);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn idle_eof_is_reconnected_before_sending_the_next_mutation() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, seen) = channel();
        let (closed_tx, closed) = channel();
        let worker = std::thread::spawn(move || {
            for i in 0..2 {
                let (stream, _) = listener.accept().unwrap();
                let mut writer = stream.try_clone().unwrap();
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let request: Value = serde_json::from_str(&line).unwrap();
                tx.send(request.clone()).unwrap();
                writeln!(writer, "{}", json!({"id":request["id"],"ok":true,"result":i})).unwrap();
                drop(writer);
                drop(reader);
                if i == 0 {
                    closed_tx.send(()).unwrap();
                }
            }
        });
        let mut remote = Remote::connect_with(&format!("127.0.0.1:{port}"), fixed_key("key")).unwrap();
        assert_eq!(remote.call("document.inspect", json!({})).unwrap(), 0);
        closed.recv_timeout(WAIT).unwrap();
        let deadline = std::time::Instant::now() + WAIT;
        while remote.conn.as_ref().is_some_and(|(_, stream)| stream.healthy_before_send()) {
            assert!(std::time::Instant::now() < deadline, "FIN must become visible before mutation");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(remote.call("engine.execute", json!({"command":"text.insert","params":{"text":"once"}})).unwrap(), 1);
        worker.join().unwrap();
        let requests: Vec<_> = seen.try_iter().collect();
        assert_eq!(requests.iter().filter(|r| r["method"] == "engine.execute").count(), 1);
    }

    #[test]
    fn oversized_request_is_refused_before_transmission() {
        let (port, seen) = fake_app(|_| Some((json!({"ok":true,"result":{}}), false)));
        let mut remote = Remote::connect_with(&format!("127.0.0.1:{port}"), fixed_key("key")).unwrap();
        let error = remote.call("engine.execute", json!({"command":"text.insert", "params":{"text":"x".repeat(1024 * 1024)}})).unwrap_err();
        assert!(error.contains("1 MiB"), "{error}");
        assert!(!error.contains("outcome_unknown"));
        assert!(seen.recv_timeout(Duration::from_millis(100)).is_err());
    }

    #[test]
    fn lost_mutation_reply_is_not_replayed() {
        let (port, seen) = fake_app(|_| None);
        let mut remote = Remote::connect_with(&format!("127.0.0.1:{port}"), fixed_key("key")).unwrap();
        let error = remote.call("engine.execute", json!({"command":"text.insert", "params":{"text":"once"}})).unwrap_err();
        assert!(error.contains("outcome_unknown"), "{error}");
        assert_eq!(seen.recv_timeout(WAIT).unwrap()["params"]["command"], "text.insert");
        assert!(seen.recv_timeout(Duration::from_millis(100)).is_err(), "mutation must not be replayed");
    }

    #[test]
    fn mismatched_reply_id_is_rejected_without_retry() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let worker = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut writer = stream.try_clone().unwrap();
            let mut request = String::new();
            BufReader::new(stream).read_line(&mut request).unwrap();
            writeln!(writer, "{}", json!({"id":999,"ok":true,"result":{}})).unwrap();
        });
        let mut remote = Remote::connect_with(&format!("127.0.0.1:{port}"), fixed_key("key")).unwrap();
        let error = remote.call("engine.execute", json!({"command":"text.insert"})).unwrap_err();
        assert!(error.contains("outcome_unknown") && error.contains("id"), "{error}");
        worker.join().unwrap();
    }

    #[test]
    fn no_key_for_other_hosts_and_a_clear_error() {
        // There is no LAN address to test with here: same stand-in, with the decision a LAN
        // address gets.
        let (port, seen) = fake_app(|req| {
            Some(if req.get("key").is_some() {
                (json!({"ok": true, "result": 1}), false)
            } else {
                (json!({"ok": false, "error": "unauthorized"}), true)
            })
        });
        let finder: KeyFinder = Box::new(|_| Err("the key must not be read for another host".to_string()));
        let mut r = Remote::connect_with(&format!("127.0.0.1:{port}"), finder).unwrap();
        r.send_key = false;
        assert!(r.key_warning().is_some_and(|w| w.contains("not a loopback address")));
        let e = r.call("document.inspect", json!({})).unwrap_err();
        assert!(e.contains("not a loopback address") && e.contains("not sent"), "{e}");
        assert!(seen.recv_timeout(WAIT).unwrap().get("key").is_none());
    }
}

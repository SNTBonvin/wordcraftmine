//! Loopback JSON-lines control server: one request per line, one reply per line.
//! This is the transport the MCP server (`wordcraft-cli mcp --connect`) wraps.
//!
//! Every request needs the window's key (`docs/control-protocol.md`, Keys): once the port is
//! bound, the app writes a random key to `<settings>/control-key.<instance>` (mode 0600) and
//! removes the file when the window closes.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use wordcraft_ui_egui::ControlRequest;

/// Longer request lines are refused (and the connection closed).
const MAX_LINE: usize = 1 << 20;

/// A connection that sends no authorised request in this time is closed.
const FIRST_REQUEST: Duration = Duration::from_secs(30);

/// The key file of a running control server; [`KeyFile::remove`] it when the window closes.
pub struct KeyFile(PathBuf, Arc<AtomicBool>, std::sync::Mutex<Option<std::thread::JoinHandle<()>>>);

impl KeyFile {
    pub fn path(&self) -> &Path {
        &self.0
    }
    pub fn remove(&self) {
        if !self.1.swap(true, Ordering::SeqCst) {
            let _ = std::fs::remove_file(&self.0);
            if let Some(worker) = self.2.lock().unwrap_or_else(|e| e.into_inner()).take() {
                let _ = worker.join();
            }
        }
    }
}

impl Drop for KeyFile {
    fn drop(&mut self) {
        self.remove();
    }
}

/// Bind `127.0.0.1:port`, write the key file into `settings` and serve. `None` when the port
/// cannot be bound or there is no random key (the control channel is off); the key file is
/// `None` when it could not be written (requests are then refused).
pub fn start(port: u16, ctx: egui::Context, settings: Option<&Path>) -> Option<(Receiver<ControlRequest>, Option<KeyFile>)> {
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            log::error!("control server failed to bind 127.0.0.1:{port}: {e}");
            return None;
        }
    };
    let key = match random_key() {
        Ok(k) => k,
        Err(e) => {
            log::error!("control server off: no random key ({e})");
            return None;
        }
    };
    let stopped = Arc::new(AtomicBool::new(false));
    let key_file = match settings {
        Some(dir) => {
            let p = wordcraft_control_key::key_path(dir, port);
            match write_key_file(&p, &key) {
                Ok(()) => Some(KeyFile(p, stopped.clone(), std::sync::Mutex::new(None))),
                Err(e) => {
                    log::error!("control key file not written ({e}): requests will be refused");
                    None
                }
            }
        }
        None => {
            log::error!("no settings folder for the control key file: requests will be refused");
            None
        }
    };
    match &key_file {
        Some(f) => log::info!("control server listening on 127.0.0.1:{port}, key in {}", f.path().display()),
        None => log::info!("control server listening on 127.0.0.1:{port}"),
    }
    // No unusable listener is left behind when its key cannot be written.
    key_file.as_ref()?;
    listener.set_nonblocking(true).ok()?;
    let (rx, worker) = serve_on_cancel(listener, ctx, key, FIRST_REQUEST, stopped)?;
    if let Some(file) = key_file.as_ref() {
        *file.2.lock().unwrap_or_else(|e| e.into_inner()) = Some(worker);
    }
    Some((rx, key_file))
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
    f.write_all(key.as_bytes()).map_err(|e| format!("cannot write {}: {e}", p.display()))
}

/// Compare a key with the window's in time that does not depend on where they differ. An
/// empty key never matches.
fn keys_match(given: &str, key: &str) -> bool {
    let (a, b) = (given.as_bytes(), key.as_bytes());
    !b.is_empty() && a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
fn serve_on(listener: TcpListener, ctx: egui::Context, key: String, first: Duration) -> Receiver<ControlRequest> {
    let _ = listener.set_nonblocking(true);
    match serve_on_cancel(listener, ctx, key, first, Arc::new(AtomicBool::new(false))) {
        Some((rx, _worker)) => rx,
        None => {
            let (_tx, rx) = channel();
            rx
        }
    }
}

fn serve_on_cancel(
    listener: TcpListener,
    ctx: egui::Context,
    key: String,
    first: Duration,
    stopped: Arc<AtomicBool>,
) -> Option<(Receiver<ControlRequest>, std::thread::JoinHandle<()>)> {
    let (tx, rx) = channel::<ControlRequest>();
    let key: Arc<str> = key.into();
    let worker = std::thread::Builder::new()
        .name("wordcraft-control".into())
        .spawn(move || {
            let active = Arc::new(AtomicUsize::new(0));
            while !stopped.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        if active.load(Ordering::SeqCst) >= 32 {
                            continue;
                        }
                        active.fetch_add(1, Ordering::SeqCst);
                        let (tx, ctx, key, stopped, active) = (tx.clone(), ctx.clone(), key.clone(), stopped.clone(), active.clone());
                        let counter = active.clone();
                        if let Err(error) = std::thread::Builder::new().name("wordcraft-control-client".into()).spawn(move || {
                            serve(stream, tx, ctx, &key, first, stopped);
                            active.fetch_sub(1, Ordering::SeqCst);
                        }) {
                            counter.fetch_sub(1, Ordering::SeqCst);
                            log::warn!("control client worker failed: {error}");
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(10)),
                    Err(e) => {
                        log::error!("control accept failed: {e}");
                        break;
                    }
                }
            }
        })
        .map_err(|error| log::error!("control worker failed: {error}"))
        .ok()?;
    Some((rx, worker))
}

/// The read side of a connection with an optional overall deadline: each read waits only for
/// the time left, so blank lines, bad JSON or a slow drip of bytes cannot extend it.
struct Timed {
    s: TcpStream,
    deadline: Option<Instant>,
    stopped: Arc<AtomicBool>,
}

impl Read for Timed {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            if self.stopped.load(Ordering::SeqCst) {
                return Err(std::io::ErrorKind::Interrupted.into());
            }
            let timeout = if let Some(d) = self.deadline {
                d.checked_duration_since(Instant::now()).filter(|l| !l.is_zero()).ok_or(std::io::ErrorKind::TimedOut)?.min(Duration::from_millis(100))
            } else {
                Duration::from_millis(100)
            };
            self.s.set_read_timeout(Some(timeout))?;
            match self.s.read(buf) {
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => continue,
                result => return result,
            }
        }
    }
}

impl Timed {
    fn set_deadline(&mut self, d: Option<Instant>) {
        self.deadline = d;
    }
}

/// The next line (trimmed), `None` at the end of the stream or after a read error.
fn read_line(r: &mut BufReader<Timed>) -> Option<Result<String, &'static str>> {
    let mut buf = Vec::new();
    let n = r.by_ref().take(MAX_LINE as u64 + 1).read_until(b'\n', &mut buf).ok()?;
    if n == 0 {
        return None;
    }
    if buf.len() > MAX_LINE {
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

fn serve(stream: TcpStream, tx: Sender<ControlRequest>, ctx: egui::Context, key: &str, first: Duration, stopped: Arc<AtomicBool>) {
    // macOS propagates the listener's nonblocking flag to accepted sockets. Timed reads need
    // blocking mode so idle connections sleep instead of spinning between cancellation checks.
    if let Err(error) = stream.set_nonblocking(false) {
        log::warn!("control socket setup failed: {error}");
        return;
    }
    if let Err(error) = stream.set_write_timeout(Some(Duration::from_secs(5))) {
        log::warn!("control write timeout setup failed: {error}");
        return;
    }
    let Ok(read) = stream.try_clone() else { return };
    let mut out = stream;
    // One overall deadline from accept to the first authorised request; after it, none.
    let mut reader = BufReader::new(Timed { s: read, deadline: Some(Instant::now() + first), stopped: stopped.clone() });
    let mut authorised = false;
    // The key an `auth` request gave this connection.
    let mut conn_key: Option<String> = None;
    while let Some(line) = read_line(&mut reader) {
        if stopped.load(Ordering::SeqCst) {
            break;
        }
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                let _ = writeln!(out, "{}", json!({"ok": false, "error": e}));
                if e == "line too long" {
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
        let params = msg.get("params").cloned().unwrap_or(json!({}));
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
        // Checked on every request.
        let k = given.or(conn_key.as_deref()).unwrap_or("");
        if !keys_match(k, key) {
            let _ = reply(json!({"ok": false, "error": "unauthorized"}));
            break;
        }
        if !authorised {
            authorised = true;
            reader.get_mut().set_deadline(None);
        }
        if method == "auth" {
            conn_key = Some(k.to_string());
            if !reply(json!({"ok": true, "result": {}})) {
                break;
            }
            continue;
        }
        if stopped.load(Ordering::SeqCst) {
            break;
        }
        let (req, rrx) = ControlRequest::new(method, params);
        if tx.send(req).is_err() {
            break;
        }
        ctx.request_repaint();
        let r = rrx.recv_timeout(Duration::from_secs(60)).unwrap_or_else(|_| json!({"ok": false, "error": "timeout"}));
        if !reply(r) {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpStream;

    #[test]
    fn disabling_closes_port_removes_key_and_allows_reconnect() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let dir = std::env::temp_dir().join(format!("wordcraft-control-toggle-{}-{port}", std::process::id()));
        let (rx, file) = start(port, egui::Context::default(), Some(&dir)).unwrap();
        let file = file.unwrap();
        let path = file.path().to_path_buf();
        assert!(path.exists());
        let mut socket = TcpStream::connect(("127.0.0.1", port)).unwrap();
        socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        file.remove();
        assert!(!path.exists());
        assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
        let mut byte = [0; 1];
        assert!(matches!(socket.read(&mut byte), Ok(0) | Err(_)));
        assert!(rx.try_recv().is_err());
        let (_rx, next) = start(port, egui::Context::default(), Some(&dir)).unwrap();
        drop(file);
        assert!(path.exists(), "dropping an old handle cannot remove the new key");
        drop(next);
        assert!(!path.exists());
        let _ = std::fs::remove_dir(&dir);
    }

    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn boot_with(first: Duration) -> (u16, Receiver<ControlRequest>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        (port, serve_on(listener, egui::Context::default(), KEY.to_string(), first))
    }

    fn boot() -> (u16, Receiver<ControlRequest>) {
        boot_with(FIRST_REQUEST)
    }

    /// Answer the next request that reaches the UI with `ok`, and return it.
    fn answer(rx: &Receiver<ControlRequest>) -> ControlRequest {
        let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let _ = req.reply.send(json!({"ok": true, "result": {}}));
        req
    }

    /// One connection, several lines.
    struct Conn(BufReader<TcpStream>, TcpStream);

    impl Conn {
        fn open(port: u16) -> Self {
            let s = TcpStream::connect(("127.0.0.1", port)).unwrap();
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
        assert!(start(port, egui::Context::default(), Some(&dir)).is_none());
        assert!(!dir.exists() || std::fs::read_dir(&dir).unwrap().next().is_none(), "no key file after a failed bind");
        drop(taken);
        // Bound: the key file holds the key that opens the port.
        let (rx, key_file) = start(port, egui::Context::default(), Some(&dir)).unwrap();
        let key_file = key_file.unwrap();
        assert_eq!(key_file.path(), wordcraft_control_key::key_path(&dir, port));
        let key = std::fs::read_to_string(key_file.path()).unwrap();
        assert_eq!(key.len(), 64);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(key_file.path()).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let mut c = Conn::open(port);
        c.send(&format!(r#"{{"id":1,"key":"{key}","method":"document.inspect"}}"#));
        answer(&rx);
        assert_eq!(c.recv()["ok"], true);
        key_file.remove();
        assert!(!key_file.path().exists());
        let _ = std::fs::remove_dir_all(&dir);
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

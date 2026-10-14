//! Live MCP integration against the desktop's real loopback transport and UI document.
//! No native window, environment changes, legacy --control listener, or document files.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use wordcraft_doc::{Document, Pos, StoryRef};
use wordcraft_engine::{Selection, Session};
use wordcraft_ui_egui::{Services, WordApp};

use crate::control_server::AgentRuntime;

static NEXT: AtomicUsize = AtomicUsize::new(0);
enum ClientRequest {
    Rpc(String),
    SecondRpc(String),
    RestartPrimary(u64),
    Stop,
}

struct Harness {
    app: WordApp,
    ctx: egui::Context,
    settings: PathBuf,
    runtime: Arc<Mutex<AgentRuntime>>,
    requests: Sender<ClientRequest>,
    replies: Receiver<Value>,
    worker: Option<JoinHandle<()>>,
    next_id: u64,
}
impl Harness {
    fn new() -> Self {
        let settings = std::env::temp_dir().join(format!("wordcraft-live-e2e-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&settings).unwrap();
        let ctx = egui::Context::default();
        let runtime = Arc::new(Mutex::new(AgentRuntime::default()));
        let (tx, rx) = crate::control_server::request_channel();
        let (hook_runtime, hook_settings, hook_ctx) = (runtime.clone(), settings.clone(), ctx.clone());
        let services = Services {
            agent_control: Some(Box::new(move |request| {
                if request.get("setup").and_then(Value::as_bool) == Some(true) {
                    return Ok(json!({"setupReady":true,"mcpConfig":"test config"}));
                }
                hook_runtime.lock().map_err(|_| "test runtime unavailable")?.update(request, &hook_ctx, &hook_settings, tx.clone())
            })),
            ..Default::default()
        };
        let mut session = Session::new(Document::from_text("Unsaved café document."));
        session.author = "Document owner".into();
        session.prefs.autocorrect.enabled = false;
        session.sel = Selection::caret(Pos::body(0, 0));
        let mut app = WordApp::new(session, services).with_control(rx);
        app.ui.allow_agent_requests = false;
        let (requests, incoming) = channel();
        let (outgoing, replies) = channel();
        let client_settings = settings.clone();
        let worker = std::thread::spawn(move || {
            let mut server = wordcraft_mcp::Server::new(Box::new(wordcraft_mcp::Live::with_settings(client_settings.clone())));
            let first_settings = client_settings.clone();
            let mut second_server = wordcraft_mcp::Server::new(Box::new(wordcraft_mcp::Live::with_settings(client_settings)));
            while let Ok(request) = incoming.recv() {
                let reply = match request {
                    ClientRequest::Rpc(line) => server.handle_line(&line),
                    ClientRequest::SecondRpc(line) => second_server.handle_line(&line),
                    ClientRequest::RestartPrimary(id) => {
                        // Dropping the bridge's TCP stream simulates an abrupt process exit:
                        // there is no MCP disconnect call to release membership.
                        server = wordcraft_mcp::Server::new(Box::new(wordcraft_mcp::Live::with_settings(first_settings.clone())));
                        if outgoing.send(json!({"jsonrpc":"2.0","id":id,"result":{"restarted":true}})).is_err() {
                            break;
                        }
                        continue;
                    }
                    ClientRequest::Stop => break,
                }
                .expect("MCP request has a reply");
                let value = serde_json::from_str(&reply).expect("MCP reply JSON");
                if outgoing.send(value).is_err() {
                    break;
                }
            }
        });
        Self { app, ctx, settings, runtime, requests, replies, worker: Some(worker), next_id: 1 }
    }
    fn tick(&mut self) {
        self.ctx.run_ui(egui::RawInput::default(), |ui| self.app.logic(ui.ctx())).drop_without_applying_deltas();
    }
    fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.rpc_for(false, method, params)
    }
    fn rpc_for(&mut self, second: bool, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let line = json!({"jsonrpc":"2.0", "id":id, "method":method,"params":params}).to_string();
        self.requests.send(if second { ClientRequest::SecondRpc(line) } else { ClientRequest::Rpc(line) }).unwrap();
        self.await_reply(id, method)
    }
    fn restart_primary(&mut self) {
        let id = self.next_id;
        self.next_id += 1;
        self.requests.send(ClientRequest::RestartPrimary(id)).unwrap();
        self.await_reply(id, "restart bridge");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            self.tick();
            if self.app.run("ui.agents", json!({})).unwrap()["agent"].is_null() {
                break;
            }
            assert!(Instant::now() < deadline, "closed transport must release its UI member");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn await_reply(&mut self, id: u64, method: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            match self.replies.try_recv() {
                Ok(reply) => {
                    assert_eq!(reply["id"], id);
                    return reply;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => panic!("MCP client worker stopped"),
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
            assert!(Instant::now() < deadline, "MCP request {method} exceeded integration deadline");
            self.tick();
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn tool(&mut self, name: &str, arguments: Value) -> Value {
        let reply = self.rpc("tools/call", json!({"name":name,"arguments":arguments}));
        assert!(reply.get("error").is_none(), "MCP error: {reply}");
        reply["result"].clone()
    }
    fn success(&mut self, name: &str, arguments: Value) -> Value {
        let result = self.tool(name, arguments);
        assert_eq!(result["isError"], false, "{name}: {result}");
        let text = result["content"][0]["text"].as_str().unwrap();
        serde_json::from_str(text).unwrap_or_else(|_| json!(text))
    }
    fn failure(&mut self, name: &str, arguments: Value) -> String {
        let result = self.tool(name, arguments);
        assert_eq!(result["isError"], true, "{name} should fail: {result}");
        result["content"][0]["text"].as_str().unwrap().to_string()
    }
    fn enable(&mut self) -> Value {
        let grant = self.app.run("ui.agents", json!({"enabled":true})).unwrap();
        assert_eq!(grant["enabled"], true);
        grant["session"].clone()
    }
    fn connect(&mut self, descriptor: &Value) -> Value {
        assert!(
            self.failure("connect_session", json!({"sessionId":descriptor["sessionId"],"agentName":"Integration agent"}))
                .contains("approval_required")
        );
        let pending = self.app.run("ui.agents", json!({})).unwrap()["pending"].clone();
        assert_eq!(pending["name"], "Integration agent");
        self.app.run("ui.agents", json!({"approve":true,"clientId":pending["clientId"]})).unwrap();
        self.success("get_session_status", json!({}))
    }
    fn text(&self) -> String {
        self.app.session.doc.plain_text(StoryRef::Body)
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        // Closing real sockets unblocks a pending client call even if the assertion failed.
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.stop();
        }
        let _ = self.requests.send(ClientRequest::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        let _ = std::fs::remove_dir_all(&self.settings);
    }
}

#[test]
fn ordinary_open_document_connects_through_real_live_mcp_and_revokes_cleanly() {
    let mut h = Harness::new();
    assert!(h.app.session.path.is_none());
    assert!(!h.app.agents.enabled());
    assert!(!wordcraft_control_key::sessions_dir(&h.settings).exists());
    assert_eq!(std::fs::read_dir(&h.settings).unwrap().count(), 0);

    let init = h.rpc("initialize", json!({"protocolVersion":"2025-06-18"}));
    assert!(init["result"]["instructions"].as_str().unwrap().contains("already open WordCraft"));
    let tools = h.rpc("tools/list", json!({}));
    let names: Vec<_> = tools["result"]["tools"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(names.contains(&"connect_session"));
    assert!(!names.contains(&"new_document"));
    assert!(!names.contains(&"save_document"));
    assert_eq!(h.success("list_sessions", json!({}))["sessions"], json!([]));
    assert!(h.failure("get_text", json!({})).contains("not_connected"));

    let descriptor = h.enable();
    let session_id = descriptor["sessionId"].as_str().unwrap().to_string();
    let descriptor_path = wordcraft_control_key::session_path(&h.settings, &session_id).unwrap();
    #[cfg(unix)]
    let transport_path = {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(descriptor["transport"], "unix");
        assert!(descriptor.get("address").is_none());
        let path = PathBuf::from(descriptor["socketPath"].as_str().unwrap());
        assert_eq!(std::fs::metadata(path.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(&descriptor_path).unwrap().permissions().mode() & 0o777, 0o600);
        path
    };
    #[cfg(not(unix))]
    let transport_path = {
        let address = descriptor["address"].as_str().unwrap().parse::<std::net::SocketAddr>().unwrap();
        assert!(address.ip().is_loopback());
        wordcraft_control_key::live_key_path(&h.settings, address.port())
    };
    assert!(descriptor_path.exists() && transport_path.exists());
    let listed = h.success("list_sessions", json!({}));
    assert_eq!(listed["sessions"][0]["sessionId"], session_id);
    assert!(h.failure("connect_session", json!({"sessionId":"not-this-window","agentName":"Integration agent"})).contains("unavailable"));
    let checked = h.success("check_session", json!({"sessionId":session_id}));
    assert_eq!(checked["documentId"], h.app.session.document_id());
    assert_eq!(checked["permissions"], "request-approval");
    assert!(h.app.run("ui.agents", json!({})).unwrap()["agent"].is_null());
    let connected = h.connect(&descriptor);
    assert_eq!(connected["documentId"], h.app.session.document_id());
    assert_eq!(connected["connected"], true);
    assert_eq!(connected["permissions"], "tracked-editing");
    assert_eq!(connected["protocolVersion"], 1);
    assert_eq!(h.success("get_session_status", json!({}))["sessionId"], session_id);

    // The same author label in a second MCP process must not share the first client's caret
    // or evict it through cleanup after a failed handshake.
    let first_client_id = connected["clientId"].clone();
    assert!(first_client_id.as_str().is_some_and(|id| !id.is_empty()));
    let competitor =
        h.rpc_for(true, "tools/call", json!({"name":"connect_session", "arguments":{"sessionId":session_id,"agentName":"Integration agent"}}));
    assert_eq!(competitor["result"]["isError"], true);
    assert!(competitor["result"]["content"][0]["text"].as_str().unwrap().contains("agent_busy"));
    let surviving = h.success("get_session_status", json!({}));
    assert_eq!(surviving["connected"], true);
    assert_eq!(surviving["clientId"], first_client_id);

    let original = h.text();
    assert!(h.failure("type_text", json!({"text":"MUST NOT INSERT"})).contains("revision_conflict"));
    assert_eq!(h.text(), original);
    let inspect = h.success("inspect_document", json!({}));
    assert_eq!(inspect["blocks"][0]["text"], "Unsaved café document.");
    assert_eq!(h.success("get_text", json!({}))["text"], original);
    let owner_caret = h.app.session.sel.clone();
    h.success("select_text", json!({"text":"café"}));
    assert_eq!(h.app.session.sel, owner_caret);
    h.success("execute", json!({"command":"format.bold","params":{"value":true}}));
    h.success("execute", json!({"command":"caret.docEnd"}));
    h.success("type_text", json!({"text":" Agent edit."}));
    assert!(h.text().ends_with(" Agent edit."));
    assert_eq!(h.app.session.sel, owner_caret);
    assert_eq!(h.app.session.author, "Document owner");
    assert!(!h.app.session.doc.settings.track_changes);
    assert!(h.app.session.path.is_none());
    let changes = h.success("execute", json!({"command":"review.changes"}));
    assert!(changes.to_string().contains("Integration agent"));
    let inspect = h.success("inspect_document", json!({}));
    assert!(inspect["blocks"][0]["runs"].as_array().unwrap().iter().any(|r| r["props"]["bold"] == true));

    h.app.run("text.insert", json!({"text":"Owner "})).unwrap();
    let before_conflict = h.text();
    assert!(h.failure("type_text", json!({"text":"MUST NOT APPLY STALE EDIT"})).contains("revision_conflict"));
    assert_eq!(h.text(), before_conflict);
    h.success("inspect_document", json!({}));
    h.success("execute", json!({"command":"caret.docEnd"}));
    h.success("type_text", json!({"text":" After reread."}));
    assert!(h.text().starts_with("Owner "));
    assert!(h.text().ends_with(" After reread."));

    // An abruptly closed MCP transport releases the slot while the owner's grant stays on.
    // A replacement bridge must obtain a fresh identity, read and reselect before edits.
    h.restart_primary();
    assert!(h.app.agents.enabled());
    assert!(descriptor_path.exists());
    let restarted = h.connect(&descriptor);
    assert_ne!(restarted["clientId"], first_client_id);
    assert!(h.failure("type_text", json!({"text":"MUST NOT SKIP REREAD"})).contains("revision_conflict"));
    h.success("inspect_document", json!({}));
    h.success("execute", json!({"command":"caret.docEnd"}));
    h.success("type_text", json!({"text":" Client restarted."}));
    assert!(h.text().ends_with(" Client restarted."));

    let commands = h.success("list_commands", json!({}));
    assert!(!commands.as_array().unwrap().iter().any(|c| c["id"] == "file.save"));
    let forbidden_path = h.settings.join("forbidden.docx");
    assert!(h.failure("execute", json!({"command":"file.save","params":{"path":forbidden_path}})).contains("not permitted"));
    assert!(h.failure("save_document", json!({"path":forbidden_path})).contains("unavailable"));
    assert!(h.failure("execute", json!({"command":"ui.agents","params":{"enabled":false}})).contains("not permitted"));
    assert!(!forbidden_path.exists());
    assert!(h.app.agents.enabled());

    let before_revoke = h.text();
    h.app.run("ui.agents", json!({"enabled":false})).unwrap();
    assert!(!descriptor_path.exists() && !transport_path.exists());
    assert!(!h.app.agents.enabled());
    assert!(h.failure("type_text", json!({"text":"MUST NOT APPLY REVOKED EDIT"})).contains("session_revoked"));
    assert_eq!(h.text(), before_revoke);
    assert_eq!(h.success("disconnect_session", json!({}))["connected"], false);

    let replacement_grant = h.enable();
    assert_ne!(replacement_grant["sessionId"], descriptor["sessionId"]);
    assert!(h.failure("connect_session", json!({"sessionId":session_id,"agentName":"Integration agent"})).contains("unavailable"));
    h.connect(&replacement_grant);
    assert_eq!(h.success("get_text", json!({}))["text"], before_revoke);
    let replacement_path = wordcraft_control_key::session_path(&h.settings, replacement_grant["sessionId"].as_str().unwrap()).unwrap();
    h.app.session.set_document(Document::from_text("Replacement document"));
    h.tick();
    assert!(h.app.agents.enabled(), "reusable connection requests remain available for the new document");
    assert!(!replacement_path.exists());
    let new_identity = h.app.run("ui.agents", json!({})).unwrap()["session"].clone();
    assert_ne!(new_identity["sessionId"], replacement_grant["sessionId"]);
    assert_eq!(new_identity["documentId"], h.app.session.document_id());
    assert!(h.failure("get_text", json!({})).contains("session_revoked"));
    assert_eq!(h.text(), "Replacement document");
    assert!(h.failure("new_document", json!({})).contains("unavailable"));
    h.success("disconnect_session", json!({}));
}

#[test]
fn saved_opt_in_discovers_requests_but_never_grants_without_consent() {
    let mut h = Harness::new();
    h.app.ui.allow_agent_requests = true;
    h.tick();
    assert!(h.app.agents.enabled());
    let sessions = h.success("list_sessions", json!({}));
    let session = sessions["sessions"][0]["sessionId"].clone();
    assert!(h.failure("connect_session", json!({"sessionId":session,"agentName":"Document owner"})).contains("approval_required"));
    assert!(h.failure("get_text", json!({})).contains("approval_required"));
    let pending = h.app.run("ui.agents", json!({})).unwrap()["pending"].clone();
    h.app.run("ui.agents", json!({"approve":false,"clientId":pending["clientId"]})).unwrap();
    assert!(h.failure("get_session_status", json!({})).contains("approval_denied"));
    assert_eq!(h.text(), "Unsaved café document.");
}

#[test]
fn fresh_launch_keeps_document_metadata_private_until_requests_are_enabled() {
    let mut h = Harness::new();
    assert!(!h.app.ui.allow_agent_requests);
    for _ in 0..5 {
        h.tick();
    }
    assert!(!h.app.agents.enabled());
    assert!(h.success("list_sessions", json!({}))["sessions"].as_array().unwrap().is_empty());
    assert!(!wordcraft_control_key::sessions_dir(&h.settings).exists());
}

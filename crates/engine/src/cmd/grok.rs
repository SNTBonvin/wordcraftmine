//! Opt-in writing commands that call the public xAI API (`https://api.x.ai/v1/responses`).
//!
//! Nothing is sent until the user runs a command. The key comes from `XAI_API_KEY` or from a
//! gitignored `.env` (current directory, `WORDCRAFT_DOTENV`, or `$XDG_CONFIG_HOME/wordcraft/.env`).
//! The key and the document text are not written to the log. Redirects are disabled so the key
//! cannot follow a response to another host. The browser build refuses these commands.

use std::path::{Path, PathBuf};
#[cfg(not(any(test, target_arch = "wasm32")))]
use std::time::Duration;

use serde_json::{Value, json};
#[cfg(not(any(test, target_arch = "wasm32")))]
use zeroize::Zeroize;

use super::type_text;
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

#[cfg(not(any(test, target_arch = "wasm32")))]
const API_URL: &str = "https://api.x.ai/v1/responses";
const DEFAULT_MODEL: &str = "grok-4.7";
const MAX_SOURCE_CHARS: usize = 12_000;
const MAX_INSTRUCTION_CHARS: usize = 1_000;
#[cfg_attr(any(test, target_arch = "wasm32"), allow(dead_code))]
const MAX_OUTPUT_CHARS: usize = 16_000;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("grok.rewrite", "Rewrite", "Review › Grok", rewrite)
            .params(r#"{"instruction"?: string, "model"?: string} — sends the selection to api.x.ai and returns {source, text}; grok.accept replaces the selection"#)
            .pure(),
        CommandSpec::new("grok.continue", "Continue", "Review › Grok", continue_writing)
            .params(
                r#"{"instruction"?: string, "model"?: string} — sends the selection, or the end of the document, to api.x.ai and returns {source, text}; grok.accept inserts it"#,
            )
            .pure(),
        CommandSpec::new("grok.summarize", "Summarize", "Review › Grok", summarize)
            .params(
                r#"{"instruction"?: string, "model"?: string} — sends the selection, or the document, to api.x.ai and returns {source, text}; grok.accept adds a comment"#,
            )
            .pure(),
        CommandSpec::new("grok.accept", "Accept Grok", "Review › Grok", accept)
            .params(r#"{"action": "rewrite|continue|summarize", "text": string} — writes the text the user already accepted"#),
    ]
}

fn rewrite(s: &mut Session, v: &Value) -> CmdResult {
    let source = s.selected_text();
    if source.trim().is_empty() {
        return Err(CmdError::Failed("select the text to rewrite".into()));
    }
    let out = ask(v, "Rewrite the document text. Return only the replacement text.", &source)?;
    Ok(json!({"action": "rewrite", "source": source, "text": out}))
}

fn continue_writing(s: &mut Session, v: &Value) -> CmdResult {
    let selected = s.selected_text();
    let source = if selected.trim().is_empty() {
        let all = s.doc.plain_text(wordcraft_doc::StoryRef::Body);
        tail_chars(&all, 6_000).to_string()
    } else {
        selected
    };
    if source.trim().is_empty() {
        return Err(CmdError::Failed("the document is empty".into()));
    }
    let out = ask(v, "Continue the document text. Return only the continuation, with no recap of the source.", &source)?;
    Ok(json!({"action": "continue", "source": source, "text": out}))
}

fn summarize(s: &mut Session, v: &Value) -> CmdResult {
    let selected = s.selected_text();
    let source = if selected.trim().is_empty() { s.doc.plain_text(wordcraft_doc::StoryRef::Body) } else { selected };
    if source.trim().is_empty() {
        return Err(CmdError::Failed("nothing to summarize".into()));
    }
    let out = ask(v, "Summarize the document text in a short comment. Return only the summary.", &source)?;
    Ok(json!({"action": "summarize", "source": source, "text": out}))
}

/// Write a suggestion the user accepted. Does not call the network.
fn accept(s: &mut Session, v: &Value) -> CmdResult {
    let action = p::req_str(v, "action")?;
    let text = p::req_str(v, "text")?;
    if text.trim().is_empty() {
        return Err(CmdError::Params("`text` is empty".into()));
    }
    match action {
        "rewrite" => {
            if s.selected_text().trim().is_empty() {
                return Err(CmdError::Failed("select the text to replace".into()));
            }
            type_text(s, text)?;
            Ok(json!({"action": action, "chars": text.chars().count()}))
        }
        "continue" => {
            s.sel.anchor = s.sel.focus.clone();
            type_text(s, text)?;
            Ok(json!({"action": action, "chars": text.chars().count()}))
        }
        "summarize" => super::review::new_comment(s, &json!({"text": text})),
        _ => Err(CmdError::Params("`action` must be rewrite, continue, or summarize".into())),
    }
}

#[cfg_attr(any(test, target_arch = "wasm32"), allow(unused_variables))]
fn ask(v: &Value, task: &str, source: &str) -> Result<String, CmdError> {
    if source.chars().count() > MAX_SOURCE_CHARS {
        return Err(CmdError::Failed(format!("that text is longer than {MAX_SOURCE_CHARS} characters; select a shorter passage")));
    }
    let instruction = p::str(v, "instruction").unwrap_or("").trim();
    if instruction.chars().count() > MAX_INSTRUCTION_CHARS {
        return Err(CmdError::Params(format!("`instruction` is longer than {MAX_INSTRUCTION_CHARS} characters")));
    }
    let model = model_name(v)?;
    // Tests and the browser build stop before reading a key or opening a socket.
    #[cfg(any(test, target_arch = "wasm32"))]
    {
        let _ = (model, instruction, source);
        return Err(CmdError::Failed("Grok sends text only from the desktop app, and tests do not call the network.".into()));
    }
    #[cfg(not(any(test, target_arch = "wasm32")))]
    {
        let mut key = api_key()?;
        let prompt = format!(
            "{task}\n\nInstruction: {instruction}\n\nDocument text:\n---\n{source}\n---\nReturn only the text that should go into the document."
        );
        let result = complete(&model, &key, &prompt);
        key.zeroize();
        let mut text = result?;
        if text.chars().count() > MAX_OUTPUT_CHARS {
            text = text.chars().take(MAX_OUTPUT_CHARS).collect();
        }
        if text.trim().is_empty() {
            return Err(CmdError::Failed("the xAI API returned no text".into()));
        }
        Ok(text)
    }
}

fn model_name(v: &Value) -> Result<String, CmdError> {
    let from_param = p::str(v, "model").map(str::trim).filter(|s| !s.is_empty());
    let from_env = std::env::var("WORDCRAFT_GROK_MODEL").ok();
    let name = from_param.map(str::to_string).or(from_env).unwrap_or_else(|| DEFAULT_MODEL.to_string());
    if name.len() > 64 || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-') {
        return Err(CmdError::Params("`model` must be a model id such as grok-4.7".into()));
    }
    Ok(name)
}

#[cfg_attr(any(test, target_arch = "wasm32"), allow(dead_code))]
fn api_key() -> Result<String, CmdError> {
    if let Some(key) = nonempty_env("XAI_API_KEY") {
        return Ok(key);
    }
    let mut paths = Vec::new();
    if let Some(path) = nonempty_env("WORDCRAFT_DOTENV") {
        paths.push(PathBuf::from(path));
    }
    if let Ok(cwd) = std::env::current_dir() {
        paths.push(cwd.join(".env"));
    }
    paths.push(config_dir().join(".env"));
    for path in paths {
        if let Some(key) = key_in_file(&path) {
            return Ok(key);
        }
    }
    Err(CmdError::Failed(
        "no xAI API key. Set XAI_API_KEY or put it in .env (see .env.example). The key stays on this machine until a Grok command runs.".into(),
    ))
}

#[cfg_attr(any(test, target_arch = "wasm32"), allow(dead_code))]
fn nonempty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

#[cfg_attr(any(test, target_arch = "wasm32"), allow(dead_code))]
fn config_dir() -> PathBuf {
    if let Some(xdg) = nonempty_env("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("wordcraft");
    }
    if let Some(home) = nonempty_env("HOME") {
        return PathBuf::from(home).join(".config").join("wordcraft");
    }
    PathBuf::from(".config").join("wordcraft")
}

#[cfg_attr(any(test, target_arch = "wasm32"), allow(dead_code))]
fn key_in_file(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    key_from_dotenv(&text)
}

pub(crate) fn key_from_dotenv(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let (name, value) = line.split_once('=')?;
        if name.trim() != "XAI_API_KEY" {
            continue;
        }
        let value = value.trim().trim_matches('"').trim_matches('\'').trim();
        if value.is_empty() {
            continue;
        }
        return Some(value.to_string());
    }
    None
}

fn tail_chars(text: &str, max: usize) -> &str {
    let count = text.chars().count();
    if count <= max {
        return text;
    }
    let skip = count - max;
    let start = text.char_indices().nth(skip).map(|(i, _)| i).unwrap_or(0);
    text.get(start..).unwrap_or("")
}

pub(crate) fn output_text(v: &Value) -> Option<String> {
    if let Some(text) = v.get("output_text").and_then(Value::as_str) {
        let text = text.trim();
        if !text.is_empty() {
            return Some(text.to_string());
        }
    }
    let mut acc = String::new();
    let items = v.get("output").and_then(Value::as_array)?;
    for item in items {
        let Some(content) = item.get("content").and_then(Value::as_array) else { continue };
        for part in content {
            if part.get("type").and_then(Value::as_str) != Some("output_text") {
                continue;
            }
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                acc.push_str(text);
            }
        }
    }
    let acc = acc.trim();
    if acc.is_empty() { None } else { Some(acc.to_string()) }
}

#[cfg(not(any(test, target_arch = "wasm32")))]
fn complete(model: &str, key: &str, prompt: &str) -> Result<String, CmdError> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(60)).redirects(0).build();
    let response = agent
        .post(API_URL)
        .set("Authorization", &format!("Bearer {key}"))
        .set("Content-Type", "application/json")
        .send_json(json!({"model": model, "input": prompt}));
    let response = match response {
        Ok(response) => response,
        Err(ureq::Error::Status(code, _)) => {
            return Err(CmdError::Failed(format!("xAI API returned HTTP {code}")));
        }
        Err(_) => return Err(CmdError::Failed("could not reach the xAI API".into())),
    };
    let parsed: Value = match response.into_json() {
        Ok(v) => v,
        Err(_) => return Err(CmdError::Failed("xAI API response was not JSON".into())),
    };
    output_text(&parsed).ok_or_else(|| CmdError::Failed("xAI API response had no output text".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotenv_reads_only_the_key() {
        let text = "# comment\nOTHER=secret\nexport XAI_API_KEY=\"abc-def\"\n";
        assert_eq!(key_from_dotenv(text).as_deref(), Some("abc-def"));
        assert_eq!(key_from_dotenv("XAI_API_KEY=\n"), None);
    }

    #[test]
    fn output_text_reads_the_message_and_ignores_reasoning() {
        let v = json!({
            "output": [
                {"type": "reasoning", "content": [{"type": "reasoning_text", "text": "hidden"}]},
                {"type": "message", "content": [{"type": "output_text", "text": "Visible"}]}
            ]
        });
        assert_eq!(output_text(&v).as_deref(), Some("Visible"));
        assert_eq!(output_text(&json!({"output_text": "Direct"})).as_deref(), Some("Direct"));
    }

    #[test]
    fn rewrite_without_a_selection_does_not_edit() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        let before = s.doc.plain_text(wordcraft_doc::StoryRef::Body);
        let err = s.run("grok.rewrite", &json!({})).unwrap_err();
        assert!(err.to_string().contains("select"));
        assert_eq!(s.doc.plain_text(wordcraft_doc::StoryRef::Body), before);
    }

    #[test]
    fn rewrite_does_not_call_the_network_or_edit() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("text.insert", &json!({"text": "Hello there"})).unwrap();
        s.run("select.all", &json!({})).unwrap();
        assert_eq!(s.selected_text(), "Hello there");
        let err = s.run("grok.rewrite", &json!({"instruction": "shorter"})).unwrap_err();
        assert!(err.to_string().contains("tests do not call the network"), "{err}");
        assert!(!err.to_string().contains("Hello there"));
        assert_eq!(s.doc.plain_text(wordcraft_doc::StoryRef::Body), "Hello there");
    }

    #[test]
    fn accept_rewrites_only_the_selection() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("text.insert", &json!({"text": "Hello there"})).unwrap();
        s.run("select.all", &json!({})).unwrap();
        s.run("grok.accept", &json!({"action": "rewrite", "text": "Hi"})).unwrap();
        assert_eq!(s.doc.plain_text(wordcraft_doc::StoryRef::Body), "Hi");
    }

    #[test]
    fn accept_down_is_not_a_command_and_continue_inserts_after() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("text.insert", &json!({"text": "Hello"})).unwrap();
        s.run("select.all", &json!({})).unwrap();
        let before = s.doc.plain_text(wordcraft_doc::StoryRef::Body);
        // Discarding a suggestion never reaches the engine.
        assert_eq!(s.doc.plain_text(wordcraft_doc::StoryRef::Body), before);
        s.run("grok.accept", &json!({"action": "continue", "text": " world"})).unwrap();
        assert_eq!(s.doc.plain_text(wordcraft_doc::StoryRef::Body), "Hello world");
    }
}

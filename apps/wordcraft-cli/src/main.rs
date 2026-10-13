//! `wordcraft-cli`: WordCraft from the command line.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod automation_args;
mod help;

use std::fmt;
use std::io::Write;
use std::process::ExitCode;

use serde_json::{Value, json};
use wordcraft_engine::Session;

fn open(path: &str) -> Result<Session, String> {
    let doc = wordcraft_engine::io::open_path(std::path::Path::new(path)).map_err(|e| format!("can't open {e}"))?;
    let mut s = Session::new(doc);
    s.path = Some(path.into());
    Ok(s)
}

fn blank() -> Session {
    Session::new(wordcraft_doc::Document::new())
}

fn from_template(name: &str) -> Result<Session, String> {
    let templates = wordcraft_engine::cmd::file::TEMPLATES;
    if !templates.contains(&name) {
        return Err(format!("`{name}` is not a template: --template takes {}.", list(&templates)));
    }
    let mut s = blank();
    s.call("file.new", &json!({"template": name}))?;
    Ok(s)
}

fn list(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [one] => one.to_string(),
        [head @ .., last] => format!("{} or {last}", head.join(", ")),
    }
}

fn page_number(args: &[String]) -> Result<Option<u64>, String> {
    match arg_value(args, "--page") {
        None => Ok(None),
        Some(p) => {
            p.parse::<u64>().ok().filter(|n| *n >= 1).map(Some).ok_or_else(|| format!("--page takes a page number, 1 = the first; `{p}` is not one"))
        }
    }
}

fn scale(args: &[String]) -> Result<Option<f64>, String> {
    match arg_value(args, "--scale") {
        None => Ok(None),
        Some(p) => p
            .parse::<f64>()
            .ok()
            .filter(|s| s.is_finite() && *s > 0.0)
            .map(Some)
            .ok_or_else(|| format!("--scale takes pixels per point, e.g. 2; `{p}` is not a number above 0")),
    }
}

fn steps(args: &[String], reg: &wordcraft_engine::Registry) -> Result<Vec<(String, Value)>, String> {
    if args.last().is_some_and(|a| a == "--cmd") {
        return Err("the last --cmd has no command: --cmd takes COMMAND=PARAMS, e.g. --cmd 'text.insert={\"text\":\"Hello\"}'. Nothing ran.".into());
    }
    let specs: Vec<&String> = args.iter().zip(args.iter().skip(1)).filter(|(a, _)| *a == "--cmd").map(|(_, v)| v).collect();
    let total = specs.len();
    let at = |n: usize| format!("--cmd {} of {total}", n + 1);
    specs
        .iter()
        .enumerate()
        .map(|(n, spec)| {
            let (id, text) = spec.split_once('=').unwrap_or((spec, ""));
            let Some(command) = reg.get(id) else {
                let read = if text.is_empty() { "no parameters".to_string() } else { format!("parameters `{}`", shown(text)) };
                return Err(format!(
                    "{} was read as command `{id}` with {read}. {} --cmd takes COMMAND=PARAMS, a command id, then `=`, then a JSON \
                     object, e.g. --cmd 'text.insert={{\"text\":\"Hello\"}}'; `wordcraft-cli commands` lists the ids. Nothing ran.",
                    at(n),
                    wordcraft_engine::call::unknown_command(reg, id)
                ));
            };
            let params = if text.is_empty() {
                json!({})
            } else {
                serde_json::from_str::<Value>(text).map_err(|e| {
                    format!(
                        "{} was read as command `{id}` with parameters `{}`, which are not JSON: {e}.{} Nothing ran.",
                        at(n),
                        shown(text),
                        wordcraft_engine::call::takes(command)
                    )
                })?
            };
            Ok((id.to_string(), params))
        })
        .collect()
}

fn shown(text: &str) -> String {
    match text.char_indices().nth(200) {
        Some((i, _)) => format!("{}… ({} chars)", text.get(..i).unwrap_or_default(), text.chars().count()),
        None => text.to_string(),
    }
}

fn run_steps(s: &mut Session, steps: &[(String, Value)], print: bool, unsaved: bool) -> Result<(), Failure> {
    let total = steps.len();
    for (n, (id, params)) in steps.iter().enumerate() {
        let r = s.call(id, params).map_err(|e| {
            let e = e.trim_end_matches('.');
            if unsaved { format!("--cmd {} of {total}: {e}. Nothing was saved.", n + 1) } else { format!("--cmd {} of {total}: {e}.", n + 1) }
        })?;
        if print {
            match r {
                Value::String(said) => write_stdout(|o| writeln!(o, "{id} → {said}"))?,
                r => write_stdout(|o| writeln!(o, "{id} → {r}"))?,
            }
        }
    }
    Ok(())
}

fn save(s: &mut Session, args: &[String], out: &str) -> Result<(), String> {
    let r = if out.to_ascii_lowercase().ends_with(".png") {
        let mut v = json!({"path": out});
        if let Some(n) = page_number(args)? {
            v["page"] = json!(n);
        }
        if let Some(k) = scale(args)? {
            v["scale"] = json!(k);
        }
        s.call("file.exportPng", &v)
    } else {
        s.call("file.save", &json!({"path": out}))
    };
    r.map(|_| ()).map_err(|e| format!("--save: {e}"))
}

/// `--author NAME`: the author recorded on tracked changes and comments (`file.setAuthor`).
fn set_author(s: &mut Session, name: &str) -> Result<(), String> {
    s.run("file.setAuthor", &json!({"name": name})).map(|_| ()).map_err(|e| format!("--author: {e}"))
}

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

#[derive(Clone, Copy)]
struct OptionSpec {
    name: &'static str,
    takes_value: bool,
}

fn validate_options(command: &str, args: &[String], specs: &[OptionSpec]) -> Result<(), String> {
    let mut i = 0;
    while let Some(arg) = args.get(i) {
        if let Some(name) = arg.strip_prefix("--") {
            let Some(spec) = specs.iter().find(|spec| spec.name == arg) else {
                let takes = match specs {
                    [] => "takes no options".to_string(),
                    specs => format!("takes {}", list(&specs.iter().map(|s| s.name).collect::<Vec<_>>())),
                };
                return Err(format!(
                    "{command} was given `--{name}`, which it doesn't take. {command} {takes}; `wordcraft-cli help {command}` explains them."
                ));
            };
            if !spec.takes_value {
                i += 1;
            } else if args.get(i + 1).is_some_and(|value| !value.starts_with("--")) {
                i += 2;
            } else {
                return Err(format!("{command} was given `{arg}` without a value; `wordcraft-cli help {command}` says what it takes."));
            }
        } else {
            i += 1;
        }
    }
    Ok(())
}

/// Take `--automation-read-root` / `--automation-write-root` (each `--flag DIR` or `--flag=DIR`,
/// once) out of `args`, and open the roots, which must be existing folders. Returns the arguments
/// that are left. A malformed root flag is an error, never a root silently left out.
fn automation_roots(
    command: &str,
    args: &[String],
) -> Result<(Option<std::sync::Arc<wordcraft_engine::file_access::AutomationRoots>>, Vec<String>), String> {
    let mut found = automation_args::AutomationArgs::new(None, None);
    let mut left = Vec::new();
    let mut it = args.iter().cloned();
    while let Some(arg) = it.next() {
        if !found.consume(&arg, &mut it).map_err(|e| format!("{command}: {e}"))? {
            left.push(arg);
        }
    }
    let roots = wordcraft_engine::file_access::AutomationRoots::new(found.read_root.as_deref(), found.write_root.as_deref())
        .map_err(|e| format!("{command}: {e}"))?;
    Ok((roots, left))
}

const NO_OPTIONS: &[OptionSpec] = &[];
const CONVERT_OPTIONS: &[OptionSpec] = &[OptionSpec { name: "--markup", takes_value: false }];
const RENDER_OPTIONS: &[OptionSpec] = &[OptionSpec { name: "--page", takes_value: true }, OptionSpec { name: "--scale", takes_value: true }];
const RUN_OPTIONS: &[OptionSpec] = &[
    OptionSpec { name: "--file", takes_value: true },
    OptionSpec { name: "--template", takes_value: true },
    OptionSpec { name: "--cmd", takes_value: true },
    OptionSpec { name: "--save", takes_value: true },
    OptionSpec { name: "--page", takes_value: true },
    OptionSpec { name: "--scale", takes_value: true },
    OptionSpec { name: "--print", takes_value: false },
    OptionSpec { name: "--author", takes_value: true },
];
const COMMANDS_OPTIONS: &[OptionSpec] = &[OptionSpec { name: "--json", takes_value: false }];
const PARITY_OPTIONS: &[OptionSpec] = &[OptionSpec { name: "--markdown", takes_value: false }];
const MCP_OPTIONS: &[OptionSpec] = &[OptionSpec { name: "--connect", takes_value: true }, OptionSpec { name: "--author", takes_value: true }];
const ZOTERO_OPTIONS: &[OptionSpec] = &[
    OptionSpec { name: "--cmd", takes_value: true },
    OptionSpec { name: "--save", takes_value: true },
    OptionSpec { name: "--trace", takes_value: false },
    OptionSpec { name: "--port", takes_value: true },
];

/// Why a CLI run stopped early.
#[derive(Debug, PartialEq)]
enum Failure {
    /// A real error, reported on stderr with a failing exit status.
    Error(String),
    /// Whoever read stdout closed it (`wordcraft-cli commands --json | head`): there is nobody left
    /// to write to, so stop quietly and exit 0 like other command-line tools.
    StdoutClosed,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::Error(e) => f.write_str(e),
            Failure::StdoutClosed => f.write_str("stdout closed"),
        }
    }
}

impl From<String> for Failure {
    fn from(e: String) -> Self {
        Failure::Error(e)
    }
}

impl From<&str> for Failure {
    fn from(e: &str) -> Self {
        Failure::Error(e.into())
    }
}

impl From<std::io::Error> for Failure {
    fn from(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::BrokenPipe { Failure::StdoutClosed } else { Failure::Error(e.to_string()) }
    }
}

/// Write to stdout through one locked handle and flush. Unlike `println!`, a closed stdout comes
/// back as an error (`Failure::StdoutClosed`) instead of a panic.
fn write_stdout(write: impl FnOnce(&mut std::io::StdoutLock<'static>) -> std::io::Result<()>) -> Result<(), Failure> {
    let mut out = std::io::stdout().lock();
    write(&mut out)?;
    out.flush()?;
    Ok(())
}

fn run(args: &[String]) -> Result<(), Failure> {
    let cmd = args.first().map(String::as_str).unwrap_or("");
    let rest: Vec<String> = args.iter().skip(1).cloned().collect();
    if rest.iter().any(|a| a == "--help" || a == "-h")
        && let Some(page) = help::page(cmd)
    {
        return write_stdout(|o| writeln!(o, "{page}"));
    }
    let pos = |i: usize| {
        rest.iter().filter(|a| !a.starts_with("--")).nth(i).cloned().ok_or_else(|| {
            let usage = help::SUBCOMMANDS.iter().find(|s| s.name == cmd).map(|s| s.usage).unwrap_or(cmd);
            format!("{cmd} is missing an argument: usage: wordcraft-cli {usage}")
        })
    };
    match cmd {
        "convert" => {
            validate_options("convert", &rest, CONVERT_OPTIONS)?;
            let (i, o) = (pos(0)?, pos(1)?);
            let markup = rest.iter().any(|a| a == "--markup");
            let mut s = open(&i)?;
            let r = if o.to_ascii_lowercase().ends_with(".png") {
                s.call("file.exportPng", &json!({"path": o, "page": 1, "markup": markup}))
            } else {
                s.call("file.save", &json!({"path": o, "markup": markup}))
            };
            r?;
            write_stdout(|out| writeln!(out, "OK: wrote {o}"))
        }
        "info" => {
            validate_options("info", &rest, NO_OPTIONS)?;
            let mut s = open(&pos(0)?)?;
            let info = serde_json::to_string_pretty(&s.call("file.info", &json!({}))?).unwrap_or_default();
            write_stdout(|o| writeln!(o, "{info}"))
        }
        "text" => {
            validate_options("text", &rest, NO_OPTIONS)?;
            let s = open(&pos(0)?)?;
            write_stdout(|o| writeln!(o, "{}", s.doc.plain_text(wordcraft_doc::StoryRef::Body)))
        }
        "inspect" => {
            validate_options("inspect", &rest, NO_OPTIONS)?;
            let mut s = open(&pos(0)?)?;
            let tree = serde_json::to_string_pretty(&s.call("document.inspect", &json!({}))?).unwrap_or_default();
            write_stdout(|o| writeln!(o, "{tree}"))
        }
        "render" => {
            validate_options("render", &rest, RENDER_OPTIONS)?;
            let (i, o) = (pos(0)?, pos(1)?);
            let page = page_number(&rest)?.unwrap_or(1);
            let scale = scale(&rest)?.unwrap_or(2.0);
            let mut s = open(&i)?;
            let r = s.call("file.exportPng", &json!({"path": o, "page": page, "scale": scale})).map_err(|e| format!("can't render {i}: {e}"))?;
            write_stdout(|o| writeln!(o, "{r}"))
        }
        "run" => {
            validate_options("run", &rest, RUN_OPTIONS)?;
            let reg = wordcraft_engine::cmd::registry();
            let steps = steps(&rest, &reg)?;
            let out = arg_value(&rest, "--save");
            page_number(&rest)?;
            scale(&rest)?;
            let mut s = match (arg_value(&rest, "--file"), arg_value(&rest, "--template")) {
                (Some(f), _) => open(&f)?,
                (None, Some(t)) => from_template(&t)?,
                _ => blank(),
            };
            if let Some(name) = arg_value(&rest, "--author") {
                set_author(&mut s, &name)?;
            }
            let print = rest.iter().any(|a| a == "--print");
            run_steps(&mut s, &steps, print, out.is_some())?;
            let mut done = vec![if steps.len() == 1 { "1 command ran".to_string() } else { format!("{} commands ran", steps.len()) }];
            if let Some(out) = out {
                save(&mut s, &rest, &out)?;
                done.push(format!("saved {out}"));
            }
            write_stdout(|o| writeln!(o, "OK: {}", done.join(", ")))
        }
        "commands" => {
            validate_options("commands", &rest, COMMANDS_OPTIONS)?;
            let reg = wordcraft_engine::cmd::registry();
            let filter = pos(0).unwrap_or_default().to_lowercase();
            let listed =
                reg.all().iter().filter(|c| filter.is_empty() || c.id.to_lowercase().contains(&filter) || c.label.to_lowercase().contains(&filter));
            if rest.iter().any(|a| a == "--json") {
                let ids: std::collections::HashSet<&str> = listed.map(|c| c.id).collect();
                let list: Vec<Value> = reg
                    .describe()
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|c| ids.contains(c["id"].as_str().unwrap_or_default()))
                    .collect();
                let list = serde_json::to_string_pretty(&list).unwrap_or_default();
                write_stdout(|o| writeln!(o, "{list}"))
            } else {
                write_stdout(|o| {
                    for c in listed {
                        match wordcraft_engine::params_doc::parse(c.params) {
                            Ok(d) if d.takes_nothing() => writeln!(o, "{} — {}", c.id, c.label)?,
                            _ => writeln!(o, "{} — {}: {}", c.id, c.label, c.params)?,
                        }
                    }
                    Ok(())
                })
            }
        }
        "describe" => {
            validate_options("describe", &rest, NO_OPTIONS)?;
            let id = pos(0)?;
            let reg = wordcraft_engine::cmd::registry();
            let spec = reg.get(&id).ok_or_else(|| wordcraft_engine::call::unknown_command(&reg, &id))?;
            write_stdout(|o| writeln!(o, "{}", describe(spec)))
        }
        "parity" => {
            validate_options("parity", &rest, PARITY_OPTIONS)?;
            let s = blank();
            let p = wordcraft_engine::catalog::parity(&s.registry);
            if rest.iter().any(|a| a == "--markdown") {
                write_stdout(|o| write!(o, "{}", parity_markdown(&p)))
            } else {
                write_stdout(|o| writeln!(o, "{}", serde_json::to_string_pretty(&p).unwrap_or_default()))
            }
        }
        "mcp" => {
            let (roots, rest) = automation_roots("mcp", &rest)?;
            validate_options("mcp", &rest, MCP_OPTIONS)?;
            let author = arg_value(&rest, "--author");
            let connect = arg_value(&rest, "--connect");
            if roots.is_some() && connect.is_some() {
                return Err(
                    "mcp: --automation-read-root and --automation-write-root confine the headless server; a running app is not confined by them"
                        .into(),
                );
            }
            let backend: Box<dyn wordcraft_mcp::Backend> = match connect {
                // The app's user name is the person's own setting; don't rewrite it from a flag.
                Some(_) if author.is_some() => {
                    return Err("mcp: --author applies to the headless server; with --connect, run file.setAuthor".into());
                }
                Some(addr) => {
                    let remote = wordcraft_mcp::Remote::connect(&addr).map_err(|e| format!("can't reach the WordCraft app at {addr}: {e}"))?;
                    if let Some(w) = remote.key_warning() {
                        eprintln!("wordcraft-cli: {w}");
                    }
                    Box::new(remote)
                }
                None => {
                    let mut headless = wordcraft_mcp::Headless::default();
                    if let Some(roots) = roots {
                        // Every thread of this process, not only the server's: nothing reaches past the roots.
                        wordcraft_engine::file_access::confine_process(roots.clone());
                        let show = |root: Option<&std::path::Path>| root.map_or_else(|| "none".to_string(), |r| r.display().to_string());
                        eprintln!("wordcraft-cli: files confined: read root {}, write root {}", show(roots.read_root()), show(roots.write_root()));
                        headless = headless.with_automation_roots(Some(roots));
                    }
                    if let Some(name) = author {
                        set_author(&mut headless.session, &name)?;
                    }
                    Box::new(headless)
                }
            };
            let mut server = wordcraft_mcp::Server::new(backend);
            let stdin = std::io::stdin();
            // A client that closes our stdout has hung up: `serve` returns BrokenPipe and we end cleanly.
            Ok(server.serve(stdin.lock(), std::io::stdout())?)
        }
        "zotero" => {
            validate_options("zotero", &rest, ZOTERO_OPTIONS)?;
            let name = pos(0)?;
            let command = wordcraft_zotero::Command::from_name(&name).ok_or_else(|| {
                format!("`{name}` is not a Zotero command: zotero takes addEditCitation, addEditBibliography, addNote, refresh, removeCodes or setDocPrefs")
            })?;
            let file = pos(1)?;
            let steps = steps(&rest, &wordcraft_engine::cmd::registry())?;
            let mut opts = wordcraft_zotero::client::Options::default();
            if let Some(p) = arg_value(&rest, "--port") {
                opts.addr.set_port(p.parse().map_err(|_| format!("--port takes a port number, e.g. 23116; `{p}` is not one"))?);
            }
            let mut s = open(&file)?;
            run_steps(&mut s, &steps, false, true)?;
            let trace = rest.iter().any(|a| a == "--trace");
            let mut bridge = wordcraft_zotero::Bridge::new();
            let mut host = wordcraft_zotero::Headless::default();
            let out = wordcraft_zotero::client::run_command(&opts, command, &mut |call| {
                let r = bridge.handle(&mut s, &mut host, call);
                if trace {
                    eprintln!("← {} {}", call.method, clip(&Value::Array(call.args.clone()).to_string()));
                    eprintln!("→ {}", clip(&String::from_utf8_lossy(&wordcraft_zotero::wire::encode_reply(&r))));
                }
                r
            })
            .map_err(|e| e.to_string())?;
            for a in &host.alerts {
                eprintln!("Zotero: {a}");
            }
            eprintln!("{} calls, {}", out.calls, if out.completed { "completed" } else { "ended without completing (cancelled?)" });
            if let Some(o) = arg_value(&rest, "--save") {
                save(&mut s, &rest, &o)?;
                write_stdout(|out| writeln!(out, "OK: saved {o}"))?;
            }
            Ok(())
        }
        "--version" | "-V" | "version" => write_stdout(|o| writeln!(o, "wordcraft-cli {}", env!("CARGO_PKG_VERSION"))),
        "help" | "--help" | "-h" => match rest.first() {
            None => write_stdout(|o| writeln!(o, "{}", help::overview())),
            Some(name) => match help::page(name) {
                Some(page) => write_stdout(|o| writeln!(o, "{page}")),
                None => Err(unknown_subcommand(name).into()),
            },
        },
        "" => Err(format!("wordcraft-cli needs a subcommand.\n\n{}", help::overview()).into()),
        other => Err(unknown_subcommand(other).into()),
    }
}

fn unknown_subcommand(name: &str) -> String {
    let names: Vec<&str> = help::SUBCOMMANDS.iter().map(|s| s.name).filter(|n| *n != "version").collect();
    format!("`{name}` is not a subcommand of wordcraft-cli; the subcommands are {}. `wordcraft-cli help` explains them.", list(&names))
}

fn describe(spec: &wordcraft_engine::CommandSpec) -> String {
    let mut out = format!("{}: {}", spec.id, spec.label);
    if !spec.location.is_empty() {
        out += &format!(", in {}", spec.location);
    }
    if !spec.shortcut.is_empty() {
        out += &format!(", shortcut {}", spec.shortcut);
    }
    out += ".\n";
    let call = match wordcraft_engine::params_doc::parse(spec.params) {
        Ok(doc) => {
            if doc.takes_nothing() {
                out += "\nTakes no parameters.\n";
            } else {
                out += "\nParameters:\n";
                let width = doc.params.iter().map(|p| param_label(p).len()).max().unwrap_or_default();
                for object in 0..doc.objects {
                    let indent = if doc.objects > 1 {
                        out += if object == 0 { "  either:\n" } else { "  or:\n" };
                        "    "
                    } else {
                        "  "
                    };
                    for p in doc.params.iter().filter(|p| p.object == object) {
                        out += &format!("{indent}{:<width$}  {}\n", param_label(p), p.about);
                    }
                }
                if doc.elided {
                    out += &format!("  and {}\n", doc.elision);
                }
            }
            if !doc.remark.is_empty() {
                out += &format!("Note: {}\n", doc.remark.trim_start_matches('(').trim_end_matches(')'));
            }
            if !doc.answer.is_empty() {
                out += &format!("Answers with {}.\n", doc.answer.trim_end_matches('.'));
            }
            if doc.takes_nothing() { format!("--cmd {}", spec.id) } else { format!("--cmd '{}={{…}}'", spec.id) }
        }
        Err(_) => {
            out += &format!("\nParameters: {}\n", spec.params);
            format!("--cmd '{}={{…}}'", spec.id)
        }
    };
    if let Some(why) = (spec.enabled)(&blank()) {
        out += &format!("Not available in a new document: {why}.{}\n", wordcraft_engine::call::how_to_enable(why));
    }
    if ["CharProps", "ParaProps", "Pos"].iter().any(|t| spec.params.contains(t)) {
        out += "CharProps, ParaProps and Pos: `wordcraft-cli help types`.\n";
    }
    out += &format!("\nRun it with: wordcraft-cli run {call}");
    out
}

fn param_label(p: &wordcraft_engine::params_doc::Param<'_>) -> String {
    if p.optional { format!("{}, optional", p.name) } else { p.name.to_string() }
}

/// At most 400 characters of `s`, for traces.
fn clip(s: &str) -> String {
    match s.char_indices().nth(400) {
        Some((i, _)) => format!("{}…", s.get(..i).unwrap_or(s)),
        None => s.to_string(),
    }
}

fn parity_markdown(p: &Value) -> String {
    let mut s = String::from(
        "# WordCraft feature parity\n\nGenerated by `cargo xtask parity` (`wordcraft-cli parity --markdown`): the word-processor feature catalog (`crates/engine/src/catalog.rs`) compared with the live command registry.\n\n",
    );
    s += &format!(
        "**{} of {} catalog features have commands ({}%).**\n\n| Tab | Live | Total |\n|---|---|---|\n",
        p["live"], p["total"], p["percent"]
    );
    for t in p["tabs"].as_array().cloned().unwrap_or_default() {
        s += &format!("| {} | {} | {} |\n", t["tab"].as_str().unwrap_or(""), t["live"], t["total"]);
    }
    s += "\n## Missing\n\n";
    for t in p["tabs"].as_array().cloned().unwrap_or_default() {
        let m = t["missing"].as_array().cloned().unwrap_or_default();
        if m.is_empty() {
            continue;
        }
        s += &format!("### {}\n\n", t["tab"].as_str().unwrap_or(""));
        for x in m {
            s += &format!("- {}\n", x.as_str().unwrap_or(""));
        }
        s += "\n";
    }
    s
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) | Err(Failure::StdoutClosed) => ExitCode::SUCCESS,
        Err(Failure::Error(e)) => {
            eprintln!("wordcraft-cli: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).into()).collect()
    }

    #[test]
    fn validators_accept_only_the_documented_options() {
        assert!(validate_options("info", &args(&["file.docx"]), NO_OPTIONS).is_ok());
        assert!(validate_options("render", &args(&["in.docx", "out.png", "--page", "2", "--scale", "1.5"]), RENDER_OPTIONS).is_ok());
        assert!(validate_options("run", &args(&["--cmd", "text.insert={}", "--cmd", "edit.undo", "--print"]), RUN_OPTIONS).is_ok());
        assert!(validate_options("commands", &args(&["--json"]), COMMANDS_OPTIONS).is_ok());
        assert!(validate_options("parity", &args(&["--markdown"]), PARITY_OPTIONS).is_ok());
        assert!(validate_options("mcp", &args(&["--connect", "127.0.0.1:9000"]), MCP_OPTIONS).is_ok());
        assert_eq!(
            validate_options("render", &args(&["in.docx", "out.png", "--page"]), RENDER_OPTIONS),
            Err("render was given `--page` without a value; `wordcraft-cli help render` says what it takes.".into())
        );
        assert!(
            validate_options(
                "zotero",
                &args(&["refresh", "in.docx", "--cmd", "caret.docEnd", "--save", "out.docx", "--trace", "--port", "23119"]),
                ZOTERO_OPTIONS
            )
            .is_ok()
        );
    }

    #[test]
    fn unknown_options_are_rejected_before_file_access() {
        assert_eq!(
            run(&args(&["info", "missing.docx", "--passwrod", "x"])),
            Err("info was given `--passwrod`, which it doesn't take. info takes no options; `wordcraft-cli help info` explains them.".into())
        );
        assert_eq!(
            run(&args(&["render", "missing.docx", "out.png", "--sclae", "2"])),
            Err("render was given `--sclae`, which it doesn't take. render takes --page or --scale; `wordcraft-cli help render` explains them."
                .into())
        );
        assert_eq!(
            run(&args(&["zotero", "refresh", "missing.docx", "--prot", "1"])),
            Err("zotero was given `--prot`, which it doesn't take. zotero takes --cmd, --save, --trace or --port; `wordcraft-cli help zotero` explains them.".into())
        );
    }

    #[test]
    fn author_flag_names_the_headless_session() {
        assert!(validate_options("run", &args(&["--author", "Claude (copyedit)", "--cmd", "edit.undo"]), RUN_OPTIONS).is_ok());
        assert!(validate_options("mcp", &args(&["--author", "Claude"]), MCP_OPTIONS).is_ok());
        let mut s = Session::new(wordcraft_doc::Document::new());
        assert!(set_author(&mut s, " Claude (copyedit) ").is_ok());
        assert_eq!(s.author, "Claude (copyedit)");
        assert!(set_author(&mut s, "   ").is_err_and(|e| e.starts_with("--author:")));
        assert!(run(&args(&["run", "--author", "Claude", "--cmd", "review.trackChanges={\"value\":true}"])).is_ok());
        // The app's user name is the person's own; a bridge flag must not rewrite it.
        assert!(run(&args(&["mcp", "--connect", "127.0.0.1:9", "--author", "Claude"])).is_err_and(|e| e.to_string().contains("--author")));
    }

    #[test]
    fn automation_root_flags_are_taken_out_of_the_mcp_arguments() {
        let base = std::env::temp_dir().join(format!("wc-cli-roots-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("in")).unwrap();
        std::fs::create_dir_all(base.join("out")).unwrap();
        let (read, write) = (base.join("in").to_string_lossy().into_owned(), base.join("out").to_string_lossy().into_owned());

        let (roots, left) =
            automation_roots("mcp", &args(&["--automation-read-root", &read, "--author", "Claude", &format!("--automation-write-root={write}")]))
                .unwrap();
        let roots = roots.unwrap();
        assert_eq!(left, args(&["--author", "Claude"]));
        assert!(roots.read_root().is_some() && roots.write_root().is_some());
        assert!(validate_options("mcp", &left, MCP_OPTIONS).is_ok());

        // A root left out grants none of its access.
        let (roots, _) = automation_roots("mcp", &args(&["--automation-read-root", &read])).unwrap();
        let roots = roots.unwrap();
        assert!(roots.write_root().is_none());
        assert!(
            roots.check(&format!("{write}/x.docx"), wordcraft_engine::file_access::Access::Write).unwrap_err().contains("write authority is absent")
        );

        // No flags: nothing is confined and the arguments pass through.
        let (roots, left) = automation_roots("mcp", &args(&["--connect", "127.0.0.1:9"])).unwrap();
        assert!(roots.is_none());
        assert_eq!(left, args(&["--connect", "127.0.0.1:9"]));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn automation_root_flags_reject_bad_operands_and_missing_folders() {
        for bad in [
            args(&["--automation-read-root"]),
            args(&["--automation-read-root", "--author"]),
            args(&["--automation-write-root="]),
            args(&["--automation-read-root", "a", "--automation-read-root=b"]),
        ] {
            assert!(automation_roots("mcp", &bad).is_err(), "{bad:?}");
        }
        let e = automation_roots("mcp", &args(&["--automation-read-root", "/definitely/not/a/folder"])).err().unwrap();
        assert!(e.starts_with("mcp: cannot open the automation read root"), "{e}");
        // Through the command line: an error, not a server that quietly runs unconfined.
        assert!(run(&args(&["mcp", "--automation-write-root", "/definitely/not/a/folder"])).is_err());
    }

    #[test]
    fn automation_roots_are_an_error_with_connect() {
        let dir = std::env::temp_dir().to_string_lossy().into_owned();
        let e = run(&args(&["mcp", "--connect", "127.0.0.1:9", "--automation-read-root", &dir])).unwrap_err().to_string();
        assert!(e.contains("--automation-read-root") && e.contains("headless"), "{e}");
    }
}

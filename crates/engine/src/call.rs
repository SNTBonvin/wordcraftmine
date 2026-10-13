//! Commands run for a caller outside the app (the CLI, MCP, the control channel), whose answers
//! and errors are read by an agent as they are.

use serde_json::{Value, json};

use crate::{CmdError, CommandSpec, Registry, Session, describe, params_doc};

impl Session {
    /// Run command `id` for a caller outside the app. Parameters the command doesn't take are
    /// refused, an error says what is wrong and the form that works, and a command that answers
    /// with the selection alone answers instead with what it changed, in words; a mutating
    /// command, or one that moved the selection, that answers with JSON carries the same under
    /// `changed`.
    pub fn call(&mut self, id: &str, params: &Value) -> Result<Value, String> {
        let reg = self.registry.clone();
        let Some(spec) = reg.get(id) else { return Err(unknown_command(&reg, id)) };
        check_params(spec, params)?;
        let before = describe::Before::of(self);
        let sel = self.sel.clone();
        let r = self.run(id, params).map_err(|e| worded(spec, &e))?;
        if r == json!({"anchor": self.sel.anchor, "focus": self.sel.focus}) {
            return Ok(Value::String(describe::changes(&before, self)));
        }
        match r {
            Value::Object(mut answer) if spec.mutates || sel != self.sel => {
                answer.insert("changed".into(), Value::String(describe::changes(&before, self)));
                Ok(Value::Object(answer))
            }
            r => Ok(r),
        }
    }
}

/// The error for a command id that isn't one, naming the ids like it.
pub fn unknown_command(reg: &Registry, id: &str) -> String {
    let like = similar_commands(reg, id);
    match like.len() {
        0 => format!("`{id}` is not a command."),
        n if n > 8 => format!("`{id}` is not a command; commands named like it: {} and {} more.", like[..8].join(", "), n - 8),
        _ => format!("`{id}` is not a command; commands named like it: {}.", like.join(", ")),
    }
}

/// Command ids like `id`: the same last segment first, then ids containing it.
pub fn similar_commands(reg: &Registry, id: &str) -> Vec<&'static str> {
    let q = id.trim().to_lowercase();
    if q.is_empty() {
        return vec![];
    }
    let last = q.rsplit('.').next().unwrap_or_default();
    let rank = |c: &str| {
        let c = c.to_lowercase();
        let l = c.rsplit('.').next().unwrap_or_default();
        if l == last {
            Some(0)
        } else if c.contains(&q) {
            Some(1)
        } else if last.len() >= 3 && l.starts_with(last) {
            Some(2)
        } else {
            None
        }
    };
    let mut like: Vec<(u8, &'static str)> = reg.all().iter().filter_map(|c| rank(c.id).map(|r| (r, c.id))).collect();
    like.sort_by_key(|(r, _)| *r);
    like.into_iter().map(|(_, c)| c).collect()
}

impl Session {
    /// Refuse `params` a registered command `id` doesn't take, as [`Self::call`] does; an id the
    /// registry doesn't know passes, for the caller to answer.
    pub fn check_params(&self, id: &str, params: &Value) -> Result<(), String> {
        self.registry.get(id).map_or(Ok(()), |spec| check_params(spec, params))
    }
}

pub(crate) fn check_params(spec: &CommandSpec, params: &Value) -> Result<(), String> {
    let given = match params {
        Value::Null => return Ok(()),
        Value::Object(m) => m,
        Value::Array(_) => return Err(bad_params(spec, "the parameters are a JSON object, not an array")),
        Value::String(_) => return Err(bad_params(spec, "the parameters are a JSON object, not a string")),
        Value::Number(_) => return Err(bad_params(spec, "the parameters are a JSON object, not a number")),
        Value::Bool(_) => return Err(bad_params(spec, "the parameters are a JSON object, not a boolean")),
    };
    let Ok(doc) = params_doc::parse(spec.params) else { return Ok(()) };
    let Some(known) = doc.keys() else { return Ok(()) };
    let unknown: Vec<String> = given.keys().filter(|k| !known.contains(&k.as_str())).map(|k| format!("`{k}`")).collect();
    if unknown.is_empty() {
        return Ok(());
    }
    Err(format!("`{}` was given {}, which it doesn't take.{}", spec.id, unknown.join(", "), takes(spec)))
}

/// ` `id` takes {params doc}.`, to end a parameter error with the form that works.
pub fn takes(spec: &CommandSpec) -> String {
    match params_doc::parse(spec.params) {
        Ok(d) if d.takes_nothing() => format!(" `{}` takes no parameters.", spec.id),
        _ => format!(" `{}` takes {}.", spec.id, spec.params),
    }
}

fn bad_params(spec: &CommandSpec, msg: &str) -> String {
    format!("invalid parameters for `{}`: {}.{}", spec.id, msg.trim_end_matches('.'), takes(spec))
}

fn worded(spec: &CommandSpec, e: &CmdError) -> String {
    match e {
        CmdError::Params(msg) => bad_params(spec, msg),
        CmdError::Disabled(msg) => not_available(spec.id, msg.strip_prefix(&format!("{}: ", spec.id)).unwrap_or(msg)),
        CmdError::Unknown(id) => format!("`{id}` is not a command."),
        CmdError::Failed(msg) => msg.clone(),
    }
}

/// The error for a command its gate refuses, with the call that makes it available.
pub fn not_available(id: &str, why: &str) -> String {
    format!("`{id}` is not available right now: {}.{}", why.trim_end_matches('.'), how_to_enable(why))
}

/// The call that makes a command its gate refuses for `why` available, as a sentence to append.
pub fn how_to_enable(why: &str) -> &'static str {
    match why {
        "nothing is selected" => " select.text {\"text\": …}, select.paragraph or select.all selects.",
        "the cursor isn't in a table" => {
            " insert.table inserts one and puts the caret in its first cell; caret.set {\"pos\": {\"path\": [block, row, cell, paragraph], \"off\": 0}} moves into one."
        }
        "select a picture first" | "select a picture or shape first" | "select a shape first" | "select a floating picture or shape first" => {
            " insert.picture or insert.shape inserts one and selects it; select.objects {\"index\": n} selects an existing one."
        }
        "select recipients first" => " mailings.recipients {\"path\": …} loads them.",
        "not editing an equation" | "no equation at the caret" => " insert.equation inserts one; equation.edit starts editing the one at the caret.",
        "only in Outline view" => " view.outline switches to it.",
        "the Clipboard pane is empty" => " edit.copy or edit.cut fills it.",
        "select a drawing canvas first" => " insert.canvas inserts one and selects it.",
        "select a chart first" => " insert.chart inserts one and selects it.",
        "select a SmartArt graphic first" => " insert.smartArt inserts one and selects it.",
        _ => "",
    }
}

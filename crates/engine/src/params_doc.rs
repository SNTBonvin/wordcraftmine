//! The params doc of a command, read as data: `{"text": string, "occurrence"?: n}` names the
//! keys a command takes, `?` marks an optional one, `|` between objects separates alternative
//! parameter sets, `…` elides keys listed elsewhere, `→` starts the answer, and text
//! after the objects is a remark.

/// One parameter of a params doc.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param<'a> {
    pub name: &'a str,
    pub optional: bool,
    /// What the doc says it takes: the text after `:`.
    pub about: &'a str,
    /// Which alternative object (`{…} | {…}`) it belongs to, from 0.
    pub object: usize,
}

/// A params doc read as data; see the module.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ParamsDoc<'a> {
    pub params: Vec<Param<'a>>,
    /// How many alternative objects the doc has, at least 1.
    pub objects: usize,
    /// Whether a `…` elides keys the doc doesn't list.
    pub elided: bool,
    /// The text after `…`, e.g. `options of edit.find`.
    pub elision: &'a str,
    /// What the command answers with: the text after `→`.
    pub answer: &'a str,
    /// Free text after the objects.
    pub remark: &'a str,
}

impl ParamsDoc<'_> {
    /// Every key the command takes, or `None` when the doc elides some.
    pub fn keys(&self) -> Option<Vec<&str>> {
        if self.elided {
            return None;
        }
        let mut keys = vec![];
        for p in &self.params {
            if !keys.contains(&p.name) {
                keys.push(p.name);
            }
        }
        Some(keys)
    }

    /// The doc is `{}`: the command takes no parameters.
    pub fn takes_nothing(&self) -> bool {
        self.params.is_empty() && !self.elided
    }

    /// Some alternative needs no parameter, so the command runs as its bare id.
    pub fn all_optional(&self) -> bool {
        (0..self.objects).any(|o| self.params.iter().filter(|p| p.object == o).all(|p| p.optional))
    }
}

/// Read a params doc. Errs where the doc strays from the grammar, naming the place.
pub fn parse(doc: &str) -> Result<ParamsDoc<'_>, String> {
    let (doc, answer) = match doc.split_once('→') {
        Some((d, a)) => (d, a.trim()),
        None => (doc, ""),
    };
    let mut out = ParamsDoc { answer, ..Default::default() };
    let mut at = skip_ws(doc, 0);
    loop {
        if doc.get(at..).is_none_or(|r| !r.starts_with('{')) {
            return Err(format!("expected `{{` at `{}`", clip(doc, at)));
        }
        let end = parse_object(doc, at, out.objects, &mut out)?;
        out.objects += 1;
        let after = skip_ws(doc, end);
        match doc.get(after..) {
            Some(rest) if rest.starts_with('|') => at = skip_ws(doc, after + 1),
            Some(rest) => {
                out.remark = rest.trim().trim_start_matches('—').trim();
                break;
            }
            None => break,
        }
    }
    Ok(out)
}

fn parse_object<'a>(doc: &'a str, open: usize, object: usize, out: &mut ParamsDoc<'a>) -> Result<usize, String> {
    let mut at = skip_ws(doc, open + 1);
    loop {
        let rest = doc.get(at..).ok_or_else(|| "unterminated `{`".to_string())?;
        if rest.starts_with('}') {
            return Ok(at + 1);
        }
        if rest.starts_with('…') {
            out.elided = true;
            let end = skip_value(doc, at)?;
            out.elision = doc.get(at + '…'.len_utf8()..end).unwrap_or_default().trim();
            at = end;
        } else if rest.starts_with('"') {
            let mut names = vec![];
            loop {
                let (name, next) = quoted(doc, at)?;
                names.push(name);
                match doc.get(next..) {
                    Some(r) if r.starts_with("|\"") => at = next + 1,
                    _ => {
                        at = next;
                        break;
                    }
                }
            }
            let mut optional = false;
            if doc.get(at..).is_some_and(|r| r.starts_with('?')) {
                optional = true;
                at += 1;
            }
            let about_from = skip_ws(doc, at);
            let about_from = match doc.get(about_from..) {
                Some(r) if r.starts_with(':') => skip_ws(doc, about_from + 1),
                _ => about_from,
            };
            let end = skip_value(doc, about_from)?;
            let about = doc.get(about_from..end).unwrap_or_default().trim();
            for name in names {
                out.params.push(Param { name, optional, about, object });
            }
            at = end;
        } else {
            return Err(format!("expected a key at `{}`", clip(doc, at)));
        }
        at = skip_ws(doc, at);
        if doc.get(at..).is_some_and(|r| r.starts_with(',')) {
            at = skip_ws(doc, at + 1);
        }
    }
}

fn quoted(doc: &str, at: usize) -> Result<(&str, usize), String> {
    let inner = at + 1;
    let len = doc.get(inner..).and_then(|r| r.find('"')).ok_or_else(|| format!("unterminated key at `{}`", clip(doc, at)))?;
    let name = doc.get(inner..inner + len).unwrap_or_default();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(format!("`{name}` is not a key"));
    }
    Ok((name, inner + len + 1))
}

fn skip_value(doc: &str, from: usize) -> Result<usize, String> {
    let mut depth = 0usize;
    let mut prev = ' ';
    let mut in_string = false;
    for (i, c) in doc.get(from..).unwrap_or_default().char_indices() {
        let at = from + i;
        if in_string {
            if c == '"' {
                in_string = false;
            }
        } else {
            match c {
                '"' if prev.is_whitespace() || matches!(prev, ':' | '|' | '(' | '[' | '{' | ',') => in_string = true,
                '{' | '[' | '(' => depth += 1,
                '}' | ']' | ')' if depth > 0 => depth -= 1,
                '}' | ',' if depth == 0 => return Ok(at),
                _ => {}
            }
        }
        prev = c;
    }
    Err(format!("unterminated value at `{}`", clip(doc, from)))
}

fn skip_ws(doc: &str, mut at: usize) -> usize {
    while doc.get(at..).is_some_and(|r| r.starts_with(char::is_whitespace)) {
        at += doc.get(at..).and_then(|r| r.chars().next()).map_or(1, char::len_utf8);
    }
    at
}

fn clip(doc: &str, at: usize) -> String {
    doc.get(at..).unwrap_or_default().chars().take(24).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p<'a>(name: &'a str, optional: bool, about: &'a str, object: usize) -> Param<'a> {
        Param { name, optional, about, object }
    }

    #[test]
    fn keys_with_types_and_optionals() {
        let d = parse(r#"{"text": string, "occurrence"?: n}"#).unwrap();
        assert_eq!(d.params, vec![p("text", false, "string", 0), p("occurrence", true, "n", 0)]);
        assert_eq!(d.keys(), Some(vec!["text", "occurrence"]));
        assert_eq!(d.objects, 1);
        assert!(!d.all_optional());
    }

    #[test]
    fn an_empty_doc_takes_nothing() {
        let d = parse("{}").unwrap();
        assert!(d.takes_nothing());
        assert_eq!(d.keys(), Some(vec![]));
        assert!(d.all_optional());
    }

    #[test]
    fn alternative_objects_and_an_answer() {
        let d = parse(r#"{"pos": Pos, "extend"?: bool} | {"page": n, "x": pt, "y": pt} → {"anchor", "focus"}"#).unwrap();
        assert_eq!(d.objects, 2);
        assert_eq!(d.params.iter().map(|p| (p.name, p.object)).collect::<Vec<_>>(), vec![("pos", 0), ("extend", 0), ("page", 1), ("x", 1), ("y", 1)]);
        assert_eq!(d.answer, r#"{"anchor", "focus"}"#);
    }

    #[test]
    fn values_may_hold_bars_quotes_and_brackets() {
        let d = parse(r#"{"color": "RRGGBB" | null, "fill"?: "RRGGBB"|"none" (hollow)|{"color": "RRGGBB", "transparency"?: 0-100}, "width"?: pt (default 432 = 6")}"#)
            .unwrap();
        assert_eq!(d.keys(), Some(vec!["color", "fill", "width"]));
        assert_eq!(d.params[0].about, r#""RRGGBB" | null"#);
        assert_eq!(d.params[2].about, r#"pt (default 432 = 6")"#);
    }

    #[test]
    fn keys_sharing_a_value_and_keys_without_one() {
        let d = parse(r#"{"moves"|"comments"|"tables"?: bool (accepted), "title"?, "subject"?}"#).unwrap();
        assert_eq!(
            d.params,
            vec![
                p("moves", true, "bool (accepted)", 0),
                p("comments", true, "bool (accepted)", 0),
                p("tables", true, "bool (accepted)", 0),
                p("title", true, "", 0),
                p("subject", true, "", 0)
            ]
        );
    }

    #[test]
    fn an_elision_means_the_keys_are_not_all_listed() {
        let d = parse(r#"{"text"?: string, …options of edit.find, "in"?: "main|selection"}"#).unwrap();
        assert!(d.elided);
        assert_eq!(d.keys(), None);
        assert_eq!(d.params.iter().map(|p| p.name).collect::<Vec<_>>(), vec!["text", "in"]);
    }

    #[test]
    fn a_remark_follows_the_objects() {
        let d = parse(r#"{"name": string, "wholeTable"?: Region} where Region = {"borders"?: bool | null}"#).unwrap();
        assert_eq!(d.keys(), Some(vec!["name", "wholeTable"]));
        assert_eq!(d.remark, r#"where Region = {"borders"?: bool | null}"#);
        let d = parse("{}  (groups the selected pictures)").unwrap();
        assert_eq!(d.remark, "(groups the selected pictures)");
    }

    #[test]
    fn strays_from_the_grammar_are_errors() {
        assert!(parse("text: string").is_err());
        assert!(parse(r#"{"text": string"#).is_err());
        assert!(parse(r#"{text: string}"#).is_err());
        assert!(parse(r#"{"a b": string}"#).is_err());
    }
}

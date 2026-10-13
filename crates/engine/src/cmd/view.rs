//! View tab and status bar: views, zoom, show/hide, panes.

use serde_json::{Value, json};
use wordcraft_layout::{NumeralMode, ViewMode};

use crate::{CmdError, CmdResult, CommandSpec, Session, p};

fn toggle(s: &mut Session, v: &Value, f: fn(&mut crate::ViewState) -> &mut bool) -> CmdResult {
    let cur = *f(&mut s.view);
    *f(&mut s.view) = p::bool(v, "value").unwrap_or(!cur);
    Ok(json!({"value": *f(&mut s.view)}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("view.marks", "Show/Hide ¶", "Home › Paragraph", |s, v| {
            let r = toggle(s, v, |x| &mut x.marks);
            s.relayout();
            r
        })
        .key("Mod+Shift+8 / Mod+8")
        .pure(),
        CommandSpec::new("view.printLayout", "Print Layout", "View › Views", |s, _| mode(s, ViewMode::Print, false)).key("Mod+Alt+P").pure(),
        CommandSpec::new("view.webLayout", "Web Layout", "View › Views", |s, _| mode(s, ViewMode::Web, false)).pure(),
        CommandSpec::new("view.draft", "Draft", "View › Views", |s, _| mode(s, ViewMode::Draft, false)).key("Mod+Alt+N").pure(),
        // The body as an outline of its headings, with the Outlining tab (`outline.*`).
        CommandSpec::new("view.outline", "Outline", "View › Views", |s, _| mode(s, ViewMode::Outline, false)).key("Mod+Alt+O").pure(),
        CommandSpec::new("view.readMode", "Read Mode", "View › Views", |s, _| mode(s, ViewMode::Print, true)).pure(),
        CommandSpec::new("view.readColumnWidth", "Reading Column Width", "Reading › Layout", read_column_width)
            .params(r#"{"value": "auto" | "narrow" | "medium" | "wide" | "page"}"#)
            .pure(),
        CommandSpec::new("view.readTextSize", "Reading Text Size", "Reading › Layout", read_text_size)
            .params(r#"{"value": percent (75–150)}"#)
            .pure(),
        CommandSpec::new("view.readLineSpacing", "Reading Line Spacing", "Reading › Layout", read_line_spacing)
            .params(r#"{"value": "document" | 1.0 | 1.25 | 1.5 | 2.0}"#)
            .pure(),
        // Print Layout: hide the white space (margins, headers, footers) between pages. Screen only.
        CommandSpec::new("view.hideWhiteSpace", "Hide White Space", "View › Show", |s, v| toggle(s, v, |x| &mut x.hide_white_space))
            .params(r#"{"value"?: bool}"#)
            .pure(),
        CommandSpec::new("view.focus", "Focus", "View › Immersive", |s, v| toggle(s, v, |x| &mut x.focus_mode)).pure(),
        CommandSpec::new("view.ruler", "Ruler", "View › Show", |s, v| toggle(s, v, |x| &mut x.ruler)).pure(),
        // The drawing grid over the page (screen only; never printed or exported).
        CommandSpec::new("view.gridlines", "Gridlines", "View › Show", |s, v| toggle(s, v, |x| &mut x.gridlines))
            .params(r#"{"value"?: bool}"#)
            .pure(),
        // Outlines of table cells (screen only; never printed or exported).
        CommandSpec::new("table.viewGridlines", "View Gridlines", "Table Layout › Table", |s, v| toggle(s, v, |x| &mut x.table_gridlines))
            .params(r#"{"value"?: bool}"#)
            .pure(),
        CommandSpec::new("view.navigationPane", "Navigation Pane", "View › Show", |s, v| toggle(s, v, |x| &mut x.nav_pane)).pure(),
        CommandSpec::new("view.zoom", "Zoom", "View › Zoom", zoom)
            .params(r#"{"value": percent (10-500) | "pageWidth" | "onePage" | "multiplePages"}"#)
            .pure(),
        CommandSpec::new("view.zoom100", "100%", "View › Zoom", |s, _| zoom(s, &json!({"value": 100}))).pure(),
        CommandSpec::new("view.zoomIn", "Zoom In", "View › Zoom", |s, _| {
            let z = ((s.view.zoom * 10.0).round() / 10.0 + 0.1).min(5.0);
            zoom(s, &json!({"value": z * 100.0}))
        })
        .pure(),
        CommandSpec::new("view.zoomOut", "Zoom Out", "View › Zoom", |s, _| {
            let z = ((s.view.zoom * 10.0).round() / 10.0 - 0.1).max(0.1);
            zoom(s, &json!({"value": z * 100.0}))
        })
        .pure(),
        CommandSpec::new("view.onePage", "One Page", "View › Zoom", |s, _| zoom(s, &json!({"value": "onePage"}))).pure(),
        CommandSpec::new("view.multiplePages", "Multiple Pages", "View › Zoom", |s, _| zoom(s, &json!({"value": "multiplePages"}))).pure(),
        CommandSpec::new("view.pageWidth", "Page Width", "View › Zoom", |s, _| zoom(s, &json!({"value": "pageWidth"}))).pure(),
        CommandSpec::new("view.darkMode", "Switch Modes", "View › Dark Mode", |s, v| toggle(s, v, |x| &mut x.dark_mode)).pure(),
        CommandSpec::new("view.stylesPane", "Styles Pane", "Home › Styles", |s, v| toggle(s, v, |x| &mut x.styles_pane)).pure(),
        CommandSpec::new("view.commentsPane", "Comments Pane", "Review › Comments", |s, v| toggle(s, v, |x| &mut x.comments_pane)).pure(),
        CommandSpec::new("view.accessibilityPane", "Accessibility Pane", "Review › Accessibility", |s, v| {
            toggle(s, v, |x| &mut x.accessibility_pane)
        })
        .params(r#"{"value"?: bool}"#)
        .pure(),
        CommandSpec::new("view.altTextPane", "Alt Text Pane", "Review › Accessibility", |s, v| toggle(s, v, |x| &mut x.alt_text_pane))
            .params(r#"{"value"?: bool}"#)
            .pure(),
        CommandSpec::new("view.newWindow", "New Window", "View › Window", new_window).pure(),
        CommandSpec::new("view.split", "Split", "View › Window", split)
            .key("Mod+Alt+S")
            .params(r#"{"value"?: bool, "ratio"?: 10-90 (percent) | 0.1-0.9}"#)
            .pure(),
        CommandSpec::new("view.arrangeAll", "Arrange All", "View › Window", arrange_all).params(r#"{"tiling"?: "vertical" | "horizontal"}"#).pure(),
        CommandSpec::new("view.sideBySide", "View Side by Side", "View › Window", side_by_side).params(r#"{"value"?: bool}"#).pure(),
        CommandSpec::new("view.syncScroll", "Synchronous Scrolling", "View › Window", sync_scroll)
            .params(r#"{"value"?: bool}"#)
            .when(sync_scroll_enabled)
            .pure(),
        CommandSpec::new("view.switchWindows", "Switch Windows", "View › Window", switch_windows)
            .params(r#"{"index"?: 0-based window, "next"?: bool, "previous"?: bool}"#)
            .when(switch_windows_enabled)
            .pure(),
        CommandSpec::new("view.state", "View State", "View", |s, _| serde_json::to_value(&s.view).map_err(|e| CmdError::Failed(e.to_string())))
            .pure(),
        // Word › File › Options › Advanced › Numeral: how digits look on screen (display
        // only; the document text keeps the typed codepoints).
        CommandSpec::new("view.numeral", "Numerals", "File › Options › Advanced", |s, v| match v.get("value") {
            None => Ok(json!({"value": s.view.numeral})),
            Some(Value::String(name)) => {
                let Some(mode) = NumeralMode::parse(name) else {
                    return Err(CmdError::Params("numeral mode: arabic|hindi|context|system".into()));
                };
                s.view.numeral = mode;
                s.relayout();
                Ok(json!({"value": s.view.numeral}))
            }
            Some(_) => Err(CmdError::Params("numeral mode: arabic|hindi|context|system".into())),
        })
        .params(r#"{"value"?: "arabic|hindi|context|system"}"#)
        .pure(),
    ]
}

fn mode(s: &mut Session, m: ViewMode, read: bool) -> CmdResult {
    if read && !s.view.read_mode {
        // Reading controls are session-only, and reset on each entry to the view.
        s.view.read_column_width = None;
        s.view.read_zoom = 1.0;
        s.view.read_line_spacing = None;
    }
    s.view.mode = m;
    s.view.read_mode = read;
    s.relayout();
    Ok(json!({"mode": m, "readMode": read}))
}

fn read_column_width(s: &mut Session, v: &Value) -> CmdResult {
    let width = match v.get("value").and_then(Value::as_str) {
        Some("auto") => None,
        Some("narrow") => Some(180.0),
        Some("medium") => Some(240.0),
        Some("wide") => Some(350.0),
        Some("page") => Some(612.0),
        _ => return Err(CmdError::Params("column width must be auto, narrow, medium, wide, or page".into())),
    };
    s.view.read_column_width = width;
    Ok(json!({"width": width}))
}

fn read_text_size(s: &mut Session, v: &Value) -> CmdResult {
    let Some(pct) = v.get("value").and_then(Value::as_f64) else {
        return Err(CmdError::Params("reading text size must be a percentage".into()));
    };
    if !pct.is_finite() || !(75.0..=150.0).contains(&pct) {
        return Err(CmdError::Params("reading text size must be 75–150%".into()));
    }
    s.view.read_zoom = pct as f32 / 100.0;
    Ok(json!({"percent": pct}))
}

fn read_line_spacing(s: &mut Session, v: &Value) -> CmdResult {
    let value = v.get("value");
    let spacing = match value {
        Some(Value::String(s)) if s == "document" => None,
        Some(Value::Number(n)) if matches!(n.as_f64(), Some(1.0 | 1.25 | 1.5 | 2.0)) => n.as_f64().map(|n| n as f32),
        _ => return Err(CmdError::Params("reading line spacing must be document, 1, 1.25, 1.5, or 2".into())),
    };
    s.view.read_line_spacing = spacing;
    Ok(json!({"spacing": spacing}))
}

fn zoom(s: &mut Session, v: &Value) -> CmdResult {
    match v.get("value") {
        Some(Value::String(f)) if ["pageWidth", "onePage", "multiplePages", "textWidth"].contains(&f.as_str()) => {
            s.view.fit = f.clone();
            s.view.multi_page = f == "multiplePages";
            s.view.single_column = f == "onePage" || f == "pageWidth";
        }
        Some(x) => {
            let pct = x.as_f64().ok_or_else(|| CmdError::Params("zoom percent or fit mode".into()))? as f32;
            if !(10.0..=500.0).contains(&pct) {
                return Err(CmdError::Params("zoom must be 10–500%".into()));
            }
            s.view.zoom = pct / 100.0;
            s.view.fit.clear();
            s.view.multi_page = false;
        }
        None => {
            s.ui_requests.push(json!({"open": "zoom"}));
        }
    }
    Ok(json!({"zoom": s.view.zoom, "fit": s.view.fit}))
}

/// Extra views of the same document (View › Window › New Window, #27).
/// In-window tiling: every window shows the same `doc`; the UI tiles the panes.
pub const MAX_WINDOWS: usize = 16;

fn clamp_windows(s: &mut Session) {
    s.view.window_count = s.view.window_count.clamp(1, MAX_WINDOWS);
    s.view.active_window = s.view.active_window.min(s.view.window_count - 1);
}

fn new_window(s: &mut Session, _: &Value) -> CmdResult {
    clamp_windows(s);
    if s.view.window_count < MAX_WINDOWS {
        s.view.window_count += 1;
        s.view.active_window = s.view.window_count - 1;
    }
    // A second window must be visible somewhere: tile it in-window (screen only).
    if s.view.window_count > 1 && !s.view.split && !s.view.side_by_side {
        s.view.side_by_side = true;
    }
    Ok(json!({"windows": s.view.window_count, "active": s.view.active_window, "sideBySide": s.view.side_by_side}))
}

fn split_ratio_from(v: &Value) -> Result<Option<f32>, CmdError> {
    let Some(n) = v.get("ratio").and_then(Value::as_f64) else { return Ok(None) };
    if !n.is_finite() {
        return Err(CmdError::Params("split ratio must be 10–90 (percent) or 0.1–0.9".into()));
    }
    // Accept a percent (10–90) or a fraction (0.1–0.9); both clamp to the divider range.
    let ratio = if (10.0..=90.0).contains(&n) {
        (n / 100.0) as f32
    } else if (0.1..=0.9).contains(&n) {
        n as f32
    } else {
        return Err(CmdError::Params("split ratio must be 10–90 (percent) or 0.1–0.9".into()));
    };
    Ok(Some(ratio.clamp(0.1, 0.9)))
}

fn split(s: &mut Session, v: &Value) -> CmdResult {
    let ratio = split_ratio_from(v)?;
    // A bare call toggles the panes; with only a ratio the panes stay as they are.
    let value = match p::bool(v, "value") {
        Some(explicit) => explicit,
        None if ratio.is_some() => s.view.split,
        None => !s.view.split,
    };
    s.view.split = value;
    if let Some(r) = ratio {
        s.view.split_ratio = r;
    }
    if !s.view.split_ratio.is_finite() {
        s.view.split_ratio = 0.5;
    }
    s.view.split_ratio = s.view.split_ratio.clamp(0.1, 0.9);
    Ok(json!({"split": s.view.split, "ratio": s.view.split_ratio}))
}

fn side_by_side(s: &mut Session, v: &Value) -> CmdResult {
    let value = p::bool(v, "value").unwrap_or(!s.view.side_by_side);
    s.view.side_by_side = value;
    if !value {
        s.view.sync_scroll = false;
    }
    Ok(json!({"sideBySide": s.view.side_by_side, "syncScroll": s.view.sync_scroll}))
}

fn sync_scroll_enabled(s: &Session) -> Option<&'static str> {
    if s.view.side_by_side { None } else { Some("Turn on Side by Side first") }
}

fn sync_scroll(s: &mut Session, v: &Value) -> CmdResult {
    let value = p::bool(v, "value").unwrap_or(!s.view.sync_scroll);
    if value && !s.view.side_by_side {
        return Err(CmdError::Disabled("view.syncScroll: Turn on Side by Side first".into()));
    }
    s.view.sync_scroll = value;
    Ok(json!({"syncScroll": s.view.sync_scroll}))
}

fn arrange_all(s: &mut Session, v: &Value) -> CmdResult {
    let tiling = match v.get("tiling").and_then(Value::as_str) {
        None => "vertical",
        Some("vertical" | "horizontal") => v.get("tiling").and_then(Value::as_str).unwrap_or("vertical"),
        Some(_) => return Err(CmdError::Params("tiling must be vertical or horizontal".into())),
    };
    s.view.arrange = tiling.to_string();
    // With a single window and no split there would be nothing to tile: show the
    // side panes so Arrange All is visible (screen only; the document is unchanged).
    if !s.view.split && !s.view.side_by_side {
        s.view.side_by_side = true;
    }
    clamp_windows(s);
    Ok(
        json!({"tiling": s.view.arrange, "windows": s.view.window_count, "active": s.view.active_window, "sideBySide": s.view.side_by_side, "split": s.view.split}),
    )
}

fn switch_windows_enabled(s: &Session) -> Option<&'static str> {
    if s.view.window_count > 1 { None } else { Some("Only one window open") }
}

fn switch_windows(s: &mut Session, v: &Value) -> CmdResult {
    clamp_windows(s);
    if s.view.window_count <= 1 {
        return Err(CmdError::Disabled("view.switchWindows: Only one window open".into()));
    }
    let count = s.view.window_count;
    if v.get("index").is_some() {
        let Some(i) = p::u64(v, "index") else {
            return Err(CmdError::Params("index must be a window number from 0".into()));
        };
        if i as usize >= count {
            return Err(CmdError::Params("index is past the last window".into()));
        }
        s.view.active_window = i as usize;
    } else if p::bool(v, "previous").unwrap_or(false) {
        s.view.active_window = (s.view.active_window + count - 1) % count;
    } else {
        // `{"next": true}` or no params: cycle forward (the Switch Windows menu's
        // default: move to the next open window).
        s.view.active_window = (s.view.active_window + 1) % count;
    }
    Ok(json!({"active": s.view.active_window, "windows": s.view.window_count}))
}

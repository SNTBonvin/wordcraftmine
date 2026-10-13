//! Insert tab: online pictures, screenshots, online video and 3D models.
//!
//! Offline-safe: WordCraft never fetches a URL. Remote content becomes a clearly
//! labelled placeholder picture (real PNG bytes drawn from the original icon
//! library, `wordcraft_render::icon_lib`) that behaves like any picture: it lays
//! out, renders, exports and round-trips through DOCX, and `picture.*`,
//! `arrange.*` and alt-text commands all work on it. Bytes the caller already
//! has (`data`, or a gated `path`) insert as a real picture.

use serde_json::{Value, json};
use wordcraft_doc::props::Rgb;

use super::insert::{base64_decode, place_picture};
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("insert.onlinePicture", "Online Pictures", "Insert › Illustrations", online_picture)
            .params(r#"{"url"?: string, "path"?: string, "data"?: base64, "width"?: pt, "alt"?: string}  (a url inserts a labelled placeholder: WordCraft never fetches it)"#),
        CommandSpec::new("insert.screenshot", "Screenshot", "Insert › Illustrations", screenshot)
            .params(r#"{"source"?: "window"|"screen", "data"?: base64, "width"?: pt, "alt"?: string}  (capture isn't available headlessly: pass data, or paste)"#),
        CommandSpec::new("insert.video", "Online Video", "Insert › Media", video)
            .params(r#"{"url"?: string, "path"?: string, "poster"?: base64 (a poster frame), "width"?: pt, "alt"?: string}  (a url inserts a labelled poster placeholder: WordCraft never fetches it)"#),
        CommandSpec::new("insert.model3d", "3D Models", "Insert › Illustrations", model_3d)
            .params(r#"{"path"?: string, "format"?: "glb|gltf|obj|stl|fbx|3mf|usdz", "width"?: pt, "alt"?: string}  (inserts a labelled placeholder that keeps the file name)"#),
        CommandSpec::new("addins.get", "Get Add-ins", "Insert › Add-ins", get_addins)
            .params(r#"{"query"?: string}  (Office add-ins aren't supported: always an empty list)"#)
            .pure(),
    ]
}

/// A `url` param, trimmed: visible text, 1–2000 characters. URLs are recorded in
/// alt text and titles only; they are never fetched.
fn clean_url(v: &Value) -> Result<Option<String>, CmdError> {
    let Some(u) = p::str(v, "url") else { return Ok(None) };
    let u = u.trim();
    if u.is_empty() || u.len() > 2000 || u.chars().any(|c| c.is_control()) {
        return Err(CmdError::Params("url must be 1–2000 characters of visible text".into()));
    }
    Ok(Some(u.to_string()))
}

fn alt_or(v: &Value, fallback: &str) -> String {
    p::str(v, "alt").unwrap_or(fallback).to_string()
}

/// Ink colour for placeholders: Word grey, falling back to black.
fn placeholder_ink() -> Rgb {
    Rgb::parse("6B7280").unwrap_or(Rgb::BLACK)
}

/// A placeholder picture from the original icon library, `width` points wide.
fn placeholder(s: &mut Session, icon_id: &str, width: Option<f32>, alt: &str) -> CmdResult {
    let icon = wordcraft_render::icon_lib::find(icon_id).ok_or_else(|| CmdError::Failed(format!("no placeholder icon {icon_id:?}")))?;
    let png = wordcraft_render::icon_lib::render(icon, 192, placeholder_ink()).to_png();
    if png.is_empty() {
        return Err(CmdError::Failed("couldn't draw the placeholder".into()));
    }
    let mut r = place_picture(s, png, width, alt)?;
    r["placeholder"] = json!(true);
    Ok(r)
}

fn online_picture(s: &mut Session, v: &Value) -> CmdResult {
    // Bytes first: data, then a gated path — a real picture like insert.picture.
    if let Some(d) = p::str(v, "data") {
        let bytes = base64_decode(d).ok_or_else(|| CmdError::Params("bad base64 data".into()))?;
        let mut r = place_picture(s, bytes, p::f32(v, "width"), &alt_or(v, "Picture"))?;
        r["placeholder"] = json!(false);
        return Ok(r);
    }
    if let Some(path) = p::str(v, "path") {
        let bytes = crate::io::read_file(path).map_err(CmdError::Failed)?;
        let mut r = place_picture(s, bytes, p::f32(v, "width"), &alt_or(v, "Picture"))?;
        r["placeholder"] = json!(false);
        return Ok(r);
    }
    // A URL is never fetched: a labelled placeholder keeps its address.
    if let Some(url) = clean_url(v)? {
        let alt = alt_or(v, &format!("Online picture: {url}"));
        let mut r = placeholder(s, "mountain", p::f32(v, "width"), &alt)?;
        r["url"] = json!(url);
        return Ok(r);
    }
    // Like Pictures with nothing to place: ask the front end for a file.
    s.ui_requests.push(json!({"open": "insertPicture"}));
    super::sel_result(s)
}

fn screenshot(s: &mut Session, v: &Value) -> CmdResult {
    if let Some(d) = p::str(v, "data") {
        let bytes = base64_decode(d).ok_or_else(|| CmdError::Params("bad base64 data".into()))?;
        return place_picture(s, bytes, p::f32(v, "width"), &alt_or(v, "Screenshot"));
    }
    if p::str(v, "source").is_some_and(|w| w != "window" && w != "screen") {
        return Err(CmdError::Params("source must be window or screen".into()));
    }
    Err(CmdError::Failed("screenshot capture isn't available here: take one with your system tool, then paste it or use Insert › Pictures".into()))
}

fn video(s: &mut Session, v: &Value) -> CmdResult {
    let url = clean_url(v)?;
    // A poster frame the caller has is a real picture.
    if let Some(d) = p::str(v, "poster") {
        let bytes = base64_decode(d).ok_or_else(|| CmdError::Params("bad base64 poster".into()))?;
        let fallback = match &url {
            Some(url) => format!("Online video: {url}"),
            None => "Video".to_string(),
        };
        let mut r = place_picture(s, bytes, p::f32(v, "width"), &alt_or(v, &fallback))?;
        r["placeholder"] = json!(false);
        if let Some(url) = url {
            r["url"] = json!(url);
        }
        return Ok(r);
    }
    // A local file's bytes aren't a picture; the placeholder keeps its name.
    if let Some(path) = p::str(v, "path") {
        crate::file_access::check_read(path).map_err(CmdError::Failed)?;
        let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
        let name: String = name.chars().take(200).collect();
        let alt = alt_or(v, &format!("Video: {name}"));
        let mut r = placeholder(s, "monitor", p::f32(v, "width"), &alt)?;
        r["path"] = json!(path);
        return Ok(r);
    }
    if let Some(url) = url {
        let alt = alt_or(v, &format!("Online video: {url}"));
        let mut r = placeholder(s, "monitor", p::f32(v, "width"), &alt)?;
        r["url"] = json!(url);
        return Ok(r);
    }
    Err(CmdError::Failed("online video needs a url, a path or a poster frame: paste a poster or use Insert › Pictures".into()))
}

fn model_3d(s: &mut Session, v: &Value) -> CmdResult {
    if let Some(format) = p::str(v, "format")
        && !["glb", "gltf", "obj", "stl", "fbx", "3mf", "usdz"].contains(&format.to_ascii_lowercase().as_str())
    {
        return Err(CmdError::Params("format must be glb, gltf, obj, stl, fbx, 3mf or usdz".into()));
    }
    // The model isn't rendered; the placeholder keeps the file's name. The path is
    // still gated (automation roots, devices), like any other file parameter.
    if let Some(path) = p::str(v, "path") {
        crate::file_access::check_read(path).map_err(CmdError::Failed)?;
        let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
        let name: String = name.chars().take(200).collect();
        if name.trim().is_empty() {
            return Err(CmdError::Params("path must name a file".into()));
        }
        let alt = alt_or(v, &format!("3D model: {name}"));
        let mut r = placeholder(s, "hexagon", p::f32(v, "width"), &alt)?;
        r["path"] = json!(path);
        return Ok(r);
    }
    Err(CmdError::Failed("3D models need a path: 3D rendering isn't supported, the model is kept as a labelled placeholder".into()))
}

/// Office add-ins need an online store and script execution; WordCraft keeps
/// documents offline, so there is nothing to get. Pure: answers scripts, MCP and
/// the ribbon button without touching the document.
fn get_addins(_: &mut Session, _: &Value) -> CmdResult {
    Ok(json!({"addins": [], "message": "WordCraft keeps your documents offline: Office add-ins aren't supported"}))
}

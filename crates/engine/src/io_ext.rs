//! Format crates registered with the I/O dispatcher.

use wordcraft_doc::Document;

/// Open a format other than plain text / JSON.
pub fn open(ext: &str, bytes: &[u8]) -> Option<Result<Document, String>> {
    match ext {
        "docx" | "docm" | "dotx" | "dotm" | "doc" | "dot" => Some(open_word(ext, bytes)),
        other => wordcraft_formats::import(other, bytes),
    }
}

/// A file with a Word extension. The name doesn't say which format is inside: a `.doc` is often
/// RTF or a renamed `.docx`, and a password-protected `.docx` is a compound file, not a zip. So
/// the content decides.
fn open_word(ext: &str, bytes: &[u8]) -> Result<Document, String> {
    let legacy = matches!(ext, "doc" | "dot");
    if wordcraft_formats::doc::is_cfb(bytes) {
        wordcraft_formats::doc::import(bytes)
    } else if bytes.starts_with(b"PK") || !legacy {
        wordcraft_docx::read(bytes).map_err(|e| e.to_string())
    } else if bytes.trim_ascii_start().starts_with(b"{\\rtf") {
        wordcraft_formats::rtf::import(bytes)
    } else {
        Err("not a Word document".into())
    }
}

/// Save to a format other than plain text / JSON.
pub fn save(ext: &str, doc: &Document) -> Option<Result<Vec<u8>, String>> {
    match ext {
        "png" => Some(render_png(doc, 0, 2.0)),
        "docx" | "docm" | "dotx" => Some(wordcraft_docx::write(doc).map_err(|e| e.to_string())),
        "pdf" => Some(wordcraft_pdf::export(doc, &Default::default()).map_err(|e| e.to_string())),
        other => wordcraft_formats::export(other, doc),
    }
}

/// Render page `page` as PNG at `scale` px/pt.
pub fn render_png(doc: &Document, page: usize, scale: f32) -> Result<Vec<u8>, String> {
    let l = wordcraft_layout::layout(doc, &mut wordcraft_layout::LayoutCache::new(), &Default::default());
    let p = l.pages.get(page).ok_or_else(|| format!("no page {}", page + 1))?;
    let img = wordcraft_render::render_page(doc, p, scale, &Default::default());
    Ok(img.to_png())
}

//! Single-XML OpenDocument Text (ODF 1.3 part 3, §3.1.2 and §10.4.5).
//! Shares the package reader/writer; binary images stay in the document, never on disk.

use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::name::{LocalName, ResolveResult};

use super::*;

/// Write Flat OpenDocument Text, including embedded pictures, metadata and page setup.
pub fn export_flat(doc: &Document) -> Result<Vec<u8>, String> {
    let flow = model::from_doc(doc);
    let mut writer = Writer { flat: true, ..Default::default() };
    let mut body = String::new();
    writer.blocks(&flow.blocks, &mut body, 0);
    let font = doc.styles.default_chr.font.as_deref().unwrap_or(wordcraft_doc::styles::BODY_FONT);
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document {NS} office:mimetype=\"{TEXT_MIME}\">{}{}{}{MASTER_STYLES}<office:body><office:text>{body}</office:text></office:body></office:document>\n",
        metadata(&flow.meta), common_styles(font), writer.automatic_styles(&page_layout(&doc.last_section))
    ).into_bytes())
}

/// Read a flat text document. No linked files, DTDs or network resources are loaded.
pub fn import_flat(bytes: &[u8]) -> Result<Document, String> {
    let xml = normalize(bytes)?;
    let mut styles = Styles::default();
    styles.read(&xml, true);
    let flow = parse_body(&xml, &styles, read_meta(&xml), &mut |_| None);
    let mut doc = model::to_doc(&flow);
    apply_page_layout(&xml, &mut doc.last_section);
    Ok(doc)
}

// The package parser uses conventional ODF prefixes. Resolve aliases before passing flat XML
// to it: prefixes (including the default namespace) are chosen by the producing application.
fn name(ns: ResolveResult<'_>, local: LocalName<'_>) -> Result<String, String> {
    let prefix = match ns {
        ResolveResult::Bound(ns) => match ns.as_ref() {
            b"urn:oasis:names:tc:opendocument:xmlns:office:1.0" => "office",
            b"urn:oasis:names:tc:opendocument:xmlns:style:1.0" => "style",
            b"urn:oasis:names:tc:opendocument:xmlns:text:1.0" => "text",
            b"urn:oasis:names:tc:opendocument:xmlns:table:1.0" => "table",
            b"urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" => "draw",
            b"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" => "fo",
            b"urn:oasis:names:tc:opendocument:xmlns:meta:1.0" => "meta",
            b"urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" => "svg",
            b"http://www.w3.org/1999/xlink" => "xlink",
            b"http://purl.org/dc/elements/1.1/" => "dc",
            _ => return Ok("foreign".into()),
        },
        ResolveResult::Unbound => return Ok("foreign".into()),
        ResolveResult::Unknown(_) => return Err("FODT: undeclared XML namespace".into()),
    };
    Ok(format!("{prefix}:{}", String::from_utf8_lossy(local.as_ref())))
}

fn normalize(bytes: &[u8]) -> Result<Vec<u8>, String> {
    // Unlike a ZIP part, the whole flat file (including base64) must fit this bound.
    if bytes.len() as u64 > MAX_ENTRY {
        return Err("FODT: document exceeds the XML size limit".into());
    }
    let mut reader = quick_xml::NsReader::from_reader(bytes);
    let mut writer = quick_xml::Writer::new(Vec::new());
    let mut depth = 0usize;
    let mut root_seen = false;
    let mut body_seen = false;
    loop {
        let event = reader.read_event().map_err(|e| format!("FODT: invalid XML ({e})"))?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(event, Event::Empty(_));
                if depth >= MAX_XML_DEPTH {
                    return Err("FODT: XML nesting limit exceeded".into());
                }
                let (ns, local) = reader.resolver().resolve_element(e.name());
                let tag = name(ns, local)?;
                let mut normalized = BytesStart::new(&tag);
                let mut mime = None;
                for (index, attr) in e.attributes().enumerate() {
                    if index >= 128 {
                        return Err("FODT: too many XML attributes".into());
                    }
                    let attr = attr.map_err(|e| format!("FODT: invalid attribute ({e})"))?;
                    if attr.key.as_ref() == b"xmlns" || attr.key.as_ref().starts_with(b"xmlns:") {
                        continue;
                    }
                    let (ns, local) = reader.resolver().resolve_attribute(attr.key);
                    let key = name(ns, local)?;
                    let value =
                        attr.normalized_value(quick_xml::XmlVersion::Explicit1_0).map_err(|e| format!("FODT: invalid attribute value ({e})"))?;
                    if key == "office:mimetype" {
                        mime = Some(value.to_string());
                    }
                    if key != "foreign" {
                        normalized.push_attribute((key.as_str(), value.as_ref()));
                    }
                }
                if depth == 0 {
                    if root_seen || tag != "office:document" || !matches!(mime.as_deref(), Some(TEXT_MIME | TEMPLATE_MIME)) {
                        return Err("FODT: expected an OpenDocument text document".into());
                    }
                    root_seen = true;
                }
                if tag == "office:text" && depth == 2 {
                    body_seen = true;
                }
                let next = if empty {
                    Event::Empty(normalized)
                } else {
                    depth += 1;
                    Event::Start(normalized)
                };
                writer.write_event(next).map_err(|e| format!("FODT: {e}"))?;
            }
            Event::End(e) => {
                depth = depth.saturating_sub(1);
                let (ns, local) = reader.resolver().resolve_element(e.name());
                let tag = name(ns, local)?;
                writer.write_event(Event::End(BytesEnd::new(tag))).map_err(|e| format!("FODT: {e}"))?;
            }
            Event::DocType(_) => return Err("FODT: document type declarations are not supported".into()),
            Event::Eof => break,
            Event::Text(ref text) if depth == 0 && !text.as_ref().iter().all(u8::is_ascii_whitespace) => {
                return Err("FODT: text outside the document element".into());
            }
            _ => writer.write_event(event).map_err(|e| format!("FODT: {e}"))?,
        }
        if writer.get_ref().len() as u64 > MAX_ENTRY {
            return Err("FODT: normalized XML exceeds the size limit".into());
        }
    }
    if !root_seen || !body_seen || depth != 0 {
        return Err("FODT: missing or incomplete document body".into());
    }
    Ok(writer.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_style_inheritance_keeps_zero_spacing_and_bounds_nesting() {
        let xml = format!(
            r#"<office:document {NS} office:mimetype="{TEXT_MIME}">
            <office:styles><style:style style:name="Base" style:family="paragraph">
            <style:paragraph-properties fo:margin-top="12pt" fo:margin-left="36pt"/>
            </style:style></office:styles><office:automatic-styles>
            <style:style style:name="Child" style:family="paragraph" style:parent-style-name="Base">
            <style:paragraph-properties fo:margin-top="0pt"/></style:style></office:automatic-styles>
            <office:body><office:text><text:p text:style-name="Child">Inherited</text:p></office:text></office:body>
            </office:document>"#
        );
        let doc = import_flat(xml.as_bytes()).unwrap();
        let p = doc.body[0].as_para().unwrap();
        assert_eq!(p.props.space_before, Some(0.0));
        assert_eq!(p.props.indent_left, Some(36.0));
        let nested = xml.replace("Inherited", &format!("{}deep{}", "<text:span>".repeat(MAX_XML_DEPTH), "</text:span>".repeat(MAX_XML_DEPTH)));
        assert!(import_flat(nested.as_bytes()).unwrap_err().contains("nesting"));
    }

    #[test]
    fn inline_image_budget_is_checked_before_collecting_base64() {
        let styles = Styles::default();
        let mut fetch = |_: &str| None;
        let mut body = Body {
            styles: &styles,
            zip: &mut fetch,
            containers: vec![Vec::new()],
            fmt: Vec::new(),
            para: None,
            para_cont: false,
            lists: Vec::new(),
            item_used: Vec::new(),
            lists_saved: Vec::new(),
            tables: Vec::new(),
            cells: Vec::new(),
            in_text: true,
            skip: 0,
            depth: 0,
            frame: None,
            in_desc: false,
            binary: None,
            binary_overflow: false,
            binary_remaining: 3,
        };
        body.start("draw:frame", &vec![]);
        body.start("office:binary-data", &vec![]);
        body.text("AAAA AAAA AAAA");
        assert_eq!(body.binary.as_deref(), Some("AAAA"));
        assert!(body.binary_overflow);
        body.end("office:binary-data");
        assert!(body.frame.as_ref().unwrap().2.is_none());
    }
}

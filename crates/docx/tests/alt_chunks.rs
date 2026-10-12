//! Alternative format chunks (`w:altChunk`, ECMA-376 §17.17.2.1): content in another format that
//! is brought in where the chunk stands. All markup here is synthetic.

use std::io::Write;

use wordcraft_doc::{Block, Document, InlineObject, StoryRef};
use wordcraft_docx::{AltChunk, ReadOptions, read, read_with};

const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
const ROOT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
const DOCX_MAIN: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml";

fn p(text: &str) -> String {
    format!(r#"<w:p><w:r><w:t xml:space="preserve">{text}</w:t></w:r></w:p>"#)
}

/// A package whose body is `body`, with the chunk `afchunk.{ext}` (content type `ct`) as `rIdC`.
fn package(body: &str, chunk: Option<(&str, &str, &[u8])>) -> Vec<u8> {
    let doc = format!(r#"<w:document {NS}><w:body>{body}<w:sectPr/></w:body></w:document>"#);
    let mut files: Vec<(String, Vec<u8>)> = vec![("_rels/.rels".into(), ROOT_RELS.into()), ("word/document.xml".into(), doc.into_bytes())];
    let mut types = String::from(
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>"#,
    );
    if let Some((ext, ct, bytes)) = chunk {
        types.push_str(&format!(r#"<Default Extension="{ext}" ContentType="{ct}"/>"#));
        files.push((
            "word/_rels/document.xml.rels".into(),
            format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdC" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/aFChunk" Target="/word/afchunk.{ext}"/></Relationships>"#).into_bytes(),
        ));
        files.push((format!("word/afchunk.{ext}"), bytes.to_vec()));
    }
    types.push_str(r#"<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#);
    files.push(("[Content_Types].xml".into(), types.into_bytes()));
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in files {
        zw.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(&bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

const CHUNK: &str = r#"<w:altChunk r:id="rIdC"/>"#;

fn body_lines(d: &Document) -> Vec<String> {
    d.plain_text(StoryRef::Body).split('\n').map(str::to_string).collect()
}

#[test]
fn a_chunk_comes_in_through_the_importer_where_it_stands() {
    let html = b"<p>Chunk one</p><p>Chunk two</p>";
    let bytes = package(&format!("{}{CHUNK}{}", p("Before"), p("After")), Some(("htm", "application/xhtml+xml", html)));
    let seen = std::cell::RefCell::new(Vec::new());
    let import = |c: &AltChunk| {
        seen.borrow_mut().push((c.content_type.to_string(), c.part.to_string(), c.bytes.to_vec()));
        Some(Document::from_text("Chunk one\nChunk two"))
    };
    let d = read_with(&bytes, &ReadOptions { chunks: Some(&import), ..Default::default() }).unwrap();
    assert_eq!(body_lines(&d), ["Before", "Chunk one", "Chunk two", "After"]);
    assert_eq!(*seen.borrow(), [("application/xhtml+xml".to_string(), "word/afchunk.htm".to_string(), html.to_vec())]);
    // Without an importer the chunk is left out.
    assert_eq!(body_lines(&read(&bytes).unwrap()), ["Before", "After"]);
}

#[test]
fn a_wordprocessing_chunk_is_read_without_an_importer_and_flows_in_its_section() {
    let inner = package(&format!("{}{}", p("Inner one"), p("Inner two")), None);
    let bytes = package(&format!("{}{CHUNK}{}", p("Before"), p("After")), Some(("docx", DOCX_MAIN, &inner)));
    let d = read(&bytes).unwrap();
    assert_eq!(body_lines(&d), ["Before", "Inner one", "Inner two", "After"]);
    // The inner document's own section doesn't come along.
    assert_eq!(d.sections().len(), 1);
}

#[test]
fn nested_wordprocessing_chunks_stop_at_a_depth() {
    let mut bytes = package(&p("Deepest"), None);
    for level in 0..8 {
        bytes = package(&format!("{}{CHUNK}", p(&format!("Level {level}"))), Some(("docx", DOCX_MAIN, &bytes)));
    }
    let lines = body_lines(&read(&bytes).unwrap());
    assert_eq!(lines.first().map(String::as_str), Some("Level 7"));
    assert!(!lines.iter().any(|l| l == "Deepest"), "{lines:?}");
    assert!(lines.len() >= 4, "{lines:?}");
}

#[test]
fn a_chunks_pictures_come_with_it() {
    let bytes = package(&format!("{}{CHUNK}", p("Before")), Some(("htm", "text/html", b"<p>x</p>")));
    let import = |_: &AltChunk| {
        let mut d = Document::new();
        let key = d.add_media(vec![1, 2, 3], "png");
        let mut para = wordcraft_doc::Paragraph::new();
        let image = InlineObject::Image { media: key, w: 10.0, h: 10.0, alt: String::new(), float: Default::default(), crop: [0.0; 4], ole: None };
        para.insert_object(0, image, &Default::default()).unwrap();
        d.body = vec![std::sync::Arc::new(Block::Para(para))];
        Some(d)
    };
    let d = read_with(&bytes, &ReadOptions { chunks: Some(&import), ..Default::default() }).unwrap();
    let keys: Vec<String> = d
        .body
        .iter()
        .filter_map(|b| match &**b {
            Block::Para(p) => p.objects.iter().find_map(|o| match o {
                InlineObject::Image { media, .. } => Some(media.clone()),
                _ => None,
            }),
            _ => None,
        })
        .collect();
    assert_eq!(keys.len(), 1);
    assert_eq!(d.media.get(&keys[0]).map(|m| m.as_slice()), Some(&[1u8, 2, 3][..]));
}

#[test]
fn a_file_brings_in_a_bounded_number_of_chunks() {
    let bytes = package(&CHUNK.repeat(300), Some(("htm", "text/html", b"<p>x</p>")));
    let calls = std::cell::Cell::new(0);
    let import = |_: &AltChunk| {
        calls.set(calls.get() + 1);
        Some(Document::from_text("x"))
    };
    let d = read_with(&bytes, &ReadOptions { chunks: Some(&import), ..Default::default() }).unwrap();
    assert_eq!(calls.get(), 256);
    assert_eq!(body_lines(&d).len(), 256);
}

#[test]
fn hostile_chunks_never_panic() {
    for (ext, ct, bytes) in [("docx", DOCX_MAIN, &b"PK\x03\x04 not a zip"[..]), ("htm", "text/html", b""), ("bin", "", b"\x00\xff")] {
        let bytes = package(&format!("{CHUNK}{CHUNK}{}", p("x")), Some((ext, ct, bytes)));
        let import = |_: &AltChunk| None;
        let d = read_with(&bytes, &ReadOptions { chunks: Some(&import), ..Default::default() }).unwrap();
        assert_eq!(body_lines(&d), ["x"]);
    }
    // A chunk naming a missing part or another relationship type is left out.
    let bytes = package(&format!(r#"<w:altChunk r:id="nope"/><w:altChunk/>{}"#, p("y")), None);
    assert_eq!(body_lines(&read(&bytes).unwrap()), ["y"]);
}

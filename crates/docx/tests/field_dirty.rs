//! #877: `w:dirty` on a field (Word rebuilds it on opening) survives open and save, and is never
//! written for a field that didn't have it. Original fixtures from the public OOXML contract
//! (ECMA-376 `CT_FldChar`, `CT_SimpleField`).
use std::io::{Read, Write};

use wordcraft_doc::{Document, InlineObject};

fn package(body: &str) -> Vec<u8> {
    let xml = format!(r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#);
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, value) in [
        ("word/document.xml", xml.as_str()),
        (
            "_rels/.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="main" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
        ),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(value.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// The `document.xml` of a saved file.
fn document_xml(bytes: &[u8]) -> String {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut out = String::new();
    zip.by_name("word/document.xml").unwrap().read_to_string(&mut out).unwrap();
    out
}

/// A complex field with a one-run result; `attrs` go on the begin marker.
fn complex(attrs: &str, code: &str, result: &str) -> String {
    format!(
        r#"<w:r><w:fldChar w:fldCharType="begin"{attrs}/></w:r><w:r><w:instrText xml:space="preserve"> {code} </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>{result}</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>"#
    )
}

/// Open, save, open and save again: what the first save wrote and what the second wrote.
fn two_saves(body: &str) -> (Document, String, String) {
    let doc = wordcraft_docx::read(&package(body)).unwrap();
    let first = wordcraft_docx::write(&doc).unwrap();
    let again = wordcraft_docx::read(&first).unwrap();
    let second = wordcraft_docx::write(&again).unwrap();
    (doc, document_xml(&first), document_xml(&second))
}

fn dirty_count(xml: &str) -> usize {
    xml.matches(r#"w:dirty="true""#).count()
}

fn fields(doc: &Document) -> Vec<&InlineObject> {
    doc.body
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| p.objects.iter())
        .filter(|o| matches!(o, InlineObject::Field { .. } | InlineObject::FieldStart { .. }))
        .collect()
}

#[test]
fn a_dirty_toc_placeholder_stays_dirty() {
    let body = format!("<w:p>{}</w:p>", complex(r#" w:dirty="true""#, r#"TOC \o "1-3" \h"#, "Update this field"));
    let (doc, first, second) = two_saves(&body);
    // A TOC is kept as a range field (a start marker), whatever its result looks like.
    assert!(
        matches!(fields(&doc)[..], [InlineObject::FieldStart { dirty: true, .. }] | [InlineObject::Field { dirty: true, .. }]),
        "{:?}",
        fields(&doc)
    );
    for xml in [&first, &second] {
        assert_eq!(dirty_count(xml), 1, "{xml}");
        assert!(xml.contains(r#"<w:fldChar w:fldCharType="begin" w:dirty="true"/>"#), "{xml}");
    }
}

#[test]
fn dirty_goes_with_each_field_that_had_it() {
    let body = format!(
        "<w:p>{}{}{}</w:p>",
        complex(r#" w:dirty="1""#, "PAGE", "1"),
        complex("", "NUMPAGES", "9"),
        complex(r#" w:fldLock="1" w:dirty="true""#, "DATE", "today"),
    );
    let (doc, first, second) = two_saves(&body);
    let flags: Vec<(bool, bool)> = fields(&doc)
        .into_iter()
        .filter_map(|o| if let InlineObject::Field { locked, dirty, .. } = o { Some((*locked, *dirty)) } else { None })
        .collect();
    assert_eq!(flags, [(false, true), (false, false), (true, true)]);
    for xml in [&first, &second] {
        assert_eq!(dirty_count(xml), 2, "{xml}");
        assert_eq!(xml.matches(r#"w:fldLock="1""#).count(), 1, "{xml}");
    }
}

#[test]
fn off_values_and_a_clean_field_write_no_dirty_flag() {
    let body = format!("<w:p>{}{}</w:p>", complex(r#" w:dirty="false""#, "PAGE", "1"), complex(r#" w:dirty="0""#, "NUMPAGES", "9"));
    let (doc, first, second) = two_saves(&body);
    assert!(fields(&doc).iter().all(|o| matches!(o, InlineObject::Field { dirty: false, .. })));
    for xml in [&first, &second] {
        assert!(!xml.contains("w:dirty"), "{xml}");
    }
}

#[test]
fn a_dirty_simple_field_stays_dirty() {
    let body = r#"<w:p><w:fldSimple w:instr=" PAGE " w:dirty="true"><w:r><w:t>1</w:t></w:r></w:fldSimple></w:p>"#;
    let (doc, first, second) = two_saves(body);
    assert!(matches!(fields(&doc)[..], [InlineObject::Field { dirty: true, .. }]));
    for xml in [&first, &second] {
        assert_eq!(dirty_count(xml), 1, "{xml}");
    }
}

#[test]
fn a_dirty_field_whose_result_spans_paragraphs_stays_dirty() {
    let body = r#"<w:p><w:r><w:fldChar w:fldCharType="begin" w:dirty="true"/></w:r><w:r><w:instrText xml:space="preserve"> TOC \o "1-2" </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>First entry</w:t></w:r></w:p><w:p><w:r><w:t>Second entry</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#;
    let (doc, first, second) = two_saves(body);
    assert!(matches!(fields(&doc)[..], [InlineObject::FieldStart { dirty: true, .. }]), "{:?}", fields(&doc));
    for xml in [&first, &second] {
        assert_eq!(dirty_count(xml), 1, "{xml}");
    }
}

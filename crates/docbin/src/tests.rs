//! Synthetic `.doc` fixtures, written for these tests from the [MS-DOC] specification.
//! Files produced by Word are never committed; every fixture here is built in code.

use std::io::Write;

use proptest::prelude::*;

use super::read;

mod drawing;

/// Offset in the WordDocument stream where test text pieces are placed (past the FIB).
const TEXT_FC: u32 = 0x600;
/// Offset in the Table stream where the test Clx is placed.
const CLX_AT: u32 = 0x40;

/// Build an in-memory compound file holding the given root streams.
pub(crate) fn cfb_file(streams: &[(&str, &[u8])]) -> Vec<u8> {
    let mut comp = cfb::CompoundFile::create(std::io::Cursor::new(Vec::new())).expect("cfb create");
    for (name, data) in streams {
        let mut s = comp.create_stream(format!("/{name}")).expect("cfb stream");
        s.write_all(data).expect("cfb write");
    }
    comp.into_inner().into_inner()
}

/// Knobs of the minimal FIB these tests build (Word 97 shape, `cbRgFcLcb` = 0x5D).
pub(crate) struct FibSpec {
    pub(crate) nfib: u16,
    pub(crate) flags: u16,
    pub(crate) ccp_text: u32,
    pub(crate) ccp_ftn: u32,
    pub(crate) ccp_hdd: u32,
    pub(crate) ccp_edn: u32,
    /// (pair index, fc, lcb) entries to poke into the RgFcLcb blob.
    pub(crate) pairs: Vec<(usize, u32, u32)>,
}

impl Default for FibSpec {
    fn default() -> Self {
        FibSpec { nfib: 0x00C1, flags: 0x1000, ccp_text: 0, ccp_ftn: 0, ccp_hdd: 0, ccp_edn: 0, pairs: Vec::new() }
    }
}

/// A minimal Word 97 FIB: FibBase + csw/rgw + cslw/rglw + 0x5D fc/lcb pairs + cswNew = 0.
pub(crate) fn fib_bytes(spec: &FibSpec) -> Vec<u8> {
    let mut v = vec![0u8; 0x9A + 0x5D * 8 + 2];
    let put16 = |v: &mut Vec<u8>, at: usize, x: u16| {
        v[at..at + 2].copy_from_slice(&x.to_le_bytes());
    };
    let put32 = |v: &mut Vec<u8>, at: usize, x: u32| {
        v[at..at + 4].copy_from_slice(&x.to_le_bytes());
    };
    put16(&mut v, 0x00, 0xA5EC);
    put16(&mut v, 0x02, spec.nfib);
    put16(&mut v, 0x0A, spec.flags);
    put16(&mut v, 0x0C, 0x00BF);
    put16(&mut v, 0x20, 0x000E);
    put16(&mut v, 0x3E, 0x0016);
    put32(&mut v, 0x40, 0x1000); // cbMac
    put32(&mut v, 0x4C, spec.ccp_text); // rglw[3] = ccpText
    put32(&mut v, 0x50, spec.ccp_ftn); // rglw[4] = ccpFtn
    put32(&mut v, 0x54, spec.ccp_hdd); // rglw[5] = ccpHdd
    put32(&mut v, 0x60, spec.ccp_edn); // rglw[8] = ccpEdn
    put16(&mut v, 0x98, 0x005D);
    for (i, fc, lcb) in &spec.pairs {
        put32(&mut v, 0x9A + i * 8, *fc);
        put32(&mut v, 0x9A + i * 8 + 4, *lcb);
    }
    v
}

/// One test piece: text stored either uncompressed (UTF-16LE) or compressed (Windows-1252).
#[derive(Clone)]
pub(crate) struct Piece {
    pub(crate) compressed: bool,
    pub(crate) text: String,
}

/// Build a PlcPcd (aCp[n+1] + aPcd[n]) for pieces stored at consecutive offsets from
/// `first_fc` (compressed pieces start at `fc = 2 × byte offset` per FcCompressed).
fn plcpcd(pieces: &[Piece], first_fc: u32) -> Vec<u8> {
    let mut v = Vec::new();
    let mut cp = 0u32;
    v.extend_from_slice(&cp.to_le_bytes());
    for p in pieces {
        cp += p.text.chars().count() as u32;
        v.extend_from_slice(&cp.to_le_bytes());
    }
    let mut off = 0u32;
    for p in pieces {
        // Bytes this piece occupies in the text region: one per character compressed, two
        // uncompressed (count chars, not UTF-8 bytes).
        let len = if p.compressed { p.text.chars().count() } else { p.text.chars().count() * 2 } as u32;
        let fc = if p.compressed { 2 * (first_fc + off) } else { first_fc + off };
        let fc_raw = if p.compressed { fc | 0x4000_0000 } else { fc };
        v.extend_from_slice(&[0, 0]); // fNoParaLast & friends
        v.extend_from_slice(&fc_raw.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes()); // prm
        off += len;
    }
    v
}

fn piece_bytes(p: &Piece) -> Vec<u8> {
    if p.compressed {
        // Tests only use cp1252-encodable chars in compressed pieces.
        p.text.chars().map(|c| c as u8).collect()
    } else {
        let units: Vec<u16> = p.text.chars().map(|c| c as u16).collect();
        units.iter().flat_map(|u| u.to_le_bytes()).collect()
    }
}

/// A document whose main text is the given pieces (extra subdocument text may follow).
fn text_doc(main: &[Piece], extra: &[Piece], ccp_hdd: u32) -> Vec<u8> {
    let all: Vec<Piece> = main.iter().chain(extra.iter()).cloned().collect();
    let ccp_text: u32 = main.iter().map(|p| p.text.chars().count() as u32).sum();
    let mut word = vec![0u8; TEXT_FC as usize];
    for p in &all {
        word.extend_from_slice(&piece_bytes(p));
    }
    let plc = plcpcd(&all, TEXT_FC);
    let mut clx = vec![0x02u8];
    clx.extend_from_slice(&(plc.len() as u32).to_le_bytes());
    clx.extend_from_slice(&plc);
    let mut table = vec![0u8; CLX_AT as usize];
    table.extend_from_slice(&clx);
    let spec = FibSpec { ccp_text, ccp_hdd, pairs: vec![(33, CLX_AT, clx.len() as u32)], ..Default::default() };
    word[..fib_bytes(&spec).len()].copy_from_slice(&fib_bytes(&spec));
    cfb_file(&[("WordDocument", &word), ("0Table", &table)])
}

fn body_texts(doc: &wordcraft_doc::Document) -> Vec<String> {
    doc.body.iter().filter_map(|b| b.as_para().map(|p| p.text.clone())).collect()
}

#[test]
fn not_a_compound_file() {
    assert!(matches!(read(&[0u8; 64]), Err(super::DocbinError::Container(_))));
}

#[test]
fn cfb_without_word_stream() {
    let f = cfb_file(&[("0Table", &[0u8; 16])]);
    assert!(matches!(read(&f), Err(super::DocbinError::NotWord(_))));
}

#[test]
fn word_stream_too_short() {
    let f = cfb_file(&[("WordDocument", &[0u8; 16])]);
    assert!(matches!(read(&f), Err(super::DocbinError::NotWord(_))));
}

#[test]
fn wrong_magic_rejected() {
    let mut fib = fib_bytes(&FibSpec::default());
    fib[0] = 0x34;
    fib[1] = 0x12;
    let f = cfb_file(&[("WordDocument", &fib)]);
    assert!(matches!(read(&f), Err(super::DocbinError::NotWord(m)) if m.contains("magic")));
}

#[test]
fn word_6_95_rejected() {
    let mut fib = fib_bytes(&FibSpec::default());
    fib[0] = 0xDC;
    fib[1] = 0xA5;
    let f = cfb_file(&[("WordDocument", &fib)]);
    assert!(matches!(read(&f), Err(super::DocbinError::NotWord(m)) if m.contains("6.0/95")));
}

#[test]
fn encrypted_rejected() {
    let spec = FibSpec { flags: 0x1100, ..Default::default() };
    let f = cfb_file(&[("WordDocument", &fib_bytes(&spec))]);
    assert!(matches!(read(&f), Err(super::DocbinError::Encrypted)));
}

#[test]
fn wrong_csw_rejected() {
    let mut fib = fib_bytes(&FibSpec::default());
    fib[0x20] = 0x0D;
    fib[0x21] = 0x00;
    let f = cfb_file(&[("WordDocument", &fib)]);
    assert!(matches!(read(&f), Err(super::DocbinError::NotWord(m)) if m.contains("csw")));
}

#[test]
fn missing_clx_rejected() {
    let spec = FibSpec { ccp_text: 3, ..Default::default() };
    let f = cfb_file(&[("WordDocument", &fib_bytes(&spec)), ("0Table", &[0u8; 16])]);
    assert!(matches!(read(&f), Err(super::DocbinError::Malformed(m)) if m.contains("piece table")));
}

#[test]
fn unicode_text_extracted() {
    let f = text_doc(&[Piece { compressed: false, text: "Héllo Wörld\rSecond€\r".into() }], &[], 0);
    let doc = read(&f).expect("opens");
    assert_eq!(body_texts(&doc), ["Héllo Wörld", "Second€"]);
}

#[test]
fn compressed_cp1252_text_extracted() {
    let f = text_doc(&[Piece { compressed: true, text: "caf\u{E9} 100\u{BD}\r".into() }], &[], 0);
    let doc = read(&f).expect("opens");
    assert_eq!(body_texts(&doc), ["café 100½"]);
}

#[test]
fn multiple_pieces_and_encodings() {
    let f = text_doc(
        &[
            Piece { compressed: false, text: "plain ".into() },
            Piece { compressed: true, text: "mixed\u{E9} ".into() },
            Piece { compressed: false, text: "end\r".into() },
        ],
        &[],
        0,
    );
    let doc = read(&f).expect("opens");
    assert_eq!(body_texts(&doc), ["plain mixedé end"]);
}

#[test]
fn special_characters_mapped() {
    let raw = "a\tb\u{B}c\u{C}d\u{E}e\u{1E}f\u{1F}g\r\u{13}field\u{14}result\u{15}\r";
    let f = text_doc(&[Piece { compressed: false, text: raw.into() }], &[], 0);
    let doc = read(&f).expect("opens");
    // The field collapses to one anchor object; its instruction is hidden and its cached
    // result kept on the object.
    assert_eq!(body_texts(&doc), ["a\tb\nc\u{C}d\u{E}e\u{2011}f\u{AD}g", "\u{FFFC}"]);
    let p1 = doc.body.get(1).and_then(|b| b.as_para()).expect("field para");
    assert!(
        matches!(p1.objects.first(), Some(wordcraft_doc::para::InlineObject::Field { instr, result, .. }) if instr == "field" && result == "result"),
        "objects: {:?}",
        p1.objects
    );
}

#[test]
fn empty_paragraphs_kept() {
    let f = text_doc(&[Piece { compressed: false, text: "a\r\rb\r".into() }], &[], 0);
    let doc = read(&f).expect("opens");
    assert_eq!(body_texts(&doc), ["a", "", "b"]);
}

#[test]
fn empty_document_gets_a_paragraph() {
    let f = text_doc(&[Piece { compressed: false, text: "".into() }], &[], 0);
    let doc = read(&f).expect("opens");
    assert_eq!(doc.body.len(), 1);
}

#[test]
fn header_text_stays_out_of_body() {
    let f = text_doc(
        &[Piece { compressed: false, text: "body\r".into() }],
        &[Piece { compressed: false, text: "header\u{D}".into() }],
        8, // ccpHdd covers the extra piece
    );
    let doc = read(&f).expect("opens");
    assert_eq!(body_texts(&doc), ["body"]);
}

#[test]
fn clx_outside_table_rejected() {
    let spec = FibSpec { pairs: vec![(33, 0x1000, 0x40)], ..Default::default() };
    let f = cfb_file(&[("WordDocument", &fib_bytes(&spec)), ("0Table", &[0u8; 16])]);
    assert!(matches!(read(&f), Err(super::DocbinError::Malformed(_))));
}

#[test]
fn garbage_plcpcd_rejected() {
    let mut table = vec![0u8; CLX_AT as usize];
    table.extend_from_slice(&[0x02, 0x05, 0x00, 0x00, 0x00, 1, 2, 3, 4]); // lcb 5: not ≡ 1 mod 12
    let spec = FibSpec { pairs: vec![(33, CLX_AT, 9)], ..Default::default() };
    let f = cfb_file(&[("WordDocument", &fib_bytes(&spec)), ("0Table", &table)]);
    assert!(matches!(read(&f), Err(super::DocbinError::Malformed(m)) if m.contains("PlcPcd")));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn junk_bytes_never_panic(b in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let _ = read(&b);
    }

    #[test]
    fn truncated_file_never_panics(n in 0..4096usize) {
        let f = text_doc(
            &[Piece { compressed: false, text: "Hello\rworld\r\u{E9}x\r".into() }],
            &[],
            0,
        );
        let n = n.min(f.len());
        let _ = read(&f[..n]);
    }

    #[test]
    fn flipped_bytes_never_panic(pos in 0..4096usize, bit in 0..8usize) {
        let mut f = text_doc(
            &[Piece { compressed: false, text: "Hello\rworld\r\u{E9}x\r".into() }],
            &[],
            0,
        );
        let at = pos.min(f.len() - 1);
        if let Some(b) = f.get_mut(at) {
            *b ^= 1 << (bit & 7);
        }
        let _ = read(&f);
    }

    #[test]
    fn structured_truncated_never_panics(n in 0..8192usize) {
        let f = structured_doc();
        let n = n.min(f.len());
        let _ = read(&f[..n]);
    }

    #[test]
    fn structured_flipped_never_panics(pos in 0..8192usize, bit in 0..8usize) {
        let mut f = structured_doc();
        let at = pos.min(f.len() - 1);
        if let Some(b) = f.get_mut(at) {
            *b ^= 1 << (bit & 7);
        }
        let _ = read(&f);
    }

    #[test]
    fn extras_truncated_never_panics(n in 0..8192usize) {
        let f = extras_doc();
        let n = n.min(f.len());
        let _ = read(&f[..n]);
    }

    #[test]
    fn extras_flipped_never_panics(pos in 0..8192usize, bit in 0..8usize) {
        let mut f = extras_doc();
        let at = pos.min(f.len() - 1);
        if let Some(b) = f.get_mut(at) {
            *b ^= 1 << (bit & 7);
        }
        let _ = read(&f);
    }
}

// ---------------------------------------------------------------------------------------------
// D2: formatting — fixtures with a stylesheet, font table and FKP pages.

/// Twips→FC units inside a piece: 2 per character.
fn fc_of(cp: u32) -> u32 {
    TEXT_FC + 2 * cp
}

/// A grpprl of Prls given as (opcode, operand) pairs.
fn grpprl(prls: &[(u16, &[u8])]) -> Vec<u8> {
    let mut v = Vec::new();
    for (op, operand) in prls {
        v.extend_from_slice(&op.to_le_bytes());
        v.extend_from_slice(operand);
    }
    v
}

/// One STD record (Word 97 shape, cbSTDBaseInFile = 10) with no next-paragraph style.
/// `istd_base` is zero-based; 0x0FFF means no parent.
fn std_bytes(stk: u8, istd_base: u16, name: &str, papx: &[u8], chpx: &[u8]) -> Vec<u8> {
    std_bytes_next(stk, istd_base, 0x0FFF, name, papx, chpx)
}

/// One STD record with an explicit next-paragraph style ([MS-DOC] §2.9.260 StdfBase).
fn std_bytes_next(stk: u8, istd_base: u16, istd_next: u16, name: &str, papx: &[u8], chpx: &[u8]) -> Vec<u8> {
    let cupx = if stk == 1 { 2 } else { 1 };
    let mut base = vec![0u8; 10];
    // sti 0xFFF (user-defined) | flags
    base[0] = 0xFF;
    base[1] = 0x0F;
    // stk(4) | istdBase(12)
    let w1 = (stk as u16) | (istd_base << 4);
    base[2..4].copy_from_slice(&w1.to_le_bytes());
    // cupx(4) | istdNext(12)
    let w2 = (cupx as u16) | (istd_next << 4);
    base[4..6].copy_from_slice(&w2.to_le_bytes());
    // bchUpe filled below; grfstd = 0
    let mut v = base;
    // xstzName: cch + UTF-16 + terminator
    let units: Vec<u16> = name.chars().map(|c| c as u16).collect();
    v.extend_from_slice(&(units.len() as u16).to_le_bytes());
    for u in &units {
        v.extend_from_slice(&u.to_le_bytes());
    }
    v.extend_from_slice(&0u16.to_le_bytes());
    if stk == 1 {
        // LPUpxPapx: cbUpx + UpxPapx (istd u16 + grpprl), padded to even.
        let papx_upx = [0u16.to_le_bytes().as_slice(), papx].concat();
        v.extend_from_slice(&(papx_upx.len() as u16).to_le_bytes());
        v.extend_from_slice(&papx_upx);
        if papx_upx.len() % 2 == 1 {
            v.push(0);
        }
    }
    // LPUpxChpx: cbUpx + raw grpprl.
    v.extend_from_slice(&(chpx.len() as u16).to_le_bytes());
    v.extend_from_slice(chpx);
    if chpx.len() % 2 == 1 {
        v.push(0);
    }
    let cb = v.len() as u16;
    let mut out = cb.to_le_bytes().to_vec();
    out.extend_from_slice(&v);
    // bchUpe (offset 6 in the Stdf) equals cbStd.
    out[6 + 2] = cb as u8;
    out[7 + 2] = (cb >> 8) as u8;
    out
}

/// An STSH with the given styles.
fn stsh_bytes(styles: &[Vec<u8>]) -> Vec<u8> {
    let stshi = [(styles.len() as u16).to_le_bytes(), 10u16.to_le_bytes()].concat(); // cstd, cbSTDBaseInFile
    let mut v = (stshi.len() as u16).to_le_bytes().to_vec();
    v.extend_from_slice(&stshi);
    for s in styles {
        v.extend_from_slice(s);
    }
    v
}

/// A non-extended SttbfFfn of font names.
fn ffn_bytes(names: &[&str]) -> Vec<u8> {
    let fonts: Vec<(&str, Option<&str>)> = names.iter().map(|n| (*n, None)).collect();
    ffn_bytes_alt(&fonts)
}

/// A non-extended SttbfFfn ([MS-DOC] §2.9.286): cData, cbExtra = 0, then per font a one-byte
/// cchData and an FFN (39-byte header, NUL-terminated UTF-16 name, optional alternate name
/// whose offset in characters is `ixchSzAlt`).
fn ffn_bytes_alt(fonts: &[(&str, Option<&str>)]) -> Vec<u8> {
    let utf16z = |s: &str| -> Vec<u8> { s.chars().chain(['\0']).flat_map(|c| (c as u16).to_le_bytes()).collect() };
    let mut v = (fonts.len() as u16).to_le_bytes().to_vec();
    v.extend_from_slice(&0u16.to_le_bytes()); // cbExtra
    for (name, alt) in fonts {
        let mut ffn = vec![0u8; 39];
        let primary = utf16z(name);
        if alt.is_some() {
            ffn[4] = (primary.len() / 2) as u8; // ixchSzAlt
        }
        ffn.extend_from_slice(&primary);
        if let Some(a) = alt {
            ffn.extend_from_slice(&utf16z(a));
        }
        v.push(ffn.len() as u8); // cchData
        v.extend_from_slice(&ffn);
    }
    v
}

/// A ChpxFkp page mapping run boundaries (in CPs, ends exclusive) to grpprls.
fn chpx_fkp(runs: &[(u32, Vec<u8>)], end_cp: u32) -> Vec<u8> {
    let mut page = vec![0u8; 512];
    let n = runs.len();
    for (i, r) in runs.iter().enumerate() {
        page[i * 4..i * 4 + 4].copy_from_slice(&fc_of(r.0).to_le_bytes());
    }
    page[n * 4..n * 4 + 4].copy_from_slice(&fc_of(end_cp).to_le_bytes());
    let rgb_at = (n + 1) * 4;
    let mut data_at = rgb_at + n + 1;
    for (i, r) in runs.iter().enumerate() {
        if r.1.is_empty() {
            page[rgb_at + i] = 0;
            continue;
        }
        // Offsets are half-bytes; align the data to even offsets.
        data_at += data_at & 1;
        let half = (data_at / 2) as u8;
        page[rgb_at + i] = half;
        page[data_at] = r.1.len() as u8;
        page[data_at + 1..data_at + 1 + r.1.len()].copy_from_slice(&r.1);
        data_at += 1 + r.1.len();
    }
    page[511] = n as u8;
    page
}

/// A PapxFkp page mapping paragraph-mark FCs (as "FC after the mark") to PAPX data
/// (istd + grpprl).
fn papx_fkp(entries: &[(u32, u16, Vec<u8>)]) -> Vec<u8> {
    let mut page = vec![0u8; 512];
    let n = entries.len();
    for (i, e) in entries.iter().enumerate() {
        page[i * 4..i * 4 + 4].copy_from_slice(&fc_of(e.0).to_le_bytes());
    }
    page[n * 4..n * 4 + 4].copy_from_slice(&fc_of(0x3FF0_0000).to_le_bytes()); // far end
    let rgbx_at = (n + 1) * 4;
    let mut data_at = rgbx_at + n * 13;
    for (i, e) in entries.iter().enumerate() {
        data_at += data_at & 1;
        let half = (data_at / 2) as u8;
        page[rgbx_at + i * 13] = half;
        let grp = [e.1.to_le_bytes().as_slice(), e.2.as_slice()].concat();
        let cb = ((grp.len() + 2) / 2) as u8;
        page[data_at] = cb;
        page[data_at + 1..data_at + 1 + grp.len()].copy_from_slice(&grp);
        data_at += 1 + grp.len() + ((1 + grp.len()) & 1);
    }
    page[511] = n as u8;
    page
}

/// A document with one uncompressed text piece, a stylesheet, a font table and formatting
/// pages, assembled the way Word lays out the streams.
struct RichSpec {
    text: &'static str,
    /// Styles in istd order, (stk, base, name, papx, chpx).
    styles: Vec<(u8, u16, &'static str, Vec<u8>, Vec<u8>)>,
    fonts: Vec<&'static str>,
    /// Character runs (start CP, grpprl); text length ends the last.
    chpx_runs: Vec<(u32, Vec<u8>)>,
    /// Paragraph entries (mark CP, istd, grpprl).
    papx_entries: Vec<(u32, u16, Vec<u8>)>,
}

/// Optional extras of a [`rich_doc`]: Prcs and the piece's Prm, alternate font names
/// (parallel to `RichSpec::fonts`) and next-paragraph styles (parallel to `RichSpec::styles`).
#[derive(Default)]
struct RichExtra {
    prcs: Vec<Vec<u8>>,
    prm: u16,
    alt_fonts: Vec<Option<&'static str>>,
    style_next: Vec<u16>,
}

fn rich_doc(spec: &RichSpec) -> Vec<u8> {
    rich_doc_with(spec, &RichExtra::default())
}

fn rich_doc_with(spec: &RichSpec, extra: &RichExtra) -> Vec<u8> {
    let text: String = spec.text.into();
    let ccp = text.chars().count() as u32;
    let mut plc = plcpcd(&[Piece { compressed: false, text: text.clone() }], TEXT_FC);
    // The single Pcd is last: its Prm is the final two bytes.
    let n = plc.len();
    plc[n - 2..].copy_from_slice(&extra.prm.to_le_bytes());
    let mut clx = Vec::new();
    for g in &extra.prcs {
        clx.push(0x01u8);
        clx.extend_from_slice(&(g.len() as u16).to_le_bytes());
        clx.extend_from_slice(g);
    }
    clx.push(0x02u8);
    clx.extend_from_slice(&(plc.len() as u32).to_le_bytes());
    clx.extend_from_slice(&plc);

    // WordDocument: FIB | padding | text | FKP pages (512-aligned).
    let chpx_page_at = (TEXT_FC as usize + text.len() * 2).div_ceil(512) * 512;
    let papx_page_at = chpx_page_at + 512;
    let mut word = vec![0u8; TEXT_FC as usize];
    word.extend_from_slice(&piece_bytes(&Piece { compressed: false, text: text.clone() }));
    word.resize(chpx_page_at, 0);
    word.extend_from_slice(&chpx_fkp(&spec.chpx_runs, ccp));
    word.extend_from_slice(&papx_fkp(&spec.papx_entries));

    // Table: STSH | FFN | Clx | bin tables.
    let styles: Vec<Vec<u8>> = spec
        .styles
        .iter()
        .enumerate()
        .map(|(i, s)| std_bytes_next(s.0, s.1, extra.style_next.get(i).copied().unwrap_or(0x0FFF), s.2, &s.3, &s.4))
        .collect();
    let stsh = stsh_bytes(&styles);
    let fonts: Vec<(&str, Option<&str>)> = spec.fonts.iter().enumerate().map(|(i, n)| (*n, extra.alt_fonts.get(i).copied().flatten())).collect();
    let ffn = ffn_bytes_alt(&fonts);
    let clx_at = 0x40;
    let bins_at = clx_at + clx.len();
    let plcf_chpx =
        [0u32.to_le_bytes().as_slice(), fc_of(0x3FF0_0000).to_le_bytes().as_slice(), ((chpx_page_at / 512) as u32).to_le_bytes().as_slice()].concat();
    let plcf_papx =
        [0u32.to_le_bytes().as_slice(), fc_of(0x3FF0_0000).to_le_bytes().as_slice(), ((papx_page_at / 512) as u32).to_le_bytes().as_slice()].concat();
    let mut table = vec![0u8; clx_at];
    table.extend_from_slice(&clx);
    let chpx_bins_at = table.len();
    table.extend_from_slice(&plcf_chpx);
    let papx_bins_at = table.len();
    table.extend_from_slice(&plcf_papx);
    table.extend(std::iter::repeat_n(0, stsh.len() + ffn.len()));
    let stsh_at = table.len() - stsh.len() - ffn.len();
    let ffn_at = stsh_at + stsh.len();
    table[stsh_at..stsh_at + stsh.len()].copy_from_slice(&stsh);
    table[ffn_at..ffn_at + ffn.len()].copy_from_slice(&ffn);
    let _ = bins_at;

    let spec_fib = FibSpec {
        ccp_text: ccp,
        pairs: vec![
            (33, clx_at as u32, clx.len() as u32),
            (1, stsh_at as u32, stsh.len() as u32),
            (15, ffn_at as u32, ffn.len() as u32),
            (12, chpx_bins_at as u32, plcf_chpx.len() as u32),
            (13, papx_bins_at as u32, plcf_papx.len() as u32),
        ],
        ..Default::default()
    };
    let fib = fib_bytes(&spec_fib);
    word[..fib.len()].copy_from_slice(&fib);
    cfb_file(&[("WordDocument", &word), ("0Table", &table)])
}

fn para0(doc: &wordcraft_doc::Document) -> wordcraft_doc::Paragraph {
    doc.body.first().and_then(|b| b.as_para().cloned()).unwrap_or_default()
}

#[test]
fn character_formatting_applied() {
    let spec = RichSpec {
        text: "plain bold red\r",
        styles: vec![(1, 0x0FFF, "Normal", Vec::new(), Vec::new())],
        fonts: vec!["Times New Roman", "Arial"],
        chpx_runs: vec![
            (0, Vec::new()),
            (6, grpprl(&[(0x0835, &[0x01])])),                     // bold
            (11, grpprl(&[(0x0835, &[0x01]), (0x2A42, &[0x06])])), // bold + red (ico 6)
        ],
        papx_entries: vec![(0, 0, Vec::new())],
    };
    let doc = read(&rich_doc(&spec)).expect("opens");
    let p = para0(&doc);
    assert_eq!(p.text, "plain bold red");
    // runs: plain(0..6) bold(6..11) bold+red(11..16)
    assert!(p.runs.get(1).is_some_and(|r| r.props.bold == Some(true)), "run 1 bold: {:?}", p.runs);
    assert!(p.runs.get(2).is_some_and(|r| r.props.bold == Some(true)), "run 2 bold+red: {:?}", p.runs);
    assert!(p.runs.get(2).is_some_and(|r| matches!(r.props.color, Some(wordcraft_doc::props::TextColor::Rgb(_)))), "run 2 red");
    assert!(p.runs.first().is_some_and(|r| r.props.bold.is_none()));
}

#[test]
fn font_and_size_from_style_and_direct() {
    let spec = RichSpec {
        text: "Hello\r",
        styles: vec![
            (1, 0x0FFF, "Normal", Vec::new(), grpprl(&[(0x4A43, &24u16.to_le_bytes())])), // 12pt
            (2, 0x0FFF, "Emphasis", Vec::new(), grpprl(&[(0x4A4F, &1u16.to_le_bytes())])), // Arial
        ],
        fonts: vec!["Times New Roman", "Arial"],
        chpx_runs: vec![(0, grpprl(&[(0x4A4F, &1u16.to_le_bytes())]))],
        papx_entries: vec![(0, 0, Vec::new())],
    };
    let doc = read(&rich_doc(&spec)).expect("opens");
    assert!(doc.styles.styles.iter().any(|s| s.name == "Normal" && s.chr.size == Some(12.0)), "Normal 12pt");
    let p = para0(&doc);
    assert!(p.runs.first().is_some_and(|r| r.props.font.as_deref() == Some("Arial")), "run font: {:?}", p.runs);
}

#[test]
fn paragraph_props_and_heading_style() {
    let spec = RichSpec {
        text: "centered\rplain\r",
        styles: vec![
            (1, 0x0FFF, "Normal", Vec::new(), Vec::new()),
            (1, 0, "heading 1", grpprl(&[(0x2461, &[0x01, 0x00])]), Vec::new()), // centered
        ],
        fonts: vec!["Times New Roman"],
        chpx_runs: vec![(0, Vec::new())],
        papx_entries: vec![(0, 1, Vec::new()), (9, 0, grpprl(&[(0x845E, &720u16.to_le_bytes())]))],
    };
    let doc = read(&rich_doc(&spec)).expect("opens");
    let heading = doc.styles.styles.iter().find(|s| s.name == "Heading 1").expect("style");
    assert_eq!(heading.para.align, Some(wordcraft_doc::props::Align::Center));
    let p0 = doc.body.first().and_then(|b| b.as_para()).expect("para");
    assert_eq!(p0.props.style.as_deref(), Some("Heading 1"));
    assert_eq!(heading.para.outline_level, Some(0));
    let p1 = doc.body.get(1).and_then(|b| b.as_para()).expect("para");
    assert_eq!(p1.props.indent_left, Some(36.0)); // 720 twips = 36pt
    assert_eq!(p1.props.style.as_deref(), Some("Normal"));
}

/// #534: Chinese theses set a first-line indent of two characters (`sprmPDxaLeft1`, whose operand
/// is two bytes, plus `sprmPDxcLeft1` in hundredths of a character), and exact line spacing as a
/// negative LSPD `dyaLine`. Both were lost: the indent read as four bytes, and exact 20pt as
/// 799pt, a page per empty paragraph.
#[test]
fn first_line_indent_chars_and_exact_line_spacing() {
    let lspd = |dya: i16, mult: u16| [dya.to_le_bytes(), mult.to_le_bytes()].concat();
    let (exact, multiple) = (lspd(-400, 0), lspd(360, 1));
    let spec = RichSpec {
        text: "indented\rexact\rold\r",
        styles: vec![(1, 0, "Normal", Vec::new(), Vec::new())],
        fonts: vec!["Times New Roman"],
        chpx_runs: vec![(0, Vec::new())],
        papx_entries: vec![
            (0, 0, grpprl(&[(0x8460, &480i16.to_le_bytes()), (0x4457, &200i16.to_le_bytes()), (0x6412, &multiple)])),
            (9, 0, grpprl(&[(0x6412, &exact)])),
            (15, 0, grpprl(&[(0x8411, &(-360i16).to_le_bytes())])),
        ],
    };
    let doc = read(&rich_doc(&spec)).expect("opens");
    let para = |i: usize| doc.body.get(i).and_then(|b| b.as_para()).expect("para").props.clone();
    let p0 = para(0);
    assert_eq!((p0.indent_first, p0.indent_first_chars), (Some(24.0), Some(2.0)));
    assert_eq!(p0.line_spacing, Some(wordcraft_doc::props::LineSpacing::Multiple(1.5)));
    assert_eq!(para(1).line_spacing, Some(wordcraft_doc::props::LineSpacing::Exactly(20.0)));
    assert_eq!(para(2).indent_first, Some(-18.0)); // sprmPDxaLeft1_80: a hanging indent
}

/// Local corpus (gitignored `plan/word/fixtures`): real files must open with text and styles.
#[test]
fn corpus_files_open() {
    let Some(dir) = std::env::var_os("WORDCRAFT_DOCBIN_FIXTURES") else { return };
    let mut checked = 0;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("doc")) {
                let bytes = std::fs::read(&p).expect("read");
                let doc = match super::read(&bytes) {
                    Ok(d) => d,
                    // Word 6/95 files are a documented, intentional rejection.
                    Err(super::DocbinError::NotWord(m)) if m.contains("6.0/95") => continue,
                    Err(e) => panic!("{}: {e}", p.display()),
                };
                assert!(!doc.styles.styles.is_empty(), "{}: no styles", p.display());
                let text = doc.plain_text(wordcraft_doc::StoryRef::Body);
                assert!(text.chars().count() > 3, "{}: no text", p.display());
                checked += 1;
            }
        }
    }
    assert!(checked > 0, "no .doc files found in {}", dir.display());
}

// ---------------------------------------------------------------------------------------------
// D3: sections, headers/footers, tables.

/// RichSpec extended for the D3 fixtures.
struct StructSpec {
    text: &'static str,
    /// Extra pieces after the main text: header subdocument stories.
    header_stories: Vec<&'static str>,
    /// (end_cp, sepx grpprl) per section; the last one reaching the text end is the final one.
    sections: Vec<(u32, Vec<u8>)>,
    /// Character runs as in RichSpec.
    chpx_runs: Vec<(u32, Vec<u8>)>,
    /// Paragraph entries as in RichSpec, with the grpprl also carrying table sprms.
    papx_entries: Vec<(u32, u16, Vec<u8>)>,
    /// Raw `PlfLst` bytes (LSTF array + appended LVLs) for FIB pair 73.
    plf_lst: Vec<u8>,
    /// Raw `PlfLfo` bytes for FIB pair 74.
    plf_lfo: Vec<u8>,
    /// Footnote subdocument stories, placed right after the main text.
    note_stories: Vec<&'static str>,
    /// Endnote subdocument stories, placed after the header stories.
    endnote_stories: Vec<&'static str>,
    /// Extra raw Table-stream blobs with their FIB pair index.
    extra_pairs: Vec<(usize, Vec<u8>)>,
    /// The Data stream (pictures), written when non-empty.
    data_stream: Vec<u8>,
}

fn struct_doc(spec: &StructSpec) -> Vec<u8> {
    let text: String = spec.text.into();
    let ccp = text.chars().count() as u32;
    let mut all: Vec<Piece> = vec![Piece { compressed: false, text: text.clone() }];
    let mut story_ends = Vec::new();
    let mut ftn_len = 0u32;
    for s in &spec.note_stories {
        let t: String = (*s).into();
        ftn_len += t.chars().count() as u32;
        all.push(Piece { compressed: false, text: t });
    }
    let mut hdd_len = 0u32;
    for s in &spec.header_stories {
        let t: String = (*s).into();
        hdd_len += t.chars().count() as u32;
        story_ends.push(hdd_len);
        all.push(Piece { compressed: false, text: t });
    }
    let mut edn_len = 0u32;
    for s in &spec.endnote_stories {
        let t: String = (*s).into();
        edn_len += t.chars().count() as u32;
        all.push(Piece { compressed: false, text: t });
    }
    let plc = plcpcd(&all, TEXT_FC);
    let mut clx = vec![0x02u8];
    clx.extend_from_slice(&(plc.len() as u32).to_le_bytes());
    clx.extend_from_slice(&plc);

    let chpx_page_at = (TEXT_FC as usize + text.len() * 2).div_ceil(512) * 512;
    let papx_page_at = chpx_page_at + 512;
    let mut word = vec![0u8; TEXT_FC as usize];
    for piece in &all {
        word.extend_from_slice(&piece_bytes(piece));
    }
    word.resize(chpx_page_at, 0);
    word.extend_from_slice(&chpx_fkp(&spec.chpx_runs, ccp));
    word.extend_from_slice(&papx_fkp(&spec.papx_entries));

    // PlcfSed: aCP[n+1] + aSed[n] (fn 2, fcSepx 4, fnMpr 2, fcMpr 4), and each Sepx after the
    // pages. Plcfhdd: separator stories first (6 empties) then one empty + one per story.
    let mut sepxes = Vec::new();
    let mut plcf_sed = Vec::new();
    let mut seds = Vec::new();
    let first_sepx = papx_page_at + 512;
    let mut off = first_sepx;
    plcf_sed.extend_from_slice(&0u32.to_le_bytes());
    for (end, grpprl) in &spec.sections {
        plcf_sed.extend_from_slice(&end.to_le_bytes());
        seds.extend_from_slice(&[0, 0]); // fn
        seds.extend_from_slice(&(off as u32).to_le_bytes());
        seds.extend_from_slice(&[0, 0, 0, 0, 0, 0]); // fnMpr + fcMpr
        let mut sepx = (grpprl.len() as u16).to_le_bytes().to_vec();
        sepx.extend_from_slice(grpprl);
        sepxes.push(sepx);
        off += grpprl.len() + 2;
    }
    plcf_sed.extend_from_slice(&seds);
    // Plcfhdd: six empty separator stories, then the section's six slots: even hdr, odd hdr,
    // even ftr, odd ftr, first hdr, first ftr — story 0 goes into the odd header slot (1).
    let mut plcf_hdd: Vec<u8> = Vec::new();
    let mut cp = 0u32;
    plcf_hdd.extend_from_slice(&cp.to_le_bytes());
    for _ in 0..6 {
        plcf_hdd.extend_from_slice(&cp.to_le_bytes()); // separators: empty
    }
    // slot 0 (even header): empty
    plcf_hdd.extend_from_slice(&cp.to_le_bytes());
    // slot 1 (odd header): the story, when present
    let has_story = !spec.header_stories.is_empty();
    if has_story {
        cp += spec.header_stories.iter().map(|s| s.chars().count() as u32).sum::<u32>();
    }
    plcf_hdd.extend_from_slice(&cp.to_le_bytes());
    // remaining slots: empty
    for _ in 0..4 {
        plcf_hdd.extend_from_slice(&cp.to_le_bytes());
    }

    let clx_at = 0x40;
    let mut table = vec![0u8; clx_at];
    table.extend_from_slice(&clx);
    let chpx_bins_at = table.len();
    let plcf_chpx =
        [0u32.to_le_bytes().as_slice(), fc_of(0x3FF0_0000).to_le_bytes().as_slice(), ((chpx_page_at / 512) as u32).to_le_bytes().as_slice()].concat();
    table.extend_from_slice(&plcf_chpx);
    let papx_bins_at = table.len();
    let plcf_papx =
        [0u32.to_le_bytes().as_slice(), fc_of(0x3FF0_0000).to_le_bytes().as_slice(), ((papx_page_at / 512) as u32).to_le_bytes().as_slice()].concat();
    table.extend_from_slice(&plcf_papx);
    let sed_at = table.len();
    table.extend_from_slice(&plcf_sed);
    let hdd_plc_at = table.len();
    table.extend_from_slice(&plcf_hdd);
    let stsh = stsh_bytes(&[std_bytes(1, 0x0FFF, "Normal", &Vec::new(), &Vec::new())]);
    let stsh_at = table.len();
    table.extend_from_slice(&stsh);
    let ffn = ffn_bytes(&["Times New Roman"]);
    let ffn_at = table.len();
    table.extend_from_slice(&ffn);
    let lst_at = table.len();
    table.extend_from_slice(&spec.plf_lst);
    let lfo_at = table.len();
    table.extend_from_slice(&spec.plf_lfo);

    let mut extra_at = Vec::new();
    for (i, blob) in &spec.extra_pairs {
        let at = table.len() as u32;
        table.extend_from_slice(blob);
        extra_at.push((*i, at, blob.len() as u32));
    }

    let mut pairs = vec![
        (33, clx_at as u32, clx.len() as u32),
        (1, stsh_at as u32, stsh.len() as u32),
        (15, ffn_at as u32, ffn.len() as u32),
        (12, chpx_bins_at as u32, plcf_chpx.len() as u32),
        (13, papx_bins_at as u32, plcf_papx.len() as u32),
        (6, sed_at as u32, plcf_sed.len() as u32),
        (11, hdd_plc_at as u32, plcf_hdd.len() as u32),
    ];
    if !spec.plf_lst.is_empty() {
        pairs.push((73, lst_at as u32, spec.plf_lst.len() as u32));
    }
    if !spec.plf_lfo.is_empty() {
        pairs.push((74, lfo_at as u32, spec.plf_lfo.len() as u32));
    }
    pairs.extend(extra_at);
    let spec_fib = FibSpec { ccp_text: ccp, ccp_ftn: ftn_len, ccp_hdd: hdd_len, ccp_edn: edn_len, pairs, ..Default::default() };
    let fib = fib_bytes(&spec_fib);
    word[..fib.len()].copy_from_slice(&fib);
    word.extend(std::iter::repeat_n(0, first_sepx - word.len()));
    for sepx in sepxes {
        word.extend_from_slice(&sepx);
    }
    if spec.data_stream.is_empty() {
        cfb_file(&[("WordDocument", &word), ("0Table", &table)])
    } else {
        cfb_file(&[("WordDocument", &word), ("0Table", &table), ("Data", &spec.data_stream)])
    }
}

#[test]
fn section_props_parsed() {
    let spec = StructSpec {
        text: "a\rb\r",
        header_stories: vec![],
        sections: vec![(2, grpprl(&[(0xB01F, &12240u16.to_le_bytes()), (0xB020, &15840u16.to_le_bytes()), (0xB021, &1440u16.to_le_bytes())]))],
        chpx_runs: vec![(0, Vec::new())],
        papx_entries: vec![(0, 0, Vec::new()), (2, 0, Vec::new())],
        plf_lst: Vec::new(),
        plf_lfo: Vec::new(),
        note_stories: vec![],
        endnote_stories: vec![],
        extra_pairs: Vec::new(),
        data_stream: Vec::new(),
    };
    let doc = read(&struct_doc(&spec)).expect("opens");
    // The single section ends at the text end: it becomes the document's last section.
    assert_eq!(doc.last_section.page_w, 612.0);
    assert_eq!(doc.last_section.page_h, 792.0);
    assert_eq!(doc.last_section.margin_left, 72.0);
}

#[test]
fn two_sections_box_the_first() {
    let spec = StructSpec {
        text: "first\rsecond\r",
        header_stories: vec![],
        sections: vec![(6, grpprl(&[(0x300A, &[0x01])])), (12, Vec::new())],
        chpx_runs: vec![(0, Vec::new())],
        papx_entries: vec![(0, 0, Vec::new()), (6, 0, Vec::new())],
        plf_lst: Vec::new(),
        plf_lfo: Vec::new(),
        note_stories: vec![],
        endnote_stories: vec![],
        extra_pairs: Vec::new(),
        data_stream: Vec::new(),
    };
    let doc = read(&struct_doc(&spec)).expect("opens");
    let p0 = doc.body.first().and_then(|b| b.as_para()).expect("para");
    let sec = p0.section.as_ref().expect("section on first paragraph");
    assert!(sec.title_page);
    assert!(!doc.last_section.title_page);
}

/// #880. Real `.doc` files end every non-final section with a 0x0C character whose CP just
/// past it is listed in PlcfSed. The walker split paragraphs only at 0x0D/0x07, so no
/// paragraph ended at the section CP and the section's properties were silently dropped:
/// the whole document then laid out with the final section's margins.
#[test]
fn section_break_char_ends_paragraph() {
    // "one" + section break (0x0C at CP 3) + "two" + paragraph mark; section 0 ends at CP 4.
    let text = "one\u{C}two\r";
    let margins = |tw: u16| grpprl(&[(0xB021, &tw.to_le_bytes()), (0xB022, &tw.to_le_bytes())]);
    let spec = StructSpec {
        text,
        sections: vec![(4, margins(850)), (8, margins(1133))], // 1.5 cm vs 2 cm
        papx_entries: vec![(0, 0, Vec::new()), (4, 0, Vec::new())],
        ..structured_spec(text, 8)
    };
    let doc = read(&struct_doc(&spec)).expect("opens");
    // The 0x0C ends the first paragraph and is not part of its text.
    assert_eq!(body_texts(&doc), ["one", "two"]);
    let sec = doc.body.first().and_then(|b| b.as_para()).and_then(|p| p.section.as_ref()).expect("section on the break paragraph");
    assert_eq!(sec.margin_left, 42.5); // 850 twips = 1.5 cm
    assert_eq!(sec.margin_right, 42.5);
    // The final section keeps its own margins (1133 twips = 56.65 pt, float-rounded).
    assert!((doc.last_section.margin_left - 56.65).abs() < 0.05, "{}", doc.last_section.margin_left);
    assert!((doc.last_section.margin_right - 56.65).abs() < 0.05, "{}", doc.last_section.margin_right);
}

/// #880. A 0x0C that PlcfSed does not list is a plain page break inside a paragraph
/// (Ctrl+Enter): it stays inline in the paragraph's text and creates no section of its own.
#[test]
fn page_break_char_stays_inline() {
    let text = "one\u{C}two\r";
    let spec = StructSpec {
        text,
        // One final section reaching the text end: no PlcfSed entry at CP 4.
        sections: vec![(8, grpprl(&[(0xB021, &1133u16.to_le_bytes())]))],
        ..structured_spec(text, 8)
    };
    let doc = read(&struct_doc(&spec)).expect("opens");
    assert_eq!(body_texts(&doc), ["one\u{C}two"]);
    for b in &doc.body {
        if let Some(p) = b.as_para() {
            assert!(p.section.is_none(), "section props on {p:?}");
        }
    }
    assert!((doc.last_section.margin_left - 56.65).abs() < 0.05, "{}", doc.last_section.margin_left);
}

#[test]
fn header_story_becomes_part() {
    let spec = StructSpec {
        text: "body\r",
        header_stories: vec!["Header text\r\r"],
        sections: vec![(5, Vec::new())],
        chpx_runs: vec![(0, Vec::new())],
        papx_entries: vec![(0, 0, Vec::new())],
        plf_lst: Vec::new(),
        plf_lfo: Vec::new(),
        note_stories: vec![],
        endnote_stories: vec![],
        extra_pairs: Vec::new(),
        data_stream: Vec::new(),
    };
    let doc = read(&struct_doc(&spec)).expect("opens");
    let id = doc.last_section.headers.default.expect("odd header part");
    let part = doc.parts.get(&id).expect("part");
    assert!(part.blocks.iter().any(|b| b.as_para().is_some_and(|p| p.text.contains("Header text"))));
}

/// A `sprmTDefTable` Prl for `edges` twip boundaries and TC80s of 20 bytes per column,
/// built per the spec (cb counts the bytes after it, plus one).
fn t_def_table(edges: &[i16]) -> Vec<u8> {
    let itc = edges.len() as u8 - 1;
    let mut body = vec![itc];
    for e in edges {
        body.extend_from_slice(&e.to_le_bytes());
    }
    for _ in 0..itc {
        body.extend_from_slice(&[0u8; 20]); // TC80: default cell
    }
    // cb = bytes after cb + 1.
    let cb = (body.len() + 1) as u16;
    let mut v = (0xD608u16).to_le_bytes().to_vec();
    v.extend_from_slice(&cb.to_le_bytes());
    v.extend_from_slice(&body);
    v
}

#[test]
fn table_assembled_from_marks() {
    // A 2×2 table: "a\x07 b\x07|ttp  c\x07 d\x07|ttp" then a normal paragraph.
    let def_table = t_def_table(&[0, 2880, 5760]);
    let text = "a\u{7}b\u{7}c\u{7}d\u{7}tail\r";
    let in_tbl = grpprl(&[(0x2416, &[0x01])]);
    let ttp: Vec<u8> = [grpprl(&[(0x2416, &[0x01]), (0x2417, &[0x01])]), def_table].concat();
    let spec = StructSpec {
        text,
        header_stories: vec![],
        sections: vec![(text.chars().count() as u32, Vec::new())],
        chpx_runs: vec![(0, Vec::new())],
        papx_entries: vec![
            (0, 0, in_tbl.clone()), // "a" cell paragraph
            (2, 0, ttp.clone()),    // "b" cell paragraph, ends the row (TTP)
            (4, 0, in_tbl),         // "c" cell paragraph
            (6, 0, ttp),            // "d" cell paragraph, ends the row (TTP)
            (8, 0, Vec::new()),     // "tail" paragraph
        ],
        plf_lst: Vec::new(),
        plf_lfo: Vec::new(),
        note_stories: vec![],
        endnote_stories: vec![],
        extra_pairs: Vec::new(),
        data_stream: Vec::new(),
    };
    let doc = read(&struct_doc(&spec)).expect("opens");
    let tbl = doc.body.iter().find_map(|b| b.as_table()).expect("table");
    assert_eq!(tbl.rows.len(), 2);
    assert_eq!(tbl.rows[0].cells.len(), 2);
    assert_eq!(tbl.grid.len(), 2);
    assert_eq!(tbl.grid[0], 144.0); // 2880 twips = 144 pt
    assert_eq!(tbl.total_width(), 288.0);
    let cell_text = |r: usize, c: usize| tbl.rows[r].cells[c].blocks.iter().filter_map(|b| b.as_para()).map(|p| p.text.clone()).collect::<String>();
    assert_eq!(cell_text(0, 0), "a");
    assert_eq!(cell_text(0, 1), "b");
    assert_eq!(cell_text(1, 1), "d");
    let last = doc.body.last().and_then(|b| b.as_para()).expect("tail para");
    assert_eq!(last.text, "tail");
}

#[test]
fn table_merges_borders_shading() {
    // Two rows: the first "a\x07 b\x07 c\x07" with cells 0-1 horizontally merged; the second
    // "d\x07 e\x07" with table borders, a shaded first cell and a vertical-merge restart.
    let def3 = t_def_table(&[0, 1920, 3840, 5760]);
    let def2 = t_def_table(&[0, 2880, 5760]);
    let borders = {
        // sprmTTableBorders80: cb = 0x18, then 6 × Brc80 (width 4/8pt, single, ico 1, space 0).
        let brc = [4u8, 1, 1, 0];
        let mut op = vec![0x18u8];
        for _ in 0..6 {
            op.extend_from_slice(&brc);
        }
        let mut v = (0xD605u16).to_le_bytes().to_vec();
        v.extend_from_slice(&op);
        v
    };
    let shd = {
        // sprmTSetShd: cb = 12, itc 0..1, Shd { cvFore = FFFF00 (yellow), ipat = 1 (solid,
        // which shows the foreground) }.
        let mut op = vec![12u8, 0, 1];
        op.extend_from_slice(&[0xFF, 0xFF, 0x00, 0x00]); // cvFore = yellow
        op.extend_from_slice(&[0, 0, 0, 0]); // cvBack
        op.extend_from_slice(&1u16.to_le_bytes()); // ipat solid
        let mut v = (0xD62Du16).to_le_bytes().to_vec();
        v.extend_from_slice(&op);
        v
    };
    let vert_merge = {
        // sprmTVertMerge: cb = 2, itc 0, flag 3 (restart).
        let mut v = (0xD62Bu16).to_le_bytes().to_vec();
        v.extend_from_slice(&[2, 0, 3]);
        v
    };
    let h_merge = {
        // sprmTMerge: itcFirstLim 0..2.
        let mut v = (0x5624u16).to_le_bytes().to_vec();
        v.extend_from_slice(&[0, 2]);
        v
    };
    let text = "a\u{7}b\u{7}c\u{7}d\u{7}e\u{7}\r";
    let ttp_base = || grpprl(&[(0x2416, &[0x01]), (0x2417, &[0x01])]);
    let mut row1: Vec<u8> = [ttp_base(), def3.clone(), h_merge].concat();
    let mut row2: Vec<u8> = [ttp_base(), def2, borders, shd, vert_merge, grpprl(&[(0x9407, &(-400i16).to_le_bytes())])].concat();
    let in_tbl = grpprl(&[(0x2416, &[0x01])]);
    let spec = StructSpec {
        text,
        header_stories: vec![],
        sections: vec![(text.chars().count() as u32, Vec::new())],
        chpx_runs: vec![(0, Vec::new())],
        papx_entries: vec![
            (0, 0, in_tbl.clone()),
            (2, 0, in_tbl.clone()),
            (4, 0, std::mem::take(&mut row1)), // "c" cell paragraph, ends row 1 (merge 0..2)
            (6, 0, in_tbl),
            (8, 0, std::mem::take(&mut row2)), // "e" cell paragraph, ends row 2
            (10, 0, Vec::new()),
        ],
        plf_lst: Vec::new(),
        plf_lfo: Vec::new(),
        note_stories: vec![],
        endnote_stories: vec![],
        extra_pairs: Vec::new(),
        data_stream: Vec::new(),
    };
    let doc = read(&struct_doc(&spec)).expect("opens");
    let tbl = doc.body.iter().find_map(|b| b.as_table()).expect("table");
    assert_eq!(tbl.rows.len(), 2);
    // Row 1: three cell marks merged to two cells, the first spanning two columns.
    assert_eq!(tbl.rows[0].cells.len(), 2);
    assert_eq!(tbl.rows[0].cells[0].span(), 2);
    let r0 = tbl.rows[0].cells[0].blocks.iter().filter_map(|b| b.as_para()).map(|p| p.text.clone()).collect::<String>();
    assert_eq!(r0, "ab");
    // Row 2: shading, vertical-merge restart, exact height 20pt.
    assert_eq!(tbl.rows[1].cells[0].props.shading, Some(wordcraft_doc::Rgb(255, 255, 0)));
    assert_eq!(tbl.rows[1].cells[0].props.vmerge, wordcraft_doc::props::VMerge::Restart);
    assert_eq!(tbl.rows[1].props.height, Some(20.0));
    // Table borders from the first row that carries them: single 0.5pt black.
    let b = tbl.props.borders.expect("borders");
    let top = b.top.expect("top border");
    assert_eq!(top.style, wordcraft_doc::props::BorderStyle::Single);
    assert_eq!(top.width, 0.5);
}

/// #880. A PAPX too large for the FKP is stored in the Data stream: the FKP grpprl is just
/// `sprmPHugePapx` with the fc of the real grpprl, which sits there as `cb: u16` plus bytes
/// ([MS-DOC] §2.6.68). Before the swap, the whole row shape (and any other huge-paragraph
/// properties) was lost.
#[test]
fn huge_papx_row_properties_apply() {
    // One row, one cell: the row-end mark's PAPX is the huge marker pointing at fc 0.
    let real: Vec<u8> = [grpprl(&[(0x2416, &[0x01]), (0x2417, &[0x01])]), t_def_table(&[0, 2880, 5760])].concat();
    let mut data_stream = (real.len() as u16).to_le_bytes().to_vec();
    data_stream.extend_from_slice(&real);
    let text = "a\u{7}tail\r";
    let spec = StructSpec {
        text,
        papx_entries: vec![
            (0, 0, grpprl(&[(0x6646, &0u32.to_le_bytes())])), // "a" cell paragraph, huge PAPX
            (2, 0, Vec::new()),                               // "tail" paragraph
        ],
        data_stream,
        ..structured_spec(text, text.chars().count() as u32)
    };
    let doc = read(&struct_doc(&spec)).expect("opens");
    let tbl = doc.body.iter().find_map(|b| b.as_table()).expect("table");
    assert_eq!(tbl.rows.len(), 1);
    assert_eq!(tbl.rows[0].cells.len(), 1);
    let cell: String = tbl.rows[0].cells[0].blocks.iter().filter_map(|b| b.as_para()).map(|p| p.text.clone()).collect();
    assert_eq!(cell, "a");
    // The grid comes from the huge grpprl's TDefTable, not from the FKP.
    assert_eq!(tbl.grid.len(), 2, "grid: {tbl:?}");
    assert_eq!(tbl.grid[0], 144.0); // 2880 twips = 144 pt
    assert_eq!(tbl.total_width(), 288.0);
    let last = doc.body.last().and_then(|b| b.as_para()).expect("tail para");
    assert_eq!(last.text, "tail");
}

/// #880: a huge-PAPX marker whose fc points past the Data stream, and one whose cb claims
/// 65 535 bytes when only the count exists: both fall back to no properties, keep the
/// paragraphs and never panic.
#[test]
fn hostile_huge_papx() {
    let data_stream = [0xFFu8, 0xFF]; // cb = 0xFFFF, nothing after it
    let text = "one\rtwo\r";
    let huge = |fc: u32| grpprl(&[(0x6646, &fc.to_le_bytes())]);
    let spec = StructSpec {
        text,
        // Paragraph 1's marker points past the stream; paragraph 2's cb overruns it.
        papx_entries: vec![(0, 0, huge(data_stream.len() as u32 + 1000)), (4, 0, huge(0))],
        data_stream: data_stream.to_vec(),
        ..structured_spec(text, text.chars().count() as u32)
    };
    let doc = read(&struct_doc(&spec)).expect("opens");
    assert_eq!(body_texts(&doc), ["one", "two"]);
    assert!(doc.body.iter().all(|b| b.as_table().is_none()), "no row detected");
}

/// #880: [MS-DOC] defines cell edges PER ROW (each row-end mark carries its own
/// `sprmTDefTable`), so a two-column banner above nine-edge data rows must not fix the
/// whole table's shape. The grid is the union of every row's edges, and each row's cells
/// take their widths from that row's own edges. The row-end PAPXs are huge (their
/// TDefTables don't fit one FKP page), pointing at real grpprls in the Data stream.
#[test]
fn per_row_edges_form_union_grid() {
    let banner_edges: [i16; 3] = [0, 5000, 10423];
    let data_edges: [i16; 9] = [0, 800, 1600, 2694, 3261, 3686, 4111, 8931, 9639];
    let mut data_stream = Vec::new();
    let huge_at = |data_stream: &mut Vec<u8>, real: Vec<u8>| -> Vec<u8> {
        let fc = data_stream.len() as u32;
        data_stream.extend_from_slice(&(real.len() as u16).to_le_bytes());
        data_stream.extend_from_slice(&real);
        grpprl(&[(0x6646, &fc.to_le_bytes())])
    };
    let ttp_row = |edges: &[i16]| [grpprl(&[(0x2416, &[0x01]), (0x2417, &[0x01])]), t_def_table(edges)].concat();
    let in_tbl = grpprl(&[(0x2416, &[0x01])]);
    let text = "a\u{7}b\u{7}c\u{7}d\u{7}e\u{7}f\u{7}g\u{7}h\u{7}i\u{7}j\u{7}k\u{7}l\u{7}m\u{7}n\u{7}o\u{7}p\u{7}q\u{7}r\u{7}tail\r";
    let spec = StructSpec {
        text,
        // Banner (a-b), then two 8-cell data rows (c-j, k-r), then a tail paragraph.
        papx_entries: vec![
            (0, 0, in_tbl.clone()),
            (2, 0, huge_at(&mut data_stream, ttp_row(&banner_edges))),
            (4, 0, in_tbl.clone()),
            (18, 0, huge_at(&mut data_stream, ttp_row(&data_edges))),
            (20, 0, in_tbl),
            (34, 0, huge_at(&mut data_stream, ttp_row(&data_edges))),
            (36, 0, Vec::new()),
        ],
        data_stream,
        ..structured_spec(text, text.chars().count() as u32)
    };
    let doc = read(&struct_doc(&spec)).expect("opens");
    let tbl = doc.body.iter().find_map(|b| b.as_table()).expect("table");
    assert_eq!(tbl.rows.len(), 3);
    assert_eq!(tbl.rows[0].cells.len(), 2);
    assert_eq!(tbl.rows[1].cells.len(), 8);
    // The grid is the widths of the sorted, de-duplicated union of both rows' edges.
    let pts = |es: &[i16]| -> Vec<f32> { es.iter().map(|&t| t as f32 / 20.0).collect() };
    let widths_of = |es: &[f32]| -> Vec<f32> { es.windows(2).map(|w| w[1] - w[0]).collect() };
    let mut union = pts(&banner_edges);
    union.extend(pts(&data_edges));
    union.sort_by(f32::total_cmp);
    union.dedup();
    assert_eq!(tbl.grid.len(), 10);
    assert_eq!(tbl.grid, widths_of(&union));
    // Both data rows carry their own edges' widths (800, 800, 1094, … twips), not the
    // banner's.
    let want = widths_of(&pts(&data_edges)).into_iter().map(Some).collect::<Vec<_>>();
    for r in &tbl.rows[1..] {
        assert_eq!(r.cells.iter().map(|c| c.props.width).collect::<Vec<_>>(), want);
    }
    // The banner cells too: 5000 and 5423 twips.
    let bpts = pts(&banner_edges);
    assert_eq!(tbl.rows[0].cells[0].props.width, Some(bpts[1] - bpts[0]));
    assert_eq!(tbl.rows[0].cells[1].props.width, Some(bpts[2] - bpts[1]));
    let last = doc.body.last().and_then(|b| b.as_para()).expect("tail para");
    assert_eq!(last.text, "tail");
}

/// #880 hardening: `sprmTDefTable` edges are hostile file data — unsorted, negative, at
/// the i16 extremes, and more per row than the model's 63-column cap. Nothing may panic,
/// every width stays non-negative, and the unioned grid stays within the cap.
#[test]
fn hostile_edges() {
    let row1_edges: Vec<i16> = vec![5000, 0, -200, 32767];
    let row2_edges: Vec<i16> = (0..70).map(|i| i * 100).collect(); // 70 edges: past the cap
    let mut data_stream = Vec::new();
    let huge_at = |data_stream: &mut Vec<u8>, real: Vec<u8>| -> Vec<u8> {
        let fc = data_stream.len() as u32;
        data_stream.extend_from_slice(&(real.len() as u16).to_le_bytes());
        data_stream.extend_from_slice(&real);
        grpprl(&[(0x6646, &fc.to_le_bytes())])
    };
    let ttp_row = |edges: &[i16]| [grpprl(&[(0x2416, &[0x01]), (0x2417, &[0x01])]), t_def_table(edges)].concat();
    let in_tbl = grpprl(&[(0x2416, &[0x01])]);
    let text = "a\u{7}b\u{7}c\u{7}d\u{7}e\u{7}f\u{7}tail\r";
    let spec = StructSpec {
        text,
        // Row 1 (a-c) with the hostile edges; row 2 (d-f) wider than the column cap.
        papx_entries: vec![
            (0, 0, in_tbl.clone()),
            (4, 0, huge_at(&mut data_stream, ttp_row(&row1_edges))),
            (6, 0, in_tbl),
            (10, 0, huge_at(&mut data_stream, ttp_row(&row2_edges))),
            (12, 0, Vec::new()),
        ],
        data_stream,
        ..structured_spec(text, text.chars().count() as u32)
    };
    let doc = read(&struct_doc(&spec)).expect("opens");
    let tbl = doc.body.iter().find_map(|b| b.as_table()).expect("table");
    assert_eq!(tbl.rows.len(), 2);
    assert_eq!(tbl.rows[0].cells.len(), 3);
    assert_eq!(tbl.rows[1].cells.len(), 3);
    assert!(tbl.grid.len() <= 63, "grid columns: {}", tbl.grid.len());
    assert!(tbl.grid.iter().all(|w| *w >= 0.0 && w.is_finite()), "grid: {:?}", tbl.grid);
    for r in &tbl.rows {
        assert!(r.cells.iter().all(|c| c.props.width.is_none_or(|w| w >= 0.0 && w.is_finite())));
    }
    assert!(tbl.total_width().is_finite());
    // Row 2's own edges still size its first cell (100 twips = 5pt).
    assert_eq!(tbl.rows[1].cells[0].props.width, Some(100.0f32 / 20.0));
    let last = doc.body.last().and_then(|b| b.as_para()).expect("tail para");
    assert_eq!(last.text, "tail");
}

/// #880 fix round 1: the union grid interleaves every row's edges, so a coarse row's cell
/// (a change-history banner: `2018-06`, `SP-230144`) covers several union columns. With
/// span 1 it was laid out inside the first narrow column and wrapped mid-value
/// (`SP-230` + `144`); a cell must span the union-grid columns it covers. Explicit
/// `sprmTMerge` spans keep winning over the computed ones.
#[test]
fn union_grid_cells_span_covered_columns() {
    let coarse_edges: [i16; 3] = [0, 1000, 2000]; // 2 cells over the fine grid
    let fine_edges: [i16; 5] = [0, 500, 1000, 1500, 2000]; // 4 cells = the grid itself
    let mut data_stream = Vec::new();
    let huge_at = |data_stream: &mut Vec<u8>, real: Vec<u8>| -> Vec<u8> {
        let fc = data_stream.len() as u32;
        data_stream.extend_from_slice(&(real.len() as u16).to_le_bytes());
        data_stream.extend_from_slice(&real);
        grpprl(&[(0x6646, &fc.to_le_bytes())])
    };
    let ttp_row = |edges: &[i16]| [grpprl(&[(0x2416, &[0x01]), (0x2417, &[0x01])]), t_def_table(edges)].concat();
    // sprmTMerge over both cells of the coarse row: the sprm span (2) must win.
    let merged_row = [ttp_row(&coarse_edges), grpprl(&[(0x5624, &[0, 2])])].concat();
    let in_tbl = grpprl(&[(0x2416, &[0x01])]);
    let text = "a\u{7}b\u{7}c\u{7}d\u{7}e\u{7}f\u{7}g\u{7}h\u{7}tail\r";
    let spec = StructSpec {
        text,
        // Coarse row (a-b), fine row (c-f), merged coarse row (g-h), then a tail paragraph.
        papx_entries: vec![
            (0, 0, in_tbl.clone()),
            (2, 0, huge_at(&mut data_stream, ttp_row(&coarse_edges))),
            (4, 0, in_tbl.clone()),
            (10, 0, huge_at(&mut data_stream, ttp_row(&fine_edges))),
            (12, 0, in_tbl),
            (14, 0, huge_at(&mut data_stream, merged_row)),
            (16, 0, Vec::new()),
        ],
        data_stream,
        ..structured_spec(text, text.chars().count() as u32)
    };
    let doc = read(&struct_doc(&spec)).expect("opens");
    let tbl = doc.body.iter().find_map(|b| b.as_table()).expect("table");
    assert_eq!(tbl.rows.len(), 3);
    assert_eq!(tbl.grid, vec![25.0f32; 4], "grid: {:?}", tbl.grid);
    // The coarse row's two cells each cover two grid columns — their text lays out across
    // both instead of wrapping inside the first.
    assert_eq!(tbl.rows[0].cells.len(), 2);
    assert_eq!(tbl.rows[0].cells[0].props.span, 2);
    assert_eq!(tbl.rows[0].cells[1].props.span, 2);
    // The cell width covers its spanned union columns (50pt = 25 + 25).
    assert_eq!(tbl.rows[0].cells[0].props.width, Some(tbl.grid[0] + tbl.grid[1]));
    assert_eq!(tbl.rows[0].cells[1].props.width, Some(tbl.grid[2] + tbl.grid[3]));
    // The fine row defines the grid: one column per cell.
    assert_eq!(tbl.rows[1].cells.len(), 4);
    assert!(tbl.rows[1].cells.iter().all(|c| c.props.span == 1), "fine row spans");
    // sprmTMerge keeps winning: the merged cell's span is the sprm's 2, not the 4 grid
    // columns its edges cover.
    assert_eq!(tbl.rows[2].cells.len(), 1);
    assert_eq!(tbl.rows[2].cells[0].props.span, 2);
    let last = doc.body.last().and_then(|b| b.as_para()).expect("tail para");
    assert_eq!(last.text, "tail");
}

/// #880 fix round 2: `sprmTDxaGapHalf` is the table's cell side margin (half the gap
/// between cells, applied inside each cell). The change-history rows carry 40 twips = 2pt
/// — dropping it made layout fall back to Word's 108-twip default and wrap `SP-230144`
/// mid-value. It maps 1:1 onto OOXML `w:tblCellMar` → `TableProps.cell_margins`
/// `[top, left, bottom, right]`, first row wins like the other table-level props.
#[test]
fn table_cell_margins_from_gap_half() {
    let def_table = t_def_table(&[0, 2880, 5760]);
    let gap_bytes = 40i16.to_le_bytes(); // 40 twips = 2pt
    let gap = (0x9602u16, gap_bytes.as_slice()); // sprmTDxaGapHalf
    let text = "a\u{7}b\u{7}tail\r";
    let in_tbl = grpprl(&[(0x2416, &[0x01])]);
    let ttp = [grpprl(&[(0x2416, &[0x01]), (0x2417, &[0x01])]), def_table, grpprl(&[gap])].concat();
    let spec = StructSpec {
        text,
        papx_entries: vec![
            (0, 0, in_tbl),     // "a" cell paragraph
            (2, 0, ttp),        // "b" cell paragraph, ends the row (TTP)
            (4, 0, Vec::new()), // "tail" paragraph
        ],
        ..structured_spec(text, text.chars().count() as u32)
    };
    let doc = read(&struct_doc(&spec)).expect("opens");
    let tbl = doc.body.iter().find_map(|b| b.as_table()).expect("table");
    assert_eq!(tbl.rows.len(), 1);
    assert_eq!(tbl.props.cell_margins, Some([0.0, 2.0, 0.0, 2.0]), "props: {:?}", tbl.props);
    // The grid and cell widths are untouched by the margin.
    assert_eq!(tbl.grid, vec![144.0f32, 144.0]);
    assert_eq!(tbl.rows[0].cells[0].props.width, Some(144.0));
}

/// #880 fix round 3: a .doc table whose rows carry `sprmTDefTable` edges is
/// edge-defined — Word lays it out at exactly those widths (the OOXML equivalent is
/// `w:tblLayout w:type="fixed"`). docbin left the flag at its default, so layout's
/// autofit recomputed columns from content and the model's exact TDefTable widths never
/// reached the page (the rendered TDoc column was ~31 pt instead of 54.7). Tables with
/// no edges of their own keep the autofit default.
#[test]
fn edge_defined_table_lays_out_fixed() {
    let text = "a\u{7}b\u{7}tail\r";
    let spec_of = |ttp: Vec<u8>| StructSpec {
        text,
        papx_entries: vec![
            (0, 0, grpprl(&[(0x2416, &[0x01])])), // "a" cell paragraph
            (2, 0, ttp),                          // "b" cell paragraph, ends the row
            (4, 0, Vec::new()),                   // "tail" paragraph
        ],
        ..structured_spec(text, text.chars().count() as u32)
    };
    // With TDefTable edges: edge-defined, so the grid must reach the page unchanged.
    let ttp_edges = [grpprl(&[(0x2416, &[0x01]), (0x2417, &[0x01])]), t_def_table(&[0, 2880, 5760])].concat();
    let doc = read(&struct_doc(&spec_of(ttp_edges))).expect("opens");
    let tbl = doc.body.iter().find_map(|b| b.as_table()).expect("table");
    assert_eq!(tbl.grid, vec![144.0f32, 144.0]);
    assert!(tbl.props.fixed, "edge-defined table must lay out fixed, props: {:?}", tbl.props);
    // Without edges: no grid, autofit keeps deciding the widths.
    let ttp_plain = grpprl(&[(0x2416, &[0x01]), (0x2417, &[0x01])]);
    let doc = read(&struct_doc(&spec_of(ttp_plain))).expect("opens");
    let tbl = doc.body.iter().find_map(|b| b.as_table()).expect("table");
    assert!(tbl.grid.is_empty());
    assert!(!tbl.props.fixed, "edgeless table stays autofit, props: {:?}", tbl.props);
}

/// #880 fix round 4, defect 1: real Word rows are written as N content cells, each ended
/// by its own cell mark, followed by an **empty** row-end (TTP) mark that carries the
/// row's `sprmTDefTable` (verified against the 3GPP repro: every change-history row is
/// 8 content marks + an empty TTP whose TDefTable defines itcMac = 8). The row-end mark
/// terminates the row; when it ends no content of its own and the TDefTable already has
/// a column per flushed cell, it must not flush a phantom (N+1)th cell — that extra
/// cell had no column (width `None`) and pushed every real cell's span one column over,
/// misaligning the fixed grid (Subject 241 pt → ~50 pt).
#[test]
fn row_end_mark_is_not_an_extra_cell() {
    let banner_edges: [i16; 2] = [0, 9639];
    let data_edges: [i16; 9] = [0, 800, 1600, 2694, 3261, 3686, 4111, 8931, 9639];
    let mut data_stream = Vec::new();
    let huge_at = |data_stream: &mut Vec<u8>, real: Vec<u8>| -> Vec<u8> {
        let fc = data_stream.len() as u32;
        data_stream.extend_from_slice(&(real.len() as u16).to_le_bytes());
        data_stream.extend_from_slice(&real);
        grpprl(&[(0x6646, &fc.to_le_bytes())])
    };
    let ttp_row = |edges: &[i16]| [grpprl(&[(0x2416, &[0x01]), (0x2417, &[0x01])]), t_def_table(edges)].concat();
    let in_tbl = grpprl(&[(0x2416, &[0x01])]);
    // Banner "B" + row-end, then a data row "a".."h" + row-end, then a tail paragraph.
    let text = "B\u{7}\u{7}a\u{7}b\u{7}c\u{7}d\u{7}e\u{7}f\u{7}g\u{7}h\u{7}\u{7}tail\r";
    let spec = StructSpec {
        text,
        papx_entries: vec![
            (0, 0, in_tbl.clone()),                                    // "B" cell paragraph
            (2, 0, huge_at(&mut data_stream, ttp_row(&banner_edges))), // banner row-end para
            (3, 0, in_tbl.clone()),                                    // "a"
            (5, 0, in_tbl.clone()),
            (7, 0, in_tbl.clone()),
            (9, 0, in_tbl.clone()),
            (11, 0, in_tbl.clone()),
            (13, 0, in_tbl.clone()),
            (15, 0, in_tbl.clone()),
            (17, 0, in_tbl.clone()),                                  // "h" cell paragraph
            (19, 0, huge_at(&mut data_stream, ttp_row(&data_edges))), // data row-end para
            (20, 0, Vec::new()),                                      // "tail"
        ],
        data_stream,
        ..structured_spec(text, text.chars().count() as u32)
    };
    let doc = read(&struct_doc(&spec)).expect("opens");
    let tbl = doc.body.iter().find_map(|b| b.as_table()).expect("table");
    assert_eq!(tbl.rows.len(), 2, "rows: {}", tbl.rows.len());
    // Banner: one cell spanning the whole (8-column union) grid.
    assert_eq!(tbl.rows[0].cells.len(), 1, "banner cells: {:?}", tbl.rows[0].cells.len());
    assert_eq!(tbl.rows[0].cells[0].props.span, 8);
    assert_eq!(tbl.rows[0].cells[0].props.width, Some(9639.0 / 20.0));
    // Data row: exactly the TDefTable's 8 cells, each with its own width (computed with
    // the reader's per-edge i16→pt arithmetic, so the assertion is bit-exact).
    assert_eq!(tbl.rows[1].cells.len(), 8, "data cells: {}", tbl.rows[1].cells.len());
    let pts: Vec<f32> = data_edges.iter().map(|&t| t as f32 / 20.0).collect();
    let want: Vec<Option<f32>> = pts.windows(2).map(|w| Some(w[1] - w[0])).collect();
    let got: Vec<Option<f32>> = tbl.rows[1].cells.iter().map(|c| c.props.width).collect();
    assert_eq!(got, want, "data widths");
    assert!(tbl.rows[1].cells.iter().all(|c| c.props.span == 1), "data spans");
    let last = doc.body.last().and_then(|b| b.as_para()).expect("tail para");
    assert_eq!(last.text, "tail");
}

/// #880 fix round 4, defect 2: the table-level props accumulator was never reset between
/// tables, so the first table's `sprmTDxaGapHalf` (108 twips = Word's default, written
/// explicitly) leaked into every later table and the change-history table's own 40-twip
/// gap lost the first-wins pick — its cells got 5.4 pt side margins instead of 2.0.
/// Props are per-table: the first row that carries the sprm wins within its own table.
#[test]
fn table_props_reset_between_tables() {
    let in_tbl = grpprl(&[(0x2416, &[0x01])]);
    let ttp = |gap_tw: i16, align: bool| {
        let gap = gap_tw.to_le_bytes();
        let mut prls = vec![(0x2416u16, &[0x01u8][..]), (0x2417, &[0x01][..]), (0x9602, gap.as_slice())];
        if align {
            prls.push((0x5400, &[0x01, 0x00][..])); // sprmTJc: center
        }
        grpprl(&prls)
    };
    let text = "a\u{7}b\u{7}mid\rc\u{7}d\u{7}";
    let spec = StructSpec {
        text,
        papx_entries: vec![
            (0, 0, in_tbl.clone()),  // "a"
            (2, 0, ttp(108, true)),  // "b": table 1's row end, 108-twip gap
            (4, 0, Vec::new()),      // "mid": closes table 1
            (8, 0, in_tbl),          // "c"
            (10, 0, ttp(40, false)), // "d": table 2's row end, 40-twip gap
        ],
        ..structured_spec(text, text.chars().count() as u32)
    };
    let doc = read(&struct_doc(&spec)).expect("opens");
    let tables: Vec<&wordcraft_doc::table::Table> = doc.body.iter().filter_map(|b| b.as_table()).collect();
    assert_eq!(tables.len(), 2, "tables: {}", tables.len());
    assert_eq!(tables[0].props.cell_margins, Some([0.0, 5.4, 0.0, 5.4]), "table 1 margins");
    assert_eq!(tables[0].props.align, Some(wordcraft_doc::props::Align::Center), "table 1 align");
    assert_eq!(tables[1].props.cell_margins, Some([0.0, 2.0, 0.0, 2.0]), "table 2 margins: {:?}", tables[1].props);
    assert_eq!(tables[1].props.align, None, "table 1's align must not leak");
}

// ---------------------------------------------------------------------------------------------
// D3: lists (PlfLst + PlfLfo) and the combined document.

/// One LVL: number text given as chars (a `0`-valued char at placeholder position `ph` is
/// the level placeholder), with a 720-twip left indent and a 360-twip hanging first line.
fn lvl(start: u32, nfc: u8, chars: &[u16], ph: bool) -> Vec<u8> {
    let mut v = vec![0u8; 28];
    v[0..4].copy_from_slice(&start.to_le_bytes());
    v[4] = nfc;
    if ph {
        v[6] = 1; // rgbxchNums[0]: the placeholder sits at one-based offset 1
    }
    v[15] = 1; // a space follows the number text
    let papx: Vec<u8> = [grpprl(&[(0x840F, &720u16.to_le_bytes())]), grpprl(&[(0x8411, &(-360i16).to_le_bytes())])].concat();
    v[25] = papx.len() as u8; // cbGrpprlPapx
    v.extend_from_slice(&papx);
    v.extend_from_slice(&(chars.len() as u16).to_le_bytes());
    for c in chars {
        v.extend_from_slice(&c.to_le_bytes());
    }
    v
}

/// A `PlfLst` with one multi-level list (nine decimal levels) or one simple list.
fn plf_lst(lsid: i32, simple: bool, levels: &[Vec<u8>]) -> Vec<u8> {
    let mut v = 1u16.to_le_bytes().to_vec();
    v.extend_from_slice(&lsid.to_le_bytes());
    v.extend_from_slice(&0u32.to_le_bytes()); // tplc
    for _ in 0..9 {
        v.extend_from_slice(&0x0FFFu16.to_le_bytes()); // no style links
    }
    v.push(u8::from(simple)); // flags: fSimpleList
    v.push(0); // grfhic
    for l in levels {
        v.extend_from_slice(l);
    }
    v
}

/// A `PlfLfo` with one LFO bound to `lsid`.
fn plf_lfo(lsid: i32) -> Vec<u8> {
    let mut v = 1u32.to_le_bytes().to_vec(); // lfoMac
    v.extend_from_slice(&lsid.to_le_bytes()); // LFO.lsid
    v.extend_from_slice(&[0u8; 8]); // unused1/2
    v.push(0); // clfolvl
    v.extend_from_slice(&[0u8; 3]); // ibstFltAutoNum, grfhic, unused3
    v.extend_from_slice(&0u32.to_le_bytes()); // LFOData.cp
    v
}

#[test]
fn list_numbering_parsed() {
    let levels: Vec<Vec<u8>> = (0..9)
        .map(|_| lvl(1, 0x00, &[0x0000, 0x002E], true)) // "%1." style, decimal
        .collect();
    let text = "one\rtwo\rplain\r";
    let listed = |off: bool| {
        if off { grpprl(&[(0x460B, &[0x00, 0x00])]) } else { [grpprl(&[(0x260A, &[0x00])]), grpprl(&[(0x460B, &[0x01, 0x00])])].concat() }
    };
    let spec = StructSpec {
        text,
        header_stories: vec![],
        sections: vec![(text.chars().count() as u32, Vec::new())],
        chpx_runs: vec![(0, Vec::new())],
        papx_entries: vec![(0, 0, listed(false)), (4, 0, listed(false)), (8, 0, listed(true))],
        plf_lst: plf_lst(0x1000, false, &levels),
        plf_lfo: plf_lfo(0x1000),
        note_stories: vec![],
        endnote_stories: vec![],
        extra_pairs: Vec::new(),
        data_stream: Vec::new(),
    };
    let doc = read(&struct_doc(&spec)).expect("opens");
    assert_eq!(doc.numbering.abstracts.len(), 1);
    assert_eq!(doc.numbering.nums.len(), 1);
    let l0 = doc.numbering.level(1, 0).expect("level 0");
    assert_eq!(l0.format, wordcraft_doc::section::NumFormat::Decimal);
    assert_eq!(l0.text, "%1.");
    assert_eq!(l0.indent, 36.0);
    assert_eq!(l0.hanging, 18.0);
    assert_eq!(l0.suffix, wordcraft_doc::numbering::LevelSuffix::Space);
    // Paragraphs 1-2 are in the list, paragraph 3 is not.
    let n = |i: usize| doc.body.get(i).and_then(|b| b.as_para()).and_then(|p| p.props.numbering);
    assert_eq!(n(0), Some(wordcraft_doc::props::NumRef { num: 1, level: 0 }));
    assert_eq!(n(1), Some(wordcraft_doc::props::NumRef { num: 1, level: 0 }));
    assert_eq!(n(2), None);
    // The numbering produces labels through the shared counter.
    let mut c = wordcraft_doc::numbering::Counters::default();
    assert_eq!(c.next_label(&doc.numbering, 1, 0).unwrap().0, "1.");
    assert_eq!(c.next_label(&doc.numbering, 1, 0).unwrap().0, "2.");
}

/// The combined D3 fixture: intro paragraph, table, list, two sections, header story.
fn structured_doc() -> Vec<u8> {
    let def2 = t_def_table(&[0, 2880, 5760]);
    let mut row: Vec<u8> = [grpprl(&[(0x2416, &[0x01]), (0x2417, &[0x01])]), def2].concat();
    let listed = [grpprl(&[(0x260A, &[0x00])]), grpprl(&[(0x460B, &[0x01, 0x00])])].concat();
    let text = "Intro\ra\u{7}b\u{7}first\rsecond\r";
    let levels: Vec<Vec<u8>> = (0..9).map(|_| lvl(1, 0x00, &[0x0000, 0x002E], true)).collect();
    let spec = StructSpec {
        text,
        header_stories: vec!["Running head\r\r"],
        sections: vec![
            (6, Vec::new()), // section 1: the intro paragraph; its header story is the odd header
            (
                text.chars().count() as u32,
                grpprl(&[(0xB01F, &15840u16.to_le_bytes()), (0xB020, &12240u16.to_le_bytes())]), // landscape letter
            ),
        ],
        chpx_runs: vec![(0, Vec::new())],
        papx_entries: vec![
            (0, 0, Vec::new()),                   // "Intro" paragraph, ends section 1
            (6, 0, grpprl(&[(0x2416, &[0x01])])), // "a" cell paragraph
            (8, 0, std::mem::take(&mut row)),     // "b" cell paragraph, ends the row
            (10, 0, listed.clone()),              // "first" paragraph, in the list
            (16, 0, listed),                      // "second" paragraph
        ],
        plf_lst: plf_lst(0x2000, false, &levels),
        plf_lfo: plf_lfo(0x2000),
        note_stories: vec![],
        endnote_stories: vec![],
        extra_pairs: Vec::new(),
        data_stream: Vec::new(),
    };
    struct_doc(&spec)
}

#[test]
fn combined_structure_document() {
    let doc = read(&structured_doc()).expect("opens");
    // Body shape: intro paragraph, table, two list paragraphs.
    assert!(doc.body.first().and_then(|b| b.as_para()).is_some_and(|p| p.text == "Intro"), "intro para");
    let tbl = doc.body.iter().find_map(|b| b.as_table()).expect("table");
    assert_eq!(tbl.rows.len(), 1);
    assert_eq!(tbl.rows[0].cells.len(), 2);
    let texts: Vec<String> = doc.plain_text(wordcraft_doc::StoryRef::Body).split('\n').map(str::to_string).collect();
    assert!(texts.contains(&"first".to_string()) && texts.contains(&"second".to_string()), "list text present: {texts:?}");
    // Section 1 is attached to the intro paragraph and owns the header story; the final
    // section is landscape.
    let sec1 = doc.body.first().and_then(|b| b.as_para()).and_then(|p| p.section.clone()).expect("section on intro");
    let h = sec1.headers.default.expect("header part");
    assert!(doc.parts.get(&h).is_some_and(|p| p.blocks.iter().any(|b| b.as_para().is_some_and(|p| p.text.contains("Running head")))));
    assert_eq!(doc.last_section.page_w, 792.0);
    assert_eq!(doc.last_section.page_h, 612.0);
    assert!(doc.last_section.landscape);
    // The whole document round-trips through JSON (the wcraft.json shape).
    let json = serde_json::to_value(&doc).expect("serialises");
    assert!(json.get("body").is_some_and(|b| b.as_array().is_some_and(|a| !a.is_empty())));
    assert!(json.get("numbering").is_some_and(|n| n.get("nums").is_some_and(|x| x.as_array().is_some_and(|a| !a.is_empty()))));
    assert!(json.get("lastSection").is_some_and(|s| s.get("landscape") == Some(&serde_json::json!(true))));
}

// ---------------------------------------------------------------------------------------------
// D4: notes, fields, bookmarks, images.

/// A PLC: aCP[n+1] u32s followed by `data` bytes per element.
fn plc(cps: &[u32], data: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    for c in cps {
        v.extend_from_slice(&c.to_le_bytes());
    }
    v.extend_from_slice(data);
    v
}

/// A SttbfBkmk with the given names.
fn sttbf_bkmk(names: &[&str]) -> Vec<u8> {
    let mut v = 0xFFFFu16.to_le_bytes().to_vec();
    v.extend_from_slice(&(names.len() as u16).to_le_bytes());
    v.extend_from_slice(&0u16.to_le_bytes()); // cbExtra
    for n in names {
        v.extend_from_slice(&(n.chars().count() as u16).to_le_bytes());
        for c in n.chars() {
            v.extend_from_slice(&(c as u16).to_le_bytes());
        }
    }
    v
}

#[test]
fn footnote_reference_links_story_part() {
    let text = "See\u{2} here\r";
    let ccp = text.chars().count() as u32;
    let spec = StructSpec {
        text,
        note_stories: vec!["Note body\r"],
        // Pair 2 (PlcffndRef): the 0x02 sits at CP 3, auto-numbered. Pair 3 (PlcffndTxt):
        // one story spanning the whole footnote subdocument.
        extra_pairs: vec![(2, plc(&[3, ccp], &1u16.to_le_bytes())), (3, plc(&[0, 10], &[]))],
        ..structured_spec(text, ccp)
    };
    let doc = read(&struct_doc(&spec)).expect("opens");
    let p0 = doc.body.first().and_then(|b| b.as_para()).expect("para");
    let note = p0.objects.first().expect("note ref object");
    let wordcraft_doc::para::InlineObject::NoteRef { kind, id, .. } = note else { panic!("not a note: {note:?}") };
    assert_eq!(*kind, wordcraft_doc::para::NoteKind::Footnote);
    let part = doc.parts.get(id).expect("note part");
    assert!(part.blocks.iter().any(|b| b.as_para().is_some_and(|p| p.text.contains("Note body"))), "note text: {:?}", part.blocks);
    assert_eq!(part.kind, wordcraft_doc::PartKind::Footnote);
}

#[test]
fn endnote_reference_links_story_part() {
    let text = "Ref\u{2}\r";
    let ccp = text.chars().count() as u32;
    let spec = StructSpec {
        text,
        // A footnote story keeps the endnote subdocument at a non-zero offset.
        note_stories: vec!["Ftn\r"],
        endnote_stories: vec!["Endnote body\r"],
        extra_pairs: vec![
            (46, plc(&[3, ccp], &1u16.to_le_bytes())),
            (47, plc(&[0, 13], &[])), // "Endnote body\r"
        ],
        ..structured_spec(text, ccp)
    };
    let doc = read(&struct_doc(&spec)).expect("opens");
    let p0 = doc.body.first().and_then(|b| b.as_para()).expect("para");
    let wordcraft_doc::para::InlineObject::NoteRef { kind, id, .. } = p0.objects.first().expect("note ref object") else { panic!("no note ref") };
    assert_eq!(*kind, wordcraft_doc::para::NoteKind::Endnote);
    let part = doc.parts.get(id).expect("endnote part");
    assert!(part.blocks.iter().any(|b| b.as_para().is_some_and(|p| p.text.contains("Endnote body"))));
}

#[test]
fn fields_and_hyperlinks() {
    // A HYPERLINK field (result kept as linked text) and a PAGE field (inline object).
    let text = "Go \u{13}HYPERLINK http://x\u{14}site\u{15} n\u{13}PAGE\u{14}3\u{15}.\r";
    let ccp = text.chars().count() as u32;
    let spec = structured_spec(text, ccp);
    let doc = read(&struct_doc(&spec)).expect("opens");
    let p0 = doc.body.first().and_then(|b| b.as_para()).expect("para");
    assert_eq!(p0.text, "Go site n\u{FFFC}.");
    // The hyperlink result run carries the target.
    let linked = p0.runs.iter().find(|r| r.props.link.is_some()).expect("linked run");
    assert_eq!(linked.props.link.as_deref(), Some("http://x"));
    // The PAGE field became one object with its cached result.
    let wordcraft_doc::para::InlineObject::Field { instr, result, .. } = p0.objects.first().expect("field object") else {
        panic!("no field: {:?}", p0.objects)
    };
    assert_eq!(instr, "PAGE");
    assert_eq!(result, "3");
}

#[test]
fn toc_field_result_becomes_paragraphs() {
    // "Contents\r" + field begin (\u{13}) " TOC \o "1-3" " sep (\u{14}) + "Foreword\t16\r"
    // + "Scope\t17\r" + end (\u{15}) + trailing paragraph: the cached entries are content.
    let raw = "Contents\r\u{13} TOC \\o \"1-3\" \u{14}Foreword\t16\rScope\t17\r\u{15}Tail\r";
    let f = text_doc(&[Piece { compressed: false, text: raw.into() }], &[], 0);
    let doc = read(&f).expect("opens");
    // The entries become real paragraphs (tab preserved), not one field result that
    // collapses onto the last of them.
    assert_eq!(body_texts(&doc), ["Contents", "\u{FFFC}Foreword\t16", "Scope\t17", "Tail"]);
    // One field anchor with an empty result sits on the paragraph that held the begin
    // character, the way the docx reader anchors a TOC on its heading paragraph.
    let p1 = doc.body.get(1).and_then(|b| b.as_para()).expect("entry para");
    assert_eq!(p1.objects.len(), 1, "one anchor for the whole TOC: {:?}", p1.objects);
    assert!(
        matches!(p1.objects.first(), Some(wordcraft_doc::para::InlineObject::Field { instr, result, .. }) if instr == "TOC \\o \"1-3\"" && result.is_empty()),
        "objects: {:?}",
        p1.objects
    );
}

#[test]
fn truncated_field_keeps_text() {
    // begin + instr + separator + "partial" with no end character: the walk finishes
    // the still-open field at the end of its range, so the collected characters are
    // not lost.
    let raw = "Head\r\u{13}PAGE\u{14}partial";
    let f = text_doc(&[Piece { compressed: false, text: raw.into() }], &[], 0);
    let doc = read(&f).expect("opens");
    assert_eq!(body_texts(&doc), ["Head", "\u{FFFC}"]);
    let p1 = doc.body.get(1).and_then(|b| b.as_para()).expect("field para");
    assert!(
        matches!(p1.objects.first(), Some(wordcraft_doc::para::InlineObject::Field { instr, result, .. }) if instr == "PAGE" && result == "partial"),
        "objects: {:?}",
        p1.objects
    );
}

#[test]
fn nested_field_in_toc_result_keeps_own_anchor() {
    // A TOC entry's page number is a nested PAGE field: with the TOC spilled, the
    // inner field keeps today's single-anchor behaviour, its finish_field pushing the
    // anchor straight into the entry paragraph.
    let raw = "Contents\r\u{13} TOC \\o \"1-3\" \u{14}Foreword\t\u{13}PAGE\u{14}16\u{15}\r\u{15}Tail\r";
    let f = text_doc(&[Piece { compressed: false, text: raw.into() }], &[], 0);
    let doc = read(&f).expect("opens");
    assert_eq!(body_texts(&doc), ["Contents", "\u{FFFC}Foreword\t\u{FFFC}", "Tail"]);
    let p1 = doc.body.get(1).and_then(|b| b.as_para()).expect("entry para");
    let objs = &p1.objects;
    assert_eq!(objs.len(), 2, "objects: {objs:?}");
    assert!(
        matches!(objs.first(), Some(wordcraft_doc::para::InlineObject::Field { instr, result, .. }) if instr == "TOC \\o \"1-3\"" && result.is_empty()),
        "objects: {objs:?}"
    );
    assert!(
        matches!(objs.get(1), Some(wordcraft_doc::para::InlineObject::Field { instr, result, .. }) if instr == "PAGE" && result == "16"),
        "objects: {objs:?}"
    );
}

#[test]
fn bookmarks_become_objects() {
    let text = "Hello world\r";
    let ccp = text.chars().count() as u32;
    let spec = StructSpec {
        text,
        extra_pairs: vec![(21, sttbf_bkmk(&["bm1"])), (22, plc(&[0, ccp], &[0, 0, 0, 0])), (23, plc(&[5, ccp], &[0, 0]))],
        ..structured_spec(text, ccp)
    };
    let doc = read(&struct_doc(&spec)).expect("opens");
    let p0 = doc.body.first().and_then(|b| b.as_para()).expect("para");
    // The start anchor sits before the text, the end after "Hello".
    assert!(
        matches!(p0.objects.first(), Some(wordcraft_doc::para::InlineObject::BookmarkStart { name }) if name == "bm1"),
        "objects: {:?}",
        p0.objects
    );
    assert!(matches!(p0.objects.get(1), Some(wordcraft_doc::para::InlineObject::BookmarkEnd { name }) if name == "bm1"), "objects: {:?}", p0.objects);
    assert_eq!(p0.text, "\u{FFFC}Hello\u{FFFC} world");
}

#[test]
fn inline_png_picture() {
    let text = "Pic\u{1}!\r";
    let ccp = text.chars().count() as u32;
    // A PICF header (68 bytes) with a 1×0.5-inch goal size, then PNG-ish bytes.
    let png: Vec<u8> = [b"\x89PNG\r\n\x1a\n".as_slice(), &[0u8; 16]].concat();
    let mut picf = vec![0u8; 0x44];
    let lcb = (0x44 + png.len()) as u32;
    picf[0..4].copy_from_slice(&lcb.to_le_bytes());
    picf[4..6].copy_from_slice(&0x44u16.to_le_bytes());
    picf[28..30].copy_from_slice(&1440i16.to_le_bytes()); // dxaGoal
    picf[30..32].copy_from_slice(&720i16.to_le_bytes()); // dyaGoal
    picf[32..34].copy_from_slice(&1000u16.to_le_bytes()); // mx = 100%
    picf[34..36].copy_from_slice(&1000u16.to_le_bytes()); // my = 100%
    let data: Vec<u8> = [picf, png.clone()].concat();
    let spec = StructSpec { text, data_stream: data, chpx_runs: vec![(0, grpprl(&[(0x6A03, &0u32.to_le_bytes())]))], ..structured_spec(text, ccp) };
    let doc = read(&struct_doc(&spec)).expect("opens");
    let p0 = doc.body.first().and_then(|b| b.as_para()).expect("para");
    assert_eq!(p0.text, "Pic\u{FFFC}!");
    let wordcraft_doc::para::InlineObject::Image { media, w, h, .. } = p0.objects.first().expect("image object") else {
        panic!("no image: {:?}", p0.objects)
    };
    assert_eq!((*w, *h), (72.0, 36.0)); // 1440 × 720 twips
    let bytes = doc.media.get(media).expect("media entry");
    assert_eq!(bytes.as_slice(), png.as_slice());
}

/// The D4 hostile-input fixture: notes, a field, a bookmark and a picture in one file.
fn extras_doc() -> Vec<u8> {
    let png: Vec<u8> = [b"\x89PNG\r\n\x1a\n".as_slice(), &[0u8; 8]].concat();
    let mut picf = vec![0u8; 0x44];
    let lcb = (0x44 + png.len()) as u32;
    picf[0..4].copy_from_slice(&lcb.to_le_bytes());
    picf[4..6].copy_from_slice(&0x44u16.to_le_bytes());
    picf[28..30].copy_from_slice(&720i16.to_le_bytes());
    picf[30..32].copy_from_slice(&720i16.to_le_bytes());
    picf[32..34].copy_from_slice(&1000u16.to_le_bytes());
    picf[34..36].copy_from_slice(&1000u16.to_le_bytes());
    let text: &'static str = "Box\u{2}\u{1}\u{13}PAGE\u{14}7\u{15}\r";
    let ccp = text.chars().count() as u32;
    let spec = StructSpec {
        text,
        note_stories: vec!["Note text\r"],
        extra_pairs: vec![
            (2, plc(&[3, ccp], &1u16.to_le_bytes())),
            (3, plc(&[0, 10], &[])),
            (21, sttbf_bkmk(&["bm"])),
            (22, plc(&[0, ccp], &[0, 0, 0, 0])),
            (23, plc(&[4, ccp], &[0, 0])),
        ],
        data_stream: [picf, png].concat(),
        chpx_runs: vec![(0, grpprl(&[(0x6A03, &0u32.to_le_bytes())]))],
        ..structured_spec(text, ccp)
    };
    struct_doc(&spec)
}

/// The shared base of the D4 fixtures: one Normal style, one mark, no sections extras.
fn structured_spec(text: &'static str, ccp: u32) -> StructSpec {
    StructSpec {
        text,
        header_stories: vec![],
        sections: vec![(ccp, Vec::new())],
        chpx_runs: vec![(0, Vec::new())],
        // The PAPX entry key is the CP just past the paragraph mark.
        papx_entries: vec![(0, 0, Vec::new())],
        plf_lst: Vec::new(),
        plf_lfo: Vec::new(),
        note_stories: vec![],
        endnote_stories: vec![],
        extra_pairs: Vec::new(),
        data_stream: Vec::new(),
    }
}

// ---------------------------------------------------------------------------------------------
// Hardening regressions: hostile CP ranges, piece lookups, picture fan-out, overflows.

/// All the paragraph text of a document: body plus every part.
fn all_text(doc: &wordcraft_doc::Document) -> String {
    let mut s: String = body_texts(doc).join("\n");
    for part in doc.parts.values() {
        for b in &part.blocks {
            if let Some(p) = b.as_para() {
                s.push('\n');
                s.push_str(&p.text);
            }
        }
    }
    s
}

#[test]
fn huge_story_ranges_are_clamped_to_their_subdocument() {
    // Footnote, header and endnote stories whose Table-stream CPs claim [0, 4e9) (and
    // ranges near u32::MAX that overflowed `base + cp`): before the clamp, each pushed
    // billions of U+FFFD (or panicked on the addition).
    let text = "A\u{2}\u{2}\r";
    let ccp = text.chars().count() as u32;
    let big = 4_000_000_000u32;
    let hdd = plc(&[0, 0, 0, 0, 0, 0, 0, 0, big, 0xFFFF_FFF0, 0xFFFF_FFFF, 0xFFFF_FFFF, 0xFFFF_FFFF], &[]);
    let spec = StructSpec {
        text,
        note_stories: vec!["Note body\r"],
        header_stories: vec!["Head\r"],
        endnote_stories: vec!["End\r"],
        extra_pairs: vec![
            (2, plc(&[1, ccp], &1u16.to_le_bytes())),
            (3, plc(&[0, big], &[])),
            (11, hdd),
            (46, plc(&[2, ccp], &1u16.to_le_bytes())),
            (47, plc(&[0xFFFF_FFF0, 0xFFFF_FFFF], &[])),
        ],
        ..structured_spec(text, ccp)
    };
    let doc = read(&struct_doc(&spec)).expect("opens");
    let all = all_text(&doc);
    assert!(!all.contains('\u{FFFD}'), "replacement characters: {} bytes of text", all.len());
    assert!(all.len() < 200, "text grew to {} bytes", all.len());
    let header = doc.last_section.headers.default.and_then(|id| doc.parts.get(&id)).expect("header part");
    assert!(header.blocks.iter().any(|b| b.as_para().is_some_and(|p| p.text == "Head")), "header: {:?}", header.blocks);
    let footnote = doc.parts.values().find(|p| p.kind == wordcraft_doc::PartKind::Footnote).expect("footnote part");
    assert!(footnote.blocks.iter().any(|b| b.as_para().is_some_and(|p| p.text == "Note body")));
    // The endnote story starts past the endnote subdocument: nothing to read.
    assert!(!doc.parts.values().any(|p| p.kind == wordcraft_doc::PartKind::Endnote));
}

/// A document whose piece table is given raw (cps + one Pcd per piece), with `text` bytes
/// at TEXT_FC and the FIB claiming `ccp_text` main-document characters.
fn raw_piece_doc(cps: &[u32], fcs: &[u32], text: &[u8], ccp_text: u32) -> Vec<u8> {
    raw_piece_doc_with(cps, fcs, text, ccp_text, &[])
}

/// [`raw_piece_doc`] with extra raw FIB (pair, fc, lcb) entries.
fn raw_piece_doc_with(cps: &[u32], fcs: &[u32], text: &[u8], ccp_text: u32, extra: &[(usize, u32, u32)]) -> Vec<u8> {
    let mut plc = Vec::new();
    for c in cps {
        plc.extend_from_slice(&c.to_le_bytes());
    }
    for fc in fcs {
        plc.extend_from_slice(&[0, 0]);
        plc.extend_from_slice(&fc.to_le_bytes());
        plc.extend_from_slice(&0u16.to_le_bytes());
    }
    let mut clx = vec![0x02u8];
    clx.extend_from_slice(&(plc.len() as u32).to_le_bytes());
    clx.extend_from_slice(&plc);
    let mut table = vec![0u8; CLX_AT as usize];
    table.extend_from_slice(&clx);
    let mut word = vec![0u8; TEXT_FC as usize];
    word.extend_from_slice(text);
    let mut pairs = vec![(33, CLX_AT, clx.len() as u32)];
    pairs.extend_from_slice(extra);
    let spec = FibSpec { ccp_text, pairs, ..Default::default() };
    let fib = fib_bytes(&spec);
    word[..fib.len()].copy_from_slice(&fib);
    cfb_file(&[("WordDocument", &word), ("0Table", &table)])
}

#[test]
fn piece_claiming_far_more_text_than_stored_is_skipped() {
    // One compressed piece claims CPs [0, 100M) but only "Hi\r" is stored: the walk used to
    // emit a U+FFFD per missing CP (100M of them).
    let fc = (2 * TEXT_FC) | 0x4000_0000;
    let f = raw_piece_doc(&[0, 100_000_000], &[fc], b"Hi\r", 100_000_000);
    let doc = read(&f).expect("opens");
    assert_eq!(body_texts(&doc), vec!["Hi".to_string()]);
}

#[test]
fn walk_stops_at_the_last_piece() {
    // The FIB claims 100M characters but the piece table ends at CP 3.
    let fc = (2 * TEXT_FC) | 0x4000_0000;
    let f = raw_piece_doc(&[0, 3], &[fc], b"Hi\r", 100_000_000);
    let doc = read(&f).expect("opens");
    assert_eq!(body_texts(&doc), vec!["Hi".to_string()]);
}

#[test]
fn inverted_and_gapped_pieces_are_skipped() {
    // Piece 0 is inverted (CPs 5 → 2), piece 1 reads "ok\r", piece 2 points past the stream.
    let base = (2 * TEXT_FC) | 0x4000_0000;
    let f = raw_piece_doc(&[5, 2, 5, 50_000_000], &[base, base, 0x3FFF_0000], b"ok\r", 50_000_000);
    let doc = read(&f).expect("opens");
    let all = all_text(&doc);
    assert!(!all.contains('\u{FFFD}'));
    assert!(all.len() < 20, "{all:?}");
}

#[test]
fn many_pieces_walk_in_n_log_n() {
    // 200k one-character pieces: the old per-character scan over every piece was
    // O(chars × pieces) = 4e10 steps; with binary search this opens instantly.
    let n = 200_000usize;
    let mut text = vec![b'x'; n - 1];
    text.push(b'\r');
    let cps: Vec<u32> = (0..=n as u32).collect();
    let fcs: Vec<u32> = (0..n as u32).map(|i| (2 * (TEXT_FC + i)) | 0x4000_0000).collect();
    let started = std::time::Instant::now();
    let doc = read(&raw_piece_doc(&cps, &fcs, &text, n as u32)).expect("opens");
    let body = body_texts(&doc);
    assert_eq!(body.len(), 1);
    assert_eq!(body[0].len(), n - 1);
    assert!(started.elapsed() < std::time::Duration::from_secs(30), "took {:?}", started.elapsed());
}

#[test]
fn surrogate_pairs_decode_across_one_cp_each() {
    // `Piece` stores one u16 per char, so write the pair by hand.
    let units: Vec<u16> = "a\u{1F600}b\r".encode_utf16().collect();
    let bytes: Vec<u8> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
    let n = units.len() as u32;
    let doc = read(&raw_piece_doc(&[0, n], &[TEXT_FC], &bytes, n)).expect("opens");
    assert_eq!(body_texts(&doc), vec!["a\u{1F600}b".to_string()]);
}

/// A PICF header (68 bytes) for an `n`-byte PNG-looking payload.
fn picf_with_png(n: usize) -> Vec<u8> {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.resize(n.max(8), 0xAB);
    let mut picf = vec![0u8; 0x44];
    picf[0..4].copy_from_slice(&((0x44 + png.len()) as u32).to_le_bytes());
    picf[4..6].copy_from_slice(&0x44u16.to_le_bytes());
    picf[28..30].copy_from_slice(&1440i16.to_le_bytes());
    picf[30..32].copy_from_slice(&1440i16.to_le_bytes());
    picf.extend_from_slice(&png);
    picf
}

#[test]
fn anchors_sharing_one_picture_share_one_media_entry() {
    // 5000 anchors on the same 16 MB picture used to copy it once per anchor (80 GB)
    // before the copies were deduplicated.
    let text: &'static str = Box::leak(format!("{}\r", "\u{1}".repeat(5000)).into_boxed_str());
    let ccp = text.chars().count() as u32;
    let spec = StructSpec {
        text,
        data_stream: picf_with_png(16 << 20),
        chpx_runs: vec![(0, grpprl(&[(0x6A03, &0u32.to_le_bytes())]))],
        ..structured_spec(text, ccp)
    };
    let doc = read(&struct_doc(&spec)).expect("opens");
    assert_eq!(doc.media.len(), 1);
    let p0 = doc.body.first().and_then(|b| b.as_para()).expect("para");
    assert_eq!(p0.objects.len(), 5000);
    assert!(p0.objects.iter().all(|o| matches!(o, wordcraft_doc::para::InlineObject::Image { media, .. } if media == "image1.png")));
}

#[test]
fn picture_bytes_are_capped_per_document() {
    let a = picf_with_png(600);
    let b = picf_with_png(600);
    let data: Vec<u8> = [a.clone(), b].concat();
    let mut pics = crate::media::Pictures::default();
    pics.limit = 1000;
    let first = pics.get(&data, 0).expect("first fits");
    // Same location again: no new bytes.
    assert_eq!(pics.get(&data, 0).map(|p| p.key), Some(first.key.clone()));
    // A second distinct picture would exceed the cap.
    assert!(pics.get(&data, a.len() as u32).is_none());
    assert_eq!(pics.media.len(), 1);
    const { assert!(crate::media::MAX_MEDIA_BYTES <= 256 << 20) };
}

#[test]
fn row_height_of_i16_min_does_not_overflow() {
    // sprmTDyaRowHeight = -32768: negating the i16 overflowed (a debug-build panic).
    let papx = grpprl(&[(0x9407, &i16::MIN.to_le_bytes())]);
    let row = crate::table::decode(&papx);
    assert_eq!(row.height(), Some(1584.0));
}

#[test]
fn list_and_lfo_offsets_near_u32_max_are_safe() {
    // PlfLst / PlfLfo at fc = u32::MAX: offset sums like `at + 2 + cLst * 28` must saturate
    // (they overflow on 32-bit targets such as wasm32).
    let fc = (2 * TEXT_FC) | 0x4000_0000;
    let far = u32::MAX - 1;
    let f = raw_piece_doc_with(&[0, 2], &[fc], b"a\r", 2, &[(73, far, 2), (74, far, 4)]);
    let doc = read(&f).expect("opens");
    assert!(doc.numbering.nums.is_empty());
}

/// A StructSpec with one section (`sepx`) over `text`, no lists, notes or extras.
fn one_section(text: &'static str, sepx: Vec<u8>, papx_entries: Vec<(u32, u16, Vec<u8>)>) -> StructSpec {
    StructSpec {
        text,
        header_stories: vec![],
        sections: vec![(text.chars().count() as u32, sepx)],
        chpx_runs: vec![(0, Vec::new())],
        papx_entries,
        plf_lst: Vec::new(),
        plf_lfo: Vec::new(),
        note_stories: vec![],
        endnote_stories: vec![],
        extra_pairs: Vec::new(),
        data_stream: Vec::new(),
    }
}

#[test]
fn piece_prm_character_formatting_applied() {
    // #458. Bit 0 of a Prm is fComplex ([MS-DOC] §2.9.215/§2.9.216): Prm0 = isprm in bits
    // 1-7 plus a one-byte operand in bits 8-15 (isprm 0x55 is sprmCFBold); Prm1 = a
    // zero-based Prc index in bits 1-15. Piece properties apply after the CHPX.
    let bold = grpprl(&[(0x0835, &[0x01])]);
    let prcs = vec![bold.clone(), [grpprl(&[(0x0836, &[0x01])]), bold.clone()].concat()];
    let run = |prm: u16, chpx: Vec<u8>| {
        let spec = RichSpec {
            text: "Bold\r",
            styles: vec![(1, 0x0FFF, "Normal", Vec::new(), Vec::new())],
            fonts: vec!["Times New Roman"],
            chpx_runs: vec![(0, chpx)],
            papx_entries: vec![(0, 0, Vec::new())],
        };
        let doc = read(&rich_doc_with(&spec, &RichExtra { prcs: prcs.clone(), prm, ..Default::default() })).expect("opens");
        let p = para0(&doc);
        assert_eq!(p.text, "Bold");
        p.runs.first().map(|r| (r.props.bold, r.props.italic)).unwrap_or_default()
    };
    assert_eq!(run(0x01AA, Vec::new()), (Some(true), None), "Prm0 bold on");
    assert_eq!(run(0x0001, Vec::new()), (Some(true), None), "Prm1 → Prc 0 (bold)");
    assert_eq!(run(0x0003, Vec::new()), (Some(true), Some(true)), "Prm1 → Prc 1 (italic + bold)");
    assert_eq!(run(0x00AA, bold.clone()).0, Some(false), "Prm0 bold off overrides the CHPX");
    assert_eq!(run(0x0000, Vec::new()), (None, None), "no Prm: plain");
    assert_eq!(run(0x0000, bold).0, Some(true), "CHPX bold alone");
}

#[test]
fn section_break_kinds_imported() {
    // #459. SBkcOperand: 0 continuous, 1 new column, 2 new page, 3 even page, 4 odd page.
    use wordcraft_doc::section::SectionStart;
    let kinds = [SectionStart::Continuous, SectionStart::NextColumn, SectionStart::NextPage, SectionStart::EvenPage, SectionStart::OddPage];
    for (bkc, want) in kinds.into_iter().enumerate() {
        let spec = one_section("Text\r", grpprl(&[(0x3009, &[bkc as u8])]), vec![(0, 0, Vec::new())]);
        let doc = read(&struct_doc(&spec)).expect("opens");
        assert_eq!(doc.last_section.start, want, "sprmSBkc {bkc}");
    }
}

#[test]
fn explicit_section_orientation_imported() {
    // #460. SBOrientationOperand ([MS-DOC] §2.9.236): 1 portrait, 2 landscape.
    let page = |w: u16, h: u16, orient: Option<u8>| {
        let mut prls: Vec<(u16, &[u8])> = Vec::new();
        let (w, h) = (w.to_le_bytes(), h.to_le_bytes());
        let o = orient.map(|o| [o]);
        if let Some(o) = &o {
            prls.push((0x301D, o));
        }
        prls.push((0xB01F, &w));
        prls.push((0xB020, &h));
        let doc = read(&struct_doc(&one_section("Text\r", grpprl(&prls), vec![(0, 0, Vec::new())]))).expect("opens");
        (doc.last_section.landscape, doc.last_section.page_w, doc.last_section.page_h)
    };
    assert_eq!(page(12240, 15840, Some(1)), (false, 612.0, 792.0), "explicit portrait");
    assert_eq!(page(15840, 12240, Some(2)), (true, 792.0, 612.0), "explicit landscape");
    assert_eq!(page(12240, 15840, None), (false, 612.0, 792.0), "inferred portrait");
    assert_eq!(page(15840, 12240, None), (true, 792.0, 612.0), "inferred landscape");
}

#[test]
fn negative_list_reference_keeps_list_membership() {
    // #461. sprmPIlfo 0xF802-0xFFFF is the negation of a 1-based LFO index whose paragraph
    // keeps its own indents ([MS-DOC] §2.6.2); 0 and 0xF801 mean "not in a list".
    let levels: Vec<Vec<u8>> = (0..9).map(|_| lvl(1, 0x00, &[0x0000, 0x002E], true)).collect();
    let listed = |ilfo: u16| [grpprl(&[(0x260A, &[0x00]), (0x840F, &720u16.to_le_bytes())]), grpprl(&[(0x460B, &ilfo.to_le_bytes())])].concat();
    let text = "pos\rneg\rzero\rnone\r";
    let mut spec = one_section(text, Vec::new(), vec![(0, 0, listed(0x0001)), (4, 0, listed(0xFFFF)), (8, 0, listed(0)), (13, 0, listed(0xF801))]);
    spec.plf_lst = plf_lst(0x1000, true, &levels[..1]);
    spec.plf_lfo = plf_lfo(0x1000);
    let doc = read(&struct_doc(&spec)).expect("opens");
    let para = |i: usize| doc.body.get(i).and_then(|b| b.as_para()).map(|p| (p.props.numbering, p.props.indent_left)).unwrap_or_default();
    let one = Some(wordcraft_doc::props::NumRef { num: 1, level: 0 });
    assert_eq!(para(0), (one, Some(36.0)), "0x0001");
    assert_eq!(para(1), (one, Some(36.0)), "0xFFFF = -1");
    assert_eq!(para(2).0, None, "0");
    assert_eq!(para(3).0, None, "0xF801");
}

#[test]
fn style_inherits_its_declared_base() {
    // #462. StdfBase ([MS-DOC] §2.9.260): stk + istdBase share the second word, cupx +
    // istdNext the third; istdBase is zero-based. The next style must not affect the base.
    for next in [0u16, 1, 2, 0x0FFF] {
        let spec = RichSpec {
            text: "Text\r",
            styles: vec![
                (1, 0x0FFF, "Normal", Vec::new(), grpprl(&[(0x0835, &[0x01])])), // bold
                (1, 0, "Derived", Vec::new(), Vec::new()),
                (1, 1, "Grandchild", Vec::new(), grpprl(&[(0x0836, &[0x01])])), // italic
                // A corrupt cycle must neither hang nor crash.
                (1, 4, "Loop A", Vec::new(), Vec::new()),
                (1, 3, "Loop B", Vec::new(), Vec::new()),
            ],
            fonts: vec!["Times New Roman"],
            chpx_runs: vec![(0, Vec::new())],
            papx_entries: vec![(0, 1, Vec::new())],
        };
        let extra = RichExtra { style_next: vec![0x0FFF, next, 0x0FFF, 0x0FFF, 0x0FFF], ..Default::default() };
        let doc = read(&rich_doc_with(&spec, &extra)).expect("opens");
        let style = |name: &str| doc.styles.styles.iter().find(|s| s.name == name).cloned().expect("style");
        let derived = style("Derived");
        assert_eq!(derived.based_on.as_deref(), Some("Normal"), "next {next}");
        assert_eq!(derived.chr.bold, Some(true), "next {next}");
        let grand = style("Grandchild");
        assert_eq!(grand.based_on.as_deref(), Some("Derived"));
        assert_eq!((grand.chr.bold, grand.chr.italic), (Some(true), Some(true)), "two-level chain");
        assert_eq!(style("Normal").based_on, None);
        assert_eq!(para0(&doc).props.style.as_deref(), Some("Derived"));
    }
}

#[test]
fn alternate_font_name_keeps_later_entries() {
    // #463. An STTB entry's length is its cchData ([MS-DOC] §2.2.4), which covers the FFN's
    // optional alternate name (§2.9.82 xszAlt); the next entry starts after it.
    for alt in [None, Some("ArialMT")] {
        let spec = RichSpec {
            text: "ab\r",
            styles: vec![(1, 0x0FFF, "Normal", Vec::new(), Vec::new())],
            fonts: vec!["Arial", "Courier New"],
            chpx_runs: vec![(0, grpprl(&[(0x4A4F, &0u16.to_le_bytes())])), (1, grpprl(&[(0x4A4F, &1u16.to_le_bytes())]))],
            papx_entries: vec![(0, 0, Vec::new())],
        };
        let doc = read(&rich_doc_with(&spec, &RichExtra { alt_fonts: vec![alt, None], ..Default::default() })).expect("opens");
        let fonts: Vec<Option<String>> = para0(&doc).runs.iter().map(|r| r.props.font.clone()).collect();
        assert_eq!(fonts, [Some("Arial".to_string()), Some("Courier New".to_string())], "alt {alt:?}");
    }
}

//! Legacy Word binary documents (`.doc`, Word 97–2003), import only.
//!
//! Written from the published specifications: [MS-CFB] for the compound file container and
//! [MS-DOC] for the `WordDocument` and table streams. The reader takes the main document text
//! through the piece table, character and paragraph formatting from the formatted disk pages and
//! the style sheet, lists, tables, hyperlink fields, page breaks and the page setup of the last
//! section. Headers, footers, footnotes, comments, text boxes and pictures are left out.
//!
//! Every offset and count in the file is hostile: all reads are bounds-checked, chains and
//! recursion are bounded and nothing here panics.

use std::collections::HashMap;

use wordcraft_doc::{Align, Document, Rgb};

use crate::model::{self, Cell, FBlock, FTable, Flow, Fmt, Kind, ListInfo, Para};

/// The compound file signature every `.doc` starts with.
pub const CFB_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

/// Characters of main text read at most (hostile counts).
const MAX_CHARS: usize = 64 << 20;
/// Directory entries read at most.
const MAX_DIR: usize = 1 << 16;
/// Style inheritance depth followed.
const MAX_STYLE_DEPTH: usize = 16;
/// Nested fields tracked.
const MAX_FIELDS: usize = 64;
/// Bytes of one style's property list read (real ones are a few dozen bytes).
const MAX_STYLE_PROPS: usize = 4096;
/// Sections whose properties are read.
const MAX_SECTIONS: usize = 4096;
/// Bytes of oversized paragraph property lists (in the Data stream) read in all.
const MAX_HUGE_PROPS: usize = 64 << 20;
/// Cells kept per table row.
const MAX_CELLS: usize = 63;

/// Does `bytes` start like a compound file (legacy `.doc`, or an encrypted OOXML package)?
pub fn is_cfb(bytes: &[u8]) -> bool {
    bytes.starts_with(&CFB_MAGIC)
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    let s = b.get(o..o.checked_add(2)?)?;
    Some(u16::from_le_bytes([*s.first()?, *s.get(1)?]))
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    let s = b.get(o..o.checked_add(4)?)?;
    Some(u32::from_le_bytes([*s.first()?, *s.get(1)?, *s.get(2)?, *s.get(3)?]))
}

fn sub(b: &[u8], o: usize, len: usize) -> Option<&[u8]> {
    b.get(o..o.checked_add(len)?)
}

// ---------------------------------------------------------------------------------------------
// Compound file ([MS-CFB])

/// Sector numbers at and above this are markers (free, end of chain, FAT, DIFAT).
const MAX_REG_SECT: u32 = 0xFFFF_FFFA;
const NO_STREAM: u32 = 0xFFFF_FFFF;

struct Entry {
    name: String,
    left: u32,
    right: u32,
    child: u32,
    start: u32,
    size: u64,
}

struct Cfb<'a> {
    data: &'a [u8],
    sector: usize,
    fat: Vec<u32>,
    minifat: Vec<u32>,
    mini: Vec<u8>,
    dir: Vec<Entry>,
    cutoff: u64,
}

impl<'a> Cfb<'a> {
    fn open(data: &'a [u8]) -> Result<Cfb<'a>, String> {
        if !is_cfb(data) {
            return Err("not a Word document".into());
        }
        let bad = || "damaged Word document (compound file header)".to_string();
        let major = u16_at(data, 26).ok_or_else(bad)?;
        let shift = u16_at(data, 30).ok_or_else(bad)?;
        if shift != 9 && shift != 12 {
            return Err(bad());
        }
        let sector = 1usize << shift;
        let per = sector / 4;
        let max_sectors = data.len() / sector + 1;
        let mut cfb = Cfb { data, sector, fat: Vec::new(), minifat: Vec::new(), mini: Vec::new(), dir: Vec::new(), cutoff: 4096 };

        // The FAT's own sectors: 109 in the header, the rest in the DIFAT chain.
        let mut fat_sectors = Vec::new();
        for i in 0..109 {
            let s = u32_at(data, 76 + i * 4).ok_or_else(bad)?;
            if s < MAX_REG_SECT {
                fat_sectors.push(s);
            }
        }
        let mut difat = u32_at(data, 68).ok_or_else(bad)?;
        let mut hops = 0;
        while difat < MAX_REG_SECT && hops < max_sectors {
            let Some(sec) = cfb.sector_bytes(difat) else { break };
            for i in 0..per.saturating_sub(1) {
                if let Some(s) = u32_at(sec, i * 4)
                    && s < MAX_REG_SECT
                {
                    fat_sectors.push(s);
                }
            }
            difat = u32_at(sec, (per.saturating_sub(1)) * 4).unwrap_or(NO_STREAM);
            hops += 1;
        }
        fat_sectors.truncate(max_sectors);
        let mut fat = Vec::with_capacity(fat_sectors.len().saturating_mul(per).min(max_sectors.saturating_mul(2)));
        for s in fat_sectors {
            let Some(sec) = cfb.sector_bytes(s) else { continue };
            for i in 0..per {
                fat.push(u32_at(sec, i * 4).unwrap_or(NO_STREAM));
            }
        }
        cfb.fat = fat;

        let dir_bytes = cfb.chain_bytes(u32_at(data, 48).ok_or_else(bad)?, u64::MAX);
        for e in dir_bytes.as_chunks::<128>().0.iter().take(MAX_DIR) {
            let name_len = (u16_at(e, 64).unwrap_or(0) as usize).clamp(2, 64) - 2;
            let units: Vec<u16> = e.get(..name_len).unwrap_or(&[]).as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
            let size = u32_at(e, 120).unwrap_or(0) as u64 | if major >= 4 { (u32_at(e, 124).unwrap_or(0) as u64) << 32 } else { 0 };
            cfb.dir.push(Entry {
                name: String::from_utf16_lossy(&units),
                left: u32_at(e, 68).unwrap_or(NO_STREAM),
                right: u32_at(e, 72).unwrap_or(NO_STREAM),
                child: u32_at(e, 76).unwrap_or(NO_STREAM),
                start: u32_at(e, 116).unwrap_or(NO_STREAM),
                size,
            });
        }
        cfb.cutoff = u32_at(data, 56).unwrap_or(4096) as u64;
        let minifat = cfb.chain_bytes(u32_at(data, 60).unwrap_or(NO_STREAM), u64::MAX);
        cfb.minifat = minifat.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect();
        if let Some(root) = cfb.dir.first() {
            cfb.mini = cfb.chain_bytes(root.start, root.size);
        }
        Ok(cfb)
    }

    /// The bytes of sector `n` (the last one may be short).
    fn sector_bytes(&self, n: u32) -> Option<&'a [u8]> {
        let start = (n as usize).checked_add(1)?.checked_mul(self.sector)?;
        let end = start.checked_add(self.sector)?.min(self.data.len());
        self.data.get(start..end).filter(|s| !s.is_empty())
    }

    /// A stream stored in the regular sectors, at most `size` bytes.
    fn chain_bytes(&self, start: u32, size: u64) -> Vec<u8> {
        let cap = size.min(self.data.len() as u64) as usize;
        let mut out = Vec::with_capacity(cap.min(1 << 24));
        let mut s = start;
        let mut hops = 0;
        while s < MAX_REG_SECT && out.len() < cap && hops <= self.fat.len() {
            let Some(sec) = self.sector_bytes(s) else { break };
            out.extend_from_slice(sec);
            s = self.fat.get(s as usize).copied().unwrap_or(NO_STREAM);
            hops += 1;
        }
        out.truncate(cap);
        out
    }

    /// A stream stored in the mini stream (64-byte sectors).
    fn mini_bytes(&self, start: u32, size: u64) -> Vec<u8> {
        let cap = size.min(self.mini.len() as u64) as usize;
        let mut out = Vec::with_capacity(cap);
        let mut s = start;
        let mut hops = 0;
        while s < MAX_REG_SECT && out.len() < cap && hops <= self.minifat.len() {
            let Some(sec) = (s as usize).checked_mul(64).and_then(|o| self.mini.get(o..o.saturating_add(64).min(self.mini.len()))) else { break };
            out.extend_from_slice(sec);
            s = self.minifat.get(s as usize).copied().unwrap_or(NO_STREAM);
            hops += 1;
        }
        out.truncate(cap);
        out
    }

    /// The top-level stream called `name` (embedded objects carry streams of the same names in
    /// storages of their own, so only the root's children are searched).
    fn stream(&self, name: &str) -> Option<Vec<u8>> {
        let mut stack = vec![self.dir.first()?.child];
        let mut seen = 0;
        while let Some(id) = stack.pop() {
            seen += 1;
            if seen > self.dir.len() + 1 {
                break;
            }
            let Some(e) = self.dir.get(id as usize) else { continue };
            if e.name.eq_ignore_ascii_case(name) {
                return Some(if e.size < self.cutoff { self.mini_bytes(e.start, e.size) } else { self.chain_bytes(e.start, e.size) });
            }
            stack.push(e.left);
            stack.push(e.right);
        }
        None
    }
}

// ---------------------------------------------------------------------------------------------
// Property modifiers ([MS-DOC] 2.6)

/// The `(sprm, operand)` pairs of a `grpprl`. Operands of variable-length sprms start with their
/// length prefix.
fn sprms(grpprl: &[u8]) -> impl Iterator<Item = (u16, &[u8])> {
    let mut at = 0usize;
    std::iter::from_fn(move || {
        let op = u16_at(grpprl, at)?;
        let o = at + 2;
        let len = match op >> 13 {
            0 | 1 => 1,
            2 | 4 | 5 => 2,
            3 => 4,
            7 => 3,
            _ => match op {
                // sprmTDefTable: a two-byte count, one more than the bytes that follow it.
                0xD608 => (u16_at(grpprl, o)? as usize).checked_add(1)?,
                // sprmPChgTabs: 255 means "work it out from the two tab lists".
                0xC615 if grpprl.get(o) == Some(&255) => {
                    let del = *grpprl.get(o + 1)? as usize;
                    let add = *grpprl.get(o + 2 + del * 4)? as usize;
                    3 + del * 4 + add * 3
                }
                _ => 1 + *grpprl.get(o)? as usize,
            },
        };
        let operand = sub(grpprl, o, len)?;
        at = o + len;
        Some((op, operand))
    })
}

/// Character properties, as far as the flow model carries them.
#[derive(Clone, PartialEq)]
struct Chp {
    bold: bool,
    italic: bool,
    strike: bool,
    underline: bool,
    hidden: bool,
    deleted: bool,
    /// Half-points.
    hps: u16,
    color: Option<Rgb>,
    highlight: Option<Rgb>,
    /// 1 = superscript, 2 = subscript.
    iss: u8,
    /// Font indexes: Latin text, East Asian text.
    ftc: u16,
    ftc_fe: u16,
}

impl Default for Chp {
    fn default() -> Self {
        Chp {
            bold: false,
            italic: false,
            strike: false,
            underline: false,
            hidden: false,
            deleted: false,
            hps: 20,
            color: None,
            highlight: None,
            iss: 0,
            ftc: 0,
            ftc_fe: 0,
        }
    }
}

/// The 16-colour palette of `Ico` values (0 = automatic).
fn ico(i: u8) -> Option<Rgb> {
    Some(match i {
        1 => Rgb(0, 0, 0),
        2 => Rgb(0, 0, 255),
        3 => Rgb(0, 255, 255),
        4 => Rgb(0, 255, 0),
        5 => Rgb(255, 0, 255),
        6 => Rgb(255, 0, 0),
        7 => Rgb(255, 255, 0),
        8 => Rgb(255, 255, 255),
        9 => Rgb(0, 0, 128),
        10 => Rgb(0, 128, 128),
        11 => Rgb(0, 128, 0),
        12 => Rgb(128, 0, 128),
        13 => Rgb(128, 0, 0),
        14 => Rgb(128, 128, 0),
        15 => Rgb(128, 128, 128),
        16 => Rgb(192, 192, 192),
        _ => return None,
    })
}

/// A toggle operand: off, on, "as the style", "the opposite of the style".
fn toggle(v: u8, cur: bool, style: bool) -> bool {
    match v {
        0 => false,
        1 => true,
        0x80 => style,
        0x81 => !style,
        _ => cur,
    }
}

/// Paragraph properties, as far as the flow model carries them.
#[derive(Clone, Default)]
struct Pap {
    istd: u16,
    jc: u8,
    in_table: bool,
    ttp: bool,
    inner_cell: bool,
    inner_ttp: bool,
    itap: u32,
    ilfo: u16,
    ilvl: u8,
    page_break_before: bool,
    /// The operand of the row's sprmTDefTable (row-end paragraphs).
    tdef: Option<Vec<u8>>,
}

struct Std {
    sti: u16,
    base: u16,
    papx: Vec<u8>,
    chpx: Vec<u8>,
}

/// The style sheet, with what each style comes to worked out once (documents have thousands of
/// paragraphs per style).
#[derive(Default)]
struct Styles {
    stds: Vec<Option<Std>>,
    ftc: u16,
    ftc_fe: u16,
    /// Per style: its character and paragraph properties, base styles applied.
    chps: Vec<Chp>,
    paps: Vec<Pap>,
    /// Per style: the character property lists of its base styles and its own, in a row.
    char_chains: Vec<Vec<u8>>,
}

impl Styles {
    /// The style sheet ([MS-DOC] 2.9.271 STSH).
    fn parse(b: &[u8]) -> Styles {
        let mut st = Styles::default();
        let Some(cb_stshi) = u16_at(b, 0).map(usize::from) else { return st };
        let Some(stshi) = sub(b, 2, cb_stshi) else { return st };
        let cstd = u16_at(stshi, 0).unwrap_or(0) as usize;
        let base_len = (u16_at(stshi, 2).unwrap_or(10) as usize).max(10);
        st.ftc = u16_at(stshi, 12).unwrap_or(0);
        st.ftc_fe = u16_at(stshi, 14).unwrap_or(0);
        let mut at = 2 + cb_stshi;
        for _ in 0..cstd.min(4096) {
            let Some(cb) = u16_at(b, at).map(usize::from) else { break };
            let Some(std) = sub(b, at + 2, cb) else { break };
            at += 2 + cb;
            st.stds.push(Self::std(std, base_len));
        }
        let all = 0..st.stds.len() as u16;
        st.char_chains = all
            .clone()
            .map(|i| {
                let mut chain: Vec<u8> = st.chain(i).iter().flat_map(|s| s.chpx.iter().copied()).collect();
                chain.truncate(MAX_STYLE_PROPS);
                chain
            })
            .collect();
        st.chps = all.clone().map(|i| st.resolve_chp(i)).collect();
        st.paps = all.map(|i| st.resolve_pap(i)).collect();
        st
    }

    fn std(s: &[u8], base_len: usize) -> Option<Std> {
        let sti = u16_at(s, 0)? & 0x0FFF;
        let w1 = u16_at(s, 2)?;
        let (stk, base) = (w1 & 0xF, w1 >> 4);
        // The name: a length-prefixed, null-terminated UTF-16 string.
        let cch = u16_at(s, base_len)? as usize;
        let even = |o: usize| o + (o & 1);
        let mut at = even(base_len + 2 + cch * 2 + 2);
        let mut std = Std { sti, base, papx: Vec::new(), chpx: Vec::new() };
        if stk == 1 {
            let cb = u16_at(s, at).unwrap_or(0) as usize;
            // The paragraph UPX starts with the style's own index.
            std.papx = sub(s, at + 4, cb.saturating_sub(2).min(MAX_STYLE_PROPS)).unwrap_or(&[]).to_vec();
            at = even(at + 2 + cb);
        }
        if stk == 1 || stk == 2 {
            let cb = u16_at(s, at).unwrap_or(0) as usize;
            std.chpx = sub(s, at + 2, cb.min(MAX_STYLE_PROPS)).unwrap_or(&[]).to_vec();
        }
        Some(std)
    }

    fn get(&self, istd: u16) -> Option<&Std> {
        self.stds.get(istd as usize)?.as_ref()
    }

    /// `istd` and the styles it is based on, base first.
    fn chain(&self, istd: u16) -> Vec<&Std> {
        let mut v = Vec::new();
        let mut cur = istd;
        while v.len() < MAX_STYLE_DEPTH
            && let Some(s) = self.get(cur)
        {
            v.push(s);
            if s.base == cur {
                break;
            }
            cur = s.base;
        }
        v.reverse();
        v
    }

    /// The character properties style `istd` gives.
    fn chp(&self, istd: u16) -> Chp {
        self.chps.get(istd as usize).cloned().unwrap_or_else(|| Chp { ftc: self.ftc, ftc_fe: self.ftc_fe, ..Default::default() })
    }

    /// The paragraph properties style `istd` gives.
    fn pap(&self, istd: u16) -> Pap {
        self.paps.get(istd as usize).cloned().unwrap_or_else(|| Pap { istd, ..Default::default() })
    }

    fn resolve_chp(&self, istd: u16) -> Chp {
        let mut chp = Chp { ftc: self.ftc, ftc_fe: self.ftc_fe, ..Default::default() };
        for s in self.chain(istd) {
            let base = chp.clone();
            self.apply_chp(&mut chp, &s.chpx, &base, 0);
        }
        chp
    }

    fn resolve_pap(&self, istd: u16) -> Pap {
        let mut pap = Pap::default();
        for s in self.chain(istd) {
            apply_pap(&mut pap, &s.papx, &[], &mut 0, 0);
        }
        pap.istd = istd;
        pap
    }

    /// Apply character sprms; `style` is what toggles are relative to.
    fn apply_chp(&self, chp: &mut Chp, grpprl: &[u8], style: &Chp, depth: usize) {
        let mut styled = false;
        for (op, v) in sprms(grpprl) {
            let b = v.first().copied().unwrap_or(0);
            let w = u16_at(v, 0).unwrap_or(0);
            match op {
                0x0800 => chp.deleted = b != 0,
                0x0835 => chp.bold = toggle(b, chp.bold, style.bold),
                0x0836 => chp.italic = toggle(b, chp.italic, style.italic),
                0x0837 | 0x2A53 => chp.strike = toggle(b, chp.strike, style.strike),
                0x083C => chp.hidden = toggle(b, chp.hidden, style.hidden),
                0x2A3E => chp.underline = b != 0,
                0x4A43 if w > 0 => chp.hps = w,
                0x2A42 => chp.color = ico(b),
                0x6870 => chp.color = if v.get(3) == Some(&0xFF) { None } else { Some(Rgb(b, *v.get(1).unwrap_or(&0), *v.get(2).unwrap_or(&0))) },
                0x2A0C => chp.highlight = ico(b),
                0x2A48 => chp.iss = b,
                0x4A4F => chp.ftc = w,
                0x4A50 => chp.ftc_fe = w,
                // sprmCIstd: a character style, laid over what the paragraph gives. A run has one.
                0x4A30 if depth == 0 && !styled => {
                    styled = true;
                    if let Some(chain) = self.char_chains.get(w as usize) {
                        self.apply_chp(chp, chain, style, depth + 1);
                    }
                }
                _ => {}
            }
        }
    }
}

/// Apply paragraph sprms. `data` is the `Data` stream, where oversized property lists live;
/// `huge` is how many bytes of those may still be read.
fn apply_pap(pap: &mut Pap, grpprl: &[u8], data: &[u8], huge: &mut usize, depth: usize) {
    for (op, v) in sprms(grpprl) {
        let b = v.first().copied().unwrap_or(0);
        match op {
            0x4600 => pap.istd = u16_at(v, 0).unwrap_or(0),
            0x2403 | 0x2461 => pap.jc = b,
            0x2407 => pap.page_break_before = b != 0,
            0x2416 => pap.in_table = b != 0,
            0x2417 => pap.ttp = b != 0,
            0x244B => pap.inner_cell = b != 0,
            0x244C => pap.inner_ttp = b != 0,
            0x6649 => pap.itap = u32_at(v, 0).unwrap_or(0),
            0x260A => pap.ilvl = b,
            0x460B => pap.ilfo = u16_at(v, 0).unwrap_or(0),
            0xD608 => pap.tdef = Some(v.to_vec()),
            // sprmPHugePapx: the real list is in the Data stream.
            0x6645 | 0x6646 if depth == 0 => {
                let at = u32_at(v, 0).unwrap_or(u32::MAX) as usize;
                if let Some(cb) = u16_at(data, at).filter(|cb| *cb as usize <= *huge)
                    && let Some(list) = at.checked_add(2).and_then(|o| sub(data, o, cb as usize))
                {
                    *huge -= cb as usize;
                    apply_pap(pap, list, data, huge, depth + 1);
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The document

/// A run of file positions with one property list (from the formatted disk pages).
struct FcRun<'a> {
    start: u32,
    end: u32,
    /// Paragraph runs: the style index. Character runs: unused.
    istd: u16,
    grpprl: &'a [u8],
}

/// The run holding file position `fc`.
fn run_at<'a, 'b>(runs: &'b [FcRun<'a>], fc: u32) -> Option<&'b FcRun<'a>> {
    runs.get(runs.partition_point(|r| r.end <= fc)).filter(|r| r.start <= fc)
}

/// The `PlcBte` at `plc` in the table stream → the runs of its 512-byte pages in `wd`.
fn fkp_runs<'a>(wd: &'a [u8], plc: &[u8], para: bool) -> Vec<FcRun<'a>> {
    let n = plc.len().saturating_sub(4) / 8;
    let mut runs = Vec::new();
    for i in 0..n {
        let pn = u32_at(plc, (n + 1) * 4 + i * 4).unwrap_or(0) & 0x3F_FFFF;
        let Some(page) = (pn as usize).checked_mul(512).and_then(|o| sub(wd, o, 512)) else { continue };
        let count = page.get(511).copied().unwrap_or(0) as usize;
        for j in 0..count {
            let (Some(start), Some(end)) = (u32_at(page, j * 4), u32_at(page, j * 4 + 4)) else { break };
            let entry = (count + 1) * 4 + j * if para { 13 } else { 1 };
            let off = page.get(entry).copied().unwrap_or(0) as usize * 2;
            let mut run = FcRun { start, end, istd: 0, grpprl: &[] };
            if off != 0 {
                let cb = page.get(off).copied().unwrap_or(0) as usize;
                if !para {
                    run.grpprl = sub(page, off + 1, cb).unwrap_or(&[]);
                } else {
                    // A zero count byte means the real one follows (in words).
                    let body = if cb == 0 {
                        sub(page, off + 2, page.get(off + 1).copied().unwrap_or(0) as usize * 2)
                    } else {
                        sub(page, off + 1, cb * 2 - 1)
                    }
                    .unwrap_or(&[]);
                    run.istd = u16_at(body, 0).unwrap_or(0);
                    run.grpprl = body.get(2..).unwrap_or(&[]);
                }
            }
            if end > start {
                runs.push(run);
            }
        }
    }
    runs.sort_by_key(|r| r.start);
    runs
}

/// One piece of the text: `chars` characters at byte `fc` of the WordDocument stream.
struct Piece {
    chars: usize,
    fc: u32,
    /// 8-bit characters (Windows-1252) rather than UTF-16.
    compressed: bool,
}

/// The piece table inside the `Clx` ([MS-DOC] 2.9.38).
fn pieces(clx: &[u8]) -> Vec<Piece> {
    let mut at = 0usize;
    // Skip the property lists in front.
    while clx.get(at) == Some(&1) {
        let Some(cb) = u16_at(clx, at + 1) else { return Vec::new() };
        at += 3 + cb as usize;
    }
    if clx.get(at) != Some(&2) {
        return Vec::new();
    }
    let Some(plc) = u32_at(clx, at + 1).and_then(|lcb| sub(clx, at + 5, lcb as usize)) else { return Vec::new() };
    let n = plc.len().saturating_sub(4) / 12;
    let mut v = Vec::with_capacity(n);
    for i in 0..n {
        let (Some(a), Some(b), Some(fc)) = (u32_at(plc, i * 4), u32_at(plc, i * 4 + 4), u32_at(plc, (n + 1) * 4 + i * 8 + 2)) else { break };
        let compressed = fc & 0x4000_0000 != 0;
        let fc = fc & 0x3FFF_FFFF;
        v.push(Piece { chars: b.saturating_sub(a) as usize, fc: if compressed { fc / 2 } else { fc }, compressed });
    }
    v
}

/// Windows-1252 bytes 0x80..=0x9F.
const CP1252: [char; 32] = [
    '\u{20AC}', '\u{81}', '\u{201A}', '\u{192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}', '\u{2C6}', '\u{2030}', '\u{160}', '\u{2039}',
    '\u{152}', '\u{8D}', '\u{17D}', '\u{8F}', '\u{90}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{2DC}', '\u{2122}', '\u{161}', '\u{203A}', '\u{153}', '\u{9D}', '\u{17E}', '\u{178}',
];

/// The font names of the font table ([MS-DOC] 2.9.273 SttbfFfn).
fn fonts(b: &[u8]) -> Vec<String> {
    let count = u16_at(b, 0).unwrap_or(0) as usize;
    let mut at = 4usize;
    let mut v = Vec::new();
    for _ in 0..count {
        let Some(cb) = b.get(at).map(|c| *c as usize) else { break };
        let Some(ffn) = sub(b, at + 1, cb) else { break };
        // The name follows 39 bytes of metrics; a null ends it (an alternate name may follow).
        let units: Vec<u16> =
            ffn.get(39..).unwrap_or(&[]).as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).take_while(|u| *u != 0).collect();
        v.push(String::from_utf16_lossy(&units));
        at += 1 + cb;
    }
    v
}

/// List formats: for each list override (`ilfo`, 1-based), whether each level is numbered.
fn lists(plf_lst: &[u8], plf_lfo: &[u8]) -> Vec<[bool; 9]> {
    let count = u16_at(plf_lst, 0).unwrap_or(0) as usize;
    let mut defs: Vec<(u32, usize)> = Vec::new();
    for i in 0..count {
        let Some(lstf) = sub(plf_lst, 2 + i * 28, 28) else { break };
        let simple = lstf.get(26).copied().unwrap_or(0) & 1 != 0;
        defs.push((u32_at(lstf, 0).unwrap_or(0), if simple { 1 } else { 9 }));
    }
    // The level definitions follow the list headers, in order.
    let mut at = 2 + defs.len() * 28;
    let mut by_id: HashMap<u32, [bool; 9]> = HashMap::new();
    'lists: for (lsid, levels) in defs {
        let mut ordered = [true; 9];
        for slot in ordered.iter_mut().take(levels) {
            let Some(lvlf) = sub(plf_lst, at, 28) else { break 'lists };
            // nfc 23 is a bullet, 255 no number at all.
            *slot = !matches!(lvlf.get(4), Some(23 | 255));
            let grpprls = lvlf.get(24).copied().unwrap_or(0) as usize + lvlf.get(25).copied().unwrap_or(0) as usize;
            let Some(cch) = u16_at(plf_lst, at + 28 + grpprls) else { break 'lists };
            at += 28 + grpprls + 2 + cch as usize * 2;
        }
        let first = ordered[0];
        ordered.iter_mut().skip(levels).for_each(|o| *o = first);
        by_id.entry(lsid).or_insert(ordered);
    }
    let overrides = (u32_at(plf_lfo, 0).unwrap_or(0) as usize).min(plf_lfo.len() / 16);
    (0..overrides)
        .map(|i| {
            let lsid = u32_at(plf_lfo, 4 + i * 16).unwrap_or(0);
            by_id.get(&lsid).copied().unwrap_or([false; 9])
        })
        .collect()
}

/// Page size and margins of a section, points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Page {
    pub w: f32,
    pub h: f32,
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
    pub landscape: bool,
}

/// A section's properties: its page, and whether it starts on a new page.
fn section(wd: &[u8], fc_sepx: u32) -> (Page, bool) {
    // The defaults of the format: US Letter, 1" top and bottom, 1.25" left and right.
    let (mut w, mut h, mut left, mut right, mut top, mut bottom) = (12240i32, 15840i32, 1800i32, 1800i32, 1440i32, 1440i32);
    let (mut landscape, mut new_page) = (false, true);
    let grpprl = match fc_sepx {
        u32::MAX => None,
        fc => u16_at(wd, fc as usize).and_then(|cb| sub(wd, (fc as usize).checked_add(2)?, cb as usize)),
    };
    for (op, v) in sprms(grpprl.unwrap_or(&[])) {
        let word = u16_at(v, 0).unwrap_or(0);
        match op {
            0xB01F => w = word as i32,
            0xB020 => h = word as i32,
            0xB021 => left = word as i32,
            0xB022 => right = word as i32,
            // Negative = "exactly"; the size is what counts here.
            0x9023 => top = (word as i16 as i32).abs(),
            0x9024 => bottom = (word as i16 as i32).abs(),
            0x301D => landscape = v.first() == Some(&2),
            0x3009 => new_page = v.first().is_some_and(|b| *b >= 2),
            _ => {}
        }
    }
    let pt = |twips: i32| twips as f32 / 20.0;
    // Keep hostile sizes sane: at least an inch of page, margins that leave room for text.
    let (w, h) = (pt(w).clamp(72.0, 1584.0), pt(h).clamp(72.0, 1584.0));
    let side = |m: i32, total: f32| pt(m).clamp(0.0, total / 2.0 - 18.0);
    (Page { w, h, left: side(left, w), right: side(right, w), top: side(top, h), bottom: side(bottom, h), landscape }, new_page)
}

/// A stretch of a paragraph's text with one property list.
struct Chunk<'a> {
    text: String,
    grpprl: &'a [u8],
    /// Index into the builder's link targets.
    link: Option<usize>,
}

struct Field {
    instr: String,
    in_result: bool,
    link: Option<usize>,
}

/// Is `c` East Asian text (which takes the East Asian font)?
fn is_east_asian(c: char) -> bool {
    matches!(c as u32, 0x1100..=0x11FF | 0x2E80..=0xA4CF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFFEF | 0x20000..=0x3FFFF)
}

/// Collects paragraphs into blocks, gathering table rows as they end.
struct Builder<'a> {
    styles: Styles,
    fonts: Vec<String>,
    lists: Vec<[bool; 9]>,
    data: &'a [u8],
    blocks: Vec<FBlock>,
    /// The table being read: finished rows (with their cell edges), the row and the cell in progress.
    rows: Vec<(Vec<Cell>, Vec<f32>)>,
    row: Vec<Cell>,
    cell: Vec<FBlock>,
    chunks: Vec<Chunk<'a>>,
    fields: Vec<Field>,
    /// The targets of the hyperlink fields met so far.
    links: Vec<String>,
    huge_budget: usize,
    /// Fields opened beyond `MAX_FIELDS` (their text is dropped).
    overflow: usize,
    page_break: bool,
}

impl<'a> Builder<'a> {
    fn push_char(&mut self, c: char, grpprl: &'a [u8]) {
        if self.overflow > 0 {
            return;
        }
        if let Some(f) = self.fields.iter_mut().rev().find(|f| !f.in_result) {
            if f.instr.len() < 4096 {
                f.instr.push(c);
            }
            return;
        }
        let link = self.fields.iter().rev().find_map(|f| f.link);
        match self.chunks.last_mut() {
            Some(last) if std::ptr::eq(last.grpprl, grpprl) && last.link == link => last.text.push(c),
            _ => self.chunks.push(Chunk { text: c.to_string(), grpprl, link }),
        }
    }

    fn field_char(&mut self, c: u16) {
        match c {
            0x13 if self.fields.len() >= MAX_FIELDS => self.overflow += 1,
            0x13 => self.fields.push(Field { instr: String::new(), in_result: false, link: None }),
            _ if self.overflow > 0 => {
                if c == 0x15 {
                    self.overflow -= 1;
                }
            }
            0x14 => {
                if let Some(f) = self.fields.last_mut() {
                    f.in_result = true;
                    f.link = crate::rtf::hyperlink(&f.instr).map(|target| {
                        self.links.push(target);
                        self.links.len() - 1
                    });
                }
            }
            _ => {
                self.fields.pop();
            }
        }
    }

    /// The paragraph in progress, formatted with `pap`.
    fn take_para(&mut self, pap: &Pap) -> Para {
        let std = self.styles.get(pap.istd);
        let kind = match std.map(|s| s.sti) {
            Some(n @ 1..=6) => Kind::Heading(n as u8),
            Some(62) => Kind::Title,
            _ => Kind::Normal,
        };
        let mut para = Para::new(kind);
        para.align = match pap.jc {
            1 => Some(Align::Center),
            2 => Some(Align::Right),
            3 | 4 => Some(Align::Justify),
            _ => None,
        };
        para.page_break = std::mem::take(&mut self.page_break) || pap.page_break_before;
        if (1..0x07FF).contains(&pap.ilfo) {
            let level = pap.ilvl.min(8);
            let ordered = self.lists.get(pap.ilfo as usize - 1).and_then(|l| l.get(level as usize)).copied().unwrap_or(false);
            para.list = Some(ListInfo { ordered, level });
        }
        let base = self.styles.chp(pap.istd);
        for chunk in std::mem::take(&mut self.chunks) {
            let mut chp = base.clone();
            self.styles.apply_chp(&mut chp, chunk.grpprl, &base, 0);
            if chp.hidden || chp.deleted {
                continue;
            }
            let mut fmt = Fmt {
                bold: chp.bold,
                italic: chp.italic,
                underline: chp.underline,
                strike: chp.strike,
                sup: chp.iss == 1,
                sub: chp.iss == 2,
                link: chunk.link.and_then(|i| self.links.get(i).cloned()),
                color: chp.color,
                size: Some(chp.hps.min(3276) as f32 / 2.0),
                background: chp.highlight,
                ..Default::default()
            };
            // Latin and East Asian stretches take different fonts.
            let mut rest = chunk.text.as_str();
            while let Some(first) = rest.chars().next() {
                let fe = is_east_asian(first);
                let end = rest.char_indices().find(|(_, c)| is_east_asian(*c) != fe).map_or(rest.len(), |(i, _)| i);
                let (head, tail) = rest.split_at_checked(end).unwrap_or((rest, ""));
                fmt.font = self.fonts.get(if fe { chp.ftc_fe } else { chp.ftc } as usize).filter(|f| !f.is_empty()).cloned();
                para.push_text(head, &fmt);
                rest = tail;
            }
        }
        para
    }

    /// A paragraph ended with the mark `mark` (0x0D, or 0x07 for a table cell or row).
    fn end_para(&mut self, mark: u16, pap: &Pap) {
        let in_table = pap.in_table || pap.itap > 0;
        if !in_table {
            self.end_table();
            let para = self.take_para(pap);
            self.blocks.push(FBlock::Para(para));
            return;
        }
        // The end of a row of a table inside a cell: tables in tables are read as paragraphs.
        if pap.inner_ttp {
            self.chunks.clear();
            return;
        }
        if mark != 0x07 {
            let para = self.take_para(pap);
            self.cell.push(FBlock::Para(para));
        } else if pap.ttp {
            self.chunks.clear();
            let mut row = std::mem::take(&mut self.row);
            let edges = pap.tdef.as_deref().map(|t| shape_row(&mut row, t)).unwrap_or_default();
            if !row.is_empty() {
                self.rows.push((row, edges));
            }
        } else {
            let para = self.take_para(pap);
            self.cell.push(FBlock::Para(para));
            let blocks = std::mem::take(&mut self.cell);
            if self.row.len() < MAX_CELLS {
                self.row.push(Cell { blocks, ..Default::default() });
            }
        }
    }

    /// Close the table in progress, if any.
    fn end_table(&mut self) {
        // A table cut short (damaged file): keep what was read.
        let blocks = std::mem::take(&mut self.cell);
        if !blocks.is_empty() {
            self.row.push(Cell { blocks, ..Default::default() });
        }
        let row = std::mem::take(&mut self.row);
        if !row.is_empty() {
            self.rows.push((row, Vec::new()));
        }
        if self.rows.is_empty() {
            return;
        }
        let mut rows = std::mem::take(&mut self.rows);
        let widths = grid(&mut rows);
        self.blocks.push(FBlock::Table(FTable { rows: rows.into_iter().map(|(r, _)| r).collect(), widths }));
    }
}

/// Lay rows with different cell edges on one grid: every edge of every row becomes a grid line
/// and each cell spans the grid columns between its own edges. Returns the grid column widths in
/// points, or nothing (cells left one column each) when a row's edges are unknown.
fn grid(rows: &mut [(Vec<Cell>, Vec<f32>)]) -> Vec<f32> {
    if rows.iter().any(|(cells, edges)| edges.len() != cells.len() + 1) {
        return Vec::new();
    }
    let mut lines: Vec<f32> = rows.iter().flat_map(|(_, e)| e.iter().copied()).filter(|x| x.is_finite()).collect();
    lines.sort_by(f32::total_cmp);
    // Edges within a point of each other are the same line.
    lines.dedup_by(|b, a| *b - *a < 1.0);
    if lines.len() < 2 || lines.len() > MAX_CELLS + 1 {
        return Vec::new();
    }
    let line = |x: f32| lines.partition_point(|l| *l < x - 0.5).min(lines.len() - 1);
    for (cells, edges) in rows.iter_mut() {
        for (cell, edge) in cells.iter_mut().zip(edges.windows(2)) {
            cell.colspan = line(edge[1]).saturating_sub(line(edge[0])).max(1) as u32;
        }
    }
    lines.windows(2).map(|w| w[1] - w[0]).collect()
}

/// Apply a row's table definition (sprmTDefTable) to its cells: merged cells fold into their
/// neighbour, vertical merges are marked. Returns the edges of the cells that remain, in points
/// (one more than the cells), or nothing when the definition doesn't match the row.
fn shape_row(row: &mut Vec<Cell>, tdef: &[u8]) -> Vec<f32> {
    let n = tdef.get(2).copied().unwrap_or(0) as usize;
    let edge = |i: usize| u16_at(tdef, 3 + i * 2).map(|x| x as i16 as f32 / 20.0);
    let Some(first) = edge(0) else { return Vec::new() };
    if n == 0 || n != row.len() {
        return Vec::new();
    }
    let flags = |i: usize| u16_at(tdef, 3 + (n + 1) * 2 + i * 20).unwrap_or(0);
    let mut out: Vec<Cell> = Vec::with_capacity(n);
    let mut edges = vec![first];
    for (i, mut cell) in std::mem::take(row).into_iter().enumerate() {
        let Some(right) = edge(i + 1) else {
            out.push(cell);
            continue;
        };
        // fMerged: continues the cell before it, which now reaches this cell's right edge.
        if flags(i) & 0x0002 != 0 && !out.is_empty() {
            edges.pop();
        } else {
            // fVertMerge without fVertRestart: continues the cell above.
            cell.covered = flags(i) & 0x0020 != 0 && flags(i) & 0x0040 == 0;
            out.push(cell);
        }
        edges.push(right);
    }
    *row = out;
    edges
}

/// Parse a legacy Word document into the flow model and the page of its last section.
pub fn parse(bytes: &[u8]) -> Result<(Flow, Page), String> {
    let cfb = Cfb::open(bytes)?;
    let Some(wd) = cfb.stream("WordDocument") else {
        return Err(if cfb.stream("EncryptedPackage").is_some() {
            "this document is password-protected; encrypted documents can't be opened yet".into()
        } else {
            "not a Word document (no WordDocument stream)".into()
        });
    };
    let wd = wd.as_slice();
    let bad = || "damaged Word document (file information block)".to_string();
    let ident = u16_at(wd, 0).ok_or_else(bad)?;
    let version = u16_at(wd, 2).ok_or_else(bad)?;
    let flags = u16_at(wd, 10).ok_or_else(bad)?;
    if ident == 0xA5DC || (ident == 0xA5EC && version < 0x00C1) {
        return Err("this document was saved by Word 6.0 or Word 95; only Word 97–2003 documents can be opened".into());
    }
    if ident != 0xA5EC {
        return Err("not a Word document".into());
    }
    if flags & 0x8100 != 0 {
        return Err("this document is password-protected; encrypted documents can't be opened yet".into());
    }
    let table = cfb.stream(if flags & 0x0200 != 0 { "1Table" } else { "0Table" }).ok_or("damaged Word document (no table stream)")?;
    let table = table.as_slice();
    let data = cfb.stream("Data").unwrap_or_default();

    // The file information block: three counted arrays after the 32-byte base.
    let rg_lw = 34 + u16_at(wd, 32).ok_or_else(bad)? as usize * 2;
    let rg_fc = rg_lw + 2 + u16_at(wd, rg_lw).ok_or_else(bad)? as usize * 4;
    let pairs = u16_at(wd, rg_fc).ok_or_else(bad)? as usize;
    let ccp_text = u32_at(wd, rg_lw + 2 + 12).ok_or_else(bad)? as usize;
    // Entry `i` of the offset/size pairs, as bytes of the table stream.
    let part = |i: usize| -> &[u8] {
        if i >= pairs {
            return &[];
        }
        let at = rg_fc + 2 + i * 8;
        match (u32_at(wd, at), u32_at(wd, at + 4)) {
            (Some(fc), Some(lcb)) => sub(table, fc as usize, lcb as usize).unwrap_or(&[]),
            _ => &[],
        }
    };

    let pieces = pieces(part(33));
    if pieces.is_empty() {
        return Err("damaged Word document (no text)".into());
    }
    let chpx = fkp_runs(wd, part(12), false);
    let papx = fkp_runs(wd, part(13), true);

    // Section ends: the character ending each section, and whether the next starts a new page.
    let plc_sed = part(6);
    let sections = plc_sed.len().saturating_sub(4) / 16;
    let sepx = |i: usize| u32_at(plc_sed, (sections + 1) * 4 + i * 12 + 2).unwrap_or(u32::MAX);
    let mut continuous_after = std::collections::HashSet::new();
    for i in 1..sections.min(MAX_SECTIONS) {
        if let Some(cp) = u32_at(plc_sed, i * 4)
            && !section(wd, sepx(i)).1
        {
            continuous_after.insert(cp.saturating_sub(1) as usize);
        }
    }
    let page = section(wd, if sections > 0 { sepx(sections - 1) } else { u32::MAX }).0;

    let mut b = Builder {
        styles: Styles::parse(part(1)),
        fonts: fonts(part(15)),
        lists: lists(part(73), part(74)),
        data: &data,
        blocks: Vec::new(),
        rows: Vec::new(),
        row: Vec::new(),
        cell: Vec::new(),
        chunks: Vec::new(),
        fields: Vec::new(),
        links: Vec::new(),
        huge_budget: MAX_HUGE_PROPS,
        overflow: 0,
        page_break: false,
    };
    let para_props = |b: &mut Builder, fc: u32| -> Pap {
        match run_at(&papx, fc) {
            Some(run) => {
                let mut pap = b.styles.pap(run.istd);
                apply_pap(&mut pap, run.grpprl, b.data, &mut b.huge_budget, 0);
                pap
            }
            None => Pap::default(),
        }
    };

    let limit = ccp_text.min(MAX_CHARS);
    let mut cp = 0usize;
    'text: for piece in &pieces {
        let width: u32 = if piece.compressed { 1 } else { 2 };
        for i in 0..piece.chars {
            if cp >= limit {
                break 'text;
            }
            let Some(fc) = (i as u32).checked_mul(width).and_then(|o| piece.fc.checked_add(o)) else { break 'text };
            let unit = if piece.compressed { wd.get(fc as usize).map(|c| *c as u16) } else { u16_at(wd, fc as usize) };
            let Some(unit) = unit else { break 'text };
            let grpprl: &[u8] = run_at(&chpx, fc).map_or(&[], |r| r.grpprl);
            match unit {
                0x0D | 0x07 => {
                    let pap = para_props(&mut b, fc);
                    b.end_para(unit, &pap);
                }
                0x0C => {
                    // A page break, or the end of a section. Text before it stays a paragraph.
                    let pap = para_props(&mut b, fc);
                    if !b.chunks.is_empty() {
                        b.end_para(0x0D, &pap);
                    }
                    b.page_break = !continuous_after.contains(&cp);
                }
                0x13..=0x15 => b.field_char(unit),
                0x0B => b.push_char('\n', grpprl),
                0x09 => b.push_char('\t', grpprl),
                0x1E => b.push_char('\u{2011}', grpprl),
                // Other control characters stand for pictures, notes and the like.
                u if u < 0x20 => {}
                0x80..=0x9F if piece.compressed => b.push_char(CP1252.get(unit as usize - 0x80).copied().unwrap_or('?'), grpprl),
                _ => b.push_char(char::from_u32(unit as u32).unwrap_or('\u{FFFD}'), grpprl),
            }
            cp += 1;
        }
    }
    if !b.chunks.is_empty() {
        b.end_para(0x0D, &Pap::default());
    }
    b.end_table();
    Ok((Flow { blocks: b.blocks, meta: Default::default() }, page))
}

/// Parse a legacy Word document (`.doc`) into a document.
pub fn import(bytes: &[u8]) -> Result<Document, String> {
    let (flow, page) = parse(bytes)?;
    let mut doc = model::to_doc(&flow);
    let s = &mut doc.last_section;
    (s.page_w, s.page_h, s.landscape) = (page.w, page.h, page.landscape);
    (s.margin_left, s.margin_right, s.margin_top, s.margin_bottom) = (page.left, page.right, page.top, page.bottom);
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Inline;

    fn w16(v: &mut Vec<u8>, x: u16) {
        v.extend_from_slice(&x.to_le_bytes());
    }
    fn w32(v: &mut Vec<u8>, x: u32) {
        v.extend_from_slice(&x.to_le_bytes());
    }
    fn utf16(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }
    fn sprm(op: u16, operand: &[u8]) -> Vec<u8> {
        [&op.to_le_bytes()[..], operand].concat()
    }

    /// A compound file (512-byte sectors) holding `streams`; those under 4096 bytes go to the
    /// mini stream, as in real files.
    fn cfb(streams: &[(&str, Vec<u8>)]) -> Vec<u8> {
        const END: u32 = 0xFFFF_FFFE;
        let mut sectors: Vec<Vec<u8>> = vec![Vec::new()]; // sector 0: the FAT, filled in last
        let mut fat: Vec<u32> = vec![0xFFFF_FFFD];
        // Append `bytes` as a chain of sectors; returns the first.
        let chain = |bytes: &[u8], sectors: &mut Vec<Vec<u8>>, fat: &mut Vec<u32>| -> u32 {
            let first = sectors.len() as u32;
            let chunks: Vec<&[u8]> = bytes.chunks(512).collect();
            for (i, c) in chunks.iter().enumerate() {
                let mut s = c.to_vec();
                s.resize(512, 0);
                sectors.push(s);
                fat.push(if i + 1 == chunks.len() { END } else { first + i as u32 + 1 });
            }
            first
        };
        let (mut mini, mut minifat) = (Vec::new(), Vec::new());
        let mut entries = Vec::new(); // (name, start, size)
        for (name, bytes) in streams {
            if bytes.len() < 4096 {
                let first = minifat.len() as u32;
                let n = bytes.len().div_ceil(64).max(1);
                for i in 0..n {
                    minifat.push(if i + 1 == n { END } else { first + i as u32 + 1 });
                }
                mini.extend_from_slice(bytes);
                mini.resize(minifat.len() * 64, 0);
                entries.push((*name, first, bytes.len()));
            } else {
                entries.push((*name, chain(bytes, &mut sectors, &mut fat), bytes.len()));
            }
        }
        let mini_start = if mini.is_empty() { END } else { chain(&mini, &mut sectors, &mut fat) };
        let minifat_bytes: Vec<u8> = minifat.iter().flat_map(|x| x.to_le_bytes()).collect();
        let minifat_start = if minifat.is_empty() { END } else { chain(&minifat_bytes, &mut sectors, &mut fat) };
        // The directory: the root, then the streams as a chain of right siblings.
        let mut dir = Vec::new();
        let mut entry = |name: &str, kind: u8, right: u32, child: u32, start: u32, size: usize| {
            let mut e = utf16(name);
            let len = e.len() as u16 + 2;
            e.resize(64, 0);
            w16(&mut e, len);
            e.push(kind);
            e.push(1);
            w32(&mut e, NO_STREAM);
            w32(&mut e, right);
            w32(&mut e, child);
            e.resize(116, 0);
            w32(&mut e, start);
            w32(&mut e, size as u32);
            w32(&mut e, 0);
            dir.extend_from_slice(&e);
        };
        entry("Root Entry", 5, NO_STREAM, if entries.is_empty() { NO_STREAM } else { 1 }, mini_start, mini.len());
        for (i, (name, start, size)) in entries.iter().enumerate() {
            let right = if i + 1 == entries.len() { NO_STREAM } else { i as u32 + 2 };
            entry(name, 2, right, NO_STREAM, *start, *size);
        }
        let dir_start = chain(&dir, &mut sectors, &mut fat);
        assert!(fat.len() <= 128, "test file too large for one FAT sector");
        let mut fat_bytes: Vec<u8> = fat.iter().flat_map(|x| x.to_le_bytes()).collect();
        fat_bytes.resize(512, 0xFF);
        sectors[0] = fat_bytes;

        let mut h = CFB_MAGIC.to_vec();
        h.resize(24, 0);
        w16(&mut h, 0x3E);
        w16(&mut h, 3);
        w16(&mut h, 0xFFFE);
        w16(&mut h, 9);
        w16(&mut h, 6);
        h.resize(44, 0);
        w32(&mut h, 1); // FAT sectors
        w32(&mut h, dir_start);
        w32(&mut h, 0);
        w32(&mut h, 4096);
        w32(&mut h, minifat_start);
        w32(&mut h, 1);
        w32(&mut h, END); // no DIFAT sectors
        w32(&mut h, 0);
        w32(&mut h, 0); // the FAT is sector 0
        h.resize(512, 0xFF);
        h.extend(sectors.into_iter().flatten());
        h
    }

    /// Builds the WordDocument and table streams of a small document.
    #[derive(Default)]
    struct Doc {
        /// Text bytes, stored from `TEXT` on.
        text: Vec<u8>,
        chars: u32,
        /// (first character, file position, compressed).
        pieces: Vec<(u32, u32, bool)>,
        chpx: Vec<(u32, u32, Vec<u8>)>,
        papx: Vec<(u32, u32, u16, Vec<u8>)>,
        para_start: u32,
        flags: u16,
        version: u16,
    }

    const TEXT: u32 = 0x400;

    impl Doc {
        fn new() -> Doc {
            Doc { para_start: TEXT, flags: 0x0200, version: 0x00C1, ..Default::default() }
        }

        /// Text with character properties `chpx`: 8-bit when it fits, UTF-16 otherwise.
        fn run(&mut self, s: &str, chpx: &[u8]) -> &mut Doc {
            let start = TEXT + self.text.len() as u32;
            let narrow = s.chars().all(|c| (c as u32) < 0x100);
            self.pieces.push((self.chars, start, narrow));
            if narrow {
                self.text.extend(s.chars().map(|c| c as u8));
            } else {
                self.text.extend(utf16(s));
            }
            self.chars += s.encode_utf16().count() as u32;
            self.chpx.push((start, TEXT + self.text.len() as u32, chpx.to_vec()));
            self
        }

        /// End the paragraph with `mark`, style `istd` and paragraph properties `papx`.
        fn end(&mut self, mark: char, istd: u16, papx: &[u8]) -> &mut Doc {
            self.run(&mark.to_string(), &[]);
            let end = TEXT + self.text.len() as u32;
            self.papx.push((self.para_start, end, istd, papx.to_vec()));
            self.para_start = end;
            self
        }

        /// A formatted disk page: positions, then entries pointing at property lists stored from the end.
        fn fkp(runs: &[(u32, u32, Vec<u8>)], para: bool) -> Vec<u8> {
            let mut page = vec![0u8; 512];
            let n = runs.len();
            let mut at = 0;
            for (i, (start, end, _)) in runs.iter().enumerate() {
                page[i * 4..i * 4 + 4].copy_from_slice(&start.to_le_bytes());
                page[i * 4 + 4..i * 4 + 8].copy_from_slice(&end.to_le_bytes());
                at = i * 4 + 8;
            }
            let mut free = 510;
            for (i, (_, _, body)) in runs.iter().enumerate() {
                let stored = if !para {
                    [&[body.len() as u8][..], body].concat()
                } else if body.len() % 2 == 1 {
                    [&[body.len().div_ceil(2) as u8][..], body].concat()
                } else {
                    [&[0, (body.len() / 2) as u8][..], body].concat()
                };
                free = (free - stored.len()) & !1;
                page[free..free + stored.len()].copy_from_slice(&stored);
                page[at + i * if para { 13 } else { 1 }] = (free / 2) as u8;
            }
            assert!(at + n * if para { 13 } else { 1 } <= free, "test page overflow");
            page[511] = n as u8;
            page
        }

        fn style(sti: u16, base: u16, name: &str, chpx: &[u8]) -> Vec<u8> {
            let mut s = Vec::new();
            w16(&mut s, sti);
            w16(&mut s, 1 | base << 4);
            w16(&mut s, 2);
            w16(&mut s, 0);
            w16(&mut s, 0);
            w16(&mut s, name.len() as u16);
            s.extend(utf16(name));
            w16(&mut s, 0);
            w16(&mut s, 2); // the paragraph UPX: just the style index
            w16(&mut s, 0);
            w16(&mut s, chpx.len() as u16);
            s.extend_from_slice(chpx);
            s
        }

        fn build(&self) -> Vec<u8> {
            // The WordDocument stream: header, text, section properties, two formatted pages.
            let mut wd = vec![0u8; TEXT as usize];
            wd.extend_from_slice(&self.text);
            let sepx_at = wd.len() as u32;
            let sep = [sprm(0xB01F, &11906u16.to_le_bytes()), sprm(0xB020, &16838u16.to_le_bytes()), sprm(0xB021, &1440u16.to_le_bytes())].concat();
            w16(&mut wd, sep.len() as u16);
            wd.extend_from_slice(&sep);
            wd.resize(wd.len().next_multiple_of(512), 0);
            let chpx_page = (wd.len() / 512) as u32;
            wd.extend(Doc::fkp(&self.chpx, false));
            let papx: Vec<(u32, u32, Vec<u8>)> = self.papx.iter().map(|(a, b, istd, g)| (*a, *b, [&istd.to_le_bytes()[..], g].concat())).collect();
            wd.extend(Doc::fkp(&papx, true));
            wd.resize(wd.len().max(4096), 0);

            // The table stream, part by part.
            let mut table = Vec::new();
            let mut parts: Vec<(usize, u32, u32)> = Vec::new();
            let mut part = |i: usize, bytes: Vec<u8>, table: &mut Vec<u8>| {
                parts.push((i, table.len() as u32, bytes.len() as u32));
                table.extend(bytes);
            };
            let mut stsh = Vec::new();
            w16(&mut stsh, 18);
            for x in [2u16, 10, 0, 0, 0, 0, 0, 1, 0] {
                w16(&mut stsh, x);
            }
            for std in [Doc::style(0, 0xFFF, "Normal", &sprm(0x4A43, &[24, 0])), Doc::style(1, 0, "heading 1", &sprm(0x0835, &[1]))] {
                w16(&mut stsh, std.len() as u16);
                stsh.extend(std);
            }
            part(1, stsh, &mut table);
            let mut sed = Vec::new();
            w32(&mut sed, 0);
            w32(&mut sed, self.chars);
            w16(&mut sed, 0);
            w32(&mut sed, sepx_at);
            w16(&mut sed, 0);
            w32(&mut sed, 0);
            part(6, sed, &mut table);
            let bte = |first: u32, last: u32, page: u32| [first.to_le_bytes(), last.to_le_bytes(), page.to_le_bytes()].concat();
            let end = TEXT + self.text.len() as u32;
            part(12, bte(TEXT, end, chpx_page), &mut table);
            part(13, bte(TEXT, end, chpx_page + 1), &mut table);
            let mut ffn = Vec::new();
            w16(&mut ffn, 2);
            w16(&mut ffn, 0);
            for name in ["Arial", "MingLiU"] {
                let name = [utf16(name), vec![0, 0]].concat();
                ffn.push(39 + name.len() as u8);
                ffn.extend([0u8; 39]);
                ffn.extend(name);
            }
            part(15, ffn, &mut table);
            let mut clx = vec![2u8];
            w32(&mut clx, (self.pieces.len() * 12 + 4) as u32);
            for (cp, _, _) in &self.pieces {
                w32(&mut clx, *cp);
            }
            w32(&mut clx, self.chars);
            for (_, fc, compressed) in &self.pieces {
                w16(&mut clx, 0);
                w32(&mut clx, if *compressed { (fc * 2) | 0x4000_0000 } else { *fc });
                w16(&mut clx, 0);
            }
            part(33, clx, &mut table);
            // One bulleted list (id 7) and the override that paragraphs refer to.
            let mut lst = Vec::new();
            w16(&mut lst, 1);
            w32(&mut lst, 7);
            lst.resize(2 + 26, 0);
            lst.extend([1, 0]);
            let mut lvl = vec![0u8; 28];
            lvl[4] = 23;
            lst.extend(lvl);
            w16(&mut lst, 1);
            w16(&mut lst, 0xF0B7);
            part(73, lst, &mut table);
            let mut lfo = Vec::new();
            w32(&mut lfo, 1);
            w32(&mut lfo, 7);
            lfo.resize(4 + 16, 0);
            part(74, lfo, &mut table);

            wd[0..2].copy_from_slice(&0xA5ECu16.to_le_bytes());
            wd[2..4].copy_from_slice(&self.version.to_le_bytes());
            wd[10..12].copy_from_slice(&self.flags.to_le_bytes());
            wd[32..34].copy_from_slice(&14u16.to_le_bytes());
            wd[62..64].copy_from_slice(&22u16.to_le_bytes());
            wd[76..80].copy_from_slice(&self.chars.to_le_bytes());
            wd[152..154].copy_from_slice(&93u16.to_le_bytes());
            for (i, fc, lcb) in parts {
                wd[154 + i * 8..158 + i * 8].copy_from_slice(&fc.to_le_bytes());
                wd[158 + i * 8..162 + i * 8].copy_from_slice(&lcb.to_le_bytes());
            }
            cfb(&[("WordDocument", wd), ("1Table", table)])
        }
    }

    /// A heading, formatted text in both encodings, a list item, a table, a link and a page break.
    fn sample() -> Vec<u8> {
        let bold = sprm(0x0835, &[1]);
        let cell = sprm(0x2416, &[1]);
        // The row: two cells with edges at 0, 100 and 250 points.
        let mut tdef = vec![48, 0, 2];
        for edge in [0u16, 2000, 5000] {
            w16(&mut tdef, edge);
        }
        tdef.resize(49, 0);
        let row = [cell.clone(), sprm(0x2417, &[1]), sprm(0xD608, &tdef)].concat();
        let mut d = Doc::new();
        d.run("Head", &[]).end('\r', 1, &[]);
        d.run("Plain ", &[]).run("bold", &bold).run(" caf\u{e9} \u{93}q\u{94}", &[]).end('\r', 0, &sprm(0x2461, &[1]));
        d.run("\u{4e2d}\u{6587} text", &sprm(0x4A43, &[40, 0])).end('\r', 0, &[]);
        d.run("item", &[]).end('\r', 0, &[sprm(0x460B, &[1, 0]), sprm(0x260A, &[0])].concat());
        d.run("a", &[]).end('\u{7}', 0, &cell).run("b", &[]).end('\u{7}', 0, &cell).end('\u{7}', 0, &row);
        d.run("\u{13}HYPERLINK \"http://a.b\"\u{14}link\u{15} x\u{1}y", &[]).end('\r', 0, &[]);
        d.run("\u{c}after", &[]).end('\r', 0, &[]);
        d.build()
    }

    fn para(b: &FBlock) -> &Para {
        match b {
            FBlock::Para(p) => p,
            FBlock::Table(_) => panic!("expected a paragraph"),
        }
    }

    #[test]
    fn reads_text_formatting_lists_tables_and_links() {
        let (flow, page) = parse(&sample()).unwrap();
        let texts: Vec<String> = flow
            .blocks
            .iter()
            .map(|b| match b {
                FBlock::Para(p) => format!("{:?}:{}", p.kind, p.text()),
                FBlock::Table(t) => {
                    format!("table{}x{}:{}", t.rows.len(), t.cols(), t.rows.iter().flatten().map(Cell::text).collect::<Vec<_>>().join("|"))
                }
            })
            .collect();
        assert_eq!(
            texts,
            ["Heading(1):Head", "Normal:Plain bold café “q”", "Normal:中文 text", "Normal:item", "table1x2:a|b", "Normal:link xy", "Normal:after"]
        );
        // The heading style's bold and the Normal style's 12 points reach the text.
        assert!(
            matches!(&para(&flow.blocks[0]).inlines[0], Inline::Text(_, f) if f.bold && f.size == Some(12.0) && f.font.as_deref() == Some("Arial"))
        );
        let p = para(&flow.blocks[1]);
        assert_eq!(p.align, Some(Align::Center));
        assert!(p.inlines.iter().any(|i| matches!(i, Inline::Text(t, f) if t == "bold" && f.bold)));
        assert!(p.inlines.iter().any(|i| matches!(i, Inline::Text(t, f) if t == "Plain " && !f.bold)));
        // East Asian text takes the East Asian font, Latin text in the same run the Latin one.
        let p = para(&flow.blocks[2]);
        assert!(matches!(&p.inlines[0], Inline::Text(t, f) if t == "中文" && f.font.as_deref() == Some("MingLiU") && f.size == Some(20.0)));
        assert!(matches!(&p.inlines[1], Inline::Text(t, f) if t == " text" && f.font.as_deref() == Some("Arial")));
        assert_eq!(para(&flow.blocks[3]).list, Some(ListInfo { ordered: false, level: 0 }));
        let FBlock::Table(t) = &flow.blocks[4] else { panic!("expected a table") };
        assert_eq!(t.widths, [100.0, 150.0]);
        assert!(matches!(&para(&flow.blocks[5]).inlines[0], Inline::Text(t, f) if t == "link" && f.link.as_deref() == Some("http://a.b")));
        assert!(para(&flow.blocks[6]).page_break && !para(&flow.blocks[5]).page_break);
        // A4, with the left margin the section gives and the format's default right margin.
        assert_eq!((page.w, page.h, page.left, page.right), (595.3, 841.9, 72.0, 90.0));
        let doc = import(&sample()).unwrap();
        assert_eq!(doc.last_section.page_w, 595.3);
        assert!(doc.plain_text(wordcraft_doc::StoryRef::Body).contains("Plain bold café"));
    }

    #[test]
    fn rows_with_different_cells_share_one_grid() {
        let cell = |text: &str| Cell {
            blocks: vec![FBlock::Para(Para { inlines: vec![Inline::Text(text.into(), Fmt::default())], ..Default::default() })],
            ..Default::default()
        };
        let mut rows =
            vec![(vec![cell("a"), cell("b"), cell("c")], vec![0.0, 100.0, 200.0, 300.0]), (vec![cell("wide"), cell("d")], vec![0.0, 200.3, 300.0])];
        assert_eq!(grid(&mut rows), [100.0, 100.0, 100.0]);
        assert_eq!(rows[1].0.iter().map(|c| c.colspan).collect::<Vec<_>>(), [2, 1]);
        assert_eq!(rows[0].0.iter().map(|c| c.colspan).collect::<Vec<_>>(), [1, 1, 1]);
    }

    #[test]
    fn says_why_a_file_cannot_be_opened() {
        let mut d = Doc::new();
        d.run("secret", &[]).end('\r', 0, &[]);
        d.flags |= 0x0100;
        assert!(parse(&d.build()).unwrap_err().contains("password-protected"));
        // A password-protected .docx is a compound file with the package encrypted inside.
        let ooxml = cfb(&[("EncryptionInfo", vec![0; 64]), ("EncryptedPackage", vec![0; 5000])]);
        assert!(parse(&ooxml).unwrap_err().contains("password-protected"));
        let mut d = Doc::new();
        d.run("old", &[]).end('\r', 0, &[]);
        d.version = 104;
        assert!(parse(&d.build()).unwrap_err().contains("Word 95"));
        assert!(parse(b"plain text").is_err());
        assert!(parse(&cfb(&[("Other", vec![1, 2, 3])])).unwrap_err().contains("not a Word document"));
        assert!(parse(&CFB_MAGIC).is_err());
    }

    /// A file made to be slow: long style chains, a character style named over and over, and
    /// thousands of paragraphs. Styles are worked out once, so this stays quick.
    #[test]
    fn crafted_style_sheets_stay_fast() {
        let mut st = Styles::default();
        let chpx: Vec<u8> = (0..1000).flat_map(|_| sprm(0x4A30, &[15, 0])).collect();
        for i in 0..16u16 {
            st.stds.push(Some(Std { sti: 0, base: i.saturating_sub(1), papx: Vec::new(), chpx: chpx.clone() }));
        }
        let all = 0..st.stds.len() as u16;
        st.char_chains = all.clone().map(|i| st.chain(i).iter().flat_map(|s| s.chpx.iter().copied()).take(MAX_STYLE_PROPS).collect()).collect();
        st.chps = all.map(|i| st.resolve_chp(i)).collect();
        let start = std::time::Instant::now();
        for _ in 0..100_000 {
            let base = st.chp(15);
            let mut chp = base.clone();
            st.apply_chp(&mut chp, &chpx[..252], &base, 0);
        }
        assert!(start.elapsed().as_secs() < 20, "{:?}", start.elapsed());
    }

    /// Crash regression: every offset, count and chain in the file is input. Truncated and
    /// corrupted files may fail or come out garbled, but never panic or hang.
    #[test]
    fn damaged_files_never_panic() {
        let good = sample();
        for len in (0..good.len()).step_by(61) {
            let _ = parse(&good[..len]);
        }
        for at in (0..good.len()).step_by(5) {
            for value in [0x00, 0x01, 0x7F, 0xFF] {
                let mut bad = good.clone();
                bad[at] = value;
                let _ = import(&bad);
            }
        }
        // Whole fields at their extremes.
        for at in (0..good.len().saturating_sub(4)).step_by(4) {
            let mut bad = good.clone();
            bad[at..at + 4].copy_from_slice(&[0xFF; 4]);
            let _ = parse(&bad);
            bad[at..at + 4].copy_from_slice(&[0xFE, 0xFF, 0xFF, 0x7F]);
            let _ = parse(&bad);
        }
    }
}

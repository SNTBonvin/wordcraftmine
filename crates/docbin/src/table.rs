//! Tables: the table sprms (sgc 5) carried by a row-end (TTP) mark's grpprl, and the
//! assembly of cell-ended paragraphs into [`Table`] blocks ([MS-DOC] §2.6.3, §2.4.3).

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::Arc;

use wordcraft_doc::Rgb;
use wordcraft_doc::para::Paragraph;
use wordcraft_doc::props::{Align, Border, BorderStyle, Borders, CellProps, HeightRule, RowProps, TableProps, VAlign, VMerge};
use wordcraft_doc::table::{Cell, Row, Table};
use wordcraft_doc::{Block, Blocks, para_block};

use crate::fmt;
use crate::sprm::{self, Prl};

// Paragraph sprms for table membership.
const P_IN_TABLE: u16 = 0x2416;
const P_TTP: u16 = 0x2417;
/// `sprmPItap`: table depth (we only model depth 1; deeper rows fall back to paragraphs).
const P_ITAP: u16 = 0x6649;

// Table sprms ([MS-DOC] §2.6.3).
const T_JC: u16 = 0x5400;
const T_JC_90: u16 = 0x548A;
const T_DXA_LEFT: u16 = 0x9601;
const T_DXA_GAP_HALF: u16 = 0x9602;
const T_TABLE_HEADER: u16 = 0x3404;
const T_TABLE_BORDERS_80: u16 = 0xD605;
const T_ROW_HEIGHT: u16 = 0x9407;
const T_DEF_TABLE: u16 = 0xD608;
const T_DEF_TABLE_SHD_80: u16 = 0xD609;
const T_CANT_SPLIT_90: u16 = 0x3403;
const T_MERGE: u16 = 0x5624;
const T_VERT_MERGE: u16 = 0xD62B;
const T_VERT_ALIGN: u16 = 0xD62C;
const T_SET_SHD: u16 = 0xD62D;
const T_CANT_SPLIT: u16 = 0x3466;
const T_CELL_WIDTH: u16 = 0xD635;

/// Most grid columns we build from a `sprmTDefTable` (the model's own cap is 63; the union
/// grid is merged down to it, and merged rows can sum spans slightly over, so clamp when
/// assembling).
const MAX_COLS: usize = 63;

/// What a paragraph mark says about its table context, from its PAPX.
#[derive(Default)]
pub(crate) struct RowInfo {
    pub(crate) in_table: bool,
    pub(crate) ttp: bool,
    /// The mark itself was 0x07 (a cell end), set by the walker.
    pub(crate) cell_mark: bool,
    /// Table depth from `sprmPItap` (0 or 1 supported; deeper tables degrade to paragraphs).
    pub(crate) depth: u32,
    /// Cell-edge positions (points) from `sprmTDefTable` — [MS-DOC] defines them per row,
    /// so each row keeps its own; ascending in well-formed rows, the first edge usually 0
    /// (the row origin). Rows without a `sprmTDefTable` keep this empty.
    pub(crate) edges: Vec<f32>,
    height: Option<f32>,
    height_rule: HeightRule,
    header: bool,
    cant_split: bool,
    align: Option<Align>,
    indent: Option<f32>,
    borders: Option<Borders>,
    /// Horizontal merges: (itcFirst, itcLim) cell-index ranges.
    merges: Vec<(u8, u8)>,
    /// Per-cell overrides keyed by (pre-merge) cell index.
    vmerge: Vec<(u8, VMerge)>,
    shading: Vec<(u8, Rgb)>,
    widths: Vec<(u8, f32)>,
    valign: Vec<(u8, VAlign)>,
}

/// Read the table context out of a paragraph mark's PAPX grpprl.
impl RowInfo {
    /// The row height from `sprmTDyaRowHeight`, for tests.
    #[cfg(test)]
    pub(crate) fn height(&self) -> Option<f32> {
        self.height
    }
}

pub(crate) fn decode(papx: &[u8]) -> RowInfo {
    let mut r = RowInfo::default();
    let mut gap_half: Option<i16> = None;
    let mut dxa_left: Option<i16> = None;
    for prl in sprm::iter(papx) {
        // The membership sprms are paragraph sprms (sgc 1); the rest here are table sprms
        // (sgc 5), so match opcodes directly.
        match prl.op {
            P_IN_TABLE => r.in_table = prl.operand.first().copied().unwrap_or(0) != 0,
            P_TTP => r.ttp = prl.operand.first().copied().unwrap_or(0) != 0,
            P_ITAP => r.depth = i32_of(&prl).unwrap_or(0).max(0) as u32,
            T_DEF_TABLE => r.edges = def_table_edges(prl.operand),
            T_TABLE_HEADER => r.header = prl.operand.first().copied().unwrap_or(0) != 0,
            T_CANT_SPLIT | T_CANT_SPLIT_90 => r.cant_split = prl.operand.first().copied().unwrap_or(0) != 0,
            T_ROW_HEIGHT => {
                if let Some(v) = s16(&prl) {
                    if v < 0 {
                        r.height_rule = HeightRule::Exact;
                        r.height = Some((-(v as f32) / 20.0).clamp(0.0, 1584.0));
                    } else {
                        r.height_rule = HeightRule::AtLeast;
                        r.height = Some((v as f32 / 20.0).clamp(0.0, 1584.0));
                    }
                }
            }
            T_JC | T_JC_90 => {
                r.align = match u16_of(&prl).unwrap_or(0) {
                    1 => Some(Align::Center),
                    2 => Some(Align::Right),
                    _ => Some(Align::Left),
                };
            }
            T_DXA_LEFT => dxa_left = s16(&prl),
            T_DXA_GAP_HALF => gap_half = s16(&prl),
            T_TABLE_BORDERS_80 => r.borders = table_borders(prl.operand),
            T_MERGE => {
                if let [a, b, ..] = prl.operand
                    && b > a
                {
                    r.merges.push((*a, *b));
                }
            }
            T_VERT_MERGE => {
                // cb, itc, flag: 1 = continues the merge from above, 3 = restarts it.
                if let [_, itc, flag, ..] = prl.operand {
                    let m = match flag {
                        3 => VMerge::Restart,
                        1 => VMerge::Continue,
                        _ => VMerge::None,
                    };
                    if m != VMerge::None {
                        r.vmerge.push((*itc, m));
                    }
                }
            }
            T_SET_SHD => {
                if let Some(rgb) = prl.operand.get(3..).and_then(shd_rgb)
                    && let [a, b, ..] = prl.operand.get(1..3).unwrap_or(&[])
                {
                    for c in *a..*b {
                        r.shading.push((c, rgb));
                    }
                }
            }
            T_DEF_TABLE_SHD_80 => {
                let cells = prl.operand.get(1..).unwrap_or(&[]);
                for (i, b) in cells.as_chunks::<2>().0.iter().take(MAX_COLS).enumerate() {
                    if let Some(rgb) = shd80_rgb(b) {
                        r.shading.push((i as u8, rgb));
                    }
                }
            }
            T_CELL_WIDTH => {
                // cb, itcFirst, itcLim, ftsWidth (3 = twips), wWidth.
                if let [a, b, fts, w0, w1, ..] = prl.operand
                    && *fts == 3
                {
                    let w = (i16::from_le_bytes([*w0, *w1]) as f32 / 20.0).clamp(0.0, 1584.0);
                    for c in *a..(*b).min(MAX_COLS as u8) {
                        r.widths.push((c, w));
                    }
                }
            }
            T_VERT_ALIGN => {
                if let [_, a, b, va, ..] = prl.operand {
                    let v = match va {
                        1 => VAlign::Center,
                        2 => VAlign::Bottom,
                        _ => VAlign::Top,
                    };
                    for c in *a..*b {
                        r.valign.push((c, v));
                    }
                }
            }
            _ => {}
        }
    }
    // The table indent is the row origin minus half the cell gap.
    if let Some(l) = dxa_left {
        let gap = gap_half.unwrap_or(0) as f32 / 20.0;
        r.indent = Some(((l as f32 / 20.0) - gap).clamp(-1584.0, 1584.0));
    }
    r
}

/// A finished paragraph plus the bookkeeping the block assembly needs.
pub(crate) struct ParaOut {
    pub(crate) para: Paragraph,
    /// CP just past the terminating mark.
    pub(crate) end_cp: u32,
    /// Table context from the mark's PAPX (empty default = not in a table).
    pub(crate) row: RowInfo,
}

/// Group consecutive in-table paragraphs into [`Table`] blocks; the rest pass through.
/// Rows that never close (or tables nested deeper than we model) degrade to paragraphs so
/// no text is lost.
pub(crate) fn assemble(paras: Vec<ParaOut>) -> Blocks {
    let mut out: Blocks = Vec::new();
    let mut rows: Vec<Row> = Vec::new();
    let mut cells: Vec<Cell> = Vec::new();
    let mut cell: Blocks = Vec::new();
    let mut all_edges: Vec<f32> = Vec::new();
    let mut props = TableProps::default();
    let mut open = false;
    let mut depth_seen = 0u32;

    fn flush_cell(cells: &mut Vec<Cell>, cell: &mut Blocks) {
        let mut blocks = std::mem::take(cell);
        if blocks.is_empty() {
            blocks.push(para_block(Paragraph::new()));
        }
        cells.push(Cell { props: CellProps { span: 1, ..Default::default() }, blocks, controls: Default::default() });
    }
    fn build_row(mut cells: Vec<Cell>, info: &RowInfo) -> Row {
        // Per-cell overrides first (sprm cell indices count pre-merge cells), then merges.
        for (i, c) in cells.iter_mut().enumerate() {
            let i = i as u8;
            if let Some((_, m)) = info.vmerge.iter().find(|(k, _)| *k == i) {
                c.props.vmerge = *m;
            }
            if let Some((_, w)) = info.widths.iter().find(|(k, _)| *k == i) {
                c.props.width = Some(*w);
            }
            if let Some((_, s)) = info.shading.iter().find(|(k, _)| *k == i) {
                c.props.shading = Some(*s);
            }
            if let Some((_, v)) = info.valign.iter().find(|(k, _)| *k == i) {
                c.props.valign = *v;
            }
        }
        for &(a, b) in info.merges.iter().take(MAX_COLS) {
            let a = a as usize;
            let b = (b as usize).min(cells.len());
            if a < b && b <= cells.len() && b - a > 1 {
                let removed: Vec<Cell> = cells.drain(a + 1..b).collect();
                if let Some(first) = cells.get_mut(a) {
                    first.props.span = ((b - a) as u32).min(63);
                    for c in removed {
                        first.blocks.extend(c.blocks.into_iter().filter(|x| x.as_para().is_none_or(|p| !p.is_empty())));
                    }
                    if first.blocks.len() > 1 && first.blocks.first().and_then(|x| x.as_para()).is_some_and(Paragraph::is_empty) {
                        first.blocks.remove(0);
                    }
                }
            }
        }
        // Widths from this row's own edges: cell spans reconstruct the pre-merge column
        // indices, so a merged cell spans the columns its marks covered. Cells past the
        // row's edges (or rows without any) keep `None` for the flush-time grid fallback.
        let mut col = 0usize;
        for c in cells.iter_mut() {
            let span = (c.props.span.max(1)) as usize;
            if c.props.width.is_none() {
                c.props.width = edge_width(&info.edges, col, span);
            }
            col += span;
        }
        Row {
            props: RowProps {
                height: info.height,
                height_rule: info.height_rule,
                header: info.header,
                cant_split: info.cant_split,
                fmt_change: None,
            },
            cells,
            controls: Default::default(),
        }
    }
    fn flush_table(
        out: &mut Blocks,
        rows: &mut Vec<Row>,
        cells: &mut Vec<Cell>,
        cell: &mut Blocks,
        all_edges: &mut Vec<f32>,
        props: &TableProps,
        nested: bool,
    ) {
        // One grid from every row's edges, computed before anything can return so the
        // accumulator never leaks into the next table.
        let grid = union_widths(all_edges);
        if !cells.is_empty() || !cell.is_empty() {
            // A stray trailing row (no TTP of its own): its cells have no edges of their own.
            flush_cell(cells, cell);
            rows.push(build_row(std::mem::take(cells), &RowInfo::default()));
        }
        if rows.is_empty() {
            return;
        }
        // Cells their own row couldn't size (stray rows, more marks than edges) take the
        // union grid's columns.
        for r in rows.iter_mut() {
            for (i, c) in r.cells.iter_mut().enumerate() {
                if c.props.width.is_none() && c.props.span == 1 {
                    c.props.width = grid.get(i).copied();
                }
            }
        }
        if nested {
            // Unclosed or nested deeper than we model: keep the text, drop the table shape.
            for r in rows.iter() {
                for c in &r.cells {
                    out.extend(c.blocks.iter().cloned());
                }
            }
        } else {
            out.push(Arc::new(Block::Table(Table { props: props.clone(), grid, rows: std::mem::take(rows), controls: Default::default() })));
        }
        rows.clear();
    }

    for p in paras {
        let info = p.row;
        let is_table = info.in_table || info.ttp || info.cell_mark;
        if !is_table {
            if open {
                flush_table(&mut out, &mut rows, &mut cells, &mut cell, &mut all_edges, &props, depth_seen > 1);
                open = false;
                depth_seen = 0;
            }
            out.push(para_block(p.para));
            continue;
        }
        if !open {
            open = true;
        }
        depth_seen = depth_seen.max(info.depth);
        if info.borders.is_some() && props.borders.is_none() {
            props.borders = info.borders;
        }
        if props.align.is_none() {
            props.align = info.align;
        }
        if props.indent.is_none() {
            props.indent = info.indent;
        }
        if info.cell_mark {
            if !p.para.text.is_empty() {
                cell.push(para_block(p.para));
            }
            flush_cell(&mut cells, &mut cell);
            if info.ttp {
                // The row's own edges feed the table grid.
                all_edges.extend_from_slice(&info.edges);
                let row = build_row(std::mem::take(&mut cells), &info);
                rows.push(row);
            }
        } else {
            cell.push(para_block(p.para));
        }
    }
    if open {
        flush_table(&mut out, &mut rows, &mut cells, &mut cell, &mut all_edges, &props, depth_seen > 1);
    }
    out
}

/// Cell-edge positions (points) from a `sprmTDefTable` operand: cb u16, itcMac u8, then
/// itcMac+1 twip boundaries followed by the TC80s (skipped). Edges are returned as stored:
/// per-row, in operand order; sorting and capping happen when the table grid is assembled.
fn def_table_edges(operand: &[u8]) -> Vec<f32> {
    let itc = operand.get(2).copied().unwrap_or(0) as usize;
    if itc == 0 {
        return Vec::new();
    }
    // itcMac is a u8, so this allocation stays tiny; the loop stops at the operand's end.
    let mut edges = Vec::with_capacity((itc + 1).min(256));
    for i in 0..=itc {
        match operand.get(3 + i * 2..3 + i * 2 + 2) {
            Some(b) => edges.push(i16::from_le_bytes([b[0], b[1]]) as f32 / 20.0),
            None => break,
        }
    }
    edges
}

/// A gap between two union edges, ordered by [`f32::total_cmp`] so it can key a binary
/// heap (`f32` itself is neither `Ord` nor `Eq`).
#[derive(Clone, Copy)]
struct Gap(f32);

impl PartialEq for Gap {
    fn eq(&self, other: &Self) -> bool {
        self.0.total_cmp(&other.0) == std::cmp::Ordering::Equal
    }
}

impl Eq for Gap {}

impl Ord for Gap {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl PartialOrd for Gap {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// The width (points) of the `span` columns from `col` in a row's edges, or `None` when the
/// edges don't reach that far. Edges are file data and may be unsorted or negative, so each
/// column clamps instead of ever going below zero.
fn edge_width(edges: &[f32], col: usize, span: usize) -> Option<f32> {
    let last = col.checked_add(span)?;
    if edges.len() <= last {
        return None;
    }
    let mut w = 0.0;
    for k in col..last {
        w += (edges[k + 1] - edges[k]).max(0.0);
    }
    Some(w.clamp(0.0, 1584.0))
}

/// One grid from every row's edges: sorted and de-duplicated, then the closest adjacent
/// edges merged (the boundary between them dropped) until at most `MAX_COLS` columns
/// remain. Returns the column widths; consumes the accumulator.
///
/// Edges are i16 twips, so after de-duplication there are at most 65 536 of them; the
/// merge therefore runs through a revalidating binary heap (closest-current gap first)
/// instead of repeated linear scans, keeping a hostile table that claims thousands of
/// distinct boundaries at `n log n`.
fn union_widths(all: &mut Vec<f32>) -> Vec<f32> {
    let mut edges = std::mem::take(all);
    if edges.len() < 2 {
        return Vec::new();
    }
    edges.sort_by(f32::total_cmp);
    edges.dedup();
    // Doubly linked list of alive edges; edge 0 (the origin) is never queued for removal,
    // so its `prev` slot is never read.
    let n = edges.len();
    let mut prev: Vec<usize> = (0..n).map(|i| i.saturating_sub(1)).collect();
    let mut next: Vec<usize> = (1..n).chain([usize::MAX]).collect();
    let mut alive = vec![true; n];
    // Gap of edge i to its current previous alive edge, smallest first (index breaks ties).
    let mut gaps: BinaryHeap<Reverse<(Gap, usize)>> = (1..n).map(|i| Reverse((Gap(edges[i] - edges[i - 1]), i))).collect();
    let mut count = n;
    while count > MAX_COLS + 1 {
        let Some(Reverse((Gap(g), i))) = gaps.pop() else { break };
        if !alive[i] {
            continue;
        }
        // Neighbours may have merged since this gap was queued: revalidate, or requeue the
        // grown gap (each index's gap only ever grows, so this terminates).
        let p = prev[i];
        let cur = edges[i] - edges[p];
        if cur != g {
            gaps.push(Reverse((Gap(cur), i)));
            continue;
        }
        alive[i] = false;
        count -= 1;
        let nx = next[i];
        if nx != usize::MAX {
            prev[nx] = p;
            next[p] = nx;
            gaps.push(Reverse((Gap(edges[nx] - edges[p]), nx)));
        } else {
            next[p] = usize::MAX;
        }
    }
    let kept: Vec<usize> = (0..n).filter(|&i| alive[i]).collect();
    kept.windows(2).map(|w| (edges[w[1]] - edges[w[0]]).max(0.0)).collect()
}

/// Table borders from `sprmTTableBorders80`: cb (0x18) then six Brc80s in the order top,
/// left, bottom, right, inside-horizontal, inside-vertical.
fn table_borders(operand: &[u8]) -> Option<Borders> {
    let b = |i: usize| operand.get(1 + i * 4..1 + i * 4 + 4).and_then(brc80);
    Some(Borders { top: b(0), left: b(1), bottom: b(2), right: b(3), between: b(4), inside_v: b(5) })
}

/// One Brc80: width in 1/8 pt, type, palette colour, space/flags byte.
fn brc80(b: &[u8]) -> Option<Border> {
    let [w, ty, ico, flags] = b else { return None };
    let style = match ty {
        0x00 => BorderStyle::None,
        0x02 | 0x15 => BorderStyle::Thick,
        0x03 => BorderStyle::Double,
        0x04 | 0x05 => BorderStyle::Dotted,
        0x06 | 0x07 => BorderStyle::Dashed,
        0x08 | 0x09 => BorderStyle::DotDash,
        0x0A | 0x18 => BorderStyle::Wave,
        0x11 => BorderStyle::Triple,
        _ => BorderStyle::Single,
    };
    if *ty == 0 && *w == 0 {
        return None;
    }
    Some(Border { style, width: (*w as f32 / 8.0).clamp(0.25, 12.0), color: fmt::ico(*ico), space: (flags & 0x1F) as f32 })
}

/// Background colour from a 10-byte `Shd`: cvFore, cvBack (COLORREFs), ipat. Solid patterns
/// show the foreground; other patterns are approximated by their background.
fn shd_rgb(b: &[u8]) -> Option<Rgb> {
    let cv = |b: &[u8]| -> Option<Rgb> {
        match b {
            [_, _, _, 0xFF] => None, // fAuto
            [r, g, bl, _] => Some(Rgb(*r, *g, *bl)),
            _ => None,
        }
    };
    let ipat = b.get(8..10).map(|x| u16::from_le_bytes([x[0], x[1]]))?;
    match ipat {
        0 | 0xFFFF => None,
        1 => cv(b.get(0..4)?),
        _ => cv(b.get(4..8)?),
    }
}

/// Background colour from a 2-byte `Shd80` (icoFore, icoBack, ipat bit-fields).
fn shd80_rgb(b: &[u8]) -> Option<Rgb> {
    let v = u16::from_le_bytes([*b.first()?, *b.get(1)?]);
    let fore = v & 0x1F;
    let back = (v >> 5) & 0x1F;
    let ipat = v >> 10;
    if ipat == 0 || (fore == 0x1F && back == 0x1F) {
        return None;
    }
    fmt::ico(if ipat == 1 { fore as u8 } else { back as u8 })
}

fn u16_of(p: &Prl) -> Option<u16> {
    match p.operand {
        [b0, b1, ..] => Some(u16::from_le_bytes([*b0, *b1])),
        _ => None,
    }
}

fn s16(p: &Prl) -> Option<i16> {
    u16_of(p).map(|v| v as i16)
}

fn i32_of(p: &Prl) -> Option<i32> {
    match p.operand {
        [b0, b1, b2, b3, ..] => Some(i32::from_le_bytes([*b0, *b1, *b2, *b3])),
        _ => None,
    }
}

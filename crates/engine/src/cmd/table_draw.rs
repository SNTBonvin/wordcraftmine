//! Table Layout › Draw: Draw Table and Eraser. A pen stroke on a page either draws a new one-cell
//! table (a rectangle outside any table) or splits the cells a straight line crosses; the eraser
//! removes the border it's clicked on, or every border a rectangle dragged with it crosses: the
//! two cells beside one merge, or, when they can't make one cell, the border is hidden.
//! Coordinates are page points, like the layout's; the app turns the pen and eraser on as canvas
//! modes and sends each finished stroke, drag or click here, so every one is one undo step.

use serde_json::Value;
use wordcraft_doc::props::{Border, Borders, HeightRule, VMerge};
use wordcraft_doc::table::{MAX_COLS, MAX_ROWS};
use wordcraft_doc::{Block, Cell, Path, Pos, StoryRef, Table, para_block};
use wordcraft_geom::Rect;
use wordcraft_layout::{DocLayout, Placed};

use super::sel_result;
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

/// A drag shorter than this (points) both ways isn't a rectangle.
pub const MIN_RECT: f32 = 12.0;
/// How near (points) the eraser has to be to a border.
pub const ERASER_REACH: f32 = 4.0;
/// A line closer than this (points) to an existing border lands on it.
const SNAP: f32 = 3.0;
/// The shortest stroke (points) that counts as a line.
const MIN_LINE: f32 = 6.0;
/// An eraser drag's rectangle takes a border whose line is this near (points) and which it
/// crosses this far or more from the border's ends.
const DRAG_SLACK: f32 = 2.0;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("table.draw", "Draw Table", "Insert › Tables", draw).params(
            r#"{"page": n, "x0": pt, "y0": pt, "x1": pt, "y1": pt, "story"?: StoryRef} (a pen stroke in page points, 0-based page: a rectangle at least 12 pt each way outside any table inserts a one-cell table of that size at the paragraph under its start; a straight vertical or horizontal line inside a table splits the cells it crosses there. In the app the button turns the pen on.)"#,
        ),
        CommandSpec::new("table.eraser", "Eraser", "Table Layout › Draw", eraser).params(
            r#"{"page": n, "x": pt, "y": pt} or {"page": n, "x0": pt, "y0": pt, "x1": pt, "y1": pt} (removes the border within 4 pt of the point, or, as a drag does in Word, every one the rectangle crosses, in one step; a crossing at a border's end doesn't count. The table's edge is hidden. The cells beside a border merge when they make one rectangle, cells merged down the same rows too; otherwise the border is hidden on both sides, all along both cells. In the app the button turns the eraser on.)"#,
        ),
    ]
}

/// What a pen stroke does, judged from the layout (also the app's live preview).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stroke {
    /// Draw a new table in this rectangle (page points).
    Rect(Rect),
    /// Split the cells the vertical line at `x` crosses between `y0` and `y1`.
    Vertical { x: f32, y0: f32, y1: f32 },
    /// Split the cells the horizontal line at `y` crosses between `x0` and `x1`.
    Horizontal { y: f32, x0: f32, x1: f32 },
}

/// A table cell's area on a page.
#[derive(Clone, Debug)]
struct CellBox {
    rect: Rect,
    table: Path,
    story: StoryRef,
    row: usize,
    cell: usize,
    /// A vertical merge's continuation, under its merged cell's box.
    covered: bool,
}

fn cell_boxes(l: &DocLayout, page: usize) -> Vec<CellBox> {
    let Some(pg) = l.pages.get(page) else { return Vec::new() };
    pg.items
        .iter()
        .filter_map(|it| match it {
            Placed::Cell { rect, table, row, cell, story, covered, .. } => {
                Some(CellBox { rect: *rect, table: table.clone(), story: *story, row: *row, cell: *cell, covered: *covered })
            }
            _ => None,
        })
        .collect()
}

fn inside(r: &Rect, x: f32, y: f32) -> bool {
    x >= r.x && x <= r.right() && y >= r.y && y <= r.bottom()
}

/// The innermost table with a cell under the point.
fn table_at(boxes: &[CellBox], x: f32, y: f32) -> Option<(StoryRef, Path)> {
    boxes.iter().filter(|b| inside(&b.rect, x, y)).max_by_key(|b| b.table.0.len()).map(|b| (b.story, b.table.clone()))
}

/// Classify a pen stroke: a straight line inside a table, a rectangle outside one, else `None`.
pub fn stroke(l: &DocLayout, page: usize, x0: f32, y0: f32, x1: f32, y1: f32) -> Option<Stroke> {
    if ![x0, y0, x1, y1].iter().all(|v| v.is_finite()) {
        return None;
    }
    let (dx, dy) = ((x1 - x0).abs(), (y1 - y0).abs());
    let boxes = cell_boxes(l, page);
    let (mx, my) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    if table_at(&boxes, x0, y0).is_some() || table_at(&boxes, mx, my).is_some() {
        if dy >= dx && dy >= MIN_LINE && dx <= MIN_RECT.max(dy * 0.2) {
            return Some(Stroke::Vertical { x: mx, y0: y0.min(y1), y1: y0.max(y1) });
        }
        if dx > dy && dx >= MIN_LINE && dy <= MIN_RECT.max(dx * 0.2) {
            return Some(Stroke::Horizontal { y: my, x0: x0.min(x1), x1: x0.max(x1) });
        }
        return None;
    }
    (dx >= MIN_RECT && dy >= MIN_RECT).then(|| Stroke::Rect(Rect::new(x0.min(x1), y0.min(y1), dx, dy)))
}

fn page_param(v: &Value) -> Result<usize, CmdError> {
    p::u64(v, "page").map(|n| usize::try_from(n).unwrap_or(usize::MAX)).ok_or_else(|| CmdError::Params("`page` (number) is required".into()))
}

fn draw(s: &mut Session, v: &Value) -> CmdResult {
    let page = page_param(v)?;
    let (x0, y0, x1, y1) = (p::req_f32(v, "x0")?, p::req_f32(v, "y0")?, p::req_f32(v, "x1")?, p::req_f32(v, "y1")?);
    let l = s.layout();
    if l.pages.get(page).is_none() {
        return Err(CmdError::Params(format!("no page {page}")));
    }
    match stroke(&l, page, x0, y0, x1, y1) {
        Some(Stroke::Rect(r)) => draw_table(s, v, &l, page, (x0, y0), r),
        Some(Stroke::Vertical { x, y0, y1 }) => split_columns(s, &l, page, x, (y0, y1)),
        Some(Stroke::Horizontal { y, x0, x1 }) => split_rows(s, &l, page, y, (x0, x1)),
        None => Err(CmdError::Failed(
            "draw a rectangle (at least 12 pt each way) outside a table, or a straight horizontal or vertical line across table cells".into(),
        )),
    }
}

/// A one-cell table the size of `r`, at the paragraph under the stroke's start.
fn draw_table(s: &mut Session, v: &Value, l: &DocLayout, page: usize, start: (f32, f32), r: Rect) -> CmdResult {
    let story = super::story_param(s, v);
    let pos = l.hit(page, start.0, start.1, story).ok_or_else(|| CmdError::Failed("there's no text to put a table at there".into()))?;
    let pos = s.doc.clamp(&pos);
    if pos.path.cell().is_some() {
        return Err(CmdError::Failed("draw new tables outside a table; inside one, draw a line to split cells".into()));
    }
    let len = s.doc.para(pos.story, &pos.path).map(|q| q.len()).ok_or_else(|| CmdError::Failed("there's no paragraph there".into()))?;
    let tw = super::page::sect(s).text_width().max(18.0);
    let w = r.w.clamp(18.0, tw);
    let mut t = Table::new(1, 1, w);
    t.props.width = Some(w);
    let body_x = l.pages.get(page).map_or(0.0, |pg| pg.body.x);
    let indent = r.x - body_x;
    if indent > 1.0 {
        t.props.indent = Some(indent.min((tw - w).max(0.0)));
    }
    if let Some(row) = t.rows.first_mut() {
        row.props.height = Some(r.h.clamp(4.0, 1584.0));
        row.props.height_rule = HeightRule::AtLeast;
    }
    // Before the paragraph when the stroke starts at its beginning (or it's empty), else after it.
    let at = if pos.off == 0 || len == 0 { pos.path.clone() } else { s.doc.split_paragraph(&Pos { off: len, ..pos.clone() })?.path };
    s.doc.insert_block(pos.story, &at, Block::Table(t))?;
    let mut first = at.0.clone();
    first.extend([0, 0, 0]);
    s.sel = Selection::caret(Pos { story: pos.story, path: Path(first), off: 0 });
    sel_result(s)
}

/// The cells of one table a stroke crosses, as (row, cell) with their areas, and that table.
fn crossed(boxes: &[CellBox], at: (f32, f32), hit: impl Fn(&Rect) -> bool) -> Option<(StoryRef, Path, Vec<(usize, usize, Rect)>)> {
    let (story, table) = table_at(boxes, at.0, at.1)
        .or_else(|| boxes.iter().filter(|b| hit(&b.rect)).max_by_key(|b| b.table.0.len()).map(|b| (b.story, b.table.clone())))?;
    let mut cells: Vec<(usize, usize, Rect)> =
        boxes.iter().filter(|b| b.story == story && b.table == table && hit(&b.rect)).map(|b| (b.row, b.cell, b.rect)).collect();
    cells.sort_by_key(|c| (c.0, c.1));
    cells.dedup_by_key(|c| (c.0, c.1));
    Some((story, table, cells))
}

/// Grid column offsets: `cum[i]` is where grid column `i` starts.
fn offsets(t: &Table) -> Vec<f32> {
    let mut cum = Vec::with_capacity(t.grid.len() + 1);
    let mut x = 0.0f32;
    cum.push(0.0);
    for w in &t.grid {
        x += w.max(0.0);
        cum.push(x);
    }
    cum
}

/// The rows of the vertically merged group cell `c` of row `r` belongs to (just `r` when none).
fn vgroup(t: &Table, r: usize, c: usize) -> (usize, usize) {
    let g = t.grid_col(r, c);
    let starts_at = |row: usize| t.cell_at_grid(row, g).filter(|&ci| t.grid_col(row, ci) == g).and_then(|ci| t.rows.get(row)?.cells.get(ci));
    let Some(cell) = starts_at(r) else { return (r, r) };
    if cell.props.vmerge == VMerge::None {
        return (r, r);
    }
    let mut r0 = r;
    while r0 > 0 && starts_at(r0).is_some_and(|x| x.props.vmerge == VMerge::Continue) {
        r0 -= 1;
    }
    let mut r1 = r;
    while starts_at(r1 + 1).is_some_and(|x| x.props.vmerge == VMerge::Continue) {
        r1 += 1;
    }
    (r0, r1)
}

/// In row `row`, the cells ending and starting at grid line `line` (where grid column `line`
/// starts), when there's a border between two cells there.
fn beside_line(t: &Table, row: usize, line: usize) -> Option<(usize, usize)> {
    let n = t.rows.get(row)?.cells.len();
    let left = (0..n).find(|&ci| t.grid_col(row, ci + 1) == line)?;
    (left + 1 < n).then_some((left, left + 1))
}

/// A vertical line at page `x`: split each crossed cell into two columns there.
fn split_columns(s: &mut Session, l: &DocLayout, page: usize, x: f32, (ya, yb): (f32, f32)) -> CmdResult {
    let boxes = cell_boxes(l, page);
    let hit = |r: &Rect| {
        let overlap = yb.min(r.bottom()) - ya.max(r.y);
        x > r.x + SNAP && x < r.right() - SNAP && overlap >= r.h * 0.5
    };
    let Some((story, tp, cells)) = crossed(&boxes, (x, (ya + yb) / 2.0), hit) else {
        return Err(CmdError::Failed("the line doesn't cross a table cell".into()));
    };
    let Some(&(r0, c0, rect0)) = cells.first() else { return Err(CmdError::Failed("the line doesn't cross a table cell".into())) };
    let t = s.doc.table(story, &tp).ok_or_else(|| CmdError::Failed("the table isn't there any more".into()))?;
    // Where the line falls in the table's grid, measured in the first crossed cell.
    let cum = offsets(t);
    let a = t.grid_col(r0, c0);
    let span = t.rows.get(r0).and_then(|row| row.cells.get(c0)).map_or(1, Cell::span);
    let (start, end) = (cum.get(a).copied().unwrap_or(0.0), cum.get(a + span).or(cum.last()).copied().unwrap_or(0.0));
    let g = start + (x - rect0.x) / rect0.w.max(1.0) * (end - start);
    if !g.is_finite() || g <= 0.0 {
        return Err(CmdError::Failed("the line doesn't cross a table cell".into()));
    }
    let crossed_set: std::collections::BTreeSet<(usize, usize)> = cells.iter().map(|c| (c.0, c.1)).collect();
    // A vertically merged cell splits only as a whole.
    for &(r, c, _) in &cells {
        let (g0, g1) = vgroup(t, r, c);
        let gc = t.grid_col(r, c);
        for rr in g0..=g1 {
            let ci = t.cell_at_grid(rr, gc).unwrap_or(usize::MAX);
            if !crossed_set.contains(&(rr, ci)) {
                return Err(CmdError::Failed("the line crosses only part of a merged cell; draw it across the whole cell".into()));
            }
        }
    }
    // Snap to an existing grid line, else a new one inside grid column `k`.
    let existing = (1..t.grid.len()).find(|&j| cum.get(j).is_some_and(|c| (c - g).abs() < SNAP));
    let k = (0..t.grid.len()).find(|&k| cum.get(k + 1).is_some_and(|&e| g < e)).unwrap_or(t.grid.len().saturating_sub(1));
    if existing.is_none() && t.grid.len() >= MAX_COLS {
        return Err(CmdError::Failed(format!("a table can't have more than {MAX_COLS} columns")));
    }
    let t = s.doc.table_mut(story, &tp)?;
    let mut changed = false;
    match existing {
        Some(j) => {
            for &(r, c) in crossed_set.iter().rev() {
                let a = t.grid_col(r, c);
                let Some(row) = t.rows.get_mut(r) else { continue };
                let Some(cell) = row.cells.get_mut(c) else { continue };
                let span = cell.span();
                if a < j && j < a + span {
                    cell.props.span = (j - a) as u32;
                    let mut nc = Cell::empty();
                    nc.props = cell.props.clone();
                    nc.props.span = (a + span - j) as u32;
                    row.cells.insert(c + 1, nc);
                    changed = true;
                }
            }
        }
        None => {
            let ks = cum.get(k).copied().unwrap_or(0.0);
            let ke = cum.get(k + 1).copied().unwrap_or(ks);
            let (left, right) = (g - ks, ke - g);
            if left < SNAP || right < SNAP {
                return Err(CmdError::Failed("there's already a border there".into()));
            }
            if let Some(w) = t.grid.get_mut(k) {
                *w = left;
            }
            t.grid.insert((k + 1).min(t.grid.len()), right);
            for r in 0..t.rows.len() {
                let Some(ci) = t.cell_at_grid(r, k) else { continue };
                let a = t.grid_col(r, ci);
                let Some(row) = t.rows.get_mut(r) else { continue };
                let Some(cell) = row.cells.get_mut(ci) else { continue };
                if crossed_set.contains(&(r, ci)) {
                    let span = cell.span();
                    cell.props.span = (k + 1 - a) as u32;
                    let mut nc = Cell::empty();
                    nc.props = cell.props.clone();
                    nc.props.span = (a + span - k) as u32;
                    row.cells.insert(ci + 1, nc);
                } else {
                    cell.props.span = cell.props.span.saturating_add(1);
                }
            }
            changed = true;
        }
    }
    if !changed {
        return Err(CmdError::Failed("there's already a border there".into()));
    }
    super::table::sync_cell_widths(t);
    finish(s, story, &tp, r0, c0 + 1)
}

/// A horizontal line at page `y`: split the row it crosses there. Crossed cells become two;
/// the row's other cells span both rows (a vertical merge).
fn split_rows(s: &mut Session, l: &DocLayout, page: usize, y: f32, (xa, xb): (f32, f32)) -> CmdResult {
    let boxes = cell_boxes(l, page);
    let hit = |r: &Rect| {
        let overlap = xb.min(r.right()) - xa.max(r.x);
        y > r.y + SNAP && y < r.bottom() - SNAP && overlap >= r.w * 0.5
    };
    let Some((story, tp, cells)) = crossed(&boxes, ((xa + xb) / 2.0, y), hit) else {
        return Err(CmdError::Failed("the line doesn't cross a table cell".into()));
    };
    let Some(&(r, c0, _)) = cells.first() else { return Err(CmdError::Failed("the line doesn't cross a table cell".into())) };
    if cells.iter().any(|c| c.0 != r) {
        return Err(CmdError::Failed("draw the line within one row".into()));
    }
    // The row's extent on this page.
    let row_boxes: Vec<&Rect> = boxes.iter().filter(|b| b.story == story && b.table == tp && b.row == r).map(|b| &b.rect).collect();
    let top = row_boxes.iter().map(|b| b.y).fold(f32::INFINITY, f32::min);
    let bottom = row_boxes.iter().map(|b| b.bottom()).fold(f32::NEG_INFINITY, f32::max);
    let t = s.doc.table(story, &tp).ok_or_else(|| CmdError::Failed("the table isn't there any more".into()))?;
    if t.rows.len() >= MAX_ROWS {
        return Err(CmdError::Failed("the table has too many rows".into()));
    }
    let crossed_set: std::collections::BTreeSet<usize> = cells.iter().map(|c| c.1).collect();
    let merged = |ci: &usize| t.rows.get(r).and_then(|row| row.cells.get(*ci)).is_some_and(|c| c.props.vmerge != VMerge::None);
    if crossed_set.iter().any(merged) {
        return Err(CmdError::Failed("splitting a vertically merged cell across rows isn't supported; erase its merge first".into()));
    }
    let t = s.doc.table_mut(story, &tp)?;
    let Some(row) = t.rows.get_mut(r) else { return Err(CmdError::Failed("the row isn't there any more".into())) };
    let mut new_row = wordcraft_doc::Row { props: row.props.clone(), cells: Vec::with_capacity(row.cells.len()), controls: Default::default() };
    new_row.props.header = false;
    if top.is_finite() && bottom.is_finite() && bottom > top {
        let rule = if row.props.height_rule == HeightRule::Exact { HeightRule::Exact } else { HeightRule::AtLeast };
        row.props.height = Some((y - top).clamp(1.0, 1584.0));
        row.props.height_rule = rule;
        new_row.props.height = Some((bottom - y).clamp(1.0, 1584.0));
        new_row.props.height_rule = rule;
    }
    for (ci, cell) in row.cells.iter_mut().enumerate() {
        let mut nc = Cell::empty();
        nc.props = cell.props.clone();
        if crossed_set.contains(&ci) {
            nc.props.vmerge = VMerge::None;
        } else {
            if cell.props.vmerge == VMerge::None {
                cell.props.vmerge = VMerge::Restart;
            }
            nc.props.vmerge = VMerge::Continue;
        }
        new_row.cells.push(nc);
    }
    t.rows.insert(r + 1, new_row);
    finish(s, story, &tp, r + 1, c0)
}

/// Put the caret in cell (`r`, `c`) of the table (clamped) and re-lay its paragraphs out.
fn finish(s: &mut Session, story: StoryRef, tp: &Path, r: usize, c: usize) -> CmdResult {
    let Some(t) = s.doc.table(story, tp) else { return sel_result(s) };
    let r = r.min(t.rows.len().saturating_sub(1));
    let c = c.min(t.rows.get(r).map_or(0, |x| x.cells.len().saturating_sub(1)));
    let mut path = tp.0.clone();
    path.extend([r as u32, c as u32, 0]);
    s.sel = Selection::caret(s.doc.clamp(&Pos { story, path: Path(path), off: 0 }));
    super::table::touch_cells(s, tp)?;
    sel_result(s)
}

/// What the eraser removes: a border between two cells, with its ends (page points) for the
/// preview.
#[derive(Clone, Debug, PartialEq)]
pub struct EraseTarget {
    pub from: (f32, f32),
    pub to: (f32, f32),
    story: StoryRef,
    table: Path,
    /// (row, cell) of the cell above the border, or before it in its row (left of it, or right
    /// of it in a right-to-left table); on the table's edge, the cell inside it.
    first: (usize, usize),
    beyond: Beyond,
    across: bool,
}

/// What's on a border's other side.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Beyond {
    /// The cell (row, cell) below it, or after it in its row.
    Cell((usize, usize)),
    /// Nothing: it's the table's edge, on `first`'s right or bottom side (as laid out) when
    /// `far`, else on its left or top.
    Edge { far: bool },
}

/// A border between two cells of a table on a page: the cell left of or above it, the other
/// one, and whether it's vertical.
type Shared<'a> = (&'a CellBox, &'a CellBox, bool);

/// Every border between two cells on a page.
fn shared_borders(boxes: &[CellBox]) -> Vec<Shared<'_>> {
    let mut out = Vec::new();
    for a in boxes {
        let (ar, ab) = (a.rect.right(), a.rect.bottom());
        for b in boxes.iter().filter(|b| b.story == a.story && b.table == a.table) {
            // The cell on a's right in its row: the next one, or the one before it in a
            // right-to-left table.
            if b.row == a.row && b.cell.abs_diff(a.cell) == 1 && (b.rect.x - ar).abs() <= 1.5 {
                out.push((a, b, true));
            }
            // A cell of the next row starting at a's bottom, beside it.
            if b.row == a.row + 1 && (b.rect.y - ab).abs() <= 1.5 && b.rect.x < ar && b.rect.right() > a.rect.x {
                out.push((a, b, false));
            }
        }
    }
    out
}

/// The target for the border between `a` and `b` (see [`Shared`]).
fn target(a: &CellBox, b: &CellBox, across: bool) -> EraseTarget {
    let (ar, ab) = (a.rect.right(), a.rect.bottom());
    if across {
        // Both cells' full height: when they can't merge, the border is hidden all along them.
        let (first, second) = if b.cell < a.cell { (b, a) } else { (a, b) };
        EraseTarget {
            from: (ar, a.rect.y.min(b.rect.y)),
            to: (ar, ab.max(b.rect.bottom())),
            story: a.story,
            table: a.table.clone(),
            first: (first.row, first.cell),
            beyond: Beyond::Cell((second.row, second.cell)),
            across,
        }
    } else {
        EraseTarget {
            from: (a.rect.x.max(b.rect.x), ab),
            to: (ar.min(b.rect.right()), ab),
            story: a.story,
            table: a.table.clone(),
            first: (a.row, a.cell),
            beyond: Beyond::Cell((b.row, b.cell)),
            across,
        }
    }
}

/// The border nearest page point (`x`, `y`), within [`ERASER_REACH`], an inner table's on a tie:
/// between two cells, or, as in Word, on the table's edge.
pub fn erase_target(l: &DocLayout, page: usize, x: f32, y: f32) -> Option<EraseTarget> {
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    let boxes = cell_boxes(l, page);
    let mut best: Option<(f32, usize, EraseTarget)> = None;
    let mut offer = |d: f32, a: &CellBox, make: &dyn Fn() -> EraseTarget| {
        let depth = a.table.0.len();
        if d <= ERASER_REACH && best.as_ref().is_none_or(|(bd, bdepth, _)| d < bd - 0.01 || (d <= bd + 0.01 && depth > *bdepth)) {
            best = Some((d, depth, make()));
        }
    };
    for &(a, b, across) in &shared_borders(&boxes) {
        let (ar, ab) = (a.rect.right(), a.rect.bottom());
        let d = if across {
            if y < a.rect.y || y > ab {
                continue;
            }
            (x - ar).abs()
        } else {
            if x < a.rect.x.max(b.rect.x) || x > ar.min(b.rect.right()) {
                continue;
            }
            (y - ab).abs()
        };
        offer(d, a, &|| target(a, b, across));
    }
    for (a, across, far) in outer_edges(&boxes) {
        let (line, (e0, e1)) = side_line(a, across, far);
        let (along, off) = if across { (y, x) } else { (x, y) };
        if along >= e0 && along <= e1 {
            offer((off - line).abs(), a, &|| edge_target(a, across, far));
        }
    }
    best.map(|(_, _, e)| e)
}

/// Every side of a cell on a page that no other cell of its table touches, the table's edge
/// there: (cell, vertical, on its right or bottom). A merge's continuation has no top of its
/// own: its merged cell's box is over it.
fn outer_edges(boxes: &[CellBox]) -> Vec<(&CellBox, bool, bool)> {
    let near = |p: f32, q: f32| (p - q).abs() <= 1.5;
    let mut out = Vec::new();
    for a in boxes {
        let r = a.rect;
        let touched = |by: &dyn Fn(&CellBox) -> bool| boxes.iter().any(|b| b.story == a.story && b.table == a.table && !std::ptr::eq(b, a) && by(b));
        let beside = |b: &CellBox| b.rect.y < r.bottom() && b.rect.bottom() > r.y;
        let over = |b: &CellBox| b.rect.x < r.right() && b.rect.right() > r.x;
        if !touched(&|b| beside(b) && near(b.rect.right(), r.x)) {
            out.push((a, true, false));
        }
        if !touched(&|b| beside(b) && near(b.rect.x, r.right())) {
            out.push((a, true, true));
        }
        if !a.covered && !touched(&|b| over(b) && near(b.rect.bottom(), r.y)) {
            out.push((a, false, false));
        }
        if !touched(&|b| over(b) && near(b.rect.y, r.bottom())) {
            out.push((a, false, true));
        }
    }
    out
}

/// The line of a side of cell box `a` (vertical when `across`; its right or bottom one when
/// `far`) and its ends.
fn side_line(a: &CellBox, across: bool, far: bool) -> (f32, (f32, f32)) {
    let r = a.rect;
    match (across, far) {
        (true, false) => (r.x, (r.y, r.bottom())),
        (true, true) => (r.right(), (r.y, r.bottom())),
        (false, false) => (r.y, (r.x, r.right())),
        (false, true) => (r.bottom(), (r.x, r.right())),
    }
}

/// The target for the table's edge on a side of cell box `a` (see [`side_line`]).
fn edge_target(a: &CellBox, across: bool, far: bool) -> EraseTarget {
    let (line, (e0, e1)) = side_line(a, across, far);
    let (from, to) = if across { ((line, e0), (line, e1)) } else { ((e0, line), (e1, line)) };
    EraseTarget { from, to, story: a.story, table: a.table.clone(), first: (a.row, a.cell), beyond: Beyond::Edge { far }, across }
}

/// Every border that a rectangle dragged with the eraser from (`x0`, `y0`) to (`x1`, `y1`)
/// crosses, as in Word: between two cells, in the page's order, then on the table's edge. The
/// border's line is within [`DRAG_SLACK`] of the rectangle, which reaches over it at least that
/// far from its ends: a crossing at a border's end doesn't count, so a drag along one border
/// leaves the ones across it.
pub fn erase_targets(l: &DocLayout, page: usize, x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<EraseTarget> {
    if ![x0, y0, x1, y1].iter().all(|v| v.is_finite()) {
        return Vec::new();
    }
    let (left, right, top, bottom) = (x0.min(x1), x0.max(x1), y0.min(y1), y0.max(y1));
    // Whether it crosses a vertical (`across`) or horizontal border on `line` from `e0` to `e1`,
    // away from its ends (its middle, when it's shorter than that).
    let crosses = |across: bool, line: f32, (e0, e1): (f32, f32)| {
        let ((lo, hi), (along0, along1)) = if across { ((left, right), (top, bottom)) } else { ((top, bottom), (left, right)) };
        let mid = (e0 + e1) / 2.0;
        let (s0, s1) = if e1 - e0 >= 2.0 * DRAG_SLACK { (e0 + DRAG_SLACK, e1 - DRAG_SLACK) } else { (mid, mid) };
        line >= lo - DRAG_SLACK && line <= hi + DRAG_SLACK && along0 <= s1 && along1 >= s0
    };
    let boxes = cell_boxes(l, page);
    let mut found = Vec::new();
    for &(a, b, across) in &shared_borders(&boxes) {
        let (line, ends) = if across {
            (a.rect.right(), (a.rect.y.max(b.rect.y), a.rect.bottom().min(b.rect.bottom())))
        } else {
            (a.rect.bottom(), (a.rect.x.max(b.rect.x), a.rect.right().min(b.rect.right())))
        };
        if crosses(across, line, ends) {
            found.push(target(a, b, across));
        }
    }
    for (a, across, far) in outer_edges(&boxes) {
        let (line, ends) = side_line(a, across, far);
        if crosses(across, line, ends) {
            found.push(edge_target(a, across, far));
        }
    }
    found
}

/// A border the eraser removes, by its place in the table's grid: merging renumbers a row's
/// cells, so in a drag a cell index found before an earlier border was erased could name
/// another cell by the time its border's turn comes.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Line {
    /// The vertical border at grid line `at` in row `row`.
    Across { row: usize, at: usize },
    /// The horizontal border between two cells, each as (row, a grid column it covers).
    Below { upper: (usize, usize), lower: (usize, usize) },
    /// The table's edge on side `side` of the cell over grid column `g` in row `row`.
    Edge { row: usize, g: usize, side: Side },
}

impl Line {
    fn of(t: &Table, e: &EraseTarget) -> Option<Line> {
        let (r, c) = e.first;
        let span = t.rows.get(r)?.cells.get(c)?.span();
        let g = t.grid_col(r, c);
        Some(match (e.beyond, e.across) {
            (Beyond::Cell(_), true) => Line::Across { row: r, at: g + span },
            (Beyond::Cell((r2, c2)), false) => Line::Below { upper: (r, g), lower: (r2, t.grid_col(r2, c2)) },
            // As laid out: a right-to-left table's right edge is its cells' start (left) side.
            (Beyond::Edge { far }, true) => Line::Edge { row: r, g, side: if far != t.props.bidi_visual { Side::Right } else { Side::Left } },
            (Beyond::Edge { far }, false) => Line::Edge { row: r, g, side: if far { Side::Bottom } else { Side::Top } },
        })
    }

    /// Its turn among a drag's borders (see `eraser`).
    fn turn(&self) -> u8 {
        match self {
            Line::Across { .. } => 0,
            Line::Below { .. } => 1,
            Line::Edge { .. } => 2,
        }
    }
}

fn eraser(s: &mut Session, v: &Value) -> CmdResult {
    let page = page_param(v)?;
    // A dragged rectangle, or a click.
    let drag = v.get("x0").is_some();
    let (x0, y0) = if drag { (p::req_f32(v, "x0")?, p::req_f32(v, "y0")?) } else { (p::req_f32(v, "x")?, p::req_f32(v, "y")?) };
    let (x1, y1) = if drag { (p::req_f32(v, "x1")?, p::req_f32(v, "y1")?) } else { (x0, y0) };
    let l = s.layout();
    if l.pages.get(page).is_none() {
        return Err(CmdError::Params(format!("no page {page}")));
    }
    let targets = if drag { erase_targets(&l, page, x0, y0, x1, y1) } else { erase_target(&l, page, x0, y0).into_iter().collect() };
    // Every border by its place in the grid, before anything changes (see `Line`), each once.
    let mut lines: Vec<(StoryRef, Path, Line)> = Vec::new();
    for e in targets {
        if let Some(line) = s.doc.table(e.story, &e.table).and_then(|t| Line::of(t, &e))
            && !lines.iter().any(|(story, tp, ln)| *story == e.story && *tp == e.table && *ln == line)
        {
            lines.push((e.story, e.table, line));
        }
    }
    // Tables in cells first: merging an outer table's cells can move them. Then vertical borders
    // before horizontal ones, so a block of cells becomes one cell, as in Word: row by row, then
    // the rows down (a row merged before the cells below it wouldn't line up with them any more).
    // The table's edges last, hidden on the cells as merged.
    lines.sort_by_key(|(_, tp, line)| (std::cmp::Reverse(tp.0.len()), line.turn()));
    let mut changed: Vec<(StoryRef, Path, (usize, usize))> = Vec::new();
    let mut why = None;
    for (story, tp, line) in lines {
        let Ok(t) = s.doc.table_mut(story, &tp) else { continue };
        let done = match line {
            Line::Across { row, at } => erase_across(t, row, at),
            Line::Below { upper, lower } => erase_below(t, upper, lower),
            Line::Edge { row, g, side } => erase_edge(t, row, g, side),
        };
        match done {
            Ok(cell) => match changed.iter_mut().find(|c| c.0 == story && c.1 == tp) {
                Some(c) => c.2 = cell,
                None => changed.push((story, tp, cell)),
            },
            // A border gone with an earlier one of the drag (merged away), or hidden already, is
            // passed over.
            Err(e) => why = why.or(Some(e)),
        }
    }
    // Every changed table is laid out again; the caret goes to the last one's last changed cell.
    let Some((last, rest)) = changed.split_last() else {
        return Err(CmdError::Failed(why.unwrap_or("there's no border between two cells there").into()));
    };
    for (story, tp, (r, c)) in rest {
        finish(s, *story, tp, *r, *c)?;
    }
    let (story, tp, (r, c)) = last;
    finish(s, *story, tp, *r, *c)
}

/// The vertical border at grid line `at` in row `row`. As in Word, the cells beside it merge
/// when they span the same rows (merged down alike, or neither is), so they make one cell;
/// otherwise the border is hidden on both sides, all along both cells. The cell for the caret,
/// or why nothing changed.
fn erase_across(t: &mut Table, row: usize, at: usize) -> Result<(usize, usize), &'static str> {
    let (left, right) = beside_line(t, row, at).ok_or("those cells are already merged")?;
    let (rows_l, rows_r) = (vgroup(t, row, left), vgroup(t, row, right));
    let (r0, r1) = (rows_l.0.min(rows_r.0), rows_l.1.max(rows_r.1));
    let pairs: Vec<(usize, usize, usize)> = (r0..=r1).filter_map(|r| beside_line(t, r, at).map(|(a, b)| (r, a, b))).collect();
    if rows_l == rows_r && pairs.len() == r1 - r0 + 1 {
        for &(r, a, b) in &pairs {
            merge_across(t, r, a, b);
        }
        super::table::sync_cell_widths(t);
        return Ok((r0, pairs.first().map_or(left, |p| p.1)));
    }
    let mut hid = false;
    for &(r, a, b) in &pairs {
        hid |= hide(t, r, a, Side::Right);
        hid |= hide(t, r, b, Side::Left);
    }
    if hid { Ok((row, left)) } else { Err("that border is already erased") }
}

/// The horizontal border between the cell over grid column `upper.1` of row `upper.0` and the
/// one over `lower.1` of row `lower.0`. Cells that line up merge down (the lower one's text
/// moves up); otherwise, as in Word, the border is hidden on both sides. The cell for the caret,
/// or why nothing changed.
fn erase_below(t: &mut Table, upper: (usize, usize), lower: (usize, usize)) -> Result<(usize, usize), &'static str> {
    // A cell's index, first grid column, span and merge.
    let at = |t: &Table, (r, g): (usize, usize)| {
        let c = t.cell_at_grid(r, g)?;
        let cell = t.rows.get(r)?.cells.get(c)?;
        Some((c, t.grid_col(r, c), cell.span(), cell.props.vmerge))
    };
    let (Some((c, ga, sa, _)), Some((c2, gb, sb, vb))) = (at(t, upper), at(t, lower)) else {
        return Err("the cells aren't there any more");
    };
    let (r, r2) = (upper.0, lower.0);
    if vb == VMerge::Continue {
        return Err("those cells are already merged");
    }
    if ga != gb || sa != sb {
        let hid = (hide_all_along(t, r, c, Side::Bottom), hide_all_along(t, r2, c2, Side::Top));
        return if hid.0 || hid.1 { Ok((r, c)) } else { Err("that border is already erased") };
    }
    let (top, _) = vgroup(t, r, c);
    if let Some(a) = t.rows.get_mut(r).and_then(|row| row.cells.get_mut(c))
        && a.props.vmerge == VMerge::None
    {
        a.props.vmerge = VMerge::Restart;
    }
    let mut moved = Vec::new();
    if let Some(b) = t.rows.get_mut(r2).and_then(|row| row.cells.get_mut(c2)) {
        b.props.vmerge = VMerge::Continue;
        let blocks = std::mem::replace(&mut b.blocks, vec![para_block(wordcraft_doc::Paragraph::new())]);
        moved.extend(blocks.into_iter().filter(|x| x.as_para().is_none_or(|q| !q.is_empty())));
    }
    let tc = t.cell_at_grid(top, ga).unwrap_or(0);
    if !moved.is_empty()
        && let Some(dst) = t.rows.get_mut(top).and_then(|row| row.cells.get_mut(tc))
    {
        if dst.blocks.len() == 1 && dst.blocks.first().and_then(|b| b.as_para()).is_some_and(wordcraft_doc::Paragraph::is_empty) {
            dst.blocks.clear();
        }
        dst.blocks.extend(moved);
    }
    Ok((top, tc))
}

/// Merge cells `a..=b` of row `r` into `a` (`Table::merge_right`), which takes `b`'s right
/// side: a border hidden between them mustn't hide the merged cell's edge.
fn merge_across(t: &mut Table, r: usize, a: usize, b: usize) {
    let right = t.rows.get(r).and_then(|row| row.cells.get(b)).and_then(|c| c.props.borders).and_then(|bs| bs.right);
    t.merge_right(r, a, b);
    if let Some(cell) = t.rows.get_mut(r).and_then(|row| row.cells.get_mut(a))
        && (right.is_some() || cell.props.borders.is_some())
    {
        cell.props.borders.get_or_insert_with(Borders::default).right = right;
    }
}

/// The table's edge on side `side` of the cell over grid column `g` in row `row`: as in Word,
/// with nothing beyond it to merge with, it's hidden, in every row the cell is merged down. The
/// cell for the caret, or why nothing changed.
fn erase_edge(t: &mut Table, row: usize, g: usize, side: Side) -> Result<(usize, usize), &'static str> {
    let c = t.cell_at_grid(row, g).ok_or("the cells aren't there any more")?;
    if hide_all_along(t, row, c, side) { Ok((row, c)) } else { Err("that border is already erased") }
}

/// A side of a cell, as stored (left is the start side: the right one in a right-to-left table).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    fn of(self, b: &mut Borders) -> &mut Option<Border> {
        match self {
            Side::Left => &mut b.left,
            Side::Right => &mut b.right,
            Side::Top => &mut b.top,
            Side::Bottom => &mut b.bottom,
        }
    }
}

/// Hide side `side` of cell `c` of row `r`: a border of style none of its own (`nil` in the
/// file), so the table's or its style's border isn't drawn there either. `false` when it was
/// hidden already.
fn hide(t: &mut Table, r: usize, c: usize, side: Side) -> bool {
    let Some(cell) = t.rows.get_mut(r).and_then(|row| row.cells.get_mut(c)) else { return false };
    let edge = side.of(cell.props.borders.get_or_insert_with(Borders::default));
    if matches!(*edge, Some(b) if !b.is_visible()) {
        return false;
    }
    *edge = Some(Border::default());
    true
}

/// [`hide`] a side of cell `c` of row `r` in every row it's merged down, as Word does.
fn hide_all_along(t: &mut Table, r: usize, c: usize, side: Side) -> bool {
    let (g, (r0, r1)) = (t.grid_col(r, c), vgroup(t, r, c));
    let mut hid = false;
    for row in r0..=r1 {
        if let Some(ci) = t.cell_at_grid(row, g) {
            hid |= hide(t, row, ci, side);
        }
    }
    hid
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Each row's cell spans.
    fn shape(t: &Table) -> Value {
        json!(t.rows.iter().map(|r| r.cells.iter().map(|c| c.span()).collect::<Vec<_>>()).collect::<Vec<_>>())
    }

    fn first_table(s: &Session) -> Table {
        s.doc.body.iter().find_map(|b| b.as_table()).cloned().unwrap()
    }

    fn cell_rect(s: &mut Session, row: usize, cell: usize) -> Rect {
        s.layout().pages[0]
            .items
            .iter()
            .find_map(|it| match it {
                Placed::Cell { rect, row: r, cell: c, .. } if *r == row && *c == cell => Some(*rect),
                _ => None,
            })
            .unwrap()
    }

    /// A rectangle drawn on the page inserts a one-cell table of its size; undo takes it out.
    #[test]
    fn a_drawn_rectangle_inserts_a_table() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("document.setText", &json!({"text": "Above the table"})).unwrap();
        let before = s.doc.clone();
        let l = s.layout();
        let body = l.pages[0].body;
        let (x0, y0) = (body.x + 40.0, body.y + 2.0);
        s.run("table.draw", &json!({"page": 0, "x0": x0, "y0": y0, "x1": x0 + 200.0, "y1": y0 + 60.0})).unwrap();
        let t = first_table(&s);
        assert_eq!((t.rows.len(), t.rows[0].cells.len()), (1, 1));
        assert_eq!(t.props.width, Some(200.0));
        assert_eq!(t.rows[0].props.height, Some(60.0));
        assert!(t.props.indent.is_some_and(|i| (i - 40.0).abs() < 0.5));
        assert!(s.sel.focus.path.cell().is_some(), "the caret goes into the new cell");
        let r = cell_rect(&mut s, 0, 0);
        assert!((r.w - 200.0).abs() < 1.0 && r.h >= 59.5, "{r:?}");
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(s.doc.body, before.body, "one undo step");
        // Too small, or nowhere: errors, never panics.
        assert!(s.run("table.draw", &json!({"page": 0, "x0": x0, "y0": y0, "x1": x0 + 5.0, "y1": y0 + 5.0})).is_err());
        assert!(s.run("table.draw", &json!({"page": 9, "x0": 1, "y0": 1, "x1": 100, "y1": 100})).is_err());
        assert!(s.run("table.draw", &json!({})).is_err());
    }

    /// Erasing a horizontal border merges the cells into one laid-out cell (#640): the lower
    /// row's box is `covered`, the merged cell's box spans both rows, and erasing the border
    /// below the merge extends it.
    #[test]
    fn erasing_a_horizontal_border_lays_the_merge_out_as_one_cell() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("insert.table", &json!({"rows": 3, "cols": 2})).unwrap();
        let (a, b) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 1, 0));
        s.run("table.eraser", &json!({"page": 0, "x": a.x + a.w / 2.0, "y": a.bottom() + 0.5})).unwrap();
        let t = first_table(&s);
        assert_eq!((t.rows[0].cells[0].props.vmerge, t.rows[1].cells[0].props.vmerge), (VMerge::Restart, VMerge::Continue));
        let merged = cell_rect(&mut s, 0, 0);
        assert!((merged.bottom() - b.bottom()).abs() < 0.5 && merged.h > a.h * 1.9, "{merged:?}");
        let covered = s.layout().pages[0].items.iter().any(|it| matches!(it, Placed::Cell { row: 1, cell: 0, covered: true, .. }));
        assert!(covered, "the continuation row's box is covered");
        let hit = s.layout().cell_at(0, merged.x + 3.0, b.y + b.h / 2.0).map(|c| (c.2, c.3));
        assert_eq!(hit, Some((0, 0)), "a click in the lower half is in the merged cell");
        // The border below the merge: the merge grows to three rows.
        s.run("table.eraser", &json!({"page": 0, "x": merged.x + merged.w / 2.0, "y": merged.bottom() + 0.5})).unwrap();
        let t = first_table(&s);
        assert_eq!(t.rows[2].cells[0].props.vmerge, VMerge::Continue);
    }

    /// A vertical line through a cell splits it into two columns; a horizontal one splits a row;
    /// the eraser merges the cells back; each is one undo step.
    #[test]
    fn lines_split_cells_and_the_eraser_merges_them() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("insert.table", &json!({"rows": 2, "cols": 2})).unwrap();
        s.run("text.insert", &json!({"text": "A"})).unwrap();
        let start = first_table(&s);
        let r = cell_rect(&mut s, 0, 0);
        let x = r.x + r.w * 0.25;
        s.run("table.draw", &json!({"page": 0, "x0": x, "y0": r.y + 1.0, "x1": x + 1.0, "y1": r.bottom() - 1.0})).unwrap();
        let t = first_table(&s);
        assert_eq!(t.grid.len(), 3);
        assert_eq!(shape(&t), json!([[1, 1, 1], [2, 1]]), "the crossed cell splits, the other row's cell spans the new column");
        assert!((t.grid[0] - start.grid[0] * 0.25).abs() < 1.0, "{:?}", t.grid);
        let text = t.rows[0].cells[0].blocks.iter().filter_map(|b| b.as_para().map(|q| q.plain_text())).collect::<String>();
        assert_eq!(text, "A", "the text stays in the left part");
        // The eraser on the new border merges the two cells again.
        let r0 = cell_rect(&mut s, 0, 0);
        s.run("table.eraser", &json!({"page": 0, "x": r0.right() + 1.0, "y": r0.y + r0.h / 2.0})).unwrap();
        let t = first_table(&s);
        assert_eq!(shape(&t), json!([[2, 1], [2, 1]]));
        // A horizontal line across the second column's first cell splits its row.
        let r1 = cell_rect(&mut s, 0, 1);
        let y = r1.y + r1.h / 2.0;
        s.run("table.draw", &json!({"page": 0, "x0": r1.x + 1.0, "y0": y, "x1": r1.right() - 1.0, "y1": y})).unwrap();
        let t = first_table(&s);
        assert_eq!(t.rows.len(), 3);
        assert_eq!(t.rows[0].cells[0].props.vmerge, VMerge::Restart, "the uncrossed cell spans both rows");
        assert_eq!(t.rows[1].cells[0].props.vmerge, VMerge::Continue);
        assert_eq!(t.rows[1].cells[1].props.vmerge, VMerge::None);
        // Erasing that line merges the split cells (a vertical merge).
        let a = cell_rect(&mut s, 0, 1);
        s.run("table.eraser", &json!({"page": 0, "x": a.x + a.w / 2.0, "y": a.bottom() + 0.5})).unwrap();
        let t = first_table(&s);
        assert_eq!(t.rows[0].cells[1].props.vmerge, VMerge::Restart);
        assert_eq!(t.rows[1].cells[1].props.vmerge, VMerge::Continue);
        // Four steps, four undos back to the start.
        for _ in 0..4 {
            s.run("edit.undo", &json!({})).unwrap();
        }
        assert_eq!(first_table(&s), start);
        // Nothing near a border, or a hostile point: an error.
        let c = cell_rect(&mut s, 0, 0);
        assert!(s.run("table.eraser", &json!({"page": 0, "x": c.x + c.w / 2.0, "y": c.y + c.h / 2.0})).is_err());
        assert!(s.run("table.eraser", &json!({"page": 0, "x": 1e30, "y": -1e30})).is_err());
        assert_eq!(first_table(&s), start);
    }

    /// A `rows` × `cols` table whose cells say where they are: "A1", "B1"… (column, row).
    fn named_table(rows: usize, cols: usize) -> Session {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("insert.table", &json!({"rows": rows, "cols": cols})).unwrap();
        for i in 0..rows * cols {
            if i > 0 {
                s.run("text.tab", &json!({})).unwrap();
            }
            let name = format!("{}{}", char::from(b'A' + (i % cols) as u8), i / cols + 1);
            s.run("text.insert", &json!({"text": name})).unwrap();
        }
        s
    }

    /// Cell (`r`, `c`)'s paragraphs, joined with `|`.
    fn text(t: &Table, r: usize, c: usize) -> String {
        t.rows[r].cells[c].blocks.iter().filter_map(|b| b.as_para().map(|q| q.plain_text())).collect::<Vec<_>>().join("|")
    }

    fn erase(s: &mut Session, x: f32, y: f32) -> CmdResult {
        s.run("table.eraser", &json!({"page": 0, "x": x, "y": y}))
    }

    fn drag(s: &mut Session, (x0, y0): (f32, f32), (x1, y1): (f32, f32)) -> CmdResult {
        s.run("table.eraser", &json!({"page": 0, "x0": x0, "y0": y0, "x1": x1, "y1": y1}))
    }

    /// Whether a visible border is drawn through page point (`x`, `y`).
    fn ruled(s: &mut Session, x: f32, y: f32) -> bool {
        s.layout().pages[0].items.iter().any(|it| match it {
            Placed::Rule { x0, y0, x1, y1, border } => {
                border.is_visible() && x >= x0.min(*x1) - 0.5 && x <= x0.max(*x1) + 0.5 && y >= y0.min(*y1) - 0.5 && y <= y0.max(*y1) + 0.5
            }
            _ => false,
        })
    }

    /// As in Word (#1039): the border between two cells merged down the same rows merges them
    /// into one cell, still merged down those rows, with the text of all four.
    #[test]
    fn erasing_between_cells_merged_down_alike_merges_them() {
        let mut s = named_table(3, 3);
        // b1–b2 and c1–c2 merged down: the borders under b1 and c1 erased.
        for c in [1, 2] {
            let r = cell_rect(&mut s, 0, c);
            erase(&mut s, r.x + r.w / 2.0, r.bottom() + 0.5).unwrap();
        }
        let before = first_table(&s);
        assert_eq!((before.rows[1].cells[1].props.vmerge, before.rows[1].cells[2].props.vmerge), (VMerge::Continue, VMerge::Continue));
        let (b, c, a2) = (cell_rect(&mut s, 0, 1), cell_rect(&mut s, 0, 2), cell_rect(&mut s, 1, 0));
        // Erased beside the second row: the same border.
        erase(&mut s, b.right() + 1.0, a2.y + a2.h / 2.0).unwrap();
        let t = first_table(&s);
        assert_eq!(shape(&t), json!([[1, 2], [1, 2], [1, 1, 1]]));
        assert_eq!((t.rows[0].cells[1].props.vmerge, t.rows[1].cells[1].props.vmerge), (VMerge::Restart, VMerge::Continue));
        assert_eq!(text(&t, 0, 1), "B1|B2|C1|C2");
        // Four paragraphs: the rows may grow, so measured again.
        let (m, a2) = (cell_rect(&mut s, 0, 1), cell_rect(&mut s, 1, 0));
        assert!((m.w - (b.w + c.w)).abs() < 1.0 && (m.bottom() - a2.bottom()).abs() < 0.5, "one cell over both columns and rows: {m:?}");
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(first_table(&s), before, "one undo step");
    }

    /// As in Word (#1039): cells that can't become one cell keep their shape and the border is
    /// hidden on both sides (`nil`), all along a neighbour merged down further; once both sides
    /// span the same rows, the border merges them, and the cell keeps the far side's edge.
    #[test]
    fn a_border_whose_cells_cannot_become_one_is_hidden() {
        let mut s = named_table(2, 3);
        let c1 = cell_rect(&mut s, 0, 2);
        erase(&mut s, c1.x + c1.w / 2.0, c1.bottom() + 0.5).unwrap();
        let (a1, b1, b2) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 0, 1), cell_rect(&mut s, 1, 1));
        assert!(erase_target(&s.layout(), 0, c1.x + c1.w / 2.0, b1.bottom()).is_none(), "no border inside the merged cell");
        let x = b1.right();
        let (y1, y2) = (b1.y + b1.h / 2.0, b2.y + b2.h / 2.0);
        assert!(ruled(&mut s, x, y1) && ruled(&mut s, x, y2));
        let before = first_table(&s);
        let depth = s.undo_depth();
        // Beside b1 (one row) and c1–c2 (two rows).
        erase(&mut s, x + 1.0, y1).unwrap();
        let t = first_table(&s);
        assert_eq!(shape(&t), shape(&before), "nothing merges");
        let hidden = |r: usize, c: usize, right: bool| {
            let bs = t.rows[r].cells[c].props.borders.unwrap_or_default();
            (if right { bs.right } else { bs.left }).is_some_and(|b| !b.is_visible())
        };
        assert!(hidden(0, 1, true) && hidden(1, 1, true) && hidden(0, 2, false) && hidden(1, 2, false), "both sides, both rows");
        assert!(!ruled(&mut s, x, y1) && !ruled(&mut s, x, y2), "the border is gone all along c1–c2");
        assert!(ruled(&mut s, a1.right(), y1), "the other borders stay");
        // Erased again: nothing to do, an error, no undo step.
        assert!(erase(&mut s, x + 1.0, y2).is_err());
        assert_eq!(s.undo_depth(), depth + 1);
        // b1–b2 merged down: now both sides span the same rows, and the border merges them.
        erase(&mut s, b1.x + b1.w / 2.0, b1.bottom() + 0.5).unwrap();
        erase(&mut s, x + 1.0, y1).unwrap();
        let t = first_table(&s);
        assert_eq!(shape(&t), json!([[1, 2], [1, 2]]));
        assert_eq!(t.rows[0].cells[1].props.borders.and_then(|b| b.right), None, "c's right side, not the hidden one");
        let m = cell_rect(&mut s, 0, 1);
        assert!(ruled(&mut s, m.right(), y1) && ruled(&mut s, m.right(), y2), "the table's right edge is drawn");
    }

    /// As in Word (#1039): a horizontal border between cells that don't line up (a wide cell
    /// over two) is hidden on both sides; the part over the other cell stays.
    #[test]
    fn a_border_under_a_wider_cell_is_hidden() {
        let mut s = named_table(2, 3);
        let a1 = cell_rect(&mut s, 0, 0);
        erase(&mut s, a1.right() + 1.0, a1.y + a1.h / 2.0).unwrap();
        let (a2, b2) = (cell_rect(&mut s, 1, 0), cell_rect(&mut s, 1, 1));
        erase(&mut s, b2.x + b2.w / 2.0, b2.y - 0.5).unwrap();
        let t = first_table(&s);
        assert_eq!(shape(&t), json!([[2, 1], [1, 1, 1]]));
        assert!(t.rows[0].cells[0].props.borders.and_then(|b| b.bottom).is_some_and(|b| !b.is_visible()));
        assert!(t.rows[1].cells[1].props.borders.and_then(|b| b.top).is_some_and(|b| !b.is_visible()));
        assert!(!ruled(&mut s, b2.x + b2.w / 2.0, b2.y), "gone over b2");
        assert!(ruled(&mut s, a2.x + a2.w / 2.0, a2.y), "kept over a2");
    }

    /// As in Word, a drag draws a rectangle and every border it crosses goes, in one undo step.
    /// Across a row the second border is found again after the first merge renumbered the row's
    /// cells; a block of cells becomes one cell; a drag down a column merges it; a drag along a
    /// border leaves the borders that meet it.
    #[test]
    fn a_dragged_rectangle_erases_every_border_it_crosses() {
        let mut s = named_table(3, 3);
        let start = first_table(&s);
        let depth = s.undo_depth();
        let (a1, c1, b2, a3) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 0, 2), cell_rect(&mut s, 1, 1), cell_rect(&mut s, 2, 0));
        drag(&mut s, (a1.x + 5.0, a1.y + 3.0), (c1.right() - 5.0, a1.bottom() - 3.0)).unwrap();
        let t = first_table(&s);
        assert_eq!(shape(&t), json!([[3], [1, 1, 1], [1, 1, 1]]));
        assert_eq!(text(&t, 0, 0), "A1|B1|C1");
        assert_eq!(s.undo_depth(), depth + 1, "one undo step");
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(first_table(&s), start);
        // From inside A1 to inside B2: one cell over both columns and rows, as in Word.
        drag(&mut s, (a1.x + 10.0, a1.y + 3.0), (b2.right() - 10.0, b2.bottom() - 3.0)).unwrap();
        let t = first_table(&s);
        assert_eq!(shape(&t), json!([[2, 1], [2, 1], [1, 1, 1]]));
        assert_eq!((t.rows[0].cells[0].props.vmerge, t.rows[1].cells[0].props.vmerge), (VMerge::Restart, VMerge::Continue));
        assert_eq!(text(&t, 0, 0), "A1|B1|A2|B2");
        s.run("edit.undo", &json!({})).unwrap();
        // Down the first column: one cell over three rows.
        let x = a1.x + a1.w / 2.0;
        drag(&mut s, (x, a1.y + 3.0), (x, a3.bottom() - 3.0)).unwrap();
        let t = first_table(&s);
        let merges: Vec<VMerge> = t.rows.iter().map(|r| r.cells[0].props.vmerge).collect();
        assert_eq!(merges, [VMerge::Restart, VMerge::Continue, VMerge::Continue]);
        assert_eq!(text(&t, 0, 0), "A1|A2|A3");
        s.run("edit.undo", &json!({})).unwrap();
        // Along the border between A and B, a point off it either way: that border goes in every
        // row, and the rows stay apart.
        let x = a1.right();
        drag(&mut s, (x + 1.0, a1.y + 3.0), (x - 1.0, a3.bottom() - 3.0)).unwrap();
        let t = first_table(&s);
        assert_eq!(shape(&t), json!([[2, 1], [2, 1], [2, 1]]));
        assert!(t.rows.iter().all(|r| r.cells.iter().all(|c| c.props.vmerge == VMerge::None)));
        s.run("edit.undo", &json!({})).unwrap();
        // Within one cell: nothing crossed, an error.
        assert!(drag(&mut s, (a1.x + 5.0, a1.y + 3.0), (a1.x + 9.0, a1.y + 5.0)).is_err());
        assert_eq!(first_table(&s), start);
    }

    /// As in Word (observed): a rectangle from above a 4×4 table down along the border between A
    /// and B into the last row takes that border, the borders across columns A and B and the
    /// table's top edge there: A and B are one cell over all four rows, its top hidden in every
    /// row (`nil`, as Word writes it). The left edge, crossed in one row, is hidden in that row.
    #[test]
    fn a_rectangle_over_the_tables_edge_hides_it() {
        let mut s = named_table(4, 4);
        let (a1, b1, c1, a4) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 0, 1), cell_rect(&mut s, 0, 2), cell_rect(&mut s, 3, 0));
        let x = a1.right();
        drag(&mut s, (x - 40.0, a1.y - 12.0), (x + 30.0, a4.y + a4.h / 2.0)).unwrap();
        let t = first_table(&s);
        assert_eq!(shape(&t), json!([[2, 1, 1], [2, 1, 1], [2, 1, 1], [2, 1, 1]]));
        assert_eq!(text(&t, 0, 0), "A1|B1|A2|B2|A3|B3|A4|B4");
        let top = |r: usize| t.rows[r].cells[0].props.borders.and_then(|b| b.top).is_some_and(|b| !b.is_visible());
        assert!((0..4).all(top), "the top hidden on every row of the merged cell");
        assert!(!ruled(&mut s, a1.x + a1.w / 2.0, a1.y) && !ruled(&mut s, b1.x + b1.w / 2.0, b1.y), "no top over A and B");
        assert!(ruled(&mut s, c1.x + c1.w / 2.0, c1.y), "the rest of the top edge stays");
        s.run("edit.undo", &json!({})).unwrap();
        // Across the left edge in the first row only.
        drag(&mut s, (a1.x - 10.0, a1.y + 3.0), (a1.x + 10.0, a1.bottom() - 3.0)).unwrap();
        let t = first_table(&s);
        let left = |r: usize| t.rows[r].cells[0].props.borders.and_then(|b| b.left).is_some_and(|b| !b.is_visible());
        assert!(left(0) && !left(1));
        assert!(!ruled(&mut s, a1.x, a1.y + a1.h / 2.0));
        // A click on the edge hides it in that cell (Word: `nil` on the second row's left only).
        let (a2, a3) = (cell_rect(&mut s, 1, 0), cell_rect(&mut s, 2, 0));
        erase(&mut s, a2.x - 1.0, a2.y + a2.h / 2.0).unwrap();
        assert!(!ruled(&mut s, a2.x, a2.y + a2.h / 2.0) && ruled(&mut s, a3.x, a3.y + a3.h / 2.0));
        assert!(erase(&mut s, a2.x - 1.0, a2.y + a2.h / 2.0).is_err(), "hidden already");
    }

    /// Hostile rectangles are refused; one over everything makes the table one cell.
    #[test]
    fn hostile_eraser_rectangles() {
        let mut s = named_table(2, 2);
        let start = first_table(&s);
        let bad = [
            json!({"page": 0, "x0": 1.0}),
            json!({"page": 0, "x0": "a", "y0": 1, "x1": 2, "y1": 3}),
            json!({"page": 0, "x0": 1e39, "y0": 0, "x1": 0, "y1": 0}),
            json!({"page": 7, "x0": 0, "y0": 0, "x1": 600, "y1": 800}),
        ];
        for v in bad {
            assert!(s.run("table.eraser", &v).is_err(), "{v}");
        }
        assert_eq!(first_table(&s), start);
        drag(&mut s, (-3e38, -3e38), (3e38, 3e38)).unwrap();
        let t = first_table(&s);
        assert_eq!(shape(&t), json!([[2], [2]]));
        assert_eq!(text(&t, 0, 0), "A1|B1|A2|B2");
    }

    /// In a right-to-left table, whose first column stands on the right, the eraser finds the
    /// borders between columns too.
    #[test]
    fn the_eraser_works_in_a_right_to_left_table() {
        let mut s = named_table(1, 3);
        s.run("table.properties", &json!({"rtl": true})).unwrap();
        let (a1, b1) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 0, 1));
        assert!(b1.right() <= a1.x + 1.0, "b1 stands left of a1");
        erase(&mut s, b1.right() - 1.0, b1.y + b1.h / 2.0).unwrap();
        let t = first_table(&s);
        assert_eq!(shape(&t), json!([[2, 1]]));
        assert_eq!(text(&t, 0, 0), "A1|B1");
    }
}

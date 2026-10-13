//! Ink to Math: single-glyph template recognition for handwritten digits and
//! operators, plus the lasso-to-equation conversion.
//!
//! Original templates, geometric matching, no ML and no bundled handwriting data:
//! each glyph is an idealised polyline in a 64-point box, matched by resampled
//! point distance. Only confident matches convert (mean distance under
//! [`ACCEPT`]); anything else stays ink. Stacked layouts (fractions) are refused
//! with a clear error rather than guessed.

use serde_json::{Value, json};
use wordcraft_doc::Pos;
use wordcraft_doc::para::OBJ;

use super::draw::{PlacedInk, delete_object, ink_on, keep_lasso, lasso_required};
use crate::{CmdError, CmdResult, Selection, Session};

/// Recognition box, points.
const BOX: f32 = 64.0;
/// Points per glyph after resampling.
const RESAMPLE: usize = 16;
/// A mean point distance (box points) at or above this is not a glyph: above
/// the worst wobble of a true glyph (5.9), below the closest glyph pair (7.9).
const ACCEPT: f32 = 7.0;
/// A smaller span is a dot or a slip, not a glyph.
const MIN_SPAN: f32 = 6.0;
/// Strokes in one cluster beyond this are a drawing, not a glyph.
const MAX_CLUSTER_STROKES: usize = 2;
/// Clusters whose middles differ by more than this share of the median glyph
/// height are stacked rows (a fraction), which we don't do yet.
const ROW_GAP: f32 = 0.75;

/// Idealised glyphs: the character and its waypoints in a 64-point box, y down.
/// Multi-part glyphs (`+`, `=`, `x`) are one polyline through the parts; stroke
/// order and direction are tried both ways at match time.
fn templates() -> Vec<(char, Vec<[f32; 2]>)> {
    let ellipse = |cx: f32, cy: f32, rx: f32, ry: f32| {
        (0..RESAMPLE)
            .map(|i| {
                let a = i as f32 / RESAMPLE as f32 * std::f32::consts::TAU;
                [cx + rx * a.cos(), cy + ry * a.sin()]
            })
            .collect::<Vec<_>>()
    };
    vec![
        ('0', ellipse(32.0, 32.0, 26.0, 30.0)),
        ('1', vec![[24.0, 16.0], [33.0, 8.0], [33.0, 56.0]]),
        ('2', vec![[10.0, 18.0], [26.0, 8.0], [44.0, 10.0], [46.0, 24.0], [38.0, 34.0], [14.0, 52.0], [48.0, 54.0]]),
        ('3', vec![[12.0, 12.0], [30.0, 6.0], [44.0, 14.0], [38.0, 26.0], [28.0, 30.0], [40.0, 38.0], [40.0, 50.0], [26.0, 58.0], [12.0, 52.0]]),
        ('4', vec![[38.0, 6.0], [18.0, 38.0], [48.0, 38.0]]),
        ('5', vec![[46.0, 10.0], [20.0, 8.0], [14.0, 24.0], [30.0, 32.0], [44.0, 34.0], [42.0, 50.0], [26.0, 58.0], [12.0, 52.0]]),
        ('6', vec![[42.0, 10.0], [26.0, 18.0], [16.0, 34.0], [20.0, 50.0], [36.0, 56.0], [48.0, 44.0], [40.0, 32.0], [28.0, 32.0]]),
        ('7', vec![[10.0, 12.0], [50.0, 10.0], [26.0, 58.0]]),
        (
            '8',
            vec![
                [32.0, 6.0],
                [46.0, 14.0],
                [42.0, 28.0],
                [32.0, 32.0],
                [20.0, 38.0],
                [20.0, 52.0],
                [32.0, 58.0],
                [44.0, 52.0],
                [46.0, 40.0],
                [36.0, 32.0],
            ],
        ),
        ('9', vec![[22.0, 54.0], [38.0, 50.0], [48.0, 34.0], [44.0, 18.0], [30.0, 8.0], [16.0, 18.0], [22.0, 30.0], [36.0, 32.0]]),
        ('+', vec![[32.0, 10.0], [32.0, 54.0], [32.0, 32.0], [10.0, 32.0], [54.0, 32.0]]),
        ('-', vec![[10.0, 32.0], [54.0, 32.0]]),
        ('=', vec![[10.0, 24.0], [54.0, 24.0], [54.0, 40.0], [10.0, 40.0]]),
        ('x', vec![[16.0, 16.0], [48.0, 48.0], [48.0, 16.0], [16.0, 48.0]]),
        ('/', vec![[44.0, 8.0], [20.0, 56.0]]),
        ('(', vec![[42.0, 8.0], [28.0, 20.0], [22.0, 32.0], [28.0, 44.0], [42.0, 56.0]]),
        (')', vec![[22.0, 8.0], [36.0, 20.0], [42.0, 32.0], [36.0, 44.0], [22.0, 56.0]]),
    ]
}

/// Evenly spaced points along a polyline (drops non-finite input).
fn resample(pts: &[[f32; 2]], n: usize) -> Vec<[f32; 2]> {
    let pts: Vec<[f32; 2]> = pts.iter().copied().filter(|[x, y]| x.is_finite() && y.is_finite()).collect();
    if pts.len() < 2 || n == 0 {
        return pts;
    }
    let mut lengths = vec![0.0];
    for w in pts.windows(2) {
        let dx = w[1][0] - w[0][0];
        let dy = w[1][1] - w[0][1];
        lengths.push(lengths.last().copied().unwrap_or(0.0) + dx.hypot(dy));
    }
    let total = lengths.last().copied().unwrap_or(0.0);
    if total <= 0.0 {
        return vec![pts[0]; n.min(pts.len()).max(1)];
    }
    let step = total / (n - 1) as f32;
    let mut out = Vec::with_capacity(n);
    let mut j = 0;
    for i in 0..n {
        let target = step * i as f32;
        while j + 1 < lengths.len() && lengths[j + 1] < target {
            j += 1;
        }
        let k = j.min(pts.len() - 2);
        let seg = (lengths[k + 1] - lengths[k]).max(1e-6);
        let t = ((target - lengths[k]) / seg).clamp(0.0, 1.0);
        out.push([pts[k][0] + t * (pts[k + 1][0] - pts[k][0]), pts[k][1] + t * (pts[k + 1][1] - pts[k][1])]);
    }
    out
}

/// Into the recognition box, uniform scale (orientation matters: `-` is not
/// `|`), centred. `None` for degenerate input.
fn normalize(pts: &[[f32; 2]]) -> Option<Vec<[f32; 2]>> {
    let mut x0 = f32::INFINITY;
    let mut y0 = f32::INFINITY;
    let mut x1 = f32::NEG_INFINITY;
    let mut y1 = f32::NEG_INFINITY;
    for [x, y] in pts.iter().copied().filter(|[x, y]| x.is_finite() && y.is_finite()) {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    if !x0.is_finite() {
        return None;
    }
    let (w, h) = (x1 - x0, y1 - y0);
    if w.max(h) < MIN_SPAN || !w.is_finite() || !h.is_finite() {
        return None;
    }
    let k = BOX / w.max(h);
    let (ox, oy) = ((BOX - w * k) / 2.0 - x0 * k, (BOX - h * k) / 2.0 - y0 * k);
    Some(pts.iter().copied().filter(|[x, y]| x.is_finite() && y.is_finite()).map(|[x, y]| [x * k + ox, y * k + oy]).collect())
}

fn distance(a: &[[f32; 2]], b: &[[f32; 2]]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return f32::INFINITY;
    }
    a.iter().zip(b).map(|([ax, ay], [bx, by])| (ax - bx).hypot(ay - by)).sum::<f32>() / a.len() as f32
}

/// One glyph from one polyline (possibly concatenated strokes): the best
/// template under the acceptance threshold, trying both stroke directions.
/// Templates are normalised exactly like the input.
fn match_glyph(pts: &[[f32; 2]]) -> Option<char> {
    let norm = normalize(pts)?;
    let fwd = resample(&norm, RESAMPLE);
    let mut rev = fwd.clone();
    rev.reverse();
    let mut best: Option<(char, f32)> = None;
    for (ch, template) in templates() {
        let Some(n) = normalize(&template) else { continue };
        let t = resample(&n, RESAMPLE);
        let d = distance(&fwd, &t).min(distance(&rev, &t));
        if d < ACCEPT && best.is_none_or(|(_, bd)| d < bd) {
            best = Some((ch, d));
        }
    }
    best.map(|(ch, _)| ch)
}

/// Recognise concatenated stroke point lists (one cluster) as one glyph.
pub fn recognize_strokes(strokes: &[Vec<[f32; 2]>]) -> Option<char> {
    if strokes.is_empty() || strokes.len() > MAX_CLUSTER_STROKES {
        return None;
    }
    // Stroke order varies (a `+` starts with either bar): try both.
    let mut variants = Vec::new();
    let joined: Vec<[f32; 2]> = strokes.iter().flatten().copied().collect();
    variants.push(joined);
    if strokes.len() == 2 {
        variants.push(strokes[1].iter().chain(strokes[0].iter()).copied().collect());
    }
    variants.into_iter().filter_map(|v| match_glyph(&v)).next()
}

/// Bounding boxes (page points) of ink objects, from their placed paths.
fn bbox(paths: &[Vec<[f32; 2]>]) -> Option<[f32; 4]> {
    let mut box_: Option<[f32; 4]> = None;
    for pts in paths.iter().flatten().copied().filter(|[x, y]| x.is_finite() && y.is_finite()) {
        box_ = Some(match box_ {
            Some([x0, y0, x1, y1]) => [x0.min(pts[0]), y0.min(pts[1]), x1.max(pts[0]), y1.max(pts[1])],
            None => [pts[0], pts[1], pts[0], pts[1]],
        });
    }
    box_
}

/// Strokes whose boxes overlap (4 pt slop) are one glyph (`+`, `=`, `x` are
/// drawn in two strokes). Returns index clusters in x order.
fn cluster(objects: &[Vec<Vec<[f32; 2]>>]) -> Vec<Vec<usize>> {
    const SLOP: f32 = 4.0;
    let boxes: Vec<Option<[f32; 4]>> = objects.iter().map(|p| bbox(p)).collect();
    let mut order: Vec<usize> = (0..objects.len()).collect();
    order.sort_by(|a, b| {
        let xa = boxes[*a].map_or(f32::INFINITY, |b| b[0]);
        let xb = boxes[*b].map_or(f32::INFINITY, |b| b[0]);
        xa.partial_cmp(&xb).unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut clusters: Vec<Vec<usize>> = Vec::new();
    for i in order {
        let Some([x0, y0, x1, y1]) = boxes[i] else { continue };
        let mut placed = false;
        for c in clusters.iter_mut() {
            let mut cx = boxes[c[0]].unwrap_or([0.0; 4]);
            for m in c.iter().skip(1) {
                if let Some([a, b2, c2, d]) = boxes[*m] {
                    cx = [cx[0].min(a), cx[1].min(b2), cx[2].max(c2), cx[3].max(d)];
                }
            }
            if x0 <= cx[2] + SLOP && cx[0] - SLOP <= x1 && y0 <= cx[3] + SLOP && cx[1] - SLOP <= y1 {
                c.push(i);
                placed = true;
                break;
            }
        }
        if !placed {
            clusters.push(vec![i]);
        }
    }
    clusters
}

/// The lasso-selected ink as recognised glyph clusters: `(document positions,
/// glyph)`, clusters left to right. Errors when nothing is selected, a stroke
/// is turned or uses the highlighter, a cluster isn't recognised, or clusters
/// stack (a fraction).
fn recognize_lasso(s: &mut Session) -> Result<Vec<(Vec<Pos>, char)>, CmdError> {
    let layout = s.layout();
    let sel = lasso_required(s)?;
    // Resolve the selection to placed ink.
    let mut items: Vec<(Pos, PlacedInk)> = Vec::new();
    for pos in &sel.items {
        let found = (0..layout.pages.len()).flat_map(|pg| ink_on(s, &layout, pg)).find(|k| k.pos == *pos);
        let Some(k) = found else {
            return Err(CmdError::Failed("the selection holds no ink stroke there".into()));
        };
        items.push((pos.clone(), k));
    }
    if items.is_empty() {
        return Err(CmdError::Failed("select ink strokes first".into()));
    }
    // Usable strokes only: pen or pencil, unturned, a single path.
    let mut usable: Vec<(Pos, Vec<[f32; 2]>)> = Vec::new();
    for (pos, k) in &items {
        if !k.spin.is_identity() {
            return Err(CmdError::Failed("turned ink can't become math".into()));
        }
        let is_pen = s
            .doc
            .para_at(&k.pos)
            .and_then(|p| p.object_at(k.pos.off))
            .and_then(|o| o.ink())
            .is_some_and(|f| matches!(f.ink, Some(wordcraft_doc::freeform::InkTool::Pen | wordcraft_doc::freeform::InkTool::Pencil)));
        if !is_pen {
            return Err(CmdError::Failed("only pen and pencil strokes become math".into()));
        }
        let mut paths = k.paths();
        if paths.len() != 1 {
            return Err(CmdError::Failed("only single-path strokes become math".into()));
        }
        usable.push((pos.clone(), paths.pop().unwrap_or_default()));
    }
    // Cluster, recognise, single row only.
    let path_lists: Vec<Vec<Vec<[f32; 2]>>> = usable.iter().map(|(_, p)| vec![p.clone()]).collect();
    let clusters = cluster(&path_lists);
    let mut heights = Vec::new();
    let mut out = Vec::new();
    for c in &clusters {
        let mut strokes = Vec::new();
        let mut positions = Vec::new();
        let mut cy = Vec::new();
        for i in c {
            strokes.push(usable[*i].1.clone());
            positions.push(usable[*i].0.clone());
            if let Some([_, y0, _, y1]) = bbox(&[usable[*i].1.clone()]) {
                cy.push((y0 + y1) / 2.0);
                heights.push((y1 - y0).max(1.0));
            }
        }
        let Some(ch) = recognize_strokes(&strokes) else {
            return Err(CmdError::Failed("none of the selected strokes looks like math".into()));
        };
        out.push((positions, ch, cy.first().copied().unwrap_or(0.0)));
    }
    // Stacked rows (a fraction) are refused rather than guessed.
    let mut ys: Vec<f32> = out.iter().map(|(_, _, y)| *y).collect();
    ys.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median_h = {
        let mut h = heights.clone();
        h.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        h.get(h.len() / 2).copied().unwrap_or(24.0)
    };
    if ys.last().copied().unwrap_or(0.0) - ys.first().copied().unwrap_or(0.0) > ROW_GAP * median_h {
        return Err(CmdError::Failed("stacked math like fractions isn't supported yet".into()));
    }
    // Left-to-right glyph order (document order: paragraph path, then offset).
    let mut order: Vec<usize> = (0..out.len()).collect();
    order.sort_by(|a, b| {
        let ka = out[*a].0.first().map(|p| (p.path.clone(), p.off));
        let kb = out[*b].0.first().map(|p| (p.path.clone(), p.off));
        ka.cmp(&kb)
    });
    Ok(order.into_iter().map(|i| (out[i].0.clone(), out[i].1)).collect())
}

/// Delete the lasso-selected ink strokes and return where the leftmost one
/// was, without recognising anything (for callers supplying their own text).
pub(crate) fn take_selected_ink(s: &mut Session) -> Result<Pos, CmdError> {
    let sel = lasso_required(s)?;
    let layout = s.layout();
    let ink: std::collections::HashSet<Pos> = (0..layout.pages.len()).flat_map(|pg| ink_on(s, &layout, pg)).map(|k| k.pos).collect();
    let positions: Vec<Pos> = sel.items.into_iter().filter(|p| ink.contains(p)).collect();
    if positions.is_empty() {
        return Err(CmdError::Failed("select ink strokes first".into()));
    }
    delete_ink(s, positions)
}

/// Delete ink positions, latest first so earlier ones keep their offsets, and
/// return where the leftmost deletion was: each deletion in its paragraph
/// before that point moves it left by one object character.
fn delete_ink(s: &mut Session, mut positions: Vec<Pos>) -> Result<Pos, CmdError> {
    positions.sort_by(|a, b| if a.story == b.story && a.path == b.path { b.off.cmp(&a.off) } else { std::cmp::Ordering::Equal });
    let anchor = positions
        .iter()
        .min_by(|a, b| a.path.cmp(&b.path).then(a.off.cmp(&b.off)))
        .cloned()
        .ok_or_else(|| CmdError::Failed("select ink strokes first".into()))?;
    let before = positions.iter().filter(|p| p.story == anchor.story && p.path == anchor.path && p.off < anchor.off).count();
    for pos in &positions {
        delete_object(s, pos)?;
    }
    let shift = before.saturating_mul(OBJ.len_utf8());
    Ok(Pos { off: anchor.off.saturating_sub(shift), ..anchor })
}

/// Convert the lasso-selected ink into equations, one undo step: recognised
/// glyph clusters become one joined equation at the leftmost deletion.
pub(crate) fn convert_selected_ink(s: &mut Session, v: &Value) -> CmdResult {
    let sel = lasso_required(s)?;
    let groups = recognize_lasso(s)?;
    let linear: String = groups.iter().map(|(_, ch)| *ch).collect();
    let positions: Vec<Pos> = groups.into_iter().flat_map(|(p, _)| p).collect();
    let at = delete_ink(s, positions)?;
    s.sel = Selection::caret(at);
    let nodes = wordcraft_doc::math::parse_linear(&linear);
    super::equation::insert_nodes_as_equation(s, nodes, v)?;
    keep_lasso(s, sel);
    Ok(json!({"linear": linear}))
}

/// Try converting the single stroke at `pos` (the Ink to Math mode path):
/// `None` when it doesn't convert (stays ink, like Ink to Shape misses).
pub(crate) fn try_convert_stroke_at(s: &mut Session, pos: &Pos) -> Result<Option<String>, CmdError> {
    let layout = s.layout();
    let Some(k) = (0..layout.pages.len()).flat_map(|pg| super::draw::ink_on(s, &layout, pg)).find(|k| k.pos == *pos) else {
        return Ok(None);
    };
    if !k.spin.is_identity() {
        return Ok(None);
    }
    let mut paths = k.paths();
    if paths.len() != 1 {
        return Ok(None);
    }
    let Some(ch) = recognize_strokes(&[paths.pop().unwrap_or_default()]) else {
        return Ok(None);
    };
    let linear = ch.to_string();
    let at = delete_ink(s, vec![pos.clone()])?;
    s.sel = Selection::caret(at);
    let nodes = wordcraft_doc::math::parse_linear(&linear);
    super::equation::insert_nodes_as_equation(s, nodes, &json!({"edit": false}))?;
    Ok(Some(linear))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every template matches itself exactly.
    #[test]
    fn templates_match_themselves() {
        for (ch, waypoints) in templates() {
            assert_eq!(recognize_strokes(&[waypoints]), Some(ch), "{ch}");
        }
    }

    /// Templates stay apart: no template is within the acceptance threshold of
    /// another, so a confident match is unambiguous.
    #[test]
    fn templates_are_distinct() {
        let all = templates();
        let normed: Vec<(char, Vec<[f32; 2]>)> = all.iter().map(|(ch, w)| (*ch, resample(&normalize(w).unwrap(), RESAMPLE))).collect();
        for (i, (a, pa)) in normed.iter().enumerate() {
            for (j, (b, pb)) in normed.iter().enumerate() {
                if i != j {
                    assert!(distance(pa, pb) > ACCEPT, "{a} too close to {b}");
                }
            }
        }
    }

    /// Handwriting wobble (fixed ±2.5 pt jitter) still recognises.
    #[test]
    fn jittered_templates_still_match() {
        for (ch, waypoints) in templates() {
            let wobbled: Vec<[f32; 2]> = waypoints
                .iter()
                .enumerate()
                .map(|(i, [x, y])| {
                    let d = if i % 2 == 0 { 2.5 } else { -2.5 };
                    [x + d, y - d]
                })
                .collect();
            assert_eq!(recognize_strokes(&[wobbled]), Some(ch), "{ch}");
        }
    }

    /// Dots and scribbles are not math.
    #[test]
    fn dots_and_scribbles_match_nothing() {
        assert_eq!(recognize_strokes(&[vec![[30.0, 30.0], [31.0, 31.0]]]), None, "a dot");
        assert_eq!(recognize_strokes(&[]), None, "no strokes");
        assert_eq!(recognize_strokes(&[vec![[0.0, 0.0], [1.0, 1.0], [2.0, 0.0]]]), None, "too small");
    }

    /// Stroke order and direction don't matter for two-part glyphs.
    #[test]
    fn plus_matches_either_stroke_order() {
        let vertical = vec![[32.0, 10.0], [32.0, 54.0]];
        let horizontal = vec![[10.0, 32.0], [54.0, 32.0]];
        assert_eq!(recognize_strokes(&[vertical.clone(), horizontal.clone()]), Some('+'));
        assert_eq!(recognize_strokes(&[horizontal, vertical]), Some('+'));
    }

    /// Three strokes are a drawing, not a glyph.
    #[test]
    fn three_strokes_match_nothing() {
        let s = vec![vec![[10.0, 10.0], [20.0, 20.0]]; 3];
        assert_eq!(recognize_strokes(&s), None);
    }
}

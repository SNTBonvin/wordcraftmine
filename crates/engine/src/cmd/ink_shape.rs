//! Draw › Ink to Shape: a small geometric recogniser that tells whether a hand-drawn stroke is a
//! straight line, a rectangle (or square), an ellipse (or circle), a triangle or another polygon
//! of up to eight corners. Our own fit: an open stroke is a line when it stays close to the chord
//! between its ends; a closed one is an ellipse when its points sit on the ellipse inscribed in
//! its bounds (low RMS of the normalised radius), else a polygon from its corners (the closed
//! path simplified with Ramer–Douglas–Peucker, near-straight and crowded corners merged) when
//! that polygon stays close to every point. Anything else (scribbles, curves, letters) stays ink.

use wordcraft_doc::freeform::MAX_POINTS;

/// A shape recognised in a stroke, in the stroke's own coordinates.
#[derive(Clone, Debug, PartialEq)]
pub enum InkShape {
    /// A straight line from `a` to `b`.
    Line { a: [f32; 2], b: [f32; 2] },
    /// A straight arrow from the tail `a` to the tip `b`; the heads are the `head_at_*` flags
    /// (recognised strokes have one, at `b`).
    Arrow { a: [f32; 2], b: [f32; 2], head_at_end: bool, head_at_start: bool },
    /// An upright rectangle: top-left and size (equal sides when drawn nearly square).
    Rectangle { x: f32, y: f32, w: f32, h: f32 },
    /// An upright ellipse in this box (a circle when drawn nearly round).
    Ellipse { x: f32, y: f32, w: f32, h: f32 },
    /// A closed polygon through its 3–8 corners (a triangle has three).
    Polygon(Vec<[f32; 2]>),
}

impl InkShape {
    /// What it is, for messages and command results.
    pub fn name(&self) -> &'static str {
        match self {
            InkShape::Line { .. } => "line",
            InkShape::Arrow { .. } => "arrow",
            InkShape::Rectangle { w, h, .. } if w == h => "square",
            InkShape::Rectangle { .. } => "rectangle",
            InkShape::Ellipse { w, h, .. } if w == h => "circle",
            InkShape::Ellipse { .. } => "ellipse",
            InkShape::Polygon(c) if c.len() == 3 => "triangle",
            InkShape::Polygon(_) => "polygon",
        }
    }

    /// Its bounds: left, top, width, height.
    pub fn bounds(&self) -> [f32; 4] {
        match self {
            InkShape::Rectangle { x, y, w, h } | InkShape::Ellipse { x, y, w, h } => [*x, *y, *w, *h],
            InkShape::Line { a, b } | InkShape::Arrow { a, b, .. } => bbox(&[*a, *b]),
            InkShape::Polygon(c) => bbox(c),
        }
    }
}

/// Smallest stroke (its longer side, points) that is turned into a shape: a tap or a tick stays ink.
const MIN_SIZE: f32 = 6.0;
/// Most corners a polygon may have.
pub const MAX_CORNERS: usize = 8;

/// The shape `pts` draws, if it is one.
pub fn recognise(pts: &[[f32; 2]]) -> Option<InkShape> {
    // Finite points only, without repeats.
    let mut p: Vec<[f32; 2]> = Vec::with_capacity(pts.len().min(MAX_POINTS));
    for q in pts.iter().take(MAX_POINTS).filter(|[x, y]| x.is_finite() && y.is_finite() && x.abs() < 1e6 && y.abs() < 1e6) {
        if p.last() != Some(q) {
            p.push(*q);
        }
    }
    let (first, last) = (*p.first()?, *p.last()?);
    if p.len() < 2 {
        return None;
    }
    let [x0, y0, w, h] = bbox(&p);
    let size = w.max(h);
    let diag = w.hypot(h);
    if size < MIN_SIZE {
        return None;
    }
    let length: f32 = p.windows(2).map(|s| s.first().zip(s.get(1)).map_or(0.0, |(a, b)| dist(*a, *b))).sum();
    let gap = dist(first, last);
    // A line: the stroke barely leaves the chord between its ends and doesn't double back.
    if gap > 0.85 * length && gap >= MIN_SIZE {
        let off = p.iter().map(|q| seg_distance(first, last, *q)).fold(0.0, f32::max);
        if off <= (0.06 * gap).max(1.5) {
            return Some(InkShape::Line { a: first, b: last });
        }
    }
    // An arrow: a shaft and a head with both barbs, drawn in one stroke (either end first).
    if let Some(arrow) = arrow(&p, diag).or_else(|| arrow(&p.iter().rev().copied().collect::<Vec<_>>(), diag)) {
        return Some(arrow);
    }
    // Other shapes are closed: the stroke ends near where it began and goes around.
    if gap > 0.2 * size || length < 2.0 * size {
        return None;
    }
    // Ellipse: the points sit on the ellipse inscribed in their bounds.
    let ellipse_rms = ellipse_rms(&p, x0, y0, w, h);
    if w > 0.2 * size && h > 0.2 * size && ellipse_rms < 0.06 {
        let (x, y, w, h) = snap_square(x0, y0, w, h);
        return Some(InkShape::Ellipse { x, y, w, h });
    }
    // Polygon: the corners of the closed path, if they describe it.
    let corners = corners(&p, diag);
    let fits = corners.len() >= 3 && p.iter().all(|q| polygon_distance(&corners, *q) <= 0.08 * diag);
    if fits && corners.len() == 4 && upright(&corners) {
        let [x, y, w, h] = bbox(&corners);
        let (x, y, w, h) = snap_square(x, y, w, h);
        return Some(InkShape::Rectangle { x, y, w, h });
    }
    if fits && corners.len() <= MAX_CORNERS {
        return Some(InkShape::Polygon(corners));
    }
    // A wobbly round stroke is still an ellipse.
    (w > 0.2 * size && h > 0.2 * size && ellipse_rms < 0.1).then(|| {
        let (x, y, w, h) = snap_square(x0, y0, w, h);
        InkShape::Ellipse { x, y, w, h }
    })
}

/// Barb length limits, as a fraction of the shaft, and the angle each barb makes with the shaft
/// (degrees).
const BARB_LEN: (f32, f32) = (0.05, 0.5);
const BARB_ANGLE: (f32, f32) = (15.0, 75.0);

/// An arrow drawn shaft first: the simplified stroke is tail, tip, one barb, back at the tip, the
/// other barb. The barbs must leave the tip on opposite sides of the shaft at similar angles and
/// lengths. (A stroke drawn head first is tried reversed by the caller.)
fn arrow(p: &[[f32; 2]], diag: f32) -> Option<InkShape> {
    let v = rdp(p, (0.025 * diag).max(1.5));
    let &[tail, tip, b1, tip2, b2] = v.as_slice() else { return None };
    let shaft = dist(tail, tip);
    if shaft < MIN_SIZE || dist(tip, tip2) > 0.15 * shaft {
        return None;
    }
    // Each barb as a vector from the tip, against the direction back along the shaft.
    let back = [tail[0] - tip[0], tail[1] - tip[1]];
    let barb = |e: [f32; 2]| {
        let u = [e[0] - tip[0], e[1] - tip[1]];
        let len = u[0].hypot(u[1]);
        let lb = back[0].hypot(back[1]);
        let angle = if len > 0.0 && lb > 0.0 { ((back[0] * u[0] + back[1] * u[1]) / (lb * len)).clamp(-1.0, 1.0).acos().to_degrees() } else { 0.0 };
        let side = back[0] * u[1] - back[1] * u[0];
        (len, angle, side)
    };
    let ((l1, a1, s1), (l2, a2, s2)) = (barb(b1), barb(b2));
    let ok = |l: f32, a: f32| (BARB_LEN.0 * shaft..=BARB_LEN.1 * shaft).contains(&l) && (BARB_ANGLE.0..=BARB_ANGLE.1).contains(&a);
    let similar = l1.max(l2) <= 2.5 * l1.min(l2) && (a1 - a2).abs() <= 35.0;
    (ok(l1, a1) && ok(l2, a2) && s1 * s2 < 0.0 && similar).then_some(InkShape::Arrow { a: tail, b: tip, head_at_end: true, head_at_start: false })
}

/// Left, top, width and height of `pts` (zero for none).
fn bbox(pts: &[[f32; 2]]) -> [f32; 4] {
    let Some(f) = pts.first() else { return [0.0; 4] };
    let (mut x0, mut y0, mut x1, mut y1) = (f[0], f[1], f[0], f[1]);
    for [x, y] in pts {
        (x0, y0, x1, y1) = (x0.min(*x), y0.min(*y), x1.max(*x), y1.max(*y));
    }
    [x0, y0, x1 - x0, y1 - y0]
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}

/// Distance from `p` to the segment `a`–`b`.
fn seg_distance(a: [f32; 2], b: [f32; 2], p: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    dist([a[0] + t * dx, a[1] + t * dy], p)
}

/// Distance from `p` to the closed polygon through `c`.
fn polygon_distance(c: &[[f32; 2]], p: [f32; 2]) -> f32 {
    let n = c.len();
    (0..n).filter_map(|i| Some(seg_distance(*c.get(i)?, *c.get((i + 1) % n)?, p))).fold(f32::INFINITY, f32::min)
}

/// How far the points stray from the ellipse inscribed in the `w` × `h` box at (`x0`, `y0`): the
/// RMS of each point's normalised radius minus one (0 on the ellipse).
fn ellipse_rms(p: &[[f32; 2]], x0: f32, y0: f32, w: f32, h: f32) -> f32 {
    let (rx, ry) = (w / 2.0, h / 2.0);
    if rx <= 0.0 || ry <= 0.0 || p.is_empty() {
        return f32::INFINITY;
    }
    let (cx, cy) = (x0 + rx, y0 + ry);
    let sum: f32 = p.iter().map(|[x, y]| (((x - cx) / rx).hypot((y - cy) / ry) - 1.0).powi(2)).sum();
    (sum / p.len() as f32).sqrt()
}

/// A box drawn nearly square made square (same centre).
fn snap_square(x: f32, y: f32, w: f32, h: f32) -> (f32, f32, f32, f32) {
    if (w - h).abs() <= 0.12 * w.max(h) {
        let s = (w + h) / 2.0;
        (x + (w - s) / 2.0, y + (h - s) / 2.0, s, s)
    } else {
        (x, y, w, h)
    }
}

/// The corners of the closed path through `p`: simplified with Ramer–Douglas–Peucker (split
/// at the point farthest from the start), then corners that barely turn or crowd a neighbour
/// merged away.
fn corners(p: &[[f32; 2]], diag: f32) -> Vec<[f32; 2]> {
    let eps = 0.05 * diag;
    let Some(&start) = p.first() else { return Vec::new() };
    let far = p.iter().enumerate().fold((0, 0.0), |best, (i, q)| if dist(start, *q) > best.1 { (i, dist(start, *q)) } else { best }).0;
    let (a, b) = p.split_at(far.min(p.len()));
    let mut ring: Vec<[f32; 2]> = rdp(&[a, b.get(..1).unwrap_or(&[])].concat(), eps);
    let mut back = rdp(&[b, &[start]].concat(), eps);
    // The halves share the far point and the start.
    ring.pop();
    back.pop();
    ring.extend(back);
    // Merge until every corner turns sharply and none sits on top of the next.
    while ring.len() > 3 {
        let n = ring.len();
        let turn = |i: usize| -> f32 {
            let (Some(a), Some(b), Some(c)) = (ring.get((i + n - 1) % n), ring.get(i), ring.get((i + 1) % n)) else { return 0.0 };
            turning(*a, *b, *c)
        };
        let (i, t) = (0..n).map(|i| (i, turn(i))).fold((0, f32::INFINITY), |m, x| if x.1 < m.1 { x } else { m });
        let short = (0..n).find(|&j| ring.get(j).zip(ring.get((j + 1) % n)).is_some_and(|(a, b)| dist(*a, *b) < 0.1 * diag));
        if t < 30.0 {
            ring.remove(i);
        } else if let Some(j) = short {
            // Of the two crowded corners keep the sharper.
            let k = if turn(j) < turn((j + 1) % n) { j } else { (j + 1) % n };
            ring.remove(k);
        } else {
            break;
        }
    }
    ring
}

/// How far the path turns at `b` coming from `a` and going on to `c`, degrees (0 = straight on).
fn turning(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f32 {
    let (u, v) = ([b[0] - a[0], b[1] - a[1]], [c[0] - b[0], c[1] - b[1]]);
    let (lu, lv) = (u[0].hypot(u[1]), v[0].hypot(v[1]));
    if lu <= 0.0 || lv <= 0.0 {
        return 0.0;
    }
    ((u[0] * v[0] + u[1] * v[1]) / (lu * lv)).clamp(-1.0, 1.0).acos().to_degrees()
}

/// Ramer–Douglas–Peucker simplification of an open path (iterative: no deep recursion on long
/// strokes); keeps both ends.
fn rdp(p: &[[f32; 2]], eps: f32) -> Vec<[f32; 2]> {
    let n = p.len();
    if n < 3 {
        return p.to_vec();
    }
    let mut keep = vec![false; n];
    if let Some(f) = keep.first_mut() {
        *f = true;
    }
    if let Some(l) = keep.last_mut() {
        *l = true;
    }
    let mut stack = vec![(0usize, n - 1)];
    while let Some((a, b)) = stack.pop() {
        let (Some(&pa), Some(&pb)) = (p.get(a), p.get(b)) else { continue };
        let far = (a + 1..b).filter_map(|i| Some((i, seg_distance(pa, pb, *p.get(i)?)))).fold((a, 0.0), |m, x| if x.1 > m.1 { x } else { m });
        if far.1 > eps
            && let Some(k) = keep.get_mut(far.0)
        {
            *k = true;
            stack.push((a, far.0));
            stack.push((far.0, b));
        }
    }
    p.iter().zip(keep).filter(|(_, k)| *k).map(|(q, _)| *q).collect()
}

/// A quadrilateral whose sides run (nearly) across and down the page, alternately.
fn upright(c: &[[f32; 2]]) -> bool {
    let n = c.len();
    let dirs: Vec<Option<bool>> = (0..n)
        .map(|i| {
            let (a, b) = (c.get(i)?, c.get((i + 1) % n)?);
            let deg = (b[1] - a[1]).atan2(b[0] - a[0]).to_degrees().rem_euclid(180.0);
            // true: across, false: down; None when it's slanted.
            if deg <= 15.0 || deg >= 165.0 {
                Some(true)
            } else if (75.0..=105.0).contains(&deg) {
                Some(false)
            } else {
                None
            }
        })
        .collect();
    dirs.iter().all(Option::is_some) && (0..n).all(|i| dirs.get(i) != dirs.get((i + 1) % n))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Points along a closed polygon, `per` to a side, with a little deterministic hand wobble.
    fn around(corners: &[[f32; 2]], per: usize) -> Vec<[f32; 2]> {
        let mut out = Vec::new();
        for (i, a) in corners.iter().enumerate() {
            let b = corners[(i + 1) % corners.len()];
            for k in 0..per {
                let t = k as f32 / per as f32;
                let wob = (out.len() as f32 * 1.7).sin() * 0.6;
                out.push([a[0] + (b[0] - a[0]) * t + wob, a[1] + (b[1] - a[1]) * t - wob]);
            }
        }
        out.push(corners[0]);
        out
    }

    #[test]
    fn rectangles_ellipses_lines_and_triangles_are_recognised() {
        // A rectangle, started mid-side as people often do.
        let rect = around(&[[50.0, 10.0], [110.0, 10.0], [110.0, 70.0 + 0.0], [10.0, 70.0], [10.0, 10.0]], 12);
        let Some(InkShape::Rectangle { x, y, w, h }) = recognise(&rect) else { panic!("{:?}", recognise(&rect)) };
        assert!((x - 10.0).abs() < 2.0 && (y - 10.0).abs() < 2.0 && (w - 100.0).abs() < 3.0 && (h - 60.0).abs() < 3.0, "{x} {y} {w} {h}");
        // Nearly square: a square.
        assert_eq!(recognise(&around(&[[0.0, 0.0], [60.0, 0.0], [60.0, 57.0], [0.0, 57.0]], 10)).map(|s| s.name()), Some("square"));
        // An ellipse and a circle.
        let ring = |rx: f32, ry: f32| {
            (0..=64).map(|i| (i as f32 / 64.0) * std::f32::consts::TAU).map(|a| [100.0 + rx * a.cos(), 80.0 + ry * a.sin()]).collect::<Vec<_>>()
        };
        let Some(InkShape::Ellipse { x, w, h, .. }) = recognise(&ring(40.0, 25.0)) else { panic!() };
        assert!((x - 60.0).abs() < 0.5 && (w - 80.0).abs() < 0.5 && (h - 50.0).abs() < 0.5);
        assert_eq!(recognise(&ring(30.0, 29.0)).map(|s| s.name()), Some("circle"));
        // A slightly shaky line keeps its ends.
        let line: Vec<[f32; 2]> = (0..=40).map(|i| i as f32).map(|t| [10.0 + t * 3.0, 20.0 + t + (t * 2.1).sin() * 0.8]).collect();
        assert_eq!(recognise(&line), Some(InkShape::Line { a: line[0], b: line[40] }));
        // A triangle and a pentagon.
        let tri = around(&[[0.0, 80.0], [45.0, 0.0], [90.0, 80.0]], 15);
        let Some(InkShape::Polygon(c)) = recognise(&tri) else { panic!("{:?}", recognise(&tri)) };
        assert_eq!(c.len(), 3);
        let penta: Vec<[f32; 2]> = (0..5)
            .map(|i| (i as f32 / 5.0) * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2)
            .map(|a| [60.0 + 50.0 * a.cos(), 60.0 + 50.0 * a.sin()])
            .collect();
        assert_eq!(recognise(&around(&penta, 12)).map(|s| s.name()), Some("polygon"));
    }

    #[test]
    fn scribbles_ticks_and_hostile_points_stay_ink() {
        // An open zigzag, a hook, a tap and a tiny tick.
        let zigzag: Vec<[f32; 2]> = (0..20).map(|i| [i as f32 * 8.0, if i % 2 == 0 { 0.0 } else { 30.0 }]).collect();
        assert_eq!(recognise(&zigzag), None);
        assert_eq!(recognise(&[[0.0, 0.0], [50.0, 0.0], [50.0, 50.0]]), None);
        assert_eq!(recognise(&[[5.0, 5.0]]), None);
        assert_eq!(recognise(&[[0.0, 0.0], [2.0, 3.0]]), None);
        // A closed scribble with many spikes: not a polygon of eight or fewer corners, not round.
        let star: Vec<[f32; 2]> = (0..=24)
            .map(|i| {
                let a = i as f32 / 24.0 * std::f32::consts::TAU;
                let r = if i % 2 == 0 { 60.0 } else { 20.0 };
                [80.0 + r * a.cos(), 80.0 + r * a.sin()]
            })
            .collect();
        assert_eq!(recognise(&star), None);
        // NaN, infinities and a flood of points never panic.
        let mut junk = vec![[f32::NAN, 1.0], [f32::INFINITY, 2.0], [3.0, f32::NEG_INFINITY]];
        junk.extend(std::iter::repeat_n([1.0, 1.0], MAX_POINTS * 3));
        assert_eq!(recognise(&junk), None);
        assert_eq!(recognise(&[]), None);
    }

    /// A stroke drawn as a shaft from `tail` to `tip`, then both barbs of a head (back to the
    /// tip between them).
    fn arrow_stroke(tail: [f32; 2], tip: [f32; 2], barb: f32, spread_deg: f32) -> Vec<[f32; 2]> {
        let back = (tail[1] - tip[1]).atan2(tail[0] - tip[0]);
        let barb_end = |sign: f32| {
            let a = back + sign * spread_deg.to_radians();
            [tip[0] + barb * a.cos(), tip[1] + barb * a.sin()]
        };
        let line = |a: [f32; 2], b: [f32; 2], n: usize| -> Vec<[f32; 2]> {
            (0..n).map(|k| k as f32 / n as f32).map(|t| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]).collect()
        };
        let mut pts = line(tail, tip, 24);
        pts.extend(line(tip, barb_end(1.0), 5));
        pts.extend(line(barb_end(1.0), tip, 5));
        pts.extend(line(tip, barb_end(-1.0), 5));
        pts.push(barb_end(-1.0));
        pts
    }

    #[test]
    fn arrows_drawn_in_one_stroke_are_recognised_in_any_direction() {
        for (tail, tip) in [
            ([20.0, 100.0], [180.0, 100.0]),
            ([180.0, 100.0], [20.0, 100.0]),
            ([100.0, 20.0], [100.0, 180.0]),
            ([100.0, 180.0], [100.0, 20.0]),
            ([30.0, 30.0], [150.0, 120.0]),
            ([150.0, 120.0], [30.0, 30.0]),
        ] {
            let stroke = arrow_stroke(tail, tip, 25.0, 30.0);
            let Some(InkShape::Arrow { a, b, head_at_end, head_at_start }) = recognise(&stroke) else {
                panic!("{tail:?}->{tip:?}: {:?}", recognise(&stroke))
            };
            assert!(head_at_end && !head_at_start);
            assert!(dist(a, tail) < 3.0 && dist(b, tip) < 3.0, "{a:?} {b:?} for {tail:?} {tip:?}");
            assert_eq!(recognise(&stroke).map(|s| s.name()), Some("arrow"));
        }
        // Head first, then the shaft: the same arrow.
        let mut reversed = arrow_stroke([20.0, 100.0], [180.0, 100.0], 25.0, 30.0);
        reversed.reverse();
        let Some(InkShape::Arrow { a, b, .. }) = recognise(&reversed) else { panic!("{:?}", recognise(&reversed)) };
        assert!(dist(a, [20.0, 100.0]) < 3.0 && dist(b, [180.0, 100.0]) < 3.0, "{a:?} {b:?}");
    }

    #[test]
    fn lines_vs_barbs_hooks_and_scribbles_are_not_arrows() {
        let stroke = arrow_stroke([20.0, 100.0], [180.0, 100.0], 25.0, 30.0);
        // A plain line stays a line.
        let line: Vec<[f32; 2]> = (0..=30).map(|i| [20.0 + i as f32 * 5.0, 100.0]).collect();
        assert_eq!(recognise(&line).map(|s| s.name()), Some("line"));
        // A "V" alone is not an arrow (nor anything).
        let v = [[100.0, 100.0], [75.0, 87.5], [100.0, 100.0], [75.0, 112.5]];
        assert_eq!(recognise(&v), None);
        assert_eq!(recognise(&[[75.0, 87.5], [100.0, 100.0], [75.0, 112.5]]), None);
        // A line with one barb (a hook) is not an arrow.
        let hook = &stroke[..34];
        assert!(!matches!(recognise(hook), Some(InkShape::Arrow { .. })), "{:?}", recognise(hook));
        // Barbs outside 15..75 degrees, or too long or too short, are not heads.
        for (barb, spread) in [(25.0, 5.0), (25.0, 88.0), (100.0, 30.0), (1.0, 30.0)] {
            let s = arrow_stroke([20.0, 100.0], [180.0, 100.0], barb, spread);
            assert!(!matches!(recognise(&s), Some(InkShape::Arrow { .. })), "barb {barb} spread {spread}");
        }
        // Both barbs on the same side are not a head.
        let lopsided = [[20.0, 100.0], [180.0, 100.0], [155.0, 87.0], [180.0, 100.0], [158.0, 80.0]];
        assert!(!matches!(recognise(&lopsided), Some(InkShape::Arrow { .. })));
        // A zigzag scribble stays ink.
        let zig: Vec<[f32; 2]> = (0..9).map(|i| [i as f32 * 20.0, if i % 2 == 0 { 0.0 } else { 40.0 }]).collect();
        assert_eq!(recognise(&zig), None);
    }

    #[test]
    fn hostile_arrow_points_never_panic() {
        let mut junk = arrow_stroke([20.0, 100.0], [180.0, 100.0], 25.0, 30.0);
        junk.insert(3, [f32::NAN, 5.0]);
        junk.push([f32::INFINITY, 1.0]);
        assert!(matches!(recognise(&junk), Some(InkShape::Arrow { .. })));
        let huge: Vec<[f32; 2]> = (0..MAX_POINTS * 2).map(|i| [(i % 977) as f32, ((i * 31) % 541) as f32]).collect();
        let _ = recognise(&huge);
        let _ = recognise(&[[1e5, 1e5], [-1e5, 1e5], [1e5, -1e5], [-1e5, -1e5], [0.0, 0.0]]);
        let _ = recognise(&[[0.0, 0.0]; 7]);
    }
}

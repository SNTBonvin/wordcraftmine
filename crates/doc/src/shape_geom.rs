//! The outline of every preset shape, in one place.
//!
//! [`shape_geom`] is the single source of truth for a preset shape's geometry: the gallery
//! draws it, the raster/PDF renderers fall back to it, and the wrapper falls back to it for
//! Tight/Through outlines. The outlines here are the preset shapes at their default
//! adjustments; only the round- and snip-corner rectangles honour `adj` (the corner radius as
//! a fraction of the shorter side) and NoSmoking (its bar thickness), because those are the
//! shapes whose adjustment a user drags or a file stores. Interior detail lines (a
//! predefined process's side bars, a callout's leader, a cube's inner edges, a frame's inner
//! rectangle) are separate open sub-paths; the gallery draws them, the renderers stroke
//! them, the wrapper ignores them (it takes the silhouette). The one interior part that is a
//! filled area, not a line — NoSmoking's bar — is a closed sub-path after the circle, so it
//! fills with the shape and strokes as an outlined bar under No Fill. The exact curve
//! control points are our own approximation of the preset geometry, not copied from any
//! other implementation.

use crate::connector::connector_segs;
use crate::para::ShapeKind;

/// One sub-path of a shape's outline: points in unit coordinates (0..1 across the frame),
/// `closed` (an area, filled) or open (a stroke, never filled).
#[derive(Clone, Debug, PartialEq)]
pub struct ShapePath {
    pub pts: Vec<(f32, f32)>,
    pub closed: bool,
}

fn closed(pts: Vec<(f32, f32)>) -> ShapePath {
    ShapePath { pts, closed: true }
}

fn open(pts: Vec<(f32, f32)>) -> ShapePath {
    ShapePath { pts, closed: false }
}

/// The internal marks of shapes whose outline alone doesn't identify them: the smiley's
/// face and the action buttons' glyphs. These are real geometric parts (they render on
/// the canvas, in PDF and in DOCX, and survive No Fill as stroked lines), not
/// gallery-only decoration. Unit coordinates, open polylines.
fn glyph_paths(kind: ShapeKind) -> Vec<ShapePath> {
    let line = |pts: &[(f32, f32)]| open(pts.to_vec());
    let mut paths = Vec::new();
    match kind {
        ShapeKind::SmileyFace => {
            // Eyes: two short vertical strokes. Smile: a curved arc, not a straight line.
            paths.push(line(&[(0.35, 0.35), (0.35, 0.42)]));
            paths.push(line(&[(0.65, 0.35), (0.65, 0.42)]));
            paths.push(open(arc_pts(0.5, 0.42, 0.22, 0.22, 0.2 * std::f32::consts::PI, 0.8 * std::f32::consts::PI, 16)));
        }
        ShapeKind::ActionButtonHome => {
            paths.push(line(&[(0.3, 0.7), (0.3, 0.4), (0.5, 0.25), (0.7, 0.4), (0.7, 0.7)]));
            paths.push(line(&[(0.42, 0.7), (0.42, 0.55), (0.58, 0.55), (0.58, 0.7)]));
        }
        ShapeKind::ActionButtonHelp => {
            paths.push(line(&[(0.38, 0.32), (0.5, 0.25), (0.62, 0.32), (0.55, 0.45), (0.5, 0.55), (0.5, 0.62)]));
            paths.push(line(&[(0.5, 0.72), (0.5, 0.75)]));
        }
        ShapeKind::ActionButtonInformation => {
            paths.push(line(&[(0.5, 0.28), (0.5, 0.32)]));
            paths.push(line(&[(0.5, 0.42), (0.5, 0.72)]));
        }
        ShapeKind::ActionButtonForwardNext => {
            paths.push(line(&[(0.3, 0.3), (0.62, 0.5), (0.3, 0.7)]));
            paths.push(line(&[(0.7, 0.3), (0.7, 0.7)]));
        }
        ShapeKind::ActionButtonBackPrevious => {
            paths.push(line(&[(0.7, 0.3), (0.38, 0.5), (0.7, 0.7)]));
            paths.push(line(&[(0.3, 0.3), (0.3, 0.7)]));
        }
        ShapeKind::ActionButtonEnd => {
            paths.push(line(&[(0.3, 0.3), (0.7, 0.3), (0.7, 0.7), (0.3, 0.7)]));
            paths.push(line(&[(0.3, 0.3), (0.3, 0.7)]));
        }
        ShapeKind::ActionButtonBeginning => {
            paths.push(line(&[(0.3, 0.3), (0.7, 0.3), (0.7, 0.7), (0.3, 0.7)]));
            paths.push(line(&[(0.7, 0.3), (0.7, 0.7)]));
        }
        ShapeKind::ActionButtonReturn => {
            paths.push(line(&[(0.7, 0.3), (0.35, 0.3), (0.35, 0.7), (0.55, 0.7)]));
            paths.push(line(&[(0.35, 0.3), (0.2, 0.45), (0.35, 0.6)]));
        }
        ShapeKind::ActionButtonDocument => {
            paths.push(line(&[(0.35, 0.25), (0.6, 0.25), (0.65, 0.3), (0.65, 0.75), (0.35, 0.75), (0.35, 0.25)]));
            paths.push(line(&[(0.42, 0.4), (0.58, 0.4)]));
            paths.push(line(&[(0.42, 0.5), (0.58, 0.5)]));
            paths.push(line(&[(0.42, 0.6), (0.58, 0.6)]));
        }
        ShapeKind::ActionButtonSound => {
            paths.push(line(&[(0.3, 0.45), (0.42, 0.45), (0.42, 0.55), (0.3, 0.55), (0.3, 0.45)]));
            paths.push(line(&[(0.5, 0.38), (0.6, 0.28)]));
            paths.push(line(&[(0.5, 0.5), (0.65, 0.5)]));
            paths.push(line(&[(0.5, 0.62), (0.6, 0.72)]));
        }
        ShapeKind::ActionButtonMovie => {
            paths.push(line(&[(0.28, 0.3), (0.72, 0.3), (0.72, 0.7), (0.28, 0.7), (0.28, 0.3)]));
            paths.push(line(&[(0.42, 0.38), (0.42, 0.62), (0.62, 0.5), (0.42, 0.38)]));
        }
        _ => {}
    }
    paths
}

/// The outline of `kind` in a `w` × `h` frame: its sub-paths in unit coordinates. `None`
/// for shapes whose outline is their frame (rectangle, text box, freeform). `adj` is the
/// shape's adjustment: the corner radius as a fraction of the shorter side for the
/// round- and snip-corner rectangles, the bar thickness for NoSmoking; other shapes ignore
/// it.
pub fn shape_geom(kind: ShapeKind, w: f32, h: f32, adj: f32) -> Option<Vec<ShapePath>> {
    let adj = if adj.is_finite() { adj.clamp(0.0, 0.5) } else { default_adj(kind) };
    let paths = match kind {
        // Lines and connectors: one open path from their segments.
        ShapeKind::Line
        | ShapeKind::LineInv
        | ShapeKind::StraightConnector
        | ShapeKind::ElbowConnector
        | ShapeKind::CurvedConnector
        | ShapeKind::BentConnector2
        | ShapeKind::BentConnector4
        | ShapeKind::BentConnector5
        | ShapeKind::CurvedConnector2
        | ShapeKind::CurvedConnector4
        | ShapeKind::CurvedConnector5 => vec![connector_path(kind)],
        ShapeKind::Rectangle | ShapeKind::TextBox | ShapeKind::Freeform => return None,
        ShapeKind::RoundedRectangle => vec![closed(round_corners_per(r4(w, h, adj)))],
        ShapeKind::Round1Rect => vec![closed(round_corners_per((corner_radius(w, h, adj), (0.0, 0.0), (0.0, 0.0), (0.0, 0.0))))],
        ShapeKind::Round2SameRect => vec![closed(round_corners_per((corner_radius(w, h, adj), corner_radius(w, h, adj), (0.0, 0.0), (0.0, 0.0))))],
        ShapeKind::Round2DiagRect => vec![closed(round_corners_per((corner_radius(w, h, adj), (0.0, 0.0), corner_radius(w, h, adj), (0.0, 0.0))))],
        ShapeKind::Snip1Rect => vec![closed(snip_corners(corner_radius(w, h, adj), (0.0, 0.0), (0.0, 0.0), (0.0, 0.0)))],
        ShapeKind::Snip2SameRect => vec![closed(snip_corners(corner_radius(w, h, adj), corner_radius(w, h, adj), (0.0, 0.0), (0.0, 0.0)))],
        ShapeKind::Snip2DiagRect => vec![closed(snip_corners(corner_radius(w, h, adj), (0.0, 0.0), corner_radius(w, h, adj), (0.0, 0.0)))],
        ShapeKind::SnipRoundRect => vec![closed(mixed_corners(corner_radius(w, h, adj), corner_radius(w, h, adj)))],
        ShapeKind::Plaque => {
            vec![closed(snip_corners(corner_radius(w, h, 0.12), corner_radius(w, h, 0.12), corner_radius(w, h, 0.12), corner_radius(w, h, 0.12)))]
        }
        ShapeKind::Ellipse => vec![closed(ellipse_pts(0.5, 0.5, 0.5, 0.5, 48))],
        ShapeKind::Triangle => vec![closed(vec![(0.5, 0.0), (1.0, 1.0), (0.0, 1.0)])],
        ShapeKind::RightTriangle => vec![closed(vec![(0.0, 0.0), (1.0, 1.0), (0.0, 1.0)])],
        ShapeKind::Diamond => vec![closed(vec![(0.5, 0.0), (1.0, 0.5), (0.5, 1.0), (0.0, 0.5)])],
        ShapeKind::Parallelogram => vec![closed(vec![(0.2, 0.0), (1.0, 0.0), (0.8, 1.0), (0.0, 1.0)])],
        ShapeKind::Trapezoid => vec![closed(vec![(0.2, 0.0), (0.8, 0.0), (1.0, 1.0), (0.0, 1.0)])],
        ShapeKind::NonIsoscelesTrapezoid => vec![closed(vec![(0.0, 0.0), (0.7, 0.0), (1.0, 1.0), (0.0, 1.0)])],
        ShapeKind::Pentagon => vec![closed(ngon(5))],
        ShapeKind::Hexagon => vec![closed(ngon(6))],
        ShapeKind::Heptagon => vec![closed(ngon(7))],
        ShapeKind::Octagon => vec![closed(ngon(8))],
        ShapeKind::Decagon => vec![closed(ngon(10))],
        ShapeKind::Dodecagon => vec![closed(ngon(12))],
        ShapeKind::Teardrop => vec![closed(teardrop())],
        ShapeKind::HomePlate => vec![closed(vec![(0.0, 0.0), (0.7, 0.0), (1.0, 0.5), (0.7, 1.0), (0.0, 1.0)])],
        ShapeKind::Chevron => vec![closed(vec![(0.0, 0.0), (0.7, 0.0), (1.0, 0.5), (0.7, 1.0), (0.0, 1.0), (0.3, 0.5)])],
        ShapeKind::Pie => vec![closed(pie())],
        ShapeKind::PieWedge => vec![closed(pie_wedge())],
        ShapeKind::BlockArc => vec![closed(block_arc())],
        ShapeKind::Donut => vec![closed(donut())],
        ShapeKind::NoSmoking => vec![closed(ellipse_pts(0.5, 0.5, 0.5, 0.5, 48)), closed(no_smoking_bar(adj))],
        ShapeKind::Frame => {
            vec![
                closed(vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]),
                open(vec![(0.15, 0.15), (0.85, 0.15), (0.85, 0.85), (0.15, 0.85), (0.15, 0.15)]),
            ]
        }
        ShapeKind::HalfFrame => {
            vec![closed(vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]), open(vec![(0.15, 0.15), (0.85, 0.15), (0.85, 0.85), (0.15, 0.85)])]
        }
        ShapeKind::Corner => vec![closed(vec![(0.0, 0.0), (1.0, 0.0), (1.0, 0.25), (0.25, 0.25), (0.25, 1.0), (0.0, 1.0)])],
        ShapeKind::DiagStripe => vec![closed(vec![(0.0, 0.25), (0.75, 0.0), (1.0, 0.0), (1.0, 0.75), (0.25, 1.0), (0.0, 1.0)])],
        ShapeKind::Cross => vec![closed(cross())],
        ShapeKind::Plus => vec![closed(plus())],
        ShapeKind::Cloud => vec![closed(cloud())],
        ShapeKind::FoldedCorner => {
            vec![closed(vec![(0.0, 0.0), (0.72, 0.0), (1.0, 0.28), (1.0, 1.0), (0.0, 1.0)]), open(vec![(0.72, 0.0), (0.72, 0.28), (1.0, 0.28)])]
        }
        ShapeKind::SmileyFace => vec![closed(ellipse_pts(0.5, 0.5, 0.5, 0.5, 48))],
        ShapeKind::LightningBolt => vec![closed(vec![(0.55, 0.0), (0.2, 0.55), (0.45, 0.55), (0.35, 1.0), (0.85, 0.4), (0.55, 0.4)])],
        ShapeKind::Sun => vec![closed(sun())],
        ShapeKind::Moon => vec![closed(moon())],
        ShapeKind::Arc => vec![open(arc_pts(0.5, 0.5, 0.5, 0.5, 0.0, std::f32::consts::FRAC_PI_2, 24))],
        ShapeKind::Chord => vec![closed(chord())],
        ShapeKind::Can => vec![closed(can())],
        ShapeKind::Cube => {
            vec![closed(cube()), open(vec![(0.5, 0.5), (0.5, 0.0)]), open(vec![(0.5, 0.5), (1.0, 0.25)]), open(vec![(0.5, 0.5), (0.0, 0.25)])]
        }
        ShapeKind::Bevel => {
            vec![closed(snip_corners(corner_radius(w, h, 0.06), corner_radius(w, h, 0.06), corner_radius(w, h, 0.06), corner_radius(w, h, 0.06)))]
        }
        ShapeKind::Funnel => vec![closed(vec![(0.0, 0.0), (1.0, 0.0), (0.7, 0.45), (0.7, 1.0), (0.3, 1.0), (0.3, 0.45)])],
        ShapeKind::Gear6 => vec![closed(gear(6))],
        ShapeKind::Gear9 => vec![closed(gear(9))],
        ShapeKind::Ribbon => vec![closed(vec![
            (0.0, 0.0),
            (1.0, 0.0),
            (1.0, 0.3),
            (0.85, 0.5),
            (1.0, 0.7),
            (1.0, 1.0),
            (0.0, 1.0),
            (0.0, 0.7),
            (0.15, 0.5),
            (0.0, 0.3),
        ])],
        ShapeKind::Ribbon2 => vec![closed(vec![(0.0, 0.0), (0.9, 0.0), (1.0, 0.5), (0.9, 1.0), (0.0, 1.0), (0.0, 0.7), (0.15, 0.5), (0.0, 0.3)])],
        ShapeKind::EllipseRibbon => vec![closed(ellipse_ribbon(false))],
        ShapeKind::EllipseRibbon2 => vec![closed(ellipse_ribbon(true))],
        ShapeKind::VerticalScroll => vec![closed(scroll(true))],
        ShapeKind::HorizontalScroll => vec![closed(scroll(false))],
        ShapeKind::Wave => vec![closed(wave(1))],
        ShapeKind::DoubleWave => vec![closed(wave(2))],
        ShapeKind::SquareTabs => vec![closed(square_tabs())],
        ShapeKind::ChartPlus => vec![closed(plus())],
        ShapeKind::ChartStar => vec![closed(star_pts(5, 0.42))],
        ShapeKind::ChartX => vec![closed(cross())],
        ShapeKind::LeftBracket => vec![open(vec![(0.3, 0.0), (0.0, 0.0), (0.0, 1.0), (0.3, 1.0)])],
        ShapeKind::RightBracket => vec![open(vec![(0.7, 0.0), (1.0, 0.0), (1.0, 1.0), (0.7, 1.0)])],
        ShapeKind::LeftBrace => vec![open(brace_at(true, 0.0, 1.0))],
        ShapeKind::RightBrace => vec![open(brace_at(false, 0.0, 1.0))],
        ShapeKind::LeftBracePair => vec![open(brace_at(true, 0.0, 0.4)), open(brace_at(true, 0.6, 1.0))],
        ShapeKind::RightBracePair => vec![open(brace_at(false, 0.0, 0.4)), open(brace_at(false, 0.6, 1.0))],
        ShapeKind::BracketPair => {
            vec![open(vec![(0.3, 0.0), (0.0, 0.0), (0.0, 1.0), (0.3, 1.0)]), open(vec![(0.7, 0.0), (1.0, 0.0), (1.0, 1.0), (0.7, 1.0)])]
        }
        ShapeKind::BracePair => vec![open(brace_at(true, 0.0, 0.4)), open(brace_at(false, 0.6, 1.0))],
        ShapeKind::Star => vec![closed(star_pts(5, 0.4))],
        ShapeKind::Star4 => vec![closed(star_pts(4, 0.42))],
        ShapeKind::Star6 => vec![closed(star_pts(6, 0.42))],
        ShapeKind::Star7 => vec![closed(star_pts(7, 0.42))],
        ShapeKind::Star8 => vec![closed(star_pts(8, 0.42))],
        ShapeKind::Star10 => vec![closed(star_pts(10, 0.42))],
        ShapeKind::Star12 => vec![closed(star_pts(12, 0.42))],
        ShapeKind::Star16 => vec![closed(star_pts(16, 0.42))],
        ShapeKind::Star24 => vec![closed(star_pts(24, 0.42))],
        ShapeKind::Star32 => vec![closed(star_pts(32, 0.42))],
        ShapeKind::Heart => vec![closed(heart())],
        ShapeKind::Arrow => vec![closed(arrow_right(0.3, 0.35, 0.7))],
        ShapeKind::LeftArrow => vec![closed(mirror_x(arrow_right(0.3, 0.35, 0.7)))],
        ShapeKind::UpArrow => vec![closed(rot_ccw(arrow_right(0.3, 0.35, 0.7)))],
        ShapeKind::DownArrow => vec![closed(rot_cw(arrow_right(0.3, 0.35, 0.7)))],
        ShapeKind::LeftRightArrow => vec![closed(left_right_arrow())],
        ShapeKind::UpDownArrow => vec![closed(rot_cw(left_right_arrow()))],
        ShapeKind::QuadArrow => vec![closed(quad_arrow())],
        ShapeKind::CornerArrows => vec![closed(corner_arrows_ne()), closed(corner_arrows_sw())],
        ShapeKind::SplitArrow => vec![closed(split_arrow())],
        ShapeKind::UturnArrow => vec![closed(uturn_arrow())],
        ShapeKind::CircularArrow => vec![closed(circular_arrow(true))],
        ShapeKind::StripedRightArrow => {
            vec![
                closed(arrow_right(0.3, 0.35, 0.7)),
                open(vec![(0.15, 0.35), (0.15, 0.65)]),
                open(vec![(0.25, 0.35), (0.25, 0.65)]),
                open(vec![(0.35, 0.35), (0.35, 0.65)]),
            ]
        }
        ShapeKind::NotchedRightArrow => vec![closed(notched_arrow())],
        ShapeKind::BentUpArrow => vec![closed(bent_up_arrow())],
        ShapeKind::BentArrow => vec![closed(bent_arrow())],
        ShapeKind::LeftRightCircularArrow => vec![closed(circular_arrow(true)), closed(circular_arrow(false))],
        ShapeKind::SwooshArrow => vec![closed(swoosh_arrow())],
        ShapeKind::LeftCircularArrow => vec![closed(circular_arrow(false))],
        ShapeKind::RightCircularArrow => vec![closed(circular_arrow(true))],
        ShapeKind::FlowChartProcess => vec![closed(vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)])],
        ShapeKind::FlowChartDecision => vec![closed(vec![(0.5, 0.0), (1.0, 0.5), (0.5, 1.0), (0.0, 0.5)])],
        ShapeKind::FlowChartInputOutput => vec![closed(vec![(0.2, 0.0), (1.0, 0.0), (0.8, 1.0), (0.0, 1.0)])],
        ShapeKind::FlowChartPredefinedProcess => vec![
            closed(vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]),
            open(vec![(0.12, 0.0), (0.12, 1.0)]),
            open(vec![(0.88, 0.0), (0.88, 1.0)]),
        ],
        ShapeKind::FlowChartInternalStorage => vec![
            closed(vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]),
            open(vec![(0.0, 0.25), (1.0, 0.25)]),
            open(vec![(0.25, 0.0), (0.25, 1.0)]),
        ],
        ShapeKind::FlowChartDocument => vec![closed(document_at(0.0, 0.0, 1.0, 1.0))],
        ShapeKind::FlowChartMultidocument => {
            vec![closed(document_at(0.0, 0.0, 0.84, 0.84)), closed(document_at(0.08, 0.08, 0.92, 0.92)), closed(document_at(0.16, 0.16, 1.0, 1.0))]
        }
        ShapeKind::FlowChartTerminator => vec![closed(round_corners_per(((0.5, 0.5), (0.5, 0.5), (0.5, 0.5), (0.5, 0.5))))],
        ShapeKind::FlowChartPreparation => vec![closed(vec![(0.15, 0.0), (0.85, 0.0), (1.0, 0.5), (0.85, 1.0), (0.15, 1.0), (0.0, 0.5)])],
        ShapeKind::FlowChartManualInput => vec![closed(vec![(0.0, 0.25), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)])],
        ShapeKind::FlowChartManualOperation => vec![closed(vec![(0.2, 0.0), (0.8, 0.0), (1.0, 1.0), (0.0, 1.0)])],
        ShapeKind::FlowChartConnector => vec![closed(ellipse_pts(0.5, 0.5, 0.5, 0.5, 48))],
        ShapeKind::FlowChartPunchedCard => vec![closed(vec![(0.0, 0.0), (0.8, 0.0), (1.0, 0.25), (1.0, 1.0), (0.0, 1.0)])],
        ShapeKind::FlowChartPunchedTape => vec![closed(punched_tape())],
        ShapeKind::FlowChartSummingJunction => {
            vec![closed(ellipse_pts(0.5, 0.5, 0.5, 0.5, 48)), open(vec![(0.15, 0.15), (0.85, 0.85)]), open(vec![(0.85, 0.15), (0.15, 0.85)])]
        }
        ShapeKind::FlowChartOr => {
            vec![closed(ellipse_pts(0.5, 0.5, 0.5, 0.5, 48)), open(vec![(0.5, 0.15), (0.5, 0.85)]), open(vec![(0.15, 0.5), (0.85, 0.5)])]
        }
        ShapeKind::FlowChartCollate => vec![closed(vec![(0.0, 0.0), (1.0, 0.5), (0.0, 1.0)]), open(vec![(0.2, 0.15), (0.2, 0.85)])],
        ShapeKind::FlowChartSort => vec![closed(vec![(0.0, 0.0), (1.0, 0.0), (0.5, 1.0)])],
        ShapeKind::FlowChartExtract => vec![closed(vec![(0.0, 0.0), (1.0, 0.0), (0.5, 1.0)]), open(vec![(0.2, 0.2), (0.8, 0.2)])],
        ShapeKind::FlowChartMerge => vec![closed(vec![(0.0, 1.0), (1.0, 1.0), (0.5, 0.0)])],
        ShapeKind::FlowChartOfflineStorage => vec![closed(vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]), open(vec![(0.0, 1.0), (1.0, 0.0)])],
        ShapeKind::FlowChartOnlineStorage => vec![closed(online_storage())],
        ShapeKind::FlowChartMagneticTape => vec![closed(side_bulge(0.25))],
        ShapeKind::FlowChartMagneticDisk => vec![closed(can())],
        ShapeKind::FlowChartMagneticDrum => vec![closed(drum())],
        ShapeKind::FlowChartDisplay => vec![closed(side_bulge(0.12))],
        ShapeKind::FlowChartDelay => vec![closed(side_bulge(0.4))],
        ShapeKind::FlowChartAlternateProcess => vec![closed(round_corners_per(r4(w, h, adj)))],
        ShapeKind::FlowChartOffpageConnector => vec![closed(vec![(0.0, 0.0), (1.0, 0.0), (1.0, 0.7), (0.5, 1.0), (0.0, 0.7)])],
        ShapeKind::Callout1 => vec![closed(vec![(0.1, 0.0), (1.0, 0.0), (1.0, 0.8), (0.1, 0.8)]), open(vec![(0.35, 0.8), (0.1, 1.0)])],
        ShapeKind::Callout2 => vec![closed(vec![(0.1, 0.0), (1.0, 0.0), (1.0, 0.8), (0.1, 0.8)]), open(vec![(0.35, 0.8), (0.2, 0.95), (0.05, 0.95)])],
        ShapeKind::Callout3 => {
            vec![closed(vec![(0.1, 0.0), (1.0, 0.0), (1.0, 0.8), (0.1, 0.8)]), open(vec![(0.35, 0.8), (0.25, 0.9), (0.15, 0.9), (0.05, 1.0)])]
        }
        ShapeKind::AccentCallout1 => vec![
            closed(vec![(0.1, 0.0), (1.0, 0.0), (1.0, 0.8), (0.1, 0.8)]),
            open(vec![(0.35, 0.8), (0.1, 1.0)]),
            open(vec![(0.18, 0.1), (0.18, 0.7)]),
        ],
        ShapeKind::AccentCallout2 => vec![
            closed(vec![(0.1, 0.0), (1.0, 0.0), (1.0, 0.8), (0.1, 0.8)]),
            open(vec![(0.35, 0.8), (0.2, 0.95), (0.05, 0.95)]),
            open(vec![(0.18, 0.1), (0.18, 0.7)]),
        ],
        ShapeKind::AccentCallout3 => vec![
            closed(vec![(0.1, 0.0), (1.0, 0.0), (1.0, 0.8), (0.1, 0.8)]),
            open(vec![(0.35, 0.8), (0.25, 0.9), (0.15, 0.9), (0.05, 1.0)]),
            open(vec![(0.18, 0.1), (0.18, 0.7)]),
        ],
        ShapeKind::BorderCallout1 => vec![closed(vec![(0.1, 0.0), (1.0, 0.0), (1.0, 0.8), (0.1, 0.8)]), open(vec![(0.35, 0.8), (0.1, 1.0)])],
        ShapeKind::BorderCallout2 => {
            vec![closed(vec![(0.1, 0.0), (1.0, 0.0), (1.0, 0.8), (0.1, 0.8)]), open(vec![(0.35, 0.8), (0.2, 0.95), (0.05, 0.95)])]
        }
        ShapeKind::BorderCallout3 => {
            vec![closed(vec![(0.1, 0.0), (1.0, 0.0), (1.0, 0.8), (0.1, 0.8)]), open(vec![(0.35, 0.8), (0.25, 0.9), (0.15, 0.9), (0.05, 1.0)])]
        }
        ShapeKind::AccentBorderCallout1 => vec![
            closed(vec![(0.1, 0.0), (1.0, 0.0), (1.0, 0.8), (0.1, 0.8)]),
            open(vec![(0.35, 0.8), (0.1, 1.0)]),
            open(vec![(0.18, 0.1), (0.18, 0.7)]),
        ],
        ShapeKind::AccentBorderCallout2 => vec![
            closed(vec![(0.1, 0.0), (1.0, 0.0), (1.0, 0.8), (0.1, 0.8)]),
            open(vec![(0.35, 0.8), (0.2, 0.95), (0.05, 0.95)]),
            open(vec![(0.18, 0.1), (0.18, 0.7)]),
        ],
        ShapeKind::AccentBorderCallout3 => vec![
            closed(vec![(0.1, 0.0), (1.0, 0.0), (1.0, 0.8), (0.1, 0.8)]),
            open(vec![(0.35, 0.8), (0.25, 0.9), (0.15, 0.9), (0.05, 1.0)]),
            open(vec![(0.18, 0.1), (0.18, 0.7)]),
        ],
        ShapeKind::WedgeRectCallout => vec![closed(vec![(0.0, 0.0), (1.0, 0.0), (1.0, 0.75), (0.55, 0.75), (0.45, 1.0), (0.25, 0.75), (0.0, 0.75)])],
        ShapeKind::WedgeRoundRectCallout => vec![closed(wedge_round_rect(corner_radius(w, h, adj)))],
        ShapeKind::WedgeEllipseCallout => vec![closed(wedge_ellipse())],
        ShapeKind::CloudCallout => {
            vec![closed(cloud()), closed(ellipse_pts(0.3, 0.9, 0.08, 0.05, 16)), closed(ellipse_pts(0.15, 0.97, 0.04, 0.03, 12))]
        }
        ShapeKind::ActionButtonBlank => vec![closed(round_corners_per(r4(w, h, 0.1)))],
        ShapeKind::ActionButtonHome => vec![closed(round_corners_per(r4(w, h, 0.1)))],
        ShapeKind::ActionButtonHelp => vec![closed(round_corners_per(r4(w, h, 0.1)))],
        ShapeKind::ActionButtonInformation => vec![closed(round_corners_per(r4(w, h, 0.1)))],
        ShapeKind::ActionButtonForwardNext => vec![closed(round_corners_per(r4(w, h, 0.1)))],
        ShapeKind::ActionButtonBackPrevious => vec![closed(round_corners_per(r4(w, h, 0.1)))],
        ShapeKind::ActionButtonEnd => vec![closed(round_corners_per(r4(w, h, 0.1)))],
        ShapeKind::ActionButtonBeginning => vec![closed(round_corners_per(r4(w, h, 0.1)))],
        ShapeKind::ActionButtonReturn => vec![closed(round_corners_per(r4(w, h, 0.1)))],
        ShapeKind::ActionButtonDocument => vec![closed(round_corners_per(r4(w, h, 0.1)))],
        ShapeKind::ActionButtonSound => vec![closed(round_corners_per(r4(w, h, 0.1)))],
        ShapeKind::ActionButtonMovie => vec![closed(round_corners_per(r4(w, h, 0.1)))],
    };
    // The internal marks (smiley face, action-button glyphs) are part of the shape.
    let mut paths = paths;
    paths.extend(glyph_paths(kind));
    Some(paths)
}

/// The default adjustment: the corner radius as a fraction of the shorter side, matching the
/// `0.16` the renderers used before adjustments existed (and close to the preset default of
/// 16667/100000).
pub const DEFAULT_ADJ: f32 = 0.16;

/// The preset-default adjustment for `kind`: the OOXML preset default for shapes whose
/// adjustment is not a corner radius (NoSmoking's bar thickness, 18750/100000);
/// [`DEFAULT_ADJ`] for everything else. Callers resolving a stored `None` adjustment
/// through this keep the preset's own default instead of the corner-radius default.
pub fn default_adj(kind: ShapeKind) -> f32 {
    match kind {
        ShapeKind::NoSmoking => 0.1875,
        _ => DEFAULT_ADJ,
    }
}

/// A connector or line as one open sub-path, from its segments flattened to points.
fn connector_path(kind: ShapeKind) -> ShapePath {
    let mut pts: Vec<(f32, f32)> = Vec::new();
    let mut cur = (0.0, 0.0);
    let segs = match kind {
        ShapeKind::Line => vec![crate::graphic::PathSeg::Move(0.0, 0.0), crate::graphic::PathSeg::Line(1.0, 1.0)],
        ShapeKind::LineInv => vec![crate::graphic::PathSeg::Move(0.0, 1.0), crate::graphic::PathSeg::Line(1.0, 0.0)],
        _ => connector_segs(kind, 1.0, 1.0).unwrap_or_default(),
    };
    for seg in segs {
        match seg {
            crate::graphic::PathSeg::Move(x, y) => {
                cur = (x, y);
                pts.push(cur);
            }
            crate::graphic::PathSeg::Line(x, y) => {
                cur = (x, y);
                pts.push(cur);
            }
            crate::graphic::PathSeg::Cubic(x1, y1, x2, y2, x, y) => {
                let (x0, y0) = cur;
                for i in 1..=16 {
                    let t = i as f32 / 16.0;
                    let mt = 1.0 - t;
                    let bx = mt * mt * mt * x0 + 3.0 * mt * mt * t * x1 + 3.0 * mt * t * t * x2 + t * t * t * x;
                    let by = mt * mt * mt * y0 + 3.0 * mt * mt * t * y1 + 3.0 * mt * t * t * y2 + t * t * t * y;
                    pts.push((bx, by));
                }
                cur = (x, y);
            }
            crate::graphic::PathSeg::Close => {}
        }
    }
    open(pts)
}

/// The corner radius, in unit x/y, for an adjustment `adj` (a fraction of the shorter side).
fn corner_radius(w: f32, h: f32, adj: f32) -> (f32, f32) {
    let m = w.min(h).max(0.0);
    let r = adj * m;
    let (w, h) = (if w > 0.0 { w } else { 1.0 }, if h > 0.0 { h } else { 1.0 });
    (r / w, r / h)
}

/// The same unit corner radius four times (TL, TR, BR, BL).
fn r4(w: f32, h: f32, adj: f32) -> ((f32, f32), (f32, f32), (f32, f32), (f32, f32)) {
    let r = corner_radius(w, h, adj);
    (r, r, r, r)
}

/// A rectangle with all four corners snipped by the given per-corner unit cut sizes
/// (TL, TR, BR, BL), each `(sx, sy)`; `(0, 0)` = sharp.
fn snip_corners(tl: (f32, f32), tr: (f32, f32), br: (f32, f32), bl: (f32, f32)) -> Vec<(f32, f32)> {
    vec![(tl.0, 0.0), (1.0 - tr.0, 0.0), (1.0, tr.1), (1.0, 1.0 - br.1), (1.0 - br.0, 1.0), (bl.0, 1.0), (0.0, 1.0 - bl.1), (0.0, tl.1)]
}

/// A rectangle whose corners are rounded by the given per-corner unit radii (0 = sharp).
/// The radii are `(rx, ry)` per corner in TL, TR, BR, BL order.
fn round_corners_per(corners: ((f32, f32), (f32, f32), (f32, f32), (f32, f32))) -> Vec<(f32, f32)> {
    let (tl, tr, br, bl) = corners;
    let mut pts = Vec::new();
    // Top edge, then each corner clockwise: arc (or the corner point when sharp).
    pts.push((tl.0, 0.0));
    pts.push((1.0 - tr.0, 0.0));
    push_corner(&mut pts, tr, (1.0 - tr.0, tr.1), 0.0);
    pts.push((1.0, 1.0 - br.1));
    push_corner(&mut pts, br, (1.0 - br.0, 1.0 - br.1), 1.0);
    pts.push((bl.0, 1.0));
    push_corner(&mut pts, bl, (bl.0, 1.0 - bl.1), 2.0);
    pts.push((0.0, tl.1));
    push_corner(&mut pts, tl, (tl.0, tl.1), 3.0);
    pts
}

/// Append a corner to `pts`: an arc for a rounded corner (starting just past the edge point
/// already pushed), or the corner point for a sharp one. `quarter` is 0 = TR, 1 = BR,
/// 2 = BL, 3 = TL.
fn push_corner(pts: &mut Vec<(f32, f32)>, radii: (f32, f32), centre: (f32, f32), quarter: f32) {
    let (rx, ry) = radii;
    if rx <= 0.0 || ry <= 0.0 {
        let corner = match quarter {
            0.0 => (1.0, 0.0),
            1.0 => (1.0, 1.0),
            2.0 => (0.0, 1.0),
            _ => (0.0, 0.0),
        };
        pts.push(corner);
        return;
    }
    let a0 = -std::f32::consts::FRAC_PI_2 + quarter * std::f32::consts::FRAC_PI_2;
    push_arc(pts, centre.0, centre.1, rx, ry, a0, a0 + std::f32::consts::FRAC_PI_2, 6, true);
}

/// A rectangle with the top-left corner snipped and the bottom-right rounded (snipRoundRect).
fn mixed_corners(snip: (f32, f32), round: (f32, f32)) -> Vec<(f32, f32)> {
    let mut pts = vec![(snip.0, 0.0), (1.0, 0.0), (1.0, 1.0 - round.1)];
    push_arc(&mut pts, 1.0 - round.0, 1.0 - round.1, round.0, round.1, 0.0, std::f32::consts::FRAC_PI_2, 6, true);
    pts.push((0.0, 1.0));
    pts.push((0.0, snip.1));
    pts
}

/// Append the points of an elliptical arc from `a0` to `a1` (radians, y-down: 0 = east,
/// increasing = clockwise), skipping the first point when asked.
fn push_arc(pts: &mut Vec<(f32, f32)>, cx: f32, cy: f32, rx: f32, ry: f32, a0: f32, a1: f32, n: usize, skip_first: bool) {
    let start = usize::from(skip_first);
    for i in start..=n {
        let t = i as f32 / n as f32;
        let a = a0 + (a1 - a0) * t;
        pts.push((cx + rx * a.cos(), cy + ry * a.sin()));
    }
}

/// The points of an elliptical arc from `a0` to `a1`, including both ends.
fn arc_pts(cx: f32, cy: f32, rx: f32, ry: f32, a0: f32, a1: f32, n: usize) -> Vec<(f32, f32)> {
    let mut pts = Vec::new();
    push_arc(&mut pts, cx, cy, rx, ry, a0, a1, n, false);
    pts
}

/// The points of a full ellipse (`n` points, no repeated start).
fn ellipse_pts(cx: f32, cy: f32, rx: f32, ry: f32, n: usize) -> Vec<(f32, f32)> {
    (0..n)
        .map(|i| {
            let a = 2.0 * std::f32::consts::PI * i as f32 / n as f32;
            (cx + rx * a.cos(), cy + ry * a.sin())
        })
        .collect()
}

/// A regular `n`-gon. Odd counts get a vertex at the top; even counts a flat top.
fn ngon(n: usize) -> Vec<(f32, f32)> {
    let start = if n % 2 == 1 { -std::f32::consts::FRAC_PI_2 } else { 0.0 };
    (0..n)
        .map(|i| {
            let a = start + 2.0 * std::f32::consts::PI * i as f32 / n as f32;
            (0.5 + 0.5 * a.cos(), 0.5 + 0.5 * a.sin())
        })
        .collect()
}

/// An `n`-point star (a vertex at the top), `inner` = inner radius as a fraction of the outer.
fn star_pts(n: usize, inner: f32) -> Vec<(f32, f32)> {
    (0..2 * n)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / n as f32;
            let r = if i % 2 == 0 { 0.5 } else { 0.5 * inner };
            (0.5 + r * a.cos(), 0.5 + r * a.sin())
        })
        .collect()
}

/// Append a cubic Bezier from `p0` to `p3` (controls `p1`, `p2`), skipping `p0` when asked.
fn push_cubic(pts: &mut Vec<(f32, f32)>, p0: (f32, f32), p1: (f32, f32), p2: (f32, f32), p3: (f32, f32), n: usize, skip_first: bool) {
    let start = usize::from(skip_first);
    for i in start..=n {
        let t = i as f32 / n as f32;
        let mt = 1.0 - t;
        let x = mt * mt * mt * p0.0 + 3.0 * mt * mt * t * p1.0 + 3.0 * mt * t * t * p2.0 + t * t * t * p3.0;
        let y = mt * mt * mt * p0.1 + 3.0 * mt * mt * t * p1.1 + 3.0 * mt * t * t * p2.1 + t * t * t * p3.1;
        pts.push((x, y));
    }
}

fn heart() -> Vec<(f32, f32)> {
    let mut pts = vec![(0.5, 0.95)];
    push_cubic(&mut pts, (0.5, 0.95), (0.0, 0.55), (0.0, 0.1), (0.25, 0.1), 12, true);
    push_cubic(&mut pts, (0.25, 0.1), (0.35, 0.0), (0.45, 0.15), (0.5, 0.35), 8, true);
    push_cubic(&mut pts, (0.5, 0.35), (0.55, 0.15), (0.65, 0.0), (0.75, 0.1), 8, true);
    push_cubic(&mut pts, (0.75, 0.1), (1.0, 0.1), (1.0, 0.55), (0.5, 0.95), 12, true);
    pts
}

fn teardrop() -> Vec<(f32, f32)> {
    let mut pts = vec![(0.5, 0.0)];
    push_cubic(&mut pts, (0.5, 0.0), (0.0, 0.4), (0.0, 1.0), (0.5, 1.0), 16, true);
    push_cubic(&mut pts, (0.5, 1.0), (1.0, 1.0), (1.0, 0.4), (0.5, 0.0), 16, true);
    pts
}

/// A three-quarter pie (the north-east quadrant missing).
fn pie() -> Vec<(f32, f32)> {
    let mut pts = vec![(0.5, 0.5)];
    push_arc(&mut pts, 0.5, 0.5, 0.5, 0.5, 0.0, 3.0 * std::f32::consts::FRAC_PI_2, 36, false);
    pts
}

/// A quarter pie wedge (a slice pointing north-east).
fn pie_wedge() -> Vec<(f32, f32)> {
    let mut pts = vec![(0.5, 0.5)];
    push_arc(&mut pts, 0.5, 0.5, 0.5, 0.5, -std::f32::consts::FRAC_PI_2, 0.0, 12, false);
    pts
}

/// A ring segment: three quarters of a ring (the north-east quadrant open).
fn block_arc() -> Vec<(f32, f32)> {
    let mut pts = Vec::new();
    push_arc(&mut pts, 0.5, 0.5, 0.5, 0.5, 0.0, 3.0 * std::f32::consts::FRAC_PI_2, 36, false);
    push_arc(&mut pts, 0.5, 0.5, 0.25, 0.25, 3.0 * std::f32::consts::FRAC_PI_2, 0.0, 24, true);
    pts
}

/// A ring: the outer circle and the inner circle joined by one bridge segment, traversed
/// out and back so the bridge cancels. One closed path, so the gallery fills a ring.
fn donut() -> Vec<(f32, f32)> {
    let mut outer = ellipse_pts(0.5, 0.5, 0.5, 0.5, 48);
    outer.push((1.0, 0.5));
    let mut inner = ellipse_pts(0.5, 0.5, 0.22, 0.22, 32);
    inner.reverse();
    let first = inner[0];
    inner.push(first);
    outer.extend(inner);
    outer
}

/// The No Smoking bar: a thick diagonal parallelogram across the circle, bottom-left to
/// top-right (the direction the gallery always drew it), `thickness` as a fraction of the
/// frame (the preset's `adj`, default 18750/100000). The half-length keeps every corner
/// inside the circle, so the bar's ends meet the rim without poking past it.
fn no_smoking_bar(thickness: f32) -> Vec<(f32, f32)> {
    let t = (thickness * 0.5).clamp(0.0, 0.5);
    let l = (0.25 - t * t).max(0.0).sqrt().min(0.49);
    let (dx, dy) = (std::f32::consts::FRAC_1_SQRT_2, -std::f32::consts::FRAC_1_SQRT_2);
    let (nx, ny) = (std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2);
    let c = |a: f32, b: f32| (0.5 + a * dx + b * nx, 0.5 + a * dy + b * ny);
    vec![c(l, t), c(l, -t), c(-l, -t), c(-l, t)]
}

/// A diagonal cross (an X with thick arms).
fn cross() -> Vec<(f32, f32)> {
    vec![
        (0.0, 0.2),
        (0.2, 0.0),
        (0.5, 0.3),
        (0.8, 0.0),
        (1.0, 0.2),
        (0.7, 0.5),
        (1.0, 0.8),
        (0.8, 1.0),
        (0.5, 0.7),
        (0.2, 1.0),
        (0.0, 0.8),
        (0.3, 0.5),
    ]
}

/// A plus sign.
fn plus() -> Vec<(f32, f32)> {
    vec![
        (0.35, 0.0),
        (0.65, 0.0),
        (0.65, 0.35),
        (1.0, 0.35),
        (1.0, 0.65),
        (0.65, 0.65),
        (0.65, 1.0),
        (0.35, 1.0),
        (0.35, 0.65),
        (0.0, 0.65),
        (0.0, 0.35),
        (0.35, 0.35),
    ]
}

/// A cloud: a bumpy top over a flat bottom.
fn cloud() -> Vec<(f32, f32)> {
    let mut pts = Vec::new();
    for i in 0..=24 {
        let a = std::f32::consts::PI + std::f32::consts::PI * i as f32 / 24.0;
        let bump = 1.0 + 0.15 * (6.0 * a).sin();
        pts.push((0.5 + 0.42 * a.cos() * bump, 0.55 + 0.32 * a.sin() * bump));
    }
    pts.push((0.98, 0.85));
    pts.push((0.02, 0.85));
    pts
}

/// A sun: a disc with triangular rays.
fn sun() -> Vec<(f32, f32)> {
    (0..24)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / 12.0;
            let r = if i % 2 == 0 { 0.5 } else { 0.36 };
            (0.5 + r * a.cos(), 0.5 + r * a.sin())
        })
        .collect()
}

/// A crescent moon: the outer arc (left side) closed by an inner arc (bulging left).
fn moon() -> Vec<(f32, f32)> {
    let mut pts = Vec::new();
    push_arc(&mut pts, 0.45, 0.5, 0.45, 0.45, 60.0_f32.to_radians(), 300.0_f32.to_radians(), 24, false);
    // Inner arc: from the outer arc's end (top) through the left to the outer arc's start.
    let mut inner = arc_pts(0.65, 0.5, 0.38, 0.38, 274.0_f32.to_radians(), 86.0_f32.to_radians(), 24);
    inner.remove(0);
    pts.extend(inner);
    pts
}

/// A circle segment (an arc closed by its chord).
fn chord() -> Vec<(f32, f32)> {
    arc_pts(0.5, 0.5, 0.5, 0.5, -60.0_f32.to_radians(), 60.0_f32.to_radians(), 16)
}

/// A can (a cylinder): the top ellipse, the sides, the bottom ellipse.
fn can() -> Vec<(f32, f32)> {
    let mut pts = Vec::new();
    push_arc(&mut pts, 0.5, 0.125, 0.5, 0.125, std::f32::consts::PI, 2.0 * std::f32::consts::PI, 12, false);
    pts.push((1.0, 0.875));
    push_arc(&mut pts, 0.5, 0.875, 0.5, 0.125, 0.0, std::f32::consts::PI, 12, true);
    pts.push((0.0, 0.125));
    pts
}

/// A cube: the isometric silhouette (the inner edges are a gallery detail).
fn cube() -> Vec<(f32, f32)> {
    vec![(0.5, 0.0), (1.0, 0.25), (1.0, 0.75), (0.5, 1.0), (0.0, 0.75), (0.0, 0.25)]
}

/// A gear with `teeth` teeth.
fn gear(teeth: usize) -> Vec<(f32, f32)> {
    let step = 2.0 * std::f32::consts::PI / teeth as f32;
    let mut pts = Vec::new();
    for i in 0..teeth {
        let a = -std::f32::consts::FRAC_PI_2 + step * i as f32;
        for (frac, r) in [(0.0, 0.32), (0.15, 0.5), (0.35, 0.5), (0.5, 0.32)] {
            let a = a + step * frac;
            pts.push((0.5 + r * a.cos(), 0.5 + r * a.sin()));
        }
    }
    pts
}

/// A ribbon with elliptical ends: both ends elliptical, or only the right one (the left
/// end then has the swallow-tail notch).
fn ellipse_ribbon(one_end: bool) -> Vec<(f32, f32)> {
    let mut pts = Vec::new();
    if one_end {
        pts.extend([(0.0, 0.0), (0.85, 0.0)]);
    } else {
        pts.extend([(0.15, 0.0), (0.85, 0.0)]);
    }
    push_arc(&mut pts, 0.85, 0.5, 0.15, 0.5, -std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_2, 12, true);
    if one_end {
        pts.extend([(0.0, 1.0), (0.0, 0.7), (0.15, 0.5), (0.0, 0.3)]);
    } else {
        pts.extend([(0.15, 1.0)]);
        push_arc(&mut pts, 0.15, 0.5, 0.15, 0.5, std::f32::consts::FRAC_PI_2, 3.0 * std::f32::consts::FRAC_PI_2, 12, true);
    }
    pts
}

/// A scroll: a sheet with the top and bottom (or left and right) edges bowed out.
fn scroll(vertical: bool) -> Vec<(f32, f32)> {
    let mut pts = Vec::new();
    push_arc(&mut pts, 0.5, 0.1, 0.5, 0.1, std::f32::consts::PI, 2.0 * std::f32::consts::PI, 12, false);
    pts.push((1.0, 0.9));
    push_arc(&mut pts, 0.5, 0.9, 0.5, 0.1, 0.0, std::f32::consts::PI, 12, true);
    pts.push((0.0, 0.1));
    if vertical { pts } else { pts.into_iter().map(|(x, y)| (y, x)).collect() }
}

/// A wavy band with `periods` sine periods.
fn wave(periods: usize) -> Vec<(f32, f32)> {
    let mut pts = Vec::new();
    for i in 0..=24 {
        let x = i as f32 / 24.0;
        pts.push((x, 0.35 + 0.15 * (2.0 * std::f32::consts::PI * periods as f32 * x).sin()));
    }
    for i in (0..=24).rev() {
        let x = i as f32 / 24.0;
        pts.push((x, 0.65 + 0.15 * (2.0 * std::f32::consts::PI * periods as f32 * x).sin()));
    }
    pts
}

/// A rectangle with square tabs along the top edge.
fn square_tabs() -> Vec<(f32, f32)> {
    vec![
        (0.05, 0.0),
        (0.25, 0.0),
        (0.25, 0.25),
        (0.4, 0.25),
        (0.4, 0.0),
        (0.6, 0.0),
        (0.6, 0.25),
        (0.75, 0.25),
        (0.75, 0.0),
        (0.95, 0.0),
        (0.95, 0.25),
        (1.0, 0.25),
        (1.0, 1.0),
        (0.0, 1.0),
        (0.0, 0.25),
    ]
}

/// A brace (`{` when `left`, `}` otherwise) fitted into `[x0, x1]`.
fn brace_at(left: bool, x0: f32, x1: f32) -> Vec<(f32, f32)> {
    let base: [(f32, f32); 7] = if left {
        [(1.0, 0.0), (0.5, 0.0), (0.5, 0.4), (0.0, 0.5), (0.5, 0.6), (0.5, 1.0), (1.0, 1.0)]
    } else {
        [(0.0, 0.0), (0.5, 0.0), (0.5, 0.4), (1.0, 0.5), (0.5, 0.6), (0.5, 1.0), (0.0, 1.0)]
    };
    base.iter().map(|(x, y)| (x0 + x * (x1 - x0), *y)).collect()
}

/// A right arrow: shaft height `shaft`, head length `head_len`, head height `head_h`.
fn arrow_right(shaft: f32, head_len: f32, head_h: f32) -> Vec<(f32, f32)> {
    vec![
        (0.0, 0.5 - shaft / 2.0),
        (1.0 - head_len, 0.5 - shaft / 2.0),
        (1.0 - head_len, 0.5 - head_h / 2.0),
        (1.0, 0.5),
        (1.0 - head_len, 0.5 + head_h / 2.0),
        (1.0 - head_len, 0.5 + shaft / 2.0),
        (0.0, 0.5 + shaft / 2.0),
    ]
}

fn mirror_x(pts: Vec<(f32, f32)>) -> Vec<(f32, f32)> {
    pts.into_iter().map(|(x, y)| (1.0 - x, y)).collect()
}

/// Rotate 90° counter-clockwise (a right arrow becomes an up arrow).
fn rot_ccw(pts: Vec<(f32, f32)>) -> Vec<(f32, f32)> {
    pts.into_iter().map(|(x, y)| (y, 1.0 - x)).collect()
}

/// Rotate 90° clockwise (a right arrow becomes a down arrow).
fn rot_cw(pts: Vec<(f32, f32)>) -> Vec<(f32, f32)> {
    pts.into_iter().map(|(x, y)| (1.0 - y, x)).collect()
}

fn left_right_arrow() -> Vec<(f32, f32)> {
    vec![(0.0, 0.5), (0.35, 0.15), (0.35, 0.35), (0.65, 0.35), (0.65, 0.15), (1.0, 0.5), (0.65, 0.85), (0.65, 0.65), (0.35, 0.65), (0.35, 0.85)]
}

fn quad_arrow() -> Vec<(f32, f32)> {
    vec![(0.5, 0.0), (0.65, 0.35), (1.0, 0.5), (0.65, 0.65), (0.5, 1.0), (0.35, 0.65), (0.0, 0.5), (0.35, 0.35)]
}

/// One diagonal arrow of a corner-arrows pair: the north-east one.
fn corner_arrows_ne() -> Vec<(f32, f32)> {
    vec![(0.9, 0.1), (0.829, 0.383), (0.574, 0.482), (0.518, 0.426), (0.617, 0.171)]
}

/// The south-west one.
fn corner_arrows_sw() -> Vec<(f32, f32)> {
    vec![(0.1, 0.9), (0.383, 0.829), (0.482, 0.574), (0.426, 0.518), (0.171, 0.617)]
}

/// A right arrow split into two heads (a fork).
fn split_arrow() -> Vec<(f32, f32)> {
    vec![(0.0, 0.4), (0.5, 0.4), (0.5, 0.1), (1.0, 0.25), (0.5, 0.45), (0.5, 0.55), (1.0, 0.75), (0.5, 0.9), (0.5, 0.6), (0.0, 0.6)]
}

/// A U-turn arrow: right, down, then left with a head.
fn uturn_arrow() -> Vec<(f32, f32)> {
    vec![(0.0, 0.15), (0.85, 0.15), (0.85, 1.0), (0.35, 1.0), (0.05, 0.85), (0.35, 0.7), (0.65, 0.7), (0.65, 0.35), (0.0, 0.35)]
}

/// A circular arrow: a three-quarter ring band with a head at the top. `clockwise` heads
/// right at the top; the other way heads left.
fn circular_arrow(clockwise: bool) -> Vec<(f32, f32)> {
    let mut pts = Vec::new();
    push_arc(&mut pts, 0.5, 0.5, 0.5, 0.5, 0.0, 3.0 * std::f32::consts::FRAC_PI_2, 36, false);
    pts.push((0.75, 0.05));
    push_arc(&mut pts, 0.5, 0.5, 0.32, 0.32, 3.0 * std::f32::consts::FRAC_PI_2, 0.0, 24, true);
    if clockwise { pts } else { mirror_x(pts) }
}

/// A right arrow with a V-notch cut into its tail.
fn notched_arrow() -> Vec<(f32, f32)> {
    vec![(0.2, 0.5), (0.0, 0.35), (0.65, 0.35), (0.65, 0.15), (1.0, 0.5), (0.65, 0.85), (0.65, 0.65), (0.0, 0.65)]
}

/// A bent-up arrow: right, then up with a head.
fn bent_up_arrow() -> Vec<(f32, f32)> {
    vec![(0.0, 0.65), (0.65, 0.65), (0.65, 0.35), (0.5, 0.35), (0.75, 0.0), (1.0, 0.35), (0.85, 0.35), (0.85, 0.85), (0.0, 0.85)]
}

/// A bent arrow: right, then up-right at a diagonal with a head.
fn bent_arrow() -> Vec<(f32, f32)> {
    vec![(0.0, 0.65), (0.5, 0.65), (0.462, 0.672), (0.865, 0.439), (0.95, 0.1), (0.611, 0.185), (0.378, 0.588), (0.5, 0.85), (0.0, 0.85)]
}

/// A swoosh arrow: a quarter-arc band with a head pointing up at its right end.
fn swoosh_arrow() -> Vec<(f32, f32)> {
    let mut pts = Vec::new();
    push_arc(&mut pts, 0.5, 0.5, 0.45, 0.45, std::f32::consts::FRAC_PI_2, 0.0, 16, false);
    pts.push((0.95, 0.15));
    push_arc(&mut pts, 0.5, 0.5, 0.3, 0.3, 0.0, std::f32::consts::FRAC_PI_2, 12, true);
    pts
}

/// A document: a rectangle with a wavy bottom edge, in `[x0, y0, x1, y1]`.
fn document_at(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<(f32, f32)> {
    let mut pts = vec![(x0, y0), (x1, y0), (x1, y0 + 0.94 * (y1 - y0))];
    for i in 1..=16 {
        let x = 1.0 - i as f32 / 16.0;
        pts.push((x0 + x * (x1 - x0), y0 + (0.97 - 0.03 * (2.0 * std::f32::consts::PI * x).cos()) * (y1 - y0)));
    }
    pts
}

/// A punched tape: a rectangle with wavy top and bottom edges.
fn punched_tape() -> Vec<(f32, f32)> {
    let mut pts = Vec::new();
    for i in 0..=16 {
        let x = i as f32 / 16.0;
        pts.push((x, 0.03 + 0.03 * (2.0 * std::f32::consts::PI * x).cos()));
    }
    pts.push((1.0, 0.94));
    for i in (0..=16).rev() {
        let x = i as f32 / 16.0;
        pts.push((x, 0.97 - 0.03 * (2.0 * std::f32::consts::PI * x).cos()));
    }
    pts
}

/// Online storage: a rectangle with the left edge bowed out.
fn online_storage() -> Vec<(f32, f32)> {
    let mut pts = vec![(0.15, 0.0), (1.0, 0.0), (1.0, 1.0), (0.15, 1.0)];
    push_arc(&mut pts, 0.15, 0.5, 0.15, 0.5, std::f32::consts::FRAC_PI_2, 3.0 * std::f32::consts::FRAC_PI_2, 12, true);
    pts
}

/// A rectangle whose right edge bows out by `bulge` (magnetic tape, display, delay).
fn side_bulge(bulge: f32) -> Vec<(f32, f32)> {
    let mut pts = vec![(0.0, 0.0), (1.0 - bulge, 0.0)];
    push_arc(&mut pts, 1.0 - bulge, 0.5, bulge, 0.5, -std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_2, 12, true);
    pts.push((0.0, 1.0));
    pts
}

/// A magnetic drum: a rectangle with both side edges bowed out.
fn drum() -> Vec<(f32, f32)> {
    let mut pts = vec![(0.15, 0.0), (0.85, 0.0)];
    push_arc(&mut pts, 0.85, 0.5, 0.15, 0.5, -std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_2, 12, true);
    pts.push((0.15, 1.0));
    push_arc(&mut pts, 0.15, 0.5, 0.15, 0.5, std::f32::consts::FRAC_PI_2, 3.0 * std::f32::consts::FRAC_PI_2, 12, true);
    pts
}

/// A rounded rectangle callout: the rounded box over the top three quarters, with the
/// wedge hanging from its bottom edge.
fn wedge_round_rect(r: (f32, f32)) -> Vec<(f32, f32)> {
    let mut pts = vec![(r.0, 0.0), (1.0 - r.0, 0.0)];
    push_arc(&mut pts, 1.0 - r.0, r.1, r.0, r.1, -std::f32::consts::FRAC_PI_2, 0.0, 6, true);
    pts.push((1.0, 0.75));
    pts.push((0.55, 0.75));
    pts.push((0.45, 1.0));
    pts.push((0.25, 0.75));
    pts.push((0.0, 0.75));
    pts.push((0.0, r.1));
    push_arc(&mut pts, r.0, r.1, r.0, r.1, std::f32::consts::PI, 3.0 * std::f32::consts::FRAC_PI_2, 6, true);
    pts
}

/// An oval callout: the ellipse over the top four fifths, with the wedge below it.
fn wedge_ellipse() -> Vec<(f32, f32)> {
    let mut pts = arc_pts(0.5, 0.4, 0.5, 0.4, 80.0_f32.to_radians(), -240.0_f32.to_radians(), 40);
    pts.push((0.45, 1.0));
    pts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::para::ShapeKind;

    fn area(pts: &[(f32, f32)]) -> f32 {
        pts.iter()
            .enumerate()
            .map(|(i, (x, y))| {
                let (x2, y2) = pts[(i + 1) % pts.len()];
                x * y2 - x2 * y
            })
            .sum::<f32>()
            .abs()
            / 2.0
    }

    /// NoSmoking is a circle PLUS a thick diagonal bar: the bar is a closed sub-path after
    /// the ellipse, a parallelogram whose corners all sit inside the circle.
    #[test]
    fn no_smoking_geometry_contains_the_bar() {
        let paths = shape_geom(ShapeKind::NoSmoking, 100.0, 100.0, default_adj(ShapeKind::NoSmoking)).unwrap();
        assert_eq!(paths.len(), 2, "circle + bar");
        assert!(paths.iter().all(|p| p.closed), "both parts are filled areas");
        assert_eq!(paths[0].pts.len(), 48, "the circle");
        let bar = &paths[1].pts;
        assert_eq!(bar.len(), 4, "the bar is a parallelogram");
        for (x, y) in bar {
            let d = ((x - 0.5).powi(2) + (y - 0.5).powi(2)).sqrt();
            assert!(d <= 0.5, "bar corner ({x}, {y}) sits inside the circle, d={d}");
        }
        // A parallelogram's diagonals share a midpoint.
        let mid = |a: (f32, f32), b: (f32, f32)| ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
        let (m1, m2) = (mid(bar[0], bar[2]), mid(bar[1], bar[3]));
        assert!((m1.0 - m2.0).abs() < 1e-6 && (m1.1 - m2.1).abs() < 1e-6, "diagonals share a midpoint");
        assert!(area(bar) > 0.05, "the bar has real area, {}", area(bar));
    }

    /// The bar's thickness follows `adj`: a bigger adjustment is a thicker bar.
    #[test]
    fn no_smoking_bar_thickness_follows_adj() {
        let at = |adj: f32| {
            let paths = shape_geom(ShapeKind::NoSmoking, 100.0, 100.0, adj).unwrap();
            area(&paths[1].pts)
        };
        assert!(at(0.3) > at(0.1), "0.3 bar is thicker than 0.1 bar");
        assert!(at(0.0) < 1e-6, "adj 0 is no bar at all");
    }

    /// The preset default for NoSmoking's bar is 18750/100000, not the corner-radius 0.16:
    /// a stored `None` adjustment and a non-finite one both resolve to it.
    #[test]
    fn no_smoking_default_adj_is_the_preset_default() {
        assert_eq!(default_adj(ShapeKind::NoSmoking), 0.1875);
        assert_eq!(default_adj(ShapeKind::Rectangle), DEFAULT_ADJ);
        let nan = shape_geom(ShapeKind::NoSmoking, 100.0, 100.0, f32::NAN).unwrap();
        let preset = shape_geom(ShapeKind::NoSmoking, 100.0, 100.0, 0.1875).unwrap();
        assert_eq!(nan[1].pts, preset[1].pts, "NaN adj falls back to the preset default");
    }

    /// The audit's other moved interiors: real geometric parts that used to be gallery-only
    /// detail lines are now canonical sub-paths, so canvas/PDF/DOCX render them.
    #[test]
    fn moved_interiors_are_canonical_geometry() {
        let geom = |kind| shape_geom(kind, 100.0, 100.0, DEFAULT_ADJ).unwrap();
        // FoldedCorner: the fold edge.
        let folded = geom(ShapeKind::FoldedCorner);
        assert_eq!(folded.len(), 2);
        assert!(!folded[1].closed && folded[1].pts == vec![(0.72, 0.0), (0.72, 0.28), (1.0, 0.28)]);
        // Cube: the three inner edges meeting at the centre.
        let cube = geom(ShapeKind::Cube);
        assert_eq!(cube.len(), 4);
        assert!(cube[1..].iter().all(|p| !p.closed && p.pts.len() == 2 && p.pts[0] == (0.5, 0.5)));
        // Frame: the inner rectangle, as an open loop.
        let frame = geom(ShapeKind::Frame);
        assert_eq!(frame.len(), 2);
        assert!(!frame[1].closed && frame[1].pts.first() == frame[1].pts.last());
        // HalfFrame: the inner U.
        let half = geom(ShapeKind::HalfFrame);
        assert_eq!(half.len(), 2);
        assert!(!half[1].closed && half[1].pts.len() == 4);
        // StripedRightArrow: the three stripes across the shaft.
        let striped = geom(ShapeKind::StripedRightArrow);
        assert_eq!(striped.len(), 4);
        assert!(striped[1..].iter().all(|p| !p.closed && p.pts.len() == 2));
    }

    /// The smiley's face and the action buttons' glyphs are canonical geometry, so a
    /// drawn shape shows its pattern — not just its outline.
    #[test]
    fn smiley_and_action_buttons_have_glyphs() {
        let geom = |kind| shape_geom(kind, 100.0, 100.0, DEFAULT_ADJ).unwrap();
        // SmileyFace: outline + 2 eyes + curved smile.
        let smiley = geom(ShapeKind::SmileyFace);
        assert_eq!(smiley.len(), 4, "smiley: outline + eyes + smile");
        assert!(smiley[0].closed);
        assert!(smiley[1..].iter().all(|p| !p.closed));
        // The smile is a curve (more than 2 points), not a straight line.
        assert!(smiley[3].pts.len() > 2, "the smile is a curved arc");
        // ActionButtonHome: outline + roof + door.
        let home = geom(ShapeKind::ActionButtonHome);
        assert_eq!(home.len(), 3, "home button: outline + roof + door");
        assert!(home[1..].iter().all(|p| !p.closed));
        // Every action button with a glyph has open sub-paths past its outline.
        for kind in [
            ShapeKind::ActionButtonHelp,
            ShapeKind::ActionButtonInformation,
            ShapeKind::ActionButtonForwardNext,
            ShapeKind::ActionButtonBackPrevious,
            ShapeKind::ActionButtonEnd,
            ShapeKind::ActionButtonBeginning,
            ShapeKind::ActionButtonReturn,
            ShapeKind::ActionButtonDocument,
            ShapeKind::ActionButtonSound,
            ShapeKind::ActionButtonMovie,
        ] {
            let g = geom(kind);
            assert!(g.len() > 1 && g[1..].iter().all(|p| !p.closed), "{kind:?} has glyph sub-paths");
        }
    }
}

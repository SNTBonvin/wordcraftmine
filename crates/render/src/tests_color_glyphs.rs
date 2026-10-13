//! Colour glyphs (#1269): emoji from colour fonts (bitmap strikes, COLR) are drawn in colour.

use super::*;
use wordcraft_layout::{LayoutCache, LayoutOptions, layout};

/// Colour emoji families the system fallback may pick.
const COLOR_EMOJI: &[&str] = &["Noto Color Emoji", "Apple Color Emoji", "Segoe UI Emoji"];

/// A strongly coloured (non-grey) pixel.
fn colourful(p: [u8; 4]) -> bool {
    let (hi, lo) = (p[0].max(p[1]).max(p[2]), p[0].min(p[1]).min(p[2]));
    hi - lo > 60
}

/// System-dependent: skips (passes) when no colour emoji font is installed. Each emoji glyph's box
/// on the rendered page holds coloured pixels.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn emoji_are_drawn_in_colour() {
    let has_colour_font = FontDb::global().fallback_for('🧠', u32::MAX).is_some_and(|f| COLOR_EMOJI.contains(&f.family.as_str()));
    if !has_colour_font {
        eprintln!("skipped: no colour emoji font installed");
        return;
    }
    let d = Document::from_text("Emoji test: 🧠 brain, ✅ check.");
    let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    let page = &l.pages[0];
    let scale = 2.0;
    let img = render_page(&d, page, scale, &RenderOptions::default());
    let mut boxes = vec![];
    for it in page_display(&d, page, &DisplayOptions::default()) {
        if let Draw::Glyphs { face, size, glyphs, .. } = it
            && COLOR_EMOJI.contains(&face.family.as_str())
        {
            for (_, x, y) in glyphs {
                boxes.push((x, y - size, x + size, y));
            }
        }
    }
    assert_eq!(boxes.len(), 2, "both emoji use the colour emoji font");
    for (x0, y0, x1, y1) in boxes {
        let (x0, y0, x1, y1) = ((x0 * scale) as u32, (y0 * scale) as u32, (x1 * scale) as u32, (y1 * scale) as u32);
        let n = (x0..x1).flat_map(|x| (y0..y1).map(move |y| (x, y))).filter(|(x, y)| colourful(img.pixel(*x, *y))).count();
        assert!(n > 20, "only {n} coloured pixels in emoji box {x0},{y0}..{x1},{y1}");
    }
}

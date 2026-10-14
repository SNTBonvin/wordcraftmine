//! Branded builds: a product built on WordCraft under its own name, accent colour, icon set and
//! ribbon tab, chosen at build time (`WORDCRAFT_BRAND_DIR` = a folder with `brand.toml`, read by
//! `build.rs`). Without it every value here is WordCraft's own and nothing changes. Widgets ask
//! this module instead of hard-coding the product: one layer to keep or propose upstream.

use egui::Color32;
use serde_json::Value;
use wordcraft_engine::{CmdError, CmdResult, CommandSpec, Session};

include!(concat!(env!("OUT_DIR"), "/brand_gen.rs"));

/// The egui font family holding the brand's icon font.
pub const ICON_FAMILY: &str = "brand-icons";

/// The brand's glyph for a WordCraft icon name, when its icon set has one.
pub fn glyph(icon: &str) -> Option<char> {
    ICON_GLYPHS.iter().find(|(n, _)| *n == icon).map(|(_, c)| *c).or_else(|| {
        // The brand tab's commands show the icon named by their id's last part.
        TAB?.1.iter().flat_map(|(_, c)| c.iter()).find(|(id, _, _)| id.rsplit('.').next() == Some(icon)).map(|(_, _, g)| *g).filter(|g| *g != '\u{0}')
    })
}

/// A built-in ribbon tab the brand hides (its code stays).
pub fn tab_hidden(name: &str) -> bool {
    HIDE_TABS.contains(&name)
}

/// A label shown under the brand's wording (an item named after another product), before it is
/// translated; labels the brand doesn't list stay as they are.
pub fn relabel(label: &str) -> std::borrow::Cow<'_, str> {
    text(label).map_or(std::borrow::Cow::Borrowed(label), std::borrow::Cow::Borrowed)
}

/// The brand's replacement for a UI string (`[relabel]`, exact match), used by `i18n::t`.
pub fn text(s: &str) -> Option<&'static str> {
    RELABEL.iter().find(|(from, _)| *from == s).map(|(_, to)| *to)
}

/// The accent, its text/hover shade and the colour drawn on it, for the light or dark theme.
pub fn accent(dark: bool) -> Option<(Color32, Color32, Color32)> {
    let ([lr, lg, lb], [dr, dg, db]) = ACCENT?;
    let (r, g, b) = if dark { (dr, dg, db) } else { (lr, lg, lb) };
    let base = Color32::from_rgb(r, g, b);
    // Hover/text shade: darker on light, lighter on dark.
    let shade = |v: u8| if dark { v.saturating_add(((255 - v as u16) / 5) as u8) } else { (v as u16 * 4 / 5) as u8 };
    let ink = Color32::from_rgb(shade(r), shade(g), shade(b));
    // Text on the accent: whichever of white and near-black has the higher contrast.
    let dark_ink = Color32::from_rgb(0x11, 0x16, 0x2E);
    let on = if contrast(dark_ink, base) > contrast(Color32::WHITE, base) { dark_ink } else { Color32::WHITE };
    Some((base, ink, on))
}

/// WCAG 2 contrast ratio of two colours (1–21).
pub fn contrast(a: Color32, b: Color32) -> f32 {
    let lum = |c: Color32| {
        let ch = |v: u8| {
            let v = f32::from(v) / 255.0;
            if v <= 0.039_28 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
    };
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

fn not_yet(_: &Session) -> Option<&'static str> {
    Some("Not available in this version yet")
}

fn unavailable(_: &mut Session, _: &Value) -> CmdResult {
    Err(CmdError::Disabled("not available in this version yet".into()))
}

/// The brand tab's commands (shown, not yet available), for the registry.
pub fn commands() -> Vec<CommandSpec> {
    let Some((tab, groups)) = TAB else { return Vec::new() };
    groups
        .iter()
        .flat_map(|(_, cmds)| cmds.iter())
        .map(|(id, label, _)| {
            let mut c = CommandSpec::new(id, label, tab, unavailable);
            c.enabled = not_yet;
            c.mutates = false;
            c
        })
        .collect()
}

/// The glyph of a brand tab command.
pub fn command_glyph(id: &str) -> Option<char> {
    TAB?.1.iter().flat_map(|(_, c)| c.iter()).find(|(i, _, _)| *i == id).map(|(_, _, g)| *g).filter(|g| *g != '\u{0}')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Without WORDCRAFT_BRAND_DIR every hook falls through to WordCraft's own behaviour.
    #[test]
    fn unbranded_build_is_wordcraft() {
        if BRANDED {
            return;
        }
        assert_eq!(NAME, "WordCraft");
        assert!(accent(false).is_none() && accent(true).is_none());
        assert!(text("About WordCraft").is_none());
        assert!(glyph("bold").is_none());
        assert!(!tab_hidden("Developer") && TAB.is_none() && commands().is_empty());
        let hidden = [HIDE_DISCORD, HIDE_SHARE, HIDE_AUTOSAVE, HIDE_ARTCRAFT];
        assert!(hidden.iter().all(|h| !h), "{hidden:?}");
    }

    /// A brand's accent must stay readable in both themes: text on the accent and accent-coloured
    /// text on the panels at least 4.5:1, the accent against the ribbon at least 3:1 (WCAG AA).
    #[test]
    fn accent_meets_contrast_in_both_themes() {
        for dark in [false, true] {
            let Some((base, ink, on)) = accent(dark) else { return };
            let t = if dark { crate::theme::Tokens::dark() } else { crate::theme::Tokens::light() };
            assert!(contrast(on, base) >= 4.5, "text on the accent (dark {dark}): {}", contrast(on, base));
            for bg in [t.panel, t.ribbon, t.tab_strip] {
                assert!(contrast(ink, bg) >= 4.5, "accent text on {bg:?} (dark {dark}): {}", contrast(ink, bg));
                assert!(contrast(base, bg) >= 3.0, "accent on {bg:?} (dark {dark}): {}", contrast(base, bg));
            }
        }
    }
}

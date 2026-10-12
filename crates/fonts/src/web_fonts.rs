//! Browser font selection shared with the optional craft-fonts build input.
//!
//! A browser cannot use installed system fonts. Document substitutes must survive the
//! web-size filter or fixed-height text boxes can lose lines and anchored objects drift.
pub fn embed_for_web(family: &str, style: &str) -> bool {
    (family == "BIZ UDPGothic" && style == "Regular")
        || matches!(family, "Arimo" | "Liberation Sans" | "Carlito" | "Caladea" | "Tinos" | "Liberation Serif" | "Cousine" | "Liberation Mono")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_keeps_all_styles_of_metric_compatible_document_fonts() {
        for family in ["Arimo", "Liberation Sans", "Carlito", "Caladea", "Tinos", "Liberation Serif", "Cousine", "Liberation Mono"] {
            for style in ["Regular", "Bold", "Italic", "Bold Italic"] {
                assert!(embed_for_web(family, style), "missing {family} {style}");
            }
        }
        assert!(embed_for_web("BIZ UDPGothic", "Regular"));
        assert!(!embed_for_web("BIZ UDPGothic", "Bold"));
        assert!(!embed_for_web("Unrelated Optional Font", "Regular"));
    }
}

//! Native CommonMark/GFM rendering, without image fetching, HTML execution or local URI links.
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use pulldown_cmark::{Event, Options, Parser, Tag};
use std::borrow::Cow;

fn safe_source(text: &str) -> Cow<'_, str> {
    let mut blocked = Vec::new();
    let mut depth = 0usize;
    for (event, range) in Parser::new_ext(text, Options::all()).into_offset_iter().take(100_000) {
        match event {
            Event::Start(tag) => {
                depth = depth.saturating_add(1);
                if depth > 64 {
                    return Cow::Owned(format!("```text\n{}\n```", super::clip(&text.replace("```", "'''"), 24_000)));
                }
                match tag {
                    Tag::Image { .. } => blocked.push((range, "[image omitted]")),
                    Tag::Link { dest_url, .. }
                        if !dest_url.starts_with("https://")
                            && !dest_url.starts_with("http://")
                            && !dest_url.starts_with("mailto:")
                            && !dest_url.starts_with('#') =>
                    {
                        blocked.push((range, "[unsupported link]"))
                    }
                    _ => {}
                }
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    if blocked.is_empty() {
        return Cow::Borrowed(text);
    }
    blocked.sort_by_key(|(r, _)| r.start);
    let mut out = String::new();
    let mut offset = 0;
    for (range, replacement) in blocked {
        if range.start < offset {
            continue;
        }
        out.push_str(text.get(offset..range.start).unwrap_or(""));
        out.push_str(replacement);
        offset = range.end;
    }
    out.push_str(text.get(offset..).unwrap_or(""));
    Cow::Owned(out)
}
pub fn show(ui: &mut egui::Ui, cache: &mut CommonMarkCache, text: &str) {
    let clipped = super::clip(text, 48_000);
    let source = safe_source(&clipped);
    ui.scope(|ui| {
        ui.style_mut().interaction.selectable_labels = true;
        CommonMarkViewer::new().enable_scroll_to_heading(false).show(ui, cache, &source);
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn standard_markdown_is_preserved() {
        let text = "# Heading\n\n**Bold** *italic* `code`\n\n- item\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n[Source](https://example.com)";
        assert_eq!(safe_source(text), text);
    }
    #[test]
    fn images_and_local_links_cannot_fetch_files() {
        let result = safe_source("![secret](file:///private/key) [open](javascript:alert) [good](https://example.com)");
        assert!(!result.contains("file:///"));
        assert!(!result.contains("javascript:"));
        assert!(result.contains("https://example.com"));
    }
    #[test]
    fn incomplete_streams_and_unicode_render() {
        for text in ["**Café 世界", "```rust\nlet x = 1;", "[link](https://example.com)", "> quote\n\n- [x] task"] {
            assert!(!safe_source(text).is_empty());
        }
    }
}

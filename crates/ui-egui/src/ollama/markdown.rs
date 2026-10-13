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
    for clicked in cache.link_hooks_mut().values_mut() {
        *clicked = false;
    }
    // A child's intrinsic width must never resize the containing right panel. GFM tables
    // intentionally scroll within their own block; prose remains wrapped and selectable.
    let width = ui.available_width().max(1.0);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(egui::Rect::from_min_size(ui.next_widget_position(), egui::vec2(width, ui.available_height())))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(ui.clip_rect());
    child.style_mut().interaction.selectable_labels = true;
    child.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
    let mut blocks = Vec::new();
    let mut table_start = None;
    for (event, range) in Parser::new_ext(&source, Options::all()).into_offset_iter().take(100_000) {
        match event {
            Event::Start(Tag::Table(_)) => table_start = Some(range.start),
            Event::End(pulldown_cmark::TagEnd::Table) => {
                if let Some(start) = table_start.take() {
                    blocks.push(start..range.end);
                }
            }
            _ => {}
        }
    }
    let mut offset = 0;
    for (i, range) in blocks.iter().enumerate() {
        if let Some(prose) = source.get(offset..range.start) {
            viewer(&mut child, cache, prose);
        }
        egui::ScrollArea::horizontal()
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
            .id_salt(("markdown_table", i))
            .auto_shrink([false, true])
            .max_width(width)
            .show(&mut child, |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                if let Some(table) = source.get(range.clone()) {
                    viewer(ui, cache, table);
                }
            });
        offset = range.end;
    }
    if let Some(prose) = source.get(offset..) {
        viewer(&mut child, cache, prose);
    }
    ui.allocate_space(egui::vec2(width, child.min_rect().height()));
}
fn viewer(ui: &mut egui::Ui, cache: &mut CommonMarkCache, source: &str) {
    let clicked: Vec<String> = cache.link_hooks().iter().filter(|(_, v)| **v).map(|(k, _)| k.clone()).collect();
    CommonMarkViewer::new().enable_scroll_to_heading(false).show(ui, cache, source);
    for url in clicked {
        if let Some(value) = cache.link_hooks_mut().get_mut(&url) {
            *value = true;
        }
    }
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

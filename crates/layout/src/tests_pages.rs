//! Page geometry: mirrored margins, headers and footers taller than their margin (also when
//! linked to the previous section), endnotes continuing over pages, and the caret in a footnote
//! continued on the next page.
use super::*;
use wordcraft_doc::para::{InlineObject, NoteKind};
use wordcraft_doc::{PartKind, Pos, para_block};

fn lay(doc: &Document) -> DocLayout {
    let mut c = LayoutCache::new();
    layout(doc, &mut c, &LayoutOptions::default())
}

fn paras(text: &str, n: usize) -> Vec<Arc<Block>> {
    (1..=n).map(|i| para_block(Paragraph::with_text(&format!("{text} {i}"), Default::default()))).collect()
}

/// Top and bottom of the lines of `story` among `items`.
fn extent(items: &[Placed], story: StoryRef) -> Option<(f32, f32)> {
    items
        .iter()
        .filter_map(|it| match it {
            Placed::Lines { story: s, y, para, l0, l1, .. } if *s == story => Some((*y, item_bottom(*y, para, *l0, *l1)?)),
            _ => None,
        })
        .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
}

/// The left edge of the body text on each page.
fn body_left(l: &DocLayout) -> Vec<f32> {
    l.pages
        .iter()
        .map(|p| p.items.iter().find_map(|it| if let Placed::Lines { story: StoryRef::Body, x, .. } = it { Some(*x) } else { None }).unwrap())
        .collect()
}

#[test]
fn mirrored_margins_put_the_inside_margin_toward_the_binding() {
    // #452: inside 108 (plus a 10pt gutter), outside 36. Odd pages: inside on the left; even
    // pages: on the right.
    let mut d = Document::from_text(&"Body line\n".repeat(90));
    d.last_section.margin_left = 108.0;
    d.last_section.margin_right = 36.0;
    d.last_section.gutter = 10.0;
    let plain = body_left(&lay(&d));
    assert!(plain.len() >= 2 && plain.iter().all(|x| (x - 118.0).abs() < 0.01), "not mirrored: {plain:?}");
    d.settings.mirror_margins = true;
    let l = lay(&d);
    let lefts = body_left(&l);
    for (i, (x, p)) in lefts.iter().zip(&l.pages).enumerate() {
        let want = if p.number % 2 == 1 { 118.0 } else { 36.0 };
        assert!((x - want).abs() < 0.01, "page {i} (number {}): text at {x}, want {want}", p.number);
        assert!((p.body.x - want).abs() < 0.01 && (p.body.w - d.last_section.text_width()).abs() < 0.01, "page {i}: {:?}", p.body);
    }
}

#[test]
fn inside_and_outside_floats_swap_sides_on_even_pages() {
    // #1181: shapes Word places in the outside margin of a book (mirrored margins) all came out
    // in the right margin. Inside 108 plus a 10pt gutter, outside 36; the paragraph on page 2
    // carries the same six shapes as page 1's, told apart by their widths.
    use super::tests::{rect_shape, shapes};
    use wordcraft_doc::para::{Anchor, Float, FloatAlign, Wrap};
    let mut d = Document::from_text("Odd page\nEven page");
    d.last_section.margin_left = 108.0;
    d.last_section.margin_right = 36.0;
    d.last_section.gutter = 10.0;
    d.settings.mirror_margins = true;
    if let Some(Block::Para(p)) = d.body.get_mut(1).map(Arc::make_mut) {
        p.props.page_break_before = Some(true);
    }
    let floats = [
        (Anchor::Margin, Some(FloatAlign::Outside), 0.0),
        (Anchor::OutsideMargin, Some(FloatAlign::Center), 0.0),
        (Anchor::InsideMargin, Some(FloatAlign::Center), 0.0),
        (Anchor::Page, Some(FloatAlign::Inside), 0.0),
        (Anchor::Page, None, 10.0),
        (Anchor::LeftMargin, Some(FloatAlign::End), 0.0),
    ];
    for block in 0..2 {
        for (i, (h_rel, h_align, x)) in floats.iter().enumerate() {
            let f = Float { wrap: Wrap::InFrontOfText, h_rel: *h_rel, h_align: *h_align, x: *x, ..Default::default() };
            d.insert_object(&Pos::body(block, 0), rect_shape(20.0 + 2.0 * i as f32, 10.0, f), &Default::default()).unwrap();
        }
    }
    // Left edge of shape `i` on an odd and on an even page (page coordinates).
    let want = |i: usize, even: bool| {
        let w = 20.0 + 2.0 * i as f32;
        match (i, even) {
            (0, false) => 576.0 - w,
            (0, true) => 36.0,
            (1, false) => 576.0 + (36.0 - w) / 2.0,
            (1, true) => (36.0 - w) / 2.0,
            (2, false) => (118.0 - w) / 2.0,
            (2, true) => 494.0 + (118.0 - w) / 2.0,
            (3, false) => 0.0,
            (3, true) => 612.0 - w,
            (4, _) => 10.0,
            (5, false) => 118.0 - w,
            _ => 36.0 - w,
        }
    };
    let l = lay(&d);
    assert_eq!(l.pages.len(), 2);
    for (pi, page) in l.pages.iter().enumerate() {
        let r = shapes(&page.items);
        assert_eq!(r.len(), floats.len(), "page {pi}: {r:?}");
        for (i, f) in floats.iter().enumerate() {
            let w = 20.0 + 2.0 * i as f32;
            let got = r.iter().find(|r| (r.w - w).abs() < 0.01).unwrap();
            let x = want(i, page.number % 2 == 0);
            assert!((got.x - x).abs() < 0.01, "page {} shape {i} ({:?}): at {}, want {x}", page.number, f, got.x);
        }
    }
    // Without mirrored margins the margins stay put, but inside and outside still follow the
    // page (ECMA-376 §20.4.3.4): the inside of an even page is its right side.
    d.settings.mirror_margins = false;
    let l = lay(&d);
    let r = shapes(&l.pages[1].items);
    let at = |w: f32| r.iter().find(|r| (r.w - w).abs() < 0.01).unwrap().x;
    assert!((at(20.0) - 118.0).abs() < 0.01, "outside of the margins on page 2: {}", at(20.0));
    assert!((at(26.0) - (612.0 - 26.0)).abs() < 0.01, "inside of page 2: {}", at(26.0));
    assert!((at(28.0) - 10.0).abs() < 0.01, "{}", at(28.0));
}

#[test]
fn outside_floating_tables_swap_sides_on_even_pages() {
    // #1181, for a floating table (`w:tblpXSpec="outside"`): on page 2 of a book the outside is
    // the left. One positioned from the page edge stays there on both pages.
    use wordcraft_doc::para::{Anchor, FloatAlign};
    use wordcraft_doc::props::TableFloat;
    let table_x = |page_two: bool, float: TableFloat| {
        let mut d = Document::from_text("First\nSecond\nAfter");
        d.last_section.margin_left = 108.0;
        d.last_section.margin_right = 36.0;
        d.last_section.gutter = 10.0;
        d.settings.mirror_margins = true;
        if let Some(Block::Para(p)) = d.body.get_mut(1).map(Arc::make_mut) {
            p.props.page_break_before = Some(page_two);
        }
        let mut t = wordcraft_doc::Table::new(1, 1, 100.0);
        t.props.float = Some(float);
        d.insert_block(StoryRef::Body, &Path::top(2), Block::Table(t)).unwrap();
        let l = lay(&d);
        let page = &l.pages[page_two as usize];
        let x = page.items.iter().find_map(|it| match it {
            Placed::Lines { story: StoryRef::Body, path, x, .. } if path.0.len() > 1 => Some(*x),
            _ => None,
        });
        x.unwrap()
    };
    let outside = TableFloat { h_rel: Anchor::Margin, v_rel: Anchor::Paragraph, h_align: Some(FloatAlign::Outside), ..Default::default() };
    let (odd, even) = (table_x(false, outside), table_x(true, outside));
    assert!(odd > 576.0 - 100.0 - 0.01 && odd < 576.0, "odd page: at the right margin, text at {odd}");
    assert!(even > 36.0 - 0.01 && even < 36.0 + 20.0, "even page: at the left margin, text at {even}");
    let from_page = TableFloat { h_rel: Anchor::Page, v_rel: Anchor::Paragraph, x: 200.0, ..Default::default() };
    let (odd, even) = (table_x(false, from_page), table_x(true, from_page));
    assert!((odd - even).abs() < 0.01, "same place from the page edge: {odd} and {even}");
}

#[test]
fn a_tall_footer_pushes_the_body_up() {
    // #453: an eight-line footer reaches far above the 72pt bottom margin.
    let mut d = Document::from_text(&"Body line\n".repeat(60));
    let id = d.add_part(PartKind::Footer, paras("Footer line", 8));
    d.last_section.footers.default = Some(id);
    let l = lay(&d);
    assert!(l.pages.len() >= 2);
    let foot_top = extent(&l.pages[0].footer, StoryRef::Part(id)).unwrap().0;
    assert!(foot_top < 792.0 - 72.0, "the footer is taller than the margin: {foot_top}");
    for (i, p) in l.pages.iter().enumerate() {
        let (_, body_bottom) = extent(&p.items, StoryRef::Body).unwrap();
        let (top, _) = extent(&p.footer, StoryRef::Part(id)).unwrap();
        assert!(body_bottom <= top + 0.01, "page {i}: body ends at {body_bottom}, footer starts at {top}");
        assert!(p.body.bottom() <= top + 0.01, "page {i}: {:?}", p.body);
    }
    // Every body paragraph is still there, once.
    let placed: usize = l.pages.iter().flat_map(|p| &p.items).filter(|it| matches!(it, Placed::Lines { story: StoryRef::Body, l0: 0, .. })).count();
    assert_eq!(placed, d.body.len());
    // A one-line footer fits in the margin: the body keeps its full height.
    d.parts.insert(id, wordcraft_doc::Part { kind: PartKind::Footer, blocks: paras("Footer", 1), body: Default::default() });
    let l = lay(&d);
    assert!((l.pages[0].body.bottom() - (792.0 - 72.0)).abs() < 0.01, "{:?}", l.pages[0].body);
}

#[test]
fn a_linked_tall_header_pushes_the_next_sections_body_down() {
    // #454: section 2 has no header of its own and shows section 1's (link to previous).
    let mut d = Document::from_text("First section\nSecond section");
    let id = d.add_part(PartKind::Header, paras("Header line", 8));
    let first = SectionProps { headers: wordcraft_doc::section::HeaderSet { default: Some(id), ..Default::default() }, ..d.last_section.clone() };
    d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().section = Some(Box::new(first));
    let l = lay(&d);
    assert_eq!(l.pages.len(), 2);
    for (i, p) in l.pages.iter().enumerate() {
        assert_eq!(p.header_story, Some(id), "page {i}");
        let (_, header_bottom) = extent(&p.header, StoryRef::Part(id)).unwrap();
        assert!(header_bottom > 72.0, "the header reaches past the top margin");
        let (body_top, _) = extent(&p.items, StoryRef::Body).unwrap();
        assert!(body_top >= header_bottom - 0.01, "page {i}: body at {body_top}, header ends at {header_bottom}");
    }
    assert_eq!(l.pages[0].body.y, l.pages[1].body.y, "same header, same body top");
}

#[test]
fn long_endnotes_continue_on_new_pages() {
    // #456: an 80-paragraph endnote after a one-line body.
    let mut d = Document::from_text("Body");
    let id = d.add_part(PartKind::Endnote, paras("Endnote paragraph", 80));
    d.insert_object(&Pos::body(0, 4), InlineObject::NoteRef { kind: NoteKind::Endnote, id, custom: String::new() }, &Default::default()).unwrap();
    let l = lay(&d);
    assert!(l.pages.len() >= 2, "{} pages", l.pages.len());
    let mut seen = Vec::new();
    for (i, p) in l.pages.iter().enumerate() {
        if let Some((top, bottom)) = extent(&p.items, StoryRef::Part(id)) {
            assert!(top >= p.body.y - 0.01 && bottom <= p.body.bottom() + 0.01, "page {i}: note at {top}..{bottom}, body {:?}", p.body);
        }
        for it in &p.items {
            if let Placed::Lines { story: StoryRef::Part(s), path, l0: 0, .. } = it
                && *s == id
            {
                seen.push(path.0.clone());
            }
        }
    }
    assert_eq!(seen, (0..80u32).map(|i| vec![i]).collect::<Vec<_>>(), "every paragraph once, in order");
    // The caret reaches the last paragraph, on a page.
    let c = l.caret(&Pos { story: StoryRef::Part(id), path: Path::top(79), off: 0 }).unwrap();
    assert!(c.top < 792.0 - 72.0, "{c:?}");
}

#[test]
fn caret_in_a_continued_footnote_is_found_on_its_page() {
    // #457: one long footnote paragraph split over pages 1 and 2.
    let mut d = Document::from_text(&"Body text line.\n".repeat(16));
    let id = d.add_part(
        PartKind::Footnote,
        vec![para_block(Paragraph::with_text(&"A long footnote that runs on and on. ".repeat(100), Default::default()))],
    );
    d.insert_object(&Pos::body(4, 4), InlineObject::NoteRef { kind: NoteKind::Footnote, id, custom: String::new() }, &Default::default()).unwrap();
    let l = lay(&d);
    // The first line of the piece on page 2.
    let (para, k) = l.pages[1]
        .items
        .iter()
        .find_map(|it| match it {
            Placed::Lines { story: StoryRef::Part(s), para, l0, .. } if *s == id && *l0 > 0 => Some((para.clone(), *l0)),
            _ => None,
        })
        .expect("the note continues on page 2");
    let at = |off: usize| Pos { story: StoryRef::Part(id), path: Path::top(0), off };
    let cont = at(para.lines[k + 1].start);
    for hint in [0, 1] {
        assert_eq!(l.caret_on(&at(0), hint).map(|c| c.page), Some(0), "start, hint {hint}");
        let c = l.caret_on(&cont, hint).unwrap_or_else(|| panic!("no caret with hint {hint}"));
        assert_eq!(c.page, 1, "hint {hint}");
        // Up goes to the line above, not to the note's start.
        let (up, _) = l.vertical(&cont, None, -1, hint).unwrap();
        assert_eq!(para.line_of(up.off), k, "hint {hint}: {up:?}");
    }
}

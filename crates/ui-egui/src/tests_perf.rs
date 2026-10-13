//! Scrolling performance with a very large table (#811).

use std::sync::Arc;
use std::time::Instant;

use wordcraft_doc::table::{Cell, Table};
use wordcraft_doc::{Block, Document, PartKind, Path, Pos, StoryRef};

use crate::{Services, WordApp};

fn page_breaks(layout: &wordcraft_layout::DocLayout) -> Vec<(usize, wordcraft_doc::Path)> {
    layout
        .pages
        .iter()
        .enumerate()
        .filter_map(|(page, p)| {
            p.items.iter().find_map(|item| match item {
                wordcraft_layout::Placed::Lines { story: StoryRef::Body, path, .. } => Some((page, path.clone())),
                _ => None,
            })
        })
        .collect()
}

/// A document with one `rows × cols` table, every cell holding a few words.
pub(crate) fn big_table_doc(rows: usize, cols: usize) -> Document {
    let mut d = Document::from_text("Before the table\nAfter the table");
    let mut t = Table::new(rows, cols, 468.0);
    for (r, row) in t.rows.iter_mut().enumerate() {
        for (c, cell) in row.cells.iter_mut().enumerate() {
            let w = cell.props.width;
            *cell = Cell::with_text(&format!("R{r} C{c} value {}", r * cols + c));
            cell.props.width = w;
        }
    }
    let _ = d.insert_block(StoryRef::Body, &Path::top(1), Block::Table(t));
    d
}

fn harness(doc: Document) -> egui_kittest::Harness<'static, WordApp> {
    let a = WordApp::new(wordcraft_engine::Session::new(doc), Services::default());
    egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    )
}

fn wheel(h: &mut egui_kittest::Harness<'static, WordApp>, dy: f32) {
    h.input_mut().events.push(egui::Event::PointerMoved(egui::pos2(720.0, 500.0)));
    h.input_mut().events.push(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, dy),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    });
}

/// `cargo test -p wordcraft-ui-egui --release big_table_scroll_bench -- --ignored --nocapture`
/// (`WORDCRAFT_BENCH_DOCX=path` times that file instead of the synthetic table).
#[test]
#[ignore]
fn big_table_scroll_bench() {
    let doc = match std::env::var("WORDCRAFT_BENCH_DOCX") {
        Ok(p) => wordcraft_engine::io::open_path(std::path::Path::new(&p)).unwrap(),
        Err(_) => big_table_doc(2000, 8),
    };
    let mut s = wordcraft_engine::Session::new(doc.clone());
    let t = Instant::now();
    let l = s.layout();
    eprintln!("layout: {:.1} ms, {} pages", t.elapsed().as_secs_f64() * 1e3, l.pages.len());
    let t = Instant::now();
    let _ = s.export_layout();
    eprintln!("export_layout: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
    if let Some(p) = l.pages.get(1) {
        let t = Instant::now();
        let _ = wordcraft_render::render_page(&s.doc, p, 1.5, &crate::canvas::screen_render_options());
        eprintln!("render_page: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
    }
    let mut h = harness(doc);
    let t = Instant::now();
    h.run_steps(3);
    eprintln!("first frames: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
    for _ in 0..10 {
        h.step();
    }
    let mut idle = Vec::new();
    for _ in 0..10 {
        let t = Instant::now();
        h.step();
        idle.push(t.elapsed().as_secs_f64() * 1e3);
    }
    let mut steps = Vec::new();
    for _ in 0..40 {
        wheel(&mut h, -120.0);
        let t = Instant::now();
        h.step();
        steps.push(t.elapsed().as_secs_f64() * 1e3);
    }
    let stat = |v: &mut Vec<f64>| {
        v.sort_by(|a, b| a.total_cmp(b));
        (v.get(v.len() / 2).copied().unwrap_or(0.0), v.last().copied().unwrap_or(0.0))
    };
    eprintln!("idle frame: median/max {:.2?}", stat(&mut idle));
    eprintln!("scroll frame: median/max {:.2?}", stat(&mut steps));
    eprintln!("scroll offset after: {:?}", h.state().canvas.scroll_offset);
}

/// `cargo test -p wordcraft-ui-egui --release large_table_docx_open_bench -- --ignored --nocapture`
///
/// Builds a deterministic DOCX with one 15,000 × 8 table (about 1,000 Letter pages), then
/// measures the same in-memory DOCX decoding used after the file bytes have been read and a full
/// cold layout. Serialization deliberately happens before either timer.
#[test]
#[ignore]
fn large_table_docx_open_bench() {
    let bytes = wordcraft_engine::io::save_bytes("large-table.docx", &big_table_doc(15_000, 8)).unwrap();
    let t = Instant::now();
    let doc = wordcraft_engine::io::open_bytes("large-table.docx", &bytes).unwrap();
    let open = t.elapsed();
    let mut session = wordcraft_engine::Session::new(doc);
    let t = Instant::now();
    let layout = session.layout();
    let full_layout = t.elapsed();
    eprintln!(
        "large table DOCX: {} bytes, open: {:.1} ms, full layout: {:.1} ms, {} pages",
        bytes.len(),
        open.as_secs_f64() * 1e3,
        full_layout.as_secs_f64() * 1e3,
        layout.pages.len()
    );
    assert!(layout.pages.len() >= 1_000, "fixture must stay approximately 1,000 pages");
}

fn mixed_book_doc() -> Document {
    let mut doc = Document::new();
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89\0\0\0\x0dIDAT\x08\xd7c\xf8\xcf\xc0\xf0\x1f\0\x05\0\x01\xff\x89\x99=\x1d\0\0\0\0IEND\xaeB`\x82";
    doc.media.insert("benchmark.png".into(), Arc::new(PNG.to_vec()));
    for i in 0..13_000usize {
        let text = if i.is_multiple_of(50) {
            format!("Chapter {}: a measured heading", i / 50 + 1)
        } else {
            String::from("A measured paragraph keeps realistic prose flowing through a long document.")
        };
        let mut p = wordcraft_doc::Paragraph::with_text(&text, Default::default());
        if i.is_multiple_of(50) {
            p.props.style = Some("Heading1".into());
        }
        doc.body.push(wordcraft_doc::para_block(p));
    }
    for i in (50..13_000usize).step_by(250) {
        let id = doc.add_part(
            PartKind::Footnote,
            vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(
                "A benchmark footnote with enough text to exercise note layout.",
                Default::default(),
            ))],
        );
        doc.insert_object(
            &Pos::body(i + 1, 8),
            wordcraft_doc::InlineObject::NoteRef { kind: wordcraft_doc::para::NoteKind::Footnote, id, custom: String::new() },
            &Default::default(),
        )
        .unwrap();
    }
    for i in (125..13_000usize).step_by(1_000) {
        doc.insert_object(
            &Pos::body(i + 1, 0),
            wordcraft_doc::InlineObject::Image {
                media: "benchmark.png".into(),
                w: 36.0,
                h: 24.0,
                alt: "Benchmark picture".into(),
                float: Default::default(),
                crop: [0.0; 4],
                ole: None,
            },
            &Default::default(),
        )
        .unwrap();
    }
    doc
}

/// `cargo test -p wordcraft-ui-egui --release mixed_book_docx_open_bench -- --ignored --nocapture`
///
/// A deterministic text-heavy DOCX with headings, footnotes and inline pictures. Serialization is
/// outside the timers; this measures decode/open and the first complete pagination separately.
#[test]
#[ignore]
fn mixed_book_docx_open_bench() {
    let bytes = wordcraft_engine::io::save_bytes("mixed-book.docx", &mixed_book_doc()).unwrap();
    let t = Instant::now();
    let doc = wordcraft_engine::io::open_bytes("mixed-book.docx", &bytes).unwrap();
    let open = t.elapsed();
    let mut session = wordcraft_engine::Session::new(doc);
    let t = Instant::now();
    let layout = session.layout();
    let full_layout = t.elapsed();
    eprintln!(
        "mixed book DOCX: {} bytes, open: {:.1} ms, full layout: {:.1} ms, {} pages",
        bytes.len(),
        open.as_secs_f64() * 1e3,
        full_layout.as_secs_f64() * 1e3,
        layout.pages.len()
    );
    assert!(layout.pages.len() >= 500, "fixture must stay at least 500 pages");
}

/// AutoFit measurement is an implementation detail: a warm session and a clean layout cache must
/// produce identical pagination for both the built-in sample and a long table.
#[test]
fn autofit_change_keeps_sample_and_big_table_page_breaks() {
    for doc in [wordcraft_engine::sample::sample_document(), big_table_doc(600, 8)] {
        let mut session = wordcraft_engine::Session::new(doc.clone());
        let cached = session.layout();
        let fresh = wordcraft_layout::layout(&doc, &mut wordcraft_layout::LayoutCache::new(), &wordcraft_layout::LayoutOptions::default());
        assert_eq!(cached.pages.len(), fresh.pages.len());
        assert_eq!(page_breaks(&cached), page_breaks(&fresh));
    }
}

/// #811: scrolling through one very large table was slow because every frame fingerprinted every
/// item of every page (thousands of cell borders a page) for the page texture cache, and every
/// page ever shown kept its texture. Now a frame lays nothing out again, fingerprints a page once
/// per layout and only when it is on screen, and keeps textures only near the view.
#[test]
fn scrolling_a_big_table_fingerprints_only_pages_coming_into_view() {
    let mut h = harness(big_table_doc(600, 8));
    h.run_steps(4);
    let layout = h.state_mut().session.layout();
    let pages = layout.pages.len();
    assert!(pages > 30, "the table runs over many pages ({pages})");
    let first = h.state().canvas.page_keys.hashed;
    assert!(first > 0 && first < 6, "only the pages on screen are fingerprinted, not all {pages}: {first}");
    // Idle frames do no per-page work.
    h.run_steps(3);
    assert_eq!(h.state().canvas.page_keys.hashed, first, "idle frames fingerprint nothing");
    // Scroll a long way down: each newly shown page is fingerprinted once.
    let mut max_textures = 0;
    for _ in 0..60 {
        wheel(&mut h, -400.0);
        h.step();
        max_textures = max_textures.max(h.state().canvas.texture_count());
    }
    let offset = h.state().canvas.scroll_offset.y;
    assert!(offset > 20_000.0, "scrolled through many pages: {offset}");
    let after = std::sync::Arc::clone(&h.state_mut().session.layout());
    assert!(std::sync::Arc::ptr_eq(&layout, &after), "scrolling never lays the document out again");
    let hashed = h.state().canvas.page_keys.hashed;
    // 60 steps of 400 pt cross about 24'000 / 1'056 ≈ 23 pages (at 100%, 1.33 px a point).
    assert!(hashed < pages && hashed <= first + 30, "pages fingerprinted: {hashed} of {pages}");
    // Textures stay bounded to the pages near the view.
    assert!(max_textures <= 12, "textures kept while scrolling: {max_textures}");
    assert!(h.state().canvas.texture_count() >= 1, "the pages in view stay rendered");
}

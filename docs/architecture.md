# WordCraft architecture

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-13 · **Change:** minor (shared RTL UI support and regression coverage; previously minor: `proof` is per language: English and Polish; previously major: first committed version; `plan/architecture.md` is local-only) · **Target:** Microsoft Word (Microsoft 365) for Mac 16.113.4

How WordCraft is built today (origin/main `40c5d51`). The longer design notes in
`plan/architecture.md` are gitignored and local to the original author's machine; this file is the
committed, current summary. Rules for working in the code are in [`AGENTS.md`](../AGENTS.md).

## Layers

~85,600 lines of Rust, 15 crates, 3 apps and `xtask`. Layering is enforced by `cargo xtask layers`:
a crate may only depend on crates in lower layers, and nothing below L6 depends on egui, eframe,
winit or rfd.

| Layer | Crate | Lines | Job |
|---|---|---|---|
| L0 | `geom` | 0.2k | Units (points, twips, EMU, inches, cm), rectangles, measurement parsing |
| L1 | `doc` | 10.0k | Document model: stories, paragraphs, runs, tables, sections, styles, numbering, fields, notes, comments, revisions, equations (OMML-shaped math tree, linear/LaTeX input); edit primitives; bidi |
| L1 | `fonts` | 2.3k | Font database (bundled OFL + system fonts), metrics, outlines, shaping with HarfRust, Word font substitution (`word.rs`: Carlito ↔ Calibri, Caladea ↔ Cambria, Liberation ↔ Arial/Times/Courier), OpenType MATH |
| L1 | `proof` | 3.8k | Proofing per language (`ProofLang` from a run's `w:lang`): English (public-domain word list) and Polish (installed Hunspell dictionary through a Hunspell-style affix engine) spelling and suggestions, rule-based grammar for both, hyphenation dictionary and Liang patterns (English, Polish) |
| L2 | `layout` | 9.6k | Shaping runs, line breaking, tabs, lists, tables (row splitting, floating tables), pagination, headers/footers, notes, floating objects and wrap, math layout, fields, hit testing; produces a display list |
| L2 | `docx` | 8.4k | OOXML WordprocessingML reader and writer (Transitional and Strict namespaces, `.docx/.docm/.dotx/.dotm`), OMML, custom properties, VBA parts preserved |
| L2 | `docbin` | 4.4k | Word 97-2003 `.doc` reader ([MS-DOC]: FIB, piece table, FKPs, SPRMs, stylesheet, lists, tables, notes, pictures) over a CFB container |
| L2 | `formats` | 10.6k | TXT, Markdown, HTML, RTF, ODT and LaTeX import/export through a shared intermediate model |
| L3 | `render` | 0.6k | vello_cpu rasteriser: pages and thumbnails from the display list; PNG/JPEG |
| L3 | `pdf` | 1.6k | krilla PDF export: font subsets, selectable text, links, outline, tagging |
| L4 | `engine` | 15.2k | `Session` (document, selection, undo, layout cache, view state), **command registry** (422 `CommandSpec`s in `cmd/*.rs`), Word feature catalog (`catalog.rs`), samples, file I/O dispatch (`io.rs`), Read Aloud sentence model |
| L4 | `zotero` | 1.7k | Zotero's word-processor integration protocol answered against a `Session` |
| L4 | `control-key` | 0.1k | Per-window control-port key files shared by the app and the MCP bridge |
| L5 | `mcp` | 1.0k | MCP server: every command as a tool; headless or bridged to the running app |
| L6 | `ui-egui` | 13.1k | Word-style front end: ribbon, keytips, mini-toolbar, Backstage, canvas, rulers, panes, dialogs, Equation tab, i18n catalogs, themes, control channel, headless `ui_shot` example |
| apps | `wordcraft` | 1.6k | Desktop app (eframe + wgpu; logging, window geometry, file associations) |
| apps | `wordcraft-cli` | 0.4k | `convert`, `text`, `inspect`, `run`, `commands`, `parity`, `mcp`, `zotero` |
| apps | `wordcraft-web` | 0.3k | Browser build (eframe web runner, WebGPU with WebGL2 fallback) |

## Data flow

1. **Open:** `engine::io` picks a reader by extension (`docx`, `docbin`, `formats`, JSON) and
   produces a `doc::Document`.
2. **Edit:** every user-visible action is a command id run through `Session::run` (ribbon,
   shortcuts, command search, CLI, control channel and MCP all dispatch by id). `Session::run`
   snapshots for undo (blocks are `Arc`-shared, so snapshots are cheap) and catches an escaped
   panic, restoring the document (never-crash standard).
3. **Layout:** `layout` turns the document into pages of positioned glyphs, boxes and images
   (display list), incrementally: an edit relayouts only what changed (1.4 ms on a 188-page
   document, measured 2026-10-06).
4. **Draw:** the UI paints pages through `render` (CPU raster, cached per page, pixel-aligned) on
   an egui/wgpu canvas; PDF export walks the same display list through `pdf`.
5. **Save:** `engine::io` picks a writer by extension.

## Testing and gates

- 821 `#[test]` functions and 10 property-test blocks: model, layout, DOCX round-trip and hostile
  input, `.doc`, formats, typing parity (`engine/src/tests_typing.rs`), bidi, i18n catalogs, UI
  interaction (egui_kittest), MCP acceptance; `hostile_params_never_panic` runs every command with
  junk params.
- `cargo xtask ci`: fmt, clippy `-D warnings`, tests, asset attribution, layering, wasm build.
- `cargo xtask parity` regenerates [`parity-checklist.md`](parity-checklist.md); a test keeps the
  catalog percentage above a floor that only rises.
- GitHub Actions on PRs: packaging lint, and FreeBSD / Windows ARM64 when build files change;
  release builds on pushes to `release`.

## Interface reading direction

`ui-egui::rtl` centralizes interface direction. Wrap panel, window and popup bodies with
`rtl::root`, then use `rtl::column` for nested columns; egui's native `vertical` resets alignment.
Horizontal rows inherit the parent. `rtl::columns` preserves reading order where native
columns reset it. `rtl::singleline` aligns plain interface text and search hints without
reordering stored text; numeric/code editors retain their own rules. `rtl::form_field`
places a two-column form label before its value in reading order.
Use the shared combo, checkbox/radio and menu wrappers;
LTR delegates to the original egui operation. Custom-painted controls place their icon at the
reading start and their arrow at the opposite edge. New catalogs declare their reading direction in `LangInfo::rtl`; changing a display name
does not change layout.

Grids keep semantic columns in an LTR island aligned at the reading edge (egui cannot lay out a
Grid under an RTL parent). Data grids, equations and document geometry must not be blindly
reversed. Document paragraph direction controls its ruler and layout independently of UI direction.

`ui_bidi` arranges display labels with UAX #9 while egui shapes logical script stretches.
Catalogs retain logical translations (`tr`/preview contexts); formatting resolves templates before
arranging display text. Use `logical_fmt` for generated document values. Do not feed display-order
strings into document layout, persisted values or editable text. egui's editable fields and line
wrapping still need upstream bidi support; this is not a replacement document text engine.

Ribbon groups cache their actual mirrored intrinsic width and execute their content once;
the group column must use the same reading edge as its content, or minimum-width expansion
adds blank space. Group dividers use the trailing cursor edge in each direction.

The ribbon uses egui `ScrollArea` for clipping, scrolling and bounds. A measured finite
row width anchors RTL groups; vertical wheel input follows reading order. Horizontal
touchpad input keeps its physical direction. Do not implement a second scroll engine.

Local screenshots can be generated with `WORDCRAFT_RTL_AUDIT_DIR` and the
`tests_rtl::local_direction_screenshots` test. The synthetic catalog is compiled only for tests.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-13 | minor | Shared RTL UI infrastructure and regression coverage: logical chart/preview text, reachable chart data, grid choice controls and preserved native LTR paths; catalogs, native frame and full bidi editing remain separate |
| 2026-10-10 | minor | `proof`: proofing languages (English, Polish), affix engine, Polish patterns; layout and engine route each run by its language |
| 2026-10-10 | major | First committed architecture doc, from the crates' sources and `AGENTS.md` |

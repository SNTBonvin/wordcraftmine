# Embedding WordCraft in a web app

> **Last reviewed:** 2026-10-13 · **Last updated:** 2026-10-13 · **Change:** major (first version: embed mode, `WordcraftEmbed`) · **Target:** Microsoft Word (Microsoft 365) for Mac 16.113.4

The web build is also a library: `WordcraftEmbed` runs the editor on a canvas that another web app
owns, and that app drives it from JavaScript. Memento uses it to edit Word files; the contract both
sides build against is Memento's `docs/wordcraft-embed.md`. This page is WordCraft's side.

## Build

```sh
cargo build --release --target wasm32-unknown-unknown -p wordcraft-web --lib
wasm-bindgen --target web --out-dir dist/embed target/wasm32-unknown-unknown/release/wordcraft_web.wasm
wasm-opt -Oz dist/embed/wordcraft_web_bg.wasm -o dist/embed/wordcraft_web_bg.wasm   # optional, as trunk does
```

Without `wasm-opt` the module is about 47 MB (15 MB gzipped); hosts should load it only when a
document is opened.

`dist/embed` then holds `wordcraft_web.js`, `wordcraft_web_bg.wasm` and `wordcraft_web.d.ts`
(typed: `ImportReport`, `Decoration`, `EmbedOptions`, `EmbedEvent`). The full app is still the
`wordcraft-web` binary, built with `trunk build --release`.

## API

```ts
import init, { WordcraftEmbed } from './wordcraft_web.js'
await init()
const wc = await WordcraftEmbed.start(canvas, { theme: 'light', readOnly: false })
const report = wc.open('notes.docx', bytes)    // { lossy, dropped }
wc.on('change', () => { /* wc.text(), wc.isDirty() */ })
wc.on('selection', () => { /* wc.selection() */ })
wc.on('save-shortcut', () => save(wc.save()))
wc.setDecorations([{ start: 120, end: 168, color: '#ffd54f' }])
wc.select(120, 168)
wc.destroy()
```

| Method | What it does |
|---|---|
| `start(canvas, { theme, readOnly? })` | Starts the editor on `canvas`. Embed mode: the ribbon (no File tab or Share button) and the pages; no title bar, status bar, side panes, Backstage, Start screen or AutoSave |
| `open(name, bytes)` | Opens a document from its bytes (`file.open`), replacing the open one without asking; `name`'s extension gives the format. Clears the decorations |
| `save()` | The document as `.docx` bytes; clears the dirty flag. Works while read-only |
| `isDirty()` | Unsaved changes since `open()` or `save()` |
| `text()` | The body text: paragraphs separated by `\n`, table cells by `\t`, fields as their results, tracked deletions left out |
| `selection()` | The selection as `[start, end]` UTF-16 offsets into `text()`, or `null` |
| `select(start, end)` | Selects `text()[start, end)` (`select.range`) and scrolls it into view; offsets are clamped |
| `setDecorations(ranges)` | Tints ranges of `text()` (`#rrggbb` is drawn see-through, `#rrggbbaa` as given). Drawn over the page, never saved; edits move them along. At most 10,000 |
| `setTheme(theme)` | `light` or `dark` interface |
| `setReadOnly(on)` | Refuses every command that would change the document (see below) |
| `on(event, callback)` | `change`, `selection`, `save-shortcut` (⌘S, Ctrl+S or the Save button: the host saves) |
| `destroy()` | Stops the editor and frees it; later calls are errors |

Errors are thrown as JavaScript `Error`s; nothing panics. Events are sent after the frame they
happened in, so a callback may call back into the editor.

**`open()`'s report is conservative for now:** every file reports `lossy: true` (with
`dropped: [{ what: "report/not yet available", count: 1 }]`) until the readers count what they
can't keep (the second embedding PR). A host that follows the contract opens files read-only
until then.

## How it works

- **Offsets** come from `wordcraft_engine::TextIndex` (`crates/engine/src/text_index.rs`): the
  body walked paragraph by paragraph, with every UTF-16 offset mapped to a `Pos` and back. An
  offset inside a field's result maps to the field's start (or, for a range's end, just after it);
  positions in deleted text read as the live text after them.
- **Decorations** are kept as offsets and carried over each edit by comparing the text before and
  after it (`remap_ranges`): an edit before a passage moves it, typing inside grows it, typing
  just before or after it leaves it out. They are painted with the layout's selection rectangles.
- **Read-only** is `Session::read_only`: `Session::run` refuses commands marked `mutates`, and
  takes back any other command that changes the document anyway (Undo, Redo, setting a password).
  Replacing the document (`open()`) is allowed. Buttons for refused commands are greyed out.
- **Embed mode** is `WordApp::embedded()` (`crates/ui-egui/src/embed.rs`). File › New, Open and
  Close are refused there: the host has the files.
- The handle and the eframe shell share the `WordApp` through a `RefCell`; a call that would find
  it borrowed returns an error instead of panicking.

Tests: `crates/engine/src/tests_text_index.rs` (round trips in plain paragraphs, tables, field
results and tracked changes; decorations across edits; read-only) and the tests in
`crates/ui-egui/src/embed.rs`.

## Revision history

| Date | Change | What |
|---|---|---|
| 2026-10-13 | major | First version: embed mode, `WordcraftEmbed`, `TextIndex`, decorations, read-only |

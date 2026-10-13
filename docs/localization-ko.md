# Korean interface

> **Last reviewed:** 2026-10-13 · **Last updated:** 2026-10-13 · **Change:** minor (first version)

The `ko` catalog has 2,463 entries: every `(context, source)` key in the existing interface
catalogs, including ribbon labels, commands, dialogs, tooltips, count templates and built-in
style names. Missing future keys still fall back to English. English literals that have not
been connected to the translation system remain English, as in the other languages.

Select **한국어** in File › Options › Interface language, or send
`ui.language` with `{"value":"ko"}` through the control channel. The saved preference survives
restart. Automatic selection recognizes `ko-KR`, `ko_KR.UTF-8`, `ko-Kore-KR` and other Korean
locale tags. `WORDCRAFT_LANGUAGE=ko cargo run -p wordcraft -- --sample` selects Korean when the
interface setting is automatic.

The translations were written from the English source meaning, without consulting another
product's Korean UI or translation tables. Product names, format names and command IDs retain
their spelling. Interface language does not translate document contents or rename user styles.

## Terminology

| English | Korean |
|---|---|
| Home / Insert / Layout / Review / View | 홈 / 삽입 / 레이아웃 / 검토 / 보기 |
| Font / Paragraph / Style | 글꼴 / 문단 / 스타일 |
| Heading / Table of Contents / Index | 제목 / 목차 / 색인 |
| Track Changes / Comment | 변경 내용 추적 / 메모 |
| Header / Footer / Footnote / Endnote | 머리글 / 바닥글 / 각주 / 미주 |
| Save / Undo / Redo | 저장 / 실행 취소 / 다시 실행 |

Count labels use Korean units without English plural variants (`0단어`, `1단어`, `21단어`).
Placeholders can move in a sentence: `Page {page} of {pages}` becomes
`전체 {pages}쪽 중 {page}쪽`. Long instructions use polite Korean; buttons use short action labels.

## Fonts and scope

Hangul uses the existing CJK interface fallback chain. A native build with `CRAFT_FONTS_DIR`
uses the Noto Sans CJK SC face from craft-fonts for Hangul that the Japanese face does not cover.
Without embedded CJK fonts, native builds look for installed fonts; the candidate list includes
Malgun Gothic on Windows, Apple SD Gothic Neo on macOS, and Noto CJK KR or UnDotum on Linux/BSD.
Coverage is measured from the catalog, rather than inferred from the font's name. If the first
installed face covers Japanese but lacks Hangul, a second face covers the remaining characters.

Web builds with craft-fonts include Noto Sans CJK SC Regular as well as BIZ UDPGothic Regular.
This adds the full CJK face to the WASM download, but supplies Hangul without system fonts.
A web build without craft-fonts does not have CJK glyph coverage.

This catalog does not add Korean spelling, grammar, hyphenation or document templates, and
makes no changes to native IME handling. Live macOS Korean composition testing and native
speaker terminology review remain separate work.

## Validation

- `cargo test -p wordcraft-ui-egui i18n`: catalog syntax, complete key coverage, ribbon and
  command translations, Korean locale matching, saved preference, unchanged document content,
  placeholder substitution and count labels.
- `cargo test -p wordcraft-ui-egui command_search_matches_translated_labels_and_canonical_commands`:
  Korean labels and canonical English command names/IDs remain searchable.
- Font checks exercise every translated Hangul syllable in regular, medium and semibold UI
  families; embedded CJK checks run with `CRAFT_FONTS_DIR`, otherwise installed fonts are used.
- Headless screenshots check the Korean ribbon and Options dialog without driving a desktop.

## Local validation results (2026-10-12)

Validated on Linux with Rust 1.96.0 and craft-fonts checkout
`3d9ab3a132ed0eff457e424226a435a901da30d3`:

- Formatting, attribution (51 assets), layering (20 crates), `cargo xtask wasm` and the
  `wordcraft-web` WASM target check passed.
- Workspace tests passed: 1,524 tests, one existing ignored interface audit, with the existing
  `myanmar_prevowel_is_drawn_before_its_consonant` test excluded. An unfiltered workspace run
  failed that test; the unmodified base (`b0f3d52`) also failed it in the full layout suite.
- Strict Clippy passed with only the two existing lint findings excluded:
  `cargo clippy --workspace --all-targets -- -D warnings -A clippy::nonminimal_bool -A clippy::drop_non_drop`.
  Standard `cargo xtask ci` still stops at `nonminimal_bool` in
  `crates/docx/src/write/embed.rs:530`; `drop_non_drop` in
  `crates/engine/src/ribbon.rs:465` is the other existing finding. These files were not changed.
- The 37 localization tests also passed without embedded craft-fonts, using installed fonts
  for Hangul coverage.
- The headless Home ribbon, Insert ribbon and Options screenshots displayed Korean correctly.
  No live macOS IME test was performed.

## Current split validation (2026-10-13)

Synced to upstream main `159b84f`; translated 47 additional catalog keys for
2,463 entries total. This branch includes the catalog, font support and app-side
IME regression coverage, with no vendored dependency or native winit changes.
See [Korean IME validation](korean-ime.md) for the tested event sequences and
the native first-key/duplicate-space fixes deferred to #862.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-12 | minor | Latest main synchronized; 30 newly introduced catalog keys translated, 2,416 total |
| 2026-10-12 | minor | Initial Korean catalog, terminology, font requirements and validation |
| 2026-10-13 | minor | Split from #862 without vendoring; translated 47 latest-main keys, 2,463 total |

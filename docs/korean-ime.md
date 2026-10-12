# Korean IME issue resolving: app-side handling

> **Last reviewed:** 2026-10-13 · **Last updated:** 2026-10-13 · **Change:** minor (split app-side fixes from native winit changes)

## Scope and deferred native fixes

This change splits the Korean interface catalog and app-side IME handling from
[PR #862](https://github.com/storytold/wordcraft/pull/862), following maintainer
feedback. It uses main's unchanged winit dependency and includes no vendored code
or dependency override. The native Apple Korean first-key retry and duplicate
space callback guard remain in #862, pending the maintainers' shared winit fork
decision. This change alone does not resolve those native callback bugs.

On the original Mac reproduction, switching ABC → Apple Korean two-set and typing
`동해물과` delivered plain `Text("ㄷ")` followed by separate vowel/final-consonant
callbacks. This matches [winit #3095](https://github.com/rust-windowing/winit/issues/3095).
Once native input has separated those callbacks, the app cannot safely repair the
syllable by joining document characters. Successful native retests of the original
combined branch therefore are not evidence that this split fixes first-key input.

## App-side changes

- Publish IME placement during the frame that takes canvas focus; retain native
  readiness while the window is inactive, with caret drawing following activation.
- Repaint on preedit/commit and position the caret and candidate window after the
  pending syllable. The native input method owns composition Backspace.
- Complete pending text at the old insertion point before pointer navigation or
  window focus loss, consuming same-batch native commits once.
- Preserve forwarded Enter/Backspace after commit plus empty preedit; complete
  pending text for application navigation and shortcuts, while ordinary physical
  letter keys leave composition intact.
- Retain symbol/emoji fallbacks for medium and semibold interface fonts and enable
  Korean catalog, locale preferences and Hangul font coverage.

## Test scenarios

| Scenario | Expected behavior | Evidence |
|---|---|---|
| Type `가나다라`, leave `라` pending, Backspace twice | `라 → ㄹ → empty`; committed `가나다` is unchanged until the next document Backspace | Synthetic UI regression |
| Enter during pending `라` | Complete `라` and create exactly one new paragraph; the next Enter creates one more | Previously failing regression now passes; final native retest reported normal |
| Period/comma/exclamation/question mark during pending `라` | Complete the syllable and insert the delimiter once, whether included in IME commit or sent separately | Synthetic UI regression for both native event forms |
| Backspace after commit plus trailing empty preedit | Apply the forwarded document Backspace, while preedit-only Backspace stays owned by IME | Actual event sequence and UI regressions |
| Composition changes during typing | Immediate repaint; cursor lies after the pending syllable | Synthetic UI regression and offscreen screenshot |
| Ordinary physical letter key while first `ㄷ` is pending | No forced commit or native interruption; later preedit becomes `도`, then `동` | Synthetic UI regression |
| Click/navigate away or deactivate the window with pending `라` | Pending syllable finishes once at the old caret | Synthetic UI regression |
| Native commit and pointer press in one frame | No duplicate syllable | Synthetic UI regression |
| Backspace/Undo on committed `가나다라` | Backspace gives `가나다`; Undo restores `가나다라` | Engine typing regression |


The tests inject egui input events and verify document content, caret placement and
repaint behavior. They do not claim native verification of this split on every
platform, every Korean input method, or observed Word parity. A synthetic space
commit inserts its delimiter once; duplicate callbacks from native winit require
the deferred native guard.

## Running tests and diagnostics

```sh
cargo test -p wordcraft-ui-egui --lib
cargo test -p wordcraft-engine committed_korean_syllables
cargo xtask ci
cargo run -p wordcraft -- --control 7981
```

Opt in to native input tracing with
`RUST_LOG=info,wordcraft_ui_egui::ime=trace` during a deliberate synthetic-text
session. It includes typed text and is disabled by default. Local logs and
screenshots are not committed.

## Revision history

| Date | Change | Detail |
|---|---|---|
| 2026-10-13 | minor | Latest-main sync; app-side/catalog split without vendor; native retry and space guard explicitly deferred to #862 |

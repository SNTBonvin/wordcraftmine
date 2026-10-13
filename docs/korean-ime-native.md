# Korean IME issue resolving: deferred native patches

> **Last reviewed:** 2026-10-13 · **Last updated:** 2026-10-13 · **Change:** minor (macOS native first-key fix and composition regression coverage)

This document preserves the combined-branch native test evidence for #862.
The no-vendor app-side split is [PR #1127](https://github.com/storytold/wordcraft/pull/1127).
Native dependency changes here remain pending the maintainers' shared winit fork decision.

## Reproduction and cause

On this Mac, start a fresh WordCraft, focus the document, switch from ABC to the
Apple Korean two-set input method and immediately type `동해물과` (`ehgoanfrhk`).
The first syllable was separated: native input delivered `Text("ㄷ")`, followed
by an independent `Preedit("ㅗ")`, `Commit("ㅗ")` and `Commit("ㅇ")`, before normal
composition resumed with `해`. Earlier testing also separated the first `가`
of `가나다라` as `ㄱㅏ`.

Hangul input incrementally combines initial consonants, vowels and final
consonants. Once the native input method has emitted the first consonant as
plain text, the next vowel no longer shares its composition. This is not a
Unicode normalization problem, a font-rendering problem, or a requirement to
implement Korean composition inside the document engine.

The failing run emitted `IMKCFRunLoopWakeUpReliable` when changing input source.
The event sequence matches [winit #3095](https://github.com/rust-windowing/winit/issues/3095).
The reviewed workaround retries the eligible first raw-jamo callback within
AppKit before the application receives it; see [patch provenance and guards](../vendor/patches/winit/README.md).

## Change

- A local override of **winit 0.30.13** adds the scoped first-Apple-Korean-key retry and prevents a
  space already committed by IME from being emitted again as keyboard text. It preserves the same dependency version and retries at most once,
  only with the same focused native first responder and input source, no active
  composition, no repeat and no Command/Control/Option modifiers. Native
  callbacks or reentrant events invalidate the retry. It does not join
  independently inserted jamo in document content.
- The canvas publishes IME placement in the frame that it takes keyboard focus
  and retains native IME readiness while the window is inactive. Caret drawing
  still follows actual window activation.
- Composition changes request an immediate repaint; the visible caret and
  candidate placement follow the pending syllable. Pending text remains owned
  by the native input method, including its two-set Backspace edits.
- Pointer presses/window focus loss complete pending text at its original
  insertion point before application navigation. Already committed native text
  in that event batch is inserted once. A trailing empty preedit after a commit
  no longer suppresses forwarded Enter/Backspace actions. Navigation/application shortcuts finish
  pending text, while ordinary physical letter keys do not force completion.
- Medium/semibold interface fonts retain egui's symbol and emoji fallbacks.
  This also avoids egui's replacement-face false negatives in Hangul coverage
  checks. The branch's existing Korean translation catalog remains enabled.

## Test scenarios

| Scenario | Expected behavior | Evidence |
|---|---|---|
| Fresh launch, ABC → Korean, immediately type `동해물과` | First syllable begins `ㄷ → 도 → 동`; committed document starts with `동`, with no stray `ㄷㅗㅇ` | Native user retest on this Mac; captured preedit/commit sequence |
| Without spaces: `재현` → ABC `asdf` → Korean `동해물과` | `재현asdf동해물과`, with the first new Korean syllable composed normally | Native user retest on the latest single diagnostic instance; retry policy and UI regressions |
| Switch back to ABC and then Korean and type again | ASCII follows the normal text path; Korean re-enters composition with `ㄷ → 도 → 동` | Native event trace from the same retest |
| Type `가나다라`, leave `라` pending, Backspace twice | `라 → ㄹ → empty`; committed `가나다` is unchanged until the next document Backspace | Synthetic UI regression |
| Space during pending `라` | Complete the syllable and insert exactly one space, `라 ` | Native space-commit trace and synthetic UI regression; duplicate-keyboard-text guard in winit |
| Enter during pending `라` | Complete `라` and create exactly one new paragraph; the next Enter creates one more | Previously failing regression now passes; final native retest reported normal |
| Period/comma/exclamation/question mark during pending `라` | Complete the syllable and insert the delimiter once, whether included in IME commit or sent separately | Synthetic UI regression for both native event forms |
| Backspace after commit plus trailing empty preedit | Apply the forwarded document Backspace, while preedit-only Backspace stays owned by IME | Actual event sequence and UI regressions |
| Composition changes during typing | Immediate repaint; cursor lies after the pending syllable | Synthetic UI regression and offscreen screenshot |
| Ordinary physical letter key while first `ㄷ` is pending | No forced commit or native interruption; later preedit becomes `도`, then `동` | Synthetic UI regression |
| Click/navigate away or deactivate the window with pending `라` | Pending syllable finishes once at the old caret | Synthetic UI regression |
| Native commit and pointer press in one frame | No duplicate syllable | Synthetic UI regression |
| Backspace/Undo on committed `가나다라` | Backspace gives `가나다`; Undo restores `가나다라` | Engine typing regression |
| Retry policy negative controls | ABC/Japanese, mismatched/multiple callbacks, ineligible shortcuts/repeats, composition callbacks, reentrancy and context resets do not replay | Seven pure native policy tests plus a no-space Korean/ASCII/Korean rearm regression in workspace CI |

The native successful trace begins:

```text
Preedit("ㄷ")
Preedit("도")
Preedit("동")
Preedit("")
Commit("동")
Preedit("ㅎ")
Preedit("해")
```

Native user retesting reported successful first-key, no-space Korean/ASCII/Korean
transitions, Space and Enter on the final single diagnostic instance. Pending-syllable UI cases
also have synthetic tests; this document does not claim native verification for every
editing case, every Korean input method, Windows/Linux or observed Word parity.

## Running tests and diagnostics

```sh
cargo test -p wordcraft --test macos_ime_startup
cargo test -p wordcraft-ui-egui --lib
cargo test -p wordcraft-engine committed_korean_syllables
cargo xtask ci
cargo run -p wordcraft -- --control 7981
```

For a deliberate synthetic-text diagnostic session, opt in to the input-event
trace with `RUST_LOG=info,wordcraft_ui_egui::ime=trace`. The trace includes typed
text and native IME events; it is disabled by default. Full local event logs and
screenshots are not committed.

## Dependency overlap and removal

[WordCraft PR #787](https://github.com/storytold/wordcraft/pull/787) independently
vendors this winit version with ten platform fixes. This branch imports its first-key retry policy and space-commit suppression, adapting the
native hooks to pristine 0.30.13; it does not include the other eight fixes. Reconcile with #787's vendor baseline if it
lands, and remove this override when the resolved winit release contains the
first-key fix.

## Revision history

| Date | Change | Detail |
|---|---|---|
| 2026-10-12 | minor | Actual first-key and mixed-source traces; first-key/space native guards; no-space mixed-source, Enter/Backspace and punctuation regressions; final native retest recorded |

| 2026-10-13 | minor | Synced main and app-side split #1127; native patch evidence preserved separately during conflict resolution |

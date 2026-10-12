# winit 0.30.13: Apple Korean first key and space

The local override starts from the crates.io `winit` 0.30.13 archive, SHA-256
`a6755fa58a9f8350bd1e472d4c3fcc25f824ec358933bba33306d0b63df5978d`.
All dependency files except the three listed below remain pristine; Cargo's
registry-cache marker and VCS metadata are not copied.

- `src/platform_impl/macos/mod.rs`: register the retry policy module.
- `src/platform_impl/macos/ime_startup.rs`: pure, tested retry policy adapted from
  [WordCraft PR #787, commit 0d4a18d](https://github.com/cyfung1031/wordcraft/commit/0d4a18d),
  itself from [PdfCraft #680](https://github.com/storytold/pdfcraft/pull/680) and
  [winit #4744](https://github.com/rust-windowing/winit/pull/4744).
- `src/platform_impl/macos/view.rs`: report native callbacks to that policy and
  repeat `interpretKeyEvents` once for an eligible first raw compatibility-jamo
  `insertText` callback. The policy is disarmed before the retry. Also remember
  commits during key interpretation so a Korean Space commit followed by empty
  preedit does not emit the same space again as keyboard text.

On this Mac, immediately switching from ABC to Korean and typing `동해` produced
`Text("ㄷ")`, `Preedit("ㅗ")`, `Commit("ㅗ")`, `Commit("ㅇ")`, then normal
`Preedit("해")`, with `IMKCFRunLoopWakeUpReliable` in the system log. This matches
[winit #3095](https://github.com/rust-windowing/winit/issues/3095). UI focus changes
alone cannot repair it after the native IME has lost the first key.

The retry requires Apple Korean, a single matching compatibility jamo callback,
IME enabled but no active composition, a non-repeated key without Command,
Control or Option, unchanged input source and the same focused first responder.
Preedit, command, unmark, unmatched/multiple insert callbacks, nested key events
and IME resets invalidate it. It does not join jamo in application text.

The space guard is adapted from [#787 commit 2956f8b](https://github.com/cyfung1031/wordcraft/commit/2956f8b), related to [winit #4478](https://github.com/rust-windowing/winit/issues/4478). The commit marker resets before each key interpretation and explicit native command forwarding is retained (including Enter).

Unlike #787, this override contains only the first-key and space fixes. Its guards are adapted
to pristine 0.30.13: no `pending_commit` or `in_key_down` fields from #787's other
patches are introduced. The disabled-state and empty-marked-text checks exclude
an active or just-cleared composition from the retry.

`apps/wordcraft/tests/macos_ime_startup.rs` runs all seven pure policy tests and
a no-space Korean/ASCII/Korean rearm regression through workspace CI. Native AppKit behavior also needs real keyboard retesting.
The vendor retains the original Apache-2.0 code license; its keyboard diagrams
retain CC BY-SA 4.0 with their original attribution. Example/docs assets are
attributed in `ATTRIBUTION.md` and are not used by WordCraft.

The broader existing PR #787 also vendors this exact winit release. When that PR
lands, reconcile the override with its baseline rather than keeping two copies.
Remove the local override and vendor once the resolved winit release includes
both fixes; keep the startup/input regression coverage across that upgrade.

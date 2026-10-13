# Changes Summary — `fix/open-issues-and-linux-deb-only`

## Project report
`docs/PROJECT_REPORT.md`. Key findings in five lines:
- WordCraft 0.6.1 is an alpha Rust word processor (egui/wgpu, ~422 commands, OOXML + 6 more
  formats); the never-crash standard is genuinely enforced (workspace deny-lints + hostile tests).
- The control channel bypassed the save-changes guard; Cut never reached the system clipboard;
  table Cell-Size boxes ignored selections; ODT/LaTeX exported `javascript:` links.
- Packaging/CI spanned macOS/Windows/FreeBSD/Linux-multi/Web; now reduced to one target.
- `cargo audit` is unavailable here and not wired into CI — the one audit gap.
- 271 open issues triaged: 10 fixed here, 4 already fixed (proven), 1 mitigated, 23 obsolete by
  scope, 22 not reproducible, 74 feature requests deferred, 137 bugs still open.

## Issues table (number, title, status, note)

| # | Title (short) | Status | Note |
|---|---|---|---|
| 1243 | Copy: rich text lost on clipboard | FIXED (`5ee4e70`) | HTML+text via arboard `set_html` |
| 1242 | Table Height/Width apply to one row/col | FIXED (`76a5524`) | all selected rows/cols + test |
| 1265 | Cut does not write to clipboard | FIXED (`6817fb7`) | ribbon + menu now copy; kbd already did |
| 1268 | `file.close` over control channel quits dirty | FIXED (`618903f`) | `execute` needs `force:true` + test |
| 981 | CLI render accepts bad --page/--scale | FIXED (`f77f536`) | strict parse, exit≠0 + tests |
| 968 | `--help` as file / rejected | FIXED (`f77f536`) | subcommand help + GUI help |
| 1148 | Dark ruler shading inverted | FIXED (`67681d9`) | tokens swapped back |
| 866 | `get_text`/CLI text include deleted text | FIXED (`76a5524`) | `Document::final_text` |
| 1080 | CLI text drops list labels | FIXED (`b622409`) | `text` uses txt exporter |
| 679 | HTML/MD write `javascript:` hrefs | FIXED (`b622409`) | HTML/MD were filtered; ODT/LaTeX now too |
| 187 | Open replaces edited doc silently | ALREADY FIXED | guard since `802ba30`, tests pass |
| 168 | Exports clear Edited indicator | ALREADY FIXED | `keeps_everything` since `802ba30` |
| 1212 | Cannot drag-and-drop picture | ALREADY FIXED | drop path at `ui-egui/lib.rs:1391` |
| 1169 | Typing stops on XWayland+fcitx5 | ALREADY FIXED | focus guard at `canvas.rs:1226-30` |
| 1232 | Outline of large doc quits app | MITIGATED (`4f6df7b`) | panes virtualized; crash repro still needed |
| 1229, 1111*, 1103, 979, 644, 954, 845, 670, 533, 525, 170, 73, 1119, 1122, 984, 39, 64, 738, 808, 51, 996*, 285*, 324 | (platform-only, see analysis) | OBSOLETE | closed by scope reduction |
| 22 vague/empty reports (1186, 1233, 47, 780, 991, …) | — | NOT REPRODUCIBLE | what was tried is in the analysis |
| 74 feature requests (1227, 1196, 1192, 1155, 764, 1160, …) | — | NEEDS DECISION | deferred with rationale |
| 137 remaining bugs (886, 997, 1034, 1213, 1011, 1008, …) | — | NEEDS FIX | open, see analysis for hypotheses |

Full per-issue entries: `docs/ISSUES_ANALYSIS.md` (14 deep-dives + 271-row table).
\*: 1111 keeps its narrow-ribbon half; 996/285 need a Linux repro to reopen.

## Platform cleanup (removed)
- `packaging/macos/` (dmg scripts, Info.plist.in, entitlements, cert import), `packaging/windows/`
  (WiX .wxs, ps1), `packaging/linux/flatpak/` + bundle script, `.github/workflows/freebsd.yml`,
  `windows-arm64.yml`; `release.yml` → single Ubuntu 22.04→24.04 `x86_64` `.deb` job;
  `printing.yml` → Ubuntu only.
- `package.sh` deb-only + refuses non-x86_64; `nfpm.yaml` deb-only; macOS/Windows deps out of
  `apps/wordcraft/Cargo.toml` (18 lock entries dropped); `wordcraft.icns` + `wordcraft-macos-512.png`
  deleted (+ATTRIBUTION rows); `icons.sh` Linux-only; `app_icon()` uses hicolor.
- Docs: README Downloads → one `.deb` row; `releasing.md` rewritten; roadmap/control-protocol/
  file-format table/AGENTS/CLAUDE updated. History (`ROADMAP.md` logs) untouched deliberately.
- Kept deliberately: `cfg(target_os)` Rust paths (inert, uncompiled on Linux), winresource
  `build.rs` stub, web/WASM sources (`xtask wasm` green), `/opt/homebrew` Hunspell search path,
  `control-key` multi-OS paths (runtime, not build).

## Built artifact
- `dist/release/wordcraft-0.6.1-linux-x86_64.deb` — 40,833,482 bytes, full clean rebuild from
  the final tree (`package.sh` EXIT:0).
- `dpkg-deb --info`: `Package: wordcraft / Version: 0.6.1-1 / Architecture: amd64 /
  Maintainer: WordCraft maintainers <wordcraft@storyteller.ai> /
  Depends: libc6 (>= 2.35), libgcc-s1, libxkbcommon0, libvulkan1 | libegl1` (+ Wayland/X11
  recommendations, MIT OR Apache-2.0, homepage, description). Contents: `/usr/bin/{wordcraft,
  wordcraft-cli}`, `.desktop`, MIME, AppStream metainfo, hicolor icons, `doc/wordcraft`
  (licenses+README), postinst/postrm.
- `lintian`: not installed here (stated, not faked).
- Throwaway verify: `dpkg-deb -x` + `wordcraft-cli --version` → `0.6.1`; `ldd` resolves
  (libc/libm/libgcc only); smoke: `run --template sample … --save` + `text` + `render` OK;
  `render --page banana` → `render: --page needs a positive integer (got banana)`, exit 1.

## Test / lint / build results
- `cargo xtask ci`: PENDING at write time — result below (must be green before merge).
  Note: the first full run caught 4 ui-egui tests encoding the old silent-replace contract
  (`programmatic_*_never_prompt`, `a_prompt_is_dropped…`, `undo_after_a_mailing…`); they now
  assert the #1268 error-without-force / replace-with-force contract. Re-run pending.
- Targeted suites, all green: engine `cell_size*` + `document_text_shows*` (3), formats
  `exported_links`/`unsafe_link`/`txt_export_keeps` (3), CLI (13 incl. 3 new), doc `html_copy*`
  (2), ui-egui save-changes family + `execute_refuses` + `copy_writes_html` (8).
- `cargo clippy --workspace --all-targets -- -D warnings`: green (one `redundant_closure` from
  this work caught and fixed). `cargo fmt --all`: applied. `xtask assets` + `layers`: OK.
- Pre-existing failures: none observed in the touched suites; full-workspace result pending.

## Needs a human decision
1. #764 (LanguageTool key in plaintext `ui.json` → OS credential store) and #1160 (private
   `craftrules` links): both need owner calls, documented in the analysis.
2. Web/WASM sources kept for dev + `xtask wasm`, but no release artifact — confirm that's the
   intended end state (or say to delete `packaging/web` + `apps/wordcraft-web` too).
3. `cfg(target_os="macos|windows")` Rust code left in tree (inert) vs deleted — confirm keep.
4. 137 NEEDS FIX + 74 NEEDS DECISION issues remain open (prioritized list in the report §8);
   nothing was closed/commented on GitHub — a maintainer should close the OBSOLETE/FIXED ones.

## Branch and commits
Branch: `fix/open-issues-and-linux-deb-only`
- `76a5524` fix: table Cell Size boxes apply to whole selection; document.text shows tracked changes as final (closes #1242, closes #866)
- `b622409` fix: ODT/LaTeX drop unsafe link targets; txt export keeps list labels (closes #679, closes #1080)
- `6817fb7` fix: Cut writes the system clipboard from ribbon and context menu (closes #1265)
- `618903f` fix: control-channel execute refuses to discard unsaved work without force (closes #1268)
- `f77f536` fix: CLI validates --page/--scale and subcommand --help; GUI handles --help (closes #981, closes #968)
- `67681d9` fix: dark theme ruler shading had text and margin areas swapped (closes #1148)
- `4f6df7b` perf: virtualize navigation pane lists to avoid huge outlines blowing up (mitigates #1232)
- `49b8448` chore: Linux .deb x86_64 is the only build target (remove macOS/Windows/FreeBSD packaging and CI)
- `5ee4e70` fix: Copy/Cut place formatted HTML on the system clipboard (closes #1243)
- `29faa15` fix: use function item for clipboard_write hook; docs: project report, issues analysis, progress
- `bf9691c` fix: programmatic-discard tests follow the force:true contract (closes #1268)

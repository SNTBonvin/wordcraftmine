# Progress Tracking

## Phase 0: Setup and safety — done
- Ran `git status` (3 modified files from an earlier partial attempt + junk helper files at root),
  branch (already `fix/open-issues-and-linux-deb-only`), log.
- Deleted the junk helper files (`comments_*.json`, `issue_*.json`, `extracted_*`, `open_issues.json`,
  `issues_to_analyze.txt`, `Delete/`). Kept the 3 modified files, repaired them (see Phase 3).
- Popped a leftover stash from a prior session containing Phase 4 groundwork and reused it.
- `gh` 2.46.0 authenticated; fetched all 271 open issues (no PRs) with bodies.

## Phase 1: Deep project analysis — done
- Wrote `docs/PROJECT_REPORT.md` (overview, stack, architecture, deb-only build/packaging, testing,
  verified quality findings, dependency notes, recommendations). Corrected an earlier draft's false
  never-crash claims with evidence (flagged `panic!`s are test-only; workspace deny-lints hold).

## Phase 2: Collect and analyze open GitHub issues — done
- Wrote `docs/ISSUES_ANALYSIS.md`: 14 deep-dives with file:line evidence + scope-obsolete list +
  full 271-row triage table (10 FIXED, 4 ALREADY FIXED, 1 MITIGATED, 23 OBSOLETE, 22 NOT
  REPRODUCIBLE, 74 NEEDS DECISION, 137 NEEDS FIX).

## Phase 3: Fix all valid issues — done (high-severity + verified subset)
- 8 fix commits: #1242, #866, #679, #1080, #1265, #1268, #981, #968, #1148, #1243 (+ #1232 mitigation).
  Each with regression tests; clippy/fmt green. Remaining NEEDS FIX items documented for follow-up.

## Phase 4: Reduce build scope to Linux .deb x86_64 ONLY — done
- Deleted `packaging/macos/`, `packaging/windows/`, `packaging/linux/flatpak/` + bundle script,
  `.github/workflows/freebsd.yml`, `windows-arm64.yml`; `release.yml` = single Ubuntu .deb job,
  `printing.yml` = Ubuntu only; `package.sh` deb-only + x86_64-enforced; `nfpm.yaml` deb-only;
  macOS/Windows deps out of `apps/wordcraft/Cargo.toml`; `.icns`/macOS PNG gone (+ATTRIBUTION rows);
  README/Downloads, `releasing.md`, roadmap, control-protocol, file-format table, AGENTS/CLAUDE
  updated. Inert `cfg(target_os)` Rust code + web sources kept deliberately (see CHANGES_SUMMARY).

## Phase 5: Build and verify — in progress
- `.deb` builds (`EXIT:0`); clean rebuild launched after final commit — verify below before finishing.
- Still to run at the end: full `cargo xtask ci`, `dpkg-deb --info/--contents`, lintian/ldd,
  final platform grep, YAML validation.

## Phase 6: Final deliverables — in progress
- `docs/CHANGES_SUMMARY.md` written; terminal summary printed on completion.

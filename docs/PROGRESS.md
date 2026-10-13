# Progress Tracking

## Phase 0: Setup and safety
- [x] cd /home/vlad/wordcraft
- [x] Run git status, branch, and log
- [x] Create branch `fix/open-issues-and-linux-deb-only`
- [x] Stash uncommitted changes
- [x] Verify tooling (gh not found, using curl fallback)

## Phase 1: Deep project analysis
- [ ] Explore codebase and write `docs/PROJECT_REPORT.md`

## Phase 2: Collect and analyze open GitHub issues
- [ ] Fetch open issues and write `docs/ISSUES_ANALYSIS.md`

## Phase 3: Fix all valid issues
- [ ] Fix issues marked `NEEDS FIX`

## Phase 4: Reduce build scope to Linux .deb x86_64 ONLY
- [ ] Remove macOS, Windows, and FreeBSD build configurations and assets

## Phase 5: Build and verify
- [ ] Build .deb and verify artifact

## Phase 6: Final deliverables
- [ ] Produce `docs/CHANGES_SUMMARY.md`

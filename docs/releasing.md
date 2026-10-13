# Releasing WordCraft

This repository ships one build target only: **Linux x86_64 as a `.deb`** (`amd64`).

1. Bump the version on `main`: `cargo xtask version set 0.2.0` (the one source of truth is
   `[workspace.package] version`; the app shows it in `--version` and About, with the commit from
   `WORDCRAFT_BUILD_SHA`).
2. Merge `main` into `release` and push. `.github/workflows/release.yml` builds on `ubuntu-24.04`:

| Platform | Artifacts | Signing |
|---|---|---|
| Linux x86_64 | `wordcraft-<version>-linux-x86_64.deb` | checksums |

3. The run drafts a GitHub Release `WordCraft v<version>` with `SHA256SUMS.txt`; review and publish it.

Local packaging: `packaging/linux/package.sh` (needs `cargo` and `nfpm`).

Dry run: `gh workflow run release.yml --ref <branch>` builds the unsigned `.deb` on any branch, so
no draft release is made.

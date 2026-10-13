# Releasing WordCraft (this fork)

This fork (`hoblin/wordcraft`) is maintained for Bondata's own use, so it releases one build: Linux
x86_64. Upstream's workflows for macOS, Windows, FreeBSD and the rest need signing secrets the fork
does not have and were replaced by `.github/workflows/release.yml`.

1. Tag the commit on `main` as `v<upstream version>-bondata.<n>`, e.g. `v0.6.1-bondata.2`, and push
   the tag. `[workspace.package] version` in `Cargo.toml` stays at the upstream version; the tag is
   the release's version (`WORDCRAFT_VERSION`), and it names the archive.
2. The push runs `packaging/linux/package.sh --formats tar` on `ubuntu-22.04` (glibc 2.35) with
   the fonts from `storytold/craft-fonts` embedded, and publishes a GitHub release `WordCraft
   v<version>` with:

| File | Contents |
|---|---|
| `wordcraft-<version>-linux-x86_64.tar.gz` | upstream's Linux tarball shape: `wordcraft-<version>-linux-x86_64/{bin/wordcraft,bin/wordcraft-cli,share/...}` |
| `SHA256SUMS.txt` | `sha256sum` of every file |

A consumer downloads it the way it downloaded upstream's tarball, from
`https://github.com/hoblin/wordcraft/releases/download/v<version>/`.

To release a tag that already exists (or retry a failed run): `gh workflow run release.yml -f tag=v<version>`.

Local packaging: `packaging/linux/package.sh` (every format; add `--formats tar` for the tarball alone).
